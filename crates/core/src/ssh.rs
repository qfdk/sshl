// SSH backend — russh client.
// SshSession owns: Handle (shared with SFTP), the shell's outbound sender.
// The shell pump task owns only the Channel; shell 输出的原始字节直接交给 TerminalSink
// （界面的终端线程，由 libghostty-vt 解析），这里不解码、不缓冲。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::future::{AbortHandle, Abortable};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};

use crate::auto_auth;
use crate::config_store::{load_secret, AuthKind};
use crate::error::{AppError, AppResult};
use crate::known_hosts::{self, HostKeyCheck, Verdict};
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct ConnectionDetails {
    /// 保存的连接 id；临时连接为 None。用来取已保存的密码，也是「填充密码」的依据。
    pub id: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub auth: AuthKind,
    /// PTY 按终端区的真实网格开：不带尺寸时 shell 先按 80 列画提示符 / MOTD，终端出来后再重排，一连上就跳一下。
    pub cols: u32,
    pub rows: u32,
}

/// 登录凭据。
#[derive(Debug, Clone, Copy)]
pub enum Credentials<'a> {
    Password(&'a str),
    /// 私钥：路径或 PEM 内容。
    Key { key: &'a str, passphrase: Option<&'a str> },
    /// 免密：见 [`auto_auth`]。
    Auto,
}

/// 终端通道的输出去处。
pub trait TerminalSink: Send + Sync + 'static {
    /// shell 输出的原始字节（stdout、stderr 按到达顺序都在这里）。解析完要用 [`ShellHandle::ack`] 确认。
    fn data(&self, bytes: &[u8]);
    /// 通道结束：远端退出 / 断开、本地关闭。只调一次。
    fn closed(&self, code: Option<u32>, reason: String);
}

enum Outbound {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    /// 终端线程确认已解析的原始字节数。
    Ack(u32),
    Close,
}

/// 往终端通道写的一端：键盘输入、终端应答、改尺寸、流控确认。全是同步的（无界队列），
/// 终端线程直接持有它，按键不经过任何异步查找。会话结束后发送静默丢弃。
#[derive(Clone)]
pub struct ShellHandle(mpsc::UnboundedSender<Outbound>);

/// 对应 [`ShellHandle`] 的接收端，交给 [`connect`]。连接前写入的会排队，连上后按序发出。
pub struct ShellReceiver(mpsc::UnboundedReceiver<Outbound>);

pub fn shell_channel() -> (ShellHandle, ShellReceiver) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ShellHandle(tx), ShellReceiver(rx))
}

impl ShellHandle {
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.0.send(Outbound::Data(bytes.into()));
    }

    pub fn resize(&self, cols: u32, rows: u32) {
        let _ = self.0.send(Outbound::Resize { cols, rows });
    }

    /// 终端线程解析完的原始字节数，用来补终端通道的 SSH 窗口（见 TerminalWindow）。
    pub fn ack(&self, bytes: u32) {
        let _ = self.0.send(Outbound::Ack(bytes));
    }

    pub fn close(&self) {
        let _ = self.0.send(Outbound::Close);
    }
}

pub struct SshSession {
    // Identity fields retained for diagnostics/logging; not all are read yet.
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub host: String,
    #[allow(dead_code)]
    pub username: String,
    /// 所属保存连接的 id。fill_password 用它从加密库解密密码并直接写入通道 —— 密码全程不经过界面。
    pub connection_id: Option<String>,
    /// host:port:user —— 预热池据此跳过已有会话的主机（见 AppState::active_warm_keys）。
    pub warm_key: String,
    /// Shared SSH handle（shell / SFTP / exec 共用）。认证之后用到的 is_closed、channel_open_session、
    /// disconnect 都是 &self、各等各的通道确认，不加锁：加锁会把同一会话上的开通道串行化。
    pub handle: Arc<russh::client::Handle<ClientHandler>>,
    /// Cached SFTP session — reused across file ops to avoid sshd MaxSessions exhaustion.
    /// 通道卡死 / 关闭后不再复用（见 sftp_for）。
    pub sftp: Mutex<Option<Arc<crate::sftp_stall::Sftp>>>,
    /// Lazily loaded uid→username map (from /etc/passwd).
    pub passwd_cache: Mutex<Option<Arc<HashMap<u32, String>>>>,
    /// Lazily loaded gid→groupname map (from /etc/group).
    pub group_cache: Mutex<Option<Arc<HashMap<u32, String>>>>,
    /// 终端切换了用户（su / sudo -i）后，文件管理以该用户身份运行：SFTP 换成以他身份起的
    /// sftp-server，删除 / chmod / chown 也以他身份执行。None = SSH 登录用户。
    pub elevation: Mutex<Option<Arc<crate::terminal_user::Elevation>>>,
    shell: ShellHandle,
}

impl SshSession {
    /// 测试用：只有 SSH 连接、没有终端的会话（文件管理的命令只用到 handle、缓存和身份）。
    #[cfg(test)]
    pub(crate) fn for_test(handle: Arc<russh::client::Handle<ClientHandler>>, username: &str) -> Self {
        Self {
            id: "test".into(),
            host: String::new(),
            username: username.into(),
            connection_id: None,
            warm_key: String::new(),
            handle,
            sftp: Mutex::new(None),
            passwd_cache: Mutex::new(None),
            group_cache: Mutex::new(None),
            elevation: Mutex::new(None),
            shell: shell_channel().0,
        }
    }
}

pub(crate) fn warm_key(host: &str, port: u16, username: &str) -> String {
    format!("{}:{}:{}", host, port, username)
}

/// 连接上各通道（SFTP / exec）默认的 SSH 接收窗口。SFTP 下载吞吐上限 ≈ 窗口 / RTT：russh 默认 2MB
/// 在 50ms RTT 下约 40MB/s，8MB 约 160MB/s。终端通道不用它（单独开 TERMINAL_WINDOW 的手动窗口），
/// 刷屏时的积压不跟着变大。
pub(crate) const CONNECTION_WINDOW: u32 = 8 * 1024 * 1024;

fn russh_config() -> Arc<russh::client::Config> {
    Arc::new(russh::client::Config {
        window_size: CONNECTION_WINDOW,
        inactivity_timeout: Some(Duration::from_secs(600)),
        keepalive_interval: Some(Duration::from_secs(10)),
        keepalive_max: 1,
        ..Default::default()
    })
}

fn is_transient_connect_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::HostUnreachable
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::NetworkUnreachable
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
    ) || matches!(error.raw_os_error(), Some(51 | 60 | 61 | 64 | 65))
}

async fn connect_tcp(addr: &str) -> AppResult<TcpStream> {
    const ATTEMPTS: u64 = 4;
    const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

    for attempt in 1..=ATTEMPTS {
        let connect_result =
            match tokio::time::timeout(ATTEMPT_TIMEOUT, TcpStream::connect(addr)).await {
                Ok(result) => result,
                Err(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("connect timed out after {}s", ATTEMPT_TIMEOUT.as_secs()),
                )),
            };

        match connect_result {
            Ok(tcp) => return Ok(tcp),
            Err(error) if is_transient_connect_error(&error) && attempt < ATTEMPTS => {
                let delay = Duration::from_millis(400 * attempt);
                tracing::warn!(
                    "tcp connect {addr} transient {error}; retry {attempt}/{ATTEMPTS} in {}ms",
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                return Err(AppError::Ssh(format!("tcp connect {addr}: {error}")));
            }
        }
    }

    unreachable!("TCP retry loop always returns")
}

/// 开终端（shell）通道：russh 补丁的手动窗口模式，初始窗口 TERMINAL_WINDOW，之后只按
/// TerminalWindow 的记账补。只有终端通道这样开；SFTP / exec / 探活仍走连接默认窗口、自动补。
async fn open_terminal_channel(
    handle: &russh::client::Handle<ClientHandler>,
    timeout: Duration,
) -> AppResult<russh::Channel<russh::client::Msg>> {
    if handle.is_closed() {
        return Err(AppError::Ssh("ssh transport is closed".into()));
    }

    match tokio::time::timeout(timeout, handle.channel_open_session_manual_window(TERMINAL_WINDOW)).await {
        Ok(Ok(channel)) => Ok(channel),
        Ok(Err(error)) => Err(AppError::Ssh(format!("open channel: {error}"))),
        Err(_) => Err(AppError::Ssh(format!(
            "open channel timed out after {}s",
            timeout.as_secs()
        ))),
    }
}

/// TCP + SSH handshake + auth. Returns the russh Handle wrapped in Arc so it can be shared
/// between the shell pump, SFTP channel, and warm pool. 只有认证要 &mut，在这里就做完了。
pub(crate) async fn do_handshake(
    host: &str,
    port: u16,
    username: &str,
    credentials: Credentials<'_>,
    host_key: HostKeyCheck,
) -> AppResult<Arc<russh::client::Handle<ClientHandler>>> {
    let mut handle = connect_unauthenticated(host, port, host_key).await?;
    authenticate(&mut handle, host, port, username, credentials).await?;
    Ok(Arc::new(handle))
}

/// 私钥路径可以写成 `~/.ssh/id_ed25519`（界面里这样存，短，换台机器也能用）。
pub(crate) fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => std::path::PathBuf::from(path),
    }
}

/// TCP + SSH 握手（含主机密钥核对），还没认证。
pub(crate) async fn connect_unauthenticated(
    host: &str,
    port: u16,
    host_key: HostKeyCheck,
) -> AppResult<russh::client::Handle<ClientHandler>> {
    let addr = format!("{}:{}", host, port);
    let tcp = connect_tcp(&addr).await?;
    tcp.set_nodelay(true).ok();

    let rejected = Arc::new(std::sync::Mutex::new(None));
    let handler = ClientHandler { host: host.to_string(), port, host_key, rejected: rejected.clone() };
    russh::client::connect_stream(russh_config(), tcp, handler)
        .await
        .map_err(|e| match rejected.lock().unwrap().take() {
            Some(HostKeyRejection::Declined) => AppError::HostKeyDeclined,
            Some(HostKeyRejection::Untrusted { changed: true }) => AppError::Ssh(format!("host key for {addr} changed")),
            Some(HostKeyRejection::Untrusted { changed: false }) => AppError::Ssh(format!("host key for {addr} not trusted yet")),
            None => AppError::Ssh(format!("ssh handshake: {e}")),
        })
}

async fn authenticate(
    handle: &mut russh::client::Handle<ClientHandler>,
    host: &str,
    port: u16,
    username: &str,
    credentials: Credentials<'_>,
) -> AppResult<()> {
    let authed = match credentials {
        Credentials::Key { key: pk, passphrase } => {
            // 用户可能传路径或 PEM 内容。PEM 以 "-----BEGIN" 开头，否则按路径读文件。
            let pem = if pk.trim_start().starts_with("-----BEGIN") {
                pk.to_string()
            } else {
                tokio::fs::read_to_string(expand_home(pk))
                    .await
                    .map_err(|e| AppError::Ssh(format!("read key file {pk}: {e}")))?
            };
            let key = russh_keys::decode_secret_key(&pem, passphrase)
                .map_err(|e| AppError::Ssh(format!("decode key: {e}")))?;
            handle
                .authenticate_publickey(username, Arc::new(key))
                .await
                .map_err(|e| AppError::Ssh(format!("pubkey auth: {e}")))?
        }
        Credentials::Password(pw) => handle
            .authenticate_password(username, pw)
            .await
            .map_err(|e| AppError::Ssh(format!("password auth: {e}")))?,
        Credentials::Auto => {
            auto_auth::authenticate(handle, host, port, username).await?;
            true
        }
    };
    if !authed {
        return Err(AppError::Ssh("authentication rejected".into()));
    }
    Ok(())
}

pub struct ClientHandler {
    host: String,
    port: u16,
    host_key: HostKeyCheck,
    /// 主机密钥没过的原因，握手失败后 do_handshake 据此报错。
    rejected: Arc<std::sync::Mutex<Option<HostKeyRejection>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostKeyRejection {
    /// 用户在确认框里拒绝了。
    Declined,
    /// 没人可问（后台预热）。
    Untrusted { changed: bool },
}

#[async_trait::async_trait]
impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let verdict = known_hosts::verdict(&self.host, self.port, server_public_key, &self.host_key.ours, self.host_key.openssh.as_deref());
        if verdict == Verdict::Known {
            return Ok(true);
        }
        let changed = verdict == Verdict::Changed;
        if changed {
            tracing::warn!("host key for {}:{} differs from the recorded one", self.host, self.port);
        }
        let Some(prompt) = self.host_key.prompt.clone() else {
            *self.rejected.lock().unwrap() = Some(HostKeyRejection::Untrusted { changed });
            return Ok(false);
        };
        if !prompt(known_hosts::question(&self.host, self.port, server_public_key, changed)).await {
            *self.rejected.lock().unwrap() = Some(HostKeyRejection::Declined);
            return Ok(false);
        }
        if let Err(error) = known_hosts::trust(&self.host, self.port, server_public_key, &self.host_key.ours) {
            tracing::warn!("remember host key for {}:{}: {error}", self.host, self.port);
        }
        Ok(true)
    }
}

/// 连接并打开终端：先为界面开好 shell 写端（[`shell_channel`]）和输出去处，连上后终端线程直接收发。
/// 返回会话 id。`attempt_id` 用于 [`cancel_connect`]。
pub async fn connect(
    state: &AppState,
    details: ConnectionDetails,
    attempt_id: &str,
    shell: ShellHandle,
    shell_rx: ShellReceiver,
    sink: Arc<dyn TerminalSink>,
    host_key: HostKeyCheck,
) -> AppResult<String> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    let target = format!("{}:{}", details.host, details.port);

    tracing::info!("ssh connect start attempt={attempt_id} target={target}");
    state.pending_insert(attempt_id.to_string(), abort_handle).await;
    let result = Abortable::new(
        connect_session(state, details, session_id.clone(), shell, shell_rx, sink, host_key),
        abort_registration,
    )
    .await;
    state.pending_remove(attempt_id).await;

    match result {
        Ok(result) => {
            match &result {
                Ok(_) => tracing::info!("ssh connect success attempt={attempt_id} target={target}"),
                Err(error) => {
                    tracing::warn!("ssh connect failed attempt={attempt_id} target={target}: {error}")
                }
            }
            result.map(|()| session_id)
        }
        Err(_) => {
            tracing::info!("ssh connect cancelled attempt={attempt_id} target={target}");
            if let Some(session) = state.remove(&session_id).await {
                session.shell.close();
            }
            Err(AppError::Other("connection cancelled".into()))
        }
    }
}

async fn connect_session(
    state: &AppState,
    details: ConnectionDetails,
    session_id: String,
    shell: ShellHandle,
    shell_rx: ShellReceiver,
    sink: Arc<dyn TerminalSink>,
    host_key: HostKeyCheck,
) -> AppResult<()> {
    let password = details.password.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "password"))
    });
    let passphrase = details.passphrase.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "passphrase"))
    });
    let credentials = match details.auth {
        AuthKind::Password => Credentials::Password(
            password.as_deref().ok_or_else(|| AppError::Ssh("no password provided".into()))?,
        ),
        AuthKind::Key => Credentials::Key {
            key: details.private_key.as_deref().ok_or_else(|| AppError::Ssh("no private key provided".into()))?,
            passphrase: passphrase.as_deref(),
        },
        AuthKind::Auto => Credentials::Auto,
    };

    let key = warm_key(&details.host, details.port, &details.username);
    let (mut handle_arc, used_warm_connection) = if let Some(warm) = state.warm_take(&key).await {
        (warm.handle, true)
    } else {
        (
            do_handshake(
                &details.host,
                details.port,
                &details.username,
                credentials,
                host_key.clone(),
            )
            .await?,
            false,
        )
    };

    let channel = match open_terminal_channel(&handle_arc, Duration::from_secs(2)).await {
        Ok(channel) => channel,
        Err(error) if used_warm_connection => {
            tracing::warn!("prewarmed connection {key} is stale ({error}); reconnecting");
            handle_arc = do_handshake(
                &details.host,
                details.port,
                &details.username,
                credentials,
                host_key,
            )
            .await?;
            open_terminal_channel(&handle_arc, Duration::from_secs(5)).await?
        }
        Err(error) => return Err(error),
    };
    channel.request_pty(false, "xterm-256color", details.cols, details.rows, 0, 0, &[]).await
        .map_err(|e| AppError::Ssh(format!("request pty: {e}")))?;
    channel.request_shell(false).await
        .map_err(|e| AppError::Ssh(format!("request shell: {e}")))?;

    let session = Arc::new(SshSession {
        id: session_id.clone(),
        host: details.host.clone(),
        username: details.username.clone(),
        connection_id: details.id.clone(),
        warm_key: key,
        handle: handle_arc.clone(),
        sftp: Mutex::new(None),
        passwd_cache: Mutex::new(None),
        elevation: Mutex::new(None),
        group_cache: Mutex::new(None),
        shell,
    });

    state.insert(session_id.clone(), session.clone()).await;

    let sessions = state.sessions.clone();
    tokio::spawn(async move {
        run_channel(sessions, session_id, session, channel, shell_rx.0, sink).await;
    });
    Ok(())
}

pub async fn cancel_connect(state: &AppState, attempt_id: &str) -> bool {
    let cancelled = state.pending_abort(attempt_id).await;
    tracing::info!("ssh cancel attempt={attempt_id} matched={cancelled}");
    cancelled
}

async fn validate_session(session: &SshSession) -> bool {
    let probe = async {
        if session.handle.is_closed() {
            return false;
        }

        match session.handle.channel_open_session().await {
            Ok(channel) => {
                let _ = channel.close().await;
                true
            }
            Err(error) => {
                let transport_open = !session.handle.is_closed();
                if transport_open {
                    tracing::warn!("session probe rejected by live server: {error}");
                }
                transport_open
            }
        }
    };

    tokio::time::timeout(Duration::from_secs(2), probe)
        .await
        .unwrap_or(false)
}

/// 会话还活着吗（2s 内能在连接上开出通道）。死了就从会话表移除并关掉终端通道。
/// 往返延迟（标签栏显示）：开一个会话通道再关掉，正好一个来回。5 秒没回来算超时（None）。
pub async fn latency(state: &AppState, session_id: &str) -> Option<Duration> {
    let session = state.get(session_id).await?;
    round_trip(&session.handle).await
}

async fn round_trip<H: russh::client::Handler>(handle: &russh::client::Handle<H>) -> Option<Duration> {
    if handle.is_closed() {
        return None;
    }
    let started = std::time::Instant::now();
    let channel = tokio::time::timeout(Duration::from_secs(5), handle.channel_open_session()).await.ok()?.ok()?;
    let elapsed = started.elapsed();
    let _ = channel.close().await;
    Some(elapsed)
}

pub async fn validate(state: &AppState, session_id: &str) -> bool {
    let Some(session) = state.get(session_id).await else {
        return false;
    };

    if validate_session(&session).await {
        return true;
    }

    tracing::warn!("discarding stale ssh session {session_id}");
    if let Some(stale) = state.remove(session_id).await {
        stale.shell.close();
    }
    false
}

async fn run_channel(
    sessions: Arc<Mutex<HashMap<String, Arc<SshSession>>>>,
    session_id: String,
    session: Arc<SshSession>,
    mut channel: russh::Channel<russh::client::Msg>,
    mut out_rx: mpsc::UnboundedReceiver<Outbound>,
    sink: Arc<dyn TerminalSink>,
) {
    let mut exit_code: Option<u32> = None;
    let exit_reason: String;
    let mut window = TerminalWindow::default();
    loop {
        tokio::select! {
            // 先处理写端：刷屏时按键（^C）不能排在一长串输出后面。
            biased;
            cmd = out_rx.recv() => {
                match cmd {
                    Some(Outbound::Data(bytes)) => {
                        if let Err(e) = channel.data(&bytes[..]).await {
                            tracing::warn!("ssh write failed: {e}");
                            exit_reason = format!("write_error: {e}");
                            break;
                        }
                    }
                    Some(Outbound::Resize { cols, rows }) => {
                        if let Err(e) = channel.window_change(cols, rows, 0, 0).await {
                            tracing::warn!("ssh resize failed: {e}");
                        }
                    }
                    Some(Outbound::Ack(bytes)) => {
                        if let Some(grant) = window.acked(bytes) {
                            let _ = channel.grant_window(grant).await;
                        }
                    }
                    Some(Outbound::Close) => {
                        exit_reason = "client_close".into();
                        break;
                    }
                    None => {
                        exit_reason = "outbound_closed".into();
                        break;
                    }
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(russh::ChannelMsg::Data { data })
                    | Some(russh::ChannelMsg::ExtendedData { data, ext: _ }) => {
                        window.emitted(data.len() as u32);
                        sink.data(&data);
                    }
                    Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                        // do NOT break on exit_status alone — server may keep channel open for cleanup
                        exit_code = Some(exit_status);
                    }
                    Some(russh::ChannelMsg::Eof) => {
                        exit_reason = "server_eof".into();
                        break;
                    }
                    Some(russh::ChannelMsg::Close) => {
                        exit_reason = "server_close".into();
                        break;
                    }
                    None => {
                        exit_reason = "channel_stream_end".into();
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = session.handle.disconnect(russh::Disconnect::ByApplication, "client closed", "en").await;
    // 退出路径（含 server EOF / exit 命令）也要从会话表移除，否则死 session
    // 连同 Arc handle、缓存的 SFTP / passwd / group map 永久泄漏。主动 disconnect 已先 remove 过，这里是幂等的。
    sessions.lock().await.remove(&session_id);
    sink.closed(exit_code, exit_reason);
}

pub async fn disconnect(state: &AppState, session_id: &str) {
    if let Some(session) = state.remove(session_id).await {
        session.shell.close();
    }
}

/// 把该会话所属连接保存的密码解密后写入 SSH 通道并自动回车提交（用于 sudo / su 等提示）。
/// `kind` 选择填哪一份：None/"password" = 连接主密码；"acct:<账号>" = 该连接下保存的账号密码
/// （同一台服务器常有多个账号）。界面仅在终端出现密码提示时才显示按钮，不会误填。
/// 密码全程留在后端，不经过界面。无对应密码时返回错误。
pub async fn fill_password(state: &AppState, session_id: &str, kind: Option<String>) -> AppResult<()> {
    let session = state.get(session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.to_string()))?;
    let cid = session.connection_id.clone()
        .ok_or_else(|| AppError::Ssh("当前连接未保存，无密码可填充".into()))?;
    let kind = kind.unwrap_or_else(|| "password".into());
    let password = load_secret(&cid, &kind)
        .ok_or_else(|| AppError::Ssh("未找到对应的已保存密码".into()))?;
    let mut bytes = password.into_bytes();
    bytes.push(b'\r');
    session.shell.write(bytes);
    Ok(())
}

/// 终端通道的 SSH 接收窗口上限：服务端最多比终端线程已解析完的多发这么多。
/// 刷屏时积压在 russh 队列和终端线程里的、^C 之后还要滚完的尾巴都以它为界。
/// 交互输出远小于它，打字 / vim 不会因为没确认而停下。
pub(crate) const TERMINAL_WINDOW: u32 = 2 * 1024 * 1024;
/// 攒够这么多再补窗口，免得每个小包都发一次 WINDOW_ADJUST。
const GRANT_BATCH: u32 = 64 * 1024;

/// 终端通道的窗口记账（russh 补丁的手动窗口模式，见 vendor/README.md）。服务端还能发的量 =
/// TERMINAL_WINDOW − 已收到但还没补回的字节：交给 TerminalSink 的字节，等终端线程解析完 ack 才算消费。
/// 确认量按「已交出未确认」封顶：重复确认不会把窗口撑大。
#[derive(Default)]
struct TerminalWindow {
    unacked: u64,
    ungranted: u32,
}

impl TerminalWindow {
    /// 交给终端线程、等确认的字节。
    fn emitted(&mut self, raw: u32) {
        self.unacked += u64::from(raw);
    }

    /// 终端线程确认。返回此刻该补的窗口。
    fn acked(&mut self, bytes: u32) -> Option<u32> {
        let bytes = u64::from(bytes).min(self.unacked);
        self.unacked -= bytes;
        self.consumed(bytes as u32)
    }

    fn consumed(&mut self, bytes: u32) -> Option<u32> {
        self.ungranted = self.ungranted.saturating_add(bytes);
        (self.ungranted >= GRANT_BATCH).then(|| std::mem::take(&mut self.ungranted))
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_home, is_transient_connect_error, TerminalWindow, GRANT_BATCH, TERMINAL_WINDOW};

    #[test]
    fn key_paths_may_start_from_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_home("~/.ssh/id_ed25519"), home.join(".ssh/id_ed25519"));
        assert_eq!(expand_home("/opt/keys/deploy"), std::path::PathBuf::from("/opt/keys/deploy"));
        assert_eq!(expand_home("~user/key"), std::path::PathBuf::from("~user/key"), "别人的家目录不展开");
    }
    use std::time::Duration;

    /// 服务端此刻还能在终端通道上发多少：初始窗口 + 已补 − 已收。
    struct Peer {
        window: TerminalWindow,
        received: u64,
        granted: u64,
    }

    impl Peer {
        fn new() -> Self {
            Peer { window: TerminalWindow::default(), received: 0, granted: 0 }
        }
        fn credit(&self) -> u64 {
            u64::from(TERMINAL_WINDOW) + self.granted - self.received
        }
        fn grant(&mut self, grant: Option<u32>) {
            self.granted += u64::from(grant.unwrap_or(0));
        }
    }

    #[test]
    fn emitted_output_waits_for_the_renderer_ack() {
        let mut peer = Peer::new();
        // 刷屏：终端线程不确认，服务端最多再发满一个窗口就停
        while peer.credit() > 0 {
            let n = peer.credit().min(32 * 1024);
            peer.received += n;
            peer.window.emitted(n as u32);
        }
        assert_eq!(peer.received, u64::from(TERMINAL_WINDOW));
        // 解析完 128KB → 补回 128KB
        let grant = peer.window.acked(128 * 1024);
        assert_eq!(grant, Some(128 * 1024));
        peer.grant(grant);
        assert_eq!(peer.credit(), 128 * 1024);
    }

    #[test]
    fn small_acks_are_batched_into_one_window_adjust() {
        let mut window = TerminalWindow::default();
        window.emitted(GRANT_BATCH);
        assert_eq!(window.acked(GRANT_BATCH / 2), None);
        assert_eq!(window.acked(GRANT_BATCH / 2), Some(GRANT_BATCH));
    }

    #[test]
    fn over_acks_cannot_grow_the_window() {
        // 多确认（重复、超量）时，补回去的总量不能超过真正交出去的。
        let mut peer = Peer::new();
        peer.received += 300 * 1024;
        peer.window.emitted(300 * 1024);
        let grant = peer.window.acked(u32::MAX);
        peer.grant(grant);
        let grant = peer.window.acked(200 * 1024);
        peer.grant(grant);
        assert_eq!(peer.granted, 300 * 1024);
        assert_eq!(peer.credit(), u64::from(TERMINAL_WINDOW));
    }

    #[test]
    fn terminal_window_stays_bounded_under_any_ack_pattern() {
        // 伪随机地交错：收数据、终端线程解析完确认、偶尔超量确认（u32::MAX）。
        // 超量确认之前交出的字节视作作废，之后不再确认。
        // 任何时刻：服务端余量 + 还会确认的字节 ≤ 终端窗口；算上作废的也不超过两个窗口。
        let mut peer = Peer::new();
        let (mut fresh, mut stale): (u64, u64) = (0, 0);
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for step in 0..20_000 {
            let r = next();
            match r % 4 {
                0 | 1 if peer.credit() > 0 => {
                    let n = ((r >> 8) % 40_000 + 1).min(peer.credit());
                    peer.received += n;
                    peer.window.emitted(n as u32);
                    fresh += n;
                }
                2 => {
                    let mut n = (r >> 8) % 200_000;
                    let old = n.min(stale);
                    stale -= old;
                    n = (n - old).min(fresh);
                    fresh -= n;
                    let grant = peer.window.acked(n as u32);
                    peer.grant(grant);
                }
                3 if r % 97 == 0 => {
                    stale += std::mem::take(&mut fresh);
                    let grant = peer.window.acked(u32::MAX);
                    peer.grant(grant);
                }
                _ => {}
            }
            assert!(peer.credit() + fresh <= u64::from(TERMINAL_WINDOW), "step {step}");
            assert!(peer.credit() + fresh + stale <= 2 * u64::from(TERMINAL_WINDOW), "step {step}");
        }
    }

    /// 真实 sshd 上的主机身份确认：第一次问、拒绝就不连；确认后记下，再连不问；密钥变了再问并标明「变了」。
    /// 任意 sshd 即可（认证在核对之后，这里不需要能登录；OpenSSH 9.8+ 要配 `PerSourcePenalties no`）：
    /// `SSHL_HOSTKEY_IT=127.0.0.1:2222 SSHL_HOSTKEY_IT_FP=SHA256:… cargo test --lib host_key_confirmation_against_real_sshd -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_HOSTKEY_IT=host:port 指向任一 sshd"]
    async fn host_key_confirmation_against_real_sshd() {
        use crate::error::AppError;
        use crate::known_hosts::{HostKeyCheck, HostKeyQuestion};
        use std::sync::Arc;
        let Ok(target) = std::env::var("SSHL_HOSTKEY_IT") else { return };
        let (host, port) = target.split_once(':').unwrap();
        let port: u16 = port.parse().unwrap();
        let dir = std::env::temp_dir().join(format!("sshl-hostkey-it-{}", uuid::Uuid::new_v4()));
        let ours = dir.join("known_hosts");
        let asked: Arc<std::sync::Mutex<Vec<HostKeyQuestion>>> = Arc::default();
        let check = |answer: bool| {
            let asked = asked.clone();
            HostKeyCheck {
                ours: ours.clone(),
                openssh: None,
                prompt: Some(Arc::new(move |q| {
                    asked.lock().unwrap().push(q);
                    Box::pin(async move { answer })
                })),
            }
        };
        let handshake = |check| super::do_handshake(host, port, "nobody", super::Credentials::Password("x"), check);

        let declined = handshake(check(false)).await.err().unwrap();
        assert!(matches!(declined, AppError::HostKeyDeclined), "{declined}");
        assert!(!ours.exists(), "拒绝了不记");
        let first = asked.lock().unwrap().pop().unwrap();
        assert!(!first.changed);
        if let Ok(expected) = std::env::var("SSHL_HOSTKEY_IT_FP") {
            assert_eq!(first.fingerprint, expected, "指纹同 ssh-keygen -lf");
        }

        // 确认：握手继续（停在认证：nobody / 错密码），密钥记下
        let err = handshake(check(true)).await.err().unwrap();
        assert!(!matches!(err, AppError::HostKeyDeclined) && err.to_string().contains("auth"), "{err}");
        assert_eq!(asked.lock().unwrap().len(), 1);
        asked.lock().unwrap().clear();
        handshake(check(false)).await.err().map(|e| assert!(!matches!(e, AppError::HostKeyDeclined), "已信任不该再问：{e}"));
        assert!(asked.lock().unwrap().is_empty());

        // 没人可问（后台预热）：未知主机直接不连
        let warm = HostKeyCheck { ours: dir.join("empty"), openssh: None, prompt: None };
        assert!(handshake(warm).await.err().unwrap().to_string().contains("not trusted yet"));

        // 记录里换成另一把同类型的密钥：问，且标明变了
        let recorded = std::fs::read_to_string(&ours).unwrap();
        let kind = recorded.split_whitespace().nth(1).unwrap().to_string();
        let other = match kind.as_str() {
            "ssh-ed25519" => russh_keys::key::KeyPair::generate_ed25519().unwrap(),
            other => panic!("测试服务器的主机密钥类型 {other}：本测试只造 ed25519"),
        };
        std::fs::remove_file(&ours).unwrap();
        crate::known_hosts::trust(host, port, &other.clone_public_key().unwrap(), &ours).unwrap();
        assert!(matches!(handshake(check(false)).await.err().unwrap(), AppError::HostKeyDeclined));
        assert!(asked.lock().unwrap().pop().unwrap().changed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实 sshd 上量往返延迟：本机连接应在几毫秒内；连接断了是 None。
    /// `SSHL_AUTO_AUTH_IT=127.0.0.1:2222 SSHL_AUTO_AUTH_IT_USER=me SSHL_AUTO_AUTH_IT_KEY=/path/key cargo test --lib round_trip_against_real_sshd -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_AUTO_AUTH_IT / _USER / _KEY 指向认可该私钥的 sshd"]
    async fn round_trip_against_real_sshd() {
        let (Ok(target), Ok(user), Ok(key)) =
            (std::env::var("SSHL_AUTO_AUTH_IT"), std::env::var("SSHL_AUTO_AUTH_IT_USER"), std::env::var("SSHL_AUTO_AUTH_IT_KEY"))
        else {
            return;
        };
        let (host, port) = target.split_once(':').unwrap();
        let handle = super::do_handshake(host, port.parse().unwrap(), &user, super::Credentials::Key { key: &key, passphrase: None }, super::HostKeyCheck::trust_any_for_tests())
            .await
            .unwrap();
        let rtt = super::round_trip(&handle).await.expect("连着就量得到");
        assert!(rtt < Duration::from_secs(1), "{rtt:?}");
        handle.disconnect(russh::Disconnect::ByApplication, "", "").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(super::round_trip(&handle).await, None, "断了就没有");
    }

    /// 真实 sshd：终端通道不确认就停在一个窗口内，同连接的其他通道照常跑；确认后继续流动。
    /// 容器见 tests/fixtures-sudo.Dockerfile（以 alice 登录）：
    /// `SSHL_SFTP_IT=127.0.0.1:2233 cargo test --lib terminal_window_against_real_sshd -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "需要 SSHL_SFTP_IT=host:port 指向 sshd 测试容器"]
    async fn terminal_window_against_real_sshd() {
        use russh::ChannelMsg;
        let Ok(target) = std::env::var("SSHL_SFTP_IT") else { return };
        let (host, port) = target.split_once(':').unwrap();
        let h = super::do_handshake(host, port.parse().unwrap(), "alice", super::Credentials::Password("alicepw"), super::HostKeyCheck::trust_any_for_tests()).await.unwrap();
        let mut term = super::open_terminal_channel(&h, Duration::from_secs(5)).await.unwrap();
        term.request_pty(false, "xterm-256color", 120, 40, 0, 0, &[]).await.unwrap();
        term.request_shell(false).await.unwrap();
        term.data(&b"head -c 64M /dev/zero | tr '\\0' x\r"[..]).await.unwrap();

        // 读到不再有数据（服务端窗口耗尽）为止，一个字节都不确认
        async fn drain(term: &mut russh::Channel<russh::client::Msg>) -> u64 {
            let mut got = 0u64;
            while let Ok(msg) = tokio::time::timeout(Duration::from_millis(800), term.wait()).await {
                match msg {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => got += data.len() as u64,
                    Some(_) => {}
                    None => panic!("终端通道被关闭"),
                }
            }
            got
        }
        let got = drain(&mut term).await;
        eprintln!("terminal received {got} bytes without acks (window {TERMINAL_WINDOW})");
        assert!(got > u64::from(TERMINAL_WINDOW) / 2, "刷屏应当先填满窗口，只收到 {got}");
        assert!(got <= u64::from(TERMINAL_WINDOW), "没确认却收到 {got} 字节，超出终端窗口");

        // 终端停着，同一连接上开通道、读 16MB 照常（russh 读循环没被挡住）
        let side = async {
            let mut ch = h.channel_open_session().await.unwrap();
            ch.exec(true, "head -c 16M /dev/zero").await.unwrap();
            let mut n = 0u64;
            while let Some(msg) = ch.wait().await {
                match msg {
                    ChannelMsg::Data { data } => n += data.len() as u64,
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            n
        };
        let side = tokio::time::timeout(Duration::from_secs(10), side).await.expect("同连接的其他通道被终端拖住了");
        assert_eq!(side, 16 * 1024 * 1024);

        // 按确认补窗口后继续流动，且仍不超过补回的量
        let mut window = TerminalWindow::default();
        window.emitted(got as u32);
        let grant = window.acked(got as u32).unwrap();
        term.grant_window(grant).await.unwrap();
        let more = drain(&mut term).await;
        assert!(more > 0 && more <= u64::from(grant), "补了 {grant}，又收到 {more}");

        term.data(&b"\x03exit\r"[..]).await.unwrap();
        let _ = term.grant_window(TERMINAL_WINDOW).await;
        let _ = h.disconnect(russh::Disconnect::ByApplication, "", "en").await;
    }

    #[test]
    fn treats_macos_host_down_as_transient() {
        let error = std::io::Error::from_raw_os_error(64);
        assert!(is_transient_connect_error(&error));
    }
}

// SSH backend — russh client.
// SshSession owns: Handle (shared with SFTP), shell-task outbound sender, output buffer.
// The shell pump task owns only the Channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::future::{AbortHandle, Abortable};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex, Notify};

use crate::config_store::load_secret;
use crate::error::{AppError, AppResult};
use crate::state::{AppState, WarmConn};

const BUFFER_CAP: usize = 100 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDetails {
    pub id: Option<String>,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    #[serde(default = "default_cols")]
    pub cols: u32,
    #[serde(default = "default_rows")]
    pub rows: u32,
}

fn default_port() -> u16 { 22 }
fn default_cols() -> u32 { 80 }
fn default_rows() -> u32 { 24 }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectResult {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshDataEvent {
    pub session_id: String,
    pub data: String,
    /// 这段输出对应的原始 SSH 字节数。前端 xterm 解析完要用 ssh_ack_data 确认，
    /// 后端据此补终端通道的 SSH 窗口（见 TerminalWindow）。激活快照为 0，不用确认。
    pub bytes: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshClosedEvent {
    pub session_id: String,
    pub code: Option<u32>,
    pub reason: String,
}

enum Outbound {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    /// 前端确认已解析的原始字节数（ssh_ack_data）。
    Ack(u32),
    Close,
}

pub struct SshSession {
    // Identity fields retained for diagnostics/logging; not all are read yet.
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub host: String,
    #[allow(dead_code)]
    pub username: String,
    /// 所属保存连接的 id（来自 ~/.sshl/connections.json）。ssh_fill_password 用它
    /// 从加密库解密密码并直接写入通道 —— 密码全程不经过渲染层（JS）。
    pub connection_id: Option<String>,
    /// host:port:user —— 预热池据此跳过已有会话的主机（见 AppState::active_warm_keys）。
    pub warm_key: String,
    pub buffer: Mutex<String>,
    /// Shared SSH handle（shell / SFTP / exec 共用）。认证之后用到的 is_closed、channel_open_session、
    /// disconnect 都是 &self、各等各的通道确认，不加锁：加锁会把同一会话上的开通道串行化。
    pub handle: Arc<russh::client::Handle<ClientHandler>>,
    /// Cached SFTP session — reused across file ops to avoid sshd MaxSessions exhaustion.
    /// Mirrors the Electron-side fix from commits 8cb552c / a819e15.
    pub sftp: Mutex<Option<Arc<russh_sftp::client::SftpSession>>>,
    /// Lazily loaded uid→username map (from /etc/passwd).
    pub passwd_cache: Mutex<Option<Arc<HashMap<u32, String>>>>,
    /// Lazily loaded gid→groupname map (from /etc/group).
    pub group_cache: Mutex<Option<Arc<HashMap<u32, String>>>>,
    /// 终端切换了用户（su / sudo -i）后，文件管理以该用户身份运行：SFTP 换成以他身份起的
    /// sftp-server，删除 / chmod / chown 也以他身份执行。None = SSH 登录用户。
    pub elevation: Mutex<Option<Arc<crate::terminal_user::Elevation>>>,
    /// Renderer must explicitly activate the session to start receiving ssh:data events.
    /// Until then, output is silently buffered; ssh_activate_session emits the buffered
    /// prelude as the first ssh:data event so snapshot and live data stay in one
    /// ordered channel (avoids banner double-render and prompt/banner reordering).
    pub activated: AtomicBool,
    /// Notified the first time shell data lands in `buffer`. `ssh_connect` awaits this
    /// (with a 400ms cap) so the welcome/PS1 is visible the instant the loader closes —
    /// mirrors the Electron-side waitForFirstData fix from commit c51afa0.
    pub first_data: Arc<Notify>,
    outbound: mpsc::UnboundedSender<Outbound>,
}

impl SshSession {
    /// Append a chunk to the buffer. Returns whether the session was already
    /// activated at the instant the chunk landed — i.e. whether the caller
    /// should also emit it to the renderer. The `activated` flag is read while
    /// holding the buffer lock so it is atomic w.r.t. `ssh_activate_session`'s
    /// snapshot: a chunk is delivered exactly once — either via the activation
    /// snapshot or via `ssh:data`, never both, never neither. This closes the
    /// first-prompt gap where a late PS1 (slow MOTD) landed between the
    /// renderer's buffer fetch and activation and was lost.
    pub async fn append_to_buffer(&self, chunk: &str) -> bool {
        let mut buf = self.buffer.lock().await;
        let was_empty = buf.is_empty();
        buf.push_str(chunk);
        if buf.len() > BUFFER_CAP {
            let mut overflow = buf.len() - BUFFER_CAP;
            // 向后推到 UTF-8 字符边界，避免 drain 切断多字节字符触发 panic
            while overflow < buf.len() && !buf.is_char_boundary(overflow) {
                overflow += 1;
            }
            buf.drain(..overflow);
        }
        let activated = self.activated.load(Ordering::Relaxed);
        drop(buf);
        if was_empty {
            self.first_data.notify_waiters();
        }
        activated
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
    password: Option<&str>,
    private_key: Option<&str>,
    passphrase: Option<&str>,
) -> AppResult<Arc<russh::client::Handle<ClientHandler>>> {
    let addr = format!("{}:{}", host, port);
    let tcp = connect_tcp(&addr).await?;
    tcp.set_nodelay(true).ok();

    let handler = ClientHandler { host: host.to_string(), port };
    let mut handle = russh::client::connect_stream(russh_config(), tcp, handler)
        .await
        .map_err(|e| AppError::Ssh(format!("ssh handshake: {e}")))?;

    let authed = if let Some(pk) = private_key {
        // 用户可能传路径或 PEM 内容。PEM 以 "-----BEGIN" 开头，否则按路径读文件。
        let pem = if pk.trim_start().starts_with("-----BEGIN") {
            pk.to_string()
        } else {
            tokio::fs::read_to_string(pk)
                .await
                .map_err(|e| AppError::Ssh(format!("read key file {pk}: {e}")))?
        };
        let key = russh_keys::decode_secret_key(&pem, passphrase)
            .map_err(|e| AppError::Ssh(format!("decode key: {e}")))?;
        handle
            .authenticate_publickey(username, Arc::new(key))
            .await
            .map_err(|e| AppError::Ssh(format!("pubkey auth: {e}")))?
    } else if let Some(pw) = password {
        handle
            .authenticate_password(username, pw)
            .await
            .map_err(|e| AppError::Ssh(format!("password auth: {e}")))?
    } else {
        return Err(AppError::Ssh("no password or private key provided".into()));
    };
    if !authed {
        return Err(AppError::Ssh("authentication rejected".into()));
    }
    Ok(Arc::new(handle))
}

pub struct ClientHandler {
    host: String,
    port: u16,
}

#[async_trait::async_trait]
impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TOFU known_hosts 校验：
        // - 已知且匹配 → 接受
        // - 已知但密钥变更（KeyChanged）→ 拒绝（可能的中间人攻击）
        // - 未知 / known_hosts 文件不存在 → 首次信任并写入 ~/.ssh/known_hosts
        match russh_keys::check_known_hosts(&self.host, self.port, server_public_key) {
            Ok(true) => Ok(true),
            Ok(false) => {
                if let Err(e) =
                    russh_keys::learn_known_hosts(&self.host, self.port, server_public_key)
                {
                    tracing::warn!("learn_known_hosts {}:{} failed: {e}", self.host, self.port);
                }
                Ok(true)
            }
            Err(russh_keys::Error::KeyChanged { line }) => {
                tracing::error!(
                    "known_hosts: host key CHANGED for {}:{} (entry line {line}) — rejecting connection",
                    self.host,
                    self.port
                );
                Ok(false)
            }
            Err(e) => {
                // known_hosts 文件缺失等非密钥变更错误：按未知主机处理，TOFU 学习。
                tracing::warn!(
                    "known_hosts read issue for {}:{}: {e}; treating as new host",
                    self.host,
                    self.port
                );
                let _ = russh_keys::learn_known_hosts(&self.host, self.port, server_public_key);
                Ok(true)
            }
        }
    }
}

#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    details: ConnectionDetails,
    attempt_id: Option<String>,
) -> AppResult<ConnectResult> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let attempt_id = attempt_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    let target = format!("{}:{}", details.host, details.port);

    tracing::info!("ssh connect start attempt={attempt_id} target={target}");
    state.pending_insert(attempt_id.clone(), abort_handle).await;
    let result = Abortable::new(
        connect_session(app, state.inner(), details, session_id.clone()),
        abort_registration,
    )
    .await;
    state.pending_remove(&attempt_id).await;

    match result {
        Ok(result) => {
            match &result {
                Ok(_) => tracing::info!("ssh connect success attempt={attempt_id} target={target}"),
                Err(error) => {
                    tracing::warn!("ssh connect failed attempt={attempt_id} target={target}: {error}")
                }
            }
            result
        }
        Err(_) => {
            tracing::info!("ssh connect cancelled attempt={attempt_id} target={target}");
            if let Some(session) = state.remove(&session_id).await {
                let _ = session.outbound.send(Outbound::Close);
            }
            Err(AppError::Other("connection cancelled".into()))
        }
    }
}

async fn connect_session(
    app: AppHandle,
    state: &AppState,
    details: ConnectionDetails,
    session_id: String,
) -> AppResult<ConnectResult> {
    let password = details.password.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "password"))
    });
    let passphrase = details.passphrase.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "passphrase"))
    });

    let key = warm_key(&details.host, details.port, &details.username);
    let (mut handle_arc, used_warm_connection) = if let Some(warm) = state.warm_take(&key).await {
        (warm.handle, true)
    } else {
        (
            do_handshake(
                &details.host,
                details.port,
                &details.username,
                password.as_deref(),
                details.private_key.as_deref(),
                passphrase.as_deref(),
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
                password.as_deref(),
                details.private_key.as_deref(),
                passphrase.as_deref(),
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

    let (out_tx, out_rx) = mpsc::unbounded_channel::<Outbound>();
    let first_data = Arc::new(Notify::new());

    let session = Arc::new(SshSession {
        id: session_id.clone(),
        host: details.host.clone(),
        username: details.username.clone(),
        connection_id: details.id.clone(),
        warm_key: key,
        buffer: Mutex::new(String::new()),
        handle: handle_arc.clone(),
        sftp: Mutex::new(None),
        passwd_cache: Mutex::new(None),
        elevation: Mutex::new(None),
        group_cache: Mutex::new(None),
        activated: AtomicBool::new(false),
        first_data: first_data.clone(),
        outbound: out_tx,
    });

    state.insert(session_id.clone(), session.clone()).await;

    let app_for_task = app.clone();
    let session_for_task = session.clone();
    let sid_for_task = session_id.clone();
    tokio::spawn(async move {
        run_channel(app_for_task, sid_for_task, session_for_task, channel, out_rx).await;
    });

    // waitForFirstData: gate the response on welcome/PS1 landing in buffer so the renderer
    // has something to write the moment the loader closes. 400ms cap matches Electron.
    let notified = first_data.notified();
    tokio::pin!(notified);
    tokio::select! {
        _ = &mut notified => {}
        _ = tokio::time::sleep(Duration::from_millis(400)) => {}
    }

    Ok(ConnectResult { session_id })
}

#[tauri::command]
pub async fn ssh_cancel_connect(
    state: State<'_, AppState>,
    attempt_id: String,
) -> AppResult<bool> {
    let cancelled = state.pending_abort(&attempt_id).await;
    tracing::info!("ssh cancel attempt={attempt_id} matched={cancelled}");
    Ok(cancelled)
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

#[tauri::command]
pub async fn ssh_validate_session(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<bool> {
    let Some(session) = state.get(&session_id).await else {
        return Ok(false);
    };

    if validate_session(&session).await {
        return Ok(true);
    }

    tracing::warn!("discarding stale ssh session {session_id}");
    if let Some(stale) = state.remove(&session_id).await {
        let _ = stale.outbound.send(Outbound::Close);
    }
    Ok(false)
}

/// Prewarm a single connection: handshake + auth, park the Handle in the warm pool.
/// Called at startup for every saved connection that has usable credentials.
#[tauri::command]
pub async fn ssh_prewarm(
    state: State<'_, AppState>,
    details: ConnectionDetails,
) -> AppResult<bool> {
    let key = warm_key(&details.host, details.port, &details.username);
    if state.warm_contains(&key).await {
        return Ok(true);
    }
    let password = details.password.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "password"))
    });
    let passphrase = details.passphrase.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "passphrase"))
    });
    if password.is_none() && details.private_key.is_none() {
        return Ok(false);
    }
    let handle = do_handshake(
        &details.host,
        details.port,
        &details.username,
        password.as_deref(),
        details.private_key.as_deref(),
        passphrase.as_deref(),
    )
    .await?;
    state
        .warm_insert(key, WarmConn { handle })
        .await;
    Ok(true)
}

async fn run_channel(
    app: AppHandle,
    session_id: String,
    session: Arc<SshSession>,
    mut channel: russh::Channel<russh::client::Msg>,
    mut out_rx: mpsc::UnboundedReceiver<Outbound>,
) {
    let mut exit_code: Option<u32> = None;
    let exit_reason: String;

    let mut pending = PendingOutput { text: String::with_capacity(COALESCE_MAX), raw: 0 };
    let mut window = TerminalWindow::default();
    // 只解出半个多字节字符的 chunk 不产出文本，它的原始字节记到下一段有文本的输出上。
    let mut raw_carried: u32 = 0;
    let mut flush_deadline: Option<tokio::time::Instant> = None;
    let mut last_emit = tokio::time::Instant::now() - COALESCE_WINDOW;
    // 跨 chunk UTF-8 残字节暂存：SSH 数据按任意字节边界到达，一个多字节字符
    // （中文/emoji）可能被切在两个 chunk 之间。stdout / stderr 是独立字节流，各自一份。
    let mut utf8_out: Vec<u8> = Vec::new();
    let mut utf8_err: Vec<u8> = Vec::new();
    loop {
        tokio::select! {
            biased;
            _ = async {
                if let Some(d) = flush_deadline {
                    tokio::time::sleep_until(d).await
                } else {
                    std::future::pending::<()>().await
                }
            }, if flush_deadline.is_some() => {
                if !pending.text.is_empty() {
                    emit_pending(&app, &session_id, &mut pending, &mut window);
                    last_emit = tokio::time::Instant::now();
                }
                flush_deadline = None;
            }
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
                let (carry, data) = match msg {
                    Some(russh::ChannelMsg::Data { data }) => (&mut utf8_out, data),
                    Some(russh::ChannelMsg::ExtendedData { data, ext: _ }) => (&mut utf8_err, data),
                    Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                        exit_code = Some(exit_status);
                        // do NOT break on exit_status alone — server may keep channel open for cleanup
                        continue;
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
                    _ => continue,
                };
                raw_carried = raw_carried.saturating_add(data.len() as u32);
                let s = decode_stream_chunk(carry, &data);
                if s.is_empty() {
                    continue;
                }
                let raw = std::mem::take(&mut raw_carried);
                if session.append_to_buffer(&s).await {
                    queue_output(&app, &session_id, &s, raw, &mut pending, &mut window, &mut flush_deadline, &mut last_emit);
                } else if let Some(grant) = window.buffered(raw) {
                    // 激活前只进缓冲、前端还没在听：收到即算消费，首屏缓冲阶段不能被流控卡住。
                    let _ = channel.grant_window(grant).await;
                }
            }
        }
    }

    // 退出前刷出残余 pending
    if !pending.text.is_empty() {
        emit_pending(&app, &session_id, &mut pending, &mut window);
    }

    let _ = session.handle.disconnect(russh::Disconnect::ByApplication, "client closed", "en").await;
    // 退出路径（含 server EOF / exit 命令）也要从 AppState 移除，否则死 session
    // 连同 Arc handle、缓存的 SFTP / passwd / group map 永久泄漏在 sessions map 里。
    // 主动 ssh_disconnect 已先 remove 过，这里 remove 是幂等的 no-op。
    {
        use tauri::Manager;
        app.state::<AppState>().remove(&session_id).await;
    }
    emit_closed(&app, &session_id, exit_code, exit_reason);
}

#[tauri::command]
pub async fn ssh_disconnect(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<()> {
    if let Some(session) = state.remove(&session_id).await {
        let _ = session.outbound.send(Outbound::Close);
    }
    Ok(())
}

#[tauri::command]
pub async fn ssh_send_data(
    state: State<'_, AppState>,
    session_id: String,
    data: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    session.outbound.send(Outbound::Data(data.into_bytes()))
        .map_err(|_| AppError::Ssh("session channel closed".into()))?;
    Ok(())
}

/// 把该会话所属连接保存的密码解密后写入 SSH 通道并自动回车提交（用于 sudo / su 等提示）。
/// `kind` 选择填哪一份：None/"password" = 连接主密码；"acct:<账号>" = 该连接下保存的账号密码
/// （同一台服务器常有多个账号）。前端仅在终端出现密码提示时才显示按钮，不会误填。
/// 密码全程留在后端，不经过渲染层。无对应密码时返回错误。
#[tauri::command]
pub async fn ssh_fill_password(
    state: State<'_, AppState>,
    session_id: String,
    kind: Option<String>,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let cid = session.connection_id.clone()
        .ok_or_else(|| AppError::Ssh("当前连接未保存，无密码可填充".into()))?;
    let kind = kind.unwrap_or_else(|| "password".into());
    let password = load_secret(&cid, &kind)
        .ok_or_else(|| AppError::Ssh("未找到对应的已保存密码".into()))?;
    // 填充并自动回车提交（按钮仅在密码提示出现时可见，sudo 提示不回显，安全）。
    let mut bytes = password.into_bytes();
    bytes.push(b'\r');
    session.outbound.send(Outbound::Data(bytes))
        .map_err(|_| AppError::Ssh("session channel closed".into()))?;
    Ok(())
}

#[tauri::command]
pub async fn ssh_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    session.outbound.send(Outbound::Resize { cols, rows })
        .map_err(|_| AppError::Ssh("session channel closed".into()))?;
    Ok(())
}

/// 前端 xterm 解析完 ssh:data 后确认的原始字节数（按会话攒到 ~128KB 才发一次），用来补终端通道的
/// SSH 窗口。会话已经不在（断开、关闭）就忽略：确认本来就是尽力而为。
#[tauri::command]
pub async fn ssh_ack_data(
    state: State<'_, AppState>,
    session_id: String,
    bytes: u32,
) -> AppResult<()> {
    if let Some(session) = state.get(&session_id).await {
        let _ = session.outbound.send(Outbound::Ack(bytes));
    }
    Ok(())
}

#[tauri::command]
pub async fn ssh_execute(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> AppResult<String> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let mut channel = session.handle.channel_open_session().await
        .map_err(|e| AppError::Ssh(format!("open exec channel: {e}")))?;
    channel.exec(true, command.as_bytes()).await
        .map_err(|e| AppError::Ssh(format!("exec: {e}")))?;
    let mut out = Vec::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::Data { data } => out.extend_from_slice(&data),
            russh::ChannelMsg::ExtendedData { data, .. } => out.extend_from_slice(&data),
            russh::ChannelMsg::ExitStatus { .. }
            | russh::ChannelMsg::Eof
            | russh::ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}

#[tauri::command]
pub async fn ssh_refresh_prompt(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    session.outbound.send(Outbound::Data(b"\n".to_vec()))
        .map_err(|_| AppError::Ssh("session channel closed".into()))?;
    Ok(())
}

#[tauri::command]
pub async fn ssh_activate_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;

    // Atomic with append_to_buffer's activated-read: take the buffer lock,
    // flip activated, then emit the snapshot as the FIRST ssh:data event —
    // still holding the lock, so no later chunk can be appended (hence
    // emitted) before it. Snapshot and live data now share one ordered
    // event channel. Returning the snapshot via the invoke response raced
    // it against ssh:data events (separate IPC paths, no ordering
    // guarantee): with a slow MOTD the first PS1 could render before the
    // banner. swap() keeps re-activation (session switch) from re-emitting
    // the whole buffer.
    let buf = session.buffer.lock().await;
    let was_activated = session.activated.swap(true, Ordering::Relaxed);
    if !was_activated && !buf.is_empty() {
        // 快照里的字节进缓冲时就已补过窗口，不用前端确认。
        emit_data(&app, &session_id, &buf, 0);
    }
    drop(buf);
    Ok(())
}

#[tauri::command]
pub async fn ssh_get_session_buffer(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<String> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let buf = session.buffer.lock().await.clone();
    Ok(buf)
}

// ssh:data 事件合并：累积 4ms 窗口或 32KB 上限内的数据为单次 emit，
// 避免 vim / tmux ctrl+B+F 等大流量爆发拖垮 webview IPC 队列。
const COALESCE_WINDOW: Duration = Duration::from_millis(4);
const COALESCE_MAX: usize = 32 * 1024;

/// 合并窗口按「前沿」触发：空闲后的第一包（按键回显、vim 单次重绘的开头）立刻发，
/// 只有紧跟在刚发出的一包之后到达的数据才并进窗口。原来一律等满 4ms，每次按键回显都多 4ms。
fn flush_output_now(pending_len: usize, flush_pending: bool, since_last_emit: Duration) -> bool {
    pending_len >= COALESCE_MAX || (!flush_pending && since_last_emit >= COALESCE_WINDOW)
}

/// 等着合并发出的输出，连同它对应的原始 SSH 字节数。
struct PendingOutput {
    text: String,
    raw: u32,
}

#[allow(clippy::too_many_arguments)]
fn queue_output(
    app: &AppHandle,
    session_id: &str,
    chunk: &str,
    raw: u32,
    pending: &mut PendingOutput,
    window: &mut TerminalWindow,
    flush_deadline: &mut Option<tokio::time::Instant>,
    last_emit: &mut tokio::time::Instant,
) {
    pending.text.push_str(chunk);
    pending.raw = pending.raw.saturating_add(raw);
    let now = tokio::time::Instant::now();
    if flush_output_now(pending.text.len(), flush_deadline.is_some(), now.duration_since(*last_emit)) {
        emit_pending(app, session_id, pending, window);
        *flush_deadline = None;
        *last_emit = now;
    } else if flush_deadline.is_none() {
        *flush_deadline = Some(now + COALESCE_WINDOW);
    }
}

fn emit_pending(app: &AppHandle, session_id: &str, pending: &mut PendingOutput, window: &mut TerminalWindow) {
    emit_data(app, session_id, &pending.text, pending.raw);
    window.emitted(pending.raw);
    pending.text.clear();
    pending.raw = 0;
}

pub fn emit_data(app: &AppHandle, session_id: &str, data: &str, bytes: u32) {
    let _ = app.emit("ssh:data", SshDataEvent {
        session_id: session_id.to_string(),
        data: data.to_string(),
        bytes,
    });
}

/// 终端通道的 SSH 接收窗口上限：服务端最多比前端已确认（xterm 解析完）的多发这么多。
/// 刷屏时 WebView + xterm 的积压、^C 之后还要滚完的尾巴都以它为界（xterm ≈65MB/s，2MB ≈30ms）。
/// 交互输出远小于它，打字 / vim 不会因为没确认而停下。
pub(crate) const TERMINAL_WINDOW: u32 = 2 * 1024 * 1024;
/// 攒够这么多再补窗口，免得每个小包都发一次 WINDOW_ADJUST。
const GRANT_BATCH: u32 = 64 * 1024;

/// 终端通道的窗口记账（russh 补丁的手动窗口模式，见 vendor/README.md）。服务端还能发的量 =
/// TERMINAL_WINDOW − 已收到但还没补回的字节；什么时候补回：
/// - 激活前：输出只进 100KB 缓冲、不发给前端，收到即算消费——首屏缓冲阶段不能被流控卡住；
/// - 激活后：随 ssh:data 发出的原始字节，等前端 xterm 解析完 ack 才算消费。
/// 确认量按「已发出未确认」封顶：重同步、旧终端迟到的回调等重复确认不会把窗口撑大。
#[derive(Default)]
struct TerminalWindow {
    unacked: u64,
    ungranted: u32,
}

impl TerminalWindow {
    /// 进了缓冲、没有发给前端的字节。返回此刻该补的窗口。
    fn buffered(&mut self, raw: u32) -> Option<u32> {
        self.consumed(raw)
    }

    /// 随 ssh:data 发给前端、等确认的字节。
    fn emitted(&mut self, raw: u32) {
        self.unacked += u64::from(raw);
    }

    /// 前端确认。返回此刻该补的窗口。
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

pub fn emit_closed(app: &AppHandle, session_id: &str, code: Option<u32>, reason: String) {
    let _ = app.emit("ssh:closed", SshClosedEvent {
        session_id: session_id.to_string(),
        code,
        reason,
    });
}

/// 将 SSH 字节流按 UTF-8 字符边界切分。返回可安全解码的完整前缀，把末尾不完整的
/// 多字节序列残留在 `carry` 里等待下一个 chunk，从而避免中文/emoji 被 chunk 边界
/// 截断成 `�`。中间真正非法的字节走 lossy 替换、不滞留。
pub(crate) fn decode_stream_chunk(carry: &mut Vec<u8>, data: &[u8]) -> String {
    carry.extend_from_slice(data);
    let valid = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        Err(e) => match e.error_len() {
            // None = 末尾序列不完整：只输出到 valid_up_to，剩余留到下个 chunk。
            None => e.valid_up_to(),
            // Some = 中间出现真正非法字节：整段 lossy 输出，不滞留（否则 carry 永久卡住）。
            Some(_) => carry.len(),
        },
    };
    let out = String::from_utf8_lossy(&carry[..valid]).into_owned();
    carry.drain(..valid);
    out
}

#[cfg(test)]
mod tests {
    use super::{
        decode_stream_chunk, flush_output_now, is_transient_connect_error, TerminalWindow, COALESCE_MAX,
        COALESCE_WINDOW, GRANT_BATCH, TERMINAL_WINDOW,
    };
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
    fn output_before_activation_never_waits_for_the_renderer() {
        // 激活前前端还没在听：缓冲阶段收到的字节必须自己补回去，否则 MOTD 一长首屏就卡死。
        let mut peer = Peer::new();
        for _ in 0..1000 {
            let n = 32 * 1024;
            peer.received += n;
            let grant = peer.window.buffered(n as u32);
            peer.grant(grant);
            assert!(peer.credit() + u64::from(GRANT_BATCH) >= u64::from(TERMINAL_WINDOW));
        }
    }

    #[test]
    fn emitted_output_waits_for_the_renderer_ack() {
        let mut peer = Peer::new();
        // 刷屏：前端不确认，服务端最多再发满一个窗口就停
        while peer.credit() > 0 {
            let n = peer.credit().min(32 * 1024);
            peer.received += n;
            peer.window.emitted(n as u32);
        }
        assert_eq!(peer.received, u64::from(TERMINAL_WINDOW));
        // xterm 解析完 128KB → 补回 128KB
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
        // 重同步（ACK_ALL）、旧终端迟到的回调都可能多确认；补回去的总量不能超过真正发出去的。
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
        // 伪随机地交错：收数据（激活前 / 后）、xterm 解析完确认、偶尔重同步（ACK_ALL）。
        // 前端重同步时作废旧回调（flow-ack.ts 的 generation），那些字节解析完不再确认。
        // 任何时刻：服务端余量 + 前端还会确认的字节 ≤ 终端窗口；算上作废的也不超过两个窗口。
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
                    if step < 500 {
                        let grant = peer.window.buffered(n as u32);
                        peer.grant(grant);
                    } else {
                        peer.window.emitted(n as u32);
                        fresh += n;
                    }
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

    #[test]
    fn idle_output_goes_out_immediately() {
        // 按键回显：距上次发送已超过合并窗口、也没有等待中的 flush → 立刻发，不再固定等 4ms。
        assert!(flush_output_now(1, false, COALESCE_WINDOW));
        assert!(flush_output_now(1, false, Duration::from_secs(1)));
    }

    #[test]
    fn bursts_are_still_coalesced() {
        // 刚发过一包：后续数据并进合并窗口，避免 vim / tmux 大量重绘拖垮 IPC。
        assert!(!flush_output_now(64, false, Duration::from_millis(1)));
        // 已有等待中的 flush：并进去，由定时器统一发。
        assert!(!flush_output_now(64, true, Duration::from_secs(1)));
    }

    #[test]
    fn full_batch_flushes_regardless_of_timing() {
        assert!(flush_output_now(COALESCE_MAX, true, Duration::ZERO));
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
        let h = super::do_handshake(host, port.parse().unwrap(), "alice", Some("alicepw"), None, None).await.unwrap();
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

        // 按前端确认补窗口后继续流动，且仍不超过补回的量
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

    #[test]
    fn passes_through_complete_utf8() {
        let mut carry = Vec::new();
        assert_eq!(decode_stream_chunk(&mut carry, "héllo 世界".as_bytes()), "héllo 世界");
        assert!(carry.is_empty());
    }

    #[test]
    fn holds_split_multibyte_until_completed() {
        // "世" = E4 B8 96，跨两个 chunk 切开。
        let full = "世".as_bytes();
        let (a, b) = full.split_at(1);
        let mut carry = Vec::new();
        // 第一个 chunk 只有半个字符：不应输出任何内容，残字节滞留。
        assert_eq!(decode_stream_chunk(&mut carry, a), "");
        assert_eq!(carry.len(), 1);
        // 第二个 chunk 补全：完整字符输出，carry 清空。
        assert_eq!(decode_stream_chunk(&mut carry, b), "世");
        assert!(carry.is_empty());
    }

    #[test]
    fn emits_valid_prefix_and_holds_trailing_fragment() {
        let mut carry = Vec::new();
        let mut bytes = b"ok ".to_vec();
        bytes.push("界".as_bytes()[0]); // 追加 "界" 的首字节
        let out = decode_stream_chunk(&mut carry, &bytes);
        assert_eq!(out, "ok ");
        assert_eq!(carry.len(), 1); // 残留首字节等待补全
    }

    #[test]
    fn truly_invalid_bytes_do_not_stick() {
        // 0xFF 不是合法 UTF-8 起始字节，必须 lossy 输出而非永久滞留。
        let mut carry = Vec::new();
        let out = decode_stream_chunk(&mut carry, &[0xFF, b'a']);
        assert!(out.contains('a'));
        assert!(carry.is_empty());
    }
}

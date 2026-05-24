// SSH backend — russh client.
// SshSession owns: Handle (shared with SFTP), shell-task outbound sender, output buffer.
// The shell pump task owns only the Channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    Close,
}

pub struct SshSession {
    pub id: String,
    pub host: String,
    pub username: String,
    pub buffer: Mutex<String>,
    /// Shared SSH handle — locked briefly when opening new channels (SFTP).
    pub handle: Arc<Mutex<russh::client::Handle<ClientHandler>>>,
    /// Cached SFTP session — reused across file ops to avoid sshd MaxSessions exhaustion.
    /// Mirrors the Electron-side fix from commits 8cb552c / a819e15.
    pub sftp: Mutex<Option<Arc<russh_sftp::client::SftpSession>>>,
    /// Lazily loaded uid→username map (from /etc/passwd).
    pub passwd_cache: Mutex<Option<Arc<HashMap<u32, String>>>>,
    /// Lazily loaded gid→groupname map (from /etc/group).
    pub group_cache: Mutex<Option<Arc<HashMap<u32, String>>>>,
    /// Renderer must explicitly activate the session to start receiving ssh:data events.
    /// Until then, output is silently buffered; the renderer fetches the prelude via
    /// ssh_get_session_buffer to avoid the welcome-banner duplicate-render bug.
    pub activated: AtomicBool,
    /// Notified the first time shell data lands in `buffer`. `ssh_connect` awaits this
    /// (with a 400ms cap) so the welcome/PS1 is visible the instant the loader closes —
    /// mirrors the Electron-side waitForFirstData fix from commit c51afa0.
    pub first_data: Arc<Notify>,
    outbound: mpsc::UnboundedSender<Outbound>,
}

impl SshSession {
    pub async fn append_to_buffer(&self, chunk: &str) {
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
        drop(buf);
        if was_empty {
            self.first_data.notify_waiters();
        }
    }
}

fn warm_key(host: &str, port: u16, username: &str) -> String {
    format!("{}:{}:{}", host, port, username)
}

fn russh_config() -> Arc<russh::client::Config> {
    Arc::new(russh::client::Config {
        inactivity_timeout: Some(Duration::from_secs(600)),
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    })
}

/// TCP + SSH handshake + auth. Returns the russh Handle wrapped in Arc<Mutex<>>
/// so it can be shared between the shell pump, SFTP channel, and warm pool.
pub(crate) async fn do_handshake(
    host: &str,
    port: u16,
    username: &str,
    password: Option<&str>,
    private_key: Option<&str>,
    passphrase: Option<&str>,
) -> AppResult<Arc<Mutex<russh::client::Handle<ClientHandler>>>> {
    let addr = format!("{}:{}", host, port);
    // macOS 冷启动/网络刚切换时常出现瞬时 EHOSTUNREACH / ENETUNREACH（ARP 缓存未填充），
    // 给一次短暂重试再失败。
    let tcp = match TcpStream::connect(&addr).await {
        Ok(t) => t,
        Err(e) => {
            let kind = e.kind();
            let transient = matches!(
                kind,
                std::io::ErrorKind::HostUnreachable
                    | std::io::ErrorKind::NetworkUnreachable
                    | std::io::ErrorKind::ConnectionRefused
            ) || e.raw_os_error() == Some(65) // EHOSTUNREACH
                || e.raw_os_error() == Some(51); // ENETUNREACH
            if transient {
                tracing::warn!("tcp connect {addr} transient {e}; retrying once");
                tokio::time::sleep(Duration::from_millis(400)).await;
                TcpStream::connect(&addr)
                    .await
                    .map_err(|e| AppError::Ssh(format!("tcp connect {addr}: {e}")))?
            } else {
                return Err(AppError::Ssh(format!("tcp connect {addr}: {e}")));
            }
        }
    };
    tcp.set_nodelay(true).ok();

    let mut handle = russh::client::connect_stream(russh_config(), tcp, ClientHandler)
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
    Ok(Arc::new(Mutex::new(handle)))
}

pub struct ClientHandler;

#[async_trait::async_trait]
impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO: known_hosts verification — accept-all for now.
        Ok(true)
    }
}

#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    details: ConnectionDetails,
) -> AppResult<ConnectResult> {
    let session_id = uuid::Uuid::new_v4().to_string();

    let password = details.password.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "password"))
    });
    let passphrase = details.passphrase.clone().or_else(|| {
        details.id.as_deref().and_then(|cid| load_secret(cid, "passphrase"))
    });

    let key = warm_key(&details.host, details.port, &details.username);
    let handle_arc = if let Some(warm) = state.warm_take(&key).await {
        tracing::info!("ssh_connect: warm pool hit for {key}");
        warm.handle
    } else {
        tracing::info!("ssh_connect: cold connect for {key}");
        do_handshake(
            &details.host,
            details.port,
            &details.username,
            password.as_deref(),
            details.private_key.as_deref(),
            passphrase.as_deref(),
        )
        .await?
    };

    let channel = {
        let h = handle_arc.lock().await;
        h.channel_open_session().await
            .map_err(|e| AppError::Ssh(format!("open channel: {e}")))?
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
        buffer: Mutex::new(String::new()),
        handle: handle_arc.clone(),
        sftp: Mutex::new(None),
        passwd_cache: Mutex::new(None),
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
        _ = tokio::time::sleep(Duration::from_millis(400)) => {
            tracing::debug!("ssh_connect: first-data wait timed out for {session_id}");
        }
    }

    Ok(ConnectResult { session_id })
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
    let t0 = Instant::now();
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
        .warm_insert(key.clone(), WarmConn { handle, created_at: Instant::now() })
        .await;
    tracing::info!("ssh_prewarm: {key} ready in {}ms", t0.elapsed().as_millis());
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

    // ssh:data 事件合并：累积 4ms 窗口或 32KB 上限内的数据为单次 emit，
    // 避免 vim / tmux ctrl+B+F 等大流量爆发拖垮 webview IPC 队列。
    const COALESCE_WINDOW: Duration = Duration::from_millis(4);
    const COALESCE_MAX: usize = 32 * 1024;
    let mut pending = String::with_capacity(COALESCE_MAX);
    let mut flush_deadline: Option<tokio::time::Instant> = None;
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
                if !pending.is_empty() {
                    emit_data(&app, &session_id, &pending);
                    pending.clear();
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
                    Some(russh::ChannelMsg::Data { data }) => {
                        let s = String::from_utf8_lossy(&data).to_string();
                        session.append_to_buffer(&s).await;
                        if session.activated.load(Ordering::Relaxed) {
                            pending.push_str(&s);
                            if pending.len() >= COALESCE_MAX {
                                emit_data(&app, &session_id, &pending);
                                pending.clear();
                                flush_deadline = None;
                            } else if flush_deadline.is_none() {
                                flush_deadline = Some(tokio::time::Instant::now() + COALESCE_WINDOW);
                            }
                        }
                    }
                    Some(russh::ChannelMsg::ExtendedData { data, ext: _ }) => {
                        let s = String::from_utf8_lossy(&data).to_string();
                        session.append_to_buffer(&s).await;
                        if session.activated.load(Ordering::Relaxed) {
                            pending.push_str(&s);
                            if pending.len() >= COALESCE_MAX {
                                emit_data(&app, &session_id, &pending);
                                pending.clear();
                                flush_deadline = None;
                            } else if flush_deadline.is_none() {
                                flush_deadline = Some(tokio::time::Instant::now() + COALESCE_WINDOW);
                            }
                        }
                    }
                    Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                        exit_code = Some(exit_status);
                        // do NOT break on exit_status alone — server may keep channel open for cleanup
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

    // 退出前刷出残余 pending
    if !pending.is_empty() {
        emit_data(&app, &session_id, &pending);
    }

    tracing::info!("ssh session {session_id} exiting: {exit_reason}");
    {
        let h = session.handle.lock().await;
        let _ = h.disconnect(russh::Disconnect::ByApplication, "client closed", "en").await;
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

#[tauri::command]
pub async fn ssh_execute(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> AppResult<String> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let handle = session.handle.lock().await;
    let mut channel = handle.channel_open_session().await
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
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<()> {
    if let Some(session) = state.get(&session_id).await {
        session.activated.store(true, Ordering::Relaxed);
    }
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

pub fn emit_data(app: &AppHandle, session_id: &str, data: &str) {
    let _ = app.emit("ssh:data", SshDataEvent {
        session_id: session_id.to_string(),
        data: data.to_string(),
    });
}

pub fn emit_closed(app: &AppHandle, session_id: &str, code: Option<u32>, reason: String) {
    let _ = app.emit("ssh:closed", SshClosedEvent {
        session_id: session_id.to_string(),
        code,
        reason,
    });
}

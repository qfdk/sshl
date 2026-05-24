// SSH backend — russh client.
// SshSession owns: Handle (shared with SFTP), shell-task outbound sender, output buffer.
// The shell pump task owns only the Channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};

use crate::config_store::load_secret;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

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
    /// Renderer must explicitly activate the session to start receiving ssh:data events.
    /// Until then, output is silently buffered; the renderer fetches the prelude via
    /// ssh_get_session_buffer to avoid the welcome-banner duplicate-render bug.
    pub activated: AtomicBool,
    outbound: mpsc::UnboundedSender<Outbound>,
}

impl SshSession {
    pub async fn append_to_buffer(&self, chunk: &str) {
        let mut buf = self.buffer.lock().await;
        buf.push_str(chunk);
        if buf.len() > BUFFER_CAP {
            let overflow = buf.len() - BUFFER_CAP;
            buf.drain(..overflow);
        }
    }
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

    let addr = format!("{}:{}", details.host, details.port);
    let tcp = TcpStream::connect(&addr).await
        .map_err(|e| AppError::Ssh(format!("tcp connect {addr}: {e}")))?;
    tcp.set_nodelay(true).ok();

    let config = Arc::new(russh::client::Config {
        inactivity_timeout: Some(Duration::from_secs(600)),
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });

    let mut handle = russh::client::connect_stream(config, tcp, ClientHandler).await
        .map_err(|e| AppError::Ssh(format!("ssh handshake: {e}")))?;

    let authed = if let Some(pk) = &details.private_key {
        let key = russh_keys::decode_secret_key(pk, passphrase.as_deref())
            .map_err(|e| AppError::Ssh(format!("decode key: {e}")))?;
        handle.authenticate_publickey(&details.username, Arc::new(key)).await
            .map_err(|e| AppError::Ssh(format!("pubkey auth: {e}")))?
    } else if let Some(pw) = &password {
        handle.authenticate_password(&details.username, pw).await
            .map_err(|e| AppError::Ssh(format!("password auth: {e}")))?
    } else {
        return Err(AppError::Ssh("no password or private key provided".into()));
    };
    if !authed {
        return Err(AppError::Ssh("authentication rejected".into()));
    }

    let channel = handle.channel_open_session().await
        .map_err(|e| AppError::Ssh(format!("open channel: {e}")))?;
    channel.request_pty(false, "xterm-256color", details.cols, details.rows, 0, 0, &[]).await
        .map_err(|e| AppError::Ssh(format!("request pty: {e}")))?;
    channel.request_shell(false).await
        .map_err(|e| AppError::Ssh(format!("request shell: {e}")))?;

    let (out_tx, out_rx) = mpsc::unbounded_channel::<Outbound>();
    let handle_arc = Arc::new(Mutex::new(handle));

    let session = Arc::new(SshSession {
        id: session_id.clone(),
        host: details.host.clone(),
        username: details.username.clone(),
        buffer: Mutex::new(String::new()),
        handle: handle_arc.clone(),
        sftp: Mutex::new(None),
        activated: AtomicBool::new(false),
        outbound: out_tx,
    });

    state.insert(session_id.clone(), session.clone()).await;

    let app_for_task = app.clone();
    let session_for_task = session.clone();
    let sid_for_task = session_id.clone();
    tokio::spawn(async move {
        run_channel(app_for_task, sid_for_task, session_for_task, channel, out_rx).await;
    });

    Ok(ConnectResult { session_id })
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
    loop {
        tokio::select! {
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
                            emit_data(&app, &session_id, &s);
                        }
                    }
                    Some(russh::ChannelMsg::ExtendedData { data, ext: _ }) => {
                        let s = String::from_utf8_lossy(&data).to_string();
                        session.append_to_buffer(&s).await;
                        if session.activated.load(Ordering::Relaxed) {
                            emit_data(&app, &session_id, &s);
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

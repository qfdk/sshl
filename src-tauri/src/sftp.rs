// SFTP backend — russh-sftp on top of an existing SSH session.
// Per-session SftpSession is cached on SshSession.sftp to avoid Channel accumulation
// (mirrors Electron-side fix in commits 8cb552c / a819e15).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, AppResult};
use crate::ssh::SshSession;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub size: u64,
    #[serde(rename = "modifyTime")]
    pub modified: Option<u64>,
    pub permissions: u32,
    pub owner: String,
    pub group: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub session_id: String,
    pub remote_path: String,
    pub transferred: u64,
    pub total: u64,
}

async fn sftp_for(session: &Arc<SshSession>) -> AppResult<Arc<SftpSession>> {
    {
        let cache = session.sftp.lock().await;
        if let Some(s) = cache.as_ref() {
            return Ok(s.clone());
        }
    }
    // Open new SFTP subsystem channel and cache it.
    let handle = session.handle.lock().await;
    let channel = handle.channel_open_session().await
        .map_err(|e| AppError::Sftp(format!("open sftp channel: {e}")))?;
    channel.request_subsystem(true, "sftp").await
        .map_err(|e| AppError::Sftp(format!("request sftp subsystem: {e}")))?;
    drop(handle);
    let sftp = SftpSession::new(channel.into_stream()).await
        .map_err(|e| AppError::Sftp(format!("sftp init: {e}")))?;
    let sftp = Arc::new(sftp);
    let mut cache = session.sftp.lock().await;
    *cache = Some(sftp.clone());
    Ok(sftp)
}

fn join_remote(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{}", name)
    } else if base.ends_with('/') {
        format!("{}{}", base, name)
    } else {
        format!("{}/{}", base, name)
    }
}

#[tauri::command]
pub async fn file_list(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> AppResult<Vec<RemoteEntry>> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    let entries = sftp.read_dir(&path).await
        .map_err(|e| AppError::Sftp(format!("read_dir {path}: {e}")))?;
    let mut out: Vec<RemoteEntry> = Vec::new();
    for e in entries {
        let name = e.file_name();
        if name == "." || name == ".." { continue; }
        let mut meta = e.metadata();
        let full = join_remote(&path, &name);

        // If symlink, follow it so size/is_directory reflect the target.
        // POSIX S_IFLNK = 0o120000, S_IFMT = 0o170000.
        let is_symlink = meta.permissions.map(|p| p & 0o170000 == 0o120000).unwrap_or(false);
        if is_symlink {
            if let Ok(target) = sftp.metadata(&full).await {
                meta = target;
            }
        }

        out.push(RemoteEntry {
            path: full,
            is_directory: meta.is_dir(),
            size: meta.size.unwrap_or(0),
            // mtime is seconds since epoch; JS Date() wants milliseconds.
            modified: meta.mtime.map(|s| s as u64 * 1000),
            permissions: meta.permissions.unwrap_or(0),
            owner: meta.uid.map(|u| u.to_string()).unwrap_or_default(),
            group: meta.gid.map(|g| g.to_string()).unwrap_or_default(),
            name,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn file_upload(
    _app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    local_path: String,
    remote_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    let data = tokio::fs::read(&local_path).await?;
    let mut file = sftp.open_with_flags(
        &remote_path,
        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
    ).await.map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?;
    file.write_all(&data).await
        .map_err(|e| AppError::Sftp(format!("write {remote_path}: {e}")))?;
    file.shutdown().await
        .map_err(|e| AppError::Sftp(format!("close {remote_path}: {e}")))?;
    Ok(())
}

#[tauri::command]
pub async fn file_download(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    let meta = sftp.metadata(&remote_path).await
        .map_err(|e| AppError::Sftp(format!("stat {remote_path}: {e}")))?;
    let total = meta.size.unwrap_or(0);

    use tokio::io::AsyncReadExt;
    let mut remote = sftp.open(&remote_path).await
        .map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?;
    if let Some(parent) = Path::new(&local_path).parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let mut local = tokio::fs::File::create(&local_path).await?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut transferred: u64 = 0;
    loop {
        let n = remote.read(&mut buf).await
            .map_err(|e| AppError::Sftp(format!("read {remote_path}: {e}")))?;
        if n == 0 { break; }
        local.write_all(&buf[..n]).await?;
        transferred += n as u64;
        emit_download_progress(&app, DownloadProgress {
            session_id: session_id.clone(),
            remote_path: remote_path.clone(),
            transferred,
            total,
        });
    }
    local.flush().await?;
    Ok(())
}

#[tauri::command]
pub async fn file_create_remote_directory(
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    // mkdir -p semantics
    let mut accum = String::new();
    for part in remote_path.split('/') {
        if part.is_empty() { accum.push('/'); continue; }
        if !accum.ends_with('/') { accum.push('/'); }
        accum.push_str(part);
        // Ignore "already exists" errors silently.
        let _ = sftp.create_dir(&accum).await;
    }
    Ok(())
}

fn upload_dir<'a>(
    sftp: Arc<SftpSession>,
    local: PathBuf,
    remote: String,
) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let _ = sftp.create_dir(&remote).await;
        let mut rd = tokio::fs::read_dir(&local).await?;
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let child_local = entry.path();
            let child_remote = join_remote(&remote, &name);
            let meta = entry.metadata().await?;
            if meta.is_dir() {
                upload_dir(sftp.clone(), child_local, child_remote).await?;
            } else {
                let data = tokio::fs::read(&child_local).await?;
                let mut f = sftp.open_with_flags(
                    &child_remote,
                    OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
                ).await.map_err(|e| AppError::Sftp(format!("open {child_remote}: {e}")))?;
                f.write_all(&data).await
                    .map_err(|e| AppError::Sftp(format!("write {child_remote}: {e}")))?;
                f.shutdown().await
                    .map_err(|e| AppError::Sftp(format!("close {child_remote}: {e}")))?;
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn file_upload_directory(
    _app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    local_path: String,
    remote_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    upload_dir(sftp, PathBuf::from(local_path), remote_path).await
}

fn download_dir<'a>(
    sftp: Arc<SftpSession>,
    remote: String,
    local: PathBuf,
) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(&local).await?;
        let entries = sftp.read_dir(&remote).await
            .map_err(|e| AppError::Sftp(format!("read_dir {remote}: {e}")))?;
        for e in entries {
            let name = e.file_name();
            if name == "." || name == ".." { continue; }
            let child_remote = join_remote(&remote, &name);
            let child_local = local.join(&name);
            let meta = e.metadata();
            if meta.is_dir() {
                download_dir(sftp.clone(), child_remote, child_local).await?;
            } else {
                use tokio::io::AsyncReadExt;
                let mut rf = sftp.open(&child_remote).await
                    .map_err(|e| AppError::Sftp(format!("open {child_remote}: {e}")))?;
                let mut lf = tokio::fs::File::create(&child_local).await?;
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let n = rf.read(&mut buf).await
                        .map_err(|e| AppError::Sftp(format!("read {child_remote}: {e}")))?;
                    if n == 0 { break; }
                    lf.write_all(&buf[..n]).await?;
                }
                lf.flush().await?;
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn file_download_directory(
    _app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    download_dir(sftp, remote_path, PathBuf::from(local_path)).await
}

#[tauri::command]
pub async fn file_change_permissions(
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    permissions: u32,
) -> AppResult<()> {
    // Run via shell exec — russh-sftp's set_metadata API surface varies across versions
    // and chmod via `chmod` is universally portable.
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let handle = session.handle.lock().await;
    let channel = handle.channel_open_session().await
        .map_err(|e| AppError::Sftp(format!("open exec channel: {e}")))?;
    let cmd = format!("chmod {:o} {}", permissions, shell_quote(&remote_path));
    drain_exec(channel, &cmd).await
}

#[tauri::command]
pub async fn file_change_owner(
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    owner: String,
    group: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let handle = session.handle.lock().await;
    let channel = handle.channel_open_session().await
        .map_err(|e| AppError::Sftp(format!("open exec channel: {e}")))?;
    let cmd = format!(
        "chown {}:{} {}",
        shell_quote(&owner),
        shell_quote(&group),
        shell_quote(&remote_path)
    );
    drain_exec(channel, &cmd).await
}

fn shell_quote(s: &str) -> String {
    // Single-quote and escape embedded single quotes.
    let escaped = s.replace('\'', r"'\''");
    format!("'{}'", escaped)
}

async fn drain_exec(
    mut channel: russh::Channel<russh::client::Msg>,
    cmd: &str,
) -> AppResult<()> {
    channel.exec(true, cmd.as_bytes()).await
        .map_err(|e| AppError::Ssh(format!("exec: {e}")))?;
    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::ExitStatus { exit_status } => {
                if exit_status != 0 {
                    return Err(AppError::Other(format!("`{}` exited {}", cmd, exit_status)));
                }
            }
            russh::ChannelMsg::Eof | russh::ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok(())
}

pub fn emit_download_progress(app: &AppHandle, payload: DownloadProgress) {
    let _ = app.emit("file:download-progress", payload);
}

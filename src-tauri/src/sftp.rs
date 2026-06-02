// SFTP backend — russh-sftp on top of an existing SSH session.
// Per-session SftpSession is cached on SshSession.sftp to avoid Channel accumulation
// (mirrors Electron-side fix in commits 8cb552c / a819e15).

use std::collections::HashMap;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgress {
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

/// Read a small text file via SFTP and parse `colon:separated:fields` lines,
/// extracting `(name, id)` from `name:_:id:...`. Used for /etc/passwd and /etc/group.
async fn load_id_map(sftp: &SftpSession, path: &str) -> Option<HashMap<u32, String>> {
    use tokio::io::AsyncReadExt;
    let mut file = sftp.open(path).await.ok()?;
    let mut buf = Vec::with_capacity(8 * 1024);
    file.read_to_end(&mut buf).await.ok()?;
    Some(parse_id_map(&String::from_utf8_lossy(&buf)))
}

/// 解析 `name:_:id:...` 形式（/etc/passwd、/etc/group）为 id→name 映射。
/// 坏行（字段缺失、id 非数字）跳过而非整体失败。
fn parse_id_map(text: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') { continue; }
        let mut parts = line.splitn(4, ':');
        let (Some(name), Some(_), Some(id_str)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if let Ok(id) = id_str.parse::<u32>() {
            map.insert(id, name.to_string());
        }
    }
    map
}

async fn passwd_map(session: &Arc<SshSession>, sftp: &SftpSession) -> Arc<HashMap<u32, String>> {
    {
        let cache = session.passwd_cache.lock().await;
        if let Some(m) = cache.as_ref() { return m.clone(); }
    }
    let map = Arc::new(load_id_map(sftp, "/etc/passwd").await.unwrap_or_default());
    let mut cache = session.passwd_cache.lock().await;
    *cache = Some(map.clone());
    map
}

async fn group_map(session: &Arc<SshSession>, sftp: &SftpSession) -> Arc<HashMap<u32, String>> {
    {
        let cache = session.group_cache.lock().await;
        if let Some(m) = cache.as_ref() { return m.clone(); }
    }
    let map = Arc::new(load_id_map(sftp, "/etc/group").await.unwrap_or_default());
    let mut cache = session.group_cache.lock().await;
    *cache = Some(map.clone());
    map
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
    let users = passwd_map(&session, &sftp).await;
    let groups = group_map(&session, &sftp).await;
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
            owner: meta.uid
                .map(|u| users.get(&u).cloned().unwrap_or_else(|| u.to_string()))
                .unwrap_or_default(),
            group: meta.gid
                .map(|g| groups.get(&g).cloned().unwrap_or_else(|| g.to_string()))
                .unwrap_or_default(),
            name,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn file_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    local_path: String,
    remote_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    let total = tokio::fs::metadata(&local_path).await.map(|m| m.len()).unwrap_or(0);
    let mut file = sftp.open_with_flags(
        &remote_path,
        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
    ).await.map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?;
    // 进度事件按 100ms 节流，避免大文件刷爆 webview 事件队列（与下载侧一致）。
    let mut last_emit = std::time::Instant::now();
    stream_to_remote(&local_path, &mut file, &remote_path, |transferred| {
        if last_emit.elapsed() >= std::time::Duration::from_millis(100) {
            emit_upload_progress(&app, UploadProgress {
                session_id: session_id.clone(),
                remote_path: remote_path.clone(),
                transferred,
                total,
            });
            last_emit = std::time::Instant::now();
        }
    }).await?;
    file.shutdown().await
        .map_err(|e| AppError::Sftp(format!("close {remote_path}: {e}")))?;
    // 收尾事件：保证进度条最终走到 100%（节流可能漏掉最后一段）。
    emit_upload_progress(&app, UploadProgress {
        session_id,
        remote_path,
        transferred: total,
        total,
    });
    Ok(())
}

/// 分块从本地文件流式写入 SFTP 远端句柄，避免大文件一次性读进内存。
/// 下载侧本来就是 64KB 流式的，上传侧此前用 `fs::read` 全量加载，几 GB 文件会 OOM。
/// `on_progress` 接收累计已传字节数，调用方负责节流。
async fn stream_to_remote<W>(
    local_path: &str,
    file: &mut W,
    remote_path: &str,
    mut on_progress: impl FnMut(u64),
) -> AppResult<()>
where
    W: AsyncWriteExt + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut local = tokio::fs::File::open(local_path).await?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut transferred: u64 = 0;
    loop {
        let n = local.read(&mut buf).await?;
        if n == 0 { break; }
        file.write_all(&buf[..n]).await
            .map_err(|e| AppError::Sftp(format!("write {remote_path}: {e}")))?;
        transferred += n as u64;
        on_progress(transferred);
    }
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
    // 进度事件按 100ms 节流：64KB 一次 emit 会让 1GB 文件产生上万次 IPC + 字符串 clone，
    // 淹没 webview 事件队列。只在间隔过去或传输完成时才 emit。
    let mut last_emit = std::time::Instant::now();
    loop {
        let n = remote.read(&mut buf).await
            .map_err(|e| AppError::Sftp(format!("read {remote_path}: {e}")))?;
        if n == 0 { break; }
        local.write_all(&buf[..n]).await?;
        transferred += n as u64;
        if last_emit.elapsed() >= std::time::Duration::from_millis(100) {
            emit_download_progress(&app, DownloadProgress {
                session_id: session_id.clone(),
                remote_path: remote_path.clone(),
                transferred,
                total,
            });
            last_emit = std::time::Instant::now();
        }
    }
    local.flush().await?;
    // 收尾事件：保证进度条最终走到 100%（节流可能漏掉最后一段）。
    emit_download_progress(&app, DownloadProgress {
        session_id,
        remote_path,
        transferred,
        total,
    });
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
                let mut f = sftp.open_with_flags(
                    &child_remote,
                    OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
                ).await.map_err(|e| AppError::Sftp(format!("open {child_remote}: {e}")))?;
                let child_local_str = child_local.to_string_lossy().to_string();
                stream_to_remote(&child_local_str, &mut f, &child_remote, |_| {}).await?;
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

pub fn emit_upload_progress(app: &AppHandle, payload: UploadProgress) {
    let _ = app.emit("file:upload-progress", payload);
}

#[cfg(test)]
mod tests {
    use super::{join_remote, parse_id_map, shell_quote};

    #[test]
    fn join_remote_handles_root_and_trailing_slash() {
        assert_eq!(join_remote("/", "etc"), "/etc");
        assert_eq!(join_remote("/var", "log"), "/var/log");
        assert_eq!(join_remote("/var/", "log"), "/var/log");
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("a b"), "'a b'");
        // 单引号需要闭合-转义-重开，防止 chmod/chown 命令注入。
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("a; rm -rf /"), "'a; rm -rf /'");
    }

    #[test]
    fn parse_id_map_extracts_id_to_name() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\ndaemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n";
        let map = parse_id_map(passwd);
        assert_eq!(map.get(&0).map(String::as_str), Some("root"));
        assert_eq!(map.get(&1).map(String::as_str), Some("daemon"));
    }

    #[test]
    fn parse_id_map_skips_comments_and_malformed_lines() {
        let text = "# comment\n\nbad-line-no-colons\nnouid:x:notanumber:5\nok:x:42:42\n";
        let map = parse_id_map(text);
        // 坏行被跳过，但合法行仍被收录（旧实现遇坏行会丢掉整张表）。
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&42).map(String::as_str), Some("ok"));
    }
}

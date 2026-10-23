// SFTP backend — russh-sftp on top of an existing SSH session.
// Per-session SftpSession is cached on SshSession.sftp to avoid Channel accumulation
// (mirrors Electron-side fix in commits 8cb552c / a819e15).

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use russh_sftp::client::fs::Metadata;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, AppResult};
use crate::ssh::SshSession;
use crate::state::AppState;

/// 单次 SFTP 读/写的块大小 = OpenSSH sftp-server 的 max-read/write-length（256KiB − 1KiB），
/// 每块恰好一个 SFTP 请求。russh-sftp 的读是一问一答（等到回复才发下一个 READ），
/// 吞吐 ≈ 块大小 / RTT：64KB 时 50ms RTT 下只有 ~1.3MB/s。写有 8 路 pipeline，
/// 块越大在途数据越多（8 × 255KB ≈ SSH 默认 2MB 窗口）。
const TRANSFER_CHUNK: usize = 255 * 1024;

/// 符号链接 STAT 的并发度。每个 STAT 一个 RTT，russh-sftp 按请求 id 分发响应，可并发；
/// 串行时 /usr/lib 这类几百个 .so 链接的目录要几百个 RTT。
const SYMLINK_STAT_CONCURRENCY: usize = 32;

/// 大文件分段并发下载的段数。单句柄下载一问一答（吞吐 ≈ 块大小 / RTT），
/// 开 N 个句柄读不相交区间，在途 READ 就是 N 个。
const DOWNLOAD_SEGMENTS: usize = 4;

/// 小于此大小走单句柄流式下载：多开句柄的 open/close 往返抵不过并发收益。
const SEGMENTED_DOWNLOAD_MIN: u64 = 4 * TRANSFER_CHUNK as u64;

/// 目录传输时同一层文件的并发数。小文件的耗时主要是 open/close 各一个 RTT，
/// 串行时几百个小文件就是几百轮往返。子目录仍逐个递归，避免并发数随深度相乘。
const DIR_FILE_CONCURRENCY: usize = 4;

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
    /// 目录传输才带：已完成 / 总文件数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_total: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgress {
    pub session_id: String,
    pub remote_path: String,
    pub transferred: u64,
    pub total: u64,
    /// 目录传输才带：已完成 / 总文件数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_total: Option<u64>,
}

async fn sftp_for(session: &Arc<SshSession>) -> AppResult<Arc<SftpSession>> {
    {
        let cache = session.sftp.lock().await;
        if let Some(s) = cache.as_ref() {
            return Ok(s.clone());
        }
    }
    // Open new SFTP subsystem channel and cache it. Handle 锁只覆盖开通道（同 ssh_execute）。
    let channel = {
        let handle = session.handle.lock().await;
        handle.channel_open_session().await
            .map_err(|e| AppError::Sftp(format!("open sftp channel: {e}")))?
    };
    channel.request_subsystem(true, "sftp").await
        .map_err(|e| AppError::Sftp(format!("request sftp subsystem: {e}")))?;
    let sftp = SftpSession::new(channel.into_stream()).await
        .map_err(|e| AppError::Sftp(format!("sftp init: {e}")))?;
    // russh-sftp 的读请求从发出起计时，默认 10s 超时（写请求不计时）。分段下载 / 目录并发下载
    // 会同时挂约 1MB 的 READ，链路慢于 ~100KB/s 时排在后面的回复会超时；放宽到 30s（≥ ~35KB/s）。
    sftp.set_timeout(30);
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

/// 对符号链接条目跟随一次 STAT，让 size / is_directory 反映目标；保持原顺序。
/// STAT 失败（悬空链接等）时保留链接自身的元数据。
async fn follow_symlinks<F, Fut>(
    entries: Vec<(String, String, Metadata)>,
    stat: F,
) -> Vec<(String, String, Metadata)>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Option<Metadata>>,
{
    stream::iter(entries)
        .map(|(name, full, meta)| {
            // POSIX S_IFLNK = 0o120000, S_IFMT = 0o170000.
            let is_symlink = meta.permissions.map(|p| p & 0o170000 == 0o120000).unwrap_or(false);
            let target = is_symlink.then(|| stat(full.clone()));
            async move {
                let meta = match target {
                    Some(fut) => fut.await.unwrap_or(meta),
                    None => meta,
                };
                (name, full, meta)
            }
        })
        .buffered(SYMLINK_STAT_CONCURRENCY)
        .collect()
        .await
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
    // read_dir 与首次加载 /etc/passwd、/etc/group 互不依赖，并发发出省掉两轮往返。
    let (entries, users, groups) = tokio::join!(
        sftp.read_dir(&path),
        passwd_map(&session, &sftp),
        group_map(&session, &sftp),
    );
    let entries = entries.map_err(|e| AppError::Sftp(format!("read_dir {path}: {e}")))?;
    let listed: Vec<(String, String, Metadata)> = entries
        .filter_map(|e| {
            let name = e.file_name();
            if name == "." || name == ".." { return None; }
            let full = join_remote(&path, &name);
            Some((name, full, e.metadata()))
        })
        .collect();
    let sftp_ref = &sftp;
    let resolved = follow_symlinks(listed, |full| async move {
        sftp_ref.metadata(full).await.ok()
    }).await;
    let mut out: Vec<RemoteEntry> = Vec::with_capacity(resolved.len());
    for (name, full, meta) in resolved {
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

/// 远程 home = SFTP 服务端的初始工作目录（OpenSSH sftp 客户端也是 realpath "." 取初始目录）。
/// 不用 exec `echo $HOME`：省一个 exec 通道和远端一次 shell 启动，SFTP 会话反正要开；
/// 只允许 SFTP（ForceCommand internal-sftp）的服务器上 exec 会失败，这里照样能拿到。
#[tauri::command]
pub async fn file_remote_home(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<String> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    sftp.canonicalize(".").await
        .map_err(|e| AppError::Sftp(format!("realpath .: {e}")))
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
                files_done: None,
                files_total: None,
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
        files_done: None,
        files_total: None,
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
    let mut buf = vec![0u8; TRANSFER_CHUNK];
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

/// 与 `stream_to_remote` 对称：分块把 SFTP 远端句柄流式写进本地文件，返回累计字节数。
/// `on_progress` 接收累计已传字节数，调用方负责节流。
async fn stream_to_local<R, W>(
    remote: &mut R,
    local: &mut W,
    remote_path: &str,
    mut on_progress: impl FnMut(u64),
) -> AppResult<u64>
where
    R: tokio::io::AsyncRead + Unpin,
    W: AsyncWriteExt + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut transferred: u64 = 0;
    loop {
        let n = remote.read(&mut buf).await
            .map_err(|e| AppError::Sftp(format!("read {remote_path}: {e}")))?;
        if n == 0 { break; }
        local.write_all(&buf[..n]).await?;
        transferred += n as u64;
        on_progress(transferred);
    }
    local.flush().await?;
    Ok(transferred)
}

/// 把 [0, total) 切成至多 `segments` 段，段长按 TRANSFER_CHUNK 对齐，让每个 READ 都是整块。
fn segment_ranges(total: u64, segments: usize) -> Vec<(u64, u64)> {
    let chunk = TRANSFER_CHUNK as u64;
    let per = total.div_ceil(segments.max(1) as u64).div_ceil(chunk).max(1) * chunk;
    (0..segments as u64)
        .map(|i| (i * per, ((i + 1) * per).min(total)))
        .filter(|(start, end)| start < end)
        .collect()
}

/// 分段并发下载：每段用 `open_remote` / `open_local` 各开一个独立句柄，seek 到段首后
/// 顺序读写，返回累计字节数。本地文件不预分配长度——各段写到自己的偏移，最终长度
/// 自然等于实际写到的末尾。非最后一段提前 EOF 说明文件在下载中变短，直接报错，
/// 不留下零填充的空洞；最后一段不设上界读到 EOF。
async fn download_segmented<R, W, OR, ORF, OL, OLF>(
    total: u64,
    segments: usize,
    open_remote: OR,
    open_local: OL,
    remote_path: &str,
    on_progress: impl Fn(u64),
) -> AppResult<u64>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
    W: AsyncWriteExt + tokio::io::AsyncSeek + Unpin,
    OR: Fn() -> ORF,
    ORF: Future<Output = AppResult<R>>,
    OL: Fn() -> OLF,
    OLF: Future<Output = AppResult<W>>,
{
    use std::io::SeekFrom;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let ranges = segment_ranges(total, segments);
    let last = ranges.len().saturating_sub(1);
    let transferred = AtomicU64::new(0);
    let (open_remote, open_local, transferred_ref, on_progress) =
        (&open_remote, &open_local, &transferred, &on_progress);
    let workers = ranges.into_iter().enumerate().map(|(i, (start, end))| async move {
        let mut remote = open_remote().await?;
        let mut local = open_local().await?;
        remote.seek(SeekFrom::Start(start)).await
            .map_err(|e| AppError::Sftp(format!("seek {remote_path}: {e}")))?;
        local.seek(SeekFrom::Start(start)).await?;
        let mut buf = vec![0u8; TRANSFER_CHUNK];
        let mut pos = start;
        loop {
            let want = if i == last {
                TRANSFER_CHUNK
            } else {
                (end - pos).min(TRANSFER_CHUNK as u64) as usize
            };
            if want == 0 { break; }
            let n = remote.read(&mut buf[..want]).await
                .map_err(|e| AppError::Sftp(format!("read {remote_path}: {e}")))?;
            if n == 0 {
                if i != last {
                    return Err(AppError::Sftp(format!(
                        "{remote_path} shrank during download (EOF at {pos}, expected {end})"
                    )));
                }
                break;
            }
            local.write_all(&buf[..n]).await?;
            pos += n as u64;
            on_progress(transferred_ref.fetch_add(n as u64, Ordering::Relaxed) + n as u64);
        }
        local.flush().await?;
        Ok::<(), AppError>(())
    });
    futures::future::try_join_all(workers).await?;
    Ok(transferred.into_inner())
}

/// 大文件分段下载到 `local_path`。先确认远端可读再创建/截断本地文件：能 stat 但没有读权限
/// （如 root 的大日志）时，不留下 0 字节文件、也不截断同名的已有文件。之后各段再各自以写模式打开。
async fn download_large(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &Path,
    total: u64,
    on_progress: impl Fn(u64),
) -> AppResult<u64> {
    drop(sftp.open(remote_path).await
        .map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?);
    tokio::fs::File::create(local_path).await?;
    download_segmented(
        total,
        DOWNLOAD_SEGMENTS,
        move || async move {
            sftp.open(remote_path).await
                .map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))
        },
        move || async move {
            Ok(tokio::fs::OpenOptions::new().write(true).open(local_path).await?)
        },
        remote_path,
        on_progress,
    ).await
}

/// 目录下载的同层文件：小文件按 DIR_FILE_CONCURRENCY 并发；≥ SEGMENTED_DOWNLOAD_MIN 的大文件
/// 在小文件之后逐个分段下载。两批不重叠，在途 READ 至多 max(DIR_FILE_CONCURRENCY, DOWNLOAD_SEGMENTS)
/// 个，与单文件分段下载持平，SFTP 30s 读超时的余量不变。
async fn download_dir_files<T, FS, FutS, FL, FutL>(
    files: Vec<(T, u64)>,
    small: FS,
    large: FL,
) -> AppResult<()>
where
    FS: Fn(T) -> FutS,
    FutS: Future<Output = AppResult<()>>,
    FL: Fn(T, u64) -> FutL,
    FutL: Future<Output = AppResult<()>>,
{
    let (large_files, small_files): (Vec<_>, Vec<_>) =
        files.into_iter().partition(|(_, size)| *size >= SEGMENTED_DOWNLOAD_MIN);
    transfer_files_concurrently(small_files.into_iter().map(|(item, _)| item).collect(), small).await?;
    for (item, size) in large_files {
        large(item, size).await?;
    }
    Ok(())
}

/// 按 DIR_FILE_CONCURRENCY 并发处理同一层的一批文件，遇到第一个错误即返回。
async fn transfer_files_concurrently<T, F, Fut>(items: Vec<T>, transfer: F) -> AppResult<()>
where
    F: Fn(T) -> Fut,
    Fut: Future<Output = AppResult<()>>,
{
    use futures::TryStreamExt;
    stream::iter(items.into_iter().map(Ok))
        .try_for_each_concurrent(DIR_FILE_CONCURRENCY, transfer)
        .await
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

    if let Some(parent) = Path::new(&local_path).parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    // 进度事件按 100ms 节流：每块一次 emit 会让 1GB 文件产生上万次 IPC + 字符串 clone，
    // 淹没 webview 事件队列。只在间隔过去或传输完成时才 emit。
    // 分段下载时各段并发回调，节流时间戳放 Mutex 里共享。
    let last_emit = std::sync::Mutex::new(std::time::Instant::now());
    let on_progress = |transferred: u64| {
        let mut last = last_emit.lock().unwrap();
        if last.elapsed() >= std::time::Duration::from_millis(100) {
            emit_download_progress(&app, DownloadProgress {
                session_id: session_id.clone(),
                remote_path: remote_path.clone(),
                transferred,
                total,
                files_done: None,
                files_total: None,
            });
            *last = std::time::Instant::now();
        }
    };
    let transferred = if total >= SEGMENTED_DOWNLOAD_MIN {
        download_large(&sftp, &remote_path, Path::new(&local_path), total, &on_progress).await?
    } else {
        let mut remote = sftp.open(&remote_path).await
            .map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?;
        let mut local = tokio::fs::File::create(&local_path).await?;
        stream_to_local(&mut remote, &mut local, &remote_path, &on_progress).await?
    };
    // 收尾事件：保证进度条最终走到 100%（节流可能漏掉最后一段）。
    emit_download_progress(&app, DownloadProgress {
        session_id,
        remote_path,
        transferred,
        total,
        files_done: None,
        files_total: None,
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

/// 目录传输进度事件的节流间隔（与单文件传输一致）。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// 目录传输计划：先完整遍历，拿到全部子目录（父目录在前）和每个文件的大小，才有总量和百分比；
/// 遍历结果直接用于传输，不重复列目录。files 为 (源, 目标, 字节数)。
struct DirPlan<S, D> {
    dirs: Vec<D>,
    files: Vec<(S, D, u64)>,
}

impl<S, D> DirPlan<S, D> {
    fn total_bytes(&self) -> u64 {
        self.files.iter().map(|(_, _, size)| size).sum()
    }
}

/// 上传前遍历本地目录。目录判断沿用 DirEntry 的元数据（不跟随符号链接），
/// 文件大小跟随符号链接取目标的，与实际传输的字节数一致。
async fn scan_local(local: PathBuf, remote: String) -> AppResult<DirPlan<PathBuf, String>> {
    let mut plan = DirPlan { dirs: vec![remote.clone()], files: Vec::new() };
    let mut queue = VecDeque::from([(local, remote)]);
    while let Some((dir_local, dir_remote)) = queue.pop_front() {
        let mut rd = tokio::fs::read_dir(&dir_local).await?;
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let (child_local, child_remote) = (entry.path(), join_remote(&dir_remote, &name));
            if entry.metadata().await?.is_dir() {
                plan.dirs.push(child_remote.clone());
                queue.push_back((child_local, child_remote));
            } else {
                let size = tokio::fs::metadata(&child_local).await.map(|m| m.len()).unwrap_or(0);
                plan.files.push((child_local, child_remote, size));
            }
        }
    }
    Ok(plan)
}

/// 下载前遍历远端目录。`list` 列出一个远端目录的 (名称, 是否目录, 字节数)。
async fn scan_remote<L, Fut>(remote: String, local: PathBuf, list: L) -> AppResult<DirPlan<String, PathBuf>>
where
    L: Fn(String) -> Fut,
    Fut: Future<Output = AppResult<Vec<(String, bool, u64)>>>,
{
    let mut plan = DirPlan { dirs: vec![local.clone()], files: Vec::new() };
    let mut queue = VecDeque::from([(remote, local)]);
    while let Some((dir_remote, dir_local)) = queue.pop_front() {
        for (name, is_dir, size) in list(dir_remote.clone()).await? {
            if name == "." || name == ".." { continue; }
            let (child_remote, child_local) = (join_remote(&dir_remote, &name), dir_local.join(&name));
            if is_dir {
                plan.dirs.push(child_local.clone());
                queue.push_back((child_remote, child_local));
            } else {
                plan.files.push((child_remote, child_local, size));
            }
        }
    }
    Ok(plan)
}

/// 目录传输的整体进度。各文件回报自己的累计字节，这里换算成增量累加到全目录；
/// 按 interval 节流回调 (已传字节, 已完成文件数)，finish 时无条件再回调一次，保证进度走到终点。
struct DirProgress<F: Fn(u64, u64)> {
    interval: Duration,
    transferred: AtomicU64,
    files_done: AtomicU64,
    last_emit: Mutex<Instant>,
    emit: F,
}

impl<F: Fn(u64, u64)> DirProgress<F> {
    fn new(interval: Duration, emit: F) -> Self {
        Self {
            interval,
            transferred: AtomicU64::new(0),
            files_done: AtomicU64::new(0),
            last_emit: Mutex::new(Instant::now()),
            emit,
        }
    }

    /// 单个文件的进度回调，参数是该文件的累计字节数。
    fn file(&self) -> impl Fn(u64) + '_ {
        let seen = AtomicU64::new(0);
        move |current| {
            let delta = current.saturating_sub(seen.swap(current, Ordering::Relaxed));
            self.transferred.fetch_add(delta, Ordering::Relaxed);
            self.maybe_emit();
        }
    }

    fn file_done(&self) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
        self.maybe_emit();
    }

    fn maybe_emit(&self) {
        let mut last = self.last_emit.lock().unwrap();
        if last.elapsed() >= self.interval {
            *last = Instant::now();
            drop(last);
            self.finish();
        }
    }

    fn finish(&self) {
        (self.emit)(self.transferred.load(Ordering::Relaxed), self.files_done.load(Ordering::Relaxed));
    }
}

#[tauri::command]
pub async fn file_upload_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    local_path: String,
    remote_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    let plan = scan_local(PathBuf::from(local_path), remote_path.clone()).await?;
    for dir in &plan.dirs {
        // 已存在不算错（mkdir -p 语义）。
        let _ = sftp.create_dir(dir).await;
    }
    let (total, files_total) = (plan.total_bytes(), plan.files.len() as u64);
    let progress = DirProgress::new(PROGRESS_INTERVAL, |transferred, files_done| {
        emit_upload_progress(&app, UploadProgress {
            session_id: session_id.clone(),
            remote_path: remote_path.clone(),
            transferred,
            total,
            files_done: Some(files_done),
            files_total: Some(files_total),
        });
    });
    let (sftp_ref, progress_ref) = (&sftp, &progress);
    transfer_files_concurrently(plan.files, |(local, remote, _): (PathBuf, String, u64)| async move {
        let mut f = sftp_ref.open_with_flags(
            &remote,
            OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
        ).await.map_err(|e| AppError::Sftp(format!("open {remote}: {e}")))?;
        stream_to_remote(&local.to_string_lossy(), &mut f, &remote, progress_ref.file()).await?;
        f.shutdown().await
            .map_err(|e| AppError::Sftp(format!("close {remote}: {e}")))?;
        progress_ref.file_done();
        Ok(())
    }).await?;
    progress.finish();
    Ok(())
}

#[tauri::command]
pub async fn file_download_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    let sftp_ref = &sftp;
    let plan = scan_remote(remote_path.clone(), PathBuf::from(local_path), |dir| async move {
        let entries = sftp_ref.read_dir(&dir).await
            .map_err(|e| AppError::Sftp(format!("read_dir {dir}: {e}")))?;
        Ok(entries.into_iter().map(|e| {
            let meta = e.metadata();
            (e.file_name(), meta.is_dir(), meta.size.unwrap_or(0))
        }).collect())
    }).await?;
    for dir in &plan.dirs {
        tokio::fs::create_dir_all(dir).await?;
    }
    let (total, files_total) = (plan.total_bytes(), plan.files.len() as u64);
    let progress = DirProgress::new(PROGRESS_INTERVAL, |transferred, files_done| {
        emit_download_progress(&app, DownloadProgress {
            session_id: session_id.clone(),
            remote_path: remote_path.clone(),
            transferred,
            total,
            files_done: Some(files_done),
            files_total: Some(files_total),
        });
    });
    let progress_ref = &progress;
    download_dir_files(
        plan.files.into_iter().map(|(remote, local, size)| ((remote, local), size)).collect(),
        |(remote, local): (String, PathBuf)| async move {
            let mut rf = sftp_ref.open(&remote).await
                .map_err(|e| AppError::Sftp(format!("open {remote}: {e}")))?;
            let mut lf = tokio::fs::File::create(&local).await?;
            stream_to_local(&mut rf, &mut lf, &remote, progress_ref.file()).await?;
            progress_ref.file_done();
            Ok(())
        },
        |(remote, local): (String, PathBuf), size| async move {
            download_large(sftp_ref, &remote, &local, size, progress_ref.file()).await?;
            progress_ref.file_done();
            Ok(())
        },
    ).await?;
    progress.finish();
    Ok(())
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
    // Handle 锁只覆盖开通道，命令执行期间不占锁（同 ssh_execute）。
    let channel = {
        let handle = session.handle.lock().await;
        handle.channel_open_session().await
            .map_err(|e| AppError::Sftp(format!("open exec channel: {e}")))?
    };
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
    // Handle 锁只覆盖开通道，命令执行期间不占锁（同 ssh_execute）。
    let channel = {
        let handle = session.handle.lock().await;
        handle.channel_open_session().await
            .map_err(|e| AppError::Sftp(format!("open exec channel: {e}")))?
    };
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
    use super::{
        download_dir_files, download_segmented, follow_symlinks, join_remote, parse_id_map,
        scan_local, scan_remote, segment_ranges, shell_quote, stream_to_local, stream_to_remote,
        transfer_files_concurrently, DirProgress, DOWNLOAD_SEGMENTS, SEGMENTED_DOWNLOAD_MIN,
        TRANSFER_CHUNK,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;
    use crate::error::{AppError, AppResult};
    use russh_sftp::client::fs::Metadata;
    use std::io::{self, SeekFrom};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};

    /// OpenSSH sftp-server 的 max-read-length / max-write-length（256KiB − 1KiB）。
    const OPENSSH_MAX_RW_LEN: usize = 261_120;

    /// 模拟远端文件句柄：每个 READ 先挂起一轮（像真实往返一样让出执行权，借此统计在途峰值），
    /// 再返回 [pos, pos + len) 的数据。
    struct MockRemote {
        data: Arc<Vec<u8>>,
        pos: u64,
        waiting: bool,
        in_flight: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl AsyncRead for MockRemote {
        fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            if !this.waiting {
                this.waiting = true;
                let now = this.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                this.peak.fetch_max(now, Ordering::SeqCst);
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            this.waiting = false;
            this.in_flight.fetch_sub(1, Ordering::SeqCst);
            let start = (this.pos as usize).min(this.data.len());
            let n = buf.remaining().min(this.data.len() - start);
            buf.put_slice(&this.data[start..start + n]);
            this.pos += n as u64;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncSeek for MockRemote {
        fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
            let SeekFrom::Start(pos) = position else { return Err(io::Error::other("unsupported seek")) };
            self.get_mut().pos = pos;
            Ok(())
        }
        fn poll_complete(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<u64>> {
            Poll::Ready(Ok(self.pos))
        }
    }

    /// 模拟本地文件：多个句柄共享同一块缓冲，各自按 seek 到的偏移写入。
    struct MockLocal {
        buf: Arc<Mutex<Vec<u8>>>,
        pos: u64,
    }

    impl AsyncWrite for MockLocal {
        fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
            let this = self.get_mut();
            let start = this.pos as usize;
            let mut buf = this.buf.lock().unwrap();
            if buf.len() < start + data.len() {
                buf.resize(start + data.len(), 0);
            }
            buf[start..start + data.len()].copy_from_slice(data);
            this.pos += data.len() as u64;
            Poll::Ready(Ok(data.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncSeek for MockLocal {
        fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
            let SeekFrom::Start(pos) = position else { return Err(io::Error::other("unsupported seek")) };
            self.get_mut().pos = pos;
            Ok(())
        }
        fn poll_complete(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<u64>> {
            Poll::Ready(Ok(self.pos))
        }
    }

    /// 用 DOWNLOAD_SEGMENTS 跑一次分段下载：远端内容是 `remote`，按 `total` 切段。
    async fn run_segmented(remote: Vec<u8>, total: u64) -> (AppResult<u64>, Vec<u8>, usize, u64) {
        let data = Arc::new(remote);
        let local = Arc::new(Mutex::new(Vec::new()));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let last_progress = AtomicU64::new(0);
        let result = download_segmented(
            total,
            DOWNLOAD_SEGMENTS,
            || {
                let remote = MockRemote {
                    data: data.clone(),
                    pos: 0,
                    waiting: false,
                    in_flight: in_flight.clone(),
                    peak: peak.clone(),
                };
                async move { Ok(remote) }
            },
            || {
                let file = MockLocal { buf: local.clone(), pos: 0 };
                async move { Ok(file) }
            },
            "/r",
            |t| last_progress.store(t, Ordering::SeqCst),
        ).await;
        let bytes = local.lock().unwrap().clone();
        (result, bytes, peak.load(Ordering::SeqCst), last_progress.load(Ordering::SeqCst))
    }

    /// 非常量的字节模式：周期 251 与块大小互质，段错位或重叠时一定比对不上。
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn segment_ranges_are_chunk_aligned_and_cover_the_file() {
        let chunk = TRANSFER_CHUNK as u64;
        let total = 10 * chunk + 123;
        let ranges = segment_ranges(total, 4);
        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[0].0, 0);
        assert_eq!(ranges.last().unwrap().1, total);
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "segments must be contiguous");
        }
        for (start, _) in &ranges {
            assert_eq!(start % chunk, 0, "segment starts must be chunk aligned");
        }
        // 文件太小时段数自动收缩，不产生空段。
        assert_eq!(segment_ranges(chunk + 1, 4), vec![(0, chunk), (chunk, chunk + 1)]);
    }

    #[tokio::test]
    async fn segmented_download_runs_segments_concurrently_and_reassembles_bytes() {
        let source = pattern(3 * TRANSFER_CHUNK + 4321);
        let total = source.len() as u64;
        let (result, bytes, peak, last_progress) = run_segmented(source.clone(), total).await;
        assert_eq!(result.unwrap(), total);
        // 各段 READ 必须同时在途，串行时吞吐仍是单句柄的块大小 / RTT。
        assert!(peak > 1, "segments were read serially (peak in-flight = {peak})");
        assert!(bytes == source, "reassembled bytes differ from the remote file");
        assert_eq!(last_progress, total);
    }

    #[tokio::test]
    async fn segmented_download_errors_when_file_shrinks_mid_transfer() {
        // stat 时 4 块多，读的时候只剩 1 块：中间段会提前 EOF。
        let total = 4 * TRANSFER_CHUNK as u64 + 10;
        let (result, _, _, _) = run_segmented(pattern(TRANSFER_CHUNK), total).await;
        assert!(matches!(result, Err(AppError::Sftp(_))), "shrunk file must not download silently");
    }

    #[tokio::test]
    async fn directory_files_transfer_concurrently_and_propagate_errors() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        let run = |fail_on: Option<usize>| {
            let (in_flight, peak, done) = (in_flight.clone(), peak.clone(), done.clone());
            transfer_files_concurrently((0..8).collect(), move |i: usize| {
                let (in_flight, peak, done) = (in_flight.clone(), peak.clone(), done.clone());
                async move {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    if Some(i) == fail_on {
                        return Err(AppError::Sftp(format!("file {i} failed")));
                    }
                    done.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
        };

        run(None).await.unwrap();
        assert_eq!(done.load(Ordering::SeqCst), 8);
        // 同层文件必须并发：每个小文件的 open/close 各一个 RTT，串行就是 N 轮往返。
        assert!(peak.load(Ordering::SeqCst) > 1, "directory files were transferred serially");
        assert!(matches!(run(Some(3)).await, Err(AppError::Sftp(_))));
    }

    #[tokio::test]
    async fn directory_download_segments_large_files_one_at_a_time() {
        let min = SEGMENTED_DOWNLOAD_MIN;
        let in_flight = Arc::new(AtomicUsize::new(0));
        let small_seen = Arc::new(Mutex::new(Vec::new()));
        let large_seen = Arc::new(Mutex::new(Vec::new()));
        let files = vec![("a", 10), ("big1", min), ("b", min - 1), ("big2", 3 * min), ("c", 0)];

        download_dir_files(
            files,
            |name| {
                let (in_flight, seen) = (in_flight.clone(), small_seen.clone());
                async move {
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    seen.lock().unwrap().push(name);
                    Ok(())
                }
            },
            |name, size| {
                let (in_flight, seen) = (in_flight.clone(), large_seen.clone());
                async move {
                    // 大文件自带 DOWNLOAD_SEGMENTS 路在途 READ，开始时不能还有别的文件在传，
                    // 否则在途总量翻倍，慢链路上排在后面的 READ 会撞 SFTP 读超时。
                    assert_eq!(in_flight.fetch_add(1, Ordering::SeqCst), 0, "{name} overlapped another transfer");
                    tokio::task::yield_now().await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    seen.lock().unwrap().push((name, size));
                    Ok(())
                }
            },
        ).await.unwrap();

        let mut small = small_seen.lock().unwrap().clone();
        small.sort();
        assert_eq!(small, ["a", "b", "c"]);
        // 大文件必须走分段：单句柄一问一答的吞吐只有块大小 / RTT。
        assert_eq!(*large_seen.lock().unwrap(), [("big1", min), ("big2", 3 * min)]);

        let failed = download_dir_files(
            vec![("big", min)],
            |_| async { Ok(()) },
            |name, _| async move { Err(AppError::Sftp(format!("{name} failed"))) },
        ).await;
        assert!(matches!(failed, Err(AppError::Sftp(_))));
    }

    #[derive(Default)]
    struct RecordingWriter {
        writes: Vec<usize>,
    }

    impl AsyncWrite for RecordingWriter {
        fn poll_write(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            self.writes.push(buf.len());
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// 模拟远端文件：记录每次 poll_read 请求的长度（russh-sftp 据此决定 READ 请求大小）。
    struct RecordingReader {
        remaining: usize,
        requested: Vec<usize>,
    }

    impl AsyncRead for RecordingReader {
        fn poll_read(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            self.requested.push(buf.remaining());
            let n = buf.remaining().min(self.remaining);
            buf.put_slice(&vec![7u8; n]);
            self.remaining -= n;
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn upload_writes_one_max_size_sftp_request_per_chunk() {
        let path = std::env::temp_dir().join(format!("sshl-upload-{}", uuid::Uuid::new_v4()));
        let size = OPENSSH_MAX_RW_LEN * 2 + 10;
        std::fs::write(&path, vec![1u8; size]).unwrap();
        let mut writer = RecordingWriter::default();
        let mut last = 0;
        let result = stream_to_remote(path.to_str().unwrap(), &mut writer, "/r", |t| last = t).await;
        std::fs::remove_file(&path).ok();
        result.unwrap();
        // 每块恰好占满一个 SFTP WRITE（上传有 8 路 pipeline，块越大在途数据越多），
        // 且不超过服务端上限，不会被拆成「一大一小」两个请求浪费 pipeline 槽位。
        assert_eq!(writer.writes, vec![OPENSSH_MAX_RW_LEN, OPENSSH_MAX_RW_LEN, 10]);
        assert_eq!(last, size as u64);
    }

    #[tokio::test]
    async fn download_requests_max_size_sftp_reads() {
        let size = OPENSSH_MAX_RW_LEN * 2 + 10;
        let mut reader = RecordingReader { remaining: size, requested: Vec::new() };
        let mut local: Vec<u8> = Vec::new();
        let mut last = 0;
        let transferred = stream_to_local(&mut reader, &mut local, "/r", |t| last = t).await.unwrap();
        // 下载是一问一答，吞吐 ≈ 单次 READ 大小 / RTT：每次都要按服务端上限请求。
        assert_eq!(reader.requested[0], OPENSSH_MAX_RW_LEN);
        assert_eq!(transferred, size as u64);
        assert_eq!(last, size as u64);
        assert_eq!(local.len(), size);
    }

    fn meta(permissions: u32) -> Metadata {
        let mut m = Metadata::empty();
        m.permissions = Some(permissions);
        m
    }

    #[tokio::test]
    async fn follow_symlinks_stats_links_concurrently_in_order() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let stated = Arc::new(Mutex::new(Vec::new()));
        let link = |name: &str| (name.to_string(), format!("/d/{name}"), meta(0o120777));
        let entries = vec![
            link("a"),
            ("f".to_string(), "/d/f".to_string(), meta(0o100644)),
            link("b"),
            link("dangling"),
            link("c"),
        ];

        let out = follow_symlinks(entries, |full| {
            let (in_flight, peak, stated) = (in_flight.clone(), peak.clone(), stated.clone());
            async move {
                stated.lock().unwrap().push(full.clone());
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::task::yield_now().await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                if full.ends_with("dangling") { None } else { Some(meta(0o040755)) }
            }
        }).await;

        // 每个 STAT 一个 RTT：必须并发发出，串行时几百个链接的目录要几百个 RTT。
        assert!(peak.load(Ordering::SeqCst) > 1, "symlink STATs ran serially");
        let names: Vec<&str> = out.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, ["a", "f", "b", "dangling", "c"]);
        assert!(out[0].2.is_dir() && out[2].2.is_dir() && out[4].2.is_dir());
        // 普通文件不发 STAT；悬空链接保留自身元数据。
        assert_eq!(out[1].2.permissions, Some(0o100644));
        assert_eq!(out[3].2.permissions, Some(0o120777));
        assert_eq!(stated.lock().unwrap().len(), 4);
    }

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

    #[test]
    fn dir_progress_sums_per_file_cumulative_reports() {
        let emitted = Mutex::new(Vec::new());
        let progress = DirProgress::new(Duration::ZERO, |transferred, files_done| {
            emitted.lock().unwrap().push((transferred, files_done));
        });
        let (a, b) = (progress.file(), progress.file());
        // 各文件回报的是自己的累计字节：a 先 10 后 30，只能再计入 20。
        a(10);
        b(5);
        a(30);
        progress.file_done();
        b(20);
        progress.file_done();
        progress.finish();
        assert_eq!(
            *emitted.lock().unwrap(),
            [(10, 0), (15, 0), (35, 0), (35, 1), (50, 1), (50, 2), (50, 2)]
        );
    }

    #[test]
    fn dir_progress_throttles_but_always_emits_on_finish() {
        let emitted = Mutex::new(Vec::new());
        let progress = DirProgress::new(Duration::from_secs(3600), |transferred, files_done| {
            emitted.lock().unwrap().push((transferred, files_done));
        });
        let file = progress.file();
        file(10);
        file(30);
        progress.file_done();
        assert!(emitted.lock().unwrap().is_empty(), "节流期内不应发事件");
        progress.finish();
        assert_eq!(*emitted.lock().unwrap(), [(30, 1)]);
    }

    #[tokio::test]
    async fn scan_local_lists_dirs_parent_first_with_file_sizes() {
        let root = std::env::temp_dir().join(format!("sshl-scan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
        std::fs::write(root.join("a.txt"), b"abc").unwrap();
        std::fs::write(root.join("sub/b.bin"), b"12345").unwrap();

        let plan = scan_local(root.clone(), "/r".to_string()).await;
        std::fs::remove_dir_all(&root).unwrap();
        let plan = plan.unwrap();

        // 先建父目录再建子目录，否则远端 mkdir 子目录会失败。
        assert_eq!(plan.dirs, ["/r", "/r/sub", "/r/sub/deeper"]);
        let mut files = plan.files.clone();
        files.sort();
        assert_eq!(
            files,
            [
                (root.join("a.txt"), "/r/a.txt".to_string(), 3),
                (root.join("sub/b.bin"), "/r/sub/b.bin".to_string(), 5),
            ]
        );
        assert_eq!(plan.total_bytes(), 8);
    }

    #[tokio::test]
    async fn scan_remote_walks_listing_and_skips_dot_entries() {
        let tree: HashMap<&str, Vec<(&str, bool, u64)>> = HashMap::from([
            ("/r", vec![(".", true, 0), ("..", true, 0), ("a.txt", false, 3), ("sub", true, 0)]),
            ("/r/sub", vec![(".", true, 0), ("b.bin", false, 5), ("deeper", true, 0)]),
            ("/r/sub/deeper", vec![]),
        ]);
        let list = |dir: String| {
            let entries = tree.get(dir.as_str()).cloned();
            async move {
                let entries = entries.ok_or_else(|| AppError::Sftp(format!("read_dir {dir}: no such file")))?;
                Ok(entries.into_iter().map(|(n, d, s)| (n.to_string(), d, s)).collect())
            }
        };

        let plan = scan_remote("/r".to_string(), PathBuf::from("/l"), list).await.unwrap();
        assert_eq!(plan.dirs, [PathBuf::from("/l"), PathBuf::from("/l/sub"), PathBuf::from("/l/sub/deeper")]);
        assert_eq!(
            plan.files,
            [
                ("/r/a.txt".to_string(), PathBuf::from("/l/a.txt"), 3),
                ("/r/sub/b.bin".to_string(), PathBuf::from("/l/sub/b.bin"), 5),
            ]
        );
        assert_eq!(plan.total_bytes(), 8);

        let missing = scan_remote("/nope".to_string(), PathBuf::from("/l"), list).await;
        assert!(matches!(missing, Err(AppError::Sftp(msg)) if msg.contains("/nope")));
    }
}

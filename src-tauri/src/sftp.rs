// SFTP backend — russh-sftp on top of an existing SSH session.
// Per-session SftpSession is cached on SshSession.sftp to avoid Channel accumulation
// (mirrors Electron-side fix in commits 8cb552c / a819e15).

use std::collections::HashMap;
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

use crate::config_store::load_secret;
use crate::terminal_user::{self, Elevation, Via};
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

/// 一条 SFTP 通道上同时在途的数据上限 ≈ SSH 默认通道窗口（russh 2MB）：再多也只是在窗口外排队，
/// 还让排在后面的请求（OPEN / CLOSE / READ 的回复）更晚才到，慢链路上撞 SFTP 请求超时（见 new_sftp）。
const IN_FLIGHT_BYTES: u64 = 8 * TRANSFER_CHUNK as u64;

/// 大文件分段并发下载的段数。单句柄下载一问一答（吞吐 ≈ 块大小 / RTT），
/// 开 N 个句柄读不相交区间，在途 READ 就是 N 个：8 × 255KB = IN_FLIGHT_BYTES ≈ SSH 通道窗口，
/// 与上传的 8 路写 pipeline 在途量持平。
const DOWNLOAD_SEGMENTS: usize = (IN_FLIGHT_BYTES / TRANSFER_CHUNK as u64) as usize;

/// 小于此大小走单句柄流式下载。按 RTT 算（块 = TRANSFER_CHUNK）：单句柄 = open + ⌈S/块⌉ 个 READ
/// + 1 个读到 EOF 的 READ；分段 = 段 0 open → 其余段 open → 每段 ⌈S/(段数·块)⌉ 个 READ（只有最后一段
/// 多读一次 EOF）。2 块时两者打平、分段还多开句柄；4 块时分段 4 个 RTT 对单句柄 6 个。再往上调
/// 只会让 1–2MB 的文件白等往返，所以保持 4 块（≈1MB）。
const SEGMENTED_DOWNLOAD_MIN: u64 = 4 * TRANSFER_CHUNK as u64;

/// 目录传输同时在传的文件数上限。小文件的耗时几乎全是 open / 读写 / close 的往返（各一个 RTT），
/// 串行时几千个小文件就是几千轮往返；数据量另由 IN_FLIGHT_BYTES 限，这里限的是同时开着的句柄数
/// （远端远低于 OpenSSH 的 1019，本地远低于 macOS 默认 256 个 fd）。50ms RTT、3000 个小文件实测：
/// 16 路上传 / 下载 33s / 36s，32 路 17s / 19s。
const DIR_FILE_CONCURRENCY: usize = 32;

/// 目录传输时同一层内建目录 / 列目录的并发度。每个 MKDIR 一个 RTT，每次列目录至少三个
/// （opendir、readdir、close），请求和回复都很小。
const DIR_OP_CONCURRENCY: usize = 32;

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
    // Open new SFTP subsystem channel and cache it.
    let channel = open_exec_channel(session).await?;
    let elevation = session.elevation.lock().await.clone();
    let sftp = match elevation {
        // 终端切换了用户：以他的身份起 sftp-server（缓存的会话断了也按同一身份重开）
        Some(e) => open_sftp_as(channel, &e.user, e.via, e.password.as_deref()).await?,
        None => {
            channel.request_subsystem(true, "sftp").await
                .map_err(|e| AppError::Sftp(format!("request sftp subsystem: {e}")))?;
            new_sftp(channel).await?
        }
    };
    let sftp = Arc::new(sftp);
    let mut cache = session.sftp.lock().await;
    // 打开期间刚切换过身份（file_switch_user 已放好新会话）：以缓存里的为准
    if let Some(existing) = cache.as_ref() {
        return Ok(existing.clone());
    }
    *cache = Some(sftp.clone());
    Ok(sftp)
}

async fn new_sftp(channel: russh::Channel<russh::client::Msg>) -> AppResult<SftpSession> {
    let sftp = SftpSession::new(channel.into_stream()).await
        .map_err(|e| AppError::Sftp(format!("sftp init: {e}")))?;
    sftp.set_timeout(SFTP_TIMEOUT_SECS);
    Ok(sftp)
}

/// SFTP 请求超时（秒）。russh-sftp 的请求从发出起计时，默认 10s（写请求不计时）。分段下载、目录并发传输
/// 会让至多 IN_FLIGHT_BYTES（≈2MB）的数据同时在途，排在后面的请求 / 回复要等它们先过完链路：
/// 2MB / 60s ≈ 35KB/s，链路不慢于此就不会误超时。
const SFTP_TIMEOUT_SECS: u64 = 60;

/// 在 exec 通道上以 `user` 身份起 sftp-server（见 terminal_user::sftp_server_script），返回 SFTP 会话。
/// 等到 stderr 出现密码提示才写密码、出现就绪标记才开始 SFTP 协议：免密 sudo 不会把密码误当成 SFTP 数据，
/// 密码错（sudo 再次提示 / su 报 Authentication failure）在协议开始前就能识别。
async fn open_sftp_as(
    mut channel: russh::Channel<russh::client::Msg>,
    user: &str,
    via: Via,
    password: Option<&str>,
) -> AppResult<SftpSession> {
    let ready = if via == Via::Su && password.is_none() {
        // 没密码就不跑 su：空密码同样会记一次认证失败
        Err(AppError::Other(format!("SUDO_PASSWORD_REQUIRED: 需要 {user} 的密码")))
    } else {
        wait_for_sftp_ready(&mut channel, user, via, password).await
    };
    if let Err(e) = ready {
        // russh 的 Channel 丢弃时不会关通道：不显式关，密码错后 sudo 会一直等第二次输入、
        // 占着该用户的 sudo 时间戳锁，之后所有 sudo 都卡住
        let _ = channel.eof().await;
        let _ = channel.close().await;
        return Err(e);
    }
    new_sftp(channel).await
}

async fn wait_for_sftp_ready(
    channel: &mut russh::Channel<russh::client::Msg>,
    user: &str,
    via: Via,
    password: Option<&str>,
) -> AppResult<()> {
    let cmd = terminal_user::run_as_command(user, via, password.is_some(), &terminal_user::sftp_server_script());
    channel.exec(true, cmd.as_bytes()).await
        .map_err(|e| AppError::Ssh(format!("exec: {e}")))?;
    let prompt = if via == Via::Sudo { terminal_user::SUDO_PROMPT } else { terminal_user::SU_PROMPT };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let (mut log, mut answered) = (String::new(), 0usize);
    loop {
        let Ok(msg) = tokio::time::timeout_at(deadline, channel.wait()).await else {
            return Err(AppError::Other(format!("SUDO_FAILED: 切换到 {user} 超时 {}", terminal_user::clean_auth_noise(&log))));
        };
        match msg {
            Some(russh::ChannelMsg::Data { data }) | Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                log.push_str(&String::from_utf8_lossy(&data));
                if log.contains(terminal_user::SFTP_READY) {
                    return Ok(());
                }
                let prompts = log.matches(prompt).count();
                if prompts > answered {
                    if answered > 0 {
                        return Err(AppError::Other(format!("SUDO_PASSWORD_INCORRECT: {}", terminal_user::clean_auth_noise(&log))));
                    }
                    let pw = password.ok_or_else(|| AppError::Other("SUDO_PASSWORD_REQUIRED: 需要密码".into()))?;
                    channel.data(format!("{pw}\n").as_bytes()).await
                        .map_err(|e| AppError::Ssh(format!("exec stdin: {e}")))?;
                    answered = prompts;
                }
            }
            Some(russh::ChannelMsg::ExitStatus { .. }) | Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => {
                if log.contains(terminal_user::NO_SFTP_SERVER) {
                    return Err(AppError::Other("SUDO_FAILED: 服务器上找不到 sftp-server，无法以该用户身份管理文件".into()));
                }
                let text = terminal_user::clean_auth_noise(&log);
                let marker = if via == Via::Sudo { sudo_failure(&text) } else { terminal_user::su_failure(&text) };
                return Err(AppError::Other(format!("{marker}: {text}")));
            }
            _ => {}
        }
    }
}

/// 以文件管理当前身份跑一条命令（删除 / chmod / chown）：终端切换过用户就经 su / sudo 以他身份执行。
async fn exec_as_file_user(session: &SshSession, cmd: &str) -> AppResult<(u32, String, String)> {
    let elevation = session.elevation.lock().await.clone();
    let channel = open_exec_channel(session).await?;
    let Some(e) = elevation else { return exec_capture(channel, cmd, None).await };
    let input = e.password.as_ref().map(|p| format!("{p}\n"));
    let wrapped = terminal_user::run_as_command(&e.user, e.via, e.password.is_some(), cmd);
    let (status, out, err) = exec_capture(channel, &wrapped, input.as_deref().map(str::as_bytes)).await?;
    Ok((status, terminal_user::clean_auth_noise(&out), terminal_user::clean_auth_noise(&err)))
}

async fn run_as_file_user(session: &SshSession, cmd: &str) -> AppResult<String> {
    let (status, out, err) = exec_as_file_user(session, cmd).await?;
    if status != 0 {
        return Err(AppError::Other(if err.is_empty() { format!("`{cmd}` exited {status}") } else { err }));
    }
    Ok(out)
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
    upload_file_to(&app, &sftp, &session_id, &local_path, &remote_path, &remote_path).await
}

/// 上传单个文件到 `remote_path`；进度事件按 `progress_path` 上报（sudo 上传写暂存区，但进度仍报目标路径）。
async fn upload_file_to(
    app: &AppHandle,
    sftp: &SftpSession,
    session_id: &str,
    local_path: &str,
    remote_path: &str,
    progress_path: &str,
) -> AppResult<()> {
    let total = tokio::fs::metadata(local_path).await.map(|m| m.len()).unwrap_or(0);
    let mut file = sftp.open_with_flags(
        remote_path,
        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
    ).await.map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?;
    // 进度事件按 100ms 节流，避免大文件刷爆 webview 事件队列（与下载侧一致）。
    let mut last_emit = std::time::Instant::now();
    stream_to_remote(local_path, &mut file, remote_path, |transferred| {
        if last_emit.elapsed() >= std::time::Duration::from_millis(100) {
            emit_upload_progress(app, UploadProgress {
                session_id: session_id.to_string(),
                remote_path: progress_path.to_string(),
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
    emit_upload_progress(app, UploadProgress {
        session_id: session_id.to_string(),
        remote_path: progress_path.to_string(),
        transferred: total,
        total,
        files_done: None,
        files_total: None,
    });
    Ok(())
}

/// 远程路径拆成（父目录, 名字）。"/a/b/" → ("/a", "b")，"/x" → ("/", "x")。
fn split_remote(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim_end_matches('/');
    let idx = trimmed.rfind('/')?;
    let name = &trimmed[idx + 1..];
    if name.is_empty() {
        return None;
    }
    let dir = if idx == 0 { "/" } else { &trimmed[..idx] };
    Some((dir.to_string(), name.to_string()))
}

/// `need_user` 自己的已保存密码标识：登录用户的就是连接密码，否则找以用户名保存的账号密码（`acct:<用户名>`）。
/// 只认这一份，不拿其他已保存的密码去试（每次试错都会记进服务器的认证日志、可能触发锁定）。
fn saved_password_for(connection_id: Option<&str>, login_user: &str, need_user: &str, has: impl Fn(&str, &str) -> bool) -> Option<String> {
    let cid = connection_id?;
    let login = (need_user == login_user).then(|| "password".to_string());
    login.into_iter().chain([format!("acct:{need_user}")]).find(|kind| has(cid, kind))
}

/// 本次 sudo 用哪份密码：点选的已保存密码（`credential` = "password" / "acct:<账号>"，只传标识，
/// 后端解密，不经过渲染层）> 手动输入 > 无（`sudo -n`）。用谁的密码由前端按 `file_sudo_identity` 决定。
fn pick_sudo_password(
    connection_id: Option<&str>,
    credential: Option<&str>,
    typed: Option<String>,
    load: impl Fn(&str, &str) -> Option<String>,
) -> AppResult<Option<String>> {
    if let Some(kind) = credential {
        let cid = connection_id.ok_or_else(|| AppError::Other("当前连接未保存，没有已保存的密码".into()))?;
        return load(cid, kind).map(Some).ok_or_else(|| AppError::Other("未找到对应的已保存密码".into()));
    }
    Ok(typed.filter(|p| !p.is_empty()))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SudoIdentity {
    /// 登录用户名：sudo 要的就是这个用户的密码。
    pub user: String,
    /// 该用户自己的已保存密码标识，没有为 None。
    pub credential: Option<String>,
}

/// sudo 上传前先确认当前登录用户是谁、有没有保存他自己的密码。
#[tauri::command]
pub async fn file_sudo_identity(state: State<'_, AppState>, session_id: String) -> AppResult<SudoIdentity> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let credential = saved_password_for(session.connection_id.as_deref(), &session.username, &session.username, crate::crypto_store::has);
    Ok(SudoIdentity { user: session.username.clone(), credential })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalIdentity {
    pub login_user: String,
    /// 终端此刻的用户（su / sudo -i 之后）。
    pub user: String,
    /// 怎么切过去的："su"（要 user 的密码）/ "sudo"（要登录用户的密码）；没切换为 None。
    pub via: Option<&'static str>,
    /// 这次需要的那份密码有没有保存：su → user 的，sudo / 未切换 → 登录用户的。
    pub credential: Option<String>,
    /// 文件管理当前以谁的身份运行。
    pub file_user: String,
}

/// uid → 用户名：先查已缓存的 /etc/passwd，没有再问服务器（getent 也覆盖 LDAP 等）。
async fn user_name(session: &SshSession, uid: u32) -> String {
    if let Some(name) = session.passwd_cache.lock().await.as_ref().and_then(|m| m.get(&uid).cloned()) {
        return name;
    }
    if uid == 0 {
        return "root".into();
    }
    if let Ok(channel) = open_exec_channel(session).await {
        // 管道的退出码是 cut 的：getent 查不到时用 grep . 让它失败，才会退到 id -nu
        let cmd = format!("getent passwd {uid} 2>/dev/null | cut -d: -f1 | grep . || id -nu {uid}");
        if let Ok((_, out, _)) = exec_capture(channel, &cmd, None).await {
            let name = out.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    uid.to_string()
}

/// 终端此刻是谁（见 terminal_user）。文件管理据此决定是否切换身份。
#[tauri::command]
pub async fn file_terminal_user(state: State<'_, AppState>, session_id: String) -> AppResult<TerminalIdentity> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let login_user = session.username.clone();
    let file_user = session.elevation.lock().await.as_ref().map_or_else(|| login_user.clone(), |e| e.user.clone());
    // 探测一律以登录用户身份跑（exec 的 $PPID 才是这条连接的 sshd 会话进程）
    let (_, out, _) = exec_capture(open_exec_channel(&session).await?, terminal_user::PROBE_COMMAND, None).await?;
    let found = terminal_user::parse_probe(&out).and_then(|(sshd, procs)| terminal_user::terminal_user(sshd, &procs));
    let (user, via) = match found {
        Some((uid, Some(via))) => (user_name(&session, uid).await, Some(via)),
        _ => (login_user.clone(), None),
    };
    let need = if via == Some(Via::Su) { user.as_str() } else { login_user.as_str() };
    let credential = saved_password_for(session.connection_id.as_deref(), &login_user, need, crate::crypto_store::has);
    Ok(TerminalIdentity { login_user, user, via: via.map(Via::as_str), credential, file_user })
}

/// 切换文件管理的身份。`user` 为 None 或登录用户 → 回到登录用户；否则以 `via`（su / sudo）切过去，
/// 密码：点选的已保存密码 > 手动输入 > 无（仅免密 sudo）。成功后这条连接的 SFTP、删除、chmod、chown 都以新身份进行。
#[tauri::command]
pub async fn file_switch_user(
    state: State<'_, AppState>,
    session_id: String,
    user: Option<String>,
    via: Option<String>,
    password: Option<String>,
    credential: Option<String>,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let Some(user) = user.filter(|u| *u != session.username) else {
        *session.elevation.lock().await = None;
        *session.sftp.lock().await = None;
        return Ok(());
    };
    let via = if via.as_deref() == Some("sudo") { Via::Sudo } else { Via::Su };
    let password = pick_sudo_password(session.connection_id.as_deref(), credential.as_deref(), password, load_secret)?;
    let sftp = open_sftp_as(open_exec_channel(&session).await?, &user, via, password.as_deref()).await?;
    *session.elevation.lock().await = Some(Arc::new(Elevation { user, via, password }));
    *session.sftp.lock().await = Some(Arc::new(sftp));
    Ok(())
}

/// 以文件管理当前身份跑命令（远程删除用）。
#[tauri::command]
pub async fn file_shell(state: State<'_, AppState>, session_id: String, command: String) -> AppResult<String> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    run_as_file_user(&session, &command).await
}

/// sudo_prepare_command 建暂存区失败时的退出码（`sudo true` 自身失败只会是 1，sh 找不到命令是 126/127）。
const STAGING_FAILED: u32 = 3;

/// sudo 上传第一步，一条 exec：先验 sudo 能不能用（密码对不对、有没有权限；`sudo true` 不做任何事，
/// 放在上传之前，密码错不会白传一遍大文件），过了再以登录用户身份建暂存区，stdout 即其路径。
/// /var/tmp 落盘（/tmp 常是内存盘，大文件会撑满），不可用再退回 /tmp。mktemp -d 建出来即 0700。
/// sudo 失败按 sudo 的退出码退出，建暂存区失败退出 STAGING_FAILED，见 staging_from_prepare。
/// 包一层 sh：登录 shell 不一定是 POSIX sh。
fn sudo_prepare_command(with_password: bool) -> String {
    let auth = if with_password { "-S -p \"\"" } else { "-n" };
    format!(
        "sh -c 'LC_ALL=C sudo {auth} true || exit; \
         mktemp -d /var/tmp/.sshl-upload.XXXXXXXX 2>/dev/null || mktemp -d /tmp/.sshl-upload.XXXXXXXX || exit {STAGING_FAILED}'"
    )
}

/// sudo_prepare_command 的结果 → 暂存区路径。sudo 失败带上前端认的 SUDO_* 标记，建暂存区失败不带。
fn staging_from_prepare(status: u32, out: &str, err: &str) -> AppResult<String> {
    let staging = out.trim();
    if status == STAGING_FAILED || (status == 0 && staging.is_empty()) {
        return Err(AppError::Other(format!("创建暂存目录失败: {}", err.trim())));
    }
    if status != 0 {
        return Err(AppError::Other(format!("{}: {}", sudo_failure(err), err.trim())));
    }
    Ok(staging.to_string())
}

/// 登录用户没有目标目录写权限时的上传（前端在普通上传报 Permission denied 后、用户确认才调用）：
/// 1) 先验 sudo、建 0700 暂存区（sudo_prepare_command）；2) 以登录用户身份传到暂存区；
/// 3) sudo 以目标目录属主身份解包到位，无论成败都删暂存区（sudo_extract_command）。共两条 exec。
/// sudo 密码：前端传入 > 连接保存的登录密码 > 免密 sudo（-n）。
#[tauri::command]
pub async fn file_upload_sudo(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    local_path: String,
    remote_path: String,
    is_directory: bool,
    sudo_password: Option<String>,
    credential: Option<String>,
) -> AppResult<()> {
    let session = state.get(&session_id).await
        .ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
    let sftp = sftp_for(&session).await?;
    let (dest_dir, name) = split_remote(&remote_path)
        .ok_or_else(|| AppError::Other(format!("无效的上传路径: {remote_path}")))?;
    let password = pick_sudo_password(session.connection_id.as_deref(), credential.as_deref(), sudo_password, load_secret)?;
    let input = password.as_ref().map(|p| format!("{p}\n"));
    let stdin = input.as_deref().map(str::as_bytes);

    let (status, out, err) = exec_capture(open_exec_channel(&session).await?, &sudo_prepare_command(password.is_some()), stdin).await?;
    let staging = staging_from_prepare(status, &out, &err)?;

    let extracted = async {
        let staged = join_remote(&staging, &name);
        if is_directory {
            upload_tree_to(&app, &sftp, &session_id, &local_path, &staged, &remote_path).await?;
        } else {
            upload_file_to(&app, &sftp, &session_id, &local_path, &staged, &remote_path).await?;
        }
        let cmd = sudo_extract_command(password.is_some(), &staging, &dest_dir, &name);
        exec_capture(open_exec_channel(&session).await?, &cmd, stdin).await
    }.await;
    let (status, _, err) = match extracted {
        Ok(done) => done,
        Err(e) => {
            // 没走到「解包 + 删暂存区」那条命令（上传失败 / 通道出错）：单独删暂存区
            if let Ok(channel) = open_exec_channel(&session).await {
                let _ = exec_capture(channel, &format!("rm -rf -- {}", shell_quote(&staging)), None).await;
            }
            return Err(e);
        }
    };
    if status != 0 {
        return Err(AppError::Other(format!("{}: {}", sudo_failure(&err), err.trim())));
    }
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

/// 关掉读完的远端句柄，不等回复。只 drop 也会发 CLOSE，但 russh-sftp 只有等到 CLOSE 的回复才给已开句柄
/// 计数减一，drop 掉的句柄永远占着计数：服务端声明了句柄上限（OpenSSH limits 扩展为 1019）时，同一 SFTP
/// 会话累计下载约一千个文件后，open / 列目录全部报 "handle limit reached"，只能重连。
fn close_in_background(mut file: russh_sftp::client::fs::File) {
    tokio::spawn(async move {
        let _ = file.shutdown().await;
    });
}

/// 单句柄流式下载到 `local_path`（小文件），返回累计字节数。
async fn download_small(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &Path,
    on_progress: impl FnMut(u64),
) -> AppResult<u64> {
    let mut remote = sftp.open(remote_path).await
        .map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?;
    let result = async {
        let mut local = tokio::fs::File::create(local_path).await?;
        stream_to_local(&mut remote, &mut local, remote_path, on_progress).await
    }.await;
    close_in_background(remote);
    result
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

/// 分段并发下载：段 0 用调用方已打开的远端句柄 `first`，其余段用 `open_remote` 各开一个；
/// 本地各段都用 `open_local` 开独立句柄。seek 到段首后顺序读写，返回累计字节数。本地文件不预分配长度——
/// 各段写到自己的偏移，最终长度自然等于实际写到的末尾。非最后一段提前 EOF 说明文件在下载中变短，
/// 直接报错，不留下零填充的空洞；最后一段不设上界读到 EOF。每段的远端句柄用完交给 `release` 关闭。
async fn download_segmented<R, W, OR, ORF, OL, OLF>(
    total: u64,
    segments: usize,
    first: R,
    open_remote: OR,
    open_local: OL,
    release: impl Fn(R),
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
    let (open_remote, open_local, release, transferred_ref, on_progress) =
        (&open_remote, &open_local, &release, &transferred, &on_progress);
    let mut first = Some(first);
    let workers: Vec<_> = ranges.into_iter().enumerate().map(|(i, (start, end))| {
        let opened = first.take();
        async move {
            let mut remote = match opened {
                Some(remote) => remote,
                None => open_remote().await?,
            };
            let result = async {
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
            }.await;
            release(remote);
            result
        }
    }).collect();
    // 没有段（total = 0）时段 0 的句柄没人用
    if let Some(unused) = first {
        release(unused);
    }
    futures::future::try_join_all(workers).await?;
    Ok(transferred.into_inner())
}

/// 大文件分段下载到 `local_path`。先开段 0 的远端句柄，成功了才创建 / 截断本地文件：能 stat 但
/// 没有读权限（如 root 的大日志）时，不留下 0 字节文件、也不截断同名的已有文件。这个句柄就是
/// 段 0，不再单独探测一次（省一对 open / close）；其余段之后各自打开。
async fn download_large(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &Path,
    total: u64,
    on_progress: impl Fn(u64),
) -> AppResult<u64> {
    let first = sftp.open(remote_path).await
        .map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))?;
    if let Err(e) = tokio::fs::File::create(local_path).await {
        close_in_background(first);
        return Err(e.into());
    }
    download_segmented(
        total,
        DOWNLOAD_SEGMENTS,
        first,
        move || async move {
            sftp.open(remote_path).await
                .map_err(|e| AppError::Sftp(format!("open remote {remote_path}: {e}")))
        },
        move || async move {
            Ok(tokio::fs::OpenOptions::new().write(true).open(local_path).await?)
        },
        close_in_background,
        remote_path,
        on_progress,
    ).await
}

/// 目录下载的文件：小文件按在途字节预算并发（单句柄一问一答，每个文件在途至多一个块）；
/// ≥ SEGMENTED_DOWNLOAD_MIN 的大文件在小文件之后逐个分段下载（自带 DOWNLOAD_SEGMENTS 路 READ）。
/// 两批不重叠，在途 READ 始终 ≤ IN_FLIGHT_BYTES，与单文件分段下载持平，SFTP 请求超时的推导不变。
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
    let small_files = small_files.into_iter().map(|(item, size)| (item, size.min(TRANSFER_CHUNK as u64))).collect();
    transfer_with_budget(small_files, DIR_FILE_CONCURRENCY, IN_FLIGHT_BYTES, small).await?;
    for (item, size) in large_files {
        large(item, size).await?;
    }
    Ok(())
}

/// 并发处理一批 (文件, 在途字节)，按原顺序放行：同时在传的不超过 `max_files` 个，在途字节之和不超过
/// `budget`（超过 budget 的按 budget 计，即独占）；队首放不下就等前面的传完。遇到第一个错误即返回。
async fn transfer_with_budget<T, F, Fut>(
    items: Vec<(T, u64)>,
    max_files: usize,
    budget: u64,
    transfer: F,
) -> AppResult<()>
where
    F: Fn(T) -> Fut,
    Fut: Future<Output = AppResult<()>>,
{
    let transfer = &transfer;
    let mut queue = items.into_iter().map(|(item, cost)| (item, cost.min(budget))).peekable();
    let mut running = stream::FuturesUnordered::new();
    let mut used = 0;
    loop {
        // 没有在传的时候队首一定放得下（cost 已截到 budget），所以不会空转
        while running.len() < max_files.max(1) && queue.peek().is_some_and(|(_, cost)| used + cost <= budget) {
            let (item, cost) = queue.next().unwrap();
            used += cost;
            running.push(async move { transfer(item).await.map(|()| cost) });
        }
        match running.next().await {
            Some(done) => used -= done?,
            None => return Ok(()),
        }
    }
}

/// 按层建目录：同层并发（父目录都在上一层），层与层之间按顺序；任一失败即停。
async fn create_dir_levels<'a, D, F, Fut>(levels: &'a [Vec<D>], create: F) -> AppResult<()>
where
    F: Fn(&'a D) -> Fut,
    Fut: Future<Output = AppResult<()>>,
{
    use futures::TryStreamExt;
    for level in levels {
        stream::iter(level).map(Ok).try_for_each_concurrent(DIR_OP_CONCURRENCY, &create).await?;
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
        download_small(&sftp, &remote_path, Path::new(&local_path), &on_progress).await?
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

/// 目录传输计划：先完整遍历，拿到全部子目录和每个文件的大小，才有总量和百分比；
/// 遍历结果直接用于传输，不重复列目录。dirs 按层分组（第 0 层是根，每层的父目录都在上一层），
/// files 为 (源, 目标, 字节数)，顺序同逐层遍历。
struct DirPlan<S, D> {
    dirs: Vec<Vec<D>>,
    files: Vec<(S, D, u64)>,
}

impl<S, D> DirPlan<S, D> {
    fn total_bytes(&self) -> u64 {
        self.files.iter().map(|(_, _, size)| size).sum()
    }
}

/// 上传前遍历本地目录。整棵树在一个阻塞任务里用 std::fs 走完（tokio::fs 每条元数据都各起一次
/// spawn_blocking）；目录判断用 DirEntry 的类型（不跟随符号链接，多数文件系统不用额外 stat），
/// 只对文件取大小，跟随符号链接取目标的，与实际传输的字节数一致。
async fn scan_local(local: PathBuf, remote: String) -> AppResult<DirPlan<PathBuf, String>> {
    tokio::task::spawn_blocking(move || {
        let mut plan = DirPlan { dirs: vec![vec![remote.clone()]], files: Vec::new() };
        let mut level = vec![(local, remote)];
        while !level.is_empty() {
            let mut next = Vec::new();
            for (dir_local, dir_remote) in &level {
                for entry in std::fs::read_dir(dir_local)? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().to_string();
                    let (child_local, child_remote) = (entry.path(), join_remote(dir_remote, &name));
                    if entry.file_type()?.is_dir() {
                        next.push((child_local, child_remote));
                    } else {
                        let size = std::fs::metadata(&child_local).map(|m| m.len()).unwrap_or(0);
                        plan.files.push((child_local, child_remote, size));
                    }
                }
            }
            if !next.is_empty() {
                plan.dirs.push(next.iter().map(|(_, remote)| remote.clone()).collect());
            }
            level = next;
        }
        Ok(plan)
    })
    .await
    .map_err(|e| AppError::Other(format!("scan local directory: {e}")))?
}

/// 列一个远端目录：(名称, 是否目录, 字节数)，供 scan_remote 用。
async fn list_remote_dir(sftp: &SftpSession, dir: String) -> AppResult<Vec<(String, bool, u64)>> {
    let entries = sftp.read_dir(&dir).await
        .map_err(|e| AppError::Sftp(format!("read_dir {dir}: {e}")))?;
    Ok(entries.into_iter().map(|e| {
        let meta = e.metadata();
        (e.file_name(), meta.is_dir(), meta.size.unwrap_or(0))
    }).collect())
}

/// 下载前遍历远端目录。`list` 列出一个远端目录的 (名称, 是否目录, 字节数)。
/// 逐层遍历，同层各目录并发列（每次至少三个 RTT），结果仍按列表顺序处理，与逐个遍历的顺序一致。
async fn scan_remote<L, Fut>(remote: String, local: PathBuf, list: L) -> AppResult<DirPlan<String, PathBuf>>
where
    L: Fn(String) -> Fut,
    Fut: Future<Output = AppResult<Vec<(String, bool, u64)>>>,
{
    use futures::TryStreamExt;
    let mut plan = DirPlan { dirs: vec![vec![local.clone()]], files: Vec::new() };
    let mut level = vec![(remote, local)];
    while !level.is_empty() {
        let dirs: Vec<String> = level.iter().map(|(dir_remote, _)| dir_remote.clone()).collect();
        let listings: Vec<_> = stream::iter(dirs)
            .map(&list)
            .buffered(DIR_OP_CONCURRENCY)
            .try_collect()
            .await?;
        let mut next = Vec::new();
        for ((dir_remote, dir_local), entries) in level.iter().zip(listings) {
            for (name, is_dir, size) in entries {
                if name == "." || name == ".." { continue; }
                let (child_remote, child_local) = (join_remote(dir_remote, &name), dir_local.join(&name));
                if is_dir {
                    next.push((child_remote, child_local));
                } else {
                    plan.files.push((child_remote, child_local, size));
                }
            }
        }
        if !next.is_empty() {
            plan.dirs.push(next.iter().map(|(_, local)| local.clone()).collect());
        }
        level = next;
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
    upload_tree_to(&app, &sftp, &session_id, &local_path, &remote_path, &remote_path).await
}

/// 目录传输的进度快照，交给调用方转成上传 / 下载进度事件。
#[derive(Debug, Clone, Copy)]
struct DirCounts {
    transferred: u64,
    total: u64,
    files_done: u64,
    files_total: u64,
}

/// 上传整个本地目录到 `remote_path`；进度事件按 `progress_path` 上报。
async fn upload_tree_to(
    app: &AppHandle,
    sftp: &SftpSession,
    session_id: &str,
    local_path: &str,
    remote_path: &str,
    progress_path: &str,
) -> AppResult<()> {
    upload_tree(sftp, local_path, remote_path, |c| {
        emit_upload_progress(app, UploadProgress {
            session_id: session_id.to_string(),
            remote_path: progress_path.to_string(),
            transferred: c.transferred,
            total: c.total,
            files_done: Some(c.files_done),
            files_total: Some(c.files_total),
        });
    }).await
}

/// 上传整个本地目录到 `remote_path`，进度按 PROGRESS_INTERVAL 节流交给 `emit`。
async fn upload_tree(
    sftp: &SftpSession,
    local_path: &str,
    remote_path: &str,
    emit: impl Fn(DirCounts),
) -> AppResult<()> {
    let plan = scan_local(PathBuf::from(local_path), remote_path.to_string()).await?;
    create_dir_levels(&plan.dirs, |dir: &String| async move {
        // mkdir -p 语义：已存在的目录不算错。其他失败（如 Permission denied）要报出来，
        // 否则后面开文件只会报 No such file，前端认不出权限问题，不会提示改用 sudo。
        let Err(e) = sftp.create_dir(dir).await else { return Ok(()) };
        match sftp.metadata(dir).await {
            Ok(meta) if meta.is_dir() => Ok(()),
            _ => Err(AppError::Sftp(format!("mkdir {dir}: {e}"))),
        }
    }).await?;
    let (total, files_total) = (plan.total_bytes(), plan.files.len() as u64);
    let progress = DirProgress::new(PROGRESS_INTERVAL, |transferred, files_done| {
        emit(DirCounts { transferred, total, files_done, files_total });
    });
    let (sftp_ref, progress_ref) = (&sftp, &progress);
    // 一个文件的在途写入至多 russh-sftp 的 8 路 pipeline × 块 ≈ IN_FLIGHT_BYTES，按文件大小计（大文件独占）
    let files = plan.files.into_iter().map(|(local, remote, size)| ((local, remote), size)).collect();
    transfer_with_budget(files, DIR_FILE_CONCURRENCY, IN_FLIGHT_BYTES, |(local, remote): (PathBuf, String)| async move {
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
    download_tree(&sftp, &remote_path, &local_path, |c| {
        emit_download_progress(&app, DownloadProgress {
            session_id: session_id.clone(),
            remote_path: remote_path.clone(),
            transferred: c.transferred,
            total: c.total,
            files_done: Some(c.files_done),
            files_total: Some(c.files_total),
        });
    }).await
}

/// 下载整个远端目录到 `local_path`，进度按 PROGRESS_INTERVAL 节流交给 `emit`。
async fn download_tree(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &str,
    emit: impl Fn(DirCounts),
) -> AppResult<()> {
    let sftp_ref = sftp;
    let plan = scan_remote(remote_path.to_string(), PathBuf::from(local_path), |dir| list_remote_dir(sftp_ref, dir)).await?;
    for dir in plan.dirs.iter().flatten() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let (total, files_total) = (plan.total_bytes(), plan.files.len() as u64);
    let progress = DirProgress::new(PROGRESS_INTERVAL, |transferred, files_done| {
        emit(DirCounts { transferred, total, files_done, files_total });
    });
    let progress_ref = &progress;
    download_dir_files(
        plan.files.into_iter().map(|(remote, local, size)| ((remote, local), size)).collect(),
        |(remote, local): (String, PathBuf)| async move {
            download_small(sftp_ref, &remote, &local, progress_ref.file()).await?;
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
    let cmd = format!("chmod {:o} {}", permissions, shell_quote(&remote_path));
    run_as_file_user(&session, &cmd).await.map(drop)
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
    let cmd = format!(
        "chown {}:{} {}",
        shell_quote(&owner),
        shell_quote(&group),
        shell_quote(&remote_path)
    );
    run_as_file_user(&session, &cmd).await.map(drop)
}

fn shell_quote(s: &str) -> String {
    // Single-quote and escape embedded single quotes.
    let escaped = s.replace('\'', r"'\''");
    format!("'{}'", escaped)
}

/// sudo 上传最后一步，一条 exec：把暂存区里的条目 `name` 以**目标目录属主**的身份解包进 `dest_dir`，
/// 然后无论成败（含 sudo 认证失败）都以登录用户身份删掉暂存区，退出码和 stderr 仍是解包那一步的。
/// root 读暂存区（0700，属于登录用户），`sudo -u #<属主 uid> tar x` 写入：新文件直接归属主、
/// 目录里已有文件的属主不变。只打包条目本身（不带 "."），否则解包会把目标目录权限改成暂存区的 0700。
/// GNU tar 解包默认会改写已存在目录的权限（合并上传时把属主原来的 755 改成 775），加 --no-overwrite-dir；
/// BSD tar 本就不改、也不认这个参数，所以只对 GNU 加。
/// stat 先试 GNU（-c），再回落 BSD（-f）。LC_ALL=C 让 sudo 的报错保持英文，便于识别。
fn sudo_extract_command(with_password: bool, staging: &str, dest_dir: &str, name: &str) -> String {
    let auth = if with_password { "-S -p \"\"" } else { "-n" };
    // 以 root 跑：$1 暂存区、$2 目标目录、$3 条目名
    let extract = "set -e; uid=$(stat -c %u -- \"$2\" 2>/dev/null || stat -f %u -- \"$2\"); \
                   keep=; tar --version 2>/dev/null | grep -q GNU && keep=--no-overwrite-dir; \
                   tar -C \"$1\" -cf - -- \"$3\" | sudo -u \"#$uid\" tar -C \"$2\" $keep -xf -";
    // 以登录用户跑，$4 是上面的脚本
    let outer = format!(
        "LC_ALL=C sudo {auth} sh -c \"$4\" sshl-sudo \"$1\" \"$2\" \"$3\"; st=$?; rm -rf -- \"$1\" 2>/dev/null; exit $st"
    );
    format!(
        "sh -c '{outer}' sshl-upload {} {} {} '{extract}'",
        shell_quote(staging),
        shell_quote(dest_dir),
        shell_quote(name),
    )
}

/// 把 sudo 失败的 stderr 归成前端可识别的标记（前端据此决定是否弹框要密码）。
fn sudo_failure(stderr: &str) -> &'static str {
    let s = stderr.to_ascii_lowercase();
    if s.contains("incorrect password") || s.contains("sorry, try again") {
        "SUDO_PASSWORD_INCORRECT"
    } else if s.contains("a password is required") || s.contains("no password was provided") || s.contains("a terminal is required") {
        "SUDO_PASSWORD_REQUIRED"
    } else if s.contains("not in the sudoers") || s.contains("is not allowed to") {
        "SUDO_NOT_ALLOWED"
    } else if s.contains("must have a tty") {
        "SUDO_TTY_REQUIRED"
    } else {
        "SUDO_FAILED"
    }
}

/// 跑一条命令：可选写入 stdin（写完即 EOF），收集退出码和 stdout/stderr。
async fn exec_capture(
    mut channel: russh::Channel<russh::client::Msg>,
    cmd: &str,
    stdin: Option<&[u8]>,
) -> AppResult<(u32, String, String)> {
    channel.exec(true, cmd.as_bytes()).await
        .map_err(|e| AppError::Ssh(format!("exec: {e}")))?;
    if let Some(input) = stdin {
        channel.data(input).await
            .map_err(|e| AppError::Ssh(format!("exec stdin: {e}")))?;
    }
    channel.eof().await
        .map_err(|e| AppError::Ssh(format!("exec eof: {e}")))?;
    let (mut out, mut err, mut status) = (Vec::new(), Vec::new(), None);
    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::Data { data } => out.extend_from_slice(&data),
            russh::ChannelMsg::ExtendedData { data, .. } => err.extend_from_slice(&data),
            russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
            russh::ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok((
        status.unwrap_or(255),
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    ))
}

async fn open_exec_channel(session: &SshSession) -> AppResult<russh::Channel<russh::client::Msg>> {
    session.handle.channel_open_session().await
        .map_err(|e| AppError::Sftp(format!("open exec channel: {e}")))
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
        create_dir_levels, download_dir_files, download_segmented, follow_symlinks, join_remote, parse_id_map,
        scan_local, scan_remote, segment_ranges, shell_quote, stream_to_local, stream_to_remote,
        pick_sudo_password, saved_password_for, split_remote, sudo_extract_command, sudo_failure, transfer_with_budget, DirProgress,
        DIR_FILE_CONCURRENCY, DIR_OP_CONCURRENCY, DOWNLOAD_SEGMENTS, IN_FLIGHT_BYTES, SEGMENTED_DOWNLOAD_MIN, SFTP_TIMEOUT_SECS, TRANSFER_CHUNK,
    };
    use super::{staging_from_prepare, sudo_prepare_command};

    /// 假 sudo：按真 sudo 的参数形态吃掉 -S（读一行密码，"wrong" 按密码错处理）/-p ''/-n/-u 用户，然后原样执行命令。
    /// 用来在本机跑通 sudo_prepare_command / sudo_extract_command 生成的整条 shell（引号、tar、stat 回落）。
    fn run_with_fake_sudo(cmd: &str, stdin: &[u8]) -> std::process::Output {
        run_with_fakes(cmd, stdin, &[])
    }

    /// 同 run_with_fake_sudo，另把 `fakes`（命令名, sh 脚本体）作为假命令放进 PATH。
    fn run_with_fakes(cmd: &str, stdin: &[u8], fakes: &[(&str, &str)]) -> std::process::Output {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let bin = std::env::temp_dir().join(format!("sshl-fake-sudo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&bin).unwrap();
        let sudo = "while [ $# -gt 0 ]; do case \"$1\" in \
                    -S) read -r _pw; if [ \"$_pw\" = wrong ]; then printf 'Sorry, try again.\\nsudo: 1 incorrect password attempt\\n' >&2; exit 1; fi; shift;; \
                    -p|-u) shift 2;; -n) shift;; *) break;; esac; done\nexec \"$@\"";
        for (name, body) in [("sudo", sudo)].iter().chain(fakes) {
            let file = bin.join(name);
            std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        let mut child = std::process::Command::new("sh").arg("-c").arg(cmd).env("PATH", path)
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        let out = child.wait_with_output().unwrap();
        let _ = std::fs::remove_dir_all(&bin);
        out
    }

    #[test]
    fn sudo_extract_places_file_with_awkward_name_into_target_dir() {
        let root = std::env::temp_dir().join(format!("sshl-sudo-{}", uuid::Uuid::new_v4()));
        let (staging, dest) = (root.join("staging"), root.join("dest dir"));
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(staging.join("it's a file.txt"), b"hello").unwrap();
        let cmd = sudo_extract_command(true, &staging.to_string_lossy(), &dest.to_string_lossy(), "it's a file.txt");
        let out = run_with_fake_sudo(&cmd, b"secret\n");
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read(dest.join("it's a file.txt")).unwrap(), b"hello");
        assert!(!staging.exists(), "解包后同一条命令删掉暂存区");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sudo_extract_removes_staging_even_when_sudo_or_extract_fails() {
        let root = std::env::temp_dir().join(format!("sshl-sudo-{}", uuid::Uuid::new_v4()));
        let (staging, dest) = (root.join("staging"), root.join("dest"));
        std::fs::create_dir_all(&dest).unwrap();
        let run = |dest: &std::path::Path, stdin: &[u8]| {
            std::fs::create_dir_all(&staging).unwrap();
            std::fs::write(staging.join("f.txt"), b"x").unwrap();
            let cmd = sudo_extract_command(true, &staging.to_string_lossy(), &dest.to_string_lossy(), "f.txt");
            let out = run_with_fake_sudo(&cmd, stdin);
            (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
        };
        // sudo 认证失败：解包没跑，暂存区照样删，报错仍能认出是密码错
        let (code, err) = run(&dest, b"wrong\n");
        assert_ne!(code, Some(0));
        assert_eq!(sudo_failure(&err), "SUDO_PASSWORD_INCORRECT", "{err}");
        assert!(!staging.exists(), "sudo 失败也要删暂存区");
        assert!(!dest.join("f.txt").exists());
        // sudo 过了、解包失败（目标目录不存在）：暂存区照样删，退出码仍是失败
        let (code, err) = run(&root.join("missing"), b"secret\n");
        assert_ne!(code, Some(0), "{err}");
        assert_eq!(sudo_failure(&err), "SUDO_FAILED", "{err}");
        assert!(!staging.exists(), "解包失败也要删暂存区");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn prepare_result(out: &std::process::Output) -> crate::error::AppResult<String> {
        let status = out.status.code().map_or(255, |c| c as u32);
        staging_from_prepare(status, &String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr))
    }

    #[test]
    fn sudo_prepare_checks_sudo_then_creates_private_staging_in_var_tmp() {
        use std::os::unix::fs::PermissionsExt;
        let out = run_with_fake_sudo(&sudo_prepare_command(true), b"secret\n");
        let staging = prepare_result(&out).unwrap();
        assert!(staging.starts_with("/var/tmp/.sshl-upload."), "{staging}");
        let mode = std::fs::metadata(&staging).unwrap().permissions().mode();
        std::fs::remove_dir(&staging).unwrap();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn sudo_prepare_tells_sudo_failure_from_staging_failure() {
        let ran = std::env::temp_dir().join(format!("sshl-mktemp-ran-{}", uuid::Uuid::new_v4()));
        let fake_mktemp = format!("touch {}; echo /var/tmp/.sshl-upload.fake", shell_quote(&ran.to_string_lossy()));
        // sudo 没过：不建暂存区，报 sudo 的错
        let out = run_with_fakes(&sudo_prepare_command(true), b"wrong\n", &[("mktemp", &fake_mktemp)]);
        let err = prepare_result(&out).unwrap_err().to_string();
        assert!(err.starts_with("SUDO_PASSWORD_INCORRECT: "), "{err}");
        assert!(!ran.exists(), "sudo 没过就不该建暂存区");
        let out = run_with_fakes(&sudo_prepare_command(false), b"", &[("mktemp", &fake_mktemp)]);
        assert_eq!(prepare_result(&out).unwrap(), "/var/tmp/.sshl-upload.fake");
        assert!(ran.exists());
        let _ = std::fs::remove_file(&ran);
        // sudo 过了、/var/tmp 和 /tmp 都建不了：报建暂存区失败，不带 SUDO_ 标记（前端不会当成要密码）
        let out = run_with_fakes(&sudo_prepare_command(true), b"secret\n", &[("mktemp", "echo 'mktemp: boom' >&2; exit 1")]);
        let err = prepare_result(&out).unwrap_err().to_string();
        assert!(err.starts_with("创建暂存目录失败: ") && err.contains("mktemp: boom"), "{err}");
        assert!(!err.contains("SUDO_"), "{err}");
    }

    #[test]
    fn sudo_extract_merges_directory_tree_without_touching_target_dir_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("sshl-sudo-{}", uuid::Uuid::new_v4()));
        let (staging, dest) = (root.join("staging"), root.join("dest"));
        std::fs::create_dir_all(staging.join("my dir/sub")).unwrap();
        std::fs::write(staging.join("my dir/sub/a.txt"), b"A").unwrap();
        std::fs::create_dir_all(dest.join("my dir")).unwrap();
        std::fs::write(dest.join("my dir/keep.txt"), b"K").unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
        // 真实暂存区由 mktemp -d 建，是 0700。
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cmd = sudo_extract_command(false, &staging.to_string_lossy(), &dest.to_string_lossy(), "my dir");
        let out = run_with_fake_sudo(&cmd, b"");
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read(dest.join("my dir/sub/a.txt")).unwrap(), b"A");
        // 合并进已有目录：原有文件保留；只打包条目本身，不带 "."，目标目录的权限不会被改成暂存区的 0700。
        assert_eq!(std::fs::read(dest.join("my dir/keep.txt")).unwrap(), b"K");
        assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(!staging.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sudo_extract_uses_non_interactive_sudo_without_password() {
        for cmd in [sudo_extract_command(false, "/tmp/s", "/home/bob", "f"), sudo_prepare_command(false)] {
            assert!(cmd.contains("sudo -n "), "{cmd}");
            assert!(!cmd.contains("-S"), "{cmd}");
        }
        // 密码经 stdin 只给一次 sudo -S（root 里那层 sudo -u 不要密码）
        for cmd in [sudo_extract_command(true, "/tmp/s", "/home/bob", "f"), sudo_prepare_command(true)] {
            assert_eq!(cmd.matches("sudo -S -p \"\" ").count(), 1, "{cmd}");
        }
    }

    /// 真实 Linux（GNU tar/stat + 真 sudo）上的 sudo 上传：
    /// `docker build -t sshl-sudo-test -f tests/fixtures-sudo.Dockerfile tests && docker run -d --rm -p 127.0.0.1:2223:22 sshl-sudo-test` 后
    /// `SSHL_SUDO_IT=127.0.0.1:2223 cargo test --lib sudo_upload_against_real_linux -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_SUDO_IT=host:port 指向 sudo 测试容器"]
    async fn sudo_upload_against_real_linux() {
        use super::exec_capture;
        let Ok(target) = std::env::var("SSHL_SUDO_IT") else { return };
        let (host, port) = target.split_once(':').unwrap();
        let port: u16 = port.parse().unwrap();
        type H = std::sync::Arc<russh::client::Handle<crate::ssh::ClientHandler>>;
        async fn exec(h: &H, cmd: &str, stdin: Option<&[u8]>) -> (u32, String, String) {
            let ch = h.channel_open_session().await.unwrap();
            exec_capture(ch, cmd, stdin).await.unwrap()
        }
        async fn fill(h: &H, staging: &str) {
            let (st, _, err) = exec(h, &format!(
                "cd {0} && printf hi > \"it's a.txt\" && mkdir -p existing/sub && printf new > existing/sub/new.txt",
                shell_quote(staging)), None).await;
            assert_eq!(st, 0, "{err}");
        }
        // 第一步（同 file_upload_sudo）：sudo 预检 + 以登录用户建 0700 暂存区，再放进要上传的东西
        async fn stage(h: &H, user: &str, stdin: Option<&[u8]>) -> String {
            let (st, out, err) = exec(h, &sudo_prepare_command(stdin.is_some()), stdin).await;
            let s = staging_from_prepare(st, &out, &err).unwrap();
            assert!(s.starts_with("/var/tmp/.sshl-upload."), "/var/tmp 优先：{s}");
            let (_, owner, _) = exec(h, &format!("stat -c '%U %a' {}", shell_quote(&s)), None).await;
            assert_eq!(owner.trim(), format!("{user} 700"));
            fill(h, &s).await;
            s
        }
        async fn gone(h: &H, path: &str) -> bool {
            exec(h, &format!("test -e {}", shell_quote(path)), None).await.0 != 0
        }
        let dest = "/home/bob/uploads";
        let alice = crate::ssh::do_handshake(host, port, "alice", Some("alicepw"), None, None).await.unwrap();
        assert_ne!(exec(&alice, "touch /home/bob/uploads/x", None).await.0, 0, "前提：alice 不能直接写 bob 的目录");

        // 上传前的 sudo 预检：密码错 / 没给密码在传文件之前就能识别，且不建暂存区
        let (st, out, err) = exec(&alice, &sudo_prepare_command(true), Some(b"wrong\n")).await;
        assert_eq!(sudo_failure(&err), "SUDO_PASSWORD_INCORRECT", "{err}");
        assert!(staging_from_prepare(st, &out, &err).unwrap_err().to_string().starts_with("SUDO_PASSWORD_INCORRECT: "));
        assert_eq!(out, "", "sudo 没过不建暂存区");
        let (st, out, err) = exec(&alice, &sudo_prepare_command(false), None).await;
        assert!(staging_from_prepare(st, &out, &err).unwrap_err().to_string().starts_with("SUDO_PASSWORD_REQUIRED: "), "{err}");

        // 解包和删暂存区是同一条命令：sudo 失败、解包失败、成功，暂存区都会被删
        let staging = stage(&alice, "alice", Some(b"alicepw\n")).await;
        let (st, _, err) = exec(&alice, &sudo_extract_command(true, &staging, dest, "it's a.txt"), Some(b"wrong\n")).await;
        assert_ne!(st, 0);
        assert_eq!(sudo_failure(&err), "SUDO_PASSWORD_INCORRECT", "{err}");
        assert!(gone(&alice, &staging).await, "sudo 失败也删暂存区");
        let staging = stage(&alice, "alice", Some(b"alicepw\n")).await;
        let (st, _, err) = exec(&alice, &sudo_extract_command(false, &staging, dest, "it's a.txt"), None).await;
        assert_ne!(st, 0);
        assert_eq!(sudo_failure(&err), "SUDO_PASSWORD_REQUIRED", "{err}");
        assert!(gone(&alice, &staging).await);
        let staging = stage(&alice, "alice", Some(b"alicepw\n")).await;
        let (st, _, err) = exec(&alice, &sudo_extract_command(true, &staging, "/home/bob/missing", "it's a.txt"), Some(b"alicepw\n")).await;
        assert_ne!(st, 0, "解包失败要报出来");
        assert_eq!(sudo_failure(&err), "SUDO_FAILED", "{err}");
        assert!(gone(&alice, &staging).await, "解包失败也删暂存区");
        for name in ["it's a.txt", "existing"] {
            let staging = stage(&alice, "alice", Some(b"alicepw\n")).await;
            let (st, _, err) = exec(&alice, &sudo_extract_command(true, &staging, dest, name), Some(b"alicepw\n")).await;
            assert_eq!(st, 0, "{name}: {err}");
            assert!(gone(&alice, &staging).await);
        }
        let (_, out, _) = exec(&alice, "cd /home/bob/uploads && stat -c '%U %a %n' . existing existing/keep.txt existing/sub existing/sub/new.txt \"it's a.txt\" && cat \"it's a.txt\" existing/sub/new.txt", None).await;
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "bob 755 .", "目标目录权限不能变：{out}");
        assert_eq!(lines[1], "bob 755 existing", "合并进已有目录时不能改它的权限：{out}");
        assert!(lines[2].starts_with("bob ") && lines[2].ends_with(" existing/keep.txt"), "{out}");
        assert!(lines[3].starts_with("bob ") && lines[4].starts_with("bob ") && lines[5].starts_with("bob "), "新文件归目录属主：{out}");
        assert_eq!(lines[6], "hinew", "{out}");

        let carol = crate::ssh::do_handshake(host, port, "carol", Some("carolpw"), None, None).await.unwrap();
        let (st, out, err) = exec(&carol, &sudo_prepare_command(true), Some(b"carolpw\n")).await;
        assert!(staging_from_prepare(st, &out, &err).unwrap_err().to_string().starts_with("SUDO_NOT_ALLOWED: "), "{err}");
        let (_, staging, _) = exec(&carol, "mktemp -d /var/tmp/.sshl-upload.XXXXXXXX", None).await;
        let staging = staging.trim().to_string();
        fill(&carol, &staging).await;
        let (_, _, err) = exec(&carol, &sudo_extract_command(true, &staging, dest, "it's a.txt"), Some(b"carolpw\n")).await;
        assert_eq!(sudo_failure(&err), "SUDO_NOT_ALLOWED", "{err}");
        assert!(gone(&carol, &staging).await);

        let dave = crate::ssh::do_handshake(host, port, "dave", Some("davepw"), None, None).await.unwrap();
        let staging = stage(&dave, "dave", None).await;
        let (st, _, err) = exec(&dave, &sudo_extract_command(false, &staging, dest, "existing"), None).await;
        assert_eq!(st, 0, "免密 sudo 走 -n：{err}");
        assert!(gone(&dave, &staging).await);
    }

    /// 真实 Linux 上「文件管理跟随终端用户」：终端（带 pty 的交互 shell，同应用）里 su / sudo -i 之后，
    /// 能识别出当前用户和切换方式，并以他的身份起 SFTP、执行命令。容器同 sudo_upload_against_real_linux，
    /// `SSHL_SUDO_IT=127.0.0.1:2223 cargo test --lib follows_terminal_user_against_real_linux -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_SUDO_IT=host:port 指向 sudo 测试容器"]
    async fn follows_terminal_user_against_real_linux() {
        use super::{exec_capture, open_sftp_as};
        use crate::terminal_user::{parse_probe, run_as_command, terminal_user, Via, PROBE_COMMAND};
        use tokio::io::AsyncWriteExt;
        let Ok(target) = std::env::var("SSHL_SUDO_IT") else { return };
        let (host, port) = target.split_once(':').unwrap();
        let port: u16 = port.parse().unwrap();
        type H = std::sync::Arc<russh::client::Handle<crate::ssh::ClientHandler>>;
        type Ch = russh::Channel<russh::client::Msg>;
        async fn channel(h: &H) -> Ch {
            h.channel_open_session().await.unwrap()
        }
        async fn exec(h: &H, cmd: &str, stdin: Option<&[u8]>) -> (u32, String, String) {
            exec_capture(channel(h).await, cmd, stdin).await.unwrap()
        }
        async fn identify(h: &H) -> Option<(u32, Option<Via>)> {
            let (_, out, _) = exec(h, PROBE_COMMAND, None).await;
            parse_probe(&out).and_then(|(sshd, procs)| terminal_user(sshd, &procs))
        }
        // 往终端里敲一行，等输出里出现 `until`
        async fn type_line(term: &mut Ch, line: &str, until: &str) {
            term.data(format!("{line}\n").as_bytes()).await.unwrap();
            let mut seen = String::new();
            let wait = async {
                while let Some(msg) = term.wait().await {
                    if let russh::ChannelMsg::Data { data } = msg {
                        seen.push_str(&String::from_utf8_lossy(&data));
                        if seen.contains(until) {
                            return;
                        }
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(10), wait).await
                .unwrap_or_else(|_| panic!("终端里等不到 {until:?}，已输出：{seen}"));
        }
        async fn sftp_err(h: &H, user: &str, via: Via, pw: Option<&str>) -> String {
            open_sftp_as(channel(h).await, user, via, pw).await.err().expect("应当失败").to_string()
        }

        let alice = crate::ssh::do_handshake(host, port, "alice", Some("alicepw"), None, None).await.unwrap();
        let uid_of = |name: &'static str| { let alice = alice.clone(); async move { exec(&alice, &format!("id -u {name}"), None).await.1.trim().parse::<u32>().unwrap() } };
        let (alice_uid, bob_uid) = (uid_of("alice").await, uid_of("bob").await);
        let mut term = channel(&alice).await;
        term.request_pty(false, "xterm-256color", 120, 40, 0, 0, &[]).await.unwrap();
        term.request_shell(false).await.unwrap();
        type_line(&mut term, "echo ready", "ready").await;
        assert_eq!(identify(&alice).await, Some((alice_uid, None)), "没切换：就是登录用户");

        // su - root
        type_line(&mut term, "su - root", "Password:").await;
        type_line(&mut term, "rootpw", "# ").await;
        assert_eq!(identify(&alice).await, Some((0, Some(Via::Su))));
        type_line(&mut term, "sleep 30 &", "# ").await;
        type_line(&mut term, "vim -u NONE /tmp/x 2>/dev/null || sleep 3", "").await;
        assert_eq!(identify(&alice).await, Some((0, Some(Via::Su))), "root shell 里开着程序 / 后台任务也还是 root");
        term.data(&b"\x1b:q!\n\x03"[..]).await.unwrap();

        // 以 root 起 SFTP：密码错 / 没给密码在协议开始前识别，且没给密码时根本不跑 su
        assert!(sftp_err(&alice, "root", Via::Su, Some("wrong")).await.contains("SUDO_PASSWORD_INCORRECT"));
        assert!(sftp_err(&alice, "root", Via::Su, None).await.contains("SUDO_PASSWORD_REQUIRED"));
        let sftp = open_sftp_as(channel(&alice).await, "root", Via::Su, Some("rootpw")).await.unwrap();
        assert_eq!(sftp.canonicalize(".").await.unwrap(), "/root", "主目录跟着变成 root 的");
        sftp.read_dir("/root").await.expect("能看到 root 的东西");
        let mut f = sftp.create("/home/bob/uploads/by-root.txt").await.expect("root 能写进 bob 的目录");
        f.write_all(b"root").await.unwrap();
        f.shutdown().await.unwrap();
        let (_, out, _) = exec(&alice, "stat -c %U /home/bob/uploads/by-root.txt", None).await;
        assert_eq!(out.trim(), "root", "以 root 上传，文件归 root");
        let (st, _, err) = exec(&alice, &run_as_command("root", Via::Su, true, "rm -f -- '/home/bob/uploads/by-root.txt'"), Some(b"rootpw\n")).await;
        assert_eq!(st, 0, "以 root 删除：{err}");
        assert_ne!(exec(&alice, "test -e /home/bob/uploads/by-root.txt", None).await.0, 0);

        // 回到 alice，再 sudo -i（要 alice 的密码）
        type_line(&mut term, "exit", "$ ").await;
        assert_eq!(identify(&alice).await, Some((alice_uid, None)));
        type_line(&mut term, "sudo -i", "password for alice").await;
        type_line(&mut term, "alicepw", "# ").await;
        assert_eq!(identify(&alice).await, Some((0, Some(Via::Sudo))));
        let err = sftp_err(&alice, "root", Via::Sudo, Some("wrong")).await;
        assert!(err.contains("SUDO_PASSWORD_INCORRECT"), "{err}");
        let err = sftp_err(&alice, "root", Via::Sudo, None).await;
        assert!(err.contains("SUDO_PASSWORD_REQUIRED"), "{err}");
        let sftp = open_sftp_as(channel(&alice).await, "root", Via::Sudo, Some("alicepw")).await.unwrap();
        assert_eq!(sftp.canonicalize(".").await.unwrap(), "/root");

        // su 成普通用户 bob：以 bob 身份，主目录是 bob 的
        type_line(&mut term, "exit", "$ ").await;
        type_line(&mut term, "su - bob", "Password:").await;
        type_line(&mut term, "bobpw", "$ ").await;
        assert_eq!(identify(&alice).await, Some((bob_uid, Some(Via::Su))));
        let sftp = open_sftp_as(channel(&alice).await, "bob", Via::Su, Some("bobpw")).await.unwrap();
        assert_eq!(sftp.canonicalize(".").await.unwrap(), "/home/bob");
    }

    #[test]
    fn splits_remote_path_into_parent_and_name() {
        assert_eq!(split_remote("/home/bob/a.txt"), Some(("/home/bob".into(), "a.txt".into())));
        assert_eq!(split_remote("/home/bob/dir/"), Some(("/home/bob".into(), "dir".into())));
        assert_eq!(split_remote("/x"), Some(("/".into(), "x".into())));
        assert_eq!(split_remote("/"), None);
        assert_eq!(split_remote("relative"), None);
    }

    #[test]
    fn uses_only_the_needed_users_own_saved_password() {
        let saved = |entries: &'static [&'static str]| move |_cid: &str, kind: &str| entries.contains(&kind);
        // 登录用户 alice 自己的：连接密码，没有时找以 alice 保存的账号密码；别人的（root）不拿来试
        assert_eq!(saved_password_for(Some("c1"), "alice", "alice", saved(&["password", "acct:alice", "acct:root"])).as_deref(), Some("password"));
        assert_eq!(saved_password_for(Some("c1"), "alice", "alice", saved(&["acct:alice", "acct:root"])).as_deref(), Some("acct:alice"));
        assert_eq!(saved_password_for(Some("c1"), "alice", "alice", saved(&["acct:root", "acct:bob"])), None);
        // 终端 su 成了 root：要的是 root 的密码，连接密码（alice 的）不算
        assert_eq!(saved_password_for(Some("c1"), "alice", "root", saved(&["password", "acct:root"])).as_deref(), Some("acct:root"));
        assert_eq!(saved_password_for(Some("c1"), "alice", "root", saved(&["password", "acct:alice"])), None);
        assert_eq!(saved_password_for(None, "alice", "alice", saved(&["password"])), None);
    }

    #[test]
    fn picks_sudo_password_by_precedence() {
        let saved = |cid: &str, kind: &str| match (cid, kind) {
            ("c1", "password") => Some("login-pw".to_string()),
            ("c1", "acct:root") => Some("root-pw".to_string()),
            _ => None,
        };
        // 点选的已保存密码优先于输入框（选了就以它为准）
        assert_eq!(pick_sudo_password(Some("c1"), Some("acct:root"), Some("typed".into()), saved).unwrap().as_deref(), Some("root-pw"));
        assert_eq!(pick_sudo_password(Some("c1"), None, Some("typed".into()), saved).unwrap().as_deref(), Some("typed"));
        // 什么都没给（或空串）→ None（走 sudo -n），不自己去翻已保存的密码
        assert_eq!(pick_sudo_password(Some("c1"), None, Some(String::new()), saved).unwrap(), None);
        assert_eq!(pick_sudo_password(Some("c1"), None, None, saved).unwrap(), None);
        // 点选的密码已被删 / 连接没保存：报错而不是悄悄换成别的密码
        assert!(pick_sudo_password(Some("c1"), Some("acct:gone"), None, saved).is_err());
        assert!(pick_sudo_password(None, Some("acct:root"), None, saved).is_err());
    }

    #[test]
    fn classifies_sudo_failures() {
        assert_eq!(sudo_failure("sudo: a password is required"), "SUDO_PASSWORD_REQUIRED");
        assert_eq!(sudo_failure("sudo: no password was provided"), "SUDO_PASSWORD_REQUIRED");
        assert_eq!(sudo_failure("Sorry, try again.\nsudo: 1 incorrect password attempt"), "SUDO_PASSWORD_INCORRECT");
        assert_eq!(sudo_failure("alice is not in the sudoers file."), "SUDO_NOT_ALLOWED");
        assert_eq!(sudo_failure("sudo: sorry, you must have a tty to run sudo"), "SUDO_TTY_REQUIRED");
        assert_eq!(sudo_failure("tar: x: Cannot open: Permission denied"), "SUDO_FAILED");
    }
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

    struct SegmentedRun {
        result: AppResult<u64>,
        bytes: Vec<u8>,
        /// READ 在途峰值。
        peak: usize,
        last_progress: u64,
        /// 远端句柄总数（调用方预先开好的段 0 句柄 + open_remote 新开的）。
        opened: usize,
        /// 其中经 open_remote 新开的。
        reopened: usize,
        released: usize,
    }

    /// 用 DOWNLOAD_SEGMENTS 跑一次分段下载：远端内容是 `remote`，按 `total` 切段。
    async fn run_segmented(remote: Vec<u8>, total: u64) -> SegmentedRun {
        let data = Arc::new(remote);
        let local = Arc::new(Mutex::new(Vec::new()));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let last_progress = AtomicU64::new(0);
        let (reopened, released) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let new_remote = || MockRemote {
            data: data.clone(),
            pos: 0,
            waiting: false,
            in_flight: in_flight.clone(),
            peak: peak.clone(),
        };
        let result = download_segmented(
            total,
            DOWNLOAD_SEGMENTS,
            new_remote(),
            || {
                reopened.fetch_add(1, Ordering::SeqCst);
                let remote = new_remote();
                async move { Ok(remote) }
            },
            || {
                let file = MockLocal { buf: local.clone(), pos: 0 };
                async move { Ok(file) }
            },
            |_: MockRemote| { released.fetch_add(1, Ordering::SeqCst); },
            "/r",
            |t| last_progress.store(t, Ordering::SeqCst),
        ).await;
        let bytes = local.lock().unwrap().clone();
        SegmentedRun {
            result,
            bytes,
            peak: peak.load(Ordering::SeqCst),
            last_progress: last_progress.load(Ordering::SeqCst),
            opened: 1 + reopened.load(Ordering::SeqCst),
            reopened: reopened.into_inner(),
            released: released.into_inner(),
        }
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
        let run = run_segmented(source.clone(), total).await;
        assert_eq!(run.result.unwrap(), total);
        // 各段 READ 必须同时在途，串行时吞吐仍是单句柄的块大小 / RTT。
        assert!(run.peak > 1, "segments were read serially (peak in-flight = {})", run.peak);
        assert!(run.bytes == source, "reassembled bytes differ from the remote file");
        assert_eq!(run.last_progress, total);
    }

    #[tokio::test]
    async fn segmented_download_releases_every_remote_handle() {
        let source = pattern(5 * TRANSFER_CHUNK);
        let run = run_segmented(source.clone(), source.len() as u64).await;
        run.result.unwrap();
        // 只 drop 不关，russh-sftp 的已开句柄计数不会减，累计到服务端上限（1019）后 open 全部失败。
        assert!(run.opened > 1);
        assert_eq!(run.released, run.opened, "every segment handle must be closed");
    }

    #[tokio::test]
    async fn segmented_download_reuses_the_probe_handle_for_segment_zero() {
        let source = pattern(5 * TRANSFER_CHUNK);
        let segments = segment_ranges(source.len() as u64, DOWNLOAD_SEGMENTS).len();
        let run = run_segmented(source.clone(), source.len() as u64).await;
        run.result.unwrap();
        assert!(run.bytes == source);
        // 探测可读用的句柄就是段 0：不再先 open 一次、关掉、再给每段各 open 一次
        assert_eq!(run.reopened, segments - 1);
        assert_eq!(run.released, segments);
    }

    #[tokio::test]
    async fn segmented_download_keeps_a_window_of_reads_in_flight() {
        let source = pattern(16 * TRANSFER_CHUNK);
        let run = run_segmented(source.clone(), source.len() as u64).await;
        run.result.unwrap();
        // 上传有 russh-sftp 的 8 路写 pipeline（8 × 块 ≈ 2MB 通道窗口），下载靠分段补齐同样的在途量：
        // 50ms RTT 下 4 段只有 ~20MB/s。
        let upload_pipeline = russh_sftp::client::Config::default().max_concurrent_writes;
        assert_eq!(run.peak, upload_pipeline);
        assert_eq!(run.peak as u64 * TRANSFER_CHUNK as u64, IN_FLIGHT_BYTES);
    }

    #[tokio::test]
    async fn segmented_download_errors_when_file_shrinks_mid_transfer() {
        // stat 时 4 块多，读的时候只剩 1 块：中间段会提前 EOF。
        let total = 4 * TRANSFER_CHUNK as u64 + 10;
        let run = run_segmented(pattern(TRANSFER_CHUNK), total).await;
        assert!(matches!(run.result, Err(AppError::Sftp(_))), "shrunk file must not download silently");
    }

    #[tokio::test]
    async fn directory_download_small_files_share_the_in_flight_budget() {
        // 单句柄下载每个文件在途至多一个块：小于一块的文件按自身大小计，接近一块的按一块计。
        let peak_for = |size: u64| async move {
            let probe = Concurrency::default();
            let files: Vec<(usize, u64)> = (0..40).map(|i| (i, size)).collect();
            download_dir_files(files, |_| async { probe.hold().await; Ok(()) }, |_, _| async { Ok(()) }).await.unwrap();
            probe.peak()
        };
        assert_eq!(peak_for(1000).await, DIR_FILE_CONCURRENCY);
        assert_eq!(peak_for(SEGMENTED_DOWNLOAD_MIN - 1).await as u64, IN_FLIGHT_BYTES / TRANSFER_CHUNK as u64);
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
        assert_eq!(plan.dirs, [vec!["/r"], vec!["/r/sub"], vec!["/r/sub/deeper"]]);
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
    async fn scan_local_does_not_descend_into_symlinked_dirs_but_sizes_link_targets() {
        let root = std::env::temp_dir().join(format!("sshl-scan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("real/inner")).unwrap();
        std::fs::write(root.join("real/data.bin"), b"123456").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link-dir")).unwrap();
        std::os::unix::fs::symlink(root.join("real/data.bin"), root.join("link-file")).unwrap();

        let plan = scan_local(root.clone(), "/r".to_string()).await;
        let dir_size = std::fs::metadata(root.join("real")).unwrap().len();
        std::fs::remove_dir_all(&root).unwrap();
        let plan = plan.unwrap();

        // 目录判断不跟随符号链接：指向目录的链接不展开、按文件列出；文件大小跟随链接取目标的。
        assert_eq!(plan.dirs, [vec!["/r"], vec!["/r/real"], vec!["/r/real/inner"]]);
        let mut files: Vec<(&str, u64)> = plan.files.iter().map(|(_, r, s)| (r.as_str(), *s)).collect();
        files.sort();
        assert_eq!(files, [("/r/link-dir", dir_size), ("/r/link-file", 6), ("/r/real/data.bin", 6)]);
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
        assert_eq!(plan.dirs, [vec![PathBuf::from("/l")], vec![PathBuf::from("/l/sub")], vec![PathBuf::from("/l/sub/deeper")]]);
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

    /// 并发度探针：进入时计数、让出一轮、退出时减计数，记录峰值。
    #[derive(Default)]
    struct Concurrency {
        now: AtomicUsize,
        peak: AtomicUsize,
    }

    impl Concurrency {
        async fn hold(&self) {
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            self.now.fetch_sub(1, Ordering::SeqCst);
        }
        fn peak(&self) -> usize {
            self.peak.load(Ordering::SeqCst)
        }
    }

    fn parent_of(path: &str) -> &str {
        &path[..path.rfind('/').unwrap()]
    }

    #[tokio::test]
    async fn scan_remote_lists_each_level_concurrently() {
        // /r 下 40 个子目录，每个子目录下 1 个文件和 1 个孙目录。
        let mut tree: HashMap<String, Vec<(String, bool, u64)>> = HashMap::new();
        tree.insert("/r".into(), (0..40).map(|i| (format!("d{i:02}"), true, 0)).collect());
        for i in 0..40 {
            tree.insert(format!("/r/d{i:02}"), vec![(format!("f{i}"), false, i), ("g".into(), true, 0)]);
            tree.insert(format!("/r/d{i:02}/g"), vec![]);
        }
        let probe = Concurrency::default();
        let listed = Mutex::new(Vec::<String>::new());
        let (tree, probe_ref, listed_ref) = (&tree, &probe, &listed);
        let plan = scan_remote("/r".to_string(), PathBuf::from("/l"), |dir: String| async move {
            // 列子目录时父目录一定已经列完（子目录名就是从父目录的列表里来的）。
            if dir != "/r" {
                assert!(listed_ref.lock().unwrap().iter().any(|d| d == parent_of(&dir)), "{dir} listed before its parent");
            }
            probe_ref.hold().await;
            listed_ref.lock().unwrap().push(dir.clone());
            Ok(tree[&dir].clone())
        }).await.unwrap();

        // 每次列目录至少 opendir / readdir / close 三个 RTT：同层必须并发，但不超过上限。
        assert!(probe.peak() > 1, "directories were listed serially");
        assert!(probe.peak() <= DIR_OP_CONCURRENCY, "peak {} > {DIR_OP_CONCURRENCY}", probe.peak());
        // 结果与逐个 BFS 的顺序一致：按层分组，层内保持列表顺序。
        let level = |f: &dyn Fn(usize) -> String| (0..40).map(|i| PathBuf::from(f(i))).collect::<Vec<_>>();
        assert_eq!(plan.dirs, [
            vec![PathBuf::from("/l")],
            level(&|i| format!("/l/d{i:02}")),
            level(&|i| format!("/l/d{i:02}/g")),
        ]);
        let files: Vec<(String, u64)> = plan.files.iter().map(|(r, _, s)| (r.clone(), *s)).collect();
        assert_eq!(files, (0..40).map(|i| (format!("/r/d{i:02}/f{i}"), i)).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn create_dir_levels_runs_each_level_concurrently_after_its_parents() {
        let levels: Vec<Vec<String>> = vec![
            vec!["/r".into()],
            (0..40).map(|i| format!("/r/d{i}")).collect(),
            (0..40).map(|i| format!("/r/d{i}/sub")).collect(),
        ];
        let probe = Concurrency::default();
        let created = Mutex::new(Vec::<String>::new());
        let (probe_ref, created_ref) = (&probe, &created);
        create_dir_levels(&levels, |dir: &String| async move {
            if dir != "/r" {
                assert!(created_ref.lock().unwrap().iter().any(|d| d == parent_of(dir)), "{dir} created before its parent");
            }
            probe_ref.hold().await;
            created_ref.lock().unwrap().push(dir.clone());
            Ok(())
        }).await.unwrap();
        assert_eq!(created.lock().unwrap().len(), 81);
        // 每个 MKDIR 一个 RTT：同层必须并发。
        assert!(probe.peak() > 1, "directories were created serially");
        assert!(probe.peak() <= DIR_OP_CONCURRENCY);
    }

    #[tokio::test]
    async fn create_dir_levels_stops_at_first_failing_level() {
        let levels: Vec<Vec<String>> = vec![vec!["/r".into()], vec!["/r/a".into(), "/r/denied".into()], vec!["/r/a/sub".into()]];
        let tried = Mutex::new(Vec::<String>::new());
        let tried_ref = &tried;
        let result = create_dir_levels(&levels, |dir: &String| async move {
            tried_ref.lock().unwrap().push(dir.clone());
            if dir.ends_with("denied") { Err(AppError::Sftp("mkdir: Permission denied".into())) } else { Ok(()) }
        }).await;
        assert!(result.unwrap_err().to_string().contains("Permission denied"));
        assert!(!tried.lock().unwrap().iter().any(|d| d == "/r/a/sub"), "失败后不应再建下一层");
    }

    /// 跑一批 (编号, 在途字节) 的传输，返回 (同时在传的文件数峰值, 在途字节峰值, 开始顺序)。
    async fn run_budgeted(items: Vec<(usize, u64)>, fail_on: Option<usize>) -> (AppResult<()>, usize, u64, Vec<usize>) {
        let costs: HashMap<usize, u64> = items.iter().copied().collect();
        let probe = Concurrency::default();
        let (bytes_now, bytes_peak) = (AtomicU64::new(0), AtomicU64::new(0));
        let started = Mutex::new(Vec::new());
        let (costs, probe_ref, bytes_now_ref, bytes_peak_ref, started_ref) = (&costs, &probe, &bytes_now, &bytes_peak, &started);
        let result = transfer_with_budget(items, DIR_FILE_CONCURRENCY, IN_FLIGHT_BYTES, |i: usize| async move {
            started_ref.lock().unwrap().push(i);
            let cost = costs[&i].min(IN_FLIGHT_BYTES);
            let now = bytes_now_ref.fetch_add(cost, Ordering::SeqCst) + cost;
            bytes_peak_ref.fetch_max(now, Ordering::SeqCst);
            probe_ref.hold().await;
            bytes_now_ref.fetch_sub(cost, Ordering::SeqCst);
            if Some(i) == fail_on {
                return Err(AppError::Sftp(format!("file {i} failed")));
            }
            Ok(())
        }).await;
        let started = started.into_inner().unwrap();
        (result, probe.peak(), bytes_peak.into_inner(), started)
    }

    #[tokio::test]
    async fn budgeted_transfer_runs_many_tiny_files_at_once() {
        let (result, peak, _, started) = run_budgeted((0..60).map(|i| (i, 100)).collect(), None).await;
        result.unwrap();
        assert_eq!(started.len(), 60);
        // 小文件的耗时几乎全是 open / 读写 / close 的往返（约 3 RTT），并发数直接决定速度：
        // 50ms RTT 下 4 路只有 ~27 个文件 / 秒。
        assert_eq!(peak, DIR_FILE_CONCURRENCY);
        assert!(peak >= 16, "tiny files ran only {peak} at a time");
    }

    #[tokio::test]
    async fn budgeted_transfer_caps_bytes_in_flight() {
        let chunk = TRANSFER_CHUNK as u64;
        let (result, peak, bytes_peak, _) = run_budgeted((0..30).map(|i| (i, chunk)).collect(), None).await;
        result.unwrap();
        assert_eq!(peak as u64, IN_FLIGHT_BYTES / chunk);
        assert!(bytes_peak <= IN_FLIGHT_BYTES);

        // 超过预算的文件独占：不和别的文件同时传，也不会因为超预算卡死。
        let items = vec![(0, 10), (1, 10), (2, 3 * IN_FLIGHT_BYTES), (3, 10), (4, IN_FLIGHT_BYTES), (5, 10)];
        let (result, _, bytes_peak, started) = run_budgeted(items, None).await;
        result.unwrap();
        assert_eq!(bytes_peak, IN_FLIGHT_BYTES);
        // 按原顺序放行，队首放不下就等。
        assert_eq!(started, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn in_flight_budget_fits_ssh_window_and_request_timeout() {
        // 在途再多也只是在通道窗口外排队
        assert!(IN_FLIGHT_BYTES <= russh::client::Config::default().window_size as u64);
        // 排在满额在途数据后面的请求，在 ~35KB/s 的慢链路上也要在超时前等到回复
        assert!(IN_FLIGHT_BYTES / SFTP_TIMEOUT_SECS <= 35 * 1024, "{}B/s", IN_FLIGHT_BYTES / SFTP_TIMEOUT_SECS);
    }

    #[tokio::test]
    async fn budgeted_transfer_propagates_errors() {
        let (result, _, _, _) = run_budgeted((0..20).map(|i| (i, 1)).collect(), Some(7)).await;
        assert!(matches!(result, Err(AppError::Sftp(msg)) if msg.contains("file 7")));
    }

    /// 真实 sshd 上的目录 / 大文件传输。容器同 sudo_upload_against_real_linux（以 alice 登录）：
    /// `SSHL_SFTP_IT=127.0.0.1:2231 cargo test --lib real_sftp -- --ignored --nocapture`
    mod real_sftp {
        use super::super::{
            download_large, download_tree, exec_capture, list_remote_dir, new_sftp, scan_remote,
            stream_to_remote, upload_tree, DirCounts, SEGMENTED_DOWNLOAD_MIN, TRANSFER_CHUNK,
        };
        use russh_sftp::client::SftpSession;
        use std::collections::BTreeMap;
        use std::path::{Path, PathBuf};
        use std::sync::Mutex;
        use std::time::Instant;
        use tokio::io::AsyncWriteExt;

        type H = std::sync::Arc<russh::client::Handle<crate::ssh::ClientHandler>>;

        async fn channel(h: &H) -> russh::Channel<russh::client::Msg> {
            h.channel_open_session().await.unwrap()
        }

        async fn connect() -> Option<(H, SftpSession)> {
            let target = std::env::var("SSHL_SFTP_IT").ok()?;
            let (host, port) = target.split_once(':').unwrap();
            let h = crate::ssh::do_handshake(host, port.parse().unwrap(), "alice", Some("alicepw"), None, None).await.unwrap();
            let ch = channel(&h).await;
            ch.request_subsystem(true, "sftp").await.unwrap();
            let sftp = new_sftp(ch).await.unwrap();
            Some((h, sftp))
        }

        async fn exec(h: &H, cmd: &str) -> String {
            let (status, out, err) = exec_capture(channel(h).await, cmd, None).await.unwrap();
            assert_eq!(status, 0, "`{cmd}`: {err}");
            out
        }

        fn temp_dir(tag: &str) -> PathBuf {
            std::env::temp_dir().join(format!("sshl-{tag}-{}", uuid::Uuid::new_v4()))
        }

        fn bytes(len: usize, seed: usize) -> Vec<u8> {
            (0..len).map(|i| ((i * 31 + seed) % 251) as u8).collect()
        }

        fn write(path: &Path, data: &[u8]) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }

        /// 目录树快照：相对路径 → None（目录）/ Some(内容)。
        fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
            fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
                for entry in std::fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    let rel = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                    if path.is_dir() {
                        out.insert(rel, None);
                        walk(root, &path, out);
                    } else {
                        out.insert(rel, Some(std::fs::read(&path).unwrap()));
                    }
                }
            }
            let mut out = BTreeMap::new();
            walk(root, root, &mut out);
            out
        }

        fn files_equal(a: &Path, b: &Path) -> bool {
            use std::io::Read;
            let (mut fa, mut fb) = (std::fs::File::open(a).unwrap(), std::fs::File::open(b).unwrap());
            let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
            loop {
                let na = fa.read(&mut ba).unwrap();
                let mut nb = 0;
                while nb < na {
                    let n = fb.read(&mut bb[nb..na]).unwrap();
                    if n == 0 { return false; }
                    nb += n;
                }
                if na == 0 { return fb.read(&mut bb).unwrap() == 0; }
                if ba[..na] != bb[..na] { return false; }
            }
        }

        /// 最后一次进度回报必须走到终点（字节和文件数都满）。
        fn assert_finished(last: &Mutex<Option<DirCounts>>) {
            let c = last.lock().unwrap().expect("没有进度回报");
            assert_eq!((c.transferred, c.files_done), (c.total, c.files_total), "{c:?}");
        }

        #[tokio::test(flavor = "multi_thread")]
        #[ignore = "需要 SSHL_SFTP_IT=host:port 指向 sshd 测试容器"]
        async fn directory_round_trip_keeps_tree_and_bytes() {
            let Some((h, sftp)) = connect().await else { return };
            let chunk = TRANSFER_CHUNK;
            let src = temp_dir("rt-src");
            let files: Vec<(&str, usize)> = vec![
                ("zero.txt", 0),
                ("it's ü name.bin", 1000),
                ("l1/chunk-1.bin", chunk - 1),
                ("l1/chunk.bin", chunk),
                ("l1/l2/chunk+1.bin", chunk + 1),
                ("l1/l2/three.bin", 3 * chunk),
                ("l1/l2/l3/seg-min.bin", SEGMENTED_DOWNLOAD_MIN as usize),
                ("l1/l2/l3/big.bin", 9 * chunk + 7),
            ];
            for (i, (name, len)) in files.iter().enumerate() {
                write(&src.join(name), &bytes(*len, i));
            }
            for i in 0..60 {
                write(&src.join(format!("many/f{i:02}.txt")), &bytes(1 + i * 67, i));
            }
            for dir in ["empty", "l1/l2/l3/empty-deep", "many/empty"] {
                std::fs::create_dir_all(src.join(dir)).unwrap();
            }

            let base = exec(&h, "mktemp -d /home/alice/sshl-it.XXXXXXXX").await.trim().to_string();
            let remote = format!("{base}/tree");
            let last = Mutex::new(None);
            upload_tree(&sftp, &src.to_string_lossy(), &remote, |c| *last.lock().unwrap() = Some(c)).await.unwrap();
            assert_finished(&last);

            let dst = temp_dir("rt-dst");
            let last = Mutex::new(None);
            download_tree(&sftp, &remote, &dst.to_string_lossy(), |c| *last.lock().unwrap() = Some(c)).await.unwrap();
            assert_finished(&last);
            let (before, after) = (snapshot(&src), snapshot(&dst));
            assert_eq!(before.keys().collect::<Vec<_>>(), after.keys().collect::<Vec<_>>(), "目录结构（含空目录）要一致");
            for (path, content) in &before {
                assert!(after[path] == *content, "{path} 内容不一致");
            }

            // 单文件分段下载
            let single = dst.join("single.bin");
            let n = download_large(&sftp, &format!("{remote}/l1/l2/l3/big.bin"), &single, 9 * chunk as u64 + 7, |_| {}).await.unwrap();
            assert_eq!(n, 9 * chunk as u64 + 7);
            assert!(files_equal(&single, &src.join("l1/l2/l3/big.bin")));

            exec(&h, &format!("rm -rf -- '{base}'")).await;
            std::fs::remove_dir_all(&src).unwrap();
            std::fs::remove_dir_all(&dst).unwrap();
        }

        /// 目标目录没有写权限：要报出 Permission denied（前端据此提示改用 sudo），不能被吞成后面开文件的 No such file。
        /// 目录已存在（重复上传合并）不算错。
        #[tokio::test(flavor = "multi_thread")]
        #[ignore = "需要 SSHL_SFTP_IT=host:port 指向 sshd 测试容器"]
        async fn upload_tree_reports_permission_denied_but_merges_into_existing_dirs() {
            let Some((h, sftp)) = connect().await else { return };
            let src = temp_dir("perm-src");
            write(&src.join("a/b/f.txt"), b"hello");
            write(&src.join("top.txt"), b"top");

            // 容器里 /home/bob/uploads 属 bob、755，alice 不可写
            let err = upload_tree(&sftp, &src.to_string_lossy(), "/home/bob/uploads/perm-tree", |_| {}).await.unwrap_err();
            assert!(err.to_string().to_lowercase().contains("permission denied"), "应报权限错误，实际: {err}");

            let base = exec(&h, "mktemp -d /home/alice/sshl-it.XXXXXXXX").await.trim().to_string();
            let remote = format!("{base}/tree");
            upload_tree(&sftp, &src.to_string_lossy(), &remote, |_| {}).await.unwrap();
            upload_tree(&sftp, &src.to_string_lossy(), &remote, |_| {}).await.expect("目录已存在时再次上传应合并成功");
            assert_eq!(exec(&h, &format!("cat {remote}/a/b/f.txt")).await, "hello");

            exec(&h, &format!("rm -rf -- '{base}'")).await;
            std::fs::remove_dir_all(&src).unwrap();
        }

        /// 能 stat 但没有读权限的大文件：报错，且不截断本地同名文件（段 0 的句柄兼作探测，开成功才建本地文件）。
        #[tokio::test(flavor = "multi_thread")]
        #[ignore = "需要 SSHL_SFTP_IT=host:port 指向 sshd 测试容器"]
        async fn unreadable_large_file_leaves_local_file_untouched() {
            let Some((h, sftp)) = connect().await else { return };
            let base = exec(&h, "mktemp -d /home/alice/sshl-it.XXXXXXXX").await.trim().to_string();
            let remote = format!("{base}/secret.bin");
            exec(&h, &format!("truncate -s {SEGMENTED_DOWNLOAD_MIN} {remote} && chmod 000 {remote}")).await;
            let local = temp_dir("unreadable");
            write(&local, b"keep me");
            let result = download_large(&sftp, &remote, &local, SEGMENTED_DOWNLOAD_MIN, |_| {}).await;
            assert!(result.is_err(), "没有读权限应当报错");
            assert_eq!(std::fs::read(&local).unwrap(), b"keep me", "本地已有文件不能被截断");
            exec(&h, &format!("rm -rf -- '{base}'")).await;
            std::fs::remove_file(&local).unwrap();
        }

        /// OpenSSH 的 limits 扩展声明句柄上限 1019，russh-sftp 按它在客户端计数、只有等到 CLOSE 回复才减一：
        /// 下载完只 drop 不关的只读句柄会一直占着计数，同一会话下载约一千个文件后 open / 列目录全部失败。
        #[tokio::test(flavor = "multi_thread")]
        #[ignore = "需要 SSHL_SFTP_IT=host:port 指向 sshd 测试容器"]
        async fn downloading_many_files_does_not_exhaust_handle_limit() {
            let Some((h, sftp)) = connect().await else { return };
            let base = exec(&h, "mktemp -d /home/alice/sshl-it.XXXXXXXX").await.trim().to_string();
            exec(&h, &format!(
                "mkdir {base}/many && cd {base}/many && seq -f f%04g 1 1100 | xargs touch && for i in 1 2 3; do truncate -s {SEGMENTED_DOWNLOAD_MIN} big$i; done"
            )).await;
            let dst = temp_dir("many");
            download_tree(&sftp, &format!("{base}/many"), &dst.to_string_lossy(), |_| {}).await.unwrap();
            assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 1103);
            sftp.read_dir(base.as_str()).await.expect("下载之后还能列目录");
            exec(&h, &format!("rm -rf -- '{base}'")).await;
            std::fs::remove_dir_all(&dst).unwrap();
        }

        /// 分段数对大文件下载吞吐的影响（另需 SSHL_SFTP_BENCH=1）：同一连接上 4 / 8 / 16 段交替跑几轮，
        /// 排除机器负载、网络抖动对单次结果的干扰。
        #[tokio::test(flavor = "multi_thread")]
        #[ignore = "需要 SSHL_SFTP_IT=host:port 和 SSHL_SFTP_BENCH=1"]
        async fn bench_segment_count() {
            use super::super::{close_in_background, download_segmented};
            if std::env::var("SSHL_SFTP_BENCH").is_err() { return; }
            let Some((h, sftp)) = connect().await else { return };
            let base = exec(&h, "mktemp -d /home/alice/sshl-bench.XXXXXXXX").await.trim().to_string();
            let remote = format!("{base}/big.bin");
            exec(&h, &format!("head -c 100M /dev/urandom > {remote}")).await;
            let size = 100 * 1024 * 1024u64;
            let local = temp_dir("segments");
            let mut results: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
            for _ in 0..3 {
                for segments in [4, 8, 16] {
                    let t = Instant::now();
                    let first = sftp.open(remote.as_str()).await.unwrap();
                    tokio::fs::File::create(&local).await.unwrap();
                    let (sftp, remote, local) = (&sftp, remote.as_str(), local.as_path());
                    let n = download_segmented(
                        size,
                        segments,
                        first,
                        || async move { Ok(sftp.open(remote).await.unwrap()) },
                        || async move { Ok(tokio::fs::OpenOptions::new().write(true).open(local).await?) },
                        close_in_background,
                        remote,
                        |_| {},
                    ).await.unwrap();
                    assert_eq!(n, size);
                    results.entry(segments).or_default().push(t.elapsed().as_secs_f64());
                }
            }
            for (segments, secs) in &results {
                let list: Vec<String> = secs.iter().map(|s| format!("{s:.2}s")).collect();
                let best = secs.iter().cloned().fold(f64::MAX, f64::min);
                eprintln!("BENCH 100MB download, {segments:>2} segments: {} (best {:.1} MB/s)", list.join(" "), 100.0 / best);
            }
            exec(&h, &format!("rm -rf -- '{base}'")).await;
            std::fs::remove_file(&local).unwrap();
        }

        /// 耗时基准（另需 SSHL_SFTP_BENCH=1）：300 个目录 / 3000 个小文件的上传、遍历、下载，100MB 单文件上传、下载。
        /// 模拟 WAN：容器里 `apt-get install -y iproute2 && tc qdisc add dev eth0 root netem delay 50ms`。
        #[tokio::test(flavor = "multi_thread")]
        #[ignore = "需要 SSHL_SFTP_IT=host:port 和 SSHL_SFTP_BENCH=1"]
        async fn bench_directory_and_large_file() {
            if std::env::var("SSHL_SFTP_BENCH").is_err() { return; }
            let Some((h, sftp)) = connect().await else { return };
            let src = temp_dir("bench-src");
            let mut n = 0usize;
            for top in 0..10 {
                for sub in 0..29 {
                    for f in 0..10 {
                        write(&src.join(format!("d{top}/s{sub}/f{f}.txt")), &bytes((n * 7919) % 16384, n));
                        n += 1;
                    }
                }
                for f in 0..10 {
                    write(&src.join(format!("d{top}/f{f}.txt")), &bytes((n * 7919) % 16384, n));
                    n += 1;
                }
            }
            assert_eq!(n, 3000);
            let base = exec(&h, "mktemp -d /home/alice/sshl-bench.XXXXXXXX").await.trim().to_string();
            let remote = format!("{base}/tree");

            let t = Instant::now();
            upload_tree(&sftp, &src.to_string_lossy(), &remote, |_| {}).await.unwrap();
            eprintln!("BENCH upload tree (300 dirs / 3000 files): {:.2}s", t.elapsed().as_secs_f64());

            let t = Instant::now();
            let plan = scan_remote(remote.clone(), PathBuf::from("/unused"), |d| list_remote_dir(&sftp, d)).await.unwrap();
            eprintln!("BENCH scan remote tree ({} files): {:.2}s", plan.files.len(), t.elapsed().as_secs_f64());

            let dst = temp_dir("bench-dst");
            let t = Instant::now();
            download_tree(&sftp, &remote, &dst.to_string_lossy(), |_| {}).await.unwrap();
            eprintln!("BENCH download tree: {:.2}s", t.elapsed().as_secs_f64());
            assert!(snapshot(&src) == snapshot(&dst), "目录往返后内容不一致");

            let big = src.join("big.bin");
            let size = 100 * 1024 * 1024;
            std::fs::write(&big, bytes(size, 1)).unwrap();
            let remote_big = format!("{base}/big.bin");
            let t = Instant::now();
            let mut f = sftp.create(remote_big.as_str()).await.unwrap();
            stream_to_remote(&big.to_string_lossy(), &mut f, &remote_big, |_| {}).await.unwrap();
            f.shutdown().await.unwrap();
            let secs = t.elapsed().as_secs_f64();
            eprintln!("BENCH upload 100MB: {secs:.2}s ({:.1} MB/s)", 100.0 / secs);

            let got = dst.join("big.bin");
            let t = Instant::now();
            download_large(&sftp, &remote_big, &got, size as u64, |_| {}).await.unwrap();
            let secs = t.elapsed().as_secs_f64();
            eprintln!("BENCH download 100MB: {secs:.2}s ({:.1} MB/s)", 100.0 / secs);
            assert!(files_equal(&big, &got));

            exec(&h, &format!("rm -rf -- '{base}'")).await;
            std::fs::remove_dir_all(&src).unwrap();
            std::fs::remove_dir_all(&dst).unwrap();
        }
    }
}

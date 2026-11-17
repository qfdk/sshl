//! 打开文件：双击或右键「打开」。本地的交给默认程序（同 Finder）；远程的先下载到缓存目录（`~/Library/Caches/SSHL/open`），
//! 再用本机的程序打开，每秒看一眼副本，保存后自动传回原路径（文件管理当前身份，没权限时走 sudo 上传）。
//! 传回前比一比服务器上的大小和修改时间，打开之后被别人改过就先问。用什么打开、什么时候传在 `files::open`。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::time::SystemTime;

use sshl_core::sftp::RemoteStamp;

use super::super::open::{HEAD_BYTES, LARGE_FILE, OpenWith, open_with, ready_to_upload};
use super::transfer::upload_with_sudo;
use super::*;

/// 从服务器打开、正在盯着的一个文件。
pub(super) struct OpenedFile {
    session: FileSession,
    remote: String,
    local: PathBuf,
    /// 下载时 / 上次传回后服务器上的样子：传回前比一比，不一样就是这期间被别人改过。
    stamp: Option<RemoteStamp>,
    /// 副本上次和服务器一致时的修改时间（下载完、传回后）。
    synced: Option<SystemTime>,
    /// 上一轮看到的修改时间：连着两轮一样才传（编辑器写到一半不传）。
    seen: Option<SystemTime>,
    /// 正在传回（或在问要不要覆盖）。
    busy: bool,
}

impl FileManager {
    /// 双击文件 / 右键「打开」。
    pub(super) fn open_file(&mut self, kind: PaneKind, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        if entry.is_directory || entry.is_parent() {
            return;
        }
        match kind {
            PaneKind::Local => launch(Path::new(&entry.path), None),
            PaneKind::Remote => self.open_remote(entry.clone(), window, cx),
        }
    }

    fn open_remote(&mut self, entry: FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let existing = self
            .opened
            .iter()
            .find(|file| file.session.session_id == session.session_id && file.remote == entry.path)
            .map(|file| (file.local.clone(), file.stamp, file.synced));
        cx.spawn_in(window, async move |this, cx| {
            let sid = session.session_id.clone();
            let name = entry.name.clone();
            // 先记下服务器上的样子再下载：下载途中被改了，保存时会多问一句，不会悄悄覆盖
            let stamp = remote_stamp(&sid, &entry.path).await;
            if let Some((local, recorded, synced)) = &existing
                // 服务器上没变，或者副本上还有没传回去的改动（不能冲掉）：直接打开手上这份
                && (stamp == *recorded || modified(local) != *synced)
            {
                if let Some(with) = open_with(&name, &read_head(local)) {
                    launch(local, Some(with));
                }
                return;
            }
            if existing.is_none() && entry.size > LARGE_FILE {
                let options = Options::new(tr!("「{}」有 {}", name, format_file_size(entry.size)))
                    .message(tr!("要先整个下载下来才能打开。继续吗？"))
                    .confirm(tr!("打开"));
                let Ok(answer) = cx.update(|window, cx| prompts::confirm(window, cx, options)) else { return };
                if !answer.await.unwrap_or(false) {
                    return;
                }
            }
            let local = existing.map(|(local, ..)| local).unwrap_or_else(|| new_copy_path(&name));
            let (remote, path) = (entry.path.clone(), local.to_string_lossy().into_owned());
            let b = backend::get();
            let downloaded = backend::spawn(async move {
                prepare_cache(Path::new(&path))?;
                sftp::file_download(&b.events, &b.state, sid, remote, path).await
            });
            let downloaded = downloaded.await;
            // 下载的进度条不留着（打开了、打不开、下载失败都一样）
            let _ = this.update(cx, |this, cx| {
                this.status.hide();
                cx.notify();
            });
            if let Outcome::Failed(error) = Outcome::from_result(downloaded) {
                alert(cx, tr!("打开失败"), error);
                return;
            }
            let Some(with) = open_with(&name, &read_head(&local)) else {
                let _ = std::fs::remove_dir_all(local.parent().unwrap_or(&local));
                alert(cx, tr!("不能直接打开「{}」", name), tr!("从服务器打开的只能是文本、图片、PDF 和音视频：别的文件交给默认程序可能会被执行或安装。可以下载下来再处理。"));
                return;
            };
            let synced = modified(&local);
            let _ = this.update_in(cx, |this, window, cx| {
                this.opened.retain(|file| file.local != local);
                this.opened.push(OpenedFile { session, remote: entry.path.clone(), local: local.clone(), stamp, synced, seen: synced, busy: false });
                this.watch_opened(window, cx);
            });
            launch(&local, Some(with));
        })
        .detach();
    }

    /// 每秒看一眼打开的副本，保存完了就传回去。没有在盯的了就停。
    fn watch_opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.watching {
            return;
        }
        self.watching = true;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(paths) = this.update(cx, |this, _| {
                    let paths: Vec<PathBuf> = this.opened.iter().map(|file| file.local.clone()).collect();
                    if paths.is_empty() {
                        this.watching = false;
                    }
                    paths
                }) else {
                    return;
                };
                if paths.is_empty() {
                    return;
                }
                let times = cx.background_executor().spawn(async move { paths.into_iter().map(|path| (modified(&path), path)).collect::<Vec<_>>() }).await;
                let _ = this.update_in(cx, |this, window, cx| this.check_opened(times, window, cx));
            }
        })
        .detach();
    }

    fn check_opened(&mut self, times: Vec<(Option<SystemTime>, PathBuf)>, window: &mut Window, cx: &mut Context<Self>) {
        for (now, path) in times {
            let Some(file) = self.opened.iter_mut().find(|file| file.local == path && !file.busy) else { continue };
            let ready = ready_to_upload(file.synced, file.seen, now);
            file.seen = now;
            if ready {
                file.busy = true;
                let job = (file.session.clone(), file.remote.clone(), file.local.clone(), file.stamp, now);
                self.upload_opened(job, window, cx);
            }
        }
    }

    /// 把保存过的副本传回去。`now` 是这次要传的那一版的修改时间。
    fn upload_opened(&mut self, job: (FileSession, String, PathBuf, Option<RemoteStamp>, Option<SystemTime>), window: &mut Window, cx: &mut Context<Self>) {
        let (session, remote, local, stamp, now) = job;
        cx.spawn_in(window, async move |this, cx| {
            let name = basename(&remote);
            let current = remote_stamp(&session.session_id, &remote).await;
            if current != stamp {
                let mut options = Options::new(tr!("「{}」在服务器上被改过了", name))
                    .message(tr!("打开之后，服务器上的这个文件又被改过。用你保存的版本覆盖它吗？"))
                    .detail(parent_path(&remote, PaneKind::Remote))
                    .confirm(tr!("覆盖"))
                    .cancel(tr!("不传"));
                options.destructive = true;
                let overwrite = match cx.update(|window, cx| prompts::confirm(window, cx, options)) {
                    Ok(answer) => answer.await.unwrap_or(false),
                    Err(_) => false,
                };
                if !overwrite {
                    // 这一版不传；下次保存还会再问（服务器上的仍然不是你打开时的那份）
                    let _ = this.update(cx, |this, _| this.opened_done(&local, now, None));
                    return;
                }
            }
            let stop = Arc::new(AtomicBool::new(false));
            let outcome = upload_with_sudo(&this, cx, &session, local.to_string_lossy().into_owned(), remote.clone(), false, stop).await;
            let stamp = match outcome {
                Outcome::Ok => Some(remote_stamp(&session.session_id, &remote).await),
                _ => None,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                // 传失败也算处理过这一版：不然每秒重试、每秒弹框。下次保存再传
                this.opened_done(&local, now, stamp);
                match outcome {
                    Outcome::Ok => {
                        this.status = TransferStatus::show(100, tr!("{} 已传回服务器", name));
                        this.hide_status_later(cx);
                        if this.session_id().as_deref() == Some(session.session_id.as_str()) && this.remote.path == parent_path(&remote, PaneKind::Remote) {
                            this.invalidate(PaneKind::Remote);
                            this.refresh_list(PaneKind::Remote, window, cx);
                        }
                    }
                    Outcome::Failed(error) => prompts::alert_later(window, cx, Options::new(tr!("传回服务器失败")).detail(remote.clone()).message(error)),
                    Outcome::Cancelled | Outcome::Stopped => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 这一版处理完了（传了、没传、传失败）：记下它的修改时间；传成功时换上服务器上新的样子。
    fn opened_done(&mut self, local: &Path, synced: Option<SystemTime>, stamp: Option<Option<RemoteStamp>>) {
        if let Some(file) = self.opened.iter_mut().find(|file| file.local == local) {
            file.busy = false;
            file.synced = synced;
            if let Some(stamp) = stamp {
                file.stamp = stamp;
            }
        }
    }

    /// 会话没了：它的文件不再传回。
    pub(super) fn forget_opened(&mut self, session_id: &str) {
        self.opened.retain(|file| file.session.session_id != session_id);
    }
}

async fn remote_stamp(session_id: &str, path: &str) -> Option<RemoteStamp> {
    let (sid, path) = (session_id.to_string(), path.to_string());
    backend::spawn(async move { sftp::file_remote_stamp(&backend::get().state, sid, path).await }).await.and_then(Result::ok).flatten()
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn read_head(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut head = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(HEAD_BYTES as u64).read_to_end(&mut head);
    }
    head
}

fn cache_root() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("SSHL").join("open")
}

/// 每个打开的文件一个目录，文件名照原样（编辑器标题、按扩展名的语法高亮都对）。
fn new_copy_path(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    cache_root().join(format!("{stamp:x}-{}", NEXT.fetch_add(1, Ordering::Relaxed))).join(name)
}

/// 建好副本所在的目录（只有自己能进：副本可能是 .env 之类）。这次运行第一次打开时顺手清掉上次留下的副本。
fn prepare_cache(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    static CLEANED: AtomicBool = AtomicBool::new(false);
    let root = cache_root();
    if !CLEANED.swap(true, Ordering::Relaxed) {
        let _ = std::fs::remove_dir_all(&root);
    }
    let dir = path.parent().unwrap_or(&root);
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// 交给系统打开：`Some(TextEditor)` 是默认文本编辑器，其余是这种文件的默认程序。测试口开着时只记日志，不去开别的程序。
fn launch(path: &Path, with: Option<OpenWith>) {
    tracing::info!("open file: {}", path.display());
    #[cfg(any(debug_assertions, feature = "devhook"))]
    if std::env::var_os("SSHL_DEV_SOCKET").is_some() {
        return;
    }
    let mut command = std::process::Command::new("/usr/bin/open");
    if with == Some(OpenWith::TextEditor) {
        command.arg("-t");
    }
    match command.arg(path).spawn() {
        // 等它退出，不留僵尸进程
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(error) => tracing::warn!("open {}: {error}", path.display()),
    }
}

//! 传输：一次传一个的队列、进度、同名先问、没有写权限时用 sudo 上传、拖到终端上传。

use super::*;

impl FileManager {
    /// 正在传的和排队的一共几个（断开前提醒用）。
    pub fn active_transfers(&self) -> usize {
        self.queue.len() + usize::from(self.current.is_some())
    }

    /// 传完 / 停下 / 跳过的那句提示 3 秒后收起。
    pub(super) fn hide_status_later(&mut self, cx: &mut Context<Self>) {
        self.hide_status = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update(cx, |this, cx| {
                this.status.hide();
                cx.notify();
            });
        }));
    }

    /// 有传输正在进行（显示「停止」）。
    pub fn is_transferring(&self) -> bool {
        self.current.is_some()
    }

    /// 停止：排队的不传了，正在传的在下一块时停下（服务器 / 本地原有的同名文件不受影响）。
    pub fn stop_transfers(&mut self, cx: &mut Context<Self>) {
        self.queue.clear();
        if let Some(stop) = &self.current {
            stop.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    pub fn core_event(&mut self, event: &CoreEvent, cx: &mut Context<Self>) {
        let (upload, progress) = match event {
            CoreEvent::UploadProgress(p) => (true, TransferProgress::from(p)),
            CoreEvent::DownloadProgress(p) => (false, TransferProgress::from(p)),
            CoreEvent::ConnectionsUpdated => return,
        };
        let speed = self.speed.record(std::time::Instant::now(), progress.transferred);
        self.status = describe_progress(upload, &progress, speed, self.queue.len());
        cx.notify();
    }

    pub(super) fn upload_items(&mut self, items: Vec<FileEntry>, directory: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let directory = directory.unwrap_or_else(|| self.remote.path.clone());
        let jobs = items
            .into_iter()
            .map(|item| Job {
                session: session.clone(),
                op: if item.is_directory { Op::UploadDir } else { Op::UploadFile },
                destination: join_path(&directory, &basename(&item.path)),
                source: item.path,
            })
            .collect();
        self.enqueue(jobs, window, cx);
    }

    pub(super) fn download_items(&mut self, items: Vec<FileEntry>, directory: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let directory = directory.unwrap_or_else(|| self.local.path.clone());
        if directory.is_empty() {
            return;
        }
        let jobs = items
            .into_iter()
            .map(|item| Job {
                session: session.clone(),
                op: if item.is_directory { Op::DownloadDir } else { Op::DownloadFile },
                destination: join_path(&directory, &basename(&item.path)),
                source: item.path,
            })
            .collect();
        self.enqueue(jobs, window, cx);
    }

    pub(super) fn upload_paths(&mut self, paths: Vec<String>, directory: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let kinds = backend::spawn(local_fs::file_local_kinds(paths.clone()));
        cx.spawn_in(window, async move |this, cx| {
            let kinds = kinds.await.unwrap_or_else(|| vec![false; paths.len()]);
            let jobs = paths
                .into_iter()
                .enumerate()
                .map(|(index, local)| Job {
                    session: session.clone(),
                    op: if kinds.get(index).copied().unwrap_or(false) { Op::UploadDir } else { Op::UploadFile },
                    destination: join_path(&directory, &basename(&local)),
                    source: local,
                })
                .collect();
            let _ = this.update_in(cx, |this, window, cx| this.enqueue(jobs, window, cx));
        })
        .detach();
    }

    /// 加进传输队列；队列没在跑就跑起来，一个传完接下一个。
    pub(super) fn enqueue(&mut self, jobs: Vec<Job>, window: &mut Window, cx: &mut Context<Self>) {
        if jobs.is_empty() {
            return;
        }
        self.queue.extend(jobs);
        cx.notify();
        if self.worker {
            return;
        }
        self.worker = true;
        self.finished = 0;
        self.on_conflict = None;
        cx.spawn_in(window, async move |this, cx| {
            // 取下一个和「队列空了就停」在同一次 update 里：中间不会有新加的任务被漏掉
            while let Some(job) = this
                .update(cx, |this, _| {
                    let job = this.queue.pop_front();
                    this.worker = job.is_some();
                    job
                })
                .ok()
                .flatten()
            {
                run_transfer(&this, cx, job).await;
            }
        })
        .detach();
    }

    pub(super) fn select_folder_to_upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let directory = self.remote.path.clone();
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().into_owned();
                let _ = this.update_in(cx, |this, window, cx| this.upload_paths(vec![path], directory, window, cx));
            }
        })
        .detach();
    }

    pub(super) fn transfer_finished(&mut self, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        let kind = if op.is_upload() { PaneKind::Remote } else { PaneKind::Local };
        if kind == PaneKind::Remote && self.session.is_none() {
            return;
        }
        self.invalidate(kind);
        let path = self.pane(kind).path.clone();
        let path = if kind == PaneKind::Remote && path.is_empty() { "/".to_string() } else { path };
        self.load(kind, Some(path), true, window, cx);
    }

    /// 文件拖到终端上：传到终端此刻所在的目录（读不到就传到文件管理的远程目录，再不行传到主目录）。
    /// 先对一下终端此刻是谁：root 登录后 su 成别人，传上去的文件要归那个人，不能还是 root 的。
    pub fn upload_to_terminal_cwd(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.after_sync.push(Box::new(move |this, window, cx| this.upload_to_terminal_cwd_now(paths, window, cx)));
        self.sync_identity(true, window, cx);
    }

    pub(super) fn upload_to_terminal_cwd_now(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let fallback = self.remote.path.clone();
        let sid = session.session_id.clone();
        let directory = backend::spawn(async move {
            let state = &backend::get().state;
            match sftp::file_terminal_cwd(state, sid.clone()).await.ok().flatten() {
                Some(cwd) => Some(cwd),
                None if !fallback.is_empty() => Some(fallback),
                None => sftp::file_remote_home(state, sid).await.ok().filter(|home| home.starts_with('/')),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let directory = directory.await.flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                // 等目录期间切走了会话：不往别的服务器传
                if this.session_id().as_deref() != Some(session.session_id.as_str()) {
                    return;
                }
                match directory {
                    Some(directory) => this.upload_paths(paths, directory, window, cx),
                    None => prompts::alert_later(window, cx, Options::new(tr!("上传失败")).message(tr!("读不到终端当前目录，也拿不到远程主目录。"))),
                }
            });
        })
        .detach();
    }

    /// 正在进行或刚结束的传输（终端界面上也要看得到进度）。
    pub fn transfer_status(&self) -> Option<&TransferStatus> {
        self.status.active.then_some(&self.status)
    }

    pub(super) fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let progress = self.status.progress.min(100) as f32 / 100.;
        div()
            .h(px(44.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .px(px(14.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .text_size(px(12.))
            .child(
                div()
                    .size(px(26.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .bg(theme::color(ui::ACCENT))
                    .text_color(theme::color(ui::PRIMARY))
                    .child(kit::icon_sized(Icon::ArrowDownUp, 14.)),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.status.info.clone()))
            .child(
                div()
                    .w(px(192.))
                    .h(px(6.))
                    .flex_shrink_0()
                    .rounded_full()
                    .overflow_hidden()
                    .bg(theme::alpha(ui::PRIMARY, 0.2))
                    .child(div().h_full().w(gpui::relative(progress)).bg(theme::color(ui::PRIMARY))),
            )
            .when(self.current.is_some(), |d| {
                d.child(
                    kit::mini_button("transfer-stop", Icon::X, tr!("停止传输"), ui::MUTED_FOREGROUND)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.stop_transfers(cx))),
                )
            })
            .into_any_element()
    }
}

/// 传输队列里的一项。
pub(super) struct Job {
    session: FileSession,
    op: Op,
    source: String,
    destination: String,
}

/// 一次传输（旧 useTransfer.run）：进度条、完成后刷新目标一侧、失败弹框、3 秒后收起。
#[derive(Clone, Copy)]
pub(super) enum Op {
    UploadFile,
    UploadDir,
    DownloadFile,
    DownloadDir,
}

impl Op {
    pub(super) fn verb(self) -> &'static str {
        match self {
            Op::UploadFile => tr!("上传"),
            Op::UploadDir => tr!("上传文件夹"),
            Op::DownloadFile => tr!("下载"),
            Op::DownloadDir => tr!("下载文件夹"),
        }
    }

    /// 「正在……」后面接的那个词：中文和 `verb` 一样，英文是进行时。
    pub(super) fn doing(self) -> &'static str {
        match self {
            Op::UploadFile => trc!("进行中", "上传"),
            Op::UploadDir => trc!("进行中", "上传文件夹"),
            Op::DownloadFile => trc!("进行中", "下载"),
            Op::DownloadDir => trc!("进行中", "下载文件夹"),
        }
    }

    pub(super) fn is_upload(self) -> bool {
        matches!(self, Op::UploadFile | Op::UploadDir)
    }

    pub(super) fn is_dir(self) -> bool {
        matches!(self, Op::UploadDir | Op::DownloadDir)
    }

    /// 目标落在哪一边。
    pub(super) fn target_side(self) -> PaneKind {
        if self.is_upload() { PaneKind::Remote } else { PaneKind::Local }
    }
}

/// 目标已有同名的时问的话：（标题，说明，复选框）。`merge` = 文件夹传到已有的同名文件夹；还有排队的才给复选框。
pub(super) fn conflict_text(name: &str, merge: bool, queued: usize) -> (String, &'static str, Option<&'static str>) {
    let message = if merge { tr!("覆盖会合并进原来的文件夹，同名的文件被替换。") } else { tr!("覆盖会替换原来的文件。") };
    (tr!("{} 已存在", name), message, (queued > 0).then_some(tr!("后面再遇到同名的也这样处理")))
}

/// 目标在不在：None = 不在（或看不到，照常传），Some(是不是目录)。上传看服务器（以文件管理当前身份），下载看本地。
pub(super) async fn destination_exists(session: &FileSession, op: Op, destination: &str) -> Option<bool> {
    let path = destination.to_string();
    if op.is_upload() {
        let sid = session.session_id.clone();
        backend::spawn(async move { sftp::file_remote_exists(&backend::get().state, sid, path).await }).await.and_then(Result::ok).flatten()
    } else {
        backend::spawn(async move { local_fs::local_exists(&path) }).await.flatten()
    }
}

/// 传完的提示写明落在哪个目录：从终端拖文件上传时，看这一句就知道传到哪了。
/// 一次排了好几个时说一共几个（`count` = 这一轮传完的个数，含这一个）。
pub(super) fn finished_message(op: Op, destination: &str, count: usize) -> String {
    let place = parent_path(destination, op.target_side());
    match (op.is_upload(), count > 1) {
        (true, true) => tr!("{} 个已上传到 {}", count, place),
        (true, false) => tr!("{} 已上传到 {}", basename(destination), place),
        (false, true) => tr!("{} 个已下载到 {}", count, place),
        (false, false) => tr!("{} 已下载到 {}", basename(destination), place),
    }
}

pub(super) async fn run_transfer(this: &WeakEntity<FileManager>, cx: &mut AsyncWindowContext, job: Job) -> Outcome {
    let Job { session, op, source, destination } = job;
    let destination_for_message = destination.clone();
    let verb = op.verb();
    let name = basename(&source);
    let stop = Arc::new(AtomicBool::new(false));
    let started = this.update(cx, |this, cx| {
        this.current = Some(stop.clone());
        this.speed.reset();
        this.hide_status = None;
        let queued = if this.queue.is_empty() { String::new() } else { tr!(" · 还有 {} 个", this.queue.len()) };
        this.status = TransferStatus::show(0, tr!("正在{} {}{}", op.doing(), name, queued));
        cx.notify();
    });
    if started.is_err() {
        return Outcome::Cancelled;
    }
    // 目标已有同名的：问覆盖还是跳过（勾过「后面也这样」就照办，不再问）
    if let Some(existing_dir) = destination_exists(&session, op, &destination).await {
        let (remembered, queued) = this.read_with(cx, |this, _| (this.on_conflict, this.queue.len())).unwrap_or((None, 0));
        let overwrite = match remembered {
            Some(choice) => choice,
            None => {
                let (title, message, checkbox) = conflict_text(&name, op.is_dir() && existing_dir, queued);
                let mut options = Options::new(title).detail(parent_path(&destination, op.target_side())).message(message).confirm(tr!("覆盖")).cancel(tr!("跳过"));
                options.destructive = true;
                if let Some(label) = checkbox {
                    options = options.checkbox(label);
                }
                let Ok(answer) = cx.update(|window, cx| prompts::confirm_with_check(window, cx, options)) else { return Outcome::Cancelled };
                let (overwrite, remember) = answer.await.unwrap_or((false, false));
                if remember {
                    let _ = this.update(cx, |this, _| this.on_conflict = Some(overwrite));
                }
                overwrite
            }
        };
        if !overwrite {
            let _ = this.update(cx, |this, cx| {
                this.current = None;
                this.status = TransferStatus::show(0, tr!("已跳过 {}", name));
                this.hide_status_later(cx);
                cx.notify();
            });
            return Outcome::Cancelled;
        }
    }
    let outcome = match op {
        Op::UploadFile | Op::UploadDir => upload_with_sudo(this, cx, &session, source, destination, matches!(op, Op::UploadDir), stop).await,
        Op::DownloadFile => {
            let sid = session.session_id.clone();
            let b = backend::get();
            Outcome::from_transfer(
                backend::spawn(sftp::cancellable(stop, async move { sftp::file_download(&b.events, &b.state, sid, source, destination).await })).await,
            )
        }
        Op::DownloadDir => {
            let sid = session.session_id.clone();
            let b = backend::get();
            Outcome::from_transfer(
                backend::spawn(sftp::cancellable(stop, async move { sftp::file_download_directory(&b.events, &b.state, sid, source, destination).await }))
                    .await,
            )
        }
    };
    let finished = outcome.clone();
    let _ = this.update_in(cx, |this, window, cx| {
        this.current = None;
        match &finished {
            Outcome::Ok => {
                this.finished += 1;
                this.status = TransferStatus::show(100, finished_message(op, &destination_for_message, this.finished));
                this.transfer_finished(op, window, cx);
            }
            // 用户在 sudo 确认或密码框里取消：不算失败，不弹报错
            Outcome::Cancelled => this.status = TransferStatus::show(0, tr!("已取消{}", verb)),
            // 停在中途：目录传输已经传完的文件留着，刷新一下目标那边
            Outcome::Stopped => {
                this.status = TransferStatus::show(0, tr!("已停止{}", verb));
                this.transfer_finished(op, window, cx);
            }
            Outcome::Failed(error) => {
                this.status = TransferStatus::show(0, tr!("{}失败", verb));
                // 会话没了：同一会话后面排队的也传不了，别一个个弹框
                if error.contains("session not found") {
                    this.queue.retain(|job| job.session.session_id != session.session_id);
                }
                prompts::alert_later(window, cx, Options::new(tr!("{}失败", verb)).message(error.clone()));
            }
        }
        this.hide_status_later(cx);
        cx.notify();
    });
    outcome
}

/// 普通上传；目标目录无写权限时问一次是否用 sudo（见 sudo.rs）。
pub(super) async fn upload_with_sudo(
    this: &WeakEntity<FileManager>,
    cx: &mut AsyncWindowContext,
    session: &FileSession,
    local: String,
    remote: String,
    is_dir: bool,
    stop: Arc<AtomicBool>,
) -> Outcome {
    let sudo_stop = stop.clone();
    let sid = session.session_id.clone();
    let elevated = this.read_with(cx, |this, _| this.records.get(&sid).and_then(|r| r.identity.as_ref()).is_some_and(FileIdentity::elevated)).unwrap_or(false);
    let user = Rc::new(RefCell::new(String::new()));
    let parent = {
        let trimmed = remote.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(i) if i > 0 => trimmed[..i].to_string(),
            _ => "/".to_string(),
        }
    };
    let (normal_sid, normal_local, normal_remote) = (sid.clone(), local.clone(), remote.clone());
    let (sudo_sid, sudo_local, sudo_remote) = (sid.clone(), local.clone(), remote.clone());
    let own_sid = sid.clone();
    let own_user = user.clone();
    let ask_user = user.clone();
    let connection_id = session.connection_id.clone();
    let mut confirm_cx = cx.clone();
    let mut ask_cx = cx.clone();
    let outcome = upload_with_sudo_fallback(
        async move || {
            let b = backend::get();
            Outcome::from_transfer(
                backend::spawn(sftp::cancellable(stop, async move {
                    if is_dir {
                        sftp::file_upload_directory(&b.events, &b.state, normal_sid, normal_local, normal_remote).await
                    } else {
                        sftp::file_upload(&b.events, &b.state, normal_sid, normal_local, normal_remote).await
                    }
                }))
                .await,
            )
        },
        async move |secret| {
            let b = backend::get();
            let (password, credential) = split_secret(secret);
            let (sid, local, remote) = (sudo_sid.clone(), sudo_local.clone(), sudo_remote.clone());
            Outcome::from_transfer(
                backend::spawn(sftp::cancellable(sudo_stop.clone(), async move {
                    sftp::file_upload_sudo(&b.events, &b.state, sid, local, remote, is_dir, password, credential).await
                }))
                .await,
            )
        },
        async move || {
            let options = Options::new(tr!("没有写入权限"))
                .message(tr!("当前登录用户不能写入这个目录。要用 sudo 上传吗？文件会归目录的所有者。"))
                .detail(parent)
                .confirm(tr!("用 sudo 上传"));
            let Ok(answer) = confirm_cx.update(|window, cx| prompts::confirm(window, cx, options)) else { return false };
            answer.await.unwrap_or(false)
        },
        async move || {
            let identity = backend::spawn(async move { sftp::file_sudo_identity(&backend::get().state, own_sid).await }).await;
            let identity = identity.and_then(Result::ok)?;
            *own_user.borrow_mut() = identity.user;
            identity.credential
        },
        async move |retry, exclude| {
            let user = ask_user.borrow().clone();
            let connection_id = connection_id.clone();
            let saved = backend::spawn(async move { config_store::cred_list(connection_id) }).await;
            let choices = saved_password_choices(saved.as_ref(), &user, &exclude, &user);
            let title = if retry { tr!("sudo 密码不正确") } else { tr!("需要 sudo 密码") };
            let message = if user.is_empty() { tr!("sudo 要的是当前登录用户的密码。").to_string() } else { tr!("sudo 要的是当前登录用户 {} 的密码。", user) };
            ask_password(&mut ask_cx, Options::new(title).message(message).confirm(tr!("上传")), choices).await
        },
        elevated,
    )
    .await;
    match outcome {
        Outcome::Failed(error) if error.starts_with("SUDO_") => Outcome::Failed(sudo_error_message(Some(&error))),
        other => other,
    }
}

pub(super) fn split_secret(secret: Option<Secret>) -> (Option<String>, Option<String>) {
    match secret {
        Some(Secret::Password(password)) => (Some(password), None),
        Some(Secret::Credential(credential)) => (None, Some(credential)),
        None => (None, None),
    }
}

/// 密码框：确定 → 手输的密码，点选已保存的 → 它的标识（密码不经过界面），取消 → None。
pub(super) async fn ask_password(cx: &mut AsyncWindowContext, options: Options, choices: Vec<(String, String)>) -> Option<Secret> {
    let placeholder = if choices.is_empty() { tr!("输入密码") } else { tr!("或输入密码") };
    let choices = choices.into_iter().map(|(value, label)| Choice { value, label: label.into() }).collect();
    let answer = cx.update(|window, cx| prompts::password(window, cx, options, placeholder, choices)).ok()?;
    match answer.await.ok().flatten()? {
        PasswordAnswer::Password(password) => Some(Secret::Password(password)),
        PasswordAnswer::Choice(choice) => Some(Secret::Credential(choice)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Op, conflict_text, finished_message};

    #[test]
    fn finished_transfers_say_where_the_file_went() {
        assert_eq!(finished_message(Op::UploadFile, "/var/www/html/a.zip", 1), "a.zip 已上传到 /var/www/html");
        assert_eq!(finished_message(Op::UploadDir, "/srv/app", 1), "app 已上传到 /srv");
        assert_eq!(finished_message(Op::DownloadFile, "/Users/me/Downloads/log.txt", 1), "log.txt 已下载到 /Users/me/Downloads");
        assert_eq!(finished_message(Op::UploadFile, "/home/q/b.txt", 3), "3 个已上传到 /home/q", "排了好几个时说一共几个");
    }

    #[test]
    fn same_name_prompt_says_what_overwriting_does() {
        assert_eq!(conflict_text("a.zip", false, 0), ("a.zip 已存在".into(), "覆盖会替换原来的文件。", None));
        let (_, message, checkbox) = conflict_text("app", true, 2);
        assert_eq!(message, "覆盖会合并进原来的文件夹，同名的文件被替换。");
        assert_eq!(checkbox, Some("后面再遇到同名的也这样处理"), "还有排队的才问要不要都这样");
    }
}

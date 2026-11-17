//! 跟随终端用户：终端里 su / sudo -i 之后，文件管理也以那个用户运行；终端 cd 之后跟过去。
//! 地址栏左边的身份徽标也在这里。

use super::transfer::{ask_password, split_secret};
use super::*;

impl FileManager {
    /// 终端 cd 到了别处：文件管理跟过去（只在文件管理与终端是同一个用户时，见 sync_identity）。
    pub(super) fn follow_terminal_cwd(&mut self, session_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let sid = session_id.clone();
        let cwd = backend::spawn(async move { sftp::file_terminal_cwd(&backend::get().state, sid).await });
        cx.spawn_in(window, async move |this, cx| {
            let cwd = cwd.await.and_then(Result::ok).flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                if this.session_id().as_deref() != Some(session_id.as_str()) || this.remote.path.is_empty() {
                    return;
                }
                let Some(path) = this.records.get_mut(&session_id).and_then(|record| record.follow_terminal_cwd(cwd)) else { return };
                if path != this.remote.path {
                    this.load(PaneKind::Remote, Some(path), false, window, cx);
                }
            });
        })
        .detach();
    }

    /// 「系统」页要以终端此刻的用户跑命令：切过去时同步一下身份（不弹密码框，已拒绝过的不再问）。
    pub fn sync_identity_quietly(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_some() {
            self.sync_identity(false, window, cx);
        }
    }

    /// 切到文件管理 / 刷新时看终端此刻是谁，变了就切换身份并去终端所在目录（读不到就去他的主目录）；
    /// 没变就看终端是否 cd 到了别处，跟过去。`interactive`：点徽标。
    pub(super) fn sync_identity(&mut self, interactive: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        if self.syncing {
            return;
        }
        self.syncing = true;
        let record = self.records.get(&session.session_id).cloned().unwrap_or_default();
        cx.spawn_in(window, async move |this, cx| {
            let sid = session.session_id.clone();
            let names = Rc::new(RefCell::new((String::new(), String::new())));
            let identify_names = names.clone();
            let identify_sid = sid.clone();
            let switch_sid = sid.clone();
            let connection_id = session.connection_id.clone();
            let mut ask_cx = cx.clone();
            let result = sync_file_user(
                async move || {
                    let who = backend::spawn(async move { sftp::file_terminal_user(&backend::get().state, identify_sid).await }).await;
                    let who: TerminalIdentity = who?.ok()?.into();
                    *identify_names.borrow_mut() = (who.login_user.clone(), who.user.clone());
                    Some(who)
                },
                async move |target, secret| {
                    let sid = switch_sid.clone();
                    let (user, via) = target.map(|t| (Some(t.user), Some(t.via.as_str().to_string()))).unwrap_or((None, None));
                    let (password, credential) = split_secret(secret);
                    Outcome::from_result(
                        backend::spawn(async move { sftp::file_switch_user(&backend::get().state, sid, user, via, password, credential).await }).await,
                    )
                },
                async move |retry, exclude, context| {
                    let connection_id = connection_id.clone();
                    let saved = backend::spawn(async move { config_store::cred_list(connection_id) }).await;
                    let choices = saved_password_choices(saved.as_ref(), &context.need_user, &exclude, &context.login_user);
                    let title = if retry { tr!("{} 的密码不正确", context.need_user) } else { tr!("以 {} 身份管理文件", context.user) };
                    let message = match context.via {
                        Via::Su => tr!("终端已切换到 {0}，文件管理也以 {0} 身份浏览、上传和下载，需要 {0} 的密码。", context.user),
                        Via::Sudo => tr!(
                            "终端用 sudo 切换到了 {0}，文件管理也以 {0} 身份浏览、上传和下载，需要 {1} 的 sudo 密码。",
                            context.user,
                            context.login_user
                        ),
                    };
                    ask_password(&mut ask_cx, Options::new(title).message(message).confirm(tr!("切换")), choices).await
                },
                interactive,
                record.declined.as_deref(),
                &record.rejected_saved,
            )
            .await;
            let (login_user, terminal_user) = names.borrow().clone();
            let _ = this.update_in(cx, |this, window, cx| {
                this.syncing = false;
                // 等着这次同步的事先取出来：下面提前返回时也一并丢掉，不会留给别的会话去做
                let pending = std::mem::take(&mut this.after_sync);
                // 会话在等密码时已被移除：丢弃结果，不写记录、不刷新、不弹报错
                if !record_sync_result(&mut this.records, &sid, &result, &login_user, &terminal_user, record.declined.as_deref()) {
                    return;
                }
                match result.status {
                    // 第一次打开时 remote.path 还空着：load_remote_initial 会去终端目录
                    SyncStatus::Same if this.session_id().as_deref() == Some(sid.as_str()) && !this.remote.path.is_empty() => {
                        this.follow_terminal_cwd(sid.clone(), window, cx);
                    }
                    SyncStatus::Switched => {
                        if this.session_id().as_deref() == Some(sid.as_str()) {
                            this.remote_cache.clear();
                            if let Some(record) = this.records.get_mut(&sid) {
                                record.remote_path.clear();
                            }
                            this.remote.path.clear();
                            this.load_remote_initial(window, cx);
                        }
                    }
                    SyncStatus::Failed => prompts::alert_later(
                        window,
                        cx,
                        Options::new(tr!("无法以 {} 身份管理文件", terminal_user)).message(sudo_error_message(result.error.as_deref())),
                    ),
                    _ => {}
                }
                // 等着这次同步的事：身份对上了（没变或刚切过去）且还在这个会话上才做，否则丢掉
                if matches!(result.status, SyncStatus::Same | SyncStatus::Switched) && this.session_id().as_deref() == Some(sid.as_str()) {
                    for then in pending {
                        then(this, window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_badge(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.session.as_ref()?;
        let identity = self.identity()?;
        if identity.elevated() {
            let user = identity.file_user.clone();
            let title: SharedString = tr!("文件管理以 {} 身份运行（跟随终端）", user).into();
            return Some(
                div()
                    .id("file-user-badge")
                    .h(px(24.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .rounded(px(6.))
                    .bg(theme::alpha(ui::PRIMARY, 0.1))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(ui::PRIMARY))
                    .tooltip(move |window, cx| kit::tooltip(title.clone(), window, cx))
                    .child(kit::icon_sized(Icon::UserRound, 14.))
                    .child(user)
                    .into_any_element(),
            );
        }
        let terminal = identity.terminal_user.clone().filter(|t| *t != identity.file_user)?;
        let title: SharedString = tr!("终端当前是 {}，点击让文件管理也切过去", terminal).into();
        Some(
            div()
                .id("file-user-switch")
                .role(Role::Button)
                .h(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .rounded(px(6.))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .text_size(px(12.))
                .text_color(theme::color(ui::MUTED_FOREGROUND))
                .cursor_pointer()
                .hover(|s| s.bg(theme::color(ui::MUTED)))
                .tooltip(move |window, cx| kit::tooltip(title.clone(), window, cx))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.sync_identity(true, window, cx)))
                .child(kit::icon_sized(Icon::UserRound, 14.))
                .child(tr!("切到 {}", terminal))
                .into_any_element(),
        )
    }
}

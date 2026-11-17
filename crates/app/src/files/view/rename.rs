//! 重命名：文件行里的名字原地换成输入框（同 Finder、标签改名），先选中扩展名前面那段。
//! ↵ 确认、Esc 取消、点到别处也算确认。入口是右键菜单的「重命名」和选中一项后按 ↵（`files-rename`，同 Finder）。
//! 改名的规则（选哪段、什么名字不行）在 `files::rename`，同名不覆盖由后端保证。

use super::super::rename::{rename_target, stem_end};
use super::*;

/// 输入框所在的上下文：Esc 在这里是取消改名，不是清选择。
pub(super) const RENAME_CONTEXT: &str = "FileRename";

/// 正在原地改名的那一项。会话在开始时就记下：等着确认的工夫切走了标签，也改在原来那台服务器上。
pub(super) struct RenameState {
    pane: PaneKind,
    path: String,
    is_directory: bool,
    session: Option<FileSession>,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl FileManager {
    pub(super) fn start_rename(&mut self, kind: PaneKind, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        if entry.is_parent() || (kind == PaneKind::Remote && self.session.is_none()) {
            return;
        }
        // 另一项还在改：先按确认收掉
        self.finish_rename(true, window, cx);
        self.menu = None;
        self.pane_mut(kind).selected = HashSet::from([entry.path.clone()]);
        self.pane_mut(kind.other()).selected.clear();
        let input = cx.new(|cx| InputState::new(window, cx));
        let stem = stem_end(&entry.name, entry.is_directory);
        input.update(cx, |input, cx| {
            input.set_value(entry.name.clone(), window, cx);
            input.set_selected_range(0..stem, cx);
            input.focus(window, cx);
        });
        // 失焦 = 确认（点了别的行、地址栏，或切走了）。等这一轮事件处理完再收：焦点已经去了别处，不抢回来。
        // ↵ 不看输入框的 PressEnter：走自己绑的 ConfirmRename（见 `key_bindings`）
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            if let InputEvent::Blur = event {
                this.finish_rename_later(input, window, cx);
            }
        });
        self.rename = Some(RenameState {
            pane: kind,
            path: entry.path.clone(),
            is_directory: entry.is_directory,
            session: self.session.clone(),
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// ↵：选中的正好一项就改它的名字（同 Finder）；没选或选了好几项不管。
    pub(super) fn rename_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.focused_pane(window);
        match self.selected_items(kind).as_slice() {
            [item] => {
                let item = item.clone();
                self.start_rename(kind, &item, window, cx);
            }
            _ => cx.propagate(),
        }
    }

    /// 收掉改名框；`commit` 时真的去改。回车 / Esc 时焦点还在框里，交回这一栏的列表，接着用键盘。
    pub(super) fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else { return };
        if rename.input.read(cx).focus_handle(cx).is_focused(window) {
            let focus = self.pane(rename.pane).list_focus.clone();
            window.focus(&focus, cx);
        }
        cx.notify();
        if !commit {
            return;
        }
        let value = rename.input.read(cx).value().to_string();
        match rename_target(&rename.path, rename.pane, &value) {
            Ok(None) => {}
            Ok(Some(target)) => self.rename_path(rename, target, window, cx),
            Err(error) => prompts::alert_later(window, cx, Options::new(tr!("无法重命名")).message(error.message())),
        }
    }

    fn finish_rename_later(&mut self, input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        let input = input.clone();
        cx.defer_in(window, move |this, window, cx| {
            if this.rename.as_ref().is_some_and(|rename| rename.input == input) {
                this.finish_rename(true, window, cx);
            }
        });
    }

    /// 收尾不能只靠失焦事件：窗口不在前台时 GPUI 不发 Focus / Blur。每次画的时候看一眼，框已经没焦点了就按确认收掉。
    /// 那一行不在列表里了（目录换了、文件被删了）：框画不出来，焦点落空，按键谁也收不到——取消，焦点交回列表。
    pub(super) fn settle_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = &self.rename else { return };
        if !self.pane(rename.pane).files.iter().any(|file| file.path == rename.path) {
            self.finish_rename(false, window, cx);
        } else if !rename.input.read(cx).focus_handle(cx).is_focused(window) {
            let input = rename.input.clone();
            self.finish_rename_later(&input, window, cx);
        }
    }

    fn rename_path(&mut self, rename: RenameState, to: String, window: &mut Window, cx: &mut Context<Self>) {
        let RenameState { pane: kind, path: from, is_directory, session, .. } = rename;
        let session_id = session.as_ref().map(|s| s.session_id.clone());
        let (source, target) = (from.clone(), to.clone());
        let remote = match (kind, session_id.clone()) {
            (PaneKind::Remote, None) => return,
            (PaneKind::Remote, Some(sid)) => Some(sid),
            (PaneKind::Local, _) => None,
        };
        let renamed = backend::spawn(async move {
            match remote {
                Some(sid) => sftp::file_rename(&backend::get().state, sid, source, target).await,
                None => local_fs::file_rename_local(source, target).await,
            }
        });
        // 收藏跟着改：远程的按连接，本地栏就是这台 Mac，和本机标签共用一份
        let scope = match kind {
            PaneKind::Remote => session.map(|s| s.connection_id),
            PaneKind::Local => Some(crate::workspace::LOCAL_SCOPE.to_string()),
        };
        cx.spawn_in(window, async move |this, cx| {
            if let Outcome::Failed(error) = Outcome::from_result(renamed.await) {
                alert(cx, tr!("重命名失败"), error);
                return;
            }
            if is_directory
                && let Some(scope) = &scope
                && let Err(error) = config_store::move_bookmarks(scope, &from, &to)
            {
                tracing::warn!("move bookmarks: {error}");
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if is_directory {
                    this.reload_bookmarks();
                }
                // 等着改名的工夫切走了会话：远程栏现在列的是别的服务器，不动它
                if kind == PaneKind::Remote && this.session_id() != session_id {
                    return;
                }
                // 改完选中新名字（同一个目录里刷新不清选择）
                let pane = this.pane_mut(kind);
                if pane.selected.remove(&from) {
                    pane.selected.insert(to);
                }
                this.invalidate(kind);
                this.refresh_list(kind, window, cx);
            });
        })
        .detach();
    }

    /// 正在改名的那一行，名字的位置换成输入框。
    pub(super) fn rename_field(&self, kind: PaneKind, path: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rename = self.rename.as_ref().filter(|rename| rename.pane == kind && rename.path == path)?;
        Some(
            div()
                .id("rename-field")
                .key_context(RENAME_CONTEXT)
                .flex_1()
                .min_w_0()
                .h(px(24.))
                .px(px(6.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(theme::color(ui::PRIMARY))
                .bg(theme::color(ui::CARD))
                .text_color(theme::color(ui::FOREGROUND))
                // 在框里按下是放光标、选字：不选行、不拖行、不弹文件的右键菜单，焦点也不交给列表
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_action(cx.listener(|this, _: &ConfirmRename, window, cx| this.finish_rename(true, window, cx)))
                .on_action(cx.listener(|this, _: &CancelRename, window, cx| this.finish_rename(false, window, cx)))
                .child(Input::new(&rename.input))
                .into_any_element(),
        )
    }
}

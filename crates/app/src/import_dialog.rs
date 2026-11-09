//! 从 `~/.ssh/config` 导入主机：列出 config 里的 Host，勾选后存成连接（分组「ssh config」）。
//! 写了 IdentityFile 的导入为「私钥 · 指定文件」，没写的导入为「自动」（免密）。已有的同一台主机不重复导入。

use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use sshl_core::config_store::{self, AuthKind, SaveConnectionInput, StoredConnection};
use sshl_core::ssh_config::{self, ImportCandidate};

use crate::backend;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

pub enum ImportDialogEvent {
    Close,
    /// 导入了几条（工作区刷新列表）。
    Imported(usize),
}

/// 导入后放进的分组。
pub const IMPORT_GROUP: &str = "ssh config";

struct Row {
    candidate: ImportCandidate,
    /// 同一台主机（主机 + 端口 + 用户）已经有连接了。
    exists: bool,
    checked: bool,
}

pub struct ImportDialog {
    rows: Option<Vec<Row>>,
    error: Option<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<ImportDialogEvent> for ImportDialog {}

impl Focusable for ImportDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

/// 已有连接里有没有同一台主机（主机名不分大小写）。
pub fn already_saved(candidate: &ImportCandidate, saved: &[StoredConnection]) -> bool {
    saved.iter().any(|c| c.host.eq_ignore_ascii_case(&candidate.host) && c.port == candidate.port && c.username == candidate.user)
}

/// 导入项 → 要保存的连接。
pub fn to_save_input(candidate: &ImportCandidate) -> SaveConnectionInput {
    SaveConnectionInput {
        id: None,
        name: candidate.alias.clone(),
        host: candidate.host.clone(),
        port: candidate.port,
        username: candidate.user.clone(),
        password: None,
        private_key: candidate.identity.clone(),
        passphrase: None,
        group: IMPORT_GROUP.into(),
        auth: if candidate.identity.is_some() { AuthKind::Key } else { AuthKind::Auto },
    }
}

impl ImportDialog {
    pub fn new(saved: Vec<StoredConnection>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let candidates = backend::spawn(ssh_config::candidates());
        cx.spawn(async move |this, cx| {
            let candidates = candidates.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.rows = Some(
                    candidates
                        .into_iter()
                        .map(|candidate| {
                            let exists = already_saved(&candidate, &saved);
                            Row { candidate, exists, checked: !exists }
                        })
                        .collect(),
                );
                cx.notify();
            });
        })
        .detach();
        Self { rows: None, error: None, focus }
    }

    fn checked(&self) -> usize {
        self.rows.iter().flatten().filter(|r| r.checked && !r.exists).count()
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let events = &backend::get().events;
        let mut imported = 0;
        for row in self.rows.iter().flatten().filter(|r| r.checked && !r.exists) {
            match config_store::save_connection(events, to_save_input(&row.candidate)) {
                Ok(_) => imported += 1,
                Err(error) => {
                    self.error = Some(format!("导入 {} 失败：{error}", row.candidate.alias).into());
                    cx.notify();
                    return;
                }
            }
        }
        cx.emit(ImportDialogEvent::Imported(imported));
    }

    fn row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &row.candidate;
        let target = if c.port == 22 { format!("{}@{}", c.user, c.host) } else { format!("{}@{}:{}", c.user, c.host, c.port) };
        let key: SharedString = match &c.identity {
            Some(path) => path.rsplit('/').next().unwrap_or(path).to_string().into(),
            None => "自动".into(),
        };
        div()
            .id(("import-row", index))
            .role(Role::CheckBox)
            .aria_label(SharedString::from(c.alias.clone()))
            .px_2()
            .py(px(7.))
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(8.))
            .when(!row.exists, |d| {
                d.cursor_pointer().hover(|s| s.bg(theme::color(ui::MUTED))).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(row) = this.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
                        row.checked = !row.checked;
                        cx.notify();
                    }
                }))
            })
            .when(row.exists, |d| d.opacity(0.5))
            .child(kit::checkbox(("import-check", index), row.checked && !row.exists))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(c.alias.clone()))
                    .child(div().truncate().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(target)),
            )
            .child(div().flex_shrink_0().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(if row.exists { "已存在".into() } else { key }))
    }
}

impl Render for ImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.checked();
        let list: gpui::AnyElement = match &self.rows {
            None => div().py_6().text_sm().text_color(theme::color(ui::MUTED_FOREGROUND)).child("正在读取 ~/.ssh/config…").into_any_element(),
            Some(rows) if rows.is_empty() => {
                div().py_6().text_sm().text_color(theme::color(ui::MUTED_FOREGROUND)).child("~/.ssh/config 里没有找到主机。").into_any_element()
            }
            Some(rows) => {
                let items: Vec<gpui::AnyElement> = rows.iter().enumerate().map(|(i, row)| self.row(i, row, cx).into_any_element()).collect();
                div().id("import-list").max_h(px(380.)).overflow_y_scroll().flex().flex_col().gap_1().children(items).into_any_element()
            }
        };
        let body = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(ImportDialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("从 ~/.ssh/config 导入"))
                    .child(list)
                    .when_some(self.error.clone(), |d, error| d.child(div().text_sm().text_color(theme::color(ui::DESTRUCTIVE)).child(error))),
            )
            .child(
                kit::modal_footer()
                    .child(kit::labeled_button("import-cancel", Variant::Outline, None, "取消").on_click(cx.listener(|_, _, _, cx| cx.emit(ImportDialogEvent::Close))))
                    .child(
                        kit::labeled_button("import-submit", Variant::Primary, None, if checked > 0 { format!("导入 {checked} 个") } else { "导入".into() })
                            .when(checked == 0, |b| b.opacity(0.5).cursor_default())
                            .when(checked > 0, |b| b.on_click(cx.listener(|this, _, _, cx| this.import(cx)))),
                    ),
            );
        kit::modal("import-dialog", 520., body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(identity: Option<&str>) -> ImportCandidate {
        ImportCandidate { alias: "web".into(), host: "Web.Example".into(), port: 2200, user: "deploy".into(), identity: identity.map(Into::into) }
    }

    #[test]
    fn imports_with_the_configured_key_or_automatic_login() {
        let keyed = to_save_input(&candidate(Some("~/.ssh/work_key")));
        assert_eq!((keyed.auth, keyed.private_key.as_deref(), keyed.name.as_str()), (AuthKind::Key, Some("~/.ssh/work_key"), "web"));
        assert_eq!(keyed.group, IMPORT_GROUP);
        let auto = to_save_input(&candidate(None));
        assert_eq!((auto.auth, auto.private_key), (AuthKind::Auto, None));
    }

    #[test]
    fn the_same_host_is_not_imported_twice() {
        let saved = StoredConnection {
            id: "1".into(),
            name: "我的".into(),
            host: "web.example".into(),
            port: 2200,
            username: "deploy".into(),
            private_key: None,
            has_password: false,
            has_passphrase: false,
            group: String::new(),
            auth: AuthKind::Password,
        };
        assert!(already_saved(&candidate(None), std::slice::from_ref(&saved)));
        assert!(!already_saved(&candidate(None), &[StoredConnection { port: 22, ..saved.clone() }]));
        assert!(!already_saved(&candidate(None), &[StoredConnection { username: "root".into(), ..saved }]));
    }
}

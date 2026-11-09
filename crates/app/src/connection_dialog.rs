//! 新建 / 编辑连接（旧 ConnectionDialog.tsx）。

use std::path::Path;

use gpui::{
    AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{InputEvent, InputState};
use sshl_core::config_store::{AuthKind, StoredConnection};

use crate::assets::Icon;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

/// 指定私钥文件时的默认值（同 OpenSSH）。
const DEFAULT_KEY: &str = "~/.ssh/id_rsa";

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionForm {
    pub name: String,
    pub group: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthKind,
    pub password: String,
    pub private_key: String,
    pub passphrase: String,
    pub save_password: bool,
}

pub enum ConnectionDialogEvent {
    Cancel,
    /// 新建：先连接，连上再保存。
    Connect(ConnectionForm),
    /// 编辑：直接保存。
    Save { id: String, form: ConnectionForm },
    /// 新建时点了「从 ~/.ssh/config 导入」。
    Import,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Group,
    Host,
    Port,
    Username,
    Password,
    PrivateKey,
    Passphrase,
}

pub struct ConnectionDialog {
    editing: Option<StoredConnection>,
    inputs: Vec<(Field, Entity<InputState>)>,
    auth: AuthKind,
    save_password: bool,
    focused: Option<Field>,
    error: Option<SharedString>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectionDialogEvent> for ConnectionDialog {}

impl Focusable for ConnectionDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ConnectionDialog {
    pub fn new(editing: Option<StoredConnection>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = |f: &dyn Fn(&StoredConnection) -> String| editing.as_ref().map(f).unwrap_or_default();
        let fields = [
            (Field::Name, value(&|c| c.name.clone()), "我的服务器", false),
            (Field::Group, value(&|c| c.group.clone()), "例如：生产环境", false),
            (Field::Host, value(&|c| c.host.clone()), "example.com 或 192.168.1.1", false),
            (Field::Port, editing.as_ref().map_or("22".into(), |c| c.port.to_string()), "", false),
            (Field::Username, value(&|c| c.username.clone()), "root", false),
            (Field::Password, String::new(), "密码", true),
            (Field::PrivateKey, value(&|c| c.private_key.clone().unwrap_or_default()), DEFAULT_KEY, false),
            (Field::Passphrase, String::new(), "私钥密码（可选）", true),
        ];
        let mut inputs = Vec::new();
        let mut subscriptions = Vec::new();
        for (field, initial, placeholder, masked) in fields {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(masked).default_value(initial));
            subscriptions.push(cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                InputEvent::Focus => {
                    this.focused = Some(field);
                    cx.notify();
                }
                InputEvent::Blur => {
                    if this.focused == Some(field) {
                        this.focused = None;
                    }
                    cx.notify();
                }
                InputEvent::Change => {
                    this.error = None;
                    cx.notify();
                }
            }));
            inputs.push((field, input));
        }
        let auth = editing.as_ref().map_or(AuthKind::Password, |c| c.auth);
        let this = Self {
            // 编辑时密码从不回显（密文只在后端）；不勾「保存密码」就不改已存的密码
            save_password: editing.is_none(),
            editing,
            inputs,
            auth,
            focused: None,
            error: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        let name = this.input(Field::Name).read(cx).focus_handle(cx);
        window.focus(&name, cx);
        this
    }

    fn input(&self, field: Field) -> &Entity<InputState> {
        &self.inputs.iter().find(|(f, _)| *f == field).expect("field").1
    }

    fn text(&self, field: Field, cx: &gpui::App) -> String {
        self.input(field).read(cx).value().to_string()
    }

    fn form(&self, cx: &gpui::App) -> Result<ConnectionForm, SharedString> {
        let name = self.text(Field::Name, cx);
        let host = self.text(Field::Host, cx);
        let username = self.text(Field::Username, cx);
        if name.trim().is_empty() {
            return Err("请填写名称".into());
        }
        if host.trim().is_empty() {
            return Err("请填写主机".into());
        }
        if username.trim().is_empty() {
            return Err("请填写用户名".into());
        }
        let port = match self.text(Field::Port, cx).trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => return Err("端口应为 1–65535".into()),
        };
        let private_key = self.text(Field::PrivateKey, cx);
        if self.auth == AuthKind::Key && private_key.trim().is_empty() {
            return Err("请选择私钥文件".into());
        }
        Ok(ConnectionForm {
            name,
            group: self.text(Field::Group, cx),
            host,
            port,
            username,
            auth: self.auth,
            password: self.text(Field::Password, cx),
            private_key,
            passphrase: self.text(Field::Passphrase, cx),
            save_password: self.save_password,
        })
    }

    fn submit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        match self.form(cx) {
            Ok(form) => match &self.editing {
                Some(conn) => cx.emit(ConnectionDialogEvent::Save { id: conn.id.clone(), form }),
                None => cx.emit(ConnectionDialogEvent::Connect(form)),
            },
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    /// 认证方式只分密码 / 私钥；私钥里再选「自动」（免密）或具体的私钥文件。
    fn set_use_key(&mut self, use_key: bool, cx: &mut Context<Self>) {
        self.auth = switch_auth(self.auth, use_key, !self.text(Field::PrivateKey, cx).trim().is_empty());
        cx.notify();
    }

    fn set_key_choice(&mut self, auth: AuthKind, window: &mut Window, cx: &mut Context<Self>) {
        self.auth = auth;
        // 指定文件：没填过就先填 OpenSSH 的默认私钥 ~/.ssh/id_rsa 并全选，光标进路径框，
        // 直接连或马上改成别的（~/.ssh 是隐藏目录，文件选择框里默认看不到）
        if auth == AuthKind::Key {
            let input = self.input(Field::PrivateKey).clone();
            let empty = input.read(cx).value().trim().is_empty();
            input.update(cx, |input, cx| {
                if empty {
                    input.set_value(DEFAULT_KEY, window, cx);
                }
                input.select_all(window, cx);
            });
            let focus = input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn set_key(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.auth = AuthKind::Key;
        // 家目录下的存成 ~/...（后端认）：路径框里短，换台机器也能用
        let path = match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        };
        self.input(Field::PrivateKey).clone().update(cx, |input, cx| input.set_value(path, window, cx));
        cx.notify();
    }

    /// 私钥的选法：自动（免密）或指定文件。
    fn key_choices(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let chip = |id: &'static str, choice: AuthKind, icon: Icon, label: &'static str, aria: &'static str| {
            kit::button(id, if self.auth == choice { Variant::Primary } else { Variant::Outline })
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .text_size(px(12.))
                .aria_label(aria)
                .child(kit::icon_sized(icon, 13.))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.set_key_choice(choice, window, cx)))
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(chip("ssh-key-auto", AuthKind::Auto, Icon::Zap, "自动", "自动选择私钥（免密）"))
            .child(chip("ssh-key-file", AuthKind::Key, Icon::FileText, "指定文件", "指定私钥文件"))
    }

    fn browse_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, window, cx| this.set_key(&path, window, cx));
        })
        .detach();
    }

    fn field(&self, label: &'static str, hint: Option<&'static str>, field: Field) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_1()
                    .child(kit::field_label(label))
                    .when_some(hint, |d, hint| d.child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(hint))),
            )
            .child(kit::text_field(self.input(field), self.focused == Some(field)))
    }
}

/// 在「密码 / 私钥」之间切换后的认证方式。切到私钥时还没填过私钥路径就用自动（免密：按 ~/.ssh/config、
/// SSH Agent、默认私钥依次试），填过就用那个文件；已经在私钥里（自动或文件）就不变。
fn switch_auth(current: AuthKind, use_key: bool, has_key_path: bool) -> AuthKind {
    match (use_key, current) {
        (false, _) => AuthKind::Password,
        (true, AuthKind::Password) if has_key_path => AuthKind::Key,
        (true, AuthKind::Password) => AuthKind::Auto,
        (true, current) => current,
    }
}

impl Render for ConnectionDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.editing.is_some();
        let this = cx.entity().downgrade();
        let body = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(ConnectionDialogEvent::Cancel);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(if editing { "编辑连接" } else { "新建连接" }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().flex().gap_3().child(self.field("名称", None, Field::Name)).child(self.field("分组", Some("（可选）"), Field::Group)))
                            .child(div().flex().gap_3().child(self.field("主机", None, Field::Host)).child(div().w(px(96.)).child(self.field("端口", None, Field::Port))))
                            .child(self.field("用户名", None, Field::Username))
                            .child(
                                div().flex().flex_col().gap(px(6.)).child(kit::field_label("认证方式")).child(kit::segmented(
                                    "auth-type",
                                    &[(false, Some(Icon::Lock), "密码".into()), (true, Some(Icon::KeyRound), "私钥".into())],
                                    self.auth != AuthKind::Password,
                                    move |use_key, _, cx| {
                                        let _ = this.update(cx, |this, cx| this.set_use_key(use_key, cx));
                                    },
                                )),
                            )
                            .map(|d| match self.auth {
                                AuthKind::Password => d.child(kit::text_field(self.input(Field::Password), self.focused == Some(Field::Password))),
                                auth => d
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(8.))
                                            .child(self.key_choices(cx))
                                            .map(|d| {
                                                if auth == AuthKind::Auto {
                                                    d.child(div().text_sm().text_color(theme::color(ui::MUTED_FOREGROUND)).child(
                                                        "依次尝试 SSH Agent 和 ~/.ssh 里的私钥，遵循 ~/.ssh/config",
                                                    ))
                                                } else {
                                                    d.child(
                                                        div()
                                                            .flex()
                                                            .gap_2()
                                                            .child(div().flex_1().min_w_0().child(kit::text_field(self.input(Field::PrivateKey), self.focused == Some(Field::PrivateKey))))
                                                            .child(
                                                                kit::labeled_button("browse-key", Variant::Outline, Some(Icon::FolderOpen), "浏览")
                                                                    .on_click(cx.listener(|this, _, window, cx| this.browse_key(window, cx))),
                                                            ),
                                                    )
                                                }
                                            }),
                                    )
                                    .when(auth == AuthKind::Key, |d| d.child(kit::text_field(self.input(Field::Passphrase), self.focused == Some(Field::Passphrase)))),
                            })
                            .when(self.auth != AuthKind::Auto, |d| d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(kit::checkbox("save-password", self.save_password).on_click(cx.listener(|this, _, _, cx| {
                                        this.save_password = !this.save_password;
                                        cx.notify();
                                    })))
                                    .child(
                                        div()
                                            .id("save-password-label")
                                            .cursor_pointer()
                                            .child(if self.auth == AuthKind::Key { "保存私钥密码" } else { "保存密码" })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.save_password = !this.save_password;
                                                cx.notify();
                                            })),
                                    ),
                            ))
                            .when_some(self.error.clone(), |d, error| d.child(div().text_sm().text_color(theme::color(ui::DESTRUCTIVE)).child(error))),
                    ),
            )
            .child(
                kit::modal_footer()
                    .when(!editing, |f| {
                        f.child(
                            div()
                                .id("connection-import")
                                .mr_auto()
                                .flex()
                                .items_center()
                                .role(gpui::Role::Button)
                                .aria_label("从 ~/.ssh/config 导入")
                                .text_sm()
                                .text_color(theme::color(ui::PRIMARY))
                                .cursor_pointer()
                                .child("从 ~/.ssh/config 导入")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionDialogEvent::Import))),
                        )
                    })
                    .child(
                        kit::labeled_button("connection-cancel", Variant::Outline, None, "取消")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionDialogEvent::Cancel))),
                    )
                    .child(
                        kit::labeled_button("connection-submit", Variant::Primary, None, if editing { "保存" } else { "连接" })
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            );
        kit::modal("connection-dialog", 520., body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_mode_defaults_to_automatic_unless_a_file_was_chosen() {
        assert_eq!(switch_auth(AuthKind::Password, true, false), AuthKind::Auto);
        assert_eq!(switch_auth(AuthKind::Password, true, true), AuthKind::Key);
        // 已经在私钥里：点「私钥」不改选法
        assert_eq!(switch_auth(AuthKind::Key, true, true), AuthKind::Key);
        assert_eq!(switch_auth(AuthKind::Auto, true, true), AuthKind::Auto);
        assert_eq!(switch_auth(AuthKind::Auto, false, false), AuthKind::Password);
        assert_eq!(switch_auth(AuthKind::Key, false, true), AuthKind::Password);
    }
}

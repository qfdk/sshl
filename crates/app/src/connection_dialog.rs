//! 新建 / 编辑连接（旧 ConnectionDialog.tsx）。

use std::path::{Path, PathBuf};

use gpui::{
    AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{InputEvent, InputState};
use sshl_core::config_store::{AuthKind, StoredConnection};

use crate::assets::Icon;
use crate::ssh_keys;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

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
    /// ~/.ssh 下找到的私钥；第一次用到私钥方式时才扫描。
    keys: Option<Vec<PathBuf>>,
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
            (Field::Password, String::new(), "", true),
            (Field::PrivateKey, value(&|c| c.private_key.clone().unwrap_or_default()), "", false),
            (Field::Passphrase, String::new(), "", true),
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
            keys: (auth == AuthKind::Key).then(ssh_keys::discover),
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

    fn set_auth(&mut self, auth: AuthKind, window: &mut Window, cx: &mut Context<Self>) {
        self.auth = auth;
        if auth == AuthKind::Key {
            // 切到私钥且还没填：默认用 ~/.ssh 里找到的第一把（id_ed25519 优先），一把都没有就留空
            let first = self.keys.get_or_insert_with(ssh_keys::discover).first().cloned();
            if let Some(first) = first
                && self.text(Field::PrivateKey, cx).is_empty()
            {
                self.set_key(&first, window, cx);
            }
        }
        cx.notify();
    }

    fn set_key(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let path = path.to_string_lossy().to_string();
        self.input(Field::PrivateKey).clone().update(cx, |input, cx| input.set_value(path, window, cx));
        cx.notify();
    }

    /// 找到的私钥，点一下填进路径框；当前选中的那把高亮。
    fn key_choices(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let keys = self.keys.as_ref().filter(|keys| !keys.is_empty())?;
        let current = self.text(Field::PrivateKey, cx);
        let home = dirs::home_dir();
        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child("~/.ssh 中的私钥"))
                .children(keys.iter().enumerate().map(|(i, path)| {
                    let shown: SharedString = ssh_keys::display(path, home.as_deref()).into();
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let selected = current == path.to_string_lossy();
                    let path = path.clone();
                    let tip = shown.clone();
                    kit::button(("ssh-key", i), if selected { Variant::Primary } else { Variant::Outline })
                        .h(px(28.))
                        .px(px(10.))
                        .gap(px(6.))
                        .text_size(px(12.))
                        .aria_label(SharedString::from(format!("使用私钥 {shown}")))
                        .tooltip(move |window, cx| kit::tooltip(tip.clone(), window, cx))
                        .child(kit::icon_sized(Icon::KeyRound, 13.))
                        .child(name)
                        .on_click(cx.listener(move |this, _, window, cx| this.set_key(&path, window, cx)))
                })),
        )
    }

    fn browse_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                let path = path.to_string_lossy().to_string();
                this.input(Field::PrivateKey).clone().update(cx, |input, cx| input.set_value(path, window, cx));
            });
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
                    .child(
                        div()
                            .relative()
                            .pr_8()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(if editing { "编辑连接" } else { "新建连接" }))
                            .child(div().text_sm().text_color(theme::color(ui::MUTED_FOREGROUND)).child(if editing {
                                "修改服务器信息后点保存。"
                            } else {
                                "填写服务器信息，连接成功后自动保存到列表。"
                            }))
                            .child(
                                div().absolute().top(px(-6.)).right(px(-6.)).child(
                                    kit::icon_button("connection-close", Variant::Ghost, Icon::X, "关闭")
                                        .w(px(32.))
                                        .h(px(32.))
                                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionDialogEvent::Cancel))),
                                ),
                            ),
                    )
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
                                    &[
                                        (AuthKind::Password, Some(Icon::Lock), "密码".into()),
                                        (AuthKind::Key, Some(Icon::KeyRound), "私钥".into()),
                                        (AuthKind::Auto, Some(Icon::Zap), "免密".into()),
                                    ],
                                    self.auth,
                                    move |auth, window, cx| {
                                        let _ = this.update(cx, |this, cx| this.set_auth(auth, window, cx));
                                    },
                                )),
                            )
                            .map(|d| match self.auth {
                                AuthKind::Auto => d.child(
                                    div()
                                        .text_sm()
                                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                                        .child("依次尝试服务器免认证、SSH Agent 和私钥文件，私钥按 ~/.ssh/config 为这台主机配的来，没配就用 ~/.ssh 里的默认私钥。"),
                                ),
                                AuthKind::Key => {
                                    d.child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(6.))
                                            .child(kit::field_label("私钥文件"))
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .child(div().flex_1().min_w_0().child(kit::text_field(self.input(Field::PrivateKey), self.focused == Some(Field::PrivateKey))))
                                                    .child(
                                                        kit::labeled_button("browse-key", Variant::Outline, Some(Icon::FolderOpen), "浏览")
                                                            .on_click(cx.listener(|this, _, window, cx| this.browse_key(window, cx))),
                                                    ),
                                            )
                                            .when_some(self.key_choices(cx), |d, choices| d.child(choices)),
                                    )
                                    .child(self.field("私钥密码", Some("（可选）"), Field::Passphrase))
                                }
                                AuthKind::Password => d.child(self.field("密码", None, Field::Password)),
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
                                            .child("保存密码")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.save_password = !this.save_password;
                                                cx.notify();
                                            })),
                                    )
                                    .child(div().text_sm().text_color(theme::color(ui::MUTED_FOREGROUND)).child("· AES 加密保存在本机")),
                            ))
                            .when_some(self.error.clone(), |d, error| d.child(div().text_sm().text_color(theme::color(ui::DESTRUCTIVE)).child(error))),
                    ),
            )
            .child(
                kit::modal_footer()
                    .child(
                        kit::labeled_button("connection-cancel", Variant::Outline, Some(Icon::X), "取消")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionDialogEvent::Cancel))),
                    )
                    .child(
                        kit::labeled_button("connection-submit", Variant::Primary, Some(if editing { Icon::Save } else { Icon::ArrowRight }), if editing { "保存" } else { "连接" })
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            );
        kit::modal("connection-dialog", 520., body)
    }
}

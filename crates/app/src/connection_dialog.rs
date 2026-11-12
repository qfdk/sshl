//! 新建 / 编辑连接（旧 ConnectionDialog.tsx）。

use std::path::Path;

use gpui::{
    AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, KeyDownEvent, MouseButton, ParentElement, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, deferred, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{InputEvent, InputState, Position};
use sshl_core::config_store::{AuthKind, StoredConnection};

use crate::assets::Icon;
use crate::groups;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

actions!(connection_dialog, [GroupNext, GroupPrev, GroupDismiss]);

const GROUP_CONTEXT: &str = "ConnectionGroup";

/// 要晚于 gpui-base 绑定（同一节点上后绑的优先）：分组框里的上下 / Esc 先给下拉。
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("down", GroupNext, Some("ConnectionGroup > Input")),
        KeyBinding::new("up", GroupPrev, Some("ConnectionGroup > Input")),
        KeyBinding::new("escape", GroupDismiss, Some("ConnectionGroup > Input")),
    ]
}

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
    /// 已有的分组（顺序同侧栏），分组框的下拉里选。
    groups: Vec<String>,
    group_open: bool,
    /// 用户正在分组框里敲字：下拉只留匹配的；刚点开 / 刚选完时全部列出。
    group_filtering: bool,
    group_highlight: Option<usize>,
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
    pub fn new(editing: Option<StoredConnection>, groups: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = |f: &dyn Fn(&StoredConnection) -> String| editing.as_ref().map(f).unwrap_or_default();
        let fields = [
            (Field::Name, value(&|c| c.name.clone()), tr!("不填就用主机地址"), false),
            (Field::Group, value(&|c| c.group.clone()), tr!("选择或输入新分组"), false),
            (Field::Host, value(&|c| c.host.clone()), tr!("example.com 或 192.168.1.1"), false),
            (Field::Port, editing.as_ref().map_or("22".into(), |c| c.port.to_string()), "", false),
            (Field::Username, value(&|c| c.username.clone()), "root", false),
            (Field::Password, String::new(), tr!("密码"), true),
            (Field::PrivateKey, value(&|c| c.private_key.clone().unwrap_or_default()), DEFAULT_KEY, false),
            (Field::Passphrase, String::new(), tr!("私钥密码（可选）"), true),
        ];
        let mut inputs = Vec::new();
        let mut subscriptions = Vec::new();
        for (field, initial, placeholder, masked) in fields {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(masked).default_value(initial));
            subscriptions.push(cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, window, cx| match event {
                // 分组下拉里用上下键选了一项：回车是选它，不是提交表单
                InputEvent::PressEnter { .. } => match this.group_highlight.filter(|_| field == Field::Group && this.group_open) {
                    Some(index) => this.pick_group(index, window, cx),
                    None => this.submit(window, cx),
                },
                InputEvent::Focus => {
                    this.focused = Some(field);
                    if field == Field::Group {
                        this.open_groups(cx);
                    } else {
                        this.close_groups(cx);
                    }
                    cx.notify();
                }
                InputEvent::Blur => {
                    if this.focused == Some(field) {
                        this.focused = None;
                    }
                    if field == Field::Group {
                        this.close_groups(cx);
                    }
                    cx.notify();
                }
                InputEvent::Change => {
                    this.error = None;
                    if field == Field::Group {
                        this.group_open = true;
                        this.group_filtering = true;
                        this.group_highlight = None;
                    }
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
            groups,
            group_open: false,
            group_filtering: false,
            group_highlight: None,
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
        let host = self.text(Field::Host, cx);
        let name = connection_name(&self.text(Field::Name, cx), &host);
        let username = self.text(Field::Username, cx);
        if host.trim().is_empty() {
            return Err(tr!("请填写主机").into());
        }
        if username.trim().is_empty() {
            return Err(tr!("请填写用户名").into());
        }
        let port = match self.text(Field::Port, cx).trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => return Err(tr!("端口应为 1–65535").into()),
        };
        let private_key = self.text(Field::PrivateKey, cx);
        if self.auth == AuthKind::Key && private_key.trim().is_empty() {
            return Err(tr!("请选择私钥文件").into());
        }
        Ok(ConnectionForm {
            name,
            group: groups::resolve_group(&self.groups, &self.text(Field::Group, cx)),
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

    fn group_choices(&self, cx: &gpui::App) -> groups::GroupChoices {
        groups::group_choices(&self.groups, &self.text(Field::Group, cx), self.group_filtering)
    }

    /// 下拉里有几行（已有分组 + 可能的「新建分组」）。
    fn group_rows(&self, cx: &gpui::App) -> usize {
        let choices = self.group_choices(cx);
        choices.existing.len() + usize::from(choices.create.is_some())
    }

    fn open_groups(&mut self, cx: &mut Context<Self>) {
        if !self.group_open {
            self.group_open = true;
            self.group_filtering = false;
            self.group_highlight = None;
            cx.notify();
        }
    }

    fn close_groups(&mut self, cx: &mut Context<Self>) {
        if self.group_open {
            self.group_open = false;
            self.group_highlight = None;
            cx.notify();
        }
    }

    /// 选下拉里的第 `index` 行：已有分组就填进框里（光标放末尾）；「新建分组」那行名字已经在框里，收起下拉就行。
    fn pick_group(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = self.group_choices(cx).existing.get(index).cloned() {
            self.input(Field::Group).clone().update(cx, |input, cx| {
                let end = name.encode_utf16().count() as u32;
                input.set_value(name, window, cx);
                input.set_cursor_position(Position::new(0, end), window, cx);
            });
        }
        self.group_filtering = false;
        self.close_groups(cx);
        cx.notify();
    }

    fn group_step(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.group_rows(cx);
        if count == 0 {
            cx.propagate();
            return;
        }
        let last = count - 1;
        // 收起着时按上下：先展开全部
        if !self.group_open {
            self.open_groups(cx);
        }
        self.group_highlight = Some(match (self.group_highlight, down) {
            (Some(i), true) if i < last => i + 1,
            (_, true) => 0,
            (Some(i), false) if i > 0 => i - 1,
            (_, false) => last,
        });
        cx.notify();
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
            .child(chip("ssh-key-auto", AuthKind::Auto, Icon::Zap, tr!("自动"), tr!("自动选择私钥（免密）")))
            .child(chip("ssh-key-file", AuthKind::Key, Icon::FileText, tr!("指定文件"), tr!("指定私钥文件")))
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
        self.labeled(label, hint, kit::text_field(self.input(field), self.focused == Some(field)))
    }

    fn labeled(&self, label: &'static str, hint: Option<&'static str>, control: impl IntoElement) -> gpui::Div {
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
            .child(control)
    }

    /// 分组：可以敲字的下拉。点进去列出已有分组，敲字过滤；敲的名字还没有就在列表里提示会新建。
    fn group_field(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let choices = if self.group_open { self.group_choices(cx) } else { groups::GroupChoices { existing: Vec::new(), create: None } };
        let current = groups::resolve_group(&self.groups, &self.text(Field::Group, cx));
        let highlight = self.group_highlight;
        let row = |index: usize| {
            div()
                .id(("connection-group-option", index))
                .role(Role::ListBoxOption)
                .px_2()
                .py(px(6.))
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(6.))
                .cursor_pointer()
                .when(highlight == Some(index), |d| d.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND)))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && this.group_highlight != Some(index) {
                        this.group_highlight = Some(index);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| this.pick_group(index, window, cx)))
        };
        let mut rows = Vec::new();
        for (index, name) in choices.existing.iter().enumerate() {
            rows.push(
                row(index)
                    .aria_label(name.clone())
                    .child(kit::icon_sized(Icon::Folder, 14.).text_color(theme::color(ui::PRIMARY)))
                    .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                    .when(*name == current, |d| d.child(kit::icon_sized(Icon::Check, 14.))),
            );
        }
        if let Some(name) = &choices.create {
            let label = tr!("新建分组「{}」", name);
            rows.push(
                row(choices.existing.len())
                    .aria_label(label.clone())
                    .child(kit::icon_sized(Icon::FolderPlus, 14.).text_color(theme::color(ui::PRIMARY)))
                    .child(div().flex_1().min_w_0().truncate().child(label)),
            );
        }
        let list = (!rows.is_empty()).then(|| {
            div()
                .id("connection-group-list")
                .role(Role::ListBox)
                .aria_label(tr!("分组"))
                .absolute()
                .left_0()
                .right_0()
                .top_full()
                .mt_1()
                .max_h(px(200.))
                .overflow_y_scroll()
                .p_1()
                .rounded(px(10.))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .bg(theme::color(ui::CARD))
                .shadow_md()
                .text_sm()
                .occlude()
                // 按在下拉上不让对话框把焦点从分组框拿走：一失焦下拉就收了，松开时点击落空
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .children(rows)
        });
        let control = div()
            .id("connection-group")
            .key_context(GROUP_CONTEXT)
            .relative()
            .on_action(cx.listener(|this, _: &GroupNext, _, cx| this.group_step(true, cx)))
            .on_action(cx.listener(|this, _: &GroupPrev, _, cx| this.group_step(false, cx)))
            .on_action(cx.listener(|this, _: &GroupDismiss, _, cx| {
                // 下拉开着：Esc 只收下拉；收起着才轮到关对话框
                if this.group_open && this.group_rows(cx) > 0 {
                    this.close_groups(cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(
                kit::text_field(self.input(Field::Group), self.focused == Some(Field::Group))
                    // 已经有焦点时再点一下也展开（选完 / Esc 收起之后）
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.open_groups(cx)))
                    .when(!self.groups.is_empty(), |f| f.child(kit::icon_sized(Icon::ChevronDown, 14.).text_color(theme::color(ui::MUTED_FOREGROUND)))),
            )
            // 延后到最上层画：不然被下面几行输入框压住
            .when_some(list, |d, list| d.child(deferred(list).with_priority(1)));
        self.labeled(tr!("分组"), Some(tr!("（可选）")), control)
    }
}

/// 连接的名称：可以不填，不填就用主机地址（主机必填）。
fn connection_name(name: &str, host: &str) -> String {
    let name = name.trim();
    if name.is_empty() { host.trim().to_string() } else { name.to_string() }
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
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(if editing { tr!("编辑连接") } else { tr!("新建连接") }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().flex().gap_3().child(self.field(tr!("名称"), Some(tr!("（可选）")), Field::Name)).child(self.group_field(cx)))
                            .child(div().flex().gap_3().child(self.field(tr!("主机"), None, Field::Host)).child(div().w(px(96.)).child(self.field(tr!("端口"), None, Field::Port))))
                            .child(self.field(tr!("用户名"), None, Field::Username))
                            .child(
                                div().flex().flex_col().gap(px(6.)).child(kit::field_label(tr!("认证方式"))).child(kit::segmented(
                                    "auth-type",
                                    &[(false, Some(Icon::Lock), tr!("密码").into()), (true, Some(Icon::KeyRound), tr!("私钥").into())],
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
                                                        tr!("依次尝试 SSH Agent 和 ~/.ssh 里的私钥，遵循 ~/.ssh/config"),
                                                    ))
                                                } else {
                                                    d.child(
                                                        div()
                                                            .flex()
                                                            .gap_2()
                                                            .child(div().flex_1().min_w_0().child(kit::text_field(self.input(Field::PrivateKey), self.focused == Some(Field::PrivateKey))))
                                                            .child(
                                                                kit::labeled_button("browse-key", Variant::Outline, Some(Icon::FolderOpen), tr!("浏览"))
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
                                            .child(if self.auth == AuthKind::Key { tr!("保存私钥密码") } else { tr!("保存密码") })
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
                                .aria_label(tr!("从 ~/.ssh/config 导入"))
                                .text_sm()
                                .text_color(theme::color(ui::PRIMARY))
                                .cursor_pointer()
                                .child(tr!("从 ~/.ssh/config 导入"))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionDialogEvent::Import))),
                        )
                    })
                    .child(
                        kit::labeled_button("connection-cancel", Variant::Outline, None, tr!("取消"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionDialogEvent::Cancel))),
                    )
                    .child(
                        kit::labeled_button("connection-submit", Variant::Primary, None, if editing { tr!("保存") } else { tr!("连接") })
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
    fn arrows_and_escape_reach_the_group_dropdown_before_the_input_box() {
        use gpui::{KeyContext, Keymap, Keystroke};
        gpui::actions!(test, [InputDown, InputEscape]);
        // 同 gpui-base：输入框自己的键先绑（main.rs 里 gpui_base::init 在前）
        let mut keymap = Keymap::new(vec![KeyBinding::new("down", InputDown, Some("Input")), KeyBinding::new("escape", InputEscape, Some("Input"))]);
        keymap.add_bindings(key_bindings());
        let first = |key: &'static str, stack: &[&str]| {
            let stack: Vec<KeyContext> = stack.iter().map(|c| KeyContext::parse(c).unwrap()).collect();
            let bindings = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack).0;
            bindings.first().map(|b| b.action().name().to_string())
        };
        let group = ["Root", "Workspace", GROUP_CONTEXT, "Input"];
        assert_eq!(first("down", &group).as_deref(), Some("connection_dialog::GroupNext"));
        assert_eq!(first("up", &group).as_deref(), Some("connection_dialog::GroupPrev"));
        assert_eq!(first("escape", &group).as_deref(), Some("connection_dialog::GroupDismiss"));
        // 对话框里别的输入框不受影响
        assert_eq!(first("escape", &["Root", "Workspace", "Input"]).as_deref(), Some("test::InputEscape"));
    }

    #[test]
    fn a_connection_without_a_name_is_named_after_its_host() {
        assert_eq!(connection_name("", " 10.0.1.11 "), "10.0.1.11");
        assert_eq!(connection_name("   ", "example.com"), "example.com");
        assert_eq!(connection_name(" web-01 ", "10.0.1.11"), "web-01");
    }

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

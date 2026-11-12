//! 应用内提示 / 确认 / 密码框（旧 lib/dialogs.ts 的原生版）。多个请求排队，一个一个显示；
//! 回车 = 确认（默认按钮），Esc = 取消；最后一个关掉后焦点还给弹出前的地方（多半是终端）。

use std::collections::VecDeque;

use futures::channel::oneshot;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, Global, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use gpui_base::input::{InputEvent, InputState};

use crate::assets::Icon;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

#[derive(Clone)]
pub struct Choice {
    pub value: String,
    pub label: SharedString,
}

pub struct Options {
    pub title: SharedString,
    pub message: Option<SharedString>,
    /// 单行补充信息（如文件名清单），超出省略。
    pub detail: Option<SharedString>,
    pub confirm_label: SharedString,
    pub cancel_label: SharedString,
    pub destructive: bool,
    /// 按钮上方的复选框（`confirm_with_check` 用），如「剩下的也这样处理」。
    pub checkbox: Option<SharedString>,
}

impl Options {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: None,
            detail: None,
            confirm_label: tr!("确定").into(),
            cancel_label: tr!("取消").into(),
            destructive: false,
            checkbox: None,
        }
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn confirm(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    pub fn cancel(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    pub fn checkbox(mut self, label: impl Into<SharedString>) -> Self {
        self.checkbox = Some(label.into());
        self
    }

}

pub enum PasswordAnswer {
    Password(String),
    /// 点了某个已保存的密码（只回传标识，密码不经过界面）。
    Choice(String),
}

enum Kind {
    Alert(oneshot::Sender<()>),
    Confirm(oneshot::Sender<bool>),
    /// 带复选框的确认：回（按了确认，勾了复选框）。
    Check { checked: bool, reply: oneshot::Sender<(bool, bool)> },
    Password { choices: Vec<Choice>, input: Entity<InputState>, reply: oneshot::Sender<Option<PasswordAnswer>> },
}

struct Request {
    options: Options,
    kind: Kind,
}

pub struct PromptHost {
    queue: VecDeque<Request>,
    focus: FocusHandle,
    /// 第一个提示弹出前的焦点，最后一个关掉后还回去。
    restore: Option<FocusHandle>,
    input_focused: bool,
    _subscriptions: Vec<Subscription>,
}

struct GlobalHost(Entity<PromptHost>);

impl Global for GlobalHost {}

pub fn init(cx: &mut App) -> Entity<PromptHost> {
    let host = cx.new(|cx| PromptHost {
        queue: VecDeque::new(),
        focus: cx.focus_handle(),
        restore: None,
        input_focused: false,
        _subscriptions: Vec::new(),
    });
    cx.set_global(GlobalHost(host.clone()));
    host
}

fn host(cx: &App) -> Entity<PromptHost> {
    cx.global::<GlobalHost>().0.clone()
}

/// 有提示框开着（这时 Esc 归它，不能顺手取消正在进行的连接）。
pub fn is_open(cx: &App) -> bool {
    !host(cx).read(cx).queue.is_empty()
}

pub fn alert_later(window: &mut Window, cx: &mut App, options: Options) {
    let (tx, _rx) = oneshot::channel();
    let options = Options { confirm_label: tr!("知道了").into(), ..options };
    push(window, cx, Request { options, kind: Kind::Alert(tx) });
}

pub fn confirm(window: &mut Window, cx: &mut App, options: Options) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    push(window, cx, Request { options, kind: Kind::Confirm(tx) });
    rx
}

/// 带复选框（`Options::checkbox`）的确认框，回（按了确认，勾了复选框）；Esc 等于点取消按钮。
pub fn confirm_with_check(window: &mut Window, cx: &mut App, options: Options) -> oneshot::Receiver<(bool, bool)> {
    let (reply, rx) = oneshot::channel();
    push(window, cx, Request { options, kind: Kind::Check { checked: false, reply } });
    rx
}

pub fn password(
    window: &mut Window,
    cx: &mut App,
    options: Options,
    placeholder: impl Into<SharedString>,
    choices: Vec<Choice>,
) -> oneshot::Receiver<Option<PasswordAnswer>> {
    let (reply, rx) = oneshot::channel();
    let placeholder = placeholder.into();
    let input = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(placeholder.clone()));
    push(window, cx, Request { options, kind: Kind::Password { choices, input, reply } });
    rx
}

fn push(window: &mut Window, cx: &mut App, request: Request) {
    let host = host(cx);
    host.update(cx, |host, cx| {
        if host.queue.is_empty() {
            host.restore = window.focused(cx);
        }
        host.queue.push_back(request);
        host.focus_head(window, cx);
        cx.notify();
    });
}

impl PromptHost {
    fn focus_head(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(head) = self.queue.front() else { return };
        match &head.kind {
            Kind::Password { input, .. } => {
                let input = input.clone();
                self._subscriptions = vec![cx.subscribe_in(&input, window, |host, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => host.settle(true, None, window, cx),
                    InputEvent::Focus => {
                        host.input_focused = true;
                        cx.notify();
                    }
                    InputEvent::Blur => {
                        host.input_focused = false;
                        cx.notify();
                    }
                    _ => {}
                })];
                let focus = input.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            _ => {
                self._subscriptions.clear();
                window.focus(&self.focus, cx);
            }
        }
    }

    fn settle(&mut self, ok: bool, choice: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(head) = self.queue.pop_front() else { return };
        match head.kind {
            Kind::Alert(tx) => {
                let _ = tx.send(());
            }
            Kind::Confirm(tx) => {
                let _ = tx.send(ok);
            }
            Kind::Check { checked, reply } => {
                let _ = reply.send((ok, checked));
            }
            Kind::Password { input, reply, .. } => {
                let answer = match (ok, choice) {
                    (false, _) => None,
                    (true, Some(choice)) => Some(PasswordAnswer::Choice(choice)),
                    (true, None) => Some(PasswordAnswer::Password(input.read(cx).value().to_string())),
                };
                let _ = reply.send(answer);
            }
        }
        if self.queue.is_empty() {
            self._subscriptions.clear();
            if let Some(restore) = self.restore.take() {
                window.focus(&restore, cx);
            }
        } else {
            self.focus_head(window, cx);
        }
        cx.notify();
    }

    fn toggle_check(&mut self, cx: &mut Context<Self>) {
        if let Some(Request { kind: Kind::Check { checked, .. }, .. }) = self.queue.front_mut() {
            *checked = !*checked;
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => {
                self.settle(false, None, window, cx);
                cx.stop_propagation();
            }
            "enter" if !matches!(self.queue.front().map(|r| &r.kind), Some(Kind::Password { .. })) => {
                self.settle(true, None, window, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Render for PromptHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(head) = self.queue.front() else {
            return div().into_any_element();
        };
        let options = &head.options;
        let is_alert = matches!(head.kind, Kind::Alert(_));
        let password = match &head.kind {
            Kind::Password { choices, input, .. } => Some((choices.clone(), input.clone())),
            _ => None,
        };
        let check = match &head.kind {
            Kind::Check { checked, .. } => options.checkbox.clone().map(|label| (label, *checked)),
            _ => None,
        };
        let body = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .min_w_0()
                            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(options.title.clone()))
                            .when_some(options.detail.clone(), |d, detail| {
                                d.child(div().text_size(px(14.)).text_color(theme::alpha(ui::FOREGROUND, 0.8)).truncate().child(detail))
                            })
                            .when_some(options.message.clone(), |d, message| {
                                d.child(div().text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(message))
                            }),
                    )
                    .when_some(password, |d, (choices, input)| {
                        d.when(!choices.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .child(div().text_size(px(12.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("已保存的密码")))
                                    .child(div().flex().flex_wrap().gap(px(6.)).children(choices.into_iter().enumerate().map(|(i, choice)| {
                                        let value = choice.value.clone();
                                        kit::button(("prompt-choice", i), Variant::Outline)
                                            .h(px(32.))
                                            .px(px(10.))
                                            .text_size(px(12.))
                                            .aria_label(choice.label.clone())
                                            .child(kit::icon_sized(Icon::KeyRound, 14.))
                                            .child(choice.label)
                                            .on_click(cx.listener(move |host, _, window, cx| host.settle(true, Some(value.clone()), window, cx)))
                                    }))),
                            )
                        })
                        .child(kit::text_field(&input, self.input_focused))
                    })
                    .when_some(check, |d, (label, checked)| {
                        d.child(
                            div()
                                .id("prompt-check")
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(13.))
                                .cursor_pointer()
                                // 方框自己也能点（辅助功能「按下」落在它上面），点了不再冒泡到整行，免得切换两次
                                .child(kit::checkbox("prompt-check-box", checked).aria_label(label.clone()).on_click(cx.listener(|host, _, _, cx| {
                                    cx.stop_propagation();
                                    host.toggle_check(cx);
                                })))
                                .child(label)
                                .on_click(cx.listener(|host, _, _, cx| host.toggle_check(cx))),
                        )
                    }),
            )
            .child(
                kit::modal_footer()
                    .when(!is_alert, |f| {
                        f.child(
                            kit::button("prompt-cancel", Variant::Outline)
                                .aria_label(options.cancel_label.clone())
                                .child(options.cancel_label.clone())
                                .on_click(cx.listener(|host, _, window, cx| host.settle(false, None, window, cx))),
                        )
                    })
                    .child(
                        kit::button("prompt-ok", if options.destructive { Variant::Destructive } else { Variant::Primary })
                            .aria_label(options.confirm_label.clone())
                            .child(options.confirm_label.clone())
                            .on_click(cx.listener(|host, _, window, cx| host.settle(true, None, window, cx))),
                    ),
            );
        kit::modal("prompt", 420., body).into_any_element()
    }
}

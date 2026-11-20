//! 应用内提示 / 确认 / 密码框（旧 lib/dialogs.ts 的原生版）。多个请求排队，一个一个显示；
//! 回车 = 确认（默认按钮），Esc = 取消；最后一个关掉后焦点还给弹出前的地方（多半是终端）。
//! 弹出时选中默认按钮（画焦点框）；Tab / ⇧Tab 在框里的按钮（和密码框、复选框）之间转、出不了框，空格或回车按下选中的那个（同系统对话框）。

use std::collections::VecDeque;

use futures::channel::oneshot;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, Global, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use gpui_base::FocusTrapElement;
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
    Password { choices: Vec<Choice>, choice_focus: Vec<FocusHandle>, input: Entity<InputState>, reply: oneshot::Sender<Option<PasswordAnswer>> },
}

/// Tab 选中的是哪个按钮。
enum Focused {
    Cancel,
    Ok,
    Check,
    Choice(String),
}

struct Request {
    options: Options,
    kind: Kind,
}

pub struct PromptHost {
    queue: VecDeque<Request>,
    /// 整个框：弹出时焦点在这里（回车 = 默认按钮），也是 Tab 出不去的那个范围。
    focus: FocusHandle,
    cancel_focus: FocusHandle,
    ok_focus: FocusHandle,
    check_focus: FocusHandle,
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
        cancel_focus: cx.focus_handle().tab_stop(true),
        ok_focus: cx.focus_handle().tab_stop(true),
        check_focus: cx.focus_handle().tab_stop(true),
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
    let choice_focus = choices.iter().map(|_| cx.focus_handle().tab_stop(true)).collect();
    push(window, cx, Request { options, kind: Kind::Password { choices, choice_focus, input, reply } });
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
            // 焦点直接放在默认按钮上（画焦点框）：一眼看出回车 / 空格按的是它，Tab 从它往下转
            _ => {
                self._subscriptions.clear();
                window.focus(&self.ok_focus, cx);
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

    fn focused(&self, window: &Window) -> Option<Focused> {
        if self.cancel_focus.is_focused(window) {
            return Some(Focused::Cancel);
        }
        if self.ok_focus.is_focused(window) {
            return Some(Focused::Ok);
        }
        if self.check_focus.is_focused(window) {
            return Some(Focused::Check);
        }
        let Some(Request { kind: Kind::Password { choices, choice_focus, .. }, .. }) = self.queue.front() else { return None };
        choices.iter().zip(choice_focus).find(|(_, focus)| focus.is_focused(window)).map(|(choice, _)| Focused::Choice(choice.value.clone()))
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.settle(false, None, window, cx);
            cx.stop_propagation();
            return;
        }
        if key != "enter" && key != "space" {
            return;
        }
        let password = matches!(self.queue.front().map(|r| &r.kind), Some(Kind::Password { .. }));
        match self.focused(window) {
            Some(Focused::Cancel) => self.settle(false, None, window, cx),
            Some(Focused::Ok) => self.settle(true, None, window, cx),
            Some(Focused::Check) if key == "space" => self.toggle_check(cx),
            Some(Focused::Choice(value)) => self.settle(true, Some(value), window, cx),
            // 框本身（鼠标点了框里的空白）上的回车、空格和复选框上的回车 = 默认按钮（它画着焦点框，见 `default_ringed`）；
            // 密码框里的回车（它自己的 PressEnter）和空格归输入框
            Some(Focused::Check) | None if !password => self.settle(true, None, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// 默认按钮画不画焦点框：画的就是回车会按下的那个。焦点在它上面、在框本身上，或者在鼠标点过的复选框上
    /// （复选框只有 Tab 选中时自己画框，空格勾它）。密码框不画：焦点在输入框里，看得出来。
    fn default_ringed(&self, password: bool, window: &Window) -> bool {
        match self.focused(window) {
            Some(Focused::Ok) => true,
            Some(Focused::Check) => !window.last_input_was_keyboard(),
            None => !password,
            Some(Focused::Cancel | Focused::Choice(_)) => false,
        }
    }
}

impl Render for PromptHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(head) = self.queue.front() else {
            return div().into_any_element();
        };
        let ok_ringed = self.default_ringed(matches!(head.kind, Kind::Password { .. }), window);
        let cancel_ringed = self.cancel_focus.is_focused(window);
        let options = &head.options;
        let is_alert = matches!(head.kind, Kind::Alert(_));
        let password = match &head.kind {
            Kind::Password { choices, choice_focus, input, .. } => Some((choices.clone(), choice_focus.clone(), input.clone())),
            _ => None,
        };
        let check = match &head.kind {
            Kind::Check { checked, .. } => options.checkbox.clone().map(|label| (label, *checked)),
            _ => None,
        };
        let body = div()
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
                                d.child(div().text_size(px(14.)).text_color(theme::alpha(ui::FOREGROUND, 0.8)).child(detail))
                            })
                            .when_some(options.message.clone(), |d, message| {
                                d.child(div().text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(message))
                            }),
                    )
                    .when_some(password, |d, (choices, choice_focus, input)| {
                        d.when(!choices.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .child(div().text_size(px(12.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("已保存的密码")))
                                    .child(div().flex().flex_wrap().gap(px(6.)).children(choices.into_iter().zip(choice_focus).enumerate().map(|(i, (choice, focus))| {
                                        let value = choice.value.clone();
                                        let ringed = focus.is_focused(window);
                                        kit::ring(kit::button(("prompt-choice", i), Variant::Outline).track_focus(&focus), ringed)
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
                                .child(kit::focus_ring(kit::checkbox("prompt-check-box", checked).track_focus(&self.check_focus)).aria_label(label.clone()).on_click(cx.listener(|host, _, _, cx| {
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
                            kit::ring(kit::button("prompt-cancel", Variant::Outline).track_focus(&self.cancel_focus), cancel_ringed)
                                .aria_label(options.cancel_label.clone())
                                .child(options.cancel_label.clone())
                                .on_click(cx.listener(|host, _, window, cx| host.settle(false, None, window, cx))),
                        )
                    })
                    .child(
                        kit::ring(
                            kit::button("prompt-ok", if options.destructive { Variant::Destructive } else { Variant::Primary })
                                .track_focus(&self.ok_focus),
                            ok_ringed,
                        )
                        .aria_label(options.confirm_label.clone())
                        .child(options.confirm_label.clone())
                        .on_click(cx.listener(|host, _, window, cx| host.settle(true, None, window, cx))),
                    ),
            )
            .focus_trap("prompt-trap", &self.focus);
        kit::modal("prompt", 420., body).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;

    /// 框后面还有一个 Tab 能停的地方（真实界面里是侧栏的搜索框、地址栏）。
    struct Page {
        host: Entity<PromptHost>,
        behind: FocusHandle,
    }

    impl Render for Page {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(div().track_focus(&self.behind).size(px(10.))).child(self.host.clone())
        }
    }

    fn window(cx: &mut TestAppContext) -> &mut VisualTestContext {
        let host = cx.update(|cx| {
            gpui_base::init(cx);
            init(cx)
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let page = cx.new(|cx| Page { host, behind: cx.focus_handle().tab_stop(true) });
            gpui_base::Root::new(page, window, cx)
        });
        cx
    }

    /// 弹一个确认框（取消 / 关闭），按这几个键，回答案；按完框还开着就是 None。
    fn answer(cx: &mut VisualTestContext, keys: &str) -> Option<bool> {
        let mut answer = cx.update(|window, cx| confirm(window, cx, Options::new("关闭「本机」").confirm("关闭")));
        cx.run_until_parked();
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        let result = answer.try_recv().ok().flatten();
        if result.is_none() {
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
        }
        result
    }

    /// Tab 曾经把焦点切到框后面去（终端、侧栏搜索框），之后回车、空格都到不了框里，只能用鼠标点。
    #[gpui::test]
    fn keyboard_reaches_every_button_of_a_confirm(cx: &mut TestAppContext) {
        let cx = window(cx);
        assert_eq!(answer(cx, "enter"), Some(true), "回车 = 默认按钮");
        assert_eq!(answer(cx, "escape"), Some(false));
        assert_eq!(answer(cx, "tab enter"), Some(false), "Tab 从默认按钮转到「取消」");
        assert_eq!(answer(cx, "tab tab enter"), Some(true));
        assert_eq!(answer(cx, "tab tab space"), Some(true), "空格按下选中的按钮");
        assert_eq!(answer(cx, "tab tab tab space"), Some(false), "转一圈回到「取消」，不跑出框");
        assert_eq!(answer(cx, "shift-tab enter"), Some(false));
        assert_eq!(answer(cx, "shift-tab shift-tab enter"), Some(true));
        assert_eq!(answer(cx, "tab escape"), Some(false));
    }

    /// 弹出时看不出回车会按哪个（用户：「看不出来是不是关闭」）：焦点要落在默认按钮上（它画焦点框），空格也按它。
    #[gpui::test]
    fn the_default_button_is_selected_when_a_prompt_opens(cx: &mut TestAppContext) {
        let cx = window(cx);
        let host = cx.update(|_, cx| host(cx));
        let _answer = cx.update(|window, cx| confirm(window, cx, Options::new("关闭「本机」").confirm("关闭")));
        cx.run_until_parked();
        let ok = cx.update(|window, cx| host.read(cx).ok_focus.is_focused(window));
        assert!(ok, "焦点在「关闭」上");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(answer(cx, "space"), Some(true), "空格 = 默认按钮");
    }

    /// 鼠标点了框里的空白（焦点到框本身），默认按钮照样是回车、空格按的那个。
    #[gpui::test]
    fn space_presses_the_default_button_after_clicking_the_prompt(cx: &mut TestAppContext) {
        let cx = window(cx);
        let host = cx.update(|_, cx| host(cx));
        let mut answer = cx.update(|window, cx| confirm(window, cx, Options::new("关闭「本机」").confirm("关闭")));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let focus = host.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        assert_eq!(answer.try_recv().ok().flatten(), Some(true));
    }

    #[gpui::test]
    fn space_toggles_the_checkbox_and_enter_still_confirms(cx: &mut TestAppContext) {
        let cx = window(cx);
        let mut answer = cx.update(|window, cx| confirm_with_check(window, cx, Options::new("替换").checkbox("全部这样处理")));
        cx.run_until_parked();
        cx.simulate_keystrokes("tab space enter");
        cx.run_until_parked();
        assert_eq!(answer.try_recv().ok().flatten(), Some((true, true)));
    }
}

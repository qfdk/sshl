//! 顶栏：会话标签（可拖动排序）+ 右侧的 终端 / 文件管理、填充密码、断开（旧 SessionTabs.tsx + App.tsx 顶栏）。

use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, FontWeight, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, prelude::FluentBuilder, px,
};

use crate::assets::Icon;
use crate::groups::{self, Side};
use crate::settings;
use crate::sidebar::DragPreview;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};
use crate::workspace::{DraggedTab, MainTab, Workspace, drag_region};

/// 按住 ⌘ 时浮在标签 / 按钮右上角的快捷键。
fn hint(key: impl Into<SharedString>, visible: bool) -> AnyElement {
    div()
        .absolute()
        .right(px(-6.))
        .top(px(-8.))
        .h(px(16.))
        .min_w(px(16.))
        .px_1()
        .rounded(px(5.))
        .bg(theme::color(ui::PRIMARY))
        .text_color(theme::color(ui::PRIMARY_FOREGROUND))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .shadow_sm()
        .when(!visible, |h| h.opacity(0.))
        .child(key.into())
        .into_any_element()
}

impl Workspace {
    pub(crate) fn top_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !cx.has_active_drag() {
            self.tab_drop = None;
        }
        let connected = self.active.is_some();
        drag_region("topbar")
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(self.session_tabs(cx))
            .child(
                div()
                    .ml_auto()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(self.latency.clone().filter(|(id, _)| connected && self.active.as_ref() == Some(id)), |r, (_, rtt)| {
                        let (text, color) = latency_text(rtt);
                        r.child(
                            div()
                                .min_w(px(52.))
                                .flex()
                                .justify_end()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::color(color))
                                .child(text),
                        )
                    })
                    .when(connected, |r| r.child(self.snippet_button(cx)).child(self.main_tabs(cx)))
                    .when(connected && (self.fill_feedback || self.active_session().is_some_and(|s| s.view.read(cx).password_prompt())), |r| {
                        r.child(self.fill_button(cx))
                    })
                    .when_some(self.fill_menu_panel(window, cx), |r, panel| r.child(panel)),
            )
            .into_any_element()
    }

    fn main_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let hints = self.hint_visible;
        let this = cx.entity().downgrade();
        div()
            .relative()
            .child(kit::segmented(
                "main-tabs",
                &[(MainTab::Terminal, None, "终端".into()), (MainTab::Files, None, "文件管理".into()), (MainTab::System, None, "系统".into())],
                self.main_tab,
                move |tab, window, cx| {
                    let _ = this.update(cx, |this, cx| this.set_main_tab(tab, window, cx));
                },
            ))
            // 数字角标浮在每个选项的右上角（与旧版一样不挤动布局）；选项等宽，叠一层同样三等分的格子放角标
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .p(px(3.))
                    .flex()
                    .gap(px(2.))
                    .children((1..=3).map(|n| div().relative().flex_1().h_full().child(hint(n.to_string(), hints)))),
            )
            .into_any_element()
    }

    /// 命令片段按钮（⌘P）和它下面的面板。
    fn snippet_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.snippets_open;
        let button = div()
            .id("snippets")
            .role(Role::Button)
            .aria_label("命令片段")
            .relative()
            .size(px(34.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .bg(theme::color(ui::SECONDARY))
            .text_color(theme::color(if open { ui::PRIMARY } else { ui::MUTED_FOREGROUND }))
            .cursor_pointer()
            .hover(|s| s.text_color(theme::color(if open { ui::PRIMARY } else { ui::FOREGROUND })))
            .when(!self.hint_visible, |b| b.tooltip(|window, cx| kit::tooltip("命令片段".into(), window, cx)))
            .child(kit::icon_sized(Icon::CodeXml, 16.))
            .child(hint("P", self.hint_visible))
            .on_hover(cx.listener(|this, hovered: &bool, _, _| this.snippet_button_hovered = *hovered))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_snippets(window, cx)));
        let panel = open.then(|| {
            let this = cx.entity().downgrade();
            div().absolute().right_0().top(px(40.)).child(
                deferred(
                    anchored().anchor(gpui::Anchor::TopRight).child(
                        div()
                            .on_mouse_down_out(move |_, window, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    // 点的是片段按钮：交给按钮的点击去关，不然这里先关、按钮又打开
                                    if !this.snippet_button_hovered {
                                        this.close_snippets(window, cx);
                                    }
                                });
                            })
                            .child(self.snippet_panel.clone()),
                    ),
                )
                .with_priority(2),
            )
        });
        div().relative().child(button).when_some(panel, |d, panel| d.child(panel)).into_any_element()
    }

    fn fill_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.fill_feedback { "填充发送完毕" } else { "填充密码" };
        div()
            .flex()
            .items_center()
            .child(
                div()
                    .id("fill-password")
                    .role(Role::Button)
                    .aria_label("填充密码")
                    .h(px(34.))
                    .pl_3()
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_l(px(10.))
                    .bg(theme::color(ui::PRIMARY))
                    .text_color(theme::color(ui::PRIMARY_FOREGROUND))
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::alpha(ui::PRIMARY, 0.9)))
                    .child(kit::icon_sized(Icon::KeyRound, 14.))
                    .child(label)
                    .on_click(cx.listener(|this, _, window, cx| this.fill_password_primary(window, cx))),
            )
            .child(
                div()
                    .id("fill-password-menu")
                    .role(Role::Button)
                    .aria_label("选择账号密码")
                    .h(px(34.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded_r(px(10.))
                    .border_l_1()
                    .border_color(theme::alpha(ui::PRIMARY_FOREGROUND, 0.25))
                    .bg(theme::color(ui::PRIMARY))
                    .text_color(theme::color(ui::PRIMARY_FOREGROUND))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::alpha(ui::PRIMARY, 0.9)))
                    .child(kit::icon_sized(Icon::ChevronDown, 14.))
                    .on_click(cx.listener(|this, _, window, cx| this.open_fill_menu(window, cx))),
            )
            .into_any_element()
    }

    fn fill_menu_panel(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.fill_menu.as_ref()?;
        let mut items: Vec<AnyElement> = Vec::new();
        let row = |id: SharedString, label: SharedString, kind: String, cx: &mut Context<Self>| {
            div()
                .id(id)
                .role(Role::MenuItem)
                .aria_label(label.clone())
                .px_2()
                .py(px(6.))
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .hover(|s| s.bg(theme::color(ui::MUTED)))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.fill_password(kind.clone(), window, cx)))
                .into_any_element()
        };
        if menu.has_password {
            items.push(row("fill-main".into(), "连接密码".into(), "password".into(), cx));
        }
        for (i, account) in menu.accounts.iter().enumerate() {
            items.push(row(SharedString::from(format!("fill-acct-{i}")), account.clone().into(), format!("acct:{account}"), cx));
        }
        let panel = div()
            .id("fill-menu")
            .occlude()
            .w(px(256.))
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(12.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow_lg()
            .child(div().flex().flex_col().gap_1().children(items))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(theme::color(ui::BORDER))
                    .child(div().text_xs().font_weight(FontWeight::MEDIUM).child("新增账号密码"))
                    .child(kit::text_field(&menu.account, false).h(px(32.)).text_xs())
                    .child(kit::text_field(&menu.password, false).h(px(32.)).text_xs())
                    .child(
                        kit::button("fill-save", Variant::Primary)
                            .h(px(30.))
                            .text_xs()
                            .child("保存")
                            .on_click(cx.listener(|this, _, window, cx| this.save_fill_account(window, cx))),
                    ),
            );
        Some(
            div()
                .absolute()
                .right_0()
                .top(px(40.))
                .child(deferred(anchored().anchor(gpui::Anchor::TopRight).offset(point(px(0.), px(0.))).child(panel)))
                .into_any_element(),
        )
    }

    fn session_tabs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.sessions.is_empty() {
            return div().into_any_element();
        }
        let tabs: Vec<AnyElement> = self
            .sessions
            .iter()
            .map(|session| {
                let id = session.connection_id.clone();
                let active = self.active.as_deref() == Some(id.as_str());
                let name = self.connection(&id).map(|c| SharedString::from(c.name.clone())).unwrap_or_else(|| session.name.clone());
                let host: SharedString = self.connection(&id).map(|c| groups::distinct_host(&name, &c.host).to_string()).unwrap_or_default().into();
                let mark = match &self.tab_drop {
                    Some((target, side)) if *target == id => Some(*side),
                    _ => None,
                };
                let dragging = self.dragging.as_deref() == Some(id.as_str()) && cx.has_active_drag();
                let disconnected = session.view.read(cx).is_disconnected();
                let (click_id, close_id, drag_id, move_id, drop_name) = (id.clone(), id.clone(), id.clone(), id.clone(), name.clone());
                div()
                    .id(SharedString::from(format!("tab-{id}")))
                    .role(Role::Tab)
                    .aria_label(name.clone())
                    .relative()
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .rounded(px(10.))
                    .border_1()
                    .map(|t| {
                        if active {
                            t.border_color(theme::color(ui::BORDER)).bg(theme::color(ui::CARD)).shadow_sm()
                        } else {
                            t.border_color(gpui::transparent_black()).hover(|s| s.bg(theme::alpha(ui::FOREGROUND, 0.04)))
                        }
                    })
                    .when(dragging, |t| t.opacity(0.5))
                    // 落点竖线：看得出会插到这个标签的左边还是右边
                    .when_some(mark, |t, side| {
                        t.child(
                            div()
                                .absolute()
                                .top(px(4.))
                                .bottom(px(4.))
                                .w(px(2.))
                                .rounded_full()
                                .bg(theme::color(ui::PRIMARY))
                                .map(|line| if side == Side::Before { line.left(px(-5.)) } else { line.right(px(-5.)) }),
                        )
                    })
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pl_3()
                            .pr(px(6.))
                            .cursor_pointer()
                            .child(div().size(px(7.)).flex_shrink_0().rounded_full().bg(theme::color(if disconnected { ui::MUTED_FOREGROUND } else { ui::SUCCESS })))
                            .child(
                                div()
                                    .truncate()
                                    .map(|t| if active { t.font_weight(FontWeight::SEMIBOLD) } else { t.text_color(theme::alpha(ui::FOREGROUND, 0.8)) })
                                    .child(name.clone()),
                            )
                            .when(active && !host.is_empty(), |t| {
                                t.child(
                                    div()
                                        .truncate()
                                        .font_family(settings::DEFAULT_FONT_FAMILY)
                                        .text_size(px(11.))
                                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                                        .child(host.clone()),
                                )
                            }),
                    )
                    .child(
                        kit::mini_button(SharedString::from(format!("close-{id}")), Icon::X, SharedString::from(format!("断开 {name}")), ui::MUTED_FOREGROUND)
                            .mr_1()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.disconnect(&close_id, window, cx);
                            })),
                    )
                    // 走 connect_to_saved 而不是直接切：会话已失效时它会接着重连，和侧栏双击一致；
                    // 已断开的标签只切过去看（终端里提示按 Enter 重连），不自动重连
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.active.as_deref() == Some(click_id.as_str()) {
                            return;
                        }
                        if this.session(&click_id).is_some_and(|s| s.view.read(cx).is_disconnected()) {
                            this.activate(&click_id, window, cx);
                        } else {
                            this.connect_to_saved(&click_id, window, cx);
                        }
                    }))
                    .on_drag(DraggedTab { connection_id: drag_id, name: drop_name }, {
                        let this = cx.entity().downgrade();
                        move |dragged, _, _, cx| {
                            let id = dragged.connection_id.clone();
                            let _ = this.update(cx, |this, _| this.dragging = Some(id));
                            let name = dragged.name.clone();
                            cx.new(|_| DragPreview(name))
                        }
                    })
                    .on_drag_move::<DraggedTab>(cx.listener(move |this, event: &DragMoveEvent<DraggedTab>, _, cx| {
                        if !event.bounds.contains(&event.event.position) || event.drag(cx).connection_id == move_id {
                            return;
                        }
                        let side = if event.event.position.x < event.bounds.center().x { Side::Before } else { Side::After };
                        let mark = Some((move_id.clone(), side));
                        if this.tab_drop != mark {
                            this.tab_drop = mark;
                            cx.notify();
                        }
                    }))
                    .on_drop(cx.listener(|this, dragged: &DraggedTab, _, cx| {
                        if let Some((target, side)) = this.tab_drop.take() {
                            this.move_tab(&dragged.connection_id, &target, side, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .id("session-tabs")
            .role(Role::TabList)
            .aria_label("已打开的会话")
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .children(tabs)
            // 没有会话时侧栏的「新建连接」就够了，这里只在已有标签时提供 +
            .child(
                div()
                    .id("tab-new")
                    .role(Role::Button)
                    .aria_label("新建连接")
                    .size(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(10.))
                    .border_1()
                    .border_dashed()
                    .border_color(theme::alpha(ui::FOREGROUND, 0.2))
                    .text_color(theme::color(ui::MUTED_FOREGROUND))
                    .cursor_pointer()
                    .hover(|s| s.border_color(theme::alpha(ui::FOREGROUND, 0.35)).text_color(theme::color(ui::FOREGROUND)))
                    .child(kit::icon_sized(Icon::Plus, 14.))
                    .on_click(cx.listener(|this, _, window, cx| this.open_connection_dialog(None, window, cx))),
            )
            .into_any_element()
    }
}

/// 延迟的显示：< 100ms 绿、≤ 200ms 黄、更慢或超时红。
pub(crate) fn latency_text(rtt: Option<std::time::Duration>) -> (String, u32) {
    match rtt {
        None => ("超时".into(), ui::DESTRUCTIVE),
        Some(rtt) => {
            let ms = rtt.as_millis();
            let color = if ms < 100 {
                ui::SUCCESS
            } else if ms <= 200 {
                ui::WARNING
            } else {
                ui::DESTRUCTIVE
            };
            (if ms == 0 { "<1 ms".into() } else { format!("{ms} ms") }, color)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn latency_is_colored_by_how_slow_it_is() {
        assert_eq!(latency_text(Some(Duration::from_millis(23))), ("23 ms".into(), ui::SUCCESS));
        assert_eq!(latency_text(Some(Duration::from_millis(99))).1, ui::SUCCESS);
        assert_eq!(latency_text(Some(Duration::from_millis(100))).1, ui::WARNING);
        assert_eq!(latency_text(Some(Duration::from_millis(200))).1, ui::WARNING);
        assert_eq!(latency_text(Some(Duration::from_millis(201))).1, ui::DESTRUCTIVE);
        assert_eq!(latency_text(None), ("超时".into(), ui::DESTRUCTIVE));
        assert_eq!(latency_text(Some(Duration::from_micros(300))).0, "<1 ms");
    }
}

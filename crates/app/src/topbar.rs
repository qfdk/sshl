//! 顶栏：会话标签（可拖动排序）+ 右侧的 终端 / 文件管理、填充密码、断开（旧 SessionTabs.tsx + App.tsx 顶栏）。

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, DragMoveEvent, Entity, Focusable, Font, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, Subscription, TextRun, Window, actions, anchored,
    deferred, div, point, prelude::FluentBuilder, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::Icon;
use crate::groups::{self, Side};
use crate::settings;
use crate::sidebar::{DragPreview, sidebar_toggle_button};
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};
use crate::workspace::{CONTENT_EDGE, ConnectionMenu, DraggedTab, MainTab, TOPBAR_TOP_BESIDE_TRAFFIC_LIGHTS, TRAFFIC_LIGHTS_WIDTH, Workspace, controls, drag_region};

actions!(topbar, [ConfirmTabRename, CancelTabRename]);

/// 标签重命名的输入框。它在顶栏里、不在终端的上下文里：敲的字不会被终端的按键处理发给远端。
const RENAME_CONTEXT: &str = "TabRename";
/// 重命名输入框的宽度跟着字走，夹在这两个数之间：太窄不好敲，太宽把别的标签挤走。
const RENAME_MIN_WIDTH: f32 = 80.;
const RENAME_MAX_WIDTH: f32 = 240.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        // 绑在比 gpui-base 输入框更深的上下文里，压过它自己的回车 / Esc
        KeyBinding::new("enter", ConfirmTabRename, Some("TabRename > Input")),
        KeyBinding::new("escape", CancelTabRename, Some("TabRename > Input")),
    ]
}

/// 正在原地重命名的标签：名字换成输入框。
pub(crate) struct TabRename {
    pub(crate) connection_id: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl TabRename {
    pub(crate) fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }
}

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
        self.settle_rename(window, cx);
        let connected = self.active.is_some();
        // 本机标签只有终端：不放 终端 / 文件管理 / 系统 和填充密码
        let local = self.active_session().is_some_and(|s| s.local);
        let sidebar_collapsed = settings::get(cx).sidebar_collapsed;
        drag_region("topbar")
            // 窗口顶上到标签之间那条也属于拖动区；侧栏收起时顶栏和红绿灯同一行，上边距收到让两者竖直居中对齐
            .map(|bar| if sidebar_collapsed && !window.is_fullscreen() { bar.pt(px(TOPBAR_TOP_BESIDE_TRAFFIC_LIGHTS)) } else { bar.pt_3() })
            // 内容区（终端页）贴满窗口，两边的留白只给顶栏；侧栏展开时左边紧挨侧栏只留 4px
            .pr(px(CONTENT_EDGE))
            .map(|bar| if sidebar_collapsed { bar.pl(px(CONTENT_EDGE)) } else { bar.pl_1() })
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            // 侧栏收起后整列让给终端，红绿灯右边只留展开按钮（全屏时红绿灯藏起来，不用让位）。
            // 新建连接不再放一个：有标签时标签栏末尾有 +，没标签时页面中间有「新建连接」
            .when(sidebar_collapsed, |bar| {
                bar.child(
                    div()
                        .flex_shrink_0()
                        .when(!window.is_fullscreen(), |d| d.pl(px(TRAFFIC_LIGHTS_WIDTH - CONTENT_EDGE)))
                        .child(controls("topbar-expand").child(sidebar_toggle_button(true, cx))),
                )
            })
            .child(self.session_tabs(window, cx))
            .child(
                controls("topbar-actions")
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
                    .when(connected, |r| r.child(self.snippet_button(cx)))
                    .when(connected && !local, |r| r.child(self.main_tabs(cx)))
                    .when(connected && !local && (self.fill_feedback || self.active_session().is_some_and(|s| s.view.read(cx).password_prompt())), |r| {
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
            // 快捷键角标（⌘I / ⌘O / ⌘P）浮在每个选项的右上角（与旧版一样不挤动布局）；选项等宽，叠一层同样三等分的格子放角标
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .p(px(3.))
                    .flex()
                    .gap(px(2.))
                    .children(["I", "O", "P"].map(|key| div().relative().flex_1().h_full().child(hint(key, hints)))),
            )
            .into_any_element()
    }

    /// 命令片段按钮（⌘⇧P）和它下面的面板。
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
            // 和「终端 / 文件管理 / 系统」的 I / O / P 同一个基准：分段控件里的选项内缩 3px，角标相对选项定位
            .child(div().absolute().inset(px(3.)).child(div().relative().size_full().child(hint("⇧P", self.hint_visible))))
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

    fn session_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.sessions.is_empty() {
            return div().into_any_element();
        }
        let hints = self.hint_visible;
        let tabs: Vec<AnyElement> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                let id = session.connection_id.clone();
                let active = self.active.as_deref() == Some(id.as_str());
                let name = self.tab_name(session);
                let host: SharedString = self.connection(&id).map(|c| groups::distinct_host(&name, &c.host).to_string()).unwrap_or_default().into();
                let mark = match &self.tab_drop {
                    Some((target, side)) if *target == id => Some(*side),
                    _ => None,
                };
                let dragging = self.dragging.as_deref() == Some(id.as_str()) && cx.has_active_drag();
                let disconnected = session.view.read(cx).is_disconnected();
                let close_label = SharedString::from(format!("{} {name}", if session.local { "关闭" } else { "断开" }));
                let (click_id, close_id, drag_id, move_id, drop_name) = (id.clone(), id.clone(), id.clone(), id.clone(), name.clone());
                let rename = self.renaming.as_ref().filter(|r| r.connection_id == id);
                let label = match rename {
                    Some(rename) => self.rename_field(rename, window, cx),
                    None => div()
                        .truncate()
                        .map(|t| if active { t.font_weight(FontWeight::SEMIBOLD) } else { t.text_color(theme::alpha(ui::FOREGROUND, 0.8)) })
                        .child(name.clone())
                        .into_any_element(),
                };
                let tab = div()
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
                            .when(rename.is_none(), |t| t.cursor_pointer())
                            .child(div().size(px(7.)).flex_shrink_0().rounded_full().bg(theme::color(if disconnected { ui::MUTED_FOREGROUND } else { ui::SUCCESS })))
                            .child(label)
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
                        kit::mini_button(SharedString::from(format!("close-{id}")), Icon::X, close_label, ui::MUTED_FOREGROUND)
                            .mr_1()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.disconnect(&close_id, window, cx);
                            })),
                    )
                    // ⌘ 角标：前 9 个标签浮出序号（⌘1 … ⌘9）；正在改名的不显示。延后画：gpui 的边框画在子元素之后，
                    // 不延后的话当前标签的边框线会压在角标上
                    .when(index < 9 && rename.is_none(), |t| t.child(gpui::deferred(hint((index + 1).to_string(), hints))));
                // 改名中：不接单击 / 双击 / 右键 / 拖动，在输入框里点、拖选文字只归输入框
                if rename.is_some() {
                    return tab.into_any_element();
                }
                tab
                    .on_mouse_down(gpui::MouseButton::Right, cx.listener({
                        let id = id.clone();
                        move |this, event: &gpui::MouseDownEvent, _, cx| {
                            this.menu = Some(ConnectionMenu { connection_id: id.clone(), position: event.position, on_tab: true });
                            cx.notify();
                        }
                    }))
                    // 单击切过去，双击原地改名（非当前标签：第一下已经切过去了）
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.start_rename(&click_id, window, cx);
                        } else {
                            this.select_tab(&click_id, window, cx);
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
        controls("session-tabs")
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

    /// 改名中的标签里，名字换成的输入框。宽度跟着字走（夹在最小 / 最大之间），换上去时标签只宽出输入框的内边距。
    fn rename_field(&self, rename: &TabRename, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let text = rename.input.read(cx).value();
        let style = window.text_style();
        let text_width = if text.is_empty() {
            0.
        } else {
            let font = Font { weight: FontWeight::SEMIBOLD, ..style.font() };
            let run = TextRun { len: text.len(), font, color: style.color, background_color: None, underline: None, strikethrough: None };
            f32::from(window.text_system().shape_line(text, px(14.), &[run], None).width)
        };
        // 左右内边距 12 + 边框 2 + 光标留一点
        let width = (text_width + 20.).clamp(RENAME_MIN_WIDTH, RENAME_MAX_WIDTH);
        let input = rename.input.clone();
        div()
            .id("tab-rename")
            .key_context(RENAME_CONTEXT)
            .role(Role::TextInput)
            .aria_label("重命名标签")
            .w(px(width))
            .h(px(26.))
            .px(px(6.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(theme::color(ui::RING))
            .bg(theme::color(ui::CARD))
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(px(20.))
            .on_action(cx.listener(|this, _: &ConfirmTabRename, window, cx| this.finish_rename(true, window, cx)))
            .on_action(cx.listener(|this, _: &CancelTabRename, window, cx| this.finish_rename(false, window, cx)))
            // 点到输入框外面就确认，拖动区、侧栏这类不拿焦点的地方也算。等这次点击把焦点挪完再收尾：
            // 点进终端的焦点留在终端，点在不拿焦点的地方才把焦点交回内容区
            .on_mouse_down_out(cx.listener(move |this, _, window, cx| this.finish_rename_later(&input, true, window, cx)))
            .child(div().flex_1().min_w_0().child(Input::new(&rename.input)))
            .into_any_element()
    }

    /// 双击标签 / 右键「重命名」：名字原地换成输入框，预填当前名字并全选。非当前标签先切过去（同点击标签）。
    pub(crate) fn start_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.as_ref().is_some_and(|r| r.connection_id == id) {
            return;
        }
        // 另一个标签还在改：先按确认收掉
        self.finish_rename(true, window, cx);
        self.select_tab(id, window, cx);
        let Some(current) = self.session(id).map(|s| self.tab_name(s)) else { return };
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |input, cx| {
            input.set_value(current, window, cx);
            input.select_all(window, cx);
            input.focus(window, cx);
        });
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| match event {
            // 宽度跟着字走
            InputEvent::Change => cx.notify(),
            // 失焦 = 确认（焦点已经去了别处，不抢回来）
            InputEvent::Blur => this.finish_rename_later(input, true, window, cx),
            _ => {}
        });
        self.renaming = Some(TabRename { connection_id: id.to_string(), input, _subscription: subscription });
        cx.notify();
    }

    /// 收掉重命名。`commit` 时把输入规整成自定义名（`groups::tab_custom_title`：空的 / 和默认一样 = 恢复默认名字）。
    /// 焦点交回这个标签的内容区：回车 / Esc 时它还在输入框里；点在不拿焦点的地方（顶栏空白、侧栏）时它落到了工作区自己身上，
    /// 都算没地方去。已经去了别处（点进终端、切了标签、弹了对话框 / 提示框）的不抢回来。
    pub(crate) fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.renaming.take() else { return };
        if commit {
            let value = rename.input.read(cx).value();
            if let Some(at) = self.sessions.iter().position(|s| s.connection_id == rename.connection_id) {
                let default = self.default_tab_name(&self.sessions[at]);
                self.sessions[at].custom_title = groups::tab_custom_title(&value, &default).map(SharedString::from);
            }
        }
        if rename.is_focused(window, cx) || self.focus.is_focused(window) || window.focused(cx).is_none() {
            self.refocus(window, cx);
        }
        cx.notify();
    }

    /// 这一轮事件处理完再收尾（失焦、点到别处时：那次点击先把焦点挪到它该去的地方）。
    /// 期间已经收掉、或换成了另一次重命名的，不动。
    fn finish_rename_later(&mut self, input: &Entity<InputState>, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let input = input.clone();
        cx.defer_in(window, move |this, window, cx| {
            if this.renaming.as_ref().is_some_and(|r| r.input == input) {
                this.finish_rename(commit, window, cx);
            }
        });
    }

    /// 收尾不能只靠失焦事件：窗口不在前台时 GPUI 不发 Focus / Blur，那时焦点被拿走（切标签、弹提示框）就收不到，
    /// 输入框会一直挂着。每次画顶栏时看一眼，输入框已经没焦点了就按确认收掉。
    fn settle_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(rename) = &self.renaming
            && !rename.is_focused(window, cx)
        {
            let input = rename.input.clone();
            self.finish_rename_later(&input, true, window, cx);
        }
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
    fn enter_and_escape_reach_the_tab_rename_before_the_input_box() {
        use gpui::{KeyContext, Keymap, Keystroke};
        gpui::actions!(test, [InputEnter, InputEscape]);
        // 同 gpui-base：输入框自己的回车 / Esc 先绑（main.rs 里 gpui_base::init 在前）
        let mut keymap = Keymap::new(vec![
            KeyBinding::new("enter", InputEnter, Some("Input")),
            KeyBinding::new("escape", InputEscape, Some("Input")),
        ]);
        keymap.add_bindings(crate::workspace::key_bindings());
        keymap.add_bindings(key_bindings());
        let stack: Vec<KeyContext> = ["Root", "Workspace", RENAME_CONTEXT, "Input"].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let first = |key: &'static str| {
            let bindings = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack).0;
            bindings.first().map(|b| b.action().name().to_string())
        };
        assert_eq!(first("enter").as_deref(), Some("topbar::ConfirmTabRename"));
        assert_eq!(first("escape").as_deref(), Some("topbar::CancelTabRename"));
        // 改名时 ⌘ 快捷键照常归工作区（切走 = 输入框失焦 = 确认）
        assert_eq!(first("cmd-2").as_deref(), Some("workspace::SelectTab"));
        // 别的输入框（侧栏搜索）里回车还是输入框自己的
        let search: Vec<KeyContext> = ["Root", "Workspace", "Input"].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let bindings = keymap.bindings_for_input(&[Keystroke::parse("enter").unwrap()], &search).0;
        assert_eq!(bindings.first().map(|b| b.action().name()), Some("test::InputEnter"));
    }

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

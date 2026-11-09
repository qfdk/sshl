//! 侧栏：应用标识、搜索、按分组的连接列表（拖动换组 / 排序）、新建连接与设置（旧 Sidebar.tsx + ConnectionList.tsx）。

use gpui::{
    AccessibleAction, AnyElement, AppContext, ClickEvent, Context, DragMoveEvent, FontWeight, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Render, Role, SharedString, StatefulInteractiveElement,
    Styled, Window, anchored, deferred, div, img, prelude::FluentBuilder, px,
};
use sshl_core::config_store::StoredConnection;

use crate::assets::{APP_ICON, Icon};
use crate::groups::{self, Side};
use crate::settings;
use crate::theme::{self, ui};
use crate::ui::{self as kit, MenuEntry, MenuItem, Variant};
use crate::workspace::{ConnectionMenu, DraggedConnection, DropTarget, Workspace, drag_region};

pub(crate) struct DragPreview(pub(crate) SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded(px(10.))
            .bg(theme::color(ui::CARD))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .shadow_lg()
            .text_size(px(14.))
            .child(self.0.clone())
    }
}

impl Workspace {
    fn sidebar_collapsed(&self, cx: &gpui::App) -> bool {
        settings::get(cx).sidebar_collapsed
    }

    pub(crate) fn sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !cx.has_active_drag() {
            self.drop_target = None;
            self.dragging = None;
        }
        let collapsed = self.sidebar_collapsed(cx);
        let toggle = kit::icon_button("toggle-sidebar", Variant::Ghost, if collapsed { Icon::ChevronRight } else { Icon::ChevronLeft }, if collapsed { "展开侧边栏" } else { "收起侧边栏" })
            .w(px(36.))
            .h(px(36.))
            .on_click(cx.listener(|_, _, _, cx| {
                settings::update(cx, |s| s.sidebar_collapsed = !s.sidebar_collapsed);
                cx.notify();
            }));
        div()
            .w(px(if collapsed { 76. } else { 240. }))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .overflow_hidden()
            // 透明标题栏：红绿灯单独占这一行，只作窗口拖动区。收起时 76px 宽刚好容下红绿灯。
            // 全屏时红绿灯藏起来（鼠标移到屏幕顶端才浮出），不再给它留一行，侧栏标题和顶栏标签对齐
            .child(drag_region("sidebar-titlebar").h(px(if window.is_fullscreen() { 6. } else { 50. })).flex_shrink_0())
            .map(|s| {
                if collapsed {
                    s.child(div().flex().justify_center().pb_1().child(toggle))
                } else {
                    s.child(
                        drag_region("sidebar-header")
                            .h(px(44.))
                            .pl_3()
                            .pr_2()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .child(img(APP_ICON).size(px(30.)).m(px(-3.)))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("SSHL"))
                            .child(div().ml_auto().child(toggle)),
                    )
                    .child(
                        div().px_3().pb_2().child(
                            div()
                                .relative()
                                .child(kit::text_field(&self.search, self.search_focused).pl(px(35.)))
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(11.))
                                        .top(px(10.))
                                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                                        .child(kit::icon(Icon::Search)),
                                ),
                        ),
                    )
                }
            })
            .child(div().mt_1().flex_1().min_h_0().child(self.connection_list(collapsed, window, cx)))
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .flex_shrink_0()
                    .gap_2()
                    .map(|f| if collapsed { f.flex_col().items_center().p_2() } else { f.p_3() })
                    .child(
                        kit::button("new-connection", Variant::Primary)
                            .aria_label("新建连接")
                            .map(|b| if collapsed { b.w(px(36.)).px_0() } else { b.flex_1() })
                            .child(kit::icon(Icon::Plus))
                            .when(!collapsed, |b| b.child("新建连接"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_connection_dialog(None, window, cx))),
                    )
                    .child(
                        kit::icon_button("open-settings", Variant::Outline, Icon::Settings, "设置")
                            .w(px(36.))
                            .h(px(36.))
                            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn connection_list(&mut self, collapsed: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let query = if collapsed { String::new() } else { self.search.read(cx).value().to_string() };
        let visible: Vec<StoredConnection> = self.connections.iter().filter(|c| groups::matches(c, &query)).cloned().collect();
        if visible.is_empty() {
            let text = if self.connections.is_empty() { "没有保存的连接" } else { "没有匹配的连接" };
            return div().p_4().text_center().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(text).into_any_element();
        }
        let refs: Vec<&StoredConnection> = visible.iter().collect();
        let ungrouped: Vec<StoredConnection> = groups::ungrouped(&refs).into_iter().cloned().collect();
        let grouped: Vec<(String, Vec<StoredConnection>)> = groups::group_connections(&refs, &self.group_order)
            .into_iter()
            .map(|g| (g.name, g.connections.into_iter().cloned().collect()))
            .collect();
        let dragging = self.dragging.is_some();
        let drop_ungrouped = self.drop_target == Some(DropTarget::Ungrouped);
        let ungrouped_empty = ungrouped.is_empty();

        let mut list = div()
            .id("connection-list")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1()
            .map(|l| if collapsed { l.px_1().py_2() } else { l.px_3().py_2() })
            // 列表里的空白 = 移出分组
            .on_drag_move::<DraggedConnection>(cx.listener(|this, event: &DragMoveEvent<DraggedConnection>, _, cx| {
                if event.bounds.contains(&event.event.position) && this.drop_target.is_none() {
                    this.drop_target = Some(DropTarget::Ungrouped);
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|this, dragged: &DraggedConnection, _, cx| {
                if let Some(target) = this.drop_target.take() {
                    this.move_connection(&dragged.id, &target, cx);
                }
            }));

        let items: Vec<AnyElement> = ungrouped.iter().map(|c| self.connection_item(c, "", collapsed, window, cx)).collect();
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .rounded(px(6.))
                // 未分组为空时高度为 0，拖动中给它一个最小高度，否则永远拖不进去
                .when(dragging, |d| d.min_h(px(40.)).border_1().border_dashed().border_color(theme::color(ui::BORDER)))
                .when(drop_ungrouped, |d| d.border_color(theme::color(ui::PRIMARY)).bg(theme::alpha(ui::PRIMARY, 0.1)))
                .children(items)
                .when(dragging && ungrouped_empty, |d| {
                    d.child(div().px_2().py_2().text_center().text_size(px(11.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child("拖到此处移出分组"))
                }),
        );

        for (name, connections) in grouped {
            let group_collapsed = settings::get(cx).collapsed_groups.contains(&name);
            let drop_here = self.drop_target == Some(DropTarget::Group(name.clone()));
            let count = connections.len();
            let toggle_name = name.clone();
            let header = div()
                .id(SharedString::from(format!("group-{name}")))
                .role(Role::Button)
                .aria_label(SharedString::from(if collapsed { format!("{name}，{count} 个连接") } else { name.clone() }))
                .flex()
                .items_center()
                .py_1()
                .rounded(px(6.))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::color(if drop_here { ui::PRIMARY } else { ui::MUTED_FOREGROUND }))
                .cursor_pointer()
                .hover(|s| s.bg(theme::alpha(ui::FOREGROUND, 0.04)))
                .map(|h| {
                    if collapsed {
                        // 收起后裸数字看不出是什么：文件夹图标（形态即展开状态）+ 右上角数量角标
                        h.justify_center().child(
                            div()
                                .relative()
                                .size(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(kit::icon(if group_collapsed { Icon::Folder } else { Icon::FolderOpen }))
                                .child(
                                    div()
                                        .absolute()
                                        .top(px(-4.))
                                        .right(px(-4.))
                                        .h(px(14.))
                                        .min_w(px(14.))
                                        .px_1()
                                        .rounded_full()
                                        .bg(theme::alpha(ui::MUTED_FOREGROUND, 0.2))
                                        .text_size(px(9.))
                                        .text_color(theme::color(ui::FOREGROUND))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(count.to_string()),
                                ),
                        )
                    } else {
                        h.gap_1()
                            .px(px(10.))
                            .child(kit::icon_sized(if group_collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, 14.))
                            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                            .child(count.to_string())
                    }
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    let name = toggle_name.clone();
                    settings::update(cx, |s| {
                        if let Some(at) = s.collapsed_groups.iter().position(|g| *g == name) {
                            s.collapsed_groups.remove(at);
                        } else {
                            s.collapsed_groups.push(name);
                        }
                    });
                    cx.notify();
                }));
            let target_name = name.clone();
            let items: Vec<AnyElement> =
                if group_collapsed { Vec::new() } else { connections.iter().map(|c| self.connection_item(c, &name, collapsed, window, cx)).collect() };
            list = list.child(
                div()
                    .id(SharedString::from(format!("section-{name}")))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .rounded(px(6.))
                    .when(drop_here, |d| d.bg(theme::alpha(ui::PRIMARY, 0.1)).border_1().border_color(theme::color(ui::PRIMARY)))
                    .on_drag_move::<DraggedConnection>(cx.listener(move |this, event: &DragMoveEvent<DraggedConnection>, _, cx| {
                        if !event.bounds.contains(&event.event.position) {
                            return;
                        }
                        let keep_item = matches!(&this.drop_target, Some(DropTarget::Item { group, .. }) if *group == target_name);
                        if !keep_item && this.drop_target != Some(DropTarget::Group(target_name.clone())) {
                            this.drop_target = Some(DropTarget::Group(target_name.clone()));
                            cx.notify();
                        }
                    }))
                    .child(header)
                    .when(!items.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .when(collapsed, |d| d.ml(px(6.)).border_l_1().border_color(theme::color(ui::BORDER)).pl_1())
                                .children(items),
                        )
                    }),
            );
        }
        list.into_any_element()
    }

    fn connection_item(&mut self, conn: &StoredConnection, group: &str, collapsed: bool, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let id = conn.id.clone();
        let connected = self.session(&conn.id).is_some();
        let active = connected && self.active.as_deref() == Some(conn.id.as_str());
        let dragging = self.dragging.as_deref() == Some(conn.id.as_str());
        let edge = match &self.drop_target {
            Some(DropTarget::Item { item, side, .. }) if *item == conn.id => Some(*side),
            _ => None,
        };
        let group_name = SharedString::from(format!("conn-{}", conn.id));
        let name: SharedString = conn.name.clone().into();
        let address: SharedString = if conn.username.is_empty() { conn.host.clone().into() } else { format!("{}@{}", conn.username, conn.host).into() };
        let (connect_id, menu_id, drag_id, item_group, item_id) = (id.clone(), id.clone(), id.clone(), group.to_string(), id.clone());
        let item_name = name.clone();

        let body = if collapsed {
            // 收起后一排相同的小圆点分不出哪台机器，改成名称缩写徽章，状态点退到角上
            div()
                .flex_1()
                .flex()
                .justify_center()
                .py(px(6.))
                .child(
                    div()
                        .relative()
                        .size(px(32.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .map(|b| {
                            if active {
                                b.bg(theme::color(ui::PRIMARY)).text_color(theme::color(ui::PRIMARY_FOREGROUND))
                            } else if connected {
                                b.bg(theme::alpha(ui::SUCCESS, 0.2))
                            } else {
                                b.bg(theme::color(ui::MUTED)).text_color(theme::color(ui::MUTED_FOREGROUND))
                            }
                        })
                        .child(groups::initials(&conn.name, &conn.host))
                        .when(connected, |b| {
                            b.child(
                                div()
                                    .absolute()
                                    .top(px(-2.))
                                    .right(px(-2.))
                                    .size(px(10.))
                                    .rounded_full()
                                    .bg(theme::color(ui::SUCCESS))
                                    .border_2()
                                    .border_color(theme::color(ui::CARD)),
                            )
                        }),
                )
        } else {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(6.))
                .child(
                    // 在线状态是图标角上的绿点，不占一列文字宽度，地址少被截断
                    div()
                        .relative()
                        .flex_shrink_0()
                        .text_color(theme::color(if connected { ui::FOREGROUND } else { ui::MUTED_FOREGROUND }))
                        .child(kit::icon(Icon::Server))
                        .when(connected, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .bottom(px(-2.))
                                    .right(px(-2.))
                                    .size(px(8.))
                                    .rounded_full()
                                    .bg(theme::color(ui::SUCCESS))
                                    .border_2()
                                    .border_color(theme::color(if active { ui::CARD } else { ui::SHELL })),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().font_weight(FontWeight::MEDIUM).line_height(px(20.)).child(name.clone()))
                        .child(
                            div()
                                .truncate()
                                .font_family(settings::DEFAULT_FONT_FAMILY)
                                .text_size(px(11.))
                                .line_height(px(16.))
                                .text_color(theme::color(ui::MUTED_FOREGROUND))
                                .child(address.clone()),
                        ),
                )
        };

        let actions = (!collapsed).then(|| {
            let (disconnect_id, edit_id, delete_id) = (id.clone(), id.clone(), id.clone());
            let editing = conn.clone();
            div()
                .absolute()
                .right(px(6.))
                .top(px(9.))
                .flex()
                .items_center()
                .gap(px(2.))
                .rounded(px(8.))
                .bg(theme::color(ui::CARD))
                .shadow_sm()
                .opacity(0.)
                .group_hover(group_name.clone(), |s| s.opacity(1.))
                .when(connected, |a| {
                    a.child(kit::mini_button(SharedString::from(format!("disconnect-{id}")), Icon::Power, "断开连接", ui::FOREGROUND).on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.disconnect(&disconnect_id, window, cx);
                        },
                    )))
                })
                .child(
                    kit::mini_button(SharedString::from(format!("edit-{id}")), Icon::Pencil, if connected { "断开后才能编辑" } else { "编辑连接" }, ui::FOREGROUND)
                        .when(connected, |b| b.opacity(0.4).cursor_default())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            if this.session(&edit_id).is_none() {
                                this.open_connection_dialog(Some(editing.clone()), window, cx);
                            }
                        })),
                )
                .child(
                    kit::mini_button(SharedString::from(format!("delete-{id}")), Icon::Trash, if connected { "断开后才能删除" } else { "删除连接" }, ui::DESTRUCTIVE)
                        .when(connected, |b| b.opacity(0.4).cursor_default())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            if this.session(&delete_id).is_none() {
                                this.delete_connection(&delete_id, window, cx);
                            }
                        })),
                )
        });

        div()
            .id(SharedString::from(format!("item-{}", conn.id)))
            .group(group_name)
            .role(Role::Button)
            .aria_label(name.clone())
            .relative()
            .flex()
            .min_h(px(44.))
            .rounded(px(10.))
            .border_1()
            .cursor_pointer()
            .text_size(px(14.))
            .map(|d| {
                if active {
                    d.border_color(theme::color(ui::BORDER)).bg(theme::color(ui::CARD)).shadow_sm()
                } else {
                    d.border_color(gpui::transparent_black()).hover(|s| s.bg(theme::alpha(ui::FOREGROUND, 0.04)))
                }
            })
            .when(dragging, |d| d.opacity(0.4))
            .when(collapsed, |d| d.tooltip({
                let text: SharedString = format!("{} · {}", conn.name, if connected { "已连接" } else { "未连接" }).into();
                move |window, cx| kit::tooltip(text.clone(), window, cx)
            }))
            // 插入线：看得出会落在这一项的上边还是下边
            .when_some(edge, |d, side| {
                d.child(
                    div()
                        .absolute()
                        .left(px(4.))
                        .right(px(4.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(theme::color(ui::PRIMARY))
                        .map(|line| if side == Side::Before { line.top(px(-3.)) } else { line.bottom(px(-3.)) }),
                )
            })
            .child(body)
            .when_some(actions, |d, actions| d.child(actions))
            // 鼠标单击不连接，双击才连；键盘和辅助功能的「按下」直接连接（VoiceOver 才连得上）
            .on_click(cx.listener({
                let id = connect_id.clone();
                move |this, event: &ClickEvent, window, cx| {
                    if event.is_keyboard() || event.click_count() >= 2 {
                        this.connect_to_saved(&id, window, cx);
                    }
                }
            }))
            .on_a11y_action(AccessibleAction::Click, {
                let this = cx.entity().downgrade();
                let id = connect_id;
                move |_, window, cx| {
                    let _ = this.update(cx, |this, cx| this.connect_to_saved(&id, window, cx));
                }
            })
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.menu = Some(ConnectionMenu { connection_id: menu_id.clone(), position: event.position });
                cx.notify();
            }))
            .on_drag(DraggedConnection { id: drag_id, name: item_name }, {
                let this = cx.entity().downgrade();
                move |dragged, _, _, cx| {
                    let id = dragged.id.clone();
                    let _ = this.update(cx, |this, _| this.dragging = Some(id));
                    let name = dragged.name.clone();
                    cx.new(|_| DragPreview(name))
                }
            })
            .on_drag_move::<DraggedConnection>(cx.listener(move |this, event: &DragMoveEvent<DraggedConnection>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                let dragged = event.drag(cx).id.clone();
                if dragged == item_id {
                    return;
                }
                let before_half = event.event.position.y < event.bounds.center().y;
                let next = if before_half {
                    Some(item_id.clone())
                } else {
                    this.next_in_group(&item_group, &item_id)
                };
                let target = DropTarget::Item {
                    item: item_id.clone(),
                    side: if before_half { Side::Before } else { Side::After },
                    group: item_group.clone(),
                    before: next,
                };
                if this.drop_target.as_ref() != Some(&target) {
                    this.drop_target = Some(target);
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    /// 同一分组里排在 `id` 后面的那一项（落在下半边时插到它前面）。
    fn next_in_group(&self, group: &str, id: &str) -> Option<String> {
        let members: Vec<&StoredConnection> = self.connections.iter().filter(|c| groups::normalize(&c.group) == group).collect();
        let at = members.iter().position(|c| c.id == id)?;
        members.get(at + 1).map(|c| c.id.clone())
    }

    pub(crate) fn connection_menu_overlay(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let id = menu.connection_id.clone();
        let position = menu.position;
        let conn = self.connection(&id)?.clone();
        let connected = self.session(&id).is_some();
        let this = cx.entity().downgrade();
        let dismiss = {
            let this = this.clone();
            move |_: &mut Window, cx: &mut gpui::App| {
                let _ = this.update(cx, |this, cx| {
                    this.menu = None;
                    cx.notify();
                });
            }
        };
        let action = |f: fn(&mut Workspace, &str, &mut Window, &mut Context<Workspace>)| {
            let this = this.clone();
            let id = id.clone();
            move |window: &mut Window, cx: &mut gpui::App| {
                let _ = this.update(cx, |this, cx| f(this, &id, window, cx));
            }
        };
        let mut entries = vec![MenuEntry::Item(MenuItem::new("连接", action(|this, id, window, cx| this.connect_to_saved(id, window, cx))))];
        let editing = conn.clone();
        let edit_this = this.clone();
        entries.push(MenuEntry::Item(
            MenuItem::new("编辑", move |window, cx| {
                let editing = editing.clone();
                let _ = edit_this.update(cx, |this, cx| this.open_connection_dialog(Some(editing), window, cx));
            })
            .disabled(connected),
        ));
        if connected {
            entries.push(MenuEntry::Item(MenuItem::new("断开连接", action(|this, id, window, cx| this.disconnect(id, window, cx)))));
        }
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Item(MenuItem::new("删除", action(|this, id, window, cx| this.delete_connection(id, window, cx))).destructive().disabled(connected)));
        let panel = kit::menu_panel(entries, dismiss);
        Some(
            div()
                .id("menu-catcher")
                .absolute()
                .inset_0()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }))
                .child(deferred(anchored().position(position).snap_to_window().child(panel)))
                .into_any_element(),
        )
    }
}

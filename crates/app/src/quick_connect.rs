//! 快速连接（⌘K）：只搜保存的连接，↑↓ 选、↵ 连（已开着的就切过去），Esc 关。
//! 侧栏整个收起后不用展开也能连另一台。

use std::collections::HashSet;

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions, div,
    prelude::FluentBuilder, px,
};
use gpui_base::input::{Input, InputEvent, InputState};
use sshl_core::config_store::StoredConnection;

use crate::assets::Icon;
use crate::groups;
use crate::settings;
use crate::theme::{self, ui};
use crate::ui as kit;

actions!(quick_connect, [QuickPrev, QuickNext, QuickOpen, QuickClose]);

pub fn bind_keys(cx: &mut App) {
    // 绑在比 gpui-base 输入框更深的上下文里，压过它自己的 上 / 下 / 回车 / Esc
    cx.bind_keys([
        KeyBinding::new("up", QuickPrev, Some("QuickConnect > Input")),
        KeyBinding::new("down", QuickNext, Some("QuickConnect > Input")),
        KeyBinding::new("enter", QuickOpen, Some("QuickConnect > Input")),
        KeyBinding::new("escape", QuickClose, Some("QuickConnect > Input")),
    ]);
}

// ---------------------------------------------------------------- 纯逻辑

/// 侧栏里的顺序：未分组在前，再按分组顺序，组内按各自的排序。
pub fn sidebar_order(connections: &[StoredConnection], group_order: &[String]) -> Vec<StoredConnection> {
    let refs: Vec<&StoredConnection> = connections.iter().collect();
    let mut ordered: Vec<StoredConnection> = groups::ungrouped(&refs).into_iter().cloned().collect();
    for group in groups::group_connections(&refs, group_order) {
        ordered.extend(group.connections.into_iter().cloned());
    }
    ordered
}

/// 面板最高时的高度：搜索行 + 分隔线 + 列表上限 380 + 内边距。
const PANEL_MAX_HEIGHT: f32 = 440.;

/// 面板顶边离窗口顶的距离：窗口高度的 30%，搜索框落在中线上方一点、列表往下展开。
/// 定的是顶边而不是整体居中：敲字过滤、列表变短时搜索框不跟着上下跳。
/// 矮窗口里往上收，让最高的面板底下还留 24；再矮也不低于 72。
pub fn top_offset(window_height: f32) -> f32 {
    (window_height * 0.3).min(window_height - PANEL_MAX_HEIGHT - 24.).max(72.).round()
}

/// 按搜索词筛选：空格分开的每个词都要出现在名称 / 主机 / 用户 / 分组里（不分大小写）；
/// 名称以第一个词开头的排前面（敲开头几个字母就能回车），其余保持原顺序。返回下标。
pub fn filter_connections(connections: &[StoredConnection], query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut matched: Vec<usize> = connections
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let haystack = format!("{}\n{}\n{}\n{}", c.name, c.host, c.username, c.group).to_lowercase();
            terms.iter().all(|term| haystack.contains(term.as_str()))
        })
        .map(|(i, _)| i)
        .collect();
    if let Some(first) = terms.first() {
        // 稳定排序：开头匹配的提前，其余不动
        matched.sort_by_key(|&i| !connections[i].name.to_lowercase().starts_with(first.as_str()));
    }
    matched
}

// ---------------------------------------------------------------- 面板

pub enum QuickConnectEvent {
    /// 连这个保存的连接（已开着的由工作区切过去）。
    Connect(String),
    Close,
}

pub struct QuickConnect {
    connections: Vec<StoredConnection>,
    /// 已经开着会话的连接：行首图标带绿点。
    connected: HashSet<String>,
    search: Entity<InputState>,
    /// 筛选后列表里的位置。
    selected: usize,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<QuickConnectEvent> for QuickConnect {}

impl QuickConnect {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索连接…"));
        let subscription = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
        });
        Self {
            connections: Vec::new(),
            connected: HashSet::new(),
            search,
            selected: 0,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }

    /// 打开：换上最新的连接列表、清空搜索、焦点给搜索框。
    pub fn open(&mut self, connections: Vec<StoredConnection>, connected: HashSet<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.connections = connections;
        self.connected = connected;
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        let focus = self.search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn visible(&self, cx: &App) -> Vec<usize> {
        filter_connections(&self.connections, &self.search.read(cx).value())
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.visible(cx).len();
        if count == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn confirm(&mut self, position: usize, cx: &mut Context<Self>) {
        let visible = self.visible(cx);
        if let Some(conn) = visible.get(position).and_then(|&i| self.connections.get(i)) {
            cx.emit(QuickConnectEvent::Connect(conn.id.clone()));
        }
    }

    fn row(&self, position: usize, conn: &StoredConnection, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = position == self.selected;
        let connected = self.connected.contains(&conn.id);
        let address = if conn.username.is_empty() { conn.host.clone() } else { format!("{}@{}", conn.username, conn.host) };
        let group = groups::normalize(&conn.group).to_string();
        div()
            .id(("quick-connect", position))
            .role(Role::MenuItem)
            .aria_label(SharedString::from(conn.name.clone()))
            .h(px(42.))
            .px_2()
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(7.))
            .cursor_pointer()
            .when(selected, |row| row.bg(theme::color(ui::ACCENT)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.selected != position {
                    this.selected = position;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.confirm(position, cx)))
            .child(
                // 同侧栏：已连接是图标角上的绿点
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
                                .border_color(theme::color(if selected { ui::ACCENT } else { ui::CARD })),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).line_height(px(18.)).child(conn.name.clone()))
                    .child(
                        div()
                            .truncate()
                            .font_family(settings::DEFAULT_FONT_FAMILY)
                            .text_size(px(11.))
                            .line_height(px(15.))
                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                            .child(address),
                    ),
            )
            .when(!group.is_empty(), |row| {
                row.child(div().flex_shrink_0().max_w(px(120.)).truncate().text_size(px(11.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(group))
            })
    }
}

impl Render for QuickConnect {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible(cx);
        let muted = |text: &'static str| div().px_2().py_3().text_size(px(13.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(text);
        let list = if self.connections.is_empty() {
            muted("没有保存的连接").into_any_element()
        } else if visible.is_empty() {
            muted("没有匹配的连接").into_any_element()
        } else {
            let rows: Vec<_> = visible.iter().enumerate().map(|(position, &i)| self.row(position, &self.connections[i], cx).into_any_element()).collect();
            div().id("quick-connect-list").max_h(px(380.)).overflow_y_scroll().track_scroll(&self.scroll).flex().flex_col().children(rows).into_any_element()
        };
        div()
            .id("quick-connect")
            .key_context("QuickConnect")
            .role(Role::Dialog)
            .aria_label("快速连接")
            .occlude()
            .w(px(460.))
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &QuickPrev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &QuickNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &QuickOpen, _, cx| this.confirm(this.selected, cx)))
            .on_action(cx.listener(|_, _: &QuickClose, _, cx| cx.emit(QuickConnectEvent::Close)))
            .child(
                div()
                    .h(px(34.))
                    .pl_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .child(kit::icon_sized(Icon::Search, 14.).text_color(theme::color(ui::MUTED_FOREGROUND)))
                    .child(div().flex_1().min_w_0().child(Input::new(&self.search))),
            )
            .child(div().h(px(1.)).mx(px(-6.)).bg(theme::color(ui::BORDER)))
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: &str, name: &str, host: &str, user: &str, group: &str) -> StoredConnection {
        StoredConnection {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port: 22,
            username: user.into(),
            private_key: None,
            has_password: false,
            has_passphrase: false,
            group: group.into(),
            auth: Default::default(),
        }
    }

    fn sample() -> Vec<StoredConnection> {
        vec![
            conn("1", "akko (de-fra-cn)", "de-fra-cn.skyferry.net", "root", "k3s"),
            conn("2", "tiny", "192.168.1.8", "qfdk", "CDG"),
            conn("3", "de-fra-o3", "130.162.215.244", "root", "k3s"),
            conn("4", "deo1.qfdk.me", "deo1.qfdk.me", "ubuntu", "Oracle"),
            conn("5", "scratch", "10.0.0.5", "root", ""),
        ]
    }

    fn names(list: &[StoredConnection], picked: &[usize]) -> Vec<String> {
        picked.iter().map(|&i| list[i].name.clone()).collect()
    }

    #[test]
    fn panel_top_sits_a_little_above_the_middle() {
        assert_eq!(top_offset(1000.), 300.);
        assert_eq!(top_offset(1400.), 420.);
        // 矮窗口：最高的面板也要放得下，底下留 24
        assert_eq!(top_offset(600.), 136.);
        assert_eq!(top_offset(300.), 72.);
    }

    #[test]
    fn empty_query_lists_everything_in_order() {
        let list = sample();
        assert_eq!(filter_connections(&list, ""), [0, 1, 2, 3, 4]);
        assert_eq!(filter_connections(&list, "   "), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn every_term_must_match_name_host_user_or_group() {
        let list = sample();
        assert_eq!(names(&list, &filter_connections(&list, "TINY")), ["tiny"]);
        assert_eq!(names(&list, &filter_connections(&list, "192.168")), ["tiny"]);
        assert_eq!(names(&list, &filter_connections(&list, "ubuntu")), ["deo1.qfdk.me"]);
        assert_eq!(names(&list, &filter_connections(&list, "k3s o3")), ["de-fra-o3"]);
        assert!(filter_connections(&list, "nothing-like-this").is_empty());
    }

    #[test]
    fn names_starting_with_the_first_term_come_first() {
        let list = sample();
        // 「akko (de-fra-cn)」只是名字里含 de，排在名字以 de 开头的后面
        assert_eq!(names(&list, &filter_connections(&list, "de")), ["de-fra-o3", "deo1.qfdk.me", "akko (de-fra-cn)"]);
    }

    #[test]
    fn sidebar_order_puts_ungrouped_first_then_groups_in_order() {
        let list = sample();
        let order = vec!["CDG".to_string(), "Oracle".to_string(), "k3s".to_string()];
        let ordered: Vec<String> = sidebar_order(&list, &order).into_iter().map(|c| c.name).collect();
        assert_eq!(ordered, ["scratch", "tiny", "deo1.qfdk.me", "akko (de-fra-cn)", "de-fra-o3"]);
    }
}

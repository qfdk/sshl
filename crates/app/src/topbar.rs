//! 顶栏：会话标签（可拖动排序）+ 右侧的 终端 / 文件管理、填充密码、断开（旧 SessionTabs.tsx + App.tsx 顶栏）。

use gpui::{
    AnyElement, AnyView, App, AppContext, ClickEvent, Context, Div, DragMoveEvent, ElementId, Entity, Focusable, Font, FontWeight, Hsla,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription,
    TextRun, Window, actions, anchored, deferred, div, point, prelude::FluentBuilder, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::Icon;
use crate::groups::{self, Side};
use crate::keymap;
use crate::settings;
use crate::sidebar::{DragPreview, sidebar_toggle_button};
use crate::terminal::frame::Rgb;
use crate::theme::{self, term, ui};
use crate::ui::{self as kit, Variant};
use crate::workspace::{
    COMPACT_BAR_HEIGHT, COMPACT_TRAFFIC_LIGHTS_WIDTH, CONTENT_EDGE, ConnectionMenu, DraggedTab, MainTab, TOPBAR_TOP_BESIDE_TRAFFIC_LIGHTS,
    TRAFFIC_LIGHTS_WIDTH, TopPanel, Workspace, controls, drag_region, main_tabs_for,
};

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

/// 紧凑标题栏里标签、按钮的高度（细条 `COMPACT_BAR_HEIGHT` 上下各留 4pt）。
const COMPACT_CONTROL: f32 = 24.;

/// 紧凑标题栏铺在终端上时字和图标的颜色：同 Ghostty 的标题栏，深底用白、浅底用黑（不用终端前景色：
/// 前景是黄的、绿的配色上，标签会成一排彩色的字），再按不透明度分出层次。
fn ink(t: &term::Theme) -> Rgb {
    if t.is_light() { Rgb::hex(0x000000) } else { Rgb::hex(0xffffff) }
}

/// 当前标签的字。
const COMPACT_INK_ALPHA: f32 = 0.9;

/// 没选中的标签、图标：从 45% 不透明往上找，到叠在底色上对底色 ≥ 4.5:1 为止。
fn muted_alpha(t: &term::Theme) -> f32 {
    (9..18).map(|step| step as f32 * 0.05).find(|&a| contrast(blend(ink(t), t.background, a), t.background) >= 4.5).unwrap_or(COMPACT_INK_ALPHA)
}

/// 前景色按 `a` 的不透明度叠在底色上的样子。
fn blend(fg: Rgb, bg: Rgb, a: f32) -> Rgb {
    let mix = |f: u8, b: u8| (f32::from(f) * a + f32::from(b) * (1. - a)).round() as u8;
    Rgb { r: mix(fg.r, bg.r), g: mix(fg.g, bg.g), b: mix(fg.b, bg.b) }
}

/// WCAG 对比度。
fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (term::luminance(a), term::luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// 紧凑标题栏（设置 →「外观」，仿 Ghostty；侧栏收起时）的配色。铺在终端（和没会话时的空页面）上面时取终端配色，
/// 和终端连成一片，同 Ghostty 的标题栏；在文件管理 / 系统页上面取界面的颜色。
#[derive(Clone, Copy)]
pub(crate) struct CompactBar {
    /// 底色；None = 窗口底色（界面的）。
    bg: Option<Hsla>,
    fg: Hsla,
    /// 没选中的标签、图标。
    muted: Hsla,
    hover: Hsla,
    /// 当前标签、分段控件选中项、开着的面板按钮的底。
    selected: Hsla,
    /// 分段控件的槽。
    track: Hsla,
    /// 底色是深的：状态色（延迟、连接的点）取界面那套颜色的深色值，浅的取浅色值。
    dark: bool,
}

impl CompactBar {
    fn over_terminal(t: &term::Theme) -> Self {
        let ink = term::hsla(ink(t));
        let fg = ink.opacity(COMPACT_INK_ALPHA);
        Self {
            bg: Some(term::hsla(t.background)),
            fg,
            muted: ink.opacity(muted_alpha(t)),
            hover: ink.opacity(0.08),
            selected: ink.opacity(0.12),
            track: ink.opacity(0.06),
            dark: !t.is_light(),
        }
    }

    fn over_ui() -> Self {
        let fg = theme::color(ui::FOREGROUND);
        Self {
            bg: None,
            fg,
            muted: theme::color(ui::MUTED_FOREGROUND),
            hover: fg.opacity(0.06),
            selected: fg.opacity(0.1),
            track: fg.opacity(0.05),
            dark: theme::is_dark(),
        }
    }

    fn status(&self, token: ui::Token) -> Hsla {
        theme::color(token.resolve(self.dark))
    }
}

/// 紧凑标题栏里的图标按钮：24pt 见方、不带底，平常淡色，悬停加底；`on`（面板开着）时一直带底。
fn compact_icon_button(id: impl Into<ElementId>, label: SharedString, glyph: Icon, bar: CompactBar, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .relative()
        .size(px(COMPACT_CONTROL))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_color(if on { bar.fg } else { bar.muted })
        .when(on, |b| b.bg(bar.selected))
        .cursor_pointer()
        .hover(move |s| s.bg(if on { bar.selected } else { bar.hover }).text_color(bar.fg))
        .child(kit::icon_sized(glyph, 14.))
}

/// 按住 ⌘ 时浮在标签 / 按钮右上角的快捷键。紧凑标题栏里小一号、往下收：标签离窗口顶只有 4pt，原来的位置会被窗口顶边切掉，
/// 原来的大小在 24pt 高的标签上会盖住半个字。
fn hint(key: impl Into<SharedString>, visible: bool, compact: bool) -> AnyElement {
    let size = if compact { 14. } else { 16. };
    div()
        .absolute()
        .right(px(-6.))
        .top(px(if compact { -4. } else { -8. }))
        .h(px(size))
        .min_w(px(size))
        .px(px(if compact { 3. } else { 4. }))
        .rounded(px(if compact { 4. } else { 5. }))
        .bg(theme::color(ui::PRIMARY))
        .text_color(theme::color(ui::PRIMARY_FOREGROUND))
        .text_size(px(if compact { 9. } else { 10. }))
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
        // 本机标签没有文件管理（只切 终端 / 系统）、没有填充密码
        let local = self.active_session().is_some_and(|s| s.local);
        let settings = settings::get(cx);
        let sidebar_collapsed = settings.sidebar_collapsed;
        // 紧凑标题栏：底下是终端（或没会话时同样是终端底色的空页面）就取终端配色，文件管理 / 系统页上取界面的
        let bar = settings.compact_bar().then(|| {
            if self.active_session().is_none() || self.main_tab == MainTab::Terminal {
                CompactBar::over_terminal(settings.terminal_theme())
            } else {
                CompactBar::over_ui()
            }
        });
        let compact = bar.is_some();
        let fullscreen = window.is_fullscreen();
        drag_region("topbar")
            .map(|el| match bar {
                // 紧凑：和红绿灯同一行的细条（同 Ghostty 的标题栏），标签、按钮 24pt 高、上下各留 4pt；
                // 标签多了换行时每行之间留 4pt，细条跟着长高（字不截断）
                Some(bar) => el
                    .min_h(px(COMPACT_BAR_HEIGHT))
                    .py(px((COMPACT_BAR_HEIGHT - COMPACT_CONTROL) / 2.))
                    .gap_x(px(8.))
                    .gap_y(px(4.))
                    .text_size(px(13.))
                    .text_color(bar.fg)
                    .when_some(bar.bg, |el, bg| el.bg(bg)),
                // 窗口顶上到标签之间那条也属于拖动区；侧栏收起时顶栏和红绿灯同一行，上边距收到让两者竖直居中对齐
                None => el.map(|el| if sidebar_collapsed && !fullscreen { el.pt(px(TOPBAR_TOP_BESIDE_TRAFFIC_LIGHTS)) } else { el.pt_3() }).gap(px(10.)),
            })
            // 内容区（终端页）贴满窗口，两边的留白只给顶栏；侧栏展开时左边紧挨侧栏只留 4px
            .pr(px(CONTENT_EDGE))
            .map(|el| if sidebar_collapsed { el.pl(px(CONTENT_EDGE)) } else { el.pl_1() })
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            // 侧栏收起后整列让给终端，红绿灯右边只留展开按钮（全屏时红绿灯藏起来，不用让位）。
            // 新建连接不再放一个：有标签时标签栏末尾有 +，没标签时页面中间有「新建连接」
            .when(sidebar_collapsed, |el| {
                let lights = if compact { COMPACT_TRAFFIC_LIGHTS_WIDTH } else { TRAFFIC_LIGHTS_WIDTH };
                let toggle = match bar {
                    Some(bar) => compact_icon_button("toggle-sidebar", tr!("展开侧边栏").into(), Icon::PanelLeft, bar, false)
                        .tooltip(|window, cx| kit::tooltip(keymap::with_shortcut(cx, tr!("展开侧边栏"), "toggle-sidebar"), window, cx))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx))),
                    None => sidebar_toggle_button(true, cx),
                };
                el.child(div().flex_shrink_0().when(!fullscreen, |d| d.pl(px(lights - CONTENT_EDGE))).child(controls("topbar-expand").child(toggle)))
            })
            .child(self.session_tabs(bar, window, cx))
            .child(
                controls("topbar-actions")
                    .ml_auto()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(if compact { 6. } else { 8. }))
                    .when_some(self.latency.clone().filter(|(id, _)| connected && self.active.as_ref() == Some(id)), |r, (_, rtt)| {
                        let (text, color) = latency_text(rtt);
                        r.child(
                            div()
                                .min_w(px(52.))
                                .flex()
                                .justify_end()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(bar.map_or_else(|| theme::color(color), |bar| bar.status(color)))
                                .child(text),
                        )
                    })
                    .when(connected, |r| r.child(self.panel_button(TopPanel::Snippets, bar, cx)).child(self.panel_button(TopPanel::Favorites, bar, cx)))
                    .when(connected, |r| r.child(self.main_tabs(local, bar, cx)))
                    .when(connected && !local && (self.fill_feedback || self.active_session().is_some_and(|s| s.view.read(cx).password_prompt())), |r| {
                        r.child(self.fill_button(compact, cx))
                    })
                    .when_some(self.fill_menu_panel(compact, window, cx), |r, panel| r.child(panel)),
            )
            .into_any_element()
    }

    /// 终端 / 文件管理 / 系统（本机标签没有文件管理）。
    fn main_tabs(&self, local: bool, bar: Option<CompactBar>, cx: &mut Context<Self>) -> AnyElement {
        let hints = self.hint_visible;
        let this = cx.entity().downgrade();
        let tabs = main_tabs_for(local);
        let control = match bar {
            None => {
                let options: Vec<(MainTab, Option<Icon>, SharedString)> = tabs.iter().map(|&tab| (tab, None, main_tab_label(tab).into())).collect();
                kit::segmented("main-tabs", &options, self.main_tab, move |tab, window, cx| {
                    let _ = this.update(cx, |this, cx| this.set_main_tab(tab, window, cx));
                })
            }
            // 紧凑标题栏里的小号分段控件：槽和选中项都是半透明的前景色，铺在终端配色上也看得清
            Some(bar) => div().id("main-tabs").role(Role::TabList).h(px(COMPACT_CONTROL)).p(px(2.)).flex().gap(px(2.)).rounded(px(6.)).bg(bar.track).children(
                tabs.iter().enumerate().map(|(index, &tab)| {
                    let active = tab == self.main_tab;
                    let label = main_tab_label(tab);
                    let this = this.clone();
                    div()
                        .id(("main-tabs", index))
                        .role(Role::Tab)
                        .aria_label(label)
                        .h_full()
                        .px(px(10.))
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .map(|t| {
                            if active {
                                t.bg(bar.selected).text_color(bar.fg).font_weight(FontWeight::MEDIUM)
                            } else {
                                t.text_color(bar.muted).hover(move |s| s.text_color(bar.fg))
                            }
                        })
                        .child(label)
                        .on_click(move |_: &ClickEvent, window, cx| {
                            if !active {
                                let _ = this.update(cx, |this, cx| this.set_main_tab(tab, window, cx));
                            }
                        })
                }),
            ),
        };
        div()
            .relative()
            .child(control)
            // 快捷键角标（⌘I / ⌘O / ⌘P）浮在每个选项的右上角（与旧版一样不挤动布局）；选项等宽，叠一层同样等分的格子放角标
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .p(px(if bar.is_some() { 2. } else { 3. }))
                    .flex()
                    .gap(px(2.))
                    .children(tabs.iter().map(|&tab| {
                        div()
                            .relative()
                            .flex_1()
                            .h_full()
                            .when_some(keymap::hint(cx, main_tab_command(tab)), |slot, key| slot.child(hint(key, hints, bar.is_some())))
                    })),
            )
            .into_any_element()
    }

    /// 顶栏的面板按钮（命令片段 ⌘⇧P、目录收藏 ⌘⇧D）和它下面的面板。
    fn panel_button(&self, kind: TopPanel, bar: Option<CompactBar>, cx: &mut Context<Self>) -> AnyElement {
        let (id, label, glyph, command, content): (&'static str, &'static str, Icon, &'static str, AnyView) = match kind {
            TopPanel::Snippets => ("snippets", tr!("命令片段"), Icon::CodeXml, "show-snippets", self.snippet_panel.clone().into()),
            TopPanel::Favorites => ("favorites", tr!("目录收藏"), Icon::Star, "show-favorites", self.favorite_panel.clone().into()),
        };
        let key = keymap::hint(cx, command);
        let open = self.top_panel == Some(kind);
        let button = match bar {
            Some(bar) => compact_icon_button(id, label.into(), glyph, bar, open),
            None => div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
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
                .child(kit::icon_sized(glyph, 16.)),
        };
        // 和「终端 / 文件管理 / 系统」的 I / O / P 同一个基准：分段控件里的选项内缩 3px（紧凑时 2px），角标相对选项定位
        let inset = if bar.is_some() { 2. } else { 3. };
        let button = button
            .when(!self.hint_visible, |b| b.tooltip(move |window, cx| kit::tooltip(label.into(), window, cx)))
            .when_some(key, |b, key| {
                b.child(div().absolute().inset(px(inset)).child(div().relative().size_full().child(hint(key, self.hint_visible, bar.is_some()))))
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, _| {
                // 从一个按钮滑到另一个：前一个的「移出」可能晚到，别把后一个的记号清掉
                if *hovered {
                    this.top_panel_hovered = Some(kind);
                } else if this.top_panel_hovered == Some(kind) {
                    this.top_panel_hovered = None;
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.toggle_top_panel(kind, window, cx)));
        let panel = open.then(|| {
            let this = cx.entity().downgrade();
            div().absolute().right_0().top(px(if bar.is_some() { COMPACT_CONTROL + 6. } else { 40. })).child(
                deferred(
                    anchored().anchor(gpui::Anchor::TopRight).child(
                        div()
                            .on_mouse_down_out(move |_, window, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    // 点的是这个面板自己的按钮：交给按钮的点击去关，不然这里先关、按钮又打开
                                    if this.top_panel_hovered != Some(kind) {
                                        this.close_top_panel(window, cx);
                                    }
                                });
                            })
                            .child(content),
                    ),
                )
                .with_priority(2),
            )
        });
        div().relative().child(button).when_some(panel, |d, panel| d.child(panel)).into_any_element()
    }

    fn fill_button(&self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.fill_feedback { tr!("填充发送完毕") } else { tr!("填充密码") };
        // 紧凑标题栏里同别的控件一样矮一号
        let (height, radius, text, glyph) = if compact { (COMPACT_CONTROL, 6., 12., 12.) } else { (34., 10., 14., 14.) };
        div()
            .flex()
            .items_center()
            .child(
                div()
                    .id("fill-password")
                    .role(Role::Button)
                    .aria_label(tr!("填充密码"))
                    .h(px(height))
                    .pl(px(if compact { 8. } else { 12. }))
                    .pr(px(if compact { 8. } else { 10. }))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_l(px(radius))
                    .bg(theme::color(ui::PRIMARY))
                    .text_color(theme::color(ui::PRIMARY_FOREGROUND))
                    .text_size(px(text))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::alpha(ui::PRIMARY, 0.9)))
                    .child(kit::icon_sized(Icon::KeyRound, glyph))
                    .child(label)
                    .on_click(cx.listener(|this, _, window, cx| this.fill_password_primary(window, cx))),
            )
            .child(
                div()
                    .id("fill-password-menu")
                    .role(Role::Button)
                    .aria_label(tr!("选择账号密码"))
                    .h(px(height))
                    .px(px(if compact { 4. } else { 6. }))
                    .flex()
                    .items_center()
                    .rounded_r(px(radius))
                    .border_l_1()
                    .border_color(theme::alpha(ui::PRIMARY_FOREGROUND, 0.25))
                    .bg(theme::color(ui::PRIMARY))
                    .text_color(theme::color(ui::PRIMARY_FOREGROUND))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::alpha(ui::PRIMARY, 0.9)))
                    .child(kit::icon_sized(Icon::ChevronDown, glyph))
                    .on_click(cx.listener(|this, _, window, cx| this.open_fill_menu(window, cx))),
            )
            .into_any_element()
    }

    fn fill_menu_panel(&self, compact: bool, _: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
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
            items.push(row("fill-main".into(), tr!("连接密码").into(), "password".into(), cx));
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
                    .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(tr!("新增账号密码")))
                    .child(kit::text_field(&menu.account, false).h(px(32.)).text_xs())
                    .child(kit::text_field(&menu.password, false).h(px(32.)).text_xs())
                    .child(
                        kit::button("fill-save", Variant::Primary)
                            .h(px(30.))
                            .text_xs()
                            .child(tr!("保存"))
                            .on_click(cx.listener(|this, _, window, cx| this.save_fill_account(window, cx))),
                    ),
            );
        Some(
            div()
                .absolute()
                .right_0()
                .top(px(if compact { COMPACT_CONTROL + 6. } else { 40. }))
                .child(deferred(anchored().anchor(gpui::Anchor::TopRight).offset(point(px(0.), px(0.))).child(panel)))
                .into_any_element(),
        )
    }

    fn session_tabs(&mut self, bar: Option<CompactBar>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.sessions.is_empty() {
            return div().into_any_element();
        }
        let hints = self.hint_visible;
        let compact = bar.is_some();
        let tab_keys: Vec<Option<String>> = (1..=9).map(|n| keymap::hint(cx, &format!("select-tab-{n}"))).collect();
        let tabs: Vec<AnyElement> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                let id = session.connection_id.clone();
                let active = self.active.as_deref() == Some(id.as_str());
                let name = self.tab_name(session);
                // 标签上只有名字；主机地址悬停时才提示（名字本身就是地址的不重复）
                let host: SharedString = self.connection(&id).map(|c| groups::distinct_host(&name, &c.host).to_string()).unwrap_or_default().into();
                let mark = match &self.tab_drop {
                    Some((target, side)) if *target == id => Some(*side),
                    _ => None,
                };
                let dragging = self.dragging.as_deref() == Some(id.as_str()) && cx.has_active_drag();
                let disconnected = session.view.read(cx).is_disconnected();
                let close_label = SharedString::from(format!("{} {name}", if session.local { tr!("关闭") } else { tr!("断开") }));
                let (click_id, close_id, drag_id, move_id, drop_name) = (id.clone(), id.clone(), id.clone(), id.clone(), name.clone());
                let rename = self.renaming.as_ref().filter(|r| r.connection_id == id);
                let label = match rename {
                    Some(rename) => self.rename_field(rename, compact, window, cx),
                    // 名字一行写全，标签跟着变宽（标签栏放不下会换到下一行）
                    None => div()
                        .whitespace_nowrap()
                        .map(|t| match (bar, active) {
                            (Some(bar), true) => t.text_color(bar.fg).font_weight(FontWeight::MEDIUM),
                            (Some(bar), false) => t.text_color(bar.muted),
                            (None, true) => t.font_weight(FontWeight::SEMIBOLD),
                            (None, false) => t.text_color(theme::alpha(ui::FOREGROUND, 0.8)),
                        })
                        .child(name.clone())
                        .into_any_element(),
                };
                let tab = div()
                    .id(SharedString::from(format!("tab-{id}")))
                    .role(Role::Tab)
                    .aria_label(name.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .map(|t| match bar {
                        // 紧凑：不描边、不带阴影，当前标签只铺一层半透明的底（同 Ghostty / Safari 的紧凑标签）
                        Some(bar) => t.h(px(COMPACT_CONTROL)).rounded(px(6.)).map(|t| if active { t.bg(bar.selected) } else { t.hover(move |s| s.bg(bar.hover)) }),
                        None => t.h(px(34.)).rounded(px(10.)).border_1().map(|t| {
                            if active {
                                t.border_color(theme::color(ui::BORDER)).bg(theme::color(ui::CARD)).shadow_sm()
                            } else {
                                t.border_color(gpui::transparent_black()).hover(|s| s.bg(theme::alpha(ui::FOREGROUND, 0.04)))
                            }
                        }),
                    })
                    .when(dragging, |t| t.opacity(0.5))
                    // 落点竖线：看得出会插到这个标签的左边还是右边
                    .when_some(mark, |t, side| {
                        t.child(
                            div()
                                .absolute()
                                .top(px(if compact { 3. } else { 4. }))
                                .bottom(px(if compact { 3. } else { 4. }))
                                .w(px(2.))
                                .rounded_full()
                                .bg(theme::color(ui::PRIMARY))
                                .map(|line| if side == Side::Before { line.left(px(-5.)) } else { line.right(px(-5.)) }),
                        )
                    })
                    .child(
                        div()
                            .id(("tab-body", index))
                            .h_full()
                            .flex()
                            .items_center()
                            .gap(px(if compact { 6. } else { 8. }))
                            .pl(px(if compact { 8. } else { 12. }))
                            .pr(px(if compact { 4. } else { 6. }))
                            .when(rename.is_none(), |t| t.cursor_pointer())
                            .child(div().size(px(if compact { 6. } else { 7. })).flex_shrink_0().rounded_full().bg(match bar {
                                Some(bar) if disconnected => bar.muted,
                                Some(bar) => bar.status(ui::SUCCESS),
                                None => theme::color(if disconnected { ui::MUTED_FOREGROUND } else { ui::SUCCESS }),
                            }))
                            .child(label)
                            .when(rename.is_none() && !host.is_empty(), |t| t.tooltip(move |window, cx| kit::tooltip(host.clone(), window, cx))),
                    )
                    .child(
                        match bar {
                            None => kit::mini_button(SharedString::from(format!("close-{id}")), Icon::X, close_label, ui::MUTED_FOREGROUND).mr_1(),
                            Some(bar) => div()
                                .id(SharedString::from(format!("close-{id}")))
                                .role(Role::Button)
                                .aria_label(close_label.clone())
                                .size(px(18.))
                                .mr(px(3.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .text_color(bar.muted)
                                .cursor_pointer()
                                .hover(move |s| s.bg(bar.hover).text_color(bar.fg))
                                .tooltip(move |window, cx| kit::tooltip(close_label.clone(), window, cx))
                                .child(kit::icon_sized(Icon::X, 12.)),
                        }
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.disconnect(&close_id, window, cx);
                        })),
                    )
                    // ⌘ 角标：前 9 个标签浮出切过去的快捷键（默认 ⌘1 … ⌘9，设置里改过的照改的）；正在改名的不显示。
                    // 延后画：gpui 的边框画在子元素之后，不延后的话当前标签的边框线会压在角标上
                    .when_some(tab_keys.get(index).cloned().flatten().filter(|_| rename.is_none()), |t, key| t.child(gpui::deferred(hint(key, hints, compact))));
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
            .aria_label(tr!("已打开的会话"))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(if compact { 4. } else { 6. }))
            .children(tabs)
            // 没有会话时侧栏的「新建连接」就够了，这里只在已有标签时提供 +
            .child(match bar {
                Some(bar) => compact_icon_button("tab-new", tr!("新建连接").into(), Icon::Plus, bar, false)
                    .tooltip(|window, cx| kit::tooltip(keymap::with_shortcut(cx, tr!("新建连接"), "new-connection"), window, cx))
                    .on_click(cx.listener(|this, _, window, cx| this.open_connection_dialog(None, window, cx))),
                None => div()
                    .id("tab-new")
                    .role(Role::Button)
                    .aria_label(tr!("新建连接"))
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
            })
            .into_any_element()
    }

    /// 改名中的标签里，名字换成的输入框。宽度跟着字走（夹在最小 / 最大之间），换上去时标签只宽出输入框的内边距。
    fn rename_field(&self, rename: &TabRename, compact: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let text = rename.input.read(cx).value();
        let style = window.text_style();
        let size = if compact { 13. } else { 14. };
        let text_width = if text.is_empty() {
            0.
        } else {
            let font = Font { weight: FontWeight::SEMIBOLD, ..style.font() };
            let run = TextRun { len: text.len(), font, color: style.color, background_color: None, underline: None, strikethrough: None };
            f32::from(window.text_system().shape_line(text, px(size), &[run], None).width)
        };
        // 左右内边距 12 + 边框 2 + 光标留一点
        let width = (text_width + 20.).clamp(RENAME_MIN_WIDTH, RENAME_MAX_WIDTH);
        let input = rename.input.clone();
        div()
            .id("tab-rename")
            .key_context(RENAME_CONTEXT)
            .role(Role::TextInput)
            .aria_label(tr!("重命名标签"))
            .w(px(width))
            .h(px(if compact { 20. } else { 26. }))
            .px(px(6.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(theme::color(ui::RING))
            .bg(theme::color(ui::CARD))
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(size))
            .line_height(px(if compact { 18. } else { 20. }))
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

fn main_tab_label(tab: MainTab) -> &'static str {
    match tab {
        MainTab::Terminal => tr!("终端"),
        MainTab::Files => tr!("文件管理"),
        MainTab::System => tr!("系统"),
    }
}

/// 切到这一页的命令（`keymap::COMMANDS` 里的 id），角标读它的实际绑定。
fn main_tab_command(tab: MainTab) -> &'static str {
    match tab {
        MainTab::Terminal => "show-terminal",
        MainTab::Files => "show-files",
        MainTab::System => "show-system",
    }
}

/// 延迟的显示：< 100ms 绿、≤ 200ms 黄、更慢或超时红。
pub(crate) fn latency_text(rtt: Option<std::time::Duration>) -> (String, ui::Token) {
    match rtt {
        None => (tr!("超时").into(), ui::DESTRUCTIVE),
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
        gpui::actions!(topbar_test, [InputEnter, InputEscape]);
        // 同 gpui-base：输入框自己的回车 / Esc 先绑（main.rs 里 gpui_base::init 在前）
        let mut keymap = Keymap::new(vec![
            KeyBinding::new("enter", InputEnter, Some("Input")),
            KeyBinding::new("escape", InputEscape, Some("Input")),
        ]);
        keymap.add_bindings(crate::keymap::default_bindings());
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
        assert_eq!(bindings.first().map(|b| b.action().name()), Some("topbar_test::InputEnter"));
    }

    /// 紧凑标题栏铺在终端上时按终端底色画：每套配色上，当前标签和没选中的标签的字对底色都 ≥ 4.5:1，而且没选中的比当前的淡；
    /// 延迟、连接的点（界面那套状态色，按底色深浅取浅色 / 深色值）不比在界面自己的底上难认。
    #[test]
    fn compact_bar_is_readable_on_every_terminal_theme() {
        for t in term::THEMES {
            let bg = t.background;
            let alpha = muted_alpha(t);
            let (current, muted) = (blend(ink(t), bg, COMPACT_INK_ALPHA), blend(ink(t), bg, alpha));
            assert!(contrast(current, bg) >= 4.5, "{}：当前标签 {:.2}:1", t.name, contrast(current, bg));
            assert!(contrast(muted, bg) >= 4.5, "{}：没选中的标签 {:.2}:1", t.name, contrast(muted, bg));
            assert!(alpha < COMPACT_INK_ALPHA, "{}：没选中的标签和当前标签一样亮", t.name);
            // 状态色按底色深浅取界面那套的浅色 / 深色值：在终端底上不比在界面自己的底上难认（界面浅色下的绿也只有 2.3:1）
            let bar = CompactBar::over_terminal(t);
            let shell = Rgb::hex(ui::SHELL.resolve(bar.dark));
            for (name, token) in [("绿", ui::SUCCESS), ("黄", ui::WARNING), ("红", ui::DESTRUCTIVE)] {
                let color = Rgb::hex(token.resolve(bar.dark));
                let (here, in_ui) = (contrast(color, bg), contrast(color, shell));
                assert!(here >= in_ui.min(3.) - 0.1, "{}：状态色{name} {here:.2}:1（界面里 {in_ui:.2}:1）", t.name);
            }
        }
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

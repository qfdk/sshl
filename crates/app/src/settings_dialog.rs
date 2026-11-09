//! 设置（旧 SettingsDialog.tsx + GroupTree.tsx）：终端字号 / 字体（实时预览），分组管理（改完要点保存）。

use std::collections::HashSet;

use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, deferred, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{InputEvent, InputState};
use sshl_core::config_store::{self, ConnectionLayoutInput, StoredConnection};

use crate::assets::Icon;
use crate::backend;
use crate::groups::{self, Side};
use crate::prompts::{self, Options};
use crate::settings::{self, DEFAULT_FONT_FAMILY, DEFAULT_FONT_SIZE, Settings};
use crate::sidebar::DragPreview;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

const UNGROUPED: &str = "未分组";
const GROUP_NERD: &str = "图标字体（Nerd Font）";
const GROUP_MONO: &str = "等宽字体";
const NERD_CANDIDATES: &[&str] = &[
    "JetBrainsMono Nerd Font Mono", "JetBrainsMono Nerd Font", "FiraCode Nerd Font Mono", "FiraCode Nerd Font",
    "Hack Nerd Font Mono", "Hack Nerd Font", "MesloLGM Nerd Font Mono", "MesloLGM Nerd Font", "Iosevka Nerd Font Mono",
    "Iosevka Nerd Font", "CaskaydiaCove Nerd Font Mono", "CaskaydiaCove Nerd Font", "Symbols Nerd Font Mono",
    "Symbols Nerd Font",
];
const MONO_CANDIDATES: &[&str] = &[
    "Menlo", "Monaco", "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", "Cascadia Mono", "Hack", "Iosevka",
    "Source Code Pro", "IBM Plex Mono", "Roboto Mono", "Inconsolata", "Ubuntu Mono", "Anonymous Pro", "PT Mono",
    "Noto Sans Mono", "Courier New", "Consolas",
];

#[derive(Clone)]
struct Preset {
    label: SharedString,
    family: String,
    group: &'static str,
}

fn presets(installed: &[String]) -> Vec<Preset> {
    let has = |name: &str| installed.iter().any(|f| f == name);
    let mut out = vec![Preset { label: "JetBrainsMono Nerd Font (内置·含图标)".into(), family: DEFAULT_FONT_FAMILY.into(), group: GROUP_NERD }];
    for name in NERD_CANDIDATES.iter().filter(|n| has(n) && **n != DEFAULT_FONT_FAMILY) {
        out.push(Preset { label: format!("{name} (含图标)").into(), family: (*name).into(), group: GROUP_NERD });
    }
    for name in MONO_CANDIDATES.iter().filter(|n| has(n)) {
        out.push(Preset { label: (*name).into(), family: (*name).into(), group: GROUP_MONO });
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Font,
    Groups,
}

pub enum SettingsDialogEvent {
    Close,
}

#[derive(Clone)]
struct DraggedMember {
    id: String,
    name: SharedString,
}

#[derive(Clone)]
struct DraggedGroup {
    name: String,
}

#[derive(Clone, PartialEq)]
enum GroupDrop {
    /// 落进分组（末尾）。
    Into(String),
    /// 放在同组某一项的前面 / 后面。
    Beside { group: String, item: String, side: Side, before: Option<String> },
    /// 分组排序：放在某个分组前面 / 后面。
    Order { target: String, side: Side },
}

pub struct SettingsDialog {
    tab: Tab,
    /// 打开时的设置：关掉不保存就换回去（预览会改到所有终端）。
    snapshot: Settings,
    presets: Vec<Preset>,
    family_open: bool,
    custom: Entity<InputState>,
    custom_mode: bool,
    connections: Vec<StoredConnection>,
    groups: Vec<String>,
    saved: (Vec<(String, String)>, Vec<String>),
    new_group: Entity<InputState>,
    renames: Vec<(String, Entity<InputState>)>,
    expanded: HashSet<String>,
    drop: Option<GroupDrop>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsDialogEvent> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsDialog {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let snapshot = settings::get(cx).clone();
        let custom = cx.new(|cx| InputState::new(window, cx).placeholder("Cascadia Code").default_value(snapshot.font_family.clone()));
        let new_group = cx.new(|cx| InputState::new(window, cx).placeholder("新分组名称"));
        let subscriptions = vec![
            cx.subscribe_in(&custom, window, |this, input, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event
                    && this.custom_mode {
                        let family = input.read(cx).value().to_string();
                        settings::preview(cx, |s| s.font_family = if family.trim().is_empty() { DEFAULT_FONT_FAMILY.into() } else { family });
                    }
            }),
            cx.subscribe_in(&new_group, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.add_group(window, cx);
                }
            }),
        ];
        let mut this = Self {
            tab: Tab::Font,
            snapshot,
            presets: presets(&[]),
            family_open: false,
            custom,
            custom_mode: false,
            connections: Vec::new(),
            groups: Vec::new(),
            saved: (Vec::new(), Vec::new()),
            new_group,
            renames: Vec::new(),
            expanded: HashSet::new(),
            drop: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.custom_mode = !this.presets.iter().any(|p| p.family == this.snapshot.font_family);
        this.load_groups(window, cx);
        // 枚举系统字体约 100ms，放后台
        let fonts = backend::spawn(async { sshl_core::system_fonts::list_system_fonts().await.unwrap_or_default() });
        cx.spawn(async move |this, cx| {
            let installed = fonts.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.presets = presets(&installed);
                let family = settings::get(cx).font_family.clone();
                this.custom_mode = !this.presets.iter().any(|p| p.family == family);
                cx.notify();
            });
        })
        .detach();
        window.focus(&this.focus, cx);
        this
    }

    fn layout_of(connections: &[StoredConnection], groups: &[String]) -> (Vec<(String, String)>, Vec<String>) {
        (connections.iter().map(|c| (c.id.clone(), groups::normalize(&c.group).to_string())).collect(), groups.to_vec())
    }

    fn load_groups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connections = config_store::connections().unwrap_or_default();
        self.groups = config_store::groups().unwrap_or_default().into_iter().map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect();
        self.saved = Self::layout_of(&self.connections, &self.groups);
        self.rebuild_renames(window, cx);
    }

    fn rebuild_renames(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut renames = Vec::new();
        for name in &self.groups {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
            let old = name.clone();
            self._subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    let value = input.read(cx).value().to_string();
                    this.rename_group(&old, &value, window, cx);
                }
            }));
            renames.push((name.clone(), input));
        }
        self.renames = renames;
    }

    fn dirty(&self) -> bool {
        Self::layout_of(&self.connections, &self.groups) != self.saved
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        // 预览过的字体 / 字号没保存就换回去
        let snapshot = self.snapshot.clone();
        let current = settings::get(cx).clone();
        if current.font_size != snapshot.font_size || current.font_family != snapshot.font_family {
            settings::replace(cx, Settings { font_size: snapshot.font_size, font_family: snapshot.font_family, ..current });
        }
        cx.emit(SettingsDialogEvent::Close);
    }

    fn save_font(&mut self, cx: &mut Context<Self>) {
        settings::persist(cx);
        self.snapshot = settings::get(cx).clone();
        cx.emit(SettingsDialogEvent::Close);
    }

    fn step_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        settings::preview(cx, |s| s.font_size = (s.font_size + delta).round());
        cx.notify();
    }

    fn pick_family(&mut self, family: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.family_open = false;
        match family {
            Some(family) => {
                self.custom_mode = false;
                settings::preview(cx, |s| s.font_family = family);
            }
            None => {
                self.custom_mode = true;
                let current = settings::get(cx).font_family.clone();
                self.custom.update(cx, |input, cx| input.set_value(current, window, cx));
                let focus = self.custom.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
        }
        cx.notify();
    }

    fn reset_font(&mut self, cx: &mut Context<Self>) {
        // 只回到默认值并刷新预览，不落盘：和其他改动一样要按保存才生效，误点还能直接关掉
        self.custom_mode = false;
        settings::preview(cx, |s| {
            s.font_size = DEFAULT_FONT_SIZE;
            s.font_family = DEFAULT_FONT_FAMILY.into();
        });
        cx.notify();
    }

    fn add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_group.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        if self.groups.contains(&name) {
            prompts::alert_later(window, cx, Options::new("无法保存分组").message("分组名称已存在。"));
            return;
        }
        self.groups.push(name);
        self.new_group.update(cx, |input, cx| input.set_value("", window, cx));
        self.rebuild_renames(window, cx);
        cx.notify();
    }

    fn rename_group(&mut self, old: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let new = value.trim();
        if new.is_empty() || new == old || self.groups.iter().any(|g| g == new) {
            return;
        }
        for g in &mut self.groups {
            if g == old {
                *g = new.to_string();
            }
        }
        for c in &mut self.connections {
            if groups::normalize(&c.group) == old {
                c.group = new.to_string();
            }
        }
        self.rebuild_renames(window, cx);
        cx.notify();
    }

    fn remove_group(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.groups.retain(|g| g != name);
        for c in &mut self.connections {
            if groups::normalize(&c.group) == name {
                c.group.clear();
            }
        }
        self.rebuild_renames(window, cx);
        cx.notify();
    }

    fn save_groups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let layout = self.connections.iter().map(|c| ConnectionLayoutInput { id: c.id.clone(), group: groups::normalize(&c.group).to_string() }).collect();
        match config_store::apply_connection_layout(&backend::get().events, layout, Some(self.groups.clone())) {
            Ok(()) => {
                self.saved = Self::layout_of(&self.connections, &self.groups);
                cx.emit(SettingsDialogEvent::Close);
            }
            Err(error) => prompts::alert_later(window, cx, Options::new("保存分组失败").message(error.to_string())),
        }
    }

    fn apply_drop(&mut self, dragged_member: Option<&str>, dragged_group: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drop) = self.drop.take() else { return };
        match (drop, dragged_member, dragged_group) {
            (GroupDrop::Into(group), Some(id), _) => {
                self.connections = groups::move_connection(&self.connections, &self.groups, id, &group, None);
            }
            (GroupDrop::Beside { group, before, .. }, Some(id), _) => {
                self.connections = groups::move_connection(&self.connections, &self.groups, id, &group, before.as_deref());
            }
            (GroupDrop::Order { target, side }, _, Some(name)) => {
                self.groups = groups::reorder_groups(&self.groups, name, &target, side == Side::After);
                self.rebuild_renames(window, cx);
            }
            _ => {}
        }
        cx.notify();
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drop = None;
        }
        let tab_button = |id: &'static str, tab: Tab, glyph: Icon, label: &'static str, active: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .role(Role::Tab)
                .aria_label(label)
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(8.))
                .border_1()
                .cursor_pointer()
                .map(|b| if active { b.bg(theme::color(ui::CARD)).border_color(theme::color(ui::BORDER)) } else { b.border_color(gpui::transparent_black()).hover(|s| s.bg(theme::color(ui::CARD))) })
                .child(kit::icon(glyph))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    cx.notify();
                }))
        };
        let panel = match self.tab {
            Tab::Font => self.font_panel(window, cx),
            Tab::Groups => self.groups_panel(window, cx),
        };
        let body = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    if this.family_open {
                        this.family_open = false;
                        cx.notify();
                    } else {
                        this.close(cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .py(px(10.))
                    .pl_5()
                    .pr_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme::color(ui::BORDER))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("设置"))
                    .child(kit::icon_button("settings-close", Variant::Ghost, Icon::X, "关闭").w(px(32.)).h(px(32.)).on_click(cx.listener(|this, _, _, cx| this.close(cx)))),
            )
            .child(
                div()
                    .flex()
                    .min_h(px(352.))
                    .child(
                        div()
                            .w(px(144.))
                            .flex_shrink_0()
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .bg(theme::color(ui::SHELL))
                            .rounded_bl(px(15.))
                            .border_r_1()
                            .border_color(theme::color(ui::BORDER))
                            .child(tab_button("settings-tab-font", Tab::Font, Icon::Type, "字体", self.tab == Tab::Font, cx))
                            .child(tab_button("settings-tab-groups", Tab::Groups, Icon::Folder, "分组管理", self.tab == Tab::Groups, cx)),
                    )
                    .child(div().flex_1().min_w_0().p_5().child(panel)),
            );
        kit::modal("settings-dialog", 768., body)
    }
}

impl SettingsDialog {
    fn font_panel(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let current = settings::get(cx).clone();
        let selected = self.presets.iter().find(|p| p.family == current.font_family && !self.custom_mode).map(|p| p.label.clone()).unwrap_or_else(|| "自定义…".into());
        let family_list = self.family_open.then(|| {
            let mut rows: Vec<AnyElement> = Vec::new();
            for group in [GROUP_NERD, GROUP_MONO] {
                rows.push(div().px_2().pt_2().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::color(ui::MUTED_FOREGROUND)).child(group).into_any_element());
                for (i, preset) in self.presets.iter().enumerate().filter(|(_, p)| p.group == group) {
                    let family = preset.family.clone();
                    let active = !self.custom_mode && preset.family == current.font_family;
                    rows.push(
                        div()
                            .id(("font-preset", i))
                            .role(Role::ListBoxOption)
                            .aria_label(preset.label.clone())
                            .px_2()
                            .py(px(6.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::color(ui::ACCENT)))
                            .child(div().w(px(14.)).when(active, |d| d.child(kit::icon_sized(Icon::Check, 14.))))
                            .child(preset.label.clone())
                            .on_click(cx.listener(move |this, _, window, cx| this.pick_family(Some(family.clone()), window, cx)))
                            .into_any_element(),
                    );
                }
            }
            rows.push(div().px_2().pt_2().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::color(ui::MUTED_FOREGROUND)).child("其他").into_any_element());
            rows.push(
                div()
                    .id("font-custom")
                    .role(Role::ListBoxOption)
                    .px_2()
                    .py(px(6.))
                    .flex()
                    .gap_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::color(ui::ACCENT)))
                    .child(div().w(px(14.)).when(self.custom_mode, |d| d.child(kit::icon_sized(Icon::Check, 14.))))
                    .child("自定义…")
                    .on_click(cx.listener(|this, _, window, cx| this.pick_family(None, window, cx)))
                    .into_any_element(),
            );
            div()
                .id("font-list")
                .absolute()
                .top(px(64.))
                .left_0()
                .right_0()
                .max_h(px(260.))
                .overflow_y_scroll()
                .p_1()
                .rounded(px(10.))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .bg(theme::color(ui::CARD))
                .shadow_lg()
                .text_sm()
                .occlude()
                .children(rows)
        });
        let preview_line = |text: &'static str, color: u32| div().truncate().text_color(theme::color(color)).child(text);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .relative()
                    .flex()
                    .gap_4()
                    .child(
                        div().w(px(112.)).flex().flex_col().gap_2().child(kit::field_label("字号")).child(
                            div()
                                .h(px(40.))
                                .flex()
                                .items_center()
                                .rounded(px(8.))
                                .border_1()
                                .border_color(theme::color(ui::BORDER))
                                .overflow_hidden()
                                .child(kit::button("size-down", Variant::Ghost).w(px(32.)).h_full().px_0().rounded_none().aria_label("减小").child("−").on_click(cx.listener(|this, _, _, cx| this.step_size(-1., cx))))
                                .child(div().flex_1().text_center().child(format!("{}", current.font_size as i32)))
                                .child(kit::button("size-up", Variant::Ghost).w(px(32.)).h_full().px_0().rounded_none().aria_label("增大").child("+").on_click(cx.listener(|this, _, _, cx| this.step_size(1., cx)))),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().flex().flex_col().gap_2().child(kit::field_label("字体")).child(
                            div()
                                .id("font-family")
                                .role(Role::ComboBox)
                                .aria_label("字体")
                                .h(px(40.))
                                .px_3()
                                .flex()
                                .items_center()
                                .justify_between()
                                .rounded(px(8.))
                                .border_1()
                                .border_color(theme::color(if self.family_open { ui::RING } else { ui::BORDER }))
                                .cursor_pointer()
                                .child(div().truncate().child(selected))
                                .child(kit::icon_sized(Icon::ChevronDown, 14.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.family_open = !this.family_open;
                                    cx.notify();
                                })),
                        ),
                    )
                    // 延后到最上层画：下拉列表按顺序先画，会被后面的预览卡片、保存按钮压住
                    .when_some(family_list, |d, list| d.child(deferred(list).with_priority(1))),
            )
            .when(self.custom_mode, |d| {
                d.child(div().flex().flex_col().gap_2().child(kit::field_label("自定义字体")).child(kit::text_field(&self.custom, false).h(px(40.))))
            })
            .child(
                div()
                    .px_4()
                    .py_3()
                    .rounded(px(12.))
                    .bg(theme::color(0x1d1d1d))
                    .shadow_md()
                    .font_family(current.font_family.clone())
                    .text_size(px(current.font_size))
                    .line_height(px(current.font_size * 1.5))
                    .child(preview_line("user@host:~$ ls -la /etc", 0xf6f825))
                    .child(preview_line("drwxr-xr-x  123 root  4096 May 24 10:42 .", 0xdfdfe0))
                    .child(preview_line("The quick brown fox 0123456789 → ✓", 0xf6f825))
                    .child(preview_line("Emoji: 😀 😃 🚀 🐱 ❤️ 🌈 🍎 ✅ ⚡ 中文", 0xf6f825))
                    .child(preview_line("Nerd Font 图标: \u{e0b0} \u{f07b} \u{f015} \u{f120} \u{e0a0} \u{f09b} \u{f135}", 0xf6f825)),
            )
            .child(
                div()
                    .pt_3()
                    .flex()
                    .justify_between()
                    .border_t_1()
                    .border_color(theme::color(ui::BORDER))
                    .child(kit::labeled_button("settings-reset", Variant::Outline, None, "还原默认").on_click(cx.listener(|this, _, _, cx| this.reset_font(cx))))
                    .child(kit::labeled_button("settings-save", Variant::Primary, None, "保存").on_click(cx.listener(|this, _, _, cx| this.save_font(cx)))),
            )
            .into_any_element()
    }

    fn groups_panel(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let dirty = self.dirty();
        let refs: Vec<&StoredConnection> = self.connections.iter().collect();
        let ungrouped: Vec<StoredConnection> = groups::ungrouped(&refs).into_iter().cloned().collect();
        let mut sections: Vec<AnyElement> = vec![self.group_section(UNGROUPED, "", ungrouped, None, cx)];
        let names = self.groups.clone();
        for name in names {
            let members: Vec<StoredConnection> = self.connections.iter().filter(|c| groups::normalize(&c.group) == name).cloned().collect();
            let rename = self.renames.iter().find(|(n, _)| *n == name).map(|(_, input)| input.clone());
            sections.push(self.group_section(&name.clone(), &name, members, rename, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(kit::text_field(&self.new_group, false)))
                    .child(kit::labeled_button("group-add", Variant::Primary, Some(Icon::Plus), "添加分组").on_click(cx.listener(|this, _, window, cx| this.add_group(window, cx)))),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("分组与机器"))
                    .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child("拖动机器归组，拖动把手排序分组")),
            )
            .child(div().id("group-tree").max_h(px(300.)).overflow_y_scroll().flex().flex_col().gap_1().children(sections))
            .child(
                div()
                    .pt_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(theme::color(ui::BORDER))
                    .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).when(dirty, |d| d.child("有未保存的改动")))
                    .child(
                        kit::labeled_button("group-save", Variant::Primary, None, "保存")
                            .when(!dirty, |b| b.opacity(0.5).cursor_default())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.dirty() {
                                    this.save_groups(window, cx);
                                }
                            })),
                    ),
            )
            .into_any_element()
    }

    fn group_section(&self, label: &str, group: &str, members: Vec<StoredConnection>, rename: Option<Entity<InputState>>, cx: &mut Context<Self>) -> AnyElement {
        let group = group.to_string();
        let is_named = !group.is_empty();
        // 默认全部折叠：整棵树就是一列分组，拖起来最好排；展开某个分组后才排它的机器
        let expanded = self.expanded.contains(&group) || !is_named;
        let into_here = self.drop == Some(GroupDrop::Into(group.clone()));
        let order_mark = match &self.drop {
            Some(GroupDrop::Order { target, side }) if *target == group => Some(*side),
            _ => None,
        };
        let count = members.len();
        let (toggle_group, drop_group, move_group, remove_group, drag_group) = (group.clone(), group.clone(), group.clone(), group.clone(), group.clone());
        let header = div()
            .id(SharedString::from(format!("tree-group-{label}")))
            .relative()
            .h(px(36.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::color(if into_here { ui::PRIMARY } else { ui::BORDER }))
            .bg(theme::color(if into_here { ui::ACCENT } else { ui::CARD }))
            .when_some(order_mark, |h, side| {
                h.child(
                    div()
                        .absolute()
                        .left(px(4.))
                        .right(px(4.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(theme::color(ui::PRIMARY))
                        .map(|l| if side == Side::Before { l.top(px(-4.)) } else { l.bottom(px(-4.)) }),
                )
            })
            .when(is_named, |h| {
                h.child(
                    div()
                        .id(SharedString::from(format!("grip-{label}")))
                        .cursor_grab()
                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                        .child(kit::icon_sized(Icon::GripVertical, 14.))
                        .on_drag(DraggedGroup { name: drag_group.clone() }, |dragged, _, _, cx| {
                            let name = dragged.name.clone();
                            cx.new(|_| DragPreview(name.into()))
                        }),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("toggle-{label}")))
                        .cursor_pointer()
                        .child(kit::icon_sized(if expanded { Icon::ChevronDown } else { Icon::ChevronRight }, 14.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&toggle_group) {
                                this.expanded.insert(toggle_group.clone());
                            }
                            cx.notify();
                        })),
                )
            })
            .map(|h| match rename {
                Some(input) => h.child(div().flex_1().min_w_0().h(px(28.)).px_1().text_sm().child(gpui_base::input::Input::new(&input))),
                None => h.child(div().flex_1().px_1().text_sm().font_weight(FontWeight::MEDIUM).text_color(theme::color(ui::MUTED_FOREGROUND)).child(label.to_string())),
            })
            .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(count.to_string()))
            .when(is_named, |h| {
                h.child(kit::mini_button(SharedString::from(format!("remove-{label}")), Icon::Trash, "删除分组（机器移到未分组）", ui::DESTRUCTIVE).on_click(cx.listener(move |this, _, window, cx| this.remove_group(&remove_group, window, cx))))
            })
            .on_drag_move::<DraggedMember>(cx.listener(move |this, event: &DragMoveEvent<DraggedMember>, _, cx| {
                if event.bounds.contains(&event.event.position) && this.drop != Some(GroupDrop::Into(drop_group.clone())) {
                    this.drop = Some(GroupDrop::Into(drop_group.clone()));
                    cx.notify();
                }
            }))
            .on_drag_move::<DraggedGroup>(cx.listener(move |this, event: &DragMoveEvent<DraggedGroup>, _, cx| {
                if !event.bounds.contains(&event.event.position) || !is_named || event.drag(cx).name == move_group {
                    return;
                }
                let side = if event.event.position.y < event.bounds.center().y { Side::Before } else { Side::After };
                let target = Some(GroupDrop::Order { target: move_group.clone(), side });
                if this.drop != target {
                    this.drop = target;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|this, dragged: &DraggedMember, window, cx| this.apply_drop(Some(&dragged.id), None, window, cx)))
            .on_drop(cx.listener(|this, dragged: &DraggedGroup, window, cx| this.apply_drop(None, Some(&dragged.name), window, cx)));
        let member_rows: Vec<AnyElement> = if expanded {
            members
                .iter()
                .map(|m| {
                    let (id, item_group, drag_id) = (m.id.clone(), group.clone(), m.id.clone());
                    let mark = match &self.drop {
                        Some(GroupDrop::Beside { item, side, .. }) if *item == m.id => Some(*side),
                        _ => None,
                    };
                    let next_in_group: Option<String> = members.iter().skip_while(|o| o.id != m.id).nth(1).map(|o| o.id.clone());
                    let name: SharedString = m.name.clone().into();
                    div()
                        .id(SharedString::from(format!("tree-item-{}", m.id)))
                        .relative()
                        .ml_6()
                        .px_2()
                        .py(px(6.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(6.))
                        .cursor_grab()
                        .hover(|s| s.bg(theme::color(ui::MUTED)))
                        .text_sm()
                        .when_some(mark, |row, side| {
                            row.child(
                                div()
                                    .absolute()
                                    .left(px(4.))
                                    .right(px(4.))
                                    .h(px(2.))
                                    .rounded_full()
                                    .bg(theme::color(ui::PRIMARY))
                                    .map(|l| if side == Side::Before { l.top(px(-2.)) } else { l.bottom(px(-2.)) }),
                            )
                        })
                        .child(kit::icon_sized(Icon::Server, 14.))
                        .child(div().truncate().child(name.clone()))
                        .child(div().truncate().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(format!("{}@{}", m.username, m.host)))
                        .on_drag(DraggedMember { id: drag_id, name: name.clone() }, |dragged, _, _, cx| {
                            let name = dragged.name.clone();
                            cx.new(|_| DragPreview(name))
                        })
                        .on_drag_move::<DraggedMember>(cx.listener(move |this, event: &DragMoveEvent<DraggedMember>, _, cx| {
                            if !event.bounds.contains(&event.event.position) || event.drag(cx).id == id {
                                return;
                            }
                            let before_half = event.event.position.y < event.bounds.center().y;
                            let target = Some(GroupDrop::Beside {
                                group: item_group.clone(),
                                item: id.clone(),
                                side: if before_half { Side::Before } else { Side::After },
                                before: if before_half { Some(id.clone()) } else { next_in_group.clone() },
                            });
                            if this.drop != target {
                                this.drop = target;
                                cx.notify();
                            }
                        }))
                        .on_drop(cx.listener(|this, dragged: &DraggedMember, window, cx| this.apply_drop(Some(&dragged.id), None, window, cx)))
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        div().flex().flex_col().gap_1().child(header).children(member_rows).into_any_element()
    }
}

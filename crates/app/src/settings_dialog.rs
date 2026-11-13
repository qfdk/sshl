//! 设置（旧 SettingsDialog.tsx + GroupTree.tsx）：终端字号 / 字体 / 配色、外观、快捷键（实时预览，点保存才写盘），分组管理（改完要点保存）。

use std::collections::HashSet;

use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    HighlightStyle, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, ParentElement, Render, Role, SharedString,
    StatefulInteractiveElement, StyledText, Styled, Subscription, Toggled, Window, deferred, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{InputEvent, InputState};
use sshl_core::config_store::{self, ConnectionLayoutInput, StoredConnection};

use crate::assets::Icon;
use crate::backend;
use crate::groups::{self, Side};
use crate::keymap::{self, COMMANDS, Command, Group, Recorded};
use crate::prompts::{self, Options};
use crate::settings::{self, DEFAULT_FONT_FAMILY, DEFAULT_FONT_SIZE, Language, Settings};
use crate::sidebar::DragPreview;
use crate::terminal::frame::Rgb;
use crate::theme::term::{self, Theme};
use crate::theme::Appearance;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

/// 只列非 Mono 版：Mono 版把图标压成一格宽，和正常字符大小不一样（用户不要）。
const NERD_CANDIDATES: &[&str] = &[
    "JetBrainsMono Nerd Font", "FiraCode Nerd Font", "Hack Nerd Font", "MesloLGM Nerd Font", "Iosevka Nerd Font",
    "CaskaydiaCove Nerd Font", "Symbols Nerd Font",
];
const MONO_CANDIDATES: &[&str] = &[
    "Menlo", "Monaco", "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", "Cascadia Mono", "Hack", "Iosevka",
    "Source Code Pro", "IBM Plex Mono", "Roboto Mono", "Inconsolata", "Ubuntu Mono", "Anonymous Pro", "PT Mono",
    "Noto Sans Mono", "Courier New", "Consolas",
];

#[derive(Clone)]
struct Preset {
    family: String,
    /// 带图标的（Nerd Font）一组，其余是普通等宽字体。
    icons: bool,
}

impl Preset {
    fn label(&self) -> SharedString {
        if self.family == DEFAULT_FONT_FAMILY {
            tr!("JetBrainsMono Nerd Font (内置·含图标)").into()
        } else if self.icons {
            tr!("{} (含图标)", self.family).into()
        } else {
            self.family.clone().into()
        }
    }
}

fn presets(installed: &[String]) -> Vec<Preset> {
    let has = |name: &str| installed.iter().any(|f| f == name);
    let mut out = vec![Preset { family: DEFAULT_FONT_FAMILY.into(), icons: true }];
    for name in NERD_CANDIDATES.iter().filter(|n| has(n) && **n != DEFAULT_FONT_FAMILY) {
        out.push(Preset { family: (*name).into(), icons: true });
    }
    for name in MONO_CANDIDATES.iter().filter(|n| has(n)) {
        out.push(Preset { family: (*name).into(), icons: false });
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Font,
    Appearance,
    Shortcuts,
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
    /// 正在录哪条命令的快捷键（「快捷键」页里点了那一格）。
    recording: Option<&'static str>,
    /// 录的时候按了不带 ⌘ / ⌃ / ⌥ 的键：格子里提示要带修饰键。
    record_hint: bool,
    /// 录的时候截住所有按键（先于快捷键生效，⌘Q / ⌘W 也录得进来）；不录了就丢掉。
    recorder: Option<Subscription>,
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
        let new_group = kit::input(|| tr!("新分组名称"), window, cx);
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
            recording: None,
            record_hint: false,
            recorder: None,
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
        // 预览过的字体 / 字号 / 配色没保存就换回去
        let snapshot = self.snapshot.clone();
        let current = settings::get(cx).clone();
        let restored = Settings {
            font_size: snapshot.font_size,
            font_family: snapshot.font_family,
            copy_on_select: snapshot.copy_on_select,
            right_click_paste: snapshot.right_click_paste,
            terminal_theme: snapshot.terminal_theme,
            appearance: snapshot.appearance,
            key_bindings: snapshot.key_bindings,
            ..current.clone()
        };
        if restored != current {
            settings::replace(cx, restored);
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
            let defaults = Settings::default();
            s.font_size = DEFAULT_FONT_SIZE;
            s.font_family = DEFAULT_FONT_FAMILY.into();
            s.copy_on_select = defaults.copy_on_select;
            s.right_click_paste = defaults.right_click_paste;
            s.terminal_theme = defaults.terminal_theme;
        });
        cx.notify();
    }

    fn pick_theme(&mut self, id: &'static str, cx: &mut Context<Self>) {
        settings::preview(cx, |s| s.terminal_theme = id.into());
        cx.notify();
    }

    /// 点了某条命令的快捷键格子：开始录（再点一下同一格 = 不录了）。
    fn toggle_recording(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if self.recording == Some(id) {
            self.stop_recording(cx);
            return;
        }
        self.recording = Some(id);
        self.record_hint = false;
        let this = cx.entity().downgrade();
        self.recorder = Some(cx.intercept_keystrokes(move |event, window, cx| {
            let _ = this.update(cx, |this, cx| this.recorded(&event.keystroke, window, cx));
        }));
        cx.notify();
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        self.record_hint = false;
        self.recorder = None;
        cx.notify();
    }

    fn recorded(&mut self, stroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.recording else { return };
        let Some(command) = keymap::find(id) else { return };
        match keymap::record(stroke) {
            // 单按修饰键（松开时也报一次）：放过去，接着等
            Recorded::Ignore => return,
            Recorded::Cancel => self.stop_recording(cx),
            Recorded::NeedsModifier => {
                self.record_hint = true;
                cx.notify();
            }
            Recorded::Clear => {
                self.stop_recording(cx);
                self.set_keys(id, Vec::new(), cx);
            }
            Recorded::Set(keys) => {
                self.stop_recording(cx);
                match keymap::conflict(id, &keys, keymap::active(cx)) {
                    None => self.set_keys(id, vec![keys], cx),
                    Some(other) => self.confirm_takeover(command, other, keys, window, cx),
                }
            }
        }
        // 这一下是录快捷键用的：不再当快捷键、也不给对话框（Esc 不关对话框）
        cx.stop_propagation();
    }

    /// 改快捷键先预览（键位表马上换），点保存才写盘，关掉不保存会还原。
    fn set_keys(&mut self, id: &str, keys: Vec<String>, cx: &mut Context<Self>) {
        let mut overrides = settings::get(cx).key_bindings.clone();
        keymap::assign_in(cx, &mut overrides, id, keys);
        settings::preview(cx, |s| s.key_bindings = overrides);
        cx.notify();
    }

    /// 录的键已经给了别的命令：问一下，改用的话那条命令去掉这个键（它还有别的键就留着别的）。
    fn confirm_takeover(&mut self, command: &'static Command, other: &'static Command, keys: String, window: &mut Window, cx: &mut Context<Self>) {
        let (label, other_label) = (command.label(), other.label());
        let options = prompts::Options::new(tr!("{} 已经是「{}」的快捷键", keymap::display(&keys), other_label))
            .message(tr!("改给「{}」后，「{}」就不再用这个快捷键。", label, other_label))
            .confirm(tr!("改用"));
        let answer = prompts::confirm(window, cx, options);
        cx.spawn(async move |this, cx| {
            if !answer.await.unwrap_or(false) {
                return;
            }
            let _ = this.update(cx, |_, cx| {
                let remaining: Vec<String> = keymap::active(cx).get(other.id).into_iter().flatten().filter(|k| **k != keys).cloned().collect();
                let mut overrides = settings::get(cx).key_bindings.clone();
                keymap::assign_in(cx, &mut overrides, other.id, remaining);
                keymap::assign_in(cx, &mut overrides, command.id, vec![keys]);
                settings::preview(cx, |s| s.key_bindings = overrides);
                cx.notify();
            });
        })
        .detach();
    }

    fn reset_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.stop_recording(cx);
        settings::preview(cx, |s| s.key_bindings.clear());
    }

    fn add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_group.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        if self.groups.contains(&name) {
            prompts::alert_later(window, cx, Options::new(tr!("无法保存分组")).message(tr!("分组名称已存在。")));
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
            Err(error) => prompts::alert_later(window, cx, Options::new(tr!("保存分组失败")).message(error.to_string())),
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
                    this.stop_recording(cx);
                }))
        };
        let panel = match self.tab {
            Tab::Font => self.font_panel(window, cx),
            Tab::Appearance => self.appearance_panel(cx),
            Tab::Shortcuts => self.shortcuts_panel(window, cx),
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
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(tr!("设置")))
                    .child(kit::icon_button("settings-close", Variant::Ghost, Icon::X, tr!("关闭")).w(px(32.)).h(px(32.)).on_click(cx.listener(|this, _, _, cx| this.close(cx)))),
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
                            .child(tab_button("settings-tab-font", Tab::Font, Icon::Type, tr!("终端"), self.tab == Tab::Font, cx))
                            .child(tab_button("settings-tab-appearance", Tab::Appearance, Icon::Moon, tr!("外观"), self.tab == Tab::Appearance, cx))
                            .child(tab_button("settings-tab-shortcuts", Tab::Shortcuts, Icon::Keyboard, tr!("快捷键"), self.tab == Tab::Shortcuts, cx))
                            .child(tab_button("settings-tab-groups", Tab::Groups, Icon::Folder, tr!("分组管理"), self.tab == Tab::Groups, cx)),
                    )
                    // 窗口矮（最小 600）时「终端」这页放不下：右边这块自己滚动，标题栏和保存按钮不会被挤出窗口
                    .child(
                        div()
                            .id("settings-panel")
                            .flex_1()
                            .min_w_0()
                            .max_h(window.viewport_size().height - px(96.))
                            .overflow_y_scroll()
                            .p_5()
                            .child(panel),
                    ),
            );
        kit::modal("settings-dialog", 768., body)
    }
}

impl SettingsDialog {
    fn font_panel(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let current = settings::get(cx).clone();
        let selected = self.presets.iter().find(|p| p.family == current.font_family && !self.custom_mode).map(Preset::label).unwrap_or_else(|| tr!("自定义…").into());
        let family_list = self.family_open.then(|| {
            let mut rows: Vec<AnyElement> = Vec::new();
            for (icons, group) in [(true, tr!("图标字体（Nerd Font）")), (false, tr!("等宽字体"))] {
                rows.push(div().px_2().pt_2().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::color(ui::MUTED_FOREGROUND)).child(group).into_any_element());
                for (i, preset) in self.presets.iter().enumerate().filter(|(_, p)| p.icons == icons) {
                    let family = preset.family.clone();
                    let active = !self.custom_mode && preset.family == current.font_family;
                    rows.push(
                        div()
                            .id(("font-preset", i))
                            .role(Role::ListBoxOption)
                            .aria_label(preset.label())
                            .px_2()
                            .py(px(6.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::color(ui::ACCENT)))
                            .child(div().w(px(14.)).when(active, |d| d.child(kit::icon_sized(Icon::Check, 14.))))
                            .child(preset.label())
                            .on_click(cx.listener(move |this, _, window, cx| this.pick_family(Some(family.clone()), window, cx)))
                            .into_any_element(),
                    );
                }
            }
            rows.push(div().px_2().pt_2().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("其他")).into_any_element());
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
                    .child(tr!("自定义…"))
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
        let palette = current.terminal_theme();
        // 第二行是浅灰的 ls 输出，用 7 号白；浅色主题的白在浅底上看不清，改用前景色
        let dim = if palette.is_light() { palette.foreground } else { palette.ansi[7] };
        let preview_line = |text: SharedString, color: Rgb| div().truncate().text_color(term::hsla(color)).child(text);
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
                        div().w(px(112.)).flex().flex_col().gap_2().child(kit::field_label(tr!("字号"))).child(
                            div()
                                .h(px(40.))
                                .flex()
                                .items_center()
                                .rounded(px(8.))
                                .border_1()
                                .border_color(theme::color(ui::BORDER))
                                .overflow_hidden()
                                .child(kit::button("size-down", Variant::Ghost).w(px(32.)).h_full().px_0().rounded_none().aria_label(tr!("减小")).child("−").on_click(cx.listener(|this, _, _, cx| this.step_size(-1., cx))))
                                .child(div().flex_1().text_center().child(format!("{}", current.font_size as i32)))
                                .child(kit::button("size-up", Variant::Ghost).w(px(32.)).h_full().px_0().rounded_none().aria_label(tr!("增大")).child("+").on_click(cx.listener(|this, _, _, cx| this.step_size(1., cx)))),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().flex().flex_col().gap_2().child(kit::field_label(tr!("字体"))).child(
                            div()
                                .id("font-family")
                                .role(Role::ComboBox)
                                .aria_label(tr!("字体"))
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
                d.child(div().flex().flex_col().gap_2().child(kit::field_label(tr!("自定义字体"))).child(kit::text_field(&self.custom, false).h(px(40.))))
            })
            .child(
                div()
                    .px_4()
                    .py_3()
                    .rounded(px(12.))
                    .bg(term::hsla(palette.background))
                    .shadow_md()
                    .font_family(current.font_family.clone())
                    .text_size(px(current.font_size))
                    .line_height(px(current.font_size * 1.5))
                    .child(preview_line("user@host:~$ ls -la /etc".into(), palette.foreground))
                    .child(preview_line("drwxr-xr-x  123 root  4096 May 24 10:42 .".into(), dim))
                    .child(preview_line("The quick brown fox 0123456789 → ✓".into(), palette.foreground))
                    .child(preview_line("Emoji: 😀 😃 🚀 🐱 ❤️ 🌈 🍎 ✅ ⚡ 中文".into(), palette.foreground))
                    .child(preview_line(format!("{} \u{e0b0} \u{f07b} \u{f015} \u{f120} \u{e0a0} \u{f09b} \u{f135}", tr!("Nerd Font 图标:")).into(), palette.foreground)),
            )
            .child(div().flex().flex_col().gap_2().child(kit::field_label(tr!("配色"))).child(self.theme_cards(palette, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.toggle_row("copy-on-select", tr!("选中即复制"), tr!("拖选文字后自动复制到剪贴板"), current.copy_on_select, |s| &mut s.copy_on_select, cx))
                    .child(self.toggle_row("right-click-paste", tr!("右键粘贴"), tr!("在终端里点右键直接粘贴（程序接收鼠标时除外）"), current.right_click_paste, |s| &mut s.right_click_paste, cx)),
            )
            .child(
                div()
                    .pt_3()
                    .flex()
                    .justify_between()
                    .border_t_1()
                    .border_color(theme::color(ui::BORDER))
                    .child(kit::labeled_button("settings-reset", Variant::Outline, None, tr!("还原默认")).on_click(cx.listener(|this, _, _, cx| this.reset_font(cx))))
                    .child(kit::labeled_button("settings-save", Variant::Primary, None, tr!("保存")).on_click(cx.listener(|this, _, _, cx| this.save_font(cx)))),
            )
            .into_any_element()
    }

    /// 配色：每套一张小卡片，主题的底色上用它的前景和两个 ANSI 色写一行提示符 + 命令，下面是名字；
    /// 选中的描主色蓝边。点了先预览（所有终端跟着换），保存才写盘。
    fn theme_cards(&self, current: &'static Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // 目录蓝（4 号）、提示符绿（2 号）、命令是前景色
        const SAMPLE: &str = "~/app $ ls -la";
        let cards = term::THEMES.iter().enumerate().map(|(i, t)| {
            let active = t.id == current.id;
            let id = t.id;
            let color = |c: Rgb| HighlightStyle { color: Some(term::hsla(c)), ..Default::default() };
            let sample = StyledText::new(SAMPLE).with_highlights([(0..5, color(t.ansi[4])), (6..7, color(t.ansi[2]))]);
            div()
                .id(("terminal-theme", i))
                .role(Role::RadioButton)
                .aria_label(t.name)
                .aria_toggled(if active { Toggled::True } else { Toggled::False })
                .min_w_0()
                .px(px(8.))
                .py(px(4.))
                .rounded(px(8.))
                .border_2()
                .border_color(theme::color(if active { ui::PRIMARY } else { ui::BORDER }))
                .when(!active, |card| card.hover(|s| s.border_color(theme::color(ui::RING))))
                .bg(term::hsla(t.background))
                .text_color(term::hsla(t.foreground))
                .line_height(px(16.))
                .cursor_pointer()
                .child(div().truncate().font_family(DEFAULT_FONT_FAMILY).text_size(px(11.)).child(sample))
                .child(div().truncate().text_xs().child(t.name))
                .on_click(cx.listener(move |this, _, _, cx| this.pick_theme(id, cx)))
        });
        div().id("terminal-themes").role(Role::RadioGroup).aria_label(tr!("配色")).grid().grid_cols(4).gap(px(6.)).children(cards)
    }

    /// 一行开关：勾选框 + 名称 + 灰色说明。和字体一样先预览，保存才写盘。
    fn toggle_row(
        &self,
        id: &'static str,
        label: &'static str,
        hint: &'static str,
        checked: bool,
        field: fn(&mut Settings) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .role(Role::CheckBox)
            .aria_label(label)
            .child(kit::checkbox((id, 0usize), checked).flex_shrink_0())
            .child(div().flex_shrink_0().text_sm().child(label))
            .child(div().min_w_0().truncate().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(hint))
            .on_click(cx.listener(move |_, _, _, cx| {
                settings::preview(cx, |s| {
                    let value = field(s);
                    *value = !*value;
                });
                cx.notify();
            }))
    }

    /// 界面外观：点了立刻换（预览），保存才记住；和「终端」页共用一套保存 / 不保存还原。
    fn appearance_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let current = settings::get(cx).appearance;
        let language = settings::get(cx).language;
        let entity = cx.entity();
        let language_entity = entity.clone();
        // 语言名用它自己的文字写，不跟着界面语言变：换错了也认得出哪个是自己的
        let languages = [(Language::System, None, SharedString::from(tr!("跟随系统"))), (Language::Zh, None, "简体中文".into()), (Language::En, None, "English".into())];
        let options = [
            (Appearance::System, None, SharedString::from(tr!("跟随系统"))),
            (Appearance::Light, None, tr!("浅色").into()),
            (Appearance::Dark, None, tr!("深色").into()),
        ];
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(kit::field_label(tr!("界面外观")))
                    .child(div().w(px(300.)).child(kit::segmented("appearance", &options, current, move |value, _, cx| {
                        settings::preview(cx, |s| s.appearance = value);
                        entity.update(cx, |_, cx| cx.notify());
                    })))
                    .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("终端里的颜色不跟着变，在「终端」页的配色里选"))),
            )
            .child(div().flex().flex_col().gap_2().child(kit::field_label(tr!("语言"))).child(div().w(px(300.)).child(kit::segmented(
                "language",
                &languages,
                language,
                move |value, _, cx| {
                    settings::preview(cx, |s| s.language = value);
                    language_entity.update(cx, |_, cx| cx.notify());
                },
            ))))
            .child(
                div()
                    .pt_3()
                    .flex()
                    .justify_end()
                    .border_t_1()
                    .border_color(theme::color(ui::BORDER))
                    .child(kit::labeled_button("appearance-save", Variant::Primary, None, tr!("保存")).on_click(cx.listener(|this, _, _, cx| this.save_font(cx)))),
            )
            .into_any_element()
    }

    /// 快捷键：按分组列出能改的命令。点格子再按新的组合键（录的是这个键盘布局下实际按出来的键）；
    /// 改过的那一行多一个「恢复默认」。和「终端」「外观」页一样先预览，点保存才写盘。
    /// 列表很长：自己滚动，底下的「还原默认 / 保存」一直看得见（同「分组管理」页）。
    fn shortcuts_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let changed: HashSet<&str> = settings::get(cx).key_bindings.keys().filter_map(|id| keymap::find(id)).map(|c| c.id).collect();
        let active = keymap::active(cx).clone();
        let sections: Vec<AnyElement> = Group::ALL
            .into_iter()
            .map(|group| {
                let rows: Vec<AnyElement> = COMMANDS
                    .iter()
                    .enumerate()
                    .filter(|(_, command)| command.group == group)
                    .map(|(index, command)| self.shortcut_row(index, command, active.get(command.id).map_or(&[][..], Vec::as_slice), changed.contains(command.id), cx))
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .child(div().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::color(ui::MUTED_FOREGROUND)).child(group.label()))
                    .children(rows)
                    .into_any_element()
            })
            .collect();
        // 对话框最高是窗口高减 96（见 render），再除去标题栏、说明、底栏和内边距
        let list_height = (window.viewport_size().height - px(250.)).clamp(px(240.), px(440.));
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("点快捷键后按新的组合键：Esc 取消，⌫ 去掉")))
            .child(div().id("shortcut-list").max_h(list_height).overflow_y_scroll().pr_2().flex().flex_col().gap_4().children(sections))
            .child(
                div()
                    .pt_3()
                    .flex()
                    .justify_between()
                    .border_t_1()
                    .border_color(theme::color(ui::BORDER))
                    .child(kit::labeled_button("shortcuts-reset", Variant::Outline, None, tr!("还原默认")).on_click(cx.listener(|this, _, _, cx| this.reset_shortcuts(cx))))
                    .child(kit::labeled_button("shortcuts-save", Variant::Primary, None, tr!("保存")).on_click(cx.listener(|this, _, _, cx| this.save_font(cx)))),
            )
            .into_any_element()
    }

    fn shortcut_row(&self, index: usize, command: &'static Command, keys: &[String], changed: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = command.id;
        let label = command.label();
        let recording = self.recording == Some(id);
        let text: SharedString = if recording {
            if self.record_hint { tr!("要带 ⌘、⌃ 或 ⌥").into() } else { tr!("按下新的快捷键…").into() }
        } else if keys.is_empty() {
            tr!("无").into()
        } else {
            keys.iter().map(|keys| keymap::display(keys)).collect::<Vec<_>>().join("  ").into()
        };
        let muted = recording || keys.is_empty();
        div()
            .h(px(34.))
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().truncate().text_sm().child(label.clone()))
            .when(changed && !recording, |row| {
                row.child(
                    kit::mini_button(("shortcut-reset", index), Icon::RotateCcw, tr!("恢复默认"), ui::MUTED_FOREGROUND).on_click(cx.listener(move |this, _, _, cx| {
                        settings::preview(cx, |s| {
                            s.key_bindings.remove(id);
                        });
                        this.stop_recording(cx);
                    })),
                )
            })
            .child(
                div()
                    .id(("shortcut", index))
                    .role(Role::Button)
                    .aria_label(tr!("{} 的快捷键", label))
                    .flex_shrink_0()
                    .min_w(px(132.))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme::color(if recording { ui::RING } else { ui::CONTROL_BORDER }))
                    .bg(theme::color(ui::CONTROL))
                    .text_sm()
                    .text_color(theme::color(if muted { ui::MUTED_FOREGROUND } else { ui::FOREGROUND }))
                    .cursor_pointer()
                    .when(!recording, |b| b.hover(|s| s.border_color(theme::color(ui::RING))))
                    .child(text)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_recording(id, cx))),
            )
            .into_any_element()
    }

    fn groups_panel(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let dirty = self.dirty();
        let refs: Vec<&StoredConnection> = self.connections.iter().collect();
        let ungrouped: Vec<StoredConnection> = groups::ungrouped(&refs).into_iter().cloned().collect();
        let mut sections: Vec<AnyElement> = vec![self.group_section(tr!("未分组"), "", ungrouped, None, cx)];
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
                    .child(kit::labeled_button("group-add", Variant::Primary, Some(Icon::Plus), tr!("添加分组")).on_click(cx.listener(|this, _, window, cx| this.add_group(window, cx)))),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(tr!("分组与机器")))
                    .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("拖动机器归组，拖动把手排序分组"))),
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
                    .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).when(dirty, |d| d.child(tr!("有未保存的改动"))))
                    .child(
                        kit::labeled_button("group-save", Variant::Primary, None, tr!("保存"))
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
                h.child(kit::mini_button(SharedString::from(format!("remove-{label}")), Icon::Trash, tr!("删除分组（机器移到未分组）"), ui::DESTRUCTIVE).on_click(cx.listener(move |this, _, window, cx| this.remove_group(&remove_group, window, cx))))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nerd_font_mono_variants_are_never_offered() {
        // Mono 版把图标压成一格宽，和正常字符大小不一样：装了也不列出来
        let installed: Vec<String> =
            ["JetBrainsMono Nerd Font Mono", "JetBrainsMono Nerd Font", "Hack Nerd Font Mono", "Hack Nerd Font", "Menlo"].map(String::from).to_vec();
        let families: Vec<String> = presets(&installed).into_iter().map(|p| p.family).collect();
        assert!(families.iter().all(|f| !f.ends_with("Nerd Font Mono")), "{families:?}");
        assert!(families.contains(&"Hack Nerd Font".to_string()));
        assert!(families.contains(&"Menlo".to_string()));
        assert_eq!(families.iter().filter(|f| *f == DEFAULT_FONT_FAMILY).count(), 1);
    }
}

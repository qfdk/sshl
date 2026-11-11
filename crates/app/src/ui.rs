//! 纸页设计的基础控件（按钮、输入框、复选框、分段控件、图标、模态框、菜单）。
//! 交互行为（输入法、焦点、选择）用 gpui-base，外观按旧界面的 shadcn 样式自己画。

use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Div, Element, ElementId, Entity, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels, Role, SharedString, Stateful,
    StatefulInteractiveElement, Style, Styled, TransformationMatrix, Window, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{Input, InputState};

use crate::assets::Icon;
use crate::theme::{self, ui};

pub fn icon(icon: Icon) -> Glyph {
    Glyph { icon, size: px(16.), color: None }
}

pub fn icon_sized(icon: Icon, size: f32) -> Glyph {
    Glyph { icon, size: px(size), color: None }
}

/// 线条图标。不指定颜色时用所在位置继承下来的文字颜色（同 CSS 的 currentColor）：
/// gpui 自带的 svg 元素只认自身的 text_color，不继承。
pub struct Glyph {
    icon: Icon,
    size: Pixels,
    color: Option<Hsla>,
}

impl Glyph {
    pub fn text_color(mut self, color: impl Into<Hsla>) -> Self {
        self.color = Some(color.into());
        self
    }
}

impl IntoElement for Glyph {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Glyph {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = self.size.into();
        style.size.height = self.size.into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut Window, _: &mut App) {}

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let color = self.color.unwrap_or_else(|| window.text_style().color);
        let _ = window.paint_svg(bounds, self.icon.path(), None, TransformationMatrix::unit(), color, cx);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Outline,
    Ghost,
    Destructive,
}

/// 按钮外壳：h-9、圆角 8px、14px 中等字重。调用方加图标 / 文字和 on_click。
pub fn button(id: impl Into<ElementId>, variant: Variant) -> Stateful<Div> {
    let base = div()
        .id(id)
        .role(Role::Button)
        .h(px(36.))
        .px(px(14.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .rounded(px(8.))
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer();
    match variant {
        Variant::Primary => base
            .bg(theme::color(ui::PRIMARY))
            .text_color(theme::color(ui::PRIMARY_FOREGROUND))
            .hover(|s| s.bg(theme::alpha(ui::PRIMARY, 0.9))),
        Variant::Destructive => base
            .bg(theme::color(ui::DESTRUCTIVE))
            .text_color(theme::color(0xffffff))
            .hover(|s| s.bg(theme::alpha(ui::DESTRUCTIVE, 0.9))),
        Variant::Outline => base
            .border_1()
            .border_color(theme::color(ui::CONTROL_BORDER))
            .bg(theme::color(ui::CONTROL))
            .text_color(theme::color(ui::FOREGROUND))
            .shadow_xs()
            .hover(|s| s.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND))),
        Variant::Ghost => base
            .text_color(theme::color(ui::FOREGROUND))
            .hover(|s| s.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND))),
    }
}

/// 带图标和文字的按钮。
pub fn labeled_button(id: impl Into<ElementId>, variant: Variant, glyph: Option<Icon>, label: impl Into<SharedString>) -> Stateful<Div> {
    let label = label.into();
    button(id, variant)
        .aria_label(label.clone())
        .when_some(glyph, |b, glyph| b.child(icon(glyph)))
        .child(label)
}

/// 只有图标的方形按钮（34px），悬停显示 `title`。
pub fn icon_button(id: impl Into<ElementId>, variant: Variant, glyph: Icon, title: impl Into<SharedString>) -> Stateful<Div> {
    let title = title.into();
    button(id, variant).w(px(34.)).h(px(34.)).px_0().aria_label(title.clone()).tooltip(move |window, cx| tooltip(title.clone(), window, cx)).child(icon(glyph))
}

/// 小号图标按钮（行内操作，26px）。
pub fn mini_button(id: impl Into<ElementId>, glyph: Icon, title: impl Into<SharedString>, color: ui::Token) -> Stateful<Div> {
    let title = title.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.clone())
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .text_color(theme::color(color))
        .cursor_pointer()
        .hover(|s| s.bg(theme::color(ui::MUTED)))
        .tooltip(move |window, cx| tooltip(title.clone(), window, cx))
        .child(icon_sized(glyph, 14.))
}

pub fn tooltip(text: SharedString, _window: &mut Window, cx: &mut App) -> gpui::AnyView {
    cx.new(|_| Tooltip(text)).into()
}

struct Tooltip(SharedString);

impl gpui::Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(theme::color(ui::TOOLTIP))
            .text_color(theme::color(ui::TOOLTIP_FOREGROUND))
            .text_size(px(12.))
            .child(self.0.clone())
    }
}

/// 单行输入框外观：h-9、1px 边框、圆角 8px，聚焦时主色描边。
pub fn text_field(state: &Entity<InputState>, focused: bool) -> Div {
    div()
        .h(px(36.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(8.))
        .border_1()
        .border_color(theme::color(if focused { ui::RING } else { ui::BORDER }))
        .bg(theme::color(ui::CARD))
        .text_size(px(14.))
        .child(div().flex_1().min_w_0().child(Input::new(state)))
}

pub fn field_label(text: impl Into<SharedString>) -> Div {
    div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(text.into())
}

pub fn checkbox(id: impl Into<ElementId>, checked: bool) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::CheckBox)
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .cursor_pointer()
        .map(|b| {
            if checked {
                b.bg(theme::color(ui::PRIMARY))
                    .border_color(theme::color(ui::PRIMARY))
                    .text_color(theme::color(ui::PRIMARY_FOREGROUND))
                    .child(icon_sized(Icon::Check, 12.))
            } else {
                b.bg(theme::color(ui::CARD)).border_color(theme::color(ui::CHECK_BORDER))
            }
        })
}

/// 分段控件（旧界面的 TabsList 样式）：浅灰槽里放几个选项，选中项是白卡片。
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<SharedString>,
    options: &[(T, Option<Icon>, SharedString)],
    selected: T,
    on_select: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    div()
        .id(id.clone())
        .role(Role::TabList)
        .h(px(34.))
        .p(px(3.))
        .flex()
        .gap(px(2.))
        .rounded(px(10.))
        .bg(theme::color(ui::SECONDARY))
        .children(options.iter().enumerate().map(|(index, (value, glyph, label))| {
            let value = *value;
            let active = value == selected;
            let on_select = on_select.clone();
            div()
                .id((id.clone(), index))
                .role(Role::Tab)
                .aria_label(label.clone())
                .h(px(28.))
                .px(px(12.))
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .rounded(px(7.))
                .text_size(px(14.))
                .cursor_pointer()
                .map(|t| {
                    if active {
                        t.bg(theme::color(ui::RAISED)).font_weight(FontWeight::MEDIUM).shadow_xs()
                    } else {
                        t.text_color(theme::color(ui::MUTED_FOREGROUND)).hover(|s| s.text_color(theme::color(ui::FOREGROUND)))
                    }
                })
                .when_some(*glyph, |t, glyph| t.child(icon_sized(glyph, 14.)))
                .child(label.clone())
                .on_click(move |_: &ClickEvent, window, cx| {
                    if !active {
                        on_select(value, window, cx)
                    }
                })
        }))
}

/// 模态框：半透明遮罩 + 居中白卡片。点遮罩不关（避免误触丢失表单）。
pub fn modal(id: impl Into<ElementId>, width: f32, content: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::alpha(0x000000, 0.25))
        .occlude()
        .child(
            div()
                .id("dialog")
                .role(Role::Dialog)
                .w(px(width))
                .max_w_full()
                .rounded(px(16.))
                .bg(theme::color(ui::CARD))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .shadow_xl()
                .overflow_hidden()
                .child(content),
        )
}

/// 模态框底部的按钮条（旧界面 DialogFooter：浅灰底、上边框、右对齐）。
pub fn modal_footer() -> Div {
    div()
        .px_5()
        .py_3()
        .flex()
        .justify_end()
        .gap_2()
        .border_t_1()
        .border_color(theme::color(ui::BORDER))
        .bg(theme::color(ui::MUTED))
        // gpui 的 overflow_hidden 只按矩形裁，底色要自己跟着卡片的圆角
        .rounded_b(px(15.))
}

pub type MenuAction = Box<dyn Fn(&mut Window, &mut App)>;

pub struct MenuItem {
    pub label: SharedString,
    pub glyph: Option<Icon>,
    pub destructive: bool,
    pub disabled: bool,
    pub action: MenuAction,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), glyph: None, destructive: false, disabled: false, action: Box::new(action) }
    }

    pub fn icon(mut self, glyph: Icon) -> Self {
        self.glyph = Some(glyph);
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

/// 弹出菜单的面板（定位由调用方用 anchored + deferred 决定）。`dismiss` 在点了某项后调用。
pub fn menu_panel(entries: Vec<MenuEntry>, dismiss: impl Fn(&mut Window, &mut App) + Clone + 'static) -> AnyElement {
    div()
        .id("menu")
        .role(Role::Menu)
        .min_w(px(160.))
        .p_1()
        .rounded(px(10.))
        .bg(theme::color(ui::CARD))
        .border_1()
        .border_color(theme::color(ui::BORDER))
        .shadow_lg()
        .text_size(px(13.))
        .children(entries.into_iter().enumerate().map(|(index, entry)| match entry {
            MenuEntry::Separator => div().my_1().h(px(1.)).bg(theme::color(ui::BORDER)).into_any_element(),
            MenuEntry::Item(item) => {
                let dismiss = dismiss.clone();
                let color: Hsla = if item.disabled {
                    theme::alpha(ui::FOREGROUND, 0.4)
                } else if item.destructive {
                    theme::color(ui::DESTRUCTIVE)
                } else {
                    theme::color(ui::FOREGROUND)
                };
                let MenuItem { label, glyph, disabled, action, .. } = item;
                div()
                    .id(("menu-item", index))
                    .role(Role::MenuItem)
                    .aria_label(label.clone())
                    .px_2()
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(6.))
                    .text_color(color)
                    .when(!disabled, |row| {
                        row.cursor_pointer().hover(|s| s.bg(theme::color(ui::ACCENT))).on_click(move |_, window, cx| {
                            dismiss(window, cx);
                            action(window, cx);
                        })
                    })
                    .when_some(glyph, |row, glyph| row.child(icon_sized(glyph, 14.)))
                    .child(label)
                    .into_any_element()
            }
        }))
        .into_any_element()
}

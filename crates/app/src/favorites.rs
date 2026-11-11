//! 目录收藏：顶栏的星星按钮 / ⌘⇧D 打开面板，列出这个连接收藏的目录（本机标签共用一份），回车过去。
//! 和文件管理远程地址栏里的收藏是同一份数据（`config_store::bookmarks`）。
//! 在终端页选中 = 在终端里 `cd` 过去（文件管理跟着终端目录走），在文件管理页选中 = 文件管理直接跳过去。

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{Input, InputEvent, InputState};
use sshl_core::config_store;

use crate::assets::Icon;
use crate::files::{bookmark_label, shell_quote};
use crate::prompts::{self, Options};
use crate::settings;
use crate::theme::{self, ui};
use crate::ui as kit;

actions!(favorites, [FavoritePrev, FavoriteNext, FavoriteGo, FavoriteClose]);

pub fn bind_keys(cx: &mut App) {
    // 绑在比 gpui-base 输入框更深的上下文里，压过它自己的 上 / 下 / 回车 / Esc
    cx.bind_keys([
        KeyBinding::new("up", FavoritePrev, Some("FavoritePanel > Input")),
        KeyBinding::new("down", FavoriteNext, Some("FavoritePanel > Input")),
        KeyBinding::new("enter", FavoriteGo, Some("FavoritePanel > Input")),
        KeyBinding::new("escape", FavoriteClose, Some("FavoritePanel > Input")),
    ]);
}

// ---------------------------------------------------------------- 纯逻辑

/// 按搜索词筛选：空格分开的每个词都要出现在路径里（不分大小写）。返回下标。
pub fn filter_paths(paths: &[String], query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    paths
        .iter()
        .enumerate()
        .filter(|(_, path)| {
            let haystack = path.to_lowercase();
            terms.iter().all(|term| haystack.contains(term.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// 发到终端的 `cd`：普通路径原样写（看着干净），带空格、引号、`$` 之类的用单引号包住。
pub fn cd_command(path: &str) -> String {
    let plain = !path.is_empty() && !path.starts_with('-') && path.chars().all(|c| c.is_ascii_alphanumeric() || "/_.-+@:,%".contains(c));
    if plain { format!("cd {path}") } else { format!("cd -- {}", shell_quote(path)) }
}

// ---------------------------------------------------------------- 面板

pub enum FavoritePanelEvent {
    /// 去这个目录。
    Go(String),
    /// 收藏增删了：文件管理地址栏的那份要重读。
    Changed,
    Close,
}

pub struct FavoritePanel {
    /// 收藏归谁：连接 id，本机标签是共用的那一个。
    scope: Option<String>,
    paths: Vec<String>,
    /// 当前目录（终端或文件管理此刻所在）：能读到才有右上角的收藏开关。
    current: Option<String>,
    search: Entity<InputState>,
    /// 筛选后列表里的位置。
    selected: usize,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<FavoritePanelEvent> for FavoritePanel {}

impl FavoritePanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索收藏的目录…"));
        let subscription = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
        });
        Self { scope: None, paths: Vec::new(), current: None, search, selected: 0, scroll: ScrollHandle::new(), _subscription: subscription }
    }

    /// 打开面板：重新读一遍收藏、清空搜索、焦点给搜索框。`current` 还不知道就先给 None，读到了再 [`Self::set_current`]。
    pub fn open(&mut self, scope: String, current: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = Some(scope);
        self.current = current;
        self.reload();
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        let focus = self.search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// 终端目录是后台读的：读回来时面板已经换给别的连接就不要了。
    pub fn set_current(&mut self, scope: &str, current: Option<String>, cx: &mut Context<Self>) {
        if self.scope.as_deref() == Some(scope) {
            self.current = current;
            cx.notify();
        }
    }

    fn reload(&mut self) {
        let Some(scope) = &self.scope else { return };
        match config_store::bookmarks(scope) {
            Ok(paths) => self.paths = paths,
            Err(error) => tracing::warn!("load bookmarks: {error}"),
        }
    }

    fn visible(&self, cx: &App) -> Vec<usize> {
        filter_paths(&self.paths, &self.search.read(cx).value())
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

    fn go(&mut self, position: usize, cx: &mut Context<Self>) {
        let visible = self.visible(cx);
        if let Some(path) = visible.get(position).and_then(|&i| self.paths.get(i)) {
            cx.emit(FavoritePanelEvent::Go(path.clone()));
        }
    }

    fn set_bookmarked(&mut self, path: &str, bookmarked: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = &self.scope else { return };
        if let Err(error) = config_store::set_bookmarked(scope, path, bookmarked) {
            prompts::alert_later(window, cx, Options::new("收藏失败").message(error.to_string()));
        }
        self.reload();
        let count = self.visible(cx).len();
        self.selected = self.selected.min(count.saturating_sub(1));
        cx.emit(FavoritePanelEvent::Changed);
        cx.notify();
    }

    /// 搜索行右端的星星：收藏 / 取消当前目录，已收藏是实心蓝色（同文件管理地址栏）。
    fn current_toggle(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let path = self.current.clone()?;
        let marked = self.paths.contains(&path);
        let (glyph, title, color) = if marked {
            (Icon::StarFilled, format!("取消收藏 {path}"), ui::PRIMARY)
        } else {
            (Icon::Star, format!("收藏当前目录 {path}"), ui::MUTED_FOREGROUND)
        };
        Some(
            kit::mini_button("favorite-current", glyph, title, color)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.set_bookmarked(&path, !marked, window, cx))),
        )
    }

    fn row(&self, position: usize, path: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (name, parent) = bookmark_label(path);
        let selected = position == self.selected;
        let current = self.current.as_deref() == Some(path);
        let remove = path.to_string();
        div()
            .id(("favorite", position))
            .group("favorite-row")
            .role(Role::MenuItem)
            .aria_label(SharedString::from(path.to_string()))
            .pl_2()
            .pr_1()
            .py(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(7.))
            .cursor_pointer()
            .when(selected, |row| row.bg(theme::color(ui::ACCENT)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.selected != position {
                    this.selected = position;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(position, cx)))
            .child(
                kit::icon_sized(if current { Icon::FolderOpen } else { Icon::Folder }, 15.)
                    .text_color(theme::color(if current { ui::PRIMARY } else { ui::MUTED_FOREGROUND })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .when(current, |t| t.text_color(theme::color(ui::PRIMARY)))
                            .child(name),
                    )
                    .when(!parent.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .font_family(settings::DEFAULT_FONT_FAMILY)
                                .text_size(px(11.))
                                .text_color(theme::color(ui::MUTED_FOREGROUND))
                                .child(parent),
                        )
                    }),
            )
            .child(div().flex_shrink_0().opacity(0.).group_hover("favorite-row", |s| s.opacity(1.)).child(
                kit::mini_button(("favorite-remove", position), Icon::X, "删除这个收藏", ui::MUTED_FOREGROUND).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.set_bookmarked(&remove, false, window, cx);
                    },
                )),
            ))
    }
}

impl Render for FavoritePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible(cx);
        let muted = |text: &'static str| div().px_2().py_3().text_size(px(13.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(text);
        let list = if self.paths.is_empty() {
            muted("还没有收藏的目录。进到常用的目录，点右上角的星星收藏。").into_any_element()
        } else if visible.is_empty() {
            muted("没有匹配的目录").into_any_element()
        } else {
            let rows: Vec<_> = visible.iter().enumerate().map(|(position, &i)| self.row(position, &self.paths[i], cx).into_any_element()).collect();
            div().id("favorite-list").max_h(px(360.)).overflow_y_scroll().track_scroll(&self.scroll).flex().flex_col().children(rows).into_any_element()
        };
        div()
            .id("favorite-panel")
            .key_context("FavoritePanel")
            .role(Role::Dialog)
            .aria_label("目录收藏")
            .occlude()
            .w(px(380.))
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &FavoritePrev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &FavoriteNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &FavoriteGo, _, cx| this.go(this.selected, cx)))
            .on_action(cx.listener(|_, _: &FavoriteClose, _, cx| cx.emit(FavoritePanelEvent::Close)))
            .child(
                div()
                    .h(px(34.))
                    .pl_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .child(kit::icon_sized(Icon::Search, 14.).text_color(theme::color(ui::MUTED_FOREGROUND)))
                    .child(div().flex_1().min_w_0().child(Input::new(&self.search)))
                    .children(self.current_toggle(cx)),
            )
            .child(div().h(px(1.)).mx(px(-6.)).bg(theme::color(ui::BORDER)))
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_search_word_has_to_appear_in_the_path() {
        let all = ["/srv/www/Blog".to_string(), "/var/log/nginx".to_string(), "/etc/nginx".to_string()];
        assert_eq!(filter_paths(&all, ""), [0, 1, 2]);
        assert_eq!(filter_paths(&all, "NGINX"), [1, 2]);
        assert_eq!(filter_paths(&all, "nginx log"), [1]);
        assert_eq!(filter_paths(&all, " blog "), [0]);
        assert_eq!(filter_paths(&all, "nginx blog"), Vec::<usize>::new());
    }

    #[test]
    fn cd_quotes_only_the_paths_a_shell_would_mangle() {
        assert_eq!(cd_command("/srv/www/app-1.2_x"), "cd /srv/www/app-1.2_x");
        assert_eq!(cd_command("/Users/me/My Files"), "cd -- '/Users/me/My Files'");
        assert_eq!(cd_command("/tmp/it's"), "cd -- '/tmp/it'\\''s'");
        assert_eq!(cd_command("/tmp/$HOME"), "cd -- '/tmp/$HOME'");
        assert_eq!(cd_command("/data/项目"), "cd -- '/data/项目'");
        assert_eq!(cd_command("-old"), "cd -- '-old'");
    }
}

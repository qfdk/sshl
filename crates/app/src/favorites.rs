//! 目录收藏：顶栏的星星按钮 / ⌘⇧D 打开面板，列出这个连接收藏的目录（本机标签共用一份），回车过去。
//! 和文件管理远程地址栏里的收藏是同一份数据（`config_store::bookmarks`）。
//! 在终端页选中 = 在终端里 `cd` 过去（文件管理跟着终端目录走），在文件管理页选中 = 文件管理直接跳过去。
//! 收藏可以起名字（悬停的铅笔，原地改）：起了名的显示「名字 + 完整路径」，没起的显示「目录名 + 上一级路径」。

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{Input, InputEvent, InputState};
use sshl_core::config_store::{self, Bookmark};

use crate::assets::Icon;
use crate::files::{bookmark_label, bookmark_lines, shell_quote};
use crate::groups;
use crate::prompts::{self, Options};
use crate::settings;
use crate::theme::{self, ui};
use crate::ui as kit;

actions!(favorites, [FavoritePrev, FavoriteNext, FavoriteGo, FavoriteClose]);

pub fn key_bindings() -> Vec<KeyBinding> {
    // 绑在比 gpui-base 输入框更深的上下文里，压过它自己的 上 / 下 / 回车 / Esc
    vec![
        KeyBinding::new("up", FavoritePrev, Some("FavoritePanel > Input")),
        KeyBinding::new("down", FavoriteNext, Some("FavoritePanel > Input")),
        KeyBinding::new("enter", FavoriteGo, Some("FavoritePanel > Input")),
        KeyBinding::new("escape", FavoriteClose, Some("FavoritePanel > Input")),
    ]
}

// ---------------------------------------------------------------- 纯逻辑

/// 按搜索词筛选：空格分开的每个词都要出现在名字或路径里（不分大小写）。返回下标。
pub fn filter_bookmarks(bookmarks: &[Bookmark], query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    bookmarks
        .iter()
        .enumerate()
        .filter(|(_, bookmark)| {
            let haystack = format!("{}\n{}", bookmark.name, bookmark.path).to_lowercase();
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

/// 改名输入框里敲的字 → 存下来的名字：去掉首尾空白、最多 40 个字（同标签改名）；清空或和目录名一样 = 不起名（存空）。
pub fn bookmark_name(input: &str, path: &str) -> String {
    groups::tab_custom_title(input, &bookmark_label(path).0).unwrap_or_default()
}

// ---------------------------------------------------------------- 面板

/// 正在原地改名的那一条。
struct Rename {
    path: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

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
    bookmarks: Vec<Bookmark>,
    rename: Option<Rename>,
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
        let search = kit::input(|| tr!("搜索收藏的目录…"), window, cx);
        let subscription = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
        });
        Self { scope: None, bookmarks: Vec::new(), rename: None, current: None, search, selected: 0, scroll: ScrollHandle::new(), _subscription: subscription }
    }

    /// 打开面板：重新读一遍收藏、清空搜索、焦点给搜索框。`current` 还不知道就先给 None，读到了再 [`Self::set_current`]。
    pub fn open(&mut self, scope: String, current: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = Some(scope);
        self.current = current;
        self.rename = None;
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
            Ok(bookmarks) => self.bookmarks = bookmarks,
            Err(error) => tracing::warn!("load bookmarks: {error}"),
        }
    }

    fn visible(&self, cx: &App) -> Vec<usize> {
        filter_bookmarks(&self.bookmarks, &self.search.read(cx).value())
    }

    fn is_bookmarked(&self, path: &str) -> bool {
        self.bookmarks.iter().any(|bookmark| bookmark.path == path)
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.visible(cx).len();
        if count == 0 || self.rename.is_some() {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn go(&mut self, position: usize, cx: &mut Context<Self>) {
        let visible = self.visible(cx);
        if let Some(bookmark) = visible.get(position).and_then(|&i| self.bookmarks.get(i)) {
            cx.emit(FavoritePanelEvent::Go(bookmark.path.clone()));
        }
    }

    /// 悬停的铅笔：标题原地换成输入框，预填现在显示的名字并全选。
    fn start_rename(&mut self, bookmark: &Bookmark, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_rename(true, window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(bookmark_label(&bookmark.path).0));
        input.update(cx, |input, cx| {
            input.set_value(bookmark_lines(bookmark).0, window, cx);
            input.select_all(window, cx);
            input.focus(window, cx);
        });
        let subscription = cx.subscribe_in(&input, window, |_, input, event: &InputEvent, window, cx| {
            // 失焦 = 确认（点了搜索框、别的行，或面板关了）。等这一轮事件处理完再收；已经被回车 / Esc 收掉的不动
            if let InputEvent::Blur = event {
                let input = input.clone();
                cx.defer_in(window, move |this, window, cx| {
                    if this.rename.as_ref().is_some_and(|rename| rename.input == input) {
                        this.finish_rename(true, window, cx);
                    }
                });
            }
        });
        self.rename = Some(Rename { path: bookmark.path.clone(), input, _subscription: subscription });
        cx.notify();
    }

    /// 收掉改名；`commit` 时存下名字（[`bookmark_name`]）。焦点还在改名框里（回车 / Esc）就交回搜索框，接着用键盘。
    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else { return };
        if commit && let Some(scope) = &self.scope {
            let name = bookmark_name(&rename.input.read(cx).value(), &rename.path);
            if let Err(error) = config_store::rename_bookmark(scope, &rename.path, &name) {
                prompts::alert_later(window, cx, Options::new(tr!("改名失败")).message(error.to_string()));
            }
            self.reload();
            cx.emit(FavoritePanelEvent::Changed);
        }
        if rename.input.read(cx).focus_handle(cx).is_focused(window) {
            let focus = self.search.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn set_bookmarked(&mut self, path: &str, bookmarked: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = &self.scope else { return };
        if let Err(error) = config_store::set_bookmarked(scope, path, bookmarked) {
            prompts::alert_later(window, cx, Options::new(tr!("收藏失败")).message(error.to_string()));
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
        let marked = self.is_bookmarked(&path);
        let (glyph, title, color) = if marked {
            (Icon::StarFilled, tr!("取消收藏 {}", path), ui::PRIMARY)
        } else {
            (Icon::Star, tr!("收藏当前目录 {}", path), ui::MUTED_FOREGROUND)
        };
        Some(
            kit::mini_button("favorite-current", glyph, title, color)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.set_bookmarked(&path, !marked, window, cx))),
        )
    }

    fn row(&self, position: usize, bookmark: &Bookmark, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let path = bookmark.path.as_str();
        let (name, parent) = bookmark_lines(bookmark);
        let selected = position == self.selected;
        let current = self.current.as_deref() == Some(path);
        let renaming = self.rename.as_ref().filter(|rename| rename.path == path).map(|rename| rename.input.clone());
        let editing = renaming.is_some();
        let (edit, remove) = (bookmark.clone(), path.to_string());
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
            // 正在改名的这一行：点在输入框上是放光标，不是进目录
            .when(!editing, |row| row.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(position, cx))))
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
                    .map(|d| match renaming {
                        Some(input) => d.child(
                            // 和标题那一行一样高：改名时这一行不上下跳
                            div()
                                .h(px(21.))
                                .px(px(6.))
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(theme::color(ui::PRIMARY))
                                .bg(theme::color(ui::CARD))
                                .text_size(px(13.))
                                .child(Input::new(&input)),
                        ),
                        None => d.child(
                            div()
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .when(current, |t| t.text_color(theme::color(ui::PRIMARY)))
                                .child(name),
                        ),
                    })
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
            .when(!editing, |row| {
                row.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .opacity(0.)
                        .group_hover("favorite-row", |s| s.opacity(1.))
                        .child(kit::mini_button(("favorite-rename", position), Icon::Pencil, tr!("改名"), ui::MUTED_FOREGROUND).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.start_rename(&edit, window, cx);
                            },
                        )))
                        .child(kit::mini_button(("favorite-remove", position), Icon::X, tr!("删除这个收藏"), ui::MUTED_FOREGROUND).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.set_bookmarked(&remove, false, window, cx);
                            },
                        ))),
                )
            })
    }
}

impl Render for FavoritePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible(cx);
        let muted = |text: &'static str| div().px_2().py_3().text_size(px(13.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(text);
        let list = if self.bookmarks.is_empty() {
            muted(tr!("还没有收藏的目录。进到常用的目录，点右上角的星星收藏。")).into_any_element()
        } else if visible.is_empty() {
            muted(tr!("没有匹配的目录")).into_any_element()
        } else {
            let rows: Vec<_> = visible.iter().enumerate().map(|(position, &i)| self.row(position, &self.bookmarks[i], cx).into_any_element()).collect();
            div().id("favorite-list").max_h(px(360.)).overflow_y_scroll().track_scroll(&self.scroll).flex().flex_col().children(rows).into_any_element()
        };
        div()
            .id("favorite-panel")
            .key_context("FavoritePanel")
            .role(Role::Dialog)
            .aria_label(tr!("目录收藏"))
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
            // 改名框也是这个面板里的输入框：回车 / Esc 先归它（确认 / 取消改名）
            .on_action(cx.listener(|this, _: &FavoriteGo, window, cx| {
                if this.rename.is_some() { this.finish_rename(true, window, cx) } else { this.go(this.selected, cx) }
            }))
            .on_action(cx.listener(|this, _: &FavoriteClose, window, cx| {
                if this.rename.is_some() { this.finish_rename(false, window, cx) } else { cx.emit(FavoritePanelEvent::Close) }
            }))
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

    fn bookmark(path: &str, name: &str) -> Bookmark {
        Bookmark { path: path.into(), name: name.into() }
    }

    #[test]
    fn every_search_word_has_to_appear_in_the_name_or_the_path() {
        let all = [bookmark("/srv/www/Blog", ""), bookmark("/var/log/nginx", "网站日志"), bookmark("/etc/nginx", "")];
        assert_eq!(filter_bookmarks(&all, ""), [0, 1, 2]);
        assert_eq!(filter_bookmarks(&all, "NGINX"), [1, 2]);
        assert_eq!(filter_bookmarks(&all, "nginx log"), [1]);
        assert_eq!(filter_bookmarks(&all, " blog "), [0]);
        assert_eq!(filter_bookmarks(&all, "nginx blog"), Vec::<usize>::new());
        assert_eq!(filter_bookmarks(&all, "日志"), [1], "起的名字也能搜");
        assert_eq!(filter_bookmarks(&all, "网站 nginx"), [1], "名字和路径里各中一个词也算");
    }

    #[test]
    fn a_blank_name_or_the_folder_name_itself_means_no_custom_name() {
        assert_eq!(bookmark_name("  博客  ", "/srv/www/blog"), "博客");
        assert_eq!(bookmark_name("   ", "/srv/www/blog"), "");
        assert_eq!(bookmark_name("blog", "/srv/www/blog"), "", "和目录名一样不算起名");
        assert_eq!(bookmark_name(&"长".repeat(60), "/srv").chars().count(), 40);
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

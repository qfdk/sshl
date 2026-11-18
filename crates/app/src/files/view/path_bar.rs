//! 地址栏：敲路径时的子目录补全（↑↓ / Tab / Esc），和远程目录收藏。
//!
//! 收藏同浏览器地址栏：地址栏右端的星星一点就收藏 / 取消当前目录（⌘D 同）；
//! 点进地址栏，下拉列出收藏的目录，点了过去，悬停的 × 直接删，不用先进那个目录。

use super::load::list_directory;
use super::*;

impl FileManager {
    pub(super) fn path_event(&mut self, kind: PaneKind, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                let value = self.pane(kind).input.read(cx).value().to_string();
                // 跳转后程序写回地址栏（set_path）也会触发 Change：值没变就不是用户在输入，不重新弹补全
                if value == self.pane(kind).path {
                    return;
                }
                let pane = self.pane_mut(kind);
                pane.path = value;
                pane.suggesting = true;
                pane.highlight = None;
                // 一敲字就是在输路径：收藏列表让位给子目录补全
                if kind == PaneKind::Remote {
                    self.bookmark_list = false;
                }
                self.request_listing(kind, window, cx);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if let Some(path) = self.highlighted_bookmark(kind) {
                    self.open_bookmark(path, window, cx);
                    return;
                }
                let suggestions = self.pane(kind).suggestions();
                if let Some(entry) = self.pane(kind).highlight.and_then(|i| suggestions.get(i)).cloned() {
                    self.open_suggestion(kind, &entry, window, cx);
                } else {
                    self.pane_mut(kind).suggesting = false;
                    self.load(kind, None, true, window, cx);
                }
            }
            InputEvent::Focus => {
                self.pane_mut(kind).input_focused = true;
                cx.notify();
            }
            InputEvent::Blur => {
                let pane = self.pane_mut(kind);
                pane.input_focused = false;
                pane.suggesting = false;
                if kind == PaneKind::Remote {
                    self.bookmark_list = false;
                }
                cx.notify();
            }
        }
    }

    /// 只在目录部分变了（敲了分隔符或 Tab 补全）才取列表；同一层里继续输入只在本地过滤。地址栏补全只读不改当前路径。
    pub(super) fn request_listing(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        if kind == PaneKind::Remote && self.session.is_none() {
            return;
        }
        let Some(query) = self.pane(kind).query() else { return };
        let pane = self.pane_mut(kind);
        if pane.listing.as_ref().is_some_and(|(dir, _)| *dir == query.directory) || pane.listing_requested.as_ref() == Some(&query.directory) {
            return;
        }
        pane.listing_requested = Some(query.directory.clone());
        let listing = list_directory(kind, self.session_id(), query.directory.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = listing.await;
            let _ = this.update(cx, |this, cx| {
                let pane = this.pane_mut(kind);
                if pane.listing_requested.as_ref() == Some(&query.directory) {
                    pane.listing_requested = None;
                    if let Ok(entries) = result {
                        pane.listing = Some((query.directory, entries));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn open_suggestion(&mut self, kind: PaneKind, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.pane(kind).query() else { return };
        self.pane_mut(kind).suggesting = false;
        self.load(kind, Some(format!("{}{}", query.base, entry.name)), true, window, cx);
    }

    pub(super) fn path_next(&mut self, kind: PaneKind, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let showing_bookmarks = self.showing_bookmarks(kind);
        if down && !showing_bookmarks && !self.pane(kind).suggesting {
            self.pane_mut(kind).suggesting = true;
            self.request_listing(kind, window, cx);
            cx.notify();
            return;
        }
        let count = if showing_bookmarks { self.bookmarks.len() } else { self.pane(kind).suggestions().len() };
        if count == 0 {
            cx.propagate();
            return;
        }
        let pane = self.pane_mut(kind);
        let last = count - 1;
        pane.highlight = Some(match (pane.highlight, down) {
            (Some(i), true) if i < last => i + 1,
            (_, true) => 0,
            (Some(i), false) if i > 0 => i - 1,
            (_, false) => last,
        });
        cx.notify();
    }

    pub(super) fn path_complete(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.pane(kind);
        let suggestions = pane.suggestions();
        let Some(query) = pane.query().filter(|_| !suggestions.is_empty()) else {
            cx.propagate();
            return;
        };
        if let Some(completed) = tab_completion(&query, &suggestions, pane.highlight) {
            self.set_path(kind, completed, window, cx);
            let pane = self.pane_mut(kind);
            pane.highlight = None;
            pane.suggesting = true;
            self.request_listing(kind, window, cx);
        }
        cx.notify();
    }

    pub(super) fn path_dismiss(&mut self, kind: PaneKind, cx: &mut Context<Self>) {
        if self.showing_bookmarks(kind) {
            self.bookmark_list = false;
            cx.notify();
            return;
        }
        if self.pane(kind).suggestions().is_empty() {
            cx.propagate();
            return;
        }
        self.pane_mut(kind).suggesting = false;
        cx.notify();
    }

    pub fn reload_bookmarks(&mut self) {
        self.bookmarks = match &self.session {
            Some(session) => config_store::bookmarks(&session.connection_id).unwrap_or_else(|error| {
                tracing::warn!("load bookmarks: {error}");
                Vec::new()
            }),
            None => Vec::new(),
        };
        if self.bookmarks.is_empty() {
            self.bookmark_list = false;
        }
    }

    pub(super) fn set_bookmarked(&mut self, path: &str, bookmarked: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| !path.is_empty()) else { return };
        match config_store::set_bookmarked(&session.connection_id, path, bookmarked) {
            Ok(()) => self.reload_bookmarks(),
            Err(error) => prompts::alert_later(window, cx, Options::new(tr!("收藏失败")).message(error.to_string())),
        }
        // 删掉一条后高亮可能越界
        if self.remote.highlight.is_some_and(|i| i >= self.bookmarks.len()) {
            self.remote.highlight = None;
        }
        cx.notify();
    }

    pub(super) fn toggle_bookmark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.remote.path.clone();
        let bookmarked = !self.is_bookmarked(&path);
        self.set_bookmarked(&path, bookmarked, window, cx);
    }

    pub fn open_bookmark(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.bookmark_list = false;
        self.remote.highlight = None;
        self.load(PaneKind::Remote, Some(path), false, window, cx);
    }

    /// 远程栏此刻的目录（还没列出来时为 None）：顶栏的目录收藏拿它当「当前目录」。
    pub fn remote_path(&self) -> Option<String> {
        Some(self.remote.path.clone()).filter(|path| !path.is_empty())
    }

    pub(super) fn is_bookmarked(&self, path: &str) -> bool {
        self.bookmarks.iter().any(|bookmark| bookmark.path == path)
    }

    pub(super) fn showing_bookmarks(&self, kind: PaneKind) -> bool {
        kind == PaneKind::Remote && self.bookmark_list && !self.bookmarks.is_empty() && !self.remote.suggesting
    }

    pub(super) fn highlighted_bookmark(&self, kind: PaneKind) -> Option<String> {
        if !self.showing_bookmarks(kind) {
            return None;
        }
        self.remote.highlight.and_then(|i| self.bookmarks.get(i)).map(|bookmark| bookmark.path.clone())
    }

    /// 地址栏右端的收藏开关：当前目录已收藏是实心蓝色。
    pub(super) fn bookmark_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let marked = self.is_bookmarked(&self.remote.path);
        let label = if marked { tr!("取消收藏当前目录") } else { tr!("收藏当前目录") };
        let tip = if marked { tr!("取消收藏") } else { tr!("收藏当前目录") };
        div()
            .id("path-bookmark")
            .role(Role::Button)
            .aria_label(label)
            .flex_shrink_0()
            .size(px(24.))
            .mr(px(-6.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_color(theme::color(if marked { ui::PRIMARY } else { ui::MUTED_FOREGROUND }))
            .hover(move |s| {
                let s = s.bg(theme::alpha(ui::FOREGROUND, 0.06));
                if marked { s } else { s.text_color(theme::color(ui::FOREGROUND)) }
            })
            .tooltip(move |window, cx| kit::tooltip(crate::keymap::with_shortcut(cx, tip, "files-toggle-bookmark"), window, cx))
            // 点开关不弹收藏列表（地址栏按下会弹）
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_bookmark(window, cx)))
            .child(kit::icon_sized(if marked { Icon::StarFilled } else { Icon::Star }, 15.))
            .into_any_element()
    }

    /// 地址栏下拉里的收藏列表：目录名 + 灰色上级路径，当前目录标蓝，悬停出 ×。
    pub(super) fn bookmark_dropdown(&self, cx: &mut Context<Self>) -> AnyElement {
        let highlight = self.remote.highlight;
        let current = self.remote.path.clone();
        div()
            .id("bookmark-list")
            .role(Role::ListBox)
            .aria_label(tr!("收藏的目录"))
            .absolute()
            .left_0()
            .right_0()
            .top_full()
            .mt_1()
            .max_h(px(256.))
            .overflow_y_scroll()
            .p_1()
            .rounded(px(10.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow_md()
            .occlude()
            .children(self.bookmarks.iter().enumerate().map(|(index, bookmark)| {
                let path = &bookmark.path;
                let (name, parent) = bookmark_lines(bookmark);
                let highlighted = highlight == Some(index);
                let here = *path == current;
                let group = SharedString::from(format!("bookmark-{index}"));
                let (open, remove) = (path.clone(), path.clone());
                div()
                    .id(("bookmark", index))
                    .group(group.clone())
                    .role(Role::ListBoxOption)
                    .aria_label(SharedString::from(path.clone()))
                    .min_h(px(30.))
                    .py(px(4.))
                    .pl_2()
                    .pr(px(2.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(highlighted, |d| d.bg(theme::color(ui::ACCENT)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.remote.highlight = Some(index);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_bookmark(open.clone(), window, cx)))
                    .child(kit::icon_sized(Icon::Folder, 14.).text_color(theme::color(ui::PRIMARY)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(220.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(if here { ui::PRIMARY } else { ui::FOREGROUND }))
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(MONO_FONT)
                            .text_size(px(11.))
                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                            .child(parent),
                    )
                    .child(
                        kit::mini_button(("bookmark-remove", index), Icon::X, tr!("删除这个收藏"), ui::MUTED_FOREGROUND)
                            .flex_shrink_0()
                            .opacity(if highlighted { 1. } else { 0. })
                            .group_hover(group, |s| s.opacity(1.))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.set_bookmarked(&remove, false, window, cx);
                            })),
                    )
            }))
            .into_any_element()
    }
}

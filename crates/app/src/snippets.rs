//! 命令片段：常用命令存起来，一点就发到终端。顶栏按钮 / ⌘⇧P 打开面板，回车执行、⇧回车只填入不执行。
//! 片段可以所有服务器通用，也可以只属于某个连接（`config_store::snippets`）。

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions,
    div, prelude::FluentBuilder, px,
};
use gpui_base::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use sshl_core::config_store::{self, SaveSnippetInput, Snippet};

use crate::assets::Icon;
use crate::prompts::{self, Options};
use crate::settings;
use crate::theme::{self, ui};
use crate::ui::{self as kit, Variant};

actions!(snippets, [SnippetPrev, SnippetNext, SnippetRun, SnippetInsert, SnippetClose, SnippetSave]);

pub fn bind_keys(cx: &mut App) {
    // 绑在比 gpui-base 输入框更深的上下文里，压过它自己的 上 / 下 / 回车 / Esc
    cx.bind_keys([
        KeyBinding::new("up", SnippetPrev, Some("SnippetPanel > Input")),
        KeyBinding::new("down", SnippetNext, Some("SnippetPanel > Input")),
        KeyBinding::new("enter", SnippetRun, Some("SnippetPanel > Input")),
        KeyBinding::new("shift-enter", SnippetInsert, Some("SnippetPanel > Input")),
        KeyBinding::new("escape", SnippetClose, Some("SnippetPanel > Input")),
        KeyBinding::new("cmd-enter", SnippetSave, Some("SnippetDialog > Input")),
        KeyBinding::new("escape", SnippetClose, Some("SnippetDialog > Input")),
        // 点了「所有服务器 / 仅…」之后焦点在对话框本身上
        KeyBinding::new("cmd-enter", SnippetSave, Some("SnippetDialog")),
        KeyBinding::new("escape", SnippetClose, Some("SnippetDialog")),
    ]);
}

// ---------------------------------------------------------------- 纯逻辑

/// 按搜索词筛选：空格分开的每个词都要出现在名称或命令里（不分大小写）。返回下标。
pub fn filter_snippets(snippets: &[Snippet], query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    snippets
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            let haystack = format!("{}\n{}", s.name, s.command).to_lowercase();
            terms.iter().all(|term| haystack.contains(term.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// 列表里一行的标题和小字：有名称时标题是名称、小字是命令；没名称时标题就是命令。
/// 多行命令只显示第一行，后面跟「…」。
pub fn snippet_lines(snippet: &Snippet) -> (String, Option<String>) {
    let mut lines = snippet.command.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or_default();
    let preview = if lines.next().is_some() { format!("{first} …") } else { first.to_string() };
    if snippet.name.is_empty() { (preview, None) } else { (snippet.name.clone(), Some(preview)) }
}

// ---------------------------------------------------------------- 面板

pub enum SnippetPanelEvent {
    /// 发到终端；`execute` 为 false 时只填入、不回车。
    Run { command: String, execute: bool },
    New,
    Edit(Snippet),
    Close,
}

pub struct SnippetPanel {
    connection_id: Option<String>,
    snippets: Vec<Snippet>,
    search: Entity<InputState>,
    /// 筛选后列表里的位置。
    selected: usize,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<SnippetPanelEvent> for SnippetPanel {}

impl SnippetPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索命令片段…"));
        let subscription = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
        });
        Self { connection_id: None, snippets: Vec::new(), search, selected: 0, scroll: ScrollHandle::new(), _subscription: subscription }
    }

    /// 打开面板：重新读一遍片段、清空搜索、焦点给搜索框。
    pub fn open(&mut self, connection_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.connection_id = Some(connection_id);
        self.reload();
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        let focus = self.search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub fn reload(&mut self) {
        let Some(connection_id) = &self.connection_id else { return };
        match config_store::snippets(connection_id) {
            Ok(snippets) => self.snippets = snippets,
            Err(error) => tracing::warn!("load snippets: {error}"),
        }
    }

    fn visible(&self, cx: &App) -> Vec<usize> {
        filter_snippets(&self.snippets, &self.search.read(cx).value())
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

    fn run(&mut self, position: usize, execute: bool, cx: &mut Context<Self>) {
        let visible = self.visible(cx);
        if let Some(snippet) = visible.get(position).and_then(|&i| self.snippets.get(i)) {
            cx.emit(SnippetPanelEvent::Run { command: snippet.command.clone(), execute });
        }
    }

    fn delete(&mut self, snippet: Snippet, window: &mut Window, cx: &mut Context<Self>) {
        let (title, _) = snippet_lines(&snippet);
        let answer = prompts::confirm(window, cx, Options::new("删除命令片段").message(format!("确定要删除「{title}」吗？")).confirm("删除"));
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await.unwrap_or(false) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = config_store::delete_snippet(snippet.id) {
                    prompts::alert_later(window, cx, Options::new("删除失败").message(error.to_string()));
                }
                this.reload();
                let count = this.visible(cx).len();
                this.selected = this.selected.min(count.saturating_sub(1));
                // 确认框关掉后焦点回到搜索框，接着用键盘
                let focus = this.search.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn row(&self, position: usize, snippet: &Snippet, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, detail) = snippet_lines(snippet);
        let selected = position == self.selected;
        let named = detail.is_some();
        let (edit, delete) = (snippet.clone(), snippet.clone());
        div()
            .id(("snippet", position))
            .group("snippet-row")
            .role(Role::MenuItem)
            .aria_label(SharedString::from(title.clone()))
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
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| this.run(position, !event.modifiers().shift, cx)))
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
                            .map(|t| if named { t.font_weight(FontWeight::MEDIUM) } else { t.font_family(settings::DEFAULT_FONT_FAMILY) })
                            .child(title),
                    )
                    .when_some(detail, |d, detail| {
                        d.child(
                            div()
                                .truncate()
                                .font_family(settings::DEFAULT_FONT_FAMILY)
                                .text_size(px(11.))
                                .text_color(theme::color(ui::MUTED_FOREGROUND))
                                .child(detail),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .opacity(0.)
                    .group_hover("snippet-row", |s| s.opacity(1.))
                    .child(kit::mini_button(("snippet-edit", position), Icon::Pencil, "编辑", ui::MUTED_FOREGROUND).on_click(cx.listener(
                        move |_, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            cx.emit(SnippetPanelEvent::Edit(edit.clone()));
                        },
                    )))
                    .child(kit::mini_button(("snippet-delete", position), Icon::Trash, "删除", ui::MUTED_FOREGROUND).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.delete(delete.clone(), window, cx);
                        },
                    ))),
            )
    }
}

impl Render for SnippetPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible(cx);
        let muted = |text: &'static str| div().px_2().py_3().text_size(px(13.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(text);
        let list = if self.snippets.is_empty() {
            muted("还没有命令片段。点 ＋ 把常用命令存起来。").into_any_element()
        } else if visible.is_empty() {
            muted("没有匹配的命令片段").into_any_element()
        } else {
            let rows: Vec<_> = visible.iter().enumerate().map(|(position, &i)| self.row(position, &self.snippets[i], cx).into_any_element()).collect();
            div().id("snippet-list").max_h(px(360.)).overflow_y_scroll().track_scroll(&self.scroll).flex().flex_col().children(rows).into_any_element()
        };
        div()
            .id("snippet-panel")
            .key_context("SnippetPanel")
            .role(Role::Dialog)
            .aria_label("命令片段")
            .occlude()
            .w(px(420.))
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &SnippetPrev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SnippetNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SnippetRun, _, cx| this.run(this.selected, true, cx)))
            .on_action(cx.listener(|this, _: &SnippetInsert, _, cx| this.run(this.selected, false, cx)))
            .on_action(cx.listener(|_, _: &SnippetClose, _, cx| cx.emit(SnippetPanelEvent::Close)))
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
                    .child(
                        kit::mini_button("snippet-new", Icon::Plus, "新建命令片段", ui::MUTED_FOREGROUND)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(SnippetPanelEvent::New))),
                    ),
            )
            .child(div().h(px(1.)).mx(px(-6.)).bg(theme::color(ui::BORDER)))
            .child(list)
            .when(!visible.is_empty(), |panel| {
                panel.child(
                    div()
                        .px_2()
                        .pt(px(2.))
                        .text_size(px(11.))
                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                        .child("↵ 执行　⇧↵ 只填入"),
                )
            })
    }
}

// ---------------------------------------------------------------- 新建 / 编辑

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    ThisServer,
}

pub enum SnippetDialogEvent {
    Saved,
    Close,
}

pub struct SnippetDialog {
    id: Option<i64>,
    command: Entity<TextareaState>,
    name: Entity<InputState>,
    connection_id: String,
    connection_name: SharedString,
    scope: Scope,
    error: Option<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<SnippetDialogEvent> for SnippetDialog {}

impl Focusable for SnippetDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SnippetDialog {
    /// `editing` 为 None 时新建；新建默认所有服务器通用。
    pub fn new(editing: Option<Snippet>, connection_id: String, connection_name: SharedString, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let command = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 10).placeholder("命令，可以多行"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("名称（可选）"));
        let mut scope = Scope::All;
        let mut id = None;
        if let Some(snippet) = editing {
            id = Some(snippet.id);
            command.update(cx, |state, cx| state.set_value(snippet.command.clone(), window, cx));
            name.update(cx, |state, cx| state.set_value(snippet.name.clone(), window, cx));
            if snippet.connection_id.is_some() {
                scope = Scope::ThisServer;
            }
        }
        let focus = command.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Self { id, command, name, connection_id, connection_name, scope, error: None, focus: cx.focus_handle() }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let input = SaveSnippetInput {
            id: self.id,
            name: self.name.read(cx).value().to_string(),
            command: self.command.read(cx).value().to_string(),
            connection_id: (self.scope == Scope::ThisServer).then(|| self.connection_id.clone()),
        };
        match config_store::save_snippet(input) {
            Ok(_) => cx.emit(SnippetDialogEvent::Saved),
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }
}

impl Render for SnippetDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let body = div()
            .key_context("SnippetDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SnippetSave, _, cx| this.save(cx)))
            .on_action(cx.listener(|_, _: &SnippetClose, _, cx| cx.emit(SnippetDialogEvent::Close)))
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(if self.id.is_some() { "编辑命令片段" } else { "新建命令片段" }))
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme::color(ui::BORDER))
                            .font_family(settings::DEFAULT_FONT_FAMILY)
                            .text_size(px(13.))
                            .child(Textarea::new(&self.command)),
                    )
                    .child(kit::text_field(&self.name, false))
                    .child(kit::segmented(
                        "snippet-scope",
                        &[(Scope::All, None, "所有服务器".into()), (Scope::ThisServer, None, format!("仅 {}", self.connection_name).into())],
                        self.scope,
                        move |scope, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.scope = scope;
                                cx.notify();
                            });
                        },
                    ))
                    .when_some(self.error.clone(), |d, error| d.child(div().text_sm().text_color(theme::color(ui::DESTRUCTIVE)).child(error))),
            )
            .child(
                kit::modal_footer()
                    .child(kit::labeled_button("snippet-cancel", Variant::Outline, None, "取消").on_click(cx.listener(|_, _, _, cx| cx.emit(SnippetDialogEvent::Close))))
                    .child(kit::labeled_button("snippet-save", Variant::Primary, None, "保存").on_click(cx.listener(|this, _, _, cx| this.save(cx)))),
            );
        kit::modal("snippet-dialog", 480., body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(name: &str, command: &str) -> Snippet {
        Snippet { id: 0, name: name.into(), command: command.into(), connection_id: None }
    }

    #[test]
    fn every_search_word_has_to_match_the_name_or_the_command() {
        let all = [snippet("重启 Nginx", "systemctl restart nginx"), snippet("", "docker ps -a"), snippet("磁盘", "df -h")];
        assert_eq!(filter_snippets(&all, ""), [0, 1, 2]);
        assert_eq!(filter_snippets(&all, "NGINX"), [0]);
        assert_eq!(filter_snippets(&all, "重启 restart"), [0], "名称和命令里各中一个词也算");
        assert_eq!(filter_snippets(&all, "docker nginx"), Vec::<usize>::new());
        assert_eq!(filter_snippets(&all, "  ps  "), [1]);
    }

    #[test]
    fn a_row_shows_the_name_over_the_first_line_of_the_command() {
        assert_eq!(snippet_lines(&snippet("部署", "cd /srv/app\ngit pull\n")), ("部署".into(), Some("cd /srv/app …".into())));
        assert_eq!(snippet_lines(&snippet("", " uptime")), ("uptime".into(), None));
        assert_eq!(snippet_lines(&snippet("", "\n  cd /tmp\n\n")), ("cd /tmp".into(), None), "空行不算多行");
    }
}

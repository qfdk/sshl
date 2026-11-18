//! 文件管理视图：这里是状态（两栏、会话、队列、弹出的东西）和整体布局；各部分的做法在 `view/` 下面：
//! 列目录（load）、地址栏和收藏（path_bar）、一栏的画法（pane）、右键菜单（menu）、删除（ops）、
//! 新建文件夹 / 改权限（dialogs）、传输（transfer）、拖放（dragging）、跟随终端用户（follow）。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, AppContext, AsyncWindowContext, BoxShadow, Bounds, ClickEvent, Context,
    DragMoveEvent, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathPromptOptions, Pixels, Point, Render, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Transformation, WeakEntity, Window, actions, anchored, canvas,
    deferred, div, list, percentage, point, prelude::FluentBuilder, px, svg,
};
use gpui_base::input::{Input, InputEvent, InputState};
use sshl_core::config_store;
use sshl_core::events::CoreEvent;
use sshl_core::{local_fs, sftp};

use super::completion::{CompletionQuery, match_directories, split_completion_path, tab_completion};
use super::delete::{delete_prompt, remote_delete_commands};
use super::drag::{DragSource, DropKind, DropTarget, drag_items, drag_label, plan_drop};
use super::identity::{
    FileIdentity, SessionRecord, SyncStatus, TerminalIdentity, Via, record_sync_result, sync_file_user,
};
use super::model::{
    FileEntry, ListCache, PaneKind, basename, bookmark_lines, checks_to_permissions, format_date, format_file_size,
    format_permissions, is_root_path, join_path, normalize_remote_path, parent_path, permissions_to_checks,
    readable_permissions, remote_home_from, valid_octal, visible_files,
};
use super::progress::{SpeedMeter, TransferProgress, TransferStatus, describe_progress};
use super::sudo::{Outcome, Secret, saved_password_choices, sudo_error_message, upload_with_sudo_fallback};
use crate::assets::Icon;
use crate::backend;
use crate::prompts::{self, Choice, Options, PasswordAnswer};
use crate::theme::{self, ui};
use crate::ui::{self as kit, MenuEntry, MenuItem, Variant};

actions!(files, [SelectAllFiles, ClearFileSelection, PathNext, PathPrev, PathComplete, PathDismiss, ToggleHiddenFiles, ToggleBookmark, RenameFile, ConfirmRename, CancelRename]);

mod dialogs;
mod dragging;
mod follow;
mod load;
mod menu;
mod open;
mod ops;
mod pane;
mod path_bar;
mod rename;
mod transfer;

use dialogs::{NewFolderDialog, PermissionDialog};
use dragging::DragState;
use open::OpenedFile;
use rename::RenameState;
use transfer::Job;

/// 文件管理用的会话：切会话 / 断开时由工作区设置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSession {
    pub session_id: String,
    pub connection_id: String,
    pub name: SharedString,
}

pub enum FileManagerEvent {
    /// 断开后的空状态里点了「重新连接」（连接 id）。
    Reconnect(String),
}

const PANE_MIN_WIDTH: f32 = 220.;
const DIVIDER_WIDTH: f32 = 10.;

const MONO_FONT: &str = "Menlo";

/// 文件列表（不含地址栏、对话框）：↵ 改名只在这里管用。
pub const FILE_LIST_CONTEXT: &str = "FileList";

/// 写死的按键（⌘A / ⌘⇧. / ⌘D / ↵ 能在设置里改，在 `keymap.rs`）。要晚于 gpui-base 绑：同一节点上后绑的优先，
/// 地址栏里的方向键 / Tab / Esc 先给补全。改名框里的 ↵ / Esc 也要自己绑：gpui-base 的单行输入框处理完回车还让它
/// 往外传，传到列表上就成了「再改一次名」。
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("escape", ClearFileSelection, Some("FileManager")),
        KeyBinding::new("enter", ConfirmRename, Some("FileRename > Input")),
        KeyBinding::new("escape", CancelRename, Some("FileRename > Input")),
        KeyBinding::new("down", PathNext, Some("FilePathBar > Input")),
        KeyBinding::new("up", PathPrev, Some("FilePathBar > Input")),
        KeyBinding::new("tab", PathComplete, Some("FilePathBar > Input")),
        KeyBinding::new("escape", PathDismiss, Some("FilePathBar > Input")),
    ]
}

/// 远程栏没东西可列时的原因。存原因不存文字：换语言后提示跟着换。
#[derive(Clone, Copy, PartialEq)]
enum RemoteEmpty {
    NotConnected,
    Disconnected,
}

impl RemoteEmpty {
    fn text(self) -> &'static str {
        match self {
            RemoteEmpty::NotConnected => tr!("请先连接到服务器"),
            RemoteEmpty::Disconnected => tr!("连接已断开，请重新连接到服务器"),
        }
    }
}

struct Pane {
    kind: PaneKind,
    /// 地址栏里的路径（敲字时就跟着变）。
    path: String,
    files: Vec<FileEntry>,
    loading: bool,
    error: Option<String>,
    request: u64,
    input: Entity<InputState>,
    input_focused: bool,
    /// 补全只在用户在地址栏里敲字后出现；双击进目录这类从外部改路径不弹。
    suggesting: bool,
    highlight: Option<usize>,
    listing: Option<(String, Vec<FileEntry>)>,
    listing_requested: Option<String>,
    list_focus: FocusHandle,
    /// 文件列表（行高跟着内容：名字长了折行）。
    rows: kit::Rows,
    /// 上次进入的目录：进入新目录时回到顶部，同目录刷新保留滚动位置。
    listed_path: String,
    selected: HashSet<String>,
    /// 这个目录里没显示的隐藏文件（点开头）个数。
    hidden: usize,
}

impl Pane {
    fn new(kind: PaneKind, window: &mut Window, cx: &mut Context<FileManager>) -> Self {
        Self {
            kind,
            path: String::new(),
            files: Vec::new(),
            loading: false,
            error: None,
            request: 0,
            input: cx.new(|cx| InputState::new(window, cx)),
            input_focused: false,
            suggesting: false,
            highlight: None,
            listing: None,
            listing_requested: None,
            list_focus: cx.focus_handle(),
            rows: kit::Rows::default(),
            listed_path: String::new(),
            selected: HashSet::new(),
            hidden: 0,
        }
    }

    /// 非根目录时最前面有一行「..」。
    fn parent_offset(&self) -> usize {
        usize::from(!is_root_path(&self.path, self.kind))
    }

    fn row_count(&self) -> usize {
        self.files.len() + self.parent_offset()
    }

    /// 第 ix 行（含「..」）。
    fn row(&self, ix: usize) -> Option<FileEntry> {
        let offset = self.parent_offset();
        if ix < offset {
            return Some(FileEntry { name: "..".into(), path: String::new(), is_directory: true, size: 0, modified: None, permissions: None, owner: None });
        }
        self.files.get(ix - offset).cloned()
    }

    fn query(&self) -> Option<CompletionQuery> {
        if !self.suggesting {
            return None;
        }
        split_completion_path(&self.path, self.kind)
    }

    fn suggestions(&self) -> Vec<FileEntry> {
        let Some(query) = self.query() else { return Vec::new() };
        match &self.listing {
            Some((directory, entries)) if *directory == query.directory => match_directories(entries, &query.prefix),
            _ => Vec::new(),
        }
    }
}

struct MenuState {
    pane: PaneKind,
    position: Point<Pixels>,
    target: Option<FileEntry>,
}

struct Resize {
    start_x: Pixels,
    local_width: f32,
    remote_width: f32,
    total_flex: f32,
}

pub struct FileManager {
    local: Pane,
    remote: Pane,
    session: Option<FileSession>,
    records: HashMap<String, SessionRecord>,
    local_cache: ListCache,
    remote_cache: ListCache,
    last_selected: Option<(PaneKind, usize)>,
    /// 最近按过的一侧：焦点不在列表上时 ⌘A 全选这一侧；都没碰过时已连接选远程，否则选本地。
    active_pane: Option<PaneKind>,
    remote_empty: Option<RemoteEmpty>,
    remote_empty_connection: Option<String>,
    active: bool,
    syncing: bool,
    /// 身份同步完成后要接着做的事（如拖到终端的上传）：文件身份和终端一致了才做，取消 / 失败就丢掉。
    after_sync: Vec<AfterSync>,
    /// 排队等着传的：一次只传一个，后加的接在后面（多批同时传会抢带宽、进度条来回跳）。
    queue: VecDeque<Job>,
    /// 队列在跑。
    worker: bool,
    /// 这一轮（队列从空到空）传完了几个：传完的提示说「3 个已上传到 …」。
    finished: usize,
    /// 这一轮里同名冲突勾了「后面也这样」：Some(true) 都覆盖，Some(false) 都跳过。
    on_conflict: Option<bool>,
    /// 正在传的那一个的停止开关；None = 没在传。
    current: Option<Arc<AtomicBool>>,
    speed: SpeedMeter,
    status: TransferStatus,
    hide_status: Option<Task<()>>,
    menu: Option<MenuState>,
    /// 当前连接收藏的远程目录（按收藏先后）。
    bookmarks: Vec<config_store::Bookmark>,
    /// 远程地址栏下拉列着收藏（点进地址栏时出现，一敲字换成子目录补全）。
    bookmark_list: bool,
    permission: Option<PermissionDialog>,
    new_folder: Option<NewFolderDialog>,
    rename: Option<RenameState>,
    /// 从服务器打开、保存后要传回去的文件。
    opened: Vec<OpenedFile>,
    /// 在每秒看一眼 `opened`。
    watching: bool,
    dragging: Option<DragState>,
    hover_pane: Option<PaneKind>,
    hover_row: Option<(PaneKind, String)>,
    split_bounds: Option<Bounds<Pixels>>,
    local_flex: f32,
    remote_flex: f32,
    resize: Option<Resize>,
    focus: FocusHandle,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<FileManagerEvent> for FileManager {}

impl Focusable for FileManager {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn spinner(size: f32) -> impl IntoElement {
    svg()
        .path(Icon::LoaderCircle.path())
        .size(px(size))
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .with_animation("spin", Animation::new(Duration::from_secs(1)).repeat(), |svg, delta| {
            svg.with_transformation(Transformation::rotate(percentage(delta)))
        })
}

fn alert(cx: &mut AsyncWindowContext, title: impl Into<SharedString>, message: impl Into<SharedString>) {
    let options = Options::new(title).message(message);
    let _ = cx.update(|window, cx| prompts::alert_later(window, cx, options));
}

/// 身份同步完成后接着做的事（见 `FileManager::after_sync`）。
type AfterSync = Box<dyn FnOnce(&mut FileManager, &mut Window, &mut Context<FileManager>)>;

impl FileManager {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let local = Pane::new(PaneKind::Local, window, cx);
        let remote = Pane::new(PaneKind::Remote, window, cx);
        remote.input.update(cx, |input, cx| input.set_disabled(true, cx));
        let subscriptions = [
            cx.subscribe_in(&local.input, window, |this, _, event: &InputEvent, window, cx| this.path_event(PaneKind::Local, event, window, cx)),
            cx.subscribe_in(&remote.input, window, |this, _, event: &InputEvent, window, cx| this.path_event(PaneKind::Remote, event, window, cx)),
        ];
        let local_flex = crate::settings::get(cx).local_pane_flex.filter(|f| *f > 0.0).unwrap_or(1.0);
        Self {
            local,
            remote,
            session: None,
            records: HashMap::new(),
            local_cache: ListCache::new(50),
            remote_cache: ListCache::new(50),
            last_selected: None,
            active_pane: None,
            remote_empty: Some(RemoteEmpty::NotConnected),
            remote_empty_connection: None,
            active: false,
            syncing: false,
            after_sync: Vec::new(),
            queue: VecDeque::new(),
            worker: false,
            finished: 0,
            on_conflict: None,
            current: None,
            speed: SpeedMeter::default(),
            status: TransferStatus::default(),
            hide_status: None,
            menu: None,
            bookmarks: Vec::new(),
            bookmark_list: false,
            permission: None,
            new_folder: None,
            rename: None,
            opened: Vec::new(),
            watching: false,
            dragging: None,
            hover_pane: None,
            hover_row: None,
            split_bounds: None,
            local_flex,
            remote_flex: 1.0,
            resize: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    // ---------------------------------------------------------------- 对外接口

    /// 文件管理标签显示 / 隐藏。显示时加载两侧并同步终端用户。
    pub fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        if !active {
            self.menu = None;
            cx.notify();
            return;
        }
        self.load_local_initial(window, cx);
        self.load_remote_initial(window, cx);
        if self.session.is_some() {
            self.sync_identity(false, window, cx);
        }
        cx.notify();
    }

    /// 切会话 / 断开（None）。
    pub fn set_session(&mut self, session: Option<FileSession>, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.as_ref().map(|s| &s.session_id) == session.as_ref().map(|s| &s.session_id) {
            self.session = session;
            return;
        }
        self.session = session.clone();
        self.bookmark_list = false;
        self.reload_bookmarks();
        self.remote.request += 1;
        self.remote.files.clear();
        self.remote.hidden = 0;
        self.remote.error = None;
        self.remote.loading = false;
        self.remote.suggesting = false;
        self.remote.listing = None;
        self.remote.listing_requested = None;
        self.remote.selected.clear();
        self.remote_cache.clear();
        self.remote_empty_connection = None;
        self.remote.input.update(cx, |input, cx| input.set_disabled(session.is_none(), cx));
        match session {
            Some(session) => {
                let record = self.records.entry(session.session_id.clone()).or_default();
                let path = record.remote_path.clone();
                self.remote_empty = None;
                self.set_path(PaneKind::Remote, path, window, cx);
                if self.active {
                    self.load_remote_initial(window, cx);
                    self.sync_identity(false, window, cx);
                }
            }
            None => {
                self.remote_empty = Some(RemoteEmpty::NotConnected);
                self.set_path(PaneKind::Remote, String::new(), window, cx);
            }
        }
        cx.notify();
    }

    /// 断开 / 失效后的远程空状态；带连接 id 时显示「重新连接」。
    pub fn show_disconnected(&mut self, connection_id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.remote_empty_connection = connection_id;
        self.remote_empty = Some(RemoteEmpty::Disconnected);
        self.remote.request += 1;
        self.remote.files.clear();
        self.remote.hidden = 0;
        self.remote.loading = false;
        self.remote.listing = None;
        self.remote.listing_requested = None;
        self.remote.selected.clear();
        self.remote_cache.clear();
        self.set_path(PaneKind::Remote, String::new(), window, cx);
        cx.notify();
    }

    /// 会话被移除（断开 / 远端关闭 / 失效丢弃）：丢掉它的文件管理身份、拒绝记录和远程目录。
    pub fn forget_session(&mut self, session_id: &str) {
        self.records.remove(session_id);
        self.forget_opened(session_id);
    }

    // ---------------------------------------------------------------- 状态工具

    fn pane(&self, kind: PaneKind) -> &Pane {
        match kind {
            PaneKind::Local => &self.local,
            PaneKind::Remote => &self.remote,
        }
    }

    fn pane_mut(&mut self, kind: PaneKind) -> &mut Pane {
        match kind {
            PaneKind::Local => &mut self.local,
            PaneKind::Remote => &mut self.remote,
        }
    }

    fn session_id(&self) -> Option<String> {
        self.session.as_ref().map(|s| s.session_id.clone())
    }

    fn identity(&self) -> Option<&FileIdentity> {
        let id = self.session.as_ref()?.session_id.as_str();
        self.records.get(id)?.identity.as_ref()
    }

    fn set_path(&mut self, kind: PaneKind, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.pane_mut(kind);
        pane.path = path.clone();
        pane.input.update(cx, |input, cx| input.set_value(path, window, cx));
    }

    fn cache_key(&self, kind: PaneKind, path: &str) -> String {
        match (kind, &self.session) {
            (PaneKind::Remote, Some(session)) => format!("{}:{}", session.session_id, normalize_remote_path(path)),
            _ => path.to_string(),
        }
    }

    fn invalidate(&mut self, kind: PaneKind) {
        let key = self.cache_key(kind, &self.pane(kind).path.clone());
        match kind {
            PaneKind::Local => self.local_cache.delete(&key),
            PaneKind::Remote => self.remote_cache.delete(&key),
        }
    }

    // ---------------------------------------------------------------- 选择

    fn select_row(&mut self, kind: PaneKind, entry: &FileEntry, index: usize, modifiers: Modifiers) {
        let mut next = HashSet::new();
        if modifiers.platform || modifiers.control {
            next = self.pane(kind).selected.clone();
            if !next.remove(&entry.path) {
                next.insert(entry.path.clone());
            }
        } else if let (true, Some((pane, last))) = (modifiers.shift, self.last_selected) {
            if pane == kind {
                let files = &self.pane(kind).files;
                let (first, end) = (last.min(index), last.max(index));
                for item in files.iter().skip(first).take(end + 1 - first) {
                    next.insert(item.path.clone());
                }
            } else {
                next.insert(entry.path.clone());
            }
        } else {
            next.insert(entry.path.clone());
        }
        self.pane_mut(kind).selected = next;
        self.pane_mut(kind.other()).selected.clear();
        self.last_selected = Some((kind, index));
    }

    /// 按键（⌘A、↵）管哪一栏：焦点在哪个列表上就是哪个；都不在时是最近按过的一侧，都没碰过时已连接选远程，否则选本地。
    fn focused_pane(&self, window: &Window) -> PaneKind {
        if self.local.list_focus.is_focused(window) {
            PaneKind::Local
        } else if self.remote.list_focus.is_focused(window) {
            PaneKind::Remote
        } else {
            self.active_pane.unwrap_or(if self.session.is_some() { PaneKind::Remote } else { PaneKind::Local })
        }
    }

    /// 全选当前目录的全部条目（含没画出来的行），并清空另一侧，与点选规则一致。
    fn select_all(&mut self, _: &SelectAllFiles, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.focused_pane(window);
        let all = self.pane(kind).files.iter().map(|f| f.path.clone()).collect();
        self.pane_mut(kind).selected = all;
        self.pane_mut(kind.other()).selected.clear();
        cx.notify();
    }

    fn clear_selection(&mut self, _: &ClearFileSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some()
            || std::mem::take(&mut self.bookmark_list)
            || self.new_folder.take().is_some()
            || self.permission.take().is_some()
        {
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if self.local.input_focused || self.remote.input_focused {
            cx.propagate();
            return;
        }
        self.local.selected.clear();
        self.remote.selected.clear();
        cx.notify();
    }

    fn selected_items(&self, kind: PaneKind) -> Vec<FileEntry> {
        let pane = self.pane(kind);
        pane.files.iter().filter(|f| pane.selected.contains(&f.path)).cloned().collect()
    }

    fn ensure_context_selection(&mut self, kind: PaneKind, entry: &FileEntry) {
        if !self.pane(kind).selected.contains(&entry.path) {
            self.pane_mut(kind).selected = HashSet::from([entry.path.clone()]);
            self.pane_mut(kind.other()).selected.clear();
        }
    }

    /// 显示 / 隐藏点开头的文件（⌘⇧.）。两侧都从缓存重新套用，不重新列目录。
    fn toggle_hidden_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::settings::update(cx, |s| s.show_hidden_files = !s.show_hidden_files);
        self.load(PaneKind::Local, None, false, window, cx);
        if self.session.is_some() && !self.remote.path.is_empty() {
            self.load(PaneKind::Remote, None, false, window, cx);
        }
    }

    // ---------------------------------------------------------------- 面板宽度

    fn start_resize(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.split_bounds else { return };
        let available = f32::from(bounds.size.width) - DIVIDER_WIDTH;
        let total_flex = self.local_flex + self.remote_flex;
        let local_width = available * self.local_flex / total_flex;
        self.resize = Some(Resize { start_x: position.x, local_width, remote_width: available - local_width, total_flex });
        cx.notify();
    }

    fn resize_to(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(resize) = &self.resize else { return };
        let total = resize.local_width + resize.remote_width;
        let width = (resize.local_width + f32::from(x - resize.start_x)).max(PANE_MIN_WIDTH).min(total - PANE_MIN_WIDTH);
        let ratio = width / total;
        self.local_flex = ratio * resize.total_flex;
        self.remote_flex = resize.total_flex - self.local_flex;
        cx.notify();
    }

    /// 一栏里列表的宽度（两栏按 flex 分，减去 1px 边框 ×2）；还没量到时 None。
    fn pane_width(&self, kind: PaneKind) -> Option<f32> {
        let bounds = self.split_bounds?;
        let available = f32::from(bounds.size.width) - DIVIDER_WIDTH;
        let flex = if kind == PaneKind::Local { self.local_flex } else { self.remote_flex };
        Some(available * flex / (self.local_flex + self.remote_flex) - 2.)
    }

    fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resize.take().is_some() {
            let flex = self.local_flex;
            crate::settings::update(cx, |settings| settings.local_pane_flex = Some(flex));
            cx.notify();
        }
    }

}

impl Render for FileManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.settle_rename(window, cx);
        let weak = cx.entity().downgrade();
        let resizing = self.resize.is_some();
        let local = self.render_pane(PaneKind::Local, window, cx);
        let remote = self.render_pane(PaneKind::Remote, window, cx);
        let menu = self.menu.as_ref().map(|menu| {
            let entries = self.menu_entries(menu, cx);
            let dismiss_weak = cx.entity().downgrade();
            let dismiss = move |_: &mut Window, cx: &mut App| {
                let _ = dismiss_weak.update(cx, |this, cx| {
                    this.menu = None;
                    cx.notify();
                });
            };
            let outside = dismiss.clone();
            deferred(
                anchored().position(menu.position).snap_to_window().child(
                    div()
                        .occlude()
                        .on_mouse_down_out(move |_, window, cx| outside(window, cx))
                        .child(kit::menu_panel(entries, dismiss)),
                ),
            )
            .with_priority(2)
        });
        let permission = self.permission.as_ref().map(|d| Self::overlay(self.render_permission_dialog(d, cx), window));
        let new_folder = self.new_folder.as_ref().map(|d| Self::overlay(self.render_new_folder_dialog(d, cx), window));

        div()
            .id("file-manager")
            .key_context("FileManager")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(|this, _: &RenameFile, window, cx| this.rename_selected(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleHiddenFiles, window, cx| this.toggle_hidden_files(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleBookmark, window, cx| {
                if this.session.is_some() {
                    this.toggle_bookmark(window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    if !this.focus.contains_focused(window, cx) {
                        window.focus(&this.focus, cx);
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .relative()
                    .child(
                        canvas(
                            {
                                let weak = weak.clone();
                                move |bounds, _, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        // 宽度变了要重绘：栏窄时藏哪些列按这个宽度算（columns_for）
                                        let resized = this.split_bounds.is_none_or(|old| old.size.width != bounds.size.width);
                                        this.split_bounds = Some(bounds);
                                        if resized {
                                            cx.notify();
                                        }
                                    });
                                }
                            },
                            move |_, _, window, _| {
                                if !resizing {
                                    return;
                                }
                                let moved = weak.clone();
                                window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
                                    let _ = moved.update(cx, |this, cx| {
                                        if event.pressed_button == Some(MouseButton::Left) {
                                            this.resize_to(event.position.x, cx);
                                        } else {
                                            this.end_resize(cx);
                                        }
                                    });
                                });
                                let released = weak.clone();
                                window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                                    let _ = released.update(cx, |this, cx| this.end_resize(cx));
                                });
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .min_h_0()
                            .flex_basis(px(0.))
                            .map(|d| {
                                let mut d = d;
                                d.style().flex_grow = Some(self.local_flex);
                                d
                            })
                            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.active_pane = Some(PaneKind::Local)))
                            .child(local),
                    )
                    .child(
                        div()
                            .id("pane-divider")
                            .w(px(DIVIDER_WIDTH))
                            .flex_shrink_0()
                            .rounded_full()
                            .cursor_col_resize()
                            .hover(|s| s.bg(theme::alpha(ui::PRIMARY, 0.15)))
                            .when(resizing, |d| d.bg(theme::alpha(ui::PRIMARY, 0.15)))
                            .tooltip(|window, cx| kit::tooltip(tr!("拖动调整面板宽度").into(), window, cx))
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| this.start_resize(event.position, cx))),
                    )
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .min_h_0()
                            .flex_basis(px(0.))
                            .map(|d| {
                                let mut d = d;
                                d.style().flex_grow = Some(self.remote_flex);
                                d
                            })
                            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.active_pane = Some(PaneKind::Remote)))
                            .child(remote),
                    ),
            )
            .when(self.status.active, |root| root.child(self.render_status(cx)))
            .children(menu)
            .children(permission)
            .children(new_folder)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyBinding, KeyContext, Keymap, Keystroke, Modifiers};

    use super::{FILE_LIST_CONTEXT, ToggleHiddenFiles, key_bindings};

    /// macOS 上按 ⌘⇧. 送来的按键：Shift 并进字符，key 是「>」、只剩 ⌘（gpui-pre-macos 的 parse_keystroke）。
    /// 绑定写成 cmd-shift-. 永远对不上。
    #[test]
    fn cmd_shift_period_toggles_hidden_files() {
        let keymap = Keymap::new(crate::keymap::default_bindings());
        let stack: Vec<KeyContext> = ["Root", "Workspace", "FileManager"].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let typed = Keystroke { modifiers: Modifiers::command(), key: ">".into(), key_char: None };
        let (bindings, _) = keymap.bindings_for_input(&[typed], &stack);
        assert_eq!(bindings.len(), 1);
        assert!(bindings[0].action().partial_eq(&ToggleHiddenFiles));
    }

    /// ↵：文件列表上是改名（同 Finder）；改名框里是确认改名，排在最前、不往外传；地址栏和对话框里的输入框
    /// 还是它自己的回车，不能顺带把选中的文件改名（gpui-base 的单行输入框处理完回车会让它接着往外传）。
    #[test]
    fn enter_renames_only_in_the_file_list() {
        gpui::actions!(test, [InputEnter]);
        let mut keymap = Keymap::new(vec![KeyBinding::new("enter", InputEnter, Some("Input"))]);
        keymap.add_bindings(crate::keymap::default_bindings());
        keymap.add_bindings(key_bindings());
        let enter = |path: &[&str]| -> Vec<String> {
            let stack: Vec<KeyContext> = path.iter().map(|c| KeyContext::parse(c).unwrap()).collect();
            let (bindings, _) = keymap.bindings_for_input(&[Keystroke::parse("enter").unwrap()], &stack);
            bindings.iter().map(|b| b.action().name().to_string()).collect()
        };
        let list = ["Root", "Workspace", "FileManager", FILE_LIST_CONTEXT];
        assert_eq!(enter(&list), ["files::RenameFile"]);
        let renaming = enter(&[&list[..], &["FileRename", "Input"]].concat());
        assert_eq!(renaming.first().map(String::as_str), Some("files::ConfirmRename"), "{renaming:?}");
        for path in [&["Root", "Workspace", "FileManager", "FilePathBar", "Input"][..], &["Root", "Workspace", "FileManager", "Input"]] {
            assert!(!enter(path).iter().any(|a| a == "files::RenameFile"), "{path:?}");
        }
    }
}

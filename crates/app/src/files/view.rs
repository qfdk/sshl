//! 文件管理视图（旧 FileManager.tsx + FilePane.tsx + useFileList.ts + useTransfer.ts）。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Once;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, AppContext, AsyncWindowContext, BoxShadow, Bounds, ClickEvent, Context,
    DragMoveEvent, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathPromptOptions, Pixels, Point, Render, Role, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Transformation, UniformListScrollHandle, WeakEntity, Window, actions, anchored, canvas,
    deferred, div, percentage, point, prelude::FluentBuilder, px, svg, uniform_list,
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
    FileEntry, ListCache, PaneKind, basename, checks_to_permissions, format_date, format_file_size,
    format_permissions, is_root_path, join_path, normalize_remote_path, parent_path, permissions_to_checks,
    readable_permissions, remote_home_from, sort_files, valid_octal,
};
use super::progress::{TransferProgress, TransferStatus, describe_progress};
use super::sudo::{Outcome, Secret, saved_password_choices, sudo_error_message, upload_with_sudo_fallback};
use crate::assets::Icon;
use crate::backend;
use crate::prompts::{self, Choice, Options, PasswordAnswer};
use crate::theme::{self, ui};
use crate::ui::{self as kit, MenuEntry, MenuItem, Variant};

actions!(files, [SelectAllFiles, ClearFileSelection, PathNext, PathPrev, PathComplete, PathDismiss]);

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

const ROW_HEIGHT: f32 = 34.;
const PANE_MIN_WIDTH: f32 = 220.;
const DIVIDER_WIDTH: f32 = 10.;
const MONO_FONT: &str = "Menlo";

fn bind_keys(cx: &mut App) {
    static BOUND: Once = Once::new();
    BOUND.call_once(|| {
        // 晚于 gpui-base 绑定：同一节点上后绑的优先，地址栏里的方向键 / Tab / Esc 先给补全
        cx.bind_keys([
            KeyBinding::new("cmd-a", SelectAllFiles, Some("FileManager")),
            KeyBinding::new("escape", ClearFileSelection, Some("FileManager")),
            KeyBinding::new("down", PathNext, Some("FilePathBar > Input")),
            KeyBinding::new("up", PathPrev, Some("FilePathBar > Input")),
            KeyBinding::new("tab", PathComplete, Some("FilePathBar > Input")),
            KeyBinding::new("escape", PathDismiss, Some("FilePathBar > Input")),
        ]);
    });
}

/// 拖动的那一行（实际拖哪些在开始拖时按选择算）。
#[derive(Clone)]
struct DragFiles {
    pane: PaneKind,
    entry: FileEntry,
}

struct DragState {
    source: PaneKind,
    items: Vec<FileEntry>,
    preview: Option<Entity<DragPreview>>,
}

/// 拖拽中跟着指针的提示：拖了什么、松手会传到哪。
struct DragPreview {
    label: SharedString,
    folder: bool,
    source: PaneKind,
    destination: Option<String>,
    offset: Point<Pixels>,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let verb = if self.source == PaneKind::Local { "上传到" } else { "下载到" };
        div().pl(self.offset.x + px(14.)).pt(self.offset.y + px(14.)).child(
            div()
                .max_w(px(320.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .bg(theme::color(ui::CARD))
                .shadow_lg()
                .text_size(px(12.))
                .text_color(theme::color(ui::FOREGROUND))
                .child(if self.folder {
                    kit::icon_sized(Icon::Folder, 14.).text_color(theme::color(ui::PRIMARY))
                } else {
                    kit::icon_sized(Icon::FileText, 14.).text_color(theme::color(ui::MUTED_FOREGROUND))
                })
                .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(self.label.clone()))
                .when_some(self.destination.clone(), |d, destination| {
                    let name = basename(&destination);
                    let name = if name.is_empty() || destination == "/" { "/".to_string() } else { name };
                    d.child(div().flex_shrink_0().text_color(theme::color(ui::MUTED_FOREGROUND)).child(format!("{verb} {name}")))
                }),
        )
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
    scroll: UniformListScrollHandle,
    /// 上次进入的目录：进入新目录时回到顶部，同目录刷新保留滚动位置。
    listed_path: String,
    selected: HashSet<String>,
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
            scroll: UniformListScrollHandle::new(),
            listed_path: String::new(),
            selected: HashSet::new(),
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

struct PermissionDialog {
    path: String,
    value: String,
    checks: [bool; 9],
    input: Entity<InputState>,
    _subscription: Subscription,
}

struct NewFolderDialog {
    input: Entity<InputState>,
    _subscription: Subscription,
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
    remote_empty: Option<SharedString>,
    remote_empty_connection: Option<String>,
    active: bool,
    syncing: bool,
    transfers: usize,
    status: TransferStatus,
    hide_status: Option<Task<()>>,
    menu: Option<MenuState>,
    /// 当前连接收藏的远程目录（按收藏先后）。
    bookmarks: Vec<String>,
    bookmark_menu: bool,
    permission: Option<PermissionDialog>,
    new_folder: Option<NewFolderDialog>,
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

impl FileManager {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
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
            remote_empty: Some("请先连接到服务器".into()),
            remote_empty_connection: None,
            active: false,
            syncing: false,
            transfers: 0,
            status: TransferStatus::default(),
            hide_status: None,
            menu: None,
            bookmarks: Vec::new(),
            bookmark_menu: false,
            permission: None,
            new_folder: None,
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

    /// 正在进行的传输数（断开确认文案用）。
    pub fn active_transfers(&self) -> usize {
        self.transfers
    }

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
        self.bookmark_menu = false;
        self.reload_bookmarks();
        self.remote.request += 1;
        self.remote.files.clear();
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
                self.remote_empty = Some("请先连接到服务器".into());
                self.set_path(PaneKind::Remote, String::new(), window, cx);
            }
        }
        cx.notify();
    }

    /// 断开 / 失效后的远程空状态；带连接 id 时显示「重新连接」。
    pub fn show_remote_empty(&mut self, message: impl Into<SharedString>, connection_id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.remote_empty_connection = connection_id;
        self.remote_empty = Some(message.into());
        self.remote.request += 1;
        self.remote.files.clear();
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
    }

    pub fn core_event(&mut self, event: &CoreEvent, cx: &mut Context<Self>) {
        let (verb, progress) = match event {
            CoreEvent::UploadProgress(p) => ("上传", TransferProgress::from(p)),
            CoreEvent::DownloadProgress(p) => ("下载", TransferProgress::from(p)),
            CoreEvent::ConnectionsUpdated => return,
        };
        self.status = describe_progress(verb, &progress);
        cx.notify();
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

    // ---------------------------------------------------------------- 加载

    fn load_local_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.local.path.is_empty() {
            self.load(PaneKind::Local, None, false, window, cx);
            return;
        }
        let home = backend::spawn(local_fs::file_get_home_dir());
        cx.spawn_in(window, async move |this, cx| {
            if let Some(Ok(home)) = home.await {
                let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Local, Some(home), false, window, cx));
            }
        })
        .detach();
    }

    fn load_remote_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id() else { return };
        if !self.remote.path.is_empty() {
            self.load(PaneKind::Remote, None, false, window, cx);
            return;
        }
        // 这个会话第一次打开文件管理：从终端所在目录开始，读不到就从远程 home 开始。期间切了会话，这次结果作废。
        self.remote.request += 1;
        let ticket = self.remote.request;
        self.remote.loading = true;
        let start = backend::spawn(async move {
            let state = &backend::get().state;
            let (cwd, home) = futures::join!(sftp::file_terminal_cwd(state, session_id.clone()), sftp::file_remote_home(state, session_id));
            (cwd.ok().flatten(), home.ok())
        });
        cx.spawn_in(window, async move |this, cx| {
            let (cwd, home) = start.await.unwrap_or_default();
            let home = remote_home_from(home.as_deref());
            let _ = this.update_in(cx, |this, window, cx| {
                if this.remote.request != ticket {
                    return;
                }
                // 第一次打开（或刚换了身份）：终端在哪就从哪开始，不管上次跟没跟过
                if let (Some(cwd), Some(record)) = (&cwd, this.session_id().and_then(|sid| this.records.get_mut(&sid))) {
                    record.terminal_cwd = Some(cwd.clone());
                }
                let path = cwd.unwrap_or(if home.is_empty() { "/".to_string() } else { home });
                this.load(PaneKind::Remote, Some(path), false, window, cx);
            });
        })
        .detach();
    }

    /// 终端 cd 到了别处：文件管理跟过去（只在文件管理与终端是同一个用户时，见 sync_identity）。
    fn follow_terminal_cwd(&mut self, session_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let sid = session_id.clone();
        let cwd = backend::spawn(async move { sftp::file_terminal_cwd(&backend::get().state, sid).await });
        cx.spawn_in(window, async move |this, cx| {
            let cwd = cwd.await.and_then(Result::ok).flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                if this.session_id().as_deref() != Some(session_id.as_str()) || this.remote.path.is_empty() {
                    return;
                }
                let Some(path) = this.records.get_mut(&session_id).and_then(|record| record.follow_terminal_cwd(cwd)) else { return };
                if path != this.remote.path {
                    this.load(PaneKind::Remote, Some(path), false, window, cx);
                }
            });
        })
        .detach();
    }

    fn load(&mut self, kind: PaneKind, requested: Option<String>, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut path = requested.filter(|p| !p.is_empty()).unwrap_or_else(|| self.pane(kind).path.clone());
        let session_id = self.session_id();
        if kind == PaneKind::Remote {
            let Some(session_id) = &session_id else {
                self.remote.files.clear();
                self.remote_empty = Some("请先连接到服务器".into());
                self.set_path(PaneKind::Remote, String::new(), window, cx);
                cx.notify();
                return;
            };
            if path.is_empty() {
                path = self.records.get(session_id).map(|r| r.remote_path.clone()).unwrap_or_default();
            }
            path = normalize_remote_path(if path.is_empty() { "/" } else { &path });
            if let Some(record) = self.records.get_mut(session_id) {
                record.remote_path = path.clone();
            }
            self.remote_empty = None;
        } else if path.is_empty() {
            return;
        }
        self.set_path(kind, path.clone(), window, cx);

        let key = self.cache_key(kind, &path);
        let cached = if force {
            None
        } else {
            match kind {
                PaneKind::Local => self.local_cache.get(&key),
                PaneKind::Remote => self.remote_cache.get(&key),
            }
        };
        let pane = self.pane_mut(kind);
        pane.request += 1;
        let request = pane.request;
        if let Some(cached) = &cached {
            pane.files = sort_files(cached.clone());
        }
        pane.loading = cached.is_none();
        pane.error = None;
        self.after_files_changed(kind);
        cx.notify();

        let listing = list_directory(kind, session_id.clone(), path.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = listing.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.pane(kind).request != request {
                    return;
                }
                match result {
                    Ok(files) => {
                        this.pane_mut(kind).loading = false;
                        match kind {
                            PaneKind::Local => this.local_cache.set(key, files.clone()),
                            PaneKind::Remote => this.remote_cache.set(key, files.clone()),
                        }
                        let current = session_id.as_ref().and_then(|id| this.records.get(id)).map(|r| r.remote_path.as_str());
                        if kind == PaneKind::Remote && current != Some(path.as_str()) {
                            return;
                        }
                        let pane = this.pane_mut(kind);
                        pane.files = sort_files(files);
                        pane.loading = false;
                        this.after_files_changed(kind);
                    }
                    Err(error) => {
                        this.pane_mut(kind).error = Some(error);
                        this.pane_mut(kind).loading = false;
                        // 远程目录打不开（没权限、被删了）就退到上一级
                        if kind == PaneKind::Remote && path != "/" {
                            let parent = parent_path(&path, kind);
                            if parent != path {
                                this.load(kind, Some(parent), force, window, cx);
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 文件列表换了：进入新目录时回到顶部、清掉两侧选择（同旧版按 path 清选择）。
    fn after_files_changed(&mut self, kind: PaneKind) {
        let pane = self.pane_mut(kind);
        if pane.listed_path != pane.path {
            pane.listed_path = pane.path.clone();
            pane.scroll.scroll_to_item(0, ScrollStrategy::Top);
            self.local.selected.clear();
            self.remote.selected.clear();
            self.last_selected = None;
        }
    }

    fn refresh(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            PaneKind::Local => self.load(PaneKind::Local, None, true, window, cx),
            PaneKind::Remote => {
                if self.session.is_none() {
                    return;
                }
                let path = if self.remote.path.is_empty() { "/".to_string() } else { self.remote.path.clone() };
                self.load(PaneKind::Remote, Some(path), true, window, cx);
                self.sync_identity(false, window, cx);
            }
        }
    }

    fn go_home(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            PaneKind::Local => {
                let home = backend::spawn(local_fs::file_get_home_dir());
                cx.spawn_in(window, async move |this, cx| {
                    if let Some(Ok(home)) = home.await {
                        let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Local, Some(home), true, window, cx));
                    }
                })
                .detach();
            }
            PaneKind::Remote => {
                let Some(session_id) = self.session_id() else { return };
                let home = backend::spawn(sftp::file_remote_home(&backend::get().state, session_id));
                cx.spawn_in(window, async move |this, cx| {
                    let home = remote_home_from(home.await.and_then(Result::ok).as_deref());
                    if !home.is_empty() {
                        let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Remote, Some(home), true, window, cx));
                    }
                })
                .detach();
            }
        }
    }

    fn browse_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().into_owned();
                let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Local, Some(path), true, window, cx));
            }
        })
        .detach();
    }

    fn open_entry(&mut self, kind: PaneKind, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let path = if entry.is_parent() { parent_path(&self.pane(kind).path, kind) } else { entry.path.clone() };
        self.load(kind, Some(path), true, window, cx);
    }

    // ---------------------------------------------------------------- 地址栏补全

    fn path_event(&mut self, kind: PaneKind, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
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
                self.request_listing(kind, window, cx);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
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
                cx.notify();
            }
        }
    }

    /// 只在目录部分变了（敲了分隔符或 Tab 补全）才取列表；同一层里继续输入只在本地过滤。地址栏补全只读不改当前路径。
    fn request_listing(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
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

    fn open_suggestion(&mut self, kind: PaneKind, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.pane(kind).query() else { return };
        self.pane_mut(kind).suggesting = false;
        self.load(kind, Some(format!("{}{}", query.base, entry.name)), true, window, cx);
    }

    fn path_next(&mut self, kind: PaneKind, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if down && !self.pane(kind).suggesting {
            self.pane_mut(kind).suggesting = true;
            self.request_listing(kind, window, cx);
            cx.notify();
            return;
        }
        let count = self.pane(kind).suggestions().len();
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

    fn path_complete(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
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

    fn path_dismiss(&mut self, kind: PaneKind, cx: &mut Context<Self>) {
        if self.pane(kind).suggestions().is_empty() {
            cx.propagate();
            return;
        }
        self.pane_mut(kind).suggesting = false;
        cx.notify();
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

    /// 全选当前目录的全部条目（含没画出来的行），并清空另一侧，与点选规则一致。
    fn select_all(&mut self, _: &SelectAllFiles, window: &mut Window, cx: &mut Context<Self>) {
        let kind = if self.local.list_focus.is_focused(window) {
            PaneKind::Local
        } else if self.remote.list_focus.is_focused(window) {
            PaneKind::Remote
        } else {
            self.active_pane.unwrap_or(if self.session.is_some() { PaneKind::Remote } else { PaneKind::Local })
        };
        let all = self.pane(kind).files.iter().map(|f| f.path.clone()).collect();
        self.pane_mut(kind).selected = all;
        self.pane_mut(kind.other()).selected.clear();
        cx.notify();
    }

    fn clear_selection(&mut self, _: &ClearFileSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some()
            || std::mem::take(&mut self.bookmark_menu)
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

    fn open_menu(&mut self, kind: PaneKind, target: Option<FileEntry>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = &target {
            self.ensure_context_selection(kind, entry);
        }
        self.menu = Some(MenuState { pane: kind, position, target });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // ---------------------------------------------------------------- 传输

    fn upload_items(&mut self, items: Vec<FileEntry>, directory: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let directory = directory.unwrap_or_else(|| self.remote.path.clone());
        cx.spawn_in(window, async move |this, cx| {
            for item in items {
                let destination = join_path(&directory, &basename(&item.path));
                let op = if item.is_directory { Op::UploadDir } else { Op::UploadFile };
                run_transfer(&this, cx, &session, op, item.path.clone(), destination).await;
            }
        })
        .detach();
    }

    fn download_items(&mut self, items: Vec<FileEntry>, directory: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let directory = directory.unwrap_or_else(|| self.local.path.clone());
        if directory.is_empty() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            for item in items {
                let destination = join_path(&directory, &basename(&item.path));
                let op = if item.is_directory { Op::DownloadDir } else { Op::DownloadFile };
                run_transfer(&this, cx, &session, op, item.path.clone(), destination).await;
            }
        })
        .detach();
    }

    fn upload_paths(&mut self, paths: Vec<String>, directory: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        let kinds = backend::spawn(local_fs::file_local_kinds(paths.clone()));
        cx.spawn_in(window, async move |this, cx| {
            let kinds = kinds.await.unwrap_or_else(|| vec![false; paths.len()]);
            for (index, local) in paths.into_iter().enumerate() {
                let destination = join_path(&directory, &basename(&local));
                let op = if kinds.get(index).copied().unwrap_or(false) { Op::UploadDir } else { Op::UploadFile };
                run_transfer(&this, cx, &session, op, local, destination).await;
            }
        })
        .detach();
    }

    fn select_folder_to_upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let directory = self.remote.path.clone();
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().into_owned();
                let _ = this.update_in(cx, |this, window, cx| this.upload_paths(vec![path], directory, window, cx));
            }
        })
        .detach();
    }

    fn transfer_finished(&mut self, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        let kind = if op.is_upload() { PaneKind::Remote } else { PaneKind::Local };
        if kind == PaneKind::Remote && self.session.is_none() {
            return;
        }
        self.invalidate(kind);
        let path = self.pane(kind).path.clone();
        let path = if kind == PaneKind::Remote && path.is_empty() { "/".to_string() } else { path };
        self.load(kind, Some(path), true, window, cx);
    }

    // ---------------------------------------------------------------- 拖放

    fn begin_drag(&mut self, value: &DragFiles) -> Vec<FileEntry> {
        let pane = self.pane(value.pane);
        let items = drag_items(&value.entry, &pane.selected, &pane.files);
        self.dragging = Some(DragState { source: value.pane, items: items.clone(), preview: None });
        self.hover_pane = None;
        self.hover_row = None;
        items
    }

    fn drag_target(&self) -> Option<DropTarget> {
        let pane = self.hover_pane?;
        let directory = self.hover_row.as_ref().filter(|(p, _)| *p == pane).map(|(_, dir)| dir.clone());
        Some(DropTarget { pane, directory })
    }

    fn update_drag_preview(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = &self.dragging else { return };
        let destination = plan_drop(DragSource::Pane(drag.source), self.drag_target().as_ref(), &self.local.path, &self.remote.path).map(|a| a.directory);
        if let Some(preview) = drag.preview.clone() {
            preview.update(cx, |preview, cx| {
                if preview.destination != destination {
                    preview.destination = destination;
                    cx.notify();
                }
            });
        }
    }

    fn drag_over_pane(&mut self, kind: PaneKind, inside: bool, cx: &mut Context<Self>) {
        let next = if inside { Some(kind) } else if self.hover_pane == Some(kind) { None } else { self.hover_pane };
        if next != self.hover_pane {
            self.hover_pane = next;
            self.update_drag_preview(cx);
        }
    }

    fn drag_over_row(&mut self, kind: PaneKind, directory: &str, inside: bool, cx: &mut Context<Self>) {
        let current = self.hover_row.as_ref().is_some_and(|(p, d)| *p == kind && d == directory);
        if inside && !current {
            self.hover_row = Some((kind, directory.to_string()));
            self.update_drag_preview(cx);
        } else if !inside && current {
            self.hover_row = None;
            self.update_drag_preview(cx);
        }
    }

    fn drop_files(&mut self, target: DropTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.hover_pane = None;
        self.hover_row = None;
        let Some(drag) = self.dragging.take() else { return };
        let Some(action) = plan_drop(DragSource::Pane(drag.source), Some(&target), &self.local.path, &self.remote.path) else { return };
        match action.kind {
            DropKind::Upload => self.upload_items(drag.items, Some(action.directory), window, cx),
            DropKind::Download => self.download_items(drag.items, Some(action.directory), window, cx),
        }
    }

    fn drop_external(&mut self, paths: &ExternalPaths, directory: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let target = DropTarget { pane: PaneKind::Remote, directory };
        let Some(action) = plan_drop(DragSource::Finder, Some(&target), &self.local.path, &self.remote.path) else { return };
        let paths: Vec<String> = paths.paths().iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if !paths.is_empty() {
            self.upload_paths(paths, action.directory, window, cx);
        }
    }

    // ---------------------------------------------------------------- 删除、新建、权限

    fn delete_items(&mut self, kind: PaneKind, items: Vec<FileEntry>, window: &mut Window, cx: &mut Context<Self>) {
        if items.is_empty() {
            return;
        }
        let prompt = delete_prompt(&items.iter().map(|i| (i.path.clone(), i.is_directory)).collect::<Vec<_>>());
        let mut options = Options::new(prompt.title).message(prompt.message).confirm(prompt.confirm_label);
        if let Some(detail) = prompt.detail {
            options = options.detail(detail);
        }
        let confirmed = prompts::confirm(window, cx, options);
        let session_id = self.session_id();
        cx.spawn_in(window, async move |this, cx| {
            if !confirmed.await.unwrap_or(false) {
                return;
            }
            match kind {
                PaneKind::Remote => {
                    let Some(session_id) = session_id else { return };
                    let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
                    for command in remote_delete_commands(&paths) {
                        // 以文件管理当前身份删除（终端切成 root 后就以 root 删）
                        let sid = session_id.clone();
                        let result = backend::spawn(async move { sftp::file_shell(&backend::get().state, sid, command).await }).await;
                        if let Outcome::Failed(error) = Outcome::from_result(result) {
                            alert(cx, "删除失败", error);
                        }
                    }
                }
                PaneKind::Local => {
                    for item in items {
                        let path = item.path.clone();
                        let result = if item.is_directory {
                            backend::spawn(local_fs::file_delete_local_directory(path)).await
                        } else {
                            backend::spawn(local_fs::file_delete_local(path)).await
                        };
                        if let Outcome::Failed(error) = Outcome::from_result(result) {
                            alert(cx, "删除失败", error);
                        }
                    }
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.invalidate(kind);
                this.refresh_list(kind, window, cx);
            });
        })
        .detach();
    }

    /// 只重新列当前目录（不同步身份）。
    fn refresh_list(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.pane(kind).path.clone();
        let path = if kind == PaneKind::Remote && path.is_empty() { "/".to_string() } else { path };
        self.load(kind, Some(path), true, window, cx);
    }

    fn open_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("请输入文件夹名称"));
        let focus = input.read(cx).focus_handle(cx);
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.create_folder(window, cx);
            }
        });
        self.new_folder = Some(NewFolderDialog { input, _subscription: subscription });
        window.focus(&focus, cx);
        cx.notify();
    }

    fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.new_folder.take() else { return };
        let name = dialog.input.read(cx).value().trim().to_string();
        window.focus(&self.focus, cx);
        cx.notify();
        if name.is_empty() {
            return;
        }
        if name.contains('/') || name.contains('\\') {
            prompts::alert_later(window, cx, Options::new("无法新建文件夹").message("文件夹名称不能包含斜杠。"));
            return;
        }
        let Some(session_id) = self.session_id() else { return };
        let full = if self.remote.path == "/" { format!("/{name}") } else { format!("{}/{name}", self.remote.path) };
        let created = backend::spawn(async move { sftp::file_create_remote_directory(&backend::get().state, session_id, full).await });
        cx.spawn_in(window, async move |this, cx| match Outcome::from_result(created.await) {
            Outcome::Failed(error) => alert(cx, "新建文件夹失败", error),
            _ => {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.invalidate(PaneKind::Remote);
                    this.refresh_list(PaneKind::Remote, window, cx);
                });
            }
        })
        .detach();
    }

    fn open_permissions(&mut self, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let value = format_permissions(entry.permissions);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, _, cx| {
            if let (InputEvent::Change, Some(dialog)) = (event, this.permission.as_mut()) {
                dialog.value = input.read(cx).value().trim().chars().take(3).collect();
                dialog.checks = permissions_to_checks(&dialog.value);
                cx.notify();
            }
        });
        self.permission = Some(PermissionDialog { path: entry.path.clone(), checks: permissions_to_checks(&value), value, input, _subscription: subscription });
        cx.notify();
    }

    fn set_permission_value(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.permission.as_mut() else { return };
        dialog.checks = permissions_to_checks(&value);
        dialog.value = value.clone();
        dialog.input.update(cx, |input, cx| input.set_value(value, window, cx));
        cx.notify();
    }

    fn toggle_permission(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.permission.as_mut() else { return };
        let mut checks = dialog.checks;
        checks[index] = !checks[index];
        let value = checks_to_permissions(&checks);
        self.set_permission_value(value, window, cx);
    }

    fn apply_permissions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.permission else { return };
        let Some(session_id) = self.session_id() else { return };
        if !valid_octal(&dialog.value) {
            return;
        }
        let mode = u32::from_str_radix(&dialog.value, 8).unwrap_or(0o755);
        let path = dialog.path.clone();
        let changed = backend::spawn(async move { sftp::file_change_permissions(&backend::get().state, session_id, path, mode).await });
        cx.spawn_in(window, async move |this, cx| match Outcome::from_result(changed.await) {
            Outcome::Failed(error) => alert(cx, "修改权限失败", error),
            _ => {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.permission = None;
                    this.invalidate(PaneKind::Remote);
                    this.refresh_list(PaneKind::Remote, window, cx);
                });
            }
        })
        .detach();
    }

    // ---------------------------------------------------------------- 远程目录收藏

    fn reload_bookmarks(&mut self) {
        self.bookmarks = match &self.session {
            Some(session) => config_store::bookmarks(&session.connection_id).unwrap_or_else(|error| {
                tracing::warn!("load bookmarks: {error}");
                Vec::new()
            }),
            None => Vec::new(),
        };
    }

    fn toggle_bookmark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bookmark_menu = false;
        let path = self.remote.path.clone();
        let Some(session) = self.session.clone().filter(|_| !path.is_empty()) else { return };
        let bookmarked = !self.bookmarks.contains(&path);
        match config_store::set_bookmarked(&session.connection_id, &path, bookmarked) {
            Ok(()) => self.reload_bookmarks(),
            Err(error) => prompts::alert_later(window, cx, Options::new("收藏失败").message(error.to_string())),
        }
        cx.notify();
    }

    fn open_bookmark(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.bookmark_menu = false;
        self.load(PaneKind::Remote, Some(path), false, window, cx);
    }

    /// 路径框旁的收藏按钮：当前目录已收藏时是蓝色；点开是「收藏 / 取消收藏当前目录」和收藏的目录。
    fn bookmark_button(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let marked = !self.remote.path.is_empty() && self.bookmarks.contains(&self.remote.path);
        let button = kit::button("pane-bookmark", Variant::Ghost)
            .w(px(32.))
            .h(px(32.))
            .px_0()
            .aria_label("收藏的目录")
            .tooltip(|window, cx| kit::tooltip("收藏的目录".into(), window, cx))
            .child(kit::icon(Icon::Bookmark).when(marked, |g| g.text_color(theme::color(ui::PRIMARY))))
            .when(disabled, |b| b.opacity(0.5).cursor_default())
            .when(!disabled, |b| {
                b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.bookmark_menu = !this.bookmark_menu;
                    cx.notify();
                }))
            });
        let menu = (self.bookmark_menu && !disabled).then(|| {
            let weak = cx.entity().downgrade();
            let mut entries = Vec::new();
            if !self.remote.path.is_empty() {
                let toggle = weak.clone();
                let label = if marked { "取消收藏当前目录" } else { "收藏当前目录" };
                entries.push(MenuEntry::Item(
                    MenuItem::new(label, move |window, cx| {
                        let _ = toggle.update(cx, |this, cx| this.toggle_bookmark(window, cx));
                    })
                    .icon(Icon::Bookmark),
                ));
            }
            if !self.bookmarks.is_empty() {
                entries.push(MenuEntry::Separator);
            }
            for path in &self.bookmarks {
                let open = weak.clone();
                let target = path.clone();
                entries.push(MenuEntry::Item(
                    MenuItem::new(path.clone(), move |window, cx| {
                        let _ = open.update(cx, |this, cx| this.open_bookmark(target.clone(), window, cx));
                    })
                    .icon(Icon::Folder),
                ));
            }
            let dismiss = move |_: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| {
                    this.bookmark_menu = false;
                    cx.notify();
                });
            };
            let outside = dismiss.clone();
            div().absolute().right_0().top(px(36.)).child(
                deferred(
                    anchored().anchor(gpui::Anchor::TopRight).child(
                        div().occlude().on_mouse_down_out(move |_, window, cx| outside(window, cx)).child(kit::menu_panel(entries, dismiss)),
                    ),
                )
                .with_priority(2),
            )
        });
        div().relative().child(button).when_some(menu, |d, menu| d.child(menu)).into_any_element()
    }

    // ---------------------------------------------------------------- 跟随终端用户

    /// 切到文件管理 / 刷新时看终端此刻是谁，变了就切换身份并去终端所在目录（读不到就去他的主目录）；
    /// 没变就看终端是否 cd 到了别处，跟过去。`interactive`：点徽标。
    fn sync_identity(&mut self, interactive: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        if self.syncing {
            return;
        }
        self.syncing = true;
        let record = self.records.get(&session.session_id).cloned().unwrap_or_default();
        cx.spawn_in(window, async move |this, cx| {
            let sid = session.session_id.clone();
            let names = Rc::new(RefCell::new((String::new(), String::new())));
            let identify_names = names.clone();
            let identify_sid = sid.clone();
            let switch_sid = sid.clone();
            let connection_id = session.connection_id.clone();
            let mut ask_cx = cx.clone();
            let result = sync_file_user(
                async move || {
                    let who = backend::spawn(async move { sftp::file_terminal_user(&backend::get().state, identify_sid).await }).await;
                    let who: TerminalIdentity = who?.ok()?.into();
                    *identify_names.borrow_mut() = (who.login_user.clone(), who.user.clone());
                    Some(who)
                },
                async move |target, secret| {
                    let sid = switch_sid.clone();
                    let (user, via) = target.map(|t| (Some(t.user), Some(t.via.as_str().to_string()))).unwrap_or((None, None));
                    let (password, credential) = split_secret(secret);
                    Outcome::from_result(
                        backend::spawn(async move { sftp::file_switch_user(&backend::get().state, sid, user, via, password, credential).await }).await,
                    )
                },
                async move |retry, exclude, context| {
                    let connection_id = connection_id.clone();
                    let saved = backend::spawn(async move { config_store::cred_list(connection_id) }).await;
                    let choices = saved_password_choices(saved.as_ref(), &context.need_user, &exclude, &context.login_user);
                    let title = if retry { format!("{} 的密码不正确", context.need_user) } else { format!("以 {} 身份管理文件", context.user) };
                    let message = match context.via {
                        Via::Su => format!(
                            "终端已切换到 {user}，文件管理也以 {user} 身份浏览、上传和下载，需要 {user} 的密码。",
                            user = context.user
                        ),
                        Via::Sudo => format!(
                            "终端用 sudo 切换到了 {user}，文件管理也以 {user} 身份浏览、上传和下载，需要 {login} 的 sudo 密码。",
                            user = context.user,
                            login = context.login_user
                        ),
                    };
                    ask_password(&mut ask_cx, Options::new(title).message(message).confirm("切换"), choices).await
                },
                interactive,
                record.declined.as_deref(),
                &record.rejected_saved,
            )
            .await;
            let (login_user, terminal_user) = names.borrow().clone();
            let _ = this.update_in(cx, |this, window, cx| {
                this.syncing = false;
                // 会话在等密码时已被移除：丢弃结果，不写记录、不刷新、不弹报错
                if !record_sync_result(&mut this.records, &sid, &result, &login_user, &terminal_user, record.declined.as_deref()) {
                    return;
                }
                match result.status {
                    // 第一次打开时 remote.path 还空着：load_remote_initial 会去终端目录
                    SyncStatus::Same if this.session_id().as_deref() == Some(sid.as_str()) && !this.remote.path.is_empty() => {
                        this.follow_terminal_cwd(sid.clone(), window, cx);
                    }
                    SyncStatus::Switched => {
                        if this.session_id().as_deref() == Some(sid.as_str()) {
                            this.remote_cache.clear();
                            if let Some(record) = this.records.get_mut(&sid) {
                                record.remote_path.clear();
                            }
                            this.remote.path.clear();
                            this.load_remote_initial(window, cx);
                        }
                    }
                    SyncStatus::Failed => prompts::alert_later(
                        window,
                        cx,
                        Options::new(format!("无法以 {terminal_user} 身份管理文件")).message(sudo_error_message(result.error.as_deref())),
                    ),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
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

    fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resize.take().is_some() {
            let flex = self.local_flex;
            crate::settings::update(cx, |settings| settings.local_pane_flex = Some(flex));
            cx.notify();
        }
    }

    // ---------------------------------------------------------------- 渲染

    fn menu_entries(&self, menu: &MenuState, cx: &mut Context<Self>) -> Vec<MenuEntry> {
        let kind = menu.pane;
        let this = cx.entity().downgrade();
        let on = move |f: fn(&mut FileManager, &mut Window, &mut Context<FileManager>)| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |fm, cx| f(fm, window, cx));
            }
        };
        let this = cx.entity().downgrade();
        let with_items = move |items: Vec<FileEntry>, f: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>)| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |fm, cx| f(fm, items.clone(), window, cx));
            }
        };
        let refresh = MenuItem::new("刷新", {
            let f: fn(&mut FileManager, &mut Window, &mut Context<FileManager>) = match kind {
                PaneKind::Local => |fm, w, cx| fm.refresh(PaneKind::Local, w, cx),
                PaneKind::Remote => |fm, w, cx| fm.refresh(PaneKind::Remote, w, cx),
            };
            on(f)
        })
        .icon(Icon::RefreshCw);
        let connected = self.session.is_some();
        let Some(target) = &menu.target else {
            let first = match kind {
                PaneKind::Remote => MenuItem::new("新建文件夹", on(|fm, w, cx| fm.open_new_folder(w, cx))).icon(Icon::FolderPlus).disabled(!connected),
                PaneKind::Local => MenuItem::new("选择文件夹上传", on(|fm, w, cx| fm.select_folder_to_upload(w, cx))).icon(Icon::Upload).disabled(!connected),
            };
            return vec![MenuEntry::Item(first), MenuEntry::Item(refresh)];
        };
        let items = self.selected_items(kind);
        let selected = if items.iter().any(|i| i.path == target.path) { items } else { vec![target.clone()] };
        let delete: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>) = match kind {
            PaneKind::Local => |fm, items, w, cx| fm.delete_items(PaneKind::Local, items, w, cx),
            PaneKind::Remote => |fm, items, w, cx| fm.delete_items(PaneKind::Remote, items, w, cx),
        };
        let transfer: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>) = match kind {
            PaneKind::Local => |fm, items, w, cx| fm.upload_items(items, None, w, cx),
            PaneKind::Remote => |fm, items, w, cx| fm.download_items(items, None, w, cx),
        };
        if selected.len() > 1 {
            let count = selected.len();
            let (verb, glyph) = if kind == PaneKind::Remote { ("下载", Icon::Download) } else { ("上传", Icon::Upload) };
            return vec![
                MenuEntry::Item(MenuItem::new(format!("{verb} {count} 个项目"), with_items(selected.clone(), transfer)).icon(glyph)),
                MenuEntry::Item(MenuItem::new(format!("删除 {count} 个项目"), with_items(selected, delete)).icon(Icon::Trash)),
                MenuEntry::Item(refresh),
            ];
        }
        let item = selected[0].clone();
        let label = match (kind, item.is_directory) {
            (PaneKind::Remote, true) => ("下载文件夹", Icon::Download),
            (PaneKind::Remote, false) => ("下载文件", Icon::Download),
            (PaneKind::Local, true) => ("上传文件夹", Icon::Upload),
            (PaneKind::Local, false) => ("上传文件", Icon::Upload),
        };
        let delete_label = if item.is_directory { "删除目录" } else { "删除文件" };
        vec![
            MenuEntry::Item(MenuItem::new(label.0, with_items(vec![item.clone()], transfer)).icon(label.1)),
            MenuEntry::Item(refresh),
            MenuEntry::Item(MenuItem::new(delete_label, with_items(vec![item], delete)).icon(Icon::Trash)),
        ]
    }

    fn render_rows(&mut self, kind: PaneKind, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let connected = self.session.is_some();
        let pane = self.pane(kind);
        let parent_offset = pane.parent_offset();
        let current = pane.path.clone();
        let mut out = Vec::with_capacity(range.len());
        for ix in range {
            let Some(entry) = pane.row(ix) else { break };
            let parent = entry.is_parent();
            let selected = !parent && pane.selected.contains(&entry.path);
            let directory = if parent { Some(parent_path(&current, kind)) } else if entry.is_directory { Some(entry.path.clone()) } else { None };
            let file_index = ix.saturating_sub(parent_offset);
            let muted = theme::color(ui::MUTED_FOREGROUND);
            let icon = if parent {
                kit::icon(Icon::ArrowUp).text_color(theme::color(ui::PRIMARY))
            } else if entry.is_directory {
                kit::icon(Icon::Folder).text_color(theme::color(ui::PRIMARY))
            } else {
                kit::icon(Icon::FileText).text_color(muted)
            };
            let click_entry = entry.clone();
            let menu_entry = entry.clone();
            let mut row = div()
                .id(("file-row", ix))
                .role(Role::Row)
                .aria_label(SharedString::from(entry.name.clone()))
                // uniform_list 的行默认按内容收缩，不撑满就和表头的列对不齐
                .w_full()
                .h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .text_size(px(12.))
                .border_b_1()
                .border_color(theme::color(ui::BORDER))
                .cursor_pointer()
                .map(|r| {
                    if selected {
                        r.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND))
                    } else {
                        r.hover(|s| s.bg(theme::alpha(ui::MUTED, 0.5)))
                    }
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.active_pane = Some(kind);
                        let focus = this.pane(kind).list_focus.clone();
                        window.focus(&focus, cx);
                    }),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if !click_entry.is_parent() {
                        this.select_row(kind, &click_entry, file_index, event.modifiers());
                    }
                    if event.click_count() >= 2 && click_entry.is_directory {
                        this.open_entry(kind, &click_entry, window, cx);
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.active_pane = Some(kind);
                        let target = (!menu_entry.is_parent()).then(|| menu_entry.clone());
                        this.open_menu(kind, target, event.position, window, cx);
                    }),
                );
            if let Some(directory) = directory.clone().filter(|_| connected) {
                let drop_dir = directory.clone();
                let move_dir = directory.clone();
                row = row
                    .drag_over::<DragFiles>(move |s, drag, _, _| if drag.pane != kind { s.bg(theme::alpha(ui::PRIMARY, 0.15)) } else { s })
                    .on_drop(cx.listener(move |this, _: &DragFiles, window, cx| {
                        this.drop_files(DropTarget { pane: kind, directory: Some(drop_dir.clone()) }, window, cx);
                    }))
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DragFiles>, _, cx| {
                        this.drag_over_row(kind, &move_dir, event.bounds.contains(&event.event.position), cx);
                    }));
                if kind == PaneKind::Remote {
                    let finder_dir = directory.clone();
                    row = row
                        .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(theme::alpha(ui::PRIMARY, 0.15)))
                        .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.drop_external(paths, Some(finder_dir.clone()), window, cx);
                        }));
                }
            }
            if !parent && connected {
                let weak = cx.entity().downgrade();
                row = row.on_drag(DragFiles { pane: kind, entry: entry.clone() }, move |value, offset, _, cx| {
                    let items = weak.update(cx, |this, _| this.begin_drag(value)).unwrap_or_default();
                    let folder = !(items.len() == 1 && !items[0].is_directory);
                    let preview = cx.new(|_| DragPreview { label: drag_label(&items).into(), folder, source: value.pane, destination: None, offset });
                    let _ = weak.update(cx, |this, _| {
                        if let Some(drag) = &mut this.dragging {
                            drag.preview = Some(preview.clone());
                        }
                    });
                    preview
                });
            }
            let mut name = div()
                .flex_1()
                .min_w_0()
                .px(px(14.))
                .flex()
                .items_center()
                .gap_2()
                .child(icon)
                .child(div().min_w_0().truncate().child(entry.name.clone()));
            if selected {
                name = name.font_weight(FontWeight::MEDIUM);
            }
            let size = if parent { String::new() } else if entry.is_directory { "-".into() } else { format_file_size(entry.size) };
            let date = if parent { String::new() } else { format_date(entry.modified) };
            row = row
                .child(name)
                .child(div().w(px(80.)).px_3().flex().justify_end().text_color(muted).child(size))
                .child(div().w(px(144.)).px_3().text_color(muted).child(date));
            if kind == PaneKind::Remote {
                let owner = if parent { String::new() } else { entry.owner.clone().unwrap_or_else(|| "-".into()) };
                let permissions = if parent { String::new() } else { format_permissions(entry.permissions) };
                let perm_entry = entry.clone();
                row = row.child(div().w(px(96.)).px_3().min_w_0().truncate().text_color(muted).child(owner)).child(
                    div()
                        .id(("perm", ix))
                        .role(Role::Button)
                        .aria_label("修改权限")
                        .w(px(64.))
                        .px_3()
                        .when(!parent, |cell| {
                            cell.tooltip(|window, cx| kit::tooltip("点击修改权限".into(), window, cx)).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_permissions(&perm_entry, window, cx);
                            }))
                        })
                        .child(permissions),
                );
            }
            out.push(row.into_any_element());
        }
        out
    }

    fn pane_button(&self, id: (&'static str, usize), glyph: Icon, title: &'static str, disabled: bool, on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> AnyElement {
        kit::button(id, Variant::Ghost)
            .w(px(32.))
            .h(px(32.))
            .px_0()
            .aria_label(title)
            .tooltip(move |window, cx| kit::tooltip(title.into(), window, cx))
            .child(kit::icon(glyph))
            .when(disabled, |b| b.opacity(0.5).cursor_default())
            .when(!disabled, |b| b.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| on_click(this, window, cx))))
            .into_any_element()
    }

    fn render_badge(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.session.as_ref()?;
        let identity = self.identity()?;
        if identity.elevated() {
            let user = identity.file_user.clone();
            let title: SharedString = format!("文件管理以 {user} 身份运行（跟随终端）").into();
            return Some(
                div()
                    .id("file-user-badge")
                    .h(px(24.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .rounded(px(6.))
                    .bg(theme::alpha(ui::PRIMARY, 0.1))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(ui::PRIMARY))
                    .tooltip(move |window, cx| kit::tooltip(title.clone(), window, cx))
                    .child(kit::icon_sized(Icon::UserRound, 14.))
                    .child(user)
                    .into_any_element(),
            );
        }
        let terminal = identity.terminal_user.clone().filter(|t| *t != identity.file_user)?;
        let title: SharedString = format!("终端当前是 {terminal}，点击让文件管理也切过去").into();
        Some(
            div()
                .id("file-user-switch")
                .role(Role::Button)
                .h(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .rounded(px(6.))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .text_size(px(12.))
                .text_color(theme::color(ui::MUTED_FOREGROUND))
                .cursor_pointer()
                .hover(|s| s.bg(theme::color(ui::MUTED)))
                .tooltip(move |window, cx| kit::tooltip(title.clone(), window, cx))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.sync_identity(true, window, cx)))
                .child(kit::icon_sized(Icon::UserRound, 14.))
                .child(format!("切到 {terminal}"))
                .into_any_element(),
        )
    }

    fn render_pane(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let remote = kind == PaneKind::Remote;
        let disabled = remote && self.session.is_none();
        let badge = if remote { self.render_badge(cx) } else { None };
        let pane = self.pane(kind);
        let suggestions = pane.suggestions();
        let highlight = pane.highlight;
        let input_focused = pane.input_focused;
        let input = pane.input.clone();
        let loading = pane.loading;
        let rows = pane.row_count();
        let list_focus = pane.list_focus.clone();
        let scroll = pane.scroll.clone();
        let empty: Option<AnyElement> = if loading {
            None
        } else if remote && let (Some(connection), Some(message)) = (self.remote_empty_connection.clone(), self.remote_empty.clone()) {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(message)
                    .child(
                        kit::button("reconnect", Variant::Primary)
                            .h(px(32.))
                            .px_3()
                            .text_size(px(12.))
                            .aria_label("重新连接")
                            .child("重新连接")
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(FileManagerEvent::Reconnect(connection.clone())))),
                    )
                    .into_any_element(),
            )
        } else {
            let message = if remote {
                self.remote_empty
                    .clone()
                    .or_else(|| if self.session.is_none() { Some("请先连接到服务器".into()) } else { pane.error.clone().map(Into::into) })
            } else {
                pane.error.clone().map(Into::into)
            };
            message.map(|m| div().child(m).into_any_element()).or_else(|| (rows == 0).then(|| div().child("目录为空").into_any_element()))
        };

        let header = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(14.))
            .pt_3()
            .pb(px(10.))
            .child(div().mr_1().flex_shrink_0().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(if remote { "远程文件" } else { "本地文件" }))
            .children(badge)
            .child(
                div()
                    .id(("path-bar", kind as usize))
                    .key_context("FilePathBar")
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .on_action(cx.listener(move |this, _: &PathNext, window, cx| this.path_next(kind, true, window, cx)))
                    .on_action(cx.listener(move |this, _: &PathPrev, window, cx| this.path_next(kind, false, window, cx)))
                    .on_action(cx.listener(move |this, _: &PathComplete, window, cx| this.path_complete(kind, window, cx)))
                    .on_action(cx.listener(move |this, _: &PathDismiss, _, cx| this.path_dismiss(kind, cx)))
                    .child(
                        div()
                            .h(px(32.))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded(px(9.))
                            .border_1()
                            .border_color(theme::color(if input_focused { ui::RING } else { ui::BORDER }))
                            .bg(theme::color(ui::MUTED))
                            .font_family(MONO_FONT)
                            .text_size(px(12.))
                            .when(disabled, |d| d.opacity(0.5))
                            .child(div().flex_1().min_w_0().child(Input::new(&input))),
                    )
                    .when(!suggestions.is_empty(), |bar| {
                        bar.child(
                            div()
                                .id(("suggestions", kind as usize))
                                .role(Role::ListBox)
                                .aria_label("子目录")
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
                                .children(suggestions.into_iter().enumerate().map(|(index, entry)| {
                                    let open = entry.clone();
                                    div()
                                        .id(("suggestion", index))
                                        .role(Role::ListBoxOption)
                                        .px_2()
                                        .py(px(6.))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .rounded(px(6.))
                                        .font_family(MONO_FONT)
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .when(highlight == Some(index), |d| d.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND)))
                                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                            if *hovered {
                                                this.pane_mut(kind).highlight = Some(index);
                                                cx.notify();
                                            }
                                        }))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_suggestion(kind, &open, window, cx)))
                                        .child(kit::icon_sized(Icon::Folder, 14.).text_color(theme::color(ui::PRIMARY)))
                                        .child(div().min_w_0().truncate().child(entry.name))
                                })),
                        )
                    }),
            )
            .child(if remote {
                self.pane_button(("pane-go", 1), Icon::ChevronRight, "转到路径", disabled, move |this, w, cx| this.load(PaneKind::Remote, None, true, w, cx), cx).into_any_element()
            } else {
                self.pane_button(("pane-browse", 0), Icon::FolderOpen, "选择目录", false, |this, w, cx| this.browse_local(w, cx), cx).into_any_element()
            })
            .when(remote, |d| d.child(self.bookmark_button(disabled, cx)))
            .child(self.pane_button(("pane-home", kind as usize), Icon::House, "主目录", disabled, move |this, w, cx| this.go_home(kind, w, cx), cx))
            .child(self.pane_button(("pane-refresh", kind as usize), Icon::RefreshCw, "刷新", disabled, move |this, w, cx| this.refresh(kind, w, cx), cx));

        let columns = div()
            .h(px(32.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::color(ui::MUTED_FOREGROUND))
            .border_b_1()
            .border_color(theme::color(ui::BORDER))
            .child(div().flex_1().min_w_0().px(px(14.)).child("名称"))
            .child(div().w(px(80.)).px_3().flex().justify_end().child("大小"))
            .child(div().w(px(144.)).px_3().child("修改日期"))
            .when(remote, |c| c.child(div().w(px(96.)).px_3().child("所有者")).child(div().w(px(64.)).px_3().child("权限")));

        let list = div()
            .id(("file-list", kind as usize))
            .track_focus(&list_focus)
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(columns)
            .child(match empty {
                Some(message) => div()
                    .h(px(128.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .text_size(px(12.))
                    .text_color(theme::color(ui::MUTED_FOREGROUND))
                    .child(message)
                    .into_any_element(),
                None => uniform_list(("rows", kind as usize), rows, cx.processor(move |this, range, window, cx| this.render_rows(kind, range, window, cx)))
                    .track_scroll(&scroll)
                    .w_full()
                    .flex_1()
                    .into_any_element(),
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    let focus = this.pane(kind).list_focus.clone();
                    window.focus(&focus, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.active_pane = Some(kind);
                    this.open_menu(kind, None, event.position, window, cx);
                }),
            )
            .when(loading, |l| {
                l.child(div().absolute().inset_0().rounded_b(px(8.)).flex().items_center().justify_center().bg(theme::alpha(ui::CARD, 0.7)).child(spinner(24.)))
            });

        let ring = |s: gpui::StyleRefinement| {
            s.border_color(theme::color(ui::PRIMARY)).shadow(vec![BoxShadow {
                color: theme::alpha(ui::PRIMARY, 0.3),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset: false,
            }])
        };
        let _ = window;
        div()
            .id(("pane", kind as usize))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow(vec![
                BoxShadow { color: theme::alpha(0x16181d, 0.06), offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false },
                BoxShadow { color: theme::alpha(0x16181d, 0.06), offset: point(px(0.), px(8.)), blur_radius: px(24.), spread_radius: px(0.), inset: false },
            ])
            .when(!disabled, |p| {
                p.drag_over::<DragFiles>(move |s, drag, _, _| if drag.pane != kind { ring(s) } else { s })
                    .on_drop(cx.listener(move |this, _: &DragFiles, window, cx| this.drop_files(DropTarget { pane: kind, directory: None }, window, cx)))
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DragFiles>, _, cx| {
                        this.drag_over_pane(kind, event.bounds.contains(&event.event.position), cx);
                    }))
            })
            .when(remote && !disabled, |p| {
                p.drag_over::<ExternalPaths>(move |s, _, _, _| ring(s))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.drop_external(paths, None, window, cx)))
            })
            .child(header)
            .child(list)
            .into_any_element()
    }

    fn render_status(&self) -> AnyElement {
        let progress = self.status.progress.min(100) as f32 / 100.;
        div()
            .h(px(44.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .px(px(14.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .text_size(px(12.))
            .child(
                div()
                    .size(px(26.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .bg(theme::color(ui::ACCENT))
                    .text_color(theme::color(ui::PRIMARY))
                    .child(kit::icon_sized(Icon::ArrowDownUp, 14.)),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.status.info.clone()))
            .child(
                div()
                    .w(px(192.))
                    .h(px(6.))
                    .flex_shrink_0()
                    .rounded_full()
                    .overflow_hidden()
                    .bg(theme::alpha(ui::PRIMARY, 0.2))
                    .child(div().h_full().w(gpui::relative(progress)).bg(theme::color(ui::PRIMARY))),
            )
            .into_any_element()
    }

    fn render_permission_dialog(&self, dialog: &PermissionDialog, cx: &mut Context<Self>) -> AnyElement {
        let valid = valid_octal(&dialog.value);
        let groups = ["所有者", "组", "其他用户"];
        let bits = ["读取", "写入", "执行"];
        let body = div()
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .min_w_0()
                            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("修改文件权限"))
                            .child(div().text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).truncate().child(dialog.path.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child("八进制权限"))
                            .child(div().w(px(80.)).font_family(MONO_FONT).child(kit::text_field(&dialog.input, false)))
                            .child(div().font_family(MONO_FONT).text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(format!("= {}", readable_permissions(&dialog.value)))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(div().mr_2().text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child("常用权限:"))
                            .children(["755", "644", "777", "600"].into_iter().enumerate().map(|(index, value)| {
                                kit::button(("perm-preset", index), Variant::Outline)
                                    .h(px(32.))
                                    .px_3()
                                    .aria_label(value)
                                    .child(value)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.set_permission_value(value.to_string(), window, cx)))
                            })),
                    )
                    .children(groups.iter().enumerate().map(|(group, label)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .pt_3()
                            .border_t_1()
                            .border_color(theme::color(ui::BORDER))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(*label))
                            .child(div().flex().gap_4().children(bits.iter().enumerate().map(|(bit, name)| {
                                let index = group * 3 + bit;
                                div()
                                    .id(("perm-bit", index))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(14.))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.toggle_permission(index, window, cx)))
                                    .child(kit::checkbox(("perm-check", index), dialog.checks[index]).aria_label(format!("{label}{name}")))
                                    .child(*name)
                            })))
                    })),
            )
            .child(
                kit::modal_footer()
                    .child(
                        kit::button("perm-cancel", Variant::Outline).aria_label("取消").child("取消").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.permission = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        kit::button("perm-apply", Variant::Primary)
                            .aria_label("应用")
                            .child("应用")
                            .when(!valid, |b| b.opacity(0.5).cursor_default())
                            .when(valid, |b| b.on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.apply_permissions(window, cx)))),
                    ),
            );
        kit::modal("permission-dialog", 512., body).into_any_element()
    }

    fn render_new_folder_dialog(&self, dialog: &NewFolderDialog, cx: &mut Context<Self>) -> AnyElement {
        let body = div()
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("新建文件夹"))
                    .child(kit::text_field(&dialog.input, true)),
            )
            .child(
                kit::modal_footer()
                    .child(
                        kit::button("folder-cancel", Variant::Outline).aria_label("取消").child("取消").on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.new_folder = None;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        kit::button("folder-create", Variant::Primary)
                            .aria_label("新建")
                            .child("新建")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.create_folder(window, cx))),
                    ),
            );
        kit::modal("new-folder-dialog", 420., body).into_any_element()
    }

    fn overlay(element: impl IntoElement, window: &Window) -> AnyElement {
        let size = window.viewport_size();
        deferred(anchored().position(Point::default()).child(div().relative().w(size.width).h(size.height).child(element))).with_priority(1).into_any_element()
    }
}

impl Render for FileManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                                    let _ = weak.update(cx, |this, _| this.split_bounds = Some(bounds));
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
                            .tooltip(|window, cx| kit::tooltip("拖动调整面板宽度".into(), window, cx))
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
            .when(self.status.active, |root| root.child(self.render_status()))
            .children(menu)
            .children(permission)
            .children(new_folder)
    }
}

/// 一次传输（旧 useTransfer.run）：进度条、完成后刷新目标一侧、失败弹框、3 秒后收起。
#[derive(Clone, Copy)]
enum Op {
    UploadFile,
    UploadDir,
    DownloadFile,
    DownloadDir,
}

impl Op {
    fn verb(self) -> &'static str {
        match self {
            Op::UploadFile => "上传",
            Op::UploadDir => "上传文件夹",
            Op::DownloadFile => "下载",
            Op::DownloadDir => "下载文件夹",
        }
    }

    fn is_upload(self) -> bool {
        matches!(self, Op::UploadFile | Op::UploadDir)
    }
}

async fn run_transfer(this: &WeakEntity<FileManager>, cx: &mut AsyncWindowContext, session: &FileSession, op: Op, source: String, destination: String) -> Outcome {
    let verb = op.verb();
    let name = basename(&source);
    let started = this.update(cx, |this, cx| {
        this.transfers += 1;
        this.hide_status = None;
        this.status = TransferStatus::show(0, format!("正在{verb}: {name}"));
        cx.notify();
    });
    if started.is_err() {
        return Outcome::Cancelled;
    }
    let outcome = match op {
        Op::UploadFile | Op::UploadDir => upload_with_sudo(this, cx, session, source, destination, matches!(op, Op::UploadDir)).await,
        Op::DownloadFile => {
            let sid = session.session_id.clone();
            let b = backend::get();
            Outcome::from_result(backend::spawn(async move { sftp::file_download(&b.events, &b.state, sid, source, destination).await }).await)
        }
        Op::DownloadDir => {
            let sid = session.session_id.clone();
            let b = backend::get();
            Outcome::from_result(backend::spawn(async move { sftp::file_download_directory(&b.events, &b.state, sid, source, destination).await }).await)
        }
    };
    let finished = outcome.clone();
    let _ = this.update_in(cx, |this, window, cx| {
        match &finished {
            Outcome::Ok => {
                this.status = TransferStatus::show(100, format!("{verb}完成"));
                this.transfer_finished(op, window, cx);
            }
            // 用户在 sudo 确认或密码框里取消：不算失败，不弹报错
            Outcome::Cancelled => this.status = TransferStatus::show(0, format!("已取消{verb}")),
            Outcome::Failed(error) => {
                this.status = TransferStatus::show(0, format!("{verb}失败"));
                prompts::alert_later(window, cx, Options::new(format!("{verb}失败")).message(error.clone()));
            }
        }
        this.transfers = this.transfers.saturating_sub(1);
        this.hide_status = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update(cx, |this, cx| {
                this.status.hide();
                cx.notify();
            });
        }));
        cx.notify();
    });
    outcome
}

/// 普通上传；目标目录无写权限时问一次是否用 sudo（见 sudo.rs）。
async fn upload_with_sudo(this: &WeakEntity<FileManager>, cx: &mut AsyncWindowContext, session: &FileSession, local: String, remote: String, is_dir: bool) -> Outcome {
    let sid = session.session_id.clone();
    let elevated = this.read_with(cx, |this, _| this.records.get(&sid).and_then(|r| r.identity.as_ref()).is_some_and(FileIdentity::elevated)).unwrap_or(false);
    let user = Rc::new(RefCell::new(String::new()));
    let parent = {
        let trimmed = remote.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(i) if i > 0 => trimmed[..i].to_string(),
            _ => "/".to_string(),
        }
    };
    let (normal_sid, normal_local, normal_remote) = (sid.clone(), local.clone(), remote.clone());
    let (sudo_sid, sudo_local, sudo_remote) = (sid.clone(), local.clone(), remote.clone());
    let own_sid = sid.clone();
    let own_user = user.clone();
    let ask_user = user.clone();
    let connection_id = session.connection_id.clone();
    let mut confirm_cx = cx.clone();
    let mut ask_cx = cx.clone();
    let outcome = upload_with_sudo_fallback(
        async move || {
            let b = backend::get();
            Outcome::from_result(
                backend::spawn(async move {
                    if is_dir {
                        sftp::file_upload_directory(&b.events, &b.state, normal_sid, normal_local, normal_remote).await
                    } else {
                        sftp::file_upload(&b.events, &b.state, normal_sid, normal_local, normal_remote).await
                    }
                })
                .await,
            )
        },
        async move |secret| {
            let b = backend::get();
            let (password, credential) = split_secret(secret);
            let (sid, local, remote) = (sudo_sid.clone(), sudo_local.clone(), sudo_remote.clone());
            Outcome::from_result(
                backend::spawn(async move { sftp::file_upload_sudo(&b.events, &b.state, sid, local, remote, is_dir, password, credential).await }).await,
            )
        },
        async move || {
            let options = Options::new("没有写入权限")
                .message("当前登录用户不能写入这个目录。要用 sudo 上传吗？文件会归目录的所有者。")
                .detail(parent)
                .confirm("用 sudo 上传");
            let Ok(answer) = confirm_cx.update(|window, cx| prompts::confirm(window, cx, options)) else { return false };
            answer.await.unwrap_or(false)
        },
        async move || {
            let identity = backend::spawn(async move { sftp::file_sudo_identity(&backend::get().state, own_sid).await }).await;
            let identity = identity.and_then(Result::ok)?;
            *own_user.borrow_mut() = identity.user;
            identity.credential
        },
        async move |retry, exclude| {
            let user = ask_user.borrow().clone();
            let connection_id = connection_id.clone();
            let saved = backend::spawn(async move { config_store::cred_list(connection_id) }).await;
            let choices = saved_password_choices(saved.as_ref(), &user, &exclude, &user);
            let title = if retry { "sudo 密码不正确" } else { "需要 sudo 密码" };
            let who = if user.is_empty() { String::new() } else { format!(" {user} ") };
            ask_password(&mut ask_cx, Options::new(title).message(format!("sudo 要的是当前登录用户{who}的密码。")).confirm("上传"), choices).await
        },
        elevated,
    )
    .await;
    match outcome {
        Outcome::Failed(error) if error.starts_with("SUDO_") => Outcome::Failed(sudo_error_message(Some(&error))),
        other => other,
    }
}

fn split_secret(secret: Option<Secret>) -> (Option<String>, Option<String>) {
    match secret {
        Some(Secret::Password(password)) => (Some(password), None),
        Some(Secret::Credential(credential)) => (None, Some(credential)),
        None => (None, None),
    }
}

/// 密码框：确定 → 手输的密码，点选已保存的 → 它的标识（密码不经过界面），取消 → None。
async fn ask_password(cx: &mut AsyncWindowContext, options: Options, choices: Vec<(String, String)>) -> Option<Secret> {
    let placeholder = if choices.is_empty() { "输入密码" } else { "或输入密码" };
    let choices = choices.into_iter().map(|(value, label)| Choice { value, label: label.into() }).collect();
    let answer = cx.update(|window, cx| prompts::password(window, cx, options, placeholder, choices)).ok()?;
    match answer.await.ok().flatten()? {
        PasswordAnswer::Password(password) => Some(Secret::Password(password)),
        PasswordAnswer::Choice(choice) => Some(Secret::Credential(choice)),
    }
}

/// 列目录（不改任何状态）：远程走 SFTP，本地走 std::fs。
fn list_directory(kind: PaneKind, session_id: Option<String>, path: String) -> impl std::future::Future<Output = Result<Vec<FileEntry>, String>> {
    let listing = backend::spawn(async move {
        match kind {
            PaneKind::Remote => {
                let Some(session_id) = session_id else { return Err("请先连接到服务器".to_string()) };
                sftp::file_list(&backend::get().state, session_id, path)
                    .await
                    .map(|files| files.into_iter().map(FileEntry::from).collect())
                    .map_err(|e| e.to_string())
            }
            PaneKind::Local => local_fs::file_list_local(path)
                .await
                .map(|files| files.into_iter().map(FileEntry::from).collect())
                .map_err(|e| e.to_string()),
        }
    });
    async move { listing.await.unwrap_or_else(|| Err("获取文件列表失败".into())) }
}

//! 主窗口：左侧连接列表，右侧会话标签 + 终端 / 文件管理，底部状态栏。
//! 连接、切换、断开、探活、填充密码这些流程都在这里（旧 useConnections.ts + App.tsx）。

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, DragMoveEvent, Entity, ExternalPaths, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyBinding, KeyDownEvent, Modifiers, ModifiersChangedEvent, MouseButton, MouseMoveEvent, ParentElement, Pixels, PlatformInput, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, actions, canvas, div, prelude::FluentBuilder, px,
};
use gpui_base::input::{InputEvent, InputState};
use sshl_core::config_store::{self, AuthKind, SaveConnectionInput, StoredConnection};
use sshl_core::crypto_store;
use sshl_core::events::CoreEvent;
use sshl_core::ssh::{self, ConnectionDetails};

use crate::backend;
use crate::connection_dialog::{ConnectionDialog, ConnectionDialogEvent, ConnectionForm};
use crate::import_dialog::{ImportDialog, ImportDialogEvent};
use crate::files::{FileManager, FileManagerEvent, FileSession};
use crate::system_panel::{SystemPanel, SystemPanelEvent};
use crate::groups::{self, Side};
use crate::prompts::{self, Options, PasswordAnswer};
use crate::quick_connect::{self, QuickConnect, QuickConnectEvent};
use crate::settings;
use crate::settings_dialog::{SettingsDialog, SettingsDialogEvent};
use crate::snippets::{SnippetDialog, SnippetDialogEvent, SnippetPanel, SnippetPanelEvent};
use crate::terminal::{Metrics, Status, TerminalEvent, TerminalView};
use crate::theme::{self, ui};
use crate::ui as kit;

actions!(
    workspace,
    [CloseTab, Minimize, ShowTerminal, ShowFiles, ShowSystem, ShowSnippets, NewConnection, NewLocalTerminal, NextTab, PrevTab, OpenSettings, ToggleSidebar, OpenQuickConnect]
);

/// ⌘1 … ⌘9：切到第 n 个标签（从 1 数，标签栏从左到右，本机标签也算）。
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct SelectTab(pub usize);

/// ⌘1 … ⌘9 依次对应第 1 … 9 个标签（KeyBinding 要 'static 的按键串）。
const TAB_KEYS: [&str; 9] = ["cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-5", "cmd-6", "cmd-7", "cmd-8", "cmd-9"];

pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("cmd-w", CloseTab, None),
        // 视图：终端 / 文件管理 / 系统，顺序同顶栏（系统不常用，放最后）；数字键留给切标签
        KeyBinding::new("cmd-i", ShowTerminal, Some("Workspace")),
        KeyBinding::new("cmd-o", ShowFiles, Some("Workspace")),
        KeyBinding::new("cmd-p", ShowSystem, Some("Workspace")),
        KeyBinding::new("cmd-shift-p", ShowSnippets, Some("Workspace")),
        KeyBinding::new("cmd-n", NewConnection, Some("Workspace")),
        KeyBinding::new("cmd-,", OpenSettings, Some("Workspace")),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
        KeyBinding::new("cmd-k", OpenQuickConnect, Some("Workspace")),
        // 本机终端和标签切换：同 Ghostty / Terminal / 浏览器
        KeyBinding::new("cmd-t", NewLocalTerminal, Some("Workspace")),
        KeyBinding::new("cmd-}", NextTab, Some("Workspace")),
        KeyBinding::new("cmd-{", PrevTab, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Workspace")),
    ];
    bindings.extend(TAB_KEYS.into_iter().zip(1..).map(|(key, n)| KeyBinding::new(key, SelectTab(n), Some("Workspace"))));
    bindings
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(key_bindings());
}

/// 连接遮罩延迟出现：预热池命中时整个连接 <300ms，像本地终端一样直接出终端，不闪一层遮罩。
const LOADING_DELAY: Duration = Duration::from_millis(300);
/// 标签栏延迟多久量一次。
const LATENCY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
/// 按住 ⌘ 多久后浮出快捷键角标：太短则 ⌘C / ⌘V 这类快捷键会让角标一闪。
const HINT_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    Terminal,
    Files,
    /// 进程、端口、负载。
    System,
}

pub struct Session {
    /// 保存的连接 id；本机终端是 `local:<n>`。
    pub connection_id: String,
    /// 会话建立时的名称（连接被删后标签仍用它）。
    pub name: SharedString,
    /// 标签的自定义名字（双击标签 / 右键「重命名」）；None 用默认名字。只跟着这个打开的标签，不存库、重启不留。
    pub custom_title: Option<SharedString>,
    pub view: Entity<TerminalView>,
    /// 本机终端（没有文件管理 / 系统页 / 填充密码）。
    pub local: bool,
    _subscription: Subscription,
}

/// 命令片段里「仅本机」的归属：所有本机标签共用（标签 id 每次都不同）。
const LOCAL_SNIPPET_SCOPE: &str = "local";

struct Pending {
    connection_id: String,
    name: SharedString,
    view: Entity<TerminalView>,
    /// 新建连接：连上后再保存。
    save: Option<SaveConnectionInput>,
    show_overlay: bool,
    _subscription: Subscription,
    _overlay_timer: Task<()>,
}

#[derive(Clone, PartialEq)]
pub enum DropTarget {
    Item { item: String, side: Side, group: String, before: Option<String> },
    Group(String),
    Ungrouped,
}

/// 拖动中的连接（GPUI drag 值）。
#[derive(Clone)]
pub struct DraggedConnection {
    pub id: String,
    pub name: SharedString,
}

/// 拖动中的会话标签。
#[derive(Clone)]
pub struct DraggedTab {
    pub connection_id: String,
    pub name: SharedString,
}

pub struct ConnectionMenu {
    pub connection_id: String,
    pub position: Point<Pixels>,
    /// 在标签上点的右键（菜单是标签那一套），否则是侧栏连接项。
    pub on_tab: bool,
}

pub struct FillMenu {
    pub has_password: bool,
    pub accounts: Vec<String>,
    pub account: Entity<InputState>,
    pub password: Entity<InputState>,
}

pub struct Workspace {
    pub(crate) connections: Vec<StoredConnection>,
    pub(crate) group_order: Vec<String>,
    /// 已打开的会话，按标签顺序（拖动会重排）。
    pub(crate) sessions: Vec<Session>,
    /// 当前会话（按保存的连接 id；一个连接只开一个会话）。
    pub(crate) active: Option<String>,
    pending: Option<Pending>,
    pub(crate) main_tab: MainTab,
    pub(crate) search: Entity<InputState>,
    pub(crate) search_focused: bool,
    terminal_area: Option<Bounds<Pixels>>,
    pub(crate) menu: Option<ConnectionMenu>,
    pub(crate) drop_target: Option<DropTarget>,
    /// 正在拖动的连接 / 标签（连接 id），拖动中半透明。
    pub(crate) dragging: Option<String>,
    pub(crate) tab_drop: Option<(String, Side)>,
    /// 正在原地重命名的标签（`topbar.rs`）。
    pub(crate) renaming: Option<crate::topbar::TabRename>,
    pub(crate) hint_visible: bool,
    hint_task: Option<Task<()>>,
    pub(crate) fill_menu: Option<FillMenu>,
    pub(crate) fill_feedback: bool,
    fill_feedback_task: Option<Task<()>>,
    /// 终端界面右下角显示的传输进度（0–100，说明，是否还在传）。
    transfer_shown: Option<(u32, String, bool)>,
    /// 后台探活中的会话：会话 id → 连接 id。用户在探活期间主动断开会移除它，借此区分「连接失效该重连」和「用户自己断开」。
    revalidating: HashMap<String, String>,
    connection_dialog: Option<Entity<ConnectionDialog>>,
    import_dialog: Option<Entity<ImportDialog>>,
    settings_dialog: Option<Entity<SettingsDialog>>,
    prompts: Entity<prompts::PromptHost>,
    pub(crate) file_manager: Entity<FileManager>,
    system_panel: Entity<SystemPanel>,
    pub(crate) snippet_panel: Entity<SnippetPanel>,
    /// 命令片段面板开着（顶栏按钮下面）。
    pub(crate) snippets_open: bool,
    /// 鼠标在片段按钮上：点按钮关面板时，「点到面板外面」先关了它，按钮的点击别再把它打开。
    pub(crate) snippet_button_hovered: bool,
    snippet_dialog: Option<Entity<SnippetDialog>>,
    quick_connect: Entity<QuickConnect>,
    /// 快速连接（⌘K）开着。
    quick_open: bool,
    pub(crate) focus: FocusHandle,
    /// 当前会话的往返延迟（连接 id，None = 超时）；切会话时清掉，量到再显示。
    pub(crate) latency: Option<(String, Option<std::time::Duration>)>,
    /// 下一个本机标签的编号（标签 id 用，不显示）。
    next_local: u64,
    _latency_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(prompts: Entity<prompts::PromptHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索连接..."));
        let file_manager = cx.new(|cx| FileManager::new(window, cx));
        let system_panel = cx.new(|cx| SystemPanel::new(window, cx));
        let snippet_panel = cx.new(|cx| SnippetPanel::new(window, cx));
        let quick_connect = cx.new(|cx| QuickConnect::new(window, cx));
        let subscriptions = vec![
            // 窗口退到后台（⌘ 点链接后浏览器抢了前台）：之后松开 ⌘ 我们收不到，角标不能一直挂着
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.hint_task = None;
                    if this.hint_visible {
                        this.hint_visible = false;
                        cx.notify();
                    }
                }
            }),
            cx.subscribe_in(&snippet_panel, window, Self::snippet_panel_event),
            cx.subscribe_in(&quick_connect, window, Self::quick_connect_event),
            // 系统页：没权限时的 sudo 命令、看服务日志，都发到终端
            cx.subscribe_in(&system_panel, window, |this, _, event: &SystemPanelEvent, window, cx| match event {
                SystemPanelEvent::RunInTerminal(command) => this.run_in_terminal(command, true, window, cx),
            }),
            cx.subscribe_in(&file_manager, window, |this, _, event: &FileManagerEvent, window, cx| match event {
                FileManagerEvent::Reconnect(connection_id) => this.connect_to_saved(connection_id, window, cx),
            }),
            cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::Focus => {
                    this.search_focused = true;
                    cx.notify();
                }
                InputEvent::Blur => {
                    this.search_focused = false;
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe(&prompts, |_, _, cx| cx.notify()),
            // 在终端界面时也显示传输进度（从终端拖文件上传）：只在进度真有变化时重画
            cx.observe(&file_manager, |this: &mut Self, files, cx| {
                let files = files.read(cx);
                let shown = files.transfer_status().map(|s| (s.progress, s.info.clone(), files.is_transferring()));
                if shown != this.transfer_shown {
                    this.transfer_shown = shown;
                    if this.main_tab == MainTab::Terminal {
                        cx.notify();
                    }
                }
            }),
        ];
        let mut this = Self {
            connections: Vec::new(),
            group_order: Vec::new(),
            sessions: Vec::new(),
            active: None,
            pending: None,
            main_tab: MainTab::Terminal,
            search,
            search_focused: false,
            terminal_area: None,
            menu: None,
            drop_target: None,
            dragging: None,
            tab_drop: None,
            renaming: None,
            hint_visible: false,
            hint_task: None,
            fill_menu: None,
            fill_feedback: false,
            fill_feedback_task: None,
            transfer_shown: None,
            revalidating: HashMap::new(),
            connection_dialog: None,
            import_dialog: None,
            settings_dialog: None,
            prompts,
            file_manager,
            system_panel,
            snippet_panel,
            snippets_open: false,
            snippet_button_hovered: false,
            snippet_dialog: None,
            quick_connect,
            quick_open: false,
            focus: cx.focus_handle(),
            latency: None,
            next_local: 1,
            _latency_task: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(LATENCY_INTERVAL).await;
                    if this.update(cx, |this, cx| this.measure_latency(cx)).is_err() {
                        break;
                    }
                }
            }),
            _subscriptions: subscriptions,
        };
        this.reload_connections();
        // 启动时就有焦点：否则 ⌘N、⌘, 等工作区快捷键要先随便点一下才生效
        window.focus(&this.focus, cx);
        this
    }

    pub(crate) fn reload_connections(&mut self) {
        match config_store::connections() {
            Ok(connections) => self.connections = connections,
            Err(error) => tracing::warn!("load connections: {error}"),
        }
        match config_store::groups() {
            Ok(groups) => self.group_order = groups,
            Err(error) => tracing::warn!("load groups: {error}"),
        }
    }

    pub fn core_event(&mut self, event: CoreEvent, cx: &mut Context<Self>) {
        self.file_manager.update(cx, |files, cx| files.core_event(&event, cx));
        if let CoreEvent::ConnectionsUpdated = event {
            self.reload_connections();
            cx.notify();
        }
    }

    pub(crate) fn session(&self, connection_id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.connection_id == connection_id)
    }

    pub(crate) fn active_session(&self) -> Option<&Session> {
        self.active.as_deref().and_then(|id| self.session(id))
    }

    pub(crate) fn connection(&self, id: &str) -> Option<&StoredConnection> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// 标签的默认名字：保存的连接名（连接被删了用会话建立时的名字；本机标签是「本机」「本机 2」…）。
    pub(crate) fn default_tab_name(&self, session: &Session) -> SharedString {
        self.connection(&session.connection_id).map(|c| SharedString::from(c.name.clone())).unwrap_or_else(|| session.name.clone())
    }

    /// 标签上显示的名字：改过名的用自定义名，否则默认名字。确认框里说「断开 / 关闭『某某』」也用它。
    pub(crate) fn tab_name(&self, session: &Session) -> SharedString {
        let default = self.default_tab_name(session);
        groups::tab_title(session.custom_title.as_deref(), &default).to_string().into()
    }

    // ---------- 连接 ----------

    /// 侧栏双击 / 标签点击 / 右键「连接」：已开着就切过去（后台探活，失效自动重连），否则新连。
    pub(crate) fn connect_to_saved(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        if let Some(session) = self.session(id) {
            if session.view.read(cx).is_disconnected() {
                self.reconnect_in_place(id, window, cx);
            } else {
                self.switch_to(id, window, cx);
            }
            return;
        }
        self.prepare_saved(id, window, cx, |this, conn, details, window, cx| {
            if this.pending.is_some() || this.session(&conn.id).is_some() {
                return;
            }
            this.start_connect(conn.id.clone(), conn.name.clone().into(), details, None, window, cx);
        });
    }

    /// 断开的标签里原地重连：屏幕内容留着，新连接接在下面。
    fn reconnect_in_place(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(id) {
            self.activate(id, window, cx);
        }
        self.prepare_saved(id, window, cx, |this, conn, details, window, cx| {
            if let Some(view) = this.session(&conn.id).map(|s| s.view.clone()) {
                view.update(cx, |view, cx| view.reconnect(details, window, cx));
            }
        });
    }

    /// 标签右键「重新连接」：还连着的先在后端断开旧会话（终端的 reconnect 只换 shell 通道，不关旧连接），
    /// 再原地重连，屏幕内容和回滚历史留着。
    pub(crate) fn reconnect_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // 连接被删了就没有参数可重连，别先把旧会话断掉
        if self.connection(id).is_none() {
            return;
        }
        let old = self.session(id).and_then(|s| s.view.read(cx).session_id().map(str::to_string));
        if let Some(old) = old {
            self.revalidating.remove(&old);
            self.file_manager.update(cx, |files, _| files.forget_session(&old));
            backend::get().runtime.spawn(async move { ssh::disconnect(&backend::get().state, &old).await });
        }
        self.reconnect_in_place(id, window, cx);
    }

    /// ⌘T：开一个本机终端标签。当前是本机标签时从它的目录开始（同 Ghostty）。
    pub(crate) fn new_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 对话框 / 提示框开着时 ⌘T 不在背后开标签
        if self.modal_open(cx) {
            return;
        }
        let cwd = self.active_session().filter(|s| s.local).and_then(|s| s.view.read(cx).local_cwd());
        let metrics = Metrics::measure(window, cx);
        let area = self.terminal_area.map(|b| b.size).unwrap_or_else(|| window.viewport_size());
        let view = cx.new(|cx| TerminalView::local(cwd, metrics.grid(area), window, cx));
        if let Status::Failed(error) = view.read(cx).status.clone() {
            prompts::alert_later(window, cx, Options::new("打开本机终端失败").message(error));
            return;
        }
        let id = format!("local:{}", self.next_local);
        self.next_local += 1;
        let names: Vec<String> = self.sessions.iter().filter(|s| s.local).map(|s| s.name.to_string()).collect();
        let subscription = cx.subscribe_in(&view, window, Self::terminal_event);
        self.sessions.push(Session {
            connection_id: id.clone(),
            name: groups::local_tab_name(&names).into(),
            custom_title: None,
            view,
            local: true,
            _subscription: subscription,
        });
        self.activate(&id, window, cx);
    }

    /// 切到一个已打开的标签：点标签、⌘1 … ⌘9、⌘⇧] / ⌘⇧[ 都走这里。
    /// 走 connect_to_saved 而不是直接切：会话已失效时它会接着重连，和侧栏双击一致；
    /// 已断开的标签只切过去看（终端里提示按 Enter 重连），不自动重连。
    pub(crate) fn select_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.as_deref() == Some(id) {
            return;
        }
        let Some(session) = self.session(id) else { return };
        if session.view.read(cx).is_disconnected() {
            self.activate(id, window, cx);
        } else {
            self.connect_to_saved(id, window, cx);
        }
    }

    /// ⌘⇧] / ⌘⇧[：切到右 / 左边的标签（两头绕回）。
    fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dismiss_panels_for_shortcut(cx) {
            return;
        }
        let ids: Vec<String> = self.sessions.iter().map(|s| s.connection_id.clone()).collect();
        if let Some(next) = groups::cycle_tab(&ids, self.active.as_ref(), step) {
            self.select_tab(&next, window, cx);
        }
        // 已经在那个标签上（select_tab 不动）时，收掉面板后焦点也回内容区
        self.refocus(window, cx);
        cx.notify();
    }

    /// ⌘1 … ⌘9：切到第 n 个标签；没有那么多标签时什么都不做。
    fn select_nth_tab(&mut self, action: &SelectTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dismiss_panels_for_shortcut(cx) {
            return;
        }
        let ids: Vec<String> = self.sessions.iter().map(|s| s.connection_id.clone()).collect();
        if let Some(id) = groups::nth_tab(&ids, action.0) {
            self.select_tab(&id, window, cx);
        }
        self.refocus(window, cx);
        cx.notify();
    }

    /// 标签右键「关闭其他标签」：一次确认，断开除它以外的所有会话，停在它上面。
    pub(crate) fn close_other_tabs(&mut self, keep: &str, window: &mut Window, cx: &mut Context<Self>) {
        let others: Vec<String> = self.sessions.iter().map(|s| s.connection_id.clone()).filter(|id| id != keep).collect();
        if others.is_empty() {
            return;
        }
        let message = format!("确定要关闭其他 {} 个标签吗？连着的会话会断开。", others.len());
        let answer = prompts::confirm(window, cx, Options::new("关闭其他标签").message(message).confirm("关闭"));
        let keep = keep.to_string();
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await.unwrap_or(false) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                for id in &others {
                    this.disconnect_now(id, window, cx);
                }
                this.switch_to(&keep, window, cx);
            });
        })
        .detach();
    }

    /// 已保存连接的连接参数，交给 `go`。密码方式但没存密码时先问（只用这一次，不保存）。
    fn prepare_saved(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        go: impl FnOnce(&mut Self, StoredConnection, ConnectionDetails, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(conn) = self.connection(id).cloned() else { return };
        let mut details = ConnectionDetails {
            id: Some(conn.id.clone()),
            host: conn.host.clone(),
            port: conn.port,
            username: conn.username.clone(),
            password: None,
            private_key: conn.private_key.clone(),
            passphrase: None,
            auth: conn.auth,
            cols: 0,
            rows: 0,
        };
        if conn.auth == AuthKind::Password && !crypto_store::has(&conn.id, "password") {
            let target = format!("{}@{}", conn.username, conn.host);
            let options = Options::new(format!("连接 {}", conn.name)).message(format!("输入 {target} 的密码。")).confirm("连接");
            let answer = prompts::password(window, cx, options, "密码", Vec::new());
            cx.spawn_in(window, async move |this, cx| {
                let Ok(Some(PasswordAnswer::Password(password))) = answer.await else { return };
                let _ = this.update_in(cx, |this, window, cx| {
                    details.password = Some(password);
                    go(this, conn, details, window, cx);
                });
            })
            .detach();
            return;
        }
        go(self, conn, details, window, cx);
    }

    fn start_connect(
        &mut self,
        connection_id: String,
        name: SharedString,
        details: ConnectionDetails,
        save: Option<SaveConnectionInput>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let metrics = Metrics::measure(window, cx);
        let area = self.terminal_area.map(|b| b.size).unwrap_or_else(|| window.viewport_size());
        let grid = metrics.grid(area);
        let view = cx.new(|cx| TerminalView::connect(details, grid, window, cx));
        let subscription = cx.subscribe_in(&view, window, Self::terminal_event);
        let overlay_timer = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(LOADING_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(pending) = &mut this.pending {
                    pending.show_overlay = true;
                    cx.notify();
                }
            });
        });
        self.pending = Some(Pending {
            connection_id,
            name,
            view,
            save,
            show_overlay: false,
            _subscription: subscription,
            _overlay_timer: overlay_timer,
        });
        cx.notify();
    }

    /// 取消正在进行的连接（遮罩上的「取消」、Esc）。
    pub(crate) fn cancel_connect(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else { return };
        let attempt = pending.view.read(cx).attempt_id.clone();
        pending.view.read(cx).close();
        backend::get().runtime.spawn(async move {
            ssh::cancel_connect(&backend::get().state, &attempt).await;
        });
        cx.notify();
    }

    fn terminal_event(&mut self, view: &Entity<TerminalView>, event: &TerminalEvent, window: &mut Window, cx: &mut Context<Self>) {
        let is_pending = self.pending.as_ref().is_some_and(|p| &p.view == view);
        match event {
            TerminalEvent::Connected if is_pending => {
                let pending = self.pending.take().expect("pending");
                if let Some(save) = pending.save {
                    if let Err(error) = config_store::save_connection(&backend::get().events, save) {
                        prompts::alert_later(window, cx, Options::new("保存连接失败").message(error.to_string()));
                    }
                    self.reload_connections();
                }
                let subscription = cx.subscribe_in(&pending.view, window, Self::terminal_event);
                self.sessions.push(Session {
                    connection_id: pending.connection_id.clone(),
                    name: pending.name,
                    custom_title: None,
                    view: pending.view,
                    local: false,
                    _subscription: subscription,
                });
                self.activate(&pending.connection_id, window, cx);
            }
            TerminalEvent::Failed(error) if is_pending => {
                self.pending = None;
                prompts::alert_later(window, cx, Options::new("连接失败").message(error.clone()));
                cx.notify();
            }
            TerminalEvent::Declined if is_pending => {
                self.pending = None;
                cx.notify();
            }
            // 本机 shell 退出（exit、^D）：标签直接关掉，同 Ghostty / Terminal
            TerminalEvent::Closed if self.sessions.iter().any(|s| &s.view == view && s.local) => {
                if let Some(connection_id) = self.sessions.iter().find(|s| &s.view == view).map(|s| s.connection_id.clone()) {
                    self.disconnect_now(&connection_id, window, cx);
                }
            }
            // 远端关了（exit、掉线）：标签和屏幕留着，终端里提示按回车重连；文件管理显示已断开
            TerminalEvent::Closed => {
                if let Some(connection_id) = self.sessions.iter().find(|s| &s.view == view).map(|s| s.connection_id.clone()) {
                    if self.active.as_deref() == Some(connection_id.as_str()) {
                        self.latency = None;
                        self.fill_menu = None;
                        self.file_manager.update(cx, |files, cx| {
                            files.set_session(None, window, cx);
                            files.show_remote_empty("连接已断开，请重新连接到服务器", Some(connection_id), window, cx);
                        });
                        self.system_panel.update(cx, |panel, cx| panel.set_session(None, cx));
                    }
                    cx.notify();
                }
            }
            TerminalEvent::ReconnectRequested => {
                if let Some(connection_id) = self.sessions.iter().find(|s| &s.view == view).map(|s| s.connection_id.clone()) {
                    self.reconnect_in_place(&connection_id, window, cx);
                }
            }
            // 原地重连成功：当前标签的话，文件管理和延迟换到新会话
            TerminalEvent::Connected => {
                if let Some(connection_id) = self.sessions.iter().find(|s| &s.view == view).map(|s| s.connection_id.clone())
                    && self.active.as_deref() == Some(connection_id.as_str())
                {
                    self.activate(&connection_id, window, cx);
                }
                cx.notify();
            }
            TerminalEvent::PasswordPrompt => cx.notify(),
            TerminalEvent::FilesDropped(paths) if self.active_session().is_some_and(|s| &s.view == view) => {
                let paths = paths.clone();
                self.file_manager.update(cx, |files, cx| files.upload_to_terminal_cwd(paths, window, cx));
            }
            _ => {}
        }
    }

    /// 量一次当前会话的往返延迟（标签栏右侧显示）。
    fn measure_latency(&mut self, cx: &mut Context<Self>) {
        let Some((connection_id, session_id)) =
            self.active_session().and_then(|s| Some((s.connection_id.clone(), s.view.read(cx).session_id()?.to_string())))
        else {
            return;
        };
        let rtt = backend::spawn(async move { ssh::latency(&backend::get().state, &session_id).await });
        cx.spawn(async move |this, cx| {
            let rtt = rtt.await.flatten();
            let _ = this.update(cx, |this, cx| {
                if this.active.as_deref() == Some(connection_id.as_str()) {
                    this.latency = Some((connection_id, rtt));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn activate(&mut self, connection_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(connection_id) {
            self.latency = None;
        }
        self.active = Some(connection_id.to_string());
        self.measure_latency(cx);
        self.fill_menu = None;
        self.snippets_open = false;
        let file_session = self.session(connection_id).and_then(|s| {
            let session_id = s.view.read(cx).session_id()?.to_string();
            Some(FileSession { session_id, connection_id: s.connection_id.clone(), name: s.name.clone() })
        });
        let disconnected = self.session(connection_id).is_some_and(|s| s.view.read(cx).is_disconnected());
        // 本机标签只有终端：停在文件管理 / 系统页时切回终端
        if self.session(connection_id).is_some_and(|s| s.local) && self.main_tab != MainTab::Terminal {
            self.main_tab = MainTab::Terminal;
            self.file_manager.update(cx, |files, cx| files.set_active(false, window, cx));
            self.system_panel.update(cx, |panel, cx| panel.set_active(false, window, cx));
        }
        let id = connection_id.to_string();
        let system_session = file_session.as_ref().map(|s| s.session_id.clone());
        self.system_panel.update(cx, |panel, cx| panel.set_session(system_session, cx));
        self.file_manager.update(cx, |files, cx| {
            files.set_session(file_session, window, cx);
            if disconnected {
                files.show_remote_empty("连接已断开，请重新连接到服务器", Some(id), window, cx);
            }
        });
        if self.main_tab == MainTab::Terminal
            && let Some(session) = self.session(connection_id) {
                let focus = session.view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
        cx.notify();
    }

    /// 切到已开的会话：乐观切换，探活放到后台（不挡在每次切换前面），失效且用户仍停在该会话上时自动重连。
    fn switch_to(&mut self, connection_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.session(connection_id).and_then(|s| s.view.read(cx).session_id().map(str::to_string)) else {
            // 断开的标签：切过去看（终端里有「按 Enter 重新连接」），不自动重连
            if self.session(connection_id).is_some() && self.active.as_deref() != Some(connection_id) {
                self.activate(connection_id, window, cx);
            }
            return;
        };
        if self.active.as_deref() != Some(connection_id) {
            self.activate(connection_id, window, cx);
        }
        if self.revalidating.contains_key(&session_id) {
            return;
        }
        self.revalidating.insert(session_id.clone(), connection_id.to_string());
        let probe_id = session_id.clone();
        let probe = backend::spawn(async move { ssh::validate(&backend::get().state, &probe_id).await });
        cx.spawn_in(window, async move |this, cx| {
            let alive = probe.await.unwrap_or(false);
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(connection_id) = this.revalidating.remove(&session_id) else { return };
                if alive {
                    return;
                }
                // 后端探活失败时已关闭该会话；用户还停在这个标签上就原地重连（屏幕内容留着）
                if this.active.as_deref() == Some(connection_id.as_str()) && this.session(&connection_id).is_some() {
                    this.reconnect_in_place(&connection_id, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 所有断开入口（标签 ×、侧栏、右键菜单、顶栏按钮、⌘W）都走这里，统一先确认。
    pub(crate) fn disconnect(&mut self, connection_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session(connection_id) else { return };
        // 本机标签：前台只有 shell 时直接关；有程序在跑（vim、ssh、dev server）才问
        if session.local {
            let connection_id = connection_id.to_string();
            let Some(program) = session.view.read(cx).local_busy() else {
                return self.disconnect_now(&connection_id, window, cx);
            };
            let message = format!("「{program}」还在运行，关闭标签会结束它。");
            let answer = prompts::confirm(window, cx, Options::new(format!("关闭「{}」", self.tab_name(session))).message(message).confirm("关闭"));
            cx.spawn_in(window, async move |this, cx| {
                if answer.await.unwrap_or(false) {
                    let _ = this.update_in(cx, |this, window, cx| this.disconnect_now(&connection_id, window, cx));
                }
            })
            .detach();
            return;
        }
        // 已经断开的标签：关掉就是了，不用确认
        if session.view.read(cx).is_disconnected() {
            let connection_id = connection_id.to_string();
            return self.disconnect_now(&connection_id, window, cx);
        }
        let name = self.tab_name(session);
        let name = if name.is_empty() { SharedString::from("这个会话") } else { name };
        let on_files = self.main_tab == MainTab::Files && self.active.as_deref() == Some(connection_id);
        let transferring = self.file_manager.read(cx).active_transfers() > 0;
        let consequence = if transferring {
            "正在进行的文件传输会中断。"
        } else if on_files {
            "文件管理会一并关闭。"
        } else {
            ""
        };
        let message = [format!("确定要断开「{name}」吗？"), consequence.to_string()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let answer = prompts::confirm(window, cx, Options::new("断开连接").message(message).confirm("断开"));
        let connection_id = connection_id.to_string();
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await.unwrap_or(false) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.disconnect_now(&connection_id, window, cx));
        })
        .detach();
    }

    fn disconnect_now(&mut self, connection_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.sessions.iter().position(|s| s.connection_id == connection_id) else { return };
        // 正在重命名的标签被关掉：输入框跟着收掉（不改名）。焦点在它里面的话，最后交给内容区，
        // 免得停在没了的输入框上（切到文件管理页的标签时 activate 不挪焦点）
        let rename_focused = self
            .renaming
            .take_if(|r| r.connection_id == connection_id)
            .is_some_and(|r| r.is_focused(window, cx));
        let session = self.sessions.remove(index);
        // 正在重连时关掉：连接请求一起取消，免得连上一个没人管的会话
        if !session.local {
            let attempt = session.view.read(cx).attempt_id.clone();
            backend::get().runtime.spawn(async move { ssh::cancel_connect(&backend::get().state, &attempt).await });
        }
        if let Some(session_id) = session.view.read(cx).session_id().map(str::to_string) {
            // 用户主动断开：探活结果回来时不能把它当成「连接失效」去自动重连
            self.revalidating.remove(&session_id);
            backend::get().runtime.spawn(async move { ssh::disconnect(&backend::get().state, &session_id).await });
        }
        session.view.read(cx).close();
        let session_id = session.view.read(cx).session_id().map(str::to_string);
        let was_active = self.active.as_deref() == Some(connection_id);
        // 关掉的是当前标签、还剩别的标签：切到相邻的那个，不要停在「点击左侧连接」的空页面上（标签明明还在）
        let remaining: Vec<String> = self.sessions.iter().map(|s| s.connection_id.clone()).collect();
        let next = if was_active { groups::tab_after_close(&remaining, index) } else { None };
        self.session_gone(session_id.as_deref(), connection_id, was_active && next.is_none(), "连接已断开，请重新连接到服务器", window, cx);
        if let Some(next) = next {
            self.switch_to(&next, window, cx);
        }
        if rename_focused {
            self.refocus(window, cx);
        }
        cx.notify();
    }

    /// 会话没了（主动断开、远端关闭、失效丢弃）：清掉文件管理里它的记录；是当前会话就回到终端占位、文件管理显示空状态。
    fn session_gone(&mut self, session_id: Option<&str>, connection_id: &str, was_active: bool, message: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let connection_id = connection_id.to_string();
        self.file_manager.update(cx, |files, cx| {
            if let Some(session_id) = session_id {
                files.forget_session(session_id);
            }
            if was_active {
                files.set_session(None, window, cx);
                files.show_remote_empty(message, Some(connection_id), window, cx);
            }
        });
        if was_active {
            self.active = None;
            if self.main_tab != MainTab::Terminal {
                self.main_tab = MainTab::Terminal;
                self.file_manager.update(cx, |files, cx| files.set_active(false, window, cx));
                self.system_panel.update(cx, |panel, cx| panel.set_active(false, window, cx));
            }
            self.system_panel.update(cx, |panel, cx| panel.set_session(None, cx));
            self.fill_menu = None;
            self.snippets_open = false;
            window.focus(&self.focus, cx);
        }
    }

    pub(crate) fn delete_connection(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let answer = prompts::confirm(window, cx, Options::new("删除连接").message("确定要删除这个连接吗？").confirm("删除"));
        let id = id.to_string();
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await.unwrap_or(false) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = config_store::delete_connection(&backend::get().events, id) {
                    tracing::warn!("delete connection: {error}");
                }
                this.reload_connections();
                cx.notify();
            });
        })
        .detach();
    }

    /// 侧栏拖动松手：移到目标分组 / 位置，立即写库。
    pub(crate) fn move_connection(&mut self, id: &str, target: &DropTarget, cx: &mut Context<Self>) {
        let (group, before) = match target {
            DropTarget::Ungrouped => (String::new(), None),
            DropTarget::Group(group) => (group.clone(), None),
            DropTarget::Item { group, before, .. } => (group.clone(), before.clone()),
        };
        self.connections = groups::move_connection(&self.connections, &self.group_order, id, &group, before.as_deref());
        let layout = self
            .connections
            .iter()
            .map(|c| config_store::ConnectionLayoutInput { id: c.id.clone(), group: c.group.trim().to_string() })
            .collect();
        if let Err(error) = config_store::apply_connection_layout(&backend::get().events, layout, None) {
            tracing::warn!("save layout: {error}");
        }
        cx.notify();
    }

    pub(crate) fn move_tab(&mut self, dragged: &str, target: &str, side: Side, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.sessions.iter().map(|s| s.connection_id.clone()).collect();
        let order = groups::move_tab(&ids, &dragged.to_string(), &target.to_string(), side);
        self.sessions.sort_by_key(|s| order.iter().position(|id| *id == s.connection_id));
        cx.notify();
    }

    // ---------- 对话框 ----------

    pub(crate) fn open_connection_dialog(&mut self, editing: Option<StoredConnection>, window: &mut Window, cx: &mut Context<Self>) {
        let groups = groups::group_names(&self.connections, &self.group_order);
        let dialog = cx.new(|cx| ConnectionDialog::new(editing, groups, window, cx));
        self._subscriptions.push(cx.subscribe_in(&dialog, window, Self::connection_dialog_event));
        self.connection_dialog = Some(dialog);
        cx.notify();
    }

    fn connection_dialog_event(&mut self, _: &Entity<ConnectionDialog>, event: &ConnectionDialogEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ConnectionDialogEvent::Cancel => self.close_connection_dialog(window, cx),
            ConnectionDialogEvent::Import => {
                self.connection_dialog = None;
                let saved = self.connections.clone();
                let dialog = cx.new(|cx| ImportDialog::new(saved, window, cx));
                self._subscriptions.push(cx.subscribe_in(&dialog, window, |this, _, event: &ImportDialogEvent, window, cx| {
                    if let ImportDialogEvent::Imported(count) = event {
                        this.reload_connections();
                        tracing::info!("imported {count} hosts from ~/.ssh/config");
                    }
                    this.import_dialog = None;
                    this.refocus(window, cx);
                    cx.notify();
                }));
                self.import_dialog = Some(dialog);
                cx.notify();
            }
            ConnectionDialogEvent::Connect(form) => {
                if self.pending.is_some() {
                    return;
                }
                let id = new_connection_id();
                let save = form.to_save_input(Some(id.clone()), form.save_password);
                let details = form.to_details(Some(id.clone()));
                self.close_connection_dialog(window, cx);
                self.start_connect(id, form.name.clone().into(), details, Some(save), window, cx);
            }
            ConnectionDialogEvent::Save { id, form } => {
                let input = form.to_save_input(Some(id.clone()), form.save_password);
                match config_store::save_connection(&backend::get().events, input) {
                    Ok(_) => {
                        self.reload_connections();
                        self.close_connection_dialog(window, cx);
                    }
                    Err(error) => prompts::alert_later(window, cx, Options::new("编辑连接失败").message(error.to_string())),
                }
            }
        }
    }

    fn close_connection_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connection_dialog = None;
        self.refocus(window, cx);
        cx.notify();
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| SettingsDialog::new(window, cx));
        self._subscriptions.push(cx.subscribe_in(&dialog, window, |this, _, event: &SettingsDialogEvent, window, cx| match event {
            SettingsDialogEvent::Close => {
                this.settings_dialog = None;
                this.reload_connections();
                this.refocus(window, cx);
                cx.notify();
            }
        }));
        self.settings_dialog = Some(dialog);
        cx.notify();
    }

    /// 对话框关掉后焦点回到当前终端（否则要再点一下才能继续输入）。
    pub(crate) fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_tab == MainTab::Files && self.active.is_some() {
            let focus = self.file_manager.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            return;
        }
        if self.main_tab == MainTab::System && self.active.is_some() {
            let focus = self.system_panel.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            return;
        }
        if self.main_tab == MainTab::Terminal
            && let Some(session) = self.active_session() {
                let focus = session.view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
                return;
            }
        window.focus(&self.focus, cx);
    }

    // ---------- 填充密码 ----------

    pub(crate) fn fill_password_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection_id) = self.active.clone() else { return };
        let creds = config_store::cred_list(connection_id);
        if usize::from(creds.has_password) + creds.accounts.len() <= 1 {
            if creds.has_password {
                return self.fill_password("password".into(), window, cx);
            }
            if let [account] = creds.accounts.as_slice() {
                return self.fill_password(format!("acct:{account}"), window, cx);
            }
        }
        self.open_fill_menu(window, cx);
    }

    pub(crate) fn open_fill_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection_id) = self.active.clone() else { return };
        let creds = config_store::cred_list(connection_id);
        self.fill_menu = Some(FillMenu {
            has_password: creds.has_password,
            accounts: creds.accounts,
            account: cx.new(|cx| InputState::new(window, cx).placeholder("账号")),
            password: cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("密码")),
        });
        cx.notify();
    }

    pub(crate) fn fill_password(&mut self, kind: String, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_menu = None;
        let Some(session_id) = self.active_session().and_then(|s| s.view.read(cx).session_id().map(str::to_string)) else { return };
        let fill = backend::spawn(async move { ssh::fill_password(&backend::get().state, &session_id, Some(kind)).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = fill.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Some(Ok(())) => {
                        this.fill_feedback = true;
                        this.fill_feedback_task = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(Duration::from_millis(1500)).await;
                            let _ = this.update(cx, |this, cx| {
                                this.fill_feedback = false;
                                cx.notify();
                            });
                        }));
                        this.refocus(window, cx);
                    }
                    Some(Err(error)) => prompts::alert_later(window, cx, Options::new("填充密码失败").message(error.to_string())),
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn save_fill_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(connection_id), Some(menu)) = (self.active.clone(), &self.fill_menu) else { return };
        let account = menu.account.read(cx).value().trim().to_string();
        let password = menu.password.read(cx).value().to_string();
        if account.is_empty() || password.is_empty() {
            return;
        }
        if let Err(error) = config_store::cred_set(connection_id, account, password) {
            prompts::alert_later(window, cx, Options::new("保存失败").message(error.to_string()));
        }
        self.open_fill_menu(window, cx);
    }

    /// 有对话框 / 提示框开着，或正在连接（连接遮罩）：这时快捷键不在它们背后切标签、切视图、弹面板。
    fn modal_open(&self, cx: &App) -> bool {
        let dialog_open = self.connection_dialog.is_some() || self.import_dialog.is_some() || self.settings_dialog.is_some() || self.snippet_dialog.is_some();
        self.pending.is_some() || dialog_open || prompts::is_open(cx)
    }

    /// 快捷键切标签 / 切视图之前：有对话框时不动（返回 false）；浮着的面板（⌘K、命令片段、填充密码、右键菜单）先收掉。
    fn dismiss_panels_for_shortcut(&mut self, cx: &App) -> bool {
        if self.modal_open(cx) {
            return false;
        }
        self.quick_open = false;
        self.snippets_open = false;
        self.fill_menu = None;
        self.menu = None;
        true
    }

    /// ⌘I / ⌘O / ⌘P：切视图。
    fn show_main_tab(&mut self, tab: MainTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismiss_panels_for_shortcut(cx) {
            self.set_main_tab(tab, window, cx);
            // 视图没变（已经在这一页）时 set_main_tab 不动：面板收掉后焦点也要回内容区
            self.refocus(window, cx);
            cx.notify();
        }
    }

    // ---------- 快速连接（⌘K） ----------

    fn toggle_quick_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_open {
            return self.close_quick_connect(window, cx);
        }
        // 正在连 / 有对话框或提示框时不弹，免得叠在上面抢焦点
        if self.modal_open(cx) {
            return;
        }
        self.snippets_open = false;
        self.fill_menu = None;
        self.menu = None;
        let list = quick_connect::sidebar_order(&self.connections, &self.group_order);
        let connected = self.sessions.iter().map(|s| s.connection_id.clone()).collect();
        self.quick_open = true;
        self.quick_connect.update(cx, |panel, cx| panel.open(list, connected, window, cx));
        cx.notify();
    }

    fn close_quick_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.quick_open {
            return;
        }
        self.quick_open = false;
        self.refocus(window, cx);
        cx.notify();
    }

    fn quick_connect_event(&mut self, _: &Entity<QuickConnect>, event: &QuickConnectEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            QuickConnectEvent::Connect(id) => {
                self.close_quick_connect(window, cx);
                self.connect_to_saved(id, window, cx);
            }
            QuickConnectEvent::Close => self.close_quick_connect(window, cx),
        }
    }

    // ---------- 命令片段 ----------

    /// 命令片段归属的连接 id 和名字：本机标签共用一个（[`LOCAL_SNIPPET_SCOPE`]）。
    fn snippet_scope(&self) -> Option<(String, SharedString)> {
        let id = self.active.clone()?;
        if self.session(&id).is_some_and(|s| s.local) {
            return Some((LOCAL_SNIPPET_SCOPE.into(), "本机".into()));
        }
        let name = self
            .connection(&id)
            .map(|c| SharedString::from(c.name.clone()))
            .or_else(|| self.session(&id).map(|s| s.name.clone()))
            .unwrap_or_default();
        Some((id, name))
    }

    pub(crate) fn toggle_snippets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.snippets_open {
            return self.close_snippets(window, cx);
        }
        let Some((connection_id, _)) = self.snippet_scope() else { return };
        if self.modal_open(cx) {
            return;
        }
        self.fill_menu = None;
        self.snippets_open = true;
        self.snippet_panel.update(cx, |panel, cx| panel.open(connection_id, window, cx));
        cx.notify();
    }

    pub(crate) fn close_snippets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.snippets_open {
            return;
        }
        self.snippets_open = false;
        self.refocus(window, cx);
        cx.notify();
    }

    fn snippet_panel_event(&mut self, _: &Entity<SnippetPanel>, event: &SnippetPanelEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            SnippetPanelEvent::Run { command, execute } => {
                self.snippets_open = false;
                self.run_in_terminal(command, *execute, window, cx);
            }
            SnippetPanelEvent::New => self.open_snippet_dialog(None, window, cx),
            SnippetPanelEvent::Edit(snippet) => self.open_snippet_dialog(Some(snippet.clone()), window, cx),
            SnippetPanelEvent::Close => self.close_snippets(window, cx),
        }
    }

    /// 发到当前会话的终端（不在终端页就先切过去）。
    fn run_in_terminal(&mut self, command: &str, execute: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.active_session().map(|s| s.view.clone()) else { return };
        if self.main_tab != MainTab::Terminal {
            self.set_main_tab(MainTab::Terminal, window, cx);
        }
        view.update(cx, |view, _| view.send_command(command, execute));
        self.refocus(window, cx);
        cx.notify();
    }

    fn open_snippet_dialog(&mut self, editing: Option<sshl_core::config_store::Snippet>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection_id, name)) = self.snippet_scope() else { return };
        self.snippets_open = false;
        let dialog = cx.new(|cx| SnippetDialog::new(editing, connection_id, name, window, cx));
        // 存好或取消都回到片段面板：新建的那条马上就能点
        self._subscriptions.push(cx.subscribe_in(&dialog, window, |this, _, _: &SnippetDialogEvent, window, cx| {
            this.snippet_dialog = None;
            this.toggle_snippets(window, cx);
        }));
        self.snippet_dialog = Some(dialog);
        cx.notify();
    }

    // ---------- 快捷键 ----------

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        // 正在改名：先收掉。不然断开的确认框会记着「关掉后把焦点还给改名输入框」，取消时输入框已经没了，
        // 焦点落空，键盘什么都按不动
        self.finish_rename(true, window, cx);
        match self.active.clone() {
            Some(id) => self.disconnect(&id, window, cx),
            None => cx.hide(),
        }
    }

    fn minimize(&mut self, _: &Minimize, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }

    pub(crate) fn set_main_tab(&mut self, tab: MainTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_none() || self.main_tab == tab {
            return;
        }
        // 本机标签没有文件管理 / 系统页
        if tab != MainTab::Terminal && self.active_session().is_some_and(|s| s.local) {
            return;
        }
        self.main_tab = tab;
        self.fill_menu = None;
        self.file_manager.update(cx, |files, cx| files.set_active(tab == MainTab::Files, window, cx));
        // 系统页的命令以文件管理当前身份跑：先跟上终端此刻的用户（sudo -i 之后是 root）
        if tab == MainTab::System {
            self.file_manager.update(cx, |files, cx| files.sync_identity_quietly(window, cx));
        }
        self.system_panel.update(cx, |panel, cx| panel.set_active(tab == MainTab::System, window, cx));
        self.refocus(window, cx);
        cx.notify();
    }

    fn modifiers_changed(&mut self, event: &ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &event.modifiers;
        let only_cmd = m.platform && !m.shift && !m.alt && !m.control;
        // 有对话框 / 提示框时不浮角标：那些快捷键这时不管用，角标还会盖在遮罩上
        if only_cmd && self.active.is_some() && !self.modal_open(cx) {
            if !self.hint_visible && self.hint_task.is_none() {
                self.hint_task = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(HINT_DELAY).await;
                    let _ = this.update(cx, |this, cx| {
                        this.hint_task = None;
                        this.hint_visible = true;
                        cx.notify();
                    });
                }));
            }
        } else {
            self.hint_task = None;
            if self.hint_visible {
                self.hint_visible = false;
                cx.notify();
            }
        }
    }

    /// 捕获阶段：延时内按了别的键（⌘C 等）就不浮出角标；连接中按 Esc 取消（有提示框时 Esc 归提示框）。
    fn capture_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.hint_task = None;
        if event.keystroke.key == "escape" && self.pending.is_some() && !prompts::is_open(cx) {
            self.cancel_connect(cx);
            cx.stop_propagation();
        }
    }
}

/// 新连接的 id：毫秒时间戳（同旧版）。
fn new_connection_id() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis().to_string()).unwrap_or_default()
}

impl ConnectionForm {
    pub fn to_details(&self, id: Option<String>) -> ConnectionDetails {
        let key = self.auth == AuthKind::Key;
        ConnectionDetails {
            id,
            host: self.host.trim().to_string(),
            port: self.port,
            username: self.username.trim().to_string(),
            password: (self.auth == AuthKind::Password).then(|| self.password.clone()),
            private_key: key.then(|| self.private_key.clone()),
            passphrase: (key && !self.passphrase.is_empty()).then(|| self.passphrase.clone()),
            auth: self.auth,
            cols: 0,
            rows: 0,
        }
    }

    pub fn to_save_input(&self, id: Option<String>, save_password: bool) -> SaveConnectionInput {
        let key = self.auth == AuthKind::Key;
        SaveConnectionInput {
            id,
            name: self.name.trim().to_string(),
            host: self.host.trim().to_string(),
            port: self.port,
            username: self.username.trim().to_string(),
            password: (self.auth == AuthKind::Password && save_password).then(|| self.password.clone()),
            private_key: key.then(|| self.private_key.clone()),
            passphrase: (key && save_password && !self.passphrase.is_empty()).then(|| self.passphrase.clone()),
            group: self.group.trim().to_string(),
            auth: self.auth,
        }
    }
}

/// 终端界面右下角的传输进度（从终端拖文件上传时，不用切到文件管理也看得到）。
fn transfer_toast(progress: u32, info: String, running: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .absolute()
        .right(px(12.))
        .bottom(px(12.))
        .w(px(360.))
        .px_3()
        .py(px(10.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::color(ui::BORDER))
        .bg(theme::color(ui::CARD))
        .shadow_lg()
        .text_size(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w_0().truncate().child(info))
                .when(running, |row| {
                    row.child(
                        kit::mini_button("toast-stop", crate::assets::Icon::X, "停止传输", ui::MUTED_FOREGROUND)
                            .size(px(20.))
                            .on_click(cx.listener(|this, _, _, cx| this.file_manager.update(cx, |files, cx| files.stop_transfers(cx)))),
                    )
                }),
        )
        .child(
            div()
                .h(px(4.))
                .rounded_full()
                .overflow_hidden()
                .bg(theme::alpha(ui::PRIMARY, 0.2))
                .child(div().h_full().w(gpui::relative(progress.min(100) as f32 / 100.)).bg(theme::color(ui::PRIMARY))),
        )
}

/// 红绿灯（`main.rs` 的 `traffic_light_position`）连同右边留白占的宽度：侧栏收起时顶栏从这里往右排。
pub(crate) const TRAFFIC_LIGHTS_WIDTH: f32 = 84.;
/// 侧栏收起时右栏的上边距：让 34px 高的顶栏和红绿灯竖直居中对齐。
pub(crate) const TOPBAR_TOP_BESIDE_TRAFFIC_LIGHTS: f32 = 7.;
/// 顶栏两边的留白；侧栏展开时文件管理 / 系统页到窗口右边、下边也留这么多（左边紧挨侧栏只留 4px）。
/// 终端页（和没有会话时的空页面）任何时候都贴满：左边贴侧栏 / 窗口，右边、下边贴窗口，同 Ghostty；
/// 侧栏收起时文件管理 / 系统页也贴满。
pub(crate) const CONTENT_EDGE: f32 = 6.;

/// 拖动区里的控件（标签、按钮）包一层：在它们上面按下不把事件冒泡给拖动区，
/// 否则拖标签排序时整个窗口跟着走，点按钮时手一抖窗口也会挪。控件自己的点击、拖动不受影响（子元素先收到按下）。
pub fn controls(id: impl Into<gpui::ElementId>) -> gpui::Stateful<gpui::Div> {
    div().id(id).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// 透明标题栏下的拖动区：按住拖动窗口，双击缩放（同系统标题栏）。
pub fn drag_region(id: impl Into<gpui::ElementId>) -> gpui::Stateful<gpui::Div> {
    div().id(id).on_mouse_down(MouseButton::Left, |event, window, _| {
        if event.click_count >= 2 {
            window.titlebar_double_click();
        } else {
            window.start_window_move();
        }
    })
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let terminal = self.active_session().map(|s| s.view.clone());
        let entity = cx.entity();
        let overlay = self.pending.as_ref().filter(|p| p.show_overlay).map(|_| self.connecting_overlay(cx));
        let menu = self.connection_menu_overlay(window, cx);
        let sidebar_collapsed = settings::get(cx).sidebar_collapsed;
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::color(ui::SHELL))
            .text_color(theme::color(ui::FOREGROUND))
            .text_size(px(14.))
            .key_context("Workspace")
            .track_focus(&self.focus)
            // GPUI 在最后一次输入是键盘时把所有元素都当作没被悬停，而 Finder 的拖放事件不会切回鼠标：
            // 刚在终端敲完 cd、或按 ⌘O 切到文件管理，再从 Finder 拖文件进来，松手什么都不会发生。
            // 拖进窗口时补一个鼠标移动，把输入方式切回鼠标，终端和文件管理的 on_drop 才收得到。
            .on_drag_move::<ExternalPaths>(|event: &DragMoveEvent<ExternalPaths>, window, cx| {
                if window.last_input_was_keyboard() {
                    let position = event.event.position;
                    window.defer(cx, move |window, cx| {
                        window.dispatch_event(
                            PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }),
                            cx,
                        );
                    });
                }
            })
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::minimize))
            .on_action(cx.listener(|this, _: &ShowTerminal, window, cx| this.show_main_tab(MainTab::Terminal, window, cx)))
            .on_action(cx.listener(|this, _: &ShowFiles, window, cx| this.show_main_tab(MainTab::Files, window, cx)))
            .on_action(cx.listener(|this, _: &ShowSystem, window, cx| this.show_main_tab(MainTab::System, window, cx)))
            .on_action(cx.listener(|this, _: &ShowSnippets, window, cx| this.toggle_snippets(window, cx)))
            .on_action(cx.listener(|this, _: &NewConnection, window, cx| this.open_connection_dialog(None, window, cx)))
            .on_action(cx.listener(|this, _: &NewLocalTerminal, window, cx| this.new_local_terminal(window, cx)))
            .on_action(cx.listener(Self::select_nth_tab))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.cycle_tab(-1, window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)))
            .on_action(cx.listener(|this, _: &OpenQuickConnect, window, cx| this.toggle_quick_connect(window, cx)))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .capture_key_down(cx.listener(Self::capture_key))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    // 收起 = 整个侧栏不画，展开按钮挪到顶栏红绿灯右边（`top_bar`）
                    .when(!sidebar_collapsed, |row| row.child(self.sidebar(window, cx)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            // 上边距算在顶栏里（顶栏是拖动区，窗口最上面那条也要能拖），见 `top_bar`
                            .child(self.top_bar(window, cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .relative()
                                    .when(!sidebar_collapsed && terminal.is_some() && self.main_tab != MainTab::Terminal, |area| {
                                        area.ml_1().mr(px(CONTENT_EDGE)).mb(px(CONTENT_EDGE))
                                    })
                                    .child(
                                        canvas(
                                            move |bounds, _, cx| {
                                                entity.update(cx, |this, _| this.terminal_area = Some(bounds));
                                            },
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .size_full(),
                                    )
                                    .map(|area| match (terminal, self.main_tab) {
                                        (Some(view), MainTab::Terminal) => area
                                            .child(div().absolute().size_full().child(view))
                                            .when_some(self.transfer_shown.clone(), |area, (progress, info, running)| {
                                                area.child(transfer_toast(progress, info, running, cx))
                                            }),
                                        (Some(_), MainTab::Files) => area.child(div().absolute().size_full().child(self.file_manager.clone())),
                                        (Some(_), MainTab::System) => area.child(div().absolute().size_full().child(self.system_panel.clone())),
                                        (None, _) => area.child(
                                            div()
                                                .absolute()
                                                .size_full()
                                                .bg(theme::color(0x1d1d1d))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .flex_col()
                                                .gap_4()
                                                .child(div().text_sm().text_color(theme::color(0xcbd5e1)).child(if sidebar_collapsed { "按 ⌘K 快速连接，或新建连接开始使用" } else { "点击左侧连接或创建新连接开始使用" }))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .gap_3()
                                                        .child(
                                                            kit::labeled_button("empty-new-connection", kit::Variant::Primary, Some(crate::assets::Icon::Plus), "新建连接")
                                                                .h(px(32.))
                                                                .text_size(px(13.))
                                                                .on_click(cx.listener(|this, _, window, cx| this.open_connection_dialog(None, window, cx))),
                                                        )
                                                        .child(
                                                            kit::labeled_button("empty-local-terminal", kit::Variant::Outline, Some(crate::assets::Icon::SquareTerminal), "本机终端 ⌘T")
                                                                .h(px(32.))
                                                                .text_size(px(13.))
                                                                .on_click(cx.listener(|this, _, window, cx| this.new_local_terminal(window, cx))),
                                                        ),
                                                ),
                                        ),
                                    }),
                            ),
                    ),
            )
            .when_some(overlay, |root, overlay| root.child(overlay))
            .when_some(menu, |root, menu| root.child(menu))
            // 快速连接：浮在顶部居中，点面板外面关掉
            .when(self.quick_open, |root| {
                root.child(
                    div()
                        .id("quick-connect-layer")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .justify_center()
                        // 不让下面那层拦截点击的 div 被拉成整列高：面板正下方也算「外面」
                        .items_start()
                        .pt(px(72.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_quick_connect(window, cx)))
                        .child(div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(self.quick_connect.clone())),
                )
            })
            .when_some(self.connection_dialog.clone(), |root, dialog| root.child(dialog))
            .when_some(self.import_dialog.clone(), |root, dialog| root.child(dialog))
            .when_some(self.settings_dialog.clone(), |root, dialog| root.child(dialog))
            .when_some(self.snippet_dialog.clone(), |root, dialog| root.child(dialog))
            .child(self.prompts.clone())
    }
}

impl Workspace {
    fn connecting_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("connecting")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::alpha(ui::CARD, 0.7))
            .occlude()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_5()
                    .py_4()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme::color(ui::BORDER))
                    .bg(theme::color(ui::CARD))
                    .shadow_xl()
                    .child(kit::icon(crate::assets::Icon::LoaderCircle).text_color(theme::color(ui::PRIMARY)))
                    .child("正在连接服务器...")
                    .child(
                        kit::button("cancel-connect", kit::Variant::Outline)
                            .h(px(28.))
                            .px_2()
                            .text_size(px(12.))
                            .child("取消")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_connect(cx))),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Action, KeyContext, Keymap, Keystroke};

    use super::*;

    #[test]
    fn cmd_numbers_switch_tabs_and_cmd_letters_switch_views() {
        let mut keymap = Keymap::new(key_bindings());
        // 终端自己的 ⌘ 键（⌘C / ⌘F / ⌘0 …）不能把这些抢走
        keymap.add_bindings(crate::terminal::view::key_bindings());
        let stack: Vec<KeyContext> = ["Root", "Workspace", "Terminal"].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let first = |key: &'static str| -> Option<Box<dyn Action>> {
            let bindings = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack).0;
            bindings.first().map(|b| b.action().boxed_clone())
        };
        for (key, n) in TAB_KEYS.into_iter().zip(1..) {
            assert_eq!(key, format!("cmd-{n}"));
            let action = first(key).unwrap_or_else(|| panic!("{key} 没有绑定"));
            assert!(action.partial_eq(&SelectTab(n)), "{key} 应切到第 {n} 个标签，实际是 {}", action.name());
        }
        let name = |key: &'static str| first(key).map(|a| a.name());
        assert_eq!(name("cmd-i"), Some("workspace::ShowTerminal"));
        assert_eq!(name("cmd-o"), Some("workspace::ShowFiles"));
        assert_eq!(name("cmd-p"), Some("workspace::ShowSystem"));
        assert_eq!(name("cmd-shift-p"), Some("workspace::ShowSnippets"));
        assert_eq!(name("cmd-0"), Some("terminal::ZoomReset"), "⌘0 还是终端的字号复原，不是标签");
    }

    fn form(auth: AuthKind) -> ConnectionForm {
        ConnectionForm {
            name: " web ".into(),
            group: "".into(),
            host: " example.com ".into(),
            port: 22,
            username: "root".into(),
            auth,
            password: "pw".into(),
            private_key: "/k/id_ed25519".into(),
            passphrase: "".into(),
            save_password: true,
        }
    }

    #[test]
    fn each_auth_method_carries_only_its_own_credentials() {
        let auto = form(AuthKind::Auto);
        let details = auto.to_details(None);
        assert_eq!((details.password, details.private_key, details.auth), (None, None, AuthKind::Auto));
        let saved = auto.to_save_input(None, true);
        assert_eq!((saved.password, saved.private_key, saved.auth), (None, None, AuthKind::Auto), "免密不存密码 / 私钥");

        let key = form(AuthKind::Key);
        let details = key.to_details(None);
        assert_eq!((details.password, details.private_key.as_deref(), details.passphrase), (None, Some("/k/id_ed25519"), None));
        assert_eq!(key.to_save_input(None, true).auth, AuthKind::Key);

        let password = form(AuthKind::Password);
        assert_eq!(password.to_details(None).password.as_deref(), Some("pw"));
        assert_eq!(password.to_save_input(None, false).password, None, "不勾保存密码就不存");
        assert_eq!(password.to_save_input(Some("1".into()), true).host, "example.com");
    }
}

//! 终端视图：铺满内容区的深色直角卡片，四边 4px 内边距。左、右、上的内边距用边缘格子的底色延伸
//! （全屏 TUI 的 cursorline 贴到卡片边）；底边那 4px 不延伸，是卡片的底色。

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::channel::oneshot;
use gpui::{
    App, AppContext, BorderStyle, Bounds, ClipboardItem, Context, Corners, Element, ElementId, ElementInputHandler,
    DragMoveEvent, Entity, EntityInputHandler, EventEmitter, ExternalPaths, FocusHandle, Focusable, Font, FontFallbacks, FontStyle, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, LayoutId, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, NoAction, ParentElement, Pixels, Point, Render, ScrollEvent, SharedString,
    Role, StatefulInteractiveElement, StrikethroughStyle, Style, Styled, Subscription, Task, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
    fill, font, point, prelude::FluentBuilder, px, quad, relative, size,
};
use gpui_base::input::{Input as TextInput, InputEvent, InputState};
use libghostty_vt::{key, mouse};
use sshl_core::error::AppError;
use sshl_core::known_hosts::{HostKeyCheck, HostKeyPrompt, HostKeyQuestion};
use sshl_core::local_shell::{self, LocalOptions, LocalShell};
use sshl_core::ssh::{self, ConnectionDetails, ShellHandle, TerminalSink};

use super::engine::{self, GridSize, Input, MouseAction, MouseInput, TerminalHandle};
use super::frame::{Frame, Link, MatchSpan, Rgb, SearchInfo, flags, link_at, openable};
use super::keys;
use crate::assets::Icon;
use crate::backend;
use crate::prompts::{self, Options};
use crate::theme::{self, TERMINAL_PADDING};
use crate::ui as kit;

actions!(terminal, [Copy, Paste, SelectAll, Find, FindNext, FindPrev, FindClose, ZoomIn, ZoomOut, ZoomReset]);

const CONTEXT: &str = "Terminal";
/// 搜索条（⌘F）。它和终端是并排的兄弟，不在终端的上下文里：在里面敲的字不会被终端的按键处理发给远端。
const SEARCH_CONTEXT: &str = "TerminalSearch";

/// 终端里的按键绑定。Tab / Shift-Tab 被 gpui-base 的 Root 绑去切换焦点，在终端里压掉，
/// 让它们落到 key_down 编码给远端（补全、反向补全）。
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("cmd-f", Find, Some(CONTEXT)),
        KeyBinding::new("cmd-g", FindNext, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-g", FindPrev, Some(CONTEXT)),
        KeyBinding::new("cmd-f", Find, Some(SEARCH_CONTEXT)),
        KeyBinding::new("cmd-g", FindNext, Some(SEARCH_CONTEXT)),
        KeyBinding::new("cmd-shift-g", FindPrev, Some(SEARCH_CONTEXT)),
        // 绑在比 gpui-base 输入框更深的上下文里，压过它自己的回车 / Esc
        KeyBinding::new("enter", FindNext, Some("TerminalSearch > Input")),
        KeyBinding::new("shift-enter", FindPrev, Some("TerminalSearch > Input")),
        KeyBinding::new("escape", FindClose, Some("TerminalSearch > Input")),
        // 字号：美式键盘上 ⌘⇧= 报成 cmd-+，不按 Shift 的 ⌘= 也算放大（同浏览器）
        KeyBinding::new("cmd-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd-+", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd--", ZoomOut, Some(CONTEXT)),
        KeyBinding::new("cmd-0", ZoomReset, Some(CONTEXT)),
        KeyBinding::new("tab", NoAction, Some(CONTEXT)),
        KeyBinding::new("shift-tab", NoAction, Some(CONTEXT)),
    ]
}

// 测试口用：把选中文字打进日志（不碰系统剪贴板）。
#[cfg(any(debug_assertions, feature = "devhook"))]
actions!(terminal, [LogSelection]);

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// 终端字体度量：一格多宽、一行多高。按设备像素取整，相邻格子的底色之间不留发丝缝。
#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub font_size: Pixels,
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub scale: f32,
}

impl Metrics {
    pub fn measure(window: &mut Window, cx: &App) -> Self {
        let settings = crate::settings::get(cx);
        let scale = window.scale_factor();
        let size = px(settings.font_size);
        let text = window.text_system();
        let id = text.resolve_font(&font(settings.font_family.clone()));
        let advance = text.advance(id, size, 'm').map(|s| s.width).unwrap_or(size * 0.6);
        let height = text.ascent(id, size) + text.descent(id, size).abs();
        let snap = |v: Pixels| px((f32::from(v) * scale).round().max(1.0) / scale);
        Self { font_size: size, cell_width: snap(advance), line_height: snap(height), scale }
    }

    /// 在 `available` 大小的区域里能排下的网格（四边扣掉内边距）。
    pub fn grid(&self, available: gpui::Size<Pixels>) -> GridSize {
        let pad = px(TERMINAL_PADDING) * 2.;
        let cols = ((available.width - pad) / self.cell_width).floor().max(1.0) as u16;
        let rows = ((available.height - pad) / self.line_height).floor().max(1.0) as u16;
        GridSize {
            cols,
            rows,
            cell_width: (f32::from(self.cell_width) * self.scale).round() as u32,
            cell_height: (f32::from(self.line_height) * self.scale).round() as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    Connected { session_id: String },
    /// 本机 shell 在跑。
    Local,
    Failed(String),
    Closed,
}

pub enum TerminalEvent {
    Connected,
    Failed(String),
    /// 用户在主机身份确认框里取消了：不连，也不用再报错。
    Declined,
    /// 从 Finder 拖到终端上的文件（本地路径），由工作区传到终端当前目录。
    FilesDropped(Vec<String>),
    /// 断开后在终端里按了回车：请工作区在这个标签里重连。
    ReconnectRequested,
    Closed,
    /// 光标行出现 / 离开密码提示（当前状态见 `password_prompt()`）。
    PasswordPrompt,
}

enum UiMsg {
    Engine(engine::Event),
    Closed,
}

struct Sink {
    output: std::sync::mpsc::Sender<Input>,
    ui: UnboundedSender<UiMsg>,
}

impl TerminalSink for Sink {
    fn data(&self, bytes: &[u8]) {
        let _ = self.output.send(Input::Output(bytes.to_vec()));
    }

    fn closed(&self, code: Option<u32>, reason: String) {
        tracing::info!("terminal closed code={code:?} reason={reason}");
        let _ = self.ui.unbounded_send(UiMsg::Closed);
    }
}

static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(1);

/// 拖文件到终端上时盖在上面的提示。
fn drop_hint() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .border_2()
        .border_color(theme::color(theme::ui::PRIMARY))
        .bg(theme::alpha(theme::ui::PRIMARY, 0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .px_4()
                .py_2()
                .rounded(px(10.))
                .bg(theme::color(theme::ui::CARD))
                .text_color(theme::color(theme::ui::FOREGROUND))
                .text_size(px(14.))
                .shadow_md()
                .child("松开上传到终端当前目录"),
        )
}

/// 主机身份确认：后端握手时问到这里，在界面线程弹应用内确认框，答案送回去。
fn host_key_prompt(window: &mut Window, cx: &mut Context<TerminalView>) -> HostKeyPrompt {
    let (tx, mut rx) = unbounded::<(HostKeyQuestion, oneshot::Sender<bool>)>();
    cx.spawn_in(window, async move |_, cx| {
        while let Some((question, reply)) = rx.next().await {
            let answer = cx.update(|window, cx| prompts::confirm(window, cx, host_key_options(&question)));
            let trusted = match answer {
                Ok(answer) => answer.await.unwrap_or(false),
                Err(_) => false,
            };
            let _ = reply.send(trusted);
        }
    })
    .detach();
    Arc::new(move |question| {
        let tx = tx.clone();
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            if tx.unbounded_send((question, reply)).is_err() {
                return false;
            }
            answer.await.unwrap_or(false)
        })
    })
}

fn host_key_options(question: &HostKeyQuestion) -> Options {
    let host = if question.port == 22 { question.host.clone() } else { format!("{}:{}", question.host, question.port) };
    let fingerprint = format!("{} 密钥指纹\n{}", question.key_type, question.fingerprint);
    if question.changed {
        let mut options = Options::new(format!("{host} 的主机密钥变了"))
            .message(format!(
                "和上次记录的不一样。可能是服务器重装或换了密钥，也可能有人在冒充它（中间人攻击）。不确定就取消，先问问服务器管理员。\n\n{fingerprint}"
            ))
            .confirm("信任新密钥");
        options.destructive = true;
        options
    } else {
        Options::new(format!("第一次连接 {host}"))
            .message(format!("请核对服务器的密钥指纹，确认无误再连接。\n\n{fingerprint}"))
            .confirm("信任并连接")
    }
}

pub struct TerminalView {
    handle: TerminalHandle,
    shell: ShellHandle,
    /// 本机终端（不是 SSH）。
    local: Option<LocalShell>,
    /// shell 输出的去处（终端线程 + 界面消息），重连时接到新通道上。
    sink: Arc<Sink>,
    connect_task: Option<Task<()>>,
    frame: Arc<Frame>,
    focus: FocusHandle,
    pub status: Status,
    pub attempt_id: String,
    grid: GridSize,
    /// 网格左上角在窗口里的位置（上一帧），鼠标坐标据此换算。
    grid_origin: Point<Pixels>,
    metrics: Option<Metrics>,
    blink_on: bool,
    blink_reset: Instant,
    /// 上次渲染时终端有没有焦点（窗口在前台）。没焦点时光标画成不闪的细线，闪烁定时器不用再触发重绘。
    focused: bool,
    /// 输入法正在组的字（还没上屏）。
    marked: Option<String>,
    password_prompt: bool,
    /// 正从 Finder 拖文件到终端上方（画「松开上传」的提示）。
    drop_hover: bool,
    scroll_remainder: f32,
    /// 搜索条（⌘F）的输入框；`search_open` 时显示在右上角。
    search: Entity<InputState>,
    search_open: bool,
    _subscriptions: Vec<Subscription>,
    /// 按住 ⌘ 时鼠标下的纯文本网址（画下划线、手形光标）。
    hover_link: Option<Link>,
    /// 鼠标最近一次在终端上的位置（窗口坐标）；移出终端是 None。按下 / 松开 ⌘ 时据此重新找链接。
    pointer: Option<Point<Pixels>>,
    /// ⌘ 按着。
    cmd_down: bool,
    /// 这次左键按下是 ⌘ 点链接（按下的位置）：之后的拖动和松开都不交给选区、也不上报给程序，松开时打开。
    link_press: Option<Point<Pixels>>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    pub fn connect(details: ConnectionDetails, grid: GridSize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (mut view, shell_rx) = Self::new(grid, window, cx);
        view.start_connection(details, shell_rx, window, cx);
        view
    }

    /// 本机终端：起登录 shell（`cwd` 为 None 时在主目录）。起不来时状态是 Failed。
    pub fn local(cwd: Option<std::path::PathBuf>, grid: GridSize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (mut view, shell_rx) = Self::new(grid, window, cx);
        let options = LocalOptions { cwd, cols: grid.cols, rows: grid.rows, program: None };
        match local_shell::spawn(options, shell_rx, view.sink.clone()) {
            Ok(shell) => {
                view.local = Some(shell);
                view.status = Status::Local;
            }
            Err(error) => view.status = Status::Failed(error.to_string()),
        }
        view
    }

    fn new(grid: GridSize, window: &mut Window, cx: &mut Context<Self>) -> (Self, ssh::ShellReceiver) {
        let (shell, shell_rx) = ssh::shell_channel();
        let (ui_tx, mut ui_rx) = unbounded::<UiMsg>();
        let notify_tx = ui_tx.clone();
        let handle = engine::spawn(grid, shell.clone(), move |event| {
            let _ = notify_tx.unbounded_send(UiMsg::Engine(event));
        });
        let sink = Arc::new(Sink { output: handle.sender(), ui: ui_tx });

        let events_task = cx.spawn(async move |this, cx| {
            while let Some(msg) = ui_rx.next().await {
                let alive = this.update(cx, |view, cx| match msg {
                    UiMsg::Engine(engine::Event::Frame) => {
                        let frame = view.handle.take_frame();
                        if frame.cursor != view.frame.cursor {
                            view.show_cursor();
                        }
                        view.frame = frame;
                        // 内容滚动了，链接可能不在原处
                        if view.hover_link.is_some() {
                            view.update_hover_link(cx);
                        }
                        if view.frame.password_prompt != view.password_prompt {
                            view.password_prompt = view.frame.password_prompt;
                            cx.emit(TerminalEvent::PasswordPrompt);
                        }
                        cx.notify();
                    }
                    UiMsg::Engine(engine::Event::Bell) => {}
                    UiMsg::Closed => {
                        if view.status == Status::Local {
                            // 本机 shell 退出了：工作区直接关掉标签（同 Ghostty / Terminal）
                            view.status = Status::Closed;
                            cx.emit(TerminalEvent::Closed);
                        } else if matches!(view.status, Status::Connected { .. }) {
                            view.status = Status::Closed;
                            view.notice("连接已断开 · 按 Enter 重新连接");
                            cx.emit(TerminalEvent::Closed);
                            cx.notify();
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let blink_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this.update(cx, |view, cx| {
                    if !view.focused {
                        // 没焦点：光标不闪，不白白重绘整个窗口（焦点回来时本来就会重绘，闪烁接着来）
                        view.blink_on = true;
                    } else if view.blink_reset.elapsed() < BLINK_INTERVAL {
                        // 刚按过键 / 光标刚动过：这一拍不熄，移动中的光标一直看得见
                    } else if view.frame.cursor.is_some_and(|c| c.blinking) {
                        view.blink_on = !view.blink_on;
                        cx.notify();
                    } else if !view.blink_on {
                        view.blink_on = true;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let focus = cx.focus_handle();
        cx.on_focus(&focus, window, |view, _, cx| {
            view.handle.send(Input::Focus(true));
            view.show_cursor();
            cx.notify();
        })
        .detach();
        cx.on_blur(&focus, window, |view, _, cx| {
            view.handle.send(Input::Focus(false));
            cx.notify();
        })
        .detach();

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("查找"));
        let search_subscription = cx.subscribe(&search, |view, search, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && view.search_open {
                view.handle.send(Input::Search(search.read(cx).value().to_string()));
            }
        });

        // 窗口退到后台（⌘ 点链接后浏览器抢了前台）：之后松开 ⌘ 我们收不到，回来时下划线不能还留着
        let activation = cx.observe_window_activation(window, |view, window, cx| {
            if !window.is_window_active() {
                view.cmd_down = false;
                view.link_press = None;
                view.update_hover_link(cx);
            }
        });

        let view = Self {
            handle,
            shell,
            local: None,
            sink,
            connect_task: None,
            frame: Arc::new(Frame::default()),
            focus,
            status: Status::Connecting,
            attempt_id: String::new(),
            grid,
            grid_origin: Point::default(),
            metrics: None,
            blink_on: true,
            blink_reset: Instant::now(),
            focused: false,
            marked: None,
            password_prompt: false,
            drop_hover: false,
            scroll_remainder: 0.0,
            search,
            search_open: false,
            _subscriptions: vec![search_subscription, activation],
            hover_link: None,
            pointer: None,
            cmd_down: false,
            link_press: None,
            _tasks: vec![events_task, blink_task],
        };
        (view, shell_rx)
    }

    /// 发起连接（首次 / 重连），结果更新状态并通知工作区。
    fn start_connection(&mut self, details: ConnectionDetails, shell_rx: ssh::ShellReceiver, window: &mut Window, cx: &mut Context<Self>) {
        let mut details = details;
        details.cols = u32::from(self.grid.cols);
        details.rows = u32::from(self.grid.rows);
        self.attempt_id = format!("connect-{}", NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed));
        self.status = Status::Connecting;
        let attempt = self.attempt_id.clone();
        let shell = self.shell.clone();
        let sink = self.sink.clone();
        let host_key = HostKeyCheck::user(Some(host_key_prompt(window, cx)));
        let connecting = backend::spawn(async move {
            ssh::connect(&backend::get().state, details, &attempt, shell, shell_rx, sink, host_key).await
        });
        self.connect_task = Some(cx.spawn(async move |this, cx| {
            let result = connecting.await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Some(Ok(session_id)) => {
                        view.status = Status::Connected { session_id };
                        cx.emit(TerminalEvent::Connected);
                    }
                    Some(Err(AppError::HostKeyDeclined)) => {
                        view.status = Status::Failed("未信任主机".into());
                        view.notice("没有信任主机密钥 · 按 Enter 重试");
                        cx.emit(TerminalEvent::Declined);
                    }
                    Some(Err(error)) => {
                        let message = error.to_string();
                        view.notice(&format!("连接失败：{message} · 按 Enter 重试"));
                        view.status = Status::Failed(message.clone());
                        cx.emit(TerminalEvent::Failed(message));
                    }
                    None => {
                        view.status = Status::Failed("后台任务异常退出".into());
                        cx.emit(TerminalEvent::Failed("后台任务异常退出".into()));
                    }
                }
                cx.notify();
            });
        }));
    }

    /// 断开之后在原来的终端里重连：屏幕和回滚历史都留着，新连接的输出接在下面。
    pub fn reconnect(&mut self, details: ConnectionDetails, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Connecting) {
            return;
        }
        let (shell, shell_rx) = ssh::shell_channel();
        self.shell.close();
        self.shell = shell.clone();
        self.handle.send(Input::Rebind(shell));
        self.notice("正在重新连接…");
        self.start_connection(details, shell_rx, window, cx);
        cx.notify();
    }

    /// 是否已断开（远端关闭、重连失败、没信任主机）：标签还在，按回车可重连。
    pub fn is_disconnected(&self) -> bool {
        matches!(self.status, Status::Closed | Status::Failed(_))
    }

    /// 命令片段发到终端：按粘贴发（多行命令不会一行一行地先执行），`execute` 时再补一个回车。断开时不发。
    pub fn send_command(&mut self, command: &str, execute: bool) {
        if self.is_disconnected() || (self.session_id().is_none() && !self.is_local()) {
            return;
        }
        self.input(Input::Paste(command.to_string()));
        if execute {
            self.input(Input::Text("\r".into()));
        }
    }

    /// 在终端里另起一行写一句灰色的提示（不经过远端）。
    fn notice(&mut self, text: &str) {
        let line = format!("\r\n\x1b[0m\x1b[2m[{text}]\x1b[0m\r\n");
        self.handle.send(Input::Output(line.into_bytes()));
    }

    pub fn session_id(&self) -> Option<&str> {
        match &self.status {
            Status::Connected { session_id } => Some(session_id),
            _ => None,
        }
    }

    pub fn password_prompt(&self) -> bool {
        self.password_prompt
    }

    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    /// 本机终端前台在跑的程序（不是 shell 自己时）：关标签前问一声。
    pub fn local_busy(&self) -> Option<String> {
        self.local.as_ref().filter(|_| self.status == Status::Local)?.foreground_program()
    }

    /// 本机终端此刻的目录：新开的本机标签从这里开始。
    pub fn local_cwd(&self) -> Option<std::path::PathBuf> {
        self.local.as_ref()?.cwd()
    }

    /// 关闭：结束终端通道（远端会话随之断开）。
    pub fn close(&self) {
        self.shell.close();
    }

    /// 光标亮起来，并且闪烁从头计时。
    fn show_cursor(&mut self) {
        self.blink_on = true;
        self.blink_reset = Instant::now();
    }

    fn input(&mut self, input: Input) {
        self.show_cursor();
        self.handle.send(input);
    }

    fn resize(&mut self, grid: GridSize, origin: Point<Pixels>) {
        self.grid_origin = origin;
        if grid != self.grid {
            self.grid = grid;
            self.handle.send(Input::Resize(grid));
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            return;
        }
        // 断开了：回车重连，其他键不往已经关掉的通道里送
        if self.is_disconnected() {
            if event.keystroke.key == "enter" && !event.keystroke.modifiers.modified() {
                cx.emit(TerminalEvent::ReconnectRequested);
            }
            if !event.keystroke.modifiers.platform {
                cx.stop_propagation();
            }
            return;
        }
        if let Some(input) = keys::encode(&event.keystroke, event.is_held) {
            self.input(Input::Key(input));
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_clipboard(cx);
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.input(Input::Paste(text));
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    /// 选中的文字放进剪贴板；没选中就不动剪贴板。
    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.handle.send(Input::Copy(tx));
        cx.spawn(async move |_, cx| {
            if let Ok(Some(text)) = rx.await {
                cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
            }
        })
        .detach();
    }

    #[cfg(any(debug_assertions, feature = "devhook"))]
    fn log_selection(&mut self, _: &LogSelection, _: &mut Window, cx: &mut Context<Self>) {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.handle.send(Input::Copy(tx));
        cx.spawn(async move |_, _| tracing::info!("devhook selection: {:?}", rx.await.ok().flatten())).detach();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, _: &mut Context<Self>) {
        self.handle.send(Input::SelectAll);
    }

    /// ⌘F：打开搜索条（已经开着就回到输入框），上次搜的词留着并全选，直接敲新词就替换。
    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        let needle = self.search.read(cx).value().to_string();
        self.handle.send(Input::Search(needle));
        self.search.update(cx, |search, cx| {
            search.focus(window, cx);
            search.select_all(window, cx);
        });
        cx.notify();
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, _: &mut Context<Self>) {
        self.find_step(true);
    }

    fn find_prev(&mut self, _: &FindPrev, _: &mut Window, _: &mut Context<Self>) {
        self.find_step(false);
    }

    fn find_step(&mut self, older: bool) {
        if self.search_open {
            self.handle.send(Input::SearchStep { older });
        }
    }

    /// 关掉搜索条：高亮去掉，焦点回终端（屏幕停在找到的地方，敲键才回到底部）。
    fn find_close(&mut self, _: &FindClose, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.handle.send(Input::Search(String::new()));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// ⌘+ / ⌘- / ⌘0：改全局字号并存盘（所有终端一起变，上下限由 settings 管）。
    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        crate::settings::update(cx, |s| s.font_size = (s.font_size + 1.).round());
    }

    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        crate::settings::update(cx, |s| s.font_size = (s.font_size - 1.).round());
    }

    fn zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        crate::settings::update(cx, |s| s.font_size = crate::settings::DEFAULT_FONT_SIZE);
    }

    /// 窗口坐标落在哪一格（列, 行）；在网格外是 None。
    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let m = self.metrics.as_ref()?;
        grid_cell(position, self.grid_origin, size(m.cell_width, m.line_height), self.grid)
    }

    /// 按住 ⌘ 时找鼠标下的网址；变了才重绘。
    fn update_hover_link(&mut self, cx: &mut Context<Self>) {
        let link = if self.cmd_down {
            self.pointer.and_then(|p| self.cell_at(p)).and_then(|(x, y)| link_at(&self.frame.rows, x, y))
        } else {
            None
        };
        if link != self.hover_link {
            self.hover_link = link;
            cx.notify();
        }
    }

    fn modifiers_changed(&mut self, event: &ModifiersChangedEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.cmd_down != event.modifiers.platform {
            self.cmd_down = event.modifiers.platform;
            self.update_hover_link(cx);
        }
    }

    /// ⌘ 点击：先问终端线程那一格有没有 OSC 8 超链接（程序明说的目标，显示的字可能是截断的或另一个网址），
    /// 没有再用那里的纯文本网址。只开 http / https / ftp / mailto。
    fn open_link_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((x, y)) = self.cell_at(position) else { return };
        let plain = link_at(&self.frame.rows, x, y).map(|link| link.url);
        let (x, y) = self.mouse_position(position);
        let (reply, answer) = oneshot::channel();
        self.handle.send(Input::LinkAt { x, y, reply });
        cx.spawn(async move |_, cx| {
            if let Some(url) = answer.await.ok().flatten().or(plain) {
                cx.update(|cx| open_link(&url, cx));
            }
        })
        .detach();
    }

    fn search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = search_label(&self.search.read(cx).value(), self.frame.search.as_deref());
        div()
            .id("terminal-search")
            .key_context(SEARCH_CONTEXT)
            .role(Role::Search)
            .aria_label("在终端里查找")
            .absolute()
            .top(px(8.))
            .right(px(12.))
            .occlude()
            .h(px(34.))
            .pl(px(10.))
            .pr(px(4.))
            .flex()
            .items_center()
            .gap(px(2.))
            .rounded(px(9.))
            .border_1()
            .border_color(theme::color(theme::ui::BORDER))
            .bg(theme::color(theme::ui::CARD))
            .text_color(theme::color(theme::ui::FOREGROUND))
            .text_size(px(13.))
            .shadow_md()
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::find_close))
            .child(div().w(px(170.)).child(TextInput::new(&self.search)))
            .child(
                div()
                    .min_w(px(44.))
                    .px(px(6.))
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(theme::color(theme::ui::MUTED_FOREGROUND))
                    .child(label),
            )
            .child(
                kit::mini_button("terminal-search-prev", Icon::ChevronUp, "上一个（↵）", theme::ui::MUTED_FOREGROUND)
                    .on_click(cx.listener(|this, _, _, _| this.find_step(true))),
            )
            .child(
                kit::mini_button("terminal-search-next", Icon::ChevronDown, "下一个（⇧↵）", theme::ui::MUTED_FOREGROUND)
                    .on_click(cx.listener(|this, _, _, _| this.find_step(false))),
            )
            .child(
                kit::mini_button("terminal-search-close", Icon::X, "关闭（Esc）", theme::ui::MUTED_FOREGROUND)
                    .on_click(cx.listener(|this, _, window, cx| this.find_close(&FindClose, window, cx))),
            )
    }

    fn mouse_position(&self, position: Point<Pixels>) -> (f32, f32) {
        let scale = self.metrics.as_ref().map_or(2.0, |m| m.scale);
        (
            f32::from(position.x - self.grid_origin.x) * scale,
            f32::from(position.y - self.grid_origin.y) * scale,
        )
    }

    fn mods(m: &gpui::Modifiers) -> key::Mods {
        let mut mods = key::Mods::empty();
        if m.shift {
            mods |= key::Mods::SHIFT;
        }
        if m.alt {
            mods |= key::Mods::ALT;
        }
        if m.control {
            mods |= key::Mods::CTRL;
        }
        mods
    }

    fn button(button: MouseButton) -> Option<mouse::Button> {
        match button {
            MouseButton::Left => Some(mouse::Button::Left),
            MouseButton::Right => Some(mouse::Button::Right),
            MouseButton::Middle => Some(mouse::Button::Middle),
            _ => None,
        }
    }

    /// 程序开了鼠标上报、且没按 Shift（按住 Shift 留给本地选择）。
    fn reports_mouse(&self, m: &gpui::Modifiers) -> bool {
        self.frame.mouse_tracking && !m.shift
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        // ⌘ 点击开链接：这次按下不做选区、也不上报给程序。每次左键按下都重设标记：
        // 上一次在终端外松开（收不到 mouse_up）时不会卡住，把之后正常的松开也吞掉
        if event.button == MouseButton::Left {
            self.link_press = event.modifiers.platform.then_some(event.position);
            if self.link_press.is_some() {
                return;
            }
        }
        // 右键粘贴（Xshell / PuTTY 习惯）；程序在接收鼠标时右键还是交给它
        if event.button == MouseButton::Right && !self.reports_mouse(&event.modifiers) && crate::settings::get(cx).right_click_paste {
            self.paste_clipboard(cx);
            return;
        }
        if event.button != MouseButton::Left && !self.reports_mouse(&event.modifiers) {
            return;
        }
        if let Some(button) = Self::button(event.button) {
            let (x, y) = self.mouse_position(event.position);
            self.input(Input::Mouse(MouseInput { action: MouseAction::Press(button), x, y, mods: Self::mods(&event.modifiers) }));
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        // ⌘ 点链接那次的松开：不交给选区 / 程序，也不「选中即复制」。松开时才打开（按下就开的话浏览器抢走前台，
        // 这次松开收不到）；按下后拖开了不算点击
        if event.button == MouseButton::Left
            && let Some(pressed) = self.link_press.take()
        {
            let moved = event.position - pressed;
            if moved.x.abs() <= px(4.) && moved.y.abs() <= px(4.) {
                self.open_link_at(event.position, cx);
            }
            return;
        }
        if event.button != MouseButton::Left && !self.reports_mouse(&event.modifiers) {
            return;
        }
        if let Some(button) = Self::button(event.button) {
            let (x, y) = self.mouse_position(event.position);
            self.input(Input::Mouse(MouseInput { action: MouseAction::Release(button), x, y, mods: Self::mods(&event.modifiers) }));
        }
        // 选中即复制：松开时选区已定（终端线程按顺序处理，复制排在松开之后）
        if event.button == MouseButton::Left && !self.reports_mouse(&event.modifiers) && crate::settings::get(cx).copy_on_select {
            self.copy_selection(cx);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.pointer = Some(event.position);
        self.cmd_down = event.modifiers.platform;
        self.update_hover_link(cx);
        // 拖到窗口外或取消了拖动：收起「松开上传」提示
        if self.drop_hover && !cx.has_active_drag() {
            self.drop_hover = false;
            cx.notify();
        }
        if self.link_press.is_some() {
            return;
        }
        if !self.reports_mouse(&event.modifiers) && event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (x, y) = self.mouse_position(event.position);
        let pressed = event.pressed_button.and_then(Self::button);
        self.handle.send(Input::Mouse(MouseInput {
            action: MouseAction::Motion { pressed },
            x,
            y,
            mods: Self::mods(&event.modifiers),
        }));
    }

    fn scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = self.metrics.as_ref().map_or(px(20.), |m| m.line_height);
        let delta = event.delta.pixel_delta(line_height).y;
        self.scroll_remainder += f32::from(delta) / f32::from(line_height);
        let lines = self.scroll_remainder.trunc() as isize;
        if lines == 0 {
            return;
        }
        self.scroll_remainder -= lines as f32;
        if self.reports_mouse(&event.modifiers) {
            // 滚轮在鼠标上报里是 4（上）/ 5（下）号键
            let (x, y) = self.mouse_position(event.position);
            let button = if lines > 0 { mouse::Button::Four } else { mouse::Button::Five };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.handle.send(Input::Mouse(MouseInput { action: MouseAction::Press(button), x, y, mods: Self::mods(&event.modifiers) }));
            }
        } else if self.frame.alternate_screen {
            // 备用屏（less、man、没开鼠标的全屏程序）没有回滚：滚轮当方向键
            let key = if lines > 0 { key::Key::ArrowUp } else { key::Key::ArrowDown };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.handle.send(Input::Key(engine::KeyInput {
                    key,
                    mods: key::Mods::empty(),
                    consumed_mods: key::Mods::empty(),
                    text: None,
                    unshifted: '\0',
                    repeat: false,
                }));
            }
        } else {
            self.handle.send(Input::Scroll(-lines));
        }
        cx.notify();
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = Metrics::measure(window, cx);
        if self.metrics.as_ref() != Some(&metrics) {
            self.metrics = Some(metrics.clone());
        }
        let focused = self.focus.is_focused(window) && window.is_window_active();
        self.focused = focused;
        let connected = matches!(self.status, Status::Connected { .. });
        let local = self.status == Status::Local;
        let terminal = div()
            .id("terminal")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .map(|d| {
                #[cfg(any(debug_assertions, feature = "devhook"))]
                let d = d.on_action(cx.listener(Self::log_selection));
                d
            })
            .on_key_down(cx.listener(Self::key_down))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            // 移出终端（到侧栏、搜索条上）：之后按 ⌘ 不再按旧位置找链接
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered {
                    this.pointer = None;
                    this.update_hover_link(cx);
                }
            }))
            .when(self.hover_link.is_some(), |d| d.cursor_pointer())
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll(cx.listener(Self::scroll))
            // 本机：从 Finder 拖进来的文件把路径（转义好）填到命令行，同 Terminal / Ghostty
            .when(local, |d| {
                d.on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    let text: Vec<String> = paths.paths().iter().map(|p| shell_escape(&p.to_string_lossy())).collect();
                    this.input(Input::Paste(format!("{} ", text.join(" "))));
                    cx.notify();
                }))
            })
            .when(connected, |d| {
                d.on_drag_move::<ExternalPaths>(cx.listener(|this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                    let inside = event.bounds.contains(&event.event.position);
                    if this.drop_hover != inside {
                        this.drop_hover = inside;
                        cx.notify();
                    }
                }))
                .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    this.drop_hover = false;
                    let paths = paths.paths().iter().map(|p| p.to_string_lossy().to_string()).collect();
                    cx.emit(TerminalEvent::FilesDropped(paths));
                    cx.notify();
                }))
            })
            .child(TerminalElement {
                view: cx.entity(),
                frame: self.frame.clone(),
                metrics,
                focused,
                cursor_on: self.blink_on || !focused,
                marked: self.marked.clone(),
                link: self.hover_link.clone(),
            })
            .when(self.drop_hover, |d| d.child(drop_hint()));
        div().relative().size_full().child(terminal).when(self.search_open, |d| d.child(self.search_bar(cx)))
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(&mut self, _: Range<usize>, _: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if !text.is_empty() && !self.is_disconnected() {
            self.input(Input::Text(text.to_string()));
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // 输入法候选框跟着光标
        let metrics = self.metrics.as_ref()?;
        let cursor = self.frame.cursor?;
        let origin = self.grid_origin
            + point(metrics.cell_width * f32::from(cursor.x), metrics.line_height * f32::from(cursor.y));
        Some(Bounds::new(origin, size(metrics.cell_width, metrics.line_height)))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

/// 搜索条里输入框右边那句：没敲词不显示；「第几个/共几个」（1 是最新的）；还没找到时区分在搜和没有。
fn search_label(needle: &str, info: Option<&SearchInfo>) -> String {
    let Some(info) = info.filter(|_| !needle.is_empty()) else { return String::new() };
    match (info.total, info.selected) {
        (0, _) if info.searching => "查找中…".into(),
        (0, _) => "无结果".into(),
        (total, Some(index)) => format!("{}/{total}", index + 1),
        (total, None) => format!("{total} 个"),
    }
}

/// 这一格在不在搜索匹配里（`spans` 是这一行的，按列排好）。
fn highlighted(spans: &[MatchSpan], x: usize) -> bool {
    spans.iter().any(|span| usize::from(span.start) <= x && x < usize::from(span.end))
}

/// 网格离卡片左 / 上边的距离（逻辑像素）：固定内边距 + 排不下整格的余量里放在这一边的份额（`share`，0–1），
/// 按设备像素取整（不足一个设备像素的留在另一边）。
fn grid_offset(available: f32, cells: u16, cell: f32, scale: f32, share: f32) -> f32 {
    let leftover = (available - TERMINAL_PADDING * 2. - cell * f32::from(cells)).max(0.);
    TERMINAL_PADDING + (leftover * share * scale).floor() / scale
}

/// 用默认程序打开链接（只开 `openable` 认的那几种）。
fn open_link(url: &str, cx: &mut App) {
    if !openable(url) {
        return;
    }
    tracing::info!("open link: {url}");
    // 测试口开着（无头测试）：只记日志，不去抢用户的浏览器
    #[cfg(any(debug_assertions, feature = "devhook"))]
    if std::env::var_os("SSHL_DEV_SOCKET").is_some() {
        return;
    }
    cx.open_url(url);
}

/// 窗口坐标落在网格的哪一格（列, 行）：`origin` 是网格左上角，`cell` 是一格的大小；在网格外（内边距、余量里）是 None。
fn grid_cell(position: Point<Pixels>, origin: Point<Pixels>, cell: gpui::Size<Pixels>, grid: GridSize) -> Option<(usize, usize)> {
    let x = (position.x - origin.x) / cell.width;
    let y = (position.y - origin.y) / cell.height;
    if !(x >= 0. && y >= 0.) {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    (x < usize::from(grid.cols) && y < usize::from(grid.rows)).then_some((x, y))
}

/// 拖进本机终端的路径：shell 特殊字符前加反斜杠（中文等非 ASCII 原样保留）。
fn shell_escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if !(c.is_ascii_alphanumeric() || !c.is_ascii() || "/._-+,:@%=^~".contains(c)) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn hsla(c: Rgb) -> Hsla {
    theme::color(c.to_u32())
}

struct TerminalElement {
    view: Entity<TerminalView>,
    frame: Arc<Frame>,
    metrics: Metrics,
    focused: bool,
    cursor_on: bool,
    marked: Option<String>,
    /// 按住 ⌘ 时鼠标下的网址：每段格子底下画下划线。
    link: Option<Link>,
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    /// 卡片（铺满整块区域）和网格左上角。
    type PrepaintState = (Bounds<Pixels>, Point<Pixels>);

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut Window, cx: &mut App) -> (Bounds<Pixels>, Point<Pixels>) {
        let grid = self.metrics.grid(bounds.size);
        let m = &self.metrics;
        // 卡片铺满（同 Ghostty），四边 4px 内边距。排不下一整行的余量全放在上面：最后一行（nvim 状态栏）下面正好只有 4px。
        // 余量放底下的话状态栏下面那条边时宽时窄；上面那条是终端底色，和第一行连成一片看不出来。
        // 左右的余量平分（行的底色左右本来就铺到边）
        let origin = bounds.origin
            + point(
                px(grid_offset(f32::from(bounds.size.width), grid.cols, f32::from(m.cell_width), m.scale, 0.5)),
                px(grid_offset(f32::from(bounds.size.height), grid.rows, f32::from(m.line_height), m.scale, 1.)),
            );
        self.view.update(cx, |view, _| view.resize(grid, origin));
        (bounds, origin)
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), state: &mut (Bounds<Pixels>, Point<Pixels>), window: &mut Window, cx: &mut App) {
        let (card, origin) = *state;
        let frame = self.frame.clone();
        let m = &self.metrics;
        let pad = px(TERMINAL_PADDING);
        // 终端贴满内容区（同 Ghostty）：直角、不画阴影；窗口的圆角由系统裁
        let radius = Corners::all(px(0.));
        let padding_color = if frame.rows.is_empty() { theme::term::BACKGROUND } else { frame.padding };

        window.paint_quad(quad(card, radius, hsla(padding_color), px(0.), gpui::transparent_black(), BorderStyle::default()));

        let grid_size = size(m.cell_width * frame.cols as f32, m.line_height * frame.rows.len() as f32);
        let grid = Bounds::new(origin, grid_size);
        let cell_x = |x: usize| origin.x + m.cell_width * x as f32;
        let cell_y = |y: usize| origin.y + m.line_height * y as f32;

        // 内边距延伸：边缘整行有底色时铺进内边距。上边只铺固定的内边距那么高（再往上是余量，用卡片底色，
        // 整行往上拉长看着像多了一行）；左右照旧铺到边（cursorline 之类贴边才自然）。下边那 4px 不铺最后一行的颜色，
        // 留卡片的底色：铺下去的话 nvim 的状态栏比别的行高出一截（用户嫌多出 4px），不留又贴着窗口底边太挤
        let ext = &frame.extension;
        if let Some(colors) = ext.top.as_deref().filter(|colors| !colors.is_empty()) {
            // 同色的相邻格子合成一段；两头的铺到卡片边
            let top = origin.y - pad;
            let mut start = 0;
            for x in 1..=colors.len() {
                if x < colors.len() && colors[x] == colors[start] {
                    continue;
                }
                let left = if start == 0 { card.origin.x } else { cell_x(start) };
                let right = if x == colors.len() { card.right() } else { cell_x(x) };
                window.paint_quad(fill(Bounds::from_corners(point(left, top), point(right, top + pad)), hsla(colors[start])));
                start = x;
            }
        }
        for y in 0..frame.rows.len() {
            let row = Bounds::from_corners(point(card.origin.x, cell_y(y)), point(origin.x, cell_y(y + 1)));
            if let Some(Some(left)) = ext.left.get(y) {
                window.paint_quad(fill(row, hsla(*left)));
            }
            if let Some(Some(right)) = ext.right.get(y) {
                let b = Bounds::from_corners(point(grid.right(), cell_y(y)), point(card.right(), cell_y(y + 1)));
                window.paint_quad(fill(b, hsla(*right)));
            }
        }

        // 网格底色：卡片取的是边缘色，默认底色的格子要单独铺回终端默认底色
        if padding_color != frame.background {
            window.paint_quad(fill(grid, hsla(frame.background)));
        }
        for (y, row) in frame.rows.iter().enumerate() {
            let mut x = 0;
            while x < row.cells.len() {
                let Some(bg) = row.cells[x].bg else {
                    x += 1;
                    continue;
                };
                let start = x;
                while x < row.cells.len() && row.cells[x].bg == Some(bg) {
                    x += 1;
                }
                let b = Bounds::from_corners(point(cell_x(start), cell_y(y)), point(cell_x(x), cell_y(y + 1)));
                window.paint_quad(fill(b, hsla(bg)));
            }
        }

        // 搜索匹配：盖在格子底色上面，字改成黑的
        let spans: &[MatchSpan] = frame.search.as_ref().map_or(&[], |search| &search.spans);
        for span in spans {
            let y = usize::from(span.y);
            let b = Bounds::from_corners(point(cell_x(span.start.into()), cell_y(y)), point(cell_x(span.end.into()), cell_y(y + 1)));
            let color = if span.selected { theme::term::SEARCH_SELECTED_BG } else { theme::term::SEARCH_BG };
            window.paint_quad(fill(b, hsla(color)));
        }

        // 文字：同样式的相邻格子合成一段，按格宽强制排版，回退字体也对齐网格
        let fonts = Fonts::new(&crate::settings::get(cx).font_family);
        let mut span_at = 0;
        for (y, row) in frame.rows.iter().enumerate() {
            // 这一行的匹配（spans 按行、列排好）
            let from = span_at;
            while span_at < spans.len() && usize::from(spans[span_at].y) <= y {
                span_at += 1;
            }
            let row_spans = &spans[from..span_at];
            let cells = &row.cells;
            let mut x = 0;
            while x < cells.len() {
                let cell = &cells[x];
                let decorated = cell.flags & (flags::UNDERLINE | flags::STRIKE) != 0;
                if cell.has(flags::SPACER) || cell.has(flags::INVISIBLE) || (cell.ch == ' ' && cell.extra.is_none() && !decorated) {
                    x += 1;
                    continue;
                }
                let start = x;
                let matched = highlighted(row_spans, start);
                let mut text = String::new();
                let force_width;
                if cell.has(flags::WIDE) || cell.extra.is_some() {
                    text.push(cell.ch);
                    if let Some(extra) = &cell.extra {
                        text.push_str(extra);
                    }
                    force_width = if cell.has(flags::WIDE) { m.cell_width * 2. } else { m.cell_width };
                    x += 1;
                } else {
                    while x < cells.len() {
                        let next = &cells[x];
                        if next.flags & (flags::WIDE | flags::SPACER | flags::INVISIBLE) != 0
                            || next.extra.is_some()
                            || !same_style(cell, next)
                            || highlighted(row_spans, x) != matched
                        {
                            break;
                        }
                        text.push(next.ch);
                        x += 1;
                    }
                    force_width = m.cell_width;
                }
                let mut run = text_run(cell, text.len(), &fonts);
                if matched {
                    run.color = hsla(theme::term::SEARCH_FG);
                }
                let line = window.text_system().shape_line(SharedString::from(text), m.font_size, &[run], Some(force_width));
                let _ = line.paint(point(cell_x(start), cell_y(y)), m.line_height, TextAlign::Left, None, window, cx);
            }
        }

        // ⌘ 悬停的链接：每段格子底边画 1px 的线，颜色同那段第一格的字
        for span in self.link.iter().flat_map(|link| &link.spans) {
            let y = usize::from(span.y);
            let Some(cell) = frame.rows.get(y).and_then(|row| row.cells.get(usize::from(span.start))) else { continue };
            let bottom = cell_y(y + 1);
            let b = Bounds::from_corners(point(cell_x(span.start.into()), bottom - px(1.)), point(cell_x(span.end.into()), bottom));
            window.paint_quad(fill(b, hsla(cell.fg)));
        }

        // 光标
        if let Some(cursor) = frame.cursor {
            let (cx0, cy0) = (cell_x(usize::from(cursor.x)), cell_y(usize::from(cursor.y)));
            let wide = frame
                .rows
                .get(usize::from(cursor.y))
                .and_then(|row| row.cells.get(usize::from(cursor.x)))
                .is_some_and(|cell| cell.has(flags::WIDE));
            let width = if wide { m.cell_width * 2. } else { m.cell_width };
            let cell_bounds = Bounds::new(point(cx0, cy0), size(width, m.line_height));
            let color = hsla(cursor.color);
            if !self.focused {
                window.paint_quad(fill(Bounds::new(point(cx0, cy0), size(px(1.), m.line_height)), color));
            } else if self.cursor_on || !cursor.blinking {
                use super::frame::CursorShape;
                match cursor.shape {
                    CursorShape::Bar => window.paint_quad(fill(Bounds::new(point(cx0, cy0), size(px(1.), m.line_height)), color)),
                    CursorShape::Underline => window.paint_quad(fill(
                        Bounds::new(point(cx0, cy0 + m.line_height - px(2.)), size(width, px(2.))),
                        color,
                    )),
                    CursorShape::BlockHollow => window.paint_quad(quad(cell_bounds, px(0.), gpui::transparent_black(), px(1.), color, BorderStyle::default())),
                    CursorShape::Block => {
                        window.paint_quad(fill(cell_bounds, color));
                        if let Some(cell) = frame.rows.get(usize::from(cursor.y)).and_then(|row| row.cells.get(usize::from(cursor.x)))
                            && (cell.ch != ' ' || cell.extra.is_some()) {
                                let mut text = cell.ch.to_string();
                                if let Some(extra) = &cell.extra {
                                    text.push_str(extra);
                                }
                                let mut run = text_run(cell, text.len(), &fonts);
                                run.color = hsla(cursor.text_color);
                                let line = window.text_system().shape_line(text.into(), m.font_size, &[run], Some(width));
                                let _ = line.paint(point(cx0, cy0), m.line_height, TextAlign::Left, None, window, cx);
                            }
                    }
                }
            }

            // 输入法组字：画在光标处，带下划线
            if let Some(marked) = &self.marked {
                let run = TextRun {
                    len: marked.len(),
                    font: fonts.regular.clone(),
                    color: hsla(theme::term::FOREGROUND),
                    background_color: Some(hsla(frame.background)),
                    underline: Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(marked.clone().into(), m.font_size, &[run], None);
                let _ = line.paint(point(cx0, cy0), m.line_height, TextAlign::Left, None, window, cx);
            }
        }

        let focus = self.view.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
    }
}

struct Fonts {
    regular: Font,
    bold: Font,
    italic: Font,
    bold_italic: Font,
}

impl Fonts {
    fn new(family: &str) -> Self {
        let regular = Font { fallbacks: Some(FontFallbacks::from_fonts(vec![crate::settings::DEFAULT_FONT_FAMILY.into(), "Apple Color Emoji".into()])), ..font(family.to_string()) };
        let bold = Font { weight: FontWeight::BOLD, ..regular.clone() };
        let italic = Font { style: FontStyle::Italic, ..regular.clone() };
        let bold_italic = Font { style: FontStyle::Italic, ..bold.clone() };
        Self { regular, bold, italic, bold_italic }
    }

    fn pick(&self, cell_flags: u16) -> Font {
        match (cell_flags & flags::BOLD != 0, cell_flags & flags::ITALIC != 0) {
            (false, false) => self.regular.clone(),
            (true, false) => self.bold.clone(),
            (false, true) => self.italic.clone(),
            (true, true) => self.bold_italic.clone(),
        }
    }
}

const STYLE_FLAGS: u16 = flags::BOLD
    | flags::ITALIC
    | flags::FAINT
    | flags::UNDERLINE
    | flags::UNDERLINE_DOUBLE
    | flags::UNDERLINE_CURLY
    | flags::STRIKE
    | flags::OVERLINE;

fn same_style(a: &super::frame::Cell, b: &super::frame::Cell) -> bool {
    a.fg == b.fg && (a.flags & STYLE_FLAGS) == (b.flags & STYLE_FLAGS)
}

fn text_run(cell: &super::frame::Cell, len: usize, fonts: &Fonts) -> TextRun {
    let mut color = hsla(cell.fg);
    if cell.has(flags::FAINT) {
        color.a = 0.5;
    }
    TextRun {
        len,
        font: fonts.pick(cell.flags),
        color,
        background_color: None,
        underline: cell.has(flags::UNDERLINE).then(|| UnderlineStyle {
            thickness: px(1.),
            color: Some(color),
            wavy: cell.has(flags::UNDERLINE_CURLY),
        }),
        strikethrough: cell.has(flags::STRIKE).then(|| StrikethroughStyle { thickness: px(1.), color: Some(color) }),
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyContext, Keymap, Keystroke};

    use super::*;

    gpui::actions!(test, [FocusNext]);

    fn resolve(keymap: &Keymap, key: &'static str) -> usize {
        let stack: Vec<KeyContext> = ["Root", "Workspace", CONTEXT].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack).0.len()
    }

    #[test]
    fn host_key_prompt_shows_the_full_fingerprint_and_warns_on_change() {
        let question = HostKeyQuestion {
            host: "srv.example".into(),
            port: 2200,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:nYEchQnrpBDtWk7pRcxxfXQN63cacfPSE6hRM1VmckY".into(),
            changed: false,
        };
        let first = host_key_options(&question);
        assert!(first.title.contains("srv.example:2200"));
        assert!(first.message.as_ref().unwrap().contains(&question.fingerprint));
        assert!(!first.destructive);
        let changed = host_key_options(&HostKeyQuestion { port: 22, changed: true, ..question.clone() });
        assert!(changed.title.contains("srv.example") && !changed.title.contains(":22"));
        assert!(changed.message.as_ref().unwrap().contains(&question.fingerprint));
        assert!(changed.destructive, "密钥变了用红色按钮");
    }

    #[test]
    fn tab_reaches_the_terminal_instead_of_moving_focus() {
        // 同 gpui-base Root：tab / shift-tab 切换焦点
        let mut keymap = Keymap::new(vec![
            KeyBinding::new("tab", FocusNext, Some("Root")),
            KeyBinding::new("shift-tab", FocusNext, Some("Root")),
        ]);
        keymap.add_bindings(key_bindings());
        assert_eq!(resolve(&keymap, "tab"), 0);
        assert_eq!(resolve(&keymap, "shift-tab"), 0);
        assert_eq!(resolve(&keymap, "cmd-c"), 1);
    }

    #[test]
    fn zoom_keys_change_the_font_size_in_the_terminal() {
        let mut keymap = Keymap::new(Vec::new());
        keymap.add_bindings(key_bindings());
        let stack: Vec<KeyContext> = ["Root", "Workspace", CONTEXT].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let first = |key: &'static str| {
            let bindings = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack).0;
            bindings.first().map(|b| b.action().name().to_string())
        };
        assert_eq!(first("cmd-=").as_deref(), Some("terminal::ZoomIn"));
        // 美式键盘的 ⌘⇧=：macOS 报成 cmd-+（Shift 已算进字符里）
        assert_eq!(first("cmd-+").as_deref(), Some("terminal::ZoomIn"));
        assert_eq!(first("cmd--").as_deref(), Some("terminal::ZoomOut"));
        assert_eq!(first("cmd-0").as_deref(), Some("terminal::ZoomReset"));
    }

    #[test]
    fn window_positions_map_to_grid_cells() {
        let grid = GridSize { cols: 80, rows: 24, cell_width: 16, cell_height: 36 };
        // 网格左上角在 (10, 20)，一格 8×18
        let at = |x: f32, y: f32| grid_cell(point(px(x), px(y)), point(px(10.), px(20.)), size(px(8.), px(18.)), grid);
        assert_eq!(at(10., 20.), Some((0, 0)));
        assert_eq!(at(17.9, 37.9), Some((0, 0)));
        assert_eq!(at(18., 38.), Some((1, 1)));
        assert_eq!(at(10. + 8. * 79. + 1., 20. + 18. * 23. + 1.), Some((79, 23)), "右下角那一格");
        assert_eq!(at(9.5, 30.), None, "左边内边距");
        assert_eq!(at(20., 19.5), None, "上面的余量");
        assert_eq!(at(10. + 8. * 80., 30.), None, "网格右边");
        assert_eq!(at(20., 20. + 18. * 24.), None, "网格下面");
    }

    #[test]
    fn dropped_paths_are_escaped_for_the_shell_but_keep_non_ascii() {
        assert_eq!(shell_escape("/Users/me/My File (1).txt"), r"/Users/me/My\ File\ \(1\).txt");
        assert_eq!(shell_escape("/tmp/a'b\"$c&d"), r#"/tmp/a\'b\"\$c\&d"#);
        assert_eq!(shell_escape("/Users/me/下载/报告-v2.pdf"), "/Users/me/下载/报告-v2.pdf");
    }

    #[test]
    fn search_label_says_which_match_of_how_many() {
        let info = |total, selected, searching| SearchInfo { total, selected, searching, spans: Vec::new() };
        assert_eq!(search_label("", Some(&info(3, Some(0), false))), "", "没敲词不显示");
        assert_eq!(search_label("x", None), "");
        assert_eq!(search_label("x", Some(&info(17, Some(0), false))), "1/17");
        assert_eq!(search_label("x", Some(&info(17, Some(16), true))), "17/17");
        assert_eq!(search_label("x", Some(&info(0, None, true))), "查找中…");
        assert_eq!(search_label("x", Some(&info(0, None, false))), "无结果");
    }

    #[test]
    fn search_keys_reach_the_search_bar_before_the_input_box() {
        gpui::actions!(test, [InputEnter, InputEscape]);
        let mut keymap = Keymap::new(vec![
            KeyBinding::new("enter", InputEnter, Some("Input")),
            KeyBinding::new("escape", InputEscape, Some("Input")),
        ]);
        keymap.add_bindings(key_bindings());
        let stack: Vec<KeyContext> = ["Root", "Workspace", SEARCH_CONTEXT, "Input"].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let first = |key: &'static str| {
            let bindings = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack).0;
            bindings.first().map(|b| b.action().name().to_string())
        };
        assert_eq!(first("enter").as_deref(), Some("terminal::FindNext"));
        assert_eq!(first("shift-enter").as_deref(), Some("terminal::FindPrev"));
        assert_eq!(first("escape").as_deref(), Some("terminal::FindClose"));
        assert_eq!(first("cmd-g").as_deref(), Some("terminal::FindNext"));
        // 终端里 Esc 照旧发给程序（vim），不关搜索条
        assert_eq!(resolve(&keymap, "escape"), 0);
        assert_eq!(resolve(&keymap, "cmd-f"), 1);
    }

    #[test]
    fn leftover_rows_go_on_top_and_leftover_columns_are_split() {
        // 高 500、行高 18：(500 - 8) / 18 = 27 行，余 6px 全放上面：最后一行下面正好是 4px 内边距
        assert_eq!(grid_offset(500., 27, 18., 2., 1.), 10.);
        // 左右平分；余 7px 时 2 倍屏上能放 3.5px，1 倍屏按整像素取 3px（多的 1px 留在另一边）
        assert_eq!(grid_offset(501., 27, 18., 2., 0.5), 7.5);
        assert_eq!(grid_offset(501., 27, 18., 1., 0.5), 7.);
        // 正好排满：只有内边距
        assert_eq!(grid_offset(8. + 18. * 10., 10, 18., 2., 1.), 4.);
    }

    /// 四边都是 4px 内边距，底边也留：最后一行下面不多不少正好 4px（贴着窗口底边看着怪）。
    #[test]
    fn the_last_row_keeps_the_padding_below_it() {
        let m = Metrics { font_size: px(13.), cell_width: px(8.), line_height: px(18.), scale: 2. };
        for height in [8. + 18. * 10., 8. + 18. * 10. + 7., 500.] {
            let grid = m.grid(size(px(8. + 8. * 80.), px(height)));
            let top = grid_offset(height, grid.rows, 18., 2., 1.);
            assert_eq!(height - (top + 18. * f32::from(grid.rows)), TERMINAL_PADDING, "高 {height}");
        }
        assert_eq!(m.grid(size(px(8. + 8. * 80.), px(4. + 18. * 10.))).rows, 9, "上下各扣 4px：差 4px 排不下第 10 行");
    }
}

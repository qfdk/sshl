//! 终端视图：一张深色圆角卡片，按整行整列贴齐（余下的不足一行 / 一列交给卡片外的浅色背景），
//! 四边 4px 内边距；内边距用边缘格子的底色延伸（全屏 TUI 的状态栏、cursorline 贴到卡片边）。

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::channel::oneshot;
use gpui::{
    App, BorderStyle, Bounds, ClipboardItem, Context, Corners, Element, ElementId, ElementInputHandler,
    DragMoveEvent, Entity, EntityInputHandler, EventEmitter, ExternalPaths, FocusHandle, Focusable, Font, FontFallbacks, FontStyle, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, NoAction, ParentElement, Pixels, Point, Render, ScrollEvent, SharedString,
    StrikethroughStyle, Style, Styled, Task, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
    fill, font, point, prelude::FluentBuilder, px, quad, relative, size,
};
use libghostty_vt::{key, mouse};
use sshl_core::error::AppError;
use sshl_core::known_hosts::{HostKeyCheck, HostKeyPrompt, HostKeyQuestion};
use sshl_core::local_shell::{self, LocalOptions, LocalShell};
use sshl_core::ssh::{self, ConnectionDetails, ShellHandle, TerminalSink};

use super::engine::{self, GridSize, Input, MouseAction, MouseInput, TerminalHandle};
use super::frame::{Frame, Rgb, flags};
use super::keys;
use crate::backend;
use crate::prompts::{self, Options};
use crate::theme::{self, TERMINAL_PADDING};

actions!(terminal, [Copy, Paste, SelectAll]);

const CONTEXT: &str = "Terminal";

/// 终端里的按键绑定。Tab / Shift-Tab 被 gpui-base 的 Root 绑去切换焦点，在终端里压掉，
/// 让它们落到 key_down 编码给远端（补全、反向补全）。
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
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
        // 拖到窗口外或取消了拖动：收起「松开上传」提示
        if self.drop_hover && !cx.has_active_drag() {
            self.drop_hover = false;
            cx.notify();
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
        div()
            .id("terminal")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .map(|d| {
                #[cfg(any(debug_assertions, feature = "devhook"))]
                let d = d.on_action(cx.listener(Self::log_selection));
                d
            })
            .on_key_down(cx.listener(Self::key_down))
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
            })
            .when(self.drop_hover, |d| d.child(drop_hint()))
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

/// 网格离卡片边的距离（逻辑像素）：固定内边距 + 排不下整格的余量的一半，按设备像素取整。
fn grid_offset(available: f32, cells: u16, cell: f32, scale: f32) -> f32 {
    let leftover = (available - TERMINAL_PADDING * 2. - cell * f32::from(cells)).max(0.);
    TERMINAL_PADDING + (leftover / 2. * scale).floor() / scale
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
        // 卡片铺满（同 Ghostty）：排不下一整行 / 一整列的余量上下、左右平分（Ghostty 的 window-padding-balance），
        // 每边最多多出半行，不会全堆在底下、看着像多了一行
        let origin = bounds.origin
            + point(
                px(grid_offset(f32::from(bounds.size.width), grid.cols, f32::from(m.cell_width), m.scale)),
                px(grid_offset(f32::from(bounds.size.height), grid.rows, f32::from(m.line_height), m.scale)),
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

        // 内边距延伸：边缘整行有底色时铺进内边距。上下只铺固定的内边距那么高（再往外是平分的余量，用卡片底色）：
        // 整行往下拉长看着像多了一行（nvim 的状态栏）；左右照旧铺到边（cursorline 之类贴边才自然）
        let ext = &frame.extension;
        let strip = |window: &mut Window, colors: &[Rgb], top: Pixels, height: Pixels, round_top: bool| {
            if colors.is_empty() {
                return;
            }
            let mut start = 0;
            for x in 1..=colors.len() {
                if x < colors.len() && colors[x] == colors[start] {
                    continue;
                }
                let left = if start == 0 { card.origin.x } else { cell_x(start) };
                let right = if x == colors.len() { card.right() } else { cell_x(x) };
                let mut corners = Corners::all(px(0.));
                if start == 0 {
                    if round_top { corners.top_left = radius.top_left } else { corners.bottom_left = radius.bottom_left }
                }
                if x == colors.len() {
                    if round_top { corners.top_right = radius.top_right } else { corners.bottom_right = radius.bottom_right }
                }
                let b = Bounds::from_corners(point(left, top), point(right, top + height));
                window.paint_quad(quad(b, corners, hsla(colors[start]), px(0.), gpui::transparent_black(), BorderStyle::default()));
                start = x;
            }
        };
        if let Some(top) = &ext.top {
            strip(window, top, origin.y - pad, pad, true);
        }
        if let Some(bottom) = &ext.bottom {
            strip(window, bottom, grid.bottom(), pad, false);
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

        // 文字：同样式的相邻格子合成一段，按格宽强制排版，回退字体也对齐网格
        let fonts = Fonts::new(&crate::settings::get(cx).font_family);
        for (y, row) in frame.rows.iter().enumerate() {
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
                        if next.flags & (flags::WIDE | flags::SPACER | flags::INVISIBLE) != 0 || next.extra.is_some() || !same_style(cell, next) {
                            break;
                        }
                        text.push(next.ch);
                        x += 1;
                    }
                    force_width = m.cell_width;
                }
                let run = text_run(cell, text.len(), &fonts);
                let line = window.text_system().shape_line(SharedString::from(text), m.font_size, &[run], Some(force_width));
                let _ = line.paint(point(cell_x(start), cell_y(y)), m.line_height, TextAlign::Left, None, window, cx);
            }
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
    fn dropped_paths_are_escaped_for_the_shell_but_keep_non_ascii() {
        assert_eq!(shell_escape("/Users/me/My File (1).txt"), r"/Users/me/My\ File\ \(1\).txt");
        assert_eq!(shell_escape("/tmp/a'b\"$c&d"), r#"/tmp/a\'b\"\$c\&d"#);
        assert_eq!(shell_escape("/Users/me/下载/报告-v2.pdf"), "/Users/me/下载/报告-v2.pdf");
    }

    #[test]
    fn leftover_space_is_split_between_both_sides() {
        // 高 500、行高 18：(500 - 8) / 18 = 27 行，余 6px → 上下各 3px，加 4px 内边距
        assert_eq!(grid_offset(500., 27, 18., 2.), 7.);
        // 余 7px：2 倍屏上能放 3.5px；1 倍屏按整像素取 3px（多的 1px 留在下边）
        assert_eq!(grid_offset(501., 27, 18., 2.), 7.5);
        assert_eq!(grid_offset(501., 27, 18., 1.), 7.);
        // 正好排满：只有内边距
        assert_eq!(grid_offset(8. + 18. * 10., 10, 18., 2.), 4.);
    }
}

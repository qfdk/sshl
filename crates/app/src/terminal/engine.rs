//! 终端线程：每个会话一个，独占 libghostty-vt 的终端（它的类型都是 !Send）。
//! SSH 输出、键鼠、改尺寸都以消息进来；解析完一批就把屏幕整理成 [`Frame`] 交给界面，并确认流控字节数。
//! 界面线程从不碰终端本身，刷屏时解析和整理帧都不占界面。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libghostty_vt::fmt::{Format, Formatter};
use libghostty_vt::key;
use libghostty_vt::mouse;
use libghostty_vt::selection::gesture::{DragEvent, Gesture, Geometry, PressEvent, ReleaseEvent};
use libghostty_vt::search::{MatchBuffer, Search, Status as SearchStatus};
use libghostty_vt::terminal::{Point, PointCoordinate, PointSpace};
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RowIterator};
use libghostty_vt::screen::{CellWide, Screen};
use libghostty_vt::style::{PaletteIndex, RgbColor, StyleColor, Underline};
use libghostty_vt::terminal::{CursorStyle, Mode, ScrollViewport};
use libghostty_vt::{RenderState, Terminal};
use sshl_core::ssh::ShellHandle;

use super::frame::{self, flags, Cell, Cursor, CursorShape, Frame, Rgb, Row};
use crate::theme::term as colors;

/// 给远端 PTY 的 window_change 最多约 30 次/秒：拖动窗口时本地每帧都重排，远端只跟最后的尺寸。
const PTY_RESIZE_INTERVAL: Duration = Duration::from_millis(33);
/// 每轮最多解析这么多输出就回头处理按键、确认流控。
const OUTPUT_SLICE: usize = 64 * 1024;
/// 刷屏时出帧的最短间隔（约一个显示刷新周期）。
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// 同步输出（DEC 2026）最多压住画面这么久：程序崩了 / 忘了结束时不至于一直卡在旧画面（同 Ghostty）。
const HOLD_TIMEOUT: Duration = Duration::from_secs(1);
/// 搜索回滚历史每轮最多占这么久，然后回头处理按键和输出。
const SEARCH_SLICE: Duration = Duration::from_millis(4);
/// 匹配不超过这么多时，每次刷新都重新核对选中项和「第几个」（要取整张匹配列表）；再多就只在选中、跳转时算。
const NEWEST_EXACT_MAX: usize = 20_000;
/// 回滚历史上限。
const SCROLLBACK_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    /// 一格的像素尺寸（设备像素），用于图片协议、尺寸报告和鼠标坐标换算。
    pub cell_width: u32,
    pub cell_height: u32,
}

pub struct KeyInput {
    pub key: key::Key,
    pub mods: key::Mods,
    pub consumed_mods: key::Mods,
    pub text: Option<String>,
    pub unshifted: char,
    pub repeat: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum MouseAction {
    Press(mouse::Button),
    Release(mouse::Button),
    Motion { pressed: Option<mouse::Button> },
}

#[derive(Clone, Copy, Debug)]
pub struct MouseInput {
    pub action: MouseAction,
    /// 相对网格左上角的设备像素坐标。
    pub x: f32,
    pub y: f32,
    pub mods: key::Mods,
}

pub enum Input {
    Output(Vec<u8>),
    Key(KeyInput),
    /// 已定稿的文字（普通键入、输入法上屏）。
    Text(String),
    Paste(String),
    Resize(GridSize),
    /// 滚动回滚历史若干行（负数向上）。
    Scroll(isize),
    Mouse(MouseInput),
    Focus(bool),
    /// 复制选区（纯文本）；没有选区时回 None。
    Copy(futures::channel::oneshot::Sender<Option<String>>),
    SelectAll,
    /// 搜索（⌘F）：在回滚历史和屏幕里找这个词（ASCII 字母不分大小写）；空串 = 关掉搜索、去掉高亮。
    Search(String),
    /// 选中下一个匹配并滚到它：`older` 往更早的（上面），否则往更新的；到头绕回。
    SearchStep { older: bool },
    /// ⌘ 点击：查那一格的 OSC 8 超链接（x、y 同 [`MouseInput`]，相对网格左上角的设备像素）；没有回 None。
    LinkAt { x: f32, y: f32, reply: futures::channel::oneshot::Sender<Option<String>> },
    /// 断线重连：之后的输入、终端应答、改尺寸、流控确认都写到新的 shell 通道。屏幕和回滚历史不动。
    Rebind(ShellHandle),
    Shutdown,
}

/// 终端线程通知界面的事。
pub enum Event {
    /// 有新帧了（合并：界面取走之前不重复通知）。
    Frame,
    /// 程序响铃（BEL）。
    Bell,
}

struct Shared {
    frame: Mutex<Arc<Frame>>,
    /// 已通知、界面还没取走帧。
    notified: AtomicBool,
}

/// 界面持有的一端。drop 时让终端线程退出。
pub struct TerminalHandle {
    tx: Sender<Input>,
    shared: Arc<Shared>,
}

impl TerminalHandle {
    pub fn send(&self, input: Input) {
        let _ = self.tx.send(input);
    }

    /// 给 SSH 输出用的发送端（在 tokio 线程上调用）。
    pub fn sender(&self) -> Sender<Input> {
        self.tx.clone()
    }

    /// 取最新帧；之后有新帧会再通知。
    pub fn take_frame(&self) -> Arc<Frame> {
        self.shared.notified.store(false, Ordering::Release);
        self.shared.frame.lock().unwrap().clone()
    }
}

impl Drop for TerminalHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Input::Shutdown);
    }
}

/// 起一个终端线程。`notify` 在终端线程上调用，负责把事件转回界面线程。
pub fn spawn(
    size: GridSize,
    shell: ShellHandle,
    notify: impl Fn(Event) + Send + 'static,
) -> TerminalHandle {
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Shared { frame: Mutex::new(Arc::new(Frame::default())), notified: AtomicBool::new(false) });
    let thread_shared = shared.clone();
    std::thread::Builder::new()
        .name("terminal".into())
        .spawn(move || {
            let notify: Box<dyn Fn(Event) + Send> = Box::new(notify);
            match Engine::new(size, shell, thread_shared, notify) {
                Ok(mut engine) => engine.run(rx),
                Err(error) => tracing::error!("terminal engine failed to start: {error}"),
            }
        })
        .expect("spawn terminal thread");
    TerminalHandle { tx, shared }
}

fn rgb(c: RgbColor) -> Rgb {
    Rgb { r: c.r, g: c.g, b: c.b }
}

fn ghostty(c: Rgb) -> RgbColor {
    RgbColor { r: c.r, g: c.g, b: c.b }
}

/// 进行中的搜索。libghostty 自己跟着屏幕变化、换屏、改尺寸更新结果，这里只负责分片推进和取结果。
struct SearchDriver {
    search: Search<'static, 'static>,
    storage: MatchBuffer,
    /// 换了搜索词、还没选中任何一个：一有结果就选中最新的那个（同浏览器的查找）。
    autoselect: bool,
    /// 当前选中的匹配：它第一格的位置（整个屏幕含回滚历史里的行、列）。选中由我们自己记，不用 libghostty 的
    /// select_next / select_prev——它的列表顺序在屏幕跨内部页时是乱的（见 `frame::newest_first_segments`），而且只能一步步走。
    selected: Option<(u32, u16)>,
    /// 它是从新到旧第几个。
    rank: Option<usize>,
}

/// 选中项怎么动。
#[derive(Clone, Copy, PartialEq)]
enum Select {
    /// 不动，只核对位置和编号（内容变了、改了尺寸之后）。
    Keep,
    /// 最新的那个。
    Newest,
    /// 往更早 / 更新的走一个，到头绕回。
    Step { older: bool },
}

struct Engine {
    /// 写在 term 前面：先于终端释放。
    search: Option<SearchDriver>,
    term: Terminal<'static, 'static>,
    /// 整理帧的那部分。同步输出开始时 libghostty 的回调要在 vt_write 中途拼帧，所以单独拿出来共用。
    painter: Rc<RefCell<Painter>>,
    key_encoder: key::Encoder<'static>,
    key_event: key::Event<'static>,
    mouse_encoder: mouse::Encoder<'static>,
    mouse_event: mouse::Event<'static>,
    selection: SelectionGesture,
    shell: Arc<Mutex<ShellHandle>>,
    notify: Rc<dyn Fn(Event) + Send>,
    size: GridSize,
    /// 解析完、还没确认给 SSH 流控的字节数。
    unacked: u32,
    pty_size: (u16, u16),
    pty_sent_at: Option<Instant>,
    pty_pending: bool,
    buf: Vec<u8>,
    focused: bool,
    blink: BlinkPolicy,
}

/// 把终端整理成 [`Frame`] 交给界面。
///
/// 同步输出（DEC 2026，Claude Code / Ink、neovim 等重画前后各发一次）：程序开始一段「hold」时，
/// 终端里正好是它想留在屏幕上的那一帧——就在 libghostty 的回调里（vt_write 中途，hold 之后的字节还没解析）
/// 把它拼好；hold 期间界面一直显示这一帧，hold 结束再按最新的出帧。不这样做的话清屏、逐行重写的中间状态
/// 会被画出来，整块界面闪。结束一段马上开始下一段时，中间那个完整的帧也是在回调里留下的，不会丢。
struct Painter {
    render: RenderState<'static>,
    row_it: RowIterator<'static>,
    cell_it: CellIterator<'static>,
    rows: Vec<Arc<Row>>,
    text: String,
    /// 最近拼好的帧；`unsent` = 界面还没拿到。
    built: Arc<Frame>,
    unsent: bool,
    /// 当前 hold 开始的时刻；None = 没有 hold。
    held_since: Option<Instant>,
    /// 搜索状态（终端线程每轮算好放这儿），拼帧时带上。
    search: Option<Arc<frame::SearchInfo>>,
    shared: Arc<Shared>,
    notify: Rc<dyn Fn(Event) + Send>,
}

/// 光标闪不闪只听 DECSCUSR（`CSI Ps SP q`）：0 回到默认（闪），1 / 3 / 5 闪，2 / 4 / 6 不闪。
/// DEC 12 不认：vim / htop / less 退出时发的 cnorm（xterm terminfo 是 `\e[?12l\e[?25h`）会用它关掉闪烁，
/// 之后光标就一直不闪了。libghostty 里两者是同一个模式位，所以自己记下 DECSCUSR，每片输出之后把模式位设回去。
/// 状态跨输出切片保留，序列被切成两片也认得出。
#[derive(Default)]
struct BlinkPolicy {
    scan: Scan,
    param: u16,
    /// 最后一次 DECSCUSR 要的；None = 没设过或 Ps 0，用默认（闪）。
    requested: Option<bool>,
}

#[derive(Default, Clone, Copy)]
enum Scan {
    #[default]
    Ground,
    Esc,
    Csi,
    CsiSpace,
}

impl BlinkPolicy {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.scan = match (self.scan, byte) {
                (_, 0x1b) => Scan::Esc,
                (Scan::Esc, b'[') => {
                    self.param = 0;
                    Scan::Csi
                }
                (Scan::Csi, b'0'..=b'9') => {
                    self.param = self.param.saturating_mul(10).saturating_add(u16::from(byte - b'0'));
                    Scan::Csi
                }
                (Scan::Csi, b' ') => Scan::CsiSpace,
                (Scan::CsiSpace, b'q') => {
                    match self.param {
                        0 => self.requested = None,
                        1 | 3 | 5 => self.requested = Some(true),
                        2 | 4 | 6 => self.requested = Some(false),
                        _ => {}
                    }
                    Scan::Ground
                }
                _ => Scan::Ground,
            };
        }
    }

    fn blinking(&self) -> bool {
        self.requested.unwrap_or(true)
    }
}

impl Engine {
    fn new(
        size: GridSize,
        shell: ShellHandle,
        shared: Arc<Shared>,
        notify: Box<dyn Fn(Event) + Send>,
    ) -> libghostty_vt::error::Result<Self> {
        let mut term = Terminal::new(size.cols.max(1), size.rows.max(1))?;
        term.resize(size.cols.max(1), size.rows.max(1), size.cell_width, size.cell_height)?;
        term.set_scrollback_max_bytes(Some(SCROLLBACK_BYTES))?;
        term.set_default_fg_color(Some(ghostty(colors::FOREGROUND)))?;
        term.set_default_bg_color(Some(ghostty(colors::BACKGROUND)))?;
        term.set_default_cursor_color(Some(ghostty(colors::CURSOR)))?;
        // 默认光标：闪烁竖线。程序发 DECSCUSR 0（nvim 退出时）就回到这里。
        term.set_default_cursor_style(Some(CursorStyle::Bar))?;
        term.set_default_cursor_blink(Some(true))?;
        let mut palette = term.default_color_palette()?;
        for (i, color) in colors::ANSI.iter().enumerate() {
            palette.set(PaletteIndex(i as _), ghostty(*color));
        }
        term.set_default_color_palette(Some(palette))?;
        // 终端应答（光标位置报告等）也走可替换的那条通道：断线重连后写到新连接
        let shell = Arc::new(Mutex::new(shell));
        let reply = shell.clone();
        term.on_pty_write(move |_, data| reply.lock().unwrap().write(data.to_vec()))?;

        let notify: Rc<dyn Fn(Event) + Send> = Rc::from(notify);
        let painter = Rc::new(RefCell::new(Painter {
            render: RenderState::new()?,
            row_it: RowIterator::new()?,
            cell_it: CellIterator::new()?,
            rows: Vec::new(),
            text: String::with_capacity(8),
            built: Arc::new(Frame::default()),
            unsent: false,
            held_since: None,
            search: None,
            shared,
            notify: notify.clone(),
        }));
        let hold = painter.clone();
        term.on_render_hold(move |term, held| {
            // 回调在 vt_write / resize / reset 里面跑：那时不会有别处借着 painter，借不到就当没 hold（照常出帧）
            let Ok(mut painter) = hold.try_borrow_mut() else { return };
            if held {
                if let Err(error) = painter.build(term) {
                    tracing::warn!("terminal frame failed: {error}");
                }
                painter.held_since = Some(Instant::now());
            } else {
                painter.held_since = None;
            }
        })?;

        Ok(Self {
            search: None,
            term,
            painter,
            key_encoder: key::Encoder::new()?,
            key_event: key::Event::new()?,
            mouse_encoder: mouse::Encoder::new()?,
            mouse_event: mouse::Event::new()?,
            selection: SelectionGesture::new()?,
            shell,
            notify,
            size,
            unacked: 0,
            pty_size: (size.cols, size.rows),
            pty_sent_at: None,
            pty_pending: false,
            buf: Vec::with_capacity(64),
            focused: true,
            blink: BlinkPolicy::default(),
        })
    }

    /// 远端输出交给 libghostty 解析；光标闪烁按 `BlinkPolicy` 改回来（不认 DEC 12）。
    fn write_output(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
        self.blink.feed(bytes);
        let _ = self.term.set_mode(Mode::CURSOR_BLINKING, self.blink.blinking());
    }

    fn run(&mut self, rx: Receiver<Input>) {
        // 还没解析的 SSH 输出（最多一个流控窗口，2MB）
        let mut backlog: VecDeque<Vec<u8>> = VecDeque::new();
        let mut offset = 0usize;
        let mut dirty = false;
        // 回滚历史还没搜完：不阻塞等消息，每轮接着搜一片
        let mut searching = false;
        // 画面变了、搜索结果还没跟上；下次最早什么时候再刷新搜索（按上次的耗时节流）
        let mut search_stale = false;
        let mut search_next = Instant::now();
        let mut bell = false;
        let mut last_frame = Instant::now() - FRAME_INTERVAL;
        loop {
            // 有积压就不阻塞；否则等消息。有待发的 PTY 尺寸、或画面改了但还没到出帧时刻时，最多等到那一刻
            let search_due = (self.search.is_some() && (searching || search_stale)).then_some(search_next);
            let wait = if backlog.is_empty() {
                // hold 期间画面改了也不出帧（只等 hold 结束或超时）；hold 开始时留下的那一帧照常按时交出去
                let (unsent, held_since) = {
                    let painter = self.painter.borrow();
                    (painter.unsent, painter.held_since)
                };
                let frame_due = (unsent || (dirty && held_since.is_none())).then(|| last_frame + FRAME_INTERVAL);
                let hold_due = held_since.map(|since| since + HOLD_TIMEOUT);
                earliest(earliest(earliest(self.pty_deadline(), frame_due), hold_due), search_due)
                    .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            } else {
                Some(Duration::ZERO)
            };
            let first = match wait {
                Some(timeout) => match rx.recv_timeout(timeout) {
                    Ok(input) => Some(input),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(input) => Some(input),
                    Err(_) => return,
                },
            };
            // 按键、改尺寸等控制消息立刻处理（刷屏时 ^C 不排在积压的输出后面），输出进积压
            // 这一轮终端有没有变（按键、改尺寸、新输出）：搜索结果要跟着刷新
            let mut touched = false;
            for input in first.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok())) {
                match input {
                    Input::Shutdown => return,
                    Input::Output(bytes) => backlog.push_back(bytes),
                    // 只读查询不改画面，不为它出帧
                    query @ (Input::Copy(_) | Input::LinkAt { .. }) => self.handle(query),
                    other => {
                        self.handle(other);
                        dirty = true;
                        touched = true;
                    }
                }
            }
            // 解析一片输出，解析完立刻确认给 SSH 流控
            let mut budget = OUTPUT_SLICE;
            while budget > 0 {
                let Some(front) = backlog.front() else { break };
                let take = (front.len() - offset).min(budget);
                let piece = &front[offset..offset + take];
                if piece.contains(&0x07) {
                    bell = true;
                }
                self.write_output(piece);
                self.unacked = self.unacked.saturating_add(take as u32);
                budget -= take;
                offset += take;
                if offset == front.len() {
                    backlog.pop_front();
                    offset = 0;
                }
                dirty = true;
                touched = true;
            }
            if self.unacked > 0 {
                self.shell.lock().unwrap().ack(std::mem::take(&mut self.unacked));
            }
            self.flush_pty_size();
            if self.search.is_none() {
                searching = false;
                search_stale = false;
            } else {
                search_stale |= touched;
                if (searching || search_stale) && Instant::now() >= search_next {
                    let started = Instant::now();
                    let (changed, pending) = self.refresh_search();
                    dirty |= changed;
                    searching = pending;
                    search_stale = false;
                    // 节流：搜索最多占这个线程约两成时间。平时一次几十微秒，等于不限；匹配几十万个时一次要几毫秒，
                    // 刷屏中每帧都刷新会拖慢解析和按键，高亮晚一点跟上没关系
                    search_next = Instant::now() + started.elapsed() * 4;
                }
            }
            // 出帧：两帧至少隔 FRAME_INTERVAL（屏幕刷新一次）。原来积压一清空就立刻出帧，刷屏时数据一小块一小块地到、
            // 每块都清空积压，一秒出上千帧（屏幕只显示 60 帧），多出来的都白算。打字时两次按键间隔远大于它，回显照样立刻出帧
            self.expire_hold(Instant::now());
            let (unsent, held) = {
                let painter = self.painter.borrow();
                (painter.unsent, painter.held_since.is_some())
            };
            if ((dirty && !held) || unsent) && last_frame.elapsed() >= FRAME_INTERVAL {
                if let Err(error) = self.paint() {
                    tracing::warn!("terminal frame failed: {error}");
                }
                last_frame = Instant::now();
                // hold 里的改动留到 hold 结束 / 超时再画
                if !held {
                    dirty = false;
                }
            }
            if bell {
                bell = false;
                (self.notify)(Event::Bell);
            }
        }
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Output(_) | Input::Shutdown => unreachable!("handled in run"),
            Input::Key(key) => {
                self.clear_selection();
                self.key(key)
            }
            Input::Text(text) => {
                self.clear_selection();
                self.scroll_to_bottom();
                self.shell.lock().unwrap().write(text.into_bytes());
            }
            Input::Paste(text) => self.paste(text),
            Input::Resize(size) => self.resize(size),
            Input::Scroll(lines) => self.term.scroll_viewport(ScrollViewport::Delta(lines)),
            Input::Mouse(input) => self.mouse(input),
            Input::Focus(focused) => self.focus(focused),
            Input::Copy(reply) => {
                let _ = reply.send(self.selection_text());
            }
            Input::Search(needle) => self.set_search(&needle),
            Input::SearchStep { older } => self.search_step(older),
            Input::LinkAt { x, y, reply } => {
                let _ = reply.send(self.hyperlink_at(x, y));
            }
            Input::Rebind(shell) => {
                *self.shell.lock().unwrap() = shell;
                // 旧通道没确认完的字节不能记到新通道头上
                self.unacked = 0;
            }
            Input::SelectAll => {
                if let Ok(Some(all)) = self.term.select_all() {
                    let _ = self.term.set_selection(Some(&all));
                }
            }
        }
    }

    fn scroll_to_bottom(&mut self) {
        if !self.term.viewport_active().unwrap_or(true) {
            self.term.scroll_viewport(ScrollViewport::Bottom);
        }
    }

    fn key(&mut self, input: KeyInput) {
        self.key_event
            .set_action(if input.repeat { key::Action::Repeat } else { key::Action::Press })
            .set_key(input.key)
            .set_mods(input.mods)
            .set_consumed_mods(input.consumed_mods)
            .set_unshifted_codepoint(input.unshifted)
            .set_utf8(input.text);
        self.buf.clear();
        let encoded = self
            .key_encoder
            .set_options_from_terminal(&self.term)
            .encode_to_vec(&self.key_event, &mut self.buf);
        if let Err(error) = encoded {
            tracing::warn!("key encode failed: {error}");
            return;
        }
        if !self.buf.is_empty() {
            self.scroll_to_bottom();
            self.shell.lock().unwrap().write(std::mem::take(&mut self.buf));
        }
    }

    fn paste(&mut self, text: String) {
        let bracketed = self.term.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
        let mut data = text.into_bytes();
        let mut out = vec![0u8; data.len() + 16];
        match libghostty_vt::paste::encode(&mut data, bracketed, &mut out) {
            Ok(len) => {
                out.truncate(len);
                self.scroll_to_bottom();
                self.shell.lock().unwrap().write(out);
            }
            Err(error) => tracing::warn!("paste encode failed: {error}"),
        }
    }

    fn resize(&mut self, size: GridSize) {
        if size == self.size || size.cols == 0 || size.rows == 0 {
            return;
        }
        self.size = size;
        if let Err(error) = self.term.resize(size.cols, size.rows, size.cell_width, size.cell_height) {
            tracing::warn!("terminal resize failed: {error}");
        }
        if (size.cols, size.rows) != self.pty_size {
            self.pty_size = (size.cols, size.rows);
            self.pty_pending = true;
        }
    }

    fn pty_deadline(&self) -> Option<Instant> {
        if !self.pty_pending {
            return None;
        }
        Some(self.pty_sent_at.map_or_else(Instant::now, |at| at + PTY_RESIZE_INTERVAL))
    }

    fn flush_pty_size(&mut self) {
        let Some(deadline) = self.pty_deadline() else { return };
        if Instant::now() < deadline {
            return;
        }
        self.pty_pending = false;
        self.pty_sent_at = Some(Instant::now());
        self.shell.lock().unwrap().resize(u32::from(self.pty_size.0), u32::from(self.pty_size.1));
    }

    fn mouse(&mut self, input: MouseInput) {
        // 程序开了鼠标上报且没按 Shift：交给程序；否则左键做本地选区
        let reporting = self.term.is_mouse_tracking().unwrap_or(false) && !input.mods.contains(key::Mods::SHIFT);
        if !reporting {
            if let Err(error) = self.select(input) {
                tracing::debug!("selection: {error}");
            }
            return;
        }
        let pressed = match input.action {
            MouseAction::Press(_) => true,
            MouseAction::Motion { pressed } => pressed.is_some(),
            MouseAction::Release(_) => false,
        };
        let size = mouse::EncoderSize {
            screen_width: u32::from(self.size.cols) * self.size.cell_width,
            screen_height: u32::from(self.size.rows) * self.size.cell_height,
            cell_width: self.size.cell_width.max(1),
            cell_height: self.size.cell_height.max(1),
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        };
        let (action, button) = match input.action {
            MouseAction::Press(button) => (mouse::Action::Press, Some(button)),
            MouseAction::Release(button) => (mouse::Action::Release, Some(button)),
            MouseAction::Motion { pressed } => (mouse::Action::Motion, pressed),
        };
        self.mouse_event
            .set_action(action)
            .set_button(button)
            .set_mods(input.mods)
            .set_position(mouse::Position { x: input.x, y: input.y });
        self.buf.clear();
        let encoder = self.mouse_encoder.set_options_from_terminal(&self.term);
        let encoded = encoder
            .set_size(size)
            .map(|encoder| encoder.set_any_button_pressed(pressed).set_track_last_cell(true))
            .and_then(|encoder| encoder.encode_to_vec(&self.mouse_event, &mut self.buf));
        if let Err(error) = encoded {
            tracing::warn!("mouse encode failed: {error}");
            return;
        }
        if !self.buf.is_empty() {
            self.shell.lock().unwrap().write(std::mem::take(&mut self.buf));
        }
    }

    fn viewport_point(&self, x: f32, y: f32) -> Point {
        let cols = u32::from(self.size.cols.max(1));
        let rows = u32::from(self.size.rows.max(1));
        let col = (x.max(0.0) / self.size.cell_width.max(1) as f32) as u32;
        let row = (y.max(0.0) / self.size.cell_height.max(1) as f32) as u32;
        Point::Viewport(PointCoordinate { x: col.min(cols - 1) as u16, y: row.min(rows - 1) })
    }

    fn select(&mut self, input: MouseInput) -> libghostty_vt::error::Result<()> {
        let point = self.viewport_point(input.x, input.y);
        let g = &mut self.selection;
        match input.action {
            MouseAction::Press(mouse::Button::Left) => {
                let grid_ref = self.term.grid_ref(point)?;
                let selection = g
                    .press
                    .set_repeat_distance(f64::from(self.size.cell_width))?
                    .set_time(g.start.elapsed())?
                    .set_position(f64::from(input.x), f64::from(input.y))?
                    .apply(&mut g.gesture, &self.term, grid_ref)?;
                g.pressed = true;
                self.term.set_selection(selection.as_ref())?;
            }
            MouseAction::Motion { pressed: Some(mouse::Button::Left) } if g.pressed => {
                let grid_ref = self.term.grid_ref(point)?;
                let geometry = Geometry {
                    columns: u32::from(self.size.cols.max(1)),
                    cell_width: self.size.cell_width.max(1),
                    padding_left: 0,
                    screen_height: (u32::from(self.size.rows) * self.size.cell_height).max(1),
                };
                // ⌥ 拖动是矩形选择（同 Ghostty / iTerm2）
                let rectangle = input.mods.contains(key::Mods::ALT);
                let selection = g
                    .drag
                    .set_rectangle(rectangle)?
                    .set_position(f64::from(input.x), f64::from(input.y))?
                    .apply(&mut g.gesture, &self.term, grid_ref, geometry)?;
                self.term.set_selection(selection.as_ref())?;
            }
            MouseAction::Release(mouse::Button::Left) if g.pressed => {
                g.pressed = false;
                let grid_ref = self.term.grid_ref(point).ok();
                g.release.apply(&mut g.gesture, &self.term, grid_ref)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 那一格的 OSC 8 超链接（程序用 `ESC ] 8 ;; 网址 ST 文字 ESC ] 8 ;; ST` 写的，文字不一定是网址本身）。
    fn hyperlink_at(&self, x: f32, y: f32) -> Option<String> {
        let grid_ref = self.term.grid_ref(self.viewport_point(x, y)).ok()?;
        let mut buf = vec![0u8; 256];
        let len = match grid_ref.hyperlink_uri(&mut buf) {
            Ok(len) => len,
            // 网址比缓冲区长：按它要的大小再取一次
            Err(libghostty_vt::error::Error::OutOfSpace { required }) => {
                buf.resize(required, 0);
                grid_ref.hyperlink_uri(&mut buf).ok()?
            }
            Err(_) => return None,
        };
        (len > 0).then(|| String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    fn clear_selection(&mut self) {
        if self.term.selection().is_ok_and(|s| s.is_some()) {
            let _ = self.term.set_selection(None);
        }
    }

    fn selection_text(&mut self) -> Option<String> {
        let mut formatter = Formatter::new_selection(&self.term).ok()??;
        formatter.set_format(Format::Plain).ok()?;
        formatter.set_unwrap(true).ok()?;
        formatter.set_trim(true).ok()?;
        let bytes = formatter.format_alloc(None).ok()?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (!text.is_empty()).then_some(text)
    }

    fn focus(&mut self, focused: bool) {
        if focused == self.focused {
            return;
        }
        self.focused = focused;
        if !self.term.mode(Mode::FOCUS_EVENT).unwrap_or(false) {
            return;
        }
        let event = if focused { libghostty_vt::focus::Event::Gained } else { libghostty_vt::focus::Event::Lost };
        let mut out = [0u8; 8];
        if let Ok(len) = event.encode(&mut out) {
            self.shell.lock().unwrap().write(out[..len].to_vec());
        }
    }

    fn set_search(&mut self, needle: &str) {
        if needle.is_empty() {
            self.search = None;
            self.painter.borrow_mut().search = None;
            return;
        }
        if self.search.is_none() {
            match Search::new(&mut self.term) {
                Ok(search) => self.search = Some(SearchDriver { search, storage: MatchBuffer::new(), autoselect: false, selected: None, rank: None }),
                Err(error) => {
                    tracing::warn!("terminal search failed to start: {error}");
                    return;
                }
            }
        }
        let Some(driver) = self.search.as_mut() else { return };
        // 和原来等价的词（只差大小写）结果和选中都留着，不用重选
        let same = driver.search.needle().ok().flatten().is_some_and(|old| old.eq_ignore_ascii_case(needle.as_bytes()));
        if let Err(error) = driver.search.set_needle(&mut self.term, needle.as_bytes()) {
            tracing::warn!("terminal search: {error}");
        }
        if !same {
            driver.selected = None;
            driver.rank = None;
        }
        driver.autoselect = driver.selected.is_none();
    }

    fn search_step(&mut self, older: bool) {
        let Some(driver) = self.search.as_mut() else { return };
        // 词刚敲完、还没来得及选中就按了 ↵：先落在最新的那个上，不从它再走一步
        let select = if std::mem::take(&mut driver.autoselect) && driver.selected.is_none() { Select::Newest } else { Select::Step { older } };
        if let Err(error) = driver.select(&mut self.term, self.size, select) {
            tracing::warn!("terminal search: {error}");
        }
    }

    /// 推进搜索一片（最多 [`SEARCH_SLICE`]）并把结果放进 painter。返回（结果变了，还没搜完）。
    fn refresh_search(&mut self) -> (bool, bool) {
        let Some(driver) = self.search.as_mut() else { return (false, false) };
        match driver.refresh(&mut self.term, self.size) {
            Ok(info) => {
                let searching = info.searching;
                let mut painter = self.painter.borrow_mut();
                let changed = painter.search.as_deref() != Some(&info);
                if changed {
                    painter.search = Some(Arc::new(info));
                }
                (changed, searching)
            }
            Err(error) => {
                tracing::warn!("terminal search: {error}");
                (false, false)
            }
        }
    }

    /// 把屏幕整理成帧交给界面：只重建脏行，没变的行沿用上一帧的 Arc。
    /// 出帧：没有 hold 时按终端此刻的样子拼；hold 里只交 hold 开始时留下的那一帧（还没交的话）。
    fn paint(&mut self) -> libghostty_vt::error::Result<()> {
        let mut painter = self.painter.borrow_mut();
        if painter.held_since.is_none() {
            painter.build(&self.term)?;
        }
        painter.send();
        Ok(())
    }

    /// hold 超过 [`HOLD_TIMEOUT`]：自己把模式关掉，回到正常出帧（libghostty 没有时钟，不会自己结束）。
    fn expire_hold(&mut self, now: Instant) {
        let expired = self.painter.borrow().held_since.is_some_and(|since| now.saturating_duration_since(since) >= HOLD_TIMEOUT);
        if expired {
            let _ = self.term.set_mode(Mode::SYNC_OUTPUT, false);
            self.painter.borrow_mut().held_since = None;
        }
    }
}

impl SearchDriver {
    /// 推进一片并取结果。libghostty 的 feed 在匹配很多时不便宜（22 万个匹配实测 7ms 一次），所以每次只 feed 一回：
    /// 先在已经取到的数据里 tick，再用一份快照（它会 feed）同时算编号、取屏幕上的匹配。
    fn refresh(&mut self, term: &mut Terminal<'static, 'static>, size: GridSize) -> libghostty_vt::error::Result<frame::SearchInfo> {
        let started = Instant::now();
        let mut status = self.search.status()?;
        while status == SearchStatus::Running && started.elapsed() < SEARCH_SLICE {
            status = self.search.tick()?;
        }
        let found = self.search.total_matches()?;
        if self.autoselect && found > 0 {
            self.autoselect = false;
            self.select(term, size, Select::Newest)?;
        } else if self.selected.is_some() && found > 0 && found <= NEWEST_EXACT_MAX {
            // 内容变了（新输出里又有匹配、改了尺寸）：选中项和它的编号跟上。匹配太多时不值得每次都取整张列表，等下次跳转再算
            self.select(term, size, Select::Keep)?;
        }
        let mut spans = Vec::new();
        {
            let snapshot = self.search.snapshot(term)?;
            let term = snapshot.terminal();
            let at = |grid_ref| term.point_from_grid_ref(&grid_ref, PointSpace::Viewport).ok().flatten().map(|p| (p.x, p.y));
            for found in snapshot.viewport_matches(&mut self.storage)? {
                let start = term.point_from_grid_ref(&found.start(), PointSpace::Screen).ok().flatten().map(|p| (p.y, p.x));
                let is_current = self.selected.is_some() && start == self.selected;
                frame::match_spans(at(found.start()), at(found.end()), size.cols, size.rows, is_current, &mut spans);
            }
        }
        spans.sort_by_key(|span| (span.y, span.start));
        // 快照 feed 过了：状态和总数以它之后的为准
        let searching = self.search.status()? != SearchStatus::Complete;
        let total = self.search.total_matches()?;
        if total == 0 && !searching {
            // 确实一个都没有了。还在搜（改尺寸后 libghostty 从头重搜）时选中项先留着，结果回来再对上
            self.autoselect = false;
            self.selected = None;
            self.rank = None;
        }
        let selected = self.rank.filter(|rank| *rank < total);
        // 有结果了但还没选中（刚 feed 出来的）：下一轮接着做
        Ok(frame::SearchInfo { total, selected, searching: searching || (self.autoselect && total > 0), spans })
    }
}

impl SearchDriver {
    /// 定出选中项（位置和「第几个」）；选了新的就把视口滚到看得见它的地方。
    fn select(&mut self, term: &mut Terminal<'static, 'static>, size: GridSize, select: Select) -> libghostty_vt::error::Result<()> {
        let rows = usize::from(size.rows.max(1));
        let active_top = term.total_rows()?.saturating_sub(rows) as u32;
        // 一页大约能放 46000 / 列数 行（80 列约 570 行、180 列约 256 行）；比活动区顶上再早这么多行的肯定是回滚历史
        let page_rows = 60_000 / u32::from(size.cols.max(1)) + 64;
        {
            let snapshot = self.search.snapshot(term)?;
            let term = snapshot.terminal();
            let all: Vec<_> = snapshot.matches(&mut self.storage)?.collect();
            if all.is_empty() {
                return Ok(());
            }
            let position = |i: usize| term.point_from_grid_ref(&all[i].start(), PointSpace::Screen).ok().flatten().map(|p| (p.y, p.x));
            let segments = frame::newest_first_segments(all.len(), position, |(y, _)| y + page_rows < active_top);
            let at = |rank: usize| frame::segment_index(&segments, rank).and_then(position);
            // 选中项现在排第几：第一个不比它新的（它那里已经不是匹配了——被刷掉、改尺寸重排——就落在挨着的更早那个上）
            let current = self.selected.map(|target| {
                let (mut low, mut high) = (0, all.len());
                while low < high {
                    let middle = (low + high) / 2;
                    if at(middle) > Some(target) {
                        low = middle + 1;
                    } else {
                        high = middle;
                    }
                }
                low.min(all.len() - 1)
            });
            let rank = match (select, current) {
                (Select::Keep, None) => return Ok(()),
                (Select::Keep, Some(rank)) => rank,
                (Select::Newest, _) | (Select::Step { older: true }, None) => 0,
                (Select::Step { older: false }, None) => all.len() - 1,
                // 原来的位置上还是匹配才从它走一步；不是的话 current 已经是挨着的更早那个
                (Select::Step { older: true }, Some(rank)) if at(rank) == self.selected => (rank + 1) % all.len(),
                (Select::Step { older: true }, Some(rank)) => rank,
                (Select::Step { older: false }, Some(rank)) => (rank + all.len() - 1) % all.len(),
            };
            self.selected = at(rank);
            self.rank = Some(rank);
        }
        if select != Select::Keep
            && let Some((row, _)) = self.selected
        {
            scroll_into_view(term, row as usize, rows)?;
        }
        Ok(())
    }
}

/// 把回滚历史里的第 `row` 行滚到看得见的地方。已经看得见就不动；头两行算看不见——搜索条盖在右上角。
/// 要滚的时候把它放在离顶三分之一处，上下都留点上下文。
fn scroll_into_view(term: &mut Terminal<'static, 'static>, row: usize, rows: usize) -> libghostty_vt::error::Result<()> {
    let corner = term.grid_ref(Point::Viewport(PointCoordinate { x: 0, y: 0 }))?;
    let Some(top) = term.point_from_grid_ref(&corner, PointSpace::Screen)?.map(|p| p.y as usize) else { return Ok(()) };
    let covered = 2.min(rows.saturating_sub(1));
    if (top + covered..top + rows).contains(&row) {
        return Ok(());
    }
    let bottom = term.total_rows()?.saturating_sub(rows);
    let target = row.saturating_sub(rows / 3);
    // 到底了用 Bottom：视口继续跟着新输出走
    term.scroll_viewport(if target >= bottom { ScrollViewport::Bottom } else { ScrollViewport::Row(target) });
    Ok(())
}

impl Painter {
    /// 交给界面（拼好了还没交才交）。
    fn send(&mut self) {
        if !self.unsent {
            return;
        }
        self.unsent = false;
        *self.shared.frame.lock().unwrap() = self.built.clone();
        if !self.shared.notified.swap(true, Ordering::AcqRel) {
            (self.notify)(Event::Frame);
        }
    }

    /// 按终端此刻的样子拼一帧，和上一帧一样就不动。
    fn build(&mut self, term: &Terminal<'static, 'static>) -> libghostty_vt::error::Result<()> {
        let snapshot = self.render.update(term)?;
        let dirty = snapshot.dirty()?;
        let cols = snapshot.cols()?;
        let rows = snapshot.rows()?;
        let full = dirty == Dirty::Full
            || self.rows.len() != usize::from(rows)
            || self.rows.first().is_some_and(|row| row.cells.len() != usize::from(cols));
        // 格子没变也不能直接返回：只移动光标（← →、Home / End）不会把行标脏
        let clean = dirty == Dirty::Clean && !full;
        let palette = snapshot.colors()?;
        let background = rgb(palette.background);
        let foreground = rgb(palette.foreground);
        if full {
            // 空行之间共用一个 Arc（行不可变，重建时整行替换）
            let empty = Arc::new(Row::default());
            self.rows = vec![empty; usize::from(rows)];
        }

        let mut row_it = self.row_it.update(&snapshot)?;
        let mut y = 0usize;
        while let Some(row) = row_it.next() {
            if y >= self.rows.len() {
                break;
            }
            if full || row.dirty()? {
                let wrapped = row.raw_row()?.is_wrapped()?;
                let mut cells = Vec::with_capacity(usize::from(cols));
                let mut cell_it = self.cell_it.update(row)?;
                while let Some(cell) = cell_it.next() {
                    let mut out = Cell { fg: foreground, ..Cell::BLANK };
                    match cell.raw_cell()?.wide()? {
                        CellWide::Wide => out.flags |= flags::WIDE,
                        CellWide::SpacerTail | CellWide::SpacerHead => out.flags |= flags::SPACER,
                        _ => {}
                    }
                    if cell.graphemes_len()? > 0 {
                        self.text.clear();
                        cell.graphemes_utf8(&mut self.text)?;
                        let mut chars = self.text.chars();
                        out.ch = chars.next().unwrap_or(' ');
                        let rest = chars.as_str();
                        if !rest.is_empty() {
                            out.extra = Some(rest.into());
                        }
                    }
                    let mut fg = cell.fg_color()?.map_or(foreground, rgb);
                    let mut bg = cell.bg_color()?.map(rgb);
                    if cell.has_styling()? {
                        let style = cell.style()?;
                        // 粗体 + 基本 8 色画成亮色（xterm 的 drawBoldTextInBrightColors，旧版 SSHL 的观感）
                        if style.bold
                            && let StyleColor::Palette(PaletteIndex(i)) = style.fg_color
                                && i < 8 {
                                    fg = rgb(palette.palette[usize::from(i) + 8]);
                                }
                        if style.inverse {
                            let swapped = bg.unwrap_or(background);
                            bg = Some(fg);
                            fg = swapped;
                        }
                        out.flags |= flag(style.bold, flags::BOLD)
                            | flag(style.italic, flags::ITALIC)
                            | flag(style.faint, flags::FAINT)
                            | flag(style.invisible, flags::INVISIBLE)
                            | flag(style.strikethrough, flags::STRIKE)
                            | flag(style.overline, flags::OVERLINE)
                            | match style.underline {
                                Underline::None => 0,
                                Underline::Double => flags::UNDERLINE | flags::UNDERLINE_DOUBLE,
                                Underline::Curly => flags::UNDERLINE | flags::UNDERLINE_CURLY,
                                _ => flags::UNDERLINE,
                            };
                    }
                    out.base_bg = bg;
                    if cell.is_selected()? {
                        fg = colors::SELECTION_FG;
                        bg = Some(colors::SELECTION_BG);
                        out.flags |= flags::SELECTED;
                    }
                    out.fg = fg;
                    out.bg = bg;
                    cells.push(out);
                }
                self.rows[y] = Arc::new(Row { cells, wrapped });
            }
            y += 1;
        }

        let cursor = snapshot.cursor()?;
        let cursor = match (cursor.visible, cursor.viewport) {
            (true, Some(at)) => Some(Cursor {
                x: at.x,
                y: at.y,
                shape: match cursor.visual_style {
                    CursorVisualStyle::Block => CursorShape::Block,
                    CursorVisualStyle::Underline => CursorShape::Underline,
                    CursorVisualStyle::BlockHollow => CursorShape::BlockHollow,
                    _ => CursorShape::Bar,
                },
                blinking: cursor.blinking,
                color: palette.cursor.map_or(foreground, rgb),
                text_color: background,
            }),
            _ => None,
        };
        snapshot.clean()?;
        let mouse_tracking = term.is_mouse_tracking().unwrap_or(false);
        let alternate_screen = term.active_screen().is_ok_and(|s| s == Screen::Alternate);
        if clean {
            let last = &self.built;
            if last.cursor == cursor && last.mouse_tracking == mouse_tracking && last.alternate_screen == alternate_screen && last.search == self.search {
                return Ok(());
            }
        }

        let grid = &self.rows;
        let width = usize::from(cols);
        let at = |x: usize, y: usize| grid.get(y).and_then(|row| row.cells.get(x));
        let padding = frame::pick_edge_bg(width, grid.len(), |x, y| {
            at(x, y).map(|cell| cell.base_bg.unwrap_or(background))
        })
        .unwrap_or(background);
        let extension = frame::edge_extension(width, grid.len(), |x, y| at(x, y).and_then(|cell| cell.base_bg));
        let password_prompt = cursor
            .and_then(|c| grid.get(usize::from(c.y)))
            .is_some_and(|row| frame::is_password_prompt(&row.text()));

        let frame = Frame {
            cols,
            rows: self.rows.clone(),
            cursor,
            background,
            padding,
            extension,
            password_prompt,
            mouse_tracking,
            alternate_screen,
            search: self.search.clone(),
        };
        self.built = Arc::new(frame);
        self.unsent = true;
        Ok(())
    }
}

struct SelectionGesture {
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    drag: DragEvent<'static>,
    release: ReleaseEvent<'static>,
    start: Instant,
    pressed: bool,
}

impl SelectionGesture {
    fn new() -> libghostty_vt::error::Result<Self> {
        let mut press = PressEvent::new()?;
        // 双击 / 三击的间隔（同 macOS 默认）
        press.set_repeat_interval(Duration::from_millis(500))?;
        Ok(Self { gesture: Gesture::new()?, press, drag: DragEvent::new()?, release: ReleaseEvent::new()?, start: Instant::now(), pressed: false })
    }
}

/// 两个截止时刻里早的那个（都没有就 None：一直等消息）。
fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn flag(on: bool, bit: u16) -> u16 {
    if on { bit } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> (Engine, Arc<Shared>) {
        let (shell, _rx) = sshl_core::ssh::shell_channel();
        let shared = Arc::new(Shared { frame: Mutex::new(Arc::new(Frame::default())), notified: AtomicBool::new(false) });
        let size = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };
        (Engine::new(size, shell, shared.clone(), Box::new(|_| {})).unwrap(), shared)
    }

    fn cursor_x(engine: &mut Engine, shared: &Shared, bytes: &[u8]) -> u16 {
        engine.term.vt_write(bytes);
        engine.paint().unwrap();
        shared.frame.lock().unwrap().cursor.unwrap().x
    }

    /// 只移动光标（← →、Home / End）不改任何格子，帧里的光标位置也要跟上。
    #[test]
    fn moving_the_cursor_without_changing_cells_updates_the_frame() {
        let (mut engine, shared) = engine();
        assert_eq!(cursor_x(&mut engine, &shared, b"ls -la"), 6);
        assert_eq!(cursor_x(&mut engine, &shared, b"\x08"), 5);
        assert_eq!(cursor_x(&mut engine, &shared, b"\x08\x08"), 3);
        assert_eq!(cursor_x(&mut engine, &shared, b"\x1b[C"), 4);
        assert_eq!(cursor_x(&mut engine, &shared, b"\r"), 0);
    }

    fn blinking(engine: &mut Engine, shared: &Shared, bytes: &[u8]) -> bool {
        engine.write_output(bytes);
        engine.paint().unwrap();
        shared.frame.lock().unwrap().cursor.unwrap().blinking
    }

    /// 光标默认闪。vim / htop / less 退出时发的 cnorm（xterm 的 terminfo 里是 `\e[?12l\e[?25h`）
    /// 会用 DEC 12 关掉闪烁，之后光标就一直不闪了——这条不认；程序用 DECSCUSR 明说要什么样的光标才照办。
    #[test]
    fn the_cursor_keeps_blinking_after_cnorm_but_follows_decscusr() {
        let (mut engine, shared) = engine();
        assert!(blinking(&mut engine, &shared, b"$ "), "刚连上默认闪");
        assert!(blinking(&mut engine, &shared, b"\x1b[?12l\x1b[?25h"), "cnorm 之后照样闪");
        assert!(!blinking(&mut engine, &shared, b"\x1b[2 q"), "DECSCUSR 2：不闪的方块");
        assert!(blinking(&mut engine, &shared, b"\x1b[0 q"), "DECSCUSR 0：回到默认（闪）");
        assert!(blinking(&mut engine, &shared, b"\x1b[5 q"), "DECSCUSR 5：闪的竖线");
        // 序列被切在两片输出之间
        assert!(blinking(&mut engine, &shared, b"\x1b[4"), "还没收完，不变");
        assert!(!blinking(&mut engine, &shared, b" q"), "DECSCUSR 4（分两片到）：不闪的下划线");
        assert!(!blinking(&mut engine, &shared, b"\x1b[?12h\x1b[?12l"), "DEC 12 不认，仍按 DECSCUSR 不闪");
    }

    fn first_row(shared: &Shared) -> String {
        shared.frame.lock().unwrap().rows[0].text().trim_end().to_string()
    }

    fn painted(engine: &mut Engine, shared: &Shared, bytes: &[u8]) -> String {
        engine.write_output(bytes);
        engine.paint().unwrap();
        first_row(shared)
    }

    /// 同步输出（DEC 2026）：程序在 hold 里擦掉重写，hold 结束前界面只看到上一个完整的帧，不出半截的。
    #[test]
    fn a_render_hold_shows_the_last_finished_frame_until_it_ends() {
        let (mut engine, shared) = engine();
        assert_eq!(painted(&mut engine, &shared, b"old frame"), "old frame");
        assert_eq!(painted(&mut engine, &shared, b"\x1b[?2026h\r\x1b[2Knew fr"), "old frame", "hold 里擦了一半不画");
        assert_eq!(painted(&mut engine, &shared, b"ame"), "old frame");
        assert_eq!(painted(&mut engine, &shared, b"\x1b[?2026l"), "new frame", "hold 结束一次换上");
    }

    /// 一段 hold 结束、下一段马上开始（同一片输出里）：中间那个完整的帧要画出来，不能一直停在更早的帧上。
    #[test]
    fn back_to_back_holds_still_show_each_finished_frame() {
        let (mut engine, shared) = engine();
        assert_eq!(painted(&mut engine, &shared, b"frame 0"), "frame 0");
        let both = b"\x1b[?2026h\r\x1b[2Kframe 1\x1b[?2026l\x1b[?2026h\r\x1b[2Kfra";
        assert_eq!(painted(&mut engine, &shared, both), "frame 1");
        assert_eq!(painted(&mut engine, &shared, b"me 2\x1b[?2026l"), "frame 2");
    }

    /// 程序开了 hold 再也不结束（崩了）：超时后照常出帧。
    #[test]
    fn a_hold_that_never_ends_times_out() {
        let (mut engine, shared) = engine();
        assert_eq!(painted(&mut engine, &shared, b"\x1b[?2026hstuck"), "");
        engine.expire_hold(Instant::now() + HOLD_TIMEOUT);
        engine.paint().unwrap();
        assert_eq!(first_row(&shared), "stuck");
        assert!(!engine.term.mode(Mode::SYNC_OUTPUT).unwrap());
    }

    fn search(shared: &Shared) -> Arc<frame::SearchInfo> {
        shared.frame.lock().unwrap().search.clone().expect("帧里有搜索状态")
    }

    /// 搜到底（测试里不分片）并出帧。
    fn searched(engine: &mut Engine, shared: &Shared, input: Input) -> Option<Arc<frame::SearchInfo>> {
        engine.handle(input);
        while engine.refresh_search().1 {}
        engine.paint().unwrap();
        shared.frame.lock().unwrap().search.clone()
    }

    /// ⌘F：回滚历史里的也找得到，不分大小写；一搜就选中最新的那个，屏幕上看得见的匹配带高亮。
    #[test]
    fn search_finds_matches_in_scrollback_and_highlights_the_visible_ones() {
        let (mut engine, shared) = engine();
        engine.write_output(b"Needle one\r\n");
        for i in 0..100 {
            engine.write_output(format!("filler {i}\r\n").as_bytes());
        }
        engine.write_output(b"second NEEDLE here");
        let info = searched(&mut engine, &shared, Input::Search("needle".into())).unwrap();
        assert_eq!((info.total, info.selected, info.searching), (2, Some(0), false));
        // 最新的那个在最后一行（第 23 行）第 7–12 列，是选中的
        assert_eq!(info.spans, [frame::MatchSpan { y: 23, start: 7, end: 13, selected: true }]);
    }

    /// ↵ 往更早的走（滚到回滚历史里那一个），⇧↵ 往回；到头绕回。
    #[test]
    fn search_steps_through_matches_and_scrolls_to_them() {
        let (mut engine, shared) = engine();
        engine.write_output(b"needle one\r\n");
        for i in 0..100 {
            engine.write_output(format!("filler {i}\r\n").as_bytes());
        }
        engine.write_output(b"second needle here");
        searched(&mut engine, &shared, Input::Search("needle".into()));
        let info = searched(&mut engine, &shared, Input::SearchStep { older: true }).unwrap();
        assert_eq!(info.selected, Some(1));
        assert!(!engine.term.viewport_active().unwrap(), "滚到回滚历史里去了");
        assert_eq!(info.spans.len(), 1);
        assert!(info.spans[0].selected && (info.spans[0].start, info.spans[0].end) == (0, 6));
        let info = searched(&mut engine, &shared, Input::SearchStep { older: true }).unwrap();
        assert_eq!(info.selected, Some(0), "过了最早的绕回最新的");
        let info = searched(&mut engine, &shared, Input::SearchStep { older: false }).unwrap();
        assert_eq!(info.selected, Some(1));
    }

    /// 逐字敲搜索词（先「1」再「12」）：每次都选中最新的那个，它本来就在屏幕上，视口不该动。
    #[test]
    fn typing_the_needle_letter_by_letter_keeps_the_viewport_at_the_bottom() {
        let (mut engine, shared) = engine();
        for i in 1..=300 {
            engine.write_output(format!("{i}\r\n").as_bytes());
        }
        engine.write_output(b"needle 12 NEEDLE\r\n$ ");
        let info = searched(&mut engine, &shared, Input::Search("1".into())).unwrap();
        assert_eq!(info.selected, Some(0));
        assert!(engine.term.viewport_active().unwrap(), "搜「1」之后还在底部");
        let info = searched(&mut engine, &shared, Input::Search("12".into())).unwrap();
        assert_eq!((info.total, info.selected), (14, Some(0)));
        assert!(engine.term.viewport_active().unwrap(), "搜「12」之后还在底部");
        assert!(info.spans.iter().any(|span| span.selected && span.y == 22), "选中的是倒数第二行那个：{:?}", info.spans);
    }

    /// 宽终端（回滚历史和屏幕落在同一页）上，libghostty 把屏幕上的匹配排在回滚历史的后面。
    /// 不管它怎么排：一搜选中的是最新（最靠下）的那个、视口不动、编号从它算起，↵ 再一个个往上走。
    #[test]
    fn the_newest_match_is_selected_first_on_a_wide_terminal() {
        let (mut engine, shared) = engine();
        engine.handle(Input::Resize(GridSize { cols: 180, rows: 47, cell_width: 8, cell_height: 16 }));
        for i in 1..=300 {
            engine.write_output(format!("{i}\r\n").as_bytes());
        }
        engine.write_output(b"% echo needle 12 NEEDLE\r\nneedle 12 NEEDLE\r\n% ");
        let selected_row = |info: &frame::SearchInfo| info.spans.iter().find(|span| span.selected).map(|span| span.y);
        let info = searched(&mut engine, &shared, Input::Search("12".into())).unwrap();
        assert_eq!((info.total, info.selected), (15, Some(0)));
        assert!(engine.term.viewport_active().unwrap(), "最新的匹配就在屏幕上，视口不动");
        assert_eq!(selected_row(&info), Some(45), "选中的是输出那一行（倒数第二行）");
        let info = searched(&mut engine, &shared, Input::SearchStep { older: true }).unwrap();
        assert_eq!((info.selected, selected_row(&info)), (Some(1), Some(44)), "再往上是命令那一行");
        let info = searched(&mut engine, &shared, Input::SearchStep { older: true }).unwrap();
        assert_eq!(info.selected, Some(2));
        assert!(!engine.term.viewport_active().unwrap(), "第三个（212）在回滚历史里，滚过去");
        let info = searched(&mut engine, &shared, Input::SearchStep { older: false }).unwrap();
        let info2 = searched(&mut engine, &shared, Input::SearchStep { older: false }).unwrap();
        assert_eq!((info.selected, info2.selected), (Some(1), Some(0)));
        let info = searched(&mut engine, &shared, Input::SearchStep { older: false }).unwrap();
        assert_eq!(info.selected, Some(14), "从最新的再往新走，绕到最早的");
    }

    /// 不管回滚历史多长、屏幕跨不跨内部页：一搜选中最下面那个，↵ 一路往上（位置严格递减），不来回跳。
    #[test]
    fn search_steps_strictly_upward_whatever_the_history_length() {
        let mut bad = Vec::new();
        // 541、800、837、1096、1355 行是实际出过错的（屏幕跨两个内部页、前面还有回滚历史）
        for lines in [60, 300, 541, 800, 837, 1096, 1355] {
            let (mut engine, _shared) = engine();
            engine.handle(Input::Resize(GridSize { cols: 180, rows: 47, cell_width: 8, cell_height: 16 }));
            for i in 1..=lines {
                engine.write_output(format!("row {i}\r\n").as_bytes());
            }
            engine.handle(Input::Search("row".into()));
            while engine.refresh_search().1 {}
            let at = |engine: &mut Engine| engine.search.as_ref().unwrap().selected.unwrap().0;
            let mut seen = vec![at(&mut engine)];
            for _ in 0..(lines.min(120) - 1) {
                engine.handle(Input::SearchStep { older: true });
                seen.push(at(&mut engine));
            }
            let first_ok = seen[0] as usize == lines - 1;
            let descending = seen.windows(2).all(|w| w[1] + 1 == w[0]);
            if !first_ok || !descending {
                let jump = seen.windows(2).position(|w| w[1] + 1 != w[0]);
                bad.push((lines, seen[0], jump.map(|j| (j, seen[j], seen[j + 1]))));
            }
        }
        assert!(bad.is_empty(), "（行数, 首次选中的行, 第一次乱跳的位置）：{bad:?}");
    }

    /// 搜索开着时新输出里的匹配跟着算进来；没有结果时是 0；关掉（空串）后帧里不再带搜索状态。
    #[test]
    fn search_follows_new_output_and_goes_away_when_closed() {
        let (mut engine, shared) = engine();
        engine.write_output(b"alpha\r\n");
        let info = searched(&mut engine, &shared, Input::Search("beta".into())).unwrap();
        assert_eq!((info.total, info.selected), (0, None));
        engine.write_output(b"beta\r\nBETA\r\n");
        while engine.refresh_search().1 {}
        engine.paint().unwrap();
        let info = search(&shared);
        assert_eq!(info.total, 2);
        assert_eq!(info.spans.len(), 2);
        assert!(searched(&mut engine, &shared, Input::Search(String::new())).is_none());
    }

    /// 问 `col` 列、第 0 行那一格的 OSC 8 超链接（格子 8×16 设备像素，取格子中间）。
    fn hyperlink(engine: &mut Engine, col: u16) -> Option<String> {
        let (reply, mut answer) = futures::channel::oneshot::channel();
        engine.handle(Input::LinkAt { x: f32::from(col) * 8.0 + 4.0, y: 8.0, reply });
        answer.try_recv().unwrap().expect("终端线程处理时就答了")
    }

    /// ⌘ 点击 OSC 8 链接：链接文字所在的格子查得到网址，后面的普通文字没有。
    #[test]
    fn osc8_hyperlinks_are_found_by_cell() {
        let (mut engine, _shared) = engine();
        engine.write_output(b"\x1b]8;;https://example.com/x\x1b\\link\x1b]8;;\x1b\\ plain");
        assert_eq!(hyperlink(&mut engine, 0).as_deref(), Some("https://example.com/x"));
        assert_eq!(hyperlink(&mut engine, 3).as_deref(), Some("https://example.com/x"));
        assert_eq!(hyperlink(&mut engine, 4), None, "链接后面的空格");
        assert_eq!(hyperlink(&mut engine, 6), None, "plain");
    }

    /// 网址比默认缓冲区（256 字节）长：按 libghostty 给的大小再取一次，拿到完整的。
    #[test]
    fn long_osc8_hyperlinks_are_read_in_full() {
        let (mut engine, _shared) = engine();
        let url = format!("https://example.com/{}", "a".repeat(500));
        engine.write_output(format!("\x1b]8;;{url}\x1b\\link\x1b]8;;\x1b\\").as_bytes());
        assert_eq!(hyperlink(&mut engine, 1), Some(url));
    }

    /// 排满一行接着写的（软换行）在帧里标出来：⌘ 点击按它把折成几行的网址拼回一个。回车换行的不算。
    #[test]
    fn soft_wrapped_rows_are_marked_in_the_frame() {
        let (mut engine, shared) = engine();
        engine.write_output(format!("{}tail\r\nnext", "x".repeat(80)).as_bytes());
        engine.paint().unwrap();
        let frame = shared.frame.lock().unwrap().clone();
        let wrapped: Vec<bool> = frame.rows.iter().take(3).map(|row| row.wrapped).collect();
        assert_eq!(wrapped, [true, false, false]);
    }

    /// Claude Code 先用 DECRQM 问支不支持 2026，答了「支持」才发同步输出（它不认识 TERM_PROGRAM=SSHL）。
    /// 应答由 libghostty 生成、经 on_pty_write 写回 shell。
    #[test]
    fn the_terminal_reports_synchronized_output_as_supported() {
        let replies = Rc::new(RefCell::new(Vec::new()));
        let sink = replies.clone();
        let mut term = Terminal::new(80, 24).unwrap();
        term.on_pty_write(move |_, data| sink.borrow_mut().extend_from_slice(data)).unwrap();
        term.vt_write(b"\x1b[?2026$p");
        assert_eq!(replies.borrow().as_slice(), b"\x1b[?2026;2$y", "2 = 支持、当前关着");
    }
}

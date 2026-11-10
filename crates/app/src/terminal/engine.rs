//! 终端线程：每个会话一个，独占 libghostty-vt 的终端（它的类型都是 !Send）。
//! SSH 输出、键鼠、改尺寸都以消息进来；解析完一批就把屏幕整理成 [`Frame`] 交给界面，并确认流控字节数。
//! 界面线程从不碰终端本身，刷屏时解析和整理帧都不占界面。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libghostty_vt::fmt::{Format, Formatter};
use libghostty_vt::key;
use libghostty_vt::mouse;
use libghostty_vt::selection::gesture::{DragEvent, Gesture, Geometry, PressEvent, ReleaseEvent};
use libghostty_vt::terminal::{Point, PointCoordinate};
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

struct Engine {
    term: Terminal<'static, 'static>,
    render: RenderState<'static>,
    row_it: RowIterator<'static>,
    cell_it: CellIterator<'static>,
    key_encoder: key::Encoder<'static>,
    key_event: key::Event<'static>,
    mouse_encoder: mouse::Encoder<'static>,
    mouse_event: mouse::Event<'static>,
    selection: SelectionGesture,
    shell: Arc<Mutex<ShellHandle>>,
    shared: Arc<Shared>,
    notify: Box<dyn Fn(Event) + Send>,
    size: GridSize,
    rows: Vec<Arc<Row>>,
    /// 解析完、还没确认给 SSH 流控的字节数。
    unacked: u32,
    pty_size: (u16, u16),
    pty_sent_at: Option<Instant>,
    pty_pending: bool,
    buf: Vec<u8>,
    text: String,
    focused: bool,
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

        Ok(Self {
            term,
            render: RenderState::new()?,
            row_it: RowIterator::new()?,
            cell_it: CellIterator::new()?,
            key_encoder: key::Encoder::new()?,
            key_event: key::Event::new()?,
            mouse_encoder: mouse::Encoder::new()?,
            mouse_event: mouse::Event::new()?,
            selection: SelectionGesture::new()?,
            shell,
            shared,
            notify,
            size,
            rows: Vec::new(),
            unacked: 0,
            pty_size: (size.cols, size.rows),
            pty_sent_at: None,
            pty_pending: false,
            buf: Vec::with_capacity(64),
            text: String::with_capacity(8),
            focused: true,
        })
    }

    fn run(&mut self, rx: Receiver<Input>) {
        // 还没解析的 SSH 输出（最多一个流控窗口，2MB）
        let mut backlog: VecDeque<Vec<u8>> = VecDeque::new();
        let mut offset = 0usize;
        let mut dirty = false;
        let mut bell = false;
        let mut last_frame = Instant::now() - FRAME_INTERVAL;
        loop {
            // 有积压就不阻塞；否则等消息（有待发的 PTY 尺寸时最多等到它该发的时刻）
            let wait = if backlog.is_empty() {
                self.pty_deadline().map(|deadline| deadline.saturating_duration_since(Instant::now()))
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
            for input in first.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok())) {
                match input {
                    Input::Shutdown => return,
                    Input::Output(bytes) => backlog.push_back(bytes),
                    other => {
                        self.handle(other);
                        dirty = true;
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
                self.term.vt_write(piece);
                self.unacked = self.unacked.saturating_add(take as u32);
                budget -= take;
                offset += take;
                if offset == front.len() {
                    backlog.pop_front();
                    offset = 0;
                }
                dirty = true;
            }
            if self.unacked > 0 {
                self.shell.lock().unwrap().ack(std::mem::take(&mut self.unacked));
            }
            self.flush_pty_size();
            // 出帧：积压清空时立刻出；刷屏中最多每 FRAME_INTERVAL 一帧，整理帧不拖慢解析
            if dirty && (backlog.is_empty() || last_frame.elapsed() >= FRAME_INTERVAL) {
                if let Err(error) = self.publish() {
                    tracing::warn!("terminal frame failed: {error}");
                }
                last_frame = Instant::now();
                dirty = false;
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

    /// 把屏幕整理成帧交给界面：只重建脏行，没变的行沿用上一帧的 Arc。
    fn publish(&mut self) -> libghostty_vt::error::Result<()> {
        let snapshot = self.render.update(&self.term)?;
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
                self.rows[y] = Arc::new(Row { cells });
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
        let mouse_tracking = self.term.is_mouse_tracking().unwrap_or(false);
        let alternate_screen = self.term.active_screen().is_ok_and(|s| s == Screen::Alternate);
        if clean {
            let last = self.shared.frame.lock().unwrap();
            if last.cursor == cursor && last.mouse_tracking == mouse_tracking && last.alternate_screen == alternate_screen {
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
        };
        *self.shared.frame.lock().unwrap() = Arc::new(frame);
        if !self.shared.notified.swap(true, Ordering::AcqRel) {
            (self.notify)(Event::Frame);
        }
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
        engine.publish().unwrap();
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
}

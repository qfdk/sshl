//! 「系统」页：负载 / 内存 / 磁盘 / 运行时间，进程、监听端口和 systemd 服务；结束进程、启停服务。
//! 看服务日志发到终端里做。看的是当前标签那台机器：远程会话的服务器，或者本机标签的这台 Mac（没有服务列表）。
//! 远程的命令以文件管理当前身份执行（终端 sudo -i 之后就是 root），本机的以 SSHL 自己的身份；
//! 没权限时提议改在终端里用 sudo 执行。停在这页时每 3 秒刷新一次。

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, ClipboardItem, Context, Div, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    FontFeatures, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, PathBuilder, Pixels,
    Point, Render, Role, SharedString, Stateful, StatefulInteractiveElement, StyledText, Styled, Task,
    Window, anchored, canvas, deferred, div, img, list, point, prelude::FluentBuilder, px, size, svg,
};
use gpui_base::input::{Input, InputEvent, InputState};
use sshl_core::server_info::{self, Machine, Overview, PortInfo, ProcessInfo};
use sshl_core::services::{self, ServiceAction, ServiceInfo};

use crate::app_icons::{self, ICON_SIZE};
use crate::assets::{Brand, Icon};
use crate::brands;
use crate::backend;
use crate::prompts::{self, Options};
use crate::theme::{self, ui};
use crate::ui::{self as kit, MenuEntry, MenuItem};

mod apps;
mod disk;
mod treemap;

use apps::{AppGroup, Listed};
use disk::DiskState;

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const ROW_HEIGHT: f32 = 34.;
/// 进程名后面那段（命令行）一行的高度。
const DETAIL_LINE: f32 = 18.;
/// 命令行、监听地址用等宽字；数字用界面字体的等宽数字（[`TABULAR`]）。
const MONO: &str = "Menlo";
const PID_WIDTH: f32 = 76.;
const USER_WIDTH: f32 = 130.;
const CPU_WIDTH: f32 = 96.;
const MEMORY_WIDTH: f32 = 104.;
const ACTION_WIDTH: f32 = 64.;
const ADDRESS_WIDTH: f32 = 250.;
const STATUS_WIDTH: f32 = 104.;
/// 「开机启动 ↑」放得下箭头。
const BOOT_WIDTH: f32 = 96.;
/// 指标那一段的同色浅底、圆环轨道的不透明度：[浅色, 深色]。
pub const SEGMENT_TINT: [f32; 2] = [0.06, 0.10];
const RING_TRACK: [f32; 2] = [0.16, 0.22];
/// 搜索命中的那一段的底色（琥珀色）：[浅色, 深色]。
const MATCH_TINT: [f32; 2] = [0.28, 0.4];
/// 软件标志画多大：标志铺满自己的方框，应用图标四周有留白，画小一点看着一样大。
const BRAND_SIZE: f32 = 17.;
/// CPU 用得多（≥ 5%）的进程那一格的同色浅底：[浅色, 深色]。原来是实心色块 + 白字，用户嫌「颜色过于重」。
pub const CPU_TINT: [f32; 2] = [0.10, 0.18];
/// 鼠标停在哪一行，那一行铺一层浅蓝：[浅色, 深色]。
const ROW_HOVER: [f32; 2] = [0.08, 0.14];
const GAUGE_SIZE: f32 = 58.;
/// 指标一段至少这么宽：圆环 + 三个负载值（19px）放得下；四段一排放不下时换行。
const SEGMENT_MIN_WIDTH: f32 = 250.;
const RING_STROKE: f32 = 6.;
/// 等宽数字（tnum）：一列数字上下对齐，不用借等宽字体。
static TABULAR: LazyLock<FontFeatures> = LazyLock::new(|| FontFeatures(Arc::new(vec![("tnum".into(), 1)])));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Processes,
    Ports,
    Services,
    /// 分区用量和目录占用（见 `disk.rs`）。
    Disk,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Pid,
    User,
    Cpu,
    Memory,
    /// 「进程」列：按进程名（不按命令行路径）。
    Name,
}

/// 可点表头排序的列。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortColumn {
    Process(SortKey),
    Port(PortSortKey),
    Service(ServiceSortKey),
}

/// 表头文字的对齐（跟下面的列一致）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Start,
    End,
    Center,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceSortKey {
    Status,
    Name,
    Description,
    Boot,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortSortKey {
    Proto,
    Port,
    Address,
    Pid,
    Process,
}

enum MenuTarget {
    Process { pid: u32, name: String, command: Option<String> },
    Service(ServiceInfo),
    /// 磁盘页里的一个目录：远程的能在文件管理里打开，本机的在访达里显示。
    /// `folder` 是在文件管理里要打开的目录（文件就是它所在的目录）。
    Directory { path: String, folder: String, remote: bool },
}

/// 进程行前面的图标。
enum ProcessIcon {
    /// 所在应用的图标（还没取到时 None，先留位置）。
    App(Option<Arc<gpui::Image>>),
    /// 认得出的服务端软件（nginx、redis…）的标志，品牌色。
    Brand(Brand),
    /// 别的：淡色线条图标。
    Glyph(Icon),
}

/// 一个应用的图标取到哪了。
#[derive(Clone)]
enum AppIcon {
    Loading,
    Ready(Arc<gpui::Image>),
    /// 没有自己的图标（系统给的是通用占位图）或者取不到：画齿轮，不再取。
    Missing,
}

/// 一行用哪个图标：应用（路径）的、软件的标志，或者线条图标。
#[derive(Clone)]
enum RowIcon {
    App(String),
    Brand(Brand),
    Glyph(Icon),
}

/// 不在应用里的进程用什么图标：认得出的软件画标志；系统的后台程序（放在系统目录、或者不是登录用户自己跑的）画齿轮，
/// 别的（自己跑的 shell、脚本）画终端。
fn process_icon(name: &str, program: &str, user: &str, me: Option<&str>, kernel: bool) -> RowIcon {
    if let Some(brand) = brands::brand_of(name) {
        return RowIcon::Brand(brand);
    }
    let system = kernel || app_icons::is_system_program(program) || user_kind(user, me) != UserKind::Me;
    RowIcon::Glyph(if system { Icon::Settings } else { Icon::SquareTerminal })
}

/// 当前这张表的行：每帧在 `render` 里算一次，可变行高的列表按行从里面取。
enum Table {
    Processes(Rc<Vec<ProcessRow>>),
    Ports(Rc<Vec<PortInfo>>),
    Services(Rc<Vec<ServiceInfo>>),
    None,
}

/// 进程列表要画的一行（从进程 / 应用里取出来，不借着面板）。
#[derive(Clone)]
struct ProcessRow {
    /// 应用那一行是主进程的。
    pid: u32,
    user: String,
    /// 应用那一行是加起来的。
    cpu: f32,
    rss_kb: u64,
    name: String,
    /// 名字后面淡色的那段：远程是完整命令行；本机只有应用写「N 个进程」（同 Mole，路径、参数都不要；完整命令行在右键「复制命令」里）。
    detail: Option<String>,
    command: String,
    /// None = 不画图标（远程）。
    icon: Option<RowIcon>,
    /// 应用那一行：(应用路径, 几个进程, 展开了没有)。
    app: Option<(String, usize, bool)>,
    /// 展开的应用里的一个进程（缩进）。
    member: bool,
    kernel: bool,
}

impl ProcessRow {
    /// 远程：程序名后面跟完整命令行（命令行就只是程序名时不重复）。服务器上没有应用图标，认得出的软件画标志（`brands`）。
    /// 展开的进程树里的进程不画图标（缩进对齐）。
    fn remote(p: &ProcessInfo, me: Option<&str>, member: bool) -> Self {
        let name = process_name(&p.command, p.program.as_deref());
        let detail = (p.command != name).then(|| p.command.clone());
        let kernel = p.command.starts_with('[');
        let program = p.program.as_deref().unwrap_or_else(|| p.command.split(' ').next().unwrap_or(""));
        let icon = (!member).then(|| process_icon(&name, program, &p.user, me, kernel));
        Self { pid: p.pid, user: p.user.clone(), cpu: p.cpu, rss_kb: p.rss_kb, name, detail, command: p.command.clone(), icon, app: None, member, kernel }
    }

    /// 本机的单个进程：只有名字。用户：「本地理论上不要这么长的路径吧」「这样的参数应该用不着了」。
    fn local(p: &ProcessInfo, me: Option<&str>, member: bool) -> Self {
        let name = process_name(&p.command, p.program.as_deref());
        // 展开的应用里的进程不画图标（缩进对齐应用名）；别的：认得出的软件（claude、node）画标志，系统的后台程序（放在 /System、
        // /usr/libexec 这些地方，或者以 root、系统账号跑的）画齿轮，剩下的是自己跑的命令行程序，画终端
        let program = p.program.as_deref().unwrap_or_else(|| p.command.split(' ').next().unwrap_or(""));
        let icon = (!member).then(|| process_icon(&name, program, &p.user, me, false));
        Self {
            pid: p.pid,
            user: p.user.clone(),
            cpu: p.cpu,
            rss_kb: p.rss_kb,
            name,
            detail: None,
            command: p.command.clone(),
            icon,
            app: None,
            member,
            kernel: false,
        }
    }

    /// 一个应用（本机）或一棵进程树（远程）：主进程的 PID 和用户，CPU、内存加起来；有几个进程时后面写几个。
    /// 本机的应用用应用图标，远程的按进程名认标志。
    fn app(group: AppGroup, open: bool, me: Option<&str>) -> Self {
        let count = group.members.len();
        let detail = (count > 1).then(|| tr!("{} 个进程", count));
        let icon = if group.app {
            RowIcon::App(group.key.clone())
        } else {
            let program = group.main.program.as_deref().unwrap_or_else(|| group.main.command.split(' ').next().unwrap_or(""));
            process_icon(&group.name, program, &group.main.user, me, false)
        };
        Self {
            pid: group.main.pid,
            user: group.main.user.clone(),
            cpu: group.cpu,
            rss_kb: group.rss_kb,
            name: group.name,
            detail,
            command: group.main.command.clone(),
            icon: Some(icon),
            app: Some((group.key, count, open)),
            member: false,
            kernel: false,
        }
    }
}

struct MenuState {
    position: Point<Pixels>,
    target: MenuTarget,
}

pub enum SystemPanelEvent {
    /// 发到终端执行（没权限时的 sudo 命令、看服务日志）。
    RunInTerminal(String),
    /// 在文件管理里打开这个远程目录（磁盘页的右键菜单）。
    OpenInFiles(String),
}

/// 一次刷新读到的列表。
enum Loaded {
    /// 进程和负责进程（本机才有，见 `apps::responsible_pids`）。
    Processes(Vec<ProcessInfo>, HashMap<u32, u32>),
    Ports(Vec<PortInfo>),
    /// None = 没有 systemd。
    Services(Option<Vec<ServiceInfo>>),
    Filesystems(Vec<sshl_core::disk_usage::Filesystem>),
}

pub struct SystemPanel {
    /// 看的是哪台机器；None = 当前标签没连着。
    machine: Option<Machine>,
    /// 那台机器的登录用户（进程列表里标蓝）。
    me: Option<String>,
    active: bool,
    view: View,
    overview: Option<Overview>,
    processes: Option<Vec<ProcessInfo>>,
    ports: Option<Vec<PortInfo>>,
    services: Option<Vec<ServiceInfo>>,
    disk: DiskState,
    /// 本机进程的应用图标：应用路径 → 图标。
    icons: HashMap<String, AppIcon>,
    /// 应用路径 → 访达里的名字（微信、日历），和图标一起取。
    app_names: HashMap<String, String>,
    /// 展开了的应用（路径）。
    expanded_apps: std::collections::HashSet<String>,
    /// 本机进程的负责进程（见 `apps::responsible_pids`）。
    responsible: HashMap<u32, u32>,
    no_systemd: bool,
    /// 正在启停的服务（状态列显示「处理中…」）。
    busy: Option<String>,
    error: Option<SharedString>,
    sort: (SortKey, bool),
    port_sort: (PortSortKey, bool),
    service_sort: (ServiceSortKey, bool),
    search: Entity<InputState>,
    search_focused: bool,
    menu: Option<MenuState>,
    request: u64,
    /// 进程 / 端口 / 服务表（行高跟着内容）。
    rows: kit::Rows,
    focus: FocusHandle,
    _refresh: Option<Task<()>>,
    _subscription: gpui::Subscription,
}

impl EventEmitter<SystemPanelEvent> for SystemPanel {}

impl Focusable for SystemPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

// ---------------------------------------------------------------- 纯函数（有测试）

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    // 数字和单位之间是不断行空格（U+00A0）：折行时不会拆成「3.3」「GB」两行
    if unit == 0 { format!("{bytes}\u{a0}B") } else if value >= 100. { format!("{value:.0}\u{a0}{}", UNITS[unit]) } else { format!("{value:.1}\u{a0}{}", UNITS[unit]) }
}

pub fn format_uptime(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    match (days, hours) {
        (0, 0) => tr!("{} 分钟", minutes.max(1)),
        (0, _) => tr!("{} 小时 {} 分", hours, minutes),
        _ => tr!("{} 天 {} 小时", days, hours),
    }
}

/// 这台机器有哪几个列表：本机（macOS）没有 systemd 服务。没连着时照旧全列（只显示空状态）。
pub fn views_for(machine: Option<&Machine>) -> &'static [View] {
    match machine {
        Some(Machine::Local) => &[View::Processes, View::Ports, View::Disk],
        _ => &[View::Processes, View::Ports, View::Services, View::Disk],
    }
}

/// 换到另一台机器时还停在原来的列表；那台没有这个列表（本机没有服务）就回到进程。
pub fn view_on(machine: Option<&Machine>, view: View) -> View {
    if views_for(machine).contains(&view) { view } else { View::Processes }
}

/// 搜索：端口号、进程名、PID、地址里含关键字。
/// 排序：看不到进程（没权限）的行按 PID / 进程排时总在最后；同值的按端口从小到大。
pub fn filter_ports<'a>(ports: &'a [PortInfo], query: &str, sort: (PortSortKey, bool)) -> Vec<&'a PortInfo> {
    let query = query.trim().to_lowercase();
    let mut out: Vec<&PortInfo> = ports
        .iter()
        .filter(|p| {
            query.is_empty()
                || p.port.to_string().contains(&query)
                || p.address.contains(&query)
                || p.pid.is_some_and(|pid| pid.to_string().contains(&query))
                || p.process.as_deref().is_some_and(|name| name.to_lowercase().contains(&query))
        })
        .collect();
    out.sort_by(|a, b| {
        let missing = match sort.0 {
            PortSortKey::Pid => a.pid.is_none().cmp(&b.pid.is_none()),
            PortSortKey::Process => a.process.is_none().cmp(&b.process.is_none()),
            _ => std::cmp::Ordering::Equal,
        };
        let order = match sort.0 {
            PortSortKey::Proto => a.proto.cmp(&b.proto),
            PortSortKey::Port => a.port.cmp(&b.port),
            PortSortKey::Address => a.address.cmp(&b.address),
            PortSortKey::Pid => a.pid.cmp(&b.pid),
            PortSortKey::Process => a.process.as_deref().map(str::to_lowercase).cmp(&b.process.as_deref().map(str::to_lowercase)),
        };
        missing.then(if sort.1 { order.reverse() } else { order }).then(a.port.cmp(&b.port)).then_with(|| a.proto.cmp(&b.proto))
    });
    out
}

/// 结束进程失败的说明：没权限时教用户怎么拿到权限。
pub fn kill_error_message(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("not permitted") || lower.contains("permission") {
        tr!("没有权限结束这个进程。在终端里 sudo -i（或 su）切到 root 后再试，这里会跟着用 root 身份。").into()
    } else if lower.contains("no such process") {
        tr!("进程已经不在了。").into()
    } else {
        error.trim().to_string()
    }
}

/// 搜索：服务名或说明里含关键字。按状态排（默认）同后端：失败、运行中、其余；没有说明的按说明排时总在最后；同值按显示的名字（不带 .service，apport 才在 apport-autoreport 前面）。
pub fn filter_services<'a>(services: &'a [ServiceInfo], query: &str, sort: (ServiceSortKey, bool)) -> Vec<&'a ServiceInfo> {
    let query = query.trim().to_lowercase();
    let mut out: Vec<&ServiceInfo> = services
        .iter()
        .filter(|s| query.is_empty() || s.unit.to_lowercase().contains(&query) || s.description.to_lowercase().contains(&query))
        .collect();
    let status_rank = |s: &ServiceInfo| match (s.active.as_str(), s.sub.as_str()) {
        ("failed", _) => 0,
        ("active", "running") | ("activating" | "reloading", _) => 1,
        _ => 2,
    };
    let boot_rank = |s: &ServiceInfo| match s.enabled.as_str() {
        _ if s.starts_at_boot() => 0,
        "disabled" => 1,
        "masked" => 2,
        _ => 3,
    };
    out.sort_by(|a, b| {
        let missing = match sort.0 {
            ServiceSortKey::Description => a.description.is_empty().cmp(&b.description.is_empty()),
            _ => std::cmp::Ordering::Equal,
        };
        let order = match sort.0 {
            ServiceSortKey::Status => status_rank(a).cmp(&status_rank(b)),
            ServiceSortKey::Name => a.name().to_lowercase().cmp(&b.name().to_lowercase()),
            ServiceSortKey::Description => a.description.to_lowercase().cmp(&b.description.to_lowercase()),
            ServiceSortKey::Boot => boot_rank(a).cmp(&boot_rank(b)),
        };
        missing.then(if sort.1 { order.reverse() } else { order }).then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
    });
    out
}

/// 服务的状态文字和颜色。
pub fn service_status(service: &ServiceInfo) -> (&'static str, ui::Token) {
    match (service.active.as_str(), service.sub.as_str()) {
        ("failed", _) => (tr!("失败"), ui::DESTRUCTIVE),
        // 一次性服务（防火墙规则、AppArmor）跑完退出但仍算启用
        ("active", "exited") => (tr!("已生效"), ui::SUCCESS),
        ("active", _) => (tr!("运行中"), ui::SUCCESS),
        ("activating", _) => (tr!("启动中"), ui::WARNING),
        ("deactivating", _) => (tr!("停止中"), ui::WARNING),
        ("reloading", _) => (tr!("重载中"), ui::WARNING),
        _ => (tr!("已停止"), ui::MUTED_FOREGROUND),
    }
}

/// 用量比例 → 颜色：≥ 90% 红、≥ 75% 橙，其余用这项指标自己的颜色。
pub fn usage_color(ratio: f32, base: ui::Token) -> ui::Token {
    if ratio >= 0.9 {
        ui::DESTRUCTIVE
    } else if ratio >= 0.75 {
        ui::WARNING
    } else {
        base
    }
}

/// 进程 CPU%（一个核满载是 100）→ 颜色：≥ 50 红、≥ 20 橙，其余绿。
pub fn cpu_color(cpu: f32) -> ui::Token {
    if cpu >= 50. {
        ui::DESTRUCTIVE
    } else if cpu >= 20. {
        ui::WARNING
    } else {
        ui::SUCCESS
    }
}

/// 颜色做文字（多半在同色浅底上）时用的颜色：绿、橙、红换成文字用的那一号，主色换成深 / 浅蓝。
pub fn readable(color: ui::Token) -> ui::Token {
    match color {
        ui::SUCCESS => ui::SUCCESS_FOREGROUND,
        ui::WARNING => ui::WARNING_FOREGROUND,
        ui::DESTRUCTIVE => ui::DESTRUCTIVE_FOREGROUND,
        ui::PRIMARY => ui::ACCENT_FOREGROUND,
        other => other,
    }
}

/// 进程 CPU% 怎么标：没在用的淡色数字，用一点（≥ 0.5）数字上色，用得多（≥ 5）再加同色浅底（[`CPU_TINT`]）。颜色见 [`cpu_color`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuLevel {
    Idle,
    Light(ui::Token),
    Busy(ui::Token),
}

pub fn cpu_level(cpu: f32) -> CpuLevel {
    let color = cpu_color(cpu);
    if cpu < 0.5 {
        CpuLevel::Idle
    } else if cpu < 5. {
        CpuLevel::Light(color)
    } else {
        CpuLevel::Busy(color)
    }
}

/// 常驻内存多的标紫：≥ 100 MB 加粗一点，≥ 1 GB 再粗。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryLevel {
    Normal,
    Large,
    Huge,
}

pub fn memory_level(rss_kb: u64) -> MemoryLevel {
    match rss_kb {
        kb if kb >= 1024 * 1024 => MemoryLevel::Huge,
        kb if kb >= 100 * 1024 => MemoryLevel::Large,
        _ => MemoryLevel::Normal,
    }
}

/// 用户列的几种人：root、登录用户自己、macOS 的系统账号（`_windowserver` 这类）、其他。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserKind {
    Root,
    Me,
    Service,
    Other,
}

pub fn user_kind(user: &str, me: Option<&str>) -> UserKind {
    if user == "root" {
        UserKind::Root
    } else if me == Some(user) {
        UserKind::Me
    } else if user.starts_with('_') {
        UserKind::Service
    } else {
        UserKind::Other
    }
}

/// 圆环上的点：`turn` 是第几圈（0 在 12 点钟，顺时针，0.25 在 3 点钟）。
pub fn ring_point(center: Point<Pixels>, radius: Pixels, turn: f32) -> Point<Pixels> {
    let angle = turn * TAU;
    point(center.x + radius * angle.sin(), center.y - radius * angle.cos())
}

/// 监听地址的范围：所有地址（对外开放）、只有本机、某个网卡。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressScope {
    All,
    Loopback,
    Specific,
}

pub fn address_scope(address: &str) -> AddressScope {
    let address = address.trim_start_matches('[').trim_end_matches(']');
    match address {
        "*" | "0.0.0.0" | "::" => AddressScope::All,
        "::1" | "localhost" => AddressScope::Loopback,
        a if a.starts_with("127.") => AddressScope::Loopback,
        _ => AddressScope::Specific,
    }
}

/// 命令行拆成 目录 / 程序名 / 参数。内核线程（[kworker/…]）整个当名字。知道程序路径（`program`，macOS 的 comm）时
/// 按它切，路径里的空格（`Google Chrome Helper`）不算参数的开始；不知道就切在第一个空格。
pub fn command_parts<'a>(command: &'a str, program: Option<&str>) -> (&'a str, &'a str, &'a str) {
    if command.starts_with('[') {
        return ("", command, "");
    }
    let end = program
        .filter(|program| command.starts_with(program))
        .map_or_else(|| command.find(char::is_whitespace).unwrap_or(command.len()), str::len);
    let (first, args) = command.split_at(end);
    let (dir, name) = first.split_at(first.rfind('/').map_or(0, |i| i + 1));
    if name.is_empty() { ("", first, args) } else { (dir, name, args) }
}

/// 进程名：程序名去掉路径，冒号结尾的（「nginx: worker」）去掉冒号；内核线程整个。
/// node / bun / python 跑的脚本是认得出的软件（`node /usr/local/bin/gemini`）时是脚本名，见 [`known_script`]。
pub fn process_name(command: &str, program: Option<&str>) -> String {
    let (_, name, args) = command_parts(command, program);
    let name = name.trim_end_matches(':');
    if name.is_empty() {
        return command.to_string();
    }
    known_script(name, args).unwrap_or(name).to_string()
}

/// 解释器跑的脚本（参数里第一个不是选项的，去掉路径和 .js / .mjs / .cjs / .py）认得出是哪个软件时的脚本名：
/// Gemini CLI、Qwen Code 这类 node 写的命令行工具不改进程名，不认脚本的话只是一个 node。认不出的（`node server.js`）不算。
fn known_script<'a>(interpreter: &str, args: &'a str) -> Option<&'a str> {
    let interpreter = interpreter.to_lowercase();
    let bare = interpreter.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if !matches!(bare, "node" | "nodejs" | "bun" | "python") {
        return None;
    }
    let script = args.split_whitespace().find(|arg| !arg.starts_with('-'))?;
    let file = script.rsplit('/').next()?;
    let stem = [".js", ".mjs", ".cjs", ".py"].iter().find_map(|ext| file.strip_suffix(ext)).unwrap_or(file);
    brands::brand_of(stem).is_some().then_some(stem)
}

/// 搜索词在 `text` 里出现的每一处（字节范围，落在字符边界上）：不分大小写，中文也行，不重叠。空的搜索词没有。
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let needle: Vec<char> = query.trim().chars().map(fold).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = text.char_indices().map(|(at, c)| (at, fold(c))).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()].iter().map(|(_, c)| *c).eq(needle.iter().copied()) {
            let end = chars.get(i + needle.len()).map_or(text.len(), |(at, _)| *at);
            out.push(chars[i].0..end);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// 搜索命中的那几段铺一层琥珀色浅底、字换正文色（深色外观下淡色字压在底上不够清楚），同终端里查找的黄底。
fn highlighted(text: impl Into<SharedString>, query: &str) -> StyledText {
    let text: SharedString = text.into();
    let ranges = match_ranges(&text, query);
    let style = HighlightStyle {
        background_color: Some(theme::alpha(ui::WARNING, MATCH_TINT[usize::from(theme::is_dark())])),
        color: Some(theme::color(ui::FOREGROUND)),
        ..Default::default()
    };
    StyledText::new(text).with_highlights(ranges.into_iter().map(move |range| (range, style)))
}

/// 读列表失败的说明：没权限时说怎么办。
pub fn list_error_message(error: &str) -> String {
    if services::needs_root(error) {
        return tr!("没有权限：在终端里 sudo -i 后再看。").into();
    }
    tr!("读取失败：{}", error.trim())
}

fn action_label(action: ServiceAction) -> &'static str {
    match action {
        ServiceAction::Start => tr!("启动"),
        ServiceAction::Stop => tr!("停止"),
        ServiceAction::Restart => tr!("重启"),
        ServiceAction::Enable => tr!("设为开机启动"),
        ServiceAction::Disable => tr!("取消开机启动"),
    }
}

// ---------------------------------------------------------------- 状态

impl SystemPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = kit::input(|| tr!("搜索…"), window, cx);
        let subscription = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Change => {
                this.rows.reset();
                cx.notify();
            }
            InputEvent::Focus => {
                this.search_focused = true;
                cx.notify();
            }
            InputEvent::Blur => {
                this.search_focused = false;
                cx.notify();
            }
            _ => {}
        });
        Self {
            machine: None,
            me: None,
            active: false,
            view: View::Processes,
            overview: None,
            processes: None,
            ports: None,
            services: None,
            disk: DiskState::default(),
            icons: HashMap::new(),
            app_names: HashMap::new(),
            expanded_apps: Default::default(),
            responsible: HashMap::new(),
            no_systemd: false,
            busy: None,
            error: None,
            sort: (SortKey::Cpu, true),
            port_sort: (PortSortKey::Port, false),
            service_sort: (ServiceSortKey::Status, false),
            search,
            search_focused: false,
            menu: None,
            request: 0,
            rows: kit::Rows::default(),
            focus: cx.focus_handle(),
            _refresh: None,
            _subscription: subscription,
        }
    }

    /// 切标签（远程会话 / 本机）/ 断开（None）。几个本机标签看的是同一台机器，在它们之间切不重新读。
    /// `me` 是那台机器的登录用户。
    pub fn set_machine(&mut self, machine: Option<Machine>, me: Option<String>, cx: &mut Context<Self>) {
        if self.me != me {
            self.me = me;
            cx.notify();
        }
        if self.machine == machine {
            return;
        }
        // 换到别的会话：这台机器上的磁盘扫描停下（不在后台接着占资源）
        self.stop_scans(cx);
        self.view = view_on(machine.as_ref(), self.view);
        self.machine = machine;
        self.overview = None;
        self.processes = None;
        self.ports = None;
        self.services = None;
        // 扫描结果按机器留着，分区列表随机器换
        self.disk.filesystems = None;
        self.no_systemd = false;
        self.busy = None;
        self.error = None;
        self.menu = None;
        self.request += 1;
        self.restart(cx);
        cx.notify();
    }

    /// 「系统」标签显示 / 隐藏：显示时立刻刷新并每 3 秒刷新一次，隐藏就停。
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        // 显示时的焦点由工作区给（`Workspace::refocus`：有框开着时不挪）
        if !active {
            self.menu = None;
            // 切到终端 / 文件管理：磁盘扫描停下
            self.stop_scans(cx);
        }
        self.restart(cx);
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        self._refresh = (self.active && self.machine.is_some()).then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                        break;
                    }
                    cx.background_executor().timer(REFRESH_INTERVAL).await;
                }
            })
        });
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(machine) = self.machine.clone() else { return };
        self.request += 1;
        let request = self.request;
        let view = self.view;
        let target = machine.clone();
        let overview = backend::spawn(async move { server_info::overview(&backend::get().state, &target).await });
        let list = backend::spawn(async move {
            let state = &backend::get().state;
            match view {
                View::Processes => {
                    let processes = server_info::processes(state, &machine).await?;
                    // 本机：查每个进程的负责进程，自己不在应用里的归到启动它的应用（`apps::list`）
                    let responsible = if machine == Machine::Local { apps::responsible_pids(processes.iter().map(|p| p.pid)) } else { HashMap::new() };
                    Ok(Loaded::Processes(processes, responsible))
                }
                View::Ports => server_info::ports(state, &machine).await.map(Loaded::Ports),
                View::Services => services::services(state, &machine).await.map(Loaded::Services),
                View::Disk => sshl_core::disk_usage::filesystems(state, &machine).await.map(Loaded::Filesystems),
            }
        });
        cx.spawn(async move |this, cx| {
            let (overview, list) = futures::join!(overview, list);
            let _ = this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                if let Some(Ok(overview)) = overview {
                    this.overview = Some(overview);
                }
                match list {
                    Some(Ok(loaded)) => {
                        this.error = None;
                        match loaded {
                            Loaded::Processes(processes, responsible) => {
                                this.processes = Some(processes);
                                this.responsible = responsible;
                            }
                            Loaded::Ports(ports) => this.ports = Some(ports),
                            Loaded::Services(services) => {
                                this.no_systemd = services.is_none();
                                this.services = Some(services.unwrap_or_default());
                            }
                            Loaded::Filesystems(filesystems) => this.disk.filesystems = Some(filesystems),
                        }
                    }
                    Some(Err(error)) => this.error = Some(error.to_string().into()),
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        if self.view == view {
            return;
        }
        if self.view == View::Disk {
            self.stop_scans(cx);
        }
        self.view = view;
        self.menu = None;
        self.rows.reset();
        self.restart(cx);
        cx.notify();
    }

    fn set_sort(&mut self, column: SortColumn, cx: &mut Context<Self>) {
        // 再点同一列反过来；进程的 CPU、内存默认从大到小，其余从小到大
        match column {
            SortColumn::Process(key) => {
                self.sort = if self.sort.0 == key { (key, !self.sort.1) } else { (key, matches!(key, SortKey::Cpu | SortKey::Memory)) };
            }
            SortColumn::Port(key) => {
                self.port_sort = if self.port_sort.0 == key { (key, !self.port_sort.1) } else { (key, false) };
            }
            SortColumn::Service(key) => {
                self.service_sort = if self.service_sort.0 == key { (key, !self.service_sort.1) } else { (key, false) };
            }
        }
        cx.notify();
    }

    fn kill(&mut self, pid: u32, name: String, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(machine) = self.machine.clone() else { return };
        let (title, detail) = if force {
            (tr!("强制结束 {}？", name), tr!("PID {} · kill -9：进程来不及收尾（不保存、不清理），只在普通结束不管用时用。", pid))
        } else {
            (tr!("结束 {}？", name), tr!("PID {} · 发送 TERM，进程可以先收尾再退出。", pid))
        };
        let mut options = Options::new(title).message(detail).confirm(if force { tr!("强制结束") } else { tr!("结束进程") });
        options.destructive = true;
        let answer = prompts::confirm(window, cx, options);
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await.unwrap_or(false) {
                return;
            }
            let result = backend::spawn(async move { server_info::kill(&backend::get().state, &machine, pid, force).await }).await;
            // 给进程一点时间退出，再刷新列表
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Some(Ok(())) => {}
                    Some(Err(error)) if services::needs_root(&error.to_string()) => {
                        let command = format!("sudo {}", server_info::kill_command(pid, force));
                        this.offer_sudo(tr!("结束 {} 需要 root 权限", name), None, command, window, cx);
                    }
                    Some(Err(error)) => prompts::alert_later(window, cx, Options::new(tr!("没能结束 {}", name)).message(kill_error_message(&error.to_string()))),
                    None => {}
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    /// 没权限：问要不要改在终端里用 sudo 执行（终端里输密码，或点「填充密码」）。
    fn offer_sudo(&mut self, title: String, note: Option<&str>, command: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut options = Options::new(title).detail(command.clone()).confirm(tr!("在终端里用 sudo 执行"));
        if let Some(note) = note {
            options = options.message(note.to_string());
        }
        let answer = prompts::confirm(window, cx, options);
        cx.spawn(async move |this, cx| {
            if answer.await.unwrap_or(false) {
                let _ = this.update(cx, |_, cx| cx.emit(SystemPanelEvent::RunInTerminal(command)));
            }
        })
        .detach();
    }

    /// 启动 / 停止 / 重启 / 开关开机启动。停止和重启先确认（停错了可能把正在用的东西断掉）。
    fn service_action(&mut self, service: ServiceInfo, action: ServiceAction, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(machine) = self.machine.clone() else { return };
        let name = service.name().to_string();
        let confirm = matches!(action, ServiceAction::Stop | ServiceAction::Restart).then(|| {
            let mut options = Options::new(tr!("{}{}？", action_label(action), name)).confirm(action_label(action));
            if !service.description.is_empty() {
                options = options.message(service.description.clone());
            }
            options.destructive = action == ServiceAction::Stop;
            prompts::confirm(window, cx, options)
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Some(answer) = confirm
                && !answer.await.unwrap_or(false)
            {
                return;
            }
            let unit = service.unit.clone();
            let _ = this.update(cx, |this, cx| {
                this.busy = Some(unit.clone());
                cx.notify();
            });
            let result = backend::spawn(async move { services::act(&backend::get().state, &machine, action, unit).await }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Some(Ok(())) | None => {}
                    Some(Err(error)) if services::needs_root(&error.to_string()) => {
                        let command = services::terminal_command(action, &service.unit);
                        this.offer_sudo(tr!("{} {} 需要 root 权限", action_label(action), name), None, command, window, cx);
                    }
                    Some(Err(error)) => prompts::alert_later(window, cx, Options::new(tr!("没能{}{}", action_label(action), name)).message(error.to_string().trim().to_string())),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    fn service_logs(&mut self, service: &ServiceInfo, cx: &mut Context<Self>) {
        self.menu = None;
        cx.emit(SystemPanelEvent::RunInTerminal(services::logs_command(&service.unit)));
    }

    /// 去后台取这些应用的图标和名字（一批一起取，同一个只取一次），取到了重画。取不到的记成没有，不再取。
    fn request_icons(&mut self, apps: Vec<String>, cx: &mut Context<Self>) {
        for app in &apps {
            self.icons.insert(app.clone(), AppIcon::Loading);
        }
        let drawn = cx
            .background_executor()
            .spawn(async move { apps.into_iter().map(|app| (app_icons::icon_png(&app), app_icons::display_name(&app), app)).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| {
            let drawn = drawn.await;
            let _ = this.update(cx, |this, cx| {
                for (png, name, app) in drawn {
                    if let Some(name) = name {
                        this.app_names.insert(app.clone(), name);
                    }
                    let icon = match png {
                        Some(png) => AppIcon::Ready(Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png))),
                        None => AppIcon::Missing,
                    };
                    this.icons.insert(app, icon);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_menu(&mut self, position: Point<Pixels>, target: MenuTarget, cx: &mut Context<Self>) {
        self.menu = Some(MenuState { position, target });
        cx.notify();
    }

    // ---------------------------------------------------------------- 渲染

    /// 四项指标在同一张卡片里，每项一段同色浅底 + 圆环（已运行是时钟）。用量高了那一段整个换成橙 / 红。
    fn overview(&self) -> impl IntoElement {
        let o = self.overview.clone().unwrap_or_default();
        let dark = usize::from(theme::is_dark());
        let segment = |color: ui::Token, title: &'static str, value: String, sub: Option<String>, ratio: Option<f32>, clock: bool| {
            let color = ratio.map_or(color, |ratio| usage_color(ratio, color));
            let gauge = div().relative().size(px(GAUGE_SIZE)).flex_shrink_0();
            let gauge = if clock {
                gauge
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme::alpha(color, RING_TRACK[dark]))
                    .text_color(theme::color(color))
                    .child(kit::icon_sized(Icon::Clock, 24.))
            } else {
                let (stroke, track) = (theme::color(color), theme::alpha(color, RING_TRACK[dark]));
                let fill = ratio.unwrap_or(0.);
                gauge
                    .child(canvas(|_, _, _| {}, move |bounds, _, window, _| paint_ring(bounds, fill, stroke, track, window)).absolute().top_0().left_0().size_full())
                    .when_some(ratio, |d, ratio| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(13.))
                                .font_weight(FontWeight::BOLD)
                                .font_features(TABULAR.clone())
                                .text_color(theme::color(readable(color)))
                                .child(format!("{:.0}%", ratio * 100.)),
                        )
                    })
            };
            div()
                .flex_1()
                .min_w(px(SEGMENT_MIN_WIDTH))
                .flex()
                .items_center()
                .gap(px(14.))
                .px_4()
                .py(px(14.))
                .rounded(px(10.))
                .bg(theme::alpha(color, SEGMENT_TINT[dark]))
                .child(gauge)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(title))
                        // 数值不截断也不从中间折开：负载的三个数各是一段，窄了整段换到下一行
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_x(px(10.))
                                .text_size(px(19.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .font_features(TABULAR.clone())
                                .children(value.split("  ").map(|part| div().whitespace_nowrap().child(part.to_string()))),
                        )
                        .when_some(sub, |d, sub| d.child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(sub))),
                )
        };
        let dash = || "—".to_string();
        let load = o.load.map(|l| format!("{:.2}  {:.2}  {:.2}", l[0], l[1], l[2])).unwrap_or_else(dash);
        let cpus = o.cpus.map(|c| tr!("{} 核", c));
        let load_ratio = o.load.zip(o.cpus).map(|(l, c)| l[0] / c.max(1) as f32);
        let (memory, memory_sub, memory_ratio) = match (o.mem_total, o.mem_available) {
            (Some(total), Some(available)) => {
                let used = total.saturating_sub(available);
                (format_bytes(used), Some(tr!("共 {}", format_bytes(total))), Some(used as f32 / total.max(1) as f32))
            }
            (Some(total), None) => (format_bytes(total), Some(tr!("总量").into()), None),
            _ => (dash(), None, None),
        };
        let (disk, disk_sub, disk_ratio) = match (o.disk_total, o.disk_used) {
            (Some(total), Some(used)) => (format_bytes(used), Some(tr!("共 {} · /", format_bytes(total))), Some(used as f32 / total.max(1) as f32)),
            _ => (dash(), None, None),
        };
        let uptime = o.uptime_secs.map(format_uptime).unwrap_or_else(dash);
        // 两段一组：宽时一排四段，窄了两组各占一排（2 × 2），再窄一段一排；不会剩一段单独铺满一排
        let pair = |a: Div, b: Div| div().flex_1().flex_basis(px(2. * SEGMENT_MIN_WIDTH + 8.)).flex().flex_wrap().gap(px(8.)).child(a).child(b);
        div()
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .gap(px(8.))
            .p(px(8.))
            .rounded(px(14.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .child(pair(
                segment(ui::PRIMARY, tr!("负载（1 / 5 / 15 分钟）"), load, cpus, load_ratio, false),
                segment(ui::VIOLET, tr!("内存已用"), memory, memory_sub, memory_ratio, false),
            ))
            .child(pair(
                segment(ui::TEAL, tr!("磁盘已用"), disk, disk_sub, disk_ratio, false),
                segment(ui::SUCCESS_FOREGROUND, tr!("已运行"), uptime, None, None, true),
            ))
    }

    fn header_cell(&self, label: &'static str, key: Option<SortColumn>, width: Option<f32>, align: Align, cx: &mut Context<Self>) -> AnyElement {
        let descending = key.and_then(|key| match key {
            SortColumn::Process(key) => (key == self.sort.0).then_some(self.sort.1),
            SortColumn::Port(key) => (key == self.port_sort.0).then_some(self.port_sort.1),
            SortColumn::Service(key) => (key == self.service_sort.0).then_some(self.service_sort.1),
        });
        div()
            .id(label)
            .px_3()
            .flex()
            .items_center()
            .gap(px(3.))
            .map(|d| match width {
                Some(w) => d.w(px(w)).flex_shrink_0(),
                None => d.flex_1().min_w_0(),
            })
            .map(|d| match align {
                Align::Start => d,
                Align::End => d.justify_end(),
                Align::Center => d.justify_center(),
            })
            .child(label)
            // 按这一列排序：蓝字 + 箭头
            .when_some(descending, |d, descending| {
                d.text_color(theme::color(ui::PRIMARY)).child(kit::icon_sized(if descending { Icon::ArrowDown } else { Icon::ArrowUp }, 12.))
            })
            .when_some(key, |d, key| {
                d.role(Role::Button)
                    .aria_label(SharedString::from(tr!("按{}排序", label)))
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::color(ui::FOREGROUND)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_sort(key, cx)))
            })
            .into_any_element()
    }

    fn kill_button(&self, id: (&'static str, usize), pid: u32, name: String, cx: &mut Context<Self>) -> AnyElement {
        Self::hover_button(id, tr!("结束"), tr!("结束 {}", name), ui::DESTRUCTIVE)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.kill(pid, name.clone(), false, window, cx);
            }))
            .into_any_element()
    }

    /// 进程列表的行：本机按应用合起来（`apps::list`），远程按进程树合（`apps::tree`：nginx 的 master 和 worker 一行）。
    fn process_rows(&self, query: &str) -> Vec<ProcessRow> {
        let processes = self.processes.as_deref().unwrap_or(&[]);
        let me = self.me.as_deref();
        if self.machine != Some(Machine::Local) {
            return apps::tree(processes, query, self.sort, &self.expanded_apps)
                .into_iter()
                .map(|row| match row {
                    Listed::Process(p) => ProcessRow::remote(p, me, false),
                    Listed::Member(p) => ProcessRow::remote(p, me, true),
                    Listed::App(group, open) => ProcessRow::app(group, open, me),
                })
                .collect();
        }
        apps::list(processes, query, self.sort, &self.expanded_apps, &self.app_names, &self.responsible)
            .into_iter()
            .map(|row| match row {
                Listed::Process(p) => ProcessRow::local(p, me, false),
                Listed::Member(p) => ProcessRow::local(p, me, true),
                Listed::App(group, open) => ProcessRow::app(group, open, me),
            })
            .collect()
    }

    /// `rows` 是从第 `start` 行开始的几行（整张表在 `render` 里算一次）。
    fn render_process_rows(&mut self, rows: Vec<ProcessRow>, start: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        // 还没有图标的应用去后台取（取到了重画）
        let missing: Vec<String> = rows
            .iter()
            .filter_map(|row| match &row.icon {
                Some(RowIcon::App(app)) if !self.icons.contains_key(app) => Some(app.clone()),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !missing.is_empty() {
            self.request_icons(missing, cx);
        }
        let me = self.me.clone();
        rows.into_iter()
            .enumerate()
            .map(|(i, row)| {
                let ix = start + i;
                let ProcessRow { pid, user, cpu, rss_kb, name, detail, command, icon, app, member, kernel } = row;
                let (menu_name, menu_command) = (name.clone(), command);
                let line = table_row(("process", ix)).on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.open_menu(event.position, MenuTarget::Process { pid, name: menu_name.clone(), command: Some(menu_command.clone()) }, cx);
                    }),
                );
                // 有好几个进程的应用：点一下展开 / 收起
                let toggle = app.as_ref().filter(|(_, count, _)| *count > 1).map(|(bundle, _, open)| (bundle.clone(), *open));
                let line = match &toggle {
                    Some((bundle, _)) => {
                        let bundle = bundle.clone();
                        line.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_app(&bundle, cx)))
                    }
                    None => line,
                };
                let icon = icon.map(|icon| match icon {
                    RowIcon::App(app) => match self.icons.get(&app) {
                        Some(AppIcon::Ready(image)) => ProcessIcon::App(Some(image.clone())),
                        Some(AppIcon::Missing) => ProcessIcon::Glyph(Icon::Settings),
                        _ => ProcessIcon::App(None),
                    },
                    RowIcon::Brand(brand) => ProcessIcon::Brand(brand),
                    RowIcon::Glyph(glyph) => ProcessIcon::Glyph(glyph),
                });
                line.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        // 展开的应用里的进程缩进，名字和应用名对齐
                        .when(member, |d| d.pl(px(12. + ICON_SIZE + 7.)))
                        // 图标没取到时留着位置，名字不跳
                        .when_some(icon, |d, icon| {
                            let slot = || div().size(px(ICON_SIZE)).flex_shrink_0().mr(px(-3.));
                            d.child(match icon {
                                ProcessIcon::App(Some(image)) => img(image).size(px(ICON_SIZE)).flex_shrink_0().mr(px(-3.)).into_any_element(),
                                ProcessIcon::App(None) => slot().into_any_element(),
                                ProcessIcon::Brand(brand) => slot()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(svg().path(brand.path()).size(px(BRAND_SIZE)).text_color(theme::color(brands::color(brand, theme::is_dark()))))
                                    .into_any_element(),
                                ProcessIcon::Glyph(glyph) => slot()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(theme::alpha(ui::MUTED_FOREGROUND, 0.7))
                                    .child(kit::icon_sized(glyph, 17.))
                                    .into_any_element(),
                            })
                        })
                        // 名字放全（比整格还长就折行），剩下的给后面那段
                        .child(
                            div()
                                .flex_shrink_0()
                                .max_w_full()
                                .font_weight(if member { FontWeight::MEDIUM } else { FontWeight::SEMIBOLD })
                                .when(kernel, |d| d.text_color(theme::color(ui::MUTED_FOREGROUND)))
                                .child(highlighted(name.clone(), &query)),
                        )
                        // 后面那段（远程是完整命令行，可能几千字）整段放得下才显示，不截成半截：一行高的能换行的格子，前面垫一个零宽的，
                        // 放不下时它整段换到第二行、被裁掉。藏起来时悬停这块空白看全文，右键也能复制。
                        .when_some(detail, |d, detail| {
                            let tip: SharedString = detail.clone().into();
                            d.child(
                                div()
                                    .id(("process-detail", ix))
                                    .min_w_0()
                                    .h(px(DETAIL_LINE))
                                    .overflow_hidden()
                                    .flex()
                                    .flex_wrap()
                                    .content_start()
                                    .child(div().w(px(0.)).h(px(DETAIL_LINE)))
                                    .child(
                                        div()
                                            .whitespace_nowrap()
                                            .line_height(px(DETAIL_LINE))
                                            .when(app.is_none(), |d| d.font_family(MONO).text_size(px(11.)))
                                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                                            .child(highlighted(detail, &query)),
                                    )
                                    .tooltip(move |window, cx| kit::tooltip(tip.clone(), window, cx)),
                            )
                        })
                        .when_some(toggle, |d, (_, open)| {
                            d.child(kit::icon_sized(if open { Icon::ChevronDown } else { Icon::ChevronRight }, 12.).text_color(theme::color(ui::MUTED_FOREGROUND)))
                        }),
                )
                .child(div().w(px(USER_WIDTH)).flex_shrink_0().px_3().flex().child(user_label(&user, me.as_deref(), &query)))
                .child(
                    div()
                        .w(px(MEMORY_WIDTH))
                        .flex_shrink_0()
                        .px_3()
                        .flex()
                        .justify_end()
                        .font_features(TABULAR.clone())
                        .map(|d| match memory_level(rss_kb) {
                            MemoryLevel::Normal => d,
                            MemoryLevel::Large => d.text_color(theme::color(ui::VIOLET)).font_weight(FontWeight::SEMIBOLD),
                            MemoryLevel::Huge => d.text_color(theme::color(ui::VIOLET)).font_weight(FontWeight::BOLD),
                        })
                        .child(format_bytes(rss_kb * 1024)),
                )
                .child(div().w(px(CPU_WIDTH)).flex_shrink_0().px_3().flex().justify_end().font_features(TABULAR.clone()).child(cpu_label(cpu)))
                .child(
                    div()
                        .w(px(PID_WIDTH))
                        .flex_shrink_0()
                        .px_3()
                        .flex()
                        .justify_end()
                        .font_features(TABULAR.clone())
                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                        .child(highlighted(pid.to_string(), &query)),
                )
                .child(div().w(px(ACTION_WIDTH)).flex_shrink_0().flex().justify_center().child(self.kill_button(("kill-process", ix), pid, name, cx)))
                .into_any_element()
            })
            .collect()
    }

    /// 展开 / 收起一个应用的进程。
    fn toggle_app(&mut self, bundle: &str, cx: &mut Context<Self>) {
        if !self.expanded_apps.remove(bundle) {
            self.expanded_apps.insert(bundle.to_string());
        }
        cx.notify();
    }

    fn render_port_rows(&mut self, rows: Vec<PortInfo>, start: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        rows.into_iter()
            .enumerate()
            .map(|(i, p)| {
                let ix = start + i;
                let name = p.process.clone().unwrap_or_else(|| "—".into());
                table_row(("port", ix))
                    .when_some(p.pid, |d, pid| {
                        let menu_name = name.clone();
                        d.on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_menu(event.position, MenuTarget::Process { pid, name: menu_name.clone(), command: None }, cx);
                        }))
                    })
                    .child(div().w(px(64.)).flex_shrink_0().px_3().flex().child(if p.proto.eq_ignore_ascii_case("udp") {
                        pill(ui::VIOLET, theme::alpha(ui::VIOLET, 0.1)).child(p.proto.to_uppercase())
                    } else {
                        pill(ui::ACCENT_FOREGROUND, theme::color(ui::ACCENT)).child(p.proto.to_uppercase())
                    }))
                    .child(
                        div()
                            .w(px(86.))
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .justify_end()
                            .font_features(TABULAR.clone())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::color(ui::PRIMARY))
                            .child(highlighted(p.port.to_string(), &query)),
                    )
                    .child(
                        div()
                            .w(px(ADDRESS_WIDTH))
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().min_w_0().font_family(MONO).text_color(theme::alpha(ui::FOREGROUND, 0.85)).child(highlighted(p.address.clone(), &query)))
                            .map(|d| match address_scope(&p.address) {
                                // 所有地址 = 对外开放，橙色提醒；只听本机的灰色
                                AddressScope::All => d.child(pill(ui::WARNING_FOREGROUND, theme::alpha(ui::WARNING, 0.12)).child(tr!("所有地址"))),
                                AddressScope::Loopback => d.child(pill(ui::MUTED_FOREGROUND, theme::color(ui::MUTED)).child(trc!("监听地址", "本机"))),
                                AddressScope::Specific => d,
                            }),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .justify_end()
                            .font_features(TABULAR.clone())
                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                            .child(highlighted(p.pid.map(|pid| pid.to_string()).unwrap_or_else(|| "—".into()), &query)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .map(|d| if p.pid.is_some() { d.font_weight(FontWeight::SEMIBOLD) } else { d.text_color(theme::color(ui::MUTED_FOREGROUND)) })
                            .child(if p.pid.is_some() { highlighted(name.clone(), &query) } else { StyledText::new(tr!("看不到进程（需要 root）")) }),
                    )
                    .child(
                        div()
                            .w(px(ACTION_WIDTH))
                            .flex_shrink_0()
                            .flex()
                            .justify_center()
                            .when_some(p.pid, |d, pid| d.child(self.kill_button(("kill-port", ix), pid, name.clone(), cx))),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

impl SystemPanel {
    /// 行尾悬停才出现的文字按钮（结束、启动、停止…）：同色浅底，按上去再深一点。
    fn hover_button(id: (&'static str, usize), label: &'static str, aria: String, color: ui::Token) -> Stateful<Div> {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(SharedString::from(aria))
            .px_2()
            .py(px(2.))
            .rounded(px(6.))
            .text_xs()
            .text_color(theme::color(readable(color)))
            .bg(theme::alpha(color, 0.08))
            .opacity(0.)
            .group_hover("system-row", |s| s.opacity(1.))
            .hover(move |s| s.bg(theme::alpha(color, 0.16)))
            .cursor_pointer()
            .child(label)
    }

    fn render_service_rows(&mut self, rows: Vec<ServiceInfo>, start: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        rows.into_iter()
            .enumerate()
            .map(|(i, service)| {
                let ix = start + i;
                let name = service.name().to_string();
                let busy = self.busy.as_deref() == Some(service.unit.as_str());
                let (status, color) = if busy { (tr!("处理中…"), ui::WARNING) } else { service_status(&service) };
                let running = service.is_running();
                let button = |id: &'static str, action: ServiceAction, color: ui::Token, cx: &mut Context<Self>| {
                    let target = service.clone();
                    Self::hover_button((id, ix), action_label(action), format!("{} {name}", action_label(action)), color).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.service_action(target.clone(), action, window, cx);
                        },
                    ))
                };
                let logs_target = service.clone();
                let menu_target = service.clone();
                let actions = div()
                    .w(px(150.))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .pr_2()
                    .when(!busy, |d| {
                        d.child(Self::hover_button(("service-logs", ix), tr!("日志"), tr!("{} 的日志", name), ui::MUTED_FOREGROUND).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.service_logs(&logs_target, cx);
                            },
                        )))
                        .map(|d| {
                            if running {
                                d.child(button("service-restart", ServiceAction::Restart, ui::PRIMARY, cx))
                                    .child(button("service-stop", ServiceAction::Stop, ui::DESTRUCTIVE, cx))
                            } else {
                                d.child(button("service-start", ServiceAction::Start, ui::PRIMARY, cx))
                            }
                        })
                    });
                table_row(("service", ix))
                    // 失败的服务整行淡红，一眼能看到
                    .when(color == ui::DESTRUCTIVE, |d| d.bg(theme::alpha(ui::DESTRUCTIVE, 0.05)))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.open_menu(event.position, MenuTarget::Service(menu_target.clone()), cx);
                    }))
                    .child(
                        div().w(px(STATUS_WIDTH)).flex_shrink_0().px_3().flex().child(
                            pill(readable(color), if color == ui::MUTED_FOREGROUND { theme::color(ui::MUTED) } else { theme::alpha(color, 0.12) })
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(div().size(px(6.)).rounded_full().bg(theme::color(color)))
                                .child(status),
                        ),
                    )
                    .child(div().w(px(220.)).flex_shrink_0().px_3().font_weight(FontWeight::SEMIBOLD).child(highlighted(name.clone(), &query)))
                    .child(div().flex_1().min_w_0().px_3().text_color(theme::color(ui::MUTED_FOREGROUND)).child(highlighted(service.description.clone(), &query)))
                    .child(
                        div()
                            .w(px(BOOT_WIDTH))
                            .flex_shrink_0()
                            .flex()
                            .justify_center()
                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                            .map(|d| match service.enabled.as_str() {
                                e if e.starts_with("enabled") => d.child(kit::icon_sized(Icon::Check, 14.).text_color(theme::color(ui::SUCCESS_FOREGROUND))),
                                "masked" => d.child(tr!("已屏蔽")),
                                _ => d,
                            }),
                    )
                    .child(actions)
                    .into_any_element()
            })
            .collect()
    }
}

/// 带底色的小标签（协议、地址范围、服务状态）。
fn pill(text: ui::Token, background: gpui::Hsla) -> gpui::Div {
    div()
        .flex_shrink_0()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(text))
        .bg(background)
}

/// 圆环：一圈同色浅轨道，上面从 12 点钟顺时针画用量那一段，两头是圆的。
fn paint_ring(bounds: Bounds<Pixels>, ratio: f32, color: Hsla, track: Hsla, window: &mut Window) {
    let stroke = px(RING_STROKE);
    let side = if bounds.size.width < bounds.size.height { bounds.size.width } else { bounds.size.height };
    let radius = (side - stroke) / 2.;
    let center = bounds.center();
    let radii = point(radius, radius);
    let at = |turn: f32| ring_point(center, radius, turn);
    let circle = |path: &mut PathBuilder| {
        path.move_to(at(0.));
        path.arc_to(radii, px(0.), false, true, at(0.5));
        path.arc_to(radii, px(0.), false, true, at(0.));
    };
    let mut path = PathBuilder::stroke(stroke);
    circle(&mut path);
    if let Ok(path) = path.build() {
        window.paint_path(path, track);
    }
    let ratio = ratio.clamp(0., 1.);
    if ratio <= 0. {
        return;
    }
    let mut path = PathBuilder::stroke(stroke);
    if ratio >= 1. {
        circle(&mut path);
    } else {
        path.move_to(at(0.));
        path.arc_to(radii, px(0.), ratio > 0.5, true, at(ratio));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
    if ratio < 1. {
        // PathBuilder 的线头是平的：两头各补一个圆点
        for turn in [0., ratio] {
            let cap = Bounds::centered_at(at(turn), size(stroke, stroke));
            window.paint_quad(gpui::fill(cap, color).corner_radii(stroke / 2.));
        }
    }
}

/// 表格的一行：细线分隔，鼠标停着时铺浅蓝；行尾按钮悬停才出来（group「system-row」）。名字长了折行，行跟着长高。
fn table_row(id: impl Into<ElementId>) -> Stateful<Div> {
    let hover = theme::alpha(ui::PRIMARY, ROW_HOVER[usize::from(theme::is_dark())]);
    div()
        .id(id)
        .group("system-row")
        .min_h(px(ROW_HEIGHT))
        .py(px(6.))
        .w_full()
        .flex()
        .items_center()
        .text_size(px(12.))
        .border_b_1()
        .border_color(theme::alpha(ui::BORDER, 0.6))
        .hover(move |s| s.bg(hover))
}

/// 表头：上下两条线，次要文字色。
fn table_header() -> Div {
    div()
        .h(px(32.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .border_t_1()
        .border_b_1()
        .border_color(theme::color(ui::BORDER))
}

/// 用户列：root 橙色标签、登录用户蓝色标签、系统账号淡色。标签往左挪出内边距，字和表头对齐。
fn user_label(user: &str, me: Option<&str>, query: &str) -> Div {
    let label = div().min_w_0().ml(px(-7.)).px(px(7.)).py(px(1.)).rounded(px(5.)).child(highlighted(user.to_string(), query));
    match user_kind(user, me) {
        UserKind::Root => label.bg(theme::alpha(ui::WARNING, 0.13)).text_color(theme::color(ui::WARNING_FOREGROUND)).font_weight(FontWeight::SEMIBOLD),
        UserKind::Me => label.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND)).font_weight(FontWeight::MEDIUM),
        UserKind::Service => label.text_color(theme::color(ui::MUTED_FOREGROUND)),
        UserKind::Other => label,
    }
}

/// CPU% 那一格（见 [`cpu_level`]）。
fn cpu_label(cpu: f32) -> Div {
    let value = format!("{cpu:.1}");
    let badge = || div().min_w(px(40.)).px(px(7.)).py(px(2.)).rounded(px(6.)).flex().justify_end().child(value.clone());
    match cpu_level(cpu) {
        CpuLevel::Idle => div().text_color(theme::color(ui::MUTED_FOREGROUND)).child(value),
        CpuLevel::Light(color) => badge().text_color(theme::color(readable(color))).font_weight(FontWeight::MEDIUM),
        CpuLevel::Busy(color) => {
            badge().bg(theme::alpha(color, CPU_TINT[usize::from(theme::is_dark())])).text_color(theme::color(readable(color))).font_weight(FontWeight::SEMIBOLD)
        }
    }
}

impl Render for SystemPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_string();
        let process_rows = self.processes.is_some().then(|| Rc::new(self.process_rows(&query)));
        let process_count = process_rows.as_ref().map(|rows| rows.len());
        let port_count = self.ports.as_deref().map(|p| filter_ports(p, &query, self.port_sort).len());
        let service_count = self.services.as_deref().filter(|_| !self.no_systemd).map(|s| filter_services(s, &query, self.service_sort).len());
        let disk_count = self.disk_count();
        let (count, loaded) = match self.view {
            View::Processes => (process_count.unwrap_or(0), self.processes.is_some()),
            View::Ports => (port_count.unwrap_or(0), self.ports.is_some()),
            View::Services => (service_count.unwrap_or(0), self.services.is_some()),
            View::Disk => (disk_count.unwrap_or(0), self.disk.filesystems.is_some()),
        };
        // 几个列表的标签：选中的实心蓝底，后面是条数
        let tabs = div()
            .id("system-view")
            .role(Role::TabList)
            .flex_shrink_0()
            .h(px(34.))
            .p(px(3.))
            .flex()
            .gap(px(2.))
            .rounded(px(10.))
            .bg(theme::color(ui::SECONDARY))
            .children(views_for(self.machine.as_ref()).iter().enumerate().map(|(index, &view)| {
                let (label, count) = match view {
                    View::Processes => (trc!("标签页", "进程"), process_count),
                    View::Ports => (trc!("标签页", "端口"), port_count),
                    View::Services => (trc!("标签页", "服务"), service_count),
                    View::Disk => (trc!("标签页", "磁盘"), disk_count),
                };
                let active = view == self.view;
                let aria: SharedString = match count {
                    Some(n) => format!("{label} {n}").into(),
                    None => label.into(),
                };
                div()
                    .id(("system-view", index))
                    .role(Role::Tab)
                    .aria_label(aria)
                    .h(px(28.))
                    .pl(px(12.))
                    .pr(px(if count.is_some() { 8. } else { 12. }))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(7.))
                    .text_size(px(14.))
                    .cursor_pointer()
                    .map(|t| {
                        if active {
                            t.bg(theme::color(ui::PRIMARY)).text_color(theme::color(ui::PRIMARY_FOREGROUND)).font_weight(FontWeight::SEMIBOLD)
                        } else {
                            t.text_color(theme::color(ui::MUTED_FOREGROUND)).hover(|s| s.text_color(theme::color(ui::FOREGROUND)))
                        }
                    })
                    .child(label)
                    .when_some(count, |t, n| {
                        t.child(
                            div()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(5.))
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .font_features(TABULAR.clone())
                                .bg(if active { theme::alpha(ui::PRIMARY_FOREGROUND, 0.22) } else { theme::alpha(ui::FOREGROUND, 0.07) })
                                .child(n.to_string()),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_view(view, cx)))
            }));
        let search = div()
            .flex_1()
            .min_w_0()
            .max_w(px(300.))
            .h(px(34.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::color(if self.search_focused { ui::RING } else { ui::BORDER }))
            .bg(theme::color(ui::CARD))
            .text_color(theme::color(ui::MUTED_FOREGROUND))
            .child(kit::icon_sized(Icon::Search, 14.))
            .child(div().flex_1().min_w_0().text_size(px(13.)).child(Input::new(&self.search)));
        // 停在这页时每 3 秒自动刷新，不放手动刷新按钮
        let toolbar = div().flex().items_center().gap_3().p_3().child(tabs).child(search);

        let header = match self.view {
            // 磁盘页自己画表头和内容（`render_disk`）
            View::Disk => table_header(),
            View::Processes => table_header()
                // 同 Mole：名字在最前，数字在后，PID 最后
                .child(self.header_cell(tr!("进程"), Some(SortColumn::Process(SortKey::Name)), None, Align::Start, cx))
                .child(self.header_cell(tr!("用户"), Some(SortColumn::Process(SortKey::User)), Some(USER_WIDTH), Align::Start, cx))
                .child(self.header_cell(tr!("内存"), Some(SortColumn::Process(SortKey::Memory)), Some(MEMORY_WIDTH), Align::End, cx))
                .child(self.header_cell("CPU%", Some(SortColumn::Process(SortKey::Cpu)), Some(CPU_WIDTH), Align::End, cx))
                .child(self.header_cell("PID", Some(SortColumn::Process(SortKey::Pid)), Some(PID_WIDTH), Align::End, cx))
                .child(div().w(px(ACTION_WIDTH)).flex_shrink_0()),
            View::Ports => table_header()
                .child(self.header_cell(tr!("协议"), Some(SortColumn::Port(PortSortKey::Proto)), Some(64.), Align::Start, cx))
                .child(self.header_cell(tr!("端口"), Some(SortColumn::Port(PortSortKey::Port)), Some(86.), Align::End, cx))
                .child(self.header_cell(tr!("地址"), Some(SortColumn::Port(PortSortKey::Address)), Some(ADDRESS_WIDTH), Align::Start, cx))
                .child(self.header_cell("PID", Some(SortColumn::Port(PortSortKey::Pid)), Some(80.), Align::End, cx))
                .child(self.header_cell(tr!("进程"), Some(SortColumn::Port(PortSortKey::Process)), None, Align::Start, cx))
                .child(div().w(px(ACTION_WIDTH)).flex_shrink_0()),
            View::Services => table_header()
                .child(self.header_cell(tr!("状态"), Some(SortColumn::Service(ServiceSortKey::Status)), Some(STATUS_WIDTH), Align::Start, cx))
                .child(self.header_cell(tr!("服务"), Some(SortColumn::Service(ServiceSortKey::Name)), Some(220.), Align::Start, cx))
                .child(self.header_cell(tr!("说明"), Some(SortColumn::Service(ServiceSortKey::Description)), None, Align::Start, cx))
                .child(self.header_cell(tr!("开机启动"), Some(SortColumn::Service(ServiceSortKey::Boot)), Some(BOOT_WIDTH), Align::Center, cx))
                .child(div().w(px(150.)).flex_shrink_0()),
        };

        let view_is_services = self.view == View::Services;
        let empty: Option<SharedString> = if self.machine.is_none() {
            Some(tr!("连接服务器后显示它的进程、端口和服务").into())
        } else if let Some(error) = &self.error {
            Some(list_error_message(error).into())
        } else if !loaded {
            Some(tr!("正在读取…").into())
        } else if view_is_services && self.no_systemd {
            Some(tr!("这台服务器没有 systemd").into())
        } else if count == 0 {
            Some(if query.trim().is_empty() { tr!("没有内容").into() } else { tr!("没有匹配的结果").into() })
        } else {
            None
        };
        let view = self.view;
        let body = match empty {
            Some(message) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme::color(ui::MUTED_FOREGROUND))
                .child(message)
                .into_any_element(),
            None => {
                // 行高跟着内容（名字长了折行）：整张表在这里算一次，每一行从里面取
                let table = match view {
                    View::Processes => Table::Processes(process_rows.clone().unwrap_or_default()),
                    View::Ports => Table::Ports(Rc::new(filter_ports(self.ports.as_deref().unwrap_or(&[]), &query, self.port_sort).into_iter().cloned().collect())),
                    View::Services => Table::Services(Rc::new(
                        filter_services(self.services.as_deref().unwrap_or(&[]), &query, self.service_sort).into_iter().cloned().collect(),
                    )),
                    View::Disk => Table::None,
                };
                self.rows.sync(count);
                let entity = cx.entity();
                list(self.rows.state(), move |ix, _, cx| {
                    entity
                        .update(cx, |this, cx| match &table {
                            Table::Processes(rows) => this.render_process_rows(rows.get(ix).cloned().into_iter().collect(), ix, cx),
                            Table::Ports(rows) => this.render_port_rows(rows.get(ix).cloned().into_iter().collect(), ix, cx),
                            Table::Services(rows) => this.render_service_rows(rows.get(ix).cloned().into_iter().collect(), ix, cx),
                            Table::None => Vec::new(),
                        })
                        .pop()
                        .unwrap_or_else(|| div().into_any_element())
                })
                .flex_1()
                .w_full()
                .into_any_element()
            }
        };

        let menu = self.menu.as_ref().map(|menu| {
            let weak = cx.entity().downgrade();
            let entries = match &menu.target {
                MenuTarget::Process { pid, name, command } => {
                    let (pid, name, command) = (*pid, name.clone(), command.clone());
                    let action = |force: bool| {
                        let (weak, name) = (weak.clone(), name.clone());
                        move |window: &mut Window, cx: &mut App| {
                            let _ = weak.update(cx, |this, cx| this.kill(pid, name.clone(), force, window, cx));
                        }
                    };
                    let mut entries = vec![
                        MenuEntry::Item(MenuItem::new(tr!("结束进程"), action(false)).destructive()),
                        MenuEntry::Item(MenuItem::new(tr!("强制结束（kill -9）"), action(true)).destructive()),
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuItem::new(tr!("复制 PID"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(pid.to_string())))),
                    ];
                    if let Some(command) = command {
                        entries.push(MenuEntry::Item(MenuItem::new(tr!("复制命令"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(command.clone())))));
                    }
                    entries
                }
                MenuTarget::Service(service) => {
                    let running = service.is_running();
                    let item = |action: ServiceAction| {
                        let (weak, service) = (weak.clone(), service.clone());
                        MenuItem::new(action_label(action), move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.service_action(service.clone(), action, window, cx));
                        })
                    };
                    // 只有 enabled / disabled 能开关（static、masked 这些 enable 不了）
                    let switchable = service.starts_at_boot() || service.enabled == "disabled";
                    let boot = if service.starts_at_boot() { ServiceAction::Disable } else { ServiceAction::Enable };
                    let (logs_weak, logs_service, unit) = (weak.clone(), service.clone(), service.unit.clone());
                    vec![
                        MenuEntry::Item(item(ServiceAction::Start).disabled(running)),
                        MenuEntry::Item(item(ServiceAction::Restart).disabled(!running)),
                        MenuEntry::Item(item(ServiceAction::Stop).disabled(!running).destructive()),
                        MenuEntry::Separator,
                        MenuEntry::Item(item(boot).disabled(!switchable)),
                        MenuEntry::Item(MenuItem::new(tr!("在终端里看日志"), move |_, cx| {
                            let _ = logs_weak.update(cx, |this, cx| this.service_logs(&logs_service, cx));
                        })),
                        MenuEntry::Item(MenuItem::new(tr!("复制名称"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(unit.clone())))),
                    ]
                }
                MenuTarget::Directory { path, folder, remote } => {
                    let (open_weak, open_folder, reveal, copied) = (weak.clone(), folder.clone(), path.clone(), path.clone());
                    let open = if *remote {
                        MenuItem::new(tr!("在文件管理中打开"), move |_, cx| {
                            let _ = open_weak.update(cx, |this, cx| {
                                this.menu = None;
                                cx.emit(SystemPanelEvent::OpenInFiles(open_folder.clone()));
                            });
                        })
                    } else {
                        MenuItem::new(tr!("在访达中显示"), move |_, cx| disk::reveal_in_finder(&reveal, cx))
                    };
                    let mut entries = vec![MenuEntry::Item(open.icon(Icon::FolderOpen))];
                    entries.push(MenuEntry::Item(MenuItem::new(tr!("复制路径"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))).icon(Icon::Copy)));
                    entries
                }
            };
            let dismiss = {
                let weak = weak.clone();
                move |_: &mut Window, cx: &mut App| {
                    let _ = weak.update(cx, |this, cx| {
                        this.menu = None;
                        cx.notify();
                    });
                }
            };
            let outside = dismiss.clone();
            deferred(
                anchored().position(menu.position).snap_to_window().child(
                    div().occlude().on_mouse_down_out(move |_, window, cx| outside(window, cx)).child(kit::menu_panel(entries, dismiss)),
                ),
            )
            .with_priority(2)
        });
        let _ = window;

        div()
            .id("system-panel")
            .key_context("SystemPanel")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(self.overview())
            .child(
                // 表头、各行铺满卡片的宽；底下留一点，最后一行的悬停底色不压到圆角上（gpui 只按矩形裁）
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(theme::color(ui::BORDER))
                    .bg(theme::color(ui::CARD))
                    .child(toolbar)
                    .child(if view == View::Disk {
                        self.render_disk(cx)
                    } else {
                        div().flex_1().min_h_0().flex().flex_col().pb_2().child(header).child(body).into_any_element()
                    }),
            )
            .when_some(menu, |d, menu| d.child(menu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, user: &str, cpu: f32, rss_kb: u64, command: &str) -> ProcessInfo {
        ProcessInfo { pid, ppid: 1, user: user.into(), cpu, mem: 0., rss_kb, command: command.into(), program: None }
    }

    #[test]
    fn formats_sizes_and_uptime_for_people() {
        // 数字和单位之间是不断行空格：哪里折行都不会拆成「3.3」「GB」两行
        assert_eq!(format_bytes(512), "512\u{a0}B");
        assert_eq!(format_bytes(1536), "1.5\u{a0}KB");
        assert_eq!(format_bytes(8 * 1024 * 1024 * 1024), "8.0\u{a0}GB");
        assert_eq!(format_bytes(200 * 1024 * 1024), "200\u{a0}MB");
        assert_eq!(format_uptime(30), "1 分钟");
        assert_eq!(format_uptime(3 * 3600 + 20 * 60), "3 小时 20 分");
        assert_eq!(format_uptime(12 * 86400 + 5 * 3600), "12 天 5 小时");
    }

    /// 远程的列表（`apps::tree`）里各自单独的进程：PID 顺序。
    fn listed(procs: &[ProcessInfo], query: &str, sort: (SortKey, bool)) -> Vec<u32> {
        apps::tree(procs, query, sort, &Default::default())
            .iter()
            .map(|row| match row {
                Listed::Process(p) | Listed::Member(p) => p.pid,
                Listed::App(group, _) => group.main.pid,
            })
            .collect()
    }

    /// 搜索高亮的位置：不分大小写、每一处都标、中文按字符切（不会切在一个字中间）。
    #[test]
    #[allow(clippy::single_range_in_vec_init)] // 就是只有一处命中
    fn search_matches_are_found_everywhere() {
        assert_eq!(match_ranges("nginx: worker process", "NGINX"), [0..5]);
        assert_eq!(match_ranges("redis-server redis", "redis"), [0..5, 13..18]);
        assert_eq!(match_ranges("微信 WeChat", "信"), [3..6]);
        assert_eq!(match_ranges("微信 WeChat", "chat"), [9..13]);
        assert_eq!(match_ranges("aaaa", "aa"), [0..2, 2..4], "不重叠");
        assert!(match_ranges("nginx", "").is_empty() && match_ranges("nginx", "  ").is_empty());
        assert!(match_ranges("ab", "abc").is_empty());
    }

    #[test]
    fn searches_and_sorts_processes() {
        let procs = vec![p(10, "root", 1.0, 500, "/usr/sbin/sshd -D"), p(20, "www-data", 30.0, 9000, "nginx: worker process"), p(30, "mysql", 5.0, 90000, "/usr/sbin/mysqld")];
        assert_eq!(listed(&procs, "", (SortKey::Cpu, true)), [20, 30, 10]);
        assert_eq!(listed(&procs, "", (SortKey::Memory, true)), [30, 20, 10]);
        assert_eq!(listed(&procs, "", (SortKey::Pid, false)), [10, 20, 30]);
        assert_eq!(listed(&procs, "NGINX", (SortKey::Cpu, true)), [20]);
        assert_eq!(listed(&procs, "mysql", (SortKey::Cpu, true)), [30], "按用户或命令都能搜");
        assert_eq!(listed(&procs, "10", (SortKey::Cpu, true)), [10]);
    }

    /// 「进程」那一列显示的是进程名，按它排也按名字排（不分大小写），不按路径：/usr/sbin/sshd 不该排在 nginx 前面。
    #[test]
    fn the_process_column_sorts_by_name() {
        let procs = vec![
            p(1, "root", 0., 0, "/usr/sbin/sshd -D"),
            p(2, "www-data", 0., 0, "nginx: worker process"),
            p(3, "qfdk", 0., 0, "/Applications/Adguard.app/Contents/MacOS/Adguard"),
            p(4, "qfdk", 0., 0, "./sshl-dev"),
        ];
        assert_eq!(listed(&procs, "", (SortKey::Name, false)), [3, 2, 1, 4]);
        assert_eq!(listed(&procs, "", (SortKey::Name, true)), [4, 1, 2, 3]);
    }

    #[test]
    fn searches_ports_by_number_name_or_pid() {
        let ports = vec![
            PortInfo { proto: "tcp".into(), address: "0.0.0.0".into(), port: 80, pid: Some(1234), process: Some("nginx".into()) },
            PortInfo { proto: "tcp".into(), address: "127.0.0.1".into(), port: 3306, pid: None, process: None },
        ];
        let by_port = (PortSortKey::Port, false);
        assert_eq!(filter_ports(&ports, "3306", by_port).len(), 1);
        assert_eq!(filter_ports(&ports, "Nginx", by_port)[0].port, 80);
        assert_eq!(filter_ports(&ports, "1234", by_port)[0].port, 80);
        assert_eq!(filter_ports(&ports, "", by_port).len(), 2);
    }

    #[test]
    fn ports_sort_by_any_column() {
        let port = |proto: &str, address: &str, port: u16, pid: Option<u32>, process: Option<&str>| PortInfo {
            proto: proto.into(),
            address: address.into(),
            port,
            pid,
            process: process.map(Into::into),
        };
        let ports = vec![
            port("tcp", "0.0.0.0", 443, Some(900), Some("nginx")),
            port("udp", "127.0.0.53", 53, None, None),
            port("tcp", "127.0.0.1", 3306, Some(120), Some("mysqld")),
            port("tcp", "0.0.0.0", 22, Some(700), Some("sshd")),
        ];
        let order = |key: PortSortKey, descending: bool| filter_ports(&ports, "", (key, descending)).iter().map(|p| p.port).collect::<Vec<_>>();
        assert_eq!(order(PortSortKey::Port, false), [22, 53, 443, 3306]);
        assert_eq!(order(PortSortKey::Port, true), [3306, 443, 53, 22]);
        assert_eq!(order(PortSortKey::Pid, false), [3306, 22, 443, 53], "看不到进程的排在最后");
        assert_eq!(order(PortSortKey::Process, false), [3306, 443, 22, 53], "看不到进程的排在最后");
        assert_eq!(order(PortSortKey::Proto, true), [53, 22, 443, 3306], "同协议按端口");
        assert_eq!(order(PortSortKey::Address, false), [22, 443, 3306, 53]);
    }

    fn service(unit: &str, description: &str, active: &str, sub: &str) -> ServiceInfo {
        ServiceInfo { unit: unit.into(), description: description.into(), active: active.into(), sub: sub.into(), enabled: String::new() }
    }

    #[test]
    fn services_are_searched_by_name_or_description_and_labelled_by_state() {
        let all = vec![service("nginx.service", "A high performance web server", "active", "running"), service("ufw.service", "Uncomplicated firewall", "active", "exited")];
        let by_status = (ServiceSortKey::Status, false);
        assert_eq!(filter_services(&all, "WEB", by_status).len(), 1);
        assert_eq!(filter_services(&all, "ufw", by_status)[0].unit, "ufw.service");
        assert_eq!(filter_services(&all, " ", by_status).len(), 2);
        assert_eq!(service_status(&all[0]), ("运行中", ui::SUCCESS));
        assert_eq!(service_status(&all[1]).0, "已生效");
        assert_eq!(service_status(&service("mysql.service", "", "failed", "failed")), ("失败", ui::DESTRUCTIVE));
        assert_eq!(service_status(&service("redis.service", "", "inactive", "dead")).0, "已停止");
        assert_eq!(service_status(&service("docker.service", "", "activating", "start-pre")).0, "启动中");
    }

    #[test]
    fn services_sort_by_any_column() {
        let with_boot = |mut s: ServiceInfo, enabled: &str| {
            s.enabled = enabled.into();
            s
        };
        let all = vec![
            with_boot(service("nginx.service", "Web server", "active", "running"), "enabled"),
            service("mysql.service", "Database", "failed", "failed"),
            with_boot(service("cron.service", "cron daemon", "inactive", "dead"), "disabled"),
            with_boot(service("apparmor.service", "", "active", "exited"), "enabled"),
        ];
        let names = |key: ServiceSortKey, descending: bool| filter_services(&all, "", (key, descending)).iter().map(|s| s.name().to_string()).collect::<Vec<_>>();
        assert_eq!(names(ServiceSortKey::Status, false), ["mysql", "nginx", "apparmor", "cron"], "默认同后端：失败、运行中、其余，同级按名字");
        assert_eq!(names(ServiceSortKey::Status, true), ["apparmor", "cron", "nginx", "mysql"]);
        assert_eq!(names(ServiceSortKey::Name, false), ["apparmor", "cron", "mysql", "nginx"]);
        assert_eq!(names(ServiceSortKey::Name, true), ["nginx", "mysql", "cron", "apparmor"]);
        assert_eq!(names(ServiceSortKey::Description, false), ["cron", "mysql", "nginx", "apparmor"], "不分大小写，没有说明的在最后");
        assert_eq!(names(ServiceSortKey::Description, true), ["nginx", "mysql", "cron", "apparmor"]);
        assert_eq!(names(ServiceSortKey::Boot, false), ["apparmor", "nginx", "cron", "mysql"], "开机启动的在前");
        // 按显示的名字排（不带 .service）：apport 在 apport-autoreport 前面
        let similar = vec![service("apport-autoreport.service", "", "inactive", "dead"), service("apport.service", "", "inactive", "dead")];
        let order = filter_services(&similar, "", (ServiceSortKey::Name, false)).iter().map(|s| s.name().to_string()).collect::<Vec<_>>();
        assert_eq!(order, ["apport", "apport-autoreport"]);
        let order = filter_services(&similar, "", (ServiceSortKey::Status, false)).iter().map(|s| s.name().to_string()).collect::<Vec<_>>();
        assert_eq!(order, ["apport", "apport-autoreport"], "同状态也按显示的名字");
    }

    #[test]
    fn the_local_machine_has_processes_and_ports_but_no_services() {
        let (local, remote) = (Machine::Local, Machine::Remote("s1".into()));
        assert_eq!(views_for(Some(&local)), [View::Processes, View::Ports, View::Disk], "macOS 没有 systemd");
        assert_eq!(views_for(Some(&remote)), [View::Processes, View::Ports, View::Services, View::Disk]);
        assert_eq!(views_for(None), [View::Processes, View::Ports, View::Services, View::Disk], "没连着时照旧全列");
        assert_eq!(view_on(Some(&local), View::Disk), View::Disk, "本机也能看磁盘");
        assert_eq!(view_on(Some(&local), View::Services), View::Processes, "从服务器的服务列表切到本机：回到进程");
        assert_eq!(view_on(Some(&local), View::Ports), View::Ports, "两边都有的留在原处");
        assert_eq!(view_on(Some(&remote), View::Services), View::Services);
    }

    #[test]
    fn list_errors_explain_missing_permissions() {
        assert!(list_error_message("Failed to list units: Access denied").contains("sudo -i"));
        assert_eq!(list_error_message("boom\n"), "读取失败：boom");
    }

    #[test]
    fn colors_carry_meaning() {
        assert_eq!(usage_color(0.3, ui::VIOLET), ui::VIOLET, "正常时用指标自己的颜色");
        assert_eq!(usage_color(0.8, ui::VIOLET), ui::WARNING);
        assert_eq!(usage_color(0.95, ui::VIOLET), ui::DESTRUCTIVE);
        assert_eq!(cpu_color(0.5), ui::SUCCESS);
        assert_eq!(cpu_color(25.), ui::WARNING);
        assert_eq!(cpu_color(180.), ui::DESTRUCTIVE, "多核进程会超过 100");
        assert_eq!(readable(ui::SUCCESS), ui::SUCCESS_FOREGROUND);
        assert_eq!(readable(ui::WARNING), ui::WARNING_FOREGROUND);
        assert_eq!(readable(ui::DESTRUCTIVE), ui::DESTRUCTIVE_FOREGROUND);
        assert_eq!(readable(ui::PRIMARY), ui::ACCENT_FOREGROUND, "负载圆环里的百分比：主色在浅蓝底上不够清楚");
        assert_eq!(readable(ui::VIOLET), ui::VIOLET);
    }

    /// CPU：没在用的淡色数字，用一点的浅色小标签，用得多的（≥ 5%）实心标签、白字，越多越红。
    #[test]
    fn cpu_badges_get_louder_with_usage() {
        assert_eq!(cpu_level(0.0), CpuLevel::Idle);
        assert_eq!(cpu_level(0.4), CpuLevel::Idle);
        assert_eq!(cpu_level(0.5), CpuLevel::Light(ui::SUCCESS));
        assert_eq!(cpu_level(4.9), CpuLevel::Light(ui::SUCCESS));
        assert_eq!(cpu_level(5.0), CpuLevel::Busy(ui::SUCCESS));
        assert_eq!(cpu_level(25.), CpuLevel::Busy(ui::WARNING));
        assert_eq!(cpu_level(180.), CpuLevel::Busy(ui::DESTRUCTIVE));
    }

    #[test]
    fn big_memory_users_stand_out() {
        assert_eq!(memory_level(99 * 1024), MemoryLevel::Normal);
        assert_eq!(memory_level(100 * 1024), MemoryLevel::Large);
        assert_eq!(memory_level(1024 * 1024), MemoryLevel::Huge);
    }

    /// 用户列：root 橙色、登录用户蓝色、macOS 的系统账号（_ 开头）淡色，其余照常。
    #[test]
    fn users_are_told_apart() {
        assert_eq!(user_kind("root", Some("qfdk")), UserKind::Root);
        assert_eq!(user_kind("root", Some("root")), UserKind::Root, "用 root 登录时 root 还是橙色");
        assert_eq!(user_kind("qfdk", Some("qfdk")), UserKind::Me);
        assert_eq!(user_kind("qfdk", None), UserKind::Other, "不知道登录用户时不猜");
        assert_eq!(user_kind("_windowserver", Some("qfdk")), UserKind::Service);
        assert_eq!(user_kind("www-data", Some("deploy")), UserKind::Other);
    }

    /// 圆环从 12 点钟开始顺时针画。
    #[test]
    fn ring_points_go_clockwise_from_twelve_o_clock() {
        let center = gpui::point(px(29.), px(29.));
        let at = |turn: f32| {
            let p = ring_point(center, px(24.), turn);
            ((f32::from(p.x) * 100.).round() / 100., (f32::from(p.y) * 100.).round() / 100.)
        };
        assert_eq!(at(0.), (29., 5.), "顶上");
        assert_eq!(at(0.25), (53., 29.), "右边");
        assert_eq!(at(0.5), (29., 53.), "底下");
        assert_eq!(at(0.75), (5., 29.), "左边");
    }

    #[test]
    fn listening_addresses_are_told_apart() {
        for address in ["*", "0.0.0.0", "::", "[::]"] {
            assert_eq!(address_scope(address), AddressScope::All, "{address}");
        }
        for address in ["127.0.0.1", "127.0.0.53", "::1", "[::1]", "localhost"] {
            assert_eq!(address_scope(address), AddressScope::Loopback, "{address}");
        }
        assert_eq!(address_scope("192.168.5.15"), AddressScope::Specific);
    }

    #[test]
    fn commands_split_into_directory_name_and_arguments() {
        assert_eq!(command_parts("/usr/bin/dockerd -H fd://", None), ("/usr/bin/", "dockerd", " -H fd://"));
        assert_eq!(command_parts("/sbin/init", None), ("/sbin/", "init", ""));
        assert_eq!(command_parts("nginx: worker process", None), ("", "nginx:", " worker process"));
        assert_eq!(command_parts("python3 app.py", None), ("", "python3", " app.py"));
        assert_eq!(command_parts("[kworker/u4:0-events_unbound]", None), ("", "[kworker/u4:0-events_unbound]", ""), "内核线程整个当名字");
    }

    /// 知道程序路径（macOS 的 comm）时按它切：路径里的空格不算参数的开始。
    #[test]
    fn a_known_program_path_keeps_its_spaces() {
        let program = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome Helper (Aperitif)";
        let command = format!("{program} --type=gpu-process");
        assert_eq!(
            command_parts(&command, Some(program)),
            ("/Applications/Google Chrome.app/Contents/MacOS/", "Google Chrome Helper (Aperitif)", " --type=gpu-process")
        );
        assert_eq!(process_name(&command, Some(program)), "Google Chrome Helper (Aperitif)");
        assert_eq!(process_name(&command, None), "Google", "不知道程序路径时只能按空格切");
    }

    #[test]
    fn names_and_kill_errors_read_well() {
        assert_eq!(process_name("/usr/sbin/sshd -D", None), "sshd");
        assert_eq!(process_name("nginx: worker process", None), "nginx");
        assert_eq!(process_name("python3 app.py", None), "python3");
        assert_eq!(process_name("[kworker/u4:0-events_unbound]", None), "[kworker/u4:0-events_unbound]");
        assert!(kill_error_message("kill: (1) - Operation not permitted").contains("sudo -i"));
        assert_eq!(kill_error_message("kill: (99999) - No such process"), "进程已经不在了。");
    }

    /// node / bun / python 跑的脚本是认得出的软件时，名字用脚本的（图标跟着名字认）：Gemini CLI、Qwen Code 不改进程名，
    /// 不这样就只是一个 node。认不出的脚本照旧是解释器的名字。
    #[test]
    fn a_known_script_names_its_interpreter_process() {
        assert_eq!(process_name("node /usr/local/bin/gemini --yolo", None), "gemini");
        assert_eq!(
            process_name("node --max-old-space-size=8192 /opt/homebrew/lib/node_modules/@google/gemini-cli/bundle/gemini.js", None),
            "gemini",
            "跳过参数，去掉 .js"
        );
        assert_eq!(process_name("/usr/local/bin/node /usr/local/bin/qwen", Some("/usr/local/bin/node")), "qwen");
        assert_eq!(process_name("node /usr/local/bin/codex exec", None), "codex");
        assert_eq!(process_name("/usr/bin/python3.12 /usr/local/bin/vllm serve Qwen/Qwen3-8B", None), "vllm");
        assert_eq!(process_name("python3 /usr/local/bin/jupyter-lab --no-browser", None), "jupyter-lab");
        assert_eq!(process_name("bun /srv/app/node_modules/.bin/next start", None), "next");
        for (command, name) in [
            ("node server.js", "node"),
            ("python3 app.py", "python3"),
            ("bun run dev", "bun"),
            ("node", "node"),
            ("/usr/bin/python3 /usr/bin/fail2ban-server -xf start", "python3"),
            // 不是解释器：参数里的程序名不算
            ("sudo /usr/local/bin/gemini", "sudo"),
        ] {
            assert_eq!(process_name(command, None), name, "{command}");
        }
        assert_eq!(brands::brand_of(&process_name("node /usr/local/bin/gemini", None)), Some(crate::assets::Brand::Gemini));
    }
}

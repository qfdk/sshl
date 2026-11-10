//! 「系统」页：负载 / 内存 / 磁盘 / 运行时间，进程、监听端口、systemd 服务和 Docker 容器；结束进程、启停服务和容器。
//! 看日志、进容器发到终端里做。
//! 命令以文件管理当前身份执行（终端 sudo -i 之后就是 root）；没权限时提议改在终端里用 sudo 执行。
//! 停在这页时每 3 秒刷新一次。

use std::ops::Range;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, Role,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Task, UniformListScrollHandle, Window, anchored,
    deferred, div, prelude::FluentBuilder, px, uniform_list,
};
use gpui_base::input::{InputEvent, InputState};
use sshl_core::server_info::{self, Overview, PortInfo, ProcessInfo};
use sshl_core::containers::{self, ContainerAction, ContainerInfo, Containers};
use sshl_core::services::{self, ServiceAction, ServiceInfo};

use crate::assets::Icon;
use crate::backend;
use crate::prompts::{self, Options};
use crate::theme::{self, ui};
use crate::ui::{self as kit, MenuEntry, MenuItem, Variant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const ROW_HEIGHT: f32 = 32.;
const MONO: &str = "Menlo";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Processes,
    Ports,
    Services,
    Containers,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Pid,
    User,
    Cpu,
    Memory,
    Command,
}

enum MenuTarget {
    Process { pid: u32, name: String, command: Option<String> },
    Service(ServiceInfo),
    Container(ContainerInfo),
}

struct MenuState {
    position: Point<Pixels>,
    target: MenuTarget,
}

pub enum SystemPanelEvent {
    /// 发到终端执行（没权限时的 sudo 命令、看服务日志）。
    RunInTerminal(String),
}

/// 一次刷新读到的列表。
enum Loaded {
    Processes(Vec<ProcessInfo>),
    Ports(Vec<PortInfo>),
    /// None = 没有 systemd。
    Services(Option<Vec<ServiceInfo>>),
    /// None = 没装 Docker / Podman。
    Containers(Option<Containers>),
}

pub struct SystemPanel {
    session_id: Option<String>,
    active: bool,
    view: View,
    overview: Option<Overview>,
    processes: Option<Vec<ProcessInfo>>,
    ports: Option<Vec<PortInfo>>,
    services: Option<Vec<ServiceInfo>>,
    no_systemd: bool,
    containers: Option<Vec<ContainerInfo>>,
    /// docker / podman；空 = 没装。
    engine: String,
    /// 正在启停的服务 / 容器（状态列显示「处理中…」）。
    busy: Option<String>,
    error: Option<SharedString>,
    sort: (SortKey, bool),
    search: Entity<InputState>,
    search_focused: bool,
    menu: Option<MenuState>,
    request: u64,
    scroll: UniformListScrollHandle,
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
    if unit == 0 { format!("{bytes} B") } else if value >= 100. { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}

pub fn format_uptime(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    match (days, hours) {
        (0, 0) => format!("{} 分钟", minutes.max(1)),
        (0, _) => format!("{hours} 小时 {minutes} 分"),
        _ => format!("{days} 天 {hours} 小时"),
    }
}

/// 搜索：PID、用户、命令里含关键字（不分大小写）。
pub fn filter_processes<'a>(processes: &'a [ProcessInfo], query: &str, sort: (SortKey, bool)) -> Vec<&'a ProcessInfo> {
    let query = query.trim().to_lowercase();
    let mut out: Vec<&ProcessInfo> = processes
        .iter()
        .filter(|p| query.is_empty() || p.pid.to_string().contains(&query) || p.user.to_lowercase().contains(&query) || p.command.to_lowercase().contains(&query))
        .collect();
    out.sort_by(|a, b| {
        let order = match sort.0 {
            SortKey::Pid => a.pid.cmp(&b.pid),
            SortKey::User => a.user.cmp(&b.user),
            SortKey::Cpu => a.cpu.total_cmp(&b.cpu).then(a.rss_kb.cmp(&b.rss_kb)),
            SortKey::Memory => a.rss_kb.cmp(&b.rss_kb),
            SortKey::Command => a.command.cmp(&b.command),
        };
        if sort.1 { order.reverse() } else { order }
    });
    out
}

/// 搜索：端口号、进程名、PID、地址里含关键字。
pub fn filter_ports<'a>(ports: &'a [PortInfo], query: &str) -> Vec<&'a PortInfo> {
    let query = query.trim().to_lowercase();
    ports
        .iter()
        .filter(|p| {
            query.is_empty()
                || p.port.to_string().contains(&query)
                || p.address.contains(&query)
                || p.pid.is_some_and(|pid| pid.to_string().contains(&query))
                || p.process.as_deref().is_some_and(|name| name.to_lowercase().contains(&query))
        })
        .collect()
}

/// 结束进程失败的说明：没权限时教用户怎么拿到权限。
pub fn kill_error_message(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("not permitted") || lower.contains("permission") {
        "没有权限结束这个进程。在终端里 sudo -i（或 su）切到 root 后再试，这里会跟着用 root 身份。".into()
    } else if lower.contains("no such process") {
        "进程已经不在了。".into()
    } else {
        error.trim().to_string()
    }
}

/// 搜索：服务名或说明里含关键字。
pub fn filter_services<'a>(services: &'a [ServiceInfo], query: &str) -> Vec<&'a ServiceInfo> {
    let query = query.trim().to_lowercase();
    services
        .iter()
        .filter(|s| query.is_empty() || s.unit.to_lowercase().contains(&query) || s.description.to_lowercase().contains(&query))
        .collect()
}

/// 服务的状态文字和颜色。
pub fn service_status(service: &ServiceInfo) -> (&'static str, u32) {
    match (service.active.as_str(), service.sub.as_str()) {
        ("failed", _) => ("失败", ui::DESTRUCTIVE),
        // 一次性服务（防火墙规则、AppArmor）跑完退出但仍算启用
        ("active", "exited") => ("已生效", ui::SUCCESS),
        ("active", _) => ("运行中", ui::SUCCESS),
        ("activating", _) => ("启动中", ui::WARNING),
        ("deactivating", _) => ("停止中", ui::WARNING),
        ("reloading", _) => ("重载中", ui::WARNING),
        _ => ("已停止", ui::MUTED_FOREGROUND),
    }
}

/// 搜索：容器名、镜像、端口、ID 开头。
pub fn filter_containers<'a>(containers: &'a [ContainerInfo], query: &str) -> Vec<&'a ContainerInfo> {
    let query = query.trim().to_lowercase();
    containers
        .iter()
        .filter(|c| {
            query.is_empty()
                || c.name.to_lowercase().contains(&query)
                || c.image.to_lowercase().contains(&query)
                || c.ports.contains(&query)
                || c.id.starts_with(&query)
        })
        .collect()
}

/// 容器的状态文字和颜色：非零退出码标红（崩了、被 OOM 杀了）。
pub fn container_status(container: &ContainerInfo) -> (SharedString, u32) {
    let (label, color): (SharedString, u32) = match container.state.as_str() {
        "running" => ("运行中".into(), ui::SUCCESS),
        "paused" => ("已暂停".into(), ui::WARNING),
        "restarting" => ("重启中".into(), ui::WARNING),
        "created" => ("未启动".into(), ui::MUTED_FOREGROUND),
        "dead" => ("已失效".into(), ui::DESTRUCTIVE),
        "exited" => match container.exit_code() {
            Some(code) if code != 0 => (format!("退出 {code}").into(), ui::DESTRUCTIVE),
            _ => ("已停止".into(), ui::MUTED_FOREGROUND),
        },
        other => (other.to_string().into(), ui::MUTED_FOREGROUND),
    };
    (label, color)
}

fn container_action_label(action: ContainerAction) -> &'static str {
    match action {
        ContainerAction::Start => "启动",
        ContainerAction::Stop => "停止",
        ContainerAction::Restart => "重启",
        ContainerAction::Remove => "删除",
    }
}

/// 读列表失败的说明：没权限时说怎么办。
pub fn list_error_message(view: View, error: &str) -> String {
    if services::needs_root(error) {
        return match view {
            View::Containers => "没有权限访问 Docker：把当前用户加进 docker 组，或在终端里 sudo -i 后再看。".into(),
            _ => "没有权限：在终端里 sudo -i 后再看。".into(),
        };
    }
    format!("读取失败：{}", error.trim())
}

fn action_label(action: ServiceAction) -> &'static str {
    match action {
        ServiceAction::Start => "启动",
        ServiceAction::Stop => "停止",
        ServiceAction::Restart => "重启",
        ServiceAction::Enable => "设为开机启动",
        ServiceAction::Disable => "取消开机启动",
    }
}

// ---------------------------------------------------------------- 状态

impl SystemPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索…"));
        let subscription = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Change => {
                this.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
            session_id: None,
            active: false,
            view: View::Processes,
            overview: None,
            processes: None,
            ports: None,
            services: None,
            no_systemd: false,
            containers: None,
            engine: String::new(),
            busy: None,
            error: None,
            sort: (SortKey::Cpu, true),
            search,
            search_focused: false,
            menu: None,
            request: 0,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _refresh: None,
            _subscription: subscription,
        }
    }

    /// 切会话 / 断开（None）。
    pub fn set_session(&mut self, session_id: Option<String>, cx: &mut Context<Self>) {
        if self.session_id == session_id {
            return;
        }
        self.session_id = session_id;
        self.overview = None;
        self.processes = None;
        self.ports = None;
        self.services = None;
        self.no_systemd = false;
        self.containers = None;
        self.engine.clear();
        self.busy = None;
        self.error = None;
        self.menu = None;
        self.request += 1;
        self.restart(cx);
        cx.notify();
    }

    /// 「系统」标签显示 / 隐藏：显示时立刻刷新并每 3 秒刷新一次，隐藏就停。
    pub fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        if active {
            window.focus(&self.focus, cx);
        } else {
            self.menu = None;
        }
        self.restart(cx);
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        self._refresh = (self.active && self.session_id.is_some()).then(|| {
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
        let Some(session_id) = self.session_id.clone() else { return };
        self.request += 1;
        let request = self.request;
        let view = self.view;
        let sid = session_id.clone();
        let overview = backend::spawn(async move { server_info::overview(&backend::get().state, sid).await });
        let list = backend::spawn(async move {
            let state = &backend::get().state;
            match view {
                View::Processes => server_info::processes(state, session_id).await.map(Loaded::Processes),
                View::Ports => server_info::ports(state, session_id).await.map(Loaded::Ports),
                View::Services => services::services(state, session_id).await.map(Loaded::Services),
                View::Containers => containers::containers(state, session_id).await.map(Loaded::Containers),
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
                            Loaded::Processes(processes) => this.processes = Some(processes),
                            Loaded::Ports(ports) => this.ports = Some(ports),
                            Loaded::Services(services) => {
                                this.no_systemd = services.is_none();
                                this.services = Some(services.unwrap_or_default());
                            }
                            Loaded::Containers(found) => {
                                let (engine, list) = found.map(|c| (c.engine, c.list)).unwrap_or_default();
                                this.engine = engine;
                                this.containers = Some(list);
                            }
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
        self.view = view;
        self.menu = None;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.restart(cx);
        cx.notify();
    }

    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        // 再点同一列反过来；数值列默认从大到小
        self.sort = if self.sort.0 == key { (key, !self.sort.1) } else { (key, matches!(key, SortKey::Cpu | SortKey::Memory)) };
        cx.notify();
    }

    fn kill(&mut self, pid: u32, name: String, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(session_id) = self.session_id.clone() else { return };
        let (title, detail) = if force {
            (format!("强制结束 {name}？"), format!("PID {pid} · kill -9：进程来不及收尾（不保存、不清理），只在普通结束不管用时用。"))
        } else {
            (format!("结束 {name}？"), format!("PID {pid} · 发送 TERM，进程可以先收尾再退出。"))
        };
        let mut options = Options::new(title).message(detail).confirm(if force { "强制结束" } else { "结束进程" });
        options.destructive = true;
        let answer = prompts::confirm(window, cx, options);
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await.unwrap_or(false) {
                return;
            }
            let result = backend::spawn(async move { server_info::kill(&backend::get().state, session_id, pid, force).await }).await;
            // 给进程一点时间退出，再刷新列表
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Some(Ok(())) => {}
                    Some(Err(error)) if services::needs_root(&error.to_string()) => {
                        let command = format!("sudo {}", server_info::kill_command(pid, force));
                        this.offer_sudo(format!("结束 {name} 需要 root 权限"), None, command, window, cx);
                    }
                    Some(Err(error)) => prompts::alert_later(window, cx, Options::new(format!("没能结束 {name}")).message(kill_error_message(&error.to_string()))),
                    None => {}
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    /// 没权限：问要不要改在终端里用 sudo 执行（终端里输密码，或点「填充密码」）。
    fn offer_sudo(&mut self, title: String, note: Option<&str>, command: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut options = Options::new(title).detail(command.clone()).confirm("在终端里用 sudo 执行");
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
        let Some(session_id) = self.session_id.clone() else { return };
        let name = service.name().to_string();
        let confirm = matches!(action, ServiceAction::Stop | ServiceAction::Restart).then(|| {
            let mut options = Options::new(format!("{}{name}？", action_label(action))).confirm(action_label(action));
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
            let result = backend::spawn(async move { services::act(&backend::get().state, session_id, action, unit).await }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Some(Ok(())) | None => {}
                    Some(Err(error)) if services::needs_root(&error.to_string()) => {
                        let command = services::terminal_command(action, &service.unit);
                        this.offer_sudo(format!("{} {name} 需要 root 权限", action_label(action)), None, command, window, cx);
                    }
                    Some(Err(error)) => prompts::alert_later(window, cx, Options::new(format!("没能{}{name}", action_label(action))).message(error.to_string().trim().to_string())),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    /// 启动 / 停止 / 重启 / 删除容器。停止、重启、删除先确认。
    fn container_action(&mut self, container: ContainerInfo, action: ContainerAction, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(session_id) = self.session_id.clone() else { return };
        let engine = self.engine.clone();
        let name = container.name.clone();
        let label = container_action_label(action);
        let confirm = (action != ContainerAction::Start).then(|| {
            let mut options = Options::new(format!("{label}容器 {name}？")).message(container.image.clone()).confirm(label);
            if action == ContainerAction::Remove {
                options = options.detail("容器里没放在卷上的数据会一起删掉。");
            }
            options.destructive = matches!(action, ContainerAction::Stop | ContainerAction::Remove);
            prompts::confirm(window, cx, options)
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Some(answer) = confirm
                && !answer.await.unwrap_or(false)
            {
                return;
            }
            let id = container.id.clone();
            let _ = this.update(cx, |this, cx| {
                this.busy = Some(id.clone());
                cx.notify();
            });
            let run_engine = engine.clone();
            let result = backend::spawn(async move { containers::act(&backend::get().state, session_id, run_engine, action, id).await }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Some(Ok(())) | None => {}
                    Some(Err(error)) if services::needs_root(&error.to_string()) => {
                        let command = containers::terminal_command(&engine, action, &name);
                        this.offer_sudo(format!("{label}容器 {name} 需要 root 权限"), Some("也可以把当前用户加进 docker 组。"), command, window, cx);
                    }
                    Some(Err(error)) => prompts::alert_later(window, cx, Options::new(format!("没能{label}容器 {name}")).message(error.to_string().trim().to_string())),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    fn container_terminal(&mut self, command: String, cx: &mut Context<Self>) {
        self.menu = None;
        cx.emit(SystemPanelEvent::RunInTerminal(command));
    }

    fn service_logs(&mut self, service: &ServiceInfo, cx: &mut Context<Self>) {
        self.menu = None;
        cx.emit(SystemPanelEvent::RunInTerminal(services::logs_command(&service.unit)));
    }

    fn open_menu(&mut self, position: Point<Pixels>, target: MenuTarget, cx: &mut Context<Self>) {
        self.menu = Some(MenuState { position, target });
        cx.notify();
    }

    // ---------------------------------------------------------------- 渲染

    fn overview_cards(&self) -> impl IntoElement {
        let o = self.overview.clone().unwrap_or_default();
        let card = |title: &'static str, value: String, sub: Option<String>, ratio: Option<f32>| {
            div()
                .flex_1()
                .min_w_0()
                .px_4()
                .py_3()
                .flex()
                .flex_col()
                .gap(px(6.))
                .rounded(px(12.))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .bg(theme::color(ui::CARD))
                .child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(title))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).truncate().child(value))
                        .when_some(sub, |d, sub| d.child(div().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).truncate().child(sub))),
                )
                .when_some(ratio, |d, ratio| {
                    let color = if ratio >= 0.9 { ui::DESTRUCTIVE } else if ratio >= 0.75 { ui::WARNING } else { ui::PRIMARY };
                    d.child(
                        div()
                            .h(px(4.))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(theme::alpha(color, 0.18))
                            .child(div().h_full().w(gpui::relative(ratio.clamp(0., 1.))).bg(theme::color(color))),
                    )
                })
        };
        let dash = || "—".to_string();
        let load = o.load.map(|l| format!("{:.2}  {:.2}  {:.2}", l[0], l[1], l[2])).unwrap_or_else(dash);
        let cpus = o.cpus.map(|c| format!("{c} 核"));
        let load_ratio = o.load.zip(o.cpus).map(|(l, c)| l[0] / c.max(1) as f32);
        let (memory, memory_sub, memory_ratio) = match (o.mem_total, o.mem_available) {
            (Some(total), Some(available)) => {
                let used = total.saturating_sub(available);
                (format_bytes(used), Some(format!("共 {}", format_bytes(total))), Some(used as f32 / total.max(1) as f32))
            }
            (Some(total), None) => (format_bytes(total), Some("总量".into()), None),
            _ => (dash(), None, None),
        };
        let (disk, disk_sub, disk_ratio) = match (o.disk_total, o.disk_used) {
            (Some(total), Some(used)) => (format_bytes(used), Some(format!("共 {} · /", format_bytes(total))), Some(used as f32 / total.max(1) as f32)),
            _ => (dash(), None, None),
        };
        let uptime = o.uptime_secs.map(format_uptime).unwrap_or_else(dash);
        div()
            .flex()
            .gap_3()
            .flex_shrink_0()
            .child(card("负载（1 / 5 / 15 分钟）", load, cpus, load_ratio))
            .child(card("内存已用", memory, memory_sub, memory_ratio))
            .child(card("磁盘已用", disk, disk_sub, disk_ratio))
            .child(card("已运行", uptime, None, None))
    }

    fn header_cell(&self, label: &'static str, key: Option<SortKey>, width: Option<f32>, right: bool, cx: &mut Context<Self>) -> AnyElement {
        let arrow = key.filter(|k| *k == self.sort.0).map(|_| if self.sort.1 { " ↓" } else { " ↑" }).unwrap_or("");
        div()
            .id(label)
            .px_3()
            .flex()
            .items_center()
            .map(|d| match width {
                Some(w) => d.w(px(w)).flex_shrink_0(),
                None => d.flex_1().min_w_0(),
            })
            .when(right, |d| d.justify_end())
            .child(format!("{label}{arrow}"))
            .when_some(key, |d, key| {
                d.cursor_pointer()
                    .hover(|s| s.text_color(theme::color(ui::FOREGROUND)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_sort(key, cx)))
            })
            .into_any_element()
    }

    fn kill_button(&self, id: (&'static str, usize), pid: u32, name: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(SharedString::from(format!("结束 {name}")))
            .px_2()
            .py(px(2.))
            .rounded(px(6.))
            .text_xs()
            .text_color(theme::color(ui::DESTRUCTIVE))
            .opacity(0.)
            .group_hover("system-row", |s| s.opacity(1.))
            .hover(|s| s.bg(theme::alpha(ui::DESTRUCTIVE, 0.1)))
            .cursor_pointer()
            .child("结束")
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.kill(pid, name.clone(), false, window, cx);
            }))
            .into_any_element()
    }

    fn render_process_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        let rows: Vec<ProcessInfo> = filter_processes(self.processes.as_deref().unwrap_or(&[]), &query, self.sort)
            .into_iter()
            .skip(range.start)
            .take(range.len())
            .cloned()
            .collect();
        rows.into_iter()
            .enumerate()
            .map(|(i, p)| {
                let ix = range.start + i;
                let name = process_name(&p.command);
                let (menu_name, menu_command) = (name.clone(), p.command.clone());
                let pid = p.pid;
                div()
                    .id(("process", ix))
                    .group("system-row")
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .border_b_1()
                    .border_color(theme::alpha(ui::BORDER, 0.6))
                    .hover(|s| s.bg(theme::color(ui::MUTED)))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.open_menu(event.position, MenuTarget::Process { pid, name: menu_name.clone(), command: Some(menu_command.clone()) }, cx);
                    }))
                    .child(div().w(px(76.)).flex_shrink_0().px_3().flex().justify_end().font_family(MONO).child(p.pid.to_string()))
                    .child(div().w(px(110.)).flex_shrink_0().px_3().truncate().child(p.user.clone()))
                    .child(
                        div()
                            .w(px(70.))
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .justify_end()
                            .font_family(MONO)
                            .when(p.cpu >= 50., |d| d.text_color(theme::color(ui::DESTRUCTIVE)).font_weight(FontWeight::SEMIBOLD))
                            .child(format!("{:.1}", p.cpu)),
                    )
                    .child(div().w(px(90.)).flex_shrink_0().px_3().flex().justify_end().font_family(MONO).child(format_bytes(p.rss_kb * 1024)))
                    .child(div().flex_1().min_w_0().px_3().truncate().font_family(MONO).text_color(theme::alpha(ui::FOREGROUND, 0.85)).child(p.command.clone()))
                    .child(div().w(px(64.)).flex_shrink_0().flex().justify_center().child(self.kill_button(("kill-process", ix), pid, name, cx)))
                    .into_any_element()
            })
            .collect()
    }

    fn render_port_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        let rows: Vec<PortInfo> =
            filter_ports(self.ports.as_deref().unwrap_or(&[]), &query).into_iter().skip(range.start).take(range.len()).cloned().collect();
        rows.into_iter()
            .enumerate()
            .map(|(i, p)| {
                let ix = range.start + i;
                let name = p.process.clone().unwrap_or_else(|| "—".into());
                div()
                    .id(("port", ix))
                    .group("system-row")
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .border_b_1()
                    .border_color(theme::alpha(ui::BORDER, 0.6))
                    .hover(|s| s.bg(theme::color(ui::MUTED)))
                    .when_some(p.pid, |d, pid| {
                        let menu_name = name.clone();
                        d.on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_menu(event.position, MenuTarget::Process { pid, name: menu_name.clone(), command: None }, cx);
                        }))
                    })
                    .child(div().w(px(64.)).flex_shrink_0().px_3().text_color(theme::color(ui::MUTED_FOREGROUND)).child(p.proto.to_uppercase()))
                    .child(div().w(px(86.)).flex_shrink_0().px_3().flex().justify_end().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(p.port.to_string()))
                    .child(div().w(px(180.)).flex_shrink_0().px_3().truncate().font_family(MONO).text_color(theme::alpha(ui::FOREGROUND, 0.85)).child(p.address.clone()))
                    .child(div().w(px(80.)).flex_shrink_0().px_3().flex().justify_end().font_family(MONO).child(p.pid.map(|pid| pid.to_string()).unwrap_or_else(|| "—".into())))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .truncate()
                            .when(p.pid.is_none(), |d| d.text_color(theme::color(ui::MUTED_FOREGROUND)))
                            .child(if p.pid.is_some() { name.clone() } else { "看不到进程（需要 root）".into() }),
                    )
                    .child(div().w(px(64.)).flex_shrink_0().flex().justify_center().when_some(p.pid, |d, pid| d.child(self.kill_button(("kill-port", ix), pid, name.clone(), cx))))
                    .into_any_element()
            })
            .collect()
    }
}

impl SystemPanel {
    /// 行尾悬停才出现的文字按钮（结束、启动、停止…）。
    fn hover_button(id: (&'static str, usize), label: &'static str, aria: String, color: u32) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(SharedString::from(aria))
            .px_2()
            .py(px(2.))
            .rounded(px(6.))
            .text_xs()
            .text_color(theme::color(color))
            .opacity(0.)
            .group_hover("system-row", |s| s.opacity(1.))
            .hover(|s| s.bg(theme::alpha(color, 0.1)))
            .cursor_pointer()
            .child(label)
    }

    fn render_service_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        let rows: Vec<ServiceInfo> =
            filter_services(self.services.as_deref().unwrap_or(&[]), &query).into_iter().skip(range.start).take(range.len()).cloned().collect();
        rows.into_iter()
            .enumerate()
            .map(|(i, service)| {
                let ix = range.start + i;
                let name = service.name().to_string();
                let busy = self.busy.as_deref() == Some(service.unit.as_str());
                let (status, color) = if busy { ("处理中…", ui::WARNING) } else { service_status(&service) };
                let running = service.is_running();
                let button = |id: &'static str, action: ServiceAction, color: u32, cx: &mut Context<Self>| {
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
                        d.child(Self::hover_button(("service-logs", ix), "日志", format!("{name} 的日志"), ui::MUTED_FOREGROUND).on_click(cx.listener(
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
                div()
                    .id(("service", ix))
                    .group("system-row")
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .border_b_1()
                    .border_color(theme::alpha(ui::BORDER, 0.6))
                    .hover(|s| s.bg(theme::color(ui::MUTED)))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.open_menu(event.position, MenuTarget::Service(menu_target.clone()), cx);
                    }))
                    .child(
                        div()
                            .w(px(96.))
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().size(px(7.)).flex_shrink_0().rounded_full().bg(theme::color(color)))
                            .child(div().when(color == ui::DESTRUCTIVE, |d| d.text_color(theme::color(ui::DESTRUCTIVE))).child(status)),
                    )
                    .child(div().w(px(220.)).flex_shrink_0().px_3().truncate().font_weight(FontWeight::MEDIUM).child(name.clone()))
                    .child(div().flex_1().min_w_0().px_3().truncate().text_color(theme::color(ui::MUTED_FOREGROUND)).child(service.description.clone()))
                    .child(
                        div()
                            .w(px(80.))
                            .flex_shrink_0()
                            .flex()
                            .justify_center()
                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                            .map(|d| match service.enabled.as_str() {
                                e if e.starts_with("enabled") => d.child(kit::icon_sized(Icon::Check, 14.).text_color(theme::color(ui::PRIMARY))),
                                "masked" => d.child("已屏蔽"),
                                _ => d,
                            }),
                    )
                    .child(actions)
                    .into_any_element()
            })
            .collect()
    }
}

impl SystemPanel {
    fn render_container_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        let rows: Vec<ContainerInfo> =
            filter_containers(self.containers.as_deref().unwrap_or(&[]), &query).into_iter().skip(range.start).take(range.len()).cloned().collect();
        let engine = self.engine.clone();
        rows.into_iter()
            .enumerate()
            .map(|(i, container)| {
                let ix = range.start + i;
                let name = container.name.clone();
                let busy = self.busy.as_deref() == Some(container.id.as_str());
                let (status, color) = if busy { ("处理中…".into(), ui::WARNING) } else { container_status(&container) };
                let running = container.is_running();
                let action_button = |id: &'static str, action: ContainerAction, color: u32, cx: &mut Context<Self>| {
                    let target = container.clone();
                    let label = container_action_label(action);
                    Self::hover_button((id, ix), label, format!("{label} {name}"), color).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.container_action(target.clone(), action, window, cx);
                    }))
                };
                let terminal_button = |id: &'static str, label: &'static str, command: String, cx: &mut Context<Self>| {
                    Self::hover_button((id, ix), label, format!("{name} {label}"), ui::MUTED_FOREGROUND).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.container_terminal(command.clone(), cx);
                    }))
                };
                let menu_target = container.clone();
                let actions = div().w(px(200.)).flex_shrink_0().flex().justify_end().pr_2().when(!busy, |d| {
                    d.child(terminal_button("container-logs", "日志", containers::logs_command(&engine, &name), cx)).map(|d| {
                        if running {
                            d.child(terminal_button("container-shell", "终端", containers::shell_command(&engine, &name), cx))
                                .child(action_button("container-restart", ContainerAction::Restart, ui::PRIMARY, cx))
                                .child(action_button("container-stop", ContainerAction::Stop, ui::DESTRUCTIVE, cx))
                        } else {
                            d.child(action_button("container-start", ContainerAction::Start, ui::PRIMARY, cx))
                        }
                    })
                });
                div()
                    .id(("container", ix))
                    .group("system-row")
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .border_b_1()
                    .border_color(theme::alpha(ui::BORDER, 0.6))
                    .hover(|s| s.bg(theme::color(ui::MUTED)))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.open_menu(event.position, MenuTarget::Container(menu_target.clone()), cx);
                    }))
                    .child(
                        div()
                            .w(px(96.))
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().size(px(7.)).flex_shrink_0().rounded_full().bg(theme::color(color)))
                            .child(div().when(color == ui::DESTRUCTIVE, |d| d.text_color(theme::color(ui::DESTRUCTIVE))).child(status)),
                    )
                    .child(div().w(px(200.)).flex_shrink_0().px_3().truncate().font_weight(FontWeight::MEDIUM).child(name.clone()))
                    .child(div().w(px(240.)).flex_shrink_0().px_3().truncate().font_family(MONO).text_color(theme::color(ui::MUTED_FOREGROUND)).child(container.image.clone()))
                    .child(div().flex_1().min_w_0().px_3().truncate().font_family(MONO).child(containers::short_ports(&container.ports)))
                    .child(actions)
                    .into_any_element()
            })
            .collect()
    }
}

/// 命令行 → 进程名（第一个词去掉路径，冒号结尾的如「nginx: worker」保留原样的头一个词）。
pub fn process_name(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or(command);
    let name = first.rsplit('/').next().unwrap_or(first).trim_end_matches(':');
    if name.is_empty() { command.to_string() } else { name.to_string() }
}

impl Render for SystemPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_string();
        let process_count = self.processes.as_deref().map(|p| filter_processes(p, &query, self.sort).len());
        let port_count = self.ports.as_deref().map(|p| filter_ports(p, &query).len());
        let service_count = self.services.as_deref().filter(|_| !self.no_systemd).map(|s| filter_services(s, &query).len());
        let container_count = self.containers.as_deref().filter(|_| !self.engine.is_empty()).map(|c| filter_containers(c, &query).len());
        let (count, loaded) = match self.view {
            View::Processes => (process_count.unwrap_or(0), self.processes.is_some()),
            View::Ports => (port_count.unwrap_or(0), self.ports.is_some()),
            View::Services => (service_count.unwrap_or(0), self.services.is_some()),
            View::Containers => (container_count.unwrap_or(0), self.containers.is_some()),
        };
        let this = cx.entity().downgrade();
        let tab_label = |label: &str, count: Option<usize>| -> SharedString {
            match count {
                Some(n) => format!("{label} {n}").into(),
                None => label.to_string().into(),
            }
        };
        let toolbar = div()
            .flex()
            .items_center()
            .gap_3()
            .child(kit::segmented(
                "system-view",
                &[
                    (View::Processes, None, tab_label("进程", process_count)),
                    (View::Ports, None, tab_label("端口", port_count)),
                    (View::Services, None, tab_label("服务", service_count)),
                    (View::Containers, None, tab_label("容器", container_count)),
                ],
                self.view,
                move |view, _, cx| {
                    let _ = this.update(cx, |this, cx| this.set_view(view, cx));
                },
            ))
            .child(div().flex_1().min_w_0().max_w(px(360.)).child(kit::text_field(&self.search, self.search_focused)))
            .child(
                kit::icon_button("system-refresh", Variant::Ghost, Icon::RefreshCw, "刷新")
                    .w(px(32.))
                    .h(px(32.))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            );

        let header = match self.view {
            View::Processes => div()
                .h(px(32.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::color(ui::MUTED_FOREGROUND))
                .border_b_1()
                .border_color(theme::color(ui::BORDER))
                .child(self.header_cell("PID", Some(SortKey::Pid), Some(76.), true, cx))
                .child(self.header_cell("用户", Some(SortKey::User), Some(110.), false, cx))
                .child(self.header_cell("CPU%", Some(SortKey::Cpu), Some(70.), true, cx))
                .child(self.header_cell("内存", Some(SortKey::Memory), Some(90.), true, cx))
                .child(self.header_cell("命令", Some(SortKey::Command), None, false, cx))
                .child(div().w(px(64.)).flex_shrink_0()),
            View::Ports => div()
                .h(px(32.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::color(ui::MUTED_FOREGROUND))
                .border_b_1()
                .border_color(theme::color(ui::BORDER))
                .child(self.header_cell("协议", None, Some(64.), false, cx))
                .child(self.header_cell("端口", None, Some(86.), true, cx))
                .child(self.header_cell("地址", None, Some(180.), false, cx))
                .child(self.header_cell("PID", None, Some(80.), true, cx))
                .child(self.header_cell("进程", None, None, false, cx))
                .child(div().w(px(64.)).flex_shrink_0()),
            View::Services => div()
                .h(px(32.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::color(ui::MUTED_FOREGROUND))
                .border_b_1()
                .border_color(theme::color(ui::BORDER))
                .child(self.header_cell("状态", None, Some(96.), false, cx))
                .child(self.header_cell("服务", None, Some(220.), false, cx))
                .child(self.header_cell("说明", None, None, false, cx))
                .child(div().w(px(80.)).flex_shrink_0().flex().justify_center().child("开机启动"))
                .child(div().w(px(150.)).flex_shrink_0()),
            View::Containers => div()
                .h(px(32.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::color(ui::MUTED_FOREGROUND))
                .border_b_1()
                .border_color(theme::color(ui::BORDER))
                .child(self.header_cell("状态", None, Some(96.), false, cx))
                .child(self.header_cell("容器", None, Some(200.), false, cx))
                .child(self.header_cell("镜像", None, Some(240.), false, cx))
                .child(self.header_cell("端口", None, None, false, cx))
                .child(div().w(px(200.)).flex_shrink_0()),
        };

        let view_is_services = self.view == View::Services;
        let empty: Option<SharedString> = if self.session_id.is_none() {
            Some("连接服务器后显示它的进程、端口、服务和容器".into())
        } else if let Some(error) = &self.error {
            Some(list_error_message(self.view, error).into())
        } else if !loaded {
            Some("正在读取…".into())
        } else if view_is_services && self.no_systemd {
            Some("这台服务器没有 systemd".into())
        } else if self.view == View::Containers && self.engine.is_empty() {
            Some("这台服务器没有装 Docker".into())
        } else if count == 0 {
            Some(if query.trim().is_empty() { "没有内容".into() } else { "没有匹配的结果".into() })
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
            None => uniform_list(
                "system-rows",
                count,
                cx.processor(move |this, range, window, cx| match view {
                    View::Processes => this.render_process_rows(range, window, cx),
                    View::Ports => this.render_port_rows(range, window, cx),
                    View::Services => this.render_service_rows(range, window, cx),
                    View::Containers => this.render_container_rows(range, window, cx),
                }),
            )
            .track_scroll(&self.scroll)
            .flex_1()
            .w_full()
            .into_any_element(),
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
                        MenuEntry::Item(MenuItem::new("结束进程", action(false)).destructive()),
                        MenuEntry::Item(MenuItem::new("强制结束（kill -9）", action(true)).destructive()),
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuItem::new("复制 PID", move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(pid.to_string())))),
                    ];
                    if let Some(command) = command {
                        entries.push(MenuEntry::Item(MenuItem::new("复制命令", move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(command.clone())))));
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
                        MenuEntry::Item(MenuItem::new("在终端里看日志", move |_, cx| {
                            let _ = logs_weak.update(cx, |this, cx| this.service_logs(&logs_service, cx));
                        })),
                        MenuEntry::Item(MenuItem::new("复制名称", move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(unit.clone())))),
                    ]
                }
                MenuTarget::Container(container) => {
                    let running = container.is_running();
                    let item = |action: ContainerAction| {
                        let (weak, container) = (weak.clone(), container.clone());
                        MenuItem::new(container_action_label(action), move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.container_action(container.clone(), action, window, cx));
                        })
                    };
                    let terminal = |label: &'static str, command: String| {
                        let weak = weak.clone();
                        MenuItem::new(label, move |_, cx| {
                            let _ = weak.update(cx, |this, cx| this.container_terminal(command.clone(), cx));
                        })
                    };
                    let (name, id) = (container.name.clone(), container.id.clone());
                    let mut entries = vec![
                        MenuEntry::Item(item(ContainerAction::Start).disabled(running)),
                        MenuEntry::Item(item(ContainerAction::Restart).disabled(!running)),
                        MenuEntry::Item(item(ContainerAction::Stop).disabled(!running).destructive()),
                        MenuEntry::Separator,
                        MenuEntry::Item(terminal("在终端里看日志", containers::logs_command(&self.engine, &container.name))),
                        MenuEntry::Item(terminal("在终端里进入容器", containers::shell_command(&self.engine, &container.name)).disabled(!running)),
                        MenuEntry::Item(MenuItem::new("复制名称", move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(name.clone())))),
                        MenuEntry::Item(MenuItem::new("复制 ID", move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(id.clone())))),
                    ];
                    if !running {
                        entries.push(MenuEntry::Separator);
                        entries.push(MenuEntry::Item(item(ContainerAction::Remove).destructive()));
                    }
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
            .gap_3()
            .child(self.overview_cards())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(theme::color(ui::BORDER))
                    .bg(theme::color(ui::CARD))
                    .child(toolbar)
                    .child(div().flex_1().min_h_0().flex().flex_col().child(header).child(body)),
            )
            .when_some(menu, |d, menu| d.child(menu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, user: &str, cpu: f32, rss_kb: u64, command: &str) -> ProcessInfo {
        ProcessInfo { pid, user: user.into(), cpu, mem: 0., rss_kb, command: command.into() }
    }

    #[test]
    fn formats_sizes_and_uptime_for_people() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(8 * 1024 * 1024 * 1024), "8.0 GB");
        assert_eq!(format_bytes(200 * 1024 * 1024), "200 MB");
        assert_eq!(format_uptime(30), "1 分钟");
        assert_eq!(format_uptime(3 * 3600 + 20 * 60), "3 小时 20 分");
        assert_eq!(format_uptime(12 * 86400 + 5 * 3600), "12 天 5 小时");
    }

    #[test]
    fn searches_and_sorts_processes() {
        let procs = vec![p(10, "root", 1.0, 500, "/usr/sbin/sshd -D"), p(20, "www-data", 30.0, 9000, "nginx: worker process"), p(30, "mysql", 5.0, 90000, "/usr/sbin/mysqld")];
        let pids = |v: Vec<&ProcessInfo>| v.iter().map(|p| p.pid).collect::<Vec<_>>();
        assert_eq!(pids(filter_processes(&procs, "", (SortKey::Cpu, true))), [20, 30, 10]);
        assert_eq!(pids(filter_processes(&procs, "", (SortKey::Memory, true))), [30, 20, 10]);
        assert_eq!(pids(filter_processes(&procs, "", (SortKey::Pid, false))), [10, 20, 30]);
        assert_eq!(pids(filter_processes(&procs, "NGINX", (SortKey::Cpu, true))), [20]);
        assert_eq!(pids(filter_processes(&procs, "mysql", (SortKey::Cpu, true))), [30], "按用户或命令都能搜");
        assert_eq!(pids(filter_processes(&procs, "10", (SortKey::Cpu, true))), [10]);
    }

    #[test]
    fn searches_ports_by_number_name_or_pid() {
        let ports = vec![
            PortInfo { proto: "tcp".into(), address: "0.0.0.0".into(), port: 80, pid: Some(1234), process: Some("nginx".into()) },
            PortInfo { proto: "tcp".into(), address: "127.0.0.1".into(), port: 3306, pid: None, process: None },
        ];
        assert_eq!(filter_ports(&ports, "3306").len(), 1);
        assert_eq!(filter_ports(&ports, "Nginx")[0].port, 80);
        assert_eq!(filter_ports(&ports, "1234")[0].port, 80);
        assert_eq!(filter_ports(&ports, "").len(), 2);
    }

    fn service(unit: &str, description: &str, active: &str, sub: &str) -> ServiceInfo {
        ServiceInfo { unit: unit.into(), description: description.into(), active: active.into(), sub: sub.into(), enabled: String::new() }
    }

    #[test]
    fn services_are_searched_by_name_or_description_and_labelled_by_state() {
        let all = vec![service("nginx.service", "A high performance web server", "active", "running"), service("ufw.service", "Uncomplicated firewall", "active", "exited")];
        assert_eq!(filter_services(&all, "WEB").len(), 1);
        assert_eq!(filter_services(&all, "ufw")[0].unit, "ufw.service");
        assert_eq!(filter_services(&all, " ").len(), 2);
        assert_eq!(service_status(&all[0]), ("运行中", ui::SUCCESS));
        assert_eq!(service_status(&all[1]).0, "已生效");
        assert_eq!(service_status(&service("mysql.service", "", "failed", "failed")), ("失败", ui::DESTRUCTIVE));
        assert_eq!(service_status(&service("redis.service", "", "inactive", "dead")).0, "已停止");
        assert_eq!(service_status(&service("docker.service", "", "activating", "start-pre")).0, "启动中");
    }

    fn container(name: &str, state: &str, status: &str) -> ContainerInfo {
        ContainerInfo { id: format!("{name}0123"), name: name.into(), image: "nginx:alpine".into(), state: state.into(), status: status.into(), ports: "0.0.0.0:8080->80/tcp".into() }
    }

    #[test]
    fn containers_are_searched_and_crashes_stand_out() {
        let all = vec![container("web", "running", "Up 2 hours"), container("db", "exited", "Exited (137) 1 day ago"), container("job", "exited", "Exited (0) 1 day ago")];
        assert_eq!(filter_containers(&all, "8080").len(), 3, "按端口搜");
        assert_eq!(filter_containers(&all, "WEB")[0].name, "web");
        assert_eq!(filter_containers(&all, "db0")[0].name, "db", "按 ID 开头搜");
        assert_eq!(container_status(&all[0]), ("运行中".into(), ui::SUCCESS));
        assert_eq!(container_status(&all[1]), ("退出 137".into(), ui::DESTRUCTIVE));
        assert_eq!(container_status(&all[2]), ("已停止".into(), ui::MUTED_FOREGROUND));
    }

    #[test]
    fn list_errors_explain_missing_permissions() {
        let denied = "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock";
        assert!(list_error_message(View::Containers, denied).contains("docker 组"));
        assert_eq!(list_error_message(View::Processes, "boom\n"), "读取失败：boom");
    }

    #[test]
    fn names_and_kill_errors_read_well() {
        assert_eq!(process_name("/usr/sbin/sshd -D"), "sshd");
        assert_eq!(process_name("nginx: worker process"), "nginx");
        assert_eq!(process_name("python3 app.py"), "python3");
        assert!(kill_error_message("kill: (1) - Operation not permitted").contains("sudo -i"));
        assert_eq!(kill_error_message("kill: (99999) - No such process"), "进程已经不在了。");
    }
}

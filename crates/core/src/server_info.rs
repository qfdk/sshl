//! 「系统」页：进程、监听端口、负载 / 内存 / 磁盘，以及结束进程。看的是某个 SSH 会话那台服务器，或者本机（[`Machine`]）。
//! 远程的命令以文件管理当前身份跑（终端里 sudo -i 之后就是 root：看得到所有端口对应的进程、能结束 root 的进程）；
//! 本机的以 SSHL 自己的身份跑，同一套命令。
//! Linux 用 ss / netstat / /proc；macOS 用 netstat -anvl（不是 root 也看得到每个端口的进程）/ vm_stat / sysctl，其余退到 lsof。

use std::collections::HashMap;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// 「系统」页看的是哪台机器。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Machine {
    /// SSH 会话（会话 id）那台服务器。
    Remote(String),
    /// 这台 Mac（本机终端标签）。
    Local,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    /// 父进程（界面把 nginx 的 master 和 worker、postgres 和它的后台进程这样的父子合成一行）。
    pub ppid: u32,
    pub user: String,
    pub cpu: f32,
    pub mem: f32,
    /// 常驻内存，KB。
    pub rss_kb: u64,
    pub command: String,
    /// 命令行开头的程序（可执行文件，macOS 上是完整路径，里面可能有空格）。从 ps 的 comm 来，命令行以它开头才算；
    /// None 时程序就是命令行的第一个词。
    pub program: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// tcp / udp（v6 也归到这两个）。
    pub proto: String,
    /// 监听地址（`0.0.0.0`、`[::]`、`127.0.0.1`、`*`）。
    pub address: String,
    pub port: u16,
    /// 没权限时看不到别人的进程：None。
    pub pid: Option<u32>,
    pub process: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overview {
    pub load: Option<[f32; 3]>,
    pub cpus: Option<u32>,
    pub mem_total: Option<u64>,
    pub mem_available: Option<u64>,
    /// 根分区，字节。
    pub disk_total: Option<u64>,
    pub disk_used: Option<u64>,
    pub uptime_secs: Option<u64>,
}

/// 全部进程。args 放最后：命令行里有空格。后面附每个进程的 comm（macOS 上是程序的完整路径，用来切出带空格的程序名），
/// 读不到 comm 不算失败。用户列：Linux 的 ps 默认只给 8 个字、长的截成「systemd+」，认得 `user:64` 时用宽的；
/// macOS / BusyBox 不认（macOS 只警告一句、照样输出缺了这一列的行，所以先拿自己试一下，不能失败了再跑一遍）。
pub const PROCESSES_COMMAND: &str = "sh -c 'u=user; LC_ALL=C ps -o user:64= -p $$ >/dev/null 2>&1 && u=user:64; \
LC_ALL=C ps -eo pid=,ppid=,$u=,pcpu=,pmem=,rss=,args= && { echo @@comm; LC_ALL=C ps -eo pid=,comm= 2>/dev/null; true; }'";

pub fn parse_processes(out: &str) -> Vec<ProcessInfo> {
    let (list, comm) = out.split_once("\n@@comm\n").unwrap_or((out, ""));
    let programs: HashMap<u32, String> = comm.lines().filter_map(comm_entry).collect();
    list.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let user = it.next()?.to_string();
            let cpu = it.next()?.parse().ok()?;
            let mem = it.next()?.parse().ok()?;
            let rss_kb = it.next()?.parse().ok()?;
            let command = it.collect::<Vec<_>>().join(" ");
            let program = programs
                .get(&pid)
                .filter(|program| command.strip_prefix(program.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with(' ')))
                .cloned();
            Some(ProcessInfo { pid, ppid, user, cpu, mem, rss_kb, command, program })
        })
        .filter(|process| !is_own_probe(&process.command))
        .collect()
}

/// SSHL 自己取数据跑的命令：几条命令都用 `@@xxx` 分隔各段输出（sh -c 那一层的命令行里带着），加上里面实际干活的 ps。
fn is_own_probe(command: &str) -> bool {
    command.contains(" @@") || command.starts_with("ps -eo pid=")
}

/// 监听端口：有 ss 用 ss，没有用 netstat（Linux 的 -p）；macOS 的 netstat -anvl 带 `进程名:pid`（不是 root 也有别人的，
/// lsof 只看得到自己的），名字被截到 16 个字，后面附 ps 的完整名字；再没有用 lsof（BSD、老 macOS）。`@@` 行标出下面是哪个的输出。
pub const PORTS_COMMAND: &str = "sh -c 'if command -v ss >/dev/null 2>&1; then echo @@ss; ss -lntup 2>/dev/null; \
elif netstat -lntup >/dev/null 2>&1; then echo @@netstat; netstat -lntup 2>/dev/null; \
elif [ \"$(uname -s)\" = Darwin ] && netstat -anvl -p tcp 2>/dev/null | grep -q process:pid; then \
echo @@darwin; netstat -anvl -p tcp 2>/dev/null; netstat -anvl -p udp 2>/dev/null; echo @@comm; ps -axo pid=,comm= 2>/dev/null; \
elif command -v lsof >/dev/null 2>&1; then echo @@lsof; lsof +c 0 -nP -iTCP -sTCP:LISTEN -iUDP 2>/dev/null; fi'";

/// `addr:port`（IPv6 是 `[::]:22`，ss 里可能带 `%lo` 之类的网卡后缀）→ (地址, 端口)。
fn split_address(value: &str) -> Option<(String, u16)> {
    let (address, port) = value.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let address = address.split('%').next().unwrap_or(address);
    Some((address.to_string(), port))
}

fn proto_of(value: &str) -> String {
    if value.to_ascii_lowercase().starts_with("udp") { "udp".into() } else { "tcp".into() }
}

/// ss 的 `users:(("nginx",pid=1234,fd=6),(...))` → 第一个 (进程名, pid)。
fn ss_user(rest: &str) -> (Option<String>, Option<u32>) {
    let Some(start) = rest.find("((\"") else { return (None, None) };
    let rest = &rest[start + 3..];
    let Some((name, rest)) = rest.split_once('"') else { return (None, None) };
    let pid = rest.split_once("pid=").and_then(|(_, r)| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|p| p.parse().ok());
    (Some(name.to_string()), pid)
}

/// macOS netstat 里的进程名是内核的 p_comm，最多这么多字节（MAXCOMLEN），长的被截断。
const NETSTAT_NAME_MAX: usize = 16;

/// macOS `netstat -anvl` 表头里 `process:pid` 那一列：前面几列（按 TCP 行数；UDP 没有状态列，少一列）、后面几列。
/// 进程名里可能有空格，只能从两头数。
#[derive(Clone, Copy)]
struct OwnerColumn {
    before: usize,
    after: usize,
}

fn owner_column(header: &str) -> Option<OwnerColumn> {
    let fields: Vec<&str> = header.split_whitespace().collect();
    let at = fields.iter().position(|f| *f == "process:pid")?;
    // 「Local Address」「Foreign Address」在表头里各是两个词，数据里是一列
    let split = fields[..at].iter().filter(|f| **f == "Address").count();
    Some(OwnerColumn { before: at - split, after: fields.len() - at - 1 })
}

/// netstat 的地址 `127.0.0.1.8080` / `*.5353` / `fe80::1%en0.49157`：端口在最后一个点后面。
/// IPv6 加方括号（同 ss 的 `[::1]`），网卡后缀去掉。
fn darwin_address(value: &str) -> Option<(String, u16)> {
    let (address, port) = value.rsplit_once('.')?;
    let port = port.parse().ok()?;
    let address = address.split('%').next().unwrap_or(address);
    Some((if address.contains(':') { format!("[{address}]") } else { address.to_string() }, port))
}

/// Proto Recv-Q Send-Q Local Foreign [(state)] rxbytes txbytes rhiwat shiwat 进程名:pid state options …
fn parse_darwin(line: &str, column: OwnerColumn) -> Option<PortInfo> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let proto = *fields.first()?;
    let udp = proto.starts_with("udp");
    // TCP 只要在监听的；UDP 没有状态，连着对端的是往外的连接
    let listening = if udp { fields.get(4) == Some(&"*.*") } else { proto.starts_with("tcp") && fields.get(5) == Some(&"LISTEN") };
    if !listening {
        return None;
    }
    let (address, port) = darwin_address(fields.get(3)?)?;
    let start = column.before.checked_sub(usize::from(udp))?;
    let end = fields.len().checked_sub(column.after)?;
    let owner = fields.get(start..end)?.join(" ");
    let (name, pid) = owner.rsplit_once(':')?;
    Some(PortInfo { proto: proto_of(proto), address, port, pid: pid.parse().ok(), process: Some(name.to_string()).filter(|n| !n.is_empty()) })
}

/// `ps -o pid=,comm=` 的一行 → (pid, comm)。macOS 的 comm 是可执行文件的完整路径（里面可能有空格）；
/// 连续的空白并成一个，同 [`parse_processes`] 里的命令行。
fn comm_entry(line: &str) -> Option<(u32, String)> {
    let (pid, comm) = line.trim().split_once(char::is_whitespace)?;
    let comm = comm.split_whitespace().collect::<Vec<_>>().join(" ");
    if comm.is_empty() {
        return None;
    }
    Some((pid.parse().ok()?, comm))
}

/// `ps -axo pid=,comm=` 的一行 → (pid, 程序名)。
fn comm_line(line: &str) -> Option<(u32, String)> {
    let (pid, comm) = comm_entry(line)?;
    let name = comm.rsplit('/').next()?;
    if name.is_empty() {
        return None;
    }
    Some((pid, name.to_string()))
}

pub fn parse_ports(out: &str) -> Vec<PortInfo> {
    let mut tool = "";
    let mut owner: Option<OwnerColumn> = None;
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut ports: Vec<PortInfo> = Vec::new();
    for line in out.lines() {
        if let Some(marker) = line.trim().strip_prefix("@@") {
            tool = marker;
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let parsed = match tool {
            // Netid State Recv-Q Send-Q Local:Port Peer:Port [Process]
            "ss" if fields.len() >= 5 && fields[0] != "Netid" => split_address(fields[4]).map(|(address, port)| {
                let (process, pid) = ss_user(&fields[5..].join(" "));
                PortInfo { proto: proto_of(fields[0]), address, port, pid, process }
            }),
            // Proto Recv-Q Send-Q Local Foreign [State] PID/Program
            "netstat" if fields.len() >= 6 && (fields[0].starts_with("tcp") || fields[0].starts_with("udp")) => {
                split_address(fields[3]).map(|(address, port)| {
                    let owner = fields.last().copied().unwrap_or("-");
                    let (pid, process) = match owner.split_once('/') {
                        Some((pid, name)) => (pid.parse().ok(), Some(name.to_string())),
                        None => (None, None),
                    };
                    PortInfo { proto: proto_of(fields[0]), address, port, pid, process }
                })
            }
            // COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME [(LISTEN)]
            "lsof" if fields.len() >= 9 && fields[0] != "COMMAND" && !fields[8].contains("->") => {
                split_address(fields[8]).map(|(address, port)| PortInfo {
                    proto: proto_of(fields[7]),
                    address,
                    port,
                    pid: fields[1].parse().ok(),
                    // lsof 把名字里的空格等写成 \x20
                    process: Some(fields[0].replace("\\x20", " ")),
                })
            }
            "darwin" if fields.first() == Some(&"Proto") => {
                owner = owner_column(line);
                None
            }
            "darwin" => owner.and_then(|column| parse_darwin(line, column)),
            "comm" => {
                names.extend(comm_line(line));
                None
            }
            _ => None,
        };
        // 同一进程在 IPv4 / IPv6、多个 worker 上重复监听：同协议同端口同地址只留一条
        if let Some(port) = parsed {
            if !ports.iter().any(|p| p.proto == port.proto && p.port == port.port && p.address == port.address && p.pid == port.pid) {
                ports.push(port);
            }
        }
    }
    // netstat 的进程名最多 16 个字节（`com.docker.backe`）：被截断的换成 ps 里以它开头的完整名字。
    // ps 的 comm 是 argv[0]，程序可以改写（redis 写成「redis-server 127.0.0.1:6379」），没被截断的照用 netstat 的
    for port in &mut ports {
        let full = port.pid.and_then(|pid| names.get(&pid));
        if let (Some(short), Some(full)) = (port.process.as_deref(), full) {
            if short.len() >= NETSTAT_NAME_MAX && full.starts_with(short) {
                port.process = Some(full.clone());
            }
        }
    }
    ports.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.proto.cmp(&b.proto)));
    ports
}

/// 负载 / 内存 / 磁盘 / 运行时间。Linux 读 /proc，macOS / BSD 退到 sysctl（内存用量看 vm_stat）；读不到的项留空。
/// macOS 的磁盘标 `@@apfs`：/ 是只读的系统卷，和数据卷共用一个容器，已用要按容器算。
pub const OVERVIEW_COMMAND: &str = "sh -c 'echo @@load; cat /proc/loadavg 2>/dev/null || sysctl -n vm.loadavg 2>/dev/null; \
echo @@cpus; nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null; \
echo @@mem; cat /proc/meminfo 2>/dev/null || { echo \"MemTotal: $(sysctl -n hw.memsize 2>/dev/null) B\"; vm_stat 2>/dev/null; }; \
echo @@uptime; cat /proc/uptime 2>/dev/null || echo \"boot $(sysctl -n kern.boottime 2>/dev/null | sed -n \"s/^{ sec = \\([0-9]*\\),.*/\\1/p\") now $(date +%s)\"; \
if [ \"$(uname -s)\" = Darwin ]; then echo @@apfs; else echo @@df; fi; df -kP / 2>/dev/null | tail -n 1'";

/// macOS `vm_stat` 里算已用内存要的几项（页数）和页大小。
#[derive(Default)]
struct VmStat {
    page_size: Option<u64>,
    anonymous: Option<u64>,
    purgeable: Option<u64>,
    wired: Option<u64>,
    compressed: Option<u64>,
}

impl VmStat {
    fn read(&mut self, line: &str) {
        // Mach Virtual Memory Statistics: (page size of 16384 bytes)
        if let Some((_, rest)) = line.split_once("page size of ") {
            self.page_size = rest.split_whitespace().next().and_then(|n| n.parse().ok());
            return;
        }
        // Pages wired down:                             337382.
        let Some((key, value)) = line.rsplit_once(':') else { return };
        let value = value.trim().trim_end_matches('.').parse().ok();
        match key.trim() {
            "Anonymous pages" => self.anonymous = value,
            "Pages purgeable" => self.purgeable = value,
            "Pages wired down" => self.wired = value,
            "Pages occupied by compressor" => self.compressed = value,
            _ => {}
        }
    }

    /// 同活动监视器的「已使用内存」：App 内存（匿名页减去可清除的）+ 联动内存 + 被压缩的，字节。
    fn used(&self) -> Option<u64> {
        let pages = self.anonymous?.saturating_sub(self.purgeable.unwrap_or(0)) + self.wired? + self.compressed?;
        Some(pages * self.page_size?)
    }
}

pub fn parse_overview(out: &str) -> Overview {
    let mut overview = Overview::default();
    let mut vm = VmStat::default();
    let mut section = "";
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            section = match name.trim() {
                "load" => "load",
                "cpus" => "cpus",
                "mem" => "mem",
                "uptime" => "uptime",
                "df" => "df",
                "apfs" => "apfs",
                _ => "",
            };
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().filter(|f| *f != "{" && *f != "}").collect();
        match section {
            "load" if overview.load.is_none() && fields.len() >= 3 => {
                if let (Ok(a), Ok(b), Ok(c)) = (fields[0].parse(), fields[1].parse(), fields[2].parse()) {
                    overview.load = Some([a, b, c]);
                }
            }
            "cpus" if overview.cpus.is_none() => overview.cpus = fields.first().and_then(|v| v.parse().ok()),
            "mem" => {
                let value = || -> Option<u64> {
                    let n: u64 = fields.get(1)?.parse().ok()?;
                    Some(if fields.get(2) == Some(&"kB") { n * 1024 } else { n })
                };
                match fields.first().copied() {
                    Some("MemTotal:") => overview.mem_total = value(),
                    Some("MemAvailable:") => overview.mem_available = value(),
                    _ => vm.read(line),
                }
            }
            "uptime" => {
                if let Some(secs) = fields.first().and_then(|v| v.parse::<f64>().ok()) {
                    overview.uptime_secs = Some(secs as u64);
                } else if let (Some("boot"), Some(boot), Some("now"), Some(now)) = (fields.first().copied(), fields.get(1), fields.get(2).copied(), fields.get(3)) {
                    if let (Ok(boot), Ok(now)) = (boot.parse::<u64>(), now.parse::<u64>()) {
                        overview.uptime_secs = now.checked_sub(boot);
                    }
                }
            }
            // Filesystem 1024-blocks Used Available Capacity Mounted
            "df" | "apfs" if fields.len() >= 6 => {
                let kb = |i: usize| fields[i].parse::<u64>().ok().map(|k| k * 1024);
                overview.disk_total = kb(1);
                overview.disk_used = if section == "apfs" { kb(1).zip(kb(3)).map(|(total, free)| total.saturating_sub(free)) } else { kb(2) };
            }
            _ => {}
        }
    }
    // macOS 没有 MemAvailable：总量减去 vm_stat 算出的已用
    if overview.mem_available.is_none() {
        if let (Some(total), Some(used)) = (overview.mem_total, vm.used()) {
            overview.mem_available = Some(total.saturating_sub(used));
        }
    }
    overview
}

pub fn kill_command(pid: u32, force: bool) -> String {
    format!("kill -{} {pid}", if force { "KILL" } else { "TERM" })
}

pub(crate) async fn run(state: &AppState, machine: &Machine, command: &str) -> AppResult<String> {
    let (status, out, err) = exec(state, machine, command).await?;
    if status != 0 {
        let err = err.trim();
        return Err(AppError::Other(if err.is_empty() { format!("`{command}` exited {status}") } else { err.to_string() }));
    }
    Ok(out)
}

/// 同 [`exec`]，stdout 边到边交给 `on_stdout`（长命令要边跑边看：磁盘扫描先拿到 du 的进程号，好中途停下）。
pub(crate) async fn exec_watching(
    state: &AppState,
    machine: &Machine,
    command: &str,
    on_stdout: &mut (dyn FnMut(&[u8]) + Send),
) -> AppResult<(u32, String, String)> {
    match machine {
        Machine::Remote(session_id) => {
            let session = state.get(session_id).await.ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
            crate::sftp::exec_as_file_user_watching(&session, command, on_stdout).await
        }
        Machine::Local => {
            use tokio::io::AsyncReadExt;
            let mut child = tokio::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(command)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;
            let (mut stdout, mut stderr) = (child.stdout.take(), child.stderr.take());
            let read_out = async {
                let (mut out, mut chunk) = (Vec::new(), [0u8; 16 * 1024]);
                while let Some(stdout) = stdout.as_mut() {
                    let n = stdout.read(&mut chunk).await?;
                    if n == 0 {
                        break;
                    }
                    on_stdout(&chunk[..n]);
                    out.extend_from_slice(&chunk[..n]);
                }
                Ok::<_, std::io::Error>(out)
            };
            let read_err = async {
                let mut err = Vec::new();
                if let Some(stderr) = stderr.as_mut() {
                    stderr.read_to_end(&mut err).await?;
                }
                Ok::<_, std::io::Error>(err)
            };
            let (out, err) = tokio::try_join!(read_out, read_err)?;
            let status = child.wait().await?.code().map_or(255, |code| code as u32);
            Ok((status, String::from_utf8_lossy(&out).into_owned(), String::from_utf8_lossy(&err).into_owned()))
        }
    }
}

/// 跑一条命令，退出码、stdout、stderr 都要（退出码非 0 也照样给输出：du 读不了某些目录时就是这样）。
/// 远程以文件管理当前身份，本机以 SSHL 自己的身份用 /bin/sh。
pub(crate) async fn exec(state: &AppState, machine: &Machine, command: &str) -> AppResult<(u32, String, String)> {
    match machine {
        Machine::Remote(session_id) => {
            let session = state.get(session_id).await.ok_or_else(|| AppError::SessionNotFound(session_id.clone()))?;
            crate::sftp::exec_as_file_user(&session, command).await
        }
        Machine::Local => {
            let output = tokio::process::Command::new("/bin/sh").arg("-c").arg(command).stdin(std::process::Stdio::null()).output().await?;
            // 被信号杀掉的没有退出码：按 255 算
            let status = output.status.code().map_or(255, |code| code as u32);
            Ok((status, String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned()))
        }
    }
}

pub async fn processes(state: &AppState, machine: &Machine) -> AppResult<Vec<ProcessInfo>> {
    Ok(parse_processes(&run(state, machine, PROCESSES_COMMAND).await?))
}

pub async fn ports(state: &AppState, machine: &Machine) -> AppResult<Vec<PortInfo>> {
    Ok(parse_ports(&run(state, machine, PORTS_COMMAND).await?))
}

pub async fn overview(state: &AppState, machine: &Machine) -> AppResult<Overview> {
    Ok(parse_overview(&run(state, machine, OVERVIEW_COMMAND).await?))
}

/// 结束进程（TERM，force 时 KILL）。没权限时报错里带原话（Operation not permitted），界面据此提议在终端里 sudo。
/// 本机直接发信号，不经过 shell。
pub async fn kill(state: &AppState, machine: &Machine, pid: u32, force: bool) -> AppResult<()> {
    if *machine != Machine::Local {
        return run(state, machine, &kill_command(pid, force)).await.map(|_| ());
    }
    let target = local_pid(pid).ok_or_else(|| AppError::Other(format!("not a process id: {pid}")))?;
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: kill(2) 只发信号；target > 0，只发给这一个进程
    if unsafe { libc::kill(target, signal) } == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

/// kill(2) 的 pid：0 是自己所在的进程组（连 SSHL 一起结束），负数是进程组，-1 是自己的所有进程。只认单个进程号。
fn local_pid(pid: u32) -> Option<libc::pid_t> {
    libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 0)
}

/// 是什么系统：内核名（`uname -s`）和 os-release 里的发行版。os-release 先看 /etc 的，没有再看 /usr/lib 的（同 os-release(5)）；
/// macOS、OpenBSD 没有 os-release，只有内核名。
pub const OS_COMMAND: &str =
    "sh -c 'echo @@uname; uname -s; echo @@os-release; cat /etc/os-release 2>/dev/null || cat /usr/lib/os-release 2>/dev/null; true'";

/// [`OS_COMMAND`] 的输出 → `内核` 或 `内核/发行版`，都是小写：`linux/ubuntu`、`linux`（没有 os-release）、`darwin`、`freebsd/freebsd`。
/// 认不出（命令没跑起来、输出不是它的：Windows 的 cmd、只许跑固定命令的账号）是 None。
/// 两段都是服务器给的：只收 os-release 规定 ID 能用的字符（a–z、0–9、`.` `_` `-`，大写的转小写：deepin 20 写的是 `ID=Deepin`），
/// 不像的、太长的那段不要。
pub fn parse_os(out: &str) -> Option<String> {
    #[derive(PartialEq)]
    enum Section {
        Uname,
        Release,
    }
    fn word(value: &str) -> Option<String> {
        let value = value.trim().to_ascii_lowercase();
        let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
        (!value.is_empty() && value.len() <= 32 && value.chars().all(allowed)).then_some(value)
    }
    let (mut section, mut kernel, mut distro) = (None, None, None);
    for line in out.lines() {
        match line.trim() {
            "@@uname" => section = Some(Section::Uname),
            "@@os-release" => section = Some(Section::Release),
            _ if section == Some(Section::Uname) && kernel.is_none() => kernel = word(line),
            _ if section == Some(Section::Release) && distro.is_none() => {
                // ID=ubuntu / ID="centos" / ID='opensuse-leap'；ID_LIKE=、VERSION_ID= 不是
                if let Some(value) = line.strip_prefix("ID=") {
                    let value = value.trim();
                    let unquoted = ['"', '\''].iter().find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q)).unwrap_or(value);
                    distro = word(unquoted);
                }
            }
            _ => {}
        }
    }
    let kernel = kernel?;
    Some(match distro {
        Some(distro) => format!("{kernel}/{distro}"),
        None => kernel,
    })
}

/// 认一下这台机器是什么系统（见 [`parse_os`]）。认不出是 Ok(None)。
pub async fn os(state: &AppState, machine: &Machine) -> AppResult<Option<String>> {
    let (_, out, _) = exec(state, machine, OS_COMMAND).await?;
    Ok(parse_os(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SSHL 自己取数据跑的命令（带 `@@` 分隔标记的 sh -c、里面的 ps）不列：每 3 秒一批，列出来只是噪音。
    #[test]
    fn our_own_probes_are_left_out() {
        let out = "\
 4517  4516 deploy    0.0  0.0   1500 sh -c LC_ALL=C ps -eo pid=,ppid=,user=,pcpu=,pmem=,rss=,args= && { echo @@comm; LC_ALL=C ps -eo pid=,comm= 2>/dev/null; true; }\n\
 4518  4516 deploy    0.0  0.0   1600 sh -c echo @@load; cat /proc/loadavg 2>/dev/null\n\
 4519  4517 deploy    0.0  0.0   3500 ps -eo pid=,ppid=,user=,pcpu=,pmem=,rss=,args=\n\
 4497  4496 deploy    0.0  0.0   3800 -bash\n\
   20    19 www-data  0.0  0.0   2900 nginx: worker process\n";
        let procs = parse_processes(out);
        assert_eq!(procs.iter().map(|p| p.pid).collect::<Vec<_>>(), [4497, 20]);
    }

    #[test]
    fn parses_gnu_and_bsd_ps() {
        let out = "    1     0 root      0.0  0.1  11236 /sbin/init splash\n 1234  1200 www-data 12.5  2.3 190000 nginx: worker process\n  363     1 root               0.0  0.0   2624 /usr/libexec/dprivacyd\nbad line\n";
        let procs = parse_processes(out);
        assert_eq!(procs.len(), 3);
        assert_eq!(
            procs[1],
            ProcessInfo { pid: 1234, ppid: 1200, user: "www-data".into(), cpu: 12.5, mem: 2.3, rss_kb: 190000, command: "nginx: worker process".into(), program: None }
        );
        assert_eq!((procs[0].ppid, procs[2].ppid), (0, 1), "父进程号（远程按它把 nginx 的 master 和 worker 合起来）");
        assert_eq!(procs[2].command, "/usr/libexec/dprivacyd");
    }

    /// macOS 的程序路径里常有空格（Chrome 的 Helper），光看命令行切不出程序名；ps 的 comm 是完整路径，按 pid 对上、
    /// 命令行以它开头时就是程序。Linux 的 comm 是内核里最多 15 个字的短名，`nginx: worker process` 这类还改写过命令行，对不上的不算。
    #[test]
    fn the_program_comes_from_comm_when_the_command_line_starts_with_it() {
        let out = "\
 3130   600 qfdk      3.1  0.5 171000 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome Helper (Aperitif) --type=gpu-process\n\
 9140  9100 qfdk      9.3  1.2 391000 claude\n\
 1234  1200 www-data 12.5  2.3 190000 nginx: worker process\n\
  700     1 root      0.0  0.0   1000 /usr/sbin/sshd -D\n\
   42     2 root      0.0  0.0      0 [kworker/u4:0-events_unbound]\n\
@@comm\n\
 3130 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome Helper (Aperitif)\n\
 9140 claude\n\
 1234 nginx\n\
  700 sshd\n\
   42 kworker/u4:0-ev\n";
        let procs = parse_processes(out);
        assert_eq!(procs.iter().map(|p| p.pid).collect::<Vec<_>>(), [3130, 9140, 1234, 700, 42], "@@comm 下面的不是进程");
        assert_eq!(procs[0].program.as_deref(), Some("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome Helper (Aperitif)"));
        assert_eq!(procs[1].program.as_deref(), Some("claude"), "整个命令行就是程序");
        assert_eq!(procs[2].program, None, "nginx 改写了命令行，「nginx」后面不是空格");
        assert_eq!(procs[3].program, None, "Linux 的 comm 只是短名");
        assert_eq!(procs[4].program, None);
        assert_eq!(parse_processes("    1 0 root 0.0 0.1 11236 /sbin/init splash\n")[0].program, None, "没有 comm 那段（老输出）照样能读");
    }

    #[test]
    fn parses_ss_with_and_without_owner() {
        let out = "@@ss\nNetid State  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\n\
tcp   LISTEN 0      511          0.0.0.0:80        0.0.0.0:*    users:((\"nginx\",pid=1234,fd=6),(\"nginx\",pid=1235,fd=6))\n\
tcp   LISTEN 0      511             [::]:80           [::]:*    users:((\"nginx\",pid=1234,fd=7))\n\
tcp   LISTEN 0      4096   127.0.0.53%lo:53        0.0.0.0:*\n\
udp   UNCONN 0      0            0.0.0.0:68        0.0.0.0:*    users:((\"dhclient\",pid=700,fd=7))\n";
        let ports = parse_ports(out);
        assert_eq!(ports.len(), 4);
        assert_eq!(ports[0], PortInfo { proto: "tcp".into(), address: "127.0.0.53".into(), port: 53, pid: None, process: None });
        assert_eq!(ports[1], PortInfo { proto: "udp".into(), address: "0.0.0.0".into(), port: 68, pid: Some(700), process: Some("dhclient".into()) });
        assert_eq!((ports[2].port, ports[2].pid, ports[2].address.as_str()), (80, Some(1234), "0.0.0.0"));
        assert_eq!(ports[3].address, "[::]");
    }

    #[test]
    fn parses_netstat_and_lsof() {
        let netstat = "@@netstat\nActive Internet connections (only servers)\nProto Recv-Q Send-Q Local Address Foreign Address State PID/Program name\n\
tcp        0      0 0.0.0.0:22              0.0.0.0:*               LISTEN      812/sshd\n\
tcp6       0      0 :::3306                 :::*                    LISTEN      -\n\
udp        0      0 0.0.0.0:68              0.0.0.0:*                           700/dhclient\n";
        let ports = parse_ports(netstat);
        assert_eq!(ports.iter().map(|p| (p.port, p.pid)).collect::<Vec<_>>(), [(22, Some(812)), (68, Some(700)), (3306, None)]);
        assert_eq!(ports[0].process.as_deref(), Some("sshd"));
        let lsof = "@@lsof\nCOMMAND     PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME\n\
rapportd   1089 qfdk   16u  IPv6 0x4cecfc906653f861      0t0  TCP *:59631 (LISTEN)\n\
mDNSRespo   400 root    7u  IPv4 0x1111111111111111      0t0  UDP *:5353\n\
Chrome     2222 qfdk   30u  IPv4 0x2222222222222222      0t0  UDP 192.168.1.2:51000->1.2.3.4:443\n";
        let ports = parse_ports(lsof);
        assert_eq!(ports.len(), 2, "连出去的 UDP 不算监听");
        assert_eq!(ports[0], PortInfo { proto: "udp".into(), address: "*".into(), port: 5353, pid: Some(400), process: Some("mDNSRespo".into()) });
        assert_eq!(ports[1].port, 59631);
        let spaced = parse_ports("@@lsof\nBoom\\x203D 1539 me 5u IPv4 0x1 0t0 TCP *:49170 (LISTEN)\n");
        assert_eq!(spaced[0].process.as_deref(), Some("Boom 3D"));
    }

    /// macOS 的 `netstat -anvl`（本机实测的行）：别的用户的端口也带进程（lsof 不是 root 时只看得到自己的）；
    /// 进程名可能带空格、被截到 16 个字节，后面附的 ps 补全被截断的名字。ps 的 comm 是 argv[0]，程序可以改写
    /// （redis 改成「redis-server 127.0.0.1:6379」、claude 的可执行文件叫 2.1.325），没被截断的照用 netstat 的。
    #[test]
    fn parses_macos_netstat_with_every_owner() {
        let header = "Proto Recv-Q Send-Q  Local Address                                 Foreign Address                               (state)          rxbytes      txbytes  rhiwat  shiwat          process:pid    state  options           gencnt    flags   flags1 usecnt rtncnt fltrs";
        let out = format!(
            "@@darwin\nActive Internet connections (including servers)\n{header}\n\
tcp4       0      0  192.168.1.70.55822                            34.149.66.165.443                             ESTABLISHED        10944        50549  131072  131600          2.1.325:2426   00102 00000008 0000000000115f13 00000001 04000900      2      0 000000\n\
tcp4       0      0  127.0.0.1.55714                               *.*                                           LISTEN                 0            0  131072  131072          2.1.325:9140   00100 00000006 0000000000114c36 00000001 00000800      1      0 000000\n\
tcp46      0      0  *.27017                                       *.*                                           LISTEN                 0            0  131072  131072 com.docker.backe:6020   00100 00000006 000000000004f689 00000000 00000800      1      0 000000\n\
tcp4       0      0  127.0.0.1.12434                               *.*                                           LISTEN                 0            0  131072  131072 com.docker.backe:6020   00100 00000006 000000000004f42e 00000000 00000800      1      0 000000\n\
tcp4       0      0  127.0.0.1.9222                                *.*                                           LISTEN                 0            0  131072  131072    Google Chrome:3124   00100 00000006 00000000000050e8 00000000 00000800      1      0 000000\n\
tcp6       0      0  ::1.6379                                      *.*                                           LISTEN                 0            0  131072  131072     redis-server:778    00100 00000006 000000000000174f 00000000 00000800      1      0 000000\n\
tcp6       0      0  fe80::d07c:81ff:fece:7e24%anri0.49157         *.*                                           LISTEN                 0            0  131072  131072          remoted:373    00180 00000006 0000000000000a18 00000000 00000800      1      0 000000\n\
Active Internet connections (including servers)\n{header}\n\
udp4       0      0  *.56788                                       *.*                                                              48912         7516       0    9216    WeatherWidget:924    ffffa130 00020000 000000000011376c 10100009 04280908      1      0 000002\n\
udp46      0      0  *.5353                                        *.*                                                               4848         2004  786896    9216 Google Chrome He:3131   00100 00000204 0000000000112b25 00000000 00000800      1      0 000002\n\
udp46      0      0  *.5353                                        *.*                                                               4848         2004  786896    9216 Google Chrome He:3131   00100 00000204 0000000000112b24 00000000 00000800      1      0 000002\n\
udp4       0      0  192.168.1.70.61234                            1.1.1.1.53                                                         100          200       0    9216    mDNSResponder:516    00100 00000204 0000000000112b11 00000000 00000800      1      0 000002\n\
@@comm\n 9140 claude\n 6020 /Applications/Docker.app/Contents/MacOS/com.docker.backend\n  778 /opt/homebrew/opt/redis/bin/redis-server 127.0.0.1:6379\n\
 3131 /Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/157.0.8103.79/Helpers/Google Chrome Helper (Aperitif).app/Contents/MacOS/Google Chrome Helper (Aperitif)\n"
        );
        let ports = parse_ports(&out);
        let rows: Vec<_> = ports.iter().map(|p| (p.proto.as_str(), p.address.as_str(), p.port, p.pid, p.process.as_deref())).collect();
        assert_eq!(
            rows,
            [
                ("udp", "*", 5353, Some(3131), Some("Google Chrome Helper (Aperitif)")),
                ("tcp", "[::1]", 6379, Some(778), Some("redis-server")),
                ("tcp", "127.0.0.1", 9222, Some(3124), Some("Google Chrome")),
                ("tcp", "127.0.0.1", 12434, Some(6020), Some("com.docker.backend")),
                ("tcp", "*", 27017, Some(6020), Some("com.docker.backend")),
                ("tcp", "[fe80::d07c:81ff:fece:7e24]", 49157, Some(373), Some("remoted")),
                ("tcp", "127.0.0.1", 55714, Some(9140), Some("2.1.325")),
                ("udp", "*", 56788, Some(924), Some("WeatherWidget")),
            ],
            "连着的（ESTABLISHED、连了对端的 UDP）不算；同一进程重复监听的只留一条"
        );
    }

    #[test]
    fn parses_linux_and_macos_overview() {
        let linux = "@@load\n0.52 0.58 0.59 1/389 12345\n@@cpus\n4\n@@mem\nMemTotal:        8000000 kB\nMemFree:          100000 kB\nMemAvailable:    6000000 kB\n@@uptime\n123456.78 400000.00\n@@df\n/dev/vda1 51475068 21000000 28000000 43% /\n";
        let o = parse_overview(linux);
        assert_eq!(o.load, Some([0.52, 0.58, 0.59]));
        assert_eq!(o.cpus, Some(4));
        assert_eq!((o.mem_total, o.mem_available), (Some(8_192_000_000), Some(6_144_000_000)));
        assert_eq!(o.uptime_secs, Some(123456));
        assert_eq!((o.disk_total, o.disk_used), (Some(51475068 * 1024), Some(21000000 * 1024)));
        // vm_stat 是本机实测的输出
        let mac = "@@load\n{ 4.07 6.45 5.16 }\n@@cpus\n10\n@@mem\nMemTotal: 34359738368 B\n\
Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
Pages free:                                    57151.\nPages active:                                 797400.\n\
Pages inactive:                               806450.\nPages speculative:                              2055.\n\
Pages throttled:                                   0.\nPages wired down:                             337382.\n\
Pages purgeable:                               57672.\n\"Translation faults\":                      370692587.\n\
File-backed pages:                            664530.\nAnonymous pages:                              941375.\n\
Pages stored in compressor:                   128894.\nPages occupied by compressor:                  44557.\n\
@@uptime\nboot 1794064076 now 1794164076\n@@apfs\n/dev/disk3s1s1   971350180  13438012 474222596     3%    /\n";
        let o = parse_overview(mac);
        assert_eq!(o.load, Some([4.07, 6.45, 5.16]));
        assert_eq!((o.cpus, o.mem_total), (Some(10), Some(34359738368)));
        // 已用同活动监视器的「已使用内存」：App 内存（匿名页 - 可清除）+ 联动 + 压缩
        let used = (941375 - 57672 + 337382 + 44557) * 16384;
        assert_eq!(o.mem_available, Some(34359738368 - used));
        assert_eq!(o.uptime_secs, Some(100000));
        // APFS 的卷共用一个容器：/ 是只读的系统卷（只占 13 GB），已用 = 总量 - 可用
        assert_eq!((o.disk_total, o.disk_used), (Some(971350180 * 1024), Some((971350180 - 474222596) * 1024)));
    }

    /// 本机：看得到这个测试进程自己开的端口（名字完整）和它自己；负载 / 内存 / 磁盘都读得到。
    /// 结束进程真的结束，结束不了的认得出是权限问题。
    /// macOS 上测试程序是 ad-hoc 签名的，它起的 netstat 一个端口都看不到，走的是 lsof（只有自己的进程）；
    /// netstat 那条路见 `local_ports_of_every_user_when_signed`。
    #[tokio::test]
    async fn local_machine_lists_and_kills_for_real() {
        let state = AppState::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let me = std::process::id();
        let ports = ports(&state, &Machine::Local).await.unwrap();
        let mine = ports.iter().find(|p| p.port == port && p.proto == "tcp").unwrap_or_else(|| panic!("没看到自己的端口 {port}：{ports:?}"));
        assert_eq!((mine.pid, mine.address.as_str()), (Some(me), "127.0.0.1"));
        #[cfg(target_os = "macos")]
        {
            let exe = std::env::current_exe().unwrap();
            assert_eq!(mine.process.as_deref(), exe.file_name().and_then(|n| n.to_str()), "测试程序的名字超过 16 个字，不能被截断");
        }
        assert!(processes(&state, &Machine::Local).await.unwrap().iter().any(|p| p.pid == me));

        let o = overview(&state, &Machine::Local).await.unwrap();
        assert!(o.load.is_some() && o.cpus.is_some() && o.uptime_secs.is_some(), "{o:?}");
        let (total, available) = (o.mem_total.unwrap(), o.mem_available.unwrap());
        assert!(available > 0 && available < total, "{o:?}");
        let (disk_total, disk_used) = (o.disk_total.unwrap(), o.disk_used.unwrap());
        assert!(disk_used > 0 && disk_used < disk_total, "{o:?}");

        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        kill(&state, &Machine::Local, child.id(), false).await.unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGTERM));
        let error = kill(&state, &Machine::Local, 1, false).await.unwrap_err().to_string();
        assert!(crate::services::needs_root(&error), "普通用户结束不了 1 号进程，认得出是权限：{error}");
        drop(listener);
    }

    /// macOS 的 netstat 只在进程的祖先里没有 ad-hoc 签名的程序时才列得出端口（实测 macOS 27：同一个程序 ad-hoc 签名时
    /// 一行都没有，Developer ID 签名后正常；装好的 SSHL 和发版包都是 Developer ID 签名的）。cargo 编出来的测试程序是 ad-hoc 的，
    /// 所以要手动签名再跑：
    /// `cargo test -p sshl-core --no-run`，对打出来的 target/debug/build/sshl-core/*/out/sshl_core-* 执行
    /// `codesign -f -s "Developer ID Application: …"`，再 `<它> local_ports_of_every_user_when_signed --ignored`。
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "测试程序要用 Developer ID 签名（ad-hoc 签名的进程起的 netstat 看不到端口）"]
    async fn local_ports_of_every_user_when_signed() {
        let out = run(&AppState::new(), &Machine::Local, PORTS_COMMAND).await.unwrap();
        assert!(out.starts_with("@@darwin"), "走的是 netstat：{}", out.lines().next().unwrap_or(""));
        let ports = parse_ports(&out);
        let mdns = ports.iter().find(|p| p.process.as_deref() == Some("mDNSResponder")).unwrap_or_else(|| panic!("别的用户（_mdnsresponder）的端口也列出来：{ports:?}"));
        assert_eq!((mdns.proto.as_str(), mdns.port), ("udp", 5353));
        assert!(ports.iter().all(|p| p.pid.is_some()), "每个端口都有进程");
    }

    /// 0 和超出 pid_t 的数（转过去是负数）在 kill(2) 里是进程组 / 所有进程，一个信号都不能发。
    #[test]
    fn local_kill_only_targets_a_single_process() {
        assert_eq!(local_pid(4242), Some(4242));
        assert_eq!(local_pid(0), None);
        assert_eq!(local_pid(u32::MAX), None, "当成 -1 就是自己的所有进程");
        assert_eq!(local_pid(i32::MAX as u32 + 1), None);
    }

    /// 真实 sshd（本机 macOS 或 Linux 均可）：几条命令都跑得通、解析得出东西；结束进程真的结束；认出的系统和服务器自己说的一样。
    /// `SSHL_AUTO_AUTH_IT=127.0.0.1:2222 SSHL_AUTO_AUTH_IT_USER=me SSHL_AUTO_AUTH_IT_KEY=/path/key cargo test --lib server_info_against_real_sshd -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_AUTO_AUTH_IT / _USER / _KEY 指向认可该私钥的 sshd"]
    async fn server_info_against_real_sshd() {
        let (Ok(target), Ok(user), Ok(key)) =
            (std::env::var("SSHL_AUTO_AUTH_IT"), std::env::var("SSHL_AUTO_AUTH_IT_USER"), std::env::var("SSHL_AUTO_AUTH_IT_KEY"))
        else {
            return;
        };
        let (host, port) = target.split_once(':').unwrap();
        let handle = crate::ssh::do_handshake(
            host,
            port.parse().unwrap(),
            &user,
            crate::ssh::Credentials::Key { key: &key, passphrase: None },
            crate::known_hosts::HostKeyCheck::trust_any_for_tests(),
        )
        .await
        .unwrap();
        let exec = |cmd: String| {
            let handle = handle.clone();
            async move { crate::sftp::exec_capture(handle.channel_open_session().await.unwrap(), &cmd, None).await.unwrap() }
        };
        let procs = parse_processes(&exec(PROCESSES_COMMAND.into()).await.1);
        assert!(procs.len() > 5 && procs.iter().any(|p| p.pid == 1), "{} 个进程", procs.len());
        // Linux 的 ps 默认把用户名截到 8 个字加「+」（systemd-resolve → systemd+）
        let cut: Vec<&str> = procs.iter().map(|p| p.user.as_str()).filter(|user| user.ends_with('+')).collect();
        assert!(cut.is_empty(), "用户名被截断了：{cut:?}");
        let (_, out, _) = exec(PORTS_COMMAND.into()).await;
        assert!(out.starts_with("@@"), "{out}");
        let ports = parse_ports(&out);
        // 端口转发进来的（如 lima 虚拟机）sshd 在里面听的是 22
        assert!(ports.iter().any(|p| p.port == port.parse::<u16>().unwrap() || p.port == 22), "看得到 sshd 自己监听的端口：{ports:?}");
        let overview = parse_overview(&exec(OVERVIEW_COMMAND.into()).await.1);
        assert!(overview.load.is_some() && overview.cpus.is_some() && overview.mem_total.is_some() && overview.disk_total.is_some(), "{overview:?}");
        let uptime = overview.uptime_secs.unwrap_or(0);
        assert!(uptime > 0 && uptime < 10 * 365 * 86400, "开机时长读对了：{overview:?}");

        // 包一层 sh、断开标准输入输出：exec 通道结束时它不会被一起带走
        let (_, pid, _) = exec("sh -c 'sleep 300 </dev/null >/dev/null 2>&1 & echo $!'".into()).await;
        let pid: u32 = pid.trim().parse().unwrap();
        assert!(parse_processes(&exec(PROCESSES_COMMAND.into()).await.1).iter().any(|p| p.pid == pid));
        let (status, _, _) = exec(kill_command(pid, false)).await;
        assert_eq!(status, 0);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!parse_processes(&exec(PROCESSES_COMMAND.into()).await.1).iter().any(|p| p.pid == pid), "结束了");
        let (status, _, err) = exec(kill_command(1, false)).await;
        assert_ne!(status, 0, "普通用户结束不了 1 号进程：{err}");

        // 认系统走连上之后的那条路（会话上开 exec 通道）；对照是让服务器自己的 sh 读 os-release（macOS 没有这个文件，只有内核名）
        let state = AppState::new();
        state.insert("it".into(), std::sync::Arc::new(crate::ssh::SshSession::for_test(handle.clone(), &user))).await;
        let detected = os(&state, &Machine::Remote("it".into())).await.unwrap().expect("认得出");
        let (_, own, _) = exec("sh -c 'uname -s; . /etc/os-release 2>/dev/null && echo \"$ID\"'".into()).await;
        let own: Vec<String> = own.lines().map(|line| line.trim().to_ascii_lowercase()).collect();
        assert_eq!(detected, own.join("/"));
    }

    /// 本机（macOS 的 ps 不认 `user:64` 这种宽度写法，只警告一句、照样输出缺了用户列的行）：退回普通写法，用户照样读得出来。
    #[test]
    fn process_list_reads_users_on_this_machine() {
        let out = std::process::Command::new("/bin/sh").arg("-c").arg(PROCESSES_COMMAND).output().unwrap();
        let procs = parse_processes(&String::from_utf8_lossy(&out.stdout));
        let init = procs.iter().find(|p| p.pid == 1).expect("有 1 号进程");
        assert_eq!(init.user, "root");
        let me = std::env::var("USER").unwrap();
        assert!(procs.iter().any(|p| p.user == me), "有自己的进程");
    }

    #[test]
    fn kill_is_term_unless_forced() {
        assert_eq!(kill_command(42, false), "kill -TERM 42");
        assert_eq!(kill_command(42, true), "kill -KILL 42");
    }

    /// 常见系统：内核名 + os-release 的 ID（引号去掉，大写转小写）；没有 os-release 的只有内核名。
    #[test]
    fn os_is_the_kernel_and_the_distribution() {
        let ubuntu = "@@uname\nLinux\n@@os-release\nPRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"22.04\"\nID=ubuntu\nID_LIKE=debian\n";
        assert_eq!(parse_os(ubuntu).as_deref(), Some("linux/ubuntu"));
        let centos = "@@uname\nLinux\n@@os-release\nNAME=\"CentOS Stream\"\nID_LIKE=\"rhel fedora\"\nVERSION_ID=\"9\"\nID=\"centos\"\n";
        assert_eq!(parse_os(centos).as_deref(), Some("linux/centos"), "ID_LIKE、VERSION_ID 不是 ID");
        assert_eq!(parse_os("@@uname\nLinux\n@@os-release\nID='opensuse-leap'\n").as_deref(), Some("linux/opensuse-leap"));
        assert_eq!(parse_os("@@uname\nLinux\n@@os-release\nNAME=\"Deepin\"\nID=Deepin\n").as_deref(), Some("linux/deepin"));
        assert_eq!(parse_os("@@uname\nDarwin\n@@os-release\n").as_deref(), Some("darwin"));
        assert_eq!(parse_os("@@uname\nFreeBSD\n@@os-release\nNAME=FreeBSD\nID=freebsd\n").as_deref(), Some("freebsd/freebsd"));
        assert_eq!(parse_os("@@uname\nLinux\n@@os-release\n").as_deref(), Some("linux"), "没有 os-release（Termux、很老的系统）");
    }

    /// 不是这条命令的输出（Windows 的 cmd 不认 sh、只许跑固定命令的账号）不算；服务器给的怪值不收。
    #[test]
    fn os_ignores_output_that_is_not_ours() {
        assert_eq!(parse_os(""), None);
        assert_eq!(parse_os("'sh' is not recognized as an internal or external command,\r\noperable program or batch file.\r\n"), None);
        assert_eq!(parse_os("This account is restricted to git.\nLinux\n"), None);
        assert_eq!(parse_os("@@uname\n@@os-release\nID=ubuntu\n"), None, "uname 没跑成：说不清是什么系统");
        assert_eq!(parse_os("@@uname\n<b>Linux</b>\n@@os-release\nID=ubuntu\n"), None);
        assert_eq!(parse_os("@@uname\nLinux\n@@os-release\nID=\"ubuntu; rm -rf /\"\n").as_deref(), Some("linux"));
        assert_eq!(parse_os(&format!("@@uname\nLinux\n@@os-release\nID={}\n", "x".repeat(33))).as_deref(), Some("linux"));
    }

    /// 本机真跑一遍：macOS 没有 os-release，只有内核名。
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn os_of_this_mac() {
        assert_eq!(os(&AppState::new(), &Machine::Local).await.unwrap().as_deref(), Some("darwin"));
    }
}

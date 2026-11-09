//! 「系统」页：进程、监听端口、负载 / 内存 / 磁盘，以及结束进程。
//! 命令以文件管理当前身份跑（终端里 sudo -i 之后就是 root：看得到所有端口对应的进程、能结束 root 的进程）。
//! 主要面向 Linux（ss / netstat / /proc），macOS / BSD 退到 lsof / sysctl。

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub user: String,
    pub cpu: f32,
    pub mem: f32,
    /// 常驻内存，KB。
    pub rss_kb: u64,
    pub command: String,
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

/// 全部进程。args 放最后：命令行里有空格。
pub const PROCESSES_COMMAND: &str = "LC_ALL=C ps -eo pid=,user=,pcpu=,pmem=,rss=,args=";

pub fn parse_processes(out: &str) -> Vec<ProcessInfo> {
    out.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let user = it.next()?.to_string();
            let cpu = it.next()?.parse().ok()?;
            let mem = it.next()?.parse().ok()?;
            let rss_kb = it.next()?.parse().ok()?;
            let command = it.collect::<Vec<_>>().join(" ");
            Some(ProcessInfo { pid, user, cpu, mem, rss_kb, command })
        })
        .collect()
}

/// 监听端口：有 ss 用 ss，没有用 netstat（Linux 的 -p），再没有用 lsof（macOS / BSD）。第一行标出用的哪个。
pub const PORTS_COMMAND: &str = "sh -c 'if command -v ss >/dev/null 2>&1; then echo @@ss; ss -lntup 2>/dev/null; \
elif netstat -lntup >/dev/null 2>&1; then echo @@netstat; netstat -lntup 2>/dev/null; \
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

pub fn parse_ports(out: &str) -> Vec<PortInfo> {
    let mut lines = out.lines();
    let tool = lines.next().unwrap_or("").trim();
    let mut ports: Vec<PortInfo> = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let parsed = match tool {
            // Netid State Recv-Q Send-Q Local:Port Peer:Port [Process]
            "@@ss" if fields.len() >= 5 && fields[0] != "Netid" => split_address(fields[4]).map(|(address, port)| {
                let (process, pid) = ss_user(&fields[5..].join(" "));
                PortInfo { proto: proto_of(fields[0]), address, port, pid, process }
            }),
            // Proto Recv-Q Send-Q Local Foreign [State] PID/Program
            "@@netstat" if fields.len() >= 6 && (fields[0].starts_with("tcp") || fields[0].starts_with("udp")) => {
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
            "@@lsof" if fields.len() >= 9 && fields[0] != "COMMAND" && !fields[8].contains("->") => {
                split_address(fields[8]).map(|(address, port)| PortInfo {
                    proto: proto_of(fields[7]),
                    address,
                    port,
                    pid: fields[1].parse().ok(),
                    // lsof 把名字里的空格等写成 \x20
                    process: Some(fields[0].replace("\\x20", " ")),
                })
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
    ports.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.proto.cmp(&b.proto)));
    ports
}

/// 负载 / 内存 / 磁盘 / 运行时间。Linux 读 /proc，macOS / BSD 退到 sysctl；读不到的项留空。
pub const OVERVIEW_COMMAND: &str = "sh -c 'echo @@load; cat /proc/loadavg 2>/dev/null || sysctl -n vm.loadavg 2>/dev/null; \
echo @@cpus; nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null; \
echo @@mem; cat /proc/meminfo 2>/dev/null || echo \"MemTotal: $(sysctl -n hw.memsize 2>/dev/null) B\"; \
echo @@uptime; cat /proc/uptime 2>/dev/null || echo \"boot $(sysctl -n kern.boottime 2>/dev/null | sed -n \"s/^{ sec = \\([0-9]*\\),.*/\\1/p\") now $(date +%s)\"; \
echo @@df; df -kP / 2>/dev/null | tail -n 1'";

pub fn parse_overview(out: &str) -> Overview {
    let mut overview = Overview::default();
    let mut section = "";
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            section = match name.trim() {
                "load" => "load",
                "cpus" => "cpus",
                "mem" => "mem",
                "uptime" => "uptime",
                "df" => "df",
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
                    _ => {}
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
            "df" if fields.len() >= 6 => {
                overview.disk_total = fields[1].parse::<u64>().ok().map(|k| k * 1024);
                overview.disk_used = fields[2].parse::<u64>().ok().map(|k| k * 1024);
            }
            _ => {}
        }
    }
    overview
}

pub fn kill_command(pid: u32, force: bool) -> String {
    format!("kill -{} {pid}", if force { "KILL" } else { "TERM" })
}

async fn run(state: &AppState, session_id: &str, command: &str) -> AppResult<String> {
    let session = state.get(session_id).await.ok_or_else(|| AppError::SessionNotFound(session_id.to_string()))?;
    crate::sftp::run_as_file_user(&session, command).await
}

pub async fn processes(state: &AppState, session_id: String) -> AppResult<Vec<ProcessInfo>> {
    Ok(parse_processes(&run(state, &session_id, PROCESSES_COMMAND).await?))
}

pub async fn ports(state: &AppState, session_id: String) -> AppResult<Vec<PortInfo>> {
    Ok(parse_ports(&run(state, &session_id, PORTS_COMMAND).await?))
}

pub async fn overview(state: &AppState, session_id: String) -> AppResult<Overview> {
    Ok(parse_overview(&run(state, &session_id, OVERVIEW_COMMAND).await?))
}

/// 结束进程（TERM，force 时 KILL）。没权限时报错里带 kill 的原话（Operation not permitted）。
pub async fn kill(state: &AppState, session_id: String, pid: u32, force: bool) -> AppResult<()> {
    run(state, &session_id, &kill_command(pid, force)).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnu_and_bsd_ps() {
        let out = "    1 root      0.0  0.1  11236 /sbin/init splash\n 1234 www-data 12.5  2.3 190000 nginx: worker process\n  363 root               0.0  0.0   2624 /usr/libexec/dprivacyd\nbad line\n";
        let procs = parse_processes(out);
        assert_eq!(procs.len(), 3);
        assert_eq!(procs[1], ProcessInfo { pid: 1234, user: "www-data".into(), cpu: 12.5, mem: 2.3, rss_kb: 190000, command: "nginx: worker process".into() });
        assert_eq!(procs[2].command, "/usr/libexec/dprivacyd");
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

    #[test]
    fn parses_linux_and_macos_overview() {
        let linux = "@@load\n0.52 0.58 0.59 1/389 12345\n@@cpus\n4\n@@mem\nMemTotal:        8000000 kB\nMemFree:          100000 kB\nMemAvailable:    6000000 kB\n@@uptime\n123456.78 400000.00\n@@df\n/dev/vda1 51475068 21000000 28000000 43% /\n";
        let o = parse_overview(linux);
        assert_eq!(o.load, Some([0.52, 0.58, 0.59]));
        assert_eq!(o.cpus, Some(4));
        assert_eq!((o.mem_total, o.mem_available), (Some(8_192_000_000), Some(6_144_000_000)));
        assert_eq!(o.uptime_secs, Some(123456));
        assert_eq!((o.disk_total, o.disk_used), (Some(51475068 * 1024), Some(21000000 * 1024)));
        let mac = "@@load\n{ 4.07 6.45 5.16 }\n@@cpus\n10\n@@mem\nMemTotal: 34359738368 B\n@@uptime\nboot 1794064076 now 1794164076\n@@df\n/dev/disk3s1s1   971350180  13437800 192380972     7%    /\n";
        let o = parse_overview(mac);
        assert_eq!(o.load, Some([4.07, 6.45, 5.16]));
        assert_eq!((o.cpus, o.mem_total, o.mem_available), (Some(10), Some(34359738368), None));
        assert_eq!(o.uptime_secs, Some(100000));
        assert_eq!(o.disk_used, Some(13437800 * 1024));
    }

    /// 真实 sshd（本机 macOS 或 Linux 均可）：三条命令都跑得通、解析得出东西；结束进程真的结束。
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
        let (_, out, _) = exec(PORTS_COMMAND.into()).await;
        assert!(out.starts_with("@@"), "{out}");
        let ports = parse_ports(&out);
        assert!(ports.iter().any(|p| p.port == port.parse::<u16>().unwrap()), "看得到 sshd 自己监听的端口：{ports:?}");
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
    }

    #[test]
    fn kill_is_term_unless_forced() {
        assert_eq!(kill_command(42, false), "kill -TERM 42");
        assert_eq!(kill_command(42, true), "kill -KILL 42");
    }
}

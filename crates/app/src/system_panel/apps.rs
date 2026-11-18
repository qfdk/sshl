//! 本机的进程按应用合起来（同 Mole、活动监视器）：同一个 .app 里的进程（Chrome 的几十个 Helper）合成一行，
//! CPU、内存加起来，名字是应用的显示名（微信、日历）。点开看里面每个进程。不在 .app 里的系统进程、命令行程序照旧一行一个。
//! 自己不在 .app 里、但是由某个应用启动的进程（Docker 的虚拟机 com.apple.Virtualization.VirtualMachine、应用内嵌网页的
//! WebContent）按 macOS 的「负责进程」归到那个应用（同活动监视器、Mole：Docker 的内存把虚拟机算进去）。
//! 远程服务器不合（Linux 上没有应用包）。

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use sshl_core::server_info::ProcessInfo;

use super::{SortKey, process_name};
use crate::app_icons;

/// 一个应用（本机）或者一棵同名的进程树（远程）的所有进程。
#[derive(Debug)]
pub struct AppGroup<'a> {
    /// 展开记在哪个键上：本机是应用路径（`/Applications/Google Chrome.app`，也是取图标的键），远程是 `pid:<主进程>`。
    pub key: String,
    /// 本机的应用（用应用图标）；远程的进程树是 false（按进程名认标志，见 `brands`）。
    pub app: bool,
    pub name: String,
    /// 主进程。本机：程序直接在 `Contents/MacOS` 下的（不是 Frameworks 里的 Helper），没有就 PID 最小的；远程：树最上面那个。
    /// PID 列显示它，结束也是结束它（退出应用 / 停掉 nginx 的 master，worker 跟着退）。
    pub main: &'a ProcessInfo,
    /// 全部进程，按当前的排序排好。
    pub members: Vec<&'a ProcessInfo>,
    pub cpu: f32,
    pub rss_kb: u64,
}

/// 列表的一行。
#[derive(Debug)]
pub enum Listed<'a> {
    /// 不在应用里的进程。
    Process(&'a ProcessInfo),
    /// 一个应用（展开了没有）。
    App(AppGroup<'a>, bool),
    /// 展开的应用里的一个进程。
    Member(&'a ProcessInfo),
}

/// 每个本机进程的负责进程（PID → PID，只记和自己不一样的）。用的是系统私有的 `responsibility_get_pid_responsible_for_pid`
/// （活动监视器用的同一个），运行时找：找不到就是空表，退回只按程序路径合。每次刷新几百个进程，一次几微秒，放在后台线程里调。
/// 只记 launchd（PID 1）替应用起的进程（XPC 服务：虚拟机、WebContent）：在终端里敲出来的程序负责进程也是终端应用，
/// 可它们是用户自己跑的（claude、node），算进 SSHL 的话 SSHL 看着占了 1 GB，其实它自己只有一百多 MB。
pub fn responsible_pids(pids: impl Iterator<Item = u32>) -> HashMap<u32, u32> {
    type Lookup = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;
    static LOOKUP: OnceLock<Option<Lookup>> = OnceLock::new();
    let lookup = LOOKUP.get_or_init(|| {
        // SAFETY: dlsym 查当前进程里已经载入的符号；找到的就是这个签名的 C 函数（int 进、int 出）
        unsafe {
            let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"responsibility_get_pid_responsible_for_pid".as_ptr());
            (!symbol.is_null()).then(|| std::mem::transmute::<*mut libc::c_void, Lookup>(symbol))
        }
    });
    let Some(lookup) = lookup else { return HashMap::new() };
    pids.filter(|&pid| parent_of(pid) == Some(1))
        .filter_map(|pid| {
            // SAFETY: 只读查询，进程不在了返回 -1
            let responsible = unsafe { lookup(pid as libc::pid_t) };
            (responsible > 0 && responsible as u32 != pid).then_some((pid, responsible as u32))
        })
        .collect()
}

/// 父进程的 PID（进程不在了、看不到的 None）。
fn parent_of(pid: u32) -> Option<u32> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: 缓冲区就是 proc_bsdinfo 那么大，内核最多写这么多
    let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (n == size).then_some(info.pbi_ppid)
}

/// 应用路径 → 显示名还没取到时用的名字：去掉 `.app` 的文件名。
pub fn fallback_name(bundle: &str) -> &str {
    let file = bundle.rsplit('/').next().unwrap_or(bundle);
    file.strip_suffix(".app").unwrap_or(file)
}

/// 主进程：程序就在 `<应用>/Contents/MacOS/` 下面。
fn is_main(bundle: &str, process: &ProcessInfo) -> bool {
    let program = process.program.as_deref().unwrap_or_else(|| process.command.split(' ').next().unwrap_or(""));
    program.strip_prefix(bundle).and_then(|rest| rest.strip_prefix("/Contents/MacOS/")).is_some_and(|name| !name.contains('/'))
}

/// 搜索：PID、用户、命令行里含关键字（不分大小写，`query` 已经是小写）。
fn matches(process: &ProcessInfo, query: &str) -> bool {
    query.is_empty() || process.pid.to_string().contains(query) || process.user.to_lowercase().contains(query) || process.command.to_lowercase().contains(query)
}

/// 两个进程按当前的排序比（`descending` 时反过来）。
pub fn compare(a: &ProcessInfo, b: &ProcessInfo, sort: (SortKey, bool)) -> Ordering {
    let order = match sort.0 {
        SortKey::Pid => a.pid.cmp(&b.pid),
        SortKey::User => a.user.cmp(&b.user),
        SortKey::Cpu => a.cpu.total_cmp(&b.cpu).then(a.rss_kb.cmp(&b.rss_kb)),
        SortKey::Memory => a.rss_kb.cmp(&b.rss_kb),
        SortKey::Name => process_name(&a.command, a.program.as_deref())
            .to_lowercase()
            .cmp(&process_name(&b.command, b.program.as_deref()).to_lowercase())
            .then_with(|| a.command.cmp(&b.command)),
    };
    if sort.1 { order.reverse() } else { order }
}

/// 本机的列表：按应用合起来、搜索、排序，展开的应用后面跟着它的进程。`names` 是取到了的应用显示名（应用路径 → 名字），
/// 没取到的用文件名。应用的名字也能搜。`responsible` 是 [`responsible_pids`]：自己不在应用里的进程归到负责它的那个应用。
pub fn list<'a>(
    processes: &'a [ProcessInfo],
    query: &str,
    sort: (SortKey, bool),
    expanded: &HashSet<String>,
    names: &HashMap<String, String>,
    responsible: &HashMap<u32, u32>,
) -> Vec<Listed<'a>> {
    let own: HashMap<u32, String> = processes.iter().filter_map(|p| Some((p.pid, app_icons::icon_key(p.program.as_deref(), &p.command)?))).collect();
    let mut singles = Vec::new();
    let mut apps: HashMap<String, Vec<&ProcessInfo>> = HashMap::new();
    for process in processes {
        let app = own.get(&process.pid).or_else(|| responsible.get(&process.pid).and_then(|owner| own.get(owner)));
        match app {
            Some(bundle) => apps.entry(bundle.clone()).or_default().push(process),
            None => singles.push(process),
        }
    }
    let groups = apps
        .into_iter()
        .filter_map(|(bundle, members)| {
            let name = names.get(&bundle).cloned().unwrap_or_else(|| fallback_name(&bundle).to_string());
            let main = members.iter().copied().min_by_key(|p| (!is_main(&bundle, p), p.pid))?;
            Some(Draft { key: bundle, name, main, members, app: true })
        })
        .collect();
    assemble(singles, groups, query, sort, expanded)
}

/// 远程服务器（Linux 没有 .app）的列表：按进程树合，一个进程和它下面同名的子孙合成一行——nginx 的 master 和 worker、
/// postgres 和它的后台进程、php-fpm 和进程池、sshd 和登录会话。主进程是最上面那个同名的（PID 列、「结束」都是它）。
/// 只有一个进程的照旧单独一行。展开用的键是 `pid:<主进程>`。
pub fn tree<'a>(processes: &'a [ProcessInfo], query: &str, sort: (SortKey, bool), expanded: &HashSet<String>) -> Vec<Listed<'a>> {
    let by_pid: HashMap<u32, &ProcessInfo> = processes.iter().map(|p| (p.pid, p)).collect();
    let name = |p: &ProcessInfo| process_name(&p.command, p.program.as_deref());
    let mut trees: HashMap<u32, Vec<&ProcessInfo>> = HashMap::new();
    for process in processes {
        // 往上找：父进程同名就并进去（层数有上限，ps 读到一半进程换了父亲也不会转圈）
        let own = name(process);
        let mut top = process;
        for _ in 0..processes.len() {
            match by_pid.get(&top.ppid) {
                Some(parent) if parent.pid != top.pid && name(parent) == own => top = parent,
                _ => break,
            }
        }
        trees.entry(top.pid).or_default().push(process);
    }
    let mut singles = Vec::new();
    let mut groups = Vec::new();
    for (top, members) in trees {
        match (members.len(), by_pid.get(&top)) {
            (2.., Some(main)) => groups.push(Draft { key: format!("pid:{top}"), name: name(main), main, members, app: false }),
            _ => singles.extend(members),
        }
    }
    assemble(singles, groups, query, sort, expanded)
}

/// 合起来之前的一组。
struct Draft<'a> {
    key: String,
    name: String,
    main: &'a ProcessInfo,
    members: Vec<&'a ProcessInfo>,
    app: bool,
}

/// 单个的进程和合起来的组 → 列表：搜索（组的名字或者里面任何一个进程对得上就留下整组）、排序（组按合计比），展开的组后面跟着它的进程。
fn assemble<'a>(singles: Vec<&'a ProcessInfo>, groups: Vec<Draft<'a>>, query: &str, sort: (SortKey, bool), expanded: &HashSet<String>) -> Vec<Listed<'a>> {
    let query = query.trim().to_lowercase();
    // (排序用的名字, 这一行)
    let mut rows: Vec<(String, Listed)> =
        singles.into_iter().filter(|p| matches(p, &query)).map(|p| (process_name(&p.command, p.program.as_deref()).to_lowercase(), Listed::Process(p))).collect();
    for Draft { key, name, main, mut members, app } in groups {
        if !(name.to_lowercase().contains(&query) || members.iter().any(|p| matches(p, &query))) {
            continue;
        }
        members.sort_by(|a, b| compare(a, b, sort));
        let open = expanded.contains(&key) && members.len() > 1;
        let group = AppGroup { cpu: members.iter().map(|p| p.cpu).sum(), rss_kb: members.iter().map(|p| p.rss_kb).sum(), key, name: name.clone(), main, members, app };
        rows.push((name.to_lowercase(), Listed::App(group, open)));
    }
    let key = |row: &Listed| -> (u32, String, f32, u64) {
        match row {
            Listed::App(group, _) => (group.main.pid, group.main.user.clone(), group.cpu, group.rss_kb),
            Listed::Process(p) | Listed::Member(p) => (p.pid, p.user.clone(), p.cpu, p.rss_kb),
        }
    };
    rows.sort_by(|(a_name, a), (b_name, b)| {
        let (a, b) = (key(a), key(b));
        let order = match sort.0 {
            SortKey::Pid => a.0.cmp(&b.0),
            SortKey::User => a.1.cmp(&b.1),
            SortKey::Cpu => a.2.total_cmp(&b.2).then(a.3.cmp(&b.3)),
            SortKey::Memory => a.3.cmp(&b.3),
            SortKey::Name => a_name.cmp(b_name),
        };
        let order = order.then_with(|| a_name.cmp(b_name)).then(a.0.cmp(&b.0));
        if sort.1 { order.reverse() } else { order }
    });
    let mut out = Vec::with_capacity(rows.len());
    for (_, row) in rows {
        let members = match &row {
            Listed::App(group, true) => group.members.clone(),
            _ => Vec::new(),
        };
        out.push(row);
        out.extend(members.into_iter().map(Listed::Member));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, user: &str, cpu: f32, rss_kb: u64, program: &str, args: &str) -> ProcessInfo {
        let command = if args.is_empty() { program.to_string() } else { format!("{program} {args}") };
        let program = program.starts_with('/').then(|| program.to_string());
        ProcessInfo { pid, ppid: 1, user: user.into(), cpu, mem: 0., rss_kb, command, program }
    }

    const CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    const HELPER: &str = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)";
    const WECHAT: &str = "/Applications/WeChat.app/Contents/MacOS/WeChat";

    fn sample() -> Vec<ProcessInfo> {
        vec![
            p(900, "me", 30.0, 300_000, HELPER, "--type=renderer"),
            p(400, "me", 2.0, 500_000, CHROME, ""),
            p(901, "me", 10.0, 200_000, HELPER, "--type=renderer"),
            p(425, "_windowserver", 25.0, 270_000, "/System/Library/PrivateFrameworks/SkyLight.framework/Resources/WindowServer", "-daemon"),
            p(700, "me", 1.0, 1_100_000, WECHAT, ""),
            p(13, "me", 12.0, 380_000, "claude", ""),
        ]
    }

    fn names(rows: &[Listed]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Listed::Process(p) => process_name(&p.command, p.program.as_deref()),
                Listed::App(group, open) => format!("{}{}×{}", group.name, if *open { "▾" } else { "" }, group.members.len()),
                Listed::Member(p) => format!("  {}", p.pid),
            })
            .collect()
    }

    /// Chrome 的主进程和 Helper 合成一行：CPU、内存加起来，PID 是主进程的；按合计排。
    #[test]
    fn processes_of_one_app_become_one_row() {
        let processes = sample();
        let rows = list(&processes, "", (SortKey::Cpu, true), &HashSet::new(), &HashMap::new(), &HashMap::new());
        assert_eq!(names(&rows), ["Google Chrome×3", "WindowServer", "claude", "WeChat×1"]);
        let Listed::App(chrome, false) = &rows[0] else { panic!() };
        assert_eq!((chrome.main.pid, chrome.rss_kb), (400, 1_000_000), "主进程是直接在 Contents/MacOS 下的，不是 PID 最小的 Helper");
        assert!((chrome.cpu - 42.0).abs() < 0.01);
        assert_eq!(chrome.members.iter().map(|p| p.pid).collect::<Vec<_>>(), [900, 901, 400], "里面也按 CPU 排");

        let rows = list(&processes, "", (SortKey::Memory, true), &HashSet::new(), &HashMap::new(), &HashMap::new());
        assert_eq!(names(&rows), ["WeChat×1", "Google Chrome×3", "claude", "WindowServer"]);
    }

    /// 展开的应用后面跟着它的每个进程；只有一个进程的应用不展开。显示名取到了就用（也能搜）。
    #[test]
    fn an_expanded_app_lists_its_processes() {
        let processes = sample();
        let expanded: HashSet<String> = ["/Applications/Google Chrome.app".to_string(), "/Applications/WeChat.app".to_string()].into();
        let names_of: HashMap<String, String> = [("/Applications/WeChat.app".to_string(), "微信".to_string())].into();
        let rows = list(&processes, "", (SortKey::Memory, true), &expanded, &names_of, &HashMap::new());
        assert_eq!(names(&rows), ["微信×1", "Google Chrome▾×3", "  400", "  900", "  901", "claude", "WindowServer"]);
        assert_eq!(names(&list(&processes, "微信", (SortKey::Cpu, true), &expanded, &names_of, &HashMap::new())), ["微信×1"]);
    }

    /// 搜索：应用里有一个进程对得上，整个应用都在；不在应用里的照旧按 PID / 用户 / 命令行搜。
    #[test]
    fn search_finds_apps_by_any_of_their_processes() {
        let processes = sample();
        assert_eq!(names(&list(&processes, "renderer", (SortKey::Cpu, true), &HashSet::new(), &HashMap::new(), &HashMap::new())), ["Google Chrome×3"]);
        assert_eq!(names(&list(&processes, "425", (SortKey::Cpu, true), &HashSet::new(), &HashMap::new(), &HashMap::new())), ["WindowServer"]);
        assert_eq!(names(&list(&processes, "chrome", (SortKey::Name, false), &HashSet::new(), &HashMap::new(), &HashMap::new())), ["Google Chrome×3"]);
    }

    /// 自己不在应用里、由应用启动的进程（Docker 的虚拟机）算进那个应用；负责进程是自己的、或者不在应用里的照旧单独一行。
    #[test]
    fn helpers_started_by_an_app_count_towards_it() {
        let docker = "/Applications/Docker.app/Contents/MacOS/com.docker.backend";
        let vm = "/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine";
        let processes = vec![p(21462, "me", 0.2, 600_000, docker, "services"), p(21503, "me", 1.4, 3_200_000, vm, ""), p(504, "_mds_stores", 0.1, 750_000, "/usr/libexec/mds_stores", "")];
        let responsible: HashMap<u32, u32> = [(21503, 21462), (504, 1)].into();
        let rows = list(&processes, "", (SortKey::Memory, true), &HashSet::new(), &HashMap::new(), &responsible);
        assert_eq!(names(&rows), ["Docker×2", "mds_stores"]);
        let Listed::App(group, _) = &rows[0] else { panic!() };
        assert_eq!((group.main.pid, group.rss_kb), (21462, 3_800_000), "虚拟机的内存算进 Docker，PID 还是 Docker 的");
    }

    /// 真问系统：父进程查得到；测试进程是终端（或 cargo）起的、不是 launchd 起的，不记（用户自己跑的程序不算进终端应用）。
    #[test]
    fn only_services_launched_for_an_app_are_attributed_to_it() {
        let me = std::process::id();
        assert_eq!(parent_of(me), Some(std::os::unix::process::parent_id()));
        let table = responsible_pids([me, 1].into_iter());
        assert!(!table.contains_key(&me) && !table.contains_key(&1), "{table:?}");
    }

    /// 远程按进程树合：nginx 的 master 和 worker、postgres 和它的后台进程、sshd 和登录会话各一行；不同名的子进程（sshd 下面的 bash）单独一行。
    #[test]
    fn remote_processes_group_by_parent() {
        let q = |pid: u32, ppid: u32, user: &str, cpu: f32, rss_kb: u64, command: &str| ProcessInfo {
            pid,
            ppid,
            user: user.into(),
            cpu,
            mem: 0.,
            rss_kb,
            command: command.into(),
            program: None,
        };
        let processes = vec![
            q(1, 0, "root", 0., 10_000, "/sbin/init"),
            q(20, 1, "root", 0., 2_000, "nginx: master process /usr/sbin/nginx"),
            q(21, 20, "www-data", 3., 3_000, "nginx: worker process"),
            q(22, 20, "www-data", 1., 3_000, "nginx: worker process"),
            q(59, 1, "postgres", 0., 30_000, "/usr/lib/postgresql/16/bin/postgres -D /var/lib/postgresql/16/main"),
            q(60, 59, "postgres", 0.5, 7_000, "postgres: 16/main: checkpointer"),
            q(100, 1, "root", 0., 7_000, "sshd: /usr/sbin/sshd -D [listener] 0 of 10-100 startups"),
            q(200, 100, "root", 0., 7_000, "sshd: deploy [priv]"),
            q(201, 200, "deploy", 0., 6_000, "sshd: deploy@pts/0"),
            q(202, 201, "deploy", 0., 4_000, "-bash"),
            q(38, 1, "redis", 0.3, 12_000, "/usr/bin/redis-server 127.0.0.1:6379"),
        ];
        let rows = tree(&processes, "", (SortKey::Memory, true), &HashSet::new());
        assert_eq!(names(&rows), ["postgres×2", "sshd×3", "redis-server", "init", "nginx×3", "-bash"]);
        let Listed::App(nginx, _) = &rows[4] else { panic!() };
        assert_eq!((nginx.main.pid, nginx.rss_kb, nginx.key.as_str(), nginx.app), (20, 8_000, "pid:20", false), "主进程是 master");
        assert!((nginx.cpu - 4.).abs() < 0.01);

        let expanded: HashSet<String> = ["pid:20".to_string()].into();
        let rows = tree(&processes, "worker", (SortKey::Pid, false), &expanded);
        assert_eq!(names(&rows), ["nginx▾×3", "  20", "  21", "  22"], "搜到 worker 留下整组，展开的跟着每个进程");
    }

    /// 主进程不在了（只剩 Helper）就用 PID 最小的。
    #[test]
    fn without_its_main_process_an_app_uses_the_oldest_one() {
        let processes = vec![p(901, "me", 1., 1, HELPER, ""), p(900, "me", 1., 1, HELPER, "")];
        let rows = list(&processes, "", (SortKey::Cpu, true), &HashSet::new(), &HashMap::new(), &HashMap::new());
        let Listed::App(group, _) = &rows[0] else { panic!() };
        assert_eq!(group.main.pid, 900);
        assert_eq!(fallback_name("/Applications/Google Chrome.app"), "Google Chrome");
    }
}

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

/// 一个应用的所有进程。
#[derive(Debug)]
pub struct AppGroup<'a> {
    /// 应用路径（`/Applications/Google Chrome.app`）。
    pub bundle: String,
    pub name: String,
    /// 主进程：程序直接在 `Contents/MacOS` 下的（不是 Frameworks 里的 Helper）；没有就 PID 最小的。PID 列显示它，结束也是结束它（退出应用）。
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
    let query = query.trim().to_lowercase();
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
    // (排序用的名字, 这一行)
    let mut rows: Vec<(String, Listed)> =
        singles.into_iter().filter(|p| matches(p, &query)).map(|p| (process_name(&p.command, p.program.as_deref()).to_lowercase(), Listed::Process(p))).collect();
    for (bundle, mut members) in apps {
        let name = names.get(&bundle).cloned().unwrap_or_else(|| fallback_name(&bundle).to_string());
        if !(name.to_lowercase().contains(&query) || members.iter().any(|p| matches(p, &query))) {
            continue;
        }
        members.sort_by(|a, b| compare(a, b, sort));
        let main = members.iter().filter(|p| is_main(&bundle, p)).chain(members.iter()).min_by_key(|p| (!is_main(&bundle, p), p.pid)).copied();
        let Some(main) = main else { continue };
        let group = AppGroup {
            cpu: members.iter().map(|p| p.cpu).sum(),
            rss_kb: members.iter().map(|p| p.rss_kb).sum(),
            bundle: bundle.clone(),
            name: name.clone(),
            main,
            members,
        };
        let open = expanded.contains(&bundle) && group.members.len() > 1;
        rows.push((name.to_lowercase(), Listed::App(group, open)));
    }
    // 应用按合计比，单个进程按它自己
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
        ProcessInfo { pid, user: user.into(), cpu, mem: 0., rss_kb, command, program }
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

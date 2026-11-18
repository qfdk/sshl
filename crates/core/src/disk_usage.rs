//! 「系统」页的磁盘：各分区用了多少（`df`），点一个分区整个扫一遍（`du`），之后像 ncdu 一样按大小一层层往下点。
//! 命令在那台机器上以文件管理当前身份跑（终端 sudo -i 之后就是 root，整台机器都看得到）。

use std::collections::{HashMap, HashSet};

pub use crate::disk_scan::ScanProgress;
use crate::error::{AppError, AppResult};
use crate::server_info::{self, Machine};
use crate::state::AppState;

/// 一个分区。大小都是字节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filesystem {
    pub device: String,
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

/// `df -kP`（POSIX 格式，Linux / macOS / BusyBox 都认）；后面接 `mount`，看哪些卷带 `nobrowse`（macOS：Finder 不显示的卷）。
/// 出错的分区（失效的网络盘）df 照样列别的、退出码非 0，不当失败。
const FILESYSTEMS_COMMAND: &str = "LC_ALL=C df -kP 2>/dev/null; echo @@mount; LC_ALL=C mount 2>/dev/null; true";

pub async fn filesystems(state: &AppState, machine: &Machine) -> AppResult<Vec<Filesystem>> {
    Ok(parse_filesystems(&server_info::run(state, machine, FILESYSTEMS_COMMAND).await?))
}

/// macOS 放东西的数据卷（`/` 是只读的系统卷）。
const MAC_DATA: &str = "/System/Volumes/Data";

/// 只留能清理空间的盘：块设备（`/dev/…`）、网络盘（`host:/path`）和根目录（容器里 / 是 overlay）。不要的：tmpfs、/proc 这类、
/// snap 的 loop 设备、容器里从宿主 bind mount 进来的 /etc/hosts 这类单个文件；macOS 上 Finder 不显示的卷（`nobrowse`：系统卷、
/// iOS 模拟器镜像、cryptex、Xcode 的 DeviceFS），以及只读的系统卷 /（东西都在数据卷上）。
/// 同一个设备挂了好几处（bind mount）只留路径最短的。
pub fn parse_filesystems(out: &str) -> Vec<Filesystem> {
    let (df, mounts) = out.split_once("@@mount").unwrap_or((out, ""));
    let hidden = nobrowse_mounts(mounts);
    let mac = df.lines().any(|line| line.trim_end().ends_with(MAC_DATA));
    let mut found: Vec<Filesystem> = Vec::new();
    for line in df.lines().skip(1) {
        let Some(fs) = parse_df_line(line) else { continue };
        // 网络盘是 host:/path；`devices -- file:///…`（Xcode 的 DeviceFS）这种带 :// 的不是
        let device_ok = fs.device.starts_with('/') || (fs.device.contains(":/") && !fs.device.contains("://"));
        let skipped_mount = ["/proc", "/sys", "/dev", "/run", "/snap", "/etc", "/System/Volumes"]
            .iter()
            .any(|prefix| fs.mount == *prefix || fs.mount.starts_with(&format!("{prefix}/")))
            && fs.mount != MAC_DATA;
        let mac_hidden = fs.mount != MAC_DATA && (hidden.contains(&fs.mount) || (mac && fs.mount == "/"));
        if fs.total == 0 || !(device_ok || fs.mount == "/") || fs.device.starts_with("/dev/loop") || skipped_mount || mac_hidden {
            continue;
        }
        match found.iter_mut().find(|other| other.device == fs.device) {
            Some(other) if fs.mount.len() < other.mount.len() => *other = fs,
            Some(_) => {}
            None => found.push(fs),
        }
    }
    found.sort_by(|a, b| a.mount.cmp(&b.mount));
    found
}

/// `mount` 的输出里带 `nobrowse` 的挂载点。macOS 是「设备 on 挂载点 (选项)」，Linux 是「设备 on 挂载点 type 类型 (选项)」（不会有 nobrowse）。
fn nobrowse_mounts(out: &str) -> HashSet<String> {
    out.lines()
        .filter_map(|line| {
            let (rest, options) = line.rsplit_once(" (")?;
            if !options.trim_end_matches(')').split(',').any(|option| option.trim() == "nobrowse") {
                return None;
            }
            let (_, mount) = rest.split_once(" on ")?;
            Some(mount.split(" type ").next().unwrap_or(mount).to_string())
        })
        .collect()
}

/// 设备名、挂载点里都可能有空格（macOS 的 `map auto_home`）：找到「xx%」那一列，前三个数是总量 / 已用 / 可用。
fn parse_df_line(line: &str) -> Option<Filesystem> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let capacity = fields.iter().position(|field| field.ends_with('%') && field[..field.len() - 1].parse::<u64>().is_ok())?;
    if capacity < 4 || capacity + 1 >= fields.len() {
        return None;
    }
    let kb = |index: usize| fields[index].parse::<u64>().ok().map(|kb| kb * 1024);
    Some(Filesystem {
        device: fields[..capacity - 3].join(" "),
        mount: fields[capacity + 1..].join(" "),
        total: kb(capacity - 3)?,
        used: kb(capacity - 2)?,
        available: kb(capacity - 1)?,
    })
}

/// 一次列多深：扫的目录下面这么多层。du 照样把整棵树走一遍（总数是准的），但只输出这几层，几万行以内，
/// 传回来、建树都快，大分区也不会让界面卡；点到更深的目录时再补扫那一个目录（子树小，很快）。
pub const SCAN_DEPTH: usize = 4;

/// 扫一个分区：每个目录的占用（KB，`-x` 不跨到别的分区），只列 [`SCAN_DEPTH`] 层。放在最低的 IO / CPU 优先级上跑，
/// 尽量不压着服务器上的正事：Linux 用 `ionice -c3`，macOS 用 `taskpolicy -b`（后台 QoS，磁盘 IO 被系统节流），都再加 `nice`。
/// 限制层数的写法各家不同：GNU 老版本只认 `--max-depth`，BusyBox、macOS 只认 `-d`——拿 /dev/null 试一下（只看一个文件，不走树）。
pub fn scan_command(root: &str) -> String {
    let root = shell_quote(root);
    format!(
        "cd / && export LC_ALL=C; if du --max-depth=0 /dev/null >/dev/null 2>&1; then d=--max-depth={SCAN_DEPTH}; else d='-d {SCAN_DEPTH}'; fi; \
         if command -v ionice >/dev/null 2>&1; then q='ionice -c3 -t'; elif command -v taskpolicy >/dev/null 2>&1; then q='taskpolicy -b'; else q=; fi; \
         $q nice -n 19 du -xk $d -- {root}"
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// 扫出来的目录树（只有目录，文件算在「这一层的文件」里）。
#[derive(Debug, Clone, Default)]
pub struct DiskTree {
    root: String,
    /// 目录 → 占用（字节，含下面所有东西）。
    sizes: HashMap<String, u64>,
    /// 目录 → 下一层的子目录，从大到小排好（建树时排一次，画的时候不再排）。
    children: HashMap<String, Vec<String>>,
    /// 在列出来的最深那一层上的目录：下面还没列，点进去要补扫。
    frontier: HashSet<String>,
    /// 没权限读（或读失败）的目录个数：有的话总数偏小。
    pub unreadable: usize,
}

/// 一层里的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// 下面还有子目录（或者还没扫到那么深）：能往下点。
    pub has_children: bool,
}

impl DiskTree {
    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn size(&self, path: &str) -> Option<u64> {
        self.sizes.get(path).copied()
    }

    /// 这一层的子目录，从大到小（一样大按名字）。
    pub fn entries(&self, path: &str) -> Vec<DiskEntry> {
        self.children
            .get(path)
            .into_iter()
            .flatten()
            .map(|child| DiskEntry {
                name: child.rsplit('/').next().unwrap_or(child).to_string(),
                path: child.clone(),
                size: self.sizes.get(child).copied().unwrap_or(0),
                has_children: self.frontier.contains(child) || self.children.get(child).is_some_and(|c| !c.is_empty()),
            })
            .collect()
    }

    /// 直接放在这一层的文件一共多大：这一层的占用减去各子目录。还没往下扫的目录不知道，算 0。
    pub fn files_size(&self, path: &str) -> u64 {
        if self.frontier.contains(path) {
            return 0;
        }
        let children: u64 = self.children.get(path).into_iter().flatten().filter_map(|child| self.sizes.get(child)).sum();
        self.size(path).unwrap_or(0).saturating_sub(children)
    }

    /// 这个目录下面还没列出来，要补扫。
    pub fn needs_scan(&self, path: &str) -> bool {
        self.frontier.contains(path)
    }

    /// 本机并行扫描用（`disk_scan`）：第 `depth` 层以内每个目录直接记在它名下的大小（它自己的块 + 直接放在里面的文件，
    /// 第 `depth` 层的还包括下面的一切）→ 整棵树：从最深的一层往上加，子目录排好序，第 `depth` 层的记成边上的。
    pub(crate) fn from_own_sizes(root: String, own: HashMap<String, u64>, depth: usize, unreadable: usize) -> DiskTree {
        let below = if root == "/" { "/".to_string() } else { format!("{root}/") };
        let level = |path: &str| path[root.len()..].split('/').filter(|part| !part.is_empty()).count();
        let mut nodes: Vec<(usize, String)> =
            own.keys().filter(|path| **path == root || path.starts_with(&below)).map(|path| (level(path), path.clone())).collect();
        nodes.sort_by_key(|(level, _)| std::cmp::Reverse(*level));
        let mut tree = DiskTree { root: root.clone(), sizes: own, unreadable, ..DiskTree::default() };
        for (node_level, node) in nodes {
            if node == root {
                continue;
            }
            if node_level == depth {
                tree.frontier.insert(node.clone());
            }
            let Some(parent) = parent_of(&node) else { continue };
            let size = tree.sizes.get(&node).copied().unwrap_or(0);
            *tree.sizes.entry(parent.clone()).or_default() += size;
            tree.children.entry(parent).or_default().push(node);
        }
        for siblings in tree.children.values_mut() {
            sort_by_size(siblings, &tree.sizes);
        }
        tree
    }

    /// 补扫了 `sub.root`（原来在边上的目录）：接到树上。补扫时它的大小可能变了，用新的，它在上一层的位置跟着重排。
    pub fn merge(&mut self, sub: DiskTree) {
        self.frontier.remove(&sub.root);
        let parent = parent_of(&sub.root);
        self.sizes.extend(sub.sizes);
        self.children.extend(sub.children);
        self.frontier.extend(sub.frontier);
        self.unreadable += sub.unreadable;
        if let Some(siblings) = parent.and_then(|parent| self.children.get_mut(&parent)) {
            sort_by_size(siblings, &self.sizes);
        }
    }
}

fn parent_of(path: &str) -> Option<String> {
    match path.rsplit_once('/') {
        Some(("", "")) | None => None,
        Some(("", _)) => Some("/".to_string()),
        Some((parent, _)) => Some(parent.to_string()),
    }
}

fn sort_by_size(paths: &mut [String], sizes: &HashMap<String, u64>) {
    let size = |path: &String| sizes.get(path).copied().unwrap_or(0);
    paths.sort_by(|a, b| size(b).cmp(&size(a)).then_with(|| a.cmp(b)));
}

/// `du -k` 的输出（每行「KB<Tab>路径」，子目录在父目录前面）建成树。不在 `root` 下的、比 `depth` 深的、读不懂的行不要；
/// 正好在第 `depth` 层的目录记成「边上的」。
pub fn parse_du(out: &str, root: &str, unreadable: usize, depth: usize) -> DiskTree {
    let root = normalize(root);
    let below = if root == "/" { "/".to_string() } else { format!("{root}/") };
    let mut tree = DiskTree { root: root.clone(), unreadable, ..DiskTree::default() };
    for line in out.lines() {
        let Some((kb, path)) = line.split_once('\t') else { continue };
        let Ok(kb) = kb.trim().parse::<u64>() else { continue };
        let path = normalize(path);
        if path != root && !path.starts_with(&below) {
            continue;
        }
        let level = path[root.len()..].split('/').filter(|part| !part.is_empty()).count();
        if level > depth {
            continue;
        }
        if level == depth && path != root {
            tree.frontier.insert(path.clone());
        }
        if let Some(parent) = parent_of(&path).filter(|_| path != root) {
            tree.children.entry(parent).or_default().push(path.clone());
        }
        tree.sizes.insert(path, kb * 1024);
    }
    for siblings in tree.children.values_mut() {
        sort_by_size(siblings, &tree.sizes);
    }
    tree
}

pub(crate) fn normalize(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() }
}

/// 一个目录里直接放着的文件（不含子目录）：(名字, 大小)，从大到小。磁盘页每一层把它们和子目录放在一起排，看得出是哪个文件占的。
/// 远程走文件管理的 SFTP（当前身份，大小是文件长度）；本机按实际占的块算（同扫描）。
pub async fn files_in(state: &AppState, machine: &Machine, path: &str) -> AppResult<Vec<(String, u64)>> {
    let mut files: Vec<(String, u64)> = match machine {
        Machine::Remote(session_id) => crate::sftp::file_list(state, session_id.clone(), path.to_string())
            .await?
            .into_iter()
            .filter(|entry| !entry.is_directory)
            .map(|entry| (entry.name, entry.size))
            .collect(),
        Machine::Local => {
            let path = path.to_string();
            tokio::task::spawn_blocking(move || local_files(&path)).await.map_err(|e| AppError::Other(format!("list: {e}")))??
        }
    };
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(files)
}

fn local_files(path: &str) -> AppResult<Vec<(String, u64)>> {
    use std::os::unix::fs::MetadataExt;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path)?.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() {
            files.push((entry.file_name().to_string_lossy().into_owned(), meta.blocks() * 512));
        }
    }
    Ok(files)
}

/// 扫 `root`。本机多线程并行扫（`disk_scan`，`progress` 跟着走）；远程用 du：没权限读的目录 du 会报错（退出码 1）
/// 但照样给出读得到的部分，有输出就算成功，记下读不了几个。
pub async fn scan(state: &AppState, machine: &Machine, root: &str, progress: std::sync::Arc<ScanProgress>) -> AppResult<DiskTree> {
    if *machine == Machine::Local {
        let root = root.to_string();
        return tokio::task::spawn_blocking(move || crate::disk_scan::scan_parallel(&root, SCAN_DEPTH, &progress))
            .await
            .map_err(|e| AppError::Other(format!("scan: {e}")))?;
    }
    let (status, out, err) = server_info::exec(state, machine, &scan_command(root)).await?;
    if out.trim().is_empty() {
        let err = err.trim();
        return Err(AppError::Other(if err.is_empty() { format!("du exited {status}") } else { err.to_string() }));
    }
    let unreadable = err.lines().filter(|line| !line.trim().is_empty()).count();
    Ok(parse_du(&out, root, unreadable, SCAN_DEPTH))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX: &str = "\
Filesystem     1024-blocks      Used Available Capacity Mounted on
/dev/vda1         51475068  21000000  28045376      43% /
tmpfs              1015048         0   1015048       0% /dev/shm
tmpfs               203012      1100    201912       1% /run
/dev/vda15          106858      6186    100673       6% /boot/efi
/dev/loop0           65536     65536         0     100% /snap/core20/2105
/dev/vdb1        309506048  96000000 197762304      33% /data
/dev/vda1         51475068  21000000  28045376      43% /var/lib/docker/overlay2/abc/merged
nas:/export/backup 1000000   500000    500000      50% /mnt/backup
overlay           51475068  21000000  28045376      43% /var/lib/docker/overlay2/def/merged
";

    #[test]
    fn real_disks_are_listed_once_and_pseudo_ones_are_left_out() {
        let found = parse_filesystems(LINUX);
        let mounts: Vec<&str> = found.iter().map(|fs| fs.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/boot/efi", "/data", "/mnt/backup"]);
        let root = &found[0];
        assert_eq!((root.device.as_str(), root.total, root.used, root.available), ("/dev/vda1", 51475068 * 1024, 21000000 * 1024, 28045376 * 1024));
    }

    #[test]
    fn a_containers_overlay_root_still_counts() {
        let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\noverlay 61202244 25000000 33062844 44% /\nshm 65536 0 65536 0% /dev/shm\n/dev/vda1 61202244 25000000 33062844 44% /etc/hosts\n";
        let mounts: Vec<String> = parse_filesystems(out).into_iter().map(|fs| fs.mount).collect();
        assert_eq!(mounts, ["/"], "容器里 / 是 overlay；bind mount 进来的 /etc/hosts 不是盘");
    }

    /// 一台装了 Xcode 的 Mac 上的原样输出：只剩数据卷和外接盘。系统卷 /、iOS 模拟器镜像、cryptex、Xcode 的 DeviceFS
    /// （设备名里有 file:///，不是网络盘）都是 nobrowse 或只读的系统卷，清理不了空间。
    #[test]
    fn a_mac_lists_only_the_data_volume_and_real_disks() {
        let out = "\
Filesystem     1024-blocks      Used  Available Capacity  Mounted on
/dev/disk3s1s1   971350180  13438012  455956192     3%    /
devfs                  237       237          0   100%    /dev
/dev/disk3s6     971350180   3145760  455956192     1%    /System/Volumes/VM
/dev/disk3s5     971350180 487539684  455956192    52%    /System/Volumes/Data
map auto_home            0         0          0   100%    /System/Volumes/Data/home
/dev/disk5s1      17659904  17159516     455144    98%    /Library/Developer/CoreSimulator/Volumes/iOS_23F77
/dev/disk7s1       3221504   3098920     109136    97%    /private/var/run/com.apple.security.cryptexd/mnt/com.apple.MobileAsset.MetalToolchain-v27.2.47.0.aMwan8
devices -- file:///Users/me/Library/Containers/com.apple.CoreDevice.CoreDeviceService/Data/  1073741824         0 1073741824     0%    /Users/me/Library/Developer/CoreDevice/DeviceFS
/dev/disk9s1      62914560  10000000   52914560    16%    /Volumes/My Backup
@@mount
/dev/disk3s1s1 on / (apfs, sealed, local, read-only, journaled)
devfs on /dev (devfs, local, nobrowse)
/dev/disk3s6 on /System/Volumes/VM (apfs, local, noexec, journaled, noatime, nobrowse)
/dev/disk3s5 on /System/Volumes/Data (apfs, local, journaled, nobrowse, protect, root data)
map auto_home on /System/Volumes/Data/home (autofs, automounted, nobrowse)
/dev/disk5s1 on /Library/Developer/CoreSimulator/Volumes/iOS_23F77 (apfs, sealed, local, nodev, nosuid, read-only, journaled, noatime, nobrowse)
/dev/disk7s1 on /private/var/run/com.apple.security.cryptexd/mnt/com.apple.MobileAsset.MetalToolchain-v27.2.47.0.aMwan8 (apfs, sealed, local, read-only, journaled, nobrowse)
devices -- file:///Users/me/Library/Containers/com.apple.CoreDevice.CoreDeviceService/Data/ on /Users/me/Library/Developer/CoreDevice/DeviceFS (devicefs, local, nodev, nosuid, noowners, noatime, quarantine, nobrowse, fskit, mounted by me)
/dev/disk9s1 on /Volumes/My Backup (apfs, local, nodev, nosuid, journaled, noowners)
";
        let mounts: Vec<String> = parse_filesystems(out).into_iter().map(|fs| fs.mount).collect();
        assert_eq!(mounts, ["/System/Volumes/Data", "/Volumes/My Backup"]);
    }

    /// 本机：只要文件（子目录、子目录里的不要），从大到小。
    #[tokio::test]
    async fn files_in_a_folder_biggest_first() {
        let root = std::env::temp_dir().join(format!("sshl-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("small.txt"), vec![1u8; 100]).unwrap();
        std::fs::write(root.join("big.log"), vec![1u8; 300_000]).unwrap();
        std::fs::write(root.join("sub/inner.bin"), vec![1u8; 900_000]).unwrap();
        let files = files_in(&AppState::new(), &Machine::Local, &root.to_string_lossy()).await.unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(files.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["big.log", "small.txt"]);
        assert!(files[0].1 >= 300_000, "按占的块算");
    }

    /// Linux 的 mount 输出（「on 挂载点 type 类型」）不影响：照 df 的规则来。
    #[test]
    fn linux_mount_output_changes_nothing() {
        let out = format!("{LINUX}@@mount\n/dev/vda1 on / type ext4 (rw,relatime)\n/dev/vdb1 on /data type xfs (rw,noatime)\n");
        let mounts: Vec<String> = parse_filesystems(&out).into_iter().map(|fs| fs.mount).collect();
        assert_eq!(mounts, ["/", "/boot/efi", "/data", "/mnt/backup"]);
    }

    #[test]
    fn the_scan_runs_quietly_and_quotes_the_path() {
        let command = scan_command("/srv/it's");
        assert!(command.contains("du -xk $d -- '/srv/it'\\''s'"), "{command}");
        assert!(command.contains("nice -n 19") && command.contains("ionice -c3") && command.contains("taskpolicy -b"), "{command}");
        assert!(command.contains("--max-depth=4") && command.contains("-d 4"), "GNU 老版本和 BusyBox / macOS 的写法都有：{command}");
    }

    const DU: &str = "\
8\t/var/lib/apt/lists/partial
50000\t/var/lib/apt/lists
50100\t/var/lib/apt
900000\t/var/lib/docker
950200\t/var/lib
4\t/var/empty
300000\t/var/log/journal
320000\t/var/log
1270300\t/var
12\t/etc
1300000\t/
";

    #[test]
    fn a_scan_becomes_a_tree_sorted_by_size() {
        let tree = parse_du(DU, "/", 2, 4);
        assert_eq!(tree.root(), "/");
        assert_eq!(tree.size("/"), Some(1300000 * 1024));
        assert_eq!(tree.unreadable, 2);
        let names = |path: &str| tree.entries(path).into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names("/"), ["var", "etc"]);
        assert_eq!(names("/var"), ["lib", "log", "empty"], "从大到小");
        assert_eq!(names("/var/lib"), ["docker", "apt"]);
        let var = &tree.entries("/")[0];
        assert_eq!((var.path.as_str(), var.size, var.has_children), ("/var", 1270300 * 1024, true));
        assert!(!tree.entries("/var")[2].has_children, "空目录点不进去");
        // 直接放在这一层的文件：/ 下面 1300000 - 1270300 - 12
        assert_eq!(tree.files_size("/"), (1300000 - 1270300 - 12) * 1024);
        assert_eq!(tree.files_size("/var/empty"), 4 * 1024);
    }

    /// 只列 4 层：第 4 层的目录能往下点（还没扫到），比它深的行不要；点进去补扫那一个目录，接回树上。
    #[test]
    fn the_deepest_level_is_scanned_again_when_opened() {
        let mut tree = parse_du(DU, "/", 0, 4);
        let apt = tree.entries("/var/lib/apt");
        assert_eq!(apt.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["lists"]);
        assert!(apt[0].has_children, "还没扫到那么深：能往下点");
        assert!(tree.needs_scan("/var/lib/apt/lists"));
        assert_eq!(tree.size("/var/lib/apt/lists/partial"), None, "第 5 层不要");
        assert_eq!(tree.files_size("/var/lib/apt/lists"), 0, "还没扫的不知道文件多大");
        assert!(!tree.needs_scan("/var/lib"));

        // 补扫：大小变大了，在上一层里排到前面
        let sub = parse_du("8\t/var/lib/apt/lists/partial\n2000000\t/var/lib/apt/lists\n", "/var/lib/apt/lists", 1, 4);
        tree.merge(sub);
        assert!(!tree.needs_scan("/var/lib/apt/lists"));
        assert_eq!(tree.entries("/var/lib/apt/lists").iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["partial"]);
        assert_eq!(tree.files_size("/var/lib/apt/lists"), (2000000 - 8) * 1024);
        assert_eq!(tree.unreadable, 1);
        let apt = tree.entries("/var/lib/apt");
        assert_eq!(apt[0].size, 2000000 * 1024);
    }

    /// 真实 Linux（容器同 sftp 的 real_sftp，以 alice 登录）：列得出 /；扫一个造好的目录，层级、大小、这一层的文件、
    /// 读不了的目录都对。`SSHL_SFTP_IT=127.0.0.1:2223 cargo test --lib scans_a_real_linux_machine -- --ignored`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "需要 SSHL_SFTP_IT=host:port 指向 sshd 测试容器"]
    async fn scans_a_real_linux_machine() {
        let Ok(target) = std::env::var("SSHL_SFTP_IT") else { return };
        let (host, port) = target.split_once(':').unwrap();
        let handle = crate::ssh::do_handshake(
            host,
            port.parse().unwrap(),
            "alice",
            crate::ssh::Credentials::Password("alicepw"),
            crate::known_hosts::HostKeyCheck::trust_any_for_tests(),
        )
        .await
        .unwrap();
        let state = AppState::new();
        state.insert("it".into(), std::sync::Arc::new(crate::ssh::SshSession::for_test(handle, "alice"))).await;
        let machine = Machine::Remote("it".into());

        let found = filesystems(&state, &machine).await.unwrap();
        assert!(found.iter().any(|fs| fs.mount == "/" && fs.total > fs.used), "{found:?}");

        let base = server_info::run(&state, &machine, "mktemp -d /home/alice/sshl-du.XXXXXX").await.unwrap().trim().to_string();
        let setup = "mkdir -p big/deeper small locked && head -c 2097152 /dev/zero > big/deeper/blob && head -c 1024 /dev/zero > small/f \
            && head -c 65536 /dev/zero > top.bin && chmod 000 locked";
        server_info::run(&state, &machine, &format!("cd '{base}' && {setup}")).await.unwrap();
        let tree = scan(&state, &machine, &base, Default::default()).await.unwrap();
        let files = files_in(&state, &machine, &base).await.unwrap();
        server_info::run(&state, &machine, &format!("chmod 755 '{base}/locked' && rm -rf -- '{base}'")).await.unwrap();
        assert_eq!(files, [("top.bin".to_string(), 65536)], "这一层只有 top.bin（子目录不算）");

        assert_eq!(tree.root(), base);
        let entries = tree.entries(&base);
        let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names[0], "big", "最大的在最前：{entries:?}");
        names.sort();
        assert_eq!(names, ["big", "locked", "small"]);
        assert!(entries[0].size >= 2 * 1024 * 1024 && entries[0].has_children, "{entries:?}");
        assert_eq!(tree.entries(&format!("{base}/big")).iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["deeper"]);
        assert!(tree.files_size(&base) >= 65536, "top.bin 算在这一层的文件里：{}", tree.files_size(&base));
        assert_eq!(tree.unreadable, 1, "locked 读不了");

        // 比 4 层深的：第一次只列到第 4 层（那一层能往下点），补扫后接上，大小一路都算进去了
        let deep = server_info::run(&state, &machine, "mktemp -d /home/alice/sshl-deep.XXXXXX").await.unwrap().trim().to_string();
        server_info::run(&state, &machine, &format!("cd '{deep}' && mkdir -p a/b/c/d/e/f && head -c 3145728 /dev/zero > a/b/c/d/e/f/blob")).await.unwrap();
        let mut tree = scan(&state, &machine, &deep, Default::default()).await.unwrap();
        let fourth = format!("{deep}/a/b/c/d");
        assert!(tree.needs_scan(&fourth), "第 4 层还没往下列");
        assert_eq!(tree.size(&format!("{fourth}/e")), None);
        assert!(tree.size(&format!("{deep}/a")).unwrap() >= 3 * 1024 * 1024, "深处的文件算进了上面每一层");
        tree.merge(scan(&state, &machine, &fourth, Default::default()).await.unwrap());
        assert!(!tree.needs_scan(&fourth));
        assert_eq!(tree.entries(&fourth).iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["e"]);
        assert!(tree.size(&format!("{fourth}/e/f")).is_some());
        server_info::run(&state, &machine, &format!("rm -rf -- '{deep}'")).await.unwrap();
    }

    #[test]
    fn a_scan_below_the_root_and_odd_lines() {
        let out = "4\t/data/a/x\n10\t/data/a\nnot a line\n6\t/elsewhere\n12\t/data/\n";
        let tree = parse_du(out, "/data/", 0, 4);
        assert_eq!(tree.root(), "/data", "末尾的 / 不算");
        assert_eq!(tree.size("/data"), Some(12 * 1024));
        assert_eq!(tree.entries("/data").iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["/data/a"]);
        assert_eq!(tree.size("/elsewhere"), None, "不在扫的分区下面的不要");
    }
}

//! 「系统」页的「磁盘」：看磁盘空间被什么占了（磁盘快满时找是哪个文件夹、哪个文件）。点开先看默认分区（/，没有就用得最多的那个），
//! 像 ncdu 一样按大小一层层往下点；每一层子目录和文件放在一起排，大文件一眼看得到。别的分区在顶上切换。
//! 宽的时候列表右边是矩形树图（同 Mole，见 `treemap`），面积就是占用。
//!
//! 扫描要用户点（「开始扫描」）：读遍一个大分区很重，不能一打开页面就压到服务器上。离开这一页（换标签、换页面、换会话、换分区）
//! 就停下（远程杀掉 du，本机的扫描线程收手），回来时显示「已停止」，等用户再点，不会自己重新开始；失败了也不自己重试。
//! 点进第 4 层往下还没列出来的目录时补扫那一个目录（点进去就是要看），一次只补扫一个，离开那个目录就停下。
//!
//! 要快、不卡：远程一次只列 4 层（du 照样走完整棵树，总数是准的，输出几万行以内），点到更深的目录时只补扫那一个目录；
//! 本机多线程并行扫（`sshl_core::disk_scan`）。每一层的文件在打开那一层时才列（一个目录，很快）。
//! 当前这一层的列表缓存着，换目录、改搜索词、扫描或文件列表变了才重算，每帧只画看得见的几行。
//! 远程的 du 挂在最低的 IO / CPU 优先级下跑。扫描结果按机器和分区留着：切走再回来不用重扫，要新的点「重新扫描」。
//!
//! 本机标签也有：第一次扫描时 macOS 可能会问 SSHL 能不能访问「文稿」「桌面」这些文件夹，不允许的就算不进来。

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

use gpui::{Animation, AnimationExt, Font, TextRun, Transformation, container_query, percentage, svg};
use sshl_core::disk_usage::{self, DiskEntry, DiskTree, Filesystem, ScanProgress};

use super::treemap::{self, Item};
use super::*;

const USAGE_WIDTH: f32 = 220.;
const SIZE_WIDTH: f32 = 96.;
const SHARE_WIDTH: f32 = 72.;
/// 每一层最多列这么多个文件（最大的那些），其余的合成一行。
pub const FILES_SHOWN: usize = 200;
/// 宽过这么多才在列表右边画矩形树图（同 Mole）；窄了只有列表，带用量条。
const SPLIT_MIN_WIDTH: f32 = 760.;
/// 和矩形树图并排时列表占总宽的这么多，夹在下面两个数之间。
const LIST_SHARE: f32 = 0.4;
const LIST_MIN_WIDTH: f32 = 340.;
const LIST_MAX_WIDTH: f32 = 460.;
/// 矩形树图和列表、卡片边的距离；块之间的缝。
const MAP_INSET: f32 = 12.;
const TILE_GAP: f32 = 3.;
/// 色块的不透明度：(最小的块, 最大的块)，[浅色, 深色]。大的颜色深一点，越大越显眼。
const TILE_TINT: [(f32, f32); 2] = [(0.10, 0.32), (0.14, 0.40)];
/// 合起来的那些（「其余 N 项」、还没列出来的文件）是灰的。
const REST_TINT: [f32; 2] = [0.05, 0.08];
/// 还没有扫描结果时铺在底下的那张示意矩形树图：这些比例的灰块，很淡。
const GHOST: [u64; 10] = [34, 21, 13, 9, 7, 5, 4, 3, 2, 2];
const GHOST_TINT: [f32; 2] = [0.035, 0.06];
/// 正中那张卡片的宽。
const STAGE_WIDTH: f32 = 440.;

/// 一个分区的扫描。
pub(super) enum Scan {
    Running {
        started: Instant,
        /// 本机并行扫描的进度（远程用 du，一直是 0）。
        progress: Arc<ScanProgress>,
    },
    Done {
        tree: Arc<DiskTree>,
        at: Instant,
        /// 正在补扫的目录（点到了还没列出来的深处）、开始的时间和进度。
        deeper: Option<(String, Instant, Arc<ScanProgress>)>,
        /// 补扫失败的目录和原因。
        failed: Option<(String, String)>,
    },
    Failed(String),
    /// 扫到一半离开了这一页：等用户再点「开始扫描」。
    Stopped,
}

/// 一个目录里直接放着的文件。
enum Files {
    Loading,
    Loaded(Arc<Vec<(String, u64)>>),
    Failed,
}

/// 一层里的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Dir(DiskEntry),
    File { name: String, path: String, size: u64 },
    /// 列不下的那些文件合成一行。
    MoreFiles { count: usize, size: u64 },
    /// 文件还没列出来（或列失败）：这一层文件的总和。
    Files(u64),
}

impl Row {
    fn size(&self) -> u64 {
        match self {
            Row::Dir(entry) => entry.size,
            Row::File { size, .. } | Row::MoreFiles { size, .. } | Row::Files(size) => *size,
        }
    }

    /// 代表几项（矩形树图把小的合成「其余 N 项」时数）。
    fn count(&self) -> usize {
        match self {
            Row::MoreFiles { count, .. } => *count,
            _ => 1,
        }
    }
}

/// 鼠标停在哪：列表的一行和矩形树图里它那一块一起亮。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hover {
    Row(usize),
    /// 「其余 N 项」那一块。
    Rest,
}

/// 当前这一层要画的东西：(机器, 目录, 搜索词, 版本) 变了才重算。
struct Listing {
    key: (Machine, String, String, u64),
    rows: Arc<Vec<Row>>,
    total: u64,
}

#[derive(Default)]
pub(super) struct DiskState {
    /// 当前机器的分区（每 3 秒随页面刷新）。
    pub(super) filesystems: Option<Vec<Filesystem>>,
    /// (机器, 分区挂载点) → 扫描。
    scans: HashMap<(Machine, String), Scan>,
    /// 每台机器看的哪个分区、到了哪一层：(挂载点, 目录)。
    at: HashMap<Machine, (String, String)>,
    /// (机器, 目录) → 里面的文件。
    files: HashMap<(Machine, String), Files>,
    /// `scans` / `files` 每变一次加一：`listing` 据此知道要重算。
    version: u64,
    listing: Option<Listing>,
    /// 列表（行高跟着内容：名字长了折行）。
    rows: kit::Rows,
    hovered: Option<Hover>,
}

impl DiskState {
    /// 停下所有正在跑的扫描：整个分区的变成「已停止」，补扫的目录回到「还没扫」。有停下的返回 true。
    fn stop_all(&mut self) -> bool {
        let mut stopped = false;
        for scan in self.scans.values_mut() {
            match scan {
                Scan::Running { progress, .. } => {
                    progress.stop();
                    *scan = Scan::Stopped;
                    stopped = true;
                }
                Scan::Done { deeper, .. } => {
                    if let Some((_, _, progress)) = deeper.take() {
                        progress.stop();
                        stopped = true;
                    }
                }
                Scan::Failed(_) | Scan::Stopped => {}
            }
        }
        if stopped {
            self.version += 1;
        }
        stopped
    }

    /// 整个分区扫完：只认还在跑的这一次（期间停下过、又开始了一次新的，旧的结果不要）。收下了返回 true。
    fn finish_scan(&mut self, key: &(Machine, String), progress: &Arc<ScanProgress>, scan: Scan) -> bool {
        let current = matches!(self.scans.get(key), Some(Scan::Running { progress: running, .. }) if Arc::ptr_eq(running, progress));
        if current {
            self.scans.insert(key.clone(), scan);
            self.version += 1;
        }
        current
    }

    /// 补扫完：同样只认还在跑的这一次，接回树上（失败的记下原因，等用户再点）。
    fn finish_deeper(&mut self, key: &(Machine, String), progress: &Arc<ScanProgress>, result: Result<DiskTree, String>) -> bool {
        let Some(Scan::Done { tree, deeper, failed, .. }) = self.scans.get_mut(key) else { return false };
        let Some((path, ..)) = deeper.take_if(|(_, _, running)| Arc::ptr_eq(running, progress)) else { return false };
        match result {
            Ok(sub) => Arc::make_mut(tree).merge(sub),
            Err(error) => *failed = Some((path, error)),
        }
        self.version += 1;
        true
    }
}

// ---------------------------------------------------------------- 纯函数（有测试）

/// 面包屑第一段（分区根）叫什么：/ 和 Mac 的数据卷是「整个磁盘」（同 Mole 的 Whole Disk；单独一个「/」看着奇怪，用户报过），
/// 别的分区就是挂载点（/data、/Volumes/备份）。
pub fn root_label(mount: &str) -> String {
    if mount == "/" || mount == "/System/Volumes/Data" { tr!("整个磁盘").into() } else { mount.to_string() }
}

/// 面包屑：从分区根到 `path` 的每一段（显示的名字, 那一段的路径）。根见 [`root_label`]。
pub fn crumbs(root: &str, path: &str) -> Vec<(String, String)> {
    let mut out = vec![(root_label(root), root.to_string())];
    let rest = path.strip_prefix(root).unwrap_or("");
    let mut current = root.to_string();
    for part in rest.split('/').filter(|part| !part.is_empty()) {
        current = if current == "/" { format!("/{part}") } else { format!("{current}/{part}") };
        out.push((part.to_string(), current.clone()));
    }
    out
}

/// 占这一层的多少。
pub fn share(size: u64, total: u64) -> String {
    if total == 0 || size == 0 {
        return "0%".into();
    }
    let percent = size as f64 * 100. / total as f64;
    if percent < 0.1 { "<0.1%".into() } else if percent >= 10. { format!("{percent:.0}%") } else { format!("{percent:.1}%") }
}

/// 扫描结果有多旧。
pub fn scanned_ago(secs: u64) -> String {
    match secs {
        0..60 => tr!("刚刚扫描").into(),
        60..3600 => tr!("{} 分钟前扫描", secs / 60),
        _ => tr!("{} 小时前扫描", secs / 3600),
    }
}

/// 大数字每三位一个逗号：644498 → 644,498。
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// 这一层：子目录和文件放在一起从大到小（一样大时目录在前）。文件最多列 [`FILES_SHOWN`] 个最大的，其余合成一行；
/// 文件还没列出来时用一行「这一层的文件」总和代替（搜索时不显示）。搜索：名字里含关键字（不分大小写）。
pub fn rows(dirs: Vec<DiskEntry>, files: Option<&[(String, u64)]>, files_total: u64, dir: &str, query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let matches = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
    let mut out: Vec<Row> = dirs.into_iter().filter(|entry| matches(&entry.name)).map(Row::Dir).collect();
    let mut tail = None;
    match files {
        Some(files) => {
            let mut matching = files.iter().filter(|(name, _)| matches(name));
            for (name, size) in matching.by_ref().take(FILES_SHOWN) {
                let path = if dir == "/" { format!("/{name}") } else { format!("{dir}/{name}") };
                out.push(Row::File { name: name.clone(), path, size: *size });
            }
            let (count, size) = matching.fold((0, 0), |(count, total), (_, size)| (count + 1, total + size));
            if count > 0 {
                tail = Some(Row::MoreFiles { count, size });
            }
        }
        None if query.is_empty() && files_total > 0 => tail = Some(Row::Files(files_total)),
        None => {}
    }
    // 稳定排序：一样大时目录（先放进去的）在前
    out.sort_by_key(|row| std::cmp::Reverse(row.size()));
    out.extend(tail);
    out
}

/// 点开磁盘页时先看哪个分区：有 / 就是 /，没有（Mac 上是数据卷）就是用得最多的那个。
pub fn default_volume(filesystems: &[Filesystem]) -> Option<String> {
    filesystems.iter().find(|fs| fs.mount == "/").or_else(|| filesystems.iter().max_by_key(|fs| fs.used)).map(|fs| fs.mount.clone())
}

/// 重新扫描后树只有前几层：`path` 不在树上了就退到树上最深的祖先（不会退出分区根）。
pub fn deepest_known(tree: &DiskTree, mount: &str, path: &str) -> String {
    let mut current = path.to_string();
    while tree.size(&current).is_none() && current.len() > mount.len() {
        current = match current.rsplit_once('/') {
            Some(("", _)) => "/".to_string(),
            Some((parent, _)) => parent.to_string(),
            None => break,
        };
    }
    if current.len() < mount.len() { mount.to_string() } else { current }
}

fn spinner(size: f32) -> impl IntoElement {
    svg()
        .path(Icon::LoaderCircle.path())
        .size(px(size))
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .with_animation("disk-spin", Animation::new(Duration::from_secs(1)).repeat(), |svg, delta| {
            svg.with_transformation(Transformation::rotate(percentage(delta)))
        })
}

/// 一条用量条：`ratio` 是 0–1。
fn bar(ratio: f32, color: ui::Token) -> Div {
    let dark = usize::from(theme::is_dark());
    div()
        .flex_1()
        .h(px(6.))
        .rounded_full()
        .bg(theme::alpha(color, RING_TRACK[dark]))
        .child(div().h_full().rounded_full().w(gpui::relative(ratio.clamp(0., 1.))).bg(theme::color(color)))
}

/// 本机：在访达里选中这个目录 / 文件。测试口开着时只记日志，不去开访达。
pub(super) fn reveal_in_finder(path: &str, cx: &mut App) {
    tracing::info!("reveal: {path}");
    #[cfg(any(debug_assertions, feature = "devhook"))]
    if std::env::var_os("SSHL_DEV_SOCKET").is_some() {
        return;
    }
    cx.reveal_path(std::path::Path::new(path));
}

impl SystemPanel {
    /// 标签上的数字：几个分区。
    pub(super) fn disk_count(&self) -> Option<usize> {
        self.disk.filesystems.as_ref().map(Vec::len)
    }

    /// 停下所有正在跑的扫描（离开磁盘页：换标签、换页面、换会话、换分区）：远程的 du 被杀掉，本机的扫描线程收手。
    /// 不会自己重新开始，等用户再点。
    pub(super) fn stop_scans(&mut self, cx: &mut Context<Self>) {
        if self.disk.stop_all() {
            cx.notify();
        }
    }

    /// 看这个分区：只是切过去（扫过的直接显示），不自己开始扫——读遍一个大分区很重，要用户点。换了分区，别的扫描停下。
    fn open_filesystem(&mut self, mount: String, cx: &mut Context<Self>) {
        let Some(machine) = self.machine.clone() else { return };
        if self.disk.at.get(&machine).is_some_and(|(current, _)| *current == mount) {
            self.go_to(mount, cx);
            return;
        }
        self.stop_scans(cx);
        self.disk.at.insert(machine, (mount.clone(), mount));
        self.disk.rows.reset();
        self.disk.hovered = None;
        cx.notify();
    }

    /// 扫整个分区（用户点「开始扫描」「重新扫描」）。一次只跑一个：别的先停下。之前列过的文件作废（重新扫描就是要新的）。
    fn start_scan(&mut self, mount: String, cx: &mut Context<Self>) {
        let Some(machine) = self.machine.clone() else { return };
        self.disk.stop_all();
        let below = if mount == "/" { "/".to_string() } else { format!("{mount}/") };
        self.disk.files.retain(|(owner, path), _| *owner != machine || (*path != mount && !path.starts_with(&below)));
        let key = (machine.clone(), mount.clone());
        let progress = Arc::new(ScanProgress::default());
        self.disk.scans.insert(key.clone(), Scan::Running { started: Instant::now(), progress: progress.clone() });
        self.disk.version += 1;
        self.tick_while_scanning(key.clone(), cx);
        let running = progress.clone();
        let scanned = backend::spawn(async move { disk_usage::scan(&backend::get().state, &machine, &mount, progress).await });
        cx.spawn(async move |this, cx| {
            let scan = match scanned.await {
                Some(Ok(tree)) => Scan::Done { tree: Arc::new(tree), at: Instant::now(), deeper: None, failed: None },
                Some(Err(error)) => Scan::Failed(error.to_string()),
                None => Scan::Failed(tr!("后台任务异常退出").into()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.disk.finish_scan(&key, &running, scan) {
                    this.settle_at(&key);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 补扫一个还没列出来的目录（用户点进去，或点「扫描这个目录」），接回这个分区的树上。一次只补扫一个：换了目录，旧的停下。
    fn scan_deeper(&mut self, key: (Machine, String), path: String, cx: &mut Context<Self>) {
        let Some(Scan::Done { deeper, failed, .. }) = self.disk.scans.get_mut(&key) else { return };
        if deeper.as_ref().is_some_and(|(scanning, ..)| *scanning == path) {
            return;
        }
        if let Some((_, _, old)) = deeper.take() {
            old.stop();
        }
        let progress = Arc::new(ScanProgress::default());
        *deeper = Some((path.clone(), Instant::now(), progress.clone()));
        *failed = None;
        self.disk.version += 1;
        self.tick_while_scanning(key.clone(), cx);
        let running = progress.clone();
        let (machine, root) = (key.0.clone(), path);
        let scanned = backend::spawn(async move { disk_usage::scan(&backend::get().state, &machine, &root, progress).await });
        cx.spawn(async move |this, cx| {
            let result = match scanned.await {
                Some(Ok(tree)) => Ok(tree),
                Some(Err(error)) => Err(error.to_string()),
                None => Err(tr!("后台任务异常退出").to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.disk.finish_deeper(&key, &running, result) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 列一个目录里的文件（打开那一层时）。
    fn load_files(&mut self, machine: Machine, path: String, cx: &mut Context<Self>) {
        let key = (machine.clone(), path.clone());
        self.disk.files.insert(key.clone(), Files::Loading);
        let listed = backend::spawn(async move { disk_usage::files_in(&backend::get().state, &machine, &path).await });
        cx.spawn(async move |this, cx| {
            let files = match listed.await {
                Some(Ok(files)) => Files::Loaded(Arc::new(files)),
                _ => Files::Failed,
            };
            let _ = this.update(cx, |this, cx| {
                // 期间重新扫描过（列表被清掉了）：结果不要
                if matches!(this.disk.files.get(&key), Some(Files::Loading)) {
                    this.disk.files.insert(key, files);
                    this.disk.version += 1;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 扫描中每秒画一次（显示用了多久；页面的 3 秒刷新太慢）。
    fn tick_while_scanning(&mut self, key: (Machine, String), cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let scanning = this.update(cx, |this, cx| {
                    cx.notify();
                    matches!(this.disk.scans.get(&key), Some(Scan::Running { .. } | Scan::Done { deeper: Some(_), .. }))
                });
                if !scanning.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// 整个分区扫完：停着的那一层不在新树上（重新扫描时停在深处）就退到树上最深的祖先。那一层要补扫的话等用户点，不接着自己扫。
    fn settle_at(&mut self, key: &(Machine, String)) {
        let Some(Scan::Done { tree, .. }) = self.disk.scans.get(key) else { return };
        let Some((mount, path)) = self.disk.at.get(&key.0).filter(|(mount, _)| *mount == key.1) else { return };
        let settled = deepest_known(tree, mount, path);
        if let Some(at) = self.disk.at.get_mut(&key.0) {
            at.1 = settled;
        }
    }

    /// 点进一个目录（或面包屑往回走）。补扫只为正在看的那个目录：离开它就停下；点进的目录还没列出来就补扫它（点进去就是要看）。
    fn go_to(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(machine) = self.machine.clone() else { return };
        let Some(at) = self.disk.at.get_mut(&machine) else { return };
        at.1 = path.clone();
        let key = (machine, at.0.clone());
        self.disk.rows.reset();
        self.disk.hovered = None;
        let needs_scan = match self.disk.scans.get_mut(&key) {
            Some(Scan::Done { tree, deeper, .. }) => {
                if let Some((_, _, progress)) = deeper.take_if(|(scanning, ..)| *scanning != path) {
                    progress.stop();
                    self.disk.version += 1;
                }
                tree.needs_scan(&path)
            }
            _ => false,
        };
        if needs_scan {
            self.scan_deeper(key, path, cx);
        }
        cx.notify();
    }

    /// 当前这一层的行和总大小（见 [`Listing`]）。这一层的文件还没列就去列（列好了版本一变，这里重算）。
    fn listing(&mut self, cx: &mut Context<Self>) -> Option<(Arc<Vec<Row>>, u64)> {
        let machine = self.machine.clone()?;
        let (mount, path) = self.disk.at.get(&machine).cloned()?;
        let query = self.search.read(cx).value().trim().to_string();
        let key = (machine.clone(), path.clone(), query.clone(), self.disk.version);
        if let Some(listing) = self.disk.listing.as_ref().filter(|listing| listing.key == key) {
            return Some((listing.rows.clone(), listing.total));
        }
        let Some(Scan::Done { tree, .. }) = self.disk.scans.get(&(machine.clone(), mount)) else { return None };
        let tree = tree.clone();
        if !tree.needs_scan(&path) && !self.disk.files.contains_key(&(machine.clone(), path.clone())) {
            self.load_files(machine.clone(), path.clone(), cx);
        }
        let files = match self.disk.files.get(&(machine, path.clone())) {
            Some(Files::Loaded(files)) => Some(files.clone()),
            _ => None,
        };
        let rows = Arc::new(rows(tree.entries(&path), files.as_deref().map(Vec::as_slice), tree.files_size(&path), &path, &query));
        let total = tree.size(&path).unwrap_or(0);
        self.disk.listing = Some(Listing { key, rows: rows.clone(), total });
        // 行变了，记着的第几行不再是鼠标下那一行
        self.disk.hovered = None;
        Some((rows, total))
    }

    /// 整个「磁盘」页（标签栏下面的部分）。第一次看这台机器时先打开默认分区（不扫，等用户点）。
    pub(super) fn render_disk(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(machine) = self.machine.clone() else { return empty_message(tr!("连接服务器后显示它的进程、端口和服务")) };
        if !self.disk.at.contains_key(&machine) {
            match self.disk.filesystems.as_deref().and_then(default_volume) {
                Some(mount) => self.open_filesystem(mount, cx),
                None => {
                    return match &self.error {
                        Some(error) => empty_message(list_error_message(error)),
                        None if self.disk.filesystems.is_some() => empty_message(tr!("没有内容")),
                        None => empty_message(tr!("正在读取…")),
                    };
                }
            }
        }
        let Some((mount, path)) = self.disk.at.get(&machine).cloned() else { return empty_message(tr!("正在读取…")) };
        self.render_scan(&machine, mount, path, cx)
    }

    /// 别的分区：一排按钮，带用了多少；正在看的那个蓝色。只有一个分区时不显示。
    fn volume_switcher(&self, current: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let filesystems = self.disk.filesystems.as_deref().filter(|filesystems| filesystems.len() > 1)?;
        Some(
            div()
                .flex_shrink_0()
                .px_3()
                .pb_2()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .children(filesystems.iter().enumerate().map(|(index, fs)| {
                    let active = fs.mount == current;
                    let ratio = fs.used as f32 / fs.total.max(1) as f32;
                    let color = usage_color(ratio, ui::TEAL);
                    let mount = fs.mount.clone();
                    div()
                        .id(("disk-volume", index))
                        .role(Role::Button)
                        .aria_label(SharedString::from(fs.mount.clone()))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .py(px(4.))
                        .rounded(px(7.))
                        .text_size(px(12.))
                        .border_1()
                        .cursor_pointer()
                        .map(|d| {
                            if active {
                                d.border_color(theme::color(ui::PRIMARY)).bg(theme::alpha(ui::PRIMARY, 0.08)).text_color(theme::color(ui::PRIMARY))
                            } else {
                                d.border_color(theme::color(ui::BORDER)).hover(|s| s.bg(theme::alpha(ui::FOREGROUND, 0.05)))
                            }
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_filesystem(mount.clone(), cx)))
                        .child(div().whitespace_nowrap().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(fs.mount.clone()))
                        .child(div().font_features(TABULAR.clone()).text_color(theme::color(readable(color))).child(format!("{:.0}%", ratio * 100.)))
                }))
                .into_any_element(),
        )
    }

    fn render_scan(&mut self, machine: &Machine, mount: String, path: String, cx: &mut Context<Self>) -> AnyElement {
        // 先把要用的取出来：下面还要可变地借 self 画树
        enum Shown {
            /// 还没扫，或扫到一半离开过（true）：等用户点「开始扫描」。
            Idle(bool),
            /// 整个分区在扫（秒数, 已扫的文件数, 字节）。
            Scanning(u64, u64, u64),
            /// 这一层在补扫。
            Deeper(u64, u64, u64),
            /// 整个分区扫失败了。
            Failed(String),
            /// 这一层还没列出来（第 4 层往下）：等用户点「扫描这个目录」。带着上次补扫失败的原因。
            DeeperIdle(Option<String>),
            Done(Instant, usize),
        }
        let counted = |started: &Instant, progress: &ScanProgress| {
            (started.elapsed().as_secs(), progress.files.load(Ordering::Relaxed), progress.bytes.load(Ordering::Relaxed))
        };
        let key = (machine.clone(), mount.clone());
        let shown = match self.disk.scans.get(&key) {
            None => Shown::Idle(false),
            Some(Scan::Stopped) => Shown::Idle(true),
            Some(Scan::Running { started, progress }) => {
                let (secs, files, bytes) = counted(started, progress);
                Shown::Scanning(secs, files, bytes)
            }
            Some(Scan::Failed(error)) => Shown::Failed(error.clone()),
            Some(Scan::Done { deeper: Some((scanning, started, progress)), .. }) if *scanning == path => {
                let (secs, files, bytes) = counted(started, progress);
                Shown::Deeper(secs, files, bytes)
            }
            Some(Scan::Done { failed: Some((at, error)), .. }) if *at == path => Shown::DeeperIdle(Some(error.clone())),
            Some(Scan::Done { tree, .. }) if tree.needs_scan(&path) => Shown::DeeperIdle(None),
            Some(Scan::Done { tree, at, .. }) => Shown::Done(*at, tree.unreadable),
        };
        let local = *machine == Machine::Local;
        let rescan_mount = mount.clone();
        // 有扫好的树才在面包屑右边放「重新扫描」：没扫过、扫失败的在中间有自己的按钮
        let has_tree = matches!(shown, Shown::Deeper(..) | Shown::DeeperIdle(_) | Shown::Done(..));
        let switcher = self.volume_switcher(&mount, cx);
        // 这个分区用了多少（同 Mole 右上角的用量条）。还没有扫描结果时中间的卡片里有大的，这里不重复
        let usage = self.disk.filesystems.as_deref().and_then(|all| all.iter().find(|fs| fs.mount == mount)).map(|fs| (fs.used, fs.total));
        // 面包屑：整个磁盘 › 目录 › 目录 … 右边是用量和重新扫描
        let crumbs_bar = div()
            .flex_shrink_0()
            .px_3()
            .pb_2()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            // 层级多、名字长时面包屑换到下一行，不截断；一段比整行还长时它自己折行
            .child(div().flex_1().min_w_0().flex().flex_wrap().items_center().gap(px(6.)).children(crumbs(&mount, &path).into_iter().enumerate().map(|(index, (name, target))| {
                let last = target == path;
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .when(index > 0, |d| d.child(kit::icon_sized(Icon::ChevronRight, 12.).text_color(theme::color(ui::MUTED_FOREGROUND))))
                    .child(
                        div()
                            .id(("disk-crumb", index))
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            // 分区根前面是硬盘图标；「整个磁盘」是字不是路径，不用等宽字体
                            .when(index == 0, |d| d.child(kit::icon_sized(Icon::HardDrive, 14.).text_color(theme::color(ui::MUTED_FOREGROUND))))
                            .when(name.starts_with('/') || index > 0, |d| d.font_family(MONO))
                            .px(px(4.))
                            .rounded(px(5.))
                            .map(|d| {
                                if last {
                                    d.font_weight(FontWeight::SEMIBOLD)
                                } else {
                                    d.cursor_pointer()
                                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                                        .hover(|s| s.text_color(theme::color(ui::FOREGROUND)).bg(theme::alpha(ui::FOREGROUND, 0.06)))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go_to(target.clone(), cx)))
                                }
                            })
                            .child(div().min_w_0().child(name)),
                    )
            })))
            .when_some(usage.filter(|_| has_tree), |d, (used, total)| {
                let ratio = used as f32 / total.max(1) as f32;
                d.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_xs()
                        .font_features(TABULAR.clone())
                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                        .child(tr!("已用 {} / {}", format_bytes(used), format_bytes(total)))
                        .child(div().w(px(90.)).flex().child(bar(ratio, usage_color(ratio, ui::TEAL)))),
                )
            })
            .when(has_tree, |d| {
                d.child(
                    kit::button("disk-rescan", kit::Variant::Outline)
                        .h(px(28.))
                        .px_3()
                        .text_size(px(12.))
                        .aria_label(tr!("重新扫描"))
                        .child(div().flex().items_center().gap(px(6.)).child(kit::icon_sized(Icon::RefreshCw, 12.)).child(tr!("重新扫描")))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.start_scan(rescan_mount.clone(), cx))),
                )
            });

        let action = |id: &'static str, variant: kit::Variant, label: &'static str| {
            kit::button(id, variant).h(px(30.)).px_4().text_size(px(13.)).aria_label(label).child(label)
        };
        let stop = action("disk-stop", kit::Variant::Outline, tr!("停止")).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.stop_scans(cx)));
        let start = {
            let mount = mount.clone();
            move |id: &'static str, label: &'static str, cx: &mut Context<Self>| {
                action(id, kit::Variant::Primary, label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.start_scan(mount.clone(), cx)))
            }
        };
        let scan_folder = {
            let (key, path) = (key.clone(), path.clone());
            action("disk-scan-folder", kit::Variant::Primary, tr!("扫描这个目录"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.scan_deeper(key.clone(), path.clone(), cx)))
        };
        let muted = |text: &str| div().w_full().text_center().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(text.to_string());
        // 卡片下半截：这一步的说明和按钮
        let step = || div().w_full().flex().flex_col().items_center().gap(px(10.));
        let permission_hint = local.then(|| muted(tr!("第一次扫描时 macOS 可能会问 SSHL 能不能访问「文稿」「桌面」等文件夹；不允许的话这些文件夹算不进来。")));
        // 本机并行扫描有进度：已扫多少文件、多大（远程用 du，拿不到，只显示用了几秒）
        // 整个磁盘不写「/」（单独一个斜杠看着奇怪），别的分区、目录写路径
        let whole_disk = root_label(&mount) != mount;
        let scanning = |target: &str, (secs, files, bytes): (u64, u64, u64)| {
            step()
                .child(
                    div().flex().items_center().gap(px(8.)).child(spinner(16.)).child(div().text_sm().font_weight(FontWeight::MEDIUM).child(if whole_disk && target == mount {
                        tr!("正在扫描整个磁盘…已用 {} 秒", secs)
                    } else {
                        tr!("正在扫描 {}…已用 {} 秒", target, secs)
                    })),
                )
                .when(files > 0, |d| {
                    d.child(div().text_xs().font_features(TABULAR.clone()).text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("已扫 {} 个文件 · {}", group_digits(files), format_bytes(bytes))))
                })
        };
        let title = |text: String| div().text_sm().font_weight(FontWeight::MEDIUM).text_center().child(text);
        let body = match shown {
            Shown::Idle(stopped) => {
                let heading = if whole_disk { tr!("扫描整个磁盘，看看是哪些文件夹和文件占了空间").to_string() } else { tr!("扫描 {}，看看是哪些文件夹和文件占了空间", mount) };
                let (heading, hint) = if stopped {
                    (tr!("扫描已停止").to_string(), tr!("离开这一页时扫描会停下，不在后台一直占着资源。"))
                } else if local {
                    (heading, tr!("会读遍整个分区，分区越大越久。离开这一页就停下。"))
                } else {
                    (heading, tr!("会读遍整个分区，分区越大越久。在服务器上以最低的 CPU 和磁盘优先级跑，离开这一页就停下。"))
                };
                let content = step().child(title(heading)).child(muted(hint)).children(permission_hint).child(start("disk-start", tr!("开始扫描"), cx));
                self.stage(&mount, content)
            }
            Shown::Scanning(secs, files, bytes) => {
                let content = scanning(&mount, (secs, files, bytes)).child(muted(tr!("分区越大越久。扫完后往下点都是现成的，不用再等。"))).children(permission_hint).child(stop);
                self.stage(&mount, content)
            }
            Shown::Deeper(secs, files, bytes) => self.stage(&mount, scanning(&path, (secs, files, bytes)).child(stop)),
            Shown::Failed(error) => self.stage(&mount, step().child(title(tr!("扫描失败：{}", error.trim()))).child(start("disk-retry", tr!("重新扫描"), cx))),
            Shown::DeeperIdle(error) => {
                let heading = error.map_or_else(|| tr!("这个目录还没扫").to_string(), |error| tr!("扫描失败：{}", error.trim()));
                self.stage(&mount, step().child(title(heading)).child(scan_folder))
            }
            Shown::Done(at, unreadable) => self.render_tree(at, unreadable, local, cx),
        };
        div().flex_1().min_h_0().flex().flex_col().children(switcher).child(crumbs_bar).child(body).into_any_element()
    }

    /// 还没有扫描结果时（没扫、扫描中、停下了、失败了）中间那一块：底下铺一张很淡的示意矩形树图（扫完就是这样；不然一大片空白，
    /// 用户：「很空白，很难受」），正中一张卡片：这个分区（硬盘图标、名字、设备）、粗用量条和已用 / 可用 / 总共，下面是这一步的说明和按钮（`content`）。
    fn stage(&self, mount: &str, content: Div) -> AnyElement {
        let dark = usize::from(theme::is_dark());
        let fs = self.disk.filesystems.as_deref().and_then(|all| all.iter().find(|fs| fs.mount == mount)).cloned();
        let ghost = container_query(move |size, _, _| {
            let bounds = treemap::Rect { x: 0., y: 0., w: f32::from(size.width), h: f32::from(size.height) };
            div().size_full().relative().children(treemap::squarify(&GHOST, bounds).into_iter().map(move |rect| {
                div()
                    .absolute()
                    .left(px(rect.x + TILE_GAP / 2.))
                    .top(px(rect.y + TILE_GAP / 2.))
                    .w(px((rect.w - TILE_GAP).max(0.)))
                    .h(px((rect.h - TILE_GAP).max(0.)))
                    .rounded(px(8.))
                    .bg(theme::alpha(ui::FOREGROUND, GHOST_TINT[dark]))
            }))
        });
        let label = root_label(mount);
        // 「整个磁盘」下面写设备和挂载点；挂载点本身就是名字的，只写设备
        let device = fs.as_ref().map(|fs| if label == mount { fs.device.clone() } else { format!("{} · {}", fs.device, fs.mount) });
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .size(px(44.))
                    .flex_shrink_0()
                    .rounded(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::alpha(ui::TEAL, super::SEGMENT_TINT[dark] + 0.04))
                    .text_color(theme::color(ui::TEAL))
                    .child(kit::icon_sized(Icon::HardDrive, 22.)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(label))
                    .when_some(device, |d, device| d.child(div().font_family(MONO).text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(device))),
            );
        let usage = fs.map(|fs| {
            let ratio = fs.used as f32 / fs.total.max(1) as f32;
            let color = usage_color(ratio, ui::TEAL);
            let track = theme::alpha(color, RING_TRACK[dark]);
            let dot = |fill: Hsla| div().size(px(8.)).rounded_full().bg(fill);
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().h(px(10.)).w_full().rounded_full().bg(track).child(div().h_full().rounded_full().w(gpui::relative(ratio.clamp(0., 1.))).bg(theme::color(color))))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .text_xs()
                        .font_features(TABULAR.clone())
                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                        .child(div().flex().items_center().gap(px(6.)).child(dot(theme::color(color))).child(tr!("已用 {}", format_bytes(fs.used))))
                        .child(div().flex().items_center().gap(px(6.)).child(dot(track)).child(tr!("可用 {}", format_bytes(fs.available))))
                        .child(div().flex_1())
                        .child(tr!("总共 {}", format_bytes(fs.total))),
                )
        });
        let card = div()
            .w(px(STAGE_WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(20.))
            .rounded(px(14.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow_lg()
            .child(header)
            .children(usage)
            .child(div().w_full().h(px(1.)).bg(theme::color(ui::BORDER)))
            .child(content);
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(div().absolute().top_0().left_0().size_full().px(px(MAP_INSET)).pb(px(MAP_INSET)).child(ghost))
            .child(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().p_4().child(card))
            .into_any_element()
    }

    fn render_tree(&mut self, at: Instant, unreadable: usize, local: bool, cx: &mut Context<Self>) -> AnyElement {
        if self.listing(cx).is_none() {
            return empty_message(tr!("正在读取…"));
        }
        let mut note = scanned_ago(at.elapsed().as_secs());
        if unreadable > 0 {
            let reason = if local {
                tr!("有 {} 个目录没有权限读（系统保护的，或没允许 SSHL 访问的），实际占用比这里大。", unreadable)
            } else {
                tr!("有 {} 个目录没有权限读，实际占用比这里大。在终端里 sudo -i 之后重新扫描就能看全。", unreadable)
            };
            note = format!("{note} · {reason}");
        }
        // 下面的内容要看有多宽（够宽才在右边画矩形树图），布局排好之后才知道：交给 container_query
        let panel = cx.entity();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().flex_shrink_0().px_4().pb_2().text_xs().text_color(theme::color(ui::MUTED_FOREGROUND)).child(note))
            .child(div().flex_1().min_h_0().relative().child(
                container_query(move |size, window, cx| panel.update(cx, |this, cx| this.render_listing(size, window, cx))).absolute().top_0().left_0(),
            ))
            .into_any_element()
    }

    /// 列表（宽时右边再加矩形树图）。`size` 是这块地方实际有多大。
    fn render_listing(&mut self, size: gpui::Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some((rows, _)) = self.listing(cx) else { return empty_message(tr!("正在读取…")) };
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        let split = width >= SPLIT_MIN_WIDTH && !rows.is_empty();
        let header = table_header()
            .child(self.header_cell(tr!("名称"), None, None, Align::Start, cx))
            // 矩形树图已经画出了谁大谁小，列表里不再要用量条
            .when(!split, |d| d.child(self.header_cell(tr!("占用"), None, Some(USAGE_WIDTH), Align::Start, cx)))
            .child(self.header_cell(tr!("大小"), None, Some(SIZE_WIDTH), Align::End, cx))
            .child(self.header_cell(tr!("占比"), None, Some(SHARE_WIDTH), Align::End, cx));
        let body = if rows.is_empty() {
            let query = self.search.read(cx).value().trim().to_string();
            empty_message(if query.is_empty() { tr!("这个目录是空的") } else { tr!("没有匹配的结果") })
        } else {
            self.disk.rows.sync(rows.len());
            let entity = cx.entity();
            list(self.disk.rows.state(), move |ix, _, cx| {
                entity.update(cx, |this, cx| this.render_tree_rows(ix..ix + 1, !split, cx)).pop().unwrap_or_else(|| div().into_any_element())
            })
            .flex_1()
            .w_full()
            .into_any_element()
        };
        let list_width = if split { (width * LIST_SHARE).clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH) } else { width };
        let list = div().w(px(list_width)).h_full().flex_shrink_0().flex().flex_col().pb_2().child(header).child(body);
        if !split {
            return list.into_any_element();
        }
        let map = treemap::Rect { x: 0., y: 0., w: width - list_width - 2. * MAP_INSET, h: height - MAP_INSET };
        div()
            .size_full()
            .flex()
            .child(list)
            .child(div().pl(px(MAP_INSET)).child(self.render_treemap(&rows, map, window, cx)))
            .into_any_element()
    }

    /// 矩形树图：这一层每一项一块，面积就是占用（排法见 `treemap`）。目录蓝、文件紫、合起来的灰；块越大颜色越深。
    /// 点目录那块进去，右键同列表；鼠标停着时列表里那一行跟着亮。字整段写得下才写（`treemap::label`），写不下的悬停看名字和大小。
    fn render_treemap(&mut self, rows: &[Row], bounds: treemap::Rect, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(machine) = self.machine.clone() else { return div() };
        let folder = self.disk.at.get(&machine).map(|(_, path)| path.clone()).unwrap_or_default();
        let remote = matches!(machine, Machine::Remote(_));
        let total: u64 = rows.iter().map(Row::size).sum();
        let items = treemap::items(&rows.iter().map(|row| (row.size(), row.count())).collect::<Vec<_>>());
        let sizes: Vec<u64> = items.iter().map(|(_, size)| *size).collect();
        let rects = treemap::squarify(&sizes, bounds);
        let largest = sizes.first().copied().unwrap_or(0).max(1);
        // 鼠标停在列表里被合进「其余」的一行时，「其余」那块亮
        let hovered_rest = match self.disk.hovered {
            Some(Hover::Rest) => true,
            Some(Hover::Row(ix)) => rows.get(ix).is_some_and(|row| row.size() > 0) && !items.iter().any(|(item, _)| *item == Item::Row(ix)),
            None => false,
        };
        let dark = usize::from(theme::is_dark());
        let (tint_low, tint_high) = TILE_TINT[dark];
        // 比缝还窄的块看不见也点不中：不画（画出来的话内边距、边框会把它撑出边界）
        let visible = |rect: &treemap::Rect| rect.w - TILE_GAP >= 2. && rect.h - TILE_GAP >= 2.;
        // 量字用的字体：同画出来的（继承的界面字体，名字加粗一档，大小用等宽数字）
        let style = window.text_style();
        let name_font = Font { weight: FontWeight::MEDIUM, ..style.font() };
        let size_font = Font { features: TABULAR.clone(), ..style.font() };
        let run = |text: &str, font: &Font| TextRun { len: text.len(), font: font.clone(), color: style.color, background_color: None, underline: None, strikethrough: None };
        let text_system = window.text_system();
        let width = |text: &str, font: &Font, font_size: f32| f32::from(text_system.shape_line(text.to_string().into(), px(font_size), &[run(text, font)], None).width);
        let tiles = items.into_iter().zip(rects).enumerate().filter(|(_, (_, rect))| visible(rect)).map(|(n, ((item, size), rect))| {
            let row = match item {
                Item::Row(ix) => rows.get(ix),
                Item::Rest { .. } => None,
            };
            let (icon, name, color) = match (item, row) {
                (_, Some(Row::Dir(entry))) => (Some(Icon::Folder), entry.name.clone(), Some(ui::PRIMARY)),
                (_, Some(Row::File { name, .. })) => (Some(Icon::FileText), name.clone(), Some(ui::VIOLET)),
                (_, Some(Row::MoreFiles { count, .. })) => (None, tr!("其余 {} 个文件", group_digits(*count as u64)), None),
                (_, Some(Row::Files(_))) => (None, tr!("这一层的文件").to_string(), None),
                (Item::Rest { count }, None) => (None, tr!("其余 {} 项", group_digits(count as u64)), None),
                (Item::Row(_), None) => (None, String::new(), None),
            };
            let lit = match item {
                Item::Row(ix) => self.disk.hovered == Some(Hover::Row(ix)),
                Item::Rest { .. } => hovered_rest,
            };
            let background = match color {
                Some(color) => theme::alpha(color, tint_low + (tint_high - tint_low) * (size as f32 / largest as f32).sqrt() + if lit { 0.08 } else { 0. }),
                None => theme::alpha(ui::FOREGROUND, REST_TINT[dark] + if lit { 0.05 } else { 0. }),
            };
            let (w, h) = (rect.w - TILE_GAP, rect.h - TILE_GAP);
            let size_text = format_bytes(size);
            let lines = treemap::wrap_name(&name, treemap::text_width(w), |text| width(text, &name_font, treemap::NAME_FONT));
            let label = treemap::label(w, h, icon.is_some(), width(&name, &name_font, treemap::NAME_FONT), lines.len(), width(&size_text, &size_font, treemap::SIZE_FONT));
            let tip: SharedString = format!("{name} · {size_text} · {}", share(size, total)).into();
            let hover = match item {
                Item::Row(ix) => Hover::Row(ix),
                Item::Rest { .. } => Hover::Rest,
            };
            let tile = div()
                .id(("disk-tile", n))
                .absolute()
                .left(px(rect.x + TILE_GAP / 2.))
                .top(px(rect.y + TILE_GAP / 2.))
                .w(px(w.max(0.)))
                .h(px(h.max(0.)))
                .rounded(px(7_f32.min(w / 3.).min(h / 3.).max(0.)))
                .bg(background)
                .border_1()
                .border_color(if lit { theme::color(color.unwrap_or(ui::MUTED_FOREGROUND)) } else { gpui::transparent_black() })
                .overflow_hidden()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(treemap::LINE_GAP))
                .when(label.lines > 0, |d| d.px(px(treemap::PAD)))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.disk.hovered = Some(hover);
                    } else if this.disk.hovered == Some(hover) {
                        this.disk.hovered = None;
                    }
                    cx.notify();
                }))
                .tooltip(move |window, cx| kit::tooltip(tip.clone(), window, cx))
                // 名字：一行（前面可能带图标），或者在断点折好的几行，每行都不再换行
                .when(label.lines > 0, |d| {
                    let name_color = color.is_none().then(|| theme::color(ui::MUTED_FOREGROUND));
                    let text = |line: String| {
                        div()
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_size(px(treemap::NAME_FONT))
                            .line_height(px(treemap::NAME_LINE))
                            .font_weight(FontWeight::MEDIUM)
                            .when_some(name_color, |d, color| d.text_color(color))
                            .child(line)
                    };
                    if label.lines > 1 {
                        return d.child(div().flex().flex_col().items_center().children(lines.into_iter().map(text)));
                    }
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(treemap::ICON_GAP))
                            .when_some(icon.filter(|_| label.icon), |d, icon| {
                                d.child(div().flex_shrink_0().text_color(theme::color(readable(color.unwrap_or(ui::MUTED_FOREGROUND)))).child(kit::icon_sized(icon, treemap::ICON)))
                            })
                            .child(text(name.clone())),
                    )
                })
                .when(label.size, |d| {
                    d.child(
                        div()
                            .text_size(px(treemap::SIZE_FONT))
                            .line_height(px(treemap::SIZE_LINE))
                            .whitespace_nowrap()
                            .font_features(TABULAR.clone())
                            .text_color(theme::color(ui::MUTED_FOREGROUND))
                            .child(size_text),
                    )
                });
            let tile = match row {
                Some(Row::Dir(entry)) => {
                    let target = entry.path.clone();
                    tile.role(Role::Button)
                        .aria_label(SharedString::from(name.clone()))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go_to(target.clone(), cx)))
                }
                _ => tile,
            };
            let menu = match row {
                Some(Row::Dir(entry)) => Some((entry.path.clone(), entry.path.clone())),
                Some(Row::File { path, .. }) => Some((path.clone(), folder.clone())),
                _ => None,
            };
            match menu {
                Some((path, open)) => tile.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.open_menu(event.position, MenuTarget::Directory { path: path.clone(), folder: open.clone(), remote }, cx);
                    }),
                ),
                None => tile,
            }
        });
        div().relative().w(px(bounds.w)).h(px(bounds.h)).children(tiles)
    }

    /// `bars`：画用量条那一列（旁边没有矩形树图时）。
    fn render_tree_rows(&mut self, range: Range<usize>, bars: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(machine) = self.machine.clone() else { return Vec::new() };
        let Some((rows, total)) = self.listing(cx) else { return Vec::new() };
        let folder = self.disk.at.get(&machine).map(|(_, path)| path.clone()).unwrap_or_default();
        // 条的长度按这一层最大的那项算，看得出谁大；占比按这一层的总量
        let largest = rows.iter().map(Row::size).max().unwrap_or(0).max(1);
        let remote = matches!(machine, Machine::Remote(_));
        let lit = theme::alpha(ui::PRIMARY, ROW_HOVER[usize::from(theme::is_dark())]);
        let query = self.search.read(cx).value().to_string();
        range
            .filter_map(|ix| rows.get(ix).map(|row| (ix, row)))
            .map(|(ix, row)| {
                let size = row.size();
                // 鼠标停在矩形树图里它那一块上时这一行也亮
                let line = table_row(("disk-row", ix))
                    .when(self.disk.hovered == Some(Hover::Row(ix)), |d| d.bg(lit))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.disk.hovered = Some(Hover::Row(ix));
                        } else if this.disk.hovered == Some(Hover::Row(ix)) {
                            this.disk.hovered = None;
                        }
                        cx.notify();
                    }));
                let (icon, name, muted) = match row {
                    Row::Dir(entry) => (Icon::Folder, entry.name.clone(), false),
                    Row::File { name, .. } => (Icon::FileText, name.clone(), false),
                    Row::MoreFiles { count, .. } => (Icon::FileText, tr!("其余 {} 个文件", group_digits(*count as u64)), true),
                    Row::Files(_) => (Icon::FileText, tr!("这一层的文件").to_string(), true),
                };
                // 每个目录都能点进去：没有子目录也有文件要看（大日志常在最底下一层）
                let has_children = matches!(row, Row::Dir(_));
                let menu = match row {
                    Row::Dir(entry) => Some((entry.path.clone(), entry.path.clone())),
                    Row::File { path, .. } => Some((path.clone(), folder.clone())),
                    _ => None,
                };
                let line = match row {
                    Row::Dir(entry) => {
                        let target = entry.path.clone();
                        line.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go_to(target.clone(), cx)))
                    }
                    _ => line,
                };
                let line = match menu {
                    Some((path, open)) => line.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_menu(event.position, MenuTarget::Directory { path: path.clone(), folder: open.clone(), remote }, cx);
                        }),
                    ),
                    None => line,
                };
                let color = match row {
                    Row::Dir(_) => ui::PRIMARY,
                    Row::File { .. } => ui::VIOLET,
                    _ => ui::MUTED_FOREGROUND,
                };
                line.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .when(muted, |d| d.text_color(theme::color(ui::MUTED_FOREGROUND)))
                        .child(kit::icon_sized(icon, 14.).text_color(theme::color(if muted { ui::MUTED_FOREGROUND } else { color })))
                        .child(div().min_w_0().when(!muted, |d| d.font_weight(FontWeight::MEDIUM)).child(super::highlighted(name, &query)))
                        .when(has_children, |d| d.child(kit::icon_sized(Icon::ChevronRight, 12.).text_color(theme::color(ui::MUTED_FOREGROUND)))),
                )
                .when(bars, |d| d.child(usage_cell(size as f32 / largest as f32, color)))
                .child(size_cell(size))
                .child(share_cell(size, total))
                .into_any_element()
            })
            .collect()
    }
}

fn empty_message(message: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .text_sm()
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .child(message.into())
        .into_any_element()
}

fn size_cell(bytes: u64) -> Div {
    div().w(px(SIZE_WIDTH)).flex_shrink_0().px_3().flex().justify_end().font_features(TABULAR.clone()).child(format_bytes(bytes))
}

fn share_cell(size: u64, total: u64) -> Div {
    div()
        .w(px(SHARE_WIDTH))
        .flex_shrink_0()
        .px_3()
        .flex()
        .justify_end()
        .font_features(TABULAR.clone())
        .text_color(theme::color(ui::MUTED_FOREGROUND))
        .child(share(size, total))
}

fn usage_cell(ratio: f32, color: ui::Token) -> Div {
    div().w(px(USAGE_WIDTH)).flex_shrink_0().px_3().flex().items_center().child(bar(ratio, color))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, size: u64) -> DiskEntry {
        DiskEntry { name: name.into(), path: format!("/var/{name}"), size, has_children: true }
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Dir(entry) => format!("{}/", entry.name),
                Row::File { name, .. } => name.clone(),
                Row::MoreFiles { count, size } => format!("+{count}={size}"),
                Row::Files(size) => format!("files={size}"),
            })
            .collect()
    }

    /// 子目录和文件放在一起从大到小：一个 8G 的日志排在小目录前面，一眼看得到。
    #[test]
    fn folders_and_files_are_ranked_together() {
        let files = [("huge.log".to_string(), 8000), ("small.txt".to_string(), 5)];
        let rows = rows(vec![dir("lib", 9000), dir("cache", 100)], Some(&files), 8005, "/var", "");
        assert_eq!(names(&rows), ["lib/", "huge.log", "cache/", "small.txt"]);
        assert_eq!(rows[1], Row::File { name: "huge.log".into(), path: "/var/huge.log".into(), size: 8000 });
        // 根目录下的文件路径
        let rows = super::rows(Vec::new(), Some(&files), 0, "/", "");
        assert_eq!(rows[0], Row::File { name: "huge.log".into(), path: "/huge.log".into(), size: 8000 });
    }

    #[test]
    fn many_small_files_are_folded_into_one_line() {
        let files: Vec<(String, u64)> = (0..250).map(|i| (format!("f{i:03}"), 1000 - i)).collect();
        let rows = rows(Vec::new(), Some(&files), 0, "/tmp", "");
        assert_eq!(rows.len(), FILES_SHOWN + 1);
        let rest: u64 = (200..250).map(|i| 1000 - i).sum();
        assert_eq!(rows.last(), Some(&Row::MoreFiles { count: 50, size: rest }), "最大的 200 个之外的合成一行，放在最后");
    }

    #[test]
    fn before_files_are_listed_their_total_stands_in() {
        let rows = rows(vec![dir("lib", 10)], None, 500, "/var", "");
        assert_eq!(names(&rows), ["lib/", "files=500"]);
        assert_eq!(names(&super::rows(vec![dir("lib", 10)], None, 0, "/var", "")), ["lib/"], "没有文件就不要这一行");
    }

    #[test]
    fn search_filters_folders_and_files_by_name() {
        let files = [("Docker.log".to_string(), 50), ("other".to_string(), 7)];
        let rows = rows(vec![dir("docker", 10), dir("log", 99)], Some(&files), 57, "/var", "dock");
        assert_eq!(names(&rows), ["Docker.log", "docker/"]);
        assert_eq!(names(&super::rows(vec![dir("lib", 1)], None, 500, "/var", "x")), Vec::<String>::new(), "搜索时不显示文件总和");
    }

    #[test]
    fn the_root_volume_is_opened_first() {
        let fs = |mount: &str, used: u64| Filesystem { device: "/dev/x".into(), mount: mount.into(), total: 100, used, available: 100 - used };
        assert_eq!(default_volume(&[fs("/boot", 50), fs("/", 10)]).as_deref(), Some("/"));
        assert_eq!(default_volume(&[fs("/Volumes/USB", 5), fs("/System/Volumes/Data", 60)]).as_deref(), Some("/System/Volumes/Data"), "没有 / 用用得最多的");
        assert_eq!(default_volume(&[]), None);
    }

    #[test]
    fn crumbs_walk_from_the_mount_point_down() {
        let names = |root: &str, path: &str| crumbs(root, path).into_iter().map(|(name, path)| format!("{name}={path}")).collect::<Vec<_>>();
        assert_eq!(names("/", "/"), ["整个磁盘=/"], "单独一个「/」看着奇怪");
        assert_eq!(names("/", "/var/lib/docker"), ["整个磁盘=/", "var=/var", "lib=/var/lib", "docker=/var/lib/docker"]);
        assert_eq!(names("/System/Volumes/Data", "/System/Volumes/Data/Users"), ["整个磁盘=/System/Volumes/Data", "Users=/System/Volumes/Data/Users"], "Mac 的数据卷");
        assert_eq!(names("/data", "/data/backups/2026"), ["/data=/data", "backups=/data/backups", "2026=/data/backups/2026"]);
    }

    #[test]
    fn shares_read_well() {
        assert_eq!(share(50, 100), "50%");
        assert_eq!(share(5, 100), "5.0%");
        assert_eq!(share(1, 10_000), "<0.1%");
        assert_eq!(share(0, 100), "0%");
        assert_eq!(share(10, 0), "0%", "空目录不除以 0");
    }

    #[test]
    fn scan_age_is_rounded_for_people() {
        assert_eq!(scanned_ago(5), "刚刚扫描");
        assert_eq!(scanned_ago(150), "2 分钟前扫描");
        assert_eq!(scanned_ago(7300), "2 小时前扫描");
    }

    #[test]
    fn big_counts_get_thousands_separators() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(644498), "644,498");
        assert_eq!(group_digits(4544185), "4,544,185");
    }

    fn progress() -> Arc<ScanProgress> {
        Arc::new(ScanProgress::default())
    }

    fn done(deeper: Option<Arc<ScanProgress>>) -> Scan {
        let tree = disk_usage::parse_du("5\t/a\n10\t/\n", "/", 0, 4);
        Scan::Done { tree: Arc::new(tree), at: Instant::now(), deeper: deeper.map(|p| ("/a/b/c/d/e".to_string(), Instant::now(), p)), failed: None }
    }

    /// 离开磁盘页：所有正在跑的扫描都叫停（远程据此杀掉 du）。整个分区的变「已停止」，补扫的回到「还没扫」；扫完的结果留着。
    #[test]
    fn leaving_stops_every_running_scan() {
        let (full, deeper) = (progress(), progress());
        let mut disk = DiskState::default();
        let remote = Machine::Remote("s1".into());
        disk.scans.insert((Machine::Local, "/".into()), Scan::Running { started: Instant::now(), progress: full.clone() });
        disk.scans.insert((remote.clone(), "/".into()), done(Some(deeper.clone())));
        disk.scans.insert((remote.clone(), "/data".into()), done(None));
        assert!(disk.stop_all());
        assert!(full.stopped() && deeper.stopped());
        assert!(matches!(disk.scans[&(Machine::Local, "/".into())], Scan::Stopped));
        assert!(matches!(&disk.scans[&(remote.clone(), "/".into())], Scan::Done { deeper: None, .. }), "补扫停了，树还在");
        assert!(matches!(&disk.scans[&(remote, "/data".into())], Scan::Done { .. }));
        let version = disk.version;
        assert!(!disk.stop_all(), "没有在跑的了");
        assert_eq!(disk.version, version, "什么都没停就不用重画");
    }

    /// 停下的扫描晚一步回来的结果不能盖掉后来的状态（「已停止」、或者用户又点了一次的新扫描）。
    #[test]
    fn a_stopped_scan_never_overwrites_what_came_after() {
        let key = (Machine::Local, "/".to_string());
        let first = progress();
        let mut disk = DiskState::default();
        disk.scans.insert(key.clone(), Scan::Running { started: Instant::now(), progress: first.clone() });
        disk.stop_all();
        assert!(!disk.finish_scan(&key, &first, Scan::Failed("cancelled".into())));
        assert!(matches!(disk.scans[&key], Scan::Stopped), "停下的还是停下的，不变成失败、也不自己重来");

        let second = progress();
        disk.scans.insert(key.clone(), Scan::Running { started: Instant::now(), progress: second.clone() });
        assert!(!disk.finish_scan(&key, &first, done(None)), "旧的那次的结果不要");
        assert!(matches!(disk.scans[&key], Scan::Running { .. }));
        assert!(disk.finish_scan(&key, &second, done(None)));
        assert!(matches!(disk.scans[&key], Scan::Done { .. }));

        // 补扫一样：停下以后回来的结果不接到树上
        let deeper = progress();
        disk.scans.insert(key.clone(), done(Some(deeper.clone())));
        disk.stop_all();
        let sub = disk_usage::parse_du("1\t/a/b/c/d/e/f\n2\t/a/b/c/d/e\n", "/a/b/c/d/e", 0, 4);
        assert!(!disk.finish_deeper(&key, &deeper, Ok(sub.clone())));
        let again = progress();
        let Some(Scan::Done { deeper: slot, .. }) = disk.scans.get_mut(&key) else { unreachable!() };
        *slot = Some(("/a/b/c/d/e".into(), Instant::now(), again.clone()));
        assert!(disk.finish_deeper(&key, &again, Ok(sub)));
        let Some(Scan::Done { tree, deeper, .. }) = disk.scans.get(&key) else { unreachable!() };
        assert!(deeper.is_none() && tree.size("/a/b/c/d/e/f").is_some());
    }

    /// 在深处点了「重新扫描」：新树只有前 4 层，退到树上最深的祖先，不退出分区。
    #[test]
    fn a_rescan_falls_back_to_the_deepest_known_folder() {
        let tree = disk_usage::parse_du("10\t/data/a/b/c/d\n20\t/data/a/b/c\n30\t/data/a/b\n40\t/data/a\n50\t/data\n", "/data", 0, 4);
        assert_eq!(deepest_known(&tree, "/data", "/data/a/b/c/d/e/f"), "/data/a/b/c/d");
        assert_eq!(deepest_known(&tree, "/data", "/data/a/b"), "/data/a/b", "在树上的不动");
        assert_eq!(deepest_known(&tree, "/data", "/data/gone/x"), "/data");
        let root = disk_usage::parse_du("5\t/\n", "/", 0, 4);
        assert_eq!(deepest_known(&root, "/", "/var/lib"), "/");
    }
}

//! 本机的磁盘扫描：多线程并行走目录（同 gdu / dua 的做法），比单线程的 du 快几倍。远程服务器上跑不了我们的代码，还是 du。
//! 结果和 `du -xk -d <depth>` 一模一样：占用按实际占的块算（st_blocks），不跨分区，不跟随符号链接，硬链接只算一次，
//! 只列到第 depth 层（更深的算进第 depth 层那个目录）。
//!
//! macOS 上列目录用 `getattrlistbulk`（同 Mole 的 Mac 版）：一次系统调用拿回一批目录项的名字、类型、设备、inode、链接数和占用，
//! 不用每个文件再单独 lstat 一次（`read_dir` + `metadata` 是两次系统调用 / 项）。别的系统用 `read_dir`。

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::disk_usage::{DiskTree, normalize};
use crate::error::{AppError, AppResult};

/// 扫描进度（界面每秒读一次）和停止开关。
#[derive(Debug, Default)]
pub struct ScanProgress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    stopped: AtomicBool,
}

impl ScanProgress {
    /// 叫停：本机的扫描线程拿下一个目录前就收手，远程的 du 被杀掉（`disk_usage::scan`），扫描返回 `Cancelled`。
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

/// 要走的一个目录。`node` 是它的大小记在谁名下：第 depth 层以内是它自己，更深的记在第 depth 层的祖先上。
struct Task {
    path: PathBuf,
    level: usize,
    node: Arc<str>,
}

/// 共享的任务队列：(待走的目录, 正在走的个数)。都走完、也没人在走时结束。
struct Queue {
    state: Mutex<(Vec<Task>, usize)>,
    ready: Condvar,
}

impl Queue {
    /// 后进先出：接近深度优先，排着的目录不会堆太多。叫停了就不再给（等着的线程在别人走完一个目录时被叫醒，也看得到）。
    fn pop(&self, progress: &ScanProgress) -> Option<Task> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if progress.stopped() {
                return None;
            }
            if let Some(task) = state.0.pop() {
                state.1 += 1;
                return Some(task);
            }
            if state.1 == 0 {
                self.ready.notify_all();
                return None;
            }
            state = self.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// 一个目录走完：它下面的子目录一次放进去（一个目录只拿一次锁）。
    fn finish(&self, found: Vec<Task>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let woke = !found.is_empty();
        state.0.extend(found);
        state.1 -= 1;
        let idle = state.0.is_empty() && state.1 == 0;
        drop(state);
        if woke || idle {
            self.ready.notify_all();
        }
    }
}

type Lister = fn(&Path, &ScanProgress) -> io::Result<Vec<Item>>;

/// 并行扫 `root`（阻塞，放在后台线程里调）。中途叫停（[`ScanProgress::stop`]）返回 `Cancelled`。
pub fn scan_parallel(root: &str, depth: usize, progress: &ScanProgress) -> AppResult<DiskTree> {
    scan_with(root, depth, progress, list)
}

fn scan_with(root: &str, depth: usize, progress: &ScanProgress, list: Lister) -> AppResult<DiskTree> {
    let root = normalize(root);
    let meta = std::fs::symlink_metadata(&root)?;
    let device = meta.dev();
    // 核都用上：线程是 utility 优先级（见 `yield_to_interactive_work`），前台有事时系统自己会让
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 12);
    let start = Task { path: PathBuf::from(&root), level: 0, node: Arc::from(root.as_str()) };
    let queue = Queue { state: Mutex::new((vec![start], 0)), ready: Condvar::new() };
    let hardlinks = Mutex::new(HashSet::new());
    let unreadable = AtomicU64::new(0);
    let shared = Shared { queue: &queue, list, device, depth, hardlinks: &hardlinks, unreadable: &unreadable, progress };
    let parts: Vec<HashMap<Arc<str>, u64>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads).map(|_| scope.spawn(|| work(&shared))).collect();
        workers.into_iter().map(|worker| worker.join().unwrap_or_default()).collect()
    });
    if progress.stopped() {
        return Err(AppError::Cancelled);
    }
    let mut own: HashMap<String, u64> = HashMap::new();
    for part in parts {
        for (node, size) in part {
            *own.entry(node.to_string()).or_default() += size;
        }
    }
    *own.entry(root.clone()).or_default() += meta.blocks() * 512;
    Ok(DiskTree::from_own_sizes(root, own, depth, unreadable.load(Ordering::Relaxed) as usize))
}

struct Shared<'a> {
    queue: &'a Queue,
    /// 怎么列一个目录（测试里两种列法都跑一遍比结果）。
    list: Lister,
    device: u64,
    depth: usize,
    hardlinks: &'a Mutex<HashSet<(u64, u64)>>,
    unreadable: &'a AtomicU64,
    progress: &'a ScanProgress,
}

/// 扫描线程降到 utility 优先级：SSHL 的界面（主线程是 user-interactive）和别的程序的前台工作都排在它前面，
/// 机器空闲时照样全速跑。扫描时占满几个核也不会让界面、编辑器、浏览器变卡。
fn yield_to_interactive_work() {
    #[cfg(target_os = "macos")]
    // SAFETY: 只改当前线程自己的 QoS，参数是合法的枚举值
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

/// 目录里的一项：扫描只要这些。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    name: OsString,
    dir: bool,
    /// 实际占的字节（块数 × 512）。
    bytes: u64,
    dev: u64,
    ino: u64,
    nlink: u64,
}

/// 列一个目录（不跟随符号链接：链接本身算一项）。中途叫停了就少列一些（调用的人照样会收手）。
#[cfg(target_os = "macos")]
fn list(dir: &Path, progress: &ScanProgress) -> io::Result<Vec<Item>> {
    bulk::list(dir, progress)
}

#[cfg(not(target_os = "macos"))]
fn list(dir: &Path, progress: &ScanProgress) -> io::Result<Vec<Item>> {
    list_each(dir, progress)
}

/// `read_dir` + 每项一个 lstat。
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn list_each(dir: &Path, progress: &ScanProgress) -> io::Result<Vec<Item>> {
    let mut items = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        // 一个目录里几百万个文件也不用等它列完才停
        if progress.stopped() {
            break;
        }
        let Ok(meta) = entry.metadata() else { continue };
        // 目录的链接数用不到（只有文件的硬链接要查重），同批量列法记成 1
        let nlink = if meta.is_dir() { 1 } else { meta.nlink() };
        items.push(Item { name: entry.file_name(), dir: meta.is_dir(), bytes: meta.blocks() * 512, dev: meta.dev(), ino: meta.ino(), nlink });
    }
    Ok(items)
}

/// macOS：`getattrlistbulk` 一次拿回一批目录项和要的属性（格式见 getattrlist(2)）。
#[cfg(target_os = "macos")]
mod bulk {
    use std::ffi::{CString, OsString};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;

    use super::{Item, ScanProgress};

    /// vnode 类型里的目录（sys/vnode.h 的 VDIR）。
    const VDIR: u32 = 2;
    /// 一次取多少字节的目录项：一项一百字节上下，一次一千多项。
    const BUFFER: usize = 128 * 1024;

    pub(super) fn list(dir: &Path, progress: &ScanProgress) -> io::Result<Vec<Item>> {
        let path = CString::new(dir.as_os_str().as_bytes())?;
        // SAFETY: path 是以 NUL 结尾的路径；只读打开目录
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 刚打开的、只归这里的描述符
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut request = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_NAME | libc::ATTR_CMN_DEVID | libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_FILEID,
            volattr: 0,
            dirattr: libc::ATTR_DIR_ALLOCSIZE,
            fileattr: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE,
            forkattr: 0,
        };
        let mut buffer = vec![0u8; BUFFER];
        let mut items = Vec::new();
        while !progress.stopped() {
            // SAFETY: request、buffer 都活着且大小如实给出；内核往 buffer 里最多写 BUFFER 字节
            let count = unsafe {
                libc::getattrlistbulk(fd.as_raw_fd(), (&raw mut request).cast(), buffer.as_mut_ptr().cast(), buffer.len(), 0)
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                break;
            }
            let mut at = 0;
            for _ in 0..count {
                let Some(length) = read_u32(&buffer, at).map(|length| length as usize).filter(|length| *length > 0) else { break };
                if let Some(item) = parse(buffer.get(at..at + length).unwrap_or_default()) {
                    items.push(item);
                }
                at += length;
            }
        }
        Ok(items)
    }

    /// 一项：长度、实际返回了哪些属性（attribute_set_t），再按位从小到大排着各个属性；每个属性 4 字节对齐（8 字节的也只按 4 对齐）。
    fn parse(entry: &[u8]) -> Option<Item> {
        let returned = |group: usize| read_u32(entry, 4 + group * 4).unwrap_or(0);
        let (common, dirs, files) = (returned(0), returned(2), returned(3));
        let mut at = 4 + 20;
        let mut name = None;
        if common & libc::ATTR_CMN_NAME != 0 {
            // attrreference_t：数据相对这个结构自己的偏移、长度（含结尾的 NUL）
            let offset = i32::from_ne_bytes(entry.get(at..at + 4)?.try_into().ok()?);
            let length = read_u32(entry, at + 4)? as usize;
            let start = usize::try_from(at as i64 + i64::from(offset)).ok()?;
            let bytes = entry.get(start..start + length.saturating_sub(1))?;
            name = Some(OsString::from_vec(bytes.to_vec()));
            at += 8;
        }
        let mut dev = 0;
        if common & libc::ATTR_CMN_DEVID != 0 {
            // dev_t 是 i32：同 std 的 MetadataExt::dev（有符号扩展）
            dev = i32::from_ne_bytes(entry.get(at..at + 4)?.try_into().ok()?) as u64;
            at += 4;
        }
        let mut kind = 0;
        if common & libc::ATTR_CMN_OBJTYPE != 0 {
            kind = read_u32(entry, at)?;
            at += 4;
        }
        let mut ino = 0;
        if common & libc::ATTR_CMN_FILEID != 0 {
            ino = read_u64(entry, at)?;
            at += 8;
        }
        let dir = kind == VDIR;
        let (mut nlink, mut bytes) = (1, 0);
        if dir {
            if dirs & libc::ATTR_DIR_ALLOCSIZE != 0 {
                bytes = read_u64(entry, at)?;
            }
        } else {
            if files & libc::ATTR_FILE_LINKCOUNT != 0 {
                nlink = u64::from(read_u32(entry, at)?);
                at += 4;
            }
            if files & libc::ATTR_FILE_ALLOCSIZE != 0 {
                bytes = read_u64(entry, at)?;
            }
        }
        Some(Item { name: name?, dir, bytes, dev, ino, nlink })
    }

    fn read_u32(buffer: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_ne_bytes(buffer.get(at..at + 4)?.try_into().ok()?))
    }

    fn read_u64(buffer: &[u8], at: usize) -> Option<u64> {
        Some(u64::from_ne_bytes(buffer.get(at..at + 8)?.try_into().ok()?))
    }
}

/// 一个线程：拿目录、列出来、文件的块数记账、子目录放回队列。账先记在自己这份表里，最后再合（不抢锁）。
fn work(shared: &Shared) -> HashMap<Arc<str>, u64> {
    yield_to_interactive_work();
    let mut sizes: HashMap<Arc<str>, u64> = HashMap::new();
    while let Some(task) = shared.queue.pop(shared.progress) {
        let (mut here, mut files) = (0u64, 0u64);
        let mut found = Vec::new();
        match (shared.list)(&task.path, shared.progress) {
            Err(_) => {
                shared.unreadable.fetch_add(1, Ordering::Relaxed);
            }
            Ok(items) => {
                for item in items {
                    // 别的分区挂在这里（du -x）：整个不算
                    if item.dev != shared.device {
                        continue;
                    }
                    if item.dir {
                        let level = task.level + 1;
                        let path = task.path.join(&item.name);
                        let node: Arc<str> = if level <= shared.depth { Arc::from(path.to_string_lossy().as_ref()) } else { task.node.clone() };
                        // 目录本身占的块记在它名下（也让空目录、读不了的目录出现在树上）
                        *sizes.entry(node.clone()).or_default() += item.bytes;
                        found.push(Task { path, level, node });
                    } else {
                        // 硬链接只算一次：只有链接数大于 1 的才去查（少见，锁不常拿）
                        if item.nlink > 1 && !shared.hardlinks.lock().unwrap_or_else(|e| e.into_inner()).insert((item.dev, item.ino)) {
                            continue;
                        }
                        here += item.bytes;
                        files += 1;
                    }
                }
            }
        }
        *sizes.entry(task.node.clone()).or_default() += here;
        shared.progress.files.fetch_add(files, Ordering::Relaxed);
        shared.progress.bytes.fetch_add(here, Ordering::Relaxed);
        shared.queue.finish(found);
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_usage::parse_du;

    fn temp_dir(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sshl-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn write(path: &std::path::Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; len]).unwrap();
    }

    /// 和 `du -xk -d 4` 逐个目录比：大小、谁在边上、这一层的文件都一样。树里有比 4 层深的、硬链接、指向大文件的符号链接、空目录。
    #[test]
    fn matches_du_exactly() {
        let root = temp_dir("scan");
        write(&root.join("top.bin"), 70_000);
        write(&root.join("a/one.bin"), 5_000);
        write(&root.join("a/b/c/d/e/f/deep.bin"), 300_000);
        write(&root.join("a/b/c/d/side.bin"), 9_000);
        write(&root.join("big/blob.bin"), 1_000_000);
        // 同一个目录里的硬链接：只算一次（放在不同目录的话，算在谁名下取决于谁先走到，du 自己也不定）
        std::fs::hard_link(root.join("big/blob.bin"), root.join("big/same-blob.bin")).unwrap();
        std::os::unix::fs::symlink(root.join("big/blob.bin"), root.join("a/b/link-to-blob")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        for i in 0..200 {
            write(&root.join(format!("many/{i:03}/f.txt")), 100 + i);
        }
        let path = root.to_string_lossy().into_owned();

        let du = std::process::Command::new("du").args(["-xk", "-d", "4", "--", &path]).output().unwrap();
        let expected = parse_du(&String::from_utf8_lossy(&du.stdout), &path, 0, 4);
        let progress = ScanProgress::default();
        let tree = scan_parallel(&path, 4, &progress).unwrap();

        let mut stack = vec![path.clone()];
        let mut seen = 0;
        while let Some(dir) = stack.pop() {
            seen += 1;
            assert_eq!(tree.size(&dir), expected.size(&dir), "{dir}");
            assert_eq!(tree.needs_scan(&dir), expected.needs_scan(&dir), "{dir}");
            assert_eq!(tree.files_size(&dir), expected.files_size(&dir), "{dir}");
            let (got, want) = (tree.entries(&dir), expected.entries(&dir));
            assert_eq!(got, want, "{dir} 下面的目录（顺序也要一样）");
            stack.extend(want.into_iter().map(|entry| entry.path));
        }
        assert!(seen > 200, "整棵树都比过了：{seen}");
        assert!(progress.files.load(Ordering::Relaxed) >= 205, "进度数到了文件");
        assert_eq!(tree.unreadable, 0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 两种列法（macOS 的批量、通用的逐个 lstat）列出来的一模一样：普通文件、空文件、子目录、符号链接、硬链接、中文名，
    /// 加上 3000 个文件（批量的一次取不完，要取好几次）。
    #[test]
    fn both_ways_of_listing_agree() {
        let root = temp_dir("list");
        write(&root.join("data.bin"), 123_456);
        write(&root.join("空的"), 0);
        write(&root.join("中文 名字.txt"), 5_000);
        std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
        std::fs::hard_link(root.join("data.bin"), root.join("same.bin")).unwrap();
        std::os::unix::fs::symlink(root.join("data.bin"), root.join("link")).unwrap();
        std::os::unix::fs::symlink("/nowhere", root.join("dangling")).unwrap();
        for i in 0..3000 {
            write(&root.join(format!("many-{i:04}-with-a-longer-name-to-fill-the-buffer.txt")), 10 + i % 7000);
        }
        let progress = ScanProgress::default();
        let sorted = |mut items: Vec<Item>| {
            items.sort_by(|a, b| a.name.cmp(&b.name));
            items
        };
        let each = sorted(list_each(&root, &progress).unwrap());
        let listed = sorted(list(&root, &progress).unwrap());
        assert_eq!(each.len(), 3007);
        assert_eq!(listed.len(), each.len());
        for (a, b) in listed.iter().zip(&each) {
            assert_eq!(a, b);
        }
        let data = each.iter().find(|item| item.name == "data.bin").unwrap();
        assert!(data.nlink == 2 && data.bytes >= 123_456 && !data.dir);
        assert!(each.iter().find(|item| item.name == "sub").unwrap().dir);
        assert!(!each.iter().find(|item| item.name == "link").unwrap().dir, "符号链接不跟过去");
        assert!(list(&root.join("nope"), &progress).is_err(), "读不了的报错（算进读不了的目录里）");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unreadable_folders_are_counted_and_the_rest_still_scanned() {
        let root = temp_dir("scan-locked");
        write(&root.join("open/f.bin"), 10_000);
        std::fs::create_dir_all(root.join("locked/inner")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let tree = scan_parallel(&root.to_string_lossy(), 4, &ScanProgress::default()).unwrap();
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(tree.unreadable, 1);
        let names: Vec<String> = tree.entries(&root.to_string_lossy()).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["open", "locked"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 叫停：扫描线程收手（不会有线程卡在等任务上），返回 Cancelled；开始前就叫停的一个目录都不走。
    #[test]
    fn a_stopped_scan_ends_and_reports_cancelled() {
        let root = temp_dir("scan-stop");
        for i in 0..300 {
            write(&root.join(format!("d{i:03}/e/f.bin")), 10);
        }
        let path = root.to_string_lossy().into_owned();
        let progress = Arc::new(ScanProgress::default());
        progress.stop();
        assert!(matches!(scan_parallel(&path, 4, &progress), Err(AppError::Cancelled)));
        assert_eq!(progress.files.load(Ordering::Relaxed), 0, "一个文件都没数");

        // 扫到一半叫停：另一个线程在扫描进行中按停止，扫描照样结束（没有线程一直等着）
        let progress = Arc::new(ScanProgress::default());
        let stopper = {
            let progress = progress.clone();
            std::thread::spawn(move || {
                while progress.files.load(Ordering::Relaxed) == 0 {
                    std::thread::yield_now();
                }
                progress.stop();
            })
        };
        let result = scan_parallel(&path, 4, &progress);
        stopper.join().unwrap();
        assert!(matches!(result, Err(AppError::Cancelled)), "{:?}", result.map(|tree| tree.size(&path)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 实测速度：两种列法交替各扫两遍（第一遍可能吃冷缓存）。
    /// `SSHL_SCAN_BENCH=$HOME/Library cargo test --release -p sshl-core bench_parallel_scan -- --ignored --nocapture`
    #[test]
    #[ignore = "要 SSHL_SCAN_BENCH 指向一个大目录"]
    fn bench_parallel_scan() {
        let Ok(path) = std::env::var("SSHL_SCAN_BENCH") else { return };
        for round in 1..=2 {
            for (name, lister) in [("read_dir + lstat", list_each as Lister), ("getattrlistbulk", list as Lister)] {
                let started = std::time::Instant::now();
                let progress = ScanProgress::default();
                let tree = scan_with(&path, 4, &progress, lister).unwrap();
                println!(
                    "round {round} {name}: {:.2}s, {} files, {} MB, {} unreadable",
                    started.elapsed().as_secs_f64(),
                    progress.files.load(Ordering::Relaxed),
                    tree.size(&path).unwrap_or(0) / 1_000_000,
                    tree.unreadable,
                );
            }
        }
    }
}

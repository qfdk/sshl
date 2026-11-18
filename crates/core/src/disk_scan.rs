//! 本机的磁盘扫描：多线程并行走目录（同 gdu / dua 的做法），比单线程的 du 快几倍。远程服务器上跑不了我们的代码，还是 du。
//! 结果和 `du -xk -d <depth>` 一模一样：占用按实际占的块算（st_blocks），不跨分区，不跟随符号链接，硬链接只算一次，
//! 只列到第 depth 层（更深的算进第 depth 层那个目录）。

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::disk_usage::{DiskTree, normalize};
use crate::error::AppResult;

/// 扫描进度：界面每秒读一次。
#[derive(Debug, Default)]
pub struct ScanProgress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
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
    /// 后进先出：接近深度优先，排着的目录不会堆太多。
    fn pop(&self) -> Option<Task> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
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

/// 并行扫 `root`（阻塞，放在后台线程里调）。
pub fn scan_parallel(root: &str, depth: usize, progress: &ScanProgress) -> AppResult<DiskTree> {
    let root = normalize(root);
    let meta = std::fs::symlink_metadata(&root)?;
    let device = meta.dev();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 12);
    let start = Task { path: PathBuf::from(&root), level: 0, node: Arc::from(root.as_str()) };
    let queue = Queue { state: Mutex::new((vec![start], 0)), ready: Condvar::new() };
    let hardlinks = Mutex::new(HashSet::new());
    let unreadable = AtomicU64::new(0);
    let shared = Shared { queue: &queue, device, depth, hardlinks: &hardlinks, unreadable: &unreadable, progress };
    let parts: Vec<HashMap<Arc<str>, u64>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads).map(|_| scope.spawn(|| work(&shared))).collect();
        workers.into_iter().map(|worker| worker.join().unwrap_or_default()).collect()
    });
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
    device: u64,
    depth: usize,
    hardlinks: &'a Mutex<HashSet<(u64, u64)>>,
    unreadable: &'a AtomicU64,
    progress: &'a ScanProgress,
}

/// 一个线程：拿目录、列出来、文件的块数记账、子目录放回队列。账先记在自己这份表里，最后再合（不抢锁）。
fn work(shared: &Shared) -> HashMap<Arc<str>, u64> {
    let mut sizes: HashMap<Arc<str>, u64> = HashMap::new();
    while let Some(task) = shared.queue.pop() {
        let (mut here, mut files) = (0u64, 0u64);
        let mut found = Vec::new();
        match std::fs::read_dir(&task.path) {
            Err(_) => {
                shared.unreadable.fetch_add(1, Ordering::Relaxed);
            }
            Ok(entries) => {
                for entry in entries.flatten() {
                    // 不跟随符号链接（同 du）
                    let Ok(meta) = entry.metadata() else { continue };
                    // 别的分区挂在这里（du -x）：整个不算
                    if meta.dev() != shared.device {
                        continue;
                    }
                    let size = meta.blocks() * 512;
                    if meta.is_dir() {
                        let level = task.level + 1;
                        let path = entry.path();
                        let node: Arc<str> = if level <= shared.depth { Arc::from(path.to_string_lossy().as_ref()) } else { task.node.clone() };
                        // 目录本身占的块记在它名下（也让空目录、读不了的目录出现在树上）
                        *sizes.entry(node.clone()).or_default() += size;
                        found.push(Task { path, level, node });
                    } else {
                        // 硬链接只算一次：只有链接数大于 1 的才去查（少见，锁不常拿）
                        if meta.nlink() > 1 && !shared.hardlinks.lock().unwrap_or_else(|e| e.into_inner()).insert((meta.dev(), meta.ino())) {
                            continue;
                        }
                        here += size;
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

    /// 实测速度：`SSHL_SCAN_BENCH=/System/Volumes/Data cargo test --release -p sshl-core bench_parallel_scan -- --ignored --nocapture`
    #[test]
    #[ignore = "要 SSHL_SCAN_BENCH 指向一个大目录"]
    fn bench_parallel_scan() {
        let Ok(path) = std::env::var("SSHL_SCAN_BENCH") else { return };
        let started = std::time::Instant::now();
        let progress = ScanProgress::default();
        let tree = scan_parallel(&path, 4, &progress).unwrap();
        println!(
            "parallel: {:.1}s, {} files, {} GB, {} unreadable, root {} GB",
            started.elapsed().as_secs_f64(),
            progress.files.load(Ordering::Relaxed),
            progress.bytes.load(Ordering::Relaxed) / 1_000_000_000,
            tree.unreadable,
            tree.size(&path).unwrap_or(0) / 1_000_000_000
        );
    }
}

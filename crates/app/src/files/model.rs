//! 文件条目、路径工具和显示格式（旧 useFileList.ts 的纯函数部分）。

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

use sshl_core::local_fs::LocalEntry;
use sshl_core::sftp::RemoteEntry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaneKind {
    Local,
    Remote,
}

impl PaneKind {
    pub fn other(self) -> Self {
        match self {
            PaneKind::Local => PaneKind::Remote,
            PaneKind::Remote => PaneKind::Local,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub size: u64,
    /// 修改时间，毫秒。
    pub modified: Option<u64>,
    /// 远程才有：模式位（含类型位）。
    pub permissions: Option<u32>,
    /// 远程才有：所有者用户名（服务端没给 uid 时为空）。
    pub owner: Option<String>,
}

impl FileEntry {
    pub fn is_parent(&self) -> bool {
        self.name == ".."
    }
}

impl From<RemoteEntry> for FileEntry {
    fn from(entry: RemoteEntry) -> Self {
        Self {
            name: entry.name,
            path: entry.path,
            is_directory: entry.is_directory,
            size: entry.size,
            modified: entry.modified,
            permissions: Some(entry.permissions),
            owner: (!entry.owner.is_empty()).then_some(entry.owner),
        }
    }
}

impl From<LocalEntry> for FileEntry {
    fn from(entry: LocalEntry) -> Self {
        Self {
            name: entry.name,
            path: entry.path,
            is_directory: entry.is_directory,
            size: entry.size,
            modified: entry.modify_time,
            permissions: None,
            owner: None,
        }
    }
}

/// 名称排序同 `localeCompare`：不分大小写，只差大小写时小写在前。
fn compare_names(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

/// 文件夹在前，再按名称。
/// 排好序、按开关去掉点开头的隐藏文件，返回 (显示的, 隐藏了几个)。
pub fn visible_files(files: Vec<FileEntry>, show_hidden: bool) -> (Vec<FileEntry>, usize) {
    let total = files.len();
    let shown: Vec<FileEntry> = sort_files(files).into_iter().filter(|f| show_hidden || !f.name.starts_with('.')).collect();
    let hidden = total - shown.len();
    (shown, hidden)
}

pub fn sort_files(mut files: Vec<FileEntry>) -> Vec<FileEntry> {
    files.sort_by(|a, b| b.is_directory.cmp(&a.is_directory).then_with(|| compare_names(&a.name, &b.name)));
    files
}

fn collapse_slashes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch == '/' && out.ends_with('/') {
            continue;
        }
        out.push(ch);
    }
    out
}

pub fn normalize_remote_path(value: &str) -> String {
    collapse_slashes(&format!("/{value}"))
}

pub fn join_path(directory: &str, name: &str) -> String {
    let separator = if directory.contains('\\') { '\\' } else { '/' };
    if directory.ends_with(separator) {
        format!("{directory}{name}")
    } else {
        format!("{directory}{separator}{name}")
    }
}

fn is_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

pub fn parent_path(path: &str, kind: PaneKind) -> String {
    if kind == PaneKind::Remote {
        let normalized = normalize_remote_path(path);
        let clean = normalized.trim_end_matches('/');
        if clean.is_empty() {
            return "/".into();
        }
        return match clean.rfind('/') {
            Some(index) if index > 0 => clean[..index].to_string(),
            _ => "/".into(),
        };
    }
    let separator = if path.contains('\\') { '\\' } else { '/' };
    let clean = path.trim_end_matches(['/', '\\']);
    let index = clean.rfind(['/', '\\']);
    match index {
        None | Some(0) => separator.to_string(),
        Some(index) if is_drive(&clean[..index]) => format!("{}{separator}", &clean[..index]),
        Some(index) => clean[..index].to_string(),
    }
}

pub fn is_root_path(path: &str, kind: PaneKind) -> bool {
    match kind {
        PaneKind::Remote => normalize_remote_path(path) == "/",
        PaneKind::Local => {
            path == "/" || is_drive(path) || (path.len() == 3 && is_drive(&path[..2]) && path.ends_with(['/', '\\']))
        }
    }
}

/// SFTP realpath "." 的结果当主目录；不是绝对路径（或取失败）时给空，调用方退回 /。
pub fn remote_home_from(result: Option<&str>) -> String {
    match result {
        Some(home) if home.starts_with('/') => home.to_string(),
        _ => String::new(),
    }
}

/// 收藏列表里的一行：目录名（醒目）+ 上一级路径（灰色），整条长路径不好找。根目录只有名字。
pub fn bookmark_label(path: &str) -> (String, String) {
    if is_root_path(path, PaneKind::Remote) {
        return ("/".into(), String::new());
    }
    let normalized = normalize_remote_path(path);
    let clean = normalized.trim_end_matches('/');
    (basename(clean), parent_path(clean, PaneKind::Remote))
}

pub fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

pub fn format_file_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".into();
    }
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let index = ((bytes as f64).ln() / 1024f64.ln()).floor() as usize;
    let index = index.min(UNITS.len() - 1);
    format!("{:.2} {}", bytes as f64 / 1024f64.powi(index as i32), UNITS[index])
}

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: std::ffi::c_long,
    tm_zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
}

/// 固定宽度的列里日期要可预测：省掉秒，零填充两位数（同旧版 Intl 的 2-digit 格式）。
pub fn format_date(millis: Option<u64>) -> String {
    let Some(millis) = millis.filter(|m| *m > 0) else { return "-".into() };
    let seconds = (millis / 1000) as i64;
    let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
    // SAFETY: localtime_r 只写入我们提供的 tm，成功时返回同一指针。
    let ok = unsafe { !localtime_r(&seconds, tm.as_mut_ptr()).is_null() };
    if !ok {
        return "-".into();
    }
    // SAFETY: 成功返回即已完整写入。
    let tm = unsafe { tm.assume_init() };
    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

pub fn format_permissions(mode: Option<u32>) -> String {
    match mode {
        Some(mode) if mode != 0 => {
            let octal = format!("{mode:o}");
            octal[octal.len().saturating_sub(3)..].to_string()
        }
        _ => "-".into(),
    }
}

pub fn valid_octal(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|b| (b'0'..=b'7').contains(&b))
}

/// 八进制 → `rwxr-xr-x`；不合法时 `Invalid`。
pub fn readable_permissions(octal: &str) -> String {
    if !valid_octal(octal) {
        return "Invalid".into();
    }
    octal
        .bytes()
        .map(|digit| {
            let v = digit - b'0';
            format!(
                "{}{}{}",
                if v & 4 != 0 { 'r' } else { '-' },
                if v & 2 != 0 { 'w' } else { '-' },
                if v & 1 != 0 { 'x' } else { '-' }
            )
        })
        .collect()
}

pub fn permissions_to_checks(octal: &str) -> [bool; 9] {
    let mut checks = [false; 9];
    if !valid_octal(octal) {
        return checks;
    }
    for (group, digit) in octal.bytes().enumerate() {
        let v = digit - b'0';
        checks[group * 3] = v & 4 != 0;
        checks[group * 3 + 1] = v & 2 != 0;
        checks[group * 3 + 2] = v & 1 != 0;
    }
    checks
}

pub fn checks_to_permissions(checks: &[bool; 9]) -> String {
    (0..3)
        .map(|group| {
            let v = u8::from(checks[group * 3]) * 4 + u8::from(checks[group * 3 + 1]) * 2 + u8::from(checks[group * 3 + 2]);
            char::from(b'0' + v)
        })
        .collect()
}

/// 最近用过的目录列表缓存（LRU），切回刚看过的目录先显示缓存、后台再刷新。
pub struct ListCache {
    capacity: usize,
    values: HashMap<String, Vec<FileEntry>>,
    order: VecDeque<String>,
}

impl ListCache {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, values: HashMap::new(), order: VecDeque::new() }
    }

    fn touch(&mut self, key: &str) {
        if let Some(index) = self.order.iter().position(|k| k == key) {
            let key = self.order.remove(index).expect("index from position");
            self.order.push_back(key);
        }
    }

    pub fn get(&mut self, key: &str) -> Option<Vec<FileEntry>> {
        let value = self.values.get(key).cloned()?;
        self.touch(key);
        Some(value)
    }

    pub fn set(&mut self, key: String, value: Vec<FileEntry>) {
        self.delete(&key);
        if self.values.len() >= self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.values.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.values.insert(key, value);
    }

    pub fn delete(&mut self, key: &str) {
        if self.values.remove(key).is_some() {
            self.order.retain(|k| k != key);
        }
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool) -> FileEntry {
        FileEntry { name: name.into(), path: format!("/x/{name}"), is_directory: dir, size: 0, modified: None, permissions: None, owner: None }
    }

    #[test]
    fn hidden_files_are_left_out_and_counted() {
        let files = vec![entry(".bashrc", false), entry("b.txt", false), entry(".ssh", true), entry("app", true)];
        let (shown, hidden) = visible_files(files.clone(), false);
        assert_eq!(shown.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["app", "b.txt"]);
        assert_eq!(hidden, 2);
        let (shown, hidden) = visible_files(files, true);
        assert_eq!(shown.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), [".ssh", "app", ".bashrc", "b.txt"]);
        assert_eq!(hidden, 0);
    }

    #[test]
    fn remote_home_comes_from_an_absolute_realpath() {
        assert_eq!(remote_home_from(Some("/home/ubuntu")), "/home/ubuntu");
        assert_eq!(remote_home_from(Some("/root")), "/root");
    }

    #[test]
    fn unusable_home_falls_back_to_root() {
        assert_eq!(remote_home_from(None), "");
        assert_eq!(remote_home_from(Some("")), "");
        assert_eq!(remote_home_from(Some("relative/dir")), "");
    }

    #[test]
    fn bookmark_rows_split_name_and_parent() {
        assert_eq!(bookmark_label("/home/qfdk/deploy"), ("deploy".into(), "/home/qfdk".into()));
        assert_eq!(bookmark_label("/etc"), ("etc".into(), "/".into()));
        assert_eq!(bookmark_label("/var/log/"), ("log".into(), "/var".into()));
        assert_eq!(bookmark_label("//srv//www"), ("www".into(), "/srv".into()));
        assert_eq!(bookmark_label("/"), ("/".into(), String::new()));
    }

    #[test]
    fn remote_paths_are_normalized() {
        assert_eq!(normalize_remote_path(""), "/");
        assert_eq!(normalize_remote_path("//var//log"), "/var/log");
        assert_eq!(normalize_remote_path("var/log/"), "/var/log/");
    }

    #[test]
    fn parent_of_remote_and_local_paths() {
        assert_eq!(parent_path("/var/log", PaneKind::Remote), "/var");
        assert_eq!(parent_path("/var", PaneKind::Remote), "/");
        assert_eq!(parent_path("/", PaneKind::Remote), "/");
        assert_eq!(parent_path("/var/log/", PaneKind::Remote), "/var");
        assert_eq!(parent_path("/Users/me/Downloads", PaneKind::Local), "/Users/me");
        assert_eq!(parent_path("/Users", PaneKind::Local), "/");
        assert_eq!(parent_path("C:\\Users\\me", PaneKind::Local), "C:\\Users");
        assert_eq!(parent_path("C:\\Users", PaneKind::Local), "C:\\");
    }

    #[test]
    fn roots_have_no_parent_row() {
        assert!(is_root_path("/", PaneKind::Remote));
        assert!(is_root_path("", PaneKind::Remote));
        assert!(!is_root_path("/var", PaneKind::Remote));
        assert!(is_root_path("/", PaneKind::Local));
        assert!(is_root_path("C:\\", PaneKind::Local));
        assert!(!is_root_path("/Users", PaneKind::Local));
    }

    #[test]
    fn join_uses_the_directory_separator() {
        assert_eq!(join_path("/home/bob", "a.txt"), "/home/bob/a.txt");
        assert_eq!(join_path("/", "a.txt"), "/a.txt");
        assert_eq!(join_path("C:\\Users", "a.txt"), "C:\\Users\\a.txt");
    }

    #[test]
    fn folders_first_then_names_ignoring_case() {
        let sorted = sort_files(vec![entry("b.txt", false), entry("Zeta", true), entry("alpha", true), entry("A.txt", false)]);
        let names: Vec<_> = sorted.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "A.txt", "b.txt"]);
    }

    #[test]
    fn sizes_and_permissions_format_like_before() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(512 * 1024), "512.00 KB");
        assert_eq!(format_file_size(1200 * 1024), "1.17 MB");
        assert_eq!(format_permissions(Some(0o100644)), "644");
        assert_eq!(format_permissions(Some(0o40755)), "755");
        assert_eq!(format_permissions(None), "-");
        assert_eq!(format_date(None), "-");
    }

    #[test]
    fn permission_checks_round_trip() {
        assert_eq!(readable_permissions("755"), "rwxr-xr-x");
        assert_eq!(readable_permissions("78"), "Invalid");
        let checks = permissions_to_checks("640");
        assert_eq!(checks, [true, true, false, true, false, false, false, false, false]);
        assert_eq!(checks_to_permissions(&checks), "640");
        assert_eq!(permissions_to_checks("9x"), [false; 9]);
    }

    #[test]
    fn cache_evicts_the_least_recently_used() {
        let mut cache = ListCache::new(2);
        cache.set("a".into(), vec![]);
        cache.set("b".into(), vec![]);
        assert!(cache.get("a").is_some());
        cache.set("c".into(), vec![]);
        assert!(cache.get("b").is_none(), "b 最久没用，被挤掉");
        assert!(cache.get("a").is_some());
        assert!(cache.get("c").is_some());
    }
}

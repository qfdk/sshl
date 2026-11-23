//! 「用 SSHL 打开这个文件夹」：`open -a SSHL 目录`、把文件夹拖到程序坞图标上、访达工具栏的「在 SSHL 中打开」按钮
//! （`crates/finder-button`）、命令行 `sshl 目录`，最后都是系统发来的打开事件（GPUI 的 `on_open_urls`）。
//! 每个目录开一个本机终端标签（`Workspace::open_requested`）。
//!
//! 事件可能在窗口建好之前到（冷启动时），也可能和「关窗口后回来」（`Workspace::reopened`）前后脚到：
//! 收到时先同步记进这里，谁先轮到谁取走。打开时自动开的本机终端也先看这里，有请求就只开请求的目录，不多开一个主目录的。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// 记下系统要打开的地址（GPUI 给的是 URL 字符串）。
pub fn push(urls: &[String]) {
    let dirs = directories(urls, Path::is_dir);
    PENDING.lock().unwrap().extend(dirs);
}

/// 取走还没开的目录。
pub fn take() -> Vec<PathBuf> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

/// `file://` 地址换成目录：是文件的用它所在的目录，不存在的、别的协议的不要，重复的只留一个。
fn directories(urls: &[String], is_dir: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for url in urls {
        let Some(path) = file_path(url) else { continue };
        let dir = if is_dir(&path) { path } else { match path.parent() { Some(parent) if is_dir(parent) => parent.to_path_buf(), _ => continue } };
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// `file:///Users/me/a%20b/` → `/Users/me/a b`。只认本机的（主机为空或 localhost）。
fn file_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(rest)?;
    let trimmed = decoded.trim_end_matches('/');
    Some(PathBuf::from(if trimmed.is_empty() { "/" } else { trimmed }))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn file_urls_become_directories() {
        let dirs = ["/Users/me/a b", "/Users/me/项目", "/"];
        let is_dir = |p: &Path| dirs.iter().any(|d| Path::new(d) == p);
        let got = directories(
            &urls(&[
                "file:///Users/me/a%20b/",
                "file:///Users/me/%E9%A1%B9%E7%9B%AE/",
                "file://localhost/Users/me/a%20b/",
                "file:///",
            ]),
            is_dir,
        );
        assert_eq!(got, [PathBuf::from("/Users/me/a b"), PathBuf::from("/Users/me/项目"), PathBuf::from("/")], "重复的只留一个");
    }

    #[test]
    fn a_file_opens_its_folder_and_other_urls_are_ignored() {
        let is_dir = |p: &Path| p == Path::new("/Users/me/src");
        let got = directories(
            &urls(&["file:///Users/me/src/main.rs", "https://example.com/", "sshl://x", "file://server/share/", "file:///gone/x", "file:///bad%zz"]),
            is_dir,
        );
        assert_eq!(got, [PathBuf::from("/Users/me/src")]);
    }
}

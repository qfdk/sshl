//! 打开文件的规则：用什么打开、什么时候把改过的副本传回服务器（界面在 `view/open.rs`）。

use std::time::SystemTime;

/// 用什么打开。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenWith {
    /// 文本（包括没扩展名的配置文件、日志、脚本）：默认的文本编辑器（`open -t`）。`.command` 这类也只当文字打开，不会被执行。
    TextEditor,
    /// 图片、PDF、音视频：交给它自己的默认程序看（预览、QuickTime）。
    Viewer,
}

/// 从服务器打开的只认这两类：别的二进制（`.jar`、`.pkg`、`.dmg`、`.app` 里的东西…）交给默认程序会被执行、安装或挂载。
/// `head` 是文件开头的一段：没有零字节就当文本（空文件也是）。
pub fn open_with(name: &str, head: &[u8]) -> Option<OpenWith> {
    if !head.contains(&0) {
        return Some(OpenWith::TextEditor);
    }
    let (_, extension) = name.rsplit_once('.')?;
    VIEWABLE.contains(&extension.to_ascii_lowercase().as_str()).then_some(OpenWith::Viewer)
}

const VIEWABLE: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "ico", "pdf", "mp4", "mov", "m4v", "mp3", "m4a", "wav", "aac",
    "flac", "aiff",
];

/// 看文件开头多少字节来判断是不是文本。
pub const HEAD_BYTES: usize = 8192;

/// 比这大的先问一句：要整个下载下来才能打开。
pub const LARGE_FILE: u64 = 50 * 1024 * 1024;

/// 本地副本要不要传回：和上次同步时不一样，并且这一轮和上一轮看到的一样（编辑器写完了，不在写到一半的时候传）。
pub fn ready_to_upload(synced: Option<SystemTime>, previous: Option<SystemTime>, now: Option<SystemTime>) -> bool {
    now.is_some() && now != synced && now == previous
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn text_goes_to_the_text_editor_whatever_its_name() {
        for name in ["nginx.conf", "Dockerfile", ".env", "deploy.sh", "run.command", "notes.md", "index.html", "empty"] {
            assert_eq!(open_with(name, b"listen 80;\n"), Some(OpenWith::TextEditor), "{name}");
        }
        assert_eq!(open_with("empty.log", b""), Some(OpenWith::TextEditor), "空文件");
        assert_eq!(open_with("报告.txt", "中文内容".as_bytes()), Some(OpenWith::TextEditor));
    }

    #[test]
    fn only_viewable_binaries_are_opened() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(open_with("shot.png", png), Some(OpenWith::Viewer));
        assert_eq!(open_with("SHOT.PNG", png), Some(OpenWith::Viewer), "扩展名不分大小写");
        assert_eq!(open_with("manual.pdf", b"%PDF-1.7\n\0"), Some(OpenWith::Viewer));
        // 交给默认程序会被执行、安装或挂载的：不开
        for name in ["app.jar", "setup.pkg", "disk.dmg", "server", "lib.so", "archive.tar.gz", "png"] {
            assert_eq!(open_with(name, b"PK\x03\x04\0\0"), None, "{name}");
        }
    }

    #[test]
    fn a_save_is_uploaded_once_the_file_stops_changing() {
        let t = |s: u64| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(s));
        assert!(!ready_to_upload(t(1), t(1), t(1)), "没改过");
        assert!(!ready_to_upload(t(1), t(1), t(2)), "刚看到变了：等下一轮，编辑器可能还在写");
        assert!(ready_to_upload(t(1), t(2), t(2)), "两轮一样：写完了");
        assert!(!ready_to_upload(t(1), t(2), t(3)), "还在变");
        assert!(!ready_to_upload(t(2), t(2), t(2)), "已经传过这一版");
        assert!(!ready_to_upload(t(1), None, None), "副本没了");
    }
}

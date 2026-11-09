//! 传输进度文案（旧 transfer-progress.ts）。

use super::model::format_file_size;

/// 后端上传 / 下载进度事件的载荷；目录传输额外带文件计数。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferProgress {
    pub remote_path: String,
    pub transferred: u64,
    pub total: u64,
    pub files_done: Option<u64>,
    pub files_total: Option<u64>,
}

impl From<&sshl_core::sftp::UploadProgress> for TransferProgress {
    fn from(p: &sshl_core::sftp::UploadProgress) -> Self {
        Self { remote_path: p.remote_path.clone(), transferred: p.transferred, total: p.total, files_done: p.files_done, files_total: p.files_total }
    }
}

impl From<&sshl_core::sftp::DownloadProgress> for TransferProgress {
    fn from(p: &sshl_core::sftp::DownloadProgress) -> Self {
        Self { remote_path: p.remote_path.clone(), transferred: p.transferred, total: p.total, files_done: p.files_done, files_total: p.files_total }
    }
}

/// 进度条状态：传输结束 3 秒后收起时只改 active / progress，保留最后一条文字。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferStatus {
    pub active: bool,
    /// 0–100。
    pub progress: u32,
    pub info: String,
}

impl TransferStatus {
    pub fn show(progress: u32, info: impl Into<String>) -> Self {
        Self { active: true, progress, info: info.into() }
    }

    pub fn hide(&mut self) {
        self.active = false;
        self.progress = 0;
    }
}

pub fn describe_progress(verb: &str, data: &TransferProgress) -> TransferStatus {
    let progress = if data.total > 0 { ((data.transferred as f64 / data.total as f64) * 100.0).round().min(100.0) as u32 } else { 0 };
    let name = data.remote_path.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    let files = match data.files_total {
        Some(total) if total > 0 => format!(" · {}/{} 个文件", data.files_done.unwrap_or(0), total),
        _ => String::new(),
    };
    TransferStatus::show(
        progress,
        format!(
            "正在{verb}: {name} ({progress}% - {}/{}{files})",
            format_file_size(data.transferred),
            format_file_size(data.total)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(path: &str, transferred: u64, total: u64, files: Option<(u64, u64)>) -> TransferProgress {
        TransferProgress { remote_path: path.into(), transferred, total, files_done: files.map(|f| f.0), files_total: files.map(|f| f.1) }
    }

    #[test]
    fn single_file_shows_percent_and_bytes() {
        let status = describe_progress("下载", &p("/srv/app.log", 512 * 1024, 1024 * 1024, None));
        assert_eq!(status, TransferStatus::show(50, "正在下载: app.log (50% - 512.00 KB/1.00 MB)"));
    }

    #[test]
    fn directories_add_the_file_count() {
        let status = describe_progress("上传", &p("/home/q/logs", 300 * 1024, 1200 * 1024, Some((3, 10))));
        assert_eq!(status, TransferStatus::show(25, "正在上传: logs (25% - 300.00 KB/1.17 MB · 3/10 个文件)"));
    }

    #[test]
    fn trailing_slash_still_names_the_directory() {
        assert!(describe_progress("下载", &p("/home/q/logs/", 0, 10, Some((0, 1)))).info.starts_with("正在下载: logs ("));
    }

    #[test]
    fn zero_total_and_overshoot_are_clamped() {
        assert_eq!(describe_progress("下载", &p("/e", 0, 0, Some((0, 0)))).progress, 0);
        assert_eq!(describe_progress("下载", &p("/f", 20, 10, None)).progress, 100);
    }

    #[test]
    fn hiding_keeps_the_last_text() {
        let mut status = TransferStatus::show(40, "正在上传: a");
        status.hide();
        assert_eq!(status, TransferStatus { active: false, progress: 0, info: "正在上传: a".into() });
    }
}

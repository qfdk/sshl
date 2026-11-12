//! 传输进度文案（旧 transfer-progress.ts）。


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

/// 最近几秒的平均速度（字节 / 秒）：只看最近 `WINDOW` 的进度，忽快忽慢的网络下数字也稳。
#[derive(Default)]
pub struct SpeedMeter {
    samples: std::collections::VecDeque<(std::time::Instant, u64)>,
}

impl SpeedMeter {
    const WINDOW: std::time::Duration = std::time::Duration::from_secs(3);
    /// 不到这么久的数据不报速度（刚开始那一下不准）。
    const MIN_SPAN: std::time::Duration = std::time::Duration::from_millis(500);

    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// 记下「到 `now` 为止一共传了 `transferred` 字节」，返回当前速度。字节数变小说明换了一个传输，重新算。
    pub fn record(&mut self, now: std::time::Instant, transferred: u64) -> Option<f64> {
        if self.samples.back().is_some_and(|&(_, last)| transferred < last) {
            self.samples.clear();
        }
        self.samples.push_back((now, transferred));
        while self.samples.len() > 2 && self.samples.get(1).is_some_and(|&(t, _)| now.duration_since(t) >= Self::WINDOW) {
            self.samples.pop_front();
        }
        let (&(first_at, first), &(last_at, last)) = (self.samples.front()?, self.samples.back()?);
        let span = last_at.duration_since(first_at);
        (span >= Self::MIN_SPAN).then(|| (last - first) as f64 / span.as_secs_f64())
    }
}

/// `12.3 MB/s`、`850 KB/s`。
pub fn format_rate(bytes_per_sec: f64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let (mut value, mut unit) = (bytes_per_sec.max(0.), 0);
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 || value >= 100. { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// 剩余时间：`剩 8 秒`、`剩 3 分钟`、`剩 1 小时 5 分`。
pub fn format_eta(secs: u64) -> String {
    match secs {
        0..=59 => tr!("剩 {} 秒", secs.max(1)),
        60..=3599 => tr!("剩 {} 分钟", secs.div_ceil(60)),
        _ => tr!("剩 {} 小时 {} 分", secs / 3600, secs % 3600 / 60),
    }
}

/// 进度文字：`正在上传 big.bin 45% · 12.3 MB/s · 剩 8 秒 · 还有 2 个`。目录多一段文件计数；速度刚开始不报。
pub fn describe_progress(upload: bool, data: &TransferProgress, speed: Option<f64>, queued: usize) -> TransferStatus {
    let progress = if data.total > 0 { ((data.transferred as f64 / data.total as f64) * 100.0).round().min(100.0) as u32 } else { 0 };
    let name = data.remote_path.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    let mut parts = vec![if upload { tr!("正在上传 {} {}%", name, progress) } else { tr!("正在下载 {} {}%", name, progress) }];
    if let Some(total) = data.files_total.filter(|&t| t > 0) {
        parts.push(tr!("{}/{} 个文件", data.files_done.unwrap_or(0), total));
    }
    if let Some(speed) = speed.filter(|&s| s > 0.) {
        parts.push(format_rate(speed));
        let left = data.total.saturating_sub(data.transferred);
        if left > 0 {
            parts.push(format_eta((left as f64 / speed).ceil() as u64));
        }
    }
    if queued > 0 {
        parts.push(tr!("还有 {} 个", queued));
    }
    TransferStatus::show(progress, parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn p(path: &str, transferred: u64, total: u64, files: Option<(u64, u64)>) -> TransferProgress {
        TransferProgress { remote_path: path.into(), transferred, total, files_done: files.map(|f| f.0), files_total: files.map(|f| f.1) }
    }

    #[test]
    fn a_file_shows_percent_speed_time_left_and_the_queue() {
        let status = describe_progress(true, &p("/srv/big.bin", 45 << 20, 100 << 20, None), Some((11 << 20) as f64), 2);
        assert_eq!(status, TransferStatus::show(45, "正在上传 big.bin 45% · 11.0 MB/s · 剩 5 秒 · 还有 2 个"));
    }

    #[test]
    fn directories_add_the_file_count_and_speed_waits_for_data() {
        let status = describe_progress(false, &p("/home/q/logs/", 300 * 1024, 1200 * 1024, Some((3, 10))), None, 0);
        assert_eq!(status, TransferStatus::show(25, "正在下载 logs 25% · 3/10 个文件"));
    }

    #[test]
    fn zero_total_and_overshoot_are_clamped() {
        assert_eq!(describe_progress(false, &p("/e", 0, 0, Some((0, 0))), None, 0).progress, 0);
        assert_eq!(describe_progress(false, &p("/f", 20, 10, None), Some(5.), 0).progress, 100);
        assert!(!describe_progress(false, &p("/f", 10, 10, None), Some(5.), 0).info.contains('剩'), "传完了不报剩余时间");
    }

    #[test]
    fn speed_is_the_average_over_the_last_few_seconds() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut meter = SpeedMeter::default();
        assert_eq!(meter.record(at(0), 0), None);
        assert_eq!(meter.record(at(200), 200_000), None, "不到半秒不报");
        assert_eq!(meter.record(at(1000), 1_000_000), Some(1_000_000.));
        // 前 3 秒很快、之后变慢：只看最近 3 秒
        meter.record(at(3000), 9_000_000);
        assert_eq!(meter.record(at(6000), 9_300_000), Some(100_000.));
        // 字节数变小：换了一个传输，从头算
        assert_eq!(meter.record(at(6100), 0), None);
    }

    #[test]
    fn rates_and_time_left_read_well() {
        assert_eq!(format_rate(512.), "512 B/s");
        assert_eq!(format_rate(850. * 1024.), "850 KB/s");
        assert_eq!(format_rate(12.34 * 1024. * 1024.), "12.3 MB/s");
        assert_eq!(format_eta(0), "剩 1 秒");
        assert_eq!(format_eta(59), "剩 59 秒");
        assert_eq!(format_eta(61), "剩 2 分钟");
        assert_eq!(format_eta(3600 + 5 * 60), "剩 1 小时 5 分");
    }

    #[test]
    fn hiding_keeps_the_last_text() {
        let mut status = TransferStatus::show(40, "正在上传 a");
        status.hide();
        assert_eq!(status, TransferStatus { active: false, progress: 0, info: "正在上传 a".into() });
    }
}

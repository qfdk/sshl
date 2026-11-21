//! 检查更新（用户 2026-11-21：「有了更新检测……直接弹一框让他们去 GitHub 去下载」）：不自动下载、不自动装，
//! 只告诉有新版本、打开 GitHub 的发布页。
//!
//! 查的是 GitHub Releases 的 latest（只有正式版，没有草稿和预发布），用系统自带的 curl 在单独的线程上请求：
//! 为一次请求引入一整套 HTTP / TLS 依赖不值；curl 也照用户的代理环境变量走。
//!
//! 自动：打开 SSHL 时查一次（用户：「打开的时候检查不就好了」）——启动，和关窗口后从程序坞点开（进程常驻，这也是打开）；
//! 窗口出来两秒后在后台查，不在启动路径上；来回点开时离上次自动检查不满一小时不再查。查到新版本时窗口在前台就直接弹，
//! 不在前台（打开后马上切走了）先记着，等切回 SSHL（窗口变成激活）时再弹：不在打开的那一刻之外抢焦点
//! （终端里正敲着命令、输着 sudo 密码时弹框会抢走焦点，回车按到「前往下载」上）。
//! 手动：菜单栏「SSHL → 检查更新…」，马上显示结果（有新版本、已是最新、查不到）。
//! 「以后再说」= 这个版本不再自动提醒（`settings.skipped_update`），手动检查照样显示；更新的版本出来照常提醒。
//! 打开时的检查可以在设置 →「关于」里关掉（`settings.auto_check_updates`），那一页也有「检查更新」按钮。

use std::time::{Duration, Instant};

use futures::channel::oneshot;
use gpui::{App, Global, Window};

use crate::prompts::{self, Options};
use crate::settings;

/// 最新的正式版。
const LATEST: &str = "https://api.github.com/repos/qfdk/sshl/releases/latest";
/// 只打开这个仓库的发布页：接口返回别的地址一律不开。
const RELEASES_PAGE: &str = "https://github.com/qfdk/sshl/releases/";
/// 打开后多久自动检查：窗口已经出来了，不在启动路径上。
const AFTER_OPEN: Duration = Duration::from_secs(2);
/// 两次自动检查至少隔这么久：关窗口后从程序坞点开也算打开，来回点不反复请求 GitHub。
const MIN_GAP: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// 原样的 tag（`v0.7.6`），提示框里显示、「以后再说」时记下来。
    pub tag: String,
    pub version: Version,
    /// 发布页。
    pub url: String,
}

pub type Version = (u64, u64, u64);

/// `v0.7.6` / `0.7.6` / `v1.2`（补成 1.2.0）。带后缀的（`0.8.0-rc.1`）和别的写法不认。
pub fn parse_version(tag: &str) -> Option<Version> {
    let tag = tag.trim();
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |part| part.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |part| part.parse().ok())?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// GitHub 接口返回的 latest：草稿、预发布、版本号认不出、发布页不是这个仓库的，都不算。
pub fn parse_latest(json: &str) -> Option<Release> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if value["draft"].as_bool() == Some(true) || value["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = value["tag_name"].as_str()?;
    let version = parse_version(tag)?;
    let url = value["html_url"].as_str().filter(|url| url.starts_with(RELEASES_PAGE))?;
    Some(Release { tag: tag.to_string(), version, url: url.to_string() })
}

/// 比现在用的新（按数字比，0.7.10 比 0.7.9 新）。
pub fn is_newer(release: &Release, current: &str) -> bool {
    parse_version(current).is_some_and(|current| release.version > current)
}

/// 这次打开要不要自动检查：第一次打开，或者离上次自动检查满 [`MIN_GAP`]。
pub fn due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= MIN_GAP)
}

/// 一次检查的结果。设置 →「关于」的状态行照它写（「当前版本 0.7.5 · 已是最新版本」）。
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    UpToDate,
    Newer(Release),
    Failed(String),
}

pub fn outcome(result: Result<Release, String>, current: &str) -> Outcome {
    match result {
        Ok(release) if is_newer(&release, current) => Outcome::Newer(release),
        Ok(_) => Outcome::UpToDate,
        Err(error) => Outcome::Failed(error),
    }
}

/// 「复制版本信息」：报问题时贴出来的一行，`SSHL 0.7.5 · macOS 26.1 · aarch64`。点的时候才问系统版本（sw_vers，十几毫秒）。
pub fn version_info() -> String {
    let macos = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "?".into());
    format!("SSHL {} · macOS {macos} · {}", current_version(), std::env::consts::ARCH)
}

/// 自动检查查到的这个版本要不要提醒：比现在的新，而且没点过「以后再说」。
pub fn worth_announcing(release: &Release, current: &str, skipped: &str) -> bool {
    is_newer(release, current) && release.tag != skipped
}

/// 现在用的版本。开发版可以用 `SSHL_DEV_VERSION=0.7.0` 假装是旧版本，看「有新版本」的提示框。
pub fn current_version() -> String {
    #[cfg(any(debug_assertions, feature = "devhook"))]
    if let Ok(version) = std::env::var("SSHL_DEV_VERSION") {
        return version;
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn fetch_blocking(current: &str) -> Result<Release, String> {
    let output = std::process::Command::new("/usr/bin/curl")
        .args(["--silent", "--show-error", "--fail", "--location", "--max-time", "15"])
        .args(["--header", "Accept: application/vnd.github+json"])
        .args(["--user-agent", &format!("SSHL/{current}"), LATEST])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    parse_latest(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| tr!("GitHub 返回的内容和预期的不一样").to_string())
}

/// 在单独的线程上请求（最多 15 秒），不占界面线程和后端的 tokio 线程。
async fn fetch(current: String) -> Result<Release, String> {
    let (reply, answer) = oneshot::channel();
    std::thread::Builder::new()
        .name("sshl-update".into())
        .spawn(move || {
            let _ = reply.send(fetch_blocking(&current));
        })
        .map_err(|error| error.to_string())?;
    answer.await.map_err(|_| tr!("检查更新的线程没有回音").to_string())?
}

#[derive(Default)]
struct UpdateState {
    /// 自动检查查到、还没弹出来的新版本（打开后马上切走了，等窗口下次激活）。
    pending: Option<Release>,
    /// 正在手动检查：连点菜单不重复请求。
    checking: bool,
    /// 上次自动检查的时候（见 [`due`]）。
    last_auto: Option<Instant>,
    /// 这次运行里最近一次检查的结果（自动的失败不记：没网时状态行别冒出一句报错）。
    last: Option<Outcome>,
}

impl Global for UpdateState {}

/// 启动时调一次（窗口出来之后）：记下状态，做这次打开的自动检查。
pub fn init(cx: &mut App) {
    cx.set_global(UpdateState::default());
    check_on_open(cx);
}

/// 打开 SSHL 时（启动、从程序坞点开）自动检查一次。设置里关掉了、开发构建、开着测试口（自动化测试一遍遍起进程）时不查，
/// 手动检查照样能用。查不到（没网、GitHub 限流）不出声，下次打开再查。
pub fn check_on_open(cx: &mut App) {
    if cfg!(debug_assertions) || std::env::var_os("SSHL_DEV_SOCKET").is_some() || !cx.has_global::<UpdateState>() {
        return;
    }
    if !settings::get(cx).auto_check_updates {
        return;
    }
    let now = Instant::now();
    let state = cx.global_mut::<UpdateState>();
    if !due(state.last_auto, now) {
        return;
    }
    state.last_auto = Some(now);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(AFTER_OPEN).await;
        let current = current_version();
        let Ok(release) = fetch(current.clone()).await else { return };
        cx.update(|cx| {
            cx.global_mut::<UpdateState>().last = Some(outcome(Ok(release.clone()), &current));
            cx.refresh_windows();
            if !worth_announcing(&release, &current, &settings::get(cx).skipped_update) {
                return;
            }
            // 窗口在前台（刚打开）：直接弹；打开后马上切走了：等切回来
            let shown = cx.windows().first().copied().and_then(|window| {
                window
                    .update(cx, |_, window, cx| {
                        let active = window.is_window_active();
                        if active {
                            announce(release.clone(), window, cx);
                        }
                        active
                    })
                    .ok()
            });
            if shown != Some(true) {
                cx.global_mut::<UpdateState>().pending = Some(release);
            }
        });
    })
    .detach();
}

/// 窗口变成激活时（工作区里调）：自动检查查到的新版本这时候弹。
pub fn show_pending(window: &mut Window, cx: &mut App) {
    if let Some(release) = cx.global_mut::<UpdateState>().pending.take() {
        announce(release, window, cx);
    }
}

/// 正在手动检查（设置 →「关于」的按钮这时写「检查中…」）。
pub fn is_checking(cx: &App) -> bool {
    cx.global::<UpdateState>().checking
}

/// 这次运行里最近一次检查的结果，还没查过是 None。
pub fn last_outcome(cx: &App) -> Option<Outcome> {
    cx.global::<UpdateState>().last.clone()
}

/// 手动检查的结果怎么告诉用户。
#[derive(Clone, Copy, PartialEq)]
pub enum Report {
    /// 菜单栏「检查更新…」：弹框（有新版本、已是最新、查不到）。
    Prompt,
    /// 设置 →「关于」的按钮：写在那一页的状态行上，不弹框。
    Inline,
}

/// 马上查。结果记下来（[`last_outcome`]），按 `report` 弹框或只刷新界面。
pub fn check_now(window: &mut Window, cx: &mut App, report: Report) {
    if std::mem::replace(&mut cx.global_mut::<UpdateState>().checking, true) {
        return;
    }
    cx.refresh_windows();
    let window_handle = window.window_handle();
    cx.spawn(async move |cx| {
        let current = current_version();
        let result = outcome(fetch(current.clone()).await, &current);
        cx.update(|cx| {
            let state = cx.global_mut::<UpdateState>();
            state.checking = false;
            state.pending = None;
            state.last = Some(result.clone());
            cx.refresh_windows();
        });
        if report == Report::Inline {
            return;
        }
        let _ = window_handle.update(cx, |_, window, cx| {
            // 窗口可能最小化或藏起来了：拿出来，不然提示框在看不见的地方
            window.activate_window();
            match result {
                Outcome::Newer(release) => announce(release, window, cx),
                Outcome::UpToDate => prompts::alert_later(
                    window,
                    cx,
                    Options::new(tr!("已是最新版本")).message(tr!("SSHL v{} 是目前最新的版本。", current)),
                ),
                Outcome::Failed(error) => prompts::alert_later(window, cx, Options::new(tr!("检查更新失败")).message(error)),
            }
        });
    })
    .detach();
}

/// 「SSHL v0.7.6 发布了」：前往下载 = 打开发布页；以后再说（和 Esc）= 这个版本不再自动提醒。
fn announce(release: Release, window: &mut Window, cx: &mut App) {
    let options = Options::new(tr!("SSHL {} 发布了", release.tag))
        .message(tr!("你现在用的是 v{}。到 GitHub 下载新版本，替换「应用程序」里的 SSHL 就行。", current_version()))
        .confirm(tr!("前往下载"))
        .cancel(tr!("以后再说"));
    let answer = prompts::confirm(window, cx, options);
    cx.spawn(async move |cx| {
        let download = answer.await.unwrap_or(false);
        cx.update(|cx| {
            if download {
                cx.open_url(&release.url);
            } else {
                settings::update(cx, |s| s.skipped_update = release.tag.clone());
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Release {
        Release { tag: tag.into(), version: parse_version(tag).unwrap(), url: format!("{RELEASES_PAGE}tag/{tag}") }
    }

    #[test]
    fn versions_are_read_from_tags() {
        assert_eq!(parse_version("v0.7.6"), Some((0, 7, 6)));
        assert_eq!(parse_version("0.7.6"), Some((0, 7, 6)));
        assert_eq!(parse_version(" v1.2 "), Some((1, 2, 0)));
        assert_eq!(parse_version("v2"), Some((2, 0, 0)));
        for odd in ["v0.8.0-rc.1", "nightly", "", "v", "v1.2.3.4", "v1..2"] {
            assert_eq!(parse_version(odd), None, "{odd}");
        }
    }

    /// 按数字比，不按字符串：0.7.10 比 0.7.9 新。同一个版本、旧的版本都不算新。
    #[test]
    fn newer_compares_numbers() {
        assert!(is_newer(&release("v0.7.10"), "0.7.9"));
        assert!(is_newer(&release("v0.8.0"), "0.7.5"));
        assert!(is_newer(&release("v1.0.0"), "0.99.99"));
        assert!(!is_newer(&release("v0.7.5"), "0.7.5"));
        assert!(!is_newer(&release("v0.7.4"), "0.7.5"));
        assert!(!is_newer(&release("v0.7.6"), "garbage"), "认不出现在的版本：不提醒");
    }

    /// 打开时自动检查：第一次打开要查；之后（关窗口后从程序坞点开）离上次自动检查满一小时才再查。
    #[test]
    fn opening_again_soon_does_not_check_again() {
        let now = Instant::now();
        assert!(due(None, now));
        assert!(!due(Some(now), now));
        assert!(!due(now.checked_sub(Duration::from_secs(59 * 60)), now));
        assert!(due(now.checked_sub(Duration::from_secs(61 * 60)), now));
    }

    /// 检查的结果（「关于」页的状态行照它写）：比现在新的是新版本，一样或更旧的是已是最新，请求失败带原因。
    #[test]
    fn a_check_ends_in_one_of_three_outcomes() {
        assert_eq!(outcome(Ok(release("v0.7.6")), "0.7.5"), Outcome::Newer(release("v0.7.6")));
        assert_eq!(outcome(Ok(release("v0.7.5")), "0.7.5"), Outcome::UpToDate);
        assert_eq!(outcome(Ok(release("v0.7.4")), "0.7.5"), Outcome::UpToDate);
        assert_eq!(outcome(Err("curl: (6) Could not resolve host".into()), "0.7.5"), Outcome::Failed("curl: (6) Could not resolve host".into()));
    }

    /// 自动检查：点过「以后再说」的版本不再提醒，更新的版本出来照常提醒。
    #[test]
    fn a_skipped_version_is_not_announced_again() {
        assert!(worth_announcing(&release("v0.7.6"), "0.7.5", ""));
        assert!(!worth_announcing(&release("v0.7.6"), "0.7.5", "v0.7.6"));
        assert!(worth_announcing(&release("v0.7.7"), "0.7.5", "v0.7.6"));
        assert!(!worth_announcing(&release("v0.7.5"), "0.7.5", ""));
    }

    /// GitHub 接口的返回（截取有用的几项）：只认正式版、认得出的版本号、这个仓库的发布页。
    #[test]
    fn the_latest_release_is_read_from_the_api() {
        let json = r#"{"tag_name":"v0.7.6","name":"v0.7.6","draft":false,"prerelease":false,
            "html_url":"https://github.com/qfdk/sshl/releases/tag/v0.7.6","assets":[]}"#;
        assert_eq!(
            parse_latest(json),
            Some(Release { tag: "v0.7.6".into(), version: (0, 7, 6), url: "https://github.com/qfdk/sshl/releases/tag/v0.7.6".into() })
        );
        let with = |field: &str, value: &str| json.replace(field, value);
        assert_eq!(parse_latest(&with(r#""draft":false"#, r#""draft":true"#)), None);
        assert_eq!(parse_latest(&with(r#""prerelease":false"#, r#""prerelease":true"#)), None);
        assert_eq!(parse_latest(&with(r#""tag_name":"v0.7.6""#, r#""tag_name":"latest-build""#)), None);
        assert_eq!(parse_latest(&with("https://github.com/qfdk/sshl/releases/tag/v0.7.6", "https://evil.example/sshl.zip")), None, "只开本仓库的发布页");
        assert_eq!(parse_latest(r#"{"message":"API rate limit exceeded"}"#), None);
        assert_eq!(parse_latest("<html>"), None);
    }
}

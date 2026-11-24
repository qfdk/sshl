//! 从 SSHL 外面打开终端的两个入口，都是让 SSHL「打开文件夹」（见 `open_request.rs`）：
//! 访达工具栏按钮（`crates/finder-button`，图标是灰色线框里的「>_」）和命令行 `sshl 目录`（`assets/bin/sshl`）。
//! 两个都打包在 SSHL.app 里（scripts/bundle.sh），设置 →「集成」里一键装上、一键去掉；`cargo run` 的开发版没有，那一页只说一句。
//!
//! 访达按钮在 SSHL.app 包里面，按住 ⌘ 拖不上工具栏（用户 2026-11-24 试过），所以由我们直接改访达的工具栏配置（`finder_toolbar.js`），
//! 再重启访达。它问访达当前目录时系统把它算成 SSHL 的：查的是 SSHL 有没有控制访达的权限（见 scripts/sign.sh）。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

/// 命令行装到这里（链接到 SSHL.app 里的脚本）：macOS 默认的 PATH 里就有它。
pub const CLI_LINK: &str = "/usr/local/bin/sshl";

const FINDER_DOMAIN: &str = "com.apple.finder";
const TOOLBAR_JS: &str = include_str!("finder_toolbar.js");

/// 两个入口装上了没有；None = 这个 SSHL 没打包它们（开发版）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Status {
    pub finder: Option<bool>,
    pub cli: Option<bool>,
}

impl Status {
    /// 要起一次 osascript 读访达的配置（一百多毫秒）：在后台取，对话框打开时和每次操作后各一次。
    pub fn read() -> Self {
        Status {
            finder: finder_button().map(|button| toolbar("has", &button, FINDER_DOMAIN).as_deref() == Ok("yes")),
            cli: cli_script().map(|script| links_to(Path::new(CLI_LINK), &script)),
        }
    }
}

/// 自己所在的 SSHL.app（`…/SSHL.app/Contents/MacOS/sshl`）。
fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.ancestors().nth(3)?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// SSHL.app 里的访达按钮。
pub fn finder_button() -> Option<PathBuf> {
    bundle().map(|app| app.join("Contents/Helpers/Open in SSHL.app")).filter(|path| path.is_dir())
}

/// 命令行脚本。
pub fn cli_script() -> Option<PathBuf> {
    bundle().map(|app| app.join("Contents/Resources/bin/sshl")).filter(|path| path.is_file())
}

/// 按钮加到访达工具栏上（已经有了就不重复加），按系统外观设好图标，重启访达。会阻塞，别在界面线程上调。
pub fn add_to_finder(button: &Path) -> Result<(), String> {
    toolbar("add", button, FINDER_DOMAIN)?;
    if let Err(error) = set_button_icon(button, SYSTEM_DARK.load(Ordering::SeqCst) == DARK) {
        tracing::warn!("finder button icon: {error}");
    }
    restart_finder()
}

const LIGHT: u8 = 1;
const DARK: u8 = 2;
/// 系统现在是浅色还是深色（0 = 还不知道，当浅色）：访达跟的是系统外观，不是 SSHL 设置里的外观。
static SYSTEM_DARK: AtomicU8 = AtomicU8::new(0);
static ICON_LOCK: Mutex<()> = Mutex::new(());

/// 系统外观（启动时、系统切换深浅色时，`workspace::sync_appearance`）：按钮在工具栏上的话换成对应的图标。
/// 自定义图标不会自己跟着深浅色变，只能换掉：浅色工具栏上用深灰的，深色工具栏上用浅灰的（深灰在深色工具栏上看不清）。
/// 每个进程第一次也换一次：SSHL 被整个换掉（更新）后包里的自定义图标就没了，启动时补回来。在后台线程里做（要起 osascript）。
pub fn follow_system_appearance(dark: bool) {
    let want = if dark { DARK } else { LIGHT };
    if SYSTEM_DARK.swap(want, Ordering::SeqCst) == want {
        return;
    }
    let Some(button) = finder_button() else { return };
    std::thread::spawn(move || {
        // 连着切了两次：后跑的那个按最新的来，不会把旧的盖回去
        let _guard = ICON_LOCK.lock();
        if toolbar("has", &button, FINDER_DOMAIN).as_deref() != Ok("yes") {
            return;
        }
        if let Err(error) = set_button_icon(&button, SYSTEM_DARK.load(Ordering::SeqCst) == DARK) {
            tracing::warn!("finder button icon: {error}");
        }
    });
}

/// 给按钮设访达自定义图标（深色 / 浅色那一套），访达不用重启，工具栏上的图标当场换掉。
/// 先清掉旧的、告诉访达，停一下再设新的、再告诉它：直接从一套换成另一套访达不刷新（实测取样：换了十秒工具栏上还是旧的），
/// 从「没有」到「有」它才认。设自定义图标还为了去掉系统给不是圆角方形的应用图标套的底框（同 OpenInTerminal-Lite 的做法）。
fn set_button_icon(button: &Path, dark: bool) -> Result<(), String> {
    const SET_ICON: &str = "ObjC.import('AppKit');
function run(argv) {
  const image = $.NSImage.alloc.initWithContentsOfFile(argv[1]);
  if (image.isNil()) throw new Error('no icon ' + argv[1]);
  const workspace = $.NSWorkspace.sharedWorkspace;
  workspace.setIconForFileOptions($(), argv[0], 0);
  workspace.noteFileSystemChanged(argv[0]);
  delay(0.3);
  const done = workspace.setIconForFileOptions(image, argv[0], 0);
  workspace.noteFileSystemChanged(argv[0]);
  return done ? 'ok' : 'failed';
}";
    let icns = button.join(if dark { "Contents/Resources/icon-dark.icns" } else { "Contents/Resources/icon.icns" });
    let output = Command::new("/usr/bin/osascript").args(["-l", "JavaScript", "-e", SET_ICON]).arg(button).arg(&icns).output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout == "ok" { Ok(()) } else { Err(format!("{stdout} {}", String::from_utf8_lossy(&output.stderr).trim())) }
}

/// 从访达工具栏上去掉按钮，重启访达。
pub fn remove_from_finder(button: &Path) -> Result<(), String> {
    toolbar("remove", button, FINDER_DOMAIN)?;
    restart_finder()
}

fn toolbar(action: &str, button: &Path, domain: &str) -> Result<String, String> {
    let output = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", TOOLBAR_JS, action])
        .arg(button)
        .arg(domain)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// 访达只在启动时读工具栏配置。窗口会关掉再按原样打开。
fn restart_finder() -> Result<(), String> {
    Command::new("/usr/bin/killall").arg("Finder").status().map_err(|e| e.to_string())?;
    Ok(())
}

/// 装命令行：/usr/local/bin/sshl 链接到 SSHL.app 里的脚本（以前装的链接换掉；那儿是别的程序的文件就不动）。
/// 自己写不进去（/usr/local/bin 归 root）就用系统的授权框要管理员密码，同 VS Code 的「安装 code 命令」。
/// Ok(false) = 授权框点了取消。会阻塞，别在界面线程上调。
pub fn install_cli(script: &Path) -> Result<bool, String> {
    let link = Path::new(CLI_LINK);
    match link_in_place(link, script) {
        Ok(()) => Ok(true),
        Err(LinkError::Taken) => Err(tr!("{} 已经有别的文件，没有覆盖", CLI_LINK).to_string()),
        Err(LinkError::Denied) => as_admin(&format!("mkdir -p /usr/local/bin && ln -sfn {} {}", shell_quote(script), shell_quote(link))),
    }
}

/// 去掉命令行：只删链接，那儿是别的程序的文件不动。
pub fn remove_cli() -> Result<bool, String> {
    let link = Path::new(CLI_LINK);
    match std::fs::symlink_metadata(link) {
        Err(_) => Ok(true),
        Ok(meta) if !meta.file_type().is_symlink() => Err(tr!("{} 已经有别的文件，没有覆盖", CLI_LINK).to_string()),
        Ok(_) => match std::fs::remove_file(link) {
            Ok(()) => Ok(true),
            Err(_) => as_admin(&format!("rm -f {}", shell_quote(link))),
        },
    }
}

enum LinkError {
    /// 那儿是别的程序的文件（不是链接）。
    Taken,
    /// 没权限，要管理员。
    Denied,
}

fn link_in_place(link: &Path, script: &Path) -> Result<(), LinkError> {
    if let Ok(meta) = std::fs::symlink_metadata(link) {
        if !meta.file_type().is_symlink() {
            return Err(LinkError::Taken);
        }
        std::fs::remove_file(link).map_err(|_| LinkError::Denied)?;
    }
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir).map_err(|_| LinkError::Denied)?;
    }
    std::os::unix::fs::symlink(script, link).map_err(|_| LinkError::Denied)
}

fn links_to(link: &Path, script: &Path) -> bool {
    matches!((std::fs::canonicalize(link), std::fs::canonicalize(script)), (Ok(a), Ok(b)) if a == b)
}

/// 用系统的授权框以管理员身份跑一条命令。Ok(false) = 用户点了取消。
fn as_admin(command: &str) -> Result<bool, String> {
    let source = format!("do shell script \"{}\" with administrator privileges", command.replace('\\', "\\\\").replace('"', "\\\""));
    let output = Command::new("/usr/bin/osascript").arg("-e").arg(source).output().map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // 授权框点了取消：errUserCanceled
    if stderr.contains("(-128)") {
        return Ok(false);
    }
    Err(stderr.trim().to_string())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sshl-launchers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    /// 没装过就建链接；装过别的版本（旧路径）的链接换成这一个；那儿是别人的文件不动。
    #[test]
    fn the_cli_link_is_created_replaced_but_never_overwrites_a_real_file() {
        let dir = scratch("link");
        let script = dir.join("SSHL.app/Contents/Resources/bin/sshl");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        let link = dir.join("bin/sshl");

        assert!(link_in_place(&link, &script).is_ok());
        assert!(links_to(&link, &script));

        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(dir.join("old/sshl"), &link).unwrap();
        assert!(!links_to(&link, &script), "指向不存在的旧路径不算装好");
        assert!(link_in_place(&link, &script).is_ok());
        assert!(links_to(&link, &script));

        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, "someone else's").unwrap();
        assert!(matches!(link_in_place(&link, &script), Err(LinkError::Taken)));
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "someone else's");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[derive(serde::Deserialize)]
    struct Dump {
        ids: Vec<String>,
        paths: std::collections::BTreeMap<String, Option<String>>,
    }

    /// 在一个临时的偏好设置域里演一遍：已有一个自定义按钮（别的程序的）时，加在它后面、不重复加；
    /// 去掉只去掉我们的，别的按钮和它的位置还原；查得到 / 查不到。不碰真的访达配置。
    #[test]
    fn the_finder_toolbar_button_is_added_next_to_other_buttons_and_removed_cleanly() {
        let dir = scratch("toolbar");
        let other = dir.join("Other.app");
        let ours = dir.join("SSHL.app/Contents/Helpers/Open in SSHL.app");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(&ours).unwrap();
        let domain = format!("me.qfdk.sshl.toolbar-test-{}", std::process::id());
        let seed = dir.join("seed.plist");
        let other_url = format!("file://{}/", other.display());
        std::fs::write(
            &seed,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>NSToolbar Configuration Browser</key><dict>
<key>TB Item Identifiers</key><array><string>com.apple.finder.BACK</string><string>com.apple.finder.loc </string><string>com.apple.finder.SWCH</string><string>com.apple.finder.SRCH</string></array>
<key>TB Item Plists</key><dict><key>1</key><dict><key>_CFURLString</key><string>{other_url}</string><key>_CFURLStringType</key><integer>15</integer></dict></dict>
<key>TB Is Shown</key><true/>
</dict></dict></plist>"#
            ),
        )
        .unwrap();
        assert!(Command::new("/usr/bin/defaults").args(["import", &domain]).arg(&seed).status().unwrap().success());
        let dump = || serde_json::from_str::<Dump>(&toolbar("dump", &ours, &domain).unwrap()).unwrap();
        let path = |p: &Path| Some(p.to_string_lossy().to_string());

        assert_eq!(toolbar("has", &ours, &domain).unwrap(), "no");
        assert_eq!(toolbar("add", &ours, &domain).unwrap(), "ok");
        assert_eq!(toolbar("add", &ours, &domain).unwrap(), "ok", "第二次不重复加");
        let after = dump();
        assert_eq!(after.ids, ["com.apple.finder.BACK", "com.apple.finder.loc ", "com.apple.finder.loc ", "com.apple.finder.SWCH", "com.apple.finder.SRCH"]);
        assert_eq!(after.paths.get("1").cloned().flatten(), path(&other), "别的按钮不动");
        assert_eq!(after.paths.get("2").cloned().flatten(), path(&ours), "加在它后面");
        assert_eq!(toolbar("has", &ours, &domain).unwrap(), "yes");

        assert_eq!(toolbar("remove", &ours, &domain).unwrap(), "ok");
        let back = dump();
        assert_eq!(back.ids, ["com.apple.finder.BACK", "com.apple.finder.loc ", "com.apple.finder.SWCH", "com.apple.finder.SRCH"]);
        assert_eq!(back.paths.len(), 1);
        assert_eq!(back.paths.get("1").cloned().flatten(), path(&other));
        assert_eq!(toolbar("has", &ours, &domain).unwrap(), "no");

        // 我们的夹在两个别的中间（用户自己拖过位置）：去掉后后面那个往前挪一格，指向不变
        let third = dir.join("Third.app");
        std::fs::create_dir_all(&third).unwrap();
        let item = |p: &Path| format!("<dict><key>_CFURLString</key><string>file://{}/</string><key>_CFURLStringType</key><integer>15</integer></dict>", p.display().to_string().replace(' ', "%20"));
        std::fs::write(
            &seed,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>NSToolbar Configuration Browser</key><dict>
<key>TB Item Identifiers</key><array><string>com.apple.finder.BACK</string><string>com.apple.finder.loc </string><string>com.apple.finder.loc </string><string>com.apple.finder.loc </string><string>com.apple.finder.SRCH</string></array>
<key>TB Item Plists</key><dict><key>1</key>{}<key>2</key>{}<key>3</key>{}</dict>
</dict></dict></plist>"#,
                item(&other),
                item(&ours),
                item(&third)
            ),
        )
        .unwrap();
        assert!(Command::new("/usr/bin/defaults").args(["import", &domain]).arg(&seed).status().unwrap().success());
        assert_eq!(toolbar("has", &ours, &domain).unwrap(), "yes");
        assert_eq!(toolbar("remove", &ours, &domain).unwrap(), "ok");
        let middle = dump();
        assert_eq!(middle.ids, ["com.apple.finder.BACK", "com.apple.finder.loc ", "com.apple.finder.loc ", "com.apple.finder.SRCH"]);
        assert_eq!(middle.paths.get("1").cloned().flatten(), path(&other));
        assert_eq!(middle.paths.get("2").cloned().flatten(), path(&third), "后面那个往前挪了一格");

        let _ = Command::new("/usr/bin/defaults").args(["delete", &domain]).status();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 设访达自定义图标：浅色、深色各设一次，都设上了（包里多一个带资源分支的 Icon\r），两次的图标不一样。
    #[test]
    fn the_button_gets_the_custom_icon_for_each_appearance() {
        let dir = scratch("icon");
        let button = dir.join("Open in SSHL.app");
        let resources = button.join("Contents/Resources");
        std::fs::create_dir_all(&resources).unwrap();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/finder-button");
        std::fs::copy(assets.join("icon.icns"), resources.join("icon.icns")).unwrap();
        std::fs::copy(assets.join("icon-dark.icns"), resources.join("icon-dark.icns")).unwrap();
        let fork = || Command::new("/usr/bin/xattr").args(["-px", "com.apple.ResourceFork"]).arg(button.join("Icon\r")).output().unwrap().stdout;

        set_button_icon(&button, false).unwrap();
        let light = fork();
        assert!(!light.is_empty(), "浅色图标设上了");
        set_button_icon(&button, true).unwrap();
        let dark = fork();
        assert!(!dark.is_empty(), "深色图标设上了");
        assert_ne!(light, dark, "换成了另一套");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_are_quoted_for_the_shell() {
        assert_eq!(shell_quote(Path::new("/Applications/SSHL.app/x")), "'/Applications/SSHL.app/x'");
        assert_eq!(shell_quote(Path::new("/a/it's")), r"'/a/it'\''s'");
    }
}

//! 从 SSHL 外面打开终端的两个入口，都是让 SSHL「打开文件夹」（见 `open_request.rs`）：
//! 访达工具栏按钮「在 SSHL 中打开」（`crates/finder-button`）和命令行 `sshl 目录`（`assets/bin/sshl`）。
//! 两个都在 SSHL.app 里（scripts/bundle.sh 放进去），设置 →「终端」里显示 / 安装；`cargo run` 的开发版没有，那两行不显示。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 命令行装到这里（链接到 SSHL.app 里的脚本）：macOS 默认的 PATH 里就有它。
pub const CLI_LINK: &str = "/usr/local/bin/sshl";

/// 自己所在的 SSHL.app（`…/SSHL.app/Contents/MacOS/sshl`）。
fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.ancestors().nth(3)?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// 访达工具栏按钮。
pub fn finder_button() -> Option<PathBuf> {
    bundle().map(|app| app.join("Contents/Helpers/Open in SSHL.app")).filter(|path| path.is_dir())
}

/// 命令行脚本。
pub fn cli_script() -> Option<PathBuf> {
    bundle().map(|app| app.join("Contents/Resources/bin/sshl")).filter(|path| path.is_file())
}

/// 在访达里选中它（按住 ⌘ 从那儿拖到工具栏上）。
pub fn reveal(path: &Path) {
    if let Err(error) = Command::new("/usr/bin/open").arg("-R").arg(path).spawn() {
        tracing::warn!("reveal {}: {error}", path.display());
    }
}

/// `link` 指向的就是 `script`（装过了，而且是这一个 SSHL 的）。
pub fn cli_installed(script: &Path) -> bool {
    links_to(Path::new(CLI_LINK), script)
}

fn links_to(link: &Path, script: &Path) -> bool {
    matches!((std::fs::canonicalize(link), std::fs::canonicalize(script)), (Ok(a), Ok(b)) if a == b)
}

/// 装命令行：/usr/local/bin/sshl 链接到 SSHL.app 里的脚本（以前装的链接换掉；那儿是别的程序的文件就不动）。
/// 自己写不进去（/usr/local/bin 归 root）就用系统的授权框要管理员密码，同 VS Code 的「安装 code 命令」。
/// 返回 Ok(false) = 用户在授权框里取消了。会阻塞（授权框），别在界面线程上调。
pub fn install_cli(script: &Path) -> Result<bool, String> {
    let link = Path::new(CLI_LINK);
    match link_in_place(link, script) {
        Ok(()) => return Ok(true),
        Err(LinkError::Taken) => return Err(tr!("{} 已经有别的文件，没有覆盖", CLI_LINK).to_string()),
        Err(LinkError::Denied) => {}
    }
    let command = format!("mkdir -p /usr/local/bin && ln -sfn {} {}", shell_quote(script), shell_quote(link));
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
        dir
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

    #[test]
    fn paths_are_quoted_for_the_shell() {
        assert_eq!(shell_quote(Path::new("/Applications/SSHL.app/x")), "'/Applications/SSHL.app/x'");
        assert_eq!(shell_quote(Path::new("/a/it's")), r"'/a/it'\''s'");
    }
}

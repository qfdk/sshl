//! 访达工具栏上的「在 SSHL 中打开」按钮（同 OpenInTerminal-Lite）：设置 →「集成」把 `SSHL.app/Contents/Helpers/Open in SSHL.app`
//! 加到访达的工具栏上（它在 SSHL.app 包里面，按住 ⌘ 拖不上去）。点它时问访达最前面的窗口在哪个目录，
//! `open -a SSHL 目录` 交给 SSHL（在那儿开一个本机终端标签、切到前台，见 crates/app/src/open_request.rs），然后退出。
//! 只看当前目录、不看选中的东西（用户 2026-11-24：选中了子文件夹时开到了子文件夹里，「没有在当前目录」）。
//! 它在 SSHL.app 里面，系统把它算成 SSHL 的：第一次点时问的是「SSHL」能不能控制访达（SSHL 签名时带着 Apple Events 授权，
//! 见 scripts/sign.sh）。拒绝过、或者出了别的错，都弹框说清楚，不悄悄开到别处。
//!
//! 单独一个小程序、不链接 GPUI：点一下就要出结果，起得越快越好。

use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

/// 最前面窗口的目录；窗口没有目录（「最近使用」、搜索结果）或者没有窗口时是桌面（同访达新建文件夹的位置）。
const FINDER_FOLDER: &str = r#"
tell application "Finder"
	try
		set here to target of front Finder window
	on error
		set here to desktop
	end try
	return POSIX path of (here as alias)
end tell
"#;

/// errAEEventNotPermitted：没允许（或拒绝过）它控制访达。
const NOT_PERMITTED: i64 = -1743;

/// 被单独拷出 SSHL.app 时按它找 SSHL。
const SSHL_BUNDLE_ID: &str = "me.qfdk.sshl";

fn main() {
    objc2::rc::autoreleasepool(|_| match run_script(FINDER_FOLDER) {
        Ok(path) => open_in_sshl(Path::new(&path)),
        Err(NOT_PERMITTED) => explain_permission(),
        Err(code) => explain_error(code),
    });
}

fn open_in_sshl(dir: &Path) {
    let mut open = Command::new("/usr/bin/open");
    match sshl_app() {
        Some(app) => open.arg("-a").arg(app),
        None => open.arg("-b").arg(SSHL_BUNDLE_ID),
    };
    if let Err(error) = open.arg(dir).status() {
        eprintln!("open-in-sshl: {error}");
    }
}

/// 自己在 `SSHL.app/Contents/Helpers/Open in SSHL.app` 里：交给外面那个 SSHL（装在哪都行，开发时打的包也对得上自己）。
fn sshl_app() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // open-in-sshl → MacOS → Contents → Open in SSHL.app → Helpers → Contents → SSHL.app
    let app = exe.ancestors().nth(6)?;
    (app.extension()? == "app" && app.join("Contents/MacOS/sshl").is_file()).then(|| app.to_path_buf())
}

/// 不让控制访达：说去哪儿打开，给一个直达那一页的按钮。
fn explain_permission() {
    let (title, message, open_settings, cancel) = if prefers_chinese() {
        ("无法读取访达的当前目录", "请在「系统设置 → 隐私与安全性 → 自动化」里允许「SSHL」控制「访达」。", "打开系统设置", "取消")
    } else {
        ("Can't read Finder's current folder", "Allow SSHL to control Finder in System Settings → Privacy & Security → Automation.", "Open System Settings", "Cancel")
    };
    if alert(title, message, &[cancel, open_settings]).as_deref() == Some(open_settings) {
        let _ = Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Automation").status();
    }
}

/// 问访达出了别的错：照实说（带错误号），不开到别的目录去。
fn explain_error(code: i64) {
    let (title, message, ok) = if prefers_chinese() {
        ("无法读取访达的当前目录", format!("访达返回了错误 {code}。"), "好")
    } else {
        ("Can't read Finder's current folder", format!("Finder returned error {code}."), "OK")
    };
    alert(title, &message, &[ok]);
}

/// 弹一个框，返回点的按钮（最后一个是默认按钮）。
fn alert(title: &str, message: &str, buttons: &[&str]) -> Option<String> {
    // 没有 NSApplication 时弹不出框：自己建一个，不进程序坞（Info.plist 里也是 LSUIElement）
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: bool = msg_send![app, setActivationPolicy: 1isize];
    }
    let quote = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let list = buttons.iter().map(|b| quote(b)).collect::<Vec<_>>().join(", ");
    let default = quote(buttons.last()?);
    let script = format!("activate\nreturn button returned of (display alert {} message {} as critical buttons {{{list}}} default button {default})", quote(title), quote(message));
    run_script(&script).ok()
}

/// 系统首选语言是中文（同 SSHL 的「跟随系统」）。
fn prefers_chinese() -> bool {
    unsafe {
        let languages: *mut AnyObject = msg_send![class!(NSLocale), preferredLanguages];
        let first: *mut AnyObject = msg_send![languages, firstObject];
        string(first).is_some_and(|lang| lang.starts_with("zh"))
    }
}

/// 跑一段 AppleScript，返回结果的文字；出错返回错误号。
fn run_script(source: &str) -> Result<String, i64> {
    unsafe {
        let source = ns_string(source);
        let script: *mut AnyObject = msg_send![class!(NSAppleScript), alloc];
        let script: *mut AnyObject = msg_send![script, initWithSource: source];
        let mut error: *mut AnyObject = ptr::null_mut();
        let result: *mut AnyObject = msg_send![script, executeAndReturnError: &mut error as *mut *mut AnyObject];
        if result.is_null() {
            let code = if error.is_null() {
                0
            } else {
                let number: *mut AnyObject = msg_send![error, objectForKey: ns_string("NSAppleScriptErrorNumber")];
                if number.is_null() { 0 } else { msg_send![number, longLongValue] }
            };
            return Err(code);
        }
        let text: *mut AnyObject = msg_send![result, stringValue];
        string(text).ok_or(0)
    }
}

unsafe fn ns_string(text: &str) -> *mut AnyObject {
    let text = CString::new(text).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

unsafe fn string(object: *mut AnyObject) -> Option<String> {
    if object.is_null() {
        return None;
    }
    let utf8: *const c_char = unsafe { msg_send![object, UTF8String] };
    (!utf8.is_null()).then(|| unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned())
}

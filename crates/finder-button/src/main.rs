//! 访达工具栏上的「在 SSHL 中打开」按钮（同 OpenInTerminal-Lite）：按住 ⌘ 把 `SSHL.app/Contents/Helpers/Open in SSHL.app`
//! 拖到访达窗口的工具栏上。点它时问访达在哪个目录（选中了一个文件夹就用那个文件夹，否则最前面窗口的目录），
//! `open -a SSHL 目录` 交给 SSHL（在那儿开一个本机终端标签、切到前台，见 crates/app/src/open_request.rs），然后退出。
//! 第一次点时系统问要不要允许它控制访达；拒绝过的话弹框说去哪儿打开。
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

/// 选中的一个文件夹（或磁盘），否则最前面窗口的目录；窗口没有目录（「最近使用」、搜索结果）或者没有窗口时是桌面。
const FINDER_FOLDER: &str = r#"
tell application "Finder"
	set chosen to missing value
	set picked to selection as list
	if (count of picked) is 1 then
		if class of (item 1 of picked) is in {folder, disk} then set chosen to item 1 of picked
	end if
	if chosen is missing value then
		try
			set chosen to target of front Finder window
		on error
			set chosen to desktop
		end try
	end if
	return POSIX path of (chosen as alias)
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
        // 访达没在跑这类意外：照样开一个终端，从主目录开始
        Err(code) => {
            eprintln!("open-in-sshl: Finder error {code}");
            if let Some(home) = std::env::var_os("HOME") {
                open_in_sshl(Path::new(&home));
            }
        }
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
        ("无法读取访达的当前目录", "请在「系统设置 → 隐私与安全性 → 自动化」里允许「在 SSHL 中打开」控制「访达」。", "打开系统设置", "取消")
    } else {
        ("Can't read Finder's current folder", "Allow “Open in SSHL” to control Finder in System Settings → Privacy & Security → Automation.", "Open System Settings", "Cancel")
    };
    // 没有 NSApplication 时弹不出框：自己建一个，不进程序坞（Info.plist 里也是 LSUIElement）
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: bool = msg_send![app, setActivationPolicy: 1isize];
    }
    let script = format!(
        "activate\nreturn button returned of (display alert \"{title}\" message \"{message}\" as critical buttons {{\"{cancel}\", \"{open_settings}\"}} default button \"{open_settings}\" cancel button \"{cancel}\")"
    );
    if run_script(&script).as_deref() == Ok(open_settings) {
        let _ = Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Automation").status();
    }
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

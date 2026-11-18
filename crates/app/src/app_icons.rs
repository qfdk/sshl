//! 本机进程列表里的应用图标（同 Mole、活动监视器）：进程属于哪个 .app 就用它的图标。
//! 图标向系统要（`NSWorkspace iconForFile:`），在后台线程按 2 倍像素画成 PNG 交给 GPUI；同一个应用只取一次。
//! 不在 .app 里的进程不用系统的通用可执行文件图标（16–20pt 下是一个个黑方块，列表里大半是这种，整列很重），界面自己画淡色的线条图标；
//! 系统框架里那些没有自己图标的 .app（identityservicesd、storeuid）系统给的是通用的应用占位图（一块灰格子），也当没有图标。

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;

use objc2::encode::{Encode, Encoding};
use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};

/// 显示多大（pt）。16、20 用户都嫌小。
pub const ICON_SIZE: f32 = 22.;
/// 画成多少像素：2 倍屏上正好一比一。GPUI 的位图缩小时没有 mipmap，大图缩下来发毛。
const ICON_PIXELS: usize = 44;

/// 程序路径属于哪个应用：最外层的 `xxx.app`。Chrome 的 Helper 在 Google Chrome.app 里面的 Frameworks 里，也算 Chrome。
/// 不是绝对路径、不在 .app 里的 None。
pub fn bundle_of(program: &str) -> Option<&str> {
    if !program.starts_with('/') {
        return None;
    }
    let end = program.find(".app/").map(|at| at + ".app".len()).or_else(|| program.ends_with(".app").then_some(program.len()))?;
    Some(&program[..end])
}

/// 一个进程用哪个应用的图标（应用的路径），不在应用里的 None。程序路径优先用 ps 的 comm（`program`，带空格的路径也完整），
/// 没有时用命令行的第一个词。
pub fn icon_key(program: Option<&str>, command: &str) -> Option<String> {
    let path = program.unwrap_or_else(|| command.split(' ').next().unwrap_or(""));
    bundle_of(path).map(str::to_string)
}

/// 系统自带的后台程序（不是用户自己跑的命令行程序）：放在这些目录下面。`/usr/bin`、`/bin` 不算（ssh、vim 这些是用户敲的）。
pub fn is_system_program(path: &str) -> bool {
    ["/System/", "/usr/libexec/", "/usr/sbin/", "/sbin/", "/usr/lib/", "/Library/"].iter().any(|dir| path.starts_with(dir))
}

/// 这个应用（`icon_key` 给的路径）的图标，画成 PNG。没有自己的图标（系统给的是通用占位图）、取不到的 None。
/// 阻塞（几毫秒），放在后台线程里调。
pub fn icon_png(app: &str) -> Option<Vec<u8>> {
    if !has_own_icon(app) {
        return None;
    }
    draw_icon(app, ICON_PIXELS)
}

/// 应用有没有自己的图标：Info.plist 里写的图标文件真的在（`CFBundleIconFile`，可以不带 .icns），或者写了图标名、有 Assets.car
/// （`CFBundleIconName`）。都没有的系统给通用占位图（一块灰格子）：identityservicesd、WindowManager 没写，storeuid 写了文件却不在。
/// 不能拿画出来的图和通用图标比：同一张占位图，44px 时系统挑的那一版和「app 类型的图标」不一样，88px 时又一样。
fn has_own_icon(app: &str) -> bool {
    let resources = Path::new(app).join("Contents/Resources");
    let icon_file = info_string(app, c"CFBundleIconFile");
    let file_exists = icon_file.is_some_and(|file| resources.join(&file).exists() || resources.join(format!("{file}.icns")).exists());
    file_exists || (info_string(app, c"CFBundleIconName").is_some() && resources.join("Assets.car").exists())
}

/// 应用 Info.plist 里一个字符串（二进制、XML 格式的 plist 都认：交给 NSBundle 读）。
fn info_string(app: &str, key: &CStr) -> Option<String> {
    let path = CString::new(app).ok()?;
    autoreleasepool(|_| {
        // SAFETY: NSBundle / NSString 公开的方法，任何线程都能用；值在这个 autoreleasepool 里有效，拷出来再离开
        unsafe {
            let path: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: path.as_ptr()];
            let bundle: *mut AnyObject = msg_send![class!(NSBundle), bundleWithPath: path];
            if bundle.is_null() {
                return None;
            }
            let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: key.as_ptr()];
            let value: *mut AnyObject = msg_send![bundle, objectForInfoDictionaryKey: key];
            let is_string: bool = !value.is_null() && {
                let yes: Bool = msg_send![value, isKindOfClass: class!(NSString)];
                yes.as_bool()
            };
            if !is_string {
                return None;
            }
            let utf8: *const c_char = msg_send![value, UTF8String];
            (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned()).filter(|value| !value.is_empty())
        }
    })
}

/// 应用在访达里显示的名字（跟着系统语言：WeChat.app 是「微信」），不带 .app。阻塞，放在后台线程里调。
pub fn display_name(app: &str) -> Option<String> {
    let path = CString::new(app).ok()?;
    autoreleasepool(|_| {
        // SAFETY: NSFileManager 公开的方法，任何线程都能用；返回的字符串在这个 autoreleasepool 里有效，拷出来再离开
        unsafe {
            let manager: *mut AnyObject = msg_send![class!(NSFileManager), defaultManager];
            let file: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: path.as_ptr()];
            let name: *mut AnyObject = msg_send![manager, displayNameAtPath: file];
            if name.is_null() {
                return None;
            }
            let utf8: *const c_char = msg_send![name, UTF8String];
            if utf8.is_null() {
                return None;
            }
            let name = CStr::from_ptr(utf8).to_string_lossy().into_owned();
            Some(name.strip_suffix(".app").map(str::to_string).unwrap_or(name)).filter(|name| !name.is_empty())
        }
    })
}

/// 系统给这个文件的图标，画成 `pixels` × `pixels` 的 PNG。
fn draw_icon(path: &str, pixels: usize) -> Option<Vec<u8>> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    // SAFETY: 和 CGPoint 一样的内存布局
    unsafe impl Encode for Point {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }
    // SAFETY: 和 CGSize 一样的内存布局
    unsafe impl Encode for Size {
        const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }
    // SAFETY: 和 CGRect 一样的内存布局
    unsafe impl Encode for Rect {
        const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
    }
    /// NSCompositingOperationSourceOver、NSBitmapImageFileTypePNG。
    const SOURCE_OVER: usize = 2;
    const PNG: usize = 4;

    let path = CString::new(path).ok()?;
    let side = pixels as f64;
    autoreleasepool(|_| {
        // SAFETY: 都是 AppKit 公开的方法，参数类型照头文件；rep 是自己 alloc 的，用完 release。
        // NSWorkspace、离屏位图（自己的 NSGraphicsContext，只在本线程设成当前）在后台线程上可以用。
        unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let file: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: path.as_ptr()];
            let icon: *mut AnyObject = msg_send![workspace, iconForFile: file];
            if icon.is_null() {
                return None;
            }
            let color_space: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"NSDeviceRGBColorSpace".as_ptr()];
            let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
            let rep: *mut AnyObject = msg_send![
                rep,
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut c_char>(),
                pixelsWide: pixels as isize,
                pixelsHigh: pixels as isize,
                bitsPerSample: 8_isize,
                samplesPerPixel: 4_isize,
                hasAlpha: Bool::YES,
                isPlanar: Bool::NO,
                colorSpaceName: color_space,
                bytesPerRow: 0_isize,
                bitsPerPixel: 0_isize
            ];
            if rep.is_null() {
                return None;
            }
            // 位图的「点」= 像素：图标按 1 倍挑最合适的那一版画进来
            let _: () = msg_send![rep, setSize: Size { width: side, height: side }];
            let context: *mut AnyObject = msg_send![class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: rep];
            let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
            let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: context];
            let whole = Rect { origin: Point { x: 0., y: 0. }, size: Size { width: side, height: side } };
            let all = Rect { origin: Point { x: 0., y: 0. }, size: Size { width: 0., height: 0. } };
            let _: () = msg_send![icon, drawInRect: whole, fromRect: all, operation: SOURCE_OVER, fraction: 1.0_f64];
            let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
            let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let data: *mut AnyObject = msg_send![rep, representationUsingType: PNG, properties: properties];
            let png = if data.is_null() {
                None
            } else {
                let bytes: *const c_void = msg_send![data, bytes];
                let length: usize = msg_send![data, length];
                (!bytes.is_null()).then(|| std::slice::from_raw_parts(bytes.cast::<u8>(), length).to_vec())
            };
            let _: () = msg_send![rep, release];
            png
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processes_belong_to_their_outermost_app() {
        let chrome = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)";
        assert_eq!(bundle_of(chrome), Some("/Applications/Google Chrome.app"), "Helper 算 Chrome");
        assert_eq!(bundle_of("/System/Applications/Calendar.app/Contents/MacOS/Calendar"), Some("/System/Applications/Calendar.app"));
        assert_eq!(bundle_of("/Applications/Foo.app"), Some("/Applications/Foo.app"));
        assert_eq!(bundle_of("/usr/libexec/logd"), None);
        assert_eq!(bundle_of("/Users/me/my.application/bin/x"), None, ".app 后面得是目录分隔");
        assert_eq!(bundle_of("claude"), None, "不是路径");
    }

    #[test]
    fn the_program_path_picks_the_icon() {
        let chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        assert_eq!(icon_key(Some(chrome), &format!("{chrome} --type=renderer")).as_deref(), Some("/Applications/Google Chrome.app"));
        assert_eq!(icon_key(None, "/Applications/Docker.app/Contents/MacOS/com.docker.backend services").as_deref(), Some("/Applications/Docker.app"));
        assert_eq!(icon_key(None, "postgres: checkpointer"), None, "命令行改写过、找不到程序的");
        assert_eq!(icon_key(Some("/usr/sbin/sshd"), "/usr/sbin/sshd -D"), None, "不在应用里");
    }

    /// PNG 头里的宽高（IHDR 紧跟在 8 字节签名和块长度、类型后面）。
    fn png_size(png: &[u8]) -> (u32, u32) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let at = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
        (at(16), at(20))
    }

    /// 真向系统要：画得出来，正好 2 倍大（测试跑在后台线程上，也验证了不用主线程）；不同的应用图不一样。
    #[test]
    fn icons_are_drawn_at_twice_their_size() {
        let calculator = icon_png("/System/Applications/Calculator.app").expect("计算器的图标");
        assert_eq!(png_size(&calculator), (44, 44));
        assert_eq!(ICON_PIXELS as f32, ICON_SIZE * 2.);
        let calendar = icon_png("/System/Applications/Calendar.app").expect("日历的图标");
        assert_ne!(calculator, calendar);
    }

    /// 系统框架里没有自己图标的 .app：系统给的是通用占位图，当没有（界面画齿轮）。
    #[test]
    fn placeholder_icons_count_as_none() {
        for app in [
            "/System/Library/PrivateFrameworks/IDS.framework/identityservicesd.app",
            "/System/Library/CoreServices/WindowManager.app",
            "/System/Library/PrivateFrameworks/CommerceKit.framework/Versions/A/Resources/storeuid.app",
        ] {
            if Path::new(app).exists() {
                assert!(!has_own_icon(app), "{app}");
                assert_eq!(icon_png(app), None, "{app}");
            }
        }
        for app in ["/System/Library/CoreServices/Finder.app", "/System/Applications/Calculator.app", "/System/Library/CoreServices/ControlCenter.app"] {
            assert!(has_own_icon(app), "{app} 有自己的图标");
        }
    }

    #[test]
    fn system_programs_are_told_from_command_line_ones() {
        assert!(is_system_program("/System/Library/Frameworks/LocalAuthentication.framework/Support/coreauthd"));
        assert!(is_system_program("/usr/libexec/logd"));
        assert!(is_system_program("/Library/PrivilegedHelperTools/com.docker.vmnetd"));
        assert!(!is_system_program("/opt/homebrew/bin/node"));
        assert!(!is_system_program("/usr/bin/ssh"), "ssh、vim 是用户敲的");
        assert!(!is_system_program("claude"));
    }

    /// 显示名：访达里看到的那个，不带 .app（中文系统上计算器是「计算器」，英文是 Calculator）。
    #[test]
    fn apps_have_their_finder_names() {
        let name = display_name("/System/Applications/Calculator.app").expect("计算器的名字");
        assert!(name == "计算器" || name == "Calculator", "{name}");
        assert_eq!(display_name("/no/such/Thing.app").as_deref().unwrap_or("Thing"), "Thing", "不存在的：没有名字或者就是文件名");
    }
}

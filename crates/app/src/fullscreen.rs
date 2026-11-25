//! 原生全屏里关窗口（红点、没有标签时 ⌘W）。关窗口只是把应用藏起来（`Workspace::close_window`），
//! 可 macOS 不藏原生全屏里的应用：`hide:` 直接被忽略，红点看着没反应（用户 2026-11-25 报）。
//! 退出全屏的动画途中藏也不行：系统先报「已隐藏」，动画结束时又把窗口拿回来。
//! 所以先退出全屏，等系统发「已退出全屏」再藏（探针实测：动画约 0.55 秒，途中窗口不失焦，`closed` 不会被提前用掉）。
//! GPUI 不把这个时刻往上报（它的窗口代理只拿来挪红绿灯），这里直接听 AppKit 的通知。
//! 回来时窗口不再是全屏（同 Ghostty：全屏的窗口关掉，再开的是普通窗口）。

use std::cell::Cell;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

/// 退出失败时 AppKit 只告诉窗口代理（GPUI 占着），不发通知，请求会一直留着。
/// 超过这么久才退出的不算，免得之后用户自己退出全屏时应用突然藏起来。
const WAIT: Duration = Duration::from_secs(5);

thread_local! {
    static HIDE_REQUESTED: Cell<Option<Instant>> = const { Cell::new(None) };
    static LISTENING: Cell<bool> = const { Cell::new(false) };
}

/// 退出全屏，退完把应用藏起来。
pub fn hide_after_exit(window: &mut gpui::Window) {
    listen();
    HIDE_REQUESTED.set(Some(Instant::now()));
    window.toggle_fullscreen();
}

fn still_wanted(requested: Option<Instant>, now: Instant) -> bool {
    requested.is_some_and(|at| now.duration_since(at) < WAIT)
}

/// 第一次用到时登记一次，之后一直听着（通知中心留着这份登记，不注销）。
fn listen() {
    if LISTENING.replace(true) {
        return;
    }
    let exited = RcBlock::new(|_: NonNull<AnyObject>| {
        if still_wanted(HIDE_REQUESTED.take(), Instant::now()) {
            unsafe {
                let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app, hide: std::ptr::null_mut::<AnyObject>()];
            }
        }
    });
    unsafe {
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let name: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"NSWindowDidExitFullScreenNotification".as_ptr()];
        let main_queue: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue];
        let _: *mut AnyObject = msg_send![
            center,
            addObserverForName: name,
            object: std::ptr::null_mut::<AnyObject>(),
            queue: main_queue,
            usingBlock: &*exited
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_recent_request_hides_the_app() {
        let at = Instant::now();
        assert!(still_wanted(Some(at), at + Duration::from_millis(550)));
        // 退出失败留下的旧请求：之后用户自己退出全屏时不藏
        assert!(!still_wanted(Some(at), at + Duration::from_secs(60)));
        assert!(!still_wanted(None, at));
    }
}

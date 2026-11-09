//! 启动时窗口开在哪块屏幕：鼠标此刻所在的那块（多屏时在哪块屏幕上点开就铺满哪块）。

use std::rc::Rc;

use gpui::{App, DisplayId, PlatformDisplay};

/// 鼠标所在的屏幕；读不到时返回 None（调用方退回主屏）。
pub fn display_under_mouse(cx: &App) -> Option<Rc<dyn PlatformDisplay>> {
    let id = DisplayId::new(u64::from(mouse::display_id()?));
    cx.displays().into_iter().find(|display| display.id() == id)
}

#[cfg(target_os = "macos")]
mod mouse {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        fn CGGetDisplaysWithPoint(point: CGPoint, max: u32, displays: *mut u32, count: *mut u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    /// 鼠标所在屏幕的 CGDirectDisplayID（GPUI 的 DisplayId 就是它）。
    pub fn display_id() -> Option<u32> {
        // SAFETY: CGEventCreate(NULL) 取一个反映当前鼠标位置的事件，用完 CFRelease；
        // CGGetDisplaysWithPoint 只往给定的缓冲区写最多 1 个 id。
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let at = CGEventGetLocation(event);
            CFRelease(event);
            let (mut id, mut count) = (0u32, 0u32);
            (CGGetDisplaysWithPoint(at, 1, &mut id, &mut count) == 0 && count > 0).then_some(id)
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod mouse {
    pub fn display_id() -> Option<u32> {
        None
    }
}

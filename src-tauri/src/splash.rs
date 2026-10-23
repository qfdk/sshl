// 原生开屏层。WKWebView 冷启动要等 WebContent 进程起来才画第一帧，窗口出现后有约 0.7s
// 只有背景色（白屏）；这段时间网页里的任何东西都还画不出来，只能用原生视图盖住。
// setup 时在 WebView 上方盖一层：背景色 + 呼吸的应用图标；前端第一屏提交后调 app_ready 淡出移除。
// 前端起不来（脚本异常）时 FALLBACK 后也会移除，不会永远挡住界面。

use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};

const FALLBACK: Duration = Duration::from_secs(8);

pub fn show<R: Runtime>(app: &tauri::App<R>) {
    #[cfg(target_os = "macos")]
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(ns_window) = window.ns_window() {
            mac::show(ns_window.cast());
        }
    }
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FALLBACK).await;
        dismiss(&handle);
    });
}

pub fn dismiss<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(mac::dismiss);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// 前端第一屏画出后调用（ui/src/main.tsx）。
#[tauri::command]
pub fn app_ready(app: AppHandle) {
    dismiss(&app);
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::Mutex;

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, msg_send_id, sel};
    use objc2_foundation::{NSData, NSNumber, NSPoint, NSRect, NSSize, NSString};

    // 与侧栏 logo 同一张图；不用 applicationIconImage，系统图标渲染会给它加光环。
    const ICON_PNG: &[u8] = include_bytes!("../../icon.png");
    const ICON_SIZE: f64 = 96.0;
    const FADE_SECONDS: f64 = 0.2;

    // 开屏层只在主线程上创建和移除；存裸指针（usize）只为能放进 static。
    static COVER: Mutex<Option<usize>> = Mutex::new(None);

    pub fn show(ns_window: *mut AnyObject) {
        unsafe {
            let ns_window: &AnyObject = &*ns_window;
            let Some(content): Option<Retained<AnyObject>> = msg_send_id![ns_window, contentView] else {
                return;
            };
            let bounds: NSRect = msg_send![&content, bounds];

            // NSBox(custom) 直接用 NSColor 填底色，省去给 CALayer 设 CGColorRef。
            // 颜色与 tauri.conf.json 的窗口 backgroundColor (#f3f4f6) 一致。
            let alloc: Allocated<AnyObject> = msg_send_id![class!(NSBox), alloc];
            let cover: Retained<AnyObject> = msg_send_id![alloc, initWithFrame: bounds];
            let color: Retained<AnyObject> = msg_send_id![
                class!(NSColor),
                colorWithSRGBRed: 243.0f64 / 255.0,
                green: 244.0f64 / 255.0,
                blue: 246.0f64 / 255.0,
                alpha: 1.0f64
            ];
            let _: () = msg_send![&cover, setBoxType: 4usize]; // NSBoxCustom
            let _: () = msg_send![&cover, setTitlePosition: 0usize]; // NSNoTitle
            let _: () = msg_send![&cover, setBorderWidth: 0.0f64];
            let _: () = msg_send![&cover, setContentViewMargins: NSSize::new(0.0, 0.0)];
            let _: () = msg_send![&cover, setFillColor: &*color];
            let _: () = msg_send![&cover, setWantsLayer: true];
            let _: () = msg_send![&cover, setAutoresizingMask: 2usize | 16usize]; // 宽高随窗口

            let data = NSData::with_bytes(ICON_PNG);
            let alloc: Allocated<AnyObject> = msg_send_id![class!(NSImage), alloc];
            let image: Option<Retained<AnyObject>> = msg_send_id![alloc, initWithData: &*data];
            if let Some(image) = image {
                let frame = NSRect::new(
                    NSPoint::new(
                        (bounds.size.width - ICON_SIZE) / 2.0,
                        (bounds.size.height - ICON_SIZE) / 2.0,
                    ),
                    NSSize::new(ICON_SIZE, ICON_SIZE),
                );
                let alloc: Allocated<AnyObject> = msg_send_id![class!(NSImageView), alloc];
                let icon: Retained<AnyObject> = msg_send_id![alloc, initWithFrame: frame];
                let _: () = msg_send![&icon, setImage: &*image];
                let _: () = msg_send![&icon, setImageScaling: 3usize]; // ProportionallyUpOrDown
                let _: () = msg_send![&icon, setAutoresizingMask: 1usize | 4 | 8 | 32]; // 四边留白弹性 = 居中
                let _: () = msg_send![&icon, setWantsLayer: true];
                breathe(&icon);
                let inner: Retained<AnyObject> = msg_send_id![&cover, contentView];
                let _: () = msg_send![&inner, addSubview: &*icon];
            }

            // NSWindowAbove + relativeTo nil = 放到最上层，盖住 WebView。
            let _: () = msg_send![
                &content,
                addSubview: &*cover,
                positioned: 1isize,
                relativeTo: None::<&AnyObject>
            ];
            *COVER.lock().unwrap() = Some(Retained::into_raw(cover) as usize);
        }
    }

    /// 图标透明度往复，提示正在加载。
    unsafe fn breathe(view: &AnyObject) {
        let Some(layer): Option<Retained<AnyObject>> = msg_send_id![view, layer] else {
            return;
        };
        let key = NSString::from_str("opacity");
        let anim: Retained<AnyObject> = msg_send_id![class!(CABasicAnimation), animationWithKeyPath: &*key];
        let _: () = msg_send![&anim, setFromValue: &*NSNumber::new_f64(1.0)];
        let _: () = msg_send![&anim, setToValue: &*NSNumber::new_f64(0.45)];
        let _: () = msg_send![&anim, setDuration: 0.8f64];
        let _: () = msg_send![&anim, setAutoreverses: true];
        let _: () = msg_send![&anim, setRepeatCount: f32::INFINITY];
        let name = NSString::from_str("easeInEaseOut");
        let timing: Retained<AnyObject> = msg_send_id![class!(CAMediaTimingFunction), functionWithName: &*name];
        let _: () = msg_send![&anim, setTimingFunction: &*timing];
        let _: () = msg_send![&layer, addAnimation: &*anim, forKey: &*key];
    }

    pub fn dismiss() {
        let Some(ptr) = COVER.lock().unwrap().take() else {
            return;
        };
        unsafe {
            let cover = Retained::from_raw(ptr as *mut AnyObject).unwrap();
            let context = class!(NSAnimationContext);
            let _: () = msg_send![context, beginGrouping];
            let current: Retained<AnyObject> = msg_send_id![context, currentContext];
            let _: () = msg_send![&current, setDuration: FADE_SECONDS];
            let animator: Retained<AnyObject> = msg_send_id![&cover, animator];
            let _: () = msg_send![&animator, setAlphaValue: 0.0f64];
            let _: () = msg_send![context, endGrouping];
            // 延迟执行会持有 cover 直到移除，这里放掉自己的那份引用即可。
            let _: () = msg_send![
                &cover,
                performSelector: sel!(removeFromSuperview),
                withObject: None::<&AnyObject>,
                afterDelay: FADE_SECONDS
            ];
        }
    }
}

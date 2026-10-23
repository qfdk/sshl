// 原生开屏层。WKWebView 冷启动要等 WebContent 进程起来才画第一帧，窗口出现后有约 0.7s
// 只有背景色（白屏）；这段时间网页里的任何东西都还画不出来，只能用原生视图盖住。
// setup 时在 WebView 上方盖一层：背景色 + 矢量绘制的 logo（光标闪烁、SSHL 逐字打出）；
// 前端第一屏提交后调 app_ready 淡出移除。前端起不来（脚本异常）时 FALLBACK 后也会移除。

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
    use std::ffi::c_void;
    use std::sync::Mutex;

    use objc2::encode::{Encoding, RefEncode};
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, msg_send_id, sel};
    use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};

    /// logo 按 icon.png 的卡片坐标（824 单位见方，左上为原点）重画成矢量：位图缩到开屏尺寸会发虚。
    /// 字体用 Menlo-Bold——icon 里的 DejaVu Sans Mono 与它同源，字形一致。
    const UNITS: f64 = 824.0;
    const CARD: f64 = 88.0;
    const CARD_RADIUS: f64 = 180.0;
    const CARD_RGB: (u8, u8, u8) = (0x23, 0x26, 0x2D);
    const WHITE_RGB: (u8, u8, u8) = (0xF5, 0xF7, 0xFA);
    const GREEN_RGB: (u8, u8, u8) = (0x2F, 0xDA, 0x45);
    const BACKGROUND_RGB: (u8, u8, u8) = (0xF3, 0xF4, 0xF6); // tauri.conf.json 的窗口 backgroundColor
    const PROMPT_SIZE: f64 = 168.0;
    const PROMPT_ORIGIN: (f64, f64) = (93.0, 341.0); // (左, 基线)
    const CURSOR: (f64, f64, f64, f64) = (260.0, 218.0, 54.0, 124.0); // (左, 上, 宽, 高)
    const TITLE_SIZE: f64 = 197.0;
    const TITLE_ORIGIN: (f64, f64) = (176.0, 628.0);
    const TITLE: &str = "SSHL";
    // 逐字出现：首字延迟与字间隔（秒）。冷启动白屏约 0.7s，0.4s 内打完。
    const TYPE_DELAY: f64 = 0.1;
    const TYPE_INTERVAL: f64 = 0.07;
    const FADE_SECONDS: f64 = 0.2;

    #[repr(C)]
    struct CGColor {
        _private: [u8; 0],
    }

    unsafe impl RefEncode for CGColor {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[]));
    }

    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {
        fn CACurrentMediaTime() -> f64;
    }

    // 开屏层只在主线程上创建和移除；存裸指针（usize）只为能放进 static。
    static COVER: Mutex<Option<usize>> = Mutex::new(None);

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    unsafe fn ns_color((r, g, b): (u8, u8, u8)) -> Retained<AnyObject> {
        msg_send_id![
            class!(NSColor),
            colorWithSRGBRed: r as f64 / 255.0,
            green: g as f64 / 255.0,
            blue: b as f64 / 255.0,
            alpha: 1.0f64
        ]
    }

    // CGColor 归 NSColor 所有，图层的 setter 会自己 retain，所以 NSColor 活到 setter 调完即可。
    unsafe fn set_background(layer: &AnyObject, rgb: (u8, u8, u8)) {
        let color = ns_color(rgb);
        let cg: *const CGColor = msg_send![&color, CGColor];
        let _: () = msg_send![layer, setBackgroundColor: cg];
    }

    unsafe fn set_foreground(layer: &AnyObject, rgb: (u8, u8, u8)) {
        let color = ns_color(rgb);
        let cg: *const CGColor = msg_send![&color, CGColor];
        let _: () = msg_send![layer, setForegroundColor: cg];
    }

    pub fn show(ns_window: *mut AnyObject) {
        unsafe {
            let ns_window: &AnyObject = &*ns_window;
            let Some(content): Option<Retained<AnyObject>> = msg_send_id![ns_window, contentView] else {
                return;
            };
            let bounds: NSRect = msg_send![&content, bounds];
            let scale: f64 = msg_send![ns_window, backingScaleFactor];

            // NSBox(custom) 直接用 NSColor 填底色。
            let alloc: Allocated<AnyObject> = msg_send_id![class!(NSBox), alloc];
            let cover: Retained<AnyObject> = msg_send_id![alloc, initWithFrame: bounds];
            let _: () = msg_send![&cover, setBoxType: 4usize]; // NSBoxCustom
            let _: () = msg_send![&cover, setTitlePosition: 0usize]; // NSNoTitle
            let _: () = msg_send![&cover, setBorderWidth: 0.0f64];
            let _: () = msg_send![&cover, setContentViewMargins: NSSize::new(0.0, 0.0)];
            let _: () = msg_send![&cover, setFillColor: &*ns_color(BACKGROUND_RGB)];
            let _: () = msg_send![&cover, setWantsLayer: true];
            let _: () = msg_send![&cover, setAutoresizingMask: 2usize | 16usize]; // 宽高随窗口

            let frame = rect(
                (bounds.size.width - CARD) / 2.0,
                (bounds.size.height - CARD) / 2.0,
                CARD,
                CARD,
            );
            let alloc: Allocated<AnyObject> = msg_send_id![class!(NSView), alloc];
            let logo: Retained<AnyObject> = msg_send_id![alloc, initWithFrame: frame];
            let _: () = msg_send![&logo, setAutoresizingMask: 1usize | 4 | 8 | 32]; // 四边留白弹性 = 居中
            let _: () = msg_send![&logo, setWantsLayer: true];
            let layer: Option<Retained<AnyObject>> = msg_send_id![&logo, layer];
            if let Some(layer) = layer {
                draw_logo(&layer, scale);
            }
            let inner: Retained<AnyObject> = msg_send_id![&cover, contentView];
            let _: () = msg_send![&inner, addSubview: &*logo];

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

    unsafe fn draw_logo(root: &AnyObject, scale: f64) {
        let k = CARD / UNITS;
        // icon 坐标是左上原点、y 向下；图层是左下原点、y 向上。
        let frame = |x: f64, top: f64, width: f64, height: f64| rect(x * k, CARD - (top + height) * k, width * k, height * k);

        let card: Retained<AnyObject> = msg_send_id![class!(CALayer), layer];
        let _: () = msg_send![&card, setFrame: rect(0.0, 0.0, CARD, CARD)];
        let _: () = msg_send![&card, setCornerRadius: CARD_RADIUS * k];
        set_background(&card, CARD_RGB);
        let _: () = msg_send![root, addSublayer: &*card];

        let (x, top, width, height) = CURSOR;
        let cursor: Retained<AnyObject> = msg_send_id![class!(CALayer), layer];
        let _: () = msg_send![&cursor, setFrame: frame(x, top, width, height)];
        let _: () = msg_send![&cursor, setCornerRadius: 8.0 * k];
        set_background(&cursor, WHITE_RGB);
        blink(&cursor);
        let _: () = msg_send![&card, addSublayer: &*cursor];

        let font_name = NSString::from_str("Menlo-Bold");
        let glyph = |text: &str, size: f64, (left, baseline): (f64, f64), rgb| -> Option<Retained<AnyObject>> {
            let font: Option<Retained<AnyObject>> = msg_send_id![class!(NSFont), fontWithName: &*font_name, size: size * k];
            let font = font?;
            let ascender: f64 = msg_send![&font, ascender];
            let descender: f64 = msg_send![&font, descender];
            let layer: Retained<AnyObject> = msg_send_id![class!(CATextLayer), layer];
            let height = (ascender - descender) / k;
            // CATextLayer 把首行基线放在距顶部 ascender 处。
            let _: () = msg_send![&layer, setFrame: frame(left, baseline - ascender / k, size, height)];
            let _: () = msg_send![&layer, setString: &*NSString::from_str(text)];
            let _: () = msg_send![&layer, setFont: Retained::as_ptr(&font) as *const c_void];
            let _: () = msg_send![&layer, setFontSize: size * k];
            let _: () = msg_send![&layer, setContentsScale: scale];
            set_foreground(&layer, rgb);
            let _: () = msg_send![&card, addSublayer: &*layer];
            Some(layer)
        };

        glyph("$", PROMPT_SIZE, PROMPT_ORIGIN, WHITE_RGB);
        // 等宽字体：逐字摆放，字距 = 字号 × 0.602（Menlo 的 advance）。
        let now = CACurrentMediaTime();
        for (i, ch) in TITLE.chars().enumerate() {
            let left = TITLE_ORIGIN.0 + i as f64 * TITLE_SIZE * 0.602;
            if let Some(layer) = glyph(&ch.to_string(), TITLE_SIZE, (left, TITLE_ORIGIN.1), GREEN_RGB) {
                appear(&layer, now + TYPE_DELAY + i as f64 * TYPE_INTERVAL);
            }
        }
    }

    unsafe fn opacity_animation(from: f64, to: f64, duration: f64) -> Retained<AnyObject> {
        let key = NSString::from_str("opacity");
        let anim: Retained<AnyObject> = msg_send_id![class!(CABasicAnimation), animationWithKeyPath: &*key];
        let _: () = msg_send![&anim, setFromValue: &*NSNumber::new_f64(from)];
        let _: () = msg_send![&anim, setToValue: &*NSNumber::new_f64(to)];
        let _: () = msg_send![&anim, setDuration: duration];
        anim
    }

    /// 终端光标式闪烁。
    unsafe fn blink(layer: &AnyObject) {
        let anim = opacity_animation(1.0, 0.0, 0.5);
        let _: () = msg_send![&anim, setAutoreverses: true];
        let _: () = msg_send![&anim, setRepeatCount: f32::INFINITY];
        let name = NSString::from_str("easeInEaseOut");
        let timing: Retained<AnyObject> = msg_send_id![class!(CAMediaTimingFunction), functionWithName: &*name];
        let _: () = msg_send![&anim, setTimingFunction: &*timing];
        let _: () = msg_send![layer, addAnimation: &*anim, forKey: &*NSString::from_str("blink")];
    }

    /// 到 at 时刻才显现（之前保持透明），像逐字敲出。
    unsafe fn appear(layer: &AnyObject, at: f64) {
        let anim = opacity_animation(0.0, 1.0, 0.04);
        let _: () = msg_send![&anim, setBeginTime: at];
        let _: () = msg_send![&anim, setFillMode: &*NSString::from_str("backwards")];
        let _: () = msg_send![layer, addAnimation: &*anim, forKey: &*NSString::from_str("type")];
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

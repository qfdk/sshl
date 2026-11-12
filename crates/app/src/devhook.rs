//! 开发测试口（只在 debug 构建、且设了 SSHL_DEV_SOCKET 时启用）：经本机 Unix 套接字往窗口注入按键，
//! 走 GPUI 完整的按键分发 + 输入法通道。无头测试终端输入用，不抢用户焦点。release 构建里没有这段代码。
//!
//! 协议：一行一条命令，回 `ok` / `err ...`。
//!   keys ctrl-c enter up        按键（GPUI 写法）
//!   type echo hello             逐字符输入（含空格）
//!   click X Y [N]               在窗口坐标（逻辑像素）单击 / N 连击
//!   drag X1 Y1 X2 Y2            按住左键从一点拖到另一点
//!   selection                   把当前终端的选中文字打进日志（不碰系统剪贴板）

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{
    AnyWindowHandle, App, Keystroke, Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
    point, px,
};

/// `SSHL_DEV_DISPLAY=secondary`：窗口开在非主屏上（测试全屏时不占用户主屏的工作区）。
pub fn display_override(cx: &App) -> Option<std::rc::Rc<dyn gpui::PlatformDisplay>> {
    if std::env::var("SSHL_DEV_DISPLAY").ok().as_deref() != Some("secondary") {
        return None;
    }
    let primary = cx.primary_display().map(|d| d.id());
    cx.displays().into_iter().find(|d| Some(d.id()) != primary)
}

/// `SSHL_DEV_SIZE=1440x900`：窗口按这个大小（逻辑像素）开在屏幕中间，不铺满（截图要固定尺寸）。
pub fn size_override() -> Option<gpui::Size<gpui::Pixels>> {
    let value = std::env::var("SSHL_DEV_SIZE").ok()?;
    let (width, height) = value.split_once('x')?;
    Some(gpui::size(px(width.parse().ok()?), px(height.parse().ok()?)))
}

pub fn start(window: AnyWindowHandle, cx: &mut App) {
    let Ok(path) = std::env::var("SSHL_DEV_SOCKET") else { return };
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!("dev socket {path}: {error}");
            return;
        }
    };
    let (tx, mut rx) = mpsc::unbounded::<(String, oneshot::Sender<String>)>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut writer = match stream.try_clone() {
                Ok(writer) => writer,
                Err(_) => continue,
            };
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let (reply_tx, reply_rx) = oneshot::channel();
                if tx.unbounded_send((line, reply_tx)).is_err() {
                    return;
                }
                let reply = futures::executor::block_on(reply_rx).unwrap_or_else(|_| "err gone".into());
                let _ = writeln!(writer, "{reply}");
            }
        }
    });
    cx.spawn(async move |cx| {
        while let Some((line, reply)) = rx.next().await {
            let result = window.update(cx, |_, window, cx| run(&line, window, cx));
            let _ = reply.send(match result {
                Ok(Ok(())) => "ok".into(),
                Ok(Err(error)) => format!("err {error}"),
                Err(error) => format!("err {error}"),
            });
        }
    })
    .detach();
}

/// `mods cmd` 之后 ⌘ 一直按着：之后的 click / move 也带着它（测 ⌘ 悬停、⌘ 点击链接），`mods none` 松开。
static CMD_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn held() -> Modifiers {
    if CMD_HELD.load(std::sync::atomic::Ordering::Relaxed) { Modifiers::command() } else { Modifiers::none() }
}

fn run(line: &str, window: &mut gpui::Window, cx: &mut App) -> Result<(), String> {
    let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
    match command {
        "keys" => {
            for spec in rest.split_whitespace() {
                let keystroke = Keystroke::parse_with_os(spec, gpui::OperatingSystem::Mac).map_err(|e| format!("{spec}: {e}"))?;
                window.dispatch_keystroke(keystroke, cx);
            }
            Ok(())
        }
        // mods cmd | none：按住 / 松开修饰键（只发 ModifiersChanged），测按住 ⌘ 浮出的快捷键角标
        "mods" => {
            let modifiers = match rest.trim() {
                "cmd" => Modifiers::command(),
                "none" | "" => Modifiers::none(),
                other => return Err(format!("mods {other}：只认 cmd / none")),
            };
            CMD_HELD.store(modifiers.platform, std::sync::atomic::Ordering::Relaxed);
            window.dispatch_event(PlatformInput::ModifiersChanged(ModifiersChangedEvent { modifiers, capslock: Default::default() }), cx);
            Ok(())
        }
        "type" => {
            for ch in rest.chars() {
                let keystroke = match ch {
                    ' ' => Keystroke { modifiers: Modifiers::none(), key: "space".into(), key_char: Some(" ".into()) },
                    c if c.is_ascii_uppercase() => {
                        Keystroke { modifiers: Modifiers::shift(), key: c.to_ascii_lowercase().to_string(), key_char: Some(c.to_string()) }
                    }
                    c => Keystroke { modifiers: Modifiers::none(), key: c.to_string(), key_char: Some(c.to_string()) },
                };
                window.dispatch_keystroke(keystroke, cx);
            }
            Ok(())
        }
        "click" => {
            let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let (x, y, count) = match n.as_slice() {
                [x, y] => (*x, *y, 1),
                [x, y, c] => (*x, *y, *c as usize),
                _ => return Err("click X Y [N]".into()),
            };
            let at = point(px(x), px(y));
            for i in 1..=count {
                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: at, modifiers: held(), click_count: i, first_mouse: false }), cx);
                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: at, modifiers: held(), click_count: i }), cx);
            }
            Ok(())
        }
        // press X Y / release X Y：左键按下、松开分两条命令发，中间会重绘一帧——真实点击就是这样；
        // `click` 的按下 / 松开连着发、中间不重绘，「按下时就把被点的东西收掉了」这类毛病测不出来
        "press" | "release" => {
            let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let [x, y] = n.as_slice() else { return Err(format!("{command} X Y")) };
            let position = point(px(*x), px(*y));
            if command == "press" {
                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position, modifiers: held(), click_count: 1, first_mouse: false }), cx);
            } else {
                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position, modifiers: held(), click_count: 1 }), cx);
            }
            Ok(())
        }
        // rclick X Y：右键点击
        "rclick" => {
            let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let [x, y] = n.as_slice() else { return Err("rclick X Y".into()) };
            let at = point(px(*x), px(*y));
            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position: at, modifiers: Modifiers::none(), click_count: 1, first_mouse: false }), cx);
            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position: at, modifiers: Modifiers::none(), click_count: 1 }), cx);
            Ok(())
        }
        "drag" => {
            let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let [x1, y1, x2, y2] = n.as_slice() else { return Err("drag X1 Y1 X2 Y2".into()) };
            let (from, to) = (point(px(*x1), px(*y1)), point(px(*x2), px(*y2)));
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers: Modifiers::none() }), cx);
            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: from, modifiers: Modifiers::none(), click_count: 1, first_mouse: false }), cx);
            for step in 1..=8 {
                let t = step as f32 / 8.;
                let at = point(px(x1 + (x2 - x1) * t), px(y1 + (y2 - y1) * t));
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::none() }), cx);
            }
            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: to, modifiers: Modifiers::none(), click_count: 1 }), cx);
            Ok(())
        }
        // drop X Y PATH... ：模拟从 Finder 拖文件进来（进入 → 移动 → 松开），走 GPUI 完整的拖放分发
        "drop" => {
            let mut parts = rest.split_whitespace();
            let (Some(x), Some(y)) = (parts.next().and_then(|v| v.parse::<f32>().ok()), parts.next().and_then(|v| v.parse::<f32>().ok())) else {
                return Err("drop X Y PATH...".into());
            };
            let paths: Vec<std::path::PathBuf> = parts.map(std::path::PathBuf::from).collect();
            if paths.is_empty() {
                return Err("drop X Y PATH...".into());
            }
            let at = point(px(x), px(y));
            let paths = gpui::ExternalPaths(paths.into_iter().collect());
            window.dispatch_event(PlatformInput::FileDrop(gpui::FileDropEvent::Entered { position: at, paths }), cx);
            window.dispatch_event(PlatformInput::FileDrop(gpui::FileDropEvent::Pending { position: at }), cx);
            window.dispatch_event(PlatformInput::FileDrop(gpui::FileDropEvent::Submit { position: at }), cx);
            Ok(())
        }
        // move X Y：鼠标移到某处（悬停状态）
        "move" => {
            let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            let [x, y] = n.as_slice() else { return Err("move X Y".into()) };
            let position = point(px(*x), px(*y));
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: held() }), cx);
            Ok(())
        }
        // hover X Y PATH... ：拖着文件停在某处不松手（看「松开上传」提示）
        "hover" => {
            let mut parts = rest.split_whitespace();
            let (Some(x), Some(y)) = (parts.next().and_then(|v| v.parse::<f32>().ok()), parts.next().and_then(|v| v.parse::<f32>().ok())) else {
                return Err("hover X Y PATH...".into());
            };
            let paths = gpui::ExternalPaths(parts.map(std::path::PathBuf::from).collect());
            let at = point(px(x), px(y));
            window.dispatch_event(PlatformInput::FileDrop(gpui::FileDropEvent::Entered { position: at, paths }), cx);
            window.dispatch_event(PlatformInput::FileDrop(gpui::FileDropEvent::Pending { position: at }), cx);
            Ok(())
        }
        // appearance dark | light | system：只改内存里的界面外观（不写设置文件），看深色界面用
        "appearance" => {
            let appearance = match rest.trim() {
                "dark" => crate::theme::Appearance::Dark,
                "light" => crate::theme::Appearance::Light,
                "system" => crate::theme::Appearance::System,
                other => return Err(format!("appearance {other}：只认 dark / light / system")),
            };
            crate::settings::preview(cx, |s| s.appearance = appearance);
            Ok(())
        }
        // language zh | en | system：只改内存里的界面语言（不写设置文件），看英文界面用
        "language" => {
            let language = match rest.trim() {
                "zh" => crate::settings::Language::Zh,
                "en" => crate::settings::Language::En,
                "system" => crate::settings::Language::System,
                other => return Err(format!("language {other}：只认 zh / en / system")),
            };
            crate::settings::preview(cx, |s| s.language = language);
            Ok(())
        }
        "selection" => {
            window.dispatch_action(Box::new(crate::terminal::view::LogSelection), cx);
            Ok(())
        }
        other => Err(format!("unknown command {other}")),
    }
}

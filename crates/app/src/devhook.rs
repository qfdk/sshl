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
    AnyWindowHandle, App, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
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
                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::none(), click_count: i, first_mouse: false }), cx);
                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::none(), click_count: i }), cx);
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
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::none() }), cx);
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
        "selection" => {
            window.dispatch_action(Box::new(crate::terminal::view::LogSelection), cx);
            Ok(())
        }
        other => Err(format!("unknown command {other}")),
    }
}

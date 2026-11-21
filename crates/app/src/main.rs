#[macro_use]
extern crate sshl_core;

mod app_icons;
mod assets;
mod brands;
mod backend;
mod connection_dialog;
#[cfg(any(debug_assertions, feature = "devhook"))]
mod devhook;
mod favorites;
mod files;
mod groups;
mod i18n_en;
mod import_dialog;
mod keymap;
mod prompts;
mod quick_connect;
mod screen;
mod settings;
mod settings_dialog;
mod sidebar;
mod snippets;
mod system_panel;
mod terminal;
mod theme;
mod topbar;
mod ui;
mod update;
mod workspace;

use std::borrow::Cow;

use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{App, AppContext, Bounds, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, actions, point, px};

use workspace::Workspace;

actions!(sshl, [Quit, Hide, HideOthers, ShowAll, CheckForUpdates, About]);

/// 无障碍树是懒建的：第一次有人来问才开始建，那一次答的是一棵空树（没有焦点元素），下一帧才有真的。
/// 听写工具每次按住只问一次，不提前建的话每次启动后头几次按住都落空。这里自己先问一次，等于把它提前打开。
fn warm_accessibility() {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let windows: *mut AnyObject = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        for index in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: index];
            let _: *mut AnyObject = msg_send![window, accessibilityFocusedUIElement];
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "sshl=info,sshl_core=info,warn".into()),
        )
        .try_init()
        .ok();

    // 后端事件（传输进度、连接列表变化）从 tokio 线程转回界面线程
    let (events_tx, mut events_rx) = unbounded();
    backend::init(move |event| {
        let _ = events_tx.unbounded_send(event);
    });

    let app = gpui_platform::application().with_assets(assets::Assets);
    // 关窗只隐藏、进程常驻；点 Dock 图标直接显示，不再走一遍冷启动
    app.on_reopen(|cx| {
        cx.activate(true);
        if let Some(window) = cx.windows().first() {
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
        // 从程序坞点开也是打开：检查更新（离上次不满一小时不查）
        update::check_on_open(cx);
    });
    app.run(move |cx: &mut App| {
        gpui_base::init(cx);
        gpui_base::Theme::global_mut(cx).tokens = theme::base_tokens(false);
        let prompts = prompts::init(cx);
        sshl_core::i18n::register_en(i18n_en::TABLE);
        settings::init(cx);
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../../assets/fonts/JetBrainsMonoNerdFont-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../../../assets/fonts/JetBrainsMonoNerdFont-Bold.ttf").as_slice()),
            ])
            .expect("load bundled terminal font");

        // 快捷键：gpui-base 的、设置里能改的、写死的，一次建好（键盘布局变了、设置改了会重建）
        keymap::init(cx);
        // ⌘Q 先到工作区（开着标签时要确认，见 Workspace::request_quit）。没有主窗口时（⌘M 最小化了）菜单的 ⌘Q 没人接，
        // 落到这里：一样交给工作区去问，不能不声不响把所有会话关掉。按键分发途中窗口正被占着，等这一轮完了再进去
        cx.on_action(|_: &Quit, cx| {
            let Some(window) = cx.windows().first().and_then(|w| w.downcast::<gpui_base::Root>()) else { return cx.quit() };
            cx.defer(move |cx| {
                let asked = window.update(cx, |root, window, cx| {
                    let workspace = root.view().clone().downcast::<Workspace>().ok()?;
                    // 最小化的窗口拿出来，不然确认框在看不见的窗口里
                    window.activate_window();
                    workspace.update(cx, |workspace, cx| workspace.request_quit(window, cx));
                    Some(())
                });
                if !matches!(asked, Ok(Some(()))) {
                    cx.quit();
                }
            });
        });
        // 菜单栏「检查更新…」：同 ⌘Q，等这一轮按键 / 菜单分发完了再进窗口（结果的提示框要弹在窗口里）
        cx.on_action(|_: &CheckForUpdates, cx| {
            let Some(window) = cx.windows().first().copied() else { return };
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| update::check_now(window, cx, update::Report::Prompt));
            });
        });
        // 菜单栏「关于 SSHL」：打开设置的「关于」页
        cx.on_action(|_: &About, cx| {
            let Some(window) = cx.windows().first().and_then(|w| w.downcast::<gpui_base::Root>()) else { return };
            cx.defer(move |cx| {
                let _ = window.update(cx, |root, window, cx| {
                    window.activate_window();
                    if let Ok(workspace) = root.view().clone().downcast::<Workspace>() {
                        workspace.update(cx, |workspace, cx| workspace.open_about(window, cx));
                    }
                });
            });
        });
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        set_menus(cx);

        // 主窗口直接按主屏可用区域创建
        // 主窗口开在鼠标所在的屏幕上，铺满那块屏幕的可用区域（只让出菜单栏和 Dock），不进原生全屏
        let display = screen::display_under_mouse(cx).or_else(|| cx.primary_display());
        #[cfg(any(debug_assertions, feature = "devhook"))]
        let display = devhook::display_override(cx).or(display);
        let bounds = display
            .as_ref()
            .map(|display| display.visible_bounds())
            .unwrap_or_else(|| Bounds::centered(None, gpui::size(px(1280.), px(800.)), cx));
        #[cfg(any(debug_assertions, feature = "devhook"))]
        let bounds = devhook::size_override().map_or(bounds, |size| Bounds::centered(display.as_ref().map(|display| display.id()), size, cx));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: display.map(|display| display.id()),
            titlebar: Some(TitlebarOptions {
                title: Some("SSHL".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(17.))),
            }),
            window_min_size: Some(gpui::size(px(900.), px(600.))),
            // 标题栏是自己画的（标签、按钮都在红绿灯那一条里），拖窗口只走 workspace::drag_region。
            // 不设的话 AppKit 在标题栏高度内自己拖窗口：拖标签排序时整个窗口跟着走，GPUI 拦不住
            app_owns_titlebar_drag: true,
            ..Default::default()
        };
        // 系统问「焦点元素」时问的是窗口和第一响应者（GPUI 自己的视图），而无障碍树挂在外层的内容视图上，问不到：
        // 不补这个转发，焦点元素永远是窗口，听写工具（Voilà 按住鼠标说话）认不出终端能输入。窗口类在启动时就建好了，只补一次。
        unsafe { accesskit_macos::add_focus_forwarder_to_window_class("GPUIWindow") };

        let window = cx
            .open_window(options, |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(prompts.clone(), window, cx));
                cx.new(|cx| gpui_base::Root::new(workspace, window, cx))
            })
            .expect("open main window");
        warm_accessibility();
        // 打开时检查更新：窗口出来后在后台查，不在启动路径上
        update::init(cx);

        // 关窗口（红点 / ⌘W 没有标签时）只隐藏应用，会话、预热连接、界面都留着
        let _ = window.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, |_, cx| {
                cx.hide();
                false
            });
        });

        #[cfg(any(debug_assertions, feature = "devhook"))]
        devhook::start(window.into(), cx);

        cx.spawn(async move |cx| {
            while let Some(event) = events_rx.next().await {
                let _ = window.update(cx, |root, _, cx| {
                    if let Ok(workspace) = root.view().clone().downcast::<Workspace>() {
                        workspace.update(cx, |workspace, cx| workspace.core_event(event, cx));
                    }
                });
            }
        })
        .detach();
    });
}

/// 菜单栏。文字在建的时候就定了，所以换语言后要重建一次（`settings::replace`）。
pub fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "SSHL".into(),
            items: vec![
                MenuItem::action(tr!("关于 SSHL"), About),
                MenuItem::action(tr!("检查更新…"), CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action(tr!("隐藏 SSHL"), Hide),
                MenuItem::action(tr!("隐藏其他"), HideOthers),
                MenuItem::action(tr!("全部显示"), ShowAll),
                MenuItem::separator(),
                MenuItem::action(tr!("退出 SSHL"), Quit),
            ],
            disabled: false,
        },
        Menu {
            name: tr!("编辑").into(),
            items: vec![
                MenuItem::action(tr!("拷贝"), terminal::view::Copy),
                MenuItem::action(tr!("粘贴"), terminal::view::Paste),
                MenuItem::action(tr!("全选"), terminal::view::SelectAll),
            ],
            disabled: false,
        },
        Menu { name: tr!("窗口").into(), items: vec![MenuItem::action(tr!("最小化"), workspace::Minimize)], disabled: false },
    ]);
}

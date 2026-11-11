mod assets;
mod backend;
mod connection_dialog;
#[cfg(any(debug_assertions, feature = "devhook"))]
mod devhook;
mod files;
mod groups;
mod import_dialog;
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
mod workspace;

use std::borrow::Cow;

use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{App, AppContext, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, actions, point, px};

use workspace::Workspace;

actions!(sshl, [Quit, Hide, HideOthers, ShowAll]);

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
    });
    app.run(move |cx: &mut App| {
        gpui_base::init(cx);
        gpui_base::Theme::global_mut(cx).tokens = theme::base_tokens();
        let prompts = prompts::init(cx);
        settings::init(cx);
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../../assets/fonts/JetBrainsMonoNerdFont-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../../../assets/fonts/JetBrainsMonoNerdFont-Bold.ttf").as_slice()),
            ])
            .expect("load bundled terminal font");

        bind_keys(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.set_menus(vec![
            Menu {
                name: "SSHL".into(),
                items: vec![
                    MenuItem::action("隐藏 SSHL", Hide),
                    MenuItem::action("隐藏其他", HideOthers),
                    MenuItem::action("全部显示", ShowAll),
                    MenuItem::separator(),
                    MenuItem::action("退出 SSHL", Quit),
                ],
                disabled: false,
            },
            Menu {
                name: "编辑".into(),
                items: vec![
                    MenuItem::action("拷贝", terminal::view::Copy),
                    MenuItem::action("粘贴", terminal::view::Paste),
                    MenuItem::action("全选", terminal::view::SelectAll),
                ],
                disabled: false,
            },
            Menu { name: "窗口".into(), items: vec![MenuItem::action("最小化", workspace::Minimize)], disabled: false },
        ]);

        // 主窗口直接按主屏可用区域创建
        // 主窗口开在鼠标所在的屏幕上，铺满那块屏幕的可用区域（只让出菜单栏和 Dock），不进原生全屏
        let display = screen::display_under_mouse(cx).or_else(|| cx.primary_display());
        #[cfg(any(debug_assertions, feature = "devhook"))]
        let display = devhook::display_override(cx).or(display);
        let bounds = display
            .as_ref()
            .map(|display| display.visible_bounds())
            .unwrap_or_else(|| Bounds::centered(None, gpui::size(px(1280.), px(800.)), cx));
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
        let window = cx
            .open_window(options, |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(prompts.clone(), window, cx));
                cx.new(|cx| gpui_base::Root::new(workspace, window, cx))
            })
            .expect("open main window");

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

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", workspace::Minimize, None),
    ]);
    cx.bind_keys(terminal::view::key_bindings());
    workspace::bind_keys(cx);
    // 晚于 gpui-base：同一节点上后绑的优先，面板里的上下 / 回车 / Esc 归片段列表，标签改名框里的回车 / Esc 归改名，分组框里的上下 / Esc 归分组下拉
    snippets::bind_keys(cx);
    quick_connect::bind_keys(cx);
    cx.bind_keys(topbar::key_bindings());
    cx.bind_keys(connection_dialog::key_bindings());
}

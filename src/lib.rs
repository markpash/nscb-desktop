//! NSCB Desktop: a GPUI (gpui-kit) front end for the nscb_rust backend.

pub mod core;
pub mod runner;
pub mod ui;

use gpui_kit::{
    actions, px, size, App, AppContext, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions,
};

actions!(nscb, [Quit]);

/// Window options for the main window: 1100x750, minimum 900x600, centered.
pub fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1100.), px(750.)), cx))),
        window_min_size: Some(size(px(900.), px(600.))),
        titlebar: Some(TitlebarOptions { title: Some("NSCB Desktop".into()), ..Default::default() }),
        app_id: Some("com.nscb.desktop".into()),
        ..Default::default()
    }
}

pub fn run() {
    gpui_kit::application().with_assets(ui::icons::AppAssets).run(|cx: &mut App| {
        gpui_kit::init(cx);
        ui::theme::apply(cx);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu { name: "NSCB Desktop".into(), items: vec![MenuItem::action("Quit NSCB Desktop", Quit)], disabled: false }]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let tools = match core::tools::Tools::locate() {
            Ok(tools) => tools,
            Err(e) => {
                eprintln!("{e}");
                cx.quit();
                return;
            }
        };
        let options = window_options(cx);
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| ui::app::NscbApp::new(tools, window, cx)))
            .expect("failed to open window");
        cx.activate(true);
    });
}

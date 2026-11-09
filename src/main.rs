#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod app;
mod cache;
mod config;
mod csv;
mod journal;
mod metadata;
mod orientation;
mod persist;
mod state;
mod theme;
mod ui;

use gpui_kit::*;
use ui::AutoWordingView;

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            ui::init_keybindings(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Maximized(Bounds {
                        origin: point(px(80.0), px(60.0)),
                        size: size(px(1440.0), px(920.0)),
                    })),
                    titlebar: Some(TitlebarOptions {
                        title: Some("AutoWording".into()),
                        appears_transparent: false,
                        traffic_light_position: None,
                    }),
                    window_min_size: Some(size(px(1024.0), px(680.0))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| AutoWordingView::new(window, cx)),
            )
            .expect("failed to open AutoWording window");
        });
}

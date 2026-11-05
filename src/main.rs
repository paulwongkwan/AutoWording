#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod app;
mod cache;
mod config;
mod metadata;
mod state;
mod theme;
mod ui;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AutoWording")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([1180.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "AutoWording",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

pub mod browser;
pub mod metadata_panel;
pub mod tree;
pub mod viewer;

use crate::app::{App, WorkspaceMode};
use crate::state::Status;
use crate::theme;
use eframe::egui::{self, Align, Color32, Frame, Layout, RichText, Stroke, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    OpenFolder,
    AddImages,
    SaveSelected,
    GenerateSelected,
    SelectAll,
    ClearSelection,
    Refresh,
    CheckExif,
    About,
    Quit,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    top_bar(app, ctx);
    status_bar(app, ctx);
    match app.mode {
        WorkspaceMode::Browser => browser::show(app, ctx),
        WorkspaceMode::Viewer => viewer::show(app, ctx),
    }
    drop_overlay(ctx);
    about_window(app, ctx);
}

fn top_bar(app: &mut App, ctx: &egui::Context) {
    let mut action = None;

    let frame = Frame::none()
        .fill(theme::CARD)
        .inner_margin(egui::Margin::symmetric(12.0, 4.0));

    egui::TopBottomPanel::top("menu_bar")
        .frame(frame)
        .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.add(egui::Label::new(
                    RichText::new("AutoWording").size(14.0).strong(),
                ));
                ui.add_space(10.0);

                ui.menu_button("File", |ui| {
                    if ui.button("Open Folder…    Ctrl+O").clicked() {
                        action = Some(UiAction::OpenFolder);
                        ui.close_menu();
                    }
                    if ui.button("Add Images…    Ctrl+Shift+O").clicked() {
                        action = Some(UiAction::AddImages);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            !app.selection.is_empty() && app.batch.is_none(),
                            egui::Button::new("Save Selected to EXIF    Ctrl+S"),
                        )
                        .clicked()
                    {
                        action = Some(UiAction::SaveSelected);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        action = Some(UiAction::Quit);
                        ui.close_menu();
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(
                            !app.folder_images.is_empty(),
                            egui::Button::new("Select All    Ctrl+A"),
                        )
                        .clicked()
                    {
                        action = Some(UiAction::SelectAll);
                        ui.close_menu();
                    }
                    if ui.button("Clear Selection").clicked() {
                        action = Some(UiAction::ClearSelection);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            !app.selection.is_empty() && app.batch.is_none(),
                            egui::Button::new("Run AI on Selected    Ctrl+Enter"),
                        )
                        .clicked()
                    {
                        action = Some(UiAction::GenerateSelected);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            !app.selection.is_empty() && app.batch.is_none(),
                            egui::Button::new("Save Selected to EXIF    Ctrl+S"),
                        )
                        .clicked()
                    {
                        action = Some(UiAction::SaveSelected);
                        ui.close_menu();
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui
                        .selectable_label(app.mode == WorkspaceMode::Browser, "Browser")
                        .clicked()
                    {
                        app.mode = WorkspaceMode::Browser;
                        ui.close_menu();
                    }
                    if ui
                        .selectable_label(app.mode == WorkspaceMode::Viewer, "Viewer")
                        .clicked()
                    {
                        app.mode = WorkspaceMode::Viewer;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Refresh Folder    F5").clicked() {
                        action = Some(UiAction::Refresh);
                        ui.close_menu();
                    }
                });

                ui.menu_button("Tools", |ui| {
                    if ui.button("Re-check ExifTool").clicked() {
                        action = Some(UiAction::CheckExif);
                        ui.close_menu();
                    }
                    if ui.button("About / Diagnostics").clicked() {
                        action = Some(UiAction::About);
                        ui.close_menu();
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About AutoWording").clicked() {
                        action = Some(UiAction::About);
                        ui.close_menu();
                    }
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.selectable_value(&mut app.mode, WorkspaceMode::Viewer, "Viewer");
                    ui.selectable_value(&mut app.mode, WorkspaceMode::Browser, "Browser");
                    ui.add(egui::Label::new(
                        RichText::new("Mode").size(10.5).color(theme::TEXT_MUTED),
                    ));
                });
            });
        });

    if let Some(action) = action {
        app.handle_action(ctx, action);
    }
}

fn status_bar(app: &mut App, ctx: &egui::Context) {
    let frame = Frame::none()
        .fill(theme::CARD)
        .inner_margin(egui::Margin::symmetric(14.0, 5.0));

    egui::TopBottomPanel::bottom("status_bar")
        .frame(frame)
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.top(),
                Stroke::new(1.0_f32, theme::BORDER),
            );

            ui.horizontal(|ui| {
                match &app.toast {
                    Some(toast) => {
                        ui.add(egui::Label::new(
                            RichText::new(&toast.text).size(11.0).color(if toast.error {
                                theme::DANGER
                            } else {
                                theme::TEXT_MUTED
                            }),
                        ));
                    }
                    None => {
                        let loaded = app.folder_images.len();
                        let noun = if loaded == 1 { "image" } else { "images" };
                        ui.add(egui::Label::new(
                            RichText::new(format!(
                                "Status: {loaded} {noun} loaded  |  {} selected",
                                app.selection.count()
                            ))
                            .size(11.0)
                            .color(theme::TEXT_MUTED),
                        ));
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let exiftool = match &app.exiftool {
                        Some(tool) => format!("ExifTool {}", tool.version),
                        None => "ExifTool missing".to_string(),
                    };
                    let provider = app.provider_line();
                    if let Some(batch) = app.batch {
                        ui.add(egui::Label::new(
                            RichText::new(format!(
                                "{} {}/{}",
                                if batch.saving { "Saving" } else { "AI" },
                                batch.done,
                                batch.total
                            ))
                            .size(10.5)
                            .color(theme::ACCENT_TEXT),
                        ));
                        ui.separator();
                    }
                    ui.add(egui::Label::new(
                        RichText::new(exiftool).size(10.5).color(theme::TEXT_MUTED),
                    ));
                    ui.separator();
                    ui.add(egui::Label::new(
                        RichText::new(provider).size(10.5).color(theme::TEXT_MUTED),
                    ));
                });
            });
        });
}

fn drop_overlay(ctx: &egui::Context) {
    let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
    if !hovering {
        return;
    }

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("drop_overlay"),
    ));
    let rect = ctx.screen_rect();
    painter.rect_filled(rect, egui::Rounding::ZERO, Color32::from_rgba_unmultiplied(0, 70, 130, 60));
    let card = egui::Rect::from_center_size(rect.center(), egui::Vec2::new(380.0, 120.0));
    painter.rect_filled(card, egui::Rounding::same(12.0), Color32::WHITE);
    painter.rect_stroke(card, egui::Rounding::same(12.0), Stroke::new(2.0_f32, theme::ACCENT));
    painter.text(
        card.center(),
        egui::Align2::CENTER_CENTER,
        "Drop JPEG files or folders",
        egui::FontId::proportional(16.0),
        theme::TEXT,
    );
}

fn about_window(app: &mut App, ctx: &egui::Context) {
    egui::Window::new("About AutoWording")
        .open(&mut app.show_about)
        .collapsible(false)
        .resizable(false)
        .default_width(430.0)
        .show(ctx, |ui| {
            ui.add(egui::Label::new(
                RichText::new(format!("AutoWording {}", env!("CARGO_PKG_VERSION")))
                    .size(16.0)
                    .strong(),
            ));
            ui.add_space(4.0);
            ui.label("ACDSee-style AI keywording workspace for microstock JPEGs.");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(format!("Config: {}", app.cfg.display_path()));
            ui.label(format!(
                "Folder: {}",
                app.current_folder
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none open".to_string())
            ));
            ui.label(format!(
                "ExifTool: {}",
                app.exiftool
                    .as_ref()
                    .map(|tool| format!("{} ({})", tool.version, tool.program().display()))
                    .unwrap_or_else(|| "not detected".to_string())
            ));
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label("Shortcuts");
            ui.small("Ctrl+O open folder · Ctrl+Shift+O add images");
            ui.small("Ctrl+A select grid · Ctrl+Click toggle · Shift+Click range");
            ui.small("Ctrl+Enter run AI on selection · Ctrl+S save selection");
            ui.small("↑/↓ change active image · F5 refresh · Esc leave Viewer");
        });
}

pub fn section_title(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(14.0).strong()));
}

pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(12.5).color(Color32::WHITE))
        .fill(theme::ACCENT)
        .stroke(Stroke::NONE)
        .rounding(egui::Rounding::same(6.0))
        .min_size(egui::vec2(0.0, 28.0))
}

pub fn ghost_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(12.5).color(theme::TEXT))
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0_f32, theme::BORDER_STRONG))
        .rounding(egui::Rounding::same(6.0))
        .min_size(egui::vec2(0.0, 28.0))
}

pub fn tiny_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(11.0).color(theme::TEXT_MUTED)).frame(false)
}

pub fn text_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(11.5).color(theme::TEXT_MUTED)).frame(false)
}

pub fn count_chip(ui: &mut Ui, text: &str, ok: bool) {
    let (fg, bg) = if ok {
        (theme::SUCCESS, theme::SUCCESS_SOFT)
    } else {
        (theme::WARNING, theme::WARNING_SOFT)
    };
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(999.0))
        .inner_margin(egui::Margin::symmetric(7.0, 2.0))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).size(10.0).color(fg)));
        });
}

pub fn status_style(status: Status, dirty: bool) -> (&'static str, Color32, Color32) {
    if dirty && matches!(status, Status::Saved | Status::Generated) {
        return ("Modified", theme::WARNING, theme::WARNING_SOFT);
    }
    match status {
        Status::Pending => ("Pending", Color32::from_rgb(97, 97, 97), theme::CHIP_BG),
        Status::Queued => ("In queue", theme::ACCENT_TEXT, theme::ACCENT_SOFT),
        Status::Processing => ("Processing", theme::ACCENT_TEXT, theme::ACCENT_SOFT),
        Status::Generated => ("Generated", theme::VIOLET, theme::VIOLET_SOFT),
        Status::Saved => ("Saved", theme::SUCCESS, theme::SUCCESS_SOFT),
        Status::Error => ("Failed", theme::DANGER, theme::DANGER_SOFT),
    }
}

pub fn status_pill(ui: &mut Ui, status: Status, dirty: bool) {
    let (text, fg, bg) = status_style(status, dirty);
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(999.0))
        .inner_margin(egui::Margin::symmetric(6.0, 1.0))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).size(10.0).color(fg)));
        });
}

pub fn dot(ui: &mut Ui, color: Color32) {
    ui.add(egui::Label::new(RichText::new("●").size(9.0).color(color)));
}

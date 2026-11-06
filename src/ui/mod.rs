pub mod browser;
pub mod metadata_panel;
pub mod tree;
pub mod viewer;

use crate::app::{App, WorkspaceMode};
use crate::config::{LayoutDensity, ThemePreference};
use crate::state::Status;
use crate::theme;
use eframe::egui::{
    self, Align, Color32, FontId, Frame, Layout, Margin, RichText, Rounding, Stroke, Ui, Vec2,
};

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
    let mut theme_choice = None;
    let mut density_choice = None;

    let frame = Frame::none()
        .fill(theme::card_fill())
        .inner_margin(Margin::symmetric(14.0, 6.0));

    egui::TopBottomPanel::top("menu_bar")
        .frame(frame)
        .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                let (mark_rect, _) = ui.allocate_exact_size(Vec2::splat(23.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(mark_rect, Rounding::same(6.0), theme::accent());
                ui.painter().text(
                    mark_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "AW",
                    FontId::proportional(9.0),
                    Color32::WHITE,
                );
                ui.add(egui::Label::new(
                    RichText::new("AutoWording").size(15.0).strong(),
                ));
                ui.add_space(8.0);

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

                ui.menu_button("Theme", |ui| {
                    ui.set_max_width(270.0);
                    for preference in ThemePreference::ALL {
                        if ui
                            .selectable_label(app.cfg.theme == preference, preference.label())
                            .clicked()
                        {
                            theme_choice = Some(preference);
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Follow Windows re-checks the app theme when AutoWording regains focus.",
                            )
                            .size(9.5)
                            .color(theme::text_muted()),
                        )
                        .wrap(),
                    );
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
                    Frame::none()
                        .fill(theme::chip_bg())
                        .stroke(Stroke::new(1.0_f32, theme::border()))
                        .rounding(Rounding::same(999.0))
                        .inner_margin(Margin::same(2.0))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if ui
                                .add(segmented_button(
                                    "Browser",
                                    app.mode == WorkspaceMode::Browser,
                                ))
                                .clicked()
                            {
                                app.mode = WorkspaceMode::Browser;
                            }
                            if ui
                                .add(segmented_button(
                                    "Viewer",
                                    app.mode == WorkspaceMode::Viewer,
                                ))
                                .clicked()
                            {
                                app.mode = WorkspaceMode::Viewer;
                            }
                        });
                    ui.add(egui::Label::new(
                        RichText::new("WORKSPACE")
                            .size(9.0)
                            .strong()
                            .color(theme::text_muted()),
                    ));
                    Frame::none()
                        .fill(theme::chip_bg())
                        .stroke(Stroke::new(1.0_f32, theme::border()))
                        .rounding(Rounding::same(999.0))
                        .inner_margin(Margin::same(2.0))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for density in LayoutDensity::ALL {
                                let selected = app.cfg.density == density;
                                let tip = if density.is_compact() {
                                    "More cards and tighter metadata fields"
                                } else {
                                    "Larger cards and roomier metadata fields"
                                };
                                if ui
                                    .add(segmented_button(density.label(), selected))
                                    .on_hover_text(tip)
                                    .clicked()
                                {
                                    density_choice = Some(density);
                                }
                            }
                        });
                    ui.add(egui::Label::new(
                        RichText::new("DENSITY")
                            .size(9.0)
                            .strong()
                            .color(theme::text_muted()),
                    ));
                });
            });

            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.bottom(),
                Stroke::new(1.0_f32, theme::border()),
            );
        });

    if let Some(preference) = theme_choice {
        app.set_theme(ctx, preference);
    }
    if let Some(density) = density_choice {
        app.set_density(density);
    }
    if let Some(action) = action {
        app.handle_action(ctx, action);
    }
}

fn status_bar(app: &mut App, ctx: &egui::Context) {
    let frame = Frame::none()
        .fill(theme::card_fill())
        .inner_margin(Margin::symmetric(14.0, 6.0));

    egui::TopBottomPanel::bottom("status_bar")
        .frame(frame)
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.top(),
                Stroke::new(1.0_f32, theme::border()),
            );

            ui.horizontal(|ui| {
                match &app.toast {
                    Some(toast) => {
                        ui.add(egui::Label::new(
                            RichText::new(&toast.text).size(11.0).color(if toast.error {
                                theme::danger()
                            } else {
                                theme::text_muted()
                            }),
                        ));
                    }
                    None => {
                        let loaded = app.folder_images.len();
                        let noun = if loaded == 1 { "image" } else { "images" };
                        let selected = app.selection.count();
                        ui.add(egui::Label::new(
                            RichText::new(format!("{loaded} {noun}"))
                                .size(11.0)
                                .strong()
                                .color(theme::text()),
                        ));
                        ui.add(egui::Label::new(
                            RichText::new("·").size(12.0).color(theme::border_strong()),
                        ));
                        ui.add(egui::Label::new(
                            RichText::new(format!("{selected} selected"))
                                .size(11.0)
                                .color(if selected > 0 {
                                    theme::accent_text()
                                } else {
                                    theme::text_muted()
                                }),
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
                        Frame::none()
                            .fill(theme::accent_soft())
                            .rounding(Rounding::same(999.0))
                            .inner_margin(Margin::symmetric(8.0, 2.0))
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(
                                    RichText::new(format!(
                                        "{} {}/{}",
                                        if batch.saving { "Saving" } else { "AI" },
                                        batch.done,
                                        batch.total
                                    ))
                                    .size(10.0)
                                    .color(theme::accent_text()),
                                ));
                            });
                        if !batch.saving {
                            if app.batch_paused {
                                if ui.small_button("Resume").clicked() {
                                    app.set_batch_paused(false);
                                }
                            } else if ui
                                .small_button("Pause")
                                .on_hover_text("Pause after the current image")
                                .clicked()
                            {
                                app.set_batch_paused(true);
                            }
                            if ui.small_button("Cancel").clicked() {
                                app.cancel_batch();
                            }
                        }
                    }
                    ui.add(egui::Label::new(
                        RichText::new(exiftool).size(10.5).color(theme::text_muted()),
                    ));
                    ui.add(egui::Label::new(
                        RichText::new("·").size(12.0).color(theme::border_strong()),
                    ));
                    ui.add(egui::Label::new(
                        RichText::new(provider).size(10.5).color(theme::text_muted()),
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
    painter.rect_filled(card, egui::Rounding::same(12.0), theme::card_fill());
    painter.rect_stroke(card, egui::Rounding::same(12.0), Stroke::new(2.0_f32, theme::accent()));
    painter.text(
        card.center(),
        egui::Align2::CENTER_CENTER,
        "Drop JPEG files or folders",
        egui::FontId::proportional(16.0),
        theme::text(),
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
    ui.add(egui::Label::new(
        RichText::new(text).size(14.0).strong().color(theme::text()),
    ));
}

pub fn segmented_button(text: impl Into<String>, selected: bool) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.into())
            .size(11.0)
            .color(if selected {
                theme::accent_text()
            } else {
                theme::text_muted()
            }),
    )
    .fill(if selected {
        theme::card_fill()
    } else {
        Color32::TRANSPARENT
    })
    .stroke(if selected {
        Stroke::new(1.0_f32, theme::border())
    } else {
        Stroke::NONE
    })
    .rounding(Rounding::same(999.0))
    .min_size(egui::vec2(58.0, 22.0))
}

pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(12.5).color(Color32::WHITE))
        .fill(theme::accent())
        .stroke(Stroke::NONE)
        .rounding(egui::Rounding::same(7.0))
        .min_size(egui::vec2(0.0, 30.0))
}

pub fn ghost_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(12.0).color(theme::text()))
        .fill(theme::card_fill())
        .stroke(Stroke::new(1.0_f32, theme::border_strong()))
        .rounding(egui::Rounding::same(7.0))
        .min_size(egui::vec2(0.0, 28.0))
}

pub fn tiny_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(11.0).color(theme::text_muted())).frame(false)
}

pub fn text_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(11.5).color(theme::text_muted())).frame(false)
}

pub fn count_chip(ui: &mut Ui, text: &str, ok: bool) {
    let (fg, bg) = if ok {
        (theme::success(), theme::success_soft())
    } else {
        (theme::warning(), theme::warning_soft())
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
        return ("Modified", theme::warning(), theme::warning_soft());
    }
    match status {
        Status::Pending => ("Pending", theme::text_muted(), theme::chip_bg()),
        Status::Queued => ("In queue", theme::accent_text(), theme::accent_soft()),
        Status::Processing => ("Processing", theme::accent_text(), theme::accent_soft()),
        Status::Generated => ("Generated", theme::violet(), theme::violet_soft()),
        Status::Saved => ("Saved", theme::success(), theme::success_soft()),
        Status::Error => ("Failed", theme::danger(), theme::danger_soft()),
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

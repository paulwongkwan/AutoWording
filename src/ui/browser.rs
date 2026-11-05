use crate::app::{App, WorkspaceMode};
use crate::state::SelectModifiers;
use crate::theme;
use crate::ui::{self, count_chip, ghost_button, status_pill, status_style};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Layout, Margin, Rect, RichText, ScrollArea, Sense,
    Stroke, Vec2,
};
use std::path::{Path, PathBuf};

pub fn show(app: &mut App, ctx: &egui::Context) {
    left_panel(app, ctx);
    ui::metadata_panel::show(app, ctx);
    central(app, ctx);
}

fn left_panel(app: &mut App, ctx: &egui::Context) {
    let frame = Frame::none()
        .fill(theme::CARD)
        .inner_margin(Margin::symmetric(12.0, 12.0));

    egui::SidePanel::left("folder_panel")
        .resizable(false)
        .exact_width(276.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui::tree::show(app, ctx, ui);
        });
}

fn central(app: &mut App, ctx: &egui::Context) {
    let frame = Frame::none()
        .fill(theme::BG)
        .inner_margin(Margin::symmetric(12.0, 10.0));

    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        if app.current_folder.is_none() {
            welcome(app, ctx, ui);
            return;
        }

        folder_header(app, ctx, ui);
        ui.add_space(6.0);

        let available = ui.available_height();
        let grid_height = (available * 0.55).clamp(180.0, 560.0);
        grid(app, ctx, ui, grid_height);

        ui.add_space(8.0);
        preview(app, ctx, ui);
    });
}

fn welcome(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    theme::card().show(ui, |ui| {
        ui.set_min_height(ui.available_height());
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 32.0, theme::ACCENT_SOFT);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "AW",
                FontId::proportional(20.0),
                theme::ACCENT,
            );
            ui.add_space(14.0);
            ui.add(egui::Label::new(
                RichText::new("Open a stock folder to start").size(18.0).strong(),
            ));
            ui.add_space(5.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Browse This PC or a folder on the left, select one or many images, then run AI and write the metadata to EXIF/IPTC/XMP.",
                    )
                    .size(12.0)
                    .color(theme::TEXT_MUTED),
                )
                .wrap(),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add(ui::primary_button("Open Folder…")).clicked() {
                    app.open_folder_dialog(ctx);
                }
                if ui.add(ghost_button("Add images…")).clicked() {
                    app.add_images_dialog(ctx);
                }
            });
            ui.add_space(18.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Drag JPEGs or folders anywhere · Ctrl+Click toggles · Shift+Click selects a range",
                    )
                    .size(10.5)
                    .color(theme::TEXT_MUTED),
                )
                .wrap(),
            );
        });
    });
}

fn folder_header(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    let folder_label = app
        .current_folder
        .as_deref()
        .map(folder_name)
        .unwrap_or_else(|| "Images".to_string());

    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(folder_label).size(15.0).strong()));
        count_chip(ui, &format!("{}", app.folder_images.len()), true);
        if app.selection.count() > 0 {
            count_chip(ui, &format!("{} selected", app.selection.count()), true);
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(ghost_button("Refresh")).clicked() {
                app.refresh_browser(ctx);
            }
            if ui.add(ghost_button("Clear")).clicked() {
                app.clear_selection();
            }
            if ui
                .add_enabled(
                    !app.folder_images.is_empty(),
                    ghost_button("Select all"),
                )
                .clicked()
            {
                app.select_all(ctx);
            }
        });
    });
}

fn grid(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui, height: f32) {
    if app.folder_images.is_empty() {
        Frame::none()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(Margin::symmetric(16.0, 18.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("No JPEG images in this folder.")
                        .size(12.0)
                        .color(theme::TEXT_MUTED),
                );
            });
        return;
    }

    let paths = app.folder_images.clone();
    let card_w = 148.0;
    let card_h = 164.0;
    let spacing = ui.spacing().item_spacing.x.max(6.0);
    let columns = (((ui.available_width() + spacing) / (card_w + spacing)).floor() as usize).max(1);

    ScrollArea::vertical()
        .auto_shrink([false; 2])
        .max_height(height)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for chunk in paths.chunks(columns) {
                ui.horizontal(|ui| {
                    for path in chunk {
                        card(app, ctx, ui, path, card_w, card_h);
                    }
                });
                ui.add_space(6.0);
            }
        });
}

fn card(
    app: &mut App,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    path: &PathBuf,
    width: f32,
    height: f32,
) {
    let Some(item) = app.items.get(path).cloned() else {
        return;
    };
    let selected = app.selection.contains(path);
    let active = app.selection.active() == Some(path.as_path());
    let thumbnail = app
        .thumbs
        .texture(path)
        .map(|texture| (texture.id(), texture.size_vec2()));
    let loading = app.thumbs.is_loading(path);
    let failed = app.thumbs.has_failed(path);
    app.request_thumb(ctx, path);

    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = ui
        .ctx()
        .pointer_hover_pos()
        .map(|position| rect.contains(position))
        .unwrap_or(false);
    let painter = ui.painter_at(rect);

    let fill = if selected {
        theme::SELECTED
    } else if hovered {
        Color32::from_rgb(250.0 as u8, 252.0 as u8, 254.0 as u8)
    } else {
        theme::CARD
    };
    painter.rect_filled(rect, egui::Rounding::same(8.0), fill);
    let border = if active {
        Stroke::new(2.5_f32, theme::ACCENT)
    } else if selected {
        Stroke::new(1.8_f32, theme::ACCENT)
    } else if hovered {
        Stroke::new(1.2_f32, theme::BORDER_STRONG)
    } else {
        Stroke::new(1.0_f32, theme::BORDER)
    };
    painter.rect_stroke(rect, egui::Rounding::same(8.0), border);

    let image_rect = Rect::from_min_max(
        rect.min + egui::vec2(6.0, 6.0),
        egui::pos2(rect.max.x - 6.0, rect.max.y - 44.0),
    );
    painter.rect_filled(image_rect, egui::Rounding::same(5.0), theme::THUMB_BG);

    match thumbnail {
        Some((texture_id, size)) => {
            let scale = (image_rect.width() / size.x.max(1.0))
                .min(image_rect.height() / size.y.max(1.0));
            let fitted = Rect::from_center_size(image_rect.center(), size * scale);
            painter.image(
                texture_id,
                fitted,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let message = if failed {
                "Unreadable"
            } else if loading {
                "Loading…"
            } else {
                "JPG"
            };
            painter.text(
                image_rect.center(),
                Align2::CENTER_CENTER,
                message,
                FontId::proportional(10.0),
                theme::TEXT_MUTED,
            );
        }
    }

    let name = truncate(&item.file_name(), 21);
    painter.text(
        egui::pos2(rect.left() + 7.0, rect.bottom() - 35.0),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(10.5),
        theme::TEXT,
    );
    if let Some((width, height)) = item.dims {
        painter.text(
            egui::pos2(rect.right() - 7.0, rect.bottom() - 35.0),
            Align2::RIGHT_TOP,
            format!("{width}×{height}"),
            FontId::proportional(8.5),
            theme::TEXT_MUTED,
        );
    }

    let (label, fg, bg) = status_style(item.status, item.dirty);
    let badge_width = (label.chars().count() as f32 * 5.2 + 12.0).max(40.0);
    let badge = Rect::from_min_size(
        egui::pos2(rect.left() + 7.0, rect.bottom() - 19.0),
        Vec2::new(badge_width, 14.0),
    );
    painter.rect_filled(badge, egui::Rounding::same(999.0), bg);
    painter.text(
        badge.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(8.5),
        fg,
    );

    let checkbox_rect = Rect::from_min_size(rect.min + egui::vec2(7.0, 7.0), Vec2::splat(17.0));
    let checkbox_response = ui.interact(
        checkbox_rect,
        ui.make_persistent_id(("grid-checkbox", path)),
        Sense::click(),
    );
    painter.rect_filled(
        checkbox_rect,
        egui::Rounding::same(3.0),
        if selected {
            theme::ACCENT
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 235)
        },
    );
    painter.rect_stroke(
        checkbox_rect,
        egui::Rounding::same(3.0),
        Stroke::new(
            1.2_f32,
            if selected {
                theme::ACCENT
            } else if hovered {
                theme::BORDER_STRONG
            } else {
                Color32::from_rgba_unmultiplied(120, 120, 120, 150)
            },
        ),
    );
    if selected {
        painter.text(
            checkbox_rect.center() + egui::vec2(0.0, -1.0),
            Align2::CENTER_CENTER,
            "✓",
            FontId::proportional(11.0),
            Color32::WHITE,
        );
    }

    if checkbox_response.clicked() {
        app.toggle_selection(ctx, path);
    } else if response.clicked() {
        let modifiers = ctx.input(|input| SelectModifiers {
            ctrl: input.modifiers.ctrl || input.modifiers.command,
            shift: input.modifiers.shift,
        });
        app.apply_grid_click(ctx, path, modifiers);
    }

    if response.double_clicked() {
        app.mode = WorkspaceMode::Viewer;
    }

    response.on_hover_text(format!(
        "{}\n{}\n{}",
        item.path.display(),
        item.status.label(),
        if item.keywords.is_empty() {
            "No keywords yet".to_string()
        } else {
            format!("{} keywords", item.keywords.len())
        }
    ));
}

fn preview(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(path) = app.selection.primary(&app.folder_images) else {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new("Select an image to preview it here.")
                        .size(12.0)
                        .color(theme::TEXT_MUTED),
                )
                .wrap(),
            );
        });
        return;
    };

    app.request_preview(ctx, &path);
    let item = app.items.get(&path).cloned();
    let texture = app
        .preview
        .as_ref()
        .filter(|(current, _)| current == &path)
        .map(|(_, texture)| (texture.id(), texture.size_vec2()));

    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        item.as_ref()
                            .map(|item| item.file_name())
                            .unwrap_or_else(|| "Image".to_string()),
                    )
                    .size(13.5)
                    .strong(),
                )
                .truncate(),
            );
            if let Some(item) = &item {
                status_pill(ui, item.status, item.dirty);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(item) = &item {
                    if let Some((width, height)) = item.dims {
                        ui.add(egui::Label::new(
                            RichText::new(format!("{width} × {height} px"))
                                .size(10.5)
                                .color(theme::TEXT_MUTED),
                        ));
                    }
                }
                ui.add(egui::Label::new(
                    RichText::new("Double-click a thumbnail for Viewer")
                        .size(9.5)
                        .color(theme::TEXT_MUTED),
                ));
            });
        });

        ui.add_space(5.0);
        let height = ui.available_height().max(140.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::Rounding::same(7.0), theme::PREVIEW_BG);

        match texture {
            Some((texture_id, size)) => {
                let available = Vec2::new(
                    (rect.width() - 20.0).max(1.0),
                    (rect.height() - 20.0).max(1.0),
                );
                let scale = (available.x / size.x.max(1.0))
                    .min(available.y / size.y.max(1.0))
                    .min(1.0);
                let fitted = Rect::from_center_size(rect.center(), size * scale);
                painter.image(
                    texture_id,
                    fitted,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    if app.loading_preview.contains(&path) {
                        "Loading preview…"
                    } else {
                        "Preview unavailable"
                    },
                    FontId::proportional(13.0),
                    Color32::from_gray(150),
                );
            }
        }
    });
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let mut result: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        result.push('…');
        result
    }
}

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
        .fill(theme::card_fill())
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
        .fill(theme::bg())
        .inner_margin(Margin::symmetric(12.0, 10.0));

    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        if app.current_folder.is_none() {
            welcome(app, ctx, ui);
            return;
        }

        folder_header(app, ctx, ui);
        ui.add_space(6.0);

        let available = ui.available_height();
        let compact = app.cfg.density.is_compact();
        let grid_height = (available * if compact { 0.60 } else { 0.55 })
            .clamp(if compact { 150.0 } else { 180.0 }, 560.0);
        grid(app, ctx, ui, grid_height);

        ui.add_space(9.0);
        ui.add(egui::Label::new(
            RichText::new("ACTIVE PREVIEW")
                .size(9.0)
                .strong()
                .color(theme::text_muted()),
        ));
        ui.add_space(3.0);
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
                .circle_filled(rect.center(), 32.0, theme::accent_soft());
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "AW",
                FontId::proportional(20.0),
                theme::accent(),
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
                    .color(theme::text_muted()),
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
                    .color(theme::text_muted()),
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
    let summary = selection_summary(app);

    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(folder_label).size(15.0).strong()));
        count_chip(ui, &format!("{}", app.folder_images.len()), true);
        if summary.selected > 0 {
            count_chip(ui, &format!("{} selected", summary.selected), true);
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

    selection_summary_row(
        ui,
        summary,
        app.cfg.keyword_target(),
        app.cfg.density.is_compact(),
    );
}

#[derive(Default, Clone, Copy)]
struct SelectionSummary {
    selected: usize,
    tags: usize,
    covered: usize,
    pending: usize,
    unreadable: usize,
    tool_ready: bool,
    tool_checking: bool,
}

impl SelectionSummary {
    fn target_percent(self, keyword_target: usize) -> usize {
        if self.selected == 0 {
            return 0;
        }
        let target = self.selected.saturating_mul(keyword_target);
        if target == 0 {
            0
        } else {
            (self.tags.saturating_mul(100) / target).min(100)
        }
    }

    fn coverage_percent(self) -> usize {
        if self.selected == 0 {
            0
        } else {
            (self.covered.saturating_mul(100) / self.selected).min(100)
        }
    }
}

fn selection_summary(app: &App) -> SelectionSummary {
    let mut summary = SelectionSummary {
        tool_ready: app.exiftool.is_some(),
        tool_checking: app.exiftool_checking,
        ..SelectionSummary::default()
    };
    for path in &app.folder_images {
        if !app.selection.contains(path) {
            continue;
        }
        summary.selected += 1;
        if let Some(item) = app.items.get(path) {
            summary.tags += item.keywords.len();
            if !item.keywords.is_empty() {
                summary.covered += 1;
            }
            let empty = item.title.is_empty()
                && item.caption.is_empty()
                && item.keywords.is_empty();
            if empty {
                if item.meta_error {
                    summary.unreadable += 1;
                } else if !item.meta_loaded {
                    summary.pending += 1;
                }
            }
        }
    }
    summary
}

fn selection_summary_row(
    ui: &mut egui::Ui,
    summary: SelectionSummary,
    keyword_target: usize,
    compact: bool,
) {
    if summary.selected == 0 {
        return;
    }

    ui.add_space(if compact { 3.0 } else { 4.0 });
    let text_size = if compact { 10.0 } else { 10.5 };
    if summary.pending > 0 {
        let (message, color, help) = if summary.tool_ready {
            (
                format!(
                    "Loading metadata for {} of {} selected images…",
                    summary.pending, summary.selected
                ),
                theme::accent_text(),
                "Coverage appears after the selected images' metadata reads finish.",
            )
        } else if summary.tool_checking {
            (
                "Waiting for ExifTool…".to_string(),
                theme::text_muted(),
                "Coverage will load when ExifTool is ready.",
            )
        } else {
            (
                "ExifTool unavailable — metadata not loaded".to_string(),
                theme::warning(),
                "Install ExifTool or set its path, then reselect the images.",
            )
        };
        let row = Frame::none()
            .fill(theme::chip_bg())
            .stroke(Stroke::new(1.0_f32, theme::border()))
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(Margin::symmetric(
                if compact { 8.0 } else { 10.0 },
                if compact { 3.0 } else { 4.0 },
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing =
                        Vec2::new(if compact { 5.0 } else { 7.0 }, 2.0);
                    ui.add(egui::Label::new(
                        RichText::new(format!("{} selected", summary.selected))
                            .size(text_size)
                            .strong()
                            .color(theme::accent_text()),
                    ));
                    ui.add(egui::Label::new(
                        RichText::new("·")
                            .size(text_size + 1.0)
                            .color(theme::border_strong()),
                    ));
                    ui.add(egui::Label::new(
                        RichText::new(message).size(text_size).color(color),
                    ));
                });
            });
        row.response.on_hover_text(help);
        return;
    }

    let target_color = if summary.target_percent(keyword_target) >= 100 {
        theme::success()
    } else if summary.target_percent(keyword_target) >= 50 {
        theme::accent_text()
    } else {
        theme::warning()
    };
    let image_color = if summary.covered == summary.selected {
        theme::success()
    } else if summary.covered > 0 {
        theme::accent_text()
    } else {
        theme::warning()
    };

    let row = Frame::none()
        .fill(theme::chip_bg())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(Margin::symmetric(
            if compact { 8.0 } else { 10.0 },
            if compact { 3.0 } else { 4.0 },
        ))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(if compact { 5.0 } else { 7.0 }, 2.0);
                ui.add(egui::Label::new(
                    RichText::new(format!("{} selected", summary.selected))
                        .size(text_size)
                        .strong()
                        .color(theme::accent_text()),
                ));
                ui.add(egui::Label::new(
                    RichText::new("·")
                        .size(text_size + 1.0)
                        .color(theme::border_strong()),
                ));
                ui.add(egui::Label::new(
                    RichText::new(format!("{} keyword tags", summary.tags))
                        .size(text_size)
                        .color(theme::text()),
                ));
                ui.add(egui::Label::new(
                    RichText::new("·")
                        .size(text_size + 1.0)
                        .color(theme::border_strong()),
                ));
                ui.add(egui::Label::new(
                    RichText::new(format!(
                        "{}% of target",
                        summary.target_percent(keyword_target)
                    ))
                        .size(text_size)
                        .color(target_color),
                ));
                ui.add(egui::Label::new(
                    RichText::new("·")
                        .size(text_size + 1.0)
                        .color(theme::border_strong()),
                ));
                ui.add(egui::Label::new(
                    RichText::new(format!(
                        "{}/{} images keyworded ({}%)",
                        summary.covered,
                        summary.selected,
                        summary.coverage_percent()
                    ))
                    .size(text_size)
                    .color(image_color),
                ));
                if summary.unreadable > 0 {
                    ui.add(egui::Label::new(
                        RichText::new("·")
                            .size(text_size + 1.0)
                            .color(theme::border_strong()),
                    ));
                    ui.add(egui::Label::new(
                        RichText::new(format!("{} unreadable", summary.unreadable))
                            .size(text_size)
                            .color(theme::warning()),
                    ));
                }
            });
        });
    row.response.on_hover_text(format!(
        "Tag coverage compares total keywords with the {} keywords-per-image target. Image coverage counts selected images with at least one keyword; images whose metadata could not be read are counted as having no keywords.",
        keyword_target
    ));
}

fn grid(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui, height: f32) {
    if app.folder_images.is_empty() {
        Frame::none()
            .fill(theme::card_fill())
            .stroke(Stroke::new(1.0_f32, theme::border()))
            .rounding(egui::Rounding::same(10.0))
            .inner_margin(Margin::symmetric(18.0, 20.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("No JPEG images in this folder")
                        .size(14.0)
                        .strong()
                        .color(theme::text()),
                );
                ui.add_space(3.0);
                ui.label(
                    RichText::new(
                        "Choose another folder or add JPEGs to start keywording.",
                    )
                    .size(11.0)
                    .color(theme::text_muted()),
                );
                ui.add_space(8.0);
                if ui.add(ghost_button("Add images…")).clicked() {
                    app.add_images_dialog(ctx);
                }
            });
        return;
    }

    let paths = app.folder_images.clone();
    let compact = app.cfg.density.is_compact();
    let card_w = if compact { 126.0 } else { 156.0 };
    let card_h = if compact { 146.0 } else { 180.0 };
    let row_gap = if compact { 6.0 } else { 8.0 };
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
                        card(app, ctx, ui, path, card_w, card_h, compact);
                    }
                });
                ui.add_space(row_gap);
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
    compact: bool,
) {
    let Some(item) = app.items.get(path).cloned() else {
        return;
    };
    let edge = if compact { 5.0 } else { 7.0 };
    let footer = if compact { 42.0 } else { 50.0 };
    let thumb_font = if compact { 9.0 } else { 10.0 };
    let pill_gap = if compact { 4.0 } else { 5.0 };
    let pill_char = if compact { 4.4 } else { 4.9 };
    let pill_pad = if compact { 9.0 } else { 10.0 };
    let pill_min = if compact { 30.0 } else { 34.0 };
    let pill_offset = if compact { 17.0 } else { 19.0 };
    let pill_height = if compact { 13.0 } else { 14.0 };
    let pill_font = if compact { 7.2 } else { 8.0 };
    let name_chars = if compact { 15 } else { 19 };
    let name_top = if compact { 32.0 } else { 36.0 };
    let name_size = if compact { 9.8 } else { 10.8 };
    let badge_char = if compact { 4.8 } else { 5.2 };
    let badge_pad = if compact { 10.0 } else { 12.0 };
    let badge_min = if compact { 36.0 } else { 40.0 };
    let badge_top = if compact { 17.0 } else { 19.0 };
    let badge_height = if compact { 12.5 } else { 14.0 };
    let badge_font = if compact { 7.8 } else { 8.5 };
    let checkbox_size = if compact { 15.0 } else { 17.0 };
    let check_font = if compact { 10.0 } else { 11.0 };
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
        theme::selected()
    } else if hovered {
        theme::hover()
    } else {
        theme::card_fill()
    };
    painter.rect_filled(rect, egui::Rounding::same(9.0), fill);
    let border = if active {
        Stroke::new(2.5_f32, theme::accent())
    } else if selected {
        Stroke::new(1.8_f32, theme::accent())
    } else if hovered {
        Stroke::new(1.2_f32, theme::border_strong())
    } else {
        Stroke::new(1.0_f32, theme::border())
    };
    painter.rect_stroke(rect, egui::Rounding::same(9.0), border);

    let image_rect = Rect::from_min_max(
        rect.min + egui::vec2(edge, edge),
        egui::pos2(rect.max.x - edge, rect.max.y - footer),
    );
    painter.rect_filled(image_rect, egui::Rounding::same(6.0), theme::thumb_bg());

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
                FontId::proportional(thumb_font),
                theme::text_muted(),
            );
        }
    }

    if let Some((width, height)) = item.dims {
        let label = format!("{width} × {height}");
        let pill_width = (label.chars().count() as f32 * pill_char + pill_pad).max(pill_min);
        let pill = Rect::from_min_size(
            egui::pos2(
                image_rect.right() - pill_width - pill_gap,
                image_rect.bottom() - pill_offset,
            ),
            Vec2::new(pill_width, pill_height),
        );
        painter.rect_filled(
            pill,
            egui::Rounding::same(999.0),
            Color32::from_rgba_unmultiplied(20, 24, 30, 175),
        );
        painter.text(
            pill.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(pill_font),
            Color32::WHITE,
        );
    }

    let name = truncate(&item.file_name(), name_chars);
    painter.text(
        egui::pos2(rect.left() + edge + 1.0, rect.bottom() - name_top),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(name_size),
        theme::text(),
    );

    let (label, fg, bg) = status_style(item.status, item.dirty);
    let badge_width = (label.chars().count() as f32 * badge_char + badge_pad).max(badge_min);
    let badge = Rect::from_min_size(
        egui::pos2(rect.left() + edge + 1.0, rect.bottom() - badge_top),
        Vec2::new(badge_width, badge_height),
    );
    painter.rect_filled(badge, egui::Rounding::same(999.0), bg);
    painter.text(
        badge.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(badge_font),
        fg,
    );

    let checkbox_rect =
        Rect::from_min_size(rect.min + egui::vec2(edge, edge), Vec2::splat(checkbox_size));
    let checkbox_response = ui.interact(
        checkbox_rect,
        ui.make_persistent_id(("grid-checkbox", path)),
        Sense::click(),
    );
    painter.rect_filled(
        checkbox_rect,
        egui::Rounding::same(3.0),
        if selected {
            theme::accent()
        } else {
            theme::checkbox_bg()
        },
    );
    painter.rect_stroke(
        checkbox_rect,
        egui::Rounding::same(3.0),
        Stroke::new(
            1.2_f32,
            if selected {
                theme::accent()
            } else if hovered {
                theme::border_strong()
            } else {
                theme::text_muted()
            },
        ),
    );
    if selected {
        painter.text(
            checkbox_rect.center() + egui::vec2(0.0, -1.0),
            Align2::CENTER_CENTER,
            "✓",
            FontId::proportional(check_font),
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

    let dimensions = item
        .dims
        .map(|(width, height)| format!("\n{width} × {height} px"))
        .unwrap_or_default();
    response.on_hover_text(format!(
        "{}{}\n{}\n{}",
        item.path.display(),
        dimensions,
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
                        .color(theme::text_muted()),
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
                                .color(theme::text_muted()),
                        ));
                    }
                }
                ui.add(egui::Label::new(
                    RichText::new("Double-click a thumbnail for Viewer")
                        .size(9.5)
                        .color(theme::text_muted()),
                ));
            });
        });

        ui.add_space(5.0);
        let height = ui.available_height().max(140.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::Rounding::same(10.0), theme::preview_bg());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_summary_percentages_handle_empty_partial_and_full_sets() {
        let empty = SelectionSummary::default();
        assert_eq!(empty.target_percent(50), 0);
        assert_eq!(empty.coverage_percent(), 0);

        let partial = SelectionSummary {
            selected: 4,
            tags: 100,
            covered: 2,
            ..SelectionSummary::default()
        };
        assert_eq!(partial.target_percent(50), 50);
        assert_eq!(partial.coverage_percent(), 50);

        let full = SelectionSummary {
            selected: 2,
            tags: 100,
            covered: 2,
            ..SelectionSummary::default()
        };
        assert_eq!(full.target_percent(50), 100);
        assert_eq!(full.coverage_percent(), 100);

        let over_target = SelectionSummary {
            selected: 5,
            tags: 500,
            covered: 5,
            ..SelectionSummary::default()
        };
        assert_eq!(over_target.target_percent(50), 100);
        assert_eq!(over_target.coverage_percent(), 100);
    }
}

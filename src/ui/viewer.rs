use crate::app::{App, WorkspaceMode};
use crate::state::Status;
use crate::theme;
use crate::ui::{ghost_button, status_pill};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Layout, Margin, Rect, RichText, ScrollArea, Sense,
    Stroke, Vec2,
};

pub fn show(app: &mut App, ctx: &egui::Context) {
    crate::ui::metadata_panel::show(app, ctx);

    let frame = Frame::none()
        .fill(theme::BG)
        .inner_margin(Margin::symmetric(12.0, 10.0));

    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        toolbar(app, ctx, ui);
        ui.add_space(6.0);
        image(app, ctx, ui);
        ui.add_space(6.0);
        filmstrip(app, ctx, ui);
    });
}

fn toolbar(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    let active = app.selection.primary(&app.folder_images);
    let index = active
        .as_ref()
        .and_then(|path| app.folder_images.iter().position(|item| item == path));
    let total = app.folder_images.len();

    ui.horizontal(|ui| {
        if ui.add(ghost_button("← Browser")).clicked() {
            app.mode = WorkspaceMode::Browser;
        }
        ui.separator();
        if ui
            .add_enabled(
                index.map(|index| index > 0).unwrap_or(false),
                ghost_button("‹ Previous"),
            )
            .clicked()
        {
            app.step_active(ctx, -1);
        }
        if ui
            .add_enabled(
                index.map(|index| index + 1 < total).unwrap_or(false),
                ghost_button("Next ›"),
            )
            .clicked()
        {
            app.step_active(ctx, 1);
        }
        ui.separator();

        if let Some(active) = &active {
            let name = active
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| active.display().to_string());
            ui.add(egui::Label::new(RichText::new(name).size(14.0).strong()).truncate());
            if let Some(item) = app.items.get(active) {
                status_pill(ui, item.status, item.dirty);
            }
        } else {
            ui.add(egui::Label::new(
                RichText::new("No image selected").size(12.0).color(theme::TEXT_MUTED),
            ));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(index) = index {
                ui.add(egui::Label::new(
                    RichText::new(format!("{} / {}", index + 1, total))
                        .size(10.5)
                        .color(theme::TEXT_MUTED),
                ));
            }
            if let Some(active) = &active {
                if let Some(item) = app.items.get(active) {
                    if let Some((width, height)) = item.dims {
                        ui.add(egui::Label::new(
                            RichText::new(format!("{width} × {height} px"))
                                .size(10.5)
                                .color(theme::TEXT_MUTED),
                        ));
                    }
                }
            }
        });
    });
}

fn image(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(path) = app.selection.primary(&app.folder_images) else {
        theme::card().show(ui, |ui| {
            ui.set_min_height(ui.available_height().max(220.0));
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.label(
                    RichText::new("Select an image in Browser mode, then press Viewer.")
                        .size(13.0)
                        .color(theme::TEXT_MUTED),
                );
            });
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

    let height = (ui.available_height() - 88.0).max(180.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, egui::Rounding::same(8.0), theme::PREVIEW_BG);

    match texture {
        Some((texture_id, size)) => {
            let available = Vec2::new(
                (rect.width() - 28.0).max(1.0),
                (rect.height() - 28.0).max(1.0),
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

    if let Some(item) = &item {
        if let Some(message) = &item.message {
            let message_rect = Rect::from_min_size(
                egui::pos2(rect.left() + 12.0, rect.bottom() - 42.0),
                Vec2::new(rect.width() - 24.0, 30.0),
            );
            painter.rect_filled(
                message_rect,
                egui::Rounding::same(5.0),
                Color32::from_rgba_unmultiplied(30, 30, 30, 210),
            );
            painter.text(
                message_rect.left_center() + egui::vec2(8.0, 0.0),
                Align2::LEFT_CENTER,
                message,
                FontId::proportional(10.0),
                theme::DANGER_SOFT,
            );
        }
    }
}

fn filmstrip(app: &mut App, ctx: &egui::Context, ui: &mut egui::Ui) {
    if app.folder_images.is_empty() {
        return;
    }
    let paths = app.folder_images.clone();
    ScrollArea::horizontal()
        .auto_shrink([false, true])
        .max_height(82.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for path in paths {
                    let selected = app.selection.active() == Some(path.as_path());
                    let texture = app
                        .thumbs
                        .texture(&path)
                        .map(|texture| (texture.id(), texture.size_vec2()));
                    app.request_thumb(ctx, &path);

                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(58.0), Sense::click());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(
                        rect,
                        egui::Rounding::same(5.0),
                        if selected { theme::SELECTED } else { theme::THUMB_BG },
                    );
                    painter.rect_stroke(
                        rect,
                        egui::Rounding::same(5.0),
                        Stroke::new(
                            if selected { 2.0_f32 } else { 1.0_f32 },
                            if selected { theme::ACCENT } else { theme::BORDER },
                        ),
                    );

                    if let Some((texture_id, size)) = texture {
                        let inner = rect.shrink(4.0);
                        let scale = (inner.width() / size.x.max(1.0))
                            .min(inner.height() / size.y.max(1.0));
                        painter.image(
                            texture_id,
                            Rect::from_center_size(inner.center(), size * scale),
                            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "JPG",
                            FontId::proportional(9.0),
                            theme::TEXT_MUTED,
                        );
                    }

                    if let Some(item) = app.items.get(&path) {
                        let color = match item.status {
                            Status::Saved => theme::SUCCESS,
                            Status::Generated => theme::VIOLET,
                            Status::Processing | Status::Queued => theme::ACCENT,
                            Status::Error => theme::DANGER,
                            Status::Pending => Color32::TRANSPARENT,
                        };
                        if color != Color32::TRANSPARENT {
                            painter.circle_filled(
                                rect.right_top() + egui::vec2(-6.0, 6.0),
                                3.0,
                                color,
                            );
                        }
                    }

                    if response.clicked() {
                        app.select_single(ctx, &path);
                    }
                    response.on_hover_text(
                        path.file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    );
                }
            });
        });
}

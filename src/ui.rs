use crate::state::Status;
use crate::theme;
use eframe::egui::{self, Button, Color32, RichText, Stroke, Ui};

pub fn section_title(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(14.0).strong()));
}

pub fn primary_button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text.into()).size(12.5).color(Color32::WHITE))
        .fill(theme::ACCENT)
        .stroke(Stroke::NONE)
        .rounding(egui::Rounding::same(6.0))
        .min_size(egui::vec2(0.0, 28.0))
}

pub fn ghost_button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text.into()).size(12.5).color(theme::TEXT))
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0_f32, theme::BORDER_STRONG))
        .rounding(egui::Rounding::same(6.0))
        .min_size(egui::vec2(0.0, 28.0))
}

pub fn tiny_button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text.into()).size(11.0).color(theme::TEXT_MUTED)).frame(false)
}

pub fn text_button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text.into()).size(11.5).color(theme::TEXT_MUTED)).frame(false)
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

pub fn status_pill(ui: &mut Ui, status: Status, dirty: bool) {
    let (text, fg, bg) = if dirty && matches!(status, Status::Saved | Status::Generated) {
        ("Modified", theme::WARNING, theme::WARNING_SOFT)
    } else {
        match status {
            Status::Pending => ("Pending", Color32::from_rgb(97, 97, 97), theme::CHIP_BG),
            Status::Processing => ("Processing", theme::ACCENT_TEXT, theme::ACCENT_SOFT),
            Status::Generated => ("Generated", theme::VIOLET, theme::VIOLET_SOFT),
            Status::Saved => ("Saved", theme::SUCCESS, theme::SUCCESS_SOFT),
            Status::Error => ("Failed", theme::DANGER, theme::DANGER_SOFT),
        }
    };

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

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke, TextStyle,
};

pub const ACCENT: Color32 = Color32::from_rgb(0, 102, 184);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(223, 238, 252);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0, 84, 153);
pub const BG: Color32 = Color32::from_rgb(243, 243, 243);
pub const CARD: Color32 = Color32::WHITE;
pub const BORDER: Color32 = Color32::from_rgb(229, 229, 229);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(209, 209, 209);
pub const TEXT: Color32 = Color32::from_rgb(26, 26, 26);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(96, 96, 96);
pub const SUCCESS: Color32 = Color32::from_rgb(16, 124, 16);
pub const SUCCESS_SOFT: Color32 = Color32::from_rgb(223, 246, 221);
pub const WARNING: Color32 = Color32::from_rgb(157, 93, 0);
pub const WARNING_SOFT: Color32 = Color32::from_rgb(255, 244, 206);
pub const DANGER: Color32 = Color32::from_rgb(196, 43, 28);
pub const DANGER_SOFT: Color32 = Color32::from_rgb(253, 231, 233);
pub const VIOLET: Color32 = Color32::from_rgb(91, 45, 144);
pub const VIOLET_SOFT: Color32 = Color32::from_rgb(242, 235, 250);
pub const CHIP_BG: Color32 = Color32::from_rgb(245, 245, 245);
pub const SELECTED: Color32 = Color32::from_rgb(232, 242, 252);
pub const PREVIEW_BG: Color32 = Color32::from_rgb(31, 31, 31);
pub const THUMB_BG: Color32 = Color32::from_rgb(238, 238, 238);

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.floating = true;
    style.text_styles = text_styles();
    ctx.set_style(style);
}

/// Load the Windows 11 system font so the app feels native on Windows.
fn install_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\SegoeUI.ttf",
    ];
    let mut bytes = None;
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            bytes = Some(data);
            break;
        }
    }

    let Some(bytes) = bytes else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("segoe".to_owned(), FontData::from_owned(bytes));
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "segoe".to_owned());
    ctx.set_fonts(fonts);
}

fn text_styles() -> std::collections::BTreeMap<TextStyle, FontId> {
    let mut styles = std::collections::BTreeMap::new();
    styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
    styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
    styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
    styles.insert(TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional));
    styles.insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
    styles
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_rgb(250, 250, 250);
    v.code_bg_color = Color32::from_rgb(245, 245, 245);
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(8.0);

    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.weak_bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.widgets.inactive.bg_fill = CARD;
    v.widgets.inactive.weak_bg_fill = CARD;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.widgets.hovered.bg_fill = Color32::from_rgb(248, 249, 250);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(248, 249, 250);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(170, 170, 170));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.widgets.active.bg_fill = ACCENT_SOFT;
    v.widgets.active.weak_bg_fill = ACCENT_SOFT;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.widgets.open.bg_fill = CARD;
    v.widgets.open.weak_bg_fill = CARD;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = Rounding::same(6.0);
    }

    v
}

pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(10.0))
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
}

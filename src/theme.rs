use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke, TextStyle,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// Runtime palette shared by every custom-painted surface in the UI.
#[derive(Clone, Copy)]
pub struct Palette {
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub bg: Color32,
    pub card: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub hover: Color32,
    pub success: Color32,
    pub success_soft: Color32,
    pub warning: Color32,
    pub warning_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub danger_bright: Color32,
    pub violet: Color32,
    pub violet_soft: Color32,
    pub chip_bg: Color32,
    pub selected: Color32,
    pub preview_bg: Color32,
    pub thumb_bg: Color32,
    pub input_bg: Color32,
    pub checkbox_bg: Color32,
}

const LIGHT: Palette = Palette {
    accent: Color32::from_rgb(0, 103, 192),
    accent_soft: Color32::from_rgb(230, 241, 252),
    accent_text: Color32::from_rgb(0, 82, 155),
    bg: Color32::from_rgb(246, 247, 249),
    card: Color32::WHITE,
    border: Color32::from_rgb(228, 231, 236),
    border_strong: Color32::from_rgb(203, 209, 219),
    text: Color32::from_rgb(31, 41, 55),
    text_muted: Color32::from_rgb(100, 116, 139),
    hover: Color32::from_rgb(250, 252, 255),
    success: Color32::from_rgb(22, 120, 66),
    success_soft: Color32::from_rgb(226, 246, 235),
    warning: Color32::from_rgb(180, 100, 0),
    warning_soft: Color32::from_rgb(255, 246, 224),
    danger: Color32::from_rgb(197, 48, 48),
    danger_soft: Color32::from_rgb(253, 235, 236),
    danger_bright: Color32::from_rgb(255, 205, 205),
    violet: Color32::from_rgb(109, 40, 217),
    violet_soft: Color32::from_rgb(243, 238, 254),
    chip_bg: Color32::from_rgb(244, 246, 249),
    selected: Color32::from_rgb(235, 244, 254),
    preview_bg: Color32::from_rgb(24, 26, 30),
    thumb_bg: Color32::from_rgb(240, 242, 245),
    input_bg: Color32::WHITE,
    checkbox_bg: Color32::from_rgb(250, 252, 255),
};

const DARK: Palette = Palette {
    accent: Color32::from_rgb(37, 111, 196),
    accent_soft: Color32::from_rgb(24, 49, 76),
    accent_text: Color32::from_rgb(124, 184, 245),
    bg: Color32::from_rgb(18, 21, 26),
    card: Color32::from_rgb(27, 32, 39),
    border: Color32::from_rgb(44, 52, 63),
    border_strong: Color32::from_rgb(70, 82, 99),
    text: Color32::from_rgb(232, 236, 242),
    text_muted: Color32::from_rgb(154, 164, 180),
    hover: Color32::from_rgb(35, 42, 52),
    success: Color32::from_rgb(90, 210, 145),
    success_soft: Color32::from_rgb(23, 52, 38),
    warning: Color32::from_rgb(238, 183, 92),
    warning_soft: Color32::from_rgb(56, 44, 22),
    danger: Color32::from_rgb(242, 124, 124),
    danger_soft: Color32::from_rgb(62, 31, 34),
    danger_bright: Color32::from_rgb(255, 196, 196),
    violet: Color32::from_rgb(183, 158, 255),
    violet_soft: Color32::from_rgb(45, 37, 68),
    chip_bg: Color32::from_rgb(36, 43, 53),
    selected: Color32::from_rgb(27, 54, 83),
    preview_bg: Color32::from_rgb(9, 11, 14),
    thumb_bg: Color32::from_rgb(36, 43, 53),
    input_bg: Color32::from_rgb(21, 25, 31),
    checkbox_bg: Color32::from_rgb(30, 36, 44),
};

static DARK_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_dark(dark: bool) {
    DARK_MODE.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}

pub fn palette() -> Palette {
    if is_dark() {
        DARK
    } else {
        LIGHT
    }
}

pub fn accent() -> Color32 {
    palette().accent
}

pub fn accent_soft() -> Color32 {
    palette().accent_soft
}

pub fn accent_text() -> Color32 {
    palette().accent_text
}

pub fn bg() -> Color32 {
    palette().bg
}

pub fn card_fill() -> Color32 {
    palette().card
}

pub fn border() -> Color32 {
    palette().border
}

pub fn border_strong() -> Color32 {
    palette().border_strong
}

pub fn text() -> Color32 {
    palette().text
}

pub fn text_muted() -> Color32 {
    palette().text_muted
}

pub fn hover() -> Color32 {
    palette().hover
}

pub fn success() -> Color32 {
    palette().success
}

pub fn success_soft() -> Color32 {
    palette().success_soft
}

pub fn warning() -> Color32 {
    palette().warning
}

pub fn warning_soft() -> Color32 {
    palette().warning_soft
}

pub fn danger() -> Color32 {
    palette().danger
}

pub fn danger_soft() -> Color32 {
    palette().danger_soft
}

pub fn danger_bright() -> Color32 {
    palette().danger_bright
}

pub fn violet() -> Color32 {
    palette().violet
}

pub fn violet_soft() -> Color32 {
    palette().violet_soft
}

pub fn chip_bg() -> Color32 {
    palette().chip_bg
}

pub fn selected() -> Color32 {
    palette().selected
}

pub fn preview_bg() -> Color32 {
    palette().preview_bg
}

pub fn thumb_bg() -> Color32 {
    palette().thumb_bg
}

pub fn checkbox_bg() -> Color32 {
    palette().checkbox_bg
}

/// Install fonts once, then apply the selected appearance.
pub fn setup(ctx: &egui::Context, dark: bool) {
    install_fonts(ctx);
    apply(ctx, dark);
}

/// Apply the selected appearance. Safe to call again when the theme changes.
pub fn apply(ctx: &egui::Context, dark: bool) {
    set_dark(dark);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(11.0, 5.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.floating = true;
    style.text_styles = text_styles();
    ctx.set_style(style);
}

/// Read the Windows app-theme preference used by Settings → Personalization.
#[cfg(windows)]
pub fn system_prefers_dark() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("reg");
    command.args([
        "query",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "/v",
        "AppsUseLightTheme",
    ]);
    command.creation_flags(CREATE_NO_WINDOW);
    match command.output() {
        Ok(output) if output.status.success() => {
            prefers_dark_from_reg_output(&String::from_utf8_lossy(&output.stdout))
        }
        _ => false,
    }
}

#[cfg(not(windows))]
pub fn system_prefers_dark() -> bool {
    false
}

#[cfg(any(windows, test))]
fn prefers_dark_from_reg_output(output: &str) -> bool {
    output
        .lines()
        .find(|line| line.contains("AppsUseLightTheme"))
        .and_then(|line| line.split_whitespace().last())
        .map(|value| value.eq_ignore_ascii_case("0x0") || value == "0")
        .unwrap_or(false)
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
    styles.insert(TextStyle::Button, FontId::new(12.5, FontFamily::Proportional));
    styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
    styles.insert(TextStyle::Heading, FontId::new(18.5, FontFamily::Proportional));
    styles.insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
    styles
}

fn visuals() -> egui::Visuals {
    let c = palette();
    let mut v = if is_dark() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    v.override_text_color = Some(c.text);
    v.panel_fill = c.bg;
    v.window_fill = c.card;
    v.extreme_bg_color = c.input_bg;
    v.faint_bg_color = c.chip_bg;
    v.code_bg_color = c.input_bg;
    v.selection.bg_fill = c.selected;
    v.selection.stroke = Stroke::new(1.0_f32, c.accent);
    v.hyperlink_color = c.accent_text;
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(10.0);
    v.window_stroke = Stroke::new(1.0_f32, c.border);

    v.widgets.noninteractive.bg_fill = c.card;
    v.widgets.noninteractive.weak_bg_fill = c.card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, c.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, c.text);

    v.widgets.inactive.bg_fill = c.card;
    v.widgets.inactive.weak_bg_fill = c.card;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, c.border_strong);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, c.text);

    v.widgets.hovered.bg_fill = c.hover;
    v.widgets.hovered.weak_bg_fill = c.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, c.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, c.text);

    v.widgets.active.bg_fill = c.accent_soft;
    v.widgets.active.weak_bg_fill = c.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, c.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, c.text);

    v.widgets.open.bg_fill = c.card;
    v.widgets.open.weak_bg_fill = c.card;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, c.border);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, c.text);

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = Rounding::same(7.0);
    }

    v
}

pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(card_fill())
        .stroke(Stroke::new(1.0_f32, border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(egui::Margin::symmetric(16.0, 14.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_app_theme_registry_value_is_parsed() {
        assert!(prefers_dark_from_reg_output(
            "    AppsUseLightTheme    REG_DWORD    0x0"
        ));
        assert!(!prefers_dark_from_reg_output(
            "    AppsUseLightTheme    REG_DWORD    0x1"
        ));
        assert!(!prefers_dark_from_reg_output(""));
    }

    #[test]
    fn switching_palette_updates_egui_visuals() {
        let ctx = egui::Context::default();

        apply(&ctx, true);
        assert!(is_dark());
        let dark = ctx.style();
        assert_eq!(dark.visuals.panel_fill, DARK.bg);
        assert_eq!(dark.visuals.window_fill, DARK.card);
        assert_eq!(dark.visuals.extreme_bg_color, DARK.input_bg);

        apply(&ctx, false);
        assert!(!is_dark());
        assert_eq!(ctx.style().visuals.panel_fill, LIGHT.bg);
    }
}

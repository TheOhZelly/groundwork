//! Visual style: a dark, rounded theme with an emerald accent.

use eframe::egui::{self, vec2, Color32, FontFamily, FontId, Margin, Rounding, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0d, 0x11);
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x16, 0x1c);
pub const SURFACE: Color32 = Color32::from_rgb(0x1a, 0x1e, 0x26);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x23, 0x28, 0x33);
/// Buttons, slider rails and text fields: a step lighter than cards so
/// they stay visible on them.
pub const CONTROL: Color32 = Color32::from_rgb(0x26, 0x2b, 0x36);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x30, 0x36, 0x43);
pub const BORDER: Color32 = Color32::from_rgb(0x27, 0x2c, 0x37);
pub const TEXT: Color32 = Color32::from_rgb(0xe7, 0xe9, 0xef);
pub const MUTED: Color32 = Color32::from_rgb(0x8a, 0x92, 0xa6);
pub const ACCENT: Color32 = Color32::from_rgb(0x34, 0xd3, 0x99);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x12, 0x3d, 0x30);
pub const DANGER: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);

/// Uses Segoe UI for text, with Segoe UI Symbol and Emoji as fallbacks for
/// the icons, when running on Windows. egui's bundled fonts lack many of the
/// symbols the UI uses. Elsewhere the bundled fonts are kept.
fn load_fonts(ctx: &egui::Context) {
    let Some(windir) = std::env::var_os("WINDIR") else {
        return;
    };
    let dir = std::path::Path::new(&windir).join("Fonts");
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, file: &str| {
        let bytes = std::fs::read(dir.join(file)).ok()?;
        fonts
            .font_data
            .insert(name.to_owned(), egui::FontData::from_owned(bytes));
        Some(name.to_owned())
    };
    let text = load("segoe-ui", "segoeui.ttf");
    let symbols: Vec<String> = [
        ("segoe-symbol", "seguisym.ttf"),
        ("segoe-emoji", "seguiemj.ttf"),
    ]
    .into_iter()
    .filter_map(|(n, f)| load(n, f))
    .collect();
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if let Some(text) = text {
        proportional.insert(0, text);
    }
    proportional.extend(symbols.iter().cloned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .extend(symbols);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    load_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = SURFACE;
    v.extreme_bg_color = BG;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = BG;
    v.window_rounding = Rounding::same(14.0);
    v.menu_rounding = Rounding::same(10.0);
    v.window_stroke = Stroke::new(1.0f32, BORDER);
    v.window_shadow = egui::epaint::Shadow {
        offset: vec2(0.0, 12.0),
        blur: 32.0,
        spread: 0.0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: vec2(0.0, 6.0),
        blur: 16.0,
        spread: 0.0,
        color: Color32::from_black_alpha(120),
    };
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);

    let w = &mut v.widgets;
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.rounding = Rounding::same(8.0);
        state.expansion = 0.0;
    }
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0f32, TEXT);
    w.inactive.bg_fill = CONTROL;
    w.inactive.weak_bg_fill = CONTROL;
    w.inactive.bg_stroke = Stroke::new(1.0f32, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0f32, TEXT);
    w.hovered.bg_fill = CONTROL_HOVER;
    w.hovered.weak_bg_fill = CONTROL_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0f32, Color32::from_rgb(0x3a, 0x41, 0x50));
    w.hovered.fg_stroke = Stroke::new(1.0f32, Color32::WHITE);
    w.active.bg_fill = ACCENT_DIM;
    w.active.weak_bg_fill = ACCENT_DIM;
    w.active.bg_stroke = Stroke::new(1.0f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0f32, Color32::WHITE);
    w.open = w.hovered;

    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 8.0);
    s.button_padding = vec2(12.0, 6.0);
    s.interact_size.y = 30.0;
    s.slider_width = 150.0;
    s.window_margin = Margin::same(18.0);
    s.combo_width = 180.0;

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();
    ctx.set_style(style);
}

/// A rounded card used to group controls in side panels.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(SURFACE)
        .rounding(Rounding::same(12.0))
        .stroke(Stroke::new(1.0f32, BORDER))
        .inner_margin(Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(title.to_uppercase())
                    .small()
                    .strong()
                    .color(MUTED),
            );
            ui.add_space(2.0);
            add(ui)
        })
        .inner
}

/// The emerald call-to-action button.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).strong().color(BG))
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(8.0)),
    )
}

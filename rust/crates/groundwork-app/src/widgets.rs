//! Custom pickers with colour swatches and search.

use crate::theme;
use eframe::egui::{self, vec2, Align2, Color32, FontId, Rounding, Sense, Stroke};
use groundwork_core::render::terrain_colour;
use groundwork_core::terrain::Fidelity;
use groundwork_core::{biomes, Terrain};

fn search_box(ui: &mut egui::Ui, filter: &mut String, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text(format!("🔍 {hint}"))
            .desired_width(f32::INFINITY),
    );
}

/// One selectable row: optional swatch, label, trailing note.
fn row(
    ui: &mut egui::Ui,
    selected: bool,
    swatch: Option<Color32>,
    label: &str,
    note: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect(
            rect,
            Rounding::same(7.0),
            theme::ACCENT_DIM,
            Stroke::new(1.0f32, theme::ACCENT),
        );
    } else if response.hovered() {
        painter.rect_filled(rect, Rounding::same(7.0), theme::SURFACE_HOVER);
    }
    let mut x = rect.left() + 8.0;
    if let Some(colour) = swatch {
        let sw =
            egui::Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0));
        painter.rect(
            sw,
            Rounding::same(4.0),
            colour,
            Stroke::new(1.0f32, Color32::from_white_alpha(30)),
        );
        x += 24.0;
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        theme::TEXT,
    );
    if !note.is_empty() {
        painter.text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            note,
            FontId::proportional(11.0),
            theme::MUTED,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn colour(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub fn terrain_picker(
    ui: &mut egui::Ui,
    id: &str,
    selected: &mut Terrain,
    filter: &mut String,
    height: f32,
) {
    search_box(ui, filter, "Search terrain");
    let needle = filter.to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for info in Terrain::all() {
                if info.fidelity == Fidelity::Custom
                    || (!needle.is_empty() && !info.name.to_lowercase().contains(&needle))
                {
                    continue;
                }
                let note = if info.fidelity == Fidelity::Approximate {
                    "simplified"
                } else {
                    ""
                };
                let response = row(
                    ui,
                    *selected == info.terrain,
                    Some(colour(terrain_colour(info.terrain))),
                    info.name,
                    note,
                );
                if response.clicked() {
                    *selected = info.terrain;
                }
            }
        });
}

pub fn biome_picker(ui: &mut egui::Ui, selected: &mut u8, filter: &mut String) {
    search_box(ui, filter, "Search biomes");
    let needle = filter.to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("biomes")
        .max_height(260.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let auto = (biomes::BIOME_AUTO, "Automatic", "from terrain");
            let all =
                std::iter::once(auto).chain(biomes::all().map(|(id, name, _)| (id, name, "")));
            for (id, name, note) in all {
                if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
                    continue;
                }
                if row(ui, *selected == id, None, name, note).clicked() {
                    *selected = id;
                }
            }
        });
}

/// A row of mutually exclusive pill buttons.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let width =
            (ui.available_width() - 4.0 * (options.len() as f32 - 1.0)) / options.len() as f32;
        for &(option, label) in options {
            let button = egui::Button::new(label)
                .selected(*value == option)
                .min_size(vec2(width, 28.0));
            if ui.add(button).clicked() {
                *value = option;
            }
        }
    });
}

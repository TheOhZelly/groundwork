//! Modal dialogs: new world, export, and confirming lost changes.

use crate::theme;
use crate::widgets::terrain_picker;
use eframe::egui::{self, RichText};
use groundwork_core::generate::GenerateOptions;
use groundwork_core::{DimensionKind, Terrain};
use groundwork_export::{McVersion, Outside};
use std::path::PathBuf;

pub enum Dialog {
    NewWorld(NewWorldForm),
    Export(ExportForm),
    ImportHeightMap(HeightMapForm),
    Confirm(PendingAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    New,
    Open,
    Quit,
}

pub enum Outcome<T> {
    Open,
    Cancel,
    Accept(T),
}

/// Shows `add` in a centred, non-collapsible window over a dimmed backdrop.
pub fn modal<R>(
    ctx: &egui::Context,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new("modal_backdrop"))
        .fixed_pos(screen.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.painter()
                .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(150));
            ui.allocate_rect(screen, egui::Sense::click());
        });
    egui::Window::new(RichText::new(title).heading().strong())
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([width, 0.0])
        .show(ctx, add)
        .and_then(|r| r.inner)
}

// ------------------------------------------------------------- new world --

#[derive(Debug, Clone)]
pub struct NewWorldForm {
    pub name: String,
    pub width: u32,
    pub length: u32,
    pub seed: String,
    pub base_height: f32,
    pub water_level: i32,
    pub hilliness: f32,
    pub feature_size: f32,
    pub terrain: Terrain,
    pub terrain_filter: String,
    pub beaches: bool,
}

impl Default for NewWorldForm {
    fn default() -> Self {
        let mut form = NewWorldForm {
            name: "New World".into(),
            width: 6,
            length: 6,
            seed: String::new(),
            base_height: 0.0,
            water_level: 0,
            hilliness: 0.0,
            feature_size: 0.0,
            terrain: Terrain::Grass,
            terrain_filter: String::new(),
            beaches: true,
        };
        form.preset(PRESETS[0]);
        form
    }
}

#[derive(Clone, Copy)]
struct Preset {
    name: &'static str,
    icon: &'static str,
    base: f32,
    water: i32,
    hills: f32,
    feature: f32,
}

const PRESETS: [Preset; 4] = [
    Preset {
        name: "Islands",
        icon: "🏝",
        base: 56.0,
        water: 62,
        hills: 22.0,
        feature: 260.0,
    },
    Preset {
        name: "Continent",
        icon: "🗺",
        base: 74.0,
        water: 62,
        hills: 28.0,
        feature: 600.0,
    },
    Preset {
        name: "Mountains",
        icon: "⛰",
        base: 96.0,
        water: 62,
        hills: 70.0,
        feature: 320.0,
    },
    Preset {
        name: "Flat",
        icon: "▭",
        base: 64.0,
        water: 62,
        hills: 0.0,
        feature: 200.0,
    },
];

/// Minecraft turns text seeds into numbers with Java's `String.hashCode`.
fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

impl NewWorldForm {
    fn preset(&mut self, p: Preset) {
        self.base_height = p.base;
        self.water_level = p.water;
        self.hilliness = p.hills;
        self.feature_size = p.feature;
    }

    fn matches(&self, p: &Preset) -> bool {
        self.base_height == p.base
            && self.water_level == p.water
            && self.hilliness == p.hills
            && self.feature_size == p.feature
    }

    pub fn options(&self) -> GenerateOptions {
        let seed_text = self.seed.trim();
        let seed = if seed_text.is_empty() {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as u64)
        } else if let Ok(n) = seed_text.parse::<i64>() {
            n as u64
        } else {
            java_string_hash(seed_text) as i64 as u64
        };
        GenerateOptions {
            kind: DimensionKind::Overworld,
            width_tiles: self.width,
            length_tiles: self.length,
            base_height: self.base_height,
            water_level: self.water_level,
            hilliness: self.hilliness,
            feature_size: self.feature_size,
            terrain: self.terrain,
            beaches: self.beaches,
            seed,
            ..GenerateOptions::default()
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome<()> {
        let mut outcome = Outcome::Open;
        modal(ctx, "New world", 460.0, |ui| {
            ui.label(
                RichText::new("Pick a starting landscape. You can sculpt everything afterwards.")
                    .color(theme::MUTED),
            );
            ui.add_space(6.0);
            ui.columns(PRESETS.len(), |cols| {
                for (col, preset) in cols.iter_mut().zip(PRESETS) {
                    let selected = self.matches(&preset);
                    let text =
                        RichText::new(format!("{}\n{}", preset.icon, preset.name)).size(13.0);
                    let button = egui::Button::new(text)
                        .min_size(egui::vec2(col.available_width(), 58.0))
                        .selected(selected);
                    if col.add(button).clicked() {
                        self.preset(preset);
                    }
                }
            });
            ui.add_space(8.0);
            egui::Grid::new("new_world_grid")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
                    ui.end_row();
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.width)
                                .range(1..=32)
                                .suffix(" tiles"),
                        );
                        ui.label("×");
                        ui.add(
                            egui::DragValue::new(&mut self.length)
                                .range(1..=32)
                                .suffix(" tiles"),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{}×{} blocks",
                                self.width * 128,
                                self.length * 128
                            ))
                            .small()
                            .color(theme::MUTED),
                        );
                    });
                    ui.end_row();
                    ui.label("Seed");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.seed)
                            .hint_text("random")
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                    ui.label("Land height");
                    ui.add(egui::Slider::new(&mut self.base_height, -32.0..=200.0).step_by(1.0));
                    ui.end_row();
                    ui.label("Sea level");
                    ui.add(egui::Slider::new(&mut self.water_level, -64..=200));
                    ui.end_row();
                    ui.label("Hilliness");
                    ui.add(
                        egui::Slider::new(&mut self.hilliness, 0.0..=120.0)
                            .step_by(1.0)
                            .suffix(" blocks"),
                    );
                    ui.end_row();
                    ui.label("Hill size");
                    ui.add(
                        egui::Slider::new(&mut self.feature_size, 40.0..=1200.0)
                            .step_by(10.0)
                            .suffix(" blocks"),
                    );
                    ui.end_row();
                    ui.label("Beaches");
                    ui.checkbox(&mut self.beaches, "Around the water line");
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.label(RichText::new("Ground").color(theme::MUTED));
            terrain_picker(
                ui,
                "new_world_terrain",
                &mut self.terrain,
                &mut self.terrain_filter,
                140.0,
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, "Create world").clicked() {
                    outcome = Outcome::Accept(());
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Outcome::Cancel;
        }
        outcome
    }
}

// ------------------------------------------------------------ height map --

#[derive(Clone)]
pub struct HeightMapForm {
    pub path: String,
    pub low: f32,
    pub high: f32,
    pub scale: f32,
    pub water_level: i32,
    pub terrain: Terrain,
    pub terrain_filter: String,
    /// Start a new world rather than adding to the open one.
    pub new_world: bool,
    /// Whether a world is open (otherwise the import always starts one).
    pub world_open: bool,
}

impl HeightMapForm {
    pub fn new(world_open: bool) -> Self {
        HeightMapForm {
            path: String::new(),
            low: -64.0,
            high: 319.0,
            scale: 1.0,
            water_level: 62,
            terrain: Terrain::Grass,
            terrain_filter: String::new(),
            new_world: !world_open,
            world_open,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome<()> {
        let mut outcome = Outcome::Open;
        modal(ctx, "Import height map", 460.0, |ui| {
            ui.label(
                RichText::new("A greyscale PNG (8 or 16 bit): black becomes the low height, white the high one.")
                    .color(theme::MUTED),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text("height map .png")
                        .desired_width(ui.available_width() - 90.0),
                );
                if ui.button("Browse…").clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("PNG image", &["png"])
                        .pick_file()
                    {
                        self.path = file.display().to_string();
                    }
                }
            });
            egui::Grid::new("height_map_grid")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Black");
                    ui.add(
                        egui::Slider::new(&mut self.low, -64.0..=319.0)
                            .step_by(1.0)
                            .prefix("y = "),
                    );
                    ui.end_row();
                    ui.label("White");
                    ui.add(
                        egui::Slider::new(&mut self.high, -64.0..=319.0)
                            .step_by(1.0)
                            .prefix("y = "),
                    );
                    ui.end_row();
                    ui.label("Scale");
                    ui.add(
                        egui::DragValue::new(&mut self.scale)
                            .range(0.25..=16.0)
                            .speed(0.05)
                            .suffix(" blocks per pixel"),
                    );
                    ui.end_row();
                    ui.label("Sea level");
                    ui.add(egui::Slider::new(&mut self.water_level, -64..=319));
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui.small_button("Full height (−64…319)").clicked() {
                    (self.low, self.high) = (-64.0, 319.0);
                }
                if ui.small_button("8-bit levels (0…255)").clicked() {
                    (self.low, self.high) = (0.0, 255.0);
                }
            });
            if self.world_open {
                ui.checkbox(
                    &mut self.new_world,
                    "Start a new world (otherwise added to this one)",
                );
            }
            ui.label(RichText::new("Ground for new areas").color(theme::MUTED));
            terrain_picker(
                ui,
                "height_map_terrain",
                &mut self.terrain,
                &mut self.terrain_filter,
                120.0,
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = !self.path.trim().is_empty();
                if ui
                    .add_enabled_ui(ready, |ui| theme::primary_button(ui, "Import"))
                    .inner
                    .clicked()
                {
                    outcome = Outcome::Accept(());
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Outcome::Cancel;
        }
        outcome
    }
}

// --------------------------------------------------------------- export --

#[derive(Clone)]
pub struct ExportForm {
    pub path: String,
    pub outside: Outside,
    pub overwrite: bool,
    pub version: McVersion,
    pub cpu: CpuUse,
}

/// How hard an export works the processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuUse {
    Gentle,
    Balanced,
    Full,
}

impl CpuUse {
    pub fn threads(self) -> usize {
        match self {
            CpuUse::Gentle => 2,
            CpuUse::Balanced => groundwork_export::balanced_threads(),
            CpuUse::Full => groundwork_export::all_threads(),
        }
    }
}

impl ExportForm {
    pub fn new(world_name: &str) -> Self {
        let safe: String = world_name
            .chars()
            .map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
            .collect();
        let saves =
            std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(".minecraft").join("saves"));
        let base = match saves {
            Some(s) if s.is_dir() => s,
            _ => std::env::var_os("USERPROFILE")
                .map(|u| PathBuf::from(u).join("Documents"))
                .unwrap_or_default(),
        };
        ExportForm {
            path: base.join(safe).display().to_string(),
            outside: Outside::Vanilla,
            overwrite: false,
            version: McVersion::default(),
            cpu: CpuUse::Balanced,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome<()> {
        let mut outcome = Outcome::Open;
        modal(ctx, "Export to Minecraft", 460.0, |ui| {
            ui.label(
                RichText::new("Writes a playable Minecraft Java Edition save.").color(theme::MUTED),
            );
            ui.add_space(6.0);
            ui.label("Save folder");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.path)
                        .desired_width(ui.available_width() - 90.0),
                );
                if ui.button("Browse…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new()
                        .set_title("Choose an empty folder for the world")
                        .pick_folder()
                    {
                        self.path = dir.display().to_string();
                    }
                }
            });
            ui.add_space(4.0);
            ui.label("Minecraft version");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.version, McVersion::V1_21_11, "1.21.11");
                ui.selectable_value(&mut self.version, McVersion::V1_21_1, "1.21.1 – 1.21.10");
            });
            ui.label("Beyond the painted area");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.outside, Outside::Vanilla, "🌍 Minecraft terrain");
                ui.selectable_value(&mut self.outside, Outside::Ocean, "🌊 Endless ocean");
                ui.selectable_value(&mut self.outside, Outside::Void, "⬛ Void");
            });
            ui.label("Processor use");
            ui.horizontal(|ui| {
                for (cpu, label) in [
                    (CpuUse::Gentle, "Gentle"),
                    (CpuUse::Balanced, "Balanced"),
                    (CpuUse::Full, "Full"),
                ] {
                    ui.selectable_value(
                        &mut self.cpu,
                        cpu,
                        format!("{label} ({} threads)", cpu.threads()),
                    );
                }
            });
            if self.cpu == CpuUse::Full {
                ui.label(
                    RichText::new("Full speed keeps every core at 100% for minutes. On an unstable CPU that can crash the PC.")
                        .small()
                        .color(theme::DANGER),
                );
            }
            ui.checkbox(
                &mut self.overwrite,
                "Replace a world that's already in this folder",
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = !self.path.trim().is_empty();
                if ui
                    .add_enabled_ui(ready, |ui| theme::primary_button(ui, "Export"))
                    .inner
                    .clicked()
                {
                    outcome = Outcome::Accept(());
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Outcome::Cancel;
        }
        outcome
    }
}

// -------------------------------------------------------------- confirm --

pub fn confirm_discard(ctx: &egui::Context, action: PendingAction) -> Outcome<bool> {
    let mut outcome = Outcome::Open;
    let verb = match action {
        PendingAction::New => "start a new world",
        PendingAction::Open => "open another world",
        PendingAction::Quit => "quit",
    };
    modal(ctx, "Unsaved changes", 380.0, |ui| {
        ui.label(format!(
            "This world has changes that aren't saved. Save them before you {verb}?"
        ));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if theme::primary_button(ui, "Save").clicked() {
                outcome = Outcome::Accept(true);
            }
            if ui
                .add(egui::Button::new(
                    RichText::new("Don't save").color(theme::DANGER),
                ))
                .clicked()
            {
                outcome = Outcome::Accept(false);
            }
            if ui.button("Cancel").clicked() {
                outcome = Outcome::Cancel;
            }
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Outcome::Cancel;
    }
    outcome
}

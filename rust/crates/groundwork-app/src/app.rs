//! Application state and the per-frame UI.

use crate::canvas::{MapView, Overlay};
use crate::dialogs::{self, Dialog, ExportForm, NewWorldForm, Outcome, PendingAction};
use crate::theme;
use crate::tools::{self, Brush, BrushShape, PaintSettings, StrokeState, Tool};
use crate::widgets;
use eframe::egui::{self, vec2, Key, KeyboardShortcut, Modifiers, PointerButton, RichText, Sense};
use groundwork_core::generate::generate;
use groundwork_core::ops::{self, Falloff};
use groundwork_core::{biomes, layers, DimensionKind, GameMode, World};
use groundwork_export::{ExportOptions, ExportReport};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const OVERWORLD: DimensionKind = DimensionKind::Overworld;

/// An open world plus its file and history.
struct Document {
    world: World,
    path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<World>,
    redo: Vec<World>,
}

impl Document {
    fn new(world: World, path: Option<PathBuf>, dirty: bool) -> Self {
        Document {
            world,
            path,
            dirty,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Snapshots share unchanged tiles, so an undo step only costs the tiles
    /// that stroke touched.
    fn undo_limit(&self) -> usize {
        100
    }

    fn snapshot(&mut self) {
        if self.undo.len() >= self.undo_limit() {
            self.undo.remove(0);
        }
        self.undo.push(self.world.clone());
        self.redo.clear();
    }

    fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(previous) => {
                self.redo.push(std::mem::replace(&mut self.world, previous));
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.world, next));
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    fn title(&self) -> String {
        format!("{}{}", self.world.name, if self.dirty { " •" } else { "" })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ToastKind {
    Info,
    Success,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: Instant,
}

struct ExportJob {
    rx: mpsc::Receiver<Result<ExportReport, String>>,
    started: Instant,
    target: PathBuf,
}

pub struct App {
    doc: Option<Document>,
    tool: Tool,
    brush: Brush,
    paint: PaintSettings,
    view: MapView,
    stroke: Option<StrokeState>,
    hover: Option<(i32, i32)>,
    dialog: Option<Dialog>,
    export_job: Option<ExportJob>,
    toasts: Vec<Toast>,
    allow_close: bool,
    /// Last window title sent; resending it every frame makes the window
    /// system wake the app again, so it would never go idle.
    window_title: String,
}

impl App {
    /// `file` is a world to open at startup, e.g. from the command line.
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut app = App {
            doc: None,
            tool: Tool::Raise,
            brush: Brush::default(),
            paint: PaintSettings::default(),
            view: MapView::default(),
            stroke: None,
            hover: None,
            dialog: None,
            export_job: None,
            toasts: Vec::new(),
            allow_close: false,
            window_title: String::new(),
        };
        if let Some(path) = file {
            app.open_path(path);
        }
        app
    }

    fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            born: Instant::now(),
        });
    }

    fn dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty)
    }

    // ---------------------------------------------------------- actions --

    fn request(&mut self, action: PendingAction) {
        if self.dirty() {
            self.dialog = Some(Dialog::Confirm(action));
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::New => self.dialog = Some(Dialog::NewWorld(NewWorldForm::default())),
            PendingAction::Open => self.open(),
            PendingAction::Quit => self.allow_close = true,
        }
    }

    fn set_document(&mut self, doc: Document) {
        self.doc = Some(doc);
        self.view.reset();
        self.stroke = None;
    }

    fn create_world(&mut self, form: &NewWorldForm) {
        let options = form.options();
        match generate(&options) {
            Ok(dim) => {
                let mut world = World::new(form.name.trim(), options.seed, dim);
                world.spawn = (0, 0);
                self.paint.water_level = options.water_level;
                self.set_document(Document::new(world, None, true));
                self.dialog = None;
            }
            Err(e) => self.toast(ToastKind::Error, e),
        }
    }

    fn open(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Groundwork world", &["gwork"])
            .pick_file()
        else {
            return;
        };
        self.open_path(path);
    }

    fn open_path(&mut self, path: PathBuf) {
        match groundwork_core::io::load(&path) {
            Ok(world) => {
                self.toast(ToastKind::Info, format!("Opened {}", world.name));
                self.set_document(Document::new(world, Some(path), false));
            }
            Err(e) => self.toast(
                ToastKind::Error,
                format!("Couldn't open {}: {e}", path.display()),
            ),
        }
    }

    /// Returns true if the world was saved.
    fn save(&mut self, save_as: bool) -> bool {
        let Some(doc) = self.doc.as_mut() else {
            return false;
        };
        let path = match (&doc.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let picked = rfd::FileDialog::new()
                    .add_filter("Groundwork world", &["gwork"])
                    .set_file_name(format!("{}.gwork", doc.world.name))
                    .save_file();
                match picked {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        match groundwork_core::io::save(&doc.world, &path) {
            Ok(()) => {
                doc.path = Some(path.clone());
                doc.dirty = false;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.toast(ToastKind::Success, format!("Saved {name}"));
                true
            }
            Err(e) => {
                self.toast(ToastKind::Error, format!("Couldn't save: {e}"));
                false
            }
        }
    }

    fn import_height_map(&mut self, form: &dialogs::HeightMapForm) {
        use groundwork_core::heightmap::{import, HeightImage, ImportOptions};
        let path = PathBuf::from(form.path.trim());
        let image = match HeightImage::load(&path) {
            Ok(image) => image,
            Err(e) => return self.toast(ToastKind::Error, e),
        };
        let (w, h) = (
            image.width as f32 * form.scale,
            image.height as f32 * form.scale,
        );
        let tiles = ((w / 128.0).ceil() + 1.0) * ((h / 128.0).ceil() + 1.0);
        if tiles > 6400.0 {
            return self.toast(
                ToastKind::Error,
                format!("That would be {w:.0}×{h:.0} blocks; keep it within 10,240×10,240 (lower the scale)"),
            );
        }
        if form.new_world || self.doc.is_none() {
            let name = path
                .file_stem()
                .map_or("Imported".into(), |s| s.to_string_lossy().into_owned());
            let world = World::new(
                name,
                0,
                groundwork_core::Dimension::new(OVERWORLD, -64, 320, 0),
            );
            self.set_document(Document::new(world, None, true));
        }
        let doc = self.doc.as_mut().unwrap();
        doc.snapshot();
        let options = ImportOptions {
            x: -(w / 2.0) as i32,
            y: -(h / 2.0) as i32,
            scale: form.scale,
            low: form.low,
            high: form.high,
            terrain: form.terrain,
            water_level: form.water_level,
        };
        let changed = import(
            doc.world.dimension_mut(OVERWORLD).unwrap(),
            &image,
            &options,
        );
        doc.dirty = true;
        self.view.fit_pending = true;
        self.dialog = None;
        self.toast(
            ToastKind::Success,
            format!("Imported {changed} columns ({w:.0}×{h:.0} blocks)"),
        );
    }

    fn export_height_map(&mut self) {
        let Some(doc) = self.doc.as_ref() else { return };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{} heights.png", doc.world.name))
            .save_file()
        else {
            return;
        };
        let result =
            groundwork_core::heightmap::export_png(doc.world.dimension(OVERWORLD).unwrap())
                .and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string()));
        match result {
            Ok(()) => self.toast(
                ToastKind::Success,
                format!("Saved height map to {}", path.display()),
            ),
            Err(e) => self.toast(
                ToastKind::Error,
                format!("Couldn't export the height map: {e}"),
            ),
        }
    }

    fn start_export(&mut self, form: &ExportForm) {
        let Some(doc) = self.doc.as_ref() else { return };
        let world = doc.world.clone();
        let target = PathBuf::from(form.path.trim());
        let options = ExportOptions {
            outside: form.outside,
            overwrite: form.overwrite,
            version: form.version,
            threads: None,
        };
        let (tx, rx) = mpsc::channel();
        let dir = target.clone();
        std::thread::spawn(move || {
            let _ = tx
                .send(groundwork_export::export(&world, &dir, &options).map_err(|e| e.to_string()));
        });
        self.export_job = Some(ExportJob {
            rx,
            started: Instant::now(),
            target,
        });
        self.dialog = None;
    }

    // The map view notices which tiles undo and redo replaced.
    fn undo(&mut self) {
        if let Some(doc) = self.doc.as_mut() {
            doc.undo();
        }
    }

    fn redo(&mut self) {
        if let Some(doc) = self.doc.as_mut() {
            doc.redo();
        }
    }

    // --------------------------------------------------------- shortcuts --

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let pressed = |m: Modifiers, k: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if pressed(ctrl_shift, Key::Z) || pressed(ctrl, Key::Y) {
            self.redo();
        }
        if pressed(ctrl, Key::Z) {
            self.undo();
        }
        if pressed(ctrl_shift, Key::S) {
            self.save(true);
        }
        if pressed(ctrl, Key::S) {
            self.save(false);
        }
        if pressed(ctrl, Key::N) {
            self.request(PendingAction::New);
        }
        if pressed(ctrl, Key::O) {
            self.request(PendingAction::Open);
        }
        if pressed(ctrl, Key::E) {
            if let Some(doc) = &self.doc {
                self.dialog = Some(Dialog::Export(ExportForm::new(&doc.world.name)));
            }
        }
        if ctx.wants_keyboard_input() || self.dialog.is_some() {
            return;
        }
        ctx.input(|i| {
            let keys = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
            ];
            for (key, tool) in keys.iter().zip(Tool::ALL) {
                if i.key_pressed(*key) {
                    self.tool = tool;
                }
            }
            if i.key_pressed(Key::OpenBracket) {
                self.brush.radius = (self.brush.radius / 1.25).max(1.0);
            }
            if i.key_pressed(Key::CloseBracket) {
                self.brush.radius = (self.brush.radius * 1.25).min(256.0);
            }
            if i.key_pressed(Key::F) {
                self.view.fit_pending = true;
            }
            if i.key_pressed(Key::B) {
                use groundwork_core::render::View;
                self.view.view = if self.view.view == View::Biomes {
                    View::Terrain
                } else {
                    View::Biomes
                };
            }
        });
    }

    // ------------------------------------------------------------ panels --

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .exact_height(52.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(14.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("◆").size(20.0).color(theme::ACCENT));
                    ui.label(RichText::new("Groundwork").size(17.0).strong());
                    ui.add_space(16.0);
                    if ui
                        .button("+  New")
                        .on_hover_text("New world (Ctrl+N)")
                        .clicked()
                    {
                        self.request(PendingAction::New);
                    }
                    if ui
                        .button("📂 Open")
                        .on_hover_text("Open (Ctrl+O)")
                        .clicked()
                    {
                        self.request(PendingAction::Open);
                    }
                    let has_doc = self.doc.is_some();
                    if ui
                        .add_enabled(has_doc, egui::Button::new("💾 Save"))
                        .on_hover_text("Save (Ctrl+S)")
                        .clicked()
                    {
                        self.save(false);
                    }
                    if let Some(doc) = &self.doc {
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(12.0);
                        ui.label(RichText::new(doc.title()).color(theme::MUTED));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let exporting = self.export_job.is_some();
                        let can_export = has_doc && !exporting;
                        let export = ui
                            .add_enabled_ui(can_export, |ui| {
                                theme::primary_button(ui, "⛏  Export to Minecraft")
                            })
                            .inner
                            .on_hover_text("Ctrl+E");
                        if export.clicked() {
                            if let Some(doc) = &self.doc {
                                self.dialog =
                                    Some(Dialog::Export(ExportForm::new(&doc.world.name)));
                            }
                        }
                        ui.add_space(8.0);
                        let (can_redo, can_undo) = self
                            .doc
                            .as_ref()
                            .map_or((false, false), |d| (!d.redo.is_empty(), !d.undo.is_empty()));
                        if ui
                            .add_enabled(can_redo, egui::Button::new("↷"))
                            .on_hover_text("Redo (Ctrl+Shift+Z)")
                            .clicked()
                        {
                            self.redo();
                        }
                        if ui
                            .add_enabled(can_undo, egui::Button::new("↶"))
                            .on_hover_text("Undo (Ctrl+Z)")
                            .clicked()
                        {
                            self.undo();
                        }
                    });
                });
            });
    }

    fn tool_palette(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tools")
            .exact_width(68.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(10.0, 14.0)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for (i, tool) in Tool::ALL.into_iter().enumerate() {
                        if i == 4 || i == 6 {
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }
                        let selected = self.tool == tool;
                        let text = RichText::new(tool.icon()).size(20.0).color(if selected {
                            theme::ACCENT
                        } else {
                            theme::TEXT
                        });
                        let button = egui::Button::new(text)
                            .selected(selected)
                            .min_size(vec2(46.0, 46.0))
                            .rounding(10.0);
                        let tip = format!("{}  ({})\n{}", tool.name(), i + 1, tool.hint());
                        if ui.add(button).on_hover_text(tip).clicked() {
                            self.tool = tool;
                        }
                    }
                });
            });
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector")
            .exact_width(300.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("{}  {}", self.tool.icon(), self.tool.name()))
                                .heading()
                                .strong(),
                        );
                        ui.label(RichText::new(self.tool.hint()).color(theme::MUTED));
                        ui.add_space(6.0);

                        if self.tool.uses_brush() {
                            theme::card(ui, "Brush", |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized([44.0, 20.0], egui::Label::new("Size"));
                                    // Leave room for the value box so the row fits the card.
                                    ui.spacing_mut().slider_width =
                                        (ui.available_width() - 96.0).max(60.0);
                                    ui.add(
                                        egui::Slider::new(&mut self.brush.radius, 1.0..=256.0)
                                            .logarithmic(true)
                                            .suffix(" blocks"),
                                    );
                                });
                                if matches!(
                                    self.tool,
                                    Tool::Raise | Tool::Lower | Tool::Flatten | Tool::Smooth
                                ) {
                                    ui.horizontal(|ui| {
                                        ui.add_sized([44.0, 20.0], egui::Label::new("Power"));
                                        ui.spacing_mut().slider_width =
                                            (ui.available_width() - 96.0).max(60.0);
                                        ui.add(
                                            egui::Slider::new(&mut self.brush.strength, 0.05..=1.0)
                                                .custom_formatter(|v, _| {
                                                    format!("{:.0}%", v * 100.0)
                                                }),
                                        );
                                    });
                                }
                                ui.label(RichText::new("Edge").small().color(theme::MUTED));
                                widgets::segmented(
                                    ui,
                                    &mut self.brush.falloff,
                                    &[
                                        (Falloff::Constant, "Hard"),
                                        (Falloff::Smooth, "Soft"),
                                        (Falloff::Linear, "Linear"),
                                        (Falloff::Spike, "Spike"),
                                    ],
                                );
                                ui.label(RichText::new("Shape").small().color(theme::MUTED));
                                widgets::segmented(
                                    ui,
                                    &mut self.brush.shape,
                                    &[(BrushShape::Round, "● Round"), (BrushShape::Square, "■ Square"), (BrushShape::Organic, "☁ Organic")],
                                );
                                ui.label(
                                    RichText::new("[ and ] change the size")
                                        .small()
                                        .color(theme::MUTED),
                                );
                            });
                            ui.add_space(6.0);
                        }

                        match self.tool {
                            Tool::Terrain => {
                                theme::card(ui, "Terrain", |ui| {
                                    widgets::terrain_picker(
                                        ui,
                                        "inspector_terrain",
                                        &mut self.paint.terrain,
                                        &mut self.paint.terrain_filter,
                                        320.0,
                                    );
                                });
                            }
                            Tool::Layer => {
                                theme::card(ui, "Layer", |ui| {
                                    for (i, layer) in layers::ALL.iter().enumerate() {
                                        let label = match layer.id {
                                            "Frost" => "❄  Frost — snow and ice",
                                            "Void" => "⬛  Void — remove everything",
                                            "FloodWithLava" => "🔥  Lava instead of water",
                                            "Deciduous" => "🌳  Deciduous forest — oak, birch",
                                            "Pine" => "🌲  Pine forest — spruce",
                                            "Resources" => "💎  Ores — underground pockets",
                                            "Caverns" => "🕳  Caverns — underground caves",
                                            _ => "🌿  Biome",
                                        };
                                        let chosen = ui.selectable_value(&mut self.paint.layer, i, label).clicked();
                                        // Biomes are invisible in the terrain view, so show them.
                                        if chosen && layer.id == layers::BIOME.id {
                                            self.view.view = groundwork_core::render::View::Biomes;
                                        }
                                    }
                                });
                                let layer = self.paint.layer_def();
                                if layer.size == layers::DataSize::Nibble {
                                    let hint = match layer.id {
                                        "Resources" => "8 is WorldPainter's normal amount; 15 is nearly twice as rich",
                                        "Caverns" => "higher hollows out more rock; right-drag removes",
                                        _ => "15 is a dense forest; right-drag clears trees",
                                    };
                                    ui.add_space(6.0);
                                    theme::card(ui, "Strength", |ui| {
                                        ui.spacing_mut().slider_width = ui.available_width() - 60.0;
                                        ui.add(egui::Slider::new(&mut self.paint.layer_strength, 1..=15));
                                        ui.label(RichText::new(hint).small().color(theme::MUTED));
                                    });
                                }
                                if self.paint.layer_def().id == layers::BIOME.id {
                                    ui.add_space(6.0);
                                    theme::card(ui, "Biome", |ui| {
                                        widgets::biome_picker(
                                            ui,
                                            &mut self.paint.biome,
                                            &mut self.paint.biome_filter,
                                        );
                                        ui.label(
                                            RichText::new(
                                                "Paints solid areas. Right-drag sets them back to Automatic. B switches between biome and terrain view.",
                                            )
                                            .small()
                                            .color(theme::MUTED),
                                        );
                                    });
                                }
                            }
                            Tool::Water => {
                                theme::card(ui, "Flood", |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("Fill to");
                                        ui.add(
                                            egui::DragValue::new(&mut self.paint.water_level)
                                                .range(-64..=319)
                                                .prefix("y = "),
                                        );
                                        if let Some((x, y)) = self.hover {
                                            if let Some(h) = self.doc.as_ref().and_then(|d| {
                                                d.world.dimension(OVERWORLD)?.int_height_at(x, y)
                                            }) {
                                                if ui
                                                    .small_button("use cursor")
                                                    .on_hover_text(format!(
                                                        "Ground here is y = {h}"
                                                    ))
                                                    .clicked()
                                                {
                                                    self.paint.water_level = h;
                                                }
                                            }
                                        }
                                    });
                                    widgets::segmented(
                                        ui,
                                        &mut self.paint.lava,
                                        &[(false, "💧 Water"), (true, "🔥 Lava")],
                                    );
                                });
                            }
                            _ => {}
                        }

                        let (mut import_heights, mut export_heights) = (false, false);
                        if let Some(doc) = self.doc.as_mut() {
                            ui.add_space(6.0);
                            theme::card(ui, "World", |ui| {
                                let w = &mut doc.world;
                                let mut changed = false;
                                egui::Grid::new("world_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 8.0])
                                    .show(ui, |ui| {
                                        ui.label("Name");
                                        changed |= ui
                                            .add(
                                                egui::TextEdit::singleline(&mut w.name)
                                                    .desired_width(f32::INFINITY),
                                            )
                                            .changed();
                                        ui.end_row();
                                        ui.label("Mode");
                                        egui::ComboBox::from_id_salt("game_mode")
                                            .selected_text(format!("{:?}", w.game_mode))
                                            .width(160.0)
                                            .show_ui(ui, |ui| {
                                                for mode in [
                                                    GameMode::Survival,
                                                    GameMode::Creative,
                                                    GameMode::Adventure,
                                                    GameMode::Spectator,
                                                    GameMode::Hardcore,
                                                ] {
                                                    changed |= ui
                                                        .selectable_value(
                                                            &mut w.game_mode,
                                                            mode,
                                                            format!("{mode:?}"),
                                                        )
                                                        .changed();
                                                }
                                            });
                                        ui.end_row();
                                        ui.label("Spawn");
                                        ui.label(format!("{}, {}", w.spawn.0, w.spawn.1));
                                        ui.end_row();
                                    });
                                changed |=
                                    ui.checkbox(&mut w.allow_cheats, "Allow cheats").changed();
                                changed |= ui
                                    .checkbox(&mut w.map_features, "Structures outside the map")
                                    .changed();
                            if let Some(d) = w.dimension_mut(OVERWORLD) {
                                ui.add_space(4.0);
                                ui.label(RichText::new("Everywhere underground").small().color(theme::MUTED));
                                ui.horizontal(|ui| {
                                    ui.add_sized([60.0, 20.0], egui::Label::new("Ores"));
                                    ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(60.0);
                                    changed |= ui.add(egui::Slider::new(&mut d.resources_level, 0..=15)).changed();
                                });
                                ui.horizontal(|ui| {
                                    ui.add_sized([60.0, 20.0], egui::Label::new("Caverns"));
                                    ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(60.0);
                                    changed |= ui.add(egui::Slider::new(&mut d.caverns_level, 0..=15)).changed();
                                });
                            }
                                if let Some(d) = w.dimension(OVERWORLD) {
                                    if let Some(e) = d.extent() {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} tiles · {}×{} blocks · seed {}",
                                                d.tile_count(),
                                                e.width(),
                                                e.height(),
                                                w.seed as i64
                                            ))
                                            .small()
                                            .color(theme::MUTED),
                                        );
                                    }
                                }
                                if changed {
                                    doc.dirty = true;
                                }
                                ui.horizontal(|ui| {
                                    import_heights = ui.button("⛰ Import height map…").clicked();
                                    export_heights = ui.button("Export…").on_hover_text("Save heights as a 16-bit PNG").clicked();
                                });
                            });
                        }
                        if import_heights {
                            self.dialog = Some(Dialog::ImportHeightMap(dialogs::HeightMapForm::new(true)));
                        }
                        if export_heights {
                            self.export_height_map();
                        }
                    });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(30.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(14.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let muted = |s: String| RichText::new(s).small().color(theme::MUTED);
                    let dim = self.doc.as_ref().and_then(|d| d.world.dimension(OVERWORLD));
                    match (self.hover, dim) {
                        (Some((x, y)), Some(dim)) if dim.contains(x, y) => {
                            let h = dim.height_at(x, y).unwrap();
                            let water = dim.water_level_at(x, y).unwrap();
                            let terrain = dim.terrain_at(x, y).unwrap();
                            let biome = dim.layer_value_at(layers::BIOME, x, y).unwrap();
                            ui.label(muted(format!("📍 {x}, {y}")));
                            ui.label(muted(format!("⛰ height {:.0}", h)));
                            if water > h.round() as i32 {
                                ui.label(muted(format!(
                                    "💧 water {water} (depth {})",
                                    water - h.round() as i32
                                )));
                            }
                            ui.label(muted(format!("🖌 {terrain}")));
                            if let Some(id) = groundwork_core::render::column_biome(dim, x, y) {
                                let auto = if biome == biomes::BIOME_AUTO {
                                    " (auto)"
                                } else {
                                    ""
                                };
                                ui.label(muted(format!(
                                    "🌿 {}{auto}",
                                    biomes::display_name(id).unwrap_or("?")
                                )));
                            }
                        }
                        (Some((x, y)), _) => {
                            ui.label(muted(format!("📍 {x}, {y} · outside the map")));
                        }
                        _ => {}
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(muted(format!("{:.0}%", self.view.zoom * 100.0)));
                        if let Some(job) = &self.export_job {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label(muted(format!(
                                "Exporting… {:.0}s",
                                job.started.elapsed().as_secs_f32()
                            )));
                        }
                    });
                });
            });
    }

    // ------------------------------------------------------------ canvas --

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let ctx = ui.ctx().clone();

        let Some(doc) = self.doc.as_mut() else {
            painter.rect_filled(rect, 0.0, theme::BG);
            self.welcome(ui, rect);
            return;
        };

        if self.view.fit_pending {
            if let Some(extent) = doc.world.dimension(OVERWORLD).and_then(|d| d.extent()) {
                self.view.fit(extent, rect);
            }
            self.view.fit_pending = false;
        }

        // Navigation: wheel/pinch zooms, middle-drag or Space+drag pans.
        let (scroll, pinch, space, dt, primary, secondary) = ctx.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.zoom_delta(),
                i.key_down(Key::Space),
                i.stable_dt.min(0.1),
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
            )
        });
        if let Some(pointer) = response.hover_pos() {
            let factor = (scroll * 0.0018).exp() * pinch;
            if factor != 1.0 {
                self.view.zoom_at(rect, pointer, factor);
            }
        }
        let panning = response.dragged_by(PointerButton::Middle)
            || (space && response.dragged_by(PointerButton::Primary));
        if panning {
            self.view.cam -= response.drag_delta() / self.view.zoom;
        }
        if !ctx.wants_keyboard_input() {
            let pan = ctx.input(|i| {
                let mut d = egui::Vec2::ZERO;
                for (key, dir) in [
                    (Key::ArrowLeft, vec2(-1.0, 0.0)),
                    (Key::ArrowRight, vec2(1.0, 0.0)),
                    (Key::ArrowUp, vec2(0.0, -1.0)),
                    (Key::ArrowDown, vec2(0.0, 1.0)),
                ] {
                    if i.key_down(key) {
                        d += dir;
                    }
                }
                d
            });
            if pan != egui::Vec2::ZERO {
                self.view.cam += pan * 600.0 * dt / self.view.zoom;
                ctx.request_repaint();
            }
        }

        self.hover = response.hover_pos().map(|p| {
            let w = self.view.screen_to_world(rect, p);
            (w.x.floor() as i32, w.y.floor() as i32)
        });

        // Editing.
        let editing = response.is_pointer_button_down_on()
            && (primary || secondary)
            && !space
            && !panning
            && self.dialog.is_none();
        match (editing, self.hover) {
            (true, Some((x, y))) => {
                let first = self.stroke.is_none();
                let eyedropper = self.tool == Tool::Terrain && secondary;
                if eyedropper {
                    if let Some(t) = doc
                        .world
                        .dimension(OVERWORLD)
                        .and_then(|d| d.terrain_at(x, y))
                    {
                        self.paint.terrain = t;
                    }
                } else {
                    if first {
                        doc.snapshot();
                    }
                    let stroke = self.stroke.get_or_insert_with(StrokeState::default);
                    let dim = doc.world.dimension_mut(OVERWORLD).unwrap();
                    match self.tool {
                        Tool::Spawn if first => {
                            if dim.contains(x, y) {
                                doc.world.spawn = (x, y);
                                doc.dirty = true;
                            } else {
                                doc.undo.pop();
                            }
                        }
                        Tool::Water if first => {
                            let level = if secondary {
                                dim.min_height
                            } else {
                                self.paint.water_level
                            };
                            match ops::flood(dim, x, y, level, self.paint.lava, 4_000_000) {
                                Ok(0) => {
                                    doc.undo.pop();
                                    self.toasts.push(Toast {
                                        text: format!("Nothing to fill: the ground here is at or above y = {level}, or it's already flooded."),
                                        kind: ToastKind::Info,
                                        born: Instant::now(),
                                    });
                                }
                                Ok(_) => doc.dirty = true,
                                Err(e) => {
                                    doc.undo.pop();
                                    self.toasts.push(Toast {
                                        text: e,
                                        kind: ToastKind::Error,
                                        born: Instant::now(),
                                    });
                                }
                            }
                        }
                        Tool::Water | Tool::Spawn => {}
                        tool => {
                            if let Some((x0, y0, x1, y1)) = tools::apply(
                                tool,
                                dim,
                                x,
                                y,
                                dt,
                                secondary,
                                &self.brush,
                                &self.paint,
                                stroke,
                            ) {
                                self.view.invalidate_area(x0, y0, x1, y1);
                                doc.dirty = true;
                            }
                        }
                    }
                    self.stroke.get_or_insert_with(StrokeState::default);
                    ctx.request_repaint();
                }
            }
            _ => self.stroke = None,
        }

        let dim = doc.world.dimension(OVERWORLD).unwrap();
        let brush = (self.tool.uses_brush() && !space)
            .then_some(())
            .and(self.hover)
            .map(|(x, y)| {
                (
                    x,
                    y,
                    self.brush.radius,
                    self.brush.shape == BrushShape::Square,
                )
            });
        let overlay = Overlay {
            spawn: doc.world.spawn,
            brush,
        };
        self.view.draw(&ctx, &painter, rect, dim, &overlay);
        if response.hovered() {
            ctx.set_cursor_icon(if panning || space {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            });
        }

        // Floating zoom controls.
        egui::Area::new(egui::Id::new("zoom_hud"))
            .fixed_pos(rect.left_top() + vec2(14.0, 14.0))
            .order(egui::Order::Middle)
            .show(&ctx, |ui| {
                egui::Frame::none()
                    .fill(theme::SURFACE.gamma_multiply(0.92))
                    .rounding(10.0)
                    .stroke(egui::Stroke::new(1.0f32, theme::BORDER))
                    .inner_margin(egui::Margin::same(4.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if ui.button("−").on_hover_text("Zoom out").clicked() {
                                self.view.zoom_at(rect, rect.center(), 1.0 / 1.5);
                            }
                            if ui.button("+").on_hover_text("Zoom in").clicked() {
                                self.view.zoom_at(rect, rect.center(), 1.5);
                            }
                            if ui.button("⛶").on_hover_text("Fit world (F)").clicked() {
                                self.view.fit_pending = true;
                            }
                            ui.separator();
                            use groundwork_core::render::View;
                            ui.selectable_value(&mut self.view.view, View::Terrain, "🗺 Terrain")
                                .on_hover_text("Terrain colours (B toggles)");
                            ui.selectable_value(&mut self.view.view, View::Biomes, "🌿 Biomes")
                                .on_hover_text("Biome colours: painted and automatic (B toggles)");
                        });
                    });
            });
    }

    fn welcome(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let card = egui::Rect::from_center_size(rect.center(), vec2(420.0, 280.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(card), |ui| {
            egui::Frame::none()
                .fill(theme::SURFACE)
                .rounding(18.0)
                .stroke(egui::Stroke::new(1.0f32, theme::BORDER))
                .inner_margin(egui::Margin::same(28.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("◆").size(40.0).color(theme::ACCENT));
                        ui.label(RichText::new("Groundwork").size(28.0).strong());
                        ui.label(RichText::new("Paint Minecraft worlds.").color(theme::MUTED));
                        ui.add_space(18.0);
                        if theme::primary_button(ui, "  +  New world  ").clicked() {
                            self.dialog = Some(Dialog::NewWorld(NewWorldForm::default()));
                        }
                        ui.add_space(4.0);
                        if ui.button("📂  Open a .gwork file").clicked() {
                            self.open();
                        }
                        if ui.button("⛰  Start from a height map").clicked() {
                            self.dialog =
                                Some(Dialog::ImportHeightMap(dialogs::HeightMapForm::new(false)));
                        }
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new("An unofficial Rust port of WorldPainter")
                                .small()
                                .color(theme::MUTED),
                        );
                    });
                });
        });
    }

    // ---------------------------------------------------- dialogs/toasts --

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        match dialog {
            Dialog::NewWorld(form) => match form.show(ctx) {
                Outcome::Open => {}
                Outcome::Cancel => self.dialog = None,
                Outcome::Accept(()) => {
                    let form = form.clone();
                    self.create_world(&form);
                }
            },
            Dialog::ImportHeightMap(form) => match form.show(ctx) {
                Outcome::Open => {}
                Outcome::Cancel => self.dialog = None,
                Outcome::Accept(()) => {
                    let form = form.clone();
                    if form.new_world && self.dirty() {
                        self.toast(
                            ToastKind::Error,
                            "Save or close the open world before starting a new one",
                        );
                    } else {
                        self.import_height_map(&form);
                    }
                }
            },
            Dialog::Export(form) => match form.show(ctx) {
                Outcome::Open => {}
                Outcome::Cancel => self.dialog = None,
                Outcome::Accept(()) => {
                    let form = form.clone();
                    self.start_export(&form);
                }
            },
            Dialog::Confirm(action) => {
                let action = *action;
                match dialogs::confirm_discard(ctx, action) {
                    Outcome::Open => {}
                    Outcome::Cancel => self.dialog = None,
                    Outcome::Accept(save_first) => {
                        self.dialog = None;
                        if !save_first || self.save(false) {
                            if let Some(doc) = self.doc.as_mut() {
                                doc.dirty = false;
                            }
                            self.perform(action);
                            if action == PendingAction::Quit {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                    }
                }
            }
        }
    }

    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.export_job else { return };
        match job.rx.try_recv() {
            Ok(Ok(report)) => {
                let warnings = if report.warnings.is_empty() {
                    String::new()
                } else {
                    format!(
                        " · {} note(s), e.g. {}",
                        report.warnings.len(),
                        report.warnings[0]
                    )
                };
                let text = format!(
                    "Exported {} chunks to {} in {:.1}s{warnings}",
                    report.chunks,
                    job.target.display(),
                    job.started.elapsed().as_secs_f32()
                );
                self.export_job = None;
                self.toast(ToastKind::Success, text);
            }
            Ok(Err(e)) => {
                self.export_job = None;
                self.toast(ToastKind::Error, format!("Export failed: {e}"));
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.export_job = None;
                self.toast(ToastKind::Error, "Export stopped unexpectedly");
            }
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        const LIFETIME: Duration = Duration::from_secs(6);
        self.toasts.retain(|t| t.born.elapsed() < LIFETIME);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -44.0])
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for toast in &self.toasts {
                    let (icon, colour) = match toast.kind {
                        ToastKind::Info => ("ℹ", theme::MUTED),
                        ToastKind::Success => ("✔", theme::ACCENT),
                        ToastKind::Error => ("⚠", theme::DANGER),
                    };
                    egui::Frame::none()
                        .fill(theme::SURFACE)
                        .rounding(10.0)
                        .stroke(egui::Stroke::new(1.0f32, colour.gamma_multiply(0.6)))
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .shadow(ctx.style().visuals.popup_shadow)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(icon).color(colour));
                                ui.label(&toast.text);
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Confirm(PendingAction::Quit));
        }
        let title = match &self.doc {
            Some(doc) => format!("{} — Groundwork", doc.title()),
            None => "Groundwork".to_owned(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        self.shortcuts(ctx);
        self.poll_export(ctx);
        self.top_bar(ctx);
        self.status_bar(ctx);
        self.tool_palette(ctx);
        self.inspector(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG))
            .show(ctx, |ui| self.canvas(ui));
        self.dialogs(ctx);
        self.toasts(ctx);
    }
}

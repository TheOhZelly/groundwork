//! The map view: camera, tile textures and overlays.

use crate::theme;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use groundwork_core::{render, Dimension, Extent, TILE_SIZE};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Tiles re-rendered per frame at most; the rest follow on later frames.
const RENDER_BUDGET: usize = 48;
/// Textures not drawn for this many frames are freed.
const EVICT_AFTER_FRAMES: u64 = 300;

/// A tile's texture and what it was rendered from.
struct TileTexture {
    handle: TextureHandle,
    /// Blocks per texel: 1 at full detail, up to 16 when zoomed far out.
    step: i32,
    /// Address of the tile data it shows. Tiles are copy-on-write, so a
    /// different address means the tile changed (undo, redo, flood, …).
    source: usize,
    last_drawn: u64,
}
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 32.0;

pub struct MapView {
    /// World coordinates at the centre of the view.
    pub cam: Vec2,
    /// Screen pixels per block.
    pub zoom: f32,
    pub fit_pending: bool,
    textures: HashMap<(i32, i32), TileTexture>,
    stale: HashSet<(i32, i32)>,
    buf: Vec<u8>,
    frame: u64,
}

impl Default for MapView {
    fn default() -> Self {
        MapView {
            cam: Vec2::ZERO,
            zoom: 1.0,
            fit_pending: true,
            textures: HashMap::new(),
            stale: HashSet::new(),
            buf: Vec::new(),
            frame: 0,
        }
    }
}

pub struct Overlay {
    pub spawn: (i32, i32),
    /// Brush outline: centre in world blocks, radius in blocks, square?
    pub brush: Option<(i32, i32, f32, bool)>,
}

impl MapView {
    /// Forgets all textures and refits the view, for a newly opened world.
    /// (Undo, redo and other whole-tile changes need no call: changed tiles
    /// are detected automatically.)
    pub fn reset(&mut self) {
        self.textures.clear();
        self.stale.clear();
        self.fit_pending = true;
    }

    /// Blocks per texel for the current zoom: the coarsest power of two that
    /// still gives at least one texel per screen pixel.
    fn lod_step(&self) -> i32 {
        let mut step = 1;
        while step < 16 && self.zoom * (step * 2) as f32 <= 1.0 {
            step *= 2;
        }
        step
    }

    pub fn invalidate_area(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        for ty in (y0 >> 7)..=(y1 >> 7) {
            for tx in (x0 >> 7)..=(x1 >> 7) {
                if self.textures.contains_key(&(tx, ty)) {
                    self.stale.insert((tx, ty));
                }
            }
        }
    }

    pub fn fit(&mut self, extent: Extent, rect: Rect) {
        let (w, h) = (extent.width() as f32, extent.height() as f32);
        self.zoom = ((rect.width() / w).min(rect.height() / h) * 0.9).clamp(MIN_ZOOM, MAX_ZOOM);
        self.cam = vec2(extent.x0 as f32 + w / 2.0, extent.y0 as f32 + h / 2.0);
    }

    pub fn world_to_screen(&self, rect: Rect, x: f32, y: f32) -> Pos2 {
        rect.center() + (vec2(x, y) - self.cam) * self.zoom
    }

    pub fn screen_to_world(&self, rect: Rect, p: Pos2) -> Vec2 {
        self.cam + (p - rect.center()) / self.zoom
    }

    /// Zooms by `factor`, keeping the world point under `anchor` in place.
    pub fn zoom_at(&mut self, rect: Rect, anchor: Pos2, factor: f32) {
        let before = self.screen_to_world(rect, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.screen_to_world(rect, anchor);
        self.cam += before - after;
    }

    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        painter: &Painter,
        rect: Rect,
        dim: &Dimension,
        overlay: &Overlay,
    ) {
        painter.rect_filled(rect, 0.0, theme::BG);
        self.draw_dots(painter, rect);

        let tl = self.screen_to_world(rect, rect.min);
        let br = self.screen_to_world(rect, rect.max);
        let ts = TILE_SIZE as f32;
        let (tx0, ty0) = ((tl.x / ts).floor() as i32, (tl.y / ts).floor() as i32);
        let (tx1, ty1) = ((br.x / ts).floor() as i32, (br.y / ts).floor() as i32);

        let options = TextureOptions {
            minification: egui::TextureFilter::Linear,
            ..TextureOptions::NEAREST
        };
        self.frame += 1;
        let step = self.lod_step();
        let mut budget = RENDER_BUDGET;
        let mut pending = false;
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let Some(tile) = dim.tile_handle(tx, ty) else {
                    continue;
                };
                let key = (tx, ty);
                let source = Arc::as_ptr(tile) as usize;
                if let Some(texture) = self.textures.get(&key) {
                    if texture.source != source {
                        // The tile was replaced; its east and south
                        // neighbours shade their edges from it too.
                        self.stale.extend([key, (tx + 1, ty), (tx, ty + 1)]);
                    }
                }
                let needs_render = match self.textures.get(&key) {
                    None => true,
                    Some(texture) => texture.step != step || self.stale.contains(&key),
                };
                if needs_render {
                    if budget == 0 {
                        pending = true;
                    } else {
                        budget -= 1;
                        let side = (TILE_SIZE / step) as usize;
                        self.buf.resize(side * side * 4, 0);
                        render::render_tile_lod(dim, tx, ty, step, &mut self.buf);
                        let image = egui::ColorImage::from_rgba_unmultiplied([side; 2], &self.buf);
                        match self.textures.get_mut(&key) {
                            Some(texture) => {
                                texture.handle.set(image, options);
                                texture.step = step;
                                texture.source = source;
                            }
                            None => {
                                let handle =
                                    ctx.load_texture(format!("tile {tx},{ty}"), image, options);
                                self.textures.insert(
                                    key,
                                    TileTexture {
                                        handle,
                                        step,
                                        source,
                                        last_drawn: 0,
                                    },
                                );
                            }
                        }
                        self.stale.remove(&key);
                    }
                }
                let min = self
                    .world_to_screen(rect, tx as f32 * ts, ty as f32 * ts)
                    .round();
                let max = self
                    .world_to_screen(rect, (tx + 1) as f32 * ts, (ty + 1) as f32 * ts)
                    .round();
                if let Some(texture) = self.textures.get_mut(&key) {
                    texture.last_drawn = self.frame;
                    painter.image(
                        texture.handle.id(),
                        Rect::from_min_max(min, max),
                        uv,
                        Color32::WHITE,
                    );
                }
            }
        }
        // Free textures that have been off screen for a while.
        if self.frame.is_multiple_of(60) {
            let frame = self.frame;
            self.textures
                .retain(|_, t| frame - t.last_drawn < EVICT_AFTER_FRAMES);
            let textures = &self.textures;
            self.stale.retain(|k| textures.contains_key(k));
        }
        if pending {
            ctx.request_repaint();
        }

        // Tile grid when zoomed in far enough to be useful.
        if self.zoom >= 1.5 {
            let stroke = Stroke::new(1.0f32, Color32::from_white_alpha(5));
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    if dim.tile(tx, ty).is_some() {
                        let min = self
                            .world_to_screen(rect, tx as f32 * ts, ty as f32 * ts)
                            .round();
                        let max = self
                            .world_to_screen(rect, (tx + 1) as f32 * ts, (ty + 1) as f32 * ts)
                            .round();
                        painter.rect_stroke(Rect::from_min_max(min, max), 0.0, stroke);
                    }
                }
            }
        }

        self.draw_spawn(painter, rect, overlay.spawn);
        if let Some((x, y, radius, square)) = overlay.brush {
            self.draw_brush(painter, rect, x, y, radius, square);
        }
    }

    fn draw_dots(&self, painter: &Painter, rect: Rect) {
        let spacing = 28.0;
        let colour = Color32::from_white_alpha(10);
        let mut y = rect.top() + spacing / 2.0;
        while y < rect.bottom() {
            let mut x = rect.left() + spacing / 2.0;
            while x < rect.right() {
                painter.circle_filled(pos2(x, y), 1.0, colour);
                x += spacing;
            }
            y += spacing;
        }
    }

    fn draw_spawn(&self, painter: &Painter, rect: Rect, (x, y): (i32, i32)) {
        let p = self.world_to_screen(rect, x as f32 + 0.5, y as f32 + 0.5);
        if !rect.contains(p) {
            return;
        }
        painter.circle_filled(p, 11.0, Color32::from_black_alpha(150));
        painter.circle_stroke(p, 11.0, Stroke::new(2.0f32, theme::ACCENT));
        painter.text(
            p,
            Align2::CENTER_CENTER,
            "⚑",
            FontId::proportional(13.0),
            theme::ACCENT,
        );
    }

    fn draw_brush(&self, painter: &Painter, rect: Rect, x: i32, y: i32, radius: f32, square: bool) {
        let centre = self.world_to_screen(rect, x as f32 + 0.5, y as f32 + 0.5);
        let r = ((radius + 0.5) * self.zoom).max(3.0);
        for (width, colour) in [
            (3.5f32, Color32::from_black_alpha(160)),
            (1.5f32, Color32::WHITE),
        ] {
            let stroke = Stroke::new(width, colour);
            if square {
                painter.rect_stroke(
                    Rect::from_center_size(centre, Vec2::splat(r * 2.0)),
                    2.0,
                    stroke,
                );
            } else {
                painter.circle_stroke(centre, r, stroke);
            }
        }
        painter.circle_filled(centre, 1.5, Color32::WHITE);
    }
}

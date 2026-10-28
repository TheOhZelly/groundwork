//! The map view: camera, tile textures and overlays.

use crate::theme;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use groundwork_core::{render, Dimension, Extent, TILE_SIZE};
use std::collections::{HashMap, HashSet};

/// Tiles re-rendered per frame at most; the rest follow on later frames.
const RENDER_BUDGET: usize = 48;
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 32.0;

pub struct MapView {
    /// World coordinates at the centre of the view.
    pub cam: Vec2,
    /// Screen pixels per block.
    pub zoom: f32,
    pub fit_pending: bool,
    textures: HashMap<(i32, i32), TextureHandle>,
    stale: HashSet<(i32, i32)>,
    buf: Vec<u8>,
}

impl Default for MapView {
    fn default() -> Self {
        MapView {
            cam: Vec2::ZERO,
            zoom: 1.0,
            fit_pending: true,
            textures: HashMap::new(),
            stale: HashSet::new(),
            buf: vec![0; (TILE_SIZE * TILE_SIZE * 4) as usize],
        }
    }
}

pub struct Overlay {
    pub spawn: (i32, i32),
    /// Brush outline: centre in world blocks, radius in blocks, square?
    pub brush: Option<(i32, i32, f32, bool)>,
}

impl MapView {
    /// Forget all textures, e.g. after loading a different world or undo.
    pub fn reset(&mut self, refit: bool) {
        self.stale.extend(self.textures.keys().copied());
        if refit {
            self.textures.clear();
            self.stale.clear();
            self.fit_pending = true;
        }
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
        let mut budget = RENDER_BUDGET;
        let mut pending = false;
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if dim.tile(tx, ty).is_none() {
                    continue;
                }
                let key = (tx, ty);
                let needs_render = !self.textures.contains_key(&key) || self.stale.contains(&key);
                if needs_render {
                    if budget == 0 {
                        pending = true;
                    } else {
                        budget -= 1;
                        render::render_tile(dim, tx, ty, &mut self.buf);
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [TILE_SIZE as usize; 2],
                            &self.buf,
                        );
                        match self.textures.get_mut(&key) {
                            Some(handle) => handle.set(image, options),
                            None => {
                                let handle =
                                    ctx.load_texture(format!("tile {tx},{ty}"), image, options);
                                self.textures.insert(key, handle);
                            }
                        }
                        self.stale.remove(&key);
                    }
                }
                if let Some(handle) = self.textures.get(&key) {
                    let min = self
                        .world_to_screen(rect, tx as f32 * ts, ty as f32 * ts)
                        .round();
                    let max = self
                        .world_to_screen(rect, (tx + 1) as f32 * ts, (ty + 1) as f32 * ts)
                        .round();
                    painter.image(
                        handle.id(),
                        Rect::from_min_max(min, max),
                        uv,
                        Color32::WHITE,
                    );
                }
            }
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

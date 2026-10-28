//! Editing tools and how a brush stroke applies them.

use groundwork_core::layers::DataSize;
use groundwork_core::ops::{self, Falloff, Shape};
use groundwork_core::{biomes, layers, Dimension, Terrain};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Raise,
    Lower,
    Flatten,
    Smooth,
    Terrain,
    Layer,
    Water,
    Spawn,
}

impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::Raise,
        Tool::Lower,
        Tool::Flatten,
        Tool::Smooth,
        Tool::Terrain,
        Tool::Layer,
        Tool::Water,
        Tool::Spawn,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Raise => "⬆",
            Tool::Lower => "⬇",
            Tool::Flatten => "⬌",
            Tool::Smooth => "〰",
            Tool::Terrain => "🖌",
            Tool::Layer => "❄",
            Tool::Water => "💧",
            Tool::Spawn => "⚑",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Raise => "Raise",
            Tool::Lower => "Lower",
            Tool::Flatten => "Flatten",
            Tool::Smooth => "Smooth",
            Tool::Terrain => "Paint terrain",
            Tool::Layer => "Paint layer",
            Tool::Water => "Flood",
            Tool::Spawn => "Spawn point",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Raise => "Drag to raise terrain. Right-drag lowers.",
            Tool::Lower => "Drag to lower terrain. Right-drag raises.",
            Tool::Flatten => "Drag to level terrain to the height where the stroke started.",
            Tool::Smooth => "Drag to soften bumps and sharp edges.",
            Tool::Terrain => "Drag to paint the selected terrain. Right-click picks the terrain under the cursor.",
            Tool::Layer => "Drag to paint the selected layer. Right-drag erases it.",
            Tool::Water => "Click inside a basin to fill it to the water level. Right-click drains connected water.",
            Tool::Spawn => "Click to set where players spawn.",
        }
    }

    pub fn uses_brush(self) -> bool {
        !matches!(self, Tool::Water | Tool::Spawn)
    }
}

#[derive(Debug, Clone)]
pub struct Brush {
    pub radius: f32,
    pub strength: f32,
    pub falloff: Falloff,
    pub square: bool,
}

impl Default for Brush {
    fn default() -> Self {
        Brush {
            radius: 24.0,
            strength: 0.5,
            falloff: Falloff::Smooth,
            square: false,
        }
    }
}

impl Brush {
    pub fn shape(&self, x: i32, y: i32) -> Shape {
        if self.square {
            Shape::Square {
                x,
                y,
                radius: self.radius.round() as i32,
            }
        } else {
            Shape::Circle {
                x,
                y,
                radius: self.radius,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaintSettings {
    pub terrain: Terrain,
    pub terrain_filter: String,
    /// Index into `layers::ALL`.
    pub layer: usize,
    pub biome: u8,
    pub biome_filter: String,
    pub water_level: i32,
    pub lava: bool,
    /// Strength for 0–15 layers (forests, ores, caverns).
    pub layer_strength: u8,
}

impl Default for PaintSettings {
    fn default() -> Self {
        PaintSettings {
            terrain: Terrain::Sand,
            terrain_filter: String::new(),
            layer: 0,
            biome: 1,
            biome_filter: String::new(),
            water_level: 62,
            lava: false,
            layer_strength: 8,
        }
    }
}

impl PaintSettings {
    pub fn layer_def(&self) -> &'static layers::LayerDef {
        layers::ALL[self.layer]
    }
}

/// Per-stroke state, reset when the mouse button is released.
#[derive(Debug, Default)]
pub struct StrokeState {
    pub flatten_target: Option<f32>,
    pub smooth_budget: f32,
}

/// Applies one frame of a brush tool. `inverse` is the right mouse button.
/// Returns the block area that changed, if any.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    tool: Tool,
    dim: &mut Dimension,
    x: i32,
    y: i32,
    dt: f32,
    inverse: bool,
    brush: &Brush,
    paint: &PaintSettings,
    stroke: &mut StrokeState,
) -> Option<(i32, i32, i32, i32)> {
    let shape = brush.shape(x, y);
    let r = brush.radius.ceil() as i32 + 2;
    let area = Some((x - r, y - r, x + r, y + r));
    match tool {
        Tool::Raise | Tool::Lower => {
            let sign = if (tool == Tool::Lower) != inverse {
                -1.0
            } else {
                1.0
            };
            // Up to 60 blocks per second at full strength.
            let amount = sign * brush.strength.powi(2) * 60.0 * dt;
            (ops::raise_lower(dim, shape, brush.falloff, amount) > 0).then_some(())?;
            area
        }
        Tool::Flatten => {
            let target = match stroke.flatten_target {
                Some(t) => t,
                None => {
                    let t = dim.height_at(x, y)?;
                    stroke.flatten_target = Some(t);
                    t
                }
            };
            let rate = (brush.strength * 8.0 * dt).min(1.0);
            shape.for_each(brush.falloff, |cx, cy, s| {
                if let Some(h) = dim.height_at(cx, cy) {
                    dim.set_height_at(cx, cy, h + (target - h) * (s * rate).min(1.0));
                }
            });
            area
        }
        Tool::Smooth => {
            stroke.smooth_budget += brush.strength * 30.0 * dt;
            let mut changed = false;
            while stroke.smooth_budget >= 1.0 {
                stroke.smooth_budget -= 1.0;
                changed |= ops::smooth(dim, shape, brush.falloff, 1) > 0;
            }
            changed.then_some(())?;
            area
        }
        Tool::Terrain => {
            (ops::paint_terrain(dim, shape, brush.falloff, paint.terrain) > 0).then_some(())?;
            area
        }
        Tool::Layer => {
            let layer = paint.layer_def();
            let value = match (inverse, layer.id == layers::BIOME.id) {
                (true, true) => biomes::BIOME_AUTO,
                (true, false) => 0,
                (false, true) => paint.biome,
                (false, false) if layer.size == DataSize::Nibble => paint.layer_strength,
                (false, false) => layer.size.max_value(),
            };
            // Biomes paint as solid areas: dithered edges would scatter odd
            // biome cells along every border.
            let falloff = if layer.id == layers::BIOME.id {
                ops::Falloff::Constant
            } else {
                brush.falloff
            };
            (ops::paint_layer(dim, shape, falloff, layer, value) > 0).then_some(())?;
            area
        }
        Tool::Water | Tool::Spawn => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use groundwork_core::generate::{generate, GenerateOptions};
    use groundwork_core::render::column_biome;

    #[test]
    fn painting_a_biome_changes_it() {
        let mut dim = generate(&GenerateOptions {
            width_tiles: 1,
            length_tiles: 1,
            ..Default::default()
        })
        .unwrap();
        let cherry = biomes::find("cherry_grove").unwrap();
        let paint = PaintSettings {
            layer: layers::ALL
                .iter()
                .position(|l| l.id == layers::BIOME.id)
                .unwrap(),
            biome: cherry,
            ..PaintSettings::default()
        };
        let brush = Brush::default();
        let mut stroke = StrokeState::default();
        let area = apply(
            Tool::Layer,
            &mut dim,
            64,
            64,
            0.016,
            false,
            &brush,
            &paint,
            &mut stroke,
        );
        assert!(area.is_some(), "the stroke reported no change");
        assert_eq!(column_biome(&dim, 64, 64), Some(cherry));
    }
}

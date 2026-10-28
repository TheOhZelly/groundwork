//! Editing operations. Ports the behaviour of the core GUI tools in
//! `org.pepsoft.worldpainter.operations` (Raise/Lower, Flatten, Smooth,
//! paint terrain / layers, Flood) without the Swing dependencies.
//!
//! Every operation takes a [`Shape`] and a [`Falloff`] and returns the number
//! of columns it touched. Columns without a tile are skipped.

use crate::dimension::Dimension;
use crate::layers::{self, DataSize, LayerDef};
use crate::noise::hash01;
use crate::terrain::Terrain;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Circle {
        x: i32,
        y: i32,
        radius: f32,
    },
    Square {
        x: i32,
        y: i32,
        radius: i32,
    },
    /// A circle with a ragged, natural outline: the edge is pushed in and
    /// out by noise fixed in world space, so overlapping dabs agree.
    Organic {
        x: i32,
        y: i32,
        radius: f32,
        seed: u64,
    },
    /// Inclusive corners.
    Rect {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    },
}

/// How strongly a brush applies away from its centre (Java `BrushShape` +
/// `Brush` intensity curves).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Falloff {
    /// Full strength everywhere.
    Constant,
    /// Linear ramp from 1 at the centre to 0 at the edge.
    Linear,
    /// Cosine ramp: flat top, smooth edges.
    Smooth,
    /// Quadratic: strong centre, quick decay.
    Spike,
}

impl Falloff {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "constant" | "flat" | "none" => Falloff::Constant,
            "linear" => Falloff::Linear,
            "smooth" | "cosine" => Falloff::Smooth,
            "spike" => Falloff::Spike,
            _ => return None,
        })
    }

    fn apply(self, d: f32) -> f32 {
        let d = d.clamp(0.0, 1.0);
        match self {
            Falloff::Constant => 1.0,
            Falloff::Linear => 1.0 - d,
            Falloff::Smooth => 0.5 + 0.5 * (d * std::f32::consts::PI).cos(),
            Falloff::Spike => (1.0 - d) * (1.0 - d),
        }
    }
}

impl Shape {
    /// Visits each column with its brush strength (0..=1, never 0).
    pub fn for_each(&self, falloff: Falloff, mut f: impl FnMut(i32, i32, f32)) {
        match *self {
            Shape::Organic { x, y, radius, seed } => {
                let r = radius.max(1.0);
                let noise = crate::noise::PerlinNoise::new(seed);
                // Features about half the radius across, scaled with the brush.
                let scale = (r as f64 / 2.0).max(4.0);
                let reach = (r * 1.45).ceil() as i32;
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let (wx, wy) = (x + dx, y + dy);
                        let warp =
                            noise.fbm(wx as f64 / scale, wy as f64 / scale, 4, 0.5) as f32 * 0.45;
                        let d = ((dx * dx + dy * dy) as f32).sqrt() / r + warp;
                        if d < 1.0 {
                            let s = falloff.apply(d.max(0.0));
                            if s > 0.0 {
                                f(wx, wy, s);
                            }
                        }
                    }
                }
            }
            Shape::Circle { x, y, radius } => {
                let r = radius.max(0.0);
                let ri = r.ceil() as i32;
                for dy in -ri..=ri {
                    for dx in -ri..=ri {
                        let d = ((dx * dx + dy * dy) as f32).sqrt();
                        if d <= r {
                            let s = if r == 0.0 {
                                1.0
                            } else {
                                falloff.apply(d / (r + 0.5))
                            };
                            if s > 0.0 {
                                f(x + dx, y + dy, s);
                            }
                        }
                    }
                }
            }
            Shape::Square { x, y, radius } => {
                let r = radius.max(0);
                for dy in -r..=r {
                    for dx in -r..=r {
                        let d = dx.abs().max(dy.abs()) as f32 / (r as f32 + 0.5);
                        let s = falloff.apply(d);
                        if s > 0.0 {
                            f(x + dx, y + dy, s);
                        }
                    }
                }
            }
            Shape::Rect { x0, y0, x1, y1 } => {
                let (x0, x1) = (x0.min(x1), x0.max(x1));
                let (y0, y1) = (y0.min(y1), y0.max(y1));
                let (cx, cy) = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
                let (hw, hh) = ((x1 - x0) as f32 / 2.0 + 0.5, (y1 - y0) as f32 / 2.0 + 0.5);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let d = ((x as f32 - cx).abs() / hw).max((y as f32 - cy).abs() / hh);
                        let s = falloff.apply(d);
                        if s > 0.0 {
                            f(x, y, s);
                        }
                    }
                }
            }
        }
    }

    pub fn area(&self) -> u64 {
        match *self {
            Shape::Organic { radius, .. } => {
                let d = 2 * (radius.max(1.0) * 1.45).ceil() as u64 + 1;
                d * d
            }
            Shape::Circle { radius, .. } => {
                let d = 2 * radius.max(0.0).ceil() as u64 + 1;
                d * d
            }
            Shape::Square { radius, .. } => {
                let d = 2 * radius.max(0) as u64 + 1;
                d * d
            }
            Shape::Rect { x0, y0, x1, y1 } => {
                ((x1 - x0).unsigned_abs() as u64 + 1) * ((y1 - y0).unsigned_abs() as u64 + 1)
            }
        }
    }
}

/// Raises (positive `amount`) or lowers terrain by up to `amount` blocks.
pub fn raise_lower(dim: &mut Dimension, shape: Shape, falloff: Falloff, amount: f32) -> usize {
    let mut n = 0;
    shape.for_each(falloff, |x, y, s| {
        if let Some(h) = dim.height_at(x, y) {
            dim.set_height_at(x, y, h + amount * s);
            n += 1;
        }
    });
    n
}

/// Moves terrain towards `target` (default: the height at the shape's
/// first column / centre). With [`Falloff::Constant`] the area becomes flat.
pub fn flatten(dim: &mut Dimension, shape: Shape, falloff: Falloff, target: Option<f32>) -> usize {
    let target = match target.or_else(|| {
        let (x, y) = centre(&shape);
        dim.height_at(x, y)
    }) {
        Some(t) => t,
        None => return 0,
    };
    let mut n = 0;
    shape.for_each(falloff, |x, y, s| {
        if let Some(h) = dim.height_at(x, y) {
            dim.set_height_at(x, y, h + (target - h) * s);
            n += 1;
        }
    });
    n
}

/// Smooths terrain with repeated 3×3 box blurs blended by brush strength.
pub fn smooth(dim: &mut Dimension, shape: Shape, falloff: Falloff, iterations: u32) -> usize {
    let mut columns = Vec::new();
    shape.for_each(falloff, |x, y, s| {
        if dim.contains(x, y) {
            columns.push((x, y, s));
        }
    });
    for _ in 0..iterations.max(1) {
        let blurred: Vec<f32> = columns
            .iter()
            .map(|&(x, y, _)| {
                let (mut sum, mut count) = (0.0, 0.0);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if let Some(h) = dim.height_at(x + dx, y + dy) {
                            sum += h;
                            count += 1.0;
                        }
                    }
                }
                sum / count
            })
            .collect();
        for (&(x, y, s), b) in columns.iter().zip(blurred) {
            let h = dim.height_at(x, y).unwrap();
            dim.set_height_at(x, y, h + (b - h) * s);
        }
    }
    columns.len()
}

/// Dithered painting: partial strength paints a matching fraction of columns,
/// like Java's random-threshold painting but deterministic.
fn dither(seed: u64, x: i32, y: i32, strength: f32) -> bool {
    strength >= 1.0 || hash01(seed, x, y) < strength
}

/// Which columns a paint operation may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mask {
    #[default]
    All,
    /// Only columns whose ground is above their water level, so painting
    /// land doesn't spill onto the sea floor or lakes.
    DryLand,
}

impl Mask {
    fn allows(self, dim: &Dimension, x: i32, y: i32) -> bool {
        match self {
            Mask::All => true,
            Mask::DryLand => match (dim.int_height_at(x, y), dim.water_level_at(x, y)) {
                (Some(h), Some(w)) => w <= h,
                _ => false,
            },
        }
    }
}

pub fn paint_terrain(
    dim: &mut Dimension,
    shape: Shape,
    falloff: Falloff,
    terrain: Terrain,
) -> usize {
    paint_terrain_masked(dim, shape, falloff, terrain, Mask::All)
}

pub fn paint_terrain_masked(
    dim: &mut Dimension,
    shape: Shape,
    falloff: Falloff,
    terrain: Terrain,
    mask: Mask,
) -> usize {
    let seed = dim.seed ^ 0x7465_7272;
    let mut n = 0;
    shape.for_each(falloff, |x, y, s| {
        if dither(seed, x, y, s) && mask.allows(dim, x, y) && dim.set_terrain_at(x, y, terrain) {
            n += 1;
        }
    });
    n
}

/// Sets a layer value. For bit layers partial strength is dithered; for
/// nibble/byte layers other than Biome the value is scaled by strength.
pub fn paint_layer(
    dim: &mut Dimension,
    shape: Shape,
    falloff: Falloff,
    layer: &LayerDef,
    value: u8,
) -> usize {
    paint_layer_masked(dim, shape, falloff, layer, value, Mask::All)
}

pub fn paint_layer_masked(
    dim: &mut Dimension,
    shape: Shape,
    falloff: Falloff,
    layer: &LayerDef,
    value: u8,
    mask: Mask,
) -> usize {
    let seed = dim.seed ^ 0x6c61_7965;
    let scaled = layer.size == DataSize::Nibble
        || (layer.size == DataSize::Byte && layer.id != layers::BIOME.id);
    let mut n = 0;
    shape.for_each(falloff, |x, y, s| {
        let applied = if scaled {
            Some(((value as f32) * s).round() as u8)
        } else if dither(seed, x, y, s) {
            Some(value)
        } else {
            None
        };
        if let Some(v) = applied.filter(|_| mask.allows(dim, x, y)) {
            if dim.set_layer_value_at(layer, x, y, v) {
                n += 1;
            }
        }
    });
    n
}

/// Sets the water level of every column in the shape, regardless of terrain.
pub fn set_water_level(dim: &mut Dimension, shape: Shape, level: i32, lava: bool) -> usize {
    let mut n = 0;
    shape.for_each(Falloff::Constant, |x, y, _| {
        if dim.set_water_level_at(x, y, level) {
            dim.set_layer_value_at(layers::FLOOD_WITH_LAVA, x, y, lava as u8);
            n += 1;
        }
    });
    n
}

/// Flood fill, like Java's `Flood` operation: starting at (`x`, `y`), raises
/// the water level of every connected column whose terrain is below `level`.
/// With `level` at or below the terrain at the start point this drains the
/// connected body of water instead. Stops after `max_columns` columns.
pub fn flood(
    dim: &mut Dimension,
    x: i32,
    y: i32,
    level: i32,
    lava: bool,
    max_columns: usize,
) -> Result<usize, String> {
    let start_height = dim
        .int_height_at(x, y)
        .ok_or("no tile at the start point")?;
    let start_water = dim.water_level_at(x, y).unwrap();
    let draining = level <= start_height;
    if draining && start_water <= start_height {
        return Err("start point is dry; nothing to drain".into());
    }

    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(x, y)]);
    let mut changed: HashMap<(i32, i32), ()> = HashMap::new();
    seen.insert((x, y));
    while let Some((cx, cy)) = queue.pop_front() {
        let (Some(h), Some(w)) = (dim.int_height_at(cx, cy), dim.water_level_at(cx, cy)) else {
            continue;
        };
        let include = if draining {
            w > h && w == start_water
        } else {
            h < level && w < level
        };
        if !include {
            continue;
        }
        if changed.len() >= max_columns {
            return Err(format!(
                "flood exceeded {max_columns} columns; is the area enclosed?"
            ));
        }
        changed.insert((cx, cy), ());
        for (nx, ny) in [(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)] {
            if seen.insert((nx, ny)) {
                queue.push_back((nx, ny));
            }
        }
    }
    let new_level = if draining { dim.min_height } else { level };
    for &(cx, cy) in changed.keys() {
        dim.set_water_level_at(cx, cy, new_level);
        dim.set_layer_value_at(layers::FLOOD_WITH_LAVA, cx, cy, (lava && !draining) as u8);
    }
    Ok(changed.len())
}

fn centre(shape: &Shape) -> (i32, i32) {
    match *shape {
        Shape::Circle { x, y, .. } | Shape::Square { x, y, .. } | Shape::Organic { x, y, .. } => {
            (x, y)
        }
        Shape::Rect { x0, y0, x1, y1 } => ((x0 + x1) / 2, (y0 + y1) / 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimension::DimensionKind;

    fn flat(height: f32) -> Dimension {
        let mut d = Dimension::new(DimensionKind::Overworld, -64, 320, 1);
        for ty in -1..1 {
            for tx in -1..1 {
                let t = d.ensure_tile(tx, ty);
                for y in 0..128 {
                    for x in 0..128 {
                        t.set_height(x, y, height);
                    }
                }
            }
        }
        d
    }

    #[test]
    fn raise_with_falloff() {
        let mut d = flat(60.0);
        let n = raise_lower(
            &mut d,
            Shape::Circle {
                x: 0,
                y: 0,
                radius: 10.0,
            },
            Falloff::Linear,
            10.0,
        );
        assert!(n > 300);
        assert!(d.height_at(0, 0).unwrap() > 69.0);
        assert!(d.height_at(9, 0).unwrap() < 62.0);
        assert_eq!(d.height_at(20, 0), Some(60.0));
    }

    #[test]
    fn flatten_and_smooth() {
        let mut d = flat(60.0);
        raise_lower(
            &mut d,
            Shape::Square {
                x: 0,
                y: 0,
                radius: 2,
            },
            Falloff::Constant,
            20.0,
        );
        smooth(
            &mut d,
            Shape::Square {
                x: 0,
                y: 0,
                radius: 6,
            },
            Falloff::Constant,
            3,
        );
        let peak = d.height_at(0, 0).unwrap();
        assert!(peak < 80.0 && peak > 60.0);
        flatten(
            &mut d,
            Shape::Rect {
                x0: -10,
                y0: -10,
                x1: 10,
                y1: 10,
            },
            Falloff::Constant,
            Some(50.0),
        );
        assert_eq!(d.height_at(0, 0), Some(50.0));
        assert_eq!(d.height_at(-10, 10), Some(50.0));
    }

    #[test]
    fn flood_fills_basin_only() {
        let mut d = flat(70.0);
        // Dig a 21×21 basin at height 50 surrounded by land at 70.
        flatten(
            &mut d,
            Shape::Square {
                x: 0,
                y: 0,
                radius: 10,
            },
            Falloff::Constant,
            Some(50.0),
        );
        let n = flood(&mut d, 0, 0, 60, false, 100_000).unwrap();
        assert_eq!(n, 21 * 21);
        assert_eq!(d.water_level_at(0, 0), Some(60));
        assert_eq!(d.water_level_at(20, 20), Some(-64));
        // Drain it again.
        assert_eq!(flood(&mut d, 0, 0, 0, false, 100_000).unwrap(), 21 * 21);
        assert_eq!(d.water_level_at(0, 0), Some(-64));
    }

    #[test]
    fn organic_brush_and_dry_land_mask() {
        let mut d = flat(60.0);
        // Water over the west half.
        set_water_level(
            &mut d,
            Shape::Rect {
                x0: -128,
                y0: -128,
                x1: -1,
                y1: 127,
            },
            70,
            false,
        );
        let organic = Shape::Organic {
            x: 0,
            y: 0,
            radius: 60.0,
            seed: 9,
        };
        let n = paint_terrain_masked(
            &mut d,
            organic,
            Falloff::Constant,
            Terrain::Sand,
            Mask::DryLand,
        );
        // Roughly half a 60-block circle, with a ragged edge.
        assert!((3000..9000).contains(&n), "{n}");
        assert_eq!(
            d.terrain_at(-10, 0),
            Some(Terrain::Grass),
            "painted under water"
        );
        assert_eq!(d.terrain_at(10, 0), Some(Terrain::Sand));
        // The outline is not a circle: along a ring at 0.9 radius some
        // columns are in and some out.
        let ring: Vec<bool> = (0..64)
            .map(|i| {
                let a = i as f32 / 64.0 * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
                let (x, y) = ((a.cos() * 54.0) as i32, (a.sin() * 54.0) as i32);
                d.terrain_at(x, y) == Some(Terrain::Sand)
            })
            .collect();
        assert!(ring.contains(&true) && ring.contains(&false), "{ring:?}");
    }

    #[test]
    fn flood_refuses_runaway() {
        let mut d = flat(50.0);
        assert!(flood(&mut d, 0, 0, 60, false, 1000).is_err());
    }

    #[test]
    fn paint_with_dither() {
        let mut d = flat(60.0);
        let n = paint_terrain(
            &mut d,
            Shape::Circle {
                x: 0,
                y: 0,
                radius: 30.0,
            },
            Falloff::Linear,
            Terrain::Sand,
        );
        assert!(n > 0 && n < 2900);
        assert_eq!(d.terrain_at(0, 0), Some(Terrain::Sand));
        paint_layer(
            &mut d,
            Shape::Rect {
                x0: 0,
                y0: 0,
                x1: 3,
                y1: 3,
            },
            Falloff::Constant,
            layers::BIOME,
            2,
        );
        assert_eq!(d.layer_value_at(layers::BIOME, 3, 3), Some(2));
    }
}

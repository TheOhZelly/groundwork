//! A 128×128 block area of a dimension. Port of `org.pepsoft.worldpainter.Tile`.
//!
//! Heights are stored as Java does: `(height - min_height) * 256` in an
//! integer, giving 1/256 block precision. The Java version switches between
//! `short` and `int` storage depending on world height; the Rust version
//! always uses `i32` (Java's "tall" representation).

use crate::layers::{DataSize, LayerDef};
use crate::terrain::Terrain;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const TILE_SIZE: i32 = 128;
pub const TILE_SIZE_BITS: i32 = 7;
pub const TILE_SIZE_MASK: i32 = TILE_SIZE - 1;
const AREA: usize = (TILE_SIZE * TILE_SIZE) as usize;

#[derive(Debug, Clone, Serialize, Deserialize)]
enum LayerBuffer {
    Bit(Vec<u64>),
    /// One value per byte for simplicity; values are 0..=15.
    Nibble(Vec<u8>),
    Byte(Vec<u8>),
}

impl LayerBuffer {
    fn new(def: &LayerDef) -> Self {
        match def.size {
            DataSize::Bit => {
                LayerBuffer::Bit(vec![if def.default != 0 { u64::MAX } else { 0 }; AREA / 64])
            }
            DataSize::Nibble => LayerBuffer::Nibble(vec![def.default; AREA]),
            DataSize::Byte => LayerBuffer::Byte(vec![def.default; AREA]),
        }
    }

    fn get(&self, i: usize) -> u8 {
        match self {
            LayerBuffer::Bit(bits) => ((bits[i >> 6] >> (i & 63)) & 1) as u8,
            LayerBuffer::Nibble(v) | LayerBuffer::Byte(v) => v[i],
        }
    }

    fn set(&mut self, i: usize, value: u8) {
        match self {
            LayerBuffer::Bit(bits) => {
                if value != 0 {
                    bits[i >> 6] |= 1 << (i & 63);
                } else {
                    bits[i >> 6] &= !(1 << (i & 63));
                }
            }
            LayerBuffer::Nibble(v) => v[i] = value.min(15),
            LayerBuffer::Byte(v) => v[i] = value,
        }
    }

    fn is_all(&self, value: u8) -> bool {
        match self {
            LayerBuffer::Bit(bits) => {
                let fill = if value != 0 { u64::MAX } else { 0 };
                bits.iter().all(|&b| b == fill)
            }
            LayerBuffer::Nibble(v) | LayerBuffer::Byte(v) => v.iter().all(|&b| b == value),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tile {
    x: i32,
    y: i32,
    min_height: i32,
    max_height: i32,
    terrain: Vec<u8>,
    height: Vec<i32>,
    water: Vec<i32>,
    layers: BTreeMap<String, LayerBuffer>,
}

#[inline]
fn idx(x: i32, y: i32) -> usize {
    debug_assert!(
        (0..TILE_SIZE).contains(&x) && (0..TILE_SIZE).contains(&y),
        "{x},{y} outside tile"
    );
    (x | (y << TILE_SIZE_BITS)) as usize
}

impl Tile {
    /// A new tile at tile coordinates (`x`, `y`) with everything at the
    /// bottom of the world, terrain [`Terrain::Grass`] and no water.
    pub fn new(x: i32, y: i32, min_height: i32, max_height: i32) -> Self {
        assert!(max_height > min_height, "max_height must exceed min_height");
        Tile {
            x,
            y,
            min_height,
            max_height,
            terrain: vec![Terrain::Grass.ordinal(); AREA],
            height: vec![0; AREA],
            water: vec![0; AREA],
            layers: BTreeMap::new(),
        }
    }

    pub fn x(&self) -> i32 {
        self.x
    }

    pub fn y(&self) -> i32 {
        self.y
    }

    pub fn min_height(&self) -> i32 {
        self.min_height
    }

    pub fn max_height(&self) -> i32 {
        self.max_height
    }

    fn clamp_height(&self, height: f32) -> f32 {
        height.clamp(self.min_height as f32, (self.max_height - 1) as f32)
    }

    pub fn height(&self, x: i32, y: i32) -> f32 {
        self.height[idx(x, y)] as f32 / 256.0 + self.min_height as f32
    }

    /// Height rounded the way Java's `Math.round` does.
    pub fn int_height(&self, x: i32, y: i32) -> i32 {
        crate::java_round(self.height(x, y))
    }

    /// Sets the height, clamped to the dimension's vertical range.
    pub fn set_height(&mut self, x: i32, y: i32, height: f32) {
        let h = self.clamp_height(height);
        self.height[idx(x, y)] = ((h - self.min_height as f32) * 256.0) as i32;
    }

    /// Raw height: `(height - min_height) * 256`.
    pub fn raw_height(&self, x: i32, y: i32) -> i32 {
        self.height[idx(x, y)]
    }

    pub fn set_raw_height(&mut self, x: i32, y: i32, raw: i32) {
        let max_raw = (self.max_height - 1 - self.min_height) * 256;
        self.height[idx(x, y)] = raw.clamp(0, max_raw);
    }

    pub fn lowest_height(&self) -> f32 {
        *self.height.iter().min().unwrap() as f32 / 256.0 + self.min_height as f32
    }

    pub fn highest_height(&self) -> f32 {
        *self.height.iter().max().unwrap() as f32 / 256.0 + self.min_height as f32
    }

    pub fn terrain(&self, x: i32, y: i32) -> Terrain {
        // Ordinals are only ever written from valid Terrain values.
        Terrain::from_ordinal(self.terrain[idx(x, y)]).unwrap_or(Terrain::Grass)
    }

    pub fn set_terrain(&mut self, x: i32, y: i32, terrain: Terrain) {
        self.terrain[idx(x, y)] = terrain.ordinal();
    }

    /// The y coordinate of the highest water (or lava) block; at or below the
    /// terrain height means "dry".
    pub fn water_level(&self, x: i32, y: i32) -> i32 {
        self.water[idx(x, y)] + self.min_height
    }

    pub fn set_water_level(&mut self, x: i32, y: i32, level: i32) {
        self.water[idx(x, y)] = level.clamp(self.min_height, self.max_height - 1) - self.min_height;
    }

    pub fn layer_value(&self, layer: &LayerDef, x: i32, y: i32) -> u8 {
        self.layers
            .get(layer.id)
            .map_or(layer.default, |b| b.get(idx(x, y)))
    }

    pub fn set_layer_value(&mut self, layer: &LayerDef, x: i32, y: i32, value: u8) {
        let value = value.min(layer.size.max_value());
        if let Some(buffer) = self.layers.get_mut(layer.id) {
            buffer.set(idx(x, y), value);
        } else if value != layer.default {
            let mut buffer = LayerBuffer::new(layer);
            buffer.set(idx(x, y), value);
            self.layers.insert(layer.id.to_owned(), buffer);
        }
    }

    /// Ids of layers with at least one non-default value.
    pub fn layers(&self) -> Vec<&str> {
        self.layers
            .iter()
            .filter(|(id, buffer)| {
                let default = crate::layers::find(id).map_or(0, |l| l.default);
                !buffer.is_all(default)
            })
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Drops layer buffers that only hold default values.
    pub fn compact(&mut self) {
        self.layers.retain(|id, buffer| {
            let default = crate::layers::find(id).map_or(0, |l| l.default);
            !buffer.is_all(default)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers;

    #[test]
    fn height_precision_and_clamping() {
        let mut t = Tile::new(0, 0, -64, 320);
        t.set_height(5, 7, 62.5);
        assert_eq!(t.height(5, 7), 62.5);
        assert_eq!(t.int_height(5, 7), 63);
        assert_eq!(t.raw_height(5, 7), (62.5f32 + 64.0) as i32 * 256 + 128);
        t.set_height(0, 0, 1000.0);
        assert_eq!(t.height(0, 0), 319.0);
        t.set_height(0, 0, -1000.0);
        assert_eq!(t.height(0, 0), -64.0);
    }

    #[test]
    fn layers_are_lazy() {
        let mut t = Tile::new(0, 0, 0, 256);
        t.set_layer_value(layers::FROST, 1, 1, 0);
        assert!(t.layers().is_empty());
        t.set_layer_value(layers::FROST, 127, 127, 1);
        assert_eq!(t.layer_value(layers::FROST, 127, 127), 1);
        assert_eq!(t.layer_value(layers::FROST, 126, 127), 0);
        assert_eq!(t.layer_value(layers::BIOME, 0, 0), 255);
        assert_eq!(t.layers(), ["Frost"]);
        t.set_layer_value(layers::FROST, 127, 127, 0);
        t.compact();
        assert!(t.layers().is_empty());
    }
}

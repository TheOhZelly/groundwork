//! A dimension: a sparse collection of tiles plus export settings.
//! Port of the data side of `org.pepsoft.worldpainter.Dimension`.

use crate::layers::LayerDef;
use crate::terrain::Terrain;
use crate::tile::{Tile, TILE_SIZE, TILE_SIZE_BITS, TILE_SIZE_MASK};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DimensionKind {
    Overworld,
    Nether,
    End,
}

impl DimensionKind {
    /// Vertical range of the vanilla dimension type in Minecraft 1.18+.
    pub fn vanilla_height_range(self) -> (i32, i32) {
        match self {
            DimensionKind::Overworld => (-64, 320),
            DimensionKind::Nether | DimensionKind::End => (0, 256),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "overworld" | "surface" | "normal" => Some(DimensionKind::Overworld),
            "nether" | "the_nether" => Some(DimensionKind::Nether),
            "end" | "the_end" => Some(DimensionKind::End),
            _ => None,
        }
    }
}

/// Inclusive block-coordinate rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extent {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Extent {
    pub fn width(&self) -> i32 {
        self.x1 - self.x0 + 1
    }

    pub fn height(&self) -> i32 {
        self.y1 - self.y0 + 1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dimension {
    pub kind: DimensionKind,
    pub min_height: i32,
    pub max_height: i32,
    pub seed: u64,
    /// Block filling the column below the top layer.
    pub subsurface_block: String,
    /// Minimum depth of the terrain's top layer; noise adds up to 3 more.
    pub top_layer_depth: i32,
    /// Whether to put a bedrock floor at `min_height`.
    pub bedrock: bool,
    /// Tiles are shared copy-on-write: cloning a dimension (for undo, or to
    /// export in the background) only copies pointers, and the first edit to
    /// a tile afterwards copies just that tile.
    tiles: HashMap<(i32, i32), Arc<Tile>>,
    /// Resources (ores) level applied everywhere, 0–15, in addition to the
    /// painted Resources layer. WorldPainter's default for new worlds is 8.
    pub resources_level: u8,
    /// Caverns level applied everywhere, 0–15, in addition to the painted
    /// Caverns layer.
    pub caverns_level: u8,
}

/// Splits a block coordinate into (tile, offset-in-tile).
#[inline]
pub fn split(c: i32) -> (i32, i32) {
    (c >> TILE_SIZE_BITS, c & TILE_SIZE_MASK)
}

impl Dimension {
    pub fn new(kind: DimensionKind, min_height: i32, max_height: i32, seed: u64) -> Self {
        Dimension {
            kind,
            min_height,
            max_height,
            seed,
            subsurface_block: match kind {
                DimensionKind::Overworld => "minecraft:stone",
                DimensionKind::Nether => "minecraft:netherrack",
                DimensionKind::End => "minecraft:end_stone",
            }
            .to_owned(),
            top_layer_depth: 3,
            bedrock: kind != DimensionKind::End,
            tiles: HashMap::new(),
            resources_level: if kind == DimensionKind::Overworld {
                8
            } else {
                0
            },
            caverns_level: 0,
        }
    }

    /// Rebuilds a dimension saved in format version 1, which predates the
    /// resources and caverns settings.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_v1(
        kind: DimensionKind,
        min_height: i32,
        max_height: i32,
        seed: u64,
        subsurface_block: String,
        top_layer_depth: i32,
        bedrock: bool,
        tiles: HashMap<(i32, i32), Tile>,
    ) -> Self {
        Dimension {
            subsurface_block,
            top_layer_depth,
            bedrock,
            tiles: tiles.into_iter().map(|(k, t)| (k, Arc::new(t))).collect(),
            ..Dimension::new(kind, min_height, max_height, seed)
        }
    }

    pub fn tile(&self, tx: i32, ty: i32) -> Option<&Tile> {
        self.tiles.get(&(tx, ty)).map(|t| &**t)
    }

    /// The shared handle of a tile. Two handles that are `Arc::ptr_eq` hold
    /// identical contents, which lets callers skip unchanged tiles cheaply.
    pub fn tile_handle(&self, tx: i32, ty: i32) -> Option<&Arc<Tile>> {
        self.tiles.get(&(tx, ty))
    }

    /// Mutable access, copying the tile first if it is shared.
    pub fn tile_mut(&mut self, tx: i32, ty: i32) -> Option<&mut Tile> {
        self.tiles.get_mut(&(tx, ty)).map(Arc::make_mut)
    }

    /// Adds an empty tile if none exists yet. Returns the tile.
    pub fn ensure_tile(&mut self, tx: i32, ty: i32) -> &mut Tile {
        let (min, max) = (self.min_height, self.max_height);
        Arc::make_mut(
            self.tiles
                .entry((tx, ty))
                .or_insert_with(|| Arc::new(Tile::new(tx, ty, min, max))),
        )
    }

    pub fn insert_tile(&mut self, tile: Tile) {
        assert_eq!(
            (tile.min_height(), tile.max_height()),
            (self.min_height, self.max_height)
        );
        self.tiles.insert((tile.x(), tile.y()), Arc::new(tile));
    }

    pub fn remove_tile(&mut self, tx: i32, ty: i32) -> Option<Tile> {
        self.tiles.remove(&(tx, ty)).map(Arc::unwrap_or_clone)
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Tile coordinates, sorted for deterministic iteration.
    pub fn tile_coords(&self) -> Vec<(i32, i32)> {
        let mut coords: Vec<_> = self.tiles.keys().copied().collect();
        coords.sort_unstable();
        coords
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile> {
        self.tiles.values().map(|t| &**t)
    }

    /// Mutable access to every tile; copies any that are shared.
    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile> {
        self.tiles.values_mut().map(Arc::make_mut)
    }

    /// Compacts every tile that is not shared with a snapshot (see
    /// [`Tile::compact`]). Shared tiles are left alone rather than copied.
    pub fn compact(&mut self) {
        for tile in self.tiles.values_mut() {
            if let Some(tile) = Arc::get_mut(tile) {
                tile.compact();
            }
        }
    }

    /// Approximate memory used by tile data, counting shared tiles once
    /// per dimension.
    pub fn heap_bytes(&self) -> usize {
        self.tiles
            .values()
            .map(|t| std::mem::size_of::<Tile>() + t.heap_bytes())
            .sum()
    }

    /// Block extent covered by tiles, if any.
    pub fn extent(&self) -> Option<Extent> {
        let mut it = self.tiles.keys();
        let &(fx, fy) = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (fx, fy, fx, fy);
        for &(x, y) in it {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Some(Extent {
            x0: x0 * TILE_SIZE,
            y0: y0 * TILE_SIZE,
            x1: (x1 + 1) * TILE_SIZE - 1,
            y1: (y1 + 1) * TILE_SIZE - 1,
        })
    }

    fn column(&self, x: i32, y: i32) -> Option<(&Tile, i32, i32)> {
        let ((tx, lx), (ty, ly)) = (split(x), split(y));
        self.tiles.get(&(tx, ty)).map(|t| (&**t, lx, ly))
    }

    fn column_mut(&mut self, x: i32, y: i32) -> Option<(&mut Tile, i32, i32)> {
        let ((tx, lx), (ty, ly)) = (split(x), split(y));
        self.tiles
            .get_mut(&(tx, ty))
            .map(|t| (Arc::make_mut(t), lx, ly))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.column(x, y).is_some()
    }

    pub fn height_at(&self, x: i32, y: i32) -> Option<f32> {
        self.column(x, y).map(|(t, lx, ly)| t.height(lx, ly))
    }

    pub fn int_height_at(&self, x: i32, y: i32) -> Option<i32> {
        self.column(x, y).map(|(t, lx, ly)| t.int_height(lx, ly))
    }

    /// Returns false if there is no tile at the location.
    pub fn set_height_at(&mut self, x: i32, y: i32, height: f32) -> bool {
        self.column_mut(x, y)
            .map(|(t, lx, ly)| t.set_height(lx, ly, height))
            .is_some()
    }

    pub fn terrain_at(&self, x: i32, y: i32) -> Option<Terrain> {
        self.column(x, y).map(|(t, lx, ly)| t.terrain(lx, ly))
    }

    pub fn set_terrain_at(&mut self, x: i32, y: i32, terrain: Terrain) -> bool {
        self.column_mut(x, y)
            .map(|(t, lx, ly)| t.set_terrain(lx, ly, terrain))
            .is_some()
    }

    pub fn water_level_at(&self, x: i32, y: i32) -> Option<i32> {
        self.column(x, y).map(|(t, lx, ly)| t.water_level(lx, ly))
    }

    pub fn set_water_level_at(&mut self, x: i32, y: i32, level: i32) -> bool {
        self.column_mut(x, y)
            .map(|(t, lx, ly)| t.set_water_level(lx, ly, level))
            .is_some()
    }

    pub fn layer_value_at(&self, layer: &LayerDef, x: i32, y: i32) -> Option<u8> {
        self.column(x, y)
            .map(|(t, lx, ly)| t.layer_value(layer, lx, ly))
    }

    pub fn set_layer_value_at(&mut self, layer: &LayerDef, x: i32, y: i32, value: u8) -> bool {
        self.column_mut(x, y)
            .map(|(t, lx, ly)| t.set_layer_value(layer, lx, ly, value))
            .is_some()
    }

    /// Steepest gradient around a column, as in Java's `Tile.doGetSlope`.
    /// Missing neighbours are treated as level with the centre.
    pub fn slope_at(&self, x: i32, y: i32) -> Option<f32> {
        const SQRT_OF_EIGHT: f32 = 2.828_427;
        let centre = self.height_at(x, y)?;
        let h = |dx: i32, dy: i32| self.height_at(x + dx, y + dy).unwrap_or(centre);
        Some(
            ((h(1, 0) - h(-1, 0)).abs() / 2.0)
                .max((h(1, 1) - h(-1, -1)).abs() / SQRT_OF_EIGHT)
                .max((h(0, 1) - h(0, -1)).abs() / 2.0)
                .max((h(-1, 1) - h(1, -1)).abs() / SQRT_OF_EIGHT),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_coordinates_map_to_tiles() {
        assert_eq!(split(-1), (-1, 127));
        assert_eq!(split(-128), (-1, 0));
        assert_eq!(split(128), (1, 0));
        let mut d = Dimension::new(DimensionKind::Overworld, -64, 320, 1);
        d.ensure_tile(-1, -1);
        assert!(d.set_height_at(-1, -1, 70.0));
        assert_eq!(d.height_at(-1, -1), Some(70.0));
        assert!(!d.set_height_at(0, 0, 70.0));
        assert_eq!(
            d.extent(),
            Some(Extent {
                x0: -128,
                y0: -128,
                x1: -1,
                y1: -1
            })
        );
    }
}

#[cfg(test)]
mod sharing_tests {
    use super::*;

    #[test]
    fn clones_share_tiles_until_written() {
        let mut a = Dimension::new(DimensionKind::Overworld, -64, 320, 1);
        a.ensure_tile(0, 0);
        a.ensure_tile(1, 0);
        let b = a.clone();
        assert!(Arc::ptr_eq(
            a.tile_handle(0, 0).unwrap(),
            b.tile_handle(0, 0).unwrap()
        ));
        a.set_height_at(5, 5, 90.0);
        // Only the written tile was copied; the snapshot is unchanged.
        assert!(!Arc::ptr_eq(
            a.tile_handle(0, 0).unwrap(),
            b.tile_handle(0, 0).unwrap()
        ));
        assert!(Arc::ptr_eq(
            a.tile_handle(1, 0).unwrap(),
            b.tile_handle(1, 0).unwrap()
        ));
        assert_eq!(a.height_at(5, 5), Some(90.0));
        assert_eq!(b.height_at(5, 5), Some(-64.0));
    }
}

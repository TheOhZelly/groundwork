//! Layers: per-column values painted on top of the terrain.
//!
//! Port of the subset of `org.pepsoft.worldpainter.layers` that the Rust
//! exporter supports so far. Ids match the Java layer names.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataSize {
    /// On/off.
    Bit,
    /// 0..=15.
    Nibble,
    /// 0..=255.
    Byte,
}

impl DataSize {
    pub fn max_value(self) -> u8 {
        match self {
            DataSize::Bit => 1,
            DataSize::Nibble => 15,
            DataSize::Byte => 255,
        }
    }
}

#[derive(Debug)]
pub struct LayerDef {
    pub id: &'static str,
    pub description: &'static str,
    pub size: DataSize,
    pub default: u8,
}

pub const FROST: &LayerDef = &LayerDef {
    id: "Frost",
    description: "snow on land and ice on top of water",
    size: DataSize::Bit,
    default: 0,
};
pub const VOID: &LayerDef = &LayerDef {
    id: "Void",
    description: "no blocks at all in the column",
    size: DataSize::Bit,
    default: 0,
};
pub const FLOOD_WITH_LAVA: &LayerDef = &LayerDef {
    id: "FloodWithLava",
    description: "the column is flooded with lava instead of water",
    size: DataSize::Bit,
    default: 0,
};
pub const BIOME: &LayerDef = &LayerDef {
    id: "Biome",
    description: "biome id (see list_biomes); 255 means automatic",
    size: DataSize::Byte,
    default: crate::biomes::BIOME_AUTO,
};

/// Port of `DeciduousForest`: oak and birch trees. Density 0–15; a column
/// gets a tree with probability (density² + 1) / 1280, as in Java.
pub const DECIDUOUS: &LayerDef = &LayerDef {
    id: "Deciduous",
    description: "a deciduous forest of oak and birch trees; 0–15 sets the density",
    size: DataSize::Nibble,
    default: 0,
};
/// Port of `PineForest`: spruce trees, same density rule.
pub const PINE: &LayerDef = &LayerDef {
    id: "Pine",
    description: "a pine forest of spruce trees; 0–15 sets the density",
    size: DataSize::Nibble,
    default: 0,
};

pub const ALL: [&LayerDef; 6] = [FROST, VOID, FLOOD_WITH_LAVA, BIOME, DECIDUOUS, PINE];

pub const TREE_LAYERS: [&LayerDef; 2] = [DECIDUOUS, PINE];

pub fn find(id: &str) -> Option<&'static LayerDef> {
    ALL.iter().copied().find(|l| l.id.eq_ignore_ascii_case(id))
}

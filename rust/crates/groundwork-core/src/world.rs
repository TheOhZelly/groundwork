//! A world. Port of the data side of WorldPainter's `World2`.

use crate::dimension::{Dimension, DimensionKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
    Hardcore,
}

impl GameMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "survival" => GameMode::Survival,
            "creative" => GameMode::Creative,
            "adventure" => GameMode::Adventure,
            "spectator" => GameMode::Spectator,
            "hardcore" => GameMode::Hardcore,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub name: String,
    pub seed: u64,
    /// Spawn point in world x/y (Minecraft x/z).
    pub spawn: (i32, i32),
    pub game_mode: GameMode,
    pub allow_cheats: bool,
    /// Whether vanilla structures generate outside the painted area.
    pub map_features: bool,
    dimensions: Vec<Dimension>,
}

impl World {
    /// Reassembles a world from saved parts (used by format migrations).
    pub(crate) fn from_parts(
        name: String,
        seed: u64,
        spawn: (i32, i32),
        game_mode: GameMode,
        allow_cheats: bool,
        map_features: bool,
        dimensions: Vec<Dimension>,
    ) -> Self {
        World {
            name,
            seed,
            spawn,
            game_mode,
            allow_cheats,
            map_features,
            dimensions,
        }
    }

    pub fn new(name: impl Into<String>, seed: u64, overworld: Dimension) -> Self {
        assert_eq!(overworld.kind, DimensionKind::Overworld);
        World {
            name: name.into(),
            seed,
            spawn: (0, 0),
            game_mode: GameMode::Survival,
            allow_cheats: false,
            map_features: true,
            dimensions: vec![overworld],
        }
    }

    pub fn dimension(&self, kind: DimensionKind) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.kind == kind)
    }

    pub fn dimension_mut(&mut self, kind: DimensionKind) -> Option<&mut Dimension> {
        self.dimensions.iter_mut().find(|d| d.kind == kind)
    }

    pub fn dimensions(&self) -> &[Dimension] {
        &self.dimensions
    }

    /// Adds or replaces a dimension.
    pub fn set_dimension(&mut self, dimension: Dimension) {
        self.dimensions.retain(|d| d.kind != dimension.kind);
        self.dimensions.push(dimension);
        self.dimensions.sort_by_key(|d| d.kind as u8);
    }

    pub fn remove_dimension(&mut self, kind: DimensionKind) -> Option<Dimension> {
        if kind == DimensionKind::Overworld {
            return None;
        }
        let i = self.dimensions.iter().position(|d| d.kind == kind)?;
        Some(self.dimensions.remove(i))
    }
}

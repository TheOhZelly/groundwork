//! Core of Groundwork, a Rust port of WorldPainter.
//!
//! The model mirrors the Java version: a [`World`] has [`Dimension`]s, which
//! are made of 128×128 [`Tile`]s. Each column of a tile stores a height
//! (with 1/256 block precision), a water level, a [`Terrain`] and values for
//! any number of [`layers`]. World coordinates use x/y for the horizontal
//! plane, as in Java; y maps to Minecraft's z axis on export.

pub mod biomes;
#[rustfmt::skip]
mod biomes_table;
pub mod dimension;
pub mod generate;
pub mod heightmap;
pub mod io;
pub mod layers;
pub mod noise;
pub mod ops;
pub mod render;
pub mod terrain;
pub mod tile;
pub mod world;

pub use dimension::{Dimension, DimensionKind, Extent};
pub use terrain::Terrain;
pub use tile::{Tile, TILE_SIZE, TILE_SIZE_BITS, TILE_SIZE_MASK};
pub use world::{GameMode, World};

/// Java's `Math.round(float)`: rounds half up.
pub(crate) fn java_round(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

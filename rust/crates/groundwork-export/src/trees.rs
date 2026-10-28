//! Trees for the Deciduous and Pine layers.
//!
//! Simplified port of `TreesExporter` and the renderers in
//! `org.pepsoft.worldpainter.layers.trees`: the same density rule (a column
//! with layer strength `s` gets a tree when `random(1280) <= s²`), with
//! simpler oak, birch and spruce shapes and no mushrooms or vines yet.
//!
//! Every chunk looks for trees up to [`REACH`] blocks beyond its edges and
//! draws only the blocks that fall inside it. Placement depends only on the
//! seed and column, so neighbouring chunks agree and canopies cross chunk
//! borders seamlessly even though chunks are exported in parallel.

use crate::chunk::Blocks;
use groundwork_core::noise::splitmix64;
use groundwork_core::{layers, Dimension};

/// Furthest a tree's leaves reach from its trunk.
const REACH: i32 = 3;
const TREE_CHANCE: u64 = 1280;
const SALT: u64 = 0x7472_6565;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Oak,
    Birch,
    Spruce,
}

impl Kind {
    fn log(self) -> &'static str {
        match self {
            Kind::Oak => "minecraft:oak_log",
            Kind::Birch => "minecraft:birch_log",
            Kind::Spruce => "minecraft:spruce_log",
        }
    }

    /// Persistent, so leaves never decay regardless of log distance.
    fn leaves(self) -> &'static str {
        match self {
            Kind::Oak => "minecraft:oak_leaves[persistent=true]",
            Kind::Birch => "minecraft:birch_leaves[persistent=true]",
            Kind::Spruce => "minecraft:spruce_leaves[persistent=true]",
        }
    }
}

struct Tree {
    kind: Kind,
    /// World x/z of the trunk.
    x: i32,
    z: i32,
    /// Lowest trunk block.
    base: i32,
    height: i32,
    /// Per-tree randomness for leaf details.
    noise: u64,
}

fn column_hash(seed: u64, x: i32, z: i32) -> u64 {
    splitmix64(seed ^ SALT ^ ((x as u32 as u64) << 32 | z as u32 as u64))
}

/// Soil a tree can grow on.
fn plantable(block: &str) -> bool {
    matches!(
        block,
        "minecraft:grass_block"
            | "minecraft:dirt"
            | "minecraft:coarse_dirt"
            | "minecraft:podzol"
            | "minecraft:moss_block"
            | "minecraft:mud"
            | "minecraft:mycelium"
    )
}

fn tree_at(dim: &Dimension, x: i32, z: i32, max_y: i32) -> Option<Tree> {
    let deciduous = dim.layer_value_at(layers::DECIDUOUS, x, z)?;
    let pine = dim.layer_value_at(layers::PINE, x, z)?;
    let strength = deciduous.max(pine) as u64;
    if strength == 0 {
        return None;
    }
    let hash = column_hash(dim.seed, x, z);
    if hash % TREE_CHANCE > strength * strength {
        return None;
    }
    if dim.layer_value_at(layers::VOID, x, z) == Some(1) {
        return None;
    }
    let ground = dim.int_height_at(x, z)?;
    if dim.water_level_at(x, z)? > ground
        || !plantable(dim.terrain_at(x, z)?.info().top.unwrap_or(""))
    {
        return None;
    }
    let roll = (hash >> 16) as u32;
    let kind = if pine >= deciduous {
        Kind::Spruce
    } else if roll.is_multiple_of(5) {
        Kind::Birch
    } else {
        Kind::Oak
    };
    let height = match kind {
        Kind::Oak => 4 + (roll >> 4) % 3,
        Kind::Birch => 5 + (roll >> 4) % 3,
        Kind::Spruce => 6 + (roll >> 4) % 5,
    } as i32;
    let base = ground + 1;
    // Room for the trunk plus the leaves above it.
    if base + height + 2 >= max_y {
        return None;
    }
    Some(Tree {
        kind,
        x,
        z,
        base,
        height,
        noise: hash.rotate_left(29),
    })
}

/// Draws all trees that can reach into the chunk. Trunks go in first so a
/// neighbour's leaves never cut through them.
pub fn place(dim: &Dimension, blocks: &mut Blocks, chunk_x: i32, chunk_z: i32, max_y: i32) {
    let (x0, z0) = (chunk_x * 16, chunk_z * 16);
    let mut trees = Vec::new();
    for z in z0 - REACH..z0 + 16 + REACH {
        for x in x0 - REACH..x0 + 16 + REACH {
            if let Some(tree) = tree_at(dim, x, z, max_y) {
                trees.push(tree);
            }
        }
    }
    for tree in &trees {
        let log = blocks.id(tree.kind.log());
        for y in tree.base..tree.base + tree.height {
            blocks.set_if_air(tree.x - x0, y, tree.z - z0, log);
        }
    }
    for tree in &trees {
        let leaves = blocks.id(tree.kind.leaves());
        let mut put = |dx: i32, y: i32, dz: i32| {
            blocks.set_if_air(tree.x + dx - x0, y, tree.z + dz - z0, leaves)
        };
        let top = tree.base + tree.height - 1;
        match tree.kind {
            Kind::Oak | Kind::Birch => {
                // Two wide layers with ragged corners, then two narrow ones.
                for (layer, y) in [(0u32, top - 2), (1, top - 1)] {
                    for dz in -2i32..=2 {
                        for dx in -2i32..=2 {
                            let corner = dx.abs() == 2 && dz.abs() == 2;
                            let bit = ((dx + 2) * 5 + dz + 2) as u32 + layer * 25;
                            if !corner || (tree.noise >> (bit % 64)) & 1 == 1 {
                                put(dx, y, dz);
                            }
                        }
                    }
                }
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0
                            || dz == 0
                            || (tree.noise >> (dx + 1 + (dz + 1) * 3 + 40)) & 1 == 1
                        {
                            put(dx, top, dz);
                        }
                    }
                }
                for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                    put(dx, top + 1, dz);
                }
            }
            Kind::Spruce => {
                // A cone of diamond-shaped layers that alternate in width.
                put(0, top + 1, 0);
                let max_radius = if tree.height >= 9 { 3 } else { 2 };
                let lowest = tree.base + 2;
                for (i, y) in (lowest..=top).rev().enumerate() {
                    let radius = match i {
                        0 => 1,
                        _ if i % 2 == 1 => (1 + i as i32 / 3).min(max_radius),
                        _ => 1,
                    };
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            if dx.abs() + dz.abs() <= radius {
                                put(dx, y, dz);
                            }
                        }
                    }
                }
            }
        }
    }
}

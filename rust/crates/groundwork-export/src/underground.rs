//! Ores and caves: the Resources and Caverns layers.
//!
//! **Resources** keeps WorldPainter's default overworld table from
//! `ResourcesExporter` (materials, per-mille amounts, height ranges and the
//! deepslate ore variants), but places each material as seeded ellipsoid
//! pockets instead of thresholding a 3D noise per block per material. The
//! expected amount per block of rock is the same; the export is about a
//! thousand times cheaper, which matters for maps of hundreds of thousands
//! of chunks.
//!
//! **Caverns** is a direct port of `CavernsExporter`'s formula: 3D noise with
//! a bias that keeps caves out of the top layer and off the world floor.

use crate::chunk::Blocks;
use groundwork_core::noise::{splitmix64, PerlinNoise, MEDIUM_BLOBS, SMALL_BLOBS};
use groundwork_core::{layers, Dimension};

struct Ore {
    block: &'static str,
    deepslate: Option<&'static str>,
    /// Blocks per thousand at Resources level 8.
    promille: f32,
    min_y: i32,
    max_y: i32,
    /// Pocket radius range in blocks.
    radius: (f32, f32),
}

/// WorldPainter's `DIM_NORMAL` defaults, in its order.
const ORES: [Ore; 12] = [
    Ore {
        block: "minecraft:dirt",
        deepslate: None,
        promille: 57.0,
        min_y: 0,
        max_y: i32::MAX,
        radius: (2.0, 4.0),
    },
    Ore {
        block: "minecraft:gravel",
        deepslate: None,
        promille: 28.0,
        min_y: i32::MIN,
        max_y: i32::MAX,
        radius: (2.0, 4.0),
    },
    Ore {
        block: "minecraft:gold_ore",
        deepslate: Some("minecraft:deepslate_gold_ore"),
        promille: 1.0,
        min_y: i32::MIN,
        max_y: 31,
        radius: (1.0, 1.8),
    },
    Ore {
        block: "minecraft:iron_ore",
        deepslate: Some("minecraft:deepslate_iron_ore"),
        promille: 6.0,
        min_y: i32::MIN,
        max_y: i32::MAX,
        radius: (1.0, 2.0),
    },
    Ore {
        block: "minecraft:coal_ore",
        deepslate: Some("minecraft:deepslate_coal_ore"),
        promille: 10.0,
        min_y: 0,
        max_y: i32::MAX,
        radius: (1.2, 2.2),
    },
    Ore {
        block: "minecraft:lapis_ore",
        deepslate: Some("minecraft:deepslate_lapis_ore"),
        promille: 1.0,
        min_y: i32::MIN,
        max_y: 31,
        radius: (1.0, 1.6),
    },
    Ore {
        block: "minecraft:diamond_ore",
        deepslate: Some("minecraft:deepslate_diamond_ore"),
        promille: 1.0,
        min_y: i32::MIN,
        max_y: 15,
        radius: (0.8, 1.4),
    },
    Ore {
        block: "minecraft:redstone_ore",
        deepslate: Some("minecraft:deepslate_redstone_ore"),
        promille: 8.0,
        min_y: i32::MIN,
        max_y: 15,
        radius: (1.0, 1.8),
    },
    Ore {
        block: "minecraft:water",
        deepslate: None,
        promille: 1.0,
        min_y: i32::MIN,
        max_y: i32::MAX,
        radius: (1.0, 2.0),
    },
    Ore {
        block: "minecraft:lava",
        deepslate: None,
        promille: 2.0,
        min_y: i32::MIN,
        max_y: 15,
        radius: (1.0, 2.0),
    },
    Ore {
        block: "minecraft:emerald_ore",
        deepslate: Some("minecraft:deepslate_emerald_ore"),
        promille: 1.0,
        min_y: 64,
        max_y: i32::MAX,
        radius: (0.8, 1.2),
    },
    Ore {
        block: "minecraft:copper_ore",
        deepslate: Some("minecraft:deepslate_copper_ore"),
        promille: 6.0,
        min_y: 0,
        max_y: 88,
        radius: (1.0, 2.0),
    },
];

/// Rock that ores may replace and caverns may hollow out.
pub(crate) fn is_rock(name: &str) -> bool {
    let id = name.split('[').next().unwrap_or(name);
    matches!(
        id,
        "minecraft:stone"
            | "minecraft:deepslate"
            | "minecraft:tuff"
            | "minecraft:granite"
            | "minecraft:diorite"
            | "minecraft:andesite"
            | "minecraft:calcite"
            | "minecraft:netherrack"
            | "minecraft:end_stone"
            | "minecraft:blackstone"
            | "minecraft:basalt"
    )
}

/// Per-column facts the column pass leaves for the underground passes.
pub(crate) struct Columns {
    /// Highest block below the top layer, per local (x, z); below the world
    /// floor for columns that get nothing (void).
    pub rock_top: [[i32; 16]; 16],
    pub height: [[i32; 16]; 16],
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(splitmix64(seed))
    }
    fn next(&mut self) -> u64 {
        self.0 = splitmix64(self.0);
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// Adds ore, dirt, gravel, water and lava pockets.
pub(crate) fn resources(
    dim: &Dimension,
    blocks: &mut Blocks,
    columns: &Columns,
    chunk_x: i32,
    chunk_z: i32,
    min_y: i32,
) {
    let has_layer = |cx: i32, cz: i32| {
        (0..2).any(|i| {
            (0..2).any(|j| {
                dim.layer_value_at(layers::RESOURCES, cx * 16 + i * 15, cz * 16 + j * 15)
                    .unwrap_or(0)
                    > 0
            })
        })
    };
    // Pockets reach up to ~5 blocks, so neighbouring chunks' pockets can
    // cross into this one; every chunk generates the same pockets for a
    // given chunk position, so the halves line up.
    for ncz in chunk_z - 1..=chunk_z + 1 {
        for ncx in chunk_x - 1..=chunk_x + 1 {
            if dim.resources_level == 0 && !has_layer(ncx, ncz) {
                continue;
            }
            let top = top_of_chunk(dim, ncx, ncz);
            for (k, ore) in ORES.iter().enumerate() {
                let (lo, hi) = (ore.min_y.max(min_y + 1), ore.max_y.min(top));
                if hi < lo {
                    continue;
                }
                let mut rng = Rng::new(
                    dim.seed
                        ^ (ncx as u32 as u64) << 32
                        ^ (ncz as u32 as u64)
                        ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                );
                let mean_r = (ore.radius.0 + ore.radius.1) / 2.0;
                let pocket_volume = 4.0 / 3.0 * std::f32::consts::PI * mean_r.powi(3);
                // Pockets for the richest level (15); each is kept with
                // probability level / 15, so level 8 gives Java's amount.
                let expected = ore.promille / 1000.0 * 256.0 * (hi - lo + 1) as f32 / pocket_volume
                    * 15.0
                    / 8.0;
                let count = expected as u32 + (rng.unit() < expected.fract()) as u32;
                for _ in 0..count {
                    let (px, pz) = (
                        ncx * 16 + rng.range(0.0, 16.0) as i32,
                        ncz * 16 + rng.range(0.0, 16.0) as i32,
                    );
                    let py = lo + rng.range(0.0, (hi - lo + 1) as f32) as i32;
                    let r = rng.range(ore.radius.0, ore.radius.1);
                    let radii = [
                        r * rng.range(0.7, 1.3),
                        r * rng.range(0.6, 1.0),
                        r * rng.range(0.7, 1.3),
                    ];
                    let keep = rng.unit();
                    let level = dim
                        .resources_level
                        .max(dim.layer_value_at(layers::RESOURCES, px, pz).unwrap_or(0));
                    if keep >= level as f32 / 15.0 {
                        continue;
                    }
                    pocket(
                        blocks,
                        columns,
                        ore,
                        (px - chunk_x * 16, py, pz - chunk_z * 16),
                        radii,
                    );
                }
            }
        }
    }
}

/// Rough top of the terrain in a chunk, from five sample columns.
fn top_of_chunk(dim: &Dimension, cx: i32, cz: i32) -> i32 {
    [(0, 0), (15, 0), (0, 15), (15, 15), (8, 8)]
        .iter()
        .filter_map(|&(i, j)| dim.int_height_at(cx * 16 + i, cz * 16 + j))
        .max()
        .unwrap_or(i32::MIN)
}

fn pocket(
    blocks: &mut Blocks,
    columns: &Columns,
    ore: &Ore,
    (cx, cy, cz): (i32, i32, i32),
    radii: [f32; 3],
) {
    let id = blocks.id(ore.block);
    let deepslate_id = ore.deepslate.map(|d| blocks.id(d));
    let reach = radii.iter().fold(0.0f32, |a, &b| a.max(b)).ceil() as i32;
    for x in (cx - reach).max(0)..=(cx + reach).min(15) {
        for z in (cz - reach).max(0)..=(cz + reach).min(15) {
            let rock_top = columns.rock_top[z as usize][x as usize];
            for y in cy - reach..=(cy + reach).min(rock_top) {
                let d = ((x - cx) as f32 / radii[0]).powi(2)
                    + ((y - cy) as f32 / radii[1]).powi(2)
                    + ((z - cz) as f32 / radii[2]).powi(2);
                if d > 1.0 {
                    continue;
                }
                let Some(existing) = blocks.get(x, y, z) else {
                    continue;
                };
                let name = blocks.name(existing);
                if !is_rock(name) {
                    continue;
                }
                let new = match deepslate_id {
                    Some(deep) if name.starts_with("minecraft:deepslate") => deep,
                    _ => id,
                };
                blocks.put(x, y, z, new);
            }
        }
    }
}

/// Hollows out caverns, as `CavernsExporter` does.
pub(crate) fn caverns(
    dim: &Dimension,
    noise: &PerlinNoise,
    blocks: &mut Blocks,
    columns: &Columns,
    chunk_x: i32,
    chunk_z: i32,
    min_y: i32,
) {
    const CAVERN_CHANCE: f32 = 0.5;
    let air = blocks.id("minecraft:air");
    for z in 0..16 {
        for x in 0..16 {
            let (wx, wz) = (chunk_x * 16 + x, chunk_z * 16 + z);
            let level = dim
                .caverns_level
                .max(dim.layer_value_at(layers::CAVERNS, wx, wz).unwrap_or(0));
            if level == 0 {
                continue;
            }
            // Java keeps at least three blocks of top layer over caves.
            let ceiling = columns.rock_top[z as usize][x as usize]
                .min(columns.height[z as usize][x as usize] - 3);
            for y in (min_y + 1..=ceiling).rev() {
                let below_surface = ceiling - y;
                let above_floor = y - 1 - min_y;
                let edge = below_surface.min(above_floor).min(10);
                let bias =
                    CAVERN_CHANCE * (0.1 * (10 - edge) as f32).max(1.0 - level as f32 / 15.0);
                let likelihood = noise.noise3(
                    wx as f64 / MEDIUM_BLOBS,
                    wz as f64 / MEDIUM_BLOBS,
                    y as f64 / SMALL_BLOBS,
                ) + 0.5
                    - bias;
                if likelihood > CAVERN_CHANCE {
                    if let Some(existing) = blocks.get(x, y, z) {
                        let name = blocks.name(existing);
                        let ore = name.ends_with("_ore")
                            || name == "minecraft:dirt"
                            || name == "minecraft:gravel";
                        if is_rock(name) || ore {
                            blocks.put(x, y, z, air);
                        }
                    }
                }
            }
        }
    }
}

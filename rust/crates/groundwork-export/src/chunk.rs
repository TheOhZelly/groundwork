//! Builds the NBT for one 1.18+ chunk from a tile.

use groundwork_core::dimension::split;
use groundwork_core::noise::hash01;
use groundwork_core::terrain::Fidelity;
use groundwork_core::{biomes, layers, Dimension, DimensionKind, Terrain, Tile};
use groundwork_nbt::{compound, Compound, List, Tag, TAG_COMPOUND};
use std::collections::BTreeSet;
use std::sync::Mutex;

const AIR: &str = "minecraft:air";
const STONE: &str = "minecraft:stone";

/// Chunk-wide block palette plus per-block indices into it. Palette
/// entries are block ids with optional properties, e.g.
/// `minecraft:oak_leaves[persistent=true]`.
pub(crate) struct Blocks {
    palette: Vec<String>,
    min_y: i32,
    data: Vec<u16>,
}

impl Blocks {
    fn new(min_y: i32, max_y: i32) -> Self {
        Blocks {
            palette: vec![AIR.to_owned()],
            min_y,
            data: vec![0; ((max_y - min_y) * 256) as usize],
        }
    }

    pub(crate) fn id(&mut self, name: &str) -> u16 {
        match self.palette.iter().position(|p| p == name) {
            Some(i) => i as u16,
            None => {
                self.palette.push(name.to_owned());
                (self.palette.len() - 1) as u16
            }
        }
    }

    fn name(&self, id: u16) -> &str {
        &self.palette[id as usize]
    }

    fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        self.data[(((y - self.min_y) * 256) + z * 16 + x) as usize] = id;
    }

    /// Places a block only where there is air, so trees never cut into
    /// terrain or water. Coordinates are chunk-local; out-of-range ones are
    /// ignored.
    pub(crate) fn set_if_air(&mut self, x: i32, y: i32, z: i32, id: u16) {
        let max_y = self.min_y + (self.data.len() / 256) as i32;
        if (0..16).contains(&x) && (0..16).contains(&z) && (self.min_y..max_y).contains(&y) {
            let i = (((y - self.min_y) * 256) + z * 16 + x) as usize;
            if self.data[i] == 0 {
                self.data[i] = id;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    dim: &Dimension,
    tile: &Tile,
    chunk_x: i32,
    chunk_z: i32,
    min_y: i32,
    max_y: i32,
    data_version: i32,
    warnings: &Mutex<Vec<String>>,
) -> Compound {
    let mut blocks = Blocks::new(min_y, max_y);
    let mut column_biomes = [[0u8; 4]; 4];
    let mut problems = BTreeSet::new();
    let deepslate_transition = dim.subsurface_block == STONE;

    for lz in 0..16 {
        for lx in 0..16 {
            let (wx, wz) = (chunk_x * 16 + lx, chunk_z * 16 + lz);
            let (tx, ty) = (split(wx).1, split(wz).1);
            let height = tile.int_height(tx, ty).clamp(min_y, max_y - 1);
            let water = tile.water_level(tx, ty).min(max_y - 1);
            let terrain = tile.terrain(tx, ty);
            let underwater = water > height;
            let frost = tile.layer_value(layers::FROST, tx, ty) == 1;
            let lava = tile.layer_value(layers::FLOOD_WITH_LAVA, tx, ty) == 1;

            if lx % 4 == 0 && lz % 4 == 0 {
                let value = tile.layer_value(layers::BIOME, tx, ty);
                column_biomes[(lz / 4) as usize][(lx / 4) as usize] = resolve_biome(
                    value,
                    dim.kind,
                    terrain,
                    water - height,
                    frost,
                    &mut problems,
                );
            }
            if tile.layer_value(layers::VOID, tx, ty) == 1 {
                continue;
            }

            let info = terrain.info();
            let (top, under) = match (info.top, info.under) {
                (Some(t), Some(u)) => (t, u),
                _ => {
                    problems.insert(format!(
                        "{} is a custom terrain, which is not supported yet; exported as stone",
                        info.name
                    ));
                    (STONE, STONE)
                }
            };
            if info.fidelity == Fidelity::Approximate {
                problems.insert(format!(
                    "{} mixes several materials in WorldPainter; Groundwork uses {} for now",
                    info.name, top
                ));
            }
            // Minecraft turns submerged grass and mycelium into dirt on random
            // ticks anyway; do it up front. (Java WorldPainter leaves this to
            // the game.)
            let top = if underwater && matches!(top, "minecraft:grass_block" | "minecraft:mycelium")
            {
                "minecraft:dirt"
            } else {
                top
            };

            let top_id = blocks.id(top);
            let under_id = blocks.id(under);
            let bedrock_id = blocks.id("minecraft:bedrock");
            let subsurface_id = blocks.id(&dim.subsurface_block);
            let deepslate_id = blocks.id("minecraft:deepslate");
            let fluid_id = blocks.id(if lava {
                "minecraft:lava"
            } else {
                "minecraft:water"
            });
            let ice_id = blocks.id("minecraft:ice");
            let snow_id = blocks.id("minecraft:snow");
            let depth_limit = dim.top_layer_depth + (hash01(dim.seed, wx, wz) * 4.0) as i32;

            for y in min_y..=height {
                let id = if y == min_y && dim.bedrock {
                    bedrock_id
                } else {
                    match height - y {
                        0 => top_id,
                        d if d < depth_limit => under_id,
                        _ if deepslate_transition && is_deepslate(dim.seed, wx, y, wz) => {
                            deepslate_id
                        }
                        _ => subsurface_id,
                    }
                };
                blocks.set(lx, y, lz, id);
            }
            if underwater {
                for y in height + 1..=water {
                    let id = if frost && !lava && y == water {
                        ice_id
                    } else {
                        fluid_id
                    };
                    blocks.set(lx, y, lz, id);
                }
            } else if frost
                && height + 1 < max_y
                && top != "minecraft:water"
                && top != "minecraft:lava"
            {
                blocks.set(lx, height + 1, lz, snow_id);
            }
        }
    }

    crate::trees::place(dim, &mut blocks, chunk_x, chunk_z, max_y);

    if !problems.is_empty() {
        warnings.lock().unwrap().extend(problems);
    }

    let biome_tag = biome_container(&column_biomes);
    let mut sections = Vec::new();
    for sy in min_y.div_euclid(16)..=(max_y - 1).div_euclid(16) {
        let start = ((sy * 16 - min_y) * 256) as usize;
        let indices = &blocks.data[start..start + 4096];
        sections.push(compound! {
            "Y" => sy as i8,
            "block_states" => block_container(&blocks, indices),
            "biomes" => biome_tag.clone(),
        });
    }

    compound! {
        "DataVersion" => data_version,
        "xPos" => chunk_x,
        "yPos" => min_y.div_euclid(16),
        "zPos" => chunk_z,
        "Status" => "minecraft:full",
        "LastUpdate" => 0i64,
        "InhabitedTime" => 0i64,
        "isLightOn" => false,
        "sections" => List::compounds(sections),
        "block_entities" => List::empty(TAG_COMPOUND),
        "block_ticks" => List::empty(TAG_COMPOUND),
        "fluid_ticks" => List::empty(TAG_COMPOUND),
        "structures" => compound! { "References" => compound! {}, "starts" => compound! {} },
    }
}

/// Vanilla-like stone→deepslate gradient between y=0 and y=8.
fn is_deepslate(seed: u64, x: i32, y: i32, z: i32) -> bool {
    y < 0 || (y < 8 && hash01(seed ^ 0xdee5, x ^ (y << 20), z) < (8 - y) as f32 / 8.0)
}

/// WorldPainter's Biome layer value → biome id, with a simple automatic
/// choice for columns left at 255 ("auto").
fn resolve_biome(
    value: u8,
    kind: DimensionKind,
    terrain: Terrain,
    water_depth: i32,
    frost: bool,
    problems: &mut BTreeSet<String>,
) -> u8 {
    const PLAINS: u8 = 1;
    if value != biomes::BIOME_AUTO {
        if biomes::modern_id(value).is_some() {
            return value;
        }
        problems.insert(format!(
            "biome id {value} has no Minecraft 1.18+ equivalent; exported as plains"
        ));
        return PLAINS;
    }
    let name = match kind {
        DimensionKind::Nether => "nether_wastes",
        DimensionKind::End => "the_end",
        DimensionKind::Overworld => match terrain {
            _ if water_depth > 3 => {
                if frost {
                    "frozen_ocean"
                } else {
                    "ocean"
                }
            }
            _ if frost => "snowy_plains",
            Terrain::Beaches | Terrain::BareBeaches => "beach",
            Terrain::Desert | Terrain::Sand | Terrain::Sandstone => "desert",
            Terrain::Mesa | Terrain::RedDesert | Terrain::RedSand | Terrain::HardenedClay => {
                "badlands"
            }
            Terrain::Mycelium => "mushroom_fields",
            Terrain::DeepSnow | Terrain::Snow => "snowy_plains",
            Terrain::Netherlike | Terrain::Netherrack => "nether_wastes",
            Terrain::EndStone => "the_end",
            _ => "plains",
        },
    };
    biomes::find(name).unwrap_or(PLAINS)
}

/// `minecraft:x[a=b,c=d]` → `{Name: "minecraft:x", Properties: {a: "b", c: "d"}}`.
fn block_state(id: &str) -> Compound {
    match id.split_once('[') {
        None => compound! { "Name" => id },
        Some((name, props)) => {
            let mut properties = Compound::new();
            for pair in props.trim_end_matches(']').split(',') {
                if let Some((k, v)) = pair.split_once('=') {
                    properties.insert(k.trim().to_owned(), Tag::String(v.trim().to_owned()));
                }
            }
            compound! { "Name" => name, "Properties" => properties }
        }
    }
}

fn bits_for(len: usize) -> u32 {
    if len <= 1 {
        0
    } else {
        usize::BITS - (len - 1).leading_zeros()
    }
}

/// Packs values into longs without spanning, as Minecraft 1.16+ expects.
fn pack(values: impl ExactSizeIterator<Item = u16>, bits: u32) -> Vec<i64> {
    let per_long = (64 / bits) as usize;
    let mut out = vec![0u64; values.len().div_ceil(per_long)];
    for (i, v) in values.enumerate() {
        out[i / per_long] |= (v as u64) << ((i % per_long) as u32 * bits);
    }
    out.into_iter().map(|v| v as i64).collect()
}

fn block_container(blocks: &Blocks, indices: &[u16]) -> Compound {
    // Section-local palette in order of first appearance.
    let mut local: Vec<u16> = Vec::new();
    let mut remap = std::collections::HashMap::new();
    let packed: Vec<u16> = indices
        .iter()
        .map(|&g| {
            *remap.entry(g).or_insert_with(|| {
                local.push(g);
                (local.len() - 1) as u16
            })
        })
        .collect();
    let palette = List::compounds(local.iter().map(|&g| block_state(blocks.name(g))).collect());
    let mut c = compound! { "palette" => palette };
    if local.len() > 1 {
        let bits = bits_for(local.len()).max(4);
        c.insert(
            "data".into(),
            Tag::LongArray(pack(packed.into_iter(), bits)),
        );
    }
    c
}

/// Biomes are 4×4×4 cells; the Rust model has one biome per column, so all
/// four vertical cells of a section share it.
fn biome_container(columns: &[[u8; 4]; 4]) -> Compound {
    let mut palette: Vec<u8> = Vec::new();
    let mut cells = Vec::with_capacity(64);
    for _y in 0..4 {
        for row in columns {
            for &b in row {
                let i = palette.iter().position(|&p| p == b).unwrap_or_else(|| {
                    palette.push(b);
                    palette.len() - 1
                });
                cells.push(i as u16);
            }
        }
    }
    let names = palette
        .iter()
        .map(|&b| biomes::modern_id(b).unwrap_or("minecraft:plains"));
    let mut c = compound! { "palette" => List::strings(names) };
    if palette.len() > 1 {
        c.insert(
            "data".into(),
            Tag::LongArray(pack(cells.into_iter(), bits_for(palette.len()))),
        );
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_matches_minecraft_layout() {
        // 5 bits → 12 values per long, 4 bits of padding.
        let values: Vec<u16> = (0..4096).map(|i| (i % 17) as u16).collect();
        let packed = pack(values.iter().copied(), 5);
        assert_eq!(packed.len(), 342);
        let unpack = |i: usize| ((packed[i / 12] as u64 >> ((i % 12) * 5)) & 31) as u16;
        for i in [0, 11, 12, 4095] {
            assert_eq!(unpack(i), values[i]);
        }
        assert_eq!(bits_for(1), 0);
        assert_eq!(bits_for(2), 1);
        assert_eq!(bits_for(16), 4);
        assert_eq!(bits_for(17), 5);
    }
}

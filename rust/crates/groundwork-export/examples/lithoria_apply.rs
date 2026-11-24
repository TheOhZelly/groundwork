//! Builds a Groundwork world from per-column rasters (heights, water levels,
//! terrain, biomes, forests, frost) prepared by an external pipeline.
//!
//! ```text
//! cargo run --release -p groundwork-export --example lithoria_apply -- \
//!     RAW_DIR X0 Z0 SIZE|WxH OUT.gwork NAME SEED [PAD_TILES]
//! ```
//!
//! RAW_DIR holds SIZE×SIZE (or W×H) little-endian rasters, row-major from (X0, Z0):
//! `height.f32`, `water_level.i16`, `terrain.u8`, `biome.u8`, `deciduous.u8`,
//! `pine.u8`, `frost.u8`, plus `terrain_names.txt` and `biome_names.txt`
//! (line n names code n). X0, Z0 and SIZE must be multiples of 128.

use groundwork_core::{biomes, layers, Dimension, DimensionKind, Terrain, Tile, World, TILE_SIZE};
use std::path::{Path, PathBuf};

fn read(dir: &Path, name: &str, len: usize, elem: usize) -> Vec<u8> {
    let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(bytes.len(), len * elem, "{name} has the wrong size");
    bytes
}

fn names(dir: &Path, name: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 8 && a.len() != 9 {
        eprintln!("usage: lithoria_apply RAW_DIR X0 Z0 SIZE OUT.gwork NAME SEED [PAD_TILES]");
        std::process::exit(2);
    }
    // Optional ring of flat deep-ocean tiles around the rasters.
    let pad: i32 = a.get(8).map_or(0, |p| p.parse().unwrap());
    let dir = PathBuf::from(&a[1]);
    let (x0, z0): (i32, i32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let (w, h): (i32, i32) = match a[4].split_once('x') {
        Some((w, h)) => (w.parse().unwrap(), h.parse().unwrap()),
        None => (a[4].parse().unwrap(), a[4].parse().unwrap()),
    };
    let out = PathBuf::from(&a[5]);
    let seed: u64 = a[7].parse().unwrap();
    assert!(
        x0 % TILE_SIZE == 0 && z0 % TILE_SIZE == 0 && w % TILE_SIZE == 0 && h % TILE_SIZE == 0,
        "not tile-aligned"
    );
    let n = (w as usize) * (h as usize);

    let height = read(&dir, "height.f32", n, 4);
    let water = read(&dir, "water_level.i16", n, 2);
    let terrain = read(&dir, "terrain.u8", n, 1);
    let biome = read(&dir, "biome.u8", n, 1);
    let deciduous = read(&dir, "deciduous.u8", n, 1);
    let pine = read(&dir, "pine.u8", n, 1);
    let frost = read(&dir, "frost.u8", n, 1);
    let terrains: Vec<Terrain> = names(&dir, "terrain_names.txt")
        .iter()
        .map(|t| Terrain::find(t).unwrap_or_else(|| panic!("unknown terrain {t}")))
        .collect();
    let biome_ids: Vec<u8> = names(&dir, "biome_names.txt")
        .iter()
        .map(|b| biomes::find(b).unwrap_or_else(|| panic!("unknown biome {b}")))
        .collect();

    let started = std::time::Instant::now();
    let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320, seed);
    let (tiles_x, tiles_z) = (w / TILE_SIZE, h / TILE_SIZE);
    for ty in 0..tiles_z {
        for tx in 0..tiles_x {
            let mut tile = Tile::new(x0 / TILE_SIZE + tx, z0 / TILE_SIZE + ty, -64, 320);
            for ly in 0..TILE_SIZE {
                let row = (ty * TILE_SIZE + ly) as usize;
                for lx in 0..TILE_SIZE {
                    let i = row * w as usize + (tx * TILE_SIZE + lx) as usize;
                    let h = f32::from_le_bytes(height[i * 4..i * 4 + 4].try_into().unwrap());
                    let w = i16::from_le_bytes(water[i * 2..i * 2 + 2].try_into().unwrap());
                    tile.set_height(lx, ly, h);
                    tile.set_water_level(lx, ly, w as i32);
                    tile.set_terrain(lx, ly, terrains[terrain[i] as usize]);
                    tile.set_layer_value(layers::BIOME, lx, ly, biome_ids[biome[i] as usize]);
                    if deciduous[i] > 0 {
                        tile.set_layer_value(layers::DECIDUOUS, lx, ly, deciduous[i].min(15));
                    }
                    if pine[i] > 0 {
                        tile.set_layer_value(layers::PINE, lx, ly, pine[i].min(15));
                    }
                    if frost[i] > 0 {
                        tile.set_layer_value(layers::FROST, lx, ly, 1);
                    }
                }
            }
            dim.insert_tile(tile);
        }
    }
    if pad > 0 {
        let deep = biomes::find("minecraft:deep_ocean").unwrap();
        let (t0x, t0z) = (x0 / TILE_SIZE, z0 / TILE_SIZE);
        for ty in t0z - pad..t0z + tiles_z + pad {
            for tx in t0x - pad..t0x + tiles_x + pad {
                if dim.tile(tx, ty).is_some() {
                    continue;
                }
                let mut tile = Tile::new(tx, ty, -64, 320);
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        tile.set_height(lx, ly, -10.0);
                        tile.set_water_level(lx, ly, 62);
                        tile.set_terrain(lx, ly, Terrain::Gravel);
                        tile.set_layer_value(layers::BIOME, lx, ly, deep);
                    }
                }
                dim.insert_tile(tile);
            }
        }
    }
    dim.compact();
    let mut world = World::new(a[6].as_str(), seed, dim);
    world.spawn = (x0 + w / 2, z0 + h / 2);
    groundwork_core::io::save(&world, &out).expect("save .gwork");
    println!(
        "built {} tiles ({w}×{h} blocks at {x0},{z0}) in {:.1}s -> {}",
        tiles_x * tiles_z,
        started.elapsed().as_secs_f32(),
        out.display()
    );
}

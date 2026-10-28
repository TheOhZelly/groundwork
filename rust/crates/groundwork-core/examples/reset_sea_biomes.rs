//! Sets the Biome layer back to automatic wherever the ground is more than
//! three blocks underwater, so painting land biomes with big brushes doesn't
//! leave jungle or plains out at sea:
//! `cargo run --release -p groundwork-core --example reset_sea_biomes -- WORLD.gwork`

use groundwork_core::biomes::{modern_id, BIOME_AUTO};
use groundwork_core::{layers, DimensionKind};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: reset_sea_biomes WORLD.gwork");
    let mut world = groundwork_core::io::load(&path).expect("load world");
    let dim = world
        .dimension_mut(DimensionKind::Overworld)
        .expect("overworld");
    let mut reset = 0usize;
    for tile in dim.tiles_mut() {
        for y in 0..groundwork_core::TILE_SIZE {
            for x in 0..groundwork_core::TILE_SIZE {
                let depth = tile.water_level(x, y) - tile.int_height(x, y);
                let biome = tile.layer_value(layers::BIOME, x, y);
                let watery =
                    modern_id(biome).is_some_and(|m| m.contains("ocean") || m.contains("river"));
                if depth > 3 && biome != BIOME_AUTO && !watery {
                    tile.set_layer_value(layers::BIOME, x, y, BIOME_AUTO);
                    reset += 1;
                }
            }
        }
        tile.compact();
    }
    groundwork_core::io::save(&world, &path).expect("save world");
    println!("reset {reset} submerged columns to automatic biomes");
}

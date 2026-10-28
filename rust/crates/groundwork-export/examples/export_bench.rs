//! Times exporting a generated world with and without ores and caves:
//! `cargo run --release -p groundwork-export --example export_bench -- [TILES_PER_SIDE]`

use groundwork_core::generate::{generate, GenerateOptions};
use groundwork_core::World;
use groundwork_export::{export, ExportOptions};
use std::time::Instant;

fn main() {
    let side: u32 = std::env::args().nth(1).map_or(8, |s| s.parse().unwrap());
    let opts = GenerateOptions {
        width_tiles: side,
        length_tiles: side,
        seed: 4,
        ..Default::default()
    };
    let dir = std::env::temp_dir().join("groundwork-export-bench");
    for (label, resources, caverns) in [("plain", 0, 0), ("ores", 8, 0), ("ores + caverns 8", 8, 8)]
    {
        let mut dim = generate(&opts).unwrap();
        dim.resources_level = resources;
        dim.caverns_level = caverns;
        let world = World::new("Bench", 4, dim);
        let _ = std::fs::remove_dir_all(&dir);
        let t = Instant::now();
        let report = export(
            &world,
            &dir,
            &ExportOptions {
                threads: Some(6),
                ..Default::default()
            },
        )
        .unwrap();
        let s = t.elapsed().as_secs_f32();
        println!(
            "{label:>18}: {} chunks in {s:.2}s ({:.0} chunks/s)",
            report.chunks,
            report.chunks as f32 / s
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

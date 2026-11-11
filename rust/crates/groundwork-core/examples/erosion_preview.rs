//! Generates a hilly world, erodes it and renders both, to judge erosion:
//! `cargo run --release -p groundwork-core --example erosion_preview -- OUT_DIR [DROPLETS] [TILES] [HILLINESS]`

use groundwork_core::erosion::{erode, ErosionOptions};
use groundwork_core::generate::{generate, GenerateOptions};
use groundwork_core::render::{render_view, View};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: erosion_preview OUT_DIR [DROPLETS] [TILES] [HILLINESS]");
        std::process::exit(2);
    }
    let out = Path::new(&args[1]);
    let droplets = args.get(2).map_or(1.0, |s| s.parse().expect("DROPLETS"));
    let tiles = args.get(3).map_or(4, |s| s.parse().expect("TILES"));
    let hilliness = args.get(4).map_or(60.0, |s| s.parse().expect("HILLINESS"));
    let mut dim = generate(&GenerateOptions {
        width_tiles: tiles,
        length_tiles: tiles,
        hilliness,
        base_height: 80.0,
        seed: 42,
        ..Default::default()
    })
    .expect("generate");
    let save = |dim: &groundwork_core::Dimension, name: &str| {
        let (image, _) = render_view(dim, 1, 1024, View::Terrain).expect("world has tiles");
        std::fs::write(out.join(name), image.to_png().expect("encode PNG")).expect("write PNG");
    };
    std::fs::create_dir_all(out).expect("create OUT_DIR");
    save(&dim, "before.png");
    let started = Instant::now();
    let opts = ErosionOptions {
        droplets,
        seed: 42,
        threads: None,
    };
    let changed = erode(&mut dim, None, &opts);
    println!(
        "eroded {changed} columns in {:.2}s",
        started.elapsed().as_secs_f32()
    );
    save(&dim, "after.png");
}

//! Renders a saved world to a PNG:
//! `cargo run --release -p groundwork-core --example render_world -- WORLD.gwork OUT.png [terrain|biomes] [MAX_SIDE]`

use groundwork_core::render::{render_view, View};
use groundwork_core::DimensionKind;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: render_world WORLD.gwork OUT.png [terrain|biomes] [MAX_SIDE]");
        std::process::exit(2);
    }
    let view = match args.get(3).map(String::as_str) {
        Some("biomes") => View::Biomes,
        _ => View::Terrain,
    };
    let max_side = args.get(4).map_or(1024, |s| s.parse().expect("MAX_SIDE"));
    let world = groundwork_core::io::load(&args[1]).expect("load world");
    let dim = world.dimension(DimensionKind::Overworld).expect("overworld");
    let (image, scale) = render_view(dim, 1, max_side, view).expect("world has tiles");
    std::fs::write(&args[2], image.to_png().expect("encode PNG")).expect("write PNG");
    println!("{}×{} at {scale} blocks per pixel", image.width, image.height);
}

//! Loads a world and simulates editing with undo, to measure memory and
//! speed: `cargo run --release -p groundwork-core --example memory_bench -- WORLD.gwork [SAVE_AS.gwork]`
//! Peak memory is best read from outside, e.g. the process's peak working set.

use groundwork_core::ops::{self, Falloff, Shape};
use groundwork_core::DimensionKind;
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: memory_bench WORLD.gwork");
    let t = Instant::now();
    let mut world = groundwork_core::io::load(&path).expect("load");
    println!("load: {:.2}s", t.elapsed().as_secs_f32());
    let dim = world.dimension(DimensionKind::Overworld).unwrap();
    println!(
        "tile data: {} MB in {} tiles",
        dim.heap_bytes() / (1 << 20),
        dim.tile_count()
    );

    // An editor stroke: snapshot for undo, then a brush.
    let mut undo = Vec::new();
    let t = Instant::now();
    for i in 0..4 {
        undo.push(world.clone());
        let dim = world.dimension_mut(DimensionKind::Overworld).unwrap();
        ops::raise_lower(
            dim,
            Shape::Circle {
                x: i * 300,
                y: 0,
                radius: 40.0,
            },
            Falloff::Smooth,
            5.0,
        );
    }
    println!(
        "4 strokes with undo snapshots: {:.2}s",
        t.elapsed().as_secs_f32()
    );
    let t = Instant::now();
    let copy = world.clone();
    println!("one more world clone: {:.3}s", t.elapsed().as_secs_f32());
    drop((undo, copy));
    if let Some(out) = std::env::args().nth(2) {
        let t = Instant::now();
        groundwork_core::io::save(&world, &out).expect("save");
        println!("saved {out} in {:.2}s", t.elapsed().as_secs_f32());
    }
}

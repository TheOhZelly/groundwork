//! Exports a saved world to a Minecraft save from the command line:
//!
//! ```text
//! cargo run --release -p groundwork-export --example export_world -- \
//!     WORLD.gwork SAVE_DIR [--threads N] [--outside vanilla|void|ocean] [--version 1.21.11|1.21.1]
//! ```
//!
//! Replaces any world already in SAVE_DIR. Prints progress as region files
//! finish.

use groundwork_export::{export_with_progress, ExportOptions, McVersion, Outside};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: export_world WORLD.gwork SAVE_DIR [--threads N] [--outside vanilla|void|ocean] [--version 1.21.11|1.21.1]");
        std::process::exit(2);
    }
    let mut options = ExportOptions { overwrite: true, ..Default::default() };
    let mut rest = args[3..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--threads" => options.threads = Some(value.parse().expect("--threads takes a number")),
            "--outside" => {
                options.outside = match value.as_str() {
                    "vanilla" => Outside::Vanilla,
                    "void" => Outside::Void,
                    "ocean" => Outside::Ocean,
                    other => panic!("unknown --outside {other}"),
                }
            }
            "--version" => options.version = McVersion::parse(value).expect("--version is 1.21.11 or 1.21.1"),
            other => panic!("unknown option {other}"),
        }
    }

    let started = Instant::now();
    let world = groundwork_core::io::load(&args[1]).expect("load world");
    let threads = options.threads.unwrap_or_else(groundwork_export::balanced_threads);
    eprintln!("Exporting {:?} as Minecraft {} on {threads} threads to {}", world.name, options.version.name(), args[2]);
    let last = AtomicUsize::new(0);
    let report = export_with_progress(&world, &args[2], &options, |done, total| {
        let pct = done * 100 / total;
        if pct >= last.load(Ordering::Relaxed) + 5 {
            last.store(pct, Ordering::Relaxed);
            eprintln!("  {pct}% ({done}/{total} regions, {:.0}s)", started.elapsed().as_secs_f32());
        }
    })
    .expect("export");
    eprintln!(
        "Exported {} chunks in {} region files ({:.0}s)",
        report.chunks,
        report.regions,
        started.elapsed().as_secs_f32()
    );
    for warning in &report.warnings {
        eprintln!("  note: {warning}");
    }
}

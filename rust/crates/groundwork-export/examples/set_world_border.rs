//! Adds a world border to an exported save's `level.dat`.
//!
//! ```text
//! cargo run --release -p groundwork-export --example set_world_border -- SAVE_DIR SIZE [CENTRE_X CENTRE_Z]
//! ```

use groundwork_nbt::{Compression, Tag};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 && a.len() != 5 {
        eprintln!("usage: set_world_border SAVE_DIR SIZE [CENTRE_X CENTRE_Z]");
        std::process::exit(2);
    }
    let path = std::path::Path::new(&a[1]).join("level.dat");
    let size: f64 = a[2].parse().unwrap();
    let (cx, cz): (f64, f64) = if a.len() == 5 { (a[3].parse().unwrap(), a[4].parse().unwrap()) } else { (0.0, 0.0) };
    let (name, mut root) = groundwork_nbt::read_file(&path).expect("read level.dat");
    let Some(Tag::Compound(data)) = root.get_mut("Data") else { panic!("level.dat has no Data") };
    for (k, v) in [
        ("BorderCenterX", Tag::Double(cx)),
        ("BorderCenterZ", Tag::Double(cz)),
        ("BorderSize", Tag::Double(size)),
        ("BorderSizeLerpTarget", Tag::Double(size)),
        ("BorderSizeLerpTime", Tag::Long(0)),
        ("BorderSafeZone", Tag::Double(5.0)),
        ("BorderDamagePerBlock", Tag::Double(0.2)),
        ("BorderWarningBlocks", Tag::Double(5.0)),
        ("BorderWarningTime", Tag::Double(15.0)),
    ] {
        data.insert(k.to_owned(), v);
    }
    groundwork_nbt::write_file(&path, &name, &root, Compression::Gzip).expect("write level.dat");
    println!("world border {size} centred on {cx},{cz} written to {}", path.display());
}

//! Sets the overworld border of a save.
//!
//! ```text
//! cargo run --release -p groundwork-export --example set_world_border -- SAVE_DIR SIZE [CENTRE_X CENTRE_Z]
//! ```
//!
//! Minecraft 26.x keeps the border per dimension in
//! `dimensions/minecraft/overworld/data/minecraft/world_border.dat`; when that
//! file exists it is updated (other fields kept). Otherwise the 1.21 keys are
//! written into `level.dat`.

use groundwork_nbt::{Compression, Tag};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 && a.len() != 5 {
        eprintln!("usage: set_world_border SAVE_DIR SIZE [CENTRE_X CENTRE_Z]");
        std::process::exit(2);
    }
    let save = std::path::Path::new(&a[1]);
    let size: f64 = a[2].parse().unwrap();
    let (cx, cz): (f64, f64) = if a.len() == 5 { (a[3].parse().unwrap(), a[4].parse().unwrap()) } else { (0.0, 0.0) };

    let modern = save.join("dimensions/minecraft/overworld/data/minecraft/world_border.dat");
    if modern.exists() {
        let (name, mut root) = groundwork_nbt::read_file(&modern).expect("read world_border.dat");
        let Some(Tag::Compound(data)) = root.get_mut("data") else { panic!("world_border.dat has no data") };
        for (k, v) in [
            ("center_x", Tag::Double(cx)),
            ("center_z", Tag::Double(cz)),
            ("size", Tag::Double(size)),
            ("lerp_target", Tag::Double(size)),
            ("lerp_time", Tag::Long(0)),
        ] {
            data.insert(k.to_owned(), v);
        }
        groundwork_nbt::write_file(&modern, &name, &root, Compression::Gzip).expect("write world_border.dat");
        println!("world border {size} centred on {cx},{cz} written to {}", modern.display());
        return;
    }

    let path = save.join("level.dat");
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

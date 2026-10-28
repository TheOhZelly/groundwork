//! Prints what is in one chunk of a Minecraft save, to check exports.
//!
//! ```text
//! cargo run --release -p groundwork-export --example inspect_chunk -- SAVE_DIR BLOCK_X BLOCK_Z
//! ```

use groundwork_nbt::Tag;
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: inspect_chunk SAVE_DIR BLOCK_X BLOCK_Z");
        std::process::exit(2);
    }
    let (x, z): (i32, i32) = (args[2].parse().unwrap(), args[3].parse().unwrap());
    let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
    let path = std::path::Path::new(&args[1])
        .join("region")
        .join(groundwork_anvil::region_file_name(cx >> 5, cz >> 5));
    let mut region = groundwork_anvil::RegionFile::open_read_only(&path).expect("region file");
    let Some(chunk) = region.read_chunk(cx & 31, cz & 31).expect("chunk") else {
        println!("chunk {cx},{cz} is not in {}", path.display());
        return;
    };
    println!(
        "chunk {cx},{cz}: DataVersion {:?}, Status {:?}",
        chunk["DataVersion"],
        chunk["Status"].as_str()
    );
    let mut blocks: BTreeMap<String, usize> = BTreeMap::new();
    let mut biome_names = std::collections::BTreeSet::new();
    for section in &chunk["sections"].as_list().unwrap().items {
        let section = section.as_compound().unwrap();
        let states = section["block_states"].as_compound().unwrap();
        for entry in &states["palette"].as_list().unwrap().items {
            let entry = entry.as_compound().unwrap();
            let mut name = entry["Name"].as_str().unwrap().to_owned();
            if let Some(Tag::Compound(props)) = entry.get("Properties") {
                let p: Vec<String> = props
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("?")))
                    .collect();
                name = format!("{name}[{}]", p.join(","));
            }
            *blocks.entry(name).or_default() += 1;
        }
        for b in &section["biomes"].as_compound().unwrap()["palette"]
            .as_list()
            .unwrap()
            .items
        {
            biome_names.insert(b.as_str().unwrap().to_owned());
        }
    }
    println!("block types (sections containing each):");
    for (name, sections) in &blocks {
        println!("  {name}  ×{sections}");
    }
    println!(
        "biomes: {}",
        biome_names.into_iter().collect::<Vec<_>>().join(", ")
    );
}

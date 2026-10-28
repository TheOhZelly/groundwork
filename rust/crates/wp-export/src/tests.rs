use super::*;
use wp_core::generate::{generate, GenerateOptions};
use wp_core::{layers, ops, Terrain};
use wp_nbt::Tag;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wp-export-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Decodes the block at (x, y, z) local to a chunk from its NBT.
fn block_at(chunk: &wp_nbt::Compound, x: i32, y: i32, z: i32) -> String {
    let sections = chunk["sections"].as_list().unwrap();
    let section = sections
        .items
        .iter()
        .map(|s| s.as_compound().unwrap())
        .find(|s| s["Y"] == Tag::Byte(y.div_euclid(16) as i8))
        .unwrap();
    let states = section["block_states"].as_compound().unwrap();
    let palette = &states["palette"].as_list().unwrap().items;
    let index = match states.get("data") {
        None => 0,
        Some(Tag::LongArray(data)) => {
            let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(4) as usize;
            let per_long = 64 / bits;
            let i = (y.rem_euclid(16) * 256 + z * 16 + x) as usize;
            ((data[i / per_long] as u64 >> ((i % per_long) * bits)) & ((1 << bits) - 1)) as usize
        }
        other => panic!("unexpected data tag {other:?}"),
    };
    palette[index].as_compound().unwrap()["Name"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn exports_a_loadable_world_layout() {
    let opts = GenerateOptions {
        width_tiles: 2,
        length_tiles: 1,
        hilliness: 0.0,
        base_height: 70.0,
        water_level: 62,
        terrain: Terrain::BareGrass,
        seed: 99,
        ..Default::default()
    };
    let mut dim = generate(&opts).unwrap();
    // A frozen lake at the origin and a sand patch.
    ops::flatten(
        &mut dim,
        ops::Shape::Square {
            x: 8,
            y: 8,
            radius: 4,
        },
        ops::Falloff::Constant,
        Some(55.0),
    );
    ops::flood(&mut dim, 8, 8, 62, false, 10_000).unwrap();
    ops::paint_layer(
        &mut dim,
        ops::Shape::Square {
            x: 8,
            y: 8,
            radius: 6,
        },
        ops::Falloff::Constant,
        layers::FROST,
        1,
    );
    ops::paint_terrain(
        &mut dim,
        ops::Shape::Rect {
            x0: 20,
            y0: 0,
            x1: 20,
            y1: 0,
        },
        ops::Falloff::Constant,
        Terrain::Sand,
    );
    let mut world = World::new("Export test", 99, dim);
    world.spawn = (30, 30);

    let dir = temp_dir("basic");
    let report = export(&world, &dir, &ExportOptions::default()).unwrap();
    assert_eq!(report.chunks, 2 * 64);
    assert_eq!(report.regions, 2); // tiles -1 and 0 fall in regions -1 and 0

    let (_, level) = wp_nbt::read_file(dir.join("level.dat")).unwrap();
    let data = level["Data"].as_compound().unwrap();
    assert_eq!(data["LevelName"].as_str(), Some("Export test"));
    assert_eq!(data["SpawnY"], Tag::Int(71));

    let mut region =
        wp_anvil::RegionFile::open_read_only(dir.join("region").join("r.0.0.mca")).unwrap();
    assert_eq!(region.chunk_count(), 64);
    let chunk = region.read_chunk(0, 0).unwrap().unwrap();
    assert_eq!(chunk["Status"].as_str(), Some("minecraft:full"));
    assert_eq!(chunk["yPos"], Tag::Int(-4));
    assert_eq!(chunk["sections"].as_list().unwrap().items.len(), 24);

    assert_eq!(block_at(&chunk, 0, -64, 0), "minecraft:bedrock");
    assert_eq!(block_at(&chunk, 0, -10, 0), "minecraft:deepslate");
    assert_eq!(block_at(&chunk, 0, 70, 0), "minecraft:grass_block");
    // Like Java, Bare Grass uses grass for its whole top layer.
    assert_eq!(block_at(&chunk, 0, 69, 0), "minecraft:grass_block");
    assert_eq!(block_at(&chunk, 0, 60, 0), "minecraft:stone");
    assert_eq!(block_at(&chunk, 0, 71, 0), "minecraft:air");
    // Lake: dirt bottom (grass converted), water, ice on top, snow on the shore.
    assert_eq!(block_at(&chunk, 8, 55, 8), "minecraft:dirt");
    assert_eq!(block_at(&chunk, 8, 60, 8), "minecraft:water");
    assert_eq!(block_at(&chunk, 8, 62, 8), "minecraft:ice");
    assert_eq!(block_at(&chunk, 14, 71, 14), "minecraft:snow");
    assert_eq!(block_at(&chunk, 15, 71, 15), "minecraft:air");
    // Biome: frozen lake columns get frozen_ocean, the rest plains.
    let biomes = chunk["sections"].as_list().unwrap().items[0]
        .as_compound()
        .unwrap()["biomes"]
        .as_compound()
        .unwrap();
    let names: Vec<_> = biomes["palette"]
        .as_list()
        .unwrap()
        .items
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(names.contains(&"minecraft:plains"), "{names:?}");
    assert!(
        names.contains(&"minecraft:frozen_ocean") || names.contains(&"minecraft:snowy_plains"),
        "{names:?}"
    );

    let sand_chunk = region.read_chunk(1, 0).unwrap().unwrap();
    assert_eq!(block_at(&sand_chunk, 4, 70, 0), "minecraft:sand");

    // Refuses to overwrite without permission.
    assert!(matches!(
        export(&world, &dir, &ExportOptions::default()),
        Err(Error::Refused(_))
    ));
    let again = ExportOptions {
        overwrite: true,
        ..Default::default()
    };
    assert!(export(&world, &dir, &again).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn void_layer_and_warnings() {
    let mut dim = generate(&GenerateOptions {
        width_tiles: 1,
        length_tiles: 1,
        terrain: Terrain::Custom1,
        ..Default::default()
    })
    .unwrap();
    // A 1×1 world is tile (0,0); void out its first chunk.
    ops::paint_layer(
        &mut dim,
        ops::Shape::Rect {
            x0: 0,
            y0: 0,
            x1: 15,
            y1: 15,
        },
        ops::Falloff::Constant,
        layers::VOID,
        1,
    );
    let world = World::new("Void", 1, dim);
    let dir = temp_dir("void");
    let report = export(
        &world,
        &dir,
        &ExportOptions {
            outside: Outside::Void,
            overwrite: false,
        },
    )
    .unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("Custom 1")),
        "{:?}",
        report.warnings
    );
    let mut region =
        wp_anvil::RegionFile::open_read_only(dir.join("region").join("r.0.0.mca")).unwrap();
    let chunk = region.read_chunk(0, 0).unwrap().unwrap();
    assert_eq!(block_at(&chunk, 0, -64, 0), "minecraft:air");
    assert_eq!(block_at(&chunk, 15, 64, 15), "minecraft:air");
    let next = region.read_chunk(1, 0).unwrap().unwrap();
    assert_eq!(block_at(&next, 0, -64, 0), "minecraft:bedrock");
    let (_, level) = wp_nbt::read_file(dir.join("level.dat")).unwrap();
    let gen = &level["Data"].as_compound().unwrap()["WorldGenSettings"]
        .as_compound()
        .unwrap()["dimensions"]
        .as_compound()
        .unwrap()["minecraft:overworld"]
        .as_compound()
        .unwrap()["generator"];
    assert_eq!(
        gen.as_compound().unwrap()["type"].as_str(),
        Some("minecraft:flat")
    );
    std::fs::remove_dir_all(&dir).ok();
}

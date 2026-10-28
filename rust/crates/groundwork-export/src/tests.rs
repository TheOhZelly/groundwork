use super::*;
use groundwork_core::generate::{generate, GenerateOptions};
use groundwork_core::{layers, ops, Terrain};
use groundwork_nbt::Tag;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("groundwork-export-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Decodes the block at (x, y, z) local to a chunk from its NBT.
fn block_at(chunk: &groundwork_nbt::Compound, x: i32, y: i32, z: i32) -> String {
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

    let (_, level) = groundwork_nbt::read_file(dir.join("level.dat")).unwrap();
    let data = level["Data"].as_compound().unwrap();
    assert_eq!(data["LevelName"].as_str(), Some("Export test"));
    assert_eq!(data["SpawnY"], Tag::Int(71));

    let mut region =
        groundwork_anvil::RegionFile::open_read_only(dir.join("region").join("r.0.0.mca")).unwrap();
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
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("Custom 1")),
        "{:?}",
        report.warnings
    );
    let mut region =
        groundwork_anvil::RegionFile::open_read_only(dir.join("region").join("r.0.0.mca")).unwrap();
    let chunk = region.read_chunk(0, 0).unwrap().unwrap();
    assert_eq!(block_at(&chunk, 0, -64, 0), "minecraft:air");
    assert_eq!(block_at(&chunk, 15, 64, 15), "minecraft:air");
    let next = region.read_chunk(1, 0).unwrap().unwrap();
    assert_eq!(block_at(&next, 0, -64, 0), "minecraft:bedrock");
    let (_, level) = groundwork_nbt::read_file(dir.join("level.dat")).unwrap();
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

#[test]
fn forests_and_1_21_11_format() {
    let opts = GenerateOptions {
        width_tiles: 1,
        length_tiles: 1,
        hilliness: 0.0,
        base_height: 70.0,
        terrain: Terrain::Grass,
        seed: 5,
        ..Default::default()
    };
    let mut dim = generate(&opts).unwrap();
    let all = ops::Shape::Rect {
        x0: 0,
        y0: 0,
        x1: 127,
        y1: 127,
    };
    ops::paint_layer(&mut dim, all, ops::Falloff::Constant, layers::DECIDUOUS, 15);
    let world = World::new("Forest", 5, dim);
    let dir = temp_dir("forest");
    export(&world, &dir, &ExportOptions::default()).unwrap();

    let (_, level) = groundwork_nbt::read_file(dir.join("level.dat")).unwrap();
    let data = level["Data"].as_compound().unwrap();
    assert_eq!(data["DataVersion"], Tag::Int(4671));
    assert_eq!(
        data["Version"].as_compound().unwrap()["Name"].as_str(),
        Some("1.21.11")
    );

    let mut region =
        groundwork_anvil::RegionFile::open_read_only(dir.join("region").join("r.0.0.mca")).unwrap();
    let (mut logs, mut leaves) = (0, 0);
    for cz in 0..8 {
        for cx in 0..8 {
            let chunk = region.read_chunk(cx, cz).unwrap().unwrap();
            assert_eq!(chunk["DataVersion"], Tag::Int(4671));
            for section in &chunk["sections"].as_list().unwrap().items {
                let palette = &section.as_compound().unwrap()["block_states"]
                    .as_compound()
                    .unwrap()["palette"];
                for entry in &palette.as_list().unwrap().items {
                    let entry = entry.as_compound().unwrap();
                    let name = entry["Name"].as_str().unwrap();
                    if name.ends_with("_log") {
                        logs += 1;
                    }
                    if name.ends_with("_leaves") {
                        leaves += 1;
                        let props = entry["Properties"].as_compound().unwrap();
                        assert_eq!(props["persistent"].as_str(), Some("true"));
                    }
                }
            }
        }
    }
    // Every chunk of a dense forest should have trees in it.
    assert!(logs >= 64 && leaves >= 64, "logs {logs}, leaves {leaves}");
    // A trunk sits directly on the grass.
    let chunk = region.read_chunk(3, 3).unwrap().unwrap();
    let column_has_log =
        (0..16).any(|x| (0..16).any(|z| block_at(&chunk, x, 71, z).ends_with("_log")));
    assert!(column_has_log);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn endless_ocean_outside() {
    let dim = generate(&GenerateOptions {
        width_tiles: 1,
        length_tiles: 1,
        ..Default::default()
    })
    .unwrap();
    let world = World::new("Sea", 1, dim);
    let dir = temp_dir("ocean");
    export(
        &world,
        &dir,
        &ExportOptions {
            outside: Outside::Ocean,
            version: McVersion::V1_21_1,
            ..Default::default()
        },
    )
    .unwrap();
    let (_, level) = groundwork_nbt::read_file(dir.join("level.dat")).unwrap();
    let data = level["Data"].as_compound().unwrap();
    assert_eq!(data["DataVersion"], Tag::Int(3955));
    let generator = data["WorldGenSettings"].as_compound().unwrap()["dimensions"]
        .as_compound()
        .unwrap()["minecraft:overworld"]
        .as_compound()
        .unwrap()["generator"]
        .as_compound()
        .unwrap();
    let settings = generator["settings"].as_compound().unwrap();
    assert_eq!(settings["biome"].as_str(), Some("minecraft:ocean"));
    // Layers stack from y=-64; the water must top out at sea level, y=62.
    let total: i64 = settings["layers"]
        .as_list()
        .unwrap()
        .items
        .iter()
        .map(|l| l.as_compound().unwrap()["height"].as_i64().unwrap())
        .sum();
    assert_eq!(-64 + total - 1, 62);
    std::fs::remove_dir_all(&dir).ok();
}

/// Counts blocks by name over the whole of one chunk.
fn count_blocks(chunk: &groundwork_nbt::Compound, pred: impl Fn(&str) -> bool) -> usize {
    let mut n = 0;
    for y in -64..320 {
        for z in 0..16 {
            for x in 0..16 {
                if pred(&block_at(chunk, x, y, z)) {
                    n += 1;
                }
            }
        }
    }
    n
}

#[test]
fn ores_and_caverns() {
    let flat = GenerateOptions {
        width_tiles: 1,
        length_tiles: 1,
        hilliness: 0.0,
        base_height: 100.0,
        water_level: -64,
        terrain: Terrain::Stone,
        seed: 11,
        ..Default::default()
    };
    // Default: Resources level 8 everywhere, no caverns.
    let world = World::new("Ores", 11, generate(&flat).unwrap());
    let dir = temp_dir("ores");
    export(&world, &dir, &ExportOptions::default()).unwrap();
    let mut region =
        groundwork_anvil::RegionFile::open_read_only(dir.join("region").join("r.0.0.mca")).unwrap();
    let (mut ores, mut deep_ores, mut dirt, mut rock) = (0, 0, 0, 0);
    for (cx, cz) in [(1, 1), (4, 2), (6, 6)] {
        let chunk = region.read_chunk(cx, cz).unwrap().unwrap();
        ores += count_blocks(&chunk, |b| b.ends_with("_ore") && !b.contains("deepslate"));
        deep_ores += count_blocks(&chunk, |b| {
            b.starts_with("minecraft:deepslate_") && b.ends_with("_ore")
        });
        dirt += count_blocks(&chunk, |b| b == "minecraft:dirt");
        rock += count_blocks(&chunk, |b| {
            b == "minecraft:stone" || b == "minecraft:deepslate"
        });
    }
    let solid = (ores + deep_ores + dirt + rock) as f32;
    // WorldPainter's table gives ~3% ores and ~5.7% dirt in the rock.
    assert!(
        ores > 0 && deep_ores > 0,
        "ores {ores}, deepslate ores {deep_ores}"
    );
    let (ore_share, dirt_share) = ((ores + deep_ores) as f32 / solid, dirt as f32 / solid);
    assert!((0.005..0.06).contains(&ore_share), "ore share {ore_share}");
    assert!((0.02..0.1).contains(&dirt_share), "dirt share {dirt_share}");
    std::fs::remove_dir_all(&dir).ok();

    // Caverns at full strength hollow out a good part of the rock, but never
    // break the surface or the bedrock floor.
    let mut dim = generate(&flat).unwrap();
    dim.resources_level = 0;
    dim.caverns_level = 15;
    let world = World::new("Caves", 11, dim);
    let dir = temp_dir("caves");
    export(&world, &dir, &ExportOptions::default()).unwrap();
    let mut region =
        groundwork_anvil::RegionFile::open_read_only(dir.join("region").join("r.0.0.mca")).unwrap();
    let chunk = region.read_chunk(3, 3).unwrap().unwrap();
    let hollow = (-60..90)
        .flat_map(|y| (0..16).map(move |x| (x, y)))
        .filter(|&(x, y)| block_at(&chunk, x, y, 5) == "minecraft:air")
        .count();
    assert!(hollow > 200, "only {hollow} hollow blocks");
    for x in 0..16 {
        for z in 0..16 {
            assert_eq!(block_at(&chunk, x, -64, z), "minecraft:bedrock");
            assert_eq!(block_at(&chunk, x, 100, z), "minecraft:stone");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

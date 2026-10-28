//! `level.dat` for Minecraft 1.21. Equivalent of `Java118Level` for the
//! settings the Rust model has so far.

use crate::{Error, Outside, DATA_VERSION, VERSION_NAME};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use wp_core::{GameMode, World};
use wp_nbt::{compound, Compound, List, Tag, TAG_COMPOUND, TAG_STRING};

/// NBT format version of Anvil level.dat files.
const ANVIL_VERSION: i32 = 19133;

pub fn write_level_dat(
    world: &World,
    dir: &Path,
    spawn_y: i32,
    outside: Outside,
) -> Result<(), Error> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let (game_type, hardcore) = match world.game_mode {
        GameMode::Survival => (0i32, false),
        GameMode::Creative => (1, false),
        GameMode::Adventure => (2, false),
        GameMode::Spectator => (3, false),
        GameMode::Hardcore => (0, true),
    };
    let seed = world.seed as i64;

    let overworld_generator = match outside {
        Outside::Vanilla => {
            noise_generator("minecraft:overworld", multi_noise("minecraft:overworld"))
        }
        // The "The Void" superflat preset.
        Outside::Void => compound! {
            "type" => "minecraft:flat",
            "settings" => compound! {
                "biome" => "minecraft:the_void",
                "features" => false,
                "lakes" => false,
                "layers" => List::compounds(vec![compound! { "block" => "minecraft:air", "height" => 1i32 }]),
                "structure_overrides" => List::empty(TAG_STRING),
            },
        },
    };

    let data = compound! {
        "DataVersion" => DATA_VERSION,
        "version" => ANVIL_VERSION,
        "Version" => compound! {
            "Id" => DATA_VERSION,
            "Name" => VERSION_NAME,
            "Series" => "main",
            "Snapshot" => false,
        },
        "LevelName" => world.name.as_str(),
        "GameType" => game_type,
        "hardcore" => hardcore,
        "allowCommands" => world.allow_cheats,
        "Difficulty" => if hardcore { 3i8 } else { 2i8 },
        "initialized" => true,
        "LastPlayed" => now,
        "Time" => 0i64,
        "DayTime" => 0i64,
        "SpawnX" => world.spawn.0,
        "SpawnY" => spawn_y,
        "SpawnZ" => world.spawn.1,
        "SpawnAngle" => 0f32,
        "raining" => false,
        "rainTime" => 0i32,
        "thundering" => false,
        "thunderTime" => 0i32,
        "clearWeatherTime" => 0i32,
        "WasModded" => false,
        "ServerBrands" => List::strings(["WorldPainter (Rust)"]),
        "DataPacks" => compound! {
            "Enabled" => List::strings(["vanilla"]),
            "Disabled" => List::empty(TAG_STRING),
        },
        "GameRules" => compound! {},
        "WorldGenSettings" => compound! {
            "seed" => seed,
            "generate_features" => world.map_features,
            "bonus_chest" => false,
            "dimensions" => compound! {
                "minecraft:overworld" => compound! {
                    "type" => "minecraft:overworld",
                    "generator" => overworld_generator,
                },
                "minecraft:the_nether" => compound! {
                    "type" => "minecraft:the_nether",
                    "generator" => noise_generator("minecraft:nether", multi_noise("minecraft:nether")),
                },
                "minecraft:the_end" => compound! {
                    "type" => "minecraft:the_end",
                    "generator" => noise_generator("minecraft:end", compound! { "type" => "minecraft:the_end" }),
                },
            },
        },
        "ScheduledEvents" => List::empty(TAG_COMPOUND),
    };
    let root = compound! { "Data" => Tag::Compound(data) };
    wp_nbt::write_file(dir.join("level.dat"), "", &root, wp_nbt::Compression::Gzip)?;
    Ok(())
}

fn noise_generator(settings: &str, biome_source: Compound) -> Compound {
    compound! { "type" => "minecraft:noise", "settings" => settings, "biome_source" => biome_source }
}

fn multi_noise(preset: &str) -> Compound {
    compound! { "type" => "minecraft:multi_noise", "preset" => preset }
}

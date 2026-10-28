//! Saving and loading worlds.
//!
//! WorldPainter stores `.world` files as gzipped Java object serialization,
//! which cannot reasonably be read outside the JVM. Groundwork uses its own
//! format: a magic header, a format version, then a gzipped bincode encoding
//! of [`World`]. The conventional extension is `.gwork`. Importing
//! WorldPainter `.world` files is a separate, planned piece of work.

use crate::world::World;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"GWRK";
/// 1: initial format. 2: dimensions gained resources and caverns levels, and
/// tiles store uniform data and heights compactly.
const FORMAT_VERSION: u32 = 2;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Encoding(bincode::Error),
    NotAWorld,
    UnsupportedVersion(u32),
    JavaWorld,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Encoding(e) => write!(f, "corrupt world file: {e}"),
            Error::NotAWorld => write!(f, "not a Groundwork world file"),
            Error::UnsupportedVersion(v) => write!(
                f,
                "world file format version {v} is newer than this build supports"
            ),
            Error::JavaWorld => write!(
                f,
                "this is a WorldPainter .world file; importing those is not supported yet"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<bincode::Error> for Error {
    fn from(e: bincode::Error) -> Self {
        Error::Encoding(e)
    }
}

/// Saves atomically: writes a temporary file next to `path`, then renames it.
pub fn save(world: &World, path: impl AsRef<Path>) -> Result<(), Error> {
    let path = path.as_ref();
    let tmp = path.with_extension("gwork.tmp");
    {
        let mut out = BufWriter::new(File::create(&tmp)?);
        out.write_all(MAGIC)?;
        out.write_all(&FORMAT_VERSION.to_be_bytes())?;
        // bincode writes value by value; buffer in front of the compressor so
        // it gets large blocks instead of millions of tiny writes.
        let mut gz = BufWriter::with_capacity(1 << 20, GzEncoder::new(out, Compression::fast()));
        bincode::serialize_into(&mut gz, world)?;
        gz.into_inner()
            .map_err(|e| e.into_error())?
            .finish()?
            .flush()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(path: impl AsRef<Path>) -> Result<World, Error> {
    let mut input = BufReader::new(File::open(path)?);
    let mut header = [0u8; 8];
    input
        .read_exact(&mut header)
        .map_err(|_| Error::NotAWorld)?;
    if &header[..4] != MAGIC {
        // Java .world files are gzip streams.
        return Err(if header[..2] == [0x1f, 0x8b] {
            Error::JavaWorld
        } else {
            Error::NotAWorld
        });
    }
    let version = u32::from_be_bytes(header[4..].try_into().unwrap());
    if version > FORMAT_VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    let data = BufReader::with_capacity(1 << 20, GzDecoder::new(input));
    Ok(match version {
        1 => bincode::deserialize_from::<_, v1::World>(data)?.into(),
        _ => bincode::deserialize_from(data)?,
    })
}

/// The version 1 layout, kept so older files still open.
mod v1 {
    use crate::dimension::DimensionKind;
    use crate::tile::{LegacyLayer, Tile as CurrentTile};
    use crate::world::GameMode;
    use serde::Deserialize;
    use std::collections::{BTreeMap, HashMap};

    #[derive(Deserialize)]
    struct Tile {
        x: i32,
        y: i32,
        min_height: i32,
        max_height: i32,
        terrain: Vec<u8>,
        height: Vec<i32>,
        water: Vec<i32>,
        layers: BTreeMap<String, LayerBuffer>,
    }

    #[derive(Deserialize)]
    enum LayerBuffer {
        Bit(Vec<u64>),
        Nibble(Vec<u8>),
        Byte(Vec<u8>),
    }

    impl From<Tile> for CurrentTile {
        fn from(t: Tile) -> Self {
            let layers = t
                .layers
                .into_iter()
                .map(|(id, b)| {
                    let layer = match b {
                        LayerBuffer::Bit(bits) => LegacyLayer::Bit(bits),
                        LayerBuffer::Nibble(values) => LegacyLayer::Values {
                            nibble: true,
                            values,
                        },
                        LayerBuffer::Byte(values) => LegacyLayer::Values {
                            nibble: false,
                            values,
                        },
                    };
                    (id, layer)
                })
                .collect();
            CurrentTile::from_v1(
                (t.x, t.y, t.min_height, t.max_height),
                t.terrain,
                t.height,
                t.water,
                layers,
            )
        }
    }

    #[derive(Deserialize)]
    pub struct World {
        name: String,
        seed: u64,
        spawn: (i32, i32),
        game_mode: GameMode,
        allow_cheats: bool,
        map_features: bool,
        dimensions: Vec<Dimension>,
    }

    #[derive(Deserialize)]
    struct Dimension {
        kind: DimensionKind,
        min_height: i32,
        max_height: i32,
        seed: u64,
        subsurface_block: String,
        top_layer_depth: i32,
        bedrock: bool,
        tiles: HashMap<(i32, i32), Tile>,
    }

    impl From<World> for crate::world::World {
        fn from(w: World) -> Self {
            let dimensions = w
                .dimensions
                .into_iter()
                .map(|d| {
                    crate::dimension::Dimension::from_v1(
                        d.kind,
                        d.min_height,
                        d.max_height,
                        d.seed,
                        d.subsurface_block,
                        d.top_layer_depth,
                        d.bedrock,
                        d.tiles.into_iter().map(|(k, t)| (k, t.into())).collect(),
                    )
                })
                .collect();
            crate::world::World::from_parts(
                w.name,
                w.seed,
                w.spawn,
                w.game_mode,
                w.allow_cheats,
                w.map_features,
                dimensions,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::{generate, GenerateOptions};
    use crate::layers;

    #[test]
    fn round_trip() {
        let mut dim = generate(&GenerateOptions {
            width_tiles: 1,
            length_tiles: 1,
            seed: 3,
            ..Default::default()
        })
        .unwrap();
        dim.set_layer_value_at(layers::FROST, 5, 5, 1);
        let mut world = World::new("Test", 3, dim);
        world.spawn = (10, -4);
        let path =
            std::env::temp_dir().join(format!("groundwork-core-io-{}.gwork", std::process::id()));
        save(&world, &path).unwrap();
        let back = load(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back.name, "Test");
        assert_eq!(back.spawn, (10, -4));
        let (a, b) = (world.dimensions()[0].clone(), &back.dimensions()[0]);
        assert_eq!(a.height_at(17, 99), b.height_at(17, 99));
        assert_eq!(b.layer_value_at(layers::FROST, 5, 5), Some(1));
    }

    #[test]
    fn recognises_java_worlds() {
        let path =
            std::env::temp_dir().join(format!("groundwork-core-java-{}.world", std::process::id()));
        std::fs::write(&path, [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert!(matches!(load(&path), Err(Error::JavaWorld)));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn opens_version_1_files() {
        use crate::dimension::DimensionKind;
        use crate::world::GameMode;
        use std::collections::{BTreeMap, HashMap};

        // The version 1 layout, field for field.
        #[derive(serde::Serialize)]
        struct V1World {
            name: String,
            seed: u64,
            spawn: (i32, i32),
            game_mode: GameMode,
            allow_cheats: bool,
            map_features: bool,
            dimensions: Vec<V1Dimension>,
        }
        #[derive(serde::Serialize)]
        enum V1Layer {
            #[allow(dead_code)]
            Bit(Vec<u64>),
            Nibble(Vec<u8>),
            Byte(Vec<u8>),
        }
        #[derive(serde::Serialize)]
        struct V1Tile {
            x: i32,
            y: i32,
            min_height: i32,
            max_height: i32,
            terrain: Vec<u8>,
            height: Vec<i32>,
            water: Vec<i32>,
            layers: BTreeMap<String, V1Layer>,
        }
        #[derive(serde::Serialize)]
        struct V1Dimension {
            kind: DimensionKind,
            min_height: i32,
            max_height: i32,
            seed: u64,
            subsurface_block: String,
            top_layer_depth: i32,
            bedrock: bool,
            tiles: HashMap<(i32, i32), V1Tile>,
        }
        let mut height = vec![0; 16384];
        height[3 | (4 << 7)] = (77 + 64) * 256;
        let tile = V1Tile {
            x: 0,
            y: 0,
            min_height: -64,
            max_height: 320,
            terrain: vec![2; 16384],
            height,
            water: vec![62 + 64; 16384],
            layers: BTreeMap::from([
                ("Biome".to_owned(), V1Layer::Byte(vec![1; 16384])),
                ("Deciduous".to_owned(), V1Layer::Nibble(vec![9; 16384])),
            ]),
        };
        let old = V1World {
            name: "Old".into(),
            seed: 9,
            spawn: (1, 2),
            game_mode: GameMode::Creative,
            allow_cheats: true,
            map_features: false,
            dimensions: vec![V1Dimension {
                kind: DimensionKind::Overworld,
                min_height: -64,
                max_height: 320,
                seed: 9,
                subsurface_block: "minecraft:andesite".into(),
                top_layer_depth: 5,
                bedrock: false,
                tiles: HashMap::from([((0, 0), tile)]),
            }],
        };
        let path = std::env::temp_dir().join(format!("groundwork-v1-{}.gwork", std::process::id()));
        {
            let mut out = File::create(&path).unwrap();
            out.write_all(MAGIC).unwrap();
            out.write_all(&1u32.to_be_bytes()).unwrap();
            let mut gz = GzEncoder::new(out, Compression::fast());
            bincode::serialize_into(&mut gz, &old).unwrap();
            gz.finish().unwrap();
        }
        let world = load(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(
            (world.name.as_str(), world.spawn, world.allow_cheats),
            ("Old", (1, 2), true)
        );
        let dim = &world.dimensions()[0];
        assert_eq!(dim.subsurface_block, "minecraft:andesite");
        assert_eq!((dim.top_layer_depth, dim.bedrock), (5, false));
        assert_eq!(dim.height_at(3, 4), Some(77.0));
        assert_eq!(dim.water_level_at(100, 100), Some(62));
        assert_eq!(dim.terrain_at(9, 9), Some(crate::Terrain::Sand));
        assert_eq!(dim.layer_value_at(crate::layers::BIOME, 7, 7), Some(1));
        assert_eq!(dim.layer_value_at(crate::layers::DECIDUOUS, 7, 7), Some(9));
        // Migrated tiles come out compacted: only narrow heights remain.
        assert_eq!(
            dim.tile(0, 0).unwrap().heap_bytes(),
            16384 * 2 + "Biome".len() + "Deciduous".len()
        );
        // New settings take WorldPainter's defaults.
        assert_eq!((dim.resources_level, dim.caverns_level), (8, 0));
    }
}

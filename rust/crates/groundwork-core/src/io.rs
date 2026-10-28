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
const FORMAT_VERSION: u32 = 1;

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
        let mut gz = GzEncoder::new(out, Compression::fast());
        bincode::serialize_into(&mut gz, world)?;
        gz.finish()?.flush()?;
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
    Ok(bincode::deserialize_from(GzDecoder::new(input))?)
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
}

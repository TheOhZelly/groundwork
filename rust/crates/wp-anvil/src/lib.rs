//! Minecraft Anvil region files.
//!
//! Port of `org.pepsoft.minecraft.RegionFile`. A region file stores 32×32
//! chunks in 4 KiB sectors. It starts with two header sectors: the chunk
//! offset table (`sector << 8 | sector_count` per chunk) and the chunk
//! timestamp table. Each chunk payload is a 4-byte big-endian length, a
//! compression byte and the compressed NBT.

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SECTOR_BYTES: usize = 4096;
const SECTOR_INTS: usize = SECTOR_BYTES / 4;
const CHUNK_HEADER_SIZE: usize = 5;
/// The sector count is a single byte, so a chunk can use at most 255 sectors.
const MAX_SECTORS_PER_CHUNK: usize = 255;

const VERSION_GZIP: u8 = 1;
const VERSION_DEFLATE: u8 = 2;
const VERSION_UNCOMPRESSED: u8 = 3;
/// Flag set on the version byte when the chunk is stored in a separate
/// `c.<x>.<z>.mcc` file because it was too large for the region.
const EXTERNAL_FLAG: u8 = 0x80;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Nbt(wp_nbt::Error),
    InvalidRegion(String),
    OutOfBounds { x: i32, z: i32 },
    ChunkTooLarge(usize),
    ReadOnly,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Nbt(e) => write!(f, "{e}"),
            Error::InvalidRegion(m) => write!(f, "invalid region file: {m}"),
            Error::OutOfBounds { x, z } => write!(f, "chunk coordinates {x},{z} outside 0..32"),
            Error::ChunkTooLarge(n) => {
                write!(f, "chunk of {n} bytes exceeds the 1 MiB region limit")
            }
            Error::ReadOnly => write!(f, "region file opened read-only"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<wp_nbt::Error> for Error {
    fn from(e: wp_nbt::Error) -> Self {
        Error::Nbt(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct RegionFile {
    path: PathBuf,
    file: File,
    offsets: [u32; SECTOR_INTS],
    timestamps: [u32; SECTOR_INTS],
    sector_free: Vec<bool>,
    read_only: bool,
    region_x: i32,
    region_z: i32,
}

/// Region file name for region coordinates, e.g. `r.-1.2.mca`.
pub fn region_file_name(region_x: i32, region_z: i32) -> String {
    format!("r.{region_x}.{region_z}.mca")
}

impl RegionFile {
    /// Opens (creating if necessary) a region file for reading and writing.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path.as_ref(), false)
    }

    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path.as_ref(), true)
    }

    fn open_with(path: &Path, read_only: bool) -> Result<Self> {
        let (region_x, region_z) = parse_region_name(path)?;
        let mut file = if read_only {
            File::open(path)?
        } else {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?
        };

        let mut len = file.metadata()?.len() as usize;
        if !read_only {
            if len < SECTOR_BYTES * 2 {
                // Write the (empty) offset and timestamp tables.
                file.set_len((SECTOR_BYTES * 2) as u64)?;
                len = SECTOR_BYTES * 2;
            }
            if !len.is_multiple_of(SECTOR_BYTES) {
                // Pad to a whole number of sectors. (The Java original wrote
                // `len & 0xfff` bytes here, which is the wrong amount.)
                len = len.div_ceil(SECTOR_BYTES) * SECTOR_BYTES;
                file.set_len(len as u64)?;
            }
        }

        let sector_count = len.div_ceil(SECTOR_BYTES);
        let mut sector_free = vec![true; sector_count];
        let mut offsets = [0u32; SECTOR_INTS];
        let mut timestamps = [0u32; SECTOR_INTS];

        let mut header = vec![0u8; (SECTOR_BYTES * 2).min(len)];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header)?;
        if header.len() >= SECTOR_BYTES {
            sector_free[0] = false;
            for (i, offset) in offsets.iter_mut().enumerate() {
                *offset = u32::from_be_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
                let (start, count) = ((*offset >> 8) as usize, (*offset & 0xff) as usize);
                if *offset != 0 && start + count <= sector_count {
                    sector_free[start..start + count].fill(false);
                }
            }
        }
        if header.len() >= SECTOR_BYTES * 2 {
            sector_free[1] = false;
            for (i, ts) in timestamps.iter_mut().enumerate() {
                let p = SECTOR_BYTES + i * 4;
                *ts = u32::from_be_bytes(header[p..p + 4].try_into().unwrap());
            }
        }

        Ok(RegionFile {
            path: path.to_owned(),
            file,
            offsets,
            timestamps,
            sector_free,
            read_only,
            region_x,
            region_z,
        })
    }

    pub fn region_x(&self) -> i32 {
        self.region_x
    }

    pub fn region_z(&self) -> i32 {
        self.region_z
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn contains_chunk(&self, x: i32, z: i32) -> bool {
        index(x, z).is_ok_and(|i| self.offsets[i] != 0)
    }

    pub fn chunk_count(&self) -> usize {
        self.offsets.iter().filter(|&&o| o != 0).count()
    }

    /// Last-modified timestamp (seconds since the epoch) of a chunk.
    pub fn timestamp(&self, x: i32, z: i32) -> Result<u32> {
        Ok(self.timestamps[index(x, z)?])
    }

    /// Reads and decompresses a chunk's NBT bytes. `x` and `z` are local to the
    /// region (0..32). Returns `None` if the chunk is not present.
    pub fn read_chunk_bytes(&mut self, x: i32, z: i32) -> Result<Option<Vec<u8>>> {
        let offset = self.offsets[index(x, z)?];
        if offset == 0 {
            return Ok(None);
        }
        let (sector, count) = ((offset >> 8) as usize, (offset & 0xff) as usize);
        if sector + count > self.sector_free.len() {
            return Err(Error::InvalidRegion(format!(
                "chunk {x},{z} in region {},{} points past end of file",
                self.region_x, self.region_z
            )));
        }

        self.file
            .seek(SeekFrom::Start((sector * SECTOR_BYTES) as u64))?;
        let mut header = [0u8; CHUNK_HEADER_SIZE];
        self.file.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        if length == 0 || length > SECTOR_BYTES * count {
            return Err(Error::InvalidRegion(format!(
                "chunk {x},{z} has invalid length {length} for {count} sectors"
            )));
        }
        let version = header[4];

        let compressed = if version & EXTERNAL_FLAG != 0 {
            let chunk_x = self.region_x * 32 + x;
            let chunk_z = self.region_z * 32 + z;
            let external = self
                .path
                .with_file_name(format!("c.{chunk_x}.{chunk_z}.mcc"));
            std::fs::read(external)?
        } else {
            let mut data = vec![0u8; length - 1];
            self.file.read_exact(&mut data)?;
            data
        };

        let mut out = Vec::new();
        match version & !EXTERNAL_FLAG {
            VERSION_GZIP => {
                GzDecoder::new(&compressed[..]).read_to_end(&mut out)?;
            }
            VERSION_DEFLATE => {
                ZlibDecoder::new(&compressed[..]).read_to_end(&mut out)?;
            }
            VERSION_UNCOMPRESSED => out = compressed,
            other => {
                return Err(Error::InvalidRegion(format!(
                    "chunk {x},{z} has unsupported compression {other}"
                )));
            }
        }
        Ok(Some(out))
    }

    /// Reads a chunk and parses it as an NBT root compound.
    pub fn read_chunk(&mut self, x: i32, z: i32) -> Result<Option<wp_nbt::Compound>> {
        match self.read_chunk_bytes(x, z)? {
            Some(bytes) => Ok(Some(wp_nbt::from_bytes(&bytes)?.1)),
            None => Ok(None),
        }
    }

    /// Compresses (zlib) and stores a chunk.
    pub fn write_chunk_bytes(&mut self, x: i32, z: i32, nbt: &[u8]) -> Result<()> {
        let mut encoder = ZlibEncoder::new(Vec::with_capacity(8192), Compression::default());
        encoder.write_all(nbt)?;
        let data = encoder.finish()?;
        self.write_compressed(x, z, &data)
    }

    pub fn write_chunk(&mut self, x: i32, z: i32, root: &wp_nbt::Compound) -> Result<()> {
        let bytes = wp_nbt::to_bytes("", root, wp_nbt::Compression::None)?;
        self.write_chunk_bytes(x, z, &bytes)
    }

    fn write_compressed(&mut self, x: i32, z: i32, data: &[u8]) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let i = index(x, z)?;
        let offset = self.offsets[i];
        let (mut sector, allocated) = ((offset >> 8) as usize, (offset & 0xff) as usize);
        let needed = (data.len() + CHUNK_HEADER_SIZE).div_ceil(SECTOR_BYTES);
        if needed > MAX_SECTORS_PER_CHUNK {
            return Err(Error::ChunkTooLarge(data.len()));
        }

        if sector != 0 && allocated == needed {
            // Overwrite in place.
        } else {
            self.sector_free[sector..sector + allocated].fill(true);
            match self.find_free_run(needed) {
                Some(start) => sector = start,
                None => {
                    // Grow the file.
                    sector = self.sector_free.len();
                    self.sector_free.resize(sector + needed, true);
                    self.file
                        .set_len((self.sector_free.len() * SECTOR_BYTES) as u64)?;
                }
            }
            self.sector_free[sector..sector + needed].fill(false);
        }

        let mut buf = Vec::with_capacity(needed * SECTOR_BYTES);
        buf.extend_from_slice(&((data.len() + 1) as u32).to_be_bytes());
        buf.push(VERSION_DEFLATE);
        buf.extend_from_slice(data);
        buf.resize(needed * SECTOR_BYTES, 0);
        self.file
            .seek(SeekFrom::Start((sector * SECTOR_BYTES) as u64))?;
        self.file.write_all(&buf)?;

        self.set_offset(i, ((sector as u32) << 8) | needed as u32)?;
        self.set_timestamp(i, now())?;
        Ok(())
    }

    pub fn delete_chunk(&mut self, x: i32, z: i32) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let i = index(x, z)?;
        let offset = self.offsets[i];
        let (sector, allocated) = ((offset >> 8) as usize, (offset & 0xff) as usize);
        if offset != 0 && sector + allocated <= self.sector_free.len() {
            self.sector_free[sector..sector + allocated].fill(true);
        }
        self.set_offset(i, 0)?;
        self.set_timestamp(i, now())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }

    fn find_free_run(&self, needed: usize) -> Option<usize> {
        let mut run_start = 0;
        let mut run_len = 0;
        for (i, &free) in self.sector_free.iter().enumerate() {
            if free {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
                if run_len >= needed {
                    return Some(run_start);
                }
            } else {
                run_len = 0;
            }
        }
        None
    }

    fn set_offset(&mut self, i: usize, value: u32) -> Result<()> {
        self.offsets[i] = value;
        self.file.seek(SeekFrom::Start((i * 4) as u64))?;
        self.file.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn set_timestamp(&mut self, i: usize, value: u32) -> Result<()> {
        self.timestamps[i] = value;
        self.file
            .seek(SeekFrom::Start((SECTOR_BYTES + i * 4) as u64))?;
        self.file.write_all(&value.to_be_bytes())?;
        Ok(())
    }
}

fn index(x: i32, z: i32) -> Result<usize> {
    if (0..32).contains(&x) && (0..32).contains(&z) {
        Ok((x + z * 32) as usize)
    } else {
        Err(Error::OutOfBounds { x, z })
    }
}

fn now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
}

fn parse_region_name(path: &Path) -> Result<(i32, i32)> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let parts: Vec<&str> = name.split('.').collect();
    match parts.as_slice() {
        [_, x, z, ..] => match (x.parse(), z.parse()) {
            (Ok(x), Ok(z)) => Ok((x, z)),
            _ => Err(Error::InvalidRegion(format!(
                "cannot parse region coordinates from {name:?}"
            ))),
        },
        _ => Err(Error::InvalidRegion(format!(
            "cannot parse region coordinates from {name:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_nbt::compound;

    fn temp_region(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wp-anvil-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.-1.2.mca");
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn writes_reads_and_reuses_sectors() {
        let path = temp_region("basic");
        {
            let mut region = RegionFile::open(&path).unwrap();
            assert_eq!((region.region_x(), region.region_z()), (-1, 2));
            assert_eq!(region.chunk_count(), 0);
            region
                .write_chunk(3, 4, &compound! { "xPos" => 7i32 })
                .unwrap();
            // Incompressible payload spanning several sectors.
            let big: Vec<i64> = (0..3000i64)
                .map(|i| i.wrapping_mul(0x9E3779B97F4A7C15u64 as i64))
                .collect();
            region
                .write_chunk(31, 31, &compound! { "data" => big.clone() })
                .unwrap();
            region.delete_chunk(3, 4).unwrap();
            region
                .write_chunk(0, 0, &compound! { "small" => 1i8 })
                .unwrap();
        }
        let mut region = RegionFile::open_read_only(&path).unwrap();
        assert_eq!(region.chunk_count(), 2);
        assert!(!region.contains_chunk(3, 4));
        assert_eq!(
            region.read_chunk(0, 0).unwrap().unwrap()["small"],
            wp_nbt::Tag::Byte(1)
        );
        let big = region.read_chunk(31, 31).unwrap().unwrap();
        assert!(matches!(&big["data"], wp_nbt::Tag::LongArray(v) if v.len() == 3000));
        assert_eq!(
            std::fs::metadata(&path).unwrap().len() % SECTOR_BYTES as u64,
            0
        );
        // The small chunk should have reused the freed sector 2.
        assert_eq!(region.offsets[0] >> 8, 2);
    }

    #[test]
    fn rejects_out_of_bounds() {
        let mut region = RegionFile::open(temp_region("bounds")).unwrap();
        assert!(matches!(
            region.read_chunk(32, 0),
            Err(Error::OutOfBounds { .. })
        ));
    }
}

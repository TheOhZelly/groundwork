//! Export to Minecraft Java Edition 1.21 (Anvil).
//!
//! Covers the core of `org.pepsoft.worldpainter.exporting` for the
//! `JAVA_ANVIL_1_20_5` and `JAVA_ANVIL_1_21_11` platforms, which share one
//! chunk and level.dat format and differ only in data version: terrain
//! columns (top layer, subsurface, bedrock), water and lava, the Frost, Void,
//! Biome and tree layers, `level.dat` and the region files. Chunks are
//! written with `isLightOn = 0` and no height maps so that Minecraft computes
//! lighting and height maps itself on first load.

mod chunk;
mod level;
mod materials;
mod trees;

use groundwork_core::{Dimension, DimensionKind, World, TILE_SIZE};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Minecraft release to write. Minecraft upgrades worlds from older data
/// versions on load, but refuses (or warns about) worlds from newer ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McVersion {
    /// 1.21.1, readable by every 1.21.x release.
    V1_21_1,
    /// 1.21.11, WorldPainter's `JAVA_ANVIL_1_21_11` platform.
    #[default]
    V1_21_11,
}

impl McVersion {
    pub fn data_version(self) -> i32 {
        match self {
            McVersion::V1_21_1 => 3955,
            McVersion::V1_21_11 => 4671,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            McVersion::V1_21_1 => "1.21.1",
            McVersion::V1_21_11 => "1.21.11",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "1.21.1" => Some(McVersion::V1_21_1),
            "1.21.11" => Some(McVersion::V1_21_11),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outside {
    /// Vanilla terrain generates beyond the painted area.
    Vanilla,
    /// Nothing beyond the painted area (overworld only).
    Void,
    /// Endless flat ocean beyond the painted area, with the surface at
    /// y=62 like vanilla seas (overworld only).
    Ocean,
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub outside: Outside,
    /// Allow writing into a directory that already contains a level.dat.
    pub overwrite: bool,
    pub version: McVersion,
    /// Worker threads; `None` uses every core.
    pub threads: Option<usize>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            outside: Outside::Vanilla,
            overwrite: false,
            version: McVersion::default(),
            threads: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ExportReport {
    pub path: PathBuf,
    pub chunks: usize,
    pub regions: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Nbt(groundwork_nbt::Error),
    Region(groundwork_anvil::Error),
    Refused(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Nbt(e) => write!(f, "{e}"),
            Error::Region(e) => write!(f, "{e}"),
            Error::Refused(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
impl From<groundwork_nbt::Error> for Error {
    fn from(e: groundwork_nbt::Error) -> Self {
        Error::Nbt(e)
    }
}
impl From<groundwork_anvil::Error> for Error {
    fn from(e: groundwork_anvil::Error) -> Self {
        Error::Region(e)
    }
}

/// Writes `world` as a Minecraft save in `dir` (created if necessary).
pub fn export(
    world: &World,
    dir: impl AsRef<Path>,
    options: &ExportOptions,
) -> Result<ExportReport, Error> {
    export_with_progress(world, dir, options, |_, _| {})
}

/// Like [`export`], calling `progress(regions_done, regions_total)` from the
/// worker threads as region files complete.
pub fn export_with_progress(
    world: &World,
    dir: impl AsRef<Path>,
    options: &ExportOptions,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<ExportReport, Error> {
    let dir = dir.as_ref();
    if dir.join("level.dat").exists() && !options.overwrite {
        return Err(Error::Refused(format!(
            "{} already contains a Minecraft world; pass overwrite to replace its chunks",
            dir.display()
        )));
    }
    std::fs::create_dir_all(dir)?;

    let mut report = ExportReport {
        path: dir.to_owned(),
        ..Default::default()
    };
    let warnings = Mutex::new(Vec::new());
    let total: usize = world
        .dimensions()
        .iter()
        .map(|d| region_coords(d).len())
        .sum();
    let done = AtomicUsize::new(0);
    let tick = || progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
    for dim in world.dimensions() {
        let region_dir = match dim.kind {
            DimensionKind::Overworld => dir.join("region"),
            DimensionKind::Nether => dir.join("DIM-1").join("region"),
            DimensionKind::End => dir.join("DIM1").join("region"),
        };
        std::fs::create_dir_all(&region_dir)?;
        let (chunks, regions) = export_dimension(dim, &region_dir, options, &warnings, &tick)?;
        report.chunks += chunks;
        report.regions += regions;
    }

    let spawn_height = world
        .dimension(DimensionKind::Overworld)
        .and_then(|d| {
            let (x, y) = world.spawn;
            Some(d.int_height_at(x, y)?.max(d.water_level_at(x, y)?) + 1)
        })
        .unwrap_or(64);
    level::write_level_dat(world, dir, spawn_height, options.outside, options.version)?;
    std::fs::write(dir.join("session.lock"), "\u{2603}")?;

    let mut warnings = warnings.into_inner().unwrap();
    warnings.sort();
    warnings.dedup();
    report.warnings = warnings;
    Ok(report)
}

/// Region coordinates covered by a dimension's tiles (512 blocks = 4 tiles).
fn region_coords(dim: &Dimension) -> Vec<(i32, i32)> {
    let mut regions: Vec<(i32, i32)> = dim
        .tile_coords()
        .into_iter()
        .map(|(tx, ty)| (tx >> 2, ty >> 2))
        .collect();
    regions.sort_unstable();
    regions.dedup();
    regions
}

/// Exports one dimension's tiles, one region (4×4 tiles) per worker thread.
fn export_dimension(
    dim: &Dimension,
    region_dir: &Path,
    options: &ExportOptions,
    warnings: &Mutex<Vec<String>>,
    tick: &(impl Fn() + Sync),
) -> Result<(usize, usize), Error> {
    let (vanilla_min, vanilla_max) = dim.kind.vanilla_height_range();
    let (min_y, max_y) = (
        dim.min_height.max(vanilla_min),
        dim.max_height.min(vanilla_max),
    );
    if (min_y, max_y) != (dim.min_height, dim.max_height) {
        warnings.lock().unwrap().push(format!(
            "{:?}: world height {}..{} exceeds vanilla range {}..{}; blocks outside it were not exported",
            dim.kind, dim.min_height, dim.max_height, vanilla_min, vanilla_max
        ));
    }

    let regions = region_coords(dim);
    let threads = options
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .clamp(1, regions.len().max(1));
    let ctx = chunk::Context {
        data_version: options.version.data_version(),
        materials: materials::Materials::new(dim.seed),
    };
    let next = Mutex::new(regions.iter());
    let chunk_total = Mutex::new(0usize);
    let first_error: Mutex<Option<Error>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let Some(&(rx, rz)) = next.lock().unwrap().next() else {
                    break;
                };
                if first_error.lock().unwrap().is_some() {
                    break;
                }
                match export_region(dim, region_dir, rx, rz, min_y, max_y, &ctx, warnings) {
                    Ok(n) => {
                        *chunk_total.lock().unwrap() += n;
                        tick();
                    }
                    Err(e) => {
                        first_error.lock().unwrap().get_or_insert(e);
                    }
                }
            });
        }
    });
    if let Some(e) = first_error.into_inner().unwrap() {
        return Err(e);
    }
    Ok((chunk_total.into_inner().unwrap(), regions.len()))
}

#[allow(clippy::too_many_arguments)]
fn export_region(
    dim: &Dimension,
    region_dir: &Path,
    rx: i32,
    rz: i32,
    min_y: i32,
    max_y: i32,
    ctx: &chunk::Context,
    warnings: &Mutex<Vec<String>>,
) -> Result<usize, Error> {
    let path = region_dir.join(groundwork_anvil::region_file_name(rx, rz));
    let mut region = groundwork_anvil::RegionFile::open(&path)?;
    let mut count = 0;
    let chunks_per_tile = TILE_SIZE / 16;
    for ty in rz * 4..rz * 4 + 4 {
        for tx in rx * 4..rx * 4 + 4 {
            let Some(tile) = dim.tile(tx, ty) else {
                continue;
            };
            for cz in 0..chunks_per_tile {
                for cx in 0..chunks_per_tile {
                    let chunk_x = tx * chunks_per_tile + cx;
                    let chunk_z = ty * chunks_per_tile + cz;
                    let nbt =
                        chunk::build(dim, tile, chunk_x, chunk_z, min_y, max_y, ctx, warnings);
                    region.write_chunk(split_region(chunk_x), split_region(chunk_z), &nbt)?;
                    count += 1;
                }
            }
        }
    }
    region.flush()?;
    Ok(count)
}

fn split_region(chunk: i32) -> i32 {
    chunk & 31
}

#[cfg(test)]
mod tests;

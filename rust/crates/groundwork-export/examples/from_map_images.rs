//! Builds a large Minecraft world from three renders of a fantasy map made
//! with Azgaar's Fantasy Map Generator:
//!
//! 1. the plain map (land is near-white, water is blue, lakes light blue),
//! 2. the height map (six elevation bands, teal-green low to yellow high),
//! 3. the biome map (Azgaar's standard biome colours).
//!
//! The three screenshots may be cropped or scaled slightly differently; they
//! are aligned by the bounding box of their land. The map is scaled to fit
//! the world, and noise adds coastline, hill and ridge detail below the
//! images' resolution.
//!
//! ```text
//! cargo run --release -p groundwork-export --example from_map_images -- \
//!     map.png heights.png biomes.png OUT_DIR [NAME] [SIZE_BLOCKS] [SEED]
//! ```
//!
//! Writes OUT_DIR/<NAME>.gwork, OUT_DIR/<NAME>-preview.png and the Minecraft
//! 1.21.11 save OUT_DIR/<NAME>/.

use groundwork_core::noise::PerlinNoise;
use groundwork_core::{
    biomes, layers, render, Dimension, DimensionKind, Terrain, Tile, World, TILE_SIZE,
};
use groundwork_export::{ExportOptions, McVersion, Outside};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const SEA_LEVEL: i32 = 62;
/// Worker threads. Kept modest: this machine's CPU is unstable under full
/// multi-core load.
const THREADS: usize = 6;

// ------------------------------------------------------------ palettes --

/// Every blue Azgaar uses for sea, including the shallow-water bands.
const OCEAN: &[[u8; 3]] = &[
    [104, 136, 184],
    [120, 152, 192],
    [120, 144, 192],
    [136, 160, 200],
    [152, 168, 208],
    [96, 128, 176],
    [144, 168, 200],
    [88, 120, 160],
    [96, 128, 168],
    [96, 120, 168],
    [104, 128, 184],
    [104, 128, 176],
    [88, 112, 152],
    [88, 112, 160],
    [128, 152, 192],
];
const LAKE: [u8; 3] = [152, 176, 224];
const OUTLINE_LAND: &[([u8; 3], u8)] = &[
    ([232, 240, 248], 0),
    ([224, 232, 240], 0),
    ([240, 240, 248], 0),
];
/// Elevation bands, lowest first.
const HEIGHT_BANDS: &[([u8; 3], u8)] = &[
    ([104, 184, 168], 0),
    ([136, 208, 160], 1),
    ([176, 224, 160], 2),
    ([208, 232, 152], 3),
    ([232, 248, 168], 4),
    ([248, 248, 176], 5),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Biome {
    HotDesert = 1,
    ColdDesert = 2,
    Savanna = 3,
    Grassland = 4,
    DeciduousForest = 5,
    SeasonalForest = 6,
    Rainforest = 7,
}

const BIOME_COLOURS: &[([u8; 3], u8)] = &[
    ([248, 224, 152], Biome::HotDesert as u8),
    ([176, 184, 128], Biome::ColdDesert as u8),
    ([208, 208, 128], Biome::Savanna as u8),
    ([200, 208, 136], Biome::Grassland as u8),
    ([40, 184, 80], Biome::DeciduousForest as u8),
    ([176, 216, 88], Biome::SeasonalForest as u8),
    ([64, 152, 64], Biome::Rainforest as u8),
];

fn biome_from(v: u8) -> Biome {
    match v {
        1 => Biome::HotDesert,
        2 => Biome::ColdDesert,
        3 => Biome::Savanna,
        5 => Biome::DeciduousForest,
        6 => Biome::SeasonalForest,
        7 => Biome::Rainforest,
        _ => Biome::Grassland,
    }
}

// --------------------------------------------------------------- images --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Px {
    Ocean,
    Lake,
    Land(u8),
    Unknown,
}

struct Grid<T> {
    w: usize,
    h: usize,
    v: Vec<T>,
}

impl<T: Copy> Grid<T> {
    fn new(w: usize, h: usize, fill: T) -> Self {
        Grid {
            w,
            h,
            v: vec![fill; w * h],
        }
    }
    fn at(&self, x: usize, y: usize) -> T {
        self.v[y * self.w + x]
    }
    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        (-1..=1i64)
            .flat_map(move |dy| (-1..=1i64).map(move |dx| (dx, dy)))
            .filter_map(move |(dx, dy)| {
                let (nx, ny) = (x + dx, y + dy);
                ((dx, dy) != (0, 0)
                    && nx >= 0
                    && ny >= 0
                    && nx < self.w as i64
                    && ny < self.h as i64)
                    .then(|| ny as usize * self.w + nx as usize)
            })
    }
}

fn load_png(path: &str) -> Grid<[u8; 3]> {
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(path).unwrap_or_else(|e| panic!("{path}: {e}")),
    ));
    let mut reader = decoder.read_info().expect("PNG header");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("PNG data");
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("{path}: unsupported PNG colour type {other:?}"),
    };
    assert_eq!(
        info.bit_depth,
        png::BitDepth::Eight,
        "{path}: need 8-bit PNG"
    );
    let (w, h) = (info.width as usize, info.height as usize);
    let v = buf[..w * h * channels]
        .chunks(channels)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    Grid { w, h, v }
}

fn close(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3)
        .map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs())
        .max()
        .unwrap()
}

/// Nearest palette colour within tolerance, else Unknown.
fn classify(img: &Grid<[u8; 3]>, land: &[([u8; 3], u8)]) -> Grid<Px> {
    const TOLERANCE: u32 = 18;
    let v = img
        .v
        .iter()
        .map(|&c| {
            let mut best = (u32::MAX, Px::Unknown);
            for &o in OCEAN {
                if close(c, o) < best.0 {
                    best = (close(c, o), Px::Ocean);
                }
            }
            if close(c, LAKE) < best.0 {
                best = (close(c, LAKE), Px::Lake);
            }
            for &(colour, k) in land {
                if close(c, colour) < best.0 {
                    best = (close(c, colour), Px::Land(k));
                }
            }
            if best.0 <= TOLERANCE {
                best.1
            } else {
                Px::Unknown
            }
        })
        .collect();
    let mut grid = Grid {
        w: img.w,
        h: img.h,
        v,
    };
    // Dotted route lines and scale-bar strokes are one or two pixels wide;
    // land pixels with almost no land around them are such marks.
    let thin: Vec<usize> = (0..grid.v.len())
        .filter(|&i| matches!(grid.v[i], Px::Land(_)))
        .filter(|&i| {
            grid.neighbours(i)
                .filter(|&n| matches!(grid.v[n], Px::Land(_)))
                .count()
                < 3
        })
        .collect();
    for i in thin {
        grid.v[i] = Px::Unknown;
    }
    // Scale bars and long route lines can be a few pixels thick: drop land
    // shapes that are long but at most three pixels across.
    for component in components(&grid, |p| matches!(p, Px::Land(_))) {
        let (x0, y0, x1, y1) =
            component
                .iter()
                .fold((usize::MAX, usize::MAX, 0, 0), |(a, b, c, d), &i| {
                    let (x, y) = (i % grid.w, i / grid.w);
                    (a.min(x), b.min(y), c.max(x), d.max(y))
                });
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        if w.min(h) <= 3 && w.max(h) >= 15 {
            for i in component {
                grid.v[i] = Px::Unknown;
            }
        }
    }
    fill_unknown(&mut grid);
    // Real lakes are enclosed by land. Lake-coloured pixels touching the sea
    // are anti-aliased coastline, and specks of one to three pixels are noise.
    for component in components(&grid, |p| p == Px::Lake) {
        let touches_sea = component
            .iter()
            .any(|&i| grid.neighbours(i).any(|n| grid.v[n] == Px::Ocean));
        let replacement = if touches_sea {
            Px::Ocean
        } else if component.len() < 4 {
            Px::Unknown
        } else {
            continue;
        };
        for i in component {
            grid.v[i] = replacement;
        }
    }
    fill_unknown(&mut grid);
    grid
}

/// 8-connected components of pixels matching `pred`.
fn components(grid: &Grid<Px>, pred: impl Fn(Px) -> bool) -> Vec<Vec<usize>> {
    let mut seen = vec![false; grid.v.len()];
    let mut out = Vec::new();
    for start in 0..grid.v.len() {
        if seen[start] || !pred(grid.v[start]) {
            continue;
        }
        seen[start] = true;
        let mut members = vec![start];
        let mut k = 0;
        while k < members.len() {
            let i = members[k];
            k += 1;
            for n in grid.neighbours(i) {
                if !seen[n] && pred(grid.v[n]) {
                    seen[n] = true;
                    members.push(n);
                }
            }
        }
        out.push(members);
    }
    out
}

/// Gives every Unknown pixel the class of the nearest known pixel.
fn fill_unknown(grid: &mut Grid<Px>) {
    let mut queue: VecDeque<usize> = (0..grid.v.len())
        .filter(|&i| grid.v[i] != Px::Unknown)
        .collect();
    while let Some(i) = queue.pop_front() {
        let value = grid.v[i];
        let next: Vec<usize> = grid
            .neighbours(i)
            .filter(|&n| grid.v[n] == Px::Unknown)
            .collect();
        for n in next {
            grid.v[n] = value;
            queue.push_back(n);
        }
    }
}

/// Bounding box of land (and lakes): (x0, y0, x1, y1).
fn land_bbox(grid: &Grid<Px>) -> (f32, f32, f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..grid.h {
        for x in 0..grid.w {
            if matches!(grid.at(x, y), Px::Land(_) | Px::Lake) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x0 as f32, y0 as f32, x1 as f32, y1 as f32)
}

/// Samples `src` (aligned by land bounding boxes) for every land pixel of
/// the master grid; other pixels take the nearest land value.
fn resample_land_values(master: &Grid<Px>, src: &Grid<Px>) -> Grid<u8> {
    let (mx0, my0, mx1, my1) = land_bbox(master);
    let (sx0, sy0, sx1, sy1) = land_bbox(src);
    let mut out = Grid {
        w: master.w,
        h: master.h,
        v: vec![Px::Unknown; master.v.len()],
    };
    for y in 0..master.h {
        for x in 0..master.w {
            if !matches!(master.at(x, y), Px::Land(_)) {
                continue;
            }
            let u = sx0 + (x as f32 - mx0) / (mx1 - mx0) * (sx1 - sx0);
            let v = sy0 + (y as f32 - my0) / (my1 - my0) * (sy1 - sy0);
            let (u, v) = (
                u.round().clamp(0.0, (src.w - 1) as f32) as usize,
                v.round().clamp(0.0, (src.h - 1) as f32) as usize,
            );
            if let Px::Land(k) = src.at(u, v) {
                out.v[y * master.w + x] = Px::Land(k);
            }
        }
    }
    fill_unknown(&mut out);
    Grid {
        w: out.w,
        h: out.h,
        v: out
            .v
            .iter()
            .map(|p| if let Px::Land(k) = p { *k } else { 0 })
            .collect(),
    }
}

/// Chamfer distance (in pixels) from every pixel to the nearest `target`.
fn distance(w: usize, h: usize, target: impl Fn(usize) -> bool) -> Grid<f32> {
    let mut d = Grid {
        w,
        h,
        v: (0..w * h)
            .map(|i| if target(i) { 0.0f32 } else { 1e9 })
            .collect(),
    };
    let (a, b) = (1.0f32, std::f32::consts::SQRT_2);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut m = d.v[i];
            if x > 0 {
                m = m.min(d.v[i - 1] + a);
            }
            if y > 0 {
                m = m.min(d.v[i - w] + a);
                if x > 0 {
                    m = m.min(d.v[i - w - 1] + b);
                }
                if x + 1 < w {
                    m = m.min(d.v[i - w + 1] + b);
                }
            }
            d.v[i] = m;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut m = d.v[i];
            if x + 1 < w {
                m = m.min(d.v[i + 1] + a);
            }
            if y + 1 < h {
                m = m.min(d.v[i + w] + a);
                if x + 1 < w {
                    m = m.min(d.v[i + w + 1] + b);
                }
                if x > 0 {
                    m = m.min(d.v[i + w - 1] + b);
                }
            }
            d.v[i] = m;
        }
    }
    d
}

/// Separable box blur, repeated for a near-Gaussian result.
fn blur(src: &Grid<f32>, radius: usize, passes: usize) -> Grid<f32> {
    let (w, h) = (src.w, src.h);
    let mut cur = src.v.clone();
    let mut tmp = vec![0.0f32; w * h];
    for _ in 0..passes {
        for y in 0..h {
            for x in 0..w {
                let (lo, hi) = (x.saturating_sub(radius), (x + radius).min(w - 1));
                tmp[y * w + x] =
                    (lo..=hi).map(|i| cur[y * w + i]).sum::<f32>() / (hi - lo + 1) as f32;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let (lo, hi) = (y.saturating_sub(radius), (y + radius).min(h - 1));
                cur[y * w + x] =
                    (lo..=hi).map(|j| tmp[j * w + x]).sum::<f32>() / (hi - lo + 1) as f32;
            }
        }
    }
    Grid { w, h, v: cur }
}

fn bilinear(g: &Grid<f32>, u: f32, v: f32) -> f32 {
    let u = u.clamp(0.0, (g.w - 1) as f32);
    let v = v.clamp(0.0, (g.h - 1) as f32);
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(g.w - 1), (y0 + 1).min(g.h - 1));
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let top = g.at(x0, y0) * (1.0 - fx) + g.at(x1, y0) * fx;
    let bottom = g.at(x0, y1) * (1.0 - fx) + g.at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------- model --

/// Everything the column generator needs, in image-pixel space.
struct Map {
    class: Grid<Px>,
    biome: Grid<u8>,
    land: Grid<f32>,
    elevation: Grid<f32>,
    mountains: Grid<f32>,
    to_sea: Grid<f32>,
    to_land: Grid<f32>,
    into_lake: Grid<f32>,
    /// Lake surface for lake pixels and land pixels right next to lakes.
    lake_level: Grid<f32>,
    /// World blocks per image pixel, and the world position of pixel (0,0).
    scale: f32,
    origin: (f32, f32),
}

struct Noises {
    coast: PerlinNoise,
    coast_fine: PerlinNoise,
    warp_x: PerlinNoise,
    warp_y: PerlinNoise,
    hills: PerlinNoise,
    ridges: PerlinNoise,
    detail: PerlinNoise,
    floor: PerlinNoise,
    patches: PerlinNoise,
    clearings: PerlinNoise,
}

impl Noises {
    fn new(seed: u64) -> Self {
        let n = |salt: u64| PerlinNoise::new(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        Noises {
            coast: n(1),
            coast_fine: n(2),
            warp_x: n(3),
            warp_y: n(4),
            hills: n(5),
            ridges: n(6),
            detail: n(7),
            floor: n(8),
            patches: n(9),
            clearings: n(10),
        }
    }
}

/// Approximate land height before noise, used for lake surfaces.
fn base_height(elevation: f32, to_sea_blocks: f32) -> f32 {
    let coast = smoothstep(0.0, 140.0, to_sea_blocks);
    63.0 + coast * (1.5 + (elevation / 5.0).powf(1.5) * 95.0)
}

impl Map {
    fn build(map_png: &str, heights_png: &str, biomes_png: &str, world_size: f32) -> Map {
        let t = Instant::now();
        let class = classify(&load_png(map_png), OUTLINE_LAND);
        let heights = classify(&load_png(heights_png), HEIGHT_BANDS);
        let biome_px = classify(&load_png(biomes_png), BIOME_COLOURS);
        let (w, h) = (class.w, class.h);
        let elevation_raw = resample_land_values(&class, &heights);
        let biome = resample_land_values(&class, &biome_px);
        eprintln!(
            "  images classified and aligned in {:.1}s",
            t.elapsed().as_secs_f32()
        );

        let land_or_lake = |i: usize| matches!(class.v[i], Px::Land(_) | Px::Lake);
        let land = blur(
            &Grid {
                w,
                h,
                v: (0..w * h)
                    .map(|i| if land_or_lake(i) { 1.0 } else { 0.0 })
                    .collect(),
            },
            1,
            2,
        );
        let elevation = blur(
            &Grid {
                w,
                h,
                v: elevation_raw.v.iter().map(|&k| k as f32).collect(),
            },
            2,
            3,
        );
        let mountains = blur(
            &Grid {
                w,
                h,
                v: biome
                    .v
                    .iter()
                    .map(|&b| {
                        if b == Biome::ColdDesert as u8 {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect(),
            },
            4,
            3,
        );
        let to_sea = distance(w, h, |i| class.v[i] == Px::Ocean);
        // Chamfer distances have octagonal contours; blurring rounds the
        // shallow-water bands off.
        let to_land = blur(&distance(w, h, |i| class.v[i] != Px::Ocean), 3, 3);
        let into_lake = distance(w, h, |i| class.v[i] != Px::Lake);

        let scale = world_size * 0.95 / w.max(h) as f32;
        let origin = (-(w as f32) * scale / 2.0, -(h as f32) * scale / 2.0);

        // Lake surfaces: the lowest land around each lake, so water never
        // spills over its rim.
        let mut lake_level = Grid::new(w, h, f32::NAN);
        let mut seen = vec![false; w * h];
        let mut lakes = 0;
        for start in 0..w * h {
            if class.v[start] != Px::Lake || seen[start] {
                continue;
            }
            let mut members = vec![start];
            seen[start] = true;
            let mut rim: f32 = f32::MAX;
            let mut k = 0;
            while k < members.len() {
                let i = members[k];
                k += 1;
                for n in class.neighbours(i) {
                    match class.v[n] {
                        Px::Lake if !seen[n] => {
                            seen[n] = true;
                            members.push(n);
                        }
                        Px::Land(_) => {
                            rim = rim.min(base_height(elevation.v[n], to_sea.v[n] * scale))
                        }
                        _ => {}
                    }
                }
            }
            let surface = rim.max(63.0).floor();
            for &i in &members {
                lake_level.v[i] = surface;
            }
            lakes += 1;
        }
        // Spread each lake's level a few pixels onto its shore so the shore
        // can be kept above the water.
        let mut queue: VecDeque<(usize, u32)> = (0..w * h)
            .filter(|&i| !lake_level.v[i].is_nan())
            .map(|i| (i, 0))
            .collect();
        while let Some((i, d)) = queue.pop_front() {
            if d >= 3 {
                continue;
            }
            let level = lake_level.v[i];
            let next: Vec<usize> = class
                .neighbours(i)
                .filter(|&n| lake_level.v[n].is_nan())
                .collect();
            for n in next {
                lake_level.v[n] = level;
                queue.push_back((n, d + 1));
            }
        }
        eprintln!(
            "  {lakes} lakes; fields ready in {:.1}s; {scale:.2} blocks per pixel",
            t.elapsed().as_secs_f32()
        );
        Map {
            class,
            biome,
            land,
            elevation,
            mountains,
            to_sea,
            to_land,
            into_lake,
            lake_level,
            scale,
            origin,
        }
    }

    fn to_image(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x + 0.5 - self.origin.0) / self.scale,
            (y + 0.5 - self.origin.1) / self.scale,
        )
    }

    fn nearest<T: Copy>(&self, g: &Grid<T>, u: f32, v: f32) -> T {
        g.at(
            u.round().clamp(0.0, (g.w - 1) as f32) as usize,
            v.round().clamp(0.0, (g.h - 1) as f32) as usize,
        )
    }
}

enum Column {
    Ocean {
        floor: f32,
        from_land: f32,
    },
    Lake {
        ground: f32,
        surface: i32,
        from_shore: f32,
        biome: Biome,
    },
    Land {
        height: f32,
        biome: Biome,
        from_sea: f32,
        mountains: f32,
    },
}

fn column(map: &Map, n: &Noises, x: i32, y: i32) -> Column {
    let (xf, yf) = (x as f64, y as f64);
    let (u, v) = map.to_image(x as f32, y as f32);
    let inside =
        u >= 0.0 && v >= 0.0 && u <= (map.class.w - 1) as f32 && v <= (map.class.h - 1) as f32;
    // Warped lookups give categorical borders (biomes, lakes) organic edges.
    let wu = u + n.warp_x.fbm(xf / 55.0, yf / 55.0, 3, 0.5) as f32 * 1.6;
    let wv = v + n.warp_y.fbm(xf / 55.0, yf / 55.0, 3, 0.5) as f32 * 1.6;

    let coastline = bilinear(&map.land, u, v)
        + n.coast.fbm(xf / 80.0, yf / 80.0, 4, 0.5) as f32 * 0.28
        + n.coast_fine.fbm(xf / 22.0, yf / 22.0, 2, 0.5) as f32 * 0.09;
    if !inside || coastline <= 0.5 {
        // Beyond the image, keep deepening with distance from its edge so
        // there is no step in the sea floor at the border.
        let (cu, cv) = (
            u.clamp(0.0, (map.class.w - 1) as f32),
            v.clamp(0.0, (map.class.h - 1) as f32),
        );
        let outside = ((u - cu).powi(2) + (v - cv).powi(2)).sqrt();
        let from_land = (bilinear(&map.to_land, cu, cv) + outside) * map.scale;
        let floor = SEA_LEVEL as f32
            - 1.0
            - 4.0 * smoothstep(0.0, 180.0, from_land)
            - 10.0 * smoothstep(120.0, 700.0, from_land)
            - 14.0 * smoothstep(600.0, 2200.0, from_land)
            + n.floor.fbm(xf / 150.0, yf / 150.0, 3, 0.5) as f32 * 2.5;
        return Column::Ocean {
            floor: floor.min(SEA_LEVEL as f32 - 1.0),
            from_land,
        };
    }

    let biome = biome_from(map.nearest(&map.biome, wu, wv));
    if map.nearest(&map.class, wu, wv) == Px::Lake {
        let surface = map.nearest(&map.lake_level, u, v);
        if !surface.is_nan() {
            let from_shore = bilinear(&map.into_lake, u, v) * map.scale;
            let ground = surface - 1.5 - 8.0 * smoothstep(0.0, 120.0, from_shore)
                + n.floor.fbm(xf / 40.0, yf / 40.0, 2, 0.5) as f32;
            return Column::Lake {
                ground,
                surface: surface as i32,
                from_shore,
                biome,
            };
        }
    }

    let from_sea = bilinear(&map.to_sea, u, v) * map.scale;
    let elevation = bilinear(&map.elevation, u, v);
    let mountains = bilinear(&map.mountains, u, v);
    let coast = smoothstep(0.0, 140.0, from_sea);
    let e = elevation / 5.0;
    let hills = n.hills.fbm(xf / 240.0, yf / 240.0, 5, 0.5) as f32 * (3.0 + 16.0 * e);
    let ridge = (1.0 - n.ridges.fbm(xf / 380.0, yf / 380.0, 4, 0.5).abs() as f32).powi(3)
        * 70.0
        * mountains;
    let detail = n.detail.fbm(xf / 45.0, yf / 45.0, 3, 0.5) as f32 * (0.8 + 3.0 * mountains);
    let mut height = base_height(elevation, from_sea) + coast * (hills + ridge + detail);
    let shore_lake = map.nearest(&map.lake_level, u, v);
    if !shore_lake.is_nan() {
        height = height.max(shore_lake + 1.0);
    }
    Column::Land {
        height: height.clamp(63.0, 300.0),
        biome,
        from_sea,
        mountains,
    }
}

fn biome_id(name: &str) -> u8 {
    biomes::find(name).unwrap_or_else(|| panic!("unknown biome {name}"))
}

struct Ids {
    ocean: u8,
    deep_ocean: u8,
    beach: u8,
    stony_shore: u8,
    desert: u8,
    gravelly: u8,
    snowy_slopes: u8,
    frozen_peaks: u8,
    savanna: u8,
    plains: u8,
    forest: u8,
    sparse_jungle: u8,
    spruce_taiga: u8,
}

fn gen_tile(map: &Map, n: &Noises, ids: &Ids, tx: i32, ty: i32) -> Tile {
    const S: i32 = TILE_SIZE;
    const P: usize = (TILE_SIZE + 2) as usize;
    let mut tile = Tile::new(tx, ty, -64, 320);
    // Heights with a one-column border for slopes.
    let mut cols = Vec::with_capacity(P * P);
    let mut heights = vec![0.0f32; P * P];
    for py in 0..P as i32 {
        for px in 0..P as i32 {
            let c = column(map, n, tx * S + px - 1, ty * S + py - 1);
            heights[(py as usize) * P + px as usize] = match &c {
                Column::Ocean { floor, .. } => *floor,
                Column::Lake { ground, .. } => *ground,
                Column::Land { height, .. } => *height,
            };
            cols.push(c);
        }
    }
    let h = |px: i32, py: i32| heights[(py as usize) * P + px as usize];
    for ly in 0..S {
        for lx in 0..S {
            let (px, py) = (lx + 1, ly + 1);
            let (x, y) = (tx * S + lx, ty * S + ly);
            let (xf, yf) = (x as f64, y as f64);
            let height = h(px, py);
            tile.set_height(lx, ly, height);
            let patch = n.patches.fbm(xf / 30.0, yf / 30.0, 3, 0.5) as f32;
            match &cols[(py as usize) * P + px as usize] {
                Column::Ocean { from_land, .. } => {
                    tile.set_water_level(lx, ly, SEA_LEVEL);
                    let terrain = if *from_land < 350.0 || patch > 0.35 {
                        Terrain::Sand
                    } else if patch < -0.3 {
                        Terrain::Clay
                    } else {
                        Terrain::Gravel
                    };
                    tile.set_terrain(lx, ly, terrain);
                    let biome = if *from_land > 1500.0 {
                        ids.deep_ocean
                    } else {
                        ids.ocean
                    };
                    tile.set_layer_value(layers::BIOME, lx, ly, biome);
                }
                Column::Lake {
                    surface,
                    from_shore,
                    biome,
                    ..
                } => {
                    tile.set_water_level(lx, ly, *surface);
                    tile.set_terrain(
                        lx,
                        ly,
                        if *from_shore < 40.0 || patch > 0.2 {
                            Terrain::Sand
                        } else {
                            Terrain::Clay
                        },
                    );
                    tile.set_layer_value(layers::BIOME, lx, ly, land_biome(ids, *biome, height));
                }
                Column::Land {
                    biome,
                    from_sea,
                    mountains,
                    ..
                } => {
                    tile.set_water_level(lx, ly, -64);
                    let slope = ((h(px + 1, py) - h(px - 1, py)).abs() / 2.0)
                        .max((h(px, py + 1) - h(px, py - 1)).abs() / 2.0);
                    let snow_line = 196.0 + patch * 8.0;
                    let frost = height > snow_line;
                    let shore = *from_sea < 14.0 + patch * 8.0 && height < 65.5;
                    let (terrain, biome_value, deciduous, pine) = if shore {
                        if *biome == Biome::ColdDesert {
                            (Terrain::Gravel, ids.stony_shore, 0, 0)
                        } else {
                            (Terrain::Sand, ids.beach, 0, 0)
                        }
                    } else if slope > 1.5 && *biome != Biome::HotDesert {
                        (Terrain::Stone, land_biome(ids, *biome, height), 0, 0)
                    } else {
                        let clearing = (0.55
                            + n.clearings.fbm(xf / 90.0, yf / 90.0, 3, 0.5) as f32 * 1.4)
                            .clamp(0.0, 1.0);
                        let density = |base: f32| (base * clearing).round() as u8;
                        match biome {
                            Biome::HotDesert => (Terrain::Desert, ids.desert, 0, 0),
                            Biome::ColdDesert => {
                                let t = if patch > 0.3 {
                                    Terrain::Gravel
                                } else if patch < -0.35 {
                                    Terrain::Andesite
                                } else if *mountains < 0.5 && patch < 0.0 {
                                    Terrain::BareGrass
                                } else {
                                    Terrain::Stone
                                };
                                (t, land_biome(ids, *biome, height), 0, 0)
                            }
                            Biome::Savanna => (Terrain::BareGrass, ids.savanna, density(3.0), 0),
                            Biome::Grassland => (Terrain::BareGrass, ids.plains, density(1.5), 0),
                            Biome::DeciduousForest => {
                                (Terrain::BareGrass, ids.forest, density(14.0), 0)
                            }
                            Biome::SeasonalForest => {
                                (Terrain::BareGrass, ids.sparse_jungle, density(9.0), 0)
                            }
                            Biome::Rainforest => {
                                (Terrain::Podzol, ids.spruce_taiga, 0, density(14.0))
                            }
                        }
                    };
                    tile.set_terrain(lx, ly, terrain);
                    tile.set_layer_value(
                        layers::BIOME,
                        lx,
                        ly,
                        if frost {
                            land_biome(ids, *biome, height.max(snow_line + 1.0))
                        } else {
                            biome_value
                        },
                    );
                    if frost {
                        tile.set_layer_value(layers::FROST, lx, ly, 1);
                    } else {
                        tile.set_layer_value(layers::DECIDUOUS, lx, ly, deciduous);
                        tile.set_layer_value(layers::PINE, lx, ly, pine);
                    }
                }
            }
        }
    }
    tile.compact();
    tile
}

fn land_biome(ids: &Ids, biome: Biome, height: f32) -> u8 {
    match biome {
        Biome::ColdDesert if height > 225.0 => ids.frozen_peaks,
        Biome::ColdDesert if height > 196.0 => ids.snowy_slopes,
        Biome::ColdDesert => ids.gravelly,
        Biome::HotDesert => ids.desert,
        Biome::Savanna => ids.savanna,
        Biome::Grassland => ids.plains,
        Biome::DeciduousForest => ids.forest,
        Biome::SeasonalForest => ids.sparse_jungle,
        Biome::Rainforest => ids.spruce_taiga,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: from_map_images MAP.png HEIGHTS.png BIOMES.png OUT_DIR [NAME] [SIZE_BLOCKS] [SEED]");
        std::process::exit(2);
    }
    let out_dir = PathBuf::from(&args[4]);
    let name = args
        .get(5)
        .cloned()
        .unwrap_or_else(|| "Groundwork Continent".into());
    let size: i32 = args
        .get(6)
        .map_or(10240, |s| s.parse().expect("SIZE_BLOCKS"));
    let seed: u64 = args
        .get(7)
        .map_or(20261028, |s| s.parse::<i64>().expect("SEED") as u64);
    let tiles_per_side = (size + TILE_SIZE - 1) / TILE_SIZE;
    std::fs::create_dir_all(&out_dir).unwrap();
    let started = Instant::now();

    eprintln!("Reading map images…");
    let map = Map::build(
        &args[1],
        &args[2],
        &args[3],
        (tiles_per_side * TILE_SIZE) as f32,
    );
    let noises = Noises::new(seed);
    let ids = Ids {
        ocean: biome_id("ocean"),
        deep_ocean: biome_id("deep_ocean"),
        beach: biome_id("beach"),
        stony_shore: biome_id("stony_shore"),
        desert: biome_id("desert"),
        gravelly: biome_id("windswept_gravelly_hills"),
        snowy_slopes: biome_id("snowy_slopes"),
        frozen_peaks: biome_id("frozen_peaks"),
        savanna: biome_id("savanna"),
        plains: biome_id("plains"),
        forest: biome_id("forest"),
        sparse_jungle: biome_id("sparse_jungle"),
        spruce_taiga: biome_id("old_growth_spruce_taiga"),
    };

    let half = tiles_per_side / 2;
    let coords: Vec<(i32, i32)> = (-half..tiles_per_side - half)
        .flat_map(|ty| (-half..tiles_per_side - half).map(move |tx| (tx, ty)))
        .collect();
    eprintln!(
        "Generating {} tiles ({}×{} blocks) on {THREADS} threads…",
        coords.len(),
        tiles_per_side * TILE_SIZE,
        tiles_per_side * TILE_SIZE
    );
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let tiles = Mutex::new(Vec::with_capacity(coords.len()));
    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(tx, ty)) = coords.get(i) else {
                    break;
                };
                let tile = gen_tile(&map, &noises, &ids, tx, ty);
                tiles.lock().unwrap().push(tile);
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if d.is_multiple_of(400) || d == coords.len() {
                    eprintln!(
                        "  {d}/{} tiles ({:.0}s)",
                        coords.len(),
                        started.elapsed().as_secs_f32()
                    );
                }
            });
        }
    });
    let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320, seed);
    for tile in tiles.into_inner().unwrap() {
        dim.insert_tile(tile);
    }

    // Spawn on dry land as close to the centre of the map as possible.
    let spawn = (0..2000)
        .step_by(8)
        .flat_map(|r: i32| {
            (0..32).map(move |k| {
                let a = k as f32 / 32.0 * std::f32::consts::TAU;
                ((a.cos() * r as f32) as i32, (a.sin() * r as f32) as i32)
            })
        })
        .find(|&(x, y)| {
            dim.int_height_at(x, y)
                .zip(dim.water_level_at(x, y))
                .is_some_and(|(h, w)| w <= h && h < 120)
                && dim.layer_value_at(layers::DECIDUOUS, x, y) == Some(0)
                && dim.layer_value_at(layers::PINE, x, y) == Some(0)
        })
        .unwrap_or((0, 0));
    let mut world = World::new(name.clone(), seed, dim);
    world.spawn = spawn;
    world.map_features = false;
    eprintln!("Spawn at {spawn:?}");

    let gwork = out_dir.join(format!("{name}.gwork"));
    groundwork_core::io::save(&world, &gwork).expect("save .gwork");
    eprintln!(
        "Saved {} ({:.0}s)",
        gwork.display(),
        started.elapsed().as_secs_f32()
    );

    let dim = world.dimension(DimensionKind::Overworld).unwrap();
    let (image, scale) = render::render(dim, 1, 2048).unwrap();
    let preview = out_dir.join(format!("{name}-preview.png"));
    std::fs::write(&preview, image.to_png().unwrap()).unwrap();
    eprintln!("Preview {} at {scale} blocks/pixel", preview.display());

    if std::env::var_os("SKIP_EXPORT").is_some() {
        return;
    }
    let save_dir = out_dir.join(&name);
    eprintln!(
        "Exporting Minecraft 1.21.11 world to {}…",
        save_dir.display()
    );
    let options = ExportOptions {
        outside: Outside::Ocean,
        overwrite: true,
        version: McVersion::V1_21_11,
        threads: Some(THREADS),
    };
    let last = AtomicUsize::new(0);
    let report =
        groundwork_export::export_with_progress(&world, &save_dir, &options, |done, total| {
            let pct = done * 100 / total;
            if pct >= last.load(Ordering::Relaxed) + 5 {
                last.store(pct, Ordering::Relaxed);
                eprintln!(
                    "  {pct}% ({done}/{total} regions, {:.0}s)",
                    started.elapsed().as_secs_f32()
                );
            }
        })
        .expect("export");
    eprintln!(
        "Exported {} chunks in {} region files ({:.0}s total)",
        report.chunks,
        report.regions,
        started.elapsed().as_secs_f32()
    );
    for w in &report.warnings {
        eprintln!("  note: {w}");
    }
}

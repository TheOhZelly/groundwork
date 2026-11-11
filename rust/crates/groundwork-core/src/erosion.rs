//! Hydraulic erosion: simulated raindrops run downhill, pick up soil where
//! they run fast and drop it where they slow down. This carves gullies and
//! valleys into slopes and builds fans and deltas at their feet.
//!
//! WorldPainter has no counterpart; this is new in Groundwork. The method is
//! the particle one described by Hans Theobald Beyer ("Implementation of a
//! method for hydraulic erosion", 2015).
//!
//! Large areas are eroded in overlapping windows whose results are blended,
//! so memory stays bounded and windows can run on several threads. Windows
//! are processed in four checkerboard phases; windows of one phase never
//! overlap, so the result does not depend on the thread count.

use crate::dimension::{split, Dimension, Extent};
use crate::noise::splitmix64;
use crate::ops::{Falloff, Shape};
use crate::tile::TILE_SIZE;
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct ErosionOptions {
    /// Raindrops per column. 1 is a clearly weathered landscape; 0.25 is a
    /// light touch; 4 is heavy.
    pub droplets: f32,
    pub seed: u64,
    /// Worker threads for large areas. Default: half the machine's threads,
    /// at most 8.
    pub threads: Option<usize>,
}

impl Default for ErosionOptions {
    fn default() -> Self {
        ErosionOptions {
            droplets: 1.0,
            seed: 0,
            threads: None,
        }
    }
}

/// How much of its previous direction a drop keeps each step.
const INERTIA: f32 = 0.1;
/// Soil a drop can carry, per unit of slope, speed and water.
const CAPACITY: f32 = 2.0;
/// Carrying capacity on flat ground, so drops never stop eroding entirely.
const MIN_CAPACITY: f32 = 0.01;
/// Fraction of the spare capacity filled per step.
const ERODE: f32 = 0.3;
/// Fraction of the excess load dropped per step.
const DEPOSIT: f32 = 0.3;
const EVAPORATION: f32 = 0.02;
const GRAVITY: f32 = 2.0;
/// Steps (roughly blocks) a drop travels at most.
const MAX_STEPS: u32 = 96;
/// Soil is taken from a disc this wide around the drop, so channels are a
/// few blocks wide rather than one-block slots.
const ERODE_RADIUS: i32 = 3;

/// Window core size and overlap for large areas.
const CORE: i32 = 1024;
const MARGIN: i32 = 128;

const MISSING: i16 = i16::MAX;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = splitmix64(self.0);
        self.0
    }

    /// `0.0..1.0`
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A dense copy of part of a dimension.
struct Grid {
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    height: Vec<f32>,
    /// Water level per column; `MISSING` where there is no tile.
    water: Vec<i16>,
}

/// The part of tile `t` (in one axis) inside `start..start + len`, as
/// offsets within the tile.
fn tile_span(t: i32, start: i32, len: usize) -> Range<i32> {
    let base = t * TILE_SIZE;
    (start.max(base) - base)..((start + len as i32).min(base + TILE_SIZE) - base)
}

/// Calls `f(tx, ty, x offsets, y offsets)` for each tile position that the
/// block rectangle at (`x0`, `y0`) of `w`×`h` overlaps.
fn for_each_tile(
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    mut f: impl FnMut(i32, i32, Range<i32>, Range<i32>),
) {
    let (x1, y1) = (x0 + w as i32 - 1, y0 + h as i32 - 1);
    for ty in split(y0).0..=split(y1).0 {
        for tx in split(x0).0..=split(x1).0 {
            f(tx, ty, tile_span(tx, x0, w), tile_span(ty, y0, h));
        }
    }
}

impl Grid {
    fn load(dim: &Dimension, x0: i32, y0: i32, w: usize, h: usize) -> Grid {
        let mut height = vec![0.0f32; w * h];
        let mut water = vec![MISSING; w * h];
        for_each_tile(x0, y0, w, h, |tx, ty, xs, ys| {
            let Some(tile) = dim.tile(tx, ty) else { return };
            for ly in ys {
                let row = (ty * TILE_SIZE + ly - y0) as usize * w;
                for lx in xs.clone() {
                    let i = row + (tx * TILE_SIZE + lx - x0) as usize;
                    height[i] = tile.height(lx, ly);
                    water[i] = tile
                        .water_level(lx, ly)
                        .clamp(i16::MIN as i32, (MISSING - 1) as i32)
                        as i16;
                }
            }
        });
        Grid {
            x0,
            y0,
            w,
            h,
            height,
            water,
        }
    }

    /// Heights at the corners of the cell whose top-left node is `i`, or
    /// `None` if one of them has no tile.
    #[inline]
    fn corners(&self, i: usize) -> Option<[f32; 4]> {
        let w = self.w;
        if self.water[i] == MISSING
            || self.water[i + 1] == MISSING
            || self.water[i + w] == MISSING
            || self.water[i + w + 1] == MISSING
        {
            return None;
        }
        Some([
            self.height[i],
            self.height[i + 1],
            self.height[i + w],
            self.height[i + w + 1],
        ])
    }

    /// Lets `count` drops fall on columns where `spawn` allows.
    fn rain(&mut self, count: u64, seed: u64, spawn: impl Fn(usize, usize) -> bool) {
        let (w, h) = (self.w, self.h);
        if w < 2 || h < 2 {
            return;
        }
        let disc = erosion_disc();
        let (max_x, max_y) = ((w - 1) as f32, (h - 1) as f32);
        let mut rng = Rng(seed);
        for _ in 0..count {
            let (mut px, mut py) = (rng.unit() * max_x, rng.unit() * max_y);
            if !spawn((px as usize).min(w - 1), (py as usize).min(h - 1)) {
                continue;
            }
            let (mut dx, mut dy) = (0.0f32, 0.0f32);
            let (mut speed, mut water, mut sediment) = (1.0f32, 1.0f32, 0.0f32);
            for _ in 0..MAX_STEPS {
                let (cx, cy) = (px as usize, py as usize);
                if cx + 1 >= w || cy + 1 >= h {
                    break;
                }
                let i = cy * w + cx;
                let Some([h00, h10, h01, h11]) = self.corners(i) else {
                    break;
                };
                let (u, v) = (px - cx as f32, py - cy as f32);
                let here = bilinear(h00, h10, h01, h11, u, v);

                // Reached standing water: settle as a delta, staying below
                // the surface so the shoreline doesn't creep outwards.
                let level = self.water[i] as f32;
                if here < level {
                    let room = (level - 1.0 - here).max(0.0);
                    self.deposit(i, u, v, sediment.min(room));
                    break;
                }

                let gx = (h10 - h00) * (1.0 - v) + (h11 - h01) * v;
                let gy = (h01 - h00) * (1.0 - u) + (h11 - h10) * u;
                dx = dx * INERTIA - gx * (1.0 - INERTIA);
                dy = dy * INERTIA - gy * (1.0 - INERTIA);
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-6 {
                    break;
                }
                dx /= len;
                dy /= len;
                let (nx, ny) = (px + dx, py + dy);
                if nx < 0.0 || ny < 0.0 || nx >= max_x || ny >= max_y {
                    break;
                }
                let (ncx, ncy) = (nx as usize, ny as usize);
                let Some([n00, n10, n01, n11]) = self.corners(ncy * w + ncx) else {
                    break;
                };
                let there = bilinear(n00, n10, n01, n11, nx - ncx as f32, ny - ncy as f32);
                let dh = there - here;

                let capacity = (-dh * speed * water * CAPACITY).max(MIN_CAPACITY);
                if dh > 0.0 || sediment > capacity {
                    // Uphill: fill the pit behind. Overloaded: drop the excess.
                    let drop = if dh > 0.0 {
                        dh.min(sediment)
                    } else {
                        (sediment - capacity) * DEPOSIT
                    };
                    sediment -= drop;
                    self.deposit(i, u, v, drop);
                } else {
                    // Never dig deeper than the step down, or pits form.
                    let take = ((capacity - sediment) * ERODE).min(-dh);
                    for &(kx, ky, weight) in &disc {
                        let (x, y) = (cx as i32 + kx, cy as i32 + ky);
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                            continue;
                        }
                        let j = y as usize * w + x as usize;
                        // Leave gaps in the map and anything under water alone.
                        if self.water[j] == MISSING || self.height[j] < self.water[j] as f32 {
                            continue;
                        }
                        let amount = take * weight;
                        self.height[j] -= amount;
                        sediment += amount;
                    }
                }
                speed = (speed * speed - dh * GRAVITY).max(0.0).sqrt();
                water *= 1.0 - EVAPORATION;
                px = nx;
                py = ny;
            }
        }
    }

    #[inline]
    fn deposit(&mut self, i: usize, u: f32, v: f32, amount: f32) {
        let w = self.w;
        self.height[i] += amount * (1.0 - u) * (1.0 - v);
        self.height[i + 1] += amount * u * (1.0 - v);
        self.height[i + w] += amount * (1.0 - u) * v;
        self.height[i + w + 1] += amount * u * v;
    }

    /// Writes the grid back, blending each column towards its new height by
    /// `weight(ix, iy)` (0 leaves it alone). Returns the columns changed.
    /// Tiles without changes are not touched, so they stay shared with undo
    /// snapshots.
    fn apply(&self, dim: &mut Dimension, weight: impl Fn(usize, usize) -> f32) -> usize {
        let mut changed = 0;
        let mut edits: Vec<(i32, i32, f32)> = Vec::new();
        for_each_tile(self.x0, self.y0, self.w, self.h, |tx, ty, xs, ys| {
            edits.clear();
            {
                let Some(tile) = dim.tile(tx, ty) else { return };
                for ly in ys {
                    let iy = (ty * TILE_SIZE + ly - self.y0) as usize;
                    for lx in xs.clone() {
                        let ix = (tx * TILE_SIZE + lx - self.x0) as usize;
                        let wgt = weight(ix, iy);
                        if wgt <= 0.0 {
                            continue;
                        }
                        let old = tile.height(lx, ly);
                        let new = old + (self.height[iy * self.w + ix] - old) * wgt;
                        // Heights are stored in 1/256 blocks.
                        if (new - old).abs() >= 1.0 / 256.0 {
                            edits.push((lx, ly, new));
                        }
                    }
                }
            }
            if edits.is_empty() {
                return;
            }
            if let Some(tile) = dim.tile_mut(tx, ty) {
                for &(lx, ly, new) in &edits {
                    tile.set_height(lx, ly, new);
                }
                changed += edits.len();
            }
        });
        changed
    }
}

#[inline]
fn bilinear(h00: f32, h10: f32, h01: f32, h11: f32, u: f32, v: f32) -> f32 {
    h00 * (1.0 - u) * (1.0 - v) + h10 * u * (1.0 - v) + h01 * (1.0 - u) * v + h11 * u * v
}

/// Offsets and weights (summing to 1) of the disc soil is taken from.
fn erosion_disc() -> Vec<(i32, i32, f32)> {
    let r = ERODE_RADIUS;
    let mut cells = Vec::new();
    let mut total = 0.0;
    for dy in -r..=r {
        for dx in -r..=r {
            let weight = r as f32 - ((dx * dx + dy * dy) as f32).sqrt();
            if weight > 0.0 {
                cells.push((dx, dy, weight));
                total += weight;
            }
        }
    }
    for cell in &mut cells {
        cell.2 /= total;
    }
    cells
}

/// Rounds to a whole number of drops, at random in proportion to the
/// fraction, so small areas and short brush dabs still get their share.
fn drop_count(droplets: f32, columns: usize, seed: u64) -> u64 {
    let exact = droplets.max(0.0) as f64 * columns as f64;
    let whole = exact.floor();
    let mut rng = Rng(seed ^ 0x6472_6f70);
    whole as u64 + ((rng.unit() as f64) < exact - whole) as u64
}

fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 8))
}

/// Erodes `area` (default: the whole dimension). Returns the number of
/// columns whose height changed.
pub fn erode(dim: &mut Dimension, area: Option<Extent>, opts: &ErosionOptions) -> usize {
    let Some(extent) = dim.extent() else { return 0 };
    let rect = match area {
        None => extent,
        Some(a) => Extent {
            x0: a.x0.min(a.x1).max(extent.x0),
            y0: a.y0.min(a.y1).max(extent.y0),
            x1: a.x0.max(a.x1).min(extent.x1),
            y1: a.y0.max(a.y1).min(extent.y1),
        },
    };
    if rect.x1 < rect.x0 || rect.y1 < rect.y0 {
        return 0;
    }
    let xs = Axis::new(rect.x0, rect.x1 + 1);
    let ys = Axis::new(rect.y0, rect.y1 + 1);
    let threads = opts.threads.unwrap_or_else(default_threads).max(1);
    let mut changed = 0;
    for phase in 0..4 {
        let mut windows: Vec<(i32, i32)> = Vec::new();
        for ky in 0..ys.count {
            for kx in 0..xs.count {
                if (kx & 1) + 2 * (ky & 1) == phase {
                    windows.push((kx, ky));
                }
            }
        }
        for batch in windows.chunks(threads) {
            let grids: Vec<Grid> = {
                let shared: &Dimension = dim;
                let (xs, ys) = (&xs, &ys);
                std::thread::scope(|scope| {
                    let handles: Vec<_> = batch
                        .iter()
                        .map(|&(kx, ky)| {
                            scope.spawn(move || {
                                let ((wx0, wx1), (wy0, wy1)) = (xs.window(kx), ys.window(ky));
                                let (w, h) = ((wx1 - wx0) as usize, (wy1 - wy0) as usize);
                                let mut grid = Grid::load(shared, wx0, wy0, w, h);
                                let seed = splitmix64(
                                    opts.seed ^ ((kx as u32 as u64) << 32 | ky as u32 as u64),
                                );
                                grid.rain(drop_count(opts.droplets, w * h, seed), seed, |_, _| {
                                    true
                                });
                                grid
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .map(|handle| handle.join().expect("erosion worker panicked"))
                        .collect()
                })
            };
            for (grid, &(kx, ky)) in grids.iter().zip(batch) {
                changed += grid.apply(dim, |ix, iy| {
                    xs.weight(kx, grid.x0 + ix as i32) * ys.weight(ky, grid.y0 + iy as i32)
                });
            }
        }
    }
    changed
}

/// Erodes under a brush. Drops fall inside the shape and the result fades
/// out with the brush strength, so strokes leave no hard edge.
pub fn erode_shape(
    dim: &mut Dimension,
    shape: Shape,
    falloff: Falloff,
    opts: &ErosionOptions,
) -> usize {
    let mut cells = Vec::new();
    shape.for_each(falloff, |x, y, s| cells.push((x, y, s)));
    let Some(&(first_x, first_y, _)) = cells.first() else {
        return 0;
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first_x, first_y, first_x, first_y);
    for &(x, y, _) in &cells {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    // Room for the erosion disc and the last cell's far corners.
    let pad = ERODE_RADIUS + 1;
    let (x0, y0) = (x0 - pad, y0 - pad);
    let (w, h) = ((x1 + pad - x0 + 1) as usize, (y1 + pad - y0 + 1) as usize);
    let mut strength = vec![0.0f32; w * h];
    for &(x, y, s) in &cells {
        strength[(y - y0) as usize * w + (x - x0) as usize] = s;
    }
    let mut grid = Grid::load(dim, x0, y0, w, h);
    grid.rain(
        drop_count(opts.droplets, w * h, opts.seed),
        opts.seed,
        |ix, iy| strength[iy * w + ix] > 0.0,
    );
    grid.apply(dim, |ix, iy| strength[iy * w + ix])
}

/// One axis of the window layout: `count` cores of equal size, each window
/// reaching `MARGIN` past its core into its neighbours.
struct Axis {
    start: i32,
    end: i32,
    count: i32,
}

impl Axis {
    fn new(start: i32, end: i32) -> Axis {
        // Cores stay at least CORE / 2 wide, which is more than two
        // margins, so windows two apart never overlap.
        let count = ((end - start + CORE - 1) / CORE).max(1);
        Axis { start, end, count }
    }

    /// Where core `k` begins; `boundary(count)` is the end of the axis.
    fn boundary(&self, k: i32) -> i32 {
        self.start + ((self.end - self.start) as i64 * k as i64 / self.count as i64) as i32
    }

    /// Half-open range window `k` covers.
    fn window(&self, k: i32) -> (i32, i32) {
        (
            (self.boundary(k) - MARGIN).max(self.start),
            (self.boundary(k + 1) + MARGIN).min(self.end),
        )
    }

    /// How much window `k` contributes at `x`. Across a boundary one window
    /// fades out as the next fades in, and the two always sum to 1.
    fn weight(&self, k: i32, x: i32) -> f32 {
        let span = 2.0 * MARGIN as f32;
        let fade_in = |b: i32| ((x - (b - MARGIN)) as f32 + 0.5).clamp(0.0, span) / span;
        let mut w = 1.0;
        if k > 0 {
            w *= fade_in(self.boundary(k));
        }
        if k + 1 < self.count {
            w *= 1.0 - fade_in(self.boundary(k + 1));
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimension::DimensionKind;

    /// Rolling hills of `tiles`×`tiles` tiles starting at tile (-1, -1),
    /// from about y=34 to y=106, with sea level at 62. `seed` shifts them.
    fn hills(tiles: i32, seed: u64) -> Dimension {
        let shift = seed as f32 * 17.0;
        let mut d = Dimension::new(DimensionKind::Overworld, -64, 320, seed);
        for ty in -1..tiles - 1 {
            for tx in -1..tiles - 1 {
                let t = d.ensure_tile(tx, ty);
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let x = (tx * TILE_SIZE + lx) as f32 + shift;
                        let y = (ty * TILE_SIZE + ly) as f32;
                        let big = (x / 40.0).sin() * (y / 40.0).cos();
                        let small = (x / 9.0 + y / 13.0).sin();
                        t.set_height(lx, ly, 70.0 + big * 30.0 + small * 6.0);
                        t.set_water_level(lx, ly, 62);
                    }
                }
            }
        }
        d
    }

    fn heights(d: &Dimension) -> Vec<f32> {
        let e = d.extent().unwrap();
        let mut all = Vec::new();
        for y in e.y0..=e.y1 {
            for x in e.x0..=e.x1 {
                all.push(d.height_at(x, y).unwrap());
            }
        }
        all
    }

    #[test]
    fn carves_and_fills() {
        let before = hills(2, 7);
        let mut d = before.clone();
        let n = erode(&mut d, None, &ErosionOptions::default());
        assert!(n > 5000, "only {n} columns changed");
        let (b, a) = (heights(&before), heights(&d));
        let lowered = b.iter().zip(&a).filter(|(b, a)| a < b).count();
        let raised = b.iter().zip(&a).filter(|(b, a)| a > b).count();
        assert!(
            lowered > 1000 && raised > 1000,
            "{lowered} down, {raised} up"
        );
        // Weathering, not demolition: nothing leaves the original range by much.
        let low = b.iter().copied().fold(f32::MAX, f32::min);
        let high = b.iter().copied().fold(f32::MIN, f32::max);
        assert!(a.iter().all(|&h| h > low - 5.0 && h < high + 5.0));
    }

    #[test]
    fn same_seed_same_result_whatever_the_threads() {
        // Wider than one window, so the blending and phases are exercised.
        let base = hills(10, 3);
        let opts = |threads: usize, seed: u64| ErosionOptions {
            droplets: 0.05,
            seed,
            threads: Some(threads),
        };
        let (mut a, mut b, mut c) = (base.clone(), base.clone(), base.clone());
        assert!(erode(&mut a, None, &opts(1, 11)) > 0);
        erode(&mut b, None, &opts(4, 11));
        erode(&mut c, None, &opts(1, 12));
        assert!(heights(&a) == heights(&b), "thread count changed the result");
        assert!(heights(&a) != heights(&c), "the seed made no difference");
    }

    #[test]
    fn window_weights_sum_to_one() {
        let axis = Axis::new(-700, 2900);
        assert!(axis.count > 2);
        for x in -700..2900 {
            let total: f32 = (0..axis.count)
                .filter(|&k| (axis.window(k).0..axis.window(k).1).contains(&x))
                .map(|k| axis.weight(k, x))
                .sum();
            assert!((total - 1.0).abs() < 1e-5, "{total} at {x}");
        }
        // Windows two apart must not overlap.
        for k in 0..axis.count - 2 {
            assert!(axis.window(k).1 <= axis.window(k + 2).0);
        }
    }

    #[test]
    fn flat_land_and_sea_floor_stay_put() {
        let mut flat = Dimension::new(DimensionKind::Overworld, -64, 320, 1);
        let t = flat.ensure_tile(0, 0);
        for y in 0..TILE_SIZE {
            for x in 0..TILE_SIZE {
                t.set_height(x, y, 80.0);
            }
        }
        assert_eq!(erode(&mut flat, None, &ErosionOptions::default()), 0);

        // Under water nothing is dug away; sediment only settles.
        let before = hills(2, 5);
        let mut d = before.clone();
        erode(&mut d, None, &ErosionOptions::default());
        let e = d.extent().unwrap();
        let mut deep = 0;
        for y in e.y0..=e.y1 {
            for x in e.x0..=e.x1 {
                let b = before.height_at(x, y).unwrap();
                if b < 55.0 {
                    deep += 1;
                    assert!(d.height_at(x, y).unwrap() >= b, "sea floor dug at {x},{y}");
                }
            }
        }
        assert!(deep > 100, "test world has no deep water");
    }

    #[test]
    fn brush_stays_inside_its_shape() {
        let before = hills(4, 9);
        let mut d = before.clone();
        let shape = Shape::Circle {
            x: 0,
            y: 0,
            radius: 40.0,
        };
        let opts = ErosionOptions {
            droplets: 2.0,
            ..Default::default()
        };
        assert!(erode_shape(&mut d, shape, Falloff::Smooth, &opts) > 100);
        let e = d.extent().unwrap();
        for y in e.y0..=e.y1 {
            for x in e.x0..=e.x1 {
                if x * x + y * y > 41 * 41 {
                    assert_eq!(d.height_at(x, y), before.height_at(x, y), "{x},{y}");
                }
            }
        }
        // Tiles the brush didn't reach are still shared with the original.
        assert!(std::sync::Arc::ptr_eq(
            d.tile_handle(2, 2).unwrap(),
            before.tile_handle(2, 2).unwrap()
        ));
    }
}

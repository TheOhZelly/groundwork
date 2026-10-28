//! Seeded gradient noise for terrain generation and texture variation.
//!
//! Plays the role of `org.pepsoft.util.PerlinNoise`. Output is deterministic
//! for a given seed but is not bit-identical to the Java implementation.

#[derive(Debug, Clone)]
pub struct PerlinNoise {
    perm: [u8; 512],
}

/// SplitMix64, used for seeding and cheap per-column hashing.
pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Deterministic per-column value in `0.0..1.0`.
pub fn hash01(seed: u64, x: i32, y: i32) -> f32 {
    let h = splitmix64(seed ^ ((x as u32 as u64) << 32 | y as u32 as u64));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

impl PerlinNoise {
    pub fn new(seed: u64) -> Self {
        let mut p: [u8; 256] = std::array::from_fn(|i| i as u8);
        let mut state = seed;
        for i in (1..256).rev() {
            state = splitmix64(state);
            p.swap(i, (state % (i as u64 + 1)) as usize);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        PerlinNoise { perm }
    }

    /// Improved 3D Perlin noise scaled to roughly `-0.5..0.5`, the range
    /// WorldPainter's `PerlinNoise.getPerlinNoise(x, y, z)` formulas assume.
    pub fn noise3(&self, x: f64, y: f64, z: f64) -> f32 {
        let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
        let (xi, yi, zi) = (
            (xf as i64 & 255) as usize,
            (yf as i64 & 255) as usize,
            (zf as i64 & 255) as usize,
        );
        let (x, y, z) = (x - xf, y - yf, z - zf);
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let p = &self.perm;
        let a = p[xi] as usize + yi;
        let (aa, ab) = (p[a] as usize + zi, p[a + 1] as usize + zi);
        let b = p[xi + 1] as usize + yi;
        let (ba, bb) = (p[b] as usize + zi, p[b + 1] as usize + zi);
        let n = lerp(
            w,
            lerp(
                v,
                lerp(u, grad3(p[aa], x, y, z), grad3(p[ba], x - 1.0, y, z)),
                lerp(
                    u,
                    grad3(p[ab], x, y - 1.0, z),
                    grad3(p[bb], x - 1.0, y - 1.0, z),
                ),
            ),
            lerp(
                v,
                lerp(
                    u,
                    grad3(p[aa + 1], x, y, z - 1.0),
                    grad3(p[ba + 1], x - 1.0, y, z - 1.0),
                ),
                lerp(
                    u,
                    grad3(p[ab + 1], x, y - 1.0, z - 1.0),
                    grad3(p[bb + 1], x - 1.0, y - 1.0, z - 1.0),
                ),
            ),
        );
        (n * 0.5) as f32
    }

    /// Classic 2D Perlin noise, roughly in `-1.0..1.0`.
    pub fn noise(&self, x: f64, y: f64) -> f64 {
        let (xf, yf) = (x.floor(), y.floor());
        let (xi, yi) = ((xf as i64 & 255) as usize, (yf as i64 & 255) as usize);
        let (x, y) = (x - xf, y - yf);
        let (u, v) = (fade(x), fade(y));
        let p = &self.perm;
        let (a, b) = (p[xi] as usize + yi, p[xi + 1] as usize + yi);
        lerp(
            v,
            lerp(u, grad(p[a], x, y), grad(p[b], x - 1.0, y)),
            lerp(
                u,
                grad(p[a + 1], x, y - 1.0),
                grad(p[b + 1], x - 1.0, y - 1.0),
            ),
        )
    }

    /// Fractal sum of `octaves` layers, normalised to roughly `-1.0..1.0`.
    pub fn fbm(&self, x: f64, y: f64, octaves: u32, persistence: f64) -> f64 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for octave in 0..octaves {
            // Offset each octave so they don't share a lattice origin.
            let o = octave as f64 * 37.13;
            sum += self.noise(x * freq + o, y * freq - o) * amp;
            norm += amp;
            amp *= persistence;
            freq *= 2.0;
        }
        sum / norm
    }
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

fn grad(hash: u8, x: f64, y: f64) -> f64 {
    match hash & 7 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x,
        5 => -x,
        6 => y,
        _ => -y,
    }
}

fn grad3(hash: u8, x: f64, y: f64, z: f64) -> f64 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// WorldPainter's blob sizes (`org.pepsoft.worldpainter.Constants`): noise
/// coordinates are divided by these.
pub const TINY_BLOBS: f64 = 4.099;
pub const SMALL_BLOBS: f64 = 16.411;
pub const MEDIUM_BLOBS: f64 = 32.771;
pub const LARGE_BLOBS: f64 = 65.537;
pub const HUGE_BLOBS: f64 = 131.101;
pub const GIGANTIC_BLOBS: f64 = 524.309;

/// Equivalent of `PerlinNoise.getLevelForPromillage`: the [`PerlinNoise::noise3`]
/// value exceeded with a probability of `promille` / 1000. Derived from the
/// sampled distribution of this noise, so rarities match WorldPainter even
/// though the noise itself differs.
pub fn level_for_promille(promille: f32) -> f32 {
    static SORTED: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let sorted = SORTED.get_or_init(|| {
        let noise = PerlinNoise::new(0x5eed);
        let mut state = 1u64;
        let mut values: Vec<f32> = (0..200_000)
            .map(|_| {
                let mut coord = || {
                    state = splitmix64(state);
                    (state >> 11) as f64 / (1u64 << 53) as f64 * 256.0
                };
                noise.noise3(coord(), coord(), coord())
            })
            .collect();
        values.sort_by(f32::total_cmp);
        values
    });
    if promille <= 0.0 {
        return f32::INFINITY;
    }
    if promille >= 1000.0 {
        return f32::NEG_INFINITY;
    }
    let index = ((1.0 - promille / 1000.0) * (sorted.len() - 1) as f32) as usize;
    sorted[index]
}

/// `java.util.Random`, bit for bit, so per-column choices that WorldPainter
/// makes with `new Random(seed + x * 65537 + y * 4099)` come out the same.
#[derive(Debug, Clone)]
pub struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5_DEEC_E66D;
    const MASK: u64 = (1 << 48) - 1;

    pub fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed as u64 ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    /// Java's per-column seeding: `new Random(seed + x * 65537L + y * 4099L)`.
    pub fn for_column(seed: u64, x: i32, y: i32) -> Self {
        Self::new(
            (seed as i64)
                .wrapping_add(x as i64 * 65537)
                .wrapping_add(y as i64 * 4099),
        )
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(Self::MULTIPLIER).wrapping_add(0xB)) & Self::MASK;
        (self.seed >> (48 - bits)) as i64 as i32
    }

    pub fn next_int(&mut self, bound: i32) -> i32 {
        assert!(bound > 0);
        if (bound as u32).is_power_of_two() {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }

    pub fn next_long(&mut self) -> i64 {
        ((self.next(32) as i64) << 32).wrapping_add(self.next(32) as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        let a = PerlinNoise::new(42);
        let b = PerlinNoise::new(42);
        let c = PerlinNoise::new(43);
        let mut differs = false;
        for i in 0..1000 {
            let (x, y) = (i as f64 * 0.37, i as f64 * 0.11);
            let v = a.fbm(x, y, 4, 0.5);
            assert_eq!(v, b.fbm(x, y, 4, 0.5));
            assert!((-1.5..=1.5).contains(&v));
            differs |= v != c.fbm(x, y, 4, 0.5);
        }
        assert!(differs);
        assert_eq!(a.noise(3.0, 7.0), 0.0); // zero on lattice points
    }
}

#[cfg(test)]
mod java_tests {
    use super::*;

    #[test]
    fn matches_java_util_random() {
        // Reference values printed by java.util.Random on JDK 21.
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_int(10), 0);
        assert_eq!(r.next_int(10), 3);
        assert_eq!(r.next_int(100), 48);
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_long(), -5025562857975149833);
        let mut r = JavaRandom::new(0);
        assert_eq!(r.next_float(), 0.73096776);
        // new Random(20261028 + 5 * 65537 + -7 * 4099)
        let mut c = JavaRandom::for_column(20261028, 5, -7);
        assert_eq!(c.next_int(1000), 740);
        assert_eq!(c.next_float(), 0.962195);
        assert_eq!(c.next_int(7), 2);
    }

    #[test]
    fn promille_levels_are_ordered() {
        let rare = level_for_promille(1.0);
        let common = level_for_promille(100.0);
        assert!(rare > common && common > 0.0, "{rare} {common}");
        assert_eq!(level_for_promille(0.0), f32::INFINITY);
    }
}

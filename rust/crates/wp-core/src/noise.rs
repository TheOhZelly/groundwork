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

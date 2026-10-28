//! New-world generation. Plays the role of `HeightMapTileFactory` with a
//! `SimpleTheme`: a noise height map, one terrain type, optional beaches and
//! a uniform water level.

use crate::dimension::{Dimension, DimensionKind};
use crate::noise::PerlinNoise;
use crate::terrain::Terrain;
use crate::tile::{Tile, TILE_SIZE};

#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub kind: DimensionKind,
    /// Size in tiles (128 blocks each), centred on the origin.
    pub width_tiles: u32,
    pub length_tiles: u32,
    pub min_height: i32,
    pub max_height: i32,
    /// Average terrain height.
    pub base_height: f32,
    pub water_level: i32,
    /// Maximum deviation from `base_height`, in blocks. 0 gives a flat world.
    pub hilliness: f32,
    /// Horizontal size of hills, in blocks.
    pub feature_size: f32,
    pub terrain: Terrain,
    /// Put Beaches terrain around the water line, as Java's SimpleTheme does.
    pub beaches: bool,
    pub seed: u64,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        GenerateOptions {
            kind: DimensionKind::Overworld,
            width_tiles: 4,
            length_tiles: 4,
            min_height: -64,
            max_height: 320,
            base_height: 64.0,
            water_level: 62,
            hilliness: 20.0,
            feature_size: 200.0,
            terrain: Terrain::Grass,
            beaches: true,
            seed: 0,
        }
    }
}

impl GenerateOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.width_tiles == 0 || self.length_tiles == 0 {
            return Err("world must be at least 1×1 tiles".into());
        }
        if self.max_height <= self.min_height {
            return Err("max_height must be greater than min_height".into());
        }
        if self.min_height % 16 != 0 || self.max_height % 16 != 0 {
            return Err("min_height and max_height must be multiples of 16".into());
        }
        let range = self.min_height as f32..self.max_height as f32;
        if !range.contains(&self.base_height) {
            return Err(format!(
                "base_height {} outside {}..{}",
                self.base_height, self.min_height, self.max_height
            ));
        }
        if self.feature_size <= 0.0 {
            return Err("feature_size must be positive".into());
        }
        Ok(())
    }
}

pub fn generate(opts: &GenerateOptions) -> Result<Dimension, String> {
    opts.validate()?;
    let mut dim = Dimension::new(opts.kind, opts.min_height, opts.max_height, opts.seed);
    let noise = PerlinNoise::new(opts.seed);
    let (w, l) = (opts.width_tiles as i32, opts.length_tiles as i32);
    for ty in -l / 2..l - l / 2 {
        for tx in -w / 2..w - w / 2 {
            let mut tile = Tile::new(tx, ty, opts.min_height, opts.max_height);
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                    let n = if opts.hilliness == 0.0 {
                        0.0
                    } else {
                        let s = opts.feature_size as f64;
                        noise.fbm(x as f64 / s, y as f64 / s, 5, 0.5) as f32
                    };
                    let height = opts.base_height + n * opts.hilliness;
                    tile.set_height(lx, ly, height);
                    tile.set_water_level(lx, ly, opts.water_level);
                    let h = tile.int_height(lx, ly);
                    let beach =
                        opts.beaches && (opts.water_level - 2..=opts.water_level + 1).contains(&h);
                    tile.set_terrain(
                        lx,
                        ly,
                        if beach {
                            Terrain::Beaches
                        } else {
                            opts.terrain
                        },
                    );
                }
            }
            dim.insert_tile(tile);
        }
    }
    Ok(dim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_centred_tiles() {
        let dim = generate(&GenerateOptions {
            width_tiles: 3,
            length_tiles: 2,
            seed: 7,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(dim.tile_count(), 6);
        let e = dim.extent().unwrap();
        assert_eq!((e.x0, e.x1, e.y0, e.y1), (-128, 255, -128, 127));
        let h = dim.height_at(0, 0).unwrap();
        assert!((44.0..=84.0).contains(&h), "{h}");
    }

    #[test]
    fn flat_world_with_beaches() {
        let opts = GenerateOptions {
            hilliness: 0.0,
            base_height: 62.0,
            width_tiles: 1,
            length_tiles: 1,
            ..Default::default()
        };
        let dim = generate(&opts).unwrap();
        assert_eq!(dim.height_at(5, 5), Some(62.0));
        assert_eq!(dim.terrain_at(5, 5), Some(Terrain::Beaches));
    }

    #[test]
    fn rejects_bad_options() {
        assert!(generate(&GenerateOptions {
            min_height: 0,
            max_height: 0,
            ..Default::default()
        })
        .is_err());
        assert!(generate(&GenerateOptions {
            base_height: 1000.0,
            ..Default::default()
        })
        .is_err());
    }
}

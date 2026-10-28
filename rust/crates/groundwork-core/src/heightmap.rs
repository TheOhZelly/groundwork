//! Height map images: import a greyscale image as terrain, or export a
//! dimension's heights. Plays the role of WorldPainter's height map import
//! (`HeightMapImporter`) and export.
//!
//! Exports are 16-bit greyscale PNGs where black is the bottom of the world
//! and white the top, so importing with the default range round-trips.

use crate::dimension::Dimension;
use crate::terrain::Terrain;
use crate::tile::{TILE_SIZE, TILE_SIZE_BITS};
use std::path::Path;

/// A greyscale image as values in `0.0..=1.0`, row-major.
pub struct HeightImage {
    pub width: u32,
    pub height: u32,
    pub values: Vec<f32>,
}

impl HeightImage {
    /// Decodes a PNG in any colour type and 8 or 16 bits; colour images use
    /// their luminance.
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        let mut decoder = png::Decoder::new(bytes);
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("not a readable PNG: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("corrupt PNG: {e}"))?;
        let (w, h) = (info.width, info.height);
        let channels = info.color_type.samples();
        let sixteen = info.bit_depth == png::BitDepth::Sixteen;
        let sample = |i: usize| -> f32 {
            if sixteen {
                u16::from_be_bytes([buf[i * 2], buf[i * 2 + 1]]) as f32 / 65535.0
            } else {
                buf[i] as f32 / 255.0
            }
        };
        let values = (0..(w * h) as usize)
            .map(|p| {
                let base = p * channels;
                match channels {
                    1 | 2 => sample(base),
                    _ => {
                        0.2126 * sample(base)
                            + 0.7152 * sample(base + 1)
                            + 0.0722 * sample(base + 2)
                    }
                }
            })
            .collect();
        Ok(HeightImage {
            width: w,
            height: h,
            values,
        })
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::from_png(&bytes)
    }

    /// Bilinear sample at pixel coordinates (clamped to the image).
    fn sample(&self, u: f32, v: f32) -> f32 {
        let u = u.clamp(0.0, (self.width - 1) as f32);
        let v = v.clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (u.floor() as u32, v.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        let at = |x: u32, y: u32| self.values[(y * self.width + x) as usize];
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// World block of the image's top-left pixel.
    pub x: i32,
    pub y: i32,
    /// Blocks per image pixel; above 1 upscales smoothly.
    pub scale: f32,
    /// Heights for black and white.
    pub low: f32,
    pub high: f32,
    /// Terrain and water level for tiles the import creates.
    pub terrain: Terrain,
    pub water_level: i32,
}

/// Sets heights from an image, creating any missing tiles it covers.
/// Returns the number of columns changed.
pub fn import(dim: &mut Dimension, image: &HeightImage, options: &ImportOptions) -> usize {
    let scale = options.scale.max(0.01);
    let (w, h) = (
        (image.width as f32 * scale).round() as i32,
        (image.height as f32 * scale).round() as i32,
    );
    let (x0, y0, x1, y1) = (options.x, options.y, options.x + w - 1, options.y + h - 1);
    let mut changed = 0;
    for ty in (y0 >> TILE_SIZE_BITS)..=(y1 >> TILE_SIZE_BITS) {
        for tx in (x0 >> TILE_SIZE_BITS)..=(x1 >> TILE_SIZE_BITS) {
            let fresh = dim.tile(tx, ty).is_none();
            let tile = dim.ensure_tile(tx, ty);
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                    let inside = (x0..=x1).contains(&x) && (y0..=y1).contains(&y);
                    if fresh {
                        tile.set_terrain(lx, ly, options.terrain);
                        tile.set_water_level(lx, ly, options.water_level);
                    }
                    if inside {
                        // Pixel centres sit at (i + 0.5) / scale.
                        let u = (x - x0) as f32 / scale + 0.5 / scale - 0.5;
                        let v = (y - y0) as f32 / scale + 0.5 / scale - 0.5;
                        let value = image.sample(u, v);
                        tile.set_height(lx, ly, options.low + value * (options.high - options.low));
                        changed += 1;
                    } else if fresh {
                        // Outside the image in a new tile: the lowest height.
                        tile.set_height(lx, ly, options.low);
                    }
                }
            }
            tile.compact();
        }
    }
    changed
}

/// The dimension's heights as a 16-bit greyscale PNG covering its tiles:
/// black is `min_height`, white `max_height - 1`; missing tiles are black.
pub fn export_png(dim: &Dimension) -> Result<Vec<u8>, String> {
    let extent = dim.extent().ok_or("the world has no tiles")?;
    let (w, h) = (extent.width() as u32, extent.height() as u32);
    let range = (dim.max_height - 1 - dim.min_height) as f32;
    let mut data = Vec::with_capacity((w * h * 2) as usize);
    for y in extent.y0..=extent.y1 {
        for x in extent.x0..=extent.x1 {
            let v = dim
                .height_at(x, y)
                .map_or(0.0, |height| (height - dim.min_height as f32) / range);
            data.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes());
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&data))
        .map_err(|e| format!("PNG encoding failed: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimension::DimensionKind;
    use crate::generate::{generate, GenerateOptions};

    #[test]
    fn export_then_import_round_trips() {
        let opts = GenerateOptions {
            width_tiles: 2,
            length_tiles: 1,
            seed: 21,
            ..Default::default()
        };
        let original = generate(&opts).unwrap();
        let png = export_png(&original).unwrap();
        let image = HeightImage::from_png(&png).unwrap();
        assert_eq!((image.width, image.height), (256, 128));

        let mut copy = Dimension::new(DimensionKind::Overworld, -64, 320, 21);
        let extent = original.extent().unwrap();
        let options = ImportOptions {
            x: extent.x0,
            y: extent.y0,
            scale: 1.0,
            low: -64.0,
            high: 319.0,
            terrain: Terrain::Grass,
            water_level: 62,
        };
        assert_eq!(import(&mut copy, &image, &options), 256 * 128);
        for (x, y) in [(-128, 0), (0, 0), (100, 50), (127, 127)] {
            let (a, b) = (
                original.height_at(x, y).unwrap(),
                copy.height_at(x, y).unwrap(),
            );
            // 16 bits over 383 blocks: better than 1/100 block.
            assert!((a - b).abs() < 0.01, "{x},{y}: {a} vs {b}");
        }
        assert_eq!(copy.water_level_at(5, 5), Some(62));
    }

    #[test]
    fn imports_8_bit_images_scaled() {
        // A 2×1 image, black then white, written as 8-bit greyscale.
        let mut png_bytes = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png_bytes, 2, 1);
            e.set_color(png::ColorType::Grayscale);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[0, 255])
                .unwrap();
        }
        let image = HeightImage::from_png(&png_bytes).unwrap();
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320, 1);
        let options = ImportOptions {
            x: 10,
            y: 20,
            scale: 64.0,
            low: 60.0,
            high: 100.0,
            terrain: Terrain::Sand,
            water_level: -64,
        };
        // 128 × 64 blocks.
        assert_eq!(import(&mut dim, &image, &options), 128 * 64);
        assert_eq!(dim.height_at(10, 20), Some(60.0));
        assert_eq!(dim.height_at(137, 20), Some(100.0));
        let mid = dim.height_at(74, 40).unwrap();
        assert!(mid > 60.0 && mid < 100.0, "{mid}");
        assert_eq!(dim.terrain_at(10, 20), Some(Terrain::Sand));
        // Columns of new tiles outside the image sit at the low height.
        assert_eq!(dim.height_at(0, 0), Some(60.0));
    }
}

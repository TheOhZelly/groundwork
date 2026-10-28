//! Top-down preview rendering, the equivalent of `TileRenderer` with the
//! default colour scheme and hill shading.

use crate::dimension::Dimension;
use crate::layers;
use crate::terrain::Terrain;

pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA, row-major.
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn to_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&self.pixels)?;
        Ok(out)
    }
}

/// Approximate map colour of a terrain's surface block.
pub fn terrain_colour(terrain: Terrain) -> [u8; 3] {
    let block = terrain.info().top.unwrap_or("minecraft:stone");
    match block.trim_start_matches("minecraft:") {
        "grass_block" => [0x5f, 0x9f, 0x35],
        "dirt" | "coarse_dirt" | "dirt_path" => [0x86, 0x60, 0x43],
        "podzol" => [0x5a, 0x3d, 0x1f],
        "mycelium" => [0x6f, 0x63, 0x69],
        "sand" => [0xdb, 0xd3, 0xa0],
        "red_sand" => [0xbe, 0x66, 0x21],
        "sandstone" => [0xd8, 0xcb, 0x9b],
        "red_sandstone" => [0xb5, 0x62, 0x1f],
        "gravel" => [0x83, 0x7f, 0x7e],
        "clay" => [0xa0, 0xa6, 0xb3],
        "stone" | "cobblestone" | "andesite" => [0x7d, 0x7d, 0x7d],
        "mossy_cobblestone" | "moss_block" => [0x59, 0x6d, 0x2d],
        "pale_moss_block" => [0x6b, 0x71, 0x5f],
        "granite" => [0x95, 0x67, 0x55],
        "diorite" | "calcite" => [0xbc, 0xbc, 0xbc],
        "deepslate" | "basalt" | "blackstone" => [0x4d, 0x4d, 0x50],
        "tuff" => [0x6c, 0x6d, 0x66],
        "water" => [0x3f, 0x76, 0xe4],
        "lava" | "magma_block" => [0xd9, 0x6a, 0x14],
        "snow_block" => [0xf0, 0xfb, 0xfb],
        "netherrack" | "crimson_nylium" => [0x72, 0x36, 0x32],
        "warped_nylium" => [0x2b, 0x72, 0x65],
        "soul_sand" | "soul_soil" => [0x51, 0x3e, 0x32],
        "obsidian" => [0x14, 0x12, 0x1e],
        "bedrock" => [0x55, 0x55, 0x55],
        "end_stone" => [0xdb, 0xde, 0x9e],
        "mud" => [0x3c, 0x39, 0x3d],
        "terracotta" => [0x98, 0x5e, 0x43],
        b if b.ends_with("_terracotta") => terracotta(b),
        _ => [0x7d, 0x7d, 0x7d],
    }
}

fn terracotta(block: &str) -> [u8; 3] {
    match block.trim_end_matches("_terracotta") {
        "white" => [0xd1, 0xb2, 0xa1],
        "orange" => [0xa1, 0x53, 0x25],
        "magenta" => [0x95, 0x57, 0x6c],
        "light_blue" => [0x71, 0x6c, 0x89],
        "yellow" => [0xba, 0x85, 0x23],
        "lime" => [0x67, 0x75, 0x34],
        "pink" => [0xa1, 0x4e, 0x4e],
        "gray" => [0x39, 0x29, 0x23],
        "light_gray" => [0x87, 0x6a, 0x61],
        "cyan" => [0x56, 0x5a, 0x5a],
        "purple" => [0x76, 0x46, 0x56],
        "blue" => [0x4a, 0x3b, 0x5b],
        "brown" => [0x4d, 0x33, 0x23],
        "green" => [0x4b, 0x52, 0x2a],
        "red" => [0x8e, 0x3c, 0x2e],
        _ => [0x25, 0x16, 0x10],
    }
}

/// Renders the dimension's tiled area. `scale` is blocks per pixel (≥1);
/// it is increased automatically so that neither side exceeds `max_side`.
/// Missing tiles are transparent.
pub fn render(dim: &Dimension, scale: u32, max_side: u32) -> Option<(Image, u32)> {
    let extent = dim.extent()?;
    let (w, h) = (extent.width() as u32, extent.height() as u32);
    let scale = scale
        .max(1)
        .max(w.div_ceil(max_side.max(1)))
        .max(h.div_ceil(max_side.max(1)));
    let (iw, ih) = (w.div_ceil(scale), h.div_ceil(scale));
    let mut pixels = vec![0u8; (iw * ih * 4) as usize];
    for py in 0..ih {
        for px in 0..iw {
            let x = extent.x0 + (px * scale) as i32;
            let y = extent.y0 + (py * scale) as i32;
            if let Some(rgb) = column_colour(dim, x, y, scale as i32) {
                let i = ((py * iw + px) * 4) as usize;
                pixels[i..i + 3].copy_from_slice(&rgb);
                pixels[i + 3] = 255;
            }
        }
    }
    Some((
        Image {
            width: iw,
            height: ih,
            pixels,
        },
        scale,
    ))
}

fn column_colour(dim: &Dimension, x: i32, y: i32, step: i32) -> Option<[u8; 3]> {
    let height = dim.height_at(x, y)?;
    if dim.layer_value_at(layers::VOID, x, y) == Some(1) {
        return Some([0, 0, 0]);
    }
    let water = dim.water_level_at(x, y)?;
    let frost = dim.layer_value_at(layers::FROST, x, y) == Some(1);
    let int_height = crate::java_round(height);

    let mut rgb = terrain_colour(dim.terrain_at(x, y)?).map(f32::from);
    // Hill shading from the north-west, like WorldPainter's default view.
    let west = dim.height_at(x - step, y).unwrap_or(height);
    let north = dim.height_at(x, y - step).unwrap_or(height);
    let shade = (1.0 + ((height - west) + (height - north)) / step as f32 * 0.15).clamp(0.6, 1.4);
    rgb = rgb.map(|c| c * shade);

    if water > int_height {
        let lava = dim.layer_value_at(layers::FLOOD_WITH_LAVA, x, y) == Some(1);
        let fluid: [f32; 3] = if lava {
            [217.0, 106.0, 20.0]
        } else {
            [48.0, 92.0, 200.0]
        };
        let depth = (water - int_height) as f32;
        let alpha = if lava {
            1.0
        } else {
            (0.55 + depth * 0.03).min(0.92)
        };
        rgb = [0, 1, 2].map(|i| rgb[i] * (1.0 - alpha) + fluid[i] * alpha);
        if frost && !lava {
            rgb = rgb.map(|c| c * 0.3 + 200.0 * 0.7);
        }
    } else if frost {
        rgb = rgb.map(|c| c * 0.2 + 245.0 * 0.8);
    }
    Some(rgb.map(|c| c.clamp(0.0, 255.0) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::{generate, GenerateOptions};

    #[test]
    fn renders_png() {
        let dim = generate(&GenerateOptions {
            width_tiles: 2,
            length_tiles: 1,
            ..Default::default()
        })
        .unwrap();
        let (img, scale) = render(&dim, 1, 128).unwrap();
        assert_eq!(scale, 2);
        assert_eq!((img.width, img.height), (128, 64));
        let png = img.to_png().unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}

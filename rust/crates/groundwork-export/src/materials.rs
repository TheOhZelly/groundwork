//! Mixed terrains and their surface plants.
//!
//! Port of the `getMaterial` and `getSurfaceObject` overrides in
//! `org.pepsoft.worldpainter.Terrain`. Per-column random choices use a
//! bit-exact `java.util.Random`, so flowers, cacti and Mesa bands land where
//! WorldPainter puts them. Noise patterns are equivalent rather than
//! identical: thresholds come from Groundwork's own noise distribution, so
//! each material keeps WorldPainter's rarity.
//!
//! Coordinates follow WorldPainter: `x` and `y` horizontal, `z` vertical.

use groundwork_core::noise::{
    level_for_promille, splitmix64, JavaRandom, PerlinNoise, GIGANTIC_BLOBS, HUGE_BLOBS,
    LARGE_BLOBS, SMALL_BLOBS, TINY_BLOBS,
};
use groundwork_core::Terrain;

/// Something placed on top of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plant {
    /// One block above the ground (on land or in water).
    Single(&'static str),
    /// A two-block plant with `half=lower` / `half=upper` states.
    Double(&'static str),
    /// A stack of the same block, e.g. a cactus.
    Stack(&'static str, i32),
    /// Kelp of this many blocks under water.
    Kelp(i32),
}

const FLOWERS: [Plant; 16] = [
    Plant::Single("minecraft:dandelion"),
    Plant::Single("minecraft:poppy"),
    Plant::Single("minecraft:blue_orchid"),
    Plant::Single("minecraft:allium"),
    Plant::Single("minecraft:azure_bluet"),
    Plant::Single("minecraft:red_tulip"),
    Plant::Single("minecraft:orange_tulip"),
    Plant::Single("minecraft:white_tulip"),
    Plant::Single("minecraft:pink_tulip"),
    Plant::Single("minecraft:oxeye_daisy"),
    Plant::Double("minecraft:sunflower"),
    Plant::Double("minecraft:lilac"),
    Plant::Double("minecraft:rose_bush"),
    Plant::Double("minecraft:peony"),
    // Again, to make them a bit more common, as in Java.
    Plant::Single("minecraft:dandelion"),
    Plant::Single("minecraft:poppy"),
];

const MESA_MATERIALS: [&str; 8] = [
    "minecraft:red_sand",
    "minecraft:terracotta",
    "minecraft:white_terracotta",
    "minecraft:light_gray_terracotta",
    "minecraft:yellow_terracotta",
    "minecraft:orange_terracotta",
    "minecraft:red_terracotta",
    "minecraft:brown_terracotta",
];
const MESA_LAYERS: usize = 64;

/// Noise generators and thresholds for one dimension seed.
pub struct Materials {
    seed: u64,
    stone: PerlinNoise,
    glowstone: PerlinNoise,
    soul_sand: PerlinNoise,
    lava: PerlinNoise,
    beach_sand: PerlinNoise,
    beach_clay: PerlinNoise,
    granite: PerlinNoise,
    diorite: PerlinNoise,
    andesite: PerlinNoise,
    mesa: PerlinNoise,
    mesa_layers: [&'static str; MESA_LAYERS],
    grass: PerlinNoise,
    tall_grass: PerlinNoise,
    dandelion: PerlinNoise,
    rose: PerlinNoise,
    grass_chance: f32,
    double_tall_grass_chance: f32,
    fern_chance: f32,
    flower_chance: f32,
    beach_sand_chance: f32,
    beach_gravel_chance: f32,
    beach_clay_chance: f32,
    stone_mix_chance: f32,
}

impl Materials {
    pub fn new(seed: u64) -> Self {
        // WorldPainter seeds each noise with the world seed plus an offset.
        let noise = |offset: u64| PerlinNoise::new(seed.wrapping_add(offset));
        let mut mesa_layers = ["minecraft:terracotta"; MESA_LAYERS];
        let mut random = JavaRandom::new(seed as i64);
        for _ in 0..MESA_LAYERS / 2 {
            let index = random.next_int(MESA_LAYERS as i32 - 1) as usize;
            let material = MESA_MATERIALS[random.next_int(MESA_MATERIALS.len() as i32) as usize];
            mesa_layers[index] = material;
            mesa_layers[index + 1] = material;
        }
        Materials {
            seed,
            stone: noise(188_434_540),
            glowstone: noise(57_861_047),
            soul_sand: noise(81_867_522),
            lava: noise(189_831_882),
            beach_sand: noise(26_796_036),
            beach_clay: noise(161_603_308),
            granite: noise(145_827_825),
            diorite: noise(59_606_124),
            andesite: noise(87_772_192),
            mesa: noise(110_335_839),
            mesa_layers,
            grass: noise(169_191_195),
            tall_grass: noise(31_695_680),
            dandelion: noise(145_351_781),
            rose: noise(28_286_488),
            grass_chance: level_for_promille(100.0),
            double_tall_grass_chance: level_for_promille(50.0),
            fern_chance: level_for_promille(10.0),
            flower_chance: level_for_promille(10.0),
            beach_sand_chance: level_for_promille(400.0) * 1.5,
            beach_gravel_chance: level_for_promille(200.0) * 1.5,
            beach_clay_chance: level_for_promille(40.0),
            stone_mix_chance: level_for_promille(45.0),
        }
    }

    /// The block a mixed terrain puts at depth `height - z` of its top layer,
    /// or `None` for terrains that use their plain top/under blocks.
    pub fn top_layer_block(
        &self,
        terrain: Terrain,
        x: i32,
        y: i32,
        z: i32,
        height: i32,
    ) -> Option<&'static str> {
        let (xf, yf, zf) = (x as f64, y as f64, z as f64);
        Some(match terrain {
            Terrain::Rock | Terrain::Snow => {
                if self
                    .stone
                    .noise3(xf / TINY_BLOBS, yf / TINY_BLOBS, zf / TINY_BLOBS)
                    > 0.0
                {
                    "minecraft:stone"
                } else {
                    "minecraft:cobblestone"
                }
            }
            Terrain::Netherlike => {
                let s = (xf / SMALL_BLOBS, yf / SMALL_BLOBS, zf / SMALL_BLOBS);
                if self.glowstone.noise3(s.0, s.1, s.2) > 0.4 {
                    "minecraft:glowstone"
                } else if self.soul_sand.noise3(s.0, s.1, s.2) > 0.4 {
                    "minecraft:soul_sand"
                } else if self.lava.noise3(s.0, s.1, s.2) > 0.4 {
                    "minecraft:lava"
                } else {
                    "minecraft:netherrack"
                }
            }
            // The deprecated Resources terrain is stone; its ores come from
            // the Resources layer like everywhere else.
            Terrain::Resources => "minecraft:stone",
            Terrain::Beaches | Terrain::BareBeaches => {
                let s = (xf / SMALL_BLOBS, yf / SMALL_BLOBS, zf / SMALL_BLOBS);
                if self.beach_clay.noise3(s.0, s.1, s.2) >= self.beach_clay_chance {
                    "minecraft:clay"
                } else {
                    let noise = self
                        .beach_sand
                        .noise3(xf / HUGE_BLOBS, yf / HUGE_BLOBS, s.2)
                        + self.beach_sand.noise3(s.0, s.1, s.2) / 2.0;
                    if noise >= self.beach_sand_chance {
                        "minecraft:sand"
                    } else if -noise >= self.beach_gravel_chance {
                        "minecraft:gravel"
                    } else {
                        "minecraft:grass_block"
                    }
                }
            }
            Terrain::Mesa => {
                let n = |scale: f64| self.mesa.noise3(xf / scale, yf / scale, 0.0);
                let offset = n(GIGANTIC_BLOBS) * 4.0
                    + n(HUGE_BLOBS)
                    + n(LARGE_BLOBS)
                    + n(SMALL_BLOBS) / 4.0
                    + 3.125;
                let band = (z as f32 + offset * 8.0) as i32;
                self.mesa_layers[band.rem_euclid(MESA_LAYERS as i32) as usize]
            }
            Terrain::StoneMix => {
                let s = (xf / SMALL_BLOBS, yf / SMALL_BLOBS, zf / SMALL_BLOBS);
                let chance = self.stone_mix_chance;
                // Java blends the two halves over y = -4..0 with an unstable
                // random; a column hash keeps it stable.
                let jitter = (splitmix64(self.seed ^ ((x as u32 as u64) << 32 | y as u32 as u64))
                    % 5) as i32;
                let upper = z >= 0 || z >= -jitter;
                let granite = self.granite.noise3(s.0, s.1, s.2) > chance;
                let diorite = !granite && self.diorite.noise3(s.0, s.1, s.2) > chance;
                let andesite = !granite && !diorite && self.andesite.noise3(s.0, s.1, s.2) > chance;
                match (upper, granite, diorite, andesite) {
                    (true, true, ..) => "minecraft:granite",
                    (true, _, true, _) => "minecraft:diorite",
                    (true, _, _, true) => "minecraft:andesite",
                    (true, ..) => "minecraft:stone",
                    (false, true, ..) => "minecraft:tuff",
                    (false, _, true, _) => "minecraft:deepslate[axis=x]",
                    (false, _, _, true) => "minecraft:deepslate[axis=z]",
                    (false, ..) => "minecraft:deepslate",
                }
            }
            Terrain::GrassPath => {
                if z == height {
                    "minecraft:dirt_path"
                } else {
                    "minecraft:grass_block"
                }
            }
            _ => return None,
        })
    }

    /// The plant (or snow, or fire) a terrain puts on top of a column, given
    /// how many blocks of water are above the ground.
    pub fn surface_plant(
        &self,
        terrain: Terrain,
        x: i32,
        y: i32,
        water_depth: i32,
    ) -> Option<Plant> {
        let mut random = JavaRandom::for_column(self.seed, x, y);
        match terrain {
            Terrain::Grass => {
                let roll = random.next_int(10);
                if water_depth > 0 {
                    self.seagrass(x, y, water_depth, &mut random)
                } else if roll == 0 {
                    let s = (
                        x as f64 / SMALL_BLOBS,
                        y as f64 / SMALL_BLOBS,
                        1.0 / SMALL_BLOBS,
                    );
                    let flowering = self.dandelion.noise3(s.0, s.1, s.2) > self.flower_chance
                        || self.rose.noise3(s.0, s.1, s.2) > self.flower_chance;
                    flowering.then(|| FLOWERS[self.flower_type(x, y)])
                } else {
                    self.grass(x, y, &mut random)
                }
            }
            Terrain::Beaches if water_depth > 0 => self.seagrass(x, y, water_depth, &mut random),
            Terrain::Desert | Terrain::RedDesert if water_depth == 0 => {
                let (incidence, shrub_below) = if terrain == Terrain::Desert {
                    (1000, 6)
                } else {
                    (2000, 12)
                };
                match random.next_int(incidence) {
                    n if n < 3 => Some(Plant::Stack("minecraft:cactus", n + 1)),
                    n if n < shrub_below => Some(Plant::Single("minecraft:dead_bush")),
                    _ => None,
                }
            }
            Terrain::Mesa if water_depth == 0 => {
                (random.next_int(500) < 3).then_some(Plant::Single("minecraft:dead_bush"))
            }
            Terrain::Netherlike if water_depth == 0 => {
                (random.next_int(150) == 0).then_some(Plant::Single("minecraft:fire"))
            }
            Terrain::Snow if water_depth == 0 => Some(Plant::Single("minecraft:snow")),
            _ => None,
        }
    }

    fn grass_value(&self, x: i32, y: i32, random: &mut JavaRandom) -> (f32, bool) {
        let s = (
            x as f64 / SMALL_BLOBS,
            y as f64 / SMALL_BLOBS,
            1.0 / SMALL_BLOBS,
        );
        let value = self.grass.noise3(s.0, s.1, s.2) + (random.next_float() * 0.3 - 0.15);
        let tall = self.tall_grass.noise3(s.0, s.1, s.2) > 0.0;
        (value, tall)
    }

    fn grass(&self, x: i32, y: i32, random: &mut JavaRandom) -> Option<Plant> {
        let (value, tall) = self.grass_value(x, y, random);
        if value <= self.grass_chance {
            return None;
        }
        Some(if tall {
            if value > self.double_tall_grass_chance {
                if random.next_int(4) == 0 {
                    Plant::Double("minecraft:large_fern")
                } else {
                    Plant::Double("minecraft:tall_grass")
                }
            } else if random.next_int(4) == 0 {
                Plant::Single("minecraft:fern")
            } else {
                Plant::Single("minecraft:short_grass")
            }
        } else if value > self.fern_chance {
            Plant::Single("minecraft:fern")
        } else {
            Plant::Single("minecraft:short_grass")
        })
    }

    fn seagrass(&self, x: i32, y: i32, water_depth: i32, random: &mut JavaRandom) -> Option<Plant> {
        let (value, tall) = self.grass_value(x, y, random);
        if value <= self.grass_chance {
            return None;
        }
        Some(if tall {
            if value > self.double_tall_grass_chance {
                Plant::Kelp(random.next_int(water_depth.min(26)) + 1)
            } else if random.next_int(4) == 0 || water_depth < 2 {
                Plant::Single("minecraft:seagrass")
            } else {
                Plant::Double("minecraft:tall_seagrass")
            }
        } else if value > self.fern_chance && water_depth >= 2 {
            Plant::Double("minecraft:tall_seagrass")
        } else {
            Plant::Single("minecraft:seagrass")
        })
    }

    /// Flower species in blobs, like Java's `RandomField(4, SMALL_BLOBS)`.
    fn flower_type(&self, x: i32, y: i32) -> usize {
        let cx = (x as f64 / SMALL_BLOBS).floor() as i64 as u64;
        let cy = (y as f64 / SMALL_BLOBS).floor() as i64 as u64;
        (splitmix64(
            self.seed.wrapping_add(65_226_710) ^ cx.wrapping_mul(0x9E37_79B9) ^ cy.rotate_left(32),
        ) % 16) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesa_bands_follow_java_random() {
        let m = Materials::new(12345);
        // Replays Java: Arrays.fill(TERRACOTTA); 32 × (index = nextInt(63), material = nextInt(8)).
        let mut r = JavaRandom::new(12345);
        let mut expected = ["minecraft:terracotta"; 64];
        for _ in 0..32 {
            let i = r.next_int(63) as usize;
            let mat = MESA_MATERIALS[r.next_int(8) as usize];
            expected[i] = mat;
            expected[i + 1] = mat;
        }
        assert_eq!(m.mesa_layers, expected);
        assert!(expected.iter().any(|b| *b != "minecraft:terracotta"));
    }

    #[test]
    fn grass_has_plants_and_flowers() {
        let m = Materials::new(7);
        let (mut plants, mut flowers, mut seagrass) = (0, 0, 0);
        for x in 0..200 {
            for y in 0..200 {
                match m.surface_plant(Terrain::Grass, x, y, 0) {
                    Some(Plant::Single(id)) | Some(Plant::Double(id))
                        if id.contains("grass") || id.contains("fern") =>
                    {
                        plants += 1
                    }
                    Some(_) => flowers += 1,
                    None => {}
                }
                if m.surface_plant(Terrain::Grass, x, y, 5).is_some() {
                    seagrass += 1;
                }
            }
        }
        // Roughly a tenth of columns get grass or ferns; flowers are rarer.
        assert!((1000..12000).contains(&plants), "{plants}");
        assert!(flowers > 0 && flowers < plants, "{flowers}");
        assert!(seagrass > 1000, "{seagrass}");
    }

    #[test]
    fn deserts_match_java_rolls() {
        let m = Materials::new(99);
        for x in -50..50 {
            for y in -50..50 {
                let roll = JavaRandom::for_column(99, x, y).next_int(1000);
                let expected = match roll {
                    n if n < 3 => Some(Plant::Stack("minecraft:cactus", n + 1)),
                    n if n < 6 => Some(Plant::Single("minecraft:dead_bush")),
                    _ => None,
                };
                assert_eq!(m.surface_plant(Terrain::Desert, x, y, 0), expected);
            }
        }
        assert_eq!(m.surface_plant(Terrain::Desert, 0, 0, 3), None);
    }

    #[test]
    fn mixed_blocks_vary() {
        let m = Materials::new(3);
        let mut rock = std::collections::BTreeSet::new();
        let mut beach = std::collections::BTreeSet::new();
        for x in 0..300 {
            rock.insert(m.top_layer_block(Terrain::Rock, x, 0, 64, 64).unwrap());
            beach.insert(
                m.top_layer_block(Terrain::Beaches, x * 7, x * 3, 64, 64)
                    .unwrap(),
            );
        }
        assert_eq!(rock.len(), 2, "{rock:?}");
        assert!(beach.len() >= 3, "{beach:?}");
        assert_eq!(m.top_layer_block(Terrain::Dirt, 0, 0, 0, 0), None);
        assert_eq!(
            m.top_layer_block(Terrain::GrassPath, 0, 0, 70, 70),
            Some("minecraft:dirt_path")
        );
    }
}

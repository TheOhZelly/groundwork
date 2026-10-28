//! Biome ids as stored in the Biome layer. Port of `Minecraft1_21Biomes`.

use crate::biomes_table::{BIOME_NAMES, MODERN_IDS};

/// Biome layer value meaning "derive the biome automatically".
pub const BIOME_AUTO: u8 = 255;

/// Display name of a WorldPainter biome id.
pub fn name(id: u8) -> Option<&'static str> {
    BIOME_NAMES[id as usize]
}

/// Namespaced Minecraft 1.18+ id, e.g. `minecraft:plains`.
pub fn modern_id(id: u8) -> Option<&'static str> {
    MODERN_IDS[id as usize]
}

/// All biomes that can be exported to modern Minecraft: (id, name, modern id).
pub fn all() -> impl Iterator<Item = (u8, &'static str, &'static str)> {
    (0..=254u8).filter_map(|id| Some((id, name(id)?, modern_id(id)?)))
}

// WorldPainter biome ids used by the automatic choice.
const OCEAN: u8 = 0;
const FOREST: u8 = 4;
const RIVER: u8 = 7;
const HELL: u8 = 8;
const SKY: u8 = 9;
const FROZEN_OCEAN: u8 = 10;
const FROZEN_RIVER: u8 = 11;
const ICE_PLAINS: u8 = 12;
const DEEP_OCEAN: u8 = 24;
const COLD_TAIGA: u8 = 30;
/// Desert and badlands variants, where forests don't turn the biome to forest.
const ARID: [u8; 9] = [2, 17, 130, 37, 38, 39, 165, 166, 167];

/// What a column looks like, for choosing its biome automatically.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    pub kind: crate::DimensionKind,
    pub terrain: crate::Terrain,
    /// Water (or lava) level minus terrain height; positive means submerged.
    pub water_depth: i32,
    pub lava: bool,
    pub frost: bool,
    /// Any Deciduous or Pine forest on the column.
    pub forest: bool,
}

/// The biome for a column whose Biome layer is "auto". Port of
/// `Dimension.getAutoBiome` (without the River, Swamp and Jungle layers,
/// which Groundwork doesn't have yet).
pub fn auto_biome(c: Column) -> u8 {
    use crate::DimensionKind;
    match c.kind {
        DimensionKind::Nether => return HELL,
        DimensionKind::End => return SKY,
        DimensionKind::Overworld => {}
    }
    let water = c.water_depth > 0 && !c.lava;
    if c.frost {
        return if c.forest {
            COLD_TAIGA
        } else if c.terrain == crate::Terrain::Water {
            FROZEN_RIVER
        } else if water {
            if c.water_depth <= 5 {
                FROZEN_RIVER
            } else {
                FROZEN_OCEAN
            }
        } else {
            ICE_PLAINS
        };
    }
    if water {
        return if c.water_depth <= 5 {
            RIVER
        } else if c.water_depth <= 20 {
            OCEAN
        } else {
            DEEP_OCEAN
        };
    }
    let default = c.terrain.info().default_biome;
    if c.forest && !ARID.contains(&default) {
        FOREST
    } else {
        default
    }
}

/// Map colour for a biome, following the material colours WorldPainter's
/// biome view uses (`AbstractMinecraft1_21BiomeScheme.getColour`).
pub fn colour(id: u8) -> [u8; 3] {
    const GRASS: [u8; 3] = [0x5f, 0x9f, 0x35];
    const OAK_LEAVES: [u8; 3] = [0x3f, 0x82, 0x28];
    const SNOW: [u8; 3] = [0xf0, 0xfb, 0xfb];
    let Some(name) = modern_id(id) else {
        return [0x80, 0x80, 0x80];
    };
    let n = name.trim_start_matches("minecraft:");
    match n {
        "warm_ocean" => [0x43, 0xd5, 0xee],
        "lukewarm_ocean" | "deep_lukewarm_ocean" => [0x45, 0xad, 0xf2],
        "cold_ocean" | "deep_cold_ocean" => [0x3d, 0x57, 0xd6],
        "frozen_ocean" | "deep_frozen_ocean" | "frozen_river" => [0x91, 0xb4, 0xf0],
        _ if n.contains("ocean") || n == "river" => [0x3f, 0x76, 0xe4],
        "snowy_beach" => SNOW,
        "beach" | "desert" => [0xdb, 0xd3, 0xa0],
        "stony_shore" | "stony_peaks" => [0x7d, 0x7d, 0x7d],
        "windswept_gravelly_hills" => [0x83, 0x7f, 0x7e],
        _ if n.starts_with("snowy_") || n.contains("frozen") || n.contains("ice") => SNOW,
        "grove" | "jagged_peaks" => SNOW,
        _ if n.contains("badlands") => [0x98, 0x5e, 0x43],
        _ if n.contains("savanna") => [0xae, 0xa4, 0x2a],
        "bamboo_jungle" => [0x5c, 0x90, 0x14],
        _ if n.contains("jungle") => [0x30, 0xbb, 0x0b],
        _ if n.contains("birch") => [0x80, 0xa7, 0x55],
        _ if n.contains("taiga") => [0x61, 0x99, 0x61],
        "cherry_grove" => [0xf2, 0xb3, 0xd0],
        "pale_garden" => [0x9f, 0xc0, 0x94],
        "mushroom_fields" => [0x6f, 0x63, 0x69],
        "nether_wastes" | "soul_sand_valley" => [0x72, 0x36, 0x32],
        "basalt_deltas" => [0x4d, 0x4d, 0x50],
        "crimson_forest" => [0x94, 0x1f, 0x1f],
        "warped_forest" => [0x2b, 0x72, 0x65],
        "the_void" => [0, 0, 0],
        _ if n.starts_with("end_") || n == "the_end" || n == "small_end_islands" => {
            [0xdb, 0xde, 0x9e]
        }
        _ if n.contains("forest") || n.contains("swamp") => OAK_LEAVES,
        _ => GRASS,
    }
}

/// Finds a biome by numeric id, modern id (with or without `minecraft:`) or
/// display name, ignoring case.
pub fn find(query: &str) -> Option<u8> {
    let q = query.trim();
    if let Ok(id) = q.parse::<u8>() {
        return (id == BIOME_AUTO || modern_id(id).is_some()).then_some(id);
    }
    if q.eq_ignore_ascii_case("auto") {
        return Some(BIOME_AUTO);
    }
    let q = q.to_ascii_lowercase();
    let namespaced = if q.contains(':') {
        q.clone()
    } else {
        format!("minecraft:{q}")
    };
    all()
        .find(|(_, n, m)| {
            *m == namespaced
                || n.eq_ignore_ascii_case(&q)
                || n.replace(' ', "_").eq_ignore_ascii_case(&q)
        })
        .map(|(id, _, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_biomes_follow_worldpainter() {
        use crate::{DimensionKind, Terrain};
        let c = |terrain, water_depth, frost, forest| Column {
            kind: DimensionKind::Overworld,
            terrain,
            water_depth,
            lava: false,
            frost,
            forest,
        };
        let id = |b: u8| modern_id(b).unwrap();
        assert_eq!(
            id(auto_biome(c(Terrain::Grass, 0, false, false))),
            "minecraft:plains"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Desert, 0, false, true))),
            "minecraft:desert"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Grass, 0, false, true))),
            "minecraft:forest"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Beaches, 0, false, false))),
            "minecraft:beach"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Sand, 3, false, false))),
            "minecraft:river"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Sand, 12, false, false))),
            "minecraft:ocean"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Sand, 40, false, false))),
            "minecraft:deep_ocean"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Grass, 0, true, false))),
            "minecraft:snowy_plains"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Grass, 0, true, true))),
            "minecraft:snowy_taiga"
        );
        assert_eq!(
            id(auto_biome(c(Terrain::Grass, 9, true, false))),
            "minecraft:frozen_ocean"
        );
        assert_eq!(
            auto_biome(Column {
                kind: DimensionKind::Nether,
                ..c(Terrain::Grass, 0, false, false)
            }),
            HELL
        );
    }

    #[test]
    fn every_biome_has_a_colour() {
        for (id, _, modern) in all() {
            let rgb = colour(id);
            if modern != "minecraft:the_void" {
                assert_ne!(rgb, [0, 0, 0], "{modern}");
            }
        }
        assert_ne!(
            colour(find("desert").unwrap()),
            colour(find("forest").unwrap())
        );
    }

    #[test]
    fn known_ids() {
        assert_eq!(modern_id(0), Some("minecraft:ocean"));
        assert_eq!(modern_id(1), Some("minecraft:plains"));
        assert_eq!(find("plains"), Some(1));
        assert_eq!(find("minecraft:desert"), Some(2));
        assert_eq!(find("auto"), Some(BIOME_AUTO));
        assert_eq!(
            find("cherry grove").and_then(modern_id),
            Some("minecraft:cherry_grove")
        );
    }
}

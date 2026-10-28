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

/// Groups for presenting biomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Plains,
    Forest,
    Dry,
    Mountains,
    Snowy,
    Water,
    Caves,
    Nether,
    End,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Plains,
        Category::Forest,
        Category::Dry,
        Category::Mountains,
        Category::Snowy,
        Category::Water,
        Category::Caves,
        Category::Nether,
        Category::End,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Plains => "Plains",
            Category::Forest => "Forest & jungle",
            Category::Dry => "Dry",
            Category::Mountains => "Mountains",
            Category::Snowy => "Snowy",
            Category::Water => "Water & coast",
            Category::Caves => "Caves",
            Category::Nether => "Nether",
            Category::End => "End",
        }
    }

    fn of(modern: &str) -> Category {
        let n = modern.trim_start_matches("minecraft:");
        match n {
            "deep_dark" | "dripstone_caves" | "lush_caves" => Category::Caves,
            "nether_wastes" | "soul_sand_valley" | "crimson_forest" | "warped_forest"
            | "basalt_deltas" => Category::Nether,
            "the_end" | "small_end_islands" | "end_midlands" | "end_highlands" | "end_barrens"
            | "the_void" => Category::End,
            "beach" | "snowy_beach" | "stony_shore" | "river" | "frozen_river" => Category::Water,
            _ if n.contains("ocean") => Category::Water,
            _ if n.starts_with("snowy_")
                || n.contains("frozen")
                || n == "ice_spikes"
                || n == "grove"
                || n == "jagged_peaks" =>
            {
                Category::Snowy
            }
            _ if n == "desert" || n.contains("badlands") || n.contains("savanna") => Category::Dry,
            _ if n.starts_with("windswept") || n == "stony_peaks" || n == "meadow" => {
                Category::Mountains
            }
            "plains" | "sunflower_plains" | "mushroom_fields" => Category::Plains,
            _ => Category::Forest,
        }
    }
}

/// One Minecraft biome as presented to people.
#[derive(Debug)]
pub struct Entry {
    /// The id painted into the Biome layer.
    pub id: u8,
    /// Current Minecraft name, e.g. "Old Growth Spruce Taiga".
    pub name: String,
    /// e.g. `minecraft:old_growth_spruce_taiga`.
    pub minecraft_id: &'static str,
    pub category: Category,
}

/// Every Minecraft biome exactly once, under its current name, grouped by
/// category and sorted by name. WorldPainter's table keeps older names
/// (e.g. "Desert Hills") for biomes Minecraft has since merged; those all
/// map to the same biome and are left out here.
pub fn catalogue() -> &'static [Entry] {
    static CATALOGUE: std::sync::OnceLock<Vec<Entry>> = std::sync::OnceLock::new();
    CATALOGUE.get_or_init(|| {
        let mut seen = std::collections::HashSet::new();
        let mut entries: Vec<Entry> = all()
            .filter(|(_, _, modern)| seen.insert(*modern))
            .map(|(id, _, modern)| Entry {
                id,
                name: modern
                    .trim_start_matches("minecraft:")
                    .split('_')
                    .map(|w| {
                        let mut c = w.chars();
                        c.next()
                            .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                minecraft_id: modern,
                category: Category::of(modern),
            })
            .collect();
        entries.sort_by(|a, b| (a.category, &a.name).cmp(&(b.category, &b.name)));
        entries
    })
}

/// Current Minecraft name of a biome id, e.g. "Windswept Hills" for
/// WorldPainter's "Mountain Edge".
pub fn display_name(id: u8) -> Option<&'static str> {
    let modern = modern_id(id)?;
    catalogue()
        .iter()
        .find(|e| e.minecraft_id == modern)
        .map(|e| e.name.as_str())
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
    // "Old Growth Spruce Taiga", "old_growth_spruce_taiga" and
    // "minecraft:old_growth_spruce_taiga" all name the same biome.
    let underscored = q.replace(' ', "_");
    let namespaced = if q.contains(':') {
        underscored
    } else {
        format!("minecraft:{underscored}")
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
    fn catalogue_lists_each_biome_once() {
        let c = catalogue();
        assert_eq!(c.len(), 65);
        let names: std::collections::HashSet<_> = c.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), 65);
        assert!(names.contains("Old Growth Spruce Taiga") && names.contains("Cherry Grove"));
        assert!(!names.contains("Desert Hills"));
        let cat = |name: &str| c.iter().find(|e| e.name == name).unwrap().category;
        assert_eq!(cat("Cherry Grove"), Category::Forest);
        assert_eq!(cat("Deep Frozen Ocean"), Category::Water);
        assert_eq!(cat("Jagged Peaks"), Category::Snowy);
        assert_eq!(cat("Wooded Badlands"), Category::Dry);
        assert_eq!(cat("Lush Caves"), Category::Caves);
        // Old ids show their current name; display names are findable.
        assert_eq!(
            display_name(find("minecraft:windswept_hills").unwrap()),
            Some("Windswept Hills")
        );
        assert_eq!(
            find("Old Growth Spruce Taiga").and_then(modern_id),
            Some("minecraft:old_growth_spruce_taiga")
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

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

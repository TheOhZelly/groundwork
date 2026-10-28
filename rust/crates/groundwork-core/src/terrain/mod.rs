//! Terrain types. Port of `org.pepsoft.worldpainter.Terrain`.

#[rustfmt::skip]
mod table;

pub use table::Terrain;
use table::TABLE;

/// How faithfully the Rust exporter reproduces the Java terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    /// A single top block over a single underground block, as in Java.
    Exact,
    /// Several materials mixed with noise, and/or plants on top. `top` and
    /// `under` are only representative (for previews); the exporter has the
    /// full rules.
    Mixed,
    /// User-configured custom material; not supported yet.
    Custom,
}

#[derive(Debug)]
pub struct TerrainInfo {
    pub terrain: Terrain,
    /// The Java enum constant name, e.g. `"BARE_GRASS"`.
    pub id: &'static str,
    /// Display name, e.g. `"Bare Grass"`.
    pub name: &'static str,
    /// Surface block.
    pub top: Option<&'static str>,
    /// Block used for the rest of the top layer.
    pub under: Option<&'static str>,
    pub fidelity: Fidelity,
}

impl Terrain {
    pub fn all() -> &'static [TerrainInfo] {
        &TABLE
    }

    pub fn info(self) -> &'static TerrainInfo {
        &TABLE[self as usize]
    }

    /// Java enum ordinal, which is what tiles store.
    pub fn ordinal(self) -> u8 {
        self as u8
    }

    pub fn from_ordinal(ordinal: u8) -> Option<Terrain> {
        TABLE.get(ordinal as usize).map(|i| i.terrain)
    }

    /// Looks up a terrain by Java constant name (`BARE_GRASS`), display name
    /// (`Bare Grass`) or Rust variant name (`BareGrass`), ignoring case,
    /// spaces and underscores.
    pub fn find(query: &str) -> Option<Terrain> {
        let key = normalise(query);
        TABLE
            .iter()
            .find(|i| normalise(i.id) == key || normalise(i.name) == key)
            .map(|i| i.terrain)
    }
}

impl std::fmt::Display for Terrain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.info().name)
    }
}

fn normalise(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_order_matches_ordinals() {
        for (i, info) in Terrain::all().iter().enumerate() {
            assert_eq!(info.terrain as usize, i, "{}", info.id);
        }
    }

    #[test]
    fn java_ordinals_are_preserved() {
        // Spot checks against the Java enum declaration order.
        assert_eq!(Terrain::Grass.ordinal(), 0);
        assert_eq!(Terrain::Water.ordinal(), 6);
        assert_eq!(Terrain::PaleMoss.ordinal(), 161);
    }

    #[test]
    fn finds_by_any_name() {
        assert_eq!(Terrain::find("BARE_GRASS"), Some(Terrain::BareGrass));
        assert_eq!(Terrain::find("bare grass"), Some(Terrain::BareGrass));
        assert_eq!(Terrain::find("Terracotta"), Some(Terrain::HardenedClay));
        assert_eq!(Terrain::find("nope"), None);
    }
}

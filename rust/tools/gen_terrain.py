"""Generate wp-core/src/terrain_table.rs from WorldPainter's Terrain.java.

Usage: python -I gen_terrain.py <Terrain.java> <out.rs>
"""
import re
import sys

src, out = sys.argv[1], sys.argv[2]
text = open(src, encoding="utf-8").read()

# Java constant -> Minecraft block id.
BLOCKS = {
    "GRASS_BLOCK": "grass_block", "BLK_GRASS": "grass_block", "BLK_DIRT": "dirt", "DIRT": "dirt",
    "BLK_SAND": "sand", "SAND": "sand", "BLK_SANDSTONE": "sandstone", "BLK_STONE": "stone",
    "STATIONARY_WATER": "water", "STATIONARY_LAVA": "lava", "BLK_SNOW_BLOCK": "snow_block",
    "BLK_GRAVEL": "gravel", "BLK_CLAY": "clay", "BLK_COBBLESTONE": "cobblestone",
    "BLK_MOSSY_COBBLESTONE": "mossy_cobblestone", "BLK_NETHERRACK": "netherrack", "NETHERRACK": "netherrack",
    "BLK_SOUL_SAND": "soul_sand", "BLK_OBSIDIAN": "obsidian", "BLK_BEDROCK": "bedrock",
    "BLK_MYCELIUM": "mycelium", "BLK_END_STONE": "end_stone", "PERMADIRT": "coarse_dirt",
    "PODZOL": "podzol", "RED_SAND": "red_sand", "HARDENED_CLAY": "terracotta",
    "BLK_RED_SANDSTONE": "red_sandstone", "GRANITE": "granite", "DIORITE": "diorite",
    "ANDESITE": "andesite", "BLK_MAGMA": "magma_block", "DEEPSLATE_Y": "deepslate", "TUFF": "tuff",
    "BASALT": "basalt", "BLACKSTONE": "blackstone", "SOUL_SOIL": "soul_soil",
    "WARPED_NYLIUM": "warped_nylium", "CRIMSON_NYLIUM": "crimson_nylium", "CALCITE": "calcite",
    "MUD": "mud", "MOSS_BLOCK": "moss_block", "PALE_MOSS_BLOCK": "pale_moss_block",
}
for colour in ["WHITE", "ORANGE", "MAGENTA", "LIGHT_BLUE", "YELLOW", "LIME", "PINK", "GREY",
               "LIGHT_GREY", "CYAN", "PURPLE", "BLUE", "BROWN", "GREEN", "RED", "BLACK"]:
    BLOCKS[colour + "_CLAY"] = colour.lower().replace("grey", "gray") + "_terracotta"

# Terrains whose Java implementation mixes several materials with noise. Until
# that logic is ported they export as a single representative top/under pair.
APPROXIMATIONS = {
    "GRASS": ("grass_block", "grass_block"), "ROCK": ("stone", "stone"), "SNOW": ("stone", "stone"),
    "DESERT": ("sand", "sand"), "NETHERLIKE": ("netherrack", "netherrack"),
    "RESOURCES": ("stone", "stone"), "BEACHES": ("grass_block", "grass_block"),
    "MESA": ("terracotta", "terracotta"), "RED_DESERT": ("red_sand", "red_sand"),
    "STONE_MIX": ("stone", "stone"), "GRASS_PATH": ("dirt_path", "dirt"),
    "BARE_BEACHES": ("grass_block", "grass_block"),
}

pattern = re.compile(r'^    ([A-Z_0-9]+)\("([^"]*)"(.*)$', re.M)
rows = []
for m in pattern.finditer(text):
    ident, display, rest = m.group(1), m.group(2), m.group(3)
    args = [a.strip() for a in rest.split(",")]
    if ident.startswith("CUSTOM_"):
        rows.append((ident, display, None, None, "Custom"))
        continue
    if ident in APPROXIMATIONS:
        top, under = APPROXIMATIONS[ident]
        rows.append((ident, display, top, under, "Approximate"))
        continue
    top_c = args[1].replace("Material.", "")
    under_c = args[2].replace("Material.", "")
    if top_c not in BLOCKS or under_c not in BLOCKS:
        sys.exit(f"unmapped material for {ident}: {top_c} / {under_c}")
    rows.append((ident, display, BLOCKS[top_c], BLOCKS[under_c], "Exact"))

assert len(rows) == 162, len(rows)

def camel(ident):
    return "".join(p.capitalize() for p in ident.split("_"))

def opt(b):
    return f'Some("minecraft:{b}")' if b else "None"

with open(out, "w", encoding="utf-8", newline="\n") as f:
    f.write("// @generated from WorldPainter/WPCore/.../Terrain.java by gen_terrain.py. Do not edit by hand.\n")
    f.write("// Variant order must match the Java enum ordinals: tiles store terrain as an ordinal byte.\n\n")
    f.write("use super::{Fidelity, TerrainInfo};\n\n")
    f.write("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]\n")
    f.write("#[repr(u8)]\npub enum Terrain {\n")
    for r in rows:
        f.write(f"    {camel(r[0])},\n")
    f.write("}\n\n")
    f.write(f"pub(super) const TABLE: [TerrainInfo; {len(rows)}] = [\n")
    for ident, display, top, under, fid in rows:
        f.write(f'    TerrainInfo {{ terrain: Terrain::{camel(ident)}, id: "{ident}", name: "{display}", '
                f'top: {opt(top)}, under: {opt(under)}, fidelity: Fidelity::{fid} }},\n')
    f.write("];\n")
print(f"wrote {len(rows)} terrains")

"""Generate wp-core/src/biomes_table.rs from Minecraft1_21Biomes.java.

Usage: python -I gen_biomes.py <Minecraft1_21Biomes.java> <out.rs>
"""
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()

# String constants from org.pepsoft.minecraft.Constants referenced by the tables.
CONSTANTS = {"MC_PLAINS": "minecraft:plains"}

def array(name):
    body = re.search(r"String\[\] " + name + r" = \{(.*?)\};", text, re.S).group(1)
    body = re.sub(r"//[^\n]*", "", body)
    items = re.findall(r'"[^"]*"|null|\bMC_[A-Z_]+\b', body)
    return [None if i == "null" else CONSTANTS[i] if i.startswith("MC_") else i.strip('"') for i in items]

names, ids = array("BIOME_NAMES"), array("MODERN_IDS")
n = max(len(names), len(ids))
names += [None] * (n - len(names))
ids += [None] * (n - len(ids))

def opt(s):
    return "None" if s is None else f'Some("{s}")'

with open(sys.argv[2], "w", encoding="utf-8", newline="\n") as f:
    f.write("// @generated from Minecraft1_21Biomes.java by tools/gen_biomes.py. Do not edit by hand.\n")
    f.write("// Indexed by WorldPainter biome id, as stored in the Biome layer.\n\n")
    f.write(f"pub const BIOME_NAMES: [Option<&str>; {n}] = [\n")
    f.writelines(f"    {opt(s)},\n" for s in names)
    f.write("];\n\n")
    f.write(f"pub const MODERN_IDS: [Option<&str>; {n}] = [\n")
    f.writelines(f"    {opt(s)},\n" for s in ids)
    f.write("];\n")
print(n, "biome slots,", sum(1 for i in ids if i), "with modern ids")

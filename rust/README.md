# WorldPainter in Rust

A Rust rewrite of WorldPainter, in progress, with an MCP server so AI assistants can sculpt, paint and export Minecraft maps.

The Java application in [`../WorldPainter`](../WorldPainter) is still the complete, supported product. This workspace ports it bottom-up: file formats first, then the world model, then exporting, then the GUI. Each stage is tested as it goes.

## Crates

| Crate | Replaces (Java) | Status |
|---|---|---|
| `wp-nbt` | JNBT | Complete: all tag types, Java modified UTF-8, gzip/zlib/raw |
| `wp-anvil` | `org.pepsoft.minecraft.RegionFile` | Complete: read/write, sector reuse, external `.mcc` chunks (read) |
| `wp-core` | `Tile`, `Dimension`, `World2`, `Terrain`, core operations, `TileRenderer` | Partial, see below |
| `wp-export` | `org.pepsoft.worldpainter.exporting`, `Java118Level` | Partial: Java Edition 1.21 only |
| `wp-mcp` | (new) | MCP server, 21 tools |

## Running the MCP server

```bash
cargo build --release
```

That produces `target/release/worldpainter-mcp(.exe)`, a stdio MCP server. To register it with Claude Code:

```bash
claude mcp add worldpainter -- C:/Users/OhZelly/Documents/WorldPainter/rust/target/release/worldpainter-mcp.exe
```

For other clients, add this to their `mcpServers` config:

```json
{ "mcpServers": { "worldpainter": { "command": "C:/path/to/worldpainter-mcp.exe" } } }
```

Tools: `create_world`, `open_world`, `save_world`, `world_info`, `list_terrains`, `list_biomes`, `list_layers`, `get_column`, `raise_lower`, `flatten`, `smooth`, `paint_terrain`, `paint_layer`, `flood`, `set_water_level`, `add_tiles`, `set_spawn`, `set_world_settings`, `render_preview` (returns a PNG), `export_minecraft`, `undo`.

## What's ported

- **World model:**
  - 128×128 tiles.
  - Heights with 1/256 precision.
  - Water levels.
  - Terrain stored as Java enum ordinals: all 162 `Terrain` constants, generated from `Terrain.java` by `tools/gen_terrain.py`.
  - Bit, nibble and byte layers.
- **Layers:** Frost, Void, FloodWithLava and Biome. The biome ids are generated from `Minecraft1_21Biomes.java` by `tools/gen_biomes.py`.
- **New worlds:** a noise height map plus one terrain, with `SimpleTheme`'s beach rule.
- **Operations:**
  - Raise/lower, flatten, smooth, paint terrain and paint layer, with circle, square or rectangle brushes and four falloff curves.
  - Flood and drain, as a bounded fill.
  - Set water level.
- **Preview:** a top-down render with hill shading.
- **Export to Minecraft Java 1.21.1:**
  - Bedrock, a stone-to-deepslate subsurface, the terrain top layer, water and lava, ice and snow from Frost, Void columns and biomes.
  - Writes `level.dat` with either vanilla or void generation outside the painted area.
  - Uses one thread per region file.
  - Leaves lighting and height maps for Minecraft to compute on first load.

## Known differences from Java

- **Save files:** worlds save as `.wprs` (gzipped bincode). Java `.world` files use Java object serialization; they are recognised but can't be opened yet.
- **Mixed terrains:**
  - Terrains that Java mixes with noise (Grass with flowers, Rock, Desert, Mesa, Beaches, Resources, Stone Mix, and others) export as one representative block. The exporter reports this as a warning.
  - Custom 1–96 terrains export as stone.
- **Heights:** height storage is always 32-bit. Java's 16-bit "not tall" representation is not used.
- **Noise:** generated noise is deterministic but not bit-identical to Java's `PerlinNoise`, so the same seed gives a different landscape.
- **Undo:** keeps whole-world snapshots (10 steps). Java uses per-buffer copy-on-write.

## Roadmap

Rough order. Each step should land with tests against Java's behaviour.

1. **Exact terrain materials:** port `MixedMaterial` and each terrain's `getMaterial` overrides, plus plants on Grass and Desert.
2. **Java `.world` import:** a reader for the Java serialization stream limited to WorldPainter's classes, so existing projects open.
3. **Remaining layers:**
   - Trees (deciduous, pine, jungle, swamp, mangrove)
   - Caverns, caves, chasms and tunnels
   - Resources (ores)
   - Plants
   - Custom objects (bo2, bo3, schematic, nbt)
4. **More platforms:**
   - MCRegion and Anvil from 1.2 to 1.17
   - 26.1 (`DATA_VERSION_MC_26_1`)
   - Nether and End creation
   - Ceiling dimensions
5. **Import:** import an existing Minecraft map, and import or export height map images.
6. **Post-processing:** sand and gravel support, fluid edge fixes and lighting, matching `PostProcessor`.
7. **Scripting and plugins:** the JS scripting API and a plugin interface.
8. **GUI:** an editor (likely `egui`) covering the main window, tools, brushes, palettes, selection and the 3D view.

## Development

```bash
cargo test
cargo clippy --all-targets
```

Regenerate the tables after the Java enums change:

```bash
python -I tools/gen_terrain.py ../WorldPainter/WPCore/src/main/java/org/pepsoft/worldpainter/Terrain.java crates/wp-core/src/terrain/table.rs
python -I tools/gen_biomes.py ../WorldPainter/WPCore/src/main/java/org/pepsoft/worldpainter/biomeschemes/Minecraft1_21Biomes.java crates/wp-core/src/biomes_table.rs
```

On Windows without Visual Studio Build Tools, use the GNU toolchain (`rustup default stable-x86_64-pc-windows-gnu`).

Licensed GPL-3.0-or-later, like WorldPainter.

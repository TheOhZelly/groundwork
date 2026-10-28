# Groundwork

Groundwork is a Minecraft map painter written in Rust. It has a desktop editor for painting worlds with the mouse, and an MCP server so AI assistants can sculpt, paint and export Minecraft maps too.

Groundwork is an unofficial port of [WorldPainter](https://www.worldpainter.net/) by Pepijn Schmitz. It is not affiliated with or endorsed by the WorldPainter project. Much of its design, data model and behaviour comes from WorldPainter's Java source, which this repository keeps alongside in [`../WorldPainter`](../WorldPainter). The Java application is still far more complete; use it for real projects.

The port works bottom-up: file formats first, then the world model, then exporting, then the GUI. Each stage is tested as it goes.

## Crates

| Crate | Replaces (WorldPainter, Java) | Status |
|---|---|---|
| `groundwork-nbt` | JNBT | Complete: all tag types, Java modified UTF-8, gzip/zlib/raw |
| `groundwork-anvil` | `org.pepsoft.minecraft.RegionFile` | Complete: read/write, sector reuse, external `.mcc` chunks (read) |
| `groundwork-core` | `Tile`, `Dimension`, `World2`, `Terrain`, core operations, `TileRenderer` | Partial, see below |
| `groundwork-export` | `org.pepsoft.worldpainter.exporting`, `Java118Level` | Partial: Java Edition 1.21.11 and 1.21.1 |
| `groundwork-mcp` | (new) | MCP server, 21 tools |
| `groundwork-app` | WPGUI (Swing) | Early: editor with sculpting, painting, flood, spawn, undo/redo, save/open, export |

## Running the editor

```bash
cargo run --release -p groundwork-app
```

That builds and opens the editor; the program is `target/release/groundwork(.exe)`.

- **Navigate:** scroll to zoom, middle-drag or Space+drag to pan, arrow keys to move, F to fit the world, B to switch between terrain and biome view.
- **Tools:** 1 Raise, 2 Lower, 3 Flatten, 4 Smooth, 5 Paint terrain, 6 Paint layer, 7 Flood, 8 Spawn.
- **Brushes:** left-drag applies the tool; right-drag does the opposite (lower, erase, drain), and with Paint terrain it picks the terrain under the cursor. `[` and `]` change the brush size.
- **Files:** Ctrl+N new, Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as, Ctrl+E export, Ctrl+Z undo, Ctrl+Shift+Z or Ctrl+Y redo.
- **Height maps:** import a greyscale PNG as a new world or into the open one, or export the heights, from the World panel (or "Start from a height map" on the welcome screen).

## Building a world from map images

`examples/from_map_images.rs` in `groundwork-export` builds a large world from three renders of a map made with Azgaar's Fantasy Map Generator:

1. the plain map,
2. the height map,
3. the biome map.

It aligns the three images, upscales them with noise for coastline, hill and ridge detail, and maps the biome colours to terrain, Minecraft biomes and forests. It then saves a `.gwork`, renders a preview and exports a 1.21.11 save surrounded by endless ocean.

```bash
cargo run --release -p groundwork-export --example from_map_images -- map.png heights.png biomes.png OUT_DIR "World name" 10240
```

`examples/inspect_chunk.rs` prints the blocks and biomes of any chunk in an exported save.

## Running the MCP server

```bash
cargo build --release
```

That produces `target/release/groundwork-mcp(.exe)`, a stdio MCP server. To register it with Claude Code:

```bash
claude mcp add groundwork -- C:/Users/OhZelly/Documents/WorldPainter/rust/target/release/groundwork-mcp.exe
```

For other clients, add this to their `mcpServers` config:

```json
{ "mcpServers": { "groundwork": { "command": "C:/path/to/groundwork-mcp.exe" } } }
```

Tools: `create_world`, `open_world`, `save_world`, `world_info`, `list_terrains`, `list_biomes`, `list_layers`, `get_column`, `raise_lower`, `flatten`, `smooth`, `paint_terrain`, `paint_layer`, `flood`, `set_water_level`, `add_tiles`, `set_spawn`, `set_world_settings`, `render_preview` (returns a PNG), `export_minecraft`, `undo`.

## What's ported

- **World model:**
  - 128×128 tiles.
  - Heights with 1/256 precision.
  - Water levels.
  - Terrain stored as WorldPainter's enum ordinals: all 162 `Terrain` constants, generated from `Terrain.java` by `tools/gen_terrain.py`.
  - Bit, nibble and byte layers.
  - Compact storage: tiles are shared copy-on-write (cheap undo), tile-wide constant data is stored once, and heights use 16-bit offsets where possible.
- **Mixed terrains:** Grass, Rock, Snow on Rock, Stone Mix, Beaches, Bare Beaches, Netherlike, Mesa, Desert, Red Desert and Dirt Path, with their plants (flowers, grass, ferns, seagrass, kelp, cacti, dead bushes). Per-column choices use a bit-exact `java.util.Random`, so they land where WorldPainter puts them.
- **Layers:** Frost, Void, FloodWithLava, Biome, Deciduous and Pine forests, Resources (ores) and Caverns. The biome ids are generated from `Minecraft1_21Biomes.java` by `tools/gen_biomes.py`.
- **Biomes:** WorldPainter's automatic biomes (water depth, frost, forests, each terrain's default biome) and a biome view in the editor and previews.
- **Height maps:** import 8- or 16-bit greyscale PNGs; export 16-bit PNGs that import back losslessly.
- **New worlds:** a noise height map plus one terrain, with `SimpleTheme`'s beach rule.
- **Operations:**
  - Raise/lower, flatten, smooth, paint terrain and paint layer, with circle, square or rectangle brushes and four falloff curves.
  - Flood and drain, as a bounded fill.
  - Set water level.
- **Preview:** a top-down render with hill shading.
- **Export to Minecraft Java 1.21.11 or 1.21.1:**
  - These match WorldPainter's `JAVA_ANVIL_1_21_11` and `JAVA_ANVIL_1_20_5` platforms, which share one format and differ only in data version.
  - Bedrock, a stone-to-deepslate subsurface, the terrain top layer, water and lava, ice and snow from Frost, Void columns and biomes.
  - Trees from the forest layers: oak and birch for Deciduous, spruce for Pine. Density follows WorldPainter's rule, and canopies cross chunk borders.
  - Writes `level.dat` with vanilla terrain, void, or endless ocean outside the painted area.
  - Uses one thread per region file; the thread count can be capped.
  - Leaves lighting and height maps for Minecraft to compute on first load.
  - Not yet verified by loading in Minecraft itself.

## Known differences from WorldPainter

- **Save files:** worlds save as `.gwork` (gzipped bincode). WorldPainter `.world` files use Java object serialization; they are recognised but can't be opened yet.
- **Custom terrains:** Custom 1–96 terrains export as stone.
- **Noise:** deterministic but not bit-identical to WorldPainter's `PerlinNoise`, so the same seed gives a different landscape. Thresholds are derived from Groundwork's noise distribution, so materials keep WorldPainter's rarities.
- **Ores:** WorldPainter's table, amounts and height ranges, placed as seeded pockets instead of per-block noise per material. The amounts match; the export is about a thousand times cheaper.
- **Automatic biomes:** there are no River, Swamp or Jungle layers yet, so those parts of WorldPainter's rules don't apply.

## Roadmap

Rough order. Each step should land with tests against WorldPainter's behaviour.

1. **Custom terrains:** port `MixedMaterial`, so Custom 1–96 work.
2. **WorldPainter `.world` import:** a reader for the Java serialization stream limited to WorldPainter's classes, so existing projects open.
3. **Remaining layers:**
   - Trees: Java's exact tree shapes, plus jungle, swamp and mangrove
   - Rivers, chasms and tunnels
   - The Plants layer
   - Custom objects (bo2, bo3, schematic, nbt)
4. **More platforms:**
   - MCRegion and Anvil from 1.2 to 1.17
   - 26.1
   - Nether and End creation
   - Ceiling dimensions
5. **Import:** import an existing Minecraft map.
6. **Post-processing:** sand and gravel support, fluid edge fixes and lighting, matching `PostProcessor`.
7. **Scripting and plugins:** a scripting API and a plugin interface.
8. **GUI:** grow the `egui` editor towards WorldPainter's: selection, custom brushes, layer palettes, multiple dimensions, settings and the 3D view.

## Development

```bash
cargo test
cargo clippy --all-targets
```

Regenerate the tables after WorldPainter's Java enums change:

```bash
python -I tools/gen_terrain.py ../WorldPainter/WPCore/src/main/java/org/pepsoft/worldpainter/Terrain.java crates/groundwork-core/src/terrain/table.rs
python -I tools/gen_biomes.py ../WorldPainter/WPCore/src/main/java/org/pepsoft/worldpainter/biomeschemes/Minecraft1_21Biomes.java crates/groundwork-core/src/biomes_table.rs
```

On Windows, use the MSVC toolchain with Visual Studio Build Tools (C++ workload). The GNU toolchain can't build the editor without a full MinGW-w64 install.

## Thanks

See [THANKS.md](../THANKS.md) for the people and projects Groundwork builds on.

## License

GPL-3.0-or-later, the same as WorldPainter, from which this is derived. WorldPainter is © Pepijn Schmitz; see [`../LICENSE`](../LICENSE).

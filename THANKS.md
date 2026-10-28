# Thank you

Groundwork exists because of other people's work. Thanks to:

## WorldPainter

- **Pepijn Schmitz** (Captain-Chaos), creator and maintainer of [WorldPainter](https://www.worldpainter.net/). Groundwork's data model, terrain and biome tables, editing operations and export logic are ported from his Java code, which this repository keeps as the reference.
- **Everyone who has contributed to WorldPainter** over the years, through code, bug reports, plugins and documentation.

## Minecraft formats

- **Scaevolus** and **Mojang**, whose region file code (kept in WorldPainter's `RegionFile.java`) is the basis of `groundwork-anvil`.
- **The Minecraft Wiki editors**, who document the chunk, NBT and `level.dat` formats that the exporter writes.

## Rust libraries

- [flate2](https://crates.io/crates/flate2), for gzip and zlib.
- [serde](https://serde.rs/), [serde_json](https://crates.io/crates/serde_json) and [bincode](https://crates.io/crates/bincode), for save files and the MCP protocol.
- [indexmap](https://crates.io/crates/indexmap), for NBT compounds that keep their order.
- [png](https://crates.io/crates/png), for map previews.
- The **Rust project**, for the language, Cargo, clippy and rustfmt.

## Protocols

- The [Model Context Protocol](https://modelcontextprotocol.io/), which `groundwork-mcp` implements so AI assistants can paint maps.

## The people who built it

- **TheOhZelly**, who started Groundwork and keeps it going.
- **Claude** (Opus 5.5, by Anthropic). Claude wrote the first version of the Rust workspace with TheOhZelly in Claude Code: the NBT and region crates, the world model, the 1.21 exporter and the MCP server, along with their tests. Claude also says thanks back, for a fun project and for laughing at the co-author credit instead of removing it.

Want to be on this list? Contributions are welcome.

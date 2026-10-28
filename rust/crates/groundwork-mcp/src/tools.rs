//! MCP tool definitions and their implementations.

use crate::args::Args;
use crate::base64;
use groundwork_core::generate::{generate, GenerateOptions};
use groundwork_core::ops::{self, Falloff};
use groundwork_core::terrain::Fidelity;
use groundwork_core::{biomes, layers, render, Dimension, DimensionKind, GameMode, Terrain, World};
use groundwork_export::{ExportOptions, McVersion, Outside};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

/// Undo snapshots share unchanged tiles, so each costs only what its edit
/// touched.
const MAX_UNDO: usize = 50;
const MAX_TILES: u64 = 1024;
const NO_WORLD: &str = "no world is open; use create_world or open_world first";

#[derive(Default)]
pub struct Server {
    world: Option<World>,
    path: Option<PathBuf>,
    undo: Vec<World>,
    dirty: bool,
}

enum Output {
    Text(String),
    Image { png: Vec<u8>, text: String },
}

fn text(v: Value) -> Output {
    Output::Text(serde_json::to_string_pretty(&v).unwrap())
}

impl Server {
    pub fn call(&mut self, name: &str, arguments: &Value) -> Value {
        let args = Args(arguments);
        match self.dispatch(name, &args) {
            Ok(Output::Text(t)) => {
                json!({ "content": [{ "type": "text", "text": t }], "isError": false })
            }
            Ok(Output::Image { png, text }) => json!({
                "content": [
                    { "type": "image", "data": base64::encode(&png), "mimeType": "image/png" },
                    { "type": "text", "text": text },
                ],
                "isError": false,
            }),
            Err(message) => {
                json!({ "content": [{ "type": "text", "text": message }], "isError": true })
            }
        }
    }

    fn dispatch(&mut self, name: &str, a: &Args) -> Result<Output, String> {
        match name {
            "create_world" => self.create_world(a),
            "open_world" => self.open_world(a),
            "save_world" => self.save_world(a),
            "world_info" => self.world_info(),
            "list_terrains" => Ok(list_terrains()),
            "list_biomes" => Ok(list_biomes()),
            "list_layers" => Ok(list_layers()),
            "get_column" => self.get_column(a),
            "raise_lower" => {
                let (shape, falloff, amount) = (
                    a.shape()?,
                    a.falloff(Falloff::Smooth)?,
                    a.req_f32("amount")?,
                );
                self.edit(|d| {
                    Ok(format!(
                        "changed height of {} columns",
                        ops::raise_lower(d, shape, falloff, amount)
                    ))
                })
            }
            "flatten" => {
                let (shape, falloff, height) =
                    (a.shape()?, a.falloff(Falloff::Constant)?, a.f32("height")?);
                self.edit(|d| {
                    Ok(format!(
                        "flattened {} columns",
                        ops::flatten(d, shape, falloff, height)
                    ))
                })
            }
            "smooth" => {
                let (shape, falloff) = (a.shape()?, a.falloff(Falloff::Smooth)?);
                let iterations = a.i32("iterations")?.unwrap_or(2).clamp(1, 20) as u32;
                self.edit(|d| {
                    Ok(format!(
                        "smoothed {} columns",
                        ops::smooth(d, shape, falloff, iterations)
                    ))
                })
            }
            "paint_terrain" => {
                let (shape, falloff) = (a.shape()?, a.falloff(Falloff::Constant)?);
                let terrain = find_terrain(a.req_str("terrain")?)?;
                self.edit(|d| {
                    Ok(format!(
                        "painted {} columns with {terrain}",
                        ops::paint_terrain(d, shape, falloff, terrain)
                    ))
                })
            }
            "paint_layer" => self.paint_layer(a),
            "flood" => {
                let (x, y, level) = (a.req_i32("x")?, a.req_i32("y")?, a.req_i32("level")?);
                let lava = a.bool("lava")?.unwrap_or(false);
                self.edit(|d| {
                    let n = ops::flood(d, x, y, level, lava, 4_000_000)?;
                    Ok(if n == 0 {
                        format!("nothing to flood: the column at {x},{y} is not below level {level} or is already flooded to it")
                    } else {
                        format!("{} {n} columns", if level <= d.int_height_at(x, y).unwrap_or(level) { "drained" } else { "flooded" })
                    })
                })
            }
            "set_water_level" => {
                let (shape, level) = (a.shape()?, a.req_i32("level")?);
                let lava = a.bool("lava")?.unwrap_or(false);
                self.edit(|d| {
                    Ok(format!(
                        "set water level of {} columns to {level}",
                        ops::set_water_level(d, shape, level, lava)
                    ))
                })
            }
            "add_tiles" => self.add_tiles(a),
            "set_spawn" => {
                let (x, y) = (a.req_i32("x")?, a.req_i32("y")?);
                let world = self.world.as_mut().ok_or(NO_WORLD)?;
                if !world
                    .dimension(DimensionKind::Overworld)
                    .unwrap()
                    .contains(x, y)
                {
                    return Err(format!("{x},{y} is outside the painted area"));
                }
                world.spawn = (x, y);
                self.dirty = true;
                Ok(Output::Text(format!("spawn set to {x},{y}")))
            }
            "set_world_settings" => self.set_world_settings(a),
            "render_preview" => self.render_preview(a),
            "export_minecraft" => self.export(a),
            "undo" => {
                let previous = self.undo.pop().ok_or("nothing to undo")?;
                self.world = Some(previous);
                self.dirty = true;
                Ok(Output::Text(format!(
                    "undone; {} more step(s) available",
                    self.undo.len()
                )))
            }
            _ => Err(format!("unknown tool {name:?}")),
        }
    }

    /// Runs an edit on the overworld, keeping an undo snapshot. A failed edit
    /// leaves the world unchanged.
    fn edit(
        &mut self,
        f: impl FnOnce(&mut Dimension) -> Result<String, String>,
    ) -> Result<Output, String> {
        let world = self.world.as_mut().ok_or(NO_WORLD)?;
        let snapshot = world.clone();
        let result = f(world
            .dimension_mut(DimensionKind::Overworld)
            .expect("worlds always have an overworld"));
        match result {
            Ok(message) => {
                if self.undo.len() == MAX_UNDO {
                    self.undo.remove(0);
                }
                self.undo.push(snapshot);
                self.dirty = true;
                Ok(Output::Text(message))
            }
            Err(e) => {
                *world = snapshot;
                Err(e)
            }
        }
    }

    fn check_unsaved(&self, a: &Args) -> Result<(), String> {
        if self.dirty && !a.bool("discard_unsaved")?.unwrap_or(false) {
            return Err("the open world has unsaved changes; save_world first or pass discard_unsaved: true".into());
        }
        Ok(())
    }

    fn create_world(&mut self, a: &Args) -> Result<Output, String> {
        self.check_unsaved(a)?;
        let d = GenerateOptions::default();
        let seed = a.u64("seed")?.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |t| t.as_nanos() as u64)
        });
        let opts = GenerateOptions {
            kind: DimensionKind::Overworld,
            width_tiles: positive(a.i32("width_tiles")?, d.width_tiles, "width_tiles")?,
            length_tiles: positive(a.i32("length_tiles")?, d.length_tiles, "length_tiles")?,
            min_height: a.i32("min_height")?.unwrap_or(d.min_height),
            max_height: a.i32("max_height")?.unwrap_or(d.max_height),
            base_height: a.f32("base_height")?.unwrap_or(d.base_height),
            water_level: a.i32("water_level")?.unwrap_or(d.water_level),
            hilliness: a.f32("hilliness")?.unwrap_or(d.hilliness).max(0.0),
            feature_size: a.f32("feature_size")?.unwrap_or(d.feature_size),
            terrain: match a.str("terrain")? {
                Some(t) => find_terrain(t)?,
                None => d.terrain,
            },
            beaches: a.bool("beaches")?.unwrap_or(d.beaches),
            seed,
        };
        let tiles = opts.width_tiles as u64 * opts.length_tiles as u64;
        if tiles > MAX_TILES {
            return Err(format!("{tiles} tiles requested; the limit is {MAX_TILES} (e.g. 32×32 tiles = 4096×4096 blocks)"));
        }
        let dim = generate(&opts)?;
        let extent = dim.extent().unwrap();
        let mut world = World::new(a.req_str("name")?, seed, dim);
        world.spawn = (0, 0);
        self.world = Some(world);
        self.path = None;
        self.undo.clear();
        self.dirty = true;
        Ok(text(json!({
            "created": a.req_str("name")?,
            "seed": (seed as i64).to_string(),
            "tiles": tiles,
            "extent": extent,
            "note": "not saved yet; use save_world to keep it",
        })))
    }

    fn open_world(&mut self, a: &Args) -> Result<Output, String> {
        self.check_unsaved(a)?;
        let path = PathBuf::from(a.req_str("path")?);
        let world = groundwork_core::io::load(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let name = world.name.clone();
        self.world = Some(world);
        self.path = Some(path);
        self.undo.clear();
        self.dirty = false;
        Ok(Output::Text(format!("opened {name:?}")))
    }

    fn save_world(&mut self, a: &Args) -> Result<Output, String> {
        let path = match a.str("path")? {
            Some(p) => PathBuf::from(p),
            None => self.path.clone().ok_or(
                "this world has not been saved before; pass a path (e.g. C:/maps/island.gwork)",
            )?,
        };
        let world = self.world.as_ref().ok_or(NO_WORLD)?;
        groundwork_core::io::save(world, &path)
            .map_err(|e| format!("cannot save to {}: {e}", path.display()))?;
        self.path = Some(path.clone());
        self.dirty = false;
        Ok(Output::Text(format!("saved to {}", path.display())))
    }

    fn world_info(&self) -> Result<Output, String> {
        let world = self.world.as_ref().ok_or(NO_WORLD)?;
        let dims: Vec<Value> = world
            .dimensions()
            .iter()
            .map(|d| {
                let (low, high) = d.tiles().fold((f32::MAX, f32::MIN), |(l, h), t| {
                    (l.min(t.lowest_height()), h.max(t.highest_height()))
                });
                json!({
                    "dimension": format!("{:?}", d.kind),
                    "tiles": d.tile_count(),
                    "extent": d.extent(),
                    "build_height": [d.min_height, d.max_height],
                    "terrain_height_range": [low, high],
                    "subsurface_block": d.subsurface_block,
                })
            })
            .collect();
        Ok(text(json!({
            "name": world.name,
            "seed": (world.seed as i64).to_string(),
            "spawn": world.spawn,
            "game_mode": format!("{:?}", world.game_mode),
            "allow_cheats": world.allow_cheats,
            "map_features": world.map_features,
            "file": self.path.as_ref().map(|p| p.display().to_string()),
            "unsaved_changes": self.dirty,
            "undo_steps": self.undo.len(),
            "dimensions": dims,
        })))
    }

    fn get_column(&self, a: &Args) -> Result<Output, String> {
        let (x, y) = (a.req_i32("x")?, a.req_i32("y")?);
        let dim = self
            .world
            .as_ref()
            .ok_or(NO_WORLD)?
            .dimension(DimensionKind::Overworld)
            .unwrap();
        let height = dim
            .height_at(x, y)
            .ok_or_else(|| format!("{x},{y} is outside the painted area"))?;
        let terrain = dim.terrain_at(x, y).unwrap();
        let mut layer_values = Map::new();
        for layer in layers::ALL {
            layer_values.insert(layer.id.into(), json!(dim.layer_value_at(layer, x, y)));
        }
        let biome = dim.layer_value_at(layers::BIOME, x, y).unwrap();
        Ok(text(json!({
            "x": x,
            "y": y,
            "height": height,
            "block_height": dim.int_height_at(x, y),
            "water_level": dim.water_level_at(x, y),
            "slope": dim.slope_at(x, y),
            "terrain": terrain.info().id,
            "layers": layer_values,
            "biome": if biome == biomes::BIOME_AUTO { "auto" } else { biomes::modern_id(biome).unwrap_or("unknown") },
        })))
    }

    fn paint_layer(&mut self, a: &Args) -> Result<Output, String> {
        let layer_id = a.req_str("layer")?;
        let layer = layers::find(layer_id)
            .ok_or_else(|| format!("unknown layer {layer_id:?}; see list_layers"))?;
        let value = if layer.id == layers::BIOME.id {
            let biome = a.str("biome")?.ok_or(
                "painting the Biome layer requires a biome (name, minecraft id or 'auto')",
            )?;
            biomes::find(biome)
                .ok_or_else(|| format!("unknown biome {biome:?}; see list_biomes"))?
        } else {
            let v = a.i32("value")?.unwrap_or(layer.size.max_value() as i32);
            u8::try_from(v)
                .ok()
                .filter(|v| *v <= layer.size.max_value())
                .ok_or_else(|| {
                    format!(
                        "{} values range from 0 to {}",
                        layer.id,
                        layer.size.max_value()
                    )
                })?
        };
        let (shape, falloff) = (a.shape()?, a.falloff(Falloff::Constant)?);
        self.edit(|d| {
            Ok(format!(
                "set {} to {value} on {} columns",
                layer.id,
                ops::paint_layer(d, shape, falloff, layer, value)
            ))
        })
    }

    fn add_tiles(&mut self, a: &Args) -> Result<Output, String> {
        let (tx0, ty0, tx1, ty1) = (
            a.req_i32("tx0")?,
            a.req_i32("ty0")?,
            a.req_i32("tx1")?,
            a.req_i32("ty1")?,
        );
        let (tx0, tx1, ty0, ty1) = (tx0.min(tx1), tx0.max(tx1), ty0.min(ty1), ty0.max(ty1));
        let height = a.f32("height")?;
        let water = a.i32("water_level")?;
        let terrain = match a.str("terrain")? {
            Some(t) => find_terrain(t)?,
            None => Terrain::Grass,
        };
        let requested = (tx1 - tx0 + 1) as u64 * (ty1 - ty0 + 1) as u64;
        self.edit(|d| {
            if d.tile_count() as u64 + requested > MAX_TILES {
                return Err(format!("would exceed the limit of {MAX_TILES} tiles"));
            }
            let (min, height) = (d.min_height, height.unwrap_or(64.0));
            let water = water.unwrap_or(min);
            let mut added = 0;
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    if d.tile(tx, ty).is_some() {
                        continue;
                    }
                    let tile = d.ensure_tile(tx, ty);
                    for ly in 0..groundwork_core::TILE_SIZE {
                        for lx in 0..groundwork_core::TILE_SIZE {
                            tile.set_height(lx, ly, height);
                            tile.set_water_level(lx, ly, water);
                            tile.set_terrain(lx, ly, terrain);
                        }
                    }
                    added += 1;
                }
            }
            Ok(format!(
                "added {added} tiles; extent is now {:?}",
                d.extent().unwrap()
            ))
        })
    }

    fn set_world_settings(&mut self, a: &Args) -> Result<Output, String> {
        let world = self.world.as_mut().ok_or(NO_WORLD)?;
        if let Some(name) = a.str("name")? {
            world.name = name.to_owned();
        }
        if let Some(mode) = a.str("game_mode")? {
            world.game_mode =
                GameMode::parse(mode).ok_or_else(|| format!("unknown game mode {mode:?}"))?;
        }
        if let Some(v) = a.bool("allow_cheats")? {
            world.allow_cheats = v;
        }
        if let Some(v) = a.bool("map_features")? {
            world.map_features = v;
        }
        let dim = world.dimension_mut(DimensionKind::Overworld).unwrap();
        if let Some(block) = a.str("subsurface_block")? {
            dim.subsurface_block = if block.contains(':') {
                block.to_owned()
            } else {
                format!("minecraft:{block}")
            };
        }
        if let Some(depth) = a.i32("top_layer_depth")? {
            dim.top_layer_depth = depth.clamp(0, 64);
        }
        if let Some(bedrock) = a.bool("bedrock")? {
            dim.bedrock = bedrock;
        }
        self.dirty = true;
        self.world_info()
    }

    fn render_preview(&self, a: &Args) -> Result<Output, String> {
        let dim = self
            .world
            .as_ref()
            .ok_or(NO_WORLD)?
            .dimension(DimensionKind::Overworld)
            .unwrap();
        let scale = a.i32("scale")?.unwrap_or(1).max(1) as u32;
        let max_size = a.i32("max_size")?.unwrap_or(512).clamp(64, 2048) as u32;
        let (image, scale) =
            render::render(dim, scale, max_size).ok_or("the world has no tiles")?;
        let extent = dim.extent().unwrap();
        let png = image
            .to_png()
            .map_err(|e| format!("PNG encoding failed: {e}"))?;
        Ok(Output::Image {
            png,
            text: format!(
                "{}×{} px, 1 px = {scale} block(s). Top-left pixel is x={}, y={}; x grows right (east), y grows down (south). \
                 Water is blue, frost is white, void is black.",
                image.width, image.height, extent.x0, extent.y0
            ),
        })
    }

    fn export(&self, a: &Args) -> Result<Output, String> {
        let world = self.world.as_ref().ok_or(NO_WORLD)?;
        let path = PathBuf::from(a.req_str("path")?);
        let outside = match a.str("outside")?.unwrap_or("vanilla") {
            "vanilla" => Outside::Vanilla,
            "void" => Outside::Void,
            "ocean" => Outside::Ocean,
            other => {
                return Err(format!(
                    "unknown outside mode {other:?}; use vanilla, void or ocean"
                ))
            }
        };
        let version = match a.str("minecraft_version")? {
            None => McVersion::default(),
            Some(v) => McVersion::parse(v).ok_or_else(|| {
                format!("unsupported Minecraft version {v:?}; use 1.21.11 or 1.21.1")
            })?,
        };
        let options = ExportOptions {
            outside,
            overwrite: a.bool("overwrite")?.unwrap_or(false),
            version,
            threads: a.i32("threads")?.map(|t| t.max(1) as usize),
        };
        let started = std::time::Instant::now();
        let report = groundwork_export::export(world, &path, &options)
            .map_err(|e| format!("export failed: {e}"))?;
        Ok(text(json!({
            "exported_to": report.path.display().to_string(),
            "minecraft_version": version.name(),
            "chunks": report.chunks,
            "region_files": report.regions,
            "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
            "warnings": report.warnings,
        })))
    }
}

fn positive(v: Option<i32>, default: u32, name: &str) -> Result<u32, String> {
    match v {
        None => Ok(default),
        Some(n) if n > 0 => Ok(n as u32),
        Some(_) => Err(format!("{name} must be at least 1")),
    }
}

fn find_terrain(name: &str) -> Result<Terrain, String> {
    Terrain::find(name).ok_or_else(|| format!("unknown terrain {name:?}; see list_terrains"))
}

fn list_terrains() -> Output {
    let rows: Vec<Value> = Terrain::all()
        .iter()
        .filter(|t| t.fidelity != Fidelity::Custom)
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "surface": t.top,
                "approximated": t.fidelity == Fidelity::Approximate,
            })
        })
        .collect();
    text(
        json!({ "terrains": rows, "note": "Custom 1–96 terrains exist but are not supported yet" }),
    )
}

fn list_biomes() -> Output {
    let rows: Vec<Value> = biomes::all()
        .map(|(id, name, modern)| json!({ "id": id, "name": name, "minecraft_id": modern }))
        .collect();
    text(
        json!({ "biomes": rows, "auto": "use 'auto' (id 255) to let the exporter choose from terrain and water" }),
    )
}

fn list_layers() -> Output {
    let rows: Vec<Value> = layers::ALL
        .iter()
        .map(|l| json!({ "id": l.id, "description": l.description, "max_value": l.size.max_value(), "default": l.default }))
        .collect();
    text(json!({ "layers": rows }))
}

// ------------------------------------------------------------- schemas --

fn brush_properties() -> Map<String, Value> {
    let v = json!({
        "x": { "type": "integer", "description": "Brush centre, east-west block coordinate" },
        "y": { "type": "integer", "description": "Brush centre, north-south block coordinate (Minecraft z)" },
        "radius": { "type": "number", "minimum": 0, "description": "Brush radius in blocks (default 0 = one column)" },
        "brush": { "type": "string", "enum": ["circle", "square"], "description": "Brush shape (default circle)" },
        "x0": { "type": "integer", "description": "Rectangle mode: first corner x (use instead of x/y/radius)" },
        "y0": { "type": "integer", "description": "Rectangle mode: first corner y" },
        "x1": { "type": "integer", "description": "Rectangle mode: opposite corner x (inclusive)" },
        "y1": { "type": "integer", "description": "Rectangle mode: opposite corner y (inclusive)" },
        "falloff": { "type": "string", "enum": ["constant", "linear", "smooth", "spike"], "description": "How the effect fades towards the edge" },
    });
    v.as_object().unwrap().clone()
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    mut properties: Map<String, Value>,
    extra: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    if let Value::Object(extra) = extra {
        properties.extend(extra);
    }
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
        "annotations": { "readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false },
    })
}

pub fn definitions() -> Vec<Value> {
    let none = Map::new;
    let discard = json!({ "discard_unsaved": { "type": "boolean", "description": "Discard unsaved changes to the currently open world" } });
    let mut create_props = none();
    create_props.extend(discard.as_object().unwrap().clone());
    vec![
        tool("create_world", "Create world",
            "Create a new world from a noise height map, replacing the open world. The area is centred on 0,0 and measured in 128-block tiles.",
            create_props,
            json!({
                "name": { "type": "string" },
                "width_tiles": { "type": "integer", "minimum": 1, "description": "East-west size in tiles (default 4 = 512 blocks)" },
                "length_tiles": { "type": "integer", "minimum": 1, "description": "North-south size in tiles (default 4)" },
                "seed": { "type": ["integer", "string"], "description": "World seed (default random)" },
                "base_height": { "type": "number", "description": "Average land height (default 64)" },
                "water_level": { "type": "integer", "description": "Sea level (default 62)" },
                "hilliness": { "type": "number", "minimum": 0, "description": "Max height deviation in blocks (default 20; 0 = flat)" },
                "feature_size": { "type": "number", "description": "Horizontal size of hills in blocks (default 200)" },
                "terrain": { "type": "string", "description": "Terrain for land (default GRASS; see list_terrains)" },
                "beaches": { "type": "boolean", "description": "Beach terrain around the water line (default true)" },
                "min_height": { "type": "integer", "description": "Bottom of the world (default -64)" },
                "max_height": { "type": "integer", "description": "Top of the world, exclusive (default 320)" },
            }),
            &["name"], false),
        tool("open_world", "Open world", "Open a world saved with save_world (.gwork). WorldPainter .world files are not supported yet.",
            none(), json!({ "path": { "type": "string" }, "discard_unsaved": discard["discard_unsaved"] }), &["path"], false),
        tool("save_world", "Save world", "Save the open world. Without a path, saves to where it was last saved or opened.",
            none(), json!({ "path": { "type": "string", "description": "File path, conventionally ending in .gwork" } }), &[], false),
        tool("world_info", "World info", "Name, seed, spawn, extent, height ranges and save state of the open world.", none(), json!({}), &[], true),
        tool("list_terrains", "List terrains", "Terrain types that can be painted.", none(), json!({}), &[], true),
        tool("list_biomes", "List biomes", "Biomes that can be painted with the Biome layer.", none(), json!({}), &[], true),
        tool("list_layers", "List layers", "Layers that can be painted with paint_layer.", none(), json!({}), &[], true),
        tool("get_column", "Inspect column", "Height, water level, terrain, slope and layers of one column.",
            none(), json!({ "x": { "type": "integer" }, "y": { "type": "integer" } }), &["x", "y"], true),
        tool("raise_lower", "Raise or lower", "Raise (positive amount) or lower (negative) terrain under a brush. Default falloff: smooth.",
            brush_properties(), json!({ "amount": { "type": "number", "description": "Blocks at full brush strength" } }), &["amount"], false),
        tool("flatten", "Flatten", "Level terrain to a height (default: the height at the brush centre). Default falloff: constant.",
            brush_properties(), json!({ "height": { "type": "number" } }), &[], false),
        tool("smooth", "Smooth", "Smooth out bumps and sharp edges. Default falloff: smooth.",
            brush_properties(), json!({ "iterations": { "type": "integer", "minimum": 1, "maximum": 20, "description": "Strength (default 2)" } }), &[], false),
        tool("paint_terrain", "Paint terrain", "Paint a terrain type (surface material). Partial falloff strength scatters the terrain.",
            brush_properties(), json!({ "terrain": { "type": "string", "description": "Terrain id or name, e.g. SAND, Bare Grass" } }), &["terrain"], false),
        tool("paint_layer", "Paint layer", "Paint a layer: Frost, Void, FloodWithLava (value 1 to add, 0 to remove), Deciduous or Pine forest (value 0–15 sets tree density; 0 removes), or Biome (give biome).",
            brush_properties(),
            json!({
                "layer": { "type": "string", "enum": ["Frost", "Void", "FloodWithLava", "Biome", "Deciduous", "Pine"] },
                "value": { "type": "integer", "minimum": 0, "description": "Layer value (default: on)" },
                "biome": { "type": "string", "description": "For the Biome layer: name, minecraft id or 'auto'" },
            }),
            &["layer"], false),
        tool("flood", "Flood", "Fill the basin containing x,y with water (or lava) up to level, like a bucket fill. A level at or below the ground drains the connected water instead.",
            none(),
            json!({
                "x": { "type": "integer" }, "y": { "type": "integer" },
                "level": { "type": "integer", "description": "Y of the highest water block" },
                "lava": { "type": "boolean" },
            }),
            &["x", "y", "level"], false),
        tool("set_water_level", "Set water level", "Set the water level of every column under a brush, regardless of terrain (water only shows where terrain is lower).",
            brush_properties(), json!({ "level": { "type": "integer" }, "lava": { "type": "boolean" } }), &["level"], false),
        tool("add_tiles", "Add tiles", "Extend the world with flat 128×128 tiles in an inclusive range of tile coordinates (tile = floor(block / 128)). Existing tiles are kept.",
            none(),
            json!({
                "tx0": { "type": "integer" }, "ty0": { "type": "integer" }, "tx1": { "type": "integer" }, "ty1": { "type": "integer" },
                "height": { "type": "number", "description": "Land height (default 64)" },
                "water_level": { "type": "integer", "description": "Default: no water" },
                "terrain": { "type": "string" },
            }),
            &["tx0", "ty0", "tx1", "ty1"], false),
        tool("set_spawn", "Set spawn", "Set the player spawn point.", none(), json!({ "x": { "type": "integer" }, "y": { "type": "integer" } }), &["x", "y"], false),
        tool("set_world_settings", "World settings", "Change world name, game mode, cheats, structures, or the underground fill.",
            none(),
            json!({
                "name": { "type": "string" },
                "game_mode": { "type": "string", "enum": ["survival", "creative", "adventure", "spectator", "hardcore"] },
                "allow_cheats": { "type": "boolean" },
                "map_features": { "type": "boolean", "description": "Generate vanilla structures outside the painted area" },
                "subsurface_block": { "type": "string", "description": "Block below the top layer (default stone, with deepslate below y=0)" },
                "top_layer_depth": { "type": "integer", "description": "Minimum depth of the terrain material (default 3)" },
                "bedrock": { "type": "boolean", "description": "Bedrock floor at the bottom of the world" },
            }),
            &[], false),
        tool("render_preview", "Render preview", "Render a shaded top-down map of the world as a PNG image.",
            none(),
            json!({
                "scale": { "type": "integer", "minimum": 1, "description": "Blocks per pixel (raised automatically to fit max_size)" },
                "max_size": { "type": "integer", "minimum": 64, "maximum": 2048, "description": "Longest image side in pixels (default 512)" },
            }),
            &[], true),
        tool("export_minecraft", "Export to Minecraft",
            "Write a playable Minecraft Java Edition save (1.21.11 by default, or 1.21.1 for older 1.21 releases). Point path at a new folder, e.g. inside .minecraft/saves.",
            none(),
            json!({
                "path": { "type": "string", "description": "Save directory to create" },
                "outside": { "type": "string", "enum": ["vanilla", "void", "ocean"], "description": "What generates beyond the painted area: vanilla terrain (default), empty void, or endless ocean" },
                "overwrite": { "type": "boolean", "description": "Write into a folder that already holds a world" },
                "minecraft_version": { "type": "string", "enum": ["1.21.11", "1.21.1"], "description": "Save format (default 1.21.11)" },
                "threads": { "type": "integer", "minimum": 1, "description": "Worker threads (default: all cores)" },
            }),
            &["path"], false),
        tool("undo", "Undo", "Revert the last edit (up to 50 steps).", none(), json!({}), &[], false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(server: &mut Server, name: &str, args: Value) -> Value {
        let r = server.call(name, &args);
        assert_eq!(
            r["isError"], false,
            "{name} failed: {}",
            r["content"][0]["text"]
        );
        r
    }

    fn err(server: &mut Server, name: &str, args: Value) -> String {
        let r = server.call(name, &args);
        assert_eq!(r["isError"], true, "{name} unexpectedly succeeded");
        r["content"][0]["text"].as_str().unwrap().to_owned()
    }

    #[test]
    fn full_workflow() {
        let mut s = Server::default();
        assert!(err(&mut s, "world_info", json!({})).contains("no world"));
        ok(
            &mut s,
            "create_world",
            json!({ "name": "Isle", "width_tiles": 2, "length_tiles": 2, "seed": 5, "hilliness": 0, "base_height": 70, "water_level": 0 }),
        );
        ok(
            &mut s,
            "flatten",
            json!({ "x": 0, "y": 0, "radius": 12, "height": 50 }),
        );
        let flood = ok(&mut s, "flood", json!({ "x": 0, "y": 0, "level": 60 }));
        assert!(flood["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("flooded"));
        ok(
            &mut s,
            "paint_terrain",
            json!({ "x0": 30, "y0": 30, "x1": 40, "y1": 40, "terrain": "sand" }),
        );
        ok(
            &mut s,
            "paint_layer",
            json!({ "x": 50, "y": 50, "radius": 5, "layer": "biome", "biome": "desert" }),
        );
        ok(
            &mut s,
            "paint_layer",
            json!({ "x": 0, "y": 0, "radius": 20, "layer": "Frost" }),
        );
        ok(
            &mut s,
            "raise_lower",
            json!({ "x": -60, "y": -60, "radius": 20, "amount": 30 }),
        );
        let column = ok(&mut s, "get_column", json!({ "x": 50, "y": 50 }));
        let column: Value =
            serde_json::from_str(column["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(column["biome"], "minecraft:desert");

        let preview = ok(&mut s, "render_preview", json!({ "max_size": 128 }));
        assert_eq!(preview["content"][0]["type"], "image");

        assert!(err(&mut s, "create_world", json!({ "name": "Other" })).contains("unsaved"));
        ok(&mut s, "undo", json!({}));
        let info = ok(&mut s, "world_info", json!({}));
        assert!(info["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"undo_steps\": 5"));

        let dir = std::env::temp_dir().join(format!("groundwork-mcp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("isle.gwork");
        ok(&mut s, "save_world", json!({ "path": file }));
        ok(&mut s, "open_world", json!({ "path": file }));
        let export = ok(
            &mut s,
            "export_minecraft",
            json!({ "path": dir.join("save") }),
        );
        assert!(export["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"chunks\": 256"));
        assert!(dir.join("save").join("level.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_bad_arguments() {
        let mut s = Server::default();
        ok(
            &mut s,
            "create_world",
            json!({ "name": "W", "width_tiles": 1, "length_tiles": 1 }),
        );
        assert!(err(
            &mut s,
            "paint_terrain",
            json!({ "x": 0, "y": 0, "terrain": "unobtainium" })
        )
        .contains("unknown terrain"));
        assert!(err(&mut s, "raise_lower", json!({ "x": 0, "y": 0 })).contains("amount"));
        assert!(err(&mut s, "flatten", json!({ "x": "zero", "y": 0 })).contains("number"));
        assert!(err(&mut s, "create_world", json!({ "name": "Big", "width_tiles": 100, "length_tiles": 100, "discard_unsaved": true })).contains("limit"));
        assert!(err(&mut s, "nonexistent", json!({})).contains("unknown tool"));
        // A failed flood must not leave an undo step behind.
        let before = s.undo.len();
        err(&mut s, "flood", json!({ "x": 9999, "y": 0, "level": 80 }));
        assert_eq!(s.undo.len(), before);
    }
}

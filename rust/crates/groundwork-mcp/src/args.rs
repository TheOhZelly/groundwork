//! Typed access to tool-call arguments.

use groundwork_core::ops::{Falloff, Shape};
use serde_json::Value;

pub struct Args<'a>(pub &'a Value);

impl Args<'_> {
    fn raw(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    pub fn has(&self, key: &str) -> bool {
        self.raw(key).is_some()
    }

    pub fn f64(&self, key: &str) -> Result<Option<f64>, String> {
        self.raw(key)
            .map(|v| v.as_f64().ok_or_else(|| format!("{key} must be a number")))
            .transpose()
    }

    pub fn f32(&self, key: &str) -> Result<Option<f32>, String> {
        Ok(self.f64(key)?.map(|v| v as f32))
    }

    pub fn i32(&self, key: &str) -> Result<Option<i32>, String> {
        self.f64(key)?
            .map(|v| {
                if v.fract() != 0.0 || v < i32::MIN as f64 || v > i32::MAX as f64 {
                    Err(format!("{key} must be a whole number"))
                } else {
                    Ok(v as i32)
                }
            })
            .transpose()
    }

    pub fn u64(&self, key: &str) -> Result<Option<u64>, String> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::Number(n)) => n
                .as_u64()
                .or_else(|| n.as_i64().map(|i| i as u64))
                .map(Some)
                .ok_or_else(|| format!("{key} must be an integer")),
            // Minecraft seeds are often quoted to survive JSON number precision.
            Some(Value::String(s)) => s
                .parse::<i64>()
                .map(|i| Some(i as u64))
                .or_else(|_| s.parse::<u64>().map(Some))
                .map_err(|_| format!("{key} must be an integer")),
            Some(_) => Err(format!("{key} must be an integer")),
        }
    }

    pub fn bool(&self, key: &str) -> Result<Option<bool>, String> {
        self.raw(key)
            .map(|v| {
                v.as_bool()
                    .ok_or_else(|| format!("{key} must be true or false"))
            })
            .transpose()
    }

    pub fn str(&self, key: &str) -> Result<Option<&str>, String> {
        self.raw(key)
            .map(|v| v.as_str().ok_or_else(|| format!("{key} must be a string")))
            .transpose()
    }

    pub fn req_i32(&self, key: &str) -> Result<i32, String> {
        self.i32(key)?
            .ok_or_else(|| format!("missing required argument {key}"))
    }

    pub fn req_f32(&self, key: &str) -> Result<f32, String> {
        self.f32(key)?
            .ok_or_else(|| format!("missing required argument {key}"))
    }

    pub fn req_str(&self, key: &str) -> Result<&str, String> {
        self.str(key)?
            .ok_or_else(|| format!("missing required argument {key}"))
    }

    /// Brush area: a rectangle if `x0..y1` are given, otherwise a circle or
    /// square of `radius` around `x`, `y`.
    pub fn shape(&self) -> Result<Shape, String> {
        let shape = if ["x0", "y0", "x1", "y1"].iter().any(|k| self.has(k)) {
            Shape::Rect {
                x0: self.req_i32("x0")?,
                y0: self.req_i32("y0")?,
                x1: self.req_i32("x1")?,
                y1: self.req_i32("y1")?,
            }
        } else {
            let (x, y) = (self.req_i32("x")?, self.req_i32("y")?);
            let radius = self.f32("radius")?.unwrap_or(0.0);
            if radius < 0.0 {
                return Err("radius must not be negative".into());
            }
            match self.str("brush")?.unwrap_or("circle") {
                "circle" => Shape::Circle { x, y, radius },
                "square" => Shape::Square {
                    x,
                    y,
                    radius: radius.round() as i32,
                },
                // Noise fixed in world space, so overlapping dabs agree.
                "organic" => Shape::Organic {
                    x,
                    y,
                    radius,
                    seed: 0x6f72_6761_6e69,
                },
                other => {
                    return Err(format!(
                        "unknown brush {other:?}; use circle, square or organic"
                    ))
                }
            }
        };
        const MAX_AREA: u64 = 16 * 1024 * 1024;
        if shape.area() > MAX_AREA {
            return Err(format!(
                "area of {} columns is too large (limit {MAX_AREA})",
                shape.area()
            ));
        }
        Ok(shape)
    }

    /// `dry_land_only: true` restricts painting to columns above water.
    pub fn mask(&self) -> Result<groundwork_core::ops::Mask, String> {
        Ok(if self.bool("dry_land_only")?.unwrap_or(false) {
            groundwork_core::ops::Mask::DryLand
        } else {
            groundwork_core::ops::Mask::All
        })
    }

    pub fn falloff(&self, default: Falloff) -> Result<Falloff, String> {
        match self.str("falloff")? {
            None => Ok(default),
            Some(s) => Falloff::parse(s).ok_or_else(|| {
                format!("unknown falloff {s:?}; use constant, linear, smooth or spike")
            }),
        }
    }
}

//! Reporter drawings and their cache (M13.6 draws them on the canvas).
//!
//! Output: `{"background": [primitive…], "foreground": [primitive…]}`; `{"error": "…"}` reports
//! a failure. Coordinates and radii are font units in the layer's space, within the model's
//! `MAX_COORDINATE`; stroke widths and text sizes are screen pixels from 0 to
//! [`MAX_SCREEN_PIXELS`]; colours are RGBA bytes. At most [`MAX_PRIMITIVES`] items in all, where
//! each primitive and each step of a primitive path counts as one; text items at most
//! [`MAX_TEXT_CHARS`] characters.
//!
//! The cache keys a drawing on the module's load serial, a hash of its parameters, the layer's
//! revision and a scale bucket ([`scale_bucket`]), so a reporter runs again only when one of
//! them changes.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use fontcraft_model::MAX_COORDINATE;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::capped::{capped, with_budget};
pub use crate::error::{MAX_PRIMITIVES, MAX_SCREEN_PIXELS, MAX_TEXT_CHARS};
use crate::layer_json::cut;
use crate::{Error, Result};

/// A colour as red, green, blue and alpha bytes.
pub type Rgba = [u8; 4];

/// One step of a primitive path, in font units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum PathCommand {
    MoveTo { to: [f64; 2] },
    LineTo { to: [f64; 2] },
    QuadTo { c: [f64; 2], to: [f64; 2] },
    CurveTo { c1: [f64; 2], c2: [f64; 2], to: [f64; 2] },
    Close,
}

/// A stroke: colour and width in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeStyle {
    pub color: Rgba,
    pub width: f64,
}

/// One drawing primitive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Primitive {
    /// A path, filled, stroked or both.
    Path {
        #[serde(deserialize_with = "capped")]
        commands: Vec<PathCommand>,
        #[serde(default)]
        fill: Option<Rgba>,
        #[serde(default)]
        stroke: Option<StrokeStyle>,
    },
    /// A straight line.
    Line { from: [f64; 2], to: [f64; 2], stroke: StrokeStyle },
    /// A circle; `radius` in font units.
    Circle {
        center: [f64; 2],
        radius: f64,
        #[serde(default)]
        fill: Option<Rgba>,
        #[serde(default)]
        stroke: Option<StrokeStyle>,
    },
    /// A text label; `size` in screen pixels.
    Text { at: [f64; 2], text: String, color: Rgba, size: f64 },
}

/// What a reporter draws for one layer: behind the outline and in front of it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    #[serde(default)]
    pub background: Vec<Primitive>,
    #[serde(default)]
    pub foreground: Vec<Primitive>,
}

/// The output as read; both lists and every path's steps share one item budget.
#[derive(Deserialize)]
struct DrawingOut {
    #[serde(default)]
    error: Option<String>,
    #[serde(default, deserialize_with = "capped")]
    background: Vec<Primitive>,
    #[serde(default, deserialize_with = "capped")]
    foreground: Vec<Primitive>,
}

fn point_ok(p: &[f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
}

fn pixels_ok(v: f64) -> bool {
    v.is_finite() && (0.0..=MAX_SCREEN_PIXELS).contains(&v)
}

fn stroke_ok(s: &Option<StrokeStyle>) -> bool {
    s.as_ref().is_none_or(|s| pixels_ok(s.width))
}

impl Primitive {
    /// What is wrong with this primitive, if anything.
    fn problem(&self) -> Option<String> {
        let bad_point = || Some(format!("a coordinate is beyond ±{MAX_COORDINATE}"));
        let bad_pixels = || Some(format!("a stroke width or text size is outside 0..{MAX_SCREEN_PIXELS} pixels"));
        match self {
            Primitive::Path { commands, stroke, .. } => {
                let points_ok = commands.iter().all(|c| match c {
                    PathCommand::MoveTo { to } | PathCommand::LineTo { to } => point_ok(to),
                    PathCommand::QuadTo { c, to } => point_ok(c) && point_ok(to),
                    PathCommand::CurveTo { c1, c2, to } => point_ok(c1) && point_ok(c2) && point_ok(to),
                    PathCommand::Close => true,
                });
                if !points_ok {
                    bad_point()
                } else if !stroke_ok(stroke) {
                    bad_pixels()
                } else {
                    None
                }
            }
            Primitive::Line { from, to, stroke } => {
                if !(point_ok(from) && point_ok(to)) {
                    bad_point()
                } else if !pixels_ok(stroke.width) {
                    bad_pixels()
                } else {
                    None
                }
            }
            Primitive::Circle { center, radius, stroke, .. } => {
                if !(point_ok(center) && radius.is_finite() && (0.0..=MAX_COORDINATE).contains(radius)) {
                    bad_point()
                } else if !stroke_ok(stroke) {
                    bad_pixels()
                } else {
                    None
                }
            }
            Primitive::Text { at, text, size, .. } => {
                if !point_ok(at) {
                    bad_point()
                } else if !pixels_ok(*size) {
                    bad_pixels()
                } else if text.chars().count() > MAX_TEXT_CHARS {
                    Some(format!("a text item is longer than {MAX_TEXT_CHARS} characters"))
                } else {
                    None
                }
            }
        }
    }
}

impl Drawing {
    /// Parses a reporter's output, refusing more than [`MAX_PRIMITIVES`] items, long text,
    /// coordinates beyond the model's limit and out-of-range pixel sizes (`{"error": "…"}` is
    /// [`Error::Failed`]).
    pub fn parse(bytes: &[u8]) -> Result<Drawing> {
        let (out, over) = with_budget(MAX_PRIMITIVES, || serde_json::from_slice::<DrawingOut>(bytes));
        if over {
            return Err(Error::Output(format!("more than {MAX_PRIMITIVES} primitives and path steps")));
        }
        let out = out.map_err(|e| Error::Output(e.to_string()))?;
        if let Some(e) = out.error {
            return Err(Error::Failed(cut(&e, crate::error::MAX_LOG_LINE_CHARS)));
        }
        if let Some(problem) = out.background.iter().chain(&out.foreground).find_map(Primitive::problem) {
            return Err(Error::Output(problem));
        }
        Ok(Drawing { background: out.background, foreground: out.foreground })
    }

    pub fn is_empty(&self) -> bool {
        self.background.is_empty() && self.foreground.is_empty()
    }
}

/// What a cached drawing depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// [`crate::Plugin::serial`] of the module.
    pub module: u64,
    /// A hash of the parameter object.
    pub params: u64,
    /// The layer it was drawn for: the caller's document id and glyph and layer identity hashed
    /// with the document revision, so any edit of the layer misses.
    pub layer: u64,
    /// The view scale rounded to a bucket ([`scale_bucket`]), so zooming within a bucket reuses
    /// the drawing.
    pub scale: i32,
}

/// Buckets per doubling of the view scale: zooming by up to about 9% reuses a drawing.
pub const SCALE_BUCKETS_PER_OCTAVE: f64 = 8.0;

/// The bucket of a view scale: `round(log2(scale) × 8)`, so drawings are shared between scales
/// within about 9% of each other. A scale that is not a positive finite number has bucket 0.
pub fn scale_bucket(scale: f64) -> i32 {
    if !(scale.is_finite() && scale > 0.0) {
        return 0;
    }
    // log2 of a positive finite f64 is within ±1075, so the product fits an i32.
    (scale.log2() * SCALE_BUCKETS_PER_OCTAVE).round() as i32
}

impl CacheKey {
    /// The key for module `serial` with the resolved `params`, the caller's `layer` identity and
    /// revision hash ([`layer_key`]) and the view `scale`.
    pub fn new(serial: u64, params: &Map<String, Value>, layer: u64, scale: f64) -> CacheKey {
        let mut h = DefaultHasher::new();
        // serde_json writes a map in key order, so equal objects give equal text.
        Value::Object(params.clone()).to_string().hash(&mut h);
        CacheKey { module: serial, params: h.finish(), layer, scale: scale_bucket(scale) }
    }
}

/// A layer's identity for [`CacheKey::layer`]: the caller's document id, the glyph name, the
/// layer id and the document revision, hashed. Any edit bumps the revision, so it misses.
pub fn layer_key(document: u64, glyph: &str, layer: &str, revision: u64) -> u64 {
    let mut h = DefaultHasher::new();
    (document, glyph, layer, revision).hash(&mut h);
    h.finish()
}

/// Most drawings kept; the cache is cleared when it grows past this.
pub const MAX_CACHED_DRAWINGS: usize = 4_096;

/// Reporter drawings by [`CacheKey`]. Shared across threads; methods take `&self`, so the
/// canvas can fill it while it holds the session read-only.
#[derive(Debug, Default)]
pub struct ReporterCache {
    drawings: Mutex<HashMap<CacheKey, Arc<Drawing>>>,
}

impl ReporterCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The drawing for `key`, running `run` (the reporter call and parse) when it is not cached.
    /// An error is returned, not cached.
    pub fn get_or_run(&self, key: CacheKey, run: impl FnOnce() -> Result<Drawing>) -> Result<Arc<Drawing>> {
        if let Some(d) = self.lock().get(&key) {
            return Ok(d.clone());
        }
        // The lock is not held while the module runs, so other layers can be drawn meanwhile.
        let drawing = Arc::new(run()?);
        let mut map = self.lock();
        if map.len() >= MAX_CACHED_DRAWINGS {
            map.clear();
        }
        map.insert(key, drawing.clone());
        Ok(drawing)
    }

    /// The cached drawing for `key`, without running anything.
    pub fn get(&self, key: &CacheKey) -> Option<Arc<Drawing>> {
        self.lock().get(key).cloned()
    }

    /// How many drawings are cached.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drops the drawings of modules whose serial is not in `serials` (after an uninstall or a
    /// replacement).
    pub fn retain_modules(&self, serials: &[u64]) {
        self.lock().retain(|k, _| serials.contains(&k.module));
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    /// The map; a poisoned lock is taken over, since every value in it is complete.
    fn lock(&self) -> MutexGuard<'_, HashMap<CacheKey, Arc<Drawing>>> {
        self.drawings.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

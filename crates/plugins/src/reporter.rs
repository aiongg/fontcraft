//! Reporter drawings and their cache (M13.2 fills the bodies; M13.6 draws them on the canvas).
//!
//! Output: `{"background": [primitive…], "foreground": [primitive…]}`. Coordinates are font
//! units in the layer's space; widths and text sizes are screen pixels; colours are RGBA bytes.
//! At most [`MAX_PRIMITIVES`] primitives in all, text items at most [`MAX_TEXT_CHARS`]
//! characters, every number finite.
//!
//! The cache keys a drawing on the module's load serial, a hash of its parameters, the layer's
//! revision and a scale bucket, so a reporter runs again only when one of them changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub use crate::error::{MAX_PRIMITIVES, MAX_TEXT_CHARS};
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

impl Drawing {
    /// Parses a reporter's output, refusing more than [`MAX_PRIMITIVES`] primitives, long text and
    /// non-finite numbers (`{"error": "…"}` is [`Error::Failed`]).
    pub fn parse(bytes: &[u8]) -> Result<Drawing> {
        let _ = bytes;
        // M13.2.
        Err(Error::Unsupported("reporter output".into()))
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
    /// The view scale rounded to a bucket (M13.2 picks the buckets), so zooming within a bucket
    /// reuses the drawing.
    pub scale: i32,
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
        let _ = (key, run);
        // M13.2.
        Err(Error::Unsupported("reporter cache".into()))
    }

    /// Drops the drawings of modules whose serial is not in `serials` (after an uninstall or a
    /// replacement).
    pub fn retain_modules(&self, serials: &[u64]) {
        let _ = serials;
        // M13.2.
    }

    pub fn clear(&self) {
        if let Ok(mut d) = self.drawings.lock() {
            d.clear();
        }
    }
}

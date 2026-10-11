//! The layer JSON handed to modules, and a filter's output checked and applied to a layer
//! (M13.2 fills the bodies).
//!
//! Input (one object per call): the glyph's name and code points, the layer in the model's JSON
//! form (paths, components, anchors, width), the master's metric values by kind, and optionally
//! the Edit View selection (filters) or the view scale and active flag (reporters).
//!
//! Filter output: `{"shapes": [...], "anchors": [...], "width"?: n, "log"?: ["…"]}` in the same
//! model JSON form. [`FilterOutput::apply`] checks every value against the model's limits
//! (`MAX_OUTLINE_ELEMENTS`, `MAX_COORDINATE`, finite numbers, `MAX_NAME_LEN`) before it builds the
//! new layer, so a bad output changes nothing; the engine applies every layer's result in one
//! edit. Locked and hidden paths go to filters like the others and may change
//! (`plugin.lockedPathsAndGlyphs`); a locked glyph is refused by the engine before any call.

use std::collections::BTreeMap;

use fontcraft_model::{Anchor, Font, Glyph, Layer, Shape};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::error::{MAX_LOG_LINE_CHARS, MAX_LOG_LINES};
use crate::{Error, Result};

/// A selected node: the shape index in the layer and the node index in that path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeRef {
    pub shape: usize,
    pub node: usize,
}

/// What a module receives for one layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerInput {
    pub glyph: String,
    pub unicodes: Vec<u32>,
    /// The layer in the model's JSON form.
    pub layer: Value,
    /// The layer's master's metric values by kind (`ascender`, `cap height`, `x-height`,
    /// `baseline`, `descender`, `italic angle`…).
    pub metrics: BTreeMap<String, f64>,
    /// The Edit View selection on this layer (filters run from Edit View); empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selection: Vec<NodeRef>,
    /// The view scale (screen points per font unit), for reporters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// Whether the glyph is the one being edited, for reporters.
    #[serde(default)]
    pub active: bool,
}

impl LayerInput {
    /// The input for `layer` of `glyph` in `font`, without selection or view data.
    pub fn from_layer(font: &Font, glyph: &Glyph, layer: &Layer) -> Result<LayerInput> {
        let _ = (font, glyph, layer);
        // M13.2: the layer JSON and the master's metrics.
        Err(Error::Unsupported("module layer input".into()))
    }

    /// This input with the Edit View selection.
    pub fn with_selection(mut self, selection: Vec<NodeRef>) -> Self {
        self.selection = selection;
        self
    }

    /// This input with the view scale and the active flag (reporters).
    pub fn with_view(mut self, scale: f64, active: bool) -> Self {
        self.scale = Some(scale);
        self.active = active;
        self
    }

    /// The input as the UTF-8 JSON handed to `fc_run`.
    pub fn to_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| Error::Output(format!("layer input: {e}")))
    }
}

/// A filter's result for one layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterOutput {
    pub shapes: Vec<Shape>,
    pub anchors: Vec<Anchor>,
    /// A new advance width; `None` keeps the layer's.
    pub width: Option<f64>,
    /// Log lines for the Command Console, at most [`MAX_LOG_LINES`], each cut at
    /// [`MAX_LOG_LINE_CHARS`] characters.
    pub log: Vec<String>,
}

impl FilterOutput {
    /// Parses a filter's output JSON (`{"error": "…"}` is [`Error::Failed`]).
    pub fn parse(bytes: &[u8]) -> Result<FilterOutput> {
        let _ = bytes;
        // M13.2: parse, cap the log.
        Err(Error::Unsupported("filter output".into()))
    }

    /// `layer` with its shapes, anchors and width replaced by this output, after every value is
    /// checked against the model's limits. Everything else on the layer (guides, hints, user data)
    /// is kept.
    pub fn apply(&self, layer: &Layer) -> Result<Layer> {
        let _ = layer;
        // M13.2: check against the model's limits, then build the layer.
        Err(Error::Unsupported("applying filter output".into()))
    }
}

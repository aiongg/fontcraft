//! The layer JSON handed to modules, and a filter's output checked and applied to a layer.
//!
//! Input (one object per call): the glyph's name and code points, the layer in the native
//! format's layer form (the model's own JSON: `id`, `master`, `width`, `shapes`, `anchors`,
//! `info`…), the master's plain metric positions by kind, and optionally the Edit View selection
//! (filters) or the view scale and active flag (reporters).
//!
//! Filter output: `{"shapes": [...], "anchors"?: [...], "width"?: n, "log"?: ["…"]}` with shapes
//! and anchors in the same form; other keys are ignored, so a filter may return the `layer`
//! object it was given, changed. `shapes` replaces the layer's shapes (images and kept shapes
//! included); without `anchors` or `width` the layer keeps its own. `{"error": "…"}` reports a
//! failure. [`FilterOutput::apply`] checks every value against the model's limits
//! (`MAX_OUTLINE_ELEMENTS`, `MAX_COORDINATE`, finite numbers, `MAX_NAME_LEN`) before it builds the
//! new layer, so a bad output changes nothing; the engine applies every layer's result in one
//! edit. Locked and hidden paths go to filters like the others and may change
//! (`plugin.lockedPathsAndGlyphs`); a locked glyph is refused by the engine before any call.

use std::collections::BTreeMap;

use fontcraft_model::{Anchor, Font, Glyph, Layer, MAX_NAME_LEN, MAX_OUTLINE_ELEMENTS, Shape};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capped::{capped_opt, with_budget};
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
    /// The layer in the native format's layer form.
    pub layer: Value,
    /// The layer's master's plain metric positions by kind (`ascender`, `cap height`,
    /// `x-height`, `baseline`, `descender`, `italic angle`…).
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
    /// The input for `layer` of `glyph` in `font`, without selection or view data. A layer whose
    /// master is missing gets no metrics.
    pub fn from_layer(font: &Font, glyph: &Glyph, layer: &Layer) -> Result<LayerInput> {
        let layer_json = serde_json::to_value(layer).map_err(|e| Error::Output(format!("layer input: {e}")))?;
        let mut metrics = BTreeMap::new();
        if let Ok(master) = font.master(&layer.master) {
            for def in font.metrics.iter().filter(|d| d.name.is_none() && d.filter.is_none()) {
                if let Some(v) = font.metric(master, &def.kind) {
                    metrics.entry(def.kind.clone()).or_insert(v.pos);
                }
            }
        }
        Ok(LayerInput {
            glyph: glyph.name.clone(),
            unicodes: glyph.unicodes.clone(),
            layer: layer_json,
            metrics,
            selection: Vec::new(),
            scale: None,
            active: false,
        })
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
    /// New anchors; `None` keeps the layer's.
    pub anchors: Option<Vec<Anchor>>,
    /// A new advance width; `None` keeps the layer's.
    pub width: Option<f64>,
    /// Log lines for the Command Console, at most [`MAX_LOG_LINES`] (later lines are dropped),
    /// each cut at [`MAX_LOG_LINE_CHARS`] characters.
    pub log: Vec<String>,
}

/// The output as read; shapes and anchors share one element budget.
#[derive(Deserialize)]
struct FilterOut {
    #[serde(default)]
    error: Option<String>,
    #[serde(default, deserialize_with = "capped_opt")]
    shapes: Option<Vec<Shape>>,
    #[serde(default, deserialize_with = "capped_opt")]
    anchors: Option<Vec<Anchor>>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    log: Vec<Value>,
}

/// `s` cut to at most `max` characters.
pub(crate) fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

impl FilterOutput {
    /// Parses a filter's output JSON (`{"error": "…"}` is [`Error::Failed`]).
    pub fn parse(bytes: &[u8]) -> Result<FilterOutput> {
        let (out, over) = with_budget(MAX_OUTLINE_ELEMENTS, || serde_json::from_slice::<FilterOut>(bytes));
        if over {
            return Err(Error::Output(format!("more than {MAX_OUTLINE_ELEMENTS} shapes and anchors")));
        }
        let out = out.map_err(|e| Error::Output(e.to_string()))?;
        if let Some(e) = out.error {
            return Err(Error::Failed(cut(&e, MAX_LOG_LINE_CHARS)));
        }
        let shapes = out.shapes.ok_or_else(|| Error::Output("a filter's output must list the layer's `shapes`".into()))?;
        let log = out
            .log
            .iter()
            .take(MAX_LOG_LINES)
            .map(|v| match v {
                Value::String(s) => cut(s, MAX_LOG_LINE_CHARS),
                v => cut(&v.to_string(), MAX_LOG_LINE_CHARS),
            })
            .collect();
        Ok(FilterOutput { shapes, anchors: out.anchors, width: out.width, log })
    }

    /// `layer` with its shapes, anchors and width replaced by this output, after every value is
    /// checked against the model's limits. Everything else on the layer (guides, hints, user data)
    /// is kept.
    pub fn apply(&self, layer: &Layer) -> Result<Layer> {
        let anchors = self.anchors.as_ref().unwrap_or(&layer.anchors);
        let mut elements = anchors.len();
        for shape in &self.shapes {
            elements = elements.saturating_add(match shape {
                Shape::Path(p) => p.nodes.len().max(1),
                _ => 1,
            });
        }
        if elements > MAX_OUTLINE_ELEMENTS {
            return Err(Error::Output(format!("the layer would hold {elements} nodes, components and anchors (limit {MAX_OUTLINE_ELEMENTS})")));
        }
        if let Some(a) = anchors.iter().find(|a| a.name.is_empty() || a.name.len() > MAX_NAME_LEN || a.name.chars().any(char::is_control)) {
            return Err(Error::Output(format!("anchor name {:?} must be 1 to {MAX_NAME_LEN} bytes without control characters", cut(&a.name, 64))));
        }
        let new = Layer {
            id: layer.id.clone(),
            master: layer.master.clone(),
            width: self.width.unwrap_or(layer.width),
            shapes: self.shapes.clone(),
            anchors: anchors.clone(),
            info: layer.info.clone(),
            extra: layer.extra.clone(),
        };
        new.check_contents().map_err(Error::Output)?;
        Ok(new)
    }
}

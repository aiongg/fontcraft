//! The Shape Order panel's view model: the shapes of every layer of one glyph that takes part in interpolation, in
//! stacking order, with their outlines for the thumbnails.

use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::interp::labels::{brace_label, bracket_label};
use fontcraft_engine::model::{Font, Glyph, Layer, Shape};

use super::ProjectCx;
use super::font_view::{CellArt, flatten};
use crate::app::ViewState;
use crate::shell::dock::PanelId;
use crate::shell::masters;

/// Most layers the panel lists; the rest are counted in [`ShapeOrderModel::more_layers`].
pub const MAX_COLUMNS: usize = 64;
/// Most shapes one layer lists; the rest are counted by [`ShapeColumn::total`].
pub const MAX_SHAPES: usize = 256;
/// Most outline points all thumbnails hold together; shapes past it are listed without an outline.
const MAX_POINTS: usize = 262_144;

/// What the panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ShapeOrderModel {
    /// The glyph shown; `None` without a font or a glyph.
    pub glyph: Option<String>,
    /// One entry per layer: each master's layer, then that master's intermediate and alternate layers.
    pub columns: Vec<ShapeColumn>,
    /// Layers past [`MAX_COLUMNS`], left out.
    pub more_layers: usize,
}

/// One layer and its shapes.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ShapeColumn {
    /// The layer id `glyph.setShapeOrder` takes.
    pub layer: String,
    /// The master's name, with the brace or bracket label of a special layer.
    pub title: String,
    /// The first [`MAX_SHAPES`] shapes in stacking order: shape 0 first.
    pub shapes: Vec<ShapeEntry>,
    /// Every shape of the layer, the ones not listed included.
    pub total: usize,
    /// The outlines of all listed shapes together, `[min x, min y, max x, max y]` in font units: every thumbnail of
    /// the layer is fitted to it. `None` when no listed shape has an outline.
    pub bounds: Option<[f32; 4]>,
}

/// What a shape is.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub enum ShapeKind {
    Path {
        nodes: usize,
        closed: bool,
    },
    Component {
        glyph: String,
    },
    Image,
    #[default]
    Other,
}

impl ShapeKind {
    /// The word `ui.inspect` reports.
    pub fn word(&self) -> &'static str {
        match self {
            ShapeKind::Path { .. } => "path",
            ShapeKind::Component { .. } => "component",
            ShapeKind::Image => "image",
            ShapeKind::Other => "other",
        }
    }
}

/// One shape and its outline.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ShapeEntry {
    pub kind: ShapeKind,
    /// The flattened outline; no contours for an image or a shape whose outline fails.
    pub art: Arc<CellArt>,
}

/// The glyph the panel shows: the one it was opened on while the font has it, else the glyph being edited.
pub fn shown_glyph<'f>(font: &'f Font, view: &ViewState) -> Option<&'f Glyph> {
    view.shape_order.glyph.as_deref().and_then(|name| font.glyph(name)).map(|g| &**g).or_else(|| masters::shown_glyph(font, view))
}

pub fn project(model: &mut Tracked<ShapeOrderModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout.panel_state(&PanelId::ShapeOrder) == PanelState::Hidden {
        return;
    }
    let value = cx.session.font().ok().map(|font| shape_order(font, cx.view)).unwrap_or_default();
    super::set(model, value);
}

fn shape_order(font: &Font, view: &ViewState) -> ShapeOrderModel {
    let Some(glyph) = shown_glyph(font, view) else { return ShapeOrderModel::default() };
    let layers = layers(font, glyph);
    let more_layers = layers.len().saturating_sub(MAX_COLUMNS);
    let mut budget = MAX_POINTS;
    let columns = layers.into_iter().take(MAX_COLUMNS).map(|(layer, title)| column(font, layer, title, &mut budget)).collect();
    ShapeOrderModel { glyph: Some(glyph.name.clone()), columns, more_layers }
}

/// Whether a layer is an intermediate or alternate layer.
fn is_special(layer: &Layer) -> bool {
    !layer.info.attrs.coordinates.is_empty() || !layer.info.attrs.axis_rules.is_empty()
}

/// The layers that take part in interpolation, in order, with their titles: each master's layer, then that master's
/// special layers.
fn layers<'g>(font: &Font, glyph: &'g Glyph) -> Vec<(&'g Layer, String)> {
    let mut out = Vec::new();
    for m in font.masters.iter() {
        for layer in glyph.layers.iter().filter(|l| l.master == m.id && (l.is_master_layer() || is_special(l))) {
            let title = if layer.is_master_layer() {
                m.name.clone()
            } else {
                let label = brace_label(font, layer).into_iter().chain(bracket_label(font, layer)).collect::<Vec<_>>().join(" ");
                match (label.is_empty(), layer.info.name.as_deref()) {
                    (false, _) => format!("{} {label}", m.name),
                    (true, Some(n)) if !n.is_empty() => n.to_string(),
                    _ => layer.id.clone(),
                }
            };
            out.push((layer, title));
        }
    }
    out
}

fn column(font: &Font, layer: &Layer, title: String, budget: &mut usize) -> ShapeColumn {
    let shapes: Vec<ShapeEntry> = layer.shapes.iter().take(MAX_SHAPES).map(|s| entry(font, layer, s, budget)).collect();
    let mut bounds: Option<[f32; 4]> = None;
    for [x, y] in shapes.iter().flat_map(|s| s.art.contours.iter().flatten()).copied().filter(|[x, y]| x.is_finite() && y.is_finite()) {
        bounds = Some(match bounds {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }
    ShapeColumn { layer: layer.id.clone(), title, shapes, total: layer.shapes.len(), bounds }
}

/// One shape; its outline is flattened while `budget` lasts, and what it takes comes off `budget`.
fn entry(font: &Font, layer: &Layer, shape: &Shape, budget: &mut usize) -> ShapeEntry {
    let kind = match shape {
        Shape::Path(p) => ShapeKind::Path { nodes: p.nodes.len(), closed: p.closed },
        Shape::Component(c) => ShapeKind::Component { glyph: c.glyph.clone() },
        Shape::Image(_) => ShapeKind::Image,
        _ => ShapeKind::Other,
    };
    if *budget == 0 {
        return ShapeEntry { kind, art: Arc::default() };
    }
    let outline = match shape {
        Shape::Path(p) => p.to_bez().ok(),
        Shape::Component(c) => font.component_outline(c, &layer.master).ok(),
        _ => None,
    };
    let contours = outline.map(|p| flatten(&p)).unwrap_or_default();
    *budget = budget.saturating_sub(contours.iter().map(Vec::len).sum());
    ShapeEntry { kind, art: Arc::new(CellArt { width: 0.0, contours, color: None }) }
}

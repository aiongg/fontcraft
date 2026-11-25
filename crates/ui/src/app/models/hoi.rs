//! The Info box's interpolation curve part (M9.32, rows `infoBox.higherOrderInterpolation` and `hoi.timingGraph`): for
//! the one selected node, the curve it follows along one axis span, its knobs in glyph coordinates, and the timing graph:
//! the share of the curve's length the node has travelled at each point of its span
//! (`fontcraft_interp::hoi::timing_graph`). A linear node moves at constant speed and gives the diagonal.
//!
//! The span is the node's layer and the next layer up along the axis. The axis is the first of the node's curves with a
//! layer above it, else the first font axis with one. While a knob is picked with the Higher-Order Interpolation tool
//! ([`knob_box`]), it is that knob's span along the tool's axis.
//!
//! [`marks`] resolves what that tool shows and edits: the nodes of the edited layer and of its two neighbours along the
//! axis, and each node's path to them.

use std::collections::BTreeSet;

use craft::ui::contract::PlainData;
use fontcraft_engine::interp::hoi::{Span, auto_center, handles, neighbour_layer, node_span, position, rotation_center, timing_graph};
use fontcraft_engine::interp::{Designspace, GlyphSources};
use fontcraft_engine::model::{Font, Glyph, HoiCurve, HoiDirection, Layer, Path, Shape};
use fontcraft_geom::Point;
use fontcraft_tools::hoi::{HoiMarks, Kind, Knob, MAX_NODES, Span as Mark, Toward};

use crate::app::ViewState;

/// Straight pieces a drawn curve is made of.
const CURVE_SAMPLES: usize = 24;

/// Points the graph is sampled at.
pub const GRAPH_SAMPLES: usize = 64;

/// The timing graph of one node's curve along one axis.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TimingGraph {
    /// The axis tag of the span.
    pub axis: String,
    /// `(position along the span, share of the length travelled)`, both from 0 to 1.
    pub points: Vec<[f64; 2]>,
}

/// The interpolation curve part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct HoiBox {
    pub glyph: String,
    /// The layer whose node keeps the curve: the lower layer of the span.
    pub layer: String,
    /// The node, as `[shape, node]` on that layer.
    pub shape: usize,
    pub node: usize,
    /// The axis tag of the span.
    pub axis: String,
    /// `linear`, `intermediatePoint`, `handles` or `rotation`: the `type` of `hoi.setInterpolationType`.
    pub kind: String,
    /// The knobs in glyph coordinates: the intermediate point; the lower handle, then the upper one; the rotation's
    /// centre. Empty for a linear node, or when a knob is not finite.
    pub knobs: Vec<[f64; 2]>,
    /// The rotation's direction: `automatic`, `clockwise` or `counterclockwise`.
    pub direction: String,
    pub graph: TimingGraph,
}

/// The part for the node `points` selects alone on layer `layer` of `glyph`; `None` without one selected node, in a
/// font without an axis or two masters, or when no axis has a layer above the node's layer with the same node.
pub fn hoi_of(font: &Font, glyph: &str, layer: &str, points: &BTreeSet<(usize, usize)>) -> Option<HoiBox> {
    let mut selected = points.iter();
    let (&(shape, node), None) = (selected.next()?, selected.next()) else { return None };
    if font.axes.is_empty() || font.masters.len() < 2 {
        return None;
    }
    let g = font.glyph(glyph)?;
    let lower = path_of(g.layers.iter().find(|l| l.id == layer)?.shapes.get(shape)?)?;
    let entries = &lower.nodes.get(node)?.info.hoi.0;
    let ds = Designspace::new(font).ok()?;
    let sources = GlyphSources::of(font, &ds, glyph).ok()?;
    let curved = entries.iter().map(|e| (e.axis.clone(), Some(e.curve.clone())));
    let linear = ds.axes().iter().filter(|a| entries.iter().all(|e| e.axis != a.tag)).map(|a| (a.tag.clone(), None));
    curved.chain(linear).find_map(|(axis, curve)| {
        let upper_id = neighbour_layer(&ds, &sources, layer, &axis, true).ok()??;
        let upper = path_of(g.layers.iter().find(|l| l.id == upper_id)?.shapes.get(shape)?)?;
        let span = node_span(lower, upper, node)?;
        Some(span_box(glyph, layer, (shape, node), &axis, curve.as_ref(), &span))
    })
}

/// The part for the span the picked `knobs` belong to, along the tool's axis; `None` when they belong to several.
pub fn knob_box(font: &Font, view: &ViewState, glyph: &str, layer: &str, knobs: &[Knob]) -> Option<HoiBox> {
    let (first, rest) = knobs.split_first()?;
    let (shape, node, toward) = first.key();
    if rest.iter().any(|k| k.key() != first.key()) {
        return None;
    }
    let axis = crate::canvas::hoi::axis(font, view)?;
    let g = font.glyph(glyph)?;
    let here = g.layers.iter().find(|l| l.id == layer)?;
    let there = neighbour(font, g, here, axis, toward)?;
    let (lower, upper) = match toward {
        Toward::Up => (here, there),
        Toward::Down => (there, here),
    };
    let (a, b) = (path_of(lower.shapes.get(shape)?)?, path_of(upper.shapes.get(shape)?)?);
    let span = node_span(a, b, node)?;
    Some(span_box(glyph, &lower.id, (shape, node), axis, curve_of(a, node, axis), &span))
}

fn span_box(glyph: &str, lower: &str, (shape, node): (usize, usize), axis: &str, curve: Option<&HoiCurve>, span: &Span) -> HoiBox {
    let mut b = HoiBox { glyph: glyph.to_string(), layer: lower.to_string(), shape, node, axis: axis.to_string(), ..describe(curve, span) };
    b.graph.axis = axis.to_string();
    b
}

/// The layer next to `layer` along `axis`.
fn neighbour<'g>(font: &Font, glyph: &'g Glyph, layer: &Layer, axis: &str, toward: Toward) -> Option<&'g Layer> {
    let ds = Designspace::new(font).ok()?;
    let sources = GlyphSources::of(font, &ds, &glyph.name).ok()?;
    let id = neighbour_layer(&ds, &sources, &layer.id, axis, toward == Toward::Up).ok()??;
    glyph.layers.iter().find(|l| l.id == id)
}

fn curve_of<'p>(lower: &'p Path, node: usize, axis: &str) -> Option<&'p HoiCurve> {
    lower.nodes.get(node)?.info.hoi.0.iter().find(|e| e.axis == axis).map(|e| &e.curve)
}

/// What the Higher-Order Interpolation tool shows and edits on the edited glyph's active layer; `None` while another
/// tool is current, while the background is edited, and in a font without an axis or two masters. At most
/// [`MAX_NODES`] nodes of a layer are taken, in shape order.
pub fn marks(font: &Font, view: &ViewState) -> Option<HoiMarks> {
    if !crate::canvas::hoi::active(view) || view.edit_background || font.axes.is_empty() || font.masters.len() < 2 {
        return None;
    }
    let axis = crate::canvas::hoi::axis(font, view)?;
    let glyph = font.glyph(crate::canvas::tools::edited_glyph(font, view)?)?;
    let layer = crate::canvas::tools::active_layer(font, view)?;
    let below = neighbour(font, glyph, layer, axis, Toward::Down);
    let above = neighbour(font, glyph, layer, axis, Toward::Up);
    let mut spans = Vec::new();
    if let Some(below) = below {
        node_paths(below, layer, axis, Toward::Down, &mut spans);
    }
    if let Some(above) = above {
        node_paths(layer, above, axis, Toward::Up, &mut spans);
    }
    let nodes = |layer: Option<&Layer>| layer.map(node_points).unwrap_or_default();
    Some(HoiMarks { axis: axis.to_string(), here: node_points(layer), below: nodes(below), above: nodes(above), spans })
}

/// The positions of `layer`'s path nodes.
fn node_points(layer: &Layer) -> Vec<Point> {
    layer.shapes.iter().filter_map(path_of).flat_map(|p| p.nodes.iter()).take(MAX_NODES).map(|n| Point::new(n.x, n.y)).collect()
}

/// The path of each node from `lower` to `upper`, where the two layers have the same shape with as many nodes.
fn node_paths(lower: &Layer, upper: &Layer, axis: &str, toward: Toward, out: &mut Vec<Mark>) {
    let mut left = MAX_NODES;
    for (shape, (a, b)) in lower.shapes.iter().zip(&upper.shapes).enumerate() {
        let (Some(a), Some(b)) = (path_of(a), path_of(b)) else { continue };
        if a.nodes.len() != b.nodes.len() {
            continue;
        }
        // One centre for the whole path: `node_span` would find it again for each node.
        let Some(auto_center) = auto_center(a, b) else { continue };
        for (node, (n, m)) in a.nodes.iter().zip(&b.nodes).enumerate() {
            if left == 0 {
                return;
            }
            left -= 1;
            let span = Span { lower: (n.x, n.y), upper: (m.x, m.y), auto_center };
            let curve = curve_of(a, node, axis);
            let b = describe_knobs(curve, &span);
            let knobs = if curve.is_none() { vec![span.linear(0.5)] } else { b.knobs };
            let curve: Vec<Point> = match curve {
                None => Vec::new(),
                Some(c) => (0..=CURVE_SAMPLES).map(|k| point(position(c, &span, k as f64 / CURVE_SAMPLES as f64))).collect(),
            };
            out.push(Mark {
                shape,
                node,
                toward,
                lower: point(span.lower),
                upper: point(span.upper),
                kind: b.kind,
                knobs: knobs.into_iter().map(point).collect(),
                curve,
            });
        }
    }
}

fn point((x, y): (f64, f64)) -> Point {
    Point::new(x, y)
}

/// A curve's type, its knobs in glyph coordinates (none when one is not finite) and a rotation's direction.
struct Knobs {
    kind: Kind,
    knobs: Vec<(f64, f64)>,
    direction: Option<HoiDirection>,
}

fn describe_knobs(curve: Option<&HoiCurve>, span: &Span) -> Knobs {
    let (kind, knobs, direction) = match curve {
        None => (Kind::Linear, Vec::new(), None),
        Some(HoiCurve::Point { x, y }) => (Kind::IntermediatePoint, vec![(*x, *y)], None),
        Some(HoiCurve::Timing(t)) => {
            let (h1, h2) = handles(span, t);
            (Kind::Handles, vec![h1, h2], None)
        }
        Some(HoiCurve::Rotation { center, direction }) => (Kind::Rotation, vec![rotation_center(*center, span)], *direction),
    };
    let knobs = if knobs.iter().all(|(x, y)| x.is_finite() && y.is_finite()) { knobs } else { Vec::new() };
    Knobs { kind, knobs, direction }
}

fn path_of(shape: &Shape) -> Option<&Path> {
    match shape {
        Shape::Path(p) => Some(p),
        _ => None,
    }
}

/// The type, knobs, direction and graph of `curve` (`None`: linear) along `span`.
fn describe(curve: Option<&HoiCurve>, span: &Span) -> HoiBox {
    let Knobs { kind, knobs, direction } = describe_knobs(curve, span);
    let direction = match direction {
        None => "automatic",
        Some(HoiDirection::Clockwise) => "clockwise",
        Some(HoiDirection::CounterClockwise) => "counterclockwise",
    };
    let points = match curve {
        Some(c) => timing_graph(c, span, GRAPH_SAMPLES).into_iter().map(|(s, t)| [s, t]).collect(),
        None => (0..GRAPH_SAMPLES).map(|i| i as f64 / (GRAPH_SAMPLES - 1) as f64).map(|s| [s, s]).collect(),
    };
    HoiBox {
        kind: kind.as_str().to_string(),
        knobs: knobs.into_iter().map(|(x, y)| [x, y]).collect(),
        direction: direction.to_string(),
        graph: TimingGraph { axis: String::new(), points },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use super::hoi_of;

    /// Masters at weight 400 and 700; glyph `A` a rectangle that moves and widens from the first to the second.
    fn two_masters() -> (Session, String) {
        let mut s = Session::with_document();
        let mut run = |id: &str, params: Value| {
            s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        };
        run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
        run("font.master.add", json!({}));
        run("font.master.setAxisLocation", json!({"masters": [0], "location": {"wght": 400}}));
        run("font.master.setAxisLocation", json!({"masters": [1], "location": {"wght": 700}}));
        let ids: Vec<String> = s.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
        for (layer, x, width) in [(&ids[0], 0, 100), (&ids[1], 40, 300)] {
            s.execute("path.addRectangle", &json!({"glyph": "A", "layer": layer, "x": x, "y": 0, "width": width, "height": 500})).unwrap();
        }
        (s, ids[0].clone())
    }

    /// A node with no curve is linear on the font's axis and draws the diagonal; after the type command it shows the
    /// curve's two handles.
    #[test]
    fn a_linear_node_draws_the_diagonal_and_a_curve_shows_its_knobs() {
        let (mut s, light) = two_masters();
        let one: BTreeSet<(usize, usize)> = [(0, 1)].into_iter().collect();
        let b = hoi_of(s.font().unwrap(), "A", &light, &one).expect("the part for one node");
        assert_eq!((b.kind.as_str(), b.axis.as_str(), b.knobs.len()), ("linear", "wght", 0));
        assert!(b.graph.points.iter().all(|[x, y]| (x - y).abs() < 1e-9), "the diagonal: {:?}", b.graph.points);

        let params = json!({"glyph": "A", "layer": light, "points": [[0, 1]], "axis": "wght", "type": "handles"});
        s.execute("hoi.setInterpolationType", &params).unwrap();
        let b = hoi_of(s.font().unwrap(), "A", &light, &one).expect("the part for the curve");
        assert_eq!((b.kind.as_str(), b.knobs.len()), ("handles", 2));

        let two: BTreeSet<(usize, usize)> = [(0, 1), (0, 2)].into_iter().collect();
        assert!(hoi_of(s.font().unwrap(), "A", &light, &two).is_none(), "two nodes show no part");
    }
}

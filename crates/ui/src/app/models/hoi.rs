//! The Info box's interpolation curve part (M9.32, rows `infoBox.higherOrderInterpolation` and `hoi.timingGraph`): for
//! the one selected node, the curve it follows along one axis span, its knobs in glyph coordinates, and the timing graph:
//! the share of the curve's length the node has travelled at each point of its span
//! (`fontcraft_interp::hoi::timing_graph`). A linear node moves at constant speed and gives the diagonal.
//!
//! The span is the node's layer and the next layer up along the axis. The axis is the first of the node's curves with a
//! layer above it, else the first font axis with one.

use std::collections::BTreeSet;

use craft::ui::contract::PlainData;
use fontcraft_engine::interp::hoi::{Span, handles, neighbour_layer, node_span, rotation_center, timing_graph};
use fontcraft_engine::interp::{Designspace, GlyphSources};
use fontcraft_engine::model::{Font, HoiCurve, HoiDirection, Path, Shape};

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
        let mut b = HoiBox { glyph: glyph.to_string(), layer: layer.to_string(), shape, node, ..describe(curve.as_ref(), &span) };
        b.graph.axis.clone_from(&axis);
        b.axis = axis;
        Some(b)
    })
}

fn path_of(shape: &Shape) -> Option<&Path> {
    match shape {
        Shape::Path(p) => Some(p),
        _ => None,
    }
}

/// The type, knobs, direction and graph of `curve` (`None`: linear) along `span`.
fn describe(curve: Option<&HoiCurve>, span: &Span) -> HoiBox {
    let at = |(x, y): (f64, f64)| [x, y];
    let (kind, knobs, direction) = match curve {
        None => ("linear", Vec::new(), None),
        Some(HoiCurve::Point { x, y }) => ("intermediatePoint", vec![[*x, *y]], None),
        Some(HoiCurve::Timing(t)) => {
            let (h1, h2) = handles(span, t);
            ("handles", vec![at(h1), at(h2)], None)
        }
        Some(HoiCurve::Rotation { center, direction }) => ("rotation", vec![at(rotation_center(*center, span))], *direction),
    };
    let knobs = if knobs.iter().flatten().all(|v| v.is_finite()) { knobs } else { Vec::new() };
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
        kind: kind.to_string(),
        knobs,
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

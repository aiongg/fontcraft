//! The timing graph of the Info box (M9.32, row `hoi.timingGraph`): for the one selected node, when it holds a
//! higher-order interpolation curve, the share of the curve's length the node has travelled at each point of its span
//! (`fontcraft_interp::hoi::timing_graph`). A node moving at constant speed along a straight line gives the diagonal.

use std::collections::BTreeSet;

use craft::ui::contract::PlainData;
use fontcraft_engine::interp::hoi::{neighbour_layer, node_span, timing_graph};
use fontcraft_engine::interp::{Designspace, GlyphSources};
use fontcraft_engine::model::{Font, Shape};

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

/// The graph of the node `points` selects alone on layer `layer` of `glyph`; `None` without one selected node, or when
/// its node holds no curve with a layer above it on the curve's axis.
pub fn timing_of(font: &Font, glyph: &str, layer: &str, points: &BTreeSet<(usize, usize)>) -> Option<TimingGraph> {
    let mut selected = points.iter();
    let (&(shape, node), None) = (selected.next()?, selected.next()) else { return None };
    let g = font.glyph(glyph)?;
    let lower = match g.layers.iter().find(|l| l.id == layer)?.shapes.get(shape)? {
        Shape::Path(p) => p,
        _ => return None,
    };
    let entries = &lower.nodes.get(node)?.info.hoi.0;
    if entries.is_empty() {
        return None;
    }
    let ds = Designspace::new(font).ok()?;
    let sources = GlyphSources::of(font, &ds, glyph).ok()?;
    entries.iter().find_map(|entry| {
        let upper_id = neighbour_layer(&ds, &sources, layer, &entry.axis, true).ok()??;
        let upper = match g.layers.iter().find(|l| l.id == upper_id)?.shapes.get(shape)? {
            Shape::Path(p) => p,
            _ => return None,
        };
        let span = node_span(lower, upper, node)?;
        let points = timing_graph(&entry.curve, &span, GRAPH_SAMPLES).into_iter().map(|(s, t)| [s, t]).collect();
        Some(TimingGraph { axis: entry.axis.clone(), points })
    })
}

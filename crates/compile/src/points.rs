//! The point map: from each source node to its exported TrueType point.
//!
//! On-curve nodes keep their identity through the cubic-to-quadratic conversion; an off-curve
//! node maps to the nearest off-curve point of its segment; a node removed by overlap removal
//! maps to the nearest point the removal creates (the handbook's rule); components resolve
//! through their base glyph's map (`docs/plans/M10.md` § 3 › Point numbers). Rows:
//! `hinting.tt.cubicSourceQuadraticOutput`, `hinting.tt.export.overlapRemovalMovesHints`,
//! `hint.tt.getPointIndexes`. M10.19 fills the bodies; the outline stage (`outline.rs`, M10.19's)
//! keeps what it needs.

use fontcraft_hinting::{NodeRef, PointMap};
use fontcraft_model::Layer;

use crate::outline::Prepared;

/// One glyph's exported point numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphPointMap {
    /// `paths[path][node]`: path index as [`NodeRef::path`] counts paths.
    pub paths: Vec<Vec<Option<u16>>>,
    /// Points created by overlap removal at intersections: (segment ends, point).
    pub intersections: Vec<(NodeRef, NodeRef, u16)>,
    pub count: u16,
}

impl PointMap for GlyphPointMap {
    fn point(&self, node: NodeRef) -> Option<u16> {
        self.paths.get(node.path)?.get(node.node).copied().flatten()
    }

    fn point_count(&self) -> u16 {
        self.count
    }

    fn intersection(&self, a: NodeRef, b: NodeRef) -> Option<u16> {
        self.intersections.iter().find(|(x, y, _)| (*x, *y) == (a, b) || (*x, *y) == (b, a)).map(|(_, _, p)| *p)
    }
}

/// The map from `layer`'s nodes to the points of its prepared outline, or `None` when it cannot
/// be built (placeholder: identity for an all-quadratic outline left unchanged, `None` otherwise).
pub fn point_map(layer: &Layer, prepared: &Prepared) -> Option<GlyphPointMap> {
    // M10.19: the map through conversion, reversal and overlap removal.
    let _ = (layer, prepared);
    None
}

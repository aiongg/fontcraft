//! From source nodes to exported TrueType point numbers.
//!
//! The compiler's outline stage builds one map per glyph (`compile/src/points.rs`): on-curve
//! nodes keep their identity through the cubic-to-quadratic conversion, an off-curve node maps to
//! the nearest off-curve point of its segment, a node removed by overlap removal maps to the
//! nearest point the removal creates, components resolve through their base glyph's map
//! (`docs/plans/M10.md` § 3 › Point numbers). The glyph compiler ([`crate::glyph_program`]) reads
//! it through this trait, so tests can hand it a [`VecPointMap`].

use crate::{NodeRef, TtPoint};

/// Exported point numbers of a glyph's nodes.
pub trait PointMap {
    /// The point of `node`, or `None` when the node has no exported point.
    fn point(&self, node: NodeRef) -> Option<u16>;

    /// Outline points of the glyph, phantom points not counted.
    fn point_count(&self) -> u16;

    /// The point at the intersection of the segments ending at `a` and `b`, when overlap
    /// removal created one.
    fn intersection(&self, a: NodeRef, b: NodeRef) -> Option<u16> {
        let _ = (a, b);
        None
    }

    /// The point of any [`TtPoint`]: phantom points follow the outline points (left
    /// sidebearing first).
    fn resolve(&self, p: &TtPoint) -> Option<u16> {
        match p {
            TtPoint::Node(r) => self.point(*r),
            TtPoint::Intersection { a, b } => self.intersection(*a, *b),
            TtPoint::Lsb => Some(self.point_count()),
            TtPoint::Rsb => self.point_count().checked_add(1),
        }
    }
}

/// A point map held as tables: `paths[path][node]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VecPointMap {
    pub paths: Vec<Vec<Option<u16>>>,
    pub count: u16,
}

impl PointMap for VecPointMap {
    fn point(&self, node: NodeRef) -> Option<u16> {
        self.paths.get(node.path)?.get(node.node).copied().flatten()
    }

    fn point_count(&self) -> u16 {
        self.count
    }
}

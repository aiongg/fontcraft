//! What lies under the pointer.
//!
//! Points (nodes, handles, anchors) win over segments, segments over components, components over
//! guides. Among points the nearest wins; at equal distance a node beats an anchor and an anchor
//! beats a handle. The tolerance is in screen points, so it feels the same at every zoom.

use fontcraft_geom::Point;
use serde::Serialize;

use crate::geometry::{dist, finite, line_distance, near_box};
use crate::scene::Scene;
use crate::segment::segments;
use crate::selection::{GuideRef, GuideScope, Item, Selection};
use crate::view::View;

/// Distances closer than this count as equal when ranking points.
const TIE: f64 = 1e-9;

/// The object under the pointer.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Hit {
    /// An on-curve node.
    Node {
        shape: usize,
        node: usize,
    },
    /// An off-curve node.
    Handle {
        shape: usize,
        node: usize,
    },
    /// A segment, by the on-curve node that ends it; `t` runs from 0 at its start to 1 at its end.
    Segment {
        shape: usize,
        node: usize,
        start: usize,
        t: f64,
        line: bool,
    },
    Anchor {
        name: String,
    },
    Component {
        shape: usize,
    },
    Guide {
        guide: GuideRef,
    },
}

/// A hit, where it is, and whether it may be moved.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HitResult {
    pub hit: Hit,
    /// The object's position: the node or anchor itself, the nearest point of a segment or guide,
    /// the pointer for a component.
    pub at: Point,
    /// The glyph, the path, the node or the object is locked: it can be selected, not moved.
    pub locked: bool,
}

impl HitResult {
    /// The items a click on this hit selects: a segment selects its two end nodes.
    pub fn items(&self) -> Vec<Item> {
        match &self.hit {
            Hit::Node { shape, node } | Hit::Handle { shape, node } => vec![Item::Point(*shape, *node)],
            Hit::Segment { shape, node, start, .. } => vec![Item::Point(*shape, *start), Item::Point(*shape, *node)],
            Hit::Anchor { name } => vec![Item::Anchor(name.clone())],
            Hit::Component { shape } => vec![Item::Shape(*shape)],
            Hit::Guide { guide } => vec![Item::Guide(*guide)],
        }
    }

    /// Whether every item of the hit is in `selection`.
    pub fn selected_in(&self, selection: &Selection) -> bool {
        self.items().iter().all(|i| selection.contains(i))
    }
}

/// The object under `pos` (font units) within `tolerance` screen points, or `None`.
pub fn hit_test(scene: &Scene, view: &View, pos: Point, tolerance: f64) -> Option<HitResult> {
    if !finite(pos) {
        return None;
    }
    let tol = view.units(tolerance);
    let mut best: Option<(f64, u8, HitResult)> = None;
    let mut consider = |d: f64, rank: u8, hit: HitResult| {
        if d.is_nan() || d > tol {
            return;
        }
        let better = match &best {
            None => true,
            Some((bd, br, _)) => d < bd - TIE || (d <= bd + TIE && rank < *br),
        };
        if better {
            best = Some((d, rank, hit));
        }
    };
    for (shape, path) in scene.paths() {
        let path_locked = scene.locked || path.info.locked;
        for (node, n) in path.nodes.iter().enumerate() {
            let p = n.point();
            if !finite(p) {
                continue;
            }
            let locked = path_locked || n.locked;
            if n.on_curve() {
                consider(dist(p, pos), 0, HitResult { hit: Hit::Node { shape, node }, at: p, locked });
            } else {
                consider(dist(p, pos), 2, HitResult { hit: Hit::Handle { shape, node }, at: p, locked });
            }
        }
    }
    for a in &scene.layer.anchors {
        let p = Point::new(a.x, a.y);
        if finite(p) {
            let locked = scene.locked || a.info.locked;
            consider(dist(p, pos), 1, HitResult { hit: Hit::Anchor { name: a.name.clone() }, at: p, locked });
        }
    }
    if let Some((_, _, hit)) = best {
        return Some(hit);
    }

    let mut nearest: Option<(f64, HitResult)> = None;
    for (shape, path) in scene.paths() {
        let locked = scene.locked || path.info.locked;
        for seg in segments(shape, path) {
            if !near_box(pos, seg.bounds, tol) {
                continue;
            }
            let Some((d, t, at)) = seg.nearest(pos) else { continue };
            if d <= tol && nearest.as_ref().is_none_or(|(bd, _)| d < *bd) {
                let hit = Hit::Segment { shape, node: seg.end, start: seg.start, t, line: seg.is_line() };
                nearest = Some((d, HitResult { hit, at, locked }));
            }
        }
    }
    if let Some((_, hit)) = nearest {
        return Some(hit);
    }

    // The frontmost component is the one drawn last.
    for c in scene.components().iter().rev() {
        if scene.shape_editable(c.shape) && c.contains(pos) {
            let locked = scene.locked || c.locked;
            return Some(HitResult { hit: Hit::Component { shape: c.shape }, at: pos, locked });
        }
    }

    let local = scene.local_guides().iter().enumerate().map(|(i, g)| (GuideScope::Local, i, g));
    let global = scene.shown_global_guides().map(|(i, g)| (GuideScope::Global, i, g));
    let mut guide: Option<(f64, HitResult)> = None;
    for (scope, index, g) in local.chain(global) {
        let Some((d, at)) = line_distance(pos, Point::new(g.x, g.y), g.angle) else { continue };
        if d <= tol && guide.as_ref().is_none_or(|(bd, _)| d < *bd) {
            let hit = Hit::Guide { guide: GuideRef { scope, index } };
            guide = Some((d, HitResult { hit, at, locked: g.locked }));
        }
    }
    guide.map(|(_, h)| h)
}

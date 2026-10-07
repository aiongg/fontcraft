//! Glyphs, their layers, and what a layer holds.

use fontcraft_geom::{Affine, BezPath, Point};
use serde::{Deserialize, Serialize};

use crate::{Extra, MAX_COORDINATE, in_range};

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub name: String,
    /// Code points this glyph is mapped from (none for unencoded glyphs), in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unicodes: Vec<u32>,
    /// Is the glyph written to compiled fonts?
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub export: bool,
    /// Master layers and all others, in file order.
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// One drawing of a glyph. A *master layer* has the id of its master; every other layer (backup,
/// intermediate, alternate, colour) has an id of its own and names its master in `master`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    /// The master this layer belongs to (its own id, for a master layer).
    pub master: String,
    pub width: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl Layer {
    /// A master layer for `master`.
    pub fn new(master: impl Into<String>, width: f64) -> Self {
        let master = master.into();
        Self { id: master.clone(), master, width, shapes: Vec::new(), anchors: Vec::new(), extra: Extra::new() }
    }

    pub fn is_master_layer(&self) -> bool {
        self.id == self.master
    }
}

impl Glyph {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), unicodes: Vec::new(), export: true, layers: Vec::new(), extra: Extra::new() }
    }

    /// The master layer of `master`.
    pub fn layer(&self, master: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == master && l.master == master)
    }

    pub fn layer_mut(&mut self, master: &str) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == master && l.master == master)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shape {
    Path(Path),
    Component(Component),
}

/// A contour. Nodes are in path direction; off-curve nodes come before the on-curve node that ends
/// their segment (two before a [`NodeKind::Curve`], one or more before a [`NodeKind::QCurve`]). In
/// a closed path the segment that ends at the first node wraps around from the last nodes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Path {
    #[serde(default = "yes")]
    pub closed: bool,
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub x: f64,
    pub y: f64,
    #[serde(rename = "type")]
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub smooth: bool,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// On-curve, ends a straight segment.
    Line,
    /// On-curve, ends a cubic segment (two off-curve nodes before it).
    Curve,
    /// On-curve, ends a quadratic segment (off-curve nodes before it).
    QCurve,
    /// A control point.
    OffCurve,
}

/// How a component is placed. Sources write it in one of two forms and both are kept as written,
/// because a general matrix has no unique decomposition (`roundtrip.componentTransformKept`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transform {
    /// Position, scale, rotation in degrees counter-clockwise, and slant in degrees, applied as
    /// translate · slant · rotate · scale (the order fontc and glyphsLib use).
    Decomposed { pos: (f64, f64), scale: (f64, f64), angle: f64, slant: (f64, f64) },
    /// The six numbers `[xx, yx, xy, yy, dx, dy]`.
    Matrix([f64; 6]),
}

impl Default for Transform {
    fn default() -> Self {
        Transform::Decomposed { pos: (0.0, 0.0), scale: (1.0, 1.0), angle: 0.0, slant: (0.0, 0.0) }
    }
}

impl Transform {
    pub fn translate(dx: f64, dy: f64) -> Self {
        Transform::Decomposed { pos: (dx, dy), scale: (1.0, 1.0), angle: 0.0, slant: (0.0, 0.0) }
    }

    pub fn affine(&self) -> Affine {
        match *self {
            Transform::Matrix(m) => Affine::new(m),
            Transform::Decomposed { pos, scale, angle, slant } => {
                let slant = Affine::new([1.0, slant.1.to_radians().tan(), slant.0.to_radians().tan(), 1.0, 0.0, 0.0]);
                Affine::translate((pos.0, pos.1)) * slant * Affine::rotate(angle.to_radians()) * Affine::scale_non_uniform(scale.0, scale.1)
            }
        }
    }

    pub fn is_identity(&self) -> bool {
        match *self {
            Transform::Matrix(m) => m == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            Transform::Decomposed { pos, scale, angle, slant } => pos == (0.0, 0.0) && scale == (1.0, 1.0) && angle == 0.0 && slant == (0.0, 0.0),
        }
    }

    /// Every number in the transform, for range checks.
    pub fn numbers(&self) -> Vec<f64> {
        match *self {
            Transform::Matrix(m) => m.to_vec(),
            Transform::Decomposed { pos, scale, angle, slant } => vec![pos.0, pos.1, scale.0, scale.1, angle, slant.0, slant.1],
        }
    }
}

/// A reference to another glyph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub glyph: String,
    #[serde(default, skip_serializing_if = "Transform::is_identity")]
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl Component {
    /// A reference to `glyph` moved by `(dx, dy)`.
    pub fn new(glyph: impl Into<String>, dx: f64, dy: f64) -> Self {
        Self { glyph: glyph.into(), transform: Transform::translate(dx, dy), extra: Extra::new() }
    }

    pub fn affine(&self) -> Affine {
        self.transform.affine()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub name: String,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl Anchor {
    pub fn new(name: impl Into<String>, x: f64, y: f64) -> Self {
        Self { name: name.into(), x, y, extra: Extra::new() }
    }
}

impl Node {
    pub fn new(x: f64, y: f64, kind: NodeKind) -> Self {
        Self { x, y, kind, smooth: false, extra: Extra::new() }
    }
    pub fn point(&self) -> Point {
        Point::new(self.x, self.y)
    }
    pub fn on_curve(&self) -> bool {
        self.kind != NodeKind::OffCurve
    }
}

impl Path {
    pub fn new(closed: bool, nodes: Vec<Node>) -> Self {
        Self { closed, nodes, extra: Extra::new() }
    }

    /// A closed rectangle, counter-clockwise from the bottom left.
    pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        Self::new(true, vec![n(x0, y0), n(x1, y0), n(x1, y1), n(x0, y1)])
    }

    /// The contour as Bézier segments, or what is wrong with its node list.
    pub fn to_bez(&self) -> Result<BezPath, String> {
        let mut out = BezPath::new();
        if self.nodes.is_empty() {
            return Ok(out);
        }
        if let Some(n) = self.nodes.iter().find(|n| !(in_range(n.x) && in_range(n.y))) {
            return Err(format!("a node's coordinate ({}, {}) is not a number or is beyond ±{MAX_COORDINATE}", n.x, n.y));
        }
        // Start at the first on-curve node; a closed path may begin with off-curve nodes that
        // belong to its last segment.
        let Some(first) = self.nodes.iter().position(Node::on_curve) else {
            return self.all_off_curve();
        };
        if !self.closed && first != 0 {
            return Err("an open path must start with an on-curve node".into());
        }
        let count = self.nodes.len();
        let Some(start) = self.nodes.get(first) else { return Ok(out) };
        out.move_to(start.point());
        let mut pending: Vec<Point> = Vec::new();
        // Walk every node after the start once; a closed path comes back around to the start.
        let steps = if self.closed { count } else { count - first - 1 };
        for step in 1..=steps {
            let Some(node) = self.nodes.get((first + step) % count) else { continue };
            match node.kind {
                NodeKind::OffCurve => pending.push(node.point()),
                NodeKind::Line => {
                    if !pending.is_empty() {
                        return Err("a line node follows off-curve nodes".into());
                    }
                    // The closing line back to the start is implied by `close_path`.
                    if !(self.closed && step == steps) {
                        out.line_to(node.point());
                    }
                }
                NodeKind::Curve => {
                    let [c1, c2] = pending.as_slice() else {
                        return Err(format!("a curve node needs two off-curve nodes before it, found {}", pending.len()));
                    };
                    out.curve_to(*c1, *c2, node.point());
                    pending.clear();
                }
                NodeKind::QCurve => {
                    let Some((last, rest)) = pending.split_last() else {
                        return Err("a qcurve node needs at least one off-curve node before it".into());
                    };
                    // Consecutive off-curve nodes have an implied on-curve point halfway between.
                    let mut prev = None::<Point>;
                    for c in rest {
                        if let Some(p) = prev {
                            out.quad_to(p, p.midpoint(*c));
                        }
                        prev = Some(*c);
                    }
                    if let Some(p) = prev {
                        out.quad_to(p, p.midpoint(*last));
                    }
                    out.quad_to(*last, node.point());
                    pending.clear();
                }
            }
        }
        if !pending.is_empty() {
            return Err("off-curve nodes at the end of the path have no on-curve node".into());
        }
        if self.closed {
            out.close_path();
        }
        Ok(out)
    }

    /// A closed contour made only of off-curve nodes: a quadratic spline whose on-curve points are
    /// the midpoints between neighbours (legal in TrueType outlines and in UFO).
    fn all_off_curve(&self) -> Result<BezPath, String> {
        if !self.closed {
            return Err("an open path needs at least one on-curve node".into());
        }
        let points: Vec<Point> = self.nodes.iter().map(Node::point).collect();
        let (Some(first), Some(last)) = (points.first(), points.last()) else { return Ok(BezPath::new()) };
        if points.len() < 2 {
            return Err("a path of off-curve nodes needs at least two of them".into());
        }
        let mut out = BezPath::new();
        out.move_to(last.midpoint(*first));
        for (i, p) in points.iter().enumerate() {
            let next = points.get(i + 1).unwrap_or(first);
            out.quad_to(*p, p.midpoint(*next));
        }
        out.close_path();
        Ok(out)
    }
}

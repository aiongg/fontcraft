//! Glyphs, their layers, and what a layer holds.

use fontcraft_geom::{Affine, BezPath, Point};
use serde::{Deserialize, Serialize};

use crate::{
    AnchorInfoBox, ComponentInfoBox, Extra, Family, GlyphInfoBox, Image, KernGroups, LayerInfoBox, MAX_COORDINATE, NodeInfo, NodeInfoBox,
    Orientation, PathInfoBox, UserData, in_range,
};
use fontcraft_plist::Value;

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
    #[serde(default, skip_serializing_if = "KernGroups::is_empty")]
    pub kern_groups: KernGroups,
    /// Production name, classification, metrics keys, colour, tags, note and the rest of the
    /// glyph's metadata ([`crate::GlyphInfo`]); most glyphs have none.
    #[serde(default, skip_serializing_if = "GlyphInfoBox::is_default")]
    pub info: GlyphInfoBox,
    /// Master layers and all others, in file order.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// The file a multi-file source stored this glyph in, when it is not the one that format's
    /// naming rule gives (a package's `glyphs/*.glyph`, a UFO's `.glif`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
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
    /// Name, attributes, vertical metrics, guides, hints, annotations, image, background and the
    /// rest of the layer's data ([`crate::LayerInfo`]); most layers have none.
    #[serde(default, skip_serializing_if = "LayerInfoBox::is_default")]
    pub info: LayerInfoBox,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl Layer {
    /// A master layer for `master`.
    pub fn new(master: impl Into<String>, width: f64) -> Self {
        let master = master.into();
        Self { id: master.clone(), master, width, shapes: Vec::new(), anchors: Vec::new(), info: LayerInfoBox::default(), extra: Extra::new() }
    }

    pub fn is_master_layer(&self) -> bool {
        self.id == self.master
    }
}

impl Glyph {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            unicodes: Vec::new(),
            export: true,
            kern_groups: KernGroups::default(),
            info: GlyphInfoBox::default(),
            layers: Vec::new(),
            file_name: None,
            extra: Extra::new(),
        }
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
    /// An image placed among the shapes (`.glyphs` format version 4). Outlines and compiled fonts
    /// leave it out.
    Image(Box<Image>),
    /// A shape the model cannot represent yet (a shape group of format version 4, a path with
    /// quartic, Hobby or spiral nodes), kept exactly as read so that saving to the same format
    /// puts it back. Outlines and compiled fonts leave it out.
    Raw(RawShape),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RawShape {
    pub family: Family,
    pub value: Value,
}

/// In the native file a raw shape is `{"family", "value"}`, with the value as property-list text
/// in its family's syntax (see [`Extra`]'s native form).
#[derive(Serialize, Deserialize)]
struct RawShapeRepr {
    family: Family,
    value: String,
}

impl Serialize for RawShape {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RawShapeRepr { family: self.family, value: crate::extra::value_text(self.family, &self.value) }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for RawShape {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = RawShapeRepr::deserialize(d)?;
        let value =
            crate::extra::parse_value_text(r.family, &r.value).map_err(|e| <D::Error as serde::de::Error>::custom(format!("raw shape: {e}")))?;
        Ok(RawShape { family: r.family, value })
    }
}

/// A contour. Nodes are in path direction; off-curve nodes come before the on-curve node that ends
/// their segment (two before a [`NodeKind::Curve`], one or more before a [`NodeKind::QCurve`]). In
/// a closed path the segment that ends at the first node wraps around from the last nodes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Path {
    #[serde(default = "yes")]
    pub closed: bool,
    pub nodes: Vec<Node>,
    /// Lock, drawing attributes and identifier ([`crate::PathInfo`]); most paths have none.
    #[serde(default, skip_serializing_if = "PathInfoBox::is_default")]
    pub info: PathInfoBox,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// A point of a path: 32 bytes, its rare data ([`NodeInfo`]) behind one pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub x: f64,
    pub y: f64,
    pub kind: NodeKind,
    /// The segments on either side meet without a corner (an on-curve node).
    pub smooth: bool,
    /// The reference's tangent join, where only the curve side follows the straight side
    /// (`.glyphs` version 4 `t`). It means something only with `smooth`; other formats write
    /// such a node as smooth.
    pub tangent: bool,
    pub locked: bool,
    pub orientation: Orientation,
    /// Name, identifier, user data and unknown keys.
    pub info: NodeInfoBox,
}

/// A node in the native file: one flat object (`x`, `y`, `type`, then what is set of `smooth`,
/// `tangent`, `locked`, `orientation`, `name`, `identifier`, `user_data`, `extra`).
#[derive(Serialize)]
struct NodeOut<'a> {
    x: f64,
    y: f64,
    #[serde(rename = "type")]
    kind: NodeKind,
    #[serde(skip_serializing_if = "is_false")]
    smooth: bool,
    #[serde(skip_serializing_if = "is_false")]
    tangent: bool,
    #[serde(skip_serializing_if = "is_false")]
    locked: bool,
    #[serde(skip_serializing_if = "Orientation::is_left")]
    orientation: Orientation,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identifier: Option<&'a str>,
    #[serde(skip_serializing_if = "UserData::is_empty")]
    user_data: &'a UserData,
    #[serde(skip_serializing_if = "Extra::is_empty")]
    extra: &'a Extra,
}

#[derive(Deserialize)]
struct NodeIn {
    x: f64,
    y: f64,
    #[serde(rename = "type")]
    kind: NodeKind,
    #[serde(default)]
    smooth: bool,
    #[serde(default)]
    tangent: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    orientation: Orientation,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    identifier: Option<String>,
    #[serde(default)]
    user_data: UserData,
    #[serde(default)]
    extra: Extra,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let i = &*self.info;
        NodeOut {
            x: self.x,
            y: self.y,
            kind: self.kind,
            smooth: self.smooth,
            tangent: self.tangent,
            locked: self.locked,
            orientation: self.orientation,
            name: i.name.as_deref(),
            identifier: i.identifier.as_deref(),
            user_data: &i.user_data,
            extra: &i.extra,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let n = NodeIn::deserialize(d)?;
        let info = NodeInfo { name: n.name, identifier: n.identifier, user_data: n.user_data, extra: n.extra };
        Ok(Node {
            x: n.x,
            y: n.y,
            kind: n.kind,
            smooth: n.smooth,
            tangent: n.tangent,
            locked: n.locked,
            orientation: n.orientation,
            info: crate::Rare::new(info),
        })
    }
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

fn is_default_transform(t: &Transform) -> bool {
    *t == Transform::default()
}

/// A reference to another glyph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub glyph: String,
    /// Left out of the native file only in its default form, so an identity matrix read from a
    /// matrix-writing format stays a matrix.
    #[serde(default, skip_serializing_if = "is_default_transform")]
    pub transform: Transform,
    /// Alignment, attachment anchor, smart values, source master, lock, orientation, drawing
    /// attributes, identifier and user data ([`crate::ComponentInfo`]).
    #[serde(default, skip_serializing_if = "ComponentInfoBox::is_default")]
    pub info: ComponentInfoBox,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl Component {
    /// A reference to `glyph` moved by `(dx, dy)`.
    pub fn new(glyph: impl Into<String>, dx: f64, dy: f64) -> Self {
        Self { glyph: glyph.into(), transform: Transform::translate(dx, dy), info: ComponentInfoBox::default(), extra: Extra::new() }
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
    /// Lock, orientation, colour, identifier and user data ([`crate::AnchorInfo`]).
    #[serde(default, skip_serializing_if = "AnchorInfoBox::is_default")]
    pub info: AnchorInfoBox,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl Anchor {
    pub fn new(name: impl Into<String>, x: f64, y: f64) -> Self {
        Self { name: name.into(), x, y, info: AnchorInfoBox::default(), extra: Extra::new() }
    }
}

impl Node {
    pub fn new(x: f64, y: f64, kind: NodeKind) -> Self {
        Self { x, y, kind, smooth: false, tangent: false, locked: false, orientation: Orientation::Left, info: NodeInfoBox::default() }
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
        Self { closed, nodes, info: PathInfoBox::default(), extra: Extra::new() }
    }

    /// A closed rectangle, counter-clockwise from the bottom left.
    pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        Self::new(true, vec![n(x0, y0), n(x1, y0), n(x1, y1), n(x0, y1)])
    }

    /// The contour as Bézier segments, or what is wrong with its node list.
    pub fn to_bez(&self) -> Result<BezPath, String> {
        self.segments(false)
    }

    /// [`Path::to_bez`] in the form compilers read: a closed contour that starts with a line
    /// node also ends with an explicit line back to its start, before the close (fontTools'
    /// segment pens with `outputImpliedClosingLine`, which fontc follows). Point lists made
    /// from it then keep a closing point that sits on the start, as the reference compilers'
    /// do.
    pub fn to_contour(&self) -> Result<BezPath, String> {
        self.segments(true)
    }

    fn segments(&self, closing_line: bool) -> Result<BezPath, String> {
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
                    if closing_line || !(self.closed && step == steps) {
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

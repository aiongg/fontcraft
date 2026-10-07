//! FontCraft font source model (L1): pure data, no I/O, no UI.
//!
//! One [`Font`] holds font-wide information, its [`Master`]s and its [`Glyph`]s. A glyph has one
//! [`Layer`] per master; a layer holds [`Shape`]s (paths and components), anchors and its advance
//! width. Glyphs are shared with [`Arc`], so a snapshot of the whole font for undo costs one
//! pointer per glyph (see `fontcraft-engine`).
//!
//! Every struct keeps the keys it does not know in `extra`, so a file written by a newer version
//! (or a key carried over from another source format) survives a load and save.
//!
//! This is the M0 subset. `docs/spec/30-source-formats.md` § Recommended model lists what joins it:
//! axes, instances, kerning, features, hints, guides, custom parameters, colour layers.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use fontcraft_geom::{Affine, BezPath, Point};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Keys this version does not know, kept verbatim for lossless round trips.
pub type Extra = BTreeMap<String, Value>;

/// Smallest and largest units-per-em the OpenType `head` table allows.
pub const UPM_RANGE: std::ops::RangeInclusive<u16> = 16..=16384;
/// Most glyphs a font may hold (glyph ids are 16-bit in the tables we write).
pub const MAX_GLYPHS: usize = 65_535;
/// How deep components may nest before a glyph counts as cyclic.
pub const MAX_COMPONENT_DEPTH: usize = 32;
/// Largest coordinate or advance, in font units, the model accepts. Far beyond any real font
/// (OpenType stores 16-bit coordinates); it exists so curve work on hostile input stays bounded.
pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// Most path elements one resolved outline may have. Components can multiply an outline
/// exponentially (a glyph made of two copies of a glyph made of two copies…); this bounds the work.
pub const MAX_OUTLINE_ELEMENTS: usize = 500_000;
/// Longest family, master or glyph name, in bytes.
pub const MAX_NAME_LEN: usize = 255;
/// Most code points one glyph may be mapped from.
pub const MAX_UNICODES: usize = 1024;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("glyph name is empty")]
    EmptyGlyphName,
    #[error("glyph `{0}` already exists")]
    DuplicateGlyph(String),
    #[error("no glyph named `{0}`")]
    NoSuchGlyph(String),
    #[error("no master `{0}`")]
    NoSuchMaster(String),
    #[error("the font has no master")]
    NoMaster,
    #[error("units per em must be between 16 and 16384, got {0}")]
    BadUnitsPerEm(u32),
    #[error("glyph `{glyph}`: {message}")]
    BadOutline { glyph: String, message: String },
    #[error("the font would hold more than {MAX_GLYPHS} glyphs")]
    TooManyGlyphs,
    #[error("glyph `{0}`: components nest too deeply or refer to themselves")]
    ComponentCycle(String),
    #[error("glyph `{0}`: the outline with its components resolved has more than {MAX_OUTLINE_ELEMENTS} segments")]
    OutlineTooLarge(String),
    #[error("the {what} name {problem}")]
    BadName { what: &'static str, problem: String },
    #[error("two masters share the id `{0}`")]
    DuplicateMaster(String),
    #[error("glyph `{glyph}` has two layers for master `{master}`")]
    DuplicateLayer { glyph: String, master: String },
    #[error("glyph `{glyph}`: {value:#X} is not a Unicode code point")]
    BadUnicode { glyph: String, value: u32 },
    #[error("glyph `{glyph}` is mapped from more than {MAX_UNICODES} code points")]
    TooManyUnicodes { glyph: String },
}

/// A name is usable when it is not empty, not longer than [`MAX_NAME_LEN`] bytes and has no
/// control characters.
fn check_name(what: &'static str, name: &str) -> Result<(), ModelError> {
    let problem = if name.trim().is_empty() {
        "is empty"
    } else if name.len() > MAX_NAME_LEN {
        "is longer than 255 bytes"
    } else if name.chars().any(char::is_control) {
        "contains a control character"
    } else {
        return Ok(());
    };
    Err(ModelError::BadName { what, problem: problem.to_string() })
}

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

/// A font source: what a `.fontcraft` file stores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Font {
    pub family_name: String,
    pub units_per_em: u16,
    #[serde(default = "one")]
    pub version_major: u16,
    #[serde(default)]
    pub version_minor: u16,
    pub masters: Vec<Master>,
    /// In glyph order.
    pub glyphs: Vec<Arc<Glyph>>,
    #[serde(flatten)]
    pub extra: Extra,
}

fn one() -> u16 {
    1
}

/// One design of the family (a weight, a width…): vertical metrics now, axis locations later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Master {
    pub id: String,
    pub name: String,
    pub ascender: f64,
    pub cap_height: f64,
    pub x_height: f64,
    pub descender: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub name: String,
    /// Code points this glyph is mapped from (none for unencoded glyphs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unicodes: Vec<u32>,
    /// Is the glyph written to compiled fonts?
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub export: bool,
    /// One per master, found by [`Layer::master`].
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(flatten)]
    pub extra: Extra,
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// The id of the master this layer belongs to.
    pub master: String,
    pub width: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
    #[serde(flatten)]
    pub extra: Extra,
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
    #[serde(flatten)]
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
    #[serde(flatten)]
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

/// A reference to another glyph, placed with an affine transform `[xx, yx, xy, yy, dx, dy]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub glyph: String,
    #[serde(default = "identity", skip_serializing_if = "is_identity")]
    pub transform: [f64; 6],
    #[serde(flatten)]
    pub extra: Extra,
}

fn identity() -> [f64; 6] {
    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
}
fn is_identity(t: &[f64; 6]) -> bool {
    *t == identity()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub name: String,
    pub x: f64,
    pub y: f64,
    #[serde(flatten)]
    pub extra: Extra,
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
    /// A closed rectangle, counter-clockwise from the bottom left.
    pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        Self { closed: true, nodes: vec![n(x0, y0), n(x1, y0), n(x1, y1), n(x0, y1)], extra: Extra::new() }
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
}

impl Path {
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

impl Component {
    /// A reference to `glyph` moved by `(dx, dy)`.
    pub fn new(glyph: impl Into<String>, dx: f64, dy: f64) -> Self {
        Self { glyph: glyph.into(), transform: [1.0, 0.0, 0.0, 1.0, dx, dy], extra: Extra::new() }
    }
    pub fn affine(&self) -> Affine {
        Affine::new(self.transform)
    }
}

impl Glyph {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), unicodes: Vec::new(), export: true, layers: Vec::new(), extra: Extra::new() }
    }
    pub fn layer(&self, master: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.master == master)
    }
    pub fn layer_mut(&mut self, master: &str) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.master == master)
    }
}

impl Layer {
    pub fn new(master: impl Into<String>, width: f64) -> Self {
        Self { master: master.into(), width, shapes: Vec::new(), anchors: Vec::new(), extra: Extra::new() }
    }
}

impl Font {
    /// A new family with one Regular master and the starting glyph set: `.notdef`, `space` and
    /// the basic Latin letters (empty, encoded, ready to draw).
    pub fn new(family_name: impl Into<String>) -> Self {
        let master = Master {
            id: "m01".into(),
            name: "Regular".into(),
            ascender: 800.0,
            cap_height: 700.0,
            x_height: 500.0,
            descender: -200.0,
            extra: Extra::new(),
        };
        let mut glyphs = Vec::new();
        let mut notdef = Glyph::new(".notdef");
        let mut layer = Layer::new(&master.id, 500.0);
        layer.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 450.0, 700.0)));
        // The inner contour runs the other way, so it is a hole under the non-zero rule.
        let mut inner = Path::rect(100.0, 50.0, 400.0, 650.0);
        inner.nodes.reverse();
        layer.shapes.push(Shape::Path(inner));
        notdef.layers.push(layer);
        glyphs.push(Arc::new(notdef));
        let mut space = Glyph::new("space");
        space.unicodes.push(0x20);
        space.layers.push(Layer::new(&master.id, 250.0));
        glyphs.push(Arc::new(space));
        for c in ('A'..='Z').chain('a'..='z') {
            let mut g = Glyph::new(c.to_string());
            g.unicodes.push(c as u32);
            g.layers.push(Layer::new(&master.id, 600.0));
            glyphs.push(Arc::new(g));
        }
        Self {
            family_name: family_name.into(),
            units_per_em: 1000,
            version_major: 1,
            version_minor: 0,
            masters: vec![master],
            glyphs,
            extra: Extra::new(),
        }
    }

    pub fn default_master(&self) -> Result<&Master, ModelError> {
        self.masters.first().ok_or(ModelError::NoMaster)
    }

    pub fn master(&self, id: &str) -> Result<&Master, ModelError> {
        self.masters.iter().find(|m| m.id == id).ok_or_else(|| ModelError::NoSuchMaster(id.to_string()))
    }

    /// The glyph called `name`. A linear search today; the full model (M1) keeps a name index.
    pub fn glyph(&self, name: &str) -> Option<&Arc<Glyph>> {
        self.glyphs.iter().find(|g| g.name == name)
    }

    /// The glyph for editing. Only this glyph is copied if a snapshot still shares it.
    pub fn glyph_mut(&mut self, name: &str) -> Result<&mut Glyph, ModelError> {
        self.glyphs.iter_mut().find(|g| g.name == name).map(Arc::make_mut).ok_or_else(|| ModelError::NoSuchGlyph(name.to_string()))
    }

    /// Appends a glyph with an empty layer for every master.
    pub fn add_glyph(&mut self, name: &str, unicodes: Vec<u32>, width: f64) -> Result<(), ModelError> {
        if name.is_empty() {
            return Err(ModelError::EmptyGlyphName);
        }
        if self.glyph(name).is_some() {
            return Err(ModelError::DuplicateGlyph(name.to_string()));
        }
        if self.glyphs.len() >= MAX_GLYPHS {
            return Err(ModelError::TooManyGlyphs);
        }
        let mut g = Glyph::new(name);
        g.unicodes = unicodes;
        g.layers = self.masters.iter().map(|m| Layer::new(&m.id, width)).collect();
        self.glyphs.push(Arc::new(g));
        Ok(())
    }

    pub fn remove_glyph(&mut self, name: &str) -> Result<(), ModelError> {
        let before = self.glyphs.len();
        self.glyphs.retain(|g| g.name != name);
        if self.glyphs.len() == before { Err(ModelError::NoSuchGlyph(name.to_string())) } else { Ok(()) }
    }

    /// The outline of `glyph` on `master` with its components resolved, in font units.
    ///
    /// Bounded work: components nest at most [`MAX_COMPONENT_DEPTH`] deep, the result has at most
    /// [`MAX_OUTLINE_ELEMENTS`] segments, and every point is within [`MAX_COORDINATE`].
    pub fn outline(&self, glyph: &str, master: &str) -> Result<BezPath, ModelError> {
        let g = self.glyph(glyph).ok_or_else(|| ModelError::NoSuchGlyph(glyph.to_string()))?;
        self.outline_of(g, master)
    }

    /// [`Font::outline`] for a glyph already in hand (no lookup by name for the glyph itself).
    pub fn outline_of(&self, glyph: &Glyph, master: &str) -> Result<BezPath, ModelError> {
        let mut budget = MAX_OUTLINE_ELEMENTS;
        self.outline_at(glyph, master, 0, &mut budget)
    }

    fn outline_at(&self, g: &Glyph, master: &str, depth: usize, budget: &mut usize) -> Result<BezPath, ModelError> {
        let glyph = g.name.as_str();
        if depth > MAX_COMPONENT_DEPTH {
            return Err(ModelError::ComponentCycle(glyph.to_string()));
        }
        let bad = |message: &str| ModelError::BadOutline { glyph: glyph.to_string(), message: message.to_string() };
        let mut out = BezPath::new();
        let Some(layer) = g.layer(master) else { return Ok(out) };
        for shape in &layer.shapes {
            let part = match shape {
                Shape::Path(p) => p.to_bez().map_err(|message| bad(&message))?,
                Shape::Component(c) => {
                    if c.transform.iter().any(|v| !v.is_finite()) {
                        return Err(bad("a component transform is not a number"));
                    }
                    let base = self.glyph(&c.glyph).ok_or_else(|| ModelError::NoSuchGlyph(c.glyph.clone()))?;
                    let mut inner = self.outline_at(base, master, depth + 1, budget)?;
                    inner.apply_affine(c.affine());
                    // A transform can push points anywhere: hold the result to the same limits.
                    let far = |p: Point| !(in_range(p.x) && in_range(p.y));
                    let off_limits = inner.elements().iter().any(|el| match *el {
                        fontcraft_geom::PathEl::MoveTo(p) | fontcraft_geom::PathEl::LineTo(p) => far(p),
                        fontcraft_geom::PathEl::QuadTo(a, p) => far(a) || far(p),
                        fontcraft_geom::PathEl::CurveTo(a, b, p) => far(a) || far(b) || far(p),
                        fontcraft_geom::PathEl::ClosePath => false,
                    });
                    if off_limits {
                        return Err(bad(&format!("a component of `{}` puts points beyond ±{MAX_COORDINATE}", c.glyph)));
                    }
                    inner
                }
            };
            // Charge what this glyph adds, including what its components brought, so nested
            // references cannot multiply the work past the budget.
            *budget = budget.checked_sub(part.elements().len()).ok_or_else(|| ModelError::OutlineTooLarge(glyph.to_string()))?;
            out.extend(part);
        }
        Ok(out)
    }

    /// The checks that concern the font as a whole (not the inside of any glyph).
    fn check_font_level(&self) -> Result<(), ModelError> {
        if !UPM_RANGE.contains(&self.units_per_em) {
            return Err(ModelError::BadUnitsPerEm(u32::from(self.units_per_em)));
        }
        check_name("family", &self.family_name)?;
        if self.masters.is_empty() {
            return Err(ModelError::NoMaster);
        }
        let mut ids = HashSet::new();
        for m in &self.masters {
            check_name("master", &m.name)?;
            check_name("master id", &m.id)?;
            if !ids.insert(m.id.as_str()) {
                return Err(ModelError::DuplicateMaster(m.id.clone()));
            }
            if ![m.ascender, m.cap_height, m.x_height, m.descender].into_iter().all(in_range) {
                return Err(ModelError::BadName {
                    what: "master",
                    problem: format!("`{}` has a metric that is not a number or is out of range", m.name),
                });
            }
        }
        if self.glyphs.len() > MAX_GLYPHS {
            return Err(ModelError::TooManyGlyphs);
        }
        let mut names = HashSet::new();
        for g in &self.glyphs {
            if g.name.is_empty() {
                return Err(ModelError::EmptyGlyphName);
            }
            if !names.insert(g.name.as_str()) {
                return Err(ModelError::DuplicateGlyph(g.name.clone()));
            }
        }
        Ok(())
    }

    /// The checks inside one glyph: its name, code points, layers, and that every path is a
    /// well-formed node list within the coordinate limits. (Whether components resolve is checked
    /// when an outline is asked for: a component may name a glyph that is added later.)
    pub fn check_glyph(&self, g: &Glyph) -> Result<(), ModelError> {
        let bad = |message: String| ModelError::BadOutline { glyph: g.name.clone(), message };
        if g.name.is_empty() {
            return Err(ModelError::EmptyGlyphName);
        }
        check_name("glyph", &g.name)?;
        if g.unicodes.len() > MAX_UNICODES {
            return Err(ModelError::TooManyUnicodes { glyph: g.name.clone() });
        }
        if let Some(&value) = g.unicodes.iter().find(|u| char::from_u32(**u).is_none()) {
            return Err(ModelError::BadUnicode { glyph: g.name.clone(), value });
        }
        let mut masters = HashSet::new();
        for l in &g.layers {
            self.master(&l.master)?;
            if !masters.insert(l.master.as_str()) {
                return Err(ModelError::DuplicateLayer { glyph: g.name.clone(), master: l.master.clone() });
            }
            if !in_range(l.width) {
                return Err(bad(format!("the advance width {} is not a number or is beyond ±{MAX_COORDINATE}", l.width)));
            }
            for shape in &l.shapes {
                match shape {
                    Shape::Path(p) => {
                        p.to_bez().map_err(&bad)?;
                    }
                    Shape::Component(c) => {
                        check_name("component glyph", &c.glyph)?;
                        if c.transform.iter().any(|v| !v.is_finite()) {
                            return Err(bad("a component transform is not a number".into()));
                        }
                    }
                }
            }
            if l.anchors.iter().any(|a| !(in_range(a.x) && in_range(a.y))) {
                return Err(bad("an anchor position is not a number or is out of range".into()));
            }
        }
        Ok(())
    }

    /// Every invariant a font must satisfy: what a file must pass to load, and what every
    /// command must leave true. `Err` names the first problem.
    pub fn check(&self) -> Result<(), ModelError> {
        self.check_font_level()?;
        self.glyphs.iter().try_for_each(|g| self.check_glyph(g))
    }

    /// [`Font::check`] for a font that was valid as `before` and has since been edited: only the
    /// glyphs that are no longer shared with `before` are looked into. This is what keeps an edit
    /// to one glyph from costing a pass over the whole font.
    pub fn check_changed(&self, before: &Font) -> Result<(), ModelError> {
        self.check_font_level()?;
        // A master was added, removed or renamed: layers everywhere may be affected.
        if self.masters != before.masters {
            return self.glyphs.iter().try_for_each(|g| self.check_glyph(g));
        }
        let shared: HashSet<*const Glyph> = before.glyphs.iter().map(Arc::as_ptr).collect();
        self.glyphs.iter().filter(|g| !shared.contains(&Arc::as_ptr(g))).try_for_each(|g| self.check_glyph(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_geom::{PathEl, Shape as _};

    fn path(closed: bool, nodes: Vec<Node>) -> Path {
        Path { closed, nodes, extra: Extra::new() }
    }

    #[test]
    fn a_new_font_is_valid_and_has_the_starting_glyphs() {
        let f = Font::new("Test");
        assert_eq!(f.check(), Ok(()));
        assert_eq!(f.glyphs.first().map(|g| g.name.as_str()), Some(".notdef"));
        assert_eq!(f.glyph("A").map(|g| g.unicodes.clone()), Some(vec![0x41]));
        assert_eq!(f.glyphs.len(), 2 + 52);
    }

    #[test]
    fn rect_is_a_closed_four_segment_path() {
        let bez = Path::rect(0.0, 0.0, 10.0, 20.0).to_bez().unwrap();
        assert_eq!(bez.elements().len(), 5); // move, three lines, close
        assert_eq!(bez.bounding_box(), fontcraft_geom::Rect::new(0.0, 0.0, 10.0, 20.0));
    }

    #[test]
    fn a_closed_path_may_start_with_the_off_curves_of_its_last_segment() {
        let n = Node::new;
        // off, off, curve(0,0) closes back; then a line: the same loop written two ways.
        let a = Path {
            closed: true,
            nodes: vec![
                n(0.0, 0.0, NodeKind::Line),
                n(100.0, 0.0, NodeKind::Line),
                n(100.0, 50.0, NodeKind::OffCurve),
                n(50.0, 100.0, NodeKind::OffCurve),
                n(0.0, 100.0, NodeKind::Curve),
            ],
            extra: Extra::new(),
        };
        let b = Path {
            closed: true,
            nodes: vec![
                n(100.0, 50.0, NodeKind::OffCurve),
                n(50.0, 100.0, NodeKind::OffCurve),
                n(0.0, 100.0, NodeKind::Curve),
                n(0.0, 0.0, NodeKind::Line),
                n(100.0, 0.0, NodeKind::Line),
            ],
            extra: Extra::new(),
        };
        let (a, b) = (a.to_bez().unwrap(), b.to_bez().unwrap());
        assert!(a.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))));
        assert!((a.area() - b.area()).abs() < 1e-9);
    }

    #[test]
    fn broken_node_lists_are_errors_not_panics() {
        let n = Node::new;
        for nodes in [
            vec![n(0.0, 0.0, NodeKind::OffCurve)],
            vec![n(0.0, 0.0, NodeKind::Line), n(1.0, 1.0, NodeKind::OffCurve), n(2.0, 2.0, NodeKind::Curve)],
            vec![n(0.0, 0.0, NodeKind::Line), n(1.0, 1.0, NodeKind::OffCurve), n(2.0, 2.0, NodeKind::Line)],
            vec![n(f64::NAN, 0.0, NodeKind::Line)],
            vec![n(0.0, f64::INFINITY, NodeKind::Line)],
            vec![n(MAX_COORDINATE + 1.0, 0.0, NodeKind::Line)],
            vec![n(0.0, 0.0, NodeKind::Line), n(1.0, 1.0, NodeKind::QCurve)],
        ] {
            assert!(path(true, nodes).to_bez().is_err());
        }
        assert!(path(false, vec![n(1.0, 1.0, NodeKind::OffCurve), n(0.0, 0.0, NodeKind::Line)]).to_bez().is_err());
        assert!(path(false, vec![n(1.0, 1.0, NodeKind::OffCurve), n(2.0, 0.0, NodeKind::OffCurve)]).to_bez().is_err());
        assert_eq!(path(true, vec![]).to_bez().map(|b| b.elements().len()), Ok(0));
    }

    #[test]
    fn a_closed_path_of_only_off_curve_nodes_is_a_quadratic_spline() {
        let n = Node::new;
        // Four control points of a rounded square: on-curve points are implied at the midpoints.
        let bez = path(
            true,
            vec![
                n(0.0, 0.0, NodeKind::OffCurve),
                n(100.0, 0.0, NodeKind::OffCurve),
                n(100.0, 100.0, NodeKind::OffCurve),
                n(0.0, 100.0, NodeKind::OffCurve),
            ],
        )
        .to_bez()
        .unwrap();
        assert_eq!(bez.elements().first(), Some(&PathEl::MoveTo(Point::new(0.0, 50.0))));
        assert_eq!(bez.elements().iter().filter(|e| matches!(e, PathEl::QuadTo(..))).count(), 4);
        assert!(bez.area().abs() > 5000.0 && bez.area().abs() < 10000.0);
    }

    #[test]
    fn components_resolve_and_cycles_are_errors() {
        let mut f = Font::new("Test");
        f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        f.glyph_mut("B").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 50.0, 0.0)));
        assert_eq!(f.outline("B", "m01").unwrap().bounding_box(), fontcraft_geom::Rect::new(50.0, 0.0, 150.0, 100.0));
        f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
        assert_eq!(
            f.outline("B", "m01"),
            Err(ModelError::ComponentCycle("A".into())).or_else(|e: ModelError| if matches!(e, ModelError::ComponentCycle(_)) {
                f.outline("B", "m01")
            } else {
                Err(e)
            })
        );
        assert!(matches!(f.outline("B", "m01"), Err(ModelError::ComponentCycle(_))));
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let json = r#"{"family_name":"X","units_per_em":1000,"masters":[{"id":"m","name":"R","ascender":800,"cap_height":700,"x_height":500,"descender":-200,"futureKey":[1,2]}],"glyphs":[{"name":"a","layers":[{"master":"m","width":500,"alsoNew":true}],"note":"hi"}],"fromTheFuture":{"k":1}}"#;
        let f: Font = serde_json::from_str(json).unwrap();
        assert_eq!(f.extra.get("fromTheFuture"), Some(&serde_json::json!({"k": 1})));
        let back: Value = serde_json::to_value(&f).unwrap();
        assert_eq!(back["fromTheFuture"]["k"], 1);
        assert_eq!(back["masters"][0]["futureKey"][1], 2);
        assert_eq!(back["glyphs"][0]["note"], "hi");
        assert_eq!(back["glyphs"][0]["layers"][0]["alsoNew"], true);
    }

    #[test]
    fn check_rejects_duplicates_and_orphan_layers() {
        let mut f = Font::new("Test");
        assert_eq!(f.add_glyph("A", vec![], 500.0), Err(ModelError::DuplicateGlyph("A".into())));
        assert_eq!(f.add_glyph("", vec![], 500.0), Err(ModelError::EmptyGlyphName));
        f.glyph_mut("A").unwrap().layers[0].master = "nope".into();
        assert_eq!(f.check(), Err(ModelError::NoSuchMaster("nope".into())));
    }

    #[test]
    fn check_holds_every_limit() {
        let ok = Font::new("Test");
        let broken = |edit: &dyn Fn(&mut Font)| {
            let mut f = ok.clone();
            edit(&mut f);
            f.check().is_err()
        };
        assert!(broken(&|f| f.family_name = "x".repeat(MAX_NAME_LEN + 1)));
        assert!(broken(&|f| f.family_name = " ".into()));
        assert!(broken(&|f| f.masters.push(f.masters[0].clone())));
        assert!(broken(&|f| f.masters[0].ascender = f64::NAN));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().name = "a\u{0}b".into()));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().unicodes = vec![0xD800]));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().unicodes = vec![u32::MAX]));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().unicodes = (0..=MAX_UNICODES as u32).collect()));
        assert!(broken(&|f| {
            let g = f.glyph_mut("A").unwrap();
            g.layers.push(g.layers[0].clone());
        }));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].width = -1e308));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(path(true, vec![Node::new(0.0, 0.0, NodeKind::Curve)])))));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Component(Component {
            glyph: "B".into(),
            transform: [f64::NAN; 6],
            extra: Extra::new()
        }))));
        assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].anchors.push(Anchor {
            name: "top".into(),
            x: f64::INFINITY,
            y: 0.0,
            extra: Extra::new()
        })));
        assert_eq!(ok.check(), Ok(()));
    }

    #[test]
    fn check_changed_looks_only_at_edited_glyphs() {
        let before = Font::new("Test");
        let mut after = before.clone();
        after.glyph_mut("A").unwrap().layers[0].width = f64::NAN;
        assert!(after.check_changed(&before).is_err());
        // A glyph still shared with `before` is trusted, whatever `before` held.
        let mut bad_before = before.clone();
        bad_before.glyph_mut("B").unwrap().layers[0].width = f64::NAN;
        let mut after = bad_before.clone();
        after.glyph_mut("A").unwrap().layers[0].width = 10.0;
        assert_eq!(after.check_changed(&bad_before), Ok(()));
        assert!(after.check().is_err());
    }

    #[test]
    fn component_fan_out_and_far_transforms_are_bounded() {
        // g0 is a square; g(k) is two copies of g(k-1): 2^k squares.
        let mut f = Font::new("Test");
        f.add_glyph("g0", vec![], 500.0).unwrap();
        f.glyph_mut("g0").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        for k in 1..=24 {
            f.add_glyph(&format!("g{k}"), vec![], 500.0).unwrap();
            let shapes = &mut f.glyph_mut(&format!("g{k}")).unwrap().layers[0].shapes;
            shapes.push(Shape::Component(Component::new(format!("g{}", k - 1), 0.0, 0.0)));
            shapes.push(Shape::Component(Component::new(format!("g{}", k - 1), 1.0, 0.0)));
        }
        assert_eq!(f.check(), Ok(()));
        assert!(f.outline("g5", "m01").is_ok());
        let start = std::time::Instant::now();
        assert!(matches!(f.outline("g24", "m01"), Err(ModelError::OutlineTooLarge(_))));
        assert!(start.elapsed().as_secs() < 5, "the budget must stop the work early");

        let mut f = Font::new("Test");
        f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        for huge in [[1e300, 0.0, 0.0, 1e300, 0.0, 0.0], [1e308, 1e308, -1e308, -1e308, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, 1e12, 0.0]] {
            let mut g = f.clone();
            g.glyph_mut("B").unwrap().layers[0].shapes =
                vec![Shape::Component(Component { glyph: "A".into(), transform: huge, extra: Extra::new() })];
            assert!(matches!(g.outline("B", "m01"), Err(ModelError::BadOutline { .. })), "{huge:?}");
        }
    }

    #[test]
    fn unknown_keys_survive_inside_shapes_too() {
        let json = r#"{"family_name":"X","units_per_em":1000,"masters":[{"id":"m","name":"R","ascender":800,"cap_height":700,"x_height":500,"descender":-200}],"glyphs":[{"name":"a","layers":[{"master":"m","width":500,"shapes":[{"path":{"closed":true,"pathKey":1,"nodes":[{"x":0,"y":0,"type":"line","name":"start","userData":{"k":[1]}}]}},{"component":{"glyph":"a2","anchor":"top"}}],"anchors":[{"name":"top","x":1,"y":2,"note":"n"}]}]}]}"#;
        let f: Font = serde_json::from_str(json).unwrap();
        let back: Value = serde_json::to_value(&f).unwrap();
        let layer = &back["glyphs"][0]["layers"][0];
        assert_eq!(layer["shapes"][0]["path"]["pathKey"], 1);
        assert_eq!(layer["shapes"][0]["path"]["nodes"][0]["name"], "start");
        assert_eq!(layer["shapes"][0]["path"]["nodes"][0]["userData"]["k"][0], 1);
        assert_eq!(layer["shapes"][1]["component"]["anchor"], "top");
        assert_eq!(layer["anchors"][0]["note"], "n");
    }
}

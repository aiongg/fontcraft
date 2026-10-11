//! Outlines and font-building helpers for the test fonts.
//!
//! Every outline is a plain geometric shape (rectangles, polygons, ovals made of four curves) drawn
//! here; none is taken from an existing font. Outer contours run counter-clockwise and counters
//! clockwise. A closed path ends on its start node, as the `.glyphs` format stores it, so the node
//! indices hints refer to are the indices of the lists built here.

use std::sync::Arc;

use fontcraft_engine::model::{
    Anchor, Axis, Component, Font, GlyphTable, Hint, Instance, Layer, Master, MetricDef, MetricValue, Node, NodeKind, Param, Path, Shape,
    SourceFormat, Transform, metric,
};
use fontcraft_plist::{Value, openstep};

/// Control-point distance for a quarter ellipse, as a fraction of its radius.
const K: f64 = 0.5523;

/// The proportions one master draws with.
#[derive(Clone, Debug)]
pub struct Style {
    pub id: String,
    /// Vertical stem width.
    pub stem: f64,
    /// Horizontal stroke thickness.
    pub bar: f64,
    pub cap: f64,
    pub xh: f64,
    pub asc: f64,
    pub desc: f64,
    pub over: f64,
    /// Horizontal stretch (1 for a normal width).
    pub wide: f64,
}

impl Style {
    /// The style of a master at weight `w` (100 thin to 900 black).
    pub fn weight(id: &str, w: f64) -> Self {
        let stem = (20.0 + 0.15 * w).round();
        Style { id: id.into(), stem, bar: (stem * 0.875).round(), cap: 700.0, xh: 500.0, asc: 750.0, desc: -250.0, over: 12.0, wide: 1.0 }
    }
}

/// One master of a font being built: id, name, axis location and weight for [`Style::weight`].
pub struct MasterSpec<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub location: Vec<f64>,
    pub weight: f64,
}

pub fn m<'a>(id: &'a str, name: &'a str, location: &[f64], weight: f64) -> MasterSpec<'a> {
    MasterSpec { id, name, location: location.to_vec(), weight }
}

/// An empty font with the given axes and masters, the five usual metrics with 12-unit
/// overshoots (ascender 750, cap height 700, x-height 500, baseline 0, descender −250), and the
/// styles to draw each master with.
pub fn font(family: &str, axes: &[(&str, &str)], masters: &[MasterSpec]) -> (Font, Vec<Style>) {
    let mut f = Font::new(family);
    f.source.format = SourceFormat::Glyphs;
    f.source.glyphs_version = Some(3);
    f.glyphs = GlyphTable::from_glyphs(Vec::new());
    f.axes = Arc::new(axes.iter().map(|(name, tag)| Axis { name: (*name).into(), tag: (*tag).into(), ..Default::default() }).collect());
    f.metrics = Arc::new([metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER].map(MetricDef::new).to_vec());
    let styles: Vec<Style> = masters.iter().map(|ms| Style::weight(ms.id, ms.weight)).collect();
    f.masters = Arc::new(
        masters
            .iter()
            .zip(&styles)
            .map(|(ms, s)| Master {
                id: ms.id.into(),
                name: ms.name.into(),
                location: ms.location.clone(),
                metric_values: [(s.asc, s.over), (s.cap, s.over), (s.xh, s.over), (0.0, -s.over), (s.desc, -s.over)]
                    .map(|(pos, over)| MetricValue { pos, over })
                    .to_vec(),
                visible: true,
                ..Default::default()
            })
            .collect(),
    );
    (f, styles)
}

/// A property-list value from OpenStep text (constant text in this module).
pub fn pv(text: &str) -> Value {
    openstep::parse_value(text).unwrap_or_else(|e| panic!("bad plist text `{text}`: {e}"))
}

pub fn param(name: &str, value: &str) -> Param {
    Param::new(name, pv(value))
}

/// A hint stored as a format version 3 hint dictionary.
pub fn hint(text: &str) -> Hint {
    Hint { family: fontcraft_engine::model::Family::Glyphs, value: pv(text) }
}

pub fn instance(name: &str, location: &[f64]) -> Instance {
    Instance { name: name.into(), location: location.to_vec(), ..Default::default() }
}

// ---------------------------------------------------------------------------------------------
// Paths

fn node(x: f64, y: f64, kind: NodeKind) -> Node {
    Node::new(x, y, kind)
}

/// Builds a path segment by segment from a start node.
pub struct Pen {
    start: (f64, f64),
    nodes: Vec<Node>,
}

impl Pen {
    pub fn at(x: f64, y: f64) -> Self {
        Pen { start: (x, y), nodes: Vec::new() }
    }
    pub fn line(mut self, x: f64, y: f64) -> Self {
        self.nodes.push(node(x, y, NodeKind::Line));
        self
    }
    pub fn curve(mut self, c1: (f64, f64), c2: (f64, f64), p: (f64, f64)) -> Self {
        self.nodes.push(node(c1.0, c1.1, NodeKind::OffCurve));
        self.nodes.push(node(c2.0, c2.1, NodeKind::OffCurve));
        let mut n = node(p.0, p.1, NodeKind::Curve);
        n.smooth = true;
        self.nodes.push(n);
        self
    }
    /// A quarter ellipse from the current point, leaving vertically and arriving horizontally.
    pub fn arc_vh(self, to: (f64, f64)) -> Self {
        let from = self.last();
        self.curve((from.0, from.1 + (to.1 - from.1) * K), (to.0 + (from.0 - to.0) * K, to.1), to)
    }
    /// A quarter ellipse from the current point, leaving horizontally and arriving vertically.
    pub fn arc_hv(self, to: (f64, f64)) -> Self {
        let from = self.last();
        self.curve((from.0 + (to.0 - from.0) * K, from.1), (to.0, to.1 + (from.1 - to.1) * K), to)
    }
    fn last(&self) -> (f64, f64) {
        self.nodes.iter().rev().find(|n| n.kind != NodeKind::OffCurve).map_or(self.start, |n| (n.x, n.y))
    }
    /// The closed path. A last segment that ends on the start point closes it; otherwise a line
    /// does. The start node goes last, where the format keeps it.
    pub fn close(mut self) -> Path {
        let mut start = node(self.start.0, self.start.1, NodeKind::Line);
        if let Some(n) = self.nodes.last()
            && n.kind != NodeKind::OffCurve
            && (n.x - start.x).abs() < 1e-9
            && (n.y - start.y).abs() < 1e-9
        {
            start = self.nodes.pop().unwrap_or(start);
        }
        self.nodes.push(start);
        Path::new(true, self.nodes)
    }
    pub fn open(mut self) -> Path {
        self.nodes.insert(0, node(self.start.0, self.start.1, NodeKind::Line));
        Path::new(false, self.nodes)
    }
}

pub fn poly(points: &[(f64, f64)]) -> Path {
    let Some((&(x, y), rest)) = points.split_first() else { return Path::new(true, Vec::new()) };
    rest.iter().fold(Pen::at(x, y), |p, &(x, y)| p.line(x, y)).close()
}

pub fn open_poly(points: &[(f64, f64)]) -> Path {
    let Some((&(x, y), rest)) = points.split_first() else { return Path::new(false, Vec::new()) };
    rest.iter().fold(Pen::at(x, y), |p, &(x, y)| p.line(x, y)).open()
}

pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Path {
    poly(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

/// A clockwise rectangle (a counter).
pub fn rect_cw(x0: f64, y0: f64, x1: f64, y1: f64) -> Path {
    poly(&[(x0, y0), (x0, y1), (x1, y1), (x1, y0)])
}

/// A counter-clockwise oval. Node indices: right 2, top 5, left 8, bottom 11.
pub fn oval(cx: f64, cy: f64, rx: f64, ry: f64) -> Path {
    Pen::at(cx, cy - ry).arc_hv((cx + rx, cy)).arc_vh((cx, cy + ry)).arc_hv((cx - rx, cy)).arc_vh((cx, cy - ry)).close()
}

/// A clockwise oval (a counter). Node indices: left 2, top 5, right 8, bottom 11.
pub fn oval_cw(cx: f64, cy: f64, rx: f64, ry: f64) -> Path {
    Pen::at(cx, cy - ry).arc_hv((cx - rx, cy)).arc_vh((cx, cy + ry)).arc_hv((cx + rx, cy)).arc_vh((cx, cy - ry)).close()
}

/// `p` moved by (`dx`, `dy`) after scaling by (`sx`, `sy`) about the origin. A mirroring scale
/// reverses the path so it keeps its direction.
pub fn transformed(p: &Path, sx: f64, sy: f64, dx: f64, dy: f64) -> Path {
    let mut out = p.clone();
    for n in &mut out.nodes {
        n.x = n.x * sx + dx;
        n.y = n.y * sy + dy;
    }
    if sx * sy < 0.0 {
        out.reverse();
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Drawings

/// One master layer's content before it becomes a [`Layer`].
#[derive(Clone, Debug, Default)]
pub struct Draw {
    pub width: f64,
    pub shapes: Vec<Shape>,
    pub anchors: Vec<Anchor>,
}

impl Draw {
    pub fn new(width: f64) -> Self {
        Draw { width, ..Default::default() }
    }
    pub fn path(mut self, p: Path) -> Self {
        self.shapes.push(Shape::Path(p));
        self
    }
    pub fn paths(mut self, ps: impl IntoIterator<Item = Path>) -> Self {
        self.shapes.extend(ps.into_iter().map(Shape::Path));
        self
    }
    pub fn comp(mut self, glyph: &str, dx: f64, dy: f64) -> Self {
        self.shapes.push(Shape::Component(Component::new(glyph, dx, dy)));
        self
    }
    pub fn comp_scaled(mut self, glyph: &str, dx: f64, dy: f64, sx: f64, sy: f64) -> Self {
        let mut c = Component::new(glyph, 0.0, 0.0);
        c.transform = Transform::Decomposed { pos: (dx, dy), scale: (sx, sy), angle: 0.0, slant: (0.0, 0.0) };
        self.shapes.push(Shape::Component(c));
        self
    }
    pub fn anchor(mut self, name: &str, x: f64, y: f64) -> Self {
        self.anchors.push(Anchor::new(name, x, y));
        self
    }
    /// Shapes and anchors moved right by `lsb`, with `rsb` after the drawn extent `body`.
    pub fn spaced(mut self, lsb: f64, body: f64, rsb: f64) -> Self {
        self = self.shifted(lsb, 0.0);
        self.width = (lsb + body + rsb).round();
        self
    }
    pub fn shifted(mut self, dx: f64, dy: f64) -> Self {
        for s in &mut self.shapes {
            match s {
                Shape::Path(p) => *p = transformed(p, 1.0, 1.0, dx, dy),
                Shape::Component(c) => {
                    if let Transform::Decomposed { pos, .. } = &mut c.transform {
                        pos.0 += dx;
                        pos.1 += dy;
                    }
                }
                _ => {}
            }
        }
        for a in &mut self.anchors {
            a.x += dx;
            a.y += dy;
        }
        self
    }
    /// Paths and anchors scaled about the origin (components are left alone); the width scales
    /// by `sx`.
    pub fn scaled(mut self, sx: f64, sy: f64) -> Self {
        for s in &mut self.shapes {
            if let Shape::Path(p) = s {
                *p = transformed(p, sx, sy, 0.0, 0.0);
            }
        }
        for a in &mut self.anchors {
            a.x *= sx;
            a.y *= sy;
        }
        self.width = (self.width * sx).round();
        self
    }
    pub fn anchor_pos(&self, name: &str) -> Option<(f64, f64)> {
        self.anchors.iter().find(|a| a.name == name).map(|a| (a.x, a.y))
    }
    /// The layer, with coordinates rounded to whole units.
    pub fn into_layer(mut self, master: &str) -> Layer {
        for s in &mut self.shapes {
            match s {
                Shape::Path(p) => {
                    for n in &mut p.nodes {
                        n.x = n.x.round();
                        n.y = n.y.round();
                    }
                }
                Shape::Component(c) => {
                    if let Transform::Decomposed { pos, .. } = &mut c.transform {
                        *pos = (pos.0.round(), pos.1.round());
                    }
                }
                _ => {}
            }
        }
        for a in &mut self.anchors {
            a.x = a.x.round();
            a.y = a.y.round();
        }
        let mut l = Layer::new(master, self.width.round());
        l.shapes = self.shapes;
        l.anchors = self.anchors;
        l
    }
}

/// Adds a glyph with one master layer per style, drawn by `draw`.
pub fn add(font: &mut Font, styles: &[Style], name: &str, unicodes: &[u32], draw: impl Fn(&Style) -> Draw) {
    let mut g = fontcraft_engine::model::Glyph::new(name);
    g.unicodes = unicodes.to_vec();
    g.layers = styles.iter().map(|s| draw(s).into_layer(&s.id)).collect();
    font.glyphs.push(g);
}

/// Adds a glyph drawn by [`letter`].
pub fn add_letter(font: &mut Font, styles: &[Style], name: &str, unicodes: &[u32]) {
    add(font, styles, name, unicodes, |s| letter(name, s));
}

/// Adds several glyphs drawn by [`letter`], encoded by the code point of a one-character name or
/// by the code point given.
pub fn add_letters(font: &mut Font, styles: &[Style], names: &[(&str, Option<u32>)]) {
    for (name, cp) in names {
        let unis: Vec<u32> = match cp {
            Some(c) => vec![*c],
            None => {
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => vec![c as u32],
                    _ => Vec::new(),
                }
            }
        };
        add_letter(font, styles, name, &unis);
    }
}

/// The master layers' drawings of glyph `name`, in master order.
pub fn drawings(font: &Font, name: &str) -> Vec<Draw> {
    let Some(g) = font.glyph(name) else { return Vec::new() };
    font.masters
        .iter()
        .map(|m| match g.layer(&m.id) {
            Some(l) => Draw { width: l.width, shapes: l.shapes.clone(), anchors: l.anchors.clone() },
            None => Draw::new(0.0),
        })
        .collect()
}

/// Adds a composite: `base` and each mark attached by its anchor (`top` takes the mark's `_top`),
/// placed from the anchors of each master. A flipped mark is mirrored vertically and attached
/// by its `_top` anchor to the base's `anchor`.
pub fn add_composite(font: &mut Font, styles: &[Style], name: &str, unicodes: &[u32], base: &str, marks: &[(&str, &str, bool)]) {
    let bases = drawings(font, base);
    let mark_draws: Vec<Vec<Draw>> = marks.iter().map(|(m, _, _)| drawings(font, m)).collect();
    let mut g = fontcraft_engine::model::Glyph::new(name);
    g.unicodes = unicodes.to_vec();
    for (i, s) in styles.iter().enumerate() {
        let b = bases.get(i).cloned().unwrap_or_default();
        let mut d = Draw::new(b.width).comp(base, 0.0, 0.0);
        d.anchors = b.anchors.iter().filter(|a| !a.name.starts_with('_')).cloned().collect();
        for ((mark, anchor, flipped), draws) in marks.iter().zip(&mark_draws) {
            let md = draws.get(i).cloned().unwrap_or_default();
            let target = d.anchor_pos(anchor).unwrap_or((b.width / 2.0, s.cap));
            let own = if *flipped { "_top".to_string() } else { format!("_{anchor}") };
            let (mx, my) = md.anchor_pos(&own).unwrap_or((md.width / 2.0, 0.0));
            if *flipped {
                d = d.comp_scaled(mark, target.0 - mx, target.1 + my, 1.0, -1.0);
            } else {
                d = d.comp(mark, target.0 - mx, target.1 - my);
                // Stacking: the base's anchor moves to where the mark's own anchor lands.
                if let Some((tx, ty)) = md.anchor_pos(anchor)
                    && let Some(a) = d.anchors.iter_mut().find(|a| a.name == *anchor)
                {
                    a.x = tx + target.0 - mx;
                    a.y = ty + target.1 - my;
                }
            }
        }
        g.layers.push(d.into_layer(&s.id));
    }
    font.glyphs.push(g);
}

// ---------------------------------------------------------------------------------------------
// Letters

/// Side bearing used around most drawings.
const SB: f64 = 50.0;

fn cap_ring(s: &Style, w: f64, h: f64) -> Vec<Path> {
    vec![rect(0.0, 0.0, w, h), rect_cw(s.stem, s.bar, w - s.stem, h - s.bar)]
}

/// An n-like arch shape: a left stem rising to `top`, one arch of width `w` at the x-height.
fn arch(s: &Style, w: f64, top: f64) -> Path {
    let (st, xh) = (s.stem, s.xh);
    let cx = w / 2.0 + st / 4.0;
    let a1 = xh * 0.55;
    Pen::at(0.0, 0.0)
        .line(st, 0.0)
        .line(st, a1)
        .arc_vh((cx, xh - s.bar))
        .arc_hv((w - st, a1))
        .line(w - st, 0.0)
        .line(w, 0.0)
        .line(w, a1)
        .arc_vh((cx, xh + s.over))
        .arc_hv((st, xh - st * 0.6))
        .line(st, top)
        .line(0.0, top)
        .close()
}

/// An m-like shape: a left stem and two arches over a total width `w`.
fn double_arch(s: &Style, w: f64) -> Path {
    let (st, xh) = (s.stem, s.xh);
    let mid = (w - st) / 2.0;
    let a1 = xh * 0.55;
    let c1 = (st + mid) / 2.0 + st / 4.0;
    let c2 = (mid + w) / 2.0 + st / 4.0;
    Pen::at(0.0, 0.0)
        .line(st, 0.0)
        .line(st, a1)
        .arc_vh((c1, xh - s.bar))
        .arc_hv((mid, a1))
        .line(mid, 0.0)
        .line(mid + st, 0.0)
        .line(mid + st, a1)
        .arc_vh((c2, xh - s.bar))
        .arc_hv((w - st, a1))
        .line(w - st, 0.0)
        .line(w, 0.0)
        .line(w, a1)
        .arc_vh((c2, xh + s.over))
        .arc_hv((mid + st, xh - st * 0.6))
        .arc_vh((c1, xh + s.over))
        .arc_hv((st, xh - st * 0.6))
        .line(st, xh)
        .line(0.0, xh)
        .close()
}

/// A V: two strokes meeting at the bottom, one contour.
fn vee(s: &Style, w: f64, h: f64) -> Path {
    let cx = w / 2.0;
    let half = s.stem * 0.65;
    let d = s.stem * 1.15;
    let yn = h * (1.0 - (w - d - cx) / (w - cx - half));
    poly(&[(cx - half, 0.0), (cx + half, 0.0), (w, h), (w - d, h), (cx, yn), (d, h), (0.0, h)])
}

/// An inverted V (an A without its crossbar).
fn lambda(s: &Style, w: f64, h: f64) -> Path {
    transformed(&vee(s, w, h), 1.0, -1.0, 0.0, h)
}

/// The A: a lambda and a crossbar that overlaps both strokes.
fn a_cap(s: &Style, w: f64, h: f64) -> Vec<Path> {
    let y = h * 0.3;
    vec![lambda(s, w, h), rect(w * 0.22, y, w * 0.78, y + s.bar)]
}

/// The H as one contour.
fn h_cap(s: &Style, w: f64, h: f64) -> Path {
    let (st, yb, yt) = (s.stem, h / 2.0 - s.bar / 2.0, h / 2.0 + s.bar / 2.0);
    poly(&[(0.0, 0.0), (st, 0.0), (st, yb), (w - st, yb), (w - st, 0.0), (w, 0.0), (w, h), (w - st, h), (w - st, yt), (st, yt), (st, h), (0.0, h)])
}

fn e_cap(s: &Style, w: f64, h: f64) -> Path {
    let (st, b, ym) = (s.stem, s.bar, h / 2.0);
    poly(&[
        (0.0, 0.0),
        (w, 0.0),
        (w, b),
        (st, b),
        (st, ym - b / 2.0),
        (w * 0.9, ym - b / 2.0),
        (w * 0.9, ym + b / 2.0),
        (st, ym + b / 2.0),
        (st, h - b),
        (w, h - b),
        (w, h),
        (0.0, h),
    ])
}

/// An o-like ring between `bottom` and `top` (overshoots included).
fn ring(s: &Style, w: f64, bottom: f64, top: f64) -> Vec<Path> {
    let (cy, ry) = ((bottom + top) / 2.0, (top - bottom) / 2.0);
    vec![oval(w / 2.0, cy, w / 2.0, ry), oval_cw(w / 2.0, cy, w / 2.0 - s.stem, ry - s.bar)]
}

/// An e: an oval with a counter above and below its bar.
fn e_lower(s: &Style, w: f64) -> Vec<Path> {
    let (lo, hi) = (-s.over, s.xh + s.over);
    let mid = s.xh / 2.0;
    let rx = w / 2.0 - s.stem;
    let up = (mid + s.bar / 2.0, hi - s.bar);
    let down = (lo + s.bar, mid - s.bar / 2.0);
    vec![
        oval(w / 2.0, (lo + hi) / 2.0, w / 2.0, (hi - lo) / 2.0),
        oval_cw(w / 2.0, (up.0 + up.1) / 2.0, rx, (up.1 - up.0) / 2.0),
        oval_cw(w / 2.0, (down.0 + down.1) / 2.0, rx, (down.1 - down.0) / 2.0),
    ]
}

/// Two parallelogram strokes crossing; with `crossing_nodes`, each stroke has nodes where its
/// edges cross the other stroke's.
pub fn x_strokes(s: &Style, w: f64, h: f64, crossing_nodes: bool) -> Vec<Path> {
    let st = s.stem * 1.1;
    let a = [(0.0, 0.0), (st, 0.0), (w, h), (w - st, h)];
    let b = [(w - st, 0.0), (w, 0.0), (st, h), (0.0, h)];
    if !crossing_nodes {
        return vec![poly(&a), poly(&b)];
    }
    vec![poly(&with_crossings(&a, &b)), poly(&with_crossings(&b, &a))]
}

/// The polygon `p` with a node added wherever one of its edges crosses an edge of `other`.
fn with_crossings(p: &[(f64, f64)], other: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for i in 0..p.len() {
        let (a0, a1) = (p[i], p[(i + 1) % p.len()]);
        out.push(a0);
        let mut hits: Vec<(f64, (f64, f64))> = Vec::new();
        for j in 0..other.len() {
            let (b0, b1) = (other[j], other[(j + 1) % other.len()]);
            if let Some((t, pt)) = cross(a0, a1, b0, b1) {
                hits.push((t, pt));
            }
        }
        hits.sort_by(|x, y| x.0.total_cmp(&y.0));
        out.extend(hits.into_iter().map(|(_, pt)| (pt.0.round(), pt.1.round())));
    }
    out
}

fn cross(a0: (f64, f64), a1: (f64, f64), b0: (f64, f64), b1: (f64, f64)) -> Option<(f64, (f64, f64))> {
    let (dax, day, dbx, dby) = (a1.0 - a0.0, a1.1 - a0.1, b1.0 - b0.0, b1.1 - b0.1);
    let den = dax * dby - day * dbx;
    if den.abs() < 1e-9 {
        return None;
    }
    let t = ((b0.0 - a0.0) * dby - (b0.1 - a0.1) * dbx) / den;
    let u = ((b0.0 - a0.0) * day - (b0.1 - a0.1) * dax) / den;
    (t > 1e-6 && t < 1.0 - 1e-6 && u > 1e-6 && u < 1.0 - 1e-6).then_some((t, (a0.0 + t * dax, a0.1 + t * day)))
}

/// A small square used to tell variants apart, `n` steps down from `top`.
fn marker(s: &Style, x: f64, top: f64, n: f64) -> Path {
    let size = (s.stem * 0.8).max(30.0);
    let y = top + 30.0 - n * (size + 10.0);
    rect(x, y, x + size, y + size)
}

/// Accent marks: (name, shape kind). Top marks sit above the x-height, bottom marks below the
/// baseline.
fn mark_draw(name: &str, s: &Style) -> Option<Draw> {
    let w = 200.0;
    let cx = w / 2.0;
    let st = s.stem.max(40.0);
    let top = |paths: Vec<Path>| Some(Draw::new(w).paths(paths).anchor("_top", cx, s.xh).anchor("top", cx, s.xh + 220.0));
    let bottom = |paths: Vec<Path>| Some(Draw::new(w).paths(paths).anchor("_bottom", cx, 0.0).anchor("bottom", cx, -200.0));
    let y0 = s.xh + 60.0;
    match name {
        "gravecomb" => top(vec![poly(&[(cx - 70.0, y0 + 140.0), (cx - 70.0 + st, y0 + 140.0), (cx + 40.0, y0), (cx + 40.0 - st, y0)])]),
        "acutecomb" | "acute" | "acutecomb.case" => {
            top(vec![poly(&[(cx - 40.0, y0), (cx - 40.0 + st, y0), (cx + 70.0, y0 + 140.0), (cx + 70.0 - st, y0 + 140.0)])])
        }
        "dieresiscomb" | "dieresis" => top(vec![rect(cx - 80.0, y0, cx - 80.0 + st, y0 + st), rect(cx + 80.0 - st, y0, cx + 80.0, y0 + st)]),
        "dotaccentcomb" => top(vec![rect(cx - st / 2.0, y0, cx + st / 2.0, y0 + st)]),
        "ringcomb" => {
            let r = 60.0;
            top(vec![oval(cx, y0 + r, r, r), oval_cw(cx, y0 + r, r - st * 0.5, r - st * 0.5)])
        }
        "caroncomb" => top(vec![vee(s, 160.0, 120.0)]).map(|d| d.shifted(cx - 80.0, y0)),
        "circumflexcomb" => top(vec![lambda(s, 160.0, 120.0)]).map(|d| d.shifted(cx - 80.0, y0)),
        "brevecomb" | "tildecomb" | "macroncomb" => top(vec![rect(cx - 80.0, y0 + 20.0, cx + 80.0, y0 + 20.0 + s.bar.min(50.0))]),
        "cedillacomb" | "commaaccentcomb" | "ogonekcomb" => {
            bottom(vec![poly(&[(cx - 20.0, -40.0), (cx + 20.0, -40.0), (cx + 40.0, -180.0), (cx, -180.0)])])
        }
        "dotbelowcomb" => bottom(vec![rect(cx - st / 2.0, -60.0 - st, cx + st / 2.0, -60.0)]),
        _ => None,
    }
}

/// A placeholder drawing for any glyph name: real shapes for a few letters, generic ones by
/// script and case for the rest, and a variation for each suffix so that variants differ.
pub fn letter(name: &str, s: &Style) -> Draw {
    if let Some(d) = mark_draw(name, s) {
        return d;
    }
    let (base, suffix) = match name.split_once('.') {
        Some((b, sfx)) if !b.is_empty() => (b, sfx),
        _ => (name, ""),
    };
    let d = base_drawing(base, s);
    with_suffix(d, base, suffix, s)
}

fn is_upper(base: &str) -> bool {
    base.chars().next().is_some_and(char::is_uppercase)
}

fn base_drawing(base: &str, s: &Style) -> Draw {
    let (st, cap, xh) = (s.stem, s.cap, s.xh);
    let wd = |w: f64| (w * s.wide).round();
    let lower_ring = |w: f64| ring(s, w, -s.over, xh + s.over);
    match base {
        "space" => Draw::new(wd(250.0)),
        "H" => Draw::new(0.0).path(h_cap(s, wd(560.0), cap)).spaced(SB + 20.0, wd(560.0), SB + 20.0),
        "I" => Draw::new(0.0).path(rect(0.0, 0.0, st, cap)).spaced(SB + 20.0, st, SB + 20.0),
        "L" => Draw::new(0.0).path(poly(&[(0.0, 0.0), (wd(440.0), 0.0), (wd(440.0), s.bar), (st, s.bar), (st, cap), (0.0, cap)])).spaced(
            SB + 20.0,
            wd(440.0),
            SB,
        ),
        "E" => Draw::new(0.0).path(e_cap(s, wd(460.0), cap)).spaced(SB + 20.0, wd(460.0), SB),
        "F" => Draw::new(0.0)
            .path(poly(&[
                (0.0, 0.0),
                (st, 0.0),
                (st, cap / 2.0 - s.bar / 2.0),
                (wd(400.0), cap / 2.0 - s.bar / 2.0),
                (wd(400.0), cap / 2.0 + s.bar / 2.0),
                (st, cap / 2.0 + s.bar / 2.0),
                (st, cap - s.bar),
                (wd(450.0), cap - s.bar),
                (wd(450.0), cap),
                (0.0, cap),
            ]))
            .spaced(SB + 20.0, wd(450.0), SB),
        "T" => {
            let w = wd(560.0);
            let c = w / 2.0;
            Draw::new(0.0)
                .path(poly(&[
                    (c - st / 2.0, 0.0),
                    (c + st / 2.0, 0.0),
                    (c + st / 2.0, cap - s.bar),
                    (w, cap - s.bar),
                    (w, cap),
                    (0.0, cap),
                    (0.0, cap - s.bar),
                    (c - st / 2.0, cap - s.bar),
                ]))
                .spaced(SB - 20.0, w, SB - 20.0)
        }
        "P" => {
            let w = wd(500.0);
            let yb = cap * 0.4;
            Draw::new(0.0)
                .path(poly(&[(0.0, 0.0), (st, 0.0), (st, yb), (w, yb), (w, cap), (0.0, cap)]))
                .path(rect_cw(st, yb + s.bar, w - st, cap - s.bar))
                .spaced(SB + 20.0, w, SB)
        }
        "B" => {
            let w = wd(500.0);
            let ym = cap * 0.52;
            Draw::new(0.0)
                .path(rect(0.0, 0.0, w, cap))
                .path(rect_cw(st, s.bar, w - st, ym - s.bar / 2.0))
                .path(rect_cw(st, ym + s.bar / 2.0, w - st * 1.3, cap - s.bar))
                .spaced(SB + 20.0, w, SB)
        }
        "O" | "Q" | "C" | "G" => {
            let w = wd(640.0);
            let mut d = Draw::new(0.0).paths(ring(s, w, -s.over, cap + s.over));
            match base {
                "Q" => d = d.path(poly(&[(w * 0.55, 20.0), (w * 0.55 + st, 20.0), (w * 0.9, -120.0), (w * 0.9 - st, -120.0)])),
                "C" => d = d.path(rect(w * 0.7, cap * 0.35, w + 1.0, cap * 0.65)),
                "G" => d = d.path(rect(w * 0.5, cap * 0.3, w - st * 0.5, cap * 0.3 + s.bar)),
                _ => {}
            }
            d.spaced(SB - 10.0, w, SB - 10.0)
        }
        "V" => Draw::new(0.0).path(vee(s, wd(620.0), cap)).spaced(SB - 40.0, wd(620.0), SB - 40.0),
        "W" => {
            let w = wd(460.0);
            Draw::new(0.0).path(vee(s, w, cap)).path(vee(s, w, cap)).shift_second(w * 0.6).spaced(SB - 40.0, w * 1.6, SB - 40.0)
        }
        "Y" => {
            let w = wd(600.0);
            let c = w / 2.0;
            Draw::new(0.0)
                .path(transformed(&vee(s, w, cap * 0.5), 1.0, 1.0, 0.0, cap * 0.5))
                .path(rect(c - st / 2.0, 0.0, c + st / 2.0, cap * 0.5 + st))
                .spaced(SB - 40.0, w, SB - 40.0)
        }
        "A" => Draw::new(0.0).paths(a_cap(s, wd(640.0), cap)).spaced(SB - 40.0, wd(640.0), SB - 40.0),
        "X" => Draw::new(0.0).paths(x_strokes(s, wd(600.0), cap, false)).spaced(SB - 30.0, wd(600.0), SB - 30.0),
        "n" => Draw::new(0.0).path(arch(s, wd(440.0), xh)).spaced(SB + 10.0, wd(440.0), SB),
        "h" => Draw::new(0.0).path(arch(s, wd(440.0), s.asc)).spaced(SB + 10.0, wd(440.0), SB),
        "m" => Draw::new(0.0).path(double_arch(s, wd(700.0))).spaced(SB + 10.0, wd(700.0), SB),
        "u" => {
            let w = wd(440.0);
            Draw::new(0.0).path(transformed(&arch(s, w, xh), -1.0, -1.0, w, xh)).spaced(SB, w, SB + 10.0)
        }
        "o" => Draw::new(0.0).paths(lower_ring(wd(480.0))).spaced(SB - 10.0, wd(480.0), SB - 10.0),
        "e" => Draw::new(0.0).paths(e_lower(s, wd(480.0))).spaced(SB - 10.0, wd(480.0), SB - 10.0),
        "eight" => {
            // Two rings that overlap where they meet, with their stems at different positions.
            let w = wd(500.0);
            Draw::new(0.0)
                .paths(cap_ring(s, w, cap * 0.54))
                .paths(cap_ring(s, w - 80.0, cap * 0.46).into_iter().map(|p| transformed(&p, 1.0, 1.0, 40.0, cap * 0.54)))
                .spaced(SB, w, SB)
        }
        "l" | "idotless" | "dotlessi" => {
            let top = if base == "l" { s.asc } else { xh };
            Draw::new(0.0).path(rect(0.0, 0.0, st, top)).spaced(SB + 20.0, st, SB + 20.0)
        }
        "i" | "j" => {
            let bottom = if base == "j" { s.desc } else { 0.0 };
            Draw::new(0.0).path(rect(0.0, bottom, st, xh)).path(rect(0.0, xh + 80.0, st, xh + 80.0 + st)).spaced(SB + 20.0, st, SB + 20.0)
        }
        "period" | "periodcentered" => {
            let y = if base == "periodcentered" { xh / 2.0 - st / 2.0 } else { 0.0 };
            Draw::new(0.0).path(rect(0.0, y, st, y + st)).spaced(SB + 20.0, st, SB + 20.0)
        }
        "comma" => Draw::new(0.0)
            .path(poly(&[(0.0, 0.0), (st, 0.0), (st, st), (0.0, st)]))
            .path(poly(&[(st * 0.2, -100.0), (st * 0.6, -100.0), (st, 1.0), (st * 0.4, 1.0)]))
            .spaced(SB + 20.0, st, SB + 20.0),
        "hyphen" => Draw::new(0.0).path(rect(0.0, xh / 2.0 - s.bar / 2.0, 260.0, xh / 2.0 + s.bar / 2.0)).spaced(SB, 260.0, SB),
        "slash" | "fraction" => {
            let w = 320.0;
            Draw::new(0.0).path(poly(&[(0.0, s.desc * 0.3), (st, s.desc * 0.3), (w, cap), (w - st, cap)])).spaced(SB - 20.0, w, SB - 20.0)
        }
        "bullet" => Draw::new(0.0).path(oval(100.0, xh / 2.0, 100.0, 100.0)).spaced(SB, 200.0, SB),
        "ordfeminine" | "ordmasculine" => {
            Draw::new(0.0).paths(ring(s, 260.0, cap * 0.55, cap)).path(rect(0.0, cap * 0.42, 260.0, cap * 0.42 + s.bar * 0.6)).spaced(SB, 260.0, SB)
        }
        "numero" => Draw::new(0.0).path(h_cap(s, 420.0, cap)).paths(ring(s, 240.0, cap * 0.4, cap)).shift_second_n(1, 460.0).spaced(SB, 700.0, SB),
        "dollar" => {
            let w = wd(480.0);
            Draw::new(0.0)
                .path(rect(0.0, 0.0, w, cap))
                .path(rect_cw(st, s.bar, w - st, cap - s.bar))
                .path(rect(w / 2.0 - st * 0.3, -100.0, w / 2.0 + st * 0.3, cap + 100.0))
                .spaced(SB, w, SB)
        }
        _ => generic(base, s),
    }
}

impl Draw {
    /// Moves the second shape right by `dx` (two copies of one shape side by side).
    fn shift_second(self, dx: f64) -> Self {
        self.shift_second_n(1, dx)
    }
    fn shift_second_n(mut self, from: usize, dx: f64) -> Self {
        for s in self.shapes.iter_mut().skip(from) {
            if let Shape::Path(p) = s {
                *p = transformed(p, 1.0, 1.0, dx, 0.0);
            }
        }
        self
    }
}

const DIGITS: [&str; 10] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

/// A generic placeholder: a rectangular ring of a size that depends on the script and case,
/// with stems for ascenders and descenders and a notch that differs from letter to letter.
fn generic(base: &str, s: &Style) -> Draw {
    let (st, cap, xh) = (s.stem, s.cap, s.xh);
    // A different width per name, so that spacing and kerning checks see different glyphs.
    let hash = base.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    let vary = f64::from(hash % 5) * 30.0;
    if let Some(i) = DIGITS.iter().position(|d| *d == base) {
        let w = (420.0 + vary) * s.wide;
        let mut d = Draw::new(0.0).paths(cap_ring(s, w, cap));
        d = d.path(marker(s, w / 2.0 - 15.0, cap * 0.6, 0.0));
        // A digit's number of small squares down its middle tells it apart.
        for k in 1..i.min(4) {
            d = d.path(marker(s, w / 2.0 - 15.0, cap * 0.6, k as f64));
        }
        return d.spaced(SB, w, SB);
    }
    let script = base.rsplit_once('-').map(|(_, sc)| sc).unwrap_or("");
    match script {
        "ar" => {
            let w = 520.0 + vary;
            let mut d = Draw::new(0.0).paths(vec![rect(0.0, 0.0, w, xh * 0.5), rect_cw(st, s.bar * 0.8, w - st, xh * 0.5 - s.bar * 0.8)]);
            if base.contains("alef") || base.starts_with("lam") {
                d = d.path(rect(w - st, xh * 0.5, w, s.asc));
            }
            if base.contains("beh") {
                d = d.path(rect(w / 2.0 - st / 2.0, -160.0, w / 2.0 + st / 2.0, -160.0 + st));
            }
            d.spaced(20.0, w, 20.0)
        }
        "deva" => {
            let w = 560.0 + vary;
            let head = 680.0;
            Draw::new(0.0)
                .path(rect(0.0, head - s.bar, w, head))
                .path(rect(w - st, 0.0, w, head - s.bar))
                .paths(vec![rect(0.0, 0.0, w - st, xh * 0.8), rect_cw(st, s.bar, w - st * 2.0, xh * 0.8 - s.bar)])
                .spaced(0.0, w, 0.0)
        }
        "hira" | "kata" => {
            let w = 840.0;
            let mut d = Draw::new(0.0).paths(vec![rect(0.0, -80.0, w, 760.0), rect_cw(st, -80.0 + s.bar, w - st, 760.0 - s.bar)]);
            d = d.path(rect(w / 2.0 - st / 2.0, 0.0, w / 2.0 + st / 2.0, 680.0));
            if script == "kata" {
                d = d.path(rect(80.0, 340.0, w - 80.0, 340.0 + s.bar));
            }
            d.spaced(80.0, w, 80.0)
        }
        _ => {
            let upper = is_upper(base);
            let w = if upper { 520.0 + vary } else { 400.0 + vary } * s.wide;
            let h = if upper { cap } else { xh };
            let mut d = Draw::new(0.0).paths(cap_ring(s, w, h));
            // A notch at the top whose position depends on the name.
            let nx = st + f64::from(hash % 3) * (w - 3.0 * st) / 3.0;
            d = d.path(rect(nx, h, nx + st * 0.8, h + 30.0));
            if !upper {
                let first = base.chars().next().unwrap_or('x');
                if "bhklt".contains(first) || base == "f" {
                    d = d.path(rect(0.0, h, st, s.asc));
                } else if first == 'd' {
                    d = d.path(rect(w - st, h, w, s.asc));
                }
                if first == 'p' {
                    d = d.path(rect(0.0, s.desc, st, 0.0));
                } else if "gqy".contains(first) {
                    d = d.path(rect(w - st, s.desc, w, 0.0));
                }
            }
            d.spaced(SB, w, SB)
        }
    }
}

fn number_after(sfx: &str, prefix: &str) -> Option<f64> {
    sfx.strip_prefix(prefix).and_then(|n| n.parse::<f64>().ok())
}

/// Turns a base drawing into the drawing of one of its suffixed variants.
fn with_suffix(d: Draw, base: &str, suffix: &str, s: &Style) -> Draw {
    if suffix.is_empty() {
        return d;
    }
    let upper = is_upper(base);
    let top = if upper { s.cap } else { s.xh };
    let marked = |d: Draw, n: f64| {
        let x = d.width - SB - 40.0;
        d.path(marker(s, x.max(0.0), top + 60.0, -n))
    };
    let mut out = match suffix.split('.').next().unwrap_or("") {
        "sc" | "c2sc" | "smcp" => d.scaled(1.0, s.xh * 1.1 / s.cap),
        "pc" => d.scaled(0.95, s.xh / s.cap),
        "sups" | "numr" => d.scaled(0.6, 0.6).shifted(0.0, s.cap * 0.4),
        "subs" | "sinf" => d.scaled(0.6, 0.6).shifted(0.0, -s.cap * 0.15),
        "dnom" => d.scaled(0.6, 0.6),
        "osf" | "tosf" => {
            let mut d = d.scaled(1.0, s.xh / s.cap);
            if suffix.starts_with("tosf") {
                d = tabular(d, 600.0);
            }
            d
        }
        "tf" => tabular(d, 600.0),
        "lf" => d,
        "case" => d.shifted(0.0, 100.0),
        "titl" => d.scaled(1.0, 1.1),
        "full" => centred(d, 1000.0),
        "half" => centred(d.scaled(0.8, 1.0), 500.0),
        "thirdwidth" => centred(d.scaled(0.5, 1.0), 333.0),
        "quartwidth" | "qartwidth" => centred(d.scaled(0.4, 1.0), 250.0),
        "hori" => d,
        "vert" => d.shifted(0.0, -60.0),
        "proportional" => d.scaled(0.8, 1.0),
        "swsh" => {
            let w = d.width;
            d.path(rect(w * 0.5, -60.0, w + 60.0, -60.0 + s.bar)).spaced(0.0, w + 60.0, 0.0)
        }
        "ornm" => {
            let c = 300.0;
            Draw::new(600.0).path(poly(&[(c, -20.0), (c + 260.0, s.cap / 2.0), (c, s.cap + 20.0), (c - 260.0, s.cap / 2.0)]))
        }
        sfx => {
            if let Some(n) = number_after(sfx, "ss").or_else(|| number_after(sfx, "cv")) {
                marked(d, n)
            } else {
                // Any other suffix (`.alt`, `.single`, `.init`, `.loclCAT`, `.dlig`, `.01`…):
                // one marker whose height depends on the suffix.
                let n = f64::from(sfx.bytes().fold(0u32, |h, b| h.wrapping_add(u32::from(b))) % 4);
                marked(d, n + 1.0)
            }
        }
    };
    // A two-part suffix (`.cv01.2`, `.ss01.alt`) gets a second marker.
    if suffix.contains('.') {
        let x = (out.width - SB - 40.0).max(0.0) - 50.0;
        out = out.path(marker(s, x.max(0.0), top + 60.0, -1.0));
    }
    out
}

/// `d` centred in an advance of `w`.
fn centred(d: Draw, w: f64) -> Draw {
    let dx = (w - d.width) / 2.0;
    let mut d = d.shifted(dx, 0.0);
    d.width = w;
    d
}

fn tabular(d: Draw, w: f64) -> Draw {
    centred(d, w)
}

/// Adds a glyph made of components side by side (a ligature or a sequence of parts), each
/// placed after the advance of the one before.
pub fn add_parts(font: &mut Font, styles: &[Style], name: &str, unicodes: &[u32], parts: &[&str]) {
    let draws: Vec<Vec<Draw>> = parts.iter().map(|p| drawings(font, p)).collect();
    add(font, styles, name, unicodes, |s| {
        let i = styles.iter().position(|t| t.id == s.id).unwrap_or(0);
        let mut d = Draw::new(0.0);
        let mut x = 0.0;
        for (p, pd) in parts.iter().zip(&draws) {
            d = d.comp(p, x, 0.0);
            x += pd.get(i).map_or(0.0, |dd| dd.width);
        }
        d.width = x;
        d
    });
}

/// A layer that is not a master layer: `id` of its own, named `name`, belonging to `master`.
pub fn extra_layer(d: Draw, master: &str, id: &str, name: &str, attrs: fontcraft_engine::model::LayerAttrs) -> Layer {
    let mut l = d.into_layer(master);
    l.id = id.into();
    l.info = fontcraft_engine::model::LayerInfoBox::new(fontcraft_engine::model::LayerInfo { name: Some(name.into()), attrs, ..Default::default() });
    l
}

/// Adds a layer to glyph `glyph`.
pub fn push_layer(font: &mut Font, glyph: &str, layer: Layer) {
    if let Ok(mut g) = font.glyph_mut(glyph) {
        g.layers.push(layer);
    }
}

/// Adds `top` and `bottom` anchors at the centre of every master layer of `names` (at the cap
/// height or x-height, by case).
pub fn base_anchors(font: &mut Font, styles: &[Style], names: &[&str]) {
    for n in names {
        let upper = n.chars().next().is_some_and(char::is_uppercase);
        if let Ok(mut g) = font.glyph_mut(n) {
            for (l, s) in g.layers.iter_mut().zip(styles) {
                let top = if upper { s.cap } else { s.xh };
                let cx = (l.width / 2.0).round();
                l.anchors.push(Anchor::new("top", cx, top));
                l.anchors.push(Anchor::new("bottom", cx, 0.0));
            }
        }
    }
}

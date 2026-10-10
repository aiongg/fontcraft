//! Strokes: the outline a stroked path is drawn and exported with
//! (`docs/spec/03a-edit-view-drawing.md` § 17).
//!
//! A stroke is a band along the path, `width` across for a horizontal pen and `height` for a
//! vertical one (an elliptical pen). `position` moves the band sideways: −1 puts it wholly on
//! the left of the path direction, 0 centres it, 1 puts it wholly on the right.
//!
//! The outline is built from pieces and united with the nonzero rule:
//!
//! | Piece | Shape |
//! |---|---|
//! | Each segment | The area between its two offset curves (kurbo's offset curves) |
//! | Each corner, outer side | Miter (bevel beyond [`MITER_LIMIT`]), bevel, or a round sector |
//! | Each corner, inner side, round full join | A round fillet of half the stroke width |
//! | Each end of an open path | Butt (none), round, square, round inset, axis-aligned |
//! | A closed path with `fill` | The path itself |
//!
//! The elliptical pen is a round pen in a space squeezed along its longer axis: the pieces are
//! built there and stretched back, so round joins and caps become elliptical arcs.

use std::f64::consts::PI;

use fontcraft_geom::{Affine, BezPath, CubicBez, Line, ParamCurve, PathEl, Point, Shape, Vec2};
use kurbo::{Arc, ParamCurveArclen, PathSeg};

use crate::{PathOpError, check, union};

/// Widest stroke, in font units, either way.
pub const MAX_STROKE_WIDTH: f64 = 10_000.0;
/// Largest ratio between a stroke's width and height. Beyond it the squeezed pen space loses
/// the precision the boolean engine needs.
pub const MAX_PEN_RATIO: f64 = 100.0;
/// Most segments one stroked path may have. Each segment gives a piece of four or more
/// segments, so the pieces stay within [`crate::MAX_SEGMENTS`].
pub const MAX_STROKE_SEGMENTS: usize = 10_000;
/// A miter join longer than this many half stroke widths becomes a bevel (the SVG default).
pub const MITER_LIMIT: f64 = 4.0;
/// Largest distance, in font units, between an offset curve or arc and the true one.
pub const STROKE_TOLERANCE: f64 = 0.05;

/// Accuracy of arc-length measures when an end is trimmed.
const ARCLEN_ACCURACY: f64 = 1e-3;
/// A corner whose wedge would be narrower than this, in units, is no corner.
const JOIN_GAP: f64 = 1e-3;
/// An axis-aligned cut more than this many half stroke widths from the butt line falls back to
/// a butt end: the path ends nearly along the forced axis.
const MAX_CAP_SLANT: f64 = 10.0;
/// A round full join gets no inner fillet when the corner is sharper than this (the tangent of
/// half the turn): the fillet would reach far along the segments.
const MAX_FILLET_TAN: f64 = 4.0;
/// Pieces with less area than this are dropped.
const MIN_PIECE_AREA: f64 = 1e-9;

/// How two segments of a stroke meet at a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Join {
    /// The outer edges extended until they meet, up to [`MITER_LIMIT`].
    #[default]
    Miter,
    /// A round outer edge.
    Round,
    /// The outer edges joined by a straight line.
    Bevel,
    /// Round on both sides: the outer edge as with [`Join::Round`], and the inner corner filled
    /// with an arc of half the stroke width.
    RoundFull,
}

impl Join {
    pub const ALL: [Join; 4] = [Join::Miter, Join::Round, Join::Bevel, Join::RoundFull];

    /// The join a source stores as a number (`lineJoin`): 0 miter, 1 round, 2 bevel, 3 round full.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|j| j.code() == code)
    }

    pub fn code(self) -> u8 {
        match self {
            Join::Miter => 0,
            Join::Round => 1,
            Join::Bevel => 2,
            Join::RoundFull => 3,
        }
    }

    /// The name commands use: `miter`, `round`, `bevel`, `roundFull`.
    pub fn name(self) -> &'static str {
        match self {
            Join::Miter => "miter",
            Join::Round => "round",
            Join::Bevel => "bevel",
            Join::RoundFull => "roundFull",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|j| j.name() == name)
    }
}

/// The end of an open stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cap {
    /// Cut square at the end node.
    #[default]
    Butt,
    /// A half circle beyond the end node.
    Round,
    /// Extended by half the stroke width beyond the end node.
    Square,
    /// A half circle that ends at the end node: the band is shortened by half the stroke width.
    RoundInset,
    /// Cut along the X or Y axis through the end node, whichever is nearer the butt cut, unless
    /// [`CapAxis`] forces one.
    AxisAligned,
}

impl Cap {
    pub const ALL: [Cap; 5] = [Cap::Butt, Cap::Round, Cap::Square, Cap::RoundInset, Cap::AxisAligned];

    /// The cap a source stores as a number (`lineCapStart`, `lineCapEnd`): 0 butt, 1 round,
    /// 2 square, 3 round inset, 4 axis-aligned.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.code() == code)
    }

    pub fn code(self) -> u8 {
        match self {
            Cap::Butt => 0,
            Cap::Round => 1,
            Cap::Square => 2,
            Cap::RoundInset => 3,
            Cap::AxisAligned => 4,
        }
    }

    /// The name commands use: `butt`, `round`, `square`, `roundInset`, `axisAligned`.
    pub fn name(self) -> &'static str {
        match self {
            Cap::Butt => "butt",
            Cap::Round => "round",
            Cap::Square => "square",
            Cap::RoundInset => "roundInset",
            Cap::AxisAligned => "axisAligned",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// The axis an axis-aligned cap cuts along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CapAxis {
    /// The axis nearer the butt cut at that end.
    #[default]
    Auto,
    /// A horizontal cut.
    X,
    /// A vertical cut.
    Y,
}

impl CapAxis {
    pub const ALL: [CapAxis; 3] = [CapAxis::Auto, CapAxis::X, CapAxis::Y];

    /// The name commands and files use: `auto`, `x`, `y`.
    pub fn name(self) -> &'static str {
        match self {
            CapAxis::Auto => "auto",
            CapAxis::X => "x",
            CapAxis::Y => "y",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// How a path is stroked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    /// The pen's horizontal size, in font units.
    pub width: f64,
    /// The pen's vertical size, in font units.
    pub height: f64,
    /// −1 wholly left of the path direction, 0 centred, 1 wholly right.
    pub position: f64,
    pub join: Join,
    pub cap_start: Cap,
    pub cap_end: Cap,
    pub cap_axis: CapAxis,
    /// A closed path is also filled.
    pub fill: bool,
}

impl StrokeStyle {
    /// A centred stroke with a round pen, miter joins and butt caps.
    pub fn new(width: f64) -> Self {
        Self {
            width,
            height: width,
            position: 0.0,
            join: Join::Miter,
            cap_start: Cap::Butt,
            cap_end: Cap::Butt,
            cap_axis: CapAxis::Auto,
            fill: false,
        }
    }

    /// Whether the numbers are ones a stroke can be built from: sizes above 0 and at most
    /// [`MAX_STROKE_WIDTH`], within [`MAX_PEN_RATIO`] of each other, a position from −1 to 1.
    pub fn check(&self) -> Result<(), PathOpError> {
        if ![self.width, self.height, self.position].iter().all(|v| v.is_finite()) {
            return Err(PathOpError::NotFinite);
        }
        for (what, v) in [("width", self.width), ("height", self.height)] {
            if !(v > 0.0 && v <= MAX_STROKE_WIDTH) {
                return Err(PathOpError::Invalid(format!("the stroke {what} must be more than 0 and at most {MAX_STROKE_WIDTH}, got {v}")));
            }
        }
        if self.width.max(self.height) > MAX_PEN_RATIO * self.width.min(self.height) {
            return Err(PathOpError::Invalid(format!("the stroke width and height may differ by a factor of at most {MAX_PEN_RATIO}")));
        }
        if self.position.abs() > 1.0 {
            return Err(PathOpError::Invalid(format!("the stroke position must be from −100 % to 100 %, got {} %", self.position * 100.0)));
        }
        Ok(())
    }
}

/// The stroke position (−1 left, 0 centred, 1 right) of the value a source of `.glyphs` format
/// version `glyphs_version` stores. Versions before 4 store 0 for right (outside), 0.5 for
/// centred and 1 for left (inside); version 4 stores the position itself.
pub fn position_from_stored(stored: f64, glyphs_version: u32) -> f64 {
    if glyphs_version < 4 { 1.0 - 2.0 * stored } else { stored }
}

/// The value a source of `glyphs_version` stores for `position` (see [`position_from_stored`]).
pub fn position_to_stored(position: f64, glyphs_version: u32) -> f64 {
    if glyphs_version < 4 { (1.0 - position) / 2.0 } else { position }
}

/// The outline of `path` stroked with `style`: closed contours, outer ones counter-clockwise,
/// without overlaps. Every subpath is stroked; open ones get caps, closed ones do not. A
/// subpath without length gives nothing.
pub fn stroke_outline(path: &BezPath, style: &StrokeStyle) -> Result<Vec<BezPath>, PathOpError> {
    style.check()?;
    let subpaths = subpaths(path)?;
    let pen = Pen::new(style);
    let mut pieces = Vec::new();
    for sub in &subpaths {
        let segs: Vec<PathSeg> = sub.segs.iter().map(|s| straighten(pen.into * *s, pen.tolerance)).filter(|s| !degenerate(s)).collect();
        let mut built = Vec::new();
        if sub.closed {
            pen.closed_pieces(&segs, style, &mut built);
        } else {
            pen.open_pieces(segs, style, &mut built);
        }
        pieces.extend(built.into_iter().map(|p| pen.back * p));
        if sub.closed && style.fill {
            pieces.push(path_of(sub.start, &sub.segs, true));
        }
    }
    let pieces: Vec<BezPath> = pieces.into_iter().filter_map(oriented).collect();
    if pieces.is_empty() {
        return Ok(Vec::new());
    }
    union(&pieces)
}

/// One subpath as segments.
struct Subpath {
    start: Point,
    segs: Vec<PathSeg>,
    closed: bool,
}

/// The subpaths of `path`, after checking every coordinate and the segment count. A closed
/// subpath gets its closing line as a segment.
fn subpaths(path: &BezPath) -> Result<Vec<Subpath>, PathOpError> {
    let mut out = Vec::new();
    let mut current: Option<Subpath> = None;
    let mut last = Point::ZERO;
    let mut count = 0usize;
    for el in path.elements() {
        let points: &[Point] = match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => std::slice::from_ref(p),
            PathEl::QuadTo(a, b) => &[*a, *b],
            PathEl::CurveTo(a, b, c) => &[*a, *b, *c],
            PathEl::ClosePath => &[],
        };
        for p in points {
            check(p.x)?;
            check(p.y)?;
        }
        let seg = match *el {
            PathEl::MoveTo(p) => {
                out.extend(current.take());
                current = Some(Subpath { start: p, segs: Vec::new(), closed: false });
                last = p;
                continue;
            }
            PathEl::LineTo(p) => PathSeg::Line(Line::new(last, p)),
            PathEl::QuadTo(a, p) => PathSeg::Quad(fontcraft_geom::QuadBez::new(last, a, p)),
            PathEl::CurveTo(a, b, p) => PathSeg::Cubic(CubicBez::new(last, a, b, p)),
            PathEl::ClosePath => {
                if let Some(mut sub) = current.take() {
                    if last != sub.start {
                        sub.segs.push(PathSeg::Line(Line::new(last, sub.start)));
                    }
                    sub.closed = true;
                    last = sub.start;
                    out.push(sub);
                }
                continue;
            }
        };
        count += 1;
        if count > MAX_STROKE_SEGMENTS {
            return Err(PathOpError::TooManySegments(count));
        }
        last = seg.end();
        if let Some(sub) = current.as_mut() {
            sub.segs.push(seg);
        }
    }
    out.extend(current);
    Ok(out)
}

fn path_of(start: Point, segs: &[PathSeg], closed: bool) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(start);
    for s in segs {
        p.push(s.as_path_el());
    }
    if closed {
        p.close_path();
    }
    p
}

/// A segment whose points are all one point draws nothing and has no direction.
fn degenerate(seg: &PathSeg) -> bool {
    match *seg {
        PathSeg::Line(l) => l.p0 == l.p1,
        PathSeg::Quad(q) => q.p0 == q.p1 && q.p0 == q.p2,
        PathSeg::Cubic(c) => c.p0 == c.p1 && c.p0 == c.p2 && c.p0 == c.p3,
    }
}

/// `seg` as a cubic, or as a line when its control points lie within `tolerance` of its chord:
/// kurbo's offset curves need a cubic whose control points are not all on one line.
fn straighten(seg: PathSeg, tolerance: f64) -> PathSeg {
    let c = match seg {
        PathSeg::Line(_) => return seg,
        PathSeg::Quad(q) => q.raise(),
        PathSeg::Cubic(c) => c,
    };
    let chord = c.p3 - c.p0;
    let len = chord.hypot();
    if len <= f64::EPSILON {
        // A loop that ends where it starts: straight only if it has no width at all.
        let flat = (c.p1 - c.p0).cross(c.p2 - c.p0).abs() <= tolerance * tolerance;
        return if flat { PathSeg::Line(Line::new(c.p0, c.p3)) } else { PathSeg::Cubic(c) };
    }
    let off = |p: Point| (p - c.p0).cross(chord).abs() / len;
    if off(c.p1) <= tolerance && off(c.p2) <= tolerance { PathSeg::Line(Line::new(c.p0, c.p3)) } else { PathSeg::Cubic(c) }
}

/// The directions a segment starts and ends in, read from the nearest distinct control point.
fn tangents(seg: &PathSeg) -> (Vec2, Vec2) {
    const EPS: f64 = 1e-12;
    let first = |ds: &[Vec2]| ds.iter().copied().find(|d| d.hypot2() > EPS).unwrap_or(Vec2::ZERO);
    match *seg {
        PathSeg::Line(l) => (l.p1 - l.p0, l.p1 - l.p0),
        PathSeg::Quad(q) => (first(&[q.p1 - q.p0, q.p2 - q.p0]), first(&[q.p2 - q.p1, q.p2 - q.p0])),
        PathSeg::Cubic(c) => (first(&[c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0]), first(&[c.p3 - c.p2, c.p3 - c.p1, c.p3 - c.p0])),
    }
}

fn unit(v: Vec2) -> Option<Vec2> {
    let h = v.hypot();
    (h > 1e-12 && h.is_finite()).then_some(v / h)
}

/// The unit normal on the right of direction `t` (y up).
fn right(t: Vec2) -> Vec2 {
    Vec2::new(t.y, -t.x)
}

/// `p` drawn counter-clockwise, or `None` when it has no area or a point is not finite.
fn oriented(p: BezPath) -> Option<BezPath> {
    let area = p.area();
    if !p.is_finite() || !area.is_finite() || area.abs() < MIN_PIECE_AREA {
        return None;
    }
    Some(if area < 0.0 { p.reverse_subpaths() } else { p })
}

/// A sector of the circle around `center`: from `center + r · from`, turning by `sweep` radians
/// (counter-clockwise when positive).
fn sector(center: Point, r: f64, from: Vec2, sweep: f64, tolerance: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(center);
    p.line_to(center + r * from);
    arc_to(&mut p, center, r, from, sweep, tolerance);
    p.close_path();
    p
}

/// Appends the arc around `center` that starts at `center + r · from` (the current point) and
/// turns by `sweep` radians.
fn arc_to(p: &mut BezPath, center: Point, r: f64, from: Vec2, sweep: f64, tolerance: f64) {
    Arc::new(center, (r, r), from.atan2(), sweep, 0.0).to_cubic_beziers(tolerance, |a, b, c| p.curve_to(a, b, c));
}

fn polygon(points: &[Point]) -> BezPath {
    let mut p = BezPath::new();
    for (i, q) in points.iter().enumerate() {
        if i == 0 {
            p.move_to(*q);
        } else {
            p.line_to(*q);
        }
    }
    p.close_path();
    p
}

/// Where an axis-aligned cap cuts: the line through `m` across `normal` (a unit axis), and how
/// far the path must be shortened so that the band ends inside it.
#[derive(Clone, Copy)]
struct Cut {
    m: Point,
    normal: Vec2,
    trim: f64,
}

/// One end of an open subpath: the end point, the unit direction of travel there, and which
/// way is out of the path.
#[derive(Clone, Copy)]
struct End {
    p: Point,
    t: Vec2,
    out: Vec2,
}

/// The round pen in squeezed space, the band it draws and the way back to font units.
struct Pen {
    into: Affine,
    back: Affine,
    /// The band's edges, as signed distances to the right of the path.
    d1: f64,
    d2: f64,
    tolerance: f64,
}

impl Pen {
    fn new(style: &StrokeStyle) -> Self {
        let (into, back, diameter, squeeze) = if style.width >= style.height {
            let s = style.height / style.width;
            (Affine::scale_non_uniform(s, 1.0), Affine::scale_non_uniform(1.0 / s, 1.0), style.height, s)
        } else {
            let s = style.width / style.height;
            (Affine::scale_non_uniform(1.0, s), Affine::scale_non_uniform(1.0, 1.0 / s), style.width, s)
        };
        let half = diameter / 2.0;
        let centre = style.position * half;
        Self { into, back, d1: centre - half, d2: centre + half, tolerance: STROKE_TOLERANCE * squeeze }
    }

    fn half(&self) -> f64 {
        (self.d2 - self.d1) / 2.0
    }

    /// The offset curve of `seg` at `d` units to its right, starting with a move.
    fn offset(&self, seg: &PathSeg, d: f64) -> BezPath {
        let mut out = BezPath::new();
        match *seg {
            PathSeg::Line(l) => {
                let n = unit(l.p1 - l.p0).map_or(Vec2::ZERO, right);
                out.move_to(l.p0 + d * n);
                out.line_to(l.p1 + d * n);
            }
            PathSeg::Quad(_) | PathSeg::Cubic(_) => {
                let c = match *seg {
                    PathSeg::Quad(q) => q.raise(),
                    PathSeg::Cubic(c) => c,
                    PathSeg::Line(l) => CubicBez::new(l.p0, l.p0, l.p1, l.p1),
                };
                if d == 0.0 {
                    out.move_to(c.p0);
                    out.curve_to(c.p1, c.p2, c.p3);
                } else {
                    // kurbo offsets toward the left normal for a positive distance.
                    kurbo::offset::offset_cubic(c, -d, self.tolerance, &mut out);
                }
            }
        }
        out
    }

    /// The area between the offsets of `seg` at the band's two edges.
    fn strip(&self, seg: &PathSeg) -> BezPath {
        let mut p = self.offset(seg, self.d1);
        for el in self.offset(seg, self.d2).reverse_subpaths().elements() {
            match *el {
                PathEl::MoveTo(q) => p.line_to(q),
                PathEl::ClosePath => {}
                other => p.push(other),
            }
        }
        p.close_path();
        p
    }

    fn closed_pieces(&self, segs: &[PathSeg], style: &StrokeStyle, out: &mut Vec<BezPath>) {
        for (i, seg) in segs.iter().enumerate() {
            out.push(self.strip(seg));
            let next = segs.get(i + 1).or_else(|| segs.first());
            if let Some(next) = next {
                self.join(seg, next, style.join, out);
            }
        }
    }

    fn open_pieces(&self, mut segs: Vec<PathSeg>, style: &StrokeStyle, out: &mut Vec<BezPath>) {
        let (Some(first), Some(last)) = (segs.first().copied(), segs.last().copied()) else { return };
        let (Some(t0), Some(t1)) = (unit(tangents(&first).0), unit(tangents(&last).1)) else { return };
        let start = End { p: first.start(), t: t0, out: -t0 };
        let end = End { p: last.end(), t: t1, out: t1 };
        let start_cut = (style.cap_start == Cap::AxisAligned).then(|| self.cut(start, style.cap_axis)).flatten();
        let end_cut = (style.cap_end == Cap::AxisAligned).then(|| self.cut(end, style.cap_axis)).flatten();
        let trim = |cap: Cap, cut: Option<Cut>| match cap {
            Cap::RoundInset => self.half(),
            Cap::AxisAligned => cut.map_or(0.0, |c| c.trim),
            Cap::Butt | Cap::Round | Cap::Square => 0.0,
        };
        let (mut trim_start, mut trim_end) = (trim(style.cap_start, start_cut), trim(style.cap_end, end_cut));
        let length: f64 = segs.iter().map(|s| s.arclen(ARCLEN_ACCURACY)).sum();
        if trim_start + trim_end >= length {
            // A path too short for its caps keeps a sliver, shortened from both ends alike.
            let k = 0.999 * length / (trim_start + trim_end);
            trim_start *= k;
            trim_end *= k;
        }
        trim_front(&mut segs, trim_start);
        trim_back(&mut segs, trim_end);
        for (i, seg) in segs.iter().enumerate() {
            out.push(self.strip(seg));
            if let Some(next) = segs.get(i + 1) {
                self.join(seg, next, style.join, out);
            }
        }
        let (Some(first), Some(last)) = (segs.first(), segs.last()) else { return };
        if let Some(t) = unit(tangents(first).0) {
            self.cap(End { p: first.start(), t, out: -t }, style.cap_start, start_cut, out);
        }
        if let Some(t) = unit(tangents(last).1) {
            self.cap(End { p: last.end(), t, out: t }, style.cap_end, end_cut, out);
        }
    }

    /// The pieces where `a` meets `b`: the outer wedge, and for a round full join the inner
    /// fillet.
    fn join(&self, a: &PathSeg, b: &PathSeg, join: Join, out: &mut Vec<BezPath>) {
        let (Some(t0), Some(t1)) = (unit(tangents(a).1), unit(tangents(b).0)) else { return };
        let p = a.end();
        let turn = t0.cross(t1).atan2(t0.dot(t1));
        if turn.abs() * self.d2.max(-self.d1) <= JOIN_GAP {
            return;
        }
        let tol = self.tolerance;
        if PI - turn.abs() <= 1e-9 {
            // The path turns back on itself: both sides are outer, and only a round join closes
            // the end.
            if matches!(join, Join::Round | Join::RoundFull) {
                out.push(sector(p, self.d2, right(t0), PI, tol));
                out.push(sector(p, -self.d1, -right(t0), -PI, tol));
            }
            return;
        }
        // A left turn opens a gap on the right of the path, a right turn on the left.
        let (r_out, n0, n1, r_in, m0, m1) = if turn > 0.0 {
            (self.d2, right(t0), right(t1), -self.d1, -right(t0), -right(t1))
        } else {
            (-self.d1, -right(t0), -right(t1), self.d2, right(t0), right(t1))
        };
        if r_out > 0.0 {
            let a0 = p + r_out * n0;
            let a1 = p + r_out * n1;
            match join {
                Join::Bevel => out.push(polygon(&[p, a0, a1])),
                Join::Miter if 1.0 / (turn / 2.0).cos() <= MITER_LIMIT => {
                    let tip = p + r_out * (n0 + n1) / (1.0 + turn.cos());
                    out.push(polygon(&[p, a0, tip, a1]));
                }
                Join::Miter => out.push(polygon(&[p, a0, a1])),
                Join::Round | Join::RoundFull => out.push(sector(p, r_out, n0, turn, tol)),
            }
        }
        if join == Join::RoundFull {
            out.extend(self.fillet(p, t0, t1, turn, r_in, m0, m1));
        }
    }

    /// The fillet that rounds the inner corner of a turn by `turn` radians at `p`: the inner
    /// edges are `r_in` from the path on the side of the normals `m0` and `m1`.
    #[allow(clippy::too_many_arguments)]
    fn fillet(&self, p: Point, t0: Vec2, t1: Vec2, turn: f64, r_in: f64, m0: Vec2, m1: Vec2) -> Option<BezPath> {
        let rho = self.half();
        let tan = (turn.abs() / 2.0).tan();
        if !(rho > 0.0 && tan.is_finite() && tan <= MAX_FILLET_TAN) {
            return None;
        }
        let corner = p + r_in * (m0 + m1) / (1.0 + turn.cos());
        let k = rho * tan;
        let from = corner - k * t0;
        // The fillet's centre is on the side the path turns toward.
        let toward = if turn > 0.0 { -right(t0) } else { right(t0) };
        let centre = from + rho * toward;
        let mut path = BezPath::new();
        path.move_to(corner);
        path.line_to(from);
        arc_to(&mut path, centre, rho, -toward, turn, self.tolerance);
        path.close_path();
        Some(path)
    }

    /// The cut of an axis-aligned cap at `end`, or `None` (a butt end) when the path ends along
    /// the cut's axis.
    fn cut(&self, end: End, axis: CapAxis) -> Option<Cut> {
        let n = right(end.t);
        let horizontal = match axis {
            CapAxis::X => true,
            CapAxis::Y => false,
            CapAxis::Auto => n.x.abs() >= n.y.abs(),
        };
        let (along, normal) = if horizontal { (Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)) } else { (Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0)) };
        let cos = n.dot(along).abs();
        if cos <= 1e-9 {
            return None;
        }
        let slant = n.dot(normal).abs() / cos;
        if slant > MAX_CAP_SLANT {
            return None;
        }
        let m = end.p + (self.d1 + self.d2) / 2.0 * n;
        Some(Cut { m, normal, trim: self.half() * slant })
    }

    fn cap(&self, end: End, cap: Cap, cut: Option<Cut>, out: &mut Vec<BezPath>) {
        let n = right(end.t);
        let a = end.p + self.d1 * n;
        let b = end.p + self.d2 * n;
        let r = self.half();
        match cap {
            Cap::Butt => {}
            Cap::Square => out.push(polygon(&[a, b, b + r * end.out, a + r * end.out])),
            Cap::Round | Cap::RoundInset => {
                let m = a.midpoint(b);
                let from = -n;
                let sweep = if from.cross(end.out) > 0.0 { PI } else { -PI };
                let mut p = BezPath::new();
                p.move_to(a);
                arc_to(&mut p, m, r, from, sweep, self.tolerance);
                p.close_path();
                out.push(p);
            }
            Cap::AxisAligned => {
                let Some(cut) = cut else { return };
                let across = end.out.dot(cut.normal);
                if across.abs() <= 1e-9 {
                    return;
                }
                let reach = |q: Point| q + ((cut.m - q).dot(cut.normal) / across).max(0.0) * end.out;
                out.push(polygon(&[a, b, reach(b), reach(a)]));
            }
        }
    }
}

/// Shortens `segs` by `len` units of arc length from the start.
fn trim_front(segs: &mut Vec<PathSeg>, mut len: f64) {
    while len > 0.0 {
        let Some(first) = segs.first().copied() else { return };
        let l = first.arclen(ARCLEN_ACCURACY);
        if len >= l {
            segs.remove(0);
            len -= l;
            continue;
        }
        let t = first.inv_arclen(len, ARCLEN_ACCURACY);
        if let Some(f) = segs.first_mut() {
            *f = first.subsegment(t..1.0);
        }
        return;
    }
}

/// Shortens `segs` by `len` units of arc length from the end.
fn trim_back(segs: &mut Vec<PathSeg>, mut len: f64) {
    while len > 0.0 {
        let Some(last) = segs.last().copied() else { return };
        let l = last.arclen(ARCLEN_ACCURACY);
        if len >= l {
            segs.pop();
            len -= l;
            continue;
        }
        let t = last.inv_arclen(l - len, ARCLEN_ACCURACY);
        if let Some(e) = segs.last_mut() {
            *e = last.subsegment(0.0..t);
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn open(svg: &str) -> BezPath {
        BezPath::from_svg(svg).unwrap()
    }

    fn area(out: &[BezPath]) -> f64 {
        out.iter().map(Shape::area).sum()
    }

    fn covers(out: &[BezPath], x: f64, y: f64) -> bool {
        out.iter().map(|p| p.winding(Point::new(x, y))).sum::<i32>() != 0
    }

    fn style(width: f64, join: Join, cap: Cap) -> StrokeStyle {
        StrokeStyle { join, cap_start: cap, cap_end: cap, ..StrokeStyle::new(width) }
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// A 100-unit square, counter-clockwise.
    fn square() -> BezPath {
        open("M0,0 L100,0 L100,100 L0,100 Z")
    }

    #[test]
    fn caps_on_an_open_line() {
        let line = open("M0,0 L100,0");
        let q = PI * 100.0; // a disc of radius 10
        let cases = [(Cap::Butt, 2000.0), (Cap::Square, 2400.0), (Cap::Round, 2000.0 + q), (Cap::RoundInset, 1600.0 + q)];
        for (cap, want) in cases {
            let out = stroke_outline(&line, &style(20.0, Join::Miter, cap)).unwrap();
            assert!(near(area(&out), want, 1.0), "{cap:?}: {}", area(&out));
        }
        // Round inset ends at the end nodes; round reaches beyond them.
        let inset = stroke_outline(&line, &style(20.0, Join::Miter, Cap::RoundInset)).unwrap();
        assert!(covers(&inset, 1.0, 0.0) && !covers(&inset, -1.0, 0.0) && !covers(&inset, 101.0, 0.0));
        let round = stroke_outline(&line, &style(20.0, Join::Miter, Cap::Round)).unwrap();
        assert!(covers(&round, -9.0, 0.0) && covers(&round, 109.0, 0.0));
    }

    #[test]
    fn an_axis_aligned_cap_cuts_along_the_nearer_axis() {
        // Rising at 1 in 2: the butt cut is nearer vertical, so both ends are cut vertically
        // through the end nodes and the band is a parallelogram 100 units wide.
        let line = open("M0,0 L100,50");
        let out = stroke_outline(&line, &style(20.0, Join::Miter, Cap::AxisAligned)).unwrap();
        let want = 100.0 * 20.0 * 5f64.sqrt() / 2.0;
        assert!(near(area(&out), want, 1.0), "{}", area(&out));
        let bbox = out.iter().map(Shape::bounding_box).reduce(|a, b| a.union(b)).unwrap();
        assert!(near(bbox.x0, 0.0, 0.01) && near(bbox.x1, 100.0, 0.01), "{bbox:?}");
        // Forcing the X axis cuts horizontally instead: the band now ends at y = 0 and y = 50.
        let forced = StrokeStyle { cap_axis: CapAxis::X, ..style(20.0, Join::Miter, Cap::AxisAligned) };
        let out = stroke_outline(&line, &forced).unwrap();
        let bbox = out.iter().map(Shape::bounding_box).reduce(|a, b| a.union(b)).unwrap();
        assert!(near(bbox.y0, 0.0, 0.01) && near(bbox.y1, 50.0, 0.01), "{bbox:?}");
        // A horizontal line has a vertical butt cut already: axis-aligned is butt.
        let flat = stroke_outline(&open("M0,0 L100,0"), &style(20.0, Join::Miter, Cap::AxisAligned)).unwrap();
        assert!(near(area(&flat), 2000.0, 0.5));
    }

    #[test]
    fn joins_on_an_open_corner() {
        // East then north: the corner's outer side is the lower right.
        let corner = open("M0,0 L100,0 L100,100");
        let quarter = PI * 100.0 / 4.0;
        let cases = [(Join::Miter, 4000.0), (Join::Bevel, 3950.0), (Join::Round, 3900.0 + quarter), (Join::RoundFull, 4000.0)];
        for (join, want) in cases {
            let out = stroke_outline(&corner, &style(20.0, join, Cap::Butt)).unwrap();
            assert!(near(area(&out), want, 1.0), "{join:?}: {}", area(&out));
        }
        let miter = stroke_outline(&corner, &style(20.0, Join::Miter, Cap::Butt)).unwrap();
        assert!(covers(&miter, 109.0, -9.0));
        let round = stroke_outline(&corner, &style(20.0, Join::Round, Cap::Butt)).unwrap();
        assert!(!covers(&round, 109.0, -9.0) && covers(&round, 105.0, -5.0));
        // Round full also rounds the inner corner: a fillet of radius 10 around (80, 20).
        let full = stroke_outline(&corner, &style(20.0, Join::RoundFull, Cap::Butt)).unwrap();
        assert!(covers(&full, 89.0, 11.0) && !covers(&round, 89.0, 11.0));
    }

    #[test]
    fn joins_and_caps_on_a_closed_path() {
        let quarter = PI * 100.0 / 4.0;
        let cases = [(Join::Miter, 8000.0), (Join::Bevel, 7800.0), (Join::Round, 8000.0 - 4.0 * (100.0 - quarter)), (Join::RoundFull, 8000.0)];
        for (join, want) in cases {
            // Caps do not apply to a closed path.
            for cap in Cap::ALL {
                let out = stroke_outline(&square(), &style(20.0, join, cap)).unwrap();
                assert!(near(area(&out), want, 1.0), "{join:?} {cap:?}: {}", area(&out));
                assert!(!covers(&out, 50.0, 50.0), "a closed stroke is a ring");
            }
        }
        let full = stroke_outline(&square(), &style(20.0, Join::RoundFull, Cap::Butt)).unwrap();
        assert!(covers(&full, 11.0, 11.0), "the inner corners are round");
        let round = stroke_outline(&square(), &style(20.0, Join::Round, Cap::Butt)).unwrap();
        assert!(!covers(&round, 11.0, 11.0));
    }

    #[test]
    fn position_moves_the_band_to_one_side() {
        // The square runs counter-clockwise, so its right side is the outside.
        let at = |position: f64| StrokeStyle { position, ..StrokeStyle::new(20.0) };
        let outside = stroke_outline(&square(), &at(1.0)).unwrap();
        assert!(near(area(&outside), 140.0 * 140.0 - 10_000.0, 1.0), "{}", area(&outside));
        assert!(covers(&outside, -15.0, 50.0) && !covers(&outside, 5.0, 50.0));
        let inside = stroke_outline(&square(), &at(-1.0)).unwrap();
        assert!(near(area(&inside), 10_000.0 - 60.0 * 60.0, 1.0), "{}", area(&inside));
        // An open line moves to its right (down) as well.
        let line = stroke_outline(&open("M0,0 L100,0"), &at(0.5)).unwrap();
        assert!(covers(&line, 50.0, -14.0) && !covers(&line, 50.0, 6.0));
        assert!(near(area(&line), 2000.0, 0.5));
    }

    #[test]
    fn height_and_fill() {
        // Width is the pen's horizontal size: a horizontal line is as thick as the height.
        let pen = StrokeStyle { height: 20.0, ..StrokeStyle::new(40.0) };
        assert!(near(area(&stroke_outline(&open("M0,0 L100,0"), &pen).unwrap()), 2000.0, 0.5));
        assert!(near(area(&stroke_outline(&open("M0,0 L0,100"), &pen).unwrap()), 4000.0, 0.5));
        let filled = StrokeStyle { fill: true, ..StrokeStyle::new(20.0) };
        let out = stroke_outline(&square(), &filled).unwrap();
        assert!(near(area(&out), 120.0 * 120.0, 1.0) && covers(&out, 50.0, 50.0));
    }

    #[test]
    fn curves_are_offset() {
        // A half circle of radius 50 drawn as two cubics, stroked 20 wide: half an annulus.
        let k = 50.0 * 0.552_284_75;
        let arc = open(&format!("M0,0 C0,{k} {},50 50,50 C{},50 100,{k} 100,0", 50.0 - k, 50.0 + k));
        let out = stroke_outline(&arc, &StrokeStyle::new(20.0)).unwrap();
        let want = PI * (60.0 * 60.0 - 40.0 * 40.0) / 2.0;
        assert!(near(area(&out), want, want * 0.01), "{} vs {want}", area(&out));
    }

    #[test]
    fn bad_styles_and_paths_are_errors() {
        let line = open("M0,0 L100,0");
        for bad in [
            StrokeStyle::new(0.0),
            StrokeStyle::new(f64::NAN),
            StrokeStyle::new(MAX_STROKE_WIDTH * 2.0),
            StrokeStyle { height: 0.01, ..StrokeStyle::new(100.0) },
            StrokeStyle { position: 1.5, ..StrokeStyle::new(10.0) },
        ] {
            assert!(stroke_outline(&line, &bad).is_err(), "{bad:?}");
        }
        let far = open("M0,0 L1e9,0");
        assert!(stroke_outline(&far, &StrokeStyle::new(10.0)).is_err());
        // A path without length draws nothing.
        assert!(stroke_outline(&open("M5,5 L5,5"), &StrokeStyle::new(10.0)).unwrap().is_empty());
    }

    #[test]
    fn stored_positions_follow_the_format_version() {
        for (stored, v, want) in [(0.0, 3, 1.0), (0.5, 3, 0.0), (1.0, 3, -1.0), (-1.0, 4, -1.0), (0.25, 4, 0.25)] {
            assert_eq!(position_from_stored(stored, v), want);
            assert_eq!(position_to_stored(want, v), stored);
        }
        for j in Join::ALL {
            assert_eq!(Join::from_code(j.code()), Some(j));
            assert_eq!(Join::from_name(j.name()), Some(j));
        }
        for c in Cap::ALL {
            assert_eq!(Cap::from_code(c.code()), Some(c));
            assert_eq!(Cap::from_name(c.name()), Some(c));
        }
    }
}

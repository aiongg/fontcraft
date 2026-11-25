//! FontCraft path operations (L2), on plain Bézier paths and on contours as node lists.
//!
//! The boolean engine is linesweeper. It panics on a NaN coordinate, gives wrong results for
//! infinite ones, and loses small contours next to huge ones (its tolerance grows with the
//! largest coordinate), so every path is checked by [`prepare`] before linesweeper sees it.
//!
//! | Operation | Function |
//! |---|---|
//! | Union, difference, intersection of paths | [`union`], [`difference`], [`intersection`], [`binary`] |
//! | Booleans and Remove Overlap on contours, results as nodes | [`boolean`], [`remove_overlap`] |
//! | Correct Path Direction | [`correct_direction`] |
//! | Add Extremes, Force Extremes | [`add_extremes`] |
//! | Add Inflections | [`add_inflections`] |
//! | Tidy up Paths | [`tidy_up`] |
//! | Open Corner, Reconnect Nodes | [`open_corners`], [`reconnect_nodes`] |
//! | Cut with Line (the Knife) | [`cut_with_line`] |
//! | Coverage comparison | [`coverage_difference`] |
//! | Stroke outlines | [`stroke_outline`] |
//! | A layer's shapes in order: strokes expanded, masks applied | [`composite`] |
//! | One operation on a worker thread with a time limit | [`run_with_time_limit`] |
//!
//! Every function works on one layer's contours and holds no state, so a caller can run one
//! glyph at a time, each under its own guard or time limit (docs/plans/M3.md › M3.6, rule 8).
//! `cargo xtask overlap` measures [`union`] on the corpus.
#![forbid(unsafe_code)]

mod boolean;
mod contour;
mod corner;
pub mod corner_helpers;
mod cut;
mod direction;
mod extremes;
pub mod filters;
pub mod geometry_helpers;
mod graph;
mod inflections;
mod limit;
mod mask;
pub mod matcher;
pub mod stem_filters;
pub mod stems;
mod stroke;
mod stroker;
pub mod tension;
pub mod thickness;
mod tidy;
pub mod trace;

use std::fmt;

use fontcraft_geom::{BezPath, CubicBez, ParamCurve, PathEl, Point, QuadBez, Rect, Shape};

pub use boolean::{BoolOp, OpenPaths, OverlapOptions, SMOOTH_MAX_DEGREES, binary, boolean, difference, intersection, remove_overlap};
pub use contour::{Contour, MAX_NODES, MIN_GRID, Node, NodeKind, contours_from_bez, round_to_grid};
pub use corner::{MAX_RECONNECT_NODES, Opened, open_corners, reconnect_nodes};
pub use cut::cut_with_line;
pub use direction::{Directed, MAX_NESTING_CONTOURS, correct_direction, shape_order};
pub use extremes::{
    ExtremesOptions, MAX_EXTREME_SHIFT, MIN_EXTREME_SEGMENT, NEAR_EXTREME_DEGREES, NEAR_EXTREME_DISTANCE, NEAR_EXTREME_SHARE,
    SHORT_EXTREME_SEGMENT, SHORT_EXTREME_SHARE, add_extremes,
};
pub use graph::Rewired;
pub use inflections::{MIN_INFLECTION_DISTANCE, add_inflections};
pub use limit::{TIME_LIMIT, Unfinished, catch_panic, run_with_time_limit};
pub use mask::{LayerShape, MAX_MASKS, MaskMode, composite};
pub use stroke::{
    Cap, CapAxis, Join, MAX_PEN_RATIO, MAX_STROKE_SEGMENTS, MAX_STROKE_WIDTH, MITER_LIMIT, STROKE_TOLERANCE, StrokeStyle, position_from_stored,
    position_to_stored, stroke_outline,
};
pub use tidy::{MAX_STRAIGHTEN, TIDY_TOLERANCE, tidy_up, tidy_up_straightening};

/// Largest coordinate magnitude, in font units, an operation accepts. Equal to the model's limit.
/// Below it linesweeper's tolerance stays at its floor of 1e-6 units.
pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// Most segments (lines and curves) one operation accepts. Linesweeper's time grows with the
/// number of crossings, which can grow with the square of the segment count.
pub const MAX_SEGMENTS: usize = 100_000;
/// Most pixels one coverage raster may have (4096 × 4096).
pub const MAX_COVERAGE_PIXELS: usize = 1 << 24;
/// Input coordinates are rounded to multiples of 1 / `SNAP` units. Points that differ by less
/// than linesweeper's tolerance (1e-6 units) can make it panic; on this grid they are either
/// equal or far enough apart.
pub const SNAP: f64 = 1024.0;
/// Result points closer than this, in font units, are one point. Linesweeper can return an
/// intersection twice, a few 1e-12 units apart.
pub const MERGE_DISTANCE: f64 = 1e-3;
/// Sub-rows sampled per pixel row; coverage across a row is exact.
const SUB_ROWS: usize = 16;
/// Flattening tolerance for coverage, in font units: far below the half-pixel tolerance of the
/// measure.
const FLATTEN_TOLERANCE: f64 = 0.02;

#[derive(Clone, Debug, PartialEq)]
pub enum PathOpError {
    /// A coordinate is NaN or infinite.
    NotFinite,
    /// A coordinate is beyond ±[`MAX_COORDINATE`].
    OutOfRange(f64),
    /// More than [`MAX_SEGMENTS`] segments.
    TooManySegments(usize),
    /// A subpath does not end with a close. Overlap removal takes closed contours only.
    OpenPath,
    /// The raster for a coverage comparison would exceed [`MAX_COVERAGE_PIXELS`].
    TooLarge { width: usize, height: usize },
    /// More than [`MAX_NODES`] nodes.
    TooManyNodes(usize),
    /// More than [`MAX_NESTING_CONTOURS`] closed contours to nest.
    TooManyContours(usize),
    /// A node list that makes no path (an off-curve node where an on-curve one must be, or the
    /// wrong number of off-curve nodes before an on-curve one).
    Malformed(String),
    /// A parameter the operation cannot take.
    Invalid(String),
    /// The boolean engine refused the input.
    Engine(String),
}

impl fmt::Display for PathOpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFinite => write!(f, "a coordinate is not a finite number"),
            Self::OutOfRange(v) => write!(f, "a coordinate ({v}) is beyond ±{MAX_COORDINATE}"),
            Self::TooManySegments(n) => write!(f, "the paths have {n} segments; at most {MAX_SEGMENTS} are allowed"),
            Self::OpenPath => write!(f, "an open path cannot take part in a path operation"),
            Self::TooLarge { width, height } => write!(f, "a {width} × {height} raster is too large to compare"),
            Self::TooManyNodes(n) => write!(f, "the paths have {n} nodes; at most {MAX_NODES} are allowed"),
            Self::TooManyContours(n) => write!(f, "the layer has {n} closed paths; at most {MAX_NESTING_CONTOURS} are allowed"),
            Self::Malformed(e) => write!(f, "a path cannot be read: {e}"),
            Self::Invalid(e) => write!(f, "{e}"),
            Self::Engine(e) => write!(f, "the path operation failed: {e}"),
        }
    }
}

impl std::error::Error for PathOpError {}

/// `paths` as one path for the boolean engine, after checking every coordinate, the segment
/// count and that every subpath is closed, with every point on the [`SNAP`] grid. A curve that
/// crosses itself is split inside its loop: linesweeper 0.5 panics on a result contour made of
/// one input segment alone.
pub fn prepare(paths: &[BezPath]) -> Result<BezPath, PathOpError> {
    let mut out = BezPath::new();
    let mut segments = 0usize;
    let (mut start, mut current) = (Point::ZERO, Point::ZERO);
    for path in paths {
        let mut open = false;
        for el in path.elements() {
            let points: &[Point] = match el {
                PathEl::MoveTo(p) => {
                    if open {
                        return Err(PathOpError::OpenPath);
                    }
                    open = true;
                    std::slice::from_ref(p)
                }
                PathEl::LineTo(p) => std::slice::from_ref(p),
                PathEl::QuadTo(a, b) => &[*a, *b],
                PathEl::CurveTo(a, b, c) => &[*a, *b, *c],
                PathEl::ClosePath => {
                    open = false;
                    &[]
                }
            };
            for p in points {
                check(p.x)?;
                check(p.y)?;
            }
            if !matches!(el, PathEl::MoveTo(_) | PathEl::ClosePath) {
                segments += 1;
                if segments > MAX_SEGMENTS {
                    return Err(PathOpError::TooManySegments(segments));
                }
            }
            let el = snap(*el);
            match el {
                PathEl::MoveTo(p) => {
                    start = p;
                    current = p;
                    out.push(el);
                }
                PathEl::LineTo(p) => {
                    current = p;
                    out.push(el);
                }
                PathEl::QuadTo(a, b) => {
                    if current == b {
                        let (x, y) = QuadBez::new(current, a, b).subdivide();
                        let mid = snap_point(x.p2);
                        out.quad_to(x.p1, mid);
                        out.quad_to(y.p1, b);
                    } else {
                        out.push(el);
                    }
                    current = b;
                }
                PathEl::CurveTo(a, b, c) => {
                    let cubic = CubicBez::new(current, a, b, c);
                    if let Some(t) = loop_split(cubic) {
                        let x = cubic.subsegment(0.0..t);
                        let y = cubic.subsegment(t..1.0);
                        let mid = snap_point(x.p3);
                        out.curve_to(x.p1, x.p2, mid);
                        out.curve_to(y.p1, y.p2, c);
                    } else {
                        out.push(el);
                    }
                    current = c;
                }
                PathEl::ClosePath => {
                    current = start;
                    out.push(el);
                }
            }
        }
        if open {
            return Err(PathOpError::OpenPath);
        }
    }
    Ok(out)
}

fn snap_point(p: Point) -> Point {
    Point::new((p.x * SNAP).round() / SNAP, (p.y * SNAP).round() / SNAP)
}

fn snap(el: PathEl) -> PathEl {
    let s = snap_point;
    match el {
        PathEl::MoveTo(a) => PathEl::MoveTo(s(a)),
        PathEl::LineTo(a) => PathEl::LineTo(s(a)),
        PathEl::QuadTo(a, b) => PathEl::QuadTo(s(a), s(b)),
        PathEl::CurveTo(a, b, c) => PathEl::CurveTo(s(a), s(b), s(c)),
        PathEl::ClosePath => PathEl::ClosePath,
    }
}

/// The parameter midway between the two parameters where `c` crosses itself, if it does
/// (an ending on its own start counts).
fn loop_split(c: CubicBez) -> Option<f64> {
    if c.p0 == c.p3 {
        return Some(0.5);
    }
    // Power form a t³ + b t² + k t + p0. Two parameters s ≠ t meet where
    // a (σ² − π) + b σ + k = 0, with σ = s + t and π = s t.
    let a = (c.p3 - c.p0) + 3.0 * (c.p1 - c.p2);
    let b = 3.0 * ((c.p0 - c.p1) + (c.p2 - c.p1));
    let k = 3.0 * (c.p1 - c.p0);
    let ab = a.cross(b);
    if ab == 0.0 {
        return None;
    }
    let sigma = -a.cross(k) / ab;
    let pi = if a.x.abs() >= a.y.abs() { sigma * sigma + (b.x * sigma + k.x) / a.x } else { sigma * sigma + (b.y * sigma + k.y) / a.y };
    let disc = sigma * sigma - 4.0 * pi;
    if disc.is_nan() || disc <= 0.0 || !pi.is_finite() {
        return None;
    }
    let r = disc.sqrt();
    let (s, t) = ((sigma - r) / 2.0, (sigma + r) / 2.0);
    (s >= -1e-9 && t <= 1.0 + 1e-9).then_some(sigma / 2.0)
}

fn check(v: f64) -> Result<(), PathOpError> {
    if !v.is_finite() {
        return Err(PathOpError::NotFinite);
    }
    if v.abs() > MAX_COORDINATE {
        return Err(PathOpError::OutOfRange(v));
    }
    Ok(())
}

/// The union of the closed contours in `paths` with the nonzero rule: the outline Remove Overlap
/// produces. Each result contour is simple and ends with an explicit line back to its start
/// before the close; outer contours run counter-clockwise (positive area, y up), counters
/// clockwise. Each result segment is a piece of an input segment of the same kind, so
/// quadratic input stays quadratic. Result points that are not input points (intersections) are
/// not on any grid.
pub fn union(paths: &[BezPath]) -> Result<Vec<BezPath>, PathOpError> {
    binary(paths, &[], BoolOp::Union)
}

/// `path` without segments that end within [`MERGE_DISTANCE`] of where they start. A contour that
/// ends near its start ends exactly on it.
fn merge_close_points(path: &BezPath) -> BezPath {
    let near = |a: Point, b: Point| a.distance(b) < MERGE_DISTANCE;
    let mut out = BezPath::new();
    let mut start = Point::ZERO;
    let mut current = Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                start = p;
                current = p;
                out.push(*el);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                let flat = match *el {
                    PathEl::QuadTo(a, _) => near(a, current),
                    PathEl::CurveTo(a, b, _) => near(a, current) && near(b, current),
                    _ => true,
                };
                if flat && near(p, current) {
                    continue;
                }
                current = p;
                out.push(*el);
            }
            PathEl::ClosePath => {
                if current != start && near(current, start) {
                    snap_end(&mut out, start);
                }
                current = start;
                out.push(*el);
            }
        }
    }
    out
}

fn snap_end(path: &mut BezPath, to: Point) {
    let mut els = path.elements().to_vec();
    if let Some(last) = els.last_mut() {
        match last {
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => *p = to,
            PathEl::MoveTo(_) | PathEl::ClosePath => {}
        }
    }
    *path = BezPath::from_vec(els);
}

/// How two outlines differ when both are rendered with the nonzero rule at one pixel per unit,
/// anti-aliased.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CoverageDiff {
    /// Largest coverage difference of one pixel, 0 to 1.
    pub max_pixel: f64,
    /// Sum of the per-pixel differences, in square units.
    pub total: f64,
    /// Covered area of the first outline, in square units.
    pub ink: f64,
}

impl CoverageDiff {
    /// Within the overlap-removal measure: at most `max_pixel` per pixel and `share` of the first
    /// outline's ink in total (a share of one square unit when it has no ink).
    pub fn within(&self, max_pixel: f64, share: f64) -> bool {
        self.max_pixel <= max_pixel && self.total <= share * self.ink.max(1.0)
    }
}

/// Compares `a` and `b` by rendered coverage over the box that holds both.
pub fn coverage_difference(a: &[BezPath], b: &[BezPath]) -> Result<CoverageDiff, PathOpError> {
    let pa = prepare(a)?;
    let pb = prepare(b)?;
    let bbox = match (bounds(&pa), bounds(&pb)) {
        (Some(x), Some(y)) => x.union(y),
        (Some(x), None) | (None, Some(x)) => x,
        (None, None) => return Ok(CoverageDiff::default()),
    };
    let grid = Grid::new(bbox)?;
    let ca = grid.coverage(&pa);
    let cb = grid.coverage(&pb);
    let mut diff = CoverageDiff::default();
    for (x, y) in ca.iter().zip(&cb) {
        let d = (x - y).abs();
        diff.max_pixel = diff.max_pixel.max(d);
        diff.total += d;
        diff.ink += x;
    }
    Ok(diff)
}

fn bounds(path: &BezPath) -> Option<Rect> {
    if path.elements().iter().all(|e| matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)) {
        return None;
    }
    Some(path.bounding_box())
}

struct Grid {
    x0: f64,
    y0: f64,
    width: usize,
    height: usize,
}

impl Grid {
    /// A grid on whole units around `bbox`, with a one-pixel margin. Coordinates are within
    /// ±[`MAX_COORDINATE`], so the casts below cannot overflow.
    fn new(bbox: Rect) -> Result<Self, PathOpError> {
        let x0 = bbox.x0.floor() - 1.0;
        let y0 = bbox.y0.floor() - 1.0;
        let width = (bbox.x1.ceil() + 1.0 - x0).max(1.0) as usize;
        let height = (bbox.y1.ceil() + 1.0 - y0).max(1.0) as usize;
        if width.saturating_mul(height) > MAX_COVERAGE_PIXELS {
            return Err(PathOpError::TooLarge { width, height });
        }
        Ok(Self { x0, y0, width, height })
    }

    /// Per-pixel coverage of `path`, row by row from the bottom.
    fn coverage(&self, path: &BezPath) -> Vec<f64> {
        let rows = self.height * SUB_ROWS;
        let mut crossings: Vec<Vec<(f64, i32)>> = vec![Vec::new(); rows];
        let mut start = Point::ZERO;
        let mut last = Point::ZERO;
        let mut edge = |p0: Point, p1: Point| {
            if p0.y == p1.y {
                return;
            }
            let (lo, hi, dir) = if p0.y < p1.y { (p0, p1, 1) } else { (p1, p0, -1) };
            // Sub-row centres in [lo.y, hi.y): a shared vertex is counted once.
            let first = ((lo.y - self.y0) * SUB_ROWS as f64 - 0.5).ceil().max(0.0) as usize;
            let mut row = first;
            while row < rows {
                let y = self.y0 + (row as f64 + 0.5) / SUB_ROWS as f64;
                if y >= hi.y {
                    break;
                }
                let x = lo.x + (y - lo.y) * (hi.x - lo.x) / (hi.y - lo.y);
                if let Some(c) = crossings.get_mut(row) {
                    c.push((x, dir));
                }
                row += 1;
            }
        };
        fontcraft_geom::flatten(path.elements().iter().copied(), FLATTEN_TOLERANCE, |el| match el {
            PathEl::MoveTo(p) => {
                if last != start {
                    edge(last, start);
                }
                start = p;
                last = p;
            }
            PathEl::LineTo(p) => {
                edge(last, p);
                last = p;
            }
            PathEl::ClosePath => {
                edge(last, start);
                last = start;
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        });
        if last != start {
            edge(last, start);
        }
        let mut out = vec![0.0; self.width * self.height];
        for (row, list) in crossings.iter_mut().enumerate() {
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            let base = (row / SUB_ROWS) * self.width;
            let mut winding = 0;
            let mut span_start = 0.0;
            for &(x, dir) in list.iter() {
                let was_inside = winding != 0;
                winding += dir;
                if !was_inside && winding != 0 {
                    span_start = x;
                } else if was_inside && winding == 0 {
                    self.add_span(&mut out, base, span_start - self.x0, x - self.x0);
                }
            }
        }
        out
    }

    /// Adds one sub-row's span [a, b) (in pixels from the left edge) to the row at `base`.
    fn add_span(&self, out: &mut [f64], base: usize, a: f64, b: f64) {
        let weight = 1.0 / SUB_ROWS as f64;
        let a = a.clamp(0.0, self.width as f64);
        let b = b.clamp(0.0, self.width as f64);
        let mut px = a.floor() as usize;
        while (px as f64) < b && px < self.width {
            let lo = a.max(px as f64);
            let hi = b.min(px as f64 + 1.0);
            if let Some(c) = out.get_mut(base + px) {
                *c += (hi - lo) * weight;
            }
            px += 1;
        }
    }
}

/// Contours the tests of several modules share.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::{Contour, Node, NodeKind};
    use fontcraft_geom::Point;

    /// A closed rectangle, counter-clockwise from the bottom left.
    pub(crate) fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Contour {
        let n = |x: f64, y: f64| Node::new(Point::new(x, y), NodeKind::Line);
        Contour::new(true, vec![n(x0, y0), n(x1, y0), n(x1, y1), n(x0, y1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x, y));
        p.line_to((x + s, y));
        p.line_to((x + s, y + s));
        p.line_to((x, y + s));
        p.close_path();
        p
    }

    #[test]
    fn overlapping_squares_become_one_counter_clockwise_contour() {
        let out = union(&[square(0.0, 0.0, 100.0), square(50.0, 50.0, 100.0)]).unwrap();
        assert_eq!(out.len(), 1);
        assert!((out[0].area() - 17_500.0).abs() < 1e-6);
        let d = coverage_difference(&[square(0.0, 0.0, 100.0), square(50.0, 50.0, 100.0)], &out).unwrap();
        assert!(d.max_pixel < 1e-9 && (d.ink - 17_500.0).abs() < 1.0, "{d:?}");
    }

    // Each of these reached linesweeper unchecked: a NaN panicked, an infinity gave a wrong
    // outline, and a huge coordinate removed the small square.
    #[test]
    fn hostile_coordinates_are_refused_before_the_engine() {
        let mut nan = square(0.0, 0.0, 100.0);
        nan.line_to((f64::NAN, 5.0));
        nan.close_path();
        assert_eq!(union(&[nan]), Err(PathOpError::NotFinite));
        assert_eq!(union(&[square(0.0, 0.0, 10.0), square(f64::INFINITY, 0.0, 1.0)]), Err(PathOpError::NotFinite));
        assert_eq!(union(&[square(0.0, 0.0, 10.0), square(1e20, 0.0, 1.0)]), Err(PathOpError::OutOfRange(1e20)));
        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((10.0, 0.0));
        assert_eq!(union(&[open]), Err(PathOpError::OpenPath));
    }

    // Both made linesweeper 0.5 panic before the curves were split inside their loops.
    #[test]
    fn curves_that_cross_themselves_do_not_reach_the_engine_whole() {
        let looped = BezPath::from_svg("M0,0 C300,400 200,0 0,100 Z").unwrap();
        let out = union(std::slice::from_ref(&looped)).unwrap();
        assert!(coverage_difference(&[looped], &out).unwrap().within(0.5, 0.005));
        let spike = BezPath::from_svg("M300,100 Q100,100 200,0 Q200,400 300,100 Z M0,0 C300,400 200,0 0,100 Q200,100 0,100 Z").unwrap();
        assert!(union(&[spike]).is_ok());
    }

    #[test]
    fn coverage_sees_a_half_unit_shift() {
        let d = coverage_difference(&[square(0.0, 0.0, 10.0)], &[square(0.5, 0.0, 10.0)]).unwrap();
        assert!((d.max_pixel - 0.5).abs() < 1e-9 && (d.total - 10.0).abs() < 1e-6 && (d.ink - 100.0).abs() < 1e-6, "{d:?}");
    }

    /// One random case: up to 64 segments in closed contours, with points drawn from one of
    /// four grids (free, whole units, a coarse grid, a sub-tolerance cluster) and reused often.
    fn random_case(seed: &mut u64) -> Vec<BezPath> {
        let mut next = || {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *seed
        };
        let mode = next() % 4;
        let mut pool: Vec<Point> = Vec::new();
        let mut point = |next: &mut dyn FnMut() -> u64| {
            if !pool.is_empty() && next().is_multiple_of(5) {
                return pool[(next() % pool.len() as u64) as usize];
            }
            let unit = |n: u64| (n >> 11) as f64 / (1u64 << 53) as f64;
            let p = match mode {
                0 => Point::new(unit(next()) * 2000.0 - 1000.0, unit(next()) * 2000.0 - 1000.0),
                1 => Point::new((next() % 2001) as f64 - 1000.0, (next() % 2001) as f64 - 1000.0),
                2 => Point::new((next() % 5) as f64 * 100.0, (next() % 5) as f64 * 100.0),
                _ => Point::new((next() % 3) as f64 * 1e-7 + 500.0, (next() % 3) as f64 * 1e-7 + (next() % 2) as f64 * 300.0),
            };
            pool.push(p);
            p
        };
        let mut left = 1 + (next() % 64) as usize;
        let mut paths = Vec::new();
        while left > 0 {
            let len = 1 + (next() % left as u64) as usize;
            left -= len;
            let mut p = BezPath::new();
            p.move_to(point(&mut next));
            for _ in 0..len {
                match next() % 3 {
                    0 => p.line_to(point(&mut next)),
                    1 => p.quad_to(point(&mut next), point(&mut next)),
                    _ => p.curve_to(point(&mut next), point(&mut next), point(&mut next)),
                }
            }
            p.close_path();
            paths.push(p);
        }
        paths
    }

    /// Every operation on one random case. Errors are fine; panics are not.
    fn every_operation(paths: &[BezPath], seed: u64) {
        let contours: Vec<Contour> = paths.iter().flat_map(contours_from_bez).collect();
        let _ = union(paths);
        let (a, b) = paths.split_at(paths.len() / 2);
        let _ = difference(a, b);
        let _ = intersection(a, b);
        let _ = remove_overlap(&contours, OverlapOptions::default());
        let (ca, cb) = contours.split_at(contours.len() / 2);
        let _ = boolean(ca, cb, BoolOp::Intersection, 1.0);
        let _ = correct_direction(&contours);
        for c in &contours {
            let _ = add_extremes(c, None, ExtremesOptions { force: seed.is_multiple_of(2), grid: 1.0 });
            let _ = tidy_up(c, None);
            let all: Vec<usize> = (0..c.nodes.len()).collect();
            let _ = open_corners(c, &all, fontcraft_geom::Vec2::new(10.0, 20.0), 1.0);
        }
        let on_curve: Vec<(usize, usize)> = contours
            .iter()
            .enumerate()
            .flat_map(|(i, c)| c.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).map(move |(k, _)| (i, k)))
            .take(4)
            .collect();
        let _ = reconnect_nodes(&contours, &on_curve, 1.0);
        let from = Point::new((seed % 2001) as f64 - 1000.0, -1100.0);
        let to = Point::new(((seed >> 16) % 2001) as f64 - 1000.0, 1100.0);
        let _ = cut_with_line(&contours, from, to, 1.0);
    }

    /// Runs `cases` random cases through `run`, each on its own thread, and lists the cases that
    /// panicked or took more than a second.
    fn random_run(cases: usize, run: fn(&[BezPath], u64)) -> Vec<String> {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut failures = Vec::new();
        for i in 0..cases {
            let paths = random_case(&mut seed);
            let (tx, rx) = std::sync::mpsc::channel();
            let input = paths.clone();
            let case_seed = seed;
            std::thread::spawn(move || {
                let r = std::panic::catch_unwind(|| run(&input, case_seed));
                let _ = tx.send(r.is_ok());
            });
            let failure = match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(true) => continue,
                Ok(false) => format!("case {i} panicked"),
                Err(_) => format!("case {i} took more than a second"),
            };
            let svg = paths.iter().map(BezPath::to_svg).collect::<Vec<_>>().join(" ");
            failures.push(if failures.is_empty() { format!("{failure}: {svg}") } else { failure });
        }
        failures
    }

    #[test]
    fn random_paths_through_every_operation_never_panic() {
        let failures = random_run(500, every_operation);
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The plan's random-path measure: 10,000 cases of up to 64 segments through every operation.
    /// Ignored for its length, and because linesweeper 0.5 still hangs on rare inputs
    /// (docs/plans/M3.md › M3.6, spike result), which a thread cannot stop.
    /// `cargo test -p fontcraft-pathops -- --ignored random_paths`
    #[test]
    #[ignore]
    fn random_paths_never_panic_and_finish_within_a_second() {
        let failures = random_run(10_000, every_operation);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

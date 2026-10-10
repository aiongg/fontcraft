//! Roughen (`filter.roughen`, `filter.roughen.segmentsThenJitter`,
//! `filter.roughen.maxOffsetPerAxis`, docs/spec/11-filters.md › Roughen).
//!
//! Two steps. First every segment is cut into straight pieces of about `segmentLength` units:
//! a segment of length `L` becomes `max(1, round(L / segmentLength))` equal pieces measured along
//! the curve, so curves turn into polylines and every node is a line node. Then every node,
//! original or new, moves by a random vector. The vector is uniform in a box of ±`horizontal` by
//! ±`vertical` units, turned by `angle` degrees counter-clockwise (0 keeps the box upright).
//!
//! The random numbers come from SplitMix64 (Vigna, public domain), seeded with `seed`, and are
//! drawn in node order, so one seed gives one result.

use fontcraft_geom::Point;

use super::{Args, Context, Filter, Param, ParamKind, Slot, Value};
use crate::contour::{Contour, Node, NodeKind, Piece, Ring, Seg, round_to_grid};
use crate::{MAX_COORDINATE, MAX_NODES, PathOpError, check};

/// Shortest segment length accepted, in units. Shorter pieces would only reach the node limit.
pub const MIN_SEGMENT_LENGTH: f64 = 1.0;
/// Largest seed: the largest integer a number parameter holds exactly.
pub const MAX_SEED: i64 = 1 << 53;
/// Points sampled along each curved piece to measure and cut it.
const CURVE_SAMPLES: usize = 32;

pub struct Roughen;

const PARAMS: &[Param] = &[
    Param {
        key: "segmentLength",
        label: "Segment Length",
        kind: ParamKind::Number { min: MIN_SEGMENT_LENGTH, max: MAX_COORDINATE },
        default: Value::Number(50.0),
        required: true,
        slot: Slot::Position(0),
        doc: "target length of the straight pieces every segment is cut into, in units",
    },
    Param {
        key: "horizontal",
        label: "Horizontal",
        kind: ParamKind::Number { min: 0.0, max: MAX_COORDINATE },
        default: Value::Number(10.0),
        required: true,
        slot: Slot::Position(1),
        doc: "largest move of a node along x, in units",
    },
    Param {
        key: "vertical",
        label: "Vertical",
        kind: ParamKind::Number { min: 0.0, max: MAX_COORDINATE },
        default: Value::Number(10.0),
        required: true,
        slot: Slot::Position(2),
        doc: "largest move of a node along y, in units",
    },
    Param {
        key: "angle",
        label: "Angle",
        kind: ParamKind::Number { min: -360.0, max: 360.0 },
        default: Value::Number(0.0),
        required: false,
        slot: Slot::Position(3),
        doc: "turns the box of possible moves counter-clockwise, in degrees; 0 keeps x and y",
    },
    Param {
        key: "seed",
        label: "Seed",
        kind: ParamKind::Integer { min: 0, max: MAX_SEED },
        default: Value::Number(0.0),
        required: false,
        slot: Slot::None,
        doc: "the same seed gives the same result; not part of the export string",
    },
];

const DOC: &str = "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), segmentLength: units (1 to 1000000), horizontal: units (0 to 1000000), vertical: units (0 to 1000000), angle?: degrees (-360 to 360, default 0), seed?: integer (0 to 9007199254740992, default 0)} cuts every segment into straight pieces of about segmentLength units, then moves every node (original and new) by a random amount of at most horizontal units along x and vertical units along y, uniformly distributed; angle turns that box counter-clockwise. The result has line nodes only. The same seed gives the same result. A segment length that would give more nodes than a layer may hold is an error. Components and locked paths are not changed; moved points are rounded to the grid. Export string: Roughenizer; segmentlength; x; y; angle (the seed is not in it) → {glyphs: [names], skipped: [{glyph, layer?, reason}]}";

impl Filter for Roughen {
    fn id(&self) -> &'static str {
        "filter.roughen"
    }

    fn name(&self) -> &'static str {
        "Roughenizer"
    }

    fn label(&self) -> &'static str {
        "Roughen"
    }

    fn doc(&self) -> &'static str {
        DOC
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn apply(&self, contours: &[Contour], args: &Args, cx: &Context) -> Result<Vec<Contour>, PathOpError> {
        let options = Options {
            segment_length: args.number("segmentLength"),
            horizontal: args.number("horizontal"),
            vertical: args.number("vertical"),
            angle: args.number("angle"),
            seed: args.number("seed"),
        };
        roughen(contours, &options, cx.grid)
    }
}

/// The settings of [`roughen`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Units; at least [`MIN_SEGMENT_LENGTH`].
    pub segment_length: f64,
    /// Largest move along x before the angle, in units.
    pub horizontal: f64,
    /// Largest move along y before the angle, in units.
    pub vertical: f64,
    /// Degrees counter-clockwise.
    pub angle: f64,
    /// A whole number from 0 to [`MAX_SEED`].
    pub seed: f64,
}

/// Roughen on one layer's contours: see the module docs. New and moved points are rounded to
/// `grid`. A node that survives keeps its `tag`.
pub fn roughen(contours: &[Contour], options: &Options, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    let Options { segment_length, horizontal, vertical, angle, seed } = *options;
    for v in [segment_length, horizontal, vertical, angle, seed] {
        if !v.is_finite() {
            return Err(PathOpError::NotFinite);
        }
    }
    if !(MIN_SEGMENT_LENGTH..=MAX_COORDINATE).contains(&segment_length) {
        return Err(PathOpError::Invalid(format!(
            "the segment length ({segment_length}) must be from {MIN_SEGMENT_LENGTH} to {MAX_COORDINATE} units"
        )));
    }
    for (name, v) in [("horizontal", horizontal), ("vertical", vertical)] {
        if !(0.0..=MAX_COORDINATE).contains(&v) {
            return Err(PathOpError::Invalid(format!("{name} ({v}) must be from 0 to {MAX_COORDINATE} units")));
        }
    }
    if !(-360.0..=360.0).contains(&angle) {
        return Err(PathOpError::Invalid(format!("the angle ({angle}) must be from -360 to 360 degrees")));
    }
    if seed.fract() != 0.0 || !(0.0..=MAX_SEED as f64).contains(&seed) {
        return Err(PathOpError::Invalid(format!("the seed ({seed}) must be a whole number from 0 to {MAX_SEED}")));
    }
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;

    let (sin, cos) = angle.to_radians().sin_cos();
    let mut jitter = Jitter { rng: SplitMix64(seed as u64), horizontal, vertical, sin, cos, grid, moves: horizontal > 0.0 || vertical > 0.0 };
    let mut count = 0usize;
    let mut out = Vec::with_capacity(rings.len());
    for ring in &rings {
        let contour = roughen_ring(ring, segment_length, &mut jitter, &mut count)?;
        out.push(contour);
    }
    Ok(out)
}

fn roughen_ring(ring: &Ring, segment_length: f64, jitter: &mut Jitter, count: &mut usize) -> Result<Contour, PathOpError> {
    let m = ring.len();
    let mut nodes: Vec<Node> = Vec::new();
    let Some(first) = ring.nodes.first() else { return Ok(Contour::new(ring.closed, Vec::new())) };
    put(&mut nodes, jitter, count, first.node, false)?;
    // Segment `j` ends at node `j`; in a closed ring segment 0 comes last, back to the start.
    for j in (1..m).chain(ring.closed.then_some(0)) {
        let Some(seg) = ring.seg(j) else { continue };
        let walk = Walk::new(&seg);
        let ratio = (walk.length() / segment_length).round();
        let pieces = if ratio >= 1.0 { ratio.min((MAX_NODES + 1) as f64) as usize } else { 1 };
        // Checked before the points are made, so a tiny segment length cannot allocate.
        if count.saturating_add(pieces) > MAX_NODES {
            return Err(PathOpError::TooManyNodes(count.saturating_add(pieces)));
        }
        for point in walk.split(pieces) {
            put(&mut nodes, jitter, count, Node::new(point, NodeKind::Line), true)?;
        }
        if j > 0
            && let Some(end) = ring.nodes.get(j)
        {
            put(&mut nodes, jitter, count, end.node, false)?;
        }
    }
    Ok(Contour::new(ring.closed, nodes))
}

/// Appends `node` as a line node, moved by the jitter, and counts it against [`MAX_NODES`].
fn put(nodes: &mut Vec<Node>, jitter: &mut Jitter, count: &mut usize, node: Node, new: bool) -> Result<(), PathOpError> {
    *count += 1;
    if *count > MAX_NODES {
        return Err(PathOpError::TooManyNodes(*count));
    }
    let point = jitter.place(node.point, new)?;
    nodes.push(Node { point, kind: NodeKind::Line, smooth: false, tag: node.tag });
    Ok(())
}

/// The random move of each node.
struct Jitter {
    rng: SplitMix64,
    horizontal: f64,
    vertical: f64,
    sin: f64,
    cos: f64,
    grid: f64,
    /// Whether any move is non-zero; if not, original nodes are left exactly where they are.
    moves: bool,
}

impl Jitter {
    /// Where `p` ends up. `new` marks a point the filter made, which is rounded to the grid
    /// whether it moves or not.
    fn place(&mut self, p: Point, new: bool) -> Result<Point, PathOpError> {
        let mut q = p;
        if self.moves {
            let (u, w) = (self.rng.signed() * self.horizontal, self.rng.signed() * self.vertical);
            q = Point::new(p.x + u * self.cos - w * self.sin, p.y + u * self.sin + w * self.cos);
        }
        if self.moves || new {
            q = round_to_grid(q, self.grid);
        }
        check(q.x)?;
        check(q.y)?;
        Ok(q)
    }
}

/// SplitMix64: one `u64` of state, enough for a reproducible displacement.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [-1, 1).
    fn signed(&mut self) -> f64 {
        // The top 53 bits give an exact fraction in [0, 1).
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        unit * 2.0 - 1.0
    }
}

/// One segment sampled densely, so points can be placed at given distances along it.
struct Walk {
    pieces: Vec<Piece>,
    /// (piece index, curve parameter) of each sample, in path order.
    samples: Vec<(usize, f64)>,
    /// Distance along the segment of each sample, by the chords between samples.
    along: Vec<f64>,
}

impl Walk {
    fn new(seg: &Seg<'_>) -> Walk {
        let pieces = seg.pieces();
        let mut samples = Vec::new();
        let mut along = Vec::new();
        let mut total = 0.0;
        let mut last: Option<Point> = None;
        for (index, piece) in pieces.iter().enumerate() {
            let steps = if matches!(piece, Piece::Line(..)) { 1 } else { CURVE_SAMPLES };
            for k in 0..=steps {
                let t = k as f64 / steps as f64;
                let point = piece.eval(t);
                if let Some(prev) = last {
                    total += prev.distance(point);
                }
                last = Some(point);
                samples.push((index, t));
                along.push(total);
            }
        }
        Walk { pieces, samples, along }
    }

    fn length(&self) -> f64 {
        self.along.last().copied().unwrap_or(0.0)
    }

    /// The `n - 1` points that cut the segment into `n` pieces of equal length.
    fn split(&self, n: usize) -> Vec<Point> {
        let length = self.length();
        let mut out = Vec::with_capacity(n.saturating_sub(1));
        let mut i = 0usize;
        for k in 1..n {
            let target = length * k as f64 / n as f64;
            while self.along.get(i + 1).is_some_and(|&d| d < target) {
                i += 1;
            }
            let (Some(&(pa, ta)), Some(&(pb, tb)), Some(&da), Some(&db)) =
                (self.samples.get(i), self.samples.get(i + 1), self.along.get(i), self.along.get(i + 1))
            else {
                break;
            };
            let fraction = if db > da { ((target - da) / (db - da)).clamp(0.0, 1.0) } else { 0.0 };
            // Across a join of two pieces the interval has no length and both ends are one point.
            let t = if pa == pb { ta + (tb - ta) * fraction } else { ta };
            if let Some(piece) = self.pieces.get(pa) {
                out.push(piece.eval(t));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn poly(points: &[(f64, f64)], closed: bool) -> Contour {
        Contour::new(closed, points.iter().map(|&(x, y)| Node::new(Point::new(x, y), NodeKind::Line)).collect())
    }

    fn square(size: f64) -> Contour {
        poly(&[(0.0, 0.0), (size, 0.0), (size, size), (0.0, size)], true)
    }

    fn options(segment_length: f64, horizontal: f64, vertical: f64) -> Options {
        Options { segment_length, horizontal, vertical, angle: 0.0, seed: 0.0 }
    }

    /// A counter-clockwise circle of radius 100 as four cubics.
    fn circle() -> Contour {
        let k = 55.23;
        let on = |x: f64, y: f64| Node::new(Point::new(x, y), NodeKind::Curve);
        let off = |x: f64, y: f64| Node::off(Point::new(x, y));
        Contour::new(
            true,
            vec![
                on(100.0, 0.0),
                off(100.0, k),
                off(k, 100.0),
                on(0.0, 100.0),
                off(-k, 100.0),
                off(-100.0, k),
                on(-100.0, 0.0),
                off(-100.0, -k),
                off(-k, -100.0),
                on(0.0, -100.0),
                off(k, -100.0),
                off(100.0, -k),
            ],
        )
    }

    fn points(c: &Contour) -> Vec<(f64, f64)> {
        c.nodes.iter().map(|n| (n.point.x, n.point.y)).collect()
    }

    #[test]
    fn segments_are_cut_to_the_length_before_nodes_move() {
        let out = roughen(&[square(100.0).tagged()], &options(25.0, 0.0, 0.0), 0.0).unwrap();
        let c = &out[0];
        assert!(c.closed && c.nodes.len() == 16);
        assert!(c.nodes.iter().all(|n| n.kind == NodeKind::Line && !n.smooth));
        assert_eq!(&points(c)[..5], &[(0.0, 0.0), (25.0, 0.0), (50.0, 0.0), (75.0, 0.0), (100.0, 0.0)]);
        assert_eq!(c.nodes[4].tag, Some(1));
        assert!(c.nodes[1].tag.is_none());
        // A segment of 100 units at length 30 is three equal pieces.
        let out = roughen(&[poly(&[(0.0, 0.0), (100.0, 0.0)], false)], &options(30.0, 0.0, 0.0), 0.0).unwrap();
        let xs: Vec<f64> = out[0].nodes.iter().map(|n| n.point.x).collect();
        assert_eq!(xs.len(), 4);
        assert!(!out[0].closed && (xs[1] - 100.0 / 3.0).abs() < 1e-9 && xs[3] == 100.0);
        // A segment shorter than the length stays one piece.
        let out = roughen(&[poly(&[(0.0, 0.0), (10.0, 0.0)], false)], &options(50.0, 0.0, 0.0), 0.0).unwrap();
        assert_eq!(out[0].nodes.len(), 2);
    }

    #[test]
    fn curves_become_even_straight_pieces_on_the_curve() {
        let out = roughen(&[circle()], &options(50.0, 0.0, 0.0), 0.0).unwrap();
        let c = &out[0];
        // 628 units round: 3 pieces for each quarter.
        assert_eq!(c.nodes.len(), 12);
        assert!(c.nodes.iter().all(|n| n.kind == NodeKind::Line));
        for n in &c.nodes {
            assert!((n.point.to_vec2().hypot() - 100.0).abs() < 0.1, "{:?}", n.point);
        }
        let chord = |a: Point, b: Point| a.distance(b);
        let first = chord(c.nodes[0].point, c.nodes[1].point);
        assert!((first - chord(c.nodes[1].point, c.nodes[2].point)).abs() < 0.5);
    }

    #[test]
    fn moves_stay_in_the_box_and_a_seed_repeats() {
        let base = roughen(&[square(200.0)], &options(40.0, 0.0, 0.0), 0.0).unwrap();
        let run = |seed: f64, angle: f64| {
            roughen(&[square(200.0)], &Options { segment_length: 40.0, horizontal: 10.0, vertical: 3.0, angle, seed }, 0.0).unwrap()
        };
        let a = run(7.0, 0.0);
        assert_eq!(a, run(7.0, 0.0));
        assert_ne!(a, run(8.0, 0.0));
        assert_ne!(a, base);
        let mut spread = (0.0f64, 0.0f64);
        for (n, b) in a[0].nodes.iter().zip(&base[0].nodes) {
            let (dx, dy) = (n.point.x - b.point.x, n.point.y - b.point.y);
            assert!(dx.abs() <= 10.0 && dy.abs() <= 3.0, "{dx} {dy}");
            spread = (spread.0.max(dx.abs()), spread.1.max(dy.abs()));
        }
        // 20 nodes drawn uniformly reach well past a quarter of each range.
        assert!(spread.0 > 2.5 && spread.1 > 0.75, "{spread:?}");
        // Turned by 90 degrees the horizontal limit applies along y.
        for (n, b) in run(7.0, 90.0)[0].nodes.iter().zip(&base[0].nodes) {
            assert!((n.point.x - b.point.x).abs() <= 3.0 + 1e-9 && (n.point.y - b.point.y).abs() <= 10.0 + 1e-9);
        }
    }

    #[test]
    fn points_are_rounded_to_the_grid() {
        let out = roughen(&[square(100.0)], &options(30.0, 7.0, 7.0), 5.0).unwrap();
        assert!(out[0].nodes.iter().all(|n| n.point.x % 5.0 == 0.0 && n.point.y % 5.0 == 0.0));
        // Without moves the original corners are not touched by the grid.
        let out = roughen(&[poly(&[(1.0, 1.0), (4.0, 1.0)], false)], &options(50.0, 0.0, 0.0), 5.0).unwrap();
        assert_eq!(points(&out[0]), vec![(1.0, 1.0), (4.0, 1.0)]);
    }

    #[test]
    fn hostile_settings_and_sizes_are_errors() {
        let sq = [square(100.0)];
        for bad in [
            options(0.0, 1.0, 1.0),
            options(0.5, 1.0, 1.0),
            options(-5.0, 1.0, 1.0),
            options(f64::NAN, 1.0, 1.0),
            options(10.0, f64::INFINITY, 1.0),
            options(10.0, -1.0, 1.0),
            options(10.0, 1.0, 2.0e6),
            Options { angle: 400.0, ..options(10.0, 1.0, 1.0) },
            Options { seed: -1.0, ..options(10.0, 1.0, 1.0) },
            Options { seed: 1.5, ..options(10.0, 1.0, 1.0) },
            Options { seed: 1e300, ..options(10.0, 1.0, 1.0) },
        ] {
            assert!(roughen(&sq, &bad, 0.0).is_err(), "{bad:?}");
        }
        // Moves that leave the coordinate range.
        let edge = poly(&[(MAX_COORDINATE, 0.0), (MAX_COORDINATE, 10.0), (0.0, 10.0)], true);
        assert!(matches!(roughen(&[edge], &options(10.0, 1.0e6, 0.0), 0.0), Err(PathOpError::OutOfRange(_))));
        // 800000 units of outline cut into pieces of 1 unit: too many nodes, and no allocation of them.
        assert!(matches!(roughen(&[square(200_000.0)], &options(1.0, 0.0, 0.0), 0.0), Err(PathOpError::TooManyNodes(_))));
        // A path that cannot be read.
        let bad = Contour::new(false, vec![Node::off(Point::new(0.0, 0.0))]);
        assert!(roughen(&[bad], &options(10.0, 1.0, 1.0), 0.0).is_err());
    }

    #[test]
    fn empty_and_single_node_paths_pass() {
        let out = roughen(&[Contour::new(true, Vec::new()), poly(&[(5.0, 5.0)], true)], &options(10.0, 0.0, 0.0), 0.0).unwrap();
        assert!(out[0].nodes.is_empty());
        assert_eq!(points(&out[1]), vec![(5.0, 5.0)]);
    }
}

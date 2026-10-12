//! Adjust Stems, Change Weight, Condense/Extend and Change x-Height (M13.24), on one layer's
//! contours, using the stems of [`crate::stems`].
//!
//! Every operation moves nodes and keeps the node lists as they are (same nodes, same order,
//! same kinds), so a caller can write each result back node by node. The one exception is Change
//! Weight with `cleanup`, which runs Remove Overlap afterwards ([`Reshaped::rebuilt`]).
//!
//! | Operation | What moves |
//! |---|---|
//! | [`adjust_stems`] | The edges of each horizontal or vertical stem whose width a rule's range holds, to the rule's width, both edges by half the change; a horizontal stem's edge on a fixed height (baseline, x-height…) stays and the other edge takes the whole change. Diagonal stems stay |
//! | [`change_weight`] | Every closed contour is offset outwards by half the amount (each on-curve node along the bisector of its two normals, handles with their nodes). Method `lcg` then shifts nodes back vertically so the bottom and top zones keep their heights: nodes within the serif height of a zone shift rigidly, the rest by a linear blend. Counters `retain` spreads the vertical stems apart so the counters between them keep their widths, and the advance width grows by what that adds |
//! | [`condense_extend`] | Horizontal positions, piece by piece: vertical stems keep their widths, the spaces between and beside them scale, counters (between two stems) also take `counter_add`, and the sidebearings scale and add their own values. A slanted outline is unslanted by the italic angle first and reslanted after |
//! | [`change_x_height`] | Vertical positions: the band from the serif height to the x-height minus the serif height stretches; within the serif height of the baseline nothing moves, within it of the x-height everything shifts rigidly; between the x-height band and the ascender the outline is stretched so the ascender stays |
//!
//! Handles follow their nodes: a node's displacement moves the off-curve node next to it on each
//! side; the one control point of a quadratic segment takes the mean of both ends.

use fontcraft_geom::{BezPath, Line, ParamCurve, PathSeg, Point, Rect, Shape, Vec2};

use crate::contour::{Contour, MAX_NODES, Piece, Ring, round_to_grid};
use crate::stems::{self, Found, Part, Side, StemDirection, StemOptions};
use crate::{OpenPaths, OverlapOptions, PathOpError, remove_overlap};

/// Most rules of one direction that [`adjust_stems`] takes.
pub const MAX_RULES: usize = 64;
/// No stem is made narrower than this, in font units.
pub const MIN_STEM_WIDTH: f64 = 1.0;
/// Largest change of weight, in font units.
pub const MAX_WEIGHT_CHANGE: f64 = 1_000.0;
/// Largest scale factor of [`condense_extend`] (1,000%).
pub const MAX_SCALE: f64 = 10.0;
/// Largest value added to counters or sidebearings, in font units.
pub const MAX_ADD: f64 = 10_000.0;
/// Widest stem the filters look for, in font units.
pub const MAX_FILTER_STEM: f64 = 2_000.0;
/// Steepest italic angle unslanted, in degrees.
pub const MAX_ITALIC_ANGLE: f64 = 80.0;
/// An edge this close to a fixed height, in font units, lies on it.
const ON_LINE: f64 = 0.5;
/// A round stem's side moves the node at the nearer end of its curve when the curve's tangent
/// there is within this many degrees of the stem's direction.
const ROUND_NODE_DEGREES: f64 = 20.0;
/// At a corner sharper than this cosine (between the bisector and a normal), the offset of
/// [`change_weight`] stops growing: a spike is moved at most four times the offset.
const MIN_MITER_COS: f64 = 0.25;
/// Ink tests [`change_weight`] makes per contour to find which side the ink is on.
const SIDE_SAMPLES: usize = 16;
/// Segment visits the side tests of [`change_weight`] may make; past it, the contour's direction
/// decides.
const SIDE_WORK: usize = 50_000_000;
const PROBE: f64 = 0.5;

/// One rule of Adjust Stems: stems from `from` to `to` units wide (both included) become `set`
/// units wide, or `offset` units wider. Exactly one of `set` and `offset` is given.
#[derive(Clone, Debug, PartialEq)]
pub struct StemRule {
    pub from: f64,
    pub to: f64,
    pub set: Option<f64>,
    pub offset: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdjustStems {
    /// Rules for vertical stems (width measured horizontally); the first rule whose range holds a
    /// stem's width applies.
    pub vertical: Vec<StemRule>,
    pub horizontal: Vec<StemRule>,
    /// Heights on which a horizontal stem's edge stays put (baseline, x-height, cap height…).
    pub fixed_heights: Vec<f64>,
    /// Moved points are rounded to this grid; 0 leaves them.
    pub grid: f64,
}

/// What [`adjust_stems`] made.
#[derive(Clone, Debug, PartialEq)]
pub struct Adjusted {
    pub contours: Vec<Contour>,
    /// How many stems changed width.
    pub stems: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightMethod {
    /// Keep the bottom and top zones: letter heights do not grow.
    Lcg,
    /// Only expand (or shrink).
    Cjk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterMode {
    /// Counters shrink as the stems grow; the advance width stays.
    Squish,
    /// Counters keep their widths; the glyph gets wider.
    Retain,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeWeight {
    /// How much thicker every stem gets, in font units; negative thins.
    pub amount: f64,
    pub method: WeightMethod,
    /// The zones [`WeightMethod::Lcg`] keeps.
    pub bottom: f64,
    pub top: f64,
    /// Features within this distance of a zone move rigidly.
    pub serif_height: f64,
    pub counters: CounterMode,
    /// Remove overlaps afterwards.
    pub cleanup: bool,
    pub grid: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CondenseExtend {
    /// Factor for the spaces between and beside the vertical stems (0.8 for 80%).
    pub counter_scale: f64,
    /// Added to each counter between two stems, in font units.
    pub counter_add: f64,
    pub sidebearing_scale: f64,
    pub sidebearing_add: f64,
    /// Degrees, positive leaning right.
    pub italic_angle: f64,
    /// The layer's advance width.
    pub width: f64,
    pub grid: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeXHeight {
    pub from: f64,
    pub to: f64,
    pub serif_height: f64,
    /// The height that stays where it is above the x-height.
    pub ascender: f64,
    pub grid: f64,
}

/// What [`change_weight`] and [`condense_extend`] made.
#[derive(Clone, Debug, PartialEq)]
pub struct Reshaped {
    pub contours: Vec<Contour>,
    /// What to add to the layer's advance width.
    pub width_change: f64,
    /// Remove Overlap ran: the node lists differ from the input's.
    pub rebuilt: bool,
}

// ── Shared ──────────────────────────────────────────────────────────────────────────────────────

/// The contours as rings (closed ones with at least two nodes; `None` for the rest), their parts
/// and their fill.
struct Loaded {
    rings: Vec<Option<Ring>>,
    parts: Vec<Part>,
    ink: BezPath,
}

fn load(contours: &[Contour]) -> Result<Loaded, PathOpError> {
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let mut rings = Vec::with_capacity(contours.len());
    let mut parts = Vec::new();
    let mut ink = BezPath::new();
    for (ci, c) in contours.iter().enumerate() {
        let ring = Ring::from_contour(c)?;
        if !(ring.closed && ring.len() >= 2 && ring.nodes.iter().all(|r| r.src.is_some())) {
            rings.push(None);
            continue;
        }
        for j in ring.segment_ends() {
            let Some(seg) = ring.seg(j) else { continue };
            let pieces = seg.pieces();
            let last = pieces.len().saturating_sub(1);
            for (k, piece) in pieces.iter().enumerate() {
                let seg = match *piece {
                    Piece::Line(a, b) if a == b => continue,
                    Piece::Line(a, b) => PathSeg::Line(Line::new(a, b)),
                    Piece::Quad(q) => PathSeg::Quad(q),
                    Piece::Cubic(c) => PathSeg::Cubic(c),
                };
                let start = if k == 0 { ring.prev(j) } else { None };
                let end = (k == last).then_some(j);
                parts.push(Part { seg, contour: ci, start, end });
            }
        }
        ink.extend(ring.to_bez());
        rings.push(Some(ring));
    }
    Ok(Loaded { rings, parts, ink })
}

/// `contours` with the on-curve nodes of each ring moved by `moves[contour][ring node]` and their
/// handles with them; moved points rounded to `grid`.
fn apply_moves(contours: &[Contour], rings: &[Option<Ring>], moves: &[Vec<Vec2>], grid: f64) -> Vec<Contour> {
    let mut out = contours.to_vec();
    for ((contour, ring), by) in out.iter_mut().zip(rings).zip(moves) {
        let Some(ring) = ring else { continue };
        let n = contour.nodes.len();
        let mut shift = vec![Vec2::ZERO; n];
        for (j, r) in ring.nodes.iter().enumerate() {
            let Some(s) = r.src else { continue };
            let db = by.get(j).copied().unwrap_or(Vec2::ZERO);
            let da = ring.prev(j).and_then(|p| by.get(p)).copied().unwrap_or(Vec2::ZERO);
            if let Some(v) = shift.get_mut(s) {
                *v = db;
            }
            let k = r.ctrl.len();
            for i in 0..k {
                let d = match (k, i) {
                    (1, _) => (da + db) * 0.5,
                    (_, 0) => da,
                    (_, i) if i + 1 == k => db,
                    (_, i) => da.lerp(db, i as f64 / (k - 1) as f64),
                };
                // The handles of the segment that ends at node `s` come right before it in the
                // list, wrapping round in a closed contour.
                if let Some(v) = shift.get_mut((s + n - k + i) % n.max(1)) {
                    *v = d;
                }
            }
        }
        for (node, d) in contour.nodes.iter_mut().zip(shift) {
            if d != Vec2::ZERO {
                node.point = round_to_grid(node.point + d, grid);
            }
        }
    }
    out
}

/// `contours` with every point mapped by `f` and rounded to `grid`.
fn map_points(contours: &[Contour], grid: f64, f: impl Fn(Point) -> Point) -> Vec<Contour> {
    let mut out = contours.to_vec();
    for node in out.iter_mut().flat_map(|c| c.nodes.iter_mut()) {
        node.point = round_to_grid(f(node.point), grid);
    }
    out
}

fn finite(values: &[(&str, f64)]) -> Result<(), PathOpError> {
    match values.iter().find(|(_, v)| !v.is_finite()) {
        Some((name, v)) => Err(PathOpError::Invalid(format!("{name} must be a finite number, got {v}"))),
        None => Ok(()),
    }
}

/// The vertical stems' horizontal extents, overlapping ones joined, left to right.
fn vertical_extents(found: &[Found]) -> Vec<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = found
        .iter()
        .filter(|f| f.stem.direction == StemDirection::Vertical)
        .map(|f| {
            let xs = [f.stem.first.p0.x, f.stem.first.p1.x, f.stem.second.p0.x, f.stem.second.p1.x];
            (xs.iter().copied().fold(f64::INFINITY, f64::min), xs.iter().copied().fold(f64::NEG_INFINITY, f64::max))
        })
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (l, r) in spans {
        match out.last_mut() {
            Some(last) if l <= last.1 => last.1 = last.1.max(r),
            _ => out.push((l, r)),
        }
    }
    out
}

/// The piecewise-linear map through `points` (old, new), with `slope` beyond both ends.
fn piecewise(points: &[(f64, f64)], slope: f64, x: f64) -> f64 {
    let (Some(&(x0, y0)), Some(&(xn, yn))) = (points.first(), points.last()) else { return x };
    if x <= x0 {
        return y0 + (x - x0) * slope;
    }
    if x >= xn {
        return yn + (x - xn) * slope;
    }
    for w in points.windows(2) {
        if let [(a, fa), (b, fb)] = *w
            && a <= x
            && x <= b
            && b > a
        {
            return fa + (fb - fa) * (x - a) / (b - a);
        }
    }
    x
}

// ── Adjust Stems ────────────────────────────────────────────────────────────────────────────────

fn check_rules(rules: &[StemRule], which: &str) -> Result<(), PathOpError> {
    if rules.len() > MAX_RULES {
        return Err(PathOpError::Invalid(format!("{} {which} rules; at most {MAX_RULES} are allowed", rules.len())));
    }
    for r in rules {
        finite(&[("from", r.from), ("to", r.to), ("set", r.set.unwrap_or(0.0)), ("offset", r.offset.unwrap_or(0.0))])?;
        if r.from < 0.0 || r.to < r.from {
            return Err(PathOpError::Invalid(format!("a {which} rule's range must run from 0 or more up to `to`, got {} to {}", r.from, r.to)));
        }
        match (r.set, r.offset) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(PathOpError::Invalid(format!("a {which} rule takes `set` or `offset`, one of them")));
            }
            (Some(set), None) if set < MIN_STEM_WIDTH => {
                return Err(PathOpError::Invalid(format!("`set` must be at least {MIN_STEM_WIDTH}, got {set}")));
            }
            (None, Some(offset)) if offset.abs() > MAX_FILTER_STEM => {
                return Err(PathOpError::Invalid(format!("`offset` must be within ±{MAX_FILTER_STEM}, got {offset}")));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The ring nodes (contour, node) that carry one side of a stem running `direction`.
fn side_nodes(side: &Side, parts: &[Part], direction: StemDirection) -> Vec<(usize, usize)> {
    let node = |part: &Part, start: bool| (if start { part.start } else { part.end }).map(|j| (part.contour, j));
    match *side {
        Side::Edge(p) => parts.get(p).map(|part| [node(part, true), node(part, false)].into_iter().flatten().collect()).unwrap_or_default(),
        Side::At(p, t) => {
            let Some(part) = parts.get(p) else { return Vec::new() };
            if t <= 0.01 {
                return node(part, true).into_iter().collect();
            }
            if t >= 0.99 {
                return node(part, false).into_iter().collect();
            }
            let run = if direction == StemDirection::Vertical { Vec2::new(0.0, 1.0) } else { Vec2::new(1.0, 0.0) };
            let parallel = |at: f64| stems::tangent(&part.seg, at).is_some_and(|tau| tau.cross(run).abs() <= ROUND_NODE_DEGREES.to_radians().sin());
            let order = if t < 0.5 { [true, false] } else { [false, true] };
            order
                .into_iter()
                .find(|&start| parallel(if start { 0.0 } else { 1.0 }) && node(part, start).is_some())
                .and_then(|start| node(part, start))
                .into_iter()
                .collect()
        }
    }
}

/// Changes the widths of the horizontal and vertical stems that `options`' rules name.
pub fn adjust_stems(contours: &[Contour], options: &AdjustStems) -> Result<Adjusted, PathOpError> {
    check_rules(&options.vertical, "vertical")?;
    check_rules(&options.horizontal, "horizontal")?;
    let all = || options.vertical.iter().chain(&options.horizontal);
    let Some(widest) = all().map(|r| r.to).reduce(f64::max) else {
        return Ok(Adjusted { contours: contours.to_vec(), stems: 0 });
    };
    let narrowest = all().map(|r| r.from).fold(f64::INFINITY, f64::min);
    let loaded = load(contours)?;
    let found = stems::find(
        &loaded.parts,
        &loaded.ink,
        &StemOptions { min_width: narrowest.max(0.0), max_width: widest.min(MAX_FILTER_STEM) + 1e-6, ..StemOptions::default() },
    );
    let mut acc: Vec<Vec<Sums>> = loaded.rings.iter().map(|r| vec![Sums::default(); r.as_ref().map_or(0, Ring::len)]).collect();
    let mut changed = 0;
    for f in &found {
        let (rules, vertical) = match f.stem.direction {
            StemDirection::Vertical => (&options.vertical, true),
            StemDirection::Horizontal => (&options.horizontal, false),
            StemDirection::Diagonal => continue,
        };
        let w = f.stem.width;
        let Some(rule) = rules.iter().find(|r| w >= r.from - 1e-6 && w <= r.to + 1e-6) else { continue };
        let new = rule.set.unwrap_or(w + rule.offset.unwrap_or(0.0));
        if new < MIN_STEM_WIDTH {
            return Err(PathOpError::Invalid(format!("a stem {} units wide would become {} units wide", round2(w), round2(new))));
        }
        let change = new - w;
        if change.abs() < 1e-9 {
            continue;
        }
        let moves = if vertical {
            [-change / 2.0, change / 2.0]
        } else {
            let on = |l: &Line| {
                let y = (l.p0.y + l.p1.y) / 2.0;
                options.fixed_heights.iter().any(|h| (h - y).abs() <= ON_LINE)
            };
            match (on(&f.stem.first), on(&f.stem.second)) {
                (true, true) => continue,
                (true, false) => [0.0, change],
                (false, true) => [-change, 0.0],
                (false, false) => [-change / 2.0, change / 2.0],
            }
        };
        let nodes: Vec<Vec<(usize, usize)>> =
            f.sides.iter().map(|list| list.iter().flat_map(|s| side_nodes(s, &loaded.parts, f.stem.direction)).collect()).collect();
        if nodes.iter().any(Vec::is_empty) {
            continue;
        }
        for (list, by) in nodes.iter().zip(moves) {
            for &(c, j) in list {
                if let Some(slot) = acc.get_mut(c).and_then(|r| r.get_mut(j)) {
                    if vertical {
                        slot.x += by;
                        slot.nx += 1;
                    } else {
                        slot.y += by;
                        slot.ny += 1;
                    }
                }
            }
        }
        changed += 1;
    }
    let moves: Vec<Vec<Vec2>> = acc.iter().map(|r| r.iter().map(Sums::mean).collect()).collect();
    Ok(Adjusted { contours: apply_moves(contours, &loaded.rings, &moves, options.grid), stems: changed })
}

/// The moves one node gets from the stem edges it lies on: sums and counts per axis.
#[derive(Clone, Copy, Debug, Default)]
struct Sums {
    x: f64,
    nx: u32,
    y: f64,
    ny: u32,
}

impl Sums {
    fn mean(&self) -> Vec2 {
        let mean = |sum: f64, n: u32| if n > 0 { sum / f64::from(n) } else { 0.0 };
        Vec2::new(mean(self.x, self.nx), mean(self.y, self.ny))
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ── Change Weight ───────────────────────────────────────────────────────────────────────────────

/// Makes every stem `amount` thicker (see the module table).
pub fn change_weight(contours: &[Contour], options: &ChangeWeight) -> Result<Reshaped, PathOpError> {
    let o = options;
    finite(&[("amount", o.amount), ("bottom", o.bottom), ("top", o.top), ("serif height", o.serif_height)])?;
    if o.amount.abs() > MAX_WEIGHT_CHANGE {
        return Err(PathOpError::Invalid(format!("the amount must be within ±{MAX_WEIGHT_CHANGE}, got {}", o.amount)));
    }
    if o.method == WeightMethod::Lcg && o.top <= o.bottom {
        return Err(PathOpError::Invalid(format!("the top zone ({}) must be above the bottom zone ({})", o.top, o.bottom)));
    }
    if o.serif_height < 0.0 {
        return Err(PathOpError::Invalid(format!("the serif height must be 0 or more, got {}", o.serif_height)));
    }
    let loaded = load(contours)?;
    let h = o.amount / 2.0;
    let mut budget = SIDE_WORK;
    let cost = loaded.ink.elements().len().max(1);
    let mut moves = Vec::with_capacity(loaded.rings.len());
    for (ci, ring) in loaded.rings.iter().enumerate() {
        let Some(ring) = ring else {
            moves.push(Vec::new());
            continue;
        };
        let ink_left = ink_on_left(ci, ring, &loaded, &mut budget, cost);
        let outward = |t: Vec2| if ink_left { Vec2::new(t.y, -t.x) } else { Vec2::new(-t.y, t.x) };
        let by = (0..ring.len())
            .map(|j| {
                let (a, b) = (ring.tangent_in(j).map(outward), ring.tangent_out(j).map(outward));
                let (na, nb) = match (a, b) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(a), None) => (a, a),
                    (None, Some(b)) => (b, b),
                    (None, None) => return Vec2::ZERO,
                };
                let sum = na + nb;
                let m = if sum.hypot() < 1e-6 { na } else { sum.normalize() };
                m * (h / m.dot(na).max(MIN_MITER_COS))
            })
            .collect();
        moves.push(by);
    }
    let offset = apply_moves(contours, &loaded.rings, &moves, 0.0);

    let extents = if o.counters == CounterMode::Retain {
        let found = stems::find(
            &loaded.parts,
            &loaded.ink,
            &StemOptions { direction: Some(StemDirection::Vertical), max_width: MAX_FILTER_STEM, ..StemOptions::default() },
        );
        vertical_extents(&found)
    } else {
        Vec::new()
    };
    // Stem `i` moves right by `amount * i`; the counters between stems stretch to match.
    let mut spread: Vec<(f64, f64)> = Vec::with_capacity(extents.len() * 2);
    for (i, (l, r)) in extents.iter().enumerate() {
        let d = o.amount * i as f64;
        spread.push((*l, d));
        spread.push((*r, d));
    }
    let width_change = o.amount * extents.len().saturating_sub(1) as f64;
    let (s, bottom, top) = (o.serif_height.min((o.top - o.bottom) / 2.0), o.bottom, o.top);
    let vertical = |y: f64| -> f64 {
        if o.method != WeightMethod::Lcg {
            return 0.0;
        }
        let (lo, hi) = (bottom + s, top - s);
        if y <= lo {
            h
        } else if y >= hi {
            -h
        } else {
            h - 2.0 * h * (y - lo) / (hi - lo)
        }
    };
    let spread_map: Vec<(f64, f64)> = spread.iter().map(|&(x, d)| (x, x + d)).collect();
    // Beyond the first and last stems the slope is 1: everything there moves with that stem.
    let horizontal = |x: f64| -> f64 { if spread_map.is_empty() { 0.0 } else { piecewise(&spread_map, 1.0, x) - x } };
    let mut out = offset;
    for (moved, original) in out.iter_mut().zip(contours) {
        for (node, before) in moved.nodes.iter_mut().zip(&original.nodes) {
            let p = node.point + Vec2::new(horizontal(before.point.x), vertical(before.point.y));
            node.point = round_to_grid(p, o.grid);
        }
    }
    if o.cleanup {
        let cleaned = remove_overlap(&out, OverlapOptions { grid: o.grid.max(0.0), open_paths: OpenPaths::Keep })?;
        return Ok(Reshaped { contours: cleaned, width_change, rebuilt: true });
    }
    Ok(Reshaped { contours: out, width_change, rebuilt: false })
}

/// Whether the ink lies left of the direction of travel of ring `ci`: tested beside a few of its
/// pieces, else read from its direction (counter-clockwise outer contours).
fn ink_on_left(ci: usize, ring: &Ring, loaded: &Loaded, budget: &mut usize, cost: usize) -> bool {
    let mut votes = 0i32;
    for part in loaded.parts.iter().filter(|p| p.contour == ci).take(SIDE_SAMPLES) {
        let Some(after) = budget.checked_sub(2 * cost) else { break };
        *budget = after;
        let Some(t) = stems::tangent(&part.seg, 0.5) else { continue };
        let m = part.seg.eval(0.5);
        let left = Vec2::new(-t.y, t.x) * PROBE;
        match (loaded.ink.winding(m + left) != 0, loaded.ink.winding(m - left) != 0) {
            (true, false) => votes += 1,
            (false, true) => votes -= 1,
            _ => {}
        }
    }
    if votes == 0 { ring.area() > 0.0 } else { votes > 0 }
}

// ── Condense / Extend ───────────────────────────────────────────────────────────────────────────

/// Narrows or widens the outline, holding the vertical stems' widths (see the module table).
pub fn condense_extend(contours: &[Contour], options: &CondenseExtend) -> Result<Reshaped, PathOpError> {
    let o = options;
    finite(&[
        ("counter scale", o.counter_scale),
        ("counter add", o.counter_add),
        ("sidebearing scale", o.sidebearing_scale),
        ("sidebearing add", o.sidebearing_add),
        ("italic angle", o.italic_angle),
        ("width", o.width),
    ])?;
    for (name, scale) in [("counter scale", o.counter_scale), ("sidebearing scale", o.sidebearing_scale)] {
        if scale <= 0.0 || scale > MAX_SCALE {
            return Err(PathOpError::Invalid(format!("the {name} must be above 0% and at most {}%, got {}%", MAX_SCALE * 100.0, scale * 100.0)));
        }
    }
    for (name, add) in [("counter add", o.counter_add), ("sidebearing add", o.sidebearing_add)] {
        if add.abs() > MAX_ADD {
            return Err(PathOpError::Invalid(format!("the {name} must be within ±{MAX_ADD}, got {add}")));
        }
    }
    if o.italic_angle.abs() > MAX_ITALIC_ANGLE {
        return Err(PathOpError::Invalid(format!("the italic angle must be within ±{MAX_ITALIC_ANGLE}°, got {}", o.italic_angle)));
    }
    let slant = o.italic_angle.to_radians().tan();
    let upright = map_points(contours, 0.0, |p| Point::new(p.x - p.y * slant, p.y));
    let loaded = load(&upright)?;
    let mut bounds: Option<Rect> = None;
    for c in &upright {
        let path = c.to_bez()?;
        if path.elements().len() < 2 {
            continue;
        }
        let b = path.bounding_box();
        bounds = Some(bounds.map_or(b, |r| r.union(b)));
    }
    let Some(bounds) = bounds else {
        return Ok(Reshaped { contours: contours.to_vec(), width_change: 0.0, rebuilt: false });
    };
    let (xmin, xmax) = (bounds.x0, bounds.x1);
    let found = stems::find(
        &loaded.parts,
        &loaded.ink,
        &StemOptions { direction: Some(StemDirection::Vertical), max_width: MAX_FILTER_STEM, ..StemOptions::default() },
    );
    let stems: Vec<(f64, f64)> = vertical_extents(&found).into_iter().map(|(l, r)| (l.max(xmin), r.min(xmax))).filter(|(l, r)| r > l).collect();

    let lsb = xmin;
    let rsb = o.width - xmax;
    let mut points = vec![(xmin, lsb * o.sidebearing_scale + o.sidebearing_add)];
    let mut at = xmin;
    for (i, &(l, r)) in stems.iter().enumerate() {
        let gap = (l - at) * o.counter_scale + if i > 0 { o.counter_add } else { 0.0 };
        let new = points.last().map_or(0.0, |p| p.1) + gap.max(0.0);
        points.push((l, new));
        points.push((r, new + (r - l)));
        at = r;
    }
    let tail = points.last().map_or(0.0, |p| p.1) + (xmax - at) * o.counter_scale;
    points.push((xmax, tail));
    let new_width = tail + rsb * o.sidebearing_scale + o.sidebearing_add;
    let mapped = map_points(&upright, 0.0, |p| {
        let x = piecewise(&points, o.counter_scale, p.x);
        Point::new(x + p.y * slant, p.y)
    });
    let out = map_points(&mapped, o.grid, |p| p);
    Ok(Reshaped { contours: out, width_change: new_width - o.width, rebuilt: false })
}

// ── Change x-Height ─────────────────────────────────────────────────────────────────────────────

/// Moves the x-height from `from` to `to` (see the module table).
pub fn change_x_height(contours: &[Contour], options: &ChangeXHeight) -> Result<Vec<Contour>, PathOpError> {
    let o = options;
    finite(&[("from", o.from), ("to", o.to), ("serif height", o.serif_height), ("ascender", o.ascender)])?;
    let s = o.serif_height;
    if s < 0.0 {
        return Err(PathOpError::Invalid(format!("the serif height must be 0 or more, got {s}")));
    }
    if o.from - 2.0 * s <= 0.0 || o.to - 2.0 * s <= 0.0 {
        return Err(PathOpError::Invalid(format!("both x-heights must be more than twice the serif height ({s}): from {} to {}", o.from, o.to)));
    }
    let k = (o.to - 2.0 * s) / (o.from - 2.0 * s);
    let shift = o.to - o.from;
    let a = o.ascender;
    let keeps_ascender = a > o.from + s && a > o.to + s;
    let map = |y: f64| -> f64 {
        if y <= s {
            y
        } else if y < o.from - s {
            s + (y - s) * k
        } else if y <= o.from + s {
            y + shift
        } else if keeps_ascender && y < a {
            (o.to + s) + (y - (o.from + s)) * (a - (o.to + s)) / (a - (o.from + s))
        } else if keeps_ascender {
            y
        } else {
            y + shift
        }
    };
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    Ok(map_points(contours, o.grid, |p| Point::new(p.x, map(p.y))))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::contours_from_bez;
    use crate::stems::detect_stems;

    fn h() -> Vec<Contour> {
        let mut p = BezPath::new();
        let pts = [
            (0.0, 0.0),
            (80.0, 0.0),
            (80.0, 300.0),
            (520.0, 300.0),
            (520.0, 0.0),
            (600.0, 0.0),
            (600.0, 700.0),
            (520.0, 700.0),
            (520.0, 360.0),
            (80.0, 360.0),
            (80.0, 700.0),
            (0.0, 700.0),
        ];
        for (i, &(x, y)) in pts.iter().enumerate() {
            if i == 0 {
                p.move_to((x, y));
            } else {
                p.line_to((x, y));
            }
        }
        p.close_path();
        contours_from_bez(&p)
    }

    fn outline(contours: &[Contour]) -> BezPath {
        let mut p = BezPath::new();
        for c in contours {
            p.extend(c.to_bez().unwrap());
        }
        p
    }

    fn widths(contours: &[Contour], direction: StemDirection) -> Vec<f64> {
        let mut w: Vec<f64> = detect_stems(&outline(contours), &StemOptions::default())
            .iter()
            .filter(|s| s.direction == direction)
            .map(|s| (s.width * 1e6).round() / 1e6)
            .collect();
        w.sort_by(f64::total_cmp);
        w
    }

    #[test]
    fn rules_pick_stems_by_width_and_horizontal_edges_on_fixed_heights_stay() {
        let rule = |from, to, set, offset| StemRule { from, to, set, offset };
        let out = adjust_stems(
            &h(),
            &AdjustStems {
                vertical: vec![rule(70.0, 90.0, None, Some(20.0))],
                horizontal: vec![rule(50.0, 70.0, Some(40.0), None)],
                ..AdjustStems::default()
            },
        )
        .unwrap();
        assert_eq!(out.stems, 3);
        assert_eq!(widths(&out.contours, StemDirection::Vertical), vec![100.0, 100.0]);
        assert_eq!(widths(&out.contours, StemDirection::Horizontal), vec![40.0]);
        let xs: Vec<f64> = out.contours[0].nodes.iter().map(|n| n.point.x).collect();
        assert!(xs.contains(&-10.0) && xs.contains(&90.0) && xs.contains(&610.0), "both edges move by half: {xs:?}");

        // Out of range: nothing changes.
        let none = adjust_stems(&h(), &AdjustStems { vertical: vec![rule(100.0, 200.0, Some(50.0), None)], ..AdjustStems::default() }).unwrap();
        assert_eq!((none.stems, none.contours), (0, h()));

        // A crossbar whose lower edge lies on a fixed height keeps that edge.
        let fixed = adjust_stems(
            &h(),
            &AdjustStems { horizontal: vec![rule(60.0, 60.0, None, Some(20.0))], fixed_heights: vec![300.0], ..AdjustStems::default() },
        )
        .unwrap();
        let ys: Vec<f64> = fixed.contours[0].nodes.iter().map(|n| n.point.y).collect();
        assert!(ys.contains(&300.0) && ys.contains(&380.0), "{ys:?}");

        assert!(adjust_stems(&h(), &AdjustStems { vertical: vec![rule(70.0, 90.0, None, Some(-85.0))], ..AdjustStems::default() }).is_err());
        assert!(adjust_stems(&h(), &AdjustStems { vertical: vec![rule(70.0, 90.0, Some(5.0), Some(1.0))], ..AdjustStems::default() }).is_err());
        assert!(adjust_stems(&h(), &AdjustStems { vertical: vec![rule(90.0, 70.0, Some(5.0), None)], ..AdjustStems::default() }).is_err());
    }

    #[test]
    fn change_weight_lcg_keeps_the_zones_and_retain_keeps_the_counter() {
        let o = ChangeWeight {
            amount: 20.0,
            method: WeightMethod::Lcg,
            bottom: 0.0,
            top: 700.0,
            serif_height: 0.0,
            counters: CounterMode::Squish,
            cleanup: false,
            grid: 0.0,
        };
        let out = change_weight(&h(), &o).unwrap();
        let b = outline(&out.contours).bounding_box();
        assert!((b.y0 - 0.0).abs() < 1e-9 && (b.y1 - 700.0).abs() < 1e-9, "{b:?}");
        assert!((b.x0 + 10.0).abs() < 1e-9 && (b.x1 - 610.0).abs() < 1e-9, "{b:?}");
        assert_eq!(widths(&out.contours, StemDirection::Vertical), vec![100.0, 100.0]);
        assert_eq!(out.width_change, 0.0);

        let cjk = change_weight(&h(), &ChangeWeight { method: WeightMethod::Cjk, ..o.clone() }).unwrap();
        let b = outline(&cjk.contours).bounding_box();
        assert!((b.y0 + 10.0).abs() < 1e-9 && (b.y1 - 710.0).abs() < 1e-9, "cjk only expands: {b:?}");

        let retain = change_weight(&h(), &ChangeWeight { counters: CounterMode::Retain, grid: 1.0, ..o.clone() }).unwrap();
        assert_eq!(retain.width_change, 20.0);
        let xs: Vec<f64> = retain.contours[0].nodes.iter().map(|n| n.point.x).collect();
        assert!(xs.contains(&90.0) && xs.contains(&530.0), "the counter stays 440 wide: {xs:?}");

        assert!(change_weight(&h(), &ChangeWeight { top: 0.0, ..o.clone() }).is_err());
        assert!(change_weight(&h(), &ChangeWeight { amount: f64::NAN, ..o }).is_err());
    }

    #[test]
    fn condense_keeps_stem_widths_and_scales_the_counter() {
        let o = CondenseExtend {
            counter_scale: 0.8,
            counter_add: 0.0,
            sidebearing_scale: 1.0,
            sidebearing_add: 10.0,
            italic_angle: 0.0,
            width: 700.0,
            grid: 1.0,
        };
        let out = condense_extend(&h(), &o).unwrap();
        for w in widths(&out.contours, StemDirection::Vertical) {
            assert!((w - 80.0).abs() <= 1.0, "{w}");
        }
        let b = outline(&out.contours).bounding_box();
        assert_eq!((b.x0, b.x1), (10.0, 10.0 + 80.0 + 352.0 + 80.0));
        assert!((out.width_change - (10.0 + 512.0 + 110.0 - 700.0)).abs() < 1e-9);

        // Slanted by 10 degrees, then condensed: the stems still keep their widths.
        let slant = 10f64.to_radians().tan();
        let leaning = map_points(&h(), 0.0, |p| Point::new(p.x + p.y * slant, p.y));
        let out = condense_extend(&leaning, &CondenseExtend { italic_angle: 10.0, grid: 0.0, ..o.clone() }).unwrap();
        let upright = map_points(&out.contours, 0.0, |p| Point::new(p.x - p.y * slant, p.y));
        assert_eq!(widths(&upright, StemDirection::Vertical).iter().map(|w| w.round()).collect::<Vec<_>>(), vec![80.0, 80.0]);

        assert!(condense_extend(&h(), &CondenseExtend { counter_scale: 0.0, ..o }).is_err());
    }

    #[test]
    fn change_x_height_keeps_serifs_and_the_ascender() {
        // Nodes at the baseline serif (10), mid x-height (250), the x-height (500), between it and
        // the ascender (600) and on the ascender (750).
        let c = Contour::new(
            true,
            [(0.0, 10.0), (50.0, 250.0), (100.0, 500.0), (150.0, 600.0), (200.0, 750.0)]
                .iter()
                .map(|&(x, y)| crate::Node::new(Point::new(x, y), crate::NodeKind::Line))
                .collect(),
        );
        let out = change_x_height(&[c], &ChangeXHeight { from: 500.0, to: 540.0, serif_height: 20.0, ascender: 750.0, grid: 0.0 }).unwrap();
        let ys: Vec<f64> = out[0].nodes.iter().map(|n| n.point.y).collect();
        assert_eq!(ys[0], 10.0, "within the serif height of the baseline nothing moves");
        assert!((ys[1] - (20.0 + 230.0 * 500.0 / 460.0)).abs() < 1e-9, "{ys:?}");
        assert_eq!(ys[2], 540.0);
        assert!(ys[3] > 600.0 && ys[3] < 750.0, "{ys:?}");
        assert_eq!(ys[4], 750.0);
        assert!(change_x_height(&[], &ChangeXHeight { from: 30.0, to: 540.0, serif_height: 20.0, ascender: 750.0, grid: 0.0 }).is_err());
    }
}

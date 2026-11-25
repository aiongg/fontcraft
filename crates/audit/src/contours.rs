//! Checks of whole contours: overlaps, direction, open, stray, tiny, empty, repeated and
//! zero-area paths, start points, curve types and open corners.

use std::collections::{BTreeMap, HashMap};

use fontcraft_geom::{BezPath, PathEl, Point, Rect, Shape as _};
use fontcraft_model::{NodeKind, Shape};

use crate::contour::{Bez, MAX_NESTING_CONTOURS, ZERO_AREA_TOLERANCE, cross, nesting, overlaps, polylines_cross, proper_cross, regions};
use crate::{CurveType, Hit, LayerInput, MAX_LAYER_NODES, Nth, Params, Severity, Sink, Threshold, num};

/// Most pairs of line pieces the overlap test compares on one layer.
pub(crate) const MAX_OVERLAP_TESTS: usize = 5_000_000;
/// Most on-curve points a contour may have to count as tiny (from the reference tool).
pub(crate) const TINY_MAX_ON_CURVE: usize = 4;

pub(crate) const TINY_MIN_AREA: Threshold = Threshold {
    name: "minArea",
    default: 25.0,
    min: 0.0,
    max: 1_000_000.0,
    doc: "A contour of at most four on-curve points whose bounding box is smaller than this many square units is tiny.",
};

pub(crate) const OPEN_CORNER_MAX_LENGTH: Threshold = Threshold {
    name: "maxLength",
    default: 100.0,
    min: 0.0,
    max: 10_000.0,
    doc: "Longest segment, in units, between the two crossing segments of an open corner.",
};

/// One straight piece of an outline, for the overlap test.
struct Edge {
    shape: usize,
    /// Unique per contour within the layer.
    contour: usize,
    component: bool,
    a: Point,
    b: Point,
    bbox: Rect,
}

fn edge(shape: usize, contour: usize, component: bool, a: Point, b: Point) -> Edge {
    Edge { shape, contour, component, a, b, bbox: Rect::from_points(a, b) }
}

/// Contours that cross each other or themselves, and contours inside another one that runs the
/// same way. Components take part as their outlines; their own contours are not compared with
/// each other. A decoded variable TrueType font is skipped: it keeps its overlaps on purpose, as
/// merged contours would not interpolate.
pub(crate) fn overlap(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    if input.ctx.glyf_outlines && input.ctx.variable {
        sink.skip("a variable TrueType font keeps its overlaps");
        return;
    }
    let mut edges: Vec<Edge> = Vec::new();
    let mut contour_id = 0usize;
    for c in input.finite().filter(|c| c.closed()) {
        contour_id += 1;
        for seg in c.segs.iter().filter(|s| s.is_valid()) {
            for w in seg.polyline().windows(2) {
                if let [a, b] = w
                    && a != b
                {
                    edges.push(edge(c.shape, contour_id, false, *a, *b));
                }
            }
        }
    }
    let mut budget = MAX_LAYER_NODES;
    for (i, shape) in input.layer.shapes.iter().enumerate() {
        let Shape::Component(comp) = shape else { continue };
        let Ok(outline) = input.ctx.font.component_outline(comp, &input.layer.master) else { continue };
        let count = outline.elements().len();
        if count > budget {
            sink.skip(format!("the components' outlines have more than {MAX_LAYER_NODES} segments"));
            return;
        }
        budget -= count;
        push_outline(&outline, i, &mut contour_id, &mut edges);
    }
    edges.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
    let mut crossings: BTreeMap<(usize, usize), Point> = BTreeMap::new();
    let mut tests = 0usize;
    'outer: for (i, e) in edges.iter().enumerate() {
        for f in edges.iter().skip(i + 1) {
            if f.bbox.x0 > e.bbox.x1 {
                break;
            }
            tests += 1;
            if tests > MAX_OVERLAP_TESTS {
                sink.skip(format!("the outline needs more than {MAX_OVERLAP_TESTS} comparisons"));
                break 'outer;
            }
            if f.bbox.y0 > e.bbox.y1 || f.bbox.y1 < e.bbox.y0 {
                continue;
            }
            if e.shape == f.shape && (e.component || e.contour != f.contour) {
                continue;
            }
            if let Some(x) = proper_cross(e.a, e.b, f.a, f.b) {
                crossings.entry((e.shape.min(f.shape), e.shape.max(f.shape))).or_insert(x);
            }
        }
    }
    for ((a, b), x) in crossings {
        let (message, shapes) = if a == b {
            (format!("Path {a} crosses itself.", a = Nth(a)), vec![a])
        } else {
            (format!("Shapes {a} and {b} cross.", a = Nth(a), b = Nth(b)), vec![a, b])
        };
        sink.push(Hit::new("crossing", message).shapes(shapes).at(x));
    }
    let regions = regions(&input.contours);
    if regions.len() > MAX_NESTING_CONTOURS {
        return;
    }
    for (inner, (_, parent)) in regions.iter().zip(nesting(&regions)) {
        let Some(outer) = parent.and_then(|j| regions.get(j)) else { continue };
        if (outer.area > 0.0) == (inner.area > 0.0) {
            sink.push(
                Hit::new(
                    "nested-same-direction",
                    format!("Path {} lies inside path {} and runs the same way, so the two overlap.", Nth(inner.shape), Nth(outer.shape)),
                )
                .shapes(vec![outer.shape, inner.shape])
                .bounds(inner.bbox),
            );
        }
    }
}

fn push_outline(outline: &BezPath, shape: usize, contour_id: &mut usize, edges: &mut Vec<Edge>) {
    let (mut start, mut last) = (None::<Point>, None::<Point>);
    let mut pts = Vec::new();
    let add = |pts: &[Point], id: usize, edges: &mut Vec<Edge>| {
        for w in pts.windows(2) {
            if let [a, b] = w
                && a != b
                && a.is_finite()
                && b.is_finite()
            {
                edges.push(edge(shape, id, true, *a, *b));
            }
        }
    };
    for el in outline.elements() {
        pts.clear();
        match *el {
            PathEl::MoveTo(p) => {
                *contour_id += 1;
                start = Some(p);
                last = Some(p);
            }
            PathEl::LineTo(p) => {
                if let Some(l) = last {
                    Bez::Line(l, p).polyline(&mut pts);
                }
                last = Some(p);
            }
            PathEl::QuadTo(c, p) => {
                if let Some(l) = last {
                    Bez::Quad(l, c, p).polyline(&mut pts);
                }
                last = Some(p);
            }
            PathEl::CurveTo(c1, c2, p) => {
                if let Some(l) = last {
                    Bez::Cubic(l, c1, c2, p).polyline(&mut pts);
                }
                last = Some(p);
            }
            PathEl::ClosePath => {
                if let (Some(l), Some(s)) = (last, start) {
                    Bez::Line(l, s).polyline(&mut pts);
                }
                last = start;
            }
        }
        add(&pts, *contour_id, edges);
    }
}

/// A contour of at most four on-curve points with a bounding box smaller than `minArea`.
pub(crate) fn tiny_contour(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let min = p.get(&TINY_MIN_AREA);
    for c in input.finite() {
        let on = c.on_curve_count();
        if !(2..=TINY_MAX_ON_CURVE).contains(&on) {
            continue;
        }
        let Some(b) = c.bbox() else { continue };
        let area = b.width() * b.height();
        if area < min {
            sink.push(
                Hit::new("tiny", format!("Path {} covers only {} square units.", Nth(c.shape), num(area)))
                    .shapes(vec![c.shape])
                    .bounds(b)
                    .at(b.center())
                    .data("area", area),
            );
        }
    }
}

/// Code points of the glyphs that are meant to be empty.
const SPACE_CODEPOINTS: &[u32] = &[0x20, 0xA0, 0x1680, 0x180E, 0x202F, 0x205F, 0x3000, 0xFEFF];
const SPACE_RANGE: std::ops::RangeInclusive<u32> = 0x2000..=0x200B;

fn is_space_codepoint(u: u32) -> bool {
    SPACE_CODEPOINTS.contains(&u) || SPACE_RANGE.contains(&u)
}

/// Is the glyph a space: mapped from a space code point, or named like one (`space`,
/// `nbspace`, `uni00A0`, with any suffix)?
fn is_space(glyph: &fontcraft_model::Glyph) -> bool {
    if glyph.unicodes.iter().any(|u| is_space_codepoint(*u)) {
        return true;
    }
    let base = glyph.name.split('.').next().unwrap_or("");
    if base == "space" || base == "nbspace" {
        return true;
    }
    base.strip_prefix("uni").filter(|h| h.len() == 4).and_then(|h| u32::from_str_radix(h, 16).ok()).is_some_and(is_space_codepoint)
}

/// A layer with neither paths nor components, in a glyph that is not a space.
pub(crate) fn no_contours(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let has_ink = input.layer.shapes.iter().any(|s| matches!(s, Shape::Path(_) | Shape::Component(_) | Shape::Raw(_)));
    if !has_ink && !is_space(input.glyph) {
        sink.push(Hit::new("no-contours", format!("Glyph {} has no paths and no components in this layer.", input.glyph.name)));
    }
}

/// Outer contours that run clockwise, and contours that run the same way as the one around them.
/// A contour inside another one that also crosses a contour is part of an overlapping shape (a
/// crossbar drawn over a bowl and its counter), not a counter; the overlap check reports it.
pub(crate) fn path_direction(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let regions = regions(&input.contours);
    if regions.len() > MAX_NESTING_CONTOURS {
        sink.skip(format!("the layer has more than {MAX_NESTING_CONTOURS} closed paths"));
        return;
    }
    for (r, (depth, _)) in regions.iter().zip(nesting(&regions)) {
        let counter_clockwise = r.area > 0.0;
        // Sources wind outer contours counter-clockwise; a compiled glyf table winds them clockwise.
        let expected = (depth % 2 == 0) != input.ctx.glyf_outlines;
        if counter_clockwise == expected || (depth > 0 && regions.iter().any(|other| cross(r, other))) {
            continue;
        }
        // The codes name the source convention; on glyf outlines both directions are reversed.
        let wrong = if input.ctx.glyf_outlines { "counter-clockwise" } else { "clockwise" };
        let (code, message) = match depth {
            0 => ("outer-clockwise", format!("Path {} is an outer contour but runs {wrong}.", Nth(r.shape))),
            d if d % 2 == 1 => {
                ("counter-same-direction", format!("Path {} is a counter but runs the same way as the contour around it.", Nth(r.shape)))
            }
            _ => ("nested-clockwise", format!("Path {} lies inside a counter but runs {wrong}.", Nth(r.shape))),
        };
        sink.push(Hit::new(code, message).shapes(vec![r.shape]).bounds(r.bbox).data("depth", depth as f64));
    }
}

/// An open path.
pub(crate) fn open_path(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.contours.iter().filter(|c| !c.closed()) {
        let n = c.path.nodes.len();
        let mut ends = Vec::new();
        if n > 0 {
            ends.push([c.shape, 0]);
        }
        if n > 1 {
            ends.push([c.shape, n - 1]);
        }
        let mut hit = Hit::new("open-path", format!("Path {} is open.", Nth(c.shape))).shapes(vec![c.shape]).points(ends);
        if let Some(first) = c.path.nodes.first().filter(|_| c.finite) {
            hit = hit.at(first.point());
        }
        sink.push(hit);
    }
}

/// A path of one node, of no nodes, or of off-curve nodes that make no shape.
pub(crate) fn stray_point(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in &input.contours {
        let nodes = &c.path.nodes;
        let (code, message) = match nodes.len() {
            0 => ("empty", format!("Path {} has no nodes.", Nth(c.shape))),
            1 => ("single-node", format!("Path {} is a single node.", Nth(c.shape))),
            n if c.on_curve_count() == 0 && (!c.closed() || n < 3) => {
                ("off-curve-only", format!("Path {} has only off-curve nodes and makes no shape.", Nth(c.shape)))
            }
            _ => continue,
        };
        let mut hit = Hit::new(code, message).shapes(vec![c.shape]).points((0..nodes.len()).map(|i| [c.shape, i]).collect());
        if let Some(first) = nodes.first().filter(|_| c.finite) {
            hit = hit.at(first.point());
        }
        sink.push(hit);
    }
}

/// A closed contour that encloses no area.
pub(crate) fn zero_area_contour(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.finite().filter(|c| c.closed()) {
        if c.on_curve_count() < 2 && c.path.nodes.len() < 3 {
            continue;
        }
        let Some(bez) = c.bez() else { continue };
        if bez.elements().is_empty() {
            continue;
        }
        let area = bez.area();
        if area.is_finite() && area.abs() < ZERO_AREA_TOLERANCE {
            let mut hit = Hit::new("zero-area", format!("Path {} encloses no area.", Nth(c.shape))).shapes(vec![c.shape]);
            if let Some(b) = c.bbox() {
                hit = hit.bounds(b).at(b.center());
            }
            sink.push(hit);
        }
    }
}

/// The points of a path as exact bit patterns (with -0 read as 0) and whether each is off-curve,
/// sorted, so that the same points in another order or direction give the same key.
type ContourKey = (bool, Vec<(u64, u64, bool)>);

/// Two paths with the same points, in the same or the opposite direction.
pub(crate) fn duplicate_contour(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let mut seen: HashMap<ContourKey, usize> = HashMap::new();
    for c in input.finite() {
        if c.path.nodes.is_empty() {
            continue;
        }
        let mut pts: Vec<(u64, u64, bool)> = c.path.nodes.iter().map(|n| ((n.x + 0.0).to_bits(), (n.y + 0.0).to_bits(), !n.on_curve())).collect();
        pts.sort_unstable();
        let key = (c.closed(), pts);
        match seen.get(&key) {
            Some(&first) => {
                let mut hit =
                    Hit::new("duplicate", format!("Path {} repeats path {first}.", Nth(c.shape), first = Nth(first))).shapes(vec![first, c.shape]);
                if let Some(b) = c.bbox() {
                    hit = hit.bounds(b);
                }
                sink.push(hit);
            }
            None => {
                seen.insert(key, c.shape);
            }
        }
    }
}

/// A closed path whose first node is not the lowest, then leftmost, on-curve node.
pub(crate) fn start_point_position(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for c in input.finite().filter(|c| c.closed() && c.on_curve_count() >= 2) {
        let Some(start) = c.start_node() else { continue };
        let mut best: Option<(usize, Point)> = None;
        for (i, n) in c.path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()) {
            let p = n.point();
            let better = match best {
                None => true,
                Some((_, b)) => p.y < b.y || (p.y == b.y && p.x < b.x),
            };
            if better {
                best = Some((i, p));
            }
        }
        let (Some((best_i, best_p)), Some(start_n)) = (best, c.path.nodes.get(start)) else { continue };
        if start_n.point() != best_p {
            sink.push(
                Hit::new(
                    "start-point",
                    format!(
                        "Path {} starts at node {start}; the usual start is node {best_i}, the lowest then leftmost.",
                        Nth(c.shape),
                        start = Nth(start),
                        best_i = Nth(best_i)
                    ),
                )
                .shapes(vec![c.shape])
                .points(vec![[c.shape, start]])
                .at(start_n.point())
                .data("proposedNode", best_i as f64),
            );
        }
    }
}

/// Cubic and quadratic curves in one layer, or a layer whose curves are not of the font's main
/// type.
pub(crate) fn mixed_curve_types(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let mut cubic = Vec::new();
    let mut quad = Vec::new();
    for c in &input.contours {
        if c.path.nodes.iter().any(|n| n.kind == NodeKind::Curve) {
            cubic.push(c.shape);
        }
        if c.path.nodes.iter().any(|n| n.kind == NodeKind::QCurve) {
            quad.push(c.shape);
        }
    }
    let main = input.ctx.main_curve;
    match (cubic.is_empty(), quad.is_empty()) {
        (false, false) => {
            // Point at the paths of the less usual type.
            let shapes = if main == Some(CurveType::Quadratic) { cubic } else { quad };
            sink.push(Hit::new("mixed-in-layer", "The layer has both cubic and quadratic curves.").shapes(shapes));
        }
        (false, true) if main == Some(CurveType::Quadratic) => {
            sink.push(
                Hit::new("differs-from-font", "The layer has cubic curves; most of the font's curves are quadratic.")
                    .severity(Severity::Warn)
                    .shapes(cubic),
            );
        }
        (true, false) if main == Some(CurveType::Cubic) => {
            sink.push(
                Hit::new("differs-from-font", "The layer has quadratic curves; most of the font's curves are cubic.")
                    .severity(Severity::Warn)
                    .shapes(quad),
            );
        }
        _ => {}
    }
}

/// Two segments that cross just before a corner, with a short segment between them.
pub(crate) fn open_corners(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let max = p.get(&OPEN_CORNER_MAX_LENGTH);
    for c in input.finite() {
        let n = c.segs.len();
        let enough = if c.closed() { n >= 4 } else { n >= 3 };
        if !enough {
            continue;
        }
        let last = if c.closed() { n } else { n - 2 };
        for k in 0..last {
            let (Some(a), Some(mid), Some(b)) = (c.segs.get(k), c.segs.get((k + 1) % n), c.segs.get((k + 2) % n)) else { continue };
            if !(a.is_valid() && mid.is_valid() && b.is_valid()) || a.end != mid.start || mid.end != b.start {
                continue;
            }
            if mid.length() > max || !overlaps(a.control_box(), b.control_box()) {
                continue;
            }
            if let Some(x) = polylines_cross(&a.polyline(), &b.polyline()) {
                sink.push(
                    Hit::new("open-corner", format!("Path {} has an open corner at nodes {} and {}.", Nth(c.shape), Nth(mid.start), Nth(mid.end)))
                        .shapes(vec![c.shape])
                        .points(vec![[c.shape, mid.start], [c.shape, mid.end]])
                        .segments(vec![[c.shape, mid.end]])
                        .at(x),
                );
            }
        }
    }
}

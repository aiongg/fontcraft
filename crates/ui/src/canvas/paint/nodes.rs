//! Nodes and handles, with the display conventions of `docs/spec/03a` § 4 (View › Show Nodes):
//!
//! | Item | Drawn as |
//! |---|---|
//! | smooth node | circle, `node_smooth` |
//! | corner node | square, `node_corner` |
//! | handle | small circle, `node_handle`, with a `handle_line` to its node |
//! | start node of a closed path | arrowhead in path direction, `node_smooth` or `node_corner` |
//! | ends of an open path | strokes across the path, `open_end`; the first node also a triangle, `open_end_start` |
//! | node on a metric line / in a zone | diamond / circle behind it, `metric_highlight` |
//! | two neighbouring nodes at one place | circle around them, `duplicate_node` |
//! | where paths cross (Show Nodes › Extra Nodes) | small hollow circle, `extra_node` |
//!
//! Node, handle and mark sizes follow `appearance.handleSize` through the tokens.

use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};
use fontcraft_engine::model::{Layer, Path, Shape as ModelShape};
use fontcraft_geom::{NodeType, segments};

use super::super::layers::PaintCx;
use super::super::tokens::Tokens;
use super::screen;
use crate::app::models::canvas::EditedGlyph;

/// How close, in font units, a node must be to a metric line to count as on it. Coordinates are stored as typed, so
/// "on the line" means equal up to rounding noise.
const ON_LINE: f64 = 1e-6;

/// The metric lines and alignment zones nodes are checked against, in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    /// Heights of the horizontal metric lines.
    pub lines: Vec<f64>,
    /// Alignment zones as (bottom, top).
    pub zones: Vec<(f64, f64)>,
}

impl Metrics {
    pub fn of(glyph: &EditedGlyph) -> Metrics {
        Metrics { lines: glyph.lines.iter().map(|(_, y)| *y).collect(), zones: glyph.zones.clone() }
    }
}

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

/// The edited side's nodes and handles under Show Nodes, then its Extra Nodes under Show Nodes › Extra Nodes.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(glyph) = cx.glyph() else { return Vec::new() };
    let mut out = Vec::new();
    if cx.display.nodes {
        out.extend(layer_shapes(cx, &glyph.layer, &Metrics::of(glyph)));
    }
    if cx.display.extra_nodes {
        let t = cx.tokens;
        let ring = Stroke::new(t.handle_line_width * 1.5, t.extra_node);
        out.extend(glyph.extra_nodes.iter().filter_map(|p| screen(cx, *p)).map(|at| Shape::circle_stroke(at, t.extra_node_radius, ring)));
    }
    out
}

/// The shapes for every path of `layer`, back to front.
pub fn layer_shapes(cx: &PaintCx<'_>, layer: &Layer, metrics: &Metrics) -> Vec<Shape> {
    let t = cx.tokens;
    let mut back = Vec::new();
    let mut lines = Vec::new();
    let mut handles = Vec::new();
    let mut nodes = Vec::new();
    let mut marks = Vec::new();
    for shape in &layer.shapes {
        let ModelShape::Path(path) = shape else { continue };
        let screen: Vec<Pos2> = path.nodes.iter().map(|n| cx.camera.to_screen(n.point())).collect();
        handle_lines(path, &screen, t, &mut lines);
        let on_curve: Vec<usize> = path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).map(|(i, _)| i).collect();
        let first = on_curve.first().copied();
        let last = on_curve.last().copied();
        for (i, (node, at)) in path.nodes.iter().zip(&screen).enumerate() {
            let at = *at;
            if !(at.x.is_finite() && at.y.is_finite()) {
                continue;
            }
            if !node.on_curve() {
                handles.push(Shape::circle_filled(at, t.handle_radius, t.node_handle));
                continue;
            }
            highlight(node.y, at, t, metrics, &mut back);
            let fill = if node.smooth { t.node_smooth } else { t.node_corner };
            let direction = direction_from(path, &screen, i, true);
            if path.closed && Some(i) == first {
                match direction {
                    Some(d) => nodes.push(arrow(at, d, t.start_arrow_size, fill)),
                    None => nodes.push(node_mark(at, node.smooth, t)),
                }
                continue;
            }
            nodes.push(node_mark(at, node.smooth, t));
            if !path.closed && (Some(i) == first || Some(i) == last) {
                let along = if Some(i) == first { direction } else { direction_from(path, &screen, i, false).map(|d| -d) };
                if let Some(d) = along {
                    let across = Vec2::new(-d.y, d.x) * (t.open_end_length / 2.0);
                    marks.push(Shape::line_segment([at - across, at + across], Stroke::new(t.handle_line_width * 2.0, t.open_end)));
                    if Some(i) == first {
                        let tip = at + d * (t.node_size + t.start_arrow_size);
                        marks.push(arrow(tip, d, t.start_arrow_size, t.open_end_start));
                    }
                }
            }
        }
        duplicates(path, &screen, &on_curve, t, &mut marks);
    }
    back.extend(lines);
    back.extend(handles);
    back.extend(nodes);
    back.extend(marks);
    back
}

fn node_mark(at: Pos2, smooth: bool, t: &Tokens) -> Shape {
    if smooth {
        Shape::circle_filled(at, t.smooth_node_radius, t.node_smooth)
    } else {
        Shape::rect_filled(Rect::from_center_size(at, Vec2::splat(t.node_size)), 0.0, t.node_corner)
    }
}

/// A filled triangle pointing along `d` (a unit vector), centred on `at`.
fn arrow(at: Pos2, d: Vec2, size: f32, fill: Color32) -> Shape {
    let side = Vec2::new(-d.y, d.x) * (size * 0.45);
    let tip = at + d * (size * 0.6);
    let base = at - d * (size * 0.4);
    Shape::convex_polygon(vec![tip, base + side, base - side], fill, Stroke::NONE)
}

/// The screen direction from node `i` towards the next distinct point of its path (`forward`) or the previous one,
/// wrapping on closed paths.
fn direction_from(path: &Path, screen: &[Pos2], i: usize, forward: bool) -> Option<Vec2> {
    let n = screen.len();
    let from = *screen.get(i)?;
    for step in 1..n {
        let j = if forward {
            match i + step {
                j if j < n => j,
                j if path.closed => j - n,
                _ => return None,
            }
        } else {
            match i.checked_sub(step) {
                Some(j) => j,
                None if path.closed => i + n - step,
                None => return None,
            }
        };
        let to = *screen.get(j)?;
        let d = to - from;
        if d.length() > 1e-3 && d.x.is_finite() && d.y.is_finite() {
            return Some(d.normalized());
        }
    }
    None
}

/// Lines from each handle to the node it belongs to. A cubic segment's first handle belongs to the start node and its
/// second to the end node; a quadratic segment's handles are joined in order, as its control polygon.
fn handle_lines(path: &Path, screen: &[Pos2], t: &Tokens, out: &mut Vec<Shape>) {
    let stroke = Stroke::new(t.handle_line_width, t.handle_line);
    let points = path.node_points();
    for segment in segments(&points, path.closed) {
        if segment.handles.is_empty() {
            continue;
        }
        let at = |i: usize| screen.get(i).copied();
        let cubic = points.get(segment.end).is_some_and(|n| n.kind == NodeType::Curve) && segment.handles.len() == 2;
        let mut chain: Vec<usize> = Vec::with_capacity(segment.handles.len() + 2);
        chain.push(segment.start);
        chain.extend(&segment.handles);
        chain.push(segment.end);
        for (k, pair) in chain.windows(2).enumerate() {
            let [a, b] = pair else { continue };
            if cubic && k == 1 {
                continue;
            }
            if let (Some(a), Some(b)) = (at(*a), at(*b)) {
                out.push(Shape::line_segment([a, b], stroke));
            }
        }
    }
}

/// A diamond behind a node on a metric line, or a circle behind one inside an alignment zone.
fn highlight(y: f64, at: Pos2, t: &Tokens, metrics: &Metrics, out: &mut Vec<Shape>) {
    let r = t.metric_highlight_size / 2.0;
    if metrics.lines.iter().any(|l| (y - l).abs() <= ON_LINE) {
        let points = vec![at + Vec2::new(0.0, -r), at + Vec2::new(r, 0.0), at + Vec2::new(0.0, r), at + Vec2::new(-r, 0.0)];
        out.push(Shape::convex_polygon(points, t.metric_highlight, Stroke::NONE));
    } else if metrics.zones.iter().any(|(lo, hi)| (*lo..=*hi).contains(&y)) {
        out.push(Shape::circle_filled(at, r, t.metric_highlight));
    }
}

/// A circle around each pair of neighbouring on-curve nodes at the same place.
fn duplicates(path: &Path, screen: &[Pos2], on_curve: &[usize], t: &Tokens, out: &mut Vec<Shape>) {
    let pairs = on_curve.windows(2).filter_map(|w| match w {
        [a, b] => Some((*a, *b)),
        _ => None,
    });
    let wrap = match (path.closed && on_curve.len() > 2, on_curve.last(), on_curve.first()) {
        (true, Some(a), Some(b)) => Some((*a, *b)),
        _ => None,
    };
    for (a, b) in pairs.chain(wrap) {
        let (Some(na), Some(nb)) = (path.nodes.get(a), path.nodes.get(b)) else { continue };
        if na.x == nb.x
            && na.y == nb.y
            && let Some(at) = screen.get(a)
        {
            out.push(Shape::circle_stroke(*at, t.duplicate_node_radius, Stroke::new(t.handle_line_width * 1.5, t.duplicate_node)));
        }
    }
}

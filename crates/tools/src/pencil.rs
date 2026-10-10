//! The Pencil tool (`B`): a freehand stroke becomes a path (`[[tool]] pencil`, `docs/spec/03a`
//! § 5).
//!
//! The stroke's samples are fitted with cubic curves within about one screen point
//! ([`fontcraft_geom::fit::fit_polyline`]), so the path follows the hand closely and has many
//! nodes. A stroke that ends where it began, within the hit tolerance, gives a closed path. On
//! release one `path.addPath` adds the path: one undo step.

use fontcraft_geom::fit::{MAX_FIT_POINTS, fit_polyline};
use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_model::{Node, NodeKind};
use serde_json::{Value, json};

use crate::draw::params;
use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::selection::Selection;
use crate::tool::{Context, Tool};

/// Samples nearer than this many screen points to the last one kept are skipped.
const MIN_SAMPLE: f64 = 1.5;
/// How far the fitted path may stray from the stroke, in screen points.
const ACCURACY: f64 = 1.0;
/// Most straight pieces the stroke is drawn with while it is being drawn.
const MAX_PREVIEW_LINES: usize = 2_000;
/// The sine of the largest angle between the two sides of a node that still counts as smooth
/// (1°).
const SMOOTH_SINE: f64 = 0.017_452_4;

/// The Pencil tool.
#[derive(Clone, Debug, Default)]
pub struct PencilTool {
    stroke: Option<Vec<Point>>,
}

/// The nodes of the first subpath of `path`, quadratic pieces raised to cubic ones. For a closed
/// path the end that repeats the start is dropped. On-curve nodes whose two sides leave in one
/// direction are smooth.
pub fn nodes_of(path: &BezPath, closed: bool) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut last = Point::ZERO;
    let node = |p: Point, kind: NodeKind| Node::new(p.x, p.y, kind);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if !nodes.is_empty() {
                    break;
                }
                nodes.push(node(p, NodeKind::Line));
                last = p;
            }
            PathEl::LineTo(p) => {
                nodes.push(node(p, NodeKind::Line));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                nodes.push(node(last + (c - last) * (2.0 / 3.0), NodeKind::OffCurve));
                nodes.push(node(p + (c - p) * (2.0 / 3.0), NodeKind::OffCurve));
                nodes.push(node(p, NodeKind::Curve));
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                nodes.push(node(a, NodeKind::OffCurve));
                nodes.push(node(b, NodeKind::OffCurve));
                nodes.push(node(p, NodeKind::Curve));
                last = p;
            }
            PathEl::ClosePath => break,
        }
    }
    let repeats = closed && nodes.len() > 2 && nodes.first().zip(nodes.last()).is_some_and(|(f, l)| dist(f.point(), l.point()) < 1e-6);
    if repeats
        && let Some(kind) = nodes.pop().map(|n| n.kind)
        && let Some(first) = nodes.first_mut()
    {
        first.kind = kind;
    }
    let count = nodes.len();
    let around = |i: usize, back: bool| -> Option<Point> {
        let j = if back {
            match i.checked_sub(1) {
                Some(j) => j,
                None if closed => count.checked_sub(1)?,
                None => return None,
            }
        } else if i + 1 < count {
            i + 1
        } else if closed {
            0
        } else {
            return None;
        };
        nodes.get(j).map(Node::point)
    };
    let smooth: Vec<bool> = (0..count)
        .map(|i| {
            let (Some(n), Some(prev), Some(next)) = (nodes.get(i), around(i, true), around(i, false)) else { return false };
            let (a, b) = (n.point() - prev, next - n.point());
            let scale = a.hypot() * b.hypot();
            n.on_curve() && scale > 0.0 && a.dot(b) > 0.0 && a.cross(b).abs() <= SMOOTH_SINE * scale
        })
        .collect();
    for (n, s) in nodes.iter_mut().zip(smooth) {
        n.smooth = s;
    }
    nodes
}

fn node_json(n: &Node) -> Value {
    let kind = match n.kind {
        NodeKind::Line => "line",
        NodeKind::Curve => "curve",
        NodeKind::QCurve => "qcurve",
        NodeKind::OffCurve => "offcurve",
    };
    if n.smooth { json!({"x": n.x, "y": n.y, "type": kind, "smooth": true}) } else { json!({"x": n.x, "y": n.y, "type": kind}) }
}

impl PencilTool {
    /// The `path.addPath` for a finished stroke, or `None` when it is too short to make a path.
    fn finish(cx: &Context<'_>, stroke: &[Point]) -> Option<Invocation> {
        let (Some(&first), Some(&last)) = (stroke.first(), stroke.last()) else { return None };
        let closed = stroke.len() > 3 && dist(cx.view.to_screen(first), cx.view.to_screen(last)) <= cx.settings.hit_tolerance;
        let path = fit_polyline(stroke, cx.view.units(ACCURACY), closed);
        let nodes = nodes_of(&path, closed);
        let min_on_curve = if closed { 1 } else { 2 };
        if nodes.iter().filter(|n| n.on_curve()).count() < min_on_curve || nodes.len() < 2 {
            return None;
        }
        let list: Vec<Value> = nodes.iter().map(node_json).collect();
        let base = Selection::new().targets(cx.scene);
        Some(Invocation::new("path.addPath", params(&base, json!({"closed": closed, "nodes": list}))))
    }
}

impl Tool for PencilTool {
    fn id(&self) -> &'static str {
        "pencil"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        let far_enough = |stroke: &[Point]| {
            stroke.len() < MAX_FIT_POINTS && stroke.last().is_none_or(|q| dist(cx.view.to_screen(*q), cx.view.to_screen(ev.pos)) >= MIN_SAMPLE)
        };
        match ev.kind {
            PointerKind::Down => {
                self.stroke = Some(vec![ev.pos]);
                Output::handled()
            }
            PointerKind::Drag => match &mut self.stroke {
                Some(stroke) => {
                    if far_enough(stroke.as_slice()) {
                        stroke.push(ev.pos);
                    }
                    Output::handled()
                }
                None => Output::default(),
            },
            PointerKind::Up => {
                let Some(mut stroke) = self.stroke.take() else { return Output::default() };
                if stroke.len() < MAX_FIT_POINTS && stroke.last().is_none_or(|q| dist(*q, ev.pos) > 0.0) {
                    stroke.push(ev.pos);
                }
                match Self::finish(cx, &stroke) {
                    Some(invocation) => Output::with(Action::Run { invocation }),
                    None => Output::handled(),
                }
            }
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, _cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.pressed && ev.key == Key::Escape && self.stroke.take().is_some() {
            return Output::handled();
        }
        Output::default()
    }

    fn cancel(&mut self) -> Output {
        self.stroke = None;
        Output::default()
    }

    fn busy(&self) -> bool {
        self.stroke.is_some()
    }

    fn overlays(&self) -> Vec<Overlay> {
        let Some(stroke) = &self.stroke else { return Vec::new() };
        let stride = stroke.len() / MAX_PREVIEW_LINES + 1;
        let mut kept: Vec<Point> = stroke.iter().step_by(stride).copied().collect();
        if let Some(&last) = stroke.last()
            && kept.last() != Some(&last)
        {
            kept.push(last);
        }
        kept.windows(2)
            .filter_map(|w| match w {
                [a, b] => Some(Overlay::Line { kind: OverlayKind::DrawPreview, from: *a, to: *b }),
                _ => None,
            })
            .collect()
    }
}

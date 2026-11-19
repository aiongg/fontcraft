//! Audit markers in Edit View (`audit.toggleMarkers`) and the live audit that feeds them (`audit.toggleLiveAudit`).
//!
//! | Item | Drawn as | Layer |
//! |---|---|---|
//! | reported segment | the segment over the outline, faded severity colour | `auditHighlights` |
//! | reported path (no node or segment given) | the whole path, faded severity colour | `auditHighlights` |
//! | reported area (bounds only) | rectangle, faded severity colour | `auditHighlights` |
//! | reported node or handle | ring around it, severity colour | `auditMarkers` |
//! | the result itself | round badge beside the spot it points at, joined to it by a line | `auditMarkers` |
//!
//! Severity colours are `audit_fail`, `audit_warn` and `audit_info`. Badges sit beside the outline, not on it, so they
//! never cover a node. Live audit runs `audit.run` with the `outlines` profile on the edited layer once per document
//! revision, and not while a drag runs. With live audit off the markers show the edited layer's results of the last
//! `audit.run`, minus the stale ones.

use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};
use fontcraft_engine::Session;
use fontcraft_engine::model::{Layer, Path, Shape as ModelShape};
use fontcraft_geom::{Point, Segment, segments};
use serde_json::{Value, json};

use super::Drawn;
use super::tokens::OverlayTokens;
use crate::canvas::layers::PaintCx;
use crate::canvas::tools::Target;

/// Most results one layer shows markers for.
pub const MAX_MARKERS: usize = 1_000;
/// Most addresses one marker keeps of each kind.
const MAX_ADDRESSES: usize = 1_000;
/// Points sampled along each piece of a reported segment.
const SAMPLES: usize = 16;
/// Most times a badge moves aside to avoid another badge.
const MAX_SHIFTS: usize = 16;
/// The directions a badge tries from its spot, in screen axes (y down), in order.
const BADGE_DIRECTIONS: [(f32, f32); 8] = [(1.0, -1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0), (0.0, 1.0)];
/// The distances a badge tries, as multiples of `audit_badge_offset`.
const BADGE_DISTANCES: [f32; 3] = [1.0, 1.6, 2.4];
/// Space kept between a badge's edge and the outline.
const BADGE_MARGIN: f32 = 2.0;

/// How serious a result is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Fail,
    Warn,
    Info,
}

/// One result with a place on the edited layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub id: String,
    pub check: String,
    pub severity: Severity,
    pub message: String,
    pub shapes: Vec<usize>,
    pub points: Vec<[usize; 2]>,
    /// `[shape, end node]` of each segment.
    pub segments: Vec<[usize; 2]>,
    /// The spot the result points at, in font units.
    pub point: Option<[f64; 2]>,
    /// `[x0, y0, x1, y1]` in font units.
    pub bounds: Option<[f64; 4]>,
}

impl Marker {
    /// A result of the audit result document (`docs/spec/21-qa.md` § 4); `None` for one that does not read as one.
    pub fn from_result(r: &Value) -> Option<Marker> {
        let severity = match r.get("severity")?.as_str()? {
            "fail" => Severity::Fail,
            "warn" => Severity::Warn,
            _ => Severity::Info,
        };
        let loc = r.get("location")?;
        let pairs = |key: &str| -> Vec<[usize; 2]> {
            let list = loc.get(key).and_then(Value::as_array).into_iter().flatten().take(MAX_ADDRESSES);
            list.filter_map(|p| {
                let s = usize::try_from(p.get(0)?.as_u64()?).ok()?;
                let n = usize::try_from(p.get(1)?.as_u64()?).ok()?;
                Some([s, n])
            })
            .collect()
        };
        let numbers = |key: &str| -> Option<Vec<f64>> {
            let list = loc.get(key)?.as_array().filter(|a| a.len() <= 4)?;
            list.iter().map(|v| v.as_f64().filter(|n| n.is_finite())).collect()
        };
        let point = numbers("point").and_then(|v| Some([*v.first()?, *v.get(1)?]));
        let bounds = numbers("bounds").and_then(|v| Some([*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?]));
        let shapes = loc.get("shapes").and_then(Value::as_array).into_iter().flatten().take(MAX_ADDRESSES);
        Some(Marker {
            id: r.get("id")?.as_str()?.to_string(),
            check: r.get("check").and_then(Value::as_str).unwrap_or_default().to_string(),
            severity,
            message: r.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            shapes: shapes.filter_map(|s| usize::try_from(s.as_u64()?).ok()).collect(),
            points: pairs("points"),
            segments: pairs("segments"),
            point,
            bounds,
        })
    }
}

/// The audit results of the target's layer: a fresh `audit.run` with live audit, else the kept results.
pub fn read(session: &mut Session, target: &Target, live: bool, runs: &mut u64) -> Vec<Marker> {
    let result = if live {
        *runs = runs.saturating_add(1);
        session.query("audit.run", &json!({"profile": "outlines", "glyphs": [target.glyph], "layers": [target.layer]}))
    } else {
        // An error here means no audit has run on this document: nothing to mark.
        session.query("audit.results", &json!({"glyphs": [target.glyph]}))
    };
    let Ok(doc) = result else { return Vec::new() };
    let Some(results) = doc.get("results").and_then(Value::as_array) else { return Vec::new() };
    results
        .iter()
        .filter(|r| r.get("stale").and_then(Value::as_bool) != Some(true))
        .filter(|r| r.get("location").and_then(|l| l.get("layer")).and_then(Value::as_str) == Some(target.layer.as_str()))
        .filter_map(Marker::from_result)
        .take(MAX_MARKERS)
        .collect()
}

fn color(t: &OverlayTokens, s: Severity) -> Color32 {
    match s {
        Severity::Fail => t.audit_fail,
        Severity::Warn => t.audit_warn,
        Severity::Info => t.audit_info,
    }
}

fn path(layer: &Layer, shape: usize) -> Option<&Path> {
    match layer.shapes.get(shape)? {
        ModelShape::Path(p) => Some(p),
        _ => None,
    }
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// `segment` as a screen polyline; `None` when it has no usable length on screen.
fn sampled(cx: &PaintCx<'_>, segment: &Segment) -> Option<Vec<Pos2>> {
    let mut line: Vec<Pos2> = Vec::with_capacity(segment.pieces.len() * (SAMPLES + 1));
    for piece in &segment.pieces {
        line.extend((0..=SAMPLES).map(|k| cx.camera.to_screen(piece.eval(k as f64 / SAMPLES as f64))));
    }
    (line.len() > 1 && line.iter().all(|p| usable(*p))).then_some(line)
}

fn shown<'a>(cx: &'a PaintCx<'_>) -> &'a [Marker] {
    if cx.overlays.data.switches.audit_markers { &cx.overlays.data.markers } else { &[] }
}

/// The reported segments, paths and areas, under the nodes.
pub fn highlights(cx: &PaintCx<'_>) {
    highlight_shapes(cx).paint(cx);
}

pub fn highlight_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let Some(glyph) = cx.glyph() else { return out };
    let t = OverlayTokens::standard();
    for m in shown(cx) {
        let c = color(&t, m.severity);
        let faded = c.gamma_multiply(t.audit_highlight_alpha);
        let stroke = Stroke::new(t.audit_highlight_width, faded);
        let mut drew = false;
        for &[s, end] in &m.segments {
            let Some(p) = path(&glyph.layer, s) else { continue };
            for seg in segments(&p.node_points(), p.closed).iter().filter(|g| g.end == end) {
                if let Some(line) = sampled(cx, seg) {
                    out.shapes.push(Shape::line(line, stroke));
                    drew = true;
                }
            }
        }
        if m.points.is_empty() && m.segments.is_empty() {
            for &s in &m.shapes {
                let Some(p) = path(&glyph.layer, s) else { continue };
                for seg in &segments(&p.node_points(), p.closed) {
                    if let Some(line) = sampled(cx, seg) {
                        out.shapes.push(Shape::line(line, stroke));
                        drew = true;
                    }
                }
            }
        }
        if !drew
            && m.points.is_empty()
            && let Some([x0, y0, x1, y1]) = m.bounds
        {
            let (a, b) = (cx.camera.to_screen(Point::new(x0, y0)), cx.camera.to_screen(Point::new(x1, y1)));
            if usable(a) && usable(b) {
                out.shapes.push(Shape::rect_filled(Rect::from_two_pos(a, b), 0.0, faded.gamma_multiply(0.5)));
            }
        }
    }
    out
}

/// The spot a marker's badge points at, in font units.
fn anchor(layer: &Layer, m: &Marker) -> Option<Point> {
    if let Some([x, y]) = m.point {
        return Some(Point::new(x, y));
    }
    let node = |[s, n]: [usize; 2]| path(layer, s).and_then(|p| p.nodes.get(n)).map(|node| node.point());
    if let Some(p) = m.points.iter().copied().find_map(node) {
        return Some(p);
    }
    if let Some(p) = m.segments.iter().copied().find_map(node) {
        return Some(p);
    }
    if let Some(p) = m.shapes.iter().find_map(|&s| path(layer, s)?.nodes.first().map(|n| n.point())) {
        return Some(p);
    }
    m.bounds.map(|[x0, y0, x1, y1]| Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0))
}

/// The distance from `p` to the line piece `w`.
fn clearance(p: Pos2, w: &[Pos2]) -> f32 {
    let [a, b] = w else { return f32::INFINITY };
    let ab = *b - *a;
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 { ((p - *a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(*a + ab * t)
}

/// Where each marker's badge goes, as `(the reported spot, the badge centre)` on screen; `None` for a marker without a
/// spot. The badge takes the first place around the spot, up-right first, that keeps clear of the outline and of
/// earlier badges; when none does, it sits up-right and moves right off earlier badges.
pub fn badges(cx: &PaintCx<'_>, layer: &Layer, markers: &[Marker]) -> Vec<Option<(Pos2, Pos2)>> {
    let t = OverlayTokens::standard();
    let (radius, offset) = (t.audit_badge_radius, t.audit_badge_offset);
    let mut outline: Vec<(Rect, Vec<Pos2>)> = Vec::new();
    for shape in &layer.shapes {
        let ModelShape::Path(p) = shape else { continue };
        for seg in &segments(&p.node_points(), p.closed) {
            if let Some(line) = sampled(cx, seg) {
                outline.push((Rect::from_points(&line), line));
            }
        }
    }
    let mut placed: Vec<Pos2> = Vec::new();
    markers
        .iter()
        .map(|m| {
            let at = anchor(layer, m).map(|p| cx.camera.to_screen(p)).filter(|p| usable(*p))?;
            let reach = offset * BADGE_DISTANCES.last().copied().unwrap_or(1.0) + radius * 2.0;
            let near: Vec<&[Pos2]> = outline.iter().filter(|(b, _)| b.expand(reach).contains(at)).map(|(_, l)| l.as_slice()).collect();
            let free = |badge: Pos2, placed: &[Pos2]| {
                !placed.iter().any(|p| p.distance(badge) < radius * 2.0)
                    && near.iter().all(|line| line.windows(2).all(|w| clearance(badge, w) > radius + BADGE_MARGIN))
            };
            let spot = BADGE_DISTANCES
                .iter()
                .flat_map(|d| BADGE_DIRECTIONS.iter().map(move |(x, y)| Vec2::new(*x, *y).normalized() * offset * *d))
                .map(|v| at + v)
                .find(|b| free(*b, &placed));
            let badge = spot.unwrap_or_else(|| {
                let mut badge = at + Vec2::new(offset, -offset);
                for _ in 0..MAX_SHIFTS {
                    if !placed.iter().any(|p| p.distance(badge) < radius * 2.0) {
                        break;
                    }
                    badge.x += radius * 2.0 + 1.0;
                }
                badge
            });
            placed.push(badge);
            Some((at, badge))
        })
        .collect()
}

/// Rings around the reported nodes and the badges, over everything but the tools.
pub fn markers(cx: &PaintCx<'_>) {
    marker_shapes(cx).paint(cx);
}

pub fn marker_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let Some(glyph) = cx.glyph() else { return out };
    let t = OverlayTokens::standard();
    let list = shown(cx);
    for m in list {
        let c = color(&t, m.severity);
        for &[s, n] in &m.points {
            let Some(node) = path(&glyph.layer, s).and_then(|p| p.nodes.get(n)) else { continue };
            let at = cx.camera.to_screen(node.point());
            if usable(at) {
                out.shapes.push(Shape::circle_stroke(at, t.audit_ring_radius, Stroke::new(t.audit_ring_width, c)));
            }
        }
    }
    for (m, spot) in list.iter().zip(badges(cx, &glyph.layer, list)) {
        let Some((at, badge)) = spot else { continue };
        let c = color(&t, m.severity);
        out.shapes.push(Shape::line_segment([at, badge], Stroke::new(1.0, c)));
        out.shapes.push(Shape::circle_filled(badge, t.audit_badge_radius, c));
    }
    out
}

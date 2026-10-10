//! Audit markers in Edit View (`audit.canvasMarkers`), and the live audit that feeds them
//! (`audit.liveAudit`).
//!
//! | Item | Drawn as |
//! |---|---|
//! | reported segment | the segment over the outline, faded severity colour |
//! | reported path (no node or segment given) | the whole path, faded severity colour |
//! | reported node or handle | ring around it, severity colour |
//! | reported area (bounds only) | rectangle, faded severity colour |
//! | the result itself | round badge beside the spot it points at, joined to it by a line |
//!
//! Severity colours are `audit_fail`, `audit_warn` and `audit_info`. Hovering a badge shows the
//! result's message; clicking it selects the reported nodes, segments and components through
//! `selection.set`. Badges sit beside the outline, not on it, so they never cover a node.
//!
//! Live audit runs `audit.run` with the `outlines` profile on the edited layer once per document
//! revision, never per frame, and not while a drag or a dialog preview runs. With live audit off
//! the markers show the edited layer's results of the last `audit.run`, minus the stale ones.

use egui::{Pos2, Rect, Sense, Shape, Stroke, Vec2};
use fontcraft_engine::Session;
use fontcraft_geom::{Point, Segment, segments};
use fontcraft_model::{Layer, Path, Shape as ModelShape};
use fontcraft_tools::View;
use serde_json::{Value, json};

use super::Painting;
use super::view;
use crate::app::FontcraftApp;
use crate::shell::audit::{AuditState, Key, MAX_MARKERS, Marker, Severity, Shown};
use crate::state::Tab;
use crate::theme::Tokens;

/// Points sampled along each piece of a marked segment or path.
const SAMPLES: usize = 16;
/// Most times a badge moves aside to avoid another badge.
const MAX_SHIFTS: usize = 16;

/// What the markers would be read for now.
fn current_key(session: &Session, tab: Option<&Tab>, live: bool) -> Option<Key> {
    let doc = session.active()?;
    let glyph = tab?.edited_glyph()?;
    let master = view::active_master(&doc.font)?;
    let layer = doc.font.glyph(glyph)?.layer(&master.id)?;
    Some(Key { doc: doc.id, revision: doc.revision, glyph: glyph.to_string(), layer: layer.id.clone(), live })
}

/// Runs live audit, or reads the kept results, when the edited layer's key changed since the
/// last time. Called once per frame before the canvas draws.
pub fn refresh(app: &mut FontcraftApp) {
    let FontcraftApp { session, state, .. } = app;
    if !state.audit.live && !state.audit.markers {
        return;
    }
    // A drag or a dialog preview changes the revision at every step; the audit waits for it to
    // end, and the markers hide meanwhile (their key no longer matches).
    if session.interaction().is_some() {
        return;
    }
    let Some(key) = current_key(session, state.tab(), state.audit.live) else {
        state.audit.shown = Shown::default();
        return;
    };
    let audit = &mut state.audit;
    let reread = std::mem::take(&mut audit.reread) && !key.live;
    if audit.shown.key.as_ref() == Some(&key) && !reread {
        return;
    }
    let result = if key.live {
        audit.runs = audit.runs.saturating_add(1);
        session.execute("audit.run", &json!({"profile": "outlines", "glyphs": [key.glyph], "layers": [key.layer]}))
    } else {
        // An error here means no audit has run on this document: nothing to mark.
        session.execute("audit.results", &json!({"glyphs": [key.glyph]}))
    };
    let markers = match result {
        Ok(doc) => markers_of(&doc, &key.layer),
        Err(e) => {
            if key.live {
                log::warn!("live audit: {e}");
            }
            Vec::new()
        }
    };
    audit.shown = Shown { key: Some(key), markers };
}

/// The results of a result document that sit on `layer` and are not stale.
fn markers_of(doc: &Value, layer: &str) -> Vec<Marker> {
    let Some(results) = doc.get("results").and_then(Value::as_array) else { return Vec::new() };
    results
        .iter()
        .filter(|r| r.get("stale").and_then(Value::as_bool) != Some(true))
        .filter(|r| r.get("location").and_then(|l| l.get("layer")).and_then(Value::as_str) == Some(layer))
        .filter_map(Marker::from_result)
        .take(MAX_MARKERS)
        .collect()
}

/// The markers to draw for `tab` now: none when they are hidden or were read for another
/// revision, glyph or layer.
pub fn visible<'a>(audit: &'a AuditState, session: &Session, tab: &Tab) -> &'a [Marker] {
    if !audit.markers {
        return &[];
    }
    match (&audit.shown.key, current_key(session, Some(tab), audit.live)) {
        (Some(shown), Some(now)) if *shown == now => audit.shown.markers.as_slice(),
        _ => &[],
    }
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn path(layer: &Layer, shape: usize) -> Option<&Path> {
    match layer.shapes.get(shape)? {
        ModelShape::Path(p) => Some(p),
        _ => None,
    }
}

/// The screen polyline of each segment of `path` that `keep` accepts (by end node).
fn segment_lines(path: &Path, c: &Painting, keep: impl Fn(usize) -> bool) -> Vec<Vec<Pos2>> {
    let mut out = Vec::new();
    for segment in segments(&path.node_points(), path.closed).iter().filter(|s| keep(s.end)) {
        out.extend(sampled(segment, c.view));
    }
    out
}

/// `segment` as a screen polyline; `None` when it has no usable length on screen.
fn sampled(segment: &Segment, view: View) -> Option<Vec<Pos2>> {
    let mut line: Vec<Pos2> = Vec::with_capacity(segment.pieces.len() * SAMPLES + 1);
    for piece in &segment.pieces {
        line.extend((0..=SAMPLES).map(|k| pos(view.to_screen(piece.eval(k as f64 / SAMPLES as f64)))));
    }
    (line.len() > 1 && line.iter().all(|p| usable(*p))).then_some(line)
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

/// The `selection.set` parameters that select what a marker reports: its nodes, both ends of its
/// segments, every node of a path it reports as a whole, and its components.
pub fn selection_params(layer: &Layer, m: &Marker) -> Option<Value> {
    let mut points: Vec<[usize; 2]> = m.points.iter().copied().filter(|&[s, n]| path(layer, s).is_some_and(|p| n < p.nodes.len())).collect();
    for &[s, end] in &m.segments {
        let Some(p) = path(layer, s) else { continue };
        if let Some(seg) = segments(&p.node_points(), p.closed).iter().find(|g| g.end == end) {
            points.extend([[s, seg.start], [s, seg.end]]);
        }
    }
    let mut shapes = Vec::new();
    for &s in &m.shapes {
        match layer.shapes.get(s) {
            Some(ModelShape::Path(p)) if m.points.is_empty() && m.segments.is_empty() => points.extend((0..p.nodes.len()).map(|n| [s, n])),
            Some(ModelShape::Path(_)) | None => {}
            Some(_) => shapes.push(s),
        }
    }
    points.sort_unstable();
    points.dedup();
    if points.is_empty() && shapes.is_empty() {
        return None;
    }
    Some(json!({"points": points, "shapes": shapes}))
}

/// Where each marker's badge goes, as `(the reported spot, the badge centre)` in screen points;
/// `None` for a marker without a spot. The badge takes the first spot around the reported one,
/// up-right first, that keeps clear of the outline and of earlier badges; when none does, it
/// sits up-right and moves right off earlier badges.
pub fn badges(layer: &Layer, markers: &[Marker], view: View, tokens: &Tokens) -> Vec<Option<(Pos2, Pos2)>> {
    let radius = tokens.audit_badge_radius;
    let offset = tokens.audit_badge_offset;
    let outline = outline_lines(layer, view);
    let mut placed: Vec<Pos2> = Vec::new();
    markers
        .iter()
        .map(|m| {
            let at = anchor(layer, m).map(|p| pos(view.to_screen(p))).filter(|p| usable(*p))?;
            let reach = offset * BADGE_DISTANCES.last().copied().unwrap_or(1.0) + radius * 2.0;
            let near: Vec<&[Pos2]> =
                outline.iter().filter(|(bounds, _)| bounds.expand(reach).contains(at)).map(|(_, line)| line.as_slice()).collect();
            let free = |badge: Pos2| {
                !placed.iter().any(|p| p.distance(badge) < radius * 2.0)
                    && near.iter().all(|line| line.windows(2).all(|w| clearance(badge, w) > radius + BADGE_MARGIN))
            };
            let spot = BADGE_DISTANCES
                .iter()
                .flat_map(|d| BADGE_DIRECTIONS.iter().map(move |(x, y)| Vec2::new(*x, *y).normalized() * offset * *d))
                .map(|v| at + v)
                .find(|b| free(*b));
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

/// The directions a badge tries from its spot, in screen axes (y down), in order.
const BADGE_DIRECTIONS: [(f32, f32); 8] = [(1.0, -1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0), (0.0, 1.0)];
/// The distances a badge tries, as multiples of `audit_badge_offset`.
const BADGE_DISTANCES: [f32; 3] = [1.0, 1.6, 2.4];
/// Space kept between a badge's edge and the outline, in screen points.
const BADGE_MARGIN: f32 = 2.0;

/// Every path of `layer` as screen polylines, each with its bounds.
fn outline_lines(layer: &Layer, view: View) -> Vec<(Rect, Vec<Pos2>)> {
    let mut out = Vec::new();
    for shape in &layer.shapes {
        let ModelShape::Path(path) = shape else { continue };
        for segment in &segments(&path.node_points(), path.closed) {
            if let Some(line) = sampled(segment, view) {
                out.push((Rect::from_points(&line), line));
            }
        }
    }
    out
}

/// The distance from `p` to the line piece `w` (two points; anything else is far away).
fn clearance(p: Pos2, w: &[Pos2]) -> f32 {
    let [a, b] = w else { return f32::INFINITY };
    let ab = *b - *a;
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 { ((p - *a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(*a + ab * t)
}

/// Draws the markers of the edited layer. With `interactive`, each badge shows its message on
/// hover and a click queues the `selection.set` that selects what it reports.
pub fn draw(ui: &egui::Ui, c: &Painting, layer: &Layer, markers: &[Marker], interactive: bool, pending: &mut Vec<(&'static str, Value)>) {
    let t = c.tokens;
    let spots = badges(layer, markers, c.view, t);
    for (i, (m, spot)) in markers.iter().zip(spots).enumerate() {
        let color = match m.severity {
            Severity::Fail => t.audit_fail,
            Severity::Warn => t.audit_warn,
            Severity::Info => t.audit_info,
        };
        let faded = Stroke::new(t.audit_highlight_width, color.gamma_multiply(t.audit_highlight_alpha));
        let mut shapes: Vec<Shape> = Vec::new();
        if m.segments.is_empty() {
            for p in m.shapes.iter().filter_map(|&s| path(layer, s)) {
                shapes.extend(segment_lines(p, c, |_| true).into_iter().map(|line| Shape::line(line, faded)));
            }
        }
        for &[s, end] in &m.segments {
            if let Some(p) = path(layer, s) {
                shapes.extend(segment_lines(p, c, |e| e == end).into_iter().map(|line| Shape::line(line, faded)));
            }
        }
        for &[s, n] in &m.points {
            let Some(at) = path(layer, s).and_then(|p| p.nodes.get(n)).map(|node| pos(c.view.to_screen(node.point()))) else { continue };
            if usable(at) {
                shapes.push(Shape::circle_stroke(at, t.audit_ring_radius, Stroke::new(t.audit_ring_width, color)));
            }
        }
        if m.shapes.is_empty()
            && m.points.is_empty()
            && m.segments.is_empty()
            && let Some([x0, y0, x1, y1]) = m.bounds
        {
            let (a, b) = (pos(c.view.to_screen(Point::new(x0, y0))), pos(c.view.to_screen(Point::new(x1, y1))));
            if usable(a) && usable(b) {
                shapes.push(Shape::rect_stroke(Rect::from_two_pos(a, b), 0.0, faded, egui::StrokeKind::Middle));
            }
        }
        c.painter.extend(shapes);
        let Some((at, badge)) = spot else { continue };
        c.painter.line_segment([at, badge], Stroke::new(t.audit_ring_width, color));
        c.painter.circle(badge, t.audit_badge_radius, color, Stroke::new(1.0, t.canvas));
        if !interactive || !c.rect.contains(badge) {
            continue;
        }
        let area = Rect::from_center_size(badge, Vec2::splat(t.audit_badge_radius * 2.0));
        let response = ui.interact(area, ui.id().with(("audit-marker", i)), Sense::click_and_drag()).on_hover_text(m.message.as_str());
        if response.clicked()
            && let Some(params) = selection_params(layer, m)
        {
            pending.push(("selection.set", params));
        }
    }
}

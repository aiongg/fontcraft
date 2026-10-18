//! Display aids in front of the outline: Tunni lines, curve tension, node indexes, curve lengths,
//! details across glyphs, the ghost outline, the metrics beam, and the Lock Outline badge. The
//! switches are in `crate::shell::edit_aids`.
//!
//! The aids reuse the measurement colours of [`Tokens`]:
//!
//! | Aid | Colour |
//! |---|---|
//! | Tunni lines and points, tension percentages | `measure_off_curve` |
//! | curve lengths | `measure_delta` |
//! | indexes of nodes, segments, paths, components, anchors | `measure_on_curve`, `measure_ruler`, `guide_local`, `component_manual`, `anchor` |
//! | ghost outline | `background_outline` |
//! | metrics beam and its sidebearings | `measurement_line`, `measurement_line_label` |
//! | Lock Outline badge | `locked_point` |

use egui::{Align2, Color32, FontId, Pos2, Stroke, StrokeKind};
use fontcraft_geom::{CubicBez, ParamCurveArclen, Piece, Point, Segment, Shape as _, segments};
use fontcraft_model::{Font, Layer, Shape};
use fontcraft_tools::View;

use super::{Painting, Scene};
use crate::panels::Pending;
use crate::panels::widgets::format_number;
use crate::shell::edit_aids::{EditAidsState, beam_height};

/// Most other glyphs Details Across Glyphs draws on the edited glyph's line.
const MAX_ACROSS: usize = 200;
/// Most labels (indexes, tensions, lengths) drawn in one frame; a pathological glyph stops there.
const MAX_LABELS: usize = 20_000;
/// Accuracy of a curve length, in font units.
const LENGTH_ACCURACY: f64 = 1e-3;

thread_local! {
    /// Layouts for Details Across Glyphs. The canvas keeps its own cache in the app; overlays see
    /// only the [`Scene`], so this one is kept beside it, on the UI thread.
    static LAYOUTS: super::view::LayoutCache = super::view::LayoutCache::default();
}

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, pending);
    let aids = &scene.state.panels.edit_aids;
    ghost(c, scene);
    let beam = aids.beam.then(|| beam_height(aids, scene.font, scene.doc.0, scene.tab)).filter(|y| y.is_finite());
    if let Some(y) = beam {
        beam_line(c, y);
    }
    let mut budget = MAX_LABELS;
    if aids.details_across_glyphs {
        across(c, scene, aids, beam, &mut budget);
    }
    layer_aids(c, scene.font, scene.layer, aids, &mut budget);
    if let Some(y) = beam {
        beam_labels(c, scene.font, scene.layer, y);
    }
    if aids.lock_outline {
        badge(c);
    }
}

/// The outline as it was when the running interaction began, in grey: only the shapes the
/// interaction changed, so the unchanged ones are not drawn twice.
fn ghost(c: &Painting, scene: &Scene) {
    let Some(before) = scene.session.interaction_start() else { return };
    let Some(old) = super::view::edited_layer(before, scene.tab) else { return };
    let changed: Vec<Shape> = old.shapes.iter().enumerate().filter(|(i, s)| scene.layer.shapes.get(*i) != Some(*s)).map(|(_, s)| s.clone()).collect();
    if changed.is_empty() {
        return;
    }
    let outline = super::background::outline(before, &changed, &old.master);
    let v = c.view;
    let to_screen = fontcraft_geom::Affine::translate(v.origin.to_vec2()) * fontcraft_geom::Affine::scale_non_uniform(v.scale(), -v.scale());
    let stroke = Stroke::new(c.tokens.background_outline_width, c.tokens.background_outline);
    for contour in super::flatten(&(to_screen * outline)) {
        c.painter.add(egui::Shape::line(contour, stroke));
    }
}

/// The other glyphs on the edited glyph's line: their nodes (when nodes show) and the aids that
/// are on.
fn across(c: &Painting, scene: &Scene, aids: &EditAidsState, beam: Option<f64>, budget: &mut usize) {
    let Some(index) = scene.tab.edited_index() else { return };
    let Some(layout) = LAYOUTS.with(|cache| super::view::text_layout(scene.session, cache, scene.tab)) else { return };
    let Some(edited) = layout.placed.get(index) else { return };
    let Some(line) = layout.lines.iter().find(|l| l.items.contains(&index)) else { return };
    let s = c.view.scale();
    let metrics = super::nodes::Metrics { lines: Vec::new(), zones: Vec::new() };
    for i in line.items.clone().filter(|i| *i != index).take(MAX_ACROSS) {
        let Some(placed) = layout.placed.get(i) else { continue };
        let Some(name) = placed.glyph.as_deref() else { continue };
        let Some(layer) = super::view::shown_layer(scene.font, scene.tab, name) else { continue };
        let origin = Point::new(c.view.origin.x + (placed.x - edited.x) * s, c.view.origin.y - (placed.y - edited.y) * s);
        let gc = Painting { painter: c.painter, rect: c.rect, view: View::new(s, origin), tokens: c.tokens };
        if scene.state.display.nodes {
            c.painter.extend(super::nodes::shapes(layer, gc.view, c.tokens, &metrics, None));
        }
        layer_aids(&gc, scene.font, layer, aids, budget);
        if let Some(y) = beam {
            beam_labels(&gc, scene.font, layer, y);
        }
    }
}

/// Tunni lines, tensions, lengths and indexes of one layer, as `aids` asks.
fn layer_aids(c: &Painting, font: &Font, layer: &Layer, aids: &EditAidsState, budget: &mut usize) {
    let t = c.tokens;
    let ix = aids.indexes;
    for (s, shape) in layer.shapes.iter().enumerate() {
        match shape {
            Shape::Path(path) => {
                let points = path.node_points();
                let segs = segments(&points, path.closed);
                if ix.contours
                    && let Some(first) = points.first()
                {
                    label(c, first.point, Offset::BelowLeft, s.to_string(), t.guide_local, budget);
                }
                for (k, seg) in segs.iter().enumerate() {
                    let cubic = match seg.pieces.as_slice() {
                        [Piece::Cubic(cb)] => Some(*cb),
                        _ => None,
                    };
                    if aids.tunni_lines
                        && let Some(cb) = cubic
                    {
                        tunni(c, cb);
                    }
                    if aids.curve_tension
                        && let Some(cb) = cubic
                    {
                        tension(c, cb, budget);
                    }
                    if aids.curve_length && !seg.handles.is_empty() && *budget > 0 {
                        let length: f64 = seg.pieces.iter().map(piece_length).sum();
                        if let Some(mid) = middle(seg) {
                            label(c, mid, Offset::AboveRight, format_number((length * 10.0).round() / 10.0), t.measure_delta, budget);
                        }
                    }
                    if ix.segments
                        && let Some(mid) = middle(seg)
                    {
                        label(c, mid, Offset::BelowRight, k.to_string(), t.measure_ruler, budget);
                    }
                }
                if ix.points {
                    for (n, node) in points.iter().enumerate() {
                        label(c, node.point, Offset::AboveRight, n.to_string(), t.measure_on_curve, budget);
                    }
                }
            }
            Shape::Component(component) if ix.components => {
                let at = match font.component_outline(component, &layer.master) {
                    Ok(outline) if !outline.elements().is_empty() => outline.bounding_box().center(),
                    _ => component.affine() * Point::ZERO,
                };
                label(c, at, Offset::Center, s.to_string(), t.component_manual, budget);
            }
            _ => {}
        }
    }
    if ix.anchors {
        for (i, anchor) in layer.anchors.iter().enumerate() {
            label(c, Point::new(anchor.x, anchor.y), Offset::BelowRight, i.to_string(), t.anchor, budget);
        }
    }
}

/// The dotted line between the ends of a cubic's handles and the dot where the lines through
/// its handles meet (none when they are parallel or meet behind a node).
fn tunni(c: &Painting, cb: CubicBez) {
    let t = c.tokens;
    let (a, b) = (screen(c.view, cb.p1), screen(c.view, cb.p2));
    let (Some(a), Some(b)) = (a, b) else { return };
    let stroke = Stroke::new(t.handle_line_width, t.measure_off_curve);
    let dash = t.handle_radius * 2.0;
    c.painter.extend(egui::Shape::dashed_line(&[a, b], stroke, dash, dash));
    if let Some(p) = fontcraft_engine::pathops::tension::tunni_point(cb).and_then(|p| screen(c.view, p)) {
        c.painter.circle_filled(p, t.handle_radius, t.measure_off_curve);
    }
}

/// Each handle's tension, in whole percent, beside the handle's end.
fn tension(c: &Painting, cb: CubicBez, budget: &mut usize) {
    let Some((first, second)) = fontcraft_engine::pathops::tension::tension(cb) else { return };
    let color = c.tokens.measure_off_curve;
    label(c, cb.p1, Offset::AboveRight, format!("{}%", format_number(first.round())), color, budget);
    label(c, cb.p2, Offset::AboveRight, format!("{}%", format_number(second.round())), color, budget);
}

fn piece_length(piece: &Piece) -> f64 {
    let length = match piece {
        Piece::Line(l) => l.arclen(LENGTH_ACCURACY),
        Piece::Quad(q) => q.arclen(LENGTH_ACCURACY),
        Piece::Cubic(cb) => cb.arclen(LENGTH_ACCURACY),
    };
    if length.is_finite() { length } else { 0.0 }
}

/// The point halfway through a segment's pieces.
fn middle(seg: &Segment) -> Option<Point> {
    let count = seg.pieces.len();
    let piece = seg.pieces.get(count / 2)?;
    let t = if count % 2 == 1 { 0.5 } else { 0.0 };
    Some(piece.eval(t))
}

/// The beam across the canvas at height `y`.
fn beam_line(c: &Painting, y: f64) {
    let Some(at) = screen(c.view, Point::new(0.0, y)) else { return };
    let stroke = Stroke::new(c.tokens.measurement_line_width, c.tokens.measurement_line);
    c.painter.line_segment([Pos2::new(c.rect.left(), at.y), Pos2::new(c.rect.right(), at.y)], stroke);
}

/// The sidebearings measured on the beam, each centred over its gap.
fn beam_labels(c: &Painting, font: &Font, layer: &Layer, y: f64) {
    let Ok(Some((left, right))) = fontcraft_spacing::sidebearings_at_height(font, layer, y) else { return };
    let t = c.tokens;
    let font_id = FontId::proportional(t.measurement_line_label_size);
    for (x, value) in [(left / 2.0, left), (layer.width - right / 2.0, right)] {
        if let Some(at) = screen(c.view, Point::new(x, y)) {
            c.painter.text(at, Align2::CENTER_BOTTOM, format_number(value), font_id.clone(), t.measurement_line_label);
        }
    }
}

/// The badge in the canvas's top right corner while Lock Outline is on.
fn badge(c: &Painting) {
    let t = c.tokens;
    let at = Pos2::new(c.rect.right() - t.palette_margin, c.rect.top() + t.palette_margin);
    let text = c.painter.text(at, Align2::RIGHT_TOP, "Outline locked", FontId::proportional(t.measure_label_size), t.locked_point);
    c.painter.rect_stroke(
        text.expand(t.handle_radius * 2.0),
        f32::from(t.radius),
        Stroke::new(t.handle_line_width, t.locked_point),
        StrokeKind::Outside,
    );
}

/// Where a label sits beside its point.
#[derive(Clone, Copy)]
enum Offset {
    AboveRight,
    BelowRight,
    BelowLeft,
    Center,
}

/// One label beside the font point `p`, while the frame's label budget lasts. Labels off the
/// canvas are skipped and do not count.
fn label(c: &Painting, p: Point, offset: Offset, text: String, color: Color32, budget: &mut usize) {
    if *budget == 0 {
        return;
    }
    let Some(at) = screen(c.view, p) else { return };
    if !c.rect.expand(c.tokens.measure_label_size * 4.0).contains(at) {
        return;
    }
    *budget -= 1;
    let d = c.tokens.node_size;
    let (at, align) = match offset {
        Offset::AboveRight => (at + egui::vec2(d, -d), Align2::LEFT_BOTTOM),
        Offset::BelowRight => (at + egui::vec2(d, d), Align2::LEFT_TOP),
        Offset::BelowLeft => (at + egui::vec2(-d, d), Align2::RIGHT_TOP),
        Offset::Center => (at, Align2::CENTER_CENTER),
    };
    c.painter.text(at, align, text, FontId::proportional(c.tokens.measure_label_size), color);
}

/// A font point on screen; `None` when it is not a finite position.
fn screen(view: View, p: Point) -> Option<Pos2> {
    let s = view.to_screen(p);
    let at = Pos2::new(s.x as f32, s.y as f32);
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

//! Auto-meter labels: stem and counter widths, corner angles and the quick measurement (View › Measure:
//! `view.showStemWidths`, `view.showCounterWidths`, `view.showCornerAngles`, `view.showQuickMeasurement`).
//!
//! The numbers come from the engine's auto-meter (`fontcraft_engine::cmd::measurements`) on the edited layer side,
//! measured once per edit by [`super::refresh`]. Stems are coloured by how they compare with the master's stem values:
//! one of them, near one (with a dot beside the label), or neither.

use egui::{Align2, Pos2, Shape, Stroke};
use fontcraft_engine::cmd::measurements::{MeterOptions, measure_parts, quick_measure_parts};
use fontcraft_engine::model::Font;
use fontcraft_geom::{Point, Vec2};
use serde_json::Value;

use super::tokens::OverlayTokens;
use super::{Drawn, num};
use crate::canvas::layers::PaintCx;
use crate::canvas::tools::Target;

/// Most labels of one kind drawn.
const MAX_LABELS: usize = 2_000;

/// The auto-meter's measurement of the target's layer: `[stems, counters, corners]` say which parts.
pub fn measure(font: &Font, target: &Target, parts: [bool; 3]) -> Option<Value> {
    let layer = target.layer_in(font)?;
    let master = font.masters.iter().find(|m| m.id == layer.master)?;
    let [stems, counters, corners] = parts;
    let options = MeterOptions { stems, counters, corners, ..MeterOptions::default() };
    measure_parts(font, master, &layer.parts(target.side), &options).ok()
}

/// The quick measurement through `at`: from, to and the distance. With `shift`, horizontally or vertically, whichever
/// is nearer the free measurement's direction.
pub fn quick(font: &Font, target: &Target, at: Point, shift: bool) -> Option<([f64; 2], [f64; 2], f64)> {
    let layer = target.layer_in(font)?;
    let parts = layer.parts(target.side);
    let mut found = quick_measure_parts(font, &layer.master, &parts, at, None).ok()?;
    if shift {
        let dx = found.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
        let dy = found.get("dy").and_then(Value::as_f64).unwrap_or(0.0);
        let axis = if dx.abs() >= dy.abs() { Vec2::new(1.0, 0.0) } else { Vec2::new(0.0, 1.0) };
        found = quick_measure_parts(font, &layer.master, &parts, at, Some(axis)).ok()?;
    }
    Some((pair(found.get("from")?)?, pair(found.get("to")?)?, found.get("distance")?.as_f64().filter(|d| d.is_finite())?))
}

fn pair(v: &Value) -> Option<[f64; 2]> {
    let (x, y) = (v.get(0)?.as_f64()?, v.get(1)?.as_f64()?);
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

fn screen(cx: &PaintCx<'_>, p: [f64; 2]) -> Option<Pos2> {
    crate::canvas::paint::screen(cx, p)
}

/// The middle of a stem's edge `[[x, y], [x, y]]`.
fn edge_middle(edge: &Value) -> Option<[f64; 2]> {
    let ([ax, ay], [bx, by]) = (pair(edge.get(0)?)?, pair(edge.get(1)?)?);
    Some([(ax + bx) / 2.0, (ay + by) / 2.0])
}

pub fn autometer(cx: &PaintCx<'_>) {
    meter_shapes(cx).paint(cx);
}

pub fn meter_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let t = OverlayTokens::standard();
    let data = cx.overlays.data;
    let sw = &data.switches;
    if let Some(found) = &data.meter {
        let list = |key: &str| found.get(key).and_then(Value::as_array).into_iter().flatten().take(MAX_LABELS);
        if sw.counter_widths {
            for counter in list("counters") {
                let (Some(a), Some(b), Some(w)) = (
                    counter.get("from").and_then(pair).and_then(|p| screen(cx, p)),
                    counter.get("to").and_then(pair).and_then(|p| screen(cx, p)),
                    counter.get("width").and_then(Value::as_f64),
                ) else {
                    continue;
                };
                if a.distance(b) < t.meter_min_gap {
                    continue;
                }
                out.shapes.push(Shape::line_segment([a, b], Stroke::new(t.meter_line_width, t.counter)));
                out.label(a.lerp(b, 0.5), Align2::CENTER_BOTTOM, num(w), t.aid_label_size, t.counter);
            }
        }
        for corner in list("corners") {
            let (Some(x), Some(y), Some(angle)) =
                (corner.get("x").and_then(Value::as_f64), corner.get("y").and_then(Value::as_f64), corner.get("angle").and_then(Value::as_f64))
            else {
                continue;
            };
            if let Some(at) = screen(cx, [x, y]).filter(|p| cx.rect.contains(*p)) {
                let at = at + egui::vec2(t.corner_offset, -t.corner_offset);
                out.label(at, Align2::LEFT_BOTTOM, format!("{}°", num(angle)), t.aid_label_size, t.corner_angle);
            }
        }
        if sw.stem_widths {
            for stem in list("stems") {
                let (Some(a), Some(b), Some(width)) = (
                    stem.get("first").and_then(edge_middle).and_then(|p| screen(cx, p)),
                    stem.get("second").and_then(edge_middle).and_then(|p| screen(cx, p)),
                    stem.get("width").and_then(Value::as_f64),
                ) else {
                    continue;
                };
                let at = a.lerp(b, 0.5);
                if a.distance(b) < t.meter_min_gap || !cx.rect.contains(at) {
                    continue;
                }
                let class = stem.get("class").and_then(Value::as_str);
                let color = match class {
                    Some("common") => t.stem_common,
                    Some("uncommon") => t.stem_uncommon,
                    _ => t.stem_other,
                };
                let text = num(width);
                if class == Some("uncommon") && stem.get("editable").and_then(Value::as_bool).unwrap_or(false) {
                    let half = text.chars().count() as f32 * t.aid_label_size * 0.3 + 2.0;
                    let dot = Pos2::new(at.x + half + t.snap_dot_gap + t.snap_dot_radius, at.y);
                    out.shapes.push(Shape::circle_filled(dot, t.snap_dot_radius, color));
                }
                out.label(at, Align2::CENTER_CENTER, text, t.aid_label_size, color);
            }
        }
    }
    if let Some((from, to, distance)) = data.quick
        && let (Some(a), Some(b)) = (screen(cx, from), screen(cx, to))
    {
        out.shapes.push(Shape::line_segment([a, b], Stroke::new(t.meter_line_width, t.quick_measurement)));
        for end in [a, b] {
            out.shapes.push(Shape::circle_filled(end, t.quick_dot_radius, t.quick_measurement));
        }
        out.label(a.lerp(b, 0.5), Align2::LEFT_BOTTOM, num(distance), t.aid_label_size, t.quick_measurement);
    }
    out
}

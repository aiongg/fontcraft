//! Auto-meter labels: stem and counter widths, corner angles, the quick measurement, and their
//! View › Measure switches (`view.showStemWidths`, `view.showCounterWidths`,
//! `view.showCornerAngles`, `view.showQuickMeasurement`).
//!
//! The numbers come from the engine's `autometer.measure` and `autometer.quickMeasure`
//! (`fontcraft_engine::cmd::measurements`), run on the edited layer side. The measurement is
//! cached per document revision and layer, so the stems are found once per edit, not once per
//! frame. Clicking an editable stem label opens a field over it: Return applies the typed width,
//! Up and Down step it (Shift: ten units), each through `autometer.setStemWidth`. An uncommon
//! stem (near one of the master's stem values) has a dot beside its label that snaps it.

use std::cell::RefCell;

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke};
use fontcraft_engine::cmd::measurements::{MeterOptions, measure_parts, quick_measure_parts};
use fontcraft_geom::{Point, Vec2};
use fontcraft_model::LayerSide;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Painting, Scene};
use crate::app::FontcraftApp;
use crate::panels::{Pending, widgets};
use crate::shell::{Args, UiCommand};

/// Most stem, counter and corner labels drawn per frame.
const MAX_LABELS: usize = 2_000;
/// A stem or counter narrower than this on screen, in points, gets no label.
const MIN_LABEL_GAP: f32 = 4.0;
/// Corner angles show from this many screen points per font unit.
pub const CORNER_ZOOM: f64 = 1.5;
/// How far a corner label sits from its node, in points.
const CORNER_OFFSET: f32 = 6.0;
/// The snap dot's radius and its distance from the label, in points.
const SNAP_RADIUS: f32 = 3.5;
const SNAP_GAP: f32 = 4.0;
/// The width of the field that edits a stem's width, in points.
const FIELD_WIDTH: f32 = 48.0;

/// The switches, kept in `PanelState::autometer`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutometerState {
    pub stem_widths: bool,
    pub counter_widths: bool,
    pub corner_angles: bool,
    pub quick_measurement: bool,
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.showStemWidths",
        "Show Stem Widths",
        &["View", "Measure"],
        "",
        "{on?: bool} label each stem of the edited layer from 20 to 250 units wide with its width, coloured by how it compares with the master's stem values: one of them, near one (a dot beside the label snaps it), or neither. Clicking a label of a stem whose edges are nodes edits its width (omitted: toggle) → {on}",
    ),
    (
        "view.showCounterWidths",
        "Show Counter Widths",
        &["View", "Measure"],
        "",
        "{on?: bool} label the white space between neighbouring stems of the edited layer with its width or height (omitted: toggle) → {on}",
    ),
    (
        "view.showCornerAngles",
        "Show Corner Angles",
        &["View", "Measure"],
        "",
        "{on?: bool} show the angle at each sharp node of the edited layer's paths once the view is zoomed in far enough for the labels (omitted: toggle) → {on}",
    ),
    (
        "view.showQuickMeasurement",
        "Quick Measurement",
        &["View", "Measure"],
        "",
        "{on?: bool} while on, hovering over the edited layer shows the distance from the nearest segment through the pointer to the segment opposite; Shift measures horizontally or vertically (omitted: toggle) → {on}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let st = &mut app.state.panels.autometer;
    let flag = match id {
        "view.showStemWidths" => &mut st.stem_widths,
        "view.showCounterWidths" => &mut st.counter_widths,
        "view.showCornerAngles" => &mut st.corner_angles,
        "view.showQuickMeasurement" => &mut st.quick_measurement,
        other => return Err(format!("unknown command `{other}`")),
    };
    let on = args.toggle(*flag)?;
    *flag = on;
    Ok(json!({"on": on}))
}

/// What one cached measurement was made of: document id and revision, glyph, layer, side, and
/// which parts were asked for.
type Key = (u64, u64, String, String, bool, [bool; 3]);

thread_local! {
    static CACHE: RefCell<Option<(Key, Value)>> = const { RefCell::new(None) };
}

/// A stem label's field while it is open: which stem of which layer.
#[derive(Clone, Debug, PartialEq)]
struct Editing {
    doc: u64,
    glyph: String,
    layer: String,
    index: u64,
    /// The field has not been focused yet.
    fresh: bool,
}

fn editing_id() -> egui::Id {
    widgets::id("autometer.stemField")
}

fn side_of(scene: &Scene) -> LayerSide {
    if scene.tab.edit_background { LayerSide::Background } else { LayerSide::Foreground }
}

/// The auto-meter's measurement of the edited layer, from the cache when nothing changed.
pub fn measurement(scene: &Scene, parts: [bool; 3]) -> Option<Value> {
    let glyph = scene.tab.edited_glyph()?;
    let side = side_of(scene);
    let key: Key = (scene.doc.0, scene.doc.1, glyph.to_string(), scene.layer.id.clone(), side == LayerSide::Background, parts);
    if let Some(hit) = CACHE.with(|c| c.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v.clone())) {
        return Some(hit);
    }
    let master = scene.font.master(&scene.layer.master).ok()?;
    let options = MeterOptions { stems: parts[0], counters: parts[1], corners: parts[2], ..MeterOptions::default() };
    let value = measure_parts(scene.font, master, &scene.layer.parts(side), &options).ok()?;
    CACHE.with(|c| *c.borrow_mut() = Some((key, value.clone())));
    Some(value)
}

fn point(v: &Value) -> Option<Point> {
    Some(Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn finite(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The middle of a stem's edge `[[x, y], [x, y]]`.
fn edge_middle(edge: &Value) -> Option<Point> {
    Some(point(edge.get(0)?)?.midpoint(point(edge.get(1)?)?))
}

/// Where a stem's label goes on screen, and the screen points between its edges.
pub fn stem_label(c: &Painting, stem: &Value) -> Option<(Pos2, f32)> {
    let (a, b) = (edge_middle(stem.get("first")?)?, edge_middle(stem.get("second")?)?);
    let (sa, sb) = (pos(c.view.to_screen(a)), pos(c.view.to_screen(b)));
    let at = sa.lerp(sb, 0.5);
    finite(at).then_some((at, sa.distance(sb)))
}

/// The screen rectangle a label of `text` covers, centred at `at`.
fn label_rect(c: &Painting, at: Pos2, text: &str) -> Rect {
    let size = c.tokens.measure_label_size;
    let w = text.chars().count() as f32 * size * 0.6 + 4.0;
    Rect::from_center_size(at, egui::vec2(w, size + 4.0))
}

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let st = &scene.state.panels.autometer;
    if st.stem_widths || st.counter_widths || st.corner_angles {
        meter(ui, c, scene, pending, st);
    }
    if st.quick_measurement {
        quick(ui, c, scene);
    }
}

fn meter(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending, st: &AutometerState) {
    let corners = st.corner_angles && c.view.scale() >= CORNER_ZOOM;
    let Some(found) = measurement(scene, [st.stem_widths, st.counter_widths, corners]) else { return };
    let t = c.tokens;
    let font = FontId::proportional(t.measure_label_size);
    let empty = Vec::new();
    let list = |key: &str| found.get(key).and_then(Value::as_array).unwrap_or(&empty).iter().take(MAX_LABELS);
    if st.counter_widths {
        for counter in list("counters") {
            let (Some(a), Some(b), Some(w)) =
                (counter.get("from").and_then(point), counter.get("to").and_then(point), counter.get("width").and_then(Value::as_f64))
            else {
                continue;
            };
            let (sa, sb) = (pos(c.view.to_screen(a)), pos(c.view.to_screen(b)));
            if !(finite(sa) && finite(sb)) || sa.distance(sb) < MIN_LABEL_GAP {
                continue;
            }
            c.painter.line_segment([sa, sb], Stroke::new(t.measure_ruler_width, t.text_dim));
            c.painter.text(sa.lerp(sb, 0.5), Align2::CENTER_BOTTOM, widgets::format_number(w), font.clone(), t.text_dim);
        }
    }
    if corners {
        for corner in list("corners") {
            let (Some(x), Some(y), Some(angle)) =
                (corner.get("x").and_then(Value::as_f64), corner.get("y").and_then(Value::as_f64), corner.get("angle").and_then(Value::as_f64))
            else {
                continue;
            };
            let at = pos(c.view.to_screen(Point::new(x, y)));
            if finite(at) && c.rect.contains(at) {
                let text = format!("{}°", widgets::format_number(angle));
                c.painter.text(at + egui::vec2(CORNER_OFFSET, -CORNER_OFFSET), Align2::LEFT_BOTTOM, text, font.clone(), t.measure_off_curve);
            }
        }
    }
    if !st.stem_widths {
        return;
    }
    let Some(glyph) = scene.tab.edited_glyph() else { return };
    let side = side_of(scene).as_str();
    let open =
        ui.ctx().data(|d| d.get_temp::<Editing>(editing_id())).filter(|e| e.doc == scene.doc.0 && e.glyph == glyph && e.layer == scene.layer.id);
    // The stem of the edited layer and what to do to it (`snap` or `width`).
    let target = |index: u64, key: &str, value: Value| json!({"glyph": glyph, "layer": scene.layer.id, "side": side, "stem": index, key: value});
    for stem in list("stems") {
        let (Some((at, gap)), Some(width), Some(index)) =
            (stem_label(c, stem), stem.get("width").and_then(Value::as_f64), stem.get("index").and_then(Value::as_u64))
        else {
            continue;
        };
        if gap < MIN_LABEL_GAP || !c.rect.contains(at) {
            continue;
        }
        let color = match stem.get("class").and_then(Value::as_str) {
            Some("common") => t.measure_delta,
            Some("uncommon") => t.audit_warn,
            _ => t.measure_ruler,
        };
        let text = widgets::format_number(width);
        let rect = label_rect(c, at, &text);
        if open.as_ref().is_some_and(|e| e.index == index) {
            continue;
        }
        c.painter.text(at, Align2::CENTER_CENTER, &text, font.clone(), color);
        let editable = stem.get("editable").and_then(Value::as_bool).unwrap_or(false);
        if editable && ui.interact(rect, ui.id().with(("autometer.stem", index)), Sense::click()).on_hover_text("Click to type a width").clicked() {
            let e = Editing { doc: scene.doc.0, glyph: glyph.to_string(), layer: scene.layer.id.clone(), index, fresh: true };
            ui.ctx().data_mut(|d| d.insert_temp(editing_id(), e));
        }
        if editable && stem.get("class").and_then(Value::as_str) == Some("uncommon") {
            let dot = Pos2::new(rect.right() + SNAP_GAP + SNAP_RADIUS, at.y);
            c.painter.circle_filled(dot, SNAP_RADIUS, color);
            let hit = Rect::from_center_size(dot, egui::Vec2::splat(SNAP_RADIUS * 3.0));
            let hint = match stem.get("nearest").and_then(Value::as_f64) {
                Some(n) => format!("Snap to {}", widgets::format_number(n)),
                None => "Snap to the nearest stem value".to_string(),
            };
            if ui.interact(hit, ui.id().with(("autometer.snap", index)), Sense::click()).on_hover_text(hint).clicked() {
                pending.push(("autometer.setStemWidth", target(index, "snap", json!(true))));
            }
        }
    }
    let Some(e) = open else { return };
    let Some((at, width)) = list("stems")
        .find(|s| s.get("index").and_then(Value::as_u64) == Some(e.index))
        .and_then(|s| Some((stem_label(c, s)?.0, s.get("width").and_then(Value::as_f64)?)))
    else {
        ui.ctx().data_mut(|d| d.remove::<Editing>(editing_id()));
        return;
    };
    let field = widgets::id(("autometer.stemWidth", e.index));
    let shown = widgets::format_number(width);
    let step = |text: &str, delta: f64| text.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| widgets::format_number(n + delta));
    let mut close = false;
    egui::Area::new(widgets::id("autometer.stemArea")).order(egui::Order::Foreground).fixed_pos(at).pivot(Align2::CENTER_CENTER).show(
        ui.ctx(),
        |ui| {
            let (commit, response) = widgets::line(ui, field, &shown, FIELD_WIDTH, widgets::Look::default(), &step);
            if e.fresh {
                response.request_focus();
                let mut next = e.clone();
                next.fresh = false;
                ui.ctx().data_mut(|d| d.insert_temp(editing_id(), next));
            } else if !response.has_focus() {
                close = true;
            }
            if let Some(n) = commit.and_then(|text| text.trim().parse::<f64>().ok()).filter(|n| n.is_finite()) {
                pending.push(("autometer.setStemWidth", target(e.index, "width", json!(n))));
            }
        },
    );
    if close {
        ui.ctx().data_mut(|d| d.remove::<Editing>(editing_id()));
    }
}

/// The quick measurement under the pointer: a line across the outline and its length.
fn quick(ui: &egui::Ui, c: &Painting, scene: &Scene) {
    let (hover, shift) = ui.input(|i| (i.pointer.hover_pos(), i.modifiers.shift));
    let Some(hover) = hover.filter(|p| c.rect.contains(*p)) else { return };
    let at = c.view.to_font(Point::new(f64::from(hover.x), f64::from(hover.y)));
    let parts = scene.layer.parts(side_of(scene));
    let master = scene.layer.master.as_str();
    let Ok(mut found) = quick_measure_parts(scene.font, master, &parts, at, None) else { return };
    if shift {
        let dx = found.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
        let dy = found.get("dy").and_then(Value::as_f64).unwrap_or(0.0);
        let axis = if dx.abs() >= dy.abs() { Vec2::new(1.0, 0.0) } else { Vec2::new(0.0, 1.0) };
        let Ok(along) = quick_measure_parts(scene.font, master, &parts, at, Some(axis)) else { return };
        found = along;
    }
    let (Some(a), Some(b), Some(d)) =
        (found.get("from").and_then(point), found.get("to").and_then(point), found.get("distance").and_then(Value::as_f64))
    else {
        return;
    };
    let (sa, sb) = (pos(c.view.to_screen(a)), pos(c.view.to_screen(b)));
    if !(finite(sa) && finite(sb)) {
        return;
    }
    let t = c.tokens;
    c.painter.line_segment([sa, sb], Stroke::new(t.measure_ruler_width, t.measure_ruler));
    for end in [sa, sb] {
        c.painter.circle_filled(end, t.measure_crossing_radius, t.measure_ruler);
    }
    c.painter.text(sa.lerp(sb, 0.5), Align2::LEFT_BOTTOM, widgets::format_number(d), FontId::proportional(t.measure_label_size), t.measure_ruler);
}

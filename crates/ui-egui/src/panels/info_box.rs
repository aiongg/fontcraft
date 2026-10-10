//! The selection part of the Info box, at the foot of the canvas while Show Info is on.
//!
//! It shows what `selection.getInfo` reports for the Edit View selection at the reference point
//! chosen on the nine-point grid (`infoBox.setReferencePoint`):
//!
//! | Selection | Fields | Editing runs |
//! |---|---|---|
//! | one node | X, Y | `selection.setPosition` |
//! | one handle | X, Y, dX, dY and length to its node | `selection.setPosition` on the handle |
//! | anything else | X, Y of the box, width, height, lock | `selection.setPosition`; `transform.scale` about the reference point |
//!
//! With several points selected it adds the selected and the total point count; with two nodes,
//! their distance. At its left a split square shows the edited glyph's colour label on its left
//! half and the active layer's on its right half (`infoBox.showsColorLabels`), when either is
//! set.
//!
//! While one of the Annotation tools is the chosen tool, the box shows the annotation palette
//! instead (`infoBox.annotations`): one button per sub-tool, in the order of
//! `fontcraft_tools::annotate::SUB_TOOLS`, each running `annotation.setTool`.

use fontcraft_model::{ColorLabel, Shape};
use fontcraft_tools::TOOLS;
use fontcraft_tools::annotate::SUB_TOOLS;
use serde_json::{Map, Value, json};

use super::{Pending, edit_targets, has_lists, widgets, with};
use crate::app::FontcraftApp;
use crate::icons;
use crate::theme::Tokens;
use crate::tools_glue;

/// What the box needs, read before drawing.
struct Info {
    info: Value,
    reference: String,
    locked: bool,
    /// Targets of `selection.setPosition` and of `transform.scale`.
    place: Map<String, Value>,
    scale: Map<String, Value>,
    /// Selected points and points in the layer.
    counts: (usize, usize),
    /// The colour labels of the edited glyph and of the active layer.
    colors: (Option<ColorLabel>, Option<ColorLabel>),
}

fn read(app: &mut FontcraftApp) -> Option<Info> {
    let reference = app.state.panels.reference_point.clone();
    let mut query = edit_targets(app, "selection.getInfo").filter(has_lists)?;
    query.insert("referencePoint".into(), json!(reference));
    let place = edit_targets(app, "selection.setPosition")?;
    let scale = edit_targets(app, "transform.scale")?;
    let selected = app.state.tab().map_or(0, |t| t.selection.points.len());
    let total = match (app.session.font(), app.state.tab()) {
        (Ok(font), Some(tab)) => tools_glue::scene(font, tab)
            .map_or(0, |scene| scene.layer.shapes.iter().map(|s| if let Shape::Path(p) = s { p.nodes.len() } else { 0 }).sum()),
        _ => 0,
    };
    let colors = match (app.session.font(), app.state.tab()) {
        (Ok(font), Some(tab)) => (
            tab.edited_glyph().and_then(|g| font.glyph(g)).and_then(|g| g.info.color),
            crate::canvas::view::active_layer(font, tab).and_then(|l| l.info.color),
        ),
        _ => (None, None),
    };
    let info = app.session.execute("selection.getInfo", &Value::Object(query)).ok()?;
    Some(Info { info, reference, locked: app.state.panels.size_locked, place, scale, counts: (selected, total), colors })
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let tokens = app.tokens.clone();
    // The chosen tool, so holding Space for the Hand tool keeps the palette.
    let tool = app.tools.toolbox.base_id();
    if SUB_TOOLS.iter().any(|(_, id, _)| *id == tool) {
        frame(ui, &tokens, "infoBox.annotations", |ui| annotations(ui, tool, pending));
        return;
    }
    let Some(info) = read(app) else { return };
    frame(ui, &tokens, "infoBox", |ui| contents(ui, &tokens, &info, pending));
}

/// The box itself: centred at the foot of the canvas.
fn frame(ui: &mut egui::Ui, tokens: &Tokens, salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    let canvas = ui.available_rect_before_wrap();
    let pos = egui::pos2(canvas.center().x, canvas.bottom() - tokens.info_box_margin);
    egui::Area::new(widgets::id(salt)).order(egui::Order::Middle).fixed_pos(pos).pivot(egui::Align2::CENTER_BOTTOM).constrain_to(canvas).show(
        ui.ctx(),
        |ui| {
            egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, add);
        },
    );
}

/// The annotation palette: the button of the active sub-tool is drawn selected.
fn annotations(ui: &mut egui::Ui, active: &str, pending: &mut Pending) {
    ui.horizontal(|ui| {
        for &(name, id, _) in SUB_TOOLS {
            let Some(icon) = icons::for_tool(id) else { continue };
            let hover = TOOLS.iter().find(|t| t.id == id).map_or(name, |t| t.label);
            if widgets::icon_button(ui, widgets::id(("infoBox.annotation", name)), icon, hover, true, id == active) {
                pending.push(("annotation.setTool", json!({ "tool": name })));
            }
        }
    });
}

fn contents(ui: &mut egui::Ui, tokens: &Tokens, s: &Info, pending: &mut Pending) {
    let width = tokens.field_width;
    let place = |pending: &mut Pending, x: Option<f64>, y: Option<f64>| {
        let mut extra = Map::new();
        if let Some(x) = x {
            extra.insert("x".into(), json!(x));
        }
        if let Some(y) = y {
            extra.insert("y".into(), json!(y));
        }
        extra.insert("referencePoint".into(), json!(s.reference));
        pending.push(("selection.setPosition", with(s.place.clone(), Value::Object(extra))));
    };
    let field = |ui: &mut egui::Ui, label: &str, salt: &str, value: Option<f64>| -> Option<f64> {
        ui.label(egui::RichText::new(label).color(tokens.text_dim));
        match widgets::number(ui, widgets::id(salt), value, width, false) {
            Some(widgets::NumberEdit::Set(v)) => Some(v),
            _ => None,
        }
    };
    ui.horizontal(|ui| {
        color_labels(ui, tokens, s.colors);
        if let Some(point) = widgets::point_grid(ui, tokens, "infoBox.reference", Some(&s.reference)) {
            pending.push(("infoBox.setReferencePoint", json!({ "point": point })));
        }
        let info = &s.info;
        if let Some(node) = info.get("node") {
            if let Some(x) = field(ui, "X", "infoBox.x", num(node, "x")) {
                place(pending, Some(x), None);
            }
            if let Some(y) = field(ui, "Y", "infoBox.y", num(node, "y")) {
                place(pending, None, Some(y));
            }
        } else if let Some(handle) = info.get("handle") {
            let (hx, hy, dx, dy) = (num(handle, "x"), num(handle, "y"), num(handle, "dx"), num(handle, "dy"));
            // The handle's node, from which dX, dY and the length are measured.
            let owner = hx.zip(dx).map(|(h, d)| h - d).zip(hy.zip(dy).map(|(h, d)| h - d));
            if let Some(x) = field(ui, "X", "infoBox.x", hx) {
                place(pending, Some(x), None);
            }
            if let Some(y) = field(ui, "Y", "infoBox.y", hy) {
                place(pending, None, Some(y));
            }
            if let Some(v) = field(ui, "dX", "infoBox.dx", dx)
                && let Some((ox, _)) = owner
            {
                place(pending, Some(ox + v), None);
            }
            if let Some(v) = field(ui, "dY", "infoBox.dy", dy)
                && let Some((_, oy)) = owner
            {
                place(pending, None, Some(oy + v));
            }
            let length = num(handle, "length");
            if let Some(v) = field(ui, "L", "infoBox.length", length)
                && let (Some((ox, oy)), Some(dx), Some(dy), Some(l)) = (owner, dx, dy, length)
                && l > 0.0
            {
                let k = v / l;
                place(pending, Some(ox + dx * k), Some(oy + dy * k));
            }
        } else {
            if let Some(x) = field(ui, "X", "infoBox.x", num(info, "x")) {
                place(pending, Some(x), None);
            }
            if let Some(y) = field(ui, "Y", "infoBox.y", num(info, "y")) {
                place(pending, None, Some(y));
            }
            let (w, h) = (num(info, "width"), num(info, "height"));
            let scale = |pending: &mut Pending, [fx, fy]: [f64; 2]| {
                let extra = json!({"x": fx * 100.0, "y": fy * 100.0, "origin": {"mode": "box", "boxPoint": s.reference}});
                pending.push(("transform.scale", with(s.scale.clone(), extra)));
            };
            if let Some(v) = field(ui, "W", "infoBox.width", w)
                && let Some(old) = w.filter(|w| *w > 0.0)
            {
                let k = v / old;
                scale(pending, [k, if s.locked { k } else { 1.0 }]);
            }
            let lock = if s.locked { "🔒" } else { "🔓" };
            if widgets::button(ui, widgets::id("infoBox.lock"), lock, "Lock Info Box Proportions", true, s.locked) {
                pending.push(("infoBox.setSizeLock", json!({ "locked": !s.locked })));
            }
            if let Some(v) = field(ui, "H", "infoBox.height", h)
                && let Some(old) = h.filter(|h| *h > 0.0)
            {
                let k = v / old;
                scale(pending, [if s.locked { k } else { 1.0 }, k]);
            }
            if let Some(d) = info.get("distance").and_then(|d| num(d, "length")) {
                ui.label(egui::RichText::new(format!("↦ {}", widgets::format_number(d))).color(tokens.text_dim))
                    .on_hover_text("Distance between the two nodes");
            }
        }
        let (selected, total) = s.counts;
        if selected > 1 {
            ui.separator();
            ui.label(format!("■ {selected}")).on_hover_text("Selected points");
            ui.label(format!("□ {total}")).on_hover_text("Points in the layer");
        }
    });
}

/// The glyph colour label on the left half of a square and the layer colour label on the right
/// half; nothing when neither is set.
fn color_labels(ui: &mut egui::Ui, tokens: &Tokens, (glyph, layer): (Option<ColorLabel>, Option<ColorLabel>)) {
    if glyph.is_none() && layer.is_none() {
        return;
    }
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let mid = rect.center().x;
    let halves = [
        (egui::Rect::from_min_max(rect.min, egui::pos2(mid, rect.max.y)), glyph),
        (egui::Rect::from_min_max(egui::pos2(mid, rect.min.y), rect.max), layer),
    ];
    for (half, label) in halves {
        if let Some(label) = label {
            ui.painter().rect_filled(half, 0.0, crate::shell::context_menu::label_color(tokens, label));
        }
    }
    ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);
    response.on_hover_text("Glyph color (left) and layer color (right)");
}

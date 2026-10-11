//! The glyph box of the Info box: the current glyph's name, Unicode value, sidebearings and
//! width, with its kerning at the outer edges (`info_box_kerning`), drawn under the glyph while
//! Show Info is on.
//!
//! The current glyph is the edited glyph, or in text mode the glyph after the cursor
//! (`Tab::edited_glyph`); the layer is the tab's active layer of it. Values and keys come from
//! `spacing.getMetrics`.
//!
//! | Layout | Fields | Editing runs |
//! |---|---|---|
//! | left to right, right to left | LSB and RSB either side of the metrics mark, width below it | `spacing.setLeftSidebearing`, `spacing.setRightSidebearing`, `spacing.setWidth` |
//! | top to bottom | T and B either side of the mark; O and H (vertical origin and height) below, shown only | `spacing.setTopSidebearing`, `spacing.setBottomSidebearing` |
//!
//! The kern field of each edge sits at that edge on the metrics row, its group field and lock
//! below it.
//!
//! A field holds a number or a metrics key and follows the field rules of `widgets`; in a field
//! that holds a key Up and Down do nothing. A key is drawn in the text colour while its value
//! matches the metric; out of sync it is red with an update button beside it
//! (`spacing.updateMetricsKey`); a key naming a glyph the font lacks is turquoise. Emptying a
//! field that holds a key stores the metric's current number, which removes the key. A master
//! that takes its metrics from another master shows that master's values, read only.

use fontcraft_geom::Point;
use serde_json::{Value, json};

use super::info_box_kerning::{self as kerning, Kerning};
use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::icons::Icon;
use crate::state::WritingDirection;
use crate::theme::Tokens;
use crate::tools_glue;

/// What a metrics field's key is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// No key: the field holds a number.
    Number,
    InSync,
    /// The key's value differs from the metric.
    OutOfSync,
    /// The key names a glyph the font does not have.
    Unlinked,
    /// The key cannot be evaluated (a cycle, a division by zero, a result out of range).
    Error,
}

/// One metric field as the box shows it.
#[derive(Clone, Debug)]
pub struct MetricField {
    /// `left`, `right`, `width`, `top` or `bottom`, as `spacing.getMetrics` names it.
    pub field: &'static str,
    /// The setter that typing into the field runs.
    pub command: &'static str,
    /// The metric as it is; `None` for a sidebearing of an empty layer.
    pub value: Option<f64>,
    /// The metrics key that applies, as a field shows it.
    pub key: Option<String>,
    pub state: KeyState,
    /// Why the key is out of sync, unlinked or wrong, for the hover text.
    pub message: Option<String>,
}

impl MetricField {
    /// The text the field shows: the key, else the number.
    pub fn shown(&self) -> String {
        match (&self.key, self.value) {
            (Some(key), _) => key.clone(),
            (None, Some(v)) => widgets::format_number(v),
            (None, None) => String::new(),
        }
    }
}

/// What the box needs, read before drawing.
#[derive(Clone, Debug)]
pub struct Metrics {
    pub glyph: String,
    pub unicode: Option<u32>,
    /// The layer the setters edit.
    pub layer: String,
    pub vertical: bool,
    /// LSB, RSB, width; or T and B in a vertical layout.
    pub fields: Vec<MetricField>,
    /// Vertical origin and vertical width (height) of the layer.
    pub origin: f64,
    pub height: f64,
    /// The name of the master whose metrics the layer's master shows; the fields are then read
    /// only.
    pub linked: Option<String>,
    /// Where the box hangs from: under the glyph's descender, centred on its advance.
    pub anchor: egui::Pos2,
}

/// The colour a field's text is drawn in.
pub fn key_color(tokens: &Tokens, state: KeyState) -> egui::Color32 {
    match state {
        KeyState::Number | KeyState::InSync => tokens.text,
        KeyState::OutOfSync | KeyState::Error => tokens.info_box_key_out_of_sync,
        KeyState::Unlinked => tokens.info_box_key_unlinked,
    }
}

/// The fields of each layout, with their setters.
const HORIZONTAL: [(&str, &str); 3] =
    [("left", "spacing.setLeftSidebearing"), ("right", "spacing.setRightSidebearing"), ("width", "spacing.setWidth")];
const VERTICAL: [(&str, &str); 2] = [("top", "spacing.setTopSidebearing"), ("bottom", "spacing.setBottomSidebearing")];

/// The current glyph's metrics for the showing tab; `None` without a current glyph.
pub fn read(app: &mut FontcraftApp, tokens: &Tokens) -> Option<Metrics> {
    let center = tools_glue::canvas_center(app);
    let tab = app.state.tab()?;
    let font = app.session.font().ok()?;
    let glyph = font.glyph(tab.edited_glyph()?)?;
    let layer = view::active_layer(font, tab)?;
    let master = view::active_master(font, tab)?;
    let (descender, ascender) = view::vertical_span(font, master);
    let origin = view::edited_origin(&app.session, &app.layouts, tab);
    let width = view::edited_width(font, tab);
    let p = view::tools_view(&tab.view, origin, center).to_screen(Point::new(width / 2.0, descender));
    let anchor = egui::pos2(p.x as f32, p.y as f32 + tokens.info_box_glyph_gap);
    let vertical = tab.direction == WritingDirection::Ttb;
    let name = glyph.name.clone();
    let unicode = glyph.unicodes.first().copied();
    let edited = layer.id.clone();
    let vertical_origin = layer.info.vert_origin.filter(|v| v.is_finite()).unwrap_or(0.0);
    let height = layer.info.vert_width.filter(|v| v.is_finite()).unwrap_or(ascender - descender);
    // A linked master shows the layer of the master it follows.
    let source = fontcraft_spacing::metrics_source(font, &layer.master).filter(|m| m.id != layer.master);
    let (shown, linked) = match source {
        Some(m) => (glyph.layer(&m.id).map(|l| l.id.clone())?, Some(m.name.clone())),
        None => (edited.clone(), None),
    };
    let reply = app.session.execute("spacing.getMetrics", &json!({"glyph": name, "layer": shown})).ok()?;
    let specs: &[(&str, &str)] = if vertical { &VERTICAL } else { &HORIZONTAL };
    let mut fields: Vec<MetricField> = specs.iter().map(|&(field, command)| field_state(&reply, field, command)).collect();
    if fields.iter().any(|f| f.state == KeyState::Error) {
        mark_unlinked(app, &name, &shown, &mut fields);
    }
    Some(Metrics { glyph: name, unicode, layer: edited, vertical, fields, origin: vertical_origin, height, linked, anchor })
}

fn field_state(reply: &Value, field: &'static str, command: &'static str) -> MetricField {
    let value = reply.get(field).and_then(Value::as_f64).filter(|v| v.is_finite());
    let entry = reply.get("keys").and_then(|k| k.get(field)).filter(|e| e.is_object());
    let Some(entry) = entry else { return MetricField { field, command, value, key: None, state: KeyState::Number, message: None } };
    let key = entry.get("key").and_then(Value::as_str).map(str::to_string);
    let error = entry.get("error").and_then(Value::as_str).map(str::to_string);
    let target = entry.get("value").and_then(Value::as_f64);
    let (state, message) = match (entry.get("inSync").and_then(Value::as_bool), error) {
        (_, Some(e)) => (KeyState::Error, Some(e)),
        (Some(false), None) => {
            let target = target.map_or_else(String::new, widgets::format_number);
            let actual = value.map_or_else(String::new, widgets::format_number);
            (KeyState::OutOfSync, Some(format!("Out of sync: the key gives {target}, the metric is {actual}")))
        }
        _ => (KeyState::InSync, target.map(|t| format!("= {}", widgets::format_number(t)))),
    };
    MetricField { field, command, value, key, state, message }
}

/// Marks the keys among `fields` that name a missing glyph, from the out-of-sync scan.
fn mark_unlinked(app: &mut FontcraftApp, glyph: &str, layer: &str, fields: &mut [MetricField]) {
    let Ok(reply) = app.session.execute("spacing.listOutOfSyncMetrics", &json!({"glyphs": [glyph]})) else { return };
    let Some(findings) = reply.get("findings").and_then(Value::as_array) else { return };
    for finding in findings {
        let at = |k: &str| finding.get(k).and_then(Value::as_str);
        if at("problem") != Some("unlinked") || at("layer") != Some(layer) {
            continue;
        }
        if let Some(f) = fields.iter_mut().find(|f| Some(f.field) == at("field")) {
            f.state = KeyState::Unlinked;
            f.message = Some(format!("Names a glyph that is not in the font: {}", at("missing").unwrap_or("")));
        }
    }
}

/// Draws the box under the current glyph, and the preceding glyph's box beside it with the
/// Text tool. Call from the Info box while Show Info is on.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let tokens = app.tokens.clone();
    let Some(m) = read(app, &tokens) else { return };
    let k = kerning::read(app);
    let canvas = ui.available_rect_before_wrap();
    let shown = egui::Area::new(widgets::id("infoBox.glyph"))
        .order(egui::Order::Middle)
        .fixed_pos(m.anchor)
        .pivot(egui::Align2::CENTER_TOP)
        .constrain_to(canvas)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| contents(ui, &tokens, &m, k.as_ref(), pending));
        });
    if let Some(k) = &k {
        kerning::show_preceding(ui.ctx(), canvas, &tokens, k, shown.response.rect, pending);
    }
}

/// The box: five columns, the kerning of each edge outermost.
///
/// | | | name | Unicode | |
/// | kern | LSB (T) | mark | RSB (B) | kern |
/// | group, lock | | width (O, H) | | lock, group |
fn contents(ui: &mut egui::Ui, tokens: &Tokens, m: &Metrics, k: Option<&Kerning>, pending: &mut Pending) {
    let dim = |text: &str| egui::RichText::new(text).color(tokens.text_dim);
    let field = |ui: &mut egui::Ui, pending: &mut Pending, name: &str| {
        if let Some(f) = m.fields.iter().find(|f| f.field == name) {
            metric(ui, tokens, m, f, pending);
        }
    };
    let kern = |ui: &mut egui::Ui, pending: &mut Pending, edge: usize| match k {
        Some(k) => kerning::edge_kern(ui, tokens, k, edge, pending),
        None => {
            ui.label("");
        }
    };
    let group = |ui: &mut egui::Ui, pending: &mut Pending, edge: usize| match k {
        Some(k) => {
            ui.horizontal(|ui| kerning::edge_group(ui, tokens, k, edge, pending));
        }
        None => {
            ui.label("");
        }
    };
    // Columns start at a field's width, so the box takes its size within a frame or two.
    egui::Grid::new(widgets::id("infoBox.glyphGrid")).num_columns(5).min_col_width(tokens.field_width).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.label("");
        ui.label("");
        // A long name widens the middle column rather than wrapping.
        ui.vertical_centered(|ui| ui.add(egui::Label::new(egui::RichText::new(&m.glyph).strong()).wrap_mode(egui::TextWrapMode::Extend)));
        match m.unicode {
            Some(u) => ui.label(dim(&format!("{u:04X}"))),
            None => ui.label(""),
        };
        ui.label("");
        ui.end_row();
        kern(ui, pending, 0);
        if m.vertical {
            ui.horizontal(|ui| {
                ui.label(dim("T"));
                field(ui, pending, "top");
            });
            ui.vertical_centered(|ui| ui.label(dim("↕")).on_hover_text("Top and bottom sidebearings"));
            ui.horizontal(|ui| {
                ui.label(dim("B"));
                field(ui, pending, "bottom");
            });
        } else {
            field(ui, pending, "left");
            ui.vertical_centered(|ui| ui.label(dim("↔")).on_hover_text("Left and right sidebearings, width below"));
            field(ui, pending, "right");
        }
        kern(ui, pending, 1);
        ui.end_row();
        group(ui, pending, 0);
        if m.vertical {
            ui.label(dim(&format!("O {}", widgets::format_number(m.origin)))).on_hover_text("Vertical origin");
            ui.label("");
            ui.label(dim(&format!("H {}", widgets::format_number(m.height)))).on_hover_text("Vertical width");
        } else {
            ui.label("");
            ui.vertical_centered(|ui| field(ui, pending, "width"));
            ui.label("");
        }
        group(ui, pending, 1);
        ui.end_row();
    });
    if let Some(master) = &m.linked {
        ui.label(dim(&format!("Metrics of master {master}")));
    }
}

/// One metric field, with its update button while its key is out of sync.
fn metric(ui: &mut egui::Ui, tokens: &Tokens, m: &Metrics, f: &MetricField, pending: &mut Pending) {
    let shown = f.shown();
    let color = key_color(tokens, f.state);
    let id = widgets::id(("infoBox.metric", f.field));
    ui.horizontal(|ui| {
        // A linked master's metrics, and a sidebearing of an empty layer without a key, are not
        // edited here.
        let editable = m.linked.is_none() && (f.value.is_some() || f.key.is_some());
        if !editable {
            let response =
                ui.add_sized([tokens.field_width, ui.spacing().interact_size.y], egui::Label::new(egui::RichText::new(&shown).color(color)));
            if let Some(message) = &f.message {
                response.on_hover_text(message);
            }
            return;
        }
        let step = |text: &str, delta: f64| -> Option<String> {
            if f.key.is_some() {
                return None;
            }
            let base = text.trim().parse::<f64>().ok().filter(|n| n.is_finite()).or(f.value)?;
            Some(widgets::format_number(base + delta))
        };
        let look = widgets::Look { color: Some(color), placeholder: None };
        let (text, response) = widgets::line(ui, id, &shown, tokens.field_width, look, &step);
        if let Some(message) = &f.message {
            response.on_hover_text(message);
        }
        if let Some(text) = text
            && let Some(value) = commit_value(&text, f)
        {
            pending.push((f.command, json!({"glyphs": [m.glyph], "layer": m.layer, "value": value})));
        }
        if f.state == KeyState::OutOfSync
            && widgets::icon_button(ui, widgets::id(("infoBox.updateMetric", f.field)), Icon::Update, "Update Metrics Key", true, false)
        {
            pending.push(("spacing.updateMetricsKey", json!({"glyph": m.glyph, "layer": m.layer, "field": f.field})));
        }
    });
}

/// The value a committed text sets: a number, a key, or for an emptied field that held a key the
/// metric's current number (which removes the key). `None`: nothing to set.
fn commit_value(text: &str, f: &MetricField) -> Option<Value> {
    let t = text.trim();
    if t.is_empty() {
        return f.key.as_ref().and(f.value).map(|v| json!(v));
    }
    match t.parse::<f64>() {
        Ok(n) if n.is_finite() => Some(json!(n)),
        _ => Some(json!(t)),
    }
}

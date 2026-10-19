//! The Axis Mappings dialog (M9.23): the points of each axis map, edited with the mouse and the
//! arrow keys through the `axisMappings.*` commands.
//!
//! Rows: `dialog.axisMappings`, `axisMappings.showDialog`, `axisMappings.dialogEdit`,
//! `axisMappings.closeDialog`.
//!
//! Three panes: the axes; the points of the selected axis, internal coordinate left and external
//! right; a graph of the same points, internal across and external up, joined by a line in the
//! accent colour. The dialog edits the font's Axis Mappings, or a variable font setting's when it
//! opens on one.
//!
//! The dialog is modal and previews like any dialog: opening it begins an interaction, every edit
//! is one `axisMappings.*` command, and the edits made so far are previewed together as one
//! `command.batch` on the font as it was when the dialog opened. OK keeps them as one undo step
//! (the journal records the batch); Cancel gives back the very font the dialog opened on. An edit
//! the command refuses (points out of order) changes nothing. Repeated moves of the same point
//! are merged into one step, so dragging and holding an arrow key keep the batch short.
//!
//! Mouse and keys: a click on a point selects it; dragging it up or down moves its external
//! value (whole units); a click on the line adds a point there, on the line, so the mapping is
//! unchanged until it moves; the arrow keys move the selected point (up and down: external, left
//! and right: internal) by 1, by 10 with Shift, by 100 with Cmd. Return is OK and Escape is
//! Cancel while no value is being typed.

use std::sync::Arc;

use egui::{Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use fontcraft_engine::fontinfo::target;
use fontcraft_engine::model::Font;
use fontcraft_engine::model::axes::{AXIS_MAPPINGS, mapping};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

/// The dialog's title, also the label of the interaction and of the undo step OK makes.
const TITLE: &str = "Axis Mappings";
/// Most edits one opening of the dialog previews.
pub const MAX_EDITS: usize = 1024;
/// The commands an edit may run.
const EDITS: [&str; 4] = ["axisMappings.addPoint", "axisMappings.setPoint", "axisMappings.nudgePoint", "axisMappings.removePoint"];
/// The graph is this many dialog fields wide and high.
const GRAPH_FIELDS: f32 = 3.0;
/// Room around the points inside the graph, as a share of the coordinate range.
const GRAPH_MARGIN: f64 = 0.06;
/// The selected point is drawn this much larger than the others.
const SELECTED_SCALE: f32 = 1.7;

pub const COMMANDS: &[UiCommand] = &[
    (
        "axisMappings.showDialog",
        "Axis Mappings…",
        &[],
        "",
        "{variableFontSetting?: id|name|index, axis?: tag} open the Axis Mappings dialog on the font's mappings, or on a variable font setting's: the axes, the points of the selected axis (default the first) and a graph of them. Edits (axisMappings.dialogEdit) preview until axisMappings.closeDialog; refused while another dialog is open → the state axisMappings.dialogEdit returns",
    ),
    (
        "axisMappings.dialogEdit",
        "Edit in Axis Mappings Dialog",
        &[],
        "",
        "{axis?: tag, point?: integer|null, command?: \"axisMappings.addPoint\"|\"axisMappings.setPoint\"|\"axisMappings.nudgePoint\"|\"axisMappings.removePoint\", params?: {…}} in the open Axis Mappings dialog: select an axis and a point (counted from 0 in external order), then run one edit with the command's parameters on the preview; `axis` and `variableFontSetting` are the dialog's, `index` defaults to the selected point. The edited point becomes the selection. A refused edit changes nothing → {open, variableFontSetting, axis, point, axes: [{tag, name, points: [{internal, external}]}], edits}",
    ),
    (
        "axisMappings.closeDialog",
        "Close Axis Mappings Dialog",
        &[],
        "",
        "{confirm?: bool = false} close the Axis Mappings dialog: OK (confirm: true) keeps its edits as one undo step, Cancel gives back the font as it was when the dialog opened → {confirmed, changed}",
    ),
];

/// The open dialog, kept in `UiState::panels` and not saved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AxisMappingsState {
    pub open: bool,
    /// The variable font setting whose mappings are edited, as given; `None`: the font's.
    pub setting: Option<Value>,
    /// The selected axis, by tag.
    pub axis: String,
    /// The selected point of that axis, counted in external order.
    pub point: Option<usize>,
    /// The edits since the dialog opened, as `command.batch` steps.
    pub steps: Vec<Value>,
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "axisMappings.showDialog" => show_dialog(app, args),
        "axisMappings.dialogEdit" => edit(app, args),
        "axisMappings.closeDialog" => close(app, args),
        other => Err(format!("unknown command `{other}`")),
    }
}

// ── Reading ─────────────────────────────────────────────────────────────────────────────────────

/// The variable font setting `raw` names, as an index into `Font::instances`.
fn setting_index(font: &Font, raw: Option<&Value>) -> Result<Option<usize>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let r = target::parse(raw).map_err(|e| e.to_string())?;
    let i = target::instance(font, &r).map_err(|e| e.to_string())?;
    match font.instances.get(i) {
        Some(inst) if inst.kind.as_deref() == Some("variable") => Ok(Some(i)),
        Some(inst) => Err(format!("`{}` is a static instance, not a variable font setting", target::instance_name(inst))),
        None => Err(format!("there is no instance number {i}")),
    }
}

/// The points of axis `tag` as the mapping commands read them, `(external, internal)` in
/// external order: the setting's own Axis Mappings, else the font's, else the map the font
/// implies (Axis Location, the instances or the masters).
pub fn points(font: &Font, setting: Option<usize>, tag: &str) -> Vec<(f64, f64)> {
    let Some(axis) = font.axes.iter().position(|a| a.tag == tag) else { return Vec::new() };
    let own = setting.and_then(|i| font.instances.get(i)).and_then(|inst| inst.params.get(AXIS_MAPPINGS));
    let lite;
    let base = match own {
        Some(value) => {
            let mut f = font.clone();
            Arc::make_mut(&mut f.info).params.set(AXIS_MAPPINGS, value.clone());
            lite = f;
            &lite
        }
        None => font,
    };
    let stored = fontcraft_engine::interp::params::axis_mappings(base).ok().and_then(|list| list.into_iter().find(|(t, _)| t == tag));
    match stored {
        Some((_, points)) => points,
        None => mapping(base, axis).into_pairs(),
    }
}

fn points_json(points: &[(f64, f64)]) -> Value {
    Value::Array(points.iter().map(|(e, i)| json!({"internal": i, "external": e})).collect())
}

/// What `axisMappings.dialogEdit` and `axisMappings.showDialog` return.
fn inspect(app: &FontcraftApp) -> Value {
    let st = &app.state.panels.axis_mappings;
    let Ok(font) = app.session.font() else { return json!({"open": false}) };
    let setting = setting_index(font, st.setting.as_ref()).ok().flatten();
    let axes: Vec<Value> =
        font.axes.iter().map(|a| json!({"tag": a.tag, "name": a.name, "points": points_json(&points(font, setting, &a.tag))})).collect();
    json!({
        "open": st.open,
        "variableFontSetting": st.setting,
        "axis": st.axis,
        "point": st.point,
        "axes": axes,
        "edits": st.steps.len(),
    })
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn show_dialog(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    if let Some(d) = &app.dialog {
        return Err(args.err(format!("the dialog `{}` is open: confirm or cancel it first", d.title)));
    }
    if app.state.panels.axis_mappings.open {
        return Ok(inspect(app));
    }
    let font = app.session.font().map_err(|e| args.err(e))?;
    let setting = args.get("variableFontSetting").filter(|v| !v.is_null()).cloned();
    setting_index(font, setting.as_ref()).map_err(|e| args.err(e))?;
    let axis = match args.string("axis")? {
        Some(tag) if font.axes.iter().any(|a| a.tag == tag) => tag.to_string(),
        Some(tag) => return Err(args.err(format!("the font has no axis `{tag}`"))),
        None => font.axes.first().map(|a| a.tag.clone()).ok_or_else(|| args.err("the font has no axes"))?,
    };
    app.session.begin(TITLE).map_err(|e| args.err(e))?;
    app.state.panels.axis_mappings = AxisMappingsState { open: true, setting, axis, point: None, steps: Vec::new() };
    Ok(inspect(app))
}

/// `steps` with `step` added: a move of the point the last step moved replaces or adds to it.
fn with_step(steps: &[Value], step: Value) -> Vec<Value> {
    let mut out = steps.to_vec();
    let key = |s: &Value| {
        let p = s.get("params");
        (s.get("command").cloned(), p.and_then(|p| p.get("axis")).cloned(), p.and_then(|p| p.get("index")).cloned())
    };
    let command = step.get("command").and_then(Value::as_str).unwrap_or_default();
    if matches!(command, "axisMappings.setPoint" | "axisMappings.nudgePoint")
        && let Some(last) = out.last_mut()
        && key(last) == key(&step)
        && let (Some(Value::Object(old)), Some(Value::Object(new))) = (last.get_mut("params"), step.get("params"))
    {
        for (k, v) in new {
            let summed = match (command, old.get(k).and_then(Value::as_f64), v.as_f64()) {
                ("axisMappings.nudgePoint", Some(a), Some(b)) if k.starts_with('d') => json!(a + b),
                _ => v.clone(),
            };
            old.insert(k.clone(), summed);
        }
        return out;
    }
    out.push(step);
    out
}

fn edit(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let mut st = app.state.panels.axis_mappings.clone();
    if !st.open {
        return Err(args.err("the Axis Mappings dialog is not open"));
    }
    let font = app.session.font().map_err(|e| args.err(e))?;
    let setting = setting_index(font, st.setting.as_ref()).map_err(|e| args.err(e))?;
    // Everything is checked before the dialog's state changes.
    let (axis, mut point) = match args.string("axis")? {
        Some(tag) if !font.axes.iter().any(|a| a.tag == tag) => return Err(args.err(format!("the font has no axis `{tag}`"))),
        Some(tag) if tag != st.axis => (tag.to_string(), None),
        _ => (st.axis.clone(), st.point),
    };
    let count = points(font, setting, &axis).len();
    match args.given("point") {
        None => {}
        Some(Value::Null) => point = None,
        Some(_) => {
            let i = args.index("point")?.and_then(|i| usize::try_from(i).ok()).ok_or_else(|| args.err("`point` is a whole number from 0"))?;
            if i >= count {
                return Err(args.err(format!("axis `{axis}` has {count} points; there is no point {i}")));
            }
            point = Some(i);
        }
    }
    let command = args.string("command")?;
    if let Some(command) = command
        && !EDITS.contains(&command)
    {
        return Err(args.err(format!("`command` is one of {}", EDITS.join(", "))));
    }
    let params = match args.get("params") {
        None => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(args.err("`params` must be a JSON object")),
    };
    st.axis = axis;
    st.point = point;
    if let Some(command) = command {
        let mut params = params;
        params.insert("axis".into(), json!(st.axis));
        match &st.setting {
            Some(s) => params.insert("variableFontSetting".into(), s.clone()),
            None => params.remove("variableFontSetting"),
        };
        if command != "axisMappings.addPoint" && !params.contains_key("index") {
            let i = st.point.ok_or_else(|| args.err("select a point first"))?;
            params.insert("index".into(), json!(i));
        }
        let removed = params.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
        let steps = with_step(&st.steps, json!({"command": command, "params": params}));
        if steps.len() > MAX_EDITS {
            return Err(args.err(format!("the dialog keeps at most {MAX_EDITS} edits: press OK and open it again")));
        }
        let result = app.session.preview("command.batch", &json!({"label": TITLE, "steps": steps})).map_err(|e| args.err(e))?;
        st.steps = steps;
        let last = result.get("results").and_then(Value::as_array).and_then(|r| r.last()).cloned().unwrap_or(Value::Null);
        let left = last.get("points").and_then(Value::as_array).map_or(0, Vec::len);
        st.point = match last.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()) {
            Some(i) => Some(i),
            // A removed point: its neighbour takes the selection.
            None => removed.map(|i| i.min(left.saturating_sub(1))).filter(|_| left > 0),
        };
    }
    app.state.panels.axis_mappings = st;
    Ok(inspect(app))
}

fn close(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    if !app.state.panels.axis_mappings.open {
        return Err(args.err("the Axis Mappings dialog is not open"));
    }
    let confirm = args.flag("confirm")?.unwrap_or(false);
    let ours = app.session.interaction() == Some(TITLE);
    app.state.panels.axis_mappings = AxisMappingsState::default();
    if !ours {
        return Ok(json!({"confirmed": false, "changed": false}));
    }
    if confirm {
        let changed = app.session.commit().map_err(|e| args.err(e))?;
        Ok(json!({"confirmed": true, "changed": changed}))
    } else {
        app.session.cancel().map_err(|e| args.err(e))?;
        Ok(json!({"confirmed": false, "changed": false}))
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

/// The data range the graph shows and the rectangle it fills.
struct Graph {
    rect: Rect,
    internal: (f64, f64),
    external: (f64, f64),
}

impl Graph {
    fn new(rect: Rect, points: &[(f64, f64)]) -> Self {
        let span = |values: &mut dyn Iterator<Item = f64>| {
            let (lo, hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
            let (lo, hi) = if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0.0, 1000.0) };
            let (lo, hi) = if hi - lo < 1.0 { (lo - 1.0, hi + 1.0) } else { (lo, hi) };
            let pad = (hi - lo) * GRAPH_MARGIN;
            (lo - pad, hi + pad)
        };
        Self { rect, internal: span(&mut points.iter().map(|p| p.1)), external: span(&mut points.iter().map(|p| p.0)) }
    }

    fn at(&self, external: f64, internal: f64) -> Pos2 {
        let fx = (internal - self.internal.0) / (self.internal.1 - self.internal.0);
        let fy = (external - self.external.0) / (self.external.1 - self.external.0);
        Pos2::new(self.rect.left() + self.rect.width() * fx as f32, self.rect.bottom() - self.rect.height() * fy as f32)
    }

    fn internal_at(&self, x: f32) -> f64 {
        self.internal.0 + f64::from((x - self.rect.left()) / self.rect.width().max(1.0)) * (self.internal.1 - self.internal.0)
    }

    fn external_at(&self, y: f32) -> f64 {
        self.external.0 + f64::from((self.rect.bottom() - y) / self.rect.height().max(1.0)) * (self.external.1 - self.external.0)
    }
}

/// The distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

/// Where the next point goes when + is pressed: halfway between the selected point and the next
/// one (the one before, for the last point), on the internal axis.
pub fn new_point_internal(points: &[(f64, f64)], selected: Option<usize>) -> f64 {
    let at = selected.unwrap_or(0);
    let pair = match (points.get(at), points.get(at + 1)) {
        (Some(a), Some(b)) => Some((a.1, b.1)),
        (Some(a), None) => at.checked_sub(1).and_then(|i| points.get(i)).map(|b| (b.1, a.1)),
        _ => None,
    };
    match pair {
        Some((a, b)) => ((a + b) / 2.0).round(),
        None => points.first().map_or(0.0, |p| p.1 + 1.0),
    }
}

fn edit_step(pending: &mut Pending, command: &str, params: Value) {
    pending.push(("axisMappings.dialogEdit", json!({"command": command, "params": params})));
}

fn axis_list(ui: &mut egui::Ui, font: &Font, selected: &str, pending: &mut Pending) {
    ui.vertical(|ui| {
        ui.label(RichText::new("Axes").strong());
        for a in font.axes.iter() {
            if ui.selectable_label(a.tag == selected, format!("{} ({})", a.name, a.tag)).clicked() && a.tag != selected {
                pending.push(("axisMappings.dialogEdit", json!({"axis": a.tag})));
            }
        }
    });
}

fn point_list(ui: &mut egui::Ui, tokens: &Tokens, points: &[(f64, f64)], st: &AxisMappingsState, pending: &mut Pending) {
    ui.vertical(|ui| {
        egui::Grid::new("axisMappings.points").num_columns(3).spacing([tokens.dialog_spacing, tokens.dialog_spacing / 2.0]).show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("Internal").strong());
            ui.label(RichText::new("External").strong());
            ui.end_row();
            for (i, (external, internal)) in points.iter().enumerate() {
                let selected = st.point == Some(i);
                if ui.selectable_label(selected, format!("{}", i + 1)).clicked() && !selected {
                    pending.push(("axisMappings.dialogEdit", json!({"point": i})));
                }
                for (key, value) in [("internal", *internal), ("external", *external)] {
                    let mut v = value;
                    let r = ui.add_sized([tokens.dialog_field_width, ui.spacing().interact_size.y], egui::DragValue::new(&mut v).speed(1.0));
                    if (r.clicked() || r.gained_focus()) && !selected {
                        pending.push(("axisMappings.dialogEdit", json!({"point": i})));
                    }
                    if r.changed() && v.is_finite() && v != value {
                        let mut params = Map::new();
                        params.insert("index".into(), json!(i));
                        params.insert(key.into(), json!(v));
                        edit_step(pending, "axisMappings.setPoint", Value::Object(params));
                    }
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            if ui.button("+").on_hover_text("Add a point on the line after the selected one").clicked() {
                edit_step(pending, "axisMappings.addPoint", json!({"internal": new_point_internal(points, st.point)}));
            }
            let can_remove = st.point.is_some_and(|i| i < points.len());
            if ui.add_enabled(can_remove, egui::Button::new("−")).on_hover_text("Delete the selected point").clicked() {
                edit_step(pending, "axisMappings.removePoint", json!({}));
            }
        });
    });
}

fn graph(ui: &mut egui::Ui, tokens: &Tokens, points: &[(f64, f64)], st: &AxisMappingsState, pending: &mut Pending) {
    let side = tokens.dialog_field_width * GRAPH_FIELDS;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
    let g = Graph::new(rect, points);
    let painter = ui.painter_at(rect.expand(tokens.node_size));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);

    // A drag moves one point's external value; the font changes when the drag ends.
    let drag_id = egui::Id::new("axisMappings.drag");
    let mut drag: Option<(usize, f64)> = ui.ctx().data(|d| d.get_temp(drag_id));
    let screen: Vec<Pos2> = points.iter().map(|(e, i)| g.at(*e, *i)).collect();
    let nearest = |p: Pos2| {
        screen
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.distance(p)))
            .filter(|(_, d)| *d <= tokens.hit_tolerance.max(tokens.node_size))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    if response.drag_started()
        && let Some(i) = response.interact_pointer_pos().and_then(nearest)
        && let Some((e, _)) = points.get(i)
    {
        drag = Some((i, *e));
        if st.point != Some(i) {
            pending.push(("axisMappings.dialogEdit", json!({"point": i})));
        }
    }
    if let (Some((i, _)), Some(p)) = (drag, response.interact_pointer_pos()) {
        drag = Some((i, g.external_at(p.y).round()));
    }
    if response.drag_stopped()
        && let Some((i, external)) = drag.take()
        && points.get(i).is_some_and(|p| p.0 != external)
    {
        edit_step(pending, "axisMappings.setPoint", json!({"index": i, "external": external}));
    }
    ui.ctx().data_mut(|d| {
        if let Some(v) = drag {
            d.insert_temp(drag_id, v);
        } else {
            d.remove::<(usize, f64)>(drag_id);
        }
    });
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        match nearest(p) {
            Some(i) if st.point != Some(i) => pending.push(("axisMappings.dialogEdit", json!({"point": i}))),
            Some(_) => {}
            None => {
                let on_line = screen.windows(2).any(|w| matches!(w, [a, b] if segment_distance(p, *a, *b) <= tokens.hit_tolerance));
                if on_line {
                    edit_step(pending, "axisMappings.addPoint", json!({"internal": g.internal_at(p.x).round()}));
                }
            }
        }
    }

    let shown: Vec<Pos2> = points
        .iter()
        .enumerate()
        .map(|(i, (e, int))| match drag {
            Some((d, external)) if d == i => g.at(external, *int),
            _ => g.at(*e, *int),
        })
        .collect();
    if shown.len() > 1 {
        painter.add(Shape::line(shown.clone(), Stroke::new(tokens.outline_width.max(1.5), tokens.accent)));
    }
    for (i, p) in shown.iter().enumerate() {
        let r = tokens.node_size / 2.0;
        if st.point == Some(i) {
            painter.circle_filled(*p, r * SELECTED_SCALE, tokens.accent);
        } else {
            painter.circle(*p, r, tokens.panel, Stroke::new(1.0, tokens.accent));
        }
    }
}

/// The arrow keys move the selected point; Return is OK and Escape is Cancel. Not while a value
/// is being typed.
fn keys(ui: &egui::Ui, st: &AxisMappingsState, pending: &mut Pending) {
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return;
    }
    let (mods, pressed) = ui.input_mut(|i| {
        let mods = i.modifiers;
        let keys = [egui::Key::ArrowUp, egui::Key::ArrowDown, egui::Key::ArrowLeft, egui::Key::ArrowRight, egui::Key::Enter, egui::Key::Escape];
        (mods, keys.map(|k| i.consume_key(mods, k)))
    });
    let step = if mods.command {
        100.0
    } else if mods.shift {
        10.0
    } else {
        1.0
    };
    let [up, down, left, right, enter, escape] = pressed;
    if st.point.is_some() {
        for (on, d_internal, d_external) in [(up, 0.0, step), (down, 0.0, -step), (right, step, 0.0), (left, -step, 0.0)] {
            if on {
                edit_step(pending, "axisMappings.nudgePoint", json!({"dInternal": d_internal, "dExternal": d_external}));
            }
        }
    }
    if enter {
        pending.push(("axisMappings.closeDialog", json!({"confirm": true})));
    } else if escape {
        pending.push(("axisMappings.closeDialog", json!({"confirm": false})));
    }
}

/// Draws the dialog while it is open. Its controls queue `axisMappings.*` shell commands.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.axis_mappings.open {
        return;
    }
    if app.session.interaction() != Some(TITLE) {
        // The document closed under the dialog: nothing is left to edit.
        app.state.panels.axis_mappings = AxisMappingsState::default();
        return;
    }
    let Ok(font) = app.session.font() else { return };
    let st = app.state.panels.axis_mappings.clone();
    let setting = setting_index(font, st.setting.as_ref()).ok().flatten();
    let points = points(font, setting, &st.axis);
    let tokens = &app.tokens;
    let area = egui::Modal::default_area(egui::Id::new("axisMappings.dialog")).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, tokens.dialog_top));
    egui::Modal::new(egui::Id::new("axisMappings.dialog"))
        .area(area)
        .backdrop_color(tokens.dialog_backdrop)
        .frame(egui::Frame::window(ui.style()))
        .show(ui.ctx(), |ui| {
            let title = match &st.setting {
                Some(_) => format!("{TITLE}: {}", setting.and_then(|i| font.instances.get(i)).map_or("", |i| target::instance_name(i))),
                None => TITLE.to_string(),
            };
            ui.label(RichText::new(title).strong().size(tokens.dialog_title_size));
            ui.add_space(tokens.dialog_spacing);
            ui.horizontal_top(|ui| {
                axis_list(ui, font, &st.axis, pending);
                ui.separator();
                point_list(ui, tokens, &points, &st, pending);
                ui.separator();
                graph(ui, tokens, &points, &st, pending);
            });
            ui.add_space(tokens.dialog_spacing);
            egui::Sides::new().show(
                ui,
                |_| {},
                |ui| {
                    if ui.button("OK").clicked() {
                        pending.push(("axisMappings.closeDialog", json!({"confirm": true})));
                    }
                    if ui.button("Cancel").clicked() {
                        pending.push(("axisMappings.closeDialog", json!({"confirm": false})));
                    }
                },
            );
            keys(ui, &st, pending);
        });
}

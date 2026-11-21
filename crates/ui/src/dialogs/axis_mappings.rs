//! The Axis Mappings dialog (row `dialog.axisMappings`): the points of each axis map, edited through the
//! `axisMappings.*` shell commands.
//!
//! Three panes: the axes; the points of the selected axis (internal coordinate and external coordinate) with fields
//! for the selected point and the plus and minus buttons; a graph of the same points, internal across and external up.
//! The dialog edits the font's Axis Mappings, or a variable font setting's when it opens on one.
//!
//! The dialog is modal and previews like any dialog: opening it begins an interaction, every edit is one
//! `axisMappings.*` command, and the edits made so far are previewed together as one `command.batch` on the font as it
//! was when the dialog opened. OK keeps them as one undo step; Cancel gives back the very font the dialog opened on.
//! An edit the command refuses (points out of order) changes nothing. Repeated moves of the same point merge into one
//! step, so holding an arrow key in a field keeps the batch short.
//!
//! | Control | Posts `axisMappings.dialogEdit` with |
//! |---|---|
//! | a row of the axis list | `{axis}` |
//! | a row of the point list | `{point}` |
//! | Internal, External fields | `{command: "axisMappings.setPoint", params: {index, internal | external}}` |
//! | + | `{command: "axisMappings.addPoint", params: {internal}}`: halfway to the next point, on the line |
//! | − | `{command: "axisMappings.removePoint"}` |
//!
//! OK and Cancel post `axisMappings.closeDialog {confirm}`. `ui.inspect` reports the dialog under `axisMappings`.

use std::borrow::Cow;
use std::sync::Arc;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Picture, PictureSize, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{ListRow, ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::table::Table;
use craft::ui::text::NumberField;
use egui::{Color32, Pos2, Rect, Shape, Stroke, Ui};
use fontcraft_engine::fontinfo::target;
use fontcraft_model::Font;
use fontcraft_model::axes::{AXIS_MAPPINGS, mapping};
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};

craft::ui::words! {
    /// The words of the Axis Mappings dialog.
    pub struct AxisMappingsWords {
        title: "fontcraftAxisMappings.title" = "Axis Mappings";
        axes: "fontcraftAxisMappings.axes" = "Axes";
        axis: "fontcraftAxisMappings.axis" = "Axis";
        points: "fontcraftAxisMappings.points" = "Points";
        point: "fontcraftAxisMappings.point" = "Point";
        internal: "fontcraftAxisMappings.internal" = "Internal";
        external: "fontcraftAxisMappings.external" = "External";
        add: "fontcraftAxisMappings.add" = "+";
        add_tip: "fontcraftAxisMappings.addTip" = "Add a point on the line after the selected one";
        remove: "fontcraftAxisMappings.remove" = "−";
        remove_tip: "fontcraftAxisMappings.removeTip" = "Delete the selected point";
        graph: "fontcraftAxisMappings.graph" = "Mapping graph: internal coordinates across, external coordinates up";
        no_axes: "fontcraftAxisMappings.noAxes" = "The font has no axes.";
    }
}

pub const SHOW: &str = "axisMappings.showDialog";
pub const EDIT: &str = "axisMappings.dialogEdit";
pub const CLOSE: &str = "axisMappings.closeDialog";
/// The interaction's label, also the label of the undo step OK makes.
pub const TITLE: &str = "Axis Mappings";
/// Most edits one opening of the dialog previews.
pub const MAX_EDITS: usize = 1024;
/// The commands an edit may run.
const EDITS: [&str; 4] = ["axisMappings.addPoint", "axisMappings.setPoint", "axisMappings.nudgePoint", "axisMappings.removePoint"];
/// Room around the points inside the graph, as a share of the coordinate range.
const GRAPH_MARGIN: f64 = 0.06;
/// A point's radius in the graph, as a share of the graph's width; the selected point is larger.
const POINT_SHARE: f32 = 0.018;
const SELECTED_SCALE: f32 = 1.7;
/// The frame's ink, as a share of the line's.
const FRAME_SHARE: f32 = 0.3;
/// Rows the point list shows before it scrolls.
const LIST_ROWS: u16 = 8;

const AXIS_COLUMN: ColumnKey = ColumnKey::new("axis");
const POINT_COLUMN: ColumnKey = ColumnKey::new("point");
const INTERNAL_COLUMN: ColumnKey = ColumnKey::new("internal");
const EXTERNAL_COLUMN: ColumnKey = ColumnKey::new("external");

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: SHOW,
        label: "Axis Mappings…",
        menu: &[],
        shortcut: None,
        params: "{variableFontSetting?: id|name|index, axis?: tag} open the Axis Mappings dialog on the font's mappings, or on a variable font setting's: the axes, the points of the selected axis (default the first) and a graph of them. Edits (axisMappings.dialogEdit) preview until axisMappings.closeDialog; refused while another dialog is open. ui.inspect reports the dialog under `axisMappings`: {open, variableFontSetting, axis, point, axes: [{tag, name, points: [{internal, external}]}], edits}",
    },
    UiCommand {
        id: EDIT,
        label: "Edit in Axis Mappings Dialog",
        menu: &[],
        shortcut: None,
        params: "{axis?: tag, point?: integer|null, command?: \"axisMappings.addPoint\"|\"axisMappings.setPoint\"|\"axisMappings.nudgePoint\"|\"axisMappings.removePoint\", params?: object} in the open Axis Mappings dialog: select an axis and a point (counted from 0 in external order), then run one edit with the command's parameters on the preview; `axis` and `variableFontSetting` are the dialog's, `index` defaults to the selected point. The edited point becomes the selection. A refused edit changes nothing",
    },
    UiCommand {
        id: CLOSE,
        label: "Close Axis Mappings Dialog",
        menu: &[],
        shortcut: None,
        params: "{confirm?: bool = false} close the Axis Mappings dialog: OK (confirm: true) keeps its edits as one undo step, Cancel gives back the font as it was when the dialog opened",
    },
];

/// The open dialog, in `ViewState::axis_mappings`.
#[derive(Clone, Debug, Default, PartialEq)]
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

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    if !(p.is_null() || p.is_object()) {
        return Err(format!("{}: params must be a JSON object", command.id));
    }
    match command.id.as_str() {
        SHOW => show_dialog(app, p),
        EDIT => edit(app, p),
        CLOSE => close(app, p),
        other => Err(format!("no axis mappings command `{other}`")),
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

/// The points of axis `tag` as the mapping commands read them, `(external, internal)` in external order: the
/// setting's own Axis Mappings, else the font's, else the map the font implies (Axis Location, the instances or the
/// masters).
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

/// The `axisMappings` key of `ui.inspect`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let st = &app.view.axis_mappings;
    let value = match app.session.font() {
        Ok(font) if st.open => {
            let setting = setting_index(font, st.setting.as_ref()).ok().flatten();
            let axes: Vec<Value> =
                font.axes.iter().map(|a| json!({"tag": a.tag, "name": a.name, "points": points_json(&points(font, setting, &a.tag))})).collect();
            json!({
                "open": true,
                "variableFontSetting": st.setting,
                "axis": st.axis,
                "point": st.point,
                "axes": axes,
                "edits": st.steps.len(),
            })
        }
        _ => json!({"open": false}),
    };
    state.insert("axisMappings".into(), value);
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn text<'p>(p: &'p Value, key: &str) -> Result<Option<&'p str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn show_dialog(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    if let Some(d) = &app.dialogs.open {
        return Err(format!("the dialog `{}` is open: confirm or cancel it first", d.title));
    }
    if app.view.axis_mappings.open {
        return Ok(());
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let setting = p.get("variableFontSetting").filter(|v| !v.is_null()).cloned();
    setting_index(font, setting.as_ref())?;
    let axis = match text(p, "axis")? {
        Some(tag) if font.axes.iter().any(|a| a.tag == tag) => tag.to_string(),
        Some(tag) => return Err(format!("the font has no axis `{tag}`")),
        None => font.axes.first().map(|a| a.tag.clone()).ok_or("the font has no axes")?,
    };
    app.session.begin(TITLE).map_err(|e| e.to_string())?;
    app.view.axis_mappings = AxisMappingsState { open: true, setting, axis, point: None, steps: Vec::new() };
    Ok(())
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

fn edit(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let mut st = app.view.axis_mappings.clone();
    if !st.open {
        return Err("the Axis Mappings dialog is not open".into());
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let setting = setting_index(font, st.setting.as_ref())?;
    // Everything is checked before the dialog's state changes.
    let (axis, mut point) = match text(p, "axis")? {
        Some(tag) if !font.axes.iter().any(|a| a.tag == tag) => return Err(format!("the font has no axis `{tag}`")),
        Some(tag) if tag != st.axis => (tag.to_string(), None),
        _ => (st.axis.clone(), st.point),
    };
    let count = points(font, setting, &axis).len();
    match p.get("point") {
        None => {}
        Some(Value::Null) => point = None,
        Some(v) => {
            let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`point` is a whole number from 0")?;
            if i >= count {
                return Err(format!("axis `{axis}` has {count} points; there is no point {i}"));
            }
            point = Some(i);
        }
    }
    let command = text(p, "command")?;
    if let Some(command) = command
        && !EDITS.contains(&command)
    {
        return Err(format!("`command` is one of {}", EDITS.join(", ")));
    }
    let params = match p.get("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("`params` must be a JSON object".into()),
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
            let i = st.point.ok_or("select a point first")?;
            params.insert("index".into(), json!(i));
        }
        let removed = params.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
        let steps = with_step(&st.steps, json!({"command": command, "params": params}));
        if steps.len() > MAX_EDITS {
            return Err(format!("the dialog keeps at most {MAX_EDITS} edits: press OK and open it again"));
        }
        let result = app.session.preview("command.batch", &json!({"label": TITLE, "steps": steps})).map_err(|e| e.to_string())?;
        st.steps = steps;
        let last = result.get("results").and_then(Value::as_array).and_then(|r| r.last()).cloned().unwrap_or(Value::Null);
        let left = last.get("points").and_then(Value::as_array).map_or(0, Vec::len);
        st.point = match last.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()) {
            Some(i) => Some(i),
            // A removed point: its neighbour takes the selection.
            None => removed.map(|i| i.min(left.saturating_sub(1))).filter(|_| left > 0),
        };
    }
    app.view.axis_mappings = st;
    Ok(())
}

fn close(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    if !app.view.axis_mappings.open {
        return Err("the Axis Mappings dialog is not open".into());
    }
    let confirm = match p.get("confirm") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or("`confirm` must be true or false")?,
    };
    let ours = app.session.interaction() == Some(TITLE);
    app.view.axis_mappings = AxisMappingsState::default();
    if !ours {
        return Ok(());
    }
    if confirm { app.session.commit().map(drop).map_err(|e| e.to_string()) } else { app.session.cancel().map_err(|e| e.to_string()) }
}

/// Where the next point goes when + is pressed: halfway between the selected point and the next one (the one before,
/// for the last point), on the internal axis.
pub fn new_point_internal(points: &[(f64, f64)], selected: Option<usize>) -> f64 {
    let at = selected.unwrap_or(0);
    let pair = match (points.get(at), points.get(at.saturating_add(1))) {
        (Some(a), Some(b)) => Some((a.1, b.1)),
        (Some(a), None) => at.checked_sub(1).and_then(|i| points.get(i)).map(|b| (b.1, a.1)),
        _ => None,
    };
    match pair {
        Some((a, b)) => ((a + b) / 2.0).round(),
        None => points.first().map_or(0.0, |p| p.1 + 1.0),
    }
}

// ── The window ──────────────────────────────────────────────────────────────────────────────────

/// What the dialog shows: a copy of the app's state with the points read from the font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisModel {
    /// The variable font setting's name; `None`: the font's mappings.
    pub setting: Option<String>,
    /// Every axis: tag and name.
    pub axes: Vec<(String, String)>,
    pub axis: String,
    /// The selected axis's points, `(external, internal)` in external order.
    pub points: Vec<(f64, f64)>,
    pub point: Option<usize>,
}

impl AxisModel {
    /// The model of the open dialog; `None` while it is closed or without a document.
    pub fn of(app: &FontCraftApp) -> Option<AxisModel> {
        let st = &app.view.axis_mappings;
        if !st.open {
            return None;
        }
        let font = app.session.font().ok()?;
        let setting = setting_index(font, st.setting.as_ref()).ok().flatten();
        Some(AxisModel {
            setting: setting.and_then(|i| font.instances.get(i)).map(|i| target::instance_name(i).to_string()),
            axes: font.axes.iter().map(|a| (a.tag.clone(), a.name.clone())).collect(),
            axis: st.axis.clone(),
            points: points(font, setting, &st.axis),
            point: st.point,
        })
    }
}

/// The dialog on the dialog host.
pub struct AxisMappingsDialog {
    id: DialogId,
    pub model: AxisModel,
    /// What the person did, until the dialog module takes it.
    pub outbox: Vec<Msg>,
    /// Set when the app closed the dialog: it closes in its next pass and answers nothing.
    pub closing: bool,
    open: bool,
    /// The form's value; the dialog's state lives in the app.
    held: bool,
}

impl AxisMappingsDialog {
    pub fn new(model: AxisModel) -> Self {
        AxisMappingsDialog { id: dialog_id("fontcraftAxisMappings"), model, outbox: Vec::new(), closing: false, open: true, held: false }
    }
}

impl StackDialog for AxisMappingsDialog {
    /// What the person did in the last pass, and the answer: `Some(true)` OK, `Some(false)` Cancel, `None` closed by
    /// the app.
    type Result = (Vec<Msg>, Option<bool>);

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        if self.closing {
            return Some(DialogResult::Ok((std::mem::take(&mut self.outbox), None)));
        }
        let words = Env::of(ui).words::<AxisMappingsWords>();
        let title: Label = match &self.model.setting {
            Some(name) => format!("{}: {name}", words.title.as_str()).into(),
            None => words.title.clone(),
        };
        let AxisMappingsDialog { model, outbox, open, held, .. } = self;
        let shown = FormDialog::new(title, Bind::bound(held)).opened(Bind::bound(open)).show(ui, |ui, _, _| {
            view(ui, model, outbox);
            true
        });
        if shown.accepted.is_some() {
            return Some(DialogResult::Ok((std::mem::take(outbox), Some(true))));
        }
        shown.closed.map(|_| DialogResult::Ok((std::mem::take(outbox), Some(false))))
    }
}

fn post(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(EDIT, params))
}

fn post_edit(command: &str, params: Value) -> Msg {
    post(json!({"command": command, "params": params}))
}

/// A coordinate as the lists show it: whole numbers without a fraction.
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{v:.0}") } else { format!("{v}") }
}

/// The dialog's body over `m`; what the person did goes to `sent`.
pub fn view(ui: &mut Ui, m: &AxisModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<AxisMappingsWords>();
    if m.axes.is_empty() {
        Text::new(words.no_axes.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    Row::new().show(ui, |ui, _| {
        axis_list(ui, &words, m, sent);
        point_list(ui, &words, m, sent);
        let (points, point) = (m.points.clone(), m.point);
        Picture::painter(words.graph.clone(), ThumbAspect::SQUARE, move |cx, rect| paint(&cx.painter, cx.ink(), rect, &points, point))
            .extent(PictureSize::Thumb(ThumbSize::Large))
            .show(ui);
    });
}

fn axis_list(ui: &mut Ui, words: &AxisMappingsWords, m: &AxisModel, sent: &mut Vec<Msg>) {
    let chosen = m.axes.iter().position(|(tag, _)| *tag == m.axis);
    let mut selection = chosen.map(Selection::single).unwrap_or_default();
    let columns = vec![ColumnSpec::new(AXIS_COLUMN, words.axis.clone(), ColumnWidth::Content)];
    let positions: Vec<usize> = (0..m.axes.len()).collect();
    let name = |at: &usize| m.axes.get(*at).map_or_else(String::new, |(tag, name)| format!("{name} ({tag})"));
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(name(at)));
    let _ = Table::<usize>::new(words.axes.clone(), columns)
        .max_rows(LIST_ROWS)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &source, &mut |at, icx| ListRow::new(name(at)).cells(icx.cells()));
    let picked = selection.items.last().copied();
    if picked != chosen
        && let Some((tag, _)) = picked.and_then(|at| m.axes.get(at))
    {
        sent.push(post(json!({"axis": tag})));
    }
}

fn point_list(ui: &mut Ui, words: &AxisMappingsWords, m: &AxisModel, sent: &mut Vec<Msg>) {
    Stack::new().show(ui, |ui, _| {
        let mut selection = m.point.map(Selection::single).unwrap_or_default();
        let columns = vec![
            ColumnSpec::new(POINT_COLUMN, words.point.clone(), ColumnWidth::Content),
            ColumnSpec::new(INTERNAL_COLUMN, words.internal.clone(), ColumnWidth::Content),
            ColumnSpec::new(EXTERNAL_COLUMN, words.external.clone(), ColumnWidth::Content),
        ];
        let positions: Vec<usize> = (0..m.points.len()).collect();
        let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(at.saturating_add(1).to_string()));
        let _ = Table::<usize>::new(words.points.clone(), columns)
            .max_rows(LIST_ROWS)
            .select(SelectMode::Single)
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &source, &mut |at, icx| {
                let Some((external, internal)) = m.points.get(*at) else { return ListRow::new("") };
                ListRow::new(at.saturating_add(1).to_string())
                    .cells(icx.cells())
                    .cell(INTERNAL_COLUMN, number(*internal))
                    .cell(EXTERNAL_COLUMN, number(*external))
            });
        let picked = selection.items.last().copied();
        if picked != m.point
            && let Some(at) = picked
        {
            sent.push(post(json!({"point": at})));
        }

        // The selected point's coordinates; arrow keys in a field step the value.
        let selected = m.point.and_then(|at| m.points.get(at).map(|p| (at, *p)));
        Row::new().show(ui, |ui, _| {
            for (key, label) in [("internal", &words.internal), ("external", &words.external)] {
                let value = selected.map(|(_, (external, internal))| if key == "internal" { internal } else { external });
                let out =
                    NumberField::new(label.clone(), Bind::given(value)).enabled(selected.is_some()).id_salt(("fontcraftAxisMappings", key)).show(ui);
                if let Some(Some(typed)) = out.edit.events.committed()
                    && typed.is_finite()
                    && let Some((at, _)) = selected
                    && Some(*typed) != value
                {
                    let mut params = Map::new();
                    params.insert("index".into(), json!(at));
                    params.insert(key.into(), json!(*typed));
                    sent.push(post_edit("axisMappings.setPoint", Value::Object(params)));
                }
            }
        });
        Row::new().show(ui, |ui, _| {
            if Button::new(words.add.clone()).tooltip(words.add_tip.clone()).show(ui).clicked() {
                sent.push(post_edit("axisMappings.addPoint", json!({"internal": new_point_internal(&m.points, m.point)})));
            }
            let can_remove = m.point.is_some_and(|i| i < m.points.len());
            if Button::new(words.remove.clone()).tooltip(words.remove_tip.clone()).enabled(can_remove).show(ui).clicked() && can_remove {
                sent.push(post_edit("axisMappings.removePoint", json!({})));
            }
        });
    });
}

/// The data range the graph shows, from the points with a margin.
fn span(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let (lo, hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let (lo, hi) = if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0.0, 1000.0) };
    let (lo, hi) = if hi - lo < 1.0 { (lo - 1.0, hi + 1.0) } else { (lo, hi) };
    let pad = (hi - lo) * GRAPH_MARGIN;
    (lo - pad, hi + pad)
}

/// Paints the frame, the mapping line and its points into `rect`; the selected point is larger.
fn paint(painter: &egui::Painter, ink: Color32, rect: Rect, points: &[(f64, f64)], selected: Option<usize>) {
    let hairline = 1.0 / painter.pixels_per_point().max(0.01);
    let internal = span(points.iter().map(|p| p.1));
    let external = span(points.iter().map(|p| p.0));
    let at = |(e, i): (f64, f64)| {
        let fx = (i - internal.0) / (internal.1 - internal.0);
        let fy = (e - external.0) / (external.1 - external.0);
        Pos2::new(rect.left() + rect.width() * fx as f32, rect.bottom() - rect.height() * fy as f32)
    };
    painter.rect_stroke(rect, 0.0, Stroke::new(hairline, ink.gamma_multiply(FRAME_SHARE)), egui::StrokeKind::Inside);
    let shown: Vec<Pos2> = points.iter().copied().filter(|(e, i)| e.is_finite() && i.is_finite()).map(at).collect();
    if shown.len() > 1 {
        painter.add(Shape::line(shown.clone(), Stroke::new(hairline * 2.0, ink)));
    }
    let r = rect.width() * POINT_SHARE;
    for (i, p) in shown.iter().enumerate() {
        if selected == Some(i) {
            painter.circle_filled(*p, r * SELECTED_SCALE, ink);
        } else {
            painter.circle_stroke(*p, r, Stroke::new(hairline * 2.0, ink));
        }
    }
}

/// Opens, refreshes or closes the dialog to match [`AxisMappingsState`] and posts what the person did.
pub fn run_dialog(host: &mut DialogHost, app: &mut FontCraftApp) {
    // The document closed or the interaction ended under the dialog: nothing is left to edit.
    if app.view.axis_mappings.open && app.session.interaction() != Some(TITLE) {
        app.view.axis_mappings = AxisMappingsState::default();
    }
    let model = AxisModel::of(app);
    if let Some(dialog) = host.get_mut::<AxisMappingsDialog>() {
        app.sent.append(&mut dialog.outbox);
        match model {
            Some(model) if dialog.model != model => dialog.model = model,
            Some(_) => {}
            None => dialog.closing = true,
        }
    } else if let Some(model) = model {
        host.open(AxisMappingsDialog::new(model));
    }
    match host.take::<AxisMappingsDialog>() {
        Some(DialogResult::Ok((mut msgs, answer))) => {
            app.sent.append(&mut msgs);
            if let Some(confirm) = answer
                && app.view.axis_mappings.open
            {
                app.sent.push(Msg::Shell(CommandRef::with_params(CLOSE, json!({"confirm": confirm}))));
            }
        }
        Some(DialogResult::Cancel) if app.view.axis_mappings.open => {
            app.sent.push(Msg::Shell(CommandRef::with_params(CLOSE, json!({"confirm": false}))));
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "axis_mappings_tests.rs"]
mod tests;

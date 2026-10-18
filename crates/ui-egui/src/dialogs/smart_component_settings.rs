//! The Smart Component settings (`dialog.smartComponentSettings`, `smartComponent.showSettings`):
//! one control per property of the selected smart components' smart glyphs. The same controls sit
//! at the foot of the Palette (`crate::panels::palette_smart`).
//!
//! | Property | Control |
//! |---|---|
//! | range other than 0 to 1 | a slider from minimum to maximum and a number field taking any value |
//! | range 0 to 1 | a checkbox (`smart.zeroOneRangeIsToggle`) |
//!
//! Every control writes through `smartComponent.setValue`; with several components, one
//! `command.batch` sets them all as one undo step. A component without a stored value shows the
//! value its glyph's default pole layer draws. The slider sends its value when the drag ends, so a
//! drag is one undo step. The window keeps the components it opened for until it is closed or
//! opened again; it blocks nothing.
//!
//! Rows: `smartComponent.showSettings`, `dialog.smartComponentSettings`.

use fontcraft_engine::cmd::smart::is_toggle;
use fontcraft_engine::model::smart::pole_layers;
use fontcraft_engine::model::{Component, Font, Glyph, LayerSide, MAX_PART_PROPERTIES, PartProperty, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::panels::{Pending, widgets};
use crate::shell::{Args, UiCommand};

pub const ID: &str = "smartComponent.showSettings";

/// Most components one window or the Palette edits at once.
pub const MAX_COMPONENTS: usize = 1024;

/// The undo label of a change made with the controls.
const LABEL: &str = "Set Smart Component Value";

pub const COMMANDS: &[UiCommand] = &[(
    ID,
    "Smart Component Settings",
    &[],
    "",
    "{glyph?: name, layer?: id (default: the first master's layer, or the edited layer when `glyph` is not given), side?: \"foreground\" | \"background\", components?: [indices] (shape indices on the layer; default the selected smart components of the edited layer, and required with `glyph`), open?: bool (false closes the window)} opens the settings of the smart components: one control per property of their smart glyphs. Every index given must be a component whose glyph has properties → {open, glyph, layer, side, components, properties: [{name, min, max, toggle, value (null when the components differ), components: [indices that have the property]}]}",
)];

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SmartComponentState {
    pub open: bool,
    pub glyph: String,
    /// The layer's id.
    pub layer: String,
    /// The components are on the layer's background.
    pub background: bool,
    /// Shape indices on the layer, ascending.
    pub components: Vec<usize>,
}

/// The components the controls act on.
#[derive(Clone, Debug, PartialEq)]
pub struct Targets {
    pub glyph: String,
    /// The layer's id.
    pub layer: String,
    pub side: LayerSide,
    /// Shape indices on the layer, ascending.
    pub components: Vec<usize>,
}

impl Targets {
    fn from_state(st: &SmartComponentState) -> Self {
        let side = if st.background { LayerSide::Background } else { LayerSide::Foreground };
        Self { glyph: st.glyph.clone(), layer: st.layer.clone(), side, components: st.components.clone() }
    }
}

/// One property as the controls show it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub toggle: bool,
    /// The components' common value; `None` when they differ.
    pub value: Option<f64>,
    /// The components whose glyph has the property.
    pub components: Vec<usize>,
}

/// The component at shape `index` of `shapes` and its glyph, when that glyph has properties.
fn smart_component<'f>(font: &'f Font, shapes: &'f [Shape], index: usize) -> Option<(&'f Component, &'f Glyph)> {
    let Some(Shape::Component(c)) = shapes.get(index) else { return None };
    let base = font.glyph(&c.glyph)?;
    (!base.info.parts.is_empty()).then_some((c, base.as_ref()))
}

/// The value of `part` that component `c` of the smart glyph `base` draws in `master`: its stored
/// value, else the end of the range its glyph's default pole layer (the master layer when it is
/// a pole layer, else the first) draws, as `fontcraft_model::smart` interpolates it.
pub fn current_value(base: &Glyph, c: &Component, master: &str, part: &PartProperty) -> f64 {
    if let Some(v) = c.info.smart_values.iter().find(|v| v.property == part.name) {
        return v.value;
    }
    let mut poles = pole_layers(base, master);
    let first = poles.next();
    let default = if first.is_some_and(|l| l.is_master_layer()) { first } else { poles.find(|l| l.is_master_layer()).or(first) };
    match default.and_then(|l| l.info.attrs.smart_poles.iter().find(|p| p.property == part.name)) {
        Some(p) if p.top => part.top_value,
        _ => part.bottom_value,
    }
}

/// The property rows of the smart components among `t.components`, by name in order of first
/// appearance; the range comes from the first glyph that has the name. Indices that are not smart
/// components are skipped.
pub fn rows(font: &Font, t: &Targets) -> Vec<Row> {
    let Some(g) = font.glyph(&t.glyph) else { return Vec::new() };
    let Some(layer) = g.layers.iter().find(|l| l.id == t.layer) else { return Vec::new() };
    let shapes = layer.parts(t.side).shapes;
    let mut out: Vec<Row> = Vec::new();
    for &i in t.components.iter().take(MAX_COMPONENTS) {
        let Some((c, base)) = smart_component(font, shapes, i) else { continue };
        for part in base.info.parts.iter().take(MAX_PART_PROPERTIES) {
            let value = current_value(base, c, &layer.master, part);
            if let Some(r) = out.iter_mut().find(|r| r.name == part.name) {
                if r.value.is_some_and(|v| v != value) {
                    r.value = None;
                }
                r.components.push(i);
            } else if out.len() < MAX_PART_PROPERTIES {
                out.push(Row {
                    name: part.name.clone(),
                    min: part.bottom_value,
                    max: part.top_value,
                    toggle: is_toggle(part),
                    value: Some(value),
                    components: vec![i],
                });
            }
        }
    }
    out
}

/// The command that sets `row`'s property to `value` on the components that have it: one
/// `smartComponent.setValue`, or a `command.batch` of them for several components.
pub fn value_command(t: &Targets, row: &Row, value: f64) -> (&'static str, Value) {
    let step = |component: usize| {
        let mut params = json!({"glyph": t.glyph, "layer": t.layer, "component": component, "property": row.name, "value": value});
        if t.side == LayerSide::Background
            && let Some(m) = params.as_object_mut()
        {
            m.insert("side".into(), json!("background"));
        }
        params
    };
    match row.components.as_slice() {
        [only] => ("smartComponent.setValue", step(*only)),
        many => {
            let steps: Vec<Value> = many.iter().map(|&i| json!({"command": "smartComponent.setValue", "params": step(i)})).collect();
            ("command.batch", json!({"label": LABEL, "steps": steps}))
        }
    }
}

/// The id of the number field of property `name` in the controls drawn with `salt`.
pub fn value_field_id(salt: &str, name: &str) -> egui::Id {
    widgets::id(("smartComponent.value", salt, name))
}

/// Draws one row of controls per property of the components in `t`; edits are queued in
/// `pending`. `salt` keeps the window's controls apart from the Palette's.
pub fn controls(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending, t: &Targets, salt: &str) {
    let Ok(font) = app.session.font() else { return };
    let rows = rows(font, t);
    let tokens = &app.tokens;
    if rows.is_empty() {
        ui.label(egui::RichText::new("No smart component to set.").color(tokens.text_dim));
        return;
    }
    for row in &rows {
        ui.horizontal(|ui| {
            ui.add_sized([tokens.field_width * 1.5, tokens.list_row_height], egui::Label::new(row.name.as_str()).truncate());
            if row.toggle {
                let mut on = row.value.is_some_and(|v| v >= 0.5);
                let check = egui::Checkbox::without_text(&mut on).indeterminate(row.value.is_none());
                if ui.add(check).on_hover_text("On or off").clicked() {
                    // A mixed box turns on, as it shows nothing ticked.
                    let on = row.value.is_none() || on;
                    pending.push(value_command(t, row, if on { 1.0 } else { 0.0 }));
                }
                return;
            }
            let (lo, hi) = (row.min.min(row.max), row.min.max(row.max));
            let drag_id = widgets::id(("smartComponent.slider", salt, &row.name));
            let shown = row.value.unwrap_or(lo);
            let mut v = ui.data(|d| d.get_temp::<f64>(drag_id)).unwrap_or(shown);
            let slider = egui::Slider::new(&mut v, lo..=hi).show_value(false).clamping(egui::SliderClamping::Edits);
            let r = ui.add(slider).on_hover_text(format!("{} to {}", widgets::format_number(lo), widgets::format_number(hi)));
            if r.dragged() {
                ui.data_mut(|d| d.insert_temp(drag_id, v));
            } else {
                ui.data_mut(|d| d.remove::<f64>(drag_id));
            }
            if (r.drag_stopped() || (r.changed() && !r.dragged())) && v.is_finite() && Some(v) != row.value {
                pending.push(value_command(t, row, v));
            }
            if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, value_field_id(salt, &row.name), row.value, tokens.field_width, false) {
                pending.push(value_command(t, row, n));
            }
        });
    }
}

/// The components a request names, checked: every index given must be a smart component; with
/// none given, the selected smart components of the edited layer.
fn resolve(app: &FontcraftApp, args: &Args) -> Result<Targets, String> {
    let font = app.session.font().map_err(|e| args.err(e))?;
    let side = match args.string("side")? {
        None => None,
        Some("foreground") => Some(LayerSide::Foreground),
        Some("background") => Some(LayerSide::Background),
        Some(other) => return Err(args.err(format!("`side` must be \"foreground\" or \"background\", got \"{other}\""))),
    };
    let (glyph, layer, side, selected) = match args.string("glyph")? {
        Some(name) => {
            let g = font.glyph(name).ok_or_else(|| args.err(format!("there is no glyph `{name}`")))?;
            let layer = match args.string("layer")? {
                Some(id) => g.layers.iter().find(|l| l.id == id).ok_or_else(|| args.err(format!("glyph `{name}` has no layer `{id}`")))?,
                None => {
                    let master = font.masters.first().ok_or_else(|| args.err("the font has no master"))?;
                    g.layer(&master.id).ok_or_else(|| args.err(format!("glyph `{name}` has no layer for the first master")))?
                }
            };
            (name.to_string(), layer, side.unwrap_or_default(), None)
        }
        None => {
            let tab = app.state.tab().ok_or_else(|| args.err("no glyph is being edited: give `glyph`"))?;
            let glyph = tab.edited_glyph().ok_or_else(|| args.err("no glyph is being edited: give `glyph`"))?;
            let g = font.glyph(glyph).ok_or_else(|| args.err(format!("there is no glyph `{glyph}`")))?;
            let layer = match args.string("layer")? {
                Some(id) => g.layers.iter().find(|l| l.id == id).ok_or_else(|| args.err(format!("glyph `{glyph}` has no layer `{id}`")))?,
                None => view::active_layer(font, tab).ok_or_else(|| args.err("the edited glyph has no layer to show"))?,
            };
            let selected: Vec<usize> = tab.selection.shapes.iter().copied().collect();
            (glyph.to_string(), layer, side.unwrap_or_else(|| tab.side()), Some(selected))
        }
    };
    let shapes = layer.parts(side).shapes;
    let mut components = match (args.get("components"), selected) {
        (Some(v), _) => {
            let list = v.as_array().ok_or_else(|| args.err("`components` must be a list of shape indices"))?;
            if list.len() > MAX_COMPONENTS {
                return Err(args.err(format!("`components` holds at most {MAX_COMPONENTS} indices")));
            }
            let mut out = Vec::with_capacity(list.len());
            for item in list {
                let i =
                    item.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| args.err("`components` must be a list of shape indices"))?;
                match shapes.get(i) {
                    None => return Err(args.err(format!("`components`: {i} does not resolve: the layer has {} shapes", shapes.len()))),
                    Some(Shape::Component(c)) if smart_component(font, shapes, i).is_none() => {
                        return Err(args.err(format!("component {i} refers to `{}`, which is not a smart glyph", c.glyph)));
                    }
                    Some(Shape::Component(_)) => out.push(i),
                    Some(_) => return Err(args.err(format!("shape {i} of glyph `{glyph}` is not a component"))),
                }
            }
            out
        }
        (None, Some(selected)) => {
            let out: Vec<usize> = selected.into_iter().filter(|&i| smart_component(font, shapes, i).is_some()).take(MAX_COMPONENTS).collect();
            if out.is_empty() {
                return Err(args.err("select a smart component, or give `components`"));
            }
            out
        }
        (None, None) => return Err(args.err("give `components` with `glyph`")),
    };
    components.sort_unstable();
    components.dedup();
    Ok(Targets { glyph, layer: layer.id.clone(), side, components })
}

/// The selected smart components of the edited layer, as the Palette shows them.
pub fn selected(app: &FontcraftApp) -> Option<Targets> {
    let args = Args::new(ID, &Value::Null).ok()?;
    resolve(app, &args).ok()
}

fn row_json(r: &Row) -> Value {
    json!({"name": r.name, "min": r.min, "max": r.max, "toggle": r.toggle, "value": r.value, "components": r.components})
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != ID {
        return Err(format!("unknown command `{id}`"));
    }
    if args.flag("open")? == Some(false) {
        app.state.panels.smart_component.open = false;
        return Ok(json!({"open": false}));
    }
    let t = resolve(app, args)?;
    let font = app.session.font().map_err(|e| args.err(e))?;
    let properties: Vec<Value> = rows(font, &t).iter().map(row_json).collect();
    let reply = json!({
        "open": true,
        "glyph": t.glyph,
        "layer": t.layer,
        "side": t.side.as_str(),
        "components": t.components,
        "properties": properties,
    });
    app.state.panels.smart_component =
        SmartComponentState { open: true, glyph: t.glyph, layer: t.layer, background: t.side == LayerSide::Background, components: t.components };
    Ok(reply)
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.smart_component.open {
        return;
    }
    let t = Targets::from_state(&app.state.panels.smart_component);
    let names: Vec<String> = app
        .session
        .font()
        .ok()
        .and_then(|f| f.glyph(&t.glyph))
        .and_then(|g| g.layers.iter().find(|l| l.id == t.layer))
        .map(|l| {
            let shapes = l.parts(t.side).shapes;
            let mut names: Vec<String> = Vec::new();
            for i in &t.components {
                if let Some(Shape::Component(c)) = shapes.get(*i)
                    && !names.contains(&c.glyph)
                {
                    names.push(c.glyph.clone());
                }
            }
            names
        })
        .unwrap_or_default();
    let mut open = true;
    let shown: &FontcraftApp = app;
    let tokens = &shown.tokens;
    egui::Window::new("Smart Component")
        .id(egui::Id::new(ID))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.label(egui::RichText::new(names.join(", ")).color(tokens.text_dim));
            ui.add_space(tokens.dialog_spacing);
            controls(shown, ui, pending, &t, "window");
        });
    if !open {
        app.state.panels.smart_component.open = false;
    }
}

//! Palette sections for filter modules with parameters (`palette.pluginSections`,
//! `plugin.kind.palette`). Every installed filter module that declares parameters gets a section
//! after the built-in ones, titled with the module's name: one field per parameter, drawn from
//! the manifest's schema (modules run no interface code), and an Apply button. Apply runs
//! `plugin.runFilter` with the form's values on what the module's Filter menu item acts on: the
//! Edit View layer with its selected nodes, the glyphs selected in text mode, or the Font View
//! selection in Font View's master. A section collapses like the built-in ones
//! (`palette.toggleSection` with the module id as `section`).
//!
//! The values typed into a form are widget state for the session, as a dialog's fields are
//! before OK; a module replaced by another version starts again from its defaults.

use fontcraft_engine::Session;
use fontcraft_engine::plugins::{Kind, ParamSpec};
use serde_json::{Map, Value, json};

use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::shell::menus_dynamic;
use crate::state::UiState;

/// One module section: the module's id, name and description, its load serial and its
/// parameters in form order.
pub struct ModuleSection {
    pub id: String,
    pub name: String,
    pub description: String,
    serial: u64,
    pub params: Vec<(String, ParamSpec)>,
}

/// The installed filter modules that declare parameters, in the order of their names.
pub fn sections(session: &Session) -> Vec<ModuleSection> {
    let mut out: Vec<ModuleSection> = session
        .extensions()
        .registry
        .list()
        .iter()
        .filter(|m| m.plugin.kind() == Kind::Filter && !m.plugin.manifest().params.is_empty())
        .map(|m| {
            let man = m.plugin.manifest();
            ModuleSection {
                id: man.id.clone(),
                name: man.name.clone(),
                description: man.description.clone(),
                serial: m.plugin.serial(),
                params: man.params.clone(),
            }
        })
        .collect();
    out.sort_by(|a, b| fontcraft_engine::macros::natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Whether `section` names a module section (`palette.toggleSection`).
pub fn is_section(session: &Session, section: &str) -> bool {
    session.extensions().registry.get(section).is_some_and(|m| m.plugin.kind() == Kind::Filter && !m.plugin.manifest().params.is_empty())
}

/// The command Apply queues for module `id` with the form's `values`: `plugin.runFilter` on what
/// the selection gives. The error says why Apply is greyed out.
pub fn apply(session: &Session, state: &UiState, id: &str, values: &Map<String, Value>) -> Result<(&'static str, Value), String> {
    session.extensions().runnable(id, Kind::Filter)?;
    let mut params = menus_dynamic::filter_targets(session, state).ok_or_else(|| "select glyphs or edit a glyph first".to_string())?;
    params.insert("id".into(), json!(id));
    params.insert("params".into(), Value::Object(values.clone()));
    Ok(("plugin.runFilter", Value::Object(params)))
}

/// The form's values: the ones typed this session, else the defaults.
fn values(ui: &egui::Ui, key: egui::Id, section: &ModuleSection) -> Map<String, Value> {
    let defaults = || section.params.iter().map(|(k, spec)| (k.clone(), spec.default_value())).collect::<Map<String, Value>>();
    ui.data(|d| d.get_temp::<Map<String, Value>>(key)).unwrap_or_else(defaults)
}

/// `n` held to `min..=max` (`f64::clamp` would panic on a reversed range).
fn within(n: f64, min: f64, max: f64) -> f64 {
    n.max(min).min(max)
}

/// Draws one parameter's field; returns its new value when it was changed.
fn field(app: &FontcraftApp, ui: &mut egui::Ui, key: egui::Id, name: &str, spec: &ParamSpec, value: Option<&Value>) -> Option<Value> {
    let width = app.tokens.dialog_field_width;
    let default = spec.default_value();
    let value = value.unwrap_or(&default);
    let mut out = None;
    ui.horizontal(|ui| {
        match spec {
            ParamSpec::Number { min, max, .. } => {
                ui.label(name);
                if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, key.with(name), value.as_f64(), width, false) {
                    out = Some(json!(within(n, *min, *max)));
                }
            }
            ParamSpec::Int { min, max, .. } => {
                ui.label(name);
                let shown = value.as_i64().map(|n| n as f64);
                if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, key.with(name), shown, width, false) {
                    // `min` and `max` are within ±2^53 (the manifest checks), so they convert exactly.
                    out = Some(json!(within(n.round(), *min as f64, *max as f64) as i64));
                }
            }
            ParamSpec::Bool { .. } => {
                let mut on = value.as_bool().unwrap_or(false);
                if ui.checkbox(&mut on, name).changed() {
                    out = Some(json!(on));
                }
            }
            ParamSpec::Choice { options, .. } => {
                ui.label(name);
                let current = value.as_str().unwrap_or_default();
                egui::ComboBox::from_id_salt(key.with(name)).selected_text(current).show_ui(ui, |ui| {
                    for option in options {
                        if ui.add(egui::Button::selectable(option == current, option.as_str())).clicked() {
                            out = Some(json!(option));
                        }
                    }
                });
            }
        }
    });
    out
}

/// Draws the module sections below the built-in ones.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    for section in sections(&app.session) {
        let expanded = !app.state.panels.collapsed.iter().any(|c| *c == section.id);
        if widgets::section_header(ui, &app.tokens, &section.id, &section.name, expanded) {
            pending.push(("palette.toggleSection", json!({"section": section.id, "expanded": !expanded})));
        }
        if expanded {
            ui.push_id(("module", &section.id), |ui| form(app, ui, &section, pending));
        }
        ui.separator();
    }
}

fn form(app: &FontcraftApp, ui: &mut egui::Ui, section: &ModuleSection, pending: &mut Pending) {
    let key = widgets::id(("moduleForm", &section.id, section.serial));
    let mut current = values(ui, key, section);
    let mut changed = false;
    for (name, spec) in &section.params {
        if let Some(v) = field(app, ui, key, name, spec, current.get(name)) {
            current.insert(name.clone(), v);
            changed = true;
        }
    }
    if changed {
        ui.data_mut(|d| d.insert_temp(key, current.clone()));
    }
    let run = apply(&app.session, &app.state, &section.id, &current);
    let hover = match &run {
        Ok(_) => section.description.clone(),
        Err(why) => why.clone(),
    };
    if widgets::button(ui, key.with("apply"), "Apply", &hover, run.is_ok(), false)
        && let Ok(command) = run
    {
        pending.push(command);
    }
}

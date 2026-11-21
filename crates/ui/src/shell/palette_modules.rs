//! The Modules panel's forms: every installed filter module that declares parameters has a section with one field
//! per parameter and an Apply button (`palette.pluginSections`, `panels::palette_modules`).
//!
//! | Command | Does |
//! |---|---|
//! | `palette.setModuleValue {id, name, value}` | keeps a form value, held to the parameter's range, as view state |
//! | `palette.applyModule {id}` | runs `plugin.runFilter` with the form's values on what the module's Filter menu row acts on |
//!
//! The values are kept for the session. A module replaced by another version (a new load serial) starts again from
//! its defaults.

use std::collections::BTreeMap;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::macros::natural_cmp;
use fontcraft_engine::plugins::{Installed, Kind, ParamSpec};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::params::string;
use crate::app::FontCraftApp;
use crate::app::menus_dynamic::RUN_FILTER;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most module forms whose values are kept; forms of modules removed since are dropped first when the list is full.
pub const MAX_FORMS: usize = 256;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "palette.setModuleValue",
        label: "Set Module Parameter",
        menu: &[],
        shortcut: None,
        params: "{id: filter module id, name: parameter name, value: number | bool | string} the value of one field of the module's section in the Modules panel, held to the parameter's range; kept for the session",
    },
    UiCommand {
        id: "palette.applyModule",
        label: "Apply Module",
        menu: &[],
        shortcut: None,
        params: "{id: filter module id} the Apply button of the module's section: runs the filter module with the section's values on what its Filter menu item acts on (the edited glyph and its selected nodes, or the glyphs selected in Font View)",
    },
];

/// The values typed into one module's form, for the module's load `serial`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModuleForm {
    pub serial: u64,
    pub values: Map<String, Value>,
}

/// The forms of the module sections, by module id.
pub type ModuleForms = BTreeMap<String, ModuleForm>;

/// The filter modules that declare parameters, in the order of their names.
pub fn modules(session: &Session) -> Vec<&Installed> {
    let mut list: Vec<&Installed> =
        session.extensions().registry.list().iter().filter(|m| m.plugin.kind() == Kind::Filter && !m.plugin.manifest().params.is_empty()).collect();
    list.sort_by(|a, b| {
        let (a, b) = (a.plugin.manifest(), b.plugin.manifest());
        natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id))
    });
    list.truncate(crate::app::menus_dynamic::MAX_ROWS);
    list
}

/// The form values of module `m`: the ones kept for its serial, else its defaults.
pub fn values(forms: &ModuleForms, m: &Installed) -> Map<String, Value> {
    let man = m.plugin.manifest();
    let kept = forms.get(&man.id).filter(|f| f.serial == m.plugin.serial());
    man.params
        .iter()
        .map(|(name, spec)| (name.clone(), kept.and_then(|f| f.values.get(name)).cloned().unwrap_or_else(|| spec.default_value())))
        .collect()
}

/// `n` held to `min..=max` (`f64::clamp` panics on a reversed range).
fn within(n: f64, min: f64, max: f64) -> f64 {
    n.max(min).min(max)
}

/// `value` as parameter `spec` takes it: a number held to the range, an integer rounded, a known option.
fn checked(spec: &ParamSpec, value: &Value) -> Result<Value, String> {
    match spec {
        ParamSpec::Number { min, max, .. } => {
            let n = value.as_f64().filter(|n| n.is_finite()).ok_or("`value` is a number")?;
            Ok(json!(within(n, *min, *max)))
        }
        ParamSpec::Int { min, max, .. } => {
            let n = value.as_f64().filter(|n| n.is_finite()).ok_or("`value` is a number")?;
            // `min` and `max` are within ±2^53 (the manifest checks), so they convert exactly.
            Ok(json!(within(n.round(), *min as f64, *max as f64) as i64))
        }
        ParamSpec::Bool { .. } => value.as_bool().map(Value::Bool).ok_or_else(|| "`value` is true or false".to_string()),
        ParamSpec::Choice { options, .. } => {
            let s = value.as_str().ok_or("`value` is one of the options")?;
            options.iter().find(|o| *o == s).map(|o| json!(o)).ok_or_else(|| format!("`value` is one of {}", options.join(", ")))
        }
    }
}

/// The installed filter module `id` with parameters.
fn module<'s>(session: &'s Session, id: &str) -> Result<&'s Installed, String> {
    session
        .extensions()
        .registry
        .get(id)
        .filter(|m| m.plugin.kind() == Kind::Filter && !m.plugin.manifest().params.is_empty())
        .ok_or_else(|| format!("no filter module {id:?} with parameters is installed"))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let id = string(&command.params, "id")?.ok_or("give `id`: a filter module's id")?.to_string();
    match command.id.as_str() {
        "palette.setModuleValue" => set_value(app, &id, &command.params),
        "palette.applyModule" => apply(app, &id),
        other => Err(format!("no module section command `{other}`")),
    }
}

fn set_value(app: &mut FontCraftApp, id: &str, p: &Value) -> Result<(), String> {
    let name = string(p, "name")?.ok_or("give `name`: a parameter of the module")?;
    let value = p.get("value").ok_or("give `value`")?;
    let m = module(&app.session, id)?;
    let spec = m
        .plugin
        .manifest()
        .params
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, s)| s)
        .ok_or_else(|| format!("module {id:?} has no parameter {name:?}"))?;
    let value = checked(spec, value)?;
    let mut values = values(&app.view.palette.modules, m);
    values.insert(name.to_string(), value);
    let form = ModuleForm { serial: m.plugin.serial(), values };
    let forms = &mut app.view.palette.modules;
    if !forms.contains_key(id) && forms.len() >= MAX_FORMS {
        let session = &app.session;
        forms.retain(|k, _| module(session, k).is_ok());
    }
    if !forms.contains_key(id) && forms.len() >= MAX_FORMS {
        return Err(format!("at most {MAX_FORMS} module forms are kept"));
    }
    forms.insert(id.to_string(), form);
    Ok(())
}

/// Runs `plugin.runFilter` with the form's values, its targets taken from the selection as the Filter menu row's are.
fn apply(app: &mut FontCraftApp, id: &str) -> Result<(), String> {
    let m = module(&app.session, id)?;
    app.session.extensions().runnable(id, Kind::Filter)?;
    let params = values(&app.view.palette.modules, m);
    let fired = CommandRef::with_params(RUN_FILTER, json!({"id": id, "params": params}));
    match app.fired(fired)? {
        Some(msg) => app.apply_msg(msg),
        None => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use fontcraft_engine::plugins::fixtures;
    use serde_json::{Value, json};

    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

    const SQUARE: &str = "org.test.square";
    const SHIFT: &str = "org.test.shift";

    fn square() -> Value {
        json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}])
    }

    /// Installs a filter module that turns every layer into a 100 × 100 square.
    fn install_filter(app: &mut FontCraftApp, id: &str, name: &str, params: &str) {
        let mut scratch = Session::with_document();
        scratch.execute("path.addPath", &json!({"glyph": "A", "nodes": square()})).unwrap();
        let font = scratch.font().unwrap();
        let layer = font.glyph("A").unwrap().layers.iter().find(|l| l.is_master_layer()).unwrap();
        let shapes = serde_json::to_value(&layer.shapes).unwrap();
        let manifest = format!(r#"{{"id":"{id}","name":"{name}","version":"1","kind":"filter","description":"Makes a square","params":{params}}}"#);
        let module = fixtures::returning(&manifest, &json!({"shapes": shapes}).to_string());
        app.session.extensions_mut().registry.install_bytes(&module, None).unwrap();
    }

    fn post(app: &mut FontCraftApp, msg: Msg) -> Result<(), String> {
        app.status.clear();
        app.sent.push(msg);
        app.drain();
        if app.status.is_empty() { Ok(()) } else { Err(app.status.clone()) }
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<(), String> {
        post(app, Msg::Shell(CommandRef::with_params(id, params)))
    }

    /// Only filter modules with parameters get a section; a typed value is held to its range and Apply runs the
    /// filter with the form on the edited glyph, as one undo step.
    #[test]
    fn a_module_section_keeps_its_form_and_applies_the_filter() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        post(&mut app, Msg::View(ViewMsg::SelectGlyph("A".into()))).unwrap();
        install_filter(&mut app, SQUARE, "Square", "{}");
        install_filter(&mut app, SHIFT, "Shift", r#"{"dx": {"type": "number", "min": -100, "max": 100, "default": 10}}"#);
        app.drain();

        let sections = app.models.modules.model.sections.clone();
        assert_eq!(sections.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec![SHIFT], "only modules with parameters");
        assert_eq!(sections[0].fields[0].number, Some(10.0), "the default");

        shell(&mut app, "palette.setModuleValue", json!({"id": SHIFT, "name": "dx", "value": 250})).unwrap();
        assert_eq!(app.models.modules.model.sections[0].fields[0].number, Some(100.0), "held to the range");
        assert!(shell(&mut app, "palette.setModuleValue", json!({"id": SQUARE, "name": "dx", "value": 1})).is_err());

        let before = app.session.active().unwrap().revision;
        shell(&mut app, "palette.applyModule", json!({"id": SHIFT})).unwrap();
        assert_ne!(app.session.active().unwrap().revision, before, "the filter ran");
        let font = app.session.font().unwrap();
        let layer = font.glyph("A").unwrap().layers.iter().find(|l| l.is_master_layer()).unwrap();
        assert_eq!(layer.shapes.len(), 1, "the edited glyph became the square");

        // A module turned off cannot be applied, and its section says why.
        app.session.extensions_mut().set_module_enabled(SHIFT, false).unwrap();
        app.drain();
        assert!(app.models.modules.model.sections[0].blocked.is_some());
        assert!(shell(&mut app, "palette.applyModule", json!({"id": SHIFT})).is_err());
    }
}

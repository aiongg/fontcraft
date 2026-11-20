//! The Modules panel's view model: one section per installed filter module that declares parameters, with its
//! form's values (`shell::palette_modules`).
//!
//! The extensions move without the engine's document revision, so [`project`] runs on every projection and skips
//! the work while the extensions' revision, safe mode and the forms stay.

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::plugins::{Kind, ParamSpec};
use serde_json::Value;

use super::ProjectCx;
use crate::shell::palette_modules::{self, ModuleForms};

/// What kind of field a parameter has.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParamKind {
    #[default]
    Number,
    Int,
    Bool,
    Choice,
}

/// One parameter's field.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ModuleField {
    pub name: String,
    pub kind: ParamKind,
    /// A number or integer field's value.
    pub number: Option<f64>,
    /// A checkbox's value.
    pub on: bool,
    /// A pop-up's option and the options.
    pub choice: String,
    pub options: Vec<String>,
}

/// One module's section.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ModuleSection {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Why Apply cannot run the module now (turned off, safe mode).
    pub blocked: Option<String>,
    pub fields: Vec<ModuleField>,
}

/// The Modules panel.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ModulesModel {
    pub sections: Vec<ModuleSection>,
}

/// The model and what it was last projected from.
#[derive(Default)]
pub struct ModulesModels {
    seen: Option<(u64, bool, ModuleForms)>,
    pub model: Tracked<ModulesModel>,
}

pub fn project(models: &mut ModulesModels, cx: &mut ProjectCx<'_>) {
    let session: &fontcraft_engine::Session = cx.session;
    let ext = session.extensions();
    let key = (ext.revision(), ext.safe_mode, cx.view.palette.modules.clone());
    if models.seen.as_ref() == Some(&key) {
        return;
    }
    let sections = palette_modules::modules(session)
        .into_iter()
        .map(|m| {
            let man = m.plugin.manifest();
            let values = palette_modules::values(&cx.view.palette.modules, m);
            let fields = man
                .params
                .iter()
                .map(|(name, spec)| {
                    let value = values.get(name);
                    let mut field = ModuleField { name: name.clone(), ..Default::default() };
                    match spec {
                        ParamSpec::Number { .. } => field.number = value.and_then(Value::as_f64),
                        ParamSpec::Int { .. } => {
                            field.kind = ParamKind::Int;
                            field.number = value.and_then(Value::as_f64);
                        }
                        ParamSpec::Bool { .. } => {
                            field.kind = ParamKind::Bool;
                            field.on = value.and_then(Value::as_bool).unwrap_or(false);
                        }
                        ParamSpec::Choice { options, .. } => {
                            field.kind = ParamKind::Choice;
                            field.choice = value.and_then(Value::as_str).unwrap_or_default().to_string();
                            field.options = options.clone();
                        }
                    }
                    field
                })
                .collect();
            ModuleSection {
                id: man.id.clone(),
                name: man.name.clone(),
                description: man.description.clone(),
                blocked: ext.runnable(&man.id, Kind::Filter).err(),
                fields,
            }
        })
        .collect();
    models.seen = Some(key);
    super::set(&mut models.model, ModulesModel { sections });
}

//! The Audit Profile dialog's draft and its two commands (`dialog.auditProfile`).
//!
//! `audit.editProfile` reads a profile into a [`ProfileDraft`]; the dialog (`dialogs::audit_profile`) edits the
//! draft and changes nothing until Save, which is `audit.closeProfileDialog {action: "save"}`: `audit.profile.save`
//! with the draft's parameters, together with `audit.config.set` in one `command.batch` when a project value changed.
//! A built-in profile opens read-only, as a draft that includes it; Duplicate makes that an editable user profile. A
//! Save or a Delete the engine refuses keeps the dialog open and changes nothing.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::run::MAX_CHECKS;
use super::{flag, quoted, refresh_profiles, string};
use crate::app::{FontCraftApp, Msg};

pub const OPEN: &str = "audit.editProfile";
pub const CLOSE: &str = "audit.closeProfileDialog";
/// The undo label of a Save that also sets project values.
const TITLE: &str = "Audit Profile";
/// Most entries the dialog reads into one list of a profile.
pub const MAX_ENTRIES: usize = 1_024;
/// Most thresholds of one check.
pub const MAX_THRESHOLDS: usize = 64;
/// Longest id, label or text value the dialog takes, in characters.
pub const MAX_TEXT: usize = 256;
/// Most copies Duplicate numbers before it gives up looking for a free id.
const MAX_COPIES: usize = 1_000;

/// One check of the registry, as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckRow {
    pub id: String,
    pub label: String,
    pub group: String,
    pub severity: String,
}

/// One threshold of the selected check.
#[derive(Clone, Debug, PartialEq)]
pub struct Threshold {
    pub name: String,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub doc: String,
}

/// One project value of the open document, as `audit.config.get` gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectValue {
    pub name: String,
    /// "true or false", "whole number", "list of table tags", or text.
    pub kind: String,
    pub doc: String,
    pub value: Value,
}

/// A severity override of the profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Override {
    pub check: String,
    pub code: Option<String>,
    pub severity: String,
}

/// What the dialog edits; nothing is stored until Save.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileDraft {
    /// The saved profile the dialog opened on and its store; `None` for a new profile.
    pub original: Option<(String, String)>,
    pub read_only: bool,
    pub id: String,
    pub label: String,
    /// `user` or `project`.
    pub store: String,
    pub include: Vec<String>,
    pub checks: Vec<String>,
    pub exclude: Vec<String>,
    pub overrides: Vec<Override>,
    /// Threshold defaults: check id → threshold name → value.
    pub config: BTreeMap<String, BTreeMap<String, f64>>,
    pub values: Vec<ProjectValue>,
    /// Project values changed in the dialog (null: remove the override).
    pub changed: BTreeMap<String, Value>,
    /// The check whose thresholds show, and those thresholds.
    pub selected: Option<String>,
    pub thresholds: Vec<Threshold>,
    pub search: String,
    pub catalogue: Vec<CheckRow>,
    /// The checks the included profiles run.
    pub base: BTreeSet<String>,
    /// The profiles the include list offers.
    pub profiles: Vec<String>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().take(MAX_ENTRIES).filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Whether check `id` matches `pattern` (an id, or a prefix ending in `*`).
fn matches(id: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => id.starts_with(prefix),
        None => id == pattern,
    }
}

impl ProfileDraft {
    /// Whether the draft runs check `id`: the included profiles' checks plus `checks`, minus `exclude`.
    pub fn runs(&self, id: &str) -> bool {
        (self.base.contains(id) || self.checks.iter().any(|p| matches(id, p))) && !self.exclude.iter().any(|p| matches(id, p))
    }

    /// Turns check `id` on or off, by taking it out of `checks` or `exclude` where that is enough, else by adding it
    /// to the other list.
    pub fn toggle(&mut self, id: &str) {
        if self.runs(id) {
            self.checks.retain(|c| c != id);
            if self.runs(id) {
                self.exclude.push(id.to_string());
            }
        } else {
            self.exclude.retain(|c| c != id);
            if !self.runs(id) {
                self.checks.push(id.to_string());
            }
        }
    }

    /// The severity override of check `id` that covers every message, or "".
    pub fn override_of(&self, id: &str) -> &str {
        self.overrides.iter().find(|o| o.check == id && o.code.is_none()).map_or("", |o| o.severity.as_str())
    }

    pub fn set_override(&mut self, id: &str, severity: &str) {
        self.overrides.retain(|o| !(o.check == id && o.code.is_none()));
        if !severity.is_empty() {
            self.overrides.push(Override { check: id.to_string(), code: None, severity: severity.to_string() });
        }
    }

    /// Sets threshold `name` of the selected check, kept within its range; `None` removes it.
    pub fn set_threshold(&mut self, name: &str, value: Option<f64>) {
        let Some(check) = self.selected.clone() else { return };
        let Some(t) = self.thresholds.iter().find(|t| t.name == name) else { return };
        match value.filter(|v| v.is_finite()) {
            Some(v) => {
                let v = t.min.map_or(v, |m| v.max(m));
                let v = t.max.map_or(v, |m| v.min(m));
                self.config.entry(check).or_default().insert(name.to_string(), v);
            }
            None => {
                if let Some(m) = self.config.get_mut(&check) {
                    m.remove(name);
                    if m.is_empty() {
                        self.config.remove(&check);
                    }
                }
            }
        }
    }

    /// The value of project value `name` in the dialog.
    pub fn value_of(&self, name: &str) -> Value {
        self.changed.get(name).cloned().or_else(|| self.values.iter().find(|v| v.name == name).map(|v| v.value.clone())).unwrap_or(Value::Null)
    }

    /// Sets project value `name`; the value it had at open takes it out of the changes again.
    pub fn set_value(&mut self, name: &str, value: Value) {
        let Some(v) = self.values.iter().find(|v| v.name == name) else { return };
        if value == v.value {
            self.changed.remove(name);
        } else {
            self.changed.insert(name.to_string(), value);
        }
    }

    /// The parameters of `audit.profile.save` for the draft.
    pub fn save_params(&self) -> Value {
        let overrides: Vec<Value> = self
            .overrides
            .iter()
            .map(|o| {
                let mut m = Map::new();
                m.insert("check".into(), json!(o.check));
                if let Some(code) = &o.code {
                    m.insert("code".into(), json!(code));
                }
                m.insert("severity".into(), json!(o.severity));
                Value::Object(m)
            })
            .collect();
        json!({
            "profile": self.id,
            "label": self.label,
            "include": self.include,
            "checks": self.checks,
            "exclude": self.exclude,
            "overrides": overrides,
            "config": self.config,
            "store": self.store,
        })
    }

    /// How many checks of the registry the draft runs.
    pub fn count(&self) -> usize {
        self.catalogue.iter().filter(|c| self.runs(&c.id)).count()
    }

    /// The draft as an editable copy under a new id, in the user's profiles.
    pub fn duplicate(&self) -> ProfileDraft {
        let mut d = self.clone();
        d.id = copy_id(&d.id, &d.profiles);
        if !d.label.is_empty() {
            d.label = format!("{} Copy", d.label);
        }
        d.original = None;
        d.read_only = false;
        d.store = "user".into();
        d
    }

    /// The dialog as `ui.inspect` reports it.
    pub fn to_json(&self) -> Value {
        let values: Map<String, Value> = self.values.iter().map(|v| (v.name.clone(), self.value_of(&v.name))).collect();
        let mut out = self.save_params();
        if let Value::Object(m) = &mut out {
            m.insert("open".into(), json!(true));
            m.insert("readOnly".into(), json!(self.read_only));
            m.insert("saved".into(), json!(self.original.is_some()));
            m.insert("runs".into(), json!(self.count()));
            m.insert("selected".into(), json!(self.selected));
            m.insert("values".into(), Value::Object(values));
        }
        out
    }
}

/// The first free id of `<id>-copy`, `<id>-copy-2`… among `taken`.
fn copy_id(id: &str, taken: &[String]) -> String {
    let base = format!("{id}-copy");
    if !taken.contains(&base) {
        return base;
    }
    (2..MAX_COPIES).map(|n| format!("{base}-{n}")).find(|c| !taken.contains(c)).unwrap_or(base)
}

// ── Reading the engine ──────────────────────────────────────────────────────────────────────────

fn catalogue(app: &mut FontCraftApp) -> Vec<CheckRow> {
    let Ok(v) = app.session.query("audit.list", &json!({})) else { return Vec::new() };
    let text = |c: &Value, key: &str| c.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    v.get("checks")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_CHECKS)
                .filter_map(|c| {
                    Some(CheckRow {
                        id: c.get("id")?.as_str()?.to_string(),
                        label: text(c, "label"),
                        group: text(c, "group"),
                        severity: text(c, "severity"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The checks the profiles `include` run, together.
pub fn base_of(app: &mut FontCraftApp, include: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in include.iter().take(MAX_ENTRIES) {
        if let Ok(v) = app.session.query("audit.profile.show", &json!({"profile": p})) {
            out.extend(strings(v.pointer("/resolved/checks")));
        }
    }
    out
}

/// The thresholds of `check`, as `audit.describe` gives them.
pub fn thresholds_of(app: &mut FontCraftApp, check: &str) -> Vec<Threshold> {
    let Ok(v) = app.session.query("audit.describe", &json!({"check": check})) else { return Vec::new() };
    let number = |t: &Value, key: &str| t.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
    v.get("thresholds")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_THRESHOLDS)
                .filter_map(|t| {
                    Some(Threshold {
                        name: t.get("name")?.as_str()?.to_string(),
                        default: number(t, "default").unwrap_or(0.0),
                        min: number(t, "min"),
                        max: number(t, "max"),
                        doc: t.get("doc").and_then(Value::as_str).unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn project_values(app: &mut FontCraftApp) -> Vec<ProjectValue> {
    if app.session.active().is_none() {
        return Vec::new();
    }
    let Ok(v) = app.session.query("audit.config.get", &json!({})) else { return Vec::new() };
    let Some(values) = v.get("values").and_then(Value::as_object) else { return Vec::new() };
    values
        .iter()
        .take(MAX_ENTRIES)
        .map(|(name, d)| ProjectValue {
            name: name.clone(),
            kind: d.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
            doc: d.get("doc").and_then(Value::as_str).unwrap_or_default().to_string(),
            value: d.get("value").cloned().unwrap_or(Value::Null),
        })
        .collect()
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

pub fn open(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let profile = match string(p, "profile")? {
        Some(id) if !id.is_empty() && id.len() <= super::MAX_PROFILE_ID => id,
        Some(_) => return Err(format!("`profile` must be a profile id of 1 to {} bytes", super::MAX_PROFILE_ID)),
        None => match app.view.font_audit.profile.clone() {
            Some(id) => id,
            None => app
                .session
                .query("audit.config.get", &json!({}))
                .ok()
                .and_then(|v| v.get("profile").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| "universal".into()),
        },
    };
    let copy = flag(p, "duplicate")?.unwrap_or(false);
    let shown = app.session.query("audit.profile.show", &json!({"profile": profile})).map_err(|e| e.to_string())?;
    let id = shown.get("id").and_then(Value::as_str).unwrap_or(&profile).to_string();
    let store = shown.get("store").and_then(Value::as_str).unwrap_or("builtIn").to_string();
    let built_in = shown.get("builtIn").and_then(Value::as_bool).unwrap_or(store == "builtIn");
    let overrides = shown
        .get("overrides")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(MAX_ENTRIES)
                .filter_map(|o| {
                    Some(Override {
                        check: o.get("check")?.as_str()?.to_string(),
                        code: o.get("code").and_then(Value::as_str).map(str::to_string),
                        severity: o.get("severity")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut config: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    if let Some(c) = shown.get("config").and_then(Value::as_object) {
        for (check, values) in c.iter().take(MAX_ENTRIES) {
            let Some(values) = values.as_object() else { continue };
            let entry: BTreeMap<String, f64> =
                values.iter().take(MAX_THRESHOLDS).filter_map(|(k, v)| Some((k.clone(), v.as_f64().filter(|n| n.is_finite())?))).collect();
            if !entry.is_empty() {
                config.insert(check.clone(), entry);
            }
        }
    }
    refresh_profiles(app);
    let profiles: Vec<String> = app.view.font_audit.profiles.iter().map(|c| c.id.clone()).collect();
    let (include, checks, exclude) = if built_in {
        (vec![id.clone()], Vec::new(), Vec::new())
    } else {
        (strings(shown.get("include")), strings(shown.get("checks")), strings(shown.get("exclude")))
    };
    let base = base_of(app, &include);
    let mut draft = ProfileDraft {
        original: (!built_in).then(|| (id.clone(), store.clone())),
        read_only: built_in,
        id,
        label: shown.get("label").and_then(Value::as_str).unwrap_or_default().to_string(),
        store: if built_in { "user".into() } else { store },
        include,
        checks,
        exclude,
        overrides: if built_in { Vec::new() } else { overrides },
        config: if built_in { BTreeMap::new() } else { config },
        values: project_values(app),
        changed: BTreeMap::new(),
        selected: None,
        thresholds: Vec::new(),
        search: String::new(),
        catalogue: catalogue(app),
        base,
        profiles,
    };
    if copy {
        draft = draft.duplicate();
    }
    app.view.font_audit.draft = Some(draft);
    Ok(())
}

pub fn close(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let action = string(p, "action")?.unwrap_or_else(|| "cancel".into());
    let draft = app.view.font_audit.draft.clone().ok_or("the Audit Profile dialog is not open")?;
    match action.as_str() {
        "cancel" => {}
        "save" => {
            if draft.read_only {
                return Err("a built-in profile cannot be changed: Duplicate it first".into());
            }
            let save = draft.save_params();
            let msg = if draft.changed.is_empty() {
                Msg::run("audit.profile.save", save)
            } else {
                let steps = json!([
                    {"command": "audit.config.set", "params": {"values": draft.changed}},
                    {"command": "audit.profile.save", "params": save},
                ]);
                Msg::run("command.batch", json!({"label": TITLE, "steps": steps}))
            };
            app.apply_msg(msg)?;
        }
        "delete" => {
            let (id, store) = draft.original.as_ref().ok_or("this profile is not saved: Cancel discards it")?;
            app.apply_msg(Msg::run("audit.profile.delete", json!({"profile": id, "store": store})))?;
        }
        other => return Err(format!("`action` must be \"cancel\", \"save\" or \"delete\", got \"{}\"", quoted(other))),
    }
    app.view.font_audit.draft = None;
    if action != "cancel" {
        refresh_profiles(app);
    }
    Ok(())
}

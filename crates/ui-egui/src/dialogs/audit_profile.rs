//! The Audit Profile dialog (`dialog.auditProfile`, M12.64): edits a user or project audit
//! profile, which checks it runs with which severity and thresholds, and the open document's
//! project values (`docs/spec/21-qa.md` §§ 2, 7).
//!
//! Rows: `dialog.auditProfile`, `audit.editProfile`, `audit.closeProfileDialog`.
//!
//! | Control | Does |
//! |---|---|
//! | name, label, storage | the profile's id (new profiles only), label and store |
//! | included profiles | the profiles whose checks it starts from |
//! | a check's box | runs the check or not: adds it to `checks` or `exclude`, or takes it out of them |
//! | a check's severity pop-up | a severity override for every message of the check |
//! | a check's name | selects it; its thresholds show below the list |
//! | threshold fields | the profile's threshold defaults (`config`) |
//! | project values | `audit.config.set {values}` of the open document, on Save |
//! | Duplicate | an editable copy under a new id, in the user's profiles |
//! | Delete | `audit.closeProfileDialog {action: "delete"}`: `audit.profile.delete` |
//! | Cancel, Save | `audit.closeProfileDialog {action: "cancel" \| "save"}` |
//!
//! The dialog edits a draft ([`ProfileDraft`]) and changes nothing until Save, which runs
//! `audit.profile.save` with the draft's parameters, together with `audit.config.set` in one
//! `command.batch` when a project value changed. Agents run those commands themselves. A
//! built-in profile opens read-only, as a draft that includes it; Duplicate makes that an editable
//! user profile. A Save or a Delete the engine refuses keeps the dialog open and changes nothing.

use std::collections::{BTreeMap, HashSet};

use egui::{Id, RichText, Sense, Stroke, StrokeKind, vec2};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::panels::{Pending, widgets};
use crate::shell::font_audit::{GROUPS, Shared, refresh_profiles};
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

pub const OPEN: &str = "audit.editProfile";
pub const CLOSE: &str = "audit.closeProfileDialog";
/// The dialog's title.
const TITLE: &str = "Audit Profile";
/// Most checks the dialog lists.
const MAX_CHECKS: usize = 4_096;
/// Most entries the dialog reads into one list of a profile.
const MAX_ENTRIES: usize = 1_024;
/// Most thresholds of one check.
const MAX_THRESHOLDS: usize = 64;
/// Longest id, label or text value the dialog takes, in characters.
const MAX_TEXT: usize = 256;
/// Most copies Duplicate numbers before it gives up looking for a free id.
const MAX_COPIES: usize = 1_000;
/// Rows the check list shows before it scrolls.
const VISIBLE_ROWS: f32 = 14.0;
/// The severity pop-up's choices: no override, then the three severities.
const SEVERITIES: [(&str, &str); 4] = [("", "Default"), ("fail", "Fail"), ("warn", "Warn"), ("info", "Info")];

pub const COMMANDS: &[UiCommand] = &[
    (
        OPEN,
        "Edit Audit Profile",
        &[],
        "",
        "{profile?: id (default: the Font Audit panel's profile, else the effective default profile), duplicate?: bool (open an editable copy under a new id instead)} opens the Audit Profile dialog on a built-in, user or project profile, with the open document's project values. A built-in profile opens read-only. Nothing changes until audit.closeProfileDialog {action: \"save\"} → {open, profile, label, store, readOnly, saved, include, checks, exclude, overrides, config, runs: count of checks the draft runs, values: {name: value}}",
    ),
    (
        CLOSE,
        "Close Audit Profile",
        &[],
        "",
        "{action?: \"cancel\" (default) | \"save\" | \"delete\"} closes the Audit Profile dialog. save: audit.profile.save with the draft (and audit.config.set of the changed project values, both as one command.batch); delete: audit.profile.delete of the profile the dialog opened on. A refused save or delete keeps the dialog open and changes nothing → {closed, action, profile, result?}",
    ),
];

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
    pub catalogue: Shared<Vec<CheckRow>>,
    /// The checks the included profiles run.
    pub base: Shared<HashSet<String>>,
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
    /// Whether the draft runs check `id`: the included profiles' checks plus `checks`, minus
    /// `exclude`.
    pub fn runs(&self, id: &str) -> bool {
        (self.base.contains(id) || self.checks.iter().any(|p| matches(id, p))) && !self.exclude.iter().any(|p| matches(id, p))
    }

    /// Turns check `id` on or off, by taking it out of `checks` or `exclude` where that is
    /// enough, else by adding it to the other list.
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
    fn override_of(&self, id: &str) -> &str {
        self.overrides.iter().find(|o| o.check == id && o.code.is_none()).map_or("", |o| o.severity.as_str())
    }

    fn set_override(&mut self, id: &str, severity: &str) {
        self.overrides.retain(|o| !(o.check == id && o.code.is_none()));
        if !severity.is_empty() {
            self.overrides.push(Override { check: id.to_string(), code: None, severity: severity.to_string() });
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
    fn count(&self) -> usize {
        self.catalogue.iter().filter(|c| self.runs(&c.id)).count()
    }
}

// ── Reading the engine ──────────────────────────────────────────────────────────────────────────

fn catalogue(app: &mut FontcraftApp) -> Vec<CheckRow> {
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
fn base_of(app: &mut FontcraftApp, include: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for p in include {
        if let Ok(v) = app.session.query("audit.profile.show", &json!({"profile": p})) {
            out.extend(strings(v.pointer("/resolved/checks")));
        }
    }
    out
}

fn thresholds_of(app: &mut FontcraftApp, check: &str) -> Vec<Threshold> {
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

fn project_values(app: &mut FontcraftApp) -> Vec<ProjectValue> {
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

/// The first free id of `<id>-copy`, `<id>-copy-2`… among `taken`.
fn copy_id(id: &str, taken: &[String]) -> String {
    let base = format!("{id}-copy");
    if !taken.contains(&base) {
        return base;
    }
    (2..MAX_COPIES).map(|n| format!("{base}-{n}")).find(|c| !taken.contains(c)).unwrap_or(base)
}

/// The draft as an editable copy under a new id, in the user's profiles.
fn duplicate(mut d: ProfileDraft) -> ProfileDraft {
    d.id = copy_id(&d.id, &d.profiles);
    if !d.label.is_empty() {
        d.label = format!("{} Copy", d.label);
    }
    d.original = None;
    d.read_only = false;
    d.store = "user".into();
    d
}

fn open(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let profile = match args.string("profile")? {
        Some(p) => p.to_string(),
        None => match app.state.panels.font_audit.profile.clone() {
            Some(p) => p,
            None => app
                .session
                .query("audit.config.get", &json!({}))
                .ok()
                .and_then(|v| v.get("profile").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| "universal".into()),
        },
    };
    let copy = args.flag("duplicate")?.unwrap_or(false);
    let shown = app.session.query("audit.profile.show", &json!({"profile": profile})).map_err(|e| args.err(e))?;
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
    let profiles: Vec<String> = app.state.panels.font_audit.profiles.iter().map(|(id, _)| id.clone()).collect();
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
        catalogue: Shared::new(catalogue(app)),
        base: Shared::new(base),
        profiles,
    };
    if copy {
        draft = duplicate(draft);
    }
    app.state.panels.font_audit.dialog = Some(draft);
    dialog_json(app)
}

fn dialog_json(app: &FontcraftApp) -> Result<Value, String> {
    let d = app.state.panels.font_audit.dialog.as_ref().ok_or_else(|| format!("{OPEN}: the Audit Profile dialog is not open"))?;
    let values: Map<String, Value> =
        d.values.iter().map(|v| (v.name.clone(), d.changed.get(&v.name).cloned().unwrap_or_else(|| v.value.clone()))).collect();
    let mut out = d.save_params();
    if let Value::Object(m) = &mut out {
        m.insert("open".into(), json!(true));
        m.insert("readOnly".into(), json!(d.read_only));
        m.insert("saved".into(), json!(d.original.is_some()));
        m.insert("runs".into(), json!(d.count()));
        m.insert("values".into(), Value::Object(values));
    }
    Ok(out)
}

fn close(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let action = args.string("action")?.unwrap_or("cancel");
    let draft = app.state.panels.font_audit.dialog.clone().ok_or_else(|| args.err("the Audit Profile dialog is not open"))?;
    let result = match action {
        "cancel" => Value::Null,
        "save" => {
            if draft.read_only {
                return Err(args.err("a built-in profile cannot be changed: Duplicate it first"));
            }
            let save = draft.save_params();
            if draft.changed.is_empty() {
                app.run("audit.profile.save", &save)?
            } else {
                let steps = json!([
                    {"command": "audit.config.set", "params": {"values": draft.changed}},
                    {"command": "audit.profile.save", "params": save},
                ]);
                app.run("command.batch", &json!({"label": TITLE, "steps": steps}))?
            }
        }
        "delete" => {
            let (id, store) = draft.original.as_ref().ok_or_else(|| args.err("this profile is not saved: Cancel discards it"))?;
            app.run("audit.profile.delete", &json!({"profile": id, "store": store}))?
        }
        other => {
            return Err(
                args.err(format!("`action` must be \"cancel\", \"save\" or \"delete\", got \"{}\"", other.chars().take(64).collect::<String>()))
            );
        }
    };
    app.state.panels.font_audit.dialog = None;
    if action != "cancel" {
        refresh_profiles(app);
    }
    let mut out = json!({"closed": true, "action": action, "profile": draft.id});
    if !result.is_null()
        && let Value::Object(m) = &mut out
    {
        m.insert("result".into(), result);
    }
    Ok(out)
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        OPEN => open(app, args),
        CLOSE => close(app, args),
        other => Err(format!("unknown command `{other}`")),
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

/// The id of a dialog control: `id`, `label`, `user`, `project`, `search`, `duplicate`, `delete`,
/// `cancel`, `save`.
pub fn control_id(name: &str) -> Id {
    widgets::id(("dialog.auditProfile", name))
}

/// The id of check `check`'s box.
pub fn check_id(check: &str) -> Id {
    widgets::id(("dialog.auditProfile", "check", check))
}

/// The id of profile `profile`'s box in the include list.
pub fn include_id(profile: &str) -> Id {
    widgets::id(("dialog.auditProfile", "include", profile))
}

/// A box with a stable id, ticked while `on`; true when clicked.
fn tick(ui: &mut egui::Ui, id: Id, on: bool, enabled: bool, tokens: &Tokens) -> bool {
    let side = ui.spacing().interact_size.y * 0.7;
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let colour = if enabled { tokens.text } else { tokens.text_dim };
        ui.painter().rect(rect, visuals.corner_radius, visuals.bg_fill, Stroke::new(tokens.outline_width.max(1.0), colour), StrokeKind::Inside);
        if on {
            let r = rect.shrink(side * 0.2);
            let points = vec![egui::pos2(r.left(), r.center().y), egui::pos2(r.left() + r.width() * 0.4, r.bottom()), egui::pos2(r.right(), r.top())];
            ui.painter().add(egui::Shape::line(points, Stroke::new(tokens.outline_width.max(1.5), colour)));
        }
    }
    enabled && response.clicked()
}

/// A row of the check list.
enum ListRow<'a> {
    Group(&'a str),
    Check(&'a CheckRow),
}

/// What the person changed this frame that needs the engine: the selected check's thresholds
/// and the included profiles' checks are read after drawing.
#[derive(Default)]
struct Reads {
    selected: Option<String>,
    include_changed: bool,
}

fn names_part(ui: &mut egui::Ui, d: &mut ProfileDraft, tokens: &Tokens) {
    let editable = !d.read_only;
    egui::Grid::new("dialog.auditProfile.names").num_columns(2).spacing([tokens.dialog_spacing, tokens.dialog_spacing / 2.0]).show(ui, |ui| {
        ui.label("Name");
        if d.original.is_none() && editable {
            if let Some(id) = widgets::text(ui, control_id("id"), &d.id, tokens.dialog_field_width) {
                d.id = id.chars().take(MAX_TEXT).collect();
            }
        } else {
            ui.label(&d.id);
        }
        ui.end_row();
        ui.label("Label");
        if editable {
            if let Some(label) = widgets::text(ui, control_id("label"), &d.label, tokens.dialog_field_width) {
                d.label = label.chars().take(MAX_TEXT).collect();
            }
        } else {
            ui.label(&d.label);
        }
        ui.end_row();
        ui.label("Storage");
        ui.horizontal(|ui| {
            let fixed = d.original.is_some() || !editable;
            for (store, label, hover) in [
                ("user", "My Profiles", "Kept in the application settings, for every font"),
                ("project", "This Font", "Kept in the font file, for everyone who opens it"),
            ] {
                if widgets::button(ui, control_id(store), label, hover, !fixed, d.store == store) {
                    d.store = store.to_string();
                }
            }
        });
        ui.end_row();
    });
}

fn include_part(ui: &mut egui::Ui, d: &mut ProfileDraft, tokens: &Tokens, reads: &mut Reads) {
    ui.label(RichText::new("Starts from").strong());
    ui.horizontal_wrapped(|ui| {
        let offered: Vec<String> = d.profiles.iter().filter(|p| **p != d.id || d.read_only).cloned().collect();
        for p in offered {
            let on = d.include.contains(&p);
            if tick(ui, include_id(&p), on, !d.read_only, tokens) {
                if on {
                    d.include.retain(|i| *i != p);
                } else {
                    d.include.push(p.clone());
                }
                reads.include_changed = true;
            }
            ui.label(&p);
        }
    });
}

fn checks_part(ui: &mut egui::Ui, d: &mut ProfileDraft, tokens: &Tokens, reads: &mut Reads) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Checks ({} of {} run)", d.count(), d.catalogue.len())).strong());
        let mut search = d.search.clone();
        let edit = egui::TextEdit::singleline(&mut search).id(control_id("search")).hint_text("Search").desired_width(tokens.dialog_field_width);
        if ui.add(edit).changed() {
            d.search = search.chars().take(MAX_TEXT).collect();
        }
    });
    let needle = d.search.trim().to_lowercase();
    let catalogue = d.catalogue.clone();
    let mut rows: Vec<ListRow> = Vec::new();
    for (group, _) in GROUPS {
        let mut first = true;
        for c in catalogue.iter().filter(|c| c.group == group) {
            if !needle.is_empty() && !c.id.to_lowercase().contains(&needle) && !c.label.to_lowercase().contains(&needle) {
                continue;
            }
            if first {
                rows.push(ListRow::Group(group));
                first = false;
            }
            rows.push(ListRow::Check(c));
        }
    }
    let line = tokens.list_row_height;
    let enabled = !d.read_only;
    egui::ScrollArea::vertical().id_salt("dialog.auditProfile.checks").auto_shrink([false, true]).max_height(line * VISIBLE_ROWS).show_rows(
        ui,
        line,
        rows.len(),
        |ui, range| {
            for row in rows.get(range).unwrap_or_default() {
                match row {
                    ListRow::Group(g) => {
                        let label = GROUPS.iter().find(|(id, _)| id == g).map_or(*g, |(_, l)| *l);
                        ui.add_sized([ui.available_width(), line], egui::Label::new(RichText::new(label).strong()));
                    }
                    ListRow::Check(c) => {
                        ui.horizontal(|ui| {
                            ui.set_height(line);
                            if tick(ui, check_id(&c.id), d.runs(&c.id), enabled, tokens) {
                                d.toggle(&c.id);
                            }
                            let selected = d.selected.as_deref() == Some(c.id.as_str());
                            let name = if c.label.is_empty() { c.id.as_str() } else { c.label.as_str() };
                            if ui.selectable_label(selected, name).on_hover_text(&c.id).clicked() && !selected {
                                reads.selected = Some(c.id.clone());
                            }
                            let current = d.override_of(&c.id).to_string();
                            let caption = match current.as_str() {
                                "" => format!("Default ({})", c.severity),
                                s => SEVERITIES.iter().find(|(id, _)| *id == s).map_or_else(|| s.to_string(), |(_, l)| l.to_string()),
                            };
                            ui.add_enabled_ui(enabled, |ui| {
                                egui::ComboBox::from_id_salt(("dialog.auditProfile.severity", c.id.as_str())).selected_text(caption).show_ui(
                                    ui,
                                    |ui| {
                                        for (id, label) in SEVERITIES {
                                            if ui.selectable_label(current == id, label).clicked() && current != id {
                                                d.set_override(&c.id, id);
                                            }
                                        }
                                    },
                                );
                            });
                        });
                    }
                }
            }
        },
    );
}

fn thresholds_part(ui: &mut egui::Ui, d: &mut ProfileDraft, tokens: &Tokens) {
    let Some(check) = d.selected.clone() else { return };
    ui.label(RichText::new(format!("Thresholds of {check}")).strong());
    if d.thresholds.is_empty() {
        ui.colored_label(tokens.text_dim, "This check has no thresholds.");
        return;
    }
    let thresholds = d.thresholds.clone();
    egui::Grid::new("dialog.auditProfile.thresholds").num_columns(3).show(ui, |ui| {
        for t in &thresholds {
            ui.label(&t.name).on_hover_text(&t.doc);
            let value = d.config.get(&check).and_then(|m| m.get(&t.name)).copied();
            let id = widgets::id(("dialog.auditProfile", "threshold", t.name.as_str()));
            if !d.read_only {
                match widgets::number(ui, id, value, tokens.field_width, true) {
                    Some(widgets::NumberEdit::Set(v)) => {
                        let v = t.min.map_or(v, |m| v.max(m));
                        let v = t.max.map_or(v, |m| v.min(m));
                        d.config.entry(check.clone()).or_default().insert(t.name.clone(), v);
                    }
                    Some(widgets::NumberEdit::Clear) => {
                        if let Some(m) = d.config.get_mut(&check) {
                            m.remove(&t.name);
                            if m.is_empty() {
                                d.config.remove(&check);
                            }
                        }
                    }
                    None => {}
                }
            } else {
                ui.label(value.map_or_else(|| "--".to_string(), widgets::format_number));
            }
            ui.colored_label(tokens.text_dim, format!("default {}", widgets::format_number(t.default)));
            ui.end_row();
        }
    });
}

fn values_part(ui: &mut egui::Ui, d: &mut ProfileDraft, tokens: &Tokens) {
    if d.values.is_empty() {
        return;
    }
    ui.label(RichText::new("Project values of this font").strong());
    let values = d.values.clone();
    egui::Grid::new("dialog.auditProfile.values").num_columns(2).show(ui, |ui| {
        for v in &values {
            ui.label(&v.name).on_hover_text(&v.doc);
            let now = d.changed.get(&v.name).cloned().unwrap_or_else(|| v.value.clone());
            let id = widgets::id(("dialog.auditProfile", "value", v.name.as_str()));
            let edited: Option<Value> = match v.kind.as_str() {
                "true or false" => {
                    let on = now.as_bool().unwrap_or(false);
                    tick(ui, id, on, true, tokens).then(|| json!(!on))
                }
                "whole number" => match widgets::number(ui, id, now.as_f64(), tokens.field_width, true) {
                    Some(widgets::NumberEdit::Set(n)) => Some(json!(n.round() as i64)),
                    Some(widgets::NumberEdit::Clear) => Some(Value::Null),
                    None => None,
                },
                "list of table tags" => {
                    let text = now.as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                    widgets::text(ui, id, &text, tokens.dialog_field_width).map(|t| {
                        let tags: Vec<String> = t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(MAX_ENTRIES).collect();
                        if tags.is_empty() { Value::Null } else { json!(tags) }
                    })
                }
                _ => {
                    let text = now.as_str().unwrap_or_default().to_string();
                    widgets::text(ui, id, &text, tokens.dialog_field_width)
                        .map(|t| if t.is_empty() { Value::Null } else { json!(t.chars().take(MAX_TEXT).collect::<String>()) })
                }
            };
            if let Some(value) = edited {
                if value == v.value {
                    d.changed.remove(&v.name);
                } else {
                    d.changed.insert(v.name.clone(), value);
                }
            }
            ui.end_row();
        }
    });
}

/// Draws the dialog while it is open. Save, Delete and Cancel queue `audit.closeProfileDialog`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(mut d) = app.state.panels.font_audit.dialog.clone() else { return };
    let before = d.clone();
    let mut reads = Reads::default();
    let tokens = &app.tokens;
    let area = egui::Modal::default_area(Id::new("dialog.auditProfile")).anchor(egui::Align2::CENTER_TOP, vec2(0.0, tokens.dialog_top));
    let modal = egui::Modal::new(Id::new("dialog.auditProfile"))
        .area(area)
        .backdrop_color(tokens.dialog_backdrop)
        .frame(egui::Frame::window(ui.style()))
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new(TITLE).strong().size(tokens.dialog_title_size));
            if d.read_only {
                ui.colored_label(tokens.text_dim, "Built-in profiles cannot be changed; Duplicate makes an editable copy.");
            }
            ui.add_space(tokens.dialog_spacing);
            names_part(ui, &mut d, tokens);
            ui.add_space(tokens.dialog_spacing);
            include_part(ui, &mut d, tokens, &mut reads);
            ui.add_space(tokens.dialog_spacing);
            checks_part(ui, &mut d, tokens, &mut reads);
            ui.add_space(tokens.dialog_spacing);
            thresholds_part(ui, &mut d, tokens);
            ui.add_space(tokens.dialog_spacing);
            values_part(ui, &mut d, tokens);
            ui.add_space(tokens.dialog_spacing);
            let (saved, read_only) = (d.original.is_some(), d.read_only);
            let mut copy = false;
            ui.horizontal(|ui| {
                copy = widgets::button(ui, control_id("duplicate"), "Duplicate", "Make an editable copy under a new name", true, false);
                if widgets::button(ui, control_id("delete"), "Delete", "Delete this profile", saved, false) {
                    pending.push((CLOSE, json!({"action": "delete"})));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, control_id("save"), "Save", "Save the profile", !read_only, false) {
                        pending.push((CLOSE, json!({"action": "save"})));
                    }
                    if widgets::button(ui, control_id("cancel"), "Cancel", "Close without saving", true, false) {
                        pending.push((CLOSE, json!({"action": "cancel"})));
                    }
                });
            });
            if copy {
                d = duplicate(d.clone());
            }
        });
    if modal.should_close() {
        pending.push((CLOSE, json!({"action": "cancel"})));
    }
    if let Some(check) = reads.selected {
        d.thresholds = thresholds_of(app, &check);
        d.selected = Some(check);
    }
    if reads.include_changed {
        d.base = Shared::new(base_of(app, &d.include));
    }
    if d != before
        && let Some(slot) = app.state.panels.font_audit.dialog.as_mut()
    {
        *slot = d;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use egui::{Event, Modifiers, PointerButton, Pos2};
    use serde_json::{Value, json};

    use super::*;
    use crate::control;

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn command(app: &mut FontcraftApp, id: &str, params: Value) -> Value {
        call(app, "engine.execute", json!({"command": id, "params": params}))
    }

    /// An app with a user profile `house` that starts from the outlines profile.
    fn app_with_house() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("audit.profile.save", &json!({"profile": "house", "label": "House", "include": ["outlines"]})).unwrap();
        app
    }

    fn shown(app: &mut FontcraftApp, id: &str) -> Value {
        app.session.query("audit.profile.show", &json!({"profile": id})).unwrap()
    }

    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 1000.0))), events, ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
            out.textures_delta.clear();
        }

        /// Two frames for the modal to settle, then a click on `id`.
        fn click(&self, app: &mut FontcraftApp, id: Id) {
            self.frame(app, Vec::new());
            self.frame(app, Vec::new());
            let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(app, vec![Event::PointerMoved(at)]);
            self.frame(app, vec![button(true)]);
            self.frame(app, vec![button(false)]);
        }
    }

    #[test]
    fn audit_edit_profile_and_close_profile_dialog_save_what_audit_profile_save_saves() {
        let mut app = app_with_house();
        let opened = command(&mut app, OPEN, json!({"profile": "house"}));
        assert_eq!((opened["profile"].clone(), opened["store"].clone(), opened["readOnly"].clone()), (json!("house"), json!("user"), json!(false)));
        assert_eq!(opened["include"], json!(["outlines"]));
        let outlines = shown(&mut app, "outlines")["resolved"]["count"].clone();
        assert_eq!(opened["runs"], outlines, "the draft runs what its include runs");

        // Untick the first check of the list and save from the dialog.
        let catalogue = app.state.panels.font_audit.dialog.as_ref().unwrap().catalogue.clone();
        let first = GROUPS.iter().find_map(|(g, _)| catalogue.iter().find(|c| c.group == *g)).unwrap().id.clone();
        let w = Window { ctx: egui::Context::default() };
        w.click(&mut app, check_id(&first));
        let d = app.state.panels.font_audit.dialog.clone().unwrap();
        assert_eq!(d.runs(&first), !shown(&mut app, "outlines")["resolved"]["checks"].as_array().unwrap().contains(&json!(first)));
        assert_eq!(shown(&mut app, "house")["exclude"], json!([]), "nothing is stored before Save");
        let params = d.save_params();
        w.click(&mut app, control_id("save"));
        assert!(app.state.panels.font_audit.dialog.is_none(), "Save closes the dialog");

        let mut direct = app_with_house();
        direct.run("audit.profile.save", &params).unwrap();
        assert_eq!(shown(&mut app, "house"), shown(&mut direct, "house"), "the dialog saves what the command saves");
    }

    #[test]
    fn a_built_in_profile_opens_read_only_and_duplicate_saves_a_user_profile_that_includes_it() {
        let mut app = FontcraftApp::default();
        command(&mut app, OPEN, json!({"profile": "outlines"}));
        let r = control::handle(&mut app, &json!({"id": 1, "method": "engine.execute", "params": {"command": CLOSE, "params": {"action": "save"}}}));
        assert_eq!(r["ok"], false, "a built-in profile cannot be saved: {r}");
        assert!(app.state.panels.font_audit.dialog.is_some(), "a refused save keeps the dialog open");

        let w = Window { ctx: egui::Context::default() };
        w.click(&mut app, control_id("duplicate"));
        let d = app.state.panels.font_audit.dialog.clone().unwrap();
        assert_eq!((d.id.as_str(), d.read_only, d.store.as_str()), ("outlines-copy", false, "user"));
        let out = command(&mut app, CLOSE, json!({"action": "save"}));
        assert_eq!(out["closed"], true);
        let copy = shown(&mut app, "outlines-copy");
        assert_eq!(copy["include"], json!(["outlines"]));
        assert_eq!(copy["store"], "user");
        assert_eq!(copy["resolved"]["count"], shown(&mut app, "outlines")["resolved"]["count"]);

        // Delete from the dialog removes it; Cancel changes nothing.
        command(&mut app, OPEN, json!({"profile": "outlines-copy"}));
        command(&mut app, CLOSE, json!({"action": "cancel"}));
        assert!(app.session.query("audit.profile.show", &json!({"profile": "outlines-copy"})).is_ok());
        command(&mut app, OPEN, json!({"profile": "outlines-copy"}));
        command(&mut app, CLOSE, json!({"action": "delete"}));
        assert!(app.session.query("audit.profile.show", &json!({"profile": "outlines-copy"})).is_err());
    }

    #[test]
    fn toggling_a_check_uses_checks_and_exclude_as_little_as_it_can() {
        let mut app = app_with_house();
        command(&mut app, OPEN, json!({"profile": "house"}));
        let mut d = app.state.panels.font_audit.dialog.clone().unwrap();
        let on = d.catalogue.iter().find(|c| d.runs(&c.id)).unwrap().id.clone();
        let off = d.catalogue.iter().find(|c| !d.runs(&c.id)).unwrap().id.clone();
        d.toggle(&on);
        d.toggle(&off);
        assert_eq!((d.exclude.clone(), d.checks.clone()), (vec![on.clone()], vec![off.clone()]));
        d.toggle(&on);
        d.toggle(&off);
        assert!(d.exclude.is_empty() && d.checks.is_empty(), "turning them back takes the entries out again");
        d.checks = vec!["check.outlines.*".into()];
        let outline = d.catalogue.iter().find(|c| c.id.starts_with("check.outlines.")).unwrap().id.clone();
        d.toggle(&outline);
        assert!(!d.runs(&outline) && d.exclude == vec![outline.clone()], "a check a pattern adds is excluded by id");
        d.set_override(&outline, "fail");
        assert_eq!(d.save_params()["overrides"], json!([{"check": outline, "severity": "fail"}]));
        d.set_override(&outline, "");
        assert_eq!(d.save_params()["overrides"], json!([]));
    }
}

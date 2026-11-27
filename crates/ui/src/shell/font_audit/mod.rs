//! The Font Audit panel's shell commands and view state (`window.fontAudit`, `docs/spec/21-qa.md` § 7).
//!
//! | Command | Does |
//! |---|---|
//! | `audit.showPanel {visible?, profile?, target?}` | shows or hides the panel and sets the profile and target Run uses |
//! | `audit.panel.run {profile?, target?, again?, inSteps?}` | `audit.run`, or `audit.start` for a run in steps, and lists the result document |
//! | `audit.panel.continue {budget?}` | `audit.step` of the run in steps; the app posts it once a frame while the panel shows the run |
//! | `audit.panel.cancel` | `audit.cancel`; lists what the run had found, marked partial |
//! | `audit.panel.previewFix {results?, safeOnly?}` | a dry run of `audit.fix`, shown in the detail area with Apply |
//! | `audit.panel.exportReport {path, format?, minSeverity?}` | `audit.export` of the listed run |
//! | `audit.revealIssue`, `audit.nextIssue`, `audit.previousIssue` | open what a result is about (`reveal.rs`) |
//! | `audit.filterResults {…}` | which results the list shows and how it groups them |
//! | `audit.panel.fix`, `audit.panel.ignore`, `audit.panel.unignore` | the Fix, Fix All Safe, Ignore and Stop Ignoring buttons: their engine commands, then a run again |
//! | `audit.panel.setView {…}` | the selected rows, the open groups, the reason and the export row (view state) |
//! | `audit.editProfile`, `audit.closeProfileDialog` | the Audit Profile dialog (`profile.rs`) |
//!
//! The panel keeps the result document of its last run ([`run::Run`]). The engine keeps the last run of each open
//! document too, and its result ids are what `audit.fix` and `audit.ignoreIssue` take. Live audit and agents can
//! replace the engine's run, so before a fix or an ignore the panel compares the two by their run ids and, when they
//! differ, runs its own parameters again first: on an unchanged font that gives the same ids. A fix of the kept run
//! names its run id, so the engine refuses it when another run replaced that one in between.
//!
//! A run in steps runs on the UI thread, one `audit.step` per frame ([`logic`]), so it works the same on the web.

pub mod profile;
mod reveal;
pub mod run;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Map, Value, json};

use self::run::{BUILT_IN, CheckInfo, Filter, GROUP_BY, GROUPS, Item, MAX_CHECKS, MAX_FONTS, MAX_ITEMS, Run, SEVERITIES, TARGETS};
use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use crate::app::{FontCraftApp, Msg};

pub const MODULE: Module = Module { commands: COMMANDS, run: run_command };

/// Longest search text and reason, in characters.
pub const MAX_TEXT: usize = 256;
/// Longest profile id a command takes, in bytes.
pub const MAX_PROFILE_ID: usize = 64;
/// Most profiles the profile pop-up lists.
pub const MAX_PROFILES: usize = 256;
/// Most rows `ui.inspect` lists; `listed` counts them all.
const MAX_RETURNED: usize = 100;
/// Most planned and skipped steps a fix preview lists.
const MAX_PREVIEW: usize = 100;
/// Longest report path, in bytes.
const MAX_PATH: usize = 4096;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;

/// The formats Export Report offers: the `format` of `audit.export`.
pub const EXPORT_FORMATS: [&str; 4] = ["html", "markdown", "json", "text"];

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "audit.showPanel",
        label: "Font Audit",
        menu: &["Window"],
        shortcut: None,
        params: "{visible?: bool (omitted: show it when hidden, hide it when shown), profile?: id | null (the profile Run uses; null: the document's default profile), target?: \"font\" | \"selectedGlyphs\" | \"currentGlyph\" | \"export\" | \"family\"} shows or hides the Font Audit panel and sets the pop-ups Run reads. ui.inspect reports the panel under fontAudit",
    },
    UiCommand {
        id: "audit.panel.run",
        label: "Run Font Audit",
        menu: &[],
        shortcut: None,
        params: "{profile?: id | null, target?: \"font\" (the open document) | \"selectedGlyphs\" (the Font View selection, else the shown glyph) | \"currentGlyph\" | \"export\" (the binary and family checks on the document's compiled instances) | \"family\" (the saved files of every open document, two or more), again?: bool (run the last run's parameters once more), inSteps?: bool (start the run with audit.start; the panel continues it once a frame with a progress bar and Cancel; not for a family of files)} runs audit.run with the panel's profile and target, ignored results included, and lists its result document in the panel, which shows. A failed run keeps the list as it was",
    },
    UiCommand {
        id: "audit.panel.continue",
        label: "Continue Font Audit",
        menu: &[],
        shortcut: None,
        params: "{budget?: count (layers, 1 to 100,000; default 256)} does the next step (audit.step) of the run the panel started in steps; the app runs it once a frame while the panel shows. The last step lists the run's results. Refused when no run is in progress; a run whose document is no longer active is dropped",
    },
    UiCommand {
        id: "audit.panel.cancel",
        label: "Cancel Font Audit",
        menu: &[],
        shortcut: None,
        params: "{} stops the run the panel started in steps (audit.cancel) and lists what it had found, marked partial: the font and the engine's kept run do not change, and Fix and Ignore are off for partial results. Refused when no run is in progress",
    },
    UiCommand {
        id: "audit.panel.previewFix",
        label: "Preview Audit Fix",
        menu: &[],
        shortcut: None,
        params: "{results?: [result id] (default the selected results that have a fix), safeOnly?: bool = false} asks audit.fix for a dry run of the fixes of those results of the panel's run and shows the edits it would make in the detail area, where Apply runs them. Nothing changes. Refused for a run the engine does not keep",
    },
    UiCommand {
        id: "audit.panel.exportReport",
        label: "Export Font Audit Report",
        menu: &[],
        shortcut: None,
        params: "{path: path, format?: \"json\" | \"markdown\" | \"html\" | \"text\" (default from the extension of path), minSeverity?: \"info\" | \"warn\" | \"fail\"} writes the run the panel lists, partial runs and families of files included, with audit.export",
    },
    UiCommand {
        id: "audit.revealIssue",
        label: "Reveal Issue",
        menu: &[],
        shortcut: None,
        params: "{result: id (from the panel's last run, else from the document's last audit.run), runId?: id (refused unless the panel lists that run)} a glyph result: shows the glyph on the result's layer, selects the nodes and components it reports and zooms to them; a result naming a Font Info field: shows Font Info on its tab; a family result: makes the first open document among its fonts active. The result becomes the panel's current and only selected result",
    },
    UiCommand {
        id: "audit.nextIssue",
        label: "Next Issue",
        menu: &[],
        shortcut: None,
        params: "{scope?: \"font\" (default) | \"glyph\" (the results of the shown glyph), runId?: id} reveals the listed result after the current one, as audit.revealIssue does, wrapping after the last; the panel's filters decide what is listed. Refused when nothing is listed",
    },
    UiCommand {
        id: "audit.previousIssue",
        label: "Previous Issue",
        menu: &[],
        shortcut: None,
        params: "{scope?: \"font\" (default) | \"glyph\" (the results of the shown glyph), runId?: id} reveals the listed result before the current one, as audit.revealIssue does, wrapping before the first. Refused when nothing is listed",
    },
    UiCommand {
        id: "audit.filterResults",
        label: "Filter Audit Results",
        menu: &[],
        shortcut: None,
        params: "{minSeverity?: \"info\" | \"warn\" | \"fail\", groups?: [group] (empty: every group), text?: string (matched against the check id, glyph, table and message, ignoring case), fixableOnly?: bool, currentGlyphOnly?: bool, showIgnored?: bool, groupBy?: \"check\" | \"glyph\" | \"severity\"} sets which results the Font Audit panel lists and how it groups them; the keys left out keep their value",
    },
    UiCommand {
        id: "audit.panel.fix",
        label: "Fix Audit Results",
        menu: &[],
        shortcut: None,
        params: "{results?: [result id] (default the selected results that have a fix), safeOnly?: bool = false, preview?: bool (the results and safeOnly of the fix preview: Apply)} runs audit.fix on those results of the panel's run, then runs the audit again. When the engine's kept run is no longer the panel's, the panel's parameters run again first",
    },
    UiCommand {
        id: "audit.panel.ignore",
        label: "Ignore Audit Results",
        menu: &[],
        shortcut: None,
        params: "{results?: [result id] (default the selected results not yet ignored), reason?: string (default the panel's reason field)} runs audit.ignoreIssue on each (several: one command.batch), then runs the audit again",
    },
    UiCommand {
        id: "audit.panel.unignore",
        label: "Stop Ignoring Audit Results",
        menu: &[],
        shortcut: None,
        params: "{results?: [result id] (default the selected ignored results)} runs audit.unignoreIssue on each (several: one command.batch), then runs the audit again",
    },
    UiCommand {
        id: "audit.panel.setView",
        label: "Set Font Audit View",
        menu: &[],
        shortcut: None,
        params: "{selected?: [result id], current?: id | null, groupOpen?: {group: key, open: bool}, reason?: string, exportOpen?: bool, exportPath?: path, exportFormat?: \"html\" | \"markdown\" | \"json\" | \"text\"} sets the panel's view state: the selected results, the current result, whether a group shows its results, the reason Ignore stores and the export row",
    },
    UiCommand {
        id: profile::OPEN,
        label: "Edit Audit Profile",
        menu: &[],
        shortcut: None,
        params: "{profile?: id (default: the Font Audit panel's profile, else the effective default profile), duplicate?: bool (open an editable copy under a new id instead)} opens the Audit Profile dialog on a built-in, user or project profile, with the open document's project values. A built-in profile opens read-only. Nothing changes until audit.closeProfileDialog {action: \"save\"}. ui.inspect reports the dialog under auditProfile",
    },
    UiCommand {
        id: profile::CLOSE,
        label: "Close Audit Profile",
        menu: &[],
        shortcut: None,
        params: "{action?: \"cancel\" (default) | \"save\" | \"delete\"} closes the Audit Profile dialog. save: audit.profile.save with the draft (and audit.config.set of the changed project values, both as one command.batch); delete: audit.profile.delete of the profile the dialog opened on. A refused save or delete keeps the dialog open and changes nothing",
    },
];

/// A run in steps (`audit.start`) the panel continues once a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub run_id: String,
    pub stage: String,
    pub done: u64,
    pub total: u64,
    /// The document it runs on.
    pub doc: u64,
    pub target: String,
    pub profile: Option<String>,
    /// The `audit.start` parameters, kept with the run's results for a run again.
    pub params: Value,
}

/// What `audit.fix` would do for some results (a dry run).
#[derive(Clone, Debug, PartialEq)]
pub struct FixPreview {
    pub ids: Vec<String>,
    pub run_id: Option<String>,
    pub safe_only: bool,
    pub planned: Vec<String>,
    pub skipped: Vec<String>,
}

/// A profile the pop-up lists: its id, and its label (`None` for a built-in one, which the view names).
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileChoice {
    pub id: String,
    pub label: Option<String>,
}

/// The panel's state.
#[derive(Clone, Debug, PartialEq)]
pub struct FontAuditView {
    /// The profile Run uses; `None`: the document's default profile.
    pub profile: Option<String>,
    /// One of [`TARGETS`].
    pub target: String,
    pub filter: Filter,
    pub run: Option<Arc<Run>>,
    /// Ids of the selected results.
    pub selected: Vec<String>,
    /// The result Previous and Next step from.
    pub current: Option<String>,
    /// The reason Ignore stores.
    pub reason: String,
    /// Keys of the groups that show only their heading.
    pub collapsed: Vec<String>,
    pub profiles: Vec<ProfileChoice>,
    /// The profile Run uses without a choice: the document's default, else the application's.
    pub default_profile: String,
    pub progress: Option<Progress>,
    pub preview: Option<FixPreview>,
    pub export_open: bool,
    pub export_path: String,
    /// One of [`EXPORT_FORMATS`].
    pub export_format: String,
    /// The Audit Profile dialog's draft while it is open.
    pub draft: Option<profile::ProfileDraft>,
}

impl Default for FontAuditView {
    fn default() -> Self {
        FontAuditView {
            profile: None,
            target: "font".into(),
            filter: Filter::default(),
            run: None,
            selected: Vec::new(),
            current: None,
            reason: String::new(),
            collapsed: Vec::new(),
            profiles: BUILT_IN.iter().map(|id| ProfileChoice { id: (*id).into(), label: None }).collect(),
            default_profile: "universal".into(),
            progress: None,
            preview: None,
            export_open: false,
            export_path: String::new(),
            export_format: "html".into(),
            draft: None,
        }
    }
}

/// Whether the Font Audit panel shows.
pub fn shows(app: &FontCraftApp) -> bool {
    app.view.workspace.layout().panel_state(&PanelId::FontAudit) != craft::behavior::dock::PanelState::Hidden
}

/// Once a frame, before the drain: a run in steps goes on by one step while the panel shows, and asks for the next
/// frame.
pub fn logic(app: &mut FontCraftApp, ctx: &egui::Context) {
    if app.view.font_audit.progress.is_some() && shows(app) {
        app.sent.push(Msg::Shell(CommandRef::with_params("audit.panel.continue", json!({}))));
        ctx.request_repaint();
    }
}

fn run_command(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = command.params.as_object().cloned().unwrap_or_default();
    let p = &params;
    match command.id.as_str() {
        "audit.showPanel" => show_panel(app, p),
        "audit.panel.run" => panel_run(app, p),
        "audit.panel.continue" => panel_continue(app, p),
        "audit.panel.cancel" => panel_cancel(app),
        "audit.panel.previewFix" => preview_fix(app, p),
        "audit.panel.exportReport" => export_report(app, p),
        "audit.revealIssue" => {
            let result = string(p, "result")?.ok_or("`result` (a result id of the last run) is required")?;
            reveal::reveal(app, p, &result)
        }
        "audit.nextIssue" => reveal::step(app, p, true),
        "audit.previousIssue" => reveal::step(app, p, false),
        "audit.filterResults" => filter_results(app, p),
        "audit.panel.fix" => fix(app, p),
        "audit.panel.ignore" => ignore(app, p),
        "audit.panel.unignore" => unignore(app, p),
        "audit.panel.setView" => set_view(app, p),
        profile::OPEN => profile::open(app, p),
        profile::CLOSE => profile::close(app, p),
        other => Err(format!("no Font Audit command `{other}`")),
    }
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// At most [`MAX_QUOTED`] characters of untrusted text, for a message.
pub fn quoted(s: &str) -> String {
    s.chars().take(MAX_QUOTED).collect()
}

pub fn flag(p: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

pub fn string(p: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

/// A list of strings of at most `max` entries.
pub fn strings(p: &Map<String, Value>, key: &str, max: usize) -> Result<Option<Vec<String>>, String> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| format!("`{key}` must be a list of strings"))?;
    if list.len() > max {
        return Err(format!("`{key}` holds at most {max} entries"));
    }
    list.iter()
        .map(|s| s.as_str().map(str::to_string).ok_or_else(|| format!("`{key}` must be a list of strings")))
        .collect::<Result<_, _>>()
        .map(Some)
}

fn known(key: &str, value: &str, list: &[&str]) -> Result<String, String> {
    if list.contains(&value) { Ok(value.to_string()) } else { Err(format!("unknown {key} `{}`: use {}", quoted(value), list.join(", "))) }
}

/// The `profile` given (`Some(None)`: null, the default profile), or `None` when absent.
fn profile_param(p: &Map<String, Value>) -> Result<Option<Option<String>>, String> {
    match p.get("profile") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if !s.is_empty() && s.len() <= MAX_PROFILE_ID => Ok(Some(Some(s.clone()))),
        Some(_) => Err(format!("`profile` must be a profile id of 1 to {MAX_PROFILE_ID} bytes, or null")),
    }
}

fn target_param(p: &Map<String, Value>, current: &str) -> Result<String, String> {
    match string(p, "target")? {
        None => Ok(current.to_string()),
        Some(t) => known("target", &t, &TARGETS),
    }
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn show_panel(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let visible = flag(p, "visible")?.unwrap_or(!shows(app));
    let profile = profile_param(p)?.unwrap_or_else(|| app.view.font_audit.profile.clone());
    let target = target_param(p, &app.view.font_audit.target)?;
    set_visible(app, visible);
    let st = &mut app.view.font_audit;
    st.profile = profile;
    st.target = target;
    if visible {
        refresh_profiles(app);
    }
    Ok(())
}

fn set_visible(app: &mut FontCraftApp, visible: bool) {
    crate::shell::dock::show_window(&mut app.view.workspace, PanelId::FontAudit, visible, app.view.canvas.rect);
}

/// Reads the profiles the pop-up lists: the built-in ones, the document's and the user's, and the effective
/// default.
pub fn refresh_profiles(app: &mut FontCraftApp) {
    let mut list: Vec<ProfileChoice> = BUILT_IN.iter().map(|id| ProfileChoice { id: (*id).into(), label: None }).collect();
    let mut default = None;
    if let Ok(v) = app.session.query("audit.profile.list", &json!({}))
        && let Some(stored) = v.get("profiles").and_then(Value::as_array)
    {
        for p in stored.iter().take(MAX_PROFILES + BUILT_IN.len()) {
            let Some(id) = p.get("id").and_then(Value::as_str) else { continue };
            if p.get("default").and_then(Value::as_bool) == Some(true) {
                default = Some(id.to_string());
            }
            if list.iter().any(|c| c.id == id) {
                continue;
            }
            let label = p.get("label").and_then(Value::as_str).filter(|l| !l.is_empty()).unwrap_or(id);
            list.push(ProfileChoice { id: id.into(), label: Some(label.into()) });
        }
    }
    let st = &mut app.view.font_audit;
    for p in st.profile.iter().chain(default.iter()) {
        if !list.iter().any(|c| c.id == *p) {
            list.push(ProfileChoice { id: p.clone(), label: Some(p.clone()) });
        }
    }
    st.profiles = list;
    if let Some(d) = default {
        st.default_profile = d;
    }
}

/// The glyph the panel calls current: the shown glyph.
pub fn current_glyph(app: &FontCraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    app.view.glyph.as_ref().filter(|g| font.glyphs.get(g.as_str()).is_some()).cloned().or_else(|| font.glyphs.first().map(|g| g.name.clone()))
}

/// The glyphs Selected Glyphs audits: the Font View selection, else the shown glyph.
fn selected_glyphs(app: &FontCraftApp) -> Vec<String> {
    let selected: Vec<String> = app.view.font_view.selected.iter().take(fontcraft_model::MAX_GLYPHS).cloned().collect();
    if selected.is_empty() { current_glyph(app).into_iter().collect() } else { selected }
}

/// The `audit.run` parameters for `target` and `profile`.
fn run_params(app: &FontCraftApp, target: &str, profile: Option<&str>) -> Result<Value, String> {
    let mut p = Map::new();
    if let Some(profile) = profile {
        p.insert("profile".into(), json!(profile));
    }
    p.insert("includeIgnored".into(), json!(true));
    match target {
        "selectedGlyphs" => {
            let glyphs = selected_glyphs(app);
            if glyphs.is_empty() {
                return Err("no glyph is selected".into());
            }
            p.insert("glyphs".into(), json!(glyphs));
        }
        "currentGlyph" => {
            let glyph = current_glyph(app).ok_or("no glyph is shown")?;
            p.insert("glyphs".into(), json!([glyph]));
        }
        "export" => {
            p.insert("scope".into(), json!(["binary", "family"]));
        }
        "family" => {
            let paths: Vec<String> =
                app.session.documents().iter().filter_map(|d| d.path.as_ref()).take(MAX_FONTS).map(|p| p.to_string_lossy().into_owned()).collect();
            if paths.len() < 2 {
                return Err("Family of Open Fonts needs two or more open fonts that have been saved".into());
            }
            p.insert("target".into(), json!(paths));
        }
        _ => {}
    }
    Ok(Value::Object(p))
}

/// The registry's checks: label, description and group.
fn check_catalogue(app: &mut FontCraftApp) -> HashMap<String, CheckInfo> {
    let Ok(v) = app.session.query("audit.list", &json!({})) else { return HashMap::new() };
    let text = |c: &Value, key: &str| c.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    v.get("checks")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_CHECKS)
                .filter_map(|c| {
                    let id = c.get("id")?.as_str()?.to_string();
                    Some((id, CheckInfo { label: text(c, "label"), description: text(c, "description"), group: text(c, "group") }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn panel_run(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let again = flag(p, "again")?.unwrap_or(false);
    let in_steps = flag(p, "inSteps")?.unwrap_or(false);
    let st = &app.view.font_audit;
    let (profile, target, params) = if again {
        let run = st.run.as_ref().ok_or("nothing has run in the Font Audit panel yet")?;
        (st.profile.clone(), run.target.clone(), run.params.clone())
    } else {
        let profile = profile_param(p)?.unwrap_or_else(|| st.profile.clone());
        let target = target_param(p, &st.target)?;
        let params = run_params(app, &target, profile.as_deref())?;
        (profile, target, params)
    };
    let doc = app.session.active().map(|d| d.id).ok_or("no font is open")?;
    if in_steps && target != "family" {
        let started = app.session.execute("audit.start", &params).map_err(|e| e.to_string())?;
        let st = &mut app.view.font_audit;
        st.progress = Some(Progress {
            run_id: started.get("runId").and_then(Value::as_str).unwrap_or_default().to_string(),
            stage: started.get("stage").and_then(Value::as_str).unwrap_or_default().to_string(),
            done: 0,
            total: started.get("total").and_then(Value::as_u64).unwrap_or(0),
            doc,
            target: target.clone(),
            profile: profile.clone(),
            params,
        });
        st.profile = profile;
        st.target = target;
        set_visible(app, true);
        return Ok(());
    }
    let report = app.session.execute("audit.run", &params).map_err(|e| e.to_string())?;
    list_report(app, report, doc, target, params, profile);
    Ok(())
}

/// Lists `report`, a result document of the panel's `target`, as the panel's run.
fn list_report(app: &mut FontCraftApp, report: Value, doc: u64, target: String, params: Value, profile: Option<String>) {
    let revision = app.session.documents().iter().find(|d| d.id == doc).map_or(0, |d| d.revision);
    let checks = check_catalogue(app);
    // Every target but a family of files is kept by the engine.
    let kept = target != "family";
    let run = Run::read(report, (doc, revision), target.clone(), params, kept, checks);
    let st = &mut app.view.font_audit;
    st.profile = profile;
    st.target = target;
    st.run = Some(Arc::new(run));
    st.progress = None;
    st.preview = None;
    // Ids are numbered afresh by every run.
    st.selected.clear();
    st.current = None;
    set_visible(app, true);
    refresh_profiles(app);
}

fn progress_of(app: &FontCraftApp) -> Result<Progress, String> {
    app.view.font_audit.progress.clone().ok_or_else(|| "no Font Audit run is in progress".into())
}

fn panel_continue(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let budget = match p.get("budget") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|b| (1..=100_000).contains(b)).ok_or("`budget` must be a count from 1 to 100,000")?),
    };
    let pr = progress_of(app)?;
    if app.session.active().map(|d| d.id) != Some(pr.doc) {
        app.view.font_audit.progress = None;
        return Err("the font of the run in progress is no longer the active font: press Run again".into());
    }
    let mut step = Map::new();
    step.insert("runId".into(), json!(pr.run_id));
    if let Some(b) = budget {
        step.insert("budget".into(), json!(b));
    }
    let out = match app.session.execute("audit.step", &Value::Object(step)) {
        Ok(out) => out,
        Err(e) => {
            app.view.font_audit.progress = None;
            return Err(e.to_string());
        }
    };
    if out.get("finished").and_then(Value::as_bool) == Some(true) {
        let report = out.get("report").cloned().unwrap_or(Value::Null);
        list_report(app, report, pr.doc, pr.target, pr.params, pr.profile);
    } else if let Some(p) = app.view.font_audit.progress.as_mut() {
        p.done = out.get("done").and_then(Value::as_u64).unwrap_or(p.done);
        p.total = out.get("total").and_then(Value::as_u64).unwrap_or(p.total);
        p.stage = out.get("stage").and_then(Value::as_str).unwrap_or_default().to_string();
    }
    Ok(())
}

fn panel_cancel(app: &mut FontCraftApp) -> Result<(), String> {
    let pr = progress_of(app)?;
    app.view.font_audit.progress = None;
    let out = app.session.execute("audit.cancel", &json!({"runId": pr.run_id})).map_err(|e| e.to_string())?;
    let mut partial = out.get("partial").cloned().unwrap_or(Value::Null);
    if let Some(o) = partial.as_object_mut() {
        let (done, total) = (out.get("done").cloned().unwrap_or(Value::Null), out.get("total").cloned().unwrap_or(Value::Null));
        o.insert("progress".into(), json!({"done": done, "total": total}));
    }
    list_report(app, partial, pr.doc, pr.target, pr.params, pr.profile);
    Ok(())
}

/// One planned or skipped step of a dry run, as the detail area lists it.
fn step_text(v: &Value) -> String {
    let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default();
    let mut out = String::new();
    for (key, sep) in [("fix", ""), ("command", ": "), ("glyph", " on "), ("master", ", master "), ("reason", ": ")] {
        let t = text(key);
        if !t.is_empty() {
            out.push_str(if out.is_empty() { "" } else { sep });
            out.push_str(t);
        }
    }
    out.chars().take(MAX_TEXT).collect()
}

/// The panel's run, for a command that fixes or ignores its results: refused unless the engine keeps it.
fn kept_run(app: &FontCraftApp) -> Result<Arc<Run>, String> {
    let run = app.view.font_audit.run.clone().ok_or("nothing has run in the Font Audit panel yet")?;
    if !run.kept || run.partial {
        return Err("only results the engine keeps can be fixed or ignored: run the audit on this font".into());
    }
    Ok(run)
}

/// The `results` given, else the selected results that `keep` keeps.
fn results_param(app: &FontCraftApp, run: &Run, p: &Map<String, Value>, keep: fn(&Item) -> bool) -> Result<Vec<String>, String> {
    let ids = match strings(p, "results", MAX_ITEMS)? {
        Some(ids) => ids,
        None => app.view.font_audit.selected.iter().filter(|id| run.item(id).is_some_and(keep)).cloned().collect(),
    };
    if let Some(unknown) = ids.iter().find(|id| run.item(id).is_none()) {
        return Err(format!("the Font Audit panel's run has no result `{}`", quoted(unknown)));
    }
    Ok(ids)
}

fn preview_fix(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let safe_only = flag(p, "safeOnly")?.unwrap_or(false);
    let run = kept_run(app)?;
    let ids = results_param(app, &run, p, |i| i.fixable)?;
    if ids.is_empty() {
        return Err("no selected result has a fix".into());
    }
    let mut fix = Map::new();
    fix.insert("results".into(), json!(ids));
    fix.insert("safeOnly".into(), json!(safe_only));
    fix.insert("dryRun".into(), json!(true));
    if let Some(id) = &run.run_id {
        fix.insert("runId".into(), json!(id));
    }
    let out = app.session.execute("audit.fix", &Value::Object(fix)).map_err(|e| e.to_string())?;
    let texts = |key: &str| -> Vec<String> {
        out.get(key).and_then(Value::as_array).map(|a| a.iter().take(MAX_PREVIEW).map(step_text).collect()).unwrap_or_default()
    };
    app.view.font_audit.preview =
        Some(FixPreview { ids, run_id: run.run_id.clone(), safe_only, planned: texts("planned"), skipped: texts("skipped") });
    Ok(())
}

fn export_report(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let path = string(p, "path")?.filter(|p| !p.trim().is_empty()).ok_or("`path` (the report file) is required")?;
    if path.len() > MAX_PATH {
        return Err(format!("`path` may be at most {MAX_PATH} bytes long"));
    }
    let run = app.view.font_audit.run.clone().ok_or("nothing has run in the Font Audit panel yet")?;
    let mut e = Map::new();
    e.insert("path".into(), json!(path.trim()));
    if let Some(f) = string(p, "format")? {
        e.insert("format".into(), json!(known("format", &f, &EXPORT_FORMATS)?));
    }
    if let Some(s) = string(p, "minSeverity")? {
        e.insert("minSeverity".into(), json!(known("minSeverity", &s, &SEVERITIES)?));
    }
    e.insert("results".into(), run.document());
    app.session.execute("audit.export", &Value::Object(e)).map(|_| ()).map_err(|e| e.to_string())
}

fn filter_results(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let mut f = app.view.font_audit.filter.clone();
    if let Some(s) = string(p, "minSeverity")? {
        f.min_severity = known("minSeverity", &s, &SEVERITIES)?;
    }
    if let Some(groups) = strings(p, "groups", GROUPS.len())? {
        for g in &groups {
            known("group", g, &GROUPS)?;
        }
        f.groups = groups;
    }
    if let Some(t) = string(p, "text")? {
        if t.chars().count() > MAX_TEXT {
            return Err(format!("`text` holds at most {MAX_TEXT} characters"));
        }
        f.text = t;
    }
    if let Some(b) = flag(p, "fixableOnly")? {
        f.fixable_only = b;
    }
    if let Some(b) = flag(p, "currentGlyphOnly")? {
        f.current_glyph_only = b;
    }
    if let Some(b) = flag(p, "showIgnored")? {
        f.show_ignored = b;
    }
    if let Some(g) = string(p, "groupBy")? {
        f.group_by = known("groupBy", &g, &GROUP_BY)?;
    }
    app.view.font_audit.filter = f;
    Ok(())
}

/// Whether the engine's kept run is the panel's: same document and run id, and every one of `ids` in it.
fn kept_matches(app: &mut FontCraftApp, run: &Run, ids: &[String]) -> bool {
    if app.session.active().map(|d| d.id) != Some(run.doc) {
        return false;
    }
    let Some(run_id) = &run.run_id else { return false };
    let Ok(doc) = app.session.query("audit.results", &json!({"includeIgnored": true, "runId": run_id})) else { return false };
    doc.get("runId").and_then(Value::as_str) == Some(run_id.as_str()) && ids.iter().all(|i| run.item(i).is_some())
}

fn again(app: &mut FontCraftApp) -> Result<(), String> {
    panel_run(app, &Map::from_iter([("again".to_string(), json!(true))]))
}

/// `steps` as one engine command: the command itself when there is one step, else a `command.batch`.
fn one_step(label: &str, mut steps: Vec<(&'static str, Value)>) -> Option<Msg> {
    if steps.len() <= 1 {
        return steps.pop().map(|(command, params)| Msg::run(command, params));
    }
    let steps: Vec<Value> = steps.into_iter().map(|(command, params)| json!({"command": command, "params": params})).collect();
    Some(Msg::run("command.batch", json!({"label": label, "steps": steps})))
}

fn fix(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let run = kept_run(app)?;
    let previewed = if flag(p, "preview")?.unwrap_or(false) {
        let preview = app.view.font_audit.preview.clone().filter(|v| v.run_id == run.run_id).ok_or("no fix preview of this run to apply")?;
        Some(preview)
    } else {
        None
    };
    let safe_only = match &previewed {
        Some(v) => v.safe_only,
        None => flag(p, "safeOnly")?.unwrap_or(false),
    };
    let ids = match previewed {
        Some(v) => v.ids,
        None => results_param(app, &run, p, |i| i.fixable && !i.ignored && !i.stale)?,
    };
    if ids.is_empty() {
        return Err("no selected result has a fix".into());
    }
    let mut params = Map::new();
    params.insert("results".into(), json!(ids));
    params.insert("safeOnly".into(), json!(safe_only));
    if kept_matches(app, &run, &ids) {
        // Naming the run makes the fix refuse when another run replaced it in between.
        if let Some(id) = &run.run_id {
            params.insert("runId".into(), json!(id));
        }
    } else {
        again(app)?;
    }
    app.apply_msg(Msg::run("audit.fix", Value::Object(params)))?;
    again(app)
}

fn ignore(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let run = kept_run(app)?;
    let ids = results_param(app, &run, p, |i| !i.ignored)?;
    if ids.is_empty() {
        return Err("no selected result to ignore".into());
    }
    let reason = string(p, "reason")?.unwrap_or_else(|| app.view.font_audit.reason.clone());
    let reason: String = reason.trim().chars().take(MAX_TEXT).collect();
    if !kept_matches(app, &run, &ids) {
        again(app)?;
    }
    let steps = ids
        .into_iter()
        .map(|id| {
            let mut p = Map::new();
            p.insert("result".into(), json!(id));
            if !reason.is_empty() {
                p.insert("reason".into(), json!(reason));
            }
            ("audit.ignoreIssue", Value::Object(p))
        })
        .collect();
    if let Some(msg) = one_step("Ignore Issues", steps) {
        app.apply_msg(msg)?;
    }
    again(app)
}

fn unignore(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let run = app.view.font_audit.run.clone().ok_or("nothing has run in the Font Audit panel yet")?;
    if !run.kept {
        return Err("results of font files are ignored in the files themselves".into());
    }
    let ids = results_param(app, &run, p, |i| i.ignored)?;
    let steps: Vec<(&'static str, Value)> = ids
        .iter()
        .filter_map(|id| run.item(id))
        .map(|it| {
            let mut p = Map::new();
            p.insert("check".into(), json!(it.check));
            if let Some(g) = &it.glyph {
                p.insert("glyph".into(), json!(g));
            }
            if let Some(m) = &it.master {
                p.insert("master".into(), json!(m));
            }
            ("audit.unignoreIssue", Value::Object(p))
        })
        .collect();
    let msg = one_step("Stop Ignoring Issues", steps).ok_or("no selected result is ignored")?;
    app.apply_msg(msg)?;
    again(app)
}

fn set_view(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let selected = strings(p, "selected", MAX_ITEMS)?;
    let current = match p.get("current") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(_) => return Err("`current` must be a result id or null".into()),
    };
    let group = match p.get("groupOpen") {
        None => None,
        Some(v) => {
            let key = v.get("group").and_then(Value::as_str).filter(|k| k.len() <= MAX_PATH).ok_or("`groupOpen` needs `group`, a group key")?;
            let open = v.get("open").and_then(Value::as_bool).ok_or("`groupOpen` needs `open`, true or false")?;
            Some((key.to_string(), open))
        }
    };
    let reason = string(p, "reason")?;
    if reason.as_ref().is_some_and(|r| r.chars().count() > MAX_TEXT) {
        return Err(format!("`reason` holds at most {MAX_TEXT} characters"));
    }
    let export_open = flag(p, "exportOpen")?;
    let export_path = string(p, "exportPath")?;
    if export_path.as_ref().is_some_and(|e| e.len() > MAX_PATH) {
        return Err(format!("`exportPath` may be at most {MAX_PATH} bytes long"));
    }
    let export_format = string(p, "exportFormat")?.map(|f| known("exportFormat", &f, &EXPORT_FORMATS)).transpose()?;
    let st = &mut app.view.font_audit;
    if let Some(ids) = selected {
        st.selected = ids;
    }
    if let Some(c) = current {
        st.current = c;
    }
    if let Some((key, open)) = group {
        st.collapsed.retain(|k| *k != key);
        if !open {
            st.collapsed.push(key);
        }
    }
    if let Some(r) = reason {
        st.reason = r;
    }
    if let Some(v) = export_open {
        st.export_open = v;
    }
    if let Some(v) = export_path {
        st.export_path = v;
    }
    if let Some(v) = export_format {
        st.export_format = v;
    }
    Ok(())
}

// ── Inspect ─────────────────────────────────────────────────────────────────────────────────────

fn item_json(item: &Item) -> Value {
    json!({
        "id": item.id,
        "check": item.check,
        "severity": item.severity.name(),
        "glyph": item.glyph,
        "master": item.master,
        "message": item.message,
        "fixable": item.fixable,
        "ignored": item.ignored,
        "stale": item.stale,
    })
}

/// The panel as `ui.inspect` reports it.
pub fn panel_json(app: &FontCraftApp) -> Value {
    let st = &app.view.font_audit;
    let glyph = current_glyph(app);
    let (run, listed, groups, rows) = match &st.run {
        Some(run) => {
            let l = run::listing(run, &st.filter, glyph.as_deref());
            let rows: Vec<Value> = l.order().take(MAX_RETURNED).filter_map(|i| run.items.get(i)).map(item_json).collect();
            let about = json!({
                "runId": run.run_id,
                "partial": run.partial,
                "profile": run.profile,
                "target": run.target,
                "summary": run.summary,
                "results": run.items.len(),
                "kept": run.kept,
                "warnings": run.warnings,
            });
            (about, l.len(), l.groups.len(), rows)
        }
        None => (Value::Null, 0, 0, Vec::new()),
    };
    json!({
        "open": shows(app),
        "profile": st.profile,
        "target": st.target,
        "filter": st.filter.to_json(),
        "run": run,
        "progress": st.progress.as_ref().map(|p| json!({"runId": p.run_id, "stage": p.stage, "done": p.done, "total": p.total})),
        "preview": st.preview.as_ref().map(|p| json!({"results": p.ids, "safeOnly": p.safe_only, "planned": p.planned, "skipped": p.skipped})),
        "listed": listed,
        "groups": groups,
        "more": listed > rows.len(),
        "rows": rows,
        "selected": st.selected,
        "current": st.current,
    })
}

/// `ui.inspect`'s `fontAudit` key, and `auditProfile` while the dialog is open.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    state.insert("fontAudit".into(), panel_json(app));
    if let Some(d) = &app.view.font_audit.draft {
        state.insert("auditProfile".into(), d.to_json());
    }
}

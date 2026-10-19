//! The Font Audit panel (`window.fontAudit`, M12.64): runs audits and lists their results
//! (`docs/spec/21-qa.md` § 7).
//!
//! Rows: `window.fontAudit`, `audit.showPanel`, `audit.panel.run`, `audit.panel.continue`,
//! `audit.panel.cancel`, `audit.panel.previewFix`, `audit.panel.exportReport`, `audit.revealIssue`,
//! `audit.nextIssue`, `audit.previousIssue`, `audit.filterResults`,
//! `fontInfo.styleLinking.recommendedScheme` (listed as the results of the style-linking checks).
//!
//! | Control | Command |
//! |---|---|
//! | profile and target pop-ups | `audit.showPanel {profile}`, `{target}` |
//! | Run | `audit.panel.run {inSteps}`, then `audit.panel.continue` once a frame while the progress bar shows |
//! | Cancel (beside the progress bar) | `audit.panel.cancel` |
//! | Preview Fix, and Apply under the preview | `audit.panel.previewFix` (a dry run of `audit.fix`), then the Fix action on the previewed results |
//! | severity choices, group menu, search field, Fixable Only, Current Glyph Only, Show Ignored, group-by pop-up | `audit.filterResults` |
//! | double-click on a result, or Return | `audit.revealIssue` |
//! | Previous and Next | `audit.previousIssue`, `audit.nextIssue` |
//! | Fix | `audit.fix {results, safeOnly: false}` on the selected results, then `audit.panel.run {again}` |
//! | Fix All Safe | `audit.fix {results, safeOnly: true}` on every listed result with a safe fix, then a re-run |
//! | Ignore (with the reason field) | `audit.ignoreIssue` per selected result (several: one `command.batch`), then a re-run |
//! | Stop Ignoring | `audit.unignoreIssue` per selected ignored result, then a re-run |
//! | gear menu | `audit.editProfile`, `audit.clearCache`, `audit.toggleLiveAudit`; Export Report… shows the export row |
//! | export row: file, Choose…, format, Export | Choose… asks the save chooser for the file (view state; without a chooser the file is typed); Export runs `audit.panel.exportReport` |
//!
//! The panel keeps the result document of its last run ([`Run`]); the engine keeps the last run
//! of each open document too, and its result ids are what `audit.fix` and `audit.ignoreIssue`
//! take. Live audit and agents can replace the engine's run, so before a fix or an ignore the
//! panel compares the two by their run ids and, when they differ, runs its own parameters again
//! first: on an unchanged font that gives the same ids. A fix of the kept run names its run id,
//! so the engine refuses it when another run replaced that one in between. Selecting rows, collapsing groups and typing the
//! reason are view state; everything else goes through the commands above.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::canvas::audit_markers;
use crate::panels::{Pending, widgets};
use crate::shell::audit::{Marker, Severity};
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

/// The run targets: the `target` of `audit.showPanel` and `audit.panel.run`, and the label.
pub const TARGETS: [(&str, &str); 5] = [
    ("font", "This Font"),
    ("selectedGlyphs", "Selected Glyphs"),
    ("currentGlyph", "Current Glyph"),
    ("export", "Export of This Font"),
    ("family", "Family of Open Fonts"),
];
/// The list groupings: the `groupBy` of `audit.filterResults`, and the label.
pub const GROUP_BY: [(&str, &str); 3] = [("check", "Check"), ("glyph", "Glyph"), ("severity", "Severity")];
/// The severity choices: the `minSeverity` of `audit.filterResults`, and the label.
pub const SEVERITIES: [(&str, &str); 3] = [("info", "All"), ("warn", "Warnings and Failures"), ("fail", "Failures")];
/// The check groups of the registry, and the label the group menu shows.
pub const GROUPS: [(&str, &str); 18] = [
    ("outlines", "Outlines"),
    ("components", "Components"),
    ("anchors", "Anchors"),
    ("glyphset", "Glyph Set"),
    ("metrics", "Metrics"),
    ("names", "Names"),
    ("tables", "Tables"),
    ("layout", "Layout"),
    ("variations", "Variations"),
    ("hinting", "Hinting"),
    ("kerning", "Kerning"),
    ("family", "Family"),
    ("files", "Files"),
    ("sources", "Sources"),
    ("color", "Color"),
    ("vendor", "Vendor"),
    ("cid", "CID"),
    ("plugin", "Modules"),
];
/// The built-in profiles and their labels.
const BUILT_IN: [(&str, &str); 4] = [("outlines", "Outlines"), ("opentype", "OpenType"), ("universal", "Universal"), ("strict", "Strict")];

/// Longest search text `audit.filterResults` takes, in characters.
const MAX_TEXT: usize = 256;
/// Longest profile id a command takes, in bytes.
const MAX_PROFILE_ID: usize = 64;
/// Most results the panel keeps from one run (the engine lists at most this many per document).
const MAX_ITEMS: usize = 100_000;
/// Most checks the panel reads from `audit.list`.
const MAX_CHECKS: usize = 4_096;
/// Most profiles the profile pop-up lists.
const MAX_PROFILES: usize = 256;
/// Most fonts in a family run, and most fonts one result names.
const MAX_FONTS: usize = 256;
/// Most run warnings the panel shows.
const MAX_WARNINGS: usize = 64;
/// Most rows a command's reply lists; `listed` counts them all.
const MAX_RETURNED: usize = 100;
/// Most characters of a result's data the detail area shows.
const MAX_DATA_CHARS: usize = 2_000;
/// Smallest side of the area `audit.revealIssue` zooms to, in font units.
const MIN_REVEAL_SPAN: f64 = 200.0;
/// The area revealed around a result's bounds, as a multiple of their size.
const REVEAL_MARGIN: f64 = 1.5;
/// List rows the panel shows before it scrolls.
const VISIBLE_ROWS: f32 = 16.0;
/// Width of the place and master columns, in list column units.
const PLACE_UNITS: f32 = 2.5;
const MASTER_UNITS: f32 = 2.0;
/// Default width of the panel, in list column units.
const PANEL_UNITS: f32 = 12.0;

pub const COMMANDS: &[UiCommand] = &[
    (
        "audit.showPanel",
        "Font Audit",
        &["Window"],
        "",
        "{visible?: bool (omitted: show it when hidden, hide it when shown), profile?: id | null (the profile Run uses; null: the document's default profile), target?: \"font\" | \"selectedGlyphs\" | \"currentGlyph\" | \"export\" | \"family\"} shows or hides the Font Audit panel and sets the pop-ups Run reads → the panel as audit.panel.run returns it",
    ),
    (
        "audit.panel.run",
        "Run Font Audit",
        &[],
        "",
        "{profile?: id | null, target?: \"font\" (the open document) | \"selectedGlyphs\" (the glyphs of the Edit View tab, else the Font View selection) | \"currentGlyph\" | \"export\" (the binary and family checks on the document's compiled instances) | \"family\" (the saved files of every open document, two or more), again?: bool (run the last run's parameters once more), inSteps?: bool (start the run with audit.start and let the panel continue it frame by frame, with a progress bar and Cancel; not for a family of files)} runs audit.run with the panel's profile and target, ignored results included, and lists its result document in the panel, which opens. A failed run keeps the list as it was → {open, profile, target, filter, run: {runId, profile, target, summary, results, kept, partial, warnings}, progress: {runId, stage, done, total} | null, preview, listed, groups, rows: [{id, check, severity, glyph, master, message, fixable, ignored, stale}] (the first 100 listed), more, selected, current}",
    ),
    (
        "audit.panel.continue",
        "Continue Font Audit",
        &[],
        "",
        "{budget?: count (layers, 1 to 100,000; default 256)} does the next step (audit.step) of the run the panel started in steps; the panel runs it every frame while its progress bar shows. The last step lists the run's results. Refused when no run is in progress; a run whose document is no longer active is dropped → the panel as audit.panel.run returns it",
    ),
    (
        "audit.panel.cancel",
        "Cancel Font Audit",
        &[],
        "",
        "{} stops the run the panel started in steps (audit.cancel) and lists what it had found, marked partial: the font and the engine's kept run do not change, and Fix and Ignore are off for partial results. Refused when no run is in progress → the panel as audit.panel.run returns it",
    ),
    (
        "audit.panel.previewFix",
        "Preview Audit Fix",
        &[],
        "",
        "{results?: [result id] (default the selected results that have a fix), safeOnly?: bool = false} asks audit.fix for a dry run of the fixes of those results of the panel's run and shows the edits it would make in the detail area, where Apply runs them. Nothing changes. Refused for a run the engine no longer keeps → {preview: {results, safeOnly, planned: [text], skipped: [text]}}",
    ),
    (
        "audit.panel.exportReport",
        "Export Font Audit Report",
        &[],
        "",
        "{path: path, format?: \"json\" | \"markdown\" | \"html\" | \"text\" (default from the extension of path), minSeverity?: \"info\" | \"warn\" | \"fail\"} writes the run the panel lists, partial runs and families of files included, with audit.export → audit.export's reply",
    ),
    (
        "audit.revealIssue",
        "Reveal Issue",
        &[],
        "",
        "{result: id (from the panel's last run, else from the document's last audit.run), runId?: id (refused unless the panel lists that run)} a glyph result: opens the glyph in Edit View (the tab already editing it, else the tab holding exactly it, else a new tab) on the result's layer, selects the nodes, segments and components it reports and zooms to them; a result naming a Font Info field (sourceField): opens Font Info on its tab; a family result: makes the first open document among its fonts active. A result that names none of these reveals nothing and says why. The result becomes the panel's current and selected one → {result, runId, revealed: \"glyph\" | \"fontInfo\" | \"fonts\" | null, glyph?, layer?, selection?, tab?, field?, documents?, reason?}",
    ),
    (
        "audit.nextIssue",
        "Next Issue",
        &[],
        "",
        "{scope?: \"font\" (default) | \"glyph\" (the results of the edited glyph), runId?: id (refused unless the panel lists that run)} reveals the listed result after the current one, as audit.revealIssue does, wrapping after the last; the panel's filters decide what is listed. Refused when nothing is listed → audit.revealIssue's reply",
    ),
    (
        "audit.previousIssue",
        "Previous Issue",
        &[],
        "",
        "{scope?: \"font\" (default) | \"glyph\" (the results of the edited glyph), runId?: id (refused unless the panel lists that run)} reveals the listed result before the current one, as audit.revealIssue does, wrapping before the first; the panel's filters decide what is listed. Refused when nothing is listed → audit.revealIssue's reply",
    ),
    (
        "audit.filterResults",
        "Filter Audit Results",
        &[],
        "",
        "{minSeverity?: \"info\" | \"warn\" | \"fail\", groups?: [group] (empty: every group), text?: string (matched against the check id, glyph, table and message, ignoring case), fixableOnly?: bool, currentGlyphOnly?: bool, showIgnored?: bool, groupBy?: \"check\" | \"glyph\" | \"severity\"} sets which results the Font Audit panel lists and how it groups them; the keys left out keep their value. The results themselves do not change → the panel as audit.panel.run returns it",
    ),
];

// ── State ───────────────────────────────────────────────────────────────────────────────────────

/// A value shared between frames and compared by identity.
pub struct Shared<T>(pub Arc<T>);

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shared")
    }
}

impl<T> std::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// What the panel lists (`audit.filterResults`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Filter {
    /// A [`SEVERITIES`] id.
    pub min_severity: String,
    /// [`GROUPS`] ids; empty: every group.
    pub groups: Vec<String>,
    pub text: String,
    pub fixable_only: bool,
    pub current_glyph_only: bool,
    /// A [`GROUP_BY`] id.
    pub group_by: String,
    pub show_ignored: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            min_severity: "info".into(),
            groups: Vec::new(),
            text: String::new(),
            fixable_only: false,
            current_glyph_only: false,
            group_by: "check".into(),
            show_ignored: false,
        }
    }
}

/// What the registry says about a check.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckInfo {
    pub label: String,
    pub description: String,
    pub group: String,
}

/// One result of a run.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub check: String,
    pub severity: Severity,
    pub message: String,
    pub glyph: Option<String>,
    pub master: Option<String>,
    pub layer: Option<String>,
    /// The compiled table, else the instance, a font-level result names.
    pub table: Option<String>,
    pub source_field: Option<String>,
    /// The font id the result is about (`location.font`).
    pub font: Option<String>,
    /// The font ids of a family result (`location.fonts`).
    pub fonts: Vec<String>,
    pub fixable: bool,
    pub safe: bool,
    pub ignored: bool,
    pub stale: bool,
    /// Lower-case check id, glyph, table and message, for the search field.
    haystack: String,
    /// The result as the result document gives it.
    pub raw: Value,
}

impl Item {
    /// A result of the result document (`docs/spec/21-qa.md` § 4); `None` without an id.
    fn from_result(raw: Value) -> Option<Self> {
        let text = |ptr: &str| raw.pointer(ptr).and_then(Value::as_str).map(str::to_string);
        let flag = |ptr: &str| raw.pointer(ptr).and_then(Value::as_bool).unwrap_or(false);
        let id = text("/id")?;
        let check = text("/check").unwrap_or_default();
        let severity = match raw.get("severity").and_then(Value::as_str) {
            Some("fail") => Severity::Fail,
            Some("warn") => Severity::Warn,
            _ => Severity::Info,
        };
        let message = text("/message").unwrap_or_default();
        let glyph = text("/location/glyph");
        let table = text("/location/table").or_else(|| text("/location/instance"));
        let fonts = raw
            .pointer("/location/fonts")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(MAX_FONTS).filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let haystack = [check.as_str(), glyph.as_deref().unwrap_or(""), table.as_deref().unwrap_or(""), message.as_str()].join(" ").to_lowercase();
        Some(Self {
            id,
            check,
            severity,
            message,
            glyph,
            master: text("/location/master"),
            layer: text("/location/layer"),
            table,
            source_field: text("/location/sourceField"),
            font: text("/location/font"),
            fonts,
            fixable: flag("/fix/available"),
            safe: flag("/fix/safe"),
            ignored: flag("/ignored"),
            stale: flag("/stale"),
            haystack,
            raw,
        })
    }
}

/// The result document of the panel's last run, and what it was run for.
#[derive(Debug)]
pub struct Run {
    /// The document that was active (for a family, the one the panel ran from).
    pub doc: u64,
    /// Its revision when the run finished.
    pub revision: u64,
    /// A [`TARGETS`] id.
    pub target: String,
    /// The `audit.run` parameters, for a run again.
    pub params: Value,
    /// The engine keeps this run for the document, so its ids work in `audit.fix` and
    /// `audit.ignoreIssue` (every target but a family of files).
    pub kept: bool,
    pub profile: String,
    pub summary: Map<String, Value>,
    pub items: Vec<Item>,
    pub checks: HashMap<String, CheckInfo>,
    /// `(font id, path)` of each font the run examined.
    pub fonts: Vec<(String, Option<String>)>,
    pub warnings: Vec<String>,
    /// The run's id (`runId` of the result document).
    pub run_id: Option<String>,
    /// The results of a cancelled run: what it had examined.
    pub partial: bool,
    /// The result document without its results, for Export Report.
    pub head: Value,
}

impl Run {
    #[allow(clippy::too_many_arguments)]
    fn read(mut report: Value, doc: u64, revision: u64, target: String, params: Value, kept: bool, checks: HashMap<String, CheckInfo>) -> Self {
        let items = match report.get_mut("results").map(Value::take) {
            Some(Value::Array(list)) => list.into_iter().take(MAX_ITEMS).filter_map(Item::from_result).collect(),
            _ => Vec::new(),
        };
        let profile = report.get("profile").and_then(Value::as_str).unwrap_or_default().to_string();
        let summary = report.get("summary").and_then(Value::as_object).cloned().unwrap_or_default();
        let fonts = report
            .pointer("/target/fonts")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(MAX_FONTS)
                    .filter_map(|f| Some((f.get("id")?.as_str()?.to_string(), f.get("path").and_then(Value::as_str).map(str::to_string))))
                    .collect()
            })
            .unwrap_or_default();
        let warnings = report
            .get("warnings")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(MAX_WARNINGS).filter_map(|w| w.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let run_id = report.get("runId").and_then(Value::as_str).map(str::to_string);
        let partial = report.get("partial").and_then(Value::as_bool).unwrap_or(false);
        Self { doc, revision, target, params, kept, profile, summary, items, checks, fonts, warnings, run_id, partial, head: report }
    }

    /// The result document again, for Export Report.
    fn document(&self) -> Value {
        let mut doc = self.head.clone();
        if let Some(o) = doc.as_object_mut() {
            o.insert("results".into(), Value::Array(self.items.iter().map(|i| i.raw.clone()).collect()));
        }
        doc
    }

    fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    fn group_of(&self, item: &Item) -> &str {
        self.checks.get(&item.check).map_or("", |c| c.group.as_str())
    }
}

/// A row of the result list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A group's heading: its key (prefixed by the grouping), title and result count.
    Header { key: String, title: String, count: usize },
    /// A result, by its index in [`Run::items`].
    Result(usize),
}

/// The listed results in list order, and the rows that show them (collapsed groups show their
/// heading only).
#[derive(Debug, Default, PartialEq)]
pub struct Listing {
    pub order: Vec<usize>,
    pub rows: Vec<Row>,
    pub groups: usize,
}

/// What a [`Listing`] was made from.
#[derive(Clone, Debug, PartialEq)]
pub struct ListingKey {
    run: usize,
    filter: Filter,
    glyph: Option<String>,
    collapsed: Vec<String>,
}

/// The panel's state, kept in `UiState::panels`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FontAuditState {
    pub open: bool,
    /// The profile Run uses; `None`: the document's default profile.
    pub profile: Option<String>,
    /// A [`TARGETS`] id.
    pub target: String,
    pub filter: Filter,
    #[serde(skip)]
    pub run: Option<Shared<Run>>,
    /// Ids of the selected results.
    #[serde(skip)]
    pub selected: Vec<String>,
    /// The result Previous and Next step from.
    #[serde(skip)]
    pub current: Option<String>,
    /// The reason Ignore stores.
    #[serde(skip)]
    pub reason: String,
    /// Keys of the collapsed groups.
    #[serde(skip)]
    pub collapsed: Vec<String>,
    /// `(id, label)` of the profiles the pop-up lists.
    #[serde(skip)]
    pub profiles: Vec<(String, String)>,
    /// The profile Run uses without a choice: the document's default, else the application's.
    #[serde(skip)]
    pub default_profile: String,
    #[serde(skip)]
    pub cache: Option<(ListingKey, Shared<Listing>)>,
    /// The Audit Profile dialog while it is open.
    #[serde(skip)]
    pub dialog: Option<crate::dialogs::audit_profile::ProfileDraft>,
    /// The run the panel started in steps, while it runs.
    #[serde(skip)]
    pub progress: Option<Progress>,
    /// The dry run of the last Preview Fix.
    #[serde(skip)]
    pub preview: Option<Preview>,
    /// The export row under the header (view state): shown, its path and its format.
    #[serde(skip)]
    pub export_open: bool,
    #[serde(skip)]
    pub export_path: String,
    #[serde(skip)]
    pub export_format: String,
}

/// A run in steps (`audit.start`) the panel continues every frame.
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
pub struct Preview {
    pub ids: Vec<String>,
    pub run_id: Option<String>,
    pub safe_only: bool,
    pub planned: Vec<String>,
    pub skipped: Vec<String>,
}

impl Default for FontAuditState {
    fn default() -> Self {
        Self {
            open: false,
            profile: None,
            target: "font".into(),
            filter: Filter::default(),
            run: None,
            selected: Vec::new(),
            current: None,
            reason: String::new(),
            collapsed: Vec::new(),
            profiles: Vec::new(),
            default_profile: "universal".into(),
            cache: None,
            dialog: None,
            progress: None,
            preview: None,
            export_open: false,
            export_path: String::new(),
            export_format: "html".into(),
        }
    }
}

// ── Listing ─────────────────────────────────────────────────────────────────────────────────────

fn rank(s: Severity) -> u8 {
    match s {
        Severity::Info => 0,
        Severity::Warn => 1,
        Severity::Fail => 2,
    }
}

pub fn severity_name(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Fail => "fail",
    }
}

fn severity_rank(name: &str) -> u8 {
    match name {
        "fail" => 2,
        "warn" => 1,
        _ => 0,
    }
}

pub fn severity_colour(t: &Tokens, s: Severity) -> Color32 {
    match s {
        Severity::Fail => t.audit_fail,
        Severity::Warn => t.audit_warn,
        Severity::Info => t.audit_info,
    }
}

/// Whether `filter` lists `item`; `glyph` is the glyph Current Glyph Only keeps.
fn keeps(run: &Run, filter: &Filter, glyph: Option<&str>, needle: &str, item: &Item) -> bool {
    rank(item.severity) >= severity_rank(&filter.min_severity)
        && (filter.show_ignored || !item.ignored)
        && (!filter.fixable_only || item.fixable)
        && (!filter.current_glyph_only || (glyph.is_some() && item.glyph.as_deref() == glyph))
        && (filter.groups.is_empty() || filter.groups.iter().any(|g| g == run.group_of(item)))
        && (needle.is_empty() || item.haystack.contains(needle))
}

/// The results `filter` lists, grouped as it says: groups in the order their first result
/// comes (severity: failures, warnings, notes), results in run order within a group.
pub fn listing(run: &Run, filter: &Filter, glyph: Option<&str>, collapsed: &[String]) -> Listing {
    let needle = filter.text.trim().to_lowercase();
    let by = filter.group_by.as_str();
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, item) in run.items.iter().enumerate() {
        if !keeps(run, filter, glyph, &needle, item) {
            continue;
        }
        let key = match by {
            "glyph" => item.glyph.clone().unwrap_or_default(),
            "severity" => severity_name(item.severity).to_string(),
            _ => item.check.clone(),
        };
        match index.get(&key).and_then(|&g| groups.get_mut(g)) {
            Some((_, list)) => list.push(i),
            None => {
                index.insert(key.clone(), groups.len());
                groups.push((key, vec![i]));
            }
        }
    }
    if by == "severity" {
        groups.sort_by_key(|(k, _)| std::cmp::Reverse(severity_rank(k)));
    }
    let mut out = Listing { groups: groups.len(), ..Listing::default() };
    for (key, list) in groups {
        let title = match by {
            "glyph" if key.is_empty() => "Font-wide results".to_string(),
            "glyph" => key.clone(),
            "severity" => match key.as_str() {
                "fail" => "Failures".to_string(),
                "warn" => "Warnings".to_string(),
                _ => "Notes".to_string(),
            },
            _ => run.checks.get(&key).map_or_else(|| key.clone(), |c| if c.description.is_empty() { c.label.clone() } else { c.description.clone() }),
        };
        let key = format!("{by}:{key}");
        let folded = collapsed.contains(&key);
        out.rows.push(Row::Header { key, title, count: list.len() });
        if !folded {
            out.rows.extend(list.iter().map(|&i| Row::Result(i)));
        }
        out.order.extend(list);
    }
    out
}

/// The listing the panel draws, made again only when the run, the filter, the glyph or the
/// collapsed groups changed.
fn cached_listing(st: &mut FontAuditState, glyph: Option<&str>) -> Option<Shared<Listing>> {
    let run = st.run.clone()?;
    let key = ListingKey {
        run: Arc::as_ptr(&run.0) as usize,
        filter: st.filter.clone(),
        glyph: glyph.map(str::to_string),
        collapsed: st.collapsed.clone(),
    };
    if let Some((k, l)) = &st.cache
        && *k == key
    {
        return Some(l.clone());
    }
    let made = Shared::new(listing(&run, &st.filter, glyph, &st.collapsed));
    st.cache = Some((key, made.clone()));
    Some(made)
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "audit.showPanel" => show_panel(app, args),
        "audit.panel.run" => panel_run(app, args),
        "audit.revealIssue" => {
            let result = args.string("result")?.ok_or_else(|| args.err("`result` (a result id of the last run) is required"))?;
            reveal(app, args, result)
        }
        "audit.nextIssue" => step(app, args, true),
        "audit.previousIssue" => step(app, args, false),
        "audit.filterResults" => filter_results(app, args),
        "audit.panel.continue" => panel_continue(app, args),
        "audit.panel.cancel" => panel_cancel(app, args),
        "audit.panel.previewFix" => preview_fix(app, args),
        "audit.panel.exportReport" => export_report(app, args),
        other => Err(format!("unknown command `{other}`")),
    }
}

/// At most 64 characters of untrusted text, for a message.
fn clip(s: &str) -> String {
    s.chars().take(64).collect()
}

fn known<'a>(args: &Args, key: &str, value: &'a str, list: &[(&str, &str)]) -> Result<&'a str, String> {
    if list.iter().any(|(id, _)| *id == value) {
        Ok(value)
    } else {
        let ids: Vec<&str> = list.iter().map(|(id, _)| *id).collect();
        Err(args.err(format!("unknown {key} `{}`: use {}", clip(value), ids.join(", "))))
    }
}

/// The `profile` given (`Some(None)`: null, the default profile), or `None` when absent.
fn profile_param(args: &Args) -> Result<Option<Option<String>>, String> {
    match args.given("profile") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if !s.is_empty() && s.len() <= MAX_PROFILE_ID => Ok(Some(Some(s.clone()))),
        Some(_) => Err(args.err(format!("`profile` must be a profile id of 1 to {MAX_PROFILE_ID} bytes, or null"))),
    }
}

fn target_param(args: &Args, current: &str) -> Result<String, String> {
    match args.string("target")? {
        None => Ok(current.to_string()),
        Some(t) => known(args, "target", t, &TARGETS).map(str::to_string),
    }
}

fn show_panel(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let st = &app.state.panels.font_audit;
    let visible = match args.get("visible") {
        None => !st.open,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(args.err("`visible` must be true or false")),
    };
    let profile = profile_param(args)?.unwrap_or_else(|| st.profile.clone());
    let target = target_param(args, &st.target)?;
    if visible {
        refresh_profiles(app);
    }
    let st = &mut app.state.panels.font_audit;
    st.open = visible;
    st.profile = profile;
    st.target = target;
    Ok(panel_json(app))
}

/// Reads the profiles the pop-up lists: the built-in ones, the document's and the user's, and
/// the effective default.
pub fn refresh_profiles(app: &mut FontcraftApp) {
    let mut list: Vec<(String, String)> = BUILT_IN.iter().map(|(id, label)| (id.to_string(), label.to_string())).collect();
    let mut default = None;
    if let Ok(v) = app.session.query("audit.profile.list", &json!({}))
        && let Some(stored) = v.get("profiles").and_then(Value::as_array)
    {
        for p in stored.iter().take(MAX_PROFILES + BUILT_IN.len()) {
            let Some(id) = p.get("id").and_then(Value::as_str) else { continue };
            if p.get("default").and_then(Value::as_bool) == Some(true) {
                default = Some(id.to_string());
            }
            if list.iter().any(|(i, _)| i == id) {
                continue;
            }
            let label = p.get("label").and_then(Value::as_str).filter(|l| !l.is_empty()).unwrap_or(id);
            list.push((id.to_string(), label.to_string()));
        }
    }
    let st = &mut app.state.panels.font_audit;
    for p in st.profile.iter().chain(default.iter()) {
        if !list.iter().any(|(i, _)| i == p) {
            list.push((p.clone(), p.clone()));
        }
    }
    st.profiles = list;
    if let Some(d) = default {
        st.default_profile = d;
    }
}

/// The glyphs Selected Glyphs audits: those of the Edit View tab, else the Font View selection.
fn selected_glyphs(app: &FontcraftApp) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match app.state.tab() {
        Some(tab) => {
            for g in tab.shown_glyphs() {
                if !out.iter().any(|o| o == g) {
                    out.push(g.to_string());
                }
                if out.len() >= fontcraft_engine::model::MAX_GLYPHS {
                    break;
                }
            }
        }
        None => out.extend(app.state.selected_glyphs.iter().take(fontcraft_engine::model::MAX_GLYPHS).cloned()),
    }
    out
}

/// The `audit.run` parameters for `target` and `profile`.
fn run_params(app: &FontcraftApp, args: &Args, target: &str, profile: Option<&str>) -> Result<Value, String> {
    let mut p = Map::new();
    if let Some(profile) = profile {
        p.insert("profile".into(), json!(profile));
    }
    p.insert("includeIgnored".into(), json!(true));
    match target {
        "selectedGlyphs" => {
            let glyphs = selected_glyphs(app);
            if glyphs.is_empty() {
                return Err(args.err("no glyph is selected"));
            }
            p.insert("glyphs".into(), json!(glyphs));
        }
        "currentGlyph" => {
            let glyph = crate::panels::current_glyph(app).ok_or_else(|| args.err("no glyph is being edited or selected"))?;
            p.insert("glyphs".into(), json!([glyph]));
        }
        "export" => {
            p.insert("scope".into(), json!(["binary", "family"]));
        }
        "family" => {
            let paths: Vec<String> =
                app.session.documents().iter().filter_map(|d| d.path.as_ref()).take(MAX_FONTS).map(|p| p.to_string_lossy().into_owned()).collect();
            if paths.len() < 2 {
                return Err(args.err("Family of Open Fonts needs two or more open fonts that have been saved"));
            }
            p.insert("target".into(), json!(paths));
        }
        _ => {}
    }
    Ok(Value::Object(p))
}

/// The registry's checks: label, description and group.
fn check_catalogue(app: &mut FontcraftApp) -> HashMap<String, CheckInfo> {
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

fn panel_run(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let again = args.flag("again")?.unwrap_or(false);
    let in_steps = args.flag("inSteps")?.unwrap_or(false);
    let st = &app.state.panels.font_audit;
    let (profile, target, params) = if again {
        let run = st.run.as_ref().ok_or_else(|| args.err("nothing has run in the Font Audit panel yet"))?;
        (st.profile.clone(), run.target.clone(), run.params.clone())
    } else {
        let profile = profile_param(args)?.unwrap_or_else(|| st.profile.clone());
        let target = target_param(args, &st.target)?;
        let params = run_params(app, args, &target, profile.as_deref())?;
        (profile, target, params)
    };
    let doc = app.session.active().map(|d| d.id).ok_or_else(|| args.err("no font is open"))?;
    if in_steps && target != "family" {
        let started = app.session.execute("audit.start", &params).map_err(|e| args.err(e))?;
        let st = &mut app.state.panels.font_audit;
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
        st.open = true;
        return Ok(panel_json(app));
    }
    let report = app.session.execute("audit.run", &params).map_err(|e| args.err(e))?;
    list_report(app, report, doc, target, params, profile);
    Ok(panel_json(app))
}

/// Whether the engine keeps the runs of `target` (every target but a family of files).
fn target_kept(target: &str) -> bool {
    target != "family"
}

/// Lists `report`, a result document of the panel's `target`, as the panel's run.
fn list_report(app: &mut FontcraftApp, report: Value, doc: u64, target: String, params: Value, profile: Option<String>) {
    let revision = app.session.documents().iter().find(|d| d.id == doc).map_or(0, |d| d.revision);
    let checks = check_catalogue(app);
    let partial = report.get("partial").and_then(Value::as_bool).unwrap_or(false);
    let kept = target_kept(&target) && !partial;
    let run = Run::read(report, doc, revision, target.clone(), params, kept, checks);
    let st = &mut app.state.panels.font_audit;
    st.profile = profile;
    st.target = target;
    st.run = Some(Shared::new(run));
    st.progress = None;
    st.preview = None;
    // Ids are numbered afresh by every run.
    st.selected.clear();
    st.current = None;
    st.open = true;
    refresh_profiles(app);
}

/// The run in steps the panel continues, or why there is none.
fn progress_of(app: &FontcraftApp, args: &Args) -> Result<Progress, String> {
    app.state.panels.font_audit.progress.clone().ok_or_else(|| args.err("no Font Audit run is in progress"))
}

fn panel_continue(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let budget = args.index("budget")?;
    let pr = progress_of(app, args)?;
    if app.session.active().map(|d| d.id) != Some(pr.doc) {
        app.state.panels.font_audit.progress = None;
        return Err(args.err("the font of the run in progress is no longer the active font: press Run again"));
    }
    let mut p = Map::new();
    p.insert("runId".into(), json!(pr.run_id));
    if let Some(b) = budget {
        p.insert("budget".into(), json!(b));
    }
    let out = match app.session.execute("audit.step", &Value::Object(p)) {
        Ok(out) => out,
        Err(e) => {
            app.state.panels.font_audit.progress = None;
            return Err(args.err(e));
        }
    };
    if out.get("finished").and_then(Value::as_bool) == Some(true) {
        let report = out.get("report").cloned().unwrap_or(Value::Null);
        list_report(app, report, pr.doc, pr.target, pr.params, pr.profile);
    } else if let Some(p) = app.state.panels.font_audit.progress.as_mut() {
        p.done = out.get("done").and_then(Value::as_u64).unwrap_or(p.done);
        p.total = out.get("total").and_then(Value::as_u64).unwrap_or(p.total);
        p.stage = out.get("stage").and_then(Value::as_str).unwrap_or_default().to_string();
    }
    Ok(panel_json(app))
}

fn panel_cancel(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let pr = progress_of(app, args)?;
    app.state.panels.font_audit.progress = None;
    let out = app.session.execute("audit.cancel", &json!({"runId": pr.run_id})).map_err(|e| args.err(e))?;
    let mut partial = out.get("partial").cloned().unwrap_or(Value::Null);
    if let Some(o) = partial.as_object_mut() {
        let (done, total) = (out.get("done").cloned().unwrap_or(Value::Null), out.get("total").cloned().unwrap_or(Value::Null));
        o.insert("progress".into(), json!({"done": done, "total": total}));
    }
    list_report(app, partial, pr.doc, pr.target, pr.params, pr.profile);
    Ok(panel_json(app))
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

fn preview_fix(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let safe_only = args.flag("safeOnly")?.unwrap_or(false);
    let run = app.state.panels.font_audit.run.clone().ok_or_else(|| args.err("nothing has run in the Font Audit panel yet"))?;
    if !run.kept || run.partial {
        return Err(args.err("only results the engine keeps can be fixed: run the audit on this font"));
    }
    let ids = match args.strings("results", MAX_ITEMS)? {
        Some(ids) => ids,
        None => app.state.panels.font_audit.selected.iter().filter(|id| run.item(id).is_some_and(|i| i.fixable)).cloned().collect(),
    };
    if ids.is_empty() {
        return Err(args.err("no selected result has a fix"));
    }
    let mut p = Map::new();
    p.insert("results".into(), json!(ids));
    p.insert("safeOnly".into(), json!(safe_only));
    p.insert("dryRun".into(), json!(true));
    if let Some(id) = &run.run_id {
        p.insert("runId".into(), json!(id));
    }
    let out = app.session.execute("audit.fix", &Value::Object(p)).map_err(|e| args.err(e))?;
    let texts = |key: &str| -> Vec<String> {
        out.get(key).and_then(Value::as_array).map(|a| a.iter().take(MAX_RETURNED).map(step_text).collect()).unwrap_or_default()
    };
    let preview = Preview { ids, run_id: run.run_id.clone(), safe_only, planned: texts("planned"), skipped: texts("skipped") };
    let reply = json!({"preview": preview_json(&preview)});
    app.state.panels.font_audit.preview = Some(preview);
    Ok(reply)
}

fn preview_json(p: &Preview) -> Value {
    json!({"results": p.ids, "safeOnly": p.safe_only, "planned": p.planned, "skipped": p.skipped})
}

fn export_report(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let path = args.string("path")?.filter(|p| !p.is_empty()).ok_or_else(|| args.err("`path` (the report file) is required"))?;
    let run = app.state.panels.font_audit.run.clone().ok_or_else(|| args.err("nothing has run in the Font Audit panel yet"))?;
    let mut p = Map::new();
    p.insert("path".into(), json!(path));
    for key in ["format", "minSeverity"] {
        if let Some(v) = args.string(key)? {
            p.insert(key.into(), json!(v));
        }
    }
    p.insert("results".into(), run.document());
    app.session.execute("audit.export", &Value::Object(p)).map_err(|e| args.err(e))
}

/// Makes sure the panel holds a run: the document's last `audit.run` when the panel has none.
fn ensure_run(app: &mut FontcraftApp, args: &Args) -> Result<Shared<Run>, String> {
    if let Some(run) = &app.state.panels.font_audit.run {
        return Ok(run.clone());
    }
    let doc = app.session.active().map(|d| (d.id, d.revision)).ok_or_else(|| args.err("no font is open"))?;
    let report = app
        .session
        .query("audit.results", &json!({"includeIgnored": true}))
        .map_err(|_| args.err("no audit has run on this font: press Run in the Font Audit panel, or run audit.run"))?;
    let profile = report.get("profile").and_then(Value::as_str).map(str::to_string);
    let mut params = Map::new();
    if let Some(p) = &profile {
        params.insert("profile".into(), json!(p));
    }
    params.insert("includeIgnored".into(), json!(true));
    let checks = check_catalogue(app);
    let run = Shared::new(Run::read(report, doc.0, doc.1, "font".into(), Value::Object(params), true, checks));
    app.state.panels.font_audit.run = Some(run.clone());
    Ok(run)
}

/// The Font Info tab that holds `field` (`master.…`, `instance.…`, `font.…`).
fn font_info_tab(field: &str) -> &'static str {
    match field.split('.').next().unwrap_or("") {
        "master" | "masters" => "masters",
        "instance" | "instances" => "exports",
        "feature" | "features" => "features",
        _ => "font",
    }
}

/// The area `[x, y, width, height]` that shows a marker's spot with room around it.
fn reveal_rect(m: &Marker) -> Option<[f64; 4]> {
    let [x0, y0, x1, y1] = m.bounds.or_else(|| m.point.map(|[x, y]| [x, y, x, y]))?;
    let w = ((x1 - x0).abs() * REVEAL_MARGIN).max(MIN_REVEAL_SPAN);
    let h = ((y1 - y0).abs() * REVEAL_MARGIN).max(MIN_REVEAL_SPAN);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let rect = [cx - w / 2.0, cy - h / 2.0, w, h];
    rect.iter().all(|v| v.is_finite()).then_some(rect)
}

fn reveal_glyph(app: &mut FontcraftApp, args: &Args, index: usize, item: &Item, glyph: &str) -> Result<Value, String> {
    let font = app.session.documents().get(index).map(|d| d.font.clone()).ok_or_else(|| args.err("the font of this result is no longer open"))?;
    let g = font.glyph(glyph).ok_or_else(|| args.err(format!("the font has no glyph `{}` any more", clip(glyph))))?;
    let layer = item.layer.as_ref().and_then(|l| g.layers.iter().find(|x| &x.id == l)).or_else(|| {
        let master = item.master.as_ref()?;
        g.layers.iter().find(|x| x.is_master_layer() && &x.master == master)
    });
    if app.session.active().map(|d| d.id) != app.session.documents().get(index).map(|d| d.id) {
        app.run("document.activate", &json!({"index": index}))?;
    }
    if app.state.editing() != Some(glyph) {
        app.run("view.editSelectedGlyphs", &json!({"glyphs": [glyph]}))?;
    }
    let mut selection = Value::Null;
    if let Some(layer) = layer {
        app.run("layers.select", &json!({"layer": layer.id}))?;
        if !item.stale
            && let Some(marker) = Marker::from_result(&item.raw)
        {
            if let Some(params) = audit_markers::selection_params(layer, &marker) {
                selection = app.run("selection.set", &params)?.get("selection").cloned().unwrap_or(Value::Null);
            }
            if let Some(rect) = reveal_rect(&marker) {
                app.run("view.zoomToRect", &json!({"rect": rect}))?;
            }
        }
    }
    Ok(json!({"revealed": "glyph", "glyph": glyph, "layer": layer.map(|l| l.id.clone()), "selection": selection}))
}

fn reveal(app: &mut FontcraftApp, args: &Args, id: &str) -> Result<Value, String> {
    let run = ensure_run(app, args)?;
    if let Some(asked) = args.string("runId")?
        && run.run_id.as_deref() != Some(asked)
    {
        let listed = run.run_id.as_deref().unwrap_or("none");
        return Err(args.err(format!("the Font Audit panel lists run `{}`, not `{}`", clip(listed), clip(asked))));
    }
    let item = run.item(id).cloned().ok_or_else(|| args.err(format!("the Font Audit panel's last run has no result `{}`", clip(id))))?;
    let index = app.session.documents().iter().position(|d| d.id == run.doc);
    let mut out = match (&item.glyph, &item.source_field) {
        (Some(glyph), _) if run.kept && item.fonts.is_empty() => {
            let index = index.ok_or_else(|| args.err("the font of this result is no longer open"))?;
            reveal_glyph(app, args, index, &item, glyph)?
        }
        (_, Some(field)) if run.kept => {
            let index = index.ok_or_else(|| args.err("the font of this result is no longer open"))?;
            if app.session.active().map(|d| d.id) != Some(run.doc) {
                app.run("document.activate", &json!({"index": index}))?;
            }
            let tab = font_info_tab(field);
            app.run("font.info", &json!({"tab": tab}))?;
            json!({"revealed": "fontInfo", "tab": tab, "field": field})
        }
        _ if !item.fonts.is_empty() || !run.kept => {
            let ids: Vec<&String> = if item.fonts.is_empty() { item.font.iter().collect() } else { item.fonts.iter().collect() };
            let paths: Vec<&str> = ids.iter().filter_map(|f| run.fonts.iter().find(|(id, _)| id == *f).and_then(|(_, p)| p.as_deref())).collect();
            let docs: Vec<usize> = app
                .session
                .documents()
                .iter()
                .enumerate()
                .filter(|(_, d)| d.path.as_ref().is_some_and(|p| paths.iter().any(|q| std::path::Path::new(q) == p.as_path())))
                .map(|(i, _)| i)
                .collect();
            match docs.first() {
                Some(&first) => {
                    app.run("document.activate", &json!({"index": first}))?;
                    json!({"revealed": "fonts", "documents": docs})
                }
                None => json!({"revealed": null, "reason": "none of the fonts this result names is open"}),
            }
        }
        _ => json!({"revealed": null, "reason": "the result names no glyph and no Font Info field: its value is made when the font is compiled"}),
    };
    if let Value::Object(m) = &mut out {
        m.insert("result".into(), json!(id));
        m.insert("runId".into(), json!(run.run_id));
    }
    let st = &mut app.state.panels.font_audit;
    st.current = Some(id.to_string());
    st.selected = vec![id.to_string()];
    Ok(out)
}

fn step(app: &mut FontcraftApp, args: &Args, forward: bool) -> Result<Value, String> {
    let glyph_scope = match args.string("scope")? {
        None | Some("font") => false,
        Some("glyph") => true,
        Some(other) => return Err(args.err(format!("`scope` must be \"font\" or \"glyph\", got \"{}\"", clip(other)))),
    };
    let run = ensure_run(app, args)?;
    let st = &app.state.panels.font_audit;
    let glyph = crate::panels::current_glyph(app);
    let listed = listing(&run, &st.filter, glyph.as_deref(), &[]);
    let current = st.current.clone();
    let scope_glyph = glyph.or_else(|| current.as_deref().and_then(|c| run.item(c)).and_then(|i| i.glyph.clone()));
    let order: Vec<&Item> = listed
        .order
        .iter()
        .filter_map(|&i| run.items.get(i))
        .filter(|it| !glyph_scope || (scope_glyph.is_some() && it.glyph == scope_glyph))
        .collect();
    let n = order.len();
    if n == 0 {
        return Err(args.err("no listed audit result to step to"));
    }
    let at = current.as_deref().and_then(|c| order.iter().position(|it| it.id == c));
    let next = match (at, forward) {
        (Some(p), true) => (p + 1) % n,
        (Some(p), false) => (p + n - 1) % n,
        (None, true) => 0,
        (None, false) => n - 1,
    };
    let id = order.get(next).map(|it| it.id.clone()).ok_or_else(|| args.err("no listed audit result to step to"))?;
    reveal(app, args, &id)
}

fn filter_results(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let mut f = app.state.panels.font_audit.filter.clone();
    if let Some(s) = args.string("minSeverity")? {
        f.min_severity = known(args, "minSeverity", s, &SEVERITIES)?.to_string();
    }
    if let Some(groups) = args.strings("groups", GROUPS.len())? {
        for g in &groups {
            known(args, "group", g, &GROUPS)?;
        }
        f.groups = groups;
    }
    if let Some(t) = args.string("text")? {
        if t.chars().count() > MAX_TEXT {
            return Err(args.err(format!("`text` holds at most {MAX_TEXT} characters")));
        }
        f.text = t.to_string();
    }
    if let Some(b) = args.flag("fixableOnly")? {
        f.fixable_only = b;
    }
    if let Some(b) = args.flag("currentGlyphOnly")? {
        f.current_glyph_only = b;
    }
    if let Some(b) = args.flag("showIgnored")? {
        f.show_ignored = b;
    }
    if let Some(g) = args.string("groupBy")? {
        f.group_by = known(args, "groupBy", g, &GROUP_BY)?.to_string();
    }
    app.state.panels.font_audit.filter = f;
    Ok(panel_json(app))
}

/// One result as the commands list it.
fn item_json(item: &Item) -> Value {
    json!({
        "id": item.id,
        "check": item.check,
        "severity": severity_name(item.severity),
        "glyph": item.glyph,
        "master": item.master,
        "message": item.message,
        "fixable": item.fixable,
        "ignored": item.ignored,
        "stale": item.stale,
    })
}

/// The panel as the commands return it.
pub fn panel_json(app: &FontcraftApp) -> Value {
    let st = &app.state.panels.font_audit;
    let glyph = crate::panels::current_glyph(app);
    let (run, listed, groups, rows) = match &st.run {
        Some(run) => {
            let l = listing(run, &st.filter, glyph.as_deref(), &[]);
            let rows: Vec<Value> = l.order.iter().take(MAX_RETURNED).filter_map(|&i| run.items.get(i)).map(item_json).collect();
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
            (about, l.order.len(), l.groups, rows)
        }
        None => (Value::Null, 0, 0, Vec::new()),
    };
    json!({
        "open": st.open,
        "profile": st.profile,
        "target": st.target,
        "filter": serde_json::to_value(&st.filter).unwrap_or(Value::Null),
        "run": run,
        "progress": st.progress.as_ref().map(|p| json!({"runId": p.run_id, "stage": p.stage, "done": p.done, "total": p.total})),
        "preview": st.preview.as_ref().map(preview_json),
        "listed": listed,
        "groups": groups,
        "more": listed > rows.len(),
        "rows": rows,
        "selected": st.selected,
        "current": st.current,
    })
}

// ── Actions of the buttons ──────────────────────────────────────────────────────────────────────

/// What a click in the panel asks for; turned into commands after drawing.
enum Action {
    Fix { ids: Vec<String>, safe_only: bool },
    Ignore { ids: Vec<String>, reason: String },
    Unignore(Vec<Item>),
    Reveal(String),
}

/// Whether the engine's kept run is the panel's: same document, same results, and each of `ids`
/// names the same check, glyph and layer in both.
fn kept_matches(app: &mut FontcraftApp, run: &Run, ids: &[String]) -> bool {
    if app.session.active().map(|d| d.id) != Some(run.doc) {
        return false;
    }
    let Ok(doc) = app.session.query("audit.results", &json!({"includeIgnored": true})) else { return false };
    if let Some(id) = &run.run_id {
        return doc.get("runId").and_then(Value::as_str) == Some(id.as_str()) && ids.iter().all(|i| run.item(i).is_some());
    }
    let Some(list) = doc.get("results").and_then(Value::as_array) else { return false };
    if list.len() != run.items.len() {
        return false;
    }
    ids.iter().all(|id| {
        let Some(it) = run.item(id) else { return false };
        list.iter().any(|r| {
            r.get("id").and_then(Value::as_str) == Some(id.as_str())
                && r.get("check").and_then(Value::as_str) == Some(it.check.as_str())
                && r.pointer("/location/glyph").and_then(Value::as_str) == it.glyph.as_deref()
                && r.pointer("/location/layer").and_then(Value::as_str) == it.layer.as_deref()
        })
    })
}

/// `steps` as one command: the command itself when there is one step, else a `command.batch`.
fn one_step(label: &str, mut steps: Vec<(&'static str, Value)>, pending: &mut Pending) {
    if steps.len() == 1 {
        if let Some(step) = steps.pop() {
            pending.push(step);
        }
        return;
    }
    let steps: Vec<Value> = steps.into_iter().map(|(command, params)| json!({"command": command, "params": params})).collect();
    pending.push(("command.batch", json!({"label": label, "steps": steps})));
}

fn act(app: &mut FontcraftApp, run: &Run, action: Action, pending: &mut Pending) {
    let again = ("audit.panel.run", json!({"again": true}));
    match action {
        Action::Reveal(id) => pending.push(("audit.revealIssue", json!({"result": id}))),
        Action::Fix { ids, safe_only } => {
            let mut p = Map::new();
            p.insert("results".into(), json!(ids));
            p.insert("safeOnly".into(), json!(safe_only));
            match (&run.run_id, kept_matches(app, run, &ids)) {
                // The run the engine keeps is this one: name it, so a run that replaces it
                // between now and the fix makes the fix refuse instead of fixing other results.
                (Some(id), true) => {
                    p.insert("runId".into(), json!(id));
                }
                (_, true) => {}
                (_, false) => pending.push(again.clone()),
            }
            pending.push(("audit.fix", Value::Object(p)));
            pending.push(again);
        }
        Action::Ignore { ids, reason } => {
            if !kept_matches(app, run, &ids) {
                pending.push(again.clone());
            }
            let reason = reason.trim().to_string();
            let steps = ids
                .into_iter()
                .map(|id| {
                    let mut p = serde_json::Map::new();
                    p.insert("result".into(), json!(id));
                    if !reason.is_empty() {
                        p.insert("reason".into(), json!(reason));
                    }
                    let p = Value::Object(p);
                    ("audit.ignoreIssue", p)
                })
                .collect();
            one_step("Ignore Issues", steps, pending);
            pending.push(again);
        }
        Action::Unignore(items) => {
            let steps = items
                .into_iter()
                .map(|it| {
                    let mut p = serde_json::Map::new();
                    p.insert("check".into(), json!(it.check));
                    if let Some(g) = it.glyph {
                        p.insert("glyph".into(), json!(g));
                    }
                    if let Some(m) = it.master {
                        p.insert("master".into(), json!(m));
                    }
                    let p = Value::Object(p);
                    ("audit.unignoreIssue", p)
                })
                .collect();
            one_step("Stop Ignoring Issues", steps, pending);
            pending.push(again);
        }
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

/// The id of a panel control: `run`, `fix`, `fixAllSafe`, `ignore`, `unignore`, `previous`,
/// `next`, `reason`, `search`, `fixableOnly`, `currentGlyphOnly`, `showIgnored`.
pub fn control_id(name: &str) -> Id {
    widgets::id(("window.fontAudit", name))
}

/// The id of the row of result `id`.
pub fn row_id(id: &str) -> Id {
    widgets::id(("window.fontAudit", "row", id))
}

/// What the person did this frame that is view state, or asks for commands.
#[derive(Default)]
struct Output {
    /// A click on a result: its id, with Cmd, with Shift.
    click: Option<(String, bool, bool)>,
    toggle_group: Option<String>,
    reason: Option<String>,
    export_open: Option<bool>,
    export_path: Option<String>,
    export_format: Option<String>,
    /// Choose… of the export row: ask the save chooser for the report file.
    choose_export: bool,
    actions: Vec<Action>,
}

/// What one frame draws from.
struct Frame<'a> {
    st: &'a FontAuditState,
    tokens: &'a Tokens,
    listing: Option<Shared<Listing>>,
    /// The revision of the run's document while it is the active one.
    revision: Option<u64>,
    live: bool,
    masters: HashMap<String, String>,
    /// The window has a save chooser: the export row shows Choose….
    chooser: bool,
}

fn label_of(list: &[(String, String)], id: &str) -> String {
    list.iter().find(|(i, _)| i == id).map_or_else(|| id.to_string(), |(_, l)| l.clone())
}

/// The formats Export Report offers: the `format` of `audit.export`, the label, and the file
/// extension `audit.export` reads as that format.
const EXPORT_FORMATS: [(&str, &str, &str); 4] =
    [("html", "HTML", "html"), ("markdown", "Markdown", "md"), ("json", "JSON", "json"), ("text", "Text", "txt")];

fn header(ui: &mut egui::Ui, f: &Frame, out: &mut Output, pending: &mut Pending) {
    let st = f.st;
    ui.horizontal(|ui| {
        let caption = match &st.profile {
            Some(p) => label_of(&st.profiles, p),
            None => format!("Default ({})", label_of(&st.profiles, &st.default_profile)),
        };
        egui::ComboBox::from_id_salt("window.fontAudit.profile").selected_text(caption).show_ui(ui, |ui| {
            if ui.selectable_label(st.profile.is_none(), "Default").clicked() && st.profile.is_some() {
                pending.push(("audit.showPanel", json!({"visible": true, "profile": null})));
            }
            for (id, label) in &st.profiles {
                if ui.selectable_label(st.profile.as_deref() == Some(id), label).clicked() && st.profile.as_deref() != Some(id) {
                    pending.push(("audit.showPanel", json!({"visible": true, "profile": id})));
                }
            }
        });
        let target = TARGETS.iter().find(|(id, _)| *id == st.target).map_or(st.target.as_str(), |(_, l)| *l);
        egui::ComboBox::from_id_salt("window.fontAudit.target").selected_text(target).show_ui(ui, |ui| {
            for (id, label) in TARGETS {
                if ui.selectable_label(st.target == id, label).clicked() && st.target != id {
                    pending.push(("audit.showPanel", json!({"visible": true, "target": id})));
                }
            }
        });
        match &st.progress {
            Some(p) => {
                let fraction = if p.total == 0 { 0.0 } else { (p.done as f32 / p.total as f32).clamp(0.0, 1.0) };
                let bar = egui::ProgressBar::new(fraction)
                    .desired_width(f.tokens.field_width * 2.0)
                    .fill(f.tokens.accent)
                    .text(format!("{} of {}", p.done, p.total));
                ui.add(bar);
                if widgets::button(ui, control_id("cancel"), "Cancel", "Stop the audit; what it found so far is listed, marked partial", true, false)
                {
                    pending.push(("audit.panel.cancel", json!({})));
                }
            }
            None => {
                if widgets::button(ui, control_id("run"), "Run", "Run the audit with this profile on this target", true, false) {
                    pending.push(("audit.panel.run", json!({"inSteps": true})));
                }
            }
        }
        ui.menu_button("⚙", |ui| {
            if ui.button("Edit Profiles and Thresholds…").clicked() {
                pending.push(("audit.editProfile", json!({"profile": st.profile})));
                ui.close();
            }
            if ui.button("Export Report…").on_hover_text("Write the listed run to a JSON, Markdown, HTML or text file").clicked() {
                out.export_open = Some(!st.export_open);
                ui.close();
            }
            if ui.button("Clear Cache").on_hover_text("Forget the kept results, so the next run examines every glyph again").clicked() {
                pending.push(("audit.clearCache", json!({})));
                ui.close();
            }
            if ui.selectable_label(f.live, "Live Audit").on_hover_text("Re-run the outlines profile on the edited glyph after each change").clicked()
            {
                pending.push(("audit.toggleLiveAudit", json!({})));
                ui.close();
            }
        });
    });
}

/// The row Export Report shows: the file, its format and Export.
fn export_row(ui: &mut egui::Ui, f: &Frame, out: &mut Output, pending: &mut Pending) {
    let st = f.st;
    ui.horizontal(|ui| {
        let mut path = st.export_path.clone();
        let edit =
            egui::TextEdit::singleline(&mut path).id(control_id("exportPath")).hint_text("Report file").desired_width(f.tokens.field_width * 3.0);
        if ui.add(edit).changed() {
            out.export_path = Some(path);
        }
        if f.chooser && widgets::button(ui, control_id("exportChoose"), "Choose…", "Choose the report file", true, false) {
            out.choose_export = true;
        }
        let caption = EXPORT_FORMATS.iter().find(|(id, ..)| *id == st.export_format).map_or("HTML", |(_, l, _)| *l);
        egui::ComboBox::from_id_salt("window.fontAudit.exportFormat").selected_text(caption).show_ui(ui, |ui| {
            for (id, label, _) in EXPORT_FORMATS {
                if ui.selectable_label(st.export_format == id, label).clicked() {
                    out.export_format = Some(id.to_string());
                }
            }
        });
        let ready = !st.export_path.trim().is_empty();
        if widgets::button(ui, control_id("export"), "Export", "Write the listed run to this file", ready, false) {
            pending.push(("audit.panel.exportReport", json!({"path": st.export_path.trim(), "format": st.export_format})));
        }
    });
}

/// Choose… of the export row: asks the save chooser for the report file, offering the name typed
/// so far or one with the format's extension, and puts the answer in the path field. A cancelled
/// chooser leaves the field as it was.
fn choose_report_file(app: &mut FontcraftApp) {
    let st = &app.state.panels.font_audit;
    let (label, extension) = EXPORT_FORMATS.iter().find(|(id, ..)| *id == st.export_format).map_or(("HTML", "html"), |(_, l, e)| (*l, *e));
    let typed = std::path::Path::new(st.export_path.trim()).file_name().map(|n| n.to_string_lossy().into_owned());
    let suggested = typed.filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Font Audit Report.{extension}"));
    let Some(picker) = app.picker.as_mut() else { return };
    if let Some(path) = picker.save("Export Report", &format!("{label} report"), &[extension], &suggested) {
        app.state.panels.font_audit.export_path = path.to_string_lossy().into_owned();
    }
}

fn summary(ui: &mut egui::Ui, f: &Frame, run: &Run) {
    let t = f.tokens;
    let n = |key: &str| run.summary.get(key).and_then(Value::as_u64).unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        for (key, label, colour) in [
            ("fail", "Fail", t.audit_fail),
            ("warn", "Warn", t.audit_warn),
            ("info", "Info", t.audit_info),
            ("passed", "Passed", t.text),
            ("skipped", "Skipped", t.text_dim),
        ] {
            ui.colored_label(colour, format!("{label} {}", n(key)));
        }
        if n("ignored") > 0 {
            ui.colored_label(t.text_dim, format!("Ignored {}", n("ignored")));
        }
        if n("errors") > 0 {
            ui.colored_label(t.error_text, format!("Errors {}", n("errors")));
        }
    });
    if run.partial {
        let of = run.head.pointer("/progress/done").and_then(Value::as_u64).zip(run.head.pointer("/progress/total").and_then(Value::as_u64));
        let text = match of {
            Some((done, total)) => format!("Partial results: the run was cancelled after {done} of {total} steps."),
            None => "Partial results: the run was cancelled.".to_string(),
        };
        ui.colored_label(t.audit_warn, text);
    }
    for w in &run.warnings {
        ui.colored_label(t.text_dim, w);
    }
}

fn filters(ui: &mut egui::Ui, f: &Frame, pending: &mut Pending) {
    let filter = &f.st.filter;
    let set = |pending: &mut Pending, key: &str, value: Value| {
        let mut m = Map::new();
        m.insert(key.into(), value);
        pending.push(("audit.filterResults", Value::Object(m)));
    };
    ui.horizontal_wrapped(|ui| {
        for (id, label) in SEVERITIES {
            if widgets::button(ui, control_id(id), label, "Which severities the list shows", true, filter.min_severity == id)
                && filter.min_severity != id
            {
                set(pending, "minSeverity", json!(id));
            }
        }
        let caption = match filter.groups.as_slice() {
            [] => "All Groups".to_string(),
            [one] => GROUPS.iter().find(|(id, _)| id == one).map_or_else(|| one.clone(), |(_, l)| l.to_string()),
            more => format!("{} Groups", more.len()),
        };
        ui.menu_button(caption, |ui| {
            if ui.selectable_label(filter.groups.is_empty(), "All Groups").clicked() {
                set(pending, "groups", json!([]));
                ui.close();
            }
            for (id, label) in GROUPS {
                let on = filter.groups.iter().any(|g| g == id);
                if ui.selectable_label(on, label).clicked() {
                    let groups: Vec<&str> =
                        GROUPS.iter().map(|(g, _)| *g).filter(|g| if *g == id { !on } else { filter.groups.iter().any(|x| x == g) }).collect();
                    set(pending, "groups", json!(groups));
                }
            }
        });
        let mut text = filter.text.clone();
        let edit = egui::TextEdit::singleline(&mut text).id(control_id("search")).hint_text("Search").desired_width(f.tokens.field_width * 2.0);
        if ui.add(edit).changed() {
            set(pending, "text", json!(text));
        }
        for (key, label, on, hover) in [
            ("fixableOnly", "Fixable Only", filter.fixable_only, "List only results with an automatic fix"),
            ("currentGlyphOnly", "Current Glyph Only", filter.current_glyph_only, "List only results of the glyph being edited"),
            ("showIgnored", "Show Ignored", filter.show_ignored, "List ignored results too"),
        ] {
            if widgets::button(ui, control_id(key), label, hover, true, on) {
                set(pending, key, json!(!on));
            }
        }
        let by = GROUP_BY.iter().find(|(id, _)| *id == filter.group_by).map_or(filter.group_by.as_str(), |(_, l)| *l);
        egui::ComboBox::from_id_salt("window.fontAudit.groupBy").selected_text(format!("By {by}")).show_ui(ui, |ui| {
            for (id, label) in GROUP_BY {
                if ui.selectable_label(filter.group_by == id, label).clicked() && filter.group_by != id {
                    set(pending, "groupBy", json!(id));
                }
            }
        });
    });
}

/// A small triangle pointing right (folded) or down (open), centred on `at`.
fn disclosure(at: Pos2, size: f32, open: bool, colour: Color32) -> Shape {
    let h = size / 2.0;
    let points = if open {
        vec![pos2(at.x - h, at.y - h / 2.0), pos2(at.x + h, at.y - h / 2.0), pos2(at.x, at.y + h / 2.0)]
    } else {
        vec![pos2(at.x - h / 2.0, at.y - h), pos2(at.x + h / 2.0, at.y), pos2(at.x - h / 2.0, at.y + h)]
    };
    Shape::convex_polygon(points, colour, Stroke::NONE)
}

fn place_of(item: &Item) -> String {
    match (&item.glyph, &item.table) {
        (Some(g), _) => g.clone(),
        (None, Some(t)) => t.clone(),
        (None, None) => "font".into(),
    }
}

fn result_row(ui: &mut egui::Ui, f: &Frame, item: &Item, line: f32, out: &mut Output) {
    let t = f.tokens;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), line), Sense::hover());
    let resp = ui.interact(rect, row_id(&item.id), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let selected = f.st.selected.contains(&item.id);
    let p = ui.painter_at(rect);
    if selected {
        p.rect_filled(rect, 0.0, t.row_selected);
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, t.hover);
    }
    let colour = if selected {
        t.selected_text
    } else if item.ignored || item.stale {
        t.text_dim
    } else {
        t.text
    };
    let font = FontId::proportional(t.font_size);
    let radius = (line * 0.25).min(t.audit_badge_radius.max(1.0));
    let mut x = rect.left() + t.cell_inset + radius;
    p.circle_filled(pos2(x, rect.center().y), radius, severity_colour(t, item.severity));
    x += radius + t.cell_inset;
    let columns = [
        (place_of(item), t.list_column_unit * PLACE_UNITS),
        (item.master.as_ref().map(|m| f.masters.get(m).cloned().unwrap_or_else(|| m.clone())).unwrap_or_default(), t.list_column_unit * MASTER_UNITS),
    ];
    for (text, width) in columns {
        let cell = Rect::from_min_max(pos2(x, rect.top()), pos2((x + width).min(rect.right()), rect.bottom()));
        ui.painter_at(cell).text(pos2(cell.left(), cell.center().y), Align2::LEFT_CENTER, text, font.clone(), colour);
        x += width;
    }
    let mut message = item.message.clone();
    if item.ignored {
        message.push_str(" (ignored)");
    }
    if item.stale {
        message.push_str(" (changed since the run)");
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, &message, font, colour);
    let resp = resp.on_hover_text(message);
    if resp.double_clicked() {
        out.actions.push(Action::Reveal(item.id.clone()));
    } else if resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        out.click = Some((item.id.clone(), m.command, m.shift));
    }
}

fn list(ui: &mut egui::Ui, f: &Frame, run: &Run, out: &mut Output) {
    let t = f.tokens;
    let Some(listing) = &f.listing else { return };
    if listing.rows.is_empty() {
        let text = if run.items.is_empty() { "No issues found." } else { "No result matches the filters." };
        ui.colored_label(t.text_dim, text);
        return;
    }
    let line = t.list_row_height;
    egui::ScrollArea::vertical().id_salt("window.fontAudit.list").auto_shrink([false, true]).max_height(line * VISIBLE_ROWS).show_rows(
        ui,
        line,
        listing.rows.len(),
        |ui, range| {
            for row in listing.rows.get(range).unwrap_or_default() {
                match row {
                    Row::Header { key, title, count } => {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), line), Sense::click());
                        let open = !f.st.collapsed.contains(key);
                        let p = ui.painter_at(rect);
                        let icon = line * 0.4;
                        p.add(disclosure(pos2(rect.left() + t.cell_inset + icon / 2.0, rect.center().y), icon, open, t.text_dim));
                        let at = pos2(rect.left() + 2.0 * t.cell_inset + icon, rect.center().y);
                        p.text(at, Align2::LEFT_CENTER, format!("{title} ({count})"), FontId::proportional(t.font_size), t.text);
                        if resp.clicked() {
                            out.toggle_group = Some(key.clone());
                        }
                    }
                    Row::Result(i) => {
                        if let Some(item) = run.items.get(*i) {
                            result_row(ui, f, item, line, out);
                        }
                    }
                }
            }
        },
    );
}

fn location_text(item: &Item, masters: &HashMap<String, String>) -> String {
    let mut parts = Vec::new();
    if let Some(g) = &item.glyph {
        parts.push(format!("glyph {g}"));
    }
    if let Some(m) = &item.master {
        parts.push(format!("master {}", masters.get(m).unwrap_or(m)));
    }
    if let Some(l) = &item.layer
        && item.master.as_ref() != Some(l)
    {
        parts.push(format!("layer {l}"));
    }
    for (key, label) in [("table", "table"), ("field", "field"), ("instance", "instance"), ("sourceField", "Font Info field")] {
        if let Some(v) = item.raw.pointer(&format!("/location/{key}")).and_then(Value::as_str) {
            parts.push(format!("{label} {v}"));
        }
    }
    if !item.fonts.is_empty() {
        parts.push(format!("{} fonts", item.fonts.len()));
    }
    parts.join(", ")
}

fn details(ui: &mut egui::Ui, f: &Frame, run: &Run) {
    let t = f.tokens;
    let Some(item) = f.st.current.as_deref().and_then(|c| run.item(c)) else { return };
    ui.separator();
    let info = run.checks.get(&item.check);
    ui.label(RichText::new(info.map_or(item.check.as_str(), |c| c.label.as_str())).strong());
    if let Some(c) = info
        && !c.description.is_empty()
    {
        ui.colored_label(t.text_dim, &c.description);
    }
    let place = location_text(item, &f.masters);
    if !place.is_empty() {
        ui.label(place);
    }
    if let Some(data) = item.raw.get("data").filter(|d| d.as_object().is_some_and(|m| !m.is_empty())) {
        let text: String = data.to_string().chars().take(MAX_DATA_CHARS).collect();
        ui.label(RichText::new(text).monospace().color(t.text_dim));
    }
    let fix = if item.fixable {
        let what = item.raw.pointer("/fix/description").and_then(Value::as_str).unwrap_or("an automatic fix");
        let kind = if item.safe { "safe" } else { "changes the design" };
        format!("Fix: {what} ({kind})")
    } else {
        "No automatic fix.".to_string()
    };
    ui.colored_label(t.text_dim, fix);
    if let Some(p) = f.st.preview.as_ref().filter(|p| p.run_id == run.run_id && p.ids.contains(&item.id)) {
        ui.label(RichText::new(format!("Fix preview ({} results)", p.ids.len())).strong());
        if p.planned.is_empty() {
            ui.colored_label(t.text_dim, "Nothing would change.");
        }
        for line in &p.planned {
            ui.label(line);
        }
        for line in &p.skipped {
            ui.colored_label(t.text_dim, format!("Skipped: {line}"));
        }
    }
}

fn buttons(ui: &mut egui::Ui, f: &Frame, run: &Run, out: &mut Output, pending: &mut Pending) {
    let st = f.st;
    let current = f.revision == Some(run.revision);
    let editable = run.kept && current && !run.partial;
    let why = if run.partial {
        "Partial results of a cancelled run cannot be fixed or ignored: press Run again"
    } else if !run.kept {
        "Results of font files are fixed and ignored in the files themselves"
    } else if f.revision.is_none() {
        "The font of this run is not the active font"
    } else if !current {
        "The font changed since this run: press Run again"
    } else {
        ""
    };
    let selected: Vec<&Item> = st.selected.iter().filter_map(|id| run.item(id)).collect();
    let fixable: Vec<String> = selected.iter().filter(|i| i.fixable && !i.ignored && !i.stale).map(|i| i.id.clone()).collect();
    let safe: Vec<String> = f
        .listing
        .as_ref()
        .map(|l| {
            l.order.iter().filter_map(|&i| run.items.get(i)).filter(|i| i.fixable && i.safe && !i.ignored && !i.stale).map(|i| i.id.clone()).collect()
        })
        .unwrap_or_default();
    let to_ignore: Vec<String> = selected.iter().filter(|i| !i.ignored).map(|i| i.id.clone()).collect();
    let ignored: Vec<Item> = selected.iter().filter(|i| i.ignored).map(|i| (*i).clone()).collect();
    let hover = |text: &'static str| if editable { text } else { why };
    ui.horizontal_wrapped(|ui| {
        let preview = st.preview.as_ref().filter(|p| p.run_id == run.run_id && !p.ids.is_empty());
        if widgets::button(
            ui,
            control_id("previewFix"),
            "Preview Fix",
            hover("Show what fixing the selected results would change"),
            editable && !fixable.is_empty(),
            false,
        ) {
            pending.push(("audit.panel.previewFix", json!({"results": fixable})));
        }
        if let Some(p) = preview
            && widgets::button(ui, control_id("applyPreview"), "Apply", hover("Make the changes the preview lists"), editable, false)
        {
            out.actions.push(Action::Fix { ids: p.ids.clone(), safe_only: p.safe_only });
        }
        if widgets::button(ui, control_id("fix"), "Fix", hover("Fix the selected results"), editable && !fixable.is_empty(), false) {
            out.actions.push(Action::Fix { ids: fixable.clone(), safe_only: false });
        }
        if widgets::button(
            ui,
            control_id("fixAllSafe"),
            "Fix All Safe",
            hover("Fix every listed result whose fix keeps the design"),
            editable && !safe.is_empty(),
            false,
        ) {
            out.actions.push(Action::Fix { ids: safe.clone(), safe_only: true });
        }
        let mut reason = st.reason.clone();
        let edit = egui::TextEdit::singleline(&mut reason).id(control_id("reason")).hint_text("Reason").desired_width(f.tokens.field_width * 1.5);
        if ui.add(edit).changed() {
            out.reason = Some(reason);
        }
        if widgets::button(
            ui,
            control_id("ignore"),
            "Ignore",
            hover("Hide the selected results from now on, with this reason"),
            editable && !to_ignore.is_empty(),
            false,
        ) {
            out.actions.push(Action::Ignore { ids: to_ignore.clone(), reason: st.reason.clone() });
        }
        if st.filter.show_ignored
            && widgets::button(
                ui,
                control_id("unignore"),
                "Stop Ignoring",
                hover("List the selected ignored results again"),
                run.kept && !ignored.is_empty(),
                false,
            )
        {
            out.actions.push(Action::Unignore(ignored.clone()));
        }
        if widgets::button(ui, control_id("previous"), "Previous", "Reveal the listed result before this one", true, false) {
            pending.push(("audit.previousIssue", json!({})));
        }
        if widgets::button(ui, control_id("next"), "Next", "Reveal the listed result after this one", true, false) {
            pending.push(("audit.nextIssue", json!({})));
        }
    });
}

/// Applies a click on a result: the selection and the current result (view state).
fn select(st: &mut FontAuditState, listing: Option<&Listing>, id: String, command: bool, shift: bool) {
    let run = st.run.clone();
    if shift && let (Some(run), Some(listing), Some(anchor)) = (run, listing, st.current.clone()) {
        let ids: Vec<&str> = listing.order.iter().filter_map(|&i| run.items.get(i)).map(|i| i.id.as_str()).collect();
        if let (Some(a), Some(b)) = (ids.iter().position(|i| *i == anchor), ids.iter().position(|i| *i == id)) {
            let (lo, hi) = (a.min(b), a.max(b));
            st.selected = ids.get(lo..=hi).unwrap_or_default().iter().map(|s| s.to_string()).collect();
            return;
        }
    }
    if command {
        match st.selected.iter().position(|s| *s == id) {
            Some(i) => {
                st.selected.remove(i);
            }
            None => st.selected.push(id.clone()),
        }
    } else {
        st.selected = vec![id.clone()];
    }
    st.current = Some(id);
}

/// Draws the panel while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.font_audit.open {
        return;
    }
    let glyph = crate::panels::current_glyph(app);
    let listing = cached_listing(&mut app.state.panels.font_audit, glyph.as_deref());
    let st = &app.state.panels.font_audit;
    let run = st.run.clone();
    let doc = run.as_ref().and_then(|r| app.session.active().filter(|d| d.id == r.doc));
    let revision = doc.map(|d| d.revision);
    let masters: HashMap<String, String> = doc.map(|d| d.font.masters.iter().map(|m| (m.id.clone(), m.name.clone())).collect()).unwrap_or_default();
    let frame =
        Frame { st, tokens: &app.tokens, listing: listing.clone(), revision, live: app.state.audit.live, masters, chooser: app.picker.is_some() };
    let mut out = Output::default();
    let mut open = true;
    let tokens = &app.tokens;
    egui::Window::new("Font Audit")
        .id(Id::new("window.fontAudit"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(tokens.list_column_unit * PANEL_UNITS)
        .default_pos(pos2(ui.ctx().content_rect().right() - tokens.list_column_unit * PANEL_UNITS - tokens.dialog_spacing, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            header(ui, &frame, &mut out, pending);
            if frame.st.export_open && run.is_some() {
                export_row(ui, &frame, &mut out, pending);
            }
            match &run {
                None => {
                    ui.add_space(tokens.dialog_spacing);
                    ui.colored_label(tokens.text_dim, "Choose a profile and a target, then press Run.");
                }
                Some(run) => {
                    summary(ui, &frame, run);
                    filters(ui, &frame, pending);
                    ui.separator();
                    list(ui, &frame, run, &mut out);
                    details(ui, &frame, run);
                    ui.separator();
                    buttons(ui, &frame, run, &mut out, pending);
                    let typing = ui.ctx().memory(|m| m.focused().is_some());
                    if ui.ui_contains_pointer()
                        && !typing
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && let Some(current) = &frame.st.current
                    {
                        out.actions.push(Action::Reveal(current.clone()));
                    }
                }
            }
        });
    let st = &mut app.state.panels.font_audit;
    if !open {
        st.open = false;
    }
    if let Some(key) = out.toggle_group {
        match st.collapsed.iter().position(|k| *k == key) {
            Some(i) => {
                st.collapsed.remove(i);
            }
            None => st.collapsed.push(key),
        }
    }
    if let Some(reason) = out.reason {
        st.reason = reason;
    }
    if let Some(v) = out.export_open {
        st.export_open = v;
    }
    if let Some(v) = out.export_path {
        st.export_path = v;
    }
    if let Some(v) = out.export_format {
        st.export_format = v;
    }
    // A run in steps goes on by one step a frame until it finishes or is cancelled.
    if st.progress.is_some() && st.open {
        pending.push(("audit.panel.continue", json!({})));
        ui.ctx().request_repaint();
    }
    if let Some((id, command, shift)) = out.click {
        select(st, listing.as_deref(), id, command, shift);
    }
    if out.choose_export {
        choose_report_file(app);
    }
    if let Some(run) = run {
        for action in out.actions {
            act(app, &run, action, pending);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use egui::{Event, Modifiers, PointerButton};
    use serde_json::{Value, json};

    use super::*;
    use crate::control;

    fn call(app: &mut FontcraftApp, id: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": id, "params": params}}));
        assert_eq!(r["ok"], true, "{id}: {r}");
        r["result"].clone()
    }

    fn refused(app: &mut FontcraftApp, id: &str, params: Value) -> String {
        let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": id, "params": params}}));
        assert_eq!(r["ok"], false, "{id} should be refused: {r}");
        r["error"].as_str().unwrap_or_default().to_string()
    }

    /// Glyph A: one closed curve whose rightmost point lies between its nodes (a missing extreme,
    /// a warning with a safe fix). Glyph B: an open line (a failure whose fix is not safe).
    fn audited_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        let nodes = json!([
            {"x": 100, "y": 100}, {"x": 300, "y": 100, "type": "offcurve"}, {"x": 300, "y": 300, "type": "offcurve"}, {"x": 100, "y": 300, "type": "curve"}
        ]);
        app.run("path.addPath", &json!({"glyph": "A", "closed": true, "nodes": nodes})).unwrap();
        app.run("path.addPath", &json!({"glyph": "B", "closed": false, "nodes": [{"x": 400, "y": 0}, {"x": 500, "y": 0}]})).unwrap();
        app
    }

    fn id_of(app: &FontcraftApp, check: &str, glyph: &str) -> String {
        let run = app.state.panels.font_audit.run.clone().unwrap();
        run.items.iter().find(|i| i.check == check && i.glyph.as_deref() == Some(glyph)).unwrap_or_else(|| panic!("no {check} on {glyph}")).id.clone()
    }

    fn has(app: &FontcraftApp, check: &str, glyph: &str) -> bool {
        let run = app.state.panels.font_audit.run.clone().unwrap();
        run.items.iter().any(|i| i.check == check && i.glyph.as_deref() == Some(glyph) && !i.ignored)
    }

    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn new() -> Self {
            Self { ctx: egui::Context::default() }
        }

        fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))), events, ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
            out.textures_delta.clear();
        }

        fn click(&self, app: &mut FontcraftApp, id: Id) {
            self.frame(app, Vec::new());
            self.frame(app, Vec::new());
            let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(app, vec![Event::PointerMoved(at)]);
            self.frame(app, vec![button(true)]);
            self.frame(app, vec![button(false)]);
            self.frame(app, Vec::new());
        }
    }

    #[test]
    fn audit_show_panel_and_panel_run_list_what_audit_run_returns_and_filter_results_narrows_it() {
        let mut app = audited_app();
        let window = crate::shell::menus::menu_tree(&app.session).into_iter().find(|m| m.title == "Window").unwrap();
        assert!(window.items.iter().any(|i| i.command == "audit.showPanel" && i.label == "Font Audit"), "Window › Font Audit");
        assert_eq!(call(&mut app, "audit.showPanel", json!({}))["open"], true);
        let w = Window::new();
        w.frame(&mut app, Vec::new());

        let out = call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
        assert_eq!((out["profile"].clone(), out["run"]["kept"].clone()), (json!("outlines"), json!(true)));
        let kept = app.session.query("audit.results", &json!({"includeIgnored": true})).unwrap();
        let ids: Vec<&str> = kept["results"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        let run = app.state.panels.font_audit.run.clone().unwrap();
        assert_eq!(run.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ids, "the panel lists the run the engine keeps");
        assert_eq!(out["listed"], json!(ids.len()));
        assert!(has(&app, "check.outlines.missingExtremes", "A") && has(&app, "check.outlines.openPath", "B"));
        w.frame(&mut app, Vec::new());

        let out = call(&mut app, "audit.filterResults", json!({"minSeverity": "fail"}));
        let rows = out["rows"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.iter().all(|r| r["severity"] == "fail"), "{rows:?}");
        let out = call(&mut app, "audit.filterResults", json!({"minSeverity": "warn", "text": "MissingExtremes", "groupBy": "glyph"}));
        assert_eq!(out["listed"], 1, "the search ignores case");
        assert_eq!(out["rows"][0]["glyph"], "A");
        w.frame(&mut app, Vec::new());

        let before = app.state.panels.font_audit.filter.clone();
        refused(&mut app, "audit.filterResults", json!({"minSeverity": "worst"}));
        refused(&mut app, "audit.filterResults", json!({"groups": ["outlines", "colour"]}));
        refused(&mut app, "audit.showPanel", json!({"target": "everything"}));
        assert_eq!(app.state.panels.font_audit.filter, before, "a refused command changes nothing");
        assert_eq!(app.state.panels.font_audit.target, "font");
        assert_eq!(call(&mut app, "audit.showPanel", json!({"visible": false}))["open"], false);
    }

    #[test]
    fn audit_reveal_issue_opens_the_glyph_and_next_issue_and_previous_issue_wrap() {
        let mut app = audited_app();
        call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
        call(&mut app, "audit.filterResults", json!({"minSeverity": "warn"}));
        let extreme = id_of(&app, "check.outlines.missingExtremes", "A");
        let out = call(&mut app, "audit.revealIssue", json!({"result": extreme}));
        assert_eq!((out["revealed"].clone(), out["glyph"].clone()), (json!("glyph"), json!("A")));
        assert_eq!(app.state.editing(), Some("A"), "the glyph opens in Edit View");
        assert!(out["layer"].is_string());
        assert_eq!(app.state.panels.font_audit.current.as_deref(), Some(extreme.as_str()));
        Window::new().frame(&mut app, Vec::new());

        let listed = call(&mut app, "audit.filterResults", json!({}))["listed"].as_u64().unwrap() as usize;
        assert!(listed >= 2);
        let mut seen = Vec::new();
        for _ in 0..listed {
            seen.push(call(&mut app, "audit.nextIssue", json!({}))["result"].as_str().unwrap().to_string());
        }
        assert_eq!(seen.last(), Some(&extreme), "Next comes back to the start after the last result");
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), listed, "Next visits every listed result once");
        let back = call(&mut app, "audit.previousIssue", json!({}))["result"].as_str().unwrap().to_string();
        assert_eq!(call(&mut app, "audit.nextIssue", json!({}))["result"], json!(extreme), "Previous and Next undo each other ({back})");

        let only_a = call(&mut app, "audit.nextIssue", json!({"scope": "glyph"}));
        assert_eq!(only_a["glyph"], "A", "the glyph scope stays in the edited glyph");
        refused(&mut app, "audit.revealIssue", json!({"result": "r999999"}));
        refused(&mut app, "audit.nextIssue", json!({"scope": "everywhere"}));
        call(&mut app, "audit.filterResults", json!({"text": "nothing matches this"}));
        refused(&mut app, "audit.nextIssue", json!({}));
    }

    #[test]
    fn the_fix_button_fixes_the_selected_result_after_live_audit_replaced_the_kept_run() {
        let mut app = audited_app();
        call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
        call(&mut app, "audit.filterResults", json!({"minSeverity": "warn"}));
        let extreme = id_of(&app, "check.outlines.missingExtremes", "A");
        app.run("audit.toggleLiveAudit", &json!({"on": true})).unwrap();
        call(&mut app, "audit.revealIssue", json!({"result": extreme}));
        let w = Window::new();
        w.frame(&mut app, Vec::new());
        assert!(app.state.audit.runs >= 1, "live audit ran on the revealed glyph and replaced the kept run");
        let undo = app.session.doc().unwrap().history.undo.len();

        w.click(&mut app, row_id(&extreme));
        assert_eq!(app.state.panels.font_audit.selected, vec![extreme.clone()]);
        w.click(&mut app, control_id("fix"));
        assert!(!has(&app, "check.outlines.missingExtremes", "A"), "the re-run no longer lists the fixed result");
        assert!(has(&app, "check.outlines.openPath", "B"), "other results stay");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "the fix is one undo step");
    }

    #[test]
    fn ignore_hides_the_selected_result_and_show_ignored_lists_it_again() {
        let mut app = audited_app();
        call(&mut app, "audit.showPanel", json!({"visible": true}));
        call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
        call(&mut app, "audit.filterResults", json!({"minSeverity": "warn"}));
        let open = id_of(&app, "check.outlines.openPath", "B");
        let w = Window::new();
        w.click(&mut app, row_id(&open));
        app.state.panels.font_audit.reason = "a stroke glyph".into();
        w.click(&mut app, control_id("ignore"));
        assert!(!has(&app, "check.outlines.openPath", "B"), "the ignored result is hidden");
        let out = call(&mut app, "audit.filterResults", json!({"showIgnored": true}));
        let row = out["rows"].as_array().unwrap().iter().find(|r| r["check"] == "check.outlines.openPath").cloned().unwrap();
        assert_eq!(row["ignored"], true);
        let entries = app.session.query("audit.listIgnored", &json!({})).unwrap();
        assert_eq!(entries["entries"][0]["reason"], "a stroke glyph");

        let ignored = row["id"].as_str().unwrap().to_string();
        w.click(&mut app, row_id(&ignored));
        w.click(&mut app, control_id("unignore"));
        assert!(has(&app, "check.outlines.openPath", "B"), "Stop Ignoring lists it again");
    }

    #[test]
    fn audit_panel_run_in_steps_shows_progress_and_audit_panel_continue_lists_what_audit_run_returns() {
        let mut app = audited_app();
        let whole = app.session.execute("audit.run", &json!({"profile": "outlines", "includeIgnored": true})).unwrap();
        let started = call(&mut app, "audit.panel.run", json!({"profile": "outlines", "inSteps": true}));
        assert_eq!(started["run"], Value::Null, "nothing is listed until the run finishes");
        assert_eq!(started["progress"]["done"], json!(0));
        let total = started["progress"]["total"].as_u64().unwrap();
        let layers = app.session.doc().unwrap().font.glyphs.len() as u64;
        assert_eq!(total, layers + 3, "one layer per glyph and the three later stages");

        // Each frame continues the run by one step; the progress bar and Cancel show meanwhile.
        let w = Window::new();
        w.frame(&mut app, Vec::new());
        assert!(w.ctx.read_response(control_id("cancel")).is_some(), "Cancel shows while the run goes on");
        for _ in 0..8 {
            if app.state.panels.font_audit.progress.is_none() {
                break;
            }
            w.frame(&mut app, Vec::new());
        }
        assert!(app.state.panels.font_audit.progress.is_none(), "the run finished");
        let run = app.state.panels.font_audit.run.clone().unwrap();
        assert!(run.kept && !run.partial);
        assert_eq!(run.items.len(), whole["results"].as_array().unwrap().len(), "the same results as one audit.run");
        let kept = app.session.execute("audit.results", &json!({})).unwrap();
        assert_eq!(kept["runId"].as_str(), run.run_id.as_deref(), "the engine keeps the panel's run");
        assert!(refused(&mut app, "audit.panel.continue", json!({})).contains("no Font Audit run is in progress"));
        w.frame(&mut app, Vec::new());
        assert!(w.ctx.read_response(control_id("run")).is_some(), "Run is back");

        // Reveal names the run it reveals from.
        let extreme = id_of(&app, "check.outlines.missingExtremes", "A");
        let revealed = call(&mut app, "audit.revealIssue", json!({"result": extreme, "runId": run.run_id}));
        assert_eq!(revealed["runId"].as_str(), run.run_id.as_deref());
        assert!(refused(&mut app, "audit.revealIssue", json!({"result": extreme, "runId": "run0"})).contains("lists run"));
    }

    #[test]
    fn audit_panel_cancel_lists_partial_results_and_changes_nothing() {
        let mut app = audited_app();
        let kept = call(&mut app, "audit.panel.run", json!({"profile": "outlines"}))["run"]["runId"].clone();
        let revision = app.session.doc().unwrap().revision;
        call(&mut app, "audit.panel.run", json!({"profile": "outlines", "inSteps": true}));
        // A new font's glyphs start .notdef, space, A: three layers reach A and stop before B.
        let out = call(&mut app, "audit.panel.continue", json!({"budget": 3}));
        assert_eq!(out["progress"]["done"], json!(3));
        let out = call(&mut app, "audit.panel.cancel", json!({}));
        assert_eq!(out["progress"], Value::Null);
        assert_eq!(out["run"]["partial"], json!(true));
        assert_eq!(out["run"]["kept"], json!(false), "partial results are not the engine's kept run");
        let run = app.state.panels.font_audit.run.clone().unwrap();
        assert!(!run.items.is_empty() && run.items.iter().all(|i| i.glyph.as_deref() != Some("B")), "only the first three layers were examined");
        assert_eq!(app.session.execute("audit.results", &json!({})).unwrap()["runId"], kept, "the kept run is unchanged");
        assert_eq!(app.session.doc().unwrap().revision, revision, "the font is unchanged");
        assert!(refused(&mut app, "audit.panel.previewFix", json!({"results": [run.items[0].id]})).contains("engine keeps"));
        assert!(refused(&mut app, "audit.panel.cancel", json!({})).contains("in progress"));
        let w = Window::new();
        w.frame(&mut app, Vec::new());
        w.frame(&mut app, Vec::new());
        assert!(w.ctx.read_response(control_id("run")).is_some());
    }

    #[test]
    fn audit_panel_preview_fix_lists_the_planned_edits_and_apply_makes_them() {
        let mut app = audited_app();
        call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
        let extreme = id_of(&app, "check.outlines.missingExtremes", "A");
        let font = serde_json::to_value(&*app.session.doc().unwrap().font).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        let out = call(&mut app, "audit.panel.previewFix", json!({"results": [extreme]}));
        let planned = out["preview"]["planned"].as_array().unwrap();
        assert!(planned.iter().any(|p| p.as_str().unwrap().contains("path.addExtremes")), "{out}");
        assert_eq!(serde_json::to_value(&*app.session.doc().unwrap().font).unwrap(), font, "a preview changes nothing");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        assert_eq!(app.state.panels.font_audit.preview.as_ref().unwrap().ids, vec![extreme.clone()]);

        let w = Window::new();
        w.click(&mut app, row_id(&extreme));
        w.frame(&mut app, Vec::new());
        assert!(w.ctx.read_response(control_id("applyPreview")).is_some(), "Apply shows with the preview");
        w.click(&mut app, control_id("applyPreview"));
        assert!(!has(&app, "check.outlines.missingExtremes", "A"), "Apply fixed the previewed result");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        assert!(app.state.panels.font_audit.preview.is_none(), "the re-run drops the preview");
    }

    #[test]
    fn audit_panel_export_report_writes_the_listed_run() {
        let mut app = audited_app();
        let listed = call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
        let dir = std::env::temp_dir().join(format!("fontcraft-audit-panel-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let md = dir.join("report.md");
        let out = call(&mut app, "audit.panel.exportReport", json!({"path": md.to_string_lossy()}));
        assert_eq!(out["format"], json!("markdown"));
        assert!(!std::fs::read_to_string(&md).unwrap().is_empty());
        let js = dir.join("report.json");
        call(&mut app, "audit.panel.exportReport", json!({"path": js.to_string_lossy(), "format": "json"}));
        let written: Value = serde_json::from_slice(&std::fs::read(&js).unwrap()).unwrap();
        assert_eq!(written["runId"], listed["run"]["runId"]);
        assert_eq!(written["results"].as_array().unwrap().len(), listed["run"]["results"].as_u64().unwrap() as usize);
        assert!(refused(&mut app, "audit.panel.exportReport", json!({})).contains("`path`"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a save chooser was asked: its title, its extensions and the name it offered.
    type Asked = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<String>, String)>>>;

    /// A save chooser that answers `path` and records what it was asked.
    struct SavePicker {
        path: Option<std::path::PathBuf>,
        asked: Asked,
    }

    impl crate::shell::pickers::Picker for SavePicker {
        fn folder(&mut self, _: &str) -> Option<std::path::PathBuf> {
            None
        }

        fn files(&mut self, _: &str, _: &str, _: &[&str]) -> Vec<std::path::PathBuf> {
            Vec::new()
        }

        fn save(&mut self, title: &str, _: &str, extensions: &[&str], suggested: &str) -> Option<std::path::PathBuf> {
            self.asked.lock().unwrap().push((title.into(), extensions.iter().map(|e| e.to_string()).collect(), suggested.into()));
            self.path.clone()
        }
    }

    #[test]
    fn audit_panel_export_report_choose_fills_the_file_from_the_save_chooser_and_export_writes_there() {
        let mut app = audited_app();
        call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
        app.state.panels.font_audit.export_open = true;
        app.state.panels.font_audit.export_format = "markdown".into();
        let w = Window::new();

        // Without a chooser the file is typed: no Choose….
        w.frame(&mut app, Vec::new());
        w.frame(&mut app, Vec::new());
        assert!(w.ctx.read_response(control_id("exportChoose")).is_none());

        let dir = std::env::temp_dir().join(format!("fontcraft-audit-panel-choose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("chosen.md");
        let asked = Asked::default();

        // A cancelled chooser leaves the field as it was.
        app.picker = Some(Box::new(SavePicker { path: None, asked: asked.clone() }));
        w.click(&mut app, control_id("exportChoose"));
        assert_eq!(app.state.panels.font_audit.export_path, "");

        app.picker = Some(Box::new(SavePicker { path: Some(file.clone()), asked: asked.clone() }));
        w.click(&mut app, control_id("exportChoose"));
        assert_eq!(app.state.panels.font_audit.export_path, file.to_string_lossy());
        let offered = ("Export Report".to_string(), vec!["md".to_string()], "Font Audit Report.md".to_string());
        assert_eq!(*asked.lock().unwrap(), [offered.clone(), offered]);

        w.click(&mut app, control_id("export"));
        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.starts_with('#'), "a Markdown report: {written}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

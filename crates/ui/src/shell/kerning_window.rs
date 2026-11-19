//! The Kerning window's shell commands and view state: the search, the sort, the selected rows, and the questions
//! and kerning dialogs it asks for.
//!
//! | Command | Does |
//! |---|---|
//! | `window.kerning {open?, select?}` | shows or hides the Kerning window; `select` sets the selected rows |
//! | `kerning.filterPairs {term?, match?, kind?, side?, selectedGlyphsOnly?}` | sets the search term and options |
//! | `kerning.resetFilter` | returns the search options to their defaults; the term stays |
//! | `kerning.sortPairs {column, descending?}` | sorts the list by a column |
//! | `kerning.selectNextPair`, `kerning.selectPreviousPair` | steps the selection through the listed rows |
//! | `dialog.renameKerningGroup {side, from, to?, pair, master?, direction?}` | asks whether a group rename covers the font or the pair |
//! | `dialog.pasteKerningConflict {text \| pairs, master?, direction?}` | pastes; when a pasted pair clashes with a stored one, asks what to do |
//! | `dialog.auditKerningExceptions {threshold?, masters?, direction?, fix?}` | runs `kerning.auditExceptions` and shows what it found |
//! | `dialog.matchKerning {run?, mainMaster?, masters?}` | asks for the main master, then runs `kerning.matchAcrossMasters` and shows a summary |
//!
//! The list itself is the projection's (`app::models::kerning_window`): the stored pairs of the font's first master,
//! left to right. The dialogs module (`dialogs::kerning_questions`, `dialogs::kerning_audit`) opens what these
//! commands ask for on its next pass.

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::WordSet as _;
use fontcraft_model::MAX_NAME_LEN;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Longest search term, in bytes: the longest glyph name and an `@`, as `kerning.listPairs` takes it.
pub const MAX_TERM_BYTES: usize = MAX_NAME_LEN + 1;
/// Most pairs `window.kerning {select}` takes.
const MAX_SELECT: usize = 500_000;
/// Longest pasted text, in bytes (the kerning-text limit).
const MAX_PASTE_BYTES: usize = 16 * 1024 * 1024;
/// Most audit findings, conflicts and match changes a dialog lists.
pub const MAX_LISTED: usize = 1000;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.kerning",
        label: "Kerning",
        menu: &["Window"],
        shortcut: Some("Cmd+Alt+K"),
        params: "{open?: bool (default true), select?: [{left: name|\"@group\", right: name|\"@group\"}]} show or hide the Kerning window: the stored pairs of the font's first master, left to right. select sets the selected rows; rows not listed are dropped. ui.inspect reports the window under kerningWindow",
    },
    UiCommand {
        id: "kerning.filterPairs",
        label: "Filter Kerning Pairs",
        menu: &[],
        shortcut: None,
        params: "{term?: string, match?: \"contains\"|\"is\"|\"beginsWith\"|\"endsWith\"|\"doesNotContain\", kind?: \"group\"|\"notGroup\"|null, side?: \"left\"|\"right\"|null, selectedGlyphsOnly?: bool} set the Kerning window's search term and options; null clears kind or side; options not given keep their value. A term starting with @ is compared with the @ spelling of groups",
    },
    UiCommand {
        id: "kerning.resetFilter",
        label: "Reset",
        menu: &[],
        shortcut: None,
        params: "{} return the Kerning window's search options to their defaults (contains, every entry kind, both sides, not limited to the Font View selection); the term stays",
    },
    UiCommand {
        id: "kerning.sortPairs",
        label: "Sort Kerning Pairs",
        menu: &[],
        shortcut: None,
        params: "{column: \"left\"|\"value\"|\"right\", descending?: bool (default false)} sort the Kerning window by a column; ties keep the stored order",
    },
    UiCommand {
        id: "kerning.selectNextPair",
        label: "Next Kerning Pair",
        menu: &[],
        shortcut: None,
        params: "{} select the Kerning window's row after the current one (none: the first; at the last: it stays)",
    },
    UiCommand {
        id: "kerning.selectPreviousPair",
        label: "Previous Kerning Pair",
        menu: &[],
        shortcut: None,
        params: "{} select the Kerning window's row before the current one (none: the last; at the first: it stays)",
    },
    UiCommand {
        id: "dialog.renameKerningGroup",
        label: "Rename Kerning Group",
        menu: &[],
        shortcut: None,
        params: "{side: \"left\"|\"right\", from: group, to?: group (default from), pair: {left, right}, master?: id, direction?: \"ltr\"|\"rtl\"|\"vertical\"} ask whether to rename the group in the whole font (All Pairs) or only for the pair (This Pair); the answer runs kerning.renameGroup with scope font or pair, Cancel renames nothing",
    },
    UiCommand {
        id: "dialog.pasteKerningConflict",
        label: "Paste Kerning: existing pairs",
        menu: &[],
        shortcut: None,
        params: "{text?: kerning-text, pairs?: [{left, right, value}], master?: id, direction?: \"ltr\"|\"rtl\"|\"vertical\"} paste the pairs as kerning.pastePairs does with onConflict ask; when pasted pairs exist with other values nothing is pasted and the question lists them: Overwrite and Keep Existing paste again with that answer, Cancel pastes nothing",
    },
    UiCommand {
        id: "dialog.auditKerningExceptions",
        label: "Audit Kerning Exceptions…",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{threshold?: units (default 0), masters?: [ids] (default every master), direction?: \"ltr\"|\"rtl\" (default ltr), fix?: bool (default false)} run kerning.auditExceptions and list the glyph pairs class kerning brings closer than the threshold; Add Exceptions runs it again with fix",
    },
    UiCommand {
        id: "dialog.matchKerning",
        label: "Match Kerning…",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{run?: bool (default false), mainMaster?: id, masters?: [ids]} without run, ask for the main master; with run, run kerning.matchAcrossMasters and show how many pairs each master had before and after",
    },
];

craft::ui::words! {
    /// The status notices of the Kerning window's commands.
    pub struct KerningNoticeWords {
        pasted: "fontcraftKerningNotice.pasted" = "Pasted kerning pairs:";
    }
}

/// A column of the pair list; its id is the `column` of `kerning.sortPairs`.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Column {
    #[default]
    Left,
    Value,
    Right,
}

impl Column {
    pub const ALL: [Column; 3] = [Column::Left, Column::Value, Column::Right];

    pub fn id(self) -> &'static str {
        match self {
            Column::Left => "left",
            Column::Value => "value",
            Column::Right => "right",
        }
    }

    pub fn of_id(id: &str) -> Option<Column> {
        Column::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// The sorted column and its direction.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PairSort {
    pub column: Column,
    pub descending: bool,
}

/// The match modes, as `kerning.listPairs` names them.
pub const MATCHES: [&str; 5] = ["contains", "is", "beginsWith", "endsWith", "doesNotContain"];
/// The entry kinds.
pub const KINDS: [&str; 2] = ["group", "notGroup"];
/// The searched sides.
pub const SIDES: [&str; 2] = ["left", "right"];

/// The search term and options.
#[derive(PlainData, Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub term: String,
    /// One of [`MATCHES`].
    pub matching: String,
    /// One of [`KINDS`]; `None`: every entry.
    pub kind: Option<String>,
    /// One of [`SIDES`]; `None`: both sides.
    pub side: Option<String>,
    /// Only pairs with a glyph selected in Font View, or a group one of them is in.
    pub selected_glyphs_only: bool,
}

impl Default for Search {
    fn default() -> Self {
        Search { term: String::new(), matching: "contains".into(), kind: None, side: None, selected_glyphs_only: false }
    }
}

/// A row's name: its sides joined by a tab, which no side may hold.
pub fn row_id(left: &str, right: &str) -> String {
    format!("{left}\t{right}")
}

/// The sides of a row id.
pub fn split_id(id: &str) -> (&str, &str) {
    id.split_once('\t').unwrap_or((id, ""))
}

/// One pasted pair the master stores with another value.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub left: String,
    pub right: String,
    pub value: f64,
    pub existing: f64,
}

/// A question the window asked and the dialogs module has not opened yet.
#[derive(Clone, Debug, PartialEq)]
pub enum Question {
    /// Rename Kerning Group, with `kerning.renameGroup`'s parameters but `scope`.
    Rename(Map<String, Value>),
    /// Paste Kerning: the paste's parameters but `onConflict`, the conflicts listed and how many there are.
    Paste { params: Map<String, Value>, conflicts: Vec<Conflict>, count: usize },
}

/// One master as the kerning dialogs offer it: id and name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterChoice {
    pub id: String,
    pub name: String,
}

/// One glyph pair the audit found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Finding {
    pub master: String,
    pub left: String,
    pub right: String,
    pub class_left: String,
    pub class_right: String,
    pub value: f64,
    pub distance: f64,
    /// The exception added for it, after a run with `fix`.
    pub fix: Option<f64>,
}

/// The audit dialog's settings and the last run's report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Audit {
    pub threshold: f64,
    /// Empty: every master.
    pub masters: Vec<String>,
    /// "ltr" or "rtl".
    pub direction: String,
    /// The font's masters, for the master choice.
    pub choices: Vec<MasterChoice>,
    pub findings: Vec<Finding>,
    /// How many pairs the run found; more than `findings` holds when the list was cut.
    pub found: usize,
    pub measured: usize,
    pub skipped: usize,
    /// The run added exceptions.
    pub fixed: bool,
    /// Moves with every run, so an open dialog takes the new report.
    pub run: u64,
}

/// What Match Kerning changed in one master.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchedMaster {
    pub name: String,
    pub before: usize,
    pub after: usize,
}

/// The Match Kerning dialog: the masters to choose the main one from, and the summary once it ran.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matching {
    pub choices: Vec<MasterChoice>,
    /// `None` until the command ran.
    pub summary: Option<MatchSummary>,
    pub run: u64,
}

/// The result of `kerning.matchAcrossMasters`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchSummary {
    pub masters: Vec<MatchedMaster>,
    pub filled: usize,
    pub exceptions: usize,
    /// Masters left alone, with the reason.
    pub skipped: Vec<String>,
}

/// The window's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KerningView {
    pub search: Search,
    /// `None`: the stored order.
    pub sort: Option<PairSort>,
    /// Selected rows by [`row_id`], in the order they were picked.
    pub selected: Vec<String>,
    /// The row the steps go from: the one picked last.
    pub current: Option<String>,
    /// The question asked and not yet opened.
    pub question: Option<Question>,
    /// The audit dialog's state while it is asked for or open.
    pub audit: Option<Audit>,
    /// The Match Kerning dialog's state while it is asked for or open.
    pub matching: Option<Matching>,
}

/// Whether the Kerning window shows.
pub fn shows(app: &FontCraftApp) -> bool {
    app.view.workspace.layout.panel_state(&PanelId::KerningWindow) != craft::behavior::dock::PanelState::Hidden
}

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = command.params.as_object().cloned().unwrap_or_default();
    let p = &params;
    match command.id.as_str() {
        "window.kerning" => window(app, p),
        "kerning.filterPairs" => filter(app, p),
        "kerning.resetFilter" => {
            let search = &mut app.view.kerning.search;
            *search = Search { term: std::mem::take(&mut search.term), ..Search::default() };
            Ok(())
        }
        "kerning.sortPairs" => {
            let name = p.get("column").and_then(Value::as_str).ok_or("`column` (\"left\", \"value\" or \"right\") is required")?;
            let column = Column::of_id(name).ok_or_else(|| format!("`column` must be left, value or right, got \"{}\"", quoted(name)))?;
            let descending = flag(p, "descending")?.unwrap_or(false);
            app.view.kerning.sort = Some(PairSort { column, descending });
            Ok(())
        }
        "kerning.selectNextPair" => step(app, true),
        "kerning.selectPreviousPair" => step(app, false),
        "dialog.renameKerningGroup" => rename(app, p),
        "dialog.pasteKerningConflict" => paste(app, p),
        "dialog.auditKerningExceptions" => audit(app, p),
        "dialog.matchKerning" => matching(app, p),
        other => Err(format!("no Kerning window command `{other}`")),
    }
}

fn flag(p: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// `[{left, right}]` as row ids, at most [`MAX_SELECT`].
fn pairs_arg(p: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>, String> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| format!("`{key}` must be a list of {{left, right}}"))?;
    if list.len() > MAX_SELECT {
        return Err(format!("`{key}` holds at most {MAX_SELECT} pairs"));
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let side = |name: &str| item.get(name).and_then(Value::as_str);
        let (Some(left), Some(right)) = (side("left"), side("right")) else {
            return Err(format!("`{key}[{i}]` must be an object with left and right"));
        };
        out.push(row_id(left, right));
    }
    Ok(Some(out))
}

fn window(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let open = flag(p, "open")?.unwrap_or(true);
    let select = pairs_arg(p, "select")?;
    // A refused move (the layout has no room) leaves the panel where it was.
    let _ = app.view.workspace.set_visible(PanelId::KerningWindow, open, None, false, |panel| panel.permissions());
    if let Some(ids) = select {
        let kerning = &mut app.view.kerning;
        kerning.current = ids.last().cloned();
        kerning.selected = ids;
    }
    Ok(())
}

/// Reads one of `options` at `key`; `Some(None)` for an explicit null.
fn option(p: &Map<String, Value>, key: &str, options: &[&str]) -> Result<Option<Option<String>>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(v)) if options.contains(&v.as_str()) => Ok(Some(Some(v.clone()))),
        Some(v) => Err(format!("`{key}` must be {}, got {}", options.join(", "), quoted(&v.to_string()))),
    }
}

fn filter(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let term = match p.get("term") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t.len() > MAX_TERM_BYTES => return Err(format!("`term` may be at most {MAX_TERM_BYTES} bytes long")),
        Some(Value::String(t)) if t.chars().any(char::is_control) => return Err("`term` may not hold control characters".into()),
        Some(Value::String(t)) => Some(t.clone()),
        Some(_) => return Err("`term` must be a string".into()),
    };
    let matching = option(p, "match", &MATCHES)?;
    if matches!(matching, Some(None)) {
        return Err("`match` may not be null".into());
    }
    let kind = option(p, "kind", &KINDS)?;
    let side = option(p, "side", &SIDES)?;
    let only = flag(p, "selectedGlyphsOnly")?;
    let s = &mut app.view.kerning.search;
    if let Some(t) = term {
        s.term = t;
    }
    if let Some(Some(m)) = matching {
        s.matching = m;
    }
    if let Some(k) = kind {
        s.kind = k;
    }
    if let Some(v) = side {
        s.side = v;
    }
    if let Some(o) = only {
        s.selected_glyphs_only = o;
    }
    Ok(())
}

fn step(app: &mut FontCraftApp, forward: bool) -> Result<(), String> {
    let model = &app.models.kerning_window;
    let last = model.rows.len().checked_sub(1).ok_or("the Kerning window lists no pairs")?;
    let at = match model.current {
        Some(i) if forward => (i + 1).min(last),
        Some(i) => i.saturating_sub(1),
        None if forward => 0,
        None => last,
    };
    let row = model.rows.get(at).ok_or("the Kerning window lists no pairs")?;
    let id = row_id(&row.left, &row.right);
    app.view.kerning.selected = vec![id.clone()];
    app.view.kerning.current = Some(id);
    Ok(())
}

fn rename(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    app.session.font().map_err(|e| e.to_string())?;
    let text = |key: &str| p.get(key).and_then(Value::as_str);
    if !matches!(text("side"), Some("left" | "right")) {
        return Err("`side` must be \"left\" or \"right\"".into());
    }
    let from = text("from").map(|f| f.strip_prefix('@').unwrap_or(f)).filter(|f| !f.is_empty()).ok_or("`from` names the group to rename")?;
    let pair = p.get("pair").filter(|v| v.get("left").and_then(Value::as_str).is_some() && v.get("right").and_then(Value::as_str).is_some());
    let pair = pair.ok_or("`pair` must be an object with left and right")?;
    let mut params = Map::new();
    params.insert("side".into(), json!(text("side")));
    params.insert("from".into(), json!(from));
    params.insert("to".into(), json!(text("to").map_or(from, |t| t.strip_prefix('@').unwrap_or(t))));
    params.insert("pair".into(), pair.clone());
    for key in ["master", "direction"] {
        if let Some(v) = p.get(key) {
            params.insert(key.into(), v.clone());
        }
    }
    app.view.kerning.question = Some(Question::Rename(params));
    Ok(())
}

fn paste(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    if p.get("text").and_then(Value::as_str).is_some_and(|t| t.len() > MAX_PASTE_BYTES) {
        return Err(format!("the pasted text holds more than {MAX_PASTE_BYTES} bytes"));
    }
    let mut params = p.clone();
    params.remove("onConflict");
    let mut ask = params.clone();
    ask.insert("onConflict".into(), json!("ask"));
    let reply = app.session.execute("kerning.pastePairs", &Value::Object(ask)).map_err(|e| e.to_string())?;
    if reply.get("asked").and_then(Value::as_bool) != Some(true) {
        let added = reply.get("added").and_then(Value::as_u64).unwrap_or(0);
        app.status = format!("{} {added}", KerningNoticeWords::english().pasted.as_str());
        return Ok(());
    }
    let listed = reply.get("conflicts").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let conflicts: Vec<Conflict> = listed
        .iter()
        .take(MAX_LISTED)
        .map(|c| {
            let text = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let number = |k: &str| c.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
            Conflict { left: text("left"), right: text("right"), value: number("value"), existing: number("existing") }
        })
        .collect();
    let count = reply.get("conflictCount").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(conflicts.len());
    app.view.kerning.question = Some(Question::Paste { params, conflicts, count });
    Ok(())
}

/// The font's masters as the dialogs offer them.
fn master_choices(app: &FontCraftApp) -> Result<Vec<MasterChoice>, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    Ok(font.masters.iter().map(|m| MasterChoice { id: m.id.clone(), name: m.name.clone() }).collect())
}

fn ids_arg(p: &Map<String, Value>, key: &str, choices: &[MasterChoice]) -> Result<Vec<String>, String> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| format!("`{key}` must be a list of master ids"))?;
    if list.len() > choices.len() {
        return Err(format!("`{key}` names more masters than the font has"));
    }
    list.iter().map(|id| id.as_str().map(str::to_string).ok_or_else(|| format!("`{key}` must be a list of master ids"))).collect()
}

fn audit(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let choices = master_choices(app)?;
    let threshold = match p.get("threshold") {
        None | Some(Value::Null) => 0.0,
        Some(v) => v.as_f64().filter(|t| t.is_finite()).ok_or("`threshold` must be a number")?,
    };
    let direction = match p.get("direction").and_then(Value::as_str) {
        None => "ltr",
        Some(d @ ("ltr" | "rtl")) => d,
        Some(d) => return Err(format!("`direction` must be ltr or rtl, got \"{}\"", quoted(d))),
    };
    let masters = ids_arg(p, "masters", &choices)?;
    let fix = flag(p, "fix")?.unwrap_or(false);
    let mut params = Map::new();
    params.insert("threshold".into(), json!(threshold));
    params.insert("direction".into(), json!(direction));
    params.insert("fix".into(), json!(fix));
    if !masters.is_empty() {
        params.insert("masters".into(), json!(masters));
    }
    let reply = app.session.execute("kerning.auditExceptions", &Value::Object(params)).map_err(|e| e.to_string())?;
    let found = reply.get("pairs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let name = |id: &str| choices.iter().find(|m| m.id == id).map_or_else(|| id.to_string(), |m| m.name.clone());
    let findings = found
        .iter()
        .take(MAX_LISTED)
        .map(|f| {
            let text = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let number = |k: &str| f.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
            Finding {
                master: name(&text("master")),
                left: text("left"),
                right: text("right"),
                class_left: text("classLeft"),
                class_right: text("classRight"),
                value: number("value").unwrap_or(0.0),
                distance: number("distance").unwrap_or(0.0),
                fix: number("fix"),
            }
        })
        .collect();
    let count = |k: &str| reply.get(k).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let skipped = reply.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
    let run = app.view.kerning.audit.as_ref().map_or(0, |a| a.run) + 1;
    app.view.kerning.audit = Some(Audit {
        threshold,
        masters,
        direction: direction.to_string(),
        choices,
        findings,
        found: found.len(),
        measured: count("measured"),
        skipped,
        fixed: fix,
        run,
    });
    Ok(())
}

fn matching(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let choices = master_choices(app)?;
    let run = app.view.kerning.matching.as_ref().map_or(0, |m| m.run) + 1;
    if !flag(p, "run")?.unwrap_or(false) {
        app.view.kerning.matching = Some(Matching { choices, summary: None, run });
        return Ok(());
    }
    let mut params = Map::new();
    if let Some(main) = p.get("mainMaster").and_then(Value::as_str) {
        params.insert("mainMaster".into(), json!(main));
    }
    let masters = ids_arg(p, "masters", &choices)?;
    if !masters.is_empty() {
        params.insert("masters".into(), json!(masters));
    }
    let reply = app.session.execute("kerning.matchAcrossMasters", &Value::Object(params)).map_err(|e| e.to_string())?;
    let changes = reply.get("pairs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let kind = |k: &str| changes.iter().filter(|c| c.get("kind").and_then(Value::as_str) == Some(k)).count();
    let counts = reply.get("masters").and_then(Value::as_object);
    let number = |v: Option<&Value>, k: &str| v.and_then(|m| m.get(k)).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let masters = choices
        .iter()
        .filter_map(|m| {
            let at = counts?.get(&m.id)?;
            Some(MatchedMaster { name: m.name.clone(), before: number(Some(at), "before"), after: number(Some(at), "after") })
        })
        .collect();
    let skipped = reply
        .get("skipped")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_LISTED)
                .map(|s| {
                    let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or_default();
                    format!("{}: {}", choices.iter().find(|m| m.id == text("master")).map_or(text("master"), |m| m.name.as_str()), text("reason"))
                })
                .collect()
        })
        .unwrap_or_default();
    let summary = MatchSummary { masters, filled: kind("filled"), exceptions: kind("exception"), skipped };
    app.view.kerning.matching = Some(Matching { choices, summary: Some(summary), run });
    Ok(())
}

/// The window as `ui.inspect` reports it, under `kerningWindow`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let model = &app.models.kerning_window;
    let kerning = &app.view.kerning;
    let pair = |id: &String| {
        let (left, right) = split_id(id);
        json!({"left": left, "right": right})
    };
    let search = &kerning.search;
    state.insert(
        "kerningWindow".into(),
        json!({
            "open": shows(app),
            "master": model.master,
            "source": model.source,
            "direction": model.direction,
            "total": model.total,
            "count": model.rows.len(),
            "search": {
                "term": search.term,
                "match": search.matching,
                "kind": search.kind,
                "side": search.side,
                "selectedGlyphsOnly": search.selected_glyphs_only,
            },
            "sort": kerning.sort.map(|s| json!({"column": s.column.id(), "descending": s.descending})),
            "selected": model.selected.iter().filter_map(|&i| model.rows.get(i)).map(|r| pair(&row_id(&r.left, &r.right))).collect::<Vec<_>>(),
            "current": kerning.current.as_ref().map(pair),
            "error": model.error,
        }),
    );
}

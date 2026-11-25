//! The Kerning window's shell commands and view state: the search, the sort, the selected rows, and the questions
//! and kerning dialogs it asks for.
//!
//! | Command | Does |
//! |---|---|
//! | `window.kerning {open?, select?, direction?, edit?}` | shows or hides the Kerning window; `select` sets the selected rows, `direction` the writing direction it lists, `edit` the cell edited in place |
//! | `kerning.filterPairs {term?, match?, kind?, side?, selectedGlyphsOnly?}` | sets the search term and options |
//! | `kerning.resetFilter` | returns the search options to their defaults; the term stays |
//! | `kerning.sortPairs {column, descending?}` | sorts the list by a column |
//! | `kerning.selectNextPair`, `kerning.selectPreviousPair` | steps the selection through the listed rows |
//! | `kerning.showAllGlyphs {left?, right?}` | puts every glyph pair the current row covers into the edit view's text |
//! | `kerning.makeException {left?, right?}` | opens the group locks of the glyph pair the current row stands for |
//! | `dialog.renameKerningGroup {side, from, to?, pair, master?, direction?}` | asks whether a group rename covers the font or the pair |
//! | `dialog.pasteKerningConflict {text? \| pairs?, master?, direction?}` | pastes (neither: the clipboard's text); when a pasted pair clashes with a stored one, asks what to do |
//! | `dialog.auditKerningExceptions {threshold?, masters?, direction?, fix?}` | runs `kerning.auditExceptions` and shows what it found |
//! | `dialog.autoKern`, `dialog.kernToDistance`, `dialog.removeKerningPairs`, `dialog.verticalKerning` | open `dialogs::kerning_batch` |
//! | `dialog.matchKerning {run?, mainMaster?, masters?}` | asks for the main master, then runs `kerning.matchAcrossMasters` and shows a summary |
//!
//! The list itself is the projection's (`app::models::kerning_window`): the stored pairs of the current master,
//! in the writing direction of the Edit View tab or the one chosen in the window. The dialogs module (`dialogs::kerning_questions`, `dialogs::kerning_audit`) opens what these
//! commands ask for on its next pass.

use std::sync::Arc;

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::WordSet as _;
use fontcraft_engine::model::Font;
use fontcraft_model::MAX_NAME_LEN;
use fontcraft_spacing::kerning::{Position, facing, members};
use fontcraft_textlayout::{Direction, Item, MAX_ITEMS};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use crate::app::FontCraftApp;
use crate::app::models::kerning_window::AllRows;

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
        params: "{open?: bool (default true), select?: [{left: name|\"@group\", right: name|\"@group\"}] or \"all\", current?: {left, right} (with select \"all\"), direction?: \"ltr\"|\"rtl\"|\"vertical\"|null, edit?: {left, right, column: \"value\"|\"right\"}|null} show or hide the Kerning window: the stored pairs of the current master (view.selectMaster) in the writing direction of the Edit View tab. direction lists another one until null follows the tab again. select sets the selected rows (\"all\": every listed row of the window that shows) and shows the last one, with \"all\" the current one, in Edit View; a pair that cannot be shown stays selected and the status line says why; rows not listed are dropped. edit opens a row's value or right group for editing in the list, null closes it. ui.inspect reports the window under kerningWindow",
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
        id: "kerning.showAllGlyphs",
        label: "Show All Glyphs",
        menu: &[],
        shortcut: None,
        params: "{left?: name|\"@group\", right?: name|\"@group\" (default: the Kerning window's current row)} put every glyph pair the row covers into the edit view's text in place of the pair at the current glyph: each group side expands to all its members in glyph order, one line per left glyph, its pairs separated by the space glyph when the font has one; at most as many items as the text holds",
    },
    UiCommand {
        id: "kerning.makeException",
        label: "Make Kerning Exception",
        menu: &[],
        shortcut: None,
        params: "{left?: name|\"@group\", right?: name|\"@group\" (default: the Kerning window's current row)} store the kerning of one glyph pair the row covers as an exception (kerning.addException), in the master and direction the window lists: the pair at the edit view's cursor when the row covers it, else the first member of each group. A row of two glyphs is refused",
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
        params: "{text?: kerning-text, pairs?: [{left, right, value}] (neither: the clipboard's text, into the master and direction the Kerning window lists), master?: id, direction?: \"ltr\"|\"rtl\"|\"vertical\"} paste the pairs as kerning.pastePairs does with onConflict ask; when pasted pairs exist with other values nothing is pasted and the question lists them: Overwrite and Keep Existing paste again with that answer, Cancel pastes nothing",
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
    UiCommand {
        id: "dialog.autoKern",
        label: "Auto Kern…",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{} ask where the pairs come from (the Kerning window's selected rows, the Font View selection with itself or a typed list, the Pairs & Phrases list, every stored pair) and the options of kerning.autoKern; Preview lists what it would store, Apply stores the ticked rows in one undo step",
    },
    UiCommand {
        id: "dialog.kernToDistance",
        label: "Kern to Distance…",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{} ask for the pairs, as dialog.autoKern does, and the distance; Preview lists what kerning.kernToDistance would store, Apply stores the ticked rows in one undo step",
    },
    UiCommand {
        id: "dialog.removeKerningPairs",
        label: "Remove Pairs…",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{} ask for the rules of kerning.removePairsByRule; Preview lists and counts the pairs, Apply deletes them",
    },
    UiCommand {
        id: "dialog.verticalKerning",
        label: "Vertical Kerning from Horizontal…",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{} ask for the masters and say how many vertical pairs kerning.verticalFromHorizontal deletes before it runs",
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
    /// The writing direction chosen in the window; `None`: the Edit View tab's.
    pub direction: Option<Direction>,
    /// Selected rows by [`row_id`], in the order they were picked.
    pub selected: Arc<Vec<String>>,
    /// Select All: every row of this list is selected, whatever `selected` holds.
    pub all: Option<AllRows>,
    /// The cell edited in place: the row by [`row_id`], and its Value or Right column.
    pub editing: Option<(String, Column)>,
    /// The document the cell was opened in: another one may list a row of the same name.
    pub editing_in: Option<u64>,
    /// The row the steps go from: the one picked last.
    pub current: Option<String>,
    /// The question asked and not yet opened.
    pub question: Option<Question>,
    /// The audit dialog's state while it is asked for or open.
    pub audit: Option<Audit>,
    /// The Match Kerning dialog's state while it is asked for or open.
    pub matching: Option<Matching>,
    /// The batch dialog asked for and not yet opened.
    pub batch: Option<Batch>,
    /// The batch dialog that is open.
    pub batch_open: Option<Batch>,
}

/// A kerning dialog over many pairs (`dialogs::kerning_batch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Batch {
    AutoKern,
    Distance,
    Remove,
    Vertical,
}

fn batch(app: &mut FontCraftApp, kind: Batch) -> Result<(), String> {
    app.session.font().map_err(|e| e.to_string())?;
    let kerning = &mut app.view.kerning;
    if kerning.batch.or(kerning.batch_open).is_some_and(|other| other != kind) {
        return Err("another kerning dialog is open: close it first".into());
    }
    kerning.batch = Some(kind);
    Ok(())
}

/// The master whose kerning the window lists, whether it shows or not.
pub fn listed_master(app: &FontCraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    crate::shell::masters::current_master(font, &app.view.layers).map(|m| m.id.clone())
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
            app.view.kerning.editing = None;
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
        "kerning.showAllGlyphs" => show_all(app, p),
        "kerning.makeException" => make_exception(app, p),
        "dialog.renameKerningGroup" => rename(app, p),
        "dialog.pasteKerningConflict" => paste(app, p),
        "dialog.auditKerningExceptions" => audit(app, p),
        "dialog.matchKerning" => matching(app, p),
        "dialog.autoKern" => batch(app, Batch::AutoKern),
        "dialog.kernToDistance" => batch(app, Batch::Distance),
        "dialog.removeKerningPairs" => batch(app, Batch::Remove),
        "dialog.verticalKerning" => batch(app, Batch::Vertical),
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
    let direction = match p.get("direction") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(Some(v.as_str().and_then(Direction::from_name).ok_or("`direction` must be ltr, rtl, vertical or null")?)),
    };
    let all = p.get("select").and_then(Value::as_str) == Some("all");
    let select = if all { None } else { pairs_arg(p, "select")? };
    let current = match p.get("current") {
        None | Some(Value::Null) => None,
        Some(_) if !all => return Err("`current` goes with `select: \"all\"`".into()),
        Some(pair) => {
            let side = |key: &str| pair.get(key).and_then(Value::as_str).ok_or("`current` must be an object with left and right");
            Some(row_id(side("left")?, side("right")?))
        }
    };
    let scope = app.models.kerning_window.scope.clone();
    if all && scope.is_none() {
        return Err("the Kerning window lists no pairs: show it first".into());
    }
    let edit = match p.get("edit") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(cell) => {
            let text = |key: &str| cell.get(key).and_then(Value::as_str).ok_or("`edit` must be {left, right, column} or null");
            let column = match Column::of_id(text("column")?) {
                Some(column @ (Column::Value | Column::Right)) => column,
                _ => return Err("`edit.column` must be value or right".into()),
            };
            Some(Some((row_id(text("left")?, text("right")?), column)))
        }
    };
    // A cell stays open only in the list it was opened in.
    if !open || direction.is_some() || all || select.is_some() {
        app.view.kerning.editing = None;
    }
    if let Some(edit) = edit {
        app.view.kerning.editing = edit.filter(|_| open);
        app.view.kerning.editing_in = app.session.active().map(|d| d.id);
    }
    if let Some(direction) = direction {
        app.view.kerning.direction = direction;
        app.view.kerning.all = None;
    }
    crate::shell::dock::show_window(&mut app.view.workspace, PanelId::KerningWindow, open, app.view.canvas.rect);
    if all {
        let kerning = &mut app.view.kerning;
        kerning.all = scope;
        kerning.selected = Arc::default();
        if current.is_some() {
            kerning.current = current;
            show_current(app);
        }
    }
    if let Some(ids) = select {
        let kerning = &mut app.view.kerning;
        kerning.current = ids.last().cloned();
        kerning.selected = Arc::new(ids);
        kerning.all = None;
        show_current(app);
    }
    Ok(())
}

/// Shows the current row's pair in the edit view. An obsolete pair has nothing to show: the row stays selected and the
/// status line says why.
fn show_current(app: &mut FontCraftApp) {
    let Some(id) = app.view.kerning.current.clone() else { return };
    let (left, right) = split_id(&id);
    let direction = crate::app::models::kerning_window::direction(&app.view);
    if let Err(error) = crate::canvas::text::commands::show_pair_of(app, left, right, Some(direction)) {
        app.status = error;
    }
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
    app.view.kerning.editing = None;
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
    let id = row_id(row.left, row.right);
    let kerning = &mut app.view.kerning;
    kerning.selected = Arc::new(vec![id.clone()]);
    kerning.current = Some(id);
    kerning.all = None;
    kerning.editing = None;
    show_current(app);
    Ok(())
}

/// The glyphs a pair side covers: the glyph itself, or every member of the group on the side facing the pair.
fn side_glyphs(font: &Font, direction: Direction, position: Position, side: &str) -> Result<Vec<String>, String> {
    match side.strip_prefix('@') {
        None => font.glyph(side).map(|g| vec![g.name.clone()]).ok_or_else(|| format!("the font has no glyph `{}`", quoted(side))),
        Some(group) => {
            let group_side = facing(direction, position);
            let found: Vec<String> = members(font, group_side, group).into_iter().map(str::to_string).collect();
            if found.is_empty() {
                return Err(format!("no glyph has the {} kerning group `{}`", group_side.name(), quoted(group)));
            }
            Ok(found)
        }
    }
}

/// The row `left` and `right` name, else the current one.
fn row_arg(app: &FontCraftApp, p: &Map<String, Value>) -> Result<(String, String), String> {
    let side = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` must be a glyph name or @group")),
    };
    match (side("left")?, side("right")?) {
        (Some(l), Some(r)) => Ok((l, r)),
        (None, None) => {
            let current = app.view.kerning.current.as_ref().ok_or("select a pair in the Kerning window, or give left and right")?;
            let (l, r) = split_id(current);
            Ok((l.to_string(), r.to_string()))
        }
        _ => Err("give both `left` and `right`, or neither".into()),
    }
}

fn make_exception(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let (left, right) = row_arg(app, p)?;
    let which = match (left.starts_with('@'), right.starts_with('@')) {
        (true, true) => "both",
        (true, false) => "left",
        (false, true) => "right",
        (false, false) => return Err("the pair already names two glyphs".into()),
    };
    let direction = crate::app::models::kerning_window::direction(&app.view);
    let font = app.session.font().map_err(|e| e.to_string())?;
    let lefts = side_glyphs(font, direction, Position::First, &left)?;
    let rights = side_glyphs(font, direction, Position::Second, &right)?;
    let text = &app.view.text;
    let at_cursor = text.edited_index().and_then(|i| Some((text.item_glyph(i.checked_sub(1)?)?.to_string(), text.item_glyph(i)?.to_string())));
    let (l, r) = match at_cursor.filter(|(l, r)| lefts.contains(l) && rights.contains(r)) {
        Some(shown) => shown,
        None => (lefts.first().cloned().unwrap_or_default(), rights.first().cloned().unwrap_or_default()),
    };
    let mut params = Map::new();
    for (key, value) in [("left", l.as_str()), ("right", r.as_str()), ("side", which), ("direction", direction.name())] {
        params.insert(key.into(), json!(value));
    }
    if let Some(master) = listed_master(app) {
        params.insert("master".into(), json!(master));
    }
    app.run_command("kerning.addException", Value::Object(params)).map(|_| ())
}

fn show_all(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let (left, right) = row_arg(app, p)?;
    let direction = crate::app::models::kerning_window::direction(&app.view);
    let font = app.session.font().map_err(|e| e.to_string())?;
    // Both sides are read before a tab comes to the front, so a refused row changes nothing.
    let lefts = side_glyphs(font, direction, Position::First, &left)?;
    let rights = side_glyphs(font, direction, Position::Second, &right)?;
    let space = font.glyphs.iter().find(|g| g.unicodes.contains(&0x20)).map(|g| g.name.clone());
    // Two glyphs a pair, a space between pairs, a line break between lines.
    let per_line = rights.len().saturating_mul(2).saturating_add(if space.is_some() { rights.len().saturating_sub(1) } else { 0 });
    let count = lefts.len().saturating_mul(per_line).saturating_add(lefts.len().saturating_sub(1));
    if count > MAX_ITEMS {
        let pairs = lefts.len().saturating_mul(rights.len());
        return Err(format!("the row covers {pairs} glyph pairs, {count} items; the text holds at most {MAX_ITEMS}"));
    }
    let opened = !crate::shell::tabs::bring_edit_view(app);
    if opened {
        crate::shell::tabs::open_glyphs(app, lefts.get(..1).unwrap_or_default())?;
        crate::canvas::text::set_direction(app, direction);
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    if !crate::canvas::text::shows_text(font, &app.view) {
        crate::canvas::text::hold_shown_glyph(app);
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let item = |name: &str| crate::canvas::text::commands::glyph_item(font, name);
    let mut items = Vec::with_capacity(count);
    for (i, l) in lefts.iter().enumerate() {
        if i > 0 {
            items.push(Item::LineBreak);
        }
        for (j, r) in rights.iter().enumerate() {
            if j > 0
                && let Some(s) = &space
            {
                items.push(item(s));
            }
            items.push(item(l));
            items.push(item(r));
        }
    }
    let mut next = app.view.text.clone();
    let range = match next.edited_index() {
        _ if opened => 0..next.text.len(),
        Some(i) if i > 0 && next.item_glyph(i - 1).is_some() => i - 1..i + 1,
        _ => {
            let c = next.text.cursor();
            c..c
        }
    };
    let start = range.start;
    next.text.replace(range, items).map_err(|e| e.to_string())?;
    next.text.set_cursor(start + 1);
    app.view.text = next;
    crate::canvas::text::follow(app);
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
    if ["text", "pairs"].iter().all(|k| p.get(*k).is_none_or(Value::is_null)) {
        app.clipboard.request = Some(crate::app::clipboard::PasteKind::KerningPairs);
        return Ok(());
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
            "ownDirection": model.own_direction,
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
            "selectedCount": model.selected.len(),
            "selected": model.selected.iter().take(MAX_LISTED).filter_map(|&i| model.rows.get(i)).map(|r| pair(&row_id(r.left, r.right))).collect::<Vec<_>>(),
            "current": kerning.current.as_ref().map(pair),
        }),
    );
}

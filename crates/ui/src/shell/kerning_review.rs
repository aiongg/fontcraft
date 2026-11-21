//! Kerning review: the pair lists of the Pairs & Phrases panel (`kerning.loadPairList`, `kerning.stepPairList`,
//! `kerning.setPairListMode`), auto-kerning a pair list for review (`kerning.autoKernPairList`), and stepping a side of
//! the edited pair through its kerning group (`kerning.stepGroupMember`).
//!
//! The panel keeps two work lists, pairs and phrases, each the built-in list until a list is loaded
//! ([`fontcraft_engine::cmd::pair_list`] reads them). The mode picks the list shown; `auto` shows pairs while the edit
//! view is in text mode and phrases otherwise. A stepped pair row goes into the edit view in place of the pair at the
//! current glyph (`kerning.showPairInEditView`); a phrase row replaces its text, one line at a time
//! (`editView.setText`).

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::cmd::pair_list::{MAX_PAIR_LIST_BYTES, PairListRow, builtin, parse_pair_list, read_pair_list_file};
use fontcraft_engine::model::Font;
use fontcraft_spacing::kerning::{Position, facing, members};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::{self, PanelId};
use super::params::{flag, quoted, rows as row_list, string};
use crate::app::{FontCraftApp, Msg};

/// Glyphs a phrase line may put in the edit view.
const MAX_LINE_GLYPHS: usize = 1_000;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "kerning.loadPairList",
        label: "Load Pair List…",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{path?: path, text?: string, reset?: bool, list?: \"pairs\"|\"phrases\" (default: the list the Pairs & Phrases panel shows)} load a work list of the Pairs & Phrases panel: one pair or phrase per line, words separated by spaces (glyph names, @groups, or characters), `\\n` breaking a phrase into lines shown one at a time, blank lines and lines starting with # or %% skipped; from a file (at most 4 MiB, 100,000 lines) or from text. reset restores the built-in list. Nothing is shown or selected until stepped to. Given none of them, the command shows the panel so a path can be typed there",
    },
    UiCommand {
        id: "kerning.stepPairList",
        label: "Next / Previous Pair",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{direction?: \"next\"|\"previous\" (default next), index?: row (from 0, in place of direction), select?: [rows] (only selects, shows nothing), list?: \"pairs\"|\"phrases\" (default: the list shown)} move to the next or previous row of the work list and show it in the edit view. A pair goes in place of the pair at the current glyph; a phrase replaces the text, a line at a time: a phrase of several lines steps through them before the row changes. At the last row next stays; at the first, previous stays. The row shown becomes the selection",
    },
    UiCommand {
        id: "kerning.setPairListMode",
        label: "Pair List Mode",
        menu: &[],
        shortcut: None,
        params: "{mode: \"pairs\"|\"phrases\"|\"auto\"} choose the list the Pairs & Phrases panel shows; auto shows pairs while the edit view is in text mode, phrases otherwise",
    },
    UiCommand {
        id: "kerning.stepGroupMember",
        label: "Next / Previous Group Member",
        menu: &["Font", "Kerning"],
        shortcut: None,
        params: "{side: \"left\"|\"right\", direction?: \"next\"|\"previous\" (default next)} in the pair at the edit view's current glyph (the current glyph and the one before it), replace the left or the right glyph with the next or previous member of its kerning group on the side facing the pair, in glyph order and wrapping round, so a class value can be judged on every member; the cursor stays between the two",
    },
    UiCommand {
        id: "kerning.autoKernPairList",
        label: "Auto Kern Pair List",
        menu: &[],
        shortcut: None,
        params: "{rows?: [rows] (default: the selected rows, else every row), apply?: bool (default false), discard?: bool, list?: \"pairs\"|\"phrases\", options?: {the options of kerning.autoKern: masters, direction, separation, minKern, onlyCloser, touching, keepExisting, round, step, depth}} without apply, run kerning.autoKern without storing on the pair rows (phrase rows are left out) and show its proposal in the Pairs & Phrases panel for review; with apply, store the proposal shown (kerning.autoKern on the same pairs and options, one undo step) and clear it; with discard, clear it without storing. A proposal made before the font last changed is refused",
    },
];

/// Which list the Pairs & Phrases panel shows.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListMode {
    /// Pairs while the edit view is in text mode, phrases otherwise.
    #[default]
    Auto,
    Pairs,
    Phrases,
}

impl ListMode {
    pub fn name(self) -> &'static str {
        match self {
            ListMode::Auto => "auto",
            ListMode::Pairs => "pairs",
            ListMode::Phrases => "phrases",
        }
    }

    pub fn from_name(name: &str) -> Option<ListMode> {
        [ListMode::Auto, ListMode::Pairs, ListMode::Phrases].into_iter().find(|m| m.name() == name)
    }
}

/// One work list: its rows, where they came from, and the row being shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkList {
    /// The file the rows came from, `text` for text given to the command; `None`: built in.
    pub source: Option<String>,
    /// `None` until a list is loaded: the built-in list.
    pub rows: Option<Vec<PairListRow>>,
    /// The row shown in the edit view, and the line of it.
    pub current: Option<usize>,
    pub line: usize,
    /// The rows selected in the panel, in the order they were picked.
    pub selected: Vec<usize>,
}

impl WorkList {
    /// The rows: the built-in list when none was loaded.
    pub fn rows(&self, pairs: bool) -> Vec<PairListRow> {
        self.rows.clone().unwrap_or_else(|| builtin(pairs))
    }
}

/// An auto-kerning proposal under review: the `kerning.autoKern` parameters, the document and revision it describes,
/// the rows it covers and its reply.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoKernProposal {
    pub params: Map<String, Value>,
    pub document: (u64, u64),
    pub rows: Vec<usize>,
    pub reply: Value,
}

/// The kerning review's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KerningReviewState {
    pub mode: ListMode,
    pub pairs: WorkList,
    pub phrases: WorkList,
    pub auto_kern: Option<AutoKernProposal>,
}

impl KerningReviewState {
    pub fn list(&self, pairs: bool) -> &WorkList {
        if pairs { &self.pairs } else { &self.phrases }
    }

    pub fn list_mut(&mut self, pairs: bool) -> &mut WorkList {
        if pairs { &mut self.pairs } else { &mut self.phrases }
    }

    /// Whether the panel shows the pairs list (else the phrases list); `text_mode`: the edit view is in text mode.
    pub fn shows_pairs(&self, text_mode: bool) -> bool {
        match self.mode {
            ListMode::Pairs => true,
            ListMode::Phrases => false,
            ListMode::Auto => text_mode,
        }
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "kerning.loadPairList" => load(app, p),
        "kerning.stepPairList" => step(app, p),
        "kerning.setPairListMode" => {
            let name = string(p, "mode")?.ok_or("`mode` (pairs, phrases or auto) is required")?;
            app.view.kerning_review.mode =
                ListMode::from_name(name).ok_or_else(|| format!("`mode` must be pairs, phrases or auto, got `{}`", quoted(name)))?;
            Ok(())
        }
        "kerning.autoKernPairList" => auto_kern(app, p),
        "kerning.stepGroupMember" => step_member(app, p),
        other => Err(format!("no kerning review command `{other}`")),
    }
}

/// The list a command names with `list`, else the one shown.
fn list_arg(app: &FontCraftApp, p: &Value) -> Result<bool, String> {
    match string(p, "list")? {
        None => Ok(app.view.kerning_review.shows_pairs(crate::canvas::text::text_mode(&app.view))),
        Some("pairs") => Ok(true),
        Some("phrases") => Ok(false),
        Some(other) => Err(format!("`list` must be pairs or phrases, got `{}`", quoted(other))),
    }
}

fn load(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let pairs = list_arg(app, p)?;
    let reset = flag(p, "reset")?.unwrap_or(false);
    let (rows, source) = match (string(p, "path")?, string(p, "text")?, reset) {
        (Some(path), None, false) => {
            let bytes = read_pair_list_file(path)?;
            (parse_pair_list(&bytes).map_err(|e| format!("{}: {e}", quoted(path)))?, Some(path.to_string()))
        }
        (None, Some(text), false) => {
            if text.len() > MAX_PAIR_LIST_BYTES {
                return Err(format!("`text` is longer than {MAX_PAIR_LIST_BYTES} bytes"));
            }
            (parse_pair_list(text.as_bytes())?, Some("text".to_string()))
        }
        (None, None, true) => (builtin(pairs), None),
        (None, None, false) => {
            let workspace = &mut app.view.workspace;
            if workspace.layout.panel_state(&PanelId::PairsAndPhrases) == craft::behavior::dock::PanelState::Hidden {
                dock::toggle(workspace, PanelId::PairsAndPhrases);
            }
            return Ok(());
        }
        _ => return Err("give one of `path`, `text` and `reset`".into()),
    };
    let state = &mut app.view.kerning_review;
    *state.list_mut(pairs) = WorkList { source, rows: Some(rows), current: None, line: 0, selected: Vec::new() };
    if pairs {
        state.auto_kern = None;
    }
    Ok(())
}

/// The row and line a step goes to; `None` when the list has no rows.
fn target(rows: &[PairListRow], current: Option<usize>, line: usize, forward: bool) -> Option<(usize, usize)> {
    let lines = |i: usize| rows.get(i).map_or(1, |r| r.lines.len().max(1));
    let last = rows.len().checked_sub(1)?;
    match current.filter(|c| *c <= last) {
        None if forward => Some((0, 0)),
        None => Some((last, lines(last) - 1)),
        Some(c) if forward && line + 1 < lines(c) => Some((c, line + 1)),
        Some(c) if forward && c < last => Some((c + 1, 0)),
        Some(c) if !forward && line > 0 => Some((c, line.min(lines(c)) - 1)),
        Some(c) if !forward && c > 0 => Some((c - 1, lines(c - 1) - 1)),
        Some(c) => Some((c, line.min(lines(c) - 1))),
    }
}

fn step(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let pairs = list_arg(app, p)?;
    let rows = app.view.kerning_review.list(pairs).rows(pairs);
    if let Some(picked) = row_list(p, "select", rows.len())? {
        app.view.kerning_review.list_mut(pairs).selected = picked;
        return Ok(());
    }
    let wl = app.view.kerning_review.list(pairs);
    let index = match p.get("index").filter(|v| !v.is_null()) {
        Some(v) => Some(
            v.as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .filter(|i| *i < rows.len())
                .ok_or_else(|| format!("the list has {} rows; `index` is out of range", rows.len()))?,
        ),
        None => None,
    };
    let (row, line) = match index {
        Some(i) => (i, 0),
        None => {
            let forward = match string(p, "direction")? {
                None | Some("next") => true,
                Some("previous") => false,
                Some(other) => return Err(format!("`direction` must be next or previous, got `{}`", quoted(other))),
            };
            target(&rows, wl.current, wl.line, forward).ok_or("the list has no rows")?
        }
    };
    let entry = rows.get(row).ok_or("the list has no such row")?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let show = show_row(font, entry, line).map_err(|e| format!("row {row} (line {} of the list): {e}", entry.line))?;
    let wl = app.view.kerning_review.list_mut(pairs);
    wl.current = Some(row);
    wl.line = line;
    wl.selected = vec![row];
    app.sent.extend(show);
    Ok(())
}

/// A pair row's word as a command side: `@group` as written, a glyph name the font has, or a single character a glyph
/// encodes.
fn side_text(font: &Font, word: &str) -> Result<String, String> {
    if word.starts_with('@') || font.glyph(word).is_some() {
        return Ok(word.to_string());
    }
    let mut chars = word.chars();
    if let (Some(_), None) = (chars.next(), chars.next())
        && let Ok(found) = glyphs_for_text(font, word)
        && let Some(name) = found.into_iter().next()
    {
        return Ok(name);
    }
    Err(format!("the font has no glyph `{}`", quoted(word)))
}

/// The glyphs that encode the characters of `text`, in order.
fn glyphs_for_text(font: &Font, text: &str) -> Result<Vec<String>, String> {
    if text.chars().count() > MAX_LINE_GLYPHS {
        return Err(format!("a line holds at most {MAX_LINE_GLYPHS} characters"));
    }
    text.chars()
        .map(|c| {
            let code = u32::from(c);
            font.glyphs
                .iter()
                .find(|g| g.unicodes.contains(&code))
                .map(|g| g.name.clone())
                .ok_or_else(|| format!("no glyph encodes U+{code:04X} `{c}`"))
        })
        .collect()
}

/// The glyphs of one phrase line: a word that names a glyph is that glyph, any other word its characters; two words of
/// characters in a row get the space glyph between them.
fn phrase_glyphs(font: &Font, words: &[String]) -> Result<Vec<String>, String> {
    let space = font.glyphs.iter().find(|g| g.unicodes.contains(&0x20)).map(|g| g.name.clone());
    let mut out = Vec::new();
    let mut after_chars = false;
    for word in words {
        if font.glyph(word).is_some() {
            out.push(word.clone());
            after_chars = false;
            continue;
        }
        if after_chars && let Some(s) = &space {
            out.push(s.clone());
        }
        out.extend(glyphs_for_text(font, word)?);
        after_chars = true;
        if out.len() > MAX_LINE_GLYPHS {
            return Err(format!("a line shows at most {MAX_LINE_GLYPHS} glyphs"));
        }
    }
    Ok(out)
}

/// The edit view requests that show a row's line: a pair in place of the pair at the current glyph, a phrase line as
/// the text with the cursor at its start.
fn show_row(font: &Font, row: &PairListRow, line: usize) -> Result<Vec<Msg>, String> {
    let words = row.lines.get(line).ok_or("the row has no such line")?;
    let shell = |id: &str, params: Value| Msg::Shell(CommandRef::with_params(id, params));
    if let ([left, right], true) = (words.as_slice(), row.is_pair()) {
        let params = json!({"left": side_text(font, left)?, "right": side_text(font, right)?});
        return Ok(vec![shell("kerning.showPairInEditView", params)]);
    }
    let glyphs = phrase_glyphs(font, words)?;
    if glyphs.is_empty() {
        return Err("the line shows no glyph".into());
    }
    Ok(vec![shell("editView.setText", json!({"glyphs": glyphs})), shell("editView.setCursor", json!({"index": 0}))])
}

/// Replaces one glyph of the pair at the edit view's current glyph with the next or previous member of its kerning
/// group on the side facing the pair.
fn step_member(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let position = match string(p, "side")? {
        Some("left") => Position::First,
        Some("right") => Position::Second,
        Some(other) => return Err(format!("`side` must be left or right, got `{}`", quoted(other))),
        None => return Err("`side` (left or right) is required".into()),
    };
    let forward = match string(p, "direction")? {
        None | Some("next") => true,
        Some("previous") => false,
        Some(other) => return Err(format!("`direction` must be next or previous, got `{}`", quoted(other))),
    };
    let text = &app.view.text;
    let current = text.edited_index().filter(|i| *i > 0).ok_or("the edit view has no pair at the current glyph")?;
    let (Some(left), Some(right)) = (text.item_glyph(current - 1), text.item_glyph(current)) else {
        return Err("the edit view has no pair at the current glyph".into());
    };
    let (item, glyph) = match position {
        Position::First => (current - 1, left.to_string()),
        Position::Second => (current, right.to_string()),
    };
    let side = facing(text.direction.layout(), position);
    let font = app.session.font().map_err(|e| e.to_string())?;
    let group = font
        .glyph(&glyph)
        .and_then(|g| side.of(&g.kern_groups).map(str::to_string))
        .ok_or_else(|| format!("`{}` has no {} kerning group", quoted(&glyph), side.name()))?;
    let list = members(font, side, &group);
    let n = list.len();
    let at = list.iter().position(|m| *m == glyph).unwrap_or(0);
    let next = match (n, forward) {
        (0, _) => 0,
        (_, true) => (at + 1) % n,
        (_, false) => (at + n - 1) % n,
    };
    let member = list.get(next).map(|m| m.to_string()).ok_or_else(|| format!("the group `{}` has no members", quoted(&group)))?;
    let replacement = crate::canvas::text::commands::glyph_item(font, &member);
    let mut edited = app.view.text.clone();
    edited.text.replace(item..item + 1, vec![replacement]).map_err(|e| e.to_string())?;
    edited.text.set_cursor(current);
    app.view.text = edited;
    crate::canvas::text::follow(app);
    Ok(())
}

/// The `kerning.autoKern` options a pair-list proposal passes on.
const AUTO_KERN_OPTIONS: &[&str] =
    &["masters", "direction", "separation", "minKern", "onlyCloser", "touching", "keepExisting", "round", "step", "depth"];

fn auto_kern(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let apply = flag(p, "apply")?.unwrap_or(false);
    if flag(p, "discard")?.unwrap_or(false) {
        if apply {
            return Err("give `apply` or `discard`, not both".into());
        }
        app.view.kerning_review.auto_kern = None;
        return Ok(());
    }
    let document = app.session.active().map(|d| (d.id, d.revision)).ok_or("no font is open")?;
    if apply {
        let proposal = app.view.kerning_review.auto_kern.clone().ok_or("there is no proposal to apply")?;
        if proposal.document != document {
            return Err("the font changed since the proposal was made; propose again".into());
        }
        let mut params = proposal.params;
        params.insert("apply".into(), json!(true));
        app.session.execute("kerning.autoKern", &Value::Object(params)).map_err(|e| e.to_string())?;
        app.view.kerning_review.auto_kern = None;
        return Ok(());
    }
    let pairs = list_arg(app, p)?;
    let mut options = Map::new();
    if let Some(given) = p.get("options").filter(|v| !v.is_null()) {
        let given = given.as_object().ok_or("`options` must be an object")?;
        for (k, v) in given {
            if !AUTO_KERN_OPTIONS.contains(&k.as_str()) {
                return Err(format!("`options` takes {}, not `{}`", AUTO_KERN_OPTIONS.join(", "), quoted(k)));
            }
            options.insert(k.clone(), v.clone());
        }
    }
    let wl = app.view.kerning_review.list(pairs);
    let all = wl.rows(pairs);
    let wanted: Vec<usize> = match row_list(p, "rows", all.len())? {
        Some(rows) => rows,
        None if !wl.selected.is_empty() => wl.selected.clone(),
        None => (0..all.len()).collect(),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let mut used = Vec::new();
    let mut list = Vec::new();
    for i in wanted {
        let Some(row) = all.get(i).filter(|r| r.is_pair()) else { continue };
        let Some([left, right]) = row.lines.first().map(Vec::as_slice) else { continue };
        let l = side_text(font, left).map_err(|e| format!("row {i}: {e}"))?;
        let r = side_text(font, right).map_err(|e| format!("row {i}: {e}"))?;
        list.push(json!([l, r]));
        used.push(i);
    }
    if list.is_empty() {
        return Err("none of the rows is a pair".into());
    }
    let mut params = options;
    params.insert("pairs".into(), Value::Array(list));
    params.insert("apply".into(), json!(false));
    let reply = app.session.execute("kerning.autoKern", &Value::Object(params.clone())).map_err(|e| e.to_string())?;
    app.view.kerning_review.auto_kern = Some(AutoKernProposal { params, document, rows: used, reply });
    Ok(())
}

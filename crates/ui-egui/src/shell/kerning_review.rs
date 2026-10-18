//! Kerning review: the pair lists of the Pairs and Phrases section stepped into the tab, the
//! representative glyph of a class pair, auto-kerning a pair list for review, the Kerning Pairs
//! Preview window and the kerning groups editor (`kerning.loadPairList`, `kerning.stepPairList`,
//! `kerning.representativeGlyph`, `window.kerningPairsPreview`, `dialog.kerningGroupsEditor`,
//! `kerning.previewGroupCombinations`).
//!
//! The section keeps two work lists, pairs and phrases, each the built-in list until a list is
//! loaded ([`fontcraft_engine::cmd::pair_list`] reads them). The mode picks the list shown;
//! `auto` shows pairs while the showing tab is in text mode and phrases otherwise. A pair row goes
//! into the tab in place of the pair at the current glyph, as `kerning.showPairInEditView` puts
//! it, so typed context stays; a phrase row replaces the tab's text, one line at a time.
//!
//! The drawing lives in `panels::pairs_and_phrases`, `dialogs::kerning_pairs_preview` and
//! `dialogs::kerning_groups_editor`.

use fontcraft_engine::cmd::pair_list::{MAX_PAIR_LIST_BYTES, PairListRow, builtin, parse_pair_list, read_pair_list_file};
use fontcraft_model::Font;
use fontcraft_spacing::kerning::{Direction, Position, facing, members};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::dialogs::kerning_groups_editor::{self as groups_editor, GroupsEditorState};
use crate::dialogs::kerning_pairs_preview::{self as preview, PreviewState};
use crate::panels::Pending;
use crate::state::{WritingDirection, glyphs_for_text};

/// Most rows a reply lists; `count` gives the full number.
pub const MAX_REPLY_ROWS: usize = 1_000;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// The Palette section's id.
pub const SECTION: &str = "pairsAndPhrases";

/// Which list the Pairs and Phrases section shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListMode {
    /// Pairs while the showing tab is in text mode, phrases otherwise.
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkList {
    /// The file the rows came from, `text` for text given to the command; `None`: built in.
    pub source: Option<String>,
    /// `None` until first used: the built-in list then. Not stored with the window state, so a
    /// loaded list lasts for the session.
    #[serde(skip)]
    pub rows: Option<Vec<PairListRow>>,
    /// The row shown in the tab, and the line of it.
    pub current: Option<usize>,
    pub line: usize,
    /// The rows selected in the section, in the order they were picked.
    pub selected: Vec<usize>,
}

impl WorkList {
    /// The rows, the built-in list when none were loaded this session.
    pub fn rows(&mut self, pairs: bool) -> &[PairListRow] {
        if self.rows.is_none() {
            self.source = None;
            self.current = None;
            self.line = 0;
            self.selected.clear();
        }
        self.rows.get_or_insert_with(|| builtin(pairs))
    }
}

/// An auto-kerning proposal under review: the `kerning.autoKern` parameters, the document and
/// revision it describes, the rows it covers and its reply.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoKernProposal {
    pub params: Map<String, Value>,
    pub document: (u64, u64),
    pub rows: Vec<usize>,
    pub reply: Value,
}

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KerningReviewState {
    pub mode: ListMode,
    pub pairs: WorkList,
    pub phrases: WorkList,
    /// The section's path field.
    #[serde(skip)]
    pub path_field: String,
    #[serde(skip)]
    pub auto_kern: Option<AutoKernProposal>,
    pub preview: PreviewState,
    pub groups: GroupsEditorState,
}

impl KerningReviewState {
    pub fn list(&mut self, pairs: bool) -> &mut WorkList {
        if pairs { &mut self.pairs } else { &mut self.phrases }
    }
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "kerning.loadPairList",
        "Load Pair List…",
        &["Font", "Kerning"],
        "",
        "{path?: path, text?: string, reset?: bool, list?: \"pairs\"|\"phrases\" (default: the list the Pairs and Phrases section shows)} load a work list of the Pairs and Phrases section: one pair or phrase per line, words separated by spaces (glyph names, @groups, or characters), `\\n` breaking a phrase into lines shown one at a time, blank lines and lines starting with # or %% skipped; from a file (at most 4 MiB, 100,000 lines) or from text. reset restores the built-in list. Nothing is shown or selected until stepped to. Given none of them, the command opens the section so a path can be typed there → {mode, list, shown, source, count, current, line, selected, rows: [{label, kind: \"pair\"|\"phrase\"}] (at most 1,000)}",
    ),
    (
        "kerning.stepPairList",
        "Next / Previous Pair",
        &["Font", "Kerning"],
        "",
        "{direction?: \"next\"|\"previous\" (default next), index?: row (from 0, in place of direction), select?: [rows] (only selects, shows nothing), list?: \"pairs\"|\"phrases\" (default: the list shown)} move to the next or previous row of the work list and show it in the showing tab (Font View showing: a new tab). A pair goes in place of the pair at the current glyph, as kerning.showPairInEditView puts it; a phrase replaces the tab's text, a line at a time: a phrase of several lines steps through them before the row changes. At the last row next stays; at the first, previous stays. The row shown becomes the selection → the list as kerning.loadPairList returns it, with row, line, lines, kind and shown (the tab's text)",
    ),
    (
        "kerning.setPairListMode",
        "Pair List Mode",
        &[],
        "",
        "{mode: \"pairs\"|\"phrases\"|\"auto\"} choose the list the Pairs and Phrases section shows; auto shows pairs while the showing tab is in text mode, phrases otherwise → the list as kerning.loadPairList returns it",
    ),
    (
        "kerning.stepGroupMember",
        "Next / Previous Group Member",
        &["Font", "Kerning"],
        "",
        "{side: \"left\"|\"right\", direction?: \"next\"|\"previous\" (default next), tab?: id (default: the showing tab)} in the pair at the tab's current glyph (the current glyph and the one before it), replace the left or the right glyph with the next or previous member of its kerning group on the side facing the pair, in glyph order and wrapping round, so a class value can be judged on every member; the cursor stays between the two. In the Pairs and Phrases section: Ctrl+Up and Ctrl+Down for the left side, Alt+Up and Alt+Down for the right → {side, group, glyph, index (in the group, from 0), members, pair: [left, right]}",
    ),
    (
        "kerning.autoKernPairList",
        "Auto Kern Pair List",
        &[],
        "",
        "{rows?: [rows] (default: the selected rows, else every row), apply?: bool (default false), discard?: bool, list?: \"pairs\"|\"phrases\", options?: {the options of kerning.autoKern: masters, direction, separation, minKern, onlyCloser, touching, keepExisting, round, step, depth}} without apply, run kerning.autoKern without storing on the pair rows (phrase rows are left out) and show its proposal in the Pairs and Phrases section for review; with apply, store the proposal shown (kerning.autoKern on the same pairs and options, one undo step) and clear it; with discard, clear it without storing. A proposal made before the font last changed is refused → the kerning.autoKern reply, with rows",
    ),
    (
        "window.kerningPairsPreview",
        "Kerning Pairs Preview",
        &["Window"],
        "",
        "{open?: bool (default true), master?: id|null (default: the showing tab's, else the first), glyph?: name|null (only the pairs naming this glyph or a group it belongs to on the side facing the pair; null: every pair), sort?: \"first\"|\"second\"|\"value\" (first or second glyph in glyph order, or size of the value, largest first), size?: 24|36|48|72|96|144 (pixels per em), select?: {left, right}|null, jump?: one character (scroll to the first pair whose sorted-by glyph has it)} show or hide the Kerning Pairs Preview: every stored left-to-right pair of the master drawn kerned with its value underneath, a group side drawn by its first member. Values that differ from the ones found when the window opened are marked changed. A selected pair with a group side lists every glyph combination it covers, each with the stored pair that applies to it → {open, master, glyph, sort, size, count, rows: [{left, right, value, changed}] (at most 1,000), selected, changed: [{left, right, value, was}], combinations?: [{first, second, value, step, pair: {left, right}|null}], scrollTo?}",
    ),
    (
        "dialog.kerningGroupsEditor",
        "Kerning Groups Editor",
        &["Window"],
        "",
        "{open?: bool (default true), side?: \"left\"|\"right\", group?: name|null (the current group; a new name starts an empty group), view?: \"cells\"|\"list\"|\"stacked\", propose?: {the parameters of kerning.buildGroups without apply}|null, apply?: bool} show or hide the Kerning Groups Editor: the groups of one side, the members of the current group as cells, as a list, or stacked (every member drawn over the others, aligned on the origin for a left group and on the advance width for a right group), and every glyph of the font to drag in. Members change through kerning.setLeftGroup and kerning.setRightGroup. propose runs kerning.buildGroups without storing and shows its proposal; null discards it; apply stores the proposal shown, refused when the font changed since → {open, side, groups: [{name, glyphs}], group, members, view, master, stacked: {aligned: \"origin\"|\"advance\", glyphs: [{name, offset}]}, proposal}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "kerning.loadPairList" => load(app, args),
        "kerning.stepPairList" => step(app, args),
        "kerning.setPairListMode" => {
            let name = args.string("mode")?.ok_or_else(|| args.err("`mode` (pairs, phrases or auto) is required"))?;
            let mode = ListMode::from_name(name).ok_or_else(|| args.err(format!("`mode` must be pairs, phrases or auto, got `{}`", quoted(name))))?;
            app.state.panels.kerning_review.mode = mode;
            let shown = shows_pairs(app);
            Ok(list_reply(app, shown))
        }
        "kerning.stepGroupMember" => step_member(app, args),
        "kerning.autoKernPairList" => auto_kern(app, args),
        "window.kerningPairsPreview" => preview::window(app, args),
        "dialog.kerningGroupsEditor" => groups_editor::window(app, args),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn quoted(s: &str) -> String {
    s.chars().take(MAX_QUOTED).collect()
}

/// Whether the section shows the pairs list (else the phrases list).
pub fn shows_pairs(app: &FontcraftApp) -> bool {
    match app.state.panels.kerning_review.mode {
        ListMode::Pairs => true,
        ListMode::Phrases => false,
        ListMode::Auto => app.state.tab().is_some_and(|t| t.text_mode),
    }
}

fn list_name(pairs: bool) -> &'static str {
    if pairs { "pairs" } else { "phrases" }
}

/// The list a command names with `list`, else the one shown.
fn list_arg(app: &FontcraftApp, args: &Args) -> Result<bool, String> {
    match args.string("list")? {
        None => Ok(shows_pairs(app)),
        Some("pairs") => Ok(true),
        Some("phrases") => Ok(false),
        Some(other) => Err(args.err(format!("`list` must be pairs or phrases, got `{}`", quoted(other)))),
    }
}

fn row_kind(row: &PairListRow) -> &'static str {
    if row.is_pair() { "pair" } else { "phrase" }
}

/// A work list as `kerning.loadPairList` returns it.
pub fn list_reply(app: &mut FontcraftApp, pairs: bool) -> Value {
    let shown = shows_pairs(app);
    let st = &mut app.state.panels.kerning_review;
    let mode = st.mode.name();
    let wl = st.list(pairs);
    let rows: Vec<Value> = wl.rows(pairs).iter().take(MAX_REPLY_ROWS).map(|r| json!({"label": r.label(), "kind": row_kind(r)})).collect();
    let count = wl.rows(pairs).len();
    json!({
        "mode": mode,
        "list": list_name(pairs),
        "shown": list_name(shown),
        "source": wl.source,
        "count": count,
        "current": wl.current,
        "line": wl.line,
        "selected": wl.selected,
        "rows": rows,
    })
}

fn load(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let pairs = list_arg(app, args)?;
    let path = args.string("path")?;
    let text = args.string("text")?;
    let reset = args.flag("reset")?.unwrap_or(false);
    let (rows, source) = match (path, text, reset) {
        (Some(path), None, false) => {
            let bytes = read_pair_list_file(path).map_err(|e| args.err(e))?;
            (parse_pair_list(&bytes).map_err(|e| args.err(format!("{}: {e}", quoted(path))))?, Some(path.to_string()))
        }
        (None, Some(text), false) => {
            if text.len() > MAX_PAIR_LIST_BYTES {
                return Err(args.err(format!("`text` is longer than {MAX_PAIR_LIST_BYTES} bytes")));
            }
            (parse_pair_list(text.as_bytes()).map_err(|e| args.err(e))?, Some("text".to_string()))
        }
        (None, None, true) => (builtin(pairs), None),
        (None, None, false) => {
            app.run("palette.toggleSection", &json!({"section": SECTION, "expanded": true}))?;
            return Ok(list_reply(app, pairs));
        }
        _ => return Err(args.err("give one of `path`, `text` and `reset`")),
    };
    let wl = app.state.panels.kerning_review.list(pairs);
    *wl = WorkList { source, rows: Some(rows), current: None, line: 0, selected: Vec::new() };
    if pairs {
        app.state.panels.kerning_review.auto_kern = None;
    }
    Ok(list_reply(app, pairs))
}

/// The row and line a step goes to; `None` when the list stays where it is.
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

fn step(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let pairs = list_arg(app, args)?;
    let rows: Vec<PairListRow> = app.state.panels.kerning_review.list(pairs).rows(pairs).to_vec();
    if let Some(select) = args.get("select") {
        let picked: Vec<usize> = select
            .as_array()
            .ok_or_else(|| args.err("`select` must be a list of row numbers"))?
            .iter()
            .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < rows.len()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| args.err(format!("`select` takes row numbers from 0 to {}", rows.len().saturating_sub(1))))?;
        app.state.panels.kerning_review.list(pairs).selected = picked;
        return Ok(list_reply(app, pairs));
    }
    let wl = app.state.panels.kerning_review.list(pairs);
    let (row, line) = match args.index("index")? {
        Some(i) => {
            let i = usize::try_from(i)
                .ok()
                .filter(|i| *i < rows.len())
                .ok_or_else(|| args.err(format!("the list has {} rows; `index` is out of range", rows.len())))?;
            (i, 0)
        }
        None => {
            let forward = match args.string("direction")? {
                None | Some("next") => true,
                Some("previous") => false,
                Some(other) => return Err(args.err(format!("`direction` must be next or previous, got `{}`", quoted(other)))),
            };
            target(&rows, wl.current, wl.line, forward).ok_or_else(|| args.err("the list has no rows"))?
        }
    };
    let entry = rows.get(row).ok_or_else(|| args.err("the list has no such row"))?;
    let shown = show_row(app, entry, line).map_err(|e| args.err(format!("row {row} (line {} of the list): {e}", entry.line)))?;
    let wl = app.state.panels.kerning_review.list(pairs);
    wl.current = Some(row);
    wl.line = line;
    wl.selected = vec![row];
    let mut out = list_reply(app, pairs);
    if let Some(m) = out.as_object_mut() {
        m.insert("row".into(), json!(row));
        m.insert("lines".into(), json!(entry.lines.len()));
        m.insert("kind".into(), json!(row_kind(entry)));
        m.insert("shown".into(), shown);
    }
    Ok(out)
}

/// A pair row's word as a command side: `@group` as written, a glyph name the font has, or a
/// single character a glyph encodes.
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

/// The glyphs of one phrase line: a word that names a glyph is that glyph, any other word its
/// characters; two words of characters in a row get the space glyph between them.
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
    }
    Ok(out)
}

/// Shows a row's line in the showing tab, or in a new tab when Font View shows.
fn show_row(app: &mut FontcraftApp, row: &PairListRow, line: usize) -> Result<Value, String> {
    let words = row.lines.get(line).ok_or_else(|| "the row has no such line".to_string())?;
    if let ([left, right], true) = (words.as_slice(), row.is_pair()) {
        let font = app.session.font().map_err(|e| e.to_string())?;
        let params = json!({"left": side_text(font, left)?, "right": side_text(font, right)?});
        return app.run("kerning.showPairInEditView", &params);
    }
    let glyphs = phrase_glyphs(app.session.font().map_err(|e| e.to_string())?, words)?;
    if glyphs.is_empty() {
        return Err("the line shows no glyph".into());
    }
    match app.state.tab().map(|t| t.id) {
        Some(tab) => {
            app.run("editView.setText", &json!({"tab": tab, "glyphs": glyphs}))?;
            app.run("editView.setCursor", &json!({"tab": tab, "index": 0}))
        }
        None => {
            app.run("view.newTab", &json!({"glyphs": glyphs}))?;
            app.run("editView.getText", &json!({}))
        }
    }
}

fn kerning_direction(d: WritingDirection) -> Direction {
    match d {
        WritingDirection::Ltr => Direction::Ltr,
        WritingDirection::Rtl => Direction::Rtl,
        WritingDirection::Ttb => Direction::Vertical,
    }
}

fn step_member(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let position = match args.string("side")? {
        Some("left") => Position::First,
        Some("right") => Position::Second,
        Some(other) => return Err(args.err(format!("`side` must be left or right, got `{}`", quoted(other)))),
        None => return Err(args.err("`side` (left or right) is required")),
    };
    let forward = match args.string("direction")? {
        None | Some("next") => true,
        Some("previous") => false,
        Some(other) => return Err(args.err(format!("`direction` must be next or previous, got `{}`", quoted(other)))),
    };
    let index = match args.index("tab")? {
        Some(id) => app.state.tab_index(id).ok_or_else(|| args.err(format!("no tab has id {id}")))?,
        None => app.state.active_tab.ok_or_else(|| args.err("no Edit View tab is showing"))?,
    };
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?;
    let current = tab.edited_index().filter(|i| *i > 0).ok_or_else(|| args.err("the tab has no pair at the current glyph"))?;
    let (Some(left), Some(right)) = (tab.item_glyph(current - 1), tab.item_glyph(current)) else {
        return Err(args.err("the tab has no pair at the current glyph"));
    };
    let (left, right) = (left.to_string(), right.to_string());
    let (tab_id, direction) = (tab.id, kerning_direction(tab.direction));
    let (item, glyph) = match position {
        Position::First => (current - 1, left.clone()),
        Position::Second => (current, right.clone()),
    };
    let side = facing(direction, position);
    let font = app.session.font().map_err(|e| e.to_string())?;
    let group = font
        .glyph(&glyph)
        .and_then(|g| side.of(&g.kern_groups).map(str::to_string))
        .ok_or_else(|| args.err(format!("`{}` has no {} kerning group", quoted(&glyph), side.name())))?;
    let list: Vec<String> = members(font, side, &group).into_iter().map(str::to_string).collect();
    let n = list.len();
    let at = list.iter().position(|m| *m == glyph).unwrap_or(0);
    let next = if n == 0 {
        0
    } else if forward {
        (at + 1) % n
    } else {
        (at + n - 1) % n
    };
    let member = list.get(next).cloned().ok_or_else(|| args.err(format!("the group `{}` has no members", quoted(&group))))?;
    app.run("editView.insertText", &json!({"tab": tab_id, "glyphs": [member], "replace": [item, item + 1]}))?;
    app.run("editView.setCursor", &json!({"tab": tab_id, "index": current}))?;
    let pair = match position {
        Position::First => [member.clone(), right],
        Position::Second => [left, member.clone()],
    };
    Ok(json!({"side": args.string("side")?, "group": group, "glyph": member, "index": next, "members": n, "pair": pair}))
}

/// The `kerning.autoKern` options a pair-list proposal passes on.
const AUTO_KERN_OPTIONS: &[&str] =
    &["masters", "direction", "separation", "minKern", "onlyCloser", "touching", "keepExisting", "round", "step", "depth"];

fn auto_kern(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let apply = args.flag("apply")?.unwrap_or(false);
    if args.flag("discard")?.unwrap_or(false) {
        if apply {
            return Err(args.err("give `apply` or `discard`, not both"));
        }
        app.state.panels.kerning_review.auto_kern = None;
        return Ok(json!({"pairs": [], "rows": [], "discarded": true}));
    }
    let document = app.session.doc().map(|d| (d.id, d.revision)).map_err(|e| e.to_string())?;
    if apply {
        let proposal = app.state.panels.kerning_review.auto_kern.clone().ok_or_else(|| args.err("there is no proposal to apply"))?;
        if proposal.document != document {
            return Err(args.err("the font changed since the proposal was made; propose again"));
        }
        let mut params = proposal.params;
        params.insert("apply".into(), json!(true));
        let mut out = app.run("kerning.autoKern", &Value::Object(params)).map_err(|e| args.err(e))?;
        app.state.panels.kerning_review.auto_kern = None;
        if let Some(m) = out.as_object_mut() {
            m.insert("rows".into(), json!(proposal.rows));
        }
        return Ok(out);
    }
    let pairs = list_arg(app, args)?;
    let mut options = Map::new();
    if let Some(given) = args.get("options") {
        let given = given.as_object().ok_or_else(|| args.err("`options` must be an object"))?;
        for (k, v) in given {
            if !AUTO_KERN_OPTIONS.contains(&k.as_str()) {
                return Err(args.err(format!("`options` takes {}, not `{}`", AUTO_KERN_OPTIONS.join(", "), quoted(k))));
            }
            options.insert(k.clone(), v.clone());
        }
    }
    let wl = app.state.panels.kerning_review.list(pairs);
    let all: Vec<PairListRow> = wl.rows(pairs).to_vec();
    let wanted: Vec<usize> = match args.get("rows") {
        Some(v) => v
            .as_array()
            .ok_or_else(|| args.err("`rows` must be a list of row numbers"))?
            .iter()
            .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < all.len()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| args.err(format!("`rows` takes row numbers from 0 to {}", all.len().saturating_sub(1))))?,
        None if !wl.selected.is_empty() => wl.selected.clone(),
        None => (0..all.len()).collect(),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let mut used = Vec::new();
    let mut list = Vec::new();
    for i in wanted {
        let Some(row) = all.get(i).filter(|r| r.is_pair()) else { continue };
        let Some([left, right]) = row.lines.first().map(Vec::as_slice) else { continue };
        let (l, r) = (side_text(font, left), side_text(font, right));
        let (l, r) = (l.map_err(|e| args.err(format!("row {i}: {e}")))?, r.map_err(|e| args.err(format!("row {i}: {e}")))?);
        list.push(json!([l, r]));
        used.push(i);
    }
    if list.is_empty() {
        return Err(args.err("none of the rows is a pair"));
    }
    let mut params = options;
    params.insert("pairs".into(), Value::Array(list));
    params.insert("apply".into(), json!(false));
    let mut reply = app.run("kerning.autoKern", &Value::Object(params.clone())).map_err(|e| args.err(e))?;
    app.state.panels.kerning_review.auto_kern = Some(AutoKernProposal { params, document, rows: used.clone(), reply: reply.clone() });
    if let Some(m) = reply.as_object_mut() {
        m.insert("rows".into(), json!(used));
    }
    Ok(reply)
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    preview::show(app, ui, pending);
    groups_editor::show(app, ui, pending);
}

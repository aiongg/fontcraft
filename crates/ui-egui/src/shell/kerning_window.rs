//! The Kerning window (`window.kerning`): the stored pairs of the current master in the current
//! writing direction, listed by `kerning.listPairs`, with the search options, sorting, in-place
//! editing, copy and paste, deletion and the actions menu.
//!
//! | Input | Does |
//! |---|---|
//! | typing in the search field | `kerning.filterPairs {term}` |
//! | Options menu | the match mode, the entry kind, the side, the Font View selection, Reset |
//! | header click | `kerning.sortPairs` by the column; a second click reverses the order |
//! | row click (Shift, Cmd as in lists) | selects rows; the clicked pair is shown in Edit View |
//! | Up, Down (list focused) | `kerning.selectPreviousPair`, `kerning.selectNextPair` |
//! | double-click a Value cell | edits the value: Return or leaving confirms (`kerning.setPair`), Escape cancels |
//! | double-click a group cell | edits the group name; confirming opens `dialog.renameKerningGroup` |
//! | Cmd+A, Copy, Paste (list focused) | selects every row; `kerning.copyPairs` to the clipboard; `kerning.pastePairs` into the master shown, a clash opening `dialog.pasteKerningConflict` |
//! | − button (Option: every master) | `kerning.deletePairs` on the selected rows |
//! | Actions menu | Show All Glyphs, Clean Up, Compress |
//!
//! The master is the showing tab's (Font View's when it shows) and the direction the showing
//! tab's (the last one shown when Font View shows). The list is asked for again only when the
//! document, the master, the direction, the search or the sort change, and only the rows in view
//! are laid out.

use egui::{Align2, CornerRadius, FontId, Key, Modifiers, Rect, Sense, Shape, Stroke, pos2, vec2};
use fontcraft_model::{Font, MAX_NAME_LEN};
use fontcraft_spacing::kerning::{Direction, Position, Side, facing, members};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::state::{MAX_TAB_GLYPHS, WritingDirection};
use crate::theme::Tokens;

type Pending = Vec<(&'static str, Value)>;

/// Longest search term, in bytes: the longest glyph name and an `@`, as `kerning.listPairs` takes it.
const MAX_TERM_BYTES: usize = MAX_NAME_LEN + 1;
/// Longest search term the field takes, in characters (four bytes each stay within the limit).
const MAX_TERM_CHARS: usize = 64;
/// Most rows the window holds; a font has fewer pairs in one master than the file readers allow.
const MAX_ROWS: usize = 2_000_000;
/// Most pairs `window.kerning {select}` takes.
const MAX_SELECT: usize = 500_000;
/// Rows the table shows before it scrolls.
const VISIBLE_ROWS: f32 = 20.0;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// Longest clipboard text a paste takes (the kerning-text limit).
const MAX_PASTE_BYTES: usize = 16 * 1024 * 1024;
/// Longest text typed into a cell.
const MAX_CELL_CHARS: usize = 64;

pub const COMMANDS: &[UiCommand] = &[
    (
        "window.kerning",
        "Kerning",
        &["Window"],
        "Cmd+Alt+K",
        "{open?: bool (default true), select?: [{left: name|\"@group\", right: name|\"@group\"}]} show or hide the Kerning window: the stored pairs of the current master (the showing tab's, else Font View's) in the showing tab's writing direction. select sets the selected rows, which must be listed, and shows the last of them in Edit View as a click does → {open, master, source, direction, total, count, search, sort, selected, current, error}",
    ),
    (
        "kerning.filterPairs",
        "Filter Kerning Pairs",
        &[],
        "",
        "{term?: string, match?: \"contains\"|\"is\"|\"beginsWith\"|\"endsWith\"|\"doesNotContain\", kind?: \"group\"|\"notGroup\"|null, side?: \"left\"|\"right\"|null, selectedGlyphsOnly?: bool} set the Kerning window's search term and options; null clears kind or side; options not given keep their value. A term starting with @ is compared with the @ spelling of groups → the window as window.kerning returns it",
    ),
    (
        "kerning.resetFilter",
        "Reset",
        &[],
        "",
        "{} return the Kerning window's search options to their defaults (contains, every entry kind, both sides, not limited to the Font View selection); the term stays → the window as window.kerning returns it",
    ),
    (
        "kerning.sortPairs",
        "Sort Kerning Pairs",
        &[],
        "",
        "{column: \"left\"|\"value\"|\"right\", descending?: bool (default false)} sort the Kerning window by a column; ties keep the stored order → the window as window.kerning returns it",
    ),
    (
        "kerning.showPairInEditView",
        "Show Pair in Edit View",
        &[],
        "",
        "{left: name|\"@group\", right: name|\"@group\", tab?: id (default: the showing tab; Font View showing: a new tab)} put the pair into the tab's text in place of the pair at the current glyph (the current glyph and the one before it), or at the cursor when there is none; a group side is shown by its first member in glyph order, on the side facing the pair in the tab's direction. The cursor ends between the two → the tab's text as editView.getText returns it, with pair and glyphs",
    ),
    (
        "kerning.selectNextPair",
        "Next Kerning Pair",
        &[],
        "Down",
        "{} select the Kerning window's row after the current one (none: the first; at the last: it stays) and show it in Edit View → the window as window.kerning returns it, with shown",
    ),
    (
        "kerning.selectPreviousPair",
        "Previous Kerning Pair",
        &[],
        "Up",
        "{} select the Kerning window's row before the current one (none: the last; at the first: it stays) and show it in Edit View → the window as window.kerning returns it, with shown",
    ),
    (
        "kerning.showAllGlyphs",
        "Show All Glyphs",
        &[],
        "",
        "{left?: name|\"@group\", right?: name|\"@group\" (default: the Kerning window's current row), tab?: id} put every glyph pair the row covers into the tab's text in place of the pair at the current glyph: each group side expands to all its members in glyph order, one line per left glyph, its pairs separated by the space glyph when the font has one; at most as many items as a tab holds → the tab's text as editView.getText returns it, with pairs",
    ),
];

// ---------- state ----------

/// A column of the pair list; its id is the `column` of `kerning.sortPairs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Column {
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

    fn label(self) -> &'static str {
        match self {
            Column::Left => "Left",
            Column::Value => "Value",
            Column::Right => "Right",
        }
    }

    /// Width in `Tokens::list_column_unit`.
    fn width_units(self) -> f32 {
        match self {
            Column::Left | Column::Right => 4.0,
            Column::Value => 2.0,
        }
    }
}

/// The match modes: (`match` value, menu label).
pub const MATCHES: [(&str, &str); 5] = [
    ("contains", "Contains"),
    ("is", "Is Exactly"),
    ("beginsWith", "Begins With"),
    ("endsWith", "Ends With"),
    ("doesNotContain", "Does Not Contain"),
];
/// The entry kinds: (`kind` value, menu label).
pub const KINDS: [(&str, &str); 2] = [("group", "Is a Group"), ("notGroup", "Is Not a Group")];
/// The searched sides: (`side` value, menu label).
pub const SIDES: [(&str, &str); 2] = [("left", "Left Side"), ("right", "Right Side")];

/// The search term and options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
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
        Self { term: String::new(), matching: "contains".into(), kind: None, side: None, selected_glyphs_only: false }
    }
}

/// The window's state. Rows are named by [`row_id`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KerningWindowState {
    pub open: bool,
    pub search: Search,
    /// The sorted column and whether it descends; `None`: the stored order.
    pub sort: Option<(Column, bool)>,
    /// Selected rows, in the order they were picked.
    pub selected: Vec<String>,
    /// The row Up and Down step from and Shift+click extends from.
    pub current: Option<String>,
    /// The writing direction of the last Edit View tab shown.
    pub direction: WritingDirection,
    #[serde(skip)]
    pub listing: Listing,
    /// The cell being edited in place.
    #[serde(skip)]
    pub editing: Option<Editing>,
    /// The row to scroll into view in the next frame.
    #[serde(skip)]
    pub scroll_to: Option<usize>,
    /// The list's scroll offset and height in the last frame.
    #[serde(skip)]
    pub scrolled: [f32; 2],
    /// The rows laid out in the last frame with their rectangles, and the minus button: tests
    /// click them.
    #[serde(skip)]
    pub laid_out: Vec<(String, Rect)>,
    #[serde(skip)]
    pub minus_button: Option<Rect>,
}

/// A cell being edited in place.
#[derive(Clone, Debug, PartialEq)]
pub struct Editing {
    pub row: String,
    pub column: Column,
    pub text: String,
    /// Focus was given to the field; once it has lost it the edit ends.
    pub focused: bool,
}

/// One listed pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub left: String,
    pub right: String,
    pub value: f64,
}

impl Row {
    fn of(v: &Value) -> Option<Row> {
        let left = v.get("left").and_then(Value::as_str)?.to_string();
        let right = v.get("right").and_then(Value::as_str)?.to_string();
        let value = v.get("value").and_then(Value::as_f64).filter(|n| n.is_finite())?;
        Some(Row { left, right, value })
    }

    pub fn id(&self) -> String {
        row_id(&self.left, &self.right)
    }

    fn side(&self, column: Column) -> Option<&str> {
        match column {
            Column::Left => Some(&self.left),
            Column::Right => Some(&self.right),
            Column::Value => None,
        }
    }
}

/// A row's name: its sides joined by a tab, which no side may hold.
pub fn row_id(left: &str, right: &str) -> String {
    format!("{left}\t{right}")
}

fn split_id(id: &str) -> (&str, &str) {
    id.split_once('\t').unwrap_or((id, ""))
}

/// What a listing was asked for.
#[derive(Clone, Debug, PartialEq)]
struct ListKey {
    /// The document's id and revision.
    doc: Option<(u64, u64)>,
    master: Option<String>,
    direction: WritingDirection,
    search: Search,
    sort: Option<(Column, bool)>,
    glyphs: Option<Vec<String>>,
}

/// The rows of the last `kerning.listPairs` and what they were asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    key: Option<ListKey>,
    pub rows: Vec<Row>,
    /// The pairs the master has in the direction, listed or not.
    pub total: usize,
    pub master: Option<String>,
    /// The master whose pairs are listed (another one for a linked master).
    pub source: Option<String>,
    /// Why the list could not be read.
    pub error: Option<String>,
}

impl Listing {
    fn position(&self, id: &str) -> Option<usize> {
        let (left, right) = split_id(id);
        self.rows.iter().position(|r| r.left == left && r.right == right)
    }
}

// ---------- the list ----------

/// The master the window shows: the showing tab's, else Font View's.
fn master(app: &FontcraftApp) -> Option<String> {
    app.render_master()
}

fn kerning_direction(d: WritingDirection) -> Direction {
    d.layout()
}

fn list_key(app: &FontcraftApp) -> ListKey {
    let kw = &app.state.panels.kerning_window;
    let glyphs = kw.search.selected_glyphs_only.then(|| app.state.selected_glyphs.clone());
    ListKey {
        doc: app.session.active().map(|d| (d.id, d.revision)),
        master: master(app),
        direction: kw.direction,
        search: kw.search.clone(),
        sort: kw.sort,
        glyphs,
    }
}

/// `kerning.listPairs`'s parameters for `key`.
fn list_params(key: &ListKey) -> Value {
    let mut p = Map::new();
    if let Some(m) = &key.master {
        p.insert("master".into(), json!(m));
    }
    p.insert("direction".into(), json!(kerning_direction(key.direction).name()));
    p.insert("term".into(), json!(key.search.term));
    p.insert("match".into(), json!(key.search.matching));
    if let Some(k) = &key.search.kind {
        p.insert("kind".into(), json!(k));
    }
    if let Some(s) = &key.search.side {
        p.insert("side".into(), json!(s));
    }
    if let Some(g) = &key.glyphs {
        p.insert("glyphs".into(), json!(g));
    }
    if let Some((column, descending)) = key.sort {
        p.insert("sort".into(), json!(column.id()));
        p.insert("descending".into(), json!(descending));
    }
    Value::Object(p)
}

fn list(app: &mut FontcraftApp, key: ListKey) -> Listing {
    if key.doc.is_none() {
        return Listing { key: Some(key), error: Some("Open a font to see its kerning".into()), ..Default::default() };
    }
    match app.session.execute("kerning.listPairs", &list_params(&key)) {
        Ok(v) => {
            let rows: Vec<Row> =
                v.get("pairs").and_then(Value::as_array).map(|a| a.iter().take(MAX_ROWS).filter_map(Row::of).collect()).unwrap_or_default();
            let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
            let total = v.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(rows.len());
            Listing { master: text("master"), source: text("source"), total, rows, error: None, key: Some(key) }
        }
        Err(e) => Listing { key: Some(key), error: Some(e.to_string()), ..Default::default() },
    }
}

/// Lists the pairs again when the document, the master, the direction, the search or the sort
/// changed. The selection keeps only the rows still listed.
pub fn refresh(app: &mut FontcraftApp) {
    if let Some(d) = app.state.tab().map(|t| t.direction) {
        app.state.panels.kerning_window.direction = d;
    }
    let key = list_key(app);
    if app.state.panels.kerning_window.listing.key.as_ref() == Some(&key) {
        return;
    }
    let listing = list(app, key);
    let listed: std::collections::HashSet<String> = listing.rows.iter().map(Row::id).collect();
    let kw = &mut app.state.panels.kerning_window;
    kw.selected.retain(|id| listed.contains(id));
    if kw.current.as_ref().is_some_and(|c| !listed.contains(c)) {
        kw.current = None;
    }
    if kw.editing.as_ref().is_some_and(|e| !listed.contains(&e.row)) {
        kw.editing = None;
    }
    kw.listing = listing;
}

fn pair_json(id: &str) -> Value {
    let (left, right) = split_id(id);
    json!({"left": left, "right": right})
}

/// The window's state as its commands return it.
fn reply(app: &FontcraftApp) -> Value {
    let kw = &app.state.panels.kerning_window;
    let l = &kw.listing;
    json!({
        "open": kw.open,
        "master": l.master,
        "source": l.source,
        "direction": kerning_direction(kw.direction).name(),
        "total": l.total,
        "count": l.rows.len(),
        "search": {
            "term": kw.search.term,
            "match": kw.search.matching,
            "kind": kw.search.kind,
            "side": kw.search.side,
            "selectedGlyphsOnly": kw.search.selected_glyphs_only,
        },
        "sort": kw.sort.map(|(c, d)| json!({"column": c.id(), "descending": d})),
        "selected": kw.selected.iter().map(|id| pair_json(id)).collect::<Vec<_>>(),
        "current": kw.current.as_deref().map(pair_json),
        "error": l.error,
    })
}

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

// ---------- commands ----------

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "window.kerning" => window(app, args),
        "kerning.filterPairs" => filter(app, args),
        "kerning.resetFilter" => {
            let kw = &mut app.state.panels.kerning_window;
            kw.search = Search { term: std::mem::take(&mut kw.search.term), ..Search::default() };
            refresh(app);
            Ok(reply(app))
        }
        "kerning.sortPairs" => {
            let name = args.string("column")?.ok_or_else(|| args.err("`column` (\"left\", \"value\" or \"right\") is required"))?;
            let column = Column::of_id(name).ok_or_else(|| args.err(format!("`column` must be left, value or right, got \"{}\"", quoted(name))))?;
            let descending = args.flag("descending")?.unwrap_or(false);
            app.state.panels.kerning_window.sort = Some((column, descending));
            refresh(app);
            Ok(reply(app))
        }
        "kerning.showPairInEditView" => {
            let left = side_arg(args, "left")?.ok_or_else(|| args.err("`left` (a glyph name or @group) is required"))?;
            let right = side_arg(args, "right")?.ok_or_else(|| args.err("`right` (a glyph name or @group) is required"))?;
            show_pair(app, args, &left, &right)
        }
        "kerning.selectNextPair" => step(app, args, true),
        "kerning.selectPreviousPair" => step(app, args, false),
        "kerning.showAllGlyphs" => show_all(app, args),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn side_arg(args: &Args, key: &str) -> Result<Option<Side>, String> {
    match args.string(key)? {
        None => Ok(None),
        Some(text) => Side::parse(text).map(Some).map_err(|e| args.err(format!("`{key}`: {e}"))),
    }
}

/// `[{left, right}]` as row ids, at most [`MAX_SELECT`].
fn pairs_arg(args: &Args, key: &str) -> Result<Option<Vec<String>>, String> {
    let Some(v) = args.get(key) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| args.err(format!("`{key}` must be a list of {{left, right}}")))?;
    if list.len() > MAX_SELECT {
        return Err(args.err(format!("`{key}` holds at most {MAX_SELECT} pairs")));
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let side = |name: &str| item.get(name).and_then(Value::as_str);
        let (Some(left), Some(right)) = (side("left"), side("right")) else {
            return Err(args.err(format!("`{key}[{i}]` must be an object with left and right")));
        };
        out.push(row_id(left, right));
    }
    Ok(Some(out))
}

fn window(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let open = args.flag("open")?.unwrap_or(true);
    let select = pairs_arg(args, "select")?;
    refresh(app);
    if let Some(ids) = &select {
        let listing = &app.state.panels.kerning_window.listing;
        if let Some(bad) = ids.iter().find(|id| listing.position(id).is_none()) {
            let (l, r) = split_id(bad);
            return Err(args.err(format!("the pair {} {} is not listed", quoted(l), quoted(r))));
        }
    }
    // Showing the pair is the part that can fail, so it goes first.
    if let Some(last) = select.as_ref().and_then(|ids| ids.last()) {
        let (l, r) = split_id(last);
        let (left, right) = (Side::parse(l).map_err(|e| args.err(e))?, Side::parse(r).map_err(|e| args.err(e))?);
        show_pair(app, &Args::new("window.kerning", &Value::Null)?, &left, &right)?;
    }
    let kw = &mut app.state.panels.kerning_window;
    kw.open = open;
    if let Some(ids) = select {
        kw.current = ids.last().cloned();
        kw.scroll_to = kw.current.as_deref().and_then(|c| kw.listing.position(c));
        kw.selected = ids;
    }
    refresh(app);
    Ok(reply(app))
}

/// Reads one of `options` (`(value, label)`) at `key`.
fn option(args: &Args, key: &str, options: &[(&str, &str)]) -> Result<Option<String>, String> {
    match args.string(key)? {
        None => Ok(None),
        Some(v) if options.iter().any(|(id, _)| *id == v) => Ok(Some(v.to_string())),
        Some(v) => {
            let names: Vec<&str> = options.iter().map(|(id, _)| *id).collect();
            Err(args.err(format!("`{key}` must be {}, got \"{}\"", names.join(", "), quoted(v))))
        }
    }
}

fn filter(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let given = args.value();
    let explicit_null = |key: &str| given.get(key).is_some_and(Value::is_null);
    let term = args.string("term")?;
    if let Some(t) = term
        && t.len() > MAX_TERM_BYTES
    {
        return Err(args.err(format!("`term` may be at most {MAX_TERM_BYTES} bytes long")));
    }
    if let Some(t) = term
        && t.chars().any(char::is_control)
    {
        return Err(args.err("`term` may not hold control characters"));
    }
    let matching = option(args, "match", &MATCHES)?;
    let kind = option(args, "kind", &KINDS)?;
    let side = option(args, "side", &SIDES)?;
    let only = args.flag("selectedGlyphsOnly")?;
    let s = &mut app.state.panels.kerning_window.search;
    if let Some(t) = term {
        s.term = t.to_string();
    }
    if let Some(m) = matching {
        s.matching = m;
    }
    if kind.is_some() || explicit_null("kind") {
        s.kind = kind;
    }
    if side.is_some() || explicit_null("side") {
        s.side = side;
    }
    if let Some(o) = only {
        s.selected_glyphs_only = o;
    }
    refresh(app);
    Ok(reply(app))
}

/// The tab a pair is shown in: `tab`, else the showing one; `None`: Font View shows.
fn target_tab(app: &FontcraftApp, args: &Args) -> Result<Option<usize>, String> {
    match args.index("tab")? {
        Some(id) => app.state.tab_index(id).map(Some).ok_or_else(|| args.err(format!("no tab has id {id}"))),
        None => Ok(app.state.active_tab),
    }
}

fn tab_direction(app: &FontcraftApp, tab: Option<usize>) -> WritingDirection {
    tab.and_then(|i| app.state.tabs.get(i)).map_or(app.state.panels.kerning_window.direction, |t| t.direction)
}

/// The glyphs a side stands for, in glyph order: the glyph, or the members of the group on the
/// side that faces the pair at `position`.
fn side_glyphs(font: &Font, direction: Direction, position: Position, side: &Side) -> Result<Vec<String>, String> {
    match side {
        Side::Glyph(name) => match font.glyph(name) {
            Some(g) => Ok(vec![g.name.clone()]),
            None => Err(format!("the font has no glyph `{}`", quoted(name))),
        },
        Side::Group(group) => {
            let side = facing(direction, position);
            let found: Vec<String> = members(font, side, group).into_iter().map(str::to_string).collect();
            if found.is_empty() {
                return Err(format!("no glyph has the {} kerning group `{}`", side.name(), quoted(group)));
            }
            Ok(found)
        }
    }
}

/// The items of the tab's text the pair at the current glyph covers: the current glyph and the
/// one before it; at the cursor when there is no glyph before it.
fn current_pair(app: &FontcraftApp, index: usize) -> (usize, usize) {
    let Some(tab) = app.state.tabs.get(index) else { return (0, 0) };
    match tab.edited_index() {
        Some(i) if i > 0 && tab.item_glyph(i - 1).is_some() => (i - 1, i + 1),
        _ => {
            let c = tab.text.cursor();
            (c, c)
        }
    }
}

/// Puts `payload` (`glyphs` or `text` as `editView.insertText` takes them) into the tab in place
/// of the pair at its current glyph, or into a new tab when Font View shows, and puts the cursor
/// after the first glyph.
fn place(app: &mut FontcraftApp, tab: Option<usize>, payload: (&str, Value), first_pair: [&str; 2]) -> Result<Value, String> {
    let index = match tab {
        Some(i) => i,
        None => {
            app.run("view.newTab", &json!({"glyphs": first_pair}))?;
            // The new tab takes the direction the window lists, so the list stays the same.
            let direction = app.state.panels.kerning_window.direction;
            if direction != WritingDirection::Ltr {
                app.run("editView.setWritingDirection", &json!({"direction": direction.name()}))?;
            }
            app.state.active_tab.ok_or_else(|| "kerning: the new tab is not showing".to_string())?
        }
    };
    let id = app.state.tabs.get(index).map(|t| t.id).ok_or_else(|| "kerning: no such tab".to_string())?;
    let (start, end) = if tab.is_some() { current_pair(app, index) } else { (0, app.state.tabs.get(index).map_or(0, |t| t.text.len())) };
    let mut insert = Map::new();
    insert.insert("tab".into(), json!(id));
    insert.insert(payload.0.into(), payload.1);
    insert.insert("replace".into(), json!([start, end]));
    app.run("editView.insertText", &Value::Object(insert))?;
    app.run("editView.setCursor", &json!({"tab": id, "index": start + 1}))
}

fn show_pair(app: &mut FontcraftApp, args: &Args, left: &Side, right: &Side) -> Result<Value, String> {
    let tab = target_tab(app, args)?;
    let direction = kerning_direction(tab_direction(app, tab));
    let font = app.session.font().map_err(|e| e.to_string())?;
    let first = |side: &Side, position| side_glyphs(font, direction, position, side).map(|g| g.into_iter().next().unwrap_or_default());
    let l = first(left, Position::First).map_err(|e| args.err(e))?;
    let r = first(right, Position::Second).map_err(|e| args.err(e))?;
    let mut out = place(app, tab, ("glyphs", json!([l, r])), [&l, &r])?;
    if let Some(m) = out.as_object_mut() {
        m.insert("pair".into(), json!({"left": left.command(), "right": right.command()}));
        m.insert("glyphs".into(), json!([l, r]));
    }
    Ok(out)
}

fn step(app: &mut FontcraftApp, args: &Args, forward: bool) -> Result<Value, String> {
    refresh(app);
    let kw = &app.state.panels.kerning_window;
    let rows = &kw.listing.rows;
    let last = rows.len().checked_sub(1).ok_or_else(|| args.err("the Kerning window lists no pairs"))?;
    let at = match kw.current.as_deref().and_then(|c| kw.listing.position(c)) {
        Some(i) if forward => (i + 1).min(last),
        Some(i) => i.saturating_sub(1),
        None if forward => 0,
        None => last,
    };
    let row = rows.get(at).cloned().ok_or_else(|| args.err("the Kerning window lists no pairs"))?;
    let left = Side::parse(&row.left).map_err(|e| args.err(e))?;
    let right = Side::parse(&row.right).map_err(|e| args.err(e))?;
    let shown = show_pair(app, &Args::new(args_id(forward), &Value::Null)?, &left, &right)?;
    let kw = &mut app.state.panels.kerning_window;
    kw.selected = vec![row.id()];
    kw.current = Some(row.id());
    kw.scroll_to = Some(at);
    let mut out = reply(app);
    if let Some(m) = out.as_object_mut() {
        m.insert("shown".into(), shown);
    }
    Ok(out)
}

fn args_id(forward: bool) -> &'static str {
    if forward { "kerning.selectNextPair" } else { "kerning.selectPreviousPair" }
}

/// The space glyph's name, when the font has one (by its Unicode value).
fn space_glyph(font: &Font) -> Option<String> {
    font.glyphs.iter().find(|g| g.unicodes.contains(&0x20)).map(|g| g.name.clone())
}

fn show_all(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let (left, right) = match (side_arg(args, "left")?, side_arg(args, "right")?) {
        (Some(l), Some(r)) => (l, r),
        (None, None) => {
            refresh(app);
            let current = app
                .state
                .panels
                .kerning_window
                .current
                .clone()
                .ok_or_else(|| args.err("select a pair in the Kerning window, or give left and right"))?;
            let (l, r) = split_id(&current);
            (Side::parse(l).map_err(|e| args.err(e))?, Side::parse(r).map_err(|e| args.err(e))?)
        }
        _ => return Err(args.err("give both `left` and `right`, or neither")),
    };
    let tab = target_tab(app, args)?;
    let direction = kerning_direction(tab_direction(app, tab));
    let font = app.session.font().map_err(|e| e.to_string())?;
    let lefts = side_glyphs(font, direction, Position::First, &left).map_err(|e| args.err(e))?;
    let rights = side_glyphs(font, direction, Position::Second, &right).map_err(|e| args.err(e))?;
    let space = space_glyph(font);
    // Two glyphs a pair, a space between pairs, a line break between lines.
    let per_line = rights.len().saturating_mul(2).saturating_add(if space.is_some() { rights.len().saturating_sub(1) } else { 0 });
    let items = lefts.len().saturating_mul(per_line).saturating_add(lefts.len().saturating_sub(1));
    let pairs = lefts.len().saturating_mul(rights.len());
    if items > MAX_TAB_GLYPHS {
        return Err(args.err(format!("the row covers {pairs} glyph pairs, {items} items; a tab holds at most {MAX_TAB_GLYPHS}")));
    }
    // `/name` runs to the next `/`, space or line break.
    if let Some(bad) = lefts.iter().chain(&rights).chain(&space).find(|n| n.contains(['/', ' '])) {
        return Err(args.err(format!("the glyph `{}` cannot be written into a text", quoted(bad))));
    }
    let separator = space.as_ref().map_or(String::new(), |s| format!("/{s}"));
    let lines: Vec<String> = lefts.iter().map(|l| rights.iter().map(|r| format!("/{l}/{r}")).collect::<Vec<_>>().join(&separator)).collect();
    let text = lines.join("\n");
    let (l0, r0) = (lefts.first().cloned().unwrap_or_default(), rights.first().cloned().unwrap_or_default());
    let mut out = place(app, tab, ("text", json!(text)), [&l0, &r0])?;
    if let Some(m) = out.as_object_mut() {
        m.insert("pairs".into(), json!(pairs));
    }
    Ok(out)
}

// ---------- copy, paste ----------

/// The selected rows as `[{left, right}]`, in listed order.
fn selected_pairs(app: &FontcraftApp) -> Vec<Value> {
    let kw = &app.state.panels.kerning_window;
    let chosen: std::collections::HashSet<&str> = kw.selected.iter().map(String::as_str).collect();
    kw.listing.rows.iter().filter(|r| chosen.contains(r.id().as_str())).map(|r| json!({"left": r.left, "right": r.right})).collect()
}

/// The master and direction edits go to: the master shown (a linked one is refused by the
/// command, with the reason).
fn scope(app: &FontcraftApp) -> Map<String, Value> {
    let kw = &app.state.panels.kerning_window;
    let mut p = Map::new();
    if let Some(m) = &kw.listing.master {
        p.insert("master".into(), json!(m));
    }
    p.insert("direction".into(), json!(kerning_direction(kw.direction).name()));
    p
}

/// Puts the selected pairs on the clipboard as kerning-text.
fn copy(app: &mut FontcraftApp) {
    let pairs = selected_pairs(app);
    if pairs.is_empty() {
        return;
    }
    let mut p = scope(app);
    // The rows of a linked master are its source's.
    if let Some(source) = &app.state.panels.kerning_window.listing.source {
        p.insert("master".into(), json!(source));
    }
    p.insert("pairs".into(), Value::Array(pairs));
    match app.run("kerning.copyPairs", &Value::Object(p)) {
        Ok(r) => {
            if let Some(text) = r.get("text").and_then(Value::as_str) {
                app.clipboard_out = Some(text.to_string());
            }
            let count = r.get("count").and_then(Value::as_u64).unwrap_or(0);
            app.state.status = format!("Copied {count} kerning pairs");
        }
        Err(e) => app.state.status = e,
    }
}

/// Pastes kerning-text into the master shown. When a pasted pair clashes with a stored one
/// nothing is pasted and the question opens.
fn paste(app: &mut FontcraftApp, text: String, pending: &mut Pending) {
    if text.len() > MAX_PASTE_BYTES {
        app.state.status = format!("Paste: the clipboard holds more than {MAX_PASTE_BYTES} bytes");
        return;
    }
    let mut p = scope(app);
    p.insert("text".into(), json!(text));
    let mut ask = p.clone();
    ask.insert("onConflict".into(), json!("ask"));
    match app.run("kerning.pastePairs", &Value::Object(ask)) {
        Ok(r) if r.get("asked").and_then(Value::as_bool) == Some(true) => pending.push(("dialog.pasteKerningConflict", Value::Object(p))),
        Ok(r) => {
            let added = r.get("added").and_then(Value::as_u64).unwrap_or(0);
            app.state.status = format!("Pasted {added} kerning pairs");
        }
        Err(e) => app.state.status = e,
    }
}

// ---------- drawing ----------

/// What the person did in the window this frame.
enum Event {
    Term(String),
    Sort(Column),
    Click(String, Modifiers),
    Edit(String, Column),
    EditText(String),
    EditEnd(bool),
    Close,
    SelectAll,
    Copy,
    Paste(String),
}

/// The columns with their left edges and widths, relative to the table's left.
fn placed(tokens: &Tokens) -> Vec<(Column, f32, f32)> {
    let mut x = 0.0;
    Column::ALL
        .iter()
        .map(|&c| {
            let w = c.width_units() * tokens.list_column_unit;
            let out = (c, x, w);
            x += w;
            out
        })
        .collect()
}

/// A kerning value as the list writes it: whole units without decimals.
pub fn format_value(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{}", (v * 100.0).round() / 100.0) }
}

/// The list's focus id: while it has the keyboard, Up, Down, Cmd+A, Copy and Paste are its own.
pub fn list_id() -> egui::Id {
    egui::Id::new("kerningWindow.list")
}

fn edit_id() -> egui::Id {
    egui::Id::new("kerningWindow.cell")
}

/// The id of a column header, for tests.
pub fn header_id(column: Column) -> egui::Id {
    egui::Id::new(("kerningWindow.header", column.id()))
}

/// Draws the window while it is open. Its buttons and menus queue commands in `pending`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.kerning_window.open {
        return;
    }
    refresh(app);
    let has_doc = app.session.active().is_some();
    let FontcraftApp { state, tokens, .. } = &mut *app;
    let kw = &mut state.panels.kerning_window;
    let columns = placed(tokens);
    let width = columns.last().map_or(0.0, |(_, x, w)| x + w);
    let mods = ui.input(|i| i.modifiers);
    let height = tokens.list_row_height;
    let mut events: Vec<Event> = Vec::new();
    let mut laid_out = Vec::new();
    let mut minus_button = None;
    let mut open = true;
    let focused = ui.memory(|m| m.has_focus(list_id()));
    if focused && kw.editing.is_none() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(list_id(), egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() })
        });
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                pending.push(("kerning.selectNextPair", json!({})));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                pending.push(("kerning.selectPreviousPair", json!({})));
            }
            if i.consume_key(Modifiers::COMMAND, Key::A) {
                events.push(Event::SelectAll);
            }
            for e in &i.events {
                match e {
                    egui::Event::Copy => events.push(Event::Copy),
                    egui::Event::Paste(text) => events.push(Event::Paste(text.clone())),
                    _ => {}
                }
            }
        });
    }
    // Scrolls only to bring a stepped-to row into view; otherwise the list keeps its own offset.
    let forced = kw.scroll_to.take().map(|at| {
        let [offset, view] = kw.scrolled;
        let (top, view) = (height * at as f32, view.max(height));
        if top < offset {
            top
        } else if top + height > offset + view {
            top + height - view
        } else {
            offset
        }
    });
    egui::Window::new("Kerning")
        .id(egui::Id::new("window.kerning"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(width, height * VISIBLE_ROWS))
        .pivot(Align2::CENTER_TOP)
        .default_pos(pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                let mut term = kw.search.term.clone();
                let field = egui::TextEdit::singleline(&mut term)
                    .id(egui::Id::new("kerningWindow.search"))
                    .hint_text("Search")
                    .char_limit(MAX_TERM_CHARS)
                    .desired_width(width - tokens.list_column_unit * 2.0);
                if ui.add(field).changed() {
                    events.push(Event::Term(term));
                }
                options_menu(ui, &kw.search, pending);
            });
            ui.add_space(tokens.dialog_spacing);
            let spacing = ui.spacing().item_spacing;
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            header(ui, kw.sort, &columns, width, tokens, &mut events);
            let selected: std::collections::HashSet<&str> = kw.selected.iter().map(String::as_str).collect();
            let editing = kw.editing.clone();
            let mut edit_drawn = false;
            let mut scroll = egui::ScrollArea::vertical().id_salt("kerningWindow.v").max_height(height * VISIBLE_ROWS).auto_shrink([false, false]);
            if let Some(offset) = forced {
                scroll = scroll.vertical_scroll_offset(offset.max(0.0));
            }
            let output = scroll.show_rows(ui, height, kw.listing.rows.len(), |ui, range| {
                for at in range {
                    let Some(row) = kw.listing.rows.get(at) else { continue };
                    let id = row.id();
                    let (rect, response) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), height), Sense::click());
                    let is_selected = selected.contains(id.as_str());
                    let fill = if is_selected {
                        tokens.row_selected
                    } else if at % 2 == 1 {
                        tokens.panel
                    } else {
                        tokens.panel_darker
                    };
                    ui.painter_at(rect).rect_filled(rect, CornerRadius::ZERO, fill);
                    for &(column, x, w) in &columns {
                        let cell = Rect::from_min_size(pos2(rect.min.x + x, rect.min.y), vec2(w, height));
                        if let Some(e) = editing.as_ref().filter(|e| e.row == id && e.column == column) {
                            edit_drawn = true;
                            cell_editor(ui, cell, e, &mut events);
                            continue;
                        }
                        let inner = cell.shrink2(vec2(tokens.cell_inset, 0.0));
                        let (text, color, align, at_x) = match row.side(column) {
                            Some(side) => {
                                let color = if is_selected {
                                    tokens.selected_text
                                } else if side.starts_with('@') {
                                    tokens.kerning_group_text
                                } else {
                                    tokens.kerning_glyph_text
                                };
                                (side.to_string(), color, Align2::LEFT_CENTER, inner.left())
                            }
                            None => {
                                let color = if is_selected { tokens.selected_text } else { tokens.text };
                                (format_value(row.value), color, Align2::RIGHT_CENTER, inner.right())
                            }
                        };
                        ui.painter_at(inner).text(pos2(at_x, inner.center().y), align, text, FontId::proportional(tokens.font_size), color);
                    }
                    laid_out.push((id.clone(), rect));
                    if response.double_clicked() {
                        let x = response.interact_pointer_pos().map_or(rect.min.x, |p| p.x) - rect.min.x;
                        let column = columns.iter().find(|(_, cx, w)| x >= *cx && x < cx + w).map_or(Column::Value, |c| c.0);
                        let editable = column == Column::Value || row.side(column).is_some_and(|s| s.starts_with('@'));
                        if editable {
                            events.push(Event::Edit(id.clone(), column));
                        }
                    } else if response.clicked() {
                        events.push(Event::Click(id.clone(), mods));
                    }
                }
            });
            let list_rect = output.inner_rect;
            kw.scrolled = [output.state.offset.y, list_rect.height()];
            ui.interact(list_rect, list_id(), Sense::focusable_noninteractive());
            if editing.is_some() && !edit_drawn {
                // The edited row scrolled out of view.
                events.push(Event::EditEnd(false));
            }
            ui.spacing_mut().item_spacing = spacing;
            ui.add_space(tokens.dialog_spacing);
            ui.horizontal(|ui| {
                let enabled = has_doc && !kw.selected.is_empty();
                let minus =
                    ui.add_enabled(enabled, egui::Button::new("−")).on_hover_text("Delete the selected pairs; with Option, from every master");
                minus_button = Some(minus.rect);
                if minus.clicked() {
                    let pairs: Vec<Value> = kw.selected.iter().map(|id| pair_json(id)).collect();
                    let mut p = Map::new();
                    if let Some(m) = &kw.listing.master {
                        p.insert("master".into(), json!(m));
                    }
                    p.insert("direction".into(), json!(kerning_direction(kw.direction).name()));
                    p.insert("pairs".into(), Value::Array(pairs));
                    p.insert("allMasters".into(), json!(mods.alt));
                    pending.push(("kerning.deletePairs", Value::Object(p)));
                }
                let l = &kw.listing;
                let note = match &l.error {
                    Some(e) => e.clone(),
                    None => {
                        let mut note =
                            if l.rows.len() == l.total { format!("{} pairs", l.total) } else { format!("{} of {} pairs", l.rows.len(), l.total) };
                        if !kw.selected.is_empty() {
                            note.push_str(&format!(", {} selected", kw.selected.len()));
                        }
                        if let (Some(m), Some(s)) = (&l.master, &l.source)
                            && m != s
                        {
                            note.push_str(&format!(" (the kerning of master {s})"));
                        }
                        note
                    }
                };
                let color = if l.error.is_some() { tokens.error_text } else { tokens.text_dim };
                ui.colored_label(color, note);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    actions_menu(ui, kw, has_doc, pending);
                });
            });
        });
    if !open {
        events.push(Event::Close);
    }
    let order: Vec<String> = kw.listing.rows.iter().map(Row::id).collect();
    let mut copy_now = false;
    let mut pasted = Vec::new();
    for e in events {
        match e {
            Event::Term(t) => pending.push(("kerning.filterPairs", json!({"term": t}))),
            Event::Sort(c) => {
                let descending = matches!(kw.sort, Some((s, d)) if s == c && !d);
                pending.push(("kerning.sortPairs", json!({"column": c.id(), "descending": descending})));
            }
            Event::Click(id, mods) => {
                let order: Vec<&str> = order.iter().map(String::as_str).collect();
                super::glyph_info::click(&mut kw.selected, &mut kw.current, &order, &id, mods);
                ui.memory_mut(|m| m.request_focus(list_id()));
                pending.push(("kerning.showPairInEditView", pair_json(&id)));
            }
            Event::Edit(id, column) => {
                let text = match kw.listing.position(&id).and_then(|i| kw.listing.rows.get(i)) {
                    Some(row) => match row.side(column) {
                        Some(side) => side.trim_start_matches('@').to_string(),
                        None => format_value(row.value),
                    },
                    None => continue,
                };
                kw.selected = vec![id.clone()];
                kw.current = Some(id.clone());
                kw.editing = Some(Editing { row: id, column, text, focused: false });
            }
            Event::EditText(t) => {
                if let Some(e) = &mut kw.editing {
                    e.text = t;
                    e.focused = true;
                }
            }
            Event::EditEnd(commit) => {
                if let Some(e) = kw.editing.take()
                    && commit
                {
                    match edit_command(kw, &e) {
                        Ok(Some(cmd)) => pending.push(cmd),
                        Ok(None) => {}
                        Err(msg) => state.status = msg,
                    }
                }
                ui.memory_mut(|m| m.request_focus(list_id()));
            }
            Event::Close => kw.open = false,
            Event::SelectAll => {
                kw.selected = order.clone();
                if kw.current.is_none() {
                    kw.current = order.first().cloned();
                }
            }
            Event::Copy => copy_now = true,
            Event::Paste(text) => pasted.push(text),
        }
    }
    kw.laid_out = laid_out;
    kw.minus_button = minus_button;
    if copy_now {
        copy(app);
    }
    for text in pasted {
        paste(app, text, pending);
    }
}

/// The command a finished cell edit runs: the value through `kerning.setPair`; a group name
/// through the rename question. `None`: nothing changed.
pub(crate) fn edit_command(kw: &KerningWindowState, e: &Editing) -> Result<Option<(&'static str, Value)>, String> {
    let (left, right) = split_id(&e.row);
    let mut p = Map::new();
    if let Some(m) = &kw.listing.master {
        p.insert("master".into(), json!(m));
    }
    p.insert("direction".into(), json!(kerning_direction(kw.direction).name()));
    match e.column {
        Column::Value => {
            let value: f64 = e
                .text
                .trim()
                .parse()
                .ok()
                .filter(|v: &f64| v.is_finite())
                .ok_or_else(|| format!("Kerning: \"{}\" is not a number", quoted(&e.text)))?;
            p.insert("left".into(), json!(left));
            p.insert("right".into(), json!(right));
            p.insert("value".into(), json!(value));
            Ok(Some(("kerning.setPair", Value::Object(p))))
        }
        column => {
            let side = if column == Column::Left { left } else { right };
            let from = side.trim_start_matches('@');
            let to = e.text.trim().trim_start_matches('@');
            if to == from {
                return Ok(None);
            }
            p.insert("side".into(), json!(column.id()));
            p.insert("from".into(), json!(from));
            p.insert("to".into(), json!(to));
            p.insert("pair".into(), json!({"left": left, "right": right}));
            Ok(Some(("dialog.renameKerningGroup", Value::Object(p))))
        }
    }
}

/// The field over a cell being edited: Return or leaving confirms, Escape cancels.
fn cell_editor(ui: &mut egui::Ui, cell: Rect, e: &Editing, events: &mut Vec<Event>) {
    let mut text = e.text.clone();
    let response = ui.put(cell, egui::TextEdit::singleline(&mut text).id(edit_id()).char_limit(MAX_CELL_CHARS));
    if !e.focused {
        response.request_focus();
        events.push(Event::EditText(text));
        return;
    }
    if response.changed() {
        events.push(Event::EditText(text));
    }
    if response.lost_focus() {
        let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
        events.push(Event::EditEnd(!cancelled));
    }
}

/// The search options behind the button beside the search field.
fn options_menu(ui: &mut egui::Ui, search: &Search, pending: &mut Pending) {
    ui.menu_button("Options", |ui| {
        for (id, label) in MATCHES {
            if ui.add(egui::Button::selectable(search.matching == id, label)).clicked() {
                pending.push(("kerning.filterPairs", json!({"match": id})));
                ui.close();
            }
        }
        ui.separator();
        for (id, label) in KINDS {
            let on = search.kind.as_deref() == Some(id);
            if ui.add(egui::Button::selectable(on, label)).clicked() {
                pending.push(("kerning.filterPairs", json!({"kind": if on { Value::Null } else { json!(id) }})));
                ui.close();
            }
        }
        ui.separator();
        for (id, label) in SIDES {
            let on = search.side.as_deref() == Some(id);
            if ui.add(egui::Button::selectable(on, label)).clicked() {
                pending.push(("kerning.filterPairs", json!({"side": if on { Value::Null } else { json!(id) }})));
                ui.close();
            }
        }
        ui.separator();
        let only = search.selected_glyphs_only;
        if ui.add(egui::Button::selectable(only, "Only Glyphs Selected in Font View")).clicked() {
            pending.push(("kerning.filterPairs", json!({"selectedGlyphsOnly": !only})));
            ui.close();
        }
        ui.separator();
        if ui.button("Reset").clicked() {
            pending.push(("kerning.resetFilter", json!({})));
            ui.close();
        }
    });
}

/// The actions menu at the window's foot.
fn actions_menu(ui: &mut egui::Ui, kw: &KerningWindowState, has_doc: bool, pending: &mut Pending) {
    let mut scope = Map::new();
    if let Some(m) = &kw.listing.master {
        scope.insert("master".into(), json!(m));
    }
    scope.insert("direction".into(), json!(kerning_direction(kw.direction).name()));
    ui.menu_button("Actions", |ui| {
        let current = kw.current.as_deref();
        if ui.add_enabled(current.is_some(), egui::Button::new("Show All Glyphs")).clicked()
            && let Some(id) = current
        {
            pending.push(("kerning.showAllGlyphs", pair_json(id)));
            ui.close();
        }
        if ui.add_enabled(has_doc, egui::Button::new("Clean Up")).clicked() {
            pending.push(("kerning.cleanUp", Value::Object(scope.clone())));
            ui.close();
        }
        if ui.add_enabled(has_doc, egui::Button::new("Compress")).clicked() {
            pending.push(("kerning.compress", Value::Object(scope.clone())));
            ui.close();
        }
    });
}

/// The header: labels, the sort mark, and sorting on click.
fn header(ui: &mut egui::Ui, sort: Option<(Column, bool)>, columns: &[(Column, f32, f32)], width: f32, tokens: &Tokens, events: &mut Vec<Event>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), tokens.list_header_height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, tokens.app_bar);
    for &(column, x, w) in columns {
        let cell = Rect::from_min_size(pos2(rect.min.x + x, rect.min.y), vec2(w, rect.height()));
        let response = ui.interact(cell, header_id(column), Sense::click());
        painter.line_segment([cell.right_top(), cell.right_bottom()], Stroke::new(1.0, tokens.border));
        let text = cell.shrink2(vec2(tokens.cell_inset, 0.0));
        ui.painter_at(text).text(
            text.left_center(),
            Align2::LEFT_CENTER,
            column.label(),
            FontId::proportional(tokens.cell_label_size + 1.0),
            tokens.text,
        );
        if let Some((sorted, descending)) = sort
            && sorted == column
        {
            // A small triangle at the right edge, pointing down while the rows ascend.
            let s = tokens.badge_size / 2.0;
            let c = pos2(cell.right() - tokens.cell_inset - s, cell.center().y);
            let points = if descending {
                vec![pos2(c.x - s, c.y + s / 2.0), pos2(c.x + s, c.y + s / 2.0), pos2(c.x, c.y - s / 2.0)]
            } else {
                vec![pos2(c.x - s, c.y - s / 2.0), pos2(c.x + s, c.y - s / 2.0), pos2(c.x, c.y + s / 2.0)]
            };
            painter.add(Shape::convex_polygon(points, tokens.text_dim, Stroke::NONE));
        }
        if response.clicked() {
            events.push(Event::Sort(column));
        }
    }
}

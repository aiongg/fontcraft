//! The Table Viewer window (M12.38): a compiled font's tables as a tree of decoded fields with a
//! hex pane, the TTX text of a table, and a second font compared table by table.
//!
//! Rows: `window.tableViewer`, `tableViewer.show`.
//!
//! | Input | Does |
//! |---|---|
//! | the font field (Enter), Open…, Reload | `tableViewer.show` with `font` |
//! | the Compare With field (Enter), Compare With… | `tableViewer.show` with `compareWith` |
//! | Stop Comparing | `tableViewer.show` with `compareWith: null` |
//! | click a table | `tableViewer.show` with `table` |
//! | Fields, TTX, Differences | `tableViewer.show` with `view` |
//! | click a record | `tableViewer.show` with `record` (selects it; the hex pane shows its bytes) |
//! | click the triangle of a record whose fields are not read yet | `tableViewer.show` with `record` and `expand` |
//! | click the triangle of a record already read | opens or closes it (window state only) |
//! | the search field | `tableViewer.show` with `search` |
//!
//! Every view is what an engine command returns: the table list and the tree are `tables.show`,
//! the TTX and hex texts `font.dumpTables`, the marks and the Differences view `font.diff`. The
//! window reads them when a parameter changes and keeps them until the next change, so editing
//! the file on disk shows only after Reload. A tree is read two levels at a time and at most
//! [`MAX_HELD_NODES`] records are held; texts are held up to [`MAX_HELD_TEXT`] bytes. Only the
//! rows in view are laid out.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

/// The views of the selected table: the `view` parameter, and the button label.
pub const VIEWS: [(&str, &str); 3] = [("fields", "Fields"), ("ttx", "TTX"), ("differences", "Differences")];
/// Levels of records one read of the tree adds below the record it starts at.
const TREE_DEPTH: u64 = 2;
/// Records the window holds for one table; past it the rest is counted, not held.
pub const MAX_HELD_NODES: usize = 200_000;
/// Bytes of TTX, hex or difference text the window holds for one table.
pub const MAX_HELD_TEXT: usize = 32 * 1024 * 1024;
/// Largest table the TTX view and the hex pane dump; a larger one says how to dump it to a file.
pub const MAX_DUMPED_TABLE: u64 = 4 * 1024 * 1024;
/// Longest path or search text accepted, in bytes.
const MAX_PARAM_TEXT: usize = 4_096;
/// Most levels of a `record` list.
const MAX_RECORD_DEPTH: usize = 64;
/// Most tables a list holds (`fontcraft_tables::limits::MAX_TABLES`), with the other font's.
const MAX_TABLE_ROWS: usize = 2_048;
/// Children of the selected record the command returns.
const MAX_RETURNED_RECORDS: usize = 100;
/// Characters of a text line or a tree value laid out; the rest ends in an ellipsis.
const MAX_SHOWN_CHARS: usize = 400;
/// Width of the table list and its columns, in points.
const LIST_WIDTH: f32 = 270.0;
const TAG_WIDTH: f32 = 52.0;
const SIZE_WIDTH: f32 = 78.0;
const CHECKSUM_WIDTH: f32 = 82.0;
/// Width of the content column, in points.
const CONTENT_WIDTH: f32 = 560.0;
/// Rows the lists show before they scroll.
const VISIBLE_ROWS: f32 = 26.0;
/// Rows of the hex pane.
const HEX_ROWS: f32 = 9.0;
/// The files Open… and Compare With… offer.
const FONT_EXTENSIONS: [&str; 6] = ["ttf", "otf", "ttc", "otc", "woff", "woff2"];

pub const COMMANDS: &[UiCommand] = &[(
    "tableViewer.show",
    "Table Viewer",
    &["Window"],
    "",
    "{font?: path, face?: number, compareWith?: path|null, table?: tag, view?: \"fields\"|\"ttx\"|\"differences\", path?: string, record?: [index], expand?: bool, search?: string} open the Table Viewer window. font reads the font's table list (tables.show) and keeps the selected table when the font has it; compareWith compares the font with a second one table by table (font.diff) and marks the tables that differ, null stops comparing; table selects a table and reads it for the view: fields is the tree of tables.show with the hex pane (font.dumpTables hex), ttx the TTX text (font.dumpTables), differences the decoded comparison of that table (font.diff); path (as tables.show writes it, GPOS.lookupList.lookups[3]) or record (child positions from the table's root, [2, 0]) selects a record, reading deeper levels when needed, and expand also reads and opens its children; search keeps the records or lines whose name or value contains the text, ignoring case. A refused change leaves the window as it was → {open, font, face, compareWith, table, view, search, tables: [{tag, length, checksum, decoded, error?, status?, firstOffset?}], identical?, differing: [name], content: {view, records?|lines?, truncated?, hits?, note?}, record?: {path?, record, name, type, value?, offset?, size?, more?, children: [{name, type, value?, more?}]}}",
)];

/// One row of the table list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableRow {
    pub tag: String,
    pub length: u64,
    pub checksum: String,
    pub decoded: bool,
    pub error: Option<String>,
    /// With a second font: `same`, `different`, `onlyA` or `onlyB` (font.diff's status).
    pub status: Option<String>,
    pub first_offset: Option<u64>,
    /// False for a table only the compared font has.
    pub in_font: bool,
}

/// One record of a held tree. A record's parent comes before it in [`Tree::nodes`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub kind: String,
    pub value: Option<String>,
    pub offset: Option<u64>,
    /// Children not read yet.
    pub more: u64,
    pub parent: Option<u32>,
    pub depth: u16,
    pub children: Vec<u32>,
}

/// A table's records as far as they are read; node 0 is the table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// Records were left out: the engine's limit or [`MAX_HELD_NODES`].
    pub truncated: bool,
}

/// A text held line by line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Text {
    pub text: String,
    /// Byte offset of each line's start.
    pub starts: Vec<usize>,
    /// The text was cut at [`MAX_HELD_TEXT`], or the engine left lines out.
    pub cut: bool,
}

/// What a view shows: its content, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum Pane<T> {
    Ready(Arc<T>),
    Note(String),
}

/// The second font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compare {
    pub identical: bool,
    pub differing: Vec<String>,
}

/// What the window read for its font and table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub tables: Vec<TableRow>,
    pub compare: Option<Compare>,
    pub tree: Option<Pane<Tree>>,
    pub hex: Option<Pane<Text>>,
    pub ttx: Option<Pane<Text>>,
    pub diff: Option<Pane<Text>>,
    /// Per tree node: it or a record below it matches the search.
    pub tree_hits: Vec<bool>,
    /// Lines of the TTX and difference texts that match the search.
    pub ttx_hits: Vec<u32>,
    pub diff_hits: Vec<u32>,
}

/// The data in the window's state: shared, compared by identity.
#[derive(Clone)]
pub struct Held(pub Arc<Data>);

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Held").field("tables", &self.0.tables.len()).finish()
    }
}

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TableViewerState {
    pub open: bool,
    /// The font file; empty before one is chosen.
    pub font: String,
    pub face: u64,
    /// The compared font file; empty when not comparing.
    pub compare_with: String,
    pub table: String,
    /// A [`VIEWS`] id.
    pub view: String,
    pub search: String,
    /// The selected record, as child positions from the table's root.
    pub record: Vec<u32>,
    #[serde(skip)]
    pub selected: Option<u32>,
    /// Tree records shown open.
    #[serde(skip)]
    pub expanded: BTreeSet<u32>,
    /// The hex pane scrolls to this line on the next frame.
    #[serde(skip)]
    pub hex_scroll: Option<usize>,
    #[serde(skip)]
    pub data: Option<Held>,
}

impl Default for TableViewerState {
    fn default() -> Self {
        Self {
            open: false,
            font: String::new(),
            face: 0,
            compare_with: String::new(),
            table: String::new(),
            view: "fields".into(),
            search: String::new(),
            record: Vec::new(),
            selected: None,
            expanded: BTreeSet::new(),
            hex_scroll: None,
            data: None,
        }
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "tableViewer.show" => show_command(app, args),
        _ => Err(format!("unknown command `{id}`")),
    }
}

// ---------- the command ----------

/// At most 64 characters of untrusted text, for a message.
fn clip(s: &str) -> String {
    s.chars().take(64).collect()
}

fn text_param<'a>(args: &Args<'a>, key: &str) -> Result<Option<&'a str>, String> {
    let s = args.string(key)?;
    if s.is_some_and(|s| s.len() > MAX_PARAM_TEXT) {
        return Err(args.err(format!("`{key}` is longer than {MAX_PARAM_TEXT} bytes")));
    }
    Ok(s)
}

fn query(app: &mut FontcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    app.session.query(id, params).map_err(|e| e.to_string())
}

fn show_command(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    // Work on a copy: a refused change leaves the window as it was.
    let mut st = app.state.panels.table_viewer.clone();
    let mut data = st.data.as_ref().map(|h| (*h.0).clone()).unwrap_or_default();

    let font = text_param(args, "font")?;
    let face = args.index("face")?;
    if let Some(font) = font {
        if font.is_empty() {
            return Err(args.err("`font` is empty: give a font file"));
        }
        st.font = font.to_string();
        st.face = face.unwrap_or(0);
    } else if let Some(face) = face {
        st.face = face;
    }
    let reload = font.is_some() || face.is_some();
    if reload {
        if st.font.is_empty() {
            return Err(args.err("`face` needs a font: give `font`"));
        }
        let tables = load_tables(app, &st.font, st.face).map_err(|e| args.err(e))?;
        data = Data { tables, ..Data::default() };
        clear_table(&mut st, &mut data);
    }

    match args.given("compareWith") {
        Some(Value::Null) => {
            st.compare_with.clear();
            data.compare = None;
            data.diff = None;
            if st.view == "differences" {
                st.view = "fields".into();
            }
        }
        Some(Value::String(other)) if other.is_empty() => {
            st.compare_with.clear();
            data.compare = None;
            data.diff = None;
            if st.view == "differences" {
                st.view = "fields".into();
            }
        }
        Some(Value::String(other)) => {
            if other.len() > MAX_PARAM_TEXT {
                return Err(args.err(format!("`compareWith` is longer than {MAX_PARAM_TEXT} bytes")));
            }
            st.compare_with = other.clone();
            data.diff = None;
        }
        Some(_) => return Err(args.err("`compareWith` must be a font file or null")),
        None => {}
    }
    if !st.compare_with.is_empty() && (reload || args.given("compareWith").is_some()) {
        if st.font.is_empty() {
            return Err(args.err("open a font before comparing it"));
        }
        let (compare, statuses) = load_compare(app, &st.font, st.face, &st.compare_with).map_err(|e| args.err(e))?;
        apply_compare(&mut data.tables, &statuses);
        data.compare = Some(compare);
    } else if st.compare_with.is_empty() {
        apply_compare(&mut data.tables, &[]);
    }

    if st.table.is_empty() || !data.tables.iter().any(|t| t.tag == st.table) {
        st.table = data.tables.iter().find(|t| t.in_font).map(|t| t.tag.clone()).unwrap_or_default();
        clear_table(&mut st, &mut data);
    }
    if let Some(table) = text_param(args, "table")? {
        let tag = table.trim_end();
        if !data.tables.iter().any(|t| t.tag == tag) {
            return Err(args.err(format!("the font has no table `{}`", clip(table))));
        }
        if st.table != tag {
            st.table = tag.to_string();
            clear_table(&mut st, &mut data);
        }
    }

    if let Some(view) = text_param(args, "view")? {
        if !VIEWS.iter().any(|(id, _)| *id == view) {
            return Err(args.err(format!("unknown view `{}`: use fields, ttx or differences", clip(view))));
        }
        if view == "differences" && st.compare_with.is_empty() {
            return Err(args.err("the Differences view needs a second font: give `compareWith`"));
        }
        st.view = view.to_string();
    }
    let path = text_param(args, "path")?;
    let record = match args.get("record") {
        None => None,
        Some(v) => Some(record_param(args, v)?),
    };
    if path.is_some() || record.is_some() {
        st.view = "fields".into();
    }
    if let Some(path) = path {
        let first = path.split(['.', '[']).next().unwrap_or_default();
        if first.is_empty() || !data.tables.iter().any(|t| t.tag == first && t.in_font) {
            return Err(args.err(format!("`path` must start with a table of the font, got `{}`", clip(path))));
        }
        if st.table != first {
            st.table = first.to_string();
            clear_table(&mut st, &mut data);
        }
    }

    if !st.table.is_empty() {
        load_view(app, &st, &mut data);
    }

    if path.is_some() || record.is_some() {
        let Some(Pane::Ready(shared)) = data.tree.clone() else {
            let why = match &data.tree {
                Some(Pane::Note(note)) => note.clone(),
                _ => "no table is selected".into(),
            };
            return Err(args.err(format!("cannot select a record: {why}")));
        };
        let wanted = match (path, &record) {
            (Some(path), _) => Target::Names(parse_path(path).map_err(|e| args.err(e))?),
            (None, Some(indices)) => Target::Positions(indices),
            (None, None) => Target::Positions(&[]),
        };
        let open = args.flag("expand")?.unwrap_or(false);
        // A record already held is selected without copying the tree.
        let held =
            held_target(&shared, &wanted).filter(|id| !open || shared.nodes.get(*id as usize).is_some_and(|n| !n.children.is_empty() || n.more == 0));
        let (tree, target) = match held {
            Some(id) => (shared, id),
            None => {
                let mut tree = (*shared).clone();
                let id = select(app, &st, &mut tree, &wanted).map_err(|e| args.err(e))?;
                if open {
                    expand(app, &st, &mut tree, id, 0).map_err(|e| args.err(e))?;
                }
                (Arc::new(tree), id)
            }
        };
        if open {
            st.expanded.insert(target);
        }
        let mut up = tree.nodes.get(target as usize).and_then(|n| n.parent);
        while let Some(p) = up {
            st.expanded.insert(p);
            up = tree.nodes.get(p as usize).and_then(|n| n.parent);
        }
        st.selected = Some(target);
        st.record = record_of(&tree, target);
        st.hex_scroll = byte_range(&tree, target, table_length(&data, &st.table)).map(|(start, _)| hex_line(start));
        data.tree = Some(Pane::Ready(tree));
    }

    if let Some(search) = args.given("search") {
        match search {
            Value::Null => st.search.clear(),
            Value::String(s) if s.len() <= MAX_PARAM_TEXT => st.search = s.clone(),
            Value::String(_) => return Err(args.err(format!("`search` is longer than {MAX_PARAM_TEXT} bytes"))),
            _ => return Err(args.err("`search` must be a string")),
        }
    }
    refresh_search(&mut data, &st.search);

    st.open = true;
    st.data = Some(Held(Arc::new(data)));
    app.state.panels.table_viewer = st;
    Ok(window_json(&app.state.panels.table_viewer))
}

fn record_param(args: &Args, v: &Value) -> Result<Vec<u32>, String> {
    let bad = || args.err("`record` must be a list of child positions, as [2, 0]");
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_RECORD_DEPTH {
        return Err(args.err(format!("`record` holds at most {MAX_RECORD_DEPTH} positions")));
    }
    list.iter().map(|i| i.as_u64().and_then(|i| u32::try_from(i).ok()).ok_or_else(bad)).collect()
}

/// Forgets what was read for the selected table.
fn clear_table(st: &mut TableViewerState, data: &mut Data) {
    data.tree = None;
    data.hex = None;
    data.ttx = None;
    data.diff = None;
    st.record.clear();
    st.selected = None;
    st.expanded.clear();
    st.hex_scroll = None;
}

fn table_length(data: &Data, tag: &str) -> u64 {
    data.tables.iter().find(|t| t.tag == tag).map_or(0, |t| t.length)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The table list: `tables.show` with no levels below each table.
fn load_tables(app: &mut FontcraftApp, font: &str, face: u64) -> Result<Vec<TableRow>, String> {
    let mut out = query(app, "tables.show", &json!({ "font": font, "face": face, "depth": 0 }))?;
    let Some(Value::Array(tables)) = out.get_mut("tables").map(Value::take) else {
        return Err("tables.show returned no table list".into());
    };
    Ok(tables
        .iter()
        .take(MAX_TABLE_ROWS)
        .filter_map(|t| {
            Some(TableRow {
                tag: str_of(t, "tag")?,
                length: t.get("length").and_then(Value::as_u64).unwrap_or(0),
                checksum: str_of(t, "checksum").unwrap_or_default(),
                decoded: t.get("decoded").and_then(Value::as_bool).unwrap_or(false),
                error: str_of(t, "error"),
                status: None,
                first_offset: None,
                in_font: true,
            })
        })
        .collect())
}

/// One table's comparison: name, status, first differing offset.
type Status = (String, String, Option<u64>);

/// The font compared with `other` byte by byte, table by table (`font.diff`).
fn load_compare(app: &mut FontcraftApp, font: &str, face: u64, other: &str) -> Result<(Compare, Vec<Status>), String> {
    let out = query(app, "font.diff", &json!({ "a": font, "b": other, "faceA": face, "detail": "offset" }))?;
    let Some(tables) = out.get("tables").and_then(Value::as_object) else {
        return Err(format!("{} is not a font file to compare with", clip(other)));
    };
    let statuses = tables
        .iter()
        .take(MAX_TABLE_ROWS)
        .map(|(name, t)| (name.clone(), str_of(t, "status").unwrap_or_default(), t.get("firstOffset").and_then(Value::as_u64)))
        .collect();
    let differing = out
        .get("differing")
        .and_then(Value::as_array)
        .map(|d| d.iter().take(MAX_TABLE_ROWS).filter_map(|n| n.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let identical = out.get("identical").and_then(Value::as_bool).unwrap_or(false);
    Ok((Compare { identical, differing }, statuses))
}

/// Marks the rows with their comparison and adds the tables only the other font has; with no
/// statuses, removes the marks and those rows.
fn apply_compare(rows: &mut Vec<TableRow>, statuses: &[Status]) {
    rows.retain(|r| r.in_font);
    for r in rows.iter_mut() {
        r.status = None;
        r.first_offset = None;
    }
    for (name, status, offset) in statuses {
        if let Some(r) = rows.iter_mut().find(|r| r.tag == *name) {
            r.status = Some(status.clone());
            r.first_offset = *offset;
        } else if status == "onlyB" && rows.len() < MAX_TABLE_ROWS {
            rows.push(TableRow { tag: name.clone(), status: Some(status.clone()), in_font: false, ..TableRow::default() });
        }
    }
}

/// Reads what the current view of the selected table needs and does not have yet. A failure is
/// kept as the view's note: the window still shows the table list.
fn load_view(app: &mut FontcraftApp, st: &TableViewerState, data: &mut Data) {
    let row = data.tables.iter().find(|t| t.tag == st.table).cloned().unwrap_or_default();
    match st.view.as_str() {
        "ttx" if data.ttx.is_none() => {
            data.ttx = Some(if !row.in_font {
                Pane::Note("Only the compared font has this table.".into())
            } else if row.length > MAX_DUMPED_TABLE {
                too_large(&row)
            } else {
                dump(app, st, "ttx")
            });
        }
        "differences" if data.diff.is_none() => data.diff = Some(load_diff(app, st)),
        "fields" => {
            if data.tree.is_none() {
                data.tree = Some(if row.in_font {
                    load_tree(app, st, &st.table, TREE_DEPTH)
                        .map(|(node, truncated)| {
                            let mut tree = Tree { nodes: Vec::new(), truncated };
                            tree.nodes.push(Node::default());
                            fill(&mut tree, 0, &node);
                            Pane::Ready(Arc::new(tree))
                        })
                        .unwrap_or_else(Pane::Note)
                } else {
                    Pane::Note("Only the compared font has this table.".into())
                });
            }
            if data.hex.is_none() && row.in_font {
                data.hex = Some(if row.length > MAX_DUMPED_TABLE { too_large(&row) } else { dump(app, st, "hex") });
            }
        }
        _ => {}
    }
}

fn too_large(row: &TableRow) -> Pane<Text> {
    Pane::Note(format!(
        "The table is {} bytes; this window dumps tables up to {} bytes. Write the dump to a file with font.dumpTables and `output`.",
        row.length, MAX_DUMPED_TABLE
    ))
}

/// The TTX or hex text of the selected table (`font.dumpTables`).
fn dump(app: &mut FontcraftApp, st: &TableViewerState, format: &str) -> Pane<Text> {
    let params = json!({ "font": st.font, "face": st.face, "tables": [st.table], "format": format });
    match query(app, "font.dumpTables", &params) {
        Ok(mut out) => match out.get_mut("text").map(Value::take) {
            Some(Value::String(text)) => Pane::Ready(Arc::new(Text::new(text, false))),
            _ => Pane::Note("font.dumpTables returned no text".into()),
        },
        Err(e) => Pane::Note(e),
    }
}

/// The decoded comparison of the selected table (`font.diff`).
fn load_diff(app: &mut FontcraftApp, st: &TableViewerState) -> Pane<Text> {
    let params = json!({ "a": st.font, "b": st.compare_with, "faceA": st.face, "tables": [st.table], "detail": "fields" });
    let out = match query(app, "font.diff", &params) {
        Ok(out) => out,
        Err(e) => return Pane::Note(e),
    };
    let Some(t) = out.get("tables").and_then(|t| t.get(st.table.as_str())) else {
        return Pane::Note("font.diff did not compare this table".into());
    };
    let status = t.get("status").and_then(Value::as_str).unwrap_or_default();
    let lines: Vec<&str> = t.get("lines").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if lines.is_empty() {
        return Pane::Note(match status {
            "same" => "The table is the same in both fonts.".into(),
            "onlyA" => "Only this font has the table.".into(),
            "onlyB" => "Only the compared font has the table.".into(),
            _ => "The tables differ; the comparison lists no lines.".into(),
        });
    }
    let left_out = t.get("truncated").and_then(Value::as_u64).unwrap_or(0) > 0;
    Pane::Ready(Arc::new(Text::new(lines.join("\n"), left_out)))
}

/// `tables.show` from `path` (a table tag or a record path), `depth` levels down, with offsets:
/// the node and whether the engine left records out.
fn load_tree(app: &mut FontcraftApp, st: &TableViewerState, path: &str, depth: u64) -> Result<(Value, bool), String> {
    let mut params = json!({ "font": st.font, "face": st.face, "depth": depth.min(1_000), "offsets": true });
    if let Some(obj) = params.as_object_mut() {
        if path.contains(['.', '[']) {
            obj.insert("path".into(), json!(path));
        } else {
            obj.insert("tables".into(), json!([path]));
        }
    }
    let mut out = query(app, "tables.show", &params)?;
    let Some(Value::Array(mut tables)) = out.get_mut("tables").map(Value::take) else {
        return Err("tables.show returned no table".into());
    };
    let Some(mut table) = tables.pop() else { return Err("tables.show returned no table".into()) };
    if let Some(error) = str_of(&table, "error") {
        return Err(format!("the table could not be decoded: {error}"));
    }
    let truncated = table.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    let node = table.get_mut("tree").map(Value::take).unwrap_or(Value::Null);
    Ok((node, truncated))
}

// ---------- the tree ----------

/// Copies the fields of `json` into node `at` and adds its children below it, up to
/// [`MAX_HELD_NODES`].
fn fill(tree: &mut Tree, at: u32, json: &Value) {
    if let Some(node) = tree.nodes.get_mut(at as usize) {
        node.name = str_of(json, "name").unwrap_or_default();
        node.kind = str_of(json, "type").unwrap_or_default();
        node.value = str_of(json, "value");
        node.offset = json.get("offset").and_then(Value::as_u64);
        node.more = json.get("more").and_then(Value::as_u64).unwrap_or(0);
        node.children.clear();
    }
    add_children(tree, at, json);
}

/// Adds the children of `json` (and theirs) below node `at`, depth first, so every parent comes
/// before its children. Children past the limit are counted in their parent's `more`.
fn add_children(tree: &mut Tree, at: u32, json: &Value) {
    let mut stack: Vec<(u32, &Value)> = vec![(at, json)];
    while let Some((parent, value)) = stack.pop() {
        let Some(kids) = value.get("children").and_then(Value::as_array) else { continue };
        let depth = tree.nodes.get(parent as usize).map_or(0, |n| n.depth.saturating_add(1));
        for kid in kids {
            let Ok(id) = u32::try_from(tree.nodes.len()) else { break };
            if tree.nodes.len() >= MAX_HELD_NODES {
                tree.truncated = true;
                if let Some(p) = tree.nodes.get_mut(parent as usize) {
                    p.more = p.more.saturating_add(1);
                }
                continue;
            }
            tree.nodes.push(Node {
                name: str_of(kid, "name").unwrap_or_default(),
                kind: str_of(kid, "type").unwrap_or_default(),
                value: str_of(kid, "value"),
                offset: kid.get("offset").and_then(Value::as_u64),
                more: kid.get("more").and_then(Value::as_u64).unwrap_or(0),
                parent: Some(parent),
                depth,
                children: Vec::new(),
            });
            if let Some(p) = tree.nodes.get_mut(parent as usize) {
                p.children.push(id);
            }
            stack.push((id, kid));
        }
    }
}

/// Fills, below node `at`, every held record whose children are not read yet from `json`, the
/// same record read deeper. Held records keep their ids.
fn merge(tree: &mut Tree, at: u32, json: &Value) {
    let mut stack: Vec<(u32, &Value)> = vec![(at, json)];
    while let Some((id, value)) = stack.pop() {
        let held = match tree.nodes.get(id as usize) {
            Some(n) if n.children.is_empty() => None,
            Some(n) => Some(n.children.clone()),
            None => continue,
        };
        let has_more = value.get("children").and_then(Value::as_array).is_some_and(|c| !c.is_empty());
        let Some(held) = held else {
            if has_more {
                if let Some(n) = tree.nodes.get_mut(id as usize) {
                    n.more = value.get("more").and_then(Value::as_u64).unwrap_or(0);
                }
                add_children(tree, id, value);
            }
            continue;
        };
        for (kid, json_kid) in held.iter().zip(value.get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()) {
            stack.push((*kid, json_kid));
        }
    }
}

/// A record name as one part of a `tables.show` path; `None` when the path syntax cannot name it.
fn path_part(name: &str) -> Option<String> {
    if name.is_empty() || name.contains(['.', '[', ']']) {
        return None;
    }
    if name.bytes().all(|b| b.is_ascii_digit()) { Some(format!("[{name}]")) } else { Some(format!(".{name}")) }
}

/// The `tables.show` path of node `id`, when every name on the way can be written in one.
fn path_of(tree: &Tree, id: u32) -> Option<String> {
    let mut parts = Vec::new();
    let mut at = Some(id);
    while let Some(i) = at {
        let node = tree.nodes.get(i as usize)?;
        match node.parent {
            Some(parent) => {
                parts.push(path_part(&node.name)?);
                at = Some(parent);
            }
            None => {
                parts.push(node.name.clone());
                at = None;
            }
        }
    }
    parts.reverse();
    Some(parts.concat())
}

/// The child positions from the root to node `id`.
fn record_of(tree: &Tree, id: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = id;
    while let Some(parent) = tree.nodes.get(at as usize).and_then(|n| n.parent) {
        let pos = tree.nodes.get(parent as usize).and_then(|p| p.children.iter().position(|c| *c == at)).unwrap_or(0);
        out.push(u32::try_from(pos).unwrap_or(u32::MAX));
        at = parent;
    }
    out.reverse();
    out
}

/// Reads the children of node `target` (and `extra` levels more) when they are not held: from
/// the nearest record a path can name, since record names may hold periods.
fn expand(app: &mut FontcraftApp, st: &TableViewerState, tree: &mut Tree, target: u32, extra: u64) -> Result<(), String> {
    let Some(node) = tree.nodes.get(target as usize) else { return Err("no such record".into()) };
    if !node.children.is_empty() || node.more == 0 {
        return Ok(());
    }
    let mut from = target;
    let mut distance = 0u64;
    let path = loop {
        if let Some(path) = path_of(tree, from) {
            break path;
        }
        let Some(parent) = tree.nodes.get(from as usize).and_then(|n| n.parent) else {
            return Err("the record cannot be named in a path".into());
        };
        from = parent;
        distance = distance.saturating_add(1);
    };
    let (json, truncated) = load_tree(app, st, &path, distance.saturating_add(extra).saturating_add(TREE_DEPTH))?;
    tree.truncated |= truncated;
    merge(tree, from, &json);
    Ok(())
}

/// A record named by the command: field names below the table, or child positions.
enum Target<'a> {
    Names(Vec<String>),
    Positions(&'a [u32]),
}

impl Target<'_> {
    fn len(&self) -> usize {
        match self {
            Target::Names(n) => n.len(),
            Target::Positions(p) => p.len(),
        }
    }

    /// The child of `node` that step `i` names.
    fn child(&self, tree: &Tree, node: &Node, i: usize) -> Option<u32> {
        match self {
            Target::Names(names) => {
                let name = names.get(i)?;
                node.children.iter().copied().find(|c| tree.nodes.get(*c as usize).is_some_and(|n| n.name == *name))
            }
            Target::Positions(p) => node.children.get(*p.get(i)? as usize).copied(),
        }
    }
}

/// The field names of a `tables.show` path below its table (`GPOS.lookupList.lookups[3]` is
/// lookupList, lookups, 3).
fn parse_path(path: &str) -> Result<Vec<String>, String> {
    let bad = |part: &str| format!("`{}` is not a field name with optional [index] parts", clip(part));
    let mut names = Vec::new();
    for (i, part) in path.split('.').enumerate() {
        let (name, rest) = match part.find('[') {
            Some(at) => (part.get(..at).unwrap_or_default(), part.get(at..).unwrap_or_default()),
            None => (part, ""),
        };
        if name.is_empty() {
            return Err(bad(part));
        }
        if i > 0 {
            names.push(name.to_string());
        }
        for index in rest.split('[').skip(1) {
            let index = index.strip_suffix(']').filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            names.push(index.ok_or_else(|| bad(part))?.to_string());
        }
        if names.len() > MAX_RECORD_DEPTH {
            return Err(format!("the path names more than {MAX_RECORD_DEPTH} levels"));
        }
    }
    Ok(names)
}

/// The node `target` names when every level on the way is held; `None` when one must be read
/// (or the record does not exist, which [`select`] reports).
fn held_target(tree: &Tree, target: &Target) -> Option<u32> {
    let mut at = 0u32;
    for i in 0..target.len() {
        let node = tree.nodes.get(at as usize)?;
        at = target.child(tree, node, i)?;
    }
    Some(at)
}

/// The node `target` names, reading levels that are not held.
fn select(app: &mut FontcraftApp, st: &TableViewerState, tree: &mut Tree, target: &Target) -> Result<u32, String> {
    let mut at = 0u32;
    for i in 0..target.len() {
        let remaining = u64::try_from(target.len() - i).unwrap_or(0);
        expand(app, st, tree, at, remaining.saturating_sub(1))?;
        let node = tree.nodes.get(at as usize).ok_or("no such record")?;
        at = target.child(tree, node, i).ok_or_else(|| match target {
            Target::Names(names) => format!("`{}` has no field `{}`", clip(&node.name), clip(names.get(i).map_or("", String::as_str))),
            Target::Positions(p) => format!("`{}` has no record at position {}", clip(&node.name), p.get(i).copied().unwrap_or(0)),
        })?;
    }
    Ok(at)
}

/// The bytes of node `id` in the table: from its offset, or its nearest ancestor's, to the next
/// sibling's offset or the end of its parent.
fn byte_range(tree: &Tree, id: u32, table_length: u64) -> Option<(u64, u64)> {
    let mut at = id;
    // Records hold at most a dozen levels plus what reads added; the walk stops at the root.
    for _ in 0..=usize::from(u16::MAX) {
        let node = tree.nodes.get(at as usize)?;
        if let Some(start) = node.offset {
            let end = match node.parent {
                None => table_length,
                Some(parent) => {
                    let siblings = &tree.nodes.get(parent as usize)?.children;
                    let next = siblings.iter().filter_map(|s| tree.nodes.get(*s as usize)?.offset).filter(|o| *o > start).min();
                    match next {
                        Some(next) => next,
                        None => byte_range(tree, parent, table_length).map_or(table_length, |(_, end)| end),
                    }
                }
            };
            return Some((start, end.max(start).min(table_length.max(start))));
        }
        at = node.parent?;
    }
    None
}

/// The hex dump's line for byte `offset`: one heading line, then 16 bytes a line.
fn hex_line(offset: u64) -> usize {
    usize::try_from(offset / 16).unwrap_or(usize::MAX).saturating_add(1)
}

impl Text {
    /// Splits `text` into lines, cut at [`MAX_HELD_TEXT`] on a line end.
    pub fn new(mut text: String, cut: bool) -> Self {
        let mut cut = cut;
        if text.len() > MAX_HELD_TEXT {
            let end = text.as_bytes().get(..MAX_HELD_TEXT).and_then(|b| b.iter().rposition(|c| *c == b'\n')).unwrap_or(0);
            text.truncate(end);
            cut = true;
        }
        if text.ends_with('\n') {
            text.pop();
        }
        let mut starts = vec![0];
        starts.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
        if text.is_empty() {
            starts.clear();
        }
        Self { text, starts, cut }
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    pub fn line(&self, i: usize) -> &str {
        let Some(&start) = self.starts.get(i) else { return "" };
        let end = self.starts.get(i + 1).map_or(self.text.len(), |e| e.saturating_sub(1));
        self.text.get(start..end).unwrap_or_default().trim_end_matches('\r')
    }
}

fn contains_ci(hay: &str, needle_lower: &str) -> bool {
    hay.to_lowercase().contains(needle_lower)
}

/// Marks the tree records and text lines that match `search`.
fn refresh_search(data: &mut Data, search: &str) {
    data.tree_hits.clear();
    data.ttx_hits.clear();
    data.diff_hits.clear();
    if search.is_empty() {
        return;
    }
    let needle = search.to_lowercase();
    if let Some(Pane::Ready(tree)) = &data.tree {
        let mut hits = vec![false; tree.nodes.len()];
        // Children come after their parents, so walking backwards marks every ancestor.
        for (i, node) in tree.nodes.iter().enumerate().rev() {
            let own = contains_ci(&node.name, &needle) || node.value.as_deref().is_some_and(|v| contains_ci(v, &needle));
            let hit = own || hits.get(i).copied().unwrap_or(false);
            if let Some(h) = hits.get_mut(i) {
                *h = hit;
            }
            if hit
                && let Some(p) = node.parent
                && let Some(h) = hits.get_mut(p as usize)
            {
                *h = true;
            }
        }
        data.tree_hits = hits;
    }
    let lines = |pane: &Option<Pane<Text>>| -> Vec<u32> {
        match pane {
            Some(Pane::Ready(text)) => {
                (0..text.len()).filter(|i| contains_ci(text.line(*i), &needle)).filter_map(|i| u32::try_from(i).ok()).collect()
            }
            _ => Vec::new(),
        }
    };
    data.ttx_hits = lines(&data.ttx);
    data.diff_hits = lines(&data.diff);
}

/// The tree rows to draw: open records' children, or with a search the records that match and
/// those above them.
fn visible_rows(tree: &Tree, expanded: &BTreeSet<u32>, hits: Option<&[bool]>) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack: Vec<u32> = tree.nodes.first().map(|r| r.children.iter().rev().copied().collect()).unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(node) = tree.nodes.get(id as usize) else { continue };
        let hit = |c: &u32| hits.is_none_or(|h| h.get(*c as usize).copied().unwrap_or(false));
        if !hit(&id) {
            continue;
        }
        out.push(id);
        let open = expanded.contains(&id) || hits.is_some_and(|_| node.children.iter().any(hit));
        if open {
            stack.extend(node.children.iter().rev().copied());
        }
    }
    out
}

// ---------- what the command returns ----------

fn node_json(node: &Node) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(node.name));
    o.insert("type".into(), json!(node.kind));
    if let Some(v) = &node.value {
        o.insert("value".into(), json!(v));
    }
    if node.more > 0 {
        o.insert("more".into(), json!(node.more));
    }
    Value::Object(o)
}

fn window_json(st: &TableViewerState) -> Value {
    let data = st.data.as_ref().map(|h| h.0.clone()).unwrap_or_default();
    let tables: Vec<Value> = data
        .tables
        .iter()
        .map(|t| {
            let mut o = Map::new();
            o.insert("tag".into(), json!(t.tag));
            o.insert("length".into(), json!(t.length));
            o.insert("checksum".into(), json!(t.checksum));
            o.insert("decoded".into(), json!(t.decoded));
            if let Some(e) = &t.error {
                o.insert("error".into(), json!(e));
            }
            if let Some(s) = &t.status {
                o.insert("status".into(), json!(s));
            }
            if let Some(f) = t.first_offset {
                o.insert("firstOffset".into(), json!(f));
            }
            Value::Object(o)
        })
        .collect();
    let searching = !st.search.is_empty();
    let text_json = |pane: &Option<Pane<Text>>, hits: &[u32]| -> Value {
        match pane {
            Some(Pane::Ready(t)) => {
                let mut o = json!({ "view": st.view, "lines": t.len(), "truncated": t.cut });
                if let (true, Some(obj)) = (searching, o.as_object_mut()) {
                    obj.insert("hits".into(), json!(hits.len()));
                }
                o
            }
            Some(Pane::Note(n)) => json!({ "view": st.view, "note": n }),
            None => json!({ "view": st.view }),
        }
    };
    let content = match st.view.as_str() {
        "ttx" => text_json(&data.ttx, &data.ttx_hits),
        "differences" => text_json(&data.diff, &data.diff_hits),
        _ => match &data.tree {
            Some(Pane::Ready(t)) => {
                let mut o = json!({ "view": "fields", "records": t.nodes.len(), "truncated": t.truncated });
                if let (true, Some(obj)) = (searching, o.as_object_mut()) {
                    obj.insert("hits".into(), json!(data.tree_hits.iter().skip(1).filter(|h| **h).count()));
                }
                o
            }
            Some(Pane::Note(n)) => json!({ "view": "fields", "note": n }),
            None => json!({ "view": "fields" }),
        },
    };
    let mut out = json!({
        "open": st.open,
        "font": st.font,
        "face": st.face,
        "compareWith": if st.compare_with.is_empty() { Value::Null } else { json!(st.compare_with) },
        "table": st.table,
        "view": st.view,
        "search": st.search,
        "tables": tables,
        "differing": data.compare.as_ref().map(|c| c.differing.clone()).unwrap_or_default(),
        "content": content,
    });
    let Some(obj) = out.as_object_mut() else { return out };
    if let Some(c) = &data.compare {
        obj.insert("identical".into(), json!(c.identical));
    }
    if let (Some(id), Some(Pane::Ready(tree))) = (st.selected, &data.tree)
        && let Some(node) = tree.nodes.get(id as usize)
    {
        let mut r = node_json(node);
        if let Some(r) = r.as_object_mut() {
            r.insert("record".into(), json!(st.record));
            if let Some(p) = path_of(tree, id) {
                r.insert("path".into(), json!(p));
            }
            if let Some((start, end)) = byte_range(tree, id, table_length(&data, &st.table)) {
                r.insert("offset".into(), json!(start));
                r.insert("size".into(), json!(end.saturating_sub(start)));
            }
            let kids: Vec<Value> =
                node.children.iter().take(MAX_RETURNED_RECORDS).filter_map(|c| tree.nodes.get(*c as usize)).map(node_json).collect();
            r.insert("children".into(), Value::Array(kids));
        }
        obj.insert("record".into(), r);
    }
    out
}

// ---------- drawing ----------

/// At most [`MAX_SHOWN_CHARS`] characters of `s`.
fn shown(s: &str) -> std::borrow::Cow<'_, str> {
    match s.char_indices().nth(MAX_SHOWN_CHARS) {
        Some((at, _)) => format!("{}…", s.get(..at).unwrap_or_default()).into(),
        None => s.into(),
    }
}

/// What the person did in the window this frame, applied after it is drawn.
#[derive(Default)]
struct Clicks {
    open_font: bool,
    open_compare: bool,
    toggle: Option<u32>,
}

/// A text field whose text starts as `value` and goes back to it when `value` changes.
fn path_field(ui: &mut egui::Ui, key: &str, value: &str, hint: &str, width: f32) -> Option<String> {
    let id = egui::Id::new(("window.tableViewer", key));
    let (mut seed, mut text): (String, String) = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    if seed != value {
        seed = value.to_string();
        text = value.to_string();
    }
    let resp = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(width));
    let entered = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.data_mut(|d| d.insert_temp(id, (seed, text.clone())));
    (entered && !text.is_empty()).then_some(text)
}

fn status_label(status: Option<&str>) -> &'static str {
    match status {
        Some("different") => "differs",
        Some("onlyA") => "only in this font",
        Some("onlyB") => "only in the compared font",
        Some("same") => "same",
        _ => "",
    }
}

fn draw_table_list(ui: &mut egui::Ui, tokens: &Tokens, st: &TableViewerState, data: &Data, pending: &mut Pending) {
    let line = tokens.list_row_height;
    let mono = FontId::monospace(tokens.font_size);
    let font = FontId::proportional(tokens.font_size);
    ui.horizontal(|ui| {
        for (label, width) in [("Tag", TAG_WIDTH), ("Size", SIZE_WIDTH), ("Checksum", CHECKSUM_WIDTH)] {
            ui.add_sized([width, line], egui::Label::new(egui::RichText::new(label).strong()));
        }
    });
    ui.separator();
    if data.tables.is_empty() {
        ui.colored_label(tokens.text_dim, "Open a compiled font to list its tables.");
        return;
    }
    egui::ScrollArea::vertical().id_salt("window.tableViewer.tables").auto_shrink([false, true]).max_height(line * VISIBLE_ROWS).show_rows(
        ui,
        line,
        data.tables.len(),
        |ui, range| {
            for row in data.tables.get(range).unwrap_or_default() {
                let (rect, resp) = ui.allocate_exact_size(vec2(LIST_WIDTH, line), Sense::click());
                let p = ui.painter_at(rect);
                let selected = row.tag == st.table;
                if selected {
                    p.rect_filled(rect, 0.0, tokens.row_selected);
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, tokens.hover);
                }
                let colour = match (selected, row.status.as_deref()) {
                    (true, _) => tokens.selected_text,
                    (_, Some("different" | "onlyA" | "onlyB")) => tokens.accent,
                    _ if !row.in_font => tokens.text_dim,
                    _ => tokens.text,
                };
                let y = rect.center().y;
                let mut x = rect.left() + tokens.cell_inset;
                p.text(pos2(x, y), Align2::LEFT_CENTER, &row.tag, mono.clone(), colour);
                x += TAG_WIDTH;
                if row.in_font {
                    p.text(pos2(x + SIZE_WIDTH - tokens.cell_inset * 2.0, y), Align2::RIGHT_CENTER, row.length.to_string(), font.clone(), colour);
                    x += SIZE_WIDTH;
                    p.text(pos2(x, y), Align2::LEFT_CENTER, &row.checksum, mono.clone(), colour);
                }
                let mark = if !row.in_font {
                    "+"
                } else if matches!(row.status.as_deref(), Some("different" | "onlyA")) {
                    "≠"
                } else if row.decoded {
                    ""
                } else {
                    "raw"
                };
                p.text(pos2(rect.right() - tokens.cell_inset, y), Align2::RIGHT_CENTER, mark, font.clone(), colour);
                let mut tip = Vec::new();
                tip.push(if row.decoded { "Decoded into fields".to_string() } else { "Not decoded into fields: see TTX".to_string() });
                if let Some(s) = row.status.as_deref() {
                    tip.push(status_label(Some(s)).to_string());
                }
                if let Some(o) = row.first_offset {
                    tip.push(format!("first difference at byte {o}"));
                }
                if let Some(e) = &row.error {
                    tip.push(e.clone());
                }
                let resp = resp.on_hover_text(tip.join("\n"));
                if resp.clicked() && !selected {
                    pending.push(("tableViewer.show", json!({ "table": row.tag })));
                }
            }
        },
    );
}

fn draw_tree(ui: &mut egui::Ui, tokens: &Tokens, st: &TableViewerState, tree: &Tree, hits: &[bool], pending: &mut Pending, clicks: &mut Clicks) {
    let line = tokens.list_row_height;
    let font = FontId::proportional(tokens.font_size);
    let mono = FontId::monospace(tokens.font_size);
    let searching = !st.search.is_empty();
    let rows = visible_rows(tree, &st.expanded, searching.then_some(hits));
    if rows.is_empty() {
        let note = if searching { "No record matches the search." } else { "The table has no records." };
        ui.colored_label(tokens.text_dim, note);
        return;
    }
    let width = ui.available_width().max(CONTENT_WIDTH * 0.5);
    egui::ScrollArea::vertical().id_salt("window.tableViewer.tree").auto_shrink([false, true]).max_height(line * VISIBLE_ROWS * 0.65).show_rows(
        ui,
        line,
        rows.len(),
        |ui, range| {
            for &id in rows.get(range).unwrap_or_default() {
                let Some(node) = tree.nodes.get(id as usize) else { continue };
                let (rect, resp) = ui.allocate_exact_size(vec2(width, line), Sense::click());
                let p = ui.painter_at(rect);
                let selected = st.selected == Some(id);
                if selected {
                    p.rect_filled(rect, 0.0, tokens.row_selected);
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, tokens.hover);
                }
                let colour = if selected { tokens.selected_text } else { tokens.text };
                let indent = f32::from(node.depth.saturating_sub(1)) * tokens.layers_indent;
                let x = rect.left() + tokens.cell_inset + indent;
                let y = rect.center().y;
                let can_open = !node.children.is_empty() || node.more > 0;
                let open = st.expanded.contains(&id) || (searching && !node.children.is_empty());
                let triangle = Rect::from_min_size(pos2(x, rect.top()), vec2(tokens.layers_indent, line));
                if can_open {
                    p.text(triangle.center(), Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, font.clone(), colour);
                }
                let mut label = node.name.clone();
                if node.more > 0 && node.children.is_empty() {
                    label.push_str(&format!("  ({} more)", node.more));
                }
                let name_end = p.text(pos2(triangle.right(), y), Align2::LEFT_CENTER, &label, font.clone(), colour).right();
                let kind_end =
                    p.text(pos2(name_end + tokens.cell_inset * 2.0, y), Align2::LEFT_CENTER, &node.kind, font.clone(), tokens.text_dim).right();
                if let Some(v) = &node.value {
                    p.text(pos2(kind_end + tokens.cell_inset * 2.0, y), Align2::LEFT_CENTER, shown(v), mono.clone(), colour);
                }
                if resp.clicked() {
                    let on_triangle = resp.interact_pointer_pos().is_some_and(|at| triangle.contains(at));
                    let record = record_of(tree, id);
                    if on_triangle && can_open {
                        if node.children.is_empty() {
                            pending.push(("tableViewer.show", json!({ "record": record, "expand": true })));
                        } else {
                            clicks.toggle = Some(id);
                        }
                    } else if !selected {
                        pending.push(("tableViewer.show", json!({ "record": record })));
                    }
                }
            }
        },
    );
    if tree.truncated {
        ui.colored_label(tokens.text_dim, format!("Some records are left out: the window holds up to {MAX_HELD_NODES}."));
    }
}

/// Draws a held text, its matching lines only while searching. `range` marks lines (the hex
/// pane's selected record); `diff` colours added and removed lines.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    salt: &str,
    text: &Text,
    hits: Option<&[u32]>,
    rows: f32,
    marked: Option<(usize, usize)>,
    scroll_to: Option<usize>,
    diff: bool,
) {
    let line = tokens.list_row_height;
    let count = hits.map_or(text.len(), <[u32]>::len);
    if count == 0 {
        ui.colored_label(tokens.text_dim, if hits.is_some() { "No line matches the search." } else { "The text is empty." });
        return;
    }
    let mut area = egui::ScrollArea::both().id_salt(salt).auto_shrink([false, true]).max_height(line * rows);
    if let Some(at) = scroll_to {
        area = area.vertical_scroll_offset(at as f32 * (line + ui.spacing().item_spacing.y));
    }
    let digits = text.len().max(1).ilog10() as usize + 1;
    let mono = FontId::monospace(tokens.font_size);
    area.show_rows(ui, line, count, |ui, range| {
        for row in range {
            let index = match hits {
                Some(h) => h.get(row).map_or(usize::MAX, |i| *i as usize),
                None => row,
            };
            let content = text.line(index);
            let colour = match (diff, content.as_bytes().first()) {
                (true, Some(b'+')) => tokens.accent,
                (true, Some(b'-')) => tokens.error_text,
                (true, Some(b'@')) => tokens.text_dim,
                _ => tokens.text,
            };
            let marked = marked.is_some_and(|(a, b)| (a..=b).contains(&index));
            let colour = if marked { tokens.selected_text } else { colour };
            let galley = ui.painter().layout_no_wrap(format!("{:>digits$}  {}", index.saturating_add(1), shown(content)), mono.clone(), colour);
            let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x.max(ui.available_width()), line), Sense::hover());
            if marked {
                ui.painter().rect_filled(rect, 0.0, tokens.row_selected);
            }
            ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, colour);
        }
    });
    if text.cut {
        ui.colored_label(tokens.text_dim, "Part of the text is left out.");
    }
}

fn draw_note(ui: &mut egui::Ui, tokens: &Tokens, note: &str) {
    ui.add(egui::Label::new(egui::RichText::new(note).color(tokens.text_dim)).wrap());
}

fn draw_content(ui: &mut egui::Ui, tokens: &Tokens, st: &TableViewerState, data: &Data, pending: &mut Pending, clicks: &mut Clicks) {
    ui.horizontal(|ui| {
        for (id, label) in VIEWS {
            if id == "differences" && st.compare_with.is_empty() {
                continue;
            }
            if ui.selectable_label(st.view == id, label).clicked() && st.view != id {
                pending.push(("tableViewer.show", json!({ "view": id })));
            }
        }
        ui.add_space(tokens.dialog_spacing);
        let id = egui::Id::new(("window.tableViewer", "search"));
        let (mut seed, mut text): (String, String) = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        if seed != st.search {
            seed = st.search.clone();
            text = st.search.clone();
        }
        let resp = ui.add(egui::TextEdit::singleline(&mut text).hint_text("Search names and values").desired_width(tokens.dialog_field_width));
        if resp.changed() {
            pending.push(("tableViewer.show", json!({ "search": text })));
        }
        ui.data_mut(|d| d.insert_temp(id, (seed, text)));
    });
    ui.add_space(tokens.dialog_spacing);
    if st.table.is_empty() {
        ui.colored_label(tokens.text_dim, "Select a table.");
        return;
    }
    let searching = !st.search.is_empty();
    match st.view.as_str() {
        "ttx" => match &data.ttx {
            Some(Pane::Ready(t)) => {
                draw_text(ui, tokens, "window.tableViewer.ttx", t, searching.then_some(data.ttx_hits.as_slice()), VISIBLE_ROWS, None, None, false)
            }
            Some(Pane::Note(n)) => draw_note(ui, tokens, n),
            None => draw_note(ui, tokens, "Reading…"),
        },
        "differences" => match &data.diff {
            Some(Pane::Ready(t)) => {
                draw_text(ui, tokens, "window.tableViewer.diff", t, searching.then_some(data.diff_hits.as_slice()), VISIBLE_ROWS, None, None, true)
            }
            Some(Pane::Note(n)) => draw_note(ui, tokens, n),
            None => draw_note(ui, tokens, "Reading…"),
        },
        _ => {
            match &data.tree {
                Some(Pane::Ready(tree)) => draw_tree(ui, tokens, st, tree, &data.tree_hits, pending, clicks),
                Some(Pane::Note(n)) => draw_note(ui, tokens, n),
                None => draw_note(ui, tokens, "Reading…"),
            }
            ui.separator();
            let range = match (&data.tree, st.selected) {
                (Some(Pane::Ready(tree)), Some(id)) => byte_range(tree, id, table_length(data, &st.table)),
                _ => None,
            };
            match range {
                Some((start, end)) => ui.colored_label(tokens.text_dim, format!("Bytes {start} to {end} ({} bytes)", end.saturating_sub(start))),
                None => ui.colored_label(tokens.text_dim, "Bytes"),
            };
            match &data.hex {
                Some(Pane::Ready(hex)) => {
                    let marked = range.map(|(start, end)| (hex_line(start), hex_line(end.saturating_sub(1).max(start))));
                    draw_text(ui, tokens, "window.tableViewer.hex", hex, None, HEX_ROWS, marked, st.hex_scroll, false);
                }
                Some(Pane::Note(n)) => draw_note(ui, tokens, n),
                None => {}
            }
        }
    }
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let st = app.state.panels.table_viewer.clone();
    if !st.open {
        return;
    }
    let data = st.data.as_ref().map(|h| h.0.clone()).unwrap_or_default();
    let can_pick = app.picker.is_some();
    let tokens = &app.tokens;
    let mut clicks = Clicks::default();
    let mut open = true;
    egui::Window::new("Table Viewer")
        .id(egui::Id::new("window.tableViewer"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(LIST_WIDTH + CONTENT_WIDTH + 3.0 * tokens.dialog_spacing)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Font");
                if let Some(path) = path_field(ui, "font", &st.font, "Font file", tokens.dialog_field_width * 2.0) {
                    pending.push(("tableViewer.show", json!({ "font": path })));
                }
                if can_pick && ui.button("Open…").clicked() {
                    clicks.open_font = true;
                }
                if !st.font.is_empty() && ui.button("Reload").on_hover_text("Read the font files again").clicked() {
                    let mut params = json!({ "font": st.font, "face": st.face });
                    if let (false, Some(obj)) = (st.compare_with.is_empty(), params.as_object_mut()) {
                        obj.insert("compareWith".into(), json!(st.compare_with));
                    }
                    pending.push(("tableViewer.show", params));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Compare With");
                if let Some(path) = path_field(ui, "compare", &st.compare_with, "Second font file", tokens.dialog_field_width * 2.0) {
                    pending.push(("tableViewer.show", json!({ "compareWith": path })));
                }
                if can_pick && ui.button("Compare With…").clicked() {
                    clicks.open_compare = true;
                }
                if !st.compare_with.is_empty() && ui.button("Stop Comparing").clicked() {
                    pending.push(("tableViewer.show", json!({ "compareWith": null })));
                }
            });
            if let Some(c) = &data.compare {
                let summary = if c.identical {
                    "The fonts have the same tables.".to_string()
                } else {
                    format!("Tables that differ: {}", c.differing.join(", "))
                };
                ui.colored_label(if c.identical { tokens.text_dim } else { tokens.accent }, summary);
            }
            ui.add_space(tokens.dialog_spacing);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(LIST_WIDTH);
                    draw_table_list(ui, tokens, &st, &data, pending);
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_width(CONTENT_WIDTH);
                    draw_content(ui, tokens, &st, &data, pending, &mut clicks);
                });
            });
        });
    let state = &mut app.state.panels.table_viewer;
    if !open {
        state.open = false;
    }
    state.hex_scroll = None;
    if let Some(id) = clicks.toggle
        && !state.expanded.remove(&id)
    {
        state.expanded.insert(id);
    }
    for (asked, key, title) in [(clicks.open_font, "font", "Open Font"), (clicks.open_compare, "compareWith", "Compare With")] {
        if !asked {
            continue;
        }
        let Some(picker) = app.picker.as_mut() else { continue };
        if let Some(file) = picker.files(title, "Font", &FONT_EXTENSIONS).first() {
            let mut params = Map::new();
            params.insert(key.into(), json!(file.to_string_lossy()));
            pending.push(("tableViewer.show", Value::Object(params)));
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::path::PathBuf;

    use super::*;

    /// Two fonts exported from code: A a 500-unit square in the first, a 300-unit one in the
    /// second, so `glyf`, `hmtx` and the tables that hold their sums differ.
    fn two_fonts(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("fontcraft-table-viewer-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let export = |size: u32, file: &str| {
            let path = dir.join(file);
            let mut src = FontcraftApp::default();
            src.run("path.addRectangle", &json!({"glyph": "A", "x": 0, "y": 0, "width": size, "height": size})).unwrap();
            src.run("file.exportStatic", &json!({"destination": path})).unwrap();
            path
        };
        (export(500, "First.ttf"), export(300, "Second.ttf"))
    }

    fn draw(app: &mut FontcraftApp) {
        let ctx = egui::Context::default();
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), ..Default::default() };
        // Two frames: the window's size settles on the second.
        for _ in 0..2 {
            ctx.run_ui(input.clone(), |ui| app.ui(ui)).textures_delta.clear();
        }
    }

    fn names(list: &Value) -> Vec<String> {
        list.as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn table_viewer_show_lists_what_tables_show_reads_and_compare_with_marks_what_font_diff_finds() {
        let (first, second) = two_fonts("compare");
        let (a, b) = (first.to_string_lossy().into_owned(), second.to_string_lossy().into_owned());
        let mut app = FontcraftApp::default();
        let out = app.run("tableViewer.show", &json!({"font": a})).unwrap();
        assert_eq!(out["open"], true);

        // The list is tables.show's: tag, length, checksum and whether it decodes.
        let listed = app.session.query("tables.show", &json!({"font": a, "depth": 0})).unwrap();
        let expected: Vec<Value> = listed["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| json!({"tag": t["tag"], "length": t["length"], "checksum": t["checksum"], "decoded": t["decoded"]}))
            .collect();
        assert_eq!(out["tables"], json!(expected));
        assert_eq!(out["table"], listed["tables"][0]["tag"], "the first table is selected");
        draw(&mut app);

        // A table's fields are tables.show's tree of it.
        let out = app.run("tableViewer.show", &json!({"table": "hmtx", "record": []})).unwrap();
        let tree = app.session.query("tables.show", &json!({"font": a, "tables": ["hmtx"], "depth": 1})).unwrap();
        assert_eq!(names(&out["record"]["children"]), names(&tree["tables"][0]["tree"]["children"]));
        assert_eq!(out["record"]["offset"], 0);
        assert!(out["content"]["records"].as_u64().unwrap() > 1);
        draw(&mut app);

        // Refused changes leave the window as it was.
        let before = app.state.panels.table_viewer.clone();
        for bad in [
            json!({"table": "zzzz"}),
            json!({"view": "differences"}),
            json!({"view": "hexadecimal"}),
            json!({"font": a.replace("First", "Missing")}),
            json!({"path": "hmtx.nothing"}),
            json!({"record": [100000]}),
        ] {
            assert!(app.run("tableViewer.show", &bad).is_err(), "{bad} is refused");
            assert_eq!(app.state.panels.table_viewer, before, "{bad} changes nothing");
        }

        // Compare With marks the tables font.diff finds different.
        let out = app.run("tableViewer.show", &json!({"compareWith": b})).unwrap();
        let diff = app.session.query("font.diff", &json!({"a": a, "b": b, "detail": "offset"})).unwrap();
        assert_eq!(out["differing"], diff["differing"]);
        assert_eq!(out["identical"], false);
        let differing: Vec<&str> = diff["differing"].as_array().unwrap().iter().map(|d| d.as_str().unwrap()).collect();
        assert!(differing.contains(&"glyf"), "{differing:?}");
        for t in out["tables"].as_array().unwrap() {
            let tag = t["tag"].as_str().unwrap();
            assert_eq!(t["status"], diff["tables"][tag]["status"], "{tag}");
        }
        draw(&mut app);

        // The Differences view is font.diff's comparison of the selected table.
        let out = app.run("tableViewer.show", &json!({"table": "glyf", "view": "differences"})).unwrap();
        let lines = app.session.query("font.diff", &json!({"a": a, "b": b, "tables": ["glyf"]})).unwrap();
        assert_eq!(out["content"]["lines"], json!(lines["tables"]["glyf"]["lines"].as_array().unwrap().len()));
        draw(&mut app);

        // Stopping the comparison removes the marks and leaves the Differences view.
        let out = app.run("tableViewer.show", &json!({"compareWith": null})).unwrap();
        assert_eq!(out["view"], "fields");
        assert_eq!(out["differing"], json!([]));
        assert!(out["tables"].as_array().unwrap().iter().all(|t| t.get("status").is_none()));
        let _ = std::fs::remove_dir_all(first.parent().unwrap());
    }

    #[test]
    fn table_viewer_show_reads_records_below_the_held_depth_and_search_and_ttx_filter_them() {
        let (first, _) = two_fonts("records");
        let a = first.to_string_lossy().into_owned();
        let mut app = FontcraftApp::default();
        app.run("tableViewer.show", &json!({"font": a, "table": "glyf"})).unwrap();

        // glyf › glyphs › a glyph › its fields is one level more than the window reads at first.
        let full = app.session.query("tables.show", &json!({"font": a, "tables": ["glyf"], "depth": 4})).unwrap();
        let glyphs = &full["tables"][0]["tree"]["children"][0];
        let index = glyphs["children"].as_array().unwrap().iter().position(|g| g["children"].is_array()).expect("a glyph with outlines");
        let glyph = &glyphs["children"][index];
        let held = &app.state.panels.table_viewer.data.as_ref().unwrap().0;
        let Some(Pane::Ready(tree)) = &held.tree else { panic!("the tree is read") };
        let before = tree.nodes.len();
        let out = app.run("tableViewer.show", &json!({"record": [0, index], "expand": true})).unwrap();
        assert_eq!(out["record"]["name"], glyph["name"]);
        assert_eq!(names(&out["record"]["children"]), names(&glyph["children"]));
        assert!(out["content"]["records"].as_u64().unwrap() > before as u64, "the glyph's fields are read");
        let st = &app.state.panels.table_viewer;
        assert!(st.expanded.contains(&st.selected.unwrap()));
        draw(&mut app);

        // A path selects the same record; one beyond the table is refused.
        let path = format!("glyf.glyphs[{index}]");
        if path_part(glyph["name"].as_str().unwrap()).is_none() {
            assert!(app.run("tableViewer.show", &json!({"path": format!("glyf.glyphs.{}", glyph["name"].as_str().unwrap())})).is_err());
        }
        if glyph["name"] != json!(index.to_string()) {
            assert!(app.run("tableViewer.show", &json!({"path": path})).is_err(), "glyph records are named, not numbered");
        }

        // The search keeps the records whose name or value matches, and the ones above them.
        let out = app.run("tableViewer.show", &json!({"search": "XMAX"})).unwrap();
        assert!(out["content"]["hits"].as_u64().unwrap() >= 3, "xMax, the glyph and glyphs: {}", out["content"]);
        let st = &app.state.panels.table_viewer;
        let data = &st.data.as_ref().unwrap().0;
        let Some(Pane::Ready(tree)) = &data.tree else { panic!() };
        let rows = visible_rows(tree, &st.expanded, Some(&data.tree_hits));
        assert!(rows.iter().all(|id| data.tree_hits[*id as usize]));
        assert!(rows.iter().any(|id| tree.nodes[*id as usize].name == "xMax"));
        draw(&mut app);

        // The TTX view is font.dumpTables' text, and the search keeps its matching lines.
        let out = app.run("tableViewer.show", &json!({"view": "ttx", "search": "TTGlyph"})).unwrap();
        let ttx = app.session.query("font.dumpTables", &json!({"font": a, "tables": ["glyf"]})).unwrap();
        let text = ttx["text"].as_str().unwrap();
        let lines: Vec<&str> = text.strip_suffix('\n').unwrap_or(text).split('\n').collect();
        assert_eq!(out["content"]["lines"], json!(lines.len()));
        assert_eq!(out["content"]["hits"], json!(lines.iter().filter(|l| l.to_lowercase().contains("ttglyph")).count()));
        assert!(out["content"]["hits"].as_u64().unwrap() > 0);
        draw(&mut app);
        let out = app.run("tableViewer.show", &json!({"search": null})).unwrap();
        assert!(out["content"].get("hits").is_none());
        let _ = std::fs::remove_dir_all(first.parent().unwrap());
    }

    #[test]
    fn table_viewer_text_and_tree_helpers_keep_their_limits() {
        let text = Text::new("one\ntwo\r\nthree\n".into(), false);
        assert_eq!((text.len(), text.line(0), text.line(1), text.line(2), text.line(3)), (3, "one", "two", "three", ""));
        assert!(Text::new(String::new(), false).is_empty());
        let big = Text::new("x\n".repeat(MAX_HELD_TEXT / 2 + 10), false);
        assert!(big.cut && big.text.len() <= MAX_HELD_TEXT);

        assert_eq!(path_part("lookups"), Some(".lookups".into()));
        assert_eq!(path_part("3"), Some("[3]".into()));
        assert_eq!(path_part("A.alt (3)"), None);
        assert_eq!(hex_line(0), 1);
        assert_eq!(hex_line(33), 3);

        // A record without an offset takes its parent's; the next sibling's offset ends it.
        let mut tree = Tree::default();
        tree.nodes.push(Node { name: "GSUB".into(), offset: Some(0), ..Node::default() });
        fill(
            &mut tree,
            0,
            &json!({"name": "GSUB", "type": "table", "offset": 0, "children": [
                {"name": "a", "type": "record", "offset": 10, "children": [{"name": "x", "type": "uint16", "value": "1"}]},
                {"name": "b", "type": "record", "offset": 40, "more": 2}
            ]}),
        );
        // Siblings are numbered before their children: a 1, b 2, x 3.
        assert_eq!(byte_range(&tree, 3, 100), Some((10, 40)));
        assert_eq!(byte_range(&tree, 2, 100), Some((40, 100)));
        assert_eq!(path_of(&tree, 3), Some("GSUB.a.x".into()));
        assert_eq!(record_of(&tree, 2), vec![1]);
        merge(&mut tree, 0, &json!({"name": "GSUB", "children": [{"name": "a"}, {"name": "b", "children": [{"name": "0"}, {"name": "1"}]}]}));
        assert_eq!(tree.nodes[2].children.len(), 2, "b's children are read");
        assert_eq!((tree.nodes[2].more, tree.nodes[1].children.clone()), (0, vec![3]), "a's are kept");
        assert_eq!(path_of(&tree, tree.nodes[2].children[1]), Some("GSUB.b[1]".into()));
    }
}

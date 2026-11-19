//! The Table Viewer (M12.38): a compiled font's tables as a tree of decoded fields with a hex pane, the TTX text of a
//! table, and a second font compared table by table. This module holds its view state and the command
//! `tableViewer.show`; the panel is `panels::table_viewer`, its view model `app::models::table_viewer`.
//!
//! Every view is what an engine query returns: the table list and the tree are `tables.show`, the TTX and hex texts
//! `font.dumpTables`, the marks and the Differences view `font.diff`. They are read when the command changes a
//! parameter, which also shows the panel, and kept until the next change, so editing the file on disk shows only
//! after Reload. A tree is read two levels at a time and at most [`MAX_HELD_NODES`] records are held; texts are held
//! up to [`MAX_HELD_TEXT`] bytes.

use std::collections::BTreeSet;
use std::sync::Arc;

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, quoted, string};
use crate::app::FontCraftApp;

/// The views of the selected table: the `view` parameter.
pub const VIEWS: [&str; 3] = ["fields", "ttx", "differences"];

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

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "tableViewer.show",
    label: "Table Viewer",
    menu: &["Window"],
    shortcut: None,
    params: "{font?: path, face?: number, compareWith?: path|null, table?: tag, view?: \"fields\"|\"ttx\"|\"differences\", path?: string, record?: [index], expand?: bool, search?: string, reload?: bool} show the Table Viewer. font reads the font's table list (tables.show) and keeps the selected table when the font has it; reload reads the font and the compared font again; compareWith compares the font with a second one table by table (font.diff) and marks the tables that differ, null stops comparing; table selects a table and reads it for the view: fields is the tree of tables.show with the hex pane (font.dumpTables hex), ttx the TTX text (font.dumpTables), differences the decoded comparison of that table (font.diff); path (as tables.show writes it, GPOS.lookupList.lookups[3]) or record (child positions from the table's root, [2, 0]) selects a record, reading deeper levels when needed; expand true also reads and opens its children, false closes it; search keeps the records or lines whose name or value contains the text, ignoring case. A refused change leaves the viewer as it was. ui.inspect reports it under tableViewer",
}];

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
#[derive(PlainData, Clone, Debug, Default, PartialEq, Eq)]
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

/// The viewer's state, in `ViewState::table_viewer`.
#[derive(Clone, Debug, PartialEq)]
pub struct TableViewerState {
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
    /// The selected record's node in the held tree.
    pub selected: Option<u32>,
    /// Tree records shown open.
    pub expanded: BTreeSet<u32>,
    pub data: Option<Held>,
    /// Moves with every accepted `tableViewer.show`: the view model copies the state again only then.
    pub generation: u64,
}

impl Default for TableViewerState {
    fn default() -> Self {
        Self {
            font: String::new(),
            face: 0,
            compare_with: String::new(),
            table: String::new(),
            view: "fields".into(),
            search: String::new(),
            record: Vec::new(),
            selected: None,
            expanded: BTreeSet::new(),
            data: None,
            generation: 0,
        }
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "tableViewer.show" => show_command(app, &command.params),
        other => Err(format!("no Table Viewer command `{other}`")),
    }
}

/// Whether the Table Viewer shows.
pub fn shows(app: &FontCraftApp) -> bool {
    app.view.workspace.layout.panel_state(&PanelId::TableViewer) != craft::behavior::dock::PanelState::Hidden
}

// ---------- the command ----------

/// At most 64 characters of untrusted text, for a message.
fn clip(s: &str) -> String {
    quoted(s)
}

fn text_param<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    let s = string(p, key)?;
    if s.is_some_and(|s| s.len() > MAX_PARAM_TEXT) {
        return Err(format!("`{key}` is longer than {MAX_PARAM_TEXT} bytes"));
    }
    Ok(s)
}

/// The value of `key` as given, `null` included: for parameters where `null` clears.
fn given<'a>(p: &'a Value, key: &str) -> Option<&'a Value> {
    p.get(key)
}

fn index(p: &Value, key: &str) -> Result<Option<u64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| format!("`{key}` must be a whole number")),
    }
}

fn query(app: &mut FontCraftApp, id: &str, params: &Value) -> Result<Value, String> {
    app.session.query(id, params).map_err(|e| e.to_string())
}

fn show_command(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    if !p.is_null() && !p.is_object() {
        return Err("the parameters must be a JSON object".into());
    }
    // Work on a copy: a refused change leaves the viewer as it was.
    let mut st = app.view.table_viewer.clone();
    let mut data = st.data.as_ref().map(|h| (*h.0).clone()).unwrap_or_default();

    let font = text_param(p, "font")?;
    let face = index(p, "face")?;
    let reload = flag(p, "reload")?.unwrap_or(false);
    if let Some(font) = font {
        if font.is_empty() {
            return Err("`font` is empty: give a font file".into());
        }
        st.font = font.to_string();
        st.face = face.unwrap_or(0);
    } else if let Some(face) = face {
        st.face = face;
    }
    let reload = reload || font.is_some() || face.is_some();
    if reload {
        if st.font.is_empty() {
            return Err("open a font first: give `font`".into());
        }
        let tables = load_tables(app, &st.font, st.face)?;
        data = Data { tables, ..Data::default() };
        clear_table(&mut st, &mut data);
    }

    let stop = |st: &mut TableViewerState, data: &mut Data| {
        st.compare_with.clear();
        data.compare = None;
        data.diff = None;
        if st.view == "differences" {
            st.view = "fields".into();
        }
    };
    match given(p, "compareWith") {
        Some(Value::Null) => stop(&mut st, &mut data),
        Some(Value::String(other)) if other.is_empty() => stop(&mut st, &mut data),
        Some(Value::String(other)) => {
            if other.len() > MAX_PARAM_TEXT {
                return Err(format!("`compareWith` is longer than {MAX_PARAM_TEXT} bytes"));
            }
            st.compare_with = other.clone();
            data.diff = None;
        }
        Some(_) => return Err("`compareWith` must be a font file or null".into()),
        None => {}
    }
    if !st.compare_with.is_empty() && (reload || given(p, "compareWith").is_some()) {
        if st.font.is_empty() {
            return Err("open a font before comparing it".into());
        }
        let (compare, statuses) = load_compare(app, &st.font, st.face, &st.compare_with)?;
        apply_compare(&mut data.tables, &statuses);
        data.compare = Some(compare);
    } else if st.compare_with.is_empty() {
        apply_compare(&mut data.tables, &[]);
    }

    if st.table.is_empty() || !data.tables.iter().any(|t| t.tag == st.table) {
        st.table = data.tables.iter().find(|t| t.in_font).map(|t| t.tag.clone()).unwrap_or_default();
        clear_table(&mut st, &mut data);
    }
    if let Some(table) = text_param(p, "table")? {
        let tag = table.trim_end();
        if !data.tables.iter().any(|t| t.tag == tag) {
            return Err(format!("the font has no table `{}`", clip(table)));
        }
        if st.table != tag {
            st.table = tag.to_string();
            clear_table(&mut st, &mut data);
        }
    }

    if let Some(view) = text_param(p, "view")? {
        if !VIEWS.contains(&view) {
            return Err(format!("unknown view `{}`: use fields, ttx or differences", clip(view)));
        }
        if view == "differences" && st.compare_with.is_empty() {
            return Err("the Differences view needs a second font: give `compareWith`".into());
        }
        st.view = view.to_string();
    }
    let path = text_param(p, "path")?;
    let record = match p.get("record").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(record_param(v)?),
    };
    if path.is_some() || record.is_some() {
        st.view = "fields".into();
    }
    if let Some(path) = path {
        let first = path.split(['.', '[']).next().unwrap_or_default();
        if first.is_empty() || !data.tables.iter().any(|t| t.tag == first && t.in_font) {
            return Err(format!("`path` must start with a table of the font, got `{}`", clip(path)));
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
            return Err(format!("cannot select a record: {why}"));
        };
        let wanted = match (path, &record) {
            (Some(path), _) => Target::Names(parse_path(path)?),
            (None, Some(indices)) => Target::Positions(indices),
            (None, None) => Target::Positions(&[]),
        };
        let expand_flag = flag(p, "expand")?;
        let open = expand_flag == Some(true);
        // A record already held is selected without copying the tree.
        let held =
            held_target(&shared, &wanted).filter(|id| !open || shared.nodes.get(*id as usize).is_some_and(|n| !n.children.is_empty() || n.more == 0));
        let (tree, target) = match held {
            Some(id) => (shared, id),
            None => {
                let mut tree = (*shared).clone();
                let id = select(app, &st, &mut tree, &wanted)?;
                if open {
                    expand(app, &st, &mut tree, id, 0)?;
                }
                (Arc::new(tree), id)
            }
        };
        if expand_flag == Some(false) {
            // Closing a record leaves the selection where it is.
            st.expanded.remove(&target);
        } else {
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
        }
        data.tree = Some(Pane::Ready(tree));
    }

    if let Some(search) = given(p, "search") {
        match search {
            Value::Null => st.search.clear(),
            Value::String(s) if s.len() <= MAX_PARAM_TEXT => st.search = s.clone(),
            Value::String(_) => return Err(format!("`search` is longer than {MAX_PARAM_TEXT} bytes")),
            _ => return Err("`search` must be a string".into()),
        }
    }
    refresh_search(&mut data, &st.search);

    st.data = Some(Held(Arc::new(data)));
    st.generation = st.generation.wrapping_add(1);
    app.view.table_viewer = st;
    // A refused move (the layout has no room) leaves the panel where it was.
    let _ = app.view.workspace.set_visible(PanelId::TableViewer, true, None, false, |panel| panel.permissions());
    Ok(())
}

fn record_param(v: &Value) -> Result<Vec<u32>, String> {
    let bad = || "`record` must be a list of child positions, as [2, 0]".to_string();
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_RECORD_DEPTH {
        return Err(format!("`record` holds at most {MAX_RECORD_DEPTH} positions"));
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
}

pub fn table_length(data: &Data, tag: &str) -> u64 {
    data.tables.iter().find(|t| t.tag == tag).map_or(0, |t| t.length)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The table list: `tables.show` with no levels below each table.
fn load_tables(app: &mut FontCraftApp, font: &str, face: u64) -> Result<Vec<TableRow>, String> {
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
fn load_compare(app: &mut FontCraftApp, font: &str, face: u64, other: &str) -> Result<(Compare, Vec<Status>), String> {
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
fn load_view(app: &mut FontCraftApp, st: &TableViewerState, data: &mut Data) {
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
fn dump(app: &mut FontCraftApp, st: &TableViewerState, format: &str) -> Pane<Text> {
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
fn load_diff(app: &mut FontCraftApp, st: &TableViewerState) -> Pane<Text> {
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
fn load_tree(app: &mut FontCraftApp, st: &TableViewerState, path: &str, depth: u64) -> Result<(Value, bool), String> {
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
fn expand(app: &mut FontCraftApp, st: &TableViewerState, tree: &mut Tree, target: u32, extra: u64) -> Result<(), String> {
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
fn select(app: &mut FontCraftApp, st: &TableViewerState, tree: &mut Tree, target: &Target) -> Result<u32, String> {
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
pub fn byte_range(tree: &Tree, id: u32, table_length: u64) -> Option<(u64, u64)> {
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
pub fn hex_line(offset: u64) -> usize {
    usize::try_from(offset / 16).unwrap_or(usize::MAX).saturating_add(1)
}

impl Text {
    /// Splits `text` into lines, cut at [`MAX_HELD_TEXT`] on a line end, or on a character
    /// boundary when that much text holds no line end.
    pub fn new(mut text: String, cut: bool) -> Self {
        let mut cut = cut;
        if text.len() > MAX_HELD_TEXT {
            let end = text
                .as_bytes()
                .get(..MAX_HELD_TEXT)
                .and_then(|b| b.iter().rposition(|c| *c == b'\n'))
                .unwrap_or_else(|| (MAX_HELD_TEXT.saturating_sub(3)..=MAX_HELD_TEXT).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0));
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

/// A tree row to draw: the node and its child positions from the table's root.
pub type Visible = (u32, Vec<u32>);

/// The tree rows to draw, at most `max`: open records' children, or with a search the records that match and those
/// above them.
pub fn visible_rows(tree: &Tree, expanded: &BTreeSet<u32>, hits: Option<&[bool]>, max: usize) -> Vec<Visible> {
    let mut out = Vec::new();
    let roots = tree.nodes.first().map(|r| r.children.as_slice()).unwrap_or_default();
    let mut stack: Vec<Visible> = roots.iter().enumerate().rev().map(|(at, id)| (*id, vec![u32::try_from(at).unwrap_or(u32::MAX)])).collect();
    let hit = |c: &u32| hits.is_none_or(|h| h.get(*c as usize).copied().unwrap_or(false));
    while let Some((id, record)) = stack.pop() {
        if out.len() >= max {
            break;
        }
        let Some(node) = tree.nodes.get(id as usize) else { continue };
        if !hit(&id) {
            continue;
        }
        let open = expanded.contains(&id) || hits.is_some_and(|_| node.children.iter().any(hit));
        if open {
            for (at, kid) in node.children.iter().enumerate().rev() {
                let mut path = record.clone();
                path.push(u32::try_from(at).unwrap_or(u32::MAX));
                stack.push((*kid, path));
            }
        }
        out.push((id, record));
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

/// The viewer as `ui.inspect` reports it, under `tableViewer`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    state.insert("tableViewer".into(), viewer_json(&app.view.table_viewer, shows(app)));
}

fn viewer_json(st: &TableViewerState, open: bool) -> Value {
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
        "open": open,
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
pub fn shown(s: &str) -> std::borrow::Cow<'_, str> {
    match s.char_indices().nth(MAX_SHOWN_CHARS) {
        Some((at, _)) => format!("{}…", s.get(..at).unwrap_or_default()).into(),
        None => s.into(),
    }
}

/// What the person did in the window this frame, applied after it is drawn.

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::path::PathBuf;
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use super::*;
    use crate::app::{APP, Msg};

    /// Two fonts exported from code: A a 500-unit square in the first, a 300-unit one in the second, so `glyf`,
    /// `hmtx` and the tables that hold their sums differ.
    fn two_fonts(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-table-viewer-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let export = |size: u32, file: &str| {
            let path = dir.join(file);
            let mut s = Session::with_document();
            s.execute("path.addRectangle", &json!({"glyph": "A", "x": 0, "y": 0, "width": size, "height": size})).unwrap();
            s.execute("file.exportStatic", &json!({"destination": path})).unwrap();
            path
        };
        (export(500, "First.ttf"), export(300, "Second.ttf"))
    }

    fn viewer(app: &mut FontCraftApp, params: Value) -> Result<Value, String> {
        app.run_now(Msg::Shell(CommandRef::with_params("tableViewer.show", params)), false)?;
        let mut state = Map::new();
        inspect(app, &mut state);
        Ok(state.remove("tableViewer").unwrap())
    }

    #[test]
    fn table_viewer_shows_what_tables_show_and_font_diff_read_and_refused_changes_change_nothing() {
        let (first, second) = two_fonts("compare");
        let (a, b) = (first.to_string_lossy().into_owned(), second.to_string_lossy().into_owned());
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let out = viewer(&mut app, json!({"font": a})).unwrap();
        assert_eq!(out["open"], true, "the command shows the panel");
        let listed = app.session.query("tables.show", &json!({"font": a, "depth": 0})).unwrap();
        let tags = |v: &Value| v.as_array().unwrap().iter().map(|t| t["tag"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(tags(&out["tables"]), tags(&listed["tables"]));
        assert_eq!(out["table"], listed["tables"][0]["tag"], "the first table is selected");
        assert_eq!(app.models.table_viewer.tables.len(), listed["tables"].as_array().unwrap().len(), "the model lists them");

        // A record by path: the tree of tables.show, the hex pane marks its bytes.
        let out = viewer(&mut app, json!({"path": "hmtx"})).unwrap();
        assert_eq!(out["table"], "hmtx");
        let tree = app.session.query("tables.show", &json!({"font": a, "tables": ["hmtx"], "depth": 1})).unwrap();
        let names = |v: &Value| v.as_array().unwrap().iter().map(|c| c["name"].clone()).collect::<Vec<_>>();
        assert_eq!(names(&out["record"]["children"]), names(&tree["tables"][0]["tree"]["children"]));
        assert_eq!(out["record"]["offset"], 0);
        let model = &app.models.table_viewer;
        assert_eq!(model.rows.len(), tree["tables"][0]["tree"]["children"].as_array().unwrap().len(), "the table's children are in view");
        assert!(model.hex.text.is_some() && model.bytes.is_some(), "{:?}", model.bytes);

        // Opening a record reads its children; closing it keeps the selection.
        let first_row = app.models.table_viewer.rows.first().cloned().unwrap();
        let shown = app.models.table_viewer.rows.len();
        if first_row.has_children {
            viewer(&mut app, json!({"record": first_row.record, "expand": true})).unwrap();
            assert!(app.models.table_viewer.rows.len() > shown);
            viewer(&mut app, json!({"record": first_row.record, "expand": false})).unwrap();
            assert!(!app.models.table_viewer.rows.first().unwrap().open);
            assert_eq!(app.models.table_viewer.rows.len(), shown);
        }

        // Refused changes leave the viewer as it was.
        let before = app.view.table_viewer.clone();
        for bad in [
            json!({"table": "zzzz"}),
            json!({"view": "differences"}),
            json!({"view": "hexadecimal"}),
            json!({"font": a.replace("First", "Missing")}),
            json!({"path": "hmtx.nothing"}),
            json!({"record": [100000]}),
        ] {
            assert!(viewer(&mut app, bad.clone()).is_err(), "{bad} is refused");
            assert_eq!(app.view.table_viewer, before, "{bad} changes nothing");
        }

        // Compare With marks the tables font.diff finds different; Differences is its comparison of a table.
        let out = viewer(&mut app, json!({"compareWith": b})).unwrap();
        let diff = app.session.query("font.diff", &json!({"a": a, "b": b, "detail": "offset"})).unwrap();
        assert_eq!(out["differing"], diff["differing"]);
        assert_eq!(out["identical"], false);
        let out = viewer(&mut app, json!({"table": "glyf", "view": "differences"})).unwrap();
        let lines = app.session.query("font.diff", &json!({"a": a, "b": b, "tables": ["glyf"]})).unwrap();
        assert_eq!(out["content"]["lines"], json!(lines["tables"]["glyf"]["lines"].as_array().unwrap().len()));
        assert!(app.models.table_viewer.text.text.is_some());

        // The search keeps the matching lines; stopping the comparison leaves the Differences view.
        viewer(&mut app, json!({"view": "ttx", "search": "glyph"})).unwrap();
        let text = &app.models.table_viewer.text;
        assert!(text.hits.as_ref().is_some_and(|h| !h.is_empty() && h.len() < text.text.as_ref().unwrap().len()), "{:?}", text.hits);
        let out = viewer(&mut app, json!({"view": "differences", "search": null})).unwrap();
        assert_eq!(out["view"], "differences");
        let out = viewer(&mut app, json!({"compareWith": null})).unwrap();
        assert_eq!(out["view"], "fields");
        assert_eq!(out["differing"], json!([]));
        let _ = std::fs::remove_dir_all(first.parent().unwrap());
    }
}

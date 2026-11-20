//! The Table Viewer (`tableViewer.show`): a compiled font's tables, the selected table as a tree of decoded fields with
//! a hex pane, its TTX text, and with a second font the tables that differ and the decoded differences.
//!
//! | Input | Does |
//! |---|---|
//! | the Font field (Enter), Reload | `tableViewer.show {font}`, `{reload: true}` |
//! | the Compare With field (Enter), Stop Comparing | `tableViewer.show {compareWith}`, `{compareWith: null}` |
//! | Open…, Compare With… | `tableViewer.openFont`, `tableViewer.compareWithFile`: the window asks for the file |
//! | a row of the table list | `tableViewer.show {table}` |
//! | Fields, TTX, Differences | `tableViewer.show {view}` |
//! | a record of the tree | `tableViewer.show {record}`: the hex pane marks its bytes |
//! | a record's disclosure | `tableViewer.show {record, expand}`: reads its children when they are not read yet |
//! | the search field | `tableViewer.show {search}` |

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Tracked,
};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::style::TypeRole;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::table_viewer::{TableItem, TableViewerModel, TextPane, TreeRow};
use crate::shell::table_viewer::MAX_HELD_NODES;

craft::ui::words! {
    /// The words of the Table Viewer.
    pub struct TableViewerWords {
        font: "fontcraftTableViewer.font" = "Font";
        font_file: "fontcraftTableViewer.fontFile" = "Font file";
        reload: "fontcraftTableViewer.reload" = "Reload";
        reload_tip: "fontcraftTableViewer.reloadTip" = "Read the font files again";
        /// Asks for the font file.
        open: "fontcraftTableViewer.open" = "Open…";
        /// Asks for the second font file.
        compare_pick: "fontcraftTableViewer.comparePick" = "Compare With…";
        compare_with: "fontcraftTableViewer.compareWith" = "Compare With";
        second_file: "fontcraftTableViewer.secondFile" = "Second font file";
        stop: "fontcraftTableViewer.stop" = "Stop Comparing";
        same_tables: "fontcraftTableViewer.sameTables" = "The fonts have the same tables.";
        differ: "fontcraftTableViewer.differ" = "Tables that differ:";
        tables: "fontcraftTableViewer.tables" = "Tables";
        tag: "fontcraftTableViewer.tag" = "Tag";
        size: "fontcraftTableViewer.size" = "Size";
        checksum: "fontcraftTableViewer.checksum" = "Checksum";
        status: "fontcraftTableViewer.status" = "Status";
        no_tables: "fontcraftTableViewer.noTables" = "Open a compiled font to list its tables.";
        raw: "fontcraftTableViewer.raw" = "not decoded";
        differs: "fontcraftTableViewer.differs" = "differs";
        only_a: "fontcraftTableViewer.onlyA" = "only in this font";
        only_b: "fontcraftTableViewer.onlyB" = "only in the compared font";
        same: "fontcraftTableViewer.same" = "same";
        view: "fontcraftTableViewer.view" = "View";
        fields: "fontcraftTableViewer.fields" = "Fields";
        ttx: "fontcraftTableViewer.ttx" = "TTX";
        differences: "fontcraftTableViewer.differences" = "Differences";
        search: "fontcraftTableViewer.search" = "Search names and values";
        select_table: "fontcraftTableViewer.selectTable" = "Select a table.";
        reading: "fontcraftTableViewer.reading" = "Reading…";
        records: "fontcraftTableViewer.records" = "Records";
        name: "fontcraftTableViewer.name" = "Name";
        kind: "fontcraftTableViewer.kind" = "Type";
        value: "fontcraftTableViewer.value" = "Value";
        more: "fontcraftTableViewer.more" = "more";
        no_records: "fontcraftTableViewer.noRecords" = "The table has no records.";
        no_match: "fontcraftTableViewer.noMatch" = "No record matches the search.";
        left_out: "fontcraftTableViewer.leftOut" = "Some records are left out; the viewer holds up to";
        bytes: "fontcraftTableViewer.bytes" = "Bytes";
        to: "fontcraftTableViewer.to" = "to";
        hex: "fontcraftTableViewer.hex" = "Hex";
        lines: "fontcraftTableViewer.lines" = "Lines";
        empty_text: "fontcraftTableViewer.emptyText" = "The text is empty.";
        no_line: "fontcraftTableViewer.noLine" = "No line matches the search.";
        text_cut: "fontcraftTableViewer.textCut" = "Part of the text is left out.";
    }
}

/// Rows the table list, the tree and the texts show before they scroll.
const LIST_ROWS: u16 = 10;
const TREE_ROWS: u16 = 16;
const HEX_ROWS: u16 = 9;

const TAG: ColumnKey = ColumnKey::new("tag");
const SIZE: ColumnKey = ColumnKey::new("size");
const CHECKSUM: ColumnKey = ColumnKey::new("checksum");
const STATUS: ColumnKey = ColumnKey::new("status");
const NAME: ColumnKey = ColumnKey::new("name");
const KIND: ColumnKey = ColumnKey::new("kind");
const VALUE: ColumnKey = ColumnKey::new("value");

pub struct Props<'a> {
    pub model: &'a Tracked<TableViewerModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn show_msg(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("tableViewer.show", params))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &TableViewerModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<TableViewerWords>();
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        files(ui, &words, model, sent);
        table_list(ui, &words, model, sent);
        if model.table.is_empty() {
            if !model.tables.is_empty() {
                Text::new(words.select_table.clone()).shade(Shade::Muted).show(ui);
            }
            return;
        }
        view_bar(ui, &words, model, sent);
        match model.view.as_str() {
            "ttx" | "differences" => text_pane(ui, &words, &model.text, words.lines.clone(), TREE_ROWS, None),
            _ => fields(ui, &words, model, sent),
        }
    });
}

/// The font and compared font fields.
fn files(ui: &mut Ui, words: &TableViewerWords, model: &TableViewerModel, sent: &mut Vec<Msg>) {
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let field = TextField::new(words.font.clone(), Bind::given(model.font.clone())).placeholder(words.font_file.clone());
        if let Some(path) = field.show(ui).events.committed().filter(|p| !p.is_empty() && **p != model.font) {
            sent.push(show_msg(json!({ "font": path })));
        }
        if Button::new(words.open.clone()).show(ui).clicked() {
            sent.push(Msg::Shell(CommandRef::new("tableViewer.openFont")));
        }
        if !model.font.is_empty() && Button::new(words.reload.clone()).tooltip(words.reload_tip.clone()).show(ui).clicked() {
            sent.push(show_msg(json!({ "reload": true })));
        }
    });
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let field = TextField::new(words.compare_with.clone(), Bind::given(model.compare_with.clone())).placeholder(words.second_file.clone());
        if let Some(path) = field.show(ui).events.committed().filter(|p| !p.is_empty() && **p != model.compare_with) {
            sent.push(show_msg(json!({ "compareWith": path })));
        }
        if Button::new(words.compare_pick.clone()).show(ui).clicked() {
            sent.push(Msg::Shell(CommandRef::new("tableViewer.compareWithFile")));
        }
        if !model.compare_with.is_empty() && Button::new(words.stop.clone()).show(ui).clicked() {
            sent.push(show_msg(json!({ "compareWith": null })));
        }
    });
    match model.identical {
        Some(true) => {
            Text::new(words.same_tables.clone()).shade(Shade::Muted).show(ui);
        }
        Some(false) => {
            Text::new(format!("{} {}", words.differ.as_str(), model.differing.join(", "))).show(ui);
        }
        None => {}
    }
}

/// What the Status column says about a table.
fn status_text(words: &TableViewerWords, t: &TableItem) -> String {
    let mut parts = Vec::new();
    match t.status.as_deref() {
        Some("different") => parts.push(words.differs.as_str().to_string()),
        Some("onlyA") => parts.push(words.only_a.as_str().to_string()),
        Some("onlyB") => parts.push(words.only_b.as_str().to_string()),
        Some("same") => parts.push(words.same.as_str().to_string()),
        _ => {}
    }
    if t.in_font && !t.decoded {
        parts.push(words.raw.as_str().to_string());
    }
    if let Some(e) = &t.error {
        parts.push(e.clone());
    }
    parts.join(", ")
}

/// Rows keyed by their position.
struct Positions(usize);

impl DisplayOrder<usize> for Positions {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Positions {}

fn table_list(ui: &mut Ui, words: &TableViewerWords, model: &TableViewerModel, sent: &mut Vec<Msg>) {
    let columns = vec![
        ColumnSpec::new(TAG, words.tag.clone(), ColumnWidth::Content),
        ColumnSpec::new(SIZE, words.size.clone(), ColumnWidth::Content).numeric(),
        ColumnSpec::new(CHECKSUM, words.checksum.clone(), ColumnWidth::Content),
        ColumnSpec::new(STATUS, words.status.clone(), ColumnWidth::fill(1.0)),
    ];
    let current = model.tables.iter().position(|t| t.tag == model.table);
    let before = Selection::new(current.into_iter().collect(), current, current);
    let mut after = before.clone();
    let tables = &model.tables;
    let _ = Table::<usize>::new(words.tables.clone(), columns)
        .empty(words.no_tables.clone())
        .max_rows(LIST_ROWS)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut after))
        .show_items(ui, &Positions(tables.len()), &mut |at, icx| {
            let Some(t) = tables.get(*at) else { return ListRow::new("") };
            let row = ListRow::new(t.tag.clone()).cells(icx.cells()).role(TypeRole::Mono).cell(STATUS, status_text(words, t));
            if t.in_font { row.cell(SIZE, t.length.to_string()).cell(CHECKSUM, t.checksum.clone()) } else { row }
        });
    if after.items != before.items
        && let Some(t) = after.lead.or_else(|| after.items.last().copied()).and_then(|at| tables.get(at))
        && t.tag != model.table
    {
        sent.push(show_msg(json!({ "table": t.tag })));
    }
}

fn view_bar(ui: &mut Ui, words: &TableViewerWords, model: &TableViewerModel, sent: &mut Vec<Msg>) {
    let mut views = vec![Entry::new("fields".to_string(), words.fields.clone()), Entry::new("ttx".to_string(), words.ttx.clone())];
    if !model.compare_with.is_empty() {
        views.push(Entry::new("differences".to_string(), words.differences.clone()));
    }
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let mut picked = model.view.clone();
        Segmented::new(words.view.clone(), Bind::bound(&mut picked), &views).show(ui);
        if picked != model.view {
            sent.push(show_msg(json!({ "view": picked })));
        }
        let mut term = model.search.clone();
        TextField::search(words.search.clone(), Bind::bound(&mut term)).placeholder(words.search.clone()).show(ui);
        if term != model.search {
            sent.push(show_msg(json!({ "search": term })));
        }
    });
}

/// The tree rows in view, keyed by node.
struct Tree<'a>(&'a [TreeRow]);

impl DisplayOrder<u32> for Tree<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> u32 {
        self.0.get(at).map_or(u32::MAX, |r| r.id)
    }

    fn index_of(&self, key: &u32) -> Option<usize> {
        self.0.iter().position(|r| r.id == *key)
    }

    fn depth(&self, at: usize) -> usize {
        self.0.get(at).map_or(0, |r| r.depth)
    }

    fn has_children(&self, at: usize) -> bool {
        self.0.get(at).is_some_and(|r| r.has_children)
    }

    fn open(&self, at: usize) -> bool {
        self.0.get(at).is_some_and(|r| r.open)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|r| Cow::Borrowed(r.name.as_str()))
    }
}

impl ItemSource<u32> for Tree<'_> {}

fn fields(ui: &mut Ui, words: &TableViewerWords, model: &TableViewerModel, sent: &mut Vec<Msg>) {
    if let Some(note) = &model.tree_note {
        Text::new(note.clone()).shade(Shade::Muted).show(ui);
    } else {
        record_tree(ui, words, model, sent);
    }
    let bytes = match model.bytes {
        Some((start, end)) => {
            format!(
                "{} {start} {} {end} ({} {})",
                words.bytes.as_str(),
                words.to.as_str(),
                end.saturating_sub(start),
                words.bytes.as_str().to_lowercase()
            )
        }
        None => words.bytes.as_str().to_string(),
    };
    Text::new(bytes).shade(Shade::Muted).show(ui);
    if model.hex.text.is_some() || model.hex.note.is_some() {
        text_pane(ui, words, &model.hex, words.hex.clone(), HEX_ROWS, model.marked);
    }
}

fn record_tree(ui: &mut Ui, words: &TableViewerWords, model: &TableViewerModel, sent: &mut Vec<Msg>) {
    let rows = &model.rows;
    let empty = if model.search.is_empty() { words.no_records.clone() } else { words.no_match.clone() };
    let columns = vec![
        ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::Content),
        ColumnSpec::new(KIND, words.kind.clone(), ColumnWidth::Content),
        ColumnSpec::new(VALUE, words.value.clone(), ColumnWidth::fill(1.0)),
    ];
    let before = Selection::new(model.selected.into_iter().collect(), model.selected, model.selected);
    let mut after = before.clone();
    let mut list = List::<u32>::new(words.records.clone())
        .tree(true)
        .header(false)
        .columns(columns)
        .empty(empty)
        .max_rows(TREE_ROWS)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut after));
    if let Some(id) = model.selected {
        list = list.reveal(id);
    }
    let out = list.show_items(ui, &Tree(rows), &mut |id, icx| {
        let Some(row) = rows.iter().find(|r| r.id == *id) else { return ListRow::new("") };
        let label = if row.more > 0 { format!("{}  ({} {})", row.name, row.more, words.more.as_str()) } else { row.name.clone() };
        let mut item = ListRow::new(label).cells(icx.cells()).state(icx.row.clone()).depth(row.depth).cell(KIND, row.kind.clone());
        if row.has_children {
            item = item.group(row.open);
        }
        if let Some(v) = &row.value {
            item = item.cell(VALUE, v.clone());
        }
        item
    });
    let record = |id: u32| rows.iter().find(|r| r.id == id).map(|r| r.record.clone());
    if after.items != before.items
        && let Some(id) = after.lead.or_else(|| after.items.last().copied())
        && Some(id) != model.selected
        && let Some(record) = record(id)
    {
        sent.push(show_msg(json!({ "record": record })));
    }
    for event in &out.events {
        if let CollectionEvent::Open(change) = event
            && let Some(record) = record(change.key)
        {
            sent.push(show_msg(json!({ "record": record, "expand": change.open })));
        }
    }
    if model.rows_cut {
        Text::new(format!("{} {MAX_HELD_NODES}.", words.left_out.as_str())).shade(Shade::Muted).show(ui);
    }
}

/// The lines of a text pane, keyed by line index.
struct Lines<'a> {
    hits: Option<&'a [u32]>,
    len: usize,
}

impl Lines<'_> {
    fn line(&self, at: usize) -> usize {
        match self.hits {
            Some(h) => h.get(at).map_or(usize::MAX, |i| *i as usize),
            None => at,
        }
    }
}

impl DisplayOrder<usize> for Lines<'_> {
    fn len(&self) -> usize {
        self.hits.map_or(self.len, <[u32]>::len)
    }

    fn key(&self, at: usize) -> usize {
        self.line(at)
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        match self.hits {
            Some(h) => h.iter().position(|i| *i as usize == *key),
            None => (*key < self.len).then_some(*key),
        }
    }
}

impl ItemSource<usize> for Lines<'_> {}

/// A text, its matching lines only while searching; `marked` lines show as selected (the hex pane's record).
fn text_pane(
    ui: &mut Ui,
    words: &TableViewerWords,
    pane: &TextPane,
    label: craft::ui::runtime::env::Label,
    rows: u16,
    marked: Option<(usize, usize)>,
) {
    let Some(text) = &pane.text else {
        let note = pane.note.clone().unwrap_or_else(|| words.reading.as_str().to_string());
        Text::new(note).shade(Shade::Muted).show(ui);
        return;
    };
    let source = Lines { hits: pane.hits.as_deref(), len: text.len() };
    let empty = if pane.hits.is_some() { words.no_line.clone() } else { words.empty_text.clone() };
    let digits = text.len().max(1).ilog10() as usize + 1;
    let marked_lines: Vec<usize> = marked.map(|(a, b)| (a..=b.min(a.saturating_add(4096))).collect()).unwrap_or_default();
    let selection = Selection::new(marked_lines.clone(), marked_lines.first().copied(), marked_lines.first().copied());
    let mut list = List::<usize>::new(label).empty(empty).max_rows(rows).selection(Bind::given(selection));
    if let Some(first) = marked_lines.first() {
        list = list.reveal(*first);
    }
    let _ = list.show_items(ui, &source, &mut |line, _| {
        ListRow::new(format!("{:>digits$}  {}", line.saturating_add(1), crate::shell::table_viewer::shown(text.line(*line)))).role(TypeRole::Mono)
    });
    if text.cut {
        Text::new(words.text_cut.clone()).shade(Shade::Muted).show(ui);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::table_viewer::{TableItem, TableViewerModel, TreeRow};

    fn item(tag: &str) -> TableItem {
        TableItem { tag: tag.into(), length: 54, checksum: "0x00000000".into(), decoded: true, in_font: true, ..Default::default() }
    }

    #[test]
    fn picking_a_table_and_a_record_posts_table_viewer_show() {
        let model = Tracked::new(TableViewerModel {
            generation: 1,
            font: "a.ttf".into(),
            table: "head".into(),
            view: "fields".into(),
            tables: vec![item("head"), item("hmtx")],
            rows: vec![
                TreeRow {
                    id: 1,
                    record: vec![0],
                    depth: 0,
                    name: "majorVersion".into(),
                    kind: "uint16".into(),
                    value: Some("1".into()),
                    ..Default::default()
                },
                TreeRow {
                    id: 2,
                    record: vec![1],
                    depth: 0,
                    name: "unitsPerEm".into(),
                    kind: "uint16".into(),
                    value: Some("1000".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let hmtx = fixture.by_label("hmtx").expect("the hmtx row");
        fixture.click(&hmtx).settle();
        let units = fixture.by_label("unitsPerEm").expect("the unitsPerEm record");
        fixture.click(&units).settle();
        let shell = |p| Msg::Shell(CommandRef::with_params("tableViewer.show", p));
        assert_eq!(*posted.borrow(), vec![shell(json!({"table": "hmtx"})), shell(json!({"record": [1]}))]);

        posted.borrow_mut().clear();
        let open = fixture.by_label("Open…").expect("the Open… button");
        fixture.click(&open).settle();
        let compare = fixture.by_label("Compare With…").expect("the Compare With… button");
        fixture.click(&compare).settle();
        assert_eq!(
            *posted.borrow(),
            vec![Msg::Shell(CommandRef::new("tableViewer.openFont")), Msg::Shell(CommandRef::new("tableViewer.compareWithFile"))]
        );
    }
}

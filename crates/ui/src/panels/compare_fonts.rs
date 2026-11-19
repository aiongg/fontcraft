//! Compare Fonts (`edit.compareFonts`): two masters side by side, the differences grouped under category headings,
//! and Use Left and Use Right on the selected row.
//!
//! | Input | Does |
//! |---|---|
//! | a document or master picker | `edit.compareFonts` with that side changed |
//! | a heading's disclosure | `compareFonts.setView {collapsed}` |
//! | a row | `compareFonts.setView {selected}` |
//! | Use Left, Use Right | `compareFonts.useLeft`, `compareFonts.useRight` on the selected row |

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Tracked,
};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use egui::Ui;
use serde_json::{Map, Value, json};

use crate::app::Msg;
use crate::app::models::compare::{CompareFontsModel, DiffLine, DiffRow};
use crate::shell::compare_fonts::{CATEGORIES, side_value};

craft::ui::words! {
    /// The words of the Compare Fonts panel.
    pub struct CompareFontsWords {
        left: "fontcraftCompareFonts.left" = "Left";
        right: "fontcraftCompareFonts.right" = "Right";
        document: "fontcraftCompareFonts.document" = "Document";
        master: "fontcraftCompareFonts.master" = "Master";
        differences: "fontcraftCompareFonts.differences" = "Differences";
        name: "fontcraftCompareFonts.name" = "Name";
        font_info: "fontcraftCompareFonts.fontInfo" = "Font Info";
        master_metrics: "fontcraftCompareFonts.masterMetrics" = "Master Metrics";
        glyph: "fontcraftCompareFonts.glyph" = "Glyph Outlines";
        kerning: "fontcraftCompareFonts.kerning" = "Kerning Pairs";
        no_value: "fontcraftCompareFonts.noValue" = "–";
        same: "fontcraftCompareFonts.same" = "The two masters do not differ.";
        only_first: "fontcraftCompareFonts.onlyFirst" = "Only the first differences are listed:";
        use_left: "fontcraftCompareFonts.useLeft" = "Use Left";
        use_right: "fontcraftCompareFonts.useRight" = "Use Right";
        no_document: "fontcraftCompareFonts.noDocument" = "Open a document to compare its masters.";
    }
}

/// Lines the list shows before it scrolls.
const LIST_ROWS: u16 = 20;

const NAME: ColumnKey = ColumnKey::new("name");
const LEFT: ColumnKey = ColumnKey::new("left");
const RIGHT: ColumnKey = ColumnKey::new("right");

pub struct Props<'a> {
    pub model: &'a Tracked<CompareFontsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &CompareFontsModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<CompareFontsWords>();
    if model.documents.is_empty() {
        Text::new(words.no_document.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        side_pickers(ui, &words, model, "left", model.left.as_ref(), sent);
        side_pickers(ui, &words, model, "right", model.right.as_ref(), sent);
        match &model.error {
            Some(error) => {
                Text::new(error.clone()).show(ui);
            }
            None => diff_list(ui, &words, model, sent),
        }
        footer(ui, &words, model, sent);
    });
}

/// The document and master pickers of one side (`name`: left or right).
fn side_pickers(ui: &mut Ui, words: &CompareFontsWords, model: &CompareFontsModel, name: &str, side: Option<&(usize, String)>, sent: &mut Vec<Msg>) {
    let docs: Vec<Entry<String>> = model.documents.iter().enumerate().map(|(i, d)| Entry::new(i.to_string(), d.title.clone())).collect();
    let current = side.map(|(i, _)| *i);
    let masters: Vec<Entry<String>> = current
        .and_then(|i| model.documents.get(i))
        .map(|d| d.masters.iter().map(|(id, label)| Entry::new(id.clone(), label.clone())).collect())
        .unwrap_or_default();
    let master =
        side.map(|(_, m)| m.clone()).filter(|m| !m.is_empty()).or_else(|| masters.first().and_then(|e| e.value().cloned())).unwrap_or_default();
    let title = if name == "left" { words.left.clone() } else { words.right.clone() };
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        Text::new(title.clone()).show(ui);
        let doc = current.map(|i| i.to_string()).unwrap_or_default();
        let label = format!("{} {}", title.as_str(), words.document.as_str());
        if let Some(picked) = Select::new(label, Bind::given(doc.clone()), &docs).show(ui).events.committed()
            && *picked != doc
            && let Ok(i) = picked.parse::<usize>()
        {
            let mut p = Map::new();
            p.insert(name.into(), json!({ "document": i }));
            sent.push(shell("edit.compareFonts", Value::Object(p)));
        }
        let label = format!("{} {}", title.as_str(), words.master.as_str());
        if let Some(picked) = Select::new(label, Bind::given(master.clone()), &masters).show(ui).events.committed()
            && *picked != master
            && let Some(i) = current
        {
            let mut p = Map::new();
            p.insert(name.into(), json!({ "document": i, "master": picked }));
            sent.push(shell("edit.compareFonts", Value::Object(p)));
        }
    });
}

/// A line's key: a row's position, or for a heading a key past every row.
fn line_key(line: DiffLine) -> usize {
    match line {
        DiffLine::Heading(c, _) => usize::MAX - c,
        DiffLine::Row(i) => i,
    }
}

struct Lines<'a>(&'a [DiffLine]);

impl DisplayOrder<usize> for Lines<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        self.0.get(at).map_or(usize::MAX - CATEGORIES.len(), |l| line_key(*l))
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        self.0.iter().position(|l| line_key(*l) == *key)
    }

    fn depth(&self, at: usize) -> usize {
        usize::from(matches!(self.0.get(at), Some(DiffLine::Row(_))))
    }

    fn has_children(&self, at: usize) -> bool {
        matches!(self.0.get(at), Some(DiffLine::Heading(_, n)) if *n > 0)
    }

    fn open(&self, at: usize) -> bool {
        // A heading is open unless the line after it is another heading or the end.
        matches!(self.0.get(at.saturating_add(1)), Some(DiffLine::Row(_)))
    }
}

impl ItemSource<usize> for Lines<'_> {}

fn category_title(words: &CompareFontsWords, c: usize) -> String {
    let w = match c {
        0 => &words.font_info,
        1 => &words.master_metrics,
        2 => &words.glyph,
        _ => &words.kerning,
    };
    w.as_str().to_string()
}

fn diff_list(ui: &mut Ui, words: &CompareFontsWords, model: &CompareFontsModel, sent: &mut Vec<Msg>) {
    let rows: &[DiffRow] = &model.rows;
    let columns = vec![
        ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::fill(1.0)),
        ColumnSpec::new(LEFT, words.left.clone(), ColumnWidth::fill(1.0)),
        ColumnSpec::new(RIGHT, words.right.clone(), ColumnWidth::fill(1.0)),
    ];
    let before = Selection::new(model.selected.into_iter().collect(), model.selected, model.selected);
    let mut after = before.clone();
    let out = List::<usize>::new(words.differences.clone())
        .tree(true)
        .columns(columns)
        .max_rows(LIST_ROWS)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut after))
        .show_items(ui, &Lines(&model.lines), &mut |key, icx| {
            if let Some(c) = usize::MAX.checked_sub(*key).filter(|c| *c < CATEGORIES.len()) {
                let count = model.lines.iter().find_map(|l| match l {
                    DiffLine::Heading(h, n) if *h == c => Some(*n),
                    _ => None,
                });
                let open = !CATEGORIES.get(c).is_some_and(|id| model.collapsed.iter().any(|x| x == id));
                let mut item = ListRow::new(format!("{} ({})", category_title(words, c), count.unwrap_or(0))).depth(0).ink(RowInk::Muted);
                if count.unwrap_or(0) > 0 {
                    item = item.group(open);
                }
                return item;
            }
            let Some(row) = rows.get(*key) else { return ListRow::new("") };
            let value = |v: &Option<String>| v.clone().unwrap_or_else(|| words.no_value.as_str().to_string());
            ListRow::new(row.label.clone())
                .cells(icx.cells())
                .state(icx.row.clone())
                .depth(1)
                .cell(LEFT, value(&row.left))
                .cell(RIGHT, value(&row.right))
        });
    if after.items != before.items
        && let Some(row) = after.lead.or_else(|| after.items.last().copied()).and_then(|i| rows.get(i))
    {
        sent.push(shell("compareFonts.setView", json!({ "selected": { "category": row.category, "key": row.key } })));
    }
    for event in &out.events {
        if let CollectionEvent::Open(change) = event
            && let Some(id) = usize::MAX.checked_sub(change.key).and_then(|c| CATEGORIES.get(c))
        {
            let mut collapsed: Vec<String> = model.collapsed.iter().filter(|c| *c != id).cloned().collect();
            if !change.open {
                collapsed.push((*id).to_string());
            }
            sent.push(shell("compareFonts.setView", json!({ "collapsed": collapsed })));
        }
    }
    if rows.is_empty() {
        Text::new(words.same.clone()).shade(Shade::Muted).show(ui);
    } else if model.truncated {
        Text::new(format!("{} {}", words.only_first.as_str(), rows.len())).shade(Shade::Muted).show(ui);
    }
}

/// The command Use Left (`from_left`) or Use Right runs on the selected row; `None` while no listed row is selected.
pub fn use_command(model: &CompareFontsModel, from_left: bool) -> Option<Msg> {
    let row = model.rows.get(model.selected?)?;
    let ((l, lm), (r, rm)) = (model.left.as_ref()?, model.right.as_ref()?);
    let mut target = Map::new();
    target.insert("category".into(), json!(row.category));
    target.insert("key".into(), json!(row.key));
    if let (Some(first), Some(second)) = (&row.first, &row.second) {
        target.insert("first".into(), json!(first));
        target.insert("second".into(), json!(second));
    }
    let params = json!({ "left": side_value(*l, lm), "right": side_value(*r, rm), "row": Value::Object(target) });
    Some(Msg::run(if from_left { "compareFonts.useLeft" } else { "compareFonts.useRight" }, params))
}

fn footer(ui: &mut Ui, words: &CompareFontsWords, model: &CompareFontsModel, sent: &mut Vec<Msg>) {
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        for (label, from_left) in [(&words.use_left, true), (&words.use_right, false)] {
            let command = use_command(model, from_left);
            if Button::new(label.clone()).enabled(command.is_some()).show(ui).clicked()
                && let Some(msg) = command
            {
                sent.push(msg);
            }
        }
    });
}

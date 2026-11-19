//! Compare Family (`edit.compareFamily`): one glyph attribute across the open documents, one column per document and
//! one row per glyph.
//!
//! | Input | Does |
//! |---|---|
//! | the attribute picker | `compareFamily.setAttribute` |
//! | the filter picker | `compareFamily.setFilter` |
//! | Keep on Top | `compareFamily.togglePin` |
//! | Update | `compareFamily.update` |
//! | Enter or a double click on a row | `compareFamily.openGlyph` in the first document that has the glyph |
//! | a secondary click on a cell | `compareFamily.openGlyph` in that cell's document |

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource};
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ListRow;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::compare::{CompareFamilyModel, FamilyRow};
use crate::shell::compare_family::{ATTRIBUTES, FILTERS};

craft::ui::words! {
    /// The words of the Compare Family panel.
    pub struct CompareFamilyWords {
        attribute: "fontcraftCompareFamily.attribute" = "Attribute";
        component_names: "fontcraftCompareFamily.componentNames" = "Component Names";
        anchors: "fontcraftCompareFamily.anchors" = "Anchors";
        left_group: "fontcraftCompareFamily.leftKerningGroup" = "Left Kerning Group";
        right_group: "fontcraftCompareFamily.rightKerningGroup" = "Right Kerning Group";
        glyphs: "fontcraftCompareFamily.glyphs" = "Glyphs";
        filter: "fontcraftCompareFamily.filter" = "Filter";
        all: "fontcraftCompareFamily.all" = "All";
        missing_in_one: "fontcraftCompareFamily.missingInOne" = "Missing in One";
        ignore_missing: "fontcraftCompareFamily.ignoreIfMissingInOne" = "Ignore If Missing in One";
        keep_on_top: "fontcraftCompareFamily.keepOnTop" = "Keep on Top";
        update: "fontcraftCompareFamily.update" = "Update";
        update_tip: "fontcraftCompareFamily.updateTip" = "Read the glyphs of the open documents again";
        table: "fontcraftCompareFamily.table" = "Glyphs across the family";
        glyph: "fontcraftCompareFamily.glyph" = "Glyph";
        missing: "fontcraftCompareFamily.missing" = "(missing glyph)";
        no_value: "fontcraftCompareFamily.noValue" = "--";
        no_match: "fontcraftCompareFamily.noMatch" = "No glyph matches the filter.";
        not_read: "fontcraftCompareFamily.notRead" = "Press Update to read the open documents.";
        of: "fontcraftCompareFamily.of" = "of";
        listed: "fontcraftCompareFamily.listed" = "glyphs are listed.";
    }
}

/// Rows the table shows before it scrolls.
const LIST_ROWS: u16 = 20;
const GLYPH: ColumnKey = ColumnKey::new("glyph");

pub struct Props<'a> {
    pub model: &'a Tracked<CompareFamilyModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &CompareFamilyModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<CompareFamilyWords>();
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        toolbar(ui, &words, model, sent);
        if let Some(error) = &model.error {
            Text::new(error.clone()).show(ui);
        } else if model.attribute.is_empty() {
            Text::new(words.not_read.clone()).shade(Shade::Muted).show(ui);
        } else {
            table(ui, &words, model, sent);
        }
    });
}

fn toolbar(ui: &mut Ui, words: &CompareFamilyWords, model: &CompareFamilyModel, sent: &mut Vec<Msg>) {
    let attribute_labels = [&words.component_names, &words.anchors, &words.left_group, &words.right_group, &words.glyphs];
    let attributes: Vec<Entry<String>> = ATTRIBUTES.iter().zip(attribute_labels).map(|(id, l)| Entry::new(id.to_string(), l.clone())).collect();
    let filter_labels = [&words.all, &words.missing_in_one, &words.ignore_missing];
    let filters: Vec<Entry<String>> = FILTERS.iter().zip(filter_labels).map(|(id, l)| Entry::new(id.to_string(), l.clone())).collect();
    let attribute = if model.attribute.is_empty() { ATTRIBUTES[0].to_string() } else { model.attribute.clone() };
    let filter = if model.filter.is_empty() { FILTERS[0].to_string() } else { model.filter.clone() };
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if let Some(picked) = Select::new(words.attribute.clone(), Bind::given(attribute.clone()), &attributes).show(ui).events.committed()
            && *picked != attribute
        {
            sent.push(shell("compareFamily.setAttribute", json!({ "attribute": picked })));
        }
        if let Some(picked) = Select::new(words.filter.clone(), Bind::given(filter.clone()), &filters).show(ui).events.committed()
            && *picked != filter
        {
            sent.push(shell("compareFamily.setFilter", json!({ "filter": picked })));
        }
        if let Some(on) = Checkbox::new(Bind::given(model.pinned)).label(words.keep_on_top.clone()).show(ui).events.committed() {
            sent.push(shell("compareFamily.togglePin", json!({ "on": on })));
        }
        if Button::new(words.update.clone()).tooltip(words.update_tip.clone()).show(ui).clicked() {
            sent.push(shell("compareFamily.update", json!({})));
        }
    });
}

struct Rows<'a>(&'a [FamilyRow]);

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|r| std::borrow::Cow::Borrowed(r.glyph.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {}

/// `compareFamily.openGlyph` for `row` in column `column`, unless the glyph is missing there.
fn open_glyph(model: &CompareFamilyModel, row: usize, column: usize) -> Option<Msg> {
    let row = model.rows.get(row)?;
    if row.cells.get(column).is_none_or(|c| c.missing) {
        return None;
    }
    let document = model.columns.get(column)?.document;
    Some(shell("compareFamily.openGlyph", json!({ "document": document, "glyph": row.glyph })))
}

fn table(ui: &mut Ui, words: &CompareFamilyWords, model: &CompareFamilyModel, sent: &mut Vec<Msg>) {
    let keys: Vec<ColumnKey> = (0..model.columns.len()).filter_map(|i| u32::try_from(i).ok()).map(ColumnKey::index).collect();
    let mut columns = vec![ColumnSpec::new(GLYPH, words.glyph.clone(), ColumnWidth::Content)];
    columns.extend(model.columns.iter().zip(&keys).map(|(c, k)| ColumnSpec::new(k.clone(), c.title.clone(), ColumnWidth::fill(1.0))));
    let rows = &model.rows;
    let out = Table::<usize>::new(words.table.clone(), columns).empty(words.no_match.clone()).max_rows(LIST_ROWS).striped(true).show_items(
        ui,
        &Rows(rows),
        &mut |at, icx| {
            let Some(row) = rows.get(*at) else { return ListRow::new("") };
            // Rows whose values differ read strong.
            let mut item = ListRow::new(row.glyph.clone()).cells(icx.cells()).striped(icx.row.striped).strong(!row.same);
            for (cell, key) in row.cells.iter().zip(&keys) {
                let text = if cell.missing {
                    words.missing.as_str().to_string()
                } else if cell.empty {
                    words.no_value.as_str().to_string()
                } else {
                    cell.text.clone()
                };
                item = item.cell(key.clone(), text);
                if cell.missing || cell.empty {
                    item = item.cell_muted(key.clone());
                }
            }
            item
        },
    );
    for event in &out.events {
        let msg = match event {
            CollectionEvent::Activate(at) => (0..model.columns.len()).find_map(|c| open_glyph(model, *at, c)),
            CollectionEvent::ContextMenu(request) => {
                request.column.as_ref().and_then(ColumnKey::position).and_then(|c| open_glyph(model, request.key, c as usize))
            }
            _ => None,
        };
        sent.extend(msg);
    }
    if model.total > rows.len() {
        Text::new(format!("{} {} {} {}", rows.len(), words.of.as_str(), model.total, words.listed.as_str())).shade(Shade::Muted).show(ui);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::compare::{CompareFamilyModel, FamilyCell, FamilyColumn, FamilyRow};

    #[test]
    fn update_and_activating_a_row_post_compare_family_commands() {
        let cell = |t: &str| FamilyCell { text: t.into(), ..Default::default() };
        let model = Tracked::new(CompareFamilyModel {
            attribute: "componentNames".into(),
            filter: "all".into(),
            columns: vec![
                FamilyColumn { document: 0, title: "Regular".into(), master: "m01".into() },
                FamilyColumn { document: 1, title: "Bold".into(), master: "m01".into() },
            ],
            total: 1,
            rows: vec![FamilyRow {
                glyph: "Aacute".into(),
                cells: vec![FamilyCell { missing: true, ..Default::default() }, cell("A, acutecomb")],
                same: false,
            }],
            read: Some(1),
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
        let update = fixture.by_label("Update").expect("the Update button");
        fixture.click(&update).settle();
        let row = fixture.by_label("Aacute").expect("the Aacute row");
        fixture.click(&row).key(Key::Enter, Modifiers::NONE).settle();
        let shell = |id: &str, p| Msg::Shell(CommandRef::with_params(id, p));
        assert_eq!(
            *posted.borrow(),
            vec![shell("compareFamily.update", json!({})), shell("compareFamily.openGlyph", json!({"document": 1, "glyph": "Aacute"}))]
        );
    }
}

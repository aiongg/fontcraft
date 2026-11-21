//! Compare Fonts in List (`font.compareInTable`): one property across the compared fonts, one row per glyph and one
//! column per font. Rows whose values differ read strong; Only Differences hides the others.
//!
//! | Input | Posts |
//! |---|---|
//! | the property picker | `font.compareInTable {fonts, property}` |
//! | Only Differences | `font.compareInTable {fonts, onlyDifferences}` |

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ListRow;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::table::Table;
use egui::Ui;
use fontcraft_engine::cmd::encodings::COMPARE_PROPERTIES;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::compare_list::{CompareListModel, CompareListRow};

craft::ui::words! {
    /// The words of the Compare Fonts in List panel.
    pub struct CompareInListWords {
        property: "fontcraftCompareInList.property" = "Property";
        width: "fontcraftCompareInList.width" = "Width";
        lsb: "fontcraftCompareInList.lsb" = "Left Sidebearing";
        rsb: "fontcraftCompareInList.rsb" = "Right Sidebearing";
        nodes: "fontcraftCompareInList.nodes" = "Nodes";
        paths: "fontcraftCompareInList.paths" = "Paths";
        components: "fontcraftCompareInList.components" = "Components";
        anchors: "fontcraftCompareInList.anchors" = "Anchors";
        unicodes: "fontcraftCompareInList.unicodes" = "Unicode";
        only_differences: "fontcraftCompareInList.onlyDifferences" = "Only Differences";
        table: "fontcraftCompareInList.table" = "Glyphs across the fonts";
        glyph: "fontcraftCompareInList.glyph" = "Glyph";
        no_value: "fontcraftCompareInList.noValue" = "–";
        no_rows: "fontcraftCompareInList.noRows" = "No glyph to list.";
        closed: "fontcraftCompareInList.closed" = "The compared fonts are closed.";
        of: "fontcraftCompareInList.of" = "of";
        listed: "fontcraftCompareInList.listed" = "glyphs are listed.";
    }
}

/// Rows the table shows before it scrolls.
const LIST_ROWS: u16 = 20;
const GLYPH: ColumnKey = ColumnKey::new("glyph");

pub struct Props<'a> {
    pub model: &'a Tracked<CompareListModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &CompareListModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<CompareInListWords>();
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        toolbar(ui, &words, model, sent);
        if let Some(error) = &model.error {
            Text::new(error.clone()).show(ui);
        } else if model.fonts.is_empty() {
            Text::new(words.closed.clone()).shade(Shade::Muted).show(ui);
        } else {
            table(ui, &words, model);
        }
    });
}

/// `font.compareInTable` on the compared fonts with `extra` added.
fn rerun(model: &CompareListModel, extra: (&str, Value)) -> Msg {
    let mut params = json!({ "fonts": model.positions });
    if let Some(map) = params.as_object_mut() {
        map.insert(extra.0.to_string(), extra.1);
    }
    Msg::Shell(CommandRef::with_params("font.compareInTable", params))
}

fn property_label(words: &CompareInListWords, id: &str) -> Option<Label> {
    let label = match id {
        "width" => &words.width,
        "lsb" => &words.lsb,
        "rsb" => &words.rsb,
        "nodes" => &words.nodes,
        "paths" => &words.paths,
        "components" => &words.components,
        "anchors" => &words.anchors,
        "unicodes" => &words.unicodes,
        _ => return None,
    };
    Some(label.clone())
}

fn toolbar(ui: &mut Ui, words: &CompareInListWords, model: &CompareListModel, sent: &mut Vec<Msg>) {
    let properties: Vec<Entry<String>> =
        COMPARE_PROPERTIES.iter().filter_map(|(id, _)| property_label(words, id).map(|l| Entry::new(id.to_string(), l))).collect();
    let property = if model.property.is_empty() { "width".to_string() } else { model.property.clone() };
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if let Some(picked) = Select::new(words.property.clone(), Bind::given(property.clone()), &properties).show(ui).events.committed()
            && *picked != property
        {
            sent.push(rerun(model, ("property", json!(picked))));
        }
        if let Some(on) = Checkbox::new(Bind::given(model.only_differences)).label(words.only_differences.clone()).show(ui).events.committed() {
            sent.push(rerun(model, ("onlyDifferences", json!(on))));
        }
    });
}

struct Rows<'a>(&'a [CompareListRow]);

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

fn table(ui: &mut Ui, words: &CompareInListWords, model: &CompareListModel) {
    let keys: Vec<ColumnKey> = (0..model.fonts.len()).filter_map(|i| u32::try_from(i).ok()).map(ColumnKey::index).collect();
    let mut columns = vec![ColumnSpec::new(GLYPH, words.glyph.clone(), ColumnWidth::Content)];
    columns.extend(model.fonts.iter().zip(&keys).map(|(title, k)| ColumnSpec::new(k.clone(), title.clone(), ColumnWidth::fill(1.0))));
    let rows = &model.rows;
    let _ = Table::<usize>::new(words.table.clone(), columns).empty(words.no_rows.clone()).max_rows(LIST_ROWS).striped(true).show_items(
        ui,
        &Rows(rows),
        &mut |at, icx| {
            let Some(row) = rows.get(*at) else { return ListRow::new("") };
            // Rows whose values differ read strong.
            let mut item = ListRow::new(row.glyph.clone()).cells(icx.cells()).striped(icx.row.striped).strong(row.differ);
            for (value, key) in row.values.iter().zip(&keys) {
                match value {
                    Some(text) => item = item.cell(key.clone(), text.clone()),
                    None => item = item.cell(key.clone(), words.no_value.clone()).cell_muted(key.clone()),
                }
            }
            item
        },
    );
    if model.total > rows.len() {
        Text::new(format!("{} {} {} {}", rows.len(), words.of.as_str(), model.total, words.listed.as_str())).shade(Shade::Muted).show(ui);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::contract::Tracked;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use fontcraft_engine::Session;
    use fontcraft_engine::model::Font;
    use serde_json::{Value, json};

    use super::{Props, show};
    use crate::app::models::compare_list::{CompareListModel, CompareListRow};
    use crate::app::{APP, FontCraftApp, Msg};

    fn shell(id: &str, params: Value) -> Msg {
        Msg::Shell(CommandRef::with_params(id, params))
    }

    #[test]
    fn compare_in_list_rows_show_and_only_differences_posts_the_command() {
        let row = |g: &str, a: &str, b: Option<&str>, differ| CompareListRow {
            glyph: g.into(),
            values: vec![Some(a.into()), b.map(str::to_string)],
            differ,
        };
        let model = Tracked::new(CompareListModel {
            fonts: vec!["Regular".into(), "Bold".into()],
            positions: vec![0, 1],
            property: "width".into(),
            rows: vec![row("A", "600", Some("640"), true), row("B", "500", None, true)],
            total: 2,
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
        assert!(fixture.by_label("A").is_some(), "the A row");
        assert!(fixture.by_label("Bold").is_some(), "a column per font");
        let only = fixture.by_label("Only Differences").expect("the Only Differences box");
        fixture.click(&only).settle();
        assert_eq!(*posted.borrow(), vec![shell("font.compareInTable", json!({"fonts": [0, 1], "onlyDifferences": true}))]);
    }

    /// The command shows the panel with a width column per font; another property, Only Differences and a refused
    /// property each do what they say, and the table follows an edit of a compared font.
    #[test]
    fn compare_in_list_builds_the_table_of_the_compared_fonts() {
        let mut session = Session::with_document();
        let mut other = Font::new("Other");
        other.glyph_mut("A").unwrap().layers[0].width = 640.0;
        session.add_document(other, None).unwrap();
        session.execute("document.activate", &json!({"index": 0})).unwrap();
        let mut app = FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()));

        app.run_now(shell("font.compareInTable", json!({"fonts": [0, 1]})), false).unwrap();
        assert!(crate::shell::compare_in_list::shows(&app));
        let model = &app.models.compare_list;
        assert_eq!(model.property, "width");
        assert_eq!(model.fonts.len(), 2);
        let a = model.rows.iter().find(|r| r.glyph == "A").unwrap();
        assert_eq!(a.values, vec![Some("600".to_string()), Some("640".to_string())]);
        assert!(a.differ);

        app.run_now(shell("font.compareInTable", json!({"onlyDifferences": true})), false).unwrap();
        assert!(app.models.compare_list.rows.iter().all(|r| r.differ), "only the rows that differ");
        assert!(app.models.compare_list.rows.iter().any(|r| r.glyph == "A"));

        app.run_now(shell("font.compareInTable", json!({"property": "nodes"})), false).unwrap();
        assert_eq!(app.models.compare_list.property, "nodes");
        assert!(app.models.compare_list.only_differences, "Only Differences is kept");
        assert!(app.run_now(shell("font.compareInTable", json!({"property": "colour"})), false).is_err());
        assert_eq!(app.view.compare_in_list.property, "nodes", "a refused command changes nothing");

        // Editing the first font builds the table again.
        app.run_now(shell("font.compareInTable", json!({"property": "width", "onlyDifferences": false})), false).unwrap();
        app.run_now(Msg::run("spacing.setWidth", json!({"glyphs": ["A"], "value": 640})), false).unwrap();
        let a = app.models.compare_list.rows.iter().find(|r| r.glyph == "A").unwrap();
        assert_eq!(a.values, vec![Some("640".to_string()), Some("640".to_string())]);
        assert!(!a.differ);
    }
}

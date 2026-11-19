//! The sidebar as the user drives it: what a click, a Cmd-click, a disclosure and the actions menu post.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::Modifiers;
use serde_json::{Value, json};

use super::{Props, show};
use crate::app::Msg;
use crate::app::models::glyph_props::GlyphPropsModel;
use crate::app::models::sidebar::{SidebarModel, SidebarRow};

fn row(path: &str, kind: &str, depth: usize, count: usize) -> SidebarRow {
    let path: Vec<String> = path.split('/').map(str::to_string).collect();
    let label = path.last().cloned().unwrap_or_default();
    let has_children = kind == "section";
    SidebarRow { path, label, kind: kind.into(), count, depth, has_children, open: has_children, ..SidebarRow::default() }
}

/// All, Categories with Letter and Number, Filters with one smart filter; `selected` are paths.
fn model(selected: &[&str]) -> SidebarModel {
    let mut lacking = row("languages/latin", "script", 1, 20);
    (lacking.present, lacking.expected) = (Some(20), Some(26));
    let rows = vec![
        row("all", "all", 0, 30),
        row("categories", "section", 0, 0),
        row("categories/Letter", "category", 1, 26),
        row("categories/Number", "category", 1, 4),
        row("languages", "section", 0, 0),
        lacking,
        row("filters", "section", 0, 0),
        row("filters/Mine", "smart", 1, 3),
    ];
    SidebarModel {
        has_font: true,
        visible: (0..rows.len()).collect(),
        rows,
        selected: selected.iter().map(|p| p.split('/').map(str::to_string).collect()).collect(),
        error: None,
    }
}

fn open(model: SidebarModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let model = Tracked::new(model);
    let inspector = Tracked::new(GlyphPropsModel { has_font: true, ..GlyphPropsModel::default() });
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, inspector: &inspector, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    (fixture, posted)
}

fn select(entries: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("fontView.selectSidebarEntries", json!({ "entries": entries, "mode": "replace" })))
}

/// A click selects one entry, a Cmd-click adds one, and a click on All selects every glyph again.
#[test]
fn clicks_select_sidebar_entries() {
    let (mut fixture, posted) = open(model(&[]));
    let letter = fixture.by_label("Letter").expect("the Letter entry");
    fixture.click(&letter).settle();
    assert_eq!(*posted.borrow(), vec![select(json!(["categories/Letter"]))]);

    let (mut fixture, posted) = open(model(&["categories/Letter"]));
    let mine = fixture.by_label("Mine").expect("the Mine filter");
    fixture.hold(Modifiers::COMMAND).click(&mine).hold(Modifiers::NONE).settle();
    assert_eq!(*posted.borrow(), vec![select(json!(["categories/Letter", "filters/Mine"]))]);

    let (mut fixture, posted) = open(model(&["categories/Letter"]));
    let all = fixture.by_label("all").expect("the All entry");
    fixture.click(&all).settle();
    assert_eq!(*posted.borrow(), vec![select(json!(["all"]))]);
}

/// An entry that knows its full set shows present/expected; the others show their count.
#[test]
fn entries_show_their_counts_and_badges() {
    let (fixture, _) = open(model(&[]));
    let texts: Vec<String> = fixture.texts().iter().map(|t| t.text.clone()).collect();
    assert!(texts.iter().any(|t| t == "20/26"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "26") && texts.iter().any(|t| t == "4"), "{texts:?}");
}

/// Actions › Add Folder adds the first free folder name; Remove Filter removes the last selected filter.
#[test]
fn the_actions_menu_posts_the_filter_commands() {
    let (mut fixture, posted) = open(model(&["filters/Mine"]));
    let actions = fixture.by_label("Actions").expect("the Actions button");
    fixture.click(&actions).settle();
    let remove = fixture.by_label("Remove Filter").expect("Remove Filter");
    fixture.click(&remove).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("fontView.removeFilter", json!({ "filter": "filters/Mine" }))]);

    let (mut fixture, posted) = open(model(&[]));
    let actions = fixture.by_label("Actions").expect("the Actions button");
    fixture.click(&actions).settle();
    let add = fixture.by_label("Add Folder").expect("Add Folder");
    fixture.click(&add).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("fontView.addFilterFolder", json!({ "name": "New Folder" }))]);
}

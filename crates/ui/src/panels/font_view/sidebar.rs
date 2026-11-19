//! Font View's sidebar (`fontView.sidebar`): All and the Categories, Languages and Filters sections as a tree, each
//! entry with its glyph count, or a present/expected badge or a check mark when the entry knows its full glyph set
//! (`fontView.sidebar.missingBadge`); the actions menu; and the glyph inspector at its foot.
//!
//! | Gesture | Posts |
//! |---|---|
//! | a click, Cmd-click or Shift-click on an entry | `fontView.selectSidebarEntries` with the entries the list then holds |
//! | the disclosure of a section, group or folder | `fontView.setSidebarEntryOpen` |
//! | renaming a filter or folder in place | `fontView.renameFilter` |
//! | Actions › Add Folder | `fontView.addFilterFolder` with the first free "New Folder" name |
//! | Actions › Remove Filter | `fontView.removeFilter` on the last selected filter |

use std::borrow::Cow;

use craft::behavior::{DisplayOrder, EditEvent, IconId, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked, Trigger};
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use egui::Ui;
use serde_json::json;

use super::inspector;
use crate::app::Msg;
use crate::app::models::glyph_props::GlyphPropsModel;
use crate::app::models::sidebar::{SidebarModel, SidebarRow};
use crate::app::sidebar::{path_value, unique_name};

craft::ui::words! {
    /// The words of the sidebar.
    pub struct SidebarWords {
        /// The entry tree's name.
        entries: "fontcraftSidebar.entries" = "Sidebar Entries";
        /// The check mark of an entry whose glyph set the font has in full.
        complete: "fontcraftSidebar.complete" = "Complete";
        /// The actions menu and its rows.
        actions: "fontcraftSidebar.actions" = "Actions";
        add_folder: "fontcraftSidebar.addFolder" = "Add Folder";
        remove_filter: "fontcraftSidebar.removeFilter" = "Remove Filter";
        /// The name of a new folder.
        new_folder: "fontcraftSidebar.newFolder" = "New Folder";
        /// Without an open font.
        no_font: "fontcraftSidebar.noFont" = "No font is open";
    }
}

/// What the sidebar reads and sends.
pub struct Props<'a> {
    pub model: &'a Tracked<SidebarModel>,
    pub inspector: &'a Tracked<GlyphPropsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<SidebarWords>();
    let model: &SidebarModel = props.model;
    let glyphs: &GlyphPropsModel = props.inspector;
    if !model.has_font {
        Text::new(words.no_font.clone()).show(ui);
        return;
    }
    let (mut body, mut foot) = (Vec::new(), Vec::new());
    let _ = Stack::new()
        .fill()
        .footer(|ui, _| {
            actions(ui, model, &words, &mut foot);
            inspector::show(ui, glyphs, &mut foot);
        })
        .show(ui, |ui, _| {
            if let Some(error) = &model.error {
                Text::new(error.clone()).show(ui);
            }
            entries(ui, model, &words, &mut body);
        });
    props.sent.extend(body.into_iter().chain(foot));
}

/// The rows shown, keyed by their index in the model.
struct Entries<'a> {
    model: &'a SidebarModel,
}

impl Entries<'_> {
    fn row(&self, at: usize) -> Option<&SidebarRow> {
        self.model.visible.get(at).and_then(|&i| self.model.rows.get(i))
    }
}

impl DisplayOrder<usize> for Entries<'_> {
    fn len(&self) -> usize {
        self.model.visible.len()
    }

    fn key(&self, at: usize) -> usize {
        self.model.visible.get(at).copied().unwrap_or(usize::MAX)
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        self.model.visible.iter().position(|i| i == key)
    }

    fn depth(&self, at: usize) -> usize {
        self.row(at).map_or(0, |r| r.depth)
    }

    fn has_children(&self, at: usize) -> bool {
        self.row(at).is_some_and(|r| r.has_children)
    }

    fn open(&self, at: usize) -> bool {
        self.row(at).is_some_and(|r| r.open)
    }

    fn is_section(&self, at: usize) -> bool {
        self.row(at).is_some_and(SidebarRow::is_section)
    }

    fn section_stop(&self, at: usize) -> bool {
        self.is_section(at)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.row(at).map(|r| Cow::Borrowed(r.label.as_str()))
    }
}

impl ItemSource<usize> for Entries<'_> {}

/// The selection the list starts from: the selected entries' rows, or All's row when none is.
fn selection(model: &SidebarModel) -> Selection<usize> {
    let mut rows: Vec<usize> = model.selected.iter().filter_map(|p| model.row_of(p)).collect();
    if model.selected.is_empty()
        && let Some(all) = model.rows.iter().position(|r| r.kind == "all")
    {
        rows.push(all);
    }
    let lead = rows.last().copied();
    Selection::new(rows, lead, lead)
}

fn entries(ui: &mut Ui, model: &SidebarModel, words: &SidebarWords, sent: &mut Vec<Msg>) {
    let mut picked = selection(model);
    let before = picked.items.clone();
    let source = Entries { model };
    let out = List::<usize>::new(words.entries.clone())
        .tree(true)
        .selection(Bind::bound(&mut picked))
        .can_rename(|k: &usize| model.rows.get(*k).is_some_and(SidebarRow::is_filter))
        .show_items(ui, &source, &mut |key, _| {
            let Some(row) = model.rows.get(*key) else { return ListRow::new(String::new()) };
            let mut item = ListRow::new(row.label.clone()).depth(row.depth);
            if row.is_section() {
                item = item.section_heading();
            }
            if row.has_children {
                item = item.group(row.open);
            }
            if row.is_section() {
                item
            } else if row.complete() {
                item.trailing_icon(IconId::new("check"), words.complete.clone())
            } else if let (Some(p), Some(e)) = (row.present, row.expected) {
                item.trailing(format!("{p}/{e}"))
            } else {
                item.trailing(row.count.to_string())
            }
        });
    if picked.items != before {
        sent.push(select(model, &before, &picked.items));
    }
    for event in &out.events {
        match event {
            CollectionEvent::Open(change) => {
                if let Some(row) = model.rows.get(change.key) {
                    let params = json!({ "entry": path_value(&row.path), "open": change.open });
                    sent.push(Msg::Shell(CommandRef::with_params("fontView.setSidebarEntryOpen", params)));
                }
            }
            CollectionEvent::Rename(EditEvent::Commit { after: rename, .. } | EditEvent::Set { after: rename, .. }) => {
                let name = rename.name.trim();
                if let Some(row) = model.rows.get(rename.key)
                    && row.is_filter()
                    && !name.is_empty()
                    && name != row.label
                {
                    sent.push(Msg::run("fontView.renameFilter", json!({ "filter": path_value(&row.path), "name": name })));
                }
            }
            _ => {}
        }
    }
}

/// The `fontView.selectSidebarEntries` for the rows the list left selected: All alone when it was just picked or
/// nothing else is, else the entries without All.
fn select(model: &SidebarModel, before: &[usize], after: &[usize]) -> Msg {
    let is_all = |i: &usize| model.rows.get(*i).is_some_and(|r| r.kind == "all");
    let picked_all = after.iter().any(|i| is_all(i) && !before.contains(i));
    let paths: Vec<_> =
        after.iter().filter(|i| !is_all(i)).filter_map(|i| model.rows.get(*i)).filter(|r| !r.is_section()).map(|r| path_value(&r.path)).collect();
    let entries = if picked_all || paths.is_empty() { vec![json!("all")] } else { paths };
    Msg::Shell(CommandRef::with_params("fontView.selectSidebarEntries", json!({ "entries": entries, "mode": "replace" })))
}

/// The actions menu: Add Folder, and Remove Filter on the last selected filter.
fn actions(ui: &mut Ui, model: &SidebarModel, words: &SidebarWords, sent: &mut Vec<Msg>) {
    let chosen = model.selected.iter().rev().filter_map(|p| model.row_of(p)).filter_map(|i| model.rows.get(i)).find(|r| r.is_filter());
    let opener = Button::new(words.actions.clone()).show(ui);
    Menu::new(words.actions.clone()).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
        if MenuItem::new(words.add_folder.clone()).show(ui).ran {
            let name = unique_name(&model.filter_names(), words.new_folder.as_str());
            sent.push(Msg::run("fontView.addFilterFolder", json!({ "name": name })));
        }
        if MenuItem::new(words.remove_filter.clone()).enabled(chosen.is_some()).show(ui).ran
            && let Some(row) = chosen
        {
            sent.push(Msg::run("fontView.removeFilter", json!({ "filter": path_value(&row.path) })));
        }
    });
}

#[cfg(test)]
#[path = "sidebar_tests.rs"]
mod tests;

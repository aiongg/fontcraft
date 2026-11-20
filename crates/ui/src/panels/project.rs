//! The Project panel: one open project (`fontcraft_engine::project`), with the font link at the top, the project's
//! instance list at the left, the selected instances' fields and custom parameters at the right (as Font Info ›
//! Exports shows an export), and the export controls at the bottom. Every control posts an engine command; the panel
//! keeps only view state (`shell::project`).
//!
//! | Control | Does |
//! |---|---|
//! | Project (several open) | `window.project.select`: the project shown |
//! | Choose… beside the font | `project.linkFont` without `font`: the file picker asks for it |
//! | list entry: click, Cmd-click, Shift-click | `window.project.select` |
//! | list entry: drag onto another | `project.moveInstances` |
//! | list entry: context menu › Copy, Paste, Duplicate | `window.project.copy`, `window.project.paste`, `project.duplicateInstances` on the selection |
//! | Copy, Paste (Cmd+C, Cmd+V) while the instance list has the keyboard focus | `window.project.copy`, `window.project.paste` |
//! | plus menu › Add Instance, Duplicate | `project.addInstance`, `project.duplicateInstances` on the selection |
//! | minus | `project.removeInstances` on the selection |
//! | revert | `project.revertInstances` |
//! | Active, Style Name, Weight Class, Width Class, Style Linking | `project.editInstance` with those keys, once per selected entry |
//! | Custom Parameters: plus | `window.project.addCustomParameter`: the picker, whose Add runs `project.editInstance` |
//! | Custom Parameters: checkbox, value, minus | `project.editInstance` with `customParameters` |
//! | Choose… beside Export Destination | `project.setExportDestination` without `folder`: the folder picker asks |
//! | Save | `project.save`; a project never saved asks the save picker for its file |
//! | Export | `project.export`; disabled until the project is saved, linked to a font and has a destination |
//! | Close | `project.close`, which keeps a project with unsaved changes open and says why in the status line |
//!
//! Axes Coordinates come from the linked font as last read and are read-only.

use std::borrow::Cow;

use craft::behavior::{EditEvent, GesturePhase, IconId, Mixed, Selection, keyed};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, ReorderRule, Show as _, ShowCollection as _, ShowContainer as _, Tracked, Trigger,
};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::menu::Menu;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::{NumberField, TextField};
use craft_ext::ui::list::{CollectionEdits as _, EditCommand};
use egui::Ui;
use serde_json::{Value, json};

use super::font_info::exports_tab::{ExportsWords, class_row, flipped, mixed_bool, mixed_text};
use super::font_info::param_value::{Edit, number_json};
use super::font_info::rows::{self, RowAction};
use crate::app::Msg;
use crate::app::models::font_exports::{ExportFields, WEIGHT_CLASSES, WIDTH_CLASSES};
use crate::app::models::font_info::RowModel;
use crate::app::models::project::{Blocked, ProjectModel, ShownProject};
use crate::shell::project::{ADD_PARAMETER, COPY, PASTE, select};

craft::ui::words! {
    /// The words of the Project panel.
    pub struct ProjectWords {
        none: "fontcraftProject.none" = "No project is open. File › New Project creates one.";
        project: "fontcraftProject.project" = "Project";
        edited: "fontcraftProject.edited" = "(edited)";
        font: "fontcraftProject.font" = "Font";
        no_font: "fontcraftProject.noFont" = "No font linked";
        choose: "fontcraftProject.choose" = "Choose…";
        choose_font: "fontcraftProject.chooseFont" = "Link the project to a font file";
        font_unread: "fontcraftProject.fontUnread" = "The linked font could not be read: choose it again before exporting.";
        no_instances: "fontcraftProject.noInstances" = "This project has no instances. Add one with the plus button, or choose a font to start from its instances.";
        duplicate: "fontcraftProject.duplicate" = "Duplicate";
        revert: "fontcraftProject.revert" = "Revert to the instances of the linked font";
        choose_first: "fontcraftProject.chooseFirst" = "Choose a font first";
        destination: "fontcraftProject.destination" = "Export Destination";
        no_destination: "fontcraftProject.noDestination" = "Choose a folder";
        choose_destination: "fontcraftProject.chooseDestination" = "Choose the folder the project exports into";
        save: "fontcraftProject.save" = "Save";
        save_tip: "fontcraftProject.saveTip" = "Save the project to its file";
        save_as_tip: "fontcraftProject.saveAsTip" = "Choose the file to save the project to";
        export: "fontcraftProject.export" = "Export";
        export_tip: "fontcraftProject.exportTip" = "Export every active instance into the export destination";
        blocked_font: "fontcraftProject.blockedFont" = "Choose the font to export first";
        blocked_destination: "fontcraftProject.blockedDestination" = "Choose an export destination first";
        blocked_never_saved: "fontcraftProject.blockedNeverSaved" = "Save the project first";
        blocked_unsaved: "fontcraftProject.blockedUnsaved" = "Save the project's changes first";
        close: "fontcraftProject.close" = "Close";
        close_tip: "fontcraftProject.closeTip" = "Close the project; one with unsaved changes stays open";
        axes_read_only: "fontcraftProject.axesReadOnly" = "Axes Coordinates (read-only)";
        entry_menu: "fontcraftProject.entryMenu" = "Instance";
        copy: "fontcraftProject.copy" = "Copy";
        paste: "fontcraftProject.paste" = "Paste";
    }
}

/// The Edit commands the instance list takes while it has the keyboard focus.
const LIST_EDITS: [EditCommand; 2] = [EditCommand::Copy, EditCommand::Paste];

pub struct Props<'a> {
    pub model: &'a Tracked<ProjectModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ProjectWords>();
    let exports = Env::of(ui).words::<ExportsWords>();
    let model: &ProjectModel = props.model;
    let Some(p) = &model.shown else {
        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let sent = &mut *props.sent;
    Stack::new().id_salt("fontcraftProject").fill_width().show(ui, |ui, _| {
        if model.projects.len() > 1 {
            let entries: Vec<Entry<u64>> = model.projects.iter().map(|c| Entry::new(c.id, c.title.clone())).collect();
            Field::new(words.project.clone()).show(ui, |ui, _| {
                if let Some(id) = Select::new(words.project.clone(), Bind::given(p.id), &entries).show(ui).events.committed() {
                    sent.push(Msg::Shell(select(*id, &[])));
                }
            });
        }
        let title = if p.modified { format!("{} {}", p.title, words.edited.as_str()) } else { p.title.clone() };
        Text::new(title).show(ui);
        font_row(ui, p, &words, sent);
        Row::new().show(ui, |ui, _| {
            Stack::new().id_salt("fontcraftProject.list").show(ui, |ui, _| instance_list(ui, p, &words, &exports, sent));
            Stack::new().id_salt("fontcraftProject.fields").fill_width().show(ui, |ui, _| {
                let Some(fields) = &p.fields else {
                    Text::new(words.no_instances.clone()).shade(Shade::Muted).show(ui);
                    return;
                };
                Text::new(exports.general.clone()).show(ui);
                general(ui, p, fields, &words, &exports, sent);
                let plus = rows::section(ui, "projectParameters", exports.parameters.clone(), exports.add_parameter.clone(), |ui| {
                    parameter_rows(ui, p, sent);
                });
                if plus {
                    sent.push(Msg::Shell(CommandRef::with_params(ADD_PARAMETER, json!({ "project": p.id }))));
                }
            });
        });
        export_row(ui, p, &words, sent);
    });
}

/// The linked font's path and Choose….
fn font_row(ui: &mut Ui, p: &ShownProject, words: &ProjectWords, sent: &mut Vec<Msg>) {
    Field::new(words.font.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let shown = p.font.clone().map_or_else(|| words.no_font.clone(), Label::from);
            Text::new(shown).show(ui);
            if Button::new(words.choose.clone()).tooltip(words.choose_font.clone()).id_salt("fontcraftProject.chooseFont").show(ui).clicked() {
                sent.push(Msg::run("project.linkFont", json!({ "project": p.id })));
            }
        });
    });
    if p.font_unread {
        Text::new(words.font_unread.clone()).shade(Shade::Muted).show(ui);
    }
}

/// The value of an event that ended: a commit or a set.
fn done<T>(event: &EditEvent<T>) -> Option<&T> {
    match event.phase() {
        Some(GesturePhase::Commit) | None => event.value(),
        _ => None,
    }
}

/// The instance list with its plus menu, minus and revert buttons.
fn instance_list(ui: &mut Ui, p: &ShownProject, words: &ProjectWords, exports: &ExportsWords, sent: &mut Vec<Msg>) {
    let entries = &p.entries;
    let lead = p.selected.first().copied();
    let before = Selection::new(p.selected.clone(), lead, lead);
    let mut after = before.clone();
    let positions: Vec<usize> = (0..entries.len()).collect();
    let name = |at: usize| {
        entries.get(at).map_or_else(String::new, |e| if e.name.is_empty() { exports.untitled.as_str().to_string() } else { e.name.clone() })
    };
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(name(*at)));
    let out = List::<usize>::new(exports.list.clone())
        .header(false)
        .natural_height(true)
        .reorder(ReorderRule::new())
        .selection(Bind::bound(&mut after))
        .show_items(ui, &source, &mut |at, icx| {
            let Some(e) = entries.get(*at) else { return ListRow::new(Label::Empty) };
            let mut row = ListRow::new(name(*at)).state(icx.row.clone());
            if !e.active {
                row = row.ink(RowInk::Dimmed);
            }
            if e.variable {
                row = row.trailing_strong(exports.variable.clone(), exports.variable.clone());
            }
            row
        });
    if after.items != before.items {
        let mut picked = after.items.clone();
        picked.sort_unstable();
        sent.push(Msg::Shell(select(p.id, &picked)));
    }
    for event in &out.events {
        match event {
            CollectionEvent::Reorder(edit) => {
                if let Some(moved) = done(edit)
                    && moved.keys.len() == 1
                    && moved.to != moved.key
                {
                    sent.push(Msg::run("project.moveInstances", json!({ "project": p.id, "instances": [moved.key], "to": moved.to })));
                    sent.push(Msg::Shell(select(p.id, &[moved.to])));
                }
            }
            // A right click outside the selection selects that entry, so the menu acts on it.
            CollectionEvent::ContextMenu(request) if !before.items.contains(&request.key) => {
                sent.push(Msg::Shell(select(p.id, &[request.key])));
            }
            _ => {}
        }
    }
    let count = entries.len();
    let items = [
        Entry::new("copy", words.copy.clone()).enabled(count > 0),
        Entry::new("paste", words.paste.clone()),
        Entry::new("duplicate", words.duplicate.clone()).enabled(count > 0),
    ];
    let shell = |id: &str| Msg::Shell(CommandRef::with_params(id, json!({ "project": p.id })));
    for edit in out.edit_commands(ui, &LIST_EDITS) {
        match edit {
            EditCommand::Copy if count > 0 => sent.push(shell(COPY)),
            EditCommand::Paste => sent.push(shell(PASTE)),
            _ => {}
        }
    }
    match Menu::context(words.entry_menu.clone(), &out.response).id_salt("fontcraftProject.entryMenu").pick(ui, &items).picked {
        Some("copy") => sent.push(shell(COPY)),
        Some("paste") => sent.push(shell(PASTE)),
        Some("duplicate") => sent.push(Msg::run("project.duplicateInstances", json!({ "project": p.id, "instances": p.selected }))),
        _ => {}
    }
    Row::new().show(ui, |ui, _| {
        let plus = IconButton::new(IconId::new("plus")).label(exports.add.clone()).tooltip(exports.add.clone()).show(ui);
        let items = [
            Entry::new("instance", exports.add_instance.clone()),
            Entry::new("duplicate", words.duplicate.clone()).enabled(count > 0),
            Entry::new("variable", exports.add_variable.clone()).enabled(false).disabled_reason(exports.not_yet.clone()),
        ];
        match Menu::new(exports.add.clone()).opened_by(&plus.response, Trigger::Click).pick(ui, &items).picked {
            Some("instance") => {
                sent.push(Msg::run("project.addInstance", json!({ "project": p.id })));
                sent.push(Msg::Shell(select(p.id, &[count])));
            }
            Some("duplicate") => {
                sent.push(Msg::run("project.duplicateInstances", json!({ "project": p.id, "instances": p.selected })));
            }
            _ => {}
        }
        let minus = IconButton::new(IconId::new("minus")).label(exports.remove.clone()).tooltip(exports.remove.clone()).enabled(count > 0);
        if minus.show(ui).clicked() && count > 0 {
            sent.push(Msg::run("project.removeInstances", json!({ "project": p.id, "instances": p.selected })));
            sent.push(Msg::Shell(select(p.id, &[])));
        }
        let linked = p.font.is_some();
        let tip = if linked { words.revert.clone() } else { words.choose_first.clone() };
        let revert = IconButton::new(IconId::new("rotate_ccw")).label(words.revert.clone()).tooltip(tip).enabled(linked);
        if revert.show(ui).clicked() && linked {
            sent.push(Msg::run("project.revertInstances", json!({ "project": p.id })));
            sent.push(Msg::Shell(select(p.id, &[])));
        }
    });
}

/// One `project.editInstance` per entry of `targets` with the same `changes`.
fn edits(project: u64, targets: &[usize], changes: &Value) -> Vec<Msg> {
    targets.iter().map(|k| Msg::run("project.editInstance", json!({ "project": project, "instance": k, "changes": changes }))).collect()
}

/// The built-in fields of the selected entries.
fn general(ui: &mut Ui, p: &ShownProject, f: &ExportFields, words: &ProjectWords, exports: &ExportsWords, sent: &mut Vec<Msg>) {
    let all = |key: &str, value: Value| edits(p.id, &p.selected, &json!({ key: value }));
    let statics = |key: &str, value: Value| edits(p.id, &f.statics, &json!({ key: value }));
    Field::new(exports.active.clone()).show(ui, |ui, _| {
        let check = Checkbox::new(Bind::given(mixed_bool(f.active))).label(exports.active.clone()).tooltip(exports.active_tip.clone());
        if check.show(ui).events.committed().is_some() {
            sent.extend(all("active", json!(flipped(f.active))));
        }
    });
    let name_label = if f.variable { exports.name.clone() } else { exports.style_name.clone() };
    Field::new(name_label.clone()).show(ui, |ui, _| {
        if let Some(text) = TextField::new(name_label.clone(), Bind::given(mixed_text(&f.name, f.name_mixed))).show(ui).events.committed() {
            sent.extend(all("styleName", json!(text)));
        }
    });
    if f.variable {
        return;
    }
    let names: Vec<Label> = exports.weights().into_iter().cloned().collect();
    if let Some(v) = class_row(ui, &exports.weight_class, "projectWeight", f.weight, &WEIGHT_CLASSES, &names, &exports.standard_values) {
        sent.extend(statics("weightClass", number_json(v)));
    }
    let names: Vec<Label> = exports.widths().into_iter().cloned().collect();
    if let Some(v) = class_row(ui, &exports.width_class, "projectWidth", f.width, &WIDTH_CLASSES, &names, &exports.standard_values) {
        sent.extend(statics("widthClass", number_json(v)));
    }
    if !f.axes.is_empty() {
        Text::new(words.axes_read_only.clone()).shade(Shade::Muted).show(ui);
        for (at, (axis, value)) in f.axes.iter().enumerate() {
            Field::new(axis.clone()).id_salt(("fontcraftProject.axis", at)).show(ui, |ui, _| {
                let given = value.map_or(Mixed::Mixed, |v| Mixed::Same(Some(v)));
                let _ = NumberField::new(axis.clone(), Bind::given(given)).enabled(false).show(ui);
            });
        }
    }
    Field::new(exports.style_linking.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let bold = Checkbox::new(Bind::given(mixed_bool(f.bold))).label(exports.bold.clone()).tooltip(exports.bold_tip.clone());
            if bold.show(ui).events.committed().is_some() {
                sent.extend(statics("isBold", json!(flipped(f.bold))));
            }
            let italic = Checkbox::new(Bind::given(mixed_bool(f.italic))).label(exports.italic.clone()).tooltip(exports.italic_tip.clone());
            if italic.show(ui).events.committed().is_some() {
                sent.extend(statics("isItalic", json!(flipped(f.italic))));
            }
            let link = TextField::new(exports.link_style.clone(), Bind::given(mixed_text(&f.link_style, f.link_mixed)))
                .placeholder(exports.link_style.clone());
            if let Some(text) = link.show(ui).events.committed() {
                sent.extend(statics("linkStyle", json!(text.trim())));
            }
        });
    });
}

/// The `customParameters` edit of `project.editInstance` a row action makes; `None` for an action a project does not
/// take (a value dialog, a language edit).
pub fn parameter_edit(row: &RowModel, action: &RowAction) -> Option<Value> {
    match action {
        RowAction::Edit(Edit::Value(v)) => Some(json!({"action": "set", "name": row.key, "index": row.occurrence, "value": v})),
        RowAction::SetEnabled(on) => Some(json!({"action": "set", "name": row.key, "index": row.occurrence, "enabled": on})),
        RowAction::Remove => Some(json!({"action": "remove", "name": row.key, "index": row.occurrence})),
        RowAction::Edit(_) => None,
    }
}

/// The selected entries' custom parameters; an edit goes to every selected entry.
fn parameter_rows(ui: &mut Ui, p: &ShownProject, sent: &mut Vec<Msg>) {
    for row in &p.params {
        if let Some(edit) = rows::one(ui, row, &[], &[]).and_then(|a| parameter_edit(row, &a)) {
            sent.extend(edits(p.id, &p.selected, &json!({ "customParameters": [edit] })));
        }
    }
}

/// The Export Destination, Save, Export and Close.
fn export_row(ui: &mut Ui, p: &ShownProject, words: &ProjectWords, sent: &mut Vec<Msg>) {
    Field::new(words.destination.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let shown = p.destination.clone().map_or_else(|| words.no_destination.clone(), Label::from);
            Text::new(shown).show(ui);
            let choose = Button::new(words.choose.clone()).tooltip(words.choose_destination.clone()).id_salt("fontcraftProject.chooseDestination");
            if choose.show(ui).clicked() {
                sent.push(Msg::run("project.setExportDestination", json!({ "project": p.id })));
            }
        });
    });
    Row::new().show(ui, |ui, _| {
        let save_tip = if p.path.is_some() { words.save_tip.clone() } else { words.save_as_tip.clone() };
        if Button::new(words.save.clone()).tooltip(save_tip).show(ui).clicked() {
            // A saved project is saved to its own file; one never saved has no path, and the save picker asks.
            let params = match &p.path {
                Some(path) => json!({ "project": p.id, "path": path }),
                None => json!({ "project": p.id }),
            };
            sent.push(Msg::run("project.save", params));
        }
        let tip = match p.blocked {
            None => words.export_tip.clone(),
            Some(Blocked::NoFont) => words.blocked_font.clone(),
            Some(Blocked::NoDestination) => words.blocked_destination.clone(),
            Some(Blocked::NeverSaved) => words.blocked_never_saved.clone(),
            Some(Blocked::Unsaved) => words.blocked_unsaved.clone(),
        };
        if Button::new(words.export.clone()).tooltip(tip).enabled(p.blocked.is_none()).show(ui).clicked() && p.blocked.is_none() {
            sent.push(Msg::run("project.export", json!({ "project": p.id })));
        }
        if Button::new(words.close.clone()).tooltip(words.close_tip.clone()).show(ui).clicked() {
            sent.push(Msg::run("project.close", json!({ "project": p.id })));
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;
    use crate::app::models::font_exports::{ExportEntry, Flag};

    fn shown() -> ProjectModel {
        let fields = ExportFields { active: Flag { on: true, mixed: false }, name: "Regular".into(), statics: vec![0, 1], ..Default::default() };
        ProjectModel {
            projects: Vec::new(),
            shown: Some(ShownProject {
                id: 7,
                title: "Family".into(),
                entries: vec![
                    ExportEntry { name: "Regular".into(), active: true, variable: false },
                    ExportEntry { name: "Bold".into(), active: true, variable: false },
                ],
                selected: vec![0, 1],
                fields: Some(fields),
                blocked: Some(Blocked::NeverSaved),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn project_panel_edits_every_selected_instance_and_save_asks_for_a_file_when_never_saved() {
        let model = Tracked::new(shown());
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let active = fixture.by_label("Active").expect("the Active check box");
        fixture.click(&active).settle();
        let save = fixture.by_label("Save").expect("the Save button");
        fixture.click(&save).settle();
        let export = fixture.by_label("Export").expect("the Export button");
        fixture.click(&export).settle();
        let edit = |k: usize| Msg::run("project.editInstance", json!({"project": 7, "instance": k, "changes": {"active": false}}));
        assert_eq!(*posted.borrow(), vec![edit(0), edit(1), Msg::run("project.save", json!({"project": 7}))], "Export is disabled until saved");
    }

    /// A right click on a selected entry opens its menu: Copy and Paste post the project's commands. The plus beside
    /// Custom Parameters asks for the picker.
    #[test]
    fn project_panel_context_menu_copies_and_pastes_and_the_plus_opens_the_picker() {
        let model = Tracked::new(shown());
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.room(egui::vec2(900.0, 700.0)).settle();
        let mut pick = |item: &str| {
            let at = fixture.by_label("Regular").expect("the Regular entry").center();
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
            fixture.frame(vec![egui::Event::PointerMoved(at), button(true)]).frame(vec![button(false)]).settle();
            let row = fixture.by_label(item).unwrap_or_else(|| panic!("the context menu's {item}"));
            fixture.click(&row).settle();
        };
        pick("Copy");
        pick("Paste");
        let plus = fixture.by_label("Add a custom parameter").expect("the Custom Parameters plus");
        fixture.click(&plus).settle();
        let got = posted.borrow().clone();
        let shell = |id: &str| Msg::Shell(CommandRef::with_params(id, json!({"project": 7})));
        for want in [shell(COPY), shell(PASTE), shell(ADD_PARAMETER)] {
            assert!(got.contains(&want), "{want:?} missing from {got:?}");
        }
    }

    /// Copy and Paste fired while the instance list has the keyboard focus reach the list, which posts the project's
    /// commands; Select All stays with the document.
    #[test]
    fn edit_commands_reach_the_focused_instance_list() {
        use craft_ext::ui::list::deliver;
        let model = Tracked::new(shown());
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.room(egui::vec2(900.0, 700.0)).settle();
        let regular = fixture.by_label("Regular").expect("the Regular entry");
        fixture.click(&regular).settle();
        posted.borrow_mut().clear();
        let ctx = fixture.ctx().clone();
        for edit in [EditCommand::Copy, EditCommand::Paste] {
            assert!(deliver(&ctx, edit), "{edit:?}");
            fixture.settle();
        }
        assert!(!deliver(&ctx, EditCommand::SelectAll));
        let shell = |id: &str| Msg::Shell(CommandRef::with_params(id, json!({"project": 7})));
        assert_eq!(*posted.borrow(), vec![shell(COPY), shell(PASTE)]);
    }

    #[test]
    fn a_parameter_row_action_becomes_a_custom_parameters_edit() {
        let row = RowModel { key: "Axis Location".into(), occurrence: 1, ..Default::default() };
        assert_eq!(parameter_edit(&row, &RowAction::Remove), Some(json!({"action": "remove", "name": "Axis Location", "index": 1})));
        assert_eq!(
            parameter_edit(&row, &RowAction::SetEnabled(false)),
            Some(json!({"action": "set", "name": "Axis Location", "index": 1, "enabled": false}))
        );
    }
}

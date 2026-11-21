//! The body of the Extension Manager's Indexes tab, which Settings › Extensions shows too: the Modules and Macros
//! folders with Show, the switch that also writes the extension log to standard error, the list of extension indexes
//! (address without credentials, entry count, state or error, an automatic-updates switch and Remove), the Add field
//! and Reload.
//!
//! | Control | Posts |
//! |---|---|
//! | Show | `file.showInFinder {path}`: the folder in the file manager |
//! | log switch | `settings.set addons.useSystemConsoleForScriptOutput` |
//! | Automatic updates, Remove, Add | `settings.set addons.alternatePluginRepos` with the whole list; the indexes are then read again |
//! | Reload | `pluginManager.reloadIndexes {}`: the HTTPS indexes are downloaded again |

use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::extension_index::{IndexSource, MAX_INDEXES, SOURCES_SETTING, parse_sources, sources_text};
use serde_json::json;

use crate::app::Msg;
use crate::shell::extension_manager::{IndexState, IndexesModel, LOG_SETTING};

craft::ui::words! {
    /// The words of the Indexes body.
    pub struct IndexesWords {
        modules_folder: "fontcraftExtensionIndexes.modulesFolder" = "Modules folder";
        macros_folder: "fontcraftExtensionIndexes.macrosFolder" = "Macros folder";
        no_folder: "fontcraftExtensionIndexes.noFolder" = "none in this session";
        show: "fontcraftExtensionIndexes.show" = "Show";
        show_tip: "fontcraftExtensionIndexes.showTip" = "Show the folder in the file manager";
        log: "fontcraftExtensionIndexes.log" = "Also write the extension log to standard error";
        heading: "fontcraftExtensionIndexes.heading" = "Extension indexes";
        empty: "fontcraftExtensionIndexes.empty" = "No index yet: add a full path to an index file or an https:// URL.";
        entries: "fontcraftExtensionIndexes.entries" = "entries";
        downloading: "fontcraftExtensionIndexes.downloading" = "Downloading…";
        unread: "fontcraftExtensionIndexes.unread" = "Not read yet";
        automatic: "fontcraftExtensionIndexes.automatic" = "Automatic updates";
        automatic_tip: "fontcraftExtensionIndexes.automaticTip" = "Install this index's updates when the indexes are read; off, they are only offered";
        remove: "fontcraftExtensionIndexes.remove" = "Remove";
        new_index: "fontcraftExtensionIndexes.newIndex" = "New index (/path/to/index.json or https://…)";
        add: "fontcraftExtensionIndexes.add" = "Add";
        reload: "fontcraftExtensionIndexes.reload" = "Reload";
        reload_tip: "fontcraftExtensionIndexes.reloadTip" = "Read every index again and download the ones on the network again";
        full: "fontcraftExtensionIndexes.full" = "The list is full.";
    }
}

/// Longest text the Add field takes, in characters.
pub const MAX_NEW_INDEX_CHARS: usize = 4_096;

fn write_sources(sources: &[IndexSource], sent: &mut Vec<Msg>) {
    sent.push(Msg::run("settings.set", json!({ "id": SOURCES_SETTING, "value": sources_text(sources) })));
}

/// Shows the body. `new_index` is the Add field's text, kept by the window that shows it.
pub fn show(ui: &mut Ui, model: &IndexesModel, new_index: &mut String, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<IndexesWords>();
    for (label, folder, salt) in [
        (&words.modules_folder, &model.modules_folder, "fontcraftExtensionIndexes.showModules"),
        (&words.macros_folder, &model.macros_folder, "fontcraftExtensionIndexes.showMacros"),
    ] {
        let _ = Row::new().show(ui, |ui, _| {
            let path = folder.clone().unwrap_or_else(|| words.no_folder.as_str().to_string());
            Text::new(format!("{}: {path}", label.as_str())).show(ui);
            let show = Button::new(words.show.clone()).tooltip(words.show_tip.clone()).enabled(folder.is_some()).id_salt(salt);
            if show.show(ui).clicked()
                && let Some(path) = folder
            {
                sent.push(Msg::Shell(CommandRef::with_params("file.showInFinder", json!({ "path": path }))));
            }
        });
    }
    if let Some(on) = Checkbox::new(Bind::given(model.log_to_stderr)).label(words.log.clone()).show(ui).events.committed() {
        sent.push(Msg::run("settings.set", json!({ "id": LOG_SETTING, "value": on })));
    }
    Text::new(words.heading.clone()).role(TypeRole::Heading).show(ui);
    if model.rows.is_empty() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
    }
    for (at, row) in model.rows.iter().enumerate() {
        let _ = Row::new().show(ui, |ui, _| {
            Text::new(row.shown.clone()).show(ui);
            let state = match &row.state {
                IndexState::Unread => words.unread.as_str().to_string(),
                IndexState::Downloading => words.downloading.as_str().to_string(),
                IndexState::Entries(n) => format!("{n} {}", words.entries.as_str()),
                IndexState::Error(e) => e.clone(),
            };
            Text::new(state).shade(Shade::Muted).show(ui);
            let check = Checkbox::new(Bind::given(row.automatic))
                .label(words.automatic.clone())
                .tooltip(words.automatic_tip.clone())
                .id_salt(("fontcraftExtensionIndexes.automatic", at));
            if let Some(on) = check.show(ui).events.committed() {
                let mut list = model.sources.clone();
                if let Some(source) = list.get_mut(at) {
                    source.automatic_updates = *on;
                }
                write_sources(&list, sent);
            }
            if Button::new(words.remove.clone()).id_salt(("fontcraftExtensionIndexes.remove", at)).show(ui).clicked() {
                let list: Vec<IndexSource> = model.sources.iter().enumerate().filter(|(i, _)| *i != at).map(|(_, s)| s.clone()).collect();
                write_sources(&list, sent);
            }
        });
    }
    let _ = TextField::new(words.new_index.clone(), Bind::bound(new_index)).limit(MAX_NEW_INDEX_CHARS).show(ui);
    let room = model.sources.len() < MAX_INDEXES;
    let _ = Row::new().show(ui, |ui, _| {
        let add = Button::new(words.add.clone()).enabled(room && !new_index.trim().is_empty()).id_salt("fontcraftExtensionIndexes.add");
        if add.show(ui).clicked() && room {
            let mut list = model.sources.clone();
            list.extend(parse_sources(new_index));
            list.truncate(MAX_INDEXES);
            new_index.clear();
            write_sources(&list, sent);
        }
        let reload = Button::new(words.reload.clone()).tooltip(words.reload_tip.clone()).id_salt("fontcraftExtensionIndexes.reload");
        if reload.show(ui).clicked() {
            sent.push(Msg::run("pluginManager.reloadIndexes", json!({})));
        }
    });
    if !room {
        Text::new(words.full.clone()).shade(Shade::Muted).show(ui);
    }
    if let Some(problem) = &model.problem {
        Text::new(problem.clone()).show(ui);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};

    use super::*;
    use crate::shell::extension_manager::IndexRow;

    /// The automatic-updates switch, Remove and Add write the whole list, keeping the credentials the rows do not
    /// show; Show reveals the folder and Reload reads the indexes again. The model stays as it was, so each edit starts
    /// from the same list.
    #[test]
    fn extension_indexes_edit_the_list_and_show_folders() {
        let model = IndexesModel {
            modules_folder: Some("/data/Modules".into()),
            macros_folder: None,
            log_to_stderr: false,
            sources: vec![
                IndexSource { location: "https://user:pw@example.test/index.json".into(), automatic_updates: true },
                IndexSource { location: "/local/index.json".into(), automatic_updates: false },
            ],
            rows: vec![
                IndexRow { shown: "https://example.test/index.json".into(), automatic: true, state: IndexState::Entries(3) },
                IndexRow { shown: "/local/index.json".into(), automatic: false, state: IndexState::Error("not found".into()) },
            ],
            problem: None,
        };
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut new_index = String::new();
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, &model, &mut new_index, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.room(egui::vec2(900.0, 700.0)).settle();
        assert!(fixture.by_label("https://user:pw@example.test/index.json").is_none(), "credentials are never shown");
        // Repeated labels resolve to the first row.
        let automatic = fixture.by_label("Automatic updates").expect("the first row's switch");
        fixture.click(&automatic).settle();
        let remove = fixture.by_label("Remove").expect("the first row's Remove");
        fixture.click(&remove).settle();
        let field = fixture.by_label("New index (/path/to/index.json or https://…)").expect("the Add field");
        fixture.click(&field).type_text("/more/index.json").key(Key::Enter, Modifiers::NONE).settle();
        let add = fixture.by_label("Add").expect("Add");
        fixture.click(&add).settle();
        let show_modules = fixture.by_label("Show").expect("Show beside the Modules folder");
        fixture.click(&show_modules).settle();
        let reload = fixture.by_label("Reload").expect("Reload");
        fixture.click(&reload).settle();
        let set = |text: &str| Msg::run("settings.set", json!({"id": SOURCES_SETTING, "value": text}));
        assert_eq!(
            *posted.borrow(),
            vec![
                set("manual https://user:pw@example.test/index.json\nmanual /local/index.json"),
                set("manual /local/index.json"),
                set("https://user:pw@example.test/index.json\nmanual /local/index.json\n/more/index.json"),
                Msg::Shell(CommandRef::with_params("file.showInFinder", json!({"path": "/data/Modules"}))),
                Msg::run("pluginManager.reloadIndexes", json!({})),
            ]
        );
    }
}

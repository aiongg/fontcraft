//! The open documents and the recent files, as rows: the end of the Window menu, the document pop-up at the title
//! bar's end, and the File › Open Recent submenu.
//!
//! | Place | Rows | Command |
//! |---|---|---|
//! | Window | one per open font, then one per open project, at most [`MAX_LISTED`], the rest under More Documents; the active font checked, "Edited" after the name of one with unsaved changes | `document.activate {"index"}`, `window.project.select {"project", "show"}` |
//! | Document pop-up | the same documents in one list, shown while two or more are open | the same |
//! | File › Open Recent | the recent files by name, newest first, with the folder where names repeat; a file that is gone greyed out; then Clear Menu | `file.openRecent {"path"}`, `file.openRecent {"clear"}` |
//!
//! The rows are rebuilt when the documents or the stored list change ([`refresh`]). Whether the recent files exist is
//! read when the list changes and when the window gets the focus ([`on_focus`]), never per frame. Rows:
//! `window.documentsMenu`, `file.openRecent`, `document.activate`.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use craft::menu::{ItemKind, Row};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Entry, Props as _, Show as _, Trigger};
use craft::ui::menu::Menu;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use egui::Ui;
use fontcraft_engine::Session;
use serde_json::json;

use super::FontCraftApp;

/// The submenu of File that lists the recent files.
pub const RECENT_MENU: &[&str] = &["File", "Open Recent"];
/// The command a recent file's row runs. The submenu takes the place of its own menu row.
pub const OPEN_RECENT: &str = "file.openRecent";
/// The menu the open documents are listed in.
const WINDOW: &str = "Window";
/// The submenu of Window that lists the documents past [`MAX_LISTED`].
const MORE_MENU: &[&str] = &["Window", "More Documents"];
/// Most documents the Window menu lists itself.
pub const MAX_LISTED: usize = 32;
/// Most documents listed at all.
const MAX_DOCUMENTS: usize = super::menus_dynamic::MAX_ROWS;
/// Longest name or folder shown, in characters.
const MAX_LABEL_CHARS: usize = 120;

/// An open document as a menu lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenDocument {
    /// Shows the document.
    pub command: CommandRef,
    /// Its name, the folder where names repeat, and Edited while it has unsaved changes.
    pub label: String,
    /// The active font.
    pub active: bool,
}

/// The open fonts in the session's order, then the open projects.
pub fn open_documents(session: &Session) -> Vec<OpenDocument> {
    let active = session.active().map(|d| d.id);
    let fonts = session.documents().iter().enumerate().map(|(index, d)| {
        (CommandRef::with_params("document.activate", json!({"index": index})), &d.title, d.path.as_deref(), d.modified(), active == Some(d.id))
    });
    let projects = session.projects().iter().map(|p| {
        let command = CommandRef::with_params("window.project.select", json!({"project": p.id, "show": true}));
        (command, &p.title, p.path.as_deref(), p.modified(), false)
    });
    let listed: Vec<_> = fonts.chain(projects).take(MAX_DOCUMENTS).collect();
    let names: Vec<(String, Option<&Path>)> = listed.iter().map(|(_, title, path, ..)| (title.to_string(), *path)).collect();
    let labels = told_apart(&names);
    listed
        .into_iter()
        .zip(labels)
        .map(|((command, _, _, modified, active), label)| {
            let label = if modified { format!("{label} — Edited") } else { label };
            OpenDocument { command, label, active }
        })
        .collect()
}

/// Each name, with its file's folder after it where two names are the same: the folder's own name, or its whole path
/// where that repeats as well.
fn told_apart(names: &[(String, Option<&Path>)]) -> Vec<String> {
    let folder = |path: Option<&Path>| path.and_then(Path::parent).and_then(Path::file_name).map(|f| f.to_string_lossy().into_owned());
    let mut by_name: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_folder: BTreeMap<(&str, Option<String>), usize> = BTreeMap::new();
    for (name, path) in names {
        *by_name.entry(name.as_str()).or_default() += 1;
        *by_folder.entry((name.as_str(), folder(*path))).or_default() += 1;
    }
    names
        .iter()
        .map(|(name, path)| {
            let alone = by_name.get(name.as_str()).is_none_or(|n| *n < 2);
            let own = folder(*path);
            let shown = match (alone, &own) {
                (true, _) | (_, None) => None,
                (false, Some(own)) => {
                    let repeats = by_folder.get(&(name.as_str(), Some(own.clone()))).is_some_and(|n| *n > 1);
                    let whole = path.and_then(Path::parent).map(|f| f.to_string_lossy().into_owned());
                    if repeats { whole } else { Some(own.clone()) }
                }
            };
            match shown {
                Some(folder) => format!("{} — {}", short(name), short_end(&folder)),
                None => short(name),
            }
        })
        .collect()
}

/// At most [`MAX_LABEL_CHARS`] characters of `text`, cut at the end.
fn short(text: &str) -> String {
    if text.chars().count() <= MAX_LABEL_CHARS {
        return text.to_string();
    }
    text.chars().take(MAX_LABEL_CHARS.saturating_sub(1)).chain(['…']).collect()
}

/// At most [`MAX_LABEL_CHARS`] characters of `text`, cut at the start: a path's end tells it apart.
fn short_end(text: &str) -> String {
    let count = text.chars().count();
    if count <= MAX_LABEL_CHARS {
        return text.to_string();
    }
    let skip = count.saturating_sub(MAX_LABEL_CHARS.saturating_sub(1));
    std::iter::once('…').chain(text.chars().skip(skip)).collect()
}

/// The Window menu's document rows, after a separator.
fn document_rows(documents: &[OpenDocument]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (at, document) in documents.iter().enumerate() {
        if at == 0 {
            rows.push(Row::separator(&[WINDOW]));
        }
        let menu: &[&str] = if at < MAX_LISTED { &[WINDOW] } else { MORE_MENU };
        rows.push(Row::new(menu, document.command.clone(), document.label.clone()).kind(ItemKind::Check(document.active)));
    }
    rows
}

/// The recent files as last read, and which of them are gone.
#[derive(Clone, Debug, Default)]
pub struct RecentFiles {
    /// The stored paths and whether each file is there; `None` until the list is read.
    files: Option<Vec<(String, bool)>>,
    /// The window got the focus since the files were last looked for.
    look_again: bool,
}

/// The rows of File › Open Recent.
fn recent_rows(files: &[(String, bool)]) -> Vec<Row> {
    if files.is_empty() {
        return vec![Row::new(RECENT_MENU, CommandRef::new(OPEN_RECENT), "No Recent Files").enabled(false)];
    }
    let names: Vec<(String, Option<&Path>)> = files
        .iter()
        .map(|(path, _)| {
            let file = Path::new(path);
            (file.file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned()), Some(file))
        })
        .collect();
    let mut rows: Vec<Row> = files
        .iter()
        .zip(told_apart(&names))
        .map(|((path, there), label)| {
            let row = Row::new(RECENT_MENU, CommandRef::with_params(OPEN_RECENT, json!({"path": path})), label).enabled(*there);
            if *there { row.tooltip(path.clone()) } else { row.tooltip(format!("{path} no longer exists")) }
        })
        .collect();
    rows.push(Row::separator(RECENT_MENU));
    rows.push(Row::new(RECENT_MENU, CommandRef::with_params(OPEN_RECENT, json!({"clear": true})), "Clear Menu"));
    rows
}

/// The window got the focus: files may have been moved meanwhile, so [`refresh`] looks for them again.
pub fn on_focus(app: &mut FontCraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) {
        app.recent_files.look_again = true;
    }
}

/// Rebuilds the document rows and the recent rows when what they show changed.
pub fn refresh(app: &mut FontCraftApp) {
    let FontCraftApp { session, catalog, recent_files, .. } = app;
    let documents = open_documents(session);
    let mut h = DefaultHasher::new();
    for d in &documents {
        (&d.label, d.active, &d.command.id, d.command.params.to_string()).hash(&mut h);
    }
    catalog.set_document_rows(h.finish(), || document_rows(&documents));

    let stored = session.settings().stored_recent_documents();
    let known = recent_files.files.as_ref().is_some_and(|files| files.iter().map(|(path, _)| path).eq(stored.iter()));
    if !known || std::mem::take(&mut recent_files.look_again) {
        recent_files.look_again = false;
        recent_files.files = Some(stored.into_iter().map(|path| (Path::new(&path).exists(), path)).map(|(there, path)| (path, there)).collect());
    }
    let files = recent_files.files.as_deref().unwrap_or_default();
    let mut h = DefaultHasher::new();
    files.hash(&mut h);
    catalog.set_recent_rows(h.finish(), || recent_rows(files));
}

craft::ui::words! {
    /// The words of the document pop-up.
    pub struct DocumentWords {
        /// The pop-up's button and menu.
        documents: "fontcraftDocuments.documents" = "Documents";
        /// The button's tooltip.
        tip: "fontcraftDocuments.tip" = "Show another open document";
    }
}

/// The pop-up's button; `show_menu` hangs the menu from its response.
pub fn button(ui: &mut Ui) -> egui::Response {
    let words = Env::of(ui).words::<DocumentWords>();
    IconButton::new(craft::behavior::IconId::new("chevron_down"))
        .label(words.documents.clone())
        .tooltip(words.tip.clone())
        .id_salt("fontcraftDocuments.button")
        .show(ui)
        .response
}

/// The documents' menu, opened by `button`; returns the command of the row that ran.
pub fn show_menu(ui: &mut Ui, button: &egui::Response, documents: &[OpenDocument]) -> Option<CommandRef> {
    let words = Env::of(ui).words::<DocumentWords>();
    let entries: Vec<Entry<usize>> = documents.iter().enumerate().map(|(at, d)| Entry::new(at, d.label.clone()).checked(d.active)).collect();
    let picked = Menu::new(words.documents.clone()).opened_by(button, Trigger::Click).pick(ui, &entries).picked;
    picked.and_then(|at| documents.get(at)).map(|d| d.command.clone())
}

#[cfg(test)]
#[path = "document_menu_tests.rs"]
mod tests;

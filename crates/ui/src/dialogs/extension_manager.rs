//! The Extension Manager window: shows `shell::extension_manager`'s state while it is open. The filters post their
//! shell commands; the buttons under the list act on the selected row through engine commands, and Website and Donate
//! through `pluginManager.openWebsite` and `pluginManager.openDonationPage`. Install from File…
//! posts `plugin.installFromFile` without a path, so the file picker asks for it.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;

use crate::app::{FontCraftApp, Msg};
use crate::shell::extension_manager::{self, KINDS, MAX_QUERY_CHARS, ManagerEntry, ManagerView, STATES};

craft::ui::words! {
    /// The words of the Extension Manager.
    pub struct ManagerWords {
        title: "fontcraftExtensionManager.title" = "Extension Manager";
        install_state: "fontcraftExtensionManager.installState" = "Show";
        all: "fontcraftExtensionManager.all" = "All";
        installed: "fontcraftExtensionManager.installed" = "Installed";
        not_installed: "fontcraftExtensionManager.notInstalled" = "Not Installed";
        kind: "fontcraftExtensionManager.kind" = "Kind";
        filters: "fontcraftExtensionManager.filters" = "Filters";
        reporters: "fontcraftExtensionManager.reporters" = "Reporters";
        checks: "fontcraftExtensionManager.checks" = "Checks";
        macros: "fontcraftExtensionManager.macros" = "Macros";
        search: "fontcraftExtensionManager.search" = "Search extensions";
        list: "fontcraftExtensionManager.list" = "Extensions";
        name: "fontcraftExtensionManager.name" = "Name";
        version: "fontcraftExtensionManager.version" = "Version";
        status: "fontcraftExtensionManager.status" = "State";
        problem: "fontcraftExtensionManager.problem" = "Problem";
        on: "fontcraftExtensionManager.on" = "On";
        off: "fontcraftExtensionManager.off" = "Off";
        new_module: "fontcraftExtensionManager.newModule" = "New, off";
        update_to: "fontcraftExtensionManager.updateTo" = "Update offered, installed:";
        none: "fontcraftExtensionManager.none" = "No extension matches. Install a module or macro from a file, or add an extension index.";
        website: "fontcraftExtensionManager.website" = "Website";
        website_tip: "fontcraftExtensionManager.websiteTip" = "Open the extension's website in the browser";
        donate: "fontcraftExtensionManager.donate" = "Donate";
        donate_tip: "fontcraftExtensionManager.donateTip" = "Open the vendor's donation page in the browser";
        install: "fontcraftExtensionManager.install" = "Install";
        update: "fontcraftExtensionManager.update" = "Update";
        remove: "fontcraftExtensionManager.remove" = "Remove";
        switch: "fontcraftExtensionManager.switch" = "Turned on";
        switch_tip: "fontcraftExtensionManager.switchTip" = "Run this module from its menu items, palette section or canvas drawing";
        install_file: "fontcraftExtensionManager.installFile" = "Install from File…";
        reload: "fontcraftExtensionManager.reload" = "Reload";
        close: "fontcraftExtensionManager.close" = "Close";
        safe_mode: "fontcraftExtensionManager.safeMode" = "Safe mode: modules are listed but none runs until Help › Load Extensions.";
        failed: "fontcraftExtensionManager.failed" = "Module files that could not be loaded:";
        select_hint: "fontcraftExtensionManager.selectHint" = "Select an extension to install, update, remove or turn it on or off.";
    }
}

/// Rows the list shows before it scrolls: the dialog body scrolls too, so the list must be capped.
const LIST_ROWS: u16 = 14;
/// The footer button positions `FormDialog` reports.
const OK: usize = 0;
const FIRST_EXTRA: usize = 2;

const NAME: ColumnKey = ColumnKey::new("name");
const KIND: ColumnKey = ColumnKey::new("kind");
const VERSION: ColumnKey = ColumnKey::new("version");
const STATUS: ColumnKey = ColumnKey::new("status");
const PROBLEM: ColumnKey = ColumnKey::new("problem");

/// The window: a copy of the app's state, the dialog's own filter values and the messages to post.
pub struct ManagerDialog {
    id: DialogId,
    view: ManagerView,
    state: usize,
    kind: usize,
    query: String,
    /// The search text posted and not yet in the app's state.
    posted_query: Option<String>,
    /// The selected row's id.
    selected: Option<String>,
    posted: Vec<Msg>,
    /// The app closed the window: the next pass ends it.
    closing: bool,
    open: bool,
}

impl ManagerDialog {
    fn new(view: &ManagerView) -> ManagerDialog {
        let mut dialog = ManagerDialog {
            id: dialog_id("fontcraft.extensionManager"),
            view: ManagerView::default(),
            state: 0,
            kind: 0,
            query: view.query.clone(),
            posted_query: None,
            selected: None,
            posted: Vec::new(),
            closing: false,
            open: true,
        };
        dialog.take(view);
        dialog
    }

    /// Shows `view` from now on. The search text the person is typing stays until the app has taken it.
    fn take(&mut self, view: &ManagerView) {
        self.state = STATES.iter().position(|s| *s == view.state).unwrap_or(0);
        self.kind = KINDS.iter().position(|k| *k == view.kind).unwrap_or(0);
        match &self.posted_query {
            None => self.query = view.query.clone(),
            Some(q) if *q == view.query => self.posted_query = None,
            Some(_) => {}
        }
        self.view = view.clone();
    }

    fn post_shell(&mut self, id: &str, params: serde_json::Value) {
        self.posted.push(Msg::Shell(CommandRef::with_params(id, params)));
    }

    /// The filter buttons and the search field.
    fn filters(&mut self, ui: &mut Ui, words: &ManagerWords) {
        let states = [Entry::new(0, words.all.clone()), Entry::new(1, words.installed.clone()), Entry::new(2, words.not_installed.clone())];
        let kinds = [
            Entry::new(0, words.all.clone()),
            Entry::new(1, words.filters.clone()),
            Entry::new(2, words.reporters.clone()),
            Entry::new(3, words.checks.clone()),
            Entry::new(4, words.macros.clone()),
        ];
        let (state, kind, query) = (self.state, self.kind, self.query.clone());
        Row::new().show(ui, |ui, _| {
            Segmented::new(words.install_state.clone(), Bind::bound(&mut self.state), &states).show(ui);
            Segmented::new(words.kind.clone(), Bind::bound(&mut self.kind), &kinds).show(ui);
        });
        let _ = TextField::search(words.search.clone(), Bind::bound(&mut self.query)).limit(MAX_QUERY_CHARS).show(ui);
        if self.state != state
            && let Some(value) = STATES.get(self.state)
        {
            self.post_shell("pluginManager.filterByInstallState", json!({ "state": value }));
        }
        if self.kind != kind
            && let Some(value) = KINDS.get(self.kind)
        {
            self.post_shell("pluginManager.filterByType", json!({ "type": value }));
        }
        if self.query != query {
            self.posted_query = Some(self.query.clone());
            self.post_shell("pluginManager.search", json!({ "query": self.query }));
        }
    }

    /// The list of extensions; a click selects one row.
    fn list(&mut self, ui: &mut Ui, words: &ManagerWords) {
        let entries: &[ManagerEntry] = &self.view.entries;
        let at = self.selected.as_ref().and_then(|id| entries.iter().position(|e| e.id == *id));
        let mut selection = Selection::new(at.into_iter().collect(), at, at);
        let columns = vec![
            ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::Content),
            ColumnSpec::new(KIND, words.kind.clone(), ColumnWidth::Content),
            ColumnSpec::new(VERSION, words.version.clone(), ColumnWidth::Content),
            ColumnSpec::new(STATUS, words.status.clone(), ColumnWidth::Content),
            ColumnSpec::new(PROBLEM, words.problem.clone(), ColumnWidth::Content),
        ];
        let positions: Vec<usize> = (0..entries.len()).collect();
        let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(entries.get(*at).map_or_else(String::new, |e| e.title.clone())));
        let _ = Table::<usize>::new(words.list.clone(), columns)
            .max_rows(LIST_ROWS)
            .select(SelectMode::Single)
            .selection(Bind::bound(&mut selection))
            .empty(words.none.clone())
            .show_items(ui, &source, &mut |at, icx| {
                let Some(e) = entries.get(*at) else { return ListRow::new("") };
                let title = if e.title.is_empty() { e.id.clone() } else { e.title.clone() };
                ListRow::new(title)
                    .cells(icx.cells())
                    .cell(KIND, e.kind.clone())
                    .cell(VERSION, e.version.clone())
                    .cell(STATUS, status(e, words))
                    .cell(PROBLEM, e.error.clone().unwrap_or_default())
            });
        let picked = selection.items.first().and_then(|at| entries.get(*at)).map(|e| e.id.clone());
        if picked.is_some() || selection.items.is_empty() {
            self.selected = picked;
        }
    }

    /// The selected row's description and its buttons.
    fn actions(&mut self, ui: &mut Ui, words: &ManagerWords) {
        let Some(entry) = self.selected.as_ref().and_then(|id| self.view.entries.iter().find(|e| e.id == *id)).cloned() else {
            Text::new(words.select_hint.clone()).shade(Shade::Muted).show(ui);
            return;
        };
        if !entry.vendor.is_empty() {
            Text::new(entry.vendor.clone()).shade(Shade::Muted).show(ui);
        }
        if !entry.description.is_empty() {
            Text::new(entry.description.clone()).show(ui);
        }
        if let Some(error) = &entry.error {
            Text::new(error.clone()).show(ui);
        }
        let id = entry.id.clone();
        Row::new().show(ui, |ui, _| {
            if entry.from_index {
                let website = Button::new(words.website.clone()).tooltip(words.website_tip.clone()).id_salt("fontcraftExtensionManager.website");
                if website.show(ui).clicked() {
                    self.post_shell("pluginManager.openWebsite", json!({ "id": id }));
                }
                let donate = Button::new(words.donate.clone()).tooltip(words.donate_tip.clone()).id_salt("fontcraftExtensionManager.donate");
                if donate.show(ui).clicked() {
                    self.post_shell("pluginManager.openDonationPage", json!({ "id": id }));
                }
            }
            if entry.from_index
                && !entry.installed
                && Button::new(words.install.clone()).id_salt("fontcraftExtensionManager.install").show(ui).clicked()
            {
                self.posted.push(Msg::run("pluginManager.install", json!({ "id": id })));
            }
            if entry.from_index && entry.update && Button::new(words.update.clone()).id_salt("fontcraftExtensionManager.update").show(ui).clicked() {
                self.posted.push(Msg::run("pluginManager.install", json!({ "id": id, "version": entry.version })));
            }
            if entry.installed && Button::new(words.remove.clone()).id_salt("fontcraftExtensionManager.remove").show(ui).clicked() {
                self.posted.push(Msg::run("pluginManager.remove", json!({ "id": id })));
            }
            if let Some(on) = entry.enabled {
                let check = Checkbox::new(Bind::given(on)).label(words.switch.clone()).tooltip(words.switch_tip.clone());
                if check.show(ui).events.committed().is_some() {
                    self.posted.push(Msg::run("plugin.setEnabled", json!({ "id": id, "enabled": !on })));
                }
            }
        });
    }

    fn body(&mut self, ui: &mut Ui, words: &ManagerWords) {
        self.filters(ui, words);
        if self.view.safe_mode {
            Text::new(words.safe_mode.clone()).shade(Shade::Muted).show(ui);
        }
        self.list(ui, words);
        self.actions(ui, words);
        if !self.view.failures.is_empty() {
            Text::new(words.failed.clone()).show(ui);
            for (file, reason) in &self.view.failures {
                Text::new(format!("{file}: {reason}")).shade(Shade::Muted).show(ui);
            }
        }
        if let Some(note) = &self.view.note {
            Text::new(note.clone()).show(ui);
        }
    }
}

/// What the State column says about `e`.
fn status(e: &ManagerEntry, words: &ManagerWords) -> Label {
    if !e.installed {
        return words.not_installed.clone();
    }
    if e.update {
        let installed = e.installed_version.clone().unwrap_or_default();
        return Label::from(format!("{} {installed}", words.update_to.as_str()));
    }
    match e.enabled {
        Some(true) => words.on.clone(),
        Some(false) if e.new => words.new_module.clone(),
        Some(false) => words.off.clone(),
        None => words.installed.clone(),
    }
}

impl StackDialog for ManagerDialog {
    /// The messages still to post when the window closes.
    type Result = Vec<Msg>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Vec<Msg>>> {
        if self.closing {
            return Some(DialogResult::Ok(Vec::new()));
        }
        let words = Env::of(ui).words::<ManagerWords>();
        let mut value = false;
        let mut open = self.open;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut value))
            .opened(Bind::bound(&mut open))
            .ok_label(words.install_file.clone())
            .cancel_label(words.close.clone())
            .accept_stays_open(true)
            .enter_confirms(false)
            .extra_buttons([FooterButton::new(words.reload.clone(), DialogRole::Other)])
            .show(ui, |ui, _, _| {
                self.body(ui, &words);
                true
            });
        self.open = open;
        if shown.picked == Some(FIRST_EXTRA) {
            self.posted.push(Msg::run("pluginManager.reloadIndexes", json!({})));
        } else if shown.picked == Some(OK) || shown.accepted.is_some() {
            self.posted.push(Msg::run("plugin.installFromFile", json!({})));
        } else if shown.closed.is_some() {
            return Some(DialogResult::Ok(std::mem::take(&mut self.posted)));
        }
        None
    }
}

/// Opens the window while the app's state asks for it, shows each change of the list, and posts what the person did.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    extension_manager::refresh(app);
    if let Some(answer) = host.take::<ManagerDialog>() {
        if let DialogResult::Ok(posted) = answer {
            app.sent.extend(posted);
        }
        app.view.extensions.manager.open = false;
        app.view_changed();
        return;
    }
    let view = &app.view.extensions.manager;
    match host.get_mut::<ManagerDialog>() {
        Some(dialog) => {
            let posted = std::mem::take(&mut dialog.posted);
            if !view.open {
                dialog.closing = true;
            } else if dialog.view.revision != view.revision {
                dialog.take(view);
            }
            // Reload and Install can change what the indexes offer, which the extension fingerprint does not cover.
            let changes_list =
                posted.iter().any(|m| matches!(m, Msg::Run(c) if c.id.starts_with("pluginManager.") || c.id == "plugin.installFromFile"));
            app.sent.extend(posted);
            if changes_list {
                app.view.extensions.manager.invalidate();
            }
        }
        None if view.open => {
            host.open(ManagerDialog::new(view));
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;

    fn entry(id: &str, enabled: Option<bool>, from_index: bool, installed: bool) -> ManagerEntry {
        ManagerEntry {
            id: id.into(),
            kind: "filter".into(),
            title: format!("{id} title"),
            version: "1".into(),
            installed,
            enabled,
            from_index,
            ..Default::default()
        }
    }

    /// Selecting a row shows its buttons: the switch turns an installed module off, Remove removes it, Install
    /// installs an index entry; Install from File… asks the picker and a filter button posts its command.
    #[test]
    fn extension_manager_acts_on_the_selected_row() {
        let mut view = ManagerView::default();
        view.open = true;
        view.entries = vec![entry("square", Some(true), false, true), entry("tighten", None, true, false)];
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut dialog = ManagerDialog::new(&view);
        let mut fixture = Fixture::new(move |ui| {
            let words = Env::of(ui).words::<ManagerWords>();
            dialog.body(ui, &words);
            seen.borrow_mut().extend(std::mem::take(&mut dialog.posted));
        });
        fixture.room(egui::vec2(900.0, 700.0)).settle();
        let row = fixture.by_label("square title").expect("the square row");
        fixture.click(&row).settle();
        let switch = fixture.by_label("Turned on").expect("the switch of the selected module");
        fixture.click(&switch).settle();
        let remove = fixture.by_label("Remove").expect("Remove for an installed module");
        fixture.click(&remove).settle();
        assert!(fixture.by_label("Install").is_none(), "an installed module has no Install button");
        let row = fixture.by_label("tighten title").expect("the index entry's row");
        fixture.click(&row).settle();
        let install = fixture.by_label("Install").expect("Install for an index entry");
        fixture.click(&install).settle();
        let reporters = fixture.by_label("Reporters").expect("the Reporters filter");
        fixture.click(&reporters).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("plugin.setEnabled", json!({"id": "square", "enabled": false})),
                Msg::run("pluginManager.remove", json!({"id": "square"})),
                Msg::run("pluginManager.install", json!({"id": "tighten"})),
                Msg::Shell(CommandRef::with_params("pluginManager.filterByType", json!({"type": "reporter"}))),
            ]
        );
    }
}

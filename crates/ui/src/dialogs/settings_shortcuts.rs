//! Settings › Shortcuts and the Keyboard Shortcuts dialog it opens (`settings.editShortcuts`).
//!
//! The dialog is craft's `ShortcutsDialog` over a keymap of the catalog: it lists the commands by menu and area, finds
//! them by text, records, removes and resets shortcuts and asks about conflicts. Its Show list holds one entry per menu,
//! Tools, All Menus and Commands (what the open document and the selection allowed when the dialog opened); its filter
//! holds Changed, the reference's Customized. The dialog edits its own keymap; after each edit, and after a Cancel that
//! took edits back, the whole set goes through `settings.importShortcuts`, so the engine's rules apply and the settings
//! stay the one record. What the engine left out is taken out of the dialog's keymap again.
//!
//! The tools' letters, the keys of one area and the numbered keys are listed as fixed rows. Rows: `settings.shortcuts`,
//! `shortcuts.scopeFilters`, `shortcuts.sections`, `shortcuts.search`, `settings.recordShortcut`,
//! `settings.cancelShortcutRecording`.

use std::cell::RefCell;
use std::collections::BTreeSet;

use craft::keymap::keymap::Overrides;
use craft::keymap::{Binding, CommandDef, Keymap, Scopes};
use craft::menu::shortcuts::{Area, DialogState, FixedRow, Options, Outcome, ShortcutsDialog};
use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::style::TypeRole;
use egui::Ui;
use fontcraft_engine::shortcuts::{Chord, system};
use serde_json::{Value, json};

use crate::app::catalog::{Kind, definitions, kind};
use crate::app::{FontCraftApp, Msg, shortcuts};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::{getters, params};

/// The command that opens the dialog.
pub const EDIT: &str = "settings.editShortcuts";
/// The command the dialog's edits go through.
const IMPORT: &str = "settings.importShortcuts";

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: EDIT,
    label: "Edit Shortcuts…",
    menu: &[],
    shortcut: None,
    params: "{open?: bool (default true), command?: id} open or close the Keyboard Shortcuts dialog, on a command when one is named. ui.inspect reports it as settings.shortcutsOpen",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run: run_command };

/// What the window keeps of the dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcutsState {
    pub open: bool,
    /// The command the dialog opens on.
    pub command: Option<String>,
}

craft::ui::words! {
    /// The words of the Shortcuts section.
    pub struct ShortcutSettingsWords {
        edit: "fontcraftShortcuts.edit" = "Edit Shortcuts…";
        reset: "fontcraftShortcuts.reset" = "Reset All";
        import: "fontcraftShortcuts.import" = "Import…";
        export: "fontcraftShortcuts.export" = "Export…";
        none: "fontcraftShortcuts.none" = "Every command has its built-in shortcut.";
        one: "fontcraftShortcuts.one" = "One command has a shortcut of your own.";
        note: "fontcraftShortcuts.note" = "Click a command's shortcut in the list, or select a command without one, and press the keys. A shortcut needs Cmd, Ctrl, Alt or Shift.";
        /// Why a key cannot be given to a command.
        fixed: "fontcraftShortcuts.fixed" = "This key cannot be changed.";
    }
}

/// The section: how many shortcuts are customized, and the buttons.
pub fn section(ui: &mut Ui, customized: usize, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ShortcutSettingsWords>();
    let count = match customized {
        0 => words.none.as_str().to_string(),
        1 => words.one.as_str().to_string(),
        n => format!("{n} commands have a shortcut of your own."),
    };
    Text::new(count).show(ui);
    let _ = Row::new().show(ui, |ui, _| {
        if Button::new(words.edit.clone()).show(ui).clicked() {
            sent.push(Msg::Shell(CommandRef::new(EDIT)));
        }
        if Button::new(words.reset.clone()).enabled(customized > 0).show(ui).clicked() {
            sent.push(Msg::run("settings.resetShortcut", json!({})));
        }
        if Button::new(words.import.clone()).show(ui).clicked() {
            sent.push(Msg::run(IMPORT, json!({})));
        }
        if Button::new(words.export.clone()).enabled(customized > 0).show(ui).clicked() {
            sent.push(Msg::run("settings.exportShortcuts", json!({})));
        }
    });
    Text::new(words.note.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

fn run_command(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    let open = params::flag(p, "open")?.unwrap_or(true);
    let named = params::string(p, "command")?.map(str::to_string);
    if let Some(id) = &named
        && !definitions().iter().any(|d| d.command.id == *id)
    {
        return Err(format!("there is no command `{}`", params::quoted(id)));
    }
    app.view.settings.shortcuts = ShortcutsState { open, command: named };
    Ok(())
}

thread_local! {
    /// The commands the Commands entry lists; craft asks by a plain function.
    static ALLOWED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Whether the dialog lists `def`: a command of the engine that does something, or one of the window's, and no tool or
/// numbered key.
fn listed(def: &CommandDef) -> bool {
    let id = def.command.id.as_str();
    match kind(id) {
        Kind::Tool(_) => false,
        Kind::Shell => getters::find(id).is_none(),
        Kind::Engine => fontcraft_engine::find_command(id).is_some_and(|spec| spec.journal),
    }
}

fn in_menus(def: &CommandDef) -> bool {
    def.paths.iter().any(|path| !path.is_empty())
}

fn allowed(def: &CommandDef) -> bool {
    ALLOWED.with(|ids| ids.try_borrow().is_ok_and(|ids| ids.contains(&def.command.id)))
}

/// Whether the engine would take `binding` for `def`; the message says why not.
fn accepts(def: &CommandDef, binding: &Binding) -> Result<(), String> {
    if !listed(def) {
        return Err(ShortcutSettingsWords::english().fixed.as_str().to_string());
    }
    let chord = Chord::custom(&binding.trigger.to_string())?;
    match system(&chord) {
        Some(found) if found.blocked => Err(found.reason.to_string()),
        _ => Ok(()),
    }
}

/// The dialog's rows for keys that stay where they are.
fn fixed_rows() -> Vec<FixedRow> {
    let fixed = shortcuts::window_commands().into_iter().filter(|c| c.fixed);
    fixed
        .map(|c| {
            let keys = c.shortcut.unwrap_or_default();
            let row = FixedRow::new(c.section, c.label, keys.clone());
            match Binding::parse(&keys) {
                Ok(binding) => row.chord(binding.trigger),
                Err(_) => row,
            }
        })
        .collect()
}

fn options() -> Options {
    Options::default().with(|o| {
        o.allow_keep = false;
        o.summary = false;
        o.areas = vec![Area::new("menus", "All Menus", in_menus), Area::new("commands", "Commands", allowed)];
        o.visible = Some(listed);
        o.fixed = fixed_rows();
        o.accept = Some(Box::new(accepts));
    })
}

/// The overrides as `settings.importShortcuts` takes them: one shortcut per command, the one added last.
fn as_list(overrides: &Overrides) -> Value {
    let entry = |(command, keys): (&str, &[String])| json!({"command": command, "shortcut": keys.last().map_or("", String::as_str)});
    Value::Array(overrides.iter().map(entry).collect())
}

/// The Keyboard Shortcuts dialog on the dialog host.
pub struct ShortcutsEditor {
    id: DialogId,
    /// What the settings hold, as the app copied it last.
    pub stored: Overrides,
    /// Built on the first pass, when the platform is known.
    keymap: Option<Keymap>,
    state: DialogState,
    outbox: Vec<Msg>,
    /// An edit was posted and the settings have not answered it yet.
    waiting: bool,
    close: bool,
}

impl ShortcutsEditor {
    fn new(stored: Overrides, command: Option<&str>) -> ShortcutsEditor {
        let mut state = DialogState::new();
        if let Some(id) = command {
            state.select(&CommandRef::new(id));
        }
        ShortcutsEditor { id: dialog_id("fontcraft.shortcuts"), stored, keymap: None, state, outbox: Vec::new(), waiting: false, close: false }
    }

    /// Takes what the settings hold. An edit the engine changed or refused shows as the settings have it.
    fn take_stored(&mut self, stored: Overrides) {
        let answered = std::mem::take(&mut self.waiting);
        if !answered && stored == self.stored {
            return;
        }
        if let Some(keymap) = &mut self.keymap
            && as_list(keymap.overrides()) != as_list(&stored)
        {
            keymap.set_overrides(stored.clone());
        }
        self.stored = stored;
    }
}

impl StackDialog for ShortcutsEditor {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.close {
            return Some(DialogResult::Cancel);
        }
        let platform = craft::keymap::egui::platform(ui.ctx());
        let stored = &self.stored;
        let keymap = self.keymap.get_or_insert_with(|| Keymap::new(definitions(), Scopes::new(), platform).with_overrides(stored.clone()));
        // Made per pass: the host's dialogs are `Send`, and the options hold a closure that is not.
        let options = options();
        let shown = ShortcutsDialog::new(keymap).options(&options).show(ui, &mut self.state);
        if matches!(shown.outcome, Some(Outcome::Changed | Outcome::Ok)) && as_list(keymap.overrides()) != as_list(&self.stored) {
            self.outbox.push(Msg::run(IMPORT, json!({ "shortcuts": as_list(keymap.overrides()) })));
            self.waiting = true;
        }
        (!self.state.is_open()).then_some(DialogResult::Cancel)
    }
}

/// Opens the dialog while the app asks for it, posts its edits, and closes it when the app closed it.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<ShortcutsEditor>().is_some() {
        app.view.settings.shortcuts = ShortcutsState::default();
        return;
    }
    let wanted = app.view.settings.shortcuts.clone();
    if let Some(dialog) = host.get_mut::<ShortcutsEditor>() {
        dialog.close = !wanted.open;
        if !dialog.outbox.is_empty() {
            app.sent.append(&mut dialog.outbox);
        } else if !app.sent.iter().any(|msg| msg.command_id() == Some(IMPORT)) {
            // An edit still waiting for `drain` is not in the settings yet.
            dialog.take_stored(shortcuts::overrides(app.catalog.custom()));
        }
    } else if wanted.open {
        let ids = app.catalog.allowed();
        ALLOWED.with(|kept| {
            if let Ok(mut kept) = kept.try_borrow_mut() {
                *kept = ids;
            }
        });
        host.open(ShortcutsEditor::new(shortcuts::overrides(app.catalog.custom()), wanted.command.as_deref()));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::keymap::CommandDef;
    use craft::ui::testing::Fixture;

    use super::*;
    use crate::testing::TestApp;

    fn def(id: &str) -> CommandDef {
        definitions().into_iter().find(|d| d.command.id == id).unwrap_or_else(|| panic!("{id}"))
    }

    #[test]
    fn the_section_opens_the_dialog_and_resets() {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            section(ui, 2, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        for label in ["Edit Shortcuts…", "Reset All"] {
            let button = fixture.by_label(label).expect(label);
            fixture.click(&button).settle();
        }
        assert_eq!(*posted.borrow(), vec![Msg::Shell(CommandRef::new(EDIT)), Msg::run("settings.resetShortcut", json!({}))]);
    }

    /// The dialog opens over the Settings window and lists the commands; a shortcut set meanwhile shows in it; it closes
    /// when the app closes it.
    #[test]
    fn the_dialog_opens_and_closes_with_the_window_state() {
        let mut t = TestApp::new();
        t.ok("engine.execute", json!({"command": EDIT, "params": {}}));
        t.frames(3);
        assert!(t.frames.texts().iter().any(|x| x.text == "Keyboard Shortcuts"), "the dialog shows");
        t.ok("engine.execute", json!({"command": "settings.setShortcut", "params": {"command": "view.newTab", "shortcut": "Cmd+Alt+Shift+Y"}}));
        t.frames(3);
        t.ok("engine.execute", json!({"command": EDIT, "params": {"open": false}}));
        t.frames(3);
        assert!(!t.frames.texts().iter().any(|x| x.text == "Keyboard Shortcuts"));
        assert!(!t.app.view.settings.shortcuts.open);
        let custom = t.app.catalog.custom();
        assert_eq!(custom.shortcut("view.newTab", None).as_deref(), Some("Cmd+Alt+Shift+Y"), "closing it from outside undoes nothing");
        assert!(t.error("engine.execute", json!({"command": EDIT, "params": {"command": "no.such"}})).contains("no.such"));
    }

    /// Tools, numbered keys and queries are not listed; a key without a modifier and the system's are refused; the set
    /// goes to the engine with one shortcut per command.
    #[test]
    fn the_dialog_follows_the_engines_rules() {
        let binding = |text: &str| Binding::parse(text).unwrap();
        assert!(listed(&def("edit.undo")) && listed(&def("view.newTab")) && !listed(&def("tool.draw")) && !listed(&def("settings.get")));
        assert!(accepts(&def("edit.undo"), &binding("Cmd+Alt+Shift+U")).is_ok());
        assert!(accepts(&def("edit.undo"), &binding("U")).is_err());
        assert!(accepts(&def("tool.draw"), &binding("Cmd+Alt+D")).is_err());
        assert!(in_menus(&def("edit.undo")) && !fixed_rows().is_empty());
        let mut overrides = Overrides::new();
        overrides.insert("edit.undo", vec!["Cmd+Alt+U".into(), "Cmd+Alt+I".into()]);
        overrides.insert("edit.redo", Vec::new());
        assert_eq!(as_list(&overrides), json!([{"command": "edit.redo", "shortcut": ""}, {"command": "edit.undo", "shortcut": "Cmd+Alt+I"}]));
    }
}

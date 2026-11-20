//! The extension commands of the window: reporter switches (`view.showReporter`) and the External Changes dialog
//! (`dialog.externalChanges`, [`super::external_changes`]). The Extension Manager's state is kept here; its commands are
//! [`super::extension_manager`] and its window `dialogs::extension_manager`. Module and macro menu rows are built in
//! `app::menus_dynamic`; dropped module files are installed by `app::dropped`.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::plugins::Kind;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use super::external_changes::{self, ExternalChanges};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most reporter switches kept. Switches of modules removed since are dropped first when the list is full.
pub const MAX_SHOWN_REPORTERS: usize = 256;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "view.showReporter",
        label: "Show (reporter name)",
        menu: &[],
        shortcut: None,
        params: "{id: reporter module id, on?: bool (omitted: toggle)} show or hide an installed reporter module's drawing in Edit View; the View menu lists one Show row per reporter module. The switch is view state",
    },
    external_changes::COMMAND,
];

/// The extension windows' view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtensionsView {
    /// The reporter modules Edit View draws, by module id, in the order they were switched on.
    pub reporters: Vec<String>,
    /// The External Changes dialog.
    pub external: ExternalChanges,
    /// The Extension Manager.
    pub manager: super::extension_manager::ManagerView,
    /// The last error of each shown reporter's run, by module id; the canvas writes it at its top left.
    pub reporter_errors: std::collections::BTreeMap<String, String>,
}

/// Whether the module `id` is an installed reporter.
fn is_reporter(session: &Session, id: &str) -> bool {
    session.extensions().registry.get(id).is_some_and(|m| m.plugin.kind() == Kind::Reporter)
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "view.showReporter" => show_reporter(app, &command.params),
        "dialog.externalChanges" => external_changes::run(app, &command.params),
        other => Err(format!("no extension command `{other}`")),
    }
}

/// Switches the reporter `id` on or off (`on` omitted: the other way).
fn show_reporter(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let id = params.get("id").and_then(Value::as_str).ok_or("give `id`: a reporter module's id")?;
    let was = app.view.extensions.reporters.iter().any(|s| s == id);
    let on = match params.get("on") {
        None | Some(Value::Null) => !was,
        Some(Value::Bool(on)) => *on,
        Some(_) => return Err("`on` is true or false".into()),
    };
    if on && !is_reporter(&app.session, id) {
        return Err(match app.session.extensions().registry.get(id) {
            Some(m) => format!("module {id:?} is a {} module, not a reporter", m.plugin.kind().name()),
            None => format!("no module {id:?} is installed"),
        });
    }
    let mut list = app.view.extensions.reporters.clone();
    list.retain(|s| s != id);
    if on {
        if list.len() >= MAX_SHOWN_REPORTERS {
            list.retain(|s| is_reporter(&app.session, s));
        }
        if list.len() >= MAX_SHOWN_REPORTERS {
            return Err(format!("at most {MAX_SHOWN_REPORTERS} reporters can be shown"));
        }
        list.push(id.to_string());
    }
    app.view.extensions.reporters = list;
    Ok(())
}

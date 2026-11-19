//! Window commands: which panels show.

use craft::ui::runtime::commands::CommandRef;

use super::commands::{Module, UiCommand};
use super::dock::{self, PanelId};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.togglePanel",
        label: "Show or Hide Panel",
        menu: &[],
        shortcut: None,
        params: "{panel: string} show the panel with this id, or hide it when it shows (ids: `PanelId::id`)",
    },
    UiCommand {
        id: "window.resetLayout",
        label: "Reset Layout",
        menu: &["Window"],
        shortcut: None,
        params: "{} put every panel back where it starts",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "window.togglePanel" => {
            let id = command.params.get("panel").and_then(|p| p.as_str()).ok_or("`panel` is required")?;
            let panel = PanelId::from_id(id).ok_or_else(|| format!("no panel `{id}`"))?;
            dock::toggle(&mut app.view.workspace, panel);
            Ok(())
        }
        "window.resetLayout" => {
            app.view.workspace = dock::default_workspace();
            Ok(())
        }
        other => Err(format!("no window command `{other}`")),
    }
}

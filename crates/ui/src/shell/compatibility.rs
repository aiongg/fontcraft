//! Show Master Compatibility: the View menu toggle that draws every source layer of the edited glyph beside it
//! (`canvas::compatibility`).

use craft::ui::runtime::commands::CommandRef;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "view.showMasterCompatibility",
    label: "Show Master Compatibility",
    menu: &["View"],
    shortcut: Some("Cmd+Ctrl+Alt+N"),
    params: "{on?: bool} draw every master, intermediate and alternate layer of the edited glyph beside it: shapes tinted by their index, lines joining start nodes, component centres, anchors and selected points across the layers, segments green (compatible), yellow (turning more than 20°) or red (incompatible) (omitted: toggle)",
}];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let display = &mut app.view.display;
    let on = match command.params.get("on") {
        None | Some(Value::Null) => !display.master_compatibility,
        Some(Value::Bool(on)) => *on,
        Some(other) => return Err(format!("{}: `on` is true or false, not {other}", command.id)),
    };
    display.master_compatibility = on;
    Ok(())
}

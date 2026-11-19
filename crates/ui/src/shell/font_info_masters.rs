//! The Masters tab's view state and the shell command that changes it.
//!
//! | Command | Does |
//! |---|---|
//! | `font.info.masters.select {masters: [index]}` | selects masters of the Masters tab's list; the sections at the right edit every selected master |
//!
//! The selection is view state. The projection drops indices the font lacks and falls back to the first master, so a
//! stale selection never names a master that is gone.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "font.info.masters.select",
    label: "Select Masters",
    menu: &[],
    shortcut: None,
    params: "{masters: [index]} select masters in the Masters tab's list; the sections at the right edit every selected master. An empty list selects the first master",
}];

/// Most masters one selection names.
const MAX_SELECTED: usize = 4096;

/// The Masters tab's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MastersTabView {
    /// The selected masters, sorted; empty selects the first.
    pub selected: Vec<usize>,
}

/// The command that selects `masters`.
pub fn select_masters(masters: &[usize]) -> CommandRef {
    CommandRef::with_params("font.info.masters.select", json!({ "masters": masters }))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "font.info.masters.select" => {
            let list = command.params.get("masters").and_then(Value::as_array).ok_or("`masters` is required")?;
            let mut selected: Vec<usize> = list
                .iter()
                .take(MAX_SELECTED)
                .map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`masters` holds indices"))
                .collect::<Result<_, _>>()?;
            selected.sort_unstable();
            selected.dedup();
            app.view.font_info.masters.selected = selected;
            Ok(())
        }
        other => Err(format!("no Masters tab command `{other}`")),
    }
}

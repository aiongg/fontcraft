//! The Script menu's commands over macros (`crate::macros`) (M13.7 fills it). Rows: `script.run`,
//! `script.rerunLast`, `script.openScriptsFolder`, `script.reloadScripts`, `script.runActionSet`,
//! `script.saveActionSet`, `script.repeatLastAction`, `script.recordMacro`. `script.runActionSet`
//! and `script.repeatLastAction` are aliases of the macro commands (decision D8).

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.7.
    Vec::new()
}

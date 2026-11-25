//! The Masters tab's view state and the shell command that changes it.
//!
//! | Command | Does |
//! |---|---|
//! | `font.info.masters.select {masters: [index]}` | selects masters of the Masters tab's list; the sections at the right edit every selected master |
//! | `font.info.masters.setMapAxes {across?: tag, up?: tag}` | the axes the Masters tab's designspace map shows |
//!
//! The selection is view state. The projection drops indices the font lacks and falls back to the first master, so a
//! stale selection never names a master that is gone.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "font.info.masters.select",
        label: "Select Masters",
        menu: &[],
        shortcut: None,
        params: "{masters: [index]} select masters in the Masters tab's list; the sections at the right edit every selected master. An empty list selects the first master",
    },
    UiCommand {
        id: "font.info.masters.setMapAxes",
        label: "Designspace Map Axes",
        menu: &[],
        shortcut: None,
        params: "{across?: tag, up?: tag} the axes the designspace map of the Masters tab shows; one left out stays, and the axis the other takes trades places with it",
    },
];

/// Most masters one selection names.
const MAX_SELECTED: usize = 4096;

/// The Masters tab's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MastersTabView {
    /// The selected masters, sorted; empty selects the first.
    pub selected: Vec<usize>,
    /// The axes the designspace map shows across and up, as indices; `None` shows the first two.
    pub map_axes: Option<(usize, usize)>,
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
        "font.info.masters.setMapAxes" => {
            let font = app.session.font().map_err(|e| e.to_string())?;
            let axis = |key: &str| -> Result<Option<usize>, String> {
                let Some(tag) = command.params.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
                let tag = tag.as_str().ok_or_else(|| format!("`{key}` is an axis tag"))?;
                font.axes.iter().position(|a| a.tag == tag).map(Some).ok_or_else(|| format!("`{key}`: the font has no axis `{tag}`"))
            };
            let (across, up) = (axis("across")?, axis("up")?);
            let shown = app.view.font_info.masters.map_axes.unwrap_or((0, 1));
            app.view.font_info.masters.map_axes = Some(match (across, up) {
                (Some(a), Some(b)) => (a, b),
                (Some(a), None) => (a, if a == shown.1 { shown.0 } else { shown.1 }),
                (None, Some(b)) => (if b == shown.0 { shown.1 } else { shown.0 }, b),
                (None, None) => shown,
            });
            Ok(())
        }
        other => Err(format!("no Masters tab command `{other}`")),
    }
}

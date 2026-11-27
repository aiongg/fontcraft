//! Compare Fonts in List (`font.compareInTable`): one row per glyph and one column per chosen open font, showing one
//! property (the advance width, a sidebearing, a count…) read from each font's first master, so the fonts of a family
//! can be read across a row. This module holds the panel's view state and its command; the table is the projection's
//! (`app::models::compare_list`), built with `fontcraft_engine::cmd::encodings::compare_table` again whenever a
//! compared document changes or the command runs. This is not the two-document Compare Fonts panel.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::cmd::encodings::{COMPARE_PROPERTIES, resolve_documents};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, quoted, string};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "font.compareInTable",
    label: "Compare Fonts in List",
    menu: &["Window"],
    shortcut: None,
    params: "{fonts?: [fontRef] (positions in the open-document list, titles or paths; default every open document), property?: \"width\" | \"lsb\" | \"rsb\" | \"nodes\" | \"paths\" | \"components\" | \"anchors\" | \"unicodes\" (default the last one shown, else width), onlyDifferences?: bool (default the last choice, else false)} shows a table with one row per glyph and one column per font, showing the property in each font's first master; rows whose values differ are marked, and onlyDifferences hides the others. ui.inspect reports it under compareInList",
}];

/// The panel's state, in `ViewState::compare_in_list`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompareInListState {
    /// The compared documents, by session id, in column order.
    pub docs: Vec<u64>,
    /// A [`COMPARE_PROPERTIES`] id.
    pub property: String,
    pub only_differences: bool,
}

impl Default for CompareInListState {
    fn default() -> Self {
        Self { docs: Vec::new(), property: "width".into(), only_differences: false }
    }
}

/// Whether the Compare Fonts in List panel shows.
pub fn shows(app: &FontCraftApp) -> bool {
    app.view.workspace.layout().panel_state(&PanelId::CompareInList) != craft::behavior::dock::PanelState::Hidden
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if command.id != "font.compareInTable" {
        return Err(format!("no Compare Fonts in List command `{}`", command.id));
    }
    let p = &command.params;
    let docs = resolve_documents(&app.session, p.get("fonts"))?;
    if docs.is_empty() {
        return Err("no document is open".into());
    }
    let property = match string(p, "property")? {
        Some(id) if COMPARE_PROPERTIES.iter().any(|(known, _)| *known == id) => id.to_string(),
        Some(id) => {
            let ids: Vec<&str> = COMPARE_PROPERTIES.iter().map(|(known, _)| *known).collect();
            return Err(format!("`{}` is not a property to compare; use one of {}", quoted(id), ids.join(", ")));
        }
        None => app.view.compare_in_list.property.clone(),
    };
    let only = flag(p, "onlyDifferences")?.unwrap_or(app.view.compare_in_list.only_differences);
    // A refused move (the layout has no room) leaves the panel where it was.
    let _ = app.view.workspace.set_visible(PanelId::CompareInList, true, None, false, |panel| panel.permissions());
    app.view.compare_in_list = CompareInListState { docs, property, only_differences: only };
    Ok(())
}

/// The panel as `ui.inspect` reports it, under `compareInList`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let st = &app.view.compare_in_list;
    let model = &app.models.compare_list;
    let rows: Vec<Value> = model.rows.iter().take(INSPECT_ROWS).map(|r| json!({"glyph": r.glyph, "values": r.values, "differ": r.differ})).collect();
    state.insert(
        "compareInList".into(),
        json!({
            "open": shows(app),
            "fonts": model.fonts,
            "property": st.property,
            "onlyDifferences": st.only_differences,
            "total": model.total,
            "rows": rows,
            "error": model.error,
        }),
    );
}

/// Rows `ui.inspect` lists; `total` counts them all.
const INSPECT_ROWS: usize = 200;

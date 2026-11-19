//! Glyph Info (`window.glyphInfo`): the glyph data as a table, searched and sorted by `glyphData.search`. The panel
//! shows it (`panels::glyph_info`); this module keeps the search field, the sorted column and the selected rows.

use craft::behavior::dock::PanelState;
use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, quoted, string, strings};
use crate::app::FontCraftApp;

/// Longest query, in characters: four bytes each stay within the search's byte limit.
pub const MAX_QUERY_CHARS: usize = 64;
/// Most rows one search shows (the search's own limit).
pub const MAX_ROWS: usize = 10_000;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "window.glyphInfo",
    label: "Glyph Info",
    menu: &["Window"],
    shortcut: None,
    params: "{open?: bool (default true), query?: string, sortBy?: \"name\" | \"unicode\" | \"category\" | \"subCategory\" | \"script\" | \"description\", select?: [names]} show or hide the Glyph Info panel: the glyph data as a table, searched and sorted as glyphData.search does. query and sortBy set the search field and the sorted column; select sets the selected rows, which must be among the results",
}];

/// A column of the table. Its id is the `sortBy` value and the result field it shows.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Column {
    #[default]
    Name,
    Unicode,
    Category,
    SubCategory,
    Script,
    Description,
}

impl Column {
    pub const ALL: [Column; 6] = [Column::Name, Column::Unicode, Column::Category, Column::SubCategory, Column::Script, Column::Description];

    pub fn id(self) -> &'static str {
        match self {
            Column::Name => "name",
            Column::Unicode => "unicode",
            Column::Category => "category",
            Column::SubCategory => "subCategory",
            Column::Script => "script",
            Column::Description => "description",
        }
    }

    pub fn of_id(id: &str) -> Option<Column> {
        Column::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// The panel's view state: the search, the sort and the selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphInfoState {
    pub query: String,
    /// The sorted column; `None`: by name.
    pub sort_by: Option<Column>,
    /// Selected rows by name, in the order they were picked.
    pub selected: Vec<String>,
}

/// The parameters of `glyphData.search` for this query and sort.
pub fn search_params(query: &str, sort_by: Option<Column>) -> Value {
    let mut p = Map::new();
    p.insert("query".into(), json!(query));
    p.insert("limit".into(), json!(MAX_ROWS));
    if let Some(c) = sort_by {
        p.insert("sortBy".into(), json!(c.id()));
    }
    Value::Object(p)
}

/// The names `glyphData.search` finds for `query` and `sort_by`, in its order.
pub fn search_names(session: &mut Session, query: &str, sort_by: Option<Column>) -> Result<Vec<String>, String> {
    let reply = session.query("glyphData.search", &search_params(query, sort_by)).map_err(|e| e.to_string())?;
    let results = reply.get("results").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    Ok(results.iter().filter_map(|r| r.get("name").and_then(Value::as_str)).map(str::to_string).collect())
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if command.id != "window.glyphInfo" {
        return Err(format!("no glyph info command `{}`", command.id));
    }
    let p = &command.params;
    let open = flag(p, "open")?.unwrap_or(true);
    let state = &app.view.glyph_info;
    let query = match string(p, "query")? {
        Some(q) if q.chars().count() > MAX_QUERY_CHARS => return Err(format!("`query` holds at most {MAX_QUERY_CHARS} characters")),
        Some(q) => q.to_string(),
        None => state.query.clone(),
    };
    let sort_by = match string(p, "sortBy")? {
        Some(s) => Some(
            Column::of_id(s)
                .ok_or_else(|| format!("`sortBy` must be name, unicode, category, subCategory, script or description, got \"{}\"", quoted(s)))?,
        ),
        None => state.sort_by,
    };
    let select = strings(p, "select", MAX_ROWS)?;
    // Everything is checked before anything changes.
    let found = if query != state.query || sort_by != state.sort_by || select.is_some() || !state.selected.is_empty() {
        Some(search_names(&mut app.session, &query, sort_by)?)
    } else {
        None
    };
    if let (Some(names), Some(found)) = (&select, &found)
        && let Some(bad) = names.iter().find(|n| !found.contains(n))
    {
        return Err(format!("`{}` is not among the search results", quoted(bad)));
    }
    let shown = app.view.workspace.layout.panel_state(&PanelId::GlyphInfo) != PanelState::Hidden;
    if shown != open {
        super::dock::toggle(&mut app.view.workspace, PanelId::GlyphInfo);
    }
    let state = &mut app.view.glyph_info;
    state.query = query;
    state.sort_by = sort_by;
    match select {
        Some(names) => state.selected = names,
        None => {
            if let Some(found) = &found {
                state.selected.retain(|n| found.contains(n));
            }
        }
    }
    Ok(())
}

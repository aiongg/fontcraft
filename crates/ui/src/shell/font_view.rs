//! Font View's shell commands: its mode, cell size, sort, columns, search and glyph selection, and opening the
//! selection in Edit View. View state only; font changes go through engine commands.

use std::collections::HashSet;

use craft::ui::runtime::commands::CommandRef;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;
use crate::app::font_view::{Column, MAX_CELL_SIZE, MAX_SEARCH_BYTES, MIN_CELL_SIZE, SEARCH_FIELDS, Sort, ViewMode};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most glyph names one selection command takes.
const MAX_SELECTION: usize = 100_000;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "fontView.show",
        label: "Show Font View",
        menu: &["View"],
        shortcut: Some("Cmd+Alt+1"),
        params: "{} show Font View, the first tab",
    },
    UiCommand {
        id: "fontView.setSelection",
        label: "Set Glyph Selection",
        menu: &[],
        shortcut: None,
        params: "{glyphs: [names], mode?: \"replace\" | \"add\" | \"remove\" (default replace)} select these glyphs in Font View (unknown names are an error); commands fired without targets while Font View shows act on this selection",
    },
    UiCommand {
        id: "view.editSelectedGlyphs",
        label: "Edit Selected Glyphs",
        menu: &[],
        shortcut: Some("Cmd+Down"),
        params: "{glyphs?: [names]} open the glyphs (default: the Font View selection) in Edit View: the tab that holds exactly them, else a new tab",
    },
    UiCommand {
        id: "fontView.setViewMode",
        label: "Grid View / List View",
        menu: &[],
        shortcut: None,
        params: "{mode: \"grid\" | \"list\"} show the glyphs as a cell grid or as a table of their properties",
    },
    UiCommand {
        id: "fontView.setCellSize",
        label: "Cell Size",
        menu: &[],
        shortcut: None,
        params: "{size: points (16 to 512)} the side of a grid cell; from 64 up, cells also show the name",
    },
    UiCommand {
        id: "fontView.sortByColumn",
        label: "Sort by Column",
        menu: &[],
        shortcut: None,
        params: "{column: id, ascending?: bool} sort List View by a column; without `ascending`, the column sorted already reverses, another sorts ascending. The id column ascending is glyph order",
    },
    UiCommand {
        id: "fontView.setColumns",
        label: "Show / Hide / Reorder Columns",
        menu: &[],
        shortcut: None,
        params: "{visible?: [column ids], order?: [column ids]} choose the List View columns shown (at least one) and their order; ids `order` leaves out follow in their present order",
    },
    UiCommand {
        id: "fontView.setSearch",
        label: "Search Glyphs",
        menu: &[],
        shortcut: None,
        params: "{query: string, fields?: [\"name\" | \"unicode\" | \"note\"], matchCase?: bool, regex?: bool} narrow Font View to the glyphs that match; an empty query shows them all",
    },
    UiCommand {
        id: "fontView.focusSearch",
        label: "Find (Search Glyphs)",
        menu: &[],
        shortcut: None,
        params: "{} give the keyboard to Font View's search field; in the edit view it does nothing. Edit › Find runs it while Font View shows",
    },
];

/// Gives the keyboard to Font View's search field on the next pass, while Font View shows.
pub fn focus_search(app: &mut FontCraftApp) -> Result<(), String> {
    if app.font_view_shows() {
        app.view.font_view.focus_search = true;
    }
    Ok(())
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "fontView.show" => {
            super::tabs::show(app, None);
            Ok(())
        }
        "fontView.setSelection" => set_selection(app, p),
        "fontView.focusSearch" => focus_search(app),
        "view.editSelectedGlyphs" => {
            let glyphs = match strings(p, "glyphs", fontcraft_textlayout::MAX_ITEMS)? {
                Some(list) => list,
                None => app.view.font_view.selected.iter().take(fontcraft_textlayout::MAX_ITEMS).cloned().collect(),
            };
            super::tabs::edit_glyphs(app, &glyphs)
        }
        "fontView.setViewMode" => {
            let id = p.get("mode").and_then(Value::as_str).ok_or("`mode` (\"grid\" or \"list\") is required")?;
            app.view.font_view.mode = ViewMode::parse(id).ok_or_else(|| format!("`mode` must be \"grid\" or \"list\", got \"{id}\""))?;
            Ok(())
        }
        "fontView.setCellSize" => {
            let size = p.get("size").and_then(Value::as_f64).filter(|s| s.is_finite()).ok_or("`size` (points) is required")?;
            app.view.font_view.cell_size = size.clamp(MIN_CELL_SIZE, MAX_CELL_SIZE).round();
            Ok(())
        }
        "fontView.sortByColumn" => {
            let column = column(p.get("column")).ok_or_else(|| format!("`column` must be a column id: {}", Column::ids()))?;
            let fv = &mut app.view.font_view;
            let ascending = match p.get("ascending").and_then(Value::as_bool) {
                Some(a) => a,
                None => match fv.sort {
                    Some(s) if s.column == column => !s.ascending,
                    None if column == Column::Id => false,
                    _ => true,
                },
            };
            // The id column ascending is glyph order, which is what no sort shows.
            fv.sort = (column != Column::Id || !ascending).then_some(Sort { column, ascending });
            Ok(())
        }
        "fontView.setColumns" => set_columns(app, p),
        "fontView.setSearch" => {
            let query = p.get("query").and_then(Value::as_str).ok_or("`query` (text) is required")?;
            if query.len() > MAX_SEARCH_BYTES {
                return Err(format!("the search is longer than {MAX_SEARCH_BYTES} bytes"));
            }
            let fields = strings(p, "fields", SEARCH_FIELDS.len())?.unwrap_or_default();
            if let Some(bad) = fields.iter().find(|f| !SEARCH_FIELDS.contains(&f.as_str())) {
                return Err(format!("there is no search field `{bad}`"));
            }
            let search = &mut app.view.font_view.search;
            search.query = query.to_string();
            search.fields = fields;
            search.match_case = p.get("matchCase").and_then(Value::as_bool).unwrap_or(false);
            search.regex = p.get("regex").and_then(Value::as_bool).unwrap_or(false);
            Ok(())
        }
        other => Err(format!("no Font View command `{other}`")),
    }
}

fn column(v: Option<&Value>) -> Option<Column> {
    v.and_then(Value::as_str).and_then(Column::parse)
}

/// The list of strings under `key`, at most `max` of them; `None` when absent.
fn strings(p: &Value, key: &str, max: usize) -> Result<Option<Vec<String>>, String> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| format!("`{key}` must be a list of names"))?;
    if list.len() > max {
        return Err(format!("`{key}` names more than {max} items"));
    }
    list.iter().map(|s| s.as_str().map(str::to_string).ok_or_else(|| format!("`{key}` must be a list of names"))).collect::<Result<_, _>>().map(Some)
}

fn columns(p: &Value, key: &str) -> Result<Option<Vec<Column>>, String> {
    let Some(ids) = strings(p, key, Column::ALL.len() * 4)? else { return Ok(None) };
    let mut out = Vec::new();
    for id in ids {
        let column = Column::parse(&id).ok_or_else(|| format!("there is no column `{id}`; the ids are {}", Column::ids()))?;
        if !out.contains(&column) {
            out.push(column);
        }
    }
    Ok(Some(out))
}

fn set_columns(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let visible = columns(p, "visible")?;
    let order = columns(p, "order")?;
    if visible.as_ref().is_some_and(Vec::is_empty) {
        return Err("`visible` must name at least one column".into());
    }
    let fv = &mut app.view.font_view;
    if let Some(mut next) = order {
        next.extend(fv.columns.iter().copied().chain(Column::ALL.iter().copied()));
        let mut seen = HashSet::new();
        next.retain(|c| seen.insert(*c));
        fv.columns = next;
    }
    if let Some(visible) = visible {
        fv.hidden = Column::ALL.iter().copied().filter(|c| !visible.contains(c)).collect();
    }
    Ok(())
}

fn set_selection(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let names = strings(p, "glyphs", MAX_SELECTION)?.ok_or("`glyphs` must be a list of glyph names")?;
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace");
    if !matches!(mode, "replace" | "add" | "remove") {
        return Err(format!("`mode` must be \"replace\", \"add\" or \"remove\", got \"{mode}\""));
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    // Sets, so selecting every glyph of a large font is one pass, not a pass per name.
    let known: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    if let Some(unknown) = names.iter().find(|n| !known.contains(n.as_str())) {
        return Err(format!("no glyph named `{unknown}`"));
    }
    let fv = &mut app.view.font_view;
    let current = std::mem::take(&mut fv.selected);
    fv.selected = match mode {
        "remove" => {
            let gone: HashSet<&str> = names.iter().map(String::as_str).collect();
            current.into_iter().filter(|n| !gone.contains(n.as_str())).collect()
        }
        _ => {
            let mut out = if mode == "add" { current } else { Vec::new() };
            let mut seen: HashSet<String> = out.iter().cloned().collect();
            out.extend(names.iter().filter(|n| seen.insert((*n).clone())).cloned());
            out
        }
    };
    if mode != "remove"
        && let Some(last) = names.last()
    {
        fv.anchor = Some(last.clone());
    }
    Ok(())
}

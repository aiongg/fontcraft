//! Font View: the glyph grid and List View, the status area, the glyph selection, and opening
//! glyphs for editing.
//!
//! | Module | Holds |
//! |---|---|
//! | [`cache`] | what the view lists: effective properties, glyph order, the visible glyphs |
//! | [`columns`] | List View's columns: ids, labels, cell text, sort keys, inline edits |
//! | [`grid`] | the cell grid: cells, badges, colour labels, selection by click and drag, rename |
//! | [`list`] | List View: header, sort, column order and visibility, inline edits |
//! | [`status`] | the mode buttons and the status area: counts and the cell size slider |
//! | [`images`] | cell outlines rasterised once per glyph and cell size |
//! | [`inspector`] | the glyph inspector at the bottom left: the selected glyphs' properties |
//! | [`sidebar`] | the sidebar: entries with counts and badges, Cmd and Shift selection, filter actions |
//! | [`search`] | the search field and its options |
//!
//! Everything here is view state (`UI_COMMANDS`). Font changes go through engine commands.

pub mod cache;
pub mod columns;
pub mod grid;
pub mod images;
pub mod inspector;
pub mod list;
pub mod search;
pub mod sidebar;
pub mod status;

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use columns::Column;

/// Smallest and largest grid cell, in points.
pub const MIN_CELL_SIZE: f64 = 16.0;
pub const MAX_CELL_SIZE: f64 = 512.0;
/// Cells at least this large show the glyph name and the Unicode marker under the outline
/// (`fontView.cell.contentBySize`).
pub const LABEL_MIN_CELL: f64 = 64.0;
/// Most glyphs whose Edit View node selection Font View remembers for the Palette.
pub const MAX_REMEMBERED: usize = 4_096;

pub const COMMANDS: &[UiCommand] = &[
    ("fontView.show", "Show Font View", &[], "Cmd+Alt+1", "{} leave Edit View and show the glyph grid"),
    (
        "fontView.setSelection",
        "Set Glyph Selection",
        &[],
        "",
        "{glyphs: [names], mode?: \"replace\" | \"add\" | \"remove\" (default replace)} select these glyphs in Font View (unknown names are an error); batch commands without explicit targets act on this selection → {glyphs: [names]}",
    ),
    (
        "view.editSelectedGlyphs",
        "Edit Selected Glyphs",
        &[],
        "Cmd+Down",
        "{glyphs?: [names]} open the glyphs (default: the Font View selection) in Edit View: the tab that holds exactly them, else a new tab",
    ),
    (
        "fontView.setViewMode",
        "Grid View / List View",
        &[],
        "",
        "{mode: \"grid\" | \"list\"} show the glyphs as a cell grid or as a table of their properties → {mode}",
    ),
    (
        "fontView.setCellSize",
        "Cell Size",
        &[],
        "",
        "{size: points (16 to 512)} the side of a grid cell; from 64 up, cells also show the name and the Unicode marker → {size}",
    ),
    (
        "fontView.sortByColumn",
        "Sort by Column",
        &[],
        "",
        "{column: id, ascending?: bool} sort List View by a column (ids: see fontView.getState › columns); without `ascending`, the column sorted already reverses, another sorts ascending. The id column ascending is glyph order → {column, ascending}",
    ),
    (
        "fontView.setColumns",
        "Show / Hide / Reorder Columns",
        &[],
        "",
        "{visible?: [column ids], order?: [column ids]} choose the List View columns shown (at least one) and their order; ids `order` leaves out follow in their present order → {columns: [shown ids in order], hidden: [ids]}",
    ),
    (
        "fontView.getState",
        "Get Font View State",
        &[],
        "",
        "{} → {showing, mode, cellSize, master, sort, columns, hidden, sidebarEntries, search, selected: [names], visible: [names in display order], counts: {selected, visible, total}, sections: [{at, title}], error}",
    ),
];

/// Grid or List View (`fontView.setViewMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

impl ViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::Grid => "grid",
            ViewMode::List => "list",
        }
    }
}

/// List View's sort (`fontView.sortByColumn`). No sort is glyph order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    pub column: Column,
    pub ascending: bool,
}

/// The nodes a glyph had selected in Edit View, in one layer: what the Palette's Fit Curve
/// acts on when it runs from Font View (`fontView.batch.palette`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSelection {
    pub layer: String,
    pub points: Vec<(usize, usize)>,
}

/// Font View's own state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FontViewState {
    pub mode: ViewMode,
    pub cell_size: f64,
    /// The master the cells draw (`view.selectMaster` while Font View shows). `None`, or a
    /// master the font no longer has: the first master.
    pub master: Option<String>,
    pub sort: Option<Sort>,
    /// Every List View column, in display order.
    pub columns: Vec<Column>,
    pub hidden: Vec<Column>,
    /// The parameters of `fontView.queryGlyphs` that narrow what the view lists (the sidebar
    /// and the search field); `None` lists every glyph.
    pub query: Option<Value>,
    /// The glyph Shift+click extends a range from: the one clicked last.
    pub anchor: Option<String>,
    /// Per glyph, the nodes selected the last time an Edit View tab edited it.
    pub node_selections: BTreeMap<String, NodeSelection>,
}

impl Default for FontViewState {
    fn default() -> Self {
        Self {
            mode: ViewMode::Grid,
            cell_size: LABEL_MIN_CELL,
            master: None,
            sort: None,
            columns: Column::ALL.to_vec(),
            hidden: Vec::new(),
            query: None,
            anchor: None,
            node_selections: BTreeMap::new(),
        }
    }
}

impl FontViewState {
    /// The columns shown, in order.
    pub fn shown_columns(&self) -> Vec<Column> {
        let mut out: Vec<Column> = self.columns.iter().copied().filter(|c| !self.hidden.contains(c)).collect();
        // Columns a stored order lacks (it may come from an older state) go last.
        out.extend(Column::ALL.iter().copied().filter(|c| !self.columns.contains(c) && !self.hidden.contains(c)));
        out
    }
}

/// Font View's state that is not data: caches, the drag in progress, the field being edited.
#[derive(Default)]
pub struct FontViewUi {
    pub cache: cache::FontViewCache,
    pub images: images::CellImages,
    /// List View's Char cells drawn from the glyph's outline, at row height.
    pub list_images: images::CellImages,
    pub drag: Option<grid::Drag>,
    pub edit: Option<InlineEdit>,
    /// The grid cells or list rows laid out in the last frame, with their glyph and their
    /// rectangle in the window: tests check that only the rows in view are laid out, and click
    /// them.
    pub laid_out: Vec<(String, egui::Rect)>,
    pub sidebar: sidebar::SidebarUi,
}

/// A name or a List View cell being edited in place.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineEdit {
    pub glyph: String,
    pub column: Column,
    pub text: String,
    /// The field has not had the keyboard yet.
    pub fresh: bool,
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "fontView.show" => {
            app.state.active_tab = None;
            Ok(json!({}))
        }
        "fontView.setSelection" => set_selection(app, args),
        "view.editSelectedGlyphs" => {
            let glyphs = match args.strings("glyphs", crate::state::MAX_TAB_GLYPHS)? {
                Some(list) => list,
                None => app.state.selected_glyphs.iter().take(crate::state::MAX_TAB_GLYPHS).cloned().collect(),
            };
            if glyphs.is_empty() {
                return Err(args.err("no glyph is selected"));
            }
            if let Some(index) = app.state.tabs.iter().position(|t| t.glyphs == glyphs) {
                app.state.active_tab = Some(index);
                return Ok(super::tabs::describe(app, index));
            }
            super::tabs::open(app, args, glyphs)
        }
        "fontView.setViewMode" => {
            let mode = match args.string("mode")? {
                Some("grid") => ViewMode::Grid,
                Some("list") => ViewMode::List,
                Some(other) => return Err(args.err(format!("`mode` must be \"grid\" or \"list\", got \"{other}\""))),
                None => return Err(args.err("`mode` (\"grid\" or \"list\") is required")),
            };
            app.state.font_view.mode = mode;
            app.font_view.edit = None;
            Ok(json!({"mode": mode.as_str()}))
        }
        "fontView.setCellSize" => {
            let size = args.number("size")?.ok_or_else(|| args.err("`size` (points) is required"))?;
            let size = size.clamp(MIN_CELL_SIZE, MAX_CELL_SIZE).round();
            app.state.font_view.cell_size = size;
            Ok(json!({"size": size}))
        }
        "fontView.sortByColumn" => {
            let column = column_arg(args, "column")?.ok_or_else(|| args.err("`column` (a column id) is required"))?;
            let fv = &mut app.state.font_view;
            let ascending = match args.flag("ascending")? {
                Some(a) => a,
                None => match fv.sort {
                    Some(s) if s.column == column => !s.ascending,
                    None if column == Column::Id => false,
                    _ => true,
                },
            };
            // The id column ascending is glyph order, which is what no sort shows.
            fv.sort = (column != Column::Id || !ascending).then_some(Sort { column, ascending });
            Ok(json!({"column": column.id(), "ascending": ascending}))
        }
        "fontView.setColumns" => set_columns(app, args),
        "fontView.getState" => {
            refresh(app);
            Ok(state(app))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn column_arg(args: &Args, key: &str) -> Result<Option<Column>, String> {
    match args.string(key)? {
        None => Ok(None),
        Some(id) => Column::parse(id).map(Some).ok_or_else(|| args.err(format!("there is no column `{id}`; the ids are {}", Column::ids()))),
    }
}

fn column_list(args: &Args, key: &str) -> Result<Option<Vec<Column>>, String> {
    let Some(ids) = args.strings(key, Column::ALL.len() * 4)? else { return Ok(None) };
    let mut out = Vec::new();
    for id in ids {
        let column = Column::parse(&id).ok_or_else(|| args.err(format!("there is no column `{id}`; the ids are {}", Column::ids())))?;
        if !out.contains(&column) {
            out.push(column);
        }
    }
    Ok(Some(out))
}

fn set_columns(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let visible = column_list(args, "visible")?;
    let order = column_list(args, "order")?;
    if visible.as_ref().is_some_and(Vec::is_empty) {
        return Err(args.err("`visible` must name at least one column"));
    }
    let fv = &mut app.state.font_view;
    if let Some(order) = order {
        let mut next = order;
        next.extend(fv.columns.iter().copied().chain(Column::ALL.iter().copied()).filter(|c| !next.contains(c)).collect::<Vec<_>>());
        let mut seen = HashSet::new();
        next.retain(|c| seen.insert(*c));
        fv.columns = next;
    }
    if let Some(visible) = visible {
        fv.hidden = Column::ALL.iter().copied().filter(|c| !visible.contains(c)).collect();
    }
    Ok(json!({"columns": ids(&fv.shown_columns()), "hidden": ids(&fv.hidden)}))
}

fn ids(columns: &[Column]) -> Vec<&'static str> {
    columns.iter().map(|c| c.id()).collect()
}

fn set_selection(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let names = args.strings("glyphs", fontcraft_model::MAX_GLYPHS)?.ok_or_else(|| args.err("`glyphs` must be a list of glyph names"))?;
    let mode = args.string("mode")?.unwrap_or("replace");
    if !matches!(mode, "replace" | "add" | "remove") {
        return Err(args.err(format!("`mode` must be \"replace\", \"add\" or \"remove\", got \"{mode}\"")));
    }
    // Sets, so selecting every glyph of a large font is one pass, not a pass per name.
    let known: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    if let Some(unknown) = names.iter().find(|n| !known.contains(n.as_str())) {
        return Err(args.err(format!("no glyph named `{unknown}`")));
    }
    let current = std::mem::take(&mut app.state.selected_glyphs);
    let selected = match mode {
        "remove" => {
            let gone: HashSet<&str> = names.iter().map(String::as_str).collect();
            current.into_iter().filter(|n| !gone.contains(n.as_str())).collect()
        }
        _ => {
            let mut out = if mode == "add" { current } else { Vec::new() };
            let mut seen: HashSet<String> = out.iter().cloned().collect();
            for name in &names {
                if seen.insert(name.clone()) {
                    out.push(name.clone());
                }
            }
            out
        }
    };
    if mode != "remove"
        && let Some(last) = names.last()
    {
        app.state.font_view.anchor = Some(last.clone());
    }
    app.state.selected_glyphs = selected;
    Ok(json!({"glyphs": app.state.selected_glyphs}))
}

/// Rebuilds what Font View lists when something it depends on changed.
pub fn refresh(app: &mut FontcraftApp) {
    app.font_view.cache.refresh(&mut app.session, &app.state.font_view);
}

/// The master Font View draws: its own choice, else the first master.
pub fn master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    cache::resolve_master(font, app.state.font_view.master.as_deref())
}

/// The names Font View shows, in display order.
pub fn visible_names(app: &FontcraftApp) -> Vec<String> {
    let cache = &app.font_view.cache;
    cache.visible.iter().filter_map(|&i| cache.rows.get(i)).map(|r| r.glyph.name.clone()).collect()
}

/// `fontView.getState`'s reply. The cache must be fresh.
fn state(app: &FontcraftApp) -> Value {
    let fv = &app.state.font_view;
    let cache = &app.font_view.cache;
    let visible = visible_names(app);
    let total = cache.rows.len();
    let query = fv.query.as_ref().and_then(Value::as_object);
    let sections: Vec<Value> = cache.shown_sections(fv).iter().map(|(at, title)| json!({"at": at, "title": title})).collect();
    json!({
        "showing": app.state.tab().is_none(),
        "mode": fv.mode.as_str(),
        "cellSize": fv.cell_size,
        "master": master(app),
        "sort": fv.sort.map(|s| json!({"column": s.column.id(), "ascending": s.ascending})),
        "columns": ids(&fv.shown_columns()),
        "hidden": ids(&fv.hidden),
        "sidebarEntries": query.and_then(|q| q.get("entries")).cloned().unwrap_or_else(|| json!([])),
        "search": query.and_then(|q| q.get("search")).cloned().unwrap_or(Value::Null),
        "selected": app.state.selected_glyphs,
        "counts": {"selected": app.state.selected_glyphs.len(), "visible": visible.len(), "total": total},
        "visible": visible,
        "sections": sections,
        "error": cache.error,
    })
}

/// Select All, Deselect All and Invert Selection in Font View: on the visible glyphs
/// (`fontView.selectAllScope`, `fontView.invertSelection`).
pub fn select(app: &mut FontcraftApp, id: &str) -> Value {
    refresh(app);
    let visible = visible_names(app);
    let selected = match id {
        "edit.selectAll" => visible,
        "edit.invertSelection" => {
            let current: HashSet<&str> = app.state.selected_glyphs.iter().map(String::as_str).collect();
            visible.into_iter().filter(|n| !current.contains(n.as_str())).collect()
        }
        _ => Vec::new(),
    };
    app.state.selected_glyphs = selected;
    json!({"glyphs": app.state.selected_glyphs})
}

/// The targets a batch command gets from the Font View selection (`fontView.batch.appliesToSelection`):
/// the selected glyphs, and the layer of Font View's master when the command takes `layer`.
/// `None` when nothing is selected or the command does not act on glyphs.
pub fn batch_targets(app: &FontcraftApp, spec: &fontcraft_engine::CommandSpec) -> Option<Map<String, Value>> {
    if app.state.selected_glyphs.is_empty() || !spec.targets.contains(&fontcraft_engine::params::TargetKind::Glyphs) {
        return None;
    }
    let mut m = Map::new();
    m.insert("glyphs".into(), json!(app.state.selected_glyphs));
    if crate::tools_glue::takes_param(spec, "layer?")
        && let Some(master) = master(app)
    {
        m.insert("layer".into(), json!(master));
    }
    Some(m)
}

/// Remembers the showing tab's node selection for its edited glyph, for the Palette's Fit
/// Curve in Font View. An empty selection forgets the glyph.
pub fn remember_node_selection(app: &mut FontcraftApp) {
    let Some(tab) = app.state.tab() else { return };
    let (Some(glyph), Some(layer)) = (tab.selection_glyph.clone(), tab.selection_layer.clone()) else { return };
    if tab.edit_background {
        return;
    }
    let points: Vec<(usize, usize)> = tab.selection.points.iter().copied().collect();
    let kept = &mut app.state.font_view.node_selections;
    if points.is_empty() {
        kept.remove(&glyph);
        return;
    }
    let next = NodeSelection { layer, points };
    if kept.get(&glyph) == Some(&next) {
        return;
    }
    if kept.len() >= MAX_REMEMBERED && !kept.contains_key(&glyph) {
        kept.clear();
    }
    kept.insert(glyph, next);
}

/// Draws Font View: the mode buttons, the grid or the list, and the status area.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    refresh(app);
    status::show(app, ui, pending);
    sidebar::show(app, ui, pending);
    let frame = egui::Frame::NONE.fill(app.tokens.panel_darker).inner_margin(app.tokens.font_view_margin);
    if app.session.font().is_err() {
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.centered_and_justified(|ui| ui.label("File › New to start a font"));
        });
        return;
    }
    super::context_menu::font_view(app, ui, pending);
    egui::CentralPanel::default().frame(frame).show(ui, |ui| match app.state.font_view.mode {
        ViewMode::Grid => grid::show(app, ui, pending),
        ViewMode::List => list::show(app, ui, pending),
    });
}

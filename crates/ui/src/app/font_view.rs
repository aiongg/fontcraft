//! Font View's view state and List View's columns, and what the app layer does for Font View: the targets that
//! commands fired from menus and keys take from its glyph selection (`fontView.batch.appliesToSelection`), and what
//! follows a command it ran (the glyphs it added selected and scrolled to, how many it added, renamed and skipped, the
//! compatibility notice of Tidy up and Round Coordinates). Edit › Select All, Deselect All and Invert Selection act on
//! the visible glyphs while Font View shows ([`FontCraftApp::select_in_font_view`]).

use std::collections::{HashMap, HashSet};

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::WordSet as _;
use fontcraft_engine::model::glyph_order;
use fontcraft_engine::params::TargetKind;
use serde_json::{Map, Value, json};

use super::FontCraftApp;
use super::models::ProjectCx;

/// Smallest and largest grid cell, in points (`fontView.setCellSize`).
pub const MIN_CELL_SIZE: f64 = 16.0;
pub const MAX_CELL_SIZE: f64 = 512.0;
/// Cells at least this large show the glyph name under the outline (`fontView.cell.contentBySize`).
pub const LABEL_MIN_CELL: f64 = 64.0;
/// Longest search term, in bytes (`fontView.queryGlyphs` refuses longer ones).
pub const MAX_SEARCH_BYTES: usize = 1024;
/// The fields a search can be limited to.
pub const SEARCH_FIELDS: [&str; 3] = ["name", "unicode", "note"];
/// Largest bitmap magnification of `view.setFontViewStrike`: points per strike pixel.
pub const MAX_STRIKE_MAGNIFICATION: u8 = 16;

/// The bitmap strike the grid cells draw in place of the outlines (`view.setFontViewStrike`): its size and depth,
/// and how many points one of its pixels takes at most.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrikeView {
    pub ppem: u16,
    pub depth: u8,
    pub magnification: u8,
}
/// Most glyphs one batch over the selection names.
pub const MAX_BATCH_GLYPHS: usize = 100_000;

/// Grid or List View (`fontView.setViewMode`).
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
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

    pub fn parse(id: &str) -> Option<ViewMode> {
        [ViewMode::Grid, ViewMode::List].into_iter().find(|m| m.as_str() == id)
    }
}

/// List View's sort (`fontView.sortByColumn`). No sort is glyph order.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub ascending: bool,
}

/// The search field's term and options (`fontView.setSearch`). Empty `fields` searches every field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub fields: Vec<String>,
    pub match_case: bool,
    pub regex: bool,
}

impl Search {
    /// The `search` value of a `fontView.queryGlyphs` request; `None` when nothing is typed.
    pub fn to_json(&self) -> Option<Value> {
        if self.query.is_empty() {
            return None;
        }
        Some(json!({"query": self.query, "fields": self.fields, "matchCase": self.match_case, "regex": self.regex}))
    }
}

/// A value typed into a List View cell: kept while the command it ran has not succeeded, so a refused value stays in
/// its cell with the reason.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct CellDraft {
    pub glyph: String,
    pub column: Column,
    pub text: String,
    /// Why the command refused it; `None` until it ran.
    pub error: Option<String>,
}

/// Font View's view state: no engine command changes it.
#[derive(Clone, Debug, PartialEq)]
pub struct FontViewState {
    pub mode: ViewMode,
    pub cell_size: f64,
    pub sort: Option<Sort>,
    /// Every List View column, in display order.
    pub columns: Vec<Column>,
    pub hidden: Vec<Column>,
    pub search: Search,
    /// The sidebar entries that narrow the view (`fontView.queryGlyphs`' `entries`); `None` lists every glyph.
    pub entries: Option<Value>,
    /// The selected glyphs, in the order they were selected.
    pub selected: Vec<String>,
    /// The glyph Shift extends a range from: the one picked last.
    pub anchor: Option<String>,
    /// The search field takes the keyboard on the next pass (`fontView.focusSearch`).
    pub focus_search: bool,
    /// The glyph the grid or the list scrolls to on the next pass.
    pub reveal: Option<String>,
    /// The List View column whose cell in the lead row takes the keyboard on the next pass.
    pub edit_cell: Option<Column>,
    /// The List View value not yet taken.
    pub draft: Option<CellDraft>,
    /// The strike the grid draws; `None` draws the outlines.
    pub strike: Option<StrikeView>,
}

impl Default for FontViewState {
    fn default() -> Self {
        FontViewState {
            mode: ViewMode::Grid,
            cell_size: LABEL_MIN_CELL,
            sort: None,
            columns: Column::ALL.to_vec(),
            hidden: Vec::new(),
            search: Search::default(),
            entries: None,
            selected: Vec::new(),
            anchor: None,
            focus_search: false,
            reveal: None,
            edit_cell: None,
            draft: None,
            strike: None,
        }
    }
}

impl FontViewState {
    /// The columns shown, in order.
    pub fn shown_columns(&self) -> Vec<Column> {
        let mut out: Vec<Column> = self.columns.iter().copied().filter(|c| !self.hidden.contains(c)).collect();
        out.extend(Column::ALL.iter().copied().filter(|c| !self.columns.contains(c) && !self.hidden.contains(c)));
        out
    }

    /// The `fontView.queryGlyphs` parameters that narrow what the view lists; `None` lists every glyph.
    pub fn query(&self) -> Option<Value> {
        let mut m = Map::new();
        if let Some(entries) = &self.entries {
            m.insert("entries".into(), entries.clone());
        }
        if let Some(search) = self.search.to_json() {
            m.insert("search".into(), search);
        }
        (!m.is_empty()).then_some(Value::Object(m))
    }
}

/// A List View column (`fontView.column.*`). The id is the column's name in commands.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Column {
    Id,
    Char,
    Name,
    Unicode,
    ProductionName,
    Script,
    Category,
    SubCategory,
    Case,
    Direction,
    Export,
    Color,
    Tags,
    SortName,
    Note,
    Components,
    LastChanged,
    Lsb,
    Rsb,
    Width,
    LeftGroup,
    RightGroup,
    /// The status the glyph is in (`glyph.setStatus`).
    Status,
}

impl Column {
    /// Every column, in the default order: the documented columns first, then the properties in the order the
    /// Glyph Properties section lists them, then Status.
    pub const ALL: [Column; 23] = [
        Column::Id,
        Column::Char,
        Column::Name,
        Column::Unicode,
        Column::ProductionName,
        Column::Script,
        Column::Category,
        Column::SubCategory,
        Column::Case,
        Column::Direction,
        Column::Export,
        Column::Color,
        Column::Tags,
        Column::SortName,
        Column::Note,
        Column::Components,
        Column::LastChanged,
        Column::Lsb,
        Column::Rsb,
        Column::Width,
        Column::LeftGroup,
        Column::RightGroup,
        Column::Status,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Column::Id => "id",
            Column::Char => "char",
            Column::Name => "name",
            Column::Unicode => "unicode",
            Column::ProductionName => "productionName",
            Column::Script => "script",
            Column::Category => "category",
            Column::SubCategory => "subCategory",
            Column::Case => "case",
            Column::Direction => "direction",
            Column::Export => "export",
            Column::Color => "color",
            Column::Tags => "tags",
            Column::SortName => "sortName",
            Column::Note => "note",
            Column::Components => "components",
            Column::LastChanged => "lastChanged",
            Column::Lsb => "lsb",
            Column::Rsb => "rsb",
            Column::Width => "width",
            Column::LeftGroup => "leftGroup",
            Column::RightGroup => "rightGroup",
            Column::Status => "status",
        }
    }

    pub fn parse(id: &str) -> Option<Column> {
        Column::ALL.iter().copied().find(|c| c.id() == id)
    }

    /// Every id, for error messages.
    pub fn ids() -> String {
        Column::ALL.iter().map(|c| c.id()).collect::<Vec<_>>().join(", ")
    }

    /// The column's position in [`Column::ALL`]: where a row keeps its cell text.
    pub fn index(self) -> usize {
        Column::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// Numbers are drawn flush to the end.
    pub fn is_numeric(self) -> bool {
        matches!(self, Column::Id | Column::Lsb | Column::Rsb | Column::Width)
    }
}

craft::ui::words! {
    /// The status notices the app layer writes after a command.
    pub struct NoticeWords {
        /// After Tidy up Paths or Round Coordinates, before the glyphs whose masters no longer interpolate.
        incompatible: "fontcraftNotice.incompatible" = "No longer compatible across masters";
        /// After a command that added glyphs, each after a count.
        added: "fontcraftNotice.added" = "added";
        replaced: "fontcraftNotice.replaced" = "replaced";
        existing: "fontcraftNotice.existing" = "already in the font";
        renamed: "fontcraftNotice.renamed" = "renamed";
        left_out: "fontcraftNotice.leftOut" = "skipped";
    }
}

/// The commands whose reply lists, under `incompatible`, the glyphs the run left incompatible across masters.
const REPORTS_INCOMPATIBLE: [&str; 4] =
    ["path.tidyUpPaths", "path.tidyUpPathsForAllMasters", "path.roundCoordinates", "path.roundCoordinatesForAllMasters"];
/// Most glyph names a status notice spells out.
pub(super) const MAX_NOTICE_NAMES: usize = 20;

/// The status message for `reply` of command `id`: the glyphs it left incompatible across masters, when there are
/// any.
pub fn incompatible_notice(id: &str, reply: &Value) -> Option<String> {
    if !REPORTS_INCOMPATIBLE.contains(&id) {
        return None;
    }
    let names: Vec<&str> = reply.get("incompatible")?.as_array()?.iter().filter_map(Value::as_str).collect();
    if names.is_empty() {
        return None;
    }
    let mut list = names.iter().take(MAX_NOTICE_NAMES).copied().collect::<Vec<_>>().join(", ");
    if names.len() > MAX_NOTICE_NAMES {
        list.push_str(", …");
    }
    Some(format!("{}: {list}", NoticeWords::english().incompatible.as_str()))
}

/// The glyphs engine command `id` added or made, from its reply: they become the Font View selection. Empty for a
/// command that adds none.
pub fn results(id: &str, reply: &Value) -> Vec<String> {
    let names = |key: &str| -> Vec<String> {
        let list = reply.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        list.iter().filter_map(Value::as_str).take(MAX_BATCH_GLYPHS).map(str::to_string).collect()
    };
    let mut out = match id {
        "glyph.newGlyph" => reply.get("name").and_then(Value::as_str).map(str::to_string).into_iter().collect(),
        "glyph.addGlyphs" => [names("added"), names("replaced")].concat(),
        "fontView.generateMissingGlyphs" => names("added"),
        "glyphInfo.addToFont" => {
            let added = names("added");
            // Nothing new: the glyphs asked for are shown where they are.
            if added.is_empty() { names("existing") } else { added }
        }
        "glyph.duplicateGlyph" => names("glyphs"),
        "edit.paste" if reply.get("flavour").and_then(Value::as_str) == Some("glyphs") => names("glyphs"),
        "edit.pasteSpecial" => [names("glyphs"), names("added")].concat(),
        _ => Vec::new(),
    };
    let mut seen = HashSet::new();
    out.retain(|n| seen.insert(n.clone()));
    out.truncate(MAX_BATCH_GLYPHS);
    out
}

/// `Add Glyphs: 3 added, 1 renamed, 2 skipped (reason)`: what a command that adds glyphs did; `None` for another
/// command, for New Glyph (its one glyph shows selected) and when nothing happened.
pub fn added_notice(id: &str, reply: &Value) -> Option<String> {
    if !matches!(
        id,
        "glyph.addGlyphs" | "fontView.generateMissingGlyphs" | "glyphInfo.addToFont" | "glyph.duplicateGlyph" | "edit.paste" | "edit.pasteSpecial"
    ) || (id == "edit.paste" && reply.get("flavour").and_then(Value::as_str) != Some("glyphs"))
    {
        return None;
    }
    let words = NoticeWords::english();
    let count = |key: &str| reply.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    let added = match id {
        "glyph.duplicateGlyph" | "edit.paste" => count("glyphs"),
        _ => count("added"),
    };
    let changed = if id == "edit.pasteSpecial" { count("glyphs") } else { count("replaced") };
    let parts = [
        (added, words.added.as_str()),
        (changed, words.replaced.as_str()),
        (count("renamed"), words.renamed.as_str()),
        (count("existing"), words.existing.as_str()),
        (count("skipped"), words.left_out.as_str()),
    ];
    let mut said: Vec<String> = parts.iter().filter(|(n, _)| *n > 0).map(|(n, word)| format!("{n} {word}")).collect();
    let reason = reply.get("skipped").and_then(Value::as_array).and_then(|l| l.first()).and_then(|s| s.get("reason")).and_then(Value::as_str);
    if let (Some(last), Some(reason)) = (said.last_mut(), reason) {
        last.push_str(&format!(" ({reason})"));
    }
    if said.is_empty() {
        return None;
    }
    let label = fontcraft_engine::find_command(id).map_or(id, |spec| spec.label);
    Some(format!("{label}: {}", said.join(", ")))
}

/// What `edit.selectAll`, `edit.deselectAll` and `edit.invertSelection` make of `selected` among the `visible` glyphs.
fn selection_after(id: &str, visible: Vec<String>, selected: &[String]) -> Vec<String> {
    match id {
        "edit.selectAll" => visible,
        "edit.invertSelection" => {
            let selected: HashSet<&str> = selected.iter().map(String::as_str).collect();
            visible.into_iter().filter(|n| !selected.contains(n.as_str())).collect()
        }
        _ => Vec::new(),
    }
}

/// Commands that address one glyph and that Font View runs once per selected glyph, as one undo step
/// (`fontView.batch.addComponent`).
const PER_GLYPH: [&str; 2] = ["glyph.addComponent", "glyph.addComponentAllMasters"];

impl FontCraftApp {
    /// Whether Font View is the document area's showing tab.
    pub fn font_view_shows(&self) -> bool {
        self.view.tabs.font_view_shows()
    }

    /// Font View's master: the current master (`view.selectMaster`).
    fn font_view_master(&self) -> Option<String> {
        let font = self.session.font().ok()?;
        crate::shell::masters::current_master(font, &self.view.layers).map(|m| m.id.clone())
    }

    /// `command` fired from a menu or a key with its targets filled from the Font View selection: the selected
    /// glyphs, and the layer of Font View's master when the command takes `layer`. A command given parameters, one
    /// that takes no glyphs, and every command while Font View is hidden or has no selection stay as they are. The
    /// [`PER_GLYPH`] commands become one `command.batch` with a step per glyph.
    pub fn with_font_view_targets(&self, command: CommandRef) -> CommandRef {
        let given = match &command.params {
            Value::Null => Map::new(),
            Value::Object(m) => m.clone(),
            _ => return command,
        };
        let selected = &self.view.font_view.selected;
        if selected.is_empty() || !self.font_view_shows() {
            return command;
        }
        let Some(spec) = fontcraft_engine::find_command(&command.id) else { return command };
        let takes_layer = spec.params.split('→').next().is_some_and(|p| p.contains("layer?"));
        let layer = if takes_layer && !given.contains_key("layer") { self.font_view_master() } else { None };
        let glyphs: Vec<&String> = selected.iter().take(MAX_BATCH_GLYPHS).collect();
        if PER_GLYPH.contains(&command.id.as_str()) && !given.contains_key("glyph") {
            let steps: Vec<Value> = glyphs
                .iter()
                .map(|glyph| {
                    let mut m = given.clone();
                    m.insert("glyph".into(), json!(glyph));
                    if let Some(layer) = &layer {
                        m.insert("layer".into(), json!(layer));
                    }
                    json!({"command": command.id, "params": m})
                })
                .collect();
            return CommandRef::with_params("command.batch", json!({"label": spec.label, "steps": steps}));
        }
        if !given.is_empty() || !spec.targets.contains(&TargetKind::Glyphs) {
            return command;
        }
        let mut m = Map::new();
        m.insert("glyphs".into(), json!(glyphs));
        if let Some(layer) = layer {
            m.insert("layer".into(), json!(layer));
        }
        CommandRef::with_params(command.id, Value::Object(m))
    }

    /// What follows engine command `command` that returned `reply`: a new glyph becomes the Font View selection;
    /// returns the status notice the reply calls for.
    pub(super) fn after_command(&mut self, command: &CommandRef, reply: &Value) -> Option<String> {
        if self.view.font_view.draft.take().is_some() {
            self.view_revision += 1;
        }
        let made = results(&command.id, reply);
        if !made.is_empty() {
            self.select_and_reveal(made);
        }
        incompatible_notice(&command.id, reply).or_else(|| added_notice(&command.id, reply))
    }

    /// What follows an engine command that failed: the List View value it was to set stays in its cell with `error`.
    pub(super) fn refuse_draft(&mut self, error: &str) {
        if let Some(draft) = self.view.font_view.draft.as_mut().filter(|d| d.error.is_none()) {
            draft.error = Some(error.to_string());
            self.view_revision += 1;
        }
    }

    /// After Return in a List View cell: the same cell of `glyph`, the next row, takes the keyboard; a refused value
    /// keeps it.
    pub(super) fn next_cell(&mut self, glyph: String, column: Column) {
        if self.view.font_view.draft.is_none() {
            self.select_and_reveal(vec![glyph]);
        }
        self.view.font_view.edit_cell = Some(column);
    }

    /// Selects `glyphs` in Font View, which scrolls to the first of them when it next shows.
    pub(crate) fn select_and_reveal(&mut self, glyphs: Vec<String>) {
        let fv = &mut self.view.font_view;
        fv.reveal = glyphs.first().cloned();
        fv.anchor = glyphs.last().cloned();
        fv.selected = glyphs;
        fv.draft = None;
        self.view_revision += 1;
    }

    /// The glyphs Font View lists, in the order it shows them.
    pub fn visible_glyphs(&mut self) -> Result<Vec<String>, String> {
        let master = self.font_view_master().unwrap_or_default();
        let query = self.view.font_view.query();
        let mut names: Vec<String> = match query {
            Some(query) => super::models::font_view::query_names(&mut ProjectCx { session: &mut self.session, view: &self.view }, query, &master)?,
            None => {
                let font = self.session.font().map_err(|e| e.to_string())?;
                let arcs = font.glyphs.as_arcs();
                glyph_order(font).into_iter().filter_map(|i| arcs.get(i)).map(|g| g.name.clone()).collect()
            }
        };
        // A List View sort is in the model; a name the model has not listed yet stays last.
        if self.view.font_view.sort.is_some() {
            let m = &self.models.font_view;
            let at: HashMap<&str, usize> = m.visible.iter().enumerate().filter_map(|(at, &row)| m.name(row).map(|n| (n, at))).collect();
            names.sort_by_key(|n| at.get(n.as_str()).copied().unwrap_or(usize::MAX));
        }
        Ok(names)
    }

    /// Edit › Select All, Deselect All or Invert Selection while Font View shows: the visible glyphs, none of them, or
    /// those that were not selected (`fontView.selectAllScope`, `fontView.deselectAll`, `fontView.invertSelection`).
    pub(crate) fn select_in_font_view(&mut self, id: &str) -> Result<(), String> {
        let visible = if id == "edit.deselectAll" { Vec::new() } else { self.visible_glyphs()? };
        let selected = selection_after(id, visible, &self.view.font_view.selected);
        let fv = &mut self.view.font_view;
        fv.anchor = selected.last().cloned();
        fv.selected = selected;
        self.view_revision += 1;
        Ok(())
    }

    /// Drops the selected glyphs the font no longer has.
    pub(super) fn prune_font_view_selection(&mut self) {
        let Ok(font) = self.session.font() else {
            self.view.font_view.selected.clear();
            return;
        };
        let known: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
        let before = self.view.font_view.selected.len();
        let kept: Vec<String> = self.view.font_view.selected.iter().filter(|n| known.contains(n.as_str())).cloned().collect();
        let gone = self.view.font_view.reveal.as_deref().is_some_and(|n| !known.contains(n));
        if gone {
            self.view.font_view.reveal = None;
        }
        if kept.len() != before {
            self.view.font_view.selected = kept;
            self.view_revision += 1;
        }
    }
}

#[cfg(test)]
#[path = "font_view_tests.rs"]
mod tests;

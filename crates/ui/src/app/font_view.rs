//! Font View's view state and List View's columns, and what the app layer does for Font View: what
//! follows a command it ran (the new glyph selected, the compatibility notice of Tidy up and Round Coordinates).

use std::collections::HashSet;

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::WordSet as _;
use serde_json::{Map, Value, json};

use super::FontCraftApp;

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
        /// Before the locked glyphs a command on several glyphs left out.
        locked: "fontcraftNotice.locked" = "Locked, left as they were";
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
    Some(listed(NoticeWords::english().incompatible.as_str(), &names))
}

/// `what: A, B`, with at most [`MAX_NOTICE_NAMES`] names.
fn listed(what: &str, names: &[&str]) -> String {
    let mut list = names.iter().take(MAX_NOTICE_NAMES).copied().collect::<Vec<_>>().join(", ");
    if names.len() > MAX_NOTICE_NAMES {
        list.push_str(", …");
    }
    format!("{what}: {list}")
}

/// The status message for `reply` of a command that ran on several glyphs: the locked ones among those it lists under
/// `skipped`, as names or as objects with a `glyph`.
pub(super) fn locked_notice(font: &fontcraft_engine::model::Font, reply: &Value) -> Option<String> {
    let batched = reply.get("results").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let skipped = std::iter::once(reply).chain(batched).filter_map(|r| r.get("skipped").and_then(Value::as_array)).flatten();
    let mut seen = HashSet::new();
    let names: Vec<&str> = skipped
        .filter_map(|entry| entry.as_str().or_else(|| entry.get("glyph").and_then(Value::as_str)))
        .filter(|name| font.glyph(name).is_some_and(|g| g.info.locked) && seen.insert(*name))
        .collect();
    (!names.is_empty()).then(|| listed(NoticeWords::english().locked.as_str(), &names))
}

impl FontCraftApp {
    /// Whether Font View is the document area's showing tab.
    pub fn font_view_shows(&self) -> bool {
        self.view.tabs.font_view_shows()
    }

    /// What follows engine command `command` that returned `reply`: a new glyph becomes the Font View selection;
    /// returns the status notice the reply calls for.
    pub(super) fn after_command(&mut self, command: &CommandRef, reply: &Value) -> Option<String> {
        if command.id == "glyph.newGlyph"
            && let Some(name) = reply.get("name").and_then(Value::as_str)
        {
            self.view.font_view.selected = vec![name.to_string()];
            self.view.font_view.anchor = Some(name.to_string());
            self.view_revision += 1;
        }
        incompatible_notice(&command.id, reply)
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
        if kept.len() != before {
            self.view.font_view.selected = kept;
            self.view_revision += 1;
        }
    }
}

#[cfg(test)]
#[path = "font_view_tests.rs"]
mod tests;

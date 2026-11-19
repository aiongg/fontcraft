//! The sidebar's view model: All and the Categories, Languages and Filters sections as `fontView.queryGlyphs`
//! lists them with `sidebar: true`, flattened in display order, with each entry's count or present/expected badge,
//! whether it shows (every folder above it open) and the selected entries.
//!
//! Nothing is projected while the sidebar panel is hidden.

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use serde_json::{Map, Value, json};

use super::ProjectCx;
use crate::app::sidebar::{MAX_DEPTH, MAX_ROWS, entries_of};
use crate::shell::dock::PanelId;

/// What the sidebar shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SidebarModel {
    /// A font is open.
    pub has_font: bool,
    /// Every entry, children after their parent.
    pub rows: Vec<SidebarRow>,
    /// The rows shown, as indices into `rows`: those whose folders above are all open.
    pub visible: Vec<usize>,
    /// The selected entries' paths (none: All).
    pub selected: Vec<Vec<String>>,
    /// Why the entries could not be read.
    pub error: Option<String>,
    /// The entry whose context menu is open.
    pub menu_row: Option<Vec<String>>,
}

/// One sidebar entry.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SidebarRow {
    pub path: Vec<String>,
    pub label: String,
    /// `all`, `section`, `category`, `subcategory`, `case`, `script`, `language`, `group`, `smart`, `list` or
    /// `folder`.
    pub kind: String,
    /// How many glyphs of the font the entry shows.
    pub count: usize,
    /// For an entry that knows its full glyph set: how many of them the font has, and how many it expects.
    pub present: Option<usize>,
    pub expected: Option<usize>,
    /// Why a filter cannot be used.
    pub problem: Option<String>,
    /// Nesting depth; a section is 0.
    pub depth: usize,
    pub has_children: bool,
    /// Its children show.
    pub open: bool,
}

impl SidebarRow {
    /// A section heading: it opens and closes, and is never selected.
    pub fn is_section(&self) -> bool {
        self.kind == "section"
    }

    /// A smart filter, list filter or folder of the Filters section: it can be renamed and removed.
    pub fn is_filter(&self) -> bool {
        matches!(self.kind.as_str(), "smart" | "list" | "folder")
    }

    /// The entry knows its full set and the font lacks some of it.
    pub fn lacks_glyphs(&self) -> bool {
        matches!((self.present, self.expected), (Some(p), Some(e)) if p < e)
    }

    /// The entry knows its full set and the font has all of it.
    pub fn complete(&self) -> bool {
        matches!((self.present, self.expected), (Some(p), Some(e)) if p >= e)
    }
}

impl SidebarModel {
    /// The row of `path`.
    pub fn row_of(&self, path: &[String]) -> Option<usize> {
        self.rows.iter().position(|r| r.path == path)
    }

    /// The entries of section `section` in the order the sidebar lists them, the section itself left out.
    pub fn section_order(&self, section: &str) -> Vec<Vec<String>> {
        self.rows.iter().filter(|r| !r.is_section() && r.path.first().is_some_and(|s| s == section)).map(|r| r.path.clone()).collect()
    }

    /// The top-level names of the Filters section.
    pub fn filter_names(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| r.path.len() == 2 && r.path.first().is_some_and(|s| s == "filters"))
            .filter_map(|r| r.path.last().cloned())
            .collect()
    }
}

pub fn project(model: &mut Tracked<SidebarModel>, cx: &mut ProjectCx<'_>) {
    let shows = cx.view.workspace.layout.panel_state(&PanelId::Sidebar) != PanelState::Hidden;
    let Ok(font) = cx.session.font() else {
        super::set(model, SidebarModel::default());
        return;
    };
    if !shows {
        super::set(model, SidebarModel { has_font: true, ..SidebarModel::default() });
        return;
    }
    let master = font.masters.first().map(|m| m.id.clone());
    let mut params = Map::new();
    params.insert("sidebar".into(), json!(true));
    if let Some(m) = master {
        params.insert("master".into(), json!(m));
    }
    let (items, error) = match cx.session.query("fontView.queryGlyphs", &Value::Object(params)) {
        Ok(result) => (result.get("sidebar").and_then(Value::as_array).cloned().unwrap_or_default(), None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    let mut rows = Vec::new();
    flatten(&items, 0, &mut rows);
    let open = &cx.view.sidebar.open;
    for row in &mut rows {
        row.open = row.has_children && open.get(&row.path).copied().unwrap_or(row.is_section());
    }
    let visible = shown(&rows);
    let selected = entries_of(cx.view.font_view.entries.as_ref());
    let value = SidebarModel { has_font: true, rows, visible, selected, error, menu_row: cx.view.sidebar.menu_row.clone() };
    super::set(model, value);
}

/// The rows of `items` and their children, in order, at most [`MAX_ROWS`] and [`MAX_DEPTH`] deep.
fn flatten(items: &[Value], depth: usize, out: &mut Vec<SidebarRow>) {
    if depth > MAX_DEPTH {
        return;
    }
    for item in items {
        if out.len() >= MAX_ROWS {
            return;
        }
        let strings = |key: &str| -> Vec<String> {
            item.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
        };
        let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
        let number = |key: &str| item.get(key).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
        let path = strings("path");
        if path.is_empty() {
            continue;
        }
        let children = item.get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        out.push(SidebarRow {
            label: text("label").unwrap_or_else(|| path.last().cloned().unwrap_or_default()),
            path,
            kind: text("kind").unwrap_or_default(),
            count: number("count").unwrap_or(0),
            present: number("present"),
            expected: number("expected"),
            problem: text("problem"),
            depth,
            has_children: !children.is_empty(),
            open: false,
        });
        flatten(children, depth + 1, out);
    }
}

/// The rows whose parents are all open.
fn shown(rows: &[SidebarRow]) -> Vec<usize> {
    let mut out = Vec::with_capacity(rows.len());
    let mut closed_at: Option<usize> = None;
    for (at, row) in rows.iter().enumerate() {
        if closed_at.is_some_and(|depth| row.depth > depth) {
            continue;
        }
        closed_at = (row.has_children && !row.open).then_some(row.depth);
        out.push(at);
    }
    out
}

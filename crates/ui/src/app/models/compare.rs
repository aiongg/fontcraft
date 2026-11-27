//! The view models of Compare Fonts and Compare Family. A hidden panel's model is left as it was, so its queries run
//! only while it shows.

use craft::ui::contract::{PlainData, Tracked};
use serde_json::{Map, Value, json};

use super::ProjectCx;
use crate::shell::compare_fonts::{CATEGORIES, CompareSide, side_value};
use crate::shell::dock::PanelId;

/// Characters of a value the panels show; a longer value ends in an ellipsis.
const MAX_SHOWN_CHARS: usize = 200;
/// Rows the Compare Fonts panel holds.
const MAX_ROWS: usize = 100_000;

fn shows(cx: &ProjectCx<'_>, panel: PanelId) -> bool {
    cx.view.workspace.layout().panel_state(&panel) != craft::behavior::dock::PanelState::Hidden
}

fn clip(s: String) -> String {
    if s.chars().count() <= MAX_SHOWN_CHARS {
        return s;
    }
    let mut out: String = s.chars().take(MAX_SHOWN_CHARS).collect();
    out.push('…');
    out
}

/// Whole numbers without decimals; other values as JSON.
fn number_text(v: &Value) -> String {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 => format!("{f:.0}"),
            Some(f) => format!("{f:.2}"),
            None => n.to_string(),
        },
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// ---------- Compare Fonts ----------

/// An open document as the side pickers list it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct DocChoice {
    pub title: String,
    /// `(id, name)` of each master.
    pub masters: Vec<(String, String)>,
}

/// One row: a `compareFonts.diff` row with its values as text.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct DiffRow {
    pub category: String,
    pub key: String,
    /// The row's name; a glyph row adds what differs.
    pub label: String,
    /// `None` where that side has no value.
    pub left: Option<String>,
    pub right: Option<String>,
    /// A kerning row's sides.
    pub first: Option<String>,
    pub second: Option<String>,
}

/// A line of the list: a category heading or a row.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLine {
    /// A [`CATEGORIES`] position and its row count.
    Heading(usize, usize),
    /// A position in [`CompareFontsModel::rows`].
    Row(usize),
}

/// What the Compare Fonts panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CompareFontsModel {
    pub documents: Vec<DocChoice>,
    /// Each side's document position and master id (empty: the first master).
    pub left: Option<(usize, String)>,
    pub right: Option<(usize, String)>,
    pub rows: Vec<DiffRow>,
    /// The headings and the rows of the expanded categories, in list order.
    pub lines: Vec<DiffLine>,
    pub collapsed: Vec<String>,
    /// The selected row's position in `rows`, while it is listed.
    pub selected: Option<usize>,
    pub truncated: bool,
    pub error: Option<String>,
    /// Each side's document id, revision and master the rows came from.
    pub asked: Option<[(u64, u64, String); 2]>,
}

impl DiffRow {
    fn from_diff(v: &Value) -> Option<DiffRow> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let category = text("category")?;
        let key = text("key")?;
        let mut label = text("label").unwrap_or_else(|| key.clone());
        if let Some(d) = v.get("differences").and_then(Value::as_array) {
            let names: Vec<&str> = d.iter().filter_map(Value::as_str).collect();
            if !names.is_empty() {
                label = format!("{label} ({})", names.join(", "));
            }
        }
        Some(DiffRow { category, key, label, left: shown(v.get("left")), right: shown(v.get("right")), first: text("first"), second: text("second") })
    }
}

/// A value as the panel shows it; `None` for no value.
fn shown(v: Option<&Value>) -> Option<String> {
    let text = match v? {
        Value::Null => return None,
        Value::String(s) => s.clone(),
        Value::Object(o) if o.contains_key("paths") => glyph_summary(o),
        other => number_text(other),
    };
    Some(clip(text))
}

/// A glyph row's side: width, paths, nodes, components and anchors.
fn glyph_summary(o: &Map<String, Value>) -> String {
    let n = |k: &str| o.get(k).map(number_text).unwrap_or_default();
    let mut parts = vec![format!("width {}", n("width")), format!("{} paths", n("paths")), format!("{} nodes", n("nodes"))];
    let list = |k: &str| o.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let components: Vec<&str> = list("components").iter().filter_map(Value::as_str).collect();
    if !components.is_empty() {
        parts.push(format!("components {}", components.join(" ")));
    }
    let anchors: Vec<String> = list("anchors")
        .iter()
        .map(|a| {
            let name = a.get("name").and_then(Value::as_str).unwrap_or_default();
            let at = |k: &str| a.get(k).map(number_text).unwrap_or_default();
            format!("{name} {} {}", at("x"), at("y"))
        })
        .collect();
    if !anchors.is_empty() {
        parts.push(format!("anchors {}", anchors.join("; ")));
    }
    parts.join(", ")
}

/// The list's lines: each heading, then its rows unless it is collapsed.
fn lines(rows: &[DiffRow], collapsed: &[String]) -> Vec<DiffLine> {
    let mut by_category: [Vec<usize>; 4] = Default::default();
    for (i, r) in rows.iter().enumerate() {
        if let Some(c) = CATEGORIES.iter().position(|id| *id == r.category)
            && let Some(list) = by_category.get_mut(c)
        {
            list.push(i);
        }
    }
    let mut out = Vec::with_capacity(rows.len() + CATEGORIES.len());
    for (c, (id, list)) in CATEGORIES.iter().zip(by_category).enumerate() {
        out.push(DiffLine::Heading(c, list.len()));
        if !collapsed.iter().any(|x| x == id) {
            out.extend(list.into_iter().map(DiffLine::Row));
        }
    }
    out
}

fn stamp(cx: &ProjectCx<'_>, side: &CompareSide) -> (u64, u64, String) {
    let revision = cx.session.documents().iter().find(|d| d.id == side.document).map_or(u64::MAX, |d| d.revision);
    (side.document, revision, side.master.clone())
}

fn position(cx: &ProjectCx<'_>, document: u64) -> Option<usize> {
    cx.session.documents().iter().position(|d| d.id == document)
}

/// Asks `compareFonts.diff` again while the panel shows and a side or one of its documents moved. Runs on every
/// projection, outside the stamp, since an edit to the document that is not active moves no stamp.
pub fn project_fonts(model: &mut Tracked<CompareFontsModel>, cx: &mut ProjectCx<'_>) {
    if !shows(cx, PanelId::CompareFonts) {
        return;
    }
    let st = &cx.view.compare_fonts;
    let (Some(left), Some(right)) = (st.left.clone(), st.right.clone()) else {
        if model.asked.is_some() || !model.rows.is_empty() {
            super::set(model, CompareFontsModel::default());
        }
        return;
    };
    let asked = Some([stamp(cx, &left), stamp(cx, &right)]);
    let mut next = (**model).clone();
    if next.asked != asked {
        next.asked = asked;
        next.documents = cx
            .session
            .documents()
            .iter()
            .map(|d| DocChoice { title: d.title.clone(), masters: d.font.masters.iter().map(|m| (m.id.clone(), m.name.clone())).collect() })
            .collect();
        let (l, r) = (position(cx, left.document), position(cx, right.document));
        next.left = l.map(|i| (i, left.master.clone()));
        next.right = r.map(|i| (i, right.master.clone()));
        match (l, r) {
            (Some(l), Some(r)) => {
                let params = json!({ "left": side_value(l, &left.master), "right": side_value(r, &right.master) });
                match cx.session.query("compareFonts.diff", &params) {
                    Ok(out) => {
                        let listed = out.get("rows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
                        next.rows = listed.iter().take(MAX_ROWS).filter_map(DiffRow::from_diff).collect();
                        next.truncated = out.get("truncated").and_then(Value::as_bool).unwrap_or(false) || listed.len() > MAX_ROWS;
                        next.error = None;
                    }
                    Err(e) => {
                        next.rows.clear();
                        next.truncated = false;
                        next.error = Some(e.to_string());
                    }
                }
            }
            _ => {
                next.rows.clear();
                next.error = Some("A compared document was closed.".into());
            }
        }
    }
    next.collapsed = st.collapsed.clone();
    next.lines = lines(&next.rows, &next.collapsed);
    next.selected = st.selected.as_ref().and_then(|s| next.rows.iter().position(|r| r.category == s.category && r.key == s.key));
    super::set(model, next);
}

// ---------- Compare Family ----------

/// One column: the document it was read from.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FamilyColumn {
    /// The document's position when the table was read.
    pub document: usize,
    pub title: String,
    pub master: String,
}

/// One cell: its text, or a missing glyph.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FamilyCell {
    pub text: String,
    /// The document has no such glyph.
    pub missing: bool,
    /// The glyph has no value for the attribute.
    pub empty: bool,
}

/// One row: the glyph and its cells.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FamilyRow {
    pub glyph: String,
    pub cells: Vec<FamilyCell>,
    pub same: bool,
}

/// What the Compare Family panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CompareFamilyModel {
    pub attribute: String,
    pub filter: String,
    pub pinned: bool,
    pub columns: Vec<FamilyColumn>,
    /// Glyphs the filter keeps; `rows` holds at most the report's limit of them.
    pub total: usize,
    pub rows: Vec<FamilyRow>,
    pub error: Option<String>,
    /// The state's read count the table came from; `None` before the first read.
    pub read: Option<u64>,
}

fn cell(v: &Value) -> FamilyCell {
    if v.get("missing").and_then(Value::as_bool).unwrap_or(false) {
        return FamilyCell { missing: true, ..FamilyCell::default() };
    }
    let text = match v {
        Value::Null => return FamilyCell { empty: true, ..FamilyCell::default() },
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).collect::<Vec<_>>().join(", "),
        // A glyph cell: its advance width.
        Value::Object(o) => format!("width {}", o.get("width").map(number_text).unwrap_or_default()),
        other => other.to_string(),
    };
    FamilyCell { text: clip(text), ..FamilyCell::default() }
}

/// Reads the table with `compareFamily.report` while the panel shows and a read is due.
pub fn project_family(model: &mut Tracked<CompareFamilyModel>, cx: &mut ProjectCx<'_>) {
    if !shows(cx, PanelId::CompareFamily) {
        return;
    }
    let st = &cx.view.compare_family;
    let mut next = (**model).clone();
    next.pinned = st.pinned;
    if next.read != Some(st.reads) {
        next.read = Some(st.reads);
        next.attribute = st.attribute.clone();
        next.filter = st.filter.clone();
        match cx.session.query("compareFamily.report", &json!({ "attribute": st.attribute, "filter": st.filter })) {
            Ok(out) => {
                let docs = cx.session.documents();
                next.columns = out
                    .get("columns")
                    .and_then(Value::as_array)
                    .map(|cols| {
                        cols.iter()
                            .filter_map(|c| {
                                let document = usize::try_from(c.get("document")?.as_u64()?).ok()?;
                                let doc = docs.get(document)?;
                                let master = c.get("master").and_then(Value::as_str).unwrap_or_default().to_string();
                                Some(FamilyColumn { document, title: doc.title.clone(), master })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                next.rows = out
                    .get("rows")
                    .and_then(Value::as_array)
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|r| {
                                let glyph = r.get("glyph")?.as_str()?.to_string();
                                let same = r.get("same").and_then(Value::as_bool).unwrap_or(true);
                                let cells = r.get("cells")?.as_array()?.iter().map(cell).collect();
                                Some(FamilyRow { glyph, cells, same })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                next.total = out.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(next.rows.len());
                next.error = None;
            }
            Err(e) => {
                next.columns.clear();
                next.rows.clear();
                next.total = 0;
                next.error = Some(e.to_string());
            }
        }
    }
    super::set(model, next);
}

//! The Kerning window's view model: the stored pairs that pass the window's search, in its order, and the selection as
//! positions in that list.
//!
//! The list is read again only when the kerning, the listed master, the direction or the query moves. An edit that
//! changes values alone, in a list not sorted by value, patches those values: the names are shared with the list before.

use std::collections::HashMap;
use std::sync::Arc;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::model::Font;
use fontcraft_model::Kerning;
use fontcraft_spacing::kerning::pairs::{self, EntryKind, Filter, Match, SearchSide};
use fontcraft_spacing::metrics_source;
use fontcraft_textlayout::Direction;
use serde_json::{Map, Value, json};

use super::ProjectCx;
use crate::canvas::tokens::Tokens;
use crate::shell::dock::PanelId;
use crate::shell::kerning_window::{Column, PairSort, Search, split_id};

/// Most rows the window holds; a font has fewer pairs in one master than the file readers allow.
const MAX_ROWS: usize = 2_000_000;
/// Most values one edit patches in the list; an edit of more reads the list again.
const MAX_PATCHED: usize = 16;
/// Most ids looked up by walking the list; more build a map of it first.
const MAX_WALKED: usize = 4;

/// One listed pair: sides as `A` or `@group`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KernRow<'a> {
    pub left: &'a str,
    pub right: &'a str,
    pub value: f64,
}

/// The listed pairs. Cloning shares them, and two lists are equal when they share them.
#[derive(Clone, Debug, Default)]
pub struct KernRows {
    sides: Arc<Vec<(String, String)>>,
    values: Arc<Vec<f64>>,
    /// Each row's place in the master's stored order.
    stored: Arc<Vec<usize>>,
}

// Nothing changes behind a shared reference: the lists sit behind `Arc`s that only a rebuild replaces.
impl PlainData for KernRows {}

impl PartialEq for KernRows {
    fn eq(&self, other: &Self) -> bool {
        (self.is_empty() && other.is_empty()) || (Arc::ptr_eq(&self.sides, &other.sides) && Arc::ptr_eq(&self.values, &other.values))
    }
}

impl KernRows {
    /// The rows of `(left, right, value, place in the stored order)`, at most [`MAX_ROWS`].
    pub fn of(listed: impl IntoIterator<Item = (String, String, f64, usize)>) -> KernRows {
        let (mut sides, mut values, mut stored) = (Vec::new(), Vec::new(), Vec::new());
        for (left, right, value, at) in listed.into_iter().take(MAX_ROWS) {
            sides.push((left, right));
            values.push(value);
            stored.push(at);
        }
        KernRows { sides: Arc::new(sides), values: Arc::new(values), stored: Arc::new(stored) }
    }

    pub fn len(&self) -> usize {
        self.sides.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sides.is_empty()
    }

    pub fn get(&self, at: usize) -> Option<KernRow<'_>> {
        let (left, right) = self.sides.get(at)?;
        Some(KernRow { left, right, value: *self.values.get(at)? })
    }

    pub fn iter(&self) -> impl Iterator<Item = KernRow<'_>> {
        self.sides.iter().zip(self.values.iter()).map(|((left, right), value)| KernRow { left, right, value: *value })
    }

    /// Whether the two lists hold the same names, not only equal ones.
    #[cfg(test)]
    pub fn shares_names(&self, other: &KernRows) -> bool {
        Arc::ptr_eq(&self.sides, &other.sides)
    }

    fn walk(&self, id: &str) -> Option<usize> {
        let (left, right) = split_id(id);
        self.sides.iter().position(|(l, r)| l == left && r == right)
    }

    /// The positions of the rows `ids` name (`row_id`), in their order; ids not listed are left out.
    pub fn positions(&self, ids: &[String]) -> Vec<usize> {
        if ids.len() <= MAX_WALKED {
            return ids.iter().filter_map(|id| self.walk(id)).collect();
        }
        let at: HashMap<(&str, &str), usize> = self.sides.iter().enumerate().map(|(i, (l, r))| ((l.as_str(), r.as_str()), i)).collect();
        ids.iter().filter_map(|id| at.get(&split_id(id)).copied()).collect()
    }

    /// The rows with the values of the stored pairs `changed` (place, value) in place of theirs; the names are shared.
    fn patched(&self, changed: &[(usize, f64)]) -> KernRows {
        let mut values = Vec::clone(&self.values);
        for (stored, value) in changed {
            if let Some(slot) = self.stored.iter().position(|s| s == stored).and_then(|at| values.get_mut(at)) {
                *slot = *value;
            }
        }
        KernRows { sides: Arc::clone(&self.sides), values: Arc::new(values), stored: Arc::clone(&self.stored) }
    }
}

/// What a list was read from.
#[derive(Clone, Debug)]
pub struct Asked {
    document: u64,
    kerning: Arc<Kerning>,
    source: String,
    query: String,
    /// With the search limited to selected glyphs: the document revision, since their groups count.
    revision: Option<u64>,
}

// The kerning table is the document's shared one, which an edit replaces and never changes in place.
impl PlainData for Asked {}

impl Asked {
    /// Whether the two differ in the kerning table at most.
    fn same_list(&self, other: &Asked) -> bool {
        (self.document, &self.source, &self.query, self.revision) == (other.document, &other.source, &other.query, other.revision)
    }
}

impl PartialEq for Asked {
    fn eq(&self, other: &Self) -> bool {
        self.same_list(other) && Arc::ptr_eq(&self.kerning, &other.kerning)
    }
}

/// The values that differ between two tables of the same pairs in `master`, by place in the stored order. `None` when
/// the pairs themselves differ, or more than [`MAX_PATCHED`] values do.
fn changed_values(before: &Kerning, after: &Kerning, direction: Direction, master: &str) -> Option<Vec<(usize, f64)>> {
    let mut a = direction.table(before).0.iter().filter(|m| m.master == master).flat_map(|m| &m.firsts);
    let mut b = direction.table(after).0.iter().filter(|m| m.master == master).flat_map(|m| &m.firsts);
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        match (a.next(), b.next()) {
            (None, None) => return Some(out),
            (Some(x), Some(y)) if x.first == y.first && x.seconds.len() == y.seconds.len() => {
                for ((name, old), (other, new)) in x.seconds.iter().zip(&y.seconds) {
                    if name != other {
                        return None;
                    }
                    if old != new {
                        if out.len() >= MAX_PATCHED {
                            return None;
                        }
                        out.push((at, *new));
                    }
                    at += 1;
                }
            }
            _ => return None,
        }
    }
}

/// What the window shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct KerningWindowModel {
    pub has_doc: bool,
    /// The master the edits go to.
    pub master: Option<String>,
    /// The master whose pairs are listed (another one for a linked master).
    pub source: Option<String>,
    /// "ltr", "rtl" or "vertical".
    pub direction: String,
    /// The direction the window's control names; `None`: the one of the Edit View tab.
    pub own_direction: Option<String>,
    pub rows: KernRows,
    /// The pairs the master has in the direction, listed or not.
    pub total: usize,
    pub search: Search,
    pub sort: Option<PairSort>,
    /// Positions in `rows`, in the order they were picked.
    pub selected: Arc<Vec<usize>>,
    /// The picked ids `selected` was made from.
    pub picked: Arc<Vec<String>>,
    /// The row the steps go from.
    pub current: Option<usize>,
    /// The cell being edited in place: the row and its Value or Right column.
    pub editing: Option<(usize, Column)>,
    /// The colours of group and glyph names.
    pub group_color: [u8; 4],
    pub glyph_color: [u8; 4],
    /// What the rows were read from: the same list is not read twice.
    pub asked: Option<Asked>,
    /// What "every row" means now.
    pub scope: Option<AllRows>,
}

/// Every row of a list: the document and the query without its sort. Select All holds it, so it ends with the list it
/// was made on.
#[derive(PlainData, Clone, Debug, Default, PartialEq, Eq)]
pub struct AllRows {
    document: u64,
    query: String,
}

impl AllRows {
    /// The list as the `query` of the commands that take rows.
    pub fn query(&self) -> Value {
        let mut p = serde_json::from_str::<Map<String, Value>>(&self.query).unwrap_or_default();
        p.remove("master");
        p.remove("direction");
        Value::Object(p)
    }
}

/// The direction the window lists: its own, else the one of the Edit View tab (the tab shown last while Font View
/// shows).
pub fn direction(view: &crate::app::ViewState) -> Direction {
    view.kerning.direction.unwrap_or_else(|| view.text.direction.layout())
}

/// `kerning.listPairs`'s parameters for the window's search: the list but its sort.
fn list_params(cx: &ProjectCx<'_>, master: &str) -> String {
    let search = &cx.view.kerning.search;
    let mut p = Map::new();
    p.insert("master".into(), json!(master));
    p.insert("direction".into(), json!(direction(cx.view).name()));
    p.insert("term".into(), json!(search.term));
    p.insert("match".into(), json!(search.matching));
    if let Some(k) = &search.kind {
        p.insert("kind".into(), json!(k));
    }
    if let Some(s) = &search.side {
        p.insert("side".into(), json!(s));
    }
    if search.selected_glyphs_only {
        p.insert("glyphs".into(), json!(cx.view.font_view.selected));
    }
    Value::Object(p).to_string()
}

/// The list for `search` and `sort`, read from the font.
fn read(font: &Font, direction: Direction, source: &str, search: &Search, sort: Option<PairSort>, glyphs: &[String]) -> KernRows {
    let filter = Filter {
        term: search.term.clone(),
        matching: Match::from_name(&search.matching).unwrap_or_default(),
        kind: search.kind.as_deref().and_then(EntryKind::from_name),
        side: search.side.as_deref().and_then(SearchSide::from_name),
        glyphs: search.selected_glyphs_only.then(|| glyphs.to_vec()),
    };
    let order = sort.and_then(|s| Some((pairs::Column::from_name(s.column.id())?, s.descending)));
    let listed = pairs::list(font, direction, source, &filter, order);
    KernRows::of(listed.iter().map(|r| (r.left().command(), r.right().command(), r.value, r.index)))
}

/// How many pairs `master` stores in `direction`.
fn count(font: &Font, direction: Direction, master: &str) -> usize {
    direction.table(&font.kerning).0.iter().filter(|m| m.master == master).flat_map(|m| &m.firsts).map(|f| f.seconds.len()).sum()
}

/// Brings the list up to date while the window shows; the selection keeps the rows still listed. A hidden window holds
/// no list: what it listed may belong to another document by the time a command reads it.
pub fn project(model: &mut Tracked<KerningWindowModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout.panel_state(&PanelId::KerningWindow) == craft::behavior::dock::PanelState::Hidden {
        super::set(model, KerningWindowModel::default());
        return;
    }
    let direction = direction(cx.view);
    let kerning = &cx.view.kerning;
    let doc = cx.session.active().map(|d| (d.id, d.revision));
    let current = cx.session.font().ok().and_then(|f| crate::shell::masters::current_master(f, &cx.view.layers)).map(|m| m.id.clone());
    let mut next = (**model).clone();
    next.search = kerning.search.clone();
    next.sort = kerning.sort;
    next.direction = direction.name().to_string();
    next.own_direction = kerning.direction.map(|d| d.name().to_string());
    let (Some((document, revision)), Some(master), Ok(font)) = (doc, current, cx.session.font()) else {
        let KerningWindowModel { search, sort, direction, own_direction, .. } = next;
        super::set(model, KerningWindowModel { has_doc: doc.is_some(), search, sort, direction, own_direction, ..Default::default() });
        return;
    };
    let source = metrics_source(font, &master).map_or_else(|| master.clone(), |m| m.id.clone());
    let query = list_params(cx, &master);
    let asked = Asked {
        document,
        kerning: Arc::clone(&font.kerning),
        source,
        query: format!("{query} {:?}", kerning.sort),
        revision: kerning.search.selected_glyphs_only.then_some(revision),
    };
    let before = next.asked.replace(asked.clone());
    let mut listed_anew = before.as_ref() != Some(&asked);
    if listed_anew {
        let by_value = kerning.sort.is_some_and(|s| s.column == Column::Value);
        let patch = before
            .as_ref()
            .filter(|b| !by_value && b.same_list(&asked))
            .and_then(|b| changed_values(&b.kerning, &asked.kerning, direction, &asked.source));
        match patch {
            Some(changed) if changed.is_empty() => listed_anew = false,
            Some(changed) => {
                next.rows = next.rows.patched(&changed);
                // The rows stand where they stood.
                listed_anew = false;
            }
            None => {
                next.rows = read(font, direction, &asked.source, &kerning.search, kerning.sort, &cx.view.font_view.selected);
                next.total = count(font, direction, &asked.source);
            }
        }
        next.has_doc = true;
        next.scope = Some(AllRows { document, query });
        next.master = Some(master);
        next.source = Some(asked.source);
    }
    if kerning.all.is_some() && kerning.all == next.scope {
        if next.selected.len() != next.rows.len() || listed_anew {
            next.selected = Arc::new((0..next.rows.len()).collect());
        }
        next.picked = Arc::default();
    } else if listed_anew || !Arc::ptr_eq(&next.picked, &kerning.selected) {
        next.selected = Arc::new(next.rows.positions(&kerning.selected));
        next.picked = Arc::clone(&kerning.selected);
    }
    next.current = kerning.current.as_ref().and_then(|id| next.rows.walk(id));
    let edited = kerning.editing.as_ref().filter(|_| kerning.editing_in == Some(document));
    next.editing = edited.and_then(|(id, column)| Some((next.rows.walk(id)?, *column)));
    let tokens = Tokens::from_settings(cx.session.settings(), cx.view.system_dark);
    next.group_color = tokens.kerning_group_text.to_srgba_unmultiplied();
    next.glyph_color = tokens.kerning_glyph_text.to_srgba_unmultiplied();
    super::set(model, next);
}

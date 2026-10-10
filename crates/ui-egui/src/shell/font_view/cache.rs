//! What Font View lists. The cache is rebuilt only when something it depends on changed: the
//! document or its revision, the user settings, the glyph data, Font View's master, the query
//! (sidebar and search) or the List View sort. Effective properties are kept across rebuilds
//! for every glyph that did not change, so an edit of one glyph recomputes one glyph's.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, GlyphProps, glyph_order, glyph_order_sections, glyph_props, keeps_alternates};
use serde_json::Value;

use super::columns::{CellCx, SortValue};
use super::{FontViewState, Sort};

/// One glyph as Font View lists it.
pub struct Row {
    pub glyph: Arc<Glyph>,
    /// The position in glyph order: the ID column, and the export order.
    pub id: usize,
    pub props: GlyphProps,
}

/// What the rows were built from.
#[derive(Clone, PartialEq)]
struct RowsKey {
    doc: (u64, u64),
    data: usize,
    keep_alternates: bool,
}

/// What the visible list was built from.
#[derive(Clone, PartialEq)]
struct Key {
    rows: RowsKey,
    settings: u64,
    master: String,
    query: Option<Value>,
    sort: Option<Sort>,
}

#[derive(Default)]
pub struct FontViewCache {
    key: Option<Key>,
    /// Held so its address stays unique while the key names it.
    data: Option<Arc<GlyphData>>,
    /// Every glyph, in glyph order.
    pub rows: Vec<Row>,
    by_name: HashMap<String, usize>,
    /// The rows shown, as indices into `rows`, in display order.
    pub visible: Vec<usize>,
    /// Section headers of the `glyphOrder` parameter: the position in glyph order each one
    /// precedes, and its title.
    pub sections: Vec<(usize, String)>,
    /// The master the rows are drawn in.
    pub master: String,
    /// `appearance.displayUnicodeValue`.
    pub display_unicode: bool,
    /// Why the query failed; every glyph is listed then.
    pub error: Option<String>,
    /// Bumped by every rebuild of the visible list; layouts built from it compare it.
    pub generation: u64,
}

/// The master `wanted` names, if the font has it, else the first master.
pub fn resolve_master(font: &Font, wanted: Option<&str>) -> Option<String> {
    wanted.filter(|id| font.masters.iter().any(|m| m.id == *id)).or_else(|| font.masters.first().map(|m| m.id.as_str())).map(str::to_string)
}

impl FontViewCache {
    /// The row of the glyph named `name`.
    pub fn row_of(&self, name: &str) -> Option<&Row> {
        self.by_name.get(name).and_then(|&i| self.rows.get(i))
    }

    /// The visible row at display position `at`.
    pub fn visible_row(&self, at: usize) -> Option<&Row> {
        self.visible.get(at).and_then(|&i| self.rows.get(i))
    }

    /// The section headers to draw: only while the view lists every glyph in glyph order.
    pub fn shown_sections(&self, state: &FontViewState) -> &[(usize, String)] {
        if state.query.is_none() && state.sort.is_none() && self.error.is_none() { &self.sections } else { &[] }
    }

    /// Brings the cache up to date with the session and Font View's state.
    pub fn refresh(&mut self, session: &mut Session, state: &FontViewState) {
        let Some(doc) = session.active() else {
            *self = FontViewCache { generation: self.generation + 1, ..FontViewCache::default() };
            return;
        };
        let data = match session.glyph_data() {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let font = doc.font.clone();
        let rows_key = RowsKey { doc: (doc.id, doc.revision), data: Arc::as_ptr(&data) as usize, keep_alternates: keeps_alternates(&font) };
        let master = resolve_master(&font, state.master.as_deref()).unwrap_or_default();
        let key = Key { rows: rows_key.clone(), settings: session.settings().revision(), master, query: state.query.clone(), sort: state.sort };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if self.key.as_ref().map(|k| &k.rows) != Some(&rows_key) {
            self.rebuild_rows(&font, &data, &rows_key);
        }
        self.data = Some(data);
        self.master = key.master.clone();
        self.display_unicode = session.settings().get_bool("appearance.displayUnicodeValue").unwrap_or(false);
        self.error = None;
        self.visible = match &state.query {
            None => (0..self.rows.len()).collect(),
            Some(query) => match self.query(session, query, &key.master) {
                Ok(v) => v,
                Err(e) => {
                    self.error = Some(e);
                    (0..self.rows.len()).collect()
                }
            },
        };
        if let Some(sort) = state.sort {
            self.sort(&font, sort);
        }
        self.generation += 1;
        self.key = Some(key);
    }

    fn rebuild_rows(&mut self, font: &Font, data: &Arc<GlyphData>, key: &RowsKey) {
        let same_data = self.key.as_ref().is_some_and(|k| k.rows.data == key.data && k.rows.keep_alternates == key.keep_alternates);
        let mut old: HashMap<String, Row> =
            if same_data { std::mem::take(&mut self.rows).into_iter().map(|r| (r.glyph.name.clone(), r)).collect() } else { HashMap::new() };
        let glyphs = font.glyphs.as_arcs();
        let order = glyph_order(font);
        let mut rows = Vec::with_capacity(order.len());
        for (id, &i) in order.iter().enumerate() {
            let Some(glyph) = glyphs.get(i) else { continue };
            let props = match old.remove(&glyph.name) {
                // A glyph that did not change keeps its properties.
                Some(row) if Arc::ptr_eq(&row.glyph, glyph) => row.props,
                _ => glyph_props(glyph, data, key.keep_alternates),
            };
            rows.push(Row { glyph: glyph.clone(), id, props });
        }
        self.by_name = rows.iter().enumerate().map(|(i, r)| (r.glyph.name.clone(), i)).collect();
        self.rows = rows;
        self.sections = glyph_order_sections(font);
    }

    /// The rows `fontView.queryGlyphs` gives for `query`, in its order.
    fn query(&self, session: &mut Session, query: &Value, master: &str) -> Result<Vec<usize>, String> {
        let mut params = query.as_object().cloned().ok_or_else(|| "the Font View query must be an object".to_string())?;
        if !master.is_empty() {
            params.entry("master").or_insert_with(|| Value::String(master.to_string()));
        }
        let result = session.execute("fontView.queryGlyphs", &Value::Object(params)).map_err(|e| e.to_string())?;
        let names = result.get("glyphs").and_then(Value::as_array).ok_or_else(|| "fontView.queryGlyphs gave no glyph list".to_string())?;
        Ok(names.iter().filter_map(Value::as_str).filter_map(|n| self.by_name.get(n).copied()).collect())
    }

    /// Sorts the visible rows by a List View column; ties keep glyph order. Rows without a
    /// value go last in either direction.
    fn sort(&mut self, font: &Font, sort: Sort) {
        let cx = CellCx { font, master: &self.master };
        let mut keyed: Vec<(SortValue, usize)> =
            self.visible.iter().filter_map(|&i| self.rows.get(i).map(|r| (sort.column.sort_value(&cx, r), i))).collect();
        keyed.sort_by(|(a, ai), (b, bi)| {
            let order = match (a, b) {
                (SortValue::None, SortValue::None) => std::cmp::Ordering::Equal,
                (SortValue::None, _) => return std::cmp::Ordering::Greater,
                (_, SortValue::None) => return std::cmp::Ordering::Less,
                _ => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
            };
            let order = if sort.ascending { order } else { order.reverse() };
            order.then(ai.cmp(bi))
        });
        self.visible = keyed.into_iter().map(|(_, i)| i).collect();
    }
}

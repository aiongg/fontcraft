//! The Glyph Info view model: the rows `glyphData.search` finds for the panel's query and sort, and the selected ones.
//! The search runs again only when the query, the sort or the glyph data changed, and not while the panel is hidden.

use std::collections::HashMap;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use serde_json::Value;

use super::ProjectCx;
use crate::shell::dock::PanelId;
use crate::shell::glyph_info::{Column, search_params};

/// One result of the search, as the table shows it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct InfoRow {
    pub name: String,
    pub unicode: String,
    pub category: String,
    pub sub_category: String,
    pub script: String,
    pub description: String,
}

impl InfoRow {
    fn of(v: &Value) -> Option<InfoRow> {
        let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        Some(InfoRow {
            name: v.get("name").and_then(Value::as_str)?.to_string(),
            unicode: text("unicode"),
            category: text("category"),
            sub_category: text("subCategory"),
            script: text("script"),
            description: text("description"),
        })
    }

    pub fn cell(&self, c: Column) -> &str {
        match c {
            Column::Name => &self.name,
            Column::Unicode => &self.unicode,
            Column::Category => &self.category,
            Column::SubCategory => &self.sub_category,
            Column::Script => &self.script,
            Column::Description => &self.description,
        }
    }
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GlyphInfoModel {
    pub query: String,
    /// The sorted column; `None`: by name.
    pub sort_by: Option<Column>,
    pub rows: Vec<InfoRow>,
    /// The selected rows, as indices into `rows`, in the order they were picked.
    pub selected: Vec<usize>,
    /// Why the search was refused.
    pub error: Option<String>,
    /// A font is open: Add to Font has somewhere to add.
    pub has_font: bool,
}

/// The last search: what it was run for and what it found.
#[derive(Default)]
pub struct SearchCache {
    key: Option<(String, Option<Column>, u64)>,
    rows: Vec<InfoRow>,
    error: Option<String>,
}

pub fn project(model: &mut Tracked<GlyphInfoModel>, cache: &mut SearchCache, cx: &mut ProjectCx<'_>) {
    let state = &cx.view.glyph_info;
    let has_font = cx.session.font().is_ok();
    if cx.view.workspace.layout.panel_state(&PanelId::GlyphInfo) == PanelState::Hidden {
        super::set(model, GlyphInfoModel { query: state.query.clone(), sort_by: state.sort_by, has_font, ..GlyphInfoModel::default() });
        return;
    }
    let key = (state.query.clone(), state.sort_by, cx.session.glyph_data_revision());
    if cache.key.as_ref() != Some(&key) {
        match cx.session.query("glyphData.search", &search_params(&state.query, state.sort_by)) {
            Ok(v) => {
                let rows = v.get("results").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
                *cache = SearchCache { key: None, rows: rows.iter().filter_map(InfoRow::of).collect(), error: None };
            }
            Err(e) => *cache = SearchCache { key: None, rows: Vec::new(), error: Some(e.to_string()) },
        }
        cache.key = Some(key);
    }
    let at: HashMap<&str, usize> = cache.rows.iter().enumerate().map(|(i, r)| (r.name.as_str(), i)).collect();
    let selected = state.selected.iter().filter_map(|n| at.get(n.as_str()).copied()).collect();
    let value = GlyphInfoModel {
        query: state.query.clone(),
        sort_by: state.sort_by,
        rows: cache.rows.clone(),
        selected,
        error: cache.error.clone(),
        has_font,
    };
    super::set(model, value);
}

//! The Kerning window's view model: the pairs `kerning.listPairs` lists for the window's search and sort, and the
//! selection as positions in that list.

use std::collections::HashMap;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_textlayout::Direction;
use serde_json::{Map, Value, json};

use super::ProjectCx;
use crate::canvas::tokens::Tokens;
use crate::shell::dock::PanelId;
use crate::shell::kerning_window::{Column, PairSort, Search, row_id};

/// Most rows the window holds; a font has fewer pairs in one master than the file readers allow.
const MAX_ROWS: usize = 2_000_000;

/// One listed pair: sides as `A` or `@group`.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct KernRow {
    pub left: String,
    pub right: String,
    pub value: f64,
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
    pub rows: Vec<KernRow>,
    /// The pairs the master has in the direction, listed or not.
    pub total: usize,
    pub search: Search,
    pub sort: Option<PairSort>,
    /// Positions in `rows`, in the order they were picked.
    pub selected: Vec<usize>,
    /// The row the steps go from.
    pub current: Option<usize>,
    /// The cell being edited in place: the row and its Value or Right column.
    pub editing: Option<(usize, Column)>,
    /// The colours of group and glyph names.
    pub group_color: [u8; 4],
    pub glyph_color: [u8; 4],
    /// Why the list could not be read.
    pub error: Option<String>,
    /// The document revision and the query the rows came from: the same pair is not asked for twice.
    pub asked: Option<(u64, u64, String)>,
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

/// The direction the window lists: its own, else the one of the Edit View tab (the tab shown last while Font View
/// shows).
pub fn direction(view: &crate::app::ViewState) -> Direction {
    view.kerning.direction.unwrap_or_else(|| view.text.direction.layout())
}

/// `kerning.listPairs`'s parameters for the window's search and sort.
fn list_params(cx: &ProjectCx<'_>, master: Option<&str>) -> Value {
    let kerning = &cx.view.kerning;
    let search = &kerning.search;
    let mut p = Map::new();
    if let Some(m) = master {
        p.insert("master".into(), json!(m));
    }
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
    if let Some(sort) = kerning.sort {
        p.insert("sort".into(), json!(sort.column.id()));
        p.insert("descending".into(), json!(sort.descending));
    }
    Value::Object(p)
}

fn without_sort(params: &Value) -> String {
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.remove("sort");
    p.remove("descending");
    Value::Object(p).to_string()
}

/// Lists the pairs again while the window shows and the document, the search or the sort moved; the selection
/// keeps the rows still listed.
pub fn project(model: &mut Tracked<KerningWindowModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout.panel_state(&PanelId::KerningWindow) == craft::behavior::dock::PanelState::Hidden {
        return;
    }
    let mut next = (**model).clone();
    let Some(doc) = cx.session.active().map(|d| (d.id, d.revision)) else {
        let empty = KerningWindowModel {
            search: cx.view.kerning.search.clone(),
            sort: cx.view.kerning.sort,
            direction: direction(cx.view).name().to_string(),
            own_direction: cx.view.kerning.direction.map(|d| d.name().to_string()),
            ..Default::default()
        };
        super::set(model, empty);
        return;
    };
    let master = cx.session.font().ok().and_then(|f| crate::shell::masters::current_master(f, &cx.view.layers)).map(|m| m.id.clone());
    let params = list_params(cx, master.as_deref());
    let asked = Some((doc.0, doc.1, params.to_string()));
    if next.asked != asked {
        next.asked = asked;
        next.has_doc = true;
        next.direction = direction(cx.view).name().to_string();
        match cx.session.query("kerning.listPairs", &params) {
            Ok(v) => {
                let listed = v.get("pairs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
                next.rows = listed
                    .iter()
                    .take(MAX_ROWS)
                    .filter_map(|r| {
                        Some(KernRow {
                            left: r.get("left")?.as_str()?.to_string(),
                            right: r.get("right")?.as_str()?.to_string(),
                            value: r.get("value")?.as_f64().filter(|n| n.is_finite())?,
                        })
                    })
                    .collect();
                let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
                next.total = v.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(next.rows.len());
                next.scope = Some(AllRows { document: doc.0, query: without_sort(&params) });
                next.master = text("master");
                next.source = text("source");
                next.error = None;
            }
            Err(e) => {
                next.rows.clear();
                next.total = 0;
                next.scope = None;
                next.error = Some(e.to_string());
            }
        }
    }
    let kerning = &cx.view.kerning;
    next.search = kerning.search.clone();
    next.sort = kerning.sort;
    next.own_direction = kerning.direction.map(|d| d.name().to_string());
    let at: HashMap<String, usize> = next.rows.iter().enumerate().map(|(i, r)| (row_id(&r.left, &r.right), i)).collect();
    next.selected = if kerning.all.is_some() && kerning.all == next.scope {
        (0..next.rows.len()).collect()
    } else {
        kerning.selected.iter().filter_map(|id| at.get(id).copied()).collect()
    };
    next.current = kerning.current.as_ref().and_then(|id| at.get(id).copied());
    next.editing = kerning.editing.as_ref().and_then(|(id, column)| Some((at.get(id).copied()?, *column)));
    let tokens = Tokens::from_settings(cx.session.settings(), cx.view.system_dark);
    next.group_color = tokens.kerning_group_text.to_srgba_unmultiplied();
    next.glyph_color = tokens.kerning_glyph_text.to_srgba_unmultiplied();
    super::set(model, next);
}

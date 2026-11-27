//! The Table Viewer's view model: what `tableViewer.show` read, copied out of `ViewState::table_viewer` while the
//! panel shows. The texts are shared, not copied; the tree is flattened into the rows in view.

use std::sync::Arc;

use craft::ui::contract::{PlainData, Tracked};

use super::ProjectCx;
use crate::shell::dock::PanelId;
use crate::shell::table_viewer::{Pane, TableViewerState, Text, byte_range, hex_line, shown, table_length, visible_rows};

/// Tree rows the panel lists; more are reached by searching or closing records.
pub const MAX_SHOWN_ROWS: usize = 20_000;

/// One row of the table list.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TableItem {
    pub tag: String,
    pub length: u64,
    pub checksum: String,
    pub decoded: bool,
    pub error: Option<String>,
    /// With a second font: `same`, `different`, `onlyA` or `onlyB`.
    pub status: Option<String>,
    pub first_offset: Option<u64>,
    /// False for a table only the compared font has.
    pub in_font: bool,
}

/// One tree row in view.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TreeRow {
    /// The node in the held tree: the list's key.
    pub id: u32,
    /// Child positions from the table's root: the `record` parameter.
    pub record: Vec<u32>,
    /// 0 for a child of the table.
    pub depth: usize,
    pub name: String,
    pub kind: String,
    pub value: Option<String>,
    /// Children not read yet.
    pub more: u64,
    pub has_children: bool,
    pub open: bool,
}

/// A text pane: the text, or why there is none. `hits` are the matching lines while searching.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TextPane {
    pub text: Option<Arc<Text>>,
    pub note: Option<String>,
    pub hits: Option<Vec<u32>>,
}

/// What the panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TableViewerModel {
    /// The state's generation the model was copied at.
    pub generation: u64,
    pub font: String,
    pub compare_with: String,
    pub table: String,
    /// `fields`, `ttx` or `differences`.
    pub view: String,
    pub search: String,
    pub tables: Vec<TableItem>,
    /// With a second font: whether the fonts have the same tables, and the tables that differ.
    pub identical: Option<bool>,
    pub differing: Vec<String>,
    pub rows: Vec<TreeRow>,
    /// Records were left out: the engine's limit, the held limit, or [`MAX_SHOWN_ROWS`].
    pub rows_cut: bool,
    /// Why the Fields view has no tree.
    pub tree_note: Option<String>,
    pub selected: Option<u32>,
    /// The selected record's bytes in the table: start, end.
    pub bytes: Option<(u64, u64)>,
    /// The hex lines the selected record covers.
    pub marked: Option<(usize, usize)>,
    pub hex: TextPane,
    /// The TTX text or the Differences text, as `view` says.
    pub text: TextPane,
}

/// Copies the viewer's state while the panel shows and the state moved.
pub fn project(model: &mut Tracked<TableViewerModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout().panel_state(&PanelId::TableViewer) == craft::behavior::dock::PanelState::Hidden {
        return;
    }
    let st = &cx.view.table_viewer;
    if model.generation == st.generation && st.generation != 0 {
        return;
    }
    super::set(model, copy(st));
}

fn text_pane(pane: Option<&Pane<Text>>, hits: &[u32], searching: bool) -> TextPane {
    match pane {
        Some(Pane::Ready(text)) => TextPane { text: Some(Arc::clone(text)), note: None, hits: searching.then(|| hits.to_vec()) },
        Some(Pane::Note(note)) => TextPane { note: Some(note.clone()), ..TextPane::default() },
        None => TextPane::default(),
    }
}

/// The model of `st`.
pub fn copy(st: &TableViewerState) -> TableViewerModel {
    let mut m = TableViewerModel {
        generation: st.generation,
        font: st.font.clone(),
        compare_with: st.compare_with.clone(),
        table: st.table.clone(),
        view: st.view.clone(),
        search: st.search.clone(),
        selected: st.selected,
        ..TableViewerModel::default()
    };
    let Some(held) = &st.data else { return m };
    let data = &held.0;
    m.tables = data
        .tables
        .iter()
        .map(|t| TableItem {
            tag: t.tag.clone(),
            length: t.length,
            checksum: t.checksum.clone(),
            decoded: t.decoded,
            error: t.error.clone(),
            status: t.status.clone(),
            first_offset: t.first_offset,
            in_font: t.in_font,
        })
        .collect();
    if let Some(c) = &data.compare {
        m.identical = Some(c.identical);
        m.differing = c.differing.clone();
    }
    let searching = !st.search.is_empty();
    match st.view.as_str() {
        "ttx" => m.text = text_pane(data.ttx.as_ref(), &data.ttx_hits, searching),
        "differences" => m.text = text_pane(data.diff.as_ref(), &data.diff_hits, searching),
        _ => {
            match &data.tree {
                Some(Pane::Ready(tree)) => {
                    let hits = searching.then_some(data.tree_hits.as_slice());
                    let visible = visible_rows(tree, &st.expanded, hits, MAX_SHOWN_ROWS);
                    m.rows_cut = tree.truncated || visible.len() >= MAX_SHOWN_ROWS;
                    m.rows = visible
                        .into_iter()
                        .filter_map(|(id, record)| {
                            let node = tree.nodes.get(id as usize)?;
                            let has_children = !node.children.is_empty() || node.more > 0;
                            let open = st.expanded.contains(&id) || (searching && !node.children.is_empty());
                            Some(TreeRow {
                                id,
                                depth: usize::from(node.depth.saturating_sub(1)),
                                record,
                                name: node.name.clone(),
                                kind: node.kind.clone(),
                                value: node.value.as_deref().map(|v| shown(v).into_owned()),
                                more: if node.children.is_empty() { node.more } else { 0 },
                                has_children,
                                open,
                            })
                        })
                        .collect();
                    m.bytes = st.selected.and_then(|id| byte_range(tree, id, table_length(data, &st.table)));
                    m.marked = m.bytes.map(|(start, end)| (hex_line(start), hex_line(end.saturating_sub(1).max(start))));
                }
                Some(Pane::Note(note)) => m.tree_note = Some(note.clone()),
                None => {}
            }
            m.hex = text_pane(data.hex.as_ref(), &[], false);
        }
    }
    m
}

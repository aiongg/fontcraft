//! The view model of Compare Fonts in List (`panels::compare_in_list`): the table of one property across the compared
//! documents, built with `compare_table` while the panel shows, again only when a compared document, the property or
//! Only Differences changes.

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::cmd::encodings::compare_table;
use fontcraft_model::Font;
use serde_json::Value;

use super::{ProjectCx, set};
use crate::shell::dock::PanelId;

/// Rows the model holds; `total` counts the rest.
pub const MAX_ROWS: usize = 100_000;

/// One glyph: its value in each compared font as text (`None`: the font has no such glyph or no value).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CompareListRow {
    pub glyph: String,
    pub values: Vec<Option<String>>,
    /// The fonts' values differ.
    pub differ: bool,
}

/// What the Compare Fonts in List panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CompareListModel {
    /// The compared documents' titles, in column order.
    pub fonts: Vec<String>,
    /// Their positions in the open-document list, for the commands the panel posts.
    pub positions: Vec<usize>,
    /// A `COMPARE_PROPERTIES` id.
    pub property: String,
    pub only_differences: bool,
    /// The rows shown: every glyph, or with Only Differences those whose values differ; at most [`MAX_ROWS`].
    pub rows: Vec<CompareListRow>,
    /// Rows before the cap.
    pub total: usize,
    pub error: Option<String>,
    /// Each compared document's id and revision the table was built from.
    pub built: Vec<(u64, u64)>,
}

/// A value as the table shows it: whole numbers without decimals, others with two; nothing for a missing value.
fn shown(v: &Value) -> Option<String> {
    Some(match v {
        Value::Null => return None,
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 => format!("{f:.0}"),
            Some(f) => format!("{f:.2}"),
            None => n.to_string(),
        },
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

/// Builds the table while the panel shows and what it was built from has moved.
pub fn project(model: &mut Tracked<CompareListModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout.panel_state(&PanelId::CompareInList) == craft::behavior::dock::PanelState::Hidden {
        return;
    }
    let st = &cx.view.compare_in_list;
    let docs: Vec<(usize, &fontcraft_engine::DocState)> =
        st.docs.iter().filter_map(|id| cx.session.documents().iter().enumerate().find(|(_, d)| d.id == *id)).collect();
    let built: Vec<(u64, u64)> = docs.iter().map(|(_, d)| (d.id, d.revision)).collect();
    let positions: Vec<usize> = docs.iter().map(|(i, _)| *i).collect();
    if model.built == built && model.property == st.property && model.only_differences == st.only_differences && model.positions == positions {
        return;
    }
    let fonts: Vec<&Font> = docs.iter().map(|(_, d)| d.font.as_ref()).collect();
    let mut next = CompareListModel {
        fonts: docs.iter().map(|(_, d)| d.title.clone()).collect(),
        positions,
        property: st.property.clone(),
        only_differences: st.only_differences,
        built,
        ..CompareListModel::default()
    };
    match compare_table(&fonts, &st.property) {
        Ok(rows) => {
            let kept = rows.into_iter().filter(|r| !st.only_differences || r.differ);
            for row in kept {
                next.total += 1;
                if next.rows.len() < MAX_ROWS {
                    next.rows.push(CompareListRow { values: row.values.iter().map(shown).collect(), glyph: row.glyph, differ: row.differ });
                }
            }
        }
        Err(e) => next.error = Some(e),
    }
    set(model, next);
}

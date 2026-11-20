//! The Pairs & Phrases view model: the shown work list, each pair row with the value stored for it in the
//! current master, and the auto-kerning proposal under review.

use std::collections::HashMap;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use serde_json::{Value, json};

use super::ProjectCx;
use crate::shell::dock::PanelId;
use crate::shell::kerning_review::ListMode;

/// Proposal lines the panel lists.
pub const MAX_PROPOSAL_LINES: usize = 200;

/// One row of the work list.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PairRow {
    pub label: String,
    /// The value stored for a pair row's two sides; `None` for a phrase or an unkerned pair.
    pub value: Option<f64>,
    pub pair: bool,
}

/// One pair of an auto-kerning proposal, as `kerning.autoKern` reports it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ProposalLine {
    pub left: String,
    pub right: String,
    pub value: String,
    pub previous: String,
    pub result: String,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PairsModel {
    pub has_font: bool,
    pub mode: ListMode,
    /// The pairs list shows (else the phrases list).
    pub pairs: bool,
    /// The file the rows came from, `text`, or `None` for the built-in list.
    pub source: Option<String>,
    pub rows: Vec<PairRow>,
    pub current: Option<usize>,
    pub selected: Vec<usize>,
    /// The proposal under review; `None` when there is none.
    pub proposal: Option<Vec<ProposalLine>>,
}

pub fn project(model: &mut Tracked<PairsModel>, cx: &mut ProjectCx<'_>) {
    let state = &cx.view.kerning_review;
    let has_font = cx.session.font().is_ok();
    if !has_font || cx.view.workspace.layout.panel_state(&PanelId::PairsAndPhrases) == PanelState::Hidden {
        super::set(
            model,
            PairsModel { has_font, mode: state.mode, pairs: state.shows_pairs(crate::canvas::text::text_mode(cx.view)), ..PairsModel::default() },
        );
        return;
    }
    let pairs = state.shows_pairs(crate::canvas::text::text_mode(cx.view));
    let list = state.list(pairs);
    let rows = list.rows(pairs);
    let stored = if pairs { stored_values(cx) } else { HashMap::new() };
    let rows = rows
        .iter()
        .map(|r| {
            let value = match r.lines.first().map(Vec::as_slice) {
                Some([l, rt]) if r.is_pair() => stored.get(&(l.clone(), rt.clone())).copied(),
                _ => None,
            };
            PairRow { label: r.label(), value, pair: r.is_pair() }
        })
        .collect();
    let proposal = state.auto_kern.as_ref().map(|p| {
        let text = |v: &Value, k: &str| v.get(k).map_or(String::new(), |v| v.as_str().map_or_else(|| v.to_string(), str::to_string));
        let list = p.reply.get("pairs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        list.iter()
            .take(MAX_PROPOSAL_LINES)
            .map(|v| ProposalLine {
                left: text(v, "left"),
                right: text(v, "right"),
                value: text(v, "value"),
                previous: text(v, "previous"),
                result: text(v, "result"),
            })
            .collect()
    });
    let value = PairsModel {
        has_font,
        mode: state.mode,
        pairs,
        source: list.source.clone(),
        rows,
        current: list.current,
        selected: list.selected.clone(),
        proposal,
    };
    super::set(model, value);
}

/// The left-to-right pairs stored in the current master, by their sides as written (`@group` for a group).
fn stored_values(cx: &mut ProjectCx<'_>) -> HashMap<(String, String), f64> {
    let font = cx.session.font().ok();
    let Some(master) = font.and_then(|f| crate::shell::masters::current_master(f, &cx.view.layers)).map(|m| m.id.clone()) else {
        return HashMap::new();
    };
    let reply = cx.session.query("kerning.listPairs", &json!({ "master": master })).unwrap_or(Value::Null);
    let list = reply.get("pairs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    list.iter()
        .filter_map(|p| {
            let side = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
            Some(((side("left")?, side("right")?), p.get("value").and_then(Value::as_f64)?))
        })
        .collect()
}

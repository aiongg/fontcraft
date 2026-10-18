//! The Info box of a selected PostScript hint (`infoBox.psHint`), in place of the selection part
//! while Show Hints is on and hints are selected.
//!
//! It shows the first selected hint in list order:
//!
//! | Hint | Fields | Editing runs |
//! |---|---|---|
//! | stem | Origin, Size | `hint.ps.setHintValues` with `origin` or `size` |
//! | ghost | Origin, and an up and a down button for its direction | `hint.ps.setHintValues` with `origin`; `hint.ps.setGhostDirection` |
//!
//! A typed value unlinks the edge it sets, as `hint.ps.setHintValues` says. Under the fields, the
//! warnings `hint.ps.listHints` gives for the hint.

use fontcraft_tools::hint_drag::{HintKind, HintMark};
use serde_json::{Map, Value, json};

use super::{Pending, widgets, with};
use crate::app::FontcraftApp;
use crate::canvas::hints_ps;
use crate::theme::Tokens;
use crate::tools_glue;

/// The id of one of the box's fields or buttons: `origin`, `size`, `top`, `bottom`.
pub fn field(name: &str) -> egui::Id {
    widgets::id(("infoBox.psHint", name))
}

/// Draws the box when a hint is selected; `false` when none is, so the selection part shows.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) -> bool {
    if !app.state.display.hints || app.state.tab().is_none_or(|t| t.selection.hints.is_empty()) {
        return false;
    }
    let marks = hints_ps::marks(&mut app.session, &app.state, &app.tokens);
    let Some(tab) = app.state.tab() else { return false };
    let selected = marks.selected(&tab.selection);
    let Some(hint) = selected.first().and_then(|&i| marks.get(i)).cloned() else { return false };
    let Some(target) = target(app) else { return false };
    let tokens = app.tokens.clone();
    super::info_box::frame(ui, &tokens, "infoBox.psHint", |ui| contents(ui, &tokens, &hint, selected.len(), &target, pending));
    true
}

/// `{glyph, layer}` of the edited layer.
fn target(app: &FontcraftApp) -> Option<Map<String, Value>> {
    let font = app.session.font().ok()?;
    let scene = tools_glue::scene(font, app.state.tab()?)?;
    let mut m = Map::new();
    m.insert("glyph".into(), json!(scene.glyph));
    m.insert("layer".into(), json!(scene.layer_id));
    Some(m)
}

fn title(h: &HintMark) -> &'static str {
    match (h.kind, h.horizontal) {
        (HintKind::GhostTop, _) => "Top ghost hint",
        (HintKind::GhostBottom, _) => "Bottom ghost hint",
        (HintKind::Stem, true) => "Horizontal hint",
        (HintKind::Stem, false) => "Vertical hint",
    }
}

fn contents(ui: &mut egui::Ui, tokens: &Tokens, h: &HintMark, count: usize, target: &Map<String, Value>, pending: &mut Pending) {
    let width = tokens.field_width;
    let label = |ui: &mut egui::Ui, text: &str| {
        ui.label(egui::RichText::new(text).color(tokens.text_dim));
    };
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let name = if count > 1 { format!("{} ({count} selected)", title(h)) } else { title(h).to_string() };
            ui.label(name);
            label(ui, "Origin");
            if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, field("origin"), h.origin, width, false) {
                pending.push(("hint.ps.setHintValues", with(target.clone(), json!({"hint": h.index, "origin": v}))));
            }
            if h.kind.is_ghost() {
                for (name, arrow, hover, kind) in
                    [("top", "↑", "Top ghost hint", HintKind::GhostTop), ("bottom", "↓", "Bottom ghost hint", HintKind::GhostBottom)]
                {
                    if widgets::button(ui, field(name), arrow, hover, true, h.kind == kind) && h.kind != kind {
                        pending.push(("hint.ps.setGhostDirection", with(target.clone(), json!({"hints": [h.index], "direction": name}))));
                    }
                }
            } else {
                label(ui, "Size");
                if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, field("size"), h.size, width, false) {
                    pending.push(("hint.ps.setHintValues", with(target.clone(), json!({"hint": h.index, "size": v}))));
                }
            }
        });
        for w in &h.warnings {
            ui.label(egui::RichText::new(w).color(tokens.ps_hints.warning));
        }
    });
}

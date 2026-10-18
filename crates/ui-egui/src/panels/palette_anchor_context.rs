//! Palette › Anchor Context (`palette.anchorContext`): the context of the selected contextual
//! anchor, at the foot of the Palette while Show Info is on and exactly one anchor whose name
//! starts with `*` is selected in the edited layer. The field holds feature code, one context
//! per line; it commits when it loses the focus, through `anchor.setContext`, and Escape
//! restores the stored text.

use egui::{Id, Key, Modifiers};
use fontcraft_engine::features::marks::contextual::{MAX_CONTEXT_BYTES, anchor_context};
use serde_json::json;

use super::{Pending, edit_targets, widgets, with};
use crate::app::FontcraftApp;
use crate::tools_glue;

/// Rows the field shows before it grows.
const ROWS: usize = 3;

/// The selected contextual anchor and its stored context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selected {
    pub anchor: String,
    pub context: String,
}

/// The one selected anchor of the edited layer, when its name starts with `*`.
pub fn selected(app: &FontcraftApp) -> Option<Selected> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    let mut names = tab.selection.anchors.iter();
    let (Some(name), None) = (names.next(), names.next()) else { return None };
    if !name.starts_with('*') {
        return None;
    }
    let scene = tools_glue::scene(font, tab)?;
    let anchor = scene.layer.anchors.iter().find(|a| a.name == *name)?;
    Some(Selected { anchor: name.clone(), context: anchor_context(anchor).unwrap_or_default().to_string() })
}

/// Whether the section shows.
pub fn shown(app: &FontcraftApp) -> bool {
    selected(app).is_some()
}

/// The id of the context field of `anchor`.
pub fn field_id(anchor: &str) -> Id {
    widgets::id(("palette.anchorContext", anchor))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(sel) = selected(app) else { return };
    ui.weak(sel.anchor.as_str());
    if let Some(text) = context_field(ui, field_id(&sel.anchor), &sel.context)
        && let Some(targets) = edit_targets(app, "anchor.setContext")
    {
        pending.push(("anchor.setContext", with(targets, json!({"anchor": sel.anchor, "context": text}))));
    }
}

/// A field being edited: its text, and the pass that last drew it focused.
#[derive(Clone)]
struct Editing {
    text: String,
    pass: u64,
}

/// A multi-line field showing `shown`; the text to commit once it loses the focus changed.
fn context_field(ui: &mut egui::Ui, id: Id, shown: &str) -> Option<String> {
    let ctx = ui.ctx().clone();
    let pass = ctx.cumulative_pass_nr();
    // An edit left behind by a field that stopped being drawn (the selection changed) is dropped.
    let editing = ctx.data(|d| d.get_temp::<Editing>(id)).filter(|e| pass.saturating_sub(e.pass) <= 1);
    let mut text = editing.as_ref().map_or_else(|| shown.to_string(), |e| e.text.clone());
    let mut cancelled = false;
    if editing.is_some() && ctx.memory(|m| m.has_focus(id)) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        cancelled = true;
        text = shown.to_string();
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
    let output = egui::TextEdit::multiline(&mut text)
        .id(id)
        .code_editor()
        .desired_width(ui.available_width())
        .desired_rows(ROWS)
        .char_limit(MAX_CONTEXT_BYTES)
        .show(ui);
    if output.response.has_focus() {
        ctx.data_mut(|d| d.insert_temp(id, Editing { text, pass }));
        return None;
    }
    if editing.is_some() {
        ctx.data_mut(|d| d.remove::<Editing>(id));
        if !cancelled && text != shown {
            return Some(text);
        }
    }
    None
}

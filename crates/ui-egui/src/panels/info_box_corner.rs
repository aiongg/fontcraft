//! The corner component Info box: while Show Info is on and a selected node holds a corner
//! component, a box beside the selection box describes that corner.
//!
//! | Control | Runs |
//! |---|---|
//! | corner glyph name (a button) | opens the corner glyph picker for `cornerComponent.setGlyph` |
//! | arrow button | `cornerComponent.editGlyph`: the corner glyph joins the text and is edited |
//! | horizontal and vertical scale, in percent | `cornerComponent.setScale`; a negative value mirrors |
//! | alignment buttons: left, right, both, none | `cornerComponent.setAlignment` |
//!
//! Corner components have no selection of their own yet: the box describes the corner on the
//! first selected node that holds one, in the layer's hint order.

use fontcraft_engine::model::corner::{CornerAlignment, corner_hint};
use fontcraft_engine::model::{Font, Layer};
use serde_json::{Map, Value, json};

use super::{Pending, widgets, with};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::dialogs::glyph_picker_component::{self as picker, corners_on_nodes};
use crate::theme::Tokens;

/// The alignment buttons: the mode `cornerComponent.setAlignment` takes, the button's text and
/// its hover text.
pub const MODES: [(&str, &str, &str); 4] = [
    ("left", "←", "Align to the outgoing stroke"),
    ("right", "→", "Align to the incoming stroke"),
    ("both", "↔", "Align to both strokes"),
    ("none", "✕", "Do not align"),
];

/// What the box needs, read before drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerBox {
    pub glyph: String,
    pub layer: String,
    /// The corner's index in the layer's hint list.
    pub corner: usize,
    /// The corner glyph.
    pub name: String,
    /// Horizontal and vertical scale in percent.
    pub scale: (f64, f64),
    /// The mode of [`MODES`] the corner has; `None` for the stored value 3, which fits unturned.
    pub mode: Option<&'static str>,
    /// Whether the corner glyph exists.
    pub exists: bool,
}

fn mode_of(a: CornerAlignment) -> Option<&'static str> {
    match a {
        CornerAlignment::Left => Some("left"),
        CornerAlignment::Right => Some("right"),
        CornerAlignment::Both => Some("both"),
        CornerAlignment::Unaligned => Some("none"),
        CornerAlignment::Unused => None,
    }
}

/// The box for the corner on the first selected node of `layer` that holds one.
pub fn corner_box(font: &Font, glyph: &str, layer: &Layer, points: impl IntoIterator<Item = (usize, usize)>) -> Option<CornerBox> {
    let corner = *corners_on_nodes(layer, points).first()?;
    let hint = corner_hint(layer.info.hints.get(corner)?).ok().flatten()?;
    Some(CornerBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        corner,
        exists: font.glyph(&hint.name).is_some(),
        name: hint.name,
        scale: (hint.scale.x * 100.0, hint.scale.y * 100.0),
        mode: mode_of(hint.alignment),
    })
}

/// The box for the showing tab's selection; `None` when no selected node holds a corner.
pub fn read(app: &FontcraftApp) -> Option<CornerBox> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    if tab.selection.points.is_empty() {
        return None;
    }
    let layer = view::active_layer(font, tab)?;
    corner_box(font, tab.edited_glyph()?, layer, tab.selection.points.iter().copied())
}

/// Draws the box beside the selection box while a selected node holds a corner component.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(b) = read(app) else { return };
    let tokens = app.tokens.clone();
    let canvas = ui.available_rect_before_wrap();
    let pos = super::info_box_component::beside_selection_box(ui, &tokens, canvas);
    egui::Area::new(widgets::id("infoBox.corner"))
        .order(egui::Order::Middle)
        .fixed_pos(pos)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .constrain_to(canvas)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| contents(ui, &tokens, &b, pending));
        });
}

fn contents(ui: &mut egui::Ui, tokens: &Tokens, b: &CornerBox, pending: &mut Pending) {
    let mut target = Map::new();
    target.insert("glyph".into(), json!(b.glyph));
    target.insert("layer".into(), json!(b.layer));
    target.insert("corners".into(), json!([b.corner]));
    ui.horizontal(|ui| {
        let hover = if b.exists { "Choose another corner glyph" } else { "This corner glyph does not exist: choose another one" };
        if widgets::button(ui, widgets::id("infoBox.corner.name"), &b.name, hover, true, false) {
            pending.push((picker::COMMAND, with(target.clone(), json!({ "purpose": "cornerGlyph" }))));
        }
        if widgets::button(ui, widgets::id("infoBox.corner.edit"), "↗", "Edit the corner glyph", b.exists, false) {
            pending.push(("cornerComponent.editGlyph", json!({ "glyph": b.glyph, "layer": b.layer, "corner": b.corner })));
        }
    });
    ui.horizontal(|ui| {
        let width = tokens.field_width;
        for (label, hover, key, value) in
            [("↔%", "Horizontal scale in percent", "scaleX", b.scale.0), ("↕%", "Vertical scale in percent", "scaleY", b.scale.1)]
        {
            ui.label(egui::RichText::new(label).color(tokens.text_dim)).on_hover_text(hover);
            if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id(("infoBox.corner", key)), Some(value), width, false) {
                let mut extra = Map::new();
                extra.insert(key.into(), json!(v));
                pending.push(("cornerComponent.setScale", with(target.clone(), Value::Object(extra))));
            }
        }
        ui.separator();
        for (mode, text, hover) in MODES {
            let on = b.mode == Some(mode);
            if widgets::button(ui, widgets::id(("infoBox.corner.mode", mode)), text, hover, true, on) && !on {
                pending.push(("cornerComponent.setAlignment", with(target.clone(), json!({ "mode": mode }))));
            }
        }
    });
}

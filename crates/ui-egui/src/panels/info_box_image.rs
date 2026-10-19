//! The Info box of a selected placed image (`infoBox.image`), in place of the selection part
//! while the edited side's image is selected (`canvas::images`).
//!
//! | Control | Runs |
//! |---|---|
//! | X, Y: the image's position (its bottom-left corner before rotation) | `image.setTransform` with `x` or `y` |
//! | W, H: the whole image's size in font units | `image.setTransform` with `width` or `height` |
//! | ⟲: the rotation in degrees, counter-clockwise | `image.setTransform` with `rotation` |
//! | → | `image.revealFile`: the file in the system's file manager |
//! | 🔓 | `image.lock`; a locked image cannot be selected, so the box closes |
//!
//! The values are what `image.getInfo` reports. W and H need the file's pixel size and are left
//! out without it; a sheared image shows X and Y only.

use serde_json::{Map, Value, json};

use super::{Pending, widgets, with};
use crate::app::FontcraftApp;
use crate::theme::Tokens;

/// The id of one of the box's fields or buttons: `x`, `y`, `width`, `height`, `rotation`,
/// `reveal`, `lock`.
pub fn field(name: &str) -> egui::Id {
    widgets::id(("infoBox.image", name))
}

/// Draws the box when the edited side's image is selected; `false` when none is.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) -> bool {
    let Some(target) = crate::canvas::images::selected_targets(app) else { return false };
    let Ok(info) = app.session.execute("image.getInfo", &Value::Object(target.clone())) else { return false };
    let Some(image) = info.get("image").filter(|i| i.is_object()).cloned() else { return false };
    let tokens = app.tokens.clone();
    super::info_box::frame(ui, &tokens, "infoBox.image", |ui| contents(ui, &tokens, &image, &target, pending));
    true
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn contents(ui: &mut egui::Ui, tokens: &Tokens, image: &Value, target: &Map<String, Value>, pending: &mut Pending) {
    let width = tokens.field_width;
    let set = |pending: &mut Pending, key: &str, v: f64| {
        pending.push(("image.setTransform", with(target.clone(), json!({ key: v }))));
    };
    ui.horizontal(|ui| {
        let field_at = |ui: &mut egui::Ui, label: &str, name: &str, hover: &str, pending: &mut Pending| {
            let value = num(image, name);
            if value.is_none() && name != "x" && name != "y" {
                return;
            }
            ui.label(egui::RichText::new(label).color(tokens.text_dim)).on_hover_text(hover);
            if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, field(name), value, width, false) {
                set(pending, name, v);
            }
        };
        field_at(ui, "X", "x", "Image position", pending);
        field_at(ui, "Y", "y", "Image position", pending);
        field_at(ui, "W", "width", "Image width", pending);
        field_at(ui, "H", "height", "Image height", pending);
        field_at(ui, "⟲", "rotation", "Rotation counterclockwise, in degrees", pending);
        if widgets::button(ui, field("reveal"), "→", "Reveal Image File", true, false) {
            // Without `glyph`, Reveal Image File takes the edited glyph's image on the side being
            // edited: the selected image.
            pending.push(("image.revealFile", json!({})));
        }
        if widgets::button(ui, field("lock"), "🔓", "Lock Image", true, false) {
            pending.push(("image.lock", with(target.clone(), json!({}))));
        }
    });
    if let Some(problem) = image.get("problem").and_then(Value::as_str) {
        ui.label(egui::RichText::new(problem).color(tokens.text_dim));
    }
}

//! The master icons in the toolbar (M9.20): one per master while the font has two or more, at
//! the toolbar's left end, the showing tab's master highlighted. A click runs `view.selectMaster`
//! for that master, as Cmd+1 to Cmd+9 do for the first nine.
//!
//! Each icon is the master's icon from Font Info › Masters (`master_icons::paint_master`): a
//! built-in n, or a glyph of the font drawn in that master.
//!
//! Rows: `preview.masterIconsInToolbar`.

use egui::{Color32, Sense, vec2};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::master_icons;

/// The id of the icon of the master at `index`.
pub fn icon_id(index: usize) -> egui::Id {
    egui::Id::new(("fontcraft.masterSwitcher", index))
}

/// Hover text: the master's name, and its shortcut for the first nine.
pub fn hover(name: &str, index: usize) -> String {
    if index < 9 { format!("{name} (Cmd+{})", index + 1) } else { name.to_string() }
}

/// Draws the master icons and a separator in the toolbar's row; nothing for a font with fewer
/// than two masters.
pub fn show(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    let Ok(font) = app.session.font() else { return };
    if font.masters.len() < 2 {
        return;
    }
    let current = app.state.tab().and_then(|t| view::tab_master(font, t)).map(|m| m.id.as_str());
    let tokens = &app.tokens;
    for (i, master) in font.masters.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(tokens.toolbar_button, tokens.toolbar_button), Sense::hover());
        let response = ui.interact(rect, icon_id(i), Sense::CLICK);
        let on = current == Some(master.id.as_str());
        if ui.is_rect_visible(rect) {
            let fill = if on {
                tokens.row_selected
            } else if response.hovered() {
                tokens.hover
            } else {
                Color32::TRANSPARENT
            };
            let color = if on { tokens.selected_text } else { tokens.text };
            ui.painter().rect_filled(rect, tokens.radius, fill);
            let inner = rect.shrink((rect.width() - tokens.toolbar_icon).max(0.0) / 2.0);
            master_icons::paint_master(ui.painter(), inner, font, master, color);
        }
        if response.on_hover_text(hover(&master.name, i)).clicked() {
            pending.push(("view.selectMaster", json!({ "master": i })));
        }
    }
    ui.separator();
}

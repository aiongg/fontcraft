//! The master icon picker of the Masters tab: a click on the icon beside a master's name opens
//! it. It shows the 25 built-in icons ([`crate::master_icons`]) as a grid, weights down and
//! widths across; a click on one gives it to the selected masters (`font.master.setIcon` with
//! `icon`). The glyph field at the bottom takes a glyph name of the font, and Use Glyph (or
//! Enter) makes that glyph the icon (`font.master.setIcon` with `glyph`). Cancel, Escape or a
//! click outside closes it and changes nothing.
//!
//! Row: `fontInfo.masterIconPicker` (M7.17).

use egui::{Id, Key, RichText, Sense};
use fontcraft_model::MAX_NAME_LEN;
use serde_json::{Map, Value, json};

use crate::master_icons::{self, WEIGHTS, WIDTHS};
use crate::panels::widgets;
use crate::theme::Tokens;

/// The picker's state.
#[derive(Clone, Debug, PartialEq)]
pub struct IconPicker {
    /// The masters the icon goes to, as indices into the font's masters.
    pub masters: Vec<usize>,
    /// The icon the first master shows, highlighted in the grid.
    pub current: Option<String>,
    /// The glyph name typed into the field.
    pub glyph: String,
}

/// How the picker closed.
#[derive(Clone, Debug, PartialEq)]
pub enum Picked {
    /// A built-in icon was clicked.
    Icon(&'static str),
    /// Use Glyph was clicked, or Enter pressed in the glyph field.
    Glyph(String),
    Cancel,
}

impl IconPicker {
    pub fn new(masters: Vec<usize>, current: Option<String>) -> Self {
        Self { masters, current, glyph: String::new() }
    }

    /// The command a pick runs; nothing for Cancel or an empty glyph name.
    pub fn command(&self, picked: &Picked) -> Option<(&'static str, Value)> {
        let (key, value) = match picked {
            Picked::Icon(name) => ("icon", (*name).to_string()),
            Picked::Glyph(name) if !name.trim().is_empty() => ("glyph", name.trim().to_string()),
            Picked::Glyph(_) | Picked::Cancel => return None,
        };
        let mut p = Map::new();
        p.insert("masters".into(), json!(self.masters));
        p.insert(key.into(), json!(value));
        Some(("font.master.setIcon", Value::Object(p)))
    }
}

/// The id of the picker's modal area.
pub fn modal_id() -> Id {
    widgets::id(("fontInfo.masterIconPicker", "modal"))
}

/// The id of the built-in icon `name` in the grid.
pub fn icon_id(name: &str) -> Id {
    widgets::id(("fontInfo.masterIconPicker", "icon", name))
}

/// The id of the glyph field.
pub fn glyph_id() -> Id {
    widgets::id(("fontInfo.masterIconPicker", "glyph"))
}

/// The id of the Use Glyph button.
pub fn use_glyph_id() -> Id {
    widgets::id(("fontInfo.masterIconPicker", "useGlyph"))
}

/// The id of the Cancel button.
pub fn cancel_id() -> Id {
    widgets::id(("fontInfo.masterIconPicker", "cancel"))
}

/// Draws the picker over everything else. Returns what was picked, if the picker closed.
pub fn show(ctx: &egui::Context, tokens: &Tokens, picker: &mut IconPicker) -> Option<Picked> {
    let mut picked = None;
    let side = tokens.toolbar_button;
    let modal = egui::Modal::new(modal_id()).show(ctx, |ui| {
        ui.label(RichText::new("Master Icon").strong().size(tokens.dialog_title_size));
        ui.add_space(tokens.dialog_spacing);
        for weight in 0..WEIGHTS {
            ui.horizontal(|ui| {
                for width in 0..WIDTHS {
                    let Some(name) = master_icons::name_at(weight, width) else { continue };
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
                    let response = ui.interact(rect, icon_id(name), Sense::click());
                    let current = picker.current.as_deref() == Some(name);
                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();
                        if current || response.hovered() {
                            let fill = if current { tokens.row_selected } else { ui.visuals().widgets.hovered.weak_bg_fill };
                            painter.rect_filled(rect, f32::from(tokens.radius), fill);
                        }
                        let color = if current { tokens.selected_text } else { tokens.text };
                        master_icons::paint_builtin(painter, rect, name, color);
                    }
                    if response.on_hover_text(master_icons::label(name)).clicked() {
                        picked = Some(Picked::Icon(name));
                    }
                }
            });
        }
        ui.add_space(tokens.dialog_spacing);
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut picker.glyph)
                    .id(glyph_id())
                    .hint_text("Glyph name")
                    .char_limit(MAX_NAME_LEN)
                    .desired_width(tokens.dialog_field_width),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !picker.glyph.trim().is_empty() {
                picked = Some(Picked::Glyph(picker.glyph.clone()));
            }
            let usable = !picker.glyph.trim().is_empty();
            if widgets::button(ui, use_glyph_id(), "Use Glyph", "Use this glyph of the font as the icon", usable, false) {
                picked = Some(Picked::Glyph(picker.glyph.clone()));
            }
        });
        ui.add_space(tokens.dialog_spacing);
        if widgets::button(ui, cancel_id(), "Cancel", "Close without changing the icon", true, false) {
            picked = Some(Picked::Cancel);
        }
    });
    if picked.is_none() && modal.should_close() {
        picked = Some(Picked::Cancel);
    }
    picked
}

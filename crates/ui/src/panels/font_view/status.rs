//! The row under the glyphs (`fontView.statusBar`): the New Glyph and Remove Glyph buttons at the start (Remove Glyph
//! opens its dialog on the selection and is greyed without one), the selected, shown and total counts, and the cell
//! size slider (`fontView.setCellSize`) in Grid View.

use craft::behavior::IconId;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::value_drag::Slider;
use egui::Ui;
use serde_json::json;

use super::{FontViewWords, keys};
use crate::app::Msg;
use crate::app::font_view::{MAX_CELL_SIZE, MIN_CELL_SIZE, ViewMode};
use crate::app::models::font_view::FontViewModel;

/// The counts as the status row prints them.
pub fn counts_text(words: &FontViewWords, selected: usize, shown: usize, total: usize) -> String {
    format!("{selected} {} · {shown} {} · {total} {}", words.selected.as_str(), words.shown.as_str(), words.in_font.as_str())
}

pub fn show(ui: &mut Ui, model: &FontViewModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<FontViewWords>();
    let selected: Vec<&str> = model.selected.iter().filter_map(|&i| model.name(i)).collect();
    let mut size = model.cell_size;
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if IconButton::new(IconId::new("plus")).label(words.new_glyph.clone()).show(ui).clicked() {
            sent.push(Msg::run("glyph.newGlyph", json!({})));
        }
        let remove = IconButton::new(IconId::new("minus")).label(words.remove_glyph.clone()).enabled(!selected.is_empty()).show(ui);
        if remove.clicked() && !selected.is_empty() {
            sent.push(keys::remove(selected.iter().map(|g| (*g).to_string()).collect()));
        }
        Text::new(counts_text(&words, model.selected.len(), model.visible.len(), model.rows.len())).show(ui);
        if let Some(error) = &model.error {
            Text::new(error.clone()).show(ui);
        }
        if model.mode == ViewMode::Grid {
            Slider::new(Bind::bound(&mut size), MIN_CELL_SIZE..=MAX_CELL_SIZE).label(words.cell_size.clone()).show(ui);
        }
    });
    if (size - model.cell_size).abs() >= 0.5 {
        sent.push(Msg::Shell(CommandRef::with_params("fontView.setCellSize", json!({ "size": size }))));
    }
}

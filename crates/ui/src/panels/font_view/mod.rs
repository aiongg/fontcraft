//! Font View: the glyphs of the font as a grid of cells or as a table of their properties, with the search field
//! and the mode buttons above them and the status row below them.
//!
//! | Module | Holds |
//! |---|---|
//! | [`grid`] | Grid View: one tile per shown glyph, its outline, its name from the label size up |
//! | [`list`] | List View: one row per shown glyph, a column per property, sort and column choice, edits in place |
//! | [`cells`] | the outline painter of a grid cell |
//! | [`keys`] | what the grid's and the list's selection, keys and requests post |
//! | [`search`] | the search field and the mode buttons |
//! | [`status`] | the New Glyph and Remove Glyph buttons, the counts and the cell size slider |
//!
//! Its view state is `app::font_view`; its shell commands are `shell::font_view`. Font changes go through engine
//! commands.

pub mod cells;
pub mod grid;
pub mod inspector;
pub mod keys;
pub mod list;
pub mod metrics;
pub mod search;
pub mod sidebar;
pub mod status;

use craft::ui::contract::{Show as _, ShowContainer as _, Tracked};
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::runtime::env::Env;
use egui::Ui;

use crate::app::Msg;
use crate::app::font_view::ViewMode;
use crate::app::models::font_view::FontViewModel;

craft::ui::words! {
    /// The words of Font View.
    pub struct FontViewWords {
        /// The grid's and the list's name.
        glyphs: "fontcraftFontView.glyphs" = "Glyphs";
        /// The search field.
        search: "fontcraftFontView.search" = "Search";
        /// The mode buttons' group, and each mode.
        view: "fontcraftFontView.view" = "View";
        grid_view: "fontcraftFontView.gridView" = "Grid View";
        list_view: "fontcraftFontView.listView" = "List View";
        /// The plus and minus buttons.
        new_glyph: "fontcraftFontView.newGlyph" = "New Glyph";
        remove_glyph: "fontcraftFontView.removeGlyph" = "Remove Glyph";
        /// The cell size slider.
        cell_size: "fontcraftFontView.cellSize" = "Cell Size";
        /// The status row's counts.
        selected: "fontcraftFontView.selected" = "selected";
        shown: "fontcraftFontView.shown" = "shown";
        in_font: "fontcraftFontView.inFont" = "in font";
        /// The grid or list of a font with no glyph shown.
        no_glyphs: "fontcraftFontView.noGlyphs" = "No glyphs";
        /// Without an open font.
        no_font: "fontcraftFontView.noFont" = "File › New to start a font";
    }
}

/// What Font View reads and sends.
pub struct Props<'a> {
    pub model: &'a Tracked<FontViewModel>,
    /// The placed images the cells draw behind their glyphs.
    pub images: &'a Tracked<crate::canvas::images::cells::CellImages>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<FontViewWords>();
    let model: &FontViewModel = props.model;
    let images = props.images;
    if !model.has_font {
        Text::new(words.no_font.clone()).show(ui);
        return;
    }
    let (mut top, mut bottom, mut body) = (Vec::new(), Vec::new(), Vec::new());
    let _ = Stack::new().fill().header(|ui, _| search::show(ui, model, &mut top)).footer(|ui, _| status::show(ui, model, &mut bottom)).show(
        ui,
        |ui, _| match model.mode {
            ViewMode::Grid => grid::show(ui, model, images, &mut body),
            ViewMode::List => list::show(ui, model, &mut body),
        },
    );
    props.sent.extend(top.into_iter().chain(body).chain(bottom));
}

#[cfg(test)]
mod tests;

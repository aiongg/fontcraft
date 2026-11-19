//! Grid View (`fontView.grid`): a tile per shown glyph with its outline in the master Font View draws, and from
//! the label size up its name under it (`fontView.cell.contentBySize`). A glyph that does not export shows veiled.
//! The grid lays out only the tiles in view, so a font of tens of thousands of glyphs scrolls as a small one does.

use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource};
use craft::ui::contract::{Bind, ShowCollection as _};
use craft::ui::list::ThumbAspect;
use craft::ui::runtime::env::Env;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::Ui;

use super::{FontViewWords, cells, keys};
use crate::app::Msg;
use crate::app::font_view::LABEL_MIN_CELL;
use crate::app::models::font_view::FontViewModel;

/// The shown glyphs in display order, keyed by their row in the model.
pub struct Shown<'a> {
    model: &'a FontViewModel,
    /// Per row, its display position; `None` for a row not shown.
    at: Vec<Option<usize>>,
}

impl<'a> Shown<'a> {
    pub fn new(model: &'a FontViewModel) -> Self {
        let mut at = vec![None; model.rows.len()];
        for (position, &row) in model.visible.iter().enumerate() {
            if let Some(slot) = at.get_mut(row) {
                *slot = Some(position);
            }
        }
        Shown { model, at }
    }
}

impl DisplayOrder<usize> for Shown<'_> {
    fn len(&self) -> usize {
        self.model.visible.len()
    }

    fn key(&self, at: usize) -> usize {
        self.model.visible.get(at).copied().unwrap_or(usize::MAX)
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        self.at.get(*key).copied().flatten()
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.model.visible.get(at).and_then(|&row| self.model.name(row)).map(std::borrow::Cow::Borrowed)
    }
}

impl ItemSource<usize> for Shown<'_> {}

pub fn show(ui: &mut Ui, model: &FontViewModel, sent: &mut Vec<Msg>) {
    let env = Env::of(ui);
    let words = env.words::<FontViewWords>();
    let card = env.tokens.size.thumb.card;
    let zoom = if card > 0.0 { (model.cell_size as f32 / card).max(f32::EPSILON) } else { 1.0 };
    let caption = if model.cell_size >= LABEL_MIN_CELL { CaptionPlace::Below } else { CaptionPlace::None };
    let (ascender, descender) = (model.ascender, model.descender);
    let mut selection = keys::selection(model);
    let source = Shown::new(model);
    let out = TileGrid::<usize>::new(words.glyphs.clone())
        .empty(words.no_glyphs.clone())
        .zoom(zoom)
        .caption(caption)
        .type_ahead(true)
        .deletable(true)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &source, &mut |row, _| {
            let Some(glyph) = model.rows.get(*row) else { return Tile::new(String::new()) };
            let mut tile = Tile::new(glyph.name.clone());
            if !glyph.export {
                tile = tile.hidden();
            }
            if let Some(art) = glyph.art.as_ref().map(Arc::clone) {
                tile = tile.thumb(ThumbAspect::SQUARE, move |cx, rect| cells::paint(cx, rect, &art, ascender, descender));
            }
            tile
        });
    keys::requests(model, &selection, &out.response.events, sent);
}

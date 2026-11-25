//! Grid View (`fontView.grid`): a tile per shown glyph with its outline in the master Font View draws, and from
//! the label size up its name under it (`fontView.cell.contentBySize`). A glyph that does not export shows veiled.
//! The grid lays out only the tiles in view, so a font of tens of thousands of glyphs scrolls as a small one does.
//! A tile draws its glyph's placed image behind the glyph (`canvas::images::cells`), and image files dropped onto a
//! tile go on that glyph as Glyph › Add Image… places them (`canvas::images::drop::cell_params`). A glyph in a status
//! (`glyph.setStatus`, or the font's default status) has a thin bar of the status colour along the tile's foot. With a
//! strike shown (`view.setFontViewStrike`) a tile draws its glyph's bitmap in that strike instead of the outline and
//! image, and always shows the name, so a glyph without a bitmap is an empty tile with its name. A glyph whose layers
//! cannot interpolate has a red top-left corner, and the reason as its tooltip; a tile drawn before its glyph was checked
//! asks for the answer (`ViewMsg::UncheckedCells`).

use std::path::PathBuf;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource};
use craft::ui::contract::{Bind, CollectionEvent, DropAt, DropKind, DropPlace, OsFiles, Props as _, ShowCollection as _};
use craft::ui::list::ThumbAspect;
use craft::ui::runtime::env::Env;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::Ui;

use super::{FontViewWords, cells, keys};
use crate::app::font_view::LABEL_MIN_CELL;
use crate::app::models::font_view::FontViewModel;
use crate::app::models::incompatibility::{CellCompat, MAX_CELLS};
use crate::app::{Msg, ViewMsg};
use crate::canvas::images::cells::CellImages;
use crate::canvas::images::drop;

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

pub fn show(ui: &mut Ui, model: &FontViewModel, images: &CellImages, sent: &mut Vec<Msg>) {
    let env = Env::of(ui);
    let words = env.words::<FontViewWords>();
    let card = env.tokens.size.thumb.card;
    let zoom = if card > 0.0 { (model.cell_size as f32 / card).max(f32::EPSILON) } else { 1.0 };
    let caption = if model.cell_size >= LABEL_MIN_CELL || model.strike.is_some() { CaptionPlace::Below } else { CaptionPlace::None };
    let strike = model.strike;
    let (ascender, descender) = (model.ascender, model.descender);
    let mut selection = keys::selection(model);
    let source = Shown::new(model);
    let mut unchecked = Vec::new();
    let out = TileGrid::<usize>::new(words.glyphs.clone())
        .empty(words.no_glyphs.clone())
        .zoom(zoom)
        .caption(caption)
        .type_ahead(true)
        .deletable(true)
        .selection(Bind::bound(&mut selection))
        // A hovered file's name may not be known yet; the drop checks it again.
        .takes_drop(DropKind::new(DropPlace::Onto, |files: &OsFiles, _: &DropAt<usize>| {
            files.files.iter().any(|f| f.name.is_empty() || drop::is_image(&f.name))
        }))
        .show_items(ui, &source, &mut |row, _| {
            let Some(glyph) = model.rows.get(*row) else { return Tile::new(String::new()) };
            let mut tile = Tile::new(glyph.name.clone());
            if !glyph.export {
                tile = tile.hidden();
            }
            let art = glyph.art.as_ref().map(Arc::clone);
            let image = if strike.is_some() { None } else { images.by_glyph.get(&glyph.name).map(Arc::clone) };
            let bitmap = strike.zip(glyph.bitmap.as_ref().map(Arc::clone));
            let status = glyph.status_color;
            let incompatible = match &glyph.compat {
                CellCompat::Fine => false,
                CellCompat::Unchecked => {
                    if unchecked.len() < MAX_CELLS {
                        unchecked.push(glyph.name.clone());
                    }
                    false
                }
                CellCompat::Differs(reason) => {
                    tile = tile.tooltip(reason.to_string()).accessible_name(format!("{}, {}", glyph.name, words.incompatible.as_str()));
                    true
                }
            };
            if art.is_some() || image.is_some() || bitmap.is_some() || status.is_some() || incompatible {
                tile = tile.thumb(ThumbAspect::SQUARE, move |cx, rect| {
                    let width = art.as_ref().map_or(0.0, |a| a.width);
                    if let Some(image) = &image {
                        cells::paint_image(cx, rect, image, width, ascender, descender);
                    }
                    if let Some(art) = &art {
                        cells::paint(cx, rect, art, ascender, descender);
                    }
                    if let Some((shown, cell)) = &bitmap {
                        cells::paint_strike(cx, rect, cell, shown);
                    }
                    if let Some(color) = status {
                        cells::paint_status(cx, rect, color);
                    }
                    if incompatible {
                        cells::paint_incompatible(cx, rect);
                    }
                });
            }
            tile
        });
    if !unchecked.is_empty() {
        sent.push(Msg::View(ViewMsg::UncheckedCells(unchecked)));
    }
    for event in &out.response.events {
        if let CollectionEvent::Dropped { at: DropAt::Onto(row), payload, .. } = event
            && let Some(files) = payload.get::<OsFiles>()
            && let (Some(glyph), Some(master)) = (model.name(*row), model.master.as_deref())
        {
            let paths: Vec<PathBuf> = files.files.iter().filter_map(|f| f.path.clone()).collect();
            if let Some(params) = drop::cell_params(glyph, master, &paths) {
                sent.push(Msg::run("glyph.addImage", params));
            }
        }
    }
    keys::requests(ui.ctx(), model, &selection, &out.response.events, sent);
}

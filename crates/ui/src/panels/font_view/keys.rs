//! Font View's keyboard and pointer requests (`fontView.keyboard`), shared by the grid and the list: what a
//! selection change, an activation, Delete and a rename post.
//!
//! The grid and the list move the selection themselves: arrows by one cell (Up and Down by a row in the grid),
//! Shift with an arrow extends from the anchor, Home and End go to the first and last shown glyph, and the view
//! scrolls to keep the lead in sight. Return or a double click opens the selection in Edit View.
//! [`requests`] turns what they report into messages; a caller that drives Font View without the view (the
//! control channel) posts the same messages.

use craft::behavior::{EditEvent, Selection};
use craft::ui::contract::CollectionEvent;
use craft::ui::runtime::commands::CommandRef;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::font_view::FontViewModel;

/// The selection of `model` as the grid and the list hold it: row indices, the anchor as lead and anchor.
pub fn selection(model: &FontViewModel) -> Selection<usize> {
    let lead = model.anchor.filter(|a| model.selected.contains(a)).or_else(|| model.selected.last().copied());
    Selection::new(model.selected.clone(), lead, lead)
}

/// The messages for what the grid or the list reported: `after` is the selection it left, `events` its events.
pub fn requests<E>(model: &FontViewModel, after: &Selection<usize>, events: &[CollectionEvent<usize, E>], sent: &mut Vec<Msg>) {
    let names = |rows: &[usize]| rows.iter().filter_map(|&i| model.name(i)).map(str::to_string).collect::<Vec<_>>();
    if after.items != model.selected {
        let mut glyphs = names(&after.items);
        // The lead goes last: it is the anchor the next Shift press extends from.
        if let Some(lead) = after.lead.and_then(|i| model.name(i))
            && let Some(at) = glyphs.iter().position(|g| g == lead)
        {
            let lead = glyphs.remove(at);
            glyphs.push(lead);
        }
        sent.push(Msg::Shell(CommandRef::with_params("fontView.setSelection", json!({ "glyphs": glyphs }))));
    }
    for event in events {
        match event {
            CollectionEvent::Activate(at) => {
                let glyphs = if after.items.contains(at) { names(&after.items) } else { names(&[*at]) };
                if !glyphs.is_empty() {
                    sent.push(Msg::Shell(CommandRef::with_params("view.editSelectedGlyphs", json!({ "glyphs": glyphs }))));
                }
            }
            CollectionEvent::Delete(rows) => {
                let glyphs = names(rows);
                if !glyphs.is_empty() {
                    sent.push(Msg::run("glyph.removeGlyph", json!({ "glyphs": glyphs })));
                }
            }
            CollectionEvent::Rename(EditEvent::Commit { after: rename, .. } | EditEvent::Set { after: rename, .. }) => {
                if let Some(glyph) = model.name(rename.key)
                    && glyph != rename.name
                    && !rename.name.trim().is_empty()
                {
                    sent.push(Msg::run("glyph.rename", json!({ "glyph": glyph, "newName": rename.name.trim() })));
                }
            }
            CollectionEvent::ContextMenu(request) => {
                // A glyph that is not selected becomes the selection first; on a selected one the menu acts on all.
                let glyphs = if after.items.contains(&request.key) { names(&after.items) } else { names(&[request.key]) };
                if !after.items.contains(&request.key) && !glyphs.is_empty() {
                    sent.push(Msg::Shell(CommandRef::with_params("fontView.setSelection", json!({ "glyphs": glyphs }))));
                }
                if !glyphs.is_empty() {
                    let params = json!({ "glyphs": glyphs, "x": request.at.x, "y": request.at.y });
                    sent.push(Msg::Shell(CommandRef::with_params("fontView.openGlyphMenu", params)));
                }
            }
            _ => {}
        }
    }
}

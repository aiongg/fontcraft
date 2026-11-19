//! The `palette+outline` layout: the glyph grid, then one outline page per glyph, each grid cell a
//! link to its glyph's outline page, as the reference's palette with outlines.

use crate::ProofError;
use crate::font::Face;
use crate::page::{self, Document, Item};

pub(crate) fn sheet(face: &Face<'_>, doc: &mut Document, page_size: [f64; 2], size: f64) -> Result<(), ProofError> {
    let grid_start = doc.pages.len();
    page::grid(face, doc, page_size, size)?;
    let outline_start = doc.pages.len();
    page::outline_pages(face, doc, page_size)?;
    for page in doc.pages.iter_mut().take(outline_start).skip(grid_start) {
        let mut links = Vec::new();
        let mut cell: Option<(f64, f64, f64, f64)> = None;
        // The grid draws each cell's border, then its glyph.
        for item in &page.items {
            match item {
                Item::Rect { x, y, w, h, .. } => cell = Some((*x, *y, *w, *h)),
                Item::Glyph { glyph, primary: true, .. } => {
                    if let Some((x, y, w, h)) = cell.take() {
                        links.push(Item::Link { x, y, w, h, page: outline_start + glyph });
                    }
                }
                _ => {}
            }
        }
        page.items.extend(links);
    }
    Ok(())
}

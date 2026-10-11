//! SVG documents and the `SVG ` table (M11.9; bitmap documents M11.10).
//!
//! Rows: `SVG `, `color.export.fullColorToSvg`, `color.export.colrToSvg`,
//! `color.export.colorPaletteForSvg`, `color.export.colorLayersToSvg`, `color.layered.toSvg`,
//! `color.export.sbixToSvg`.
//!
//! Contract (`docs/plans/M11.md` § 3 › SVG documents):
//! - One uncompressed document per glyph. Its group has the id `glyph<ID>` and a transform that
//!   flips y at the baseline (nanoemoji's `untouchedsvg` layout).
//! - Palette colours are baked in from the palette `Color Palette for SVG` names; the text colour
//!   is written as `currentColor`.
//! - Centred strokes are written as SVG strokes; strokes on one side are expanded by the caller
//!   and arrive as outlines.
//! - A document is at most [`crate::MAX_SVG_DOCUMENT_BYTES`]; the table at most
//!   [`crate::MAX_SVG_TABLE_BYTES`].

use fontcraft_geom::{Affine, BezPath};

use crate::paint::{Paint, ShapeRef};
use crate::{ColorError, Rgba};

/// Where a bitmap is drawn in a glyph's document, in font units (y up, as the layer has it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BitmapPlacement {
    /// Maps the image's pixel grid (origin at its bottom left, one unit per pixel) into the
    /// glyph.
    pub transform: Affine,
    /// The image's size in pixels.
    pub width: u32,
    pub height: u32,
}

/// The document of glyph `gid` drawing `paint`. `outlines` draws a [`ShapeRef`] in font units;
/// `palette` is the palette baked in.
pub fn document(paint: &Paint, outlines: &dyn Fn(&ShapeRef) -> BezPath, gid: u16, palette: &[Rgba]) -> Result<String, ColorError> {
    // M11.9
    let _ = (paint, outlines, gid, palette);
    Err(ColorError::NotYetImplemented("SVG documents"))
}

/// The document of glyph `gid` showing a PNG image, embedded as a data URL in an `<image>`
/// element.
pub fn bitmap_document(png: &[u8], placement: &BitmapPlacement, gid: u16) -> Result<String, ColorError> {
    // M11.10
    let _ = (png, placement, gid);
    Err(ColorError::NotYetImplemented("SVG documents for bitmaps"))
}

/// The `SVG ` table from documents given as (first glyph id, last glyph id, document bytes);
/// records are sorted by glyph id and must not overlap.
pub fn table(docs: &[(u16, u16, Vec<u8>)]) -> Result<Vec<u8>, ColorError> {
    // M11.9
    let _ = docs;
    Err(ColorError::NotYetImplemented("the SVG table"))
}

//! TTX reader: variation tables (M12.9).
//!
//! HVAR, VVAR and MVAR go through the generic layout compile of [`super::read_otdata`], as in
//! fontTools, where they are `otData` tables too. Ported from fontTools' `ttLib.tables` (MIT).
//!
//! Not read yet: `gvar` and `cvar` (their tuple variation stores), which fall back to their
//! `hexdata` when the element has one.

use crate::Result;
use crate::sfnt::{SfntFont, Tag};

use super::Element;
use super::read::ReadContext;

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle. `font` gives the context some tables need (glyph order, `maxp`, `head`).
pub fn read_table(element: &Element, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    read_table_with(element, &ReadContext::from_font(font), font)
}

/// [`read_table`] with the glyph order of the text being read; `font` holds the tables read so
/// far (`fvar` among the first).
pub(crate) fn read_table_with(element: &Element, ctx: &ReadContext, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    let tag = match element.name.as_str() {
        "HVAR" => *b"HVAR",
        "VVAR" => *b"VVAR",
        "MVAR" => *b"MVAR",
        _ => return Ok(None),
    };
    let data = super::read_otdata::build::compile_layout(&element.name, element, ctx, font)?;
    Ok(Some((tag, data)))
}

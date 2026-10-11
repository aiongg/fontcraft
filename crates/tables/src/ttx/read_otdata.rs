//! TTX reader: layout and the other structured tables (M12.9).

use crate::Result;
use crate::sfnt::{SfntFont, Tag};

use super::Element;

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle. `font` gives the context some tables need (glyph order, `maxp`, `head`).
pub fn read_table(element: &Element, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    // M12.9
    let _ = (element, font);
    Ok(None)
}

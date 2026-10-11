//! FontDame (MTI) layout text to feature code (M12.13; rows `features convert`,
//! `layout.convertSyntax`). Oracle: fontTools `mtiLib` (MIT); a port is credited in this header
//! and `NOTICE`.

use crate::{Result, TablesError};

/// Feature code for FontDame text of at most [`crate::limits::MAX_FEATURE_SOURCE`] bytes.
/// `glyph_order` names the glyphs the text refers to by index.
pub fn fontdame_to_fea(text: &str, glyph_order: &[String]) -> Result<(String, Vec<String>)> {
    // M12.13
    let _ = (text, glyph_order);
    Err(TablesError::NotBuilt("FontDame conversion"))
}

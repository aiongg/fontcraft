//! Text views of a font and the normalised dump (M12.10; row `dump` with its text views,
//! `@font`, `@glyphs` and `--normalize`). Oracles: AFDKO spot and `tx -dump`, ttxn.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What [`view`] prints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewOptions {
    /// The view, by the name `docs/spec/20-cli.md` › dump gives it.
    pub view: String,
    /// Tables the view covers; empty covers every table the view knows.
    pub tables: Vec<Tag>,
    /// Glyphs the view covers (`@glyphs`); empty covers every glyph.
    pub glyphs: Vec<String>,
    /// Print the normalised dump (ttxn's form): glyph ids as names, offsets and counts left out,
    /// lookups flattened.
    pub normalize: bool,
}

/// The view as text.
pub fn view(font: &SfntFont, options: &ViewOptions) -> Result<String> {
    // M12.10
    let _ = (font, options);
    Err(TablesError::NotBuilt("dump text views"))
}

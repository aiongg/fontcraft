//! Removing hints (M12.27; rows `dehint`, `hints.remove`). Oracles: `tx -n`, pyftsubset
//! `--no-hinting`. CFF: stems, masks and flex; TrueType: glyph instructions, `fpgm`, `prep`,
//! `cvt `, `hdmx`, `LTSH`, `VDMX`.

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// What [`dehint`] keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DehintOptions {
    /// Keep `gasp` as it is; otherwise it is rewritten to one smoothing range.
    pub keep_gasp: bool,
}

/// The font without hints.
pub fn dehint(font: &SfntFont, options: &DehintOptions) -> Result<SfntFont> {
    // M12.27
    let _ = (font, options);
    Err(TablesError::NotBuilt("dehint"))
}

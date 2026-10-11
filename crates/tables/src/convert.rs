//! Format conversion of compiled fonts (M12.30; rows `convert`, `font.convert`). Oracles: AFDKO
//! otf2ttf and `tx`, fontTools `cffLib.CFFToCFF2` and `CFF2ToCFF`, `ttx`. Type 1 and bare CFF
//! reading come from `fontcraft-decompile` (M12.31).

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// What [`convert`] writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConvertTarget {
    /// TrueType outlines, cubic curves converted within `max_error` font units.
    TrueType { max_error: f64 },
    /// `CFF ` outlines.
    Cff,
    /// `CFF2` outlines.
    Cff2,
    /// A bare CFF table, outside any sfnt.
    BareCff,
}

/// The converted file's bytes: an sfnt font, or a bare CFF.
pub fn convert(font: &SfntFont, target: ConvertTarget) -> Result<Vec<u8>> {
    // M12.30
    let _ = (font, target);
    Err(TablesError::NotBuilt("convert"))
}

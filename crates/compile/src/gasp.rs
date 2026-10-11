//! The `gasp` table: rendering behaviour by size.
//!
//! Source order: the gasp Table parameter (instance, then font) or a UFO's
//! `openTypeGaspRangeRecords`. A TrueType font with instructions and no such source gets the
//! handbook's thresholds 8 and 20 with DOGRAY + SYMMETRIC_SMOOTHING up to 8 PPM, GRIDFIT +
//! SYMMETRIC_GRIDFIT up to 20 and all four bits above (a guess on the flags); an unhinted font
//! without a source gets none, as fontc and fontmake do. A key above 65535 is dropped with a
//! warning, as fontc does. Version 1 table. Rows: the `gasp` table row,
//! `hinting.tt.param.gaspTable`, `hinting.tt.gasp.ranges`, `hinting.tt.gasp.flagMapping`,
//! `gasp Table`. M10.20 fills the body; the hook in `build.rs` is in place.

use fontcraft_model::Resolved;
use write_fonts::tables::gasp::Gasp;

use crate::CompileError;
use crate::outline::Flavour;

/// Most ranges read from a source.
pub const MAX_GASP_RANGES: usize = 256;

/// The `gasp` table for `font`, or `None` when it gets none. `hinted`: the font carries TrueType
/// instructions.
pub fn gasp_table(font: &Resolved, flavour: Flavour, hinted: bool, warnings: &mut Vec<String>) -> Result<Option<Gasp>, CompileError> {
    // M10.20: the source's records, else the default for hinted TrueType fonts.
    let _ = (font, flavour, hinted, warnings);
    Ok(None)
}

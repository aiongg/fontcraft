//! Glyph programs: a layer's TrueType instructions compiled to `glyf` bytecode against the
//! exported point numbers and the `cvt ` layout, calling the `fpgm` functions of
//! [`crate::tt_font`].
//!
//! Rows: `hinting.tt.snap.*`, `hinting.tt.stem.*` (bytecode), `hinting.tt.shift.rounding`,
//! `hinting.tt.interpolate.rounding`, `hinting.tt.iup`, `hinting.tt.rounding.modes`,
//! `hinting.tt.delta.restrictions`, `Ignore Vertical Hints`, `TTFDontPreserveDiagonals`. M10.18
//! fills the body.

use crate::{CvtLayout, HintSettings, HintingError, PointMap, TtInstruction};

/// The glyph program of `instructions` (in execution order), at most
/// [`fontcraft_ttasm::MAX_GLYPH_PROGRAM_BYTES`]. Empty instructions give an empty program. A
/// point without an exported number is [`HintingError::Compile`] naming the instruction.
pub fn glyph_program(
    glyph: &str,
    instructions: &[TtInstruction],
    points: &dyn PointMap,
    layout: &CvtLayout,
    settings: &HintSettings,
) -> Result<Vec<u8>, HintingError> {
    if instructions.is_empty() {
        return Ok(Vec::new());
    }
    // M10.18: the compiler.
    let _ = (glyph, points, layout, settings);
    Err(HintingError::NotYetImplemented("compiling TrueType instructions"))
}

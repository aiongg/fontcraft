//! TrueType hinting at export: `cvt `, `fpgm`, `prep` and the glyph programs.
//!
//! Autohint off: each main-master layer's instructions compile to a glyph program against the
//! exported point numbers ([`crate::points`]); glyphs without instructions get none; `cvt `,
//! `fpgm`, `prep` are written when any glyph has instructions or the masters carry imported
//! tables. Autohint on: the autohinter replaces every manual instruction. `maxp` takes the
//! largest limits `fontcraft_ttasm::analyse` reports (`docs/plans/M10.md` § 3 › TrueType manual
//! export). M10.19 fills the bodies and the hook in `build.rs`; M10.21 adds the autohinter and
//! the external-program path (desktop only). Rows: `hinting.tt.export.*`, `Export TrueType
//! Hints`, `cvt Table`, `prep Table Assembly`, `fpgm Table Assembly`, `compile.ttfAutohintPostStep`.

use fontcraft_hinting::{FontPrograms, HintSettings};
use fontcraft_model::{Font, ResolvedGlyph};
use fontcraft_ttasm::ProgramLimits;

use crate::CompileError;
use crate::outline::Prepared;

/// The TrueType hinting of a font: the font programs and one program per glyph in glyph order
/// (empty for glyphs without instructions).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TtHinting {
    pub programs: FontPrograms,
    pub glyph_programs: Vec<Vec<u8>>,
    /// The merged limits for `maxp`.
    pub limits: ProgramLimits,
}

/// The TrueType hinting for the glyphs of `order` with outlines `outlines`, or `None` when the
/// font gets none.
pub fn tt_hinting(
    font: &Font,
    settings: &HintSettings,
    order: &[ResolvedGlyph],
    outlines: &[Prepared],
    warnings: &mut Vec<String>,
) -> Result<Option<TtHinting>, CompileError> {
    // M10.19: manual instructions; M10.21: the autohinter.
    let _ = (font, settings, order, outlines, warnings);
    Ok(None)
}

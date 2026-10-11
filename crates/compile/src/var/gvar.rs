//! The `gvar` table (M9.5).
//!
//! Rows: the `gvar` table row, `compile.gvarIupOptimisation`, `compile.gvarEveryGlyph`,
//! `compile.gvarPhantomPoints`, `compile.gvarSharedTuples`, parameters `Optimize Variable Deltas`,
//! `Variable Font Optimize Deltas`.
//!
//! Contract: every glyph of the font gets an entry (empty when it does not vary). A glyph's
//! deltas come from its sources through its own sub-model (`fontcraft_geom::variation`), the four
//! phantom points included; with optimization on, deltas IUP can infer within
//! [`IUP_TOLERANCE`] are left out. Peak tuples used by several glyphs are shared (at most 4,095),
//! and a glyph has at most 4,095 tuple variations; points per glyph are bounded by
//! [`crate::MAX_TT_POINTS`].

use write_fonts::tables::gvar::Gvar;

use crate::CompileError;

/// How far, in font units, an IUP-inferred delta may be from the real one.
pub const IUP_TOLERANCE: f64 = 0.5;
/// Most peak tuples shared between glyphs (the format's 12-bit index).
pub const MAX_SHARED_TUPLES: usize = 4_095;
/// Most tuple variations one glyph may have (the format's 12-bit count).
pub const MAX_GLYPH_TUPLES: usize = 4_095;

/// One glyph's points at each of its sources, in the same order in every source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceOutlines {
    pub glyph: String,
    /// Normalized location of each source; the first is the origin (all zeros).
    pub locations: Vec<Vec<f64>>,
    /// Per source, every point of the glyph followed by the four phantom points.
    pub points: Vec<Vec<(f64, f64)>>,
    /// Index of the last point of each contour (for IUP).
    pub contour_ends: Vec<usize>,
}

/// How `gvar` is written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GvarOptions {
    /// Leave out deltas IUP can infer.
    pub optimize: bool,
}

impl Default for GvarOptions {
    fn default() -> Self {
        Self { optimize: true }
    }
}

/// The `gvar` table of `glyphs` (one per glyph, in glyph order).
pub fn build_gvar(axis_count: u16, glyphs: &[SourceOutlines], options: &GvarOptions) -> Result<Gvar, CompileError> {
    // M9.5
    let _ = (axis_count, glyphs, options);
    Err(CompileError::NotYetImplemented("the gvar table"))
}

//! Charstring hints: stem hints, hint masks, counter masks and flex, as plain data the encoder
//! writes into a Type 2 charstring (`hstem`/`hstemhm`, `vstem`/`vstemhm`, `hintmask`,
//! `cntrmask`, `flex`, `hflex`, `hflex1`, `flex1`).
//!
//! The types are fixed by M10.1; M10.14 writes the operators in `charstring.rs` and fills
//! [`CharstringHints::validate`] beyond the limits checked here. Rows:
//! `hinting.ps.export.stemHintsMadePositive`, `hinting.ps.export.hintReplacement`,
//! `hinting.ps.flex.*`.

use crate::Error;

/// Most stem hints in one charstring, both directions together (Type 2 limit).
pub const MAX_STEM_HINTS: usize = 96;

/// One stem hint in absolute font units: the lower (or left) edge and the width. A ghost hint
/// has width −20 (top edge) or −21 (bottom edge), with `edge` at the hinted edge plus the width
/// as the format requires.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StemHint {
    pub edge: f64,
    pub width: f64,
}

/// Which stems are on: parallel to [`CharstringHints::h_stems`] and
/// [`CharstringHints::v_stems`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mask {
    pub h: Vec<bool>,
    pub v: Vec<bool>,
}

/// The hints of one glyph. Stems are written sorted by edge, as the format requires; masks and
/// flex name path operations by their index in [`crate::Glyph::path`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharstringHints {
    pub h_stems: Vec<StemHint>,
    pub v_stems: Vec<StemHint>,
    /// A mask that takes over before path operation `op` (a `hintmask`). A mask at op 0, or at a
    /// `MoveTo`, is written before the moveto. Empty: no hint replacement (`hstem`/`vstem`).
    pub masks: Vec<(usize, Mask)>,
    /// Counter masks, written after the stems (`cntrmask`).
    pub counters: Vec<Mask>,
    /// Index of the first of two `CurveTo` operations that form a flex.
    pub flex: Vec<usize>,
}

impl CharstringHints {
    pub fn is_empty(&self) -> bool {
        self.h_stems.is_empty() && self.v_stems.is_empty()
    }

    /// Checks the hints against the glyph's `path_len` operations: the stem limit, finite
    /// numbers, mask lengths, operation indexes in range.
    pub fn validate(&self, glyph: &str, path_len: usize) -> Result<(), Error> {
        let bad = |message: String| Error::BadHints { glyph: glyph.to_string(), message };
        let stems = self.h_stems.len() + self.v_stems.len();
        if stems > MAX_STEM_HINTS {
            return Err(bad(format!("{stems} stem hints, more than {MAX_STEM_HINTS}")));
        }
        if self.h_stems.iter().chain(&self.v_stems).any(|s| !s.edge.is_finite() || !s.width.is_finite()) {
            return Err(bad("a stem hint is not a finite number".into()));
        }
        for m in self.masks.iter().map(|(_, m)| m).chain(&self.counters) {
            if m.h.len() != self.h_stems.len() || m.v.len() != self.v_stems.len() {
                return Err(bad("a mask does not match the stems".into()));
            }
        }
        if self.masks.iter().map(|(op, _)| *op).chain(self.flex.iter().copied()).any(|op| op >= path_len.max(1)) {
            return Err(bad("a mask or flex names a path operation the glyph does not have".into()));
        }
        // M10.14: masks in path order, flex on two curves.
        Ok(())
    }
}

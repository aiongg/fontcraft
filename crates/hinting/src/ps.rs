//! PostScript hints on a layer: stem hints and ghost hints, parsed from and written back to the
//! stored hint values.
//!
//! Forms read (`docs/plans/M10.md` § 2): `Stem` with a horizontal flag and `origin`/`target` as
//! node references or `place` = (position, width); `TopGhost` (target `down`) and `BottomGhost`
//! (target `up`) in node and `place` forms; format 2 strings `"{0, 0}"`. Rows:
//! `hinting.ps.twoLevels`, `hinting.ps.manual.hintKinds`. M10.2 fills the bodies marked
//! `// M10.2:`.

use fontcraft_model::plist::Value;
use fontcraft_model::{Hint, LayerInfo};

use crate::{Edge, HintingError, MAX_LAYER_HINTS};

/// The kind of a PostScript hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsHintKind {
    Stem,
    /// A ghost hint at a top edge (the format's `TopGhost`).
    GhostTop,
    /// A ghost hint at a bottom edge (`BottomGhost`).
    GhostBottom,
}

/// A PostScript hint.
#[derive(Clone, Debug, PartialEq)]
pub struct PsHint {
    /// Measured vertically: a horizontal stem (`hstem`), its edges are y values.
    pub horizontal: bool,
    pub kind: PsHintKind,
    /// The first edge; for a ghost, its only edge.
    pub origin: Edge,
    /// The second edge of a stem; `None` for ghosts.
    pub other: Option<Edge>,
    /// The stored value this hint was read from. [`set_ps_hints`] writes only the keys the typed
    /// fields cover over it, so unknown keys and the stored form (format 2 strings, `place`)
    /// survive. `None` for a new hint.
    pub source: Option<Value>,
}

impl PsHint {
    /// The hint's (bottom or left, top or right) edges on a layer, ghosts as the edge twice.
    pub fn edges(&self, layer: &fontcraft_model::Layer) -> Result<(f64, f64), HintingError> {
        let a = self.origin.position(layer, self.horizontal)?;
        let b = match &self.other {
            Some(e) => e.position(layer, self.horizontal)?,
            None => a,
        };
        Ok(if a <= b { (a, b) } else { (b, a) })
    }

    /// Reads one stored hint; `None` when it is not a PostScript hint.
    pub fn from_hint(index: usize, hint: &Hint) -> Option<Result<Self, HintingError>> {
        // M10.2: every form of § 2.
        let _ = (index, hint);
        None
    }

    /// The stored value of this hint, over [`PsHint::source`].
    pub fn to_hint(&self, family: fontcraft_model::Family) -> Hint {
        // M10.2: write back in the same keys.
        Hint { family, value: self.source.clone().unwrap_or(Value::Dict(Default::default())) }
    }
}

/// The layer's PostScript hints in stored order, each with its index in
/// [`LayerInfo::hints`].
pub fn ps_hints(info: &LayerInfo) -> Result<Vec<(usize, PsHint)>, HintingError> {
    if info.hints.len() > MAX_LAYER_HINTS {
        return Err(HintingError::LimitExceeded { what: "hints on a layer", limit: MAX_LAYER_HINTS });
    }
    info.hints.iter().enumerate().filter_map(|(i, h)| PsHint::from_hint(i, h).map(|r| r.map(|p| (i, p)))).collect()
}

/// Replaces the layer's PostScript hints by `hints`, in order, at the place of the first old one
/// (at the end when there was none). TrueType instructions and other stored hints keep their
/// order. More than [`MAX_LAYER_HINTS`] in all is an error and changes nothing.
pub fn set_ps_hints(info: &mut LayerInfo, hints: &[PsHint]) -> Result<(), HintingError> {
    // M10.2: splice; the family of new hints follows the layer's other hints, else Glyphs.
    let _ = (info, hints);
    Err(HintingError::NotYetImplemented("writing PostScript hints"))
}

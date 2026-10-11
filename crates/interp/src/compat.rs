//! The compatibility check: whether a glyph's layers can interpolate (M9.3).
//!
//! Rows: `compatibility.rules`, `compatibility.checkScope`, `compatibility.exportBlocked` (the
//! check), `compile.interpolationCompatibility`, `intermediateLayer.mustBeCompatible`,
//! `alternateLayer.compatibilityAmongThemselves`, parameter `Enforce Compatibility Check`.
//!
//! Contract:
//! - Two layers are compatible when they have the same shapes in the same order (paths and
//!   components counted together), each path the same node count and the same sequence of line
//!   and curve segments, each component the same base glyph, and the same anchors by count and
//!   name.
//! - Main sources are checked against each other; each alternate set against itself; each
//!   intermediate layer against every master layer (a guess on
//!   `intermediateLayer.mustBeCompatible`).
//! - Only masters in the [`MasterPairs`] scope count: masters with a non-zero weight at some
//!   instance's location plus every master of a variable font setting not disabled; Enforce
//!   Compatibility Check on widens the scope to every master, off hides the indicators.
//! - Compatible layers whose corresponding handles turn by more than
//!   [`crate::ANGLE_WARNING_DEGREES`] get an [`AngleWarning`]; warnings do not make a glyph
//!   incompatible.
//! - The check never fails: a glyph it cannot read is reported as incompatible.

use std::fmt;

use fontcraft_model::Font;

use crate::designspace::Designspace;
use crate::sources::GlyphSources;

/// What differs between two layers. Shape and node indices count from 0.
#[derive(Clone, Debug, PartialEq)]
pub enum MismatchKind {
    /// Different numbers of shapes (paths and components together).
    ShapeCount { a: usize, b: usize },
    /// A path in one layer where the other has a component, at shape index `shape`.
    ShapeKind { shape: usize },
    /// Paths at shape index `shape` with different node counts.
    NodeCount { shape: usize, a: usize, b: usize },
    /// Paths at shape index `shape` whose segments differ (line against curve) at node `node`.
    SegmentType { shape: usize, node: usize },
    /// Components at shape index `shape` that refer to different glyphs.
    ComponentGlyph { shape: usize, a: String, b: String },
    /// Different numbers of anchors.
    AnchorCount { a: usize, b: usize },
    /// An anchor name one layer has and the other lacks.
    AnchorName { name: String },
}

/// The first difference between two layers of a glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    pub glyph: String,
    /// Layer ids.
    pub layer_a: String,
    pub layer_b: String,
    pub kind: MismatchKind,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // M9.3: a message per kind.
        write!(f, "layers `{}` and `{}` differ", self.layer_a, self.layer_b)
    }
}

/// Corresponding handles of two compatible layers that turn by more than
/// [`crate::ANGLE_WARNING_DEGREES`].
#[derive(Clone, Debug, PartialEq)]
pub struct AngleWarning {
    pub layer_a: String,
    pub layer_b: String,
    pub shape: usize,
    pub node: usize,
    /// The turn, in degrees.
    pub degrees: f64,
}

/// The result of checking one glyph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compat {
    /// Every pair of layers in scope is compatible.
    pub ok: bool,
    /// The first difference of each incompatible pair of layers.
    pub first_mismatch: Vec<Mismatch>,
    pub warnings: Vec<AngleWarning>,
}

/// The masters whose layers must be compatible with each other.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterPairs {
    /// Master ids, in the font's order.
    pub masters: Vec<String>,
    /// Whether the editor shows incompatibility indicators (Enforce Compatibility Check off hides
    /// them).
    pub indicators: bool,
}

/// Checks `sources` (one glyph's) within `scope`.
pub fn check_glyph(font: &Font, sources: &GlyphSources, scope: &MasterPairs) -> Compat {
    // M9.3
    let _ = (font, sources, scope);
    Compat { ok: true, first_mismatch: Vec::new(), warnings: Vec::new() }
}

/// The compatibility scope of `font` in `designspace`.
pub fn scope(font: &Font, designspace: &Designspace) -> MasterPairs {
    // M9.3
    let _ = font;
    MasterPairs { masters: designspace.masters().iter().map(|m| m.id.clone()).collect(), indicators: true }
}

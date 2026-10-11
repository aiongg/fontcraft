//! TrueType instructions on a layer: Snap, Stem, Shift, Interpolate and Delta, parsed from and
//! written back to the stored hint values. List order is execution order.
//!
//! Forms read (`docs/plans/M10.md` § 2): `TTStem` (origin and target as node references, as
//! 4-tuples, or `lsb`/`rsb`; `stem` index; `options`), `TTSnap`, `TTShift`, `TTInterpolate`
//! (with `other1`), `TTDelta` (`settings` = `deltaV` per master and PPM). Rows:
//! `hinting.tt.hintOrder`, `hinting.tt.rounding.modes`, `hinting.tt.snap.*`,
//! `hinting.tt.stem.*`, `hinting.tt.shift.*`, `hinting.tt.interpolate.*`,
//! `hinting.tt.delta.restrictions`. M10.2 fills the bodies marked `// M10.2:`.

use fontcraft_model::plist::Value;
use fontcraft_model::{Hint, LayerInfo};

use crate::settings::Deltas;
use crate::{HintingError, MAX_LAYER_HINTS, TtPoint};

/// The hinting direction of an instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Moves points along y (the Instructor's default).
    #[default]
    Vertical,
    /// Moves points along x.
    Horizontal,
}

/// How an instruction rounds (`hinting.tt.rounding.modes`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rounding {
    /// To the nearest pixel edge.
    #[default]
    Round,
    /// To the edge above (vertical) or to the right (horizontal).
    Up,
    /// To the edge below or to the left.
    Down,
    /// No rounding; the point still counts as touched.
    NoRounding,
    /// Round in the GDI ClearType intent, no rounding in the others.
    GdiOnly,
}

/// The zone a Snap uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ZoneRef {
    /// The zone the point lies in, if any.
    #[default]
    Auto,
    NoZone,
    /// Index into the main master's TT zones ([`crate::HintSettings::tt_zones`]).
    Index(usize),
}

/// The stem a Stem instruction uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum StemRef {
    /// The closest TT stem of the direction.
    #[default]
    Auto,
    NoStem,
    /// Index into the main master's TT stems ([`crate::HintSettings::tt_stems`]).
    Index(usize),
}

/// The type of an instruction and what only that type carries.
#[derive(Clone, Debug, PartialEq)]
pub enum TtKind {
    Snap {
        zone: ZoneRef,
    },
    Stem {
        stem: StemRef,
        /// Part of a triple hint: the id the three linked Stem instructions share.
        triple: Option<u32>,
    },
    Shift,
    Interpolate,
    /// Per-PPM corrections, by master or instance id ([`Deltas`]); vertical only.
    Delta {
        cells: Deltas,
    },
}

/// One TrueType instruction.
#[derive(Clone, Debug, PartialEq)]
pub struct TtInstruction {
    pub kind: TtKind,
    pub direction: Direction,
    /// Not used by Delta.
    pub rounding: Rounding,
    /// The point the instruction moves.
    pub target: TtPoint,
    /// The reference point of Stem and Shift; the first reference of Interpolate.
    pub origin: Option<TtPoint>,
    /// The second reference of Interpolate (`other1`).
    pub other: Option<TtPoint>,
    /// The stored value this instruction was read from; [`set_tt_instructions`] writes only the
    /// keys the typed fields cover over it. `None` for a new instruction.
    pub source: Option<Value>,
}

impl TtInstruction {
    /// The default rounding of each type: Snap rounds, the others do not (the handbook's table).
    pub fn default_rounding(kind: &TtKind) -> Rounding {
        match kind {
            TtKind::Snap { .. } => Rounding::Round,
            _ => Rounding::NoRounding,
        }
    }

    /// Every point the instruction names, references first, target last.
    pub fn points(&self) -> Vec<TtPoint> {
        self.origin.iter().chain(&self.other).copied().chain(std::iter::once(self.target)).collect()
    }

    /// Reads one stored hint; `None` when it is not a TrueType instruction.
    pub fn from_hint(index: usize, hint: &Hint) -> Option<Result<Self, HintingError>> {
        // M10.2: every form of § 2.
        let _ = (index, hint);
        None
    }

    /// The stored value of this instruction, over [`TtInstruction::source`].
    pub fn to_hint(&self, family: fontcraft_model::Family) -> Hint {
        // M10.2: write back in the same keys.
        Hint { family, value: self.source.clone().unwrap_or(Value::Dict(Default::default())) }
    }
}

/// The layer's TrueType instructions in execution order, each with its index in
/// [`LayerInfo::hints`].
pub fn tt_instructions(info: &LayerInfo) -> Result<Vec<(usize, TtInstruction)>, HintingError> {
    if info.hints.len() > MAX_LAYER_HINTS {
        return Err(HintingError::LimitExceeded { what: "hints on a layer", limit: MAX_LAYER_HINTS });
    }
    info.hints.iter().enumerate().filter_map(|(i, h)| TtInstruction::from_hint(i, h).map(|r| r.map(|t| (i, t)))).collect()
}

/// Replaces the layer's TrueType instructions by `instructions`, in order, at the place of the
/// first old one (at the end when there was none); other stored hints keep their order. More
/// than [`MAX_LAYER_HINTS`] in all is an error and changes nothing.
pub fn set_tt_instructions(info: &mut LayerInfo, instructions: &[TtInstruction]) -> Result<(), HintingError> {
    // M10.2: splice.
    let _ = (info, instructions);
    Err(HintingError::NotYetImplemented("writing TrueType instructions"))
}

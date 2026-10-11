//! A writer for the `CFF2` table of a variable OpenType font (M9.13).
//!
//! Rows: `cff2.export`, the `CFF2` table row, `compile.cff2VariableBuild`.
//!
//! Contract: [`write_cff2`] turns a [`Cff2Font`] (the variation regions, the region lists each
//! glyph's blends use, and per glyph an outline whose numbers are a default plus one delta per
//! region of its list) into the bytes of a `CFF2` table, as fontmake (ufo2ft and fontTools'
//! `varLib`) writes it:
//!
//! - Charstrings with `blend` operators and `vsindex` where a glyph uses another region list;
//!   coordinates rounded with the static writer's rule; no hints and no subroutines at first.
//! - A Top DICT with the FontMatrix only where it differs from the default; an FDArray of one
//!   Font DICT whose Private DICT may hold blended values; the item variation store (`vstore`).
//! - No glyph names: they go in the `post` table, which the compiler writes.
//!
//! Every input is untrusted: counts are checked against the limits below and the static writer's
//! ([`crate::MAX_GLYPHS`], [`crate::MAX_SEGMENTS`], [`crate::MAX_COORDINATE`]); anything out of range
//! returns a [`Cff2Error`]; nothing panics.

use std::fmt;

use crate::Error;

/// Most regions the variation store may hold (a 16-bit count).
pub const MAX_REGIONS: usize = 65_535;
/// Most region lists (`vsindex` values) the variation store may hold (a 16-bit count).
pub const MAX_REGION_LISTS: usize = 65_535;
/// Most operands the charstring interpreter's stack holds (CFF2's `maxstack` default): a `blend`
/// of `n` values with `k` regions needs `n * (k + 1) + 1`.
pub const MAX_STACK: usize = 513;

/// One axis of a variation region, in normalized coordinates (`start <= peak <= end`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RegionAxis {
    pub start: f64,
    pub peak: f64,
    pub end: f64,
}

/// A number that varies: its value at the origin and one delta per region of the glyph's list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blend {
    pub default: f64,
    pub deltas: Vec<f64>,
}

/// A point whose coordinates vary.
pub type BlendPoint = (Blend, Blend);

/// One drawing step of a variable outline, as [`crate::PathOp`] with varying points.
#[derive(Clone, Debug, PartialEq)]
pub enum BlendOp {
    MoveTo(BlendPoint),
    LineTo(BlendPoint),
    CurveTo(BlendPoint, BlendPoint, BlendPoint),
}

/// A glyph of a `CFF2` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cff2Glyph {
    /// For messages only; `CFF2` stores no names.
    pub name: String,
    /// The region list its blends use (an index into [`Cff2Font::region_lists`]).
    pub vsindex: u16,
    pub path: Vec<BlendOp>,
}

/// Private DICT values; each may vary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cff2Private {
    pub blue_values: Vec<Blend>,
    pub other_blues: Vec<Blend>,
    pub std_hw: Option<Blend>,
    pub std_vw: Option<Blend>,
}

/// Everything a `CFF2` table holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cff2Font {
    pub axis_count: u16,
    /// Each region has one entry per axis.
    pub regions: Vec<Vec<RegionAxis>>,
    /// Region lists: indices into `regions`. List 0 is the default `vsindex`.
    pub region_lists: Vec<Vec<u16>>,
    /// In glyph order; glyph 0 is `.notdef`.
    pub glyphs: Vec<Cff2Glyph>,
    pub private: Cff2Private,
    /// The FontMatrix, when the units per em are not 1000.
    pub font_matrix: Option<[f64; 6]>,
    /// As [`crate::Font::round_tolerance`].
    pub round_tolerance: f64,
}

/// Why a `CFF2` table could not be built.
#[derive(Clone, Debug, PartialEq)]
pub enum Cff2Error {
    /// A check the static writer shares (glyph counts, coordinates, segments).
    Cff(Error),
    /// A glyph's blends do not have one delta per region of its list, or name a list that does
    /// not exist.
    BadBlend { glyph: String, reason: String },
    /// More regions or region lists than the variation store can hold.
    TooManyRegions(usize),
    /// A blend that needs more operands than [`MAX_STACK`].
    StackOverflow { glyph: String },
    /// A function whose M9 package has not been written yet.
    NotYetImplemented(&'static str),
}

impl fmt::Display for Cff2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cff2Error::Cff(e) => write!(f, "{e}"),
            Cff2Error::BadBlend { glyph, reason } => write!(f, "glyph `{glyph}`: {reason}"),
            Cff2Error::TooManyRegions(n) => write!(f, "{n} variation regions is more than a CFF2 table can hold ({MAX_REGIONS})"),
            Cff2Error::StackOverflow { glyph } => write!(f, "glyph `{glyph}` varies more values at once than a CFF2 charstring can hold"),
            Cff2Error::NotYetImplemented(what) => write!(f, "{what} is not available yet"),
        }
    }
}

impl std::error::Error for Cff2Error {}

impl From<Error> for Cff2Error {
    fn from(e: Error) -> Self {
        Cff2Error::Cff(e)
    }
}

/// The bytes of the `CFF2` table for `font`.
pub fn write_cff2(font: &Cff2Font) -> Result<Vec<u8>, Cff2Error> {
    // M9.13
    let _ = font;
    Err(Cff2Error::NotYetImplemented("CFF2 tables"))
}

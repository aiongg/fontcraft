//! FontCraft's PostScript autohinter (L1, `geom` only): pure functions from a glyph's contours
//! and a set of alignment zones and stems to stem hints, hint masks, counter masks and flex.
//! A port of `otfautohint` from AFDKO 5.0.1 (Apache-2.0, Copyright 2021 Adobe; see `NOTICE`),
//! its defaults kept, measured glyph by glyph against otfautohint itself
//! (`docs/plans/M10.md` § 3, § 4 › M10.8 to M10.13, § 5). Rows: the `autohint.ps.*` rows,
//! `hinting.ps.flex.*`, `hinting.ps.export.stemHintsMadePositive`,
//! `hinting.ps.export.hintReplacement`.
//!
//! # Public API
//!
//! - [`FdDict::new`] validates a [`ZoneSet`] and a [`StemSet`] once per font (or per override
//!   set); [`hint_glyph`] hints one glyph against it; [`hint_masters`] hints the same glyph in
//!   several compatible masters so that every master gets the same stems in the same order and
//!   the same masks.
//! - [`report::report_stems`] and [`report::report_zones`] give otfautohint's stem and zone
//!   reports; [`fontinfo::parse`] and [`fontinfo::write`] read and write its override sets.
//! - Overlap removal needs boolean path operations, which live above this layer: the caller
//!   passes the overlap-free contours in [`HintInput::without_overlap`] (the compiler computes
//!   them with `fontcraft-pathops`).
//!
//! # Modules and the packages that fill them
//!
//! Each port package owns its files and fills the function bodies there; the data types are fixed
//! here so the six packages can be written at the same time. A function marked *shared* in its
//! doc comment is called from another package and keeps its signature; the others may change
//! inside their package.
//!
//! | Module | Package | Ported from |
//! |---|---|---|
//! | [`glyph`] | M10.8 | `glyphData.py`: `pt`, `stem`, `pathElement`, `glyphData` and the path walk |
//! | [`bezier`] | M10.8 | `glyphData.py`: `boundsState`, `pathBoundsState`, the curve maths of `pathElement` |
//! | [`segments`] | M10.9 | `hinter.py` lines 78–1053: segments, bends, extremes (`dimensionHinter`) |
//! | [`flex`] | M10.9 | `hinter.py`: `addFlex`, `tryFlex`, `markFlex`, `linearFlexOK` |
//! | [`overlap`] | M10.9 | `overlap.py` (association of the overlap-free outline) |
//! | [`stems`] | M10.10 | `hinter.py` lines 1073–1904: stem values, pruning, merging, main values |
//! | [`dimension`] | M10.10 | `hinter.py` lines 2277–2420: the `hhinter` and `vhinter` specifics |
//! | [`masks`] | M10.11 | `hinter.py` lines 1904–2277: instance stems, conflicts, hint masks |
//! | [`state`] | M10.11 | `hintstate.py`: segments, stem values, per-element state, links |
//! | [`driver`] | M10.12 | `hinter.py` lines 2420–2992 (`glyphHinter`), `autohint.py` |
//! | [`masters`] | M10.12 | `hinter.py`: `compatiblePaths`, `otherInstanceStems`, `otherInstanceMasks` |
//! | [`fontinfo`] | M10.13 | `fdTools.py`, `splitpsdicts.py` |
//! | [`report`] | M10.13 | `report.py` |
//! | [`fddict`] | M10.1 (types), M10.13 ([`FdDict::new`]) | `fdTools.py`: `FDDict`, `buildBlueLists` |
//! | [`hinter`] | M10.1 | the shared hinter state and otfautohint's constants |
//! | [`options`], [`error`] | M10.1 | `autohint.py`: `ACOptions` |
//!
//! # How the Python maps to Rust
//!
//! - Object references become indexes. Path elements are named by [`glyph::PeRef`] (subpath,
//!   offset, substitution index); segments and stem values live in arenas of
//!   [`state::GlyphHintState`] and are named by [`state::SegId`] and [`state::StemId`]. Arenas
//!   only grow while one glyph is hinted; deletion is a flag, as in the original.
//! - `pt.a` and `pt.o` read a thread-local alignment in Python. Here every such read names the
//!   dimension: [`glyph::Pt::a`] and [`glyph::Pt::o`] take a [`Dim`].
//! - `dimensionHinter` with its subclasses `hhinter` and `vhinter` becomes one state struct per
//!   dimension, [`hinter::DimState`], and a short-lived view [`hinter::DimHinter`] that pairs it
//!   with the glyph data; the methods of `dimensionHinter` are `impl DimHinter` blocks spread over
//!   the package modules. `glyphHinter` is [`hinter::GlyphHinter`].
//! - Python numbers are `f64`. Logging becomes warnings collected in [`GlyphHints::warnings`].
//!
//! Limits are named in [`error`]; nothing panics on any input.
#![forbid(unsafe_code)]

pub mod bezier;
pub mod dimension;
pub mod driver;
pub mod error;
pub mod fddict;
pub mod flex;
pub mod fontinfo;
pub mod glyph;
pub mod hinter;
pub mod masks;
pub mod masters;
pub mod options;
pub mod overlap;
pub mod report;
pub mod segments;
pub mod state;
pub mod stems;

pub use driver::hint_glyph;
pub use error::{
    AutohintError, MAX_COUNTER_GLYPHS, MAX_ELEMENTS, MAX_FONTINFO_BYTES, MAX_MASTERS, MAX_OVERRIDE_SETS, MAX_SEGMENTS, MAX_STEMS, MAX_ZONES,
};
pub use fddict::{BlueValue, FdDict, StemSet, Zone, ZoneSet};
pub use glyph::{Dim, Stem};
pub use masters::hint_masters;
pub use options::Options;

use fontcraft_geom::BezPath;

/// One glyph to hint.
#[derive(Clone, Copy, Debug)]
pub struct HintInput<'a> {
    /// The glyph name: options and override sets select glyphs by name.
    pub name: &'a str,
    /// Closed cubic contours in font units, in drawing order and direction. Quadratic segments
    /// must be raised to cubics first; open contours are left out by the caller.
    pub contours: &'a [BezPath],
    /// The same outline with overlaps removed, when the caller removed them; `None` when the
    /// outline has no overlap or overlap handling is off.
    pub without_overlap: Option<&'a [BezPath]>,
    /// The zones and stems in force for this glyph (its override set, else the font's).
    pub dict: &'a FdDict,
    pub options: &'a Options,
}

/// Where in a glyph a hint mask or flex applies: element `element` of subpath `subpath`, where a
/// subpath's elements are its drawing steps after the `moveto` in order, plus the implicit
/// closing line when the last point is not the start point (`glyph::PathElement::is_close`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementPos {
    pub subpath: usize,
    pub element: usize,
}

/// A hint mask or counter mask: which of the glyph's stems are on, parallel to
/// [`GlyphHints::h`] and [`GlyphHints::v`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HintMask {
    pub h: Vec<bool>,
    pub v: Vec<bool>,
}

/// The autohinter's result for one glyph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphHints {
    /// Horizontal stems (y edges), sorted by bottom edge. A ghost hint has `rt < lb`: a top ghost
    /// is `(edge, edge − 20)`, a bottom ghost `(edge + 21, edge)` (Type 2 charstring format,
    /// page 22).
    pub h: Vec<Stem>,
    /// Vertical stems (x edges), sorted by left edge.
    pub v: Vec<Stem>,
    /// The mask in force from the start of the charstring; `None` when no substitution is needed.
    pub start_mask: Option<HintMask>,
    /// Masks that take over before an element, in path order.
    pub masks: Vec<(ElementPos, HintMask)>,
    /// Counter masks (`cntrmask`), for glyphs in the counter-hinting lists.
    pub counters: Vec<HintMask>,
    /// The first element of each pair of curves joined by a flex hint.
    pub flex: Vec<ElementPos>,
    /// What otfautohint would log at warning level: skipped directions, zones a glyph misses…
    pub warnings: Vec<String>,
}

impl GlyphHints {
    /// No stem in either direction.
    pub fn is_empty(&self) -> bool {
        self.h.is_empty() && self.v.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glyph_without_contours_gets_no_hints() {
        let dict = FdDict::default();
        let options = Options::default();
        let input = HintInput { name: "space", contours: &[], without_overlap: None, dict: &dict, options: &options };
        let hints = hint_glyph(&input).unwrap_or_default();
        assert!(hints.is_empty());
    }
}

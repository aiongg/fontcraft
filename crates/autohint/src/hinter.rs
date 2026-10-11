//! The hinter's shared state and otfautohint's tuning constants.
//!
//! Ported from otfautohint's `hinter.py` (`dimensionHinter.__init__`, `setGlyph`,
//! `resetForHinting`, `glyphHinter.__init__`; AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). Fixed by M10.1.
//!
//! `dimensionHinter` held both its own state and references to the glyphs it shares with the
//! other dimension. Here the state is [`DimState`] (one per dimension, owned by
//! [`GlyphHinter`]), the glyphs are [`Glyphs`], and [`DimHinter`] pairs the two for the length of
//! a call. The methods of `dimensionHinter`, `hhinter` and `vhinter` are `impl DimHinter<'_>`
//! blocks in [`crate::segments`], [`crate::flex`], [`crate::stems`], [`crate::dimension`] and
//! [`crate::masks`]; those of `glyphHinter` are `impl GlyphHinter` blocks in [`crate::driver`]
//! and [`crate::masters`].

use std::sync::Arc;

use crate::Options;
use crate::fddict::{BlueValue, FdDict};
use crate::glyph::{Dim, GlyphData, PeRef};
use crate::report::GlyphReport;
use crate::state::GlyphHintState;

// otfautohint's constants (`dimensionHinter.__init__`), with the original's comments where they
// say why a value is what it is.

/// `(kStackLimit − 2) / 2`, kStackLimit = 46.
pub const STEM_LIMIT: usize = 22;
/// Initial widest stem hinted (`MaxStemDist`, `InitBigDist`).
pub const MAX_STEM_DIST: f64 = 150.0;
pub const BIG_DIST_FACTOR: f64 = 23.0 / 20.0;
/// Must be <= 168 for ITC Garamond Book It p, q, thorn.
pub const MIN_DIST: f64 = 7.0;
pub const GHOST_WIDTH: f64 = 20.0;
pub const GHOST_LENGTH: f64 = 4.0;
pub const GHOST_VAL: f64 = 20.0;
pub const GHOST_SPECIAL: f64 = 2.0;
pub const BEND_LENGTH: f64 = 2.0;
/// 30 sin 30 cos div abs == .57735
pub const BEND_TANGENT: f64 = 0.55735;
/// Must be <= .38 for Ryumin-Light 32.
pub const THETA: f64 = 0.38;
pub const MIN_VALUE: f64 = 0.0039;
pub const MAX_VALUE: f64 = 8_000_000.0;
pub const PRUNE_A: f64 = 50.0;
/// PruneB and PruneValue must be <= 0.01 for Yakout/Light/heM.
pub const PRUNE_B: f64 = 0.0117187;
pub const PRUNE_C: f64 = 100.0;
pub const PRUNE_D: f64 = 1.0;
pub const PRUNE_VALUE: f64 = 0.0117187;
pub const PRUNE_FACTOR: f64 = 3.0;
pub const PRUNE_FACTOR_LT: f64 = 10.0 / 3.0;
pub const PRUNE_DISTANCE: f64 = 10.0;
pub const MUCH_PF: f64 = 50.0;
pub const VERY_MUCH_PF: f64 = 100.0;
pub const CP_FRAC: f64 = 0.4;
pub const CONFLICT_VAL_MIN: f64 = 50.0;
pub const CONFLICT_VAL_MULT: f64 = 20.0;
/// Must be < 46 for Americana-Bold d bowl vs stem hinting.
pub const BAND_MARGIN: f64 = 30.0;
pub const MAX_FLARE: f64 = 10.0;
pub const MAX_BEND_MERGE: f64 = 6.0;
pub const MIN_HINT_ELEMENT_LENGTH: f64 = 12.0;
pub const AUTO_LINEAR_CURVE_FIX: bool = true;
pub const MAX_FLEX: f64 = 20.0;
/// .33² (a ratio of 1:3 or better).
pub const FLEX_LENGTH_RATIO_CUTOFF: f64 = 0.11;
pub const FLEX_CAND: f64 = 4.0;
pub const S_CURVE_TANGENT: f64 = 0.025;
pub const OPPO_FLAT_MAX: f64 = 4.0;
pub const FLAT_MIN: f64 = 50.0;
pub const EXTREMA_DIST: f64 = 20.0;
pub const NEAR_FUZZ: f64 = 6.0;
pub const NO_OVERLAP_PENALTY: f64 = 7.0 / 5.0;
/// The original notes this may have been miscorrected from 40.
pub const GAP_DIST_DENOM: f64 = 40.0;
pub const CLOSE_MERGE: f64 = 20.0;
/// Must be < 3 for Cushing-BookItalic z.
pub const MAX_MERGE: f64 = 2.0;
pub const MIN_HIGH_SEG_VAL: f64 = 1.0 / 16.0;
pub const S_FACTOR: f64 = 20.0;
pub const SPC_BONUS: f64 = 1000.0;
pub const SPECIAL_CHAR_BONUS: f64 = 200.0;
pub const GHOST_FACTOR: f64 = 1.0 / 8.0;
pub const DELTA_DIFF_MIN: f64 = 0.05;
pub const DELTA_DIFF_REPORT: f64 = 3.0;
pub const NO_SEG_SCORE: f64 = 0.1;
pub const FLEX_STRICT: bool = true;

// `glyphHinter.__init__`.
pub const FLARE_VALUE_LIMIT: f64 = 1000.0;
/// The original notes that 10 might better match the old C code.
pub const MAX_HALF_MARGIN: f64 = 20.0;
pub const PROMOTION_DISTANCE: f64 = 50.0;

/// The glyph in every master being hinted (`gllist`): the main master first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glyphs {
    pub main: GlyphData,
    pub others: Vec<GlyphData>,
}

impl Glyphs {
    /// Number of masters.
    pub fn len(&self) -> usize {
        1 + self.others.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Master `i`, 0 being the main one.
    pub fn get(&self, i: usize) -> Option<&GlyphData> {
        if i == 0 { Some(&self.main) } else { self.others.get(i - 1) }
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut GlyphData> {
        if i == 0 { Some(&mut self.main) } else { self.others.get_mut(i - 1) }
    }
}

/// An element in one master (`GlyphPE`): the master's index in [`Glyphs`] and the element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphPe {
    pub glyph: usize,
    pub pe: PeRef,
}

/// One dimension's state while a glyph is hinted (the fields of `dimensionHinter`, `hhinter`
/// and `vhinter`).
#[derive(Clone, Debug)]
pub struct DimState {
    pub dim: Dim,
    pub options: Arc<Options>,
    /// The dictionary of each master, main first (`fddicts`).
    pub fddicts: Vec<Arc<FdDict>>,
    pub report: Option<GlyphReport>,
    pub name: String,
    pub is_multi: bool,
    pub hs: GlyphHintState,
    /// `BigDist`: the widest stem hinted, set by `calcHintValues`.
    pub big_dist: f64,
    pub has_flex: bool,
    pub bonus: f64,
    pub pruning: bool,
    /// Keep the glyph's own hints in this dimension (`keepHints`).
    pub keep_hints: bool,
    /// `hhinter` only: the top and bottom zones (`topPairs`, `bottomPairs`).
    pub top_pairs: Vec<BlueValue>,
    pub bottom_pairs: Vec<BlueValue>,
    /// Warnings for [`crate::GlyphHints::warnings`].
    pub warnings: Vec<String>,
}

impl DimState {
    pub fn new(dim: Dim, options: Arc<Options>) -> Self {
        Self {
            dim,
            options,
            fddicts: Vec::new(),
            report: None,
            name: String::new(),
            is_multi: false,
            hs: GlyphHintState::default(),
            big_dist: MAX_STEM_DIST,
            has_flex: false,
            bonus: 0.0,
            pruning: false,
            keep_hints: false,
            top_pairs: Vec::new(),
            bottom_pairs: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// The main master's dictionary (`fddict`).
    pub fn fddict(&self) -> Option<&FdDict> {
        self.fddicts.first().map(|d| d.as_ref())
    }

    /// Prepares for a new glyph (`setGlyph`).
    pub fn set_glyph(&mut self, fddicts: Vec<Arc<FdDict>>, report: Option<GlyphReport>, masters: usize, name: &str) {
        self.fddicts = fddicts;
        self.report = report;
        self.is_multi = masters > 1;
        self.name = name.to_string();
        self.has_flex = false;
        self.bonus = 0.0;
        self.pruning = true;
        self.hs = GlyphHintState::default();
    }

    /// Prepares to hint the same glyph again (`resetForHinting`).
    pub fn reset_for_hinting(&mut self) {
        self.bonus = 0.0;
        self.pruning = true;
        self.hs = GlyphHintState::default();
    }
}

/// One dimension at work on the glyphs: the `self` of `dimensionHinter`'s methods.
pub struct DimHinter<'a> {
    pub st: &'a mut DimState,
    pub glyphs: &'a mut Glyphs,
}

impl DimHinter<'_> {
    pub fn dim(&self) -> Dim {
        self.st.dim
    }

    pub fn is_v(&self) -> bool {
        self.st.dim.is_v()
    }

    /// The main master's glyph (`self.glyph`).
    pub fn glyph(&self) -> &GlyphData {
        &self.glyphs.main
    }

    pub fn glyph_mut(&mut self) -> &mut GlyphData {
        &mut self.glyphs.main
    }

    /// The same element in every master, main first, for each element of the main glyph
    /// (`glIter`); with `glidx`, the main master and master `glidx` only.
    pub fn masters_elements(&self, glidx: Option<usize>) -> Vec<Vec<GlyphPe>> {
        // M10.9: glIter.
        let _ = glidx;
        Vec::new()
    }
}

/// Hints glyphs in both dimensions and distributes the masks (`glyphHinter`).
#[derive(Clone, Debug)]
pub struct GlyphHinter {
    pub options: Arc<Options>,
    pub h: DimState,
    pub v: DimState,
    pub glyphs: Glyphs,
    /// The dimension `glyphHinter`'s own methods work on (`doV`).
    pub do_v: bool,
}

impl GlyphHinter {
    pub fn new(options: Arc<Options>) -> Self {
        Self { h: DimState::new(Dim::H, options.clone()), v: DimState::new(Dim::V, options.clone()), options, glyphs: Glyphs::default(), do_v: false }
    }

    /// The hinter of one dimension, paired with the glyphs.
    pub fn dim(&mut self, dim: Dim) -> DimHinter<'_> {
        let st = match dim {
            Dim::H => &mut self.h,
            Dim::V => &mut self.v,
        };
        DimHinter { st, glyphs: &mut self.glyphs }
    }

    /// The state of the dimension [`GlyphHinter::do_v`] selects.
    pub fn current(&self) -> &DimState {
        if self.do_v { &self.v } else { &self.h }
    }
}

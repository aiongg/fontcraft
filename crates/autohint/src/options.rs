//! The autohinter's options, with otfautohint's defaults (`autohint.py`, `ACOptions`).
//!
//! Only the options that change hinting are here; file handling, process counts and logging stay
//! with the callers.

use std::collections::BTreeSet;

/// What the autohinter does and which glyphs get special treatment.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Hint glyphs that already carry hints, replacing them (`hintAll`). Off: such glyphs keep
    /// their hints in the dimensions that have some.
    pub hint_all: bool,
    /// Let the hinter reorder subpaths to reduce hint substitution (`allowChanges`).
    pub allow_changes: bool,
    /// No flex hints (`noFlex`).
    pub no_flex: bool,
    /// No hint substitution: one set of hints for the whole glyph (`noHintSub`).
    pub no_hint_sub: bool,
    /// Hint even when the zones or stems are missing (`allowNoBlues`).
    pub allow_no_blues: bool,
    /// Round coordinates before hinting (`roundCoords`).
    pub round_coords: bool,
    /// Drop hints that conflict instead of keeping the stronger (`removeConflicts`).
    pub remove_conflicts: bool,
    /// A direction with more segments than this is left unhinted (`maxSegments`), capped at
    /// [`crate::MAX_SEGMENTS`]. Applies only when the glyphs were not named explicitly.
    pub max_segments: usize,
    /// The glyphs were named explicitly (`explicitGlyphs`): [`Options::max_segments`] is ignored.
    pub explicit_glyphs: bool,
    /// Remove overlaps before hinting: `Some(true)` always, `Some(false)` never, `None` for the
    /// glyphs in [`Options::overlap_list`] and glyphs hinted in several masters
    /// (`overlapForcing`).
    pub overlap_forcing: Option<bool>,
    /// Glyphs whose overlaps are removed when [`Options::overlap_forcing`] is `None`.
    pub overlap_list: BTreeSet<String>,
    /// Match the overlap-free outline to the original loosely (`looseOverlapMapping`).
    pub loose_overlap_mapping: bool,
    /// Only report stems and zones, write no hints (`report_zones`, `report_stems`).
    pub report_zones: bool,
    pub report_stems: bool,
    /// Report curved stems too (`report_all_stems`).
    pub report_all_stems: bool,
    /// Glyphs counter-hinted horizontally (`hCounterGlyphs`).
    pub h_counter_glyphs: BTreeSet<String>,
    /// Glyphs counter-hinted vertically (`vCounterGlyphs`).
    pub v_counter_glyphs: BTreeSet<String>,
    /// Glyphs whose shapes sit in the upper and lower zones in an unusual way
    /// (`upperSpecials`, `lowerSpecials`).
    pub upper_specials: BTreeSet<String>,
    pub lower_specials: BTreeSet<String>,
    /// Glyphs hinted without zones (`noBlues`).
    pub no_blues: BTreeSet<String>,
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hint_all: false,
            allow_changes: false,
            no_flex: false,
            no_hint_sub: false,
            allow_no_blues: false,
            round_coords: true,
            remove_conflicts: true,
            max_segments: crate::MAX_SEGMENTS,
            explicit_glyphs: false,
            overlap_forcing: None,
            overlap_list: BTreeSet::new(),
            loose_overlap_mapping: false,
            report_zones: false,
            report_stems: false,
            report_all_stems: false,
            h_counter_glyphs: names(&["element", "equivalence", "notelement", "divide"]),
            v_counter_glyphs: names(&["m", "M", "T", "ellipsis"]),
            upper_specials: names(&["questiondown", "exclamdown", "semicolon"]),
            lower_specials: names(&["question", "exclam", "colon"]),
            no_blues: names(&["at", "bullet", "copyright", "currency", "registered"]),
        }
    }
}

impl Options {
    /// Only reporting, no hints (`justReporting`).
    pub fn just_reporting(&self) -> bool {
        self.report_zones || self.report_stems
    }

    /// The segment cap in force.
    pub fn segment_limit(&self) -> usize {
        if self.explicit_glyphs { crate::MAX_SEGMENTS } else { self.max_segments.min(crate::MAX_SEGMENTS) }
    }
}

//! FontCraft hinting (L2: `model`, `geom`, `glyphquery`, `autohint`, `ttasm`): typed views of
//! the PostScript hints and TrueType instructions stored on layers, node references and their
//! upkeep, the hint settings of a master or instance, the font-hint checks, the TrueType
//! execution model, the compiler from instructions to bytecode with our own `fpgm`, `prep` and
//! `cvt `, and the TrueType autohinter. Pure functions over a [`fontcraft_model::Font`]; no I/O.
//! Plan: `docs/plans/M10.md` § 3 and § 4. Behaviour notes: `docs/spec/13-postscript-hinting.md`
//! and `docs/spec/14-truetype-hinting.md`; rows in `spec/inventory/13-postscript-hinting.toml`
//! and `14-truetype-hinting.toml`.
//!
//! | Module | Package | What it gives |
//! |---|---|---|
//! | [`noderef`] | M10.2 | [`NodeRef`], [`Edge`], [`TtPoint`] and their resolution on a layer |
//! | [`ps`] | M10.2 | [`PsHint`]: [`ps_hints`] / [`set_ps_hints`] over a layer's stored hints |
//! | [`tt`] | M10.2 | [`TtInstruction`]: [`tt_instructions`] / [`set_tt_instructions`] |
//! | [`settings`] | M10.2 | [`HintSettings`] of a master or instance, parameter names, defaults |
//! | [`validate`] | M10.2 | [`validate_font_hints`]: the zone and stem rules |
//! | [`remap`] | M10.3 | [`remap_after_edit`]: node references kept right after edits |
//! | [`replay`] | M10.6 | [`TouchedState::replay`]: which points are touched after n instructions; [`unmet_needs`] |
//! | [`point_map`] | M10.1 | the [`PointMap`] trait the compiler implements |
//! | [`tt_font`] | M10.17 | [`CvtLayout`], [`FontPrograms`], [`font_programs`] |
//! | [`tt_glyph`] | M10.18 | [`glyph_program`]: instructions plus point map to bytecode, with its `maxp` needs |
//! | [`tt_auto`] | M10.21 | [`autohint_tt`] and the TTFAutohint options |
//! | [`control`] | M10.21 | [`control::parse`]: TTFAutohint control instructions |
//! | [`imported`] | M10.22 | glyph programs of an opened font, kept while the outline is unchanged |
//!
//! How hints are stored: as the source wrote them, in [`fontcraft_model::LayerInfo::hints`], so
//! `.glyphs` files round-trip unchanged. This crate parses them into typed values and writes
//! edits back into the same keys; TT zones and stems stay in the TTFZones and TTFStems
//! parameters. There is no new model field.
//!
//! Who calls what: commands (`engine/src/cmd/hint_*.rs`) read and write through [`ps`], [`tt`]
//! and [`settings`]; the engine runs [`remap_after_edit`] after every font-changing command, in
//! the same undo step; the compiler (`compile/src/cff_hints.rs`, `tt_hints.rs`) reads
//! [`HintSettings`], resolves hints through [`PointMap`] and calls [`font_programs`],
//! [`glyph_program`] and [`autohint_tt`]; PostScript autohinting is `fontcraft-autohint`.
//!
//! Limits are named in [`error`]; every function returns [`HintingError`] instead of panicking.
#![forbid(unsafe_code)]

pub mod control;
pub mod error;
pub mod imported;
pub mod noderef;
pub mod point_map;
pub mod ps;
pub mod remap;
pub mod replay;
pub mod settings;
pub mod tt;
pub mod tt_auto;
pub mod tt_font;
pub mod tt_glyph;
#[cfg(test)]
mod tt_testfont;
pub mod validate;

pub use error::{
    HintingError, MAX_CONTROL_LINES, MAX_CONTROL_TEXT, MAX_CVT_ENTRIES, MAX_DELTA_COLUMNS, MAX_DELTA_PPM, MAX_INSTRUCTOR_PPM, MAX_LAYER_HINTS,
    MAX_PS_STEMS_WRITTEN, MAX_SOURCE_STEMS, MAX_SOURCE_ZONES, MAX_TT_STEMS, MAX_TT_ZONES, MIN_INSTRUCTOR_PPM,
};
pub use fontcraft_ttasm as ttasm;
pub use noderef::{Edge, NodeOrder, NodeRef, TtPoint};
pub use point_map::{PointMap, VecPointMap};
pub use ps::{PsHint, PsHintKind, ps_hints, set_ps_hints};
pub use remap::{RemapMode, RemapReport, Renumbered, follows_nodes, remap_after_edit, renumber_side};
pub use replay::{TouchedState, unmet_needs};
pub use settings::{DeltaDir, Deltas, HintSettings, PsZone, TtStem, TtZone};
pub use tt::{Direction, Rounding, StemRef, TtInstruction, TtKind, ZoneRef, set_tt_instructions, tt_instructions};
pub use tt_auto::{
    AutoGlyph, AutoHints, MeasuredZones, PointShift, TtAutohintOptions, autohint_glyph, autohint_prep, autohint_tt, drop_unmeasured_zones,
    shift_program, shifts_as_deltas,
};
pub use tt_font::{CvtLayout, FontPrograms, control_values, font_programs};
pub use tt_glyph::{GlyphContext, GlyphOutline, GlyphProgram, OutlinePoint, glyph_program};
pub use validate::{HintFinding, Severity, validate_font_hints};

#[cfg(test)]
mod tests {
    use fontcraft_model::Layer;

    use crate::{ps_hints, tt_instructions};

    #[test]
    fn a_layer_without_hints_has_no_typed_hints() {
        let layer = Layer::new("m01", 500.0);
        assert!(ps_hints(&layer).map(|h| h.is_empty()).unwrap_or(false));
        assert!(tt_instructions(&layer).map(|h| h.is_empty()).unwrap_or(false));
    }
}

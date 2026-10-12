//! Registry entries of the `check.color.*` checks (M12.56).

use super::LAYER_OUTLINES;
use crate::Precondition::Table;
use crate::Profile::{Strict, Universal};
use crate::Severity::{Fail, Warn};
use crate::checks::color;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Runner};

/// The fields every colour check shares.
const COLOR: CheckSpec = CheckSpec { group: CheckGroup::Color, scope: CheckScope::Binary, family: None, ..LAYER_OUTLINES };

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.color.paletteBrightness",
        label: "Colour layer too dark or too bright",
        description: "A COLR version 0 layer uses a fixed palette colour that is very dark or very bright, so it vanishes on a dark or a light background; such layers should use the text foreground colour instead.",
        severity: Warn,
        profile: Universal,
        thresholds: &[color::MIN_BRIGHTNESS],
        codes: &["glyphs-too-dark-or-too-bright"],
        preconditions: &[Table("COLR"), Table("CPAL")],
        run: Runner::Binary(color::palette_brightness),
        ..COLOR
    },
    CheckSpec {
        id: "check.color.emptyGlyphOneForColrV0",
        label: "Glyph 1 not empty in a colour font",
        description: "A COLR version 0 font has contours in glyph 1. A common system renderer paints glyph 1 over other glyphs in such fonts, so an empty glyph must hold that slot.",
        severity: Fail,
        profile: Universal,
        fix: Some(FixSpec {
            id: "addMandatoryGlyphs",
            command: Some("glyph.addGlyphs"),
            safe: true,
            description: "Insert an empty glyph right after .notdef.",
        }),
        codes: &["gid1-has-contours"],
        preconditions: &[Table("COLR")],
        run: Runner::Binary(color::empty_glyph_one_for_colr_v0),
        ..COLOR
    },
    CheckSpec {
        id: "check.color.colorTableSet",
        label: "Colour tables",
        description: "The colour tables do not suit broad delivery: a COLR version 0 font also carries an SVG table it does not need, a COLR version 1 font has no SVG fallback, an SVG table has no COLR companion, or a variable font carries an SVG table (which cannot vary).",
        severity: Fail,
        profile: Strict,
        codes: &["drop-svg", "add-svg", "variable-svg", "add-colr"],
        run: Runner::Binary(color::color_table_set),
        ..COLOR
    },
];

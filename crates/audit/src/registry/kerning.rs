//! Registry entries of the `check.kerning.*` checks (M12.56).

use super::LAYER_OUTLINES;
use crate::Precondition::{Table, Variable};
use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::kerning;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Runner};

/// The fields every kerning check shares.
const KERNING: CheckSpec = CheckSpec { group: CheckGroup::Kerning, scope: CheckScope::Source, family: None, ..LAYER_OUTLINES };

const CLEAN_UP: Option<FixSpec> = Some(FixSpec {
    id: "cleanKerning",
    command: Some("kerning.cleanUp"),
    safe: false,
    description: "Delete the pairs that name a missing glyph or an empty group.",
});

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.kerning.kernTable",
        label: "Legacy kern table",
        description: "A legacy kern table is present and unusable: it kerns glyphs that have no cmap entry (failure) or has no format 0 subtable (warning). A well-formed kern table is only noted.",
        scope: CheckScope::Binary,
        severity: Fail,
        profile: Opentype,
        fix: Some(FixSpec { id: "dropTables", command: Some("tables.delete"), safe: false, description: "Remove the kern table." }),
        codes: &["kern-non-character-glyphs", "kern-unknown-format", "kern-found", "kern-unreadable"],
        code_severities: &[("kern-unknown-format", Warn), ("kern-found", Info), ("kern-unreadable", Warn)],
        preconditions: &[Table("kern")],
        run: Runner::Binary(kerning::kern_table),
        ..KERNING
    },
    CheckSpec {
        id: "check.kerning.gposKerningMissing",
        label: "No pair kerning",
        description: "A proportional font has no pair-adjustment lookups in GPOS. Ignore for scripts and designs that need no kerning.",
        scope: CheckScope::Binary,
        severity: Warn,
        profile: Universal,
        codes: &["lacks-kern-info"],
        run: Runner::Binary(kerning::gpos_kerning_missing),
        ..KERNING
    },
    CheckSpec {
        id: "check.kerning.tabularKerning",
        label: "Kerned tabular figures",
        description: "Tabular figures, or other glyphs produced by tnum, take part in kerning, which breaks column alignment.",
        scope: CheckScope::Binary,
        severity: Fail,
        profile: Universal,
        fix: Some(FixSpec { id: "cleanKerning", command: None, safe: false, description: "Delete the pairs that kern tabular glyphs." }),
        codes: &["has-tabular-kerning"],
        preconditions: &[Table("GPOS")],
        run: Runner::Binary(kerning::tabular_kerning),
        ..KERNING
    },
    CheckSpec {
        id: "check.kerning.groupsDifferBetweenMasters",
        label: "Kerning groups differ between masters",
        description: "The kerning groups of a master differ from those of the default master.",
        severity: Warn,
        profile: Universal,
        codes: &["groups-differ"],
        preconditions: &[Variable],
        run: Runner::Source(kerning::groups_differ_between_masters),
        ..KERNING
    },
    CheckSpec {
        id: "check.kerning.missingGlyphReference",
        label: "Kerning names a missing glyph",
        description: "A kerning pair names a glyph that is not in the font, or a kerning group that no glyph belongs to on the side facing the pair.",
        severity: Fail,
        profile: Universal,
        fix: CLEAN_UP,
        codes: &["missing-glyph", "empty-group"],
        run: Runner::Source(kerning::missing_glyph_reference),
        ..KERNING
    },
    CheckSpec {
        id: "check.kerning.glyphInSeveralGroups",
        label: "Glyph in several kerning groups",
        description: "A glyph belongs to two kerning groups for the same side.",
        severity: Fail,
        profile: Universal,
        codes: &["several-groups"],
        run: Runner::Source(kerning::glyph_in_several_groups),
        ..KERNING
    },
    CheckSpec {
        id: "check.kerning.redundantPairs",
        label: "Redundant kerning pairs",
        description: "A kerning pair has the value zero without overriding anything, or an exception repeats the value its group pair already gives.",
        severity: Info,
        profile: Strict,
        fix: Some(FixSpec { id: "cleanKerning", command: None, safe: true, description: "Delete the redundant pairs." }),
        codes: &["zero-pair", "repeats-group-value"],
        run: Runner::Source(kerning::redundant_pairs),
        ..KERNING
    },
    CheckSpec {
        id: "check.kerning.excessivePair",
        label: "Excessive kerning",
        description: "A negative kerning value is larger than the advance width of one of the two glyphs, so the glyphs swap places or collide.",
        severity: Warn,
        profile: Strict,
        codes: &["exceeds-width"],
        run: Runner::Source(kerning::excessive_pair),
        ..KERNING
    },
];

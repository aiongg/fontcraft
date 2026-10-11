//! Registry entries of the `check.components.*` checks (M12.46).
//!
//! `check.components.misaligned` is not registered: it needs the alignment routine of the
//! `components` crate, which the audit does not depend on.

use crate::Profile::{Opentype, Outlines, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::components as c;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Precondition, Runner};

const fn fix(id: &'static str, command: &'static str, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id, command: Some(command), safe: true, description })
}

const DECOMPOSE: Option<FixSpec> = fix("decomposeComponents", "component.decompose", "Decompose the reported components into paths.");

/// The fields the component checks of one layer share.
const LAYER: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Components,
    scope: CheckScope::Layer,
    family: None,
    severity: Warn,
    profile: Universal,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[],
    run: Runner::Layer(c::orphaned),
};

/// The checks, in the order of the inventory.
pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.components.nested",
        label: "Nested components",
        description: "A component points at a glyph that is itself made of components.",
        severity: Fail,
        profile: Universal,
        fix: fix("flattenComponents", "component.flatten", "Replace the component by components of the innermost glyphs, transforms combined."),
        codes: &["nested"],
        run: Runner::Layer(c::nested),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.transformed",
        label: "Transformed components",
        description: "A component is mirrored (failure) or scaled, rotated or slanted (warning; it matters for hinted TrueType fonts).",
        severity: Fail,
        profile: Universal,
        fix: DECOMPOSE,
        codes: &["mirrored", "scaled-or-rotated"],
        code_severities: &[("scaled-or-rotated", Warn)],
        run: Runner::Layer(c::transformed),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.duplicate",
        label: "Duplicate components",
        description: "A glyph holds the same component twice at the same place, so the copies cover each other.",
        severity: Fail,
        profile: Opentype,
        fix: fix("removeDuplicateComponents", "edit.delete", "Delete the repeated component."),
        codes: &["duplicate"],
        run: Runner::Layer(c::duplicate),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.overlap",
        label: "Overlapping components",
        description: "Two components of a composite glyph overlap each other.",
        severity: Info,
        profile: Strict,
        fix: DECOMPOSE,
        codes: &["components-overlap"],
        run: Runner::Layer(c::overlap),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.mixedWithOutlines",
        label: "Paths mixed with components",
        description: "A glyph holds both paths and components, so the TrueType export decomposes it.",
        severity: Info,
        profile: Outlines,
        fix: DECOMPOSE,
        codes: &["mixed"],
        run: Runner::Layer(c::mixed_with_outlines),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.orphaned",
        label: "Components of missing glyphs",
        description: "A component names a glyph that is not in the font.",
        severity: Fail,
        profile: Outlines,
        codes: &["missing-base"],
        run: Runner::Layer(c::orphaned),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.circular",
        label: "Circular components",
        description: "A component chain leads back to the glyph it started from, or nests too deep to draw.",
        severity: Fail,
        profile: Outlines,
        codes: &["cycle", "too-deep"],
        run: Runner::Layer(c::circular),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.nonExportingBase",
        label: "Components of non-exporting glyphs",
        description: "A component points at a glyph that is excluded from export, so the export decomposes it.",
        severity: Warn,
        profile: Universal,
        fix: DECOMPOSE,
        codes: &["non-exporting-base"],
        run: Runner::Layer(c::non_exporting_base),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.emptyBase",
        label: "Components of empty glyphs",
        description: "A component points at a glyph with no outline on the same master, so it draws nothing.",
        severity: Warn,
        profile: Universal,
        codes: &["empty-base"],
        run: Runner::Layer(c::empty_base),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.caronAlternate",
        label: "Vertical caron",
        description: "Lcaron, dcaron, lcaron or tcaron is composed with the ordinary caron (failure) or a comma or quote (warning) instead of a vertical caron of its own.",
        scope: CheckScope::Binary,
        severity: Fail,
        profile: Universal,
        codes: &["wrong-mark", "bad-mark", "decomposed-outline", "single-component"],
        code_severities: &[("bad-mark", Warn), ("decomposed-outline", Warn), ("single-component", Warn)],
        preconditions: &[Precondition::TrueTypeOutlines],
        run: Runner::Binary(c::caron_alternate),
        ..LAYER
    },
    CheckSpec {
        id: "check.components.fewComposites",
        label: "Few composites",
        description: "More than half of a TrueType font's glyphs are stored as outlines rather than composites, which costs file size.",
        scope: CheckScope::Binary,
        severity: Warn,
        profile: Strict,
        thresholds: &[c::MAX_OUTLINE_SHARE],
        codes: &["few-composites"],
        preconditions: &[Precondition::TrueTypeOutlines],
        run: Runner::Binary(c::few_composites),
        ..LAYER
    },
];

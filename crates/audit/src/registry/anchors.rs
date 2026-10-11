//! Registry entries of the `check.anchors.*` checks (M12.46).
//!
//! `check.anchors.missing` and `check.anchors.markWithoutAttachment` are not registered: they
//! need the default-anchor table of the `components` crate, which the audit does not depend on.

use crate::Profile::{Outlines, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::anchors as a;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Runner};

/// The checks, in the order of the inventory.
pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.anchors.duplicateName",
        label: "Duplicate anchor names",
        description: "A layer has two anchors with the same name.",
        group: CheckGroup::Anchors,
        scope: CheckScope::Layer,
        family: None,
        severity: Fail,
        profile: Outlines,
        thresholds: &[],
        // Which of the two the export used is unknown, so keeping the first may move marks.
        fix: Some(FixSpec {
            id: "removeDuplicateAnchors",
            command: Some("anchor.remove"),
            safe: false,
            description: "Keep the first anchor of the name and delete the others.",
        }),
        skip_variable: false,
        capped: false,
        codes: &["duplicate-name"],
        code_severities: &[],
        preconditions: &[],
        run: Runner::Layer(a::duplicate_name),
    },
    CheckSpec {
        id: "check.anchors.unknownName",
        label: "Unpaired anchors",
        description: "An anchor name matches no mark anchor, caret, entry or exit, origin or smart-glyph property in the font, so it has no effect.",
        group: CheckGroup::Anchors,
        scope: CheckScope::Source,
        family: None,
        severity: Info,
        profile: Strict,
        thresholds: &[],
        fix: None,
        skip_variable: false,
        capped: true,
        codes: &["unknown-name"],
        code_severities: &[],
        preconditions: &[],
        run: Runner::Source(a::unknown_name),
    },
    CheckSpec {
        id: "check.anchors.ligatureCaretsMissing",
        label: "Missing ligature carets",
        description: "The font has ligatures in its liga feature but no, or incomplete, caret positions for them in GDEF.",
        group: CheckGroup::Anchors,
        scope: CheckScope::Binary,
        family: None,
        severity: Warn,
        profile: Universal,
        thresholds: &[],
        fix: Some(FixSpec {
            id: "addAnchors",
            command: Some("glyph.setAnchors"),
            safe: true,
            description: "Add evenly spaced caret anchors to the ligature in the source.",
        }),
        skip_variable: false,
        capped: false,
        codes: &["no-gdef", "no-carets", "incomplete-carets"],
        code_severities: &[],
        preconditions: &[],
        run: Runner::Binary(a::ligature_carets_missing),
    },
];

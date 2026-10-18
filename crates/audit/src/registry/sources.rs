//! Registry entries of the `check.sources.*` checks (M12.57). Home profiles and fixes follow
//! `docs/spec/21-qa.md` § 8 (Source files).

use crate::Profile::Universal;
use crate::Severity::{Fail, Warn};
use crate::checks::sources;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Precondition, Runner};

/// The fields every source-file check shares; each entry sets the rest and takes these with
/// `..SOURCES`.
const SOURCES: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Sources,
    scope: CheckScope::Source,
    family: None,
    severity: Fail,
    profile: Universal,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[],
    run: Runner::Source(sources::designspace_loads),
};

const VARIABLE: &[Precondition] = &[Precondition::Variable];

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.sources.designspaceLoads",
        label: "Designspace loads",
        description: "A designspace document cannot be parsed, lists no sources, or lists sources that cannot be opened.",
        codes: &["unreadable", "no-sources", "source-unreadable"],
        run: Runner::Source(sources::designspace_loads),
        ..SOURCES
    },
    CheckSpec {
        id: "check.sources.defaultMaster",
        label: "Default master",
        description: "No master sits at the default location of every axis, so there is no default master.",
        codes: &["not-found"],
        preconditions: VARIABLE,
        run: Runner::Source(sources::default_master),
        ..SOURCES
    },
    CheckSpec {
        id: "check.sources.consistentGlyphSet",
        label: "Consistent glyph set",
        description: "A master that is not the default master has glyphs the default master lacks.",
        codes: &["inconsistent-glyphset"],
        preconditions: VARIABLE,
        run: Runner::Source(sources::consistent_glyph_set),
        ..SOURCES
    },
    CheckSpec {
        id: "check.sources.consistentCodepoints",
        label: "Consistent code points",
        description: "A glyph has different code points in different masters.",
        fix: Some(FixSpec {
            id: "updateGlyphInfo",
            command: Some("glyph.updateGlyphInfo"),
            safe: false,
            description: "Give the glyph the default master's code points in every master.",
        }),
        codes: &["inconsistent-codepoints"],
        preconditions: VARIABLE,
        run: Runner::Source(sources::consistent_codepoints),
        ..SOURCES
    },
    CheckSpec {
        id: "check.sources.ufoStructure",
        label: "UFO structure",
        description: "A UFO has a missing or malformed property list, layer contents that do not match the files on disk, or an invalid glyph file.",
        codes: &["invalid-ufo"],
        run: Runner::Source(sources::ufo_structure),
        ..SOURCES
    },
    CheckSpec {
        id: "check.sources.requiredInfoFields",
        label: "Required font info",
        description: "A value the compiler cannot work without is unset: units per em, family name, or a master's ascender, descender, x-height or cap height.",
        codes: &["missing-required-fields"],
        run: Runner::Source(sources::required_info_fields),
        ..SOURCES
    },
    CheckSpec {
        id: "check.sources.recommendedInfoFields",
        label: "Recommended font info",
        description: "A value every release font should set is unset: the copyright, a master's style name or its PANOSE values.",
        severity: Warn,
        codes: &["missing-recommended-fields"],
        run: Runner::Source(sources::recommended_info_fields),
        ..SOURCES
    },
    CheckSpec {
        id: "check.sources.unnecessaryInfoFields",
        label: "Unnecessary font info",
        description: "The font info sets values the compiler generates or that are obsolete: the unique ID, the version string, the PostScript unique ID or the year.",
        severity: Warn,
        fix: Some(FixSpec {
            id: "clearInfoFields",
            command: Some("font.info.clearGeneratedFields"),
            safe: false,
            description: "Remove the values so the compiler generates them.",
        }),
        codes: &["unnecessary-fields"],
        run: Runner::Source(sources::unnecessary_info_fields),
        ..SOURCES
    },
];

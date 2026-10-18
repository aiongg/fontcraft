//! Registry entries of the `check.layout.*` checks (M12.52), in the order of the inventory. The
//! runners are in `checks/layout.rs`.

use crate::CheckGroup::Layout;
use crate::CheckScope::{Binary, Source};
use crate::Precondition::Table;
use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Warn};
use crate::checks::layout as l;
use crate::{CheckSpec, FixSpec, Runner};

const SET_GDEF_CLASSES: Option<FixSpec> = Some(FixSpec {
    id: "setGdefClasses",
    command: Some("glyph.setCategory"),
    safe: false,
    description: "Set the reported glyphs' categories so the GDEF glyph classes are rebuilt from them.",
});

/// The fields every entry here shares; each sets the rest and takes these with `..BASE`.
const BASE: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: Layout,
    scope: Binary,
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
    run: Runner::Binary(l::shaping_regression),
};

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.layout.validFeatureTags",
        label: "Unregistered feature tags",
        description: "GSUB or GPOS uses a feature tag that is neither registered nor private (one to four capital letters).",
        profile: Opentype,
        codes: &["bad-feature-tag"],
        run: Runner::Binary(l::valid_feature_tags),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.validScriptTags",
        label: "Unregistered script tags",
        description: "GSUB, GPOS or the BASE table uses a script tag that is not registered.",
        profile: Opentype,
        codes: &["bad-script-tag"],
        run: Runner::Binary(l::valid_script_tags),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.validLanguageTags",
        label: "Unregistered language tags",
        description: "GSUB or GPOS uses a language-system tag that is not registered. TUR written for Turkish's TRK is named as such.",
        profile: Opentype,
        fix: Some(FixSpec {
            id: "renameLanguageTag",
            command: None,
            safe: false,
            description: "Replace the mistyped language tag with the registered one in the feature code.",
        }),
        codes: &["bad-language-tag", "language-slip"],
        run: Runner::Binary(l::valid_language_tags),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.defaultLanguageSystems",
        label: "Default language systems",
        description: "GSUB or GPOS has features but no default language system for script DFLT, or, in a font with Latin letters, for script latn.",
        severity: Warn,
        profile: Strict,
        codes: &["missing-dflt-dflt", "missing-latn-dflt"],
        run: Runner::Binary(l::default_language_systems),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.defaultLanguagesystemStatement",
        label: "Default languagesystem statement",
        description: "The feature code declares language systems, but 'languagesystem DFLT dflt' is not the first, so the compiler does not add it.",
        scope: Source,
        severity: Warn,
        fix: Some(FixSpec {
            id: "addDefaultLanguagesystem",
            command: None,
            safe: false,
            description: "Insert 'languagesystem DFLT dflt;' before the other language systems.",
        }),
        codes: &["default-languagesystem"],
        run: Runner::Source(l::default_languagesystem_statement),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.gdefMarkClass",
        label: "GDEF mark class",
        description: "The GDEF glyph classes disagree with Unicode: a non-spacing mark outside the mark class, another character inside it, or a mark-class glyph with an advance width.",
        severity: Warn,
        profile: Opentype,
        fix: SET_GDEF_CLASSES,
        capped: true,
        codes: &["mark-chars", "non-mark-chars", "spacing-mark-glyphs"],
        preconditions: &[Table("GDEF")],
        run: Runner::Binary(l::gdef_mark_class),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.legacyAccents",
        label: "Legacy accents",
        description: "A spacing legacy accent (grave, diaeresis, macron, acute, cedilla, circumflex, caron, breve, dot, ring, ogonek, tilde, double acute) has no advance width or is classed as a mark in GDEF.",
        fix: SET_GDEF_CLASSES,
        codes: &["legacy-accents-width", "legacy-accents-gdef"],
        run: Runner::Binary(l::legacy_accents),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.smallCapsGlyphs",
        label: "Small cap coverage",
        description: "smcp or c2sc leaves letters of the scripts it covers without a small cap, or substitutes glyphs the font does not have.",
        codes: &["missing-small-caps", "missing-glyphs"],
        preconditions: &[Table("GSUB")],
        run: Runner::Binary(l::small_caps_glyphs),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.smallCapsBeforeLigatures",
        label: "Small caps before ligatures",
        description: "The first liga lookup does not come after the first smcp lookup, so ligatures form before small caps apply.",
        fix: Some(FixSpec {
            id: "reorderFeatures",
            command: None,
            safe: false,
            description: "Move the small-cap lookups ahead of the ligature lookups.",
        }),
        codes: &["feature-ordering"],
        preconditions: &[Table("GSUB")],
        run: Runner::Binary(l::small_caps_before_ligatures),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.stylisticSetNames",
        label: "Stylistic set names",
        description: "A stylistic set (ss01 to ss20) has no description, or names one that is not in the name table, so applications show only its number.",
        severity: Warn,
        codes: &["missing-description", "name-id-missing"],
        preconditions: &[Table("GSUB")],
        run: Runner::Binary(l::stylistic_set_names),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.gpos7",
        label: "Contextual positioning lookups",
        description: "GPOS has a contextual positioning lookup (type 7), which some shapers do not apply.",
        severity: Warn,
        fix: Some(FixSpec {
            id: "recompileLayout",
            command: None,
            safe: true,
            description: "Recompile the rules as chaining contextual lookups (type 8).",
        }),
        codes: &["has-gpos7"],
        preconditions: &[Table("GPOS")],
        run: Runner::Binary(l::gpos7),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.cjkSpacingFeatures",
        label: "CJK half-width spacing",
        description: "A CJK font has no chws or no vchw feature for contextual half-width spacing.",
        severity: Warn,
        codes: &["missing-chws-feature", "missing-vchw-feature"],
        run: Runner::Binary(l::cjk_spacing_features),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.dottedCircle",
        label: "Dotted circle",
        description: "The font has non-spacing marks but no dotted circle (a failure when a script needs a complex shaper), or marks that attach to other bases do not attach to it.",
        profile: Strict,
        codes: &["missing-dotted-circle", "missing-dotted-circle-complex", "unattached-dotted-circle-marks"],
        code_severities: &[("missing-dotted-circle", Warn)],
        run: Runner::Binary(l::dotted_circle),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.softDotted",
        label: "Soft-dotted letters",
        description: "A soft-dotted letter (i, j and others) keeps its dot under a mark above. Texts that orthographies use are listed apart from the rest.",
        severity: Warn,
        profile: Strict,
        codes: &["soft-dotted-orthography", "soft-dotted"],
        run: Runner::Binary(l::soft_dotted),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.sizeFeature",
        label: "Size feature",
        description: "The GPOS size feature has no parameters, its design size lies outside its own range, or its menu name ID is not in the name table.",
        profile: Strict,
        codes: &["size-params-missing", "size-outside-range", "size-name-missing"],
        preconditions: &[Table("GPOS")],
        run: Runner::Binary(l::size_feature),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.shapingRegression",
        label: "Shaping regression tests",
        description: "The project's shaping tests fail: different glyphs or positions, a forbidden glyph such as .notdef, or colliding glyph pairs.",
        profile: Strict,
        codes: &["shaping-differs", "forbidden-glyph", "collision"],
        run: Runner::Binary(l::shaping_regression),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.languageShaping",
        label: "Language shaping",
        description: "A shaping expectation of a language the font claims fails: a missing mark attachment, a missing localised form or an unreachable letter.",
        profile: Strict,
        codes: &["shaping-expectation"],
        run: Runner::Binary(l::language_shaping),
        ..BASE
    },
    CheckSpec {
        id: "check.layout.undefinedGlyphInFeatures",
        label: "Undefined glyphs in features",
        description: "The feature code names a glyph or class member that does not exist or does not export, so the features cannot compile.",
        scope: Source,
        codes: &["undefined-glyph"],
        run: Runner::Source(l::undefined_glyph_in_features),
        ..BASE
    },
];

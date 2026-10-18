//! Behaviour tests of the `check.layout.*` checks on fonts assembled byte by byte.

use std::sync::Arc;

use fontcraft_model::{Font, Snippet};

use crate::checks::kerning::test_font::{W, cmap, maxp, metrics, sfnt};
use crate::services::{AuditServices, ShapeRequest, ShapedGlyph, Unavailable};
use crate::{AuditContext, BinaryCtx, Config, Finding, LayerReport, NoServices, Severity, SourceCtx, audit_binary, audit_source, find};

fn run_with(bytes: &[u8], id: &str, services: &dyn AuditServices) -> LayerReport {
    let spec = find(id).unwrap_or_else(|| panic!("no check {id}"));
    let ctx = BinaryCtx::new("f0", bytes, 0, services).unwrap();
    let r = audit_binary(&ctx, &[spec], &Config::default());
    for f in &r.findings {
        assert!(spec.codes.contains(&f.code), "{id} emits unlisted `{}`", f.code);
        assert_eq!(f.location.font.as_deref(), Some("f0"));
    }
    r
}

fn run(bytes: &[u8], id: &str) -> LayerReport {
    run_with(bytes, id, &NoServices)
}

fn codes(findings: &[Finding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.code).collect()
}

/// One script of a layout table: its tag, whether it has a default language system, and its
/// language-system tags. Every language system lists every feature.
struct Script(&'static [u8; 4], bool, &'static [&'static [u8; 4]]);

/// One feature: tag, lookup indices and its feature parameters' bytes.
struct Feature(&'static [u8; 4], Vec<u16>, Option<Vec<u8>>);

/// One lookup: type and subtables' bytes.
struct Lookup(u16, Vec<Vec<u8>>);

fn lang_sys(features: usize) -> Vec<u8> {
    let mut w = W::default();
    w.u16(0).u16(0xFFFF).u16(features as u16);
    for i in 0..features {
        w.u16(i as u16);
    }
    w.0
}

fn script_list(scripts: &[Script], features: usize) -> Vec<u8> {
    let bodies: Vec<Vec<u8>> = scripts
        .iter()
        .map(|Script(_, default, langs)| {
            let ls = lang_sys(features);
            let header = 4 + 6 * langs.len();
            let mut w = W::default();
            w.u16(if *default { header as u16 } else { 0 }).u16(langs.len() as u16);
            let mut offset = header + if *default { ls.len() } else { 0 };
            for tag in *langs {
                w.tag(tag).u16(offset as u16);
                offset += ls.len();
            }
            if *default {
                w.bytes(&ls);
            }
            for _ in *langs {
                w.bytes(&ls);
            }
            w.0
        })
        .collect();
    let mut w = W::default();
    w.u16(scripts.len() as u16);
    let mut offset = 2 + 6 * scripts.len();
    for (Script(tag, ..), body) in scripts.iter().zip(&bodies) {
        w.tag(tag).u16(offset as u16);
        offset += body.len();
    }
    for body in bodies {
        w.bytes(&body);
    }
    w.0
}

fn feature_list(features: &[Feature]) -> Vec<u8> {
    let bodies: Vec<Vec<u8>> = features
        .iter()
        .map(|Feature(_, lookups, params)| {
            let header = 4 + 2 * lookups.len();
            let mut w = W::default();
            w.u16(if params.is_some() { header as u16 } else { 0 }).u16(lookups.len() as u16);
            for i in lookups {
                w.u16(*i);
            }
            if let Some(p) = params {
                w.bytes(p);
            }
            w.0
        })
        .collect();
    let mut w = W::default();
    w.u16(features.len() as u16);
    let mut offset = 2 + 6 * features.len();
    for (Feature(tag, ..), body) in features.iter().zip(&bodies) {
        w.tag(tag).u16(offset as u16);
        offset += body.len();
    }
    for body in bodies {
        w.bytes(&body);
    }
    w.0
}

fn lookup_list(lookups: &[Lookup]) -> Vec<u8> {
    let bodies: Vec<Vec<u8>> = lookups
        .iter()
        .map(|Lookup(kind, subtables)| {
            let mut w = W::default();
            w.u16(*kind).u16(0).u16(subtables.len() as u16);
            let mut offset = 6 + 2 * subtables.len();
            for s in subtables {
                w.u16(offset as u16);
                offset += s.len();
            }
            for s in subtables {
                w.bytes(s);
            }
            w.0
        })
        .collect();
    let mut w = W::default();
    w.u16(lookups.len() as u16);
    let mut offset = 2 + 2 * lookups.len();
    for body in &bodies {
        w.u16(offset as u16);
        offset += body.len();
    }
    for body in bodies {
        w.bytes(&body);
    }
    w.0
}

/// A GSUB or GPOS table.
fn layout(scripts: &[Script], features: &[Feature], lookups: &[Lookup]) -> Vec<u8> {
    let s = script_list(scripts, features.len());
    let f = feature_list(features);
    let l = lookup_list(lookups);
    let mut w = W::default();
    w.u16(1).u16(0).u16(10).u16((10 + s.len()) as u16).u16((10 + s.len() + f.len()) as u16);
    w.bytes(&s).bytes(&f).bytes(&l);
    w.0
}

fn coverage(glyphs: &[u16]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(1).u16(glyphs.len() as u16);
    for g in glyphs {
        w.u16(*g);
    }
    w.0
}

/// A single substitution (format 2) of each `(from, to)`, sorted by `from`.
fn single_subst(pairs: &[(u16, u16)]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(2).u16((6 + 2 * pairs.len()) as u16).u16(pairs.len() as u16);
    for (_, to) in pairs {
        w.u16(*to);
    }
    let from: Vec<u16> = pairs.iter().map(|p| p.0).collect();
    w.bytes(&coverage(&from));
    w.0
}

/// A mark-to-base subtable: only its two coverage tables are read by the checks.
fn mark_to_base(marks: &[u16], bases: &[u16]) -> Vec<u8> {
    let m = coverage(marks);
    let mut w = W::default();
    w.u16(1).u16(12).u16((12 + m.len()) as u16).u16(1).u16(0).u16(0);
    w.bytes(&m).bytes(&coverage(bases));
    w.0
}

/// GDEF version 1.0 with a format 1 glyph class table starting at glyph 0.
fn gdef(classes: &[u16]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(1).u16(0).u16(12).u16(0).u16(0).u16(0);
    w.u16(1).u16(0).u16(classes.len() as u16);
    for c in classes {
        w.u16(*c);
    }
    w.0
}

/// A `name` table holding empty Windows records with these name IDs.
fn name(ids: &[u16]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(0).u16(ids.len() as u16).u16((6 + 12 * ids.len()) as u16);
    for id in ids {
        w.u16(3).u16(1).u16(0x409).u16(*id).u16(0).u16(0);
    }
    w.0
}

/// A font with these advance widths, character map and extra tables.
fn font(widths: &[u16], map: &[(u32, u16)], extra: Vec<(&'static [u8; 4], Vec<u8>)>) -> Vec<u8> {
    let mut map = map.to_vec();
    map.sort_unstable();
    let (hhea, hmtx) = metrics(widths);
    let mut tables = vec![(b"maxp", maxp(widths.len() as u16)), (b"cmap", cmap(&map)), (b"hhea", hhea), (b"hmtx", hmtx)];
    tables.extend(extra);
    sfnt(tables)
}

const DFLT: Script = Script(b"DFLT", true, &[]);

#[test]
fn tag_checks_report_unregistered_feature_script_and_language_tags() {
    let gsub = layout(
        &[DFLT, Script(b"latn", true, &[b"TUR ", b"QQQ ", b"TRK "]), Script(b"zzzz", true, &[])],
        &[Feature(b"liga", vec![], None), Feature(b"ab1c", vec![], None), Feature(b"PRIV", vec![], None), Feature(b"ss01", vec![], None)],
        &[],
    );
    let bytes = font(&[0, 500], &[(0x41, 1)], vec![(b"GSUB", gsub)]);

    let found = run(&bytes, "check.layout.validFeatureTags").findings;
    assert_eq!(codes(&found), ["bad-feature-tag"], "registered ss01 and the private PRIV pass");
    assert_eq!(found[0].location.feature.as_deref(), Some("ab1c"));
    assert_eq!(found[0].location.table.as_deref(), Some("GSUB"));

    let found = run(&bytes, "check.layout.validScriptTags").findings;
    assert_eq!(codes(&found), ["bad-script-tag"]);
    assert_eq!(found[0].location.script.as_deref(), Some("zzzz"));

    let found = run(&bytes, "check.layout.validLanguageTags").findings;
    let mut got: Vec<(&str, Option<&str>)> = found.iter().map(|f| (f.code, f.location.language.as_deref())).collect();
    got.sort();
    assert_eq!(got, [("bad-language-tag", Some("QQQ ")), ("language-slip", Some("TUR "))]);
    let slip = found.iter().find(|f| f.code == "language-slip").unwrap();
    assert!(slip.message.contains("'TRK '"), "{}", slip.message);
    assert!(slip.fix.is_some(), "the slip carries the rename fix");
    assert!(found.iter().find(|f| f.code == "bad-language-tag").unwrap().fix.is_none());
}

#[test]
fn default_language_systems_need_dflt_and_latin_defaults() {
    let feature = || vec![Feature(b"liga", vec![], None)];
    let latin_only = layout(&[Script(b"latn", true, &[])], &feature(), &[]);
    let bytes = font(&[0, 500], &[(0x41, 1)], vec![(b"GSUB", latin_only)]);
    assert_eq!(codes(&run(&bytes, "check.layout.defaultLanguageSystems").findings), ["missing-dflt-dflt"]);

    let dflt_only = layout(&[DFLT], &feature(), &[]);
    let bytes = font(&[0, 500], &[(0x41, 1)], vec![(b"GPOS", dflt_only.clone())]);
    let found = run(&bytes, "check.layout.defaultLanguageSystems").findings;
    assert_eq!(codes(&found), ["missing-latn-dflt"]);
    assert_eq!(found[0].location.table.as_deref(), Some("GPOS"));
    let greek = font(&[0, 500], &[(0x3B1, 1)], vec![(b"GPOS", dflt_only)]);
    assert!(run(&greek, "check.layout.defaultLanguageSystems").findings.is_empty(), "no Latin letters, no latn/dflt needed");

    // A script with only named language systems has no default one.
    let both = layout(&[DFLT, Script(b"latn", false, &[b"TRK "])], &feature(), &[]);
    let bytes = font(&[0, 500], &[(0x41, 1)], vec![(b"GSUB", both)]);
    assert_eq!(codes(&run(&bytes, "check.layout.defaultLanguageSystems").findings), ["missing-latn-dflt"]);

    let good = layout(&[DFLT, Script(b"latn", true, &[b"TRK "])], &feature(), &[]);
    let bytes = font(&[0, 500], &[(0x41, 1)], vec![(b"GSUB", good)]);
    assert!(run(&bytes, "check.layout.defaultLanguageSystems").findings.is_empty());
}

fn source_findings(font: &Font, id: &str) -> LayerReport {
    let spec = find(id).unwrap();
    let audit = AuditContext::new(font);
    let ctx = SourceCtx { audit: &audit, services: &NoServices };
    let r = audit_source(&ctx, &[spec], &Config::default());
    for f in &r.findings {
        assert!(spec.codes.contains(&f.code), "{id} emits unlisted `{}`", f.code);
    }
    r
}

fn with_prefix(code: &str) -> Font {
    let mut font = Font::new("Test");
    Arc::make_mut(&mut font.features).prefixes.push(Snippet { name: "Languagesystems".into(), code: code.into(), ..Snippet::default() });
    font
}

#[test]
fn languagesystem_dflt_dflt_must_come_first() {
    let id = "check.layout.defaultLanguagesystemStatement";
    assert!(source_findings(&Font::new("Test"), id).findings.is_empty(), "no statements: the compiler adds the default");

    let found = source_findings(&with_prefix("languagesystem latn dflt;\nlanguagesystem DFLT dflt;\n"), id).findings;
    assert_eq!(codes(&found), ["default-languagesystem"]);
    assert!(found[0].message.contains("latn dflt, DFLT dflt"), "{}", found[0].message);
    assert_eq!(found[0].location.feature.as_deref(), Some("Languagesystems"));

    assert!(source_findings(&with_prefix("languagesystem DFLT dflt;\nlanguagesystem latn dflt;"), id).findings.is_empty());
    assert!(
        source_findings(&with_prefix("# languagesystem latn dflt;\nlanguagesystem DFLT dflt; languagesystem latn TRK;"), id).findings.is_empty(),
        "comments are not statements"
    );

    // A switched-off snippet does not count.
    let mut font = with_prefix("languagesystem DFLT dflt;");
    Arc::make_mut(&mut font.features)
        .prefixes
        .insert(0, Snippet { name: "Old".into(), code: "languagesystem latn dflt;".into(), disabled: true, ..Snippet::default() });
    assert!(source_findings(&font, id).findings.is_empty());
}

#[test]
fn gdef_mark_class_follows_unicode() {
    // 1: a (in the mark class, spacing); 2: acute comb (not a mark); 3: grave comb (a mark, no
    // width); 4: an unencoded spacing glyph in the mark class.
    let bytes = font(&[0, 500, 0, 0, 300], &[(0x61, 1), (0x301, 2), (0x300, 3)], vec![(b"GDEF", gdef(&[0, 3, 0, 3, 3]))]);
    let found = run(&bytes, "check.layout.gdefMarkClass").findings;
    let mut got: Vec<(&str, String)> = found.iter().map(|f| (f.code, f.location.glyph.clone())).collect();
    got.sort();
    assert_eq!(
        got,
        [
            ("mark-chars", "gid2".to_string()),
            ("non-mark-chars", "gid1".to_string()),
            ("spacing-mark-glyphs", "gid1".to_string()),
            ("spacing-mark-glyphs", "gid4".to_string()),
        ]
    );
    assert!(found.iter().all(|f| f.severity == Severity::Warn && f.fix.is_some()));

    let no_classes = font(
        &[0, 500],
        &[(0x61, 1)],
        vec![(b"GDEF", {
            let mut w = W::default();
            w.u16(1).u16(0).u16(0).u16(0).u16(0).u16(0);
            w.0
        })],
    );
    assert_eq!(run(&no_classes, "check.layout.gdefMarkClass").skipped.len(), 1);
}

#[test]
fn legacy_accents_need_width_and_base_class() {
    // 1: acute (no width, a mark); 2: grave (spacing base).
    let bytes = font(&[0, 0, 300], &[(0xB4, 1), (0x60, 2)], vec![(b"GDEF", gdef(&[0, 3, 1]))]);
    let found = run(&bytes, "check.layout.legacyAccents").findings;
    assert_eq!(codes(&found), ["legacy-accents-width", "legacy-accents-gdef"]);
    assert!(found.iter().all(|f| f.location.glyph == "gid1" && f.severity == Severity::Fail));
    assert!(found[0].fix.is_none(), "a width has to be designed");
    assert!(found[1].fix.is_some());

    let clean = font(&[0, 300, 300], &[(0xB4, 1), (0x60, 2)], vec![(b"GDEF", gdef(&[0, 1, 1]))]);
    assert!(run(&clean, "check.layout.legacyAccents").findings.is_empty());
}

#[test]
fn small_caps_cover_every_cased_letter_of_their_scripts() {
    // 1 a, 2 b, 3 A, 4 B, 5 a.sc, 6 alpha (Greek, not covered by any small cap).
    let map = [(0x61, 1), (0x62, 2), (0x41, 3), (0x42, 4), (0x3B1, 6)];
    let gsub = layout(
        &[DFLT],
        &[Feature(b"smcp", vec![0], None), Feature(b"c2sc", vec![1], None)],
        &[Lookup(1, vec![single_subst(&[(1, 5)])]), Lookup(1, vec![single_subst(&[(3, 5), (4, 9)])])],
    );
    let bytes = font(&[0, 500, 500, 600, 600, 450, 500], &map, vec![(b"GSUB", gsub)]);
    let found = run(&bytes, "check.layout.smallCapsGlyphs").findings;
    let mut got: Vec<(&str, Option<&str>)> = found.iter().map(|f| (f.code, f.location.feature.as_deref())).collect();
    got.sort();
    assert_eq!(got, [("missing-glyphs", Some("c2sc")), ("missing-small-caps", Some("smcp"))]);
    let smcp = found.iter().find(|f| f.code == "missing-small-caps").unwrap();
    assert!(smcp.message.contains("U+0062") && !smcp.message.contains("U+03B1"), "only Latin is expected: {}", smcp.message);
    assert_eq!(smcp.data.get("letters"), Some(&1.0));
}

#[test]
fn small_caps_lookups_come_before_ligatures() {
    let id = "check.layout.smallCapsBeforeLigatures";
    let lookups = || vec![Lookup(1, vec![single_subst(&[(1, 2)])]), Lookup(1, vec![single_subst(&[(1, 2)])])];
    let late = layout(&[DFLT], &[Feature(b"liga", vec![0], None), Feature(b"smcp", vec![1], None)], &lookups());
    let found = run(&font(&[0, 500, 500], &[(0x61, 1)], vec![(b"GSUB", late)]), id).findings;
    assert_eq!(codes(&found), ["feature-ordering"]);
    assert_eq!(found[0].location.lookup, Some(0));

    let early = layout(&[DFLT], &[Feature(b"liga", vec![1], None), Feature(b"smcp", vec![0], None)], &lookups());
    assert!(run(&font(&[0, 500, 500], &[(0x61, 1)], vec![(b"GSUB", early)]), id).findings.is_empty());

    let no_liga = layout(&[DFLT], &[Feature(b"smcp", vec![0], None)], &lookups());
    let r = run(&font(&[0, 500, 500], &[(0x61, 1)], vec![(b"GSUB", no_liga)]), id);
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1);
}

fn ss_params(name_id: u16) -> Vec<u8> {
    let mut w = W::default();
    w.u16(0).u16(name_id);
    w.0
}

#[test]
fn stylistic_sets_need_names_in_the_name_table() {
    let gsub = layout(
        &[DFLT],
        &[
            Feature(b"ss01", vec![], None),
            Feature(b"ss02", vec![], Some(ss_params(256))),
            Feature(b"ss03", vec![], Some(ss_params(300))),
            Feature(b"ss21", vec![], None),
        ],
        &[],
    );
    let bytes = font(&[0, 500], &[(0x61, 1)], vec![(b"GSUB", gsub), (b"name", name(&[1, 2, 256]))]);
    let found = run(&bytes, "check.layout.stylisticSetNames").findings;
    let got: Vec<(&str, Option<&str>)> = found.iter().map(|f| (f.code, f.location.feature.as_deref())).collect();
    assert_eq!(got, [("missing-description", Some("ss01")), ("name-id-missing", Some("ss03"))], "ss21 is not a stylistic set");
}

#[test]
fn gpos7_reports_contextual_positioning_lookups() {
    let context = || {
        let mut w = W::default();
        w.u16(1).u16(6).u16(0).bytes(&coverage(&[1]));
        w.0
    };
    let gpos = layout(&[DFLT], &[Feature(b"kern", vec![1], None)], &[Lookup(8, vec![]), Lookup(7, vec![context()])]);
    let found = run(&font(&[0, 500], &[(0x61, 1)], vec![(b"GPOS", gpos)]), "check.layout.gpos7").findings;
    assert_eq!(codes(&found), ["has-gpos7"]);
    assert_eq!(found[0].location.lookup, Some(1));

    let chained = layout(&[DFLT], &[], &[Lookup(8, vec![])]);
    assert!(run(&font(&[0, 500], &[(0x61, 1)], vec![(b"GPOS", chained)]), "check.layout.gpos7").findings.is_empty());
}

#[test]
fn cjk_fonts_need_half_width_spacing_features() {
    let id = "check.layout.cjkSpacingFeatures";
    let cjk = |n: u16| -> Vec<(u32, u16)> { (0..n).map(|i| (0x4E00 + u32::from(i), i + 1)).collect() };
    let widths = vec![1000u16; 152];
    let gpos = layout(&[DFLT], &[Feature(b"chws", vec![], None)], &[]);
    let found = run(&font(&widths, &cjk(151), vec![(b"GPOS", gpos)]), id).findings;
    assert_eq!(codes(&found), ["missing-vchw-feature"]);

    let r = run(&font(&widths, &cjk(150), vec![]), id);
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1, "150 CJK characters do not make a CJK font");
}

#[test]
fn dotted_circle_carries_every_attaching_mark() {
    let id = "check.layout.dottedCircle";
    // 1 a (base), 2 acute comb (mark), 3 dotted circle.
    let classes = gdef(&[0, 1, 3, 1]);
    let without = font(&[0, 500, 0, 600], &[(0x61, 1), (0x301, 2)], vec![(b"GDEF", classes.clone())]);
    let found = run(&without, id).findings;
    assert_eq!(codes(&found), ["missing-dotted-circle"]);
    assert_eq!(found[0].severity, Severity::Warn);

    let deva = layout(&[Script(b"dev2", true, &[])], &[], &[]);
    let complex = font(&[0, 500, 0, 600], &[(0x61, 1), (0x301, 2)], vec![(b"GDEF", classes.clone()), (b"GSUB", deva)]);
    let found = run(&complex, id).findings;
    assert_eq!(codes(&found), ["missing-dotted-circle-complex"]);
    assert_eq!(found[0].severity, Severity::Fail);

    let map = [(0x61, 1), (0x301, 2), (0x25CC, 3)];
    let only_a = layout(&[DFLT], &[Feature(b"mark", vec![0], None)], &[Lookup(4, vec![mark_to_base(&[2], &[1])])]);
    let found = run(&font(&[0, 500, 0, 600], &map, vec![(b"GDEF", classes.clone()), (b"GPOS", only_a)]), id).findings;
    assert_eq!(codes(&found), ["unattached-dotted-circle-marks"]);
    assert_eq!(found[0].location.glyph, "gid2");

    let both = layout(&[DFLT], &[Feature(b"mark", vec![0], None)], &[Lookup(4, vec![mark_to_base(&[2], &[1, 3])])]);
    assert!(run(&font(&[0, 500, 0, 600], &map, vec![(b"GDEF", classes), (b"GPOS", both)]), id).findings.is_empty());

    let r = run(&font(&[0, 500], &[(0x61, 1)], vec![]), id);
    assert_eq!(r.skipped.len(), 1, "no marks: nothing to check");
}

fn size_params(design: u16, name_id: u16, start: u16, end: u16) -> Vec<u8> {
    let mut w = W::default();
    w.u16(design).u16(1).u16(name_id).u16(start).u16(end);
    w.0
}

#[test]
fn size_feature_is_consistent() {
    let id = "check.layout.sizeFeature";
    let with = |params: Vec<u8>, names: &[u16]| {
        let gpos = layout(&[DFLT], &[Feature(b"size", vec![], Some(params))], &[]);
        font(&[0, 500], &[(0x61, 1)], vec![(b"GPOS", gpos), (b"name", name(names))])
    };
    let found = run(&with(size_params(100, 256, 80, 90), &[1, 2]), id).findings;
    assert_eq!(codes(&found), ["size-outside-range", "size-name-missing"]);
    assert!(found[0].message.contains("10 pt") && found[0].message.contains("8 to 9 pt"), "{}", found[0].message);
    assert_eq!(found[1].location.name_id, Some(256));

    assert!(run(&with(size_params(100, 256, 80, 120), &[1, 256]), id).findings.is_empty());
    assert!(run(&with(size_params(100, 0, 0, 0), &[1]), id).findings.is_empty(), "no menu name: no range to check");

    let bare = layout(&[DFLT], &[Feature(b"size", vec![], None)], &[]);
    let found = run(&font(&[0, 500], &[(0x61, 1)], vec![(b"GPOS", bare)]), id).findings;
    assert_eq!(codes(&found), ["size-params-missing"]);
}

/// Shapes by the character map, replacing i by dotless i before the marks `drops` lists.
struct Shaper {
    map: Vec<(char, u32)>,
    drops: Vec<char>,
}

impl AuditServices for Shaper {
    fn shape(&self, _: &[u8], request: &ShapeRequest) -> Result<Vec<ShapedGlyph>, Unavailable> {
        let chars: Vec<char> = request.text.chars().collect();
        let drop = chars.iter().any(|c| self.drops.contains(c));
        Ok(chars
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let gid = self.map.iter().find(|(m, _)| m == c).map_or(0, |(_, g)| *g);
                ShapedGlyph { glyph_id: if i == 0 && drop && *c == 'i' { 9 } else { gid }, cluster: 0, ..ShapedGlyph::default() }
            })
            .collect())
    }
}

#[test]
fn soft_dotted_letters_lose_their_dot_under_marks_above() {
    let id = "check.layout.softDotted";
    // 1 i, 2 acute comb, 3 double acute comb, 4 cedilla comb (a mark below).
    let map = [(0x69, 1), (0x301, 2), (0x30B, 3), (0x327, 4)];
    let bytes = font(&[0, 300, 0, 0, 0], &map, vec![]);
    let r = run(&bytes, id);
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1, "no shaper: the check skips");

    let shaper = Shaper { map: vec![('i', 1), ('\u{301}', 2), ('\u{30B}', 3), ('\u{327}', 4)], drops: vec![] };
    let found = run_with(&bytes, id, &shaper).findings;
    assert_eq!(codes(&found), ["soft-dotted-orthography", "soft-dotted"]);
    assert!(found[0].message.contains("i\u{30B}"), "i with double acute is used in orthographies: {}", found[0].message);
    assert!(found.iter().all(|f| f.severity == Severity::Warn));

    let shaper = Shaper { map: vec![('i', 1), ('\u{301}', 2), ('\u{30B}', 3), ('\u{327}', 4)], drops: vec!['\u{301}', '\u{30B}'] };
    assert!(run_with(&bytes, id, &shaper).findings.is_empty());
}

#[test]
fn checks_without_their_data_skip_with_a_reason() {
    let bytes = font(&[0, 500], &[(0x61, 1)], vec![]);
    for id in ["check.layout.shapingRegression", "check.layout.languageShaping"] {
        let r = run(&bytes, id);
        assert!(r.findings.is_empty());
        assert_eq!(r.skipped.len(), 1, "{id}");
    }
    let r = source_findings(&Font::new("Test"), "check.layout.undefinedGlyphInFeatures");
    assert_eq!(r.skipped.len(), 1);
}

//! Each width check on a small TrueType font built here, byte by byte, and the source checks on
//! fonts built in code.

use fontcraft_model::{Font, Layer};

use super::{ACCENT_NAME_PARTS, accent_base};
use crate::checks::components::tests::{W, cmap, maxp, post, sfnt};
use crate::{
    AuditContext, BinaryCtx, CHECKS, CheckScope, Config, Finding, GlyphCtx, LayerReport, NoServices, Severity, audit_binary, audit_glyph, find,
};

/// One glyph: name, code point, advance and, for a drawn glyph, the x extent of its one
/// rectangle (0 to 500 high).
#[derive(Clone)]
struct G {
    name: &'static str,
    cp: Option<u32>,
    adv: u16,
    x: Option<(i16, i16)>,
}

fn g(name: &'static str, cp: u32, adv: u16) -> G {
    let inset = i16::try_from(adv / 10).unwrap();
    G { name, cp: Some(cp), adv, x: (adv > 0).then(|| (inset, i16::try_from(adv).unwrap() - inset)) }
}

/// The font the tests change.
#[derive(Clone)]
struct Spec {
    glyphs: Vec<G>,
    /// `liga` substitutions: components → ligature.
    liga: Vec<(Vec<&'static str>, &'static str)>,
    /// `tnum` single substitutions: from → to; `None` for no `tnum` feature.
    tnum: Option<Vec<(&'static str, &'static str)>>,
    /// Glyphs `GDEF` classes as marks.
    marks: Vec<&'static str>,
    /// Leave out `hmtx` (a damaged font).
    no_hmtx: bool,
}

/// A clean proportional font, units per em 1000: spaces 250, figures and math signs 600, comma
/// and period 250, the f_i ligature spaced like f and i, Aacute as wide as A, en and em dashes.
fn base() -> Spec {
    let mut glyphs = vec![G { name: ".notdef", cp: None, adv: 500, x: Some((50, 450)) }, g("space", 0x20, 250), g("nbspace", 0xA0, 250)];
    let figures = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
    glyphs.extend(figures.iter().zip(0x30u32..).map(|(n, cp)| g(n, cp, 600)));
    glyphs.extend([
        g("comma", 0x2C, 250),
        g("period", 0x2E, 250),
        g("plus", 0x2B, 600),
        g("equal", 0x3D, 600),
        g("minus", 0x2212, 600),
        g("A", 0x41, 600),
        g("Aacute", 0xC1, 600),
        G { name: "f", cp: Some(0x66), adv: 300, x: Some((30, 270)) },
        G { name: "i", cp: Some(0x69), adv: 250, x: Some((50, 200)) },
        G { name: "f_i", cp: None, adv: 500, x: Some((30, 450)) },
        G { name: "acutecomb", cp: Some(0x301), adv: 0, x: Some((-150, -50)) },
        g("circumflex", 0x2C6, 300),
        g("endash", 0x2013, 500),
        g("emdash", 0x2014, 1000),
        g("one.tf", 0xE000, 600),
    ]);
    Spec { glyphs, liga: vec![(vec!["f", "i"], "f_i")], tnum: Some(vec![("one", "one.tf")]), marks: vec!["acutecomb"], no_hmtx: false }
}

impl Spec {
    fn gid(&self, name: &str) -> u16 {
        u16::try_from(self.glyphs.iter().position(|g| g.name == name).unwrap_or_else(|| panic!("no glyph {name}"))).unwrap()
    }

    fn glyph(&mut self, name: &str) -> &mut G {
        self.glyphs.iter_mut().find(|g| g.name == name).unwrap_or_else(|| panic!("no glyph {name}"))
    }

    fn width(mut self, name: &str, adv: u16) -> Self {
        self.glyph(name).adv = adv;
        self
    }

    fn add(mut self, glyph: G) -> Self {
        self.glyphs.push(glyph);
        self
    }
}

/// One subtable lookup of the test `GSUB`.
enum Lookup {
    /// Single substitution format 2: (from, to).
    Single(Vec<(u16, u16)>),
    /// Ligature substitution format 1: (components, ligature), one per first glyph.
    Ligature(Vec<(Vec<u16>, u16)>),
}

fn coverage(w: &mut W, glyphs: &[u16]) {
    w.u16(1).u16(glyphs.len() as u16);
    for g in glyphs {
        w.u16(*g);
    }
}

fn subtable(lookup: &Lookup) -> (u16, Vec<u8>) {
    let mut w = W::default();
    match lookup {
        Lookup::Single(pairs) => {
            let mut pairs = pairs.clone();
            pairs.sort_unstable();
            let n = pairs.len() as u16;
            w.u16(2).u16(6 + 2 * n).u16(n);
            for (_, to) in &pairs {
                w.u16(*to);
            }
            coverage(&mut w, &pairs.iter().map(|p| p.0).collect::<Vec<_>>());
            (1, w.0)
        }
        Lookup::Ligature(ligs) => {
            let mut ligs: Vec<&(Vec<u16>, u16)> = ligs.iter().collect();
            ligs.sort_by_key(|l| l.0[0]);
            let n = ligs.len() as u16;
            let cov = 6 + 2 * n;
            let mut set = cov + 4 + 2 * n;
            w.u16(1).u16(cov).u16(n);
            for (components, _) in &ligs {
                w.u16(set);
                set += 8 + 2 * (components.len() as u16 - 1);
            }
            coverage(&mut w, &ligs.iter().map(|l| l.0[0]).collect::<Vec<_>>());
            for (components, lig) in &ligs {
                w.u16(1).u16(4).u16(*lig).u16(components.len() as u16);
                for c in &components[1..] {
                    w.u16(*c);
                }
            }
            (4, w.0)
        }
    }
}

/// `GSUB` without scripts: each feature lists its lookups by index.
fn gsub(features: &[(&[u8; 4], Vec<u16>)], lookups: &[Lookup]) -> ([u8; 4], Vec<u8>) {
    let mut feature_list = W::default();
    feature_list.u16(features.len() as u16);
    let mut offset = 2 + 6 * features.len() as u16;
    for (tag, indices) in features {
        feature_list.tag(tag).u16(offset);
        offset += 4 + 2 * indices.len() as u16;
    }
    for (_, indices) in features {
        feature_list.u16(0).u16(indices.len() as u16);
        for i in indices {
            feature_list.u16(*i);
        }
    }
    let mut lookup_list = W::default();
    lookup_list.u16(lookups.len() as u16);
    let tables: Vec<(u16, Vec<u8>)> = lookups.iter().map(subtable).collect();
    let mut offset = 2 + 2 * lookups.len() as u16;
    for (_, data) in &tables {
        lookup_list.u16(offset);
        offset += 8 + data.len() as u16;
    }
    for (kind, data) in &tables {
        lookup_list.u16(*kind).u16(0).u16(1).u16(8);
        lookup_list.0.extend(data);
    }
    let mut w = W::default();
    let features_at = 12u16;
    let lookups_at = features_at + feature_list.0.len() as u16;
    w.u16(1).u16(0).u16(10).u16(features_at).u16(lookups_at).u16(0);
    w.0.extend(feature_list.0);
    w.0.extend(lookup_list.0);
    (*b"GSUB", w.0)
}

/// `GDEF` whose glyph class definition makes `marks` marks.
fn gdef(marks: &[u16]) -> ([u8; 4], Vec<u8>) {
    let mut marks = marks.to_vec();
    marks.sort_unstable();
    let mut w = W::default();
    w.u16(1).u16(0).u16(12).u16(0).u16(0).u16(0);
    w.u16(2).u16(marks.len() as u16);
    for m in marks {
        w.u16(m).u16(m).u16(3);
    }
    (*b"GDEF", w.0)
}

fn build(s: &Spec) -> Vec<u8> {
    let n = s.glyphs.len() as u16;
    let mut head = W::default();
    head.u32(0x0001_0000).u32(0).u32(0).u32(0x5F0F_3CF5).u16(0).u16(1000);
    head.0.resize(50, 0);
    head.i16(1).i16(0);
    let mut hhea = W::default();
    hhea.u32(0x0001_0000).i16(800).i16(-200).i16(0).u16(s.glyphs.iter().map(|g| g.adv).max().unwrap());
    for _ in 0..11 {
        hhea.i16(0);
    }
    hhea.u16(n);
    let (mut hmtx, mut glyf, mut loca) = (W::default(), W::default(), W::default());
    for glyph in &s.glyphs {
        loca.u32(glyf.0.len() as u32);
        let Some((x0, x1)) = glyph.x else {
            hmtx.u16(glyph.adv).i16(0);
            continue;
        };
        hmtx.u16(glyph.adv).i16(x0);
        // One rectangle, all points on the curve, coordinates as 16-bit deltas.
        glyf.i16(1).i16(x0).i16(0).i16(x1).i16(500).u16(3).u16(0);
        glyf.0.extend([1u8; 4]);
        glyf.i16(x0).i16(0).i16(x1 - x0).i16(0);
        glyf.i16(0).i16(500).i16(0).i16(-500);
        while glyf.0.len() % 4 != 0 {
            glyf.0.push(0);
        }
    }
    loca.u32(glyf.0.len() as u32);
    let names: Vec<&str> = s.glyphs.iter().map(|g| g.name).collect();
    let map = s.glyphs.iter().enumerate().filter_map(|(i, g)| g.cp.map(|c| (c, i as u32))).collect();
    let mut tables = vec![(*b"head", head.0), (*b"hhea", hhea.0), maxp(n), (*b"loca", loca.0), (*b"glyf", glyf.0), cmap(map), post(&names)];
    if !s.no_hmtx {
        tables.push((*b"hmtx", hmtx.0));
    }
    let mut features: Vec<(&[u8; 4], Vec<u16>)> = Vec::new();
    let mut lookups = Vec::new();
    if !s.liga.is_empty() {
        features.push((b"liga", vec![lookups.len() as u16]));
        lookups.push(Lookup::Ligature(s.liga.iter().map(|(c, l)| (c.iter().map(|n| s.gid(n)).collect(), s.gid(l))).collect()));
    }
    if let Some(tnum) = &s.tnum {
        features.push((b"tnum", vec![lookups.len() as u16]));
        lookups.push(Lookup::Single(tnum.iter().map(|(a, b)| (s.gid(a), s.gid(b))).collect()));
    }
    if !features.is_empty() {
        tables.push(gsub(&features, &lookups));
    }
    if !s.marks.is_empty() {
        tables.push(gdef(&s.marks.iter().map(|m| s.gid(m)).collect::<Vec<_>>()));
    }
    sfnt(tables)
}

const IDS: [&str; 9] = [
    "check.metrics.baseGlyphWidth",
    "check.metrics.markWidths",
    "check.metrics.whitespaceWidths",
    "check.metrics.mathSignWidths",
    "check.metrics.tabularWidths",
    "check.metrics.digitWidths",
    "check.metrics.fixedFractionWidths",
    "check.metrics.ligatureWidths",
    "check.metrics.accentedWidths",
];

fn run_with(s: &Spec, id: &str, config: &Config) -> LayerReport {
    let bytes = build(s);
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let spec = find(id).unwrap_or_else(|| panic!("no check {id}"));
    let r = audit_binary(&ctx, &[spec], config);
    for f in &r.findings {
        assert!(spec.codes.contains(&f.code), "{id} emits `{}`, which its registry entry does not list", f.code);
        assert_eq!(f.location.font.as_deref(), Some("f0"));
        assert!(f.data.values().all(|v| v.is_finite()));
    }
    r
}

fn run(s: &Spec, id: &str) -> Vec<Finding> {
    let r = run_with(s, id, &Config::default());
    assert!(r.skipped.is_empty(), "{id} skipped: {:?}", r.skipped);
    r.findings
}

fn hits(findings: &[Finding]) -> Vec<(&'static str, &str)> {
    findings.iter().map(|f| (f.code, f.location.glyph.as_str())).collect()
}

#[test]
fn every_width_check_is_registered_with_its_scope() {
    for id in IDS {
        let spec = find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(spec.scope, CheckScope::Binary, "{id}");
    }
    for id in ["check.metrics.metricsKeyOutOfSync", "check.metrics.negativeAdvance"] {
        assert_eq!(find(id).unwrap().scope, CheckScope::Glyph, "{id}");
    }
    let source = ["check.metrics.metricsKeyOutOfSync", "check.metrics.negativeAdvance"];
    let width_checks: Vec<_> = CHECKS.iter().filter(|c| IDS.contains(&c.id) || source.contains(&c.id)).collect();
    assert_eq!(width_checks.len(), IDS.len() + 2);
    for spec in width_checks {
        for (code, _) in spec.code_severities {
            assert!(spec.codes.contains(code), "{}: severity for unlisted `{code}`", spec.id);
        }
    }
}

#[test]
fn the_clean_font_passes_every_width_check() {
    for id in IDS {
        let findings = run(&base(), id);
        assert!(findings.is_empty(), "{id}: {findings:#?}");
    }
}

#[test]
fn base_glyph_width_reports_empty_base_glyphs_but_not_marks_or_private_use() {
    let s = base().width("A", 0).width("one.tf", 0).add(G { name: "uni0302", cp: Some(0x302), adv: 0, x: None });
    assert_eq!(hits(&run(&s, "check.metrics.baseGlyphWidth")), vec![("zero-width", "A")]);
}

#[test]
fn mark_widths_wants_combining_marks_empty_and_spacing_modifiers_wide() {
    let s = base().width("acutecomb", 200).width("circumflex", 0);
    let findings = run(&s, "check.metrics.markWidths");
    assert_eq!(hits(&findings), vec![("spacing-zero", "circumflex"), ("non-spacing-not-zero", "acutecomb")]);
    assert!(findings[0].fix.is_none(), "giving a modifier a width is not the zero-width fix");
    assert_eq!(findings[1].fix.map(|f| f.id), Some("zeroMarkWidths"));
}

#[test]
fn whitespace_widths_compares_the_no_break_space_and_the_other_spaces() {
    let findings = run(&base().width("nbspace", 300), "check.metrics.whitespaceWidths");
    assert_eq!(hits(&findings), vec![("different-widths", "nbspace")]);
    assert_eq!(findings[0].severity, Severity::Fail);

    let zero = run(&base().width("space", 0).width("nbspace", 0), "check.metrics.whitespaceWidths");
    assert_eq!(zero.first().map(|f| f.code), Some("zero-space"));

    // Thin space: between a sixth (166) and a fifth (200) of the em; 100 is too narrow.
    let thin = run(&base().add(g("thinspace", 0x2009, 100)).add(g("figurespace", 0x2007, 600)), "check.metrics.whitespaceWidths");
    assert_eq!(hits(&thin), vec![("bad-whitespace-advance", "thinspace")]);
    assert_eq!(thin[0].severity, Severity::Warn);

    let mut missing = base();
    missing.glyph("nbspace").cp = None;
    let r = run_with(&missing, "check.metrics.whitespaceWidths", &Config::default());
    assert!(r.findings.is_empty() && r.skipped.len() == 1, "no no-break space: skipped");
}

#[test]
fn math_sign_widths_names_the_glyphs_off_the_common_width() {
    let findings = run(&base().width("minus", 500), "check.metrics.mathSignWidths");
    assert_eq!(hits(&findings), vec![("width-outliers", "minus")]);
    assert_eq!(findings[0].data.get("expected"), Some(&600.0));
}

#[test]
fn tabular_widths_checks_tnum_targets_name_groups_and_the_figure_space() {
    let tnum = run(&base().width("one.tf", 550), "check.metrics.tabularWidths");
    assert_eq!(tnum.iter().map(|f| f.code).collect::<Vec<_>>(), vec!["tnum-unequal"]);
    assert_eq!(tnum[0].location.feature.as_deref(), Some("tnum"));

    // The lining group: nine of ten figures at 600, so `one` at 500 is reported (tnum maps it to
    // the 600-unit one.tf, so the feature itself is fine).
    let group = run(&base().width("one", 500), "check.metrics.tabularWidths");
    assert_eq!(hits(&group), vec![("group-unequal", "one")]);

    // Fewer than ten glyphs of a group are not judged.
    let mut nine = base().width("one", 500);
    nine.glyph("nine").name = "nine.alt";
    nine.tnum = None;
    assert!(run(&nine, "check.metrics.tabularWidths").is_empty());

    let space = run(
        &base().add(G { name: "one.tab", cp: None, adv: 600, x: Some((60, 540)) }).add(g("figurespace", 0x2007, 550)),
        "check.metrics.tabularWidths",
    );
    assert_eq!(hits(&space), vec![("figure-space", "figurespace")]);
}

#[test]
fn digit_widths_wants_one_figure_width_and_comma_like_period() {
    let mut s = base().width("seven", 550).width("comma", 300);
    s.glyph("nine").cp = None;
    let findings = run(&s, "check.metrics.digitWidths");
    let codes: Vec<&str> = findings.iter().map(|f| f.code).collect();
    assert_eq!(codes, vec!["bad-digit-width", "missing-digit", "comma-period"]);
    assert_eq!(findings[0].location.glyph, "seven");
}

#[test]
fn fixed_fraction_widths_wants_en_and_em_dashes_at_their_widths() {
    let findings = run(&base().width("endash", 520).width("emdash", 900), "check.metrics.fixedFractionWidths");
    assert_eq!(hits(&findings), vec![("not-half-em", "endash"), ("not-em", "emdash")]);
}

#[test]
fn ligature_widths_compares_the_ligature_with_its_first_and_last_components() {
    let narrow = run(&base().width("f_i", 280), "check.metrics.ligatureWidths");
    assert!(narrow.iter().any(|f| f.code == "not-wider" && f.location.glyph == "f_i"), "{narrow:#?}");

    let mut shifted = base();
    shifted.glyph("f_i").x = Some((40, 460));
    let findings = run(&shifted, "check.metrics.ligatureWidths");
    assert_eq!(hits(&findings), vec![("left-sidebearing", "f_i"), ("right-sidebearing", "f_i")]);

    let mut tolerant = Config::default();
    tolerant.set("check.metrics.ligatureWidths", "tolerance", 10.0).unwrap();
    assert!(run_with(&shifted, "check.metrics.ligatureWidths", &tolerant).findings.is_empty());
}

#[test]
fn accented_widths_compares_accented_glyphs_with_their_base() {
    let findings = run(&base().width("Aacute", 620), "check.metrics.accentedWidths");
    assert_eq!(hits(&findings), vec![("width-differs", "Aacute")]);
    assert_eq!(findings[0].fix.map(|f| f.id), Some("updateMetrics"));
    let mut tolerant = Config::default();
    tolerant.set("check.metrics.accentedWidths", "tolerance", 20.0).unwrap();
    assert!(run_with(&base().width("Aacute", 620), "check.metrics.accentedWidths", &tolerant).findings.is_empty());
}

#[test]
fn accent_names_are_stripped_the_way_comparefamily_strips_them() {
    let mut sorted = ACCENT_NAME_PARTS.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, ACCENT_NAME_PARTS.to_vec(), "the parts are removed in sorted order");
    assert_eq!(accent_base("Aacute").as_deref(), Some("A"));
    assert_eq!(accent_base("edotaccent").as_deref(), Some("e"));
    assert_eq!(accent_base("aringacute").as_deref(), Some("a"));
    assert_eq!(accent_base("zero.slash").as_deref(), Some("zero"));
    assert_eq!(accent_base("acutecomb").as_deref(), Some("comb"));
    assert_eq!(accent_base("acute"), None);
    assert_eq!(accent_base("one.inferior"), None);
    assert_eq!(accent_base("dot."), None, "nothing left after the stripping");
}

#[test]
fn a_font_without_hmtx_makes_every_width_check_skip() {
    let mut s = base();
    s.no_hmtx = true;
    for id in IDS {
        let r = run_with(&s, id, &Config::default());
        assert!(r.findings.is_empty(), "{id}: {:#?}", r.findings);
        assert_eq!(r.skipped.len(), 1, "{id}");
    }
}

#[test]
fn negative_advance_reports_master_layers_and_ignores_backups() {
    let mut font = Font::new("Test");
    let master = font.masters[0].id.clone();
    {
        let mut glyph = font.glyph_mut("A").unwrap();
        glyph.layers[0].width = -10.0;
        let mut backup = Layer::new(master.clone(), -5.0);
        backup.id = "backup-1".into();
        glyph.layers.push(backup);
    }
    let audit = AuditContext::new(&font);
    let spec = find("check.metrics.negativeAdvance").unwrap();
    let report = |name: &str| {
        let ctx = GlyphCtx::new(&audit, font.glyph(name).unwrap(), &NoServices);
        audit_glyph(&ctx, &[spec], &Config::default())
    };
    let r = report("A");
    assert_eq!(r.findings.len(), 1, "{:#?}", r.findings);
    assert_eq!(r.findings[0].code, "negative-width");
    assert_eq!(r.findings[0].location.layer, master);
    assert_eq!(r.findings[0].location.glyph, "A");
    assert!(report("B").findings.is_empty());
}

#[test]
fn metrics_key_out_of_sync_reports_through_the_spacing_scan() {
    let mut font = Font::new("Test");
    let master = font.masters[0].id.clone();
    {
        let mut a = font.glyph_mut("A").unwrap();
        a.layers[0].width = 500.0;
        a.info.metrics_keys.width = Some("=600".into());
    }
    font.glyph_mut("B").unwrap().info.metrics_keys.width = Some("=missing".into());
    let audit = AuditContext::new(&font);
    let spec = find("check.metrics.metricsKeyOutOfSync").unwrap();
    let report = |name: &str| {
        let ctx = GlyphCtx::new(&audit, font.glyph(name).unwrap(), &NoServices);
        audit_glyph(&ctx, &[spec], &Config::default())
    };
    let a = report("A");
    assert!(a.skipped.is_empty(), "{:?}", a.skipped);
    assert_eq!(hits(&a.findings), vec![("out-of-sync", "A")]);
    assert_eq!(a.findings[0].location.layer, master);
    assert_eq!(a.findings[0].fix.map(|f| f.id), Some("updateMetrics"));
    let b = report("B");
    assert_eq!(hits(&b.findings), vec![("unlinked", "B")]);
    assert!(b.findings[0].fix.is_none(), "an unlinked key has no update fix");
    assert!(report("space").findings.is_empty());
}

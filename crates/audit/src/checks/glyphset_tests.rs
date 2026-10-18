//! Behaviour tests of the `check.glyphset.*` checks and `check.outlines.contourCount`, on
//! TrueType fonts assembled byte by byte and on sources built in code.

use fontcraft_model::{Font, Glyph};

use super::{is_uni_name, name_buffer_size, valid_glyph_name};
use crate::checks::kerning::test_font::{W, metrics, sfnt};
use crate::{
    AuditContext, BinaryCtx, CheckGroup, CheckScope, Config, LayerReport, NoServices, Severity, SourceCtx, audit_binary, audit_source, find, select,
};

#[derive(Clone)]
enum Draw {
    Empty,
    /// This many nested squares.
    Boxes(u16),
    /// One square of this side.
    Square(i16),
    /// Components, by glyph id.
    Composite(Vec<u16>),
}

#[derive(Clone)]
struct G {
    name: String,
    cps: Vec<u32>,
    draw: Draw,
}

fn g(name: &str, cps: &[u32], draw: Draw) -> G {
    G { name: name.to_string(), cps: cps.to_vec(), draw }
}

#[derive(Clone, Copy, PartialEq)]
enum CmapKind {
    Unicode,
    /// Only a platform 3 encoding 10 subtable.
    FullRepertoireOnly,
    Missing,
}

#[derive(Clone)]
struct Spec {
    glyphs: Vec<G>,
    cmap: CmapKind,
    family: String,
    /// OS/2 Unicode ranges 1 to 3 and code page range 1.
    ranges: [u32; 3],
    code_pages: u32,
    default_char: u16,
    /// Glyph ids GDEF classes as marks.
    marks: Vec<u16>,
    /// (input, output) pairs of one single substitution lookup.
    substitutions: Vec<(u16, u16)>,
}

impl Spec {
    fn gid(&self, name: &str) -> u16 {
        self.glyphs.iter().position(|g| g.name == name).unwrap_or_else(|| panic!("no glyph {name}")) as u16
    }

    fn glyph(&mut self, name: &str) -> &mut G {
        self.glyphs.iter_mut().find(|g| g.name == name).unwrap_or_else(|| panic!("no glyph {name}"))
    }

    fn build(&self) -> Vec<u8> {
        let n = self.glyphs.len() as u16;
        let (hhea, hmtx) = metrics(&vec![500; self.glyphs.len()]);
        let (glyf, loca) = glyf_loca(&self.glyphs);
        let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
            (b"head", head()),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"maxp", maxp(n)),
            (b"glyf", glyf),
            (b"loca", loca),
            (b"post", post(&self.glyphs.iter().map(|g| g.name.as_str()).collect::<Vec<_>>())),
            (b"OS/2", os2(self.ranges, self.code_pages, self.default_char)),
            (b"name", name(&self.family)),
        ];
        let mut map: Vec<(u32, u16)> = Vec::new();
        for (i, g) in self.glyphs.iter().enumerate() {
            map.extend(g.cps.iter().map(|c| (*c, i as u16)));
        }
        map.sort_unstable();
        match self.cmap {
            CmapKind::Unicode => tables.push((b"cmap", cmap(3, 1, &map))),
            CmapKind::FullRepertoireOnly => tables.push((b"cmap", cmap(3, 10, &map))),
            CmapKind::Missing => {}
        }
        if !self.marks.is_empty() {
            tables.push((b"GDEF", gdef(n, &self.marks)));
        }
        if !self.substitutions.is_empty() {
            tables.push((b"GSUB", gsub_single(&self.substitutions)));
        }
        sfnt(tables)
    }
}

/// A clean font: the four leading TrueType glyphs, the two spaces, A, a, B (a composite of
/// A.base), b and the unencoded A.base.
fn clean() -> Spec {
    Spec {
        glyphs: vec![
            g(".notdef", &[], Draw::Square(400)),
            g("NULL", &[0x0000], Draw::Empty),
            g("CR", &[0x000D], Draw::Empty),
            g("space", &[0x0020], Draw::Empty),
            g("uni00A0", &[0x00A0], Draw::Empty),
            g("A", &[0x0041], Draw::Boxes(2)),
            g("a", &[0x0061], Draw::Boxes(2)),
            g("B", &[0x0042], Draw::Composite(vec![9])),
            g("b", &[0x0062], Draw::Boxes(2)),
            g("A.base", &[], Draw::Boxes(2)),
        ],
        cmap: CmapKind::Unicode,
        family: "AaBb".into(),
        ranges: [0; 3],
        code_pages: 0,
        default_char: 0,
        marks: Vec::new(),
        substitutions: Vec::new(),
    }
}

fn with(change: impl FnOnce(&mut Spec)) -> Vec<u8> {
    let mut s = clean();
    change(&mut s);
    s.build()
}

// ---------------------------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------------------------

fn square(x: i16, y: i16, side: i16) -> [(i16, i16); 4] {
    [(x, y), (x, y + side), (x + side, y + side), (x + side, y)]
}

fn simple(contours: &[[(i16, i16); 4]]) -> Vec<u8> {
    let points: Vec<(i16, i16)> = contours.iter().flatten().copied().collect();
    let xs = points.iter().map(|p| p.0);
    let ys = points.iter().map(|p| p.1);
    let mut w = W::default();
    w.i16(contours.len() as i16)
        .i16(xs.clone().min().unwrap_or(0))
        .i16(ys.clone().min().unwrap_or(0))
        .i16(xs.max().unwrap_or(0))
        .i16(ys.max().unwrap_or(0));
    for k in 0..contours.len() {
        w.u16((4 * k + 3) as u16);
    }
    w.u16(0);
    for _ in &points {
        w.bytes(&[0x01]);
    }
    let mut prev = 0;
    for p in &points {
        w.i16(p.0 - prev);
        prev = p.0;
    }
    prev = 0;
    for p in &points {
        w.i16(p.1 - prev);
        prev = p.1;
    }
    w.0
}

fn composite(parts: &[u16]) -> Vec<u8> {
    let mut w = W::default();
    w.i16(-1).i16(0).i16(0).i16(500).i16(500);
    for (i, p) in parts.iter().enumerate() {
        let more = if i + 1 < parts.len() { 0x0020 } else { 0 };
        w.u16(0x0003 | more).u16(*p).i16(0).i16(0);
    }
    w.0
}

fn glyf_loca(glyphs: &[G]) -> (Vec<u8>, Vec<u8>) {
    let mut glyf = Vec::new();
    let mut loca = W::default();
    for g in glyphs {
        loca.u32(glyf.len() as u32);
        let mut data = match &g.draw {
            Draw::Empty => Vec::new(),
            Draw::Boxes(n) => simple(&(0..*n as i16).map(|k| square(100 + 20 * k, 100 + 20 * k, 400 - 40 * k)).collect::<Vec<_>>()),
            Draw::Square(side) => simple(&[square(50, 0, *side)]),
            Draw::Composite(parts) => composite(parts),
        };
        while data.len() % 4 != 0 {
            data.push(0);
        }
        glyf.extend(data);
    }
    loca.u32(glyf.len() as u32);
    (glyf, loca.0)
}

fn head() -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0001_0000).u32(0x0001_0000).u32(0).u32(0x5F0F_3CF5).u16(0).u16(1000);
    w.u32(0).u32(0).u32(0).u32(0);
    w.i16(0).i16(-200).i16(1000).i16(800).u16(0).u16(8).i16(2).i16(1).i16(0);
    w.0
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0001_0000).u16(glyphs);
    for v in [64, 16, 64, 16, 2, 0, 0, 0, 0, 0, 0, 4, 4] {
        w.u16(v);
    }
    w.0
}

fn post(names: &[&str]) -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0002_0000).u32(0).i16(-100).i16(50).u32(0).u32(0).u32(0).u32(0).u32(0);
    w.u16(names.len() as u16);
    for i in 0..names.len() {
        w.u16(258 + i as u16);
    }
    for n in names {
        w.bytes(&[n.len() as u8]).bytes(n.as_bytes());
    }
    w.0
}

fn os2(ranges: [u32; 3], code_pages: u32, default_char: u16) -> Vec<u8> {
    let mut w = W::default();
    w.u16(4).i16(500).u16(400).u16(5).u16(0);
    for _ in 0..11 {
        w.i16(0);
    }
    w.bytes(&[0; 10]);
    w.u32(ranges[0]).u32(ranges[1]).u32(ranges[2]).u32(0);
    w.tag(b"TEST").u16(0x0040).u16(0x20).u16(0x62);
    w.i16(800).i16(-200).i16(0).u16(1000).u16(200);
    w.u32(code_pages).u32(0);
    w.i16(500).i16(700).u16(default_char).u16(0x20).u16(0);
    w.0
}

/// A `name` table with the Windows English family name.
fn name(family: &str) -> Vec<u8> {
    let text: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut w = W::default();
    w.u16(0).u16(1).u16(6 + 12);
    w.u16(3).u16(1).u16(0x409).u16(1).u16(text.len() as u16).u16(0);
    w.bytes(&text);
    w.0
}

/// A `cmap` with one format 12 subtable under (`platform`, `encoding`).
fn cmap(platform: u16, encoding: u16, map: &[(u32, u16)]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(0).u16(1).u16(platform).u16(encoding).u32(12);
    w.u16(12).u16(0).u32(16 + 12 * map.len() as u32).u32(0).u32(map.len() as u32);
    for (c, g) in map {
        w.u32(*c).u32(*c).u32(u32::from(*g));
    }
    w.0
}

/// GDEF version 1.0 whose glyph classes make `marks` marks (class 3) and every other glyph a
/// base (class 1).
fn gdef(glyphs: u16, marks: &[u16]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(1).u16(0).u16(12).u16(0).u16(0).u16(0);
    w.u16(1).u16(0).u16(glyphs);
    for gid in 0..glyphs {
        w.u16(if marks.contains(&gid) { 3 } else { 1 });
    }
    w.0
}

/// GSUB with one single substitution lookup (format 2), referenced by no feature.
fn gsub_single(pairs: &[(u16, u16)]) -> Vec<u8> {
    let mut pairs = pairs.to_vec();
    pairs.sort_unstable();
    let mut sub = W::default();
    sub.u16(2).u16(6 + 2 * pairs.len() as u16).u16(pairs.len() as u16);
    for (_, out) in &pairs {
        sub.u16(*out);
    }
    sub.u16(1).u16(pairs.len() as u16);
    for (input, _) in &pairs {
        sub.u16(*input);
    }
    let mut w = W::default();
    // Header (10 bytes), empty script list at 10, empty feature list at 12, lookup list at 14.
    w.u16(1).u16(0).u16(10).u16(12).u16(14);
    w.u16(0).u16(0);
    w.u16(1).u16(4);
    w.u16(1).u16(0).u16(1).u16(8);
    w.bytes(&sub.0);
    w.0
}

// ---------------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------------

fn report(check: &str, bytes: &[u8]) -> LayerReport {
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    let spec = find(check).unwrap_or_else(|| panic!("{check} is not registered"));
    let report = audit_binary(&ctx, &[spec], &Config::default());
    for f in &report.findings {
        assert!(spec.codes.contains(&f.code), "{check} emits unlisted `{}`", f.code);
        assert_eq!(f.location.font.as_deref(), Some("f0"));
        assert!(!f.message.is_empty());
    }
    report
}

fn run(check: &str, bytes: &[u8]) -> Vec<(&'static str, Severity)> {
    report(check, bytes).findings.iter().map(|f| (f.code, f.severity)).collect()
}

fn codes(check: &str, bytes: &[u8]) -> Vec<&'static str> {
    run(check, bytes).into_iter().map(|(c, _)| c).collect()
}

fn run_source(font: &Font, check: &str) -> Vec<(&'static str, String)> {
    let spec = find(check).unwrap_or_else(|| panic!("{check} is not registered"));
    let audit = AuditContext::new(font);
    let ctx = SourceCtx { audit: &audit, services: &NoServices };
    let r = audit_source(&ctx, &[spec], &Config::default());
    assert!(r.skipped.is_empty(), "{check}: {:?}", r.skipped);
    for f in &r.findings {
        assert!(spec.codes.contains(&f.code), "{check} emits unlisted `{}`", f.code);
    }
    r.findings.iter().map(|f| (f.code, f.location.glyph.clone())).collect()
}

fn binary_checks() -> Vec<&'static crate::CheckSpec> {
    let mut checks: Vec<&'static crate::CheckSpec> =
        select(None, &["check.glyphset.*"], &[]).into_iter().filter(|c| c.scope == CheckScope::Binary).collect();
    checks.extend(find("check.outlines.contourCount"));
    checks
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn the_registry_holds_the_package_checks() {
    let all = select(None, &["check.glyphset.*"], &[]);
    assert_eq!(all.len(), 24);
    assert!(all.iter().all(|c| c.group == CheckGroup::Glyphset));
    assert_eq!(all.iter().filter(|c| c.scope == CheckScope::Source).count(), 2);
    assert_eq!(find("check.outlines.contourCount").map(|c| c.scope), Some(CheckScope::Binary));
}

#[test]
fn a_clean_font_passes_every_glyphset_check() {
    let bytes = clean().build();
    for spec in binary_checks() {
        // Whether the database knows the leading TrueType glyph names is the database's business.
        if spec.id == "check.glyphset.productionNames" {
            continue;
        }
        assert!(run(spec.id, &bytes).is_empty(), "{} fires on the clean font: {:?}", spec.id, run(spec.id, &bytes));
    }
}

#[test]
fn notdef_missing_late_encoded_or_blank() {
    let missing = with(|s| s.glyph(".notdef").name = "box".into());
    assert_eq!(run("check.glyphset.notdef", &missing), vec![("notdef-not-found", Severity::Warn)]);
    let late = with(|s| {
        s.glyphs.swap(0, 9);
        s.glyph(".notdef").cps = vec![0x25A1];
        s.glyph("B").draw = Draw::Composite(vec![0]);
    });
    assert_eq!(codes("check.glyphset.notdef", &late), vec!["notdef-not-first", "notdef-has-codepoint"]);
    let blank = with(|s| s.glyph(".notdef").draw = Draw::Empty);
    assert_eq!(run("check.glyphset.notdef", &blank), vec![("notdef-is-blank", Severity::Fail)]);
}

#[test]
fn whitespace_glyphs_and_their_ink() {
    let no_nbsp = with(|s| s.glyph("uni00A0").cps.clear());
    let r = report("check.glyphset.whitespaceGlyphs", &no_nbsp);
    assert_eq!(r.findings.len(), 1);
    assert_eq!(r.findings[0].data.get("codepoint"), Some(&160.0));
    assert!(r.findings[0].fix.is_some_and(|f| f.id == "addMandatoryGlyphs"));
    let inked = with(|s| s.glyph("space").draw = Draw::Square(100));
    let r = report("check.glyphset.whitespaceInk", &inked);
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.glyph.as_str())).collect::<Vec<_>>(), vec![("has-ink", "space")]);
    // The Ogham space mark is meant to draw.
    let ogham = with(|s| s.glyph("A.base").cps = vec![0x1680]);
    assert!(codes("check.glyphset.whitespaceInk", &ogham).is_empty());
}

#[test]
fn control_characters_by_mapping_and_by_name() {
    let bytes = with(|s| {
        s.glyph("NULL").cps.push(0x0001);
        s.glyph("A.base").name = "uni0002".into();
    });
    let r = report("check.glyphset.controlCharacters", &bytes);
    assert_eq!(r.findings.iter().map(|f| f.code).collect::<Vec<_>>(), vec!["unacceptable", "unacceptable"]);
    assert!(r.findings[0].fix.is_some(), "a mapping can be removed");
    assert!(r.findings[1].fix.is_none(), "a name alone has no mapping to remove");
}

#[test]
fn truetype_leading_glyphs_order_names_encoding_and_default_char() {
    let legacy = with(|s| {
        s.glyph("NULL").name = ".null".into();
        s.default_char = 32;
    });
    assert_eq!(
        run("check.glyphset.truetypeLeadingGlyphs", &legacy),
        vec![("default-char-not-zero", Severity::Warn), ("legacy-name", Severity::Warn)]
    );
    let swapped = with(|s| s.glyphs.swap(2, 3));
    assert_eq!(codes("check.glyphset.truetypeLeadingGlyphs", &swapped), vec!["wrong-glyph-order", "wrong-glyph-order"]);
    let misencoded = with(|s| s.glyph("CR").cps = vec![0x000A]);
    assert_eq!(codes("check.glyphset.truetypeLeadingGlyphs", &misencoded), vec!["wrong-encoding"]);
    let unencoded = with(|s| s.glyph("space").cps.clear());
    assert_eq!(codes("check.glyphset.truetypeLeadingGlyphs", &unencoded), vec!["not-encoded"]);
}

#[test]
fn glyph_names_unique_and_valid() {
    let dup = with(|s| s.glyph("A.base").name = "b".into());
    let r = report("check.glyphset.uniqueGlyphNames", &dup);
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.glyph.as_str())).collect::<Vec<_>>(), vec![("duplicated-glyph-names", "b")]);
    let bad = with(|s| {
        s.glyph("A.base").name = "A base".into();
        s.glyph("b").name = "b".repeat(40);
    });
    assert_eq!(
        run("check.glyphset.validGlyphNames", &bad),
        vec![("found-invalid-names", Severity::Fail)],
        "long names are not reported next to invalid ones"
    );
    let long = with(|s| s.glyph("A.base").name = "A".repeat(40));
    assert_eq!(run("check.glyphset.validGlyphNames", &long), vec![("legacy-long-names", Severity::Warn)]);
    assert!(valid_glyph_name("twocents") && valid_glyph_name("a1") && valid_glyph_name("_"));
    assert!(!valid_glyph_name("2cents") && !valid_glyph_name(".twocents") && !valid_glyph_name(&"a".repeat(64)) && !valid_glyph_name("a-b"));
}

#[test]
fn glyph_name_buffer_size_counts_two_bytes_a_name() {
    let names = |n: usize| -> Vec<(u32, String)> { (0..n).map(|i| (i as u32, format!("{:030}", i))).collect() };
    assert_eq!(name_buffer_size(&names(10)), 320);
    let build = |n: usize| {
        let mut s = clean();
        s.glyphs.extend((0..n).map(|i| g(&format!("g{:029}", i), &[], Draw::Empty)));
        s.build()
    };
    assert_eq!(codes("check.glyphset.glyphNameBufferSize", &build(1100)), vec!["buffer-too-large"]);
    assert_eq!(run("check.glyphset.glyphNameBufferSize", &build(980)), vec![("buffer-near-limit", Severity::Warn)]);
    assert!(codes("check.glyphset.glyphNameBufferSize", &build(100)).is_empty());
}

#[test]
fn production_names_working_and_unmappable() {
    assert!(is_uni_name("uni0041") && is_uni_name("uni00410042") && is_uni_name("u1F600") && is_uni_name("u0041"));
    assert!(!is_uni_name("uni041") && !is_uni_name("uni004a") && !is_uni_name("u12") && !is_uni_name("ugly"));
    let bytes = with(|s| {
        s.glyph("uni00A0").name = "A-cy".into();
        s.glyph("A.base").name = "xyzzy".into();
        s.glyph("b").name = "A_A-cy".into();
    });
    let r = report("check.glyphset.productionNames", &bytes);
    let found: Vec<(&str, &str)> =
        r.findings.iter().map(|f| (f.code, f.location.glyph.as_str())).filter(|(_, g)| ["A-cy", "xyzzy", "A_A-cy"].contains(g)).collect();
    assert_eq!(found, vec![("working-name", "A-cy"), ("working-name", "A_A-cy"), ("unmappable-name", "xyzzy")]);
}

#[test]
fn unreachable_glyphs_and_what_reaches_them() {
    let alt = with(|s| s.glyphs.push(g("a.alt", &[], Draw::Boxes(2))));
    let r = report("check.glyphset.unreachableGlyphs", &alt);
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.glyph.as_str())).collect::<Vec<_>>(), vec![("unreachable-glyphs", "a.alt")]);
    let substituted = with(|s| {
        s.glyphs.push(g("a.alt", &[], Draw::Boxes(2)));
        s.substitutions = vec![(s.gid("a"), s.gid("a.alt"))];
    });
    assert!(codes("check.glyphset.unreachableGlyphs", &substituted).is_empty());
    // A.base is reached only as the component of B.
    let orphan = with(|s| s.glyph("B").draw = Draw::Boxes(2));
    assert_eq!(codes("check.glyphset.unreachableGlyphs", &orphan), vec!["unreachable-glyphs"]);
}

#[test]
fn case_partners_and_their_exceptions() {
    let no_b = with(|s| s.glyph("b").cps.clear());
    let r = report("check.glyphset.caseMapping", &no_b);
    assert_eq!(r.findings.len(), 1);
    assert_eq!(r.findings[0].data.get("partner"), Some(&f64::from(0x62)));
    let micro = with(|s| s.glyph("A.base").cps = vec![0x00B5]);
    assert!(codes("check.glyphset.caseMapping", &micro).is_empty(), "U+00B5 is on the exception list");
}

#[test]
fn empty_letters_and_hangul() {
    let empty_a = with(|s| s.glyph("a").draw = Draw::Empty);
    assert_eq!(run("check.glyphset.emptyLetters", &empty_a), vec![("empty-letter", Severity::Fail)]);
    let filler = with(|s| s.glyph("uni00A0").cps.push(0x3164));
    assert!(codes("check.glyphset.emptyLetters", &filler).is_empty(), "Hangul fillers draw nothing");
    // U+AC02 is not one of the 2,350 KS X 1001 syllables.
    let rare = with(|s| s.glyph("uni00A0").cps.push(0xAC02));
    assert_eq!(run("check.glyphset.emptyLetters", &rare), vec![("empty-hangul-letter", Severity::Warn)]);
}

#[test]
fn soft_hyphen_rupee_and_cjk_count() {
    let shy = with(|s| s.glyph("A.base").cps = vec![0x00AD]);
    assert_eq!(run("check.glyphset.softHyphen", &shy), vec![("softhyphen", Severity::Warn)]);
    let indic = with(|s| s.glyph("A.base").cps = vec![0x0908]);
    assert_eq!(codes("check.glyphset.rupee", &indic), vec!["missing-rupee"]);
    let with_rupee = with(|s| s.glyph("A.base").cps = vec![0x0908, 0x20B9]);
    assert!(codes("check.glyphset.rupee", &with_rupee).is_empty());
    assert!(!report("check.glyphset.rupee", &clean().build()).skipped.is_empty(), "skipped without an Indic script");
    let cjk = with(|s| {
        s.code_pages = 1 << 17;
        s.glyph("A.base").cps = vec![0x3042];
    });
    assert_eq!(run("check.glyphset.cjkGlyphCount", &cjk), vec![("cjk-not-enough-glyphs", Severity::Warn)]);
    let range_claim = with(|s| {
        s.ranges = [0, 1 << (59 - 32), 0];
        s.glyph("A.base").cps = vec![0x4E00];
    });
    assert_eq!(codes("check.glyphset.cjkGlyphCount", &range_claim), vec!["cjk-not-enough-glyphs"]);
    assert!(!report("check.glyphset.cjkGlyphCount", &clean().build()).skipped.is_empty(), "skipped without a CJK claim");
}

#[test]
fn arabic_marks_in_gdef_and_the_high_hamza_size() {
    let hamza = |high_side: i16| {
        let mut s = clean();
        s.glyphs.push(g("hamza", &[0x0621], Draw::Square(300)));
        s.glyphs.push(g("highhamza", &[0x0674], Draw::Square(high_side)));
        s.marks = vec![s.gid("highhamza")];
        s.build()
    };
    assert_eq!(run("check.glyphset.arabicHighHamza", &hamza(100)), vec![("mark-in-gdef", Severity::Fail), ("glyph-area", Severity::Warn)]);
    assert_eq!(codes("check.glyphset.arabicHighHamza", &hamza(300)), vec!["mark-in-gdef"]);
    let symbol = with(|s| {
        s.glyphs.push(g("dotabove", &[0xFBB2], Draw::Square(50)));
        s.marks = vec![s.gid("dotabove")];
    });
    let r = report("check.glyphset.arabicSpacingSymbols", &symbol);
    assert_eq!(r.findings.iter().map(|f| f.code).collect::<Vec<_>>(), vec!["mark-in-gdef"]);
    assert!(r.findings[0].fix.is_some_and(|f| f.id == "setGdefClasses"));
}

#[test]
fn unicode_cmap_and_double_encoding() {
    let no_unicode = with(|s| s.cmap = CmapKind::FullRepertoireOnly);
    assert_eq!(codes("check.glyphset.unicodeCmapMissing", &no_unicode), vec!["no-unicode-cmap"]);
    let no_cmap = with(|s| s.cmap = CmapKind::Missing);
    assert_eq!(codes("check.glyphset.unicodeCmapMissing", &no_cmap), vec!["no-cmap"]);
    let double = with(|s| s.glyph("A").cps.push(0x0391));
    let r = report("check.glyphset.doubleEncodedGlyphs", &double);
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.glyph.as_str())).collect::<Vec<_>>(), vec![("double-mapped", "A")]);
}

#[test]
fn private_use_code_points() {
    let pua = with(|s| s.glyph("A.base").cps = vec![0xE000, 0xF8FF]);
    let r = report("check.glyphset.privateUseCodepoints", &pua);
    assert_eq!(r.findings.iter().map(|f| (f.code, f.severity)).collect::<Vec<_>>(), vec![("pua-encoded", Severity::Warn)], "U+F8FF is exempt");
    assert_eq!(r.findings[0].data.get("codepoint"), Some(&f64::from(0xE000)));
}

#[test]
fn minimum_coverage_skips_without_the_glyph_set() {
    let r = report("check.glyphset.minimumCoverage", &clean().build());
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1);
}

#[test]
fn minimum_coverage_compares_the_cmap_with_the_project_glyph_set() {
    let bytes = clean().build();
    let set = fontcraft_model::glyphdata::sets::builtin()
        .unwrap()
        .glyph_sets
        .iter()
        .find(|s| s.glyphs.iter().any(|g| g.code_point.is_some_and(|c| c > 0x7F)))
        .expect("a built-in glyph set with characters beyond ASCII");
    let spec = find("check.glyphset.minimumCoverage").unwrap();
    let with_set = |value: &str| {
        let values = std::collections::BTreeMap::from([("glyphSet".to_string(), crate::profile::ProjectValue::Text(value.to_string()))]);
        let services = NoServices;
        let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap().with_values(&values);
        audit_binary(&ctx, &[spec], &Config::default())
    };
    for value in [set.id.as_str(), set.name.as_str()] {
        let r = with_set(value);
        assert!(r.skipped.is_empty(), "{value}: {:?}", r.skipped);
        assert_eq!(r.findings.iter().map(|f| f.code).collect::<Vec<_>>(), vec!["missing-codepoints"], "{value}");
        assert!(r.findings[0].data.get("count").is_some_and(|n| *n >= 1.0));
    }
    let r = with_set("no such set");
    assert!(r.findings.is_empty());
    assert!(r.skipped.iter().any(|s| s.reason.contains("built-in glyph set")), "{:?}", r.skipped);
}

#[test]
fn the_family_name_must_render() {
    let bytes = with(|s| s.family = "Ab\u{20AC}".into());
    let r = report("check.glyphset.rendersOwnName", &bytes);
    assert_eq!(r.findings.iter().map(|f| f.code).collect::<Vec<_>>(), vec!["render-own-name"]);
    assert!(r.findings[0].message.contains("U+20AC"), "{}", r.findings[0].message);
}

#[test]
fn contour_counts_against_the_expected_table() {
    let bytes = with(|s| {
        s.glyph("a").draw = Draw::Boxes(1);
        s.glyph("A").draw = Draw::Empty;
    });
    let r = report("check.outlines.contourCount", &bytes);
    let found: Vec<(&str, Severity, &str)> = r.findings.iter().map(|f| (f.code, f.severity, f.location.glyph.as_str())).collect();
    assert_eq!(found, vec![("no-contour", Severity::Fail, "A"), ("contour-count", Severity::Warn, "a")]);
    // The composite B counts its component's two contours.
    let b = r.findings.iter().find(|f| f.location.glyph == "B");
    assert!(b.is_none());
    assert_eq!(codes("check.outlines.contourCount", &with(|s| s.cmap = CmapKind::Missing)), vec!["lacks-cmap"]);
}

#[test]
fn hostile_fonts_do_not_panic() {
    // A composite that contains itself, one that points past the glyph count, and garbage.
    let cyclic = with(|s| {
        s.glyph("B").draw = Draw::Composite(vec![7, 9]);
        s.glyph("A.base").draw = Draw::Composite(vec![9, 500]);
    });
    let mut truncated = clean().build();
    truncated.truncate(truncated.len() / 2);
    let mut noise = clean().build();
    for (i, b) in noise.iter_mut().enumerate().skip(200) {
        *b = (i as u8).wrapping_mul(37);
    }
    let fonts = [cyclic, truncated, noise];
    for bytes in &fonts {
        let services = NoServices;
        let Ok(ctx) = BinaryCtx::new("f0", bytes, 0, &services) else { continue };
        let report = audit_binary(&ctx, &binary_checks(), &Config::default());
        for f in &report.findings {
            assert!(!f.message.is_empty());
        }
    }
}

fn source(glyphs: Vec<(&str, Vec<u32>, bool)>) -> Font {
    let mut font = Font::new("Test");
    for (name, cps, export) in glyphs {
        let mut g = Glyph::new(name);
        g.unicodes = cps;
        g.export = export;
        font.glyphs.push(g);
    }
    font
}

#[test]
fn duplicate_code_points_in_a_source() {
    let font = source(vec![("A", vec![0x41], true), ("A.alt", vec![0x41], true), ("A.ss01", vec![0x41], false), ("B", vec![0x42], true)]);
    assert_eq!(run_source(&font, "check.glyphset.duplicateCodepoint"), vec![("duplicate-codepoint", "A.alt".to_string())]);
}

#[test]
fn code_points_that_disagree_with_the_name() {
    let font = source(vec![
        ("A", vec![0x42], true),
        ("uni0043", vec![], true),
        ("B", vec![0x42], true),
        ("D", vec![], false),
        ("nbspace", vec![0xA0], true),
        ("a.sc", vec![], true),
    ]);
    let found = run_source(&font, "check.glyphset.nameCodepointMismatch");
    assert_eq!(found, vec![("codepoint-mismatch", "A".to_string()), ("codepoint-mismatch", "uni0043".to_string())]);
}

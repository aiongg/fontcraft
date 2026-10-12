use fontcraft_model::{Component, Font, Glyph, Layer, Path, Shape, Transform};

use crate::{AuditContext, BinaryCtx, CheckSpec, Config, LayerReport, NoServices, Severity, audit_binary, audit_layer, find};

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
    Shape::Path(Path::rect(x0, y0, x1, y1))
}

fn comp(name: &str, dx: f64, dy: f64) -> Shape {
    Shape::Component(Component::new(name, dx, dy))
}

fn matrix(name: &str, m: [f64; 6]) -> Shape {
    let mut c = Component::new(name, 0.0, 0.0);
    c.transform = Transform::Matrix(m);
    Shape::Component(c)
}

pub(crate) fn glyph(name: &str, shapes: Vec<Shape>) -> Glyph {
    let mut g = Glyph::new(name);
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes = shapes;
    g.layers.push(layer);
    g
}

fn spec(id: &str) -> &'static CheckSpec {
    find(id).unwrap_or_else(|| panic!("no check {id}"))
}

fn run(font: &Font, glyph: &str, id: &str) -> LayerReport {
    let ctx = AuditContext::new(font);
    let g = font.glyph(glyph).unwrap();
    audit_layer(&ctx, g, &g.layers[0], &[spec(id)], &Config::default())
}

/// A small font of clean composites, plus one glyph per fault.
fn font() -> Font {
    let mut font = Font::new("Test");
    // The fixture's own glyphs only: the new font's alphabet would shadow them.
    font.glyphs.retain(|_| false);
    let mut part = glyph("_part", vec![rect(0.0, 0.0, 50.0, 50.0)]);
    part.export = false;
    for g in [
        glyph("a", vec![rect(0.0, 0.0, 400.0, 500.0)]),
        glyph("acutecomb", vec![rect(150.0, 600.0, 250.0, 700.0)]),
        glyph("aacute", vec![comp("a", 0.0, 0.0), comp("acutecomb", 0.0, 0.0)]),
        glyph("quotesingle", vec![rect(0.0, 500.0, 50.0, 700.0)]),
        glyph("dot", vec![rect(0.0, 0.0, 50.0, 50.0)]),
        glyph("space", vec![]),
        part,
        // Faults.
        glyph("aacute.ss01", vec![comp("aacute", 0.0, 0.0)]),
        glyph("u", vec![matrix("a", [1.0, 0.0, 0.0, -1.0, 0.0, 500.0])]),
        glyph("a.big", vec![matrix("a", [2.0, 0.0, 0.0, 2.0, 0.0, 0.0])]),
        glyph("quotedbl", vec![comp("quotesingle", 100.0, 0.0), comp("quotesingle", 100.0, 0.0)]),
        glyph("aa", vec![comp("a", 0.0, 0.0), comp("a", 200.0, 0.0)]),
        glyph("adot", vec![comp("a", 0.0, 0.0), comp("dot", 100.0, 100.0)]),
        glyph("a_a", vec![comp("a", 0.0, 0.0), comp("a", 400.0, 0.0)]),
        glyph("amixed", vec![rect(0.0, 0.0, 10.0, 10.0), comp("a", 0.0, 0.0)]),
        glyph("orphan", vec![comp("nothere", 0.0, 0.0)]),
        glyph("x1", vec![comp("x2", 0.0, 0.0)]),
        glyph("x2", vec![comp("x1", 0.0, 0.0)]),
        glyph("loop", vec![comp("loop", 0.0, 0.0)]),
        glyph("useit", vec![comp("_part", 0.0, 0.0)]),
        glyph("spacecomp", vec![comp("space", 0.0, 0.0)]),
    ] {
        font.glyphs.push(g);
    }
    font
}

const CLEAN: [&str; 5] = ["a", "acutecomb", "aacute", "quotesingle", "dot"];

const LAYER_CHECKS: [&str; 9] = [
    "check.components.nested",
    "check.components.transformed",
    "check.components.duplicate",
    "check.components.overlap",
    "check.components.mixedWithOutlines",
    "check.components.orphaned",
    "check.components.circular",
    "check.components.nonExportingBase",
    "check.components.emptyBase",
];

#[test]
fn every_component_check_reports_its_fault_and_passes_clean_composites() {
    let font = font();
    let cases: [(&str, &str, &str); 11] = [
        ("check.components.nested", "aacute.ss01", "nested"),
        ("check.components.transformed", "u", "mirrored"),
        ("check.components.transformed", "a.big", "scaled-or-rotated"),
        ("check.components.duplicate", "quotedbl", "duplicate"),
        ("check.components.overlap", "aa", "components-overlap"),
        ("check.components.overlap", "adot", "components-overlap"),
        ("check.components.mixedWithOutlines", "amixed", "mixed"),
        ("check.components.orphaned", "orphan", "missing-base"),
        ("check.components.circular", "x1", "cycle"),
        ("check.components.nonExportingBase", "useit", "non-exporting-base"),
        ("check.components.emptyBase", "spacecomp", "empty-base"),
    ];
    for (id, glyph, code) in cases {
        let r = run(&font, glyph, id);
        assert!(r.skipped.is_empty(), "{id} skipped {glyph}: {:?}", r.skipped);
        assert_eq!(r.findings.len(), 1, "{id} on {glyph}: {:#?}", r.findings);
        let f = &r.findings[0];
        assert_eq!(f.code, code, "{id} on {glyph}");
        assert!(spec(id).codes.contains(&f.code));
        assert_eq!(f.location.glyph, glyph);
        assert!(!f.location.components.is_empty(), "{id}: findings name their components");
    }
    for id in LAYER_CHECKS {
        for glyph in CLEAN {
            assert!(run(&font, glyph, id).findings.is_empty(), "{id} reports the clean glyph {glyph}");
        }
    }
}

#[test]
fn component_findings_carry_severities_addresses_and_fixes() {
    let font = font();
    let scaled = run(&font, "a.big", "check.components.transformed");
    assert_eq!(scaled.findings[0].severity, Severity::Warn, "only mirroring fails without knowing the hinting");
    assert_eq!(run(&font, "u", "check.components.transformed").findings[0].severity, Severity::Fail);

    let dup = &run(&font, "quotedbl", "check.components.duplicate").findings[0];
    assert_eq!(dup.location.shapes, vec![1], "the later copy is the one to delete");
    assert_eq!(dup.message, "Shape 2 repeats shape 1: quotesingle at the same place.");
    assert_eq!(dup.fix.and_then(|f| f.command), Some("edit.delete"));

    let nested = &run(&font, "aacute.ss01", "check.components.nested").findings[0];
    assert_eq!(nested.fix.and_then(|f| f.command), Some("component.flatten"));

    // Components that only touch along an edge do not overlap.
    assert!(run(&font, "a_a", "check.components.overlap").findings.is_empty());
    // Both glyphs of a cycle report it; a cycle that does not pass through the glyph does not.
    assert_eq!(run(&font, "x2", "check.components.circular").findings.len(), 1);
    assert_eq!(run(&font, "loop", "check.components.circular").findings.len(), 1);
    assert!(run(&font, "aacute.ss01", "check.components.circular").findings.is_empty());
}

#[test]
fn glyphs_that_do_not_export_are_not_held_to_export_rules() {
    let mut font = font();
    let mut g = glyph("u.off", vec![matrix("a", [1.0, 0.0, 0.0, -1.0, 0.0, 500.0]), rect(0.0, 0.0, 10.0, 10.0)]);
    g.export = false;
    font.glyphs.push(g);
    for id in ["check.components.transformed", "check.components.mixedWithOutlines"] {
        assert!(run(&font, "u.off", id).findings.is_empty(), "{id}");
    }
}

#[test]
fn hostile_component_graphs_do_not_panic() {
    let mut font = Font::new("Test");
    // A chain longer than the depth limit, and a wide fan-out.
    for i in 0..100 {
        font.glyphs.push(glyph(&format!("c{i}"), vec![comp(&format!("c{}", i + 1), 0.0, 0.0)]));
    }
    font.glyphs.push(glyph("c100", vec![rect(0.0, 0.0, 10.0, 10.0)]));
    font.glyphs.push(glyph("nan", vec![comp("c100", f64::NAN, 0.0), matrix("c100", [f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0])]));
    font.glyphs.push(glyph("fan", (0..2_000).map(|i| comp("c100", f64::from(i % 7), 0.0)).collect()));
    let checks: Vec<&'static CheckSpec> = LAYER_CHECKS.iter().map(|id| spec(id)).collect();
    let ctx = AuditContext::new(&font);
    for name in ["c0", "nan", "fan"] {
        let g = font.glyph(name).unwrap();
        let r = audit_layer(&ctx, g, &g.layers[0], &checks, &Config::default());
        for f in &r.findings {
            assert!(spec(f.check).codes.contains(&f.code), "{name}: {} emits {}", f.check, f.code);
        }
    }
    let r = run(&font, "c0", "check.components.circular");
    assert_eq!(r.findings[0].code, "too-deep");
}

/// Big-endian table writing for compiled test fonts.
#[derive(Default)]
pub(crate) struct W(pub Vec<u8>);

impl W {
    pub(crate) fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    pub(crate) fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    pub(crate) fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    pub(crate) fn tag(&mut self, t: &[u8; 4]) -> &mut Self {
        self.0.extend(t);
        self
    }
}

/// An sfnt holding `tables`.
pub(crate) fn sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let header = 12 + 16 * tables.len();
    let mut dir = W::default();
    dir.u32(0x0001_0000).u16(tables.len() as u16).u16(0).u16(0).u16(0);
    let mut body: Vec<u8> = Vec::new();
    for (tag, data) in &tables {
        dir.tag(tag).u32(0).u32((header + body.len()) as u32).u32(data.len() as u32);
        body.extend(data);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
    }
    dir.0.extend(body);
    dir.0
}

/// A glyph of a compiled test font.
#[derive(Clone)]
pub(crate) enum Tt {
    Empty,
    Simple,
    Composite(Vec<u16>),
}

pub(crate) fn maxp(n: u16) -> ([u8; 4], Vec<u8>) {
    let mut w = W::default();
    w.u32(0x0000_5000).u16(n);
    (*b"maxp", w.0)
}

/// `head`, `maxp`, `loca` (long) and `glyf` for `glyphs`.
pub(crate) fn outlines(glyphs: &[Tt]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut head = W::default();
    head.u32(0x0001_0000).u32(0).u32(0).u32(0x5F0F_3CF5).u16(0).u16(1000);
    head.0.resize(50, 0);
    head.i16(1).i16(0);
    let mut glyf = W::default();
    let mut loca = W::default();
    for g in glyphs {
        loca.u32(glyf.0.len() as u32);
        match g {
            Tt::Empty => {}
            Tt::Simple => {
                glyf.i16(1).i16(0).i16(0).i16(0).i16(0).u16(0).u16(0);
                glyf.0.push(0x31);
            }
            Tt::Composite(parts) => {
                glyf.i16(-1).i16(0).i16(0).i16(0).i16(0);
                for (k, gid) in parts.iter().enumerate() {
                    let more = if k + 1 < parts.len() { 0x0020 } else { 0 };
                    glyf.u16(0x0003 | more).u16(*gid).i16(0).i16(0);
                }
            }
        }
        while glyf.0.len() % 4 != 0 {
            glyf.0.push(0);
        }
    }
    loca.u32(glyf.0.len() as u32);
    vec![(*b"head", head.0), maxp(glyphs.len() as u16), (*b"loca", loca.0), (*b"glyf", glyf.0)]
}

/// A format 12 `cmap` of `(code point, glyph id)` pairs.
pub(crate) fn cmap(mut map: Vec<(u32, u32)>) -> ([u8; 4], Vec<u8>) {
    map.sort_unstable();
    let mut w = W::default();
    w.u16(0).u16(1).u16(3).u16(10).u32(12);
    w.u16(12).u16(0).u32(16 + 12 * map.len() as u32).u32(0).u32(map.len() as u32);
    for (u, g) in map {
        w.u32(u).u32(u).u32(g);
    }
    (*b"cmap", w.0)
}

/// A version 2 `post` naming every glyph.
pub(crate) fn post(names: &[&str]) -> ([u8; 4], Vec<u8>) {
    let mut w = W::default();
    w.u32(0x0002_0000).u32(0).i16(0).i16(0).u32(0).u32(0).u32(0).u32(0).u32(0);
    w.u16(names.len() as u16);
    for i in 0..names.len() {
        w.u16(258 + i as u16);
    }
    for n in names {
        w.0.push(n.len() as u8);
        w.0.extend(n.as_bytes());
    }
    (*b"post", w.0)
}

fn binary(bytes: &[u8], id: &str, config: &Config) -> LayerReport {
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    audit_binary(&ctx, &[spec(id)], config)
}

/// .notdef, l, caroncomb, quoteright, caroncomb.case, caronvert, then the four caron letters.
fn caron_font(lcaron: Tt) -> Vec<u8> {
    let glyphs = [
        Tt::Simple,
        Tt::Simple,
        Tt::Simple,
        Tt::Simple,
        Tt::Simple,
        Tt::Simple,
        lcaron,
        Tt::Composite(vec![1, 4]),
        Tt::Composite(vec![1, 3]),
        Tt::Simple,
    ];
    let names = [".notdef", "l", "caroncomb", "quoteright", "caroncomb.case", "caronvert", "lcaron", "Lcaron", "dcaron", "tcaron"];
    let mut tables = outlines(&glyphs);
    tables.push(cmap(vec![(0x6C, 1), (0x30C, 2), (0x2019, 3), (0x13E, 6), (0x13D, 7), (0x10F, 8), (0x165, 9)]));
    tables.push(post(&names));
    sfnt(tables)
}

#[test]
fn caron_letters_are_checked_for_their_accent() {
    let r = binary(&caron_font(Tt::Composite(vec![1, 2])), "check.components.caronAlternate", &Config::default());
    let got: Vec<(&str, &str, Severity)> = r.findings.iter().map(|f| (f.location.glyph.as_str(), f.code, f.severity)).collect();
    assert_eq!(
        got,
        vec![
            ("lcaron", "wrong-mark", Severity::Fail),
            ("Lcaron", "wrong-mark", Severity::Fail),
            ("dcaron", "bad-mark", Severity::Warn),
            ("tcaron", "decomposed-outline", Severity::Warn),
        ]
    );
    let vertical = binary(&caron_font(Tt::Composite(vec![1, 5])), "check.components.caronAlternate", &Config::default());
    assert!(vertical.findings.iter().all(|f| f.location.glyph != "lcaron"), "a dedicated vertical caron passes");
    let single = binary(&caron_font(Tt::Composite(vec![1])), "check.components.caronAlternate", &Config::default());
    assert_eq!(single.findings[0].code, "single-component");
}

#[test]
fn few_composites_counts_outlines_as_the_reference_check_does() {
    // .notdef, A (one character: not counted), b.sc, c.sc, d.sc as outlines, e as a composite: 4 of 6.
    let mut tables = outlines(&[Tt::Empty, Tt::Simple, Tt::Simple, Tt::Simple, Tt::Simple, Tt::Composite(vec![2])]);
    tables.push(post(&[".notdef", "A", "b.sc", "c.sc", "d.sc", "e"]));
    let bytes = sfnt(tables);
    let r = binary(&bytes, "check.components.fewComposites", &Config::default());
    assert_eq!(r.findings.len(), 1, "{:#?}", r.findings);
    assert_eq!(r.findings[0].data.get("percent"), Some(&67.0));
    let mut config = Config::default();
    config.set("check.components.fewComposites", "maxOutlinePercent", 70.0).unwrap();
    assert!(binary(&bytes, "check.components.fewComposites", &config).findings.is_empty());

    let mut tables = outlines(&[Tt::Empty, Tt::Simple, Tt::Composite(vec![1]), Tt::Composite(vec![1])]);
    // Names of more than one character, so every simple glyph counts: exactly half is not reported.
    tables.push(post(&[".notdef", "b.alt", "c.comp", "d.comp"]));
    assert!(binary(&sfnt(tables), "check.components.fewComposites", &Config::default()).findings.is_empty(), "2 of 4 is not more than half");
}

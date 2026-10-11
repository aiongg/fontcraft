//! Whole-table tests: build a font, read it back with read-fonts, and compare.

use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{Fixed, GlyphId};

use super::*;

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    M(i32, i32),
    L(i32, i32),
    C(i32, i32, i32, i32, i32, i32),
    Z,
}

#[derive(Default)]
struct Capture(Vec<Seg>);

fn i(v: Fixed) -> i32 {
    v.to_i32()
}

impl CommandSink for Capture {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(Seg::M(i(x), i(y)));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(Seg::L(i(x), i(y)));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, e: Fixed, f: Fixed) {
        self.0.push(Seg::C(i(a), i(b), i(c), i(d), i(e), i(f)));
    }
    fn close(&mut self) {
        self.0.push(Seg::Z);
    }
}

/// What read-fonts should draw for `path`: the same segments with a close after each contour.
fn expected(path: &[PathOp]) -> Vec<Seg> {
    let r = |v: f64| ot_round(v) as i32;
    let mut out = Vec::new();
    for op in path {
        match *op {
            PathOp::MoveTo(p) => {
                if !out.is_empty() {
                    out.push(Seg::Z);
                }
                out.push(Seg::M(r(p.0), r(p.1)));
            }
            PathOp::LineTo(p) => out.push(Seg::L(r(p.0), r(p.1))),
            PathOp::CurveTo(a, b, c) => out.push(Seg::C(r(a.0), r(a.1), r(b.0), r(b.1), r(c.0), r(c.1))),
        }
    }
    if !out.is_empty() {
        out.push(Seg::Z);
    }
    out
}

fn glyph(name: &str, advance: f64, path: Vec<PathOp>) -> Glyph {
    Glyph { name: name.to_string(), advance, path, hints: None }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<PathOp> {
    vec![PathOp::MoveTo((x0, y0)), PathOp::LineTo((x1, y0)), PathOp::LineTo((x1, y1)), PathOp::LineTo((x0, y1))]
}

fn sample_font() -> Font {
    let o = vec![
        PathOp::MoveTo((250.0, -10.0)),
        PathOp::CurveTo((390.0, -10.0), (480.0, 100.0), (480.0, 260.0)),
        PathOp::CurveTo((480.0, 420.0), (390.0, 530.0), (250.0, 530.0)),
        PathOp::CurveTo((110.0, 530.0), (20.0, 420.0), (20.0, 260.0)),
        PathOp::CurveTo((20.0, 100.0), (110.0, -10.0), (250.0, -10.0)),
        PathOp::MoveTo((250.0, 60.0)),
        PathOp::CurveTo((160.0, 60.0), (100.0, 140.0), (100.0, 260.0)),
        PathOp::CurveTo((100.0, 380.0), (160.0, 460.0), (250.0, 460.0)),
        PathOp::CurveTo((340.0, 460.0), (400.0, 380.0), (400.0, 260.0)),
        PathOp::CurveTo((400.0, 140.0), (340.0, 60.0), (250.0, 60.0)),
    ];
    let mut slash = vec![PathOp::MoveTo((10.4, -100.6)), PathOp::LineTo((90.0, -100.0))];
    slash.extend([PathOp::LineTo((390.0, 800.0)), PathOp::LineTo((310.5, 800.0))]);
    Font {
        name: "Sample-Regular".into(),
        top: TopDict {
            version: Some("1.0".into()),
            notice: Some(String::new()),
            copyright: Some("Copyright 2026 Sample".into()),
            full_name: Some("Sample Regular".into()),
            family_name: Some("Sample".into()),
            weight: Some("Regular".into()),
            italic_angle: Number::Real(0.0),
            underline_position: Number::Int(-75),
            ..TopDict::default()
        },
        private: PrivateDict { blue_values: vec![Number::Int(-10), Number::Int(0), Number::Int(520), Number::Int(530)], ..PrivateDict::default() },
        glyphs: vec![
            glyph(".notdef", 500.0, rect(50.0, 0.0, 450.0, 700.0)),
            glyph("space", 250.0, vec![]),
            glyph("o", 500.0, o),
            glyph("slash", 400.0, slash),
            glyph("uni0434", 612.4, rect(0.0, 0.0, 600.0, 500.0)),
        ],
        ..Font::default()
    }
}

#[test]
fn a_whole_font_reads_back_with_read_fonts() {
    let font = sample_font();
    let built = build(&font).unwrap();
    let cff = CffFontRef::new_cff(&built.table, 0, None).unwrap();
    assert_eq!(cff.num_glyphs(), font.glyphs.len() as u32);
    let subfont = cff.subfont(0, &[]).unwrap();
    for (gid, g) in font.glyphs.iter().enumerate() {
        let mut sink = Capture::default();
        let width = cff.evaluate_charstring(&subfont, GlyphId::new(gid as u32), &[], &mut sink).unwrap();
        assert_eq!(sink.0, expected(&g.path), "outline of {}", g.name);
        let width = width.map(|w| w.to_f64()).unwrap_or(f64::NAN);
        assert_eq!(width, ot_round(g.advance), "width of {}", g.name);
    }

    // Glyph names through the charset and the strings.
    let charset = cff.charset().unwrap();
    for (gid, g) in font.glyphs.iter().enumerate() {
        let sid = charset.string_id(GlyphId::new(gid as u32)).unwrap();
        assert_eq!(cff.string(sid).unwrap(), g.name.as_bytes());
    }

    // Top DICT values.
    let meta = cff.metadata().unwrap();
    assert_eq!(meta.name(), Some("Sample-Regular"));
    assert_eq!(meta.full_name(), Some("Sample Regular"));
    assert_eq!(meta.family_name(), Some("Sample"));
    assert_eq!(meta.weight(), Some("Regular"));
    assert_eq!(meta.underline_position().to_i32(), -75);
    let b = meta.bbox();
    assert_eq!([b.x_min, b.y_min, b.x_max, b.y_max].map(Fixed::to_i32), [0, -101, 600, 800]);
    assert_eq!(built.font_bbox, [0, -101, 600, 800]);
    assert_eq!(built.glyph_bounds[1], None);
}

#[test]
fn widths_use_the_optimised_pair() {
    let built = build(&sample_font()).unwrap();
    // Widths 500, 250, 500, 400, 612: the reference picks default 500 and nominal 293.
    assert_eq!((built.default_width_x, built.nominal_width_x), (500, 293));
    let fixed = Font { widths: Widths::Fixed { default_width_x: 0, nominal_width_x: 0 }, ..sample_font() };
    let built = build(&fixed).unwrap();
    let cff = CffFontRef::new_cff(&built.table, 0, None).unwrap();
    let subfont = cff.subfont(0, &[]).unwrap();
    let w = cff.evaluate_charstring(&subfont, GlyphId::new(1), &[], &mut Capture::default()).unwrap();
    assert_eq!(w.map(Fixed::to_i32), Some(250));
}

#[test]
fn building_is_deterministic() {
    assert_eq!(build(&sample_font()).unwrap(), build(&sample_font()).unwrap());
}

#[test]
fn bad_input_is_refused() {
    let base = sample_font();
    let mut f = base.clone();
    f.glyphs.clear();
    assert_eq!(build(&f), Err(Error::NoGlyphs));
    let mut f = base.clone();
    f.glyphs.swap(0, 1);
    assert!(matches!(build(&f), Err(Error::FirstGlyphNotNotdef(_))));
    let mut f = base.clone();
    f.glyphs[2].name = "space".into();
    assert!(matches!(build(&f), Err(Error::DuplicateGlyphName(_))));
    let mut f = base.clone();
    f.glyphs[2].path.push(PathOp::LineTo((f64::NAN, 0.0)));
    assert!(matches!(build(&f), Err(Error::NotFinite { .. })));
    let mut f = base.clone();
    f.glyphs[2].advance = f64::INFINITY;
    assert!(matches!(build(&f), Err(Error::NotFinite { .. })));
    let mut f = base.clone();
    f.top.weight = Some("Négrita".into());
    assert!(matches!(build(&f), Err(Error::BadCharacter { .. })));
    let mut f = base.clone();
    f.top.full_name = Some("名前".into());
    assert!(matches!(build(&f), Err(Error::BadCharacter { .. })));
    let mut f = base.clone();
    f.top.italic_angle = Number::Real(f64::NAN);
    assert!(matches!(build(&f), Err(Error::NotFinite { .. })));
    let mut f = base.clone();
    f.round_tolerance = -1.0;
    assert!(matches!(build(&f), Err(Error::BadRoundTolerance(_))));
    let mut f = base.clone();
    f.glyphs[2].path = rect(-30_000.0, 0.0, 30_000.0, 10.0);
    assert!(matches!(build(&f), Err(Error::OutOfRange { .. })), "a 60,000-unit line does not fit a charstring number");
    let mut f = base;
    f.glyphs[2].path = vec![PathOp::LineTo((1.0, 1.0)); MAX_SEGMENTS + 1];
    assert!(matches!(build(&f), Err(Error::TooManySegments { .. })));
}

#[test]
fn many_glyphs_use_a_ranged_charset_and_wider_offsets() {
    let mut glyphs = vec![glyph(".notdef", 500.0, rect(0.0, 0.0, 500.0, 700.0))];
    for k in 0..5000 {
        let x = f64::from(k % 400);
        glyphs.push(glyph(&format!("g{k:04}"), 300.0 + f64::from(k % 7) * 50.0, rect(x, 0.0, x + 200.0, 600.0)));
    }
    let font = Font { name: "Many".into(), glyphs, ..Font::default() };
    let built = build(&font).unwrap();
    assert!(built.table.len() > 0x1_0000, "the table needs 3-byte offsets");
    let cff = CffFontRef::new_cff(&built.table, 0, None).unwrap();
    assert_eq!(cff.num_glyphs(), 5001);
    let charset = cff.charset().unwrap();
    let sid = charset.string_id(GlyphId::new(2999)).unwrap();
    assert_eq!(cff.string(sid).unwrap(), b"g2998");
    let subfont = cff.subfont(0, &[]).unwrap();
    let mut sink = Capture::default();
    cff.evaluate_charstring(&subfont, GlyphId::new(2999), &[], &mut sink).unwrap();
    assert_eq!(sink.0, expected(&font.glyphs[2999].path));
}

/// A small deterministic generator for test outlines.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// A delta that is often zero, so every specialised operator form turns up.
    fn step(&mut self) -> f64 {
        match self.next() % 4 {
            0 | 1 => 0.0,
            2 => (self.next() % 600) as f64 - 300.0,
            _ => ((self.next() % 60_000) as f64 - 30_000.0) / 100.0,
        }
    }
}

fn random_glyphs(seed: u64, count: usize) -> Vec<Glyph> {
    let mut rng = Lcg(seed);
    let mut glyphs = vec![glyph(".notdef", 500.0, rect(50.0, 0.0, 450.0, 700.0))];
    for k in 0..count {
        let mut path = Vec::new();
        for _ in 0..rng.next() % 4 {
            let (mut x, mut y) = ((rng.next() % 1000) as f64, (rng.next() % 1000) as f64);
            path.push(PathOp::MoveTo((x, y)));
            for _ in 0..1 + rng.next() % 40 {
                if rng.next().is_multiple_of(3) {
                    x += rng.step();
                    y += rng.step();
                    path.push(PathOp::LineTo((x, y)));
                } else {
                    let mut pts = [(0.0, 0.0); 3];
                    for p in &mut pts {
                        x += rng.step();
                        y += rng.step();
                        *p = (x, y);
                    }
                    path.push(PathOp::CurveTo(pts[0], pts[1], pts[2]));
                }
            }
        }
        glyphs.push(glyph(&format!("r{k}"), (rng.next() % 3000) as f64, path));
    }
    glyphs
}

#[test]
fn random_outlines_read_back_exactly() {
    for seed in 1..=4 {
        let font = Font { name: "Random".into(), glyphs: random_glyphs(seed, 120), ..Font::default() };
        let built = build(&font).unwrap();
        let cff = CffFontRef::new_cff(&built.table, 0, None).unwrap();
        let subfont = cff.subfont(0, &[]).unwrap();
        for (gid, g) in font.glyphs.iter().enumerate() {
            let mut sink = Capture::default();
            let width = cff.evaluate_charstring(&subfont, GlyphId::new(gid as u32), &[], &mut sink).unwrap();
            let (got, want) = (normalise(&sink.0), normalise(&expected(&g.path)));
            if let Some(k) = (0..got.len().max(want.len())).find(|&k| got.get(k) != want.get(k)) {
                panic!(
                    "seed {seed}, {}: segment {k}: read {:?}, drawn {:?}",
                    g.name,
                    got.get(k.saturating_sub(1)..k + 2),
                    want.get(k.saturating_sub(1)..k + 2)
                );
            }
            assert_eq!(width.map(Fixed::to_f64), Some(ot_round(g.advance)), "seed {seed}, {}", g.name);
        }
    }
}

/// Outlines in a form the specialiser does not change: it drops segments that go nowhere, turns
/// a curve whose control points sit on its ends into a line, and sums runs of horizontal (or
/// vertical) lines, as the reference does. Both sides of a comparison go through this.
fn normalise(segs: &[Seg]) -> Vec<Seg> {
    let mut out: Vec<Seg> = Vec::new();
    let mut at = (0, 0);
    // The start and axis (true: horizontal) of the last output segment, when it is an
    // axis-aligned line.
    let mut last_line: Option<((i32, i32), bool)> = None;
    for seg in segs {
        let seg = match *seg {
            Seg::C(a, b, c, d, x, y) if (a, b) == at && (c, d) == (x, y) => Seg::L(x, y),
            ref other => other.clone(),
        };
        match seg {
            Seg::L(x, y) => {
                if (x, y) == at {
                    continue;
                }
                let (horizontal, vertical) = (y == at.1, x == at.0);
                if let Some((start, h)) = last_line
                    && ((h && horizontal) || (!h && vertical))
                {
                    out.pop();
                    at = (x, y);
                    if (x, y) == start {
                        last_line = None;
                    } else {
                        out.push(Seg::L(x, y));
                    }
                    continue;
                }
                last_line = if horizontal || vertical { Some((at, horizontal)) } else { None };
                out.push(Seg::L(x, y));
                at = (x, y);
            }
            Seg::M(x, y) | Seg::C(_, _, _, _, x, y) => {
                last_line = None;
                out.push(seg);
                at = (x, y);
            }
            Seg::Z => {
                last_line = None;
                out.push(Seg::Z);
            }
        }
    }
    out
}

#[test]
fn hostile_numbers_never_panic() {
    let values = [0.0, -0.0, 0.5, -0.5, 1e-300, 32_767.5, -32_768.5, 65_535.0, MAX_COORDINATE, -MAX_COORDINATE, 1e300];
    for &v in &values {
        for &w in &values {
            let path = vec![PathOp::MoveTo((v, w)), PathOp::CurveTo((w, v), (v, v), (w, w)), PathOp::LineTo((-v, -w))];
            let font = Font {
                name: "Hostile".into(),
                glyphs: vec![glyph(".notdef", w, path.clone()), glyph("a", v, path)],
                round_tolerance: 0.0,
                ..Font::default()
            };
            let _ = build(&font);
            let _ = build(&Font { round_tolerance: 0.5, specialise: false, ..font });
        }
    }
}

//! `autohint_cff2` on `CFF2` tables built in code: the masters' outlines read from the blends, and
//! the stems a hinter returns per master written back as blends.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use fontcraft_cff::cff2::{Cff2Font, write_cff2};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{F2Dot14, Fixed, GlyphId};

use super::*;
use crate::sfnt::Flavour;

#[derive(Default)]
struct Record(Vec<String>);

impl CommandSink for Record {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("M {} {}", x.to_f64(), y.to_f64()));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("L {} {}", x.to_f64(), y.to_f64()));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, x: Fixed, y: Fixed) {
        self.0.push(format!("C {} {} {} {} {} {}", a.to_f64(), b.to_f64(), c.to_f64(), d.to_f64(), x.to_f64(), y.to_f64()));
    }
    fn close(&mut self) {}
    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.0.push(format!("hstem {} {}", y.to_f64(), dy.to_f64()));
    }
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.0.push(format!("vstem {} {}", x.to_f64(), dx.to_f64()));
    }
}

/// What glyph `gid` of the `CFF2` table sends at normalized `wght`.
fn drawn(font: &SfntFont, gid: u32, wght: f64) -> Vec<String> {
    let table = font.get(*b"CFF2").unwrap();
    let cff = CffFontRef::new_cff2(table, None).unwrap();
    let coords = [F2Dot14::from_f64(wght)];
    let sub = cff.subfont(0, &coords).unwrap();
    let mut rec = Record::default();
    cff.evaluate_charstring(&sub, GlyphId::new(gid), &coords, &mut rec).unwrap();
    rec.0
}

fn outline(cmds: &[String]) -> Vec<String> {
    cmds.iter().filter(|c| !c.contains("stem")).cloned().collect()
}

fn varying(default: f64, delta: f64) -> Blend {
    Blend { default, deltas: vec![delta] }
}

/// One axis, one region peaking at 1. Glyph 1: a bar from x 100 to 180 at the default and to 240
/// at the peak, 0 to 700 high.
fn font() -> SfntFont {
    let p = |x: Blend, y: f64| (x, Blend::fixed(y));
    let bar = vec![
        BlendOp::MoveTo(p(Blend::fixed(100.0), 0.0)),
        BlendOp::LineTo(p(varying(180.0, 60.0), 0.0)),
        BlendOp::LineTo(p(varying(180.0, 60.0), 700.0)),
        BlendOp::LineTo(p(Blend::fixed(100.0), 700.0)),
    ];
    let cff2 = Cff2Font {
        axis_count: 1,
        regions: vec![vec![RegionAxis { start: 0.0, peak: 1.0, end: 1.0 }]],
        region_lists: vec![vec![0]],
        glyphs: vec![
            Cff2Glyph { name: ".notdef".into(), ..Cff2Glyph::default() },
            Cff2Glyph { name: "l".into(), vsindex: 0, path: bar, hints: None },
        ],
        round_tolerance: 0.01,
        ..Cff2Font::default()
    };
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF2", write_cff2(&cff2).unwrap().table);
    font
}

#[test]
fn autohint_cff2_reads_each_master_and_writes_blended_stems() {
    let font = font();
    let setup = cff2_hint_setup(&font).unwrap();
    assert_eq!(setup.names, [".notdef", "glyph00001"]);
    let mut seen: Vec<Vec<Vec<PathOp>>> = Vec::new();
    let r = autohint_cff2(&font, &setup, &CffAutohintOptions { hint_all: true, round_stems: true }, &|_| true, &mut |g| {
        seen.push(g.paths.to_vec());
        assert_eq!(g.locations, [vec![0.0], vec![1.0]]);
        // A vertical stem on the bar at each master.
        let stem = |x0: f64, x1: f64| CharstringHints { v_stems: vec![StemHint::between(x0, x1)], ..CharstringHints::default() };
        let right = |p: &[PathOp]| match p[1] {
            PathOp::LineTo((x, _)) => x,
            _ => 0.0,
        };
        Ok(Some(g.paths.iter().map(|p| stem(100.0, right(p.as_slice()))).collect()))
    })
    .unwrap();
    assert_eq!(r.hinted, [CffHinted { glyph: "glyph00001".into(), horizontal: 0, vertical: 1 }], "{:?}", r.skipped);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0][0][1], PathOp::LineTo((180.0, 0.0)));
    assert_eq!(seen[0][1][2], PathOp::LineTo((240.0, 700.0)));

    // The stem follows the bar; the outline is drawn as it was at both ends and between them.
    assert!(drawn(&r.font, 1, 0.0).contains(&"vstem 100 180".to_string()), "{:?}", drawn(&r.font, 1, 0.0));
    assert!(drawn(&r.font, 1, 1.0).contains(&"vstem 100 240".to_string()), "{:?}", drawn(&r.font, 1, 1.0));
    for w in [0.0, 0.5, 1.0] {
        assert_eq!(outline(&drawn(&r.font, 1, w)), outline(&drawn(&font, 1, w)), "at {w}");
    }
    // Already hinted glyphs are left alone unless hint_all.
    let again = autohint_cff2(&r.font, &setup, &CffAutohintOptions::default(), &|_| true, &mut |_| Ok(None)).unwrap();
    assert!(again.hinted.is_empty() && again.skipped.iter().any(|s| s.reason.contains("already")), "{:?}", again.skipped);
}

#[test]
fn masters_of_an_intermediate_region_blend_back_to_each_master() {
    // Regions peaking at 1 and at 0.5 (the second inside the first's span): at the first's peak
    // only it counts, at 0.5 both do.
    let axis = |start: f64, peak: f64, end: f64| vec![RegionAxis { start, peak, end }];
    let source = Source {
        store: None,
        axis_count: 1,
        regions: vec![axis(0.0, 1.0, 1.0), axis(0.0, 0.5, 1.0), axis(0.0, 1.0, 1.0)],
        lists: vec![vec![0, 1], vec![0, 2]],
        privates: Vec::new(),
    };
    let m = source.masters(0).unwrap();
    assert_eq!(m.locations, [vec![0.0], vec![1.0], vec![0.5]]);
    let values = [100.0, 240.0, 150.0];
    let b = m.blend(&values).unwrap();
    let mut v = vec![b.default];
    v.extend(&b.deltas);
    for (i, want) in values.iter().enumerate() {
        assert!((m.value_at(&v, i) - want).abs() < 1e-9, "master {i}: {} for {want}", m.value_at(&v, i));
    }
    // Two regions with the same peak cannot be told apart.
    assert!(source.masters(1).unwrap_err().contains("same location"));
    assert!(source.masters(5).is_err());
}

#[test]
fn decode_reads_blends_and_vsindex() {
    // 0 vsindex; 10 20 (deltas 1 2) 2 blend rmoveto; 30 (delta 5) 1 blend hlineto.
    let n = |v: i32| (v + 139) as u8;
    let code = vec![n(0), 15, n(10), n(20), n(1), n(2), n(2), 16, 21, n(30), n(5), n(1), 16, 6];
    let d = decode(&code, 0, &[1]).unwrap();
    assert_eq!(d.path, [Step::Move((vec![10.0, 1.0], vec![20.0, 2.0])), Step::Line((vec![40.0, 6.0], vec![20.0, 2.0]))]);
    assert!(!d.hinted);
    // A vsindex after a drawing operator, a list that does not exist and a varying blend count
    // are refused.
    assert!(decode(&[n(0), n(0), 21, n(0), 15], 0, &[1]).is_err());
    assert!(decode(&[n(3), 15], 0, &[1]).is_err());
    assert!(decode(&[n(1), n(1), n(1), 16, 16], 0, &[1]).is_err());
}

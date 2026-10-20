//! `CFF2` charstrings: specialised byte for byte as fontTools' `specializeCommands` does for a
//! variable merge, and drawn the same at the origin and at the masters once subroutinised.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{F2Dot14, Fixed, GlyphId};

use super::*;
use crate::subr::{SubrOptions, charstrings_of_table, subroutinize_table};

fn p(v: f64) -> Blend {
    Blend::fixed(v)
}

fn b(default: f64, deltas: &[f64]) -> Blend {
    Blend { default, deltas: deltas.to_vec() }
}

fn m(x: Blend, y: Blend) -> BlendOp {
    BlendOp::MoveTo((x, y))
}

fn l(x: Blend, y: Blend) -> BlendOp {
    BlendOp::LineTo((x, y))
}

fn c(x1: Blend, y1: Blend, x2: Blend, y2: Blend, x3: Blend, y3: Blend) -> BlendOp {
    BlendOp::CurveTo((x1, y1), (x2, y2), (x3, y3))
}

fn glyph(name: &str, vsindex: u16, path: Vec<BlendOp>) -> Cff2Glyph {
    Cff2Glyph { name: name.into(), vsindex, path, hints: None }
}

/// The glyphs of the fontTools run, in its order. Region list 0 has one region, list 1 two.
fn sample_glyphs() -> Vec<Cff2Glyph> {
    let mut wave = vec![m(p(0.0), p(0.0))];
    for i in 0..90 {
        let x = 30.0 * f64::from(i);
        let end_y = if i % 3 == 0 { b(0.0, &[5.0]) } else { p(0.0) };
        wave.push(c(p(x + 10.0), p(20.0), p(x + 20.0), p(20.0), p(x + 30.0), end_y));
    }
    vec![
        glyph("box", 0, vec![m(p(0.0), p(0.0)), l(p(500.0), p(0.0)), l(p(500.0), b(700.0, &[50.0])), l(p(0.0), b(700.0, &[50.0]))]),
        glyph(
            "o",
            0,
            vec![
                m(p(250.0), p(0.0)),
                c(b(388.0, &[20.0]), p(0.0), b(500.0, &[40.0]), p(112.0), b(500.0, &[40.0]), p(250.0)),
                c(b(500.0, &[40.0]), p(388.0), b(388.0, &[20.0]), p(500.0), p(250.0), p(500.0)),
                c(p(112.0), p(500.0), p(0.0), p(388.0), p(0.0), p(250.0)),
                c(p(0.0), p(112.0), p(112.0), p(0.0), p(250.0), p(0.0)),
            ],
        ),
        glyph("moves", 0, vec![m(p(10.0), b(10.0, &[5.0])), m(p(20.0), p(30.0)), l(p(120.0), p(30.0)), l(p(120.0), b(130.0, &[10.0]))]),
        glyph(
            "degenerate",
            0,
            vec![
                m(p(0.0), p(0.0)),
                l(p(0.0), p(0.0)),
                l(b(100.0, &[10.0]), p(0.0)),
                l(b(200.0, &[30.0]), p(0.0)),
                c(b(200.0, &[30.0]), p(0.0), p(300.0), p(100.0), p(300.0), p(100.0)),
                l(p(300.0), b(200.0, &[15.0])),
                l(p(0.0), p(200.0)),
            ],
        ),
        glyph("zigzag", 0, vec![m(p(0.0), p(0.0)), l(p(100.0), p(50.0)), l(b(200.0, &[10.0]), p(50.0)), l(p(300.0), p(150.0))]),
        glyph("wave", 0, wave),
        glyph("tri", 1, vec![m(p(0.0), p(0.0)), l(b(100.0, &[10.0, 20.0]), p(0.0)), l(p(50.0), b(80.0, &[5.0, -5.0]))]),
    ]
}

/// Glyphs the specialiser draws with the same segments as the generic form (no merged moves,
/// no dropped or summed lines).
const SAME_TOPOLOGY: [&str; 5] = ["box", "o", "zigzag", "wave", "tri"];

fn font(glyphs: Vec<Cff2Glyph>) -> Cff2Font {
    let mut all = vec![glyph(".notdef", 0, Vec::new())];
    all.extend(glyphs);
    Cff2Font {
        axis_count: 1,
        regions: vec![vec![RegionAxis { start: 0.0, peak: 1.0, end: 1.0 }], vec![RegionAxis { start: 0.0, peak: 0.5, end: 1.0 }]],
        region_lists: vec![vec![0], vec![0, 1]],
        glyphs: all,
        round_tolerance: 0.01,
        ..Cff2Font::default()
    }
}

/// fontTools 4.x: `specializeCommands(commands, generalizeFirst=False, maxstack=513)` on the
/// commands `CFF2CharStringMergePen` makes of [`sample_glyphs`], then `commandsToProgram`, with
/// `vsindex` put first as `varLib.cff.merge_charstrings` puts it, compiled by `T2CharString`.
const WAVE: &str = concat!(
    "8b16959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b957708959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b9577959f958b9577908c10959f868c10958b9577959f958b9577959f958b9577908c10959f868c10958b9577959f95",
    "8b957708",
);

const REFERENCE: [(&str, &str); 7] = [
    ("box", "8b16f888f950bd8c10fc8806"),
    ("o", "f78e16f71ef7049f9f8d10f704f71ef71efb04778c10f704fb1e778c10fb1efb04fb04fb1efb1ef704fb04f71e1f"),
    ("moves", "9fa98b8c1015efef958c1006"),
    ("degenerate", "8b16f75ca98c1006ef6d8c10ef8bef9a8c10fbc08b7c8c1005"),
    ("zigzag", "8b16efbdef958c108bef818c10ef05"),
    ("wave", WAVE),
    ("tri", "8c0f8b16ef959f8c100659db817790868d1005"),
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn charstrings_are_specialised_as_the_reference_specialises_them() {
    let built = write_cff2(&font(sample_glyphs())).unwrap();
    let set = charstrings_of_table(&built.table, true).unwrap();
    for (i, (name, want)) in REFERENCE.iter().enumerate() {
        assert_eq!(hex(&set.charstrings[i + 1]), *want, "glyph {name}");
    }
    // The generic form keeps one operator per segment: `wave` is a move and 90 curves.
    let generic = write_cff2(&Cff2Font { generic: true, ..font(sample_glyphs()) }).unwrap();
    let set = charstrings_of_table(&generic.table, true).unwrap();
    let ops: Vec<u8> = set.charstrings[6].iter().copied().filter(|&b| b == 8 || b == 21).collect();
    assert_eq!(ops.len(), 91);
}

#[derive(Debug, Default, PartialEq)]
struct Segments(Vec<(char, Vec<Fixed>)>);

impl CommandSink for Segments {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(('M', vec![x, y]));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(('L', vec![x, y]));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, e: Fixed, f: Fixed) {
        self.0.push(('C', vec![a, b, c, d, e, f]));
    }
    fn close(&mut self) {
        self.0.push(('Z', Vec::new()));
    }
}

/// Every glyph of a `CFF2` table drawn by read-fonts at `at` on the one axis.
fn draw(table: &[u8], at: f32) -> Vec<Segments> {
    let cff = CffFontRef::new_cff2(table, None).unwrap();
    let coords = [F2Dot14::from_f32(at)];
    let subfont = cff.subfont(0, &coords).unwrap();
    (0..cff.num_glyphs())
        .map(|g| {
            let mut sink = Segments::default();
            cff.evaluate_charstring(&subfont, GlyphId::new(g), &coords, &mut sink).unwrap();
            sink
        })
        .collect()
}

/// `g` moved right by `dx`, deltas kept.
fn shifted(g: &Cff2Glyph, dx: f64, name: String) -> Cff2Glyph {
    let mv = |(x, y): &BlendPoint| (Blend { default: x.default + dx, deltas: x.deltas.clone() }, y.clone());
    let path = g
        .path
        .iter()
        .map(|op| match op {
            BlendOp::MoveTo(a) => BlendOp::MoveTo(mv(a)),
            BlendOp::LineTo(a) => BlendOp::LineTo(mv(a)),
            BlendOp::CurveTo(a, b, c) => BlendOp::CurveTo(mv(a), mv(b), mv(c)),
        })
        .collect();
    Cff2Glyph { name, vsindex: g.vsindex, path, hints: g.hints.clone() }
}

#[test]
fn specialised_and_subroutinised_tables_draw_the_same_and_are_smaller() {
    // Eight copies of each sample glyph, each moved, so their tails repeat across glyphs.
    let samples = sample_glyphs();
    let mut glyphs = Vec::new();
    for k in 0..8 {
        for g in &samples {
            glyphs.push(shifted(g, f64::from(k) * 7.0, format!("{}.{k}", g.name)));
        }
    }
    let same_topology: Vec<bool> =
        std::iter::once(true).chain(glyphs.iter().map(|g| SAME_TOPOLOGY.iter().any(|n| g.name.starts_with(&format!("{n}."))))).collect();
    let font = font(glyphs);
    let generic = write_cff2(&Cff2Font { generic: true, ..font.clone() }).unwrap().table;
    let specialised = write_cff2(&font).unwrap().table;
    let subroutinised = subroutinize_table(&specialised, true, &SubrOptions::default()).unwrap();
    assert!(specialised.len() < generic.len(), "specialised {} generic {}", specialised.len(), generic.len());
    assert!(subroutinised.len() < specialised.len(), "subroutinised {} specialised {}", subroutinised.len(), specialised.len());
    let set = charstrings_of_table(&subroutinised, true).unwrap();
    assert!(!set.global_subrs.is_empty() || set.local_subrs.iter().any(|s| !s.is_empty()));
    // At the origin, at the peak of region 1 alone, and at the master where region 0 is full.
    for at in [0.0, 0.5, 1.0] {
        let (g, s, r) = (draw(&generic, at), draw(&specialised, at), draw(&subroutinised, at));
        assert_eq!(s, r, "subroutinised at {at}");
        for (gid, same) in same_topology.iter().enumerate() {
            if *same {
                assert_eq!(g[gid], s[gid], "glyph {gid} at {at}");
            }
        }
    }
}

#[test]
fn a_blend_run_in_a_charstring_leaves_room_for_a_call() {
    // 255 regions: one blended operand takes 257 slots, two take 2 × 256 + 1 = 513, the whole
    // stack. fontTools grows a run only while it stays below 513, so each of the curve's six
    // varying coordinates gets a blend of its own.
    let regions = 255;
    let v = |d: f64| Blend { default: d, deltas: vec![d / 10.0; regions] };
    let mut f = font(vec![glyph("c", 0, vec![m(p(0.0), p(0.0)), c(v(10.0), v(20.0), v(30.0), v(40.0), v(50.0), v(60.0))])]);
    f.regions = (0..regions).map(|i| vec![RegionAxis { start: 0.0, peak: (i + 1) as f64 / regions as f64, end: 1.0 }]).collect();
    f.region_lists = vec![(0..regions as u16).collect()];
    let built = write_cff2(&f).unwrap();
    let set = charstrings_of_table(&built.table, true).unwrap();
    let blends = set.charstrings[1].iter().filter(|&&b| b == OP_BLEND).count();
    assert_eq!(blends, 6);
    // Subroutinising the table keeps it drawable at the origin and along the axis.
    let sub = subroutinize_table(&built.table, true, &SubrOptions::default()).unwrap();
    for at in [0.0, 0.5, 1.0] {
        assert_eq!(draw(&built.table, at), draw(&sub, at));
    }
}

// --- Hints -----------------------------------------------------------------------------------

fn stem(edge: f64, width: f64) -> StemHint {
    StemHint { edge, width }
}

fn mask(h: &[bool], v: &[bool]) -> Mask {
    Mask { h: h.to_vec(), v: v.to_vec() }
}

/// Two rectangles whose vertical stems overlap, so the hints need two masks; the second master
/// widens both. `widths` are the stems' widths, `x2` the second rectangle's left edge, `h` its
/// height (first master, second master).
fn two_boxes() -> Cff2Glyph {
    let (w, x2, h) = (b(80.0, &[20.0]), b(150.0, &[10.0]), b(400.0, &[20.0]));
    let right = |x: &Blend| b(x.default + w.default, &[x.deltas.first().unwrap_or(&0.0) + w.deltas[0]]);
    glyph(
        "a",
        0,
        vec![
            m(p(100.0), p(0.0)),
            l(right(&p(100.0)), p(0.0)),
            l(right(&p(100.0)), p(700.0)),
            l(p(100.0), p(700.0)),
            m(x2.clone(), p(300.0)),
            l(right(&x2), p(300.0)),
            l(right(&x2), h.clone()),
            l(x2.clone(), h),
        ],
    )
}

/// The static hints of [`two_boxes`] in each master: one horizontal stem, two vertical ones,
/// a mask per rectangle.
fn two_boxes_masters() -> Vec<CharstringHints> {
    let masks = vec![(0, mask(&[true], &[true, false])), (4, mask(&[true], &[false, true]))];
    vec![
        CharstringHints {
            h_stems: vec![stem(300.0, 100.0)],
            v_stems: vec![stem(100.0, 80.0), stem(150.0, 80.0)],
            masks: masks.clone(),
            ..Default::default()
        },
        CharstringHints { h_stems: vec![stem(300.0, 120.0)], v_stems: vec![stem(100.0, 100.0), stem(160.0, 100.0)], masks, ..Default::default() },
    ]
}

/// Blends over one region from the two masters' values.
fn blended(shared: &SharedHints) -> Cff2Hints {
    let blend = |a: f64, b: f64| if a == b { Blend::fixed(a) } else { Blend { default: a, deltas: vec![b - a] } };
    let stems = |list: &[Vec<StemHint>]| -> Vec<BlendStem> {
        list.iter().map(|s| BlendStem { edge: blend(s[0].edge, s[1].edge), width: blend(s[0].width, s[1].width) }).collect()
    };
    Cff2Hints { h_stems: stems(&shared.h_stems), v_stems: stems(&shared.v_stems), masks: shared.masks.clone(), counters: shared.counters.clone() }
}

#[derive(Debug, Default, PartialEq)]
struct Stems {
    h: Vec<(i32, i32)>,
    v: Vec<(i32, i32)>,
    masks: Vec<Vec<u8>>,
}

impl CommandSink for Stems {
    fn move_to(&mut self, _: Fixed, _: Fixed) {}
    fn line_to(&mut self, _: Fixed, _: Fixed) {}
    fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed) {}
    fn close(&mut self) {}
    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.h.push((y.to_i32(), dy.to_i32()));
    }
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.v.push((x.to_i32(), dx.to_i32()));
    }
    fn hint_mask(&mut self, mask: &[u8]) {
        self.masks.push(mask.to_vec());
    }
}

fn stems_of(table: &[u8], gid: u32, at: f32) -> Stems {
    let cff = CffFontRef::new_cff2(table, None).unwrap();
    let coords = [F2Dot14::from_f32(at)];
    let subfont = cff.subfont(0, &coords).unwrap();
    let mut sink = Stems::default();
    cff.evaluate_charstring(&subfont, GlyphId::new(gid), &coords, &mut sink).unwrap();
    sink
}

/// fontTools 4.x `varLib.build` on two static CFF masters holding these hints and outlines
/// (`CFF2CharStringMergePen` with `MergeOutlineExtractor`, then `specializeCommands` for CFF2):
/// `a` is `300 100 20 1 blend hstemhm 100 80 -30 80 20 -10 20 3 blend vstemhm hintmask 0xC0
/// 100 hmoveto … hintmask 0xA0 50 10 1 blend -400 rmoveto …`.
const HINTED_A: &str = "f7c0ef9f8c1012efdb6ddb9f819f8e101713c0ef16db9f8c10f9503b778c100613a0bd958c10fc2415dbef3b9f9f778e1006";
/// `b`: `0 50 20 1 blend hstem 100 hmoveto 80 50 20 1 blend -80 hlineto`, no masks.
const HINTED_B: &str = "8bbd9f8c1001ef16dbbd9f8c103b06";

#[test]
fn hinted_masters_merge_into_blended_stems_and_masks_as_the_reference_merges_them() {
    let shared = shared_hints(&two_boxes_masters(), 0.5).unwrap();
    let mut a = two_boxes();
    a.hints = Some(blended(&shared));
    let tall = b(50.0, &[20.0]);
    let mut bar = glyph("b", 0, vec![m(p(100.0), p(0.0)), l(p(180.0), p(0.0)), l(p(180.0), tall.clone()), l(p(100.0), tall.clone())]);
    bar.hints = Some(Cff2Hints { h_stems: vec![BlendStem { edge: p(0.0), width: tall }], ..Default::default() });
    let f = font(vec![a, bar]);
    let built = write_cff2(&f).unwrap();
    let set = charstrings_of_table(&built.table, true).unwrap();
    assert_eq!(hex(&set.charstrings[1]), HINTED_A);
    assert_eq!(hex(&set.charstrings[2]), HINTED_B);
    // Read back at both masters, also once subroutinised: the stems move, the masks stay.
    let sub = subroutinize_table(&built.table, true, &SubrOptions::default()).unwrap();
    for table in [&built.table, &sub] {
        let at0 = stems_of(table, 1, 0.0);
        assert_eq!((at0.h, at0.v), (vec![(300, 400)], vec![(100, 180), (150, 230)]));
        assert_eq!(at0.masks, vec![vec![0xC0], vec![0xA0]]);
        let at1 = stems_of(table, 1, 1.0);
        assert_eq!((at1.h, at1.v), (vec![(300, 420)], vec![(100, 200), (160, 260)]));
    }
    // A glyph without hints is written as before.
    let unhinted = write_cff2(&font(sample_glyphs())).unwrap();
    let mut with_empty = sample_glyphs();
    with_empty[0].hints = Some(Cff2Hints::default());
    assert_eq!(write_cff2(&font(with_empty)).unwrap().table, unhinted.table);
}

#[test]
fn masters_with_different_hint_structures_share_none() {
    // fontTools refuses the merge (`VarLibCFFHintTypeMergeError`); the caller writes the glyph
    // unhinted instead.
    let base = two_boxes_masters();
    let mut extra = base.clone();
    extra[1].v_stems.push(stem(400.0, 20.0));
    for m in &mut extra[1].masks {
        m.1.v.push(false);
    }
    assert!(shared_hints(&extra, 0.5).is_err());
    let mut masks = base.clone();
    masks[1].masks[1].0 = 5;
    assert!(shared_hints(&masks, 0.5).is_err());
    // Stems that sort into another order in one master would blend two different stems.
    let mut swapped = base.clone();
    swapped[1].v_stems = vec![stem(260.0, 100.0), stem(160.0, 100.0)];
    assert!(shared_hints(&swapped, 0.5).is_err());
    // A ghost in one master and a stem in the other.
    let mut ghost = base;
    ghost[1].h_stems = vec![StemHint::top_ghost(420.0)];
    assert!(shared_hints(&ghost, 0.5).is_err());
}

#[test]
fn a_private_dict_blends_what_the_masters_change_as_the_reference_merges_it() {
    // fontTools `merge_PrivateDicts` of BlueValues [-15 0 500 515] / [-15 0 520 535] and StdHW
    // 50 / 70: `-15 15 500 15 0 0 20 0 4 blend BlueValues 50 20 1 blend StdHW`.
    let private = Cff2Private {
        blue_values: vec![p(-15.0), p(0.0), b(500.0, &[20.0]), b(515.0, &[20.0])],
        std_hw: Some(b(50.0, &[20.0])),
        ..Cff2Private::default()
    };
    assert_eq!(hex(&private_dict(&private, 1).unwrap()), "7c9af8889a8b8b9f8b8f1706bd9f8c170a");
}

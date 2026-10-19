//! CFF2 instances of a small variable font built in code. The expected values come from fontTools
//! 4.66.1 `varLib.instancer.instantiateVariableFont` run on the same `CFF2` and `HVAR` bytes in a
//! font made by `fontBuilder` (same metrics, axis, avar map, STAT and names).

use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::types::{F2Dot14, Fixed, NameId, Tag as WTag};

use super::*;
use crate::instance::tests::{be16, head, hhea, i16s, name, os2, stat};
use crate::instance::{AxisLimit, InstanceOptions, instantiate};
use crate::sfnt::{Flavour, Tag};

const WGHT: Tag = *b"wght";

fn n(v: i32) -> Vec<u8> {
    dict_int(i64::from(v))
}

fn ns(vs: &[i32]) -> Vec<u8> {
    vs.iter().flat_map(|v| n(*v)).collect()
}

fn i5(v: usize) -> Vec<u8> {
    let mut b = vec![29];
    b.extend_from_slice(&(v as i32).to_be_bytes());
    b
}

fn index2(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    write_index(&mut out, items.iter().map(Vec::as_slice), true).unwrap();
    out
}

fn f2(v: f64) -> [u8; 2] {
    ((v * 16384.0).round() as i16).to_be_bytes()
}

/// An item variation store on one axis with 16-bit deltas.
fn ivs(regions: &[(f64, f64, f64)], datas: &[(Vec<u16>, Vec<Vec<i16>>)]) -> Vec<u8> {
    let mut rl = Vec::new();
    be16(&mut rl, 1);
    be16(&mut rl, regions.len() as u16);
    for (s, p, e) in regions {
        rl.extend(f2(*s));
        rl.extend(f2(*p));
        rl.extend(f2(*e));
    }
    let subs: Vec<Vec<u8>> = datas
        .iter()
        .map(|(idx, rows)| {
            let mut d = Vec::new();
            for v in [rows.len() as u16, idx.len() as u16, idx.len() as u16] {
                be16(&mut d, v);
            }
            for i in idx {
                be16(&mut d, *i);
            }
            for r in rows {
                for x in r {
                    d.extend(x.to_be_bytes());
                }
            }
            d
        })
        .collect();
    let head = 8 + 4 * subs.len();
    let mut out = Vec::new();
    be16(&mut out, 1);
    out.extend((head as u32).to_be_bytes());
    be16(&mut out, subs.len() as u16);
    let mut at = head + rl.len();
    for s in &subs {
        out.extend((at as u32).to_be_bytes());
        at += s.len();
    }
    out.extend(rl);
    for s in subs {
        out.extend(s);
    }
    out
}

const REGIONS: [(f64, f64, f64); 2] = [(-1.0, -1.0, 0.0), (0.0, 1.0, 1.0)];

/// The CFF2 table: `.notdef` empty; `A` a blended hstem and vstem, a hint mask, an rmoveto and a
/// global subroutine of three lines (100,0–500,700 at the default; at −1 10 narrower each side
/// and 30 lower, at +1 20 wider each side and 50 higher); `B` (vsindex 1, the +1 region only) a
/// curve whose end rises by 20 at +1. BlueValues −10 10 690 710, the third +50 at +1.
fn cff2_table() -> Vec<u8> {
    let mut gsubr = ns(&[0, 700, 400, -30, 50, -20, 40, 2]);
    gsubr.push(16);
    gsubr.extend(ns(&[0, 0, -700, 30, -50, 1]));
    gsubr.extend([16, 5]);
    let mut a = ns(&[0, 700, -30, 50, 1]);
    a.extend([16, 1]);
    a.extend(ns(&[100, 400, 10, -20, -20, 40, 2]));
    a.extend([16, 3, 19, 0xC0]);
    a.extend(ns(&[100, 10, -20, 1]));
    a.push(16);
    a.extend(ns(&[0]));
    a.push(21);
    a.extend(ns(&[-107]));
    a.push(29);
    let mut b = ns(&[1]);
    b.push(15);
    b.extend(ns(&[50, 0]));
    b.push(21);
    b.extend(ns(&[0, 100, 100, 100, 100, -100, 20, 1]));
    b.extend([16, 8]);
    let mut private = ns(&[-10, 20, 680, 20, 0, 0, 0, 0, -30, 50, 0, 0, 4]);
    private.extend([23, 6]);
    let store = ivs(&REGIONS, &[(vec![0, 1], vec![]), (vec![1], vec![])]);
    let mut vstore = (store.len() as u16).to_be_bytes().to_vec();
    vstore.extend(store);
    let gsubrs = index2(&[gsubr]);
    let cs = index2(&[Vec::new(), a, b]);
    let top_len = 5 * 3 + 1 + 2 + 1;
    let at_vstore = 5 + top_len + gsubrs.len();
    let at_cs = at_vstore + vstore.len();
    let fd = |off: usize| {
        let mut d = i5(private.len());
        d.extend(i5(off));
        d.push(18);
        d
    };
    let at_fd = at_cs + cs.len();
    let at_priv = at_fd + index2(&[fd(0)]).len();
    let fdarray = index2(&[fd(at_priv)]);
    let mut top = i5(at_cs);
    top.push(17);
    top.extend(i5(at_fd));
    top.extend([12, 36]);
    top.extend(i5(at_vstore));
    top.push(24);
    assert_eq!(top.len(), top_len);
    let mut out = vec![2, 0, 5];
    out.extend((top.len() as u16).to_be_bytes());
    for part in [top, gsubrs, vstore, cs, fdarray, private] {
        out.extend(part);
    }
    out
}

/// Advances 500, 600, 500; A −20 at −1 and +40 at +1, B +50 at +1 (direct glyph-id mapping).
fn hvar_table() -> Vec<u8> {
    let store = ivs(&REGIONS, &[(vec![0, 1], vec![vec![0, 0], vec![-20, 40], vec![0, 50]])]);
    let mut out = Vec::new();
    be16(&mut out, 1);
    be16(&mut out, 0);
    for v in [20u32, 0, 0, 0] {
        out.extend(v.to_be_bytes());
    }
    out.extend(store);
    out
}

fn cff2_font() -> SfntFont {
    let mut hmtx = Vec::new();
    for (adv, lsb) in [(500u16, 0u16), (600, 100), (500, 50)] {
        be16(&mut hmtx, adv);
        be16(&mut hmtx, lsb);
    }
    let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
    be16(&mut maxp, 3);
    let axis =
        VariationAxisRecord::new(WTag::new(b"wght"), Fixed::from_f64(100.0), Fixed::from_f64(400.0), Fixed::from_f64(900.0), 0, NameId::new(256));
    let instance =
        InstanceRecord { subfamily_name_id: NameId::new(257), flags: 0, coordinates: vec![Fixed::from_f64(650.0)], post_script_name_id: None };
    let fvar = Fvar::new(AxisInstanceArrays::new(vec![axis], vec![instance]));
    let map = |a: i16, b: i16| AxisValueMap::new(F2Dot14::from_bits(a), F2Dot14::from_bits(b));
    let avar = Avar::new(vec![SegmentMaps::new(vec![map(-16384, -16384), map(0, 0), map(8192, 4096), map(16384, 16384)])]);
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"head", head());
    font.insert(*b"maxp", maxp);
    font.insert(*b"hhea", hhea(3));
    font.insert(*b"hmtx", hmtx);
    font.insert(*b"OS/2", os2());
    font.insert(*b"post", post);
    font.insert(*b"CFF2", cff2_table());
    font.insert(*b"HVAR", hvar_table());
    font.insert(*b"fvar", write_fonts::dump_table(&fvar).unwrap());
    font.insert(*b"avar", write_fonts::dump_table(&avar).unwrap());
    font.insert(*b"name", name(&[(1, "Test"), (2, "Regular"), (256, "Weight"), (257, "Medium")]));
    font.insert(*b"STAT", stat());
    font
}

fn pin(v: f64) -> InstanceOptions {
    InstanceOptions { limits: vec![(WGHT, AxisLimit::Pin(v))], ..InstanceOptions::default() }
}

fn range(min: f64, max: f64) -> InstanceOptions {
    InstanceOptions { limits: vec![(WGHT, AxisLimit::Range { min, default: None, max })], ..InstanceOptions::default() }
}

/// Each command of a glyph: operator and operand defaults.
fn program(t: &Table, gid: usize) -> Vec<(u16, Vec<f64>)> {
    t.glyphs[gid].iter().map(|c| (c.op, c.args.iter().map(|a| a.v).collect())).collect()
}

/// The deltas of each blended operand of one command (empty for a plain one).
fn deltas(t: &Table, gid: usize, cmd: usize) -> Vec<Vec<f64>> {
    t.glyphs[gid][cmd].args.iter().map(|a| a.item.map(|i| t.items[i].deltas.clone()).unwrap_or_default()).collect()
}

fn blue_values(t: &Table) -> Vec<f64> {
    t.privates[0].entries.iter().find(|e| e.op == 6).unwrap().vals.iter().map(|v| v.v).collect()
}

fn advances(font: &SfntFont) -> Vec<u16> {
    super::super::glyf::read_metrics(font.get(*b"hmtx").unwrap(), font.get(*b"hhea").unwrap(), 3).unwrap().advances
}

#[test]
fn cff2_full_instance_between_masters_matches_the_instancer() {
    let out = instantiate(&cff2_font(), &pin(650.0)).unwrap();
    for tag in [b"fvar", b"avar", b"HVAR"] {
        assert!(out.get(*tag).is_none(), "{} is dropped", String::from_utf8_lossy(tag));
    }
    let t = Table::read(out.get(*b"CFF2").unwrap(), 0).unwrap();
    assert!(t.datas.is_empty() && t.regions.is_empty(), "the variation store goes");
    // 650 normalises to 0.5, which avar maps to 0.25: a quarter of the +1 deltas, rounded half
    // to even as Python's round does (12.5 gives 12).
    assert_eq!(
        program(&t, 1),
        vec![(1, vec![0.0, 712.0]), (3, vec![95.0, 410.0]), (19, vec![]), (21, vec![95.0, 0.0]), (5, vec![0.0, 712.0, 410.0, 0.0, 0.0, -712.0])],
        "hints kept, the subroutine inlined"
    );
    assert_eq!(t.glyphs[1][2].mask, vec![0xC0]);
    assert_eq!(program(&t, 2), vec![(21, vec![50.0, 0.0]), (8, vec![0.0, 100.0, 100.0, 100.0, 100.0, -95.0])]);
    assert_eq!(blue_values(&t), vec![-10.0, 20.0, 692.0, 8.0], "−10 10 702 710: the delta goes to the absolute value");
    assert_eq!(advances(&out), vec![500, 610, 512], "HVAR moves the advances; 12.5 rounds to 12");
    assert_eq!(i16s(&out, b"head", &[36, 38, 40, 42]), vec![50, 0, 505, 712]);
    assert_eq!(i16s(&out, b"hhea", &[10, 12, 14, 16]), vec![610, 50, 100, 510], "from the bounds and the side bearings as stored");
    assert_eq!(i16s(&out, b"OS/2", &[2, 4]), vec![541, 650]);
    let bounds = glyph_bounds(out.get(*b"CFF2").unwrap()).unwrap();
    let b = bounds[2].unwrap();
    assert!((b[3] - 143.222_974_807_886_5).abs() < 1e-6, "B's top is the curve's extreme: {b:?}");
}

#[test]
fn cff2_downgrade_writes_a_cid_keyed_cff_table() {
    let out = instantiate(&cff2_font(), &InstanceOptions { downgrade_cff2: true, ..pin(650.0) }).unwrap();
    assert!(out.get(*b"CFF2").is_none());
    let data = out.get(*b"CFF ").unwrap();
    let cff = read_fonts::ps::cff::CffFontRef::new(data, 0, None).unwrap();
    assert!(cff.is_cid());
    // fontTools: defaultWidthX 610, nominalWidthX 500, so A stores no width and B stores 12.
    let mut widths = Vec::new();
    for gid in 0..3u32 {
        let id = read_fonts::types::GlyphId::new(gid);
        let sub = cff.subfont(cff.subfont_index(id).unwrap_or(0), &[]).unwrap();
        let mut sink = BoundsSink::default();
        let w = cff.evaluate_charstring(&sub, id, &[], &mut sink).unwrap();
        widths.push(w.map(|w| w.to_f64()));
    }
    assert_eq!(widths, vec![Some(500.0), Some(610.0), Some(512.0)]);
    let bounds = glyph_bounds(data).unwrap();
    assert_eq!(bounds[1], Some([95.0, 0.0, 505.0, 712.0]));
    assert_eq!(advances(&out), vec![500, 610, 512]);
}

#[test]
fn cff2_partial_instance_rebases_the_blends() {
    // Limited above the default: the −1 region goes from both subtables.
    let up = instantiate(&cff2_font(), &range(400.0, 900.0)).unwrap();
    let t = Table::read(up.get(*b"CFF2").unwrap(), 1).unwrap();
    assert_eq!(t.regions, vec![vec![(0.0, 1.0, 1.0)]]);
    assert_eq!(t.datas, vec![vec![0], vec![0]]);
    assert_eq!(program(&t, 1)[0], (1, vec![0.0, 700.0]));
    assert_eq!(deltas(&t, 1, 0), vec![vec![], vec![50.0]]);
    assert_eq!(deltas(&t, 2, 1), vec![vec![], vec![], vec![], vec![], vec![], vec![20.0]], "B keeps vsindex 1");
    assert_eq!(advances(&up), vec![500, 600, 500]);
    assert_eq!(i16s(&up, b"head", &[36, 38, 40, 42]), vec![50, 0, 500, 700]);

    // Limited to 100–650: the +1 region now peaks at 650 (0.25), its deltas a quarter, rounded.
    let down = instantiate(&cff2_font(), &range(100.0, 650.0)).unwrap();
    let t = Table::read(down.get(*b"CFF2").unwrap(), 1).unwrap();
    assert_eq!(t.regions.len(), 2);
    assert_eq!(deltas(&t, 1, 0), vec![vec![], vec![-30.0, 13.0]]);
    assert_eq!(deltas(&t, 2, 1)[5], vec![5.0]);
    let blues: Vec<Vec<f64>> =
        t.privates[0].entries.iter().find(|e| e.op == 6).unwrap().vals.iter().map(|v| t.items[v.item.unwrap()].deltas.clone()).collect();
    assert_eq!(blues[2], vec![-30.0, 13.0]);

    // Instancing the partial font again gives the full instance of the original.
    for v in [100.0, 300.0, 650.0] {
        let again = instantiate(&down, &pin(v)).unwrap();
        let direct = instantiate(&cff2_font(), &pin(v)).unwrap();
        let (a, b) = (Table::read(again.get(*b"CFF2").unwrap(), 0).unwrap(), Table::read(direct.get(*b"CFF2").unwrap(), 0).unwrap());
        for gid in 1..3 {
            let (pa, pb) = (program(&a, gid), program(&b, gid));
            for ((oa, va), (ob, vb)) in pa.iter().zip(&pb) {
                assert_eq!(oa, ob);
                assert!(va.iter().zip(vb).all(|(x, y)| (x - y).abs() <= 1.0), "glyph {gid} at {v}: {va:?} vs {vb:?}");
            }
        }
        assert!(advances(&again).iter().zip(advances(&direct)).all(|(x, y)| x.abs_diff(y) <= 1));
    }
}

#[test]
fn a_blend_with_too_few_operands_is_refused() {
    let mut font = cff2_font();
    let mut table = cff2_table();
    // B's blend count 1 becomes 9: more values than the stack holds.
    let at = table.windows(3).position(|w| w == [n(20)[0], n(1)[0], 16]).unwrap();
    table[at + 1] = n(9)[0];
    font.insert(*b"CFF2", table);
    let err = instantiate(&font, &pin(650.0)).unwrap_err();
    assert!(matches!(&err, TablesError::Invalid(m) if m.contains("glyph 2") && m.contains("too few")), "{err}");
}

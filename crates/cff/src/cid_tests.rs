//! CID-keyed tables: read back with read-fonts, subroutinised with a local INDEX per sub-font,
//! and refused when the CIDs or sub-fonts cannot be written.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{Fixed, GlyphId};

use super::*;
use crate::{Number, PathOp};

#[derive(Default)]
struct Points(Vec<(i32, i32)>);

impl CommandSink for Points {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push((x.to_i32(), y.to_i32()));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push((x.to_i32(), y.to_i32()));
    }
    fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, x: Fixed, y: Fixed) {
        self.0.push((x.to_i32(), y.to_i32()));
    }
    fn close(&mut self) {}
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<PathOp> {
    vec![PathOp::MoveTo((x0, y0)), PathOp::LineTo((x1, y0)), PathOp::LineTo((x1, y1)), PathOp::LineTo((x0, y1))]
}

fn bars(count: usize, x: f64) -> Vec<PathOp> {
    (0..count).flat_map(|i| rect(x + 120.0 * i as f64, 0.0, x + 120.0 * i as f64 + 80.0, 700.0)).collect()
}

fn glyph(cid: u32, fd: usize, advance: f64, path: Vec<PathOp>) -> CidGlyph {
    CidGlyph { cid, fd, glyph: Glyph { name: format!("cid{cid:05}"), advance, path, hints: None } }
}

fn private(blues: [i64; 4]) -> PrivateDict {
    PrivateDict { blue_values: blues.map(Number::Int).to_vec(), std_hw: Some(Number::Int(50)), ..PrivateDict::default() }
}

/// Three sub-fonts; CIDs with gaps, as the synthetic source of the measure has them.
fn sample() -> CidFont {
    CidFont {
        name: "CidTest-Regular".into(),
        top: TopDict { version: Some("1.0".into()), family_name: Some("CidTest".into()), ..TopDict::default() },
        ros: Ros { registry: "Test".into(), ordering: "Latin1".into(), supplement: 2 },
        cid_count: None,
        font_dicts: vec![
            FontDict { name: "CidTest-Regular-Alphabetic".into(), private: private([-10, 0, 700, 710]), widths: Widths::Optimise },
            FontDict { name: "CidTest-Regular-Lower".into(), private: private([-12, 0, 500, 512]), widths: Widths::Optimise },
            FontDict { name: "CidTest-Regular-Kana".into(), private: private([-20, 0, 800, 820]), widths: Widths::Optimise },
        ],
        glyphs: vec![
            glyph(0, 0, 500.0, rect(50.0, 0.0, 450.0, 700.0)),
            glyph(1, 0, 250.0, Vec::new()),
            glyph(2, 0, 600.0, bars(2, 50.0)),
            glyph(3, 0, 600.0, bars(3, 40.0)),
            glyph(20, 1, 520.0, bars(2, 60.0)),
            glyph(21, 1, 540.0, bars(1, 40.0)),
            glyph(30, 2, 1000.0, bars(4, 100.0)),
        ],
        round_tolerance: 0.5,
        specialise: true,
    }
}

#[test]
fn a_cid_keyed_font_reads_back_with_its_cids_sub_fonts_and_widths() {
    let font = sample();
    let built = build(&font).unwrap();
    let cff = CffFontRef::new_cff(&built.table, 0, None).unwrap();
    assert!(cff.is_cid());
    assert_eq!(cff.num_glyphs(), 7);
    assert_eq!(cff.num_subfonts(), 3);
    let charset = cff.charset().unwrap();
    for (gid, g) in font.glyphs.iter().enumerate() {
        let id = GlyphId::new(gid as u32);
        assert_eq!(u32::from(charset.string_id(id).unwrap().to_u16()), g.cid, "CID of glyph {gid}");
        let fd = cff.subfont_index(id).unwrap();
        assert_eq!(usize::from(fd), g.fd, "sub-font of glyph {gid}");
        let sub = cff.subfont(fd, &[]).unwrap();
        let mut points = Points::default();
        let width = cff.evaluate_charstring(&sub, id, &[], &mut points).unwrap();
        assert_eq!(width.map(|w| f64::from(w.to_i32())), Some(g.glyph.advance), "width of glyph {gid}");
        let ends = g.glyph.path.iter().filter(|op| !matches!(op, PathOp::CurveTo(..))).count();
        assert_eq!(points.0.len(), ends, "points of glyph {gid}");
    }
    // Each sub-font chose its own widths: the Kana sub-font's one glyph is its default width.
    assert_eq!(built.widths[2].0, 1000);
    assert_eq!(built.widths.len(), 3);
    // The ROS is the first Top DICT entry; CIDCount is the highest CID plus one.
    let ops = top_dict_ops(&built.table);
    assert_eq!(ops[0].0, 1230, "ROS first: {ops:?}");
    assert_eq!(ops[0].1[2], 2, "the supplement");
    let count = ops.iter().find(|(op, _)| *op == 1234).expect("CIDCount");
    assert_eq!(count.1, [31]);
    for op in [15, 1236, 1237, 17] {
        assert!(ops.iter().any(|(o, _)| *o == op), "operator {op} is written");
    }
    assert_eq!(built.font_bbox, [40, 0, 540, 700]);
    assert_eq!(build(&font).unwrap(), built, "building is deterministic");
}

/// The INDEX at `at`: its items and where it ends.
fn index_at(data: &[u8], at: usize) -> (Vec<&[u8]>, usize) {
    let count = usize::from(u16::from_be_bytes([data[at], data[at + 1]]));
    if count == 0 {
        return (Vec::new(), at + 2);
    }
    let size = usize::from(data[at + 2]);
    let off = |i: usize| data[at + 3 + i * size..at + 3 + (i + 1) * size].iter().fold(0usize, |a, b| a << 8 | usize::from(*b));
    let base = at + 2 + (count + 1) * size;
    let items = (0..count).map(|i| &data[base + off(i)..base + off(i + 1)]).collect();
    (items, base + off(count))
}

/// The Top DICT's operators (an escaped one as 1200 + its second byte) with their integer operands.
fn top_dict_ops(table: &[u8]) -> Vec<(u16, Vec<i64>)> {
    let (_, names_end) = index_at(table, usize::from(table[2]));
    let (tops, _) = index_at(table, names_end);
    let d = tops[0];
    let (mut out, mut operands, mut i) = (Vec::new(), Vec::new(), 0);
    while i < d.len() {
        let b = d[i];
        match b {
            12 => {
                out.push((1200 + u16::from(d[i + 1]), std::mem::take(&mut operands)));
                i += 2;
            }
            0..=21 => {
                out.push((u16::from(b), std::mem::take(&mut operands)));
                i += 1;
            }
            28 => {
                operands.push(i64::from(i16::from_be_bytes([d[i + 1], d[i + 2]])));
                i += 3;
            }
            29 => {
                operands.push(i64::from(i32::from_be_bytes([d[i + 1], d[i + 2], d[i + 3], d[i + 4]])));
                i += 5;
            }
            30 => {
                i += 1;
                while d[i] & 0x0F != 0x0F && d[i] >> 4 != 0x0F {
                    i += 1;
                }
                i += 1;
                operands.push(0);
            }
            32..=246 => {
                operands.push(i64::from(b) - 139);
                i += 1;
            }
            247..=250 => {
                operands.push((i64::from(b) - 247) * 256 + i64::from(d[i + 1]) + 108);
                i += 2;
            }
            _ => {
                operands.push(-(i64::from(b) - 251) * 256 - i64::from(d[i + 1]) - 108);
                i += 2;
            }
        }
    }
    out
}

#[test]
fn fd_select_takes_the_smaller_format() {
    let mut font = sample();
    // Seven glyphs in three runs: format 0 (8 bytes) beats format 3 (14 bytes).
    assert_eq!(fd_select(&font).unwrap()[0], 0);
    // 300 glyphs in two runs: format 3 (11 bytes) beats format 0 (301 bytes).
    font.glyphs = (0..300).map(|cid| glyph(cid, usize::from(cid >= 150), 500.0, Vec::new())).collect();
    let f = fd_select(&font).unwrap();
    assert_eq!(f, [3, 0, 2, 0, 0, 0, 0, 150, 1, 1, 44]);
}

#[test]
fn local_subroutines_stay_within_their_sub_font() {
    use crate::subr::{SubrOptions, charstrings_of_table, desubroutinize_table, subroutinize_table};
    let mut font = sample();
    // Many glyphs per sub-font that repeat the same bars, so both global and local subroutines
    // pay for themselves.
    font.glyphs = (0..60u32).map(|cid| glyph(cid, (cid as usize) % 3, 600.0, bars(1 + (cid as usize) % 4, 30.0 + f64::from(cid % 3)))).collect();
    let built = build(&font).unwrap();
    let subr = subroutinize_table(&built.table, false, &SubrOptions::default()).unwrap();
    assert!(subr.len() < built.table.len(), "subroutines make the table smaller");
    let set = charstrings_of_table(&subr, false).unwrap();
    assert_eq!(set.local_subrs.len(), 3, "one local INDEX slot per sub-font");
    assert_eq!(set.fd_select.len(), 60);
    for (gid, fd) in set.fd_select.iter().enumerate() {
        assert_eq!(usize::from(*fd), gid % 3);
    }
    let flat = desubroutinize_table(&subr, false).unwrap();
    assert_eq!(charstrings_of_table(&flat, false).unwrap().charstrings, charstrings_of_table(&built.table, false).unwrap().charstrings);
    // The subroutinised table still draws every glyph with its sub-font's subroutines.
    let cff = CffFontRef::new_cff(&subr, 0, None).unwrap();
    let plain = CffFontRef::new_cff(&built.table, 0, None).unwrap();
    for gid in 0..60u32 {
        let id = GlyphId::new(gid);
        let draw = |c: &CffFontRef| {
            let sub = c.subfont(c.subfont_index(id).unwrap(), &[]).unwrap();
            let mut p = Points::default();
            c.evaluate_charstring(&sub, id, &[], &mut p).unwrap();
            p.0
        };
        assert_eq!(draw(&cff), draw(&plain), "glyph {gid}");
    }
}

#[test]
fn cids_and_sub_fonts_that_cannot_be_written_are_refused() {
    let refused = |f: CidFont| build(&f).unwrap_err().to_string();
    let mut f = sample();
    f.glyphs[0].cid = 5;
    assert!(refused(f).contains("CID 0"));
    let mut f = sample();
    f.glyphs[3].cid = 2;
    assert!(refused(f).contains("does not follow"));
    let mut f = sample();
    f.glyphs[4].fd = 3;
    assert!(refused(f).contains("does not exist"));
    let mut f = sample();
    f.font_dicts.clear();
    assert!(refused(f).contains("sub-fonts"));
    let mut f = sample();
    f.font_dicts = (0..257).map(|i| FontDict { name: format!("FD{i}"), private: PrivateDict::default(), widths: Widths::Optimise }).collect();
    assert!(refused(f).contains("sub-fonts"));
    let mut f = sample();
    f.glyphs[6].cid = MAX_CID + 1;
    assert!(refused(f).contains("highest CID"));
    let mut f = sample();
    f.cid_count = Some(30);
    assert!(refused(f).contains("CIDCount"));
    let mut f = sample();
    f.font_dicts[1].name = f.font_dicts[0].name.clone();
    assert!(refused(f).contains("two sub-fonts"));
    let mut f = sample();
    f.ros.registry = "Tést".into();
    assert!(build(&f).is_err());
    let mut f = sample();
    f.glyphs[2].glyph.advance = f64::NAN;
    assert!(refused(f).contains("cid00002"));
    let mut f = sample();
    f.font_dicts[0].private.blue_scale = Number::Real(f64::INFINITY);
    assert!(build(&f).is_err());
    let mut f = sample();
    f.glyphs.clear();
    assert_eq!(build(&f).unwrap_err(), Error::NoGlyphs);
}

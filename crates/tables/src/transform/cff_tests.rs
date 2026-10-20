//! Units per em on CFF and CFF2 fonts built here. The expected charstrings, DICT values and
//! metrics are what fontTools 4.66.1's `ttLib.scaleUpem` wrote for the same table bytes (the
//! oracle font had a `cmap` and a version 2 `post` besides; neither changes these values).

use super::{numbers, scale_upem_within};
use crate::TablesError;
use crate::dehint::cff::Table;
use crate::instance::cff2::{dict_number, put_cs_number};
use crate::sfnt::{Flavour, SfntFont};
use crate::transform::{TransformOptions, transform, transform_with_report};

const OPS: [(&str, &[u8]); 26] = [
    ("hstem", &[1]),
    ("vmoveto", &[4]),
    ("hlineto", &[6]),
    ("vlineto", &[7]),
    ("vstem", &[3]),
    ("rlineto", &[5]),
    ("callsubr", &[10]),
    ("return", &[11]),
    ("endchar", &[14]),
    ("vsindex", &[15]),
    ("blend", &[16]),
    ("hstemhm", &[18]),
    ("hintmask", &[19]),
    ("cntrmask", &[20]),
    ("rmoveto", &[21]),
    ("hmoveto", &[22]),
    ("vstemhm", &[23]),
    ("rrcurveto", &[8]),
    ("vhcurveto", &[30]),
    ("hvcurveto", &[31]),
    ("callgsubr", &[29]),
    ("hflex", &[12, 34]),
    ("flex", &[12, 35]),
    ("hflex1", &[12, 36]),
    ("flex1", &[12, 37]),
    ("add", &[12, 10]),
];

/// A charstring from text: integers, operator names and `<hex>` mask bytes.
pub(crate) fn cs(program: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for word in program.split_whitespace() {
        if let Ok(n) = word.parse::<i32>() {
            put_cs_number(&mut out, f64::from(n)).unwrap();
        } else if let Some(hex) = word.strip_prefix('<').and_then(|w| w.strip_suffix('>')) {
            for k in (0..hex.len()).step_by(2) {
                out.push(u8::from_str_radix(hex.get(k..k + 2).unwrap(), 16).unwrap());
            }
        } else {
            let (_, bytes) = OPS.iter().find(|(name, _)| *name == word).unwrap_or_else(|| panic!("unknown operator {word}"));
            out.extend_from_slice(bytes);
        }
    }
    out
}

pub(crate) fn index(items: &[Vec<u8>], cff2: bool) -> Vec<u8> {
    let mut out = if cff2 { (items.len() as u32).to_be_bytes().to_vec() } else { (items.len() as u16).to_be_bytes().to_vec() };
    if items.is_empty() {
        return out;
    }
    let mut offsets = vec![1usize];
    for it in items {
        offsets.push(offsets.last().unwrap() + it.len());
    }
    let last = *offsets.last().unwrap();
    let size = if last < 0x100 {
        1
    } else if last < 0x1_0000 {
        2
    } else {
        3
    };
    out.push(size as u8);
    for v in offsets {
        out.extend_from_slice(&(v as u32).to_be_bytes()[4 - size..]);
    }
    for it in items {
        out.extend_from_slice(it);
    }
    out
}

pub(crate) fn di(v: i32) -> Vec<u8> {
    let mut b = vec![29];
    b.extend_from_slice(&v.to_be_bytes());
    b
}

pub(crate) const CFF_GLYPHS: [&str; 4] = [
    "300 endchar",
    "600 0 100 hstem 51 81 vstem 11 21 rmoveto hintmask <c0> 101 0 rlineto 0 101 rlineto -101 0 rlineto endchar",
    "-107 callsubr 31 41 hintmask <c0> 1 -1 rmoveto 11 0 11 11 11 0 11 0 11 -11 11 0 50 flex -107 callgsubr endchar",
    "701 51 hmoveto 21 31 11 21 31 21 31 hflex 0 -11 rlineto endchar",
];

/// A name-keyed CFF table: one global and one local subroutine, stems and hint masks, flex, a
/// Private DICT with blue zones, a stem snap, StdVW, BlueFuzz and both width defaults, a Top DICT
/// with UnderlinePosition and a FontBBox, and optionally a FontMatrix.
pub(crate) fn cff_table(glyphs: &[&str], matrix: Option<[f64; 6]>) -> Vec<u8> {
    let glyphs: Vec<Vec<u8>> = glyphs.iter().map(|p| cs(p)).collect();
    let gsubrs = index(&[cs("0 101 rlineto return")], false);
    let lsubrs = index(&[cs("0 51 hstemhm return")], false);
    let names = index(&[b"Test".to_vec()], false);
    let strings = index(&[], false);
    let charstrings = index(&glyphs, false);
    let mut private: Vec<u8> = [-11, 11, 491, 10].into_iter().flat_map(di).collect();
    private.push(6);
    private.extend(di(81));
    private.push(11);
    private.extend(di(41).into_iter().chain(di(22)).chain([12, 12]));
    private.extend(di(1).into_iter().chain([12, 11]));
    private.extend(di(501).into_iter().chain([20]).chain(di(37)).chain([21]));
    let subrs_at = private.len() + 6;
    private.extend(di(subrs_at as i32).into_iter().chain([19]));

    let mut head: Vec<u8> = di(-75).into_iter().chain([12, 3]).collect();
    head.extend([0, -11, 701, 501].into_iter().flat_map(di).chain([5]));
    if let Some(m) = matrix {
        head.extend(m.iter().flat_map(|v| dict_number(*v).unwrap()).chain([12, 7]));
    }
    head.extend(di(0).into_iter().chain([15]));
    let top_len = head.len() + 6 + 11;
    let top_index_len = 2 + 1 + 2 + top_len;
    let cs_at = 4 + names.len() + top_index_len + strings.len() + gsubrs.len();
    let private_at = cs_at + charstrings.len();
    let mut top = head;
    top.extend(di(cs_at as i32).into_iter().chain([17]).chain(di(private.len() as i32)).chain(di(private_at as i32)).chain([18]));
    assert_eq!(top.len(), top_len);

    let mut t = vec![1, 0, 4, 4];
    t.extend(names);
    t.extend(index(&[top], false));
    t.extend(strings);
    t.extend(gsubrs);
    assert_eq!(t.len(), cs_at);
    t.extend(charstrings);
    t.extend(private);
    t.extend(lsubrs);
    t
}

pub(crate) const CFF2_GLYPHS: [&str; 3] = [
    "",
    "0 101 50 51 2 blend hstem 11 21 rmoveto 101 3 1 blend 0 rlineto 0 101 rlineto -101 0 rlineto",
    "1 vsindex 0 51 hstemhm 31 41 -5 7 1 blend vstemhm hintmask <c0> 5 -7 rmoveto 11 0 11 11 11 0 11 0 11 -11 11 0 50 flex 21 3 5 -1 2 -3 2 blend rlineto",
];

/// A CFF2 table: two regions on one axis, item variation data 0 with one region and 1 with
/// both, blends under both `vsindex` values, a hint mask, and a Private DICT with blue zones and
/// StdHW.
pub(crate) fn cff2_table(glyphs: &[&str]) -> Vec<u8> {
    let glyphs: Vec<Vec<u8>> = glyphs.iter().map(|p| cs(p)).collect();
    let mut ivs = Vec::new();
    ivs.extend(1u16.to_be_bytes());
    ivs.extend(16u32.to_be_bytes());
    ivs.extend(2u16.to_be_bytes());
    ivs.extend(32u32.to_be_bytes());
    ivs.extend(40u32.to_be_bytes());
    for v in [1i16, 2, 0, 0x4000, 0x4000, 0, 0x2000, 0x4000, 0, 0, 1, 0, 0, 0, 2, 0, 1] {
        ivs.extend(v.to_be_bytes());
    }
    assert_eq!(ivs.len(), 50);
    let mut vstore = (ivs.len() as u16).to_be_bytes().to_vec();
    vstore.extend(ivs);
    let mut private: Vec<u8> = [-12, 12, 500, 12].into_iter().flat_map(di).collect();
    private.push(6);
    private.extend(di(41).into_iter().chain([10]));
    let top_len = 6 + 7 + 6;
    let gsubrs = index(&[], true);
    let vstore_at = 5 + top_len + gsubrs.len();
    let cs_at = vstore_at + vstore.len();
    let charstrings = index(&glyphs, true);
    let fdarray_at = cs_at + charstrings.len();
    let private_at = fdarray_at + 4 + 1 + 2 + 11;
    let fd: Vec<u8> = di(private.len() as i32).into_iter().chain(di(private_at as i32)).chain([18]).collect();
    let fdarray = index(&[fd], true);
    let top: Vec<u8> =
        di(cs_at as i32).into_iter().chain([17]).chain(di(fdarray_at as i32)).chain([12, 36]).chain(di(vstore_at as i32)).chain([24]).collect();
    assert_eq!(top.len(), top_len);
    let mut t = vec![2, 0, 5];
    t.extend((top_len as u16).to_be_bytes());
    t.extend(top);
    t.extend(gsubrs);
    t.extend(vstore);
    t.extend(charstrings);
    t.extend(fdarray);
    assert_eq!(t.len(), private_at);
    t.extend(private);
    t
}

/// The font around a CFF or CFF2 table: units per em `upem`, `hhea` ascender 800 and descender
/// -200, the given metrics, `maxp` 0.5 and a version 3 `post` with underline -100 and 50.
pub(crate) fn font(tag: [u8; 4], table: Vec<u8>, upem: u16, metrics: &[(u16, i16)]) -> SfntFont {
    let n = metrics.len() as u16;
    let mut f = SfntFont::new(Flavour::Cff);
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&upem.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&n.to_be_bytes());
    let hmtx: Vec<u8> = metrics.iter().flat_map(|(a, l)| a.to_be_bytes().into_iter().chain(l.to_be_bytes())).collect();
    let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
    maxp.extend(n.to_be_bytes());
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    post[8..10].copy_from_slice(&(-100i16).to_be_bytes());
    post[10..12].copy_from_slice(&50i16.to_be_bytes());
    for (t, data) in [(*b"head", head), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"maxp", maxp), (*b"post", post), (tag, table)] {
        f.insert(t, data);
    }
    f
}

pub(crate) fn cff_font(matrix: Option<[f64; 6]>, upem: u16) -> SfntFont {
    font(*b"CFF ", cff_table(&CFF_GLYPHS, matrix), upem, &[(337, 0), (637, 11), (501, 1), (738, 51)])
}

pub(crate) fn cff2_font() -> SfntFont {
    font(*b"CFF2", cff2_table(&CFF2_GLYPHS), 1000, &[(500, 0), (600, 11), (700, 5)])
}

fn upem(target: u16) -> TransformOptions {
    TransformOptions { units_per_em: Some(target), ..TransformOptions::default() }
}

pub(crate) fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_be_bytes([b[at], b[at + 1]])
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

/// `head` box, units per em; `hhea` largest advance, smallest side bearings, largest extent,
/// ascender, descender; `post` underline position and thickness.
pub(crate) fn metrics(f: &SfntFont) -> ([i16; 4], u16, [i16; 6], (i16, i16)) {
    let head = f.get(*b"head").unwrap();
    let hhea = f.get(*b"hhea").unwrap();
    let post = f.get(*b"post").unwrap();
    (
        [36, 38, 40, 42].map(|at| i16_at(head, at)),
        u16_at(head, 18),
        [10, 12, 14, 16, 4, 6].map(|at| i16_at(hhea, at)),
        (i16_at(post, 8), i16_at(post, 10)),
    )
}

/// Every glyph's advance and left side bearing, the last advance repeated past `numberOfHMetrics`.
pub(crate) fn hmtx(f: &SfntFont) -> Vec<(u16, i16)> {
    let t = f.get(*b"hmtx").unwrap();
    let long = usize::from(u16_at(f.get(*b"hhea").unwrap(), 34));
    let count = usize::from(u16_at(f.get(*b"maxp").unwrap(), 4));
    let mut advance = 0;
    (0..count)
        .map(|g| {
            if g < long {
                advance = u16_at(t, 4 * g);
                (advance, i16_at(t, 4 * g + 2))
            } else {
                (advance, i16_at(t, 4 * long + 2 * (g - long)))
            }
        })
        .collect()
}

/// Each entry of a DICT as its operator and operands.
pub(crate) fn dict(entries: &[crate::dehint::cff::Entry]) -> Vec<(u16, Vec<f64>)> {
    entries.iter().map(|e| (e.op, numbers(&e.raw).unwrap().unwrap())).collect()
}

pub(crate) fn top_value(t: &Table<'_>, op: u16) -> Option<Vec<f64>> {
    dict(&t.top).into_iter().find(|(o, _)| *o == op).map(|(_, v)| v)
}

#[test]
fn cff_units_per_em_matches_scale_upem() {
    let input = cff_font(None, 1000);
    let (out, report) = transform_with_report(&input, &upem(1500)).unwrap();
    let table = out.get(*b"CFF ").unwrap();
    let t = Table::read(table, false).unwrap();
    let expected = [
        "450 endchar",
        "900 0 150 hstem 77 122 vstem 17 32 rmoveto hintmask <c0> 152 0 rlineto 0 152 rlineto -151 0 rlineto endchar",
        "0 77 hstemhm 47 62 hintmask <c0> 2 -1 rmoveto 17 0 17 17 17 0 17 0 17 -16 17 0 75 flex 0 152 rlineto endchar",
        "1052 77 hmoveto 32 47 17 32 47 32 47 hflex 0 -16 rlineto endchar",
    ];
    for (gid, (got, want)) in t.charstrings.iter().zip(expected).enumerate() {
        assert_eq!(*got, cs(want).as_slice(), "glyph {gid}: scaleUpem writes {want}");
    }
    let private = dict(&t.privates[0].entries);
    assert_eq!(
        private,
        vec![
            (6, vec![-16.0, 16.0, 737.0, 15.0]),
            (11, vec![122.0]),
            (1212, vec![62.0, 33.0]),
            (1211, vec![1.0]),
            (20, vec![752.0]),
            (21, vec![56.0]),
        ],
        "blue zones and stem snaps scale as absolute values; BlueFuzz stays; Subrs go"
    );
    assert_eq!(top_value(&t, 1203), Some(vec![-112.0]));
    assert_eq!(top_value(&t, 1204), Some(vec![75.0]), "the default thickness is scaled and written");
    assert_eq!(top_value(&t, 5), Some(vec![2.0, -16.0, 314.0, 184.0]), "FontBBox from the scaled outlines");
    assert_eq!(top_value(&t, 1207), None, "a missing FontMatrix stays missing");

    assert_eq!(metrics(&out), ([2, -16, 314, 184], 1500, [1107, 2, 648, 314, 1200, -300], (-150, 75)));
    assert_eq!(hmtx(&out), vec![(506, 0), (956, 17), (752, 2), (1107, 77)]);
    assert_eq!(report.glyphs_changed, 4);
    assert_eq!(report.units_per_em, Some((1000, 1500)));
}

#[test]
fn cff2_units_per_em_keeps_vsindex_and_blend_counts() {
    let (out, _) = transform_with_report(&cff2_font(), &upem(2048)).unwrap();
    let t = Table::read(out.get(*b"CFF2").unwrap(), true).unwrap();
    let expected = [
        "",
        "0 207 102 104 2 blend hstem 23 43 rmoveto 207 6 1 blend 0 rlineto 0 207 rlineto -207 0 rlineto",
        "1 vsindex 0 104 hstemhm 63 84 -10 14 1 blend vstemhm hintmask <c0> 10 -14 rmoveto 23 0 23 23 23 0 23 0 23 -23 23 0 102 flex 43 6 10 -2 4 -6 2 blend rlineto",
    ];
    for (gid, (got, want)) in t.charstrings.iter().zip(expected).enumerate() {
        assert_eq!(*got, cs(want).as_slice(), "glyph {gid}: scaleUpem writes {want}");
    }
    assert_eq!(dict(&t.privates[0].entries), vec![(6, vec![-25.0, 25.0, 1024.0, 25.0]), (10, vec![84.0])]);
    assert_eq!(top_value(&t, 1207), None);
    assert_eq!(metrics(&out), ([10, -14, 230, 250], 2048, [1434, 10, 999, 230, 1638, -410], (-205, 102)));
    assert_eq!(hmtx(&out), vec![(1024, 0), (1229, 23), (1434, 10)]);
}

#[test]
fn cff_font_matrix_follows_the_factor() {
    let half = [0.0005, 0.0, 0.0, 0.0005, 0.0, 0.0];
    let input = cff_font(Some(half), 2000);
    let back = transform(&input, &upem(1000)).unwrap();
    let t = Table::read(back.get(*b"CFF ").unwrap(), false).unwrap();
    assert_eq!(top_value(&t, 1207), None, "a matrix that becomes the default is left out");
    let up = transform(&input, &upem(4000)).unwrap();
    let t = Table::read(up.get(*b"CFF ").unwrap(), false).unwrap();
    let m = top_value(&t, 1207).unwrap();
    for (got, want) in m.iter().zip([0.00025, 0.0, 0.0, 0.00025, 0.0, 0.0]) {
        assert!((got - want).abs() < 1e-12, "{m:?}");
    }
}

#[test]
fn hostile_cff_tables_are_refused() {
    let metrics = [(500u16, 0i16); 4];
    let overflow = font(*b"CFF ", cff_table(&["0 20000 rmoveto 0 1 rlineto endchar"; 4], None), 1000, &metrics);
    assert!(matches!(transform(&overflow, &upem(2048)), Err(TablesError::Invalid(_))), "a value past ±32,767 is refused");
    let arithmetic = font(*b"CFF ", cff_table(&["1 2 add 0 rmoveto endchar"; 4], None), 1000, &metrics);
    assert!(transform(&arithmetic, &upem(2048)).is_err(), "arithmetic operators are refused");

    let m3 = [(500u16, 0i16); 3];
    for program in ["0 1 2 3 300 blend hstem", "5 vsindex 0 rmoveto", "1 2 3 4 5 6 7 8 9 9 blend rlineto", "blend", "vsindex"] {
        let f = font(*b"CFF2", cff2_table(&["", program, ""]), 1000, &m3);
        assert!(transform(&f, &upem(2048)).is_err(), "{program} is refused");
    }

    let table = cff_table(&CFF_GLYPHS, None);
    for cut in (0..table.len()).step_by(7) {
        let f = font(*b"CFF ", table[..cut].to_vec(), 1000, &[(500, 0); 4]);
        assert!(transform(&f, &upem(2048)).is_err(), "a table cut at {cut} is refused");
    }
    assert!(matches!(scale_upem_within(&table, false, 2.0, 64), Err(TablesError::TooLarge { .. })), "the memory budget holds");
    assert!(scale_upem_within(&table, false, f64::NAN, 1 << 20).is_err());
}

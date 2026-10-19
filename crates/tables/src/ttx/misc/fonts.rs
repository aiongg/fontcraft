//! The test fonts of the other tables, generated together with `testdata/misc.ttx`,
//! `testdata/apple_kern.ttx`, `testdata/apple.ttx` and `testdata/apple2.ttx`: each table is written field by field, and the same bytes were
//! given to fontTools 4.66.1's `ttx`.

use crate::sfnt::{Flavour, SfntFont};

/// Big-endian table builder.
#[derive(Default)]
pub(super) struct Be(pub(super) Vec<u8>);

#[allow(dead_code)]
impl Be {
    pub(super) fn u8s(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    pub(super) fn i8s(&mut self, v: &[i8]) {
        self.0.extend(v.iter().map(|x| *x as u8));
    }
    pub(super) fn u16s(&mut self, v: &[u16]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    pub(super) fn i16s(&mut self, v: &[i16]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    pub(super) fn u32s(&mut self, v: &[u32]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    pub(super) fn i32s(&mut self, v: &[i32]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    pub(super) fn u64s(&mut self, v: &[u64]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()));
    }
    pub(super) fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
}

pub(super) fn head_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536, 65536, 0, 1594834165]);
    b.u16s(&[0, 1000]);
    b.u64s(&[0, 0]);
    b.i16s(&[0, 0, 0, 0]);
    b.u16s(&[0, 8]);
    b.i16s(&[2, 0, 0]);
    b.0
}

pub(super) fn maxp_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[20480]);
    b.u16s(&[4]);
    b.0
}

pub(super) fn post_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[131072, 0]);
    b.i16s(&[0, 0]);
    b.u32s(&[0, 0, 0, 0, 0]);
    b.u16s(&[4, 0, 258, 259, 260]);
    b.u8s(&[1]);
    b.bytes(b"A");
    b.u8s(&[1]);
    b.bytes(b"B");
    b.u8s(&[1]);
    b.bytes(b"C");
    b.0
}

pub(super) fn kern_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[0, 2, 0, 32]);
    b.u8s(&[0, 1]);
    b.u16s(&[3, 12, 1, 6, 2, 1]);
    b.i16s(&[30]);
    b.u16s(&[1, 3]);
    b.i16s(&[-10]);
    b.u16s(&[1, 2]);
    b.i16s(&[-50]);
    b.u16s(&[0, 10]);
    b.u8s(&[2, 1, 222, 173, 190, 239]);
    b.0
}

pub(super) fn dsig_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[1]);
    b.u16s(&[1, 1]);
    b.u32s(&[1, 68, 20]);
    b.u16s(&[0, 0]);
    b.i32s(&[60]);
    b.bytes(b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\x22#$%&'()*+,-./0123456789:;");
    b.0
}

pub(super) fn fftm_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[1]);
    b.u64s(&[3786000000, 3700000000, 0]);
    b.0
}

pub(super) fn ltsh_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[0, 4]);
    b.u8s(&[1, 12, 8, 255]);
    b.0
}

pub(super) fn vdmx_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[1, 1, 2]);
    b.u8s(&[0, 0, 0, 0, 1, 1, 1, 1]);
    b.u16s(&[18, 18, 2]);
    b.u8s(&[8, 9]);
    b.u16s(&[9]);
    b.i16s(&[8, -2]);
    b.u16s(&[8]);
    b.i16s(&[7, -2]);
    b.0
}

pub(super) fn prop_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[131072]);
    b.u16s(&[1, 0, 2, 6, 2, 12, 1, 0, 2, 1, 5, 3, 2, 7]);
    b.0
}

pub(super) fn apple_kern_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536, 1, 28]);
    b.u8s(&[0, 0]);
    b.u16s(&[3, 2, 12, 1, 0, 3, 1]);
    b.i16s(&[-20]);
    b.u16s(&[1, 1]);
    b.i16s(&[15]);
    b.0
}

pub(super) fn misc_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head_table());
    font.insert(*b"maxp", maxp_table());
    font.insert(*b"post", post_table());
    font.insert(*b"kern", kern_table());
    font.insert(*b"DSIG", dsig_table());
    font.insert(*b"FFTM", fftm_table());
    font.insert(*b"LTSH", ltsh_table());
    font.insert(*b"VDMX", vdmx_table());
    font.insert(*b"prop", prop_table());
    font
}

pub(super) fn apple_kern_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head_table());
    font.insert(*b"maxp", maxp_table());
    font.insert(*b"post", post_table());
    font.insert(*b"kern", apple_kern_table());
    font
}

pub(super) fn ankr_table() -> Vec<u8> {
    let mut b = Be::default();
    // header: format, flags, lookup offset, data offset
    b.u16s(&[0, 0]);
    b.u32s(&[12, 22]);
    // lookup format 8: first glyph 1, two values
    b.u16s(&[8, 1, 2, 0, 8]);
    // anchors of A, then of B
    b.u32s(&[1]);
    b.i16s(&[10, 20]);
    b.u32s(&[2]);
    b.i16s(&[1, 2, -3, -4]);
    b.0
}

pub(super) fn bsln_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536]);
    // format 3, default baseline, standard glyph, 32 control points
    b.u16s(&[
        3, 1, 2, 0, 1, 2, 3, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535,
        65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535,
    ]);
    // lookup format 2: two segments and the end marker
    b.u16s(&[2, 6, 3, 6, 0, 0, 2, 1, 3, 3, 3, 5, 65535, 65535, 0]);
    b.0
}

pub(super) fn cidg_table() -> Vec<u8> {
    let mut b = Be::default();
    // format, data format, length, registry, order, supplement
    b.u16s(&[0, 0]);
    b.u32s(&[150]);
    b.u16s(&[1]);
    b.u8s(&[
        65, 100, 111, 98, 101, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    b.u16s(&[2]);
    b.u8s(&[
        74, 97, 112, 97, 110, 49, 233, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    b.u16s(&[6]);
    // CID to glyph
    b.u16s(&[4, 1, 65535, 3, 2]);
    b.0
}

pub(super) fn feat_table() -> Vec<u8> {
    let mut b = Be::default();
    // version, feature name count, reserved
    b.u32s(&[65536]);
    b.u16s(&[2, 0]);
    b.u32s(&[0]);
    // feature names: type, settings count, settings offset, flags, name ID
    b.u16s(&[1, 2]);
    b.u32s(&[36]);
    b.u16s(&[32768, 256, 3, 1]);
    b.u32s(&[44]);
    b.u16s(&[49153, 258]);
    // settings: value, name ID
    b.u16s(&[0, 257, 1, 259, 4, 260]);
    b.0
}

pub(super) fn gcid_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[0, 0]);
    b.u32s(&[152]);
    b.u16s(&[1]);
    b.u8s(&[
        65, 100, 111, 98, 101, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    b.u16s(&[2]);
    b.u8s(&[
        74, 97, 112, 97, 110, 49, 233, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    b.u16s(&[6]);
    // glyph to CID, one more than the font has glyphs
    b.u16s(&[5, 10, 65535, 30, 20, 99]);
    b.0
}

pub(super) fn lcar_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536]);
    b.u16s(&[1]);
    // lookup format 4 at 6: one segment and the end marker; its offsets at 30
    b.u16s(&[4, 6, 2, 6, 0, 0, 2, 1, 24, 65535, 65535, 0, 34, 38]);
    // caret points of A, then of B
    b.u16s(&[1]);
    b.i16s(&[5]);
    b.u16s(&[2]);
    b.i16s(&[7, -8]);
    b.0
}

pub(super) fn ltag_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[1, 0, 2]);
    b.u16s(&[20, 2, 22, 7]);
    b.u8s(&[101, 110, 122, 104, 45, 72, 97, 110, 116]);
    b.0
}

pub(super) fn opbd_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536]);
    b.u16s(&[0]);
    // lookup format 6 at 6: two glyphs and the end marker
    b.u16s(&[6, 4, 3, 4, 0, 0, 3, 38, 1, 30, 65535, 0]);
    // bounds of A, then of C
    b.i16s(&[-10, 0, 20, 5, 1, 2, 3, 4]);
    b.0
}

pub(super) fn trak_table() -> Vec<u8> {
    let mut b = Be::default();
    // version, format, horizontal and vertical offsets, reserved
    b.u32s(&[65536]);
    b.u16s(&[0, 12, 0, 0]);
    // tracks 3, sizes 2, size table at 44
    b.u16s(&[3, 2]);
    b.u32s(&[44]);
    // tracks: value, name ID, offset of the per-size values
    b.i32s(&[65536]);
    b.u16s(&[258, 60]);
    b.i32s(&[-65536]);
    b.u16s(&[256, 52]);
    b.i32s(&[0]);
    b.u16s(&[257, 56]);
    // sizes 12 and 24.5
    b.i32s(&[786432, 1605632]);
    // per-size values
    b.i16s(&[-12, -24, 0, 0, 30, 40]);
    b.0
}

pub(super) fn name_table() -> Vec<u8> {
    let mut b = Be::default();
    // format 0, no records
    b.u16s(&[0, 0, 6]);
    b.0
}

pub(super) fn bsln_table_2() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536]);
    b.u16s(&[
        1, 0, 0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210, 220, 230, 240, 250, 260, 270, 280,
        290, 300, 310,
    ]);
    // lookup format 0: a value per glyph
    b.u16s(&[0, 0, 1, 2, 1]);
    b.0
}

pub(super) fn lcar_table_2() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536]);
    b.u16s(&[0]);
    // lookup format 2 at 6: glyphs 1 to 3 share one table
    b.u16s(&[2, 6, 1, 6, 0, 0, 3, 1, 24]);
    // caret distances at 24
    b.u16s(&[2]);
    b.i16s(&[100, 200]);
    b.0
}

pub(super) fn opbd_table_2() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536]);
    b.u16s(&[1]);
    // lookup format 8 at 6: glyphs 2 and 3, the second without bounds
    b.u16s(&[8, 2, 2, 16, 0]);
    // bounds of B at 16
    b.i16s(&[1, 2, 3, 4]);
    b.0
}

/// ankr, bsln (format 3), cidg, feat, gcid, lcar (format 1), ltag, opbd (format 0) and trak, with
/// an empty name table (`ttx` cannot write `trak` without one).
pub(super) fn apple_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head_table());
    font.insert(*b"maxp", maxp_table());
    font.insert(*b"post", post_table());
    font.insert(*b"name", name_table());
    font.insert(*b"ankr", ankr_table());
    font.insert(*b"bsln", bsln_table());
    font.insert(*b"cidg", cidg_table());
    font.insert(*b"feat", feat_table());
    font.insert(*b"gcid", gcid_table());
    font.insert(*b"lcar", lcar_table());
    font.insert(*b"ltag", ltag_table());
    font.insert(*b"opbd", opbd_table());
    font.insert(*b"trak", trak_table());
    font
}

/// The other formats: bsln 1 (lookup format 0), lcar 0 (lookup format 2), opbd 1 (lookup format
/// 8, one glyph without bounds).
pub(super) fn apple_font_2() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head_table());
    font.insert(*b"maxp", maxp_table());
    font.insert(*b"post", post_table());
    font.insert(*b"bsln", bsln_table_2());
    font.insert(*b"lcar", lcar_table_2());
    font.insert(*b"opbd", opbd_table_2());
    font
}

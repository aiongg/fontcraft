//! The test font of the colour tables, generated together with `testdata/color.ttx`: each
//! table is written field by field, and the same bytes were given to fontTools 4.66.1's `ttx`.

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

pub(super) fn colr_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[0, 2]);
    b.u32s(&[14, 26]);
    b.u16s(&[3, 3, 0, 2, 1, 2, 1, 1, 0, 2, 1, 2, 2]);
    b.0
}

pub(super) fn cpal_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[1, 2, 2, 3]);
    b.u32s(&[44]);
    b.u16s(&[0, 1]);
    b.u32s(&[28, 36, 40, 0, 1]);
    b.u16s(&[65535, 300, 65535, 301]);
    b.u8s(&[0, 0, 255, 255, 16, 128, 32, 127, 255, 255, 255, 0]);
    b.0
}

pub(super) fn svg_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[0]);
    b.u32s(&[10, 0]);
    b.u16s(&[2, 1, 1]);
    b.u32s(&[26, 81]);
    b.u16s(&[2, 3]);
    b.u32s(&[107, 86]);
    b.bytes(b"<svg xmlns=\x22http://www.w3.org/2000/svg\x22 id=\x22glyph1\x22><path d=\x22M0 0h10v10z\x22/></svg>");
    b.bytes(b"<svg xmlns=\x22http://www.w3.org/2000/svg\x22 id=\x22glyph2\x22>\x0a<text>a]]>b &amp; c</text>\x0a</svg>");
    b.0
}

pub(super) fn sbix_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[1, 1]);
    b.u32s(&[2, 16, 78]);
    b.u16s(&[20, 72]);
    b.u32s(&[24, 24, 52, 62, 62]);
    b.i16s(&[1, -2]);
    b.bytes(b"png ");
    b.bytes(b"\x89PNG\x0d\x0a\x1a\x0a");
    b.u8s(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    b.i16s(&[0, 0]);
    b.bytes(b"dupe");
    b.u16s(&[1, 10, 72]);
    b.u32s(&[24, 24, 36, 36, 36]);
    b.i16s(&[-3, 4]);
    b.bytes(b"jpg ");
    b.u8s(&[255, 216, 255, 224]);
    b.0
}

pub(super) fn color_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head_table());
    font.insert(*b"maxp", maxp_table());
    font.insert(*b"post", post_table());
    font.insert(*b"COLR", colr_table());
    font.insert(*b"CPAL", cpal_table());
    font.insert(*b"SVG ", svg_table());
    font.insert(*b"sbix", sbix_table());
    font
}

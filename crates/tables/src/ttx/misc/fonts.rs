//! The test fonts of the other tables, generated together with `testdata/misc.ttx` and
//! `testdata/apple_kern.ttx`: each table is written field by field, and the same bytes were
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

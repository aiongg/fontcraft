//! The test font of the variation tables, generated together with `testdata/var.ttx`: each
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

pub(super) fn glyf_table() -> Vec<u8> {
    let mut b = Be::default();
    b.i16s(&[1, 0, 0, 100, 100]);
    b.u16s(&[2, 0]);
    b.u8s(&[1, 1, 1]);
    b.i16s(&[0, 50, 50, 0, 100, -100]);
    b.u8s(&[0]);
    b.i16s(&[1, 0, 0, 100, 100]);
    b.u16s(&[3, 0]);
    b.u8s(&[1, 1, 1, 1]);
    b.i16s(&[0, 0, 100, 0, 0, 100, 0, -100, -1, 0, 0, 100, 100]);
    b.u16s(&[35, 1]);
    b.i16s(&[0, 0]);
    b.u16s(&[3, 2]);
    b.i16s(&[10, 0]);
    b.0
}

pub(super) fn loca_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[0, 0, 15, 32, 45]);
    b.0
}

pub(super) fn fvar_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[1, 0, 16, 2, 2, 20, 0, 12]);
    b.bytes(b"wght");
    b.u32s(&[6553600, 26214400, 58982400]);
    b.u16s(&[0, 256]);
    b.bytes(b"wdth");
    b.u32s(&[3276800, 6553600, 6553600]);
    b.u16s(&[0, 257]);
    b.0
}

pub(super) fn gvar_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[1, 0, 2, 2]);
    b.u32s(&[30]);
    b.u16s(&[4, 0]);
    b.u32s(&[38]);
    b.u16s(&[0, 0, 25, 37, 47]);
    b.i16s(&[16384, 0, 0, 16384]);
    b.u16s(&[32770, 24, 23, 0, 2, 49152]);
    b.i16s(&[8192, 0, 4096, 0, 16384, 0]);
    b.u8s(&[0, 6]);
    b.i8s(&[1, 2, 3, 4, 5, 6, 7]);
    b.u8s(&[70]);
    b.i16s(&[-100, 200, -300, 400, -500, 600, -700]);
    b.u8s(&[134, 134]);
    b.u16s(&[1, 8, 16, 8193]);
    b.u8s(&[3, 2, 0, 2, 3, 2]);
    b.i8s(&[10, -20, 30]);
    b.u8s(&[66]);
    b.i16s(&[-300, 0, 400]);
    b.u16s(&[1, 12, 7, 40960]);
    b.i16s(&[-16384, 8192]);
    b.u8s(&[1, 0, 1, 0]);
    b.i8s(&[5]);
    b.u8s(&[0]);
    b.i8s(&[-5]);
    b.u8s(&[0]);
    b.0
}

pub(super) fn cvt_table() -> Vec<u8> {
    let mut b = Be::default();
    b.i16s(&[10, 20, -30, 40]);
    b.0
}

pub(super) fn cvar_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u16s(&[1, 0, 32770, 24, 5, 32768]);
    b.i16s(&[16384, 0]);
    b.u16s(&[9, 40960]);
    b.i16s(&[0, -16384]);
    b.u8s(&[0, 3]);
    b.i8s(&[1, -1, 2, -2]);
    b.u8s(&[2, 1, 0, 3, 65]);
    b.i16s(&[300, -100]);
    b.0
}

pub(super) fn hvar_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536, 20, 83, 0, 90]);
    b.u16s(&[1]);
    b.u32s(&[16]);
    b.u16s(&[2]);
    b.u32s(&[44, 0]);
    b.u16s(&[2, 2]);
    b.i16s(&[0, 16384, 16384, 0, 0, 0, 0, 0, 0, -16384, -16384, 0]);
    b.u16s(&[3, 1, 2, 0, 1]);
    b.i16s(&[-500]);
    b.i8s(&[10]);
    b.i16s(&[20]);
    b.i8s(&[-3]);
    b.i16s(&[0]);
    b.i8s(&[0]);
    b.u16s(&[1, 3]);
    b.u8s(&[0, 1, 6]);
    b.u16s(&[16, 1, 3]);
    b.0
}

pub(super) fn vvar_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536, 24, 0, 0, 0, 32]);
    b.u16s(&[1]);
    b.u32s(&[0]);
    b.u16s(&[0]);
    b.u32s(&[]);
    b.u16s(&[0, 1]);
    b.u8s(&[0]);
    b.0
}

pub(super) fn mvar_table() -> Vec<u8> {
    let mut b = Be::default();
    b.u32s(&[65536]);
    b.u16s(&[0, 10, 2, 32]);
    b.bytes(b"hasc");
    b.u32s(&[1]);
    b.u8s(&[7, 9]);
    b.bytes(b"xhgt");
    b.u32s(&[65536]);
    b.u8s(&[0, 255]);
    b.u16s(&[1]);
    b.u32s(&[12]);
    b.u16s(&[1]);
    b.u32s(&[28]);
    b.u16s(&[2, 1]);
    b.i16s(&[0, 16384, 16384, 0, 0, 0]);
    b.u16s(&[1, 32769, 1, 0]);
    b.i32s(&[70000]);
    b.0
}

pub(super) fn var_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head_table());
    font.insert(*b"maxp", maxp_table());
    font.insert(*b"post", post_table());
    font.insert(*b"glyf", glyf_table());
    font.insert(*b"loca", loca_table());
    font.insert(*b"fvar", fvar_table());
    font.insert(*b"gvar", gvar_table());
    font.insert(*b"cvt ", cvt_table());
    font.insert(*b"cvar", cvar_table());
    font.insert(*b"HVAR", hvar_table());
    font.insert(*b"VVAR", vvar_table());
    font.insert(*b"MVAR", mvar_table());
    font
}

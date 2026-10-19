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
    pub(super) fn u24s(&mut self, v: &[u32]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_be_bytes()[1..]));
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

// ── COLR version 1 and the bitmap tables (generated with testdata/colr1.ttx, bitmap.ttx and
// cbdt.ttx: the same fields were given to fontTools 4.66.1's `ttx`) ──────────────────────────

pub(super) fn colr_v1_table() -> Vec<u8> {
    let mut b = Be::default();
    // COLR at 0
    b.u16s(&[1, 1]);
    b.u32s(&[34, 40]);
    b.u16s(&[2]);
    b.u32s(&[48, 70, 86, 119, 125]);
    // BaseGlyphRecordArray at 34
    b.u16s(&[3, 0, 2]);
    // LayerRecordArray at 40
    b.u16s(&[1, 0, 2, 1]);
    // BaseGlyphList at 48
    b.u32s(&[3]);
    b.u16s(&[1]);
    b.u32s(&[89]);
    b.u16s(&[2]);
    b.u32s(&[95]);
    b.u16s(&[3]);
    b.u32s(&[102]);
    // LayerList at 70
    b.u32s(&[3, 88, 94, 100]);
    // ClipList at 86
    b.u8s(&[1]);
    b.u32s(&[4]);
    b.u16s(&[3, 3]);
    b.u24s(&[94]);
    b.u16s(&[2, 2]);
    b.u24s(&[103]);
    b.u16s(&[1, 3]);
    b.u24s(&[116]);
    b.u16s(&[5, 9]);
    b.u24s(&[103]);
    // DeltaSetIndexMap at 119
    b.u8s(&[0, 0]);
    b.u16s(&[2]);
    b.u8s(&[0, 3]);
    // VarStore at 125
    b.u16s(&[1]);
    b.u32s(&[86]);
    b.u16s(&[1]);
    b.u32s(&[96]);
    // PaintColrLayers at 137
    b.u8s(&[1, 2]);
    b.u32s(&[0]);
    // PaintTransform at 143
    b.u8s(&[12]);
    b.u24s(&[87, 93]);
    // PaintComposite at 150
    b.u8s(&[32]);
    b.u24s(&[110]);
    b.u8s(&[23]);
    b.u24s(&[124]);
    // PaintGlyph at 158
    b.u8s(&[10]);
    b.u24s(&[132]);
    b.u16s(&[2]);
    // PaintGlyph at 164
    b.u8s(&[10]);
    b.u24s(&[131]);
    b.u16s(&[3]);
    // PaintVarScaleUniform at 170
    b.u8s(&[21]);
    b.u24s(&[141]);
    b.i16s(&[24576]);
    b.u32s(&[11]);
    // ClipBoxFormat1 at 180
    b.u8s(&[1]);
    b.i16s(&[0, 0, 500, 500]);
    // ClipBoxFormat2 at 189
    b.u8s(&[2]);
    b.i16s(&[1, 2, 3, 4]);
    b.u32s(&[9]);
    // ClipBoxFormat1 at 202
    b.u8s(&[1]);
    b.i16s(&[0, 0, 500, 500]);
    // VarRegionList at 211
    b.u16s(&[1, 1]);
    b.i16s(&[0, 16384, 16384]);
    // VarData at 221
    b.u16s(&[1, 0, 1, 0]);
    b.i8s(&[5]);
    // PaintGlyph at 230
    b.u8s(&[10]);
    b.u24s(&[93]);
    b.u16s(&[3]);
    // Affine2x3 at 236
    b.i32s(&[98304, 0, 16384, 65536, 655360, -1310720]);
    // PaintVarRotateAroundCenter at 260
    b.u8s(&[27]);
    b.u24s(&[72]);
    b.i16s(&[4142, 100, -200]);
    b.u32s(&[5]);
    // PaintVarSweepGradient at 274
    b.u8s(&[9]);
    b.u24s(&[61]);
    b.i16s(&[3, 4, -8192, 16383]);
    b.u32s(&[7]);
    // PaintSolid at 290
    b.u8s(&[2]);
    b.u16s(&[0]);
    b.i16s(&[8192]);
    // PaintLinearGradient at 295
    b.u8s(&[4]);
    b.u24s(&[63]);
    b.i16s(&[1, 2, -3, -4, 5, 6]);
    // PaintSkewAroundCenter at 311
    b.u8s(&[30]);
    b.u24s(&[62]);
    b.i16s(&[1234, -8192, 7, -9]);
    // PaintVarSolid at 323
    b.u8s(&[3]);
    b.u16s(&[1]);
    b.i16s(&[16384]);
    b.u32s(&[4294967295]);
    // PaintColrGlyph at 332
    b.u8s(&[11]);
    b.u16s(&[2]);
    // VarColorLine at 335
    b.u8s(&[2]);
    b.u16s(&[2]);
    b.i16s(&[0]);
    b.u16s(&[0]);
    b.i16s(&[16384]);
    b.u32s(&[3]);
    b.i16s(&[12288]);
    b.u16s(&[1]);
    b.i16s(&[4096]);
    b.u32s(&[4294967295]);
    // ColorLine at 358
    b.u8s(&[1]);
    b.u16s(&[2]);
    b.i16s(&[0]);
    b.u16s(&[0]);
    b.i16s(&[16384, 16384]);
    b.u16s(&[1]);
    b.i16s(&[4096]);
    // PaintSolid at 373
    b.u8s(&[2]);
    b.u16s(&[1]);
    b.i16s(&[16384]);
    b.0
}

pub(super) fn eblc_table() -> Vec<u8> {
    let mut b = Be::default();
    // header
    b.u32s(&[131072, 3]);
    // bitmap size table
    b.u32s(&[152, 52, 2, 0]);
    b.i8s(&[10, -2]);
    b.u8s(&[8]);
    b.i8s(&[1, 0, 0, 0, 0, 9, -1, 0, 0, 0, 0]);
    b.u8s(&[0]);
    b.i8s(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.u16s(&[1, 3]);
    b.u8s(&[15, 15, 1]);
    b.i8s(&[1]);
    // bitmap size table
    b.u32s(&[204, 48, 2, 0]);
    b.i8s(&[10, -2]);
    b.u8s(&[8]);
    b.i8s(&[1, 0, 0, 0, 0, 9, -1, 0, 0, 0, 0]);
    b.u8s(&[0]);
    b.i8s(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.u16s(&[1, 3]);
    b.u8s(&[15, 15, 1]);
    b.i8s(&[1]);
    // bitmap size table
    b.u32s(&[252, 68, 2, 0]);
    b.i8s(&[10, -2]);
    b.u8s(&[8]);
    b.i8s(&[1, 0, 0, 0, 0, 9, -1, 0, 0, 0, 0]);
    b.u8s(&[0]);
    b.i8s(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.u16s(&[0, 2]);
    b.u8s(&[15, 15, 1]);
    b.i8s(&[1]);
    // index subtable array at 152
    b.u16s(&[1, 3]);
    b.u32s(&[16]);
    b.u16s(&[1, 1]);
    b.u32s(&[40]);
    // index subtable format 1
    b.u16s(&[1, 1]);
    b.u32s(&[4, 0, 7, 7, 13]);
    // index subtable format 3
    b.u16s(&[3, 2]);
    b.u32s(&[17]);
    b.u16s(&[0, 7]);
    // index subtable array at 204
    b.u16s(&[1, 2]);
    b.u32s(&[16]);
    b.u16s(&[3, 3]);
    b.u32s(&[36]);
    // index subtable format 2
    b.u16s(&[2, 5]);
    b.u32s(&[24, 2]);
    b.u8s(&[2, 8]);
    b.i8s(&[0, 2]);
    b.u8s(&[9]);
    b.i8s(&[-4, 0]);
    b.u8s(&[2]);
    // index subtable format 3
    b.u16s(&[3, 6]);
    b.u32s(&[28]);
    b.u16s(&[0, 9]);
    // index subtable array at 252
    b.u16s(&[0, 1]);
    b.u32s(&[16]);
    b.u16s(&[2, 2]);
    b.u32s(&[40]);
    // index subtable format 4
    b.u16s(&[4, 7]);
    b.u32s(&[48, 2]);
    b.u16s(&[0, 0, 1, 12, 0, 21]);
    // index subtable format 5
    b.u16s(&[5, 7]);
    b.u32s(&[37, 11]);
    b.u8s(&[2, 2]);
    b.i8s(&[1, 2]);
    b.u8s(&[3]);
    b.i8s(&[-1, -1]);
    b.u8s(&[3]);
    b.u32s(&[1]);
    b.u16s(&[2, 0]);
    b.0
}

pub(super) fn ebdt_table() -> Vec<u8> {
    let mut b = Be::default();
    // header
    b.u32s(&[131072]);
    // f1
    b.u8s(&[2, 3]);
    b.i8s(&[0, 2]);
    b.u8s(&[4, 224, 160]);
    // f1c
    b.u8s(&[1, 8]);
    b.i8s(&[-1, 1]);
    b.u8s(&[9, 255]);
    // f2
    b.u8s(&[3, 3]);
    b.i8s(&[0, 3]);
    b.u8s(&[4, 247, 128]);
    // f5a
    b.u8s(&[18, 52]);
    // f5b
    b.u8s(&[86, 120]);
    // f6
    b.u8s(&[1, 4]);
    b.i8s(&[0, 1]);
    b.u8s(&[5]);
    b.i8s(&[-2, 0]);
    b.u8s(&[2, 240]);
    // f7
    b.u8s(&[2, 2]);
    b.i8s(&[1, 2]);
    b.u8s(&[3]);
    b.i8s(&[-1, -1]);
    b.u8s(&[3, 192, 0, 0]);
    // f7b
    b.u8s(&[5, 5]);
    b.i8s(&[0, 5]);
    b.u8s(&[6]);
    b.i8s(&[-2, 0]);
    b.u8s(&[6, 255, 255, 255, 128]);
    // f7c
    b.u8s(&[1, 3]);
    b.i8s(&[1, 1]);
    b.u8s(&[4]);
    b.i8s(&[-1, 0]);
    b.u8s(&[1, 160]);
    // end
    b.0
}

pub(super) fn cblc_table() -> Vec<u8> {
    let mut b = Be::default();
    // header
    b.u32s(&[196608, 2]);
    // bitmap size table
    b.u32s(&[104, 48, 2, 0]);
    b.i8s(&[10, -2]);
    b.u8s(&[8]);
    b.i8s(&[1, 0, 0, 0, 0, 9, -1, 0, 0, 0, 0]);
    b.u8s(&[0]);
    b.i8s(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.u16s(&[1, 2]);
    b.u8s(&[14, 14, 1]);
    b.i8s(&[1]);
    // bitmap size table
    b.u32s(&[152, 56, 2, 0]);
    b.i8s(&[10, -2]);
    b.u8s(&[8]);
    b.i8s(&[1, 0, 0, 0, 0, 9, -1, 0, 0, 0, 0]);
    b.u8s(&[0]);
    b.i8s(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.u16s(&[1, 3]);
    b.u8s(&[14, 14, 1]);
    b.i8s(&[1]);
    // index subtable array at 104
    b.u16s(&[1, 1]);
    b.u32s(&[16]);
    b.u16s(&[2, 2]);
    b.u32s(&[32]);
    // index subtable format 1
    b.u16s(&[1, 17]);
    b.u32s(&[4, 0, 37]);
    // index subtable format 1
    b.u16s(&[1, 18]);
    b.u32s(&[41, 0, 15]);
    // index subtable array at 152
    b.u16s(&[1, 3]);
    b.u32s(&[16]);
    b.u16s(&[2, 2]);
    b.u32s(&[44]);
    // index subtable format 5
    b.u16s(&[5, 19]);
    b.u32s(&[56, 6]);
    b.u8s(&[1, 1]);
    b.i8s(&[0, 1]);
    b.u8s(&[2]);
    b.i8s(&[0, 0]);
    b.u8s(&[2]);
    b.u32s(&[2]);
    b.u16s(&[1, 3]);
    // index subtable format 3
    b.u16s(&[3, 1]);
    b.u32s(&[68]);
    b.u16s(&[0, 6]);
    b.0
}

pub(super) fn cbdt_table() -> Vec<u8> {
    let mut b = Be::default();
    // header
    b.u32s(&[196608]);
    // f17
    b.u8s(&[4, 4]);
    b.i8s(&[0, 4]);
    b.u8s(&[5]);
    b.u32s(&[28]);
    b.u8s(&[137, 80, 78, 71, 13, 10, 26, 10, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]);
    // f18
    b.u8s(&[4, 4]);
    b.i8s(&[0, 4]);
    b.u8s(&[5]);
    b.i8s(&[-2, 0]);
    b.u8s(&[5]);
    b.u32s(&[3]);
    b.u8s(&[1, 2, 3]);
    // f19a
    b.u32s(&[2]);
    b.u8s(&[171, 205]);
    // f19b
    b.u32s(&[2]);
    b.u8s(&[239, 1]);
    // f1
    b.u8s(&[1, 1]);
    b.i8s(&[0, 1]);
    b.u8s(&[2, 128]);
    b.0
}

fn base_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head_table());
    font.insert(*b"maxp", maxp_table());
    font.insert(*b"post", post_table());
    font
}

pub(super) fn colr_v1_font() -> SfntFont {
    let mut font = base_font();
    font.insert(*b"COLR", colr_v1_table());
    font
}

/// EBLC and EBDT, and the same bytes again as the Apple bloc and bdat.
pub(super) fn bitmap_font() -> SfntFont {
    let mut font = base_font();
    font.insert(*b"EBLC", eblc_table());
    font.insert(*b"EBDT", ebdt_table());
    font.insert(*b"bloc", eblc_table());
    font.insert(*b"bdat", ebdt_table());
    font
}

pub(super) fn cbdt_font() -> SfntFont {
    let mut font = base_font();
    font.insert(*b"CBLC", cblc_table());
    font.insert(*b"CBDT", cbdt_table());
    font
}

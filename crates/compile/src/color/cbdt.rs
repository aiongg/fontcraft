//! `CBDT` and `CBLC` (M11.11), an export option the reference app does not have: strikes from the
//! same iColor layers as sbix, PNG data in glyph bitmap format 17 with small metrics, index
//! subtable format 1, as nanoemoji writes them.
//!
//! Rows: `CBDT`, `color.export.cbdtNotInReference`.
//!
//! Layout (OpenType `CBLC` and `CBDT`, version 3.0), in the order fontTools compiles it:
//! - `CBLC`: the header, one `BitmapSize` record per strike, then for each strike its index
//!   subtable array followed by its index subtables. Each run of consecutive glyph ids in a
//!   strike is one index subtable of format 1: image format 17, an image data offset into `CBDT`,
//!   and `count + 1` offsets from it.
//! - `CBDT`: the header, then each glyph's record in strike, subtable and glyph order: small
//!   metrics (height, width, bearing x, bearing y, advance), the PNG length and the PNG bytes.
//!
//! The metrics follow the image's placement on its layer as `sbix` does: the image is drawn at
//! one pixel per `units_per_em ÷ ppem` units with its bottom-left corner where the layer puts
//! it, so bearing y is that corner's height in pixels plus the image height. The advance is the
//! glyph's advance width in pixels. Each strike's line metrics span its bitmaps: the ascender is
//! the highest top, the descender the lowest bottom, the widest advance is `widthMax`; the
//! vertical line metrics repeat them.
//!
//! Small metrics and `BitmapSize` hold bytes, so a strike above 255 pixels per em, an image wider
//! or higher than 255 pixels, and a bearing or advance beyond a byte are left out with a warning.
//! JPEG and TIFF images are converted to PNG, the only image data format 17 holds.

use std::sync::Arc;

use fontcraft_color::export::CbdtPlan;
use fontcraft_color::strike::StrikeGlyph;
use fontcraft_color::{AssetStore, MAX_STRIKES};
use fontcraft_imagery::{ImageFormat, MAX_PIXELS, decode_rgba, sniff, to_png};

use super::GlyphOrder;
use crate::CompileError;

/// Largest `CBDT` table written. Its offsets are 32-bit; this keeps a font that holds it well
/// within what renderers load.
pub const MAX_TABLE_BYTES: usize = 1 << 30;

/// The largest ppem, image side and advance small metrics and `BitmapSize` can state.
const MAX_BYTE: u32 = 255;

/// How far an image's scale may be from `units_per_em ÷ ppem`, as a fraction of it, before
/// export warns that the image is not drawn as placed.
const SCALE_TOLERANCE: f64 = 0.01;

/// Glyph bitmap format 17: small metrics and PNG data.
const IMAGE_FORMAT: u16 = 17;

/// Small metrics (5 bytes) and the data length (4 bytes) before each PNG.
const RECORD_HEADER: usize = 9;

/// `BitmapSize` record length.
const BITMAP_SIZE_LEN: usize = 48;

/// The two tables built together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CbdtOutput {
    pub cbdt: Vec<u8>,
    pub cblc: Vec<u8>,
}

/// One glyph's bitmap in a strike.
struct Record {
    gid: u16,
    width: u8,
    height: u8,
    bearing_x: i8,
    bearing_y: i8,
    advance: u8,
    png: Arc<[u8]>,
}

impl Record {
    fn len(&self) -> Option<usize> {
        RECORD_HEADER.checked_add(self.png.len())
    }
}

/// A strike with at least one record, sorted by glyph id without repeats.
struct Strike {
    ppem: u8,
    records: Vec<Record>,
}

/// The tables. A missing or unreadable image leaves the glyph out of its strike, with a warning.
pub fn build(
    plan: &CbdtPlan,
    order: &GlyphOrder,
    assets: &dyn AssetStore,
    units_per_em: u16,
    warnings: &mut Vec<String>,
) -> Result<CbdtOutput, CompileError> {
    if units_per_em == 0 {
        return Err(table_error("the font has 0 units per em".into()));
    }
    let mut strikes = Vec::new();
    for s in plan.strikes.iter().take(MAX_STRIKES) {
        let Some(ppem) = u8::try_from(s.ppem).ok().filter(|p| *p > 0) else {
            warnings.push(format!("the iColor {} strike is left out of CBDT, which holds sizes from 1 to {MAX_BYTE} pixels per em", s.ppem));
            continue;
        };
        let mut records = Vec::new();
        for g in &s.glyphs {
            let Some(gid) = order.gid(&g.glyph) else { continue };
            let advance = order.glyphs.get(usize::from(gid)).map_or(0.0, |r| r.width);
            if let Some(r) = record(g, gid, ppem, units_per_em, advance, assets, warnings) {
                records.push(r);
            }
        }
        records.sort_by_key(|r| r.gid);
        records.dedup_by_key(|r| r.gid);
        if !records.is_empty() {
            strikes.push(Strike { ppem, records });
        }
    }
    if strikes.is_empty() && !plan.strikes.is_empty() {
        warnings.push("no iColor image could be exported, so the CBDT and CBLC tables have no strikes".into());
    }
    write(&strikes)
}

/// The record of `g` in the `ppem` strike, or `None` with a warning when the image cannot be used.
fn record(
    g: &StrikeGlyph,
    gid: u16,
    ppem: u8,
    units_per_em: u16,
    advance: f64,
    assets: &dyn AssetStore,
    warnings: &mut Vec<String>,
) -> Option<Record> {
    let mut left_out = |why: String| {
        warnings.push(format!("glyph `{}`: {why}; it is left out of the {ppem} pixel CBDT strike", g.glyph));
        None
    };
    let bytes = match assets.read(&g.image) {
        Ok(b) => b,
        Err(e) => return left_out(format!("the image of its iColor {ppem} layer cannot be read: {e}")),
    };
    let info = match sniff(&bytes) {
        Ok(i) => i,
        Err(e) => return left_out(format!("the image `{}` cannot be used: {e}", g.image)),
    };
    // Checked before any conversion, which keeps converted images small.
    let (Ok(width), Ok(height)) = (u8::try_from(info.width), u8::try_from(info.height)) else {
        return left_out(format!(
            "the image `{}` is {} × {} pixels, and CBDT holds images up to {MAX_BYTE} pixels wide and high",
            g.image, info.width, info.height
        ));
    };
    let png = match info.format {
        ImageFormat::Png => bytes,
        ImageFormat::Jpeg | ImageFormat::Tiff => match decode_rgba(&bytes, MAX_PIXELS).and_then(|image| to_png(&image)) {
            Ok(png) => Arc::from(png),
            Err(e) => return left_out(format!("the image `{}` cannot be converted to PNG: {e}", g.image)),
        },
        _ => return left_out(format!("`{}` is not a PNG, JPEG or TIFF image", g.image)),
    };
    let (w, h) = (f64::from(info.width), f64::from(info.height));
    let [a, b, c, d, e, f] = g.placement.affine().as_coeffs();
    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let min_x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let pixels = f64::from(ppem) / f64::from(units_per_em);
    let bearing_x = rounded(min_x * pixels).and_then(|v| i8::try_from(v).ok());
    let bearing_y = rounded(min_y * pixels).and_then(|v| i8::try_from(v + i64::from(height)).ok());
    let (Some(bearing_x), Some(bearing_y)) = (bearing_x, bearing_y) else {
        return left_out(format!("the image of its iColor {ppem} layer is placed beyond what CBDT's small metrics can state"));
    };
    let Some(advance) = rounded(advance * pixels).and_then(|v| u8::try_from(v).ok()) else {
        return left_out(format!("its advance width at {ppem} pixels per em is beyond what CBDT's small metrics can state"));
    };
    let k = f64::from(units_per_em) / f64::from(ppem);
    let off = |v: f64, want: f64| (v - want).abs() > SCALE_TOLERANCE * k;
    if off(a, k) || off(d, k) || off(b, 0.0) || off(c, 0.0) {
        warnings.push(format!(
            "glyph `{}`: the image of its iColor {ppem} layer is scaled, rotated or flipped on the layer; CBDT draws it unscaled at {ppem} pixels per em",
            g.glyph
        ));
    }
    Some(Record { gid, width, height, bearing_x, bearing_y, advance, png })
}

/// `v` rounded, when it is finite and well within `i64`.
fn rounded(v: f64) -> Option<i64> {
    let r = v.round();
    if r.is_finite() && r.abs() <= f64::from(i32::MAX) {
        // In range and integral, so the cast is exact.
        Some(r as i64)
    } else {
        None
    }
}

fn table_error(message: String) -> CompileError {
    CompileError::Table { table: "CBDT", message }
}

/// The runs of consecutive glyph ids in `records`, which is sorted without repeats.
fn runs(records: &[Record]) -> Vec<&[Record]> {
    let mut out = Vec::new();
    let mut rest = records;
    while !rest.is_empty() {
        let len = 1 + rest.windows(2).take_while(|w| matches!(w, [p, n] if u32::from(p.gid) + 1 == u32::from(n.gid))).count();
        let Some((run, tail)) = rest.split_at_checked(len) else { break };
        out.push(run);
        rest = tail;
    }
    out
}

/// The line metrics of a strike: ascender, descender and widest advance, then zeros.
fn line_metrics(records: &[Record]) -> [u8; 12] {
    let top = records.iter().map(|r| i32::from(r.bearing_y)).max().unwrap_or(0).clamp(0, 127);
    let bottom = records.iter().map(|r| i32::from(r.bearing_y) - i32::from(r.height)).min().unwrap_or(0).clamp(-128, 0);
    let width_max = records.iter().map(|r| r.advance).max().unwrap_or(0);
    // Clamped to the i8 range above, so the casts are exact.
    let [top] = (top as i8).to_be_bytes();
    let [bottom] = (bottom as i8).to_be_bytes();
    [top, bottom, width_max, 0, 0, 0, 0, 0, 0, 0, 0, 0]
}

/// The serialised tables.
fn write(strikes: &[Strike]) -> Result<CbdtOutput, CompileError> {
    let too_large = || table_error(format!("the images take more than {MAX_TABLE_BYTES} bytes"));
    let offset = |v: usize| u32::try_from(v).map_err(|_| too_large());

    // CBDT: every record's offset from the start of the table, by strike.
    let mut starts: Vec<Vec<usize>> = Vec::with_capacity(strikes.len());
    let mut cbdt_len = 4usize;
    for s in strikes {
        let mut at = Vec::with_capacity(s.records.len());
        for r in &s.records {
            at.push(cbdt_len);
            cbdt_len = r.len().and_then(|l| cbdt_len.checked_add(l)).filter(|t| *t <= MAX_TABLE_BYTES).ok_or_else(too_large)?;
        }
        starts.push(at);
    }
    let mut cbdt = Vec::with_capacity(cbdt_len);
    cbdt.extend(3u16.to_be_bytes());
    cbdt.extend(0u16.to_be_bytes());
    for s in strikes {
        for r in &s.records {
            let ([bearing_x], [bearing_y]) = (r.bearing_x.to_be_bytes(), r.bearing_y.to_be_bytes());
            cbdt.extend([r.height, r.width, bearing_x, bearing_y, r.advance]);
            cbdt.extend(offset(r.png.len())?.to_be_bytes());
            cbdt.extend_from_slice(&r.png);
        }
    }

    // CBLC: each strike's index subtable array offset and the length of its array and subtables.
    let header_len = strikes.len().checked_mul(BITMAP_SIZE_LEN).and_then(|n| n.checked_add(8)).ok_or_else(too_large)?;
    let strike_runs: Vec<Vec<&[Record]>> = strikes.iter().map(|s| runs(&s.records)).collect();
    let subtable_len = |run: &[Record]| run.len().checked_add(1).and_then(|n| n.checked_mul(4)).and_then(|n| n.checked_add(8));
    let mut placed = Vec::with_capacity(strikes.len());
    let mut at = header_len;
    for runs in &strike_runs {
        let mut len = runs.len().checked_mul(8).ok_or_else(too_large)?;
        for run in runs {
            len = subtable_len(run).and_then(|l| len.checked_add(l)).ok_or_else(too_large)?;
        }
        placed.push((at, len));
        at = at.checked_add(len).filter(|t| *t <= MAX_TABLE_BYTES).ok_or_else(too_large)?;
    }

    let mut cblc = Vec::with_capacity(at);
    cblc.extend(3u16.to_be_bytes());
    cblc.extend(0u16.to_be_bytes());
    cblc.extend(offset(strikes.len())?.to_be_bytes());
    for ((s, runs), (array_at, len)) in strikes.iter().zip(&strike_runs).zip(&placed) {
        let (Some(first), Some(last)) = (s.records.first(), s.records.last()) else {
            return Err(table_error("a strike has no glyphs".into()));
        };
        cblc.extend(offset(*array_at)?.to_be_bytes());
        cblc.extend(offset(*len)?.to_be_bytes());
        cblc.extend(offset(runs.len())?.to_be_bytes());
        // colorRef
        cblc.extend(0u32.to_be_bytes());
        let metrics = line_metrics(&s.records);
        cblc.extend(metrics);
        cblc.extend(metrics);
        cblc.extend(first.gid.to_be_bytes());
        cblc.extend(last.gid.to_be_bytes());
        // ppemX, ppemY, bitDepth 32, flags: horizontal metrics.
        cblc.extend([s.ppem, s.ppem, 32, 1]);
    }
    for (runs, at) in strike_runs.iter().zip(&starts) {
        let mut subtable_at = runs.len().checked_mul(8).ok_or_else(too_large)?;
        for run in runs {
            let (Some(first), Some(last)) = (run.first(), run.last()) else { continue };
            cblc.extend(first.gid.to_be_bytes());
            cblc.extend(last.gid.to_be_bytes());
            cblc.extend(offset(subtable_at)?.to_be_bytes());
            subtable_at = subtable_len(run).and_then(|l| subtable_at.checked_add(l)).ok_or_else(too_large)?;
        }
        let mut index = 0usize;
        for run in runs {
            let starts =
                at.get(index..index.saturating_add(run.len())).ok_or_else(|| table_error("the record offsets do not match the strike".into()))?;
            index = index.saturating_add(run.len());
            let (Some(base), Some(last), Some(last_start)) = (starts.first(), run.last(), starts.last()) else {
                continue;
            };
            let end = last.len().and_then(|l| last_start.checked_add(l)).ok_or_else(too_large)?;
            cblc.extend(1u16.to_be_bytes());
            cblc.extend(IMAGE_FORMAT.to_be_bytes());
            cblc.extend(offset(*base)?.to_be_bytes());
            for start in starts.iter().chain([&end]) {
                cblc.extend(offset(start.saturating_sub(*base))?.to_be_bytes());
            }
        }
    }
    Ok(CbdtOutput { cbdt, cblc })
}

#[cfg(test)]
mod tests {
    use fontcraft_color::layers::with_kind;
    use fontcraft_color::strike;
    use fontcraft_color::{ColorLayerKind, MemoryAssets};
    use fontcraft_model::{Font, Image, Layer, LayerAttrs, ResolvedGlyph, Transform};
    use read_fonts::tables::bitmap::{BitmapContent, BitmapDataFormat, BitmapMetrics};
    use read_fonts::tables::cbdt::Cbdt;
    use read_fonts::tables::cblc::Cblc;
    use read_fonts::types::GlyphId;
    use read_fonts::{FontData, FontRead};

    use super::*;

    /// The PNG signature and the start of an IHDR chunk: enough to be recognised and stored as
    /// read, and easy to write in the script that made the expected bytes.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend(13u32.to_be_bytes());
        b.extend(b"IHDR");
        b.extend(width.to_be_bytes());
        b.extend(height.to_be_bytes());
        b
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn icolor(master: &str, ppem: u16, path: &str, pos: (f64, f64), scale: f64) -> Layer {
        let mut l = Layer::new(master, 600.0);
        l.id = format!("{path} layer");
        l.info.attrs = with_kind(&LayerAttrs::default(), Some(ColorLayerKind::IColor(ppem)));
        let transform = Transform::Decomposed { pos, scale: (scale, scale), angle: 0.0, slant: (0.0, 0.0) };
        l.info.image = Some(Image { path: Some(path.into()), transform, ..Image::default() });
        l
    }

    fn order_of(names: &[&str]) -> Vec<ResolvedGlyph> {
        names.iter().map(|n| ResolvedGlyph::new(*n, 600.0)).collect()
    }

    // Compiled by fontTools 4 from the CBLC and CBDT objects nanoemoji's bitmap_tables.py makes
    // (version 3, index subtable format 1, glyph bitmap format 17), holding the same PNG bytes,
    // metrics and glyph runs: a 64 ppem strike with glyphs 1 and 2 in one index subtable and
    // glyph 4 in a second, and a 128 ppem strike with glyph 1.
    const CBDT_HEX: &str = "000300004040033a260000001889504e470d0a1a0a0000000d49484452000000400000004040280040260000001889504e470d0a1a0a0000000d4948445200000028000000402020062d260000001889504e470d0a1a0a0000000d494844520000002000000020608006534d0000001889504e470d0a1a0a0000000d494844520000008000000060";
    const CBLC_HEX: &str = "00030000000000020000006800000034000000020000000040fa2600000000000000000040fa2600000000000000000000010004404020010000009c00000018000000010000000053f34d00000000000000000053f34d000000000000000000000100018080200100010002000000100004000400000024000100110000000400000000000000210000004200010011000000460000000000000021000100010000000800010011000000670000000000000021";

    #[test]
    fn strikes_match_the_oracle_bytes_and_leave_out_what_cbdt_cannot_hold() {
        let mut font = Font::new("Bitmap Sans");
        let master = font.masters.first().unwrap().id.clone();
        let (k64, k128) = (1000.0 / 64.0, 1000.0 / 128.0);
        let mut assets = MemoryAssets::new();
        assets.insert("A 64.png", png(64, 64));
        assets.insert("A 128.png", png(128, 96));
        assets.insert("A 300.png", png(16, 16));
        assets.insert("B 64.png", png(40, 64));
        assets.insert("D 64.png", png(32, 32));
        assets.insert("E 64.png", png(256, 16));
        for (glyph, layers) in [
            (
                "A",
                vec![
                    icolor(&master, 64, "A 64.png", (50.0, -100.0), k64),
                    icolor(&master, 128, "A 128.png", (50.0, -100.0), k128),
                    icolor(&master, 300, "A 300.png", (0.0, 0.0), 1000.0 / 300.0),
                ],
            ),
            ("B", vec![icolor(&master, 64, "B 64.png", (0.0, 0.0), k64)]),
            // No file: left out, which splits the 64 ppem strike into two runs.
            ("C", vec![icolor(&master, 64, "C 64.png", (0.0, 0.0), k64)]),
            ("D", vec![icolor(&master, 64, "D 64.png", (100.0, 200.0), k64)]),
            ("E", vec![icolor(&master, 64, "E 64.png", (0.0, 0.0), k64)]),
        ] {
            font.glyph_mut(glyph).unwrap().layers.extend(layers);
        }
        let plan = CbdtPlan { strikes: strike::strikes(&font, &master) };
        let glyphs = order_of(&[".notdef", "A", "B", "C", "D", "E"]);
        let order = GlyphOrder::new(&glyphs, &[]).unwrap();
        let mut warnings = Vec::new();
        let out = build(&plan, &order, &assets, 1000, &mut warnings).unwrap();

        assert_eq!(out.cbdt, hex(CBDT_HEX), "CBDT");
        assert_eq!(out.cblc, hex(CBLC_HEX), "CBLC");

        // Read back: glyph 4 of the 64 ppem strike sits 100 and 200 units from the origin.
        let cblc = Cblc::read(FontData::new(&out.cblc)).unwrap();
        let cbdt = Cbdt::read(FontData::new(&out.cbdt)).unwrap();
        let size = &cblc.bitmap_sizes()[0];
        assert_eq!((size.ppem_x(), size.hori.ascender(), size.hori.descender(), size.hori.width_max()), (64, 64, -6, 38));
        let data = cbdt.data(&size.location(cblc.offset_data(), GlyphId::new(4)).unwrap()).unwrap();
        let BitmapMetrics::Small(m) = data.metrics else { panic!("small metrics") };
        assert_eq!((m.width(), m.height(), m.bearing_x(), m.bearing_y(), m.advance()), (32, 32, 6, 45, 38));
        let BitmapContent::Data(BitmapDataFormat::Png, bytes) = data.content else { panic!("PNG data") };
        assert_eq!(bytes, png(32, 32).as_slice());

        assert_eq!(warnings.len(), 3, "{warnings:#?}");
        let has = |text: &str| warnings.iter().any(|w| w.contains(text));
        assert!(has("glyph `C`: the image of its iColor 64 layer cannot be read"), "{warnings:#?}");
        assert!(has("glyph `E`: the image `E 64.png` is 256 × 16 pixels"), "{warnings:#?}");
        assert!(has("the iColor 300 strike is left out of CBDT"), "{warnings:#?}");
    }

    #[test]
    fn tiff_is_converted_and_unusable_placements_fail_with_messages() {
        let tiff_pixels: Vec<u8> = (0..16).map(|i| i * 15).collect();
        let mut assets = MemoryAssets::new();
        assets.insert("t.tif", super::super::sbix::tests::tiff(2, 2, &tiff_pixels));
        assets.insert("far.png", png(4, 4));
        let glyphs = order_of(&[".notdef", "t", "far"]);
        let order = GlyphOrder::new(&glyphs, &[]).unwrap();
        let glyph = |name: &str, image: &str, pos: (f64, f64), scale: f64| StrikeGlyph {
            glyph: name.into(),
            layer: "l".into(),
            image: image.into(),
            placement: Transform::Decomposed { pos, scale: (scale, scale), angle: 0.0, slant: (0.0, 0.0) },
        };
        let plan = CbdtPlan {
            strikes: vec![fontcraft_color::Strike {
                ppem: 100,
                // `t` at twice its iColor scale: exported unscaled, with a warning.
                glyphs: vec![glyph("t", "t.tif", (0.0, 0.0), 20.0), glyph("far", "far.png", (1.0e9, 0.0), 10.0)],
            }],
        };

        assert!(matches!(build(&plan, &order, &assets, 0, &mut Vec::new()), Err(CompileError::Table { table: "CBDT", .. })));

        let mut warnings = Vec::new();
        let out = build(&plan, &order, &assets, 1000, &mut warnings).unwrap();
        let cblc = Cblc::read(FontData::new(&out.cblc)).unwrap();
        let cbdt = Cbdt::read(FontData::new(&out.cbdt)).unwrap();
        let size = &cblc.bitmap_sizes()[0];
        assert_eq!((size.start_glyph_index().to_u32(), size.end_glyph_index().to_u32()), (1, 1));
        let data = cbdt.data(&size.location(cblc.offset_data(), GlyphId::new(1)).unwrap()).unwrap();
        let BitmapContent::Data(BitmapDataFormat::Png, bytes) = data.content else { panic!("PNG data") };
        assert_eq!(decode_rgba(bytes, MAX_PIXELS).unwrap().pixels, tiff_pixels, "TIFF converted to PNG");
        assert!(warnings.iter().any(|w| w.contains("glyph `t`") && w.contains("scaled, rotated or flipped")), "{warnings:#?}");
        assert!(warnings.iter().any(|w| w.contains("glyph `far`") && w.contains("beyond what CBDT's small metrics")), "{warnings:#?}");

        // Every glyph left out: empty tables with a warning.
        let only_far = CbdtPlan { strikes: vec![fontcraft_color::Strike { ppem: 100, glyphs: vec![glyph("far", "far.png", (1.0e9, 0.0), 10.0)] }] };
        let mut warnings = Vec::new();
        let out = build(&only_far, &order, &assets, 1000, &mut warnings).unwrap();
        assert_eq!(Cblc::read(FontData::new(&out.cblc)).unwrap().num_sizes(), 0);
        assert_eq!(out.cbdt, vec![0, 3, 0, 0]);
        assert!(warnings.iter().any(|w| w.contains("no strikes")), "{warnings:#?}");
    }
}

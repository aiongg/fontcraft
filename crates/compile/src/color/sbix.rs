//! `sbix` (M11.8): version 1, flags with bit 0 set and the draw-outlines bit clear, one strike
//! per iColor size at 72 pixels per inch. PNG and JPEG data stored as read (`png `, `jpg `),
//! TIFF converted to PNG. Serialised here and added raw, because write-fonts' strikes hold only
//! offsets.
//!
//! Rows: `sbix`, `color.sbix.bitmapFormats`, `color.export.sbixAutomatic`,
//! `color.sbix.sameAspectRatio`, `color.sbix.sizeScalesImage`, `image.ignoredAtExport`.
//!
//! Layout (OpenType `sbix`): a header (version, flags, strike count, strike offsets from the
//! start of the table), then each strike (ppem, ppi, `numGlyphs + 1` glyph data offsets from the
//! start of the strike), then each glyph's record (origin offsets in pixels, graphic type, the
//! image bytes). A glyph without an image in a strike has two equal offsets.
//!
//! The origin offsets place the bottom-left corner of the image's box on the layer, measured in
//! the strike's pixels from the glyph origin. The image is drawn at one pixel per
//! `units_per_em ÷ ppem` units, which is how an iColor layer shows it; an image scaled, rotated
//! or flipped on its layer is exported unscaled, with a warning.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_color::export::SbixPlan;
use fontcraft_color::strike::{self, SBIX_PPI, StrikeGlyph};
use fontcraft_color::{AssetStore, MAX_STRIKES};
use fontcraft_imagery::{ImageFormat, MAX_PIXELS, decode_rgba, sniff, to_png};

use super::GlyphOrder;
use crate::CompileError;

/// Largest `sbix` table written. Its offsets are 32-bit; this keeps a font that holds it well
/// within what renderers load.
pub const MAX_TABLE_BYTES: usize = 1 << 30;

/// How far an image's scale may be from `units_per_em ÷ ppem`, as a fraction of it, before
/// export warns that the image is not drawn as placed.
const SCALE_TOLERANCE: f64 = 0.01;

/// One glyph's record in a strike.
struct Record {
    gid: u16,
    x: i16,
    y: i16,
    tag: [u8; 4],
    data: Arc<[u8]>,
}

/// The table. Origin offsets in pixels come from each image's placement and `units_per_em`. A
/// missing or unreadable image leaves the glyph out of its strike, with a warning.
pub fn build(
    plan: &SbixPlan,
    order: &GlyphOrder,
    assets: &dyn AssetStore,
    units_per_em: u16,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>, CompileError> {
    if units_per_em == 0 {
        return Err(table_error("the font has 0 units per em".into()));
    }
    let num_glyphs = order.glyphs.len();
    let mut sizes: HashMap<String, (u32, u32)> = HashMap::new();
    let mut strikes: Vec<(u16, Vec<Record>)> = Vec::new();
    for s in plan.strikes.iter().take(MAX_STRIKES) {
        let mut records = Vec::new();
        for g in &s.glyphs {
            let Some(gid) = order.gid(&g.glyph) else { continue };
            if let Some((record, size)) = record(g, gid, s.ppem, units_per_em, assets, warnings) {
                sizes.insert(g.image.clone(), size);
                records.push(record);
            }
        }
        records.sort_by_key(|r| r.gid);
        records.dedup_by_key(|r| r.gid);
        if !records.is_empty() {
            strikes.push((s.ppem, records));
        }
    }
    for p in strike::aspect_problems(&plan.strikes, &|path| sizes.get(path).copied()) {
        warnings.push(match &p.glyph {
            Some(glyph) => format!("glyph `{glyph}`: {}", p.message),
            None => p.message,
        });
    }
    if strikes.is_empty() && !plan.strikes.is_empty() {
        warnings.push("no iColor image could be exported, so the sbix table has no strikes".into());
    }
    write(&strikes, num_glyphs)
}

/// The record of `g` in the `ppem` strike and the image's pixel size, or `None` with a warning
/// when the image cannot be used.
fn record(
    g: &StrikeGlyph,
    gid: u16,
    ppem: u16,
    units_per_em: u16,
    assets: &dyn AssetStore,
    warnings: &mut Vec<String>,
) -> Option<(Record, (u32, u32))> {
    let mut left_out = |why: String| {
        warnings.push(format!("glyph `{}`: {why}; it is left out of the {ppem} pixel sbix strike", g.glyph));
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
    let (tag, data) = match info.format {
        ImageFormat::Png => (*b"png ", bytes),
        ImageFormat::Jpeg => (*b"jpg ", bytes),
        // Renderers do not draw `tiff` graphics reliably, so TIFF goes in as PNG.
        ImageFormat::Tiff => match decode_rgba(&bytes, MAX_PIXELS).and_then(|image| to_png(&image)) {
            Ok(png) => (*b"png ", Arc::from(png)),
            Err(e) => return left_out(format!("the TIFF image `{}` cannot be converted to PNG: {e}", g.image)),
        },
        other => {
            return left_out(format!("`{}` is {}; sbix takes PNG, JPEG and TIFF images", g.image, format_name(other)));
        }
    };
    let (w, h) = (f64::from(info.width), f64::from(info.height));
    let [a, b, c, d, e, f] = g.placement.affine().as_coeffs();
    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let min_x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let pixels = f64::from(ppem) / f64::from(units_per_em);
    let (Some(x), Some(y)) = (to_i16(min_x * pixels), to_i16(min_y * pixels)) else {
        return left_out(format!("the image of its iColor {ppem} layer is placed beyond what sbix can state"));
    };
    let k = f64::from(units_per_em) / f64::from(ppem);
    let off = |v: f64, want: f64| (v - want).abs() > SCALE_TOLERANCE * k;
    if off(a, k) || off(d, k) || off(b, 0.0) || off(c, 0.0) {
        warnings.push(format!(
            "glyph `{}`: the image of its iColor {ppem} layer is scaled, rotated or flipped on the layer; sbix draws it unscaled at {ppem} pixels per em",
            g.glyph
        ));
    }
    Some((Record { gid, x, y, tag, data }, (info.width, info.height)))
}

/// `v` rounded, when it is finite and fits an `i16`.
fn to_i16(v: f64) -> Option<i16> {
    let r = v.round();
    if r.is_finite() && (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&r) {
        // In range and integral, so the cast is exact.
        Some(r as i16)
    } else {
        None
    }
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "a PNG image",
        ImageFormat::Jpeg => "a JPEG image",
        ImageFormat::Tiff => "a TIFF image",
        ImageFormat::Pdf => "a PDF file",
        ImageFormat::Psd => "a PSD file",
        ImageFormat::Svg => "an SVG file",
        ImageFormat::Other => "not an image file this app reads",
    }
}

fn table_error(message: String) -> CompileError {
    CompileError::Table { table: "sbix", message }
}

/// The serialised table: the header, then each strike with its offsets and records.
fn write(strikes: &[(u16, Vec<Record>)], num_glyphs: usize) -> Result<Vec<u8>, CompileError> {
    let too_large = || table_error(format!("the images take more than {MAX_TABLE_BYTES} bytes"));
    let offsets_len = num_glyphs.checked_add(1).and_then(|n| n.checked_mul(4)).ok_or_else(too_large)?;
    let header_len = strikes.len().checked_mul(4).and_then(|n| n.checked_add(8)).ok_or_else(too_large)?;
    let mut strike_lens = Vec::with_capacity(strikes.len());
    let mut total = header_len;
    for (_, records) in strikes {
        let mut len = offsets_len.checked_add(4).ok_or_else(too_large)?;
        for r in records {
            len = len.checked_add(8).and_then(|l| l.checked_add(r.data.len())).ok_or_else(too_large)?;
        }
        total = total.checked_add(len).filter(|t| *t <= MAX_TABLE_BYTES).ok_or_else(too_large)?;
        strike_lens.push(len);
    }
    let offset = |v: usize| u32::try_from(v).map_err(|_| too_large());

    let mut out = Vec::with_capacity(total);
    out.extend(1u16.to_be_bytes());
    // Bit 0 is always set; bit 1 clear: the outlines are not drawn over the bitmaps.
    out.extend(1u16.to_be_bytes());
    out.extend(offset(strikes.len())?.to_be_bytes());
    let mut at = header_len;
    for len in &strike_lens {
        out.extend(offset(at)?.to_be_bytes());
        at = at.checked_add(*len).ok_or_else(too_large)?;
    }
    for (ppem, records) in strikes {
        out.extend(ppem.to_be_bytes());
        out.extend(SBIX_PPI.to_be_bytes());
        let mut next = records.iter().peekable();
        let mut cursor = offsets_len.checked_add(4).ok_or_else(too_large)?;
        for gid in 0..num_glyphs {
            out.extend(offset(cursor)?.to_be_bytes());
            if let Some(r) = next.next_if(|r| usize::from(r.gid) == gid) {
                cursor = cursor.checked_add(8 + r.data.len()).ok_or_else(too_large)?;
            }
        }
        out.extend(offset(cursor)?.to_be_bytes());
        for r in records {
            out.extend(r.x.to_be_bytes());
            out.extend(r.y.to_be_bytes());
            out.extend(r.tag);
            out.extend_from_slice(&r.data);
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use fontcraft_color::layers::with_kind;
    use fontcraft_color::{ColorLayerKind, MemoryAssets};
    use fontcraft_imagery::Rgba8;
    use fontcraft_model::{Font, Image, Layer, LayerAttrs, ResolvedGlyph, Transform};
    use read_fonts::FontData;
    use read_fonts::tables::sbix::Sbix;
    use read_fonts::types::{GlyphId, Tag};

    use super::*;

    fn png(width: u32, height: u32, value: u8) -> Vec<u8> {
        to_png(&Rgba8 { width, height, pixels: vec![value; (width * height * 4) as usize] }).unwrap()
    }

    /// The header and frame header of a 16 × 16 baseline JPEG: enough to be recognised and
    /// stored as read.
    fn jpeg() -> Vec<u8> {
        vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0, 16, 0, 16, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 0xFF, 0xD9]
    }

    /// An uncompressed little-endian RGBA TIFF, one strip.
    pub(crate) fn tiff(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        // Header 8 bytes, a directory of 10 entries (2 + 120 + 4), then the bits per sample at
        // 134 and the pixels at 142.
        let entries: [(u16, u16, u32, u32); 10] = [
            (256, 4, 1, width),
            (257, 4, 1, height),
            (258, 3, 4, 134),
            (259, 3, 1, 1),
            (262, 3, 1, 2),
            (273, 4, 1, 142),
            (277, 3, 1, 4),
            (278, 4, 1, height),
            (279, 4, 1, pixels.len() as u32),
            (338, 3, 1, 2),
        ];
        let mut b = b"II*\0".to_vec();
        b.extend(8u32.to_le_bytes());
        b.extend((entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            b.extend(tag.to_le_bytes());
            b.extend(kind.to_le_bytes());
            b.extend(count.to_le_bytes());
            b.extend(value.to_le_bytes());
        }
        b.extend(0u32.to_le_bytes());
        assert_eq!(b.len(), 134);
        for _ in 0..4 {
            b.extend(8u16.to_le_bytes());
        }
        b.extend_from_slice(pixels);
        b
    }

    fn icolor(master: &str, ppem: u16, path: &str, pos: (f64, f64), scale: f64) -> Layer {
        let mut l = Layer::new(master, 600.0);
        l.id = format!("{path} layer");
        l.info.attrs = with_kind(&LayerAttrs::default(), Some(ColorLayerKind::IColor(ppem)));
        let transform = Transform::Decomposed { pos, scale: (scale, scale), angle: 0.0, slant: (0.0, 0.0) };
        l.info.image = Some(Image { path: Some(path.into()), transform, ..Image::default() });
        l
    }

    #[test]
    fn two_strikes_store_png_and_jpeg_as_read_and_convert_tiff() {
        let mut font = Font::new("Bitmap Sans");
        let master = font.masters.first().unwrap().id.clone();
        let (k64, k128) = (1000.0 / 64.0, 1000.0 / 128.0);
        let tiff_pixels: Vec<u8> = (0..16).map(|i| i * 15).collect();
        let mut assets = MemoryAssets::new();
        assets.insert("A 64.png", png(64, 64, 200));
        assets.insert("A 128.png", png(128, 96, 100));
        assets.insert("B 64.jpg", jpeg());
        assets.insert("C 64.tif", tiff(2, 2, &tiff_pixels));
        assets.insert("E 64.pdf", b"%PDF-1.7\n".to_vec());
        for (glyph, layers) in [
            ("A", vec![icolor(&master, 64, "A 64.png", (50.0, -100.0), k64), icolor(&master, 128, "A 128.png", (50.0, -100.0), k128)]),
            ("B", vec![icolor(&master, 64, "B 64.jpg", (0.0, 0.0), k64)]),
            // Placed at twice its iColor scale: exported unscaled, with a warning.
            ("C", vec![icolor(&master, 64, "C 64.tif", (0.0, 0.0), 2.0 * k64)]),
            ("D", vec![icolor(&master, 64, "D 64.png", (0.0, 0.0), k64)]),
            ("E", vec![icolor(&master, 64, "E 64.pdf", (0.0, 0.0), k64)]),
        ] {
            font.glyph_mut(glyph).unwrap().layers.extend(layers);
        }
        let plan = SbixPlan { strikes: strike::strikes(&font, &master) };
        let glyphs: Vec<ResolvedGlyph> = [".notdef", "A", "B", "C", "D", "E"].into_iter().map(|n| ResolvedGlyph::new(n, 600.0)).collect();
        let order = GlyphOrder::new(&glyphs, &[]).unwrap();
        let mut warnings = Vec::new();
        let bytes = build(&plan, &order, &assets, 1000, &mut warnings).unwrap();

        let sbix = Sbix::read(FontData::new(&bytes), 6).unwrap();
        assert_eq!((sbix.version(), sbix.flags().bits(), sbix.num_strikes()), (1, 1, 2));
        let s64 = sbix.strikes().get(0).unwrap();
        let s128 = sbix.strikes().get(1).unwrap();
        assert_eq!((s64.ppem(), s64.ppi(), s128.ppem(), s128.ppi()), (64, 72, 128, 72));

        let a = s64.glyph_data(GlyphId::new(1)).unwrap().unwrap();
        assert_eq!((a.origin_offset_x(), a.origin_offset_y(), a.graphic_type()), (3, -6, Tag::new(b"png ")));
        assert_eq!(a.data(), png(64, 64, 200).as_slice(), "PNG stored as read");
        let b = s64.glyph_data(GlyphId::new(2)).unwrap().unwrap();
        assert_eq!((b.origin_offset_x(), b.origin_offset_y(), b.graphic_type()), (0, 0, Tag::new(b"jpg ")));
        assert_eq!(b.data(), jpeg().as_slice(), "JPEG stored as read");
        let c = s64.glyph_data(GlyphId::new(3)).unwrap().unwrap();
        assert_eq!(c.graphic_type(), Tag::new(b"png "), "TIFF converted to PNG");
        assert_eq!(decode_rgba(c.data(), MAX_PIXELS).unwrap().pixels, tiff_pixels);
        for gid in [0, 4, 5] {
            assert!(s64.glyph_data(GlyphId::new(gid)).unwrap().is_none(), "glyph {gid} has no image at 64");
        }
        let a128 = s128.glyph_data(GlyphId::new(1)).unwrap().unwrap();
        assert_eq!((a128.origin_offset_x(), a128.origin_offset_y()), (6, -13), "50 and -100 units at 128 pixels per 1000");
        assert!(s128.glyph_data(GlyphId::new(2)).unwrap().is_none());

        assert_eq!(warnings.len(), 4, "{warnings:#?}");
        let has = |glyph: &str, text: &str| warnings.iter().any(|w| w.starts_with(&format!("glyph `{glyph}`")) && w.contains(text));
        assert!(has("D", "cannot be read"), "{warnings:#?}");
        assert!(has("D", "left out of the 64 pixel sbix strike"), "{warnings:#?}");
        assert!(has("E", "a PDF file"), "{warnings:#?}");
        assert!(has("C", "scaled, rotated or flipped"), "{warnings:#?}");
        assert!(has("A", "differ in width ÷ height"), "{warnings:#?}");
    }

    #[test]
    fn unusable_sizes_and_placements_fail_with_messages() {
        let mut assets = MemoryAssets::new();
        assets.insert("far.png", png(4, 4, 9));
        let glyphs: Vec<ResolvedGlyph> = [".notdef", "far"].into_iter().map(|n| ResolvedGlyph::new(n, 600.0)).collect();
        let order = GlyphOrder::new(&glyphs, &[]).unwrap();
        let far = StrikeGlyph {
            glyph: "far".into(),
            layer: "l".into(),
            image: "far.png".into(),
            placement: Transform::Decomposed { pos: (1.0e9, 0.0), scale: (1.0, 1.0), angle: 0.0, slant: (0.0, 0.0) },
        };
        let plan = SbixPlan { strikes: vec![fontcraft_color::Strike { ppem: 1000, glyphs: vec![far] }] };

        assert!(matches!(build(&plan, &order, &assets, 0, &mut Vec::new()), Err(CompileError::Table { table: "sbix", .. })));

        let mut warnings = Vec::new();
        let bytes = build(&plan, &order, &assets, 1000, &mut warnings).unwrap();
        assert!(warnings.iter().any(|w| w.contains("beyond what sbix can state")), "{warnings:#?}");
        assert!(warnings.iter().any(|w| w.contains("no strikes")), "{warnings:#?}");
        assert_eq!(Sbix::read(FontData::new(&bytes), 2).unwrap().num_strikes(), 0);
    }
}

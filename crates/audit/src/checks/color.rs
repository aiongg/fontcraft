//! Colour checks (M12.56). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.
//!
//! The three checks follow FontBakery 1.1.0 (Apache-2.0): `color_cpal_brightness`,
//! `empty_glyph_on_gid1_for_colrv0` and `googlefonts/colorfont_tables`.

use std::collections::HashMap;

use read_fonts::tables::colr::Layer;
use read_fonts::tables::cpal::ColorRecord;
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, TableProvider};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};

use super::components::Names;
use super::kerning::glyph_label;
use crate::{BinaryCtx, Hit, Location, Params, Sink, Threshold, num};

/// The palette index that stands for the text colour.
const FOREGROUND: u16 = 0xFFFF;
/// Most base glyph records read.
const MAX_BASE_GLYPHS: usize = 65_536;
/// Most COLR layers `paletteBrightness` measures in one font.
const MAX_LAYERS_MEASURED: usize = 1_000_000;
/// Segments each curve is split into when glyph 1's area is measured.
const CURVE_STEPS: u32 = 16;
/// Glyph 1 counts as empty below this area, in square units.
const EMPTY_AREA: f64 = 0.01;

pub(crate) const MIN_BRIGHTNESS: Threshold = Threshold {
    name: "minBrightness",
    default: 0.1,
    min: 0.0,
    max: 0.5,
    doc: "Share of full brightness a fixed layer colour must keep from black and from white.",
};

fn table_place(tag: &str) -> Location {
    Location { table: Some(tag.to_string()), ..Location::default() }
}

/// The COLR table's version, when the font has a readable one.
fn colr_version(font: &FontRef<'_>) -> Option<u16> {
    font.colr().ok().map(|c| c.version())
}

/// The fixed colour of `layers` farthest from mid-grey among those too dark or too bright, with
/// its palette index.
fn worst_layer(layers: &[Layer], colors: &[ColorRecord], start: usize, entries: u16, margin: f64) -> Option<(f64, u16)> {
    let mut worst: Option<(f64, u16)> = None;
    for layer in layers {
        let index = layer.palette_index();
        if index == FOREGROUND || index >= entries {
            continue;
        }
        let Some(c) = colors.get(start.saturating_add(usize::from(index))) else { continue };
        let brightness = (f64::from(c.red()) * 299.0 + f64::from(c.green()) * 587.0 + f64::from(c.blue()) * 114.0) / 1000.0;
        if brightness >= margin && brightness <= 256.0 - margin {
            continue;
        }
        // The colour farthest from mid-grey is the one the message names.
        if worst.is_none_or(|(b, _)| (brightness - 128.0).abs() > (b - 128.0).abs()) {
            worst = Some((brightness, index));
        }
    }
    worst
}

/// `check.color.paletteBrightness`: a COLR version 0 layer with a fixed colour of the first
/// palette whose perceived brightness (0.299 red, 0.587 green, 0.114 blue) is within
/// `minBrightness` of black or of white. One finding per base glyph.
pub(crate) fn palette_brightness(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let Ok(colr) = font.colr() else { return };
    if colr.version() != 0 {
        return;
    }
    let Ok(cpal) = font.cpal() else { return };
    let Some(Ok(colors)) = cpal.color_records_array() else { return };
    let Some(start) = cpal.color_record_indices().first().map(|i| usize::from(i.get())) else { return };
    let entries = cpal.num_palette_entries();
    let (Some(Ok(bases)), Some(Ok(layers))) = (colr.base_glyph_records(), colr.layer_records()) else { return };
    let margin = params.get(&MIN_BRIGHTNESS) * 256.0;
    let names = Names::new(font);
    // Base glyphs may share a layer range: each range is measured once, and the layers measured
    // in all are capped.
    let mut measured: HashMap<(usize, usize), Option<(f64, u16)>> = HashMap::new();
    let mut budget = MAX_LAYERS_MEASURED;
    for base in bases.iter().take(MAX_BASE_GLYPHS) {
        let first = usize::from(base.first_layer_index());
        let range = (first, usize::from(base.num_layers()));
        let Some(own) = layers.get(first..first.saturating_add(range.1)) else { continue };
        let worst = if let Some(known) = measured.get(&range) {
            *known
        } else {
            let Some(left) = budget.checked_sub(own.len()) else {
                sink.skip(format!(
                    "the COLR table has more than {MAX_LAYERS_MEASURED} layers to measure; the base glyphs past that were not checked"
                ));
                break;
            };
            budget = left;
            let worst = worst_layer(own, &colors, start, entries, margin);
            measured.insert(range, worst);
            worst
        };
        let Some((brightness, index)) = worst else { continue };
        let glyph = glyph_label(ctx, &names, base.glyph_id().to_u32());
        let side = if brightness < margin { "dark" } else { "bright" };
        let place = Location { glyph: glyph.clone(), table: Some("COLR".into()), ..Location::default() };
        sink.push(
            Hit::new(
                "glyphs-too-dark-or-too-bright",
                format!(
                    "A colour layer of `{glyph}` uses palette entry {index}, which is too {side} (brightness {} of 255) to read on a {} background; use the text colour or a lighter or darker colour.",
                    num(brightness),
                    if side == "dark" { "dark" } else { "light" }
                ),
            )
            .data("brightness", brightness)
            .data("paletteIndex", f64::from(index))
            .place(place),
        );
    }
}

/// Sums the signed area of what is drawn, with curves split into straight steps.
#[derive(Default)]
struct AreaPen {
    area: f64,
    start: (f64, f64),
    at: (f64, f64),
}

impl AreaPen {
    fn segment(&mut self, to: (f64, f64)) {
        self.area += (self.at.0 * to.1 - to.0 * self.at.1) / 2.0;
        self.at = to;
    }

    fn steps(&mut self, point: impl Fn(f64) -> (f64, f64)) {
        for i in 1..=CURVE_STEPS {
            self.segment(point(f64::from(i) / f64::from(CURVE_STEPS)));
        }
    }
}

impl OutlinePen for AreaPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = (f64::from(x), f64::from(y));
        self.at = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.segment((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.at, (f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
        self.steps(|t| {
            let u = 1.0 - t;
            (u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0, u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1)
        });
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.at, (f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
        self.steps(|t| {
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
        });
    }

    fn close(&mut self) {
        if self.at != self.start {
            self.segment(self.start);
        }
    }
}

/// `check.color.emptyGlyphOneForColrV0`: in a COLR version 0 font, glyph 1 draws something (its
/// outline has an area). A renderer of one common system paints glyph 1 over other glyphs in such
/// fonts, so an empty glyph must take that place.
pub(crate) fn empty_glyph_one_for_colr_v0(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    if colr_version(font) != Some(0) {
        return;
    }
    let Some(outline) = font.outline_glyphs().get(GlyphId::new(1)) else { return };
    let mut pen = AreaPen::default();
    if outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).is_err() {
        return;
    }
    pen.close();
    if !pen.area.is_finite() || pen.area.abs() <= EMPTY_AREA {
        return;
    }
    let names = Names::new(font);
    let glyph = glyph_label(ctx, &names, 1);
    let place = Location { glyph: glyph.clone(), ..Location::default() };
    sink.push(
        Hit::new(
            "gid1-has-contours",
            format!(
                "This is a COLR version 0 font and glyph 1 (`{glyph}`) has contours; put an empty glyph, such as the space, right after .notdef."
            ),
        )
        .data("area", pen.area.abs())
        .place(place),
    );
}

/// `check.color.colorTableSet`: the colour tables do not suit broad delivery.
pub(crate) fn color_table_set(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let has = |tag: &[u8; 4]| font.table_data(Tag::new(tag)).is_some();
    let (colr, svg, variable) = (has(b"COLR"), has(b"SVG "), has(b"fvar"));
    let version = colr_version(font);
    if colr {
        if version == Some(0) && svg {
            sink.push(
                Hit::new("drop-svg", "The font has a COLR version 0 table, which is widely supported, so the SVG table is not needed.")
                    .place(table_place("SVG ")),
            );
        } else if version == Some(1) && !svg && !variable {
            sink.push(
                Hit::new("add-svg", "The font has a COLR version 1 table but no SVG table for the programs that do not read COLR version 1 yet.")
                    .place(table_place("COLR")),
            );
        }
    }
    if svg {
        if variable {
            sink.push(Hit::new("variable-svg", "The font is variable, and SVG glyphs cannot vary; remove the SVG table.").place(table_place("SVG ")));
        }
        if !colr {
            sink.push(Hit::new("add-colr", "The font has only an SVG table; add a COLR table as well.").place(table_place("SVG ")));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::checks::kerning::test_font::*;
    use crate::{BinaryCtx, Config, LayerReport, NoServices, audit_binary, find};

    fn run(bytes: &[u8], id: &str) -> LayerReport {
        let spec = find(id).unwrap_or_else(|| panic!("no check {id}"));
        let ctx = BinaryCtx::new("f0", bytes, 0, &NoServices).unwrap();
        let r = audit_binary(&ctx, &[spec], &Config::default());
        for f in &r.findings {
            assert!(spec.codes.contains(&f.code), "{id} emits unlisted `{}`", f.code);
        }
        r
    }

    fn codes(r: &LayerReport) -> Vec<&'static str> {
        r.findings.iter().map(|f| f.code).collect()
    }

    /// COLR version 0: base glyph 3 with layers 1 and 2 in these palette entries.
    fn colr0(palette_indices: [u16; 2]) -> Vec<u8> {
        let mut w = W::default();
        w.u16(0).u16(1).u32(14).u32(20).u16(2);
        w.u16(3).u16(0).u16(2);
        w.u16(1).u16(palette_indices[0]).u16(2).u16(palette_indices[1]);
        w.0
    }

    /// COLR version 1 header with no records.
    fn colr1() -> Vec<u8> {
        let mut w = W::default();
        w.u16(1).u16(0).u32(0).u32(0).u16(0);
        for _ in 0..5 {
            w.u32(0);
        }
        w.0
    }

    /// CPAL version 0 with one palette of these (red, green, blue) colours.
    fn cpal(colors: &[(u8, u8, u8)]) -> Vec<u8> {
        let mut w = W::default();
        w.u16(0).u16(colors.len() as u16).u16(1).u16(colors.len() as u16).u32(14).u16(0);
        for (r, g, b) in colors {
            w.bytes(&[*b, *g, *r, 255]);
        }
        w.0
    }

    #[test]
    fn layer_colours_near_black_or_white_are_reported() {
        let font =
            |indices| sfnt(vec![(b"maxp", maxp(4)), (b"COLR", colr0(indices)), (b"CPAL", cpal(&[(200, 40, 40), (10, 10, 10), (250, 250, 250)]))]);
        assert!(run(&font([0, 0xFFFF]), "check.color.paletteBrightness").findings.is_empty(), "mid red and the text colour pass");
        let r = run(&font([0, 1]), "check.color.paletteBrightness");
        assert_eq!(codes(&r), ["glyphs-too-dark-or-too-bright"]);
        assert_eq!(r.findings[0].location.glyph, "gid3");
        assert_eq!(r.findings[0].data.get("paletteIndex"), Some(&1.0));
        assert_eq!(codes(&run(&font([2, 0]), "check.color.paletteBrightness")), ["glyphs-too-dark-or-too-bright"]);

        let mut config = Config::default();
        config.set("check.color.paletteBrightness", "minBrightness", 0.0).unwrap();
        let spec = find("check.color.paletteBrightness").unwrap();
        let bytes = font([1, 2]);
        let ctx = BinaryCtx::new("f0", &bytes, 0, &NoServices).unwrap();
        assert!(audit_binary(&ctx, &[spec], &config).findings.is_empty(), "a zero margin lets every colour pass");
    }

    /// `head`, `hhea`, `hmtx`, `loca` (short) and `glyf` for three glyphs: an empty .notdef, glyph 1 as given
    /// (a square or nothing) and an empty glyph 2.
    fn glyf_font(gid1_square: bool, colr: Vec<u8>) -> Vec<u8> {
        let mut glyph = W::default();
        if gid1_square {
            glyph.i16(1).i16(0).i16(0).i16(100).i16(100);
            glyph.u16(3).u16(0);
            glyph.bytes(&[1, 1, 1, 1]);
            for x in [0i16, 0, 100, 0] {
                glyph.i16(x);
            }
            for y in [0i16, 100, 0, -100] {
                glyph.i16(y);
            }
        }
        let len = glyph.0.len() as u16;
        let mut loca = W::default();
        loca.u16(0).u16(0).u16(len / 2).u16(len / 2);
        let mut head = W::default();
        head.u32(0x0001_0000).u32(0).u32(0).u32(0x5F0F_3CF5).u16(0).u16(1000);
        head.u32(0).u32(0).u32(0).u32(0);
        head.i16(0).i16(0).i16(100).i16(100).u16(0).u16(8).i16(2).i16(0).i16(0);
        let (hhea, hmtx) = metrics(&[500, 500, 500]);
        sfnt(vec![(b"head", head.0), (b"hhea", hhea), (b"hmtx", hmtx), (b"maxp", maxp(3)), (b"loca", loca.0), (b"glyf", glyph.0), (b"COLR", colr)])
    }

    #[test]
    fn glyph_one_must_be_empty_in_colr_version_0_fonts() {
        let r = run(&glyf_font(true, colr0([0, 0])), "check.color.emptyGlyphOneForColrV0");
        assert_eq!(codes(&r), ["gid1-has-contours"], "{r:?}");
        assert_eq!(r.findings[0].data.get("area"), Some(&10_000.0));
        assert!(run(&glyf_font(false, colr0([0, 0])), "check.color.emptyGlyphOneForColrV0").findings.is_empty());
        assert!(run(&glyf_font(true, colr1()), "check.color.emptyGlyphOneForColrV0").findings.is_empty(), "only version 0 is affected");
    }

    #[test]
    fn colour_table_sets_are_graded() {
        let tables = |list: &[(&[u8; 4], Vec<u8>)]| {
            let mut all = vec![(b"maxp", maxp(4))];
            all.extend(list.iter().map(|(t, d)| (*t, d.clone())));
            sfnt(all)
        };
        let svg = vec![0u8, 0, 0, 0, 0, 10, 0, 0, 0, 0];
        let fvar = vec![0u8; 16];
        type Case<'a> = (Vec<(&'a [u8; 4], Vec<u8>)>, &'a [&'a str]);
        let cases: [Case<'_>; 6] = [
            (vec![(b"COLR", colr0([0, 0]))], &[]),
            (vec![(b"COLR", colr0([0, 0])), (b"SVG ", svg.clone())], &["drop-svg"]),
            (vec![(b"COLR", colr1())], &["add-svg"]),
            (vec![(b"COLR", colr1()), (b"fvar", fvar.clone())], &[]),
            (vec![(b"SVG ", svg.clone())], &["add-colr"]),
            (vec![(b"SVG ", svg.clone()), (b"fvar", fvar.clone()), (b"COLR", colr1())], &["variable-svg"]),
        ];
        for (list, expected) in cases {
            let r = run(&tables(&list), "check.color.colorTableSet");
            assert_eq!(codes(&r), expected, "{:?}", list.iter().map(|(t, _)| String::from_utf8_lossy(*t).to_string()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn layers_shared_or_not_are_measured_within_a_budget() {
        // 65,535 base glyphs over 64 layer ranges of 65,471 layers each: 4 billion layer visits
        // when every base glyph measures its own range.
        let (bases, layers) = (0xFFFFu16, 0xFFFFu16);
        let mut w = W::default();
        w.u16(0).u16(bases).u32(14).u32(14 + 6 * u32::from(bases)).u16(layers);
        for i in 0..bases {
            w.u16(i).u16(i % 64).u16(layers - 64);
        }
        for i in 0..layers {
            w.u16(i).u16(0);
        }
        let bytes = sfnt(vec![(b"maxp", maxp(4)), (b"COLR", w.0), (b"CPAL", cpal(&[(0, 0, 0)]))]);
        let r = run(&bytes, "check.color.paletteBrightness");
        assert_eq!(r.skipped.len(), 1, "{:?}", r.skipped);
        assert!(!r.findings.is_empty());
    }

    #[test]
    fn hostile_colour_tables_do_not_panic() {
        let mut huge_layers = W::default();
        huge_layers.u16(0).u16(1).u32(14).u32(20).u16(0xFFFF);
        huge_layers.u16(3).u16(0xFFF0).u16(0xFFFF);
        let fonts = [
            sfnt(vec![(b"maxp", maxp(4)), (b"COLR", huge_layers.0), (b"CPAL", cpal(&[(0, 0, 0)]))]),
            sfnt(vec![(b"maxp", maxp(4)), (b"COLR", colr0([0, 5])), (b"CPAL", vec![0, 0, 0, 9])]),
            sfnt(vec![(b"maxp", maxp(4)), (b"COLR", vec![0, 0, 0])]),
            glyf_font(true, vec![0, 0]),
        ];
        for bytes in &fonts {
            for id in ["check.color.paletteBrightness", "check.color.emptyGlyphOneForColrV0", "check.color.colorTableSet"] {
                let _ = run(bytes, id);
            }
        }
    }
}

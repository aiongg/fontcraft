//! `COLR` (M11.6 version 0, M11.7 version 1, M11.19 variable).
//!
//! Rows: `COLR`, `color.export.colrFromPaletteLayers`, `color.export.fullColorToColrV1`,
//! `color.export.colrVersionChoice`, `color.export.strokeInColr`, `compile.colrClipBoxes`,
//! `compile.colrGradientGeometry`, `color.export.variableColr`.
//!
//! Contract (`docs/plans/M11.md` § 3): base glyph and layer records for version 0; for version 1
//! `BaseGlyphList`, `LayerList` and `ClipList`. Layer glyphs are those
//! [`super::layer_glyphs`] appended, found by name in the [`GlyphOrder`]. `*` (the text colour)
//! is palette index 0xFFFF. Colours that are not palette references become new rows, added once
//! to every palette, with their alpha in CPAL only.

use std::collections::{HashMap, VecDeque};

use fontcraft_color::export::{ColorGlyph, ColrPlan, ColrVersion};
use fontcraft_color::paint::{ColorLine, ColorRef, Paint, is_solid_stack, shape_refs};
use fontcraft_color::palette::FOREGROUND_INDEX;
use fontcraft_color::{MAX_PAINT_DEPTH, MAX_PALETTE_ROWS, Rgba};
use fontcraft_geom::{Rect, Shape as _};
use fontcraft_model::{Extend, MAX_COMPONENT_DEPTH};
use write_fonts::tables::colr::{
    BaseGlyph, BaseGlyphList, BaseGlyphPaint, Clip, ClipBox, ClipList, ColorLine as WColorLine, ColorStop as WColorStop, Colr, Extend as WExtend,
    Layer, Paint as WPaint,
};
use write_fonts::types::{F2Dot14, FWord, GlyphId16, UfWord};

use super::GlyphOrder;
use crate::CompileError;
use crate::order::MadeFrom;

/// A built `COLR` table and the colours it added to the palettes (rows after `palette_rows`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColrOutput {
    pub table: Vec<u8>,
    pub added: Vec<Rgba>,
}

/// Most paints one `PaintColrLayers` holds; longer lists are nested.
const MAX_LAYERS_PER_PAINT: usize = u8::MAX as usize;

/// The table for `plan`, numbering new colours from `palette_rows`.
///
/// Version 0: base glyph records sorted by glyph id, each glyph's layer records in paint order,
/// the bottom layer first. A base glyph that is not in the font is left out.
///
/// Version 1, as fontc writes it: a glyph that is a stack of solid layers
/// ([`is_solid_stack`]) keeps version 0 records; every other glyph gets a `BaseGlyphList`
/// record whose paint is a `PaintColrLayers` of its layer paints (the one paint itself when it
/// has one layer), each a `PaintGlyph` of its layer glyph over a `PaintSolid`,
/// `PaintLinearGradient`, `PaintRadialGradient` or `PaintSweepGradient`, and a clip box
/// (`compile.colrClipBoxes`): the bounds of its layer glyphs widened to multiples of a tenth of
/// the em rounded to 10 units, consecutive glyphs with the same box sharing one range. Nothing is
/// shared between glyphs.
pub fn build(plan: &ColrPlan, order: &GlyphOrder, palette_rows: usize, units_per_em: u16) -> Result<ColrOutput, CompileError> {
    // The layer glyphs `super::layer_glyphs` made, by base glyph and colour layer, in the order
    // it appended them, which is the order of the paint's shape references.
    let mut made: HashMap<(&str, &str), VecDeque<u16>> = HashMap::new();
    for m in order.made {
        if let MadeFrom::ColorLayer { glyph, layer } = &m.source
            && let Some(gid) = order.gid(&m.name)
        {
            made.entry((glyph.as_str(), layer.as_str())).or_default().push_back(gid);
        }
    }
    let mut glyphs: Vec<(u16, &ColorGlyph)> = plan.glyphs.iter().filter_map(|g| order.gid(&g.glyph).map(|gid| (gid, g))).collect();
    glyphs.sort_by_key(|(gid, _)| *gid);
    glyphs.dedup_by_key(|(gid, _)| *gid);

    let mut colors = AddedColors { rows: palette_rows, added: Vec::new() };
    let mut records: Vec<(u16, Vec<Layer>)> = Vec::new();
    let mut paints: Vec<BaseGlyphPaint> = Vec::new();
    let mut layer_list: Vec<WPaint> = Vec::new();
    let mut clips: Vec<Clip> = Vec::new();
    let mut bounds = Bounds { order, memo: HashMap::new() };
    let quantum = clip_quantum(units_per_em);
    for (base, g) in glyphs {
        let refs = shape_refs(&g.paint);
        // A shape reference without a made glyph is drawn by a source glyph of the name a made
        // one would have had (`super::existing_layer_glyph`).
        let gids: Vec<u16> = refs
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                made.get_mut(&(g.glyph.as_str(), s.layer.as_str()))
                    .and_then(VecDeque::pop_front)
                    .or_else(|| order.gid(&format!("{}.color{i}", g.glyph)))
            })
            .collect();
        if refs.len() != gids.len() {
            return Err(table_error(format!("glyph `{}` has {} colour layers but {} layer glyphs", g.glyph, refs.len(), gids.len())));
        }
        if plan.version == ColrVersion::V0 || is_solid_stack(&g.paint) {
            let solids = solid_layers(&g.paint).ok_or_else(|| table_error(format!("glyph `{}` needs COLR version 1", g.glyph)))?;
            let layers = gids
                .iter()
                .zip(solids)
                .map(|(&gid, color)| Ok(Layer::new(GlyphId16::new(gid), colors.index(color)?)))
                .collect::<Result<Vec<_>, CompileError>>()?;
            records.push((base, layers));
            continue;
        }
        let mut writer = PaintWriter { glyph: &g.glyph, gids: gids.iter(), colors: &mut colors, layers: &mut layer_list };
        let paint = writer.paint(&g.paint, 0)?;
        paints.push(BaseGlyphPaint::new(GlyphId16::new(base), paint));
        if let Some(b) = gids.iter().filter_map(|&gid| bounds.of(gid, 0)).reduce(|a, b| a.union(b)) {
            add_clip(&mut clips, base, quantize(b, quantum));
        }
    }

    let too_many = || table_error("more than 65,535 layer records".to_string());
    let mut base_records = Vec::with_capacity(records.len());
    let mut layer_records = Vec::new();
    for (gid, layers) in records {
        let first = u16::try_from(layer_records.len()).map_err(|_| too_many())?;
        let count = u16::try_from(layers.len()).map_err(|_| too_many())?;
        base_records.push(BaseGlyph::new(GlyphId16::new(gid), first, count));
        layer_records.extend(layers);
    }
    let num_layers = u16::try_from(layer_records.len()).map_err(|_| too_many())?;
    // At most one record per glyph id.
    let num_base = u16::try_from(base_records.len()).map_err(|_| too_many())?;
    // A version 1 table without version 0 glyphs leaves their offsets null, as fontc writes it.
    let v0_only = paints.is_empty();
    let base_records = (v0_only || !base_records.is_empty()).then_some(base_records);
    let layer_records = (v0_only || !layer_records.is_empty()).then_some(layer_records);
    let mut colr = Colr::new(num_base, base_records, layer_records, num_layers);
    if !paints.is_empty() {
        let count = |n: usize| u32::try_from(n).map_err(|_| table_error("more than 2^32 paints".to_string()));
        colr.base_glyph_list = BaseGlyphList::new(count(paints.len())?, paints).into();
        if !layer_list.is_empty() {
            colr.layer_list = write_fonts::tables::colr::LayerList::new(count(layer_list.len())?, layer_list).into();
        }
        if !clips.is_empty() {
            colr.clip_list = ClipList::new(1, count(clips.len())?, clips).into();
        }
    }
    let table = write_fonts::dump_table(&colr).map_err(|e| table_error(e.to_string()))?;
    Ok(ColrOutput { table, added: colors.added })
}

/// Turns one glyph's paint graph into version 1 paints, taking its layer glyphs in the order
/// [`shape_refs`] lists them and appending the layers of every `PaintColrLayers` to the layer
/// list.
struct PaintWriter<'a, 'b> {
    glyph: &'a str,
    gids: std::slice::Iter<'a, u16>,
    colors: &'b mut AddedColors,
    layers: &'b mut Vec<WPaint>,
}

impl PaintWriter<'_, '_> {
    fn paint(&mut self, paint: &Paint, depth: usize) -> Result<WPaint, CompileError> {
        if depth > MAX_PAINT_DEPTH {
            return Err(table_error(format!("the paints of glyph `{}` nest more than {MAX_PAINT_DEPTH} deep", self.glyph)));
        }
        Ok(match paint {
            Paint::Layers(list) => {
                let paints = list.iter().map(|p| self.paint(p, depth + 1)).collect::<Result<Vec<_>, _>>()?;
                self.colr_layers(paints, depth)?
            }
            Paint::Glyph { paint, .. } => {
                let gid = self.gids.next().copied().ok_or_else(|| table_error(format!("glyph `{}` has too few layer glyphs", self.glyph)))?;
                WPaint::glyph(self.paint(paint, depth + 1)?, GlyphId16::new(gid))
            }
            Paint::Solid(c) => {
                let (index, alpha) = self.colors.paint_color(*c)?;
                WPaint::solid(index, alpha)
            }
            Paint::Linear { p0, p1, p2, line } => {
                WPaint::linear_gradient(self.line(line)?, fword(p0.x), fword(p0.y), fword(p1.x), fword(p1.y), fword(p2.x), fword(p2.y))
            }
            Paint::Radial { c0, r0, c1, r1, line } => {
                WPaint::radial_gradient(self.line(line)?, fword(c0.x), fword(c0.y), ufword(*r0), fword(c1.x), fword(c1.y), ufword(*r1))
            }
            Paint::Sweep { center, start, end, line } => {
                let (start, end) = sweep_angles(*start, *end);
                WPaint::sweep_gradient(self.line(line)?, fword(center.x), fword(center.y), start, end)
            }
        })
    }

    /// One paint for `paints`: the paint itself when there is one, else a `PaintColrLayers`
    /// whose layers go to the end of the layer list, nested in groups of at most 255.
    fn colr_layers(&mut self, mut paints: Vec<WPaint>, depth: usize) -> Result<WPaint, CompileError> {
        if paints.len() == 1
            && let Some(one) = paints.pop()
        {
            return Ok(one);
        }
        if paints.len() > MAX_LAYERS_PER_PAINT {
            if depth > MAX_PAINT_DEPTH {
                return Err(table_error(format!("glyph `{}` has too many layers", self.glyph)));
            }
            let mut groups = Vec::with_capacity(paints.len().div_ceil(MAX_LAYERS_PER_PAINT));
            let mut rest = paints.into_iter();
            loop {
                let chunk: Vec<WPaint> = rest.by_ref().take(MAX_LAYERS_PER_PAINT).collect();
                if chunk.is_empty() {
                    break;
                }
                groups.push(self.colr_layers(chunk, depth + 1)?);
            }
            return self.colr_layers(groups, depth + 1);
        }
        let first = u32::try_from(self.layers.len()).map_err(|_| table_error("more than 2^32 layer paints".to_string()))?;
        let count = u8::try_from(paints.len()).map_err(|_| table_error("more than 255 layers in one paint".to_string()))?;
        self.layers.extend(paints);
        Ok(WPaint::colr_layers(count, first))
    }

    fn line(&mut self, line: &ColorLine) -> Result<WColorLine, CompileError> {
        let count = u16::try_from(line.stops.len()).map_err(|_| table_error(format!("a gradient of glyph `{}` has too many stops", self.glyph)))?;
        let stops = line
            .stops
            .iter()
            .map(|s| {
                let (index, alpha) = self.colors.paint_color(s.color)?;
                let offset = if s.offset.is_finite() { s.offset.clamp(0.0, 1.0) } else { 0.0 };
                Ok(WColorStop::new(f2dot14(offset), index, alpha))
            })
            .collect::<Result<Vec<_>, CompileError>>()?;
        let extend = match line.extend {
            Extend::Pad => WExtend::Pad,
            Extend::Repeat => WExtend::Repeat,
            Extend::Reflect => WExtend::Reflect,
        };
        Ok(WColorLine::new(extend, count, stops))
    }
}

/// A coordinate in font units, rounded and kept within 16 bits; 0 when it is not a number.
fn fword(v: f64) -> FWord {
    // Clamped to the i16 range and rounded, so the conversion is exact.
    #[allow(clippy::cast_possible_truncation)]
    let n = if v.is_finite() { v.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16 } else { 0 };
    FWord::new(n)
}

/// A radius in font units, rounded and kept within 16 bits.
fn ufword(v: f64) -> UfWord {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = if v.is_finite() { v.abs().round().min(f64::from(u16::MAX)) as u16 } else { 0 };
    UfWord::new(n)
}

/// A value of the F2Dot14 range; 0 when it is not a number.
fn f2dot14(v: f64) -> F2Dot14 {
    let v = if v.is_finite() { v.clamp(-2.0, 1.999_938_964_843_75) } else { 0.0 };
    // Within the F2Dot14 range, which an f32 holds exactly enough.
    #[allow(clippy::cast_possible_truncation)]
    F2Dot14::from_f32(v as f32)
}

/// Sweep angles in degrees counter-clockwise as the table stores them: 180° per 1.0, biased by
/// 180° (a stored −1.0 is 0°), so the range is −180° to 540°. The start is brought to −180°..180°
/// and the sweep kept within one turn; a backward sweep that would end below −180° is moved one
/// turn up.
fn sweep_angles(start: f64, end: f64) -> (F2Dot14, F2Dot14) {
    let (start, end) = if start.is_finite() && end.is_finite() { (start, end) } else { (0.0, 360.0) };
    let sweep = (end - start).clamp(-360.0, 360.0);
    let mut s = (start + 180.0).rem_euclid(360.0) - 180.0;
    if s + sweep < -180.0 {
        s += 360.0;
    }
    let stored = |deg: f64| f2dot14(deg / 180.0 - 1.0);
    (stored(s), stored(s + sweep))
}

/// The clip-box quantum: a tenth of the em, rounded to a multiple of 10 units.
fn clip_quantum(units_per_em: u16) -> f64 {
    (f64::from(units_per_em) / 100.0).round() * 10.0
}

/// `b` widened outward to multiples of `quantum` (nothing when it is 1 or less) and kept within
/// 16 bits.
fn quantize(b: Rect, quantum: f64) -> ClipBox {
    let (x0, y0, x1, y1) = if quantum > 1.0 {
        let down = |v: f64| (v / quantum).floor() * quantum;
        let up = |v: f64| (v / quantum).ceil() * quantum;
        (down(b.x0), down(b.y0), up(b.x1), up(b.y1))
    } else {
        (b.x0.floor(), b.y0.floor(), b.x1.ceil(), b.y1.ceil())
    };
    ClipBox::format_1(fword(x0), fword(y0), fword(x1), fword(y1))
}

/// Adds the clip of `gid`, extending the last range when it ends at the glyph before with the
/// same box. Glyphs come in glyph id order.
fn add_clip(clips: &mut Vec<Clip>, gid: u16, clip: ClipBox) {
    if let Some(last) = clips.last_mut()
        && last.end_glyph_id.to_u16().checked_add(1) == Some(gid)
        && *last.clip_box == clip
    {
        last.end_glyph_id = GlyphId16::new(gid);
        return;
    }
    clips.push(Clip::new(GlyphId16::new(gid), GlyphId16::new(gid), clip));
}

/// The bounds of the glyphs of a [`GlyphOrder`], components followed through the order at most
/// [`MAX_COMPONENT_DEPTH`] deep, each glyph measured once.
struct Bounds<'a> {
    order: &'a GlyphOrder<'a>,
    memo: HashMap<u16, Option<Rect>>,
}

impl Bounds<'_> {
    /// The bounding box of glyph `gid`'s contours and components, curve extremes included;
    /// `None` for a glyph without an outline.
    fn of(&mut self, gid: u16, depth: usize) -> Option<Rect> {
        if let Some(b) = self.memo.get(&gid) {
            return *b;
        }
        if depth > MAX_COMPONENT_DEPTH {
            return None;
        }
        let order = self.order;
        let glyph = order.glyphs.get(usize::from(gid))?;
        let mut out: Option<Rect> = None;
        let mut add = |b: Rect| out = Some(out.map_or(b, |o| o.union(b)));
        for c in glyph.contours.iter().filter(|c| !c.elements().is_empty()) {
            add(c.bounding_box());
        }
        for c in &glyph.components {
            if let Some(b) = order.gid(&c.base).and_then(|base| self.of(base, depth + 1)) {
                add(c.transform.transform_rect_bbox(b));
            }
        }
        self.memo.insert(gid, out);
        out
    }
}

fn table_error(message: String) -> CompileError {
    CompileError::Table { table: "COLR", message }
}

/// The colour of each layer of a stack of solid layers, bottom first; `None` for any other
/// paint.
fn solid_layers(paint: &Paint) -> Option<Vec<ColorRef>> {
    let solid = |p: &Paint| match p {
        Paint::Glyph { paint, .. } => match paint.as_ref() {
            Paint::Solid(c) => Some(*c),
            _ => None,
        },
        _ => None,
    };
    match paint {
        Paint::Layers(list) => list.iter().map(solid).collect(),
        other => solid(other).map(|c| vec![c]),
    }
}

/// Colours that are not palette references, numbered after the palette rows in the order COLR
/// first uses them, once each.
struct AddedColors {
    rows: usize,
    added: Vec<Rgba>,
}

impl AddedColors {
    /// The palette index and alpha of a version 1 paint: a palette reference keeps its alpha; a
    /// colour of its own is opaque in the paint, its alpha in CPAL.
    fn paint_color(&mut self, color: ColorRef) -> Result<(u16, F2Dot14), CompileError> {
        let alpha = |a: f64| f2dot14(if a.is_finite() { a.clamp(0.0, 1.0) } else { 1.0 });
        Ok(match color {
            ColorRef::Palette { index, alpha: a } => (index, alpha(a)),
            ColorRef::Foreground { alpha: a } => (FOREGROUND_INDEX, alpha(a)),
            ColorRef::Rgba(_) => (self.index(color)?, alpha(1.0)),
        })
    }

    /// The palette index of `color`. Version 0 has no alpha of its own: a palette reference's
    /// alpha is dropped (palette layers always have 1), and a colour's alpha goes into CPAL.
    fn index(&mut self, color: ColorRef) -> Result<u16, CompileError> {
        let rgba = match color {
            ColorRef::Palette { index, .. } => return Ok(index),
            ColorRef::Foreground { .. } => return Ok(FOREGROUND_INDEX),
            ColorRef::Rgba(c) => to_rgba(c),
        };
        let at = match self.added.iter().position(|&a| a == rgba) {
            Some(at) => at,
            None => {
                self.added.push(rgba);
                self.added.len().saturating_sub(1)
            }
        };
        self.rows
            .checked_add(at)
            .filter(|&i| i < MAX_PALETTE_ROWS)
            .and_then(|i| u16::try_from(i).ok())
            .ok_or_else(|| table_error(format!("the palettes and the colours the layers add are more than {MAX_PALETTE_ROWS} colours")))
    }
}

/// Components from 0 to 1 as CPAL's bytes; a component that is not a number is 0.
fn to_rgba(c: [f64; 4]) -> Rgba {
    // Clamped to 0–255 and rounded, so the conversion is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = |v: f64| if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
    let [r, g, b, a] = c;
    Rgba::new(byte(r), byte(g), byte(b), byte(a))
}

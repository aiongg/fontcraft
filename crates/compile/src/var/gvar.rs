//! The `gvar` table (M9.5).
//!
//! Rows: the `gvar` table row, `compile.gvarIupOptimisation`, `compile.gvarEveryGlyph`,
//! `compile.gvarPhantomPoints`, `compile.gvarSharedTuples`, parameters `Optimize Variable Deltas`,
//! `Variable Font Optimize Deltas`.
//!
//! Contract: every glyph of the font gets an entry (empty when it does not vary). A glyph's
//! deltas come from its sources through its own sub-model (`fontcraft_geom::variation`), the four
//! phantom points included; with optimization on, deltas IUP can infer within
//! [`IUP_TOLERANCE`] are left out. Peak tuples used by several glyphs are shared (at most 4,095),
//! and a glyph has at most 4,095 tuple variations; points per glyph are bounded by
//! [`crate::MAX_TT_POINTS`].
//!
//! Deltas are rounded one region at a time, each later region taken from the rounded earlier
//! ones (fontTools' `getDeltas` with rounding, as fontmake builds `gvar`), so the font gives each
//! source's coordinates exactly. Composites and empty glyphs are not optimized: a tuple whose
//! deltas are all zero is left out, the others are written whole, as fontc writes them.
//!
//! The sizes of packed deltas and point numbers ([`check_tuple_size`]) follow the packing rules
//! of write-fonts' `PackedDeltas` and `PackedPointNumbers` (fontations, MIT OR Apache-2.0), so a
//! tuple too big for the table is refused before the writer is handed it.

use std::collections::HashMap;

use fontcraft_geom::variation::{Region, VariationModel};
use fontcraft_geom::{Point, Vec2};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar, iup::iup_delta_optimize};
use write_fonts::tables::variations::Tent;
use write_fonts::types::{F2Dot14, GlyphId};

use crate::{CompileError, MAX_TT_POINTS, ot_round};

/// How far, in font units, an IUP-inferred delta may be from the real one.
pub const IUP_TOLERANCE: f64 = 0.5;
/// Most peak tuples shared between glyphs (the format's 12-bit index).
pub const MAX_SHARED_TUPLES: usize = 4_095;
/// Most tuple variations one glyph may have (the format's 12-bit count).
pub const MAX_GLYPH_TUPLES: usize = 4_095;
/// Most deltas (points times tuples) over the whole table, so the table stays far below the
/// format's 32-bit offsets.
pub const MAX_TOTAL_DELTAS: usize = 50_000_000;
/// The phantom points every glyph's point list ends with.
pub const PHANTOM_POINTS: usize = 4;

/// One glyph's points at each of its sources, in the same order in every source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceOutlines {
    pub glyph: String,
    /// Normalized location of each source; the first is the origin (all zeros).
    pub locations: Vec<Vec<f64>>,
    /// Per source, every point of the glyph followed by the four phantom points.
    pub points: Vec<Vec<(f64, f64)>>,
    /// Index of the last point of each contour (for IUP); empty for composites and empty glyphs.
    pub contour_ends: Vec<usize>,
}

/// How `gvar` is written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GvarOptions {
    /// Leave out deltas IUP can infer.
    pub optimize: bool,
}

impl Default for GvarOptions {
    fn default() -> Self {
        Self { optimize: true }
    }
}

fn gvar_error(glyph: &str, message: impl std::fmt::Display) -> CompileError {
    CompileError::Table { table: "gvar", message: format!("glyph `{glyph}`: {message}") }
}

/// Variation models by their set of locations, so glyphs with the same sources share one.
#[derive(Debug, Default)]
pub struct Models {
    models: HashMap<Vec<u64>, VariationModel>,
}

impl Models {
    /// The model of `locations` (normalized, one of them all zeros).
    pub fn get(&mut self, locations: &[Vec<f64>]) -> Result<&VariationModel, String> {
        let key: Vec<u64> = locations.iter().flat_map(|l| l.iter().map(|v| (v + 0.0).to_bits())).collect();
        if !self.models.contains_key(&key) {
            let model = VariationModel::new(locations, false).map_err(|e| e.to_string())?;
            self.models.insert(key.clone(), model);
        }
        self.models.get(&key).ok_or_else(|| "no variation model".to_string())
    }
}

/// The rounded deltas of `values` (one list per source, the caller's order) under `model`, each
/// with its region, the default left out.
pub fn region_deltas(model: &VariationModel, values: &[Vec<f64>]) -> Result<Vec<(Region, Vec<f64>)>, String> {
    let deltas = model.deltas_with(values, ot_round).map_err(|e| e.to_string())?;
    Ok(model.regions().into_iter().zip(deltas).skip(1).collect())
}

/// `region` as `gvar` tents, one per axis.
pub(crate) fn tents(region: &Region, axis_count: u16) -> Result<Vec<Tent>, String> {
    if region.0.len() != usize::from(axis_count) {
        return Err(format!("a region has {} axes; the font has {axis_count}", region.0.len()));
    }
    region
        .0
        .iter()
        .map(|a| {
            let ok = |v: f64| v.is_finite() && (-1.0..=1.0).contains(&v);
            if !(ok(a.start) && ok(a.peak) && ok(a.end)) {
                return Err("a region reaches outside −1…1".to_string());
            }
            let f = |v: f64| F2Dot14::from_f32(v as f32);
            Ok(Tent::new(f(a.peak), Some((f(a.start), f(a.end)))))
        })
        .collect()
}

/// Whether `v` is a finite delta within 16 bits, as every variation table here writes them.
pub(crate) fn fits_i16(v: f64) -> bool {
    v.is_finite() && (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&v)
}

fn delta_i16(glyph: &str, v: f64) -> Result<i16, CompileError> {
    if fits_i16(v) { Ok(v as i16) } else { Err(gvar_error(glyph, format!("a delta of {v} units does not fit the table"))) }
}

/// Most bytes of one tuple variation's data: the table stores its size in 16 bits.
pub const MAX_TUPLE_DATA: usize = u16::MAX as usize;

#[derive(Clone, Copy, PartialEq)]
enum RunKind {
    Zero,
    Byte,
    Word,
}

fn run_kind(v: i32) -> RunKind {
    match v {
        0 => RunKind::Zero,
        -128..=127 => RunKind::Byte,
        _ => RunKind::Word,
    }
}

/// Bytes of `values` (each within i16) packed as the table writer packs deltas: runs of at most
/// 64, a lone zero kept inside a byte run, a lone byte value kept inside a word run between word
/// values.
fn packed_deltas_size(values: &[i32]) -> usize {
    const MAX_RUN: usize = 64;
    let mut size = 0;
    let mut rest = values;
    while let Some(&first) = rest.first() {
        let kind = run_kind(first);
        let len = if kind == RunKind::Zero {
            rest.iter().take(MAX_RUN).take_while(|v| **v == 0).count()
        } else {
            let mut len = 1;
            while len < MAX_RUN && len < rest.len() {
                let cur = rest.get(len).map_or(RunKind::Zero, |v| run_kind(*v));
                let next = rest.get(len + 1).map(|v| run_kind(*v));
                let stop = match kind {
                    RunKind::Byte => cur == RunKind::Word || (cur == RunKind::Zero && next == Some(RunKind::Zero)),
                    _ => cur == RunKind::Zero || (cur == RunKind::Byte && matches!(next, Some(RunKind::Zero | RunKind::Byte))),
                };
                if stop {
                    break;
                }
                len += 1;
            }
            len
        };
        size += 1 + match kind {
            RunKind::Zero => 0,
            RunKind::Byte => len,
            RunKind::Word => 2 * len,
        };
        rest = rest.get(len.max(1)..).unwrap_or_default();
    }
    size
}

/// Bytes of the ascending point numbers `points` packed as the table writer packs them: a count
/// of one or two bytes, then runs of at most 128 that are all bytes or all words (a word when a
/// point is more than 255 past the one before).
fn packed_points_size(points: &[usize]) -> usize {
    const MAX_RUN: usize = 128;
    let mut size = if points.len() < 128 { 1 } else { 2 };
    let mut prev = 0;
    let mut rest = points;
    while let Some(&first) = rest.first() {
        let words = first.saturating_sub(prev) > 255;
        let mut last = prev;
        let len = rest
            .iter()
            .take(MAX_RUN)
            .take_while(|&&p| {
                let take = (p.saturating_sub(last) > 255) == words;
                last = p;
                take
            })
            .count()
            .max(1);
        size += 1 + if words { 2 * len } else { len };
        prev = rest.get(len - 1).copied().unwrap_or(prev);
        rest = rest.get(len..).unwrap_or_default();
    }
    size
}

/// Refuses a tuple variation whose data would not fit the table's 16-bit size field. The writer
/// sizes the data both with every point and with only the required ones, so both must fit.
fn check_tuple_size(glyph: &str, deltas: &[GlyphDelta]) -> Result<(), CompileError> {
    let packed = |ds: &mut dyn Iterator<Item = &GlyphDelta>| {
        let (xs, ys): (Vec<i32>, Vec<i32>) = ds.map(|d| (i32::from(d.x), i32::from(d.y))).unzip();
        packed_deltas_size(&xs) + packed_deltas_size(&ys)
    };
    let mut size = 1 + packed(&mut deltas.iter());
    if deltas.iter().any(|d| !d.required) {
        let points: Vec<usize> = deltas.iter().enumerate().filter(|(_, d)| d.required).map(|(i, _)| i).collect();
        if points.last().is_some_and(|&p| p > usize::from(u16::MAX)) {
            return Err(gvar_error(glyph, "a delta is needed on a point past number 65,535"));
        }
        size = size.max(packed_points_size(&points) + packed(&mut deltas.iter().filter(|d| d.required)));
    }
    if size > MAX_TUPLE_DATA {
        return Err(gvar_error(glyph, format!("one tuple variation needs {size} bytes; the table holds at most {MAX_TUPLE_DATA}")));
    }
    Ok(())
}

/// The tuple variations of one glyph.
fn glyph_variations(glyph: &SourceOutlines, axis_count: u16, options: &GvarOptions, models: &mut Models) -> Result<Vec<GlyphDeltas>, CompileError> {
    let name = glyph.glyph.as_str();
    if glyph.locations.len() < 2 {
        return Ok(Vec::new());
    }
    let count = glyph.points.first().map_or(0, Vec::len);
    if !(PHANTOM_POINTS..=MAX_TT_POINTS + PHANTOM_POINTS).contains(&count) {
        return Err(gvar_error(name, format!("{count} points, phantom points included")));
    }
    if glyph.points.len() != glyph.locations.len() || glyph.points.iter().any(|p| p.len() != count) {
        return Err(gvar_error(name, "the sources do not have the same points"));
    }
    let values: Vec<Vec<f64>> = glyph.points.iter().map(|ps| ps.iter().flat_map(|&(x, y)| [x, y]).collect()).collect();
    let model = models.get(&glyph.locations).map_err(|e| gvar_error(name, e))?;
    let regions = region_deltas(model, &values).map_err(|e| gvar_error(name, e))?;
    if regions.len() > MAX_GLYPH_TUPLES {
        return Err(gvar_error(name, format!("{} tuple variations; a glyph holds at most {MAX_GLYPH_TUPLES}", regions.len())));
    }
    let origin: Vec<Point> = glyph.points.first().map(|ps| ps.iter().map(|&(x, y)| Point::new(x, y)).collect()).unwrap_or_default();
    let simple = !glyph.contour_ends.is_empty();
    let mut out = Vec::new();
    for (region, flat) in regions {
        if flat.iter().all(|d| *d == 0.0) {
            continue;
        }
        // The optimizer rounds and saturates out-of-range deltas without a word, so every value
        // is checked here, whichever path writes it.
        for v in &flat {
            delta_i16(name, *v)?;
        }
        let pairs: Vec<(f64, f64)> = flat.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)).collect();
        let deltas: Vec<GlyphDelta> = if simple && options.optimize {
            let vecs: Vec<Vec2> = pairs.iter().map(|&(x, y)| Vec2::new(x, y)).collect();
            iup_delta_optimize(vecs, origin.clone(), IUP_TOLERANCE, &glyph.contour_ends).map_err(|e| gvar_error(name, format!("{e:?}")))?
        } else {
            pairs.iter().map(|&(x, y)| Ok(GlyphDelta::required(delta_i16(name, x)?, delta_i16(name, y)?))).collect::<Result<_, CompileError>>()?
        };
        check_tuple_size(name, &deltas)?;
        let tents = tents(&region, axis_count).map_err(|e| gvar_error(name, e))?;
        out.push(GlyphDeltas::new(tents, deltas));
    }
    Ok(out)
}

/// The `gvar` table of `glyphs` (one per glyph, in glyph order).
pub fn build_gvar(axis_count: u16, glyphs: &[SourceOutlines], options: &GvarOptions) -> Result<Gvar, CompileError> {
    if glyphs.len() > crate::MAX_FONT_GLYPHS {
        return Err(CompileError::TooManyGlyphs(glyphs.len()));
    }
    let total: usize = glyphs.iter().map(|g| g.points.first().map_or(0, Vec::len).saturating_mul(g.locations.len().saturating_sub(1))).sum();
    if total > MAX_TOTAL_DELTAS {
        return Err(CompileError::Table { table: "gvar", message: format!("{total} deltas; a font holds at most {MAX_TOTAL_DELTAS}") });
    }
    let mut models = Models::default();
    let mut variations = Vec::with_capacity(glyphs.len());
    for (gid, glyph) in glyphs.iter().enumerate() {
        crate::check_stop()?;
        let gid = u32::try_from(gid).map_err(|_| CompileError::TooManyGlyphs(glyphs.len()))?;
        variations.push(GlyphVariations::new(GlyphId::new(gid), glyph_variations(glyph, axis_count, options, &mut models)?));
    }
    Gvar::new(variations, axis_count).map_err(|e| CompileError::Table { table: "gvar", message: format!("{e:?}") })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use skrifa::raw::{FontData, FontRead};

    fn square(d: f64, advance: f64) -> Vec<(f64, f64)> {
        vec![(0.0, 0.0), (0.0, 100.0 + d), (100.0 + d, 100.0 + d), (100.0 + d, 0.0), (0.0, 0.0), (advance, 0.0), (0.0, 0.0), (0.0, 0.0)]
    }

    #[test]
    fn varying_and_still_glyphs_each_get_an_entry() {
        let moving = SourceOutlines {
            glyph: "a".into(),
            locations: vec![vec![0.0], vec![1.0]],
            points: vec![square(0.0, 200.0), square(50.0, 260.0)],
            contour_ends: vec![3],
        };
        let still = SourceOutlines { glyph: "b".into(), locations: vec![vec![0.0]], points: vec![square(0.0, 200.0)], contour_ends: vec![3] };
        let gvar = build_gvar(1, &[still, moving], &GvarOptions::default()).unwrap();
        let bytes = write_fonts::dump_table(&gvar).unwrap();
        let read = skrifa::raw::tables::gvar::Gvar::read(FontData::new(&bytes)).unwrap();
        assert_eq!(read.glyph_count(), 2);
        assert!(read.glyph_variation_data(skrifa::raw::types::GlyphId::new(0)).unwrap().is_none());
        let data = read.glyph_variation_data(skrifa::raw::types::GlyphId::new(1)).unwrap().unwrap();
        let tuple = data.tuples().next().unwrap();
        assert_eq!(tuple.peak().values.iter().map(|v| v.get().to_f32()).collect::<Vec<_>>(), vec![1.0]);
        // The advance (phantom point 1) varies by 60.
        let deltas: Vec<_> = tuple.deltas().collect();
        assert!(deltas.iter().any(|d| d.position == 5 && d.x_delta == 60));
    }

    /// A glyph of `count` points (phantoms included) whose second source moves point `i` by
    /// `delta(i)` on both axes.
    fn many_points(count: usize, delta: impl Fn(usize) -> f64) -> SourceOutlines {
        let origin: Vec<(f64, f64)> = (0..count).map(|i| ((i % 1000) as f64, (i / 1000) as f64)).collect();
        let moved = origin.iter().enumerate().map(|(i, &(x, y))| (x + delta(i), y + delta(i))).collect();
        SourceOutlines { glyph: "big".into(), locations: vec![vec![0.0], vec![1.0]], points: vec![origin, moved], contour_ends: vec![count - 5] }
    }

    #[test]
    fn tuples_too_big_for_the_size_field_are_refused() {
        // Word deltas on every point: 16,256 points pack into 65,533 bytes, one more into 65,539.
        let plain = GvarOptions { optimize: false };
        let fits = build_gvar(1, &[many_points(16_256, |_| 200.0)], &plain).unwrap();
        assert!(write_fonts::dump_table(&fits).is_ok());
        let err = build_gvar(1, &[many_points(16_257, |_| 200.0)], &plain).unwrap_err().to_string();
        assert!(err.contains("`big`") && err.contains("65539 bytes"), "{err}");
        // With optimization, deltas IUP cannot infer are kept and the glyph is still refused.
        let zigzag = |i: usize| if i.is_multiple_of(2) { 300.0 } else { -300.0 };
        let err = build_gvar(1, &[many_points(20_000, zigzag)], &GvarOptions::default()).unwrap_err().to_string();
        assert!(err.contains("`big`"), "{err}");
    }

    #[test]
    fn deltas_past_16_bits_are_refused_with_or_without_optimization() {
        for optimize in [true, false] {
            let wide = SourceOutlines {
                glyph: "wide".into(),
                locations: vec![vec![0.0], vec![1.0]],
                points: vec![square(0.0, 500.0), square(0.0, 50_000.0)],
                contour_ends: vec![3],
            };
            let err = build_gvar(1, &[wide], &GvarOptions { optimize }).unwrap_err().to_string();
            assert!(err.contains("`wide`") && err.contains("49500"), "{err}");
        }
    }

    #[test]
    fn mismatched_sources_are_refused() {
        let bad = SourceOutlines {
            glyph: "a".into(),
            locations: vec![vec![0.0], vec![1.0]],
            points: vec![square(0.0, 200.0), square(0.0, 200.0)[..6].to_vec()],
            contour_ends: vec![3],
        };
        assert!(build_gvar(1, &[bad], &GvarOptions::default()).is_err());
    }
}

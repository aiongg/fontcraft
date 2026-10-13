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
fn tents(region: &Region, axis_count: u16) -> Result<Vec<Tent>, String> {
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

fn delta_i16(glyph: &str, v: f64) -> Result<i16, CompileError> {
    if v.is_finite() && (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&v) {
        Ok(v as i16)
    } else {
        Err(gvar_error(glyph, format!("a delta of {v} units does not fit the table")))
    }
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
        let pairs: Vec<(f64, f64)> = flat.chunks_exact(2).map(|c| (c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0))).collect();
        let deltas: Vec<GlyphDelta> = if simple && options.optimize {
            let vecs: Vec<Vec2> = pairs.iter().map(|&(x, y)| Vec2::new(x, y)).collect();
            iup_delta_optimize(vecs, origin.clone(), IUP_TOLERANCE, &glyph.contour_ends).map_err(|e| gvar_error(name, format!("{e:?}")))?
        } else {
            pairs.iter().map(|&(x, y)| Ok(GlyphDelta::required(delta_i16(name, x)?, delta_i16(name, y)?))).collect::<Result<_, CompileError>>()?
        };
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

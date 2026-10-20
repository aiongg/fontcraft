//! The `CFF2` flavour of the variable compile (M9.13).
//!
//! Rows: `compile.cff2VariableBuild`, `compile.cff2CompanionTables`.
//!
//! Each glyph's cubic sources become blended charstrings for
//! [`fontcraft_cff::cff2::write_cff2`]: the sources' coordinates are rounded to whole units, as
//! fontmake rounds its CFF masters, and the deltas come from the glyph's variation model with
//! fontTools' tolerance of 0.01 unit, so intermediate sources can give fractional deltas. Glyphs
//! with the same sources share a region list; list 0 is the masters' model, which the Private
//! DICT's blended blue zones and stems use. Sources must draw the same segments; the closing
//! line of a contour is left out only where every source can leave it out.
//!
//! Companion tables: `hmtx` with `HVAR`, `post` 2.0 with the glyph names, `maxp` 0.5; no `glyf`,
//! `loca` or `gvar`. Hints are not written (see [`HINTS_DROPPED`]).

use std::collections::HashMap;

use fontcraft_cff::cff2::{Blend, BlendOp, Cff2Font, Cff2Glyph, Cff2Private, RegionAxis};
use fontcraft_cff::{Number, PathOp, PrivateDict};
use fontcraft_geom::BezPath;
use fontcraft_geom::variation::{Region, VariationModel};
use fontcraft_model::{Font, Resolved, Target};

use super::Source;
use super::gvar::Models;
use crate::{CompileError, ot_round};

/// fontTools' rounding tolerance for CFF2 coordinates and deltas (`CFF2CharStringMergePen`).
pub const CFF2_ROUND_TOLERANCE: f64 = 0.01;

/// The warning for a font whose glyphs have manual hints: a CFF2 font is written without them.
pub const HINTS_DROPPED: &str = "CFF2 fonts are written without hints; the glyphs' manual hints were left out";

/// One glyph's cubic outlines at each of its sources.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CubicSources {
    pub glyph: String,
    /// Normalized location of each source; the first is the origin.
    pub locations: Vec<Vec<f64>>,
    /// Per source, the glyph's contours, the same structure in each.
    pub paths: Vec<Vec<BezPath>>,
}

/// A master's normalized location and its Private DICT values.
pub type MasterPrivate = (Vec<f64>, PrivateDict);

fn round_tolerant(v: f64) -> f64 {
    let r = ot_round(v);
    if (r - v).abs() <= CFF2_ROUND_TOLERANCE { r } else { v }
}

fn number(n: &Number) -> f64 {
    match n {
        Number::Int(i) => *i as f64,
        Number::Real(r) => *r,
    }
}

/// The Private DICT values of the origin (first) and of each master source at a distinct
/// location.
pub(super) fn master_privates(
    prepared: &Font,
    origin: &Resolved,
    sources: &[Source],
    warnings: &mut Vec<String>,
) -> Result<Vec<MasterPrivate>, CompileError> {
    let zero = vec![0.0; sources.first().map_or(0, |s| s.normalized.len())];
    let mut out: Vec<MasterPrivate> = vec![(zero.clone(), crate::cff::private_dict(origin, None, warnings))];
    for s in sources {
        let Some(id) = s.master.as_ref() else { continue };
        if s.normalized == zero || out.iter().any(|(l, _)| *l == s.normalized) {
            continue;
        }
        let resolved = crate::drawing::resolve(prepared, &Target::Master(id.clone()))?;
        out.push((s.normalized.clone(), crate::cff::private_dict(&resolved, None, &mut Vec::new())));
    }
    Ok(out)
}

/// Region pool and region lists of the variation store being built.
#[derive(Default)]
struct Store {
    regions: Vec<Vec<RegionAxis>>,
    by_region: HashMap<Vec<u64>, u16>,
    lists: Vec<Vec<u16>>,
    by_list: HashMap<Vec<u16>, u16>,
}

impl Store {
    fn region(&mut self, r: &Region) -> Result<u16, CompileError> {
        let key: Vec<u64> = r.0.iter().flat_map(|a| [a.start, a.peak, a.end]).map(|v| (v + 0.0).to_bits()).collect();
        if let Some(&i) = self.by_region.get(&key) {
            return Ok(i);
        }
        let i = u16::try_from(self.regions.len()).map_err(|_| too_many("regions"))?;
        self.regions.push(r.0.iter().map(|a| RegionAxis { start: a.start, peak: a.peak, end: a.end }).collect());
        self.by_region.insert(key, i);
        Ok(i)
    }

    /// The region list of `model`'s regions besides the default.
    fn list(&mut self, model: &VariationModel) -> Result<u16, CompileError> {
        let list = model.regions().iter().skip(1).map(|r| self.region(r)).collect::<Result<Vec<u16>, _>>()?;
        if let Some(&i) = self.by_list.get(&list) {
            return Ok(i);
        }
        let i = u16::try_from(self.lists.len()).map_err(|_| too_many("region lists"))?;
        self.lists.push(list.clone());
        self.by_list.insert(list, i);
        Ok(i)
    }
}

fn too_many(what: &str) -> CompileError {
    CompileError::Table { table: "CFF2", message: format!("more {what} than a CFF2 table can hold") }
}

/// The drawing steps of each source of `g`, the same kinds in each.
fn source_ops(g: &CubicSources) -> Result<Vec<Vec<PathOp>>, CompileError> {
    let joined: Vec<BezPath> = g
        .paths
        .iter()
        .map(|list| {
            let mut p = BezPath::new();
            list.iter().for_each(|c| p.extend(c.clone()));
            p
        })
        .collect();
    let same = |a: &PathOp, b: &PathOp| std::mem::discriminant(a) == std::mem::discriminant(b);
    for drop_closing in [true, false] {
        let ops: Vec<Vec<PathOp>> = joined.iter().map(|p| crate::cff::path_ops(p, drop_closing)).collect();
        let first = ops.first().map_or(&[][..], Vec::as_slice);
        if ops.iter().all(|o| o.len() == first.len() && o.iter().zip(first).all(|(a, b)| same(a, b))) {
            return Ok(ops);
        }
    }
    Err(CompileError::Glyph { glyph: g.glyph.clone(), message: "its sources do not draw the same segments, so it cannot vary in CFF2".into() })
}

fn flat(ops: &[PathOp]) -> Vec<f64> {
    let mut v = Vec::with_capacity(ops.len() * 6);
    for op in ops {
        match *op {
            PathOp::MoveTo(p) | PathOp::LineTo(p) => v.extend([p.0, p.1]),
            PathOp::CurveTo(a, b, c) => v.extend([a.0, a.1, b.0, b.1, c.0, c.1]),
        }
    }
    v.into_iter().map(ot_round).collect()
}

/// The blended drawing steps of `default` with one delta list per region (`deltas[r][j]` for
/// the `j`th coordinate).
fn blend_ops(shape: &[PathOp], values: &[f64], deltas: &[Vec<f64>]) -> Vec<BlendOp> {
    let mut j = 0usize;
    let mut next = || {
        let b = Blend { default: values.get(j).copied().unwrap_or(0.0), deltas: deltas.iter().map(|d| d.get(j).copied().unwrap_or(0.0)).collect() };
        j += 1;
        b
    };
    let mut point = || (next(), next());
    shape
        .iter()
        .map(|op| match op {
            PathOp::MoveTo(_) => BlendOp::MoveTo(point()),
            PathOp::LineTo(_) => BlendOp::LineTo(point()),
            PathOp::CurveTo(..) => {
                let (a, b, c) = (point(), point(), point());
                BlendOp::CurveTo(a, b, c)
            }
        })
        .collect()
}

/// Blends one value per master over `model` (masters in the caller's order). The deltas are not
/// rounded, as fontTools merges Private DICTs, except for floating-point noise around whole
/// numbers.
fn blend_values(model: &VariationModel, values: &[f64]) -> Result<Blend, CompileError> {
    let rows: Vec<Vec<f64>> = values.iter().map(|v| vec![*v]).collect();
    let snap = |v: f64| if (v - v.round()).abs() < 1e-9 { v.round() } else { v };
    let d = model.deltas_with(&rows, snap).map_err(|e| CompileError::Table { table: "CFF2", message: e.to_string() })?;
    let default = d.first().and_then(|r| r.first()).copied().unwrap_or(0.0);
    Ok(Blend { default, deltas: d.iter().skip(1).map(|r| r.first().copied().unwrap_or(0.0)).collect() })
}

/// The Private DICT of the font: the origin's values, with the lists and stems that every master
/// gives in the same shape blended over the masters' model.
fn private(model: &VariationModel, masters: &[MasterPrivate], warnings: &mut Vec<String>) -> Result<Cff2Private, CompileError> {
    let Some((_, origin)) = masters.first() else { return Ok(Cff2Private::default()) };
    let mut list = |what: &str, get: fn(&PrivateDict) -> &Vec<Number>| -> Result<Vec<Blend>, CompileError> {
        let lists: Vec<&Vec<Number>> = masters.iter().map(|(_, p)| get(p)).collect();
        let len = get(origin).len();
        // A varying list is one blend of every value, which must fit the operand stack.
        let fits = len.saturating_mul(model.regions().len()).saturating_add(1) <= fontcraft_cff::cff2::MAX_STACK;
        if !fits {
            warnings.push(format!("{what}: the font has too many masters to vary it, so the origin's values are written for every location"));
            return Ok(get(origin).iter().map(|n| Blend::fixed(number(n))).collect());
        }
        if lists.iter().any(|l| l.len() != len) {
            let counts: Vec<String> = lists.iter().map(|l| l.len().to_string()).collect();
            warnings.push(format!("{what}: the masters have {} values, so the origin's are written for every location", counts.join(", ")));
            return Ok(get(origin).iter().map(|n| Blend::fixed(number(n))).collect());
        }
        (0..len).map(|j| blend_values(model, &lists.iter().map(|l| l.get(j).map_or(0.0, number)).collect::<Vec<_>>())).collect()
    };
    let blue_values = list("BlueValues", |p| &p.blue_values)?;
    let other_blues = list("OtherBlues", |p| &p.other_blues)?;
    let family_blues = list("FamilyBlues", |p| &p.family_blues)?;
    let family_other_blues = list("FamilyOtherBlues", |p| &p.family_other_blues)?;
    let stem_snap_h = list("StemSnapH", |p| &p.stem_snap_h)?;
    let stem_snap_v = list("StemSnapV", |p| &p.stem_snap_v)?;
    let stem = |get: fn(&PrivateDict) -> Option<Number>| -> Result<Option<Blend>, CompileError> {
        let values: Option<Vec<f64>> = masters.iter().map(|(_, p)| get(p).map(|n| number(&n))).collect();
        match values {
            Some(v) => blend_values(model, &v).map(Some),
            None => Ok(get(origin).map(|n| Blend::fixed(number(&n)))),
        }
    };
    let has_blues = !(origin.blue_values.is_empty() && origin.other_blues.is_empty());
    let all = |get: fn(&PrivateDict) -> Number| -> Result<Option<Blend>, CompileError> {
        if !has_blues {
            return Ok(None);
        }
        blend_values(model, &masters.iter().map(|(_, p)| number(&get(p))).collect::<Vec<_>>()).map(Some)
    };
    Ok(Cff2Private {
        blue_values,
        other_blues,
        family_blues,
        family_other_blues,
        std_hw: stem(|p| p.std_hw)?,
        std_vw: stem(|p| p.std_vw)?,
        stem_snap_h,
        stem_snap_v,
        blue_scale: all(|p| p.blue_scale)?,
        blue_shift: all(|p| p.blue_shift)?,
        blue_fuzz: all(|p| p.blue_fuzz)?,
        language_group: Some(number(&origin.language_group) as i64),
    })
}

/// The `CFF2` contents of `glyphs` (in glyph order) for a font with `axis_count` axes, whose
/// masters (origin first) give the Private DICT.
pub fn cff2_font(
    axis_count: u16,
    glyphs: &[CubicSources],
    masters: &[MasterPrivate],
    units_per_em: u16,
    warnings: &mut Vec<String>,
) -> Result<Cff2Font, CompileError> {
    let bad = |glyph: &str, e: String| CompileError::Glyph { glyph: glyph.to_string(), message: e };
    let mut store = Store::default();
    let master_locations: Vec<Vec<f64>> = masters.iter().map(|(l, _)| l.clone()).collect();
    let master_model = VariationModel::new(&master_locations, false).map_err(|e| bad("", e.to_string()))?;
    store.list(&master_model)?;
    let private = private(&master_model, masters, warnings)?;

    let mut models = Models::default();
    let mut out = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        let ops = source_ops(g)?;
        let shape = ops.first().map_or(&[][..], Vec::as_slice);
        if g.locations.len() < 2 {
            let values = flat(shape);
            out.push(Cff2Glyph { name: g.glyph.clone(), vsindex: 0, path: blend_ops(shape, &values, &[]) });
            continue;
        }
        if g.locations.len() != ops.len() {
            return Err(bad(&g.glyph, format!("it has {} source locations but {} source outlines", g.locations.len(), ops.len())));
        }
        let model = models.get(&g.locations).map_err(|e| bad(&g.glyph, e))?;
        let values: Vec<Vec<f64>> = ops.iter().map(|o| flat(o)).collect();
        let deltas = model.deltas_with(&values, round_tolerant).map_err(|e| bad(&g.glyph, e.to_string()))?;
        let vsindex = store.list(model)?;
        let default = deltas.first().cloned().unwrap_or_default();
        let rest = deltas.get(1..).unwrap_or(&[]);
        out.push(Cff2Glyph { name: g.glyph.clone(), vsindex, path: blend_ops(shape, &default, rest) });
    }
    let upm = f64::from(units_per_em);
    let font_matrix = (units_per_em != 1000 && units_per_em > 0).then(|| [1.0 / upm, 0.0, 0.0, 1.0 / upm, 0.0, 0.0]);
    Ok(Cff2Font {
        axis_count,
        regions: store.regions,
        region_lists: store.lists,
        glyphs: out,
        private,
        font_matrix,
        round_tolerance: CFF2_ROUND_TOLERANCE,
        generic: false,
    })
}

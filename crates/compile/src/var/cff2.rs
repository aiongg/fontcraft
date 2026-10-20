//! The `CFF2` flavour of the variable compile (M9.13).
//!
//! Rows: `compile.cff2VariableBuild`, `compile.cff2CompanionTables`.
//!
//! Each glyph's cubic sources become blended charstrings for
//! [`fontcraft_cff::cff2::write_cff2`]: the sources' coordinates are rounded to whole units, as
//! fontmake rounds its CFF masters, and the deltas come from the glyph's variation model with
//! fontTools' tolerance of 0.01 unit, so intermediate sources can give fractional deltas. Glyphs
//! with the same sources share a region list; list 0 is the masters' model, which the Private
//! DICT's blended blue zones and stems use. Each master's Private DICT values are those its
//! static CFF export writes, family blues from Family Alignment Zones included. Sources must
//! draw the same segments; the closing line of a contour is left out only where every source
//! can leave it out.
//!
//! Companion tables: `hmtx` with `HVAR`, `post` 2.0 with the glyph names, `maxp` 0.5; no `glyf`,
//! `loca` or `gvar`.
//!
//! Hints: per master, the hints a static CFF export of that master writes (manual PostScript
//! hints resolved through node links, else the autohinter's when Autohint is on), merged into one
//! structure as fontTools merges hinted masters ([`fontcraft_cff::cff2::shared_hints`]), with
//! the stem edges and widths blended over the glyph's model. Flex is dropped, as the merge draws
//! it as two curves. A glyph whose masters' hints differ in structure, or that varies with an
//! intermediate layer, is written without hints, with a warning; fontTools refuses such a merge.
//! So is a glyph whose stem blends do not fit the charstring's operand stack ([`write`]).

use std::collections::HashMap;

use fontcraft_autohint::FdDict;
use fontcraft_cff::cff2::{Blend, BlendOp, BlendStem, Cff2Built, Cff2Font, Cff2Glyph, Cff2Hints, Cff2Private, RegionAxis};
use fontcraft_cff::{CharstringHints, Number, PathOp, PrivateDict, StemHint};
use fontcraft_geom::BezPath;
use fontcraft_geom::variation::{Region, VariationModel};
use fontcraft_hinting::HintSettings;
use fontcraft_model::{Font, Resolved, Target};

use super::Source;
use super::gvar::Models;
use crate::{CompileError, HintingOptions, ot_round};

/// fontTools' rounding tolerance for CFF2 coordinates and deltas (`CFF2CharStringMergePen`).
pub const CFF2_ROUND_TOLERANCE: f64 = 0.01;

/// One glyph's cubic outlines at each of its sources.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CubicSources {
    pub glyph: String,
    /// Normalized location of each source; the first is the origin.
    pub locations: Vec<Vec<f64>>,
    /// Per source, the glyph's contours, the same structure in each.
    pub paths: Vec<Vec<BezPath>>,
    /// Per source, its master; `None` for an intermediate layer. Hints are written only when
    /// every source is a master.
    pub masters: Vec<Option<String>>,
}

/// What a static export of one master hints with.
struct MasterHinting {
    settings: HintSettings,
    /// The autohinter's zones and stems, when the master exports PostScript hints.
    dict: Option<FdDict>,
    autohint: bool,
}

/// The source the glyphs' hints are read from and each master's hint settings.
pub struct HintContext<'a> {
    font: &'a Font,
    masters: HashMap<String, MasterHinting>,
}

impl<'a> HintContext<'a> {
    /// The hint settings of `masters` in `prepared`, read as each master's static export reads
    /// them; `None` when the export writes no hints.
    pub fn new(prepared: &'a Font, masters: &[String], hinting: &HintingOptions, warnings: &mut Vec<String>) -> Result<Option<Self>, CompileError> {
        if hinting.strip {
            return Ok(None);
        }
        let mut out = HashMap::new();
        let mut notes = Vec::new();
        for id in masters {
            let resolved = crate::drawing::resolve(prepared, &Target::Master(id.clone()))?;
            let Some(settings) = crate::cff::hint_settings(prepared, &resolved, &mut notes) else { continue };
            let private = crate::cff::private_dict(&resolved, Some(&settings), &mut Vec::new());
            let autohint = settings.autohint.unwrap_or(hinting.autohint);
            let mut dict_notes = Vec::new();
            let dict = if settings.export_ps_hints { crate::cff_hints::autohint_dict(&private, &resolved.master, &mut dict_notes) } else { None };
            if autohint {
                notes.append(&mut dict_notes);
            }
            out.insert(id.clone(), MasterHinting { settings, dict, autohint });
        }
        push_unique(warnings, notes);
        Ok(Some(HintContext { font: prepared, masters: out }))
    }
}

/// Appends the warnings of `notes` that `warnings` does not hold yet: each master repeats them.
fn push_unique(warnings: &mut Vec<String>, notes: Vec<String>) {
    for n in notes {
        if !warnings.contains(&n) {
            warnings.push(n);
        }
    }
}

/// The hints the static export of `master` writes for glyph `name` drawn as `path`, flex left
/// out; masks name steps of `path`.
fn master_hints(font: &Font, master: &MasterHinting, name: &str, path: &[PathOp], warnings: &mut Vec<String>) -> Option<CharstringHints> {
    if !master.settings.export_ps_hints {
        return None;
    }
    if let Some(manual) = crate::cff_hints::manual_hints(font, &master.settings, name, path, warnings) {
        return (!manual.is_empty()).then(|| CharstringHints { flex: Vec::new(), ..manual });
    }
    let dict = master.dict.as_ref().filter(|_| master.autohint)?;
    if master.settings.disable_autohint_glyphs.iter().any(|n| n == name) {
        return None;
    }
    crate::cff_hints::autohinted_on_steps(name, path, dict, &fontcraft_autohint::Options::default(), warnings)
}

/// The blended hints of glyph `g`, whose sources draw `ops`, over `model` (`None` for a glyph
/// that does not vary), or `None` for an unhinted charstring.
fn glyph_hints(
    ctx: &HintContext<'_>,
    g: &CubicSources,
    ops: &[Vec<PathOp>],
    model: Option<&VariationModel>,
    warnings: &mut Vec<String>,
) -> Result<Option<Cff2Hints>, CompileError> {
    let name = g.glyph.as_str();
    let mut notes = Vec::new();
    let found: Vec<Option<CharstringHints>> = ops
        .iter()
        .enumerate()
        .map(|(slot, path)| {
            let master = g.masters.get(slot).and_then(Option::as_ref).and_then(|id| ctx.masters.get(id));
            master.and_then(|m| master_hints(ctx.font, m, name, path, &mut notes))
        })
        .collect();
    push_unique(warnings, notes);
    if found.iter().all(Option::is_none) {
        return Ok(None);
    }
    let Some(list) = found.into_iter().collect::<Option<Vec<CharstringHints>>>() else {
        warnings.push(format!(
            "glyph `{name}`: some of its sources have no PostScript hints (an intermediate layer, or a master left unhinted), so it was \
             exported without hints"
        ));
        return Ok(None);
    };
    let shared = match fontcraft_cff::cff2::shared_hints(&list, crate::cff::ROUND_TOLERANCE) {
        Ok(s) => s,
        Err(e) => {
            warnings
                .push(format!("glyph `{name}`: its PostScript hints differ in structure between masters ({e}), so it was exported without hints"));
            return Ok(None);
        }
    };
    let blend = |values: Vec<f64>| -> Result<Blend, CompileError> {
        match model {
            Some(m) => blend_with(m, &values, round_tolerant).map_err(|e| CompileError::Glyph { glyph: name.to_string(), message: e }),
            None => Ok(Blend::fixed(values.first().copied().unwrap_or(0.0))),
        }
    };
    let stems = |list: &[Vec<StemHint>]| -> Result<Vec<BlendStem>, CompileError> {
        list.iter()
            .map(|per_master| {
                Ok(BlendStem {
                    edge: blend(per_master.iter().map(|s| s.edge).collect())?,
                    width: blend(per_master.iter().map(|s| s.width).collect())?,
                })
            })
            .collect()
    };
    let hints = Cff2Hints { h_stems: stems(&shared.h_stems)?, v_stems: stems(&shared.v_stems)?, masks: shared.masks, counters: shared.counters };
    Ok((!hints.is_empty()).then_some(hints))
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
    // As the static export reads it: the hint settings give the family zones. Their warnings
    // are the hint context's to report.
    let private = |resolved: &Resolved, warnings: &mut Vec<String>| {
        let settings = crate::cff::hint_settings(prepared, resolved, &mut Vec::new());
        crate::cff::private_dict(resolved, settings.as_ref(), warnings)
    };
    let zero = vec![0.0; sources.first().map_or(0, |s| s.normalized.len())];
    let mut out: Vec<MasterPrivate> = vec![(zero.clone(), private(origin, warnings))];
    for s in sources {
        let Some(id) = s.master.as_ref() else { continue };
        if s.normalized == zero || out.iter().any(|(l, _)| *l == s.normalized) {
            continue;
        }
        let resolved = crate::drawing::resolve(prepared, &Target::Master(id.clone()))?;
        out.push((s.normalized.clone(), private(&resolved, &mut Vec::new())));
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
    let snap = |v: f64| if (v - v.round()).abs() < 1e-9 { v.round() } else { v };
    blend_with(model, values, snap).map_err(|message| CompileError::Table { table: "CFF2", message })
}

/// Blends one value per master over `model`, each delta passed through `round`.
fn blend_with(model: &VariationModel, values: &[f64], round: fn(f64) -> f64) -> Result<Blend, String> {
    let rows: Vec<Vec<f64>> = values.iter().map(|v| vec![*v]).collect();
    let d = model.deltas_with(&rows, round).map_err(|e| e.to_string())?;
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
    let has_blues = [&origin.blue_values, &origin.other_blues, &origin.family_blues, &origin.family_other_blues].iter().any(|l| !l.is_empty());
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
/// masters (origin first) give the Private DICT; with hints when `hints` is given.
pub fn cff2_font(
    axis_count: u16,
    glyphs: &[CubicSources],
    masters: &[MasterPrivate],
    units_per_em: u16,
    hints: Option<&HintContext<'_>>,
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
            let hints = match hints {
                Some(ctx) => glyph_hints(ctx, g, ops.get(..1).unwrap_or(&[]), None, warnings)?,
                None => None,
            };
            out.push(Cff2Glyph { name: g.glyph.clone(), vsindex: 0, path: blend_ops(shape, &values, &[]), hints });
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
        let hints = match hints {
            Some(ctx) => glyph_hints(ctx, g, &ops, Some(model), warnings)?,
            None => None,
        };
        out.push(Cff2Glyph { name: g.glyph.clone(), vsindex, path: blend_ops(shape, &default, rest), hints });
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

/// The `CFF2` table of `font`, with a warning for each glyph written without its hints because a
/// stem's blend does not fit the operand stack.
pub fn write(font: &Cff2Font, warnings: &mut Vec<String>) -> Result<Cff2Built, CompileError> {
    let built = fontcraft_cff::cff2::write_cff2(font).map_err(|e| CompileError::Table { table: "CFF2", message: e.to_string() })?;
    for name in &built.unhinted {
        warnings.push(format!(
            "glyph `{name}`: its PostScript hints vary over more regions than a CFF2 charstring can hold at once, so it was exported without hints"
        ));
    }
    Ok(built)
}

//! Interpolatable masters (M2.9): one font per master, with the same glyphs and the same points in
//! every glyph, and a sparse font per intermediate-layer location.
//!
//! Rows: `compile.outputFormats`, `compile.cu2quAcrossMasters`, `intermediateLayer.sparseInExport`.
//!
//! The masters are drawn by the outline stage of the variable compile ([`super::Outlines`]), which
//! converts every cubic segment in all sources of a glyph at once, so a master font holds the
//! points the variable font built from it would have. What fontmake 3.12.1 writes for
//! `ttf-interpolatable` and `otf-interpolatable` (fontmake Apache-2.0; ufo2ft's
//! `interpolatableTTFCompiler.py`, `interpolatableOTFCompiler.py` and `outlineCompiler.py`, MIT;
//! see `NOTICE`) is followed in:
//!
//! - overlaps are kept; composites stay composites in TrueType, unless a source needs the glyph
//!   drawn as contours, which then happens in every source;
//! - on-curve points that could be implied are kept, and `USE_MY_METRICS` is not set;
//! - CFF charstrings hold only `rlineto` and `rrcurveto`, without subroutines;
//! - every master has its own kerning and mark positions, and the glyphs made of alternate layers;
//! - a sparse font holds `.notdef`, without an outline and with the advance 65,535, which tells
//!   fontTools' `varLib` to leave it out of the variation, the glyphs with a layer at its
//!   location, and, as empty glyphs with that advance, the base glyphs their components need; it
//!   has only the tables of [`SPARSE_TABLES`].
//!
//! A glyph without a layer in one master is drawn there as the variable font would draw it. No
//! hints are written: TrueType instructions would be compiled against each master's own points.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{Affine, BezPath, PathEl, Point};
use fontcraft_interp::{Designspace, GlyphSources, Location};
use fontcraft_model::{Font, Resolved, ResolvedComponent, ResolvedGlyph, Target};
use read_fonts::{FontRef, tables::glyf::CurvePoint};
use write_fonts::FontBuilder;
use write_fonts::tables::glyf::{Contour, SimpleGlyph};
use write_fonts::types::Tag;

use super::{MAX_INTERMEDIATE_LOCATIONS, Outlines, Source, cff2, font_at, gvar};
use crate::outline::{self, Prepared, Settings};
use crate::{CompileError, Compiled, Flavour, Options, i16_of};

/// The tables a sparse font keeps (ufo2ft's `SPARSE_TTF_MASTER_TABLES` and
/// `SPARSE_OTF_MASTER_TABLES`; a CFF font's `post` holds no names and is left out).
const SPARSE_TABLES: [&[u8; 4]; 9] = [b"CFF ", b"VORG", b"glyf", b"head", b"hmtx", b"loca", b"maxp", b"post", b"vmtx"];
/// The advance that tells fontTools' `varLib` a glyph of a sparse font is no source.
const NO_SOURCE_ADVANCE: f64 = 65_535.0;

/// Which font of an interpolatable set this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MasterSource {
    /// The master's id; for a sparse font, the master its first layer belongs to.
    pub master: String,
    /// For a sparse font, its location as an intermediate layer's name states it: `{600, 400}`.
    pub intermediate: Option<String>,
}

/// One font of an interpolatable set.
#[derive(Clone, Debug)]
pub struct MasterFont {
    pub source: MasterSource,
    pub compiled: Compiled,
}

/// `location` as the name of an intermediate layer.
fn layer_name(location: &Location) -> String {
    let values: Vec<String> = location.0.iter().map(|v| v.to_string()).collect();
    format!("{{{}}}", values.join(", "))
}

/// The designspace of `font`, refused when no masters can interpolate in it.
fn designspace(font: &Font) -> Result<Designspace, CompileError> {
    let designspace = Designspace::new(font)?;
    if !designspace.axes().iter().any(|a| a.external.minimum < a.external.maximum) || designspace.masters().len() < 2 {
        return Err(CompileError::Table { table: "glyf", message: "interpolatable masters need two masters and an axis with a range".into() });
    }
    designspace.check_variable_origin()?;
    Ok(designspace)
}

/// The locations of the intermediate layers of `glyphs` inside the axes of `designspace`, each
/// with the master its first layer belongs to; `outside` receives the others.
fn intermediates<'a>(
    designspace: &Designspace,
    glyphs: impl Iterator<Item = &'a GlyphSources>,
    outside: &mut Vec<Location>,
) -> Result<Vec<(Location, String)>, CompileError> {
    let mut out: Vec<(Location, String)> = Vec::new();
    for s in glyphs.flat_map(|g| g.main.iter()).filter(|s| !s.is_master) {
        if out.iter().any(|(l, _)| *l == s.location) || outside.contains(&s.location) {
            continue;
        }
        if out.len().saturating_add(outside.len()) >= MAX_INTERMEDIATE_LOCATIONS {
            return Err(
                fontcraft_interp::InterpError::LimitExceeded { what: "intermediate layer locations", limit: MAX_INTERMEDIATE_LOCATIONS }.into()
            );
        }
        if designspace.to_normalized(&s.location)?.0.iter().any(|v| !(-1.0..=1.0).contains(v)) {
            outside.push(s.location.clone());
        } else {
            out.push((s.location.clone(), s.master.clone()));
        }
    }
    out.sort_by(|a, b| a.0.0.partial_cmp(&b.0.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

/// The fonts [`compile_masters`] makes of `font`, in its order, without compiling them.
pub fn master_sources(font: &Font) -> Result<Vec<MasterSource>, CompileError> {
    let designspace = designspace(font)?;
    let mut sources = Vec::new();
    for g in font.glyphs.iter().filter(|g| g.export) {
        sources.push(GlyphSources::of(font, &designspace, &g.name)?);
    }
    let mut out = masters(&designspace);
    out.extend(
        intermediates(&designspace, sources.iter(), &mut Vec::new())?
            .into_iter()
            .map(|(location, master)| MasterSource { master, intermediate: Some(layer_name(&location)) }),
    );
    Ok(out)
}

fn masters(designspace: &Designspace) -> Vec<MasterSource> {
    designspace.masters().iter().map(|m| MasterSource { master: m.id.clone(), intermediate: None }).collect()
}

/// Compiles every master of `font`, in the designspace's order, then every intermediate location,
/// to fonts whose outlines interpolate. Refuses incompatible glyphs and everything the static
/// compile refuses. Of `options`, the overlap removal, hinting, implied points, component metrics
/// and CFF operators and subroutines are the format's own, and a project compile and CID keying
/// do not apply.
pub fn compile_masters(font: &Font, flavour: Flavour, options: &Options) -> Result<Vec<MasterFont>, CompileError> {
    font.check()?;
    let mut notes: Vec<String> = Vec::new();
    let designspace = designspace(font)?;
    notes.extend(designspace.warnings().iter().cloned());
    let origin = designspace.origin().cloned().ok_or(CompileError::NotYetImplemented("a designspace without an origin"))?;
    super::refuse_incompatible(font, &designspace)?;
    let mut options = master_options(options);

    let aligned = super::featvar::align_alternate_layers(font, &mut notes)?;
    let font: &Font = aligned.as_ref().unwrap_or(font);
    let with_brackets = super::featvar::with_bracket_glyphs(font, &designspace)?;
    let font: &Font = with_brackets.as_ref().unwrap_or(font);
    let switches = fontcraft_color::export::read_switches(font, &Target::Master(origin.id.clone())).unwrap_or_default();
    let split = crate::color::variable::split_layer_glyphs(font, &designspace, &switches, &mut notes)?;
    let prepared = crate::prepare_source(split.as_ref().unwrap_or(font), &mut notes);
    let prepared: &Font = &prepared;
    let mut glyph_sources: HashMap<String, GlyphSources> = HashMap::new();
    for g in prepared.glyphs.iter() {
        glyph_sources.insert(g.name.clone(), GlyphSources::of(prepared, &designspace, &g.name)?);
    }
    super::keep_uneven_corners(prepared, &origin.id, &glyph_sources, &mut options, &mut notes);
    let options = &options;

    // The sources besides the origin, masters first, as the variable compile lists them.
    let mut sources: Vec<Source> = Vec::new();
    let mut sparse: Vec<(MasterSource, Font, Resolved)> = Vec::new();
    let mut gather = |drawn: &Font, target: &Target, master: Option<String>, location: Location| -> Result<Resolved, CompileError> {
        let mut resolved = crate::drawing::resolve(drawn, target)?;
        let source_font = if master.is_some() { font } else { drawn };
        let glyphs = crate::glyph_stages(Some(source_font), &mut resolved, flavour, options, &mut Vec::new())?;
        let metrics = crate::info::derive(&resolved, &options.style_name, &mut Vec::new()).metrics;
        let normalized = designspace.to_normalized(&location)?.0;
        let index = glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        sources.push(Source { master, location, normalized, glyphs, index, metrics });
        Ok(resolved)
    };
    for m in designspace.masters().iter().filter(|m| m.id != origin.id) {
        gather(prepared, &Target::Master(m.id.clone()), Some(m.id.clone()), m.location.clone())?;
    }
    let mut outside = Vec::new();
    let ordered = prepared.glyphs.iter().filter_map(|g| glyph_sources.get(&g.name));
    for (location, master) in intermediates(&designspace, ordered, &mut outside)? {
        let at = font_at(prepared, &designspace, &location)?;
        let source = MasterSource { master, intermediate: Some(layer_name(&location)) };
        let resolved = gather(&at, &at.default_target()?, None, location)?;
        sparse.push((source, at, resolved));
    }
    notes.extend(outside.iter().map(|l| format!("intermediate layers at {:?} lie outside the axes and were left out", l.0)));

    // The masters are the sources a build interpolates, so no curve is sampled.
    let samples = super::hoi::Samples::default();
    let ctx = Outlines {
        designspace: &designspace,
        prepared,
        origin: &origin.id,
        sources: &sources,
        glyph_sources: &glyph_sources,
        samples: &samples,
        flavour,
        keep_implied: true,
    };
    // The origin is compiled first: its outline stage makes the outlines of all masters.
    let mut shared: Option<Shared> = None;
    let mut fonts: Vec<(String, Compiled)> = Vec::new();
    let others = designspace.masters().iter().filter(|m| m.id != origin.id).map(|m| m.id.as_str());
    for id in std::iter::once(origin.id.as_str()).chain(others) {
        let slot = sources.iter().position(|s| s.master.as_deref() == Some(id)).map_or(0, |k| k.saturating_add(1));
        let target = Target::Master(id.to_string());
        let resolved = crate::drawing::resolve(prepared, &target)?;
        let mut stage = |_: &Resolved,
                         glyphs: &[ResolvedGlyph],
                         settings: &Settings,
                         warnings: &mut Vec<String>|
         -> Result<(Vec<Prepared>, crate::Extra), CompileError> {
            if shared.is_none() {
                shared = Some(Shared::build(&ctx, glyphs, settings, warnings)?);
            }
            let Some(made) = shared.as_mut() else { return Err(CompileError::NotYetImplemented("shared outlines")) };
            Ok((made.master(slot, &sources, glyphs, settings, warnings)?, crate::Extra::default()))
        };
        let options = crate::with_instance_feature_axes(options, font, &target);
        fonts.push((id.to_string(), crate::compile_stages_with(Some(font), resolved, flavour, &options, Some(&mut stage), None)?));
    }
    if let Some((_, first)) = fonts.first_mut() {
        notes.append(&mut first.warnings);
        first.warnings = notes;
    }

    let mut out: Vec<MasterFont> = Vec::with_capacity(fonts.len().saturating_add(sparse.len()));
    for source in masters(&designspace) {
        let Some(at) = fonts.iter().position(|(id, _)| *id == source.master) else { continue };
        out.push(MasterFont { source, compiled: fonts.swap_remove(at).1 });
    }
    let Some(mut shared) = shared else { return Ok(out) };
    let first_sparse = sources.len().saturating_sub(sparse.len());
    for (n, (source, at, resolved)) in sparse.into_iter().enumerate() {
        let k = first_sparse.saturating_add(n);
        if let Some(compiled) = shared.sparse(k.saturating_add(1), &sources, &at, &resolved, flavour, options)? {
            out.push(MasterFont { source, compiled });
        }
    }
    Ok(out)
}

/// `options` as an interpolatable master is compiled with them.
fn master_options(options: &Options) -> Options {
    let mut out = options.clone();
    out.remove_overlap = false;
    out.drop_implied_on_curves = false;
    out.use_my_metrics = false;
    out.hinting = crate::HintingOptions { autohint: false, strip: true };
    out.cff.specialise = false;
    out.subroutinize = Some(false);
    out.project = None;
    out.cid = crate::cid::CidOptions::default();
    if out.glyph_data.0.is_none()
        && let Ok(data) = fontcraft_model::glyphdata::GlyphData::new()
    {
        out.glyph_data = crate::DataRef(Some(std::sync::Arc::new(data)));
    }
    out
}

/// The outlines of every glyph at all its sources, in the origin's glyph order.
struct Shared {
    index: HashMap<String, usize>,
    /// The origin's glyphs.
    glyphs: Vec<ResolvedGlyph>,
    outlines: Vec<Prepared>,
    /// Per glyph, its points at each of its sources.
    points: Vec<gvar::SourceOutlines>,
    /// CFF only: per glyph, its cubic contours at each of its sources.
    cubic: Vec<cff2::CubicSources>,
    /// Per glyph, which source each entry is: 0 the origin, k + 1 source k.
    slots: Vec<Vec<usize>>,
    models: gvar::Models,
}

impl Shared {
    fn build(ctx: &Outlines<'_>, glyphs: &[ResolvedGlyph], settings: &Settings, warnings: &mut Vec<String>) -> Result<Self, CompileError> {
        let (outlines, points, cubic) = ctx.build(glyphs, settings, false, None, warnings)?;
        let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
        let mut slots = Vec::with_capacity(glyphs.len());
        for (g, made) in glyphs.iter().zip(&points) {
            let has = |k: &usize| ctx.sources.get(*k).is_some_and(|s| s.index.contains_key(&g.name));
            let mut ks = ctx.own_sources(&g.name, &mut Vec::new());
            // A CFF composite is drawn as contours, so it has its components' intermediate sources too.
            if ctx.flavour == Flavour::Cff && !g.components.is_empty() {
                for k in ctx.component_sources(g, glyphs, &index).into_iter().filter(has) {
                    if !ks.contains(&k) {
                        ks.push(k);
                    }
                }
            }
            let list: Vec<usize> = std::iter::once(0).chain(ks.into_iter().filter(has).map(|k| k.saturating_add(1))).collect();
            if list.len() != made.points.len() {
                return Err(CompileError::Glyph { glyph: g.name.clone(), message: "its sources could not be matched with the masters".into() });
            }
            slots.push(list);
        }
        Ok(Self {
            index: glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect(),
            glyphs: glyphs.to_vec(),
            outlines,
            points,
            cubic,
            slots,
            models: gvar::Models::default(),
        })
    }

    /// The outlines of `glyphs` (the final order of the source `slot`) with the points all sources share.
    fn master(
        &mut self,
        slot: usize,
        sources: &[Source],
        glyphs: &[ResolvedGlyph],
        settings: &Settings,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<Prepared>, CompileError> {
        // A glyph the origin's order lacks has no other source to agree with.
        let alone = if glyphs.iter().all(|g| self.index.contains_key(&g.name)) { Vec::new() } else { outline::prepare(glyphs, settings, warnings)? };
        glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| match self.index.get(&g.name).copied() {
                Some(at) => self.outline(at, slot, sources, g),
                None => Ok(alone.get(i).cloned().unwrap_or(Prepared::Empty)),
            })
            .collect()
    }

    /// The outline of glyph `at` (an index into the origin's order) at `slot`, where it is `g`.
    fn outline(&mut self, at: usize, slot: usize, sources: &[Source], g: &ResolvedGlyph) -> Result<Prepared, CompileError> {
        let (Some(outline), Some(made), Some(slots)) = (self.outlines.get(at), self.points.get(at), self.slots.get(at)) else {
            return Ok(Prepared::Empty);
        };
        let own = slots.iter().position(|s| *s == slot);
        let here = slot.checked_sub(1).and_then(|k| sources.get(k)).map_or(&[][..], |s| s.normalized.as_slice());
        let models = &mut self.models;
        // The values at this source, or what the sources interpolate to there.
        let mut at_slot = |values: &[Vec<f64>]| -> Vec<f64> {
            let first = || values.first().cloned().unwrap_or_default();
            match own {
                Some(p) => values.get(p).cloned().unwrap_or_else(first),
                None if made.locations.len() < 2 => first(),
                None => match models.get(&made.locations).map(|m| m.interpolate(here, values)) {
                    Ok(Ok(v)) => v,
                    _ => first(),
                },
            }
        };
        if let Some(cubic) = self.cubic.get(at) {
            let values: Vec<Vec<f64>> = cubic.paths.iter().map(|paths| coordinates(paths)).collect();
            let path = cubic.paths.first().map(|paths| reshaped(paths, &at_slot(&values))).unwrap_or_default();
            return Ok(if path.elements().is_empty() { Prepared::Empty } else { Prepared::Simple(path) });
        }
        let pairs = |flat: Vec<f64>| -> Vec<(f64, f64)> { flat.as_chunks().0.iter().map(|&[x, y]| (x, y)).collect() };
        Ok(match outline {
            Prepared::Empty => Prepared::Empty,
            Prepared::Composite(_) if own.is_some() => outline::composite(g)?,
            Prepared::Composite(components) => {
                let values: Vec<Vec<f64>> = made.points.iter().map(|ps| ps.iter().flat_map(|p| [p.0, p.1]).collect()).collect();
                Prepared::Composite(moved(components, &pairs(at_slot(&values))))
            }
            Prepared::Points(simple) => {
                let values: Vec<Vec<f64>> = made.points.iter().map(|ps| ps.iter().flat_map(|p| [p.0, p.1]).collect()).collect();
                Prepared::Points(with_points(simple, &pairs(at_slot(&values))))
            }
            Prepared::Simple(path) => Prepared::Simple(path.clone()),
        })
    }

    /// The sparse font of the intermediate source `slot` (see the module notes); `None` when no
    /// glyph of the font has a layer there. `at` is the font interpolated there and `resolved` its
    /// resolved form.
    fn sparse(
        &mut self,
        slot: usize,
        sources: &[Source],
        at: &Font,
        resolved: &Resolved,
        flavour: Flavour,
        options: &Options,
    ) -> Result<Option<Compiled>, CompileError> {
        let Some(source) = slot.checked_sub(1).and_then(|k| sources.get(k)) else { return Ok(None) };
        let own: Vec<usize> = self.slots.iter().enumerate().filter(|(i, s)| *i > 0 && s.contains(&slot)).map(|(i, _)| i).collect();
        if own.is_empty() {
            return Ok(None);
        }
        let mut drawn: HashMap<usize, (ResolvedGlyph, Prepared)> = HashMap::new();
        let mut bases: HashSet<usize> = HashSet::new();
        for &i in &own {
            let Some((_, g)) = self.glyphs.get(i).and_then(|g| source.glyph(&g.name)) else { continue };
            let outline = self.outline(i, slot, sources, g)?;
            if let Prepared::Composite(components) = &outline {
                bases.extend(components.iter().filter_map(|c| self.index.get(&c.base).copied()));
            }
            drawn.insert(i, (g.clone(), outline));
        }
        let mut order = Vec::new();
        let mut outlines = Vec::new();
        for (i, g) in self.glyphs.iter().enumerate() {
            if let Some((g, outline)) = drawn.remove(&i) {
                order.push(g);
                outlines.push(outline);
            } else if i == 0 || bases.contains(&i) {
                let empty = ResolvedGlyph::new(g.name.clone(), NO_SOURCE_ADVANCE);
                order.push(ResolvedGlyph {
                    production_name: g.production_name.clone(),
                    category: g.category.clone(),
                    sub_category: g.sub_category.clone(),
                    ..empty
                });
                outlines.push(Prepared::Empty);
            }
        }
        let mut warnings = Vec::new();
        let bytes = crate::build::assemble(
            Some(at),
            resolved,
            &order,
            &outlines,
            flavour,
            false,
            &crate::layout::LayoutTables::default(),
            None,
            &[],
            &crate::Extra::default(),
            options,
            &mut warnings,
        )?;
        Ok(Some(Compiled {
            bytes: sparse_tables(&bytes, flavour)?,
            glyph_order: order.into_iter().map(|g| g.name).collect(),
            warnings,
            feature_code: None,
        }))
    }
}

/// The tables of a compiled sparse font that it keeps. The underline values of its `post` are the
/// ones fontTools' `varLib` reads as "no source of `MVAR`".
fn sparse_tables(bytes: &[u8], flavour: Flavour) -> Result<Vec<u8>, CompileError> {
    let bad = |message: String| CompileError::Table { table: "glyf", message };
    let font = FontRef::new(bytes).map_err(|e| bad(format!("the sparse master cannot be read back: {e}")))?;
    let mut builder = FontBuilder::new();
    for tag in SPARSE_TABLES.map(Tag::new) {
        if tag == Tag::new(b"post") && flavour == Flavour::Cff {
            continue;
        }
        let Some(data) = font.table_data(tag) else { continue };
        let mut data = data.as_bytes().to_vec();
        if tag == Tag::new(b"post")
            && let Some(underline) = data.get_mut(8..12)
        {
            underline.copy_from_slice(&[0x80, 0, 0x80, 0]);
        }
        builder.add_raw(tag, data);
    }
    Ok(builder.build())
}

/// Every coordinate of `paths`, in drawing order.
fn coordinates(paths: &[BezPath]) -> Vec<f64> {
    paths.iter().flat_map(outline::points).flat_map(|p| [p.x, p.y]).collect()
}

/// `paths` as one path, with its coordinates replaced by `values` in drawing order.
fn reshaped(paths: &[BezPath], values: &[f64]) -> BezPath {
    let mut values = values.as_chunks().0.iter();
    let mut next = |p: Point| values.next().map_or(p, |&[x, y]| Point::new(x, y));
    let mut out = BezPath::new();
    for el in paths.iter().flat_map(|p| p.elements()) {
        out.push(match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(next(p)),
            PathEl::LineTo(p) => PathEl::LineTo(next(p)),
            PathEl::QuadTo(a, p) => PathEl::QuadTo(next(a), next(p)),
            PathEl::CurveTo(a, b, p) => PathEl::CurveTo(next(a), next(b), next(p)),
            PathEl::ClosePath => PathEl::ClosePath,
        });
    }
    out
}

/// `components` placed at `offsets`.
fn moved(components: &[ResolvedComponent], offsets: &[(f64, f64)]) -> Vec<ResolvedComponent> {
    components
        .iter()
        .zip(offsets)
        .map(|(c, &(x, y))| {
            let [a, b, d, e, ..] = c.transform.as_coeffs();
            ResolvedComponent { base: c.base.clone(), transform: Affine::new([a, b, d, e, x, y]) }
        })
        .collect()
}

/// `simple` with its points at `values`, in order.
fn with_points(simple: &SimpleGlyph, values: &[(f64, f64)]) -> SimpleGlyph {
    let mut values = values.iter().copied();
    let mut contours = Vec::with_capacity(simple.contours.len());
    for c in &simple.contours {
        let points: Vec<CurvePoint> = c
            .iter()
            .map(|p| {
                let (x, y) = values.next().unwrap_or((f64::from(p.x), f64::from(p.y)));
                CurvePoint::new(i16_of(x), i16_of(y), p.on_curve)
            })
            .collect();
        contours.push(Contour::from(points));
    }
    let mut out = simple.clone();
    out.contours = contours;
    out.overlaps = false;
    out.recompute_bounding_box();
    out
}

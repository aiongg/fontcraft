//! Colour layers across masters (M11.19): interpolated instances and variable fonts.
//!
//! Rows: `color.export.variableColr`, the variable parts of `color.export.fullColorToColrV1` and
//! `color.export.colrFromPaletteLayers`, `color.export.perInstance`.
//!
//! Matching (`docs/plans/M11.md` § 3, open question 21): the `k`-th palette or full-colour layer
//! of a master matches the `k`-th of the origin master, as both oracles match colour layers by
//! position. An intermediate palette layer is a source of the colour layer whose `n`-th
//! occurrence of the same palette row it is at its location, as fontc and glyphsLib pair them.
//!
//! - A static instance between masters ([`interpolate_color_layers`]): each matched colour layer
//!   is interpolated like a glyph's master layers, its paint attributes mixed with the master
//!   weights, and added to the instance font, so the static colour export reads it as a master's.
//!   iColor and svg layers do not interpolate: the instance takes the origin master's.
//! - A variable font ([`split_layer_glyphs`], [`variation`]): each layer glyph becomes a glyph
//!   of the font before the outlines, with one layer per master, so `gvar` varies it as any glyph;
//!   the paints' numbers at each master become the [`PaintVariation`] the COLR writer turns into
//!   `PaintVar*` paints.

use std::collections::HashMap;

use fontcraft_color::MAX_COLOR_LAYERS;
use fontcraft_color::export::{ColrPlan, Problem, ProblemKind, Switches, has_color_layers, plan_master};
use fontcraft_color::layers::{ColorLayerKind, kind, with_kind};
use fontcraft_color::paint::{
    MasterPaints, Paint, PaintVariation, ShapeRef, interpolate_paint_attrs, numbered_shape_refs, same_structure, shape_refs,
};
use fontcraft_interp::instance::{InterpOptions, interpolate_layer, master_weights};
use fontcraft_interp::{Designspace, GlyphSources, MasterPairs};
use fontcraft_model::{Font, Glyph, Layer, Shape};

use super::existing_layer_glyph;
use crate::CompileError;

/// One vector colour layer of the origin master and what matches it.
struct Slot<'a> {
    origin: &'a Layer,
    /// The matching layer of each master, the origin's first.
    masters: Vec<(&'a str, &'a Layer)>,
    /// The intermediate palette layers paired with it.
    braces: Vec<&'a Layer>,
}

fn is_vector(l: &Layer) -> bool {
    matches!(kind(l), Some(ColorLayerKind::Palette(_) | ColorLayerKind::FullColor))
}

fn same_kind(a: &Layer, b: &Layer) -> bool {
    matches!(
        (kind(a), kind(b)),
        (Some(ColorLayerKind::Palette(_)), Some(ColorLayerKind::Palette(_))) | (Some(ColorLayerKind::FullColor), Some(ColorLayerKind::FullColor))
    )
}

/// The palette and full-colour layers of `glyph` for the origin master, each with the layers of
/// `masters` (the origin first) that match it by position and kind, and its intermediate palette
/// layers. Intermediate layers that pair with nothing are named in `warnings`.
fn slots<'a>(glyph: &'a Glyph, masters: &[&'a str], warnings: &mut Vec<String>) -> Vec<Slot<'a>> {
    let own = |m: &str| -> Vec<&'a Layer> {
        glyph.layers.iter().filter(|l| l.master == m && l.info.attrs.coordinates.is_empty() && is_vector(l)).take(MAX_COLOR_LAYERS).collect()
    };
    let Some(&origin) = masters.first() else { return Vec::new() };
    let mut slots: Vec<Slot<'a>> = own(origin).into_iter().map(|l| Slot { origin: l, masters: Vec::new(), braces: Vec::new() }).collect();
    for &m in masters {
        let layers = own(m);
        let mut matched = vec![false; layers.len()];
        for ((k, l), hit) in layers.iter().enumerate().zip(matched.iter_mut()) {
            if let Some(slot) = slots.get_mut(k)
                && same_kind(slot.origin, l)
            {
                slot.masters.push((m, l));
                *hit = true;
            }
        }
        let braces: Vec<&'a Layer> = glyph
            .layers
            .iter()
            .filter(|l| l.master == m && !l.info.attrs.coordinates.is_empty() && matches!(kind(l), Some(ColorLayerKind::Palette(_))))
            .take(MAX_COLOR_LAYERS)
            .collect();
        for (bi, b) in braces.iter().enumerate() {
            let entry = kind(b);
            let n = braces.iter().take(bi).filter(|o| o.info.attrs.coordinates == b.info.attrs.coordinates && kind(o) == entry).count();
            let k = layers.iter().enumerate().filter(|(_, l)| kind(l) == entry).nth(n).map(|(k, _)| k);
            match k.filter(|&k| matched.get(k).copied().unwrap_or(false)).and_then(|k| slots.get_mut(k)) {
                Some(slot) => slot.braces.push(b),
                None => warnings.push(format!(
                    "glyph `{}`: its intermediate colour layer `{}` matches no colour layer of its master and was left out",
                    glyph.name, b.id
                )),
            }
        }
    }
    slots
}

/// `layer` copied as a plain layer of master `master`: no colour kind, the given id.
fn plain_copy(layer: &Layer, id: &str, master: &str) -> Layer {
    let mut out = layer.clone();
    out.id = id.to_string();
    out.master = master.to_string();
    out.info.attrs = with_kind(&layer.info.attrs, None);
    out
}

/// Adds the interpolated colour layers of `font` to `out`, the instance font of the instance at
/// `index` that `fontcraft_interp` made (one master, master layers only).
///
/// Each matched palette or full-colour layer is interpolated with the instance's options, its
/// intermediate palette layers included, and gets the origin layer's id, name and colour kind;
/// its paint attributes are mixed with the master weights when every master that weighs in has
/// the layer ([`interpolate_paint_attrs`]). A master layer marked full colour gets its paint
/// attributes mixed the same way. A colour layer that cannot be interpolated is the origin
/// master's, with a warning. iColor and svg layers are the origin master's.
pub fn interpolate_color_layers(
    font: &Font,
    index: usize,
    options: &InterpOptions,
    out: &mut Font,
    warnings: &mut Vec<String>,
) -> Result<(), CompileError> {
    if !has_color_layers(font) {
        return Ok(());
    }
    let designspace = Designspace::new(font)?;
    let (Some(origin), Some(target)) = (designspace.origin().map(|m| m.id.clone()), out.masters.first().map(|m| m.id.clone())) else {
        return Ok(());
    };
    let location = designspace.instance_location(font, index)?;
    let weights: HashMap<String, f64> = master_weights(&designspace, &location, options)?.into_iter().collect();
    let weighs = |m: &str| weights.get(m).copied().unwrap_or(0.0);
    let mut masters: Vec<&str> = vec![origin.as_str()];
    masters.extend(designspace.masters().iter().map(|m| m.id.as_str()).filter(|m| *m != origin));

    struct Job<'a> {
        glyph: String,
        probe: String,
        origin: &'a Layer,
        paints: Vec<(&'a [Shape], f64)>,
    }
    let mut probe = font.clone();
    let mut jobs: Vec<Job<'_>> = Vec::new();
    let mut taken: std::collections::HashSet<String> = font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
    for g in font.glyphs.as_arcs().iter().filter(|g| g.export) {
        let Some(mut target_glyph) = out.glyphs.get_mut(&g.name) else { continue };
        // Bitmap and SVG layers cannot interpolate.
        for l in g.layers.iter().filter(|l| l.master == origin && matches!(kind(l), Some(ColorLayerKind::IColor(_) | ColorLayerKind::Svg))) {
            let mut copy = l.clone();
            copy.master.clone_from(&target);
            target_glyph.layers.push(copy);
        }
        if g.layer(&origin).is_some_and(|l| l.info.attrs.full_color) {
            let sources: Option<Vec<(&[Shape], f64)>> =
                masters.iter().filter(|m| weighs(m) != 0.0).map(|m| g.layer(m).map(|l| (l.shapes.as_slice(), weighs(m)))).collect();
            if let (Some(sources), Some(master_layer)) = (sources, target_glyph.layers.iter_mut().find(|l| l.is_master_layer())) {
                interpolate_paint_attrs(&mut master_layer.shapes, &sources);
            }
        }
        drop(target_glyph);
        for (k, slot) in slots(g, &masters, warnings).into_iter().enumerate() {
            let mut name = format!("{}.colorlayer{k}", g.name);
            while !taken.insert(name.clone()) {
                name.push('_');
            }
            let mut glyph = Glyph::new(name.clone());
            glyph.export = false;
            glyph.layers = slot.masters.iter().map(|(m, l)| plain_copy(l, m, m)).collect();
            glyph.layers.extend(slot.braces.iter().map(|b| plain_copy(b, &b.id, &b.master)));
            probe.glyphs.push(glyph);
            let weighed: Vec<&str> = masters.iter().copied().filter(|m| weighs(m) != 0.0).collect();
            let paints = if weighed.iter().all(|m| slot.masters.iter().any(|(x, _)| x == m)) {
                slot.masters.iter().filter(|(m, _)| weighs(m) != 0.0).map(|(m, l)| (l.shapes.as_slice(), weighs(m))).collect()
            } else {
                Vec::new()
            };
            jobs.push(Job { glyph: g.name.clone(), probe: name, origin: slot.origin, paints });
        }
    }
    for job in jobs {
        let interpolated =
            GlyphSources::of(&probe, &designspace, &job.probe).and_then(|sources| interpolate_layer(&probe, &sources, &location, options));
        let mut layer = match interpolated {
            Ok(layer) => layer,
            Err(e) => {
                warnings.push(format!(
                    "glyph `{}`: its colour layer `{}` could not be interpolated ({e}); the instance takes the origin master's",
                    job.glyph, job.origin.id
                ));
                job.origin.clone()
            }
        };
        if !job.paints.is_empty() {
            interpolate_paint_attrs(&mut layer.shapes, &job.paints);
        }
        layer.id.clone_from(&job.origin.id);
        layer.master.clone_from(&target);
        layer.info.name.clone_from(&job.origin.info.name);
        layer.info.attrs = job.origin.info.attrs.clone();
        layer.info.attrs.coordinates.clear();
        if let Some(mut g) = out.glyphs.get_mut(&job.glyph) {
            g.layers.push(layer);
        }
    }
    Ok(())
}

/// The layer glyphs of `paint` by their number ([`numbered_shape_refs`]).
fn numbered_refs(paint: &Paint) -> std::collections::HashMap<usize, &ShapeRef> {
    numbered_shape_refs(paint).into_iter().collect()
}

/// Whether `paint` of `glyph` draws only layers of `origin` (a layered conversion draws other
/// masters' layers, which do not vary).
fn own_layers(glyph: &Glyph, paint: &Paint, origin: &str) -> bool {
    shape_refs(paint).iter().all(|s| glyph.layers.iter().find(|l| l.id == s.layer).is_some_and(|l| l.master == origin))
}

/// The layer glyphs a variable compile makes glyphs of the font: per colour glyph of `colr`
/// (planned at `origin`, by index), its layer glyphs `(i, <glyph>.color<i>)` that the source does
/// not already have ([`existing_layer_glyph`]) and whose name no glyph has. Glyphs whose paint
/// draws other masters' layers are left out.
pub fn split_names(font: &Font, colr: &ColrPlan, origin: &str) -> Vec<(usize, Vec<(usize, String)>)> {
    let mut out = Vec::new();
    for (gi, cg) in colr.glyphs.iter().enumerate() {
        let Some(source) = font.glyph(&cg.glyph) else { continue };
        if !own_layers(source, &cg.paint, origin) {
            continue;
        }
        let names: Vec<(usize, String)> = numbered_shape_refs(&cg.paint)
            .into_iter()
            .filter(|(i, s)| existing_layer_glyph(source, &cg.glyph, *i, s, origin).is_none())
            .map(|(i, _)| (i, format!("{}.color{i}", cg.glyph)))
            .filter(|(_, name)| font.glyph(name).is_none())
            .collect();
        if !names.is_empty() {
            out.push((gi, names));
        }
    }
    out
}

/// The layer of master `master` that draws layer glyph `shape` of `glyph`: the shapes it names,
/// a full-colour layer's fills without their strokes and its strokes without their fills (the
/// stroke code expands them when the glyph is drawn), with the colour layer's width.
fn layer_glyph_layer(glyph: &Glyph, shape: &ShapeRef, master: &str) -> Option<Layer> {
    let layer = glyph.layers.iter().find(|l| l.id == shape.layer)?;
    let mut shapes: Vec<Shape> = shape.shapes.iter().filter_map(|&i| layer.shapes.get(i).cloned()).collect();
    if layer.info.attrs.full_color {
        if shape.stroke.is_some() {
            shapes.retain(|s| matches!(s, Shape::Path(_)));
        }
        for s in &mut shapes {
            if let Shape::Path(p) = s {
                if shape.stroke.is_some() {
                    p.info.attrs.fill = false;
                } else {
                    p.info.attrs.stroke_width = None;
                }
            }
        }
    } else if shape.stroke.is_some() {
        return None;
    }
    let mut out = Layer::new(master, layer.width);
    out.id = master.to_string();
    out.master = master.to_string();
    out.shapes = shapes;
    Some(out)
}

/// `font` with the layer glyphs [`split_names`] lists made glyphs of their own, after the
/// font's glyphs, for a variable compile; `None` when there are none.
///
/// A layer glyph has one layer per master of `designspace`: the shapes of layer glyph `i` of that
/// master's paint where it has the default master's structure ([`same_structure`]), else a copy
/// of the default master's, which does not vary. A palette layer's intermediate palette layers
/// become its intermediate layers. A layer glyph whose layers are not compatible keeps the
/// default master's outline everywhere, with a warning. It exports, has no Unicode value, and
/// takes the base glyph's production name with `.color<i>`.
pub fn split_layer_glyphs(
    font: &Font,
    designspace: &Designspace,
    switches: &Switches,
    warnings: &mut Vec<String>,
) -> Result<Option<Font>, CompileError> {
    let Some(origin) = designspace.origin().map(|m| m.id.clone()) else { return Ok(None) };
    if !has_color_layers(font) {
        return Ok(None);
    }
    let plan = plan_master(font, &origin, switches);
    let Some(colr) = plan.colr.as_ref().filter(|_| !plan.refused()) else { return Ok(None) };
    let names = split_names(font, colr, &origin);
    if names.is_empty() {
        return Ok(None);
    }
    let others: Vec<&str> = designspace.masters().iter().map(|m| m.id.as_str()).filter(|m| *m != origin).collect();
    let mut all: Vec<&str> = vec![origin.as_str()];
    all.extend(others.iter().copied());
    let scope = MasterPairs { masters: designspace.masters().iter().map(|m| m.id.clone()).collect(), indicators: true };
    let mut out = font.clone();
    let mut per_master: Vec<MasterPaints<'_>> = others.iter().map(|m| MasterPaints::new(font, m)).collect();
    for (gi, split) in names {
        let (Some(cg), Some(source)) = (colr.glyphs.get(gi), colr.glyphs.get(gi).and_then(|cg| font.glyph(&cg.glyph))) else { continue };
        let refs = numbered_refs(&cg.paint);
        let paints: Vec<Option<Paint>> =
            per_master.iter_mut().map(|mp| mp.paint(source, &mut Vec::new()).ok().flatten().filter(|p| same_structure(&cg.paint, p))).collect();
        let slots = slots(source, &all, warnings);
        for (i, name) in split {
            let Some(default) = refs.get(&i).and_then(|s| layer_glyph_layer(source, s, &origin)) else { continue };
            let mut glyph = Glyph::new(name.clone());
            glyph.export = true;
            glyph.info.production = source.info.production.as_ref().map(|p| format!("{p}.color{i}"));
            let copy = |m: &str| plain_copy(&default, m, m);
            let mut layers = vec![default.clone()];
            for (m, paint) in others.iter().zip(&paints) {
                let own = paint.as_ref().and_then(|p| numbered_refs(p).get(&i).and_then(|s| layer_glyph_layer(source, s, m)));
                layers.push(own.unwrap_or_else(|| copy(m)));
            }
            let whole_palette_layer = refs.get(&i).and_then(|s| slots.iter().find(|slot| slot.origin.id == s.layer));
            let braces: Vec<Layer> =
                whole_palette_layer.map(|slot| slot.braces.iter().map(|b| plain_copy(b, &b.id, &b.master)).collect()).unwrap_or_default();
            glyph.layers = layers.iter().cloned().chain(braces).collect();
            out.glyphs.push(glyph);
            let compatible = GlyphSources::of(&out, designspace, &name).is_ok_and(|s| fontcraft_interp::compat::check_glyph(&out, &s, &scope).ok);
            if !compatible {
                warnings.push(format!(
                    "glyph `{}`: its colour layer glyph `{name}` is not compatible between the masters, so its outline does not vary",
                    cg.glyph
                ));
                if let Some(mut g) = out.glyphs.get_mut(&name) {
                    g.layers = all.iter().map(|m| copy(m)).collect();
                }
            }
        }
    }
    Ok(Some(out))
}

/// The paint of each colour glyph of `colr` (planned at `origin`) at each of `others` (the
/// variable font's other master sources with their normalized locations), and the layer glyphs
/// [`split_layer_glyphs`] makes. A glyph whose paint at a master does not have the default
/// master's structure does not vary there, with a warning naming the masters.
pub fn variation(font: &Font, colr: &ColrPlan, origin: &str, others: &[(String, Vec<f64>)], problems: &mut Vec<Problem>) -> PaintVariation {
    let mut out = PaintVariation { locations: others.iter().map(|(_, l)| l.clone()).collect(), ..PaintVariation::default() };
    out.layer_glyphs = split_names(font, colr, origin).into_iter().flat_map(|(_, names)| names.into_iter().map(|(_, n)| n)).collect();
    let mut per_master: Vec<MasterPaints<'_>> = others.iter().map(|(m, _)| MasterPaints::new(font, m)).collect();
    for cg in &colr.glyphs {
        let Some(source) = font.glyph(&cg.glyph) else { continue };
        if !own_layers(source, &cg.paint, origin) {
            continue;
        }
        let mut missing: Vec<String> = Vec::new();
        let per: Vec<Option<Paint>> = others
            .iter()
            .zip(per_master.iter_mut())
            .map(|((m, _), mp)| {
                let p = mp.paint(source, &mut Vec::new()).ok().flatten().filter(|p| same_structure(&cg.paint, p));
                if p.is_none() {
                    missing.push(font.master(m).map_or_else(|_| m.clone(), |master| master.name.clone()));
                }
                p
            })
            .collect();
        if !missing.is_empty() {
            problems.push(Problem {
                kind: ProblemKind::Warning,
                glyph: Some(cg.glyph.clone()),
                message: format!(
                    "its colour layers in master {} do not match the default master's, so its paints do not vary there",
                    missing.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
                ),
            });
        }
        out.glyphs.insert(cg.glyph.clone(), per);
    }
    out
}

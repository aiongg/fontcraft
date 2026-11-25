//! PostScript autohinting of glyphs in the font (`fontcraft-autohint`): the autohinter's stems
//! stored on the layers as PostScript hints, which from then on count as manual hints.
//!
//! Rows: `autohint.ps.run`, `hint.ps.autohintGlyph`.
//!
//! Each glyph is hinted as a CFF export of the master would hint it
//! (`fontcraft_compile::cff_hints`): the master's outline with components decomposed, drawn as
//! a charstring draws it, against the zones and stems of the Private dict that export writes.
//! Only stems are stored: a layer holds stem and ghost hints, and an export builds the hint
//! masks from them again; counter masks and flex are the export's alone. Each edge is linked to
//! the layer's first on-curve node at that position, else stored as a number, so the hints
//! follow the outline into the other masters.
//!
//! Override sets from fontinfo text (`autohint.ps.fontinfoOverrides`) are read against each
//! master's set, so a glyph a set takes is hinted with that set's zones and stems in every
//! master. Overlap handling (`autohint.ps.overlapCorrection`) needs the overlap-free outline,
//! which the autohinter cannot make: it is made here (`cff_hints::overlap_free`) for each layer
//! whose glyph the options select, and passed only when removing overlaps changes the outline.
//!
//! Every run is one undo step; a refusal (an unknown glyph or master, a locked glyph, fontinfo
//! text that cannot be read) changes nothing. A glyph the autohinter cannot hint is listed under
//! `skipped` and left as it was.

use std::collections::{BTreeSet, HashSet};

use fontcraft_autohint::fontinfo::{self, FontInfo};
use fontcraft_autohint::{FdDict, GlyphHints, HintInput, MAX_SEGMENTS, Options, hint_glyph, hint_masters};
use fontcraft_compile::cff::path_ops;
use fontcraft_compile::cff_hints::{self, contours, master_autohint_dict};
use fontcraft_geom::BezPath;
use fontcraft_hinting::ps::{has_ps_hints, set_ps_hints};
use fontcraft_hinting::{Edge, NodeRef, PsHint};
use fontcraft_model::{Font, Layer, Shape};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind};

/// Most glyph layers one run hints (glyphs times masters). The autohinter bounds the work per
/// glyph by its own limits (`fontcraft_autohint::MAX_ELEMENTS`, `MAX_SEGMENTS`).
const MAX_LAYERS_PER_RUN: usize = 1 << 20;
/// How far a node may lie from a stem edge and still carry it, in font units (charstring
/// coordinates are whole units).
const EDGE_TOLERANCE: f64 = 0.5;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "autohint.ps.run",
            "Autohint PostScript",
            [],
            None,
            "{glyphs?: [names] (default: every exporting glyph), excludeGlyphs?: [names], masters?: [id] (default: every master), flex?: bool (default true), hintSubstitution?: bool (default true), maxSegments?: number (a direction with more segments is left unhinted unless glyphs are named; default and at most 100), hintAll?: bool (also glyphs whose layer already has PostScript hints, which are replaced; default false), fontinfo?: text (override sets: FDDict and GlyphSet blocks; once a set is defined, glyphs no set takes are hinted without alignment zones), overlap?: \"auto\"|\"all\"|\"none\" (hint on the outline with overlaps removed: auto for the glyphs in overlapGlyphs and glyphs hinted in several masters; default auto), overlapGlyphs?: [names], looseOverlapMapping?: bool (match the overlap-free outline to the original loosely), allowChanges?: bool (let the autohinter reorder contours and treat nearly straight curves as lines while it works; the outlines stored are not changed), keepConflicts?: bool (keep a stem whose order against another changes between masters), decimal?: bool (keep fractional coordinates instead of rounding them before hinting)} run the PostScript autohinter on the glyphs in each master and store its stems as PostScript hints on the master layers, the masters of one glyph hinted together so their hints match; one undo step → {hinted: [{glyph, master, horizontal, vertical}], skipped: [{glyph, master, reason}], warnings}",
            has_doc,
            run
        ),
        cmd!(
            "hint.ps.autohintGlyph",
            "Autohint",
            ["Glyph", "PostScript Hinting"],
            None,
            "{glyphs: [names], layer?: master id (default: the first master)} run the PostScript autohinter on each glyph's layer of that master and store its stems as the layer's PostScript hints, replacing those it has; the glyph then counts as manually hinted; one undo step → {hinted: [{glyph, master, horizontal, vertical}], skipped: [{glyph, master, reason}], warnings}",
            has_doc,
            autohint_glyph
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

/// One glyph layer hinted, ready to store.
struct Hinted {
    glyph: String,
    master: String,
    hints: Vec<PsHint>,
    counts: (usize, usize),
}

/// What a run found, before anything is stored.
#[derive(Default)]
struct Outcome {
    hinted: Vec<Hinted>,
    skipped: Vec<Value>,
    warnings: Vec<String>,
}

fn run(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("autohint.ps.run", v)?;
    let font = s.font()?;
    let excluded: HashSet<String> = p.opt_strings("excludeGlyphs")?.unwrap_or_default().into_iter().map(str::to_string).collect();
    let named = p.opt_glyphs("glyphs", font)?;
    let explicit = named.is_some();
    let glyphs: Vec<String> = named
        .unwrap_or_else(|| font.glyphs.iter().filter(|g| g.export).map(|g| g.name.clone()).collect())
        .into_iter()
        .filter(|g| !excluded.contains(g))
        .collect();
    let masters = master_list(&p, font)?;
    let max_segments = p.opt_int("maxSegments", 1, MAX_SEGMENTS as i64)?.map_or(MAX_SEGMENTS, |n| usize::try_from(n).unwrap_or(MAX_SEGMENTS));
    let overlap_forcing = match p.opt_str("overlap")? {
        None | Some("auto") => None,
        Some("all") => Some(true),
        Some("none") => Some(false),
        Some(other) => return Err(p.bad(format!("`overlap` is \"auto\", \"all\" or \"none\", not `{other}`"))),
    };
    let overlap_list: BTreeSet<String> = p.opt_strings("overlapGlyphs")?.unwrap_or_default().into_iter().map(str::to_string).collect();
    let options = Options {
        no_flex: !p.bool_or("flex", true)?,
        no_hint_sub: !p.bool_or("hintSubstitution", true)?,
        max_segments,
        explicit_glyphs: explicit,
        allow_changes: p.bool_or("allowChanges", false)?,
        remove_conflicts: !p.bool_or("keepConflicts", false)?,
        round_coords: !p.bool_or("decimal", false)?,
        overlap_forcing,
        overlap_list,
        loose_overlap_mapping: p.bool_or("looseOverlapMapping", false)?,
        ..Options::default()
    };
    let hint_all = p.bool_or("hintAll", false)?;
    let fontinfo = p.opt_str("fontinfo")?;
    let outcome = hint_layers(&p, font, &glyphs, &masters, &options, hint_all, fontinfo)?;
    store(s, "Autohint PostScript", outcome)
}

fn autohint_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.autohintGlyph", v)?;
    let font = s.font()?;
    let glyphs = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    let master = match p.opt_str("layer")? {
        Some(id) => font.master(id).map_err(|_| p.bad(format!("`layer`: there is no master `{id}`")))?.id.clone(),
        None => font.default_master()?.id.clone(),
    };
    let options = Options { explicit_glyphs: true, ..Options::default() };
    let outcome = hint_layers(&p, font, &glyphs, &[master], &options, true, None)?;
    store(s, "Autohint", outcome)
}

/// The masters `masters` names (each checked), else every master.
fn master_list(p: &Params, font: &Font) -> Result<Vec<String>> {
    match p.opt_strings("masters")? {
        Some(ids) => {
            let mut out: Vec<String> = Vec::with_capacity(ids.len());
            for id in ids {
                let m = font.master(id).map_err(|_| p.bad(format!("`masters`: there is no master `{id}`")))?;
                if !out.contains(&m.id) {
                    out.push(m.id.clone());
                }
            }
            Ok(out)
        }
        None => Ok(font.masters.iter().map(|m| m.id.clone()).collect()),
    }
}

/// A master's zone-and-stem set and the override sets read against it.
type MasterDicts = (FdDict, Option<FontInfo>);

/// The set `glyph` is hinted with in a master: its override set, else the master's.
fn glyph_dict<'a>(dicts: &'a MasterDicts, glyph: &str) -> &'a FdDict {
    dicts.1.as_ref().and_then(|info| info.dict_for(glyph)).unwrap_or(&dicts.0)
}

/// Hints `glyphs` in `masters` without changing the font. `replace`: hint layers that already
/// have PostScript hints too. `fontinfo`: override sets, read against each master's set.
fn hint_layers(
    p: &Params,
    font: &Font,
    glyphs: &[String],
    masters: &[String],
    options: &Options,
    replace: bool,
    fontinfo: Option<&str>,
) -> Result<Outcome> {
    if glyphs.len().saturating_mul(masters.len()) > MAX_LAYERS_PER_RUN {
        return Err(p.bad(format!("{} glyphs in {} masters: one run hints at most {MAX_LAYERS_PER_RUN} layers", glyphs.len(), masters.len())));
    }
    let mut out = Outcome::default();
    let names: Vec<String> = if fontinfo.is_some() { font.glyphs.iter().map(|g| g.name.clone()).collect() } else { Vec::new() };
    let upm = f64::from(font.units_per_em);
    let mut dicts: Vec<Option<MasterDicts>> = Vec::with_capacity(masters.len());
    for m in masters {
        let dict = master_autohint_dict(font, m, &mut out.warnings).map_err(|e| p.bad(format!("master `{m}`: {e}")))?;
        let entry = match (dict, fontinfo) {
            (Some(base), Some(text)) => {
                // The extremes otfautohint takes for a source font, as autohint.ps.listFdDicts.
                let info = fontinfo::parse(text, &base, &names, 2.0 * upm, -upm).map_err(|e| p.bad(format!("`fontinfo`: {e}")))?;
                Some((base, Some(info)))
            }
            (Some(base), None) => Some((base, None)),
            (None, _) => None,
        };
        dicts.push(entry);
    }
    for name in glyphs {
        let glyph = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        crate::check_unlocked(glyph)?;
        // The layers to hint, with their charstring paths.
        let mut layers: Vec<(&str, &FdDict, Vec<BezPath>, Vec<usize>)> = Vec::new();
        for (m, dict) in masters.iter().zip(&dicts) {
            let skip = |out: &mut Outcome, reason: String| out.skipped.push(json!({"glyph": name, "master": m, "reason": reason}));
            let Some(dict) = dict else {
                skip(&mut out, "the master's alignment zones and standard stems cannot be used".into());
                continue;
            };
            let Some(layer) = glyph.layer(m) else {
                skip(&mut out, "the glyph has no layer for this master".into());
                continue;
            };
            if !replace && has_ps_hints(layer) {
                skip(&mut out, "the layer already has PostScript hints".into());
                continue;
            }
            let outline = match font.outline_of(glyph, m) {
                Ok(o) => o,
                Err(e) => {
                    skip(&mut out, e.to_string());
                    continue;
                }
            };
            let (contours, starts) = contours(&path_ops(&outline, true));
            layers.push((m.as_str(), glyph_dict(dict, name), contours, starts));
        }
        // As the autohinter decides (`overlapForcing`): the overlap-free outline is only made
        // for the glyphs it would use it for.
        let wanted = options.overlap_forcing.unwrap_or_else(|| options.overlap_list.contains(name.as_str()) || layers.len() > 1);
        let free: Vec<Option<Vec<BezPath>>> =
            layers.iter().map(|(m, _, contours, _)| if wanted { overlap_free(name, m, contours, &mut out.warnings) } else { None }).collect();
        let inputs: Vec<HintInput> = layers
            .iter()
            .zip(&free)
            .map(|((_, dict, contours, _), free)| HintInput { name, contours, without_overlap: free.as_deref(), dict, options })
            .collect();
        // Masters of one glyph are hinted together; outlines that do not match are hinted one by one.
        let together = if inputs.len() > 1 { hint_masters(&inputs).ok() } else { None };
        let results: Vec<std::result::Result<GlyphHints, String>> = match together {
            Some(list) => list.into_iter().map(Ok).collect(),
            None => inputs.iter().map(|i| hint_glyph(i).map_err(|e| e.to_string())).collect(),
        };
        for ((m, _, _, _), result) in layers.iter().zip(results) {
            let Some(layer) = glyph.layer(m) else { continue };
            match result {
                Ok(h) if !h.is_empty() => {
                    out.warnings.extend(h.warnings.iter().map(|w| format!("glyph `{name}`, master `{m}`: {w}")));
                    let hints = layer_hints(layer, &h);
                    // Checked on a copy, so a layer that cannot take them is skipped, not the run refused.
                    if let Err(e) = set_ps_hints(&mut layer.clone(), &hints) {
                        out.skipped.push(json!({"glyph": name, "master": m, "reason": e.to_string()}));
                        continue;
                    }
                    out.hinted.push(Hinted { glyph: name.clone(), master: m.to_string(), hints, counts: (h.h.len(), h.v.len()) });
                }
                Ok(_) => out.skipped.push(json!({"glyph": name, "master": m, "reason": "the autohinter found no stems"})),
                Err(e) => out.skipped.push(json!({"glyph": name, "master": m, "reason": e})),
            }
        }
    }
    Ok(out)
}

/// `contours` with their overlaps removed, when that changes the outline
/// ([`cff_hints::overlap_free`]); `None`, with a warning when removal fails, otherwise.
fn overlap_free(glyph: &str, master: &str, contours: &[BezPath], warnings: &mut Vec<String>) -> Option<Vec<BezPath>> {
    cff_hints::overlap_free(contours).unwrap_or_else(|why| {
        warnings.push(format!("glyph `{glyph}`, master `{master}`: hinted with its overlaps, which could not be removed: {why}"));
        None
    })
}

/// The autohinter's stems as PostScript hints on `layer`.
fn layer_hints(layer: &Layer, hints: &GlyphHints) -> Vec<PsHint> {
    let mut out = Vec::with_capacity(hints.h.len() + hints.v.len());
    for (horizontal, stems) in [(true, &hints.h), (false, &hints.v)] {
        for s in stems {
            let edge = |v: f64| edge_on(layer, horizontal, v);
            out.push(if horizontal && s.rt < s.lb {
                // A ghost: a top ghost is (edge, edge − 20), a bottom ghost (edge + 21, edge).
                if s.lb - s.rt == 20.0 { PsHint::ghost(true, edge(s.lb)) } else { PsHint::ghost(false, edge(s.rt)) }
            } else {
                PsHint::stem(horizontal, edge(s.lb.min(s.rt)), edge(s.lb.max(s.rt)))
            });
        }
    }
    out
}

/// The layer's first on-curve node at `v` (its y for a horizontal hint, else its x), the nearest
/// within [`EDGE_TOLERANCE`]; else the number.
fn edge_on(layer: &Layer, horizontal: bool, v: f64) -> Edge {
    let mut best: Option<(f64, NodeRef)> = None;
    let paths = layer.shapes.iter().filter_map(|s| match s {
        Shape::Path(p) => Some(p),
        _ => None,
    });
    for (pi, path) in paths.enumerate() {
        for (ni, node) in path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()) {
            let d = ((if horizontal { node.y } else { node.x }) - v).abs();
            if d <= EDGE_TOLERANCE && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, NodeRef::new(pi, ni)));
            }
        }
    }
    best.map_or(Edge::Value(v), |(_, r)| Edge::Node(r))
}

/// Stores what `outcome` hinted as one undo step and reports it.
fn store(s: &mut Session, label: &str, outcome: Outcome) -> Result<Value> {
    let Outcome { hinted, skipped, warnings } = outcome;
    s.edit(label, |font| {
        for h in &hinted {
            let mut glyph = font.glyph_mut(&h.glyph)?;
            let layer =
                glyph.layer_mut(&h.master).ok_or_else(|| crate::EngineError::Internal(format!("glyph `{}` has no layer `{}`", h.glyph, h.master)))?;
            set_ps_hints(layer, &h.hints).map_err(|e| crate::EngineError::Internal(format!("glyph `{}`: {e}", h.glyph)))?;
        }
        Ok(())
    })?;
    let list: Vec<Value> =
        hinted.iter().map(|h| json!({"glyph": h.glyph, "master": h.master, "horizontal": h.counts.0, "vertical": h.counts.1})).collect();
    Ok(json!({"hinted": list, "skipped": skipped, "warnings": warnings}))
}

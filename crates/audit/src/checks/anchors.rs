//! Anchor checks (M12.46). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.
//!
//! `ligatureCaretsMissing` follows FontBakery's `ligature_carets` (Apache-2.0): the ligatures are
//! the outputs of the ligature substitutions of the `liga` feature. No code is copied.

use std::collections::{BTreeSet, HashSet};

use fontcraft_geom::Point;
use fontcraft_model::{Font, Glyph, Layer};
use read_fonts::tables::gsub::SubstitutionSubtables;
use read_fonts::types::Tag;
use read_fonts::{FontRef, TableProvider};

use super::components::{Names, binary_glyph_name};
use crate::{BinaryCtx, GlyphCtx, Hit, LayerInput, Location, Params, Sink, SourceCtx};

/// Most ligature records read from one font's `liga` lookups.
pub(crate) const MAX_LIGATURE_RECORDS: usize = 1_000_000;

/// A layer has two anchors of one name; every one after the first is reported.
pub(crate) fn duplicate_name(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let anchors = &input.layer.anchors;
    let mut seen: HashSet<&str> = HashSet::new();
    for a in anchors {
        if !seen.insert(a.name.as_str()) {
            sink.push(
                Hit::new("duplicate-name", format!("The layer has more than one anchor named {}.", a.name))
                    .at(Point::new(a.x, a.y))
                    .place(Location { anchors: vec![a.name.clone()], ..Location::default() }),
            );
        }
    }
}

/// Is the layer one the audit reads? Backup layers (not a master layer, no layer attributes) are
/// left out, as the audit run leaves them out.
fn audited(layer: &Layer) -> bool {
    layer.is_master_layer() || !layer.info.attrs.is_empty()
}

/// Anchors with a fixed role whatever else the font holds: the origin, cursive entry and exit
/// (also on parts joined in composites), and ligature carets.
fn fixed_role(name: &str) -> bool {
    let n = name.strip_prefix('*').unwrap_or(name);
    let n = n.strip_prefix('#').unwrap_or(n);
    let head = n.split(['_', '.']).next().unwrap_or(n);
    matches!(head, "origin" | "entry" | "exit" | "caret" | "vcaret")
}

/// Glyphs whose anchors serve the shapes they build (corner, cap, segment and brush components).
fn special_glyph(glyph: &Glyph) -> bool {
    ["_corner.", "_cap.", "_segment.", "_brush."].iter().any(|p| glyph.name.starts_with(p))
}

/// The attachment role of an anchor: `top` for `top`, `top_1`, `top_viet`, `*top` and `_top`.
/// The second value says whether it is a mark's attaching anchor (a leading underscore).
fn role(name: &str) -> (&str, bool) {
    let (rest, mark) = match name.strip_prefix('_') {
        Some(r) => (r, true),
        None => (name.strip_prefix('*').unwrap_or(name), false),
    };
    (rest.split('_').next().unwrap_or(rest), mark)
}

/// An anchor that nothing in the font pairs with: a base anchor no mark attaches to, a mark
/// anchor no glyph offers, and neither a fixed role nor a smart-glyph property.
pub(crate) fn unknown_name(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let glyphs = ctx.audit.font.glyphs.as_arcs();
    let mut bases: HashSet<&str> = HashSet::new();
    let mut marks: HashSet<&str> = HashSet::new();
    for g in glyphs.iter().filter(|g| !special_glyph(g)) {
        for layer in g.layers.iter().filter(|l| audited(l)) {
            for a in layer.anchors.iter().filter(|a| !fixed_role(&a.name)) {
                let (r, mark) = role(&a.name);
                if mark {
                    marks.insert(r);
                } else {
                    bases.insert(r);
                }
            }
        }
    }
    for g in glyphs.iter().filter(|g| !special_glyph(g)) {
        let properties: Vec<&str> = g.info.parts.iter().map(|p| p.name.as_str()).collect();
        for layer in g.layers.iter().filter(|l| audited(l)) {
            for a in &layer.anchors {
                if fixed_role(&a.name) || properties.contains(&a.name.as_str()) {
                    continue;
                }
                let (r, mark) = role(&a.name);
                let message = if mark && !bases.contains(r) {
                    format!("No glyph has a {r} anchor for the mark anchor {} to attach to.", a.name)
                } else if !mark && !marks.contains(r) {
                    format!("No mark has a _{r} anchor to attach to {}.", a.name)
                } else {
                    continue;
                };
                let master = (!layer.is_master_layer()).then(|| layer.master.clone());
                sink.push(Hit::new("unknown-name", message).at(Point::new(a.x, a.y)).place(Location {
                    glyph: g.name.clone(),
                    layer: layer.id.clone(),
                    master,
                    anchors: vec![a.name.clone()],
                    ..Location::default()
                }));
            }
        }
    }
}

/// The names glyphs of the font attach to: `top` for an anchor called `_top`.
pub(crate) fn mark_anchors(font: &Font) -> HashSet<String> {
    let mut out = HashSet::new();
    for g in font.glyphs.as_arcs().iter().filter(|g| !special_glyph(g)) {
        for a in g.layers.iter().filter(|l| audited(l)).flat_map(|l| &l.anchors) {
            if let Some(name) = a.name.strip_prefix('_').filter(|n| !n.is_empty()) {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// The master layers of a glyph that hold something.
fn drawn_masters(glyph: &Glyph) -> impl Iterator<Item = &Layer> {
    glyph.layers.iter().filter(|l| l.is_master_layer() && !l.shapes.is_empty())
}

fn on_layer(layer: &Layer, anchors: Vec<String>) -> Location {
    Location { layer: layer.id.clone(), master: Some(layer.master.clone()), anchors, ..Location::default() }
}

/// A base glyph without an anchor the glyph data expects of it and marks of the font attach to.
/// Marks (glyphs the glyph data gives an attaching anchor) are left to
/// [`mark_without_attachment`], and anchors a composite takes over from its components count.
pub(crate) fn missing(ctx: &GlyphCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (font, glyph) = (ctx.audit.font, ctx.glyph);
    if !glyph.export || special_glyph(glyph) {
        return;
    }
    let marks = ctx.audit.mark_anchors();
    if marks.is_empty() {
        return;
    }
    let mut wanted = match ctx.services.default_anchors(font, &glyph.name) {
        Ok(names) => names,
        Err(e) => return sink.skip(e.reason),
    };
    if wanted.iter().any(|n| n.starts_with('_')) {
        return;
    }
    wanted.retain(|n| marks.contains(n));
    if wanted.is_empty() {
        return;
    }
    for layer in drawn_masters(glyph) {
        let have = match ctx.services.effective_anchors(font, &glyph.name, &layer.id) {
            Ok(names) => names,
            Err(e) => {
                sink.skip(e.reason);
                continue;
            }
        };
        let lacks: Vec<String> = wanted.iter().filter(|n| !have.contains(n)).cloned().collect();
        if lacks.is_empty() {
            continue;
        }
        let anchors = if lacks.len() == 1 { "anchor" } else { "anchors" };
        let message = format!("{} has no {anchors} {}, which marks in the font attach to.", glyph.name, lacks.join(", "));
        sink.push(Hit::new("missing-anchor", message).place(on_layer(layer, lacks)));
    }
}

/// A mark the glyph data gives an attaching anchor that has none, of its own or from its
/// components.
pub(crate) fn mark_without_attachment(ctx: &GlyphCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (font, glyph) = (ctx.audit.font, ctx.glyph);
    if special_glyph(glyph) {
        return;
    }
    let wanted = match ctx.services.default_anchors(font, &glyph.name) {
        Ok(names) => names,
        Err(e) => return sink.skip(e.reason),
    };
    let expected: Vec<String> = wanted.into_iter().filter(|n| n.starts_with('_')).collect();
    if expected.is_empty() {
        return;
    }
    for layer in drawn_masters(glyph) {
        match ctx.services.effective_anchors(font, &glyph.name, &layer.id) {
            Ok(have) if have.iter().any(|n| n.starts_with('_')) => {}
            Ok(_) => {
                let message = format!("{} is a combining mark without an attaching anchor (such as {}).", glyph.name, expected.join(", "));
                sink.push(Hit::new("no-attaching-anchor", message).place(on_layer(layer, expected.clone())));
            }
            Err(e) => sink.skip(e.reason),
        }
    }
}

/// The glyphs the ligature substitutions of the `liga` feature produce, in glyph-id order.
pub(crate) fn liga_glyphs(font: &FontRef<'_>) -> Vec<u32> {
    let Ok(gsub) = font.gsub() else { return Vec::new() };
    let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else { return Vec::new() };
    let liga = Tag::new(b"liga");
    let mut indices: BTreeSet<u16> = BTreeSet::new();
    for record in features.feature_records().iter().filter(|r| r.feature_tag() == liga) {
        let Ok(feature) = record.feature(features.offset_data()) else { continue };
        indices.extend(feature.lookup_list_indices().iter().map(|i| i.get()));
    }
    let mut out: BTreeSet<u32> = BTreeSet::new();
    let mut records = 0usize;
    for index in indices {
        let Ok(lookup) = lookups.lookups().get(usize::from(index)) else { continue };
        let Ok(SubstitutionSubtables::Ligature(subtables)) = lookup.subtables() else { continue };
        for subtable in subtables.iter().flatten() {
            for set in subtable.ligature_sets().iter().flatten() {
                for ligature in set.ligatures().iter().flatten() {
                    records += 1;
                    if records > MAX_LIGATURE_RECORDS {
                        return out.into_iter().collect();
                    }
                    out.insert(ligature.ligature_glyph().to_u32());
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Ligatures of the `liga` feature without caret positions in `GDEF`.
pub(crate) fn ligature_carets_missing(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let ligatures = liga_glyphs(font);
    if ligatures.is_empty() {
        sink.skip("the font's liga feature makes no ligatures");
        return;
    }
    let count = ligatures.len() as f64;
    // Carets are added in the source; a font given as a binary has no command to fix it.
    let fixable = ctx.source.is_some();
    let font_wide = |code: &'static str, message: String, field: Option<&str>| {
        let hit = Hit::new(code, message).data("ligatures", count).place(Location {
            table: Some("GDEF".into()),
            field: field.map(str::to_string),
            ..Location::default()
        });
        if fixable { hit } else { hit.without_fix() }
    };
    if font.table_data(Tag::new(b"GDEF")).is_none() {
        sink.push(font_wide("no-gdef", format!("The font has {} ligatures but no GDEF table, so no caret positions.", ligatures.len()), None));
        return;
    }
    let covered: HashSet<u32> = match font.gdef().ok().and_then(|g| g.lig_caret_list()).and_then(Result::ok) {
        Some(list) if list.lig_glyph_count() > 0 => list.coverage().map(|c| c.iter().map(|g| g.to_u32()).collect()).unwrap_or_default(),
        _ => {
            sink.push(font_wide(
                "no-carets",
                format!("The font has {} ligatures but no caret positions in GDEF.", ligatures.len()),
                Some("LigCaretList"),
            ));
            return;
        }
    };
    let names = Names::new(font);
    for gid in ligatures.into_iter().filter(|g| !covered.contains(g)) {
        let glyph = binary_glyph_name(ctx, &names, gid);
        let hit = Hit::new("incomplete-carets", format!("The ligature {glyph} has no caret positions.")).place(Location {
            glyph,
            table: Some("GDEF".into()),
            field: Some("LigCaretList".into()),
            ..Location::default()
        });
        sink.push(if fixable { hit } else { hit.without_fix() });
    }
}

#[cfg(test)]
mod tests;

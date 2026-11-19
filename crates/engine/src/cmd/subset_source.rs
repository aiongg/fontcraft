//! `font.subset` on sources (M12.74): `.glyphs`, `.glyphspackage`, `.fontcraft`, UFO and
//! designspace files, written back in their own format.
//!
//! The glyph set is chosen as on a compiled font ([`super::subset`]): glyph names, glyph
//! indices (positions in the glyph order), code points and text, with the bidi mirrors; then
//! `keep.exportFlags`, `keep.excludeGlyphs`, `goadb` and `keep.random` narrow it, or every glyph
//! when nothing else chose any. `.notdef` (when the source has one) and the closure follow: the
//! components of every layer of a kept glyph, and the corner, cap and brush glyphs its hints
//! name, repeated until nothing is added. The compiled font's layout closure has no counterpart:
//! glyphs that only feature rules reach are removed, with those rules.
//!
//! The removed glyphs leave every master and layer and the bitmap strikes; kerning pairs that
//! name one of them, or a kerning group no kept glyph is in, go; the other named groups lose
//! them and go when emptied; class entries and feature code are pruned as
//! [`fea`] describes. Options that only shape the tables of a compiled font have no effect;
//! those that choose glyphs by index range or Font DICT, or set a container, are refused.
//! Native builds only: sources are read and written by path.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use fontcraft_model::{Font, Shape, SourceFormat, side_group};
use fontcraft_tables::subset::{RandomShare, SubsetOptions};
use serde_json::{Map, Value, json};

use super::file_io::{self, OutputRequest};
use crate::params::Params;
use crate::{EngineError, Result};

#[path = "subset_source_fea.rs"]
mod fea;

/// Most items `keep.excludeGlyphs` may hold.
const MAX_SELECTOR_ITEMS: usize = 1_000_000;

/// Parameters that choose glyphs of a compiled font only; given (or switched on) for a source
/// they are refused.
const COMPILED_ONLY: [&str; 4] = ["retainGids", "recommendedGlyphs", "container", "zopfli"];

/// What chose the glyphs, read by [`super::subset`] before the input was known to be a source.
pub(super) struct Selection<'a> {
    pub options: &'a SubsetOptions,
    pub keep: Option<&'a Map<String, Value>>,
    pub random: Option<RandomShare>,
    pub goadb: Option<&'a BTreeSet<String>>,
}

/// Subsets the source at `input` and writes it in its own format.
pub(super) fn run(p: &Params<'_>, input: &Path, request: &OutputRequest, sel: &Selection<'_>) -> Result<Value> {
    use crate::io::SaveFormat;
    for key in COMPILED_ONLY {
        match p.get(key) {
            None | Some(Value::Bool(false)) => {}
            Some(_) => return Err(EngineError::Unsupported(format!("`{key}` applies to compiled fonts, not to a source"))),
        }
    }
    if let Some(keep) = sel.keep {
        for key in ["fd", "excludeFd"] {
            if keep.get(key).is_some_and(|v| !v.is_null()) {
                return Err(EngineError::Unsupported(format!("`keep.{key}` chooses Font DICTs of a compiled CFF font; a source has none")));
            }
        }
    }
    let (mut font, _) = crate::io::open(input, None)?;
    if font.source.format == SourceFormat::Binary {
        return Err(EngineError::Unsupported(format!("subsetting {}: give a compiled font file or a source", input.display())));
    }
    let output = file_io::output_path(input, request, ".subset", None)?;
    let format = SaveFormat::of_path(&output)
        .or_else(|| SaveFormat::of_source(font.source.format))
        .ok_or_else(|| EngineError::Io(format!("cannot tell which format to save {} in", output.display())))?;
    if format == SaveFormat::Designspace && !request.in_place {
        refuse_shared_ufos(input, &output)?;
    }

    let chosen = choose(p, &font, sel)?;
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
    let all: HashSet<String> = names.iter().cloned().collect();
    let removed: HashSet<String> = names.iter().enumerate().filter(|(i, _)| !chosen.kept.contains(i)).map(|(_, n)| n.clone()).collect();
    let report = prune(&mut font, &all, &removed);

    file_io::check_write(&output)?;
    if request.no_clobber && !request.in_place && output.exists() {
        return Err(EngineError::Io(format!("{} exists and noClobber is set", output.display())));
    }
    let created = file_io::create_output_dir(request)?;
    let saved = crate::io::save(&font, &output, format);
    if saved.is_err() {
        file_io::remove_created(&created);
    }
    let saved = saved?;

    let retained: Vec<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    let mut warnings = report.warnings;
    if !removed.is_empty() && !font.kerning.context.is_empty() {
        warnings.push(format!("{} contextual kerning entries are kept as they are; their contexts are not read", font.kerning.context.len()));
    }
    let mut out = json!({
        "output": output.display().to_string(),
        "format": format.name(),
        "glyphCount": retained.len(),
        "retained": retained,
        "removed": removed.len(),
        "kerningPairsRemoved": report.pairs_removed,
        "groupsRemoved": report.groups_removed,
        "classesRemoved": report.classes_removed,
        "featureRulesRemoved": report.rules_removed,
        "missingGlyphs": chosen.missing_glyphs,
        "missingUnicodes": chosen.missing_unicodes.iter().map(|u| format!("U+{u:04X}")).collect::<Vec<_>>(),
        "warnings": warnings,
    });
    if !saved.losses.is_empty()
        && let Value::Object(map) = &mut out
    {
        map.insert("losses".into(), Value::Array(saved.losses));
    }
    Ok(out)
}

/// A designspace written beside its input would write its UFOs over the input's.
fn refuse_shared_ufos(input: &Path, output: &Path) -> Result<()> {
    let text = std::fs::read_to_string(input).map_err(|e| EngineError::Io(format!("cannot read {}: {e}", input.display())))?;
    let ufos = fontcraft_io_ufo::designspace::ufo_paths(&text).map_err(|e| EngineError::Io(format!("{}: {e}", input.display())))?;
    let (from, to) = (input.parent().unwrap_or(Path::new(".")), output.parent().unwrap_or(Path::new(".")));
    for ufo in &ufos {
        if file_io::resolved(&from.join(ufo)) == file_io::resolved(&to.join(ufo)) {
            return Err(EngineError::Io(format!("the subset's UFO {ufo} would replace the input's: give an output in another folder, or inPlace")));
        }
    }
    Ok(())
}

/// The glyphs a subset keeps, by position in the glyph order, and what was asked for but missing.
struct Chosen {
    kept: BTreeSet<usize>,
    missing_glyphs: Vec<String>,
    missing_unicodes: Vec<u32>,
}

fn choose(p: &Params<'_>, font: &Font, sel: &Selection<'_>) -> Result<Chosen> {
    let o = sel.options;
    if o.text.len() > fontcraft_tables::limits::MAX_SUBSET_TEXT {
        return Err(p.bad(format!("`keep.text` is longer than {} bytes", fontcraft_tables::limits::MAX_SUBSET_TEXT)));
    }
    let n = font.glyphs.len();
    let mut by_name: HashMap<&str, usize> = HashMap::with_capacity(n);
    let mut cmap: HashMap<u32, usize> = HashMap::new();
    for (i, g) in font.glyphs.iter().enumerate() {
        by_name.entry(g.name.as_str()).or_insert(i);
        for u in &g.unicodes {
            cmap.entry(*u).or_insert(i);
        }
    }

    let mut kept: BTreeSet<usize> = BTreeSet::new();
    let mut missing_glyphs = Vec::new();
    if o.all_glyphs {
        kept.extend(0..n);
    }
    for name in &o.glyphs {
        match by_name.get(name.as_str()) {
            Some(i) => {
                kept.insert(*i);
            }
            None => missing_glyphs.push(name.clone()),
        }
    }
    for g in &o.gids {
        match usize::try_from(*g).ok().filter(|g| *g < n) {
            Some(i) => {
                kept.insert(i);
            }
            None => missing_glyphs.push(g.to_string()),
        }
    }
    if !missing_glyphs.is_empty() && !o.ignore_missing_glyphs {
        return Err(p.bad(format!("the font has no glyph {}", missing_glyphs.join(", "))));
    }

    let mut unicodes: BTreeSet<u32> = o.text.chars().map(u32::from).collect();
    unicodes.extend(o.unicodes.iter().copied());
    if o.all_unicodes {
        unicodes.extend(cmap.keys().copied());
    }
    if o.bidi_closure {
        let mirrors: Vec<u32> = unicodes.iter().filter_map(|u| fontcraft_tables::subset::mirror::mirrored(*u)).collect();
        unicodes.extend(mirrors);
    }
    kept.extend(unicodes.iter().filter_map(|u| cmap.get(u).copied()));
    let missing_unicodes: Vec<u32> = unicodes.iter().copied().filter(|u| !cmap.contains_key(u)).collect();
    if !missing_unicodes.is_empty() && !o.ignore_missing_unicodes {
        let list: Vec<String> = missing_unicodes.iter().map(|u| format!("U+{u:04X}")).collect();
        return Err(p.bad(format!("the font maps no glyph to {}", list.join(", "))));
    }

    let export_flags = match sel.keep.and_then(|k| k.get("exportFlags")) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(p.bad("`keep.exportFlags` must be true or false")),
    };
    let exclude = match sel.keep.and_then(|k| k.get("excludeGlyphs")).filter(|v| !v.is_null()) {
        Some(v) => excluded(p, v, font, &by_name, &cmap)?,
        None => BTreeSet::new(),
    };
    if export_flags || !exclude.is_empty() || sel.goadb.is_some() || sel.random.is_some() {
        let everything = o.glyphs.is_empty() && o.gids.is_empty() && !o.all_glyphs && o.text.is_empty() && o.unicodes.is_empty() && !o.all_unicodes;
        if everything {
            kept.extend(0..n);
        }
        if export_flags {
            kept.retain(|i| font.glyphs.as_arcs().get(*i).is_some_and(|g| g.export));
        }
        kept.retain(|i| !exclude.contains(i));
        if let Some(listed) = sel.goadb {
            kept.retain(|i| font.glyphs.as_arcs().get(*i).is_some_and(|g| listed.contains(&g.name)));
        }
        if let Some(share) = sel.random {
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            let candidates: BTreeSet<u32> = kept.iter().filter_map(|i| u32::try_from(*i).ok()).collect();
            kept = fontcraft_tables::subset::select::random_share(&candidates, share, &names).into_iter().map(|g| g as usize).collect();
        }
    }
    if o.notdef_glyph
        && let Some(i) = by_name.get(".notdef")
    {
        kept.insert(*i);
    }
    close_over_components(font, &by_name, &mut kept);
    Ok(Chosen { kept, missing_glyphs, missing_unicodes })
}

/// `keep.excludeGlyphs` on a source: glyph names, `*`, glyph indices (`12`, `gid12`), ranges of
/// indices (`3-9`, `3..9`) and code points (`U+0041`).
fn excluded(p: &Params<'_>, v: &Value, font: &Font, by_name: &HashMap<&str, usize>, cmap: &HashMap<u32, usize>) -> Result<BTreeSet<usize>> {
    let items: Vec<&str> = match v {
        Value::String(s) => s.split([',', ' ']).filter(|w| !w.is_empty()).collect(),
        Value::Array(a) => a.iter().map(|v| v.as_str().ok_or_else(|| p.bad("`keep.excludeGlyphs` items must be strings"))).collect::<Result<_>>()?,
        _ => return Err(p.bad("`keep.excludeGlyphs` must be a string or a list of strings")),
    };
    if items.len() > MAX_SELECTOR_ITEMS {
        return Err(p.bad(format!("`keep.excludeGlyphs` lists more than {MAX_SELECTOR_ITEMS} items")));
    }
    let n = font.glyphs.len();
    let gid = |text: &str| -> Option<usize> { text.strip_prefix("gid").unwrap_or(text).parse::<usize>().ok() };
    let mut out = BTreeSet::new();
    for item in items {
        if item == "*" {
            out.extend(0..n);
            continue;
        }
        if item.starts_with('/') {
            return Err(EngineError::Unsupported(format!("`keep.excludeGlyphs`: `{item}` names a CID; a source has none")));
        }
        if let Some(hex) = item.strip_prefix("U+").or_else(|| item.strip_prefix("u+")) {
            let cp = u32::from_str_radix(hex, 16).map_err(|_| p.bad(format!("`keep.excludeGlyphs`: `{item}` is not a code point")))?;
            let i = cmap.get(&cp).ok_or_else(|| p.bad(format!("`keep.excludeGlyphs`: no glyph is mapped to {item}")))?;
            out.insert(*i);
            continue;
        }
        if let Some(i) = by_name.get(item) {
            out.insert(*i);
            continue;
        }
        if let Some(g) = gid(item) {
            if g >= n {
                return Err(p.bad(format!("`keep.excludeGlyphs`: glyph {g} does not exist: the font has {n} glyphs")));
            }
            out.insert(g);
            continue;
        }
        let range = item.split_once("..").or_else(|| item.split_once('-'));
        if let Some((a, b)) = range
            && let (Some(a), Some(b)) = (gid(a), gid(b))
        {
            if a > b {
                return Err(p.bad(format!("`keep.excludeGlyphs`: the range `{item}` runs backwards")));
            }
            if b >= n {
                return Err(p.bad(format!("`keep.excludeGlyphs`: glyph {b} does not exist: the font has {n} glyphs")));
            }
            out.extend(a..=b);
            continue;
        }
        return Err(p.bad(format!("`keep.excludeGlyphs`: no glyph is named `{item}`")));
    }
    // `.notdef` stays, as on a compiled font.
    if let Some(i) = by_name.get(".notdef") {
        out.remove(i);
    }
    Ok(out)
}

/// Adds what kept glyphs draw with: components on every layer, and the glyphs that corner, cap
/// and brush hints name. Each glyph is visited once.
fn close_over_components(font: &Font, by_name: &HashMap<&str, usize>, kept: &mut BTreeSet<usize>) {
    let mut work: Vec<usize> = kept.iter().copied().collect();
    while let Some(i) = work.pop() {
        let Some(glyph) = font.glyphs.as_arcs().get(i) else { continue };
        for layer in &glyph.layers {
            let components = layer.shapes.iter().filter_map(|s| if let Shape::Component(c) = s { Some(c.glyph.as_str()) } else { None });
            let hinted = layer.info.hints.iter().filter_map(|h| h.value.as_dict()?.get("name")?.as_str());
            for name in components.chain(hinted) {
                if let Some(&j) = by_name.get(name)
                    && kept.insert(j)
                {
                    work.push(j);
                }
            }
        }
    }
}

/// What pruning removed besides the glyphs.
#[derive(Default)]
struct Report {
    pairs_removed: usize,
    groups_removed: Vec<String>,
    classes_removed: Vec<String>,
    rules_removed: usize,
    warnings: Vec<String>,
}

/// Removes the glyphs named in `removed` and everything that names them.
fn prune(font: &mut Font, all: &HashSet<String>, removed: &HashSet<String>) -> Report {
    let mut report = Report::default();
    if removed.is_empty() {
        return report;
    }
    font.glyphs.retain(|g| !removed.contains(&g.name));
    if font.bitmap_strikes.iter().any(|s| s.glyphs.iter().any(|g| removed.contains(&g.glyph))) {
        for s in Arc::make_mut(&mut font.bitmap_strikes).iter_mut() {
            s.glyphs.retain(|g| !removed.contains(&g.glyph));
            for g in s.glyphs.iter_mut() {
                g.references.retain(|r| !removed.contains(&r.glyph));
            }
        }
    }

    // A kerning group lives on as long as a kept glyph is in it, on any side.
    let groups: HashSet<&str> = font
        .glyphs
        .iter()
        .flat_map(|g| [&g.kern_groups.left, &g.kern_groups.right, &g.kern_groups.top, &g.kern_groups.bottom])
        .filter_map(|v| v.as_deref())
        .collect();
    let alive = |side: &str| match side_group(side) {
        Some(group) => groups.contains(group),
        None => !removed.contains(side),
    };
    let mut kerning = (*font.kerning).clone();
    for table in [&mut kerning.ltr, &mut kerning.rtl, &mut kerning.vertical] {
        for master in table.0.iter_mut() {
            for first in master.firsts.iter_mut() {
                let before = first.seconds.len();
                if alive(first.first.as_str()) {
                    first.seconds.retain(|(second, _)| alive(second.as_str()));
                } else {
                    first.seconds.clear();
                }
                report.pairs_removed += before - first.seconds.len();
            }
            master.firsts.retain(|f| !f.seconds.is_empty());
        }
        table.0.retain(|m| !m.firsts.is_empty());
    }
    if kerning != *font.kerning {
        font.kerning = Arc::new(kerning);
    }

    if font.groups.iter().any(|g| g.members.iter().any(|m| removed.contains(m))) {
        let groups = Arc::make_mut(&mut font.groups);
        for group in groups.iter_mut() {
            let before = group.members.len();
            group.members.retain(|m| !removed.contains(m));
            if before > 0 && group.members.is_empty() {
                report.groups_removed.push(group.name.clone());
            }
        }
        groups.retain(|g| !(g.members.is_empty() && report.groups_removed.contains(&g.name)));
    }

    let mut pruner = fea::Pruner::new(all, removed);
    let mut features = (*font.features).clone();
    features.classes.retain_mut(|c| match pruner.class_entry(&c.name, &c.code) {
        Some(code) => {
            c.code = code;
            true
        }
        None => false,
    });
    for snippet in features.prefixes.iter_mut().chain(features.features.iter_mut()) {
        snippet.code = pruner.code(&snippet.code);
    }
    if features != *font.features {
        font.features = Arc::new(features);
    }
    report.rules_removed = pruner.rules_removed;
    report.classes_removed = pruner.classes_removed;
    report.warnings = pruner.warnings;
    report
}

#[cfg(test)]
#[path = "subset_source_tests.rs"]
mod tests;

//! Glyph names across the import and export boundary: nice names when a font file opens
//! (`open.appliesNamingScheme`, `fontInfo.useCustomNaming.import`,
//! `user.keepGlyphNamesFromImportedFiles`) and production names in `file.exportUfo`
//! (`export.ufoProductionNames`).
//!
//! Both go through [`fontcraft_model::rename_glyphs`], so components, kerning, groups and the
//! `glyphOrder` parameter follow. Feature code, custom parameters, hints and display strings are
//! text the model does not parse: a glyph whose old name appears there keeps it, so the font
//! still compiles, and the caller is told which glyphs those are. Renamed glyphs keep their
//! places in the glyph order and their code points.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_compile::extras::{GLYPH_SET_KEY, glyph_set_checksum};
use fontcraft_model::glyphdata::{GlyphData, validate};
use fontcraft_model::plist::{Str, Value as PlistValue};
use fontcraft_model::{Family, Font, GLYPH_ORDER_PARAM, MAX_NAME_LEN, ModelError, RenameReport, Renamed, SourceFormat, rename_glyphs};
use serde_json::{Value, json};

use crate::Session;

/// The user setting that keeps the names a font file gives its glyphs.
pub(crate) const KEEP_GLYPH_NAMES: &str = "user.keepGlyphNamesFromImportedFiles";

/// Most glyph names a warning lists.
const MAX_LISTED: usize = 5;

/// Whether opening a file of this format renames glyphs: a UFO, a designspace or a compiled
/// font. `.fontcraft` and `.glyphs` sources already hold the names the user chose.
fn imports(format: SourceFormat) -> bool {
    matches!(format, SourceFormat::Ufo | SourceFormat::Ufoz | SourceFormat::Designspace | SourceFormat::Binary)
}

/// Renames the glyphs of the document just opened (the active one) to their nice names, unless
/// its format keeps names or the font's Use Custom Naming option is on. The renamed font
/// replaces the document as read: no undo step, and the document stays unmodified. Problems
/// become warnings and leave the names as read. Returns how many glyphs were renamed.
pub(crate) fn name_imported(s: &mut Session, warnings: &mut Vec<String>) -> usize {
    let Ok(doc) = s.doc() else { return 0 };
    let read = doc.font.clone();
    if !imports(read.source.format) || read.info.settings.disables_nice_names == Some(true) {
        return 0;
    }
    let data = match s.glyph_data() {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("the glyphs keep the names the file gives them: {e}"));
            return 0;
        }
    };
    let pairs = nice_pairs(&read, &data);
    if pairs.is_empty() {
        return 0;
    }
    let renamed = match rename_followed(&read, pairs) {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!("the glyphs keep the names the file gives them: {e}"));
            return 0;
        }
    };
    if let Err(e) = renamed.font.check_changed(&read) {
        warnings.push(format!("the glyphs keep the names the file gives them: {e}"));
        return 0;
    }
    if !renamed.held.is_empty() {
        warnings.push(format!(
            "{} glyphs keep the names the file gives them because feature code or other text uses those names: {}",
            renamed.held.len(),
            listed(&renamed.held)
        ));
    }
    let count = renamed.report.renamed.len();
    let mut font = renamed.font;
    restamp_stored_tables(&read, &mut font);
    rename_in_recorded_order(&mut font, &renamed.report.renamed);
    let Ok(doc) = s.doc_mut() else { return 0 };
    let font = Arc::new(font);
    doc.font = font.clone();
    doc.saved = font;
    count
}

/// The glyphs whose names the glyph data knows under another nice name, with that name. A nice
/// name that breaks the naming rules, is already a glyph's name or is wanted by an earlier glyph
/// is not used, so no two glyphs end up with one name.
fn nice_pairs(font: &Font, data: &GlyphData) -> Vec<(String, String)> {
    let existing: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut pairs = Vec::new();
    for g in font.glyphs.iter() {
        // `.notdef` and `.null` are fixed names in every format.
        if g.name.starts_with('.') {
            continue;
        }
        let Some(nice) = data.canonical_name(&g.name) else { continue };
        if nice == g.name || nice.len() > MAX_NAME_LEN || validate(&nice, g.export).is_err() || existing.contains(nice.as_str()) {
            continue;
        }
        if taken.insert(nice.clone()) {
            pairs.push((g.name.clone(), nice));
        }
    }
    pairs
}

/// A copy of `font` with every glyph but `.notdef` and `.null` under its production name (its
/// own, else the glyph data's), for `file.exportUfo`, and the losses to report: the glyphs that keep their names. A
/// renamed glyph's own production name is dropped, since it is now the glyph's name.
pub(crate) fn production_copy(font: &Font, data: &GlyphData) -> (Font, Vec<Value>) {
    let existing: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut pairs = Vec::new();
    let mut refused = Vec::new();
    for g in font.glyphs.iter() {
        if g.name.starts_with('.') {
            continue;
        }
        let target = g.info.production.clone().unwrap_or_else(|| data.production_name(&g.name, &g.unicodes));
        if target == g.name {
            continue;
        }
        let usable = !target.is_empty() && target.len() <= MAX_NAME_LEN && validate(&target, g.export).is_ok() && !existing.contains(target.as_str());
        if usable && taken.insert(target.clone()) {
            pairs.push((g.name.clone(), target));
        } else {
            refused.push(g.name.clone());
        }
    }
    let mut losses = Vec::new();
    if !refused.is_empty() {
        losses.push(json!({
            "family": "ufo", "entries": refused.len(),
            "what": "glyphs written under their own names: their production names are empty, break the naming rules or are taken by another glyph",
            "items": refused,
        }));
    }
    if pairs.is_empty() {
        return (font.clone(), losses);
    }
    let renamed = match rename_followed(font, pairs) {
        Ok(r) => r,
        Err(e) => {
            losses.push(json!({ "family": "ufo", "entries": font.glyphs.len(), "what": format!("glyphs written under their own names: {e}") }));
            return (font.clone(), losses);
        }
    };
    if !renamed.held.is_empty() {
        losses.push(json!({
            "family": "ufo", "entries": renamed.held.len(),
            "what": "glyphs written under their own names because feature code or other text uses those names",
            "items": renamed.held,
        }));
    }
    let mut out = renamed.font;
    for r in &renamed.report.renamed {
        if let Ok(mut g) = out.glyph_mut(&r.to)
            && g.info.production.is_some()
        {
            g.info.production = None;
            g.info.tidy();
        }
    }
    (out, losses)
}

/// A font renamed by [`rename_followed`].
struct Followed {
    font: Font,
    report: RenameReport,
    /// Old names left as they are because text the model does not parse uses them.
    held: Vec<String>,
}

/// `font` with `pairs` renamed, except the glyphs whose old names occur in text the rename
/// cannot follow. Dropping a pair cannot make another name occur in that text, so a second pass
/// without the held pairs leaves nothing in it.
fn rename_followed(font: &Font, mut pairs: Vec<(String, String)>) -> Result<Followed, ModelError> {
    let mut first = font.clone();
    let report = rename_glyphs(&mut first, &pairs)?;
    if report.still_used.is_empty() {
        return Ok(Followed { font: first, report, held: Vec::new() });
    }
    let used: HashSet<&str> = report.still_used.iter().map(|r| r.name.as_str()).collect();
    let held: Vec<String> = pairs.iter().filter(|(old, _)| used.contains(old.as_str())).map(|(old, _)| old.clone()).collect();
    pairs.retain(|(old, _)| !used.contains(old.as_str()));
    let mut second = font.clone();
    let report = rename_glyphs(&mut second, &pairs)?;
    Ok(Followed { font: second, report, held })
}

/// Keeps the tables stored from a compiled font usable after its glyphs are renamed: their
/// glyph-order checksum is recomputed for the new names when it described the order as read.
fn restamp_stored_tables(read: &Font, renamed: &mut Font) {
    let old: Vec<&str> = read.glyphs.iter().map(|g| g.name.as_str()).collect();
    let current = renamed.extra.get(Family::Binary, GLYPH_SET_KEY).and_then(PlistValue::as_str).is_some_and(|c| c == glyph_set_checksum(&old));
    if !current {
        return;
    }
    let new: Vec<String> = renamed.glyphs.iter().map(|g| g.name.clone()).collect();
    let checksum = glyph_set_checksum(&new);
    Arc::make_mut(&mut renamed.extra).insert(Family::Binary, GLYPH_SET_KEY, PlistValue::String(Str::new(checksum)));
}

/// Renames the glyphs in the glyph order a compiled font records when it opens (a `glyphOrder`
/// custom parameter kept for the `.glyphs` writer, which the model's rename does not see), so a
/// font saved to a source keeps its order.
fn rename_in_recorded_order(font: &mut Font, renamed: &[Renamed]) {
    let Some(PlistValue::Array(mut params)) = font.extra.get(Family::Glyphs, "customParameters").cloned() else { return };
    let map: HashMap<&str, &str> = renamed.iter().map(|r| (r.from.as_str(), r.to.as_str())).collect();
    let mut changed = false;
    for p in &mut params.items {
        let PlistValue::Dict(d) = p else { continue };
        if d.get("name").and_then(PlistValue::as_str) != Some(GLYPH_ORDER_PARAM) {
            continue;
        }
        let Some(PlistValue::Array(order)) = d.get_mut("value") else { continue };
        for name in &mut order.items {
            if let Some(new) = name.as_str().and_then(|n| map.get(n)) {
                *name = PlistValue::String(Str::new(*new));
                changed = true;
            }
        }
    }
    if changed {
        Arc::make_mut(&mut font.extra).insert(Family::Glyphs, "customParameters", PlistValue::Array(params));
    }
}

/// The first few names, with an ellipsis when there are more.
fn listed(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(MAX_LISTED).map(String::as_str).collect();
    format!("{}{}", shown.join(", "), if names.len() > shown.len() { ", …" } else { "" })
}

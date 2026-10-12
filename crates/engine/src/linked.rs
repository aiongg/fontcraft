//! Linked sources: the Import Font and Import Master parameters. When a document opens and before
//! each export, the files they name are read with the existing readers, and the glyphs the font
//! lacks (Import Font) or the named master with its layers (Import Master) are added, marked
//! linked and locked. The source writers leave linked glyphs and masters out ([`unlinked`], which
//! `io::save` calls before every format's writer).
//!
//! No nesting: a linked file's own Import Font is ignored with a warning. At most
//! [`fontcraft_fontinfo::MAX_IMPORT_FONT`] and [`fontcraft_fontinfo::MAX_IMPORT_MASTER`]
//! parameters; each file within the readers' input limit. A missing file is a warning, not an
//! error. In the browser there are no files to read: the parameters are kept and warned about.
//!
//! Values: Import Font is the file's path, as a string or as a dictionary's `Path` (the form the
//! format's sample files use). Import Master is the path and the master's name or id, as
//! `path;master` or a dictionary's `Path` and `Master`; with no master named, the file's first
//! master. A relative path starts from the folder of the document's file (a package's folder
//! counts as its file).
//!
//! What is added:
//! - Import Font: every glyph of the file whose name the font lacks, appended in the file's
//!   order, locked. Its layers go to the font's masters: a master with the same id, else the
//!   master at the same position; layers of masters with neither are left out.
//! - Import Master: a copy of the master with a fresh id when the font already uses its id, its
//!   axis values matched by axis tag (else name), its metric, stem and number values matched by
//!   definition; the master's layers of every glyph the font has; its kerning.
//!
//! Outlines and metrics are taken as the file holds them: a file with other units per em than the
//! font is not scaled, and a warning says so.
//!
//! Marks: [`fontcraft_model::fc_keys::LINKED_SOURCE`] in the user data of each linked glyph and
//! master, holding the parameter's path. A layer belongs to a linked master by its master id.
//!
//! Rows: `Import Font`, `Import Master` (M7.12).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use fontcraft_fontinfo::{MAX_IMPORT_FONT, MAX_IMPORT_MASTER, MAX_MASTERS};
use fontcraft_model::fc_keys::LINKED_SOURCE;
use fontcraft_model::plist::{Str, Value};
use fontcraft_model::{Font, Glyph, Layer, MAX_GLYPHS, UserData};

use crate::Result;

const IMPORT_FONT: &str = "Import Font";
const IMPORT_MASTER: &str = "Import Master";
/// Most warnings about skipped glyphs named one by one; the rest are counted.
const MAX_NAMED: usize = 8;

/// `font` with its linked sources added, or `None` when it has no Import Font or Import Master
/// parameter and nothing linked. `path` is the document's file, which relative paths start from.
/// Glyphs and masters linked earlier are dropped first, so the result holds what the files hold
/// now.
pub(crate) fn resolve(font: &Font, path: Option<&Path>, warnings: &mut Vec<String>) -> Result<Option<Font>> {
    let fonts: Vec<&Value> = font.info.params.all(IMPORT_FONT).collect();
    let masters: Vec<&Value> = font.info.params.all(IMPORT_MASTER).collect();
    let base = unlinked(font);
    if fonts.is_empty() && masters.is_empty() {
        return Ok(match base {
            Cow::Owned(f) => Some(f),
            Cow::Borrowed(_) => None,
        });
    }
    let mut out = base.into_owned();
    let mut files = Files::default();
    if fonts.len() > MAX_IMPORT_FONT {
        warnings.push(format!("{IMPORT_FONT}: {} parameters, of which the first {MAX_IMPORT_FONT} are read", fonts.len()));
    }
    for value in fonts.iter().take(MAX_IMPORT_FONT) {
        let Some(rel) = font_path(value) else {
            warnings.push(format!("{IMPORT_FONT}: the parameter names no file"));
            continue;
        };
        if let Some(linked) = files.read(IMPORT_FONT, rel, path, warnings) {
            check_upm(&out, &linked, IMPORT_FONT, rel, warnings);
            link_glyphs(&mut out, &linked, rel, warnings);
        }
    }
    if masters.len() > MAX_IMPORT_MASTER {
        warnings.push(format!("{IMPORT_MASTER}: {} parameters, of which the first {MAX_IMPORT_MASTER} are read", masters.len()));
    }
    for value in masters.iter().take(MAX_IMPORT_MASTER) {
        let Some((rel, which)) = master_ref(value) else {
            warnings.push(format!("{IMPORT_MASTER}: the parameter names no file"));
            continue;
        };
        if let Some(linked) = files.read(IMPORT_MASTER, rel, path, warnings) {
            check_upm(&out, &linked, IMPORT_MASTER, rel, warnings);
            link_master(&mut out, &linked, which, rel, warnings);
        }
    }
    // Every glyph was checked as it was added; this catches what only the whole font shows.
    if let Err(e) = out.check() {
        warnings.push(format!("linked sources left out: the font with them is not valid ({e})"));
        return Ok(Some(unlinked(&out).into_owned()));
    }
    Ok(Some(out))
}

/// `font` without its linked glyphs, linked masters, their layers and their kerning: what a source
/// writer writes. Borrowed when nothing is linked.
pub(crate) fn unlinked(font: &Font) -> Cow<'_, Font> {
    let masters: HashSet<&str> = font.masters.iter().filter(|m| is_linked(&m.user_data)).map(|m| m.id.as_str()).collect();
    let glyphs = font.glyphs.iter().any(|g| is_linked(&g.info.user_data));
    if masters.is_empty() && !glyphs {
        return Cow::Borrowed(font);
    }
    let mut out = font.clone();
    if glyphs {
        out.glyphs.retain(|g| !is_linked(&g.info.user_data));
    }
    if !masters.is_empty() {
        Arc::make_mut(&mut out.masters).retain(|m| !masters.contains(m.id.as_str()));
        for i in 0..out.glyphs.len() {
            let has = out.glyphs.as_arcs().get(i).is_some_and(|g| g.layers.iter().any(|l| masters.contains(l.master.as_str())));
            if has && let Some(mut g) = out.glyphs.get_mut_at(i) {
                g.layers.retain(|l| !masters.contains(l.master.as_str()));
            }
        }
        let k = Arc::make_mut(&mut out.kerning);
        for table in [&mut k.ltr, &mut k.rtl, &mut k.vertical] {
            table.0.retain(|e| !masters.contains(e.master.as_str()));
        }
    }
    Cow::Owned(out)
}

fn is_linked(data: &UserData) -> bool {
    data.get(LINKED_SOURCE).is_some()
}

fn mark(data: &mut UserData, source: &str) {
    data.0.insert(LINKED_SOURCE, Value::String(Str::new(source)));
}

/// Drops the linked mark, for a copy of a linked master that belongs to the document itself.
pub(crate) fn unmark(data: &mut UserData) {
    if let Some(i) = data.0.entries.iter().position(|(k, _)| k.as_str() == LINKED_SOURCE) {
        data.0.remove_at(i);
    }
}

/// Import Font's path: a string, or a dictionary's `Path`.
fn font_path(value: &Value) -> Option<&str> {
    let path = match value {
        Value::String(s) => s.as_str(),
        Value::Dict(d) => d.get("Path").or_else(|| d.get("path")).and_then(Value::as_str)?,
        _ => return None,
    };
    let path = path.trim();
    (!path.is_empty()).then_some(path)
}

/// Import Master's path and master: `path;master`, or a dictionary's `Path` and `Master`.
fn master_ref(value: &Value) -> Option<(&str, Option<&str>)> {
    let (path, master) = match value {
        Value::String(s) => match s.as_str().rsplit_once(';') {
            Some((p, m)) => (p, Some(m)),
            None => (s.as_str(), None),
        },
        Value::Dict(d) => {
            let path = d.get("Path").or_else(|| d.get("path")).and_then(Value::as_str)?;
            let master = ["Master", "MasterID", "MasterName", "master"].iter().find_map(|k| d.get(k).and_then(Value::as_str));
            (path, master)
        }
        _ => return None,
    };
    let path = path.trim();
    let master = master.map(str::trim).filter(|m| !m.is_empty());
    (!path.is_empty()).then_some((path, master))
}

/// The linked files read for one resolution, by the path as written, so a file two parameters
/// name is read once.
#[derive(Default)]
struct Files {
    read: HashMap<String, Option<Font>>,
}

impl Files {
    fn read(&mut self, param: &str, rel: &str, host: Option<&Path>, warnings: &mut Vec<String>) -> Option<Font> {
        if let Some(font) = self.read.get(rel) {
            return font.clone();
        }
        let font = load(param, rel, host, warnings);
        self.read.insert(rel.to_string(), font.clone());
        font
    }
}

/// Reads the source `rel` names; a warning naming it when it cannot be read.
#[cfg(not(target_arch = "wasm32"))]
fn load(param: &str, rel: &str, host: Option<&Path>, warnings: &mut Vec<String>) -> Option<Font> {
    let written = Path::new(rel);
    let full = if written.is_absolute() {
        written.to_path_buf()
    } else {
        let Some(dir) = host.and_then(Path::parent) else {
            warnings.push(format!("{param} `{rel}`: the document has no file yet, so a relative path has no folder to start from"));
            return None;
        };
        dir.join(written)
    };
    // The document linking to itself would add nothing; say so instead of reading it again.
    if let (Some(h), Ok(target)) = (host, std::fs::canonicalize(&full))
        && std::fs::canonicalize(h).is_ok_and(|h| h == target)
    {
        warnings.push(format!("{param} `{rel}`: names the document itself"));
        return None;
    }
    match crate::io::open(&full, None) {
        Ok((font, read_warnings)) => {
            warnings.extend(read_warnings.into_iter().map(|w| format!("{param} `{rel}`: {w}")));
            if font.info.params.get(IMPORT_FONT).is_some() || font.info.params.get(IMPORT_MASTER).is_some() {
                warnings.push(format!("{param} `{rel}`: its own {IMPORT_FONT} and {IMPORT_MASTER} parameters are not followed"));
            }
            Some(font)
        }
        Err(e) => {
            warnings.push(format!("{param} `{rel}`: {e}"));
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn load(param: &str, rel: &str, host: Option<&Path>, warnings: &mut Vec<String>) -> Option<Font> {
    let _ = host;
    warnings.push(format!("{param} `{rel}`: linked files cannot be read in the browser; the parameter is kept"));
    None
}

/// The font's master each of `linked`'s masters maps to: the one with the same id, else the
/// one at the same position, each font master taken once.
fn master_map(font: &Font, linked: &Font) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut taken: HashSet<&str> = HashSet::new();
    for m in linked.masters.iter() {
        if let Some(h) = font.masters.iter().find(|h| h.id == m.id && !is_linked(&h.user_data)) {
            taken.insert(h.id.as_str());
            map.insert(m.id.clone(), h.id.clone());
        }
    }
    for (i, m) in linked.masters.iter().enumerate() {
        if map.contains_key(&m.id) {
            continue;
        }
        if let Some(h) = font.masters.get(i).filter(|h| !taken.contains(h.id.as_str()) && !is_linked(&h.user_data)) {
            taken.insert(h.id.as_str());
            map.insert(m.id.clone(), h.id.clone());
        }
    }
    map
}

/// `layer` moved to master `to` (`from` is its master in the linked file).
fn moved(layer: &Layer, from: &str, to: &str) -> Layer {
    let mut l = layer.clone();
    if l.id == from {
        l.id = to.to_string();
    }
    l.master = to.to_string();
    l
}

/// Warns when `linked` has other units per em than `font`: its values are taken unscaled.
fn check_upm(font: &Font, linked: &Font, param: &str, source: &str, warnings: &mut Vec<String>) {
    if linked.units_per_em != font.units_per_em {
        warnings.push(format!(
            "{param} `{source}`: the file has {} units per em and the font {}; its outlines and metrics are used unscaled",
            linked.units_per_em, font.units_per_em
        ));
    }
}

/// Import Font: the glyphs of `linked` the font lacks, appended, locked and marked.
fn link_glyphs(font: &mut Font, linked: &Font, source: &str, warnings: &mut Vec<String>) {
    let map = master_map(font, linked);
    let mut refused: Vec<String> = Vec::new();
    for g in linked.glyphs.iter() {
        if font.glyphs.get(&g.name).is_some() {
            continue;
        }
        if font.glyphs.len() >= MAX_GLYPHS {
            warnings.push(format!("{IMPORT_FONT} `{source}`: the font holds {MAX_GLYPHS} glyphs; the rest are left out"));
            break;
        }
        let mut glyph = Glyph::clone(g);
        glyph.layers = g.layers.iter().filter_map(|l| map.get(&l.master).map(|to| moved(l, &l.master, to))).collect();
        glyph.file_name = None;
        glyph.info.locked = true;
        mark(&mut glyph.info.user_data, source);
        if let Err(e) = font.check_glyph(&glyph) {
            refused.push(format!("`{}` ({e})", g.name));
            continue;
        }
        font.glyphs.push(glyph);
    }
    if !refused.is_empty() {
        let more = refused.len().saturating_sub(MAX_NAMED);
        let mut named = refused.into_iter().take(MAX_NAMED).collect::<Vec<_>>().join(", ");
        if more > 0 {
            named.push_str(&format!(" and {more} more"));
        }
        warnings.push(format!("{IMPORT_FONT} `{source}`: glyphs left out: {named}"));
    }
}

/// A master id the font does not use, from `id`.
fn fresh_id(font: &Font, id: &str) -> String {
    let used = |c: &str| font.masters.iter().any(|m| m.id == c);
    if !used(id) {
        return id.to_string();
    }
    // At most one more candidate than there are masters can be taken.
    (1..=font.masters.len() + 1).map(|n| format!("{id}.linked{n}")).find(|c| !used(c)).unwrap_or_else(|| format!("{id}.linked"))
}

/// Import Master: the master `which` names (else the first) of `linked`, with its layers and
/// kerning, marked.
fn link_master(font: &mut Font, linked: &Font, which: Option<&str>, source: &str, warnings: &mut Vec<String>) {
    let found = match which {
        Some(w) => linked.masters.iter().find(|m| m.id == w).or_else(|| linked.masters.iter().find(|m| m.name == w)),
        None => linked.masters.first(),
    };
    let Some(src) = found else {
        warnings.push(format!("{IMPORT_MASTER} `{source}`: the file has no master `{}`", which.unwrap_or_default()));
        return;
    };
    if font.masters.len() >= MAX_MASTERS {
        warnings.push(format!("{IMPORT_MASTER} `{source}`: the font holds {MAX_MASTERS} masters already"));
        return;
    }
    let id = fresh_id(font, &src.id);
    let mut master = src.clone();
    master.id = id.clone();
    let mut unmatched: Vec<&str> = Vec::new();
    master.location = font
        .axes
        .iter()
        .enumerate()
        .map(|(i, axis)| {
            let j = linked.axes.iter().position(|a| a.tag == axis.tag).or_else(|| linked.axes.iter().position(|a| a.name == axis.name));
            j.and_then(|j| src.location.get(j).copied()).unwrap_or_else(|| {
                unmatched.push(axis.name.as_str());
                font.masters.first().and_then(|m| m.location.get(i).copied()).unwrap_or(0.0)
            })
        })
        .collect();
    if !unmatched.is_empty() {
        warnings.push(format!(
            "{IMPORT_MASTER} `{source}`: no value on the axes {}; the first master's value is used",
            unmatched.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(", ")
        ));
    }
    master.metric_values = font
        .metrics
        .iter()
        .map(|d| {
            let j = linked.metrics.iter().position(|l| l.kind == d.kind && l.name == d.name && l.filter == d.filter);
            j.and_then(|j| src.metric_values.get(j).copied()).unwrap_or_default()
        })
        .collect();
    master.stem_values = font
        .stems
        .iter()
        .map(|d| {
            let j = linked.stems.iter().position(|l| l.name == d.name && l.horizontal == d.horizontal);
            j.and_then(|j| src.stem_values.get(j).copied()).unwrap_or(0.0)
        })
        .collect();
    master.number_values = font
        .numbers
        .iter()
        .map(|d| {
            let j = linked.numbers.iter().position(|l| l.name == d.name);
            j.and_then(|j| src.number_values.get(j).copied()).unwrap_or(0.0)
        })
        .collect();
    mark(&mut master.user_data, source);
    Arc::make_mut(&mut font.masters).push(master);

    for i in 0..font.glyphs.len() {
        let Some(name) = font.glyphs.as_arcs().get(i).map(|g| g.name.clone()) else { continue };
        let Some(from) = linked.glyphs.get(&name) else { continue };
        let layers: Vec<Layer> = from.layers.iter().filter(|l| l.master == src.id).map(|l| moved(l, &src.id, &id)).collect();
        if layers.is_empty() {
            continue;
        }
        let Some(mut g) = font.glyphs.get_mut_at(i) else { continue };
        for l in layers {
            if !g.layers.iter().any(|x| x.id == l.id) {
                g.layers.push(l);
            }
        }
    }

    let k = Arc::make_mut(&mut font.kerning);
    for (to, from) in [(&mut k.ltr, &linked.kerning.ltr), (&mut k.rtl, &linked.kerning.rtl), (&mut k.vertical, &linked.kerning.vertical)] {
        if let Some(e) = from.0.iter().find(|e| e.master == src.id) {
            let mut e = e.clone();
            e.master = id.clone();
            to.0.push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_model::Param;

    fn font_with(names: &[&str]) -> Font {
        let mut f = Font::new("Host");
        f.glyphs.retain(|g| names.contains(&g.name.as_str()));
        f
    }

    #[test]
    fn linked_glyphs_and_masters_are_added_locked_and_left_out_when_saving() {
        let mut host = font_with(&["A"]);
        let other = {
            let mut f = font_with(&["A", "B"]);
            let m = Arc::make_mut(&mut f.masters);
            if let Some(first) = m.first_mut() {
                first.name = "Bold".into();
            }
            f
        };
        let before = host.clone();
        let mut warnings = Vec::new();
        link_glyphs(&mut host, &other, "Other.glyphs", &mut warnings);
        link_master(&mut host, &other, Some("Bold"), "Other.glyphs", &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(host.glyphs.len(), 2, "only B is added: A is the host's own");
        let b = host.glyphs.get("B").unwrap();
        assert!(b.info.locked && is_linked(&b.info.user_data));
        assert_eq!(host.masters.len(), 2);
        let added = host.masters.get(1).unwrap();
        assert_ne!(added.id, host.masters.first().unwrap().id, "the linked master gets an id of its own");
        assert_eq!(host.glyphs.get("A").unwrap().layers.len(), 2, "A gets the linked master's layer");
        host.check().unwrap();
        let saved = unlinked(&host);
        assert_eq!(*saved, before, "saving writes the host as it was");
    }

    #[test]
    fn a_file_with_other_units_per_em_is_used_unscaled_with_a_warning() {
        let host = font_with(&["A"]);
        let mut other = font_with(&["A", "B"]);
        let mut warnings = Vec::new();
        check_upm(&host, &other, IMPORT_FONT, "Other.glyphs", &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        other.units_per_em = 2048;
        check_upm(&host, &other, IMPORT_FONT, "Other.glyphs", &mut warnings);
        assert!(warnings.iter().any(|w| w.contains("2048") && w.contains("unscaled")), "{warnings:?}");
    }

    #[test]
    fn values_are_read_in_both_forms() {
        let mut d = fontcraft_model::plist::Dict::new();
        d.insert("Path", Value::String(Str::new("files/Linked.glyphs")));
        assert_eq!(font_path(&Value::Dict(d.clone())), Some("files/Linked.glyphs"));
        assert_eq!(font_path(&Value::Dict(fontcraft_model::plist::Dict::new())), None, "an empty dictionary names no file");
        assert_eq!(master_ref(&Value::String(Str::new("B.glyphs;Bold"))), Some(("B.glyphs", Some("Bold"))));
        d.insert("Master", Value::String(Str::new("m02")));
        assert_eq!(master_ref(&Value::Dict(d)), Some(("files/Linked.glyphs", Some("m02"))));
    }

    #[test]
    fn a_font_without_links_is_left_alone() {
        let f = font_with(&["A"]);
        let mut w = Vec::new();
        assert!(resolve(&f, None, &mut w).unwrap().is_none());
        assert!(matches!(unlinked(&f), Cow::Borrowed(_)));
        let mut g = f.clone();
        Arc::make_mut(&mut g.info).params.0.push(Param::new(IMPORT_FONT, Value::String(Str::new("Missing.glyphs"))));
        let out = resolve(&g, None, &mut w).unwrap().unwrap();
        assert_eq!(out.glyphs.len(), 1);
        assert!(w.iter().any(|w| w.contains("Missing.glyphs")), "{w:?}");
    }
}

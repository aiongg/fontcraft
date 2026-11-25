//! Warnings for documents whose source is a UFO, a UFOZ or a designspace
//! (`source.ufoUnsupportedFeatureWarning`). A command that creates data the file cannot hold
//! still runs; [`attach`] adds `warnings` to its reply, naming
//! - what the next save would leave out that it did not before: the entries the save loss
//!   report ([`crate::io::save_losses`]) gained, and
//! - the editor features a UFO keeps only in glyph lib keys that other UFO tools ignore (corner
//!   components, caps, brushes, segments, smart components), when the command added some.
//!
//! An edit that changes only glyphs is compared on those glyphs and the glyphs tied to them by
//! components ([`scoped`]), so a drag on a large font costs what the dragged glyph costs. Only
//! when that comparison finds a difference are the whole fonts' loss reports built, for the
//! exact wording.
//!
//! No commands live here: [`crate::Session`] calls [`attach`] after every editing command.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_model::attach::{BRUSH_HINT, CAP_HINT, SEGMENT_HINT};
use fontcraft_model::corner::CORNER_HINT;
use fontcraft_model::{Font, Glyph, GlyphTable, MAX_COMPONENT_DEPTH, Shape, SourceFormat};
use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
use super::loss_report::Fresh;
use crate::DocState;

/// Most warnings one command's reply gains.
const MAX_WARNINGS: usize = 8;

/// Most glyphs an edit is compared on glyph by glyph; past this the whole fonts are compared.
const MAX_SCOPED: usize = 256;

/// Most new items one warning names.
#[cfg(not(target_arch = "wasm32"))]
const MAX_ITEMS: usize = 3;

/// Whether documents of this source format get the warnings.
fn is_ufo(format: SourceFormat) -> bool {
    matches!(format, SourceFormat::Ufo | SourceFormat::Ufoz | SourceFormat::Designspace)
}

/// `value` with the warnings for what command `id` created in the UFO documents of `docs` that
/// `before` (the documents as they were) did not have. Undo and redo put back what was there,
/// and the saves report their own `losses`, so they gain none; nor does a document the command
/// opened, or one it moved to or from a UFO.
///
/// The warnings extend the reply's `warnings` list, or become it; a reply whose `warnings` is
/// not a list gets them as `formatWarnings`.
pub(crate) fn attach(docs: &[DocState], before: &[DocState], id: &str, value: Value) -> Value {
    if matches!(id, "edit.undo" | "edit.redo" | "file.save" | "file.saveAs") {
        return value;
    }
    let mut warnings = Vec::new();
    for doc in docs {
        let Some(old) = before.iter().find(|d| d.id == doc.id) else { continue };
        // A native file keeps everything, whatever format its font was read from.
        let same_format = old.file_format() == doc.file_format() && old.font.source.format == doc.font.source.format;
        if std::sync::Arc::ptr_eq(&old.font, &doc.font) || !is_ufo(old.file_format()) || !is_ufo(old.font.source.format) || !same_format {
            continue;
        }
        warnings.extend(edit_warnings(&old.font, &doc.font));
    }
    if warnings.is_empty() {
        return value;
    }
    warnings.truncate(MAX_WARNINGS);
    match value {
        Value::Object(mut map) => {
            let new: Vec<Value> = warnings.into_iter().map(Value::String).collect();
            match map.get_mut("warnings") {
                Some(Value::Array(list)) => list.extend(new),
                None => {
                    map.insert("warnings".into(), Value::Array(new));
                }
                Some(_) => {
                    map.insert("formatWarnings".into(), Value::Array(new));
                }
            }
            Value::Object(map)
        }
        other => {
            log::warn!("`{id}`: {}", warnings.join("; "));
            other
        }
    }
}

/// The warnings for the edit from `before` to `after`.
fn edit_warnings(before: &Font, after: &Font) -> Vec<String> {
    let Some((old, new)) = scoped(before, after) else {
        let mut out = new_features(before, after);
        out.extend(new_losses(before, after));
        return out;
    };
    if new.glyphs.is_empty() && old.glyphs.is_empty() {
        // Nothing the reports read differs.
        return Vec::new();
    }
    // The feature counts add up glyph by glyph, so the scoped difference is the whole one.
    let mut out = new_features(&old, &new);
    // A loss line can carry a count over the whole font, so the wording comes from the whole
    // fonts, once the scoped reports show there is a difference to word.
    if loss_reports_differ(&old, &new) {
        out.extend(new_losses(before, after));
    }
    out
}

/// `before` and `after` cut down to the glyphs an edit that changed only glyphs is compared on:
/// the glyphs it changed, added or removed, with every glyph tied to those by components (used
/// or using, in either font) and the colour layer glyphs named after them, which the save names
/// from the same base. Every count the loss report and the feature count make adds up glyph by
/// glyph, given the font-level data and the glyphs a glyph's components use, so the cut-down
/// fonts differ wherever the whole ones do. `None`: compare the whole fonts, because font-level
/// data changed (masters decide which layers are lost, kerning and colour data reach into
/// glyphs) or more than [`MAX_SCOPED`] glyphs are tied to the edit.
fn scoped(before: &Font, after: &Font) -> Option<(Font, Font)> {
    let font_level_same = Font { glyphs: before.glyphs.clone(), ..after.clone() }.same_as(before);
    if !font_level_same {
        return None;
    }
    let names = scope(before, after)?;
    let only = |font: &Font| {
        let glyphs: Vec<Arc<Glyph>> = font.glyphs.iter().filter(|g| names.contains(&g.name)).cloned().collect();
        Font { glyphs: GlyphTable::from_glyphs(glyphs), ..font.clone() }
    };
    Some((only(before), only(after)))
}

/// The names of the glyphs that differ between `before` and `after`: by position when both have
/// as many glyphs, else by identity. `None` past [`MAX_SCOPED`].
fn changed_names(before: &Font, after: &Font) -> Option<HashSet<String>> {
    let (old, new) = (before.glyphs.as_arcs(), after.glyphs.as_arcs());
    let mut out = HashSet::new();
    if old.len() == new.len() {
        for (a, b) in old.iter().zip(new) {
            if !Arc::ptr_eq(a, b) {
                out.insert(a.name.clone());
                out.insert(b.name.clone());
                if out.len() > MAX_SCOPED {
                    return None;
                }
            }
        }
        return Some(out);
    }
    let ids = |glyphs: &[Arc<Glyph>]| -> HashSet<*const Glyph> { glyphs.iter().map(Arc::as_ptr).collect() };
    let (had, has) = (ids(old), ids(new));
    out.extend(new.iter().filter(|g| !had.contains(&Arc::as_ptr(*g))).map(|g| g.name.clone()));
    out.extend(old.iter().filter(|g| !has.contains(&Arc::as_ptr(*g))).map(|g| g.name.clone()));
    (out.len() <= MAX_SCOPED).then_some(out)
}

/// The glyphs glyph `g` uses as components, in its layers and their backgrounds.
fn components(g: &Glyph) -> impl Iterator<Item = &str> {
    g.layers.iter().flat_map(|l| l.shapes.iter().chain(l.info.background.iter().flat_map(|b| b.shapes.iter()))).filter_map(|s| match s {
        Shape::Component(c) => Some(c.glyph.as_str()),
        _ => None,
    })
}

/// The glyph names `name` would be a colour layer glyph of (`B` for `B.color0`).
fn colour_bases(name: &str) -> impl Iterator<Item = &str> {
    name.match_indices(".color").filter_map(move |(i, _)| name.get(..i))
}

/// The changed glyphs ([`changed_names`]) and the glyphs tied to them, as [`scoped`] describes;
/// `None` past [`MAX_SCOPED`] glyphs or a tie deeper than the model allows components to nest.
fn scope(before: &Font, after: &Font) -> Option<HashSet<String>> {
    let mut scope = changed_names(before, after)?;
    if scope.is_empty() {
        return Some(scope);
    }
    let fonts = [before, after];
    for _ in 0..=MAX_COMPONENT_DEPTH {
        let mut tied: Vec<String> = Vec::new();
        for name in &scope {
            for font in fonts {
                if let Some(g) = font.glyphs.get(name) {
                    tied.extend(components(g).filter(|c| !scope.contains(*c)).map(str::to_string));
                }
                tied.extend(colour_bases(name).filter(|b| !scope.contains(*b) && font.glyphs.get(b).is_some()).map(str::to_string));
            }
        }
        for font in fonts {
            for g in font.glyphs.iter().filter(|g| !scope.contains(&g.name)) {
                if components(g).any(|c| scope.contains(c)) || colour_bases(&g.name).any(|b| scope.contains(b)) {
                    tied.push(g.name.clone());
                }
            }
        }
        let mut grew = false;
        for name in tied {
            grew |= scope.insert(name);
        }
        if scope.len() > MAX_SCOPED {
            return None;
        }
        if !grew {
            return Some(scope);
        }
    }
    None
}

/// The editor features a UFO keeps only in glyph lib keys, counted on the foreground of every
/// layer (a background's hints are lost outright, which the loss report names).
#[derive(Default)]
struct Features {
    corners: usize,
    caps: usize,
    brushes: usize,
    segments: usize,
    smart: usize,
}

impl Features {
    fn of(font: &Font) -> Self {
        let mut f = Features::default();
        font.glyphs.iter().for_each(|g| f.add(font, g));
        f
    }

    /// Counts the glyph `g` of `font`.
    fn add(&mut self, font: &Font, g: &Glyph) {
        for l in &g.layers {
            for h in &l.info.hints {
                match h.kind() {
                    Some(CORNER_HINT) => self.corners += 1,
                    Some(CAP_HINT) => self.caps += 1,
                    Some(BRUSH_HINT) => self.brushes += 1,
                    Some(SEGMENT_HINT) => self.segments += 1,
                    _ => {}
                }
            }
            for s in &l.shapes {
                if let Shape::Component(c) = s
                    && font.glyphs.get(&c.glyph).is_some_and(|g| !g.info.parts.is_empty())
                {
                    self.smart += 1;
                }
            }
        }
    }

    fn named(&self) -> [(usize, &'static str); 5] {
        [
            (self.corners, "corner components"),
            (self.caps, "caps"),
            (self.brushes, "brushes"),
            (self.segments, "segments"),
            (self.smart, "smart components"),
        ]
    }
}

/// The editor features of `font` a UFO keeps only in glyph lib keys, as entries of a loss report:
/// each kind that is there, how often and in which glyphs.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn private_features(font: &Font) -> Vec<Value> {
    let mut kinds: Vec<(&'static str, crate::io::Lost)> = Features::default().named().iter().map(|(_, what)| (*what, Default::default())).collect();
    for g in font.glyphs.iter() {
        let mut f = Features::default();
        f.add(font, g);
        for ((_, lost), (n, _)) in kinds.iter_mut().zip(f.named()) {
            if n > 0 {
                lost.add(n, Some(&g.name));
            }
        }
    }
    let entry =
        |(what, lost): (&'static str, crate::io::Lost)| lost.entry("glyphs", &format!("{what}, kept in the glyph lib, which other UFO tools ignore"));
    kinds.into_iter().filter(|(_, lost)| !lost.is_empty()).map(entry).collect()
}

/// How many smart components `font` places, on the foreground of every layer.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn smart_components(font: &Font) -> usize {
    Features::of(font).smart
}

fn new_features(before: &Font, after: &Font) -> Vec<String> {
    let (old, new) = (Features::of(before), Features::of(after));
    old.named()
        .iter()
        .zip(new.named())
        .filter(|((was, _), (now, _))| now > was)
        .map(|((was, _), (now, what))| {
            format!("a UFO has no standard place for {what} ({} added): they are kept in the glyph lib, which other UFO tools ignore", now - was)
        })
        .collect()
}

/// What the save loss report of `after` names that the one of `before` did not: an entry with
/// more `entries`, or items it did not list.
#[cfg(not(target_arch = "wasm32"))]
fn new_losses(before: &Font, after: &Font) -> Vec<String> {
    let Some(format) = crate::io::SaveFormat::of_source(after.source.format) else { return Vec::new() };
    let (old, new) = (crate::io::save_losses(before, format), crate::io::save_losses(after, format));
    let name = format.name();
    super::loss_report::fresh(&old, &new)
        .into_iter()
        .map(|Fresh { entry, added, grew }| {
            let what = entry.get("what").and_then(Value::as_str).unwrap_or_default();
            if added.is_empty() {
                return format!("the next save as {name} leaves out {what} ({grew} more)");
            }
            let shown: Vec<&str> = added.iter().take(MAX_ITEMS).map(String::as_str).collect();
            let more = added.len().saturating_sub(shown.len());
            let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
            format!("the next save as {name} leaves out {what}: {}{tail}", shown.join("; "))
        })
        .collect()
}

/// Whether the save loss reports of `before` and `after` differ at all.
#[cfg(not(target_arch = "wasm32"))]
fn loss_reports_differ(before: &Font, after: &Font) -> bool {
    let Some(format) = crate::io::SaveFormat::of_source(after.source.format) else { return false };
    crate::io::save_losses(before, format) != crate::io::save_losses(after, format)
}

/// The browser has no loss report (it writes no files by path); the features are still named.
#[cfg(target_arch = "wasm32")]
fn new_losses(_: &Font, _: &Font) -> Vec<String> {
    Vec::new()
}

#[cfg(target_arch = "wasm32")]
fn loss_reports_differ(_: &Font, _: &Font) -> bool {
    false
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use fontcraft_model::Family;
    use fontcraft_model::plist::{Str, Value as Plist};

    /// A key read from another format that a UFO has no place for is named once: when the edit
    /// adds it, not again after.
    #[test]
    fn a_new_loss_is_named_once() {
        let mut before = Font::new("Losses");
        before.source.format = SourceFormat::Ufo;
        let mut after = before.clone();
        after.glyph_mut("A").unwrap().extra.push(Family::Glyphs, "widthMetricsKey", Plist::String(Str::new("=H")));
        let w = new_losses(&before, &after);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].starts_with("the next save as ufo leaves out keys read from that format"), "{w:?}");
        assert!(new_losses(&after, &after).is_empty());
    }

    /// A UFO font with 2,000 glyphs more than the default ones, each with a key a UFO has no
    /// place for; `I` uses `A` as a component.
    fn large() -> Font {
        let mut font = Font::new("Large");
        font.source.format = SourceFormat::Ufo;
        for i in 0..2000 {
            let name = format!("g{i:04}");
            font.add_glyph(&name, Vec::new(), 500.0).unwrap();
            font.glyph_mut(&name).unwrap().extra.push(Family::Glyphs, "widthMetricsKey", Plist::String(Str::new("=H")));
        }
        font.glyph_mut("I").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(fontcraft_model::Component::new("A", 0.0, 0.0)));
        font
    }

    fn names(font: &Font) -> Vec<String> {
        font.glyphs.iter().map(|g| g.name.clone()).collect()
    }

    /// In a 2,000-glyph UFO, an edit of one glyph is compared on that glyph alone, plus the
    /// glyphs a component ties to it; a new loss is worded as the whole-font comparison words it.
    #[test]
    fn an_edit_in_a_large_ufo_compares_only_the_glyphs_it_touches() {
        let before = large();
        let mut moved = before.clone();
        moved.glyph_mut("g0500").unwrap().layer_mut("m01").unwrap().width = 520.0;
        let (old, new) = scoped(&before, &moved).unwrap();
        assert_eq!((names(&old), names(&new)), (vec!["g0500".to_string()], vec!["g0500".to_string()]));
        assert!(edit_warnings(&before, &moved).is_empty());

        let mut base = before.clone();
        base.glyph_mut("A").unwrap().layer_mut("m01").unwrap().width = 620.0;
        let (_, new) = scoped(&before, &base).unwrap();
        assert_eq!(names(&new), ["A", "I"], "the glyph using the edited one is compared too");

        let mut lossy = before.clone();
        lossy.glyph_mut("g0500").unwrap().extra.push(Family::Glyphs, "leftMetricsKey", Plist::String(Str::new("=H")));
        let w = edit_warnings(&before, &lossy);
        let mut whole = new_features(&before, &lossy);
        whole.extend(new_losses(&before, &lossy));
        assert_eq!(w, whole);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].ends_with("(1 more)"), "{w:?}");
    }

    /// An edit of font-level data is compared on the whole fonts.
    #[test]
    fn a_font_level_edit_compares_the_whole_fonts() {
        let before = large();
        let mut after = before.clone();
        Arc::make_mut(&mut after.masters).push(fontcraft_model::Master { id: "m02".into(), name: "Bold".into(), ..Default::default() });
        assert!(scoped(&before, &after).is_none());
    }
}

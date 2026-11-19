//! Warnings for documents whose source is a UFO, a UFOZ or a designspace
//! (`source.ufoUnsupportedFeatureWarning`). A command that creates data the file cannot hold
//! still runs; [`attach`] adds `warnings` to its reply, naming
//! - what the next save would leave out that it did not before: the entries the save loss
//!   report ([`crate::io::save_losses`]) gained, and
//! - the editor features a UFO keeps only in glyph lib keys that other UFO tools ignore (corner
//!   components, caps, brushes, segments, smart components), when the command added some.
//!
//! No commands live here: [`crate::Session`] calls [`attach`] after every editing command.

use fontcraft_model::attach::{BRUSH_HINT, CAP_HINT, SEGMENT_HINT};
use fontcraft_model::corner::CORNER_HINT;
use fontcraft_model::{Font, Shape, SourceFormat};
use serde_json::Value;

use crate::DocState;

/// Most warnings one command's reply gains.
const MAX_WARNINGS: usize = 8;

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
        if std::sync::Arc::ptr_eq(&old.font, &doc.font) || !is_ufo(old.font.source.format) || old.font.source.format != doc.font.source.format {
            continue;
        }
        warnings.extend(new_features(&old.font, &doc.font));
        warnings.extend(new_losses(&old.font, &doc.font));
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
        for l in font.glyphs.iter().flat_map(|g| g.layers.iter()) {
            for h in &l.info.hints {
                match h.kind() {
                    Some(CORNER_HINT) => f.corners += 1,
                    Some(CAP_HINT) => f.caps += 1,
                    Some(BRUSH_HINT) => f.brushes += 1,
                    Some(SEGMENT_HINT) => f.segments += 1,
                    _ => {}
                }
            }
            for s in &l.shapes {
                if let Shape::Component(c) = s
                    && font.glyphs.get(&c.glyph).is_some_and(|g| !g.info.parts.is_empty())
                {
                    f.smart += 1;
                }
            }
        }
        f
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
    use std::collections::HashSet;
    let Some(format) = crate::io::SaveFormat::of_source(after.source.format) else { return Vec::new() };
    let (old, new) = (crate::io::save_losses(before, format), crate::io::save_losses(after, format));
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let key = |v: &Value| (text(v, "family"), text(v, "what"));
    let entries = |v: &Value| v.get("entries").and_then(Value::as_u64).unwrap_or(0);
    let items = |v: &Value| -> Vec<String> {
        let list = v.get("items").or_else(|| v.get("files")).and_then(Value::as_array);
        list.map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    let mut out = Vec::new();
    for entry in &new {
        let was = old.iter().find(|b| key(*b) == key(entry));
        let had: HashSet<String> = was.map(items).unwrap_or_default().into_iter().collect();
        let added: Vec<String> = items(entry).into_iter().filter(|i| !had.contains(i)).collect();
        let grew = entries(entry).saturating_sub(was.map(entries).unwrap_or(0));
        if added.is_empty() && grew == 0 {
            continue;
        }
        let what = text(entry, "what");
        let name = format.name();
        out.push(if added.is_empty() {
            format!("the next save as {name} leaves out {what} ({grew} more)")
        } else {
            let shown: Vec<&str> = added.iter().take(MAX_ITEMS).map(String::as_str).collect();
            let more = added.len().saturating_sub(shown.len());
            let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
            format!("the next save as {name} leaves out {what}: {}{tail}", shown.join("; "))
        });
    }
    out
}

/// The browser has no loss report (it writes no files by path); the features are still named.
#[cfg(target_arch = "wasm32")]
fn new_losses(_: &Font, _: &Font) -> Vec<String> {
    Vec::new()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
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
}

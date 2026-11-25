//! Status notices taken from a command's reply, after it ran.
//!
//! | Reply | Notice |
//! |---|---|
//! | `glyph.matchMasters`, `glyph.relocateStartPoints`, the two hinting Autohints, the edits that take `matchLayers`: `skipped` | the label, then each skipped glyph with its reason |
//! | `font.info.pasteRows`: `skipped` | the label, then each skipped row with its reason |
//! | `color.generateDarkPalette`: `palette`, `name` | the new palette's number and name |
//! | `font.features.compile`: `errors`, `warnings`, `targetErrors`, `targetWarnings` | how many errors and warnings the code has |
//! | `font.features.update`: `added`, `updated`, `removed` | how many entries it added, changed and removed |
//! | any command: `formatWarnings` (what a UFO, UFOZ or designspace save would lose), else the command's own `warnings` | the first warning, and how many follow |
//!
//! The notices of the Font View commands (`font_view::incompatible_notice`) come first.

use craft::ui::runtime::env::WordSet as _;
use serde_json::Value;

use super::font_view::MAX_NOTICE_NAMES;

craft::ui::words! {
    /// The status notices the app layer writes from a command's reply.
    pub struct ReplyNoticeWords {
        /// Before the glyphs Match Masters, Relocate Start Points or Autohint left alone.
        skipped: "fontcraftNotice.skipped" = "skipped";
        /// Before the new palette's number.
        dark_palette: "fontcraftNotice.darkPalette" = "Added dark palette";
        error: "fontcraftNotice.error" = "error";
        errors: "fontcraftNotice.errors" = "errors";
        warning: "fontcraftNotice.warning" = "warning";
        warnings: "fontcraftNotice.warnings" = "warnings";
        no_errors: "fontcraftNotice.noErrors" = "no errors";
        /// After the number of feature entries Update added, changed or removed.
        added: "fontcraftNotice.entriesAdded" = "added";
        changed: "fontcraftNotice.entriesChanged" = "changed";
        removed: "fontcraftNotice.entriesRemoved" = "removed";
        nothing_to_update: "fontcraftNotice.nothingToUpdate" = "every automatic entry is up to date";
        /// After the first warning, before how many follow.
        more_warnings: "fontcraftNotice.moreWarnings" = "more warnings";
    }
}

/// The commands whose reply lists, under `skipped`, the glyphs or layers they left alone.
const REPORTS_SKIPPED: [&str; 4] = ["glyph.matchMasters", "glyph.relocateStartPoints", "hint.tt.autohintGlyph", "hint.ps.autohintGlyph"];

/// The notice for `reply` of command `id`, or `None` when it has nothing to say.
pub fn notice(id: &str, reply: &Value) -> Option<String> {
    if REPORTS_SKIPPED.contains(&id) || crate::shell::compatibility::MATCHED.contains(&id) {
        return skipped(id, reply).or_else(|| format_warning(reply));
    }
    if id == "font.features.compile" {
        return Some(compiled(reply));
    }
    if id == "font.features.update" {
        return Some(updated(reply));
    }
    if id == "font.info.pasteRows" {
        return skipped_rows(id, reply);
    }
    if id == "color.generateDarkPalette" {
        return dark_palette(reply);
    }
    format_warning(reply)
}

fn label(id: &str) -> &str {
    fontcraft_engine::find_command(id).map_or(id, |spec| spec.label)
}

/// `Paste Rows: skipped Filter (cannot be set on the font), …`; each key once, with its first reason.
fn skipped_rows(id: &str, reply: &Value) -> Option<String> {
    let entries = reply.get("skipped")?.as_array()?;
    let mut named: Vec<(&str, &str)> = Vec::new();
    let mut more = false;
    for entry in entries {
        let (Some(key), Some(reason)) = (entry.get("key").and_then(Value::as_str), entry.get("reason").and_then(Value::as_str)) else { continue };
        if named.iter().any(|(k, _)| *k == key) {
            continue;
        }
        if named.len() < MAX_NOTICE_NAMES {
            named.push((key, reason));
        } else {
            more = true;
        }
    }
    if named.is_empty() {
        return None;
    }
    let mut list: Vec<String> = named.iter().map(|(key, reason)| format!("{key} ({reason})")).collect();
    if more {
        list.push("…".into());
    }
    let words = ReplyNoticeWords::english();
    Some(format!("{}: {} {}", label(id), words.skipped.as_str(), list.join(", ")))
}

/// `Match Masters: skipped B (reason), C …`.
fn skipped(id: &str, reply: &Value) -> Option<String> {
    let entries = reply.get("skipped")?.as_array()?;
    if entries.is_empty() {
        return None;
    }
    let mut named: Vec<String> = entries
        .iter()
        .take(MAX_NOTICE_NAMES)
        .filter_map(|e| {
            let glyph = e.get("glyph")?.as_str()?;
            Some(match e.get("reason").and_then(Value::as_str) {
                Some(reason) => format!("{glyph} ({reason})"),
                None => glyph.to_string(),
            })
        })
        .collect();
    if entries.len() > MAX_NOTICE_NAMES {
        named.push("…".into());
    }
    Some(format!("{}: {} {}", label(id), ReplyNoticeWords::english().skipped.as_str(), named.join(", ")))
}

/// `Compile: 2 errors, 1 warning`, over the compiled master and the other targets.
fn compiled(reply: &Value) -> String {
    let words = ReplyNoticeWords::english();
    let sum = |a: &str, b: &str| [a, b].iter().filter_map(|key| reply.get(key)?.as_u64()).fold(0u64, u64::saturating_add);
    let (errors, warnings) = (sum("errors", "targetErrors"), sum("warnings", "targetWarnings"));
    let count = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(count(errors, words.error.as_str(), words.errors.as_str()));
    }
    if warnings > 0 {
        parts.push(count(warnings, words.warning.as_str(), words.warnings.as_str()));
    }
    if parts.is_empty() {
        parts.push(words.no_errors.as_str().to_string());
    }
    format!("{}: {}", label("font.features.compile"), parts.join(", "))
}

/// `Update: 2 entries added, 3 changed, 1 removed`.
fn updated(reply: &Value) -> String {
    let words = ReplyNoticeWords::english();
    let count = |key: &str| reply.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    let parts: Vec<String> = [("added", &words.added), ("updated", &words.changed), ("removed", &words.removed)]
        .into_iter()
        .map(|(key, word)| (count(key), word))
        .filter(|(n, _)| *n > 0)
        .map(|(n, word)| format!("{n} {}", word.as_str()))
        .collect();
    let text = if parts.is_empty() { words.nothing_to_update.as_str().to_string() } else { parts.join(", ") };
    format!("{}: {text}", label("font.features.update"))
}

/// `Added dark palette 2: Dark`.
fn dark_palette(reply: &Value) -> Option<String> {
    let index = reply.get("palette")?.as_u64()?;
    let words = ReplyNoticeWords::english();
    Some(match reply.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
        Some(name) => format!("{} {index}: {name}", words.dark_palette.as_str()),
        None => format!("{} {index}", words.dark_palette.as_str()),
    })
}

/// The first warning of a reply: from `formatWarnings` when the reply has it, else from the command's own
/// `warnings`.
fn format_warning(reply: &Value) -> Option<String> {
    let list = reply.get("formatWarnings").or_else(|| reply.get("warnings"))?.as_array()?;
    let texts: Vec<&str> = list.iter().filter_map(Value::as_str).collect();
    let first = texts.first()?;
    Some(match texts.len() {
        1 => first.to_string(),
        n => format!("{first} (+{} {})", n - 1, ReplyNoticeWords::english().more_warnings.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::notice;
    use crate::app::{APP, FontCraftApp, Msg};

    /// Match Masters names the glyphs it skipped, a dark palette its number and name, and an edit its first format
    /// warning; a reply with none of them says nothing.
    #[test]
    fn replies_become_status_notices() {
        let reply = json!({"glyphs": ["A"], "skipped": [{"glyph": "B", "layer": "m2", "reason": "paths differ"}]});
        assert_eq!(notice("glyph.matchMasters", &reply).as_deref(), Some("Match Masters: skipped B (paths differ)"));
        assert_eq!(notice("glyph.relocateStartPoints", &json!({"glyphs": ["A"], "skipped": []})), None);
        assert_eq!(notice("color.generateDarkPalette", &json!({"palette": 1, "name": "Dark"})).as_deref(), Some("Added dark palette 1: Dark"));
        assert_eq!(
            notice("glyph.newGlyph", &json!({"name": "a", "warnings": ["UFO drops x", "UFO drops y"]})).as_deref(),
            Some("UFO drops x (+1 more warnings)")
        );
        assert_eq!(notice("glyph.newGlyph", &json!({"name": "a"})), None);
        let reply = json!({"hinted": [], "skipped": [{"glyph": "l", "reason": "no stems"}], "warnings": []});
        assert_eq!(notice("hint.tt.autohintGlyph", &reply).as_deref(), Some("Autohint: skipped l (no stems)"));
    }

    /// Generate Dark Palette from its Font menu row adds a palette and says so in the status line.
    #[test]
    fn generate_dark_palette_reports_the_new_palette() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.session.execute("colorPalette.addColor", &json!({"color": "#FF0000"})).unwrap();
        app.sent.push(Msg::run("color.generateDarkPalette", json!({})));
        app.drain();
        assert!(app.status.starts_with("Added dark palette"), "{}", app.status);
    }
}

//! Status notices taken from a command's reply, after it ran.
//!
//! | Reply | Notice |
//! |---|---|
//! | `glyph.matchMasters`, `glyph.relocateStartPoints`: `skipped` | the label, then each skipped glyph with its reason |
//! | `color.generateDarkPalette`: `palette`, `name` | the new palette's number and name |
//! | any command: `formatWarnings` (what a UFO, UFOZ or designspace save would lose), else the command's own `warnings` | the first warning, and how many follow |
//!
//! The notices of the Font View commands (`font_view::incompatible_notice`) come first.

use craft::ui::runtime::env::WordSet as _;
use serde_json::Value;

use super::font_view::MAX_NOTICE_NAMES;

craft::ui::words! {
    /// The status notices the app layer writes from a command's reply.
    pub struct ReplyNoticeWords {
        /// Before the glyphs Match Masters or Relocate Start Points left alone.
        skipped: "fontcraftNotice.skipped" = "skipped";
        /// Before the new palette's number.
        dark_palette: "fontcraftNotice.darkPalette" = "Added dark palette";
        /// After the first warning, before how many follow.
        more_warnings: "fontcraftNotice.moreWarnings" = "more warnings";
    }
}

/// The commands whose reply lists, under `skipped`, the glyphs or layers they left alone.
const REPORTS_SKIPPED: [&str; 2] = ["glyph.matchMasters", "glyph.relocateStartPoints"];

/// The notice for `reply` of command `id`, or `None` when it has nothing to say.
pub fn notice(id: &str, reply: &Value) -> Option<String> {
    if REPORTS_SKIPPED.contains(&id) {
        return skipped(id, reply).or_else(|| format_warning(reply));
    }
    if id == "color.generateDarkPalette" {
        return dark_palette(reply);
    }
    format_warning(reply)
}

fn label(id: &str) -> &str {
    fontcraft_engine::find_command(id).map_or(id, |spec| spec.label)
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

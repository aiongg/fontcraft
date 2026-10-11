//! Text in Edit View tabs: the shell commands over a tab's text, cursor, writing direction and
//! kerning mode, and the document's display strings (the tabs' texts as the file stores them).
//!
//! A tab's text is view state, so these are shell commands (`docs/plans/M5.md` § 3). Each one
//! takes the tab by `tab` id (default: the showing tab) and answers with `editView.getText`'s
//! reply. Changing a text is no undo step and leaves the document unmodified.

use std::ops::Range;

use fontcraft_model::{Font, param_flag};
use fontcraft_textlayout::{Charmap, Item, KerningMode, MAX_ITEMS, MAX_PREVIEW_ITEMS, Text, TextError, parse, parse_with};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::state::{MAX_TAB_GLYPHS, Tab, TabText, WritingDirection};

pub const COMMANDS: &[UiCommand] = &[
    (
        "editView.toggleTextMode",
        "Toggle Text Mode",
        &[],
        "Escape",
        "{tab?: id, mode?: \"text\" | \"edit\"} switch the tab between typing text and editing the current glyph (omitted: the other one); entering text mode puts the cursor before the edited glyph",
    ),
    (
        "editView.insertText",
        "Insert Text",
        &[],
        "",
        "{tab?: id, text?: string, chars?: string, glyphs?: [names], at?: index, replace?: [start, end]} insert into the tab's text: `text` reads `/name` as the glyph name and `/Placeholder` as a placeholder, `chars` takes every character as typed, `glyphs` names glyphs; they go in place of the selection, at `at`, or in place of the items from start to end, and the cursor ends after them",
    ),
    (
        "editView.setText",
        "Set Tab Text",
        &[],
        "",
        "{tab?: id, text?: string, chars?: string, glyphs?: [names]} replace the whole text of the tab (`text`, `chars` and `glyphs` as Insert Text reads them); the cursor ends after it",
    ),
    (
        "editView.getText",
        "Get Tab Text",
        &[],
        "",
        "{tab?: id} the tab's text → {tab, items: [glyph name | {placeholder} | {lineBreak} | {missing: char}], text (with /name escapes), cursor, selection: [start, end], current (the edited glyph), textMode, direction, kerning, master}",
    ),
    (
        "editView.setCursor",
        "Set Text Cursor",
        &[],
        "",
        "{tab?: id, index: integer, selectionLength?: integer} put the text cursor before item `index` and select the `selectionLength` items after it; the glyph after the cursor becomes the current glyph",
    ),
    (
        "edit.addPlaceholder",
        "Add Placeholder",
        &["Edit"],
        "Cmd+Alt+Shift+P",
        "{tab?: id, at?: index} insert a placeholder, which shows the current glyph, in place of the selection or at `at`",
    ),
    (
        "editView.previousGlyph",
        "Previous Glyph",
        &[],
        "Home",
        "{tab?: id} replace the current glyph in the text with the glyph before it in the font's glyph order (at the first: nothing changes)",
    ),
    (
        "editView.nextGlyph",
        "Next Glyph",
        &[],
        "End",
        "{tab?: id} replace the current glyph in the text with the glyph after it in the font's glyph order (at the last: nothing changes)",
    ),
    (
        "editView.previousVisibleGlyph",
        "Previous Glyph in Font View",
        &[],
        "Shift+Home",
        "{tab?: id} replace the current glyph with the glyph before it as Font View lists the glyphs now (a glyph Font View does not list: the last one listed)",
    ),
    (
        "editView.nextVisibleGlyph",
        "Next Glyph in Font View",
        &[],
        "Shift+End",
        "{tab?: id} replace the current glyph with the glyph after it as Font View lists the glyphs now (a glyph Font View does not list: the first one listed)",
    ),
    (
        "editView.setWritingDirection",
        "Writing Direction",
        &[],
        "",
        "{tab?: id, direction: \"ltr\" | \"rtl\" | \"ttb\"} lay the tab's text out left to right, right to left or top to bottom",
    ),
    (
        "kerning.setMode",
        "Kerning Mode",
        &[],
        "",
        "{tab?: id, mode?: \"none\" | \"kerning\" | \"locked\"} set the tab's kerning mode (omitted: the next of the three)",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let index = super::view::target_tab(app, args)?;
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?.clone();
    let tab = &tab;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let len = tab.text.len();
    // Every change is worked out on a copy, so a command that fails changes nothing.
    let mut next = tab.clone();
    match id {
        "editView.getText" => return Ok(reply(app, index)),
        "editView.toggleTextMode" => {
            next.text_mode = match args.string("mode")? {
                None => !tab.text_mode,
                Some("text") => true,
                Some("edit") => false,
                Some(other) => return Err(args.err(format!("`mode` must be \"text\" or \"edit\", got \"{other}\""))),
            };
            if next.text_mode && !tab.text_mode {
                let before = tab.edited_index().unwrap_or(tab.text.cursor());
                let current = tab.edited_glyph().map(str::to_string);
                let text = next.text.edit();
                text.set_cursor(before);
                text.set_current_glyph(current.as_deref());
            }
        }
        "editView.insertText" => {
            let items = items(font, args)?.ok_or_else(|| args.err("give `text`, `chars` or `glyphs`"))?;
            let range = match (args.index("at")?, args.numbers::<2>("replace")?) {
                (Some(_), Some(_)) => return Err(args.err("give `at` or `replace`, not both")),
                (Some(at), None) => {
                    let at = in_text(args, "at", at, len)?;
                    at..at
                }
                (None, Some([start, end])) => {
                    let (start, end) = (whole(args, "replace", start)?, whole(args, "replace", end)?);
                    let (start, end) = (in_text(args, "replace", start, len)?, in_text(args, "replace", end, len)?);
                    if start > end {
                        return Err(args.err("`replace` must run from a start to an end at or after it"));
                    }
                    start..end
                }
                (None, None) => tab.text.selection(),
            };
            edit(args, &mut next, range, items)?;
        }
        "editView.setText" => {
            let items = items(font, args)?.ok_or_else(|| args.err("give `text`, `chars` or `glyphs`"))?;
            edit(args, &mut next, 0..len, items)?;
        }
        "editView.setCursor" => {
            let at = args.index("index")?.ok_or_else(|| args.err("`index` (a position between items, 0 to the text's length) is required"))?;
            let at = in_text(args, "index", at, len)?;
            let length = args.index("selectionLength")?.unwrap_or(0);
            let end = usize::try_from(length).ok().and_then(|l| at.checked_add(l)).filter(|e| *e <= len);
            let end = end.ok_or_else(|| args.err(format!("`selectionLength` reaches past the end of the text ({len} items)")))?;
            next.text.edit().select(end, at);
        }
        "edit.addPlaceholder" => {
            let range = match args.index("at")? {
                Some(at) => {
                    let at = in_text(args, "at", at, len)?;
                    at..at
                }
                None => tab.text.selection(),
            };
            edit(args, &mut next, range, vec![Item::Placeholder])?;
        }
        "editView.previousGlyph" | "editView.nextGlyph" | "editView.previousVisibleGlyph" | "editView.nextVisibleGlyph" => {
            let current = tab.edited_glyph().ok_or_else(|| args.err("the tab has no current glyph"))?.to_string();
            let forward = matches!(id, "editView.nextGlyph" | "editView.nextVisibleGlyph");
            let replacement = if id.ends_with("VisibleGlyph") {
                super::font_view::refresh(app);
                let listed = super::font_view::visible_names(app);
                step_listed(&listed, &current, forward)
            } else {
                let font = app.session.font().map_err(|e| e.to_string())?;
                let position = font.glyphs.position(&current).ok_or_else(|| args.err(format!("the font has no glyph `{current}`")))?;
                let at = if forward { position.checked_add(1) } else { position.checked_sub(1) };
                at.and_then(|i| font.glyphs.as_arcs().get(i)).map(|g| g.name.clone())
            };
            if let Some(name) = replacement {
                let font = app.session.font().map_err(|e| e.to_string())?;
                replace_current(font, &mut next, &name);
            }
        }
        "editView.setWritingDirection" => {
            let name = args.string("direction")?.ok_or_else(|| args.err("`direction` (\"ltr\", \"rtl\" or \"ttb\") is required"))?;
            next.direction = WritingDirection::from_name(name)
                .ok_or_else(|| args.err(format!("`direction` must be \"ltr\", \"rtl\" or \"ttb\", got \"{name}\"")))?;
        }
        "kerning.setMode" => {
            next.kerning = match args.string("mode")? {
                Some(name) => KerningMode::from_name(name)
                    .ok_or_else(|| args.err(format!("`mode` must be \"none\", \"kerning\" or \"locked\", got \"{name}\"")))?,
                None => {
                    let at = KerningMode::ALL.iter().position(|m| *m == tab.kerning).unwrap_or(0);
                    KerningMode::ALL.get((at + 1) % KerningMode::ALL.len()).copied().unwrap_or_default()
                }
            };
        }
        other => return Err(format!("unknown command `{other}`")),
    }
    if let Some(tab) = app.state.tabs.get_mut(index) {
        *tab = next;
    }
    if let Some(tab) = app.state.tabs.get(index) {
        app.layouts.prepare(&mut app.session, tab);
    }
    Ok(reply(app, index))
}

/// The items `text`, `chars` or `glyphs` give (`None`: none of them).
fn items(font: &Font, args: &Args) -> Result<Option<Vec<Item>>, String> {
    let given = [args.get("text"), args.get("chars"), args.get("glyphs")].iter().filter(|v| v.is_some()).count();
    if given > 1 {
        return Err(args.err("give one of `text`, `chars` and `glyphs`"));
    }
    if let Some(text) = args.string("text")? {
        return parse(font, text, MAX_ITEMS).map(Some).map_err(|e| args.err(e));
    }
    if let Some(chars) = args.string("chars")? {
        if chars.chars().nth(MAX_ITEMS).is_some() {
            return Err(args.err(TextError::TooManyItems { limit: MAX_ITEMS }));
        }
        let map = Charmap::new(font);
        return Ok(Some(chars.chars().map(|c| if c == '\n' { Item::LineBreak } else { map.item(c) }).collect()));
    }
    let Some(glyphs) = args.strings("glyphs", MAX_TAB_GLYPHS)? else { return Ok(None) };
    if let Some(missing) = glyphs.iter().find(|g| font.glyph(g).is_none()) {
        return Err(args.err(format!("no glyph named `{missing}`")));
    }
    Ok(Some(glyphs.iter().map(|g| Item::glyph(g)).collect()))
}

/// Replaces `range` of the tab's text with `items`.
fn edit(args: &Args, tab: &mut Tab, range: Range<usize>, items: Vec<Item>) -> Result<(), String> {
    tab.text.edit().replace(range, items).map_err(|e| args.err(e))
}

fn in_text(args: &Args, key: &str, at: u64, len: usize) -> Result<usize, String> {
    usize::try_from(at).ok().filter(|a| *a <= len).ok_or_else(|| args.err(format!("`{key}` must lie between 0 and the text's length ({len})")))
}

fn whole(args: &Args, key: &str, n: f64) -> Result<u64, String> {
    if n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 { Ok(n as u64) } else { Err(args.err(format!("`{key}` must hold whole numbers"))) }
}

/// The glyph before or after `current` in `listed`; for a glyph not listed, the last or the
/// first listed.
fn step_listed(listed: &[String], current: &str, forward: bool) -> Option<String> {
    let found = match listed.iter().position(|n| n == current) {
        Some(at) if forward => at.checked_add(1),
        Some(at) => at.checked_sub(1),
        None if forward => Some(0),
        None => listed.len().checked_sub(1),
    };
    found.and_then(|i| listed.get(i)).cloned()
}

/// Puts glyph `name` in place of the item that shows the current glyph, and makes it the current
/// glyph; the cursor and the selection stay. A placeholder stays a placeholder and shows it.
fn replace_current(font: &Font, tab: &mut Tab, name: &str) {
    let at = tab.edited_index();
    let typed = font.glyph(name).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32);
    let (anchor, cursor) = (tab.text.anchor(), tab.text.cursor());
    let placeholder = at.is_some_and(|i| matches!(tab.text.items().get(i), Some(Item::Placeholder)));
    let text = tab.text.edit();
    if let Some(i) = at
        && !placeholder
    {
        let _ = text.replace(i..i + 1, vec![Item::Glyph { name: name.into(), typed }]);
    }
    text.set_current_glyph(Some(name));
    text.select(anchor, cursor);
}

/// `editView.getText`'s reply for the tab at `index`.
pub(crate) fn reply(app: &FontcraftApp, index: usize) -> Value {
    let Some(tab) = app.state.tabs.get(index) else { return json!({}) };
    let font = app.session.font().ok();
    let items: Vec<Value> = tab
        .text
        .items()
        .iter()
        .map(|item| match item {
            Item::Glyph { name, .. } => json!(name.as_ref()),
            Item::Placeholder => json!({"placeholder": true}),
            Item::LineBreak => json!({"lineBreak": true}),
            Item::Missing(c) => json!({"missing": c.to_string()}),
        })
        .collect();
    let selection = tab.text.selection();
    json!({
        "tab": tab.id,
        "items": items,
        "text": font.map(|f| tab.text.to_display_string(f)),
        "cursor": tab.text.cursor(),
        "selection": [selection.start, selection.end],
        "current": tab.edited_glyph(),
        "textMode": tab.text_mode,
        "direction": tab.direction.name(),
        "kerning": tab.kerning.name(),
        "master": font.and_then(|f| view::active_master(f, tab)).map(|m| m.id.clone()),
    })
}

/// The tab texts for a document's display strings, and a warning for each string that was cut
/// to the tab item limit or could not be read.
pub fn tabs_from_display_strings(font: &Font, strings: &[String]) -> (Vec<TabText>, Vec<String>) {
    let map = Charmap::new(font);
    let mut texts = Vec::new();
    let mut warnings = Vec::new();
    for (n, s) in strings.iter().enumerate() {
        let number = n + 1;
        let items = match parse_with(&map, s, MAX_ITEMS) {
            Ok(items) => items,
            Err(TextError::TooManyItems { .. }) => {
                // Read a longer prefix, then keep the first items: a string has at most one item
                // per character.
                let end = s.char_indices().nth(MAX_PREVIEW_ITEMS).map_or(s.len(), |(i, _)| i);
                let mut items = s.get(..end).map(|p| parse_with(&map, p, MAX_PREVIEW_ITEMS)).and_then(Result::ok).unwrap_or_default();
                items.truncate(MAX_ITEMS);
                warnings.push(format!("display string {number} holds more than {MAX_ITEMS} items; its tab shows the first {MAX_ITEMS}"));
                items
            }
            Err(e) => {
                warnings.push(format!("display string {number} cannot be read and opens no tab: {e}"));
                continue;
            }
        };
        let mut text = Text::new();
        if text.replace(0..0, items).is_ok() {
            text.set_cursor(0);
            texts.push(TabText::new(text));
        }
    }
    (texts, warnings)
}

/// The display strings for the open tabs, in strip order; none when the font's
/// `Write DisplayStrings` parameter is off.
pub fn display_strings(font: &Font, tabs: &[Tab]) -> Vec<String> {
    if font.info.params.get("Write DisplayStrings").and_then(param_flag) == Some(false) {
        return Vec::new();
    }
    tabs.iter().map(|t| t.text.to_display_string(font)).collect()
}

/// Writes the open tabs into the active document's display strings before a save.
pub(crate) fn store_display_strings(app: &mut FontcraftApp) {
    let Ok(font) = app.session.font() else { return };
    let strings = display_strings(font, &app.state.tabs);
    if let Err(e) = app.session.set_display_strings(strings) {
        log::warn!("cannot store the tabs' texts in the document: {e}");
    }
}

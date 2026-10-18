//! Text in Edit View tabs: the shell commands over a tab's text, cursor, writing direction and
//! kerning mode, and the document's display strings (the tabs' texts as the file stores them).
//!
//! A tab's text is view state, so these are shell commands (`docs/plans/M5.md` § 3). Each one
//! takes the tab by `tab` id (default: the showing tab) and answers with `editView.getText`'s
//! reply. Changing a text is no undo step and leaves the document unmodified.
//!
//! The Text tool's keys are here too ([`key`]): typing, deleting, the cursor keys, Escape,
//! Home and End, select all, the clipboard as text, and the spacing and kerning shortcuts with
//! the targets the cursor gives.

use std::ops::Range;

use fontcraft_model::{Font, param_flag};
use fontcraft_textlayout::{Charmap, Item, KerningMode, MAX_ITEMS, Text, TextError, parse, parse_prefix};
use fontcraft_tools::{Key, KeyEvent};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::state::{MAX_TAB_GLYPHS, Tab, TabText, WritingDirection};
use crate::tools_glue;

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
        "{tab?: id, index: integer, selectionLength?: integer, anchor?: integer, glyph?: string} put the text cursor before item `index` and select the `selectionLength` items after it, or the items between `anchor` and the cursor; the glyph after the cursor becomes the current glyph, or `glyph` when given (the glyph a preview feature shows there)",
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
                // At the end of the text the glyph before the cursor is the edited one, and the
                // cursor stays there.
                let cursor = tab.text.cursor();
                let before = match tab.edited_index() {
                    Some(i) if i + 1 == cursor && cursor == len => cursor,
                    Some(i) => i,
                    None => cursor,
                };
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
            let anchor = match (args.index("selectionLength")?, args.index("anchor")?) {
                (Some(_), Some(_)) => return Err(args.err("give `selectionLength` or `anchor`, not both")),
                (None, Some(anchor)) => in_text(args, "anchor", anchor, len)?,
                (length, None) => {
                    let end = usize::try_from(length.unwrap_or(0)).ok().and_then(|l| at.checked_add(l)).filter(|e| *e <= len);
                    end.ok_or_else(|| args.err(format!("`selectionLength` reaches past the end of the text ({len} items)")))?
                }
            };
            next.text.edit().select(anchor, at);
            if let Some(glyph) = args.string("glyph")? {
                if font.glyph(glyph).is_none() {
                    return Err(args.err(format!("the font has no glyph `{glyph}`")));
                }
                next.text.edit().set_current_glyph(Some(glyph));
            }
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
    if id == "editView.toggleTextMode" && app.state.active_tab == Some(index) {
        tool_follows_tab(app);
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

/// The tab texts for a document's display strings, at most `tabs` of them, and a warning for
/// each string that was cut to the tab item limit or could not be read, and for the strings
/// past `tabs`, which are not read.
pub fn tabs_from_display_strings(font: &Font, strings: &[String], tabs: usize) -> (Vec<TabText>, Vec<String>) {
    let map = Charmap::new(font);
    let mut texts = Vec::new();
    let mut warnings = Vec::new();
    for (n, s) in strings.iter().enumerate().take(tabs) {
        let number = n + 1;
        let items = match parse_prefix(&map, s, MAX_ITEMS) {
            Ok((items, false)) => items,
            Ok((items, true)) => {
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
    if strings.len() > tabs {
        warnings.push(format!("the document has {} display strings; only the first {tabs} open as tabs", strings.len()));
    }
    (texts, warnings)
}

/// The display strings for these tabs, in order; none when the font's `Write DisplayStrings`
/// parameter is off.
pub fn display_strings<'t>(font: &Font, tabs: impl IntoIterator<Item = &'t Tab>) -> Vec<String> {
    if font.info.params.get("Write DisplayStrings").and_then(param_flag) == Some(false) {
        return Vec::new();
    }
    tabs.into_iter().map(|t| t.text.to_display_string(font)).collect()
}

/// Writes the active document's open tabs into its display strings before a save.
pub(crate) fn store_display_strings(app: &mut FontcraftApp) {
    let Some(doc) = app.session.active() else { return };
    let strings = display_strings(&doc.font, app.state.tabs.iter().filter(|t| t.document == doc.id));
    if let Err(e) = app.session.set_display_strings(strings) {
        log::warn!("cannot store the tabs' texts in the document: {e}");
    }
}

// ───────────────────────────── The Text tool ─────────────────────────────

/// The id of the Text tool, which stands for text mode.
pub const TEXT_TOOL: &str = "text";

/// Makes the active tool match the showing tab's mode: the Text tool in text mode, the Select
/// tool when text mode ends with the Text tool active (a guess on `editView.toggleTextMode`). A
/// tab outside text mode keeps any other tool.
pub(crate) fn tool_follows_tab(app: &mut FontcraftApp) {
    let Some(text_mode) = app.state.tab().map(|t| t.text_mode) else { return };
    let tool = match (text_mode, app.tools.toolbox.base_id() == TEXT_TOOL) {
        (true, false) => TEXT_TOOL,
        (false, true) => "select",
        _ => return,
    };
    if let Ok(switch) = app.tools.toolbox.select(tool) {
        tools_glue::apply(app, switch.output);
    }
}

/// After `tool.select` made `tool` active: the Text tool puts the showing tab in text mode, and
/// any other tool ends it.
pub(crate) fn tab_follows_tool(app: &mut FontcraftApp, tool: &str) {
    let Some(tab) = app.state.tab() else { return };
    let mode = match (tool == TEXT_TOOL, tab.text_mode) {
        (true, false) => "text",
        (false, true) => "edit",
        _ => return,
    };
    if let Err(e) = quiet(app, "editView.toggleTextMode", json!({ "mode": mode })) {
        app.state.status = e;
    }
}

/// Runs a text command without the status line's "done": typing would fill it at every key.
fn quiet(app: &mut FontcraftApp, id: &str, params: Value) -> Result<Value, String> {
    let status = app.state.status.clone();
    let result = app.run(id, &params);
    if result.is_ok() {
        app.state.status = status;
    }
    result
}

/// Whether a temporary tool (the Hand tool of `Cmd+Space`) is held.
pub(crate) fn panning(app: &FontcraftApp) -> bool {
    app.tools.toolbox.active_id() != app.tools.toolbox.base_id()
}

/// Whether the showing tab is in text mode.
pub(crate) fn in_text_mode(app: &FontcraftApp) -> bool {
    app.state.tab().is_some_and(|t| t.text_mode)
}

/// A key press in text mode: the Text tool's keys (the `text` tool row and the `textMode`
/// shortcuts of `docs/spec/08` § 1.3 and § 3.4). With `types`, a character key types its
/// character (`ui.key`, `ui.text`); a frame's keys leave typing to its text events. Returns the
/// errors (also shown in the status line); `None` when the key is not text mode's, so it goes on
/// to the tools and the menu shortcuts: a release, `Cmd+Space` (the Hand tool), `Space` while
/// panning, and a character with `Cmd` or `Ctrl`.
pub(crate) fn key(app: &mut FontcraftApp, ev: &KeyEvent, types: bool) -> Option<Vec<String>> {
    if !ev.pressed || !in_text_mode(app) {
        return None;
    }
    let m = ev.mods;
    let plain = !m.cmd && !m.ctrl && !m.alt;
    let result = match ev.key {
        Key::Space if m.cmd || m.ctrl || panning(app) => return None,
        Key::Char('a') if m.cmd && !m.ctrl && !m.alt && !m.shift => select_all(app),
        Key::Char(_) if m.cmd || m.ctrl => return None,
        Key::Char(c) if types => {
            // The tools keep letters in lower case, with Shift in the modifiers.
            let c = if m.shift { c.to_ascii_uppercase() } else { c };
            type_chars(app, &c.to_string())
        }
        Key::Space if types => type_chars(app, " "),
        Key::Char(_) | Key::Space => Ok(()),
        Key::Return if plain => type_chars(app, "\n"),
        Key::Backspace | Key::Delete if plain && !m.shift => delete(app, ev.key == Key::Delete),
        Key::Escape if plain && !m.shift => quiet(app, "editView.toggleTextMode", json!({ "mode": "edit" })).map(|_| ()),
        Key::Home | Key::End if plain => {
            let id = match (ev.key == Key::End, m.shift) {
                (false, false) => "editView.previousGlyph",
                (true, false) => "editView.nextGlyph",
                (false, true) => "editView.previousVisibleGlyph",
                (true, true) => "editView.nextVisibleGlyph",
            };
            quiet(app, id, json!({})).map(|_| ())
        }
        Key::Left | Key::Right | Key::Up | Key::Down => match super::keymap::text_mode_command(ev.key, m) {
            Some(id) => shortcut(app, id),
            None if plain => move_cursor(app, ev.key, m.shift),
            None => Ok(()),
        },
        Key::Escape => return None,
        Key::Return | Key::Backspace | Key::Delete | Key::Home | Key::End | Key::Tab => Ok(()),
    };
    let errors: Vec<String> = result.err().into_iter().collect();
    if let Some(e) = errors.last() {
        app.state.status = e.clone();
    }
    Some(errors)
}

/// `Escape` outside text mode, when no tool took it: back to text mode, the cursor before the
/// edited glyph.
pub(crate) fn escape_to_text(app: &mut FontcraftApp, ev: &KeyEvent) -> Option<Vec<String>> {
    let m = ev.mods;
    if !ev.pressed || ev.key != Key::Escape || m.cmd || m.ctrl || m.alt || m.shift || in_text_mode(app) {
        return None;
    }
    app.state.tab()?;
    Some(quiet(app, "editView.toggleTextMode", json!({ "mode": "text" })).err().into_iter().collect())
}

/// Types `chars` at the cursor, in place of the selection.
pub(crate) fn type_chars(app: &mut FontcraftApp, chars: &str) -> Result<(), String> {
    if chars.is_empty() {
        return Ok(());
    }
    quiet(app, "editView.insertText", json!({ "chars": chars })).map(|_| ())
}

/// Backspace (`forward` false) or Delete: removes the selection, else the item before or after
/// the cursor.
fn delete(app: &mut FontcraftApp, forward: bool) -> Result<(), String> {
    let Some(tab) = app.state.tab() else { return Ok(()) };
    let (selection, cursor, len) = (tab.text.selection(), tab.text.cursor(), tab.text.len());
    let range = if !selection.is_empty() {
        selection
    } else if forward {
        cursor..cursor.saturating_add(1).min(len)
    } else {
        cursor.saturating_sub(1)..cursor
    };
    if range.is_empty() {
        return Ok(());
    }
    quiet(app, "editView.insertText", json!({ "chars": "", "replace": [range.start, range.end] })).map(|_| ())
}

fn select_all(app: &mut FontcraftApp) -> Result<(), String> {
    let Some(len) = app.state.tab().map(|t| t.text.len()) else { return Ok(()) };
    quiet(app, "editView.setCursor", json!({ "index": len, "anchor": 0 })).map(|_| ())
}

/// An arrow key: along the line in the writing direction (right to left: Left steps forward in
/// the text), or to the nearest place on the line before or after. Without Shift a selection
/// collapses to its end on the arrow's side; with Shift the selection follows the cursor.
fn move_cursor(app: &mut FontcraftApp, key: Key, extend: bool) -> Result<(), String> {
    let Some(tab) = app.state.tab() else { return Ok(()) };
    let (cursor, anchor, len, selection) = (tab.text.cursor(), tab.text.anchor(), tab.text.len(), tab.text.selection());
    let forward = match (tab.direction, key) {
        (WritingDirection::Ltr, Key::Left) | (WritingDirection::Rtl, Key::Right) | (WritingDirection::Ttb, Key::Up) => Some(false),
        (WritingDirection::Ltr, Key::Right) | (WritingDirection::Rtl, Key::Left) | (WritingDirection::Ttb, Key::Down) => Some(true),
        _ => None,
    };
    let target = match forward {
        Some(forward) if !extend && !selection.is_empty() => {
            if forward {
                selection.end
            } else {
                selection.start
            }
        }
        Some(true) => cursor.saturating_add(1).min(len),
        Some(false) => cursor.saturating_sub(1),
        None => match across_lines(app, tab, key) {
            Some(at) => at.min(len),
            None => return Ok(()),
        },
    };
    let anchor = if extend { anchor } else { target };
    quiet(app, "editView.setCursor", json!({ "index": target, "anchor": anchor })).map(|_| ())
}

/// The cursor place on the next line (`Down`; in vertical text the next column, `Left`) or the
/// previous one nearest to the cursor along the line. Past the last line: the end of the text;
/// before the first: its start.
fn across_lines(app: &FontcraftApp, tab: &Tab, key: Key) -> Option<usize> {
    let layout = view::text_layout(&app.session, &app.layouts, tab)?;
    let cursor = tab.text.cursor();
    let line = match layout.placed.get(cursor) {
        Some(p) => p.line,
        None => layout.lines.len().checked_sub(1)?,
    };
    // Lines follow downwards; columns leftwards.
    let next = matches!(key, Key::Down | Key::Left);
    let target = if next { line.checked_add(1).filter(|l| *l < layout.lines.len()) } else { line.checked_sub(1) };
    let Some(to) = target.and_then(|l| layout.lines.get(l)) else { return Some(if next { tab.text.len() } else { 0 }) };
    let (x, y) = layout.caret(cursor);
    Some(match tab.direction {
        WritingDirection::Ttb => layout.hit(to.baseline, y),
        // Half a line above the line's lower edge, where `hit` finds that line.
        WritingDirection::Ltr | WritingDirection::Rtl => layout.hit(x, to.baseline + layout.ascender - 0.5 * layout.line_height),
    })
}

/// A spacing or kerning shortcut in text mode. A spacing key acts on the current glyph in its
/// active layer (the layer picked for it, else its layer of the tab's master); it is off in
/// locked kerning mode. A kerning key acts on the pair on the current glyph's left (`…Left`,
/// Ctrl) or right (`…Right`, Cmd) as the text shows it, so in right-to-left text the left pair is
/// the current glyph and the one after it; it is off in no-kerning mode. The current glyph is the
/// one after the cursor, or the one before it at the end of the text. Top-to-bottom text has no shortcuts yet (`docs/spec/08`, question 2).
fn shortcut(app: &mut FontcraftApp, id: &'static str) -> Result<(), String> {
    let Some(tab) = app.state.tab() else { return Ok(()) };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let kerning = id.starts_with("kerning.");
    match (kerning, tab.kerning, tab.direction) {
        (_, _, WritingDirection::Ttb) => return Err("The spacing and kerning keys do not work in top-to-bottom text".into()),
        (true, KerningMode::None, _) => return Err("The kerning keys are off in the no-kerning mode".into()),
        (false, KerningMode::Locked, _) => return Err("The spacing keys are off in the locked kerning mode".into()),
        _ => {}
    }
    let master = view::active_master(font, tab).map(|m| m.id.clone()).ok_or("the tab shows no master")?;
    let params = if kerning {
        let current = tab.edited_index().ok_or("the tab has no current glyph")?;
        let before = current.checked_sub(1).and_then(|i| tab.item_glyph(i));
        let at = tab.item_glyph(current);
        let next = current.checked_add(1).and_then(|i| tab.item_glyph(i));
        let rtl = tab.direction == WritingDirection::Rtl;
        let (left, right) = if id.contains("Left") != rtl { (before, at) } else { (at, next) };
        let (Some(left), Some(right)) = (left, right) else {
            return Err("A kerning key needs a glyph on each side of the pair".into());
        };
        json!({ "left": left, "right": right, "master": master, "direction": if rtl { "rtl" } else { "ltr" } })
    } else {
        let glyph = tab.edited_glyph().ok_or("the tab has no current glyph")?;
        let layer = view::active_layer(font, tab).map_or(master, |l| l.id.clone());
        json!({ "glyphs": [glyph], "layer": layer })
    };
    app.run_menu_command(id, params).map(|_| ())
}

/// Why Paste Special does nothing in text mode.
pub(crate) const NO_PASTE_SPECIAL: &str = "Paste Special pastes into glyphs: leave text mode (Escape) to use it";

/// Edit › Copy, Cut, Delete and Select All from the menu in text mode, which act on the tab's
/// text as their keys do; Delete in All Masters and Paste Special, which work on outlines, are
/// refused there. `None` outside text mode or for another command, so it runs as usual. Replies
/// with `editView.getText`'s reply.
pub(crate) fn menu_command(app: &mut FontcraftApp, id: &str) -> Option<Result<Value, String>> {
    if !in_text_mode(app) {
        return None;
    }
    let result = match id {
        "edit.copy" | "edit.cut" => {
            if let Some(text) = selected_text(app) {
                app.clipboard_out = Some(text);
            }
            if id == "edit.cut" { delete_selection(app) } else { Ok(()) }
        }
        "edit.delete" => delete(app, true),
        "edit.deleteInAllMasters" => Err("Delete in All Masters removes outline points: leave text mode (Escape) to use it".to_string()),
        "edit.pasteSpecial" => Err(NO_PASTE_SPECIAL.to_string()),
        "edit.selectAll" => select_all(app),
        _ => return None,
    };
    Some(result.and_then(|()| app.run("editView.getText", &json!({}))))
}

/// The selected items as text for the clipboard: characters, a glyph without one as `/name`,
/// as the display strings write them (`text.clipboard`). `None` when nothing is selected.
pub(crate) fn selected_text(app: &FontcraftApp) -> Option<String> {
    let tab = app.state.tab()?;
    let font = app.session.font().ok()?;
    let items = tab.text.items().get(tab.text.selection())?.to_vec();
    if items.is_empty() {
        return None;
    }
    let mut text = Text::new();
    text.replace(0..0, items).ok()?;
    text.set_current_glyph(tab.edited_glyph());
    Some(text.to_display_string(font))
}

/// Deletes the selection (Cut, after the copy).
pub(crate) fn delete_selection(app: &mut FontcraftApp) -> Result<(), String> {
    let Some(selection) = app.state.tab().map(|t| t.text.selection()).filter(|s| !s.is_empty()) else { return Ok(()) };
    quiet(app, "editView.insertText", json!({ "chars": "", "replace": [selection.start, selection.end] })).map(|_| ())
}

/// Pastes clipboard text at the cursor, in place of the selection: `/name` reads as the glyph
/// name, as in display strings.
pub(crate) fn paste(app: &mut FontcraftApp, text: &str) -> Result<(), String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.is_empty() {
        return Ok(());
    }
    quiet(app, "editView.insertText", json!({ "text": text })).map(|_| ())
}

//! The edit view's text commands. The text is view state, so these are shell commands: no undo step, the document
//! stays unmodified. Each works on a copy of the text and keeps it only when nothing failed, so a command that fails
//! changes nothing. `ui.inspect` reports the result under `editText`.

use std::ops::Range;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::{Font, MAX_COORDINATE};
use fontcraft_spacing::kerning::{Position, facing, members};
use fontcraft_textlayout::{Charmap, Item, KerningMode, MAX_ITEMS, TextError, parse};
use serde_json::{Map, Value};

use super::{EditText, TEXT_TOOL, WritingDirection};
use crate::app::FontCraftApp;
use crate::shell::commands::{Module, UiCommand};

pub const MODULE: Module = Module { commands: COMMANDS, run };

const fn command(
    id: &'static str,
    label: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    params: &'static str,
) -> UiCommand {
    UiCommand { id, label, menu, shortcut, params }
}

const COMMANDS: &[UiCommand] = &[
    command(
        "editView.toggleTextMode",
        "Toggle Text Mode",
        &[],
        None,
        "{mode?: \"text\" | \"edit\"} switch the edit view between typing text (the Text tool) and editing the current glyph (the Select tool); omitted: the other one. Entering text mode puts the cursor before the edited glyph",
    ),
    command(
        "editView.insertText",
        "Insert Text",
        &[],
        None,
        "{text?: string, chars?: string, glyphs?: [names], at?: index, replace?: [start, end]} insert into the edit view's text: `text` reads `/name` as the glyph name and `/Placeholder` as a placeholder, `chars` takes every character as typed, `glyphs` names glyphs; they go in place of the selection, at `at`, or in place of the items from start to end, and the cursor ends after them",
    ),
    command(
        "editView.setText",
        "Set Tab Text",
        &[],
        None,
        "{text?: string, chars?: string, glyphs?: [names]} replace the whole text of the edit view (`text`, `chars` and `glyphs` as Insert Text reads them); the cursor ends after it",
    ),
    command(
        "editView.setCursor",
        "Set Text Cursor",
        &[],
        None,
        "{index: integer, selectionLength?: integer, anchor?: integer, glyph?: string} put the text cursor before item `index` and select the `selectionLength` items after it, or the items between `anchor` and the cursor; the glyph after the cursor becomes the current glyph, or `glyph` when given",
    ),
    command(
        "edit.addPlaceholder",
        "Add Placeholder",
        &["Edit"],
        Some("Cmd+Alt+Shift+P"),
        "{at?: index} insert a placeholder, which shows the current glyph, in place of the selection or at `at`",
    ),
    command(
        "editView.previousGlyph",
        "Previous Glyph",
        &[],
        Some("Home"),
        "{} replace the current glyph in the text with the glyph before it in the font's glyph order (at the first: nothing changes)",
    ),
    command(
        "editView.nextGlyph",
        "Next Glyph",
        &[],
        Some("End"),
        "{} replace the current glyph in the text with the glyph after it in the font's glyph order (at the last: nothing changes)",
    ),
    command(
        "editView.previousVisibleGlyph",
        "Previous Glyph in Font View",
        &[],
        Some("Shift+Home"),
        "{} replace the current glyph with the glyph before it as Font View lists the glyphs now (a glyph Font View does not list: the last one listed)",
    ),
    command(
        "editView.nextVisibleGlyph",
        "Next Glyph in Font View",
        &[],
        Some("Shift+End"),
        "{} replace the current glyph with the glyph after it as Font View lists the glyphs now (a glyph Font View does not list: the first one listed)",
    ),
    command(
        "editView.setWritingDirection",
        "Writing Direction",
        &[],
        None,
        "{direction: \"ltr\" | \"rtl\" | \"ttb\"} lay the edit view's text out left to right, right to left or top to bottom",
    ),
    command(
        "kerning.setMode",
        "Kerning Mode",
        &[],
        None,
        "{mode?: \"none\" | \"kerning\" | \"locked\"} set the edit view's kerning mode (omitted: the next of the three)",
    ),
    command(
        "kerning.showPairInEditView",
        "Show Pair in Edit View",
        &[],
        None,
        "{left: name|\"@group\", right: name|\"@group\"} put a kerning pair into the edit view's text in place of the pair at the current glyph (the current glyph and the one before it; with no glyph before it, at the cursor); the cursor ends between the two. A group stands for its first member on the side facing the pair",
    ),
    command(
        "view.showGroupMembers",
        "Show Group Members",
        &["View"],
        None,
        "{on?: bool} draw, behind the edited pair, the other members of the edited glyph's facing kerning group and of the preceding glyph's (omitted: toggle)",
    ),
    command(
        "view.showMeasurementLine",
        "Show Measurement Line",
        &["View"],
        None,
        "{on?: bool} show the measurement line in text mode (omitted: toggle): a line at an adjustable height with each glyph's sidebearings measured on it, and thin lines at the edges of every advance. It shows only while the Text tool is active, in horizontal text",
    ),
    command(
        "view.setMeasurementLineHeight",
        "Set Measurement Line Height",
        &[],
        None,
        "{y: number (from -32768 to 32767)} put the measurement line at this height above the baseline, in font units",
    ),
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let empty = Map::new();
    let p = command.params.as_object().unwrap_or(&empty);
    let id = command.id.as_str();
    if id == "editView.toggleTextMode" {
        return toggle(app, p);
    }
    if id == "view.showGroupMembers" {
        let on = flag(p, "on")?.unwrap_or(!app.view.text.group_members);
        app.view.text.group_members = on;
        app.view.text.touch();
        return Ok(());
    }
    if id == "view.showMeasurementLine" {
        let on = flag(p, "on")?.unwrap_or(!app.view.text.measurement);
        app.view.text.measurement = on;
        app.view.text.touch();
        return Ok(());
    }
    if id == "view.setMeasurementLineHeight" {
        let y = p.get("y").and_then(Value::as_f64).ok_or("`y` (height above the baseline) is required")?;
        if !y.is_finite() || y.abs() > MAX_COORDINATE {
            return Err(format!("`y` must lie within ±{MAX_COORDINATE}"));
        }
        app.view.text.measurement_height = Some(y);
        app.view.text.touch();
        return Ok(());
    }
    if id == "kerning.showPairInEditView" {
        return show_pair(app, p);
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    // Commands on the text work on a text that shows the edited glyph.
    if !super::shows_text(font, &app.view) {
        super::hold_shown_glyph(app);
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let mut next = app.view.text.clone();
    let len = next.text.len();
    match id {
        "editView.insertText" => {
            let items = items(font, p)?.ok_or("give `text`, `chars` or `glyphs`")?;
            let range = match (index(p, "at")?, p.get("replace")) {
                (Some(_), Some(_)) => return Err("give `at` or `replace`, not both".into()),
                (Some(at), None) => {
                    let at = in_text("at", at, len)?;
                    at..at
                }
                (None, Some(replace)) => span(replace, len)?,
                (None, None) => next.text.selection(),
            };
            edit(&mut next, range, items)?;
        }
        "editView.setText" => {
            let items = items(font, p)?.ok_or("give `text`, `chars` or `glyphs`")?;
            edit(&mut next, 0..len, items)?;
            next.fit_pending = true;
        }
        "editView.setCursor" => {
            let at = index(p, "index")?.ok_or("`index` (a position between items, 0 to the text's length) is required")?;
            let at = in_text("index", at, len)?;
            let anchor = match (index(p, "selectionLength")?, index(p, "anchor")?) {
                (Some(_), Some(_)) => return Err("give `selectionLength` or `anchor`, not both".into()),
                (None, Some(anchor)) => in_text("anchor", anchor, len)?,
                (length, None) => {
                    let end = usize::try_from(length.unwrap_or(0)).ok().and_then(|l| at.checked_add(l)).filter(|e| *e <= len);
                    end.ok_or_else(|| format!("`selectionLength` reaches past the end of the text ({len} items)"))?
                }
            };
            next.text.select(anchor, at);
            if let Some(glyph) = string(p, "glyph")? {
                if font.glyph(glyph).is_none() {
                    return Err(format!("the font has no glyph `{glyph}`"));
                }
                next.text.set_current_glyph(Some(glyph));
            }
        }
        "edit.addPlaceholder" => {
            let range = match index(p, "at")? {
                Some(at) => {
                    let at = in_text("at", at, len)?;
                    at..at
                }
                None => next.text.selection(),
            };
            edit(&mut next, range, vec![Item::Placeholder])?;
        }
        "editView.previousGlyph" | "editView.nextGlyph" | "editView.previousVisibleGlyph" | "editView.nextVisibleGlyph" => {
            let current = next.current().ok_or("the edit view has no current glyph")?.to_string();
            let forward = matches!(id, "editView.nextGlyph" | "editView.nextVisibleGlyph");
            let replacement = if id.ends_with("VisibleGlyph") {
                let listed: Vec<&str> = app.models.font_view.rows.iter().map(|r| r.name.as_str()).collect();
                step_listed(&listed, &current, forward)
            } else {
                let position = font.glyphs.position(&current).ok_or_else(|| format!("the font has no glyph `{current}`"))?;
                let at = if forward { position.checked_add(1) } else { position.checked_sub(1) };
                at.and_then(|i| font.glyphs.as_arcs().get(i)).map(|g| g.name.clone())
            };
            if let Some(name) = replacement {
                replace_current(font, &mut next, &name);
            }
        }
        "editView.setWritingDirection" => {
            let name = string(p, "direction")?.ok_or("`direction` (\"ltr\", \"rtl\" or \"ttb\") is required")?;
            next.direction =
                WritingDirection::from_name(name).ok_or_else(|| format!("`direction` must be \"ltr\", \"rtl\" or \"ttb\", got \"{name}\""))?;
        }
        "kerning.setMode" => {
            next.kerning = match string(p, "mode")? {
                Some(name) => {
                    KerningMode::from_name(name).ok_or_else(|| format!("`mode` must be \"none\", \"kerning\" or \"locked\", got \"{name}\""))?
                }
                None => {
                    let at = KerningMode::ALL.iter().position(|m| *m == next.kerning).unwrap_or(0);
                    KerningMode::ALL.get((at + 1) % KerningMode::ALL.len()).copied().unwrap_or_default()
                }
            };
        }
        other => return Err(format!("unknown command `{other}`")),
    }
    app.view.text = next;
    super::follow(app);
    Ok(())
}

/// `editView.toggleTextMode`: the Text tool for text mode, the Select tool for edit mode.
fn toggle(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let on = super::text_mode(&app.view);
    let text = match string(p, "mode")? {
        None => !on,
        Some("text") => true,
        Some("edit") => false,
        Some(other) => return Err(format!("`mode` must be \"text\" or \"edit\", got \"{other}\"")),
    };
    if text == on {
        return Ok(());
    }
    if text {
        super::hold_shown_glyph(app);
        // The cursor goes before the edited glyph; at the end of the text it stays after the last one.
        let state = &mut app.view.text;
        let len = state.text.len();
        let cursor = state.text.cursor();
        let before = match state.edited_index() {
            Some(i) if i + 1 == cursor && cursor == len => cursor,
            Some(i) => i,
            None => cursor,
        };
        let current = state.current().map(str::to_string);
        state.text.set_cursor(before);
        state.text.set_current_glyph(current.as_deref());
        state.touch();
    }
    let tool = if text { TEXT_TOOL } else { "select" };
    app.view.tool = tool;
    app.view_changed();
    crate::canvas::tools::apply::select_tool(app, tool)?;
    super::sync(app);
    Ok(())
}

/// `kerning.showPairInEditView`: the pair in place of the pair at the current glyph, the cursor between the two.
pub fn show_pair(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let left = string(p, "left")?.ok_or("`left` (a glyph name or @group) is required")?.to_string();
    let right = string(p, "right")?.ok_or("`right` (a glyph name or @group) is required")?.to_string();
    let font = app.session.font().map_err(|e| e.to_string())?;
    if !super::shows_text(font, &app.view) {
        super::hold_shown_glyph(app);
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let direction = app.view.text.direction.layout();
    let l = side_glyph(font, direction, Position::First, &left)?;
    let r = side_glyph(font, direction, Position::Second, &right)?;
    let mut next = app.view.text.clone();
    let range = match next.edited_index() {
        Some(i) if i > 0 && next.item_glyph(i - 1).is_some() => i - 1..i + 1,
        _ => {
            let c = next.text.cursor();
            c..c
        }
    };
    let start = range.start;
    edit(&mut next, range, vec![glyph_item(font, &l), glyph_item(font, &r)])?;
    next.text.set_cursor(start + 1);
    app.view.text = next;
    super::follow(app);
    Ok(())
}

/// The glyph a pair side shows: the glyph itself, or the first member of the group on the side facing the pair.
fn side_glyph(font: &Font, direction: fontcraft_textlayout::Direction, position: Position, side: &str) -> Result<String, String> {
    match side.strip_prefix('@') {
        None => font.glyph(side).map(|g| g.name.clone()).ok_or_else(|| format!("the font has no glyph `{side}`")),
        Some(group) => {
            let side = facing(direction, position);
            members(font, side, group)
                .first()
                .map(|m| m.to_string())
                .ok_or_else(|| format!("no glyph has the {} kerning group `{group}`", side.name()))
        }
    }
}

pub(crate) fn glyph_item(font: &Font, name: &str) -> Item {
    let typed = font.glyph(name).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32);
    Item::Glyph { name: name.into(), typed }
}

/// The items `text`, `chars` or `glyphs` give (`None`: none of them).
pub(crate) fn items(font: &Font, p: &Map<String, Value>) -> Result<Option<Vec<Item>>, String> {
    let given = ["text", "chars", "glyphs"].iter().filter(|k| p.contains_key(**k)).count();
    if given > 1 {
        return Err("give one of `text`, `chars` and `glyphs`".into());
    }
    if let Some(text) = string(p, "text")? {
        return parse(font, text, MAX_ITEMS).map(Some).map_err(|e| e.to_string());
    }
    if let Some(chars) = string(p, "chars")? {
        if chars.chars().nth(MAX_ITEMS).is_some() {
            return Err(TextError::TooManyItems { limit: MAX_ITEMS }.to_string());
        }
        let map = Charmap::new(font);
        return Ok(Some(chars.chars().map(|c| if c == '\n' { Item::LineBreak } else { map.item(c) }).collect()));
    }
    let Some(glyphs) = p.get("glyphs") else { return Ok(None) };
    let list = glyphs.as_array().ok_or("`glyphs` must be a list of glyph names")?;
    if list.len() > MAX_ITEMS {
        return Err(TextError::TooManyItems { limit: MAX_ITEMS }.to_string());
    }
    let mut out = Vec::with_capacity(list.len());
    for name in list {
        let name = name.as_str().ok_or("`glyphs` must be a list of glyph names")?;
        if font.glyph(name).is_none() {
            return Err(format!("no glyph named `{name}`"));
        }
        out.push(glyph_item(font, name));
    }
    Ok(Some(out))
}

/// Replaces `range` of the text with `items`.
fn edit(text: &mut EditText, range: Range<usize>, items: Vec<Item>) -> Result<(), String> {
    text.text.replace(range, items).map_err(|e| e.to_string())
}

/// `replace: [start, end]` inside a text of `len` items.
fn span(value: &Value, len: usize) -> Result<Range<usize>, String> {
    let pair = value.as_array().filter(|a| a.len() == 2).ok_or("`replace` must be [start, end]")?;
    let whole = |v: Option<&Value>| v.and_then(Value::as_u64).ok_or("`replace` must hold whole numbers");
    let (start, end) = (whole(pair.first())?, whole(pair.get(1))?);
    let (start, end) = (in_text("replace", start, len)?, in_text("replace", end, len)?);
    if start > end {
        return Err("`replace` must run from a start to an end at or after it".into());
    }
    Ok(start..end)
}

fn in_text(key: &str, at: u64, len: usize) -> Result<usize, String> {
    usize::try_from(at).ok().filter(|a| *a <= len).ok_or_else(|| format!("`{key}` must lie between 0 and the text's length ({len})"))
}

/// The glyph before or after `current` in `listed`; for a glyph not listed, the last or the first listed.
fn step_listed(listed: &[&str], current: &str, forward: bool) -> Option<String> {
    let found = match listed.iter().position(|n| *n == current) {
        Some(at) if forward => at.checked_add(1),
        Some(at) => at.checked_sub(1),
        None if forward => Some(0),
        None => listed.len().checked_sub(1),
    };
    found.and_then(|i| listed.get(i)).map(|s| s.to_string())
}

/// Puts glyph `name` in place of the item that shows the current glyph and makes it the current glyph; the cursor
/// and the selection stay. A placeholder stays a placeholder and shows it.
fn replace_current(font: &Font, text: &mut EditText, name: &str) {
    let at = text.edited_index();
    let (selection, cursor) = (text.text.selection(), text.text.cursor());
    let anchor = if cursor == selection.start { selection.end } else { selection.start };
    let placeholder = at.is_some_and(|i| matches!(text.text.items().get(i), Some(Item::Placeholder)));
    if let Some(i) = at
        && !placeholder
    {
        // The item keeps its own layer while the new glyph has one of the same id (a master's).
        let own = text.text.layer(i).filter(|id| font.glyph(name).is_some_and(|g| g.layers.iter().any(|l| l.id == *id))).map(str::to_string);
        if text.text.replace(i..i + 1, vec![glyph_item(font, name)]).is_ok() {
            text.text.set_layer(i, own.as_deref());
        }
    }
    text.text.select(anchor, cursor);
    text.text.set_current_glyph(Some(name));
}

fn string<'a>(p: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn index(p: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| format!("`{key}` must be a whole number, 0 or more")),
    }
}

fn flag(p: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

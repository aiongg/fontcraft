//! The Text tool's input, run in `drain`: a click places the cursor, a drag selects, Shift with a click extends the
//! selection, a double-click on a glyph edits it; letters type, the deletes delete, the arrows move the cursor (with
//! Alt by word, Cmd+Up and Cmd+Down to the text's start and end), and the spacing and kerning shortcuts (`when = textMode`) act on the glyphs at the cursor.

use std::sync::Arc;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::cmd::CommandSpec;
use fontcraft_engine::command_specs;
use fontcraft_engine::shortcuts::Custom;
use fontcraft_textlayout::{Item, KerningMode, Layout};
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use serde_json::{Value, json};

use super::WritingDirection;
use crate::app::models::text as model;
use crate::app::{FontCraftApp, Msg};
use crate::canvas::tools::ToolInput;

/// Where the engine's commands say a shortcut works in text mode.
const TEXT_MODE: &str = "textMode";

/// Hands one event to text mode. Returns the errors, in order.
pub fn input(app: &mut FontCraftApp, input: ToolInput) -> Vec<String> {
    let result = match input {
        ToolInput::Pointer(ev) => pointer(app, ev),
        ToolInput::Key(ev) => key(app, ev),
        ToolInput::Modifiers(_) => Ok(()),
    };
    result.err().into_iter().collect()
}

/// Whether text mode takes the key: every key but a character or Space with Cmd or Ctrl (the app's shortcuts),
/// Cmd+A aside, which selects the whole text.
pub fn takes_key(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    match ev.key {
        Key::Char('a') if m.cmd && !m.ctrl && !m.alt && !m.shift => true,
        Key::Char(_) | Key::Space => !(m.cmd || m.ctrl),
        _ => true,
    }
}

/// Escape in edit mode that no tool took: back to text mode, the cursor before the edited glyph.
pub fn escape_to_text(app: &mut FontCraftApp, ev: &KeyEvent) -> Option<Result<(), String>> {
    let m = ev.mods;
    if !ev.pressed || ev.key != Key::Escape || m.cmd || m.ctrl || m.alt || m.shift || super::text_mode(&app.view) {
        return None;
    }
    Some(shell(app, "editView.toggleTextMode", json!({"mode": "text"})))
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<(), String> {
    app.apply_msg(Msg::Shell(CommandRef::with_params(id, params)))
}

/// The layout of the text as it is now: the projected one while nothing changed since, else a fresh one.
fn layout(app: &mut FontCraftApp) -> Option<Arc<Layout>> {
    let current = model::key(&app.session, &app.view);
    match (&app.models.text.layout, &app.models.text.key) {
        (Some(layout), Some(key)) if current.as_ref() == Some(key) => Some(layout.clone()),
        _ => {
            let shaper = app.models.text.shaper.for_preview(super::features::preview(&app.session, &app.view.text.features));
            model::layout(&mut app.session, &app.view, &shaper)
        }
    }
}

fn pointer(app: &mut FontCraftApp, ev: PointerEvent) -> Result<(), String> {
    if ev.kind == PointerKind::Move || !app.models.text.shown {
        return Ok(());
    }
    let Some(layout) = layout(app) else { return Ok(()) };
    // The event is in the font units of the glyph the canvas drew as edited; the text point is where that glyph sat.
    let [ox, oy] = app.models.text.origin;
    let (x, y) = (ev.pos.x + ox, ev.pos.y + oy);
    if let Some(result) = measurement_pointer(app, &layout, &ev, x, y) {
        app.view.text.drag = None;
        return result;
    }
    let index = layout.hit(x, y);
    let text = &app.view.text.text;
    match ev.kind {
        PointerKind::DoubleClick => {
            app.view.text.drag = None;
            if let Some(item) = model::item_at(&layout, x, y) {
                shell(app, "editView.setCursor", json!({"index": item}))?;
                // Leaving text mode makes the Select tool active, as Escape does.
                shell(app, "editView.toggleTextMode", json!({"mode": "edit"}))?;
            }
            Ok(())
        }
        PointerKind::Down | PointerKind::Drag | PointerKind::Up => {
            let anchor = match (ev.kind, app.view.text.drag) {
                // Shift keeps the end of the selection away from the click.
                (PointerKind::Down, _) if ev.mods.shift => {
                    let s = text.selection();
                    Some(if index < s.start { s.end } else { s.start })
                }
                (PointerKind::Down, _) => Some(index),
                // A drag or release without a press on the text gives `None`.
                (_, anchor) => anchor,
            };
            let Some(anchor) = anchor else { return Ok(()) };
            let changed = text.selection() != (anchor.min(index)..anchor.max(index)) || text.cursor() != index;
            app.view.text.drag = (ev.kind != PointerKind::Up).then_some(anchor);
            if changed { shell(app, "editView.setCursor", json!({"index": index, "anchor": anchor})) } else { Ok(()) }
        }
        PointerKind::Move => Ok(()),
    }
}

/// Cmd+Ctrl+Alt with a press or a drag, while the measurement line shows: moves the line to the pointer's height
/// above the baseline of the line under it. (x, y) is in text coordinates. `None` when the event is not the line's.
fn measurement_pointer(app: &mut FontCraftApp, layout: &Layout, ev: &PointerEvent, x: f64, y: f64) -> Option<Result<(), String>> {
    let m = ev.mods;
    if !(m.cmd && m.ctrl && m.alt) || app.models.text.measurement.is_none() {
        return None;
    }
    match ev.kind {
        PointerKind::Down | PointerKind::Drag => {}
        PointerKind::Up => return Some(Ok(())),
        PointerKind::DoubleClick | PointerKind::Move => return None,
    }
    let line = layout.placed.get(layout.hit(x, y)).or(layout.placed.last()).map_or(0, |p| p.line);
    let base = layout.lines.get(line).map_or(0.0, |l| l.baseline);
    let height = (y - base).round().clamp(-fontcraft_engine::model::MAX_COORDINATE, fontcraft_engine::model::MAX_COORDINATE);
    if !height.is_finite() {
        return Some(Ok(()));
    }
    Some(shell(app, "view.setMeasurementLineHeight", json!({"y": height})))
}

/// A key in text mode: the Text tool's keys (`docs/spec/08` § 1.3 and § 3.4). Releases do nothing.
fn key(app: &mut FontCraftApp, ev: KeyEvent) -> Result<(), String> {
    if !ev.pressed {
        return Ok(());
    }
    let m = ev.mods;
    let plain = !m.cmd && !m.ctrl && !m.alt;
    match ev.key {
        Key::Char('a') if m.cmd && !m.ctrl && !m.alt && !m.shift => select_all(app),
        Key::Char(_) | Key::Space if m.cmd || m.ctrl => Ok(()),
        Key::Char(c) => {
            // The tools keep letters in lower case, with Shift in the modifiers.
            let c = if m.shift { c.to_ascii_uppercase() } else { c };
            type_chars(app, &c.to_string())
        }
        Key::Space => type_chars(app, " "),
        Key::Return if plain => type_chars(app, "\n"),
        Key::Backspace | Key::Delete if plain && !m.shift => delete(app, ev.key == Key::Delete),
        Key::Escape if plain && !m.shift => shell(app, "editView.toggleTextMode", json!({"mode": "edit"})),
        Key::Home | Key::End if plain => {
            let id = match (ev.key == Key::End, m.shift) {
                (false, false) => "editView.previousGlyph",
                (true, false) => "editView.nextGlyph",
                (false, true) => "editView.previousVisibleGlyph",
                (true, true) => "editView.nextVisibleGlyph",
            };
            shell(app, id, json!({}))
        }
        Key::Left | Key::Right | Key::Up | Key::Down => match text_mode_command(app.catalog.custom(), ev.key, m) {
            Some(id) => shortcut(app, id),
            None if plain => move_cursor(app, ev.key, m.shift),
            None if m.alt && !m.cmd && !m.ctrl => move_by_word(app, ev.key, m.shift),
            None if m.cmd && !m.alt && !m.ctrl && matches!(ev.key, Key::Up | Key::Down) => to_text_end(app, ev.key == Key::Down, m.shift),
            None => Ok(()),
        },
        Key::Return | Key::Backspace | Key::Delete | Key::Home | Key::End | Key::Tab | Key::Escape => Ok(()),
    }
}

/// The engine command whose text-mode shortcut (the user's own, else its built-in one) is `key` with `mods`.
pub fn text_mode_command(custom: &Custom, key: Key, mods: Modifiers) -> Option<&'static str> {
    let shortcut = |c: &CommandSpec| custom.shortcut(c.id, c.shortcut).as_deref().and_then(parse_shortcut);
    command_specs().iter().filter(|c| c.when == Some(TEXT_MODE)).find(|c| shortcut(c) == Some((key, mods))).map(|c| c.id)
}

/// A shortcut such as `Cmd+Ctrl+Shift+Left` as a key of the tools and its modifiers.
fn parse_shortcut(text: &str) -> Option<(Key, Modifiers)> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part {
            "Cmd" => mods.cmd = true,
            "Ctrl" => mods.ctrl = true,
            "Alt" | "Option" => mods.alt = true,
            "Shift" => mods.shift = true,
            other => key = Some(Key::parse(other)?),
        }
    }
    Some((key?, mods))
}

/// Types `chars` at the cursor, in place of the selection.
pub fn type_chars(app: &mut FontCraftApp, chars: &str) -> Result<(), String> {
    if chars.is_empty() {
        return Ok(());
    }
    shell(app, "editView.insertText", json!({"chars": chars}))
}

/// Backspace (`forward` false) or Delete: removes the selection, else the item before or after the cursor.
fn delete(app: &mut FontCraftApp, forward: bool) -> Result<(), String> {
    let text = &app.view.text.text;
    let (selection, cursor, len) = (text.selection(), text.cursor(), text.len());
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
    shell(app, "editView.insertText", json!({"chars": "", "replace": [range.start, range.end]}))
}

fn select_all(app: &mut FontCraftApp) -> Result<(), String> {
    let len = app.view.text.text.len();
    shell(app, "editView.setCursor", json!({"index": len, "anchor": 0}))
}

/// Whether an arrow key moves forward in the text along the line in `direction` (right to left: Left steps forward;
/// top to bottom: Down does), backward, or neither (it moves across lines).
fn along_line(direction: WritingDirection, key: Key) -> Option<bool> {
    match (direction, key) {
        (WritingDirection::Ltr, Key::Left) | (WritingDirection::Rtl, Key::Right) | (WritingDirection::Ttb, Key::Up) => Some(false),
        (WritingDirection::Ltr, Key::Right) | (WritingDirection::Rtl, Key::Left) | (WritingDirection::Ttb, Key::Down) => Some(true),
        _ => None,
    }
}

/// Whether `item` ends a word: a line break, or a glyph typed as white space (or the glyph `space` entered by name).
fn ends_word(item: &Item) -> bool {
    match item {
        Item::LineBreak => true,
        Item::Glyph { typed: Some(c), .. } | Item::Missing(c) => c.is_whitespace(),
        Item::Glyph { typed: None, name } => &**name == "space",
        Item::Placeholder => false,
    }
}

/// The cursor place one word on from `from`: forward, past any spaces and line breaks and then to the end of the
/// next word; backward, to the start of the word before.
pub fn word_boundary(items: &[Item], from: usize, forward: bool) -> usize {
    let from = from.min(items.len());
    if forward {
        let rest = items.get(from..).unwrap_or_default();
        let gap = rest.iter().take_while(|i| ends_word(i)).count();
        let word = rest.iter().skip(gap).take_while(|i| !ends_word(i)).count();
        from.saturating_add(gap).saturating_add(word)
    } else {
        let before = items.get(..from).unwrap_or_default();
        let gap = before.iter().rev().take_while(|i| ends_word(i)).count();
        let word = before.iter().rev().skip(gap).take_while(|i| !ends_word(i)).count();
        from.saturating_sub(gap).saturating_sub(word)
    }
}

/// Alt with an arrow along the line: the cursor moves to the end of the word after it or the start of the word
/// before it. Without Shift it starts from the selection's edge on the arrow's side and collapses the selection; with
/// Shift the selection follows the cursor. Alt with an arrow across lines does nothing.
fn move_by_word(app: &mut FontCraftApp, key: Key, extend: bool) -> Result<(), String> {
    let state = &app.view.text;
    let Some(forward) = along_line(state.direction, key) else { return Ok(()) };
    let (cursor, selection) = (state.text.cursor(), state.text.selection());
    let anchor = if cursor == selection.start { selection.end } else { selection.start };
    let from = match (extend, forward) {
        (true, _) => cursor,
        (false, true) => selection.end,
        (false, false) => selection.start,
    };
    let target = word_boundary(state.text.items(), from, forward);
    let anchor = if extend { anchor } else { target };
    shell(app, "editView.setCursor", json!({"index": target, "anchor": anchor}))
}

/// Cmd+Up and Cmd+Down: the cursor to the start (`end` false) or the end of the text, in every writing direction.
/// With Shift the selection runs from where it was anchored to there.
fn to_text_end(app: &mut FontCraftApp, end: bool, extend: bool) -> Result<(), String> {
    let text = &app.view.text.text;
    let (cursor, selection) = (text.cursor(), text.selection());
    let anchor = if cursor == selection.start { selection.end } else { selection.start };
    let target = if end { text.len() } else { 0 };
    let anchor = if extend { anchor } else { target };
    shell(app, "editView.setCursor", json!({"index": target, "anchor": anchor}))
}

/// An arrow key: along the line in the writing direction (right to left: Left steps forward in the text), or to the
/// nearest place on the line before or after. Without Shift a selection collapses to its end on the arrow's side;
/// with Shift the selection follows the cursor.
fn move_cursor(app: &mut FontCraftApp, key: Key, extend: bool) -> Result<(), String> {
    let state = &app.view.text;
    let (cursor, len, selection) = (state.text.cursor(), state.text.len(), state.text.selection());
    let anchor = if cursor == selection.start { selection.end } else { selection.start };
    let forward = along_line(state.direction, key);
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
        None => match across_lines(app, key) {
            Some(at) => at.min(len),
            None => return Ok(()),
        },
    };
    let anchor = if extend { anchor } else { target };
    shell(app, "editView.setCursor", json!({"index": target, "anchor": anchor}))
}

/// The cursor place on the next line (`Down`; in vertical text the next column, `Left`) or the previous one nearest
/// to the cursor along the line. Past the last line: the end of the text; before the first: its start.
fn across_lines(app: &mut FontCraftApp, key: Key) -> Option<usize> {
    let layout = layout(app)?;
    let state = &app.view.text;
    let cursor = state.text.cursor();
    let line = match layout.placed.get(cursor) {
        Some(p) => p.line,
        None => layout.lines.len().checked_sub(1)?,
    };
    // Lines follow downwards; columns leftwards.
    let next = matches!(key, Key::Down | Key::Left);
    let target = if next { line.checked_add(1).filter(|l| *l < layout.lines.len()) } else { line.checked_sub(1) };
    let Some(to) = target.and_then(|l| layout.lines.get(l)) else { return Some(if next { state.text.len() } else { 0 }) };
    let (x, y) = layout.caret(cursor);
    Some(match state.direction {
        WritingDirection::Ttb => layout.hit(to.baseline, y),
        // Half a line above the line's lower edge, where `hit` finds that line.
        WritingDirection::Ltr | WritingDirection::Rtl => layout.hit(x, to.baseline + layout.ascender - 0.5 * layout.line_height),
    })
}

/// A spacing or kerning shortcut in text mode. A spacing key acts on the current glyph in its shown layer; it is off
/// in locked kerning mode. A kerning key acts on the pair on the current glyph's left (`…Left`) or right (`…Right`)
/// as the text shows it, so in right-to-left text the left pair is the current glyph and the one after it; it is off
/// in no-kerning mode. Top-to-bottom text has no shortcuts (`docs/spec/08`, question 2).
fn shortcut(app: &mut FontCraftApp, id: &'static str) -> Result<(), String> {
    let state = &app.view.text;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let kerning = id.starts_with("kerning.");
    match (kerning, state.kerning, state.direction) {
        (_, _, WritingDirection::Ttb) => return Err("The spacing and kerning keys do not work in top-to-bottom text".into()),
        (true, KerningMode::None, _) => return Err("The kerning keys are off in the no-kerning mode".into()),
        (false, KerningMode::Locked, _) => return Err("The spacing keys are off in the locked kerning mode".into()),
        _ => {}
    }
    let master = crate::shell::masters::current_master(font, &app.view.layers).map(|m| m.id.clone()).ok_or("the font has no master")?;
    let params = if kerning {
        let current = state.edited_index().ok_or("the edit view has no current glyph")?;
        let before = current.checked_sub(1).and_then(|i| state.item_glyph(i));
        let at = state.item_glyph(current);
        let next = current.checked_add(1).and_then(|i| state.item_glyph(i));
        let rtl = state.direction == WritingDirection::Rtl;
        let (left, right) = if id.contains("Left") != rtl { (before, at) } else { (at, next) };
        let (Some(left), Some(right)) = (left, right) else {
            return Err("A kerning key needs a glyph on each side of the pair".into());
        };
        json!({"left": left, "right": right, "master": master, "direction": if rtl { "rtl" } else { "ltr" }})
    } else {
        let glyph = state.current().ok_or("the edit view has no current glyph")?;
        let layer = font.glyph(glyph).and_then(|g| crate::shell::masters::shown_layer(font, &app.view.layers, g)).map_or(master, |l| l.id.clone());
        json!({"glyphs": [glyph], "layer": layer})
    };
    app.apply_msg(Msg::run(id, params))
}

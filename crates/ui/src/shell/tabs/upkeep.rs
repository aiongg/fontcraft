//! What keeps the tabs in step with the document: tabs opened from its display strings when it opens, the open tabs
//! written back into them before a save (`text.displayStrings`), and glyph names that a command renamed or removed
//! (`tabs.windowHoldsFontViewPlusEditTabs`).
//!
//! A tab is written as what it shows: its text while the canvas shows the text, else the one glyph it shows. A tab
//! that a command left without glyphs closes and is not kept for Reopen Last Closed Tab; a closed tab it left without
//! glyphs is forgotten.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_engine::model::{Font, param_flag};
use fontcraft_textlayout::{Charmap, Item, MAX_ITEMS, Text, parse_prefix};
use serde_json::Value;

use super::{EditTab, MAX_TABS, TabState, first_glyph, parked_glyph, take_live};
use crate::app::{FontCraftApp, ViewState};
use crate::canvas::text::TabText;

/// The tab texts for a document's display strings, at most `room` of them, and a warning for each string that was
/// cut to the tab item limit or could not be read, and for the strings past `room`, which are not read.
pub fn tabs_from_display_strings(font: &Font, strings: &[String], room: usize) -> (Vec<TabText>, Vec<String>) {
    let map = Charmap::new(font);
    let mut texts = Vec::new();
    let mut warnings = Vec::new();
    for (number, s) in (1usize..).zip(strings.iter().take(room)) {
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
        if let Ok(text) = TabText::of(items) {
            texts.push(text);
        }
    }
    if strings.len() > room {
        warnings.push(format!("the document has {} display strings; only the first {room} open as tabs", strings.len()));
    }
    (texts, warnings)
}

/// Opens one Edit View tab per display string of the active document, after the open tabs and up to [`MAX_TABS`];
/// the showing tab, or Font View, stays showing. Returns the warnings, which also go to the log.
pub(crate) fn open_display_strings(app: &mut FontCraftApp) -> Vec<String> {
    let room = MAX_TABS.saturating_sub(app.view.tabs.tabs.len());
    let (texts, warnings) = match app.session.font() {
        Ok(font) => tabs_from_display_strings(font, &font.display_strings, room),
        Err(_) => return Vec::new(),
    };
    // The tabs draw in the master Font View draws in.
    let master = app.view.layers.master.clone();
    let opened = !texts.is_empty();
    for text in texts {
        let glyph = first_glyph(&text);
        let tabs = &mut app.view.tabs;
        let id = tabs.next_id;
        tabs.next_id = tabs.next_id.saturating_add(1);
        tabs.tabs.push(EditTab { id, state: TabState { glyph, text, master: master.clone(), ..TabState::default() } });
    }
    if opened {
        app.view_changed();
    }
    for w in &warnings {
        log::warn!("{w}");
    }
    warnings
}

/// The display strings for the active document's open tabs, in strip order; none when the font's
/// `Write DisplayStrings` parameter is off.
pub fn display_strings(font: &Font, view: &ViewState) -> Vec<String> {
    if font.info.params.get("Write DisplayStrings").and_then(param_flag) == Some(false) {
        return Vec::new();
    }
    view.tabs
        .tabs
        .iter()
        .map(|tab| {
            if view.tabs.live == Some(tab.id) {
                let shown = crate::canvas::tools::edited_glyph(font, view);
                written(font, &view.text.text, shown)
            } else {
                let shown = parked_glyph(Some(font), &tab.state);
                written(font, &tab.state.text.text, shown.as_deref())
            }
        })
        .collect()
}

/// The display string of a tab with `text` that shows `shown`: the text while the tab shows it, else `shown` alone.
fn written(font: &Font, text: &Text, shown: Option<&str>) -> String {
    let Some(glyph) = shown.filter(|g| !shows_text(text, g)) else { return text.to_display_string(font) };
    let mut alone = Text::new();
    match alone.replace(0..0, vec![crate::canvas::text::commands::glyph_item(font, glyph)]) {
        Ok(()) => alone.to_display_string(font),
        Err(_) => text.to_display_string(font),
    }
}

/// Whether a tab with `text` shows the text around `shown` rather than `shown` alone (as `canvas::text::shown_glyphs`
/// decides).
fn shows_text(text: &Text, shown: &str) -> bool {
    text.current_glyph() == Some(shown)
        && text.items().iter().any(|item| match item {
            Item::Glyph { name, .. } => &**name == shown,
            Item::Placeholder => true,
            Item::Missing(_) | Item::LineBreak => false,
        })
}

/// Writes the active document's open tabs into its display strings, before a save. It makes no undo step and does
/// not mark the document modified.
pub(crate) fn store_display_strings(app: &mut FontCraftApp) {
    let Ok(font) = app.session.font() else { return };
    let strings = display_strings(font, &app.view);
    if let Err(e) = app.session.set_display_strings(strings) {
        log::warn!("cannot store the tabs' texts in the document: {e}");
    }
}

/// The glyph renames in the reply of command `id` run with `params`, old name first.
pub fn renames(id: &str, params: &Value, reply: &Value) -> Vec<(String, String)> {
    match id {
        "glyph.rename" => {
            let (Some(old), Some(new)) = (params.get("glyph").and_then(Value::as_str), reply.get("glyph").and_then(Value::as_str)) else {
                return Vec::new();
            };
            vec![(old.to_string(), new.to_string())]
        }
        "edit.findAndReplace" | "glyph.updateGlyphInfo" => reply
            .get("renamed")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| Some((r.get("from")?.as_str()?.to_string(), r.get("to")?.as_str()?.to_string())))
            .collect(),
        _ => Vec::new(),
    }
}

/// After engine command `id` ran with `params` and returned `reply`: renamed glyphs keep their places in every tab of
/// the active document and in Font View's selection, and removed glyphs leave its tabs. `before` is the active
/// document's id and font before the command; `switched` says whether another document's tabs show now, whose tabs
/// were put away unchanged.
pub(crate) fn after_command(app: &mut FontCraftApp, id: &str, params: &Value, reply: &Value, before: Option<(u64, Arc<Font>)>, switched: bool) {
    let Some((doc, font)) = app.session.active().map(|d| (d.id, Arc::clone(&d.font))) else { return };
    let old = match before {
        Some((was, old)) if was == doc && !switched && !Arc::ptr_eq(&old, &font) => old,
        _ => return,
    };
    for (from, to) in renames(id, params, reply) {
        if from != to {
            rename(app, &from, &to);
        }
    }
    // Glyph names a tab holds that the font did not have before (a display string may name one) stay.
    let gone: HashSet<&str> = old.glyphs.iter().map(|g| g.name.as_str()).filter(|name| font.glyph(name).is_none()).collect();
    if !gone.is_empty() {
        remove(app, &|name: &str| gone.contains(name));
    }
}

/// Points the tabs and Font View's selection at glyph `new` where they named `old`.
fn rename(app: &mut FontCraftApp, old: &str, new: &str) {
    let rename_name = |name: &mut String| {
        if name == old {
            *name = new.to_string();
        }
    };
    let view = &mut app.view;
    view.font_view.selected.iter_mut().for_each(rename_name);
    view.font_view.anchor.iter_mut().for_each(rename_name);
    let live = view.tabs.live;
    if live.is_some() {
        view.glyph.iter_mut().for_each(rename_name);
        rename_in_text(&mut view.text.text, old, new);
        view.text.touch();
        if let Some(layer) = view.layers.layers.remove(old) {
            view.layers.layers.insert(new.to_string(), layer);
        }
        app.tools.rename_glyph(old, new);
    }
    let tabs = &mut app.view.tabs;
    let parked = tabs.tabs.iter_mut().filter(|t| Some(t.id) != live).chain(tabs.closed.iter_mut().map(|(_, t)| t));
    for tab in parked {
        let state = &mut tab.state;
        state.glyph.iter_mut().for_each(rename_name);
        rename_in_text(&mut state.text.text, old, new);
        if let Some(layer) = state.layers.remove(old) {
            state.layers.insert(new.to_string(), layer);
        }
        state.selection.rename_glyph(old, new);
    }
    app.view_changed();
}

/// Drops the glyphs `gone` names from every tab; a tab left without glyphs closes and is forgotten (module doc).
/// While the showing tab closes this way, Font View shows.
fn remove(app: &mut FontCraftApp, gone: &dyn Fn(&str) -> bool) {
    let live = app.view.tabs.live;
    let mut emptied = Vec::new();
    let mut changed = false;
    if let Some(id) = live {
        let view = &mut app.view;
        if let Some(left) = leave(&mut view.text.text, &mut view.glyph, gone) {
            changed = true;
            view.text.touch();
            if left {
                emptied.push(id);
            }
        }
    }
    let tabs = &mut app.view.tabs;
    for tab in tabs.tabs.iter_mut().filter(|t| Some(t.id) != live) {
        if let Some(left) = leave(&mut tab.state.text.text, &mut tab.state.glyph, gone) {
            changed = true;
            if left {
                emptied.push(tab.id);
            }
        }
    }
    let before = tabs.closed.len();
    tabs.closed.retain_mut(|(_, tab)| leave(&mut tab.state.text.text, &mut tab.state.glyph, gone) != Some(true));
    changed |= tabs.closed.len() != before;
    for id in emptied {
        forget(app, id);
    }
    if changed {
        app.view_changed();
    }
}

/// Drops the glyphs `gone` names from a tab's `text` and `glyph`. `None` when the tab named none of them; else
/// whether the tab is left without glyphs.
fn leave(text: &mut Text, glyph: &mut Option<String>, gone: &dyn Fn(&str) -> bool) -> Option<bool> {
    let dropped = remove_from_text(text, gone);
    let glyph_gone = glyph.as_deref().is_some_and(gone);
    if glyph_gone {
        *glyph = text.current_glyph().map(str::to_string);
    }
    (dropped || glyph_gone).then_some(text.is_empty())
}

/// Removes tab `id` without keeping it for Reopen Last Closed Tab. While it was showing, Font View shows, in the
/// master the tab drew in.
fn forget(app: &mut FontCraftApp, id: u64) {
    let Some(at) = app.view.tabs.index(id) else { return };
    if app.view.tabs.live == Some(id) {
        let state = take_live(app);
        app.view.tabs.live = None;
        app.view.layers.master = state.master;
    }
    let tabs = &mut app.view.tabs;
    if at < tabs.tabs.len() {
        tabs.tabs.remove(at);
    }
    if tabs.active == Some(id) {
        tabs.active = None;
    }
}

/// Renames glyph `old` to `new` in the items and the current glyph of `text`.
fn rename_in_text(text: &mut Text, old: &str, new: &str) {
    let named = |item: &Item| matches!(item, Item::Glyph { name, .. } if &**name == old);
    if !text.items().iter().any(named) && text.current_glyph() != Some(old) {
        return;
    }
    let current = text.current_glyph().map(|c| if c == old { new.to_string() } else { c.to_string() });
    rebuild(
        text,
        |item| {
            Some(match item {
                Item::Glyph { name, typed } if &**name == old => Item::Glyph { name: Arc::from(new), typed: *typed },
                other => other.clone(),
            })
        },
        current.as_deref(),
    );
}

/// Drops the glyph items of `text` that `gone` names; a current glyph it names gives way to the glyph next to the
/// cursor. Returns whether anything was dropped.
fn remove_from_text(text: &mut Text, gone: &dyn Fn(&str) -> bool) -> bool {
    let named = |item: &Item| matches!(item, Item::Glyph { name, .. } if gone(name));
    let current_gone = text.current_glyph().is_some_and(gone);
    if !text.items().iter().any(named) && !current_gone {
        return false;
    }
    let current = if current_gone { None } else { text.current_glyph().map(str::to_string) };
    rebuild(text, |item| (!named(item)).then(|| item.clone()), current.as_deref());
    true
}

/// Passes every item of `text` through `f`, dropping those it returns `None` for, and makes `current` the current
/// glyph. The cursor and the selection keep their places among the items kept.
fn rebuild(text: &mut Text, mut f: impl FnMut(&Item) -> Option<Item>, current: Option<&str>) {
    let selection = text.selection();
    let cursor = text.cursor();
    let anchor = if cursor == selection.start { selection.end } else { selection.start };
    let (mut new_cursor, mut new_anchor) = (0, 0);
    let mut items = Vec::with_capacity(text.len());
    for (i, item) in text.items().iter().enumerate() {
        if let Some(kept) = f(item) {
            items.push(kept);
            new_cursor += usize::from(i < cursor);
            new_anchor += usize::from(i < anchor);
        }
    }
    let len = text.len();
    // `f` returns at most one item per item, so the text cannot outgrow its limit.
    if text.replace(0..len, items).is_ok() {
        text.set_current_glyph(current);
        text.select(new_anchor, new_cursor);
    }
}

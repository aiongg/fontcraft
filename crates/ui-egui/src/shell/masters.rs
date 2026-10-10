//! The active layer, its edited side and the master a tab draws in: `layers.select` (a click on a
//! row of the Layers section), `path.editBackground` (Cmd+B) and `view.selectMaster` (Cmd+1 to
//! Cmd+9), which in Font View picks the master the cells draw.
//!
//! A tab draws every glyph in its master; `layers.select` shows single glyphs in another layer
//! (another master's, or a backup). The edited glyph's shown layer is the active layer: the
//! canvas draws it, the tools edit it and every command the shell runs targets it. The metric
//! lines and zones are those of its master.
//!
//! `path.editBackground` makes the active layer's background the edited side: the canvas draws
//! it as the outline, the tools and the Palette work on it, and every command the shell runs
//! gets `side: "background"`. The side belongs to the tab, so it stays when another glyph, layer
//! or master is edited.

use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::state::MAX_TAB_GLYPHS;

pub const COMMANDS: &[UiCommand] = &[
    (
        "layers.select",
        "Select Layer",
        &[],
        "",
        "{tab?: id, glyphs?: [names] (default: the edited glyph), layer: id, extend?: \"range\" | \"toggle\"} make the layer the one the tab shows for these glyphs; for the edited glyph it becomes the active layer, which the canvas draws and edits. A master layer switches only these glyphs to that master. `extend` changes the Layers section's row selection of the edited glyph instead, as Shift+click (range from the active layer) and Cmd+click (add or remove one row) do → {tab, glyphs, layer (the active layer), master, selection: [ids]}",
    ),
    (
        "path.editBackground",
        "Edit Background",
        &["Path"],
        "Cmd+B",
        "{tab?: id, on?: bool} edit the background of the active layer (true) or its foreground (false); omitted: switch to the other side. The selection is cleared. While the background is edited the canvas is drawn a little darker → {tab, side: \"foreground\" | \"background\"}",
    ),
    (
        "view.selectMaster",
        "Select Master",
        &[],
        "Cmd+1",
        "{tab?: id, master: index (0 is the first master) | id | name} draw every glyph of the tab in this master; while Font View shows (and no tab is named), the master Font View's cells draw. Cmd+1 to Cmd+9 pick the first nine → {tab (null in Font View), master (id), name, layer (the active layer)}",
    ),
];

/// How `layers.select` extends the row selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extend {
    Range,
    Toggle,
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id == "view.selectMaster" && app.state.tab().is_none() && args.get("tab").is_none() {
        return select_font_view_master(app, args);
    }
    let index = match args.index("tab")? {
        Some(tab) => app.state.tab_index(tab).ok_or_else(|| args.err(format!("no tab has id {tab}")))?,
        None => app.state.active_tab.ok_or_else(|| args.err("no Edit View tab is showing (open a glyph first)"))?,
    };
    match id {
        "layers.select" => select_layer(app, args, index),
        "path.editBackground" => edit_background(app, args, index),
        "view.selectMaster" => select_master(app, args, index),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn select_layer(app: &mut FontcraftApp, args: &Args, index: usize) -> Result<Value, String> {
    let layer = args.string("layer")?.ok_or_else(|| args.err("`layer` (a layer id, see layers.list) is required"))?.to_string();
    let extend = match args.string("extend")? {
        None => None,
        Some("range") => Some(Extend::Range),
        Some("toggle") => Some(Extend::Toggle),
        Some(other) => return Err(args.err(format!("`extend` must be \"range\" or \"toggle\", got \"{other}\""))),
    };
    let edited = app.state.tabs.get(index).and_then(|t| t.edited_glyph()).ok_or_else(|| args.err("the tab edits no glyph"))?.to_string();
    // The rows in the order the Layers section lists them, for a range.
    let rows: Vec<String> = match extend {
        Some(Extend::Range) => crate::panels::layers::shown_rows(app, Some(index), &edited).into_iter().map(|r| r.id).collect(),
        _ => Vec::new(),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?;
    let glyphs = args.strings("glyphs", MAX_TAB_GLYPHS)?.unwrap_or_else(|| vec![edited.clone()]);
    if glyphs.is_empty() {
        return Err(args.err("`glyphs` is empty"));
    }
    for name in &glyphs {
        if !tab.glyphs.contains(name) {
            return Err(args.err(format!("`{name}` is not in the tab")));
        }
        let glyph = font.glyph(name).ok_or_else(|| args.err(format!("there is no glyph `{name}`")))?;
        if !glyph.layers.iter().any(|l| l.id == layer) {
            return Err(args.err(format!("glyph `{name}` has no layer `{layer}`")));
        }
    }
    if extend.is_some() && glyphs != [edited.as_str()] {
        return Err(args.err("`extend` works on the rows of the edited glyph only"));
    }
    let mut next = tab.clone();
    match extend {
        None => {
            let tab_master = view::tab_master(font, tab).map(|m| m.id.clone());
            for name in &glyphs {
                let Some(l) = font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == layer)) else { continue };
                if l.is_master_layer() && tab_master.as_deref() == Some(l.master.as_str()) {
                    next.layers.remove(name);
                } else {
                    next.layers.insert(name.clone(), layer.clone());
                }
            }
            if glyphs.contains(&edited) {
                next.layer_selection = vec![layer.clone()];
            }
        }
        Some(Extend::Toggle) => {
            let active = view::active_layer(font, tab).map(|l| l.id.clone());
            if next.layer_selection.is_empty() {
                next.layer_selection.extend(active);
            }
            match next.layer_selection.iter().position(|l| *l == layer) {
                Some(at) => {
                    next.layer_selection.remove(at);
                }
                None => next.layer_selection.push(layer.clone()),
            }
        }
        Some(Extend::Range) => {
            let anchor = view::active_layer(font, tab).and_then(|a| rows.iter().position(|i| *i == a.id));
            let to = rows.iter().position(|i| *i == layer).ok_or_else(|| args.err(format!("layer `{layer}` is not listed in the Layers section")))?;
            let from = anchor.unwrap_or(to);
            let (lo, hi) = (from.min(to), from.max(to));
            next.layer_selection = rows.get(lo..=hi).unwrap_or_default().to_vec();
        }
    }
    let reply = {
        let active = view::active_layer(font, &next);
        let master = view::active_master(font, &next).map(|m| m.id.clone());
        json!({"tab": next.id, "glyphs": glyphs, "layer": active.map(|l| l.id.clone()), "master": master, "selection": next.layer_selection})
    };
    if let Some(tab) = app.state.tabs.get_mut(index) {
        *tab = next;
    }
    Ok(reply)
}

fn edit_background(app: &mut FontcraftApp, args: &Args, index: usize) -> Result<Value, String> {
    let tab = app.state.tabs.get_mut(index).ok_or_else(|| args.err("no such tab"))?;
    let on = args.toggle(tab.edit_background)?;
    if on != tab.edit_background {
        tab.edit_background = on;
        tab.selection.clear();
        tab.focus = None;
    }
    Ok(json!({"tab": tab.id, "side": tab.side().as_str()}))
}

fn select_master(app: &mut FontcraftApp, args: &Args, index: usize) -> Result<Value, String> {
    let (id, name) = find_master(app, args)?;
    let tab = app.state.tabs.get_mut(index).ok_or_else(|| args.err("no such tab"))?;
    tab.master = Some(id.clone());
    tab.layers.clear();
    tab.layer_selection.clear();
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?;
    let layer = view::active_layer(font, tab).map(|l| l.id.clone());
    Ok(json!({"tab": tab.id, "master": id, "name": name, "layer": layer}))
}

/// `view.selectMaster` while Font View shows: the master its cells draw
/// (`fontView.cell.followsMaster`).
fn select_font_view_master(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let (id, name) = find_master(app, args)?;
    app.state.font_view.master = Some(id.clone());
    Ok(json!({"tab": Value::Null, "master": id, "name": name}))
}

/// The id and name of the master `master` names.
fn find_master(app: &FontcraftApp, args: &Args) -> Result<(String, String), String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let given = args.get("master").ok_or_else(|| args.err("`master` (an index, id or name) is required"))?;
    let master = match given {
        Value::Number(n) => {
            let i = n.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| args.err("`master` must be an index of 0 or more"))?;
            font.masters.get(i).ok_or_else(|| args.err(format!("the font has {} masters; there is no master {i}", font.masters.len())))?
        }
        Value::String(s) => font
            .masters
            .iter()
            .find(|m| m.id == *s)
            .or_else(|| font.masters.iter().find(|m| m.name == *s))
            .ok_or_else(|| args.err(format!("no master has the id or name `{s}`")))?,
        _ => return Err(args.err("`master` must be an index, an id or a name")),
    };
    Ok((master.id.clone(), master.name.clone()))
}

//! The master the edit view draws in, the active layer and the Layers panel's own state: `layers.select` (a click
//! on a row of the Layers panel), `view.selectMaster` (a master icon), the panel's two filters and
//! `layers.editSpecialLayerSettings`.
//!
//! Every glyph is drawn in the current master; `layers.select` shows single glyphs in another layer (another
//! master's, or a backup). The shown glyph's layer is the active layer: the canvas draws it and the tools edit it.
//! The metric lines and zones are those of its master.
//!
//! An item of the edit view's text can have its own layer (`edit.showAllMasters`). While such an item is the edited
//! one, its layer is the active layer, and the Layers panel and the master icons change that item alone.

use std::collections::{BTreeMap, HashMap, HashSet};

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::{Font, Glyph, Layer, Master};
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::models::layers::{self as model, Part};
use crate::app::{FontCraftApp, ViewState};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most glyphs one `layers.select` names.
const MAX_GLYPHS: usize = 4096;
/// Most picked layers whose master is kept.
const MAX_HOMES: usize = 100_000;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "layers.select",
        label: "Select Layer",
        menu: &[],
        shortcut: None,
        params: "{glyphs?: [names] (default: the glyphs the Layers panel lists: the text selection's, else the shown glyph), layer: id, extend?: \"range\" | \"toggle\"} show these glyphs in the layer; for the shown glyph it becomes the active layer, which the canvas draws and edits. A master layer switches only these glyphs to that master. Without `glyphs` the layer is a row of the Layers panel, and each further glyph takes the layer that row stands for; with Font View in front it only becomes the selected row. `extend` changes the Layers panel's row selection instead, as Shift+click (range from the active layer) and Cmd+click (add or remove one row) do",
    },
    UiCommand {
        id: "view.selectMaster",
        label: "Select Master",
        menu: &[],
        shortcut: None,
        params: "{master: index (0 is the first master) | id | name} draw every glyph in this master; layers picked for single glyphs and the row selection are dropped. A text item with its own layer keeps it, except the edited one, which shows this master",
    },
    UiCommand {
        id: "layers.onlyShowLayersFromCurrentMaster",
        label: "Only Show Layers from Current Master",
        menu: &[],
        shortcut: None,
        params: "{on?: bool} list only the layers of the active layer's master in the Layers panel (omitted: toggle)",
    },
    UiCommand {
        id: "layers.hideBackupLayers",
        label: "Hide Backup Layers",
        menu: &[],
        shortcut: None,
        params: "{on?: bool} leave backup layers out of the Layers panel (omitted: toggle)",
    },
    UiCommand {
        id: "layers.editSpecialLayerSettings",
        label: "Edit Special Layer Settings",
        menu: &[],
        shortcut: None,
        params: "{glyph?: name (default: the shown glyph), layer?: id (default: the active layer), part: \"intermediate\" | \"alternate\"} open the coordinate editor or the range editor of a layer, filled with its present values, as double-clicking a special layer's row in the Layers panel does",
    },
];

/// The Layers panel's and the master switcher's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayersView {
    /// The master every glyph is drawn in; `None` or an id the font lacks: the first master.
    pub master: Option<String>,
    /// The layer single glyphs are shown in instead of their layer of `master`, by glyph name.
    pub layers: BTreeMap<String, String>,
    /// The glyph and the own layer of the edited text item, when it has one; `canvas::text::sync_layer` keeps it.
    pub item: Option<(String, String)>,
    /// The master of each layer that was picked and is no master layer, by layer id: the layer a glyph shows once the
    /// picked one is deleted.
    pub homes: BTreeMap<String, String>,
    /// The selected rows of the Layers panel; empty selects the active layer's row.
    pub selection: Vec<String>,
    /// "Only Show Layers from Current Master".
    pub current_master_only: bool,
    /// "Hide Backup Layers".
    pub hide_backups: bool,
    /// The special layer whose editor waits to open.
    pub special: Option<SpecialEdit>,
    /// The row whose context menu is open.
    pub menu_row: Option<String>,
}

/// A request to edit a special layer's coordinates or ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecialEdit {
    pub glyph: String,
    pub layer: String,
    pub part: Part,
}

/// The master glyphs are drawn in: the picked one, else the first.
pub fn current_master<'f>(font: &'f Font, view: &LayersView) -> Option<&'f Master> {
    view.master.as_deref().and_then(|id| font.masters.iter().find(|m| m.id == id)).or_else(|| font.masters.first())
}

/// The id of [`current_master`]: the master the glyph panels, Font View and the Space Center draw in.
pub fn current_master_id(font: &Font, view: &LayersView) -> Option<String> {
    current_master(font, view).map(|m| m.id.clone())
}

/// The layer `glyph` is shown in: the edited text item's own, else [`base_layer`].
pub fn shown_layer<'f>(font: &'f Font, view: &LayersView, glyph: &'f Glyph) -> Option<&'f Layer> {
    if let Some((name, id)) = &view.item
        && *name == glyph.name
        && let Some(layer) = glyph.layers.iter().find(|l| l.id == *id)
    {
        return Some(layer);
    }
    base_layer(font, view, glyph)
}

/// The layer `glyph` is shown in where no text item has its own: the one picked for it, else its layer of the current
/// master.
pub fn base_layer<'f>(font: &'f Font, view: &LayersView, glyph: &'f Glyph) -> Option<&'f Layer> {
    if let Some(id) = view.layers.get(&glyph.name)
        && let Some(layer) = glyph.layers.iter().find(|l| l.id == *id)
    {
        return Some(layer);
    }
    glyph.layer(&current_master(font, view)?.id)
}

/// The master `layer` belongs to, else the current master.
pub fn layer_master<'f>(font: &'f Font, view: &LayersView, layer: &Layer) -> Option<&'f Master> {
    font.masters.iter().find(|m| m.id == layer.master).or_else(|| current_master(font, view))
}

/// The glyph the glyph panels show and their commands act on: in the edit view the glyph the canvas draws (the shown
/// one, else the font's first); with Font View in front its one selected glyph, and none while it has several or no
/// glyphs selected.
pub fn shown_glyph<'f>(font: &'f Font, view: &ViewState) -> Option<&'f Glyph> {
    if view.tabs.font_view_shows() {
        let [name] = view.font_view.selected.as_slice() else { return None };
        return font.glyph(name).map(|g| &**g);
    }
    view.glyph.as_deref().and_then(|name| font.glyph(name)).or_else(|| font.glyphs.first()).map(|g| &**g)
}

/// The glyphs the Layers panel lists the layers of: the selected ones in Font View, those of the text selection, else
/// [`shown_glyph`]. None with Font View in front and nothing selected.
pub fn panel_glyphs(font: &Font, view: &ViewState) -> Vec<String> {
    let mut seen = HashSet::new();
    let chosen = crate::app::targets::chosen_glyphs(font, view);
    let names: Vec<String> = chosen.into_iter().flat_map(|group| group.glyphs).filter(|name| seen.insert(name.clone())).collect();
    if names.is_empty() { shown_glyph(font, view).map(|g| g.name.clone()).into_iter().collect() } else { names }
}

/// The layer of [`shown_glyph`] the glyph panels show and their commands act on: the one the canvas shows, or with Font
/// View in front the glyph's layer of the current master, which its cell draws.
pub fn panel_layer<'f>(font: &'f Font, view: &ViewState) -> Option<&'f Layer> {
    let glyph = shown_glyph(font, view)?;
    if view.tabs.font_view_shows() {
        return glyph.layer(&current_master(font, &view.layers)?.id);
    }
    shown_layer(font, &view.layers, glyph)
}

/// The master of [`panel_layer`], else the current master.
pub fn panel_master<'f>(font: &'f Font, view: &ViewState) -> Option<&'f Master> {
    match panel_layer(font, view) {
        Some(layer) => layer_master(font, &view.layers, layer),
        None => current_master(font, &view.layers),
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = &command.params;
    match command.id.as_str() {
        "layers.select" => select_layer(app, params),
        "view.selectMaster" => select_master(app, params),
        "layers.onlyShowLayersFromCurrentMaster" => {
            let on = toggle(params, app.view.layers.current_master_only)?;
            app.view.layers.current_master_only = on;
            Ok(())
        }
        "layers.hideBackupLayers" => {
            let on = toggle(params, app.view.layers.hide_backups)?;
            app.view.layers.hide_backups = on;
            Ok(())
        }
        "layers.editSpecialLayerSettings" => edit_special(app, params),
        other => Err(format!("no layers command `{other}`")),
    }
}

/// `on` when given, else the opposite of `now`.
fn toggle(params: &Value, now: bool) -> Result<bool, String> {
    match params.get("on") {
        None | Some(Value::Null) => Ok(!now),
        Some(Value::Bool(on)) => Ok(*on),
        Some(_) => Err("`on` must be true or false".into()),
    }
}

fn string<'p>(params: &'p Value, key: &str) -> Result<Option<&'p str>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

/// How `layers.select` extends the row selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extend {
    Range,
    Toggle,
}

fn select_layer(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let layer = string(params, "layer")?.ok_or("`layer` (a layer id, see layers.list) is required")?.to_string();
    let extend = match string(params, "extend")? {
        None => None,
        Some("range") => Some(Extend::Range),
        Some("toggle") => Some(Extend::Toggle),
        Some(other) => return Err(format!("`extend` must be \"range\" or \"toggle\", got \"{other}\"")),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let font_view = app.view.tabs.font_view_shows();
    let panel = panel_glyphs(font, &app.view);
    let given: Option<Vec<String>> = match params.get("glyphs") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) if list.len() <= MAX_GLYPHS => {
            Some(list.iter().map(|g| g.as_str().map(str::to_string).ok_or("`glyphs` holds glyph names")).collect::<Result<_, _>>()?)
        }
        Some(Value::Array(_)) => return Err(format!("`glyphs` names more than {MAX_GLYPHS} glyphs")),
        Some(_) => return Err("`glyphs` must be a list of glyph names".into()),
    };
    if extend.is_some() && given.as_ref().is_some_and(|g| *g != panel) {
        return Err("`extend` works on the rows of the glyphs the Layers panel lists only".into());
    }
    // Each glyph with the layer it is to show.
    let picks: Vec<(String, String)> = match &given {
        Some(glyphs) if glyphs.is_empty() => return Err("`glyphs` is empty".into()),
        Some(glyphs) => glyphs.iter().map(|g| (g.clone(), layer.clone())).collect(),
        None if panel.len() <= 1 => panel.iter().map(|g| (g.clone(), layer.clone())).collect(),
        None => {
            let (glyphs, rows) = model::panel_rows(&mut app.session, &app.view).ok_or("no glyph is shown: give `glyphs`")?;
            let row = rows.iter().find(|r| r.id == layer).ok_or_else(|| format!("layer `{layer}` is not listed in the Layers panel"))?;
            glyphs.iter().enumerate().map(|(at, g)| (g.clone(), row.layer_in(at).to_string())).collect()
        }
    };
    if picks.is_empty() {
        return Err("no glyph is shown: give `glyphs`".into());
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    for (name, id) in &picks {
        let glyph = font.glyph(name).ok_or_else(|| format!("there is no glyph `{name}`"))?;
        if !glyph.layers.iter().any(|l| l.id == *id) {
            return Err(format!("glyph `{name}` has no layer `{id}`"));
        }
    }
    let mut next = app.view.layers.clone();
    // The layers items of the text with a layer of their own take.
    let mut own: Vec<(usize, String)> = Vec::new();
    match extend {
        // In Font View the cells draw the current master: a click picks the row the buttons act on.
        None if font_view && given.is_none() => next.selection = vec![layer],
        None => {
            let master = current_master(font, &app.view.layers).map(|m| m.id.clone());
            let edited = next.item.as_ref().filter(|_| !font_view).map(|(glyph, _)| glyph.clone());
            let shown = if font_view { None } else { shown_glyph(font, &app.view).map(|g| g.name.clone()) };
            let text = &app.view.text;
            // With a text selection, those of its items that have a layer of their own, and the glyphs of the rest.
            let mut items: HashMap<&str, Vec<usize>> = HashMap::new();
            let mut free: HashSet<&str> = HashSet::new();
            if given.is_none() && !crate::app::targets::chosen_glyphs(font, &app.view).is_empty() {
                for index in text.text.selection() {
                    match (text.item_glyph(index), text.text.layer(index)) {
                        (Some(glyph), Some(_)) => items.entry(glyph).or_default().push(index),
                        (Some(glyph), None) => {
                            free.insert(glyph);
                        }
                        (None, _) => {}
                    }
                }
            } else if let Some((index, glyph)) = text.edited_index().filter(|_| edited.is_some()).and_then(|i| Some((i, text.item_glyph(i)?))) {
                items.insert(glyph, vec![index]);
            }
            for (name, id) in &picks {
                let Some(l) = font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == *id)) else { continue };
                if !l.is_master_layer() && next.homes.len() < MAX_HOMES {
                    next.homes.insert(id.clone(), l.master.clone());
                }
                if let Some(indexes) = items.get(name.as_str()) {
                    own.extend(indexes.iter().map(|i| (*i, id.clone())));
                    if !free.contains(name.as_str()) {
                        continue;
                    }
                }
                if l.is_master_layer() && master.as_deref() == Some(l.master.as_str()) {
                    next.layers.remove(name);
                } else {
                    next.layers.insert(name.clone(), id.clone());
                }
            }
            if given.is_none() || shown.is_some_and(|shown| picks.iter().any(|(name, _)| *name == shown)) {
                next.selection = vec![layer];
            }
        }
        Some(Extend::Toggle) => {
            if next.selection.is_empty() {
                next.selection.extend(model::shown_rows(&mut app.session, &app.view).and_then(|(_, active)| active));
            }
            match next.selection.iter().position(|l| *l == layer) {
                Some(at) => {
                    next.selection.remove(at);
                }
                None => next.selection.push(layer),
            }
        }
        Some(Extend::Range) => {
            let (rows, active) = model::shown_rows(&mut app.session, &app.view).ok_or("no glyph is shown")?;
            let to = rows.iter().position(|r| r.id == layer).ok_or_else(|| format!("layer `{layer}` is not listed in the Layers panel"))?;
            let from = active.and_then(|a| rows.iter().position(|r| r.id == a)).unwrap_or(to);
            let (lo, hi) = (from.min(to), from.max(to));
            next.selection = rows.get(lo..=hi).unwrap_or_default().iter().map(|r| r.id.clone()).collect();
        }
    }
    app.view.layers = next;
    crate::canvas::text::set_item_layers(app, &own);
    Ok(())
}

/// Shows the master layer in place of a picked layer that is gone (`layers.delete`, an undo), for the glyphs and the text
/// items that showed it.
pub fn follow_deleted(app: &mut FontCraftApp) {
    if app.view.layers.homes.is_empty() {
        return;
    }
    let Ok(font) = app.session.font() else { return };
    let view = &app.view;
    let current = current_master(font, &view.layers).map(|m| m.id.as_str());
    // The master layer glyph `name` shows in place of `id`: `None` while it has `id`, else the master, or none when the
    // glyph has no layer of it.
    let gone = |name: &str, id: &str| -> Option<Option<String>> {
        let glyph = font.glyph(name)?;
        let home = view.layers.homes.get(id).filter(|_| !glyph.layers.iter().any(|l| l.id == id))?;
        Some(glyph.layer(home).map(|_| home.clone()))
    };
    let glyphs: Vec<(String, Option<String>)> = view
        .layers
        .layers
        .iter()
        .filter_map(|(name, id)| Some((name.clone(), gone(name, id)?.filter(|home| Some(home.as_str()) != current))))
        .collect();
    let text = &view.text;
    let items: Vec<(usize, Option<String>)> =
        (0..text.text.len()).filter_map(|index| Some((index, gone(text.item_glyph(index)?, text.text.layer(index)?)?))).collect();
    if glyphs.is_empty() && items.is_empty() {
        return;
    }
    for (name, home) in glyphs {
        match home {
            Some(home) => app.view.layers.layers.insert(name, home),
            None => app.view.layers.layers.remove(&name),
        };
    }
    for (index, home) in items {
        app.view.text.text.set_layer(index, home.as_deref());
    }
    app.view.text.touch();
    app.view_changed();
}

fn select_master(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let given = params.get("master").ok_or("`master` (an index, id or name) is required")?;
    let master = match given {
        Value::Number(n) => {
            let i = n.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`master` must be an index of 0 or more")?;
            font.masters.get(i).ok_or_else(|| format!("the font has {} masters; there is no master {i}", font.masters.len()))?
        }
        Value::String(s) => font
            .masters
            .iter()
            .find(|m| m.id == *s)
            .or_else(|| font.masters.iter().find(|m| m.name == *s))
            .ok_or_else(|| format!("no master has the id or name `{s}`"))?,
        _ => return Err("`master` must be an index, an id or a name".into()),
    };
    let id = master.id.clone();
    // The edited item keeps a layer of its own while the glyph has one in the master.
    // With Font View in front, the text item behind it is left alone.
    let own = match app.view.layers.item.as_ref().filter(|_| !app.view.tabs.font_view_shows()) {
        Some((glyph, _)) => Some(font.glyph(glyph).is_some_and(|g| g.layer(&id).is_some())),
        None => None,
    };
    let view = &mut app.view.layers;
    view.master = Some(id.clone());
    view.layers.clear();
    view.selection.clear();
    if let Some(has) = own {
        crate::canvas::text::set_item_layer(app, has.then_some(id.as_str()));
    }
    Ok(())
}

fn edit_special(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let part = match string(params, "part")? {
        Some("intermediate") => Part::Intermediate,
        Some("alternate") => Part::Alternate,
        Some(other) => return Err(format!("unknown part `{other}`: intermediate or alternate")),
        None => return Err("give `part`: intermediate or alternate".into()),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let glyph = match string(params, "glyph")? {
        Some(name) => font.glyph(name).ok_or_else(|| format!("there is no glyph `{name}`"))?,
        None => shown_glyph(font, &app.view).ok_or("no glyph is shown: give `glyph`")?,
    };
    let layer = match string(params, "layer")? {
        Some(id) => glyph.layers.iter().find(|l| l.id == id).ok_or_else(|| format!("glyph `{}` has no layer `{id}`", glyph.name))?,
        None => panel_layer(font, &app.view)
            .filter(|l| glyph.layers.iter().any(|own| own.id == l.id))
            .or_else(|| shown_layer(font, &app.view.layers, glyph))
            .ok_or_else(|| format!("glyph `{}` has no layer to edit: give `layer`", glyph.name))?,
    };
    if font.axes.is_empty() {
        return Err("the font has no axes; add one in Font Info › Axes first".into());
    }
    if part == Part::Intermediate && layer.is_master_layer() {
        return Err("a master layer cannot be an intermediate layer: add a layer under the master and make that one intermediate".into());
    }
    let edit = SpecialEdit { glyph: glyph.name.clone(), layer: layer.id.clone(), part };
    app.view.layers.special = Some(edit);
    Ok(())
}

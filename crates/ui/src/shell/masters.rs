//! The master the edit view draws in, the active layer and the Layers panel's own state: `layers.select` (a click
//! on a row of the Layers panel), `view.selectMaster` (a master icon), the panel's two filters and
//! `layers.editSpecialLayerSettings`.
//!
//! Every glyph is drawn in the current master; `layers.select` shows single glyphs in another layer (another
//! master's, or a backup). The shown glyph's layer is the active layer: the canvas draws it and the tools edit it.
//! The metric lines and zones are those of its master.

use std::collections::BTreeMap;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::{Font, Glyph, Layer, Master};
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::models::layers::{self as model, Part};
use crate::app::{FontCraftApp, ViewState};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most glyphs one `layers.select` names.
const MAX_GLYPHS: usize = 4096;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "layers.select",
        label: "Select Layer",
        menu: &[],
        shortcut: None,
        params: "{glyphs?: [names] (default: the shown glyph), layer: id, extend?: \"range\" | \"toggle\"} show these glyphs in the layer; for the shown glyph it becomes the active layer, which the canvas draws and edits. A master layer switches only these glyphs to that master. `extend` changes the Layers panel's row selection instead, as Shift+click (range from the active layer) and Cmd+click (add or remove one row) do",
    },
    UiCommand {
        id: "view.selectMaster",
        label: "Select Master",
        menu: &[],
        shortcut: None,
        params: "{master: index (0 is the first master) | id | name} draw every glyph in this master; layers picked for single glyphs and the row selection are dropped",
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
    /// The selected rows of the shown glyph; empty selects the active layer's row.
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

/// The layer `glyph` is shown in: the one picked for it, else its layer of the current master.
pub fn shown_layer<'f>(font: &'f Font, view: &LayersView, glyph: &'f Glyph) -> Option<&'f Layer> {
    if let Some(id) = view.layers.get(&glyph.name)
        && let Some(layer) = glyph.layers.iter().find(|l| l.id == *id)
    {
        return Some(layer);
    }
    glyph.layer(&current_master(font, view)?.id)
}

/// The glyph the glyph panels show: the selected one, else the font's first.
pub fn shown_glyph<'f>(font: &'f Font, view: &ViewState) -> Option<&'f Glyph> {
    view.glyph.as_deref().and_then(|name| font.glyph(name)).or_else(|| font.glyphs.first()).map(|g| &**g)
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
    let edited = shown_glyph(font, &app.view).ok_or("the font has no glyph")?.name.clone();
    let glyphs: Vec<String> = match params.get("glyphs") {
        None | Some(Value::Null) => vec![edited.clone()],
        Some(Value::Array(list)) if list.len() <= MAX_GLYPHS => {
            list.iter().map(|g| g.as_str().map(str::to_string).ok_or("`glyphs` holds glyph names")).collect::<Result<_, _>>()?
        }
        Some(Value::Array(_)) => return Err(format!("`glyphs` names more than {MAX_GLYPHS} glyphs")),
        Some(_) => return Err("`glyphs` must be a list of glyph names".into()),
    };
    if glyphs.is_empty() {
        return Err("`glyphs` is empty".into());
    }
    for name in &glyphs {
        let glyph = font.glyph(name).ok_or_else(|| format!("there is no glyph `{name}`"))?;
        if !glyph.layers.iter().any(|l| l.id == layer) {
            return Err(format!("glyph `{name}` has no layer `{layer}`"));
        }
    }
    if extend.is_some() && glyphs != [edited.as_str()] {
        return Err("`extend` works on the rows of the shown glyph only".into());
    }
    let mut next = app.view.layers.clone();
    let active = font.glyph(&edited).and_then(|g| shown_layer(font, &app.view.layers, g)).map(|l| l.id.clone());
    match extend {
        None => {
            let master = current_master(font, &app.view.layers).map(|m| m.id.clone());
            for name in &glyphs {
                let Some(l) = font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == layer)) else { continue };
                if l.is_master_layer() && master.as_deref() == Some(l.master.as_str()) {
                    next.layers.remove(name);
                } else {
                    next.layers.insert(name.clone(), layer.clone());
                }
            }
            if glyphs.contains(&edited) {
                next.selection = vec![layer];
            }
        }
        Some(Extend::Toggle) => {
            if next.selection.is_empty() {
                next.selection.extend(active);
            }
            match next.selection.iter().position(|l| *l == layer) {
                Some(at) => {
                    next.selection.remove(at);
                }
                None => next.selection.push(layer),
            }
        }
        Some(Extend::Range) => {
            let rows = model::shown_rows(&mut app.session, &app.view, &edited);
            let to = rows.iter().position(|r| r.id == layer).ok_or_else(|| format!("layer `{layer}` is not listed in the Layers panel"))?;
            let from = active.and_then(|a| rows.iter().position(|r| r.id == a)).unwrap_or(to);
            let (lo, hi) = (from.min(to), from.max(to));
            next.selection = rows.get(lo..=hi).unwrap_or_default().iter().map(|r| r.id.clone()).collect();
        }
    }
    app.view.layers = next;
    Ok(())
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
    let view = &mut app.view.layers;
    view.master = Some(id);
    view.layers.clear();
    view.selection.clear();
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
        None => shown_layer(font, &app.view.layers, glyph).ok_or_else(|| format!("glyph `{}` has no layer to edit: give `layer`", glyph.name))?,
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

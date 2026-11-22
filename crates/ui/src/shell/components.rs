//! Components in the edit view: moving to a component's base glyph or a corner's glyph, the composites of a glyph,
//! and the settings windows of smart glyphs and smart components.
//!
//! | Command | Reached from | Does |
//! |---|---|---|
//! | `component.editBaseGlyph` | the arrow button of the component Info box | edits the component's base glyph |
//! | `cornerComponent.editGlyph` | the arrow button of the corner and cap Info boxes | edits the corner or cap glyph |
//! | `glyph.showGlyphsUsingAsComponent` | the canvas context menu | selects in Font View every glyph with a component of the glyph |
//! | `smartGlyph.showSettings` | Glyph menu, the smart glyph's context menu | opens Smart Glyph Settings (`dialogs::smart_settings`) |
//! | `smartComponent.showSettings` | the Smart badge of the component Info box | opens Smart Component Settings |
//!
//! After Component from Selection the new glyph is edited ([`after_run`]).

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::attach::{Attachment, attachment};
use fontcraft_engine::model::corner::corner_hint;
use fontcraft_engine::model::{Font, Layer, LayerSide, Shape};
use serde_json::Value;

use super::commands::{Module, UiCommand};
use super::masters::shown_layer;
use super::params::{flag, quoted, string};
use crate::app::FontCraftApp;
use crate::app::models::components::{MAX_SMART_COMPONENTS, smart_model};
use crate::canvas::tools::{edited_glyph, scene};

/// Most composites `glyph.showGlyphsUsingAsComponent` selects.
const MAX_SHOWN: usize = 10_000;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "component.editBaseGlyph",
        label: "Edit Base Glyph",
        menu: &[],
        shortcut: None,
        params: "{component?: index, glyph?: name} edit a base glyph: `glyph` names it, else it is the base of component `component` of the edited layer, else of the only selected component",
    },
    UiCommand {
        id: "cornerComponent.editGlyph",
        label: "Edit Corner Glyph",
        menu: &[],
        shortcut: None,
        params: "{glyph?: name (default the edited glyph), layer?: id (default the edited layer), corner: index (in the layer's hint list)} edit the glyph of that corner or cap component",
    },
    UiCommand {
        id: "glyph.showGlyphsUsingAsComponent",
        label: "Show all glyphs that use this glyph as a component",
        menu: &[],
        shortcut: None,
        params: "{glyph?: name (default the edited glyph)} select in Font View every glyph that has a component of the glyph, in font order (glyph.listComposites); a glyph that no other glyph uses is an error",
    },
    UiCommand {
        id: "smartGlyph.showSettings",
        label: "Show Smart Glyph Settings",
        menu: &[],
        shortcut: None,
        params: "{glyph?: name (default the edited glyph), tab?: \"properties\" | \"layers\", layer?: id (the layer the Layers tab edits; default the first), open?: bool (false closes the window)} open the Smart Glyph Settings of the glyph: its properties, and for every layer the end of each property's range it draws",
    },
    UiCommand {
        id: "smartComponent.showSettings",
        label: "Smart Component Settings",
        menu: &[],
        shortcut: None,
        params: "{glyph?: name, layer?: id (default the edited layer when `glyph` is the edited glyph or not given, else the layer the glyph shows: the Layers panel's pick, else its layer of the current master), side?: \"foreground\" | \"background\", components?: [indices] (default the selected smart components of the edited layer; required with `glyph`), open?: bool (false closes the window)} open the settings of the smart components: one control per property of their smart glyphs",
    },
];

/// The tabs of Smart Glyph Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsTab {
    #[default]
    Properties,
    Layers,
}

/// What Smart Glyph Settings shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmartGlyphView {
    pub glyph: String,
    pub tab: SettingsTab,
    /// The layer the Layers tab edits; `None`: the glyph's first layer.
    pub layer: Option<String>,
}

/// The components Smart Component Settings acts on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmartTargets {
    pub glyph: String,
    pub layer: String,
    pub background: bool,
    /// Shape indices on the layer, ascending.
    pub components: Vec<usize>,
}

/// The view state of this module and of the component dialogs.
#[derive(Clone, Debug, Default)]
pub struct ComponentsView {
    /// The glyph picker of the component commands (`dialogs::glyph_picker`); `None` while closed.
    pub picker: Option<crate::dialogs::glyph_picker::PickerState>,
    pub smart_glyph: Option<SmartGlyphView>,
    pub smart_component: Option<SmartTargets>,
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "component.editBaseGlyph" => edit_base_glyph(app, p),
        "cornerComponent.editGlyph" => edit_corner_glyph(app, p),
        "glyph.showGlyphsUsingAsComponent" => show_users(app, p),
        "smartGlyph.showSettings" => show_smart_glyph(app, p),
        "smartComponent.showSettings" => show_smart_component(app, p),
        other => Err(format!("no component command `{other}`")),
    }
}

/// The index at `key`; `None` when absent or null.
fn index(p: &Value, key: &str) -> Result<Option<usize>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).map(Some).ok_or_else(|| format!("`{key}` must be an index")),
    }
}

/// The edited glyph and the layer the canvas shows.
fn edited(app: &FontCraftApp) -> Result<(String, String, LayerSide), String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let scene = scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref()).ok_or("no glyph is being edited")?;
    Ok((scene.glyph.clone(), scene.layer_id.clone(), scene.side))
}

/// The layer `layer` of `glyph`, else the edited one when `glyph` is the edited glyph, else the one it shows (the
/// Layers panel's pick, else its layer of the current master).
fn layer_of<'f>(app: &FontCraftApp, font: &'f Font, glyph: &str, layer: Option<&str>) -> Result<&'f Layer, String> {
    let g = font.glyph(glyph).ok_or_else(|| format!("no glyph is named `{}`", quoted(glyph)))?;
    if let Some(id) = layer {
        return g.layers.iter().find(|l| l.id == id).ok_or_else(|| format!("glyph `{glyph}` has no layer `{}`", quoted(id)));
    }
    let shown = edited(app).ok().filter(|(name, ..)| name == glyph).and_then(|(_, id, _)| g.layers.iter().find(|l| l.id == id));
    match shown {
        Some(l) => Ok(l),
        None => shown_layer(font, &app.view.layers, g).ok_or_else(|| format!("glyph `{glyph}` has no master layer")),
    }
}

/// Edits `glyph`. The selection of the glyph left goes with it.
fn edit(app: &mut FontCraftApp, glyph: String) {
    app.view.glyph = Some(glyph);
}

fn edit_base_glyph(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let base = match string(p, "glyph")? {
        Some(name) => name.to_string(),
        None => {
            let (glyph, layer, _) = edited(app)?;
            let layer = layer_of(app, font, &glyph, Some(&layer))?;
            let component = |i: usize| match layer.shapes.get(i) {
                Some(Shape::Component(c)) => Some(c.glyph.clone()),
                _ => None,
            };
            match index(p, "component")? {
                Some(i) => component(i).ok_or_else(|| format!("shape {i} is not a component of the edited layer"))?,
                None => {
                    let selected: Vec<String> = app.tools.selection.shapes.iter().filter_map(|&i| component(i)).take(2).collect();
                    match selected.as_slice() {
                        [only] => only.clone(),
                        _ => return Err("select exactly one component, or give `component` or `glyph`".into()),
                    }
                }
            }
        }
    };
    if font.glyph(&base).is_none() {
        return Err(format!("no glyph is named `{}`", quoted(&base)));
    }
    edit(app, base);
    Ok(())
}

fn edit_corner_glyph(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let corner = index(p, "corner")?.ok_or("give `corner`, the corner's index in the layer's hint list")?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let glyph = match string(p, "glyph")? {
        Some(g) => g.to_string(),
        None => edited(app)?.0,
    };
    let layer = layer_of(app, font, &glyph, string(p, "layer")?)?;
    let hint = layer.info.hints.get(corner).ok_or_else(|| format!("the layer has no hint {corner}"))?;
    let name = match (corner_hint(hint), attachment(hint)) {
        (Ok(Some(c)), _) => c.name,
        (_, Ok(Some(Attachment::Cap(cap)))) => cap.name,
        (Err(e), _) => return Err(e),
        _ => return Err(format!("hint {corner} is not a corner or cap component")),
    };
    if font.glyph(&name).is_none() {
        return Err(format!("the glyph `{}` does not exist", quoted(&name)));
    }
    edit(app, name);
    Ok(())
}

fn show_users(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let glyph = match string(p, "glyph")? {
        Some(g) => g.to_string(),
        None => edited(app)?.0,
    };
    let listed = app.session.query("glyph.listComposites", &serde_json::json!({ "glyph": glyph })).map_err(|e| e.to_string())?;
    let glyphs: Vec<String> = listed
        .get("composites")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).take(MAX_SHOWN).collect())
        .unwrap_or_default();
    if glyphs.is_empty() {
        return Err(format!("no glyph uses `{}` as a component", quoted(&glyph)));
    }
    app.view.font_view.anchor = glyphs.first().cloned();
    app.view.font_view.selected = glyphs;
    Ok(())
}

fn show_smart_glyph(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    if flag(p, "open")? == Some(false) {
        app.view.components.smart_glyph = None;
        return Ok(());
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let name = match string(p, "glyph")? {
        Some(n) => n.to_string(),
        None => edited_glyph(font, &app.view).map(str::to_string).ok_or("no glyph is being edited: give `glyph`")?,
    };
    let g = font.glyph(&name).ok_or_else(|| format!("there is no glyph `{}`", quoted(&name)))?;
    let tab = match string(p, "tab")? {
        None => None,
        Some("properties") => Some(SettingsTab::Properties),
        Some("layers") => Some(SettingsTab::Layers),
        Some(other) => return Err(format!("`tab` must be \"properties\" or \"layers\", got \"{}\"", quoted(other))),
    };
    let previous = app.view.components.smart_glyph.clone().filter(|v| v.glyph == name).unwrap_or_default();
    let layer = match string(p, "layer")? {
        Some(l) => {
            Some(g.layers.iter().find(|x| x.id == l).map(|x| x.id.clone()).ok_or_else(|| format!("glyph `{name}` has no layer `{}`", quoted(l)))?)
        }
        // The layer picked before stays picked while the same glyph shows and still has it.
        None => previous.layer.filter(|l| g.layers.iter().any(|x| &x.id == l)),
    };
    app.view.components.smart_glyph = Some(SmartGlyphView { glyph: name, tab: tab.unwrap_or(previous.tab), layer });
    Ok(())
}

fn show_smart_component(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    if flag(p, "open")? == Some(false) {
        app.view.components.smart_component = None;
        return Ok(());
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let side = match string(p, "side")? {
        None => None,
        Some("foreground") => Some(LayerSide::Foreground),
        Some("background") => Some(LayerSide::Background),
        Some(other) => return Err(format!("`side` must be \"foreground\" or \"background\", got \"{}\"", quoted(other))),
    };
    let (glyph, layer, side, selected) = match string(p, "glyph")? {
        Some(name) => (name.to_string(), layer_of(app, font, name, string(p, "layer")?)?, side.unwrap_or_default(), None),
        None => {
            let (glyph, shown, edited_side) = edited(app)?;
            let layer = layer_of(app, font, &glyph, Some(string(p, "layer")?.unwrap_or(&shown)))?;
            let selected: Vec<usize> = app.tools.selection.shapes.iter().copied().collect();
            (glyph, layer, side.unwrap_or(edited_side), Some(selected))
        }
    };
    let shapes = layer.parts(side).shapes;
    let components: Vec<usize> = match (p.get("components").filter(|v| !v.is_null()), selected) {
        (Some(v), _) => {
            let list = v.as_array().ok_or("`components` must be a list of shape indices")?;
            if list.len() > MAX_SMART_COMPONENTS {
                return Err(format!("`components` holds at most {MAX_SMART_COMPONENTS} indices"));
            }
            let mut out = Vec::with_capacity(list.len());
            for item in list {
                let i = item.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`components` must be a list of shape indices")?;
                match shapes.get(i) {
                    None => return Err(format!("`components`: {i} does not resolve: the layer has {} shapes", shapes.len())),
                    Some(Shape::Component(c)) if font.glyph(&c.glyph).is_none_or(|g| g.info.parts.is_empty()) => {
                        return Err(format!("component {i} refers to `{}`, which is not a smart glyph", quoted(&c.glyph)));
                    }
                    Some(Shape::Component(_)) => out.push(i),
                    Some(_) => return Err(format!("shape {i} of glyph `{}` is not a component", quoted(&glyph))),
                }
            }
            out
        }
        (None, Some(selected)) => selected,
        (None, None) => return Err("give `components` with `glyph`".into()),
    };
    let model = smart_model(font, &glyph, layer, side, &components).ok_or("select a smart component, or give `components`")?;
    app.view.components.smart_component =
        Some(SmartTargets { glyph, layer: layer.id.clone(), background: side == LayerSide::Background, components: model.components });
    Ok(())
}

/// After Component from Selection succeeds on the edited glyph, the new glyph is edited, as the person would go on.
pub fn after_run(app: &mut FontCraftApp, id: &str, result: &Value) {
    if id != "glyph.componentFromSelection" {
        return;
    }
    let (Some(name), Some(glyph)) = (result.get("name").and_then(Value::as_str), result.get("glyph").and_then(Value::as_str)) else { return };
    if edited(app).is_ok_and(|(edited, ..)| edited == glyph) {
        edit(app, name.to_string());
        app.view_changed();
    }
}

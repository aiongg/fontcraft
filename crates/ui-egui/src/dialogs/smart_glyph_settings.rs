//! The Smart Glyph Settings window (`dialog.smartGlyphSettings`, `smartGlyph.showSettings`): the
//! properties of a smart glyph and the end of each property's range its layers draw, in two tabs.
//!
//! | Tab | Control | Command |
//! |---|---|---|
//! | Properties | name field of a property | `smartGlyph.setProperty {name}` |
//! | Properties | minimum and maximum fields | `smartGlyph.setProperty {min}` or `{max}` |
//! | Properties | minus button of a property | `smartGlyph.removeProperty` |
//! | Properties | plus button, bottom left | `smartGlyph.addProperty` with the first free name of [`NEW_NAME`] |
//! | Layers | sidebar of the glyph's layers | picks the layer the checkboxes edit (view state) |
//! | Layers | Minimum and Maximum checkboxes per property | `smartGlyph.setLayerPole` (`min`, `max`, or `none` when unticked) |
//!
//! The window stays open beside the Edit View and blocks nothing; it follows edits made elsewhere
//! because it reads the font each frame. A refused edit leaves the font as it was and puts the
//! message in the status bar.
//!
//! Rows: `smartGlyph.showSettings`, `dialog.smartGlyphSettings`.

use fontcraft_engine::cmd::smart::is_toggle;
use fontcraft_engine::model::{Font, Glyph, Layer, MAX_PART_PROPERTIES};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::{Pending, current_glyph, widgets};
use crate::shell::{Args, UiCommand};

pub const ID: &str = "smartGlyph.showSettings";

/// The name the plus button gives a new property; " 2", " 3"… follow when it is taken.
pub const NEW_NAME: &str = "Property";

/// Most layers the Layers tab lists.
const MAX_LISTED_LAYERS: usize = 1024;

pub const COMMANDS: &[UiCommand] = &[(
    ID,
    "Show Smart Glyph Settings",
    &[],
    "",
    "{glyph?: name (default the edited glyph, else the first glyph selected in Font View), tab?: \"properties\" | \"layers\", layer?: id (the layer the Layers tab edits; default the first layer), open?: bool (false closes the window)} opens the Smart Glyph Settings of the glyph: its properties, and for every layer the end of each property's range it draws. Any glyph may be shown: adding a property makes it a smart glyph → {open, glyph, tab, layer, properties: [{name, min, max, toggle}], layers: [{id, name, master, poles: [{property, pole: \"min\" | \"max\"}]}]}",
)];

/// The window's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsTab {
    #[default]
    Properties,
    Layers,
}

impl SettingsTab {
    fn as_str(self) -> &'static str {
        match self {
            SettingsTab::Properties => "properties",
            SettingsTab::Layers => "layers",
        }
    }
}

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SmartGlyphState {
    pub open: bool,
    pub glyph: String,
    pub tab: SettingsTab,
    /// The id of the layer the Layers tab edits; `None`: the glyph's first layer.
    pub layer: Option<String>,
}

/// The first of `Property`, `Property 2`, `Property 3`… that no property of `g` has.
pub fn new_property_name(g: &Glyph) -> String {
    let taken = |name: &str| g.info.parts.iter().any(|p| p.name == name);
    if !taken(NEW_NAME) {
        return NEW_NAME.to_string();
    }
    // A glyph has at most MAX_PART_PROPERTIES properties, so one of these is free.
    (2..=MAX_PART_PROPERTIES + 2)
        .map(|n| format!("{NEW_NAME} {n}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| format!("{NEW_NAME} {}", g.info.parts.len() + 2))
}

/// The pole `layer` holds for `property`: `Some(true)` the maximum, `Some(false)` the minimum.
fn pole_of(layer: &Layer, property: &str) -> Option<bool> {
    layer.info.attrs.smart_poles.iter().find(|p| p.property == property).map(|p| p.top)
}

/// The command a click on a layer's Minimum (`top` false) or Maximum (`top` true) checkbox runs:
/// ticking an end sets it, unticking the ticked end clears the pole.
pub fn pole_command(glyph: &str, layer: &str, property: &str, current: Option<bool>, top: bool) -> (&'static str, Value) {
    let pole = if current == Some(top) {
        "none"
    } else if top {
        "max"
    } else {
        "min"
    };
    ("smartGlyph.setLayerPole", json!({"glyph": glyph, "layer": layer, "property": property, "pole": pole}))
}

/// The name a layer shows: its master's name for a master layer, else its own name or id.
fn layer_label(font: &Font, l: &Layer) -> String {
    match font.master(&l.master) {
        Ok(m) if l.is_master_layer() => m.name.clone(),
        _ => l.info.name.clone().unwrap_or_else(|| l.id.clone()),
    }
}

/// The id of a field of property `index` on the Properties tab: `kind` is `name`, `min` or `max`.
pub fn field_id(kind: &str, index: usize) -> egui::Id {
    widgets::id(("smartGlyph.settings", kind, index))
}

/// The id of the plus button of the Properties tab.
pub fn add_button_id() -> egui::Id {
    widgets::id("smartGlyph.settings.add")
}

/// The id of the minus button of property `index`.
pub fn remove_button_id(index: usize) -> egui::Id {
    widgets::id(("smartGlyph.settings.remove", index))
}

fn reply(font: &Font, g: &Glyph, st: &SmartGlyphState) -> Value {
    let properties: Vec<Value> =
        g.info.parts.iter().map(|p| json!({"name": p.name, "min": p.bottom_value, "max": p.top_value, "toggle": is_toggle(p)})).collect();
    let layers: Vec<Value> = g
        .layers
        .iter()
        .take(MAX_LISTED_LAYERS)
        .map(|l| {
            let poles: Vec<Value> =
                l.info.attrs.smart_poles.iter().map(|p| json!({"property": p.property, "pole": if p.top { "max" } else { "min" }})).collect();
            json!({"id": l.id, "name": layer_label(font, l), "master": l.master, "poles": poles})
        })
        .collect();
    let layer = st.layer.clone().or_else(|| g.layers.first().map(|l| l.id.clone()));
    json!({"open": true, "glyph": g.name, "tab": st.tab.as_str(), "layer": layer, "properties": properties, "layers": layers})
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != ID {
        return Err(format!("unknown command `{id}`"));
    }
    if args.flag("open")? == Some(false) {
        app.state.panels.smart_glyph.open = false;
        return Ok(json!({"open": false}));
    }
    let name = match args.string("glyph")? {
        Some(n) => n.to_string(),
        None => current_glyph(app).ok_or_else(|| args.err("no glyph is edited or selected: give `glyph`"))?,
    };
    let tab = match args.string("tab")? {
        None => None,
        Some("properties") => Some(SettingsTab::Properties),
        Some("layers") => Some(SettingsTab::Layers),
        Some(other) => return Err(args.err(format!("`tab` must be \"properties\" or \"layers\", got \"{other}\""))),
    };
    let font = app.session.font().map_err(|e| args.err(e))?;
    let g = font.glyph(&name).ok_or_else(|| args.err(format!("there is no glyph `{name}`")))?;
    let previous = &app.state.panels.smart_glyph;
    let layer = match args.string("layer")? {
        Some(l) => {
            Some(g.layers.iter().find(|x| x.id == l).map(|x| x.id.clone()).ok_or_else(|| args.err(format!("glyph `{name}` has no layer `{l}`")))?)
        }
        // The layer picked before stays picked while the same glyph shows and still has it.
        None => previous.layer.clone().filter(|l| previous.glyph == name && g.layers.iter().any(|x| &x.id == l)),
    };
    let st = SmartGlyphState {
        open: true,
        glyph: name,
        tab: tab.unwrap_or(if previous.glyph == g.name { previous.tab } else { SettingsTab::Properties }),
        layer,
    };
    let out = reply(font, g, &st);
    app.state.panels.smart_glyph = st;
    Ok(out)
}

/// Draws the Properties tab.
fn properties(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending, g: &Glyph) {
    let tokens = &app.tokens;
    if g.info.parts.is_empty() {
        ui.label(egui::RichText::new("No properties: the plus button adds one.").color(tokens.text_dim));
    } else {
        egui::Grid::new("smartGlyph.settings.properties").num_columns(4).spacing([tokens.dialog_spacing, tokens.dialog_spacing / 2.0]).show(
            ui,
            |ui| {
                for title in ["Name", "Minimum", "Maximum"] {
                    ui.label(egui::RichText::new(title).strong());
                }
                ui.end_row();
                for (i, p) in g.info.parts.iter().enumerate().take(MAX_PART_PROPERTIES) {
                    if let Some(name) = widgets::text(ui, field_id("name", i), &p.name, tokens.dialog_field_width)
                        && name != p.name
                    {
                        pending.push(("smartGlyph.setProperty", json!({"glyph": g.name, "property": p.name, "name": name})));
                    }
                    for (kind, value) in [("min", p.bottom_value), ("max", p.top_value)] {
                        if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, field_id(kind, i), Some(value), tokens.field_width, false)
                            && n != value
                        {
                            let mut params = json!({"glyph": g.name, "property": p.name});
                            if let Some(m) = params.as_object_mut() {
                                m.insert(kind.to_string(), json!(n));
                            }
                            pending.push(("smartGlyph.setProperty", params));
                        }
                    }
                    if widgets::flat_button(ui, remove_button_id(i), "−", "Remove this property") {
                        pending.push(("smartGlyph.removeProperty", json!({"glyph": g.name, "property": p.name})));
                    }
                    ui.end_row();
                }
            },
        );
    }
    ui.add_space(tokens.dialog_spacing);
    let room = g.info.parts.len() < MAX_PART_PROPERTIES;
    if widgets::button(ui, add_button_id(), "+", "Add a property", room, false) {
        pending.push(("smartGlyph.addProperty", json!({"glyph": g.name, "name": new_property_name(g)})));
    }
}

/// Draws the Layers tab; returns the layer picked in the sidebar this frame.
fn layers(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending, font: &Font, g: &Glyph, picked: Option<&str>) -> Option<String> {
    let tokens = &app.tokens;
    let several_masters = font.masters.len() > 1;
    let current = picked.and_then(|id| g.layers.iter().find(|l| l.id == id)).or_else(|| g.layers.first());
    let mut pick = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(tokens.dialog_field_width * 1.2);
            egui::ScrollArea::vertical().id_salt("smartGlyph.settings.layers").max_height(tokens.list_row_height * 12.0).show(ui, |ui| {
                for l in g.layers.iter().take(MAX_LISTED_LAYERS) {
                    let mut caption = layer_label(font, l);
                    if several_masters && !l.is_master_layer() {
                        let master = font.master(&l.master).map_or(l.master.as_str(), |m| m.name.as_str());
                        caption = format!("{caption} ({master})");
                    }
                    let mut text = egui::RichText::new(caption);
                    if l.is_master_layer() {
                        text = text.strong();
                    }
                    let selected = current.is_some_and(|c| c.id == l.id);
                    if ui.selectable_label(selected, text).clicked() && !selected {
                        pick = Some(l.id.clone());
                    }
                }
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            let Some(layer) = current else {
                ui.label(egui::RichText::new("The glyph has no layers.").color(tokens.text_dim));
                return;
            };
            if g.info.parts.is_empty() {
                ui.label(egui::RichText::new("No properties: add one on the Properties tab.").color(tokens.text_dim));
                return;
            }
            egui::Grid::new("smartGlyph.settings.poles").num_columns(3).spacing([tokens.dialog_spacing, tokens.dialog_spacing / 2.0]).show(
                ui,
                |ui| {
                    for p in g.info.parts.iter().take(MAX_PART_PROPERTIES) {
                        let pole = pole_of(layer, &p.name);
                        ui.label(p.name.as_str());
                        for (top, caption, value) in [(false, "Minimum", p.bottom_value), (true, "Maximum", p.top_value)] {
                            let mut ticked = pole == Some(top);
                            let text = format!("{caption} ({})", widgets::format_number(value));
                            if ui.checkbox(&mut ticked, text).clicked() {
                                pending.push(pole_command(&g.name, &layer.id, &p.name, pole, top));
                            }
                        }
                        ui.end_row();
                    }
                },
            );
        });
    });
    pick
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.smart_glyph.open {
        return;
    }
    let st = app.state.panels.smart_glyph.clone();
    let mut open = true;
    let mut tab = st.tab;
    let mut picked = None;
    {
        let shown: &FontcraftApp = app;
        let tokens = &shown.tokens;
        egui::Window::new("Smart Glyph Settings")
            .id(egui::Id::new(ID))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
            .show(ui.ctx(), |ui| {
                let Some(font) = shown.session.font().ok() else { return };
                let Some(g) = font.glyph(&st.glyph) else {
                    ui.label(egui::RichText::new(format!("The glyph `{}` is gone.", st.glyph)).color(tokens.text_dim));
                    return;
                };
                ui.label(egui::RichText::new(g.name.as_str()).color(tokens.text_dim));
                ui.horizontal(|ui| {
                    for t in [SettingsTab::Properties, SettingsTab::Layers] {
                        let caption = match t {
                            SettingsTab::Properties => "Properties",
                            SettingsTab::Layers => "Layers",
                        };
                        if ui.selectable_label(tab == t, caption).clicked() {
                            tab = t;
                        }
                    }
                });
                ui.separator();
                match st.tab {
                    SettingsTab::Properties => properties(shown, ui, pending, g),
                    SettingsTab::Layers => picked = layers(shown, ui, pending, font, g, st.layer.as_deref()),
                }
            });
    }
    let state = &mut app.state.panels.smart_glyph;
    state.open = open;
    state.tab = tab;
    if picked.is_some() {
        state.layer = picked;
    }
}

//! Smart Glyph Settings (`dialog.smartGlyphSettings`) and Smart Component Settings (`dialog.smartComponentSettings`):
//! windows that stay open beside the canvas and write every change through its command at once, so each change is an
//! undo step. `smartGlyph.showSettings` and `smartComponent.showSettings` (`shell::components`) open them.
//!
//! | Window | Control | Command |
//! |---|---|---|
//! | Smart Glyph, Properties tab | name, minimum and maximum of a property | `smartGlyph.setProperty` |
//! | Smart Glyph, Properties tab | minus button of a property; plus button | `smartGlyph.removeProperty`; `smartGlyph.addProperty` with the first free name of `Property`, `Property 2`… |
//! | Smart Glyph, Layers tab | layer menu | the layer the checkboxes edit (view state) |
//! | Smart Glyph, Layers tab | Minimum and Maximum checkboxes per property | `smartGlyph.setLayerPole` (`min`, `max`, or `none` when unticked) |
//! | Smart Component | one control per property (`panels::smart_component`) | `smartComponent.setValue` |
//!
//! Both read the font again whenever it changes, so they follow edits made elsewhere.

use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::model::{Font, Glyph, Layer, LayerSide, MAX_PART_PROPERTIES};
use serde_json::json;

use crate::app::models::Stamp;
use crate::app::models::components::{SmartModel, smart_model};
use crate::app::{FontCraftApp, Msg};
use crate::panels::smart_component::controls;
use crate::panels::widgets;
use crate::shell::components::{SettingsTab, SmartGlyphView, SmartTargets};

craft::ui::words! {
    /// The words of the smart settings windows.
    pub struct SmartSettingsWords {
        glyph_title: "fontcraftSmartSettings.glyphTitle" = "Smart Glyph Settings";
        component_title: "fontcraftSmartSettings.componentTitle" = "Smart Component Settings";
        close: "fontcraftSmartSettings.close" = "Close";
        tab: "fontcraftSmartSettings.tab" = "Tab";
        properties: "fontcraftSmartSettings.properties" = "Properties";
        layers: "fontcraftSmartSettings.layers" = "Layers";
        name: "fontcraftSmartSettings.name" = "Name";
        minimum: "fontcraftSmartSettings.minimum" = "Minimum";
        maximum: "fontcraftSmartSettings.maximum" = "Maximum";
        remove: "fontcraftSmartSettings.remove" = "−";
        remove_tip: "fontcraftSmartSettings.removeTip" = "Remove the property";
        add: "fontcraftSmartSettings.add" = "+";
        add_tip: "fontcraftSmartSettings.addTip" = "Add a property";
        layer: "fontcraftSmartSettings.layer" = "Layer";
        no_properties: "fontcraftSmartSettings.noProperties" = "The glyph has no properties: add one on the Properties tab";
        missing: "fontcraftSmartSettings.missing" = "The glyph is gone";
    }
}

/// The name the plus button gives a new property; " 2", " 3"… follow when it is taken.
pub const NEW_NAME: &str = "Property";
/// Most layers the Layers tab lists.
const MAX_LISTED_LAYERS: usize = 1024;

/// One property of a smart glyph.
#[derive(Clone, Debug, Default, PartialEq)]
struct Property {
    name: String,
    min: f64,
    max: f64,
}

/// One layer of a smart glyph and the pole it holds for each property: `true` the maximum.
#[derive(Clone, Debug, Default, PartialEq)]
struct LayerPoles {
    id: String,
    label: String,
    poles: Vec<(String, bool)>,
}

/// What Smart Glyph Settings shows, read from the font.
#[derive(Clone, Debug, Default, PartialEq)]
struct SmartGlyphData {
    exists: bool,
    properties: Vec<Property>,
    layers: Vec<LayerPoles>,
    /// The name the plus button gives.
    new_name: String,
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

/// The command a click on a layer's Minimum (`top` false) or Maximum (`top` true) checkbox runs: ticking an end sets
/// it, unticking the ticked end clears the pole.
pub fn pole_command(glyph: &str, layer: &str, property: &str, current: Option<bool>, top: bool) -> Msg {
    let pole = match (current == Some(top), top) {
        (true, _) => "none",
        (false, true) => "max",
        (false, false) => "min",
    };
    Msg::run("smartGlyph.setLayerPole", json!({"glyph": glyph, "layer": layer, "property": property, "pole": pole}))
}

/// The name a layer shows: its master's name for a master layer, else its own name or id.
fn layer_label(font: &Font, l: &Layer) -> String {
    match font.master(&l.master) {
        Ok(m) if l.is_master_layer() => m.name.clone(),
        _ => l.info.name.clone().unwrap_or_else(|| l.id.clone()),
    }
}

fn read_glyph(font: &Font, name: &str) -> SmartGlyphData {
    let Some(g) = font.glyph(name) else { return SmartGlyphData::default() };
    SmartGlyphData {
        exists: true,
        properties: g.info.parts.iter().map(|p| Property { name: p.name.clone(), min: p.bottom_value, max: p.top_value }).collect(),
        layers: g
            .layers
            .iter()
            .take(MAX_LISTED_LAYERS)
            .map(|l| LayerPoles {
                id: l.id.clone(),
                label: layer_label(font, l),
                poles: l.info.attrs.smart_poles.iter().map(|p| (p.property.clone(), p.top)).collect(),
            })
            .collect(),
        new_name: new_property_name(g),
    }
}

/// Smart Glyph Settings on the window's dialog host.
struct SmartGlyphDialog {
    id: DialogId,
    view: SmartGlyphView,
    data: SmartGlyphData,
    read_at: Option<Stamp>,
    /// What the person changed: commands and the tab or layer picked.
    outbox: Vec<Msg>,
    picked: Option<SmartGlyphView>,
    open: bool,
}

impl StackDialog for SmartGlyphDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<SmartSettingsWords>();
        let SmartGlyphDialog { view, data, outbox, picked, open, .. } = self;
        let mut held = true;
        let title = format!("{} — {}", words.glyph_title.as_str(), view.glyph);
        let out = FormDialog::new(title, Bind::bound(&mut held)).opened(Bind::bound(open)).ok_label(words.close.clone()).show(ui, |ui, _, _| {
            if !data.exists {
                Text::new(words.missing.clone()).shade(Shade::Muted).show(ui);
                return true;
            }
            let tabs = [Entry::new(SettingsTab::Properties, words.properties.clone()), Entry::new(SettingsTab::Layers, words.layers.clone())];
            if let Some(tab) = Segmented::new(words.tab.clone(), Bind::given(view.tab), &tabs).show(ui).events.committed().filter(|t| **t != view.tab)
            {
                *picked = Some(SmartGlyphView { tab: *tab, ..view.clone() });
            }
            match view.tab {
                SettingsTab::Properties => properties_tab(ui, &words, &view.glyph, data, outbox),
                SettingsTab::Layers => {
                    if let Some(layer) = layers_tab(ui, &words, view, data, outbox) {
                        *picked = Some(SmartGlyphView { layer: Some(layer), ..view.clone() });
                    }
                }
            }
            true
        });
        match (out.accepted, out.closed) {
            (Some(_), _) => Some(DialogResult::Ok(())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

fn properties_tab(ui: &mut Ui, words: &SmartSettingsWords, glyph: &str, data: &SmartGlyphData, outbox: &mut Vec<Msg>) {
    for (i, p) in data.properties.iter().enumerate() {
        let _ = Row::new().show(ui, |ui, _| {
            let named = TextField::new(words.name.clone(), Bind::given(p.name.clone())).id_salt(("name", i)).show(ui);
            if let Some(name) = named.events.committed().filter(|n| n.trim() != p.name && !n.trim().is_empty()) {
                outbox.push(Msg::run("smartGlyph.setProperty", json!({"glyph": glyph, "property": p.name, "name": name.trim()})));
            }
            for (label, key, value) in [(&words.minimum, "min", p.min), (&words.maximum, "max", p.max)] {
                if let Some(v) = widgets::number(ui, label.clone(), Some(value)) {
                    let mut params = json!({"glyph": glyph, "property": p.name});
                    if let Some(m) = params.as_object_mut() {
                        m.insert(key.into(), json!(v));
                    }
                    outbox.push(Msg::run("smartGlyph.setProperty", params));
                }
            }
            if Button::new(words.remove.clone()).tooltip(words.remove_tip.clone()).id_salt(("remove", i)).show(ui).clicked() {
                outbox.push(Msg::run("smartGlyph.removeProperty", json!({"glyph": glyph, "property": p.name})));
            }
        });
    }
    if Button::new(words.add.clone()).tooltip(words.add_tip.clone()).enabled(data.properties.len() < MAX_PART_PROPERTIES).show(ui).clicked() {
        outbox.push(Msg::run("smartGlyph.addProperty", json!({"glyph": glyph, "name": data.new_name})));
    }
}

/// The Layers tab; the layer picked in its menu.
fn layers_tab(ui: &mut Ui, words: &SmartSettingsWords, view: &SmartGlyphView, data: &SmartGlyphData, outbox: &mut Vec<Msg>) -> Option<String> {
    let shown = view.layer.clone().filter(|id| data.layers.iter().any(|l| &l.id == id)).or_else(|| data.layers.first().map(|l| l.id.clone()))?;
    let entries: Vec<Entry<String>> = data.layers.iter().map(|l| Entry::new(l.id.clone(), l.label.clone())).collect();
    let picked =
        Select::new(words.layer.clone(), Bind::given(shown.clone()), &entries).show(ui).events.committed().filter(|id| **id != shown).cloned();
    if data.properties.is_empty() {
        Text::new(words.no_properties.clone()).shade(Shade::Muted).show(ui);
    }
    let layer = data.layers.iter().find(|l| l.id == shown)?;
    for (i, p) in data.properties.iter().enumerate() {
        let current = layer.poles.iter().find(|(name, _)| *name == p.name).map(|(_, top)| *top);
        let _ = Row::new().show(ui, |ui, _| {
            Text::new(p.name.clone()).show(ui);
            for (label, top) in [(&words.minimum, false), (&words.maximum, true)] {
                let out = Checkbox::new(Bind::given(current == Some(top))).label(label.clone()).id_salt(("pole", i, top)).show(ui);
                if out.events.committed().is_some() {
                    outbox.push(pole_command(&view.glyph, &layer.id, &p.name, current, top));
                }
            }
        });
    }
    picked
}

/// Smart Component Settings on the window's dialog host.
struct SmartComponentDialog {
    id: DialogId,
    model: SmartModel,
    read_at: Option<Stamp>,
    outbox: Vec<Msg>,
    open: bool,
}

impl StackDialog for SmartComponentDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<SmartSettingsWords>();
        let SmartComponentDialog { model, outbox, open, .. } = self;
        let mut held = true;
        let out = FormDialog::new(words.component_title.clone(), Bind::bound(&mut held))
            .opened(Bind::bound(open))
            .ok_label(words.close.clone())
            .show(ui, |ui, _, _| {
                controls(ui, model, outbox);
                true
            });
        match (out.accepted, out.closed) {
            (Some(_), _) => Some(DialogResult::Ok(())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

fn read_components(font: &Font, t: &SmartTargets) -> SmartModel {
    let Some(layer) = font.glyph(&t.glyph).and_then(|g| g.layers.iter().find(|l| l.id == t.layer)) else { return SmartModel::default() };
    let side = if t.background { LayerSide::Background } else { LayerSide::Foreground };
    smart_model(font, &t.glyph, layer, side, &t.components).unwrap_or_default()
}

/// Opens and closes both windows as the view state asks, reads the font again when it changed, and posts what the
/// person changed.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    let stamp = Stamp::of(&app.session, 0);
    run_glyph(host, app, stamp);
    run_component(host, app, stamp);
}

fn run_glyph(host: &mut DialogHost, app: &mut FontCraftApp, stamp: Stamp) {
    let Some(view) = app.view.components.smart_glyph.clone() else {
        if let Some(d) = host.get_mut::<SmartGlyphDialog>() {
            d.open = false;
        }
        let _ = host.take::<SmartGlyphDialog>();
        return;
    };
    if host.get_mut::<SmartGlyphDialog>().is_none() {
        host.open(SmartGlyphDialog {
            id: dialog_id("smartGlyph.settings"),
            view: view.clone(),
            data: SmartGlyphData::default(),
            read_at: None,
            outbox: Vec::new(),
            picked: None,
            open: true,
        });
    }
    if let Some(d) = host.get_mut::<SmartGlyphDialog>() {
        app.sent.append(&mut d.outbox);
        if let Some(picked) = d.picked.take() {
            app.view.components.smart_glyph = Some(picked.clone());
            app.view_changed();
            d.view = picked;
        } else if d.view != view {
            d.view = view;
            d.read_at = None;
        }
        if d.read_at != Some(stamp) {
            d.data = app.session.font().map(|f| read_glyph(f, &d.view.glyph)).unwrap_or_default();
            d.read_at = Some(stamp);
        }
    }
    if host.take::<SmartGlyphDialog>().is_some() {
        app.view.components.smart_glyph = None;
    }
}

fn run_component(host: &mut DialogHost, app: &mut FontCraftApp, stamp: Stamp) {
    let Some(targets) = app.view.components.smart_component.clone() else {
        if let Some(d) = host.get_mut::<SmartComponentDialog>() {
            d.open = false;
        }
        let _ = host.take::<SmartComponentDialog>();
        return;
    };
    if host.get_mut::<SmartComponentDialog>().is_none() {
        host.open(SmartComponentDialog {
            id: dialog_id("smartComponent.settings"),
            model: SmartModel::default(),
            read_at: None,
            outbox: Vec::new(),
            open: true,
        });
    }
    if let Some(d) = host.get_mut::<SmartComponentDialog>() {
        app.sent.append(&mut d.outbox);
        let same = d.model.glyph == targets.glyph && d.model.layer == targets.layer && d.model.components == targets.components;
        if d.read_at != Some(stamp) || !same {
            d.model = app.session.font().map(|f| read_components(f, &targets)).unwrap_or_default();
            d.read_at = Some(stamp);
        }
    }
    if host.take::<SmartComponentDialog>().is_some() {
        app.view.components.smart_component = None;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::pole_command;
    use crate::app::Msg;

    /// Ticking an end sets that pole; unticking the ticked end clears it.
    #[test]
    fn pole_checkboxes_set_and_clear() {
        let pole = |msg: Msg| match msg {
            Msg::Run(c) => c.params.get("pole").cloned(),
            _ => None,
        };
        assert_eq!(pole(pole_command("a", "m01", "Width", None, true)), Some(json!("max")));
        assert_eq!(pole(pole_command("a", "m01", "Width", Some(true), true)), Some(json!("none")));
        assert_eq!(pole(pole_command("a", "m01", "Width", Some(true), false)), Some(json!("min")));
    }
}

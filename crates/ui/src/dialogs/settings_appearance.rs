//! Settings › Appearance (`settings.appearance`): how Font View and Edit View are drawn.
//!
//! | Control | Posts |
//! |---|---|
//! | Display Unicode Value | `settings.set appearance.displayUnicodeValue` |
//! | Always use Light Mode: Font View, Edit View | `settings.set appearance.alwaysLightMode{FontView,EditView}` |
//! | Handle size | `settings.set appearance.handleSize` (`small`, `regular`, `large`) |
//! | a colour well, light or dark | `settings.set appearance.color.<item>.{light,dark}` as `#RRGGBBAA` |
//! | Standard | `settings.resetColors {}`: both colour sets back to their defaults |
//!
//! A line under the wells says which set the canvas is drawn in (`app::appearance`).

use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::behavior::ColumnWidth;
use craft::ui::layout::empty_cell;
use craft::ui::layout_components::Grid;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::style::TypeRole;
use egui::Ui;
use fontcraft_engine::settings::COLOR_PREFIX;
use serde_json::{Value, json};

use crate::app::appearance::{EDIT_VIEW_LIGHT, FONT_VIEW_LIGHT};
use crate::app::models::color::{Rgba8, hex, parse_hex};
use crate::app::{FontCraftApp, Msg};
use crate::panels::attributes::color_field;

craft::ui::words! {
    /// The words of Settings › Appearance.
    pub struct AppearanceWords {
        display_unicode: "fontcraftAppearance.displayUnicode" = "Display Unicode Value";
        display_unicode_note: "fontcraftAppearance.displayUnicodeNote" = "Font View cells show the code point instead of a small marker.";
        light_font_view: "fontcraftAppearance.lightFontView" = "Always use Light Mode: Font View";
        light_edit_view: "fontcraftAppearance.lightEditView" = "Always use Light Mode: Edit View";
        light_note: "fontcraftAppearance.lightNote" = "Keeps the view's content light while the system is in dark mode. The rest of the window follows the system.";
        handle_size: "fontcraftAppearance.handleSize" = "Handle size";
        small: "fontcraftAppearance.small" = "Small";
        regular: "fontcraftAppearance.regular" = "Regular";
        large: "fontcraftAppearance.large" = "Large";
        colors: "fontcraftAppearance.colors" = "Colours";
        light: "fontcraftAppearance.light" = "Light";
        dark: "fontcraftAppearance.dark" = "Dark";
        light_in_use: "fontcraftAppearance.lightInUse" = "The canvas is drawn with the Light colours.";
        dark_in_use: "fontcraftAppearance.darkInUse" = "The canvas is drawn with the Dark colours.";
        corner_nodes: "fontcraftAppearance.cornerNodes" = "Corner Nodes";
        smooth_nodes: "fontcraftAppearance.smoothNodes" = "Smooth Nodes";
        alignment_zones: "fontcraftAppearance.alignmentZones" = "Alignment Zones";
        foreground_stroke: "fontcraftAppearance.foregroundStroke" = "Foreground Layer Stroke";
        background_stroke: "fontcraftAppearance.backgroundStroke" = "Background Layer Stroke";
        other_layers_stroke: "fontcraftAppearance.otherLayersStroke" = "Other Layers Stroke";
        canvas: "fontcraftAppearance.canvas" = "Canvas";
        kerning_negative: "fontcraftAppearance.kerningNegative" = "Negative Kerning";
        kerning_positive: "fontcraftAppearance.kerningPositive" = "Positive Kerning";
        standard: "fontcraftAppearance.standard" = "Standard";
        standard_note: "fontcraftAppearance.standardNote" = "Standard returns every colour, light and dark, to its default.";
    }
}

const DISPLAY_UNICODE: &str = "appearance.displayUnicodeValue";
const HANDLE_SIZE: &str = "appearance.handleSize";

type ItemWords = fn(&AppearanceWords) -> &Label;

/// The colour items, in the handbook's order: the part of `appearance.color.<item>.<set>` and the row's label.
const ITEMS: [(&str, ItemWords); 9] = [
    ("cornerNodes", |w| &w.corner_nodes),
    ("smoothNodes", |w| &w.smooth_nodes),
    ("alignmentZones", |w| &w.alignment_zones),
    ("foregroundStroke", |w| &w.foreground_stroke),
    ("backgroundStroke", |w| &w.background_stroke),
    ("otherLayersStroke", |w| &w.other_layers_stroke),
    ("canvas", |w| &w.canvas),
    ("kerningNegative", |w| &w.kerning_negative),
    ("kerningPositive", |w| &w.kerning_positive),
];

/// The handle-size steps and their labels.
const HANDLE_SIZES: [(&str, ItemWords); 3] = [("small", |w| &w.small), ("regular", |w| &w.regular), ("large", |w| &w.large)];

/// The setting id of `item` in the dark set when `dark`.
pub fn color_id(item: &str, dark: bool) -> String {
    format!("{COLOR_PREFIX}{item}.{}", if dark { "dark" } else { "light" })
}

/// One colour item's two wells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorRow {
    pub item: &'static str,
    pub light: Option<Rgba8>,
    pub dark: Option<Rgba8>,
}

/// What the section shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppearanceModel {
    pub display_unicode: bool,
    pub font_view_light: bool,
    pub edit_view_light: bool,
    /// `small`, `regular` or `large`.
    pub handle_size: String,
    pub colors: Vec<ColorRow>,
    /// The canvas is drawn with the dark set.
    pub canvas_dark: bool,
}

impl AppearanceModel {
    pub fn of(app: &FontCraftApp) -> AppearanceModel {
        let settings = app.session.settings();
        let on = |id: &str| settings.get_bool(id).unwrap_or(false);
        let color = |id: String| settings.get(&id).ok().and_then(|v| v.as_str().and_then(parse_hex));
        AppearanceModel {
            display_unicode: on(DISPLAY_UNICODE),
            font_view_light: on(FONT_VIEW_LIGHT),
            edit_view_light: on(EDIT_VIEW_LIGHT),
            handle_size: settings.get(HANDLE_SIZE).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "regular".into()),
            colors: ITEMS.iter().map(|(item, _)| ColorRow { item, light: color(color_id(item, false)), dark: color(color_id(item, true)) }).collect(),
            canvas_dark: app.models.appearance.edit_view_dark(),
        }
    }
}

fn set(id: &str, value: Value) -> Msg {
    Msg::run("settings.set", json!({ "id": id, "value": value }))
}

fn note(ui: &mut Ui, text: &Label) {
    Text::new(text.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

/// The colour grid's columns: the item, its light colour, its dark colour.
const COLOR_COLUMNS: [ColumnWidth; 3] = [ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content];

/// The section's body.
pub fn section(ui: &mut Ui, model: &AppearanceModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<AppearanceWords>();
    if let Some(on) = Checkbox::new(Bind::given(model.display_unicode)).label(words.display_unicode.clone()).show(ui).events.committed() {
        sent.push(set(DISPLAY_UNICODE, json!(on)));
    }
    note(ui, &words.display_unicode_note);
    for (id, label, on) in
        [(FONT_VIEW_LIGHT, &words.light_font_view, model.font_view_light), (EDIT_VIEW_LIGHT, &words.light_edit_view, model.edit_view_light)]
    {
        if let Some(next) = Checkbox::new(Bind::given(on)).label(label.clone()).show(ui).events.committed() {
            sent.push(set(id, json!(next)));
        }
    }
    note(ui, &words.light_note);

    let entries: Vec<Entry<String>> = HANDLE_SIZES.iter().map(|(id, label)| Entry::new((*id).to_string(), label(&words).clone())).collect();
    let mut size = model.handle_size.clone();
    Field::new(words.handle_size.clone()).show(ui, |ui, _| {
        Segmented::new(words.handle_size.clone(), Bind::bound(&mut size), &entries).show(ui);
    });
    if size != model.handle_size {
        sent.push(set(HANDLE_SIZE, json!(size)));
    }

    Text::new(words.colors.clone()).role(TypeRole::Heading).show(ui);
    // One grid, so the wells of every row line up under the Light and Dark headings.
    let _ = Grid::new(&COLOR_COLUMNS).show(ui, |ui, _| {
        empty_cell(ui);
        Text::new(words.light.clone()).shade(Shade::Muted).show(ui);
        Text::new(words.dark.clone()).shade(Shade::Muted).show(ui);
        for (row, (_, label_of)) in model.colors.iter().zip(ITEMS.iter()) {
            let label = label_of(&words);
            Text::new(label.clone()).show(ui);
            for (dark, color, set_name) in [(false, row.light, &words.light), (true, row.dark, &words.dark)] {
                let well = Label::from(format!("{} ({})", label.as_str(), set_name.as_str()));
                if let Some(picked) = color_field(ui, &well, color, ("fontcraftAppearance.color", row.item, dark)) {
                    sent.push(set(&color_id(row.item, dark), json!(hex(picked))));
                }
            }
        }
    });
    note(ui, if model.canvas_dark { &words.dark_in_use } else { &words.light_in_use });
    if Button::new(words.standard.clone()).show(ui).clicked() {
        sent.push(Msg::run("settings.resetColors", json!({})));
    }
    note(ui, &words.standard_note);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::{AppearanceModel, section};
    use crate::app::{APP, FontCraftApp, Msg};

    fn shown(model: AppearanceModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let posted = Rc::new(RefCell::new(Vec::<Msg>::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            section(ui, &model, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        (fixture, posted)
    }

    /// The section reads the stored settings; its switches, a colour typed into a well and Standard post their
    /// commands, and the posted commands change the settings the canvas reads.
    #[test]
    fn appearance_controls_write_their_settings() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let model = AppearanceModel::of(&app);
        assert_eq!(model.colors.len(), 9);
        assert_eq!(model.handle_size, "regular");
        assert_eq!(model.colors.iter().find(|r| r.item == "canvas").and_then(|r| r.dark), Some([0x1e, 0x1e, 0x1e, 0xff]));
        assert!(!model.canvas_dark);

        let (mut fixture, posted) = shown(model);
        assert!(fixture.texts().iter().any(|t| t.text == "The canvas is drawn with the Light colours."));
        let switch = fixture.by_label("Always use Light Mode: Edit View").expect("the Edit View switch");
        fixture.click(&switch).settle();
        let well = fixture.by_label("Corner Nodes (Dark)").expect("the dark corner-node field");
        fixture.click(&well).key(Key::A, Modifiers::COMMAND).type_text("#ff000080").key(Key::Enter, Modifiers::NONE).settle();
        let standard = fixture.by_label("Standard").expect("Standard");
        fixture.click(&standard).settle();
        let set = |id: &str, value: serde_json::Value| Msg::run("settings.set", json!({ "id": id, "value": value }));
        let expected = vec![
            set("appearance.alwaysLightModeEditView", json!(true)),
            set("appearance.color.cornerNodes.dark", json!("#FF000080")),
            Msg::run("settings.resetColors", json!({})),
        ];
        assert_eq!(*posted.borrow(), expected);

        for msg in expected.into_iter().take(2) {
            app.run_now(msg, false).unwrap();
        }
        app.set_system_dark(true);
        app.drain();
        let model = AppearanceModel::of(&app);
        assert!(model.edit_view_light && !model.canvas_dark, "Edit View is kept light under the dark window");
        assert_eq!(model.colors.iter().find(|r| r.item == "cornerNodes").and_then(|r| r.dark), Some([0xff, 0, 0, 0x80]));
        app.run_now(Msg::run("settings.resetColors", json!({})), false).unwrap();
        let model = AppearanceModel::of(&app);
        assert_eq!(model.colors.iter().find(|r| r.item == "cornerNodes").and_then(|r| r.dark), Some([0x2d, 0x6c, 0xdf, 0xff]));
        assert!(model.edit_view_light, "Standard leaves the switches alone");
    }
}

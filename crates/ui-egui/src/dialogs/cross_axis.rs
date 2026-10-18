//! The Cross-Axis Mappings window (`window.crossAxisMappings`, Font › Cross-Axis Mappings): the
//! active font's cross-axis mappings as cards, edited as a draft and written with
//! `font.setCrossAxisMappings` when the person presses Apply.
//!
//! | Control | Does |
//! |---|---|
//! | description field at the top | the description of the whole list |
//! | disclosure arrow of a card | shows or hides the card's axes; Alt-click shows or hides every card |
//! | description field of a card | the mapping's description |
//! | `−` on a card | removes the mapping |
//! | tick in the Input or Output column | the axis takes part in that side of the mapping |
//! | number in the Input or Output column | the axis's design coordinate on that side |
//! | New Cross-Axis Mapping | adds a card with every axis at the default master's location |
//! | Apply | runs `font.setCrossAxisMappings` with the draft, one undo step |
//! | Revert | drops the draft's changes |
//!
//! The draft lives in egui's memory. While it has no changes it follows the font, so an undo
//! shows at once; a draft with changes stays until the font holds them (Apply), Revert, or
//! another document becomes active. A refused Apply keeps the draft.

use fontcraft_engine::cmd::cross_axis::{CrossAxisMapping, CrossAxisMappings, mappings};
use fontcraft_engine::model::Font;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};

/// Width of a card's axis-name column and of each value field, in points.
const AXIS_WIDTH: f32 = 120.0;
const VALUE_WIDTH: f32 = 72.0;

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CrossAxisState {
    pub open: bool,
}

pub const COMMANDS: &[UiCommand] = &[(
    "window.crossAxisMappings",
    "Cross-Axis Mappings",
    &["Font"],
    "",
    "{open?: bool (default true)} shows or hides the Cross-Axis Mappings window of the active font → {open, description?, mappings: [{description?, input, output}]}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "window.crossAxisMappings" {
        return Err(format!("unknown command `{id}`"));
    }
    let open = args.flag("open")?.unwrap_or(true);
    let font = app.session.font().map_err(|e| args.err(e))?;
    let mut reply = mappings(font).to_json();
    app.state.panels.cross_axis.open = open;
    if let Value::Object(m) = &mut reply {
        m.insert("open".into(), json!(open));
    }
    Ok(reply)
}

/// One axis on one side of a card.
#[derive(Clone, Debug, PartialEq)]
struct Cell {
    on: bool,
    value: f64,
}

/// One mapping as the window edits it: a cell per font axis on each side.
#[derive(Clone, Debug, PartialEq)]
struct Card {
    description: String,
    open: bool,
    input: Vec<Cell>,
    output: Vec<Cell>,
}

/// The mappings being edited, with the document and revision they were read from.
#[derive(Clone, Debug, PartialEq)]
struct Draft {
    source: (u64, u64),
    changed: bool,
    description: String,
    cards: Vec<Card>,
}

/// The default master's design location, one value per axis.
fn default_location(font: &Font) -> Vec<f64> {
    let master = font.masters.first();
    (0..font.axes.len()).map(|i| master.and_then(|m| m.location.get(i).copied()).unwrap_or(0.0)).collect()
}

fn cells(font: &Font, side: &[(String, f64)], defaults: &[f64]) -> Vec<Cell> {
    font.axes
        .iter()
        .zip(defaults)
        .map(|(axis, d)| match side.iter().find(|(t, _)| *t == axis.tag) {
            Some((_, v)) => Cell { on: true, value: *v },
            None => Cell { on: false, value: *d },
        })
        .collect()
}

fn read_draft(font: &Font, source: (u64, u64)) -> Draft {
    let stored = mappings(font);
    let defaults = default_location(font);
    let cards = stored
        .mappings
        .iter()
        .map(|m| Card {
            description: m.description.clone().unwrap_or_default(),
            open: true,
            input: cells(font, &m.input, &defaults),
            output: cells(font, &m.output, &defaults),
        })
        .collect();
    Draft { source, changed: false, description: stored.description.unwrap_or_default(), cards }
}

/// The parameters of `font.setCrossAxisMappings` for the draft.
fn params(font: &Font, draft: &Draft) -> Value {
    let side = |cells: &[Cell]| {
        Value::Object(font.axes.iter().zip(cells).filter(|(_, c)| c.on).map(|(a, c)| (a.tag.clone(), json!(c.value))).collect::<Map<String, Value>>())
    };
    let list: Vec<Value> = draft
        .cards
        .iter()
        .map(|c| {
            let mut o = Map::new();
            o.insert("input".into(), side(&c.input));
            o.insert("output".into(), side(&c.output));
            if !c.description.trim().is_empty() {
                o.insert("description".into(), json!(c.description));
            }
            Value::Object(o)
        })
        .collect();
    let mut p = Map::new();
    p.insert("mappings".into(), Value::Array(list));
    if !draft.description.trim().is_empty() {
        p.insert("description".into(), json!(draft.description));
    }
    Value::Object(p)
}

/// What the draft would store, for telling whether Apply has anything to do.
fn as_mappings(font: &Font, draft: &Draft) -> CrossAxisMappings {
    let side = |cells: &[Cell]| font.axes.iter().zip(cells).filter(|(_, c)| c.on).map(|(a, c)| (a.tag.clone(), c.value)).collect();
    let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    CrossAxisMappings {
        description: text(&draft.description),
        mappings: draft
            .cards
            .iter()
            .map(|c| CrossAxisMapping { description: text(&c.description), input: side(&c.input), output: side(&c.output) })
            .collect(),
    }
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.cross_axis.open {
        return;
    }
    let Some(doc) = app.session.active() else { return };
    let source = (doc.id, doc.revision);
    let font = doc.font.clone();
    let memory_id = egui::Id::new("window.crossAxisMappings.draft");
    let stored = mappings(&font);
    let mut draft = match ui.ctx().data(|d| d.get_temp::<Draft>(memory_id)) {
        // A draft whose changes the font now holds (after Apply) follows the font again.
        Some(mut d) if d.source.0 == source.0 && d.changed && as_mappings(&font, &d) == stored => {
            d.changed = false;
            d.source = source;
            d
        }
        Some(d) if d.source.0 == source.0 && (d.changed || d.source.1 == source.1) => d,
        _ => read_draft(&font, source),
    };
    let before = draft.clone();
    let mut reverted = false;
    let tokens = &app.tokens;
    let mut open = true;
    egui::Window::new("Cross-Axis Mappings")
        .id(egui::Id::new("window.crossAxisMappings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(AXIS_WIDTH + 4.0 * VALUE_WIDTH + 2.0 * tokens.dialog_spacing)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            if font.axes.is_empty() {
                ui.colored_label(tokens.text_dim, "The font has no axes.");
                return;
            }
            ui.horizontal(|ui| {
                ui.label("Description");
                ui.text_edit_singleline(&mut draft.description);
            });
            ui.add_space(tokens.dialog_spacing);
            let alt = ui.input(|i| i.modifiers.alt);
            let mut toggle_all: Option<bool> = None;
            let mut remove: Option<usize> = None;
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                for (i, card) in draft.cards.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let arrow = if card.open { "▾" } else { "▸" };
                        if ui.small_button(arrow).clicked() {
                            if alt {
                                toggle_all = Some(!card.open);
                            } else {
                                card.open = !card.open;
                            }
                        }
                        ui.add(egui::TextEdit::singleline(&mut card.description).hint_text(format!("Mapping {}", i + 1)));
                        if ui.small_button("−").on_hover_text("Remove this mapping").clicked() {
                            remove = Some(i);
                        }
                    });
                    if card.open {
                        card_grid(ui, &font, i, card);
                    }
                    ui.separator();
                }
            });
            if let Some(on) = toggle_all {
                draft.cards.iter_mut().for_each(|c| c.open = on);
            }
            if let Some(i) = remove.filter(|i| *i < draft.cards.len()) {
                draft.cards.remove(i);
            }
            ui.horizontal(|ui| {
                if ui.button("New Cross-Axis Mapping").clicked() {
                    let defaults = default_location(&font);
                    let cell = |v: &f64| Cell { on: false, value: *v };
                    draft.cards.push(Card {
                        description: String::new(),
                        open: true,
                        input: defaults.iter().map(cell).collect(),
                        output: defaults.iter().map(cell).collect(),
                    });
                }
                let differs = as_mappings(&font, &draft) != stored;
                if ui.add_enabled(differs, egui::Button::new("Apply")).clicked() {
                    pending.push(("font.setCrossAxisMappings", params(&font, &draft)));
                }
                if ui.add_enabled(draft.changed, egui::Button::new("Revert")).clicked() {
                    reverted = true;
                }
            });
        });
    if reverted {
        draft = read_draft(&font, source);
    } else if as_mappings(&font, &draft) != as_mappings(&font, &before) {
        draft.changed = true;
    }
    ui.ctx().data_mut(|d| d.insert_temp(memory_id, draft));
    if !open {
        app.state.panels.cross_axis.open = false;
    }
}

/// One card's axes: a row per font axis with a tick and a value for each side.
fn card_grid(ui: &mut egui::Ui, font: &Font, index: usize, card: &mut Card) {
    egui::Grid::new(("window.crossAxisMappings.card", index)).num_columns(5).show(ui, |ui| {
        ui.label("");
        ui.label("Input");
        ui.label("");
        ui.label("Output");
        ui.label("");
        ui.end_row();
        for (k, axis) in font.axes.iter().enumerate() {
            ui.add_sized([AXIS_WIDTH, 0.0], egui::Label::new(axis.name.as_str()).truncate());
            for side in [&mut card.input, &mut card.output] {
                let Some(cell) = side.get_mut(k) else { continue };
                ui.checkbox(&mut cell.on, "");
                ui.add_enabled_ui(cell.on, |ui| {
                    ui.add_sized([VALUE_WIDTH, 0.0], egui::DragValue::new(&mut cell.value).speed(1.0).range(-1_000_000.0..=1_000_000.0));
                });
            }
            ui.end_row();
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use fontcraft_engine::model::Axis;

    use super::*;

    fn variable_app() -> FontcraftApp {
        let mut font = Font::new("Cross");
        font.axes = Arc::new(vec![
            Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() },
            Axis { name: "Width".into(), tag: "wdth".into(), ..Default::default() },
        ]);
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0, 100.0];
        let mut session = fontcraft_engine::Session::new();
        session.add_document(font, None).unwrap();
        FontcraftApp::new(session)
    }

    fn frame(app: &mut FontcraftApp, ctx: &egui::Context) {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
    }

    #[test]
    fn window_cross_axis_mappings_opens_on_the_fonts_mappings_and_applies_the_draft() {
        let mut app = variable_app();
        app.run("font.setCrossAxisMappings", &json!({"mappings": [{"input": {"wght": 700}, "output": {"wdth": 80}}]})).unwrap();
        let r = app.run("window.crossAxisMappings", &json!({})).unwrap();
        assert_eq!(r["open"], true);
        assert_eq!(r["mappings"], json!([{"input": {"wght": 700.0}, "output": {"wdth": 80.0}}]));
        assert!(app.state.panels.cross_axis.open);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);

        // The draft holds a cell per axis on each side, ticked where the mapping names the axis,
        // and gives back the same parameters.
        let font = app.session.font().unwrap().clone();
        let draft = ctx.data(|d| d.get_temp::<Draft>(egui::Id::new("window.crossAxisMappings.draft"))).unwrap();
        assert_eq!(draft.cards.len(), 1);
        assert_eq!(draft.cards[0].input, vec![Cell { on: true, value: 700.0 }, Cell { on: false, value: 100.0 }]);
        assert_eq!(as_mappings(&font, &draft), mappings(&font));
        let mut edited = draft.clone();
        edited.cards[0].output[0] = Cell { on: true, value: 650.0 };
        edited.description = "Narrow bolds".into();
        app.run("font.setCrossAxisMappings", &params(&font, &edited)).unwrap();
        let stored = mappings(app.session.font().unwrap());
        assert_eq!(stored.description.as_deref(), Some("Narrow bolds"));
        assert_eq!(stored.mappings[0].output, vec![("wght".to_string(), 650.0), ("wdth".to_string(), 80.0)]);

        // The window follows the font while its draft is unchanged, and closes.
        frame(&mut app, &ctx);
        let draft = ctx.data(|d| d.get_temp::<Draft>(egui::Id::new("window.crossAxisMappings.draft"))).unwrap();
        assert_eq!(draft.description, "Narrow bolds");
        app.run("window.crossAxisMappings", &json!({"open": false})).unwrap();
        assert!(!app.state.panels.cross_axis.open);
        frame(&mut app, &ctx);
    }

    #[test]
    fn window_cross_axis_mappings_draws_for_a_font_without_axes() {
        let mut app = FontcraftApp::new(fontcraft_engine::Session::with_document());
        let r = app.run("window.crossAxisMappings", &json!({})).unwrap();
        assert_eq!(r["mappings"], json!([]));
        frame(&mut app, &egui::Context::default());
        let mut empty = FontcraftApp::new(fontcraft_engine::Session::new());
        assert!(empty.run("window.crossAxisMappings", &json!({})).is_err(), "no document, no window");
        assert!(!empty.state.panels.cross_axis.open);
    }
}

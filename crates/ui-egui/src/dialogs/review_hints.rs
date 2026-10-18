//! The Review Hints dialog (`dialog.reviewHints`, `hint.ps.reviewHints`): steps through the
//! PostScript hints of one direction of a glyph's layer, one at a time, for inspection and
//! numeric correction.
//!
//! | Control | Does |
//! |---|---|
//! | Horizontal / Vertical | the direction reviewed; the first hint of that direction becomes the reviewed one |
//! | Position field | `hint.ps.setHintValues {origin}`: the origin edge |
//! | Width field | `hint.ps.setHintValues {size}`: the signed distance to the other edge; empty for a ghost hint, and a width makes it a stem |
//! | Previous / Next | the hint of the direction before or after the reviewed one, in list order; off at the first and the last |
//! | Create | `hint.ps.addHorizontalHint` or `hint.ps.addVerticalHint` without nodes; the new hint, at the end of the list, becomes the reviewed one |
//! | Remove | `hint.ps.deleteHints` on the reviewed hint; the next hint of the direction, else the previous one, becomes the reviewed one |
//!
//! Opening the dialog turns on View › Show Hints. When the reviewed layer is the one the showing
//! Edit View tab edits, the reviewed hint is that tab's selection, so the canvas marks it. The
//! dialog reads the font every frame and blocks nothing; edits go through the `hint.ps.*`
//! commands, so a refusal changes nothing and its message shows in the status bar.

use fontcraft_engine::hinting::settings::main_master;
use fontcraft_engine::hinting::{PsHint, ps_hints};
use fontcraft_engine::model::{Font, Layer};
use fontcraft_tools::{Item, Selection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::{Pending, current_glyph, widgets};
use crate::shell::{Args, UiCommand};

pub const ID: &str = "hint.ps.reviewHints";

pub const COMMANDS: &[UiCommand] = &[(
    ID,
    "Review Hints…",
    &[],
    "",
    "{glyph?: name (default the edited glyph, else the first glyph selected in Font View), layer?: id (default the main master's layer), direction?: \"horizontal\" | \"vertical\" (default the direction of `hint`, else the one reviewed before, else horizontal), hint?: index (in the layer's PostScript hint list), step?: \"next\" | \"previous\" (from the reviewed hint, within the direction), open?: bool (false closes the dialog)} opens the Review Hints dialog on the layer's PostScript hints of one direction, in list order, and turns on Show Hints. Without `hint` or `step` the first hint of the direction is reviewed; stepping past the first or the last is refused → {open, glyph, layer, direction, hint: index | null, hints: [index] (the direction's hints), origin: number | null, size: number | null (null for a ghost)}",
)];

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewHintsState {
    pub open: bool,
    pub glyph: String,
    pub layer: String,
    /// Horizontal hints (y edges) are reviewed; else vertical ones.
    pub horizontal: bool,
    /// The reviewed hint, by its index in the layer's hint list.
    pub hint: Option<usize>,
}

pub fn direction_id(horizontal: bool) -> egui::Id {
    widgets::id((ID, "direction", horizontal))
}

pub fn previous_button_id() -> egui::Id {
    widgets::id((ID, "previous"))
}

pub fn next_button_id() -> egui::Id {
    widgets::id((ID, "next"))
}

pub fn create_button_id() -> egui::Id {
    widgets::id((ID, "create"))
}

pub fn remove_button_id() -> egui::Id {
    widgets::id((ID, "remove"))
}

pub fn field_id(which: &str) -> egui::Id {
    widgets::id((ID, "field", which))
}

/// The layer `layer` of `glyph`.
fn layer<'f>(font: &'f Font, glyph: &str, layer: &str) -> Result<&'f Layer, String> {
    let g = font.glyph(glyph).ok_or_else(|| format!("there is no glyph `{glyph}`"))?;
    g.layers.iter().find(|l| l.id == layer).ok_or_else(|| format!("glyph `{glyph}` has no layer `{layer}`"))
}

/// The layer's hints of one direction, with their indices in the hint list.
fn hints_of(layer: &Layer, horizontal: bool) -> Result<Vec<(usize, PsHint)>, String> {
    Ok(ps_hints(layer).map_err(|e| e.to_string())?.into_iter().filter(|(_, h)| h.horizontal == horizontal).collect())
}

/// A hint's origin and its signed size (`None` for a ghost), as `hint.ps.listHints` gives them.
fn values(layer: &Layer, hint: &PsHint) -> (Option<f64>, Option<f64>) {
    let origin = hint.origin.position(layer, hint.horizontal).ok();
    let size = match (&hint.other, origin) {
        (Some(other), Some(o)) => other.position(layer, hint.horizontal).ok().map(|v| v - o),
        _ => None,
    };
    (origin, size)
}

/// The hint after (`forward`) or before `from` among `list`, by list index.
fn neighbour(list: &[(usize, PsHint)], from: Option<usize>, forward: bool) -> Option<usize> {
    let from = from?;
    if forward { list.iter().map(|(i, _)| *i).find(|i| *i > from) } else { list.iter().rev().map(|(i, _)| *i).find(|i| *i < from) }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != ID {
        return Err(format!("unknown command `{id}`"));
    }
    if args.flag("open")? == Some(false) {
        app.state.panels.review_hints.open = false;
        return Ok(json!({"open": false}));
    }
    let previous = app.state.panels.review_hints.clone();
    let font = app.session.font().map_err(|e| args.err(e))?;
    let glyph = match args.string("glyph")? {
        Some(g) => g.to_string(),
        None if previous.open && args.get("step").is_some() => previous.glyph.clone(),
        None => current_glyph(app).ok_or_else(|| args.err("no glyph is edited or selected: give `glyph`"))?,
    };
    let layer_id = match args.string("layer")? {
        Some(l) => l.to_string(),
        None if previous.open && previous.glyph == glyph => previous.layer.clone(),
        None => main_master(font),
    };
    let l = layer(font, &glyph, &layer_id).map_err(|e| args.err(e))?;
    let all = ps_hints(l).map_err(|e| args.err(e))?;
    let given = match args.index("hint")? {
        Some(i) => {
            let i = usize::try_from(i).map_err(|_| args.err("`hint` is too large"))?;
            Some(all.iter().find(|(n, _)| *n == i).ok_or_else(|| args.err(format!("the layer has no PostScript hint {i}")))?)
        }
        None => None,
    };
    let horizontal = match args.string("direction")? {
        Some("horizontal") => true,
        Some("vertical") => false,
        Some(other) => return Err(args.err(format!("`direction` must be \"horizontal\" or \"vertical\", got \"{other}\""))),
        None => match given {
            Some((_, h)) => h.horizontal,
            None if previous.open && previous.glyph == glyph => previous.horizontal,
            None => true,
        },
    };
    if let Some((i, h)) = given
        && h.horizontal != horizontal
    {
        return Err(args.err(format!("hint {i} is not a {} hint", if horizontal { "horizontal" } else { "vertical" })));
    }
    let list = hints_of(l, horizontal).map_err(|e| args.err(e))?;
    let same_place = previous.open && previous.glyph == glyph && previous.layer == layer_id && previous.horizontal == horizontal;
    let hint = match (args.string("step")?, given) {
        (Some(step @ ("next" | "previous")), _) => {
            let from = if same_place { previous.hint } else { None };
            let from = from.filter(|f| list.iter().any(|(i, _)| i == f)).ok_or_else(|| args.err("no hint is reviewed to step from"))?;
            let forward = step == "next";
            let next = neighbour(&list, Some(from), forward);
            Some(next.ok_or_else(|| args.err(format!("this is the {} {} hint", if forward { "last" } else { "first" }, direction(horizontal))))?)
        }
        (Some(other), _) => return Err(args.err(format!("`step` must be \"next\" or \"previous\", got \"{other}\""))),
        (None, Some((i, _))) => Some(*i),
        (None, None) => list.first().map(|(i, _)| *i),
    };
    let (origin, size) = hint.and_then(|h| list.iter().find(|(i, _)| *i == h)).map_or((None, None), |(_, h)| values(l, h));
    let reply = json!({
        "open": true, "glyph": glyph, "layer": layer_id, "direction": direction(horizontal), "hint": hint,
        "hints": list.iter().map(|(i, _)| *i).collect::<Vec<_>>(), "origin": origin, "size": size,
    });
    app.state.panels.review_hints = ReviewHintsState { open: true, glyph, layer: layer_id, horizontal, hint };
    app.state.display.hints = true;
    highlight(app);
    Ok(reply)
}

fn direction(horizontal: bool) -> &'static str {
    if horizontal { "horizontal" } else { "vertical" }
}

/// Makes the reviewed hint the selection of the showing tab when that tab edits the reviewed
/// layer.
fn highlight(app: &mut FontcraftApp) {
    let st = &app.state.panels.review_hints;
    let Some(hint) = st.hint else { return };
    let Ok(font) = app.session.font() else { return };
    let Some(tab) = app.state.tab() else { return };
    let edits_it = tab.edited_glyph() == Some(st.glyph.as_str()) && crate::canvas::view::active_layer(font, tab).is_some_and(|l| l.id == st.layer);
    if !edits_it {
        return;
    }
    if let Some(tab) = app.state.tab_mut() {
        tab.selection = Selection::from_items([Item::Hint(hint)]);
    }
}

/// What the person did this frame.
enum Press {
    Direction(bool),
    Step(bool),
    Create,
    Remove,
}

/// Draws the dialog while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.review_hints.open {
        return;
    }
    let st = app.state.panels.review_hints.clone();
    let mut open = true;
    let mut press = None;
    // The layer's hint count, its hints of the reviewed direction, and the first hint of each
    // direction (horizontal, vertical), for the hint reviewed next.
    let mut list_len = 0usize;
    let mut list: Vec<(usize, PsHint)> = Vec::new();
    let mut firsts: (Option<usize>, Option<usize>) = (None, None);
    {
        let shown: &FontcraftApp = app;
        let tokens = &shown.tokens;
        egui::Window::new("Review Hints")
            .id(egui::Id::new(ID))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
            .show(ui.ctx(), |ui| {
                let Ok(font) = shown.session.font() else { return };
                let l = match layer(font, &st.glyph, &st.layer) {
                    Ok(l) => l,
                    Err(e) => {
                        ui.label(egui::RichText::new(e).color(tokens.text_dim));
                        return;
                    }
                };
                let all = ps_hints(l).unwrap_or_default();
                list_len = all.len();
                firsts = (all.iter().find(|(_, h)| h.horizontal).map(|(i, _)| *i), all.iter().find(|(_, h)| !h.horizontal).map(|(i, _)| *i));
                list = all.into_iter().filter(|(_, h)| h.horizontal == st.horizontal).collect();
                ui.label(egui::RichText::new(&st.glyph).color(tokens.text_dim));
                ui.horizontal(|ui| {
                    for (horizontal, caption, hover) in
                        [(true, "Horizontal", "Review the horizontal hints (y edges)"), (false, "Vertical", "Review the vertical hints (x edges)")]
                    {
                        if widgets::button(ui, direction_id(horizontal), caption, hover, true, st.horizontal == horizontal)
                            && st.horizontal != horizontal
                        {
                            press = Some(Press::Direction(horizontal));
                        }
                    }
                });
                ui.separator();
                let reviewed = st.hint.and_then(|h| list.iter().position(|(i, _)| *i == h));
                match reviewed.and_then(|p| list.get(p)) {
                    None => {
                        ui.label(egui::RichText::new(format!("No {} hint.", direction(st.horizontal))).color(tokens.text_dim));
                    }
                    Some((index, hint)) => {
                        let (origin, size) = values(l, hint);
                        let at = reviewed.unwrap_or(0) + 1;
                        ui.label(format!("Hint {at} of {} (list index {index})", list.len()));
                        egui::Grid::new((ID, "fields")).num_columns(2).show(ui, |ui| {
                            ui.label("Position");
                            if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, field_id("origin"), origin, tokens.field_width, false) {
                                pending.push(("hint.ps.setHintValues", json!({"glyph": st.glyph, "layer": st.layer, "hint": index, "origin": v})));
                            }
                            ui.end_row();
                            ui.label("Width");
                            if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, field_id("size"), size, tokens.field_width, false) {
                                pending.push(("hint.ps.setHintValues", json!({"glyph": st.glyph, "layer": st.layer, "hint": index, "size": v})));
                            }
                            ui.end_row();
                        });
                    }
                }
                ui.horizontal(|ui| {
                    let has_previous = neighbour(&list, st.hint, false).is_some();
                    let has_next = neighbour(&list, st.hint, true).is_some();
                    if widgets::button(ui, previous_button_id(), "Previous", "Review the hint before this one", has_previous, false) {
                        press = Some(Press::Step(false));
                    }
                    if widgets::button(ui, next_button_id(), "Next", "Review the hint after this one", has_next, false) {
                        press = Some(Press::Step(true));
                    }
                    if widgets::button(ui, create_button_id(), "Create", "Add a hint of this direction and review it", true, false) {
                        press = Some(Press::Create);
                    }
                    if widgets::button(ui, remove_button_id(), "Remove", "Remove the reviewed hint", reviewed.is_some(), false) {
                        press = Some(Press::Remove);
                    }
                });
            });
    }
    let pressed = press.is_some();
    let state = &mut app.state.panels.review_hints;
    state.open = open;
    match press {
        Some(Press::Direction(horizontal)) => {
            state.horizontal = horizontal;
            state.hint = if horizontal { firsts.0 } else { firsts.1 };
        }
        Some(Press::Step(forward)) => {
            if let Some(next) = neighbour(&list, st.hint, forward) {
                state.hint = Some(next);
            }
        }
        Some(Press::Create) => {
            let id = if st.horizontal { "hint.ps.addHorizontalHint" } else { "hint.ps.addVerticalHint" };
            pending.push((id, json!({"glyph": st.glyph, "layer": st.layer})));
            // New hints go to the end of the list.
            state.hint = Some(list_len);
        }
        Some(Press::Remove) => {
            if let Some(h) = st.hint {
                pending.push(("hint.ps.deleteHints", json!({"glyph": st.glyph, "layer": st.layer, "hints": [h]})));
                // The hints after the removed one move up by one.
                state.hint = neighbour(&list, Some(h), true).map(|i| i.saturating_sub(1)).or_else(|| neighbour(&list, Some(h), false));
            }
        }
        None => {}
    }
    if pressed {
        highlight(app);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use fontcraft_engine::model::{Path, Shape};

    use super::*;

    /// `H` with hints in list order: horizontal 0, vertical 1, horizontal 2, horizontal 3.
    fn app() -> FontcraftApp {
        let mut font = Font::new("Review");
        font.glyph_mut("H").unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0))];
        let mut session = fontcraft_engine::Session::new();
        session.add_document(font, None).unwrap();
        let mut app = FontcraftApp::new(session);
        for (id, origin, size) in [
            ("hint.ps.addHorizontalHint", 0.0, 20.0),
            ("hint.ps.addVerticalHint", 50.0, 100.0),
            ("hint.ps.addHorizontalHint", 340.0, 20.0),
            ("hint.ps.addHorizontalHint", 680.0, 20.0),
        ] {
            app.run(id, &json!({"glyph": "H", "origin": origin, "size": size})).unwrap();
        }
        app
    }

    fn frame(app: &mut FontcraftApp, ctx: &egui::Context) {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
    }

    #[test]
    fn dialog_review_hints_steps_through_the_hints_of_one_direction() {
        let mut app = app();
        let ctx = egui::Context::default();
        let r = app.run(ID, &json!({"glyph": "H"})).unwrap();
        assert_eq!((r["direction"].as_str(), r["hint"].as_u64()), (Some("horizontal"), Some(0)));
        assert_eq!(r["hints"], json!([0, 2, 3]));
        assert_eq!((r["origin"].as_f64(), r["size"].as_f64()), (Some(0.0), Some(20.0)));
        assert!(app.state.display.hints, "opening shows the hints");
        frame(&mut app, &ctx);

        let r = app.run(ID, &json!({"step": "next"})).unwrap();
        assert_eq!((r["hint"].as_u64(), r["origin"].as_f64()), (Some(2), Some(340.0)));
        let r = app.run(ID, &json!({"step": "next"})).unwrap();
        assert_eq!(r["hint"], 3);
        assert!(app.run(ID, &json!({"step": "next"})).is_err(), "the last horizontal hint");
        assert_eq!(app.state.panels.review_hints.hint, Some(3), "a refused step keeps the reviewed hint");
        let r = app.run(ID, &json!({"step": "previous"})).unwrap();
        assert_eq!(r["hint"], 2);
        frame(&mut app, &ctx);

        let r = app.run(ID, &json!({"direction": "vertical"})).unwrap();
        assert_eq!((r["hint"].as_u64(), r["hints"].clone()), (Some(1), json!([1])));
        assert_eq!((r["origin"].as_f64(), r["size"].as_f64()), (Some(50.0), Some(100.0)));
        assert!(app.run(ID, &json!({"step": "previous"})).is_err(), "the first vertical hint");
        assert!(app.run(ID, &json!({"hint": 2, "direction": "vertical"})).is_err(), "hint 2 is horizontal");
        frame(&mut app, &ctx);
        app.run(ID, &json!({"open": false})).unwrap();
        frame(&mut app, &ctx);
    }

    #[test]
    fn dialog_review_hints_draws_a_layer_without_hints() {
        let mut font = Font::new("Empty");
        font.glyph_mut("H").unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0))];
        let mut session = fontcraft_engine::Session::new();
        session.add_document(font, None).unwrap();
        let mut app = FontcraftApp::new(session);
        let r = app.run(ID, &json!({"glyph": "H"})).unwrap();
        assert!(r["hint"].is_null());
        assert!(app.run(ID, &json!({"step": "next"})).is_err());
        frame(&mut app, &egui::Context::default());
        assert!(app.run(ID, &json!({"glyph": "Missing"})).is_err());
    }
}

//! The colour palettes editor: a staged copy of the palettes; OK runs the edits as one
//! `command.batch`, Cancel changes nothing. Package M11.15 (`docs/plans/M11.md` § 4); rows
//! `colorPalette.edit`, `dialog.colorPalettes`, `color.palette.editorIsTransactional`.
//!
//! `colorPalette.edit` opens it on the font's `Color Palettes`. Its one field, `edits`, is the
//! list of palette commands the person has made so far, each `{command, params}` exactly as an
//! agent would run it: `colorPalette.addColor`, `removeColor`, `setColor`, `addPalette`,
//! `removePalette`, without `master`. The table shows the palettes as they will be after those
//! edits: one row per colour index and one column per palette, both counted from 0.
//!
//! | Control | Edit added |
//! |---|---|
//! | a swatch, then a colour in its picker | `colorPalette.setColor` (a second colour for the same cell replaces the first edit) |
//! | plus | `colorPalette.addColor` (opaque black) |
//! | minus | `colorPalette.removeColor` of the selected row (a click on a row's number selects it) |
//! | gear menu › Add Palette | `colorPalette.addPalette` (a copy of palette 0) |
//! | a palette's heading › Remove Palette | `colorPalette.removePalette` (not the last palette) |
//!
//! Nothing touches the font while the editor is open ([`Mode::RunOnOk`]). OK runs
//! `command.batch {label: "Color Palettes", steps: edits}`, one undo step; a step the font
//! refuses (removing a colour a layer uses) keeps the editor open with the message. Over the
//! control channel: `ui.dialog.set {field: "edits", value: [...]}`, then `ui.dialog.confirm` or
//! `ui.dialog.cancel`.

use egui::{Id, Popup, Response, Sense, StrokeKind};
use fontcraft_engine::color::palette::{self as color_palette, Palettes, Rgba};
use serde_json::{Map, Value, json};

use super::form::{Field, Kind};
use super::{Dialog, Event, Fields, Mode, Shaped};
use crate::app::FontcraftApp;
use crate::panels::attributes::{self, Rgba8};
use crate::panels::widgets;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    // Opened by `colorPalette.edit`, never as "the dialog of `command.batch`".
    follow_up: true,
    ..Dialog::new("dialog.colorPalettes", "Color Palettes", "command.batch", Fields::Shaped(Shaped { fields, given, params, link }))
};

/// Most edits one session of the editor holds.
pub const MAX_EDITS: usize = 10_000;
/// The field holding the edits.
pub const EDITS: &str = "edits";
/// The undo label of OK's batch.
const LABEL: &str = "Color Palettes";
/// The table scrolls past this many rows.
const VISIBLE_ROWS: f32 = 12.0;
/// A colour column is this many control heights wide (the swatch width).
const COLUMN_WIDTH: f32 = 1.5;

pub const COMMANDS: &[UiCommand] = &[(
    "colorPalette.edit",
    "Edit Color Palettes",
    &[],
    "",
    "{edits?: [{command: \"colorPalette.addColor\" | \"colorPalette.removeColor\" | \"colorPalette.setColor\" | \"colorPalette.addPalette\" | \"colorPalette.removePalette\", params: {…} (that command's parameters without `master`)}]} opens the editor of the font's Color Palettes with these edits staged. Nothing changes until OK, which runs the edits as one command.batch (one undo step); Cancel drops them. Then ui.dialog.set {field: \"edits\"}, ui.dialog.confirm, ui.dialog.cancel → ui.dialog.inspect's reply",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "colorPalette.edit" {
        return Err(format!("unknown command `{id}`"));
    }
    let mut given = Map::new();
    if let Some(edits) = args.get(EDITS) {
        given.insert(EDITS.into(), edits.clone());
    }
    super::open(app, &DIALOG, &given).map_err(|e| args.err(e))
}

// ── The dialog's shape ──────────────────────────────────────────────────────────────────────────

/// The palettes as the field kind keeps them: `#RRGGBBAA` per colour, one list per palette.
pub fn table(palettes: &Palettes) -> Vec<Vec<String>> {
    palettes.palettes().iter().map(|p| p.iter().map(Rgba::hex).collect()).collect()
}

fn parse_table(table: &[Vec<String>]) -> Result<Palettes, String> {
    let rows = table
        .iter()
        .map(|p| p.iter().map(|c| Rgba::parse_hex(c).ok_or_else(|| format!("`{c}` is not a colour"))).collect::<Result<Vec<_>, _>>())
        .collect::<Result<Vec<_>, _>>()?;
    Palettes::new(rows).map_err(|e| e.to_string())
}

fn fields(session: &fontcraft_engine::Session) -> Vec<Field> {
    let palettes = session.font().ok().and_then(|f| color_palette::read(f, None).ok()).unwrap_or_default();
    vec![Field {
        key: EDITS.into(),
        label: String::new(),
        kind: Kind::Palettes { palettes: table(&palettes) },
        default: json!([]),
        doc: "The palette edits OK runs, in order: [{command, params}] with the colorPalette commands' parameters (no `master`)".into(),
        group: None,
    }]
}

/// The editor opens only on palettes it can read; `edits` is its one parameter.
fn given(session: &fontcraft_engine::Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    color_palette::read(font, None).map_err(|e| format!("the font's Color Palettes cannot be edited: {e}"))?;
    let mut out = Map::new();
    for (key, value) in p {
        if key != EDITS {
            return Err(format!("unknown parameter `{key}`: the editor takes `{EDITS}`"));
        }
        out.insert(key.clone(), value.clone());
    }
    Ok(out)
}

fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("label".into(), json!(LABEL));
    out.insert("steps".into(), v.get(EDITS).cloned().unwrap_or_else(|| json!([])));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

// ── Edits ───────────────────────────────────────────────────────────────────────────────────────

/// The parameters each palette command takes here.
fn allowed(command: &str) -> Option<&'static [&'static str]> {
    let keys: &'static [&'static str] = match command {
        "colorPalette.addColor" => &["color", "colors"],
        "colorPalette.removeColor" => &["index"],
        "colorPalette.setColor" => &["palette", "index", "color"],
        "colorPalette.addPalette" => &["copyFrom"],
        "colorPalette.removePalette" => &["palette"],
        _ => return None,
    };
    Some(keys)
}

fn colour(params: &Map<String, Value>, key: &str) -> Result<Option<Rgba>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().and_then(Rgba::parse_hex).map(Some).ok_or_else(|| format!("`{key}` must be a colour written #RRGGBB or #RRGGBBAA")),
    }
}

fn number(params: &Map<String, Value>, key: &str) -> Result<Option<usize>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).map(Some).ok_or_else(|| format!("`{key}` must be a whole number from 0")),
    }
}

fn required(params: &Map<String, Value>, key: &str) -> Result<usize, String> {
    number(params, key)?.ok_or_else(|| format!("`{key}` is required"))
}

/// `palettes` after one edit, as the command would leave them.
fn apply(palettes: &Palettes, command: &str, params: &Map<String, Value>) -> Result<Palettes, String> {
    let keys = allowed(command).ok_or_else(|| format!("`{command}` is not a palette edit"))?;
    if let Some(key) = params.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("`{command}` takes no `{key}` here"));
    }
    let next = match command {
        "colorPalette.addColor" => match (colour(params, "color")?, params.get("colors").filter(|v| !v.is_null())) {
            (Some(_), Some(_)) => return Err("give `color` or `colors`, not both".into()),
            (_, Some(list)) => {
                let list = list.as_array().ok_or("`colors` must be a list of colours")?;
                let colours = list
                    .iter()
                    .map(|c| c.as_str().and_then(Rgba::parse_hex).ok_or("`colors` must be a list of colours written #RRGGBB or #RRGGBBAA"))
                    .collect::<Result<Vec<_>, _>>()?;
                palettes.add_row_with(&colours)
            }
            (c, None) => palettes.add_row(c.unwrap_or(Rgba::BLACK)),
        },
        "colorPalette.removeColor" => palettes.remove_row(required(params, "index")?),
        "colorPalette.setColor" => {
            let color = colour(params, "color")?.ok_or("`color` is required")?;
            palettes.set(number(params, "palette")?.unwrap_or(0), required(params, "index")?, color)
        }
        "colorPalette.addPalette" => match number(params, "copyFrom")? {
            None => palettes.add_palette(),
            Some(from) => palettes.add_palette_from(from),
        },
        _ => palettes.remove_palette(required(params, "palette")?),
    };
    next.map_err(|e| e.to_string())
}

/// An edit's command and parameters.
fn parts(edit: &Value) -> Result<(&str, Map<String, Value>), String> {
    let command = edit.get("command").and_then(Value::as_str).ok_or("an edit needs `command`")?;
    let params = match edit.get("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("`params` must be an object".into()),
    };
    Ok((command, params))
}

/// `palettes` after every edit of `edits`, or why one does not apply.
fn replay(mut palettes: Palettes, edits: &[Value]) -> Result<Palettes, String> {
    for (i, edit) in edits.iter().enumerate() {
        let (command, params) = parts(edit).map_err(|e| format!("edit {i}: {e}"))?;
        palettes = apply(&palettes, command, &params).map_err(|e| format!("edit {i} (`{command}`): {e}"))?;
    }
    Ok(palettes)
}

/// The `edits` value if every edit applies to `palettes` in turn, in the form OK runs.
pub fn check(key: &str, palettes: &[Vec<String>], value: &Value) -> Result<Value, String> {
    let list = value.as_array().ok_or_else(|| format!("`{key}` must be a list of {{command, params}} edits"))?;
    if list.len() > MAX_EDITS {
        return Err(format!("`{key}` holds {} edits; at most {MAX_EDITS}", list.len()));
    }
    let start = parse_table(palettes).map_err(|e| format!("`{key}`: {e}"))?;
    replay(start, list).map_err(|e| format!("`{key}`: {e}"))?;
    let normal: Vec<Value> = list.iter().filter_map(|e| parts(e).ok()).map(|(c, p)| json!({"command": c, "params": p})).collect();
    Ok(Value::Array(normal))
}

/// `edits` with `edit` added; a colour for the cell the last edit coloured replaces that edit.
fn added(mut edits: Vec<Value>, edit: Value) -> Vec<Value> {
    let cell = |e: &Value| {
        (e.get("command").and_then(Value::as_str) == Some("colorPalette.setColor")).then(|| {
            let p = e.get("params");
            (p.and_then(|p| p.get("palette")).and_then(Value::as_u64).unwrap_or(0), p.and_then(|p| p.get("index")).and_then(Value::as_u64))
        })
    };
    if let (Some(new), Some(old)) = (cell(&edit), edits.last().and_then(cell))
        && new == old
    {
        edits.pop();
    }
    edits.push(edit);
    edits
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

/// The id of an editor control: `addColor`, `removeColor`, `gear`, `addPalette`.
pub fn control_id(name: &str) -> Id {
    widgets::id(("dialog.colorPalettes", name))
}

/// The id of the swatch of colour `row` in palette `palette`.
pub fn swatch_id(palette: usize, row: usize) -> Id {
    widgets::id(("dialog.colorPalettes", "color", palette, row))
}

/// The id of row `row`'s number, which selects the row.
pub fn row_id(row: usize) -> Id {
    widgets::id(("dialog.colorPalettes", "row", row))
}

/// The id of palette `palette`'s heading and of its Remove Palette item.
pub fn heading_id(palette: usize) -> Id {
    widgets::id(("dialog.colorPalettes", "palette", palette))
}

pub fn remove_palette_id(palette: usize) -> Id {
    widgets::id(("dialog.colorPalettes", "removePalette", palette))
}

/// The id under which the selected row is remembered (egui memory: widget state).
fn selected_id() -> Id {
    widgets::id(("dialog.colorPalettes", "selected"))
}

/// A cell-wide button with a stable id: a palette heading or a row number.
fn cell_button(ui: &mut egui::Ui, id: Id, label: &str, width: f32, selected: bool, tokens: &Tokens) -> Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ui.spacing().interact_size.y), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected || response.hovered() {
            let fill = if selected { tokens.row_selected } else { visuals.weak_bg_fill };
            ui.painter().rect(rect, visuals.corner_radius, fill, visuals.bg_stroke, StrokeKind::Inside);
        }
        let font = egui::TextStyle::Button.resolve(ui.style());
        let colour = if selected { tokens.selected_text } else { tokens.text };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, colour);
    }
    response
}

/// Draws the staged table and its buttons. A change becomes one `Event::Set` of the whole edit
/// list, with the new edit added.
pub fn draw(ui: &mut egui::Ui, field: &Field, palettes: &[Vec<String>], value: &Value, tokens: &Tokens, events: &mut Vec<Event>) -> Response {
    let edits: Vec<Value> = value.as_array().cloned().unwrap_or_default();
    let staged = parse_table(palettes).and_then(|p| replay(p, &edits));
    let mut new_edit: Option<Value> = None;
    let response = ui
        .vertical(|ui| {
            let staged = match staged {
                Ok(s) => s,
                Err(e) => {
                    ui.colored_label(tokens.error_text, e);
                    return;
                }
            };
            let rows = staged.rows();
            let count = staged.palettes().len();
            let ctx = ui.ctx().clone();
            let selected: Option<usize> = ctx.data(|d| d.get_temp::<usize>(selected_id())).filter(|r| *r < rows);
            let side = ui.spacing().interact_size.y;
            let column = side * COLUMN_WIDTH;
            let index_width = side * 2.0;
            if count == 0 {
                ui.label(egui::RichText::new("No palettes yet: + adds the first colour and the first palette.").color(tokens.text_dim));
            } else {
                egui::ScrollArea::both().id_salt(control_id("table")).max_height(side * (VISIBLE_ROWS + 1.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(index_width + ui.spacing().item_spacing.x);
                        for p in 0..count {
                            let heading = cell_button(ui, heading_id(p), &p.to_string(), column, false, tokens).on_hover_text(format!("Palette {p}"));
                            Popup::menu(&heading).id(heading_id(p).with("menu")).show(|ui| {
                                if widgets::button(ui, remove_palette_id(p), "Remove Palette", "Remove this palette", count > 1, false) {
                                    new_edit = Some(json!({"command": "colorPalette.removePalette", "params": {"palette": p}}));
                                    ui.close();
                                }
                            });
                        }
                    });
                    for row in 0..rows {
                        ui.horizontal(|ui| {
                            let number = cell_button(ui, row_id(row), &row.to_string(), index_width, selected == Some(row), tokens);
                            if number.on_hover_text(format!("Colour {row}: click to select the row")).clicked() {
                                ctx.data_mut(|d| d.insert_temp(selected_id(), row));
                            }
                            for (p, palette) in staged.palettes().iter().enumerate() {
                                let current: Option<Rgba8> = palette.get(row).map(|c| [c.r, c.g, c.b, c.a]);
                                let hover = format!("Palette {p}, colour {row}");
                                if let Some(c) = attributes::color_button(ui, swatch_id(p, row), current, &hover, tokens) {
                                    new_edit = Some(json!({"command": "colorPalette.setColor", "params": {"palette": p, "index": row, "color": attributes::hex(c)}}));
                                }
                            }
                        });
                    }
                });
            }
            ui.horizontal(|ui| {
                if widgets::button(ui, control_id("addColor"), "+", "Add a colour row (black) to every palette", true, false) {
                    new_edit = Some(json!({"command": "colorPalette.addColor", "params": {}}));
                }
                if widgets::button(ui, control_id("removeColor"), "−", "Remove the selected colour row from every palette", selected.is_some(), false)
                    && let Some(row) = selected
                {
                    new_edit = Some(json!({"command": "colorPalette.removeColor", "params": {"index": row}}));
                    ctx.data_mut(|d| d.remove::<usize>(selected_id()));
                }
                let gear = attributes::menu_button(ui, control_id("gear"), "⚙", "Palette options");
                Popup::menu(&gear).id(control_id("gear").with("menu")).show(|ui| {
                    if widgets::button(ui, control_id("addPalette"), "Add Palette", "Add a palette holding a copy of palette 0", true, false) {
                        new_edit = Some(json!({"command": "colorPalette.addPalette", "params": {}}));
                        ui.close();
                    }
                });
            });
        })
        .response;
    if let Some(edit) = new_edit {
        events.push(Event::Set(field.key.clone(), Value::Array(added(edits, edit))));
    }
    response
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use egui::{Event as Input, Modifiers, PointerButton, Pos2};
    use serde_json::{Value, json};

    use super::*;
    use crate::control;

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn palettes(app: &mut FontcraftApp) -> Value {
        app.session.execute("colorPalette.list", &json!({})).unwrap()["palettes"].clone()
    }

    fn undo_steps(app: &mut FontcraftApp) -> usize {
        call(app, "document.inspect", json!({}))["documents"][0]["undo"].as_array().map_or(0, Vec::len)
    }

    /// An app whose font has one palette of two colours.
    fn app_with_palette() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("colorPalette.addColor", &json!({"color": "#FF0000"})).unwrap();
        app.run("colorPalette.addColor", &json!({"color": "#00FF00"})).unwrap();
        app
    }

    fn edits() -> Value {
        json!([
            {"command": "colorPalette.addPalette", "params": {}},
            {"command": "colorPalette.setColor", "params": {"palette": 1, "index": 0, "color": "#0000FF"}},
            {"command": "colorPalette.addColor", "params": {"color": "#FFFFFF80"}},
        ])
    }

    #[test]
    fn color_palette_edit_over_the_control_channel_equals_the_commands_as_one_undo_step() {
        let mut app = app_with_palette();
        app.run("colorPalette.edit", &json!({})).unwrap();
        let opened = call(&mut app, "ui.dialog.inspect", json!({}));
        assert_eq!(opened["dialog"], "dialog.colorPalettes");
        assert_eq!(opened["mode"], "runOnOk");
        assert_eq!(opened["fields"][0]["palettes"], json!([["#FF0000FF", "#00FF00FF"]]));
        call(&mut app, "ui.dialog.set", json!({"field": EDITS, "value": edits()}));
        assert_eq!(palettes(&mut app), json!([["#FF0000FF", "#00FF00FF"]]), "nothing changes before OK");
        let undo_before = undo_steps(&mut app);
        let r = call(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(r["command"], "command.batch");
        assert_eq!(undo_steps(&mut app), undo_before + 1, "OK is one undo step");

        let mut by_commands = app_with_palette();
        for step in edits().as_array().unwrap() {
            by_commands.run(step["command"].as_str().unwrap(), &step["params"]).unwrap();
        }
        assert_eq!(palettes(&mut app), palettes(&mut by_commands));
        assert_eq!(palettes(&mut app), json!([["#FF0000FF", "#00FF00FF", "#FFFFFF80"], ["#0000FFFF", "#00FF00FF", "#FFFFFF80"]]));

        app.run("edit.undo", &json!({})).unwrap();
        assert_eq!(palettes(&mut app), json!([["#FF0000FF", "#00FF00FF"]]), "one undo takes back every edit");
    }

    #[test]
    fn color_palette_edit_cancel_changes_nothing() {
        let mut app = app_with_palette();
        let (before, steps) = (palettes(&mut app), undo_steps(&mut app));
        app.run("colorPalette.edit", &json!({"edits": edits()})).unwrap();
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(app.dialog.is_none());
        assert_eq!((palettes(&mut app), undo_steps(&mut app)), (before, steps));
    }

    #[test]
    fn edits_that_do_not_apply_are_refused_when_set() {
        let mut app = app_with_palette();
        app.run("colorPalette.edit", &json!({})).unwrap();
        for bad in [
            json!([{"command": "colorPalette.removePalette", "params": {"palette": 0}}]),
            json!([{"command": "colorPalette.setColor", "params": {"index": 9, "color": "#000000"}}]),
            json!([{"command": "colorPalette.addColor", "params": {"color": "red"}}]),
            json!([{"command": "colorPalette.addColor", "params": {"master": "m01"}}]),
            json!([{"command": "glyph.newGlyph", "params": {"name": "x"}}]),
            json!({"command": "colorPalette.addColor"}),
        ] {
            let r = control::handle(&mut app, &json!({"id": 1, "method": "ui.dialog.set", "params": {"field": EDITS, "value": bad}}));
            assert_eq!(r["ok"], false, "{bad}");
        }
    }

    #[test]
    fn dialog_color_palettes_plus_and_add_palette_stage_edits_that_ok_applies() {
        let mut app = app_with_palette();
        app.run("colorPalette.edit", &json!({})).unwrap();
        let ctx = egui::Context::default();
        let frame = |app: &mut FontcraftApp, events: Vec<Input>| {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| app.ui(ui));
            out.textures_delta.clear();
        };
        let click = |app: &mut FontcraftApp, id: egui::Id| {
            let at = ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
            let button = |pressed| Input::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            frame(app, vec![Input::PointerMoved(at)]);
            frame(app, vec![button(true)]);
            frame(app, vec![button(false)]);
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        click(&mut app, control_id("addColor"));
        click(&mut app, control_id("gear"));
        click(&mut app, control_id("addPalette"));
        click(&mut app, row_id(0));
        click(&mut app, control_id("removeColor"));
        let staged = app.dialog.as_ref().unwrap().values[EDITS].clone();
        assert_eq!(
            staged,
            json!([
                {"command": "colorPalette.addColor", "params": {}},
                {"command": "colorPalette.addPalette", "params": {}},
                {"command": "colorPalette.removeColor", "params": {"index": 0}},
            ])
        );
        assert_eq!(palettes(&mut app), json!([["#FF0000FF", "#00FF00FF"]]), "the font waits for OK");
        call(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(palettes(&mut app), json!([["#00FF00FF", "#000000FF"], ["#00FF00FF", "#000000FF"]]));
    }

    #[test]
    fn a_second_colour_for_the_same_cell_replaces_the_first_edit() {
        let set = |c: &str| json!({"command": "colorPalette.setColor", "params": {"palette": 0, "index": 1, "color": c}});
        let edits = added(added(Vec::new(), set("#111111")), set("#222222"));
        assert_eq!(edits, vec![set("#222222")]);
        let other = json!({"command": "colorPalette.setColor", "params": {"palette": 0, "index": 0, "color": "#333333"}});
        assert_eq!(added(edits, other).len(), 2);
    }
}

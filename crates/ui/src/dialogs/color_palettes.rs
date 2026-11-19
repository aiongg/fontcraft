//! The colour palettes editor: a staged copy of the palettes; OK runs the edits as one `command.batch`, Cancel changes
//! nothing (rows `colorPalette.edit`, `dialog.colorPalettes`, `color.palette.editorIsTransactional`).
//!
//! `colorPalette.edit` opens it on the font's `Color Palettes`. Its one field, `edits`, is the list of palette commands
//! made so far, each `{command, params}` exactly as an agent would run it: `colorPalette.addColor`, `removeColor`,
//! `setColor`, `addPalette`, `removePalette`, without `master`. The table shows the palettes as they will be after
//! those edits: one row per colour index and one column per palette.
//!
//! | Control | Edit added |
//! |---|---|
//! | a cell, then a colour typed in Colour | `colorPalette.setColor` (a second colour for the same cell replaces the first edit) |
//! | Add Color | `colorPalette.addColor` (opaque black) |
//! | Remove Color | `colorPalette.removeColor` of the selected cell's row |
//! | Add Palette | `colorPalette.addPalette` (a copy of palette 0) |
//! | Remove Palette | `colorPalette.removePalette` of the selected cell's palette (not the last palette) |
//!
//! Nothing touches the font while the editor is open (run on OK). OK runs `command.batch {label: "Color Palettes",
//! steps: edits}`, one undo step; a step the font refuses (removing a colour a layer uses) keeps the editor open with
//! the message. Over the control channel: `ui.dialog.set {field: "edits", value: [...]}`, then `ui.dialog.confirm` or
//! `ui.dialog.cancel`.

use craft::ui::buttons::Button;
use craft::ui::choice::SwatchTable;
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use egui::{Response, Ui};
use fontcraft_engine::color::palette::{self as color_palette, Palettes, Rgba};
use serde_json::{Map, Value, json};

use super::fields::{Custom, Field, Kind};
use super::spec::{DialogSpec, Fields, Shaped};
use crate::app::FontCraftApp;
use crate::panels::attributes::{color_field, srgba};
use crate::shell::commands::{Module, UiCommand};

/// Most edits one session of the editor holds.
pub const MAX_EDITS: usize = 10_000;
/// The field holding the edits.
pub const EDITS: &str = "edits";
/// The undo label of OK's batch.
const LABEL: &str = "Color Palettes";

craft::ui::words! {
    /// The words of the colour palettes editor.
    pub struct PaletteWords {
        table: "fontcraftColorPalettes.table" = "Palettes";
        cell: "fontcraftColorPalettes.cell" = "Palette {palette}, colour {index}";
        color: "fontcraftColorPalettes.color" = "Colour";
        add_color: "fontcraftColorPalettes.addColor" = "Add Color";
        remove_color: "fontcraftColorPalettes.removeColor" = "Remove Color";
        add_palette: "fontcraftColorPalettes.addPalette" = "Add Palette";
        remove_palette: "fontcraftColorPalettes.removePalette" = "Remove Palette";
    }
}

pub const SPECS: &[DialogSpec] = &[DialogSpec {
    // Opened by `colorPalette.edit`, never as "the dialog of `command.batch`".
    follow_up: true,
    ..DialogSpec::new("dialog.colorPalettes", "Color Palettes", "command.batch", Fields::Shaped(Shaped { fields, given, params, link })).run_on_ok()
}];

/// The palettes table: the field's `data` holds the palettes at open as `#RRGGBBAA` lists, one per palette.
pub const PALETTES_KIND: Custom = Custom { name: "palettes", check, row: Some(row) };

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "colorPalette.edit",
    label: "Edit Color Palettes",
    menu: &[],
    shortcut: None,
    params: "{edits?: [{command: \"colorPalette.addColor\" | \"colorPalette.removeColor\" | \"colorPalette.setColor\" | \"colorPalette.addPalette\" | \"colorPalette.removePalette\", params: {…} (that command's parameters without `master`)}]} opens the editor of the font's Color Palettes with these edits staged. Nothing changes until OK, which runs the edits as one command.batch (one undo step); Cancel drops them",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run: run_command };

fn run_command(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let mut given = Map::new();
    if let Some(edits) = command.params.get(EDITS) {
        given.insert(EDITS.into(), edits.clone());
    }
    let spec = SPECS.first().ok_or("the editor has no dialog")?;
    super::form::open(app, spec, &given).map(drop)
}

/// The palettes as the field keeps them: `#RRGGBBAA` per colour, one list per palette.
pub fn table(palettes: &Palettes) -> Value {
    json!(palettes.palettes().iter().map(|p| p.iter().map(Rgba::hex).collect::<Vec<_>>()).collect::<Vec<_>>())
}

fn parse_table(table: &Value) -> Result<Palettes, String> {
    let lists = table.as_array().ok_or("the palettes are not a list")?;
    let rows = lists
        .iter()
        .map(|p| {
            let colours = p.as_array().ok_or("a palette is not a list")?;
            colours
                .iter()
                .map(|c| c.as_str().and_then(Rgba::parse_hex).ok_or_else(|| format!("`{c}` is not a colour")))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Palettes::new(rows).map_err(|e| e.to_string())
}

fn fields(session: &fontcraft_engine::Session) -> Vec<Field> {
    let palettes = session.font().ok().and_then(|f| color_palette::read(f, None).ok()).unwrap_or_default();
    vec![Field::custom(
        EDITS,
        "",
        PALETTES_KIND,
        table(&palettes),
        json!([]),
        "The palette edits OK runs, in order: [{command, params}] with the colorPalette commands' parameters (no `master`)",
    )]
}

/// The editor opens only on palettes it can read; `edits` is its one parameter.
fn given(session: &fontcraft_engine::Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    color_palette::read(font, None).map_err(|e| format!("the font's Color Palettes cannot be edited: {e}"))?;
    if let Some(key) = p.keys().find(|k| *k != EDITS) {
        return Err(format!("unknown parameter `{key}`: the editor takes `{EDITS}`"));
    }
    Ok(p.clone())
}

fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("label".into(), json!(LABEL));
    out.insert("steps".into(), v.get(EDITS).cloned().unwrap_or_else(|| json!([])));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

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

/// The palettes the field opened on.
fn start(field: &Field) -> Result<Palettes, String> {
    match &field.kind {
        Kind::Custom { data, .. } => parse_table(data),
        _ => Err("the field holds no palettes".into()),
    }
}

/// The `edits` value if every edit applies to the palettes in turn, in the form OK runs.
fn check(field: &Field, value: &Value) -> Result<Value, String> {
    let key = &field.key;
    let list = value.as_array().ok_or_else(|| format!("`{key}` must be a list of {{command, params}} edits"))?;
    if list.len() > MAX_EDITS {
        return Err(format!("`{key}` holds {} edits; at most {MAX_EDITS}", list.len()));
    }
    let start = start(field).map_err(|e| format!("`{key}`: {e}"))?;
    replay(start, list).map_err(|e| format!("`{key}`: {e}"))?;
    let normal: Vec<Value> = list.iter().filter_map(|e| parts(e).ok()).map(|(c, p)| json!({"command": c, "params": p})).collect();
    Ok(Value::Array(normal))
}

/// `edits` with `edit` added; a colour for the cell the last edit coloured replaces that edit.
pub fn added(mut edits: Vec<Value>, edit: Value) -> Vec<Value> {
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

/// The table and its buttons: the palettes after the staged edits, cells row by row (colour index), one column per
/// palette.
fn row(ui: &mut Ui, rows: &mut DialogFields, _: &mut CustomContext, field: &Field) -> Response {
    let words = Env::of(ui).words::<PaletteWords>();
    let edits: Vec<Value> = match rows.get(&field.key) {
        Some(FieldValue::Json(Value::Array(list))) => list.clone(),
        _ => Vec::new(),
    };
    let palettes = start(field).and_then(|p| replay(p, &edits)).unwrap_or_default();
    let count = palettes.palettes().len().max(1);
    let colours = palettes.rows();
    let mut cells: Vec<(Label, craft::theme::Srgba)> = Vec::with_capacity(count * colours);
    for index in 0..colours {
        for palette in 0..count {
            let c = palettes.get(palette, index).unwrap_or(Rgba::BLACK);
            let name = words.cell.as_str().replace("{palette}", &palette.to_string()).replace("{index}", &index.to_string());
            cells.push((Label::from(name), srgba([c.r, c.g, c.b, c.a])));
        }
    }
    let mut edit = None;
    let shown = Stack::new().show(ui, |ui, _| {
        let out = SwatchTable::new(words.table.clone(), &cells).per_row(count).wells(true).tips(true).show_all(ui);
        let chosen = out.chosen.first().map(|at| (at % count, at / count));
        if let Some((palette, index)) = chosen {
            let c = palettes.get(palette, index).map(|c| [c.r, c.g, c.b, c.a]);
            if let Some(picked) = color_field(ui, &words.color, c, ("fontcraftColorPalettes.color", palette, index)) {
                let hex = crate::app::models::color::hex(picked);
                edit = Some(json!({"command": "colorPalette.setColor", "params": {"palette": palette, "index": index, "color": hex}}));
            }
        }
        let _ = Row::new().wrap(true).show(ui, |ui, _| {
            if Button::new(words.add_color.clone()).show(ui).clicked() {
                edit = Some(json!({"command": "colorPalette.addColor", "params": {}}));
            }
            if let Some((_, index)) = chosen
                && Button::new(words.remove_color.clone()).show(ui).clicked()
            {
                edit = Some(json!({"command": "colorPalette.removeColor", "params": {"index": index}}));
            }
            if Button::new(words.add_palette.clone()).show(ui).clicked() {
                edit = Some(json!({"command": "colorPalette.addPalette", "params": {"copyFrom": 0}}));
            }
            if let Some((palette, _)) = chosen.filter(|_| count > 1)
                && Button::new(words.remove_palette.clone()).show(ui).clicked()
            {
                edit = Some(json!({"command": "colorPalette.removePalette", "params": {"palette": palette}}));
            }
        });
    });
    if let Some(edit) = edit {
        let next = added(edits, edit);
        if next.len() <= MAX_EDITS {
            rows.set(field.key.clone(), FieldValue::Json(Value::Array(next)));
        }
    }
    shown.output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use fontcraft_engine::color::palette as color_palette;
    use serde_json::json;

    use super::added;
    use crate::app::{APP, FontCraftApp, Msg};
    use crate::dialogs::form;

    fn palettes(app: &FontCraftApp) -> Vec<Vec<String>> {
        let p = color_palette::read(app.session.font().unwrap(), None).unwrap_or_default();
        p.palettes().iter().map(|l| l.iter().map(|c| c.hex()).collect()).collect()
    }

    /// Nothing changes while the editor is open; OK runs the staged edits as one undo step, and an edit that does not
    /// apply is refused.
    #[test]
    fn the_editor_stages_edits_and_runs_them_on_ok() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        for c in ["#FF0000", "#00FF00"] {
            app.run_now(Msg::run("colorPalette.addColor", json!({"color": c})), false).unwrap();
        }
        let before = palettes(&app);
        assert_eq!(before, vec![vec!["#FF0000FF".to_string(), "#00FF00FF".to_string()]]);
        app.run_now(Msg::Shell(CommandRef::with_params("colorPalette.edit", json!({}))), false).unwrap();
        let edits = json!([
            {"command": "colorPalette.setColor", "params": {"palette": 0, "index": 0, "color": "#0000FF"}},
            {"command": "colorPalette.addPalette", "params": {}},
        ]);
        form::set(&mut app, "edits", &edits).unwrap();
        assert_eq!(palettes(&app), before, "nothing runs while open");
        assert!(form::set(&mut app, "edits", &json!([{"command": "colorPalette.removeColor", "params": {"index": 9}}])).is_err());
        form::confirm(&mut app).unwrap();
        let after = palettes(&app);
        assert_eq!(after.len(), 2, "{after:?}");
        assert_eq!(after[0][0], "#0000FFFF");
    }

    #[test]
    fn a_second_colour_for_the_same_cell_replaces_the_first() {
        let set = |c: &str| json!({"command": "colorPalette.setColor", "params": {"palette": 0, "index": 1, "color": c}});
        let edits = added(added(Vec::new(), set("#FF0000FF")), set("#00FF00FF"));
        assert_eq!(edits, vec![set("#00FF00FF")]);
    }
}

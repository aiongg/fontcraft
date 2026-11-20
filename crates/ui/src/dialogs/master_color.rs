//! The Master Color picker of layered colour fonts (row `dialog.masterColorPicker`).
//!
//! It edits the value of a master's `Master Color` or `Master Color Dark` parameter: a swatch beside its `#RRGGBBAA`
//! text, so a semi-transparent preview colour shows stacked layers through each other. Open it with
//! `dialog.open {dialog: "dialog.masterColorPicker", params: {targets, name?, color?}}`: `targets` names the masters
//! as `font.customParameters.set` takes them (id, name or index, or a list; the only master when the font has one),
//! `name` is `Master Color` (default) or `Master Color Dark`, and every target must already have that parameter. The
//! field `color` (`#RRGGBBAA`) starts at the first target's colour.
//!
//! Each colour previews `font.customParameters.set {scope: "master", targets, name, value}` on the font; OK keeps it
//! as one undo step and Cancel puts the font back. The value is stored as the text `R,G,B,A` with components from 0
//! to 255.

use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue};
use craft::ui::runtime::env::Label;
use egui::{Response, Ui};
use fontcraft_engine::model::plist::Value as Plist;
use serde_json::{Map, Value, json};

use super::fields::{Custom, Field};
use super::spec::{DialogSpec, Fields, Shaped};
use crate::app::models::color::{Rgba8, hex, parse_hex};
use crate::canvas::color::MASTER_COLOR_DARK;
use crate::panels::attributes::color_cell;

/// The parameter the picker edits by default.
pub const MASTER_COLOR: &str = "Master Color";
/// The colour field's key.
pub const COLOR: &str = "color";
/// The colour the field starts at when the parameter's value cannot be read.
const START: Rgba8 = [0, 0, 0, 255];
/// Most masters one picker sets.
const MAX_TARGETS: usize = 4096;

pub const SPECS: &[DialogSpec] = &[DialogSpec {
    // Opened from a master's parameter, never as "the dialog of `font.customParameters.set`".
    follow_up: true,
    ..DialogSpec::new("dialog.masterColorPicker", "Master Color", "font.customParameters.set", Fields::Shaped(Shaped { fields, given, params, link }))
}];

/// A colour field: `#RRGGBBAA` text, shown as a swatch beside its text.
pub const COLOR_KIND: Custom = Custom { name: "color", check, row: Some(row) };

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![Field::custom(
        COLOR,
        "Color",
        COLOR_KIND,
        Value::Null,
        json!(hex(START)),
        "The preview colour of the masters' filled outlines, #RRGGBBAA; the alpha is the opacity",
    )]
}

/// A stored Master Color: `R,G,B,A` (0 to 255; the alpha may be left out), the same in parentheses, a list of those
/// numbers, or `#RRGGBB[AA]`.
pub fn parse_stored(value: &Plist) -> Option<Rgba8> {
    let numbers: Vec<f64> = match value {
        Plist::String(s) => {
            let text = s.as_str().trim();
            if text.starts_with('#') {
                return parse_hex(text);
            }
            let inner = text.trim_start_matches('(').trim_end_matches(')');
            inner.split(',').take(5).map(|n| n.trim().parse::<f64>().ok()).collect::<Option<Vec<_>>>()?
        }
        Plist::Array(a) => a.items.iter().take(5).map(|v| v.as_num().map(|n| n.as_f64())).collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    let byte = |i: usize, default: f64| -> Option<u8> {
        let n = numbers.get(i).copied().unwrap_or(default);
        (n.is_finite() && (0.0..=255.0).contains(&n)).then(|| n.round() as u8)
    };
    match numbers.len() {
        3 | 4 => Some([byte(0, 0.0)?, byte(1, 0.0)?, byte(2, 0.0)?, byte(3, 255.0)?]),
        _ => None,
    }
}

/// `c` as the parameter stores it.
pub fn stored([r, g, b, a]: Rgba8) -> String {
    format!("{r},{g},{b},{a}")
}

fn resolve<'f>(font: &'f fontcraft_engine::model::Font, target: &Value) -> Option<&'f fontcraft_engine::model::Master> {
    match target {
        Value::String(s) => font.masters.iter().find(|m| m.id == *s).or_else(|| font.masters.iter().find(|m| m.name == *s)),
        Value::Number(n) => n.as_u64().and_then(|i| usize::try_from(i).ok()).and_then(|i| font.masters.get(i)),
        _ => None,
    }
}

fn given(session: &fontcraft_engine::Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    if let Some(key) = p.keys().find(|k| !matches!(k.as_str(), "targets" | "name" | COLOR)) {
        return Err(format!("unknown parameter `{key}`: the picker takes targets, name and color"));
    }
    let name = match p.get("name") {
        None | Some(Value::Null) => MASTER_COLOR,
        Some(Value::String(s)) if s == MASTER_COLOR => MASTER_COLOR,
        Some(Value::String(s)) if s == MASTER_COLOR_DARK => MASTER_COLOR_DARK,
        Some(Value::String(_)) => return Err(format!("`name` must be {MASTER_COLOR} or {MASTER_COLOR_DARK}")),
        Some(_) => return Err("`name` must be text".into()),
    };
    let targets: Vec<Value> = match p.get("targets") {
        None | Some(Value::Null) => match font.masters.as_slice() {
            [only] => vec![json!(only.id)],
            _ => return Err("give `targets`: the masters whose colour to set".into()),
        },
        Some(Value::Array(list)) if list.len() <= MAX_TARGETS => list.clone(),
        Some(Value::Array(_)) => return Err(format!("`targets` names at most {MAX_TARGETS} masters")),
        Some(one) => vec![one.clone()],
    };
    let mut masters = Vec::with_capacity(targets.len());
    for t in &targets {
        let m = resolve(font, t).ok_or_else(|| format!("there is no master {t}"))?;
        if m.params.get(name).is_none() {
            return Err(format!("the master `{}` has no `{name}` parameter: add it first", m.name));
        }
        masters.push(m);
    }
    let first = masters.first().ok_or("give at least one master in `targets`")?;
    let colour = match p.get(COLOR) {
        Some(v) => v.clone(),
        None => json!(hex(first.params.get(name).and_then(parse_stored).unwrap_or(START))),
    };
    let mut out = Map::new();
    out.insert("scope".into(), json!("master"));
    out.insert("targets".into(), json!(masters.iter().map(|m| m.id.clone()).collect::<Vec<_>>()));
    out.insert("name".into(), json!(name));
    out.insert(COLOR.into(), colour);
    Ok(out)
}

fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let colour = v.get(COLOR).and_then(Value::as_str).and_then(parse_hex).unwrap_or(START);
    let mut out = Map::new();
    out.insert("value".into(), json!(stored(colour)));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

fn check(field: &Field, value: &Value) -> Result<Value, String> {
    value.as_str().and_then(parse_hex).map(|c| json!(hex(c))).ok_or_else(|| format!("`{}` must be a colour written #RRGGBB or #RRGGBBAA", field.key))
}

fn row(ui: &mut Ui, rows: &mut DialogFields, _: &mut CustomContext, field: &Field) -> Response {
    let current = match rows.get(&field.key) {
        Some(FieldValue::Json(Value::String(s))) => parse_hex(s),
        _ => field.default.as_str().and_then(parse_hex),
    };
    let (picked, response) = color_cell(ui, &Label::from(field.label.clone()), current, ("fontcraftMasterColor", field.key.as_str()));
    if let Some(c) = picked {
        rows.set(field.key.clone(), FieldValue::Json(json!(hex(c))));
    }
    response
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg};
    use crate::dialogs::form;

    fn master_color(app: &FontCraftApp) -> Option<String> {
        app.session.font().unwrap().masters[0].params.get("Master Color").and_then(|v| v.as_str()).map(str::to_string)
    }

    /// A colour previews on the master's parameter; OK keeps it as `R,G,B,A`, Cancel puts the old value back.
    #[test]
    fn the_picker_previews_and_keeps_the_master_color() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let id = app.session.font().unwrap().masters[0].id.clone();
        let add = json!({"scope": "master", "targets": id, "name": "Master Color", "value": "0,0,0,255"});
        app.run_now(Msg::run("font.customParameters.add", add), false).unwrap();
        let open = json!({"dialog": "dialog.masterColorPicker", "params": {"targets": [id]}});
        let shown = form::open_command(&mut app, &open).unwrap();
        assert_eq!(shown["fields"][0]["value"], json!("#000000FF"), "{shown}");
        form::set(&mut app, "color", &json!("#FF000080")).unwrap();
        assert_eq!(master_color(&app).as_deref(), Some("255,0,0,128"), "the colour previews");
        form::cancel(&mut app).unwrap();
        assert_eq!(master_color(&app).as_deref(), Some("0,0,0,255"), "Cancel puts it back");
        form::open_command(&mut app, &open).unwrap();
        form::set(&mut app, "color", &json!("#00FF00")).unwrap();
        form::confirm(&mut app).unwrap();
        assert_eq!(master_color(&app).as_deref(), Some("0,255,0,255"));
        assert!(form::set(&mut app, "color", &json!("green")).is_err(), "no dialog is open");
    }

    /// On Font Info's Masters tab, a Master Color parameter row's Edit button opens the picker on the row's masters,
    /// at the stored colour.
    #[test]
    fn the_masters_tab_row_opens_the_picker() {
        use fontcraft_engine::fontinfo::Scope;

        use crate::app::models::font_info::{RowDialog, RowModel, row_dialog};
        use crate::panels::font_info::rows::dialog_message;
        use crate::shell::font_info::Target;

        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let add = json!({"scope": "master", "targets": [0], "name": "Master Color", "value": "255,0,0,128"});
        app.run_now(Msg::run("font.customParameters.add", add), false).unwrap();
        assert_eq!(row_dialog("Master Color", Scope::Master), Some(RowDialog::MasterColor));
        assert_eq!(row_dialog("Master Color Dark", Scope::Master), Some(RowDialog::MasterColor));
        let row = RowModel { key: "Master Color".into(), ..RowModel::default() };
        let target = Target { scope: Scope::Master, indices: vec![0] };
        let open = dialog_message(&target, &row, RowDialog::MasterColor).expect("the row opens a dialog");
        app.run_now(open, false).unwrap();
        let shown = form::inspect(&app);
        assert_eq!(shown["dialog"], json!("dialog.masterColorPicker"), "{shown}");
        assert_eq!(shown["fields"][0]["value"], json!("#FF000080"), "{shown}");
    }
}

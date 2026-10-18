//! The Master Color picker of layered colour fonts. Package M11.15 (`docs/plans/M11.md` § 4);
//! row `dialog.masterColorPicker`.
//!
//! It edits the value of a master's `Master Color` or `Master Color Dark` parameter: a colour
//! swatch whose picker has an opacity slider, so a semi-transparent preview colour shows stacked
//! layers through each other. Open it with `dialog.open {dialog: "dialog.masterColorPicker",
//! params: {targets, name?, color?}}`: `targets` names the masters as
//! `font.customParameters.set` takes them (id, name or index, or a list; the only master when the
//! font has one), `name` is `Master Color` (default) or `Master Color Dark`, and every target
//! must already have that parameter, as in Font Info › Masters, where the picker opens from the
//! parameter's value. The field `color` (`#RRGGBBAA`) starts at the first target's colour.
//!
//! Each colour previews `font.customParameters.set {scope: "master", targets, name, value}` on
//! the font; OK keeps it as one undo step and Cancel puts the font back. The value is stored as
//! the text `R,G,B,A` with components from 0 to 255.

use egui::Response;
use fontcraft_model::plist::Value as Plist;
use serde_json::{Map, Value, json};

use super::form::{Field, Kind, widget_id};
use super::{Dialog, Event, Fields, Shaped};
use crate::panels::attributes::{self, Rgba8};
use crate::theme::Tokens;

pub const DIALOG: Dialog = Dialog {
    // Opened from a parameter's value, never as "the dialog of `font.customParameters.set`".
    follow_up: true,
    ..Dialog::new("dialog.masterColorPicker", "Master Color", "font.customParameters.set", Fields::Shaped(Shaped { fields, given, params, link }))
};

/// The parameters the picker edits.
pub const MASTER_COLOR: &str = "Master Color";
pub const MASTER_COLOR_DARK: &str = "Master Color Dark";
/// The field holding the colour.
pub const COLOR: &str = "color";
/// The colour a parameter without a readable value starts at: opaque black.
const START: Rgba8 = [0, 0, 0, 255];
/// Most masters one request names.
const MAX_TARGETS: usize = 4096;

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![Field {
        key: COLOR.into(),
        label: "Color".into(),
        kind: Kind::Color,
        default: json!(attributes::hex(START)),
        doc: "The preview colour of the masters' filled outlines, #RRGGBBAA; the alpha is the opacity".into(),
        group: None,
    }]
}

/// A colour stored as the parameter's value: `R,G,B[,A]` (0 to 255, brackets allowed),
/// `#RRGGBB[AA]`, or a list of three or four numbers.
pub fn parse_stored(value: &Plist) -> Option<Rgba8> {
    let numbers: Vec<f64> = match value {
        Plist::String(s) => {
            let text = s.as_str().trim();
            if text.starts_with('#') {
                return attributes::parse_hex(text);
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

/// The value written for a colour.
pub fn stored(c: Rgba8) -> String {
    let [r, g, b, a] = c;
    format!("{r},{g},{b},{a}")
}

/// The master a target names: its id, its name, or its index.
fn resolve<'f>(font: &'f fontcraft_model::Font, target: &Value) -> Option<&'f fontcraft_model::Master> {
    match target {
        Value::String(s) => font.masters.iter().find(|m| m.id == *s).or_else(|| font.masters.iter().find(|m| m.name == *s)),
        Value::Number(n) => n.as_u64().and_then(|i| usize::try_from(i).ok()).and_then(|i| font.masters.get(i)),
        _ => None,
    }
}

/// The pass-through targets and the first colour. Every target must have the parameter.
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
        if !m.params.0.iter().any(|row| row.name == name) {
            return Err(format!("the master `{}` has no `{name}` parameter: add it first", m.name));
        }
        masters.push(m);
    }
    let first = masters.first().ok_or("give at least one master in `targets`")?;
    let colour = match p.get(COLOR) {
        Some(v) => v.clone(),
        None => json!(attributes::hex(first.params.get(name).and_then(parse_stored).unwrap_or(START))),
    };
    let mut out = Map::new();
    out.insert("scope".into(), json!("master"));
    out.insert("targets".into(), json!(masters.iter().map(|m| m.id.clone()).collect::<Vec<_>>()));
    out.insert("name".into(), json!(name));
    out.insert(COLOR.into(), colour);
    Ok(out)
}

fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let colour = v.get(COLOR).and_then(Value::as_str).and_then(attributes::parse_hex).unwrap_or(START);
    let mut out = Map::new();
    out.insert("value".into(), json!(stored(colour)));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

/// A colour field's value: `#RRGGBB` or `#RRGGBBAA`, kept as `#RRGGBBAA`.
pub fn check(key: &str, value: &Value) -> Result<Value, String> {
    value
        .as_str()
        .and_then(attributes::parse_hex)
        .map(|c| json!(attributes::hex(c)))
        .ok_or_else(|| format!("`{key}` must be a colour written #RRGGBB or #RRGGBBAA"))
}

/// A colour field: the swatch, which opens the picker, and the colour as text.
pub fn draw(ui: &mut egui::Ui, field: &Field, value: &Value, tokens: &Tokens, events: &mut Vec<Event>) -> Response {
    let current = value.as_str().and_then(attributes::parse_hex);
    ui.horizontal(|ui| {
        if let Some(c) = attributes::color_button(ui, widget_id(&field.key), current, &field.doc, tokens) {
            events.push(Event::Set(field.key.clone(), json!(attributes::hex(c))));
        }
        if let Some(c) = current {
            let [_, _, _, a] = c;
            let opacity = (f64::from(a) * 100.0 / 255.0).round();
            ui.label(egui::RichText::new(format!("{}  Opacity {opacity} %", attributes::hex(c))).color(tokens.text_dim));
        }
    })
    .response
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use serde_json::{Value, json};

    use super::*;
    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Result<Value, String> {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        if r["ok"] == true { Ok(r["result"].clone()) } else { Err(r["error"].as_str().unwrap_or_default().to_string()) }
    }

    fn master_color(app: &FontcraftApp) -> Option<String> {
        app.session.font().unwrap().masters[0].params.get("Master Color").and_then(|v| v.as_str()).map(str::to_string)
    }

    fn app_with_parameter() -> (FontcraftApp, String) {
        let mut app = FontcraftApp::default();
        let id = app.session.font().unwrap().masters[0].id.clone();
        app.run("font.customParameters.add", &json!({"scope": "master", "targets": id, "name": "Master Color", "value": "0,0,0,255"})).unwrap();
        (app, id)
    }

    fn open(app: &mut FontcraftApp, id: &str) -> Result<Value, String> {
        app.run("dialog.open", &json!({"dialog": "dialog.masterColorPicker", "params": {"targets": [id]}}))
    }

    #[test]
    fn dialog_master_color_picker_over_the_control_channel_equals_the_command() {
        let (mut app, id) = app_with_parameter();
        let opened = open(&mut app, &id).unwrap();
        assert_eq!(opened["fields"][0]["value"], "#000000FF", "the picker starts at the stored colour");
        call(&mut app, "ui.dialog.set", json!({"field": COLOR, "value": "#ff000080"})).unwrap();
        let r = call(&mut app, "ui.dialog.confirm", json!({})).unwrap();
        assert_eq!(master_color(&app).as_deref(), Some("255,0,0,128"));

        let (mut direct, _) = app_with_parameter();
        direct.run(r["command"].as_str().unwrap(), &r["params"]).unwrap();
        assert_eq!(master_color(&direct), master_color(&app));
    }

    #[test]
    fn dialog_master_color_picker_cancel_puts_the_font_back() {
        let (mut app, id) = app_with_parameter();
        open(&mut app, &id).unwrap();
        call(&mut app, "ui.dialog.set", json!({"field": COLOR, "value": "#00FF00"})).unwrap();
        assert_eq!(master_color(&app).as_deref(), Some("0,255,0,255"), "the colour previews");
        call(&mut app, "ui.dialog.cancel", json!({})).unwrap();
        assert_eq!(master_color(&app).as_deref(), Some("0,0,0,255"));
    }

    #[test]
    fn the_picker_needs_the_parameter_and_a_colour() {
        let mut app = FontcraftApp::default();
        let id = app.session.font().unwrap().masters[0].id.clone();
        assert!(open(&mut app, &id).unwrap_err().contains("add it first"));
        let (mut app, id) = app_with_parameter();
        open(&mut app, &id).unwrap();
        assert!(call(&mut app, "ui.dialog.set", json!({"field": COLOR, "value": "red"})).is_err());
    }

    #[test]
    fn stored_colours_parse_in_their_forms() {
        let text = |s: &str| Plist::String(fontcraft_model::plist::Str::new(s));
        assert_eq!(parse_stored(&text("255,0,0,128")), Some([255, 0, 0, 128]));
        assert_eq!(parse_stored(&text("(10, 20, 30)")), Some([10, 20, 30, 255]));
        assert_eq!(parse_stored(&text("#0000FF")), Some([0, 0, 255, 255]));
        assert_eq!(parse_stored(&text("300,0,0")), None);
        assert_eq!(parse_stored(&text("1,2")), None);
    }
}

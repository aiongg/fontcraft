//! Glyph › Transform Metrics: one advance width, or new sidebearings, for the selected glyph
//! layers, previewed as `glyph.transformMetrics` runs it.
//!
//! The Width checkbox chooses the mode. With it on, the dialog sends `width` (and
//! `onBothSides`); with it off, it sends `lsb` and `rsb` for the sides whose checkboxes are on,
//! and `relative`. The value fields of the other mode are kept but not sent. The dialog opens on
//! a change of nothing: sidebearing mode, both sides, relative, by 0.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Shaped};

pub const DIALOG: Dialog =
    Dialog::new("dialog.transformMetrics", "Transform Metrics", "glyph.transformMetrics", Fields::Shaped(Shaped { fields, given, params, link }));

/// The command's ranges (`fontcraft_spacing::WIDTH_RANGE`, `SIDEBEARING_RANGE`).
const MAX_WIDTH: f64 = 65_535.0;
const MIN_SIDEBEARING: f64 = -32_768.0;
const MAX_SIDEBEARING: f64 = 32_767.0;

fn fields(session: &Session) -> Vec<Field> {
    let upm = session.font().map_or(1000.0, |f| f64::from(f.units_per_em));
    vec![
        Field::bool("useWidth", "Width", false, "On: give every layer the width below; off: set the sidebearings checked below"),
        Field::number("width", "Width value", 0.0, MAX_WIDTH, (upm / 2.0).round(), "The new advance width in font units"),
        Field::bool(
            "onBothSides",
            "on both sides",
            false,
            "On: the outline moves by half the change, so both sidebearings share it; off: the points keep their coordinates",
        ),
        Field::bool("useLsb", "LSB", true, "Set the left sidebearing (sidebearing mode)"),
        Field::number("lsb", "LSB value", MIN_SIDEBEARING, MAX_SIDEBEARING, 0.0, "The left sidebearing, or with Relative the amount added to it"),
        Field::bool("useRsb", "RSB", true, "Set the right sidebearing (sidebearing mode)"),
        Field::number("rsb", "RSB value", MIN_SIDEBEARING, MAX_SIDEBEARING, 0.0, "The right sidebearing, or with Relative the amount added to it"),
        Field::bool("relative", "Relative", true, "On: add the LSB and RSB values to the current sidebearings; off: replace them"),
    ]
}

/// Field values from `dialog.open`'s parameters in the command's shape: `width` turns Width on;
/// `lsb` or `rsb` turns it off and checks the sides given.
fn given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    let sides = p.contains_key("lsb") || p.contains_key("rsb");
    if p.contains_key("width") {
        out.insert("useWidth".into(), json!(true));
    } else if sides {
        out.insert("useWidth".into(), json!(false));
        out.insert("useLsb".into(), json!(p.contains_key("lsb")));
        out.insert("useRsb".into(), json!(p.contains_key("rsb")));
    }
    for (key, value) in p {
        out.insert(key.clone(), value.clone());
    }
    Ok(out)
}

/// The command's parameters from the field values.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let on = |key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
    let number = |key: &str| json!(v.get(key).and_then(Value::as_f64).unwrap_or(0.0));
    let mut out = Map::new();
    if on("useWidth") {
        out.insert("width".into(), number("width"));
        out.insert("onBothSides".into(), json!(on("onBothSides")));
        return out;
    }
    if on("useLsb") {
        out.insert("lsb".into(), number("lsb"));
    }
    if on("useRsb") {
        out.insert("rsb".into(), number("rsb"));
    }
    out.insert("relative".into(), json!(on("relative")));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

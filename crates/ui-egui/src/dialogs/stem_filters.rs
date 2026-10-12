//! The dialogs of the stem filters (M13.24): Adjust Stems, Change Weight, Condense / Extend and
//! Change x-Height, each previewed as its `filter.*` command runs it.
//!
//! Adjust Stems shows one rule per direction (a checkbox, the width range, "by" or "to" and a
//! value); the command takes more rules over the control channel. Change Weight sends its zone
//! fields only with the custom method. Condense / Extend sends percentages and units as the
//! command's `counters` and `sidebearings` objects. Every dialog opens on a change of nothing.

use fontcraft_engine::Session;
use fontcraft_model::{Metrics, metric};
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Shaped};

pub const ADJUST: Dialog = Dialog::new(
    "dialog.filterAdjustStems",
    "Adjust Stems",
    "filter.adjustStems",
    Fields::Shaped(Shaped { fields: adjust_fields, given: adjust_given, params: adjust_params, link }),
);
pub const WEIGHT: Dialog = Dialog::new(
    "dialog.filterChangeWeight",
    "Change Weight",
    "filter.changeWeight",
    Fields::Shaped(Shaped { fields: weight_fields, given: weight_given, params: weight_params, link }),
);
pub const CONDENSE: Dialog = Dialog::new(
    "dialog.filterCondenseExtend",
    "Condense / Extend",
    "filter.condenseExtend",
    Fields::Shaped(Shaped { fields: condense_fields, given: condense_given, params: condense_params, link }),
);
pub const X_HEIGHT: Dialog = Dialog::new("dialog.filterChangeXHeight", "Change x-Height", "filter.changeXHeight", Fields::Fixed(x_height_fields));

/// The widest stem the filters look for (`fontcraft_pathops::stem_filters::MAX_FILTER_STEM`).
const MAX_STEM: f64 = 2_000.0;
/// The command's largest weight change and added units.
const MAX_WEIGHT: f64 = 1_000.0;
const MAX_ADD: f64 = 10_000.0;
const MAX_HEIGHT: f64 = 10_000.0;

fn link(_: &mut Map<String, Value>, _: &str) {}

/// The default master's metric `kind`, or the format's value.
fn metric_of(session: &Session, kind: &str) -> f64 {
    session
        .font()
        .ok()
        .and_then(|f| {
            let d = Metrics::defaults(f.source.format, f.units_per_em);
            let default = match kind {
                metric::CAP_HEIGHT => d.cap_height,
                _ => d.x_height,
            };
            f.masters.first().map(|m| f.metric_pos(m, kind, default))
        })
        .unwrap_or(500.0)
}

fn number(v: &Map<String, Value>, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn on(v: &Map<String, Value>, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn word<'a>(v: &'a Map<String, Value>, key: &str, default: &'a str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or(default)
}

/// The parameters that are no field's (the targets), to pass through.
fn targets(p: &Map<String, Value>, taken: &[&str]) -> Map<String, Value> {
    p.iter().filter(|(k, _)| !taken.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

// ── Adjust Stems ────────────────────────────────────────────────────────────────────────────────

const DIRECTIONS: [(&str, &str); 2] = [("vertical", "Vertical stems"), ("horizontal", "Horizontal stems")];

fn adjust_fields(_: &Session) -> Vec<Field> {
    DIRECTIONS
        .iter()
        .flat_map(|&(key, group)| {
            vec![
                Field::bool(key, "Change", key == "vertical", "Change the stems of this direction").in_group(group),
                Field::number(&format!("{key}From"), "From", 0.0, MAX_STEM, 0.0, "Narrowest stem width the rule takes").in_group(group),
                Field::number(&format!("{key}To"), "To", 0.0, MAX_STEM, MAX_STEM, "Widest stem width the rule takes").in_group(group),
                Field::choice(
                    &format!("{key}Mode"),
                    "Change",
                    &[("offset", "by"), ("set", "to")],
                    "offset",
                    "By: add the value to each width; to: make each width the value",
                )
                .in_group(group),
                Field::number(&format!("{key}Value"), "Value", -MAX_STEM, MAX_STEM, 0.0, "Units added, or the new width").in_group(group),
            ]
        })
        .collect()
}

/// The first rule of each direction fills that direction's fields.
fn adjust_given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = targets(p, &["vertical", "horizontal"]);
    for (key, _) in DIRECTIONS {
        let Some(list) = p.get(key) else { continue };
        let rule = list.as_array().and_then(|l| l.first()).and_then(Value::as_object).ok_or_else(|| format!("`{key}` must list rules"))?;
        out.insert(key.into(), json!(true));
        for (field, from) in [("From", "from"), ("To", "to")] {
            if let Some(v) = rule.get(from) {
                out.insert(format!("{key}{field}"), v.clone());
            }
        }
        let (mode, value) = match (rule.get("set"), rule.get("offset")) {
            (Some(v), _) => ("set", v.clone()),
            (None, Some(v)) => ("offset", v.clone()),
            (None, None) => ("offset", json!(0.0)),
        };
        out.insert(format!("{key}Mode"), json!(mode));
        out.insert(format!("{key}Value"), value);
    }
    Ok(out)
}

fn adjust_params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, _) in DIRECTIONS {
        if !on(v, key) {
            continue;
        }
        let mode = if word(v, &format!("{key}Mode"), "offset") == "set" { "set" } else { "offset" };
        let rule = json!({"from": number(v, &format!("{key}From")), "to": number(v, &format!("{key}To")), mode: number(v, &format!("{key}Value"))});
        out.insert(key.into(), json!([rule]));
    }
    out
}

// ── Change Weight ───────────────────────────────────────────────────────────────────────────────

fn weight_fields(session: &Session) -> Vec<Field> {
    vec![
        Field::number("amount", "Amount", -MAX_WEIGHT, MAX_WEIGHT, 0.0, "How much thicker every stem gets, in units; negative thins"),
        Field::choice(
            "method",
            "Method",
            &[("auto", "Automatic"), ("lcg", "Keep heights"), ("cjk", "Expand only"), ("custom", "Custom zones")],
            "auto",
            "Automatic: expand only for CJK, Hangul and kana, else keep heights",
        ),
        Field::number("bottom", "Bottom zone", -MAX_HEIGHT, MAX_HEIGHT, 0.0, "The lower height that stays (custom zones)").in_group("Custom zones"),
        Field::number("top", "Top zone", -MAX_HEIGHT, MAX_HEIGHT, metric_of(session, metric::X_HEIGHT), "The upper height that stays (custom zones)")
            .in_group("Custom zones"),
        Field::number("serifHeight", "Serif height", 0.0, MAX_HEIGHT, 0.0, "Features this close to a zone move without stretching (custom zones)")
            .in_group("Custom zones"),
        Field::choice(
            "counters",
            "Counters",
            &[("auto", "Automatic"), ("squish", "Squish"), ("retain", "Retain")],
            "auto",
            "Squish: counters shrink and the width stays; retain: counters keep their widths and the glyph widens",
        ),
        Field::bool("cleanup", "Remove overlaps", false, "Remove overlaps after the change"),
    ]
}

fn weight_given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = targets(p, &["zones"]);
    if let Some(zones) = p.get("zones") {
        let zones = zones.as_object().ok_or("`zones` must be an object")?;
        for key in ["bottom", "top", "serifHeight"] {
            if let Some(v) = zones.get(key) {
                out.insert(key.into(), v.clone());
            }
        }
    }
    Ok(out)
}

fn weight_params(v: &Map<String, Value>) -> Map<String, Value> {
    let method = word(v, "method", "auto");
    let mut out = Map::new();
    out.insert("amount".into(), json!(number(v, "amount")));
    out.insert("method".into(), json!(method));
    out.insert("counters".into(), json!(word(v, "counters", "auto")));
    out.insert("cleanup".into(), json!(on(v, "cleanup")));
    if method == "custom" {
        out.insert("zones".into(), json!({"bottom": number(v, "bottom"), "top": number(v, "top"), "serifHeight": number(v, "serifHeight")}));
    }
    out
}

// ── Condense / Extend ───────────────────────────────────────────────────────────────────────────

const SCALED: [(&str, &str, &str); 2] = [("counters", "counter", "Counters"), ("sidebearings", "sidebearing", "Sidebearings")];

fn condense_fields(_: &Session) -> Vec<Field> {
    SCALED
        .iter()
        .flat_map(|&(_, prefix, group)| {
            vec![
                Field::number(&format!("{prefix}Scale"), "Scale %", 1.0, 1_000.0, 100.0, "Percent of the current size").in_group(group),
                Field::number(&format!("{prefix}Add"), "Add", -MAX_ADD, MAX_ADD, 0.0, "Units added after scaling").in_group(group),
            ]
        })
        .collect()
}

fn condense_given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = targets(p, &["counters", "sidebearings"]);
    for (key, prefix, _) in SCALED {
        let Some(obj) = p.get(key) else { continue };
        let obj = obj.as_object().ok_or_else(|| format!("`{key}` must be an object"))?;
        for (field, name) in [("Scale", "scale"), ("Add", "add")] {
            if let Some(v) = obj.get(name) {
                out.insert(format!("{prefix}{field}"), v.clone());
            }
        }
    }
    Ok(out)
}

fn condense_params(v: &Map<String, Value>) -> Map<String, Value> {
    SCALED
        .iter()
        .map(|&(key, prefix, _)| {
            let scale = v.get(&format!("{prefix}Scale")).and_then(Value::as_f64).unwrap_or(100.0);
            (key.to_string(), json!({"scale": scale, "add": number(v, &format!("{prefix}Add"))}))
        })
        .collect()
}

// ── Change x-Height ─────────────────────────────────────────────────────────────────────────────

fn x_height_fields(session: &Session) -> Vec<Field> {
    let x = metric_of(session, metric::X_HEIGHT);
    vec![
        Field::number("from", "Current x-height", 1.0, MAX_HEIGHT, x, "The x-height the outlines have now"),
        Field::number("to", "New x-height", 1.0, MAX_HEIGHT, x, "The x-height they get"),
        Field::number("serifHeight", "Serif height", 0.0, MAX_HEIGHT, 0.0, "Features this close to the baseline or x-height move without stretching"),
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Font;
    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    /// An `H` with 80-unit stems and a crossbar, edited in a tab.
    fn h_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        let nodes: Vec<Value> =
            [(0, 0), (80, 0), (80, 300), (520, 300), (520, 0), (600, 0), (600, 700), (520, 700), (520, 360), (80, 360), (80, 700), (0, 700)]
                .iter()
                .map(|(x, y)| json!({"x": x, "y": y}))
                .collect();
        app.run("path.addPath", &json!({"glyph": "H", "nodes": nodes})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["H"]})).unwrap();
        app
    }

    fn font(app: &FontcraftApp) -> Arc<Font> {
        app.session.doc().unwrap().font.clone()
    }

    /// Opens `dialog`, sets `fields`, presses OK; the result must equal running `command` with
    /// `params` on the same glyph.
    fn same_as_command(dialog: &str, fields: Value, command: &str, params: Value) {
        let mut app = h_app();
        let before = font(&app);
        call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": dialog}}));
        for (field, value) in fields.as_object().unwrap() {
            call(&mut app, "ui.dialog.set", json!({"field": field, "value": value}));
        }
        call(&mut app, "ui.dialog.confirm", json!({}));
        let mut direct = h_app();
        direct.run(command, &params).unwrap();
        assert!(!font(&app).same_as(&before), "{dialog} changed nothing");
        assert!(font(&app).same_as(&font(&direct)), "{dialog} and {command} give the same font");
    }

    #[test]
    fn each_stem_filter_dialog_gives_its_commands_font() {
        same_as_command(
            "dialog.filterAdjustStems",
            json!({"verticalFrom": 70, "verticalTo": 90, "verticalValue": 10}),
            "filter.adjustStems",
            json!({"glyph": "H", "vertical": [{"from": 70, "to": 90, "offset": 10}]}),
        );
        same_as_command(
            "dialog.filterChangeWeight",
            json!({"amount": 20, "method": "custom", "top": 700, "serifHeight": 10}),
            "filter.changeWeight",
            json!({"glyph": "H", "amount": 20, "method": "custom", "zones": {"bottom": 0, "top": 700, "serifHeight": 10}}),
        );
        same_as_command(
            "dialog.filterCondenseExtend",
            json!({"counterScale": 80, "sidebearingAdd": 5}),
            "filter.condenseExtend",
            json!({"glyph": "H", "counters": {"scale": 80}, "sidebearings": {"add": 5}}),
        );
        same_as_command(
            "dialog.filterChangeXHeight",
            json!({"to": 540, "serifHeight": 20}),
            "filter.changeXHeight",
            json!({"glyph": "H", "to": 540, "serifHeight": 20}),
        );
    }
}

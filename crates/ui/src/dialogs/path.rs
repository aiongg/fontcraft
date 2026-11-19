//! Path and glyph dialogs that preview on the outlines, and the outline search.
//!
//! - Transformations (`dialog.transformations`, Path › Transformations): translate, scale, slant and Cursivy of
//!   `path.transformations` about an origin, previewed on the selected points or, with none selected, on the whole
//!   layer. The fields come in the reference's order: Translate (X, Y), Origin, Scale (horizontal, vertical and a lock)
//!   and Slant (kind, angle). The dialog shapes them into the command's `translate`, `scale`, `slant` and `cursivy`,
//!   leaving out a translation of 0 and a scale of 100 %. While the lock is on, the two scale fields hold the same
//!   value. The actions menu copies the values as a Filter or PreFilter custom parameter.
//! - Transform Metrics (`dialog.transformMetrics`, Glyph › Transform Metrics): one advance width, or new sidebearings,
//!   for the selected glyph layers. The Width checkbox chooses the mode: on sends `width` and `onBothSides`; off sends
//!   `lsb` and `rsb` for the sides whose checkboxes are on, and `relative`. It opens on a change of nothing.
//! - The stem filters: Adjust Stems, Change Weight, Condense / Extend and Change x-Height, each previewed as its
//!   `filter.*` command runs it. Adjust Stems shows one rule per direction; the command takes more rules over the
//!   control channel. Change Weight sends its zone fields only with the custom method. Condense / Extend sends
//!   percentages and units as the command's `counters` and `sidebearings` objects.
//! - Find / Replace Outline (`dialog.findOutline`): searches the font for the paths of a pattern glyph with
//!   `path.findOutlinePattern` and replaces them with the paths of a replacement glyph with
//!   `path.replaceOutlinePattern`. The glyphs searched are the selected ones, or every glyph when none is selected.
//!   Edit › Find Outline… runs Find All, then shows the matches with Replace All; Edit › Replace Outline… runs Replace
//!   All, and its actions menu copies the list of matches. Nothing changes until Replace All, one undo step.

use fontcraft_engine::Session;
use fontcraft_model::{MAX_COORDINATE, MAX_NAME_LEN, Metrics, metric};
use serde_json::{Map, Value, json};

use super::fields::Field;
use super::spec::{Action, After, DialogSpec, Fields, Shaped};

pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new(
        "dialog.transformations",
        "Transformations",
        "path.transformations",
        Fields::Shaped(Shaped { fields: transform_fields, given: transform_given, params: transform_params, link: transform_link }),
    )
    .actions(&TRANSFORM_ACTIONS),
    DialogSpec::new(
        "dialog.transformMetrics",
        "Transform Metrics",
        "glyph.transformMetrics",
        Fields::Shaped(Shaped { fields: metrics_fields, given: metrics_given, params: metrics_params, link: no_link }),
    ),
    DialogSpec::new(
        "dialog.filterAdjustStems",
        "Adjust Stems",
        "filter.adjustStems",
        Fields::Shaped(Shaped { fields: adjust_fields, given: adjust_given, params: adjust_params, link: no_link }),
    ),
    DialogSpec::new(
        "dialog.filterChangeWeight",
        "Change Weight",
        "filter.changeWeight",
        Fields::Shaped(Shaped { fields: weight_fields, given: weight_given, params: weight_params, link: no_link }),
    ),
    DialogSpec::new(
        "dialog.filterCondenseExtend",
        "Condense / Extend",
        "filter.condenseExtend",
        Fields::Shaped(Shaped { fields: condense_fields, given: condense_given, params: condense_params, link: no_link }),
    ),
    DialogSpec::new("dialog.filterChangeXHeight", "Change x-Height", "filter.changeXHeight", Fields::Fixed(x_height_fields)),
    // Edit › Replace Outline….
    DialogSpec::new("dialog.findOutline", "Replace Outline", REPLACE_COMMAND, Fields::Fixed(replace_fields))
        .run_on_ok()
        .buttons("Replace All", "")
        .actions(&[Action { id: "copyOutlineMatches", label: "Copy Matches", command: FIND_COMMAND }]),
    // Edit › Find Outline….
    DialogSpec::new("dialog.findOutline", "Find Outline", FIND_COMMAND, Fields::Fixed(find_fields)).run_on_ok().buttons("Find All", ""),
    // What Find All found, with Replace All on the same glyphs.
    DialogSpec::new("dialog.findOutline", "Find Outline", REPLACE_COMMAND, Fields::Targeted(found_fields))
        .run_on_ok()
        .buttons("Replace All", "Close")
        .after(After { command: FIND_COMMAND, params: found_params }),
];

fn no_link(_: &mut Map<String, Value>, _: &str) {}

// ── Transformations ─────────────────────────────────────────────────────────────────────────────────────────────────

const TRANSFORM_ACTIONS: [Action; 2] = [
    Action { id: "copyFilterParameter", label: "Copy Filter Parameter", command: "path.transformations.copyFilterParameter" },
    Action { id: "copyPreFilterParameter", label: "Copy PreFilter Parameter", command: "path.transformations.copyPreFilterParameter" },
];

/// The command refuses ±90° and beyond, where the shear has no finite factor.
const MAX_SLANT: f64 = 89.9;
/// The largest scale percentage the command takes.
const MAX_PERCENT: f64 = 1_000_000.0;

/// The origins with their menu labels, in the order of their export-string codes (0 cap height
/// … 4 baseline).
const ORIGINS: [(&str, &str); 5] = [
    ("capHeight", "Cap Height"),
    ("halfCapHeight", "Half Cap Height"),
    ("xHeight", "x-Height"),
    ("halfXHeight", "Half x-Height"),
    ("baseline", "Baseline"),
];

fn transform_fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![
        Field::number("translateX", "X", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "Horizontal move in font units, after the scale and slant")
            .in_group("Translate"),
        Field::number("translateY", "Y", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "Vertical move in font units, after the scale and slant")
            .in_group("Translate"),
        Field::choice(
            "origin",
            "Origin",
            &ORIGINS,
            "baseline",
            "The fixed point of the scale and slant: a metric height of the master, at half the advance width",
        )
        ,
        Field::number("scaleX", "Horizontal", -MAX_PERCENT, MAX_PERCENT, 100.0, "Horizontal scale in percent").in_group("Scale"),
        Field::number("scaleY", "Vertical", -MAX_PERCENT, MAX_PERCENT, 100.0, "Vertical scale in percent; follows the horizontal scale while the lock is on")
            .in_group("Scale"),
        Field::lock("scaleLocked", "Proportional", true, "Locked: the vertical scale stays equal to the horizontal one").in_group("Scale"),
        Field::choice(
            "slantKind",
            "Kind",
            &[("slant", "Slant"), ("cursivy", "Cursivy")],
            "slant",
            "Slant: a plain shear about the origin; Cursivy: a slant with optical correction of the curves, which needs the master's horizontal and vertical stems",
        )
        .in_group("Slant"),
        Field::number("slant", "Angle", -MAX_SLANT, MAX_SLANT, 0.0, "Degrees of the shear about the origin").in_group("Slant"),
    ]
}

/// Field values from `dialog.open`'s parameters in the command's shape: `translate: {x, y}`,
/// `scale: {x, y?}` (a missing or equal `y` keeps the lock on), `cursivy: bool`.
fn transform_given(_: &fontcraft_engine::Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (key, value) in p {
        match (key.as_str(), value) {
            ("translate", Value::Object(t)) => {
                out.insert("translateX".into(), t.get("x").cloned().unwrap_or(json!(0.0)));
                out.insert("translateY".into(), t.get("y").cloned().unwrap_or(json!(0.0)));
            }
            ("scale", Value::Object(s)) => {
                let x = s.get("x").cloned().unwrap_or(json!(100.0));
                let y = s.get("y").filter(|y| !y.is_null()).cloned().unwrap_or_else(|| x.clone());
                out.insert("scaleLocked".into(), json!(x.as_f64() == y.as_f64()));
                out.insert("scaleX".into(), x);
                out.insert("scaleY".into(), y);
            }
            ("cursivy", Value::Bool(on)) => {
                out.insert("slantKind".into(), json!(if *on { "cursivy" } else { "slant" }));
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(out)
}

/// The command's parameters from the field values.
fn transform_params(v: &Map<String, Value>) -> Map<String, Value> {
    let number = |key: &str, neutral: f64| v.get(key).and_then(Value::as_f64).unwrap_or(neutral);
    let (tx, ty) = (number("translateX", 0.0), number("translateY", 0.0));
    let (sx, sy) = (number("scaleX", 100.0), number("scaleY", 100.0));
    let mut out = Map::new();
    if tx != 0.0 || ty != 0.0 {
        out.insert("translate".into(), json!({"x": tx, "y": ty}));
    }
    if sx != 100.0 || sy != 100.0 {
        out.insert("scale".into(), json!({"x": sx, "y": sy}));
    }
    out.insert("slant".into(), json!(number("slant", 0.0)));
    out.insert("cursivy".into(), json!(v.get("slantKind").and_then(Value::as_str) == Some("cursivy")));
    out.insert("origin".into(), v.get("origin").cloned().unwrap_or(json!("baseline")));
    out
}

/// While the lock is on, the scale field not just typed into takes the other's value.
fn transform_link(v: &mut Map<String, Value>, changed: &str) {
    if v.get("scaleLocked").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let (from, to) = if changed == "scaleY" { ("scaleY", "scaleX") } else { ("scaleX", "scaleY") };
    if let Some(value) = v.get(from).cloned() {
        v.insert(to.into(), value);
    }
}

// ── Transform Metrics ───────────────────────────────────────────────────────────────────────────────────────────────

/// The command's ranges (`fontcraft_spacing::WIDTH_RANGE`, `SIDEBEARING_RANGE`).
const MAX_WIDTH: f64 = 65_535.0;
const MIN_SIDEBEARING: f64 = -32_768.0;
const MAX_SIDEBEARING: f64 = 32_767.0;

fn metrics_fields(session: &Session) -> Vec<Field> {
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
fn metrics_given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
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
fn metrics_params(v: &Map<String, Value>) -> Map<String, Value> {
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

// ── Stem filters ────────────────────────────────────────────────────────────────────────────────────────────────────

/// The widest stem the filters look for (`fontcraft_pathops::stem_filters::MAX_FILTER_STEM`).
const MAX_STEM: f64 = 2_000.0;
/// The command's largest weight change and added units.
const MAX_WEIGHT: f64 = 1_000.0;
const MAX_ADD: f64 = 10_000.0;
const MAX_HEIGHT: f64 = 10_000.0;

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

fn x_height_fields(session: &Session) -> Vec<Field> {
    let x = metric_of(session, metric::X_HEIGHT);
    vec![
        Field::number("from", "Current x-height", 1.0, MAX_HEIGHT, x, "The x-height the outlines have now"),
        Field::number("to", "New x-height", 1.0, MAX_HEIGHT, x, "The x-height they get"),
        Field::number("serifHeight", "Serif height", 0.0, MAX_HEIGHT, 0.0, "Features this close to the baseline or x-height move without stretching"),
    ]
}

// ── Find / Replace Outline ──────────────────────────────────────────────────────────────────────────────────────────

const FIND_COMMAND: &str = "path.findOutlinePattern";
const REPLACE_COMMAND: &str = "path.replaceOutlinePattern";
/// The command's largest tolerance, in font units.
const MAX_TOLERANCE: f64 = 10.0;
/// Matches the list shows; a last row counts the others.
const MAX_ROWS: usize = 1_000;

fn pattern_field() -> Field {
    Field::text("patternGlyph", "Pattern Glyph", MAX_NAME_LEN, "The glyph whose paths, on each layer searched, are the shape to find")
}

fn replacement_field() -> Field {
    Field::text("replacementGlyph", "Replacement Glyph", MAX_NAME_LEN, "The glyph whose paths take the place of each match")
}

fn option_fields() -> [Field; 3] {
    [
        Field::bool("allowFlipped", "Allow Flipped", false, "Also find mirrored and upside-down copies"),
        Field::bool(
            "endpointsLoose",
            "Loose End Points",
            false,
            "The end nodes of an open pattern give only the direction and least length of the segments leading to them",
        ),
        Field::number(
            "tolerance",
            "Tolerance",
            0.0,
            MAX_TOLERANCE,
            0.5,
            "The largest distance between a pattern node and the node it matches, in units",
        ),
    ]
}

fn find_fields(_: &Session) -> Vec<Field> {
    let mut fields = vec![pattern_field()];
    fields.extend(option_fields());
    fields
}

fn replace_fields(_: &Session) -> Vec<Field> {
    let mut fields = vec![pattern_field(), replacement_field()];
    fields.extend(option_fields());
    fields
}

/// The rows of the match list, given under `matches` by [`found_params`].
fn found_fields(_: &mut Session, given: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let rows: Vec<String> = given
        .get("matches")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let mut fields =
        vec![Field::rows("matches", "Matches", rows, "Where the pattern was found, one row per match"), pattern_field(), replacement_field()];
    fields.extend(option_fields());
    Ok(fields)
}

/// The search's parameters and one row per match (at most [`MAX_ROWS`], then a count of the
/// rest).
fn found_params(request: &Value, result: &Value) -> Option<Map<String, Value>> {
    let mut given = request.as_object().cloned().unwrap_or_default();
    let count = result.get("count").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let mut rows: Vec<Value> = result.get("text").and_then(Value::as_str).unwrap_or_default().lines().take(MAX_ROWS).map(|l| json!(l)).collect();
    if count == 0 {
        rows.push(json!("No matches"));
    } else if count > rows.len() {
        rows.push(json!(format!("and {} more", count.saturating_sub(rows.len()))));
    }
    given.insert("matches".into(), Value::Array(rows));
    Some(given)
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;

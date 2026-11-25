//! The editors of special layers: one number field per axis for an intermediate layer's coordinates, a min and a max
//! field per axis for an alternate layer's ranges, with a live preview.
//!
//! | Dialog | Edits | Opens with |
//! |---|---|---|
//! | `popover.intermediateLayer` | `layers.setIntermediate` | `{glyph, layer?: id \| layers?: [ids], glyphs?: [names], coordinates?}` |
//! | `popover.alternateLayer` | `layers.setAlternate` | `{glyph, layer?: id \| layers?: [ids], glyphs?: [names], ranges?}` |
//!
//! - Intermediate: a field per font axis, keyed by the axis tag (`wght`), labelled with the axis name and the range of
//!   the masters and virtual masters on it.
//! - Alternate: the fields `wght.min` and `wght.max` per axis, under a heading with the axis name and range. An axis with
//!   both fields empty is left out, so all empty gives the `[]` partner layer.
//! - Both: For All Masters (`allMasters`), which adds a layer under the other masters of the glyph, or of `glyphs`.
//!
//! The fields start from the first layer named. A field holds the text of a number, so that it can be empty: an empty
//! coordinate takes the master's, an empty bound is open. Text that is not a number goes to the command as it is, which
//! refuses it; the dialog shows the message and OK keeps nothing.

use fontcraft_engine::Session;
use fontcraft_engine::interp::Designspace;
use fontcraft_engine::model::{Font, Layer};
use serde_json::{Map, Value, json};

use super::super::fields::{Field, MAX_FIELDS, format_number};
use super::super::spec::{DialogSpec, Fields, Shaped};
use super::no_link;

/// The intermediate layer's coordinates.
pub const INTERMEDIATE: DialogSpec = DialogSpec::new(
    "popover.intermediateLayer",
    "Intermediate Layer",
    "layers.setIntermediate",
    Fields::Shaped(Shaped { fields: intermediate_fields, given: intermediate_given, params: intermediate_params, link: no_link }),
);

/// The alternate layer's axis ranges.
pub const ALTERNATE: DialogSpec = DialogSpec::new(
    "popover.alternateLayer",
    "Alternate Layer",
    "layers.setAlternate",
    Fields::Shaped(Shaped { fields: alternate_fields, given: alternate_given, params: alternate_params, link: no_link }),
);

/// Longest text a coordinate field takes.
const MAX_NUMBER_TEXT: usize = 32;
/// The key of For All Masters.
const ALL_MASTERS: &str = "allMasters";

/// Refuses a font whose axes the fields cannot show: none, too many, or two of one tag. `per_axis`: fields per axis.
fn check_axes(font: &Font, title: &str, per_axis: usize) -> Result<(), String> {
    if font.axes.is_empty() {
        return Err(format!("{title}: the font has no axes; add one in Font Info › Axes first"));
    }
    // One field more: For All Masters.
    if font.axes.len().saturating_mul(per_axis) >= MAX_FIELDS {
        return Err(format!("{title}: the font has {} axes, more than the dialog shows", font.axes.len()));
    }
    let twice = font.axes.iter().enumerate().find(|(i, a)| font.axes.iter().take(*i).any(|b| b.tag == a.tag) || a.tag == ALL_MASTERS);
    match twice {
        Some((_, a)) => Err(format!("{title}: the dialog cannot show the axis tag `{}`; change it in Font Info › Axes first", a.tag)),
        None => Ok(()),
    }
}

fn all_masters() -> Field {
    Field::bool(ALL_MASTERS, "For All Masters", false, "Also add such a layer under every other master, a copy of the master layer")
}

// ── Fields ──────────────────────────────────────────────────────────────────────────────────────

/// The name an axis is shown with: its name, else its tag.
fn axis_name(font: &Font, i: usize) -> String {
    font.axes.get(i).map(|a| if a.name.trim().is_empty() { a.tag.clone() } else { a.name.clone() }).unwrap_or_default()
}

/// Each font axis's name with the range of the masters and virtual masters on it, as the
/// fields show it: `Weight (100 to 900)`. Just the name when the font cannot interpolate.
fn axis_headings(font: &Font) -> Vec<String> {
    let designspace = Designspace::new(font).ok();
    (0..font.axes.len())
        .map(|i| {
            let range = designspace.as_ref().and_then(|ds| {
                let d = ds.axes().iter().position(|a| a.font_axis == i)?;
                let values = ds.masters().iter().map(|m| &m.location).chain(ds.virtual_masters().iter().map(|v| &v.location));
                let (lo, hi) =
                    values.filter_map(|l| l.0.get(d).copied()).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
                (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
            });
            match range {
                Some((lo, hi)) => format!("{} ({} to {})", axis_name(font, i), format_number(lo), format_number(hi)),
                None => axis_name(font, i),
            }
        })
        .collect()
}

fn intermediate_fields(session: &Session) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    font.axes
        .iter()
        .zip(axis_headings(font))
        .map(|(axis, heading)| {
            Field::text(
                &axis.tag,
                &heading,
                MAX_NUMBER_TEXT,
                "The layer's internal coordinate on this axis; empty takes the coordinate of its master",
            )
        })
        .chain([all_masters()])
        .collect()
}

fn alternate_fields(session: &Session) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    font.axes
        .iter()
        .zip(axis_headings(font))
        .flat_map(|(axis, heading)| {
            [
                Field::text(
                    &format!("{}.min", axis.tag),
                    "min",
                    MAX_NUMBER_TEXT,
                    "The lowest internal coordinate on this axis where the alternate shape is used; empty for no lower bound",
                )
                .in_group(&heading),
                Field::text(
                    &format!("{}.max", axis.tag),
                    "max",
                    MAX_NUMBER_TEXT,
                    "The highest internal coordinate on this axis where the alternate shape is used; empty for no upper bound",
                )
                .in_group(&heading),
            ]
        })
        .chain([all_masters()])
        .collect()
}

// ── Opening values ──────────────────────────────────────────────────────────────────────────────

/// The layers `given` names with `glyph` and `layer` or `layers`.
fn given_layers<'f>(font: &'f Font, given: &Map<String, Value>) -> Result<Vec<&'f Layer>, String> {
    let Some(glyph) = given.get("glyph").and_then(Value::as_str) else { return Ok(Vec::new()) };
    let g = font.glyph(glyph).ok_or_else(|| format!("there is no glyph `{glyph}`"))?;
    let ids: Vec<&str> = match (given.get("layer"), given.get("layers")) {
        (Some(Value::String(id)), None) => vec![id],
        (None, Some(Value::Array(list))) => list.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    ids.iter().map(|id| g.layers.iter().find(|l| l.id == *id).ok_or_else(|| format!("glyph `{glyph}` has no layer `{id}`"))).collect()
}

/// A coordinate as its field holds it: the text of a number, empty for none.
fn field_text(key: &str, v: &Value) -> Result<Value, String> {
    match v {
        Value::Null => Ok(json!("")),
        Value::Number(n) => {
            n.as_f64().filter(|x| x.is_finite()).map(|x| json!(format_number(x))).ok_or_else(|| format!("`{key}` is not a usable number"))
        }
        Value::String(_) => Ok(v.clone()),
        _ => Err(format!("`{key}` must be a number or null")),
    }
}

/// The parameters of `given` other than `drop`, with numbers given for the fields `keys` turned
/// into their text.
fn pass_through(given: &Map<String, Value>, drop: &str, keys: &[String]) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (k, v) in given.iter().filter(|(k, _)| k.as_str() != drop) {
        let value = if keys.contains(k) { field_text(k, v)? } else { v.clone() };
        out.insert(k.clone(), value);
    }
    Ok(out)
}

/// `coordinates: {tag: number | null}` to the fields; without it, the layer's own coordinates.
fn intermediate_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    check_axes(font, INTERMEDIATE.title, 1)?;
    let layers = given_layers(font, given)?;
    if layers.iter().any(|l| l.is_master_layer()) {
        return Err("a master layer cannot be an intermediate layer: add a layer under the master and make that one intermediate".into());
    }
    let tags: Vec<String> = font.axes.iter().map(|a| a.tag.clone()).collect();
    let mut out = pass_through(given, "coordinates", &tags)?;
    match given.get("coordinates") {
        Some(Value::Object(m)) => {
            for (tag, v) in m {
                if !tags.contains(tag) {
                    return Err(format!("`coordinates` names the axis `{tag}`, which the font does not have"));
                }
                out.insert(tag.clone(), field_text(&format!("coordinates.{tag}"), v)?);
            }
        }
        None | Some(Value::Null) => {
            for (tag, c) in tags.iter().zip(layers.first().map(|l| l.info.attrs.coordinates.as_slice()).unwrap_or_default()) {
                if let Some(v) = c.filter(|v| v.is_finite())
                    && !out.contains_key(tag)
                {
                    out.insert(tag.clone(), json!(format_number(v)));
                }
            }
        }
        Some(_) => return Err("`coordinates` must be an object: {tag: number or null}".into()),
    }
    Ok(out)
}

/// `ranges: {tag: {min?, max?}}` to the fields; without it, the layer's own ranges.
fn alternate_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    check_axes(font, ALTERNATE.title, 2)?;
    let layers = given_layers(font, given)?;
    let tags: Vec<String> = font.axes.iter().map(|a| a.tag.clone()).collect();
    let keys: Vec<String> = tags.iter().flat_map(|t| [format!("{t}.min"), format!("{t}.max")]).collect();
    let mut out = pass_through(given, "ranges", &keys)?;
    match given.get("ranges") {
        Some(Value::Object(m)) => {
            for (tag, range) in m {
                if !tags.contains(tag) {
                    return Err(format!("`ranges` names the axis `{tag}`, which the font does not have"));
                }
                let range = range.as_object().ok_or_else(|| format!("`ranges.{tag}` must be an object: {{min?, max?}}"))?;
                for (bound, v) in range {
                    if bound != "min" && bound != "max" {
                        return Err(format!("`ranges.{tag}` takes `min` and `max`, not `{bound}`"));
                    }
                    let key = format!("{tag}.{bound}");
                    out.insert(key.clone(), field_text(&key, v)?);
                }
            }
        }
        None | Some(Value::Null) => {
            for (tag, rule) in tags.iter().zip(layers.first().map(|l| l.info.attrs.axis_rules.as_slice()).unwrap_or_default()) {
                for (bound, v) in [("min", rule.min), ("max", rule.max)] {
                    let key = format!("{tag}.{bound}");
                    if let Some(v) = v.filter(|v| v.is_finite())
                        && !out.contains_key(&key)
                    {
                        out.insert(key, json!(format_number(v)));
                    }
                }
            }
        }
        Some(_) => return Err("`ranges` must be an object: {tag: {min?, max?}}".into()),
    }
    Ok(out)
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// A field's text as the command takes it: empty is null, a number is a number, anything else
/// goes as it is for the command to refuse.
fn field_number(v: &Value) -> Value {
    match v {
        Value::String(s) if s.trim().is_empty() => Value::Null,
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| v.clone(), |n| json!(n)),
        other => other.clone(),
    }
}

fn with_all_masters(mut m: Map<String, Value>, values: &Map<String, Value>) -> Map<String, Value> {
    if values.get(ALL_MASTERS) == Some(&Value::Bool(true)) {
        m.insert(ALL_MASTERS.into(), json!(true));
    }
    m
}

fn intermediate_params(values: &Map<String, Value>) -> Map<String, Value> {
    let coordinates: Map<String, Value> = values.iter().filter(|(k, _)| *k != ALL_MASTERS).map(|(tag, v)| (tag.clone(), field_number(v))).collect();
    let mut m = Map::new();
    m.insert("coordinates".into(), Value::Object(coordinates));
    with_all_masters(m, values)
}

fn alternate_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut ranges = Map::new();
    for (key, v) in values {
        let Some((tag, bound)) = key.rsplit_once('.') else { continue };
        let n = field_number(v);
        if n.is_null() {
            continue;
        }
        if let Value::Object(range) = ranges.entry(tag.to_string()).or_insert_with(|| json!({})) {
            range.insert(bound.to_string(), n);
        }
    }
    let mut m = Map::new();
    m.insert("ranges".into(), Value::Object(ranges));
    with_all_masters(m, values)
}

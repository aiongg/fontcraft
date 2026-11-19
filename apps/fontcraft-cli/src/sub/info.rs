//! `fontcraft-cli info`, over the engine command `font.inspect`. Text by default, one block per
//! input; `--json` prints the engine results.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "info", summary: "Describe fonts and sources", usage: USAGE, options: OPTIONS, actions: &[], run: info };

const USAGE: &str = "\
  fontcraft-cli info INPUT... [--section NAME,...] [--all-names] [--face N] [--json]
      Describe each INPUT (a compiled font, collection or source) without opening it: format,
      names, metrics, style, tables, cmap, layout, axes, instances, masters, color, hinting,
      faces. A part that cannot be read is reported in place. --json prints one JSON object
      for one input and a list of them for several.
";

const OPTIONS: &[OptSpec] = &[
    OptSpec::list("section", "NAME,...", "", "print only these parts (default: all that apply)"),
    OptSpec::switch("all-names", false, "list every name record instead of the main ones in English"),
    OptSpec::value("face", "N", "", "the member of a collection, by number"),
    OptSpec::switch("json", false, "print the engine results as JSON"),
];

/// The sections in the order the text prints them; others follow in the result's order.
const ORDER: [&str; 13] =
    ["format", "names", "metrics", "style", "tables", "cmap", "layout", "axes", "instances", "masters", "color", "hinting", "faces"];

/// The fields that name a list item, printed before the others.
const FIRST: [&str; 3] = ["tag", "name", "nameId"];

/// Deepest nesting the text follows; deeper values print as JSON on one line.
const MAX_DEPTH: usize = 12;

fn info(raw: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, raw)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let inputs = &parsed.positional;
    if inputs.is_empty() {
        return Err(usage("info needs at least one INPUT"));
    }
    let mut base = Map::new();
    if !parsed.list("section").is_empty() {
        base.insert("sections".into(), json!(parsed.list("section")));
    }
    if parsed.switch("all-names") {
        base.insert("allNames".into(), json!(true));
    }
    if let Some(f) = parsed.value("face") {
        base.insert("face".into(), f.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(f)));
    }
    let results = io::batch(inputs, parsed.jobs()?, |input| {
        let mut params = base.clone();
        params.insert("input".into(), json!(input));
        let mut session = Session::new();
        session.execute("font.inspect", &Value::Object(params)).map_err(|e| match e {
            fontcraft_engine::EngineError::BadParams { .. } => usage(e.to_string()),
            fontcraft_engine::EngineError::Io(_) => Failure::Input(e.to_string()),
            _ => Failure::Failed(e.to_string()),
        })
    })?;
    if let Some(Err(e @ Failure::Usage(_))) = results.iter().find(|r| matches!(r, Err(Failure::Usage(_)))) {
        return Err(e.clone());
    }
    if parsed.switch("json") {
        let found: Vec<&Value> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        match found.as_slice() {
            [] => {}
            [one] if inputs.len() == 1 => io::out_json(one)?,
            many => io::out_json(&json!(many))?,
        }
    } else {
        let mut first = true;
        for result in results.iter().filter_map(|r| r.as_ref().ok()) {
            if !first {
                out("")?;
            }
            first = false;
            out(text(result).trim_end())?;
        }
    }
    for (input, result) in inputs.iter().zip(&results) {
        if let Err(e) = result {
            io::error(&format!("{input}: {}", e.message()));
        }
    }
    io::batch_outcome(&results)
}

/// The text of one result: the path, then each section with its fields indented under it.
fn text(result: &Value) -> String {
    let mut text = String::new();
    let Some(obj) = result.as_object() else { return result.to_string() };
    text.push_str(obj.get("path").and_then(Value::as_str).unwrap_or_default());
    text.push('\n');
    let mut keys: Vec<&str> = ORDER.to_vec();
    keys.extend(obj.keys().map(String::as_str).filter(|k| *k != "path" && !ORDER.contains(k)));
    for key in keys {
        if let Some(value) = obj.get(key) {
            entry(key, value, 1, &mut text);
        }
    }
    text
}

/// `key: value` for a scalar or a list of scalars, a key line with the fields below it for an
/// object, and one `field=value` line per item for a list of objects.
fn entry(key: &str, value: &Value, level: usize, text: &mut String) {
    let pad = "  ".repeat(level);
    match value {
        Value::Object(map) if level < MAX_DEPTH => {
            if let Some(error) = map.get("error").and_then(Value::as_str) {
                text.push_str(&format!("{pad}{key}: cannot be read: {error}\n"));
                return;
            }
            text.push_str(&format!("{pad}{key}:\n"));
            for (k, v) in map {
                entry(k, v, level + 1, text);
            }
        }
        Value::Array(items) if items.is_empty() => text.push_str(&format!("{pad}{key}: none\n")),
        Value::Array(items) if items.iter().all(|i| !i.is_object() && !i.is_array()) => {
            text.push_str(&format!("{pad}{key}: {}\n", items.iter().map(scalar).collect::<Vec<_>>().join(", ")));
        }
        Value::Array(items) if level < MAX_DEPTH => {
            text.push_str(&format!("{pad}{key}: {}\n", items.len()));
            for item in items {
                text.push_str(&format!("{pad}  {}\n", inline(item)));
            }
        }
        other => text.push_str(&format!("{pad}{key}: {}\n", scalar(other))),
    }
}

/// An item of a list on one line: an object as `field=value` pairs, the fields that name the
/// item first.
fn inline(item: &Value) -> String {
    match item.as_object() {
        Some(map) => FIRST
            .iter()
            .filter_map(|k| map.get_key_value(*k))
            .chain(map.iter().filter(|(k, _)| !FIRST.contains(&k.as_str())))
            .map(|(k, v)| match v {
                Value::Array(items) if items.iter().all(|i| !i.is_object() && !i.is_array()) => {
                    format!("{k}={}", items.iter().map(scalar).collect::<Vec<_>>().join(","))
                }
                other => format!("{k}={}", word(other)),
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => scalar(item),
    }
}

/// A value inside a `field=value` pair: quoted when it would not read as one word.
fn word(v: &Value) -> String {
    match v {
        Value::String(s) if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c == '=' || c == '"') => v.to_string(),
        other => scalar(other),
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".to_string(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_text_lists_sections_in_order() {
        let result = json!({
            "path": "F.ttf",
            "metrics": { "hhea": { "ascender": 800 }, "unitsPerEm": 1000 },
            "format": { "faces": 1, "kind": "TrueType" },
            "tables": [{ "length": 54, "tag": "head" }, { "length": 96, "tag": "OS/2" }],
            "masters": { "error": "a compiled font has no masters" },
            "names": { "family": "My Font" },
            "hinting": { "cff": false, "truetypeTables": [] },
        });
        assert_eq!(
            text(&result),
            "F.ttf\n  format:\n    faces: 1\n    kind: TrueType\n  names:\n    family: My Font\n  metrics:\n    hhea:\n      ascender: 800\n    unitsPerEm: 1000\n  tables: 2\n    tag=head length=54\n    tag=OS/2 length=96\n  masters: cannot be read: a compiled font has no masters\n  hinting:\n    cff: no\n    truetypeTables: none\n"
        );
    }

    #[test]
    fn info_refuses_bad_lines() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(info(&s(&[])).unwrap_err().exit_code(), 2);
        assert_eq!(info(&s(&["/no/such/font.ttf"])).unwrap_err().exit_code(), 3);
        assert_eq!(info(&s(&["a.ttf", "--sect", "nope"])).unwrap_err().exit_code(), 2, "an unknown section, the option by its prefix");
    }
}

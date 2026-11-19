//! `fontcraft-cli glyphs`: `list`, `rename` and `reorder`, over the engine commands
//! `font.listGlyphs`, `font.renameGlyphs` and `font.reorderGlyphs`.
//!
//! `list` prints the reference listings as text: the index and name of every glyph wrapped into
//! lines (`spot -n`), one per line with `--one-per-line` (`spot -nc`), or a row per glyph with
//! the fields `--columns` names (`tx -dump -4`, with `--columns gid,name,unicodes`). `--json`
//! prints the engine result.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "glyphs",
    summary: "List, rename and reorder glyphs",
    usage: USAGE,
    options: OPTIONS,
    actions: &["list", "rename", "reorder"],
    run: glyphs,
};

const USAGE: &str = "\
  fontcraft-cli glyphs list FONT [--one-per-line | --columns COLS] [--glyphs SELECTOR] [--exclude-glyphs SELECTOR]
                            [--names source --goadb FILE] [--face N] [--json]
      List the glyphs of FONT (a compiled font or a source) in glyph order: [GID]=<NAME> items
      wrapped into lines, one per line with --one-per-line, or with --columns one row per glyph,
      glyph[GID] {FIELD,...}, under a header naming the fields.
  fontcraft-cli glyphs rename FONT (--goadb FILE | --map FILE | --production-names | OLD=NEW...) [-o OUT | --in-place]
  fontcraft-cli glyphs reorder FONT (--order-file FILE | --goadb FILE | --sort KEY | NAME...) [-o OUT | --in-place]
";

const FACE: OptSpec = OptSpec::value("face", "N", "", "the member of a collection, by number");
const JSON: OptSpec = OptSpec::switch("json", false, "print the engine result as JSON");
const ONE_PER_LINE: OptSpec = OptSpec::switch("one-per-line", false, "list: one glyph per line");
const COLUMNS: OptSpec = OptSpec::list("columns", "COLS", "", "list: one row per glyph with these fields: gid, name, unicodes, advance, bbox, kind");
const GLYPHS: OptSpec = OptSpec::list("glyphs", "SELECTOR", "", "list: only these glyphs (names, ids, U+XXXX, A..B, *)").repeat();
const EXCLUDE: OptSpec = OptSpec::list("exclude-glyphs", "SELECTOR", "", "list: leave these glyphs out (never .notdef)").repeat();
const NAMES: OptSpec = OptSpec::value("names", "final|source", "", "list: the names to show (default: source with --goadb, else final)");
const GOADB: OptSpec = OptSpec::value("goadb", "FILE", "", "list: map names back to source names; rename and reorder: names and order from FILE");
const MAP: OptSpec = OptSpec::value("map", "FILE", "", "rename: new and old name per line");
const PRODUCTION: OptSpec = OptSpec::switch("production-names", false, "rename: the production names of the glyph data");
const REVERSE: OptSpec = OptSpec::switch("reverse", false, "rename: read the GOADB or map the other way round");
const UPDATE_UNICODES: OptSpec = OptSpec::switch("update-unicodes", false, "rename: take the code points the GOADB gives");
const ORDER: OptSpec = OptSpec::switch("order", false, "rename: also order the glyphs as the GOADB does");
const ORDER_FILE: OptSpec = OptSpec::value("order-file", "FILE", "", "reorder: one glyph name per line");
const SORT: OptSpec = OptSpec::value("sort", "KEY", "", "reorder: sort by a key");
const OUTPUT: OptSpec = OptSpec::value("output", "FILE", "", "rename, reorder: the output file").short('o');
const OUTPUT_DIR: OptSpec = OptSpec::value("output-dir", "DIR", "", "rename, reorder: the folder the output goes into");
const IN_PLACE: OptSpec = OptSpec::switch("in-place", false, "rename, reorder: replace the input");
const NO_CLOBBER: OptSpec = OptSpec::switch("no-clobber", false, "rename, reorder: refuse to replace an existing file");

/// Every option, as help prints them.
const OPTIONS: &[OptSpec] = &[
    ONE_PER_LINE,
    COLUMNS,
    GLYPHS,
    EXCLUDE,
    NAMES,
    GOADB,
    MAP,
    PRODUCTION,
    REVERSE,
    UPDATE_UNICODES,
    ORDER,
    ORDER_FILE,
    SORT,
    OUTPUT,
    OUTPUT_DIR,
    IN_PLACE,
    NO_CLOBBER,
    FACE,
    JSON,
];
const LIST: &[OptSpec] = &[ONE_PER_LINE, COLUMNS, GLYPHS, EXCLUDE, NAMES, GOADB, FACE, JSON];
const RENAME: &[OptSpec] = &[GOADB, MAP, PRODUCTION, REVERSE, UPDATE_UNICODES, ORDER, OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE];
const REORDER: &[OptSpec] = &[ORDER_FILE, GOADB, SORT, OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE];

/// Longest line of the wrapped listing, in characters.
const WRAP: usize = 100;

fn glyphs(args: &[String]) -> Outcome {
    let Some((verb, rest)) = args.split_first() else { return Err(usage("glyphs needs a subcommand: list, rename or reorder")) };
    if verb == "-h" || verb == "--help" {
        return out(&super::help(&SUBCOMMAND));
    }
    let specs = match verb.as_str() {
        "list" => LIST,
        "rename" => RENAME,
        "reorder" => REORDER,
        other => return Err(usage(format!("unknown glyphs action `{other}` (actions: list, rename, reorder)"))),
    };
    let parsed = args::parse(specs, rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    match verb.as_str() {
        "rename" => rename(&parsed),
        "reorder" => reorder(&parsed),
        _ => list(&parsed),
    }
}

/// Runs an engine command: bad parameters are a usage error, an unreadable input exit status 3.
fn execute(command: &str, params: Map<String, Value>) -> Result<Value, Failure> {
    let mut session = Session::new();
    session.execute(command, &Value::Object(params)).map_err(|e| match e {
        fontcraft_engine::EngineError::BadParams { .. } => usage(e.to_string()),
        fontcraft_engine::EngineError::Io(_) => Failure::Input(e.to_string()),
        _ => Failure::Failed(e.to_string()),
    })
}

fn print_json(result: &Value) -> Outcome {
    out(&serde_json::to_string_pretty(result).map_err(|e| Failure::Failed(e.to_string()))?)
}

/// `--face N` as a number when it is one.
fn face(parsed: &Parsed, params: &mut Map<String, Value>) {
    if let Some(f) = parsed.value("face") {
        params.insert("face".into(), f.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(f)));
    }
}

/// The value options of `pairs` that were given, as parameters.
fn values(parsed: &Parsed, params: &mut Map<String, Value>, pairs: &[(&str, &str)]) {
    for (option, key) in pairs {
        if let Some(v) = parsed.value(option) {
            params.insert((*key).to_string(), json!(v));
        }
    }
}

/// The switches of `pairs` that are on, as parameters.
fn switches(parsed: &Parsed, params: &mut Map<String, Value>, pairs: &[(&str, &str)]) {
    for (option, key) in pairs {
        if parsed.switch(option) {
            params.insert((*key).to_string(), json!(true));
        }
    }
}

fn list(parsed: &Parsed) -> Outcome {
    let [font] = parsed.positional.as_slice() else { return Err(usage("glyphs list needs one FONT")) };
    let columns = parsed.list("columns");
    if parsed.switch("one-per-line") && !columns.is_empty() {
        return Err(usage("--one-per-line and --columns are two layouts: give one"));
    }
    let mut params = Map::new();
    params.insert("font".into(), json!(font));
    face(parsed, &mut params);
    values(parsed, &mut params, &[("names", "names"), ("goadb", "goadb")]);
    for (option, key) in [("glyphs", "glyphs"), ("exclude-glyphs", "excludeGlyphs")] {
        if !parsed.list(option).is_empty() {
            params.insert(key.into(), json!(parsed.list(option)));
        }
    }
    let json_out = parsed.switch("json");
    if !columns.is_empty() {
        params.insert("columns".into(), json!(columns));
    } else if !json_out {
        // The listings without --columns need only the names; leaving the rest out spares
        // working out every outline's bounds.
        params.insert("columns".into(), json!(["gid", "name"]));
    }
    let result = execute("font.listGlyphs", params)?;
    if json_out {
        return print_json(&result);
    }
    let glyphs = result.get("glyphs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let text = if !columns.is_empty() { rows(glyphs, columns) } else { names(glyphs, parsed.switch("one-per-line")) };
    out(text.trim_end())
}

/// The `[GID]=<NAME>` listing: wrapped at [`WRAP`] characters, or one item per line.
fn names(glyphs: &[Value], one_per_line: bool) -> String {
    let mut text = String::from("--- names[glyphId]=<name>\n");
    let mut line = String::new();
    for g in glyphs {
        let item =
            format!("[{}]=<{}>", g.get("gid").and_then(Value::as_u64).unwrap_or_default(), g.get("name").and_then(Value::as_str).unwrap_or_default());
        if one_per_line {
            text.push_str(&item);
            text.push('\n');
            continue;
        }
        if !line.is_empty() && line.len() + 1 + item.len() > WRAP {
            text.push_str(&line);
            text.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&item);
    }
    text.push_str(&line);
    text
}

/// One `glyph[GID] {FIELD,...}` row per glyph under a header naming the fields. `gid` is the
/// row's tag; code points are written U+XXXX joined by `+`, a missing value as `-`.
fn rows(glyphs: &[Value], columns: &[String]) -> String {
    let fields: Vec<&str> = columns.iter().map(String::as_str).filter(|c| *c != "gid").collect();
    let header: Vec<&str> = fields.iter().map(|c| if *c == "unicodes" { "encoding" } else { *c }).collect();
    let mut text = format!("## glyph[tag] {{{}}}\n", header.join(","));
    for g in glyphs {
        let values: Vec<String> = fields.iter().map(|c| field(g, c)).collect();
        text.push_str(&format!("glyph[{}] {{{}}}\n", g.get("gid").and_then(Value::as_u64).unwrap_or_default(), values.join(",")));
    }
    text
}

fn field(glyph: &Value, column: &str) -> String {
    let key = if column == "bbox" { "bounds" } else { column };
    match glyph.get(key) {
        Some(Value::Array(items)) if column == "unicodes" && !items.is_empty() => {
            items.iter().filter_map(Value::as_str).map(|u| format!("U+{u}")).collect::<Vec<_>>().join("+")
        }
        Some(Value::Array(items)) if !items.is_empty() => items.iter().map(number).collect::<Vec<_>>().join(" "),
        Some(Value::String(s)) => s.clone(),
        Some(v @ Value::Number(_)) => number(v),
        _ => "-".to_string(),
    }
}

/// A number without a fractional part written as an integer.
fn number(v: &Value) -> String {
    match v.as_f64() {
        Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
        Some(f) => format!("{f}"),
        None => v.to_string(),
    }
}

/// Output options shared by `rename` and `reorder`.
const OUTPUT_VALUES: [(&str, &str); 2] = [("output", "output"), ("output-dir", "outputDir")];
const OUTPUT_SWITCHES: [(&str, &str); 2] = [("in-place", "inPlace"), ("no-clobber", "noClobber")];

fn rename(parsed: &Parsed) -> Outcome {
    let Some((font, pairs)) = parsed.positional.split_first() else { return Err(usage("glyphs rename needs a FONT")) };
    let mut params = Map::new();
    params.insert("font".into(), json!(font));
    face(parsed, &mut params);
    values(parsed, &mut params, &[("goadb", "goadb"), ("map", "map")]);
    values(parsed, &mut params, &OUTPUT_VALUES);
    switches(
        parsed,
        &mut params,
        &[("production-names", "productionNames"), ("reverse", "reverse"), ("update-unicodes", "updateUnicodes"), ("order", "order")],
    );
    switches(parsed, &mut params, &OUTPUT_SWITCHES);
    if !pairs.is_empty() {
        let mut renames = Map::new();
        for pair in pairs {
            let Some((old, new)) = pair.split_once('=') else { return Err(usage(format!("`{pair}` is not OLD=NEW"))) };
            renames.insert(old.to_string(), json!(new));
        }
        params.insert("renames".into(), Value::Object(renames));
    }
    print_json(&execute("font.renameGlyphs", params)?)
}

fn reorder(parsed: &Parsed) -> Outcome {
    let Some((font, order)) = parsed.positional.split_first() else { return Err(usage("glyphs reorder needs a FONT")) };
    let mut params = Map::new();
    params.insert("font".into(), json!(font));
    face(parsed, &mut params);
    values(parsed, &mut params, &[("order-file", "orderFile"), ("goadb", "goadb"), ("sort", "sort")]);
    values(parsed, &mut params, &OUTPUT_VALUES);
    switches(parsed, &mut params, &OUTPUT_SWITCHES);
    if !order.is_empty() {
        params.insert("order".into(), json!(order));
    }
    print_json(&execute("font.reorderGlyphs", params)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(gid: u64, name: &str) -> Value {
        let unicodes = if name == "A" { json!(["0041", "0391"]) } else { json!([]) };
        json!({ "gid": gid, "name": name, "unicodes": unicodes, "advance": 500.0, "bounds": null })
    }

    #[test]
    fn listings_print_as_the_reference_tools_do() {
        let glyphs = vec![glyph(0, ".notdef"), glyph(1, "A"), glyph(2, "B")];
        assert_eq!(names(&glyphs, false), "--- names[glyphId]=<name>\n[0]=<.notdef> [1]=<A> [2]=<B>");
        assert_eq!(names(&glyphs, true), "--- names[glyphId]=<name>\n[0]=<.notdef>\n[1]=<A>\n[2]=<B>\n");
        let columns: Vec<String> = ["gid", "name", "unicodes", "advance", "bbox"].iter().map(|c| c.to_string()).collect();
        assert_eq!(
            rows(&glyphs, &columns),
            "## glyph[tag] {name,encoding,advance,bbox}\nglyph[0] {.notdef,-,500,-}\nglyph[1] {A,U+0041+U+0391,500,-}\nglyph[2] {B,-,500,-}\n"
        );
        let many: Vec<Value> = (0..40).map(|i| glyph(i, "glyphname")).collect();
        let text = names(&many, false);
        assert!(text.lines().count() > 2 && text.lines().all(|l| l.len() <= WRAP), "{text}");
    }

    #[test]
    fn glyphs_options_take_prefixes_and_refuse_mixed_layouts() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let parsed = args::parse(LIST, &s(&["F.ttf", "--one", "--exc", "A,B"])).unwrap();
        assert!(parsed.switch("one-per-line"));
        assert_eq!(parsed.list("exclude-glyphs"), ["A", "B"]);
        assert_eq!(glyphs(&s(&["list", "F.ttf", "--one-per-line", "--columns", "name"])).unwrap_err().exit_code(), 2);
        assert_eq!(glyphs(&s(&["list", "F.ttf", "--sort", "name"])).unwrap_err().exit_code(), 2, "a reorder option is unknown to list");
        assert_eq!(glyphs(&s(&["list", "/no/such/font.ttf"])).unwrap_err().exit_code(), 3);
    }
}

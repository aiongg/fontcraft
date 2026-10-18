//! `fontcraft-cli merge` (M12.23): one `font.merge` run. The per-font options (`--map`,
//! `--glyphs`, `--exclude-glyphs`, `--fd`, `--exclude-fd`, `--face`) apply to the FONT before
//! them, so they are taken out of the arguments before the rest is parsed.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, Kind, OptSpec};
use crate::io::{Failure, Outcome, execute, note, out, out_json, read_input, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand { name: "merge", summary: "Merge fonts into one", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli merge FONT [--map FILE] [--glyphs LIST] FONT [--map FILE] ... -o OUT [options]
      Combine compiled TrueType fonts into one. Without maps, every glyph of every font is
      kept (a name already used gets .1, .2 ...), the cmaps are united with the earlier font
      keeping a shared character, and GSUB, GPOS and GDEF are merged; font-wide values take
      the extremes or the first font's. A map file or glyph selection after a FONT takes only
      those glyphs from it, renamed as the map says, with every font-wide value from the first
      font and no layout unless --layout asks for it.
";

/// The options, as data (M12.23).
const OPTIONS: &[OptSpec] = &[
    OptSpec::value("map", "FILE", "", "glyph map for the preceding FONT: 'new-name source-name' per line"),
    OptSpec::list("glyphs", "SELECTOR", "", "take only these glyphs from the preceding FONT"),
    OptSpec::list("exclude-glyphs", "SELECTOR", "", "leave these glyphs of the preceding FONT out"),
    OptSpec::list("fd", "N", "", "take only glyphs of these CID font dictionaries from the preceding FONT"),
    OptSpec::list("exclude-fd", "N", "", "leave glyphs of these CID font dictionaries out"),
    OptSpec::value("face", "N|NAME", "", "the face of the preceding FONT when it is a collection"),
    OptSpec::value("input-list", "FILE", "", "read font paths from this file, one per line, '#' comments"),
    OptSpec::value("layout", "merge|base|drop", "", "merge every font's layout, keep the first font's, or drop it (default merge, drop with maps)"),
    OptSpec::value("duplicates", "first|last|error|rename", "", "what to do with a glyph name two fonts share (default rename, first with maps)"),
    OptSpec::list("drop-tables", "TAG", "", "tables to leave out of the result"),
    OptSpec::value("import-ttx", "FILE", "", "compile this XML dump into the result"),
    OptSpec::value("cid-info", "FILE", "", "make the result CID-keyed with this font information"),
    OptSpec::value("hint-dict-from", "FONT", "", "take the hint dictionary from this font"),
    OptSpec::value("ps-name", "NAME", "", "the PostScript name of a CFF result"),
    OptSpec::value("language-group", "0|1", "", "the language group of a CFF result"),
    OptSpec::value("to", "FORMAT", "", "the output format"),
    OptSpec::value("output", "OUT", "", "the output file").short('o'),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::switch("json", false, "print the result as JSON"),
];

/// Options that belong to the FONT before them, with their engine parameter names.
const PER_FONT: [(&str, &str); 6] =
    [("map", "map"), ("glyphs", "glyphs"), ("exclude-glyphs", "excludeGlyphs"), ("fd", "fd"), ("exclude-fd", "excludeFd"), ("face", "face")];

fn run(args: &[String]) -> Outcome {
    let (mut fonts, rest) = split(args)?;
    let parsed = args::parse(OPTIONS, &rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    for (name, _) in PER_FONT {
        if parsed.values.get(name).is_some_and(|v| !matches!(v, args::OptValue::List(l) if l.is_empty())) {
            return Err(usage(format!("--{name} must follow the FONT it applies to")));
        }
    }
    let mut listed: Vec<Map<String, Value>> = Vec::new();
    if let Some(list) = parsed.value("input-list") {
        let text = String::from_utf8(read_input(list)?).map_err(|_| Failure::Input(format!("{list} is not UTF-8 text")))?;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            listed.push(font_item(line));
        }
    }
    listed.extend(parsed.positional.iter().map(|p| font_item(p)));
    listed.append(&mut fonts);
    if listed.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    let Some(output) = parsed.value("output") else { return Err(usage("give the output file with -o")) };

    let mut params = Map::new();
    params.insert("fonts".into(), Value::Array(listed.into_iter().map(Value::Object).collect()));
    params.insert("output".into(), json!(output));
    params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    for (key, name) in [
        ("layout", "layout"),
        ("duplicates", "duplicates"),
        ("importTtx", "import-ttx"),
        ("cidInfo", "cid-info"),
        ("hintDictFrom", "hint-dict-from"),
        ("psName", "ps-name"),
        ("to", "to"),
    ] {
        if let Some(v) = parsed.value(name) {
            params.insert(key.into(), json!(v));
        }
    }
    if let Some(group) = parsed.value("language-group") {
        params.insert("languageGroup".into(), group.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(group)));
    }
    if !parsed.list("drop-tables").is_empty() {
        params.insert("dropTables".into(), json!(parsed.list("drop-tables")));
    }

    let result = execute(&mut Session::new(), "font.merge", &Value::Object(params)).map_err(|f| match f {
        Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
        other => other,
    })?;
    if parsed.switch("json") {
        return out_json(&result);
    }
    let strings = |key: &str| -> Vec<String> {
        result.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    for line in strings("conflicts").iter().chain(strings("warnings").iter()) {
        note(line);
    }
    out(&format!(
        "{} glyphs -> {}",
        result.get("glyphCount").and_then(Value::as_u64).unwrap_or(0),
        result.get("output").and_then(Value::as_str).unwrap_or("?")
    ))
}

fn font_item(path: &str) -> Map<String, Value> {
    let mut item = Map::new();
    item.insert("path".into(), json!(path));
    item
}

/// The FONT arguments with the per-font options attached, and every other argument.
fn split(args: &[String]) -> Result<(Vec<Map<String, Value>>, Vec<String>), Failure> {
    let mut fonts: Vec<Map<String, Value>> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--" {
            fonts.extend(it.by_ref().map(|a| font_item(a)));
            break;
        }
        let Some(body) = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-').filter(|b| b.len() == 1)) else {
            if arg.starts_with('@') {
                rest.push(arg.clone());
            } else {
                fonts.push(font_item(arg));
            }
            continue;
        };
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (body, None),
        };
        if let Some((_, key)) = PER_FONT.iter().find(|(n, _)| *n == name) {
            let value = match inline {
                Some(v) => v,
                None => it.next().cloned().ok_or_else(|| usage(format!("--{name} needs a value")))?,
            };
            let Some(font) = fonts.last_mut() else { return Err(usage(format!("--{name} must follow the FONT it applies to"))) };
            let value = match *key {
                "glyphs" | "excludeGlyphs" => json!(value.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>()),
                "fd" | "excludeFd" => json!(value.split(',').filter_map(|s| s.trim().parse::<u64>().ok()).collect::<Vec<_>>()),
                "face" => value.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(value)),
                _ => json!(value),
            };
            font.insert((*key).into(), value);
            continue;
        }
        rest.push(arg.clone());
        // A value option given as two arguments takes the next one with it.
        let spec = OPTIONS.iter().find(|s| s.name == name || (name.len() == 1 && s.short.is_some_and(|c| name.starts_with(c))));
        if inline.is_none()
            && spec.is_some_and(|s| s.kind != Kind::Switch)
            && let Some(value) = it.next()
        {
            rest.push(value.clone());
        }
    }
    Ok((fonts, rest))
}

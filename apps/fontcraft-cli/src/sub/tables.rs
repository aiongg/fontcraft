//! `fontcraft-cli tables`: the table directory and the tables of compiled fonts. `list` prints the
//! directory as text (the rows of `ttx -l`) unless `--json` is given; the other actions print
//! their engine results as JSON.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{self, Failure, Outcome, out, usage, warn};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "tables",
    summary: "Work on the tables of a compiled font",
    usage: USAGE,
    options: OPTIONS,
    actions: &["list", "extract", "add", "delete", "edit", "copy", "checksum", "import", "reorder", "show"],
    run: tables,
};

const USAGE: &str = "\
  fontcraft-cli tables list FONT... [--sort file|tag|size] [--face N|all] [--json]
      Print the table directory of each compiled font, collection, WOFF or WOFF2 file: tag,
      checksum, length and offset per table, with the compressed length for WOFF, the stored
      length of transformed WOFF2 tables, and the tables collection members share.
  fontcraft-cli tables extract FONT TAG[=FILE]... [--all] [--output-dir DIR] [--no-clobber]
      Copy the raw bytes of tables to files (default FONT-NAME.TAG beside the font).
  fontcraft-cli tables add FONT TAG=FILE... [-o OUT | --in-place]
      Add or replace tables from files of raw bytes.
  fontcraft-cli tables delete FONT TAG... [-o OUT | --in-place]
      Remove tables; a table the font does not have is a warning.
  fontcraft-cli tables edit FONT [--extract TAG[=FILE],...] [--delete TAG,...] [--add TAG=FILE,...] [-o OUT | --in-place]
      Extract, then delete, then add, in one pass.
  fontcraft-cli tables copy FROM TO TAG... [--from-face N] [-o OUT | --in-place]
      Copy tables from one font into another.
  fontcraft-cli tables checksum FONT... [--fix] [-o OUT | --in-place]
      Check every table checksum and head.checkSumAdjustment; exit status 1 when one is wrong.
      With --fix, write the font again with correct values.
  fontcraft-cli tables import FONT DUMP.ttx... [--no-recalc-bounds] [-o OUT | --in-place]
      Compile the tables of TTX dumps into the font, replacing tables with the same tag.
  fontcraft-cli tables reorder FONT [--order recommended|TAG,...] [-o OUT | --in-place]
      Write the tables in the recommended order, or the tags given first.
  fontcraft-cli tables show FONT [TAG...] [--path PATH] [--depth N] [--offsets] [--format json|text]
      Show the decoded fields of tables as a tree (3 levels unless --depth says otherwise).

  Without -o or --in-place, a changed font is written beside the input as FONT.edited.EXT
  (FONT.fixed.EXT for checksum --fix, FONT.reordered.EXT for reorder). Every action takes
  --face N for a member of a collection.
";

const FACE: OptSpec = OptSpec::value("face", "N", "", "the member of a collection (list: N or all)");
const JSON: OptSpec = OptSpec::switch("json", false, "print the result as JSON (list, show; the other actions always do)");
const SORT: OptSpec = OptSpec::value("sort", "file|tag|size", "file", "list: the order of the rows");
const OUTPUT: OptSpec = OptSpec::value("output", "FILE", "", "the output file").short('o');
const OUTPUT_DIR: OptSpec = OptSpec::value("output-dir", "DIR", "", "the folder outputs go into");
const IN_PLACE: OptSpec = OptSpec::switch("in-place", false, "replace the input");
const NO_CLOBBER: OptSpec = OptSpec::switch("no-clobber", false, "refuse to replace an existing file");
const ALL: OptSpec = OptSpec::switch("all", false, "extract: every table");
const EXTRACT: OptSpec = OptSpec::list("extract", "TAG[=FILE],...", "", "edit: tables to extract first").repeat();
const DELETE: OptSpec = OptSpec::list("delete", "TAG,...", "", "edit: tables to delete next").repeat();
const ADD: OptSpec = OptSpec::list("add", "TAG=FILE,...", "", "edit: tables to add last").repeat();
const FROM_FACE: OptSpec = OptSpec::value("from-face", "N", "", "copy: the member of FROM to copy from");
const FIX: OptSpec = OptSpec::switch("fix", false, "checksum: write the font again with correct values");
const RECALC_BOUNDS: OptSpec = OptSpec::switch("recalc-bounds", true, "import: recompute bounding boxes");
const ORDER: OptSpec = OptSpec::value("order", "recommended|TAG,...", "", "reorder: the order to write (default: recommended)");
const PATH: OptSpec = OptSpec::value("path", "PATH", "", "show: one record or field (GPOS.lookupList.lookups[3])");
const DEPTH: OptSpec = OptSpec::value("depth", "N", "", "show: levels of nested records to expand (default: 3)");
const FORMAT: OptSpec = OptSpec::value("format", "json|text", "", "show: the output format (default: text)");
const OFFSETS: OptSpec = OptSpec::switch("offsets", false, "show: give each node's offset");

/// Every option, as help prints them.
const OPTIONS: &[OptSpec] = &[
    SORT,
    OUTPUT,
    OUTPUT_DIR,
    IN_PLACE,
    NO_CLOBBER,
    ALL,
    EXTRACT,
    DELETE,
    ADD,
    FROM_FACE,
    FIX,
    RECALC_BOUNDS,
    ORDER,
    PATH,
    DEPTH,
    FORMAT,
    OFFSETS,
    FACE,
    JSON,
];
const LIST: &[OptSpec] = &[SORT, FACE, JSON];
const EXTRACT_OPTIONS: &[OptSpec] = &[ALL, OUTPUT_DIR, NO_CLOBBER, FACE, JSON];
/// The options of every action that writes a font.
const WRITE: &[OptSpec] = &[OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE, JSON];
const EDIT: &[OptSpec] = &[EXTRACT, DELETE, ADD, OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE, JSON];
const COPY: &[OptSpec] = &[FROM_FACE, OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE, JSON];
const CHECKSUM: &[OptSpec] = &[FIX, OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE, JSON];
const IMPORT: &[OptSpec] = &[RECALC_BOUNDS, OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE, JSON];
const REORDER: &[OptSpec] = &[ORDER, OUTPUT, OUTPUT_DIR, IN_PLACE, NO_CLOBBER, FACE, JSON];
const SHOW: &[OptSpec] = &[PATH, DEPTH, FORMAT, OFFSETS, FACE, JSON];

fn tables(args: &[String]) -> Outcome {
    let Some((verb, rest)) = args.split_first() else { return Err(usage("tables needs a subcommand: list")) };
    if verb == "-h" || verb == "--help" {
        return out(&super::help(&SUBCOMMAND));
    }
    let specs = match verb.as_str() {
        "list" => LIST,
        "extract" => EXTRACT_OPTIONS,
        "add" | "delete" => WRITE,
        "edit" => EDIT,
        "copy" => COPY,
        "checksum" => CHECKSUM,
        "import" => IMPORT,
        "reorder" => REORDER,
        "show" => SHOW,
        _ => return super::not_built(&SUBCOMMAND, args),
    };
    let parsed = args::parse(specs, rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    match verb.as_str() {
        "list" => list(&parsed),
        "extract" => extract(&parsed),
        "add" => add(&parsed),
        "delete" => delete(&parsed),
        "edit" => edit(&parsed),
        "copy" => copy(&parsed),
        "checksum" => checksum(&parsed),
        "import" => import(&parsed),
        "reorder" => reorder(&parsed),
        _ => show(&parsed),
    }
}

/// Runs an engine command; an unreadable input is exit status 3, any other failure 4.
fn run(id: &str, params: Map<String, Value>) -> Result<Value, Failure> {
    let mut session = Session::new();
    session.execute(id, &Value::Object(params)).map_err(|e| match e {
        fontcraft_engine::EngineError::Io(m) => Failure::Input(m),
        other => Failure::Failed(other.to_string()),
    })
}

fn print(result: &Value) -> Outcome {
    if let Some(warnings) = result.get("warnings").and_then(Value::as_array) {
        for w in warnings.iter().filter_map(Value::as_str) {
            warn(w);
        }
    }
    out(&serde_json::to_string_pretty(result).map_err(|e| Failure::Failed(e.to_string()))?)
}

/// The parameters every writing action shares: the output options and `--face`.
fn write_params(parsed: &Parsed) -> Map<String, Value> {
    let mut params = Map::new();
    for (option, key) in [("output", "output"), ("output-dir", "outputDir")] {
        if let Some(v) = parsed.value(option) {
            params.insert(key.into(), json!(v));
        }
    }
    for (option, key) in [("in-place", "inPlace"), ("no-clobber", "noClobber")] {
        if parsed.switch(option) {
            params.insert(key.into(), json!(true));
        }
    }
    face(parsed, "face", "face", &mut params);
    params
}

/// A face option as a number when it is one.
fn face(parsed: &Parsed, option: &str, key: &str, params: &mut Map<String, Value>) {
    if let Some(f) = parsed.value(option) {
        params.insert(key.into(), f.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(f)));
    }
}

fn list(parsed: &Parsed) -> Outcome {
    let fonts = &parsed.positional;
    if fonts.is_empty() {
        return Err(usage("tables list needs at least one FONT"));
    }
    let mut base = Map::new();
    if let Some(sort) = parsed.value("sort") {
        base.insert("sort".into(), json!(sort));
    }
    face(parsed, "face", "face", &mut base);
    let results = io::batch(fonts, parsed.jobs()?, |font| {
        let mut params = base.clone();
        params.insert("font".into(), json!(font));
        run("tables.list", params)
    })?;
    let json_out = parsed.switch("json");
    let mut first = true;
    for (font, result) in fonts.iter().zip(&results) {
        match result {
            Ok(r) if json_out && fonts.len() == 1 => print(r)?,
            Ok(r) if json_out => io::out_json_line(&json!({ "font": font, "result": r }))?,
            Ok(r) => {
                if !first {
                    out("")?;
                }
                first = false;
                out(directory(font, r).trim_end())?;
            }
            Err(e) => io::error(&format!("{font}: {}", e.message())),
        }
    }
    io::batch_outcome(&results)
}

/// The directory of one font as text: a line naming the font (and the member of a collection),
/// then the rows of `ttx -l`, with the WOFF and WOFF2 lengths and the shared mark after them.
fn directory(font: &str, result: &Value) -> String {
    let format = result.get("format").and_then(Value::as_str).unwrap_or_default();
    let faces = result.get("faces").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut text = String::new();
    for (i, face) in faces.iter().enumerate() {
        if i > 0 {
            text.push('\n');
        }
        let tables = face.get("tables").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let member =
            if format == "collection" { format!(" face {}", face.get("face").and_then(Value::as_u64).unwrap_or_default()) } else { String::new() };
        let version = sfnt_version(face.get("sfntVersion").and_then(Value::as_str).unwrap_or_default());
        text.push_str(&format!("{font}{member}: {format}, sfnt version {version}, {} tables\n", tables.len()));
        let has = |key: &str| tables.iter().any(|t| t.get(key).is_some());
        let (compressed, transformed, shared) = (has("compressedLength"), has("transformLength"), has("shared"));
        let mut header = String::from("    tag     checksum    length    offset");
        let mut rule = String::from("    ----  ----------  --------  --------");
        if compressed {
            header.push_str("  compressed");
            rule.push_str("  ----------");
        }
        if transformed {
            header.push_str("  transformed");
            rule.push_str("  -----------");
        }
        if shared {
            header.push_str("  shared");
            rule.push_str("  ------");
        }
        text.push_str(&header);
        text.push('\n');
        text.push_str(&rule);
        text.push('\n');
        for t in tables {
            let number = |key: &str| t.get(key).and_then(Value::as_u64);
            let checksum = t.get("checksum").and_then(Value::as_str).map_or_else(|| "-".to_string(), |c| format!("0x{c}"));
            let tag = t.get("tag").and_then(Value::as_str).unwrap_or_default();
            let mut row =
                format!("    {tag:<4}  {checksum:>10}  {:>8}  {:>8}", number("length").unwrap_or_default(), number("offset").unwrap_or_default());
            if compressed {
                row.push_str(&format!("  {:>10}", number("compressedLength").map_or_else(|| "-".to_string(), |n| n.to_string())));
            }
            if transformed {
                row.push_str(&format!("  {:>11}", number("transformLength").map_or_else(|| "-".to_string(), |n| n.to_string())));
            }
            if shared && t.get("shared") == Some(&Value::Bool(true)) {
                row.push_str("  yes");
            }
            text.push_str(row.trim_end());
            text.push('\n');
        }
    }
    text
}

/// An sfnt version as the reference tools name it: 1.0 for TrueType, the four letters of a tag
/// (`OTTO`, `true`), else hexadecimal.
fn sfnt_version(hex: &str) -> String {
    if hex == "00010000" {
        return "1.0".to_string();
    }
    match u32::from_str_radix(hex, 16) {
        Ok(v) if v.to_be_bytes().iter().all(|b| b.is_ascii_graphic() || *b == b' ') => {
            v.to_be_bytes().iter().map(|b| char::from(*b)).collect::<String>().trim_end().to_string()
        }
        _ => format!("0x{hex}"),
    }
}

fn extract(parsed: &Parsed) -> Outcome {
    let Some((font, tags)) = parsed.positional.split_first() else { return Err(usage("tables extract needs FONT and TAG[=FILE]...")) };
    let all = parsed.switch("all");
    if all == !tags.is_empty() {
        return Err(usage("tables extract needs TAG[=FILE]... or --all"));
    }
    let mut params = write_params(parsed);
    params.insert("font".into(), json!(font));
    params.insert("tables".into(), if all { json!("all") } else { json!(tags) });
    print(&run("tables.extract", params)?)
}

fn add(parsed: &Parsed) -> Outcome {
    let Some((font, tables)) = parsed.positional.split_first().filter(|(_, t)| !t.is_empty()) else {
        return Err(usage("tables add needs FONT and TAG=FILE..."));
    };
    let mut params = write_params(parsed);
    params.insert("font".into(), json!(font));
    params.insert("tables".into(), json!(tables));
    print(&run("tables.add", params)?)
}

fn import(parsed: &Parsed) -> Outcome {
    let Some((font, dumps)) = parsed.positional.split_first().filter(|(_, d)| !d.is_empty()) else {
        return Err(usage("tables import needs FONT and DUMP.ttx..."));
    };
    let mut params = write_params(parsed);
    if !parsed.switch("recalc-bounds") {
        params.insert("recalcBounds".into(), json!(false));
    }
    params.insert("font".into(), json!(font));
    params.insert("dumps".into(), json!(dumps));
    print(&run("tables.importTtx", params)?)
}

fn delete(parsed: &Parsed) -> Outcome {
    let Some((font, tables)) = parsed.positional.split_first().filter(|(_, t)| !t.is_empty()) else {
        return Err(usage("tables delete needs FONT and TAG..."));
    };
    let mut params = write_params(parsed);
    params.insert("font".into(), json!(font));
    params.insert("tables".into(), json!(tables));
    print(&run("tables.delete", params)?)
}

fn edit(parsed: &Parsed) -> Outcome {
    let [font] = parsed.positional.as_slice() else { return Err(usage("tables edit needs one FONT")) };
    let mut params = write_params(parsed);
    for key in ["extract", "delete", "add"] {
        if !parsed.list(key).is_empty() {
            params.insert(key.into(), json!(parsed.list(key)));
        }
    }
    params.insert("font".into(), json!(font));
    print(&run("tables.edit", params)?)
}

fn copy(parsed: &Parsed) -> Outcome {
    let [from, to, tags @ ..] = parsed.positional.as_slice() else { return Err(usage("tables copy needs FROM, TO and TAG...")) };
    if tags.is_empty() {
        return Err(usage("tables copy needs FROM, TO and TAG..."));
    }
    let mut params = write_params(parsed);
    let mut source = Map::new();
    source.insert("font".into(), json!(from));
    face(parsed, "from-face", "face", &mut source);
    let source = Value::Object(source);
    let tables: Vec<Value> =
        tags.iter().flat_map(|t| t.split(',')).filter(|t| !t.is_empty()).map(|t| json!({ "tag": t, "from": source.clone() })).collect();
    params.insert("font".into(), json!(to));
    params.insert("tables".into(), json!(tables));
    print(&run("tables.add", params)?)
}

fn checksum(parsed: &Parsed) -> Outcome {
    let files = &parsed.positional;
    if files.is_empty() {
        return Err(usage("tables checksum needs FONT..."));
    }
    let fix = parsed.switch("fix");
    if files.len() > 1 && parsed.value("output").is_some() {
        return Err(usage("tables checksum with several fonts writes each beside its input: give --output-dir or --in-place, not -o"));
    }
    let mut params = write_params(parsed);
    if fix {
        params.insert("fix".into(), json!(true));
    }
    let mut wrong = Vec::new();
    for font in files {
        let mut p = params.clone();
        p.insert("font".into(), json!(font));
        let result = run("tables.checksum", p)?;
        if result.get("ok") == Some(&Value::Bool(false)) {
            wrong.push(font.clone());
        }
        print(&json!({ "font": font, "result": result }))?;
    }
    if !wrong.is_empty() && !fix {
        return Err(Failure::Found(format!("wrong checksums in {}", wrong.join(", "))));
    }
    Ok(())
}

fn reorder(parsed: &Parsed) -> Outcome {
    let [font] = parsed.positional.as_slice() else { return Err(usage("tables reorder needs one FONT")) };
    let mut params = write_params(parsed);
    match parsed.value("order") {
        None => {}
        Some("recommended") => {
            params.insert("order".into(), json!("recommended"));
        }
        Some(list) => {
            params.insert("order".into(), json!(list.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>()));
        }
    }
    params.insert("font".into(), json!(font));
    print(&run("tables.reorder", params)?)
}

fn show(parsed: &Parsed) -> Outcome {
    let format = match parsed.value("format") {
        None => {
            if parsed.switch("json") {
                "json"
            } else {
                "text"
            }
        }
        Some(f @ ("json" | "text")) => f,
        Some(other) => return Err(usage(format!("--format must be json or text, got {other}"))),
    };
    let mut params = Map::new();
    face(parsed, "face", "face", &mut params);
    if let Some(path) = parsed.value("path") {
        params.insert("path".into(), json!(path));
    }
    if parsed.switch("offsets") {
        params.insert("offsets".into(), json!(true));
    }
    if let Some(d) = parsed.value("depth") {
        let depth: u64 = d.parse().map_err(|_| usage(format!("--depth must be a whole number, got {d}")))?;
        params.insert("depth".into(), json!(depth));
    }
    let Some((font, tags)) = parsed.positional.split_first() else { return Err(usage("tables show needs FONT")) };
    params.insert("font".into(), json!(font));
    if !tags.is_empty() {
        params.insert("tables".into(), json!(tags));
    }
    let result = run("tables.show", params)?;
    if format == "json" {
        return print(&result);
    }
    let mut text = String::new();
    for table in result.get("tables").and_then(Value::as_array).into_iter().flatten() {
        if let Some(error) = table.get("error").and_then(Value::as_str) {
            text.push_str(&format!("{}: cannot be decoded: {error}\n", table.get("tag").and_then(Value::as_str).unwrap_or_default()));
        }
        if let Some(tree) = table.get("tree") {
            indent(tree, 0, &mut text);
        }
        if table.get("truncated") == Some(&Value::Bool(true)) {
            text.push_str("  … (more nodes than the limit)\n");
        }
    }
    out(text.trim_end())
}

/// One line per node, two spaces per level. Trees are a dozen levels deep at most.
fn indent(node: &Value, level: usize, text: &mut String) {
    let name = node.get("name").and_then(Value::as_str).unwrap_or_default();
    let kind = node.get("type").and_then(Value::as_str).unwrap_or_default();
    text.push_str(&"  ".repeat(level));
    text.push_str(name);
    if let Some(value) = node.get("value").and_then(Value::as_str) {
        text.push_str(": ");
        text.push_str(value);
    }
    if !kind.is_empty() {
        text.push_str(&format!(" [{kind}]"));
    }
    if let Some(offset) = node.get("offset").and_then(Value::as_u64) {
        text.push_str(&format!(" @{offset}"));
    }
    if let Some(more) = node.get("more").and_then(Value::as_u64) {
        text.push_str(&format!(" (+{more} not shown)"));
    }
    text.push('\n');
    for child in node.get("children").and_then(Value::as_array).into_iter().flatten() {
        indent(child, level + 1, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_list_prints_the_directory_as_text() {
        let result = json!({ "format": "OpenType (CFF)", "faces": [{ "face": 0, "sfntVersion": "4F54544F", "tables": [
            { "tag": "head", "checksum": "19E5E74F", "offset": 236, "length": 54 },
            { "tag": "CFF", "checksum": "CA7FB70B", "offset": 3612, "length": 32516 },
        ] }] });
        assert_eq!(
            directory("F.otf", &result),
            "F.otf: OpenType (CFF), sfnt version OTTO, 2 tables\n    tag     checksum    length    offset\n    ----  ----------  --------  --------\n    head  0x19E5E74F        54       236\n    CFF   0xCA7FB70B     32516      3612\n"
        );
        let woff2 = json!({ "format": "WOFF2", "faces": [{ "face": 0, "sfntVersion": "00010000", "tables": [
            { "tag": "glyf", "offset": 0, "length": 900, "transformLength": 400 },
            { "tag": "head", "offset": 900, "length": 54 },
        ] }] });
        let text = directory("F.woff2", &woff2);
        assert!(text.starts_with("F.woff2: WOFF2, sfnt version 1.0, 2 tables\n"), "{text}");
        assert!(text.contains("    glyf           -       900         0          400\n"), "{text}");
        assert!(text.contains("    head           -        54       900            -\n"), "{text}");
    }

    #[test]
    fn tables_options_take_prefixes_and_response_files() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let dir = std::env::temp_dir().join(format!("fontcraft-cli-tables-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("list.args");
        std::fs::write(&file, "--so tag\n# a comment\nF.ttf\n").unwrap();
        let at = format!("@{}", file.display());
        let parsed = args::parse(LIST, &s(&[at.as_str(), "--js"])).unwrap();
        assert_eq!((parsed.value("sort"), parsed.switch("json"), parsed.positional.as_slice()), (Some("tag"), true, &s(&["F.ttf"])[..]));
        let parsed = args::parse(IMPORT, &s(&["F.ttf", "d.ttx", "--no-recalc"])).unwrap();
        assert!(!parsed.switch("recalc-bounds"));
        assert_eq!(tables(&s(&["list", "F.ttf", "--fix"])).unwrap_err().exit_code(), 2, "an option of another action");
        assert_eq!(tables(&s(&["list", "/no/such/font.ttf"])).unwrap_err().exit_code(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

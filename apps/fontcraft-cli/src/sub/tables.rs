//! `fontcraft-cli tables`.

use super::Subcommand;
use super::options;
use crate::io::{Failure, Outcome, note, out, usage};
use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "tables",
    summary: "Work on the tables of a compiled font",
    usage: USAGE,
    options: &[],
    actions: &["list", "extract", "add", "delete", "edit", "copy", "checksum", "import", "reorder", "show"],
    run: tables,
};

const USAGE: &str = "\
  fontcraft-cli tables list FONT [--sort file|tag|size] [--face N|all]
      Print the table directory of a compiled font, collection, WOFF or WOFF2 file as JSON.
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

/// The output options every writing action takes.
const OUTPUT: [(&str, &str); 4] = [("-o", "output"), ("--output", "output"), ("--output-dir", "outputDir"), ("--face", "face")];
const OUTPUT_FLAGS: [(&str, &str); 3] = [("--in-place", "inPlace"), ("--no-clobber", "noClobber"), ("--json", "json")];

fn with(base: &[(&'static str, &'static str)], more: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    base.iter().chain(more).copied().collect()
}

fn tables(args: &[String]) -> Outcome {
    let Some((verb, rest)) = args.split_first() else { return Err(usage("tables needs a subcommand: list")) };
    if verb == "-h" || verb == "--help" || rest.iter().any(|a| a == "-h" || a == "--help") {
        return out(&super::help(&SUBCOMMAND));
    }
    match verb.as_str() {
        "list" => list(rest),
        "extract" => extract(rest),
        "add" => add(rest),
        "delete" => delete(rest),
        "edit" => edit(rest),
        "copy" => copy(rest),
        "checksum" => checksum(rest),
        "import" => import(rest),
        "reorder" => reorder(rest),
        "show" => show(rest),
        _ => super::not_built(&SUBCOMMAND, args),
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
            note(&format!("warning: {w}"));
        }
    }
    out(&serde_json::to_string_pretty(result).map_err(|e| Failure::Failed(e.to_string()))?)
}

fn split(list: &str) -> Value {
    json!(list.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>())
}

fn drop_json(params: &mut Map<String, Value>) -> bool {
    params.remove("json").is_some()
}

fn list(rest: &[String]) -> Outcome {
    let (files, mut params) = options(rest, &[("--sort", "sort"), ("--face", "face")], &[("--json", "json")])?;
    drop_json(&mut params);
    let [font] = files.as_slice() else { return Err(usage("tables list needs one FONT")) };
    params.insert("font".into(), json!(font));
    print(&run("tables.list", params)?)
}

fn extract(rest: &[String]) -> Outcome {
    let (files, mut params) = options(rest, &OUTPUT, &with(&OUTPUT_FLAGS, &[("--all", "all")]))?;
    drop_json(&mut params);
    let Some((font, tags)) = files.split_first() else { return Err(usage("tables extract needs FONT and TAG[=FILE]...")) };
    let all = params.remove("all").is_some();
    if all == !tags.is_empty() {
        return Err(usage("tables extract needs TAG[=FILE]... or --all"));
    }
    params.remove("output");
    params.remove("inPlace");
    params.insert("font".into(), json!(font));
    params.insert("tables".into(), if all { json!("all") } else { json!(tags) });
    print(&run("tables.extract", params)?)
}

fn add(rest: &[String]) -> Outcome {
    let (files, mut params) = options(rest, &OUTPUT, &OUTPUT_FLAGS)?;
    drop_json(&mut params);
    let Some((font, tables)) = files.split_first().filter(|(_, t)| !t.is_empty()) else { return Err(usage("tables add needs FONT and TAG=FILE...")) };
    params.insert("font".into(), json!(font));
    params.insert("tables".into(), json!(tables));
    print(&run("tables.add", params)?)
}

fn import(rest: &[String]) -> Outcome {
    let flags = with(&OUTPUT_FLAGS, &[("--recalc-bounds", "recalcBounds"), ("--no-recalc-bounds", "noRecalcBounds")]);
    let (files, mut params) = options(rest, &OUTPUT, &flags)?;
    drop_json(&mut params);
    if params.remove("noRecalcBounds").is_some() {
        params.insert("recalcBounds".into(), json!(false));
    }
    let Some((font, dumps)) = files.split_first().filter(|(_, d)| !d.is_empty()) else {
        return Err(usage("tables import needs FONT and DUMP.ttx..."));
    };
    params.insert("font".into(), json!(font));
    params.insert("dumps".into(), json!(dumps));
    print(&run("tables.importTtx", params)?)
}

fn delete(rest: &[String]) -> Outcome {
    let (files, mut params) = options(rest, &OUTPUT, &OUTPUT_FLAGS)?;
    drop_json(&mut params);
    let Some((font, tables)) = files.split_first().filter(|(_, t)| !t.is_empty()) else { return Err(usage("tables delete needs FONT and TAG...")) };
    params.insert("font".into(), json!(font));
    params.insert("tables".into(), json!(tables));
    print(&run("tables.delete", params)?)
}

fn edit(rest: &[String]) -> Outcome {
    let (files, mut params) = options(rest, &with(&OUTPUT, &[("--extract", "extract"), ("--delete", "delete"), ("--add", "add")]), &OUTPUT_FLAGS)?;
    drop_json(&mut params);
    let [font] = files.as_slice() else { return Err(usage("tables edit needs one FONT")) };
    for key in ["extract", "delete", "add"] {
        if let Some(Value::String(s)) = params.get(key) {
            let list = split(s);
            params.insert(key.into(), list);
        }
    }
    params.insert("font".into(), json!(font));
    print(&run("tables.edit", params)?)
}

fn copy(rest: &[String]) -> Outcome {
    let (files, mut params) = options(rest, &with(&OUTPUT, &[("--from-face", "fromFace")]), &OUTPUT_FLAGS)?;
    drop_json(&mut params);
    let [from, to, tags @ ..] = files.as_slice() else { return Err(usage("tables copy needs FROM, TO and TAG...")) };
    if tags.is_empty() {
        return Err(usage("tables copy needs FROM, TO and TAG..."));
    }
    let mut source = json!({ "font": from });
    if let (Some(face), Some(obj)) = (params.remove("fromFace"), source.as_object_mut()) {
        obj.insert("face".into(), face);
    }
    let tables: Vec<Value> =
        tags.iter().flat_map(|t| t.split(',')).filter(|t| !t.is_empty()).map(|t| json!({ "tag": t, "from": source.clone() })).collect();
    params.insert("font".into(), json!(to));
    params.insert("tables".into(), json!(tables));
    print(&run("tables.add", params)?)
}

fn checksum(rest: &[String]) -> Outcome {
    let (files, params) = options(rest, &OUTPUT, &with(&OUTPUT_FLAGS, &[("--fix", "fix")]))?;
    if files.is_empty() {
        return Err(usage("tables checksum needs FONT..."));
    }
    let fix = params.contains_key("fix");
    if files.len() > 1 && params.contains_key("output") {
        return Err(usage("tables checksum with several fonts writes each beside its input: give --output-dir or --in-place, not -o"));
    }
    let mut wrong = Vec::new();
    for font in &files {
        let mut p = params.clone();
        drop_json(&mut p);
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

fn reorder(rest: &[String]) -> Outcome {
    let (files, mut params) = options(rest, &with(&OUTPUT, &[("--order", "order")]), &OUTPUT_FLAGS)?;
    drop_json(&mut params);
    let [font] = files.as_slice() else { return Err(usage("tables reorder needs one FONT")) };
    if let Some(Value::String(s)) = params.get("order").filter(|v| v.as_str() != Some("recommended")) {
        let list = split(s);
        params.insert("order".into(), list);
    }
    params.insert("font".into(), json!(font));
    print(&run("tables.reorder", params)?)
}

fn show(rest: &[String]) -> Outcome {
    let (files, mut params) = options(
        rest,
        &[("--path", "path"), ("--depth", "depth"), ("--format", "format"), ("--face", "face")],
        &[("--offsets", "offsets"), ("--json", "json")],
    )?;
    let json_out = drop_json(&mut params);
    let format = match params.remove("format") {
        None => if json_out { "json" } else { "text" }.to_string(),
        Some(Value::String(f)) if f == "json" || f == "text" => f,
        Some(other) => return Err(usage(format!("--format must be json or text, got {other}"))),
    };
    if let Some(Value::String(d)) = params.get("depth") {
        let depth: u64 = d.parse().map_err(|_| usage(format!("--depth must be a whole number, got {d}")))?;
        params.insert("depth".into(), json!(depth));
    }
    let Some((font, tags)) = files.split_first() else { return Err(usage("tables show needs FONT")) };
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

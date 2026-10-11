//! `fontcraft-cli dump` (M12.4, M12.10): a shell over `font.dumpTables`. The TTX, JSON and hex
//! forms are built; the text views, `@font`, `@glyphs` and `--normalize` come with M12.10.

use fontcraft_engine::{EngineError, Session};
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, note, out, out_json, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "dump", summary: "Dump tables as TTX, JSON, hex or text views", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli dump FONT... [-t TAG[=VIEW]]... [-x TAG]... [--format ttx|json|text|hex] [--split tables|glyphs] [-o PATH]
      Write the tables of a compiled font, collection face, WOFF or WOFF2 file to standard output, or
      to -o PATH or --output-dir DIR (one FONT.ttx, .json or .txt file per font). Tables that cannot
      be read are written as hex unless --strict.
";

/// The options, as data (M12.4, M12.10).
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("table", "TAG", "", "dump only these tables, in this order; repeatable").short('t'),
    OptSpec::list("exclude-table", "TAG", "", "dump every table except these; repeatable").short('x'),
    OptSpec::value("format", "FORMAT", "ttx", "ttx, json, hex or text"),
    OptSpec::value("split", "WHAT", "", "tables: one file per table beside the main file (needs -o or --output-dir)"),
    OptSpec::value("output", "PATH", "", "the file to write; - or none for standard output").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder to write FONT.ttx (or .json, .txt) into"),
    OptSpec::value("face", "N", "0", "the font of a collection"),
    OptSpec::switch("disassemble", true, "write TrueType programs as assembly (--no-disassemble: as hex)"),
    OptSpec::switch("strict", false, "stop at the first table that cannot be read"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output file"),
    OptSpec::value("newline", "lf|cr|crlf", "lf", "line ending of the output"),
];

/// Takes every `-t X`, `--table X` and `--table=X` (or the forms of `long`) out of `args`, so the
/// option can be repeated; the parser keeps only the last value of a list option.
fn take_repeated(args: &[String], short: &str, long: &str) -> (Vec<String>, Vec<String>) {
    let mut values = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    let prefix = format!("{long}=");
    while let Some(a) = it.next() {
        if a == "--" {
            rest.push(a.clone());
            rest.extend(it.by_ref().cloned());
            break;
        }
        if a == short || a == long {
            match it.next() {
                Some(v) => values.push(v.clone()),
                None => rest.push(a.clone()),
            }
        } else if let Some(v) = a.strip_prefix(&prefix) {
            values.push(v.to_string());
        } else {
            rest.push(a.clone());
        }
    }
    (values, rest)
}

fn tags(given: &[String], parsed: &[String]) -> Vec<Value> {
    given
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .chain(parsed.iter().map(String::as_str))
        .map(|t| json!(t))
        .collect()
}

fn run(args: &[String]) -> Outcome {
    let (tables, rest) = take_repeated(args, "-t", "--table");
    let (excluded, rest) = take_repeated(&rest, "-x", "--exclude-table");
    let parsed = args::parse(OPTIONS, &rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    let output = parsed.value("output").filter(|o| !o.is_empty() && *o != "-");
    if output.is_some() && parsed.positional.len() > 1 {
        return Err(usage("-o names one file: give --output-dir to dump several fonts"));
    }
    let face: u64 = parsed.value("face").unwrap_or("0").parse().map_err(|_| usage("--face takes a number"))?;
    let mut base = Map::new();
    base.insert("tables".into(), Value::Array(tags(&tables, parsed.list("table"))));
    base.insert("excludeTables".into(), Value::Array(tags(&excluded, parsed.list("exclude-table"))));
    base.insert("format".into(), json!(parsed.value("format").unwrap_or("ttx")));
    base.insert("face".into(), json!(face));
    base.insert("disassemble".into(), json!(parsed.switch("disassemble")));
    base.insert("strict".into(), json!(parsed.switch("strict")));
    base.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    base.insert("newline".into(), json!(parsed.value("newline").unwrap_or("lf")));
    if let Some(split) = parsed.value("split").filter(|s| !s.is_empty()) {
        base.insert("split".into(), json!(split));
    }
    if let Some(o) = output {
        base.insert("output".into(), json!(o));
    }
    if let Some(dir) = parsed.value("output-dir").filter(|d| !d.is_empty()) {
        base.insert("outputDir".into(), json!(dir));
    }
    let mut session = Session::new();
    for font in &parsed.positional {
        let mut params = base.clone();
        params.insert("font".into(), json!(font));
        let result = session.execute("font.dumpTables", &Value::Object(params)).map_err(|e| match e {
            EngineError::Io(m) => Failure::Input(m),
            EngineError::BadParams { message, .. } => usage(message),
            other => Failure::Failed(other.to_string()),
        })?;
        if let Some(text) = result.get("text").and_then(Value::as_str) {
            out(text.strip_suffix('\n').unwrap_or(text))?;
        } else if let Some(tree) = result.get("json") {
            out_json(tree)?;
        } else if let Some(paths) = result.get("paths").and_then(Value::as_array) {
            for p in paths.iter().filter_map(Value::as_str) {
                note(&format!("wrote {p}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_table_options_are_all_kept() {
        let args: Vec<String> = ["A.ttf", "-t", "head", "--table=name", "-x", "glyf", "--table", "OS/2,cmap"].iter().map(|s| s.to_string()).collect();
        let (tables, rest) = take_repeated(&args, "-t", "--table");
        assert_eq!(tables, ["head", "name", "OS/2,cmap"]);
        assert_eq!(tags(&tables, &[]), vec![json!("head"), json!("name"), json!("OS/2"), json!("cmap")]);
        let (excluded, rest) = take_repeated(&rest, "-x", "--exclude-table");
        assert_eq!(excluded, ["glyf"]);
        assert_eq!(rest, ["A.ttf"]);
        let e = run(&["missing.ttf".to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), 3, "an unreadable input is exit status 3");
    }
}

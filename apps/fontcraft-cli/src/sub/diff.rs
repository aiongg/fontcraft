//! `fontcraft-cli diff` (M12.32): two fonts, or two folders of fonts, compared table by table
//! through `font.diff`. Exit status 0 when no table differs after the chosen normalisation, 1
//! when one does.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "diff", summary: "Compare two fonts table by table", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli diff A B [--summary | --detail LEVEL] [-t TAG]... [-x TAG]... [--normalize none|dump|compiler] [--context N] [--output-dir DIR]
";

/// The options, as data.
const OPTIONS: &[OptSpec] = &[
    OptSpec::switch("summary", false, "only say, per table, whether it is on each side and whether the bytes are equal"),
    OptSpec::value(
        "detail",
        "LEVEL",
        "fields",
        "tables, offset (first differing byte), hex (diff of hex dumps), fields (diff of the decoded tables) or verbose (fields with every line)",
    ),
    OptSpec::list("table", "TAG", "", "compare only these tables").short('t'),
    OptSpec::list("exclude-table", "TAG", "", "compare all but these tables").short('x'),
    OptSpec::switch("compare-identical", false, "also decode and compare tables whose bytes are equal"),
    OptSpec::value("context", "N", "3", "unchanged lines shown around each difference"),
    OptSpec::value("normalize", "MODE", "none", "none, dump (the normalised dump) or compiler (hide what two compilers may do differently)"),
    OptSpec::switch("hints", true, "with --normalize, compare hinting data (--no-hints leaves it out)"),
    OptSpec::switch(
        "version",
        true,
        "with --normalize compiler, compare the font revision and the version and unique-ID names (--no-version leaves them out)",
    ),
    OptSpec::switch("show-extensions", false, "keep Extension lookup wrappers"),
    OptSpec::switch("unwrap-extensions", false, "strip Extension lookup wrappers (the default with --normalize compiler)"),
    OptSpec::value(
        "off-by-one-budget",
        "PCT",
        "0.1",
        "with --normalize compiler, the share of outline coordinates and deltas that may differ by one unit",
    ),
    OptSpec::value("size-threshold", "PCT", "10", "report tables whose size in B exceeds A's by more than this share"),
    OptSpec::switch("similarity", false, "print a similarity ratio for each differing table instead of the diff"),
    OptSpec::value("tool", "COMMAND", "", "an external diff program (not supported: use --output-dir and run it on the files)"),
    OptSpec::value("tool-args", "ARGS", "", "arguments for --tool"),
    OptSpec::value("face-a", "N|NAME", "", "the face of A when it is a collection"),
    OptSpec::value("face-b", "N|NAME", "", "the face of B when it is a collection"),
    OptSpec::switch("show-mtime", false, "print each file's modification time in the header"),
    OptSpec::value("output-dir", "DIR", "", "write the compared texts as A.TABLE.txt and B.TABLE.txt"),
    OptSpec::switch("all", false, "list every difference instead of stopping at 1000 lines (64 KiB of hex) per table"),
    OptSpec::switch("json", false, "print the engine command's result as JSON"),
    OptSpec::switch("quiet", false, "print nothing: the exit status says whether the fonts differ").short('q'),
];

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let [a, b] = parsed.positional.as_slice() else { return Err(usage("diff needs two fonts (or folders) A and B")) };
    let params = params(&parsed, a, b)?;
    let result = execute(params)?;
    let identical = result.get("identical").and_then(Value::as_bool).unwrap_or(false);
    let quiet = parsed.switch("quiet");
    if !quiet {
        if parsed.switch("json") {
            io::out_json(&result)?;
        } else {
            print(&parsed, a, b, &result)?;
        }
    }
    if identical {
        return Ok(());
    }
    Err(Failure::Found(if quiet { String::new() } else { "the fonts differ".into() }))
}

fn params(parsed: &args::Parsed, a: &str, b: &str) -> Result<Map<String, Value>, Failure> {
    let mut p = Map::new();
    p.insert("a".into(), json!(a));
    p.insert("b".into(), json!(b));
    let detail = match (parsed.switch("summary"), parsed.value("detail")) {
        (true, Some(d)) if d != "fields" && d != "tables" => return Err(usage("--summary and --detail cannot be combined")),
        (true, _) => "tables",
        (false, d) => d.unwrap_or("fields"),
    };
    p.insert("detail".into(), json!(detail));
    let (tables, skip) = (parsed.list("table"), parsed.list("exclude-table"));
    if !tables.is_empty() && !skip.is_empty() {
        return Err(usage("-t and -x cannot be combined"));
    }
    if !tables.is_empty() {
        p.insert("tables".into(), json!(tables));
    }
    if !skip.is_empty() {
        p.insert("excludeTables".into(), json!(skip));
    }
    let context = parsed.value("context").unwrap_or("3");
    let context: u64 = context.parse().map_err(|_| usage(format!("--context takes a whole number, got `{context}`")))?;
    p.insert("context".into(), json!(context));
    if let Some(n) = parsed.value("normalize") {
        p.insert("normalize".into(), json!(n));
    }
    p.insert("hints".into(), json!(parsed.switch("hints")));
    p.insert("version".into(), json!(parsed.switch("version")));
    match (parsed.switch("show-extensions"), parsed.switch("unwrap-extensions")) {
        (true, true) => return Err(usage("--show-extensions and --unwrap-extensions cannot be combined")),
        (true, false) => {
            p.insert("unwrapExtensions".into(), json!(false));
        }
        (false, true) => {
            p.insert("unwrapExtensions".into(), json!(true));
        }
        (false, false) => {}
    }
    for (option, key) in [("off-by-one-budget", "offByOneBudget"), ("size-threshold", "sizeThreshold")] {
        if let Some(v) = parsed.value(option) {
            let n: f64 = v.parse().ok().filter(|n: &f64| n.is_finite()).ok_or_else(|| usage(format!("--{option} takes a number, got `{v}`")))?;
            p.insert(key.into(), json!(n));
        }
    }
    if let Some(tool) = parsed.value("tool") {
        let mut t = Map::new();
        t.insert("command".into(), json!(tool));
        if let Some(args) = parsed.value("tool-args") {
            t.insert("args".into(), json!(args));
        }
        p.insert("tool".into(), Value::Object(t));
    }
    for (option, key) in [("face-a", "faceA"), ("face-b", "faceB")] {
        if let Some(face) = parsed.value(option) {
            p.insert(key.into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
        }
    }
    if let Some(dir) = parsed.value("output-dir") {
        p.insert("outputDir".into(), json!(dir));
    }
    p.insert("similarity".into(), json!(parsed.switch("similarity")));
    p.insert("all".into(), json!(parsed.switch("all")));
    Ok(p)
}

/// Runs `font.diff`, mapping its errors to exit statuses.
fn execute(params: Map<String, Value>) -> Result<Value, Failure> {
    let mut session = Session::new();
    session.execute("font.diff", &Value::Object(params)).map_err(|e| match e {
        fontcraft_engine::EngineError::BadParams { .. } => Failure::Usage(e.to_string()),
        // A comparison whose part has not landed yet is a failed run, not an unreadable input.
        fontcraft_engine::EngineError::Tables(ref t) if t.to_string().ends_with("is not built yet") => Failure::Failed(e.to_string()),
        fontcraft_engine::EngineError::Io(_) | fontcraft_engine::EngineError::Tables(_) => Failure::Input(e.to_string()),
        other => Failure::Failed(other.to_string()),
    })
}

fn print(parsed: &args::Parsed, a: &str, b: &str, result: &Value) -> Outcome {
    let mtime = |path: &str| -> String {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| format!("\t{} s since 1970", d.as_secs()))
            .unwrap_or_default()
    };
    let show_mtime = parsed.switch("show-mtime");
    out(&format!("--- A: {a}{}", if show_mtime { mtime(a) } else { String::new() }))?;
    out(&format!("+++ B: {b}{}", if show_mtime { mtime(b) } else { String::new() }))?;
    if let Some(pairs) = result.get("pairs").and_then(Value::as_array) {
        for name in result.get("onlyA").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            out(&format!("{name}: only in A"))?;
        }
        for name in result.get("onlyB").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            out(&format!("{name}: only in B"))?;
        }
        for pair in pairs {
            let name = pair.get("name").and_then(Value::as_str).unwrap_or_default();
            out(&format!("=== {name}"))?;
            print_tables(pair)?;
        }
        return Ok(());
    }
    print_tables(result)
}

/// One line per table that is not the same, then its difference lines.
fn print_tables(result: &Value) -> Outcome {
    let Some(tables) = result.get("tables").and_then(Value::as_object) else { return Ok(()) };
    for (name, t) in tables {
        let int = |key: &str| t.get(key).and_then(Value::as_u64);
        if let (true, Some(sa), Some(sb)) = (t.get("oversized").and_then(Value::as_bool).unwrap_or(false), int("sizeA"), int("sizeB")) {
            out(&format!("{name}: B is {} bytes larger ({sa} → {sb})", sb.saturating_sub(sa)))?;
        }
        let status = t.get("status").and_then(Value::as_str).unwrap_or_default();
        let mut line = match status {
            "same" => continue,
            "onlyA" => format!("{name}: only in A"),
            "onlyB" => format!("{name}: only in B"),
            _ => format!("{name}: different"),
        };
        if let (Some(sa), Some(sb)) = (int("sizeA"), int("sizeB")) {
            line.push_str(&format!(" ({sa} and {sb} bytes)"));
        }
        if let Some(offset) = int("firstOffset") {
            line.push_str(&format!(", first difference at byte {offset} (0x{offset:x})"));
        }
        if let Some(ratio) = t.get("similarity").and_then(Value::as_f64) {
            line.push_str(&format!(", similarity {ratio:.4}"));
        }
        out(&line)?;
        for l in t.get("lines").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            out(l)?;
        }
        if let Some(n) = int("truncated").filter(|n| *n > 0) {
            out(&format!("… {n} more removed or added lines left out (--all lists them)"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A small font written by the engine, at `path`.
    fn export(dir: &std::path::Path, name: &str, width: f64) -> String {
        let mut s = Session::with_document();
        let rect = json!([{"x": 0, "y": 0}, {"x": width, "y": 0}, {"x": width, "y": 700}, {"x": 0, "y": 700}]);
        s.execute("path.addPath", &json!({"glyph": "A", "nodes": rect})).unwrap();
        let path = dir.join(name);
        s.execute("file.exportStatic", &json!({"destination": path, "timestamp": 0})).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn diff_exit_statuses() {
        assert!((SUBCOMMAND.run)(&a(&["--help"])).is_ok());
        assert_eq!((SUBCOMMAND.run)(&a(&["one.ttf"])).unwrap_err().exit_code(), 2, "one font is a usage error");
        assert_eq!((SUBCOMMAND.run)(&a(&["x.ttf", "y.ttf", "-t", "head", "-x", "name"])).unwrap_err().exit_code(), 2);
        let missing = std::env::temp_dir().join(format!("fontcraft-no-such-font-{}.ttf", std::process::id()));
        let missing = missing.to_string_lossy();
        assert_eq!((SUBCOMMAND.run)(&a(&[&missing, &missing])).unwrap_err().exit_code(), 3);

        let dir = std::env::temp_dir().join(format!("fontcraft-cli-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let narrow = export(&dir, "narrow.ttf", 500.0);
        let wide = export(&dir, "wide.ttf", 600.0);
        assert!((SUBCOMMAND.run)(&a(&[&narrow, &narrow, "-q"])).is_ok(), "a font equals itself: exit 0");
        let e = (SUBCOMMAND.run)(&a(&[&narrow, &wide, "-q"])).unwrap_err();
        assert_eq!((e.exit_code(), e.message()), (1, ""), "differences: exit 1, quietly");
        assert_eq!((SUBCOMMAND.run)(&a(&[&narrow, &wide, "--summary", "--json"])).unwrap_err().exit_code(), 1);
        assert_eq!((SUBCOMMAND.run)(&a(&[&narrow, &wide, "--tool", "meld"])).unwrap_err().exit_code(), 2, "--tool is refused");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

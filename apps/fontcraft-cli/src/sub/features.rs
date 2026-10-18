//! `fontcraft-cli features` (M12.11–M12.14). `list`, `decompile`, `convert`, `normalize`,
//! `compile` and `lint` run `layout.listFeatures`, `layout.decompileFeatures`,
//! `layout.convertSyntax`, `layout.normalize`, `layout.compileFeatures` and
//! `layout.lintFeatures`; an action whose
//! package has not landed prints its usage with `--help` and exits 4 with "not built yet" otherwise.

use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "features",
    summary: "List, compile, decompile, check, convert and normalise layout features",
    usage: USAGE,
    options: OPTIONS,
    actions: &["list", "compile", "decompile", "lint", "convert", "normalize"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli features list|compile|decompile|lint|convert|normalize INPUT [args]
  fontcraft-cli features list FONT [--table GSUB|GPOS] [--lookups] [--json]
  fontcraft-cli features compile FEATURES... --font FONT [-o OUT] [--tables TAG,...] [--debug-lookups] [--include-dir DIR] [--syntax fea|mti|volt] [--goadb FILE] [--emit-fea FILE] [--ship] [--to font|ttx]
  fontcraft-cli features decompile FONT [--table GSUB|GPOS|GDEF|BASE,...] [--features TAG,...] [--show-extensions] [-o OUT.fea]
  fontcraft-cli features lint FEATURES [--font FONT | --glyph-order FILE] [--include-dir DIR] [--warnings default|all] [--json]
  fontcraft-cli features convert INPUT --to fea|ttx [-o OUT] [--table GSUB|GPOS|GDEF,...]
  fontcraft-cli features normalize FONT [--table gpos|gsub|gdef|all] [--location AXIS=VALUE,...] [--face N|NAME] [-o OUT] [--no-clobber]
";

/// The options, as data (M12.11–M12.14).
const OPTIONS: &[OptSpec] = &[];

/// The options of `features normalize` (M12.11).
const NORMALIZE_OPTIONS: &[OptSpec] = &[
    OptSpec::value("table", "TABLE", "all", "which part to print: gpos, gsub, gdef or all"),
    OptSpec::list(
        "location",
        "AXIS=VALUE",
        "",
        "resolve variable values at this location (user coordinates) instead of printing each master's value",
    ),
    OptSpec::value("face", "N|NAME", "", "the face of a collection, by number or PostScript name"),
    OptSpec::value("output", "FILE", "", "write the text to FILE instead of stdout").short('o'),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output file"),
];

/// The options of `features list` (M12.12).
const LIST_OPTIONS: &[OptSpec] = &[
    OptSpec::value("table", "GSUB|GPOS", "", "only this table"),
    OptSpec::switch("lookups", false, "add the lookups behind each feature with their types"),
    OptSpec::value("by", "feature|script", "feature", "group by feature tag or by script and language"),
    OptSpec::value("face", "N", "", "the face of a collection, by number"),
    OptSpec::switch("json", false, "print the engine command's result as JSON"),
];

/// The options of `features decompile` (M12.12).
const DECOMPILE_OPTIONS: &[OptSpec] = &[
    OptSpec::list("table", "TAG", "", "tables to decompile: GSUB, GPOS, GDEF, BASE; default all the font has"),
    OptSpec::list("features", "TAG", "", "only these features and the lookups they reach"),
    OptSpec::switch("show-extensions", false, "mark lookups stored through extension lookups"),
    OptSpec::value("names", "final|source", "final", "source: write development names mapped back through --goadb"),
    OptSpec::value("goadb", "FILE", "", "the glyph name file for --names source"),
    OptSpec::value("classes", "auto|inline", "auto", "name glyph sets used more than once, or write them inline"),
    OptSpec::value("face", "N", "", "the face of a collection, by number"),
    OptSpec::value("output", "FILE", "", "write the feature code to FILE instead of stdout").short('o'),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output file"),
];

/// The options of `features convert` (M12.13).
const CONVERT_OPTIONS: &[OptSpec] = &[
    OptSpec::value("to", "fea|ttx", "fea", "feature code, or the TTX text of the tables that code compiles to (needs --font)"),
    OptSpec::list("table", "TAG", "", "only these of GDEF, GSUB, GPOS; for one FontDame file without a header line, the table it describes"),
    OptSpec::value("font", "FONT", "", "the font whose glyph order FontDame `# N` references and class 0 use; required by --to ttx"),
    OptSpec::value("face", "N", "", "the face of a collection given as --font"),
    OptSpec::value("output", "FILE", "", "write the result to FILE instead of stdout").short('o'),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output file"),
];

fn run(args: &[String]) -> Outcome {
    match args.first().map(String::as_str) {
        Some("normalize") => normalize(args.get(1..).unwrap_or_default()),
        Some("list") => list(args.get(1..).unwrap_or_default()),
        Some("decompile") => decompile(args.get(1..).unwrap_or_default()),
        Some("compile") => compile(args.get(1..).unwrap_or_default()),
        Some("lint") => lint(args.get(1..).unwrap_or_default()),
        Some("convert") => convert(args.get(1..).unwrap_or_default()),
        _ => super::not_built(&SUBCOMMAND, args),
    }
}

/// Runs one engine command, mapping its errors to exit statuses.
fn execute(command: &str, params: serde_json::Map<String, Value>) -> Result<Value, Failure> {
    let mut session = Session::new();
    session.execute(command, &Value::Object(params)).map_err(|e| match e {
        fontcraft_engine::EngineError::BadParams { .. } => Failure::Usage(e.to_string()),
        fontcraft_engine::EngineError::Io(_) | fontcraft_engine::EngineError::Tables(_) => Failure::Input(e.to_string()),
        other => Failure::Failed(other.to_string()),
    })
}

fn face_param(params: &mut serde_json::Map<String, Value>, face: Option<&str>) -> Result<(), Failure> {
    if let Some(face) = face {
        let n: u64 = face.parse().map_err(|_| usage(format!("--face takes a whole number, got `{face}`")))?;
        params.insert("face".into(), json!(n));
    }
    Ok(())
}

/// `features list FONT`: the scripts, language systems and features (`layout.listFeatures`).
fn list(args: &[String]) -> Outcome {
    let parsed = args::parse(LIST_OPTIONS, args)?;
    if parsed.help {
        return out(&format!("{}\n{}", super::help(&SUBCOMMAND), args::help("", LIST_OPTIONS)));
    }
    let [font] = parsed.positional.as_slice() else { return Err(usage("features list needs one FONT")) };
    let mut params = serde_json::Map::new();
    params.insert("font".into(), json!(font));
    if let Some(table) = parsed.value("table") {
        params.insert("table".into(), json!(table));
    }
    params.insert("lookups".into(), json!(parsed.switch("lookups")));
    if let Some(by) = parsed.value("by") {
        params.insert("by".into(), json!(by));
    }
    face_param(&mut params, parsed.value("face"))?;
    let result = execute("layout.listFeatures", params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let lookups = |v: &Value| -> String {
        let Some(list) = v.get("lookups").and_then(Value::as_array) else { return String::new() };
        let items: Vec<String> = list
            .iter()
            .map(|l| {
                let index = l.get("index").and_then(Value::as_u64).unwrap_or_default();
                match l.get("type").and_then(Value::as_u64) {
                    Some(t) => format!("{index} (type {t})"),
                    None => index.to_string(),
                }
            })
            .collect();
        format!(": lookups {}", items.join(", "))
    };
    for f in result.get("features").and_then(Value::as_array).into_iter().flatten() {
        out(&format!("{} {}", text(f, "table"), text(f, "feature")))?;
        for s in f.get("systems").and_then(Value::as_array).into_iter().flatten() {
            out(&format!("  {}/{}{}", text(s, "script"), text(s, "language"), lookups(s)))?;
        }
    }
    for s in result.get("systems").and_then(Value::as_array).into_iter().flatten() {
        out(&format!("{} {}/{}", text(s, "table"), text(s, "script"), text(s, "language")))?;
        for f in s.get("features").and_then(Value::as_array).into_iter().flatten() {
            out(&format!("  {}{}", text(f, "feature"), lookups(f)))?;
        }
    }
    Ok(())
}

/// The options of `features compile` (M12.14).
const COMPILE_OPTIONS: &[OptSpec] = &[
    OptSpec::value("font", "FONT", "", "the font that gives the glyph set and receives the tables"),
    OptSpec::value("output", "FILE", "", "write the result to FILE (default: the font's name with .features)").short('o'),
    OptSpec::switch("in-place", false, "replace the font"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output file"),
    OptSpec::list("tables", "TAG", "", "build only these of BASE, GDEF, GPOS, GSUB, OS/2, head, hhea, name, vhea, STAT"),
    OptSpec::switch("debug-lookups", false, "add a table recording where each lookup is defined (not built yet)"),
    OptSpec::value("include-dir", "DIR", "", "the folder include() paths are read from (default: the source's folder)"),
    OptSpec::value("syntax", "fea|mti|volt", "", "the source's syntax (default: told from its first line)"),
    OptSpec::value("goadb", "FILE", "", "the source names glyphs by the development names this file maps"),
    OptSpec::value("emit-fea", "FILE", "", "for FontDame or VOLT input, also write the feature code it became"),
    OptSpec::switch("ship", false, "remove the VOLT source tables (TSIV, TSIS, TSIP, TSID)"),
    OptSpec::value("to", "font|ttx", "font", "write a font, or the built tables as TTX text"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection, by number or PostScript name"),
    OptSpec::switch("json", false, "print the engine command's result as JSON"),
];

/// The options of `features lint` (M12.14).
const LINT_OPTIONS: &[OptSpec] = &[
    OptSpec::value("font", "FONT", "", "check glyph names against this font"),
    OptSpec::value("glyph-order", "FILE", "", "check glyph names against this list (the first word of each line)"),
    OptSpec::value("include-dir", "DIR", "", "the folder include() paths are read from (default: the source's folder)"),
    OptSpec::value("warnings", "default|all", "default", "all: add notes on usually harmless conditions"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection, by number or PostScript name"),
    OptSpec::switch("json", false, "print the engine command's result as JSON"),
];

/// `file:line:column: severity: message` for one diagnostic of a compile or lint result.
fn diagnostic_line(d: &Value) -> String {
    let text = |key: &str| d.get(key).and_then(Value::as_str).unwrap_or_default();
    let place = match (d.get("file").and_then(Value::as_str), d.get("line").and_then(Value::as_u64), d.get("column").and_then(Value::as_u64)) {
        (Some(file), Some(line), Some(column)) => format!("{file}:{line}:{column}: "),
        (Some(file), _, _) => format!("{file}: "),
        _ => String::new(),
    };
    format!("{place}{}: {}", text("severity"), text("message"))
}

fn face_value(params: &mut serde_json::Map<String, Value>, face: Option<&str>) {
    if let Some(face) = face {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
}

/// `features compile FEATURES... --font FONT`: the code compiled into the font
/// (`layout.compileFeatures`).
fn compile(args: &[String]) -> Outcome {
    let parsed = args::parse(COMPILE_OPTIONS, args)?;
    if parsed.help {
        return out(&format!("{}\n{}", super::help(&SUBCOMMAND), args::help("", COMPILE_OPTIONS)));
    }
    if parsed.positional.is_empty() {
        return Err(usage("features compile needs FEATURES (one feature file, VOLT project, or one FontDame file per table)"));
    }
    let Some(font) = parsed.value("font") else { return Err(usage("features compile needs --font FONT")) };
    let mut params = serde_json::Map::new();
    match parsed.positional.as_slice() {
        [one] => params.insert("source".into(), json!(one)),
        many => params.insert("source".into(), json!(many)),
    };
    params.insert("font".into(), json!(font));
    let tables = parsed.list("tables");
    if !tables.is_empty() {
        params.insert("tables".into(), json!(tables));
    }
    for (option, key) in
        [("output", "output"), ("include-dir", "includeDir"), ("syntax", "syntax"), ("goadb", "goadb"), ("emit-fea", "emitFea"), ("to", "to")]
    {
        if let Some(v) = parsed.value(option) {
            params.insert(key.into(), json!(v));
        }
    }
    for (option, key) in [("in-place", "inPlace"), ("no-clobber", "noClobber"), ("debug-lookups", "debugLookups"), ("ship", "ship")] {
        params.insert(key.into(), json!(parsed.switch(option)));
    }
    face_value(&mut params, parsed.value("face"));
    let result = execute("layout.compileFeatures", params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    for d in result.get("diagnostics").and_then(Value::as_array).into_iter().flatten() {
        io::note(&diagnostic_line(d));
    }
    for w in result.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        io::note(w);
    }
    let tables: Vec<&str> = result.get("tables").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let output = result.get("output").and_then(Value::as_str).unwrap_or_default();
    io::note(&format!("wrote {output} ({})", if tables.is_empty() { "no table built".to_string() } else { tables.join(", ") }));
    Ok(())
}

/// `features lint FEATURES`: the code checked without building a font (`layout.lintFeatures`).
/// Exit status 1 when it has errors.
fn lint(args: &[String]) -> Outcome {
    let parsed = args::parse(LINT_OPTIONS, args)?;
    if parsed.help {
        return out(&format!("{}\n{}", super::help(&SUBCOMMAND), args::help("", LINT_OPTIONS)));
    }
    let [source] = parsed.positional.as_slice() else { return Err(usage("features lint needs one FEATURES file")) };
    let mut params = serde_json::Map::new();
    params.insert("source".into(), json!(source));
    for (option, key) in [("font", "font"), ("glyph-order", "glyphOrder"), ("include-dir", "includeDir"), ("warnings", "warnings")] {
        if let Some(v) = parsed.value(option) {
            params.insert(key.into(), json!(v));
        }
    }
    face_value(&mut params, parsed.value("face"));
    let result = execute("layout.lintFeatures", params)?;
    if parsed.switch("json") {
        io::out_json(&result)?;
    } else {
        for d in result.get("diagnostics").and_then(Value::as_array).into_iter().flatten() {
            out(&diagnostic_line(d))?;
        }
    }
    match result.get("errors").and_then(Value::as_u64).unwrap_or_default() {
        0 => Ok(()),
        1 => Err(Failure::Found("the feature code has 1 error".into())),
        n => Err(Failure::Found(format!("the feature code has {n} errors"))),
    }
}

/// `features decompile FONT`: the layout tables as feature code (`layout.decompileFeatures`).
fn decompile(args: &[String]) -> Outcome {
    let parsed = args::parse(DECOMPILE_OPTIONS, args)?;
    if parsed.help {
        return out(&format!("{}\n{}", super::help(&SUBCOMMAND), args::help("", DECOMPILE_OPTIONS)));
    }
    let [font] = parsed.positional.as_slice() else { return Err(usage("features decompile needs one FONT")) };
    let mut params = serde_json::Map::new();
    params.insert("font".into(), json!(font));
    let tables = parsed.list("table");
    if !tables.is_empty() {
        params.insert("tables".into(), json!(tables));
    }
    let features = parsed.list("features");
    if !features.is_empty() {
        params.insert("features".into(), json!(features));
    }
    params.insert("showExtensions".into(), json!(parsed.switch("show-extensions")));
    for (option, key) in [("names", "names"), ("goadb", "goadb"), ("classes", "classes")] {
        if let Some(v) = parsed.value(option) {
            params.insert(key.into(), json!(v));
        }
    }
    face_param(&mut params, parsed.value("face"))?;
    if let Some(output) = parsed.value("output") {
        params.insert("output".into(), json!(output));
        params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    }
    let result = execute("layout.decompileFeatures", params)?;
    for item in result.get("unsupported").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        io::note(&format!("not written as feature code: {item}"));
    }
    match result.get("text").and_then(Value::as_str) {
        Some(text) if !text.is_empty() => out(text.strip_suffix('\n').unwrap_or(text)),
        _ => Ok(()),
    }
}

/// `features convert INPUT...`: a VOLT project or FontDame layout text as feature code or TTX
/// (`layout.convertSyntax`).
fn convert(args: &[String]) -> Outcome {
    let parsed = args::parse(CONVERT_OPTIONS, args)?;
    if parsed.help {
        return out(&format!("{}\n{}", super::help(&SUBCOMMAND), args::help("", CONVERT_OPTIONS)));
    }
    if parsed.positional.is_empty() {
        return Err(usage("features convert needs at least one INPUT"));
    }
    let mut params = serde_json::Map::new();
    params.insert("input".into(), json!(parsed.positional));
    let tables = parsed.list("table");
    if !tables.is_empty() {
        params.insert("tables".into(), json!(tables));
    }
    for (option, key) in [("to", "to"), ("font", "font")] {
        if let Some(v) = parsed.value(option) {
            params.insert(key.into(), json!(v));
        }
    }
    face_param(&mut params, parsed.value("face"))?;
    if let Some(output) = parsed.value("output") {
        params.insert("output".into(), json!(output));
        params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    }
    let result = execute("layout.convertSyntax", params)?;
    for warning in result.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        io::note(warning);
    }
    match result.get("text").and_then(Value::as_str) {
        Some(text) if !text.is_empty() => out(text.strip_suffix('\n').unwrap_or(text)),
        _ => Ok(()),
    }
}

/// `features normalize FONT`: the layout rules as canonical text (`layout.normalize`).
fn normalize(args: &[String]) -> Outcome {
    let parsed = args::parse(NORMALIZE_OPTIONS, args)?;
    if parsed.help {
        return out(&format!("{}\n{}", super::help(&SUBCOMMAND), args::help("", NORMALIZE_OPTIONS)));
    }
    let [font] = parsed.positional.as_slice() else { return Err(usage("features normalize needs one FONT")) };
    let mut params = serde_json::Map::new();
    params.insert("font".into(), json!(font));
    if let Some(table) = parsed.value("table") {
        params.insert("table".into(), json!(table));
    }
    if let Some(face) = parsed.value("face") {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
    let mut location = serde_json::Map::new();
    for item in parsed.list("location") {
        let Some((axis, value)) = item.split_once('=') else { return Err(usage(format!("--location takes AXIS=VALUE, got `{item}`"))) };
        let value: f64 = value.trim().parse().map_err(|_| usage(format!("--location {axis}: `{value}` is not a number")))?;
        location.insert(axis.trim().to_string(), json!(value));
    }
    if !location.is_empty() {
        params.insert("location".into(), Value::Object(location));
    }
    if let Some(output) = parsed.value("output") {
        params.insert("output".into(), json!(output));
        params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    }
    let result = execute("layout.normalize", params)?;
    match result.get("text").and_then(Value::as_str) {
        Some(text) if !text.is_empty() => out(text.strip_suffix('\n').unwrap_or(text)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_normalize_exit_statuses() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!((SUBCOMMAND.run)(&a(&["normalize", "--help"])).is_ok());
        assert_eq!((SUBCOMMAND.run)(&a(&["normalize"])).unwrap_err().exit_code(), 2, "no FONT is a usage error");
        assert_eq!((SUBCOMMAND.run)(&a(&["normalize", "f.ttf", "--location", "wght"])).unwrap_err().exit_code(), 2);
        let missing = std::env::temp_dir().join(format!("fontcraft-no-such-font-{}.ttf", std::process::id()));
        let e = (SUBCOMMAND.run)(&a(&["normalize", &missing.to_string_lossy()])).unwrap_err();
        assert_eq!(e.exit_code(), 3, "{}", e.message());
    }

    #[test]
    fn features_compile_and_lint_exit_statuses() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for action in ["compile", "lint"] {
            assert!((SUBCOMMAND.run)(&a(&[action, "--help"])).is_ok());
            assert_eq!((SUBCOMMAND.run)(&a(&[action])).unwrap_err().exit_code(), 2, "{action}: no FEATURES is a usage error");
        }
        assert_eq!((SUBCOMMAND.run)(&a(&["compile", "f.fea"])).unwrap_err().exit_code(), 2, "compile needs --font");
        let dir = std::env::temp_dir().join(format!("fontcraft-features-lint-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.fea");
        assert_eq!((SUBCOMMAND.run)(&a(&["lint", &missing.to_string_lossy()])).unwrap_err().exit_code(), 3);
        let good = dir.join("good.fea");
        std::fs::write(&good, "feature liga {\n  sub f i by f_i;\n} liga;\n").unwrap();
        assert!((SUBCOMMAND.run)(&a(&["lint", &good.to_string_lossy()])).is_ok(), "glyph names are not checked without a glyph set");
        let bad = dir.join("bad.fea");
        std::fs::write(&bad, "feature liga {\n  sub f i by f_i\n} liga;\n").unwrap();
        assert_eq!((SUBCOMMAND.run)(&a(&["lint", &bad.to_string_lossy()])).unwrap_err().exit_code(), 1, "errors found");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn features_list_and_decompile_exit_statuses() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for action in ["list", "decompile"] {
            assert!((SUBCOMMAND.run)(&a(&[action, "--help"])).is_ok());
            assert_eq!((SUBCOMMAND.run)(&a(&[action])).unwrap_err().exit_code(), 2, "{action}: no FONT is a usage error");
            let missing = std::env::temp_dir().join(format!("fontcraft-no-such-font-{}.ttf", std::process::id()));
            let e = (SUBCOMMAND.run)(&a(&[action, &missing.to_string_lossy()])).unwrap_err();
            assert_eq!(e.exit_code(), 3, "{action}: {}", e.message());
        }
    }

    #[test]
    fn features_convert_exit_statuses() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!((SUBCOMMAND.run)(&a(&["convert", "--help"])).is_ok());
        assert_eq!((SUBCOMMAND.run)(&a(&["convert"])).unwrap_err().exit_code(), 2, "no INPUT is a usage error");
        assert_eq!((SUBCOMMAND.run)(&a(&["convert", "x.vtp", "--to", "pdf"])).unwrap_err().exit_code(), 2);
        let missing = std::env::temp_dir().join(format!("fontcraft-no-such-project-{}.vtp", std::process::id()));
        let e = (SUBCOMMAND.run)(&a(&["convert", &missing.to_string_lossy()])).unwrap_err();
        assert_eq!(e.exit_code(), 3, "{}", e.message());
    }
}

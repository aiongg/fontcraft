//! `fontcraft-cli features` (M12.11–M12.14). `normalize` runs `layout.normalize`; an action whose
//! package has not landed prints its usage with `--help` and exits 4 with "not built yet" otherwise.

use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, out, usage};

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
  fontcraft-cli features compile FEATURES --font FONT [-o OUT] [--tables TAG,...] [--debug-lookups] [--include-dir DIR] [--syntax fea|mti|volt]
  fontcraft-cli features decompile FONT [--table GSUB|GPOS|GDEF|BASE,...] [--features TAG,...] [--show-extensions] [-o OUT.fea]
  fontcraft-cli features lint FEATURES [--font FONT | --glyph-order FILE] [--include-dir DIR] [--warnings default|all]
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

fn run(args: &[String]) -> Outcome {
    match args.first().map(String::as_str) {
        Some("normalize") => normalize(args.get(1..).unwrap_or_default()),
        // M12.12–M12.14: the other actions.
        _ => super::not_built(&SUBCOMMAND, args),
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
    let mut session = Session::new();
    let result = session.execute("layout.normalize", &Value::Object(params)).map_err(|e| match e {
        fontcraft_engine::EngineError::BadParams { .. } => Failure::Usage(e.to_string()),
        fontcraft_engine::EngineError::Io(_) | fontcraft_engine::EngineError::Tables(_) => Failure::Input(e.to_string()),
        other => Failure::Failed(other.to_string()),
    })?;
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
}

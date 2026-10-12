//! `fontcraft-cli dehint` (M12.27): one `hints.remove` run per input.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, batch, batch_outcome, execute, out, out_json_line, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "dehint", summary: "Remove hints and instructions", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli dehint INPUT... [--glyphs LIST] [--keep-font-hints] [--hinting-tables TAG,...] [-o OUT]
      Remove CFF stems, hint masks and flex, or TrueType instructions, from compiled fonts.
      Without a glyph selection the font-wide hinting data goes too: the CFF zones and stems,
      or the TrueType font programs, cvt, hdmx, VDMX and LTSH, the maxp hinting limits and the
      GPOS device tables. Each output is INPUT.dehinted.EXT unless -o, --output-dir or
      --in-place says otherwise.
";

/// The options, as data (M12.27).
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("glyphs", "SELECTOR", "", "remove hints only from these glyphs (font-wide data is then kept)"),
    OptSpec::list("exclude-glyphs", "SELECTOR", "", "leave these glyphs' hints alone (font-wide data is then kept)"),
    OptSpec::switch("keep-font-hints", false, "keep CFF zones and stems, or the TrueType fpgm, prep, cvt and cvar"),
    OptSpec::list("hinting-tables", "TAG", "cvt,cvar,fpgm,prep,hdmx,VDMX", "font-wide tables to delete"),
    OptSpec::switch("keep-flex", false, "keep flex in CFF outlines"),
    OptSpec::value("gasp", "keep|smooth|remove", "keep", "leave gasp alone, replace it with one smoothing range for all sizes, or delete it"),
    OptSpec::value("output", "OUT", "", "the output file (one input only)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder outputs go into"),
    OptSpec::switch("in-place", false, "replace each input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection"),
    OptSpec::switch("json", false, "print one JSON result per input"),
    OptSpec::value("jobs", "N", "1", "inputs processed at once").short('j'),
];

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    if parsed.value("output").is_some() && parsed.positional.len() > 1 {
        return Err(usage("-o names one output: give one input, or use --output-dir"));
    }
    let mut params = Map::new();
    for (key, name) in [("glyphs", "glyphs"), ("excludeGlyphs", "exclude-glyphs")] {
        if !parsed.list(name).is_empty() {
            params.insert(key.into(), json!(parsed.list(name)));
        }
    }
    params.insert("keepFontHints".into(), json!(parsed.switch("keep-font-hints")));
    params.insert("hintingTables".into(), json!(parsed.list("hinting-tables")));
    params.insert("keepFlex".into(), json!(parsed.switch("keep-flex")));
    if let Some(gasp) = parsed.value("gasp") {
        params.insert("gasp".into(), json!(gasp));
    }
    for (key, name) in [("output", "output"), ("outputDir", "output-dir")] {
        if let Some(v) = parsed.value(name) {
            params.insert(key.into(), json!(v));
        }
    }
    params.insert("inPlace".into(), json!(parsed.switch("in-place")));
    params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    if let Some(face) = parsed.value("face") {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
    let jobs = parsed.value("jobs").map_or(Ok(1), str::parse::<usize>).map_err(|_| usage("--jobs needs a number"))?;
    let as_json = parsed.switch("json");

    let results = batch(&parsed.positional, jobs, |input| {
        let mut p = params.clone();
        p.insert("input".into(), json!(input));
        let result = execute(&mut Session::new(), "hints.remove", &Value::Object(p)).map_err(|f| match f {
            Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
            other => other,
        })?;
        Ok((input.to_string(), result))
    })?;
    for r in &results {
        match r {
            Ok((input, result)) if as_json => out_json_line(&json!({"input": input, "result": result}))?,
            Ok((input, result)) => {
                let removed: Vec<&str> =
                    result.get("tablesRemoved").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                out(&format!(
                    "{input} -> {}: {} glyphs changed; tables removed: {}",
                    result.get("output").and_then(Value::as_str).unwrap_or("?"),
                    result.get("glyphsChanged").and_then(Value::as_u64).unwrap_or(0),
                    if removed.is_empty() { "none".to_string() } else { removed.join(" ") }
                ))?;
            }
            Err(e) => eprintln!("fontcraft-cli: {}", e.message()),
        }
    }
    batch_outcome(&results)
}

//! `fontcraft-cli optimize` (M12.26): one `font.optimize` run per input.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, batch, batch_outcome, execute, out, out_json_line, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "optimize", summary: "Make a font smaller without changing what it does", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli optimize FONT... [--for size|speed] [--subroutinize | --desubroutinize] [--cff-widths] [--charstrings specialize|generalize] [--gpos-compaction N] [--varstore] [--gvar-iup] [options] [-o OUT]
      Re-encode compiled fonts more compactly without changing how they draw or shape. Without
      an optimisation named, the CFF default and nominal widths are chosen again, CFF and CFF2
      charstrings are subroutinised, gvar deltas are optimised, the GDEF variation store is
      merged and COLR layers are shared; naming optimisations runs only those. A step that
      would not make a table smaller is not applied. Each output is INPUT.optimized.EXT unless
      -o, --output-dir or --in-place says otherwise; --report only measures.
      --gpos-compaction 1 to 9 splits class-kerning subtables (default 0: left alone).
      --drop-implied-oncurves runs only when named: it renumbers points, so glyphs whose
      points instructions, composites, anchors or carets number keep them.
      --charstrings rewrites CFF and CFF2 charstrings with the compact operators (specialize)
      or with rmoveto, rlineto and rrcurveto only (generalize), even when the table grows.
      --for speed expands CFF and CFF2 subroutines and writes every gvar delta, with no size
      step; it takes no other optimisation.
";

/// The options, as data (M12.26).
const OPTIONS: &[OptSpec] = &[
    OptSpec::value("for", "size|speed", "size", "smallest file, or structures that are cheaper to read"),
    OptSpec::switch("subroutinize", false, "move repeated CFF and CFF2 charstring fragments into subroutines"),
    OptSpec::value("max-subrs", "N", "", "at most N subroutines in one INDEX (implies --subroutinize)"),
    OptSpec::switch("desubroutinize", false, "expand every subroutine"),
    OptSpec::value("charstrings", "specialize|generalize", "", "rewrite charstrings with the compact operators, or the general ones only"),
    OptSpec::switch("cff-widths", false, "choose defaultWidthX and nominalWidthX again (--cff-widths=exhaustive is the same: the search is exact)"),
    OptSpec::value("gpos-compaction", "N", "", "0 leaves class kerning as it is; 1 to 9 split class-kerning subtables"),
    OptSpec::switch("varstore", false, "merge and re-pack the item variation store (--varstore=QUANTUM rounds deltas first)"),
    OptSpec::switch("gvar-iup", false, "drop gvar deltas that interpolation reproduces (--no-gvar-iup writes every delta)"),
    OptSpec::switch("drop-implied-oncurves", false, "remove on-curve points midway between their off-curve neighbours"),
    OptSpec::switch("colr-layer-reuse", false, "share identical COLR layer sequences"),
    OptSpec::switch("canonical-order", false, "write the tables in the recommended order"),
    OptSpec::switch("report", false, "only measure: print the sizes without writing"),
    OptSpec::value("output", "OUT", "", "the output file (one input only)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder outputs go into"),
    OptSpec::switch("in-place", false, "replace each input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection"),
    OptSpec::switch("json", false, "print one JSON result per input"),
    OptSpec::value("jobs", "N", "1", "inputs processed at once").short('j'),
];

fn run(args: &[String]) -> Outcome {
    // `--cff-widths=exhaustive`, `--varstore=QUANTUM` and `--no-gvar-iup` carry more than a
    // switch says; they are read here and handed to the parser as plain switches.
    let mut params = Map::new();
    let args = args::spell_out(OPTIONS, args)?;
    let mut rest = Vec::with_capacity(args.len());
    for a in &args {
        if let Some(v) = a.strip_prefix("--cff-widths=") {
            if v != "exhaustive" {
                return Err(usage(format!("--cff-widths takes no value but `exhaustive`, got `{v}`")));
            }
            params.insert("cffWidths".into(), json!("exhaustive"));
        } else if let Some(v) = a.strip_prefix("--varstore=") {
            let quantum = v.parse::<u64>().map_err(|_| usage(format!("--varstore=QUANTUM needs a whole number, got `{v}`")))?;
            params.insert("varstore".into(), json!({"quantum": quantum}));
        } else if a == "--no-gvar-iup" {
            params.insert("gvarIup".into(), json!(false));
        } else {
            rest.push(a.clone());
        }
    }
    let parsed = args::parse(OPTIONS, &rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    if parsed.value("output").is_some() && parsed.positional.len() > 1 {
        return Err(usage("-o names one output: give one input, or use --output-dir"));
    }
    if let Some(preset) = parsed.value("for") {
        params.insert("preset".into(), json!(preset));
    }
    for (key, name) in [
        ("subroutinize", "subroutinize"),
        ("desubroutinize", "desubroutinize"),
        ("cffWidths", "cff-widths"),
        ("varstore", "varstore"),
        ("gvarIup", "gvar-iup"),
        ("dropImpliedOncurves", "drop-implied-oncurves"),
        ("colrLayerReuse", "colr-layer-reuse"),
        ("canonicalOrder", "canonical-order"),
        ("report", "report"),
        ("inPlace", "in-place"),
        ("noClobber", "no-clobber"),
    ] {
        if parsed.switch(name) && !params.contains_key(key) {
            params.insert(key.into(), json!(true));
        }
    }
    for (key, name) in [("maxSubrs", "max-subrs"), ("gposCompaction", "gpos-compaction")] {
        if let Some(v) = parsed.value(name) {
            let n = v.parse::<u64>().map_err(|_| usage(format!("--{name} needs a whole number, got `{v}`")))?;
            params.insert(key.into(), json!(n));
        }
    }
    for (key, name) in [("charstrings", "charstrings"), ("output", "output"), ("outputDir", "output-dir")] {
        if let Some(v) = parsed.value(name) {
            params.insert(key.into(), json!(v));
        }
    }
    if let Some(face) = parsed.value("face") {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
    let jobs = parsed.value("jobs").map_or(Ok(1), str::parse::<usize>).map_err(|_| usage("--jobs needs a number"))?;
    let as_json = parsed.switch("json");

    let results = batch(&parsed.positional, jobs, |input| {
        let mut p = params.clone();
        p.insert("font".into(), json!(input));
        let result = execute(&mut Session::new(), "font.optimize", &Value::Object(p)).map_err(|f| match f {
            Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
            other => other,
        })?;
        Ok((input.to_string(), result))
    })?;
    for r in &results {
        match r {
            Ok((input, result)) if as_json => out_json_line(&json!({"input": input, "result": result}))?,
            Ok((input, result)) => out(&summary(input, result))?,
            Err(e) => eprintln!("fontcraft-cli: {}", e.message()),
        }
    }
    batch_outcome(&results)
}

/// One input's result as text: the sizes, each step, then the notes.
fn summary(input: &str, result: &Value) -> String {
    let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0);
    let target = result.get("output").and_then(Value::as_str).map_or_else(|| "(report)".to_string(), |o| format!("-> {o}"));
    let mut text = format!("{input} {target}: {} -> {} bytes", n(result, "bytesBefore"), n(result, "bytesAfter"));
    for s in result.get("steps").and_then(Value::as_array).into_iter().flatten() {
        let step = s.get("step").and_then(Value::as_str).unwrap_or("?");
        let table = s.get("table").and_then(Value::as_str).unwrap_or("?").trim_end();
        let kept = if s.get("applied") == Some(&json!(true)) { "" } else { " (not applied)" };
        text.push_str(&format!("\n  {step} {table}: {} -> {} bytes{kept}", n(s, "before"), n(s, "after")));
    }
    for note in result.get("notes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        text.push_str(&format!("\n  note: {note}"));
    }
    text
}

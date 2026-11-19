//! `fontcraft-cli outlines` (M12.28, M12.29). Each action is a shell over one `outlines.*` engine
//! command; results go to stdout as one JSON line per input.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{Failure, Outcome, batch, batch_outcome, execute, out, out_json_line, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "outlines",
    summary: "Repair or convert the outlines of a whole font",
    usage: USAGE,
    options: OPTIONS,
    actions: &["remove-overlaps", "direction", "extremes", "round", "to-quadratic", "to-cubic", "componentize", "decompose"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli outlines remove-overlaps|direction|extremes|round|to-quadratic|to-cubic|componentize|decompose INPUT... [args]
  fontcraft-cli outlines remove-overlaps INPUT [GLYPH...] [--keep-hinting] [--ignore-errors] [--keep-unused-subroutines] [-o OUT]
  fontcraft-cli outlines direction INPUT... [--flavour postscript|truetype|reverse] [-o OUT | --output-dir DIR] [-j N]
  fontcraft-cli outlines extremes INPUT... [--min-segment N] [-o OUT | --output-dir DIR] [-j N]
  fontcraft-cli outlines round INPUT... [--grid N] [--tolerance F] [-o OUT | --output-dir DIR] [-j N]
  fontcraft-cli outlines to-quadratic INPUT... [--max-error EM] [--mixed] [--interpolatable] [--keep-direction] [-o OUT | --output-dir DIR]
  fontcraft-cli outlines to-cubic INPUT... [--max-error EM] [--all-cubic] [-o OUT | --output-dir DIR]
  fontcraft-cli outlines componentize FONT.ttf [--reference UFO | --detect] [-o OUT]
  fontcraft-cli outlines decompose INPUT... [--nested-only] [--transformed-only] [-o OUT | --output-dir DIR]
";

/// The options, as data (M12.28, M12.29).
const OPTIONS: &[OptSpec] = &[
    // M12.28
    OptSpec::value("output", "FILE", "", "the output file (one input)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder outputs go into, under the inputs' names"),
    OptSpec::switch("in-place", false, "replace each input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N", "", "the font of a collection to read"),
    OptSpec::list("glyphs", "NAMES", "", "only these glyphs"),
    OptSpec::list("exclude-glyphs", "NAMES", "", "leave these glyphs alone"),
    OptSpec::switch("ignore-errors", false, "leave a glyph that cannot be done as it was and go on"),
    OptSpec::switch(
        "keep-hinting",
        false,
        "remove-overlaps: keep the TrueType instructions of glyphs that do not change (CFF glyphs that do not change always keep their hints)",
    ),
    OptSpec::switch("keep-unused-subroutines", false, "remove-overlaps: keep CFF subroutines that became unused"),
    OptSpec::value("min-area", "N", "25", "remove-overlaps: drop merged contours whose bounding box is smaller, in square units"),
    OptSpec::switch("keep-contour-order", true, "remove-overlaps: merged contours keep the order and start points of the contours they come from"),
    OptSpec::value("flavour", "NAME", "", "direction: postscript, truetype or reverse; default the input's convention"),
    OptSpec::value("min-segment", "N", "3", "extremes: 3 skips extremes closer than 3 units to a point; 0 adds every one"),
    OptSpec::value("grid", "N", "1", "round: the grid, in units"),
    OptSpec::value("tolerance", "F", "", "round: only values within F of the grid"),
    OptSpec::switch("nested-only", false, "decompose: only flatten components of components"),
    OptSpec::switch("transformed-only", false, "decompose: only glyphs with a scaled, rotated or slanted component"),
    // M12.29
    OptSpec::value("max-error", "EM", "0.001", "to-quadratic, to-cubic: the largest deviation, as a fraction of the em"),
    OptSpec::switch("mixed", false, "to-quadratic: keep a cubic curve that one quadratic curve cannot replace"),
    OptSpec::switch("interpolatable", false, "to-quadratic: convert the masters of the input as one compatible set"),
    OptSpec::switch("keep-direction", false, "to-quadratic: do not reverse the contours of a source"),
    OptSpec::switch("all-cubic", false, "to-cubic: make every curve cubic"),
    OptSpec::value("reference", "SOURCE", "", "componentize: the source whose composites are copied; default the .ufo beside the font"),
    OptSpec::value("goadb", "FILE", "", "componentize: maps the source's glyph names to the font's"),
    OptSpec::switch("detect", false, "componentize: find glyphs made of other glyphs without a source"),
    OptSpec::switch("allow-transforms", false, "componentize: also use scaled, rotated or flipped components of the source"),
];

fn run(args: &[String]) -> Outcome {
    // M12.28, M12.29
    if args.iter().any(|a| a == "-h" || a == "--help") || args.is_empty() {
        return if args.is_empty() { Err(usage(super::help(&SUBCOMMAND))) } else { out(&super::help(&SUBCOMMAND)) };
    }
    let Some((action, rest)) = args.split_first() else { return Err(usage(super::help(&SUBCOMMAND))) };
    let command = match action.as_str() {
        "remove-overlaps" => "outlines.removeOverlaps",
        "direction" => "outlines.correctDirection",
        "extremes" => "outlines.addExtremes",
        "round" => "outlines.roundCoordinates",
        "decompose" => "outlines.decompose",
        "to-quadratic" | "to-cubic" => "outlines.convertCurves",
        "componentize" => "outlines.componentize",
        _ => return super::not_built(&SUBCOMMAND, args),
    };
    let parsed = args::parse(OPTIONS, rest)?;
    let Some(first) = parsed.positional.first() else { return Err(usage(format!("outlines {action} needs an input"))) };
    let mut params = shared_params(&parsed)?;
    let mut session = Session::new();
    match action.as_str() {
        "remove-overlaps" => {
            // INPUT [GLYPH...], as in the reference tool.
            let mut glyphs: Vec<String> = parsed.positional.iter().skip(1).cloned().collect();
            glyphs.extend(parsed.list("glyphs").iter().cloned());
            if !glyphs.is_empty() {
                params.insert("glyphs".into(), json!(glyphs));
            }
            params.insert("keepHinting".into(), json!(parsed.switch("keep-hinting")));
            params.insert("keepUnusedSubroutines".into(), json!(parsed.switch("keep-unused-subroutines")));
            params.insert("keepContourOrder".into(), json!(parsed.switch("keep-contour-order")));
            params.insert("minArea".into(), json!(number(&parsed, "min-area")?));
            params.insert("input".into(), json!(first));
            let result = execute(&mut session, command, &Value::Object(params))?;
            out_json_line(&result)
        }
        "to-quadratic" | "to-cubic" => {
            let quadratic = action == "to-quadratic";
            params.insert("to".into(), json!(if quadratic { "quadratic" } else { "cubic" }));
            params.insert("maxError".into(), json!(number(&parsed, "max-error")?));
            if quadratic {
                params.insert("mixed".into(), json!(parsed.switch("mixed")));
                params.insert("interpolatable".into(), json!(parsed.switch("interpolatable")));
                params.insert("keepDirection".into(), json!(parsed.switch("keep-direction")));
            } else {
                params.insert("allCubic".into(), json!(parsed.switch("all-cubic")));
            }
            params.insert("inputs".into(), json!(parsed.positional));
            let result = execute(&mut session, command, &Value::Object(params))?;
            for r in result.get("results").and_then(Value::as_array).into_iter().flatten() {
                out_json_line(r)?;
            }
            Ok(())
        }
        "componentize" => {
            if parsed.positional.len() > 1 {
                return Err(usage("outlines componentize takes one font"));
            }
            params.remove("ignoreErrors");
            for (option, key) in [("reference", "reference"), ("goadb", "goadb")] {
                if let Some(v) = parsed.value(option) {
                    params.insert(key.into(), json!(v));
                }
            }
            params.insert("detect".into(), json!(parsed.switch("detect")));
            params.insert("allowTransforms".into(), json!(parsed.switch("allow-transforms")));
            params.insert("font".into(), json!(first));
            let result = execute(&mut session, command, &Value::Object(params))?;
            out_json_line(&result)
        }
        "decompose" => {
            params.insert("nestedOnly".into(), json!(parsed.switch("nested-only")));
            params.insert("transformedOnly".into(), json!(parsed.switch("transformed-only")));
            params.insert("inputs".into(), json!(parsed.positional));
            let result = execute(&mut session, command, &Value::Object(params))?;
            for r in result.get("results").and_then(Value::as_array).into_iter().flatten() {
                out_json_line(r)?;
            }
            Ok(())
        }
        _ => {
            match action.as_str() {
                "direction" => {
                    if let Some(f) = parsed.value("flavour") {
                        params.insert("flavour".into(), json!(f));
                    }
                }
                "extremes" => {
                    params.insert("minSegment".into(), json!(number(&parsed, "min-segment")?));
                }
                _ => {
                    params.insert("grid".into(), json!(number(&parsed, "grid")?));
                    if parsed.value("tolerance").is_some() {
                        params.insert("tolerance".into(), json!(number(&parsed, "tolerance")?));
                    }
                }
            }
            if parsed.positional.len() > 1 && params.contains_key("output") {
                return Err(usage("-o names one file: give --output-dir or --in-place for several inputs"));
            }
            if let [one] = parsed.positional.as_slice() {
                params.insert("input".into(), json!(one));
                return out_json_line(&execute(&mut session, command, &Value::Object(params))?);
            }
            // Several inputs run at once (`-j`); results come out in the order given.
            let results = batch(&parsed.positional, parsed.jobs()?, |input| {
                let mut p = params.clone();
                p.insert("input".into(), json!(input));
                execute(&mut Session::new(), command, &Value::Object(p))
            })?;
            for (input, r) in parsed.positional.iter().zip(&results) {
                match r {
                    Ok(result) => out_json_line(result)?,
                    Err(e) => crate::io::error(&format!("{input}: {}", e.message())),
                }
            }
            match batch_outcome(&results) {
                Err(Failure::Partial(m)) => Err(Failure::Partial(m)),
                Err(_) => Err(Failure::Failed(format!("all {} inputs failed", results.len()))),
                Ok(()) => Ok(()),
            }
        }
    }
}

/// The parameters every action takes.
fn shared_params(parsed: &Parsed) -> Result<Map<String, Value>, Failure> {
    let mut params = Map::new();
    for (option, key) in [("output", "output"), ("output-dir", "outputDir")] {
        if let Some(v) = parsed.value(option) {
            params.insert(key.into(), json!(v));
        }
    }
    for (option, key) in [("in-place", "inPlace"), ("no-clobber", "noClobber"), ("ignore-errors", "ignoreErrors")] {
        params.insert(key.into(), json!(parsed.switch(option)));
    }
    if let Some(face) = parsed.value("face") {
        let n: u32 = face.parse().map_err(|_| usage(format!("--face takes a font number, not `{face}`")))?;
        params.insert("face".into(), json!(n));
    }
    if !parsed.list("glyphs").is_empty() {
        params.insert("glyphs".into(), json!(parsed.list("glyphs")));
    }
    if !parsed.list("exclude-glyphs").is_empty() {
        params.insert("excludeGlyphs".into(), json!(parsed.list("exclude-glyphs")));
    }
    Ok(params)
}

fn number(parsed: &Parsed, option: &str) -> Result<f64, Failure> {
    let text = parsed.value(option).unwrap_or_default();
    text.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| usage(format!("--{option} takes a number, not `{text}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_actions_parse_and_refuse() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let e = run(&args(&["round"])).unwrap_err();
        assert_eq!(e.exit_code(), 2, "an input is required");
        let e = run(&args(&["round", "x.ttf", "--grid", "fine"])).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        let e = run(&args(&["to-cubic", "x.ttf", "--max-error", "much"])).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        let e = run(&args(&["componentize", "a.ttf", "b.ttf"])).unwrap_err();
        assert_eq!(e.exit_code(), 2, "componentize takes one font");
        let e = run(&args(&["to-cubic", "/nonexistent/x.ttf", "-o", "/nonexistent/y.ttf"])).unwrap_err();
        assert_eq!(e.exit_code(), 4);
        let e = run(&args(&["round", "/nonexistent/x.ttf", "-o", "/nonexistent/y.ttf"])).unwrap_err();
        assert_eq!(e.exit_code(), 4);
        assert!(run(&args(&["--help"])).is_ok());
    }
}

//! `fontcraft-cli metrics` (M12.16): a shell over `font.reportMetrics`, one run per FONT.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "metrics", summary: "Report glyph metrics and kerning", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli metrics FONT... [--format text|json|csv|afm] [--precision integer|real] [--aggregate-bbox] [--kerning] [--statistics] [--glyphs SELECTOR]
";

/// The options, as data (M12.16).
const OPTIONS: &[OptSpec] = &[
    OptSpec::value("format", "FORMAT", "text", "text (the default), json, csv or afm (a font metrics file)"),
    OptSpec::value("precision", "integer|real", "integer", "round values to integers, or print exact values"),
    OptSpec::switch("aggregate-bbox", false, "add the font bounding box and the glyphs that set its edges"),
    OptSpec::switch("sidebearings", false, "add the left and right side bearings"),
    OptSpec::value("units", "font|1000", "font", "values in font units, or rescaled to 1000 per em"),
    OptSpec::switch("kerning", false, "add the kerning pairs, flattened from GPOS (or the kern table)"),
    OptSpec::switch("statistics", false, "add area, centre of mass, variance, covariance, correlation and slant, and font-wide estimates"),
    OptSpec::switch("control-box", false, "with --statistics, measure the control polygon instead of the curves"),
    OptSpec::switch("font-only", false, "with --statistics, print only the font-wide values"),
    OptSpec::list("glyphs", "SELECTOR", "", "glyphs to measure: names, glyph ids, /CID, U+XXXX, ranges, *"),
    OptSpec::list("exclude-glyphs", "SELECTOR", "", "glyphs to leave out"),
    OptSpec::list("fd", "LIST", "", "font dictionaries of a CID-keyed font to keep"),
    OptSpec::list("location", "AXIS=VALUE", "", "measure a variable font at this location (axis tags)"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection, by number or PostScript name"),
    OptSpec::value("output", "PATH", "", "write the report to PATH (one FONT only)").short('o'),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::switch("quiet", false, "print no warnings").short('q'),
];

fn run(argv: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, argv)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    if parsed.value("output").is_some() && parsed.positional.len() > 1 {
        return Err(usage("--output takes one FONT; give one FONT per run"));
    }
    let params = params(&parsed)?;
    let format = parsed.value("format").unwrap_or("text");
    let several = parsed.positional.len() > 1;
    let mut results = Vec::with_capacity(parsed.positional.len());
    for font in &parsed.positional {
        let mut p = params.clone();
        p.insert("font".into(), json!(font));
        let mut session = Session::new();
        let result =
            io::execute(&mut session, "font.reportMetrics", &Value::Object(p)).and_then(|r| print(font, &r, format, several, parsed.switch("quiet")));
        if let Err(e) = &result {
            io::note(&format!("{font}: {}", e.message()));
        }
        results.push(result);
    }
    io::batch_outcome(&results)
}

/// The command's parameters from the parsed options.
fn params(parsed: &args::Parsed) -> Result<Map<String, Value>, Failure> {
    let mut p = Map::new();
    for (opt, key) in [("format", "format"), ("precision", "precision"), ("output", "output")] {
        if let Some(v) = parsed.value(opt) {
            p.insert(key.into(), json!(v));
        }
    }
    match parsed.value("units") {
        Some("1000") => p.insert("units".into(), json!(1000)),
        Some("font") | None => None,
        Some(other) => return Err(usage(format!("--units takes font or 1000, not {other}"))),
    };
    if let Some(face) = parsed.value("face") {
        p.insert("face".into(), face.parse::<u64>().map_or_else(|_| json!(face), Value::from));
    }
    for (opt, key) in [
        ("aggregate-bbox", "aggregateBbox"),
        ("sidebearings", "sidebearings"),
        ("kerning", "kerning"),
        ("statistics", "statistics"),
        ("control-box", "controlBox"),
        ("font-only", "fontOnly"),
        ("no-clobber", "noClobber"),
    ] {
        if parsed.switch(opt) {
            p.insert(key.into(), json!(true));
        }
    }
    for (opt, key) in [("glyphs", "glyphs"), ("exclude-glyphs", "excludeGlyphs")] {
        let items = parsed.list(opt);
        if !items.is_empty() {
            p.insert(key.into(), json!(items));
        }
    }
    let fd = parsed.list("fd");
    if !fd.is_empty() {
        let numbers = fd
            .iter()
            .map(|v| v.parse::<u16>().map_err(|_| usage(format!("--fd takes font dictionary numbers, not {v}"))))
            .collect::<Result<Vec<_>, _>>()?;
        p.insert("fd".into(), json!(numbers));
    }
    let location = parsed.list("location");
    if !location.is_empty() {
        let mut axes = Map::new();
        for item in location.iter().flat_map(|l| l.split_whitespace()) {
            let (axis, value) = item.split_once('=').ok_or_else(|| usage(format!("--location takes AXIS=VALUE, not {item}")))?;
            let value: f64 =
                value.parse().ok().filter(|v: &f64| v.is_finite()).ok_or_else(|| usage(format!("--location: {value} is not a number")))?;
            axes.insert(axis.to_string(), json!(value));
        }
        p.insert("location".into(), Value::Object(axes));
    }
    Ok(p)
}

fn print(font: &str, result: &Value, format: &str, several: bool, quiet: bool) -> Outcome {
    if !quiet {
        for w in result.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            io::note(&format!("{font}: {w}"));
        }
    }
    if result.get("written").is_some() {
        return Ok(());
    }
    if format == "json" {
        return io::out_json(result);
    }
    if several {
        out(&format!("## {font}"))?;
    }
    let text = result.get("text").and_then(Value::as_str).unwrap_or_default();
    out(text.strip_suffix('\n').unwrap_or(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_options_become_command_parameters() {
        let argv: Vec<String> = ["A.otf", "--statistics", "--units", "1000", "--glyphs", "A,B", "--location", "wght=700", "--face", "2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = args::parse(OPTIONS, &argv).unwrap();
        let p = params(&parsed).unwrap();
        assert_eq!(p["statistics"], json!(true));
        assert_eq!(p["units"], json!(1000));
        assert_eq!(p["glyphs"], json!(["A", "B"]));
        assert_eq!(p["location"], json!({ "wght": 700.0 }));
        assert_eq!(p["face"], json!(2));
        assert_eq!(p["format"], json!("text"));
        assert!(params(&args::parse(OPTIONS, &["A.otf".into(), "--units".into(), "em".into()]).unwrap()).is_err());
    }
}

//! `fontcraft-cli transform` (M12.24): one `font.transform` run.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{Failure, Outcome, execute, out, out_json_line, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "transform", summary: "Transform every glyph or change units per em", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli transform FONT [--rotate DEG] [--translate X,Y] [--scale SX[,SY]] [--slant DEG] [--matrix A,B,C,D,E,F] [--upem N] [--glyph-table FILE] [--widths auto|keep|transform|em] [--to ttf|otf|otf-cff2] [-o OUT]
      Apply one transformation to the glyphs of a compiled font (scale, slant, rotate
      clockwise and translate, in that order, or a matrix), or set a new units per em and
      scale the whole font to it. A source takes --upem only. The output is
      FONT.transformed.EXT unless -o, --output-dir or --in-place says otherwise.
";

/// The options, as data (M12.24).
const OPTIONS: &[OptSpec] = &[
    OptSpec::value("rotate", "DEG", "", "rotate clockwise about the origin by this many degrees"),
    OptSpec::value("translate", "X,Y", "", "shift after the other operations"),
    OptSpec::value("scale", "SX[,SY]", "", "scale factors; SY defaults to SX"),
    OptSpec::value("slant", "DEG", "", "shear horizontally by this angle"),
    OptSpec::value("matrix", "A,B,C,D,E,F", "", "explicit matrix, x' = a·x + c·y + e, y' = b·x + d·y + f; excludes the four options above"),
    OptSpec::switch("scale-em", false, "with a matrix or --scale, also scale the units per em by the horizontal factor"),
    OptSpec::value("upem", "N", "", "set units per em to N and scale the whole font to match"),
    OptSpec::value("glyph-table", "FILE", "", "per-glyph names, advances and offsets (not supported yet)"),
    OptSpec::value("widths", "auto|keep|transform|em", "auto", "advance widths of transformed glyphs"),
    OptSpec::list("glyphs", "NAMES", "", "transform only these glyphs"),
    OptSpec::list("exclude-glyphs", "NAMES", "", "leave these glyphs alone"),
    OptSpec::value("to", "ttf|otf|otf-cff2", "", "convert the outlines of the result to this format"),
    OptSpec::value("output", "OUT", "", "the output file").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder the output goes into"),
    OptSpec::switch("in-place", false, "replace the input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection"),
    OptSpec::switch("json", false, "print the result as JSON"),
];

/// The numbers of a comma-separated option.
fn numbers(text: &str, option: &str) -> Result<Vec<f64>, Failure> {
    text.split(',')
        .map(|t| t.trim().parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| usage(format!("--{option} needs numbers, not `{text}`"))))
        .collect()
}

/// The engine parameters the options describe, without the font.
fn params(parsed: &Parsed) -> Result<Map<String, Value>, Failure> {
    let mut p = Map::new();
    for (key, name) in [("rotate", "rotate"), ("slant", "slant")] {
        if let Some(v) = parsed.value(name) {
            let [n] = numbers(v, name)?[..] else { return Err(usage(format!("--{name} takes one number"))) };
            p.insert(key.into(), json!(n));
        }
    }
    if let Some(v) = parsed.value("translate") {
        let [x, y] = numbers(v, "translate")?[..] else { return Err(usage("--translate takes X,Y")) };
        p.insert("translate".into(), json!({"x": x, "y": y}));
    }
    if let Some(v) = parsed.value("scale") {
        match numbers(v, "scale")?[..] {
            [s] => p.insert("scale".into(), json!(s)),
            [x, y] => p.insert("scale".into(), json!({"x": x, "y": y})),
            _ => return Err(usage("--scale takes SX or SX,SY")),
        };
    }
    if let Some(v) = parsed.value("matrix") {
        let m = numbers(v, "matrix")?;
        if m.len() != 6 {
            return Err(usage("--matrix takes six numbers A,B,C,D,E,F"));
        }
        p.insert("matrix".into(), json!(m));
    }
    if parsed.switch("scale-em") {
        p.insert("scaleEm".into(), json!(true));
    }
    if let Some(v) = parsed.value("upem") {
        let n = v.trim().parse::<u64>().map_err(|_| usage(format!("--upem needs a whole number, not `{v}`")))?;
        p.insert("unitsPerEm".into(), json!(n));
    }
    if let Some(v) = parsed.value("glyph-table") {
        p.insert("glyphTable".into(), json!(v));
    }
    if let Some(v) = parsed.value("to") {
        p.insert("to".into(), json!(v));
    }
    if let Some(v) = parsed.value("widths") {
        p.insert("widths".into(), json!(v));
    }
    for (key, name) in [("glyphs", "glyphs"), ("excludeGlyphs", "exclude-glyphs")] {
        if !parsed.list(name).is_empty() {
            p.insert(key.into(), json!(parsed.list(name)));
        }
    }
    for (key, name) in [("output", "output"), ("outputDir", "output-dir")] {
        if let Some(v) = parsed.value(name) {
            p.insert(key.into(), json!(v));
        }
    }
    p.insert("inPlace".into(), json!(parsed.switch("in-place")));
    p.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    if let Some(face) = parsed.value("face") {
        p.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
    Ok(p)
}

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let [font] = parsed.positional.as_slice() else {
        return Err(usage(super::help(&SUBCOMMAND)));
    };
    let mut p = params(&parsed)?;
    p.insert("font".into(), json!(font));
    let result = execute(&mut Session::new(), "font.transform", &Value::Object(p)).map_err(|f| match f {
        Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
        other => other,
    })?;
    if parsed.switch("json") {
        return out_json_line(&result);
    }
    for w in result.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        eprintln!("fontcraft-cli: warning: {w}");
    }
    out(&format!(
        "{font} -> {}: {} glyphs changed{}",
        result.get("output").and_then(Value::as_str).unwrap_or("?"),
        result.get("glyphsChanged").and_then(Value::as_u64).unwrap_or(0),
        match result.get("unitsPerEm").filter(|v| !v.is_null()) {
            Some(u) => format!("; units per em {} -> {}", u["from"], u["to"]),
            None => String::new(),
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(list: &[&str]) -> Parsed {
        let argv: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        args::parse(OPTIONS, &argv).unwrap()
    }

    #[test]
    fn transform_options_become_command_parameters() {
        let p = params(&parse(&["A.ttf", "--rotate", "-90", "--translate", "0,1000", "--scale", "2", "--upem", "2048", "--glyphs", "A,B"])).unwrap();
        assert_eq!(p["rotate"], json!(-90.0));
        assert_eq!(p["translate"], json!({"x": 0.0, "y": 1000.0}));
        assert_eq!(p["scale"], json!(2.0));
        assert_eq!(p["unitsPerEm"], json!(2048));
        assert_eq!(p["glyphs"], json!(["A", "B"]));
        let p = params(&parse(&["A.ttf", "--matrix", "0,-1,1,0,0,1000", "--widths", "em"])).unwrap();
        assert_eq!(p["matrix"], json!([0.0, -1.0, 1.0, 0.0, 0.0, 1000.0]));
        assert_eq!(p["widths"], json!("em"));
        assert!(params(&parse(&["A.ttf", "--matrix", "1,0,0,1"])).is_err());
        assert!(params(&parse(&["A.ttf", "--translate", "5"])).is_err());
        assert!(params(&parse(&["A.ttf", "--upem", "big"])).is_err());
        assert!(params(&parse(&["A.ttf", "--scale", "1,2,3"])).is_err());
    }
}

//! `fontcraft-cli variations` (M12.33, M12.34). `map` and `build-avar` run
//! `variations.mapCoordinates` and `variations.buildAvar`; the other actions are M12.34's and
//! exit 4 with "not built yet" until it lands.

use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "variations",
    summary: "Variation utilities: coordinates, avar, HVAR, models",
    usage: USAGE,
    options: OPTIONS,
    actions: &["build-avar", "plan-avar", "map", "build-hvar", "model", "plot"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli variations build-avar|plan-avar|map|build-hvar|model|plot INPUT [args]
  fontcraft-cli variations build-avar FONT [DESIGNSPACE] [-o OUT]
  fontcraft-cli variations plan-avar FONT [--weights LIST] [--widths LIST] [--slants LIST] [--sizes LIST] [--samples N] [--glyphs LIST] [--pins AXIS=FROM:TO,...] [--design-limits AXIS=MIN:DEF:MAX] [--sanitize] [--plot FILE] [-o OUT]
  fontcraft-cli variations map FONT [AXIS=VALUE...] [--from user|normalized] [--to normalized|user] [--all-axes]
  fontcraft-cli variations build-hvar FONT [--no-vvar] [--replace] [-o OUT]
  fontcraft-cli variations model (DESIGNSPACE | --locations LOC...) [--json]
  fontcraft-cli variations plot (DESIGNSPACE | FONT | --locations LOC...) [-o OUT.svg]
";

/// The options, as data (M12.33, M12.34).
const OPTIONS: &[OptSpec] = &[
    // M12.33
    OptSpec::value("output", "FILE", "", "build-avar: the output font (default: FONT.avar.EXT beside the input)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "build-avar: the folder the output goes into"),
    OptSpec::switch("in-place", false, "build-avar: replace the input font"),
    OptSpec::switch("no-clobber", false, "build-avar: refuse to replace an existing output"),
    OptSpec::value("from", "SPACE", "", "map: the space of the input values, user (default) or normalized"),
    OptSpec::value("to", "SPACE", "", "map: the space printed, normalized or user (default: the space other than --from)"),
    OptSpec::switch("all-axes", false, "map: also print the axes that end at their default"),
    OptSpec::value("face", "N", "", "map: the font of a collection, from 0"),
    OptSpec::switch("json", false, "print the engine command's result as JSON"),
    // M12.34
];

fn run(args: &[String]) -> Outcome {
    match args.first().map(String::as_str) {
        Some("map") => map(&parse(args)?),
        Some("build-avar") => build_avar(&parse(args)?),
        _ if args.iter().any(|a| a == "-h" || a == "--help") => out(&super::help(&SUBCOMMAND)),
        _ => super::not_built(&SUBCOMMAND, args),
    }
}

/// The arguments after the action word.
fn parse(args: &[String]) -> Result<Parsed, Failure> {
    args::parse(OPTIONS, args.get(1..).unwrap_or_default())
}

fn execute(id: &str, params: &Value) -> Result<Value, Failure> {
    let mut session = fontcraft_engine::Session::new();
    io::execute(&mut session, id, params)
}

fn map(parsed: &Parsed) -> Outcome {
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let (font, coords) = parsed.positional.split_first().ok_or_else(|| usage("variations map needs a FONT"))?;
    let mut location = Map::new();
    for item in coords {
        let (tag, value) = item
            .split_once('=')
            .filter(|(t, v)| !t.is_empty() && !v.is_empty())
            .ok_or_else(|| usage(format!("invalid coordinate `{item}`: give AXIS=VALUE, e.g. wght=500")))?;
        let v: f64 = value
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite())
            .ok_or_else(|| usage(format!("invalid coordinate value in `{item}`: give a number after `=`")))?;
        location.insert(tag.to_string(), json!(v));
    }
    let mut params = json!({ "font": font, "location": location, "allAxes": parsed.switch("all-axes") });
    for (option, key) in [("from", "from"), ("to", "to")] {
        if let (Some(v), Some(obj)) = (parsed.value(option), params.as_object_mut()) {
            obj.insert(key.into(), json!(v));
        }
    }
    if let Some(face) = parsed.value("face") {
        let face: u32 = face.parse().map_err(|_| usage(format!("--face takes a whole number, got `{face}`")))?;
        if let Some(obj) = params.as_object_mut() {
            obj.insert("face".into(), json!(face));
        }
    }
    let result = execute("variations.mapCoordinates", &params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    let shown = result.get("location").and_then(Value::as_object);
    for axis in result.get("axes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if let Some(v) = shown.and_then(|m| m.get(axis)).and_then(Value::as_f64) {
            out(&format!("{axis}={}", format_g(v)))?;
        }
    }
    Ok(())
}

fn build_avar(parsed: &Parsed) -> Outcome {
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let (font, designspace) = match parsed.positional.as_slice() {
        [font] => (font, None),
        [font, ds] => (font, Some(ds)),
        [] => return Err(usage("variations build-avar needs a FONT")),
        _ => return Err(usage("variations build-avar takes a FONT and at most one DESIGNSPACE")),
    };
    let mut params = json!({
        "font": font,
        "inPlace": parsed.switch("in-place"),
        "noClobber": parsed.switch("no-clobber"),
    });
    if let Some(obj) = params.as_object_mut() {
        for (key, value) in
            [("designspace", designspace.map(String::as_str)), ("output", parsed.value("output")), ("outputDir", parsed.value("output-dir"))]
        {
            if let Some(v) = value {
                obj.insert(key.into(), json!(v));
            }
        }
    }
    let result = execute("variations.buildAvar", &params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    if result.get("avar") == Some(&json!(false)) {
        io::note("every axis map is the identity: the font has no avar table");
    }
    out(result.get("output").and_then(Value::as_str).unwrap_or_default())
}

/// `v` as C's `%g` prints it: six significant digits, trailing zeros dropped, exponent form
/// below 1e-4 and from 1e6.
fn format_g(v: f64) -> String {
    const DIGITS: i32 = 6;
    if v == 0.0 || !v.is_finite() {
        return if v.is_nan() {
            "nan".into()
        } else if v.is_infinite() {
            if v > 0.0 { "inf".into() } else { "-inf".into() }
        } else {
            "0".into()
        };
    }
    let sci = format!("{:.*e}", (DIGITS - 1) as usize, v);
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: &str| if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() };
    if !(-4..DIGITS).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exp.abs())
    } else {
        trim(&format!("{:.*}", (DIGITS - 1 - exp).max(0) as usize, v))
    }
}

#[cfg(test)]
mod tests {
    use super::format_g;

    #[test]
    fn numbers_print_as_fonttools_prints_them() {
        for (v, text) in [
            (-0.70001220703125, "-0.700012"),
            (189.996337890625, "189.996"),
            (733.3163784333673, "733.316"),
            (900.0, "900"),
            (0.4, "0.4"),
            (-0.0, "0"),
            (0.00001234, "1.234e-05"),
            (1234567.0, "1.23457e+06"),
            (999999.5, "1e+06"),
            (0.0001, "0.0001"),
        ] {
            assert_eq!(format_g(v), text, "{v}");
        }
    }
}

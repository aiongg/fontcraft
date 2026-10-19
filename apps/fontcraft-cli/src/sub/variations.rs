//! `fontcraft-cli variations` (M12.33, M12.34). Each action runs one engine command: `map`
//! `variations.mapCoordinates`, `build-avar` `variations.buildAvar`, `plan-avar`
//! `variations.planAvar`, `build-hvar` `variations.buildHvar`, `model`
//! `variations.describeModel` and `plot` `variations.plot`.

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
    OptSpec::value(
        "output",
        "FILE",
        "",
        "the output file (default: FONT.avar.EXT for build-avar and plan-avar, FONT.hvar.EXT for build-hvar; plot prints the SVG)",
    )
    .short('o'),
    OptSpec::value("output-dir", "DIR", "", "build-avar, plan-avar, build-hvar: the folder the output goes into"),
    OptSpec::switch("in-place", false, "build-avar, plan-avar, build-hvar: replace the input font"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("from", "SPACE", "", "map: the space of the input values, user (default) or normalized"),
    OptSpec::value("to", "SPACE", "", "map: the space printed, normalized or user (default: the space other than --from)"),
    OptSpec::switch("all-axes", false, "map: also print the axes that end at their default"),
    OptSpec::value("face", "N", "", "map: the font of a collection, from 0"),
    OptSpec::switch("json", false, "print the engine command's result as JSON"),
    // M12.34
    OptSpec::switch("vvar", true, "build-hvar: also build VVAR when the font has vertical metrics (--no-vvar: never)"),
    OptSpec::switch("replace", false, "build-hvar: rebuild a table the font already has"),
    OptSpec::switch(
        "locations",
        false,
        "model, plot: the arguments are master locations, normalised: comma-separated values (axes A, B, C…) or AXIS=VALUE items (put -- before a location that starts with -)",
    ),
    OptSpec::list("axes", "TAGS", "", "plot: the one or two axes to plot when there are more"),
    OptSpec::value("weights", "LIST", "", "plan-avar: the weights to place (default: 50 to 950 in steps of 50)"),
    OptSpec::value("widths", "LIST", "", "plan-avar: the widths to place (default: 25 to 200 in steps of 12.5)"),
    OptSpec::value("slants", "LIST", "", "plan-avar: the slant angles to place (default: the angles of tangents -1 to 1 in steps of 0.05)"),
    OptSpec::value("sizes", "LIST", "", "plan-avar: the optical sizes to place (default: 5 to 288)"),
    OptSpec::value("samples", "N", "", "plan-avar: measurements between two pins (default 8)"),
    OptSpec::list("glyphs", "LIST", "", "plan-avar: the glyphs to measure, each NAME or NAME:FREQUENCY (default: every glyph once)"),
    OptSpec::list("pins", "AXIS=FROM:TO,...", "", "plan-avar: user values that map exactly to design values"),
    OptSpec::list("design-limits", "AXIS=MIN:DEF:MAX,...", "", "plan-avar: the design extent of an axis"),
    OptSpec::switch("sanitize", false, "plan-avar: warn when the measurements at the axis limits disagree with the axis values"),
    OptSpec::value("plot", "FILE", "", "plan-avar: write an SVG chart of the planned maps"),
];

fn run(args: &[String]) -> Outcome {
    match args.first().map(String::as_str) {
        Some("map") => map(&parse(args)?),
        Some("build-avar") => build_avar(&parse(args)?),
        Some("build-hvar") => build_hvar(&parse(args)?),
        Some("model") => model(&parse(args)?),
        Some("plot") => plot(&parse(args)?),
        Some("plan-avar") => plan_avar(&parse(args)?),
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

/// Exit status 3 for an input that cannot be read, before the engine runs.
fn readable(path: &str) -> Outcome {
    std::fs::metadata(path).map(|_| ()).map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))
}

/// `params` with the output options of the command line.
fn with_output(parsed: &Parsed, mut params: Value) -> Value {
    if let Some(obj) = params.as_object_mut() {
        obj.insert("inPlace".into(), json!(parsed.switch("in-place")));
        obj.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
        for (option, key) in [("output", "output"), ("output-dir", "outputDir")] {
            if let Some(v) = parsed.value(option) {
                obj.insert(key.into(), json!(v));
            }
        }
    }
    params
}

fn build_hvar(parsed: &Parsed) -> Outcome {
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let font = match parsed.positional.as_slice() {
        [font] => font,
        [] => return Err(usage("variations build-hvar needs a FONT")),
        _ => return Err(usage("variations build-hvar takes one FONT")),
    };
    readable(font)?;
    let params = with_output(parsed, json!({ "font": font, "vvar": parsed.switch("vvar"), "replace": parsed.switch("replace") }));
    let result = execute("variations.buildHvar", &params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    for table in ["hvar", "vvar"] {
        if result.get(table).and_then(Value::as_str) == Some("kept") {
            io::note(&format!("the font already has {}: kept (pass --replace to rebuild it)", table.to_uppercase()));
        }
    }
    out(result.get("output").and_then(Value::as_str).unwrap_or_default())
}

/// One location argument: `0,0.5` (axes A, B, …) or `wght=0.5,wdth=-1`.
fn location_arg(item: &str) -> Result<Value, Failure> {
    let number = |v: &str| {
        v.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| usage(format!("invalid location `{item}`: give numbers, e.g. 0,0.5 or wght=0.5")))
    };
    if item.contains('=') {
        let mut map = Map::new();
        for part in item.split(',') {
            let (axis, v) = part
                .split_once('=')
                .filter(|(a, _)| !a.is_empty())
                .ok_or_else(|| usage(format!("invalid location `{item}`: give AXIS=VALUE items")))?;
            map.insert(axis.trim().to_string(), json!(number(v)?));
        }
        Ok(Value::Object(map))
    } else {
        Ok(json!(item.split(',').map(number).collect::<Result<Vec<_>, _>>()?))
    }
}

/// The `designspace`, `font` or `locations` parameter of `model` and `plot`.
fn model_source(parsed: &Parsed, action: &str, font_allowed: bool) -> Result<Map<String, Value>, Failure> {
    let mut params = Map::new();
    if parsed.switch("locations") {
        if parsed.positional.is_empty() {
            return Err(usage(format!("variations {action} --locations needs at least one location")));
        }
        let locations = parsed.positional.iter().map(|l| location_arg(l)).collect::<Result<Vec<_>, _>>()?;
        params.insert("locations".into(), Value::Array(locations));
        return Ok(params);
    }
    match parsed.positional.as_slice() {
        [input] => {
            readable(input)?;
            let key = if font_allowed && !input.to_ascii_lowercase().ends_with(".designspace") { "font" } else { "designspace" };
            params.insert(key.into(), json!(input));
            Ok(params)
        }
        [] => Err(usage(format!("variations {action} needs a DESIGNSPACE{} or --locations", if font_allowed { ", a FONT" } else { "" }))),
        _ => Err(usage(format!("variations {action} takes one input; give --locations for several locations"))),
    }
}

/// `{axis: value}` as `{axis=value, …}`, the axes in the order given.
fn location_text(loc: &Value) -> String {
    let items: Vec<String> = loc.as_object().into_iter().flatten().map(|(k, v)| format!("{k}={}", format_g(v.as_f64().unwrap_or(0.0)))).collect();
    format!("{{{}}}", items.join(", "))
}

fn model(parsed: &Parsed) -> Outcome {
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let params = Value::Object(model_source(parsed, "model", false)?);
    let result = execute("variations.describeModel", &params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    let names: Vec<&str> = result.get("names").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    out("Normalized locations:")?;
    for (i, loc) in result.get("locations").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let name = names.get(i).filter(|n| !n.is_empty()).map(|n| format!(" ({n})")).unwrap_or_default();
        out(&format!("  {i}{name}: {}", location_text(loc)))?;
    }
    out("Sorted locations and supports:")?;
    let sorted = result.get("sorted").and_then(Value::as_array).cloned().unwrap_or_default();
    let supports = result.get("supports").and_then(Value::as_array).cloned().unwrap_or_default();
    let order = result.get("order").and_then(Value::as_array).cloned().unwrap_or_default();
    for ((master, loc), support) in order.iter().zip(&sorted).zip(&supports) {
        let tents: Vec<String> = support
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| {
                let t: Vec<String> = v.as_array().into_iter().flatten().map(|x| format_g(x.as_f64().unwrap_or(0.0))).collect();
                format!("{k}=({})", t.join(", "))
            })
            .collect();
        out(&format!("  {}: {}  support {{{}}}", master, location_text(loc), tents.join(", ")))?;
    }
    Ok(())
}

fn plot(parsed: &Parsed) -> Outcome {
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let mut params = model_source(parsed, "plot", true)?;
    if !parsed.list("axes").is_empty() {
        params.insert("axes".into(), json!(parsed.list("axes")));
    }
    if let Some(o) = parsed.value("output") {
        params.insert("output".into(), json!(o));
        params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    }
    let result = execute("variations.plot", &Value::Object(params))?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    match (result.get("output").and_then(Value::as_str), result.get("svg").and_then(Value::as_str)) {
        (Some(path), _) => out(path),
        (None, Some(svg)) => out(svg.trim_end()),
        _ => Ok(()),
    }
}

/// `AXIS=A:B[:C]` items of a list option, grouped by axis.
fn axis_items(parsed: &Parsed, option: &str, arity: usize) -> Result<Map<String, Value>, Failure> {
    let mut map = Map::new();
    for item in parsed.list(option) {
        let (axis, values) = item
            .split_once('=')
            .filter(|(a, _)| !a.is_empty())
            .ok_or_else(|| usage(format!("invalid --{option} item `{item}`: give AXIS=VALUES")))?;
        let numbers: Vec<f64> =
            values.split(':').map(|v| v.trim().parse::<f64>().ok().filter(|v| v.is_finite())).collect::<Option<Vec<_>>>().unwrap_or_default();
        if numbers.len() != arity {
            return Err(usage(format!("invalid --{option} item `{item}`: give {arity} numbers separated by `:`")));
        }
        if arity == 2 {
            let entry = map.entry(axis.to_string()).or_insert_with(|| json!([]));
            if let Some(list) = entry.as_array_mut() {
                list.push(json!(numbers));
            }
        } else {
            map.insert(axis.to_string(), json!(numbers));
        }
    }
    Ok(map)
}

fn plan_avar(parsed: &Parsed) -> Outcome {
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let font = match parsed.positional.as_slice() {
        [font] => font,
        [] => return Err(usage("variations plan-avar needs a FONT")),
        _ => return Err(usage("variations plan-avar takes one FONT")),
    };
    readable(font)?;
    let mut targets = Map::new();
    for (option, axis) in [("weights", "wght"), ("widths", "wdth"), ("slants", "slnt"), ("sizes", "opsz")] {
        let Some(list) = parsed.value(option).filter(|v| !v.trim().is_empty()) else { continue };
        let values = list
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|v| !v.is_empty())
            .map(|v| {
                v.trim().parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| usage(format!("invalid --{option} value `{v}`: give numbers")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        targets.insert(axis.into(), json!(values));
    }
    let mut params = with_output(parsed, json!({ "font": font, "sanitize": parsed.switch("sanitize") }));
    let Some(obj) = params.as_object_mut() else { return Err(usage("variations plan-avar: bad parameters")) };
    if !targets.is_empty() {
        obj.insert("targets".into(), Value::Object(targets));
    }
    if let Some(n) = parsed.value("samples") {
        let n: u32 = n.parse().map_err(|_| usage(format!("--samples takes a whole number, got `{n}`")))?;
        obj.insert("samples".into(), json!(n));
    }
    let glyphs = parsed.list("glyphs");
    if !glyphs.is_empty() {
        let mut map = Map::new();
        for g in glyphs {
            let (name, frequency) = match g.rsplit_once(':') {
                Some((name, f)) => (
                    name,
                    f.parse::<f64>()
                        .ok()
                        .filter(|f| f.is_finite())
                        .ok_or_else(|| usage(format!("invalid --glyphs item `{g}`: give NAME or NAME:FREQUENCY")))?,
                ),
                None => (g.as_str(), 1.0),
            };
            map.insert(name.to_string(), json!(frequency));
        }
        obj.insert("glyphs".into(), Value::Object(map));
    }
    let pins = axis_items(parsed, "pins", 2)?;
    if !pins.is_empty() {
        obj.insert("pins".into(), Value::Object(pins));
    }
    let limits = axis_items(parsed, "design-limits", 3)?;
    if !limits.is_empty() {
        obj.insert("designLimits".into(), Value::Object(limits));
    }
    if let Some(plot) = parsed.value("plot") {
        obj.insert("plot".into(), json!(plot));
    }
    let result = execute("variations.planAvar", &params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    for (axis, plan) in result.get("axes").and_then(Value::as_object).into_iter().flatten() {
        for w in plan.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            io::note(&format!("{axis}: {w}"));
        }
    }
    let snippet = result.get("designspace").and_then(Value::as_str).unwrap_or_default();
    if !snippet.is_empty() {
        out(snippet.trim_end_matches('\n'))?;
    }
    io::note(&format!("wrote {}", result.get("output").and_then(Value::as_str).unwrap_or_default()));
    Ok(())
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

//! `fontcraft-cli instance` (M12.20–M12.22): runs `font.instantiate`. Static and named
//! instances of compiled variable fonts (M12.20); partial instances (M12.21) and sources (M12.22)
//! report the engine's "not built yet" until their packages land.

use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "instance",
    summary: "Make static or partial instances of a variable font or source",
    usage: USAGE,
    options: OPTIONS,
    actions: &[],
    run,
};

const USAGE: &str = "\
  fontcraft-cli instance INPUT [AXIS=LOC...] [--named REGEX | --all | --index LIST] [--static] [--to same|ttf|otf|ufo] [-o OUT | --output-dir DIR]
";

/// The options, as data (M12.20–M12.22).
const OPTIONS: &[OptSpec] = &[
    // M12.20
    OptSpec::list("location", "AXIS=LOC", "", "the same as a positional location (a value, MIN:MAX, MIN:DEFAULT:MAX or drop); repeatable"),
    OptSpec::switch("static", false, "pin every axis not mentioned at its default"),
    OptSpec::value("named", "REGEX", "", "build the named instances whose name matches"),
    OptSpec::switch("all", false, "build every named instance"),
    OptSpec::list("index", "LIST", "", "build the named instances with these zero-based indices"),
    OptSpec::value("to", "KIND", "", "output kind: same (default for fonts), ttf, otf or ufo"),
    OptSpec::switch("clamp", true, "clamp locations to the axis ranges"),
    OptSpec::switch("optimize-gvar", true, "optimise the variation data that remains"),
    OptSpec::switch("overlap-flag", true, "mark glyphs of a static TrueType instance as possibly overlapping"),
    OptSpec::switch("remove-overlaps", false, "merge overlapping contours of a static instance"),
    OptSpec::switch("ignore-overlap-errors", false, "keep glyphs whose overlaps cannot be removed as they are"),
    OptSpec::switch("update-name-table", false, "name the result from the STAT axis values of the location"),
    OptSpec::switch("downgrade-cff2", false, "write CFF instead of CFF2 when every axis is pinned"),
    OptSpec::switch("recalc-bounds", true, "recompute the font-wide bounds of the result"),
    OptSpec::switch("round", true, "round interpolated coordinates to integers"),
    OptSpec::value("timestamp", "keep|now|EPOCH", "", "head.modified of the result (default: SOURCE_DATE_EPOCH, else now)"),
    OptSpec::value("face", "N", "", "the font of a collection, from 0"),
    OptSpec::value("output", "FILE", "", "the output font (default: INPUT.instance.EXT beside the input)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder the outputs go into"),
    OptSpec::switch("in-place", false, "replace the input font"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::switch("json", false, "print the engine command's result as JSON"),
];

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let params = params(&parsed)?;
    let mut session = fontcraft_engine::Session::new();
    let result = io::execute(&mut session, "font.instantiate", &params)?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    for r in result.get("results").and_then(Value::as_array).into_iter().flatten() {
        out(r.get("output").and_then(Value::as_str).unwrap_or_default())?;
    }
    Ok(())
}

/// One `AXIS=LOC` item as the engine's location entry.
fn location_item(item: &str) -> Result<(String, Value), Failure> {
    let (tag, loc) = item
        .split_once('=')
        .filter(|(t, v)| !t.is_empty() && !v.is_empty())
        .ok_or_else(|| usage(format!("invalid location `{item}`: give AXIS=VALUE, AXIS=MIN:MAX, AXIS=MIN:DEFAULT:MAX or AXIS=drop")))?;
    if loc == "drop" {
        return Ok((tag.to_string(), json!("drop")));
    }
    let number = |s: &str| -> Result<Option<f64>, Failure> {
        if s.is_empty() {
            return Ok(None);
        }
        s.parse::<f64>().ok().filter(|v| v.is_finite()).map(Some).ok_or_else(|| usage(format!("invalid number `{s}` in `{item}`")))
    };
    let parts: Vec<&str> = loc.split(':').collect();
    let value = match parts.as_slice() {
        [v] => json!(number(v)?.ok_or_else(|| usage(format!("`{item}` has no value")))?),
        [lo, hi] | [lo, _, hi] => {
            let mut range = Map::new();
            if let Some(v) = number(lo)? {
                range.insert("min".into(), json!(v));
            }
            if let [_, d, _] = parts.as_slice() {
                if let Some(v) = number(d)? {
                    range.insert("default".into(), json!(v));
                }
            }
            if let Some(v) = number(hi)? {
                range.insert("max".into(), json!(v));
            }
            Value::Object(range)
        }
        _ => return Err(usage(format!("invalid location `{item}`: at most three parts separated by `:`"))),
    };
    Ok((tag.to_string(), value))
}

fn params(parsed: &Parsed) -> Result<Value, Failure> {
    let (input, coords) = parsed.positional.split_first().ok_or_else(|| usage("instance needs an INPUT"))?;
    let mut location = Map::new();
    for item in coords.iter().map(String::as_str).chain(parsed.list("location").iter().map(String::as_str)) {
        let (tag, value) = location_item(item)?;
        if location.insert(tag.clone(), value).is_some() {
            return Err(usage(format!("the axis `{tag}` is given twice")));
        }
    }
    let mut p = Map::new();
    p.insert("input".into(), json!(input));
    if !location.is_empty() {
        p.insert("location".into(), Value::Object(location));
    }
    for (option, key) in [
        ("static", "static"),
        ("all", "all"),
        ("clamp", "clamp"),
        ("optimize-gvar", "optimizeGvar"),
        ("overlap-flag", "overlapFlag"),
        ("remove-overlaps", "removeOverlaps"),
        ("ignore-overlap-errors", "ignoreOverlapErrors"),
        ("update-name-table", "updateNameTable"),
        ("downgrade-cff2", "downgradeCff2"),
        ("recalc-bounds", "recalcBounds"),
        ("round", "round"),
        ("in-place", "inPlace"),
        ("no-clobber", "noClobber"),
    ] {
        p.insert(key.into(), json!(parsed.switch(option)));
    }
    for (option, key) in [("named", "named"), ("to", "to"), ("output", "output"), ("output-dir", "outputDir")] {
        if let Some(v) = parsed.value(option).filter(|v| !v.is_empty()) {
            p.insert(key.into(), json!(v));
        }
    }
    if !parsed.list("index").is_empty() {
        let mut indices = Vec::new();
        for item in parsed.list("index").iter().flat_map(|s| s.split(',')).filter(|s| !s.is_empty()) {
            let i: u64 = item.trim().parse().map_err(|_| usage(format!("--index takes zero-based whole numbers, got `{item}`")))?;
            indices.push(json!(i));
        }
        p.insert("indices".into(), Value::Array(indices));
    }
    if let Some(face) = parsed.value("face").filter(|v| !v.is_empty()) {
        let face: u32 = face.parse().map_err(|_| usage(format!("--face takes a whole number, got `{face}`")))?;
        p.insert("face".into(), json!(face));
    }
    if let Some(t) = parsed.value("timestamp").filter(|v| !v.is_empty()) {
        let value = match t {
            "keep" | "now" => json!(t),
            _ => json!(t.parse::<i64>().map_err(|_| usage(format!("--timestamp takes keep, now or seconds since 1970, got `{t}`")))?),
        };
        p.insert("timestamp".into(), value);
    }
    Ok(Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Parsed {
        args::parse(OPTIONS, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn instance_options_become_engine_parameters() {
        let p = params(&parsed(&["V.ttf", "wght=650", "wdth=75:100", "--location", "opsz=drop", "--no-overlap-flag", "-o", "out.ttf"])).unwrap();
        assert_eq!(p["input"], json!("V.ttf"));
        assert_eq!(p["location"]["wght"], json!(650.0));
        assert_eq!(p["location"]["wdth"], json!({ "min": 75.0, "max": 100.0 }));
        assert_eq!(p["location"]["opsz"], json!("drop"));
        assert_eq!(p["overlapFlag"], json!(false));
        assert_eq!(p["clamp"], json!(true));
        assert_eq!(p["output"], json!("out.ttf"));
        let p = params(&parsed(&["V.ttf", "wght=:500:", "--index", "0,2", "--timestamp", "keep"])).unwrap();
        assert_eq!(p["location"]["wght"], json!({ "default": 500.0 }), "empty parts keep the current bounds");
        assert_eq!(p["indices"], json!([0, 2]));
        assert_eq!(p["timestamp"], json!("keep"));
        assert!(params(&parsed(&["V.ttf", "wght=bold"])).is_err());
        assert!(params(&parsed(&["V.ttf", "wght=1", "wght=2"])).is_err());
        assert!(params(&parsed(&[])).is_err());
    }
}

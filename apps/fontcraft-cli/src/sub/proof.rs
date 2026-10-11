//! `fontcraft-cli proof` (M12.35, M12.36): the options of the `proof` row parsed into the
//! parameters of `font.proof`, and the files written printed one per line (or as JSON with
//! `--json`).

use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand { name: "proof", summary: "Make proof sheets", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli proof FONT... [--layout NAME] [--format pdf|svg|html] [--glyphs LIST] [--size PT | --glyphs-per-page N] [-o OUT] [options]
";

/// Largest `--glyphs-file`, in bytes.
const MAX_GLYPHS_FILE: usize = 16 << 20;
/// Most sizes a `--sizes` range expands to.
const MAX_SIZES: usize = 64;

/// The options, as data (M12.35, M12.36).
const OPTIONS: &[OptSpec] = &[
    OptSpec::value(
        "layout",
        "NAME",
        "palette",
        "palette | outline | waterfall (built); palette+outline | digi | boxes | hints | compare | synopsis | features | cmap | vorg | text",
    ),
    OptSpec::value("format", "FORMAT", "pdf", "pdf | svg | html; svg writes one file per page"),
    OptSpec::value("glyphs", "SELECTOR", "", "glyph names and ranges (a-b, a..b) to show, comma-separated; default all"),
    OptSpec::value("glyphs-file", "FILE", "", "a file of glyph names and ranges to show"),
    OptSpec::value("exclude-glyphs", "SELECTOR", "", "glyph names and ranges to leave out"),
    OptSpec::value("glyphs-per-page", "N", "", "scale glyphs so that N fit on a page (palette; default 156)"),
    OptSpec::value("size", "PT", "", "glyph size in points; overrides --glyphs-per-page"),
    OptSpec::list("sizes", "LIST", "", "waterfall sizes in points; ranges such as 8-12 accepted; default 36,24,20,18,16,14,12,10,9,8,7,6"),
    OptSpec::value("sort", "ORDER", "gid", "gid | name | unicode | lookup"),
    OptSpec::value("text", "STRING", "", "text for the waterfall and the HTML sample"),
    OptSpec::list("location", "AXIS=VALUE", "", "proof a variable font at this location"),
    OptSpec::value("page-size", "WxH", "612x792", "page size in points"),
    OptSpec::value("face", "N", "", "the font of a collection to proof"),
    OptSpec::value("output", "OUT", "", "the output file (one font only)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "write proofs into this folder"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing file"),
    OptSpec::switch("json", false, "print the result as JSON"),
    OptSpec::switch("quiet", false, "print nothing but errors").short('q'),
];

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    let params = params(&parsed)?;
    let mut session = fontcraft_engine::Session::new();
    let result = io::execute(&mut session, "font.proof", &Value::Object(params))?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    if parsed.switch("quiet") {
        return Ok(());
    }
    for proof in result["proofs"].as_array().into_iter().flatten() {
        for file in proof["files"].as_array().into_iter().flatten() {
            out(file.as_str().unwrap_or_default())?;
        }
    }
    Ok(())
}

fn params(parsed: &args::Parsed) -> Result<Map<String, Value>, Failure> {
    let mut p = Map::new();
    p.insert("fonts".into(), json!(parsed.positional));
    for (option, key) in
        [("layout", "layout"), ("format", "format"), ("sort", "sort"), ("text", "text"), ("output", "output"), ("output-dir", "outputDir")]
    {
        if let Some(v) = parsed.value(option).filter(|v| !v.is_empty()) {
            p.insert(key.into(), json!(v));
        }
    }
    let number = |option: &str| -> Result<Option<f64>, Failure> {
        parsed
            .value(option)
            .filter(|v| !v.is_empty())
            .map(|v| v.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| usage(format!("--{option} needs a number, got `{v}`"))))
            .transpose()
    };
    if let Some(n) = number("glyphs-per-page")? {
        p.insert("glyphsPerPage".into(), json!(n));
    }
    if let Some(n) = number("size")? {
        p.insert("size".into(), json!(n));
    }
    if let Some(n) = number("face")? {
        p.insert("face".into(), json!(n));
    }
    if parsed.switch("no-clobber") {
        p.insert("noClobber".into(), json!(true));
    }
    let mut glyphs: Vec<String> = parsed.value("glyphs").map(|v| vec![v.to_string()]).unwrap_or_default();
    if let Some(file) = parsed.value("glyphs-file").filter(|v| !v.is_empty()) {
        glyphs.extend(glyphs_file(file)?);
    }
    if !glyphs.is_empty() {
        p.insert("glyphs".into(), json!(glyphs));
    }
    if let Some(v) = parsed.value("exclude-glyphs").filter(|v| !v.is_empty()) {
        p.insert("excludeGlyphs".into(), json!(v));
    }
    let sizes = sizes(parsed.list("sizes"))?;
    if !sizes.is_empty() {
        p.insert("sizes".into(), json!(sizes));
    }
    let mut location = Map::new();
    for item in parsed.list("location") {
        let (axis, value) = item.split_once('=').ok_or_else(|| usage(format!("--location takes AXIS=VALUE, got `{item}`")))?;
        let value = value.trim().parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| usage(format!("--location {axis} needs a number")))?;
        location.insert(axis.trim().to_string(), json!(value));
    }
    if !location.is_empty() {
        p.insert("location".into(), Value::Object(location));
    }
    if let Some(size) = parsed.value("page-size").filter(|v| !v.is_empty()) {
        let parse = |v: &str| v.trim().parse::<f64>().ok().filter(|n| n.is_finite());
        let (w, h) = size
            .split_once(['x', 'X'])
            .and_then(|(w, h)| Some((parse(w)?, parse(h)?)))
            .ok_or_else(|| usage(format!("--page-size takes WxH in points, got `{size}`")))?;
        p.insert("pageSize".into(), json!([w, h]));
    }
    Ok(p)
}

/// `--sizes` items: numbers, or whole-number ranges `8-12` (every size from 8 to 12).
fn sizes(items: &[String]) -> Result<Vec<f64>, Failure> {
    let mut out = Vec::new();
    for item in items {
        let bad = || usage(format!("--sizes takes numbers and ranges such as 8-12, got `{item}`"));
        match item.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (a.trim().parse::<u32>().map_err(|_| bad())?, b.trim().parse::<u32>().map_err(|_| bad())?);
                if b < a || (b - a) as usize >= MAX_SIZES {
                    return Err(bad());
                }
                out.extend((a..=b).map(f64::from));
            }
            None => out.push(item.trim().parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(bad)?),
        }
        if out.len() > MAX_SIZES {
            return Err(usage(format!("--sizes takes at most {MAX_SIZES} sizes")));
        }
    }
    Ok(out)
}

/// Glyph names and ranges from a file: separated by white space or commas, `#` starts a comment.
fn glyphs_file(path: &str) -> Result<Vec<String>, Failure> {
    let bytes = io::read_input(path)?;
    if bytes.len() > MAX_GLYPHS_FILE {
        return Err(Failure::Input(format!("{path} is larger than {MAX_GLYPHS_FILE} bytes")));
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(text
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default())
        .flat_map(|l| l.split([',', ' ', '\t']))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_options_become_font_proof_parameters() {
        let args: Vec<String> =
            ["A.ttf", "--format", "svg", "--sizes", "8-10,12", "--location", "wght=700", "--page-size", "595x842", "-o", "out.svg"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        let parsed = args::parse(OPTIONS, &args).unwrap();
        let p = params(&parsed).unwrap();
        assert_eq!(p["fonts"], json!(["A.ttf"]));
        assert_eq!(p["layout"], "palette");
        assert_eq!(p["format"], "svg");
        assert_eq!(p["sizes"], json!([8.0, 9.0, 10.0, 12.0]));
        assert_eq!(p["location"], json!({"wght": 700.0}));
        assert_eq!(p["pageSize"], json!([595.0, 842.0]));
        assert_eq!(p["output"], "out.svg");
        assert!(sizes(&["12-8".to_string()]).is_err());
    }
}

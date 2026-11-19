//! `fontcraft-cli proof` (M12.35, M12.36, M12.78, M12.79): the options of the `proof` row parsed
//! into the parameters of `font.proof`, and the files written printed one per line (or as JSON
//! with `--json`). `--set`, `--show`, `--hide` and `--panel` are read in order before the other
//! options: one after `--panel N` belongs to copy N of the digi layout.

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
        "palette | outline | palette+outline | digi | boxes | hints | waterfall | compare | synopsis | features | cmap | vorg | text",
    ),
    OptSpec::value("format", "FORMAT", "pdf", "pdf | svg | html; svg writes one file per page"),
    OptSpec::value("glyphs", "SELECTOR", "", "glyph names and ranges (a-b, a..b) to show, comma-separated; default all"),
    OptSpec::value("glyphs-file", "FILE", "", "a file of glyph names and ranges to show"),
    OptSpec::value("exclude-glyphs", "SELECTOR", "", "glyph names and ranges to leave out"),
    OptSpec::value("glyphs-per-page", "N", "", "scale glyphs so that N fit on a page (palette and boxes 156, palette+outline 320, hints 4, digi 1)"),
    OptSpec::value("size", "PT", "", "glyph size in points; overrides --glyphs-per-page"),
    OptSpec::list("sizes", "LIST", "", "waterfall sizes in points; ranges such as 8-12 accepted; default 36,24,20,18,16,14,12,10,9,8,7,6"),
    OptSpec::value("sort", "ORDER", "gid", "gid | name | unicode | lookup (features: rules in the font's order)"),
    OptSpec::value("text", "STRING", "", "text for the text and waterfall layouts and the HTML sample"),
    OptSpec::list("features", "TAG,...", "", "features the features layout shows; '*' for all (default)"),
    OptSpec::list("location", "AXIS=VALUE", "", "proof a variable font at this location"),
    OptSpec::value("page-size", "WxH", "612x792", "page size in points"),
    OptSpec::value("metrics", "h|v|both", "", "side bearings boxes, hints and digi print: horizontal, vertical or both"),
    OptSpec::value("baseline", "N", "", "y of the em box's bottom edge in font units (boxes, hints, digi); default -120 for CID-keyed fonts, else 0"),
    OptSpec::list("hint-groups", "LIST", "", "with the hints layout, draw only the stems of these hint-replacement groups (zero-based)"),
    OptSpec::switch("black", false, "draw in black and grey only, for printing"),
    OptSpec::list(
        "show",
        "LIST",
        "",
        "drawing elements to turn on: em-box, bbox, advance, baseline, zones, hhints, vhints, points, handles, point-labels, contour-labels, center-marks, name, numeric-labels, sidebearings, hint-counts, path-counts, filled-thumbnail",
    ),
    OptSpec::list("hide", "LIST", "", "drawing elements to turn off, as --show"),
    OptSpec::switch("no-header", false, "leave out the title line with the font name and page number"),
    OptSpec::value("set", "NAME=VALUE", "", "set a drawing parameter; repeatable; --list-settings lists them"),
    OptSpec::value("panel", "N", "", "with the digi layout, the copy (0 left, 1 right) the --set, --show and --hide options after it change"),
    OptSpec::switch("list-settings", false, "print every drawing parameter with its default and meaning, and exit"),
    OptSpec::value("face", "N", "", "the font of a collection to proof"),
    OptSpec::value("output", "OUT", "", "the output file (one font only)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "write proofs into this folder"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing file"),
    OptSpec::switch("json", false, "print the result as JSON"),
    OptSpec::switch("quiet", false, "print nothing but errors").short('q'),
];

fn run(args: &[String]) -> Outcome {
    let Taken { rest, settings, show, hide, panels } = take_settings(args)?;
    let parsed = args::parse(OPTIONS, &rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let mut session = fontcraft_engine::Session::new();
    if parsed.switch("list-settings") {
        let result = io::execute(&mut session, "font.proof", &json!({ "listSettings": true }))?;
        if parsed.switch("json") {
            return io::out_json(&result);
        }
        for s in result.get("settings").and_then(|s| s.as_array()).into_iter().flatten() {
            let field = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            out(&format!("{} = {}  {}", field("name"), field("default"), field("about")))?;
        }
        return Ok(());
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    let mut params = params(&parsed)?;
    if !settings.is_empty() {
        params.insert("settings".into(), Value::Object(settings));
    }
    if !show.is_empty() {
        params.insert("show".into(), json!(show));
    }
    if !hide.is_empty() {
        params.insert("hide".into(), json!(hide));
    }
    if !panels.is_empty() {
        let list: Vec<Value> = panels
            .into_iter()
            .map(|p| {
                let mut panel = Map::new();
                panel.insert("index".into(), json!(p.index));
                if !p.settings.is_empty() {
                    panel.insert("settings".into(), Value::Object(p.settings));
                }
                if !p.show.is_empty() {
                    panel.insert("show".into(), json!(p.show));
                }
                if !p.hide.is_empty() {
                    panel.insert("hide".into(), json!(p.hide));
                }
                Value::Object(panel)
            })
            .collect();
        params.insert("panels".into(), Value::Array(list));
    }
    let result = io::execute(&mut session, "font.proof", &Value::Object(params))?;
    if parsed.switch("json") {
        return io::out_json(&result);
    }
    if parsed.switch("quiet") {
        return Ok(());
    }
    for proof in result.get("proofs").and_then(|p| p.as_array()).into_iter().flatten() {
        for file in proof.get("files").and_then(|f| f.as_array()).into_iter().flatten() {
            out(file.as_str().unwrap_or_default())?;
        }
    }
    Ok(())
}

fn params(parsed: &args::Parsed) -> Result<Map<String, Value>, Failure> {
    let mut p = Map::new();
    p.insert("fonts".into(), json!(parsed.positional));
    for (option, key) in [
        ("layout", "layout"),
        ("format", "format"),
        ("sort", "sort"),
        ("text", "text"),
        ("output", "output"),
        ("output-dir", "outputDir"),
        ("metrics", "metrics"),
    ] {
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
    if let Some(n) = number("baseline")? {
        p.insert("baseline".into(), json!(n));
    }
    if parsed.switch("black") {
        p.insert("black".into(), json!(true));
    }
    if parsed.switch("no-header") {
        p.insert("header".into(), json!(false));
    }
    let features: Vec<String> = parsed.list("features").iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    if !features.is_empty() {
        p.insert("features".into(), json!(features));
    }
    let groups = parsed
        .list("hint-groups")
        .iter()
        .map(|g| g.trim().parse::<u32>().map_err(|_| usage(format!("--hint-groups takes group numbers, got `{g}`"))))
        .collect::<Result<Vec<u32>, Failure>>()?;
    if !groups.is_empty() {
        p.insert("hintGroups".into(), json!(groups));
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

/// The drawing changes of one copy of the digi layout.
#[derive(Debug, Default)]
struct PanelArgs {
    index: usize,
    settings: Map<String, Value>,
    show: Vec<String>,
    hide: Vec<String>,
}

/// What [`take_settings`] takes out of the arguments.
#[derive(Debug, Default)]
struct Taken {
    /// The other arguments, for [`args::parse`].
    rest: Vec<String>,
    settings: Map<String, Value>,
    show: Vec<String>,
    hide: Vec<String>,
    panels: Vec<PanelArgs>,
}

/// The drawing changes of `--set NAME=VALUE`, `--show LIST` and `--hide LIST`, global and per
/// `--panel N`, taken out of `args` in order.
fn take_settings(args: &[String]) -> Result<Taken, Failure> {
    const TAKEN: [&str; 4] = ["--set", "--panel", "--show", "--hide"];
    let mut taken = Taken { rest: Vec::with_capacity(args.len()), ..Taken::default() };
    let mut panel: Option<usize> = None;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        i += 1;
        if arg == "--" {
            taken.rest.extend(args.iter().skip(i - 1).cloned());
            break;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if TAKEN.contains(&n) => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        if !TAKEN.contains(&name) {
            taken.rest.push(arg.clone());
            continue;
        }
        let value = match inline {
            Some(v) => v,
            None => {
                let v = args.get(i).cloned().ok_or_else(|| usage(format!("{name} needs a value")))?;
                i += 1;
                v
            }
        };
        if name == "--panel" {
            let index = value.trim().parse::<usize>().ok().filter(|n| *n < 2).ok_or_else(|| usage(format!("--panel takes 0 or 1, got `{value}`")))?;
            panel = Some(index);
            continue;
        }
        let target = match panel {
            None => None,
            Some(index) => {
                if !taken.panels.iter().any(|p| p.index == index) {
                    taken.panels.push(PanelArgs { index, ..PanelArgs::default() });
                }
                taken.panels.iter_mut().find(|p| p.index == index)
            }
        };
        if name == "--show" || name == "--hide" {
            let names = value.split(',').map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
            match (name == "--show", target) {
                (true, None) => taken.show.extend(names),
                (false, None) => taken.hide.extend(names),
                (true, Some(p)) => p.show.extend(names),
                (false, Some(p)) => p.hide.extend(names),
            }
            continue;
        }
        let (key, setting) = value.split_once('=').ok_or_else(|| usage(format!("--set takes NAME=VALUE, got `{value}`")))?;
        let (key, setting) = (key.trim().to_string(), Value::String(setting.trim().to_string()));
        match target {
            None => taken.settings.insert(key, setting),
            Some(p) => p.settings.insert(key, setting),
        };
    }
    Ok(taken)
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

    #[test]
    fn set_options_after_a_panel_belong_to_that_copy() {
        let args: Vec<String> =
            ["A.otf", "--layout", "digi", "--set", "hintColorRGB=(0, 0.75, 0)", "--panel", "1", "--set=drawPoint_PointLabels=0", "--black"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        let taken = take_settings(&args).unwrap();
        assert_eq!(taken.rest, ["A.otf", "--layout", "digi", "--black"]);
        assert_eq!(taken.settings["hintColorRGB"], "(0, 0.75, 0)");
        assert_eq!(taken.panels.len(), 1);
        assert_eq!(taken.panels[0].index, 1);
        assert_eq!(taken.panels[0].settings["drawPoint_PointLabels"], "0");
        assert!(take_settings(&["--panel".to_string(), "3".to_string()]).is_err());
    }

    #[test]
    fn show_and_hide_go_to_the_proof_or_to_the_panel_before_them() {
        let args: Vec<String> = ["A.otf", "--hide", "name,numeric-labels", "--show=bbox", "--panel", "1", "--hide", "handles", "--no-header"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let taken = take_settings(&args).unwrap();
        assert_eq!(taken.rest, ["A.otf", "--no-header"]);
        assert_eq!(taken.hide, ["name", "numeric-labels"]);
        assert_eq!(taken.show, ["bbox"]);
        assert_eq!(taken.panels[0].index, 1);
        assert_eq!(taken.panels[0].hide, ["handles"]);
        let parsed = args::parse(OPTIONS, &taken.rest).unwrap();
        assert_eq!(params(&parsed).unwrap()["header"], false);
    }
}

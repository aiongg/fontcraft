//! `fontcraft-cli compile`.

use super::Subcommand;
use crate::io::{Failure, Outcome, execute, open, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "compile", summary: "Compile sources to static or variable fonts", usage: USAGE, options: &[], actions: &[], run: compile };

const USAGE: &str = "\
  fontcraft-cli compile SOURCE... [--format ttf|otf]... [--container plain|woff|woff2]...
                        [-o FILE | --output-dir DIR] [--instances[=NAME,...]] [--masters-as-instances]
                        [--keep-overlaps] [--keep-direction] [--conversion-error EM] [--no-subroutinize]
                        [--timestamp SECS|now] [--ttfautohint PATH] [--plan]
  fontcraft-cli compile SOURCE... --variable [--format ttf|otf] [--settings NAME,...] [--container plain|woff|woff2]...
                        [-o FILE | --output-dir DIR] [--timestamp SECS|now]
      Compile each SOURCE to static fonts: every exportable instance (or the named ones; a font
      without instances builds its first master), plus each master with --masters-as-instances,
      once per format (default ttf) and container (default plain). Files are named by PostScript
      name in DIR (default: the current folder); with both formats each goes into a ttf or otf
      subfolder. -o names the one file a single SOURCE builds. Instances between masters are
      skipped until interpolation exists. Overlapping contours are merged unless --keep-overlaps is
      given. CFF charstrings are subroutinised unless --no-subroutinize is given or the source
      sets Disable Subroutines. --ttfautohint PATH names the trusted ttfautohint program: a source whose TTFAutohint
      binary path parameter names that same file is autohinted by it (default: none runs).
      --plan prints what would be built, without building.
      --variable builds variable fonts instead: one per variable font setting (or the settings
      --settings names), or one for the whole designspace when the source has none, with
      TrueType outlines, or CFF2 outlines with --format otf; files are named by the setting's file
      name. `--format variable` is the same as --variable, `--format variable-cff2` the same as
      --variable --format otf.
      One JSON line per SOURCE.
";

/// `compile`: each source is opened as a document and built by `font.compile` (or described by
/// `font.compilePlan` with `--plan`).
fn compile(args: &[String]) -> Outcome {
    let mut sources: Vec<&String> = Vec::new();
    let (mut formats, mut containers): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let (mut output, mut output_dir): (Option<&String>, Option<&String>) = (None, None);
    let mut params = serde_json::Map::new();
    let mut outlines = serde_json::Map::new();
    let mut plan = false;
    let mut variable = false;
    let mut settings: Option<Vec<String>> = None;
    let mut autohinter: Option<&String> = None;
    let mut it = args.iter();
    let list = |v: &str| -> Vec<String> { v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect() };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--format" => formats.extend(list(it.next().ok_or_else(|| usage("--format needs ttf or otf"))?)),
            "--container" => containers.extend(list(it.next().ok_or_else(|| usage("--container needs plain, woff or woff2"))?)),
            "-o" | "--output" => output = Some(it.next().ok_or_else(|| usage("-o needs a file"))?),
            "--output-dir" => output_dir = Some(it.next().ok_or_else(|| usage("--output-dir needs a folder"))?),
            "--instances" => {
                params.insert("instances".into(), json!(true));
            }
            "--masters-as-instances" => {
                params.insert("mastersAsInstances".into(), json!(true));
            }
            "--keep-overlaps" => {
                outlines.insert("removeOverlaps".into(), json!(false));
            }
            "--remove-overlaps" => {
                outlines.insert("removeOverlaps".into(), json!(true));
            }
            "--subroutinize" | "--no-subroutinize" => {
                params.insert("subroutinize".into(), json!(arg == "--subroutinize"));
            }
            "--keep-direction" => {
                outlines.insert("keepDirection".into(), json!(true));
            }
            "--conversion-error" => {
                let v = it.next().ok_or_else(|| usage("--conversion-error needs a number"))?;
                let e = v
                    .parse::<f64>()
                    .ok()
                    .filter(|e| e.is_finite())
                    .ok_or_else(|| usage(format!("--conversion-error must be a number, got `{v}`")))?;
                outlines.insert("conversionError".into(), json!(e));
            }
            "--timestamp" => {
                let v = it.next().ok_or_else(|| usage("--timestamp needs seconds since 1970 or `now`"))?;
                let t = if v == "now" {
                    json!("now")
                } else {
                    json!(v.parse::<i64>().map_err(|_| usage(format!("--timestamp must be whole seconds or `now`, got `{v}`")))?)
                };
                params.insert("timestamp".into(), t);
            }
            "--ttfautohint" => autohinter = Some(it.next().ok_or_else(|| usage("--ttfautohint needs the path of a ttfautohint program"))?),
            "--plan" => plan = true,
            "--variable" => variable = true,
            "--settings" => settings = Some(list(it.next().ok_or_else(|| usage("--settings needs setting names"))?)),
            other => match other.strip_prefix("--instances=") {
                Some(names) => {
                    params.insert("instances".into(), json!(list(names)));
                }
                None if other.starts_with('-') => return Err(usage(format!("unknown compile option `{other}`"))),
                None => sources.push(arg),
            },
        }
    }
    if sources.is_empty() {
        return Err(usage("compile needs at least one SOURCE"));
    }
    let destination = match (output, output_dir) {
        (Some(_), Some(_)) => return Err(usage("use either -o or --output-dir")),
        (Some(_), None) if sources.len() > 1 => return Err(usage("-o names one file; with several sources use --output-dir")),
        (Some(file), None) => file.clone(),
        (None, Some(dir)) => dir.clone(),
        (None, None) => ".".to_string(),
    };
    params.insert("destination".into(), json!(destination));
    if !formats.is_empty() {
        params.insert("formats".into(), json!(formats));
    }
    if !containers.is_empty() {
        params.insert("containers".into(), json!(containers));
    }
    if !outlines.is_empty() {
        params.insert("outlines".into(), Value::Object(outlines));
    }
    // `--format variable` is the reference's spelling of `--variable`.
    if formats.iter().any(|f| f == "variable") {
        formats.retain(|f| f != "variable");
        variable = true;
    }
    let mut cff2 = false;
    if formats.iter().any(|f| f == "variable-cff2") {
        formats.retain(|f| f != "variable-cff2");
        variable = true;
        cff2 = true;
    }
    if variable {
        match formats.as_slice() {
            [] => {}
            [f] if f == "ttf" => formats.clear(),
            [f] if f == "otf" => {
                formats.clear();
                cff2 = true;
            }
            _ => return Err(usage("--variable takes one --format: ttf (TrueType outlines) or otf (CFF2 outlines)")),
        }
    }
    if settings.is_some() && !variable {
        return Err(usage("--settings goes with --variable"));
    }
    if variable {
        if plan
            || !formats.is_empty()
            || params.contains_key("outlines")
            || params.contains_key("instances")
            || params.contains_key("mastersAsInstances")
            || params.contains_key("subroutinize")
        {
            return Err(usage("--variable takes only --format, --settings, --container, -o, --output-dir and --timestamp"));
        }
        params.remove("formats");
        if cff2 {
            params.insert("outlines".into(), json!("cff2"));
        }
        if let Some(names) = settings {
            params.insert("settings".into(), json!(names));
        }
    }
    let command = if variable {
        "file.exportVariable"
    } else if plan {
        "font.compilePlan"
    } else {
        "font.compile"
    };
    if plan {
        params.remove("timestamp");
    }
    let params = Value::Object(params);
    let build = |source: &str| -> Result<Value, Failure> {
        let mut session = Session::new();
        if let Some(path) = autohinter {
            session
                .settings_mut()
                .set(fontcraft_engine::external_autohint::SETTING, json!(path))
                .map_err(|e| usage(format!("--ttfautohint: {e}")))?;
        }
        open(&mut session, source)?;
        execute(&mut session, command, &params)
    };
    if let [source] = sources.as_slice() {
        let result = build(source)?;
        return out(&json!({"source": source, "result": result}).to_string());
    }
    // Several sources: one failing does not stop the others.
    let mut failed = 0usize;
    for source in &sources {
        match build(source) {
            Ok(result) => out(&json!({"source": source, "result": result}).to_string())?,
            Err(e) => {
                eprintln!("fontcraft-cli: {source}: {}", e.message());
                failed += 1;
            }
        }
    }
    if failed > 0 { Err(Failure::Failed(format!("{failed} of {} sources did not build", sources.len()))) } else { Ok(()) }
}

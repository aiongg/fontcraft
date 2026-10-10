//! `fontcraft-cli`: FontCraft from the command line.
//!
//! ```text
//! fontcraft-cli commands [PATTERN]
//! fontcraft-cli run [--in FILE] [--cmd ID [--params JSON]]... [--save [FILE]] [--export FILE]
//! fontcraft-cli convert IN OUT [--smart-components components|outlines]
//! fontcraft-cli compile SOURCE... [--format ttf|otf]... [--container plain|woff|woff2]... [-o FILE | --output-dir DIR] …
//! fontcraft-cli info FILE [--section NAME,...] [--all-names] [--face N]
//! fontcraft-cli tables list FONT [--sort file|tag|size] [--face N|all]
//! fontcraft-cli glyphs list FONT [--face N]
//! fontcraft-cli mcp [--connect ADDR | --headless]
//! fontcraft-cli oracle (SOURCE... | --corpus [DIR]) [--against fontc|fontmake] [--format ttf|otf] [--json] …
//! ```
//! Every subcommand is a thin shell over engine commands (`spec/inventory/20-cli.toml` lists the
//! full planned set and the reference tools each one replaces). Results are JSON on stdout and
//! messages on stderr. Exit status, shared by every subcommand (`cli.global.exitCodes`): 0 success,
//! 2 usage error, 3 an input could not be read, 4 the operation failed. (1 is reserved for "it ran
//! and found something": differences, failed checks.)
#![forbid(unsafe_code)]

use std::io::Write as _;
use std::process::ExitCode;

use fontcraft_engine::Session;
use fontcraft_mcp::{Backend, DEFAULT_ADDR, Headless, Remote, Server};
use serde_json::{Value, json};

const USAGE: &str = "\
fontcraft-cli: FontCraft from the command line

USAGE:
  fontcraft-cli commands [PATTERN]
      Print the command catalogue as JSON (only commands whose id or label contains PATTERN).

  fontcraft-cli run [--in FILE] [--cmd ID [--params JSON]]... [--save [FILE]] [--export FILE]
      Headless batch: open FILE (a source) or start a new font, run the commands in order and
      print one JSON result per line. Stops at the first command that fails. --save writes the
      document (to FILE, or back to the file it was opened from); --export compiles it to a font.

  fontcraft-cli convert IN OUT [--smart-components components|outlines]
      Open IN (a source) and write OUT in the format its extension names (.fontcraft, .glyphs,
      .glyphspackage, .ufo for one master, .designspace with a UFO per master), or compile it to
      one static font (.ttf, .otf with CFF outlines, .woff, .woff2) of its first exportable
      instance. --smart-components: a UFO or designspace keeps smart components as components
      with their values (default) or holds them drawn as outlines.

  fontcraft-cli compile SOURCE... [--format ttf|otf]... [--container plain|woff|woff2]...
                        [-o FILE | --output-dir DIR] [--instances[=NAME,...]] [--masters-as-instances]
                        [--keep-overlaps] [--keep-direction] [--conversion-error EM]
                        [--timestamp SECS|now] [--plan]
      Compile each SOURCE to static fonts: every exportable instance (or the named ones; a font
      without instances builds its first master), plus each master with --masters-as-instances,
      once per format (default ttf) and container (default plain). Files are named by PostScript
      name in DIR (default: the current folder); with both formats each goes into a ttf or otf
      subfolder. -o names the one file a single SOURCE builds. Instances between masters are
      skipped until interpolation exists. Overlapping contours are merged unless --keep-overlaps is
      given. --plan prints what would be built, without building.
      One JSON line per SOURCE.

  fontcraft-cli info FILE [--section NAME,...] [--all-names] [--face N]
      Print a JSON summary of FILE (a compiled font or a source) without opening it: format, names,
      metrics, style, tables, cmap, layout, axes, instances, masters, color, hinting, faces.
  fontcraft-cli tables list FONT [--sort file|tag|size] [--face N|all]
      Print the table directory of a compiled font, collection, WOFF or WOFF2 file as JSON.
  fontcraft-cli glyphs list FONT [--face N]
      Print the glyphs of FONT (a compiled font or a source) in glyph order as JSON.

  fontcraft-cli mcp [--connect ADDR | --headless]
      Run the MCP server on stdio. Default: attach to a running app at 127.0.0.1:7985
      (fontcraft --control 7985), falling back to a headless session with a new font.

  fontcraft-cli oracle (SOURCE... | --corpus [DIR]) [--against fontc|fontmake] [--format ttf|otf]
                      [--reference-bin PATH] [--output-dir DIR] [--timestamp SECS] [--timeout SECS]
                      [--differences N] [--match TEXT] [--json]
      Development aid: compile each source with FontCraft and with a reference compiler (fontc
      for .ttf, fontmake for .otf), read both fonts back and report per table whether they are
      identical, equivalent, allowed or different. --corpus runs over the comparison set in DIR
      (default ./corpus). Exit status 1 when a table differs or a build fails.

  fontcraft-cli help | --version
";

enum Failure {
    /// The command line is wrong: exit status 2.
    Usage(String),
    /// An input file could not be read or is not a supported format: exit status 3.
    Input(String),
    /// A command failed: exit status 4.
    Failed(String),
    /// It ran and found something (differences, failed checks): exit status 1.
    Found(String),
}

type Outcome = Result<(), Failure>;

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

/// Set once stdout is closed: nothing more is printed, but the work goes on.
static STDOUT_CLOSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Prints a line to stdout. A reader that stopped early (`fontcraft-cli commands | head`) is not
/// an error, and must not stop work that was asked for: `run … --save x | head` still saves.
fn out(line: &str) -> Outcome {
    use std::sync::atomic::Ordering;
    if STDOUT_CLOSED.load(Ordering::Relaxed) {
        return Ok(());
    }
    match writeln!(std::io::stdout(), "{line}") {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            STDOUT_CLOSED.store(true, Ordering::Relaxed);
            Ok(())
        }
        Err(e) => Err(Failure::Failed(format!("cannot write to stdout: {e}"))),
    }
}

fn execute(session: &mut Session, id: &str, params: &Value) -> Result<Value, Failure> {
    session.execute(id, params).map_err(|e| match e {
        // Commands that only change the window live in the app, not in the engine.
        fontcraft_engine::EngineError::UnknownCommand(_) => Failure::Failed(format!(
            "{e} (if it is a window or view command, it needs the running app: drive it over the control channel or `fontcraft-cli mcp --connect`)"
        )),
        other => Failure::Failed(other.to_string()),
    })
}

/// Opens `path` as the session's document.
fn open(session: &mut Session, path: &str) -> Result<Value, Failure> {
    session.execute("file.open", &json!({"path": path})).map_err(|e| Failure::Input(e.to_string()))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("commands") => commands(rest),
        Some("run") => run(rest),
        Some("convert") => convert(rest),
        Some("compile") => compile(rest),
        Some("info") => info(rest),
        Some("tables") => tables(rest),
        Some("glyphs") => glyphs(rest),
        Some("mcp") => mcp(rest),
        Some("oracle") => oracle(rest),
        Some("-h" | "--help" | "help") | None => out(USAGE.trim_end()),
        Some("-V" | "--version") => out(&format!("fontcraft-cli {}", env!("CARGO_PKG_VERSION"))),
        Some(other) => Err(usage(format!("unknown subcommand `{other}`\n\n{USAGE}"))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(e)) => {
            eprintln!("fontcraft-cli: {e}");
            ExitCode::from(2)
        }
        Err(Failure::Input(e)) => {
            eprintln!("fontcraft-cli: {e}");
            ExitCode::from(3)
        }
        Err(Failure::Failed(e)) => {
            eprintln!("fontcraft-cli: {e}");
            ExitCode::from(4)
        }
        Err(Failure::Found(e)) => {
            eprintln!("fontcraft-cli: {e}");
            ExitCode::from(1)
        }
    }
}

fn commands(args: &[String]) -> Outcome {
    let pattern = match args {
        [] => Value::Null,
        [pattern] => json!(pattern),
        _ => return Err(usage("commands takes at most one PATTERN")),
    };
    let list = execute(&mut Session::new(), "commands.list", &json!({"pattern": pattern}))?;
    out(&serde_json::to_string_pretty(&list).map_err(|e| Failure::Failed(e.to_string()))?)
}

fn run(args: &[String]) -> Outcome {
    let mut input = None;
    let mut steps: Vec<(String, Value)> = Vec::new();
    let mut save: Option<Option<String>> = None;
    let mut export = None;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--in" => input = Some(it.next().ok_or_else(|| usage("--in needs a file"))?.clone()),
            "--cmd" => steps.push((it.next().ok_or_else(|| usage("--cmd needs a command id"))?.clone(), json!({}))),
            "--params" => {
                let text = it.next().ok_or_else(|| usage("--params needs a JSON object"))?;
                let value: Value = serde_json::from_str(text).map_err(|e| usage(format!("--params is not valid JSON: {e}")))?;
                match steps.last_mut() {
                    Some(step) => step.1 = value,
                    None => return Err(usage("--params must follow a --cmd")),
                }
            }
            "--save" => save = Some(it.next_if(|next| !next.starts_with("--")).cloned()),
            "--export" => export = Some(it.next().ok_or_else(|| usage("--export needs a file"))?.clone()),
            other => return Err(usage(format!("unknown run option `{other}`"))),
        }
    }
    let mut session = Session::new();
    match &input {
        Some(path) => open(&mut session, path)?,
        None => execute(&mut session, "file.new", &json!({}))?,
    };
    for (id, params) in &steps {
        let result = execute(&mut session, id, params).map_err(|e| match e {
            Failure::Failed(message) => Failure::Failed(format!("step `{id}` failed, nothing after it ran: {message}")),
            other => other,
        })?;
        out(&json!({"command": id, "result": result}).to_string())?;
    }
    if let Some(path) = save {
        let result = execute(&mut session, "file.save", &json!({"path": path}))?;
        out(&json!({"command": "file.save", "result": result}).to_string())?;
    }
    if let Some(path) = export {
        let result = execute(&mut session, "file.exportStatic", &json!({"destination": path}))?;
        out(&json!({"command": "file.exportStatic", "result": result}).to_string())?;
    }
    Ok(())
}

fn convert(args: &[String]) -> Outcome {
    let mut files: Vec<&String> = Vec::new();
    let mut smart: Option<&str> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--smart-components" => match it.next().map(String::as_str) {
                Some(v @ ("components" | "outlines")) => smart = Some(v),
                _ => return Err(usage("--smart-components needs components or outlines")),
            },
            other if other.starts_with("--") => return Err(usage(format!("unknown convert option `{other}`"))),
            _ => files.push(arg),
        }
    }
    let [input, output] = files.as_slice() else { return Err(usage("convert needs IN and OUT")) };
    let mut session = Session::new();
    open(&mut session, input)?;
    let extension = std::path::Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if smart.is_some() && !matches!(extension.as_str(), "ufo" | "designspace") {
        return Err(usage("--smart-components applies to .ufo and .designspace output"));
    }
    let result = match extension.as_str() {
        "ttf" | "otf" | "woff" | "woff2" => execute(&mut session, "file.exportStatic", &json!({"destination": output}))?,
        "fontcraft" | "glyphs" | "glyphspackage" | "ufo" | "designspace" => {
            let mut params = json!({"path": output});
            if let (Some(v), Value::Object(m)) = (smart, &mut params) {
                m.insert("smartComponents".into(), json!(v));
            }
            execute(&mut session, "file.saveAs", &params)?
        }
        other => {
            return Err(usage(format!(
                "cannot write `.{other}` yet (writable: .ttf, .otf, .woff, .woff2, .fontcraft, .glyphs, .glyphspackage, .ufo, .designspace; see ROADMAP.md)"
            )));
        }
    };
    out(&result.to_string())
}

/// `compile`: each source is opened as a document and built by `font.compile` (or described by
/// `font.compilePlan` with `--plan`).
fn compile(args: &[String]) -> Outcome {
    let mut sources: Vec<&String> = Vec::new();
    let (mut formats, mut containers): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let (mut output, mut output_dir): (Option<&String>, Option<&String>) = (None, None);
    let mut params = serde_json::Map::new();
    let mut outlines = serde_json::Map::new();
    let mut plan = false;
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
            "--plan" => plan = true,
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
    let command = if plan { "font.compilePlan" } else { "font.compile" };
    if plan {
        params.remove("timestamp");
    }
    let params = Value::Object(params);
    let build = |source: &str| -> Result<Value, Failure> {
        let mut session = Session::new();
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
            Err(Failure::Usage(e) | Failure::Input(e) | Failure::Failed(e) | Failure::Found(e)) => {
                eprintln!("fontcraft-cli: {source}: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 { Err(Failure::Failed(format!("{failed} of {} sources did not build", sources.len()))) } else { Ok(()) }
}

/// `--name VALUE` options after the positional arguments, as JSON parameters; `flags` are the
/// options that take no value.
fn options(args: &[String], names: &[(&str, &str)], flags: &[(&str, &str)]) -> Result<(Vec<String>, serde_json::Map<String, Value>), Failure> {
    let mut positional = Vec::new();
    let mut params = serde_json::Map::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some((_, key)) = flags.iter().find(|(f, _)| f == a) {
            params.insert((*key).to_string(), json!(true));
        } else if let Some((_, key)) = names.iter().find(|(f, _)| f == a) {
            let v = it.next().ok_or_else(|| usage(format!("{a} needs a value")))?;
            let value = match *key {
                "face" => v.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(v)),
                "sections" => json!(v.split(',').collect::<Vec<_>>()),
                _ => json!(v),
            };
            params.insert((*key).to_string(), value);
        } else if a.starts_with("--") {
            return Err(usage(format!("unknown option {a}")));
        } else {
            positional.push(a.clone());
        }
    }
    Ok((positional, params))
}

fn info(args: &[String]) -> Outcome {
    let (files, mut params) = options(args, &[("--section", "sections"), ("--face", "face")], &[("--all-names", "allNames")])?;
    let [input] = files.as_slice() else { return Err(usage("info needs one FILE")) };
    params.insert("input".into(), json!(input));
    let mut session = Session::new();
    let summary = session.execute("font.inspect", &Value::Object(params)).map_err(|e| Failure::Input(e.to_string()))?;
    out(&serde_json::to_string_pretty(&summary).map_err(|e| Failure::Failed(e.to_string()))?)
}

fn tables(args: &[String]) -> Outcome {
    let Some((verb, rest)) = args.split_first() else { return Err(usage("tables needs a subcommand: list")) };
    if verb != "list" {
        return Err(usage(format!("unknown tables subcommand `{verb}` (known: list)")));
    }
    let (files, mut params) = options(rest, &[("--sort", "sort"), ("--face", "face")], &[])?;
    let [font] = files.as_slice() else { return Err(usage("tables list needs one FONT")) };
    params.insert("font".into(), json!(font));
    let mut session = Session::new();
    let result = session.execute("tables.list", &Value::Object(params)).map_err(|e| Failure::Input(e.to_string()))?;
    out(&serde_json::to_string_pretty(&result).map_err(|e| Failure::Failed(e.to_string()))?)
}

fn glyphs(args: &[String]) -> Outcome {
    let Some((verb, rest)) = args.split_first() else { return Err(usage("glyphs needs a subcommand: list")) };
    if verb != "list" {
        return Err(usage(format!("unknown glyphs subcommand `{verb}` (known: list)")));
    }
    let (files, mut params) = options(rest, &[("--face", "face")], &[])?;
    let [font] = files.as_slice() else { return Err(usage("glyphs list needs one FONT")) };
    params.insert("font".into(), json!(font));
    let mut session = Session::new();
    let result = session.execute("font.listGlyphs", &Value::Object(params)).map_err(|e| Failure::Input(e.to_string()))?;
    out(&serde_json::to_string_pretty(&result).map_err(|e| Failure::Failed(e.to_string()))?)
}

fn mcp(args: &[String]) -> Outcome {
    let mut connect = None;
    let mut headless = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--connect" => connect = Some(it.next().ok_or_else(|| usage("--connect needs an address"))?.clone()),
            "--headless" => headless = true,
            other => return Err(usage(format!("unknown mcp option `{other}`"))),
        }
    }
    if headless && connect.is_some() {
        return Err(usage("use either --connect or --headless"));
    }
    // A headless session is an editing session like the app's: changed glyphs get Last Changed
    // stamps. The other subcommands leave them out, so their output does not depend on the time.
    let headless_session = || {
        let mut h = Headless::with_document();
        h.0.set_clock(Some(fontcraft_engine::clock::system_clock()));
        h
    };
    let backend: Box<dyn Backend> = if headless {
        Box::new(headless_session())
    } else if let Some(addr) = connect {
        // An explicit address: fail loudly if the app is not there.
        Box::new(Remote::connect(&addr).map_err(|e| Failure::Failed(format!("cannot connect to {addr}: {e}")))?)
    } else {
        match Remote::connect(DEFAULT_ADDR) {
            Ok(remote) => Box::new(remote),
            Err(_) => {
                eprintln!("fontcraft-cli: no app at {DEFAULT_ADDR}; running a headless session");
                Box::new(headless_session())
            }
        }
    };
    // stdout carries only protocol messages; everything else goes to stderr.
    Server::new(backend).serve(std::io::stdin().lock(), std::io::stdout().lock()).map_err(|e| Failure::Failed(format!("MCP transport: {e}")))
}

fn oracle(args: &[String]) -> Outcome {
    use fontcraft_oracle::{Flavour, Oracle, OursError, Settings};
    use std::path::{Path, PathBuf};

    let mut sources: Vec<String> = Vec::new();
    let mut corpus: Option<PathBuf> = None;
    let (mut against, mut flavour) = (None, None);
    let (mut tool, mut keep, mut filter) = (None, None, None);
    let mut timestamp = fontcraft_oracle::DEFAULT_TIMESTAMP;
    let mut timeout = fontcraft_oracle::DEFAULT_TIMEOUT;
    let mut shown = fontcraft_oracle::DEFAULT_SHOWN;
    let mut as_json = false;
    let mut it = args.iter().peekable();
    let number = |opt: &str, v: Option<&String>, max: u64| -> Result<u64, Failure> {
        let v = v.ok_or_else(|| usage(format!("{opt} needs a number")))?;
        v.parse::<u64>().ok().filter(|n| *n <= max).ok_or_else(|| usage(format!("{opt} must be a whole number up to {max}, got `{v}`")))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--corpus" => corpus = Some(PathBuf::from(it.next_if(|n| !n.starts_with("--")).map(String::as_str).unwrap_or("corpus"))),
            "--against" => {
                let v = it.next().ok_or_else(|| usage("--against needs fontc or fontmake"))?;
                against = Some(Oracle::parse(v).ok_or_else(|| usage(format!("--against must be fontc or fontmake, got `{v}`")))?);
            }
            "--format" => {
                let v = it.next().ok_or_else(|| usage("--format needs ttf or otf"))?;
                flavour = Some(Flavour::parse(v).ok_or_else(|| usage(format!("--format must be ttf or otf, got `{v}`")))?);
            }
            "--reference-bin" => tool = Some(PathBuf::from(it.next().ok_or_else(|| usage("--reference-bin needs a path"))?)),
            "--output-dir" => keep = Some(PathBuf::from(it.next().ok_or_else(|| usage("--output-dir needs a folder"))?)),
            "--match" => filter = Some(it.next().ok_or_else(|| usage("--match needs a text"))?.clone()),
            // Years 1970 to 9999, which the head table and SOURCE_DATE_EPOCH both hold.
            "--timestamp" => timestamp = number("--timestamp", it.next(), 253_402_300_799)? as i64,
            "--timeout" => timeout = std::time::Duration::from_secs(number("--timeout", it.next(), 86_400)?.max(1)),
            "--differences" => shown = number("--differences", it.next(), 10_000)? as usize,
            "--json" => as_json = true,
            other if other.starts_with("--") => return Err(usage(format!("unknown oracle option `{other}`"))),
            source => sources.push(source.to_string()),
        }
    }
    let flavour = flavour.unwrap_or(Flavour::TrueType);
    let against = against.unwrap_or(if flavour == Flavour::Cff { Oracle::Fontmake } else { Oracle::Fontc });
    if !against.builds(flavour) {
        return Err(usage(format!("{} cannot build .{} fonts; use --against fontmake", against.name(), flavour.extension())));
    }
    // (path, label) pairs.
    let mut jobs: Vec<(PathBuf, String)> = sources.iter().map(|s| (PathBuf::from(s), s.clone())).collect();
    if let Some(dir) = &corpus {
        let found = fontcraft_oracle::corpus::discover(dir).map_err(Failure::Input)?;
        jobs.extend(found.into_iter().map(|p| {
            let label = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().into_owned();
            (p, label)
        }));
    }
    if let Some(f) = &filter {
        jobs.retain(|(_, label)| label.contains(f.as_str()));
    }
    if jobs.is_empty() {
        return Err(usage("oracle needs SOURCE... or --corpus (and --match must leave something)"));
    }
    for (path, _) in &jobs {
        if !path.exists() {
            return Err(Failure::Input(format!("{}: no such file or folder", path.display())));
        }
    }
    let Some(tool) = fontcraft_oracle::tools::locate(against, tool.as_deref()) else {
        return Err(Failure::Failed(format!(
            "{} not found ({}, put it on PATH, set {} or pass --reference-bin); nothing was compared",
            against.name(),
            against.install_hint(),
            against.env_var()
        )));
    };
    let allow = fontcraft_oracle::AllowList::builtin().map_err(Failure::Failed)?;
    let settings = Settings { oracle: against, flavour, tool, timestamp, timeout, keep, shown, allow };

    let ours = |path: &Path| -> Result<fontcraft_oracle::Ours, OursError> {
        let built = fontcraft_engine::guard::catch_panic(|| -> Result<Result<fontcraft_oracle::Ours, OursError>, String> {
            let (font, warnings) = fontcraft_engine::io::open(path, None).map_err(|e| e.to_string())?;
            // The compiler builds one master. For a .glyphs source the reference compilers take
            // the variable font origin as the default: compile that master, so both sides build
            // the same one. Otherwise the first master (a designspace's first source).
            let glyphs = matches!(
                fontcraft_oracle::tools::SourceKind::of(path),
                Some(fontcraft_oracle::tools::SourceKind::Glyphs | fontcraft_oracle::tools::SourceKind::GlyphsPackage)
            );
            let origin = fontcraft_engine::model::axes::origin(&font).filter(|_| glyphs);
            let master = match origin {
                Some(m) => m,
                None => font.default_master().map_err(|e| e.to_string())?,
            };
            let mut warnings = warnings;
            if font.masters.first().is_some_and(|first| first.id != master.id) {
                warnings.push(format!("compiled master `{}`, the variable font origin, instead of the first", master.name));
            }
            let hint = fontcraft_oracle::Hint { family: font.family_name.clone(), style: master.name.clone() };
            let options = fontcraft_engine::compile::Options { timestamp, remove_overlap: against.removes_overlaps(), ..Default::default() };
            let target = fontcraft_engine::model::Target::Master(master.id.clone());
            let outlines = match flavour {
                Flavour::TrueType => fontcraft_engine::compile::Flavour::TrueType,
                Flavour::Cff => fontcraft_engine::compile::Flavour::Cff,
            };
            let compiled = fontcraft_engine::compile::compile(&font, &target, outlines, &options).map_err(|e| e.to_string())?;
            warnings.extend(compiled.warnings);
            Ok(Ok(fontcraft_oracle::Ours { bytes: compiled.bytes, warnings, hint }))
        });
        match built {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(OursError::Failed(e)),
            Err(panic) => Err(OursError::Failed(format!("internal error: {panic}"))),
        }
    };

    let mut reports = Vec::with_capacity(jobs.len());
    for (path, label) in &jobs {
        let report = fontcraft_oracle::check(path, label, &settings, || ours(path));
        if !as_json {
            out(fontcraft_oracle::render(&report).trim_end())?;
        }
        reports.push(report);
    }
    let summary = fontcraft_oracle::summarise(&reports);
    if as_json {
        let doc = json!({ "sources": reports, "summary": summary });
        out(&serde_json::to_string_pretty(&doc).map_err(|e| Failure::Failed(e.to_string()))?)?;
    } else if reports.len() > 1 {
        out("")?;
        out(fontcraft_oracle::render_summary(&summary).trim_end())?;
    }
    let differing = reports.iter().filter(|r| r.has_differences() || r.outcome != fontcraft_oracle::Outcome::Compared).count();
    if differing > 0 { Err(Failure::Found(format!("{differing} of {} sources differ or did not build", reports.len()))) } else { Ok(()) }
}

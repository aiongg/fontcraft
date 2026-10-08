//! `fontcraft-cli`: FontCraft from the command line.
//!
//! ```text
//! fontcraft-cli commands [PATTERN]
//! fontcraft-cli run [--in FILE] [--cmd ID [--params JSON]]... [--save [FILE]] [--export FILE]
//! fontcraft-cli convert IN OUT
//! fontcraft-cli info FILE
//! fontcraft-cli mcp [--connect ADDR | --headless]
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

  fontcraft-cli convert IN OUT
      Open IN (a source) and write OUT in the format its extension names (.ttf, .fontcraft,
      .glyphs, .glyphspackage).

  fontcraft-cli info FILE
      Print a JSON summary of FILE: names, units per em, masters, glyph count.

  fontcraft-cli mcp [--connect ADDR | --headless]
      Run the MCP server on stdio. Default: attach to a running app at 127.0.0.1:7985
      (fontcraft --control 7985), falling back to a headless session with a new font.

  fontcraft-cli help | --version
";

enum Failure {
    /// The command line is wrong: exit status 2.
    Usage(String),
    /// An input file could not be read or is not a supported format: exit status 3.
    Input(String),
    /// A command failed: exit status 4.
    Failed(String),
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
        Some("info") => info(rest),
        Some("mcp") => mcp(rest),
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
    let [input, output] = args else { return Err(usage("convert needs IN and OUT")) };
    let mut session = Session::new();
    open(&mut session, input)?;
    let extension = std::path::Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let result = match extension.as_str() {
        "ttf" => execute(&mut session, "file.exportStatic", &json!({"destination": output}))?,
        "fontcraft" | "glyphs" | "glyphspackage" => execute(&mut session, "file.saveAs", &json!({"path": output}))?,
        other => return Err(usage(format!("cannot write `.{other}` yet (writable: .ttf, .fontcraft, .glyphs, .glyphspackage; see ROADMAP.md)"))),
    };
    out(&result.to_string())
}

fn info(args: &[String]) -> Outcome {
    let [input] = args else { return Err(usage("info needs one FILE")) };
    let mut session = Session::new();
    open(&mut session, input)?;
    let summary = execute(&mut session, "font.info.get", &json!({}))?;
    out(&serde_json::to_string_pretty(&summary).map_err(|e| Failure::Failed(e.to_string()))?)
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
    let backend: Box<dyn Backend> = if headless {
        Box::new(Headless::with_document())
    } else if let Some(addr) = connect {
        // An explicit address: fail loudly if the app is not there.
        Box::new(Remote::connect(&addr).map_err(|e| Failure::Failed(format!("cannot connect to {addr}: {e}")))?)
    } else {
        match Remote::connect(DEFAULT_ADDR) {
            Ok(remote) => Box::new(remote),
            Err(_) => {
                eprintln!("fontcraft-cli: no app at {DEFAULT_ADDR}; running a headless session");
                Box::new(Headless::with_document())
            }
        }
    };
    // stdout carries only protocol messages; everything else goes to stderr.
    Server::new(backend).serve(std::io::stdin().lock(), std::io::stdout().lock()).map_err(|e| Failure::Failed(format!("MCP transport: {e}")))
}

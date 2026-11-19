//! Inputs, outputs, exit statuses and logging, shared by every subcommand.
//!
//! Exit status (`cli.global.exitCodes`, `docs/plans/M12.md` § 3): 0 nothing to report, 1 it ran
//! and found something (differences, failed checks), 2 usage error, 3 an input could not be
//! read, 4 the operation failed, 5 some inputs of a batch failed, 130 interrupted. Results go to
//! stdout (JSON, one document or one line per input); messages go to stderr.

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use fontcraft_engine::Session;
use serde_json::{Value, json};

/// Largest input read from a file or stdin, in bytes.
pub const MAX_INPUT: usize = 256 << 20;
/// Most inputs one batch run takes.
pub const MAX_BATCH_INPUTS: usize = 100_000;
/// Most worker threads `--jobs` may ask for.
pub const MAX_JOBS: usize = 256;

/// Why a subcommand stopped; each maps to one exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The command line is wrong: exit status 2.
    Usage(String),
    /// An input file could not be read or is not a supported format: exit status 3.
    Input(String),
    /// A command failed: exit status 4.
    Failed(String),
    /// It ran and found something (differences, failed checks): exit status 1.
    Found(String),
    /// Some inputs of a batch failed and the others succeeded: exit status 5.
    Partial(String),
    /// Interrupted by the user: exit status 130.
    Interrupted,
}

impl Failure {
    pub fn exit_code(&self) -> u8 {
        match self {
            Failure::Found(_) => 1,
            Failure::Usage(_) => 2,
            Failure::Input(_) => 3,
            Failure::Failed(_) => 4,
            Failure::Partial(_) => 5,
            Failure::Interrupted => 130,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Failure::Usage(m) | Failure::Input(m) | Failure::Failed(m) | Failure::Found(m) | Failure::Partial(m) => m,
            Failure::Interrupted => "interrupted",
        }
    }
}

pub type Outcome = Result<(), Failure>;

pub fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

/// Set once stdout is closed: nothing more is printed, but the work goes on.
static STDOUT_CLOSED: AtomicBool = AtomicBool::new(false);

/// Prints a line to stdout. A reader that stopped early (`fontcraft-cli commands | head`) is not
/// an error, and must not stop work that was asked for: `run … --save x | head` still saves.
pub fn out(line: &str) -> Outcome {
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

/// Prints one JSON value on one line (JSON lines).
pub fn out_json_line(value: &Value) -> Outcome {
    out(&serde_json::to_string(value).map_err(|e| Failure::Failed(e.to_string()))?)
}

/// Prints one JSON value, indented.
pub fn out_json(value: &Value) -> Outcome {
    out(&serde_json::to_string_pretty(value).map_err(|e| Failure::Failed(e.to_string()))?)
}

/// How much goes to stderr: 0 quiet, 1 normal, 2 verbose.
static VERBOSITY: AtomicU8 = AtomicU8::new(1);

pub fn set_verbosity(level: u8) {
    VERBOSITY.store(level, Ordering::Relaxed);
}

/// A message on stderr unless quiet.
pub fn note(message: &str) {
    if VERBOSITY.load(Ordering::Relaxed) >= 1 {
        eprintln!("fontcraft-cli: {message}");
    }
}

/// A message on stderr only when verbose.
pub fn detail(message: &str) {
    if VERBOSITY.load(Ordering::Relaxed) >= 2 {
        eprintln!("fontcraft-cli: {message}");
    }
}

/// Runs an engine command; an unknown id gets a hint about window commands.
pub fn execute(session: &mut Session, id: &str, params: &Value) -> Result<Value, Failure> {
    session.execute(id, params).map_err(|e| match e {
        // Commands that only change the window live in the app, not in the engine.
        fontcraft_engine::EngineError::UnknownCommand(_) => Failure::Failed(format!(
            "{e} (if it is a window or view command, it needs the running app: drive it over the control channel or `fontcraft-cli mcp --connect`)"
        )),
        other => Failure::Failed(other.to_string()),
    })
}

/// Opens `path` as the session's document. The command line keeps the glyph names the file
/// gives, as the reference compilers do: what it writes names glyphs as its input did.
pub fn open(session: &mut Session, path: &str) -> Result<Value, Failure> {
    session.execute("file.open", &json!({"path": path, "keepGlyphNames": true})).map_err(|e| Failure::Input(e.to_string()))
}

/// The bytes of an input: a file, or stdin for `-`. At most [`MAX_INPUT`] bytes.
pub fn read_input(path: &str) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    let limit = MAX_INPUT as u64 + 1;
    let read = if path == "-" {
        std::io::stdin().lock().take(limit).read_to_end(&mut bytes)
    } else {
        std::fs::File::open(path).and_then(|f| f.take(limit).read_to_end(&mut bytes))
    };
    read.map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))?;
    if bytes.len() > MAX_INPUT {
        return Err(Failure::Input(format!("{path} is larger than {MAX_INPUT} bytes")));
    }
    Ok(bytes)
}

/// Writes binary output to stdout (`-o -`). Refused when stdout is a terminal, where it would
/// only garble the screen.
#[allow(dead_code)] // The M12 subcommands that write fonts to stdout.
pub fn write_stdout_binary(bytes: &[u8]) -> Outcome {
    let mut stdout = std::io::stdout().lock();
    if stdout.is_terminal() {
        return Err(usage("refusing to write a binary font to the terminal: redirect stdout or give -o FILE"));
    }
    stdout.write_all(bytes).and_then(|()| stdout.flush()).map_err(|e| Failure::Failed(format!("cannot write to stdout: {e}")))
}

/// Runs `work` on every input with up to `jobs` threads and returns the results in input order,
/// so `--jobs 1` and `--jobs 8` print the same thing.
pub fn batch<T: Send>(inputs: &[String], jobs: usize, work: impl Fn(&str) -> Result<T, Failure> + Sync) -> Result<Vec<Result<T, Failure>>, Failure> {
    if inputs.len() > MAX_BATCH_INPUTS {
        return Err(usage(format!("at most {MAX_BATCH_INPUTS} inputs in one run")));
    }
    let jobs = jobs.clamp(1, MAX_JOBS).min(inputs.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Result<T, Failure>>>> = inputs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let (Some(input), Some(slot)) = (inputs.get(i), slots.get(i)) else { break };
                    let result = work(input);
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(result);
                    }
                }
            });
        }
    });
    Ok(slots
        .into_iter()
        .map(|s| s.into_inner().ok().flatten().unwrap_or_else(|| Err(Failure::Failed("a worker stopped before finishing".into()))))
        .collect())
}

/// The outcome of a batch: success when every input succeeded, [`Failure::Partial`] when some
/// failed, the failure itself when all failed.
pub fn batch_outcome<T>(results: &[Result<T, Failure>]) -> Outcome {
    let failed: Vec<&Failure> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    match (failed.len(), results.len()) {
        (0, _) => Ok(()),
        (f, n) if f == n => Err(failed.first().map(|f| (*f).clone()).unwrap_or(Failure::Failed("no input".into()))),
        (f, n) => Err(Failure::Partial(format!("{f} of {n} inputs failed"))),
    }
}

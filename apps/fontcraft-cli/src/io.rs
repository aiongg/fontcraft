//! Inputs, outputs, exit statuses and logging, shared by every subcommand.
//!
//! Exit status (`cli.global.exitCodes`, `docs/plans/M12.md` § 3): 0 nothing to report, 1 it ran
//! and found something (differences, failed checks), 2 usage error, 3 an input could not be
//! read, 4 the operation failed, 5 some inputs of a batch failed, 130 interrupted. Results go to
//! stdout (JSON, one document or one line per input); messages go to stderr.
//!
//! Messages (`cli.global.logging`, `.color`, `.timing`): [`error`], [`warn`], [`note`],
//! [`detail`] and [`debug`] write one line each to stderr, filtered by the level the common
//! options set ([`Level`]), as `fontcraft-cli: LEVEL: INPUT: message`. The input is the one
//! [`with_input`] (or [`batch`]) is working on. `--log-file` gets the same lines without colour.
//! Messages the libraries send through the `log` crate take the same path, with per-module
//! levels. [`stage`] times a stage for `--timing`.

use std::cell::RefCell;
use std::io::{IsTerminal as _, Read as _, Write as _};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use fontcraft_engine::Session;
use serde_json::{Value, json};

/// Largest input read from a file or stdin, in bytes.
pub const MAX_INPUT: usize = 256 << 20;
/// Most inputs one batch run takes.
pub const MAX_BATCH_INPUTS: usize = 100_000;
/// Most worker threads `--jobs` may ask for.
pub const MAX_JOBS: usize = 256;
/// Most distinct stages `--timing` keeps.
pub const MAX_STAGES: usize = 1_000;

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
    /// Nothing more to do and nothing more to say (`--version` printed the version): exit
    /// status 0.
    Done,
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
            Failure::Done => 0,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Failure::Usage(m) | Failure::Input(m) | Failure::Failed(m) | Failure::Found(m) | Failure::Partial(m) => m,
            Failure::Interrupted => "interrupted",
            Failure::Done => "",
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

/// How much goes to stderr. Each level shows itself and the levels before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Only errors (`-q`).
    Error,
    Warning,
    /// Warnings and notes, such as the files written: the default.
    Note,
    /// Also informational messages (`-v`).
    Info,
    /// Also debug output (`-vv`).
    Debug,
}

impl Level {
    /// The level a `--log` value names (`warn` and `trace` are accepted as well).
    pub fn parse(text: &str) -> Option<Level> {
        match text.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Level::Error),
            "warning" | "warn" => Some(Level::Warning),
            "note" => Some(Level::Note),
            "info" => Some(Level::Info),
            "debug" | "trace" => Some(Level::Debug),
            _ => None,
        }
    }

    /// The next level up, for each `-v`.
    pub fn louder(self) -> Level {
        match self {
            Level::Error => Level::Warning,
            Level::Warning => Level::Note,
            Level::Note => Level::Info,
            Level::Info | Level::Debug => Level::Debug,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
            Level::Info => "info",
            Level::Debug => "debug",
        }
    }

    /// The ANSI style of the level's name.
    fn style(self) -> &'static str {
        match self {
            Level::Error => "1;31",
            Level::Warning => "1;33",
            Level::Note => "1;36",
            Level::Info => "1;32",
            Level::Debug => "2",
        }
    }

    fn from_u8(value: u8) -> Level {
        match value {
            0 => Level::Error,
            1 => Level::Warning,
            2 => Level::Note,
            3 => Level::Info,
            _ => Level::Debug,
        }
    }
}

/// When diagnostics are coloured (`--color`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// When the stream is a terminal, `NO_COLOR` is unset or empty and `TERM` is not `dumb`.
    Auto,
    Always,
    Never,
}

impl Color {
    pub fn parse(text: &str) -> Option<Color> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Color::Auto),
            "always" => Some(Color::Always),
            "never" => Some(Color::Never),
            _ => None,
        }
    }
}

/// What the common options ask for; `None` and `false` leave the current setting alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub level: Option<Level>,
    /// Per-module levels for library messages (`--log compile=debug`).
    pub modules: Vec<(String, Level)>,
    pub log_file: Option<String>,
    pub color: Option<Color>,
    pub timing: bool,
    pub backtrace: bool,
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Note as u8);
static MODULES: Mutex<Vec<(String, Level)>> = Mutex::new(Vec::new());
/// 0 auto, 1 always, 2 never.
static COLOR: AtomicU8 = AtomicU8::new(0);
/// The `--log-file` path and the open file.
static LOG_FILE: Mutex<Option<(String, std::fs::File)>> = Mutex::new(None);
static TIMING: AtomicBool = AtomicBool::new(false);
/// Each stage `--timing` saw: its name, the time spent in it and how often it ran.
static STAGES: Mutex<Vec<(String, Duration, u32)>> = Mutex::new(Vec::new());

thread_local! {
    /// The input the current thread works on, named on each message.
    static INPUT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Applies what the common options asked for.
pub fn configure(setup: Setup) -> Outcome {
    // A subcommand may read its arguments twice: the file opened the first time is kept.
    if let Some(path) = setup.log_file.clone()
        && let Ok(mut slot) = LOG_FILE.lock()
        && slot.as_ref().is_none_or(|(open, _)| *open != path)
    {
        let file = std::fs::File::create(&path).map_err(|e| Failure::Failed(format!("cannot write the log file {path}: {e}")))?;
        *slot = Some((path, file));
    }
    if let Some(level) = setup.level {
        LEVEL.store(level as u8, Ordering::Relaxed);
    }
    if !setup.modules.is_empty()
        && let Ok(mut modules) = MODULES.lock()
    {
        *modules = setup.modules;
    }
    if let Some(color) = setup.color {
        COLOR.store(
            match color {
                Color::Auto => 0,
                Color::Always => 1,
                Color::Never => 2,
            },
            Ordering::Relaxed,
        );
    }
    if setup.timing {
        TIMING.store(true, Ordering::Relaxed);
    }
    if setup.backtrace {
        // A panic the engine recovers from is reported by the engine; this adds the stack.
        std::panic::set_hook(Box::new(|info| {
            let trace = std::backtrace::Backtrace::force_capture();
            emit("error", Level::Error.style(), &format!("panic: {info}\n{trace}"));
        }));
    }
    Ok(())
}

/// Sets the level by number, for subcommands with their own `-q` and `-v`: 0 errors only,
/// 1 the default, 2 informational messages, 3 and up debug output.
pub fn set_verbosity(level: u8) {
    let level = match level {
        0 => Level::Error,
        1 => Level::Note,
        2 => Level::Info,
        _ => Level::Debug,
    };
    LEVEL.store(level as u8, Ordering::Relaxed);
}

/// The current level.
pub fn level() -> Level {
    Level::from_u8(LEVEL.load(Ordering::Relaxed))
}

/// Writes `message` at `level`, when the level is shown.
pub fn log(level: Level, message: &str) {
    if level <= self::level() {
        emit(level.name(), level.style(), message);
    }
}

/// An error; always shown.
pub fn error(message: &str) {
    log(Level::Error, message);
}

/// A warning; hidden by `-q`.
pub fn warn(message: &str) {
    log(Level::Warning, message);
}

/// A message shown unless quiet: what was written, what was skipped.
pub fn note(message: &str) {
    log(Level::Note, message);
}

/// An informational message, shown with `-v`.
pub fn detail(message: &str) {
    log(Level::Info, message);
}

/// Debug output, shown with `-vv`.
pub fn debug(message: &str) {
    log(Level::Debug, message);
}

/// Runs `work` with `input` named on every message it writes from this thread.
pub fn with_input<T>(input: &str, work: impl FnOnce() -> T) -> T {
    let previous = INPUT.with(|i| i.replace(Some(input.to_string())));
    let result = work();
    INPUT.with(|i| i.replace(previous));
    result
}

/// Whether output written to stdout may carry colour (a diff on the terminal).
#[allow(dead_code)] // For the coloured diffs of later packages.
pub fn color_stdout() -> bool {
    use_color(std::io::stdout().is_terminal())
}

fn use_color(terminal: bool) -> bool {
    match COLOR.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => terminal && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()) && std::env::var_os("TERM").is_none_or(|t| t != "dumb"),
    }
}

/// Writes one line to stderr and to the log file: `fontcraft-cli: LABEL: INPUT: message`. The
/// input is left out when the message already starts with it.
fn emit(label: &str, style: &str, message: &str) {
    let input = INPUT.with(|i| i.borrow().clone());
    let text = match input {
        Some(input) if !message.starts_with(&format!("{input}:")) => format!("{input}: {message}"),
        _ => message.to_string(),
    };
    if let Ok(mut slot) = LOG_FILE.lock()
        && let Some((_, file)) = slot.as_mut()
    {
        let _ = writeln!(file, "fontcraft-cli: {label}: {text}");
    }
    let mut stderr = std::io::stderr().lock();
    let _ = if use_color(stderr.is_terminal()) {
        writeln!(stderr, "fontcraft-cli: \x1b[{style}m{label}\x1b[0m: {text}")
    } else {
        writeln!(stderr, "fontcraft-cli: {label}: {text}")
    };
}

/// Runs `work` as the stage `name`: with `--timing`, its time is added to the stage's total.
/// Stages may nest; each counts its own time.
pub fn stage<T>(name: &str, work: impl FnOnce() -> T) -> T {
    if !TIMING.load(Ordering::Relaxed) {
        return work();
    }
    let start = Instant::now();
    let result = work();
    let spent = start.elapsed();
    if let Ok(mut stages) = STAGES.lock() {
        let len = stages.len();
        match stages.iter_mut().find(|(n, _, _)| n == name) {
            Some(entry) => {
                entry.1 = entry.1.saturating_add(spent);
                entry.2 = entry.2.saturating_add(1);
            }
            None if len < MAX_STAGES => stages.push((name.to_string(), spent, 1)),
            None => {}
        }
    }
    result
}

/// With `--timing`: one line per stage in the order they first ran, then the total since
/// `started`.
pub fn report_timing(started: Instant) {
    if !TIMING.load(Ordering::Relaxed) {
        return;
    }
    let ms = |d: Duration| format!("{:.1} ms", d.as_secs_f64() * 1000.0);
    let stages = STAGES.lock().map(|s| s.to_vec()).unwrap_or_default();
    for (name, spent, runs) in stages {
        let runs = if runs > 1 { format!(" ({runs} runs)") } else { String::new() };
        emit("timing", "1;34", &format!("{name}: {}{runs}", ms(spent)));
    }
    emit("timing", "1;34", &format!("total: {}", ms(started.elapsed())));
}

/// Sends the libraries' `log` messages through [`emit`], filtered by the level and the
/// per-module levels.
struct Bridge;

static BRIDGE: Bridge = Bridge;

impl log::Log for Bridge {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let rule = MODULES
            .lock()
            .ok()
            .and_then(|m| m.iter().filter(|(name, _)| in_module(metadata.target(), name)).max_by_key(|(name, _)| name.len()).map(|(_, l)| *l));
        from_log(metadata.level()) <= rule.unwrap_or_else(level)
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let level = from_log(record.level());
            emit(level.name(), level.style(), &format!("{}: {}", record.target(), record.args()));
        }
    }

    fn flush(&self) {}
}

fn from_log(level: log::Level) -> Level {
    match level {
        log::Level::Error => Level::Error,
        log::Level::Warn => Level::Warning,
        log::Level::Info => Level::Info,
        log::Level::Debug | log::Level::Trace => Level::Debug,
    }
}

/// Whether a `log` target (`fontcraft_engine::guard`) lies in `module` (`engine`,
/// `fontcraft_engine` or `engine::guard`).
fn in_module(target: &str, module: &str) -> bool {
    let under = |t: &str| t.strip_prefix(module).is_some_and(|rest| rest.is_empty() || rest.starts_with("::"));
    under(target) || target.strip_prefix("fontcraft_").is_some_and(under)
}

/// Installs the `log` bridge; called once at start.
pub fn init() {
    if log::set_logger(&BRIDGE).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
}

/// The number of worker threads a `--jobs` value asks for (`cli.global.parallelism`): unset or 0
/// the number of cores, a negative N that many fewer than the cores (at least one), at most
/// [`MAX_JOBS`].
pub fn jobs(value: Option<&str>) -> Result<usize, Failure> {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let n: i64 = match value.map(str::trim) {
        None | Some("") => 0,
        Some(v) => v.parse().map_err(|_| usage(format!("--jobs needs a whole number, not `{v}`")))?,
    };
    let jobs = match n {
        0 => cores,
        n if n > 0 => usize::try_from(n).unwrap_or(MAX_JOBS),
        n => cores.saturating_sub(usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX)).max(1),
    };
    Ok(jobs.clamp(1, MAX_JOBS))
}

/// Runs an engine command; an unknown id gets a hint about window commands.
pub fn execute(session: &mut Session, id: &str, params: &Value) -> Result<Value, Failure> {
    debug(&format!("running {id}"));
    stage(id, || session.execute(id, params)).map_err(|e| match e {
        // Commands that only change the window live in the app, not in the engine.
        fontcraft_engine::EngineError::UnknownCommand(_) => Failure::Failed(format!(
            "{e} (if it is a window or view command, it needs the running app: drive it over the control channel or `fontcraft-cli mcp --connect`)"
        )),
        other => Failure::Failed(other.to_string()),
    })
}

/// Opens `path` as the session's document.
pub fn open(session: &mut Session, path: &str) -> Result<Value, Failure> {
    detail(&format!("opening {path}"));
    stage("file.open", || session.execute("file.open", &json!({"path": path}))).map_err(|e| Failure::Input(e.to_string()))
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
                    let result = with_input(input, || work(input));
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

//! `fontcraft-cli bench` (row `bench`): times one fontcraft-cli invocation, and reference
//! command lines on the same input, and compares the result with a stored one.
//!
//! Each run is a child process (this binary, or a reference tool found on PATH) with standard
//! input, output and error discarded, so every figure includes process start-up, as a user
//! running the tool would see it. Peak memory is the child's resident high-water mark (`VmHWM`),
//! read from `/proc` while it runs: Linux only, and a lower bound when the peak falls in the last
//! millisecond of a run. A run of ours succeeds with exit status 0 or 1 (1 means it found
//! something, such as differences); a reference tool must exit 0.
//!
//! `--baseline FILE` reads a result written by `--save` and exits 1 when the median wall time is
//! more than `--threshold` percent above the stored median.

use std::ffi::OsString;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::args::{self, Kind, OptSpec};
use crate::io::{Failure, Outcome, note, out, out_json, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "bench", summary: "Time subcommands on a font", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli bench [--runs N] [--warmup N] [--compare COMMAND]... [--stages] [--save FILE]
                      [--baseline FILE] [--threshold PCT] [--json] SUBCOMMAND [ARGS...]
      Run `fontcraft-cli SUBCOMMAND ARGS...` several times with its output discarded and report
      the minimum, median, maximum and standard deviation of its wall time and peak memory.
      Bench's own options come before SUBCOMMAND; everything from SUBCOMMAND on is timed (`--`
      may separate them). --compare times a reference command line (quoted, found on PATH) the
      same way. --save stores the result as JSON; --baseline compares with a stored result and
      exits 1 when the median is more than PCT percent slower.
";

/// The options, as data.
const OPTIONS: &[OptSpec] = &[
    OptSpec::value("runs", "N", "10", "timed runs"),
    OptSpec::value("warmup", "N", "1", "untimed runs before the timed ones"),
    OptSpec::value("compare", "COMMAND", "", "also time this reference command line (quoted); may be repeated"),
    OptSpec::switch("stages", false, "add the per-stage timing the subcommand prints with --timing"),
    OptSpec::value("save", "FILE", "", "store the result as JSON"),
    OptSpec::value("baseline", "FILE", "", "compare with a stored result; exit status 1 on a slowdown"),
    OptSpec::value("threshold", "PCT", "10", "slowdown of the median, in percent, that --baseline reports"),
    OptSpec::switch("json", false, "print the result as JSON"),
];

/// Most timed runs.
pub const MAX_RUNS: u64 = 10_000;
/// Most warm-up runs.
pub const MAX_WARMUP: u64 = 1_000;
/// Most reference command lines.
pub const MAX_COMPARE: usize = 16;
/// Largest stored result `--baseline` reads, in bytes.
pub const MAX_BASELINE: usize = 1 << 20;
/// Largest `--threshold`, in percent.
pub const MAX_THRESHOLD: f64 = 10_000.0;
/// Most bytes of stage report kept.
const MAX_STAGES: usize = 64 << 10;
/// How often peak memory is sampled.
const POLL: Duration = Duration::from_millis(1);

fn run(args: &[String]) -> Outcome {
    let (head, command) = split(args);
    let (mut compare, rest) = take_compare(&head)?;
    let parsed = args::parse(OPTIONS, &rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if !parsed.positional.is_empty() {
        return Err(usage("bench's options come before the subcommand it times"));
    }
    // A `--compare` read from a response file.
    if let Some(c) = parsed.value("compare") {
        compare.push(c.to_string());
    }
    if compare.len() > MAX_COMPARE {
        return Err(usage(format!("at most {MAX_COMPARE} --compare command lines")));
    }
    let runs = count(parsed.value("runs"), "runs", 1, MAX_RUNS)?;
    let warmup = count(parsed.value("warmup"), "warmup", 0, MAX_WARMUP)?;
    let threshold = parsed
        .value("threshold")
        .unwrap_or("10")
        .parse::<f64>()
        .ok()
        .filter(|t| t.is_finite() && (0.0..=MAX_THRESHOLD).contains(t))
        .ok_or_else(|| usage(format!("--threshold takes a percentage from 0 to {MAX_THRESHOLD}")))?;

    let Some(name) = command.first() else { return Err(usage(super::help(&SUBCOMMAND))) };
    if name == SUBCOMMAND.name {
        return Err(usage("bench cannot time itself"));
    }
    if super::find(name).is_none() {
        return Err(usage(format!("unknown subcommand `{name}`")));
    }
    let baseline = parsed.value("baseline").map(read_baseline).transpose()?;
    let exe = std::env::current_exe().map_err(|e| Failure::Failed(format!("cannot find this program's path: {e}")))?.into_os_string();

    let shown = format!("fontcraft-cli {}", command.join(" "));
    let ours = measure(&exe, &command, runs, warmup, Accept::ZeroOrOne).map_err(|e| e.into_failure(&shown))?;

    let mut compared = Vec::new();
    for line in &compare {
        let words = split_words(line);
        let Some((program, tool_args)) = words.split_first() else { return Err(usage("--compare needs a command line")) };
        let entry = match measure(&OsString::from(program), tool_args, runs, warmup, Accept::Zero) {
            Ok(m) => m.to_json(line),
            Err(RunError::Interrupted) => return Err(Failure::Interrupted),
            Err(e) => {
                note(&format!("{line}: {}", e.describe()));
                json!({"command": line, "error": e.describe()})
            }
        };
        compared.push(entry);
    }

    let stages = if parsed.switch("stages") { stage_report(&exe, &command, &shown)? } else { None };

    let mut result = ours.to_json(&shown);
    if let Some(obj) = result.as_object_mut() {
        obj.insert("tool".into(), json!("fontcraft-cli"));
        obj.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
        obj.insert("arguments".into(), json!(command));
        obj.insert("runs".into(), json!(runs));
        obj.insert("warmup".into(), json!(warmup));
        obj.insert("compare".into(), Value::Array(compared));
        obj.insert("stages".into(), stages.map_or(Value::Null, Value::String));
    }

    let verdict = match (&baseline, ours.wall().map(|s| s.median)) {
        (Some(base), Some(median)) => {
            if base.command.as_deref().is_some_and(|c| c != shown) {
                note(&format!("the baseline {} timed `{}`, not `{shown}`", base.file, base.command.as_deref().unwrap_or_default()));
            }
            let v = compare_with(base, median, threshold);
            if let Some(obj) = result.as_object_mut() {
                obj.insert("baseline".into(), v.to_json());
            }
            Some(v)
        }
        _ => None,
    };

    if let Some(path) = parsed.value("save") {
        save(path, &result)?;
    }
    if parsed.switch("json") {
        out_json(&result)?;
    } else {
        print_text(&result, verdict.as_ref())?;
    }
    match verdict {
        Some(v) if v.slower => Err(Failure::Found(format!(
            "the median wall time {:.3} ms is {:.1}% above the baseline's {:.3} ms (threshold {threshold}%)",
            v.median_ms, v.change_percent, v.baseline_ms
        ))),
        _ => Ok(()),
    }
}

/// Splits the arguments into bench's own options and the timed command line: bench's options
/// (with their values, and `@FILE`s) come first, and the first other word, or the word after
/// `--`, starts the command.
fn split(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if arg == "--" {
            let head = args.get(..i).unwrap_or_default().to_vec();
            return (head, args.get(i + 1..).unwrap_or_default().to_vec());
        }
        if arg.len() < 2 || !(arg.starts_with('-') || arg.starts_with('@')) {
            break;
        }
        if let Some(body) = arg.strip_prefix("--")
            && !body.contains('=')
            && takes_value(body)
        {
            i += 1;
        }
        i += 1;
    }
    let i = i.min(args.len());
    (args.get(..i).unwrap_or_default().to_vec(), args.get(i..).unwrap_or_default().to_vec())
}

/// Bench's option that `name` names, exactly or by a unique prefix.
fn spec_of(name: &str) -> Option<&'static OptSpec> {
    if let Some(spec) = OPTIONS.iter().find(|s| s.name == name) {
        return Some(spec);
    }
    let mut matches = OPTIONS.iter().filter(|s| s.name.starts_with(name));
    match (matches.next(), matches.next()) {
        (Some(spec), None) => Some(spec),
        _ => None,
    }
}

fn takes_value(name: &str) -> bool {
    name == "options" || spec_of(name).is_some_and(|s| s.kind != Kind::Switch)
}

/// Takes every `--compare` out of bench's options: it may be repeated, and the parser keeps one
/// value per option.
fn take_compare(head: &[String]) -> Result<(Vec<String>, Vec<String>), Failure> {
    let mut compare = Vec::new();
    let mut rest = Vec::new();
    let mut it = head.iter();
    while let Some(arg) = it.next() {
        if let Some(body) = arg.strip_prefix("--") {
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (body, None),
            };
            if spec_of(name).is_some_and(|s| s.name == "compare") {
                let value = match inline {
                    Some(v) => v.to_string(),
                    None => it.next().ok_or_else(|| usage("--compare needs a command line"))?.clone(),
                };
                compare.push(value);
                continue;
            }
        }
        rest.push(arg.clone());
    }
    Ok((compare, rest))
}

fn count(value: Option<&str>, name: &str, min: u64, max: u64) -> Result<u64, Failure> {
    value
        .unwrap_or_default()
        .parse::<u64>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| usage(format!("--{name} takes a whole number from {min} to {max}")))
}

/// Words of a `--compare` command line: separated by whitespace, grouped by single or double
/// quotes.
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Which exit statuses count as a completed run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Accept {
    /// Ours: 0, or 1 for "it ran and found something".
    ZeroOrOne,
    /// A reference tool.
    Zero,
}

#[derive(Debug)]
enum RunError {
    Spawn(std::io::Error),
    Status(String),
    Interrupted,
}

impl RunError {
    fn describe(&self) -> String {
        match self {
            RunError::Spawn(e) if e.kind() == std::io::ErrorKind::NotFound => "not found on PATH".to_string(),
            RunError::Spawn(e) => format!("cannot run it: {e}"),
            RunError::Status(s) => s.clone(),
            RunError::Interrupted => "interrupted".to_string(),
        }
    }

    fn into_failure(self, shown: &str) -> Failure {
        match self {
            RunError::Interrupted => Failure::Interrupted,
            RunError::Status(s) => Failure::Failed(format!("`{shown}` {s}; run it alone to see its message")),
            e => Failure::Failed(format!("`{shown}`: {}", e.describe())),
        }
    }
}

fn check_status(status: ExitStatus, accept: Accept) -> Result<(), RunError> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        // SIGINT: the user pressed Ctrl-C.
        if status.signal() == Some(2) {
            return Err(RunError::Interrupted);
        }
    }
    match (status.code(), accept) {
        (Some(0), _) | (Some(1), Accept::ZeroOrOne) => Ok(()),
        (Some(130), _) => Err(RunError::Interrupted),
        (Some(code), _) => Err(RunError::Status(format!("exited with status {code}"))),
        (None, _) => Err(RunError::Status("was stopped by a signal".to_string())),
    }
}

/// The figures of one measured command.
#[derive(Debug, Default)]
struct Measured {
    wall_ms: Vec<f64>,
    /// Peak memory of the runs that could be sampled, in KiB.
    peak_kib: Vec<f64>,
}

impl Measured {
    fn wall(&self) -> Option<Stats> {
        Stats::of(&self.wall_ms)
    }

    fn to_json(&self, shown: &str) -> Value {
        json!({
            "command": shown,
            "wall_ms": self.wall().map_or(Value::Null, |s| s.to_json(3)),
            "peak_memory_kib": Stats::of(&self.peak_kib).map_or(Value::Null, |s| s.to_json(0)),
        })
    }
}

fn measure(program: &OsString, args: &[String], runs: u64, warmup: u64, accept: Accept) -> Result<Measured, RunError> {
    for _ in 0..warmup {
        once(program, args, accept)?;
    }
    let mut measured = Measured::default();
    for _ in 0..runs {
        let (wall, peak) = once(program, args, accept)?;
        measured.wall_ms.push(wall.as_secs_f64() * 1000.0);
        if let Some(kib) = peak {
            measured.peak_kib.push(kib as f64);
        }
    }
    Ok(measured)
}

/// One run: its wall time and, where it can be sampled, its peak memory in KiB.
fn once(program: &OsString, args: &[String], accept: Accept) -> Result<(Duration, Option<u64>), RunError> {
    let start = Instant::now();
    let mut child =
        Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(RunError::Spawn)?;
    let pid = child.id();
    let done = AtomicBool::new(false);
    let (status, wall, peak) = std::thread::scope(|scope| {
        let sampler = scope.spawn(|| peak_memory(pid, &done));
        let status = child.wait();
        let wall = start.elapsed();
        done.store(true, Ordering::Relaxed);
        (status, wall, sampler.join().ok().flatten())
    });
    check_status(status.map_err(RunError::Spawn)?, accept)?;
    Ok((wall, peak))
}

/// The highest `VmHWM` read for `pid` until `done` is set; `None` where `/proc` has none.
fn peak_memory(pid: u32, done: &AtomicBool) -> Option<u64> {
    let path = format!("/proc/{pid}/status");
    let mut peak: Option<u64> = None;
    while !done.load(Ordering::Relaxed) {
        if let Some(kib) = std::fs::read_to_string(&path).ok().as_deref().and_then(vm_hwm) {
            peak = Some(peak.map_or(kib, |p| p.max(kib)));
        }
        std::thread::sleep(POLL);
    }
    peak
}

/// The `VmHWM:  1234 kB` line of a `/proc/PID/status` text, in KiB.
fn vm_hwm(status: &str) -> Option<u64> {
    let line = status.lines().find_map(|l| l.strip_prefix("VmHWM:"))?;
    line.split_whitespace().next()?.parse().ok()
}

/// Minimum, median, maximum and sample standard deviation.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stats {
    min: f64,
    median: f64,
    max: f64,
    stddev: f64,
}

impl Stats {
    fn of(values: &[f64]) -> Option<Stats> {
        let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
        v.sort_by(f64::total_cmp);
        let n = v.len();
        let (&min, &max) = (v.first()?, v.last()?);
        let mid = v.get(n / 2).copied()?;
        let median = if n % 2 == 1 { mid } else { (v.get((n / 2).saturating_sub(1)).copied().unwrap_or(mid) + mid) / 2.0 };
        let mean = v.iter().sum::<f64>() / n as f64;
        let stddev = if n > 1 { (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt() } else { 0.0 };
        Some(Stats { min, median, max, stddev })
    }

    fn to_json(self, decimals: i32) -> Value {
        let scale = 10f64.powi(decimals);
        let r = |x: f64| (x * scale).round() / scale;
        json!({"min": r(self.min), "median": r(self.median), "max": r(self.max), "stddev": r(self.stddev)})
    }
}

/// A stored result read for `--baseline`.
#[derive(Debug)]
struct Baseline {
    file: String,
    median_ms: f64,
    command: Option<String>,
}

fn read_baseline(path: &str) -> Result<Baseline, Failure> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_BASELINE as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))?;
    if bytes.len() > MAX_BASELINE {
        return Err(Failure::Input(format!("{path} is larger than {MAX_BASELINE} bytes")));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| Failure::Input(format!("{path} is not a stored bench result: {e}")))?;
    parse_baseline(path, &value)
}

fn parse_baseline(path: &str, value: &Value) -> Result<Baseline, Failure> {
    let median_ms = value
        .get("wall_ms")
        .and_then(|w| w.get("median"))
        .and_then(Value::as_f64)
        .filter(|m| m.is_finite() && *m > 0.0)
        .ok_or_else(|| Failure::Input(format!("{path} has no positive wall_ms.median: it is not a result written by bench --save")))?;
    Ok(Baseline { file: path.to_string(), median_ms, command: value.get("command").and_then(Value::as_str).map(str::to_string) })
}

/// How this run's median compares with the baseline's.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Verdict {
    median_ms: f64,
    baseline_ms: f64,
    change_percent: f64,
    threshold_percent: f64,
    slower: bool,
}

impl Verdict {
    fn to_json(self) -> Value {
        let r = |x: f64| (x * 1000.0).round() / 1000.0;
        json!({
            "median_ms": r(self.median_ms),
            "baseline_median_ms": r(self.baseline_ms),
            "change_percent": (self.change_percent * 10.0).round() / 10.0,
            "threshold_percent": self.threshold_percent,
            "slower": self.slower,
        })
    }
}

fn compare_with(base: &Baseline, median_ms: f64, threshold: f64) -> Verdict {
    let change_percent = (median_ms - base.median_ms) / base.median_ms * 100.0;
    Verdict { median_ms, baseline_ms: base.median_ms, change_percent, threshold_percent: threshold, slower: change_percent > threshold }
}

/// The stage report of one more run with `--timing`, when the subcommand prints one.
fn stage_report(exe: &OsString, command: &[String], shown: &str) -> Result<Option<String>, Failure> {
    let mut args = command.to_vec();
    let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
    args.insert(at, "--timing".to_string());
    let output = Command::new(exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| Failure::Failed(format!("`{shown} --timing`: cannot run it: {e}")))?;
    match check_status(output.status, Accept::ZeroOrOne) {
        Err(RunError::Interrupted) => return Err(Failure::Interrupted),
        Err(_) => {
            note(&format!("`{shown}` does not report stages (it refused --timing)"));
            return Ok(None);
        }
        Ok(()) => {}
    }
    let text = output.stderr.get(..MAX_STAGES).unwrap_or(output.stderr.as_slice());
    let text = String::from_utf8_lossy(text).trim_end().to_string();
    if text.is_empty() {
        note(&format!("`{shown}` printed no stage timing"));
        return Ok(None);
    }
    Ok(Some(text))
}

/// Writes the result to `path` through a temporary file in the same folder, so a reader never
/// sees half a file.
fn save(path: &str, result: &Value) -> Outcome {
    let text = serde_json::to_string_pretty(result).map_err(|e| Failure::Failed(e.to_string()))?;
    let target = std::path::Path::new(path);
    let name = target.file_name().ok_or_else(|| usage(format!("--save needs a file name, not {path}")))?;
    let mut temp_name = OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".tmp-{}", std::process::id()));
    let temp = target.with_file_name(temp_name);
    let written = std::fs::write(&temp, format!("{text}\n")).and_then(|()| std::fs::rename(&temp, target));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(Failure::Failed(format!("cannot write {path}: {e}")));
    }
    Ok(())
}

fn print_text(result: &Value, verdict: Option<&Verdict>) -> Outcome {
    let runs = result.get("runs").and_then(Value::as_u64).unwrap_or_default();
    let warmup = result.get("warmup").and_then(Value::as_u64).unwrap_or_default();
    out(&format!("{runs} runs after {warmup} warm-up runs each"))?;
    print_entry(result)?;
    for entry in result.get("compare").and_then(Value::as_array).into_iter().flatten() {
        print_entry(entry)?;
    }
    if let Some(v) = verdict {
        let direction = if v.change_percent >= 0.0 { "slower" } else { "faster" };
        out(&format!(
            "baseline: median {:.3} ms; this run {:.1}% {direction} (threshold {}%){}",
            v.baseline_ms,
            v.change_percent.abs(),
            v.threshold_percent,
            if v.slower { ": a slowdown" } else { "" }
        ))?;
    }
    if let Some(stages) = result.get("stages").and_then(Value::as_str) {
        out("stages of the last run:")?;
        out(stages)?;
    }
    Ok(())
}

fn print_entry(entry: &Value) -> Outcome {
    out(entry.get("command").and_then(Value::as_str).unwrap_or_default())?;
    if let Some(error) = entry.get("error").and_then(Value::as_str) {
        return out(&format!("  {error}"));
    }
    let line = |label: &str, stats: Option<&Value>, unit: &str| -> Option<String> {
        let s = stats.filter(|s| !s.is_null())?;
        let f = |k: &str| s.get(k).and_then(Value::as_f64).unwrap_or_default();
        Some(format!("  {label:<12} min {} {unit}  median {} {unit}  max {} {unit}  stddev {} {unit}", f("min"), f("median"), f("max"), f("stddev")))
    };
    if let Some(l) = line("wall time", entry.get("wall_ms"), "ms") {
        out(&l)?;
    }
    match line("peak memory", entry.get("peak_memory_kib"), "KiB") {
        Some(l) => out(&l),
        None => out("  peak memory  not measured on this system"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_before_the_subcommand_belong_to_bench() {
        let (head, command) = split(&args(&["--runs", "3", "--json", "--compare=fontc a.glyphs", "dump", "a.ttf", "--json"]));
        assert_eq!(head, args(&["--runs", "3", "--json", "--compare=fontc a.glyphs"]));
        assert_eq!(command, args(&["dump", "a.ttf", "--json"]), "options after the subcommand are the subcommand's");
        let (head, command) = split(&args(&["--warm", "0", "--", "--weird"]));
        assert_eq!((head, command), (args(&["--warm", "0"]), args(&["--weird"])));
        let (compare, rest) = take_compare(&args(&["--comp", "a b", "--runs=2", "--compare=c"])).unwrap();
        assert_eq!(compare, ["a b", "c"], "--compare may be repeated and abbreviated");
        assert_eq!(rest, ["--runs=2"]);
        assert_eq!(split_words("fontc 'My Font.glyphs' -o \"out dir/x.ttf\""), ["fontc", "My Font.glyphs", "-o", "out dir/x.ttf"]);
    }

    #[test]
    fn statistics_and_baseline_verdict() {
        let s = Stats::of(&[4.0, 1.0, 3.0, 2.0]).unwrap();
        assert_eq!((s.min, s.median, s.max), (1.0, 2.5, 4.0));
        assert!((s.stddev - 1.290_994).abs() < 1e-5);
        assert_eq!(Stats::of(&[5.0]).unwrap().stddev, 0.0);
        assert!(Stats::of(&[f64::NAN]).is_none());

        let base = parse_baseline("b.json", &json!({"command": "fontcraft-cli commands", "wall_ms": {"median": 10.0}})).unwrap();
        assert!(compare_with(&base, 11.5, 10.0).slower);
        assert!(!compare_with(&base, 10.9, 10.0).slower);
        assert!(!compare_with(&base, 5.0, 0.0).slower, "faster is never a slowdown");
        assert!(parse_baseline("b.json", &json!({"wall_ms": {"median": 0}})).is_err());
        assert!(parse_baseline("b.json", &json!([1, 2])).is_err());
    }

    #[test]
    fn peak_memory_line() {
        assert_eq!(vm_hwm("Name:\tx\nVmPeak:\t  9000 kB\nVmHWM:\t    1234 kB\n"), Some(1234));
        assert_eq!(vm_hwm("Name:\tx\n"), None);
    }

    #[test]
    fn bad_command_lines_are_usage_errors() {
        let cases: [&[&str]; 6] = [&["bench"], &["--runs", "0", "commands"], &["--threshold", "inf", "commands"], &["nosuch"], &[], &["--runs", "2"]];
        for bad in cases {
            let e = run(&args(bad)).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{bad:?}: {e:?}");
        }
    }
}

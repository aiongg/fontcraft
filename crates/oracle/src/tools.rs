//! Finding and running the reference compilers.
//!
//! The tools run as subprocesses with their output sent to log files in the work folder, a fixed
//! `SOURCE_DATE_EPOCH` and a time limit. A tool that is not installed is reported as missing,
//! never as a failed comparison.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Most bytes of a tool's log read back to explain a failure.
const MAX_LOG_TAIL: u64 = 64 * 1024;
/// Most characters of a tool's log quoted in a report.
const MAX_QUOTE: usize = 600;
/// Most files looked at in a tool's output folder.
const MAX_OUTPUT_FILES: usize = 1_000;
/// Most bytes of a designspace document read to find its first source.
const MAX_DESIGNSPACE: u64 = 16 << 20;

/// A reference compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Oracle {
    /// The Rust compiler: TrueType-flavoured output only.
    Fontc,
    /// The Python pipeline: TrueType and CFF.
    Fontmake,
}

impl Oracle {
    pub fn name(self) -> &'static str {
        match self {
            Oracle::Fontc => "fontc",
            Oracle::Fontmake => "fontmake",
        }
    }

    pub fn parse(s: &str) -> Option<Oracle> {
        match s {
            "fontc" => Some(Oracle::Fontc),
            "fontmake" => Some(Oracle::Fontmake),
            _ => None,
        }
    }

    /// The environment variable that names the tool's path, ahead of `PATH`.
    pub fn env_var(self) -> &'static str {
        match self {
            Oracle::Fontc => "FONTCRAFT_FONTC",
            Oracle::Fontmake => "FONTCRAFT_FONTMAKE",
        }
    }

    pub fn install_hint(self) -> &'static str {
        match self {
            Oracle::Fontc => "install it with `cargo install fontc`",
            Oracle::Fontmake => "install it with `uv tool install fontmake` or `pip install fontmake`",
        }
    }

    /// Where the tool is looked for after `PATH`, relative to the home folder.
    fn home_locations(self) -> &'static [&'static str] {
        match self {
            Oracle::Fontc => &[".cargo/bin/fontc"],
            Oracle::Fontmake => &[".local/share/uv/tools/fontmake/bin/fontmake", ".local/bin/fontmake"],
        }
    }

    /// Can the tool build this outline flavour?
    pub fn builds(self, flavour: Flavour) -> bool {
        !(self == Oracle::Fontc && flavour == Flavour::Cff)
    }
}

/// Outline flavour of a static build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Flavour {
    /// `glyf` outlines, `.ttf`.
    TrueType,
    /// `CFF ` outlines, `.otf`.
    Cff,
}

impl Flavour {
    pub fn parse(s: &str) -> Option<Flavour> {
        match s {
            "ttf" => Some(Flavour::TrueType),
            "otf" => Some(Flavour::Cff),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Flavour::TrueType => "ttf",
            Flavour::Cff => "otf",
        }
    }
}

/// The source kinds the harness hands to a reference compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Glyphs,
    GlyphsPackage,
    Designspace,
    Ufo,
}

impl SourceKind {
    pub fn of(path: &Path) -> Option<SourceKind> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "glyphs" => Some(SourceKind::Glyphs),
            "glyphspackage" => Some(SourceKind::GlyphsPackage),
            "designspace" => Some(SourceKind::Designspace),
            "ufo" => Some(SourceKind::Ufo),
            _ => None,
        }
    }
}

/// Finds `oracle`: `explicit` when given, else the tool's environment variable, `PATH`, then the
/// documented install locations under the home folder.
pub fn locate(oracle: Oracle, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return path.is_file().then(|| path.to_path_buf());
    }
    if let Some(path) = std::env::var_os(oracle.env_var()).map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    if let Some(dirs) = std::env::var_os("PATH")
        && let Some(found) = std::env::split_paths(&dirs).map(|d| d.join(oracle.name())).find(|p| p.is_file())
    {
        return Some(found);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    oracle.home_locations().iter().map(|rel| home.join(rel)).find(|p| p.is_file())
}

/// What FontCraft read from the source, for choosing among a tool's outputs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hint {
    pub family: String,
    /// The name of the master FontCraft compiles (its first).
    pub style: String,
}

/// A reference build to make.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub oracle: Oracle,
    pub tool: &'a Path,
    pub source: &'a Path,
    pub flavour: Flavour,
    /// An empty folder the tool may write into.
    pub work_dir: &'a Path,
    /// `SOURCE_DATE_EPOCH` for the tool.
    pub timestamp: i64,
    pub timeout: Duration,
    pub hint: &'a Hint,
}

/// A finished reference build.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Built {
    pub path: PathBuf,
    /// The command line that was run.
    pub command: String,
    /// How the build relates to FontCraft's (a variable font compared at its default location,
    /// one master's file out of several).
    pub notes: Vec<String>,
    /// Other files of masters with the same name as the one in `path`. The name cannot tell them
    /// apart, so [`crate::check`] compares with the closest of them all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<PathBuf>,
}

/// Builds `req.source` with the reference compiler.
pub fn build(req: &Request) -> Result<Built, String> {
    let kind = SourceKind::of(req.source).ok_or_else(|| format!("{} is not a source the reference compilers read", req.source.display()))?;
    if !req.oracle.builds(req.flavour) {
        return Err(format!("{} cannot build .{} fonts", req.oracle.name(), req.flavour.extension()));
    }
    let source = std::fs::canonicalize(req.source).map_err(|e| format!("{}: {e}", req.source.display()))?;
    match req.oracle {
        Oracle::Fontc => build_fontc(req, kind, &source),
        Oracle::Fontmake => build_fontmake(req, kind, &source),
    }
}

fn build_fontc(req: &Request, kind: SourceKind, source: &Path) -> Result<Built, String> {
    let mut notes = Vec::new();
    // fontc builds a variable font from a designspace with several sources. FontCraft compiles the
    // first master, which is the first source: give fontc that source's UFO, a static build.
    let input = match kind {
        SourceKind::Designspace => match first_source(source) {
            Some(ufo) => {
                notes
                    .push(format!("built from the designspace's first source, {}", ufo.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
                ufo
            }
            None => source.to_path_buf(),
        },
        _ => source.to_path_buf(),
    };
    let out = req.work_dir.join(format!("reference.{}", req.flavour.extension()));
    // Layout tables are compared from M8 and FontCraft compiles no features before then: leave
    // them out of the reference build too, so a feature file only fontc rejects does not stop the
    // comparison of the other tables.
    let mut cmd = Command::new(req.tool);
    cmd.arg("--no-production-names").arg("--skip-features").arg("-b").arg(req.work_dir.join("fontc-build")).arg("-o").arg(&out).arg(&input);
    run(req, cmd)?;
    if !out.is_file() {
        return Err(format!("fontc finished but wrote no {}", out.display()));
    }
    Ok(Built {
        path: out,
        command: format!("fontc --no-production-names --skip-features -o OUT {}", input.display()),
        notes,
        alternatives: Vec::new(),
    })
}

fn build_fontmake(req: &Request, kind: SourceKind, source: &Path) -> Result<Built, String> {
    let out_dir = req.work_dir.join("fontmake");
    let mut cmd = Command::new(req.tool);
    let input_flag = match kind {
        SourceKind::Glyphs | SourceKind::GlyphsPackage => "-g",
        SourceKind::Designspace => "-m",
        SourceKind::Ufo => "-u",
    };
    let glyphs = matches!(kind, SourceKind::Glyphs | SourceKind::GlyphsPackage);
    let ext = req.flavour.extension();
    let mut args: Vec<String> = vec![input_flag.into(), source.to_string_lossy().into_owned(), "-o".into()];
    if glyphs {
        // Master UFOs written to disk, so that masters with the same name get files of their
        // own: glyphsLib numbers the UFOs (`Family-Regular#1.ufo`) and fontmake names each font
        // after its UFO. Built in memory, they would all be written to one file.
        args.push("ufo".into());
    }
    args.push(ext.into());
    if req.flavour == Flavour::Cff {
        // Specialised charstrings without subroutines: what FontCraft's CFF writer is planned to do.
        args.extend(["--optimize-cff".into(), "1".into()]);
    }
    // Overlaps stay, as in fontc and in FontCraft before M3; glyph names stay the source's.
    args.extend(["--keep-overlaps".into(), "--no-production-names".into()]);
    if req.flavour == Flavour::TrueType {
        // fontc and FontCraft leave out an on-curve point exactly between two off-curve points.
        args.push("--drop-implied-oncurves".into());
    }
    if glyphs {
        for (flag, dir) in [("--master-dir", "fontmake-masters"), ("--instance-dir", "fontmake-instances")] {
            args.extend([flag.into(), req.work_dir.join(dir).to_string_lossy().into_owned()]);
        }
    }
    cmd.args(&args).arg("--output-dir").arg(&out_dir);
    run(req, cmd)?;
    let mut outputs: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .map_err(|e| format!("fontmake finished but its output folder cannot be read: {e}"))?
        .take(MAX_OUTPUT_FILES)
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some(ext))
        .collect();
    outputs.sort();
    let command = format!("fontmake {} --output-dir OUT", args.join(" "));
    let first = if kind == SourceKind::Designspace { first_source(source) } else { None };
    let (path, alternatives, notes) = choose_master_file(&outputs, ext, first.as_deref(), req.hint)?;
    Ok(Built { path, command, notes, alternatives })
}

/// The file of the master FontCraft compiles out of fontmake's output files (sorted), with the
/// files of other masters of the same name and a note when there are several files.
/// `first_source` is a designspace's first source, which names its master's file.
pub(crate) fn choose_master_file(
    outputs: &[PathBuf],
    ext: &str,
    first_source: Option<&Path>,
    hint: &Hint,
) -> Result<(PathBuf, Vec<PathBuf>, Vec<String>), String> {
    let many = match outputs {
        [] => return Err(format!("fontmake finished but wrote no .{ext} file")),
        [only] => return Ok((only.clone(), Vec::new(), Vec::new())),
        many => many,
    };
    // One file per master: take the one FontCraft compiles.
    let mut stems = Vec::new();
    if let Some(stem) = first_source.and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())) {
        stems.push(stem);
    }
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let style = squash(&hint.style);
    stems.push(format!("{}-{style}", squash(&hint.family)));
    let stem_of = |p: &PathBuf| p.file_stem().and_then(|s| s.to_str()).map(str::to_string).unwrap_or_default();
    let mut found = stems.iter().find_map(|stem| many.iter().find(|p| stem_of(p) == *stem));
    // The family name may be spelt differently (a default family name): the style decides
    // when only one file has it.
    if found.is_none() && !style.is_empty() {
        let suffix = format!("-{style}");
        let by_style: Vec<&PathBuf> = many.iter().filter(|p| stem_of(p).ends_with(&suffix)).collect();
        if let [one] = by_style.as_slice() {
            found = Some(*one);
        }
    }
    let names: Vec<String> = many.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
    let found = match (found, many.first()) {
        (Some(f), _) => f,
        // Without FontCraft's reading there is nothing to compare with: any file shows
        // that the build worked.
        (None, Some(first)) if hint.style.is_empty() => first,
        _ => {
            return Err(format!(
                "fontmake wrote {} masters ({}) and none is named after the first master `{}`",
                many.len(),
                names.join(", "),
                hint.style
            ));
        }
    };
    // glyphsLib's names for the UFOs of masters that share a name: `#1`, `#1#2`… added.
    let base = stem_of(found);
    let same_name = |p: &&PathBuf| {
        let stem = stem_of(p);
        stem != base
            && stem.strip_prefix(base.as_str()).is_some_and(|rest| rest.starts_with('#') && rest.chars().all(|c| c == '#' || c.is_ascii_digit()))
    };
    let alternatives: Vec<PathBuf> = many.iter().filter(same_name).cloned().collect();
    let notes = vec![format!("one of {} master files: {}", many.len(), found.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())];
    Ok((found.clone(), alternatives, notes))
}

/// Runs `cmd` in the work folder with its output in `tool.log`, within the time limit.
fn run(req: &Request, mut cmd: Command) -> Result<(), String> {
    let log_path = req.work_dir.join(format!("{}.log", req.oracle.name()));
    let log = File::create(&log_path).map_err(|e| format!("cannot create {}: {e}", log_path.display()))?;
    let log2 = log.try_clone().map_err(|e| format!("cannot share {}: {e}", log_path.display()))?;
    cmd.current_dir(req.work_dir)
        .env("SOURCE_DATE_EPOCH", req.timestamp.to_string())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2));
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", req.tool.display()))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > req.timeout => {
                // Best effort: the child may have exited between the two calls.
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} took longer than {} s and was stopped", req.oracle.name(), req.timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("waiting for {}: {e}", req.oracle.name())),
        }
    };
    if status.success() { Ok(()) } else { Err(format!("{} failed ({status}): {}", req.oracle.name(), log_tail(&log_path))) }
}

/// The last lines of a log, short enough to quote.
fn log_tail(path: &Path) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(MAX_LOG_TAIL))).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if f.take(MAX_LOG_TAIL).read_to_end(&mut buf).is_err() {
        return String::new();
    }
    let text = strip_ansi(&String::from_utf8_lossy(&buf));
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    // The last line that names an error says more than the progress lines around it.
    let is_error = |l: &&str| l.contains("Error") || l.contains("ERROR") || l.contains("error:") || l.contains("panicked");
    let tail = match lines.iter().rposition(is_error) {
        Some(i) => lines.get(i..(i + 2).min(lines.len())).unwrap_or_default().join(" | "),
        None => lines.get(lines.len().saturating_sub(3)..).unwrap_or_default().join(" | "),
    };
    clip(&tail, MAX_QUOTE)
}

/// `s` without terminal colour sequences (`ESC [ … letter`).
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `s` cut to at most `max` characters.
pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

/// The UFO of a designspace's first source that is not a layer source, resolved against the
/// document's folder.
pub fn first_source(designspace: &Path) -> Option<PathBuf> {
    use std::io::Read;
    let mut text = String::new();
    File::open(designspace).ok()?.take(MAX_DESIGNSPACE).read_to_string(&mut text).ok()?;
    let filename = first_source_filename(&text)?;
    Some(designspace.parent()?.join(filename))
}

/// The `filename` of the first `<source>` element without a `layer` attribute.
pub(crate) fn first_source_filename(xml: &str) -> Option<String> {
    let sources = xml.split_once("<sources")?.1;
    let mut rest = sources;
    while let Some((_, after)) = rest.split_once("<source") {
        // `<sources>` itself and other elements that start the same way are not sources.
        if !after.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            rest = after;
            continue;
        }
        let tag_end = after.find('>')?;
        let tag = after.get(..tag_end)?;
        rest = after.get(tag_end..)?;
        if attribute(tag, "layer").is_some() {
            continue;
        }
        return attribute(tag, "filename").map(|f| unescape(&f));
    }
    None
}

/// The value of `name="…"` (or `'…'`) in an element's attribute text.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before_ok = at == 0 || rest.get(..at).and_then(|b| b.chars().last()).is_some_and(char::is_whitespace);
        let after = rest.get(at + name.len()..)?.trim_start();
        if before_ok && let Some(value) = after.strip_prefix('=') {
            let value = value.trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let body = value.get(1..)?;
                return Some(body.get(..body.find(quote)?)?.to_string());
            }
        }
        rest = rest.get(at + name.len()..)?;
    }
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

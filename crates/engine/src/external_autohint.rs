//! The external TrueType autohinter: an autohinted TrueType export whose TTFAutohint binary path
//! parameter names the user's trusted ttfautohint program runs it on the finished font, with the
//! TTFAutohint options and control instructions passed through (M10 § 7, question 2 (a)).
//!
//! Rows: `TTFAutohint binary path`, `compile.ttfAutohintPostStep`, `export.ttfAutohintBinary`.
//! A document is untrusted, so its parameter only asks for the run: the program that runs is the
//! one in the user's setting [`SETTING`] (or the command line's `--ttfautohint`), and the
//! parameter must name that same file. Options are limited to ttfautohint's own option flags,
//! written back as long options, so no word reaches the program as a file name. Nothing runs
//! while automation roots are in force. Desktop only: on the web the parameter is reported and
//! the built-in autohinter's font is kept. The program reads the font on standard input and
//! writes the hinted font to standard output; it gets [`TIMEOUT`] and at most [`MAX_OUTPUT`]
//! bytes of output. Any refusal or failure keeps the built-in autohinter's font and reaches the
//! user as a warning.

use fontcraft_hinting::{HintSettings, TtAutohintOptions};
use fontcraft_model::{Font, Resolved};

/// The user setting that holds the trusted ttfautohint program.
pub const SETTING: &str = "export.ttfAutohintBinary";
/// How long the program may run.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Most bytes of hinted font read back.
pub const MAX_OUTPUT: usize = 256 << 20;
/// Most bytes of its error output kept for the message.
#[cfg(not(target_arch = "wasm32"))]
const MAX_MESSAGE: usize = 2_000;
/// Most words of options passed through.
const MAX_ARGS: usize = 256;

/// ttfautohint's options as its manual lists them: long name, short letter, whether it takes a
/// value. Left out: the options that name files (`--reference`, `--control-file`, whose file
/// FontCraft writes itself) and the ones that print instead of hinting (`--help`, `--version`).
const OPTIONS: [(&str, Option<char>, bool); 27] = [
    ("adjust-subglyphs", Some('p'), false),
    ("composites", Some('c'), false),
    ("debug", None, false),
    ("default-script", Some('D'), true),
    ("dehint", Some('d'), false),
    ("detailed-info", Some('I'), false),
    ("epoch", None, true),
    ("fallback-scaling", Some('S'), false),
    ("fallback-script", Some('f'), true),
    ("fallback-stem-width", Some('H'), true),
    ("family-suffix", Some('F'), true),
    ("hinting-limit", Some('G'), true),
    ("hinting-range-max", Some('r'), true),
    ("hinting-range-min", Some('l'), true),
    ("ignore-restrictions", Some('i'), false),
    ("increase-x-height", Some('x'), true),
    ("no-info", Some('n'), false),
    ("pre-hinting", None, false),
    ("reference-index", Some('Z'), true),
    ("stem-width-mode", Some('a'), true),
    ("strong-stem-width", Some('w'), true),
    ("symbol", Some('s'), false),
    ("ttfa-info", Some('T'), false),
    ("ttfa-table", Some('t'), false),
    ("verbose", Some('v'), false),
    ("windows-compatibility", Some('W'), false),
    ("x-height-snapping-exceptions", Some('X'), true),
];

/// What an external run needs: the trusted program, and the options as long-option words.
#[derive(Clone, Debug, PartialEq)]
pub struct External {
    pub binary: String,
    pub options: Vec<String>,
    pub control: Option<String>,
}

/// The trusted ttfautohint program set in the user's settings, if any.
pub fn trusted(s: &crate::Session) -> Option<String> {
    let value = s.settings().get(SETTING).ok()?;
    value.as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

impl External {
    /// The external run an autohinted TrueType build of `resolved` asks for: `None` when it asks
    /// for none, an error naming why when it asks but may not run. `trusted` is the program the
    /// user approved ([`trusted`]).
    pub fn wanted(font: &Font, resolved: &Resolved, autohint: bool, trusted: Option<&str>) -> Option<Result<Self, String>> {
        let settings = match resolved.instance.filter(|i| font.instances.get(*i).is_some()) {
            Some(i) => HintSettings::for_instance(font, i, &resolved.master),
            None => HintSettings::for_master(font, &resolved.master),
        }
        .ok()?;
        if !settings.autohint.unwrap_or(autohint) || !settings.export_tt_hints {
            return None;
        }
        let asked = settings.tt_autohint_binary.map(|b| b.trim().to_string()).filter(|b| !b.is_empty())?;
        Some(Self::checked(&asked, settings.tt_autohint_options.as_deref(), settings.tt_autohint_control, trusted))
    }

    fn checked(asked: &str, options: Option<&str>, control: Option<String>, trusted: Option<&str>) -> Result<Self, String> {
        if crate::file_access::current().is_some() {
            return Err("no external program runs while automation roots are in force".into());
        }
        let Some(trusted) = trusted.map(str::trim).filter(|t| !t.is_empty()) else {
            return Err(format!(
                "it names `{asked}`, but no trusted autohinter is set; set {SETTING} in the settings (or give --ttfautohint on the command line) to that program"
            ));
        };
        if !same_file(asked, trusted) {
            return Err(format!("it names `{asked}`, which is not the trusted autohinter set in {SETTING}"));
        }
        let options = match options {
            Some(text) => long_options(text)?,
            None => Vec::new(),
        };
        let control = control.filter(|c| !c.trim().is_empty());
        Ok(Self { binary: trusted.to_string(), options, control })
    }

    /// The font `bytes` hinted by the program, or why that failed.
    #[cfg(target_arch = "wasm32")]
    pub fn run(&self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let _ = bytes;
        Err("an external autohinter cannot run here".into())
    }

    /// The font `bytes` hinted by the program, or why that failed.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(&self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};

        let path = std::path::Path::new(&self.binary);
        if !path.is_file() {
            return Err(format!("`{}` is not a program file", self.binary));
        }
        let scratch = match &self.control {
            Some(text) => Some(Scratch::with_control(text).map_err(|e| format!("the control instructions cannot be written for it ({e})"))?),
            None => None,
        };
        let mut command = Command::new(path);
        command.args(&self.options);
        if let Some(scratch) = &scratch {
            command.arg(format!("--control-file={}", scratch.file.display()));
        }
        let result = (|| {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("`{}` cannot be started ({e})", self.binary))?;
            let mut stdin = child.stdin.take().ok_or("its input cannot be opened")?;
            let input = bytes.to_vec();
            let writer = std::thread::spawn(move || {
                // A program that stops reading early ends the write with an error, which its
                // exit status reports better.
                let _ = stdin.write_all(&input);
            });
            let stdout = child.stdout.take().ok_or("its output cannot be opened")?;
            let reader = std::thread::spawn(move || {
                let mut out = Vec::new();
                let read = stdout.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut out);
                read.map(|_| out)
            });
            let stderr = child.stderr.take();
            let errors = std::thread::spawn(move || {
                let mut text = Vec::new();
                if let Some(mut e) = stderr {
                    let _ = e.by_ref().take(MAX_MESSAGE as u64).read_to_end(&mut text);
                    // Keep reading what is left, so a talkative program never blocks on a full pipe.
                    let _ = std::io::copy(&mut e, &mut std::io::sink());
                }
                String::from_utf8_lossy(&text).trim().to_string()
            });
            let started = std::time::Instant::now();
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) if started.elapsed() > TIMEOUT => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("`{}` did not finish within {} s and was stopped", self.binary, TIMEOUT.as_secs()));
                    }
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(e) => return Err(format!("`{}` cannot be waited for ({e})", self.binary)),
                }
            };
            let _ = writer.join();
            let out = reader.join().map_err(|_| "its output cannot be read".to_string())?.map_err(|e| format!("its output cannot be read ({e})"))?;
            let message = errors.join().unwrap_or_default();
            if !status.success() {
                return Err(format!(
                    "`{}` failed ({status}){}",
                    self.binary,
                    if message.is_empty() { String::new() } else { format!(": {message}") }
                ));
            }
            if out.len() > MAX_OUTPUT {
                return Err(format!("`{}` wrote more than {MAX_OUTPUT} bytes", self.binary));
            }
            skrifa_check(&out).map_err(|e| format!("`{}` wrote no usable font ({e})", self.binary))?;
            Ok(out)
        })();
        drop(scratch);
        result
    }
}

/// Do `a` and `b` name the same file: the same path, or paths that resolve to one file.
fn same_file(a: &str, b: &str) -> bool {
    let (pa, pb) = (std::path::Path::new(a), std::path::Path::new(b));
    if pa == pb {
        return true;
    }
    if let (Ok(ca), Ok(cb)) = (pa.canonicalize(), pb.canonicalize()) {
        return ca == cb;
    }
    false
}

/// The TTFAutohint options as `--name` and `--name=value` words. A word that is not one of
/// ttfautohint's options in [`OPTIONS`] (a file name, an option that names a file) is refused,
/// and the values are checked as the built-in autohinter checks them.
fn long_options(text: &str) -> Result<Vec<String>, String> {
    let bad = |m: String| format!("the TTFAutohint options cannot be passed to it: {m}");
    TtAutohintOptions::parse(text).map_err(|e| bad(e.to_string()))?;
    let mut out = Vec::new();
    let mut words = text.split_whitespace();
    while let Some(word) = words.next() {
        if out.len() >= MAX_ARGS {
            return Err(bad(format!("more than {MAX_ARGS} options")));
        }
        let (found, inline) = match word.strip_prefix("--") {
            Some(long) => {
                let (name, value) = match long.split_once('=') {
                    Some((n, v)) => (n, Some(v)),
                    None => (long, None),
                };
                (OPTIONS.iter().find(|(l, _, _)| *l == name), value)
            }
            None => {
                let mut chars = word.strip_prefix('-').unwrap_or_default().chars();
                let letter = chars.next();
                let rest = chars.as_str();
                let found = letter.and_then(|c| OPTIONS.iter().find(|(_, s, _)| *s == Some(c)));
                (found, (!rest.is_empty()).then(|| rest.trim_start_matches('=')))
            }
        };
        let Some(&(name, _, takes_value)) = found else {
            return Err(bad(format!("`{word}` is not an option the external autohinter may be given")));
        };
        let name = if name == "pre-hinting" { "adjust-subglyphs" } else { name };
        if takes_value {
            let value = match inline {
                Some(v) => v,
                None => words.next().ok_or_else(|| bad(format!("`{name}` needs a value")))?,
            };
            out.push(format!("--{name}={value}"));
        } else if inline.is_some() {
            return Err(bad(format!("`{name}` takes no value")));
        } else {
            out.push(format!("--{name}"));
        }
    }
    Ok(out)
}

/// A private folder holding the control instructions file, removed with it when dropped.
#[cfg(not(target_arch = "wasm32"))]
struct Scratch {
    dir: std::path::PathBuf,
    file: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl Scratch {
    /// A new folder only this user may enter (an existing path of the same name is never reused),
    /// with `text` in a file created inside it.
    fn with_control(text: &str) -> std::io::Result<Self> {
        use std::io::Write;
        if fontcraft_format::output::dry_run() {
            return Err(std::io::Error::other("a dry run writes no files"));
        }
        let mut last = None;
        for _ in 0..8 {
            let dir = std::env::temp_dir().join(format!("fontcraft-autohint-{}-{:x}", std::process::id(), unique()));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            match builder.create(&dir) {
                Ok(()) => {
                    let scratch = Scratch { file: dir.join("control.txt"), dir };
                    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&scratch.file)?;
                    f.write_all(text.as_bytes())?;
                    return Ok(scratch);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("no free folder name")))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// A font the program wrote opens as one.
#[cfg(not(target_arch = "wasm32"))]
fn skrifa_check(bytes: &[u8]) -> Result<(), String> {
    use skrifa::raw::TableProvider;
    let font = skrifa::FontRef::new(bytes).map_err(|e| e.to_string())?;
    font.maxp().map_err(|e| e.to_string())?;
    Ok(())
}

/// A folder name nobody can guess ahead: a counter mixed with the clock.
#[cfg(not(target_arch = "wasm32"))]
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let clock =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs().rotate_left(32) ^ u64::from(d.subsec_nanos()));
    clock ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_become_long_options_and_files_are_refused() {
        assert!(long_options("--dehint -cs").is_err(), "bundled short options are not read");
        assert_eq!(
            long_options("-W --hinting-limit 120 -x 0 -p").unwrap(),
            vec!["--windows-compatibility", "--hinting-limit=120", "--increase-x-height=0", "--adjust-subglyphs"]
        );
        for bad in ["in.ttf out.ttf", "--dehint /tmp/out.ttf", "--reference=a.ttf", "-m ctl.txt", "--control-file=x", "--help"] {
            assert!(long_options(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn nothing_runs_while_automation_roots_are_in_force() {
        let trusted = Some("/usr/bin/ttfautohint");
        assert!(External::checked("/usr/bin/ttfautohint", None, None, trusted).is_ok());
        let roots = crate::file_access::AutomationRoots::new(Some(&std::env::temp_dir()), None).unwrap().unwrap();
        let refused = crate::file_access::confine(Some(&roots), || External::checked("/usr/bin/ttfautohint", None, None, trusted));
        assert!(refused.unwrap_err().contains("automation roots"));
    }
}

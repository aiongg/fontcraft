//! The external TrueType autohinter: when the TTFAutohint binary path parameter names a
//! ttfautohint program, an autohinted TrueType export runs it on the finished font, with the
//! TTFAutohint options and control instructions passed through (M10 § 7, question 2 (a)).
//!
//! Rows: `TTFAutohint binary path`, `compile.ttfAutohintPostStep`. Desktop only: on the web the
//! parameter is reported and the built-in autohinter's font is kept. The program reads the font
//! on standard input and writes the hinted font to standard output; it gets
//! [`TIMEOUT`] and at most [`MAX_OUTPUT`] bytes of output. Any failure keeps the built-in
//! autohinter's font and reaches the user as a warning.

use fontcraft_hinting::HintSettings;
use fontcraft_model::{Font, Resolved};

/// How long the program may run.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Most bytes of hinted font read back.
pub const MAX_OUTPUT: usize = 256 << 20;
/// Most bytes of its error output kept for the message.
#[cfg(not(target_arch = "wasm32"))]
const MAX_MESSAGE: usize = 2_000;
/// Most words of options passed through.
const MAX_ARGS: usize = 256;

/// What an external run needs, read from the build's hint settings.
#[derive(Clone, Debug, PartialEq)]
pub struct External {
    pub binary: String,
    pub options: Vec<String>,
    pub control: Option<String>,
}

impl External {
    /// The external run an autohinted TrueType build of `resolved` asks for, if any.
    pub fn wanted(font: &Font, resolved: &Resolved, autohint: bool) -> Option<Self> {
        let settings = match resolved.instance.filter(|i| font.instances.get(*i).is_some()) {
            Some(i) => HintSettings::for_instance(font, i, &resolved.master),
            None => HintSettings::for_master(font, &resolved.master),
        }
        .ok()?;
        if !settings.autohint.unwrap_or(autohint) || !settings.export_tt_hints {
            return None;
        }
        let binary = settings.tt_autohint_binary.filter(|b| !b.trim().is_empty())?;
        let options = settings.tt_autohint_options.map(|o| o.split_whitespace().take(MAX_ARGS).map(str::to_string).collect()).unwrap_or_default();
        let control = settings.tt_autohint_control.filter(|c| !c.trim().is_empty());
        Some(Self { binary: binary.trim().to_string(), options, control })
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
        let control_file = match &self.control {
            Some(text) => {
                let file = std::env::temp_dir().join(format!("fontcraft-control-{}-{}.txt", std::process::id(), unique()));
                std::fs::write(&file, text).map_err(|e| format!("the control instructions cannot be written for it ({e})"))?;
                Some(file)
            }
            None => None,
        };
        let mut command = Command::new(path);
        command.args(&self.options);
        if let Some(file) = &control_file {
            command.arg(format!("--control-file={}", file.display()));
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
                if let Some(e) = stderr {
                    let _ = e.take(MAX_MESSAGE as u64).read_to_end(&mut text);
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
        if let Some(file) = control_file {
            let _ = std::fs::remove_file(file);
        }
        result
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

#[cfg(not(target_arch = "wasm32"))]
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

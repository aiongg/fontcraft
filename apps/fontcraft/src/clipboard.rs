//! The PDF a drawing app puts on the system clipboard, for Edit View's paste
//! ([`ClipboardFlavours`]). egui carries only text, so the PDF is read through the platform's
//! clipboard tool: `wl-paste` under Wayland, else `xclip`; `osascript` on macOS; PowerShell on
//! Windows. Without the tool, or with no PDF on the clipboard, there is nothing to paste.
//!
//! Each tool runs with its output capped (a PDF larger than the paste limit is refused while it
//! is read) and is stopped when it takes longer than [`TOOL_TIMEOUT`].

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use fontcraft_ui_egui::shell::clipboard::ClipboardFlavours;

/// How long one clipboard tool may take.
const TOOL_TIMEOUT: Duration = Duration::from_secs(3);
/// Most bytes of a type list read.
const MAX_TYPE_LIST: usize = 64 << 10;
/// The clipboard type a PDF goes under on Linux.
const PDF_MIME: &str = "application/pdf";
/// The clipboard formats a PDF goes under on Windows, in the order they are tried.
const WINDOWS_FORMATS: &str = "'Portable Document Format','PDF','application/pdf'";

/// The system clipboard's PDF.
pub struct SystemClipboard;

impl ClipboardFlavours for SystemClipboard {
    fn pdf(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, String> {
        if cfg!(target_os = "macos") {
            mac_pdf(max_bytes)
        } else if cfg!(target_os = "windows") {
            windows_pdf(max_bytes)
        } else {
            unix_pdf(max_bytes)
        }
    }
}

/// Wayland's `wl-paste`, else X11's `xclip`, each asked first whether a PDF is there.
fn unix_pdf(max: usize) -> Result<Option<Vec<u8>>, String> {
    let offers_pdf = |list: &[u8]| String::from_utf8_lossy(list).lines().any(|l| l.trim() == PDF_MIME);
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && let Some(types) = run("wl-paste", &["--list-types"], MAX_TYPE_LIST)?
    {
        return if offers_pdf(&types) { run("wl-paste", &["--no-newline", "--type", PDF_MIME], max) } else { Ok(None) };
    }
    match run("xclip", &["-selection", "clipboard", "-target", "TARGETS", "-out"], MAX_TYPE_LIST)? {
        Some(types) if offers_pdf(&types) => run("xclip", &["-selection", "clipboard", "-target", PDF_MIME, "-out"], max),
        _ => Ok(None),
    }
}

/// AppleScript gives the clipboard's PDF as `«data PDF 2550…»`, the bytes in hexadecimal.
fn mac_pdf(max: usize) -> Result<Option<Vec<u8>>, String> {
    let Some(out) = run("osascript", &["-e", "the clipboard as «class PDF »"], hex_limit(max))? else { return Ok(None) };
    let text = String::from_utf8_lossy(&out);
    let Some(start) = text.find("«data PDF ") else { return Ok(None) };
    let hex = text.get(start + "«data PDF ".len()..).unwrap_or_default();
    let hex = hex.split('»').next().unwrap_or_default();
    from_hex(hex, max).map(Some)
}

/// PowerShell reads the clipboard through .NET and writes the bytes in hexadecimal.
fn windows_pdf(max: usize) -> Result<Option<Vec<u8>>, String> {
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         foreach ($f in @({WINDOWS_FORMATS})) {{ \
           $d = [System.Windows.Forms.Clipboard]::GetData($f); \
           if ($d -is [byte[]]) {{ $b = $d }} \
           elseif ($d -is [System.IO.Stream]) {{ $m = New-Object System.IO.MemoryStream; $d.CopyTo($m); $b = $m.ToArray() }} \
           else {{ continue }}; \
           [Console]::Out.Write([System.BitConverter]::ToString($b).Replace('-', '')); break }}"
    );
    let args = ["-NoProfile", "-NonInteractive", "-STA", "-WindowStyle", "Hidden", "-Command", script.as_str()];
    let Some(out) = run("powershell", &args, hex_limit(max))? else { return Ok(None) };
    let hex = String::from_utf8_lossy(&out);
    let hex = hex.trim();
    if hex.is_empty() { Ok(None) } else { from_hex(hex, max).map(Some) }
}

/// Characters of hexadecimal output that hold up to `max` bytes, with room for the wrapping.
fn hex_limit(max: usize) -> usize {
    max.saturating_mul(2).saturating_add(256)
}

fn from_hex(hex: &str, max: usize) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = hex.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if digits.len() / 2 > max {
        return Err(too_large(max));
    }
    let value = |d: u8| char::from(d).to_digit(16).and_then(|v| u8::try_from(v).ok());
    digits
        .chunks(2)
        .map(|pair| match pair {
            [hi, lo] => value(*hi).zip(value(*lo)).map(|(hi, lo)| (hi << 4) | lo),
            _ => None,
        })
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| "the clipboard tool gave data that is not hexadecimal".into())
}

fn too_large(max: usize) -> String {
    format!("it is larger than {} MiB", max >> 20)
}

/// The standard output of `program` with `args`, refused past `max` bytes; `None` when the
/// program cannot be started or reports a failure (no such type on the clipboard).
fn run(program: &str, args: &[&str], max: usize) -> Result<Option<Vec<u8>>, String> {
    let Ok(mut child) = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return Ok(None);
    };
    let Some(stdout) = child.stdout.take() else {
        stop(child);
        return Ok(None);
    };
    let (tx, rx) = mpsc::channel();
    // The read runs on its own thread so that a tool that never answers can be stopped.
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let read = stdout.take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut out);
        let _ = tx.send(read.map(|_| out));
    });
    let out = match rx.recv_timeout(TOOL_TIMEOUT) {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            stop(child);
            return Err(format!("the clipboard cannot be read: {e}"));
        }
        Err(_) => {
            stop(child);
            return Err("the clipboard did not answer in time".into());
        }
    };
    if out.len() > max {
        stop(child);
        return Err(too_large(max));
    }
    match child.wait() {
        Ok(status) if status.success() => Ok(Some(out)),
        _ => Ok(None),
    }
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

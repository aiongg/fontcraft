//! The PDF a drawing app puts on the system clipboard, for Edit View's paste
//! ([`ClipboardFlavours`]). egui carries only text, so the PDF is read through the platform's
//! clipboard tool: `wl-paste` under Wayland, else `xclip`; `osascript` on macOS; PowerShell on
//! Windows. Without the tool, or with no PDF on the clipboard, there is nothing to paste.
//!
//! The read runs on a thread of its own, so a slow tool never holds up the window, which takes
//! the result in a later frame. Where the tool can list the clipboard's types quickly, it is asked
//! first whether a PDF is there. Each tool runs with its output capped (a PDF larger than the
//! paste limit is refused while it is read). A type list may take [`TYPES_TIMEOUT`] and the PDF
//! [`DATA_TIMEOUT`]; a tool that takes longer is stopped, on Unix with every process it started.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use fontcraft_ui_egui::shell::clipboard::{ClipboardFlavours, PdfRead};

/// How long a clipboard tool may take to list the clipboard's types.
const TYPES_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a clipboard tool may take to give the PDF: a large one comes slowly as hexadecimal.
const DATA_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a tool that has closed its output, or was stopped, is waited for to exit.
const EXIT_WAIT: Duration = Duration::from_secs(1);
/// Most bytes of a type list read.
const MAX_TYPE_LIST: usize = 64 << 10;
/// The clipboard type a PDF goes under on Linux.
const PDF_MIME: &str = "application/pdf";
/// The clipboard formats a PDF goes under on Windows, in the order they are tried.
const WINDOWS_FORMATS: &str = "'Portable Document Format','PDF','application/pdf'";
/// What AppleScript writes before the clipboard's PDF in hexadecimal.
const MAC_PDF_DATA: &[u8] = "«data PDF ".as_bytes();

/// The system clipboard's PDF, read on a thread of its own.
#[derive(Default)]
pub struct SystemClipboard {
    /// Where the running read sends its result.
    running: Option<mpsc::Receiver<PdfRead>>,
}

impl ClipboardFlavours for SystemClipboard {
    fn request_pdf(&mut self, max_bytes: usize, ctx: &egui::Context) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        // A thread that cannot start drops `tx`, and `take_pdf` reports that.
        let _ = std::thread::Builder::new().name("clipboard-pdf".into()).spawn(move || {
            let _ = tx.send(read_pdf(max_bytes));
            ctx.request_repaint();
        });
        self.running = Some(rx);
    }

    fn take_pdf(&mut self) -> Option<PdfRead> {
        let read = match self.running.as_ref()?.try_recv() {
            Ok(read) => read,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("the clipboard reader stopped".into()),
        };
        self.running = None;
        Some(read)
    }
}

fn read_pdf(max: usize) -> PdfRead {
    if cfg!(target_os = "macos") {
        mac_pdf(max)
    } else if cfg!(target_os = "windows") {
        windows_pdf(max)
    } else {
        unix_pdf(max)
    }
}

/// Wayland's `wl-paste`, else X11's `xclip`, each asked first whether a PDF is there.
fn unix_pdf(max: usize) -> PdfRead {
    let offers_pdf = |list: &[u8]| String::from_utf8_lossy(list).lines().any(|l| l.trim() == PDF_MIME);
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && let Some(types) = run("wl-paste", &["--list-types"], MAX_TYPE_LIST, TYPES_TIMEOUT)?
    {
        return if offers_pdf(&types) { run("wl-paste", &["--no-newline", "--type", PDF_MIME], max, DATA_TIMEOUT) } else { Ok(None) };
    }
    match run("xclip", &["-selection", "clipboard", "-target", "TARGETS", "-out"], MAX_TYPE_LIST, TYPES_TIMEOUT)? {
        Some(types) if offers_pdf(&types) => run("xclip", &["-selection", "clipboard", "-target", PDF_MIME, "-out"], max, DATA_TIMEOUT),
        _ => Ok(None),
    }
}

/// AppleScript lists the clipboard's types (`clipboard info`, as `«class PDF », 2550, …`), then
/// gives the PDF as `«data PDF 2550…»`, the bytes in hexadecimal.
fn mac_pdf(max: usize) -> PdfRead {
    let Some(info) = run("osascript", &["-e", "clipboard info"], MAX_TYPE_LIST, TYPES_TIMEOUT)? else { return Ok(None) };
    if !info.windows(3).any(|w| w == b"PDF") {
        return Ok(None);
    }
    let Some(out) = run("osascript", &["-e", "the clipboard as «class PDF »"], hex_limit(max), DATA_TIMEOUT)? else { return Ok(None) };
    let Some(at) = out.windows(MAC_PDF_DATA.len()).position(|w| w == MAC_PDF_DATA) else { return Ok(None) };
    // The digits end at the closing `»`.
    from_hex(out, at.saturating_add(MAC_PDF_DATA.len()), true, max).map(Some)
}

/// PowerShell reads the clipboard through .NET and writes the bytes in hexadecimal. Starting it
/// is the slow part, so the formats are looked for in the same run that reads them.
fn windows_pdf(max: usize) -> PdfRead {
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
    let Some(out) = run("powershell", &args, hex_limit(max), DATA_TIMEOUT)? else { return Ok(None) };
    let pdf = from_hex(out, 0, false, max)?;
    if pdf.is_empty() { Ok(None) } else { Ok(Some(pdf)) }
}

/// Characters of hexadecimal output that hold up to `max` bytes, with room for the wrapping.
fn hex_limit(max: usize) -> usize {
    max.saturating_mul(2).saturating_add(256)
}

/// The bytes written in hexadecimal in `out` from `start` on, whitespace skipped, decoded into
/// `out` itself so that a large PDF is held once. The digits end at the first other byte when
/// `stop_at_other`; otherwise such a byte is an error. More than `max` bytes are refused.
fn from_hex(mut out: Vec<u8>, start: usize, stop_at_other: bool, max: usize) -> Result<Vec<u8>, String> {
    let digit = |b: u8| char::from(b).to_digit(16).and_then(|v| u8::try_from(v).ok());
    let mut written = 0usize;
    let mut high = None;
    let mut read = start;
    // Each byte written takes two digits read, so `written` stays behind `read`.
    while let Some(&b) = out.get(read) {
        read = read.saturating_add(1);
        if b.is_ascii_whitespace() {
            continue;
        }
        let Some(value) = digit(b) else {
            if stop_at_other {
                break;
            }
            return Err(NOT_HEX.into());
        };
        match high.take() {
            None => high = Some(value),
            Some(h) => {
                if written >= max {
                    return Err(too_large(max));
                }
                let Some(slot) = out.get_mut(written) else { return Err(NOT_HEX.into()) };
                *slot = (h << 4) | value;
                written = written.saturating_add(1);
            }
        }
    }
    if high.is_some() {
        return Err(NOT_HEX.into());
    }
    out.truncate(written);
    Ok(out)
}

const NOT_HEX: &str = "the clipboard tool gave data that is not hexadecimal";

fn too_large(max: usize) -> String {
    format!("it is larger than {} MiB", max >> 20)
}

/// The standard output of `program` with `args`, refused past `max` bytes or after `timeout`;
/// `None` when the program cannot be started or reports a failure (no such type on the
/// clipboard).
fn run(program: &str, args: &[&str], max: usize, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // A process group of its own, so that stopping the tool stops what it started too.
        command.process_group(0);
    }
    let Ok(mut child) = command.spawn() else { return Ok(None) };
    let deadline = Instant::now() + timeout;
    let Some(stdout) = child.stdout.take() else {
        stop(child);
        return Ok(None);
    };
    let (tx, rx) = mpsc::channel();
    // The read runs on its own thread so that a tool that never answers can be stopped.
    let reader = std::thread::Builder::new().name("clipboard-tool".into()).spawn(move || {
        let mut out = Vec::new();
        let read = stdout.take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut out);
        let _ = tx.send(read.map(|_| out));
    });
    if let Err(e) = reader {
        stop(child);
        return Err(format!("the clipboard cannot be read: {e}"));
    }
    let out = match rx.recv_timeout(timeout) {
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
    match wait_until(&mut child, deadline.max(Instant::now() + EXIT_WAIT)) {
        Some(status) if status.success() => Ok(Some(out)),
        Some(_) => Ok(None),
        None => {
            stop(child);
            Err("the clipboard tool did not finish".into())
        }
    }
}

/// The exit status of `child`, or `None` when it is still running at `deadline`.
fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => return None,
        }
    }
}

/// Stops `child` and, on Unix, its process group. A process that does not exit within
/// [`EXIT_WAIT`] is left to the system.
fn stop(mut child: Child) {
    // The group goes before the child is reaped, while its id cannot have been reused.
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        let kill = Command::new("kill").args(["-s", "KILL", "--", &group]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        if let Ok(mut kill) = kill {
            let _ = wait_until(&mut kill, Instant::now() + EXIT_WAIT);
        }
    }
    let _ = child.kill();
    let _ = wait_until(&mut child, Instant::now() + EXIT_WAIT);
}

#[cfg(test)]
mod tests {
    use super::from_hex;

    #[test]
    fn hexadecimal_is_decoded_in_place_up_to_its_end() {
        let out = "x «data PDF 25 50444\n6»".as_bytes().to_vec();
        let at = "x «data PDF ".len();
        assert_eq!(from_hex(out, at, true, 16).unwrap(), b"%PDF");
        assert_eq!(from_hex(b"255044 46\r\n".to_vec(), 0, false, 16).unwrap(), b"%PDF");
        assert!(from_hex(b"2550 x".to_vec(), 0, false, 16).is_err(), "not hexadecimal");
        assert!(from_hex(b"255".to_vec(), 0, false, 16).is_err(), "an odd digit");
        assert!(from_hex(b"25504446".to_vec(), 0, false, 3).is_err(), "over the limit");
    }
}

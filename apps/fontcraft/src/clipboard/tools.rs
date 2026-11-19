//! Linux and BSD: the clipboard's flavours through the desktop's clipboard tools, `wl-paste` under
//! Wayland, else `xclip`. Without the tool, or with nothing of the type on the clipboard, there is
//! nothing to paste.
//!
//! Each tool is asked first whether the type is there. Each runs with its output capped (data
//! larger than the paste limit is refused while it is read). A type list may take
//! [`TYPES_TIMEOUT`] and the data [`DATA_TIMEOUT`]; a tool that takes longer is stopped with every
//! process it started.
//!
//! The tools stay because no crate covers what they do: `wl-paste` falls back to a focused surface
//! where the compositor has no data-control protocol, and X11 sends large data in increments.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fontcraft_ui_egui::shell::clipboard::PdfRead;

use super::too_large;

/// How long a clipboard tool may take to list the clipboard's types.
const TYPES_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a clipboard tool may take to give the data.
const DATA_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a tool that has closed its output, or was stopped, is waited for to exit.
const EXIT_WAIT: Duration = Duration::from_secs(1);
/// Most bytes of a type list read.
const MAX_TYPE_LIST: usize = 64 << 10;

/// The clipboard's data of MIME type `mime`, at most `max` bytes. The tools name types by MIME
/// type, so any type can be asked for.
pub(super) fn read_flavour(mime: &str, max: usize) -> PdfRead {
    let offers = |list: &[u8]| String::from_utf8_lossy(list).lines().any(|l| l.trim() == mime);
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && let Some(types) = run("wl-paste", &["--list-types"], MAX_TYPE_LIST, TYPES_TIMEOUT)?
    {
        return if offers(&types) { run("wl-paste", &["--no-newline", "--type", mime], max, DATA_TIMEOUT) } else { Ok(None) };
    }
    match run("xclip", &["-selection", "clipboard", "-target", "TARGETS", "-out"], MAX_TYPE_LIST, TYPES_TIMEOUT)? {
        Some(types) if offers(&types) => run("xclip", &["-selection", "clipboard", "-target", mime, "-out"], max, DATA_TIMEOUT),
        _ => Ok(None),
    }
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
    use super::{TYPES_TIMEOUT, run};

    #[test]
    fn a_missing_tool_is_nothing_to_paste_and_output_past_the_cap_is_refused() {
        assert_eq!(run("fontcraft-no-such-clipboard-tool", &[], 16, TYPES_TIMEOUT), Ok(None), "no PDF without the tool");
        // `yes` writes without end: the cap stops the read, and the tool with it.
        assert!(run("yes", &[], 1 << 10, TYPES_TIMEOUT).unwrap_err().contains("larger than"), "over the cap");
    }
}

//! The PDF a drawing app puts on the system clipboard, for a paste on the canvas
//! ([`ClipboardFlavours`]). The system clipboard's text reader carries only text, so the PDF is read per platform:
//! through the system clipboard's own interface on Windows (module `windows`, clipboard-win) and macOS (module
//! `macos`, the general pasteboard through objc2-app-kit), and through the desktop's clipboard tools on Linux and BSD
//! (module `tools`). With no PDF on the clipboard there is nothing to paste.
//!
//! The read runs on a thread of its own, so a slow clipboard owner never holds up the window, which takes the result
//! in a later frame. A PDF larger than the paste limit is refused.
//!
//! Each platform reader answers for a MIME type ([`read_flavour`]), the way a clipboard backend of the craft platform
//! service is asked for one, so a reader can move behind that service when it carries custom types.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(windows, target_os = "macos")))]
mod tools;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(not(any(windows, target_os = "macos")))]
use self::tools as platform;
#[cfg(windows)]
use self::windows as platform;

use std::sync::mpsc;

use fontcraft_ui::app::clipboard::{ClipboardFlavours, PdfRead, Wake};

/// The MIME type of a PDF.
const PDF_MIME: &str = "application/pdf";

/// The system clipboard's PDF, read on a thread of its own.
#[derive(Default)]
pub struct SystemClipboard {
    /// Where the running read sends its result.
    running: Option<mpsc::Receiver<PdfRead>>,
}

impl ClipboardFlavours for SystemClipboard {
    fn request_pdf(&mut self, max_bytes: usize, wake: Wake) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        // A thread that cannot start drops `tx`, and `take_pdf` reports that.
        let _ = std::thread::Builder::new().name("clipboard-pdf".into()).spawn(move || {
            let _ = tx.send(read_flavour(PDF_MIME, max_bytes));
            wake.wake();
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

/// The clipboard's data of MIME type `mime`, at most `max` bytes; `None` when the clipboard
/// holds none.
fn read_flavour(mime: &str, max: usize) -> PdfRead {
    platform::read_flavour(mime, max)
}

/// The data of the first of `names` the clipboard holds, for a system clipboard that names its
/// formats (Windows) or types (macOS) itself.
///
/// `find` looks a name up and answers what `size` and `copy` need, or `None` when the
/// clipboard holds nothing under it; `size` gives the bytes held, so that more than `max` are
/// refused before they are copied; `copy` gives the bytes. A name whose data is empty is passed
/// over.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code, reason = "only Windows and macOS name their formats"))]
fn read_named<H>(
    names: &[&str],
    max: usize,
    mut find: impl FnMut(&str) -> Option<H>,
    size: impl Fn(&H) -> usize,
    mut copy: impl FnMut(H) -> Result<Vec<u8>, String>,
) -> PdfRead {
    for &name in names {
        let Some(held) = find(name) else { continue };
        if size(&held) > max {
            return Err(too_large(max));
        }
        let data = copy(held)?;
        // The size was read before the copy; a copy is never allowed to give more.
        if data.len() > max {
            return Err(too_large(max));
        }
        if !data.is_empty() {
            return Ok(Some(data));
        }
    }
    Ok(None)
}

fn too_large(max: usize) -> String {
    format!("it is larger than {} MiB", max >> 20)
}

#[cfg(test)]
mod tests {
    use super::read_named;

    /// A clipboard holding `(name, data)` pairs.
    fn read(held: &[(&str, &[u8])], names: &[&str], max: usize) -> super::PdfRead {
        let find = |name: &str| held.iter().find(|(n, _)| *n == name).map(|(_, data)| data.to_vec());
        read_named(names, max, find, Vec::len, Ok)
    }

    #[test]
    fn the_first_name_held_is_read_and_more_than_the_cap_is_refused() {
        let names = ["Portable Document Format", "PDF"];
        assert_eq!(read(&[("PDF", b"%PDF-b"), ("Portable Document Format", b"%PDF-a")], &names, 16), Ok(Some(b"%PDF-a".to_vec())));
        assert_eq!(
            read(&[("Portable Document Format", b""), ("PDF", b"%PDF-b")], &names, 16),
            Ok(Some(b"%PDF-b".to_vec())),
            "empty data passed over"
        );
        assert_eq!(read(&[("public.utf8-plain-text", b"text")], &names, 16), Ok(None), "no PDF");
        assert!(read(&[("PDF", b"%PDF-1.7 and more")], &names, 8).is_err(), "over the cap");
        // A copy that gives more than the size said is refused too.
        let grows = read_named(&names, 8, |_| Some(()), |_| 4, |_| Ok(vec![0x25; 9]));
        assert!(grows.is_err(), "a copy over the cap");
    }
}

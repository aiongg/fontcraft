//! Windows: the clipboard's registered formats through clipboard-win. A PDF goes under a format
//! registered by name; the drawing apps differ in the name, so several are tried in turn.

use std::num::NonZeroUsize;

use clipboard_win::{Clipboard, is_format_avail, raw, register_format};
use fontcraft_ui_egui::shell::clipboard::PdfRead;

use super::read_named;

/// The registered format names a PDF goes under, in the order they are tried.
const PDF_FORMATS: &[&str] = &["Portable Document Format", "PDF", "application/pdf"];
/// How often opening the clipboard is tried while another program holds it.
const OPEN_ATTEMPTS: usize = 10;

/// The format names a MIME type goes under.
fn formats(mime: &str) -> &'static [&'static str] {
    match mime {
        super::PDF_MIME => PDF_FORMATS,
        _ => &[],
    }
}

/// The clipboard's data of MIME type `mime`, at most `max` bytes.
pub(super) fn read_flavour(mime: &str, max: usize) -> PdfRead {
    let names = formats(mime);
    let registered = |name: &str| register_format(name).map(|format| format.get());
    // Asking needs no open clipboard, so a clipboard another program holds is only an error when
    // it holds the type asked for.
    if !names.iter().any(|&name| registered(name).is_some_and(is_format_avail)) {
        return Ok(None);
    }
    // The clipboard's data cannot change while it is open, so the size read is the size copied.
    let _open = Clipboard::new_attempts(OPEN_ATTEMPTS).map_err(|e| format!("the clipboard cannot be read: {e}"))?;
    read_named(
        names,
        max,
        |name| {
            let format = registered(name).filter(|&format| is_format_avail(format))?;
            Some((format, raw::size(format).map_or(0, NonZeroUsize::get)))
        },
        |&(_, size)| size,
        |(format, size)| copy(format, size),
    )
}

/// The `size` bytes held in `format`, less the zero bytes at the end: Windows may round the size
/// of a block up, and a PDF never ends in a zero byte.
fn copy(format: u32, size: usize) -> Result<Vec<u8>, String> {
    let mut data = vec![0u8; size];
    let copied = raw::get(format, &mut data).map_err(|e| format!("the clipboard cannot be read: {e}"))?;
    data.truncate(copied);
    trim_padding(&mut data);
    Ok(data)
}

fn trim_padding(data: &mut Vec<u8>) {
    let end = data.iter().rposition(|&b| b != 0).map_or(0, |at| at.saturating_add(1));
    data.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::{PDF_FORMATS, formats, read_named, trim_padding};

    #[test]
    fn a_pdf_is_looked_for_under_each_name_and_capped() {
        assert_eq!(formats(super::super::PDF_MIME), PDF_FORMATS);
        assert!(formats("image/svg+xml").is_empty());
        // A clipboard that holds a PDF under the last name only, its block rounded up.
        let held = |name: &str| (name == "application/pdf").then(|| b"%PDF-1.7\n%%EOF\n\0\0".to_vec());
        let read = |max| {
            read_named(PDF_FORMATS, max, held, Vec::len, |mut data| {
                trim_padding(&mut data);
                Ok(data)
            })
        };
        assert_eq!(read(64), Ok(Some(b"%PDF-1.7\n%%EOF\n".to_vec())));
        assert!(read(8).is_err(), "over the cap");
        let nothing = read_named(PDF_FORMATS, 64, |_| None::<Vec<u8>>, Vec::len, Ok);
        assert_eq!(nothing, Ok(None), "no PDF");
    }
}

//! macOS: the general pasteboard's types through objc2-app-kit. A PDF goes under its uniform type
//! identifier; apps that still write the type AppKit used before identifiers have it under that
//! older name.
//!
//! The pasteboard hands over a type's data whole, so the size is known only once the data is in
//! memory; the cap then keeps it from being copied and pasted.

use fontcraft_ui_egui::shell::clipboard::PdfRead;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

use super::read_named;

/// The pasteboard types a PDF goes under, in the order they are tried. These are the values of
/// AppKit's `NSPasteboardTypePDF` and `NSPDFPboardType`, which are extern statics that only
/// unsafe code can read; the workspace forbids it.
const PDF_TYPES: &[&str] = &["com.adobe.pdf", "Apple PDF pasteboard type"]; // brand-ok: the system pasteboard type

/// The pasteboard types a MIME type goes under.
fn types(mime: &str) -> &'static [&'static str] {
    match mime {
        super::PDF_MIME => PDF_TYPES,
        _ => &[],
    }
}

/// The pasteboard's data of MIME type `mime`, at most `max` bytes.
pub(super) fn read_flavour(mime: &str, max: usize) -> PdfRead {
    let names = types(mime);
    if names.is_empty() {
        return Ok(None);
    }
    let pasteboard = NSPasteboard::generalPasteboard();
    read_named(names, max, |name| pasteboard.dataForType(&NSString::from_str(name)), |data| data.len(), |data| Ok(data.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::{PDF_TYPES, read_named, types};

    #[test]
    fn a_pdf_is_looked_for_under_each_type_and_capped() {
        assert_eq!(types(super::super::PDF_MIME), PDF_TYPES);
        assert!(types("image/svg+xml").is_empty());
        // A pasteboard that has the PDF under the older type only.
        let held = |name: &str| (name == "Apple PDF pasteboard type").then(|| b"%PDF-1.4\n%%EOF".to_vec());
        assert_eq!(read_named(PDF_TYPES, 64, held, Vec::len, Ok), Ok(Some(b"%PDF-1.4\n%%EOF".to_vec())));
        assert!(read_named(PDF_TYPES, 8, held, Vec::len, Ok).is_err(), "over the cap");
        assert_eq!(read_named(PDF_TYPES, 64, |_| None::<Vec<u8>>, Vec::len, Ok), Ok(None), "no PDF");
    }
}

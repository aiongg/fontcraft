//! What the desktop app plugs into the UI for printing: the system's printers and their queue.
//!
//! The UI crate starts no process and no thread; the app supplies a [`PrintHost`] (`apps/fontcraft`, over the suite's
//! print service). Without one (the browser build, tests) the Print dialog saves PDF files only.

use std::sync::mpsc::Receiver;

use craft::print::ui::{Printer, Settings, Submission};

/// The system's print service. Both calls return at once; their answers come on the receivers.
pub trait PrintHost: Send + Sync {
    /// Lists the printers.
    fn printers(&self) -> Receiver<Vec<Printer>>;
    /// Sends `pdf` to the printer `settings` name; the answer is the spooler's receipt or its error.
    fn print(&self, pdf: Vec<u8>, settings: Settings) -> Submission;
}

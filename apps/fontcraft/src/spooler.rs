//! The app's [`PrintHost`]: the system's print queue through the suite's print service (decision 0008), CUPS on macOS
//! and Linux and the XPS print API on Windows.
//!
//! Both calls can wait on a device, so each runs on a thread of its own and answers over a channel; a thread that
//! cannot start drops its channel, which the dialog reports. Nothing the person types reaches a command line: a job
//! names one of the printers the system listed, the settings of the print panel and a fixed title, and the pages go
//! over a pipe.

use std::sync::mpsc::{Receiver, channel};

use craft::print::ui::{Duplex, Printer, Settings, Submission};
use craft::print::{Job, Scaling};
use fontcraft_ui::app::print_host::PrintHost;

/// The job's name in the print queue.
const TITLE: &str = "FontCraft proof";

pub struct Spooler;

impl PrintHost for Spooler {
    fn printers(&self) -> Receiver<Vec<Printer>> {
        let (send, receive) = channel();
        let list = move || {
            let found = craft::print::printer_queues();
            let _ = send.send(found.into_iter().map(|p| Printer::new(p.name).label(p.description).is_default(p.default)).collect());
        };
        if let Err(e) = std::thread::Builder::new().name("print-list".into()).spawn(list) {
            log(&format!("the printers cannot be listed: {e}"));
        }
        receive
    }

    fn print(&self, pdf: Vec<u8>, settings: Settings) -> Submission {
        let (send, receive) = channel();
        let submit = move || {
            let _ = send.send(craft::print::submit(&pdf, &job(&settings)).map_err(|e| e.to_string()));
        };
        if let Err(e) = std::thread::Builder::new().name("print-job".into()).spawn(submit) {
            log(&format!("the pages cannot be printed: {e}"));
        }
        receive
    }
}

fn log(message: &str) {
    eprintln!("fontcraft: {message}");
}

/// The job for the print panel's choices. A page larger than the paper is shrunk onto it.
fn job(settings: &Settings) -> Job {
    let duplex = match settings.duplex {
        Some(Duplex::Off) => Some(craft::print::Duplex::Off),
        Some(Duplex::LongEdge) => Some(craft::print::Duplex::LongEdge),
        Some(Duplex::ShortEdge) => Some(craft::print::Duplex::ShortEdge),
        _ => None,
    };
    Job {
        printer: Some(settings.printer.clone()).filter(|name| !name.is_empty()),
        copies: settings.copies,
        collate: true,
        duplex,
        scaling: Scaling::ShrinkToFit,
        grayscale: settings.grayscale,
        title: TITLE.into(),
        ..Job::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel's choices reach the job as whole arguments; a printer's name is never read as an option.
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn the_panels_choices_become_the_jobs() {
        let mut settings = Settings::default();
        settings.printer = "-o raw".into();
        settings.copies = 3;
        settings.duplex = Some(Duplex::LongEdge);
        settings.grayscale = true;
        let args = craft::print::lp_args(&job(&settings));
        let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str);
        assert_eq!((after("-d"), after("-n"), after("-t")), (Some("-o raw"), Some("3"), Some(TITLE)));
        for option in ["sides=two-sided-long-edge", "print-color-mode=monochrome", "print-scaling=auto-fit"] {
            assert!(args.iter().any(|a| a == option), "{option} in {args:?}");
        }
        let plain = job(&Settings::default());
        assert_eq!((plain.printer, plain.duplex), (None, None), "what the panel leaves open stays the queue's");
    }
}

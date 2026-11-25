//! What opening a document reported: the view-only mark's notice and the list of the open warnings. Rows:
//! `open.compiledIsViewOnly`, `open.compiledIsLossy`, `file.openReport`.
//!
//! Every `file.open` that opens a document leaves its `warnings` here, whatever they are about: tables of a compiled
//! font that are not read, parts of a source that were skipped. The notice bar over a document shows while it is
//! view-only or its report has warnings, until the person closes the bar or saves the document. Show Warnings… and
//! `file.openReport` list them in a dialog (`dialogs/open_report.rs`); nothing opens by itself, so a request never
//! waits for an answer.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Map, Value, json};

use super::FontCraftApp;
use crate::shell::commands::{Module, UiCommand};
use crate::shell::params;

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "file.openReport",
    label: "Open Warnings…",
    menu: &[],
    shortcut: None,
    params: "{show?: bool (default true), dismiss?: bool} list what opening the active document left out or could not read, as file.open's warnings gave it; show: false closes the list, dismiss: true closes the notice bar over the document. Refused when there is nothing to list. ui.inspect reports it as openReport: {viewOnly, warnings: [text], notice: bool (the bar shows), open: bool (the list shows)}",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most warnings one document keeps.
pub const MAX_WARNINGS: usize = 5_000;
/// Longest warning kept, in characters.
const MAX_WARNING: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Report {
    doc: u64,
    warnings: Vec<String>,
}

/// The reports of the open documents.
#[derive(Clone, Debug, Default)]
pub struct OpenReports {
    reports: Vec<Report>,
    /// The documents whose notice bar was closed.
    dismissed: Vec<u64>,
    /// The document whose list shows.
    showing: Option<u64>,
}

impl OpenReports {
    /// The document whose list shows and its warnings.
    pub fn list(&self) -> Option<(u64, &[String])> {
        let doc = self.showing?;
        Some((doc, self.warnings(doc)))
    }

    fn warnings(&self, doc: u64) -> &[String] {
        self.reports.iter().find(|r| r.doc == doc).map(|r| r.warnings.as_slice()).unwrap_or_default()
    }

    fn dismiss(&mut self, doc: u64) {
        if !self.dismissed.contains(&doc) {
            self.dismissed.push(doc);
        }
    }

    /// Closes the list.
    pub fn close(&mut self) {
        self.showing = None;
    }
}

/// What the notice bar over the active document says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentNotice {
    pub view_only: bool,
    pub warnings: usize,
}

/// The notice bar of the active document; `None` hides the bar.
pub fn notice(app: &FontCraftApp) -> Option<DocumentNotice> {
    let doc = app.session.active()?;
    let reports = &app.view.open_report;
    let notice = DocumentNotice { view_only: doc.view_only(), warnings: reports.warnings(doc.id).len() };
    let says = notice.view_only || notice.warnings > 0;
    (says && !reports.dismissed.contains(&doc.id)).then_some(notice)
}

/// After command `id` ran: keeps the report of a document just opened, and closes the notice of one just saved, whose
/// next open is from its new file.
pub fn after_run(app: &mut FontCraftApp, id: &str, reply: &Value) {
    let Some(doc) = app.session.active().map(|d| d.id) else { return };
    let reports = &mut app.view.open_report;
    match id {
        "file.open" if reply.get("viewOnly").is_some() => {
            let list = reply.get("warnings").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            let warnings: Vec<String> =
                list.iter().filter_map(Value::as_str).take(MAX_WARNINGS).map(|w| w.chars().take(MAX_WARNING).collect()).collect();
            let open: Vec<u64> = app.session.documents().iter().map(|d| d.id).collect();
            reports.reports.retain(|r| r.doc != doc && open.contains(&r.doc));
            reports.dismissed.retain(|d| *d != doc && open.contains(d));
            reports.reports.push(Report { doc, warnings });
        }
        "file.save" | "file.saveAs" => reports.dismiss(doc),
        _ => {}
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    let dismiss = params::flag(p, "dismiss")?.unwrap_or(false);
    let show = params::flag(p, "show")?.unwrap_or(!dismiss);
    let doc = app.session.active().map(|d| d.id).ok_or("no font is open")?;
    if dismiss {
        app.view.open_report.dismiss(doc);
    }
    if !show {
        app.view.open_report.close();
    } else if app.view.open_report.warnings(doc).is_empty() {
        return Err("opening this document gave no warnings".into());
    } else {
        app.view.open_report.showing = Some(doc);
    }
    app.view_changed();
    Ok(())
}

/// `ui.inspect`'s `openReport`.
pub fn inspect(app: &FontCraftApp, out: &mut Map<String, Value>) {
    let doc = app.session.active();
    let warnings = doc.map(|d| app.view.open_report.warnings(d.id)).unwrap_or_default();
    let state = json!({
        "viewOnly": doc.is_some_and(|d| d.view_only()),
        "warnings": warnings,
        "notice": notice(app).is_some(),
        "open": app.view.open_report.showing.is_some(),
    });
    out.insert("openReport".into(), state);
}

/// Drops the list of a document that was closed.
pub fn prune(app: &mut FontCraftApp) {
    let gone = app.view.open_report.showing.is_some_and(|doc| app.session.documents().iter().all(|d| d.id != doc));
    if gone {
        app.view.open_report.close();
    }
}

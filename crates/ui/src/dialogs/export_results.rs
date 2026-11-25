//! The Export Results window (`app/export_results.rs`): errors, skipped fonts, warnings and files of the last export,
//! in that order. It leaves the rest of the window usable, so the rows can be worked through one by one.
//!
//! | Control | Does |
//! |---|---|
//! | Show, double press or Enter on a row | `file.exportResults {reveal}`: the glyph in a new tab, the feature code in Font Info |
//! | Reveal | the same for a file: shows it in the system's file manager |
//! | Copy All | puts the whole list on the clipboard as text |

use craft::behavior::{ColumnKey, ColumnWidth, Selection};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, ColumnSpec, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::env::Env;
use egui::Ui;

use super::report_list::{self, Line};
use crate::app::export_results::{ExportResults, Group, Target, reveal_msg};
use crate::app::{FontCraftApp, Msg};

craft::ui::words! {
    /// The words of the Export Results window.
    pub struct ExportResultsWords {
        title: "fontcraftExportResults.title" = "Export Results";
        list: "fontcraftExportResults.list" = "Results";
        kind: "fontcraftExportResults.kind" = "Result";
        subject: "fontcraftExportResults.subject" = "What";
        message: "fontcraftExportResults.message" = "Message";
        stopped: "fontcraftExportResults.stopped" = "Nothing was written.";
        error: "fontcraftExportResults.error" = "Error";
        skipped: "fontcraftExportResults.skipped" = "Skipped";
        warning: "fontcraftExportResults.warning" = "Warning";
        file: "fontcraftExportResults.file" = "File";
        errors: "fontcraftExportResults.errors" = "Errors";
        all_skipped: "fontcraftExportResults.allSkipped" = "Skipped";
        warnings: "fontcraftExportResults.warnings" = "Warnings";
        files: "fontcraftExportResults.files" = "Files";
        show: "fontcraftExportResults.show" = "Show";
        show_tip: "fontcraftExportResults.showTip" = "Show the glyph or the feature code of the selected row";
        reveal: "fontcraftExportResults.reveal" = "Reveal";
        reveal_tip: "fontcraftExportResults.revealTip" = "Show the file of the selected row in the file manager";
        copy: "fontcraftExportResults.copy" = "Copy All";
    }
}

const KIND: ColumnKey = ColumnKey::new("kind");
const SUBJECT: ColumnKey = ColumnKey::new("subject");
const MESSAGE: ColumnKey = ColumnKey::new("message");

/// The window, with the export it lists.
pub struct ExportResultsDialog {
    id: DialogId,
    results: ExportResults,
    selected: Selection<usize>,
    /// What the person did, until the dialog module takes it.
    outbox: Vec<Msg>,
    closing: bool,
    open: bool,
}

impl ExportResultsDialog {
    pub fn new(results: ExportResults) -> Self {
        ExportResultsDialog {
            id: dialog_id("fontcraftExportResults"),
            results,
            selected: Selection::default(),
            outbox: Vec::new(),
            closing: false,
            open: true,
        }
    }
}

impl StackDialog for ExportResultsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<ExportResultsWords>();
        let ExportResultsDialog { results, selected, outbox, open, .. } = self;
        let shown = Dialog::new(words.title.clone())
            .opened(Bind::bound(open))
            .modeless()
            .resizable(true)
            .show(ui, |ui, _| view(ui, results, selected, outbox));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// The window's body.
pub fn view(ui: &mut Ui, results: &ExportResults, selected: &mut Selection<usize>, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ExportResultsWords>();
    Stack::new().show(ui, |ui, _| {
        let stopped = results.rows.iter().any(|r| r.group == Group::Error);
        Text::new(if stopped { format!("{}: {}", results.title, words.stopped.as_str()) } else { results.title.clone() }).show(ui);
        let columns = vec![
            ColumnSpec::new(KIND, words.kind.clone(), ColumnWidth::Content),
            ColumnSpec::new(SUBJECT, words.subject.clone(), ColumnWidth::Content),
            ColumnSpec::new(MESSAGE, words.message.clone(), ColumnWidth::fill(1.0)),
        ];
        let lines: Vec<Line> = results
            .rows
            .iter()
            .map(|row| {
                let kind = match row.group {
                    Group::Error => words.error.as_str(),
                    Group::Skipped => words.skipped.as_str(),
                    Group::Warning => words.warning.as_str(),
                    Group::File => words.file.as_str(),
                };
                Line::new(kind).cell(SUBJECT, row.subject.clone()).cell(MESSAGE, row.text.clone())
            })
            .collect();
        let pressed = report_list::show(ui, words.list.clone(), columns, &lines, Some(&mut *selected));
        let target = |at: usize| results.rows.get(at).and_then(|r| r.target.as_ref());
        if let Some(at) = pressed.filter(|at| target(*at).is_some()) {
            sent.push(reveal_msg(at));
        }
        let chosen = selected.items.last().copied();
        let is_file = chosen.and_then(target).map(|t| matches!(t, Target::File(_)));
        Row::new().show(ui, |ui, _| {
            let (label, tip) =
                if is_file == Some(true) { (words.reveal.clone(), words.reveal_tip.clone()) } else { (words.show.clone(), words.show_tip.clone()) };
            let shown = Button::new(label).tooltip(tip).id_salt("fontcraftExportResults.show").enabled(is_file.is_some()).show(ui);
            if let Some(at) = chosen.filter(|_| shown.clicked() && is_file.is_some()) {
                sent.push(reveal_msg(at));
            }
            if Button::new(words.copy.clone()).show(ui).clicked() {
                let text = results.text(|group| {
                    let name = match group {
                        Group::Error => &words.errors,
                        Group::Skipped => &words.all_skipped,
                        Group::Warning => &words.warnings,
                        Group::File => &words.files,
                    };
                    name.as_str().to_string()
                });
                ui.ctx().copy_text(text);
            }
        });
    });
}

/// Keeps the window in step with the app's state.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<ExportResultsDialog>().is_some() {
        app.view.export_results.open = false;
        return;
    }
    let results = &app.view.export_results;
    match host.get_mut::<ExportResultsDialog>() {
        Some(dialog) => {
            app.sent.append(&mut dialog.outbox);
            dialog.closing = !results.open;
            if results.open && dialog.results.serial != results.serial {
                dialog.results = results.clone();
                dialog.selected = Selection::default();
            }
        }
        None if results.open => {
            host.open(ExportResultsDialog::new(results.clone()));
        }
        None => {}
    }
}

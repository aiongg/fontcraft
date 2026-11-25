//! The list of what opening a document left out or could not read (`app/open_report.rs`).

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, keyed};
use craft::ui::contract::{Bind, ColumnSpec, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use egui::Ui;

use crate::app::{FontCraftApp, open_report};

craft::ui::words! {
    /// The words of the open warnings' list.
    pub struct OpenReportWords {
        title: "fontcraftOpenReport.title" = "Open Warnings";
        message: "fontcraftOpenReport.message" = "was opened, but not everything in its file could be read:";
        warnings: "fontcraftOpenReport.warnings" = "Warnings";
        warning: "fontcraftOpenReport.warning" = "Warning";
        close: "fontcraftOpenReport.close" = "Close";
    }
}

/// Rows the list shows before it scrolls.
const LIST_ROWS: u16 = 12;

/// The list, with the document it is about.
pub struct OpenReportDialog {
    id: DialogId,
    doc: u64,
    title: String,
    warnings: Vec<String>,
    closing: bool,
    seen: bool,
    open: bool,
}

impl OpenReportDialog {
    pub fn new(doc: u64, title: String, warnings: Vec<String>) -> Self {
        OpenReportDialog { id: dialog_id("fontcraft.openReport"), doc, title, warnings, closing: false, seen: true, open: true }
    }
}

const WARNING: ColumnKey = ColumnKey::new("warning");

impl StackDialog for OpenReportDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<OpenReportWords>();
        let OpenReportDialog { title, warnings, seen, open, .. } = self;
        let warnings = &*warnings;
        let shown =
            FormDialog::new(words.title.clone(), Bind::bound(seen)).opened(Bind::bound(open)).ok_label(words.close.clone()).show(ui, |ui, _, _| {
                Text::new(format!("{title} {}", words.message.as_str())).show(ui);
                let columns = vec![ColumnSpec::new(WARNING, words.warning.clone(), ColumnWidth::Content)];
                let positions: Vec<usize> = (0..warnings.len()).collect();
                let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(warnings.get(*at).cloned().unwrap_or_default()));
                let _ = Table::<usize>::new(words.warnings.clone(), columns)
                    .max_rows(LIST_ROWS)
                    .show_items(ui, &source, &mut |at, icx| ListRow::new(warnings.get(*at).cloned().unwrap_or_default()).cells(icx.cells()));
                true
            });
        (shown.accepted.is_some() || shown.closed.is_some()).then_some(DialogResult::Cancel)
    }
}

/// Keeps the list in step with the app's state.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    open_report::prune(app);
    if host.take::<OpenReportDialog>().is_some() {
        app.view.open_report.close();
        return;
    }
    let shown = app.view.open_report.list().map(|(doc, warnings)| (doc, warnings.to_vec()));
    match (shown, host.get_mut::<OpenReportDialog>()) {
        (Some((doc, _)), Some(dialog)) if dialog.doc == doc => {}
        (Some((doc, warnings)), None) => {
            let title = app.session.documents().iter().find(|d| d.id == doc).map(|d| d.title.clone()).unwrap_or_default();
            host.open(OpenReportDialog::new(doc, title, warnings));
        }
        // The list of another document, or none: this one ends first. The next pass opens the other.
        (Some(_), Some(dialog)) => dialog.closing = true,
        (None, Some(dialog)) => dialog.closing = true,
        (None, None) => {}
    }
}

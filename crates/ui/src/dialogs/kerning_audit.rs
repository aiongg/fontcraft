//! Font › Kerning › Audit Kerning Exceptions… and Match Kerning….
//!
//! - Audit (`dialog.auditKerningExceptions`): the threshold, the writing direction and the masters, and the glyph
//!   pairs `kerning.auditExceptions` found closer than the threshold under class kerning. Audit runs it again with
//!   the settings; Add Exceptions runs it with `fix`, and the list then shows the exception each pair got. Close
//!   leaves.
//! - Match Kerning (`dialog.matchKerning`): the main master; Match runs `kerning.matchAcrossMasters`, and a notice
//!   says how many pairs each master had before and after.
//!
//! The shell command runs the engine command and keeps its report in the view state; this module opens the dialog
//! and takes each new report into it.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, keyed};
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, ColumnSpec, Entry, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{Alert, DialogHost, DialogId, DialogResult, DialogStack, HostedAlert, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, WordSet as _};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::table::Table;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::panels::kerning_window::format_value;
use crate::shell::kerning_window::{Audit, MasterChoice, MatchSummary};

craft::ui::words! {
    /// The words of the kerning audit and Match Kerning dialogs.
    pub struct KerningAuditWords {
        audit_title: "fontcraftKerningAudit.title" = "Audit Kerning Exceptions";
        threshold: "fontcraftKerningAudit.threshold" = "Threshold";
        direction: "fontcraftKerningAudit.direction" = "Direction";
        ltr: "fontcraftKerningAudit.ltr" = "Left to Right";
        rtl: "fontcraftKerningAudit.rtl" = "Right to Left";
        masters: "fontcraftKerningAudit.masters" = "Masters";
        every_master: "fontcraftKerningAudit.everyMaster" = "Every Master";
        audit: "fontcraftKerningAudit.audit" = "Audit";
        add_exceptions: "fontcraftKerningAudit.addExceptions" = "Add Exceptions";
        close: "fontcraftKerningAudit.close" = "Close";
        findings: "fontcraftKerningAudit.findings" = "Glyph pairs closer than the threshold";
        none_found: "fontcraftKerningAudit.noneFound" = "No glyph pair comes closer than the threshold.";
        found: "fontcraftKerningAudit.found" = "glyph pairs closer than the threshold";
        measured: "fontcraftKerningAudit.measured" = "measured";
        skipped: "fontcraftKerningAudit.skipped" = "skipped";
        listed: "fontcraftKerningAudit.listed" = "listed";
        added: "fontcraftKerningAudit.added" = "exceptions added";
        master: "fontcraftKerningAudit.master" = "Master";
        left: "fontcraftKerningAudit.left" = "Left";
        right: "fontcraftKerningAudit.right" = "Right";
        classes: "fontcraftKerningAudit.classes" = "Class Pair";
        value: "fontcraftKerningAudit.value" = "Value";
        distance: "fontcraftKerningAudit.distance" = "Distance";
        exception: "fontcraftKerningAudit.exception" = "Exception";
        match_title: "fontcraftKerningAudit.matchTitle" = "Match Kerning";
        main_master: "fontcraftKerningAudit.mainMaster" = "Main Master";
        default_master: "fontcraftKerningAudit.defaultMaster" = "Default Master";
        match_message: "fontcraftKerningAudit.matchMessage" = "Give every master the same kerning pairs, so kerning interpolates. The kerning between any two glyphs stays the same.";
        do_match: "fontcraftKerningAudit.match" = "Match";
        matched: "fontcraftKerningAudit.matched" = "Kerning matched across masters";
        filled: "fontcraftKerningAudit.filled" = "pairs filled";
        exceptions: "fontcraftKerningAudit.exceptions" = "exceptions added";
        pairs_before_after: "fontcraftKerningAudit.pairsBeforeAfter" = "pairs before → after";
        details: "fontcraftKerningAudit.details" = "Masters";
    }
}

/// The picked index of the first extra footer button (`FormOutput::picked`).
const FIRST_EXTRA: usize = 2;
/// The picked index of OK, which a held OK reports instead of `accepted`.
const OK: usize = 0;
/// Rows a dialog's list shows before it scrolls: the dialog body around it scrolls too.
const LIST_ROWS: u16 = 12;

const MASTER: ColumnKey = ColumnKey::new("master");
const LEFT: ColumnKey = ColumnKey::new("left");
const RIGHT: ColumnKey = ColumnKey::new("right");
const CLASSES: ColumnKey = ColumnKey::new("classes");
const VALUE: ColumnKey = ColumnKey::new("value");
const DISTANCE: ColumnKey = ColumnKey::new("distance");
const EXCEPTION: ColumnKey = ColumnKey::new("exception");

/// The audit dialog: its settings and the report it shows.
pub struct AuditDialog {
    id: DialogId,
    pub threshold: Option<f64>,
    /// "ltr" or "rtl".
    pub direction: String,
    /// A master id; empty: every master.
    pub master: String,
    pub report: Audit,
    /// The `dialog.auditKerningExceptions` parameters a button asked for, for the module to post.
    pub request: Option<Value>,
    open: bool,
}

impl AuditDialog {
    pub fn new(report: Audit) -> AuditDialog {
        AuditDialog {
            id: dialog_id("kerning.audit"),
            threshold: Some(report.threshold),
            direction: report.direction.clone(),
            master: report.masters.first().cloned().unwrap_or_default(),
            report,
            request: None,
            open: true,
        }
    }

    /// The shell command's parameters for the settings, with `fix`.
    pub fn params(&self, fix: bool) -> Value {
        let masters: Vec<&str> = if self.master.is_empty() { Vec::new() } else { vec![self.master.as_str()] };
        json!({ "threshold": self.threshold.unwrap_or(0.0), "direction": self.direction, "masters": masters, "fix": fix })
    }

    /// The line over the list.
    pub fn summary(&self, words: &KerningAuditWords) -> String {
        let r = &self.report;
        let mut line =
            format!("{} {} · {} {} · {} {}", r.found, words.found.as_str(), r.measured, words.measured.as_str(), r.skipped, words.skipped.as_str());
        if r.found > r.findings.len() {
            line.push_str(&format!(" · {} {}", r.findings.len(), words.listed.as_str()));
        }
        if r.fixed {
            let added = r.findings.iter().filter(|f| f.fix.is_some()).count();
            line = format!("{added} {} · {line}", words.added.as_str());
        }
        line
    }
}

fn master_entries(choices: &[MasterChoice], none: &craft::ui::runtime::env::Label) -> Vec<Entry<String>> {
    std::iter::once(Entry::new(String::new(), none.clone())).chain(choices.iter().map(|m| Entry::new(m.id.clone(), m.name.clone()))).collect()
}

impl StackDialog for AuditDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let words = Env::of(ui).words::<KerningAuditWords>();
        let summary = self.summary(&words);
        let can_fix = !self.report.findings.is_empty() && !self.report.fixed;
        let audit = FooterButton::new(words.audit.clone(), DialogRole::Other);
        let directions = [Entry::new("ltr".to_string(), words.ltr.clone()), Entry::new("rtl".to_string(), words.rtl.clone())];
        let masters = master_entries(&self.report.choices, &words.every_master);
        let AuditDialog { threshold, direction, master, report, open, .. } = self;
        let findings = &report.findings;
        let mut shown_value = false;
        let shown = FormDialog::new(words.audit_title.clone(), Bind::bound(&mut shown_value))
            .opened(Bind::bound(open))
            .ok_label(words.add_exceptions.clone())
            .cancel_label(words.close.clone())
            .ok_enabled(can_fix)
            .accept_stays_open(true)
            .extra_buttons([audit])
            .show(ui, |ui, _, _| {
                let _ = Row::new().fill_width().show(ui, |ui, _| {
                    Field::new(words.threshold.clone()).show(ui, |ui, _| {
                        NumberField::new(words.threshold.clone(), Bind::bound(threshold)).show(ui);
                    });
                    Segmented::new(words.direction.clone(), Bind::bound(direction), &directions).show(ui);
                    Field::new(words.masters.clone()).show(ui, |ui, _| {
                        Select::new(words.masters.clone(), Bind::bound(master), &masters).show(ui);
                    });
                });
                Text::new(summary.clone()).shade(Shade::Muted).show(ui);
                if findings.is_empty() {
                    Text::new(words.none_found.clone()).show(ui);
                    return true;
                }
                let columns = vec![
                    ColumnSpec::new(MASTER, words.master.clone(), ColumnWidth::Content),
                    ColumnSpec::new(LEFT, words.left.clone(), ColumnWidth::Content),
                    ColumnSpec::new(RIGHT, words.right.clone(), ColumnWidth::Content),
                    ColumnSpec::new(CLASSES, words.classes.clone(), ColumnWidth::Content),
                    ColumnSpec::new(VALUE, words.value.clone(), ColumnWidth::Content).numeric(),
                    ColumnSpec::new(DISTANCE, words.distance.clone(), ColumnWidth::Content).numeric(),
                    ColumnSpec::new(EXCEPTION, words.exception.clone(), ColumnWidth::Content).numeric(),
                ];
                let positions: Vec<usize> = (0..findings.len()).collect();
                let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(findings.get(*at).map_or_else(String::new, |f| f.left.clone())));
                let _ = Table::<usize>::new(words.findings.clone(), columns).max_rows(LIST_ROWS).show_items(ui, &source, &mut |at, icx| {
                    let Some(f) = findings.get(*at) else { return ListRow::new("") };
                    ListRow::new(f.master.clone())
                        .cells(icx.cells())
                        .cell(LEFT, f.left.clone())
                        .cell(RIGHT, f.right.clone())
                        .cell(CLASSES, format!("{} {}", f.class_left, f.class_right))
                        .cell(VALUE, format_value(f.value))
                        .cell(DISTANCE, format_value(f.distance))
                        .cell(EXCEPTION, f.fix.map(format_value).unwrap_or_default())
                });
                true
            });
        if shown.picked == Some(FIRST_EXTRA) {
            self.request = Some(self.params(false));
        } else if (shown.picked == Some(OK) || shown.accepted.is_some()) && can_fix {
            self.request = Some(self.params(true));
        }
        shown.closed.filter(|_| shown.accepted.is_none()).map(|_| DialogResult::Cancel)
    }
}

/// Match Kerning's question: the main master.
pub struct MatchDialog {
    id: DialogId,
    pub choices: Vec<MasterChoice>,
    /// A master id; empty: the font's default master.
    pub main: String,
    open: bool,
}

impl MatchDialog {
    pub fn new(choices: Vec<MasterChoice>) -> MatchDialog {
        MatchDialog { id: dialog_id("kerning.match"), choices, main: String::new(), open: true }
    }

    /// `dialog.matchKerning`'s parameters that run the match.
    pub fn params(main: &str) -> Value {
        if main.is_empty() { json!({ "run": true }) } else { json!({ "run": true, "mainMaster": main }) }
    }
}

impl StackDialog for MatchDialog {
    type Result = Value;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Value>> {
        let words = Env::of(ui).words::<KerningAuditWords>();
        let masters = master_entries(&self.choices, &words.default_master);
        let MatchDialog { main, open, .. } = self;
        let shown = FormDialog::new(words.match_title.clone(), Bind::bound(main)).opened(Bind::bound(open)).ok_label(words.do_match.clone()).show(
            ui,
            |ui, _, main| {
                Text::new(words.match_message.clone()).show(ui);
                Field::new(words.main_master.clone()).show(ui, |ui, _| {
                    Select::new(words.main_master.clone(), Bind::bound(main), &masters).show(ui);
                });
                true
            },
        );
        match (shown.accepted, shown.closed) {
            (Some(main), _) => Some(DialogResult::Ok(MatchDialog::params(&main))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// The notice after Match Kerning ran.
pub fn match_notice(summary: &MatchSummary) -> (String, String) {
    let words = KerningAuditWords::english();
    let message = format!("{} {} · {} {}", summary.filled, words.filled.as_str(), summary.exceptions, words.exceptions.as_str());
    let mut lines: Vec<String> =
        summary.masters.iter().map(|m| format!("{}: {} → {} ({})", m.name, m.before, m.after, words.pairs_before_after.as_str())).collect();
    lines.extend(summary.skipped.iter().cloned());
    (message, lines.join("\n"))
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// Opens the audit and Match Kerning dialogs the shell commands asked for, takes each new report into the open
/// audit dialog, and posts what their buttons ask.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<AuditDialog>().is_some() {
        app.view.kerning.audit = None;
    }
    if let Some(report) = &app.view.kerning.audit {
        match host.get_mut::<AuditDialog>() {
            Some(dialog) if dialog.report.run != report.run => dialog.report = report.clone(),
            Some(_) => {}
            None => {
                host.open(AuditDialog::new(report.clone()));
            }
        }
    }
    if let Some(params) = host.get_mut::<AuditDialog>().and_then(|d| d.request.take()) {
        app.sent.push(shell("dialog.auditKerningExceptions", params));
    }

    if let Some(answer) = host.take::<MatchDialog>() {
        match answer {
            DialogResult::Ok(params) => app.sent.push(shell("dialog.matchKerning", params)),
            _ => app.view.kerning.matching = None,
        }
    }
    if let Some(matching) = app.view.kerning.matching.take() {
        match &matching.summary {
            Some(summary) => {
                let words = KerningAuditWords::english();
                let (message, details) = match_notice(summary);
                host.open(HostedAlert::new(Alert::new(words.matched.clone(), message).details(words.details.clone(), details)));
            }
            None if host.get_mut::<MatchDialog>().is_none() => {
                host.open(MatchDialog::new(matching.choices));
            }
            None => {}
        }
    }
}

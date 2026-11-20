//! The Audit Profile dialog (`dialog.auditProfile`): edits the draft `audit.editProfile` read
//! (`shell::font_audit::profile`).
//!
//! | Control | Does |
//! |---|---|
//! | name, label, storage | the profile's id (new profiles only), label and store |
//! | Starts From boxes | the profiles whose checks it starts from |
//! | the check table: a check's Run box | runs the check or not (`checks` or `exclude`) |
//! | a check's row | selects it: its severity override and its thresholds show under the table |
//! | project values | the open document's `audit.config.set {values}`, on Save |
//! | Duplicate | an editable copy under a new id, in the user's profiles |
//! | Delete, Cancel, Save | `audit.closeProfileDialog {action}` |
//!
//! The dialog edits its own copy of the draft. Each pass the module takes the person's changes into the app's draft
//! (reading the selected check's thresholds and the included profiles' checks from the engine), and a draft the
//! commands replaced or closed into the dialog.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, GesturePhase, ItemSource, Selection};
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, ToggleColumn};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{ListRow, ToggleCell, ToggleState};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::table::Table;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::panels::widgets::{self, NumberEdit};
use crate::shell::font_audit::profile::{self, MAX_ENTRIES, MAX_TEXT, ProfileDraft, base_of, thresholds_of};
use crate::shell::font_audit::run::GROUPS;

craft::ui::words! {
    /// The words of the Audit Profile dialog.
    pub struct AuditProfileWords {
        title: "fontcraftAuditProfile.title" = "Audit Profile";
        read_only: "fontcraftAuditProfile.readOnly" = "Built-in profiles cannot be changed; Duplicate makes an editable copy.";
        name: "fontcraftAuditProfile.name" = "Name";
        label: "fontcraftAuditProfile.label" = "Label";
        storage: "fontcraftAuditProfile.storage" = "Storage";
        user: "fontcraftAuditProfile.user" = "My Profiles";
        project: "fontcraftAuditProfile.project" = "This Font";
        starts_from: "fontcraftAuditProfile.startsFrom" = "Starts From";
        checks: "fontcraftAuditProfile.checks" = "Checks";
        of: "fontcraftAuditProfile.of" = "of";
        run_count: "fontcraftAuditProfile.runCount" = "run";
        search: "fontcraftAuditProfile.search" = "Search";
        runs: "fontcraftAuditProfile.runs" = "Run";
        check: "fontcraftAuditProfile.check" = "Check";
        group: "fontcraftAuditProfile.group" = "Group";
        severity: "fontcraftAuditProfile.severity" = "Severity";
        default_severity: "fontcraftAuditProfile.defaultSeverity" = "Default";
        fail: "fontcraftAuditProfile.fail" = "Fail";
        warn: "fontcraftAuditProfile.warn" = "Warn";
        info: "fontcraftAuditProfile.info" = "Info";
        thresholds: "fontcraftAuditProfile.thresholds" = "Thresholds of";
        no_thresholds: "fontcraftAuditProfile.noThresholds" = "This check has no thresholds.";
        default_value: "fontcraftAuditProfile.defaultValue" = "default";
        values: "fontcraftAuditProfile.values" = "Project values of this font";
        duplicate: "fontcraftAuditProfile.duplicate" = "Duplicate";
        delete: "fontcraftAuditProfile.delete" = "Delete";
        save: "fontcraftAuditProfile.save" = "Save";
    }
}

/// Rows the check table shows before it scrolls: the dialog body around it scrolls too.
const LIST_ROWS: u16 = 12;
/// `FormOutput::picked` of OK, which a held OK reports instead of `accepted`, and of the extra buttons.
const OK: usize = 0;
const DUPLICATE: usize = 2;
const DELETE: usize = 3;

const RUNS: ColumnKey = ColumnKey::new("runs");
const CHECK: ColumnKey = ColumnKey::new("check");
const GROUP: ColumnKey = ColumnKey::new("group");
const SEVERITY: ColumnKey = ColumnKey::new("severity");

/// The dialog: its copy of the draft, the draft as last taken from or given to the app, and what a button asked.
pub struct ProfileDialog {
    id: DialogId,
    pub draft: ProfileDraft,
    synced: ProfileDraft,
    /// The `audit.closeProfileDialog` action a button asked for.
    pub request: Option<&'static str>,
    open: bool,
    /// The draft is gone: the dialog closes on its next pass.
    closing: bool,
}

impl ProfileDialog {
    pub fn new(draft: ProfileDraft) -> ProfileDialog {
        ProfileDialog { id: dialog_id("audit.profile"), synced: draft.clone(), draft, request: None, open: true, closing: false }
    }
}

/// The checks the table lists for the search, in group order: positions in the catalogue.
fn listed(d: &ProfileDraft) -> Vec<usize> {
    let needle = d.search.trim().to_lowercase();
    let mut out = Vec::new();
    for group in GROUPS {
        out.extend(
            d.catalogue
                .iter()
                .enumerate()
                .filter(|(_, c)| c.group == group)
                .filter(|(_, c)| needle.is_empty() || c.id.to_lowercase().contains(&needle) || c.label.to_lowercase().contains(&needle))
                .map(|(i, _)| i),
        );
    }
    out
}

struct Checks<'a> {
    draft: &'a ProfileDraft,
    rows: &'a [usize],
}

impl DisplayOrder<usize> for Checks<'_> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> usize {
        self.rows.get(at).copied().unwrap_or(usize::MAX)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.rows.get(at).and_then(|&i| self.draft.catalogue.get(i)).map(|c| Cow::Borrowed(c.label.as_str()))
    }
}

impl ItemSource<usize> for Checks<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == RUNS && self.rows.get(at).and_then(|&i| self.draft.catalogue.get(i)).is_some_and(|c| self.draft.runs(&c.id))
    }
}

fn severity_word(words: &AuditProfileWords, s: &str) -> Label {
    match s {
        "fail" => words.fail.clone(),
        "warn" => words.warn.clone(),
        "info" => words.info.clone(),
        other => other.to_string().into(),
    }
}

fn names_part(ui: &mut Ui, words: &AuditProfileWords, d: &mut ProfileDraft) {
    let editable = !d.read_only;
    Field::new(words.name.clone()).show(ui, |ui, _| {
        if d.original.is_none() && editable {
            TextField::new(words.name.clone(), Bind::bound(&mut d.id)).show(ui);
            d.id = d.id.chars().take(MAX_TEXT).collect();
        } else {
            Text::new(d.id.clone()).show(ui);
        }
    });
    Field::new(words.label.clone()).show(ui, |ui, _| {
        if editable {
            TextField::new(words.label.clone(), Bind::bound(&mut d.label)).show(ui);
            d.label = d.label.chars().take(MAX_TEXT).collect();
        } else {
            Text::new(d.label.clone()).show(ui);
        }
    });
    let stores = [Entry::new("user".to_string(), words.user.clone()), Entry::new("project".to_string(), words.project.clone())];
    if d.original.is_none() && editable {
        Segmented::new(words.storage.clone(), Bind::bound(&mut d.store), &stores).show(ui);
    } else {
        let store = if d.store == "project" { &words.project } else { &words.user };
        Field::new(words.storage.clone()).show(ui, |ui, _| {
            Text::new(store.clone()).show(ui);
        });
    }
}

fn include_part(ui: &mut Ui, words: &AuditProfileWords, d: &mut ProfileDraft) {
    Text::new(words.starts_from.clone()).show(ui);
    let offered: Vec<String> = d.profiles.iter().filter(|p| **p != d.id || d.read_only).cloned().collect();
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        for p in offered.iter().cloned() {
            let on = d.include.contains(&p);
            let out = Checkbox::new(Bind::given(on)).label(p.clone()).enabled(!d.read_only).show(ui);
            if let Some(now) = out.events.committed()
                && *now != on
            {
                if *now {
                    if d.include.len() < MAX_ENTRIES {
                        d.include.push(p.clone());
                    }
                } else {
                    d.include.retain(|i| *i != p);
                }
            }
        }
    });
}

fn checks_part(ui: &mut Ui, words: &AuditProfileWords, d: &mut ProfileDraft) {
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        Text::new(format!("{} ({} {} {} {})", words.checks.as_str(), d.count(), words.of.as_str(), d.catalogue.len(), words.run_count.as_str()))
            .show(ui);
        TextField::search(words.search.clone(), Bind::bound(&mut d.search)).placeholder(words.search.clone()).show(ui);
        d.search = d.search.chars().take(MAX_TEXT).collect();
    });
    let rows = listed(d);
    let selected = d.selected.as_ref().and_then(|s| d.catalogue.iter().position(|c| c.id == *s));
    let before = Selection::new(selected.into_iter().collect(), selected, selected);
    let mut after = before.clone();
    let columns = vec![
        ColumnSpec::toggle(RUNS, words.runs.clone(), ToggleColumn::sweepable()),
        ColumnSpec::new(CHECK, words.check.clone(), ColumnWidth::fill(1.0)),
        ColumnSpec::new(GROUP, words.group.clone(), ColumnWidth::Content),
        ColumnSpec::new(SEVERITY, words.severity.clone(), ColumnWidth::Content),
    ];
    let enabled = !d.read_only;
    let draft = &*d;
    let source = Checks { draft, rows: &rows };
    let out = Table::<usize>::new(words.checks.clone(), columns).max_rows(LIST_ROWS).selection(Bind::bound(&mut after)).show_items(
        ui,
        &source,
        &mut |at, icx| {
            let Some(c) = draft.catalogue.get(*at) else { return ListRow::new(String::new()) };
            let name = if c.label.is_empty() { c.id.clone() } else { c.label.clone() };
            let shown = match draft.override_of(&c.id) {
                "" => format!("{} ({})", words.default_severity.as_str(), severity_word(words, &c.severity).as_str()),
                s => severity_word(words, s).as_str().to_string(),
            };
            ListRow::new(name)
                .cells(icx.cells())
                .state(icx.row.clone())
                .toggle(ToggleCell::check(RUNS, ToggleState::of(draft.runs(&c.id)), words.runs.clone()))
                .cell(GROUP, c.group.clone())
                .cell(SEVERITY, shown)
        },
    );
    let mut flips = Vec::new();
    for event in &out.events {
        if let CollectionEvent::Toggle(edit) = event
            && matches!(edit.phase(), Some(GesturePhase::Commit) | None)
            && let Some(cell) = edit.value()
            && cell.column == RUNS
            && let Some(c) = d.catalogue.get(cell.key)
            && d.runs(&c.id) != cell.on
        {
            flips.push(c.id.clone());
        }
    }
    if enabled {
        for id in flips {
            d.toggle(&id);
        }
    }
    if after.lead != before.lead
        && let Some(c) = after.lead.and_then(|i| d.catalogue.get(i))
    {
        d.selected = Some(c.id.clone());
    }
}

fn selected_part(ui: &mut Ui, words: &AuditProfileWords, d: &mut ProfileDraft) {
    let Some(check) = d.selected.clone() else { return };
    let severity = d.catalogue.iter().find(|c| c.id == check).map(|c| c.severity.clone()).unwrap_or_default();
    let choices = [
        Entry::new(String::new(), format!("{} ({})", words.default_severity.as_str(), severity_word(words, &severity).as_str())),
        Entry::new("fail".to_string(), words.fail.clone()),
        Entry::new("warn".to_string(), words.warn.clone()),
        Entry::new("info".to_string(), words.info.clone()),
    ];
    let current = d.override_of(&check).to_string();
    Field::new(words.severity.clone()).show(ui, |ui, _| {
        let out = Select::new(words.severity.clone(), Bind::given(current.clone()), &choices).enabled(!d.read_only).show(ui);
        if let Some(picked) = out.events.committed()
            && *picked != current
        {
            d.set_override(&check, picked);
        }
    });
    Text::new(format!("{} {check}", words.thresholds.as_str())).show(ui);
    if d.thresholds.is_empty() {
        Text::new(words.no_thresholds.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    for t in d.thresholds.clone() {
        let value = d.config.get(&check).and_then(|m| m.get(&t.name)).copied();
        let _ = Row::new().fill_width().show(ui, |ui, _| {
            Field::new(t.name.clone()).show(ui, |ui, _| {
                if d.read_only {
                    Text::new(value.map_or_else(|| "--".to_string(), widgets::format_number)).show(ui);
                    return;
                }
                let field = NumberField::new(t.name.clone(), Bind::given(value)).id_salt(("fontcraftAuditProfile.threshold", t.name.as_str()));
                match widgets::clearable(ui, field, value) {
                    Some(NumberEdit::Set(v)) => d.set_threshold(&t.name, Some(v)),
                    Some(NumberEdit::Clear) => d.set_threshold(&t.name, None),
                    None => {}
                }
            });
            Text::new(format!("{} {}", words.default_value.as_str(), widgets::format_number(t.default))).shade(Shade::Muted).show(ui);
        });
    }
}

fn values_part(ui: &mut Ui, words: &AuditProfileWords, d: &mut ProfileDraft) {
    if d.values.is_empty() {
        return;
    }
    Text::new(words.values.clone()).show(ui);
    for v in d.values.clone() {
        let now = d.value_of(&v.name);
        Field::new(v.name.clone()).show(ui, |ui, _| {
            let edited: Option<Value> = match v.kind.as_str() {
                "true or false" => {
                    let on = now.as_bool().unwrap_or(false);
                    Checkbox::new(Bind::given(on)).label(v.name.clone()).show(ui).events.committed().map(|b| json!(*b))
                }
                "whole number" => {
                    let field = NumberField::new(v.name.clone(), Bind::given(now.as_f64())).id_salt(("fontcraftAuditProfile.value", v.name.as_str()));
                    match widgets::clearable(ui, field, now.as_f64()) {
                        Some(NumberEdit::Set(n)) => Some(json!(n.round() as i64)),
                        Some(NumberEdit::Clear) => Some(Value::Null),
                        None => None,
                    }
                }
                "list of table tags" => {
                    let shown = now.as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                    let mut text = shown.clone();
                    let out = TextField::new(v.name.clone(), Bind::bound(&mut text)).show(ui);
                    (out.events.committed().is_some() && text != shown).then(|| {
                        let tags: Vec<String> = text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(MAX_ENTRIES).collect();
                        if tags.is_empty() { Value::Null } else { json!(tags) }
                    })
                }
                _ => {
                    let shown = now.as_str().unwrap_or_default().to_string();
                    let mut text = shown.clone();
                    let out = TextField::new(v.name.clone(), Bind::bound(&mut text)).show(ui);
                    (out.events.committed().is_some() && text != shown)
                        .then(|| if text.is_empty() { Value::Null } else { json!(text.chars().take(MAX_TEXT).collect::<String>()) })
                }
            };
            if let Some(value) = edited {
                d.set_value(&v.name, value);
            }
        });
    }
}

impl StackDialog for ProfileDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<AuditProfileWords>();
        let read_only = self.draft.read_only;
        let saved = self.draft.original.is_some();
        let duplicate = FooterButton::new(words.duplicate.clone(), DialogRole::Other);
        let delete = FooterButton::new(words.delete.clone(), DialogRole::Destructive).enabled(saved);
        let ProfileDialog { draft, open, .. } = self;
        let mut value = false;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut value))
            .opened(Bind::bound(open))
            .ok_label(words.save.clone())
            .ok_enabled(!read_only)
            .accept_stays_open(true)
            .extra_buttons([duplicate, delete])
            .show(ui, |ui, _, _| {
                if draft.read_only {
                    Text::new(words.read_only.clone()).shade(Shade::Muted).show(ui);
                }
                names_part(ui, &words, draft);
                include_part(ui, &words, draft);
                checks_part(ui, &words, draft);
                selected_part(ui, &words, draft);
                values_part(ui, &words, draft);
                true
            });
        match shown.picked {
            Some(DUPLICATE) => self.draft = self.draft.duplicate(),
            Some(DELETE) => self.request = Some("delete"),
            Some(OK) if !read_only => self.request = Some("save"),
            _ if shown.accepted.is_some() && !read_only => self.request = Some("save"),
            _ => {}
        }
        if shown.closed.is_some() && shown.accepted.is_none() {
            self.request = Some("cancel");
        }
        None
    }
}

fn close(app: &mut FontCraftApp, action: &str) {
    app.sent.push(Msg::Shell(CommandRef::with_params(profile::CLOSE, json!({ "action": action }))));
}

/// Opens the dialog while the app has a draft and closes it when the draft is gone; takes the person's changes into
/// the app's draft and the commands' changes into the dialog; posts what the buttons ask.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    // The dialog closes only through the command, which drops the draft.
    let _ = host.take::<ProfileDialog>();
    let Some(app_draft) = app.view.font_audit.draft.clone() else {
        if let Some(dialog) = host.get_mut::<ProfileDialog>() {
            dialog.closing = true;
        }
        return;
    };
    let Some(dialog) = host.get_mut::<ProfileDialog>() else {
        host.open(ProfileDialog::new(app_draft));
        return;
    };
    if let Some(action) = dialog.request.take() {
        // Save and Delete send the dialog's draft as the app's first, so the command saves what shows.
        app.view.font_audit.draft = Some(dialog.draft.clone());
        dialog.synced = dialog.draft.clone();
        if !dialog.open {
            dialog.open = true;
        }
        close(app, action);
        return;
    }
    if app_draft != dialog.synced {
        dialog.draft = app_draft.clone();
        dialog.synced = app_draft;
        return;
    }
    if dialog.draft == dialog.synced {
        return;
    }
    let mut next = dialog.draft.clone();
    if next.selected != dialog.synced.selected {
        next.thresholds = next.selected.clone().map(|check| thresholds_of(app, &check)).unwrap_or_default();
    }
    if next.include != dialog.synced.include {
        next.base = base_of(app, &next.include);
    }
    if let Some(dialog) = host.get_mut::<ProfileDialog>() {
        dialog.draft = next.clone();
        dialog.synced = next.clone();
    }
    app.view.font_audit.draft = Some(next);
}

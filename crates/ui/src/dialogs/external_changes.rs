//! The External Changes dialog: shows `shell::external_changes`' state while it is open and answers through
//! `dialog.externalChanges` actions. First the question (Review…, Reject All, Update All), then the item list of the
//! selected font: a ticked row is accepted, activating a row shows its glyph's stored text.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Entry, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::table::Table;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::external_changes::{COMMAND, ExternalChanges, Item, Line};

craft::ui::words! {
    /// The words of the External Changes dialog.
    pub struct ExternalChangesWords {
        title: "fontcraftExternalChanges.title" = "External Changes";
        changed_one: "fontcraftExternalChanges.changedOne" = "was changed on disk by another program.";
        changed_many: "fontcraftExternalChanges.changedMany" = "open fonts were changed on disk by another program:";
        unsaved: "fontcraftExternalChanges.unsaved" = "Unsaved edits are kept; only the changes made on disk are offered.";
        review: "fontcraftExternalChanges.review" = "Review…";
        reject_all: "fontcraftExternalChanges.rejectAll" = "Reject All";
        update_all: "fontcraftExternalChanges.updateAll" = "Update All";
        accept_selected: "fontcraftExternalChanges.acceptSelected" = "Accept Selected";
        fonts: "fontcraftExternalChanges.fonts" = "Font";
        items: "fontcraftExternalChanges.items" = "Changed items";
        section: "fontcraftExternalChanges.section" = "Part";
        item: "fontcraftExternalChanges.item" = "Item";
        none: "fontcraftExternalChanges.none" = "Nothing to accept.";
        text: "fontcraftExternalChanges.text" = "the file's lines (+) and the document's (−)";
        text_list: "fontcraftExternalChanges.textList" = "Stored text";
        op: "fontcraftExternalChanges.op" = "Change";
        line: "fontcraftExternalChanges.line" = "Line";
        hint: "fontcraftExternalChanges.hint" = "Untick what to keep as it is; activate an item to see its glyph's text.";
    }
}

/// Rows the lists show before they scroll: the dialog body scrolls too, so the lists must be capped.
const LIST_ROWS: u16 = 12;
/// The footer button positions `FormDialog` reports.
const OK: usize = 0;
const FIRST_EXTRA: usize = 2;

const SECTION: ColumnKey = ColumnKey::new("section");
const ITEM: ColumnKey = ColumnKey::new("item");
const OP: ColumnKey = ColumnKey::new("op");
const LINE: ColumnKey = ColumnKey::new("line");

/// The dialog: a copy of the app's state, the ticked rows and the actions to post.
pub struct ExternalDialog {
    id: DialogId,
    state: ExternalChanges,
    /// The selected font's items with the heading of each, in list order.
    items: Vec<(String, Item)>,
    /// Ticked rows, as positions in `items`.
    ticked: Selection<usize>,
    font: usize,
    /// `dialog.externalChanges` parameters to post.
    requests: Vec<Value>,
    /// The app closed the dialog: the next pass ends it.
    closing: bool,
    open: bool,
}

impl ExternalDialog {
    fn new(state: &ExternalChanges) -> ExternalDialog {
        let mut dialog = ExternalDialog {
            id: dialog_id("fontcraft.externalChanges"),
            state: ExternalChanges::default(),
            items: Vec::new(),
            ticked: Selection::default(),
            font: 0,
            requests: Vec::new(),
            closing: false,
            open: true,
        };
        dialog.take(state);
        dialog
    }

    /// Shows `state` from now on.
    fn take(&mut self, state: &ExternalChanges) {
        let mut heading = String::new();
        self.items = state
            .lines
            .iter()
            .filter_map(|line| match line {
                Line::Header(title) => {
                    heading = title.clone();
                    None
                }
                Line::Item(item) => Some((heading.clone(), item.clone())),
            })
            .collect();
        let ticked: Vec<usize> =
            self.items.iter().enumerate().filter(|(_, (_, item))| !state.unticked.contains(&item.key)).map(|(at, _)| at).collect();
        self.ticked = Selection::new(ticked, None, None);
        self.font = state.selected;
        self.state = state.clone();
    }

    /// The tick actions that turn `before` into the dialog's ticked rows.
    fn ticks(&self, before: &Selection<usize>) -> Vec<Value> {
        let keys = |on: bool| -> Vec<String> {
            self.items
                .iter()
                .enumerate()
                .filter(|(at, _)| self.ticked.items.contains(at) == on && before.items.contains(at) != on)
                .map(|(_, (_, item))| item.key.clone())
                .collect()
        };
        let (on, off) = (keys(true), keys(false));
        let mut out = Vec::new();
        if !on.is_empty() {
            out.push(json!({"action": "tick", "on": true, "keys": on}));
        }
        if !off.is_empty() {
            out.push(json!({"action": "tick", "on": false, "keys": off}));
        }
        out
    }

    fn question(&mut self, ui: &mut Ui, words: &ExternalChangesWords) -> Option<DialogResult<Vec<Value>>> {
        let titles: Vec<&str> = self.state.fonts.iter().map(|f| f.title.as_str()).collect();
        let message = match titles.as_slice() {
            [one] => format!("{one} {}", words.changed_one.as_str()),
            _ => format!("{} {} {}", titles.len(), words.changed_many.as_str(), titles.join(", ")),
        };
        let unsaved = self.state.fonts.iter().any(|f| f.modified);
        let note = self.state.note.clone();
        let mut value = false;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut value))
            .opened(Bind::bound(&mut self.open))
            .ok_label(words.update_all.clone())
            .cancel_label(words.reject_all.clone())
            .accept_stays_open(true)
            .extra_buttons([FooterButton::new(words.review.clone(), DialogRole::Other)])
            .show(ui, |ui, _, _| {
                Text::new(message.clone()).show(ui);
                if unsaved {
                    Text::new(words.unsaved.clone()).shade(Shade::Muted).show(ui);
                }
                if let Some(note) = &note {
                    Text::new(note.clone()).show(ui);
                }
                true
            });
        if shown.picked == Some(FIRST_EXTRA) {
            self.requests.push(json!({"action": "review"}));
        } else if shown.picked == Some(OK) || shown.accepted.is_some() {
            self.requests.push(json!({"action": "updateAll"}));
        } else if shown.closed.is_some() {
            self.requests.push(json!({"action": "rejectAll"}));
            return Some(DialogResult::Ok(std::mem::take(&mut self.requests)));
        }
        None
    }

    fn review(&mut self, ui: &mut Ui, words: &ExternalChangesWords) -> Option<DialogResult<Vec<Value>>> {
        let before = self.ticked.clone();
        let fonts: Vec<Entry<usize>> = self.state.fonts.iter().enumerate().map(|(at, f)| Entry::new(at, f.title.clone())).collect();
        let note = self.state.note.clone();
        let text = self.state.text.clone();
        let mut activated = None;
        let ExternalDialog { items, ticked, font, open, .. } = self;
        let list: &[(String, Item)] = items;
        let mut value = false;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut value))
            .opened(Bind::bound(open))
            .ok_label(words.accept_selected.clone())
            .cancel_label(words.reject_all.clone())
            .accept_stays_open(true)
            .show(ui, |ui, _, _| {
                if fonts.len() > 1 {
                    Segmented::new(words.fonts.clone(), Bind::bound(&mut *font), &fonts).show(ui);
                }
                Text::new(words.hint.clone()).shade(Shade::Muted).show(ui);
                let columns = vec![
                    ColumnSpec::new(SECTION, words.section.clone(), ColumnWidth::Content),
                    ColumnSpec::new(ITEM, words.item.clone(), ColumnWidth::Content),
                ];
                let positions: Vec<usize> = (0..list.len()).collect();
                let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(list.get(*at).map_or_else(String::new, |(_, i)| i.label.clone())));
                let out = Table::<usize>::new(words.items.clone(), columns)
                    .max_rows(LIST_ROWS)
                    .select(SelectMode::Multi)
                    .selection(Bind::bound(&mut *ticked))
                    .empty(words.none.clone())
                    .show_items(ui, &source, &mut |at, icx| {
                        let Some((section, item)) = list.get(*at) else { return ListRow::new("") };
                        ListRow::new(section.clone()).cells(icx.cells()).cell(ITEM, item.label.clone())
                    });
                for event in &out.events {
                    if let CollectionEvent::Activate(at) = event {
                        activated = list.get(*at).and_then(|(_, item)| item.glyph.clone());
                    }
                }
                if let Some((glyph, lines)) = &text {
                    Text::new(format!("{glyph}: {}", words.text.as_str())).show(ui);
                    let columns = vec![
                        ColumnSpec::new(OP, words.op.clone(), ColumnWidth::Content),
                        ColumnSpec::new(LINE, words.line.clone(), ColumnWidth::Content),
                    ];
                    let rows: Vec<usize> = (0..lines.len()).collect();
                    let source = keyed(&rows, |at| *at).with_text(|at| Cow::Owned(lines.get(*at).map_or_else(String::new, |(_, t)| t.clone())));
                    let _ = Table::<usize>::new(words.text_list.clone(), columns).max_rows(LIST_ROWS).show_items(ui, &source, &mut |at, icx| {
                        let Some((op, line)) = lines.get(*at) else { return ListRow::new("") };
                        ListRow::new(op.clone()).cells(icx.cells()).cell(LINE, line.clone())
                    });
                }
                if let Some(note) = &note {
                    Text::new(note.clone()).show(ui);
                }
                true
            });
        if self.font != self.state.selected {
            self.requests.push(json!({"action": "selectFont", "index": self.font}));
        }
        let ticks = self.ticks(&before);
        self.requests.extend(ticks);
        if let Some(glyph) = activated {
            self.requests.push(json!({"action": "showText", "glyph": glyph}));
        }
        if shown.picked == Some(OK) || shown.accepted.is_some() {
            self.requests.push(json!({"action": "acceptSelected"}));
        } else if shown.closed.is_some() {
            self.requests.push(json!({"action": "rejectAll"}));
            return Some(DialogResult::Ok(std::mem::take(&mut self.requests)));
        }
        None
    }
}

impl StackDialog for ExternalDialog {
    /// The actions still to post when the dialog ends.
    type Result = Vec<Value>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Vec<Value>>> {
        if self.closing {
            return Some(DialogResult::Ok(Vec::new()));
        }
        let words = Env::of(ui).words::<ExternalChangesWords>();
        if self.state.reviewing { self.review(ui, &words) } else { self.question(ui, &words) }
    }
}

/// Opens the dialog while the app's state asks for it, shows each change of that state, and posts what the person
/// did as `dialog.externalChanges` actions.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    let post = |app: &mut FontCraftApp, requests: Vec<Value>| {
        for params in requests {
            app.sent.push(Msg::Shell(CommandRef::with_params(COMMAND.id, params)));
        }
    };
    if let Some(answer) = host.take::<ExternalDialog>() {
        match answer {
            DialogResult::Ok(requests) => post(app, requests),
            // Dismissed by the window: the documents stay as they are.
            _ if app.view.extensions.external.open => post(app, vec![json!({"action": "close"})]),
            _ => {}
        }
        return;
    }
    let state = &app.view.extensions.external;
    match host.get_mut::<ExternalDialog>() {
        Some(dialog) => {
            let requests = std::mem::take(&mut dialog.requests);
            if !state.open {
                dialog.closing = true;
            } else if dialog.state.revision != state.revision {
                dialog.take(state);
            }
            post(app, requests);
        }
        None if state.open => {
            host.open(ExternalDialog::new(state));
        }
        None => {}
    }
}

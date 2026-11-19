//! The Kerning window's questions.
//!
//! - Rename Kerning Group (`dialog.renameKerningGroup`): asked after a group name is edited. The new name stays
//!   editable. All Pairs runs `kerning.renameGroup` with scope `font`, which renames the group on every glyph and in
//!   every pair; This Pair runs it with scope `pair`, which points only that pair's side at the new name. Cancel
//!   renames nothing.
//! - Paste Kerning: existing pairs (`dialog.pasteKerningConflict`): asked after a paste found pasted pairs the
//!   master stores with other values, and so pasted nothing. It lists them; Overwrite pastes again replacing them,
//!   Keep Existing pastes only the pairs the master lacks, Cancel pastes nothing.
//!
//! Nothing runs until a button is pressed.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, keyed};
use craft::ui::contract::{Bind, ColumnSpec, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Env;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::panels::kerning_window::format_value;
use crate::shell::kerning_window::{Conflict, Question};

craft::ui::words! {
    /// The words of the Kerning window's questions.
    pub struct KerningQuestionWords {
        rename_title: "fontcraftKerningQuestion.renameTitle" = "Rename Kerning Group";
        rename_message: "fontcraftKerningQuestion.renameMessage" = "Rename the group in the whole font, or only for the selected pair?";
        new_name: "fontcraftKerningQuestion.newName" = "New name for the group";
        all_pairs: "fontcraftKerningQuestion.allPairs" = "All Pairs";
        this_pair: "fontcraftKerningQuestion.thisPair" = "This Pair";
        paste_title: "fontcraftKerningQuestion.pasteTitle" = "Paste Kerning: existing pairs";
        paste_message: "fontcraftKerningQuestion.pasteMessage" = "Some pasted pairs exist in this master with other values.";
        conflicts: "fontcraftKerningQuestion.conflicts" = "Existing pairs";
        listed_first: "fontcraftKerningQuestion.listedFirst" = "pairs exist with other values; listed:";
        left: "fontcraftKerningQuestion.left" = "Left";
        right: "fontcraftKerningQuestion.right" = "Right";
        pasted: "fontcraftKerningQuestion.pasted" = "Pasted";
        existing: "fontcraftKerningQuestion.existing" = "Now";
        overwrite: "fontcraftKerningQuestion.overwrite" = "Overwrite";
        keep_existing: "fontcraftKerningQuestion.keepExisting" = "Keep Existing";
    }
}

/// The picked index of the first extra footer button (`FormOutput::picked`).
const FIRST_EXTRA: usize = 2;
/// Rows a dialog's list shows before it scrolls: the dialog body around it scrolls too.
const LIST_ROWS: u16 = 12;

/// Rename Kerning Group over `kerning.renameGroup`'s parameters but `scope`.
pub struct RenameGroup {
    id: DialogId,
    pub params: Map<String, Value>,
    /// The new name, without @.
    pub to: String,
    open: bool,
}

impl RenameGroup {
    pub fn new(params: Map<String, Value>) -> RenameGroup {
        let to = params.get("to").and_then(Value::as_str).unwrap_or_default().to_string();
        RenameGroup { id: dialog_id("kerning.renameGroup"), params, to, open: true }
    }

    fn from(&self) -> &str {
        self.params.get("from").and_then(Value::as_str).unwrap_or_default()
    }

    /// `kerning.renameGroup`'s parameters for the answer `scope`.
    pub fn answer(&self, to: &str, scope: &str) -> Value {
        let mut p = self.params.clone();
        p.insert("to".into(), json!(to.trim().trim_start_matches('@')));
        p.insert("scope".into(), json!(scope));
        Value::Object(p)
    }
}

impl StackDialog for RenameGroup {
    type Result = Value;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Value>> {
        let words = Env::of(ui).words::<KerningQuestionWords>();
        let label = format!("{} @{}", words.new_name.as_str(), self.from());
        let this_pair = FooterButton::new(words.this_pair.clone(), DialogRole::Other);
        let RenameGroup { to, open, .. } = self;
        let shown = FormDialog::new(words.rename_title.clone(), Bind::bound(to))
            .opened(Bind::bound(open))
            .ok_label(words.all_pairs.clone())
            .extra_buttons([this_pair])
            .show(ui, |ui, _, to| {
                Text::new(words.rename_message.clone()).show(ui);
                TextField::new(label.clone(), Bind::bound(to)).show(ui);
                !to.trim().trim_start_matches('@').is_empty()
            });
        if shown.picked == Some(FIRST_EXTRA) && !self.to.trim().trim_start_matches('@').is_empty() {
            return Some(DialogResult::Ok(self.answer(&self.to, "pair")));
        }
        match (shown.accepted, shown.closed) {
            (Some(to), _) => Some(DialogResult::Ok(self.answer(&to, "font"))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Paste Kerning: existing pairs, over the paste's parameters but `onConflict`.
pub struct PasteConflict {
    id: DialogId,
    pub params: Map<String, Value>,
    pub conflicts: Vec<Conflict>,
    pub count: usize,
    seen: bool,
    open: bool,
}

impl PasteConflict {
    pub fn new(params: Map<String, Value>, conflicts: Vec<Conflict>, count: usize) -> PasteConflict {
        PasteConflict { id: dialog_id("kerning.pasteConflict"), params, conflicts, count, seen: true, open: true }
    }

    /// `kerning.pastePairs`' parameters for the answer `on_conflict`.
    pub fn answer(&self, on_conflict: &str) -> Value {
        let mut p = self.params.clone();
        p.insert("onConflict".into(), json!(on_conflict));
        Value::Object(p)
    }
}

const LEFT: ColumnKey = ColumnKey::new("left");
const RIGHT: ColumnKey = ColumnKey::new("right");
const PASTED: ColumnKey = ColumnKey::new("pasted");
const EXISTING: ColumnKey = ColumnKey::new("existing");

impl StackDialog for PasteConflict {
    type Result = Value;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Value>> {
        let words = Env::of(ui).words::<KerningQuestionWords>();
        let keep = FooterButton::new(words.keep_existing.clone(), DialogRole::Other);
        let PasteConflict { conflicts, count, seen, open, .. } = self;
        let (conflicts, count) = (&*conflicts, *count);
        let shown = FormDialog::new(words.paste_title.clone(), Bind::bound(seen))
            .opened(Bind::bound(open))
            .ok_label(words.overwrite.clone())
            .extra_buttons([keep])
            .show(ui, |ui, _, _| {
                Text::new(words.paste_message.clone()).show(ui);
                if count > conflicts.len() {
                    Text::new(format!("{count} {} {}", words.listed_first.as_str(), conflicts.len())).show(ui);
                }
                let columns = vec![
                    ColumnSpec::new(LEFT, words.left.clone(), ColumnWidth::Content),
                    ColumnSpec::new(RIGHT, words.right.clone(), ColumnWidth::Content),
                    ColumnSpec::new(PASTED, words.pasted.clone(), ColumnWidth::Content).numeric(),
                    ColumnSpec::new(EXISTING, words.existing.clone(), ColumnWidth::Content).numeric(),
                ];
                let positions: Vec<usize> = (0..conflicts.len()).collect();
                let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(conflicts.get(*at).map_or_else(String::new, |c| c.left.clone())));
                let _ = Table::<usize>::new(words.conflicts.clone(), columns).max_rows(LIST_ROWS).show_items(ui, &source, &mut |at, icx| {
                    let Some(c) = conflicts.get(*at) else { return ListRow::new("") };
                    ListRow::new(c.left.clone())
                        .cells(icx.cells())
                        .cell(RIGHT, c.right.clone())
                        .cell(PASTED, format_value(c.value))
                        .cell(EXISTING, format_value(c.existing))
                });
                true
            });
        if shown.picked == Some(FIRST_EXTRA) {
            return Some(DialogResult::Ok(self.answer("keepExisting")));
        }
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(self.answer("overwrite"))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the question the window asked, and runs the command its answer names.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    match app.view.kerning.question.take() {
        Some(Question::Rename(params)) => {
            host.open(RenameGroup::new(params));
        }
        Some(Question::Paste { params, conflicts, count }) => {
            host.open(PasteConflict::new(params, conflicts, count));
        }
        None => {}
    }
    if let Some(DialogResult::Ok(params)) = host.take::<RenameGroup>() {
        app.sent.push(Msg::run("kerning.renameGroup", params));
    }
    if let Some(DialogResult::Ok(params)) = host.take::<PasteConflict>() {
        app.sent.push(Msg::run("kerning.pastePairs", params));
    }
}

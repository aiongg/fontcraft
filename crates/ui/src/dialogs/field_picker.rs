//! Add Field: the plus button beside a General heading opens it. It lists the fields `font.info.listFields` offers
//! for the tab's scope that the targets do not have yet; a click selects one, Cmd-click or Shift-click adds more, and
//! Add posts `font.info.addField` for each selected field, in list order. Cancel and Escape add nothing.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::contract::{Bind, ColumnSpec, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::font_info::{InfoDialog, Target};

craft::ui::words! {
    /// The words of the Add Field picker.
    pub struct FieldPickerWords {
        title: "fontcraftFieldPicker.title" = "Add Field";
        add: "fontcraftFieldPicker.add" = "Add";
        list: "fontcraftFieldPicker.list" = "Fields";
        field: "fontcraftFieldPicker.field" = "Field";
        key: "fontcraftFieldPicker.key" = "Key";
        localizable: "fontcraftFieldPicker.localizable" = "Localizable";
        yes: "fontcraftFieldPicker.yes" = "Yes";
        all_there: "fontcraftFieldPicker.allThere" = "Every field is already there.";
        loading: "fontcraftFieldPicker.loading" = "Loading…";
    }
}

/// Most fields the list holds.
const MAX_FIELDS: usize = 1024;

const NAME: ColumnKey = ColumnKey::new("name");
const KEY: ColumnKey = ColumnKey::new("key");
const LOCALIZABLE: ColumnKey = ColumnKey::new("localizable");

/// One field the picker offers.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldChoice {
    pub key: String,
    pub label: String,
    pub localizable: bool,
}

/// The picker: what it adds to, the fields it offers and the selected ones (positions in `choices`).
pub struct FieldPicker {
    id: DialogId,
    pub target: Target,
    /// `None` until the list query has run.
    pub choices: Option<Vec<FieldChoice>>,
    /// Why the list could not be made.
    pub error: Option<String>,
    pub selection: Selection<usize>,
    open: bool,
}

impl FieldPicker {
    pub fn new(target: Target) -> FieldPicker {
        FieldPicker { id: dialog_id("fontInfo.fieldPicker"), target, choices: None, error: None, selection: Selection::default(), open: true }
    }

    /// The parameters of the `font.info.listFields` query that fills the list. With one target it marks what that
    /// target has; with several, what any of them has.
    pub fn query(&self) -> Value {
        let mut p = serde_json::Map::new();
        p.insert("scope".into(), json!(self.target.scope.name()));
        if let [one] = self.target.indices.as_slice()
            && self.target.below_font()
        {
            p.insert("target".into(), json!(one));
        }
        Value::Object(p)
    }

    /// Takes the query's result: every field the targets do not have, in the query's order.
    pub fn fill(&mut self, result: &Value) {
        let fields = result.get("fields").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let choices = fields
            .iter()
            .filter(|f| !f.get("present").and_then(Value::as_bool).unwrap_or(false))
            .filter_map(|f| {
                Some(FieldChoice {
                    key: f.get("key")?.as_str()?.to_string(),
                    label: f.get("label").and_then(Value::as_str).unwrap_or_default().to_string(),
                    localizable: f.get("localizable").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .take(MAX_FIELDS)
            .collect();
        self.choices = Some(choices);
    }

    /// The keys of the selected fields, in list order.
    pub fn picked(&self, selection: &Selection<usize>) -> Vec<String> {
        let choices = self.choices.as_deref().unwrap_or_default();
        choices.iter().enumerate().filter(|(at, _)| selection.items.contains(at)).map(|(_, c)| c.key.clone()).collect()
    }
}

impl StackDialog for FieldPicker {
    type Result = (Target, Vec<String>);

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<FieldPickerWords>();
        let FieldPicker { choices, error, selection, open, .. } = self;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(selection)).opened(Bind::bound(open)).ok_label(words.add.clone()).show(
            ui,
            |ui, _, selection| {
                match (choices.as_deref(), error.as_deref()) {
                    (_, Some(error)) => {
                        Text::new(error.to_string()).show(ui);
                    }
                    (None, None) => {
                        Text::new(words.loading.clone()).shade(Shade::Muted).show(ui);
                    }
                    (Some([]), None) => {
                        Text::new(words.all_there.clone()).shade(Shade::Muted).show(ui);
                    }
                    (Some(list), None) => {
                        let columns = vec![
                            ColumnSpec::new(NAME, words.field.clone(), ColumnWidth::Content),
                            ColumnSpec::new(KEY, words.key.clone(), ColumnWidth::Content),
                            ColumnSpec::new(LOCALIZABLE, words.localizable.clone(), ColumnWidth::Content),
                        ];
                        let positions: Vec<usize> = (0..list.len()).collect();
                        let source =
                            keyed(&positions, |at| *at).with_text(|at| Cow::Owned(list.get(*at).map_or_else(String::new, |c| c.label.clone())));
                        let _ = Table::<usize>::new(words.list.clone(), columns)
                            .select(SelectMode::Multi)
                            .selection(Bind::bound(selection))
                            .show_items(ui, &source, &mut |at, icx| {
                                let Some(choice) = list.get(*at) else { return ListRow::new("") };
                                let row = ListRow::new(choice.label.clone()).cells(icx.cells()).cell(KEY, choice.key.clone());
                                if choice.localizable { row.cell(LOCALIZABLE, words.yes.clone()) } else { row }
                            });
                    }
                }
                !selection.items.is_empty()
            },
        );
        match (shown.accepted, shown.closed) {
            (Some(selection), _) => Some(DialogResult::Ok((self.target.clone(), self.picked(&selection)))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the picker a row asked for, fills its list, and posts what Add picked.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(InfoDialog::FieldPicker(target)) = &app.view.font_info.request {
        let picker = FieldPicker::new(target.clone());
        app.view.font_info.request = None;
        host.open(picker);
    }
    if let Some(picker) = host.get_mut::<FieldPicker>()
        && picker.choices.is_none()
        && picker.error.is_none()
    {
        match app.session.query("font.info.listFields", &picker.query()) {
            Ok(result) => picker.fill(&result),
            Err(e) => picker.error = Some(e.to_string()),
        }
    }
    if let Some(DialogResult::Ok((target, keys))) = host.take::<FieldPicker>() {
        for key in keys {
            let mut p = target.params();
            p.insert("key".into(), json!(key));
            app.sent.push(Msg::run("font.info.addField", Value::Object(p)));
        }
    }
}

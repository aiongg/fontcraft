//! Add Feature: the plus button beside the Features heading of Font Info opens it. Typing in its search field filters
//! the registered feature tags by tag or name (`font.features.listAvailable`); Up and Down move through the list from
//! the field. Add, Return or a double-click adds the selected feature with `font.features.addFeature`, generated
//! automatically when Update could write its code for the font. Tags the font already has are marked and cannot be
//! added. Cancel and Escape add nothing.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::list::{ListRow, RowInk};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::font_info::InfoDialog;

craft::ui::words! {
    /// The words of the Add Feature picker.
    pub struct FeaturePickerWords {
        title: "fontcraftFeaturePicker.title" = "Add Feature";
        add: "fontcraftFeaturePicker.add" = "Add";
        search: "fontcraftFeaturePicker.search" = "Search features";
        list: "fontcraftFeaturePicker.list" = "Features";
        tag: "fontcraftFeaturePicker.tag" = "Tag";
        name: "fontcraftFeaturePicker.name" = "Name";
        in_font: "fontcraftFeaturePicker.inFont" = "In the font";
        none: "fontcraftFeaturePicker.none" = "No registered feature matches.";
        loading: "fontcraftFeaturePicker.loading" = "Loading…";
    }
}

/// Most characters of the search text.
const MAX_QUERY_CHARS: usize = 100;
/// Rows the list shows before it scrolls.
const LIST_ROWS: u16 = 12;
/// Most features listed.
const MAX_CHOICES: usize = 2_000;

const TAG: ColumnKey = ColumnKey::new("tag");
const NAME: ColumnKey = ColumnKey::new("name");
const IN_FONT: ColumnKey = ColumnKey::new("inFont");

/// One registered feature the picker offers.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureChoice {
    pub tag: String,
    pub name: String,
    /// The font has a feature with this tag already.
    pub in_font: bool,
    /// Update could write its code for this font.
    pub can_generate: bool,
}

/// The picker: the search text, the list made for it and the selected choice.
pub struct FeaturePicker {
    id: DialogId,
    pub query: String,
    /// The search text the list was made for; `None` before the first listing.
    pub listed: Option<String>,
    pub choices: Vec<FeatureChoice>,
    pub error: Option<String>,
    pub selection: Selection<usize>,
    /// A double-click or Return on a row: it adds that row.
    activated: Option<usize>,
    open: bool,
}

impl Default for FeaturePicker {
    fn default() -> Self {
        FeaturePicker {
            id: dialog_id("fontInfo.featurePicker"),
            query: String::new(),
            listed: None,
            choices: Vec::new(),
            error: None,
            selection: Selection::default(),
            activated: None,
            open: true,
        }
    }
}

impl FeaturePicker {
    /// The list is missing or older than the search text.
    pub fn needs_listing(&self) -> bool {
        self.listed.as_deref() != Some(self.query.as_str())
    }

    /// The parameters of the `font.features.listAvailable` query that fills the list.
    pub fn query_params(&self) -> Value {
        let query: String = self.query.trim().chars().take(MAX_QUERY_CHARS).collect();
        if query.is_empty() { json!({}) } else { json!({ "query": query }) }
    }

    /// Takes the query's result; the selection goes to the first feature that can be added.
    pub fn fill(&mut self, result: &Value) {
        let features = result.get("features").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        self.choices = features
            .iter()
            .take(MAX_CHOICES)
            .filter_map(|f| {
                Some(FeatureChoice {
                    tag: f.get("tag")?.as_str()?.to_string(),
                    name: f.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    in_font: f.get("inFont").and_then(Value::as_bool).unwrap_or(false),
                    can_generate: f.get("canGenerate").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        let first = self.choices.iter().position(|c| !c.in_font);
        self.selection = first.map_or_else(Selection::default, |at| Selection::new(vec![at], Some(at), Some(at)));
        self.listed = Some(self.query.clone());
        self.error = None;
    }

    /// Takes a refused query: the message, and no list.
    pub fn fail(&mut self, message: String) {
        self.choices.clear();
        self.selection = Selection::default();
        self.error = Some(message);
        self.listed = Some(self.query.clone());
    }

    /// What adding the choice at `at` runs: `font.features.addFeature`, automatic when Update could write its code.
    /// `None` for a tag the font has.
    pub fn command(&self, at: usize) -> Option<Msg> {
        let c = self.choices.get(at).filter(|c| !c.in_font)?;
        Some(Msg::run("font.features.addFeature", json!({ "tag": c.tag, "automatic": c.can_generate })))
    }
}

impl StackDialog for FeaturePicker {
    type Result = Msg;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<FeaturePickerWords>();
        let list_id = ui.make_persistent_id("fontcraftFeaturePicker.list");
        let FeaturePicker { query, listed, choices, error, selection, activated, open, .. } = self;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(selection)).opened(Bind::bound(open)).ok_label(words.add.clone()).show(
            ui,
            |ui, _, selection| {
                TextField::search(words.search.clone(), Bind::bound(query)).limit(MAX_QUERY_CHARS).drives(list_id).show(ui);
                match (listed.as_ref(), error.as_deref()) {
                    (_, Some(error)) => {
                        Text::new(error.to_string()).show(ui);
                    }
                    (None, None) => {
                        Text::new(words.loading.clone()).shade(Shade::Muted).show(ui);
                    }
                    (Some(_), None) if choices.is_empty() => {
                        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
                    }
                    (Some(_), None) => {
                        let columns = vec![
                            ColumnSpec::new(TAG, words.tag.clone(), ColumnWidth::Content),
                            ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::fill(1.0)),
                            ColumnSpec::new(IN_FONT, words.in_font.clone(), ColumnWidth::Content),
                        ];
                        let positions: Vec<usize> = (0..choices.len()).collect();
                        let source = keyed(&positions, |at| *at)
                            .with_text(|at| Cow::Owned(choices.get(*at).map_or_else(String::new, |c| format!("{} {}", c.tag, c.name))));
                        let out = Table::<usize>::new(words.list.clone(), columns)
                            .id(list_id)
                            .max_rows(LIST_ROWS)
                            .select(SelectMode::Single)
                            .selection(Bind::bound(selection))
                            .show_items(ui, &source, &mut |at, icx| {
                                let Some(c) = choices.get(*at) else { return ListRow::new("") };
                                let row = ListRow::new(c.tag.clone()).cells(icx.cells()).cell(NAME, c.name.clone());
                                if c.in_font { row.cell(IN_FONT, words.in_font.clone()).ink(RowInk::Dimmed) } else { row }
                            });
                        for event in &out.events {
                            if let CollectionEvent::Activate(at) = event {
                                *activated = Some(*at);
                            }
                        }
                    }
                }
                selection.lead.or_else(|| selection.items.first().copied()).and_then(|at| choices.get(at)).is_some_and(|c| !c.in_font)
            },
        );
        if let Some(command) = self.activated.take().and_then(|at| self.command(at)) {
            return Some(DialogResult::Ok(command));
        }
        match (shown.accepted, shown.closed) {
            (Some(selection), _) => {
                let at = selection.lead.or_else(|| selection.items.first().copied());
                Some(at.and_then(|at| self.command(at)).map_or(DialogResult::Cancel, DialogResult::Ok))
            }
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the picker the Features tab asked for, fills its list for the search text, and posts what Add picked.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if app.view.font_info.request == Some(InfoDialog::FeaturePicker) {
        app.view.font_info.request = None;
        host.open(FeaturePicker::default());
    }
    if let Some(picker) = host.get_mut::<FeaturePicker>()
        && picker.needs_listing()
    {
        match app.session.query("font.features.listAvailable", &picker.query_params()) {
            Ok(result) => picker.fill(&result),
            Err(e) => picker.fail(e.to_string()),
        }
    }
    if let Some(DialogResult::Ok(command)) = host.take::<FeaturePicker>() {
        app.sent.push(command);
    }
}

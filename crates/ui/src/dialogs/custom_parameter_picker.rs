//! Add Custom Parameter: the plus button beside a Custom Parameters heading opens it. A search field narrows the list
//! of parameters allowed at the tab's scope (`font.customParameters.describe`, which matches names, other spellings
//! and descriptions in any case); the list is in alphabetical order. The last selected parameter's description shows
//! under the list. Add posts `font.customParameters.add` for each selected parameter, in list order.
//!
//! The Project panel's plus (`window.project.addCustomParameter`) opens the same picker on the selected entries of a
//! project's instance list; there Add posts one `project.editInstance` per entry, adding every picked parameter.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::contract::{Bind, ColumnSpec, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::fontinfo::Scope;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::font_info::{InfoDialog, Target};

craft::ui::words! {
    /// The words of the Add Custom Parameter picker.
    pub struct ParameterPickerWords {
        title: "fontcraftParameterPicker.title" = "Add Custom Parameter";
        add: "fontcraftParameterPicker.add" = "Add";
        search: "fontcraftParameterPicker.search" = "Search parameters";
        list: "fontcraftParameterPicker.list" = "Parameters";
        name: "fontcraftParameterPicker.name" = "Name";
        kind: "fontcraftParameterPicker.kind" = "Kind";
        none: "fontcraftParameterPicker.none" = "No parameter matches the search.";
    }
}

/// Longest search text, in characters.
pub const MAX_QUERY_CHARS: usize = 256;
/// Most parameters the list holds.
const MAX_PARAMETERS: usize = 2048;
/// Rows the list shows before it scrolls: the dialog body scrolls too, so the list must be capped.
const LIST_ROWS: u16 = 12;

const NAME: ColumnKey = ColumnKey::new("name");
const KIND: ColumnKey = ColumnKey::new("kind");

/// One parameter the picker offers.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamChoice {
    pub name: String,
    /// One line on what it does.
    pub summary: String,
    /// `bool`, `number`, `string`, `list`, `dict` or `localized`.
    pub kind: String,
}

/// The picker: what it adds to, the search, the parameters it lists and the selected ones (positions in `choices`).
pub struct ParameterPicker {
    id: DialogId,
    pub target: Target,
    /// The project whose instances `target` names; `None`: the font's.
    pub project: Option<u64>,
    pub query: String,
    /// The search text the list was made for; `None` before the first listing.
    pub listed: Option<String>,
    pub choices: Vec<ParamChoice>,
    pub selection: Selection<usize>,
    pub error: Option<String>,
    open: bool,
}

impl ParameterPicker {
    pub fn new(target: Target) -> ParameterPicker {
        ParameterPicker {
            id: dialog_id("fontInfo.customParameterPicker"),
            target,
            project: None,
            query: String::new(),
            listed: None,
            choices: Vec::new(),
            selection: Selection::default(),
            error: None,
            open: true,
        }
    }

    /// The picker over the entries `instances` of the project `project`.
    pub fn for_project(project: u64, instances: Vec<usize>) -> ParameterPicker {
        let mut picker = ParameterPicker::new(Target { scope: Scope::Instance, indices: instances });
        picker.project = Some(project);
        picker
    }

    /// Whether the list is older than the search text.
    pub fn needs_listing(&self) -> bool {
        self.listed.as_deref() != Some(self.query.as_str())
    }

    /// The parameters of the `font.customParameters.describe` query that fills the list.
    pub fn query_params(&self) -> Value {
        let query: String = self.query.chars().take(MAX_QUERY_CHARS).collect();
        json!({"scope": self.target.scope.name(), "query": query})
    }

    /// Takes the query's result, sorted by name in any case. The selection keeps the names the new list holds.
    pub fn fill(&mut self, result: &Value) {
        let kept: Vec<String> = self.picked(&self.selection);
        let found = result.get("parameters").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut choices: Vec<ParamChoice> = found
            .iter()
            .filter_map(|p| {
                Some(ParamChoice {
                    name: p.get("name")?.as_str()?.to_string(),
                    summary: p.get("summary").and_then(Value::as_str).unwrap_or_default().to_string(),
                    kind: p.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
                })
            })
            .take(MAX_PARAMETERS)
            .collect();
        choices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
        let items = choices.iter().enumerate().filter(|(_, c)| kept.contains(&c.name)).map(|(at, _)| at).collect();
        self.selection = Selection::new(items, None, None);
        self.choices = choices;
        self.listed = Some(self.query.clone());
        self.error = None;
    }

    /// The names of the selected parameters, in list order.
    pub fn picked(&self, selection: &Selection<usize>) -> Vec<String> {
        self.choices.iter().enumerate().filter(|(at, _)| selection.items.contains(at)).map(|(_, c)| c.name.clone()).collect()
    }
}

impl StackDialog for ParameterPicker {
    type Result = (Target, Option<u64>, Vec<String>);

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<ParameterPickerWords>();
        let ParameterPicker { query, choices, selection, error, open, .. } = self;
        let list: &[ParamChoice] = choices;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(selection)).opened(Bind::bound(open)).ok_label(words.add.clone()).show(
            ui,
            |ui, _, selection| {
                let _ = TextField::search(words.search.clone(), Bind::bound(&mut *query)).limit(MAX_QUERY_CHARS).show(ui);
                if let Some(error) = error.as_deref() {
                    Text::new(error.to_string()).show(ui);
                }
                let columns = vec![
                    ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::Content),
                    ColumnSpec::new(KIND, words.kind.clone(), ColumnWidth::Content),
                ];
                let positions: Vec<usize> = (0..list.len()).collect();
                let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(list.get(*at).map_or_else(String::new, |c| c.name.clone())));
                let _ = Table::<usize>::new(words.list.clone(), columns)
                    .max_rows(LIST_ROWS)
                    .select(SelectMode::Multi)
                    .selection(Bind::bound(&mut *selection))
                    .empty(words.none.clone())
                    .show_items(ui, &source, &mut |at, icx| {
                        let Some(choice) = list.get(*at) else { return ListRow::new("") };
                        ListRow::new(choice.name.clone()).cells(icx.cells()).cell(KIND, choice.kind.clone())
                    });
                if let Some(described) = selection.items.last().and_then(|at| list.get(*at)) {
                    Text::new(described.summary.clone()).shade(Shade::Muted).show(ui);
                }
                !selection.items.is_empty()
            },
        );
        match (shown.accepted, shown.closed) {
            (Some(selection), _) => Some(DialogResult::Ok((self.target.clone(), self.project, self.picked(&selection)))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the picker a row asked for, lists what its search finds, and posts what Add picked.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(InfoDialog::ParameterPicker(target)) = &app.view.font_info.request {
        let picker = ParameterPicker::new(target.clone());
        app.view.font_info.request = None;
        host.open(picker);
    }
    if let Some((project, instances)) = app.view.project.picker.take() {
        host.open(ParameterPicker::for_project(project, instances));
    }
    if let Some(picker) = host.get_mut::<ParameterPicker>()
        && picker.needs_listing()
    {
        match app.session.query("font.customParameters.describe", &picker.query_params()) {
            Ok(result) => picker.fill(&result),
            Err(e) => {
                picker.error = Some(e.to_string());
                picker.listed = Some(picker.query.clone());
            }
        }
    }
    if let Some(DialogResult::Ok((target, project, names))) = host.take::<ParameterPicker>() {
        if let Some(project) = project {
            app.sent.extend(project_adds(project, &target.indices, &names));
            return;
        }
        for name in names {
            let mut p = target.params();
            p.insert("name".into(), json!(name));
            app.sent.push(Msg::run("font.customParameters.add", Value::Object(p)));
        }
    }
}

/// One `project.editInstance` per entry of `instances`, adding each of `names` as a custom parameter.
pub fn project_adds(project: u64, instances: &[usize], names: &[String]) -> Vec<Msg> {
    if names.is_empty() {
        return Vec::new();
    }
    let adds: Vec<Value> = names.iter().map(|name| json!({"action": "add", "name": name})).collect();
    instances
        .iter()
        .map(|k| Msg::run("project.editInstance", json!({"project": project, "instance": k, "changes": {"customParameters": adds}})))
        .collect()
}

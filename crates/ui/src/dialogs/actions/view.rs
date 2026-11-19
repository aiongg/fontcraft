//! The Actions window as a modeless dialog: the searchable action list, the chosen action's form or JSON editor, the
//! scope and layer pop-ups, the before-and-after preview, and the action set in the sidebar.
//!
//! | Control | Posts `dialog.actions` with |
//! |---|---|
//! | search field | `{search}` |
//! | a row of the action list | `{command}`: chooses the action |
//! | a field of the form | `{set: {key: value}}` |
//! | JSON editor | `{json}`; its error shows below it |
//! | Scope, Category, Layers | `{scope}`, `{category}`, `{layers}` |
//! | Preview, Wireframe | `{preview}`, `{wireframe}` |
//! | Run, Add to Set | `{run: true}`, `{add: true}` |
//! | a step, Remove, Clear | selects it; `{remove: index}`; `{clear: true}` |
//! | Title, Name or path, Save, Open, Run Set | `{title}`; `{save}` or `{savePath}` (a name with a folder separator is a path); `{openSet}`; `{runSet: true}` |
//!
//! The window shows [`ActionsModel`], a copy of the app's state taken when it changes; the step selection and the
//! Name or path field are the window's own.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Picture, PictureSize, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{ListRow, ThumbAspect, ThumbSize};
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamRows};
use craft::ui::pickers::Select;
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::style::TypeRole;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::{Rect, Stroke, Ui, pos2};
use serde_json::{Map, Value, json};

use super::{ActionsState, GlyphScope, ID, LayerScope, MAX_JSON_BYTES, MAX_SEARCH_CHARS, action_spec, actions, check_json, matching, preview_art};
use crate::app::Msg;
use crate::app::models::font_view::CellArt;
use crate::dialogs::fields::Field;
use crate::panels::font_view::cells;

craft::ui::words! {
    /// The words of the Actions window.
    pub struct ActionsWords {
        title: "fontcraftActions.title" = "Actions";
        search: "fontcraftActions.search" = "Search Actions";
        list: "fontcraftActions.list" = "Actions";
        action: "fontcraftActions.action" = "Action";
        menu: "fontcraftActions.menu" = "Menu";
        none_found: "fontcraftActions.noneFound" = "No action matches the search.";
        choose: "fontcraftActions.choose" = "Choose an action from the list.";
        parameters: "fontcraftActions.parameters" = "Parameters (JSON)";
        scope: "fontcraftActions.scope" = "Scope";
        current_glyph: "fontcraftActions.currentGlyph" = "Current Glyph";
        selected_glyphs: "fontcraftActions.selectedGlyphs" = "Selected Glyphs";
        category_scope: "fontcraftActions.categoryScope" = "Category";
        whole_font: "fontcraftActions.wholeFont" = "Whole Font";
        category: "fontcraftActions.category" = "Category Name";
        layers: "fontcraftActions.layers" = "Layers";
        active_layer: "fontcraftActions.activeLayer" = "Active Layer";
        visible_layers: "fontcraftActions.visibleLayers" = "Visible Layers";
        all_masters: "fontcraftActions.allMasters" = "All Masters";
        preview: "fontcraftActions.preview" = "Preview";
        wireframe: "fontcraftActions.wireframe" = "Wireframe";
        before: "fontcraftActions.before" = "Before";
        after: "fontcraftActions.after" = "After";
        nothing: "fontcraftActions.nothing" = "Nothing to show";
        run: "fontcraftActions.run" = "Run";
        add: "fontcraftActions.add" = "Add to Set";
        set: "fontcraftActions.set" = "Action Set";
        set_title: "fontcraftActions.setTitle" = "Title";
        steps: "fontcraftActions.steps" = "Steps";
        step: "fontcraftActions.step" = "Step";
        empty_set: "fontcraftActions.emptySet" = "Add actions to build a set.";
        remove: "fontcraftActions.remove" = "Remove";
        clear: "fontcraftActions.clear" = "Clear";
        name: "fontcraftActions.name" = "Name or path";
        save: "fontcraftActions.save" = "Save";
        save_tip: "fontcraftActions.saveTip" = "A name saves into the Macros folder; a path saves to that file";
        open: "fontcraftActions.open" = "Open";
        open_tip: "fontcraftActions.openTip" = "A macro of the Macros folder, or a macro file";
        run_set: "fontcraftActions.runSet" = "Run Set";
    }
}

/// Rows a list shows before it scrolls: the dialog body scrolls too, so the lists must be capped.
const LIST_ROWS: u16 = 12;
/// Rows the JSON editor shows.
const JSON_LINES: usize = 6;

const LABEL: ColumnKey = ColumnKey::new("label");
const MENU: ColumnKey = ColumnKey::new("menu");
const STEP: ColumnKey = ColumnKey::new("step");

/// One step of the set as the sidebar lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepRow {
    /// `1. Round Corners`.
    pub label: String,
    /// The step's parameters as JSON, for the tooltip.
    pub params: String,
}

/// What the window shows: a copy of [`ActionsState`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionsModel {
    pub generation: u64,
    pub search: String,
    /// The actions matching the search: id, label and menu path.
    pub entries: Vec<(String, String, String)>,
    pub command: Option<String>,
    /// The chosen action's label and params doc.
    pub label: String,
    pub doc: String,
    /// The form's fields and values; `None`: the JSON editor.
    pub form: Option<(Vec<FieldRow>, Map<String, Value>)>,
    pub json: String,
    pub json_error: Option<String>,
    pub scope: GlyphScope,
    pub category: String,
    pub layers: LayerScope,
    pub preview: bool,
    pub wireframe: bool,
    /// The preview glyph before and after, in one unit box.
    pub art: [Option<CellArt>; 2],
    pub preview_error: Option<String>,
    pub summary: String,
    pub status: String,
    pub set_title: String,
    pub steps: Vec<StepRow>,
    /// The app has a Macros folder, so a plain name saves.
    pub can_save: bool,
}

/// A form field: the field and its craft row.
#[derive(Clone, Debug)]
pub struct FieldRow(pub Field);

impl PartialEq for FieldRow {
    fn eq(&self, other: &Self) -> bool {
        self.0.key == other.0.key && self.0.label == other.0.label
    }
}

impl ActionsModel {
    pub fn of(st: &ActionsState, can_save: bool) -> Self {
        let list = actions();
        let spec = st.command.as_deref().and_then(|id| action_spec(id).ok());
        let json_error = match (spec, &st.form) {
            (Some(spec), None) => check_json(spec, &st.json).err(),
            _ => None,
        };
        let steps = st
            .set
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let label = fontcraft_engine::find_command(&step.command).map_or(step.command.as_str(), |c| c.label);
                StepRow { label: format!("{}. {label}", i + 1), params: step.params.to_string() }
            })
            .collect();
        ActionsModel {
            generation: st.generation,
            search: st.search.clone(),
            entries: matching(&list, &st.search).map(|e| (e.id.to_string(), e.label.to_string(), e.group.clone())).collect(),
            command: st.command.clone(),
            label: spec.map(|s| s.label.to_string()).unwrap_or_default(),
            doc: spec.map(|s| s.params.to_string()).unwrap_or_default(),
            form: st.form.as_ref().map(|f| (f.fields.iter().cloned().map(FieldRow).collect(), f.values.clone())),
            json: st.json.clone(),
            json_error,
            scope: st.scope,
            category: st.category.clone(),
            layers: st.layers,
            preview: st.preview,
            wireframe: st.wireframe,
            art: if st.preview { preview_art(&st.shown) } else { [None, None] },
            preview_error: st.shown.error.clone(),
            summary: st.shown.summary.clone(),
            status: st.status.clone(),
            set_title: st.set_title.clone(),
            steps,
            can_save,
        }
    }
}

/// The window's own state: the selected step and the Name or path field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Local {
    pub selected: Selection<usize>,
    pub name: String,
}

/// The window on the dialog host.
pub struct ActionsDialog {
    pub model: ActionsModel,
    pub local: Local,
    /// What the person did, until the dialog module takes it.
    pub outbox: Vec<Msg>,
    /// Set from outside (`dialog.actions {open: false}`): the window closes in its next pass.
    pub closing: bool,
    open: bool,
}

impl ActionsDialog {
    pub fn new(model: ActionsModel) -> Self {
        ActionsDialog { model, local: Local::default(), outbox: Vec::new(), closing: false, open: true }
    }
}

impl StackDialog for ActionsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftActions")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<ActionsWords>();
        let ActionsDialog { model, local, outbox, open, .. } = self;
        let shown =
            Dialog::new(words.title.clone()).opened(Bind::bound(open)).modeless().resizable(true).show(ui, |ui, _| view(ui, model, local, outbox));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

fn post(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(ID, params))
}

/// The id of a control, for tests and `ui.click`: `search`, `run`, `add`, `preview`, `remove`, `save`, …
pub fn control_id(part: &str) -> egui::Id {
    egui::Id::unique((ID, part))
}

/// Shows the window's body over `m`; what the person did goes to `sent`.
pub fn view(ui: &mut Ui, m: &ActionsModel, local: &mut Local, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ActionsWords>();
    Stack::new().show(ui, |ui, _| {
        let search = TextField::new(words.search.clone(), Bind::given(m.search.clone())).limit(MAX_SEARCH_CHARS).id(control_id("search")).show(ui);
        if let Some(text) = search.events.value().filter(|t| **t != m.search) {
            sent.push(post(json!({"search": text})));
        }
        Row::new().show(ui, |ui, _| {
            action_list(ui, &words, m, sent);
            action_form(ui, &words, m, sent);
            sidebar(ui, &words, m, local, sent);
        });
    });
}

/// The actions matching the search, with their menu.
fn action_list(ui: &mut Ui, words: &ActionsWords, m: &ActionsModel, sent: &mut Vec<Msg>) {
    if m.entries.is_empty() {
        Text::new(words.none_found.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let chosen = m.command.as_deref().and_then(|id| m.entries.iter().position(|(e, _, _)| e == id));
    let mut selection = chosen.map(Selection::single).unwrap_or_default();
    let columns =
        vec![ColumnSpec::new(LABEL, words.action.clone(), ColumnWidth::Content), ColumnSpec::new(MENU, words.menu.clone(), ColumnWidth::Content)];
    let positions: Vec<usize> = (0..m.entries.len()).collect();
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(m.entries.get(*at).map_or_else(String::new, |(_, label, _)| label.clone())));
    let _ = Table::<usize>::new(words.list.clone(), columns)
        .max_rows(LIST_ROWS)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &source, &mut |at, icx| {
            let Some((_, label, group)) = m.entries.get(*at) else { return ListRow::new("") };
            ListRow::new(label.clone()).cells(icx.cells()).cell(MENU, group.clone())
        });
    let picked = selection.items.last().copied();
    if picked != chosen
        && let Some((id, _, _)) = picked.and_then(|at| m.entries.get(at))
    {
        sent.push(post(json!({"command": id})));
    }
}

/// The chosen action's parameters, scope, preview and buttons.
fn action_form(ui: &mut Ui, words: &ActionsWords, m: &ActionsModel, sent: &mut Vec<Msg>) {
    Stack::new().show(ui, |ui, _| {
        if m.command.is_none() {
            Text::new(words.choose.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        Section::new(Label::from(m.label.clone())).show(ui, |ui, _| match &m.form {
            Some((rows, values)) => form_rows(ui, &m.label, rows, values, sent),
            None => {
                let edit = TextField::new(words.parameters.clone(), Bind::given(m.json.clone()))
                    .mono()
                    .lines(JSON_LINES)
                    .limit(MAX_JSON_BYTES)
                    .tooltip(Label::from(m.doc.clone()))
                    .id(control_id("json"))
                    .show(ui);
                if let Some(text) = edit.events.committed().filter(|t| **t != m.json) {
                    sent.push(post(json!({"json": text})));
                }
                if let Some(error) = &m.json_error {
                    let _ = Notice::new(Tone::Bad, Label::from(error.clone())).show(ui);
                }
            }
        });
        scope_controls(ui, words, m, sent);
        Row::new().show(ui, |ui, _| {
            let preview = Checkbox::new(Bind::given(m.preview)).label(words.preview.clone()).id(control_id("preview")).show(ui);
            if let Some(on) = preview.events.committed() {
                sent.push(post(json!({"preview": on})));
            }
            let wireframe = Checkbox::new(Bind::given(m.wireframe)).label(words.wireframe.clone()).id(control_id("wireframe")).show(ui);
            if let Some(on) = wireframe.events.committed() {
                sent.push(post(json!({"wireframe": on})));
            }
        });
        if m.preview {
            preview_pane(ui, words, m);
            if let Some(error) = &m.preview_error {
                let _ = Notice::new(Tone::Bad, Label::from(error.clone())).show(ui);
            }
        }
        Row::new().show(ui, |ui, _| {
            if Button::new(words.run.clone()).id(control_id("run")).show(ui).clicked() {
                sent.push(post(json!({"run": true})));
            }
            if Button::new(words.add.clone()).id(control_id("add")).show(ui).clicked() {
                sent.push(post(json!({"add": true})));
            }
        });
        for line in [&m.summary, &m.status] {
            if !line.is_empty() {
                Text::new(line.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
        }
    });
}

/// The form's fields as parameter rows; a changed row posts `{set: {key: value}}`.
fn form_rows(ui: &mut Ui, title: &str, rows: &[FieldRow], values: &Map<String, Value>, sent: &mut Vec<Msg>) {
    let mut params = Vec::with_capacity(rows.len());
    let mut group: Option<&str> = None;
    for FieldRow(field) in rows {
        if field.group.as_deref() != group {
            group = field.group.as_deref();
            if let Some(heading) = group {
                params.push(Param::section(Label::from(heading.to_string())));
            }
        }
        params.push(field.param());
    }
    let mut fields = DialogFields::new();
    for FieldRow(field) in rows {
        if let Some(value) = values.get(&field.key) {
            fields.set(field.key.clone(), field.to_field_value(value));
        }
    }
    let before = fields.clone();
    let _ = ParamRows::new(Label::from(title.to_string()), &params, Bind::bound(&mut fields)).show(ui);
    for FieldRow(field) in rows {
        let (was, now) = (before.get(&field.key), fields.get(&field.key));
        if was != now
            && let Some(value) = now.and_then(|now| field.from_field_value(now))
        {
            sent.push(post(json!({"set": {field.key.clone(): value}})));
        }
    }
}

/// The Scope and Layers pop-ups, and the category field of the Category scope.
fn scope_controls(ui: &mut Ui, words: &ActionsWords, m: &ActionsModel, sent: &mut Vec<Msg>) {
    Row::new().wrap(true).show(ui, |ui, _| {
        let scopes = [&words.current_glyph, &words.selected_glyphs, &words.category_scope, &words.whole_font];
        let entries: Vec<Entry<String>> =
            GlyphScope::ALL.iter().zip(scopes).map(|(s, label)| Entry::new(s.word().to_string(), label.clone())).collect();
        let scope = Select::new(words.scope.clone(), Bind::given(m.scope.word().to_string()), &entries).id(control_id("scope")).show(ui);
        if let Some(word) = scope.events.committed().filter(|w| **w != m.scope.word()) {
            sent.push(post(json!({"scope": word})));
        }
        if m.scope == GlyphScope::Category {
            let category =
                TextField::new(words.category.clone(), Bind::given(m.category.clone())).limit(MAX_SEARCH_CHARS).id(control_id("category")).show(ui);
            if let Some(text) = category.events.committed().filter(|t| **t != m.category) {
                sent.push(post(json!({"category": text})));
            }
        }
        let layer_words = [&words.active_layer, &words.visible_layers, &words.all_masters];
        let entries: Vec<Entry<String>> =
            LayerScope::ALL.iter().zip(layer_words).map(|(s, label)| Entry::new(s.word().to_string(), label.clone())).collect();
        let layers = Select::new(words.layers.clone(), Bind::given(m.layers.word().to_string()), &entries).id(control_id("layers")).show(ui);
        if let Some(word) = layers.events.committed().filter(|w| **w != m.layers.word()) {
            sent.push(post(json!({"layers": word})));
        }
    });
}

/// The preview glyph before (left) and after (right), at one scale.
fn preview_pane(ui: &mut Ui, words: &ActionsWords, m: &ActionsModel) {
    if m.art.iter().all(Option::is_none) {
        Text::new(words.nothing.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    Row::new().show(ui, |ui, _| {
        for (art, caption) in m.art.iter().zip([&words.before, &words.after]) {
            Stack::new().show(ui, |ui, _| {
                let art = art.clone();
                let wireframe = m.wireframe;
                Picture::painter(caption.clone(), ThumbAspect::SQUARE, move |cx, rect| {
                    if let Some(art) = &art {
                        paint_art(cx, rect, art, wireframe);
                    }
                })
                .extent(PictureSize::Thumb(ThumbSize::Large))
                .show(ui);
                Text::new(caption.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            });
        }
    });
}

/// Paints `art` (in the unit box, y up) filled, or as its outlines.
fn paint_art(cx: &ContentPainterCx<'_>, rect: Rect, art: &CellArt, wireframe: bool) {
    if !wireframe {
        cells::paint(cx, rect, art, 1.0, 0.0);
        return;
    }
    // The placement `cells::paint` gives a unit box: 80% of the shorter side, centred.
    let scale = rect.width().min(rect.height()) * 0.8;
    if !(scale.is_finite() && scale > 0.0) {
        return;
    }
    let (left, bottom) = (rect.center().x - scale / 2.0, rect.center().y + scale / 2.0);
    let hairline = 1.0 / cx.painter.pixels_per_point().max(0.01);
    for contour in &art.contours {
        let line: Vec<egui::Pos2> = contour.iter().map(|[x, y]| pos2(left + x * scale, bottom - y * scale)).collect();
        cx.painter.add(egui::Shape::closed_line(line, Stroke::new(hairline, cx.ink())));
    }
}

/// The action set: its title, its steps, and the buttons that edit, save, open and run it.
fn sidebar(ui: &mut Ui, words: &ActionsWords, m: &ActionsModel, local: &mut Local, sent: &mut Vec<Msg>) {
    Section::new(words.set.clone()).show(ui, |ui, _| {
        let title = TextField::new(words.set_title.clone(), Bind::given(m.set_title.clone())).id(control_id("setTitle")).show(ui);
        if let Some(text) = title.events.committed().filter(|t| **t != m.set_title) {
            sent.push(post(json!({"title": text})));
        }
        local.selected.items.retain(|i| *i < m.steps.len());
        if m.steps.is_empty() {
            Text::new(words.empty_set.clone()).shade(Shade::Muted).show(ui);
        } else {
            let columns = vec![ColumnSpec::new(STEP, words.step.clone(), ColumnWidth::Content)];
            let positions: Vec<usize> = (0..m.steps.len()).collect();
            let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(m.steps.get(*at).map_or_else(String::new, |s| s.label.clone())));
            let _ = Table::<usize>::new(words.steps.clone(), columns)
                .max_rows(LIST_ROWS)
                .select(SelectMode::Single)
                .selection(Bind::bound(&mut local.selected))
                .show_items(ui, &source, &mut |at, icx| {
                    let Some(step) = m.steps.get(*at) else { return ListRow::new("") };
                    ListRow::new(step.label.clone()).cells(icx.cells()).tooltip(step.params.clone())
                });
        }
        let selected = local.selected.items.last().copied();
        Row::new().show(ui, |ui, _| {
            if Button::new(words.remove.clone()).enabled(selected.is_some()).id(control_id("remove")).show(ui).clicked()
                && let Some(i) = selected
            {
                local.selected = Selection::default();
                sent.push(post(json!({"remove": i})));
            }
            if Button::new(words.clear.clone()).enabled(!m.steps.is_empty()).id(control_id("clear")).show(ui).clicked() {
                local.selected = Selection::default();
                sent.push(post(json!({"clear": true})));
            }
        });
        let _ = TextField::new(words.name.clone(), Bind::bound(&mut local.name)).id(control_id("name")).show(ui);
        let typed = local.name.trim().to_string();
        let is_path = typed.contains(['/', '\\']);
        Row::new().wrap(true).show(ui, |ui, _| {
            let save_ok = !typed.is_empty() && !m.steps.is_empty() && (is_path || m.can_save);
            if Button::new(words.save.clone()).enabled(save_ok).tooltip(words.save_tip.clone()).id(control_id("save")).show(ui).clicked() {
                let key = if is_path { "savePath" } else { "save" };
                sent.push(post(json!({key: typed})));
            }
            if Button::new(words.open.clone()).enabled(!typed.is_empty()).tooltip(words.open_tip.clone()).id(control_id("open")).show(ui).clicked() {
                local.selected = Selection::default();
                sent.push(post(json!({"openSet": typed})));
            }
            if Button::new(words.run_set.clone()).enabled(!m.steps.is_empty()).id(control_id("runSet")).show(ui).clicked() {
                sent.push(post(json!({"runSet": true})));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;

    /// With an action chosen, Preview, Run and Add to Set post their switches; a set's step can be selected and
    /// removed.
    #[test]
    fn buttons_post_dialog_actions() {
        let model = ActionsModel {
            entries: vec![("path.removeOverlap".into(), "Remove Overlap".into(), "Path".into())],
            command: Some("path.removeOverlap".into()),
            label: "Remove Overlap".into(),
            json: "{}".into(),
            steps: vec![StepRow { label: "1. Remove Overlap".into(), params: "{}".into() }],
            ..ActionsModel::default()
        };
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let local = Rc::new(RefCell::new(Local { selected: Selection::single(0), ..Local::default() }));
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &model, &mut local.borrow_mut(), &mut sent);
            seen.borrow_mut().extend(sent);
        });
        f.settle();
        for part in ["preview", "run", "add", "remove"] {
            let target = f.node_of(control_id(part)).expect(part);
            f.click(&target).settle();
        }
        assert_eq!(
            *posted.borrow(),
            vec![post(json!({"preview": true})), post(json!({"run": true})), post(json!({"add": true})), post(json!({"remove": 0}))]
        );
    }
}

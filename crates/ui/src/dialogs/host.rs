//! The open spec dialog on the window's `DialogHost`.
//!
//! [`run`] keeps the host in step with the app: it opens a [`SpecDialog`] when the app has a dialog open, shows the
//! app's values after every change, and closes it when the app closed the dialog (OK, a control request, the document
//! gone). What the person does in it comes back as [`Event`]s that `run` applies through `form.rs`, the same calls the
//! control channel makes.

use std::borrow::Cow;

use craft::behavior::{DisplayOrder, IconId, ItemSource, SelectionChange};
use craft::theme::variant::Tone;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, Trigger};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::menu::Menu;
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::{DialogFields, FormDialog, Param, ParamRows};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use egui::Ui;
use serde_json::{Map, Value};

use super::fields::Field;
use super::form::{self, OpenDialog};
use crate::app::FontCraftApp;

craft::ui::words! {
    /// The words of the spec dialogs beside the common OK and Cancel.
    pub struct DialogWords {
        /// The menu of a dialog's actions, at its bottom left.
        actions: "fontcraftDialog.actions" = "Actions";
    }
}

/// What the person did in the dialog in one pass.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A field took a value.
    Set(String, Value),
    /// A tab, by its index.
    Tab(usize),
    /// An item of the actions menu, by its id.
    Action(String),
    /// The folder chooser for a folder field.
    Choose(String),
    Confirm,
    Third,
}

/// What the dialog shows: a copy of the app's [`OpenDialog`], taken when its revision moves.
#[derive(Clone, Debug)]
pub struct FormView {
    pub serial: u64,
    pub revision: u64,
    pub title: String,
    /// The tab labels and the tab showing.
    pub tabs: Option<(Vec<String>, usize)>,
    pub message: String,
    pub fields: Vec<Field>,
    pub values: Map<String, Value>,
    pub error: Option<String>,
    /// The OK and Cancel captions; empty for the common words.
    pub ok: String,
    pub cancel: String,
    /// The field the third button answers, and the button's id and caption.
    pub third: Option<(String, String)>,
    pub third_key: Option<String>,
    /// The actions menu: id and label.
    pub actions: Vec<(String, String)>,
    pub can_confirm: bool,
}

impl FormView {
    pub fn of(d: &OpenDialog) -> Self {
        let third = d.third();
        FormView {
            serial: d.serial,
            revision: d.revision,
            title: d.title.clone(),
            tabs: d.tabs.as_ref().map(|t| (t.list.iter().map(|tab| tab.label.to_string()).collect(), t.active)),
            message: d.message.to_string(),
            fields: d.fields.clone(),
            values: d.values.clone(),
            error: d.error.clone(),
            ok: d.ok_caption().to_string(),
            cancel: d.buttons[1].to_string(),
            third: third.map(|t| (t.id.to_string(), t.caption.to_string())),
            third_key: third.map(|t| t.key.to_string()),
            actions: d.actions.iter().map(|a| (a.id.to_string(), a.label.to_string())).collect(),
            can_confirm: d.can_confirm(),
        }
    }

    /// The rows: a heading before each run of fields under one, the question of the third button as text in place of
    /// the field it answers.
    fn params(&self) -> Vec<Param> {
        let mut params = Vec::with_capacity(self.fields.len() + 1);
        let mut group: Option<&str> = None;
        for field in &self.fields {
            if Some(&field.key) == self.third_key.as_ref() {
                if !field.label.is_empty() {
                    params.push(Param::read_only_text(format!("{}.question", field.key), Label::Empty, Label::from(field.label.clone())));
                }
                continue;
            }
            if field.group.as_deref() != group {
                group = field.group.as_deref();
                if let Some(heading) = group {
                    params.push(Param::section(Label::from(heading.to_string())));
                }
            }
            params.push(field.param());
        }
        params
    }

    /// The values as the rows hold them.
    fn rows(&self) -> DialogFields {
        let mut rows = DialogFields::new();
        for field in &self.fields {
            if let Some(value) = self.values.get(&field.key) {
                rows.set(field.key.clone(), field.to_field_value(value));
            }
        }
        rows
    }
}

/// The open spec dialog as the host shows it.
pub struct SpecDialog {
    id: DialogId,
    /// The serial of the app's dialog, as text: the host keeps it with a Cancel ([`DialogHost::take_keyed`]).
    key: String,
    view: FormView,
    rows: DialogFields,
    /// What the person did, until [`run`] takes it.
    events: Vec<Event>,
    /// The app closed the dialog: the next pass ends it.
    closing: bool,
    /// Text for the system clipboard, copied in the next pass.
    clipboard: Option<String>,
}

impl SpecDialog {
    pub fn new(view: FormView) -> Self {
        SpecDialog {
            id: dialog_id("fontcraft.specDialog"),
            key: view.serial.to_string(),
            rows: view.rows(),
            view,
            events: Vec::new(),
            closing: false,
            clipboard: None,
        }
    }

    /// Shows the app's values from the next pass on.
    fn refresh(&mut self, view: FormView) {
        self.rows = view.rows();
        self.view = view;
    }

    /// What the person did since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

impl StackDialog for SpecDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        if let Some(text) = self.clipboard.take() {
            ui.ctx().copy_text(text);
        }
        let shown = view(ui, &self.view, &mut self.rows);
        self.events.extend(shown.events);
        shown.cancelled.then_some(DialogResult::Cancel)
    }
}

/// What one pass of the dialog reports.
pub struct Shown {
    pub events: Vec<Event>,
    /// Cancel, Escape or the close action.
    pub cancelled: bool,
}

/// One pass of the dialog over `rows`, the values its rows hold.
pub fn view(ui: &mut Ui, v: &FormView, rows: &mut DialogFields) -> Shown {
    let words = Env::of(ui).words::<DialogWords>();
    let params = v.params();
    let before = rows.clone();
    let mut events = Vec::new();
    let mut action = None;
    let actions: Vec<Entry<String>> = v.actions.iter().map(|(id, label)| Entry::new(id.clone(), Label::from(label.clone()))).collect();
    let title = Label::from(v.title.clone());
    let mut dialog = FormDialog::new(title.clone(), Bind::bound(&mut *rows)).accept_stays_open(true).alt_resets(false).ok_enabled(v.can_confirm);
    if !v.ok.is_empty() {
        dialog = dialog.ok_label(Label::from(v.ok.clone()));
    }
    if !v.cancel.is_empty() {
        dialog = dialog.cancel_label(Label::from(v.cancel.clone()));
    }
    if let Some((_, caption)) = &v.third {
        dialog = dialog.extra_buttons([FooterButton::new(Label::from(caption.clone()), DialogRole::Other).enabled(v.can_confirm)]);
    }
    if !actions.is_empty() {
        let label = words.actions.clone();
        let action = &mut action;
        dialog = dialog.footer_extra(move |ui| {
            let more = IconButton::new(IconId::new("more")).label(label.clone()).show(ui);
            if let Some(id) = Menu::new(label.clone()).opened_by(&more, Trigger::Click).pick(ui, &actions).picked {
                *action = Some(id);
            }
        });
    }
    let shown = dialog.show(ui, |ui, _, fields| {
        if let Some((labels, active)) = &v.tabs {
            let strip = TabStrip::new(title.clone())
                .kind(TabKind::Text)
                .shown(*active)
                .show_items(ui, &Labels(labels), &mut |key, _| Tab::new(Label::from(labels.get(*key).cloned().unwrap_or_default())));
            for event in strip.response.events {
                if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                    && let Some(key) = keys.into_iter().last()
                    && key != *active
                {
                    events.push(Event::Tab(key));
                }
            }
        }
        if !v.message.is_empty() {
            let _ = Text::new(Label::from(v.message.clone())).show(ui);
        }
        let out = ParamRows::new(title.clone(), &params, Bind::bound(&mut *fields)).show(ui);
        if let Some(path) = out.path {
            events.push(Event::Choose(path.key));
        }
        if let Some(error) = &v.error {
            let _ = Notice::new(Tone::Bad, Label::from(error.clone())).show(ui);
        }
        v.can_confirm
    });
    for field in &v.fields {
        let (was, now) = (before.get(&field.key), rows.get(&field.key));
        if was != now
            && let Some(value) = now.and_then(|now| field.from_field_value(now))
        {
            events.push(Event::Set(field.key.clone(), value));
        }
    }
    match shown.picked {
        Some(0) => events.push(Event::Confirm),
        Some(2) => events.push(Event::Third),
        _ => {}
    }
    if let Some(id) = action {
        events.push(Event::Action(id));
    }
    let cancelled = shown.closed.is_some() && shown.picked != Some(0);
    Shown { events, cancelled }
}

/// The tab labels as an item source, keyed by position.
struct Labels<'t>(&'t [String]);

impl DisplayOrder<usize> for Labels<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|label| Cow::Borrowed(label.as_str()))
    }
}

impl ItemSource<usize> for Labels<'_> {}

/// Keeps the host's dialog in step with the app's and applies what the person did.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    form::poll_chooser(app);
    form::close_if_gone(app);
    if let Some((DialogResult::Cancel, Some(key))) = host.take_keyed::<SpecDialog>()
        && app.dialogs.open.as_ref().is_some_and(|d| d.serial.to_string() == key)
        && let Err(e) = form::cancel(app)
    {
        app.status = e;
    }
    let events = host.get_mut::<SpecDialog>().filter(|d| !d.closing).map(SpecDialog::take_events).unwrap_or_default();
    for event in events {
        if let Err(e) = apply(app, event) {
            app.status = e;
        }
    }
    let shown = host.get_mut::<SpecDialog>().filter(|d| !d.closing);
    match (app.dialogs.open.as_ref(), shown) {
        (Some(d), Some(shown)) if shown.view.serial == d.serial => {
            if shown.view.revision != d.revision {
                shown.refresh(FormView::of(d));
            }
            if let Some(text) = app.dialogs.clipboard.take() {
                shown.clipboard = Some(text);
            }
        }
        (Some(d), _) => {
            host.open(SpecDialog::new(FormView::of(d)));
        }
        (None, Some(shown)) => shown.closing = true,
        (None, None) => {}
    }
}

fn apply(app: &mut FontCraftApp, event: Event) -> Result<(), String> {
    match event {
        Event::Set(key, value) => form::set(app, &key, &value).map(drop),
        Event::Tab(index) => form::switch_tab(app, index).map(drop),
        Event::Action(id) => form::run_action(app, &id).map(drop),
        Event::Choose(key) => form::choose(app, &key, false),
        Event::Confirm => form::confirm_by_person(app).map(drop),
        Event::Third => form::third_by_person(app).map(drop),
    }
}

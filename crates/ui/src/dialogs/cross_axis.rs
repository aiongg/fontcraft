//! The Cross-Axis Mappings window (`window.crossAxisMappings`, Font › Cross-Axis Mappings): the active font's
//! cross-axis mappings as cards, edited as a draft and written with `font.setCrossAxisMappings` when the person
//! presses Apply.
//!
//! | Control | Does |
//! |---|---|
//! | Description field at the top | the description of the whole list |
//! | disclosure of a card | shows or hides the card's axes; with Alt held, every card's |
//! | description field of a card | the mapping's description |
//! | `−` on a card | removes the mapping |
//! | tick in the Input or Output column | the axis takes part in that side of the mapping |
//! | number in the Input or Output column | the axis's design coordinate on that side |
//! | New Cross-Axis Mapping | adds a card with every axis at the default master's location |
//! | Apply | runs `font.setCrossAxisMappings` with the draft, one undo step |
//! | Revert | drops the draft's changes |
//!
//! The draft is the window's own state. While it has no changes it follows the font, so an undo shows at once; a
//! draft with changes stays until the font holds them (Apply), Revert, or another document becomes active. A refused
//! Apply keeps the draft. `ui.inspect` reports the font's mappings under `crossAxisMappings`.

use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::controls::disclosure::Disclosure;
use craft::ui::dialog::{Dialog, DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use fontcraft_engine::cmd::cross_axis::{CrossAxisMapping, CrossAxisMappings, mappings};
use fontcraft_model::Font;
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};

craft::ui::words! {
    /// The words of the Cross-Axis Mappings window.
    pub struct CrossAxisWords {
        title: "fontcraftCrossAxis.title" = "Cross-Axis Mappings";
        description: "fontcraftCrossAxis.description" = "Description";
        mapping: "fontcraftCrossAxis.mapping" = "Mapping";
        axes: "fontcraftCrossAxis.axes" = "Axes";
        input: "fontcraftCrossAxis.input" = "Input";
        output: "fontcraftCrossAxis.output" = "Output";
        remove: "fontcraftCrossAxis.remove" = "−";
        remove_tip: "fontcraftCrossAxis.removeTip" = "Remove this mapping";
        add: "fontcraftCrossAxis.add" = "New Cross-Axis Mapping";
        apply: "fontcraftCrossAxis.apply" = "Apply";
        revert: "fontcraftCrossAxis.revert" = "Revert";
        no_axes: "fontcraftCrossAxis.noAxes" = "The font has no axes.";
    }
}

pub const ID: &str = "window.crossAxisMappings";
/// Longest description the fields take, in bytes.
const MAX_DESCRIPTION: usize = 4096;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: ID,
    label: "Cross-Axis Mappings",
    menu: &["Font"],
    shortcut: None,
    params: "{open?: bool (default true)} shows or hides the Cross-Axis Mappings window of the active font. ui.inspect reports it under `crossAxisMappings`: {open, description?, mappings: [{description?, input, output}]}",
}];

/// Whether the window shows, in `ViewState::cross_axis`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrossAxisState {
    pub open: bool,
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if command.id != ID {
        return Err(format!("no cross-axis command `{}`", command.id));
    }
    let open = match command.params.get("open") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool().ok_or("`open` must be true or false")?,
    };
    app.session.font().map_err(|e| e.to_string())?;
    app.view.cross_axis.open = open;
    Ok(())
}

/// The `crossAxisMappings` key of `ui.inspect`: whether the window shows and the active font's mappings.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let mut value = app.session.font().map_or_else(|_| json!({}), |font| mappings(font).to_json());
    if let Value::Object(m) = &mut value {
        m.insert("open".into(), json!(app.view.cross_axis.open));
    }
    state.insert("crossAxisMappings".into(), value);
}

/// One axis on one side of a card.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub on: bool,
    pub value: f64,
}

/// One mapping as the window edits it: a cell per font axis on each side.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub description: String,
    pub open: bool,
    pub input: Vec<Cell>,
    pub output: Vec<Cell>,
}

/// The mappings being edited, with the document and revision they were read from.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub source: (u64, u64),
    pub changed: bool,
    pub description: String,
    pub cards: Vec<Card>,
}

/// What the window reads from the active font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrossModel {
    /// The document and its revision.
    pub source: (u64, u64),
    /// Every axis: tag and name.
    pub axes: Vec<(String, String)>,
    /// The default master's design location, one value per axis.
    pub defaults: Vec<f64>,
    /// The mappings the font holds.
    pub stored: CrossAxisMappings,
}

impl CrossModel {
    pub fn of(font: &Font, source: (u64, u64)) -> CrossModel {
        let master = font.masters.first();
        CrossModel {
            source,
            axes: font.axes.iter().map(|a| (a.tag.clone(), a.name.clone())).collect(),
            defaults: (0..font.axes.len()).map(|i| master.and_then(|m| m.location.get(i).copied()).unwrap_or(0.0)).collect(),
            stored: mappings(font),
        }
    }

    fn cells(&self, side: &[(String, f64)]) -> Vec<Cell> {
        self.axes
            .iter()
            .zip(&self.defaults)
            .map(|((tag, _), d)| match side.iter().find(|(t, _)| t == tag) {
                Some((_, v)) => Cell { on: true, value: *v },
                None => Cell { on: false, value: *d },
            })
            .collect()
    }

    /// A draft of the stored mappings.
    pub fn read_draft(&self) -> Draft {
        let cards = self
            .stored
            .mappings
            .iter()
            .map(|m| Card {
                description: m.description.clone().unwrap_or_default(),
                open: true,
                input: self.cells(&m.input),
                output: self.cells(&m.output),
            })
            .collect();
        Draft { source: self.source, changed: false, description: self.stored.description.clone().unwrap_or_default(), cards }
    }

    /// A new card: no axis ticked, each at the default master's location.
    fn new_card(&self) -> Card {
        let cells: Vec<Cell> = self.defaults.iter().map(|v| Cell { on: false, value: *v }).collect();
        Card { description: String::new(), open: true, input: cells.clone(), output: cells }
    }

    /// The draft to show now: `draft` kept or refreshed from the font as the module comment says.
    pub fn follow(&self, draft: Option<Draft>) -> Draft {
        match draft {
            // A draft whose changes the font now holds (after Apply) follows the font again.
            Some(mut d) if d.source.0 == self.source.0 && d.changed && self.as_mappings(&d) == self.stored => {
                d.changed = false;
                d.source = self.source;
                d
            }
            Some(d) if d.source.0 == self.source.0 && (d.changed || d.source.1 == self.source.1) => d,
            _ => self.read_draft(),
        }
    }

    /// What the draft would store, for telling whether Apply has anything to do.
    pub fn as_mappings(&self, draft: &Draft) -> CrossAxisMappings {
        let side = |cells: &[Cell]| self.axes.iter().zip(cells).filter(|(_, c)| c.on).map(|((tag, _), c)| (tag.clone(), c.value)).collect();
        let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        CrossAxisMappings {
            description: text(&draft.description),
            mappings: draft
                .cards
                .iter()
                .map(|c| CrossAxisMapping { description: text(&c.description), input: side(&c.input), output: side(&c.output) })
                .collect(),
        }
    }

    /// The parameters of `font.setCrossAxisMappings` for the draft.
    pub fn params(&self, draft: &Draft) -> Value {
        let side = |cells: &[Cell]| {
            Value::Object(self.axes.iter().zip(cells).filter(|(_, c)| c.on).map(|((tag, _), c)| (tag.clone(), json!(c.value))).collect())
        };
        let list: Vec<Value> = draft
            .cards
            .iter()
            .map(|c| {
                let mut o = Map::new();
                o.insert("input".into(), side(&c.input));
                o.insert("output".into(), side(&c.output));
                if !c.description.trim().is_empty() {
                    o.insert("description".into(), json!(c.description));
                }
                Value::Object(o)
            })
            .collect();
        let mut p = Map::new();
        p.insert("mappings".into(), Value::Array(list));
        if !draft.description.trim().is_empty() {
            p.insert("description".into(), json!(draft.description));
        }
        Value::Object(p)
    }
}

/// The window on the dialog host.
pub struct CrossAxisDialog {
    id: DialogId,
    pub model: CrossModel,
    pub draft: Draft,
    /// What the person did, until the dialog module takes it.
    pub outbox: Vec<Msg>,
    /// Set when the app closed the window: it closes in its next pass.
    pub closing: bool,
    open: bool,
}

impl CrossAxisDialog {
    pub fn new(model: CrossModel) -> Self {
        let draft = model.read_draft();
        CrossAxisDialog { id: dialog_id("fontcraftCrossAxis"), model, draft, outbox: Vec::new(), closing: false, open: true }
    }
}

impl StackDialog for CrossAxisDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<CrossAxisWords>();
        let CrossAxisDialog { model, draft, outbox, open, .. } = self;
        let shown =
            Dialog::new(words.title.clone()).opened(Bind::bound(open)).modeless().resizable(true).show(ui, |ui, _| view(ui, model, draft, outbox));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// The window's body: edits `draft` and posts Apply's command to `sent`.
pub fn view(ui: &mut Ui, m: &CrossModel, draft: &mut Draft, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<CrossAxisWords>();
    if m.axes.is_empty() {
        Text::new(words.no_axes.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let before = m.as_mappings(draft);
    let mut reverted = false;
    Stack::new().show(ui, |ui, _| {
        let typed = TextField::new(words.description.clone(), Bind::given(draft.description.clone()))
            .limit(MAX_DESCRIPTION)
            .id_salt("fontcraftCrossAxis.description")
            .show(ui);
        if let Some(text) = typed.events.value() {
            draft.description = text.clone();
        }
        let mut remove = None;
        let mut every_card = None;
        for (i, card) in draft.cards.iter_mut().enumerate() {
            card_view(ui, &words, m, i, card, &mut remove, &mut every_card);
        }
        if let Some(open) = every_card {
            draft.cards.iter_mut().for_each(|card| card.open = open);
        }
        if let Some(i) = remove.filter(|i| *i < draft.cards.len()) {
            draft.cards.remove(i);
        }
        Row::new().show(ui, |ui, _| {
            if Button::new(words.add.clone()).show(ui).clicked() {
                draft.cards.push(m.new_card());
            }
            let differs = m.as_mappings(draft) != m.stored;
            if Button::new(words.apply.clone()).enabled(differs).show(ui).clicked() && differs {
                sent.push(Msg::run("font.setCrossAxisMappings", m.params(draft)));
            }
            if Button::new(words.revert.clone()).enabled(draft.changed).show(ui).clicked() && draft.changed {
                reverted = true;
            }
        });
    });
    if reverted {
        *draft = m.read_draft();
    } else if m.as_mappings(draft) != before {
        draft.changed = true;
    }
}

/// One card: its disclosure, description and remove button, then a row per axis with a tick and a value per side. A
/// disclosure clicked with Alt held asks for every card to open or close (`every_card`).
fn card_view(
    ui: &mut Ui,
    words: &CrossAxisWords,
    m: &CrossModel,
    i: usize,
    card: &mut Card,
    remove: &mut Option<usize>,
    every_card: &mut Option<bool>,
) {
    let number = i.saturating_add(1);
    Row::new().show(ui, |ui, _| {
        let out = Disclosure::new(Bind::given(card.open)).label(words.axes.clone()).id_salt(("fontcraftCrossAxis.open", i)).show(ui);
        if let Some(open) = out.events.committed() {
            card.open = *open;
            if craft_ext::ui::contract::held_mods(ui.ctx()).alt {
                *every_card = Some(*open);
            }
        }
        let name = Label::from(format!("{} {number}", words.mapping.as_str()));
        let typed = TextField::new(name.clone(), Bind::given(card.description.clone()))
            .placeholder(name)
            .limit(MAX_DESCRIPTION)
            .id_salt(("fontcraftCrossAxis.cardDescription", i))
            .show(ui);
        if let Some(text) = typed.events.value() {
            card.description = text.clone();
        }
        if Button::new(words.remove.clone()).tooltip(words.remove_tip.clone()).id_salt(("fontcraftCrossAxis.remove", i)).show(ui).clicked() {
            *remove = Some(i);
        }
    });
    if !card.open {
        return;
    }
    Row::new().show(ui, |ui, _| {
        for (side, cells) in [(&words.input, &mut card.input), (&words.output, &mut card.output)] {
            Stack::new().show(ui, |ui, _| {
                Text::new(side.clone()).show(ui);
                for (k, (_, axis)) in m.axes.iter().enumerate() {
                    let Some(cell) = cells.get_mut(k) else { continue };
                    Row::new().show(ui, |ui, _| {
                        let tick = Checkbox::new(Bind::given(cell.on))
                            .label(Label::from(axis.clone()))
                            .id_salt(("fontcraftCrossAxis.on", i, side.as_str(), k))
                            .show(ui);
                        if let Some(on) = tick.events.committed() {
                            cell.on = *on;
                        }
                        let field = NumberField::new(Label::from(format!("{axis} {}", side.as_str())), Bind::given(Some(cell.value)))
                            .enabled(cell.on)
                            .id_salt(("fontcraftCrossAxis.value", i, side.as_str(), k))
                            .show(ui);
                        if let Some(Some(v)) = field.edit.events.committed()
                            && v.is_finite()
                        {
                            cell.value = *v;
                        }
                    });
                }
            });
        }
    });
}

/// Opens, refreshes or closes the window to match [`CrossAxisState`] and the active font, and posts Apply.
pub fn run_dialog(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<CrossAxisDialog>().is_some() && app.view.cross_axis.open {
        app.sent.push(Msg::Shell(CommandRef::with_params(ID, json!({"open": false}))));
    }
    let model = match app.session.active() {
        Some(doc) if app.view.cross_axis.open => Some(((doc.id, doc.revision), doc.font.clone())),
        _ => None,
    };
    if let Some(dialog) = host.get_mut::<CrossAxisDialog>() {
        app.sent.append(&mut dialog.outbox);
        match model {
            Some((source, font)) => {
                if dialog.model.source != source {
                    dialog.model = CrossModel::of(&font, source);
                }
                dialog.draft = dialog.model.follow(Some(dialog.draft.clone()));
            }
            None => dialog.closing = true,
        }
    } else if let Some((source, font)) = model {
        host.open(CrossAxisDialog::new(CrossModel::of(&font, source)));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::testing::Fixture;
    use fontcraft_engine::Session;
    use fontcraft_model::Axis;

    use super::*;
    use crate::app::APP;
    use crate::control;

    fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
        control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    fn variable_app() -> FontCraftApp {
        let mut font = Font::new("Cross");
        font.axes = Arc::new(vec![
            Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() },
            Axis { name: "Width".into(), tag: "wdth".into(), ..Default::default() },
        ]);
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0, 100.0];
        let mut session = Session::new();
        session.add_document(font, None).unwrap();
        FontCraftApp::new(session, Arc::new(Host::builder(APP).build()))
    }

    fn model(app: &FontCraftApp) -> CrossModel {
        let doc = app.session.active().unwrap();
        CrossModel::of(&doc.font, (doc.id, doc.revision))
    }

    #[test]
    fn window_cross_axis_mappings_opens_on_the_fonts_mappings_and_applies_the_draft() {
        let mut app = variable_app();
        let set = json!({"mappings": [{"input": {"wght": 700}, "output": {"wdth": 80}}]});
        call(&mut app, "engine.execute", json!({"command": "font.setCrossAxisMappings", "params": set}));
        call(&mut app, "engine.execute", json!({"command": ID, "params": {}}));
        let shown = call(&mut app, "ui.inspect", json!({}))["crossAxisMappings"].clone();
        assert_eq!(shown["open"], true);
        assert_eq!(shown["mappings"], json!([{"input": {"wght": 700.0}, "output": {"wdth": 80.0}}]));
        assert!(app.view.cross_axis.open);

        // The draft holds a cell per axis on each side, ticked where the mapping names the axis, and gives back the
        // same parameters.
        let m = model(&app);
        let draft = m.read_draft();
        assert_eq!(draft.cards.len(), 1);
        assert_eq!(draft.cards[0].input, vec![Cell { on: true, value: 700.0 }, Cell { on: false, value: 100.0 }]);
        assert_eq!(m.as_mappings(&draft), m.stored);
        let mut edited = draft.clone();
        edited.cards[0].output[0] = Cell { on: true, value: 650.0 };
        edited.description = "Narrow bolds".into();
        edited.changed = true;
        call(&mut app, "engine.execute", json!({"command": "font.setCrossAxisMappings", "params": m.params(&edited)}));
        let stored = mappings(app.session.font().unwrap());
        assert_eq!(stored.description.as_deref(), Some("Narrow bolds"));
        assert_eq!(stored.mappings[0].output, vec![("wght".to_string(), 650.0), ("wdth".to_string(), 80.0)]);

        // A changed draft the font now holds follows the font again; an unchanged one follows an undo.
        let after = model(&app);
        let followed = after.follow(Some(edited));
        assert!(!followed.changed);
        assert_eq!(followed.description, "Narrow bolds");
        call(&mut app, "engine.execute", json!({"command": "edit.undo", "params": {}}));
        let undone = model(&app);
        assert_eq!(undone.follow(Some(followed)).description, "");

        call(&mut app, "engine.execute", json!({"command": ID, "params": {"open": false}}));
        assert!(!app.view.cross_axis.open);
    }

    #[test]
    fn window_cross_axis_mappings_needs_a_document() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        call(&mut app, "engine.execute", json!({"command": ID, "params": {}}));
        assert_eq!(call(&mut app, "ui.inspect", json!({}))["crossAxisMappings"]["mappings"], json!([]));
        let mut empty = FontCraftApp::new(Session::new(), Arc::new(Host::builder(APP).build()));
        assert!(control::dispatch(&mut empty, "engine.execute", &json!({"command": ID, "params": {}})).is_err(), "no document, no window");
        assert!(!empty.view.cross_axis.open);
    }

    #[test]
    fn the_window_view_edits_the_draft_and_apply_posts_it() {
        let app = variable_app();
        let m = model(&app);
        let draft = Rc::new(RefCell::new(m.read_draft()));
        let posted = Rc::new(RefCell::new(Vec::new()));
        let (seen, kept) = (Rc::clone(&posted), Rc::clone(&draft));
        let shown = m.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &shown, &mut kept.borrow_mut(), &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let add = fixture.by_label("New Cross-Axis Mapping").expect("the New Cross-Axis Mapping button");
        fixture.click(&add).settle();
        assert_eq!(draft.borrow().cards.len(), 1);
        assert!(draft.borrow().changed);
        assert_eq!(draft.borrow().cards[0].input, vec![Cell { on: false, value: 400.0 }, Cell { on: false, value: 100.0 }]);

        let apply = fixture.by_label("Apply").expect("the Apply button");
        fixture.click(&apply).settle();
        let expected = Msg::run("font.setCrossAxisMappings", json!({"mappings": [{"input": {}, "output": {}}]}));
        assert_eq!(posted.borrow().last(), Some(&expected));

        let revert = fixture.by_label("Revert").expect("the Revert button");
        fixture.click(&revert).settle();
        assert!(draft.borrow().cards.is_empty(), "Revert drops the draft's changes");
        assert!(!draft.borrow().changed);
    }

    /// A card's disclosure opens or closes that card; with Alt held it opens or closes every card.
    #[test]
    fn alt_click_on_a_disclosure_opens_or_closes_every_card() {
        let app = variable_app();
        let m = model(&app);
        let mut start = m.read_draft();
        start.cards = vec![m.new_card(), m.new_card()];
        let draft = Rc::new(RefCell::new(start));
        let kept = Rc::clone(&draft);
        let mut fixture = Fixture::new(move |ui| view(ui, &m, &mut kept.borrow_mut(), &mut Vec::new()));
        fixture.settle();
        let opens = |d: &Draft| d.cards.iter().map(|c| c.open).collect::<Vec<_>>();
        let first = fixture.by_label("Axes").expect("the first card's disclosure");
        fixture.click(&first).settle();
        assert_eq!(opens(&draft.borrow()), [false, true], "a plain click closes its own card");
        fixture.click(&first).settle();
        fixture.hold(egui::Modifiers::ALT);
        let first = fixture.by_label("Axes").expect("the first card's disclosure");
        fixture.click(&first).settle();
        assert_eq!(opens(&draft.borrow()), [false, false], "Alt closes every card");
    }
}

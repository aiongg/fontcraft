//! The TTFZones and TTFStems editors: a master's TT zones (or TT stems) as rows, each control running one of the TT
//! definition commands (`hint.tt.*`, `fontcraft_engine::cmd::hint_tt_defs`).
//!
//! | Control | Command |
//! |---|---|
//! | Name, Position, Size of a row | `hint.tt.setZone` (`hint.tt.setStemDefinition`) with that key |
//! | Align of a zone row | `hint.tt.setZone {align}`, `null` for None |
//! | orientation button of a stem row (↔ X, ↕ Y) | `hint.tt.setStemDefinition {orientation}`, the other one |
//! | delta button, then a cell of the Delta table ([`super::deltas`]) | `… {deltas}` |
//! | filter button, then OK of the filter sheet | `… {filter}`; Clear sends `null` |
//! | plus | `hint.tt.addZone` at position 0, 16 units per 1000 high; `hint.tt.addStemDefinition`, an X stem 80 wide |
//! | minus | `hint.tt.removeZone` (`hint.tt.removeStemDefinition`) on the selected row |
//! | Get PS zones (stems) | `hint.tt.getPsZones` (`hint.tt.getPsStems`) |
//!
//! Each control runs its command at once, one undo step each; Done or Escape closes the editor. A click on a row's
//! number selects the row. The delta and filter buttons show on the main master only (Get Hints From Master, else the
//! first master). The Delta table's columns are the font's masters, plus any column the entry stores that names no
//! master, captioned with its key so its cells can be cleared. The Delta table and the filter sheet show in the
//! editor in place of the rows while they are open.
//!
//! The filter sheet edits the stored glyph predicate as text; an empty text clears the filter.
//!
//! Rows: `dialog.ttfZones`, `dialog.ttfStems` (M10.28).

use std::collections::BTreeMap;

use craft::ui::buttons::Button;
use craft::ui::contract::Entry as Choice;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use fontcraft_engine::hinting::settings::{TTF_STEMS, TTF_ZONES, main_master};
use fontcraft_engine::hinting::{DeltaDir, Deltas, HintSettings, TtStem, TtZone};
use fontcraft_engine::model::Font;
use serde_json::{Map, Value, json};

use super::deltas::{self, Cell, Click, Column, DeltaTable, Outcome};
use crate::app::Msg;

/// The plus button's new zone, in units per 1000 units per em.
const NEW_ZONE_SIZE: f64 = 16.0;
/// The plus button's new stem, in units per 1000 units per em.
const NEW_STEM_SIZE: f64 = 80.0;

craft::ui::words! {
    /// The words of the TTFZones and TTFStems editors.
    pub struct ZonesWords {
        /// Shown under the title off the main master.
        main_only: "fontcraftTtfZones.mainOnly" = "Deltas and filters are set on the main master.";
        /// The Name caption and field.
        name: "fontcraftTtfZones.name" = "Name";
        /// The Position caption and field.
        position: "fontcraftTtfZones.position" = "Position";
        /// The Size caption and field.
        size: "fontcraftTtfZones.size" = "Size";
        /// The Align caption and pop-up.
        align: "fontcraftTtfZones.align" = "Align";
        /// The Align entry for no alignment.
        none: "fontcraftTtfZones.none" = "None";
        /// The empty zone list.
        no_zones: "fontcraftTtfZones.noZones" = "No TT zones: add one with +, or get the PostScript ones.";
        /// The empty stem list.
        no_stems: "fontcraftTtfZones.noStems" = "No TT stems: add one with +, or get the PostScript ones.";
        /// The tooltip of a row's number.
        select_tip: "fontcraftTtfZones.selectTip" = "Select this row";
        /// A Y stem's orientation button.
        y_stem: "fontcraftTtfZones.yStem" = "↕";
        /// An X stem's orientation button.
        x_stem: "fontcraftTtfZones.xStem" = "↔";
        /// The tooltip of a Y stem's orientation button.
        y_stem_tip: "fontcraftTtfZones.yStemTip" = "Y stem: the thickness of a horizontal stroke. Click for an X stem";
        /// The tooltip of an X stem's orientation button.
        x_stem_tip: "fontcraftTtfZones.xStemTip" = "X stem: the thickness of a vertical stroke. Click for a Y stem";
        /// A row's delta button.
        deltas: "fontcraftTtfZones.deltas" = "Δ";
        /// The tooltip of a row's delta button.
        deltas_tip: "fontcraftTtfZones.deltasTip" = "Deltas by pixel size";
        /// A row's filter button.
        filter: "fontcraftTtfZones.filter" = "Filter";
        /// The tooltip of a row's filter button.
        filter_tip: "fontcraftTtfZones.filterTip" = "The glyphs this entry applies to";
        /// The plus button.
        plus: "fontcraftTtfZones.plus" = "+";
        /// The tooltip of the plus button.
        plus_tip: "fontcraftTtfZones.plusTip" = "Add an entry to every master";
        /// The minus button.
        minus: "fontcraftTtfZones.minus" = "−";
        /// The tooltip of the minus button.
        minus_tip: "fontcraftTtfZones.minusTip" = "Remove the selected entry from every master";
        /// The Get PS zones button.
        get_ps_zones: "fontcraftTtfZones.getPsZones" = "Get PS zones";
        /// The Get PS stems button.
        get_ps_stems: "fontcraftTtfZones.getPsStems" = "Get PS stems";
        /// The tooltip of the Get PS button.
        get_ps_tip: "fontcraftTtfZones.getPsTip" = "Replace this master's entries with its PostScript ones";
        /// The Done button.
        done: "fontcraftTtfZones.done" = "Done";
        /// The filter sheet's title.
        filter_title: "fontcraftTtfZones.filterTitle" = "Glyph Filter";
        /// The filter sheet's explanation.
        filter_note: "fontcraftTtfZones.filterNote" = "The entry applies to the glyphs this predicate matches; empty applies it to every glyph.";
        /// The filter sheet's predicate field.
        predicate: "fontcraftTtfZones.predicate" = "Predicate";
        /// The filter sheet's OK.
        ok: "fontcraftTtfZones.ok" = "OK";
        /// The filter sheet's Clear.
        clear: "fontcraftTtfZones.clear" = "Clear";
        /// The filter sheet's Cancel.
        cancel: "fontcraftTtfZones.cancel" = "Cancel";
        /// The Delta table's title.
        deltas_of: template "fontcraftTtfZones.deltasOf" = "Deltas of {name}";
    }
}

/// Which list an editor shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Zones,
    Stems,
}

impl Kind {
    /// The master parameter that holds the list.
    pub fn parameter(self) -> &'static str {
        match self {
            Kind::Zones => TTF_ZONES,
            Kind::Stems => TTF_STEMS,
        }
    }

    /// The editor's row in `spec/inventory`, also the salt of its control ids.
    pub fn row(self) -> &'static str {
        match self {
            Kind::Zones => "dialog.ttfZones",
            Kind::Stems => "dialog.ttfStems",
        }
    }

    /// The key that names an entry in the commands' parameters.
    fn entry_key(self) -> &'static str {
        match self {
            Kind::Zones => "zone",
            Kind::Stems => "stem",
        }
    }

    fn add(self) -> &'static str {
        match self {
            Kind::Zones => "hint.tt.addZone",
            Kind::Stems => "hint.tt.addStemDefinition",
        }
    }

    fn remove(self) -> &'static str {
        match self {
            Kind::Zones => "hint.tt.removeZone",
            Kind::Stems => "hint.tt.removeStemDefinition",
        }
    }

    fn set(self) -> &'static str {
        match self {
            Kind::Zones => "hint.tt.setZone",
            Kind::Stems => "hint.tt.setStemDefinition",
        }
    }

    fn get_ps(self) -> &'static str {
        match self {
            Kind::Zones => "hint.tt.getPsZones",
            Kind::Stems => "hint.tt.getPsStems",
        }
    }
}

/// One TT zone or TT stem as the editor shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Zones: the flat edge.
    pub position: f64,
    /// Zones: positive for a top zone. Stems: the width.
    pub size: f64,
    /// Zones: the zone this one is aligned to.
    pub align: Option<String>,
    /// Stems: measured vertically, a Y stem.
    pub horizontal: bool,
    pub deltas: Deltas,
    pub filter: Option<String>,
}

impl Entry {
    fn zone(z: &TtZone) -> Self {
        Self {
            name: z.name.clone(),
            position: z.position,
            size: z.size,
            align: z.align.clone(),
            deltas: z.deltas.clone(),
            filter: z.filter.clone(),
            ..Self::default()
        }
    }

    fn stem(s: &TtStem) -> Self {
        Self { name: s.name.clone(), size: s.size, horizontal: s.horizontal, deltas: s.deltas.clone(), filter: s.filter.clone(), ..Self::default() }
    }

    pub fn has_deltas(&self) -> bool {
        self.deltas.values().any(|sizes| !sizes.is_empty())
    }
}

/// What the person did in the editor, before it becomes a command.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Name(usize, String),
    Position(usize, f64),
    Size(usize, f64),
    /// `None`: no alignment.
    Align(usize, Option<String>),
    /// Stems: turn X into Y and back.
    Orientation(usize),
    Add,
    Remove(usize),
    GetPs,
    Delta(usize, Click),
    /// `None`: clear the filter.
    Filter(usize, Option<String>),
}

/// The list an editor shows: one master's TT zones or TT stems.
#[derive(Clone, Debug, PartialEq)]
pub struct List {
    pub kind: Kind,
    /// The master's id.
    pub master: String,
    pub master_name: String,
    /// The master is the main master, the only one with deltas and filters.
    pub main: bool,
    pub entries: Vec<Entry>,
    /// The Delta table's columns: the font's masters.
    pub masters: Vec<Column>,
    /// Units per em over 1000, which scales the plus button's sizes.
    pub scale: f64,
}

impl List {
    /// The list of `master` (an id or a name).
    pub fn read(font: &Font, kind: Kind, master: &str) -> Result<List, String> {
        let m = font.master(master).map_err(|e| e.to_string())?;
        let settings = HintSettings::for_master(font, &m.id).map_err(|e| e.to_string())?;
        let entries = match kind {
            Kind::Zones => settings.tt_zones.iter().map(Entry::zone).collect(),
            Kind::Stems => settings.tt_stems.iter().map(Entry::stem).collect(),
        };
        let masters = font.masters.iter().take(deltas::MAX_COLUMNS).map(|m| Column { key: m.id.clone(), label: m.name.clone() }).collect();
        Ok(List {
            kind,
            master: m.id.clone(),
            master_name: m.name.clone(),
            main: main_master(font) == m.id,
            entries,
            masters,
            scale: f64::from(font.units_per_em) / 1000.0,
        })
    }

    /// The Delta table of the entry at `at`, titled `title`.
    pub fn delta_table(&self, at: usize, title: String) -> Option<DeltaTable> {
        let entry = self.entries.get(at)?;
        let mut columns = self.masters.clone();
        for key in entry.deltas.keys() {
            if columns.len() < deltas::MAX_COLUMNS && !columns.iter().any(|c| &c.key == key) {
                columns.push(Column { key: key.clone(), label: key.clone() });
            }
        }
        let mut cells = BTreeMap::new();
        for (index, column) in columns.iter().enumerate() {
            let Some(sizes) = entry.deltas.get(&column.key) else { continue };
            for (ppm, dir) in sizes.range(deltas::FIRST_PPM..=deltas::LAST_PPM) {
                let cell = match dir {
                    DeltaDir::Up => Cell::Up,
                    DeltaDir::Down => Cell::Down,
                };
                cells.insert((index, *ppm), cell);
            }
        }
        Some(DeltaTable { title, columns, cells })
    }

    /// The command `control` stands for; `None` when it changes nothing or does not apply (a stale row, a delta or
    /// filter outside the main master).
    pub fn command(&self, control: &Control) -> Option<(&'static str, Value)> {
        let kind = self.kind;
        let mut p = Map::new();
        p.insert("master".into(), json!(self.master));
        let entry = |at: &usize| self.entries.get(*at);
        let mut set = |at: &usize, key: &str, value: Value| -> Option<&'static str> {
            p.insert(kind.entry_key().into(), json!(entry(at)?.name));
            p.insert(key.into(), value);
            Some(kind.set())
        };
        let id = match control {
            Control::Name(at, text) => {
                let text = text.trim();
                if text.is_empty() || text == entry(at)?.name {
                    return None;
                }
                set(at, "name", json!(text))?
            }
            Control::Position(at, n) if kind == Kind::Zones => set(at, "position", json!(n))?,
            Control::Size(at, n) => set(at, "size", json!(n))?,
            Control::Align(at, to) if kind == Kind::Zones => {
                if entry(at)?.align == *to {
                    return None;
                }
                set(at, "align", json!(to))?
            }
            Control::Orientation(at) if kind == Kind::Stems => {
                let orientation = if entry(at)?.horizontal { "x" } else { "y" };
                set(at, "orientation", json!(orientation))?
            }
            Control::Delta(at, click) if self.main => {
                let cells = self.delta_table(*at, String::new())?.cells_param(click)?;
                set(at, "deltas", cells)?
            }
            Control::Filter(at, filter) if self.main => {
                if entry(at)?.filter == *filter {
                    return None;
                }
                set(at, "filter", json!(filter))?
            }
            Control::Remove(at) => {
                p.insert(kind.entry_key().into(), json!(entry(at)?.name));
                kind.remove()
            }
            Control::Add => {
                match kind {
                    Kind::Zones => {
                        p.insert("position".into(), json!(0.0));
                        p.insert("size".into(), json!((NEW_ZONE_SIZE * self.scale).round()));
                    }
                    Kind::Stems => {
                        p.insert("orientation".into(), json!("x"));
                        p.insert("size".into(), json!((NEW_STEM_SIZE * self.scale).round()));
                    }
                }
                kind.add()
            }
            Control::GetPs => kind.get_ps(),
            Control::Position(..) | Control::Align(..) | Control::Orientation(_) | Control::Delta(..) | Control::Filter(..) => return None,
        };
        Some((id, Value::Object(p)))
    }
}

/// What shows in place of the rows.
#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    /// The Delta table of the row at this index.
    Deltas(usize),
    /// The filter sheet of a row: the predicate being typed.
    Filter { at: usize, text: String },
}

/// The editor's own state: the selected row and the open sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Editor {
    pub selected: Option<usize>,
    pub sheet: Option<Sheet>,
}

impl Editor {
    /// Drops a selection or sheet that points past the list, and a sheet off the main master.
    fn fit(&mut self, list: &List) {
        let count = list.entries.len();
        self.selected = self.selected.filter(|at| *at < count);
        let fits = match &self.sheet {
            Some(Sheet::Deltas(at)) | Some(Sheet::Filter { at, .. }) => list.main && *at < count,
            None => true,
        };
        if !fits {
            self.sheet = None;
        }
    }
}

/// The id of a control of the row at `at`: `select`, `name`, `position`, `size`, `align`, `orientation`, `deltas` or
/// `filter`.
pub fn row_id(kind: Kind, at: usize, part: &str) -> egui::Id {
    egui::Id::unique((kind.row(), at, part))
}

/// The id of a button under the rows (`plus`, `minus`, `getPs`) or of the filter sheet (`ok`, `clear`, `cancel`,
/// `predicate`).
pub fn button_id(kind: Kind, part: &str) -> egui::Id {
    egui::Id::unique((kind.row(), part))
}

/// Shows the editor's body over `list`; what the person did goes to `controls`.
pub fn view(ui: &mut Ui, list: &List, editor: &mut Editor, controls: &mut Vec<Control>) {
    editor.fit(list);
    let words = Env::of(ui).words::<ZonesWords>();
    match editor.sheet.clone() {
        Some(Sheet::Deltas(at)) => {
            let name = list.entries.get(at).map(|e| e.name.as_str()).unwrap_or_default();
            let title = Label::from(words.deltas_of.fill(&[("name", name)])).as_str().to_string();
            match list.delta_table(at, title).and_then(|table| deltas::view(ui, &table)) {
                Some(Outcome::Click(click)) => controls.push(Control::Delta(at, click)),
                Some(Outcome::Close) => editor.sheet = None,
                None => {}
            }
        }
        Some(Sheet::Filter { at, mut text }) => {
            let outcome = filter_sheet(ui, &words, list.kind, &mut text);
            editor.sheet = Some(Sheet::Filter { at, text });
            if let Some(set) = outcome {
                if let Some(filter) = set {
                    controls.push(Control::Filter(at, filter));
                }
                editor.sheet = None;
            }
        }
        None => rows(ui, &words, list, editor, controls),
    }
    // The rows move: a removal or a new list takes the selection and the sheets with it.
    if controls.iter().any(|c| matches!(c, Control::Remove(_) | Control::GetPs)) {
        *editor = Editor::default();
    }
}

/// The rows and the buttons under them.
fn rows(ui: &mut Ui, words: &ZonesWords, list: &List, editor: &mut Editor, controls: &mut Vec<Control>) {
    let kind = list.kind;
    Stack::new().show(ui, |ui, _| {
        if !list.main {
            Text::new(words.main_only.clone()).shade(Shade::Muted).show(ui);
        }
        Row::new().show(ui, |ui, _| {
            let captions = match kind {
                Kind::Zones => vec![&words.name, &words.position, &words.size, &words.align],
                Kind::Stems => vec![&words.name, &words.size],
            };
            for caption in captions {
                Text::new(caption.clone()).shade(Shade::Muted).show(ui);
            }
        });
        Stack::new().scroll(craft::ui::contract::Scroll::vertical()).show(ui, |ui, _| {
            if list.entries.is_empty() {
                let empty = if kind == Kind::Zones { &words.no_zones } else { &words.no_stems };
                Text::new(empty.clone()).shade(Shade::Muted).show(ui);
            }
            for (at, entry) in list.entries.iter().enumerate() {
                Row::new().show(ui, |ui, _| row(ui, words, list, at, entry, editor, controls));
            }
        });
        Row::new().show(ui, |ui, _| {
            let get_ps = if kind == Kind::Zones { &words.get_ps_zones } else { &words.get_ps_stems };
            if Button::new(get_ps.clone()).id(button_id(kind, "getPs")).tooltip(words.get_ps_tip.clone()).show(ui).clicked() {
                controls.push(Control::GetPs);
            }
            if Button::new(words.plus.clone()).id(button_id(kind, "plus")).tooltip(words.plus_tip.clone()).show(ui).clicked() {
                controls.push(Control::Add);
            }
            let minus =
                Button::new(words.minus.clone()).id(button_id(kind, "minus")).tooltip(words.minus_tip.clone()).enabled(editor.selected.is_some());
            if minus.show(ui).clicked()
                && let Some(at) = editor.selected
            {
                controls.push(Control::Remove(at));
            }
        });
    });
}

/// One entry's controls.
fn row(ui: &mut Ui, words: &ZonesWords, list: &List, at: usize, entry: &Entry, editor: &mut Editor, controls: &mut Vec<Control>) {
    let kind = list.kind;
    let number = at.saturating_add(1).to_string();
    let select = Button::new(number).id(row_id(kind, at, "select")).tooltip(words.select_tip.clone()).selected(editor.selected == Some(at));
    if select.show(ui).clicked() {
        editor.selected = Some(at);
    }
    if kind == Kind::Stems {
        let (mark, tip) = if entry.horizontal { (&words.y_stem, &words.y_stem_tip) } else { (&words.x_stem, &words.x_stem_tip) };
        if Button::new(mark.clone()).id(row_id(kind, at, "orientation")).tooltip(tip.clone()).show(ui).clicked() {
            controls.push(Control::Orientation(at));
        }
    }
    let name = TextField::new(words.name.clone(), Bind::given(entry.name.clone())).id(row_id(kind, at, "name")).show(ui);
    if let Some(text) = name.events.committed() {
        controls.push(Control::Name(at, text.clone()));
    }
    if kind == Kind::Zones {
        let position = NumberField::new(words.position.clone(), Bind::given(Some(entry.position))).id(row_id(kind, at, "position")).show(ui);
        if let Some(Some(n)) = position.edit.events.committed() {
            controls.push(Control::Position(at, *n));
        }
    }
    let size = NumberField::new(words.size.clone(), Bind::given(Some(entry.size))).id(row_id(kind, at, "size")).show(ui);
    if let Some(Some(n)) = size.edit.events.committed() {
        controls.push(Control::Size(at, *n));
    }
    if kind == Kind::Zones {
        let mut choices = vec![Choice::new(None, words.none.clone())];
        choices.extend(list.entries.iter().filter(|e| e.name != entry.name).map(|e| Choice::new(Some(e.name.clone()), e.name.clone())));
        let align = Select::new(words.align.clone(), Bind::given(entry.align.clone()), &choices).id(row_id(kind, at, "align")).show(ui);
        if let Some(to) = align.events.committed() {
            controls.push(Control::Align(at, to.clone()));
        }
    }
    if list.main {
        let deltas = Button::new(words.deltas.clone()).id(row_id(kind, at, "deltas")).tooltip(words.deltas_tip.clone()).selected(entry.has_deltas());
        if deltas.show(ui).clicked() {
            editor.sheet = Some(Sheet::Deltas(at));
        }
        let filter =
            Button::new(words.filter.clone()).id(row_id(kind, at, "filter")).tooltip(words.filter_tip.clone()).selected(entry.filter.is_some());
        if filter.show(ui).clicked() {
            editor.sheet = Some(Sheet::Filter { at, text: entry.filter.clone().unwrap_or_default() });
        }
    }
}

/// The filter sheet over `text`. `Some(Some(filter))` stores a filter (`None` inside clears it), `Some(None)` closes
/// without a change.
fn filter_sheet(ui: &mut Ui, words: &ZonesWords, kind: Kind, text: &mut String) -> Option<Option<Option<String>>> {
    let mut outcome = None;
    Stack::new().show(ui, |ui, _| {
        Text::new(words.filter_title.clone()).show(ui);
        Text::new(words.filter_note.clone()).shade(Shade::Muted).show(ui);
        TextField::new(words.predicate.clone(), Bind::bound(&mut *text)).id(button_id(kind, "predicate")).mono().show(ui);
        Row::new().show(ui, |ui, _| {
            if Button::new(words.clear.clone()).id(button_id(kind, "clear")).show(ui).clicked() {
                outcome = Some(Some(None));
            }
            if Button::new(words.cancel.clone()).id(button_id(kind, "cancel")).show(ui).clicked() {
                outcome = Some(None);
            }
            if Button::new(words.ok.clone()).id(button_id(kind, "ok")).show(ui).clicked() {
                let typed = text.trim();
                outcome = Some(Some((!typed.is_empty()).then(|| typed.to_string())));
            }
        });
    });
    outcome
}

/// The open editor on the window's dialog host.
pub struct DefinitionsDialog {
    pub kind: Kind,
    /// The master's id.
    pub master: String,
    /// What the editor shows, refreshed by the dialog module each pass; `None` once the master is gone.
    pub list: Option<List>,
    pub editor: Editor,
    /// The commands of the controls used, which the dialog module posts.
    pub outbox: Vec<Msg>,
    open: bool,
}

impl DefinitionsDialog {
    pub fn new(kind: Kind, master: String) -> Self {
        DefinitionsDialog { kind, master, list: None, editor: Editor::default(), outbox: Vec::new(), open: true }
    }
}

impl StackDialog for DefinitionsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftTtfDefinitions")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        // The master went away (undo, another window): the editor goes with it.
        let Some(list) = self.list.clone() else { return Some(DialogResult::Cancel) };
        let words = Env::of(ui).words::<ZonesWords>();
        let title = format!("{}: {}", self.kind.parameter(), list.master_name);
        let mut controls = Vec::new();
        let DefinitionsDialog { editor, open, .. } = self;
        let shown = Dialog::new(title)
            .opened(Bind::bound(open))
            .resizable(true)
            .buttons([FooterButton::new(words.done.clone(), DialogRole::Accept)])
            .show(ui, |ui, _| view(ui, &list, editor, &mut controls));
        for control in &controls {
            if let Some((id, params)) = list.command(control) {
                self.outbox.push(Msg::run(id, params));
            }
        }
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};

    use super::*;

    fn zones(main: bool) -> List {
        let zone = |name: &str, position: f64, size: f64| Entry { name: name.into(), position, size, ..Entry::default() };
        List {
            kind: Kind::Zones,
            master: "m01".into(),
            master_name: "Regular".into(),
            main,
            entries: vec![zone("baseline", 0.0, -16.0), zone("xHeight", 500.0, 16.0)],
            masters: vec![Column { key: "m01".into(), label: "Regular".into() }, Column { key: "m02".into(), label: "Bold".into() }],
            scale: 1.0,
        }
    }

    /// Shows the editor body over `list` and collects the commands its controls stand for.
    fn fixture(list: List) -> (Fixture, Rc<RefCell<Vec<(&'static str, Value)>>>) {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut editor = Editor::default();
        let fixture = Fixture::new(move |ui| {
            let mut controls = Vec::new();
            view(ui, &list, &mut editor, &mut controls);
            seen.borrow_mut().extend(controls.iter().filter_map(|c| list.command(c)));
        });
        (fixture, posted)
    }

    #[test]
    fn plus_minus_and_get_ps_run_their_commands() {
        let (mut f, posted) = fixture(zones(true));
        f.settle();
        let plus = f.node_of(button_id(Kind::Zones, "plus")).expect("plus");
        f.click(&plus).settle();
        let select = f.node_of(row_id(Kind::Zones, 1, "select")).expect("row 2");
        f.click(&select).settle();
        let minus = f.node_of(button_id(Kind::Zones, "minus")).expect("minus");
        f.click(&minus).settle();
        let get_ps = f.node_of(button_id(Kind::Zones, "getPs")).expect("Get PS zones");
        f.click(&get_ps).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                ("hint.tt.addZone", json!({"master": "m01", "position": 0.0, "size": 16.0})),
                ("hint.tt.removeZone", json!({"master": "m01", "zone": "xHeight"})),
                ("hint.tt.getPsZones", json!({"master": "m01"})),
            ]
        );
    }

    #[test]
    fn a_typed_position_runs_set_zone() {
        let (mut f, posted) = fixture(zones(true));
        f.settle();
        let position = f.node_of(row_id(Kind::Zones, 1, "position")).expect("position of row 2");
        f.click(&position).key(Key::A, Modifiers::COMMAND).type_text("480").key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(*posted.borrow(), vec![("hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "position": 480.0}))]);
    }

    #[test]
    fn delta_and_filter_buttons_show_on_the_main_master_only() {
        let (mut main, _) = fixture(zones(true));
        main.settle();
        assert!(main.node_of(row_id(Kind::Zones, 0, "deltas")).is_some());
        assert!(main.node_of(row_id(Kind::Zones, 0, "filter")).is_some());
        let (mut other, _) = fixture(zones(false));
        other.settle();
        assert!(other.node_of(row_id(Kind::Zones, 0, "deltas")).is_none());
        assert!(other.node_of(row_id(Kind::Zones, 0, "filter")).is_none());
    }

    #[test]
    fn a_delta_cell_sets_the_zone_deltas_of_its_master() {
        let (mut f, posted) = fixture(zones(true));
        f.room(egui::Vec2::new(800.0, 3000.0)).settle();
        let open = f.node_of(row_id(Kind::Zones, 1, "deltas")).expect("delta button");
        f.click(&open).settle();
        let cell = f.node_of(deltas::cell_id(1, 12)).expect("Bold at 12 PPM");
        f.click(&cell).settle();
        assert_eq!(*posted.borrow(), vec![("hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "deltas": {"m02": {"12": "up"}}}))]);
    }

    #[test]
    fn the_filter_sheet_stores_and_clears_the_predicate() {
        let (mut f, posted) = fixture(zones(true));
        f.settle();
        let open = f.node_of(row_id(Kind::Zones, 0, "filter")).expect("filter button");
        f.click(&open).settle();
        let field = f.node_of(button_id(Kind::Zones, "predicate")).expect("predicate field");
        f.click(&field).type_text("category == \"Letter\"").settle();
        let ok = f.node_of(button_id(Kind::Zones, "ok")).expect("OK");
        f.click(&ok).settle();
        assert_eq!(*posted.borrow(), vec![("hint.tt.setZone", json!({"master": "m01", "zone": "baseline", "filter": "category == \"Letter\""}))]);
    }

    #[test]
    fn stem_orientation_and_new_stem_follow_the_units_per_em() {
        let mut list = zones(true);
        list.kind = Kind::Stems;
        list.scale = 2.048;
        list.entries = vec![Entry { name: "x1".into(), size: 160.0, ..Entry::default() }];
        assert_eq!(
            list.command(&Control::Orientation(0)),
            Some(("hint.tt.setStemDefinition", json!({"master": "m01", "stem": "x1", "orientation": "y"})))
        );
        assert_eq!(list.command(&Control::Add), Some(("hint.tt.addStemDefinition", json!({"master": "m01", "orientation": "x", "size": 164.0}))));
        assert_eq!(list.command(&Control::Position(0, 3.0)), None);
        assert_eq!(list.command(&Control::Name(0, " x1 ".into())), None);
    }
}

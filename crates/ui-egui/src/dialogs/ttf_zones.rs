//! The TTFZones editor, and the frame it shares with the TTFStems editor ([`super::ttf_stems`]):
//! a master's TT zones (or TT stems) as rows, each control running one of the TT definition
//! commands (`hint.tt.*`, `fontcraft_engine::cmd::hint_tt_defs`).
//!
//! The editor opens from the value field of a master's TTFZones or TTFStems parameter in Font
//! Info › Masters ([`value_field`]) on the master the tab shows, and lists the master's TT zones
//! as hinting reads them: the parameter's entries, else the zones derived from its alignment
//! zones.
//!
//! | Control | Command |
//! |---|---|
//! | Name, Position, Size of a row | `hint.tt.setZone` with that key |
//! | Align pop-up of a row | `hint.tt.setZone {align}`, `null` for None |
//! | delta button, then a cell of the Delta table ([`super::tt_deltas`]) | `hint.tt.setZone {deltas}` |
//! | globe button, then OK of the filter sheet | `hint.tt.setZone {filter}`; Clear sends `null` |
//! | plus | `hint.tt.addZone` at position 0, 16 units per 1000 high |
//! | minus | `hint.tt.removeZone` on the selected row |
//! | actions menu › Get PS zones | `hint.tt.getPsZones` |
//!
//! Each control runs its command at once, one undo step each; Done, Escape or a click outside
//! closes the editor. A click on a row's number selects the row.
//!
//! The delta and globe buttons show on the main master only (Get Hints From Master, else the
//! first master), drawn pressed when the entry has a delta or a filter. The Delta table's columns
//! are the font's masters, plus any column the entry stores that names no master (an instance
//! name), captioned with its key so its cells can be cleared.
//!
//! The filter sheet is the Smart Filter rule editor ([`super::rule_tree`]); OK turns the rules
//! into the glyph predicate the command stores, and a sheet without rules clears the filter. A
//! stored predicate is not turned back into rules: it opens as one Custom rule holding it.
//!
//! Rows: `dialog.ttfZones` (M10.28).

use std::collections::BTreeMap;

use egui::{Align2, FontId, Id, RichText, Sense};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::{Row, RowKind};
use fontcraft_engine::hinting::settings::{TTF_STEMS, TTF_ZONES, main_master};
use fontcraft_engine::hinting::{DeltaDir, Deltas, HintSettings, TtStem, TtZone};
use fontcraft_engine::model::Font;
use fontcraft_glyphquery::RuleTree;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::tt_deltas::{self, Cell, Click, Column, DeltaTable, Outcome};
use super::{Event, rule_tree};
use crate::panels::Pending;
use crate::panels::widgets::{self, NumberEdit};
use crate::theme::Tokens;

/// Rows visible before the list scrolls.
const VISIBLE_ROWS: f32 = 12.0;
/// A name field and the Align pop-up are this many fields wide.
const NAME_FIELDS: f32 = 2.0;
/// The plus button's new zone, in units per 1000 units per em.
const NEW_ZONE_SIZE: f64 = 16.0;

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

    pub fn of_parameter(name: &str) -> Option<Kind> {
        [Kind::Zones, Kind::Stems].into_iter().find(|k| k.parameter() == name)
    }

    /// The editor's row in `spec/inventory`, also the salt of its widget ids.
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

    fn noun(self, count: usize) -> &'static str {
        match (self, count == 1) {
            (Kind::Zones, true) => "zone",
            (Kind::Zones, false) => "zones",
            (Kind::Stems, true) => "stem",
            (Kind::Stems, false) => "stems",
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

    fn get_ps_label(self) -> &'static str {
        match self {
            Kind::Zones => "Get PS zones",
            Kind::Stems => "Get PS stems",
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
    scale: f64,
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
        let masters = font.masters.iter().take(tt_deltas::MAX_COLUMNS).map(|m| Column { key: m.id.clone(), label: m.name.clone() }).collect();
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

    /// The Delta table of the entry at `at`.
    pub fn delta_table(&self, at: usize) -> Option<DeltaTable> {
        let entry = self.entries.get(at)?;
        let mut columns = self.masters.clone();
        for key in entry.deltas.keys() {
            if columns.len() < tt_deltas::MAX_COLUMNS && !columns.iter().any(|c| &c.key == key) {
                columns.push(Column { key: key.clone(), label: key.clone() });
            }
        }
        let mut cells = BTreeMap::new();
        for (index, column) in columns.iter().enumerate() {
            let Some(sizes) = entry.deltas.get(&column.key) else { continue };
            for (ppm, dir) in sizes.range(tt_deltas::FIRST_PPM..=tt_deltas::LAST_PPM) {
                let cell = match dir {
                    DeltaDir::Up => Cell::Up,
                    DeltaDir::Down => Cell::Down,
                };
                cells.insert((index, *ppm), cell);
            }
        }
        Some(DeltaTable { title: format!("Deltas of {}", entry.name), columns, cells })
    }

    /// The command `control` stands for; `None` when it changes nothing or does not apply (a
    /// stale row, a delta or filter outside the main master).
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
                let cells = self.delta_table(*at)?.cells_param(click)?;
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
                        p.insert("size".into(), json!((super::ttf_stems::NEW_STEM_SIZE * self.scale).round()));
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

// ── The open editor ─────────────────────────────────────────────────────────────────────────────

/// The open editor, kept in egui's memory.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    pub kind: Kind,
    /// The master's id.
    pub master: String,
    pub selected: Option<usize>,
    /// The row whose Delta table is open.
    pub deltas: Option<usize>,
    pub filter: Option<FilterSheet>,
}

/// The open filter sheet of one row.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterSheet {
    pub at: usize,
    /// The rules, in the shape the rule editor draws.
    pub rules: Value,
    pub error: Option<String>,
}

impl FilterSheet {
    /// The sheet of the row at `at`, whose stored predicate is `filter`.
    pub fn new(at: usize, filter: Option<&str>) -> Self {
        let rules = match filter {
            Some(text) => json!({"all": [{"property": "custom", "op": "is", "value": text}]}),
            None => RuleTree::starter().to_json(),
        };
        Self { at, rules, error: None }
    }

    /// The predicate the rules stand for; `None`, which clears the filter, without rules.
    pub fn predicate(&self) -> Result<Option<String>, String> {
        if rule_tree::group_of(&self.rules).is_some_and(|(_, nodes)| nodes.is_empty()) {
            return Ok(None);
        }
        let tree = RuleTree::from_json(&self.rules).map_err(|e| e.to_string())?;
        let predicate = tree.compile().map_err(|e| e.to_string())?;
        Ok(Some(predicate.to_string()))
    }
}

fn editor_memory() -> Id {
    widgets::id("dialog.ttfDefinitions.editor")
}

/// Where a value field leaves its request for an editor until [`show`] knows the master.
fn request_memory() -> Id {
    widgets::id("dialog.ttfDefinitions.request")
}

/// Opens the editor of `kind` on master `master` (an id).
pub fn open(ctx: &egui::Context, kind: Kind, master: &str) {
    let editor = Editor { kind, master: master.to_string(), selected: None, deltas: None, filter: None };
    ctx.data_mut(|d| d.insert_temp(editor_memory(), editor));
}

/// The open editor, if any.
pub fn open_editor(ctx: &egui::Context) -> Option<Editor> {
    ctx.data(|d| d.get_temp::<Editor>(editor_memory()))
}

pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Editor>(editor_memory()));
}

// ── Ids ─────────────────────────────────────────────────────────────────────────────────────────

pub fn modal_id(kind: Kind) -> Id {
    widgets::id((kind.row(), "modal"))
}

/// The id of a control of the row at `at`: `select`, `name`, `position`, `size`, `align`,
/// `orientation`, `deltas` or `filter`.
pub fn row_id(kind: Kind, at: usize, part: &str) -> Id {
    widgets::id((kind.row(), at, part))
}

pub fn plus_id(kind: Kind) -> Id {
    widgets::id((kind.row(), "plus"))
}

pub fn minus_id(kind: Kind) -> Id {
    widgets::id((kind.row(), "minus"))
}

/// The round actions button at the bottom left.
pub fn actions_id(kind: Kind) -> Id {
    widgets::id((kind.row(), "actions"))
}

/// The Get PS zones (stems) item of the actions menu.
pub fn get_ps_id(kind: Kind) -> Id {
    widgets::id((kind.row(), "getPs"))
}

pub fn done_id(kind: Kind) -> Id {
    widgets::id((kind.row(), "done"))
}

/// The key of the filter sheet's rule field; the rule editor's controls are
/// `rule_tree::control_id(filter_key(kind), path, part)`.
pub fn filter_key(kind: Kind) -> String {
    format!("{}.filter", kind.row())
}

/// The filter sheet's buttons: `ok`, `clear` or `cancel`.
pub fn filter_button_id(kind: Kind, button: &str) -> Id {
    widgets::id((kind.row(), "filterSheet", button))
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

/// The value field of a master's TTFZones or TTFStems parameter row: a button with the number of
/// entries that asks for the editor, which [`show`] opens on the master the tab shows. Draws
/// nothing and returns false for any other row.
pub fn value_field(ui: &mut egui::Ui, id: Id, row: &Row) -> bool {
    if row.kind != RowKind::Param || row.scope != Scope::Master {
        return false;
    }
    let Some(kind) = Kind::of_parameter(&row.key) else { return false };
    let count = row.value.as_array().map_or(0, |a| a.items.len());
    let label = format!("{count} {}…", kind.noun(count));
    if widgets::button(ui, id, &label, &format!("Edit the master's {}", kind.parameter()), true, false) {
        ui.ctx().data_mut(|d| d.insert_temp(request_memory(), kind));
    }
    true
}

/// Draws the open editor and its sheets over everything else, opening the one a value field
/// asked for on `master` (the id of the master Font Info shows). The commands of this frame's
/// edits go to `pending`.
pub fn show(ctx: &egui::Context, tokens: &Tokens, font: &Font, master: Option<&str>, pending: &mut Pending) {
    if let Some(kind) = ctx.data(|d| d.get_temp::<Kind>(request_memory())) {
        ctx.data_mut(|d| d.remove::<Kind>(request_memory()));
        if let Some(master) = master {
            open(ctx, kind, master);
        }
    }
    let Some(mut editor) = open_editor(ctx) else { return };
    // The master went away (undo, another window): the editor goes with it.
    let Ok(list) = List::read(font, editor.kind, &editor.master) else {
        close(ctx);
        return;
    };
    let count = list.entries.len();
    editor.selected = editor.selected.filter(|at| *at < count);
    editor.deltas = editor.deltas.filter(|at| list.main && *at < count);
    editor.filter = editor.filter.take().filter(|f| list.main && f.at < count);
    let mut controls = Vec::new();
    let keep = draw(ctx, tokens, &list, &mut editor, &mut controls);
    for control in &controls {
        if let Some(command) = list.command(control) {
            pending.push(command);
        }
        // The rows move: a removal or a new list takes the selection and the sheets with it.
        if matches!(control, Control::Remove(_) | Control::GetPs) {
            editor.selected = None;
            editor.deltas = None;
            editor.filter = None;
        }
    }
    if keep {
        ctx.data_mut(|d| d.insert_temp(editor_memory(), editor));
    } else {
        close(ctx);
    }
}

/// Draws the editor and the open sheet; false when the editor closes.
fn draw(ctx: &egui::Context, tokens: &Tokens, list: &List, editor: &mut Editor, controls: &mut Vec<Control>) -> bool {
    let kind = list.kind;
    let sheet_open = editor.deltas.is_some() || editor.filter.is_some();
    let mut done = false;
    let modal = egui::Modal::new(modal_id(kind)).show(ctx, |ui| {
        ui.label(RichText::new(format!("{}: {}", kind.parameter(), list.master_name)).strong().size(tokens.dialog_title_size));
        if !list.main {
            ui.label(RichText::new("Deltas and filters are set on the main master.").color(tokens.text_dim));
        }
        ui.add_space(tokens.dialog_spacing);
        header(ui, tokens, kind);
        egui::ScrollArea::vertical().id_salt((kind.row(), "rows")).max_height(tokens.list_row_height * VISIBLE_ROWS).show(ui, |ui| {
            if list.entries.is_empty() {
                let empty = format!("No TT {}: add one with +, or get the PostScript ones from the actions menu.", kind.noun(0));
                ui.label(RichText::new(empty).color(tokens.text_dim));
            }
            for (at, entry) in list.entries.iter().enumerate() {
                ui.horizontal(|ui| {
                    if select_cell(ui, tokens, row_id(kind, at, "select"), at, editor.selected == Some(at)) {
                        editor.selected = Some(at);
                    }
                    match kind {
                        Kind::Zones => zone_row(ui, tokens, list, at, entry, controls),
                        Kind::Stems => super::ttf_stems::row(ui, tokens, at, entry, controls),
                    }
                    if list.main {
                        let what = kind.entry_key();
                        if widgets::button(
                            ui,
                            row_id(kind, at, "deltas"),
                            "Δ",
                            &format!("Deltas of this {what} by pixel size"),
                            true,
                            entry.has_deltas(),
                        ) {
                            editor.deltas = Some(at);
                        }
                        let hover = format!("The glyphs this {what} applies to");
                        if widgets::button(ui, row_id(kind, at, "filter"), "🌐", &hover, true, entry.filter.is_some()) {
                            editor.filter = Some(FilterSheet::new(at, entry.filter.as_deref()));
                        }
                    }
                });
            }
        });
        ui.add_space(tokens.dialog_spacing);
        let what = kind.entry_key();
        let selected = editor.selected;
        let (_, closed) = egui::Sides::new().show(
            ui,
            |ui| {
                actions_menu(ui, tokens, kind, controls);
                if widgets::button(ui, plus_id(kind), "+", &format!("Add a {what} to every master"), true, false) {
                    controls.push(Control::Add);
                }
                let hover = format!("Remove the selected {what} from every master");
                if widgets::button(ui, minus_id(kind), "−", &hover, selected.is_some(), false)
                    && let Some(at) = selected
                {
                    controls.push(Control::Remove(at));
                }
            },
            |ui| widgets::button(ui, done_id(kind), "Done", "Close the editor", true, false),
        );
        done = closed;
    });
    if let Some(at) = editor.deltas {
        match list.delta_table(at).map(|table| tt_deltas::show(ctx, tokens, &table)) {
            Some(Some(Outcome::Click(click))) => controls.push(Control::Delta(at, click)),
            Some(None) => {}
            Some(Some(Outcome::Close)) | None => editor.deltas = None,
        }
    }
    if let Some(sheet) = editor.filter.as_mut() {
        match filter_sheet(ctx, tokens, kind, sheet) {
            Some(SheetOutcome::Set(filter)) => {
                controls.push(Control::Filter(sheet.at, filter));
                editor.filter = None;
            }
            Some(SheetOutcome::Cancel) => editor.filter = None,
            None => {}
        }
    }
    !(done || (!sheet_open && modal.should_close()))
}

/// The column captions above the rows.
fn header(ui: &mut egui::Ui, tokens: &Tokens, kind: Kind) {
    let w = tokens.field_width;
    let caption = |ui: &mut egui::Ui, text: &str, width: f32| {
        ui.add_sized([width, tokens.list_row_height], egui::Label::new(RichText::new(text).color(tokens.text_dim)));
    };
    ui.horizontal(|ui| {
        caption(ui, "", select_width(ui));
        match kind {
            Kind::Zones => {
                caption(ui, "Name", w * NAME_FIELDS);
                caption(ui, "Position", w);
                caption(ui, "Size", w);
                caption(ui, "Align", w * NAME_FIELDS);
            }
            Kind::Stems => {
                caption(ui, "", ui.spacing().interact_size.y);
                caption(ui, "Name", w * NAME_FIELDS);
                caption(ui, "Size", w);
            }
        }
    });
}

fn select_width(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y * 1.5
}

/// The row's number, which a click selects the row by.
fn select_cell(ui: &mut egui::Ui, tokens: &Tokens, id: Id, at: usize, selected: bool) -> bool {
    let size = egui::vec2(select_width(ui), ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
        }
        let color = if selected { tokens.selected_text } else { tokens.text_dim };
        painter.text(rect.center(), Align2::CENTER_CENTER, at.saturating_add(1).to_string(), FontId::proportional(rect.height() * 0.6), color);
    }
    response.on_hover_text("Select this row").clicked()
}

/// A zone's Name, Position, Size and Align.
fn zone_row(ui: &mut egui::Ui, tokens: &Tokens, list: &List, at: usize, entry: &Entry, controls: &mut Vec<Control>) {
    let kind = Kind::Zones;
    let w = tokens.field_width;
    if let Some(name) = widgets::text(ui, row_id(kind, at, "name"), &entry.name, w * NAME_FIELDS) {
        controls.push(Control::Name(at, name));
    }
    if let Some(NumberEdit::Set(n)) = widgets::number(ui, row_id(kind, at, "position"), Some(entry.position), w, false) {
        controls.push(Control::Position(at, n));
    }
    if let Some(NumberEdit::Set(n)) = widgets::number(ui, row_id(kind, at, "size"), Some(entry.size), w, false) {
        controls.push(Control::Size(at, n));
    }
    let shown = entry.align.as_deref().unwrap_or("None");
    egui::ComboBox::from_id_salt(row_id(kind, at, "align")).selected_text(shown).width(w * NAME_FIELDS).show_ui(ui, |ui| {
        if ui.selectable_label(entry.align.is_none(), "None").clicked() {
            controls.push(Control::Align(at, None));
        }
        for other in list.entries.iter().filter(|e| e.name != entry.name) {
            let on = entry.align.as_deref() == Some(other.name.as_str());
            if ui.selectable_label(on, &other.name).clicked() {
                controls.push(Control::Align(at, Some(other.name.clone())));
            }
        }
    });
}

/// The round "…" button at the bottom left and its menu.
fn actions_menu(ui: &mut egui::Ui, tokens: &Tokens, kind: Kind, controls: &mut Vec<Control>) {
    let side = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    let response = ui.interact(rect, actions_id(kind), Sense::click()).on_hover_text("Actions");
    if ui.is_rect_visible(rect) {
        let color = ui.style().interact(&response).fg_stroke.color;
        let painter = ui.painter();
        painter.circle_stroke(rect.center(), side / 2.0 - 0.5, egui::Stroke::new(1.0, tokens.text_dim));
        for dx in [-1.0, 0.0, 1.0] {
            painter.circle_filled(rect.center() + egui::vec2(dx * tokens.dialog_dots_gap, 0.0), tokens.dialog_dot_radius, color);
        }
    }
    egui::Popup::menu(&response).show(|ui| {
        let hover = format!("Replace this master's TT {} with its PostScript ones", kind.noun(0));
        if widgets::button(ui, get_ps_id(kind), kind.get_ps_label(), &hover, true, false) {
            controls.push(Control::GetPs);
            ui.close();
        }
    });
}

/// How the person left the filter sheet.
enum SheetOutcome {
    /// OK or Clear: the filter to store, `None` for none.
    Set(Option<String>),
    Cancel,
}

/// The filter sheet: the Smart Filter rule editor over one entry's glyph filter.
fn filter_sheet(ctx: &egui::Context, tokens: &Tokens, kind: Kind, sheet: &mut FilterSheet) -> Option<SheetOutcome> {
    let mut outcome = None;
    let key = filter_key(kind);
    let what = kind.entry_key();
    let field = Field::rules(&key, "Glyphs", sheet.rules.clone(), &format!("The glyphs the {what} applies to"));
    let modal = egui::Modal::new(widgets::id((kind.row(), "filterSheet"))).show(ctx, |ui| {
        ui.label(RichText::new("Glyph Filter").strong().size(tokens.dialog_title_size));
        ui.label(RichText::new(format!("The {what} applies to the glyphs that match these rules.")).color(tokens.text_dim));
        ui.add_space(tokens.dialog_spacing);
        let mut events = Vec::new();
        rule_tree::draw(ui, &field, &sheet.rules, tokens, &mut events);
        for event in events {
            if let Event::Set(_, rules) = event {
                sheet.rules = rules;
                sheet.error = None;
            }
        }
        if let Some(error) = &sheet.error {
            ui.colored_label(tokens.error_text, error);
        }
        ui.add_space(tokens.dialog_spacing);
        let hover = format!("Remove the filter: the {what} applies to every glyph");
        let (clear, (ok, cancel)) = egui::Sides::new().show(
            ui,
            |ui| widgets::button(ui, filter_button_id(kind, "clear"), "Clear", &hover, true, false),
            |ui| {
                // Right to left: OK on the right, Cancel beside it.
                let ok = widgets::button(ui, filter_button_id(kind, "ok"), "OK", "Keep these rules", true, false);
                let cancel = widgets::button(ui, filter_button_id(kind, "cancel"), "Cancel", "Close without changing the filter", true, false);
                (ok, cancel)
            },
        );
        if clear {
            outcome = Some(SheetOutcome::Set(None));
        } else if cancel {
            outcome = Some(SheetOutcome::Cancel);
        } else if ok {
            match sheet.predicate() {
                Ok(predicate) => outcome = Some(SheetOutcome::Set(predicate)),
                Err(e) => sheet.error = Some(e),
            }
        }
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(SheetOutcome::Cancel);
    }
    outcome
}

#[cfg(test)]
mod tests;

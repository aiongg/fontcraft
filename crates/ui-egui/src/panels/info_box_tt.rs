//! The Info box while the TrueType Instructor is the chosen tool (`infoBox.ttInstructor`).
//!
//! The pixel size, the rendering intent and the hinting direction are always there; the other
//! controls follow the kinds of the selected instructions. Each control runs one command:
//!
//! | Control | Shown | Command |
//! |---|---|---|
//! | pixel size field (Up and Down step it) | always | `hint.tt.setPixelSize` |
//! | − and + beside it | always | `hint.tt.decreasePixelSize`, `hint.tt.increasePixelSize` |
//! | rendering intent pop-up | always | `hint.tt.setRenderingIntent` |
//! | direction button | always | `hint.tt.toggleDirection` with the other direction |
//! | zone pop-up: Auto, No Zone, the TT zones | a Snap selected | `hint.tt.setSnapZone` on the selected Snaps |
//! | stem pop-up: Auto, No Stem, the TT stems of the direction | a Stem selected | `hint.tt.setStemReference` on the selected Stems |
//! | five rounding buttons | a Snap, Stem, Shift or Interpolate selected | `hint.tt.setRounding` on those |
//! | Delta button and its table ([`crate::dialogs::tt_deltas`]) | a Delta selected | `hint.tt.setDeltas` on the selected Deltas |
//!
//! Each instruction command gets the edited glyph, the active layer and the positions of the
//! selected instructions of the kinds it accepts. A pop-up or a rounding button shows the value
//! the selected instructions share, and nothing when they differ. The zone pop-up lists the main
//! master's TT zones that apply to the glyph, and only for vertical Snaps (a named zone needs the
//! vertical direction); the stem pop-up lists the TT stems that suit the direction of every
//! selected Stem.
//!
//! The Delta table's columns are the font's static instances, the masters when it has none, plus
//! any column a selected Delta stores that matches neither, captioned with its stored key so it
//! can be cleared. A cell shows the value every selected Delta shares there.
//!
//! The pixel size, intent, direction and selected instructions are the Instructor's view state
//! ([`InstructorView`], read by [`view`]).

use std::collections::BTreeSet;

use fontcraft_engine::hinting::settings::main_master;
use fontcraft_engine::hinting::tt::delta_column_key;
use fontcraft_engine::hinting::{
    DeltaDir, Direction, HintSettings, MAX_INSTRUCTOR_PPM, MIN_INSTRUCTOR_PPM, Rounding, StemRef, TtInstruction, TtKind, ZoneRef, tt_instructions,
};
use fontcraft_model::Font;
use fontcraft_render::hinted::Intent;
use serde_json::{Value, json};

use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::dialogs::tt_deltas::{self, Cell, Click, Column, DeltaTable, Outcome};
use crate::theme::Tokens;

/// The tool whose Info box this is.
pub const TOOL: &str = "trueTypeInstructor";
/// The pixel size shown before the person picks one.
pub const DEFAULT_PPM: u16 = 12;

/// The Instructor's view state that the box shows and changes through UI commands.
#[derive(Clone, Debug, PartialEq)]
pub struct InstructorView {
    pub direction: Direction,
    pub ppm: u16,
    pub intent: Intent,
    /// The selected instructions, by position in the active layer's instruction list.
    pub selected: BTreeSet<usize>,
}

impl Default for InstructorView {
    fn default() -> Self {
        Self { direction: Direction::Vertical, ppm: DEFAULT_PPM, intent: Intent::Grayscale, selected: BTreeSet::new() }
    }
}

/// The Instructor's view state.
// M10.25: return the tool's direction, pixel size, intent and selected instructions.
pub fn view(_app: &FontcraftApp) -> InstructorView {
    InstructorView::default()
}

/// A choice in the zone or stem pop-up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    None,
    Named(String),
}

impl Choice {
    fn param(&self) -> &str {
        match self {
            Choice::Auto => "auto",
            Choice::None => "none",
            Choice::Named(name) => name,
        }
    }
}

/// One control of the box, as the person used it.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    PixelSize(f64),
    Smaller,
    Larger,
    Intent(Intent),
    ToggleDirection,
    Zone(Choice),
    Stem(Choice),
    Rounding(Rounding),
    Delta(Click),
}

/// The five rounding modes in the order of the buttons: (mode, parameter, label, hover).
pub const ROUNDINGS: [(Rounding, &str, &str, &str); 5] = [
    (Rounding::Round, "round", "Round", "Round"),
    (Rounding::Up, "up", "Up", "Round up (right in the horizontal direction)"),
    (Rounding::Down, "down", "Down", "Round down (left in the horizontal direction)"),
    (Rounding::NoRounding, "none", "None", "No rounding"),
    (Rounding::GdiOnly, "gdiOnly", "GDI", "Round only in GDI ClearType"),
];

/// The three rendering intents: (intent, parameter, label).
pub const INTENTS: [(Intent, &str, &str); 3] = [
    (Intent::Grayscale, "grayscale", "Grayscale"),
    (Intent::GdiClearType, "cleartype", "ClearType"),
    (Intent::DirectWrite, "directwrite", "DirectWrite"),
];

fn rounding_param(r: Rounding) -> &'static str {
    ROUNDINGS.iter().find(|(m, ..)| *m == r).map_or("round", |(_, p, ..)| p)
}

fn intent_label(i: Intent) -> &'static str {
    INTENTS.iter().find(|(m, ..)| *m == i).map_or("Grayscale", |(_, _, l)| l)
}

fn direction_param(d: Direction) -> &'static str {
    match d {
        Direction::Vertical => "vertical",
        Direction::Horizontal => "horizontal",
    }
}

/// The value the selected instructions share, if they share one.
fn shared<T: PartialEq + Clone>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

/// What the box needs, read before drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct TtBox {
    pub glyph: String,
    pub layer: String,
    pub view: InstructorView,
    /// Positions of the selected instructions by the commands that take them.
    pub snaps: Vec<usize>,
    pub stems: Vec<usize>,
    pub rounded: Vec<usize>,
    pub deltas: Vec<usize>,
    /// The shared zone and stem as the pop-ups show them.
    pub zone: Option<Choice>,
    pub stem: Option<Choice>,
    pub rounding: Option<Rounding>,
    /// The names the pop-ups offer.
    pub zones: Vec<String>,
    pub stem_names: Vec<String>,
    /// The Delta table of the selected Deltas.
    pub table: Option<DeltaTable>,
}

impl TtBox {
    /// Reads the box for the edited glyph's active layer; `None` without one.
    pub fn read(app: &FontcraftApp, view: &InstructorView) -> Option<TtBox> {
        let font = app.session.font().ok()?;
        let tab = app.state.tab()?;
        let glyph = tab.edited_glyph()?.to_string();
        let layer = crate::canvas::view::active_layer(font, tab)?;
        let list: Vec<TtInstruction> = tt_instructions(layer).map(|l| l.into_iter().map(|(_, t)| t).collect()).unwrap_or_default();
        let selected: Vec<(usize, &TtInstruction)> = view.selected.iter().filter_map(|&i| list.get(i).map(|t| (i, t))).collect();
        let of = |pick: fn(&TtKind) -> bool| selected.iter().filter(|(_, t)| pick(&t.kind)).map(|(i, _)| *i).collect::<Vec<_>>();
        let snaps = of(|k| matches!(k, TtKind::Snap { .. }));
        let stems = of(|k| matches!(k, TtKind::Stem { .. }));
        let deltas = of(|k| matches!(k, TtKind::Delta { .. }));
        let rounded = of(|k| !matches!(k, TtKind::Delta { .. }));
        let settings = HintSettings::for_master(font, &main_master(font)).ok();
        let zone_name = |i: usize| settings.as_ref().and_then(|s| s.tt_zones.get(i)).map_or_else(|| format!("Zone {}", i + 1), |z| z.name.clone());
        let stem_name = |i: usize| settings.as_ref().and_then(|s| s.tt_stems.get(i)).map_or_else(|| format!("Stem {}", i + 1), |z| z.name.clone());
        let zone = shared(selected.iter().filter_map(|(_, t)| match &t.kind {
            TtKind::Snap { zone } => Some(match zone {
                ZoneRef::Auto => Choice::Auto,
                ZoneRef::NoZone => Choice::None,
                ZoneRef::Index(i) => Choice::Named(zone_name(*i)),
            }),
            _ => None,
        }));
        let stem = shared(selected.iter().filter_map(|(_, t)| match &t.kind {
            TtKind::Stem { stem, .. } => Some(match stem {
                StemRef::Auto => Choice::Auto,
                StemRef::NoStem => Choice::None,
                StemRef::Index(i) => Choice::Named(stem_name(*i)),
            }),
            _ => None,
        }));
        let rounding = shared(selected.iter().filter(|(_, t)| !matches!(t.kind, TtKind::Delta { .. })).map(|(_, t)| t.rounding));
        let direction_of = |pick: fn(&TtKind) -> bool| shared(selected.iter().filter(|(_, t)| pick(&t.kind)).map(|(_, t)| t.direction));
        let zones = match (&settings, direction_of(|k| matches!(k, TtKind::Snap { .. }))) {
            (Some(s), Some(Direction::Vertical)) => {
                s.tt_zones_for(font, &glyph).into_iter().filter_map(|i| s.tt_zones.get(i)).map(|z| z.name.clone()).collect()
            }
            _ => Vec::new(),
        };
        // A vertical Stem instruction measures a horizontal stem, and the other way round.
        let stem_names = match (&settings, direction_of(|k| matches!(k, TtKind::Stem { .. }))) {
            (Some(s), Some(d)) => s
                .tt_stems_for(font, &glyph)
                .into_iter()
                .filter_map(|i| s.tt_stems.get(i))
                .filter(|st| st.horizontal == (d == Direction::Vertical))
                .map(|st| st.name.clone())
                .collect(),
            _ => Vec::new(),
        };
        let table = (!deltas.is_empty()).then(|| delta_table(font, &selected));
        Some(TtBox {
            glyph,
            layer: layer.id.clone(),
            view: view.clone(),
            snaps,
            stems,
            rounded,
            deltas,
            zone,
            stem,
            rounding,
            zones,
            stem_names,
            table,
        })
    }

    /// The command `control` runs; `None` when it has nothing to act on.
    pub fn command(&self, control: &Control) -> Option<(&'static str, Value)> {
        let on = |id: &'static str, list: &[usize], key: &str, value: Value| {
            (!list.is_empty()).then(|| {
                let mut p = json!({"glyph": self.glyph, "layer": self.layer, "instructions": list});
                if let Some(m) = p.as_object_mut() {
                    m.insert(key.to_string(), value);
                }
                (id, p)
            })
        };
        match control {
            Control::PixelSize(v) => {
                let ppm = v.round();
                ppm.is_finite().then(|| {
                    let ppm = ppm.clamp(f64::from(MIN_INSTRUCTOR_PPM), f64::from(MAX_INSTRUCTOR_PPM)) as u16;
                    ("hint.tt.setPixelSize", json!({ "ppm": ppm }))
                })
            }
            Control::Smaller => Some(("hint.tt.decreasePixelSize", json!({}))),
            Control::Larger => Some(("hint.tt.increasePixelSize", json!({}))),
            Control::Intent(intent) => {
                INTENTS.iter().find(|(i, ..)| i == intent).map(|(_, p, _)| ("hint.tt.setRenderingIntent", json!({ "intent": p })))
            }
            Control::ToggleDirection => {
                let other = match self.view.direction {
                    Direction::Vertical => Direction::Horizontal,
                    Direction::Horizontal => Direction::Vertical,
                };
                Some(("hint.tt.toggleDirection", json!({ "direction": direction_param(other) })))
            }
            Control::Zone(choice) => on("hint.tt.setSnapZone", &self.snaps, "zone", json!(choice.param())),
            Control::Stem(choice) => on("hint.tt.setStemReference", &self.stems, "stem", json!(choice.param())),
            Control::Rounding(r) => on("hint.tt.setRounding", &self.rounded, "rounding", json!(rounding_param(*r))),
            Control::Delta(click) => {
                let cells = self.table.as_ref()?.cells_param(click)?;
                on("hint.tt.setDeltas", &self.deltas, "cells", cells)
            }
        }
    }
}

/// The Delta table of the selected Delta instructions.
fn delta_table(font: &Font, selected: &[(usize, &TtInstruction)]) -> DeltaTable {
    // Instances or masters at one location share a column.
    fn add(key: String, name: &str, columns: &mut Vec<Column>) {
        if let Some(c) = columns.iter_mut().find(|c| c.key == key) {
            c.label = format!("{}, {name}", c.label);
        } else if columns.len() < tt_deltas::MAX_COLUMNS {
            columns.push(Column { key, label: name.to_string() });
        }
    }
    let mut columns: Vec<Column> = Vec::new();
    let statics: Vec<_> = font.instances.iter().filter(|i| i.kind.as_deref() != Some("variable")).collect();
    if statics.is_empty() {
        for m in font.masters.iter() {
            add(delta_column_key(&m.location), &m.name, &mut columns);
        }
    } else {
        for i in statics {
            add(delta_column_key(&i.location), &i.name, &mut columns);
        }
    }
    let cells_of: Vec<_> = selected
        .iter()
        .filter_map(|(_, t)| match &t.kind {
            TtKind::Delta { cells } => Some(cells),
            _ => None,
        })
        .collect();
    for cells in &cells_of {
        for key in cells.keys() {
            if !columns.iter().any(|c| &c.key == key) && columns.len() < tt_deltas::MAX_COLUMNS {
                columns.push(Column { key: key.clone(), label: key.clone() });
            }
        }
    }
    let mut table = DeltaTable { title: "Deltas".to_string(), columns, ..DeltaTable::default() };
    for (index, column) in table.columns.iter().enumerate() {
        for ppm in tt_deltas::FIRST_PPM..=tt_deltas::LAST_PPM {
            let values = cells_of.iter().map(|c| c.get(&column.key).and_then(|sizes| sizes.get(&ppm)).copied());
            let cell = match shared(values) {
                Some(None) => continue,
                Some(Some(DeltaDir::Up)) => Cell::Up,
                Some(Some(DeltaDir::Down)) => Cell::Down,
                None => Cell::Mixed,
            };
            table.cells.insert((index, ppm), cell);
        }
    }
    table
}

/// The id of the pixel size field.
pub fn ppm_id() -> egui::Id {
    widgets::id("infoBox.tt.ppm")
}

/// The ids of the − and + buttons beside the pixel size field.
pub fn smaller_id() -> egui::Id {
    widgets::id("infoBox.tt.smaller")
}

pub fn larger_id() -> egui::Id {
    widgets::id("infoBox.tt.larger")
}

/// The id of the direction button.
pub fn direction_id() -> egui::Id {
    widgets::id("infoBox.tt.direction")
}

/// The id of an item of the intent pop-up.
pub fn intent_id(param: &str) -> egui::Id {
    widgets::id(("infoBox.tt.intent", param))
}

/// The id of an item of the zone pop-up (`auto`, `none` or the zone's name).
pub fn zone_id(param: &str) -> egui::Id {
    widgets::id(("infoBox.tt.zone", param))
}

/// The id of an item of the stem pop-up (`auto`, `none` or the stem's name).
pub fn stem_id(param: &str) -> egui::Id {
    widgets::id(("infoBox.tt.stem", param))
}

/// The id of a rounding button, by its parameter (`round`, `up`, `down`, `none`, `gdiOnly`).
pub fn rounding_id(param: &str) -> egui::Id {
    widgets::id(("infoBox.tt.rounding", param))
}

/// The id of the Delta button.
pub fn delta_id() -> egui::Id {
    widgets::id("infoBox.tt.delta")
}

/// Where the box remembers that the Delta table is open.
fn table_open_id() -> egui::Id {
    widgets::id("infoBox.tt.deltaTableOpen")
}

/// Draws the box for the Instructor's current view state.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let view = view(app);
    show_view(app, ui, pending, &view);
}

/// Draws the box for `view`.
pub fn show_view(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending, view: &InstructorView) {
    let tokens = app.tokens.clone();
    let Some(b) = TtBox::read(app, view) else { return };
    let mut used = Vec::new();
    frame(ui, &tokens, |ui| contents(ui, &tokens, &b, &mut used));
    let ctx = ui.ctx().clone();
    let open = ctx.data(|d| d.get_temp::<bool>(table_open_id())).unwrap_or(false);
    match &b.table {
        Some(table) if open => match tt_deltas::show(&ctx, &tokens, table) {
            Some(Outcome::Click(click)) => used.push(Control::Delta(click)),
            Some(Outcome::Close) => close_table(&ctx),
            None => {}
        },
        // The selection lost its Deltas: the table closes with them.
        None if open => close_table(&ctx),
        _ => {}
    }
    pending.extend(used.iter().filter_map(|c| b.command(c)));
}

fn close_table(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<bool>(table_open_id());
    });
}

/// The box: centred at the foot of the canvas, as the selection's Info box.
fn frame(ui: &mut egui::Ui, tokens: &Tokens, add: impl FnOnce(&mut egui::Ui)) {
    let canvas = ui.available_rect_before_wrap();
    let pos = egui::pos2(canvas.center().x, canvas.bottom() - tokens.info_box_margin);
    egui::Area::new(widgets::id("infoBox.ttInstructor"))
        .order(egui::Order::Middle)
        .fixed_pos(pos)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .constrain_to(canvas)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, add);
        });
}

fn contents(ui: &mut egui::Ui, tokens: &Tokens, b: &TtBox, used: &mut Vec<Control>) {
    let ppm = b.view.ppm;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("PPM").color(tokens.text_dim));
        if widgets::button(ui, smaller_id(), "−", "Smaller pixel size (,)", ppm > MIN_INSTRUCTOR_PPM, false) {
            used.push(Control::Smaller);
        }
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, ppm_id(), Some(f64::from(ppm)), tokens.field_width, false) {
            used.push(Control::PixelSize(v));
        }
        if widgets::button(ui, larger_id(), "+", "Larger pixel size (.)", ppm < MAX_INSTRUCTOR_PPM, false) {
            used.push(Control::Larger);
        }
        egui::ComboBox::from_id_salt(widgets::id("infoBox.tt.intentMenu")).selected_text(intent_label(b.view.intent)).show_ui(ui, |ui| {
            for (intent, param, label) in INTENTS {
                if widgets::button(ui, intent_id(param), label, "Rendering intent of the grid-fitted outline", true, intent == b.view.intent) {
                    used.push(Control::Intent(intent));
                }
            }
        });
        let (arrow, hover) = match b.view.direction {
            Direction::Vertical => ("↕", "Vertical hinting: points move up and down. Click for horizontal (X)"),
            Direction::Horizontal => ("↔", "Horizontal hinting: points move left and right. Click for vertical (X)"),
        };
        if widgets::button(ui, direction_id(), arrow, hover, true, false) {
            used.push(Control::ToggleDirection);
        }
        if !b.snaps.is_empty() {
            ui.separator();
            if let Some(c) = choice_menu(ui, "infoBox.tt.zoneMenu", b.zone.as_ref(), ["Auto", "No Zone"], &b.zones, zone_id) {
                used.push(Control::Zone(c));
            }
        }
        if !b.stems.is_empty() {
            ui.separator();
            if let Some(c) = choice_menu(ui, "infoBox.tt.stemMenu", b.stem.as_ref(), ["Auto", "No Stem"], &b.stem_names, stem_id) {
                used.push(Control::Stem(c));
            }
        }
        if !b.rounded.is_empty() {
            ui.separator();
            for (mode, param, label, hover) in ROUNDINGS {
                if widgets::button(ui, rounding_id(param), label, hover, true, b.rounding == Some(mode)) {
                    used.push(Control::Rounding(mode));
                }
            }
        }
        if !b.deltas.is_empty() {
            ui.separator();
            let open = ui.ctx().data(|d| d.get_temp::<bool>(table_open_id())).unwrap_or(false);
            if widgets::button(ui, delta_id(), "Delta", "Open the Delta table of the selected Delta instructions", true, open) {
                ui.ctx().data_mut(|d| d.insert_temp(table_open_id(), true));
            }
        }
    });
}

/// A zone or stem pop-up: Auto, the "none" item, then `names`. Shows the shared value, empty
/// when the selection differs.
fn choice_menu(
    ui: &mut egui::Ui,
    salt: &str,
    current: Option<&Choice>,
    [auto, none]: [&str; 2],
    names: &[String],
    item_id: fn(&str) -> egui::Id,
) -> Option<Choice> {
    let shown = match current {
        Some(Choice::Auto) => auto.to_string(),
        Some(Choice::None) => none.to_string(),
        Some(Choice::Named(name)) => name.clone(),
        None => String::new(),
    };
    let mut picked = None;
    egui::ComboBox::from_id_salt(widgets::id(salt)).selected_text(shown).show_ui(ui, |ui| {
        let items = [(Choice::Auto, auto), (Choice::None, none)].into_iter().map(|(c, l)| (c, l.to_string()));
        let named = names.iter().map(|n| (Choice::Named(n.clone()), n.clone()));
        for (choice, label) in items.chain(named) {
            if widgets::button(ui, item_id(choice.param()), &label, &label, true, current == Some(&choice)) {
                picked = Some(choice);
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests;

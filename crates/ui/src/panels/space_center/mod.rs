//! The Space Center (`window.spaceCenter`): text at reading size with a spacing matrix under it.
//!
//! | Input | Does |
//! |---|---|
//! | text, before and after fields | `window.spaceCenter {text}`, `{before}`, `{after}` on every change |
//! | size, line mode | `window.spaceCenter {size}`, `{lineMode}` |
//! | Cut-Off, suffix field | `view.setSpacingCutOff {on}`, `view.setPreferredSuffix {suffix}` |
//! | Display menu, line space, tracking | `view.setSpaceCenterDisplay` with the option changed |
//! | matrix row press, arrows | selects the glyph (`window.spaceCenter {select}`) |
//! | Left Margin, Advance Width and Right Margin fields | `spaceCenter.setValue` for the selected glyph: a number or `=` and an expression |
//!
//! The preview ([`preview`]) paints the projected lines (`app::models::space_center`) and wraps multi-line text at
//! the panel's width.

mod preview;

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked, Trigger};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ListRow;
use craft::ui::menu::Menu;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::table::Table;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::space_center::{MatrixColumn, SpaceCenterModel};
use crate::canvas::tokens::Tokens;
use crate::shell::space_center::text::Row as MatrixRow;
use crate::shell::space_center::{DISPLAY_FLAGS, LineMode};

craft::ui::words! {
    /// The words of the Space Center.
    pub struct SpaceCenterWords {
        text: "fontcraftSpaceCenter.text" = "Text";
        before: "fontcraftSpaceCenter.before" = "Before";
        after: "fontcraftSpaceCenter.after" = "After";
        size: "fontcraftSpaceCenter.size" = "Size";
        line_mode: "fontcraftSpaceCenter.lineMode" = "Lines";
        multi: "fontcraftSpaceCenter.multi" = "Lines";
        single: "fontcraftSpaceCenter.single" = "One Line";
        waterfall: "fontcraftSpaceCenter.waterfall" = "Waterfall";
        suffix: "fontcraftSpaceCenter.suffix" = "Suffix";
        cut_off: "fontcraftSpaceCenter.cutOff" = "Cut-Off";
        display: "fontcraftSpaceCenter.display" = "Display";
        right_to_left: "fontcraftSpaceCenter.rightToLeft" = "Right to Left";
        centered: "fontcraftSpaceCenter.centered" = "Centered";
        upside_down: "fontcraftSpaceCenter.upsideDown" = "Upside Down";
        metrics: "fontcraftSpaceCenter.metrics" = "Metrics";
        beam: "fontcraftSpaceCenter.beam" = "Beam";
        fill: "fontcraftSpaceCenter.fill" = "Fill";
        stroke: "fontcraftSpaceCenter.stroke" = "Stroke";
        inverse: "fontcraftSpaceCenter.inverse" = "Inverse";
        guides: "fontcraftSpaceCenter.guides" = "Guides";
        zones: "fontcraftSpaceCenter.zones" = "Zones";
        line_space: "fontcraftSpaceCenter.lineSpace" = "Line Space";
        tracking: "fontcraftSpaceCenter.tracking" = "Tracking";
        matrix: "fontcraftSpaceCenter.matrix" = "Spacing matrix";
        glyph: "fontcraftSpaceCenter.glyph" = "Glyph";
        width: "fontcraftSpaceCenter.width" = "Width";
        left: "fontcraftSpaceCenter.left" = "Left";
        right: "fontcraftSpaceCenter.right" = "Right";
        /// The selected glyph's fields under the matrix.
        cell_left: "fontcraftSpaceCenter.cellLeft" = "Left Margin";
        cell_width: "fontcraftSpaceCenter.cellWidth" = "Advance Width";
        cell_right: "fontcraftSpaceCenter.cellRight" = "Right Margin";
        empty: "fontcraftSpaceCenter.empty" = "No glyphs in the text";
        no_font: "fontcraftSpaceCenter.noFont" = "No font is open.";
    }
}

const GLYPH: ColumnKey = ColumnKey::new("glyph");
const LEFT: ColumnKey = ColumnKey::new("left");
const WIDTH: ColumnKey = ColumnKey::new("width");
const RIGHT: ColumnKey = ColumnKey::new("right");

pub struct Props<'a> {
    pub model: &'a Tracked<SpaceCenterModel>,
    /// The canvas's colours and sizes: the preview draws glyphs as the canvas does.
    pub tokens: &'a Tokens,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// A matrix value as the matrix writes it: at most two decimals; empty for none.
pub fn format_value(v: Option<f64>) -> String {
    let Some(v) = v else { return String::new() };
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r}") }
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &SpaceCenterModel = props.model;
    let tokens = props.tokens;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<SpaceCenterWords>();
    let _ = Stack::new().fill().show(ui, |ui, _| {
        fields(ui, &words, model, sent);
        options(ui, &words, model, sent);
        if !model.has_font {
            Text::new(words.no_font.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        if let Some(message) = &model.message {
            Text::new(message.clone()).show(ui);
        }
        // The selected glyph's fields come before the two parts that share the room left.
        cells(ui, &words, model, sent);
        preview::show(ui, model, tokens);
        matrix(ui, &words, model, sent);
    });
}

fn fields(ui: &mut Ui, words: &SpaceCenterWords, model: &SpaceCenterModel, sent: &mut Vec<Msg>) {
    let state = &model.state;
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        for (key, label, current) in
            [("text", &words.text, &state.text), ("before", &words.before, &state.before), ("after", &words.after, &state.after)]
        {
            let mut typed = current.clone();
            TextField::new(label.clone(), Bind::bound(&mut typed)).show(ui);
            if typed != *current {
                sent.push(shell("window.spaceCenter", json!({ key: typed })));
            }
        }
    });
}

fn options(ui: &mut Ui, words: &SpaceCenterWords, model: &SpaceCenterModel, sent: &mut Vec<Msg>) {
    let state = &model.state;
    let display = &state.display;
    let modes: Vec<Entry<LineMode>> =
        LineMode::ALL.into_iter().zip([&words.multi, &words.single, &words.waterfall]).map(|(mode, label)| Entry::new(mode, label.clone())).collect();
    let _ = Row::new().fill_width().wrap(true).show(ui, |ui, _| {
        if let Some(Some(size)) = NumberField::new(words.size.clone(), Bind::given(Some(state.size))).show(ui).edit.events.committed() {
            sent.push(shell("window.spaceCenter", json!({ "size": size })));
        }
        if let Some(mode) = Segmented::new(words.line_mode.clone(), Bind::given(state.line_mode), &modes).show(ui).events.committed()
            && *mode != state.line_mode
        {
            sent.push(shell("window.spaceCenter", json!({ "lineMode": mode.id() })));
        }
        let suffix = state.suffix.clone().unwrap_or_default();
        if let Some(typed) = TextField::new(words.suffix.clone(), Bind::given(suffix.clone())).show(ui).events.committed()
            && *typed != suffix
        {
            sent.push(shell("view.setPreferredSuffix", json!({ "suffix": typed })));
        }
        if let Some(on) = Checkbox::new(Bind::given(state.cut_off.on)).label(words.cut_off.clone()).show(ui).events.committed() {
            sent.push(shell("view.setSpacingCutOff", json!({ "on": on })));
        }
        display_menu(ui, words, model, sent);
        if let Some(Some(v)) = NumberField::new(words.line_space.clone(), Bind::given(Some(display.line_space))).show(ui).edit.events.committed() {
            sent.push(shell("view.setSpaceCenterDisplay", json!({ "lineSpace": v })));
        }
        if let Some(Some(v)) = NumberField::new(words.tracking.clone(), Bind::given(Some(display.tracking))).show(ui).edit.events.committed() {
            sent.push(shell("view.setSpaceCenterDisplay", json!({ "tracking": v })));
        }
    });
}

fn flag_label(words: &SpaceCenterWords, key: &str) -> Label {
    match key {
        "rightToLeft" => words.right_to_left.clone(),
        "centered" => words.centered.clone(),
        "upsideDown" => words.upside_down.clone(),
        "metrics" => words.metrics.clone(),
        "beam" => words.beam.clone(),
        "fill" => words.fill.clone(),
        "stroke" => words.stroke.clone(),
        "inverse" => words.inverse.clone(),
        "guides" => words.guides.clone(),
        _ => words.zones.clone(),
    }
}

fn display_menu(ui: &mut Ui, words: &SpaceCenterWords, model: &SpaceCenterModel, sent: &mut Vec<Msg>) {
    let display = &model.state.display;
    let opener = Button::new(words.display.clone()).show(ui);
    let entries: Vec<Entry<&'static str>> =
        DISPLAY_FLAGS.iter().map(|key| Entry::new(*key, flag_label(words, key)).checked(display.flag(key).unwrap_or(false))).collect();
    let menu =
        Menu::new(words.display.clone()).id_salt("fontcraftSpaceCenter.display").opened_by(&opener.response, Trigger::Click).pick(ui, &entries);
    if let Some(key) = menu.picked {
        let on = !display.flag(key).unwrap_or(false);
        sent.push(shell("view.setSpaceCenterDisplay", json!({ key: on })));
    }
}

/// The matrix columns as table rows: a row's key is its column.
struct Columns<'a>(&'a [MatrixColumn]);

impl DisplayOrder<usize> for Columns<'_> {
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
        self.0.get(at).map(|c| Cow::Borrowed(c.glyph.as_str()))
    }
}

impl ItemSource<usize> for Columns<'_> {}

fn matrix(ui: &mut Ui, words: &SpaceCenterWords, model: &SpaceCenterModel, sent: &mut Vec<Msg>) {
    let columns = vec![
        ColumnSpec::new(GLYPH, words.glyph.clone(), ColumnWidth::Content).resizable(),
        ColumnSpec::new(LEFT, words.left.clone(), ColumnWidth::Content).resizable().numeric(),
        ColumnSpec::new(WIDTH, words.width.clone(), ColumnWidth::Content).resizable().numeric(),
        ColumnSpec::new(RIGHT, words.right.clone(), ColumnWidth::Content).resizable().numeric(),
    ];
    let selected = model.state.selected.filter(|s| *s < model.columns.len());
    let before = Selection::new(selected.into_iter().collect(), selected, selected);
    let mut after = before.clone();
    let rows = &model.columns;
    let _ = Table::<usize>::new(words.matrix.clone(), columns)
        .empty(words.empty.clone())
        .striped(true)
        .selection(Bind::bound(&mut after))
        .show_items(ui, &Columns(rows), &mut |at, icx| {
            let Some(c) = rows.get(*at) else { return ListRow::new("") };
            ListRow::new(c.glyph.clone())
                .cells(icx.cells())
                .striped(icx.row.striped)
                .cell(LEFT, format_value(c.left))
                .cell(WIDTH, format_value(c.width))
                .cell(RIGHT, format_value(c.right))
        });
    if after.lead != before.lead {
        let pick = after.lead.or_else(|| after.items.last().copied());
        sent.push(shell("window.spaceCenter", json!({ "select": pick })));
    }
}

/// The selected glyph's values: typing a number or an expression and committing enters it.
fn cells(ui: &mut Ui, words: &SpaceCenterWords, model: &SpaceCenterModel, sent: &mut Vec<Msg>) {
    let selected = model.state.selected.and_then(|s| model.columns.get(s).map(|c| (s, c)));
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        for (row, label) in [(MatrixRow::Left, &words.cell_left), (MatrixRow::Width, &words.cell_width), (MatrixRow::Right, &words.cell_right)] {
            let shown = format_value(selected.and_then(|(_, c)| c.value(row)));
            let out = TextField::new(label.clone(), Bind::given(shown.clone())).enabled(selected.is_some()).show(ui);
            if let (Some(typed), Some((column, _))) = (out.events.committed(), selected)
                && typed.trim() != shown
                && !typed.trim().is_empty()
            {
                sent.push(shell("spaceCenter.setValue", json!({ "column": column, "row": row.id(), "text": typed.trim() })));
            }
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests;

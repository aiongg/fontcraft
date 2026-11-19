//! The Kerning Pairs Preview (`window.kerningPairsPreview`): every stored pair of one master in left-to-right text,
//! drawn kerned with its value underneath, for reviewing a whole font's kerning and correcting outliers in place.
//!
//! | Input | Does |
//! |---|---|
//! | Sort, Size | `window.kerningPairsPreview {sort}`, `{size}` |
//! | click a pair | selects it (`{select}`); a pair with a group side lists every glyph combination it covers below |
//! | click a combination | selects the stored pair that applies to it |
//! | Pair Value, −10, −1, +1, +10 | `kerning.setPair` on the selected pair |
//! | typing a character | moves to the first pair whose name starts with it (the grid's type-ahead) |
//! | double click or Enter on a pair | `kerning.showPairInEditView` |
//! | Show All | drops the glyph filter (`{glyph: null}`) |
//!
//! A group side is drawn by its first member in glyph order. A value that differs from the one the preview found when
//! it opened, or a pair new since, says so in its detail line.

use craft::behavior::{Selection, keyed};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, Entry, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::text::NumberField;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::Ui;
use serde_json::{Value, json};

use super::paint_run;
use crate::app::Msg;
use crate::app::models::kerning_tools::{Combination, PairsPreviewModel, PreviewRow};
use crate::panels::kerning_window::format_value;
use crate::shell::kerning_tools::{PreviewSort, SIZES};

craft::ui::words! {
    /// The words of the Kerning Pairs Preview.
    pub struct PairsPreviewWords {
        sort: "fontcraftPairsPreview.sort" = "Sort";
        first: "fontcraftPairsPreview.first" = "First Glyph";
        second: "fontcraftPairsPreview.second" = "Second Glyph";
        value: "fontcraftPairsPreview.value" = "Size of Value";
        size: "fontcraftPairsPreview.size" = "Size";
        pairs: "fontcraftPairsPreview.pairs" = "Kerning Pairs";
        every_pair: "fontcraftPairsPreview.everyPair" = "every pair";
        pairs_of: "fontcraftPairsPreview.pairsOf" = "pairs of";
        show_all: "fontcraftPairsPreview.showAll" = "Show All";
        empty: "fontcraftPairsPreview.empty" = "No kerning pairs";
        no_font: "fontcraftPairsPreview.noFont" = "No font is open";
        pair_value: "fontcraftPairsPreview.pairValue" = "Pair Value";
        minus_ten: "fontcraftPairsPreview.minusTen" = "−10";
        minus_one: "fontcraftPairsPreview.minusOne" = "−1";
        plus_one: "fontcraftPairsPreview.plusOne" = "+1";
        plus_ten: "fontcraftPairsPreview.plusTen" = "+10";
        was: "fontcraftPairsPreview.was" = "was";
        new_pair: "fontcraftPairsPreview.newPair" = "new";
        combinations: "fontcraftPairsPreview.combinations" = "Combinations of";
        no_pair: "fontcraftPairsPreview.noPair" = "no pair";
        too_many: "fontcraftPairsPreview.tooMany" = "Too many combinations to list:";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<PairsPreviewModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn preview(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("window.kerningPairsPreview", params))
}

fn select(left: &str, right: &str) -> Msg {
    preview(json!({ "select": { "left": left, "right": right } }))
}

/// The name a pair's tile shows: its sides as commands spell them.
pub fn pair_name(left: &str, right: &str) -> String {
    format!("{left} {right}")
}

/// A row's detail line: its value, and what it was when the preview opened.
fn detail(words: &PairsPreviewWords, row: &PreviewRow) -> String {
    match (row.changed, row.was) {
        (true, Some(was)) => format!("{} ({} {})", format_value(row.value), words.was.as_str(), format_value(was)),
        (true, None) => format!("{} ({})", format_value(row.value), words.new_pair.as_str()),
        _ => format_value(row.value),
    }
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &PairsPreviewModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<PairsPreviewWords>();
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        if !model.has_font {
            Text::new(words.no_font.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        options(ui, &words, model, sent);
        pair_grid(ui, &words, model, sent);
        value_row(ui, &words, model, sent);
        combinations(ui, &words, model, sent);
    });
}

fn options(ui: &mut Ui, words: &PairsPreviewWords, model: &PairsPreviewModel, sent: &mut Vec<Msg>) {
    let labels = [&words.first, &words.second, &words.value];
    let sorts: Vec<Entry<PreviewSort>> = PreviewSort::ALL.into_iter().zip(labels).map(|(s, l)| Entry::new(s, l.clone())).collect();
    let sizes: Vec<Entry<u32>> = SIZES.iter().map(|s| Entry::new(*s, s.to_string())).collect();
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if let Some(sort) = Select::new(words.sort.clone(), Bind::given(model.sort), &sorts).show(ui).events.committed() {
            sent.push(preview(json!({ "sort": sort.name() })));
        }
        if let Some(size) = Select::new(words.size.clone(), Bind::given(model.size), &sizes).show(ui).events.committed() {
            sent.push(preview(json!({ "size": size })));
        }
        let scope = model.glyph.as_deref().map_or_else(|| words.every_pair.as_str().to_string(), |g| format!("{} {g}", words.pairs_of.as_str()));
        Text::new(format!("{} ({scope})", model.rows.len())).shade(Shade::Muted).show(ui);
        if model.glyph.is_some() && Button::new(words.show_all.clone()).show(ui).clicked() {
            sent.push(preview(json!({ "glyph": null })));
        }
    });
}

/// The zoom that makes a tile `size` pixels high.
fn zoom_for(ui: &Ui, size: u32) -> f32 {
    let card = Env::of(ui).tokens.size.thumb.card;
    if card > 0.0 { (size as f32 / card).max(f32::EPSILON) } else { 1.0 }
}

fn pair_grid(ui: &mut Ui, words: &PairsPreviewWords, model: &PairsPreviewModel, sent: &mut Vec<Msg>) {
    let rows = &model.rows;
    let positions: Vec<usize> = (0..rows.len()).collect();
    let source =
        keyed(&positions, |at| *at).with_text(|at| std::borrow::Cow::Owned(rows.get(*at).map_or_else(String::new, |r| pair_name(&r.left, &r.right))));
    let before = model.selected.map_or_else(Selection::default, Selection::single);
    let mut after = before.clone();
    let upm = model.units_per_em;
    let mut grid = TileGrid::<usize>::new(words.pairs.clone())
        .empty(words.empty.clone())
        .zoom(zoom_for(ui, model.size))
        .caption(CaptionPlace::Below)
        .type_ahead(true)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut after));
    if let Some(at) = model.reveal {
        grid = grid.reveal(at);
    }
    let out = grid.show_items(ui, &source, &mut |at, _| {
        let Some(row) = rows.get(*at) else { return Tile::new(String::new()) };
        let (first, second) = (model.art.get(&row.first), model.art.get(&row.second));
        let value = row.value as f32;
        Tile::new(pair_name(&row.left, &row.right)).accessible_name(pair_name(&row.left, &row.right)).detail(detail(words, row)).thumb(
            ThumbAspect::new(2, 1),
            move |cx, rect| {
                let mut run = Vec::new();
                let mut x = 0.0;
                if let Some(a) = first {
                    run.push((a.as_ref(), 0.0));
                    x = a.width + value;
                }
                if let Some(b) = second {
                    run.push((b.as_ref(), x));
                }
                paint_run(cx, rect, upm, &run);
            },
        )
    });
    if after.items != before.items
        && let Some(row) = after.items.last().and_then(|at| rows.get(*at))
    {
        sent.push(select(&row.left, &row.right));
    }
    for event in &out.response.events {
        if let CollectionEvent::Activate(at) = event
            && let Some(row) = rows.get(*at)
        {
            sent.push(Msg::run("kerning.showPairInEditView", json!({ "left": row.left, "right": row.right })));
        }
    }
}

/// `kerning.setPair` on the selected pair of the preview's master.
fn set_pair(model: &PairsPreviewModel, left: &str, right: &str, value: f64) -> Msg {
    let mut p = json!({ "left": left, "right": right, "value": value });
    if let (Some(m), Some(map)) = (&model.master, p.as_object_mut()) {
        map.insert("master".into(), json!(m));
    }
    Msg::run("kerning.setPair", p)
}

fn value_row(ui: &mut Ui, words: &PairsPreviewWords, model: &PairsPreviewModel, sent: &mut Vec<Msg>) {
    let Some((left, right)) = &model.selected_pair else { return };
    let current = model.selected.and_then(|at| model.rows.get(at)).map_or(0.0, |r| r.value);
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        Text::new(pair_name(left, right)).show(ui);
        let field = NumberField::new(words.pair_value.clone(), Bind::given(Some(current))).show(ui);
        if let Some(Some(v)) = field.edit.events.committed() {
            sent.push(set_pair(model, left, right, *v));
        }
        for (label, delta) in [(&words.minus_ten, -10.0), (&words.minus_one, -1.0), (&words.plus_one, 1.0), (&words.plus_ten, 10.0)] {
            if Button::new(label.clone()).show(ui).clicked() {
                sent.push(set_pair(model, left, right, current + delta));
            }
        }
    });
}

/// A combination's detail line: the value and the lookup step that found it.
fn combination_detail(words: &PairsPreviewWords, c: &Combination) -> String {
    match (&c.pair, &c.step) {
        (Some(_), Some(step)) => format!("{} ({step})", format_value(c.value)),
        (Some(_), None) => format_value(c.value),
        (None, _) => words.no_pair.as_str().to_string(),
    }
}

fn combinations(ui: &mut Ui, words: &PairsPreviewWords, model: &PairsPreviewModel, sent: &mut Vec<Msg>) {
    let Some((left, right)) = model.selected_pair.as_ref().filter(|(l, r)| l.starts_with('@') || r.starts_with('@')) else { return };
    let title = format!("{} {}", words.combinations.as_str(), pair_name(left, right));
    Text::new(title.clone()).show(ui);
    if let Some(note) = &model.combinations_note {
        Text::new(format!("{} {note}", words.too_many.as_str())).shade(Shade::Muted).show(ui);
        return;
    }
    let combos = &model.combinations;
    let positions: Vec<usize> = (0..combos.len()).collect();
    let source = keyed(&positions, |at| *at)
        .with_text(|at| std::borrow::Cow::Owned(combos.get(*at).map_or_else(String::new, |c| pair_name(&c.first, &c.second))));
    let mut picked = Selection::default();
    let upm = model.units_per_em;
    let _ = TileGrid::<usize>::new(title)
        .zoom(zoom_for(ui, model.size) * 0.75)
        .caption(CaptionPlace::Below)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut picked))
        .show_items(ui, &source, &mut |at, _| {
            let Some(c) = combos.get(*at) else { return Tile::new(String::new()) };
            let (first, second) = (model.art.get(&c.first), model.art.get(&c.second));
            let value = c.value as f32;
            Tile::new(pair_name(&c.first, &c.second)).accessible_name(pair_name(&c.first, &c.second)).detail(combination_detail(words, c)).thumb(
                ThumbAspect::new(2, 1),
                move |cx, rect| {
                    let mut run = Vec::new();
                    let mut x = 0.0;
                    if let Some(a) = first {
                        run.push((a.as_ref(), 0.0));
                        x = a.width + value;
                    }
                    if let Some(b) = second {
                        run.push((b.as_ref(), x));
                    }
                    paint_run(cx, rect, upm, &run);
                },
            )
        });
    if let Some((l, r)) = picked.items.last().and_then(|at| combos.get(*at)).and_then(|c| c.pair.as_ref()) {
        sent.push(select(l, r));
    }
}

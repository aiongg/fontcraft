//! Pairs & Phrases: a work list for kerning and spacing. The commands and the lists live in `shell::kerning_review`.
//!
//! | Input | Does |
//! |---|---|
//! | Pairs, Phrases, Auto | `kerning.setPairListMode` |
//! | path field (Enter); Reset | `kerning.loadPairList {path}`, `{reset}` |
//! | a click on a row, Up, Down | `kerning.stepPairList {index}`: shows it in the edit view |
//! | Cmd-click, Shift-click | add a row to the selection, or the rows up to it (`{select}`) |
//! | Auto Kern | `kerning.autoKernPairList` on the selected rows, for review; Apply, Discard |
//!
//! Pair rows show the value stored for the pair in the current master.

use std::borrow::Cow;

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::kerning_review::{PairRow, PairsModel};
use crate::shell::kerning_review::ListMode;

craft::ui::words! {
    /// The words of the Pairs & Phrases panel.
    pub struct PairsWords {
        mode: "fontcraftPairs.mode" = "List";
        pairs: "fontcraftPairs.pairs" = "Pairs";
        phrases: "fontcraftPairs.phrases" = "Phrases";
        auto: "fontcraftPairs.auto" = "Auto";
        /// The field a list file's path is typed into.
        list_file: "fontcraftPairs.listFile" = "List file";
        reset: "fontcraftPairs.reset" = "Reset";
        reset_tip: "fontcraftPairs.resetTip" = "Go back to the built-in list";
        /// Between the row count and where the rows came from.
        rows_from: "fontcraftPairs.rowsFrom" = "rows from";
        builtin: "fontcraftPairs.builtin" = "built-in list";
        rows: "fontcraftPairs.rows" = "Rows";
        no_font: "fontcraftPairs.noFont" = "No font";
        auto_kern: "fontcraftPairs.autoKern" = "Auto Kern";
        auto_kern_tip: "fontcraftPairs.autoKernTip" = "Propose kerning for the selected pairs, for review";
        proposed: "fontcraftPairs.proposed" = "Proposed kerning";
        /// Before a pair's value before the proposal.
        was: "fontcraftPairs.was" = "was";
        apply: "fontcraftPairs.apply" = "Apply";
        discard: "fontcraftPairs.discard" = "Discard";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<PairsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// The rows as the list's item source: the row number is the key.
struct Rows<'a>(&'a [PairRow]);

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|r| Cow::Borrowed(r.label.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<PairsWords>();
    let model: &PairsModel = props.model;
    if !model.has_font {
        Text::new(words.no_font.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let sent = &mut *props.sent;
    let list = if model.pairs { "pairs" } else { "phrases" };
    let modes = [
        Entry::new(ListMode::Pairs, words.pairs.clone()),
        Entry::new(ListMode::Phrases, words.phrases.clone()),
        Entry::new(ListMode::Auto, words.auto.clone()),
    ];
    let mut mode = model.mode;
    Segmented::new(words.mode.clone(), Bind::bound(&mut mode), &modes).show(ui);
    if mode != model.mode {
        sent.push(shell("kerning.setPairListMode", json!({ "mode": mode.name() })));
    }
    let _ = Row::new().show(ui, |ui, _| {
        let out = TextField::new(words.list_file.clone(), Bind::given(String::new())).placeholder(words.list_file.clone()).show(ui);
        if out.submitted
            && let Some(path) = out.events.committed().map(|p| p.trim().to_string())
            && !path.is_empty()
        {
            sent.push(shell("kerning.loadPairList", json!({ "path": path, "list": list })));
        }
        if Button::new(words.reset.clone()).tooltip(words.reset_tip.clone()).show(ui).clicked() {
            sent.push(shell("kerning.loadPairList", json!({ "reset": true, "list": list })));
        }
    });
    let source = model.source.clone().unwrap_or_else(|| words.builtin.as_str().to_string());
    Text::new(format!("{} {} {source}", model.rows.len(), words.rows_from.as_str())).shade(Shade::Muted).show(ui);
    let lead = model.current.filter(|c| model.selected.contains(c)).or_else(|| model.selected.last().copied());
    let mut selection = Selection::new(model.selected.clone(), lead, lead);
    let out = List::<usize>::new(words.rows.clone()).current(model.current).selection(Bind::bound(&mut selection)).show_items(
        ui,
        &Rows(&model.rows),
        &mut |at, _| {
            let row = model.rows.get(*at);
            let item = ListRow::new(row.map(|r| r.label.clone()).unwrap_or_default());
            match row.and_then(|r| r.value) {
                Some(value) => item.trailing(format!("{value}")),
                None => item,
            }
        },
    );
    let mut stepped = None;
    for event in &out.events {
        if let CollectionEvent::Activate(at) = event {
            stepped = Some(*at);
        }
    }
    if stepped.is_none() && selection.items != model.selected {
        match selection.items.as_slice() {
            [only] => stepped = Some(*only),
            items => sent.push(shell("kerning.stepPairList", json!({ "select": items, "list": list }))),
        }
    }
    if let Some(at) = stepped {
        sent.push(shell("kerning.stepPairList", json!({ "index": at, "list": list })));
    }
    if Button::new(words.auto_kern.clone()).tooltip(words.auto_kern_tip.clone()).enabled(model.pairs).show(ui).clicked() && model.pairs {
        sent.push(shell("kerning.autoKernPairList", json!({ "list": list })));
    }
    if let Some(lines) = &model.proposal {
        Text::new(words.proposed.clone()).show(ui);
        for l in lines {
            Text::new(format!("{} {}  {}  ({} {})  {}", l.left, l.right, l.value, words.was.as_str(), l.previous, l.result)).show(ui);
        }
        let _ = Row::new().show(ui, |ui, _| {
            if Button::new(words.apply.clone()).show(ui).clicked() {
                sent.push(shell("kerning.autoKernPairList", json!({ "apply": true })));
            }
            if Button::new(words.discard.clone()).show(ui).clicked() {
                sent.push(shell("kerning.autoKernPairList", json!({ "discard": true })));
            }
        });
    }
}

#[cfg(test)]
mod tests;

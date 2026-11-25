//! Layers: the layers of the panel's glyphs (the glyphs selected in Font View, those of the text selection, else the shown
//! glyph; with several, the layers all of them have) in palette order (`layers.list`), master layers in bold, the others indented
//! under their master with their kind, the active layer marked as current. The eye shows or hides a layer
//! (`layers.toggleVisibility`). A click makes a row the active layer (`layers.select`), Shift+click selects a range
//! of rows and Cmd+click adds or removes one. Double-clicking a non-master name renames it (`layers.rename`). New
//! Backup Layer adds a backup of the active layer (`layers.addBackupLayer`), Delete Layer deletes the selected rows
//! (`layers.delete`), Use as Master restores a selected backup (`layers.useAsMaster`). The filter menu runs
//! `layers.onlyShowLayersFromCurrentMaster` and `layers.hideBackupLayers`. A right-click on a row opens its context
//! menu: the commands whose context names `layers.row`, acting on that row, or on every selected row when it is one of
//! them. With several glyphs each command changes every glyph, as one undo step.
//!
//! A non-master row can be dragged. Dropped among the rows of its own master it moves there (`layers.reorder`); a
//! backup dropped on its master's row takes the master's place (`layers.useAsMaster`); any other row dropped on its
//! master row moves to the top of that master. A drop under another master is refused: no drop mark shows and
//! nothing runs.
//!
//! Special layers show their label in place of the name: `[…]` for an alternate layer, `{…}` for an intermediate
//! one, both for a layer that is both. A master layer with a range shows its `[…]` after the master's name.
//! Double-clicking a special row opens its editor (`layers.editSpecialLayerSettings`). In the context menu,
//! Intermediate and Alternate are ticked while the layer has that status: choosing one opens its editor, unticking it
//! removes the status (`layers.clearIntermediate`, `layers.clearAlternate`), so those two have no item of their own.
//!
//! Colour layers show their kind's label after the name: a Color Palette layer also its colour, a click on which opens
//! the chooser (`layer.setColorPaletteIndex`); an iColor layer its size in a field (`layer.setIColorSize`).

use std::borrow::Cow;
use std::cell::{Cell, RefCell};

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, GesturePhase, ItemSource, Selection, SelectionChange};
use craft::theme::Srgba;
use craft::ui::buttons::Button;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, ListDrag, Props as _, ReorderMove, ReorderRule, Show as _, ShowCollection as _, ShowContainer as _,
    ToggleColumn, Tracked, Trigger, request_rename,
};
use craft::ui::layout_components::{Row as Line, Stack};
use craft::ui::list::{List, ListRow, RowInk, ToggleCell, TrailingFace, TrailingPart};
use craft::ui::menu::Menu;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::NumberField;
use egui::Ui;
use fontcraft_engine::color::layers::{self as color_layers, ColorLayerKind};
use fontcraft_engine::model::PaletteEntry;
use serde_json::{Value, json};

use crate::app::models::layers::{ColorEntry, LayerRow, LayersModel, MAX_CHOOSER_ROWS, Part};
use crate::app::{Msg, ViewMsg};

craft::ui::words! {
    /// The words of the Layers panel.
    pub struct LayersWords {
        /// The list's name.
        list: "fontcraftLayers.list" = "Layers";
        /// Shown without a glyph.
        no_glyph: "fontcraftLayers.noGlyph" = "No glyph selected";
        /// A layer without a name.
        untitled: "fontcraftLayers.untitled" = "Untitled";
        new_backup: "fontcraftLayers.newBackup" = "New Backup Layer";
        delete: "fontcraftLayers.delete" = "Delete Layer";
        use_as_master: "fontcraftLayers.useAsMaster" = "Use as Master";
        /// The filter menu's button.
        filter: "fontcraftLayers.filter" = "Filter";
        current_master_only: "fontcraftLayers.currentMasterOnly" = "Only Show Layers from Current Master";
        hide_backups: "fontcraftLayers.hideBackups" = "Hide Backup Layers";
        /// The row menu's name.
        row_menu: "fontcraftLayers.rowMenu" = "Layer";
        /// The swatch of a Color Palette layer.
        choose_color: "fontcraftLayers.chooseColor" = "Choose the palette colour";
        /// The chooser's text colour entry.
        text_color: "fontcraftLayers.textColor" = "Text colour";
        /// An iColor layer's size field.
        icolor_size: "fontcraftLayers.iColorSize" = "Size in pixels per em";
        /// Why Use as Master cannot act on the selected rows.
        one_backup: "fontcraftLayers.oneBackup" = "Select one backup layer of each master";
        /// Why a special layer's editor cannot open.
        one_glyph: "fontcraftLayers.oneGlyph" = "Select one glyph and one layer";
        /// Why a master layer cannot be deleted or renamed.
        master_stays: "fontcraftLayers.masterStays" = "A master layer goes with its master";
        /// Why only a backup can become the master layer.
        only_backups: "fontcraftLayers.onlyBackups" = "Only a backup layer can become the master layer";
        /// Why a master layer cannot be intermediate.
        master_not_intermediate: "fontcraftLayers.masterNotIntermediate" = "A master layer cannot be an intermediate layer";
    }
}

/// The eye column.
const EYE: ColumnKey = ColumnKey::new("eye");
/// The name column, where rename happens.
const NAME: ColumnKey = ColumnKey::new("name");
/// A Color Palette layer's swatch.
const SWATCH: TrailingPart = TrailingPart::new(1);
/// The iColor sizes the size field takes, in pixels per em.
const SIZES: std::ops::RangeInclusive<f64> = 1.0..=4096.0;

/// The row menu's ticked items: the command that sets a status, the one that removes it, and the part it edits.
const STATUSES: [(&str, &str, Part); 2] =
    [("layers.setIntermediate", "layers.clearIntermediate", Part::Intermediate), ("layers.setAlternate", "layers.clearAlternate", Part::Alternate)];

pub struct Props<'a> {
    pub model: &'a Tracked<LayersModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

/// The shown rows as the list's display order, keyed by layer id: master rows at depth 0, the others under them.
struct Rows<'a> {
    rows: Vec<&'a LayerRow>,
    untitled: &'a str,
}

impl Rows<'_> {
    /// What the row shows: a special layer's label, else its name.
    fn label_of(&self, row: &LayerRow) -> String {
        if !row.is_master() && (row.bracket.is_some() || row.brace.is_some()) {
            return [row.bracket.as_deref(), row.brace.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        }
        if row.name.is_empty() { self.untitled.to_string() } else { row.name.clone() }
    }
}

impl DisplayOrder<String> for Rows<'_> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> String {
        self.rows.get(at).map(|r| r.id.clone()).unwrap_or_default()
    }

    fn index_of(&self, key: &String) -> Option<usize> {
        self.rows.iter().position(|r| r.id == *key)
    }

    fn depth(&self, at: usize) -> usize {
        self.rows.get(at).map_or(0, |r| usize::from(!r.is_master()))
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.rows.get(at).map(|r| Cow::Owned(self.label_of(r)))
    }
}

impl ItemSource<String> for Rows<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == EYE && self.rows.get(at).is_some_and(|r| r.visible)
    }
}

/// The value of an event that ended: a commit or a set.
fn done<T>(event: &EditEvent<T>) -> Option<&T> {
    match event.phase() {
        Some(GesturePhase::Commit) | None => event.value(),
        _ => None,
    }
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<LayersWords>();
    let model: &LayersModel = props.model;
    if model.glyphs.is_empty() {
        ListRow::new(words.no_glyph.clone()).show(ui);
        return;
    }
    let sent = &mut *props.sent;
    let rows = Rows { rows: model.shown.iter().filter_map(|id| model.row(id)).collect(), untitled: words.untitled.as_str() };
    let selection = Selection::new(model.selection.clone(), model.active_layer.clone(), model.active_layer.clone());
    let columns = vec![
        ColumnSpec::toggle(EYE, Label::Empty, ToggleColumn::sweepable()).icon(craft::behavior::IconId::new("eye")),
        ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0)),
    ];
    // What the last drop test decided: the release that lands a drag runs the test once more first.
    let decided: Cell<Option<DropAction>> = Cell::new(None);
    let mut swatches = Vec::new();
    let sizes: RefCell<Vec<(String, f64)>> = RefCell::new(Vec::new());
    Stack::new().fill().show(ui, |ui, _| {
        let out = List::<String>::new(words.list.clone())
            .header(false)
            .natural_height(true)
            .tree(true)
            .rename(true)
            .rename_column(NAME)
            .columns(columns.clone())
            .reorder(ReorderRule::new())
            .current(model.active_layer.clone())
            .selection(Bind::given(selection.clone()))
            .can_rename(|id: &String| model.row(id).is_some_and(|r| !r.is_master() && r.bracket.is_none() && r.brace.is_none()))
            .can_drag(|id: &String| model.row(id).is_some_and(|r| !r.is_master()))
            .accepts(|drag: &ListDrag<String>, over: Option<&String>, insert: usize| {
                let action = drop_on(model, &rows.rows, drag, over, insert);
                decided.set(Some(action));
                matches!(action, DropAction::Reorder(_) | DropAction::UseAsMaster)
            })
            .show_items(ui, &rows, &mut |id, icx| {
                let Some(row) = model.row(id) else { return ListRow::new(Label::Empty) };
                let mut item = ListRow::new(rows.label_of(row))
                    .strong(row.is_master())
                    .cells(icx.cells())
                    .state(icx.row.clone())
                    .toggle(ToggleCell::eye(EYE, row.visible));
                if !row.visible {
                    item = item.ink(RowInk::Dimmed);
                }
                if row.is_master() {
                    if let Some(bracket) = &row.bracket {
                        item = item.trailing_strong(bracket.clone(), bracket.clone());
                    }
                } else if row.bracket.is_none() && row.brace.is_none() {
                    match &row.color {
                        Some(ColorEntry::Palette { label, current, palette }) => {
                            let color = current.and_then(|i| palette.get(usize::from(i))).map(|c| rgba(*c));
                            let face = match color {
                                Some(color) => IconSpec::swatch(color),
                                None => IconSpec::initial("*"),
                            };
                            item = item.trailing_strong(label.clone(), label.clone()).trailing_part(
                                SWATCH,
                                TrailingFace::Icon(face),
                                words.choose_color.clone(),
                            );
                            swatches.push((row.id.clone(), icx.part_opener(SWATCH.part())));
                        }
                        Some(ColorEntry::IColor { label, size }) => {
                            let field = NumberField::new(words.icolor_size.clone(), Bind::given(Some(f64::from(*size))))
                                .id_salt(("fontcraftLayers.size", row.id.as_str()));
                            let (id, sizes) = (row.id.clone(), &sizes);
                            item = item.trailing_strong(label.clone(), label.clone()).trailing_field(field, move |out| {
                                if let Some(Some(value)) = out.edit.events.committed() {
                                    sizes.borrow_mut().push((id, *value));
                                }
                            });
                        }
                        Some(ColorEntry::Full { label } | ColorEntry::Svg { label }) => item = item.trailing_strong(label.clone(), label.clone()),
                        None if !row.is_backup() => item = item.trailing_strong(row.kind.clone(), row.kind.clone()),
                        None => {}
                    }
                }
                item
            });
        let mut menu_row = model.menu_row.clone();
        for event in &out.events {
            match event {
                CollectionEvent::Selection(change) => sent.extend(select(change)),
                CollectionEvent::Toggle(edit) => {
                    if let Some(cell) = done(edit)
                        && cell.column == EYE
                        && let Some(row) = model.row(&cell.key).filter(|r| r.visible != cell.on)
                    {
                        sent.extend(on_layers(model, &[row], "layers.toggleVisibility", &json!({ "visible": cell.on })));
                    }
                }
                CollectionEvent::Rename(edit) => {
                    if let Some(rename) = done(edit) {
                        let name = rename.name.trim();
                        if let Some(row) = model.row(&rename.key).filter(|r| !name.is_empty() && r.name != name) {
                            sent.extend(on_layers(model, &[row], "layers.rename", &json!({ "name": name })));
                        }
                    }
                }
                CollectionEvent::Activate(id) => {
                    if let ([glyph], Some(part)) = (model.glyphs.as_slice(), model.row(id).and_then(special_part)) {
                        sent.push(edit_special(glyph, id, part));
                    }
                }
                CollectionEvent::Reorder(edit) => {
                    if let Some(moved) = done(edit) {
                        let action = decided.get().unwrap_or_else(|| key_move(model, &rows.rows, moved));
                        sent.extend(drop_command(model, &moved.key, action));
                    }
                }
                CollectionEvent::ContextMenu(request) => menu_row = Some(request.key.clone()),
                _ => {}
            }
        }
        for (id, value) in sizes.take() {
            if let Some(row) = model.row(&id).filter(|_| SIZES.contains(&value) && value.fract() == 0.0) {
                sent.extend(on_each(model, &[row], "layer.setIColorSize", &json!({ "size": value as u16 })));
            }
        }
        for (id, opener) in swatches.drain(..) {
            let Some(row) = model.row(&id) else { continue };
            let Some(button) = out.opener(opener) else { continue };
            if let Some(index) = chooser(ui, &words, row, button) {
                sent.extend(on_each(model, &[row], "layer.setColorPaletteIndex", &json!({ "index": index })));
            }
        }
        if let Some(row) = menu_row.as_deref().and_then(|id| model.row(id)) {
            let entries = row_entries(model, row, &words);
            let menu = Menu::context(words.row_menu.clone(), &out.response).pick(ui, &entries);
            let open = menu.closed.is_none() && menu.picked.is_none();
            if let Some(command) = menu.picked {
                if command == "layers.rename" {
                    request_rename(ui.ctx(), out.response.id, &row.id);
                } else {
                    sent.extend(row_command(model, row, &command));
                }
            }
            let wanted = open.then(|| row.id.clone());
            if wanted != model.menu_row {
                sent.push(Msg::View(ViewMsg::LayerMenu(wanted)));
            }
        } else if model.menu_row.is_some() {
            sent.push(Msg::View(ViewMsg::LayerMenu(None)));
        }
        buttons(ui, model, &words, sent);
    });
}

/// `command` with `fields`, as one undo step: one run, or a batch with a run for each set of `targets`.
fn run(command: &str, fields: &Value, targets: Vec<Value>) -> Option<Msg> {
    let mut steps: Vec<Value> = Vec::with_capacity(targets.len());
    for mut params in targets {
        if let (Value::Object(params), Value::Object(fields)) = (&mut params, fields) {
            params.extend(fields.clone());
        }
        steps.push(params);
    }
    if steps.len() > 1 {
        let label = fontcraft_engine::find_command(command).map_or(command, |spec| spec.label);
        let steps: Vec<Value> = steps.into_iter().map(|params| json!({ "command": command, "params": params })).collect();
        return Some(Msg::run("command.batch", json!({ "label": label, "steps": steps })));
    }
    steps.pop().map(|params| Msg::run(command, params))
}

/// `command`, which takes `glyphs` and `layer`, on the layers the rows stand for in every glyph of the panel.
fn on_layers(model: &LayersModel, rows: &[&LayerRow], command: &str, fields: &Value) -> Option<Msg> {
    let targets = rows.iter().flat_map(|row| row.layers(&model.glyphs)).map(|(layer, glyphs)| json!({ "glyphs": glyphs, "layer": layer }));
    run(command, fields, targets.collect())
}

/// `command`, which takes `glyph` and `layer`, on the layers the rows stand for in every glyph of the panel.
fn on_each(model: &LayersModel, rows: &[&LayerRow], command: &str, fields: &Value) -> Option<Msg> {
    let layers = rows.iter().flat_map(|row| row.layers(&model.glyphs));
    let targets = layers.flat_map(|(layer, glyphs)| glyphs.into_iter().map(move |glyph| json!({ "glyph": glyph, "layer": layer })));
    run(command, fields, targets.collect())
}

/// `layers.delete` on the layers the rows stand for in every glyph of the panel.
fn delete(model: &LayersModel, rows: &[&LayerRow]) -> Option<Msg> {
    let same = rows.iter().all(|row| row.others.is_empty());
    let targets = match rows {
        [] => Vec::new(),
        rows if same => vec![json!({ "glyphs": model.glyphs, "layers": rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>() })],
        rows => rows.iter().flat_map(|row| row.layers(&model.glyphs)).map(|(layer, glyphs)| json!({ "glyphs": glyphs, "layers": [layer] })).collect(),
    };
    run("layers.delete", &Value::Null, targets)
}

/// `fields` with a new backup's name: the person's local date and time, as the engine spells its default in UTC.
fn with_backup_name(mut fields: Value) -> Value {
    if let (Some(name), Some(fields)) = (crate::app::clock::local_minute(), fields.as_object_mut()) {
        fields.insert("name".into(), json!(name));
    }
    fields
}

/// `layers.useAsMaster` on backups ([`usable_as_masters`]) in every glyph of the panel.
fn use_as_master(model: &LayersModel, rows: &[&LayerRow]) -> Option<Msg> {
    on_layers(model, rows, "layers.useAsMaster", &with_backup_name(json!({})))
}

/// Whether Use as Master can act on `rows`: backups, at most one of each master.
fn usable_as_masters(rows: &[&LayerRow]) -> bool {
    let mut masters: Vec<&str> = rows.iter().filter(|r| r.is_backup()).map(|r| r.master.as_str()).collect();
    masters.sort_unstable();
    masters.dedup();
    !rows.is_empty() && masters.len() == rows.len()
}

/// The buttons under the list and the filter menu.
fn buttons(ui: &mut Ui, model: &LayersModel, words: &LayersWords, sent: &mut Vec<Msg>) {
    let selected: Vec<&LayerRow> = model.all.iter().filter(|r| model.selection.contains(&r.id)).collect();
    let deletable: Vec<&LayerRow> = selected.iter().copied().filter(|r| !r.is_master()).collect();
    // The palette is narrower than the four buttons: they wrap rather than cut their captions short.
    Line::new().wrap(true).show(ui, |ui, _| {
        if Button::new(words.new_backup.clone()).show(ui).clicked()
            && let Some(master) = &model.active_master
        {
            let fields = with_backup_name(json!({ "master": master }));
            let from = model.active_layer.as_deref().and_then(|id| model.row(id));
            let targets: Vec<Value> = match from {
                Some(row) => row.layers(&model.glyphs).into_iter().map(|(from, glyphs)| json!({ "glyphs": glyphs, "from": from })).collect(),
                None => vec![json!({ "glyphs": model.glyphs })],
            };
            sent.extend(run("layers.addBackupLayer", &fields, targets));
        }
        if Button::new(words.delete.clone()).enabled(!deletable.is_empty()).show(ui).clicked() {
            sent.extend(delete(model, &deletable));
        }
        let usable = usable_as_masters(&selected);
        if Button::new(words.use_as_master.clone()).enabled(usable).show(ui).clicked() && usable {
            sent.extend(use_as_master(model, &selected));
        }
        let filter = Button::new(words.filter.clone()).show(ui);
        let entries = [
            Entry::new("layers.onlyShowLayersFromCurrentMaster", words.current_master_only.clone()).checked(model.current_master_only),
            Entry::new("layers.hideBackupLayers", words.hide_backups.clone()).checked(model.hide_backups),
        ];
        let menu = Menu::new(words.filter.clone()).opened_by(&filter.response, Trigger::Click).pick(ui, &entries);
        if let Some(id) = menu.picked {
            let on = match id {
                "layers.onlyShowLayersFromCurrentMaster" => !model.current_master_only,
                _ => !model.hide_backups,
            };
            sent.push(Msg::Shell(CommandRef::with_params(id, json!({ "on": on }))));
        }
    });
}

/// The palette colour chooser of a Color Palette layer, hung from its swatch: the index picked, or `"*"` for the
/// text colour.
fn chooser(ui: &mut Ui, words: &LayersWords, row: &LayerRow, button: &egui::Response) -> Option<Value> {
    let Some(ColorEntry::Palette { current, palette, .. }) = &row.color else { return None };
    let mut entries: Vec<Entry<Option<u16>>> = palette
        .iter()
        .take(MAX_CHOOSER_ROWS)
        .zip(0u16..)
        .map(|(c, i)| {
            let label = color_layers::label(&ColorLayerKind::Palette(PaletteEntry::Index(i)));
            Entry::new(Some(i), label).swatch(rgba(*c)).checked(*current == Some(i))
        })
        .collect();
    entries.push(Entry::new(None, words.text_color.clone()).checked(current.is_none()));
    let menu = Menu::new(words.choose_color.clone())
        .id_salt(("fontcraftLayers.chooser", row.id.as_str()))
        .opened_by(button, Trigger::Click)
        .pick(ui, &entries);
    menu.picked.map(|index| index.map_or_else(|| json!("*"), |i| json!(i)))
}

fn rgba([r, g, b, a]: [u8; 4]) -> Srgba {
    Srgba::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0)
}

/// The `layers.select` a selection change asks for: a click picks the active layer, Shift+click a range from it,
/// Cmd+click adds or removes one row.
fn select(change: &SelectionChange<String>) -> Option<Msg> {
    let params = match change {
        SelectionChange::Set(keys) => match keys.as_slice() {
            [one] => json!({ "layer": one }),
            [.., last] => json!({ "layer": last, "extend": "range" }),
            [] => return None,
        },
        SelectionChange::Toggle(key) => json!({ "layer": key, "extend": "toggle" }),
        _ => return None,
    };
    Some(Msg::Shell(CommandRef::with_params("layers.select", params)))
}

/// The part a double-click on `row` edits: a special layer's own label, or a master layer's range.
fn special_part(row: &LayerRow) -> Option<Part> {
    if row.bracket.is_some() {
        Some(Part::Alternate)
    } else if row.brace.is_some() && !row.is_master() {
        Some(Part::Intermediate)
    } else {
        None
    }
}

fn edit_special(glyph: &str, layer: &str, part: Part) -> Msg {
    Msg::Shell(CommandRef::with_params("layers.editSpecialLayerSettings", json!({ "glyph": glyph, "layer": layer, "part": part.name() })))
}

/// The rows the row menu of `row` acts on: every selected row when it is one of them, else it alone.
fn menu_rows<'m>(model: &'m LayersModel, row: &'m LayerRow) -> Vec<&'m LayerRow> {
    if !model.selection.contains(&row.id) {
        return vec![row];
    }
    model.all.iter().filter(|r| model.selection.contains(&r.id)).collect()
}

/// Why command `id` of the row menu of `row` cannot act on `rows` ([`menu_rows`]), if it cannot.
fn row_refuses(model: &LayersModel, id: &str, row: &LayerRow, rows: &[&LayerRow], words: &LayersWords) -> Option<Label> {
    match id {
        "layers.delete" if rows.iter().all(|r| r.is_master()) => Some(words.master_stays.clone()),
        "layers.rename" if row.is_master() => Some(words.master_stays.clone()),
        "layers.useAsMaster" if !row.is_backup() => Some(words.only_backups.clone()),
        "layers.useAsMaster" if !usable_as_masters(rows) => Some(words.one_backup.clone()),
        "layers.setIntermediate" if row.is_master() => Some(words.master_not_intermediate.clone()),
        _ => {
            // Ticked, the item removes the status; else it opens the editor, which edits one layer.
            let (_, _, part) = STATUSES.iter().find(|(set, _, _)| *set == id)?;
            (!row.has(*part) && (model.glyphs.len() > 1 || rows.len() > 1)).then(|| words.one_glyph.clone())
        }
    }
}

/// The row menu of `row`: one entry per command, the status commands ticked while the layer has that status.
fn row_entries(model: &LayersModel, row: &LayerRow, words: &LayersWords) -> Vec<Entry<String>> {
    let rows = menu_rows(model, row);
    model
        .row_menu
        .iter()
        .filter(|c| !STATUSES.iter().any(|(_, clear, _)| *clear == c.id))
        .map(|c| {
            let mut entry = Entry::new(c.id.clone(), c.label.clone());
            if let Some((_, _, part)) = STATUSES.iter().find(|(set, _, _)| *set == c.id) {
                entry = entry.checked(row.has(*part));
            }
            match c.refused.clone().map(Label::from).or_else(|| row_refuses(model, &c.id, row, &rows, words)) {
                Some(why) => entry.disabled_reason(why),
                None => entry,
            }
        })
        .collect()
}

/// The message the row menu's command `id` sends for `row`.
fn row_command(model: &LayersModel, row: &LayerRow, id: &str) -> Option<Msg> {
    let rows = menu_rows(model, row);
    if let Some((_, clear, part)) = STATUSES.iter().find(|(set, _, _)| *set == id) {
        if row.has(*part) {
            let having: Vec<&LayerRow> = rows.into_iter().filter(|r| r.has(*part)).collect();
            return on_each(model, &having, clear, &json!({}));
        }
        return model.glyphs.first().map(|glyph| edit_special(glyph, &row.id, *part));
    }
    match id {
        "layers.delete" => delete(model, &rows.into_iter().filter(|r| !r.is_master()).collect::<Vec<_>>()),
        "layers.useAsMaster" => use_as_master(model, &rows),
        _ => on_each(model, &rows, id, &json!({})),
    }
}

/// What dropping a dragged row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// `layers.reorder` to this index among the master's non-master layers.
    Reorder(usize),
    /// `layers.useAsMaster`: a backup dropped on its master's row.
    UseAsMaster,
    /// A drop under another master, or of several rows.
    Refused,
    /// Dropped where it already is.
    Nothing,
}

/// The command a drop of `layer` that does `action` runs.
fn drop_command(model: &LayersModel, layer: &str, action: DropAction) -> Option<Msg> {
    let row = model.row(layer)?;
    match action {
        DropAction::Reorder(to) => on_layers(model, &[row], "layers.reorder", &json!({ "toIndex": to })),
        DropAction::UseAsMaster => use_as_master(model, &[row]),
        DropAction::Refused | DropAction::Nothing => None,
    }
}

/// What a drag of `drag` over the row `over` that would land before the shown row at `insert` does.
fn drop_on(model: &LayersModel, shown: &[&LayerRow], drag: &ListDrag<String>, over: Option<&String>, insert: usize) -> DropAction {
    let [layer] = drag.keys.as_slice() else { return DropAction::Refused };
    let above = insert.checked_sub(1).and_then(|at| shown.get(at)).copied();
    let below = shown.get(insert).copied();
    let over = over.and_then(|id| model.row(id));
    drop_action(model, layer, above, below, over)
}

/// What a move by key does: `moved.to` counts the shown rows without the moved one.
fn key_move(model: &LayersModel, shown: &[&LayerRow], moved: &ReorderMove<String>) -> DropAction {
    if moved.keys.len() != 1 {
        return DropAction::Refused;
    }
    let rest: Vec<&LayerRow> = shown.iter().copied().filter(|r| r.id != moved.key).collect();
    let above = moved.to.checked_sub(1).and_then(|at| rest.get(at)).copied();
    drop_action(model, &moved.key, above, rest.get(moved.to).copied(), None)
}

/// What dropping `layer` between the rows `above` and `below`, with the pointer over the row `over`, does.
/// `layers.reorder` counts its index among all the non-master layers of the master, filtered rows included.
pub fn drop_action(model: &LayersModel, layer: &str, above: Option<&LayerRow>, below: Option<&LayerRow>, over: Option<&LayerRow>) -> DropAction {
    let Some(dragged) = model.row(layer).filter(|r| !r.is_master()) else { return DropAction::Refused };
    let siblings: Vec<&str> = model.all.iter().filter(|r| r.master == dragged.master && !r.is_master()).map(|r| r.id.as_str()).collect();
    let Some(from) = siblings.iter().position(|id| *id == layer) else { return DropAction::Refused };
    let sibling = |row: Option<&LayerRow>| {
        row.filter(|r| r.master == dragged.master && !r.is_master()).and_then(|r| siblings.iter().position(|id| *id == r.id))
    };
    let own_master = |row: Option<&LayerRow>| row.is_some_and(|r| r.is_master() && r.master == dragged.master);
    let slot = if own_master(over) {
        if dragged.is_backup() {
            return DropAction::UseAsMaster;
        }
        0
    } else if let Some(at) = sibling(below) {
        at
    } else if let Some(at) = sibling(above) {
        at + 1
    } else if own_master(above) {
        0
    } else {
        return DropAction::Refused;
    };
    // `slot` counts the dragged row; the command's index counts the rows without it.
    let to = if from < slot { slot - 1 } else { slot };
    if to == from { DropAction::Nothing } else { DropAction::Reorder(to) }
}

#[cfg(test)]
mod tests;

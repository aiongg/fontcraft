//! The Exports tab: the export list at the left (static instances and variable font settings), and the selected
//! entries' General fields and Custom Parameters at the right.
//!
//! | Control | Posts |
//! |---|---|
//! | an entry: click, Cmd-click, Shift-click | `font.info.exports.select` |
//! | drag entries | `font.instance.reorder` with every dragged one |
//! | plus menu: Add Instance, Add Instance for Each Master, Instance as Master, Add Variable Font Setting | their `font.instance.*` command; disabled while the engine has none |
//! | minus, or Delete, Backspace or Cmd+Delete on the list | `font.instance.remove` on the selected entries |
//! | Active | `font.instance.setActive` |
//! | Style Name (Name for a variable font setting) | `font.instance.set` with `styleName` |
//! | Weight Class, Width Class and their menus of standard values | `font.instance.setWeightClass`, `font.instance.setWidthClass` |
//! | Axes Coordinates: one number field per axis | `font.instance.setAxisLocation` with a location list that sets that axis alone |
//! | Style Linking: Bold, Italic, base style name | `font.instance.setStyleLinking` |
//! | General plus, Custom Parameters plus | `font.info.openDialog` with the field or parameter picker |
//! | added fields and parameters | the shared rows ([`super::rows`]) |
//!
//! The list takes Copy and Paste for the selected rows ([`rows::list_edits`]).
//!
//! The list and the fields scroll apart, so the list stays while the fields move. The reply of a run that adds or moves
//! entries selects them (`shell::font_info::after_run`).
//!
//! Every edit goes to all selected entries; a field whose entries differ shows mixed. The static-only fields (classes,
//! coordinates, style linking) edit the selected static instances and do not show when the first selected entry is
//! a variable font setting. A selected instance beyond the masters on some axis shows the extrapolation warning; the
//! warnings `font.instance.list` gives (style name, weight class, style linking) show under their fields.
//!
//! The As Exported section ([`exported_names`]) shows what the first selected entry's export writes; the list marks the
//! entries that take a name another active entry takes.

use std::borrow::Cow;

use craft::behavior::{EditEvent, GesturePhase, IconId, Mixed, Selection, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::IconButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, ReorderRule, Scroll, Show as _, ShowCollection as _, ShowContainer as _, Trigger,
};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::menu::Menu;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use fontcraft_engine::fontinfo::Scope;
use serde_json::{Value, json};

use super::exported_names::{self, ExportedWords};
use super::param_value::number_json;
use super::rows;
use crate::app::Msg;
use crate::app::models::font_exports::{ExportFields, ExportsModel, Flag, MAX_WARNINGS_SHOWN, PLUS_MENU, WEIGHT_CLASSES, WIDTH_CLASSES};
use crate::shell::font_info::{InfoDialog, Target, open_params};
use crate::shell::font_info_tabs::select_exports;

craft::ui::words! {
    /// The words of the Exports tab.
    pub struct ExportsWords {
        list: "fontcraftExports.list" = "Exports";
        untitled: "fontcraftExports.untitled" = "Untitled";
        variable: "fontcraftExports.variable" = "Variable";
        add: "fontcraftExports.add" = "Add an export";
        add_instance: "fontcraftExports.addInstance" = "Add Instance";
        add_each_master: "fontcraftExports.addEachMaster" = "Add Instance for Each Master";
        to_master: "fontcraftExports.toMaster" = "Instance as Master";
        add_variable: "fontcraftExports.addVariable" = "Add Variable Font Setting";
        not_yet: "fontcraftExports.notYet" = "Not available yet";
        remove: "fontcraftExports.remove" = "Remove the selected exports";
        no_exports: "fontcraftExports.noExports" = "This font has no exports. Add one with the plus button below the list.";
        general: "fontcraftExports.general" = "General";
        add_field: "fontcraftExports.addField" = "Add a field";
        active: "fontcraftExports.active" = "Active";
        active_tip: "fontcraftExports.activeTip" = "Export this entry with File › Export";
        style_name: "fontcraftExports.styleName" = "Style Name";
        name: "fontcraftExports.name" = "Name";
        weight_class: "fontcraftExports.weightClass" = "Weight Class";
        width_class: "fontcraftExports.widthClass" = "Width Class";
        standard_values: "fontcraftExports.standardValues" = "Standard values";
        axes: "fontcraftExports.axes" = "Axes Coordinates";
        more_extrapolated: "fontcraftExports.moreExtrapolated" = "more extrapolated";
        style_linking: "fontcraftExports.styleLinking" = "Style Linking";
        bold: "fontcraftExports.bold" = "Bold";
        bold_tip: "fontcraftExports.boldTip" = "The bold style of the base instance";
        italic: "fontcraftExports.italic" = "Italic";
        italic_tip: "fontcraftExports.italicTip" = "The italic style of the base instance";
        link_style: "fontcraftExports.linkStyle" = "Base Style Name";
        parameters: "fontcraftExports.parameters" = "Custom Parameters";
        add_parameter: "fontcraftExports.addParameter" = "Add a custom parameter";
        thin: "fontcraftExports.thin" = "Thin";
        extra_light: "fontcraftExports.extraLight" = "Extra-light";
        light: "fontcraftExports.light" = "Light";
        regular: "fontcraftExports.regular" = "Regular";
        medium: "fontcraftExports.medium" = "Medium";
        semi_bold: "fontcraftExports.semiBold" = "Semi-bold";
        bold_weight: "fontcraftExports.boldWeight" = "Bold";
        extra_bold: "fontcraftExports.extraBold" = "Extra-bold";
        black: "fontcraftExports.black" = "Black";
        ultra_condensed: "fontcraftExports.ultraCondensed" = "Ultra-condensed";
        extra_condensed: "fontcraftExports.extraCondensed" = "Extra-condensed";
        condensed: "fontcraftExports.condensed" = "Condensed";
        semi_condensed: "fontcraftExports.semiCondensed" = "Semi-condensed";
        medium_width: "fontcraftExports.mediumWidth" = "Medium (normal)";
        semi_expanded: "fontcraftExports.semiExpanded" = "Semi-expanded";
        expanded: "fontcraftExports.expanded" = "Expanded";
        extra_expanded: "fontcraftExports.extraExpanded" = "Extra-expanded";
        ultra_expanded: "fontcraftExports.ultraExpanded" = "Ultra-expanded";
    }
}

impl ExportsWords {
    fn plus_label(&self, command: &str) -> Label {
        match command {
            "font.instance.addForEachMaster" => self.add_each_master.clone(),
            "font.instance.toMaster" => self.to_master.clone(),
            "font.instance.addVariableFontSetting" => self.add_variable.clone(),
            _ => self.add_instance.clone(),
        }
    }

    /// The standard weight classes with their OpenType names.
    pub fn weights(&self) -> [&Label; 9] {
        [&self.thin, &self.extra_light, &self.light, &self.regular, &self.medium, &self.semi_bold, &self.bold_weight, &self.extra_bold, &self.black]
    }

    pub fn widths(&self) -> [&Label; 9] {
        [
            &self.ultra_condensed,
            &self.extra_condensed,
            &self.condensed,
            &self.semi_condensed,
            &self.medium_width,
            &self.semi_expanded,
            &self.expanded,
            &self.extra_expanded,
            &self.ultra_expanded,
        ]
    }
}

/// The value of an event that ended: a commit or a set.
fn done<T>(event: &EditEvent<T>) -> Option<&T> {
    match event.phase() {
        Some(GesturePhase::Commit) | None => event.value(),
        _ => None,
    }
}

pub fn view(ui: &mut Ui, model: &ExportsModel, masters: &[(String, String)], sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ExportsWords>();
    Row::new().show(ui, |ui, _| {
        let mut buttons = Vec::new();
        Stack::new()
            .id_salt("fontcraftExports.list")
            .scroll(Scroll::vertical())
            .footer(|ui, _| list_buttons(ui, model, &words, &mut buttons))
            .show(ui, |ui, _| export_list(ui, model, &words, sent));
        sent.append(&mut buttons);
        Stack::new().id_salt("fontcraftExports.fields").scroll(Scroll::vertical()).fill().show(ui, |ui, _| {
            let Some(fields) = &model.fields else {
                Text::new(words.no_exports.clone()).shade(Shade::Muted).show(ui);
                return;
            };
            let target = Target { scope: Scope::Instance, indices: model.selected.clone() };
            let open = |dialog: InfoDialog| Msg::Shell(CommandRef::with_params("font.info.openDialog", open_params(&dialog)));
            let mut posted = Vec::new();
            let plus = rows::section(ui, "exportsGeneral", words.general.clone(), words.add_field.clone(), |ui| {
                general(ui, fields, &model.selected, &words, &mut posted);
                rows::rows(ui, &model.rows, &target, masters, &mut posted);
            });
            if plus {
                posted.push(open(InfoDialog::FieldPicker(target.clone())));
            }
            let plus = rows::section(ui, "exportsParameters", words.parameters.clone(), words.add_parameter.clone(), |ui| {
                rows::rows(ui, &model.params, &target, masters, &mut posted);
            });
            if plus {
                posted.push(open(InfoDialog::ParameterPicker(target)));
            }
            if let Some(exported) = &model.exported {
                exported_names::view(ui, exported);
            }
            sent.extend(posted);
        });
    });
}

/// The message that removes the selected entries; none without entries.
fn remove_selected(model: &ExportsModel) -> Option<Msg> {
    (!model.entries.is_empty()).then(|| Msg::run("font.instance.remove", json!({ "instances": model.selected })))
}

/// The list: selects, drags and removes entries.
fn export_list(ui: &mut Ui, model: &ExportsModel, words: &ExportsWords, sent: &mut Vec<Msg>) {
    let entries = &model.entries;
    let conflict = Env::of(ui).words::<ExportedWords>().conflict.clone();
    let lead = model.selected.first().copied();
    let before = Selection::new(model.selected.clone(), lead, lead);
    let mut after = before.clone();
    let positions: Vec<usize> = (0..entries.len()).collect();
    let name = |at: usize| {
        entries.get(at).map_or_else(String::new, |e| if e.name.is_empty() { words.untitled.as_str().to_string() } else { e.name.clone() })
    };
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(name(*at)));
    let out = List::<usize>::new(words.list.clone())
        .header(false)
        .natural_height(true)
        .reorder(ReorderRule::new())
        .deletable(true)
        .selection(Bind::bound(&mut after))
        .show_items(ui, &source, &mut |at, icx| {
            let Some(e) = entries.get(*at) else { return ListRow::new(Label::Empty) };
            let mut row = ListRow::new(name(*at)).state(icx.row.clone());
            if !e.active {
                row = row.ink(RowInk::Dimmed);
            }
            if e.variable {
                row = row.trailing_strong(words.variable.clone(), words.variable.clone());
            }
            if model.conflicts.contains(at) {
                row = row.trailing_icon(IconId::new("warning"), conflict.clone());
            }
            row
        });
    if after.items != before.items {
        let mut picked = after.items.clone();
        picked.sort_unstable();
        sent.push(Msg::Shell(select_exports(&picked)));
    }
    for event in &out.events {
        match event {
            CollectionEvent::Reorder(edit) => {
                if let Some(moved) = done(edit) {
                    sent.push(Msg::run("font.instance.reorder", json!({ "instances": moved.keys, "toIndex": moved.to })));
                }
            }
            CollectionEvent::Delete(_) => sent.extend(remove_selected(model)),
            _ => {}
        }
    }
    sent.extend(rows::list_edits(ui, &out, remove_selected(model)));
}

/// The plus menu and the minus button under the list.
fn list_buttons(ui: &mut Ui, model: &ExportsModel, words: &ExportsWords, sent: &mut Vec<Msg>) {
    Row::new().show(ui, |ui, _| {
        let plus = IconButton::new(IconId::new("plus")).label(words.add.clone()).tooltip(words.add.clone()).show(ui);
        let items: Vec<Entry<&'static str>> = PLUS_MENU
            .iter()
            .zip(&model.plus)
            .map(|(id, on)| {
                let entry = Entry::new(*id, words.plus_label(id)).enabled(*on);
                if *on { entry } else { entry.disabled_reason(words.not_yet.clone()) }
            })
            .collect();
        if let Some(command) = Menu::new(words.add.clone()).opened_by(&plus.response, Trigger::Click).pick(ui, &items).picked {
            let params = match command {
                "font.instance.toMaster" => json!({ "instance": model.selected.first().copied().unwrap_or(0) }),
                _ => json!({}),
            };
            sent.push(Msg::run(command, params));
        }
        let remove = remove_selected(model);
        let minus = IconButton::new(IconId::new("minus")).label(words.remove.clone()).tooltip(words.remove.clone()).enabled(remove.is_some());
        if minus.show(ui).clicked() {
            sent.extend(remove);
        }
    });
}

pub fn mixed_bool(flag: Flag) -> Mixed<bool> {
    if flag.mixed { Mixed::Mixed } else { Mixed::Same(flag.on) }
}

pub fn mixed_text(text: &str, mixed: bool) -> Mixed<String> {
    if mixed { Mixed::Mixed } else { Mixed::Same(text.to_string()) }
}

/// The new value of a check box that was clicked: a mixed box becomes checked.
pub fn flipped(flag: Flag) -> bool {
    flag.mixed || !flag.on
}

/// The built-in fields of the selected entries.
fn general(ui: &mut Ui, f: &ExportFields, selected: &[usize], words: &ExportsWords, sent: &mut Vec<Msg>) {
    let all = |id: &str, key: &str, value: Value| Msg::run(id, json!({ "instances": selected, key: value }));
    let statics = |id: &str, key: &str, value: Value| Msg::run(id, json!({ "instances": f.statics, key: value }));
    Field::new(words.active.clone()).show(ui, |ui, _| {
        let check = Checkbox::new(Bind::given(mixed_bool(f.active))).label(words.active.clone()).tooltip(words.active_tip.clone());
        if check.show(ui).events.committed().is_some() {
            sent.push(all("font.instance.setActive", "active", json!(flipped(f.active))));
        }
    });
    let name_label = if f.variable { words.name.clone() } else { words.style_name.clone() };
    Field::new(name_label.clone()).show(ui, |ui, _| {
        if let Some(text) = TextField::new(name_label.clone(), Bind::given(mixed_text(&f.name, f.name_mixed))).show(ui).events.committed() {
            sent.push(all("font.instance.set", "styleName", json!(text)));
        }
    });
    rows::warnings(ui, &f.field_warnings, "styleName");
    if f.variable {
        return;
    }
    let names: Vec<Label> = words.weights().into_iter().cloned().collect();
    if let Some(v) = class_row(ui, &words.weight_class, "weight", f.weight, &WEIGHT_CLASSES, &names, &words.standard_values) {
        sent.push(statics("font.instance.setWeightClass", "weightClass", number_json(v)));
    }
    rows::warnings(ui, &f.field_warnings, "weightClass");
    let names: Vec<Label> = words.widths().into_iter().cloned().collect();
    if let Some(v) = class_row(ui, &words.width_class, "width", f.width, &WIDTH_CLASSES, &names, &words.standard_values) {
        sent.push(statics("font.instance.setWidthClass", "widthClass", number_json(v)));
    }
    if !f.axes.is_empty() {
        Text::new(words.axes.clone()).shade(Shade::Muted).show(ui);
        for (at, (axis, value)) in f.axes.iter().enumerate() {
            Field::new(axis.clone()).id_salt(("fontcraftExports.axis", at)).show(ui, |ui, _| {
                let given = value.map_or(Mixed::Mixed, |v| Mixed::Same(Some(v)));
                if let Some(Some(v)) = NumberField::new(axis.clone(), Bind::given(given)).show(ui).edit.events.committed() {
                    sent.push(Msg::run("font.instance.setAxisLocation", json!({ "instances": f.statics, "location": location_list(at, *v) })));
                }
            });
        }
        for warning in f.warnings.iter().take(MAX_WARNINGS_SHOWN) {
            Text::new(warning.clone()).tone(Tone::Warn).show(ui);
        }
        if f.warnings.len() > MAX_WARNINGS_SHOWN {
            Text::new(words.more_extrapolated.clone()).tone(Tone::Warn).show(ui);
        }
    }
    Field::new(words.style_linking.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let bold = Checkbox::new(Bind::given(mixed_bool(f.bold))).label(words.bold.clone()).tooltip(words.bold_tip.clone());
            if bold.show(ui).events.committed().is_some() {
                sent.push(statics("font.instance.setStyleLinking", "isBold", json!(flipped(f.bold))));
            }
            let italic = Checkbox::new(Bind::given(mixed_bool(f.italic))).label(words.italic.clone()).tooltip(words.italic_tip.clone());
            if italic.show(ui).events.committed().is_some() {
                sent.push(statics("font.instance.setStyleLinking", "isItalic", json!(flipped(f.italic))));
            }
            let link =
                TextField::new(words.link_style.clone(), Bind::given(mixed_text(&f.link_style, f.link_mixed))).placeholder(words.link_style.clone());
            if let Some(text) = link.show(ui).events.committed() {
                sent.push(statics("font.instance.setStyleLinking", "linkStyle", json!(text.trim())));
            }
        });
    });
    rows::warnings(ui, &f.field_warnings, "styleLinking");
}

/// A class field with its menu of standard values; the value the user chose.
pub fn class_row(ui: &mut Ui, label: &Label, salt: &str, value: Option<f64>, standard: &[u16], names: &[Label], menu: &Label) -> Option<f64> {
    let mut chosen = None;
    Field::new(label.clone()).id_salt(("fontcraftExports.class", salt)).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            if let Some(Some(n)) = NumberField::new(label.clone(), Bind::given(value)).show(ui).edit.events.committed() {
                chosen = Some(*n);
            }
            let button = IconButton::new(IconId::new("chevron_down")).label(menu.clone()).tooltip(menu.clone()).id_salt((salt, "menu")).show(ui);
            let entries: Vec<Entry<u16>> = standard
                .iter()
                .zip(names)
                .map(|(n, name)| Entry::new(*n, format!("{n} {}", name.as_str())).checked(value == Some(f64::from(*n))))
                .collect();
            if let Some(n) = Menu::new(menu.clone()).id_salt((salt, "standard")).opened_by(&button.response, Trigger::Click).pick(ui, &entries).picked
            {
                chosen = Some(f64::from(n));
            }
        });
    });
    chosen
}

/// A `location` in the list form the axis-location commands take: `v` on the axis at `at`, the axes before it left
/// alone. The list addresses the axis by index, so it is the right one even when two axes share a tag.
pub fn location_list(at: usize, v: f64) -> Value {
    Value::Array((0..=at).map(|k| if k == at { json!(v) } else { Value::Null }).collect())
}

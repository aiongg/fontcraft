//! The Masters tab: the master list at the left, and the selected masters' sections at the right.
//!
//! | Control | Posts |
//! |---|---|
//! | a master: click, Cmd-click, Shift-click | `font.info.masters.select` |
//! | drag a master | `font.master.reorder` |
//! | plus menu: Add Master, Add Other Font…, Duplicate Selected | `font.master.add`, the Add Other Font dialog, `font.master.duplicate` |
//! | minus | `font.master.remove` on the selected masters (not all of them) |
//! | Master Name | `font.info.set` through the shared rows |
//! | Icon | the icon picker dialog (`font.master.setIcon`) |
//! | Axes Coordinates: one number field per axis | `font.master.setAxisLocation` with a location list that sets that axis alone |
//! | Designspace | the map, its table and the problems ([`super::designspace`]) |
//! | a metric's position and zone | `font.master.setMetric` |
//! | a metric's edit button, the plus beside Metrics | the metric dialogs (`font.metrics.set`, `font.metrics.add`) |
//! | a metric's minus | `font.metrics.remove` |
//! | a stem's orientation, name | `font.stems.set` |
//! | a stem's width | `font.master.setStem` |
//! | a stem's scope button | the stem scope dialog (`font.stems.set`) |
//! | the plus beside Stems, a stem's minus | `font.stems.add`, `font.stems.remove` |
//! | Custom Parameters | the shared rows; the plus opens the parameter picker |
//! | the plus beside Number Values | `font.numbers.add` without a name (the engine picks `value`, `value1`…) |
//! | a number's name | `font.numbers.rename` |
//! | a number's value | `font.master.setNumber` |
//! | a number's minus | `font.numbers.remove` |
//!
//! Every edit goes to all selected masters; a field whose masters differ shows mixed.

use std::borrow::Cow;

use craft::behavior::{EditEvent, GesturePhase, IconId, Mixed, Selection, keyed};
use craft::ui::buttons::IconButton;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, ReorderRule, Show as _, ShowCollection as _, ShowContainer as _, Trigger};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbSize};
use craft::ui::menu::Menu;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};

use super::rows;
use crate::app::Msg;
use crate::app::models::font_masters::MastersTabModel;
use crate::panels::master_icons::{MasterGlyph, paint_icon};
use crate::shell::font_info::{InfoDialog, open_params};
use crate::shell::font_info_masters::select_masters;

craft::ui::words! {
    /// The words of the Masters tab.
    pub struct MastersWords {
        list: "fontcraftMasters.list" = "Masters";
        add: "fontcraftMasters.add" = "Add a master";
        add_master: "fontcraftMasters.addMaster" = "Add Master";
        add_other_font: "fontcraftMasters.addOtherFont" = "Add Other Font…";
        duplicate: "fontcraftMasters.duplicate" = "Duplicate Selected";
        remove: "fontcraftMasters.remove" = "Remove the selected masters";
        no_masters: "fontcraftMasters.noMasters" = "The font has no master.";
        general: "fontcraftMasters.general" = "General";
        icon: "fontcraftMasters.icon" = "Icon";
        icon_tip: "fontcraftMasters.iconTip" = "Choose the master's icon";
        axes: "fontcraftMasters.axes" = "Axes Coordinates";
        metrics: "fontcraftMasters.metrics" = "Metrics & Alignment Zones";
        add_metric: "fontcraftMasters.addMetric" = "Add a metric";
        position: "fontcraftMasters.position" = "Position";
        zone: "fontcraftMasters.zone" = "Zone";
        edit_metric: "fontcraftMasters.editMetric" = "Edit the category, name and scope";
        remove_metric: "fontcraftMasters.removeMetric" = "Remove this metric from every master";
        stems: "fontcraftMasters.stems" = "Stems";
        add_stem: "fontcraftMasters.addStem" = "Add a stem";
        horizontal: "fontcraftMasters.horizontal" = "Horizontal";
        horizontal_tip: "fontcraftMasters.horizontalTip" = "Pressed: a horizontal stem, measured top to bottom; released: vertical, measured left to right";
        stem_name: "fontcraftMasters.stemName" = "Stem Name";
        width: "fontcraftMasters.width" = "Width";
        stem_scope: "fontcraftMasters.stemScope" = "Limit the stem to some glyphs";
        remove_stem: "fontcraftMasters.removeStem" = "Remove this stem from every master";
        parameters: "fontcraftMasters.parameters" = "Custom Parameters";
        add_parameter: "fontcraftMasters.addParameter" = "Add a custom parameter";
        numbers: "fontcraftMasters.numbers" = "Number Values";
        add_number: "fontcraftMasters.addNumber" = "Add a number value";
        number_name: "fontcraftMasters.numberName" = "Number Name";
        remove_number: "fontcraftMasters.removeNumber" = "Remove this number value from every master";
        none: "fontcraftMasters.none" = "None";
        multiple: "fontcraftMasters.multiple" = "Multiple Values";
        scope: "fontcraftMasters.scope" = "Scope:";
    }
}

/// The items of the plus menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Plus {
    Add,
    OtherFont,
    Duplicate,
}

/// The value of an event that ended: a commit or a set.
fn done<T>(event: &EditEvent<T>) -> Option<&T> {
    match event.phase() {
        Some(GesturePhase::Commit) | None => event.value(),
        _ => None,
    }
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// The message that opens the spec dialog `dialog` with `params`.
pub fn open_dialog(dialog: &str, params: Value) -> Msg {
    shell("dialog.open", json!({ "dialog": dialog, "params": params }))
}

/// The location list `font.master.setAxisLocation` takes to set axis `at` alone.
pub fn location_list(at: usize, v: f64) -> Value {
    Value::Array((0..=at).map(|k| if k == at { json!(v) } else { Value::Null }).collect())
}

fn mixed_number(value: Option<f64>) -> Mixed<Option<f64>> {
    value.map_or(Mixed::Mixed, |v| Mixed::Same(Some(v)))
}

/// A number field showing `value`, or mixed for `None`; the number committed.
fn number(ui: &mut Ui, label: &Label, value: Option<f64>) -> Option<f64> {
    NumberField::new(label.clone(), Bind::given(mixed_number(value))).show(ui).edit.events.committed().copied().flatten()
}

pub fn view(ui: &mut Ui, model: &MastersTabModel, masters: &[(String, String)], sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<MastersWords>();
    if model.masters.is_empty() {
        Text::new(words.no_masters.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    Row::new().show(ui, |ui, _| {
        Stack::new().id_salt("fontcraftMasters.list").show(ui, |ui, _| master_list(ui, model, &words, sent));
        Stack::new().id_salt("fontcraftMasters.fields").fill_width().show(ui, |ui, _| {
            let target = model.target();
            let sel = &model.selected;
            let mut posted = Vec::new();
            Section::new(words.general.clone()).id_salt("mastersGeneral").show(ui, |ui, _| {
                if let Some(name) = &model.name {
                    rows::rows(ui, std::slice::from_ref(name), &target, masters, &mut posted);
                }
                icon_row(ui, model, &words, &mut posted);
            });
            if !model.axes.is_empty() {
                Section::new(words.axes.clone()).id_salt("mastersAxes").show(ui, |ui, _| {
                    for (at, (axis, value)) in model.axes.iter().enumerate() {
                        let label = Label::from(axis.clone());
                        Field::new(label.clone()).id_salt(("fontcraftMasters.axis", at)).show(ui, |ui, _| {
                            if let Some(v) = number(ui, &label, *value) {
                                posted.push(Msg::run("font.master.setAxisLocation", json!({ "masters": sel, "location": location_list(at, v) })));
                            }
                        });
                    }
                });
            }
            super::designspace::view(ui, &model.designspace, &mut posted);
            let plus =
                rows::section(ui, "mastersMetrics", words.metrics.clone(), words.add_metric.clone(), |ui| metrics(ui, model, &words, &mut posted));
            if plus {
                posted.push(open_dialog("fontInfo.addMetric", json!({})));
            }
            let plus = rows::section(ui, "mastersStems", words.stems.clone(), words.add_stem.clone(), |ui| stems(ui, model, &words, &mut posted));
            if plus {
                posted.push(Msg::run("font.stems.add", json!({ "orientation": "vertical" })));
            }
            let plus = rows::section(ui, "mastersParameters", words.parameters.clone(), words.add_parameter.clone(), |ui| {
                rows::rows(ui, &model.params, &target, masters, &mut posted);
            });
            if plus {
                posted.push(shell("font.info.openDialog", open_params(&InfoDialog::ParameterPicker(target.clone()))));
            }
            let plus =
                rows::section(ui, "mastersNumbers", words.numbers.clone(), words.add_number.clone(), |ui| numbers(ui, model, &words, &mut posted));
            if plus {
                posted.push(Msg::run("font.numbers.add", json!({})));
            }
            sent.extend(posted);
        });
    });
}

/// The list with its plus menu and minus button.
fn master_list(ui: &mut Ui, model: &MastersTabModel, words: &MastersWords, sent: &mut Vec<Msg>) {
    let items = &model.masters;
    let lead = model.selected.first().copied();
    let before = Selection::new(model.selected.clone(), lead, lead);
    let mut after = before.clone();
    let positions: Vec<usize> = (0..items.len()).collect();
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(items.get(*at).map_or_else(String::new, |m| m.name.clone())));
    let out = List::<usize>::new(words.list.clone())
        .header(false)
        .natural_height(true)
        .reorder(ReorderRule::new())
        .selection(Bind::bound(&mut after))
        .show_items(ui, &source, &mut |at, icx| {
            let Some(m) = items.get(*at) else { return ListRow::new(Label::Empty) };
            let icon = m.icon.clone();
            ListRow::new(m.name.clone())
                .state(icx.row.clone())
                .thumb(ThumbSize::Small, move |cx, rect| paint_icon(&cx.painter, rect, cx.ink(), &icon))
        });
    if after.items != before.items {
        let mut picked = after.items.clone();
        picked.sort_unstable();
        sent.push(Msg::Shell(select_masters(&picked)));
    }
    for event in &out.events {
        if let CollectionEvent::Reorder(edit) = event
            && let Some(moved) = done(edit)
            && moved.keys.len() == 1
            && moved.to != moved.key
        {
            sent.push(Msg::run("font.master.reorder", json!({ "master": moved.key, "toIndex": moved.to })));
            sent.push(Msg::Shell(select_masters(&[moved.to])));
        }
    }
    let count = items.len();
    Row::new().show(ui, |ui, _| {
        let plus = IconButton::new(IconId::new("plus")).label(words.add.clone()).tooltip(words.add.clone()).show(ui);
        let entries = [
            Entry::new(Plus::Add, words.add_master.clone()),
            Entry::new(Plus::OtherFont, words.add_other_font.clone()),
            Entry::new(Plus::Duplicate, words.duplicate.clone()),
        ];
        match Menu::new(words.add.clone()).opened_by(&plus.response, Trigger::Click).pick(ui, &entries).picked {
            Some(Plus::Add) => {
                sent.push(Msg::run("font.master.add", json!({})));
                sent.push(Msg::Shell(select_masters(&[count])));
            }
            Some(Plus::OtherFont) => sent.push(open_dialog("fontInfo.addOtherFont", json!({}))),
            Some(Plus::Duplicate) => sent.push(Msg::run("font.master.duplicate", json!({ "masters": model.selected }))),
            None => {}
        }
        let removable = model.selected.len() < count;
        let minus = IconButton::new(IconId::new("minus")).label(words.remove.clone()).tooltip(words.remove.clone()).enabled(removable);
        if minus.show(ui).clicked() && removable {
            sent.push(Msg::run("font.master.remove", json!({ "masters": model.selected })));
            sent.push(Msg::Shell(select_masters(&[])));
        }
    });
}

/// The icon button of the General section, with Multiple Values beside it when the masters disagree.
fn icon_row(ui: &mut Ui, model: &MastersTabModel, words: &MastersWords, sent: &mut Vec<Msg>) {
    let Some(icon) = &model.icon else { return };
    Field::new(words.icon.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let glyph = MasterGlyph(icon);
            let out = ToggleButton::new(words.icon.clone(), Bind::given(false))
                .icon(IconSpec::glyph(&glyph))
                .icon_only()
                .tooltip(words.icon_tip.clone())
                .id_salt("fontcraftMasters.icon")
                .show(ui);
            if out.events.committed().is_some() {
                sent.push(open_dialog("fontInfo.masterIconPicker", json!({ "masters": model.selected })));
            }
            if model.icon_mixed {
                Text::new(words.multiple.clone()).shade(Shade::Muted).show(ui);
            }
        });
    });
}

/// Metrics & Alignment Zones: per metric its position, zone, edit button and minus.
fn metrics(ui: &mut Ui, model: &MastersTabModel, words: &MastersWords, sent: &mut Vec<Msg>) {
    let sel = &model.selected;
    for (at, m) in model.metrics.iter().enumerate() {
        Field::new(m.label.clone()).id_salt(("fontcraftMasters.metric", at)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                if let Some(v) = number(ui, &words.position, m.position) {
                    sent.push(Msg::run("font.master.setMetric", json!({ "masters": sel, "metric": at, "position": v })));
                }
                if !m.angle
                    && let Some(v) = number(ui, &words.zone, m.zone)
                {
                    sent.push(Msg::run("font.master.setMetric", json!({ "masters": sel, "metric": at, "zone": v })));
                }
                let tip = scoped(&words.edit_metric, &words.scope, &m.scope);
                if IconButton::new(IconId::new("more")).label(words.edit_metric.clone()).tooltip(tip).id_salt(("metricEdit", at)).show(ui).clicked() {
                    sent.push(open_dialog("fontInfo.metricEditor", json!({ "metric": at })));
                }
                let minus = IconButton::new(IconId::new("minus")).label(words.remove_metric.clone()).tooltip(words.remove_metric.clone());
                if minus.id_salt(("metricRemove", at)).show(ui).clicked() {
                    sent.push(Msg::run("font.metrics.remove", json!({ "metric": at })));
                }
            });
        });
    }
}

/// Stems: per stem its orientation, name, width, scope button and minus.
fn stems(ui: &mut Ui, model: &MastersTabModel, words: &MastersWords, sent: &mut Vec<Msg>) {
    let sel = &model.selected;
    for (at, s) in model.stems.iter().enumerate() {
        Row::new().id_salt(("fontcraftMasters.stem", at)).show(ui, |ui, _| {
            let orientation = ToggleButton::new(words.horizontal.clone(), Bind::given(s.horizontal)).tooltip(words.horizontal_tip.clone());
            if orientation.show(ui).events.committed().is_some() {
                let other = if s.horizontal { "vertical" } else { "horizontal" };
                sent.push(Msg::run("font.stems.set", json!({ "stem": at, "orientation": other })));
            }
            if let Some(name) = TextField::new(words.stem_name.clone(), Bind::given(s.name.clone())).show(ui).events.committed() {
                sent.push(Msg::run("font.stems.set", json!({ "stem": at, "name": name.trim() })));
            }
            if let Some(v) = number(ui, &words.width, s.value) {
                sent.push(Msg::run("font.master.setStem", json!({ "masters": sel, "stem": at, "value": v })));
            }
            let tip = scoped(&words.stem_scope, &words.scope, &s.scope);
            if IconButton::new(IconId::new("more")).label(words.stem_scope.clone()).tooltip(tip).id_salt(("stemScope", at)).show(ui).clicked() {
                sent.push(open_dialog("fontInfo.stemScope", json!({ "stem": at })));
            }
            let minus = IconButton::new(IconId::new("minus")).label(words.remove_stem.clone()).tooltip(words.remove_stem.clone());
            if minus.id_salt(("stemRemove", at)).show(ui).clicked() {
                sent.push(Msg::run("font.stems.remove", json!({ "stem": at })));
            }
        });
    }
}

/// Number Values: per number its name, the selection's value and minus.
fn numbers(ui: &mut Ui, model: &MastersTabModel, words: &MastersWords, sent: &mut Vec<Msg>) {
    if model.numbers.is_empty() {
        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
    }
    let sel = &model.selected;
    for (at, (name, value)) in model.numbers.iter().enumerate() {
        Row::new().id_salt(("fontcraftMasters.number", at)).show(ui, |ui, _| {
            if let Some(text) = TextField::new(words.number_name.clone(), Bind::given(name.clone())).show(ui).events.committed() {
                let text = text.trim();
                if !text.is_empty() && text != name {
                    sent.push(Msg::run("font.numbers.rename", json!({ "number": at, "name": text })));
                }
            }
            if let Some(v) = number(ui, &Label::from(name.clone()), *value) {
                sent.push(Msg::run("font.master.setNumber", json!({ "masters": sel, "number": at, "value": v })));
            }
            let minus = IconButton::new(IconId::new("minus")).label(words.remove_number.clone()).tooltip(words.remove_number.clone());
            if minus.id_salt(("numberRemove", at)).show(ui).clicked() {
                sent.push(Msg::run("font.numbers.remove", json!({ "number": at })));
            }
        });
    }
}

/// A tooltip with the scope after it, when there is one.
fn scoped(tip: &Label, word: &Label, scope: &str) -> Label {
    if scope.is_empty() { tip.clone() } else { Label::from(format!("{} ({} {scope})", tip.as_str(), word.as_str())) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::view;
    use crate::app::Msg;
    use crate::app::models::font_masters::{MasterItem, MastersTabModel, MetricItem};
    use crate::app::models::layers::MasterIcon;
    use crate::shell::font_info_masters::select_masters;

    fn model() -> MastersTabModel {
        let item = |name: &str| MasterItem { name: name.into(), icon: MasterIcon::Builtin { weight: 1, width: 2 } };
        MastersTabModel {
            masters: vec![item("Light"), item("Bold")],
            selected: vec![0, 1],
            icon: Some(MasterIcon::Builtin { weight: 1, width: 2 }),
            axes: vec![("Weight".into(), None)],
            metrics: vec![MetricItem { label: "x-Height".into(), position: Some(500.0), zone: Some(10.0), ..MetricItem::default() }],
            ..MastersTabModel::default()
        }
    }

    /// Two selected masters at different weights: the coordinate field is mixed, and a typed value goes to both, as a
    /// location list that sets the first axis alone. A click on a master selects only that master.
    #[test]
    fn a_coordinate_typed_for_two_masters_goes_to_both_and_a_click_selects_one() {
        let model = Tracked::new(model());
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &model, &[], &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let weight = fixture.by_label("Weight").expect("the Weight coordinate");
        fixture.click(&weight).type_text("120").key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(
            posted.borrow().first(),
            Some(&Msg::run("font.master.setAxisLocation", json!({"masters": [0, 1], "location": [120.0]}))),
            "{:?}",
            posted.borrow()
        );
        posted.borrow_mut().clear();
        let bold = fixture.by_label("Bold").expect("the Bold master");
        fixture.click(&bold).settle();
        assert!(posted.borrow().contains(&Msg::Shell(select_masters(&[1]))), "{:?}", posted.borrow());
    }

    /// Number Values: the plus adds a number without a name, a typed value goes to every selected master, a new name
    /// renames the number and the minus removes it.
    #[test]
    fn number_values_add_set_rename_and_remove() {
        let model = Tracked::new(MastersTabModel { numbers: vec![("value".into(), Some(10.0))], ..model() });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &model, &[], &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let plus = fixture.by_label("Add a number value").expect("the Number Values plus");
        fixture.click(&plus).settle();
        assert!(posted.borrow().contains(&Msg::run("font.numbers.add", json!({}))), "{:?}", posted.borrow());
        posted.borrow_mut().clear();
        let value = fixture.by_label("value").expect("the number's value field");
        fixture.click(&value).key(Key::A, Modifiers::COMMAND).type_text("24").key(Key::Enter, Modifiers::NONE).settle();
        assert!(
            posted.borrow().contains(&Msg::run("font.master.setNumber", json!({"masters": [0, 1], "number": 0, "value": 24.0}))),
            "{:?}",
            posted.borrow()
        );
        posted.borrow_mut().clear();
        let name = fixture.by_label("Number Name").expect("the number's name field");
        fixture.click(&name).key(Key::A, Modifiers::COMMAND).type_text("stem").key(Key::Enter, Modifiers::NONE).settle();
        assert!(posted.borrow().contains(&Msg::run("font.numbers.rename", json!({"number": 0, "name": "stem"}))), "{:?}", posted.borrow());
        posted.borrow_mut().clear();
        let minus = fixture.by_label("Remove this number value from every master").expect("the number's minus");
        fixture.click(&minus).settle();
        assert!(posted.borrow().contains(&Msg::run("font.numbers.remove", json!({"number": 0}))), "{:?}", posted.borrow());
    }
}

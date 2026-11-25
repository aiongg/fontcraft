//! The Font tab: General (Family Name and Units per Em with their warnings, Scale, Version with its stepper, Creation
//! Date with Update, the added fields), Axes and the font's Custom Parameters.
//!
//! | Control | Posts |
//! |---|---|
//! | a General field | `font.info.set` |
//! | Scale | `font.info.scaleToUnitsPerEm` |
//! | the version stepper | `font.info.stepVersion` |
//! | Update | `font.info.updateDate` |
//! | the plus beside General or Custom Parameters | `font.info.openDialog` with the field or parameter picker |
//! | the plus beside Axes | `font.axes.add` |
//! | an axis's name, predefined axis, tag, Hidden | `font.axes.set` by index |
//! | a drag in the axis list (with several axes) | `font.axes.reorder` |
//! | an axis's minus | `font.axes.remove` |

use std::borrow::Cow;

use craft::behavior::{GesturePhase, IconId, keyed};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, ReorderRule, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::pickers::{TreeEntry, TreePicker};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::text::{NumberField, TextField};
use craft_ext::ui::text::{ArrowSteps, StepKeys as _};
use egui::Ui;
use serde_json::{Value as Json, json};

use super::param_value::number_json;
use super::rows;
use crate::app::Msg;
use crate::app::models::font_info::{AxisModel, FontInfoModel, PredefinedAxis};
use crate::shell::font_info::{InfoDialog, Target, open_params};

craft::ui::words! {
    /// The words of the Font tab.
    pub struct FontTabWords {
        general: "fontcraftFontInfoFont.general" = "General";
        add_field: "fontcraftFontInfoFont.addField" = "Add a field";
        family_name: "fontcraftFontInfoFont.familyName" = "Family Name";
        units_per_em: "fontcraftFontInfoFont.unitsPerEm" = "Units per Em";
        scale_to: "fontcraftFontInfoFont.scaleTo" = "Scale to";
        scale: "fontcraftFontInfoFont.scale" = "Scale";
        scale_tip: "fontcraftFontInfoFont.scaleTip" = "Scale outlines, metrics and kerning to the new units per em";
        version: "fontcraftFontInfoFont.version" = "Version";
        version_major: "fontcraftFontInfoFont.versionMajor" = "Major Version";
        version_minor: "fontcraftFontInfoFont.versionMinor" = "Minor Version";
        version_dot: "fontcraftFontInfoFont.versionDot" = ".";
        raise_minor: "fontcraftFontInfoFont.raiseMinor" = "Raise the minor version by one";
        lower_minor: "fontcraftFontInfoFont.lowerMinor" = "Lower the minor version by one";
        date: "fontcraftFontInfoFont.date" = "Creation Date";
        no_date: "fontcraftFontInfoFont.noDate" = "No date";
        update: "fontcraftFontInfoFont.update" = "Update";
        update_tip: "fontcraftFontInfoFont.updateTip" = "Set the creation date to now";
        axes: "fontcraftFontInfoFont.axes" = "Axes";
        add_axis: "fontcraftFontInfoFont.addAxis" = "Add an axis";
        no_axes: "fontcraftFontInfoFont.noAxes" = "None";
        axis_name: "fontcraftFontInfoFont.axisName" = "Axis Name";
        predefined: "fontcraftFontInfoFont.predefined" = "Predefined Axes";
        registered: "fontcraftFontInfoFont.registered" = "Registered";
        custom: "fontcraftFontInfoFont.custom" = "Custom";
        axis_tag: "fontcraftFontInfoFont.axisTag" = "Axis Tag";
        hidden: "fontcraftFontInfoFont.hidden" = "Hidden";
        hidden_tip: "fontcraftFontInfoFont.hiddenTip" = "Hide this axis from font users";
        axis_order: "fontcraftFontInfoFont.axisOrder" = "Axis Order";
        remove_axis: "fontcraftFontInfoFont.removeAxis" = "Remove this axis and every coordinate on it";
        parameters: "fontcraftFontInfoFont.parameters" = "Custom Parameters";
        add_parameter: "fontcraftFontInfoFont.addParameter" = "Add a custom parameter";
    }
}

/// The predefined-axis picker's folder keys; an axis's key is its tag.
const REGISTERED: &str = "#registered";
const CUSTOM: &str = "#custom";

pub fn view(ui: &mut Ui, model: &FontInfoModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<FontTabWords>();
    let font = &model.font;
    let target = Target::font();
    let set = |key: &str, value: Json| Msg::run("font.info.set", json!({"scope": "font", "values": {key: value}}));
    let open = |dialog: InfoDialog| Msg::Shell(CommandRef::with_params("font.info.openDialog", open_params(&dialog)));

    let mut posted = Vec::new();
    let plus = rows::section(ui, "general", words.general.clone(), words.add_field.clone(), |ui| {
        Field::new(words.family_name.clone()).show(ui, |ui, _| {
            if let Some(text) = TextField::new(words.family_name.clone(), Bind::given(font.family_name.clone())).show(ui).events.committed() {
                posted.push(set("familyName", json!(text.trim())));
            }
        });
        rows::warnings(ui, &font.warnings, "familyName");
        Field::new(words.units_per_em.clone()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let upm = Some(f64::from(font.units_per_em));
                if let Some(Some(n)) = NumberField::new(words.units_per_em.clone(), Bind::given(upm)).show(ui).edit.events.committed() {
                    posted.push(set("unitsPerEm", number_json(*n)));
                }
                let to = NumberField::new(words.scale_to.clone(), Bind::kept().reset(upm)).show(ui);
                let wanted = to.edit.value().copied().flatten().or(upm);
                if Button::new(words.scale.clone()).tooltip(words.scale_tip.clone()).show(ui).clicked()
                    && let Some(n) = wanted
                {
                    posted.push(Msg::run("font.info.scaleToUnitsPerEm", json!({"unitsPerEm": number_json(n)})));
                }
            });
        });
        rows::warnings(ui, &font.warnings, "unitsPerEm");
        Field::new(words.version.clone()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let major = Some(f64::from(font.version_major));
                if let Some(Some(n)) = NumberField::new(words.version_major.clone(), Bind::given(major)).show(ui).edit.events.committed() {
                    posted.push(set("versionMajor", number_json(*n)));
                }
                Text::new(words.version_dot.clone()).show(ui);
                let minor = format!("{:03}", font.version_minor);
                let minor_field = TextField::new(words.version_minor.clone(), Bind::given(minor)).show(ui);
                if let Some(text) = minor_field.events.committed() {
                    // A text that is no number goes to the command, which refuses it with a message.
                    let value = text.trim().parse::<u64>().map_or_else(|_| json!(text.trim()), |n| json!(n));
                    posted.push(set("versionMinor", value));
                }
                // The arrows step the minor version as the steppers beside it do.
                if let Some(delta) = minor_field.arrow_steps(ui, ArrowSteps::UNITS) {
                    let next = (f64::from(font.version_minor) + delta).round().max(0.0);
                    posted.push(set("versionMinor", json!(next as u64)));
                }
                for (icon, label, delta) in [("plus", &words.raise_minor, 1), ("minus", &words.lower_minor, -1)] {
                    if IconButton::new(IconId::new(icon)).label(label.clone()).tooltip(label.clone()).show(ui).clicked() {
                        posted.push(Msg::run("font.info.stepVersion", json!({"delta": delta})));
                    }
                }
            });
        });
        Field::new(words.date.clone()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let shown = font.date.clone().unwrap_or_default();
                let field = TextField::new(words.date.clone(), Bind::given(shown)).placeholder(words.no_date.clone());
                if let Some(text) = field.show(ui).events.committed() {
                    posted.push(set("date", json!(text.trim())));
                }
                if Button::new(words.update.clone()).tooltip(words.update_tip.clone()).show(ui).clicked() {
                    posted.push(Msg::run("font.info.updateDate", json!({})));
                }
            });
        });
        rows::rows(ui, &font.fields, &target, &model.masters, &mut posted);
    });
    if plus {
        posted.push(open(InfoDialog::FieldPicker(target.clone())));
    }

    let plus =
        rows::section(ui, "axes", words.axes.clone(), words.add_axis.clone(), |ui| axes(ui, &font.axes, &font.predefined, &words, &mut posted));
    if plus {
        posted.push(Msg::run("font.axes.add", json!({})));
    }

    let plus = rows::section(ui, "parameters", words.parameters.clone(), words.add_parameter.clone(), |ui| {
        rows::rows(ui, &font.params, &target, &model.masters, &mut posted);
    });
    if plus {
        posted.push(open(InfoDialog::ParameterPicker(target)));
    }
    sent.extend(posted);
}

/// The axes in order, one row each: a drag, or the stacking keys on the focused list, reorders them.
fn axis_order(ui: &mut Ui, axes: &[AxisModel], words: &FontTabWords, sent: &mut Vec<Msg>) {
    let positions: Vec<usize> = (0..axes.len()).collect();
    let name = |at: usize| axes.get(at).map_or_else(String::new, |a| format!("{} ({})", a.name, a.tag));
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(name(*at)));
    let out = List::<usize>::new(words.axis_order.clone()).header(false).natural_height(true).reorder(ReorderRule::new()).show_items(
        ui,
        &source,
        &mut |at, icx| ListRow::new(name(*at)).state(icx.row.clone()),
    );
    for event in &out.events {
        let CollectionEvent::Reorder(edit) = event else { continue };
        let moved = match edit.phase() {
            Some(GesturePhase::Commit) | None => edit.value(),
            _ => None,
        };
        let Some(moved) = moved else { continue };
        let mut order: Vec<usize> = (0..axes.len()).filter(|i| !moved.keys.contains(i)).collect();
        let tail = order.split_off(moved.to.min(order.len()));
        order.extend(moved.keys.iter().copied());
        order.extend(tail);
        if order.iter().enumerate().any(|(at, old)| at != *old) {
            sent.push(Msg::run("font.axes.reorder", json!({"order": order})));
        }
    }
}

/// The axes: their order list when there are several, then per axis the name, the predefined-axis picker, the tag,
/// Hidden and minus, then its warnings. Every control addresses its axis by index, so two axes of one tag each edit their own.
fn axes(ui: &mut Ui, axes: &[AxisModel], predefined: &[PredefinedAxis], words: &FontTabWords, sent: &mut Vec<Msg>) {
    if axes.is_empty() {
        Text::new(words.no_axes.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let mut entries =
        vec![TreeEntry::folder(REGISTERED.to_string(), words.registered.clone()), TreeEntry::folder(CUSTOM.to_string(), words.custom.clone())];
    entries.extend(predefined.iter().map(|p| {
        let folder = if p.registered { REGISTERED } else { CUSTOM };
        TreeEntry::new(p.tag.clone(), format!("{} ({})", p.name, p.tag)).within(folder.to_string())
    }));
    if axes.len() > 1 {
        axis_order(ui, axes, words, sent);
    }
    for (at, axis) in axes.iter().enumerate() {
        let set = |key: &str, value: Json| Msg::run("font.axes.set", json!({"axis": at, key: value}));
        Stack::new().id_salt(("axis", at)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                if let Some(name) = TextField::new(words.axis_name.clone(), Bind::given(axis.name.clone())).show(ui).events.committed() {
                    sent.push(set("name", json!(name.trim())));
                }
                let current = predefined.iter().any(|p| p.tag == axis.tag).then(|| axis.tag.clone());
                let out = TreePicker::new(words.predefined.clone(), Bind::given(current), &entries).show(ui);
                if let Some(Some(tag)) = out.events.committed()
                    && let Some(p) = predefined.iter().find(|p| p.tag == *tag)
                {
                    sent.push(set("name", json!(p.name)));
                }
                if let Some(tag) = TextField::new(words.axis_tag.clone(), Bind::given(axis.tag.clone())).show(ui).events.committed() {
                    sent.push(set("tag", json!(tag.trim())));
                }
                let hidden = Checkbox::new(Bind::given(axis.hidden)).label(words.hidden.clone()).tooltip(words.hidden_tip.clone()).show(ui);
                if let Some(on) = hidden.events.committed() {
                    sent.push(set("hidden", json!(on)));
                }
                if IconButton::new(IconId::new("minus")).label(words.remove_axis.clone()).tooltip(words.remove_axis.clone()).show(ui).clicked() {
                    sent.push(Msg::run("font.axes.remove", json!({"axis": at})));
                }
            });
            for warning in &axis.warnings {
                Text::new(warning.clone()).shade(Shade::Muted).show(ui);
            }
        });
    }
}

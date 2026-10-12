//! The `STAT` table (M9.7).
//!
//! Rows: `stat.purpose`, `stat.elidableNames`, `stat.automaticFromInstances`,
//! `stat.manualOverrides`, `compile.statDefaults`, the `STAT` table row, parameters
//! `Export STAT Table`, `Elidable STAT Axis Value Name`, `Style Name as STAT entry`,
//! `Italic Style Linking`.
//!
//! Contract: [`stat_input`] derives the design axes and axis values as fontc's
//! `glyphs2fontir/src/stat.rs` does (Apache-2.0; ported with credit in this file's header and in
//! `NOTICE`): from the axes' labels when the font states them, else from the instances' names and
//! locations, with the elidable names and the italic linking the parameters set. [`build_stat`]
//! writes it with the shared name allocator. Axis value format 4 is never written. No table when
//! every variable font setting turns Export STAT Table off.
//!
//! Ported from fontc at commit 4504578c3f74c2739f7e55cae133bbfe773f6d8c (Apache-2.0, see
//! `NOTICE`): the derivation from instances (`glyphs2fontir/src/stat.rs`, which follows
//! glyphsLib's `builder/stat.py`), the fallback to the variable axes without values and the order
//! names are registered in (`fontir/src/ir/static_metadata.rs`), and the choice of axis value
//! format (`fontbe/src/stat.rs`). Axis labels a designspace states are read as fontTools'
//! `varLib.stat.getStatAxes` reads them (MIT).
//!
//! Where this differs from fontc:
//! - The table is built per variable font setting, and a setting whose Export STAT Table is off
//!   gets no table (fontc writes the axes without values when every setting turns it off).
//! - Italic Style Linking forces or removes the link on the `ital` axis (fontc ignores it).

use fontcraft_interp::Designspace;
use fontcraft_interp::error::{MAX_AXES, MAX_INSTANCES};
use fontcraft_model::axes::{Mapping, design_value, mapping, range};
use fontcraft_model::{Font, Instance, Params, param_flag};
use write_fonts::tables::stat::{AxisRecord, AxisValue, AxisValueTableFlags, Stat};
use write_fonts::types::{Fixed, NameId, Tag};

use super::names::NameAllocator;
use crate::CompileError;

pub const EXPORT_STAT_TABLE: &str = "Export STAT Table";
pub const ELIDABLE_STAT_AXIS_VALUE_NAME: &str = "Elidable STAT Axis Value Name";
pub const STYLE_NAME_AS_STAT_ENTRY: &str = "Style Name as STAT entry";
pub const ITALIC_STYLE_LINKING: &str = "Italic Style Linking";

/// The elided fallback name of a derived table (glyphsLib's fixed choice).
const DERIVED_ELIDED_FALLBACK: &str = "Regular";

/// Most axis values, and most design axes, one table holds (the counts are 16-bit).
const MAX_STAT_ENTRIES: usize = u16::MAX as usize;

const ITAL: Tag = Tag::new(b"ital");
const SLNT: Tag = Tag::new(b"slnt");
const WDTH: Tag = Tag::new(b"wdth");
const WGHT: Tag = Tag::new(b"wght");

/// One axis value: a name for a value, a range of values, or a value linked to another.
#[derive(Clone, Debug, PartialEq)]
pub struct StatValue {
    pub name: String,
    /// External coordinate.
    pub value: f64,
    /// `(min, max)` for a range value (format 2).
    pub range: Option<(f64, f64)>,
    /// The value it links to (format 3).
    pub linked_value: Option<f64>,
    pub elidable: bool,
    pub older_sibling: bool,
}

/// One design axis and its values.
#[derive(Clone, Debug, PartialEq)]
pub struct StatAxis {
    pub tag: Tag,
    pub name: String,
    pub ordering: u16,
    pub values: Vec<StatValue>,
}

/// What `STAT` holds, before names get IDs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatInput {
    pub axes: Vec<StatAxis>,
    /// The name used when every axis value is elided (name ID 2 when `None`).
    pub elided_fallback_name: Option<String>,
}

/// An axis of the font in user coordinates, with the map that takes design coordinates there.
#[derive(Clone, Debug)]
struct SourceAxis {
    tag: Tag,
    name: String,
    min: f64,
    default: f64,
    max: f64,
    map: Mapping,
}

/// What the derivation reads of an instance.
#[derive(Clone, Debug)]
struct StatInstance {
    name: String,
    active: bool,
    /// A variable font setting; otherwise a static instance.
    variable: bool,
    /// User coordinates, parallel to the source axes.
    user_locations: Vec<Option<f64>>,
    is_bold: bool,
    is_italic: bool,
    export_stat_table: Option<bool>,
    /// Axis tags of the Elidable STAT Axis Value Name parameters.
    elidable: Vec<String>,
    /// Axis tags of the Style Name as STAT entry parameters.
    manual: Vec<String>,
}

impl StatInstance {
    /// Named just "Italic" or "Regular Italic": the regular style of an italic family.
    fn is_regular_italic(&self) -> bool {
        matches!(self.name.trim().to_lowercase().as_str(), "italic" | "regular italic")
    }
}

/// The `STAT` contents of the variable font of `designspace`; `None` when the font turns the
/// table off.
pub fn stat_input(font: &Font, designspace: &Designspace) -> Result<Option<StatInput>, CompileError> {
    let setting = designspace.setting().and_then(|s| font.instances.get(s));
    if setting_flag(font, setting, EXPORT_STAT_TABLE) == Some(false) {
        return Ok(None);
    }
    let axes = source_axes(font, designspace)?;
    if font.axes.iter().any(|a| !a.labels.is_empty()) {
        return Ok(Some(from_axis_labels(font, &axes)));
    }
    let instances: Vec<StatInstance> = font.instances.iter().take(MAX_INSTANCES).map(|inst| stat_instance(inst, &axes)).collect();
    let Some(mut stat_axes) = derive_stat_axes(&instances, &axes) else {
        // Nothing to derive from: the variable axes without values.
        let axes = designspace
            .axes()
            .iter()
            .enumerate()
            .map(|(i, a)| Ok(StatAxis { tag: tag_of(&a.tag)?, name: a.name.clone(), ordering: ordering(i), values: Vec::new() }))
            .collect::<Result<Vec<_>, CompileError>>()?;
        return Ok(Some(StatInput { axes, elided_fallback_name: None }));
    };
    // Labels outside the variable font's user region are dropped, as fontTools does.
    for (stat_axis, axis) in stat_axes.iter_mut().zip(&axes) {
        stat_axis.values.retain(|v| (axis.min..=axis.max).contains(&v.value));
    }
    italic_style_linking(&mut stat_axes, setting_flag(font, setting, ITALIC_STYLE_LINKING));
    Ok(Some(StatInput { axes: stat_axes, elided_fallback_name: Some(DERIVED_ELIDED_FALLBACK.to_string()) }))
}

/// The `STAT` table of `input`.
pub fn build_stat(input: &StatInput, names: &mut NameAllocator) -> Result<Stat, CompileError> {
    let count: usize = input.axes.iter().map(|a| a.values.len()).sum();
    if count > MAX_STAT_ENTRIES || input.axes.len() > MAX_STAT_ENTRIES {
        return Err(table_error(format!("{count} axis values on {} axes; a table holds at most 65,535 of each", input.axes.len())));
    }
    // Names are registered in fontc's order: the elided fallback, then each axis and its values.
    let elided = match &input.elided_fallback_name {
        Some(name) if !input.axes.is_empty() => names.id_for(name)?,
        _ => NameId::SUBFAMILY_NAME,
    };
    let mut design_axes = Vec::with_capacity(input.axes.len());
    let mut values = Vec::with_capacity(count);
    for (index, axis) in input.axes.iter().enumerate() {
        let axis_index = u16::try_from(index).map_err(|_| table_error("too many design axes".to_string()))?;
        design_axes.push(AxisRecord::new(axis.tag, names.id_for(&axis.name)?, axis.ordering));
        for v in &axis.values {
            let name_id = names.id_for(&v.name)?;
            let mut flags = AxisValueTableFlags::empty();
            if v.elidable {
                flags |= AxisValueTableFlags::ELIDABLE_AXIS_VALUE_NAME;
            }
            if v.older_sibling {
                flags |= AxisValueTableFlags::OLDER_SIBLING_FONT_ATTRIBUTE;
            }
            let value = fixed(v.value);
            values.push(match (v.linked_value, v.range) {
                (Some(linked), _) => AxisValue::format_3(axis_index, flags, name_id, value, fixed(linked)),
                (None, Some((min, max))) => AxisValue::format_2(axis_index, flags, name_id, value, fixed(min), fixed(max)),
                (None, None) => AxisValue::format_1(axis_index, flags, name_id, value),
            });
        }
    }
    Ok(Stat::new(design_axes, values, elided))
}

fn table_error(message: String) -> CompileError {
    CompileError::Table { table: "STAT", message }
}

/// A value as `Fixed`, clamped to its range.
fn fixed(v: f64) -> Fixed {
    if v.is_finite() { Fixed::from_f64(v.clamp(Fixed::MIN.to_f64(), Fixed::MAX.to_f64())) } else { Fixed::from_f64(0.0) }
}

fn ordering(i: usize) -> u16 {
    u16::try_from(i).unwrap_or(u16::MAX)
}

/// An axis tag as written: its first four characters.
fn tag_of(text: &str) -> Result<Tag, CompileError> {
    let bytes = text.as_bytes();
    Tag::new_checked(bytes.get(..4).unwrap_or(bytes)).map_err(|e| table_error(format!("axis tag `{text}`: {e}")))
}

/// A yes-or-no parameter of the variable font setting, else of the font.
fn setting_flag(font: &Font, setting: Option<&Instance>, name: &str) -> Option<bool> {
    let read = |params: &Params| params.get(name).and_then(param_flag);
    setting.and_then(|s| read(&s.params)).or_else(|| read(&font.info.params))
}

/// Every axis of the font in user coordinates. An axis the designspace has takes its range and
/// map from there; one a variable font setting dropped sits at the origin's value.
fn source_axes(font: &Font, designspace: &Designspace) -> Result<Vec<SourceAxis>, CompileError> {
    let origin = designspace.origin().and_then(|m| font.masters.get(m.index)).or_else(|| fontcraft_model::axes::origin(font));
    font.axes
        .iter()
        .take(MAX_AXES)
        .enumerate()
        .map(|(i, axis)| {
            let (map, min, default, max) = match designspace.axes().iter().find(|a| a.font_axis == i) {
                Some(info) => (info.map.clone(), info.external.minimum, info.external.default, info.external.maximum),
                None if designspace.setting().is_some() => {
                    let map = mapping(font, i);
                    let at = origin.map(|m| map.to_user(design_value(&m.location, i))).filter(|v| v.is_finite()).unwrap_or(map.default);
                    (map, at, at, at)
                }
                None => {
                    let r = range(font, i);
                    (mapping(font, i), r.minimum, r.default, r.maximum)
                }
            };
            Ok(SourceAxis { tag: tag_of(&axis.tag)?, name: axis.name.clone(), min, default, max, map })
        })
        .collect()
}

fn stat_instance(inst: &Instance, axes: &[SourceAxis]) -> StatInstance {
    let tags = |name: &str| inst.params.all(name).filter_map(|v| v.as_str()).map(str::to_string).collect::<Vec<_>>();
    StatInstance {
        name: inst.name.clone(),
        active: inst.exports,
        variable: inst.kind.as_deref() == Some("variable"),
        // Adding 0.0 makes −0 and 0 one value.
        user_locations: axes
            .iter()
            .enumerate()
            .map(|(i, a)| Some(a.map.to_user(design_value(&inst.location, i)) + 0.0).filter(|v| v.is_finite()))
            .collect(),
        is_bold: inst.is_bold,
        is_italic: inst.is_italic,
        export_stat_table: inst.params.get(EXPORT_STAT_TABLE).and_then(param_flag),
        elidable: tags(ELIDABLE_STAT_AXIS_VALUE_NAME),
        manual: tags(STYLE_NAME_AS_STAT_ENTRY),
    }
}

/// The values a designspace states with its axis labels (fontTools' `getStatAxes`): an axis
/// without an ordering takes the next after the highest stated one.
fn from_axis_labels(font: &Font, axes: &[SourceAxis]) -> StatInput {
    let mut next = font.axes.iter().filter_map(|a| a.ordering).max().map_or(0, |o| o.saturating_add(1));
    let stat_axes = font
        .axes
        .iter()
        .zip(axes)
        .map(|(axis, source)| {
            let ordering = axis.ordering.unwrap_or_else(|| {
                let o = next;
                next = next.saturating_add(1);
                o
            });
            let values = axis
                .labels
                .iter()
                .filter(|l| l.user_value.is_finite() && (source.min..=source.max).contains(&l.user_value))
                .take(MAX_STAT_ENTRIES)
                .map(|l| StatValue {
                    name: l.name.clone(),
                    value: l.user_value,
                    range: (l.user_minimum.is_some() || l.user_maximum.is_some())
                        .then(|| (l.user_minimum.unwrap_or(Fixed::MIN.to_f64()), l.user_maximum.unwrap_or(Fixed::MAX.to_f64()))),
                    linked_value: l.linked_user_value,
                    elidable: l.elidable,
                    older_sibling: l.older_sibling,
                })
                .collect();
            StatAxis { tag: source.tag, name: source.name.clone(), ordering: u16::try_from(ordering).unwrap_or(u16::MAX), values }
        })
        .collect();
    StatInput { axes: stat_axes, elided_fallback_name: None }
}

/// The Italic Style Linking parameter: on links the upright value of the `ital` axis to 1, off
/// removes every link on that axis.
fn italic_style_linking(stat_axes: &mut [StatAxis], linking: Option<bool>) {
    let Some(on) = linking else { return };
    for value in stat_axes.iter_mut().filter(|a| a.tag == ITAL).flat_map(|a| a.values.iter_mut()) {
        if !on {
            value.linked_value = None;
        } else if value.value == 0.0 && value.linked_value.is_none() && value.range.is_none() {
            value.linked_value = Some(1.0);
        }
    }
}

/// fontc's `derive_stat_axes`: the source axes with values from the instances, plus a STAT-only
/// `ital` axis when the font has none. `None` when no active variable font setting asks for the
/// table.
fn derive_stat_axes(instances: &[StatInstance], axes: &[SourceAxis]) -> Option<Vec<StatAxis>> {
    let settings: Vec<&StatInstance> = instances.iter().filter(|i| i.variable && i.active).collect();
    if settings.is_empty() || settings.iter().all(|i| i.export_stat_table == Some(false)) {
        return None;
    }
    let instances: Vec<&StatInstance> = instances.iter().filter(|i| !i.variable && i.active).collect();
    let default_instance = instances.iter().copied().find(|i| at_default(i, axes, None));
    let italic = default_instance.is_some_and(is_italic);
    let has = |tag: Tag| axes.iter().any(|a| a.tag == tag);
    // An instance called just "Italic" or "Regular Italic" is the regular style of an italic
    // family, unless a slope axis gives the word a meaning of its own.
    let regular = if has(ITAL) || has(SLNT) { None } else { instances.iter().copied().find(|i| i.is_regular_italic()) };
    let manual = instances.iter().any(|i| !i.manual.is_empty());

    let mut stat_axes = if manual { manual_labels(axes, &instances, regular) } else { automatic_labels(axes, &instances, default_instance, regular) };
    link_bold(&mut stat_axes, axes, &instances);
    if !has(ITAL) {
        stat_axes.push(synthetic_italic_axis(italic, ordering(axes.len())));
    }
    Some(stat_axes)
}

fn user_loc(instance: &StatInstance, axis: usize) -> Option<f64> {
    instance.user_locations.get(axis).copied().flatten()
}

fn at_default(instance: &StatInstance, axes: &[SourceAxis], skip: Option<usize>) -> bool {
    axes.iter().enumerate().all(|(i, axis)| skip == Some(i) || user_loc(instance, i).is_none_or(|loc| loc == axis.default))
}

fn is_italic(instance: &StatInstance) -> bool {
    instance.is_italic || instance.name.to_lowercase().contains("italic")
}

fn contains_tag(values: &[String], tag: Tag) -> bool {
    values.iter().any(|v| v.as_bytes() == tag.to_be_bytes())
}

fn is_elidable(instance: &StatInstance, axis: &SourceAxis) -> bool {
    contains_tag(&instance.elidable, axis.tag)
}

fn default_name(tag: Tag) -> &'static str {
    if tag == WDTH { "Normal" } else { "Regular" }
}

fn value(name: String, value: f64, elidable: bool) -> StatValue {
    StatValue { name, value, range: None, linked_value: None, elidable, older_sibling: false }
}

fn stat_axis(axis: &SourceAxis, index: usize, values: Vec<StatValue>) -> StatAxis {
    StatAxis { tag: axis.tag, name: axis.name.clone(), ordering: ordering(index), values }
}

/// The instances with a location on axis `index`, grouped by it in increasing order; the
/// instances of a group keep their order.
fn by_value<'a>(instances: &[&'a StatInstance], index: usize) -> Vec<(f64, Vec<&'a StatInstance>)> {
    let mut located: Vec<(f64, &StatInstance)> = instances.iter().filter_map(|i| Some((user_loc(i, index)?, *i))).collect();
    located.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut groups: Vec<(f64, Vec<&StatInstance>)> = Vec::new();
    for (loc, instance) in located {
        match groups.last_mut() {
            Some((v, group)) if *v == loc => group.push(instance),
            _ => groups.push((loc, vec![instance])),
        }
    }
    groups
}

/// Style Name as STAT entry on any instance: only the instances that list an axis label it, the
/// first one at a value winning.
fn manual_labels(axes: &[SourceAxis], instances: &[&StatInstance], regular: Option<&StatInstance>) -> Vec<StatAxis> {
    axes.iter()
        .enumerate()
        .map(|(index, axis)| {
            let listing: Vec<&StatInstance> = instances.iter().copied().filter(|i| contains_tag(&i.manual, axis.tag)).collect();
            let values = by_value(&listing, index)
                .into_iter()
                .filter_map(|(loc, group)| {
                    let instance = group.first()?;
                    // The italic word goes here too, but an entry it would leave empty keeps the
                    // instance's own name.
                    let name = regular.map(|_| strip_italic(&instance.name)).filter(|n| !n.is_empty()).unwrap_or_else(|| instance.name.clone());
                    Some(value(name, loc, is_elidable(instance, axis)))
                })
                .collect();
            stat_axis(axis, index, values)
        })
        .collect()
}

fn automatic_labels(
    axes: &[SourceAxis],
    instances: &[&StatInstance],
    default_instance: Option<&StatInstance>,
    regular: Option<&StatInstance>,
) -> Vec<StatAxis> {
    // The first axis the instances vary on gets the default instance's whole name at its
    // default, so the name cannot repeat when names are composed across axes.
    let first_varying = (0..axes.len()).find(|&index| {
        let mut locs = instances.iter().filter_map(|i| user_loc(i, index));
        locs.next().is_some_and(|first| locs.any(|loc| loc != first))
    });
    let mut stat_axes: Vec<StatAxis> = axes
        .iter()
        .enumerate()
        .map(|(index, axis)| {
            let default = default_name(axis.tag);
            let values = by_value(instances, index)
                .into_iter()
                .filter_map(|(loc, group)| {
                    let representative = if loc == axis.default {
                        match default_instance {
                            Some(instance) if first_varying == Some(index) => instance,
                            _ => return Some(value(default.to_string(), loc, true)),
                        }
                    } else {
                        // Prefer the instance at the default on every other axis (Bold over
                        // Bold Condensed), else the first one.
                        group.iter().copied().find(|i| at_default(i, axes, Some(index))).or_else(|| group.first().copied())?
                    };
                    let name = label_name(representative, default, regular);
                    let elidable = name == default || is_elidable(representative, axis);
                    Some(value(name, loc, elidable))
                })
                .collect();
            stat_axis(axis, index, values)
        })
        .collect();

    // A real ital axis links the upright default to its highest value; slnt is not linked. An
    // axis whose only value is the default links nowhere.
    for (stat_axis, axis) in stat_axes.iter_mut().zip(axes) {
        if axis.tag != ITAL {
            continue;
        }
        let highest = stat_axis.values.iter().map(|v| v.value).max_by(f64::total_cmp);
        if let Some(linked) = highest.filter(|h| *h != axis.default)
            && let Some(v) = stat_axis.values.iter_mut().find(|v| v.value == axis.default)
        {
            v.linked_value = Some(linked);
        }
    }
    stat_axes
}

/// The elidable weight links to the weight of the instance flagged bold, wherever either sits on
/// the other axes; in manual mode too. A bold instance at the elidable weight links nowhere.
fn link_bold(stat_axes: &mut [StatAxis], axes: &[SourceAxis], instances: &[&StatInstance]) {
    let Some(index) = axes.iter().position(|a| a.tag == WGHT) else { return };
    let Some(bold) = instances.iter().find(|i| i.is_bold).and_then(|i| user_loc(i, index)) else { return };
    if let Some(v) = stat_axes.get_mut(index).and_then(|a| a.values.iter_mut().find(|v| v.elidable))
        && v.value != bold
    {
        v.linked_value = Some(bold);
    }
}

/// The STAT-only `ital` axis: Italic at 1 for an italic family, else an elidable Roman at 0
/// linked to 1.
fn synthetic_italic_axis(italic: bool, ordering: u16) -> StatAxis {
    let mut label = if italic { value("Italic".to_string(), 1.0, false) } else { value("Roman".to_string(), 0.0, true) };
    label.linked_value = (!italic).then_some(1.0);
    StatAxis { tag: ITAL, name: "Italic".to_string(), ordering, values: vec![label] }
}

fn label_name(instance: &StatInstance, default: &str, regular: Option<&StatInstance>) -> String {
    let name = match regular {
        // The regular instance's value takes the axis default name, whatever its own name says.
        Some(regular) if std::ptr::eq(instance, regular) => String::new(),
        Some(_) => strip_italic(&instance.name),
        None => instance.name.clone(),
    };
    if name.is_empty() { default.to_string() } else { name }
}

/// "Italic" taken out of a style name as glyphsLib does it:
/// `re.sub(r"\s*italic\s*", " ", name, flags=re.IGNORECASE).strip()`, with ASCII case and
/// whitespace. The word is not bounded: "fooitalicbar" gives "foo bar".
fn strip_italic(name: &str) -> String {
    const WORD: &str = "italic";
    let lower = name.to_ascii_lowercase();
    let bytes = name.as_bytes();
    let space = |at: usize| bytes.get(at).is_some_and(|b| b.is_ascii_whitespace() || *b == 0x0b);
    let mut out = String::with_capacity(name.len());
    let mut from = 0;
    while let Some(found) = lower.get(from..).and_then(|rest| rest.find(WORD)) {
        let start = from + found;
        let mut cut = start;
        while cut > from && space(cut - 1) {
            cut -= 1;
        }
        out.push_str(name.get(from..cut).unwrap_or(""));
        out.push(' ');
        let mut end = start + WORD.len();
        while space(end) {
            end += 1;
        }
        from = end;
    }
    out.push_str(name.get(from..).unwrap_or(""));
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use fontcraft_model::plist::{Dict, Number, Value};
    use fontcraft_model::{Axis, AxisLabel, Master, Param};
    use std::sync::Arc;

    use skrifa::raw::tables::stat::AxisValue as ReadAxisValue;
    use skrifa::raw::{FontData, FontRead};

    use super::*;

    fn single(name: &str, coords: &[f64]) -> StatInstance {
        StatInstance {
            name: name.to_string(),
            active: true,
            variable: false,
            user_locations: coords.iter().copied().map(Some).collect(),
            is_bold: false,
            is_italic: false,
            export_stat_table: None,
            elidable: Vec::new(),
            manual: Vec::new(),
        }
    }

    fn setting(export_stat_table: Option<bool>) -> StatInstance {
        StatInstance { variable: true, export_stat_table, ..single("VF", &[]) }
    }

    fn axis(tag: &str, name: &str, min: f64, default: f64, max: f64) -> SourceAxis {
        let tag = tag_of(tag).unwrap();
        SourceAxis { tag, name: name.to_string(), min, default, max, map: Mapping::new(vec![(min, min), (max, max)], default) }
    }

    fn stat(axes: Vec<SourceAxis>, mut instances: Vec<StatInstance>) -> Vec<StatAxis> {
        instances.push(setting(None));
        derive_stat_axes(&instances, &axes).unwrap()
    }

    /// The (name, value, elidable, linked value) labels on the axis `tag`.
    fn assert_labels(stat: &[StatAxis], tag: &str, expected: &[(&str, f64, bool, Option<f64>)]) {
        let tag = tag_of(tag).unwrap();
        let axis = stat.iter().find(|a| a.tag == tag).unwrap();
        let actual: Vec<_> = axis.values.iter().map(|v| (v.name.as_str(), v.value, v.elidable, v.linked_value)).collect();
        assert_eq!(expected, actual, "labels on {tag}");
    }

    fn wght() -> SourceAxis {
        axis("wght", "Weight", 400.0, 400.0, 700.0)
    }

    #[test]
    fn weight_width_labels_and_stat_only_italic() {
        let stat = stat(
            vec![wght(), axis("wdth", "Width", 75.0, 100.0, 100.0)],
            vec![single("Regular", &[400.0, 100.0]), single("Bold", &[700.0, 100.0]), single("Condensed", &[400.0, 75.0])],
        );
        assert_labels(&stat, "wght", &[("Regular", 400.0, true, None), ("Bold", 700.0, false, None)]);
        assert_labels(&stat, "wdth", &[("Condensed", 75.0, false, None), ("Normal", 100.0, true, None)]);
        assert_labels(&stat, "ital", &[("Roman", 0.0, true, Some(1.0))]);
        assert_eq!(stat.iter().map(|a| a.ordering).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn style_linked_bold_links_the_default_weight() {
        for (is_bold, linked) in [(true, Some(700.0)), (false, None)] {
            let bold = StatInstance { is_bold, ..single("Bold", &[700.0]) };
            let stat = stat(vec![wght()], vec![single("Regular", &[400.0]), bold]);
            assert_labels(&stat, "wght", &[("Regular", 400.0, true, linked), ("Bold", 700.0, false, None)]);
        }
    }

    // Values come from the instances (Medium at 500), and a value shared by several instances is
    // named after the one at the default on every other axis: Bold, not Bold Compact.
    #[test]
    fn instance_values_and_representative_instance() {
        let stat = stat(
            vec![wght(), axis("SPAC", "Spacing", -100.0, 0.0, 0.0)],
            vec![
                single("Regular", &[400.0, 0.0]),
                single("Medium", &[500.0, 0.0]),
                single("Bold", &[700.0, 0.0]),
                single("Compact", &[400.0, -100.0]),
                single("Bold Compact", &[700.0, -100.0]),
            ],
        );
        assert_labels(&stat, "wght", &[("Regular", 400.0, true, None), ("Medium", 500.0, false, None), ("Bold", 700.0, false, None)]);
        assert_labels(&stat, "SPAC", &[("Compact", -100.0, false, None), ("Regular", 0.0, true, None)]);
    }

    // No instance leaves wght 400, so the default instance's name Book goes on wdth.
    #[test]
    fn default_instance_names_the_first_axis_the_instances_vary_on() {
        let stat =
            stat(vec![wght(), axis("wdth", "Width", 75.0, 100.0, 100.0)], vec![single("Book", &[400.0, 100.0]), single("Condensed", &[400.0, 75.0])]);
        assert_labels(&stat, "wght", &[("Regular", 400.0, true, None)]);
        assert_labels(&stat, "wdth", &[("Condensed", 75.0, false, None), ("Book", 100.0, false, None)]);
    }

    #[test]
    fn export_stat_table_and_settings_decide_whether_there_is_a_table() {
        let axes = vec![wght()];
        let regular = single("Regular", &[400.0]);
        assert!(derive_stat_axes(std::slice::from_ref(&regular), &axes).is_none(), "no setting");
        let off = [regular.clone(), setting(Some(false)), setting(Some(false))];
        assert!(derive_stat_axes(&off, &axes).is_none(), "every setting off");
        let inactive = StatInstance { active: false, ..setting(None) };
        assert!(derive_stat_axes(&[regular.clone(), inactive, setting(Some(false))], &axes).is_none(), "inactive setting");
        assert!(derive_stat_axes(&[regular, setting(Some(false)), setting(None)], &axes).is_some(), "one setting on");
    }

    #[test]
    fn elidable_parameter_and_inactive_instances() {
        let bold = StatInstance { elidable: vec!["wght".into()], ..single("Bold", &[700.0]) };
        let ignored = StatInstance { active: false, ..single("Ignored", &[600.0]) };
        let stat = stat(vec![wght()], vec![single("Regular", &[400.0]), ignored, bold]);
        assert_labels(&stat, "wght", &[("Regular", 400.0, true, None), ("Bold", 700.0, true, None)]);
    }

    // One instance with the parameter switches the whole font to manual labels: wdth gets none
    // although Condensed sits at 75, and the first instance at a value wins.
    #[test]
    fn style_name_as_stat_entry_switches_to_manual_labels() {
        let bold = StatInstance { manual: vec!["wght".into()], ..single("Bold", &[700.0, 100.0]) };
        let second = StatInstance { manual: vec!["wght".into()], ..single("Second", &[700.0, 100.0]) };
        let stat = stat(
            vec![wght(), axis("wdth", "Width", 75.0, 100.0, 100.0)],
            vec![single("Regular", &[400.0, 100.0]), bold, second, single("Condensed", &[400.0, 75.0])],
        );
        assert_labels(&stat, "wght", &[("Bold", 700.0, false, None)]);
        assert_labels(&stat, "wdth", &[]);
        assert_labels(&stat, "ital", &[("Roman", 0.0, true, Some(1.0))]);
    }

    // An italic family with an ital axis: no STAT-only axis, and the upright links to the italic.
    #[test]
    fn real_italic_axis_links_upright_to_italic() {
        let stat = stat(
            vec![wght(), axis("ital", "Italic", 0.0, 0.0, 1.0)],
            vec![
                single("Regular", &[400.0, 0.0]),
                single("Bold", &[700.0, 0.0]),
                single("Italic", &[400.0, 1.0]),
                single("Bold Italic", &[700.0, 1.0]),
            ],
        );
        assert_eq!(stat.iter().filter(|a| a.tag == ITAL).count(), 1);
        assert_labels(&stat, "ital", &[("Regular", 0.0, true, Some(1.0)), ("Italic", 1.0, false, None)]);
    }

    // An italic family without an ital axis: the STAT-only axis says Italic at 1 and the names
    // lose the word when the default is called just Italic, wherever it sits.
    #[test]
    fn italic_family_without_ital_axis() {
        for regular_name in ["Italic", "Regular Italic"] {
            let it = |name: &str, w: f64| StatInstance { is_italic: true, ..single(name, &[w]) };
            let bold = StatInstance { is_bold: true, ..it("Bold Italic", 700.0) };
            let stat = stat(
                vec![axis("wght", "Weight", 100.0, 100.0, 900.0)],
                vec![it("Thin Italic", 100.0), it(regular_name, 400.0), bold, it("Black Italic", 900.0)],
            );
            assert_labels(
                &stat,
                "wght",
                &[("Thin", 100.0, false, None), ("Regular", 400.0, true, Some(700.0)), ("Bold", 700.0, false, None), ("Black", 900.0, false, None)],
            );
            assert_labels(&stat, "ital", &[("Italic", 1.0, false, None)]);
        }
        // Any other spelling keeps every name whole.
        let it = |name: &str, w: f64| StatInstance { is_italic: true, ..single(name, &[w]) };
        let stat = stat(vec![wght()], vec![it("Book Italic", 400.0), it("Bold Italic", 700.0)]);
        assert_labels(&stat, "wght", &[("Book Italic", 400.0, false, None), ("Bold Italic", 700.0, false, None)]);
        assert_labels(&stat, "ital", &[("Italic", 1.0, false, None)]);
    }

    #[test]
    fn italic_style_linking_forces_or_removes_the_link() {
        let mut axes = stat(vec![wght()], vec![single("Regular", &[400.0])]);
        italic_style_linking(&mut axes, Some(false));
        assert_labels(&axes, "ital", &[("Roman", 0.0, true, None)]);
        italic_style_linking(&mut axes, Some(true));
        assert_labels(&axes, "ital", &[("Roman", 0.0, true, Some(1.0))]);
    }

    #[test]
    fn italic_word_removal_matches_the_oracle() {
        assert_eq!(strip_italic("  Bold   ITALIC  "), "Bold");
        assert_eq!(strip_italic("fooitalicbar"), "foo bar");
        assert_eq!(strip_italic("Italic Italic"), "");
        assert_eq!(strip_italic("Bold Italic Condensed"), "Bold Condensed");
    }

    fn instance(name: &str, weight: f64) -> Instance {
        Instance { name: name.to_string(), location: vec![weight], ..Instance::default() }
    }

    /// fontc's `StatLabels` and `StatAxisMappings` sources, rebuilt.
    fn source(mappings: bool) -> Font {
        let mut font = Font::new("Test");
        font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
        font.masters = Arc::new(vec![
            Master { id: "m01".into(), name: "Regular".into(), location: vec![400.0], ..Master::default() },
            Master { id: "m02".into(), name: "Bold".into(), location: vec![700.0], ..Master::default() },
        ]);
        let bold = Instance { is_bold: true, ..instance("Bold", 700.0) };
        let third = if mappings { instance("Medium", 550.0) } else { instance("Black", 900.0) };
        let vf = Instance { name: "VF".into(), kind: Some("variable".into()), ..Instance::default() };
        font.instances = Arc::new(vec![instance("Regular", 400.0), bold, third, vf]);
        if mappings {
            let mut map = Dict::new();
            map.insert("300", Value::Number(Number::int(400)));
            map.insert("800", Value::Number(Number::int(700)));
            let mut param = Dict::new();
            param.insert("wght", Value::Dict(map));
            Arc::make_mut(&mut font.info).params.0.push(Param::new("Axis Mappings", Value::Dict(param)));
        }
        font
    }

    fn derived(font: &Font, setting: Option<usize>) -> Option<StatInput> {
        let designspace = match setting {
            Some(s) => Designspace::for_setting(font, s),
            None => Designspace::new(font),
        }
        .unwrap();
        stat_input(font, &designspace).unwrap()
    }

    // Black at 900 lies outside the variable font and gets no value.
    #[test]
    fn stat_labels_source() {
        let input = derived(&source(false), Some(3)).unwrap();
        assert_labels(&input.axes, "wght", &[("Regular", 400.0, true, Some(700.0)), ("Bold", 700.0, false, None)]);
        assert_labels(&input.axes, "ital", &[("Roman", 0.0, true, Some(1.0))]);
        assert_eq!(input.elided_fallback_name.as_deref(), Some("Regular"));
    }

    // The values sit at the mapped user locations (300, 550, 800), not at the weight classes.
    #[test]
    fn stat_axis_mappings_source() {
        let input = derived(&source(true), Some(3)).unwrap();
        assert_labels(&input.axes, "wght", &[("Regular", 300.0, true, Some(800.0)), ("Medium", 550.0, false, None), ("Bold", 800.0, false, None)]);
    }

    #[test]
    fn export_stat_table_off_on_the_setting_means_no_table() {
        let mut font = source(false);
        if let Some(vf) = Arc::make_mut(&mut font.instances).get_mut(3) {
            vf.params.0.push(Param::new(EXPORT_STAT_TABLE, Value::Number(Number::int(0))));
        }
        assert_eq!(derived(&font, Some(3)), None);
    }

    // Without a variable font setting the table has the variable axes and no values.
    #[test]
    fn without_a_setting_the_table_has_axes_only() {
        let mut font = source(false);
        Arc::make_mut(&mut font.instances).truncate(3);
        let input = derived(&font, None).unwrap();
        assert_eq!(input.axes.iter().map(|a| (a.tag, a.values.len())).collect::<Vec<_>>(), [(WGHT, 0)]);
        assert_eq!(input.elided_fallback_name, None);
    }

    #[test]
    fn axis_labels_of_a_designspace_are_used_as_stated() {
        let mut font = source(false);
        let label = |name: &str, v: f64| AxisLabel { name: name.into(), user_value: v, ..AxisLabel::default() };
        if let Some(a) = Arc::make_mut(&mut font.axes).get_mut(0) {
            a.labels = vec![
                AxisLabel { elidable: true, linked_user_value: Some(700.0), ..label("Regular", 400.0) },
                AxisLabel { user_minimum: Some(650.0), ..label("Bold", 700.0) },
                label("Black", 900.0),
            ];
        }
        let input = derived(&font, Some(3)).unwrap();
        assert_eq!(input.axes.len(), 1, "no STAT-only axis");
        let values = &input.axes.first().unwrap().values;
        assert_eq!(values.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["Regular", "Bold"]);
        assert_eq!(values.get(1).unwrap().range, Some((650.0, Fixed::MAX.to_f64())));
    }

    #[test]
    fn the_table_has_one_format_per_kind_of_value() {
        let input = StatInput {
            axes: vec![
                StatAxis {
                    tag: WGHT,
                    name: "Weight".into(),
                    ordering: 0,
                    values: vec![
                        StatValue { linked_value: Some(700.0), ..value("Regular".into(), 400.0, true) },
                        StatValue { range: Some((650.0, 750.0)), ..value("Bold".into(), 700.0, false) },
                    ],
                },
                StatAxis { tag: ITAL, name: "Italic".into(), ordering: 1, values: vec![value("Roman".into(), 0.0, true)] },
            ],
            elided_fallback_name: Some("Regular".into()),
        };
        let mut names = NameAllocator::new();
        let table = build_stat(&input, &mut names).unwrap();
        let bytes = write_fonts::dump_table(&table).unwrap();
        let read = skrifa::raw::tables::stat::Stat::read(FontData::new(&bytes)).unwrap();
        let name = |id: NameId| names.records().iter().find(|(n, _)| *n == id).map(|(_, t)| t.clone()).unwrap();
        assert_eq!(name(read.elided_fallback_name_id().unwrap()), "Regular");
        let axes = read.design_axes().unwrap();
        assert_eq!(axes.iter().map(|a| (a.axis_tag(), a.axis_ordering())).collect::<Vec<_>>(), [(WGHT, 0), (ITAL, 1)]);
        let values = read.offset_to_axis_values().unwrap().unwrap();
        let values: Vec<_> = values.axis_values().iter().map(|v| v.unwrap()).collect();
        let [ReadAxisValue::Format3(regular), ReadAxisValue::Format2(bold), ReadAxisValue::Format1(roman)] = values.as_slice() else {
            panic!("formats 3, 2, 1 expected");
        };
        assert_eq!((name(regular.value_name_id()), regular.linked_value().to_f64()), ("Regular".to_string(), 700.0));
        assert!(regular.flags().contains(AxisValueTableFlags::ELIDABLE_AXIS_VALUE_NAME));
        assert_eq!((bold.range_min_value().to_f64(), bold.range_max_value().to_f64()), (650.0, 750.0));
        assert_eq!((roman.axis_index(), name(roman.value_name_id())), (1, "Roman".to_string()));
    }
}

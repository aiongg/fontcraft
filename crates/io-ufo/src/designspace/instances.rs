//! Designspace instances and variable fonts as the model's instances.
//!
//! An instance's attributes hold its style name, family name, PostScript name, file name and
//! style-map names; its `lib` holds what glyphsLib keeps there under `com.schriftgestaltung.*`
//! keys (export, weight and width, interpolation, parameters, properties) and its user data. A
//! value the model can derive is not written: the family name when it is the font's, the file
//! name glyphsLib would make, the linked style the style name implies, and the weight and width
//! classes the location gives.
//!
//! Ported from glyphsLib 6.15.0 (Apache-2.0): `builder/instances.py` (`_to_designspace_instance`,
//! `_to_designspace_varfont`, `to_glyphs_instances`, `_to_filename`), `builder/names.py`
//! (`build_stylemap_names`), `builder/axes.py` (`user_loc_value_to_class`), `util.py`
//! (`build_ufo_path`) and the lib keys of `builder/constants.py`.

use std::collections::HashMap;

use fontcraft_model::axes::{Mapping, number_value};
use fontcraft_model::{DEFAULT_LANGUAGE, Font, Instance, Localized, Param, Params, Properties, Property, PropertyValue, StyleClass, UserData};
use fontcraft_plist::xml::{self, Element, Node as XNode};
use fontcraft_plist::{Array, Dict, Str, Value, xmlplist};

use super::{AxisInfo, DOC, MAX_ITEMS, append_child, location_element, patch_location, read_location, remove_child};
use crate::libkeys::sorted_keys;
use crate::{Result, UfoError};

pub(super) const EXPORT: &str = "com.schriftgestaltung.export";
const WEIGHT: &str = "com.schriftgestaltung.weight";
const WIDTH: &str = "com.schriftgestaltung.width";
/// Misspelled in glyphsLib, and kept so.
const INTERPOLATIONS: &str = "com.schriftgestaltung.intanceInterpolations";
const MANUAL: &str = "com.schriftgestaltung.manualInterpolation";
const PARAMETERS: &str = "com.schriftgestaltung.customParameters";
const PROPERTIES: &str = "com.schriftgestaltung.properties";
const TYPED: [&str; 7] = [EXPORT, WEIGHT, WIDTH, INTERPOLATIONS, MANUAL, PARAMETERS, PROPERTIES];

pub(super) const UFO_FILENAME: &str = "UFO Filename";
const FULL_FILENAME: &str = "com.schriftgestaltung.Glyphs.fullFilename";
const FILE_NAME: &str = "fileName";
const FAMILY_PARAM: &str = "familyName";
const PS_NAME: &str = "postscriptFontName";
const FAMILY_NAMES: &str = "familyNames";
/// The style-map family name an instance states when it is not the family name followed by a
/// link style.
const STYLE_MAP_FAMILY: &str = "styleMapFamilyName";
/// Parameters the instance's attributes hold; the lib leaves them out (glyphsLib's blacklist,
/// except `fileName`, `weightClass` and `widthClass`, which no attribute holds exactly).
const IN_ATTRIBUTES: [&str; 5] = [FAMILY_PARAM, PS_NAME, UFO_FILENAME, FULL_FILENAME, STYLE_MAP_FAMILY];

/// The depth of an instance's `<dict>` inside `<lib>` in the document.
const LIB_DICT_LEVEL: usize = 4;

/// What reading an instance needs from the document.
pub(super) struct Ctx<'a> {
    /// The font's family name.
    pub family: &'a str,
    pub axes: &'a [AxisInfo],
    pub labels: &'a HashMap<String, Vec<f64>>,
    pub unit: &'a str,
}

fn text(s: &str) -> Value {
    Value::String(Str::new(s))
}

/// The family name an instance exports with: its `familyNames` property, its `familyName`
/// parameter, or the font's.
pub(super) fn family_of(inst: &Instance, font_family: &str) -> String {
    inst.properties
        .get(FAMILY_NAMES)
        .and_then(Property::default_value)
        .or_else(|| inst.params.get(FAMILY_PARAM).and_then(Value::as_str))
        .unwrap_or(font_family)
        .to_string()
}

/// The PostScript name an instance states: its property, else its parameter.
pub(super) fn ps_name_of(inst: &Instance) -> Option<String> {
    inst.properties.get(PS_NAME).and_then(Property::default_value).or_else(|| inst.params.get(PS_NAME).and_then(Value::as_str)).map(str::to_string)
}

/// The UFO file name glyphsLib gives an instance without a file name parameter.
fn default_filename(inst: &Instance, family: &str) -> String {
    match inst.params.get(FILE_NAME).and_then(Value::as_str) {
        Some(f) => format!("{f}.ufo"),
        None => format!("instance_ufos/{}-{}.ufo", family.replace(' ', ""), inst.name.replace(' ', "")),
    }
}

/// Whether an instance states its UFO file name in a parameter, rather than leaving it to
/// [`filename_of`] to build from its names.
pub(super) fn states_filename(inst: &Instance) -> bool {
    [UFO_FILENAME, FULL_FILENAME, FILE_NAME].iter().any(|k| inst.params.get(k).and_then(Value::as_str).is_some())
}

/// The UFO file name of an instance: its `UFO Filename` parameter (or the full file name glyphsLib
/// also accepts), else the default.
pub(super) fn filename_of(inst: &Instance, family: &str) -> String {
    inst.params
        .get(UFO_FILENAME)
        .or_else(|| inst.params.get(FULL_FILENAME))
        .and_then(Value::as_str)
        .map_or_else(|| default_filename(inst, family), str::to_string)
}

/// The style name without the last `Regular` (when neither bold nor italic), `Bold` (when bold)
/// and `Italic` or `Oblique` (when italic).
fn implied_link(style: &str, bold: bool, italic: bool) -> String {
    let (mut regular, mut bold, mut italic) = (!(bold || italic), bold, italic);
    let mut kept: Vec<&str> = Vec::new();
    for part in style.split_whitespace().rev() {
        if part == "Regular" && regular {
            regular = false;
        } else if part == "Bold" && bold {
            bold = false;
        } else if (part == "Italic" || part == "Oblique") && italic {
            italic = false;
        } else {
            kept.push(part);
        }
    }
    kept.reverse();
    kept.join(" ")
}

/// The style-map family and style names. A link style of `Regular`, or none, means the one the
/// style name implies; an empty one (which glyphsLib cannot hold) links to the family alone.
pub(super) fn stylemap(inst: &Instance, family: &str) -> (String, &'static str) {
    let style = match (inst.is_bold, inst.is_italic) {
        (true, true) => "bold italic",
        (true, false) => "bold",
        (false, true) => "italic",
        (false, false) => "regular",
    };
    let linked = match inst.link_style.as_deref() {
        Some(l) if l != "Regular" => l.to_string(),
        _ => implied_link(&inst.name, inst.is_bold, inst.is_italic),
    };
    let fam = match inst.params.get(STYLE_MAP_FAMILY).and_then(Value::as_str) {
        Some(f) => f.to_string(),
        None if linked.is_empty() => family.to_string(),
        None => format!("{family} {linked}"),
    };
    (fam, style)
}

/// The four style-map styles.
const RIBBI: [&str; 4] = ["regular", "bold", "italic", "bold italic"];

/// The style-map names ufo2ft gives a font whose style-map names are not stated: the style when
/// it is one of the four, else `regular`; the family with the style unless the style (or the
/// stated style-map style) is one of the four. Ported from ufo2ft's `fontInfoData`
/// (`styleMapFamilyNameFallback`, `styleMapStyleNameFallback`; MIT).
fn style_map_fallback(family: &str, style: &str, stated_style: Option<&str>) -> (String, String) {
    let lower = style.trim().to_lowercase();
    let map_style = if RIBBI.contains(&lower.as_str()) { lower.clone() } else { "regular".to_string() };
    let suffix = match stated_style {
        Some(s) if !RIBBI.contains(&s) => s.to_string(),
        Some(_) => String::new(),
        None if RIBBI.contains(&lower.as_str()) => String::new(),
        None => style.to_string(),
    };
    let map_family = [family, suffix.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    (map_family, map_style)
}

/// The OS/2 width class nearest a width in percent (the lower on a tie).
fn width_class(percent: f64) -> f64 {
    let values = [50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 150.0, 200.0];
    let mut best = (1.0, f64::INFINITY);
    for (class, v) in (1..=9).map(f64::from).zip(values) {
        let d = (v - percent).abs();
        if d < best.1 {
            best = (class, d);
        }
    }
    best.0
}

/// The weight and width classes an instance's location gives, through the axes' maps read
/// backwards; `None` for the defaults (400 and 5) and where there is no such axis.
pub(super) fn derived_classes(axes: &[AxisInfo], location: &[f64]) -> (Option<StyleClass>, Option<StyleClass>) {
    let user = |tag: &str| -> Option<f64> {
        let i = axes.iter().position(|a| a.tag == tag)?;
        let design = *location.get(i)?;
        let a = axes.get(i)?;
        let user = if a.map.is_empty() { design } else { Mapping::new(a.map.clone(), a.user_default).to_user(design) };
        // Rounded first, so that a location written with six decimals gives the same class.
        let user = (user * 1e4).round() / 1e4;
        user.is_finite().then_some(user)
    };
    let weight = user("wght").map(f64::trunc).filter(|w| *w != 400.0).map(StyleClass::Number);
    let width = user("wdth").map(width_class).filter(|w| *w != 5.0).map(StyleClass::Number);
    (weight, width)
}

/// Properties glyphsLib's lib list holds by their default value: a localised one when the key is
/// a plural (`designers`), else a plain one.
fn property_of(key: &str, value: &str) -> Property {
    let value = if key.ends_with('s') {
        PropertyValue::Localized(vec![Localized { language: DEFAULT_LANGUAGE.into(), value: value.into(), extra: Default::default() }])
    } else {
        PropertyValue::Single(value.into())
    };
    Property { key: key.into(), value, extra: Default::default() }
}

/// `[[name, value], …]` pairs.
fn pairs(v: &Value) -> Option<Vec<(String, Value)>> {
    v.as_array()?
        .items
        .iter()
        .take(MAX_ITEMS)
        .map(|p| match p.as_array().map(|a| a.items.as_slice()) {
            Some([k, v]) => Some((k.as_str()?.to_string(), v.clone())),
            _ => None,
        })
        .collect()
}

/// A weight or width class from the lib; the default (`Regular`, 400; `Medium (normal)`, 5) is
/// `Some(None)`, the model's absent class.
fn style_class(v: &Value, default: (&str, f64)) -> Option<Option<StyleClass>> {
    let c = match v {
        Value::String(s) => StyleClass::Name(s.as_str().to_string()),
        Value::Number(n) => StyleClass::Number(n.value().as_f64()),
        _ => return None,
    };
    let is_default = match &c {
        StyleClass::Name(n) => n == default.0,
        StyleClass::Number(n) => *n == default.1,
    };
    Some((!is_default).then_some(c))
}

const WEIGHT_DEFAULT: (&str, f64) = ("Regular", 400.0);
const WIDTH_DEFAULT: (&str, f64) = ("Medium (normal)", 5.0);

/// A class for the lib; the absent class is written as the default's name, for when the location
/// implies another.
fn style_class_value(c: Option<&StyleClass>, default: (&str, f64)) -> Option<Value> {
    match c {
        Some(StyleClass::Name(n)) => Some(text(n)),
        Some(StyleClass::Number(n)) => number_value(*n),
        None => Some(text(default.0)),
    }
}

fn interpolation(v: &Value) -> Option<Vec<(String, f64)>> {
    v.as_dict()?.entries.iter().take(MAX_ITEMS).map(|(k, v)| Some((k.as_str().to_string(), v.as_num()?.as_f64()))).collect()
}

fn variable(inst: &Instance) -> bool {
    inst.kind.as_deref() == Some("variable")
}

/// The parameters an instance's lib holds: the enabled ones its attributes do not hold (a
/// variable font's attributes hold only its file name, which the caller leaves out).
fn lib_params(inst: &Instance) -> Vec<&Param> {
    inst.params.0.iter().filter(|p| !p.disabled && (variable(inst) || !IN_ATTRIBUTES.contains(&p.name.as_str()))).collect()
}

/// The properties an instance's lib holds: all but those its attributes hold (the family name's
/// other languages have no place).
fn lib_properties(inst: &Instance) -> Vec<&Property> {
    inst.properties.0.iter().filter(|p| variable(inst) || p.key != PS_NAME && p.key != FAMILY_NAMES).collect()
}

/// What a typed lib key holds for the instance, `None` when the key is not written.
fn lib_entry(key: &str, inst: &Instance, derived: &(Option<StyleClass>, Option<StyleClass>)) -> Option<Value> {
    match key {
        EXPORT => (!inst.exports).then_some(Value::Bool(false)),
        WEIGHT => (inst.weight_class != derived.0).then(|| style_class_value(inst.weight_class.as_ref(), WEIGHT_DEFAULT)).flatten(),
        WIDTH => (inst.width_class != derived.1).then(|| style_class_value(inst.width_class.as_ref(), WIDTH_DEFAULT)).flatten(),
        INTERPOLATIONS => (!inst.interpolation.is_empty()).then(|| {
            let entries = inst.interpolation.iter().filter_map(|(m, w)| Some((Str::new(m.as_str()), number_value(*w)?))).collect();
            Value::Dict(Dict { entries, ..Default::default() })
        }),
        MANUAL => inst.manual_interpolation.then_some(Value::Bool(true)),
        PARAMETERS => {
            let items: Vec<Value> = lib_params(inst)
                .into_iter()
                .map(|p| Value::Array(Array { items: vec![text(&p.name), p.value.without_spellings()], ..Default::default() }))
                .collect();
            (!items.is_empty()).then(|| Value::Array(Array { items, ..Default::default() }))
        }
        PROPERTIES => {
            let items: Vec<Value> = lib_properties(inst)
                .into_iter()
                .filter_map(|p| Some(Value::Array(Array { items: vec![text(&p.key), text(p.default_value()?)], ..Default::default() })))
                .collect();
            (!items.is_empty()).then(|| Value::Array(Array { items, ..Default::default() }))
        }
        _ => None,
    }
}

/// Reads a typed lib key into the instance; `false` when the value is not of the form glyphsLib
/// writes (the entry then stays among the user data).
fn read_lib_entry(key: &str, v: &Value, inst: &mut Instance) -> bool {
    match key {
        EXPORT => v.as_bool().map(|b| inst.exports = b).is_some(),
        WEIGHT => style_class(v, WEIGHT_DEFAULT).map(|c| inst.weight_class = c).is_some(),
        WIDTH => style_class(v, WIDTH_DEFAULT).map(|c| inst.width_class = c).is_some(),
        INTERPOLATIONS => interpolation(v).map(|w| inst.interpolation = w).is_some(),
        MANUAL => v.as_bool().map(|b| inst.manual_interpolation = b).is_some(),
        PARAMETERS => pairs(v).map(|ps| inst.params.0.extend(ps.into_iter().map(|(n, v)| Param::new(n, v.without_spellings())))).is_some(),
        PROPERTIES => {
            let Some(ps) = pairs(v) else { return false };
            let Some(props) = ps.iter().map(|(k, v)| Some(property_of(k, v.as_str()?))).collect::<Option<Vec<_>>>() else { return false };
            inst.properties.0.extend(props);
            true
        }
        _ => false,
    }
}

/// The `<dict>` of an element's `<lib>`, read with its spellings.
fn lib_dict(el: &Element, unit: &str) -> Result<Option<Dict>> {
    lib_dict_at(el, unit, LIB_DICT_LEVEL)
}

/// The `<dict>` of an element's `<lib>` that sits at depth `level` in the document.
pub(super) fn lib_dict_at(el: &Element, unit: &str, level: usize) -> Result<Option<Dict>> {
    let Some(d) = el.child("lib").and_then(|l| l.child("dict")) else { return Ok(None) };
    let mut irregular = 0;
    match xmlplist::read_value(d, unit, level, &mut irregular) {
        Ok(Value::Dict(d)) => Ok(Some(d)),
        Ok(_) => Ok(None),
        Err(e) => Err(UfoError::Syntax { file: DOC.into(), error: e }),
    }
}

/// Reads an element's lib into the instance.
/// Reads an element's lib into the instance; returns the typed keys it read.
fn read_lib(el: &Element, unit: &str, inst: &mut Instance) -> Result<Vec<&'static str>> {
    let Some(dict) = lib_dict(el, unit)? else { return Ok(Vec::new()) };
    let (rest, typed) = split_lib(&dict, inst);
    inst.user_data = UserData(rest);
    Ok(typed)
}

/// The typed keys of a lib read into `inst`; returns the other entries and the typed keys read.
fn split_lib(dict: &Dict, inst: &mut Instance) -> (Dict, Vec<&'static str>) {
    let (mut rest, mut typed) = (Dict::new(), Vec::new());
    for (k, v) in dict.entries.iter().take(MAX_ITEMS) {
        match TYPED.iter().find(|t| **t == k.as_str()) {
            Some(t) if read_lib_entry(t, v, inst) => typed.push(*t),
            _ => rest.entries.push((Str::new(k.as_str()), v.without_spellings())),
        }
    }
    (rest, typed)
}

/// The classes the lib does not state are the ones the location gives.
fn fill_classes(inst: &mut Instance, typed: &[&str], derived: &(Option<StyleClass>, Option<StyleClass>)) {
    if !typed.contains(&WEIGHT) {
        inst.weight_class = derived.0.clone();
    }
    if !typed.contains(&WIDTH) {
        inst.width_class = derived.1.clone();
    }
}

/// An `<instance>` element as an instance.
pub(super) fn read_static(el: &Element, ctx: &Ctx<'_>) -> Result<Instance> {
    let name = el.attr("stylename").or_else(|| el.attr("name")).unwrap_or("").to_string();
    let mut inst = Instance { name, location: read_location(el, ctx.axes, ctx.labels)?, ..Default::default() };
    let typed = read_lib(el, ctx.unit, &mut inst)?;
    if let Some(f) = el.attr("familyname")
        && family_of(&inst, ctx.family) != f
    {
        set_property(&mut inst.properties, property_of(FAMILY_NAMES, f));
    }
    if let Some(p) = el.attr("postscriptfontname")
        && ps_name_of(&inst).as_deref() != Some(p)
    {
        set_property(&mut inst.properties, Property { key: PS_NAME.into(), value: PropertyValue::Single(p.into()), extra: Default::default() });
    }
    let family = family_of(&inst, ctx.family);
    if let Some(f) = el.attr("filename")
        && filename_of(&inst, &family) != f
    {
        inst.params.set(UFO_FILENAME, text(f));
    }
    // Style-map names the element leaves out are the ones ufo2ft gives the instance's font.
    let stated_style = el.attr("stylemapstylename").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let (fallback_family, fallback_style) = style_map_fallback(&family, &inst.name, stated_style.as_deref());
    let map_style = stated_style.unwrap_or(fallback_style);
    inst.is_bold = map_style.contains("bold");
    inst.is_italic = map_style.contains("italic");
    let map_family = el.attr("stylemapfamilyname").filter(|s| !s.is_empty()).map_or(fallback_family, str::to_string);
    let implied = implied_link(&inst.name, inst.is_bold, inst.is_italic);
    let link =
        if map_family == family { Some("") } else { map_family.strip_prefix(family.as_str()).and_then(|r| r.strip_prefix(' ')).map(str::trim) };
    match link {
        // A link style of `Regular` stands for the implied one, so it cannot state itself.
        Some(l) if l == "Regular" && implied != "Regular" => inst.params.set(STYLE_MAP_FAMILY, text(&map_family)),
        Some(l) if l != implied => inst.link_style = Some(l.to_string()),
        Some(_) => {}
        None => inst.params.set(STYLE_MAP_FAMILY, text(&map_family)),
    }
    let derived = derived_classes(ctx.axes, &inst.location);
    fill_classes(&mut inst, &typed, &derived);
    Ok(inst)
}

fn set_property(props: &mut Properties, p: Property) {
    match props.0.iter_mut().find(|q| q.key == p.key) {
        Some(q) => *q = p,
        None => props.0.push(p),
    }
}

/// The file name glyphsLib gives a variable font without a `fileName` parameter.
fn default_vf_filename(inst: &Instance, family: &str) -> String {
    let mut f = family.to_string();
    if inst.name != "Regular" {
        f.push('-');
        f.push_str(&inst.name);
    }
    f.push_str("VF");
    f.replace(' ', "")
}

/// A `<variable-font>` element as a variable instance.
pub(super) fn read_variable(el: &Element, ctx: &Ctx<'_>) -> Result<Instance> {
    let mut inst = Instance { name: el.attr("name").unwrap_or("").to_string(), kind: Some("variable".into()), ..Default::default() };
    read_lib(el, ctx.unit, &mut inst)?;
    if let Some(f) = el.attr("filename")
        && inst.params.get(FILE_NAME).and_then(Value::as_str).map_or_else(|| default_vf_filename(&inst, ctx.family), str::to_string) != f
    {
        inst.params.set(FILE_NAME, text(f));
    }
    Ok(inst)
}

/// A dictionary of the typed lib keys and the user data, sorted by key as fontTools writes it.
fn new_lib(inst: &Instance, derived: &(Option<StyleClass>, Option<StyleClass>)) -> Dict {
    let mut entries: Vec<(Str, Value)> = TYPED.iter().filter_map(|k| Some((Str::new(*k), sorted_keys(lib_entry(k, inst, derived)?)))).collect();
    for (k, v) in &inst.user_data.0.entries {
        if !entries.iter().any(|(e, _)| e.as_str() == k.as_str()) {
            entries.push((Str::new(k.as_str()), sorted_keys(v.without_spellings())));
        }
    }
    entries.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    Dict { entries, ..Default::default() }
}

/// A `<lib>` element holding `dict`.
pub(super) fn lib_element(dict: &Dict) -> Element {
    let mut out = String::new();
    xmlplist::write_value(&mut out, &Value::Dict(dict.clone()), "  ", 0);
    let mut d = xml::parse_element(&out).unwrap_or_else(|_| Element::new("dict"));
    d.strip_layout();
    Element::new("lib").with_child(d)
}

/// A new `<instance>` element in designspaceLib's layout.
pub(super) fn static_element(font: &Font, inst: &Instance, names: &[String], axes: &[AxisInfo]) -> Element {
    let family = family_of(inst, &font.family_name);
    let mut el = Element::new("instance").with_attr("name", format!("{family} {}", inst.name)).with_attr("familyname", &family);
    el.set_attr("stylename", &inst.name);
    if !names.is_empty() {
        let loc: Vec<f64> = (0..names.len()).map(|i| fontcraft_model::axes::design_value(&inst.location, i)).collect();
        el = el.with_child(location_element(names, &loc));
    }
    el.set_attr("filename", filename_of(inst, &family));
    if let Some(p) = ps_name_of(inst) {
        el.set_attr("postscriptfontname", p);
    }
    let (smfn, smsn) = stylemap(inst, &family);
    el.set_attr("stylemapfamilyname", smfn);
    el.set_attr("stylemapstylename", smsn);
    let lib = new_lib(inst, &derived_classes(axes, &inst.location));
    if !lib.is_empty() {
        el = el.with_child(lib_element(&lib));
    }
    el
}

/// A new `<variable-font>` element: the whole range of every axis, as glyphsLib writes it.
pub(super) fn variable_element(font: &Font, inst: &Instance, names: &[String]) -> Element {
    let filename = inst.params.get(FILE_NAME).and_then(Value::as_str).map_or_else(|| default_vf_filename(inst, &font.family_name), str::to_string);
    let mut el = Element::new("variable-font").with_attr("name", &inst.name).with_attr("filename", filename);
    if !names.is_empty() {
        let mut subsets = Element::new("axis-subsets");
        for n in names {
            subsets = subsets.with_child(Element::new("axis-subset").with_attr("name", n));
        }
        el = el.with_child(subsets);
    }
    let mut bare = inst.clone();
    bare.params.remove(FILE_NAME);
    let lib = new_lib(&bare, &(None, None));
    if !lib.is_empty() {
        el = el.with_child(lib_element(&lib));
    }
    el
}

/// Brings an element's lib to what the model holds, key by key: a typed key changes only when
/// what it says differs from the model, the user data only where it differs.
fn patch_lib(el: &mut Element, model: &Instance, derived: &(Option<StyleClass>, Option<StyleClass>), unit: &str, level: usize) -> Result<()> {
    let kept = lib_dict(el, unit)?;
    let mut dict = kept.clone().unwrap_or_default();
    // What the kept lib says, read on its own.
    let mut said = Instance::default();
    let (rest, typed) = split_lib(&dict, &mut said);
    let rest: Vec<String> = rest.entries.iter().map(|(k, _)| k.as_str().to_string()).collect();
    fill_classes(&mut said, &typed, derived);
    let differs = |key: &str| -> bool {
        match key {
            EXPORT => said.exports != model.exports,
            WEIGHT => said.weight_class != model.weight_class,
            WIDTH => said.width_class != model.width_class,
            INTERPOLATIONS => said.interpolation != model.interpolation,
            MANUAL => said.manual_interpolation != model.manual_interpolation,
            PARAMETERS => lib_params(&said) != lib_params(model),
            PROPERTIES => {
                let defaults =
                    |i: &Instance| lib_properties(i).into_iter().map(|p| (p.key.clone(), p.default_value().map(str::to_string))).collect::<Vec<_>>();
                defaults(&said) != defaults(model)
            }
            _ => false,
        }
    };
    for key in TYPED {
        if !rest.iter().any(|r| r == key) && differs(key) {
            set_entry(&mut dict, key, lib_entry(key, model, derived).map(sorted_keys));
        }
    }
    // User data: the keys the model no longer has go, changed values are replaced, new keys added.
    for k in rest.iter().filter(|k| model.user_data.get(k).is_none()) {
        set_entry(&mut dict, k, None);
    }
    for (k, v) in &model.user_data.0.entries {
        if dict.get(k.as_str()) != Some(v) {
            set_entry(&mut dict, k.as_str(), Some(sorted_keys(v.without_spellings())));
        }
    }
    if Some(&dict) == kept.as_ref() || kept.is_none() && dict.is_empty() {
        return Ok(());
    }
    let lib_index = el.elements().position(|e| e.name() == "lib");
    if dict.is_empty() {
        if let Some(i) = lib_index {
            remove_child(el, i);
        }
        return Ok(());
    }
    if lib_index.is_none() {
        append_child(el, lib_element(&dict), unit, level);
        return Ok(());
    }
    let mut out = String::new();
    xmlplist::write_value(&mut out, &Value::Dict(dict), unit, LIB_DICT_LEVEL);
    let new = xml::parse_element(&out).map_err(|e| UfoError::Syntax { file: DOC.into(), error: e })?;
    if let Some(lib) = el.elements_mut().find(|e| e.name() == "lib")
        && let Some(slot) = lib.children.iter_mut().find(|n| matches!(n, XNode::Element(e) if e.name() == "dict"))
    {
        *slot = XNode::Element(new);
    }
    Ok(())
}

/// Sets or removes a key, a new key going where it keeps the keys sorted.
pub(super) fn set_entry(dict: &mut Dict, key: &str, value: Option<Value>) {
    let at = dict.entries.iter().position(|(k, _)| k.as_str() == key);
    match (at, value) {
        (Some(i), Some(v)) => {
            if let Some(slot) = dict.entries.get_mut(i) {
                slot.1 = v;
            }
        }
        (Some(i), None) => {
            dict.remove_at(i);
        }
        (None, Some(v)) => {
            let i = dict.entries.iter().position(|(k, _)| k.as_str() > key).unwrap_or(dict.entries.len());
            dict.insert_at(i, Str::new(key), v);
        }
        (None, None) => {}
    }
}

/// Whether a parameter list holds one of `names`.
fn has_param(p: &Params, names: &[&str]) -> bool {
    p.0.iter().any(|p| !p.disabled && names.contains(&p.name.as_str()))
}

/// Brings a kept `<instance>` to the model's instance. `kept` is what the element reads as now.
pub(super) fn patch_static(
    el: &mut Element,
    kept: &Instance,
    model: &Instance,
    font: &Font,
    axes: &[AxisInfo],
    names: &[String],
    unit: &str,
) -> Result<()> {
    let level = 2;
    let (kept_family, family) = (family_of(kept, &font.family_name), family_of(model, &font.family_name));
    // The `name` attribute follows the family and style names while it was made of them.
    if (kept_family != family || kept.name != model.name) && el.attr("name") == Some(format!("{kept_family} {}", kept.name).as_str()) {
        el.set_attr("name", format!("{family} {}", model.name));
    }
    if kept.name != model.name {
        el.set_attr("stylename", &model.name);
    }
    let had_family = el.attr("familyname").is_some();
    let had_file = el.attr("filename").is_some();
    let had_map = el.attr("stylemapfamilyname").is_some() || el.attr("stylemapstylename").is_some();
    let family_set = |i: &Instance| i.properties.get(FAMILY_NAMES).is_some() || has_param(&i.params, &[FAMILY_PARAM]);
    if if had_family { kept_family != family } else { family_set(kept) != family_set(model) || family_set(model) && kept_family != family } {
        el.set_attr("familyname", &family);
    }
    patch_location(el, axes, names, &model.location, unit, level);
    let (kept_ps, ps) = (ps_name_of(kept), ps_name_of(model));
    if kept_ps != ps {
        match ps {
            Some(p) => el.set_attr("postscriptfontname", p),
            None => {
                el.remove_attr("postscriptfontname");
            }
        }
    }
    let file_set = |i: &Instance| has_param(&i.params, &[UFO_FILENAME, FULL_FILENAME, FILE_NAME]);
    let (kept_file, file) = (filename_of(kept, &kept_family), filename_of(model, &family));
    if if had_file { kept_file != file } else { file_set(model) && (!file_set(kept) || kept_file != file) } {
        el.set_attr("filename", file);
    }
    let linking = |i: &Instance| (i.is_bold, i.is_italic, i.link_style.clone());
    let changed = if had_map { stylemap(kept, &kept_family) != stylemap(model, &family) } else { linking(kept) != linking(model) };
    if changed {
        let (smfn, smsn) = stylemap(model, &family);
        el.set_attr("stylemapfamilyname", smfn);
        el.set_attr("stylemapstylename", smsn);
    }
    patch_lib(el, model, &derived_classes(axes, &model.location), unit, level)
}

/// Brings a kept `<variable-font>` to the model's variable instance.
pub(super) fn patch_variable(el: &mut Element, kept: &Instance, model: &Instance, font: &Font, unit: &str) -> Result<()> {
    if kept.name != model.name {
        el.set_attr("name", &model.name);
    }
    let file =
        |i: &Instance| i.params.get(FILE_NAME).and_then(Value::as_str).map_or_else(|| default_vf_filename(i, &font.family_name), str::to_string);
    if file(kept) != file(model) {
        el.set_attr("filename", file(model));
    }
    // The file name parameter is the attribute; the lib does not repeat it.
    let strip = |i: &Instance| {
        let mut i = i.clone();
        i.params.remove(FILE_NAME);
        i
    };
    patch_lib(el, &strip(model), &(None, None), unit, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_map_names_follow_glyphslib() {
        let i = |name: &str, bold: bool, italic: bool| Instance { name: name.into(), is_bold: bold, is_italic: italic, ..Default::default() };
        assert_eq!(stylemap(&i("Regular", false, false), "Fam"), ("Fam".to_string(), "regular"));
        assert_eq!(stylemap(&i("Bold Italic", true, true), "Fam"), ("Fam".to_string(), "bold italic"));
        assert_eq!(stylemap(&i("Condensed Bold", true, false), "Fam"), ("Fam Condensed".to_string(), "bold"));
        assert_eq!(stylemap(&i("Light", false, false), "Fam"), ("Fam Light".to_string(), "regular"));
        let linked = Instance { link_style: Some("Light".into()), ..i("Light Italic", false, true) };
        assert_eq!(stylemap(&linked, "Fam"), ("Fam Light".to_string(), "italic"));
    }

    #[test]
    fn missing_style_map_names_are_ufo2ft_s() {
        let f = |style: &str, stated: Option<&str>| style_map_fallback("Fam", style, stated);
        assert_eq!(f("Bold", None), ("Fam".to_string(), "bold".to_string()));
        assert_eq!(f("Bold Italic", None), ("Fam".to_string(), "bold italic".to_string()));
        assert_eq!(f("Light", None), ("Fam Light".to_string(), "regular".to_string()));
        assert_eq!(f("Light", Some("italic")), ("Fam".to_string(), "regular".to_string()), "a stated style-map style keeps the family alone");
        assert_eq!(f("Light", Some("thin")), ("Fam thin".to_string(), "regular".to_string()));
    }

    #[test]
    fn width_classes_are_the_nearest() {
        assert_eq!(width_class(100.0), 5.0);
        assert_eq!(width_class(81.25), 3.0, "a tie goes to the lower class");
        assert_eq!(width_class(180.0), 9.0);
    }
}

//! Stems, numbers, master guides and the instance fields.
//!
//! As format versions 3 and 4 write them:
//!
//! - The font's `stems` (`{name, horizontal?, filter?}`) and `numbers` (`{name}`), with each
//!   master's `stemValues` and `numberValues` in the same order.
//! - A master's `guides`: `{pos?, angle?, name?, locked?, lockAngle?, showMeasurement?, filter?,
//!   orientation?}`.
//! - An instance's `id`, `type`, `visible`, `weightClass`, `widthClass`, `isBold`, `isItalic`,
//!   `linkStyle`, `manualInterpolation` and `instanceInterpolations`. Version 2 spells these the
//!   same way, so [`crate::v2`] uses the same functions; its master stems, guides and axis values
//!   are its own.
//!
//! A guide's UFO colour and identifier have no key of their own: they are written into its name
//! as glyphsLib 6.15.0 writes them (`builder/guidelines.py`, Apache-2.0; see `NOTICE`), and read
//! back as part of the name.
//!
//! A list becomes model fields only when every item reads and writing the items back gives the
//! same text; a single key only when writing its field back gives the same text. Anything else
//! stays in `Extra` as written. A key the writer leaves out (a flag at its default) stays in
//! `Extra` too, and is written from there.

use fontcraft_model::{Guide, Instance, NumberDef, StemDef, StyleClass};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Dict, Layout, Num, Str, Value};

use crate::{as_pair, finish, int, into_extra, list, num, pair, string, take};

/// Whether two values would be written as the same text.
pub(crate) fn same_text(a: &Value, b: &Value) -> bool {
    let (mut x, mut y) = (String::new(), String::new());
    openstep::write_value(&mut x, a, Dialect::Glyphs3);
    openstep::write_value(&mut y, b, Dialect::Glyphs3);
    x == y
}

/// The list under `key` read item by item; taken out of `d` only when every item reads and writes
/// back as the same text. An empty list stays as written.
pub(crate) fn take_checked<T>(d: &mut Dict, key: &str, read: impl Fn(&Value) -> Option<T>, write: impl Fn(&T) -> Option<Value>) -> Vec<T> {
    let Some(Value::Array(a)) = d.get(key) else { return Vec::new() };
    if a.items.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(a.items.len());
    for item in &a.items {
        match read(item) {
            Some(t) if write(&t).is_some_and(|w| same_text(&w, item)) => out.push(t),
            _ => return Vec::new(),
        }
    }
    take(d, key);
    out
}

/// The value under `key`, taken out of `d` when writing it back gives the same text.
pub(crate) fn take_same<T>(d: &mut Dict, key: &str, read: impl Fn(&Value) -> Option<T>, write: impl Fn(&T) -> Option<Value>) -> Option<T> {
    let v = d.get(key)?;
    let t = read(v)?;
    if !write(&t).is_some_and(|w| same_text(&w, v)) {
        return None;
    }
    take(d, key);
    Some(t)
}

pub(crate) fn str_of(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// An optional string key.
pub(crate) fn take_text(d: &mut Dict, key: &str) -> Option<String> {
    take_same(d, key, str_of, |s: &String| Some(string(s)))
}

/// A flag written as `1` when on and left out when off. Any other spelling stays as written.
pub(crate) fn take_flag(d: &mut Dict, key: &str) -> bool {
    take_same(d, key, |v| matches!(v.as_num(), Some(Num::Int(1))).then_some(()), |_| Some(int(1))).is_some()
}

pub(crate) fn push_flag(e: &mut Vec<(Str, Value)>, key: &str, on: bool) {
    if on {
        e.push((key.into(), int(1)));
    }
}

pub(crate) fn push_text(e: &mut Vec<(Str, Value)>, key: &str, value: &Option<String>) {
    if let Some(v) = value {
        e.push((key.into(), string(v)));
    }
}

// ── stems and numbers ──────────────────────────────────────────────────────────────────────────

fn read_stem(v: &Value) -> Option<StemDef> {
    let mut d = v.as_dict()?.clone();
    let name = take_text(&mut d, "name")?;
    let horizontal = take_flag(&mut d, "horizontal");
    let filter = take_text(&mut d, "filter");
    Some(StemDef { name, horizontal, filter, extra: into_extra(d) })
}

fn write_stem(s: &StemDef) -> Option<Value> {
    let mut e = vec![("name".into(), string(&s.name))];
    push_flag(&mut e, "horizontal", s.horizontal);
    push_text(&mut e, "filter", &s.filter);
    Some(Value::Dict(finish(e, &s.extra)))
}

fn read_number(v: &Value) -> Option<NumberDef> {
    let mut d = v.as_dict()?.clone();
    let name = take_text(&mut d, "name")?;
    Some(NumberDef { name, extra: into_extra(d) })
}

fn write_number(n: &NumberDef) -> Option<Value> {
    Some(Value::Dict(finish(vec![("name".into(), string(&n.name))], &n.extra)))
}

pub(crate) fn take_stem_defs(d: &mut Dict) -> Vec<StemDef> {
    take_checked(d, "stems", read_stem, write_stem)
}

pub(crate) fn take_number_defs(d: &mut Dict) -> Vec<NumberDef> {
    take_checked(d, "numbers", read_number, write_number)
}

pub(crate) fn push_defs(e: &mut Vec<(Str, Value)>, stems: &[StemDef], numbers: &[NumberDef]) {
    if !stems.is_empty() {
        e.push(("stems".into(), list(stems.iter().filter_map(write_stem).collect())));
    }
    if !numbers.is_empty() {
        e.push(("numbers".into(), list(numbers.iter().filter_map(write_number).collect())));
    }
}

/// A list of plain numbers, such as `stemValues`.
pub(crate) fn take_values(d: &mut Dict, key: &str) -> Vec<f64> {
    take_checked(d, key, |v| v.as_num().map(Num::as_f64), |n| num(*n, key).ok())
}

pub(crate) fn push_values(e: &mut Vec<(Str, Value)>, key: &str, values: &[f64]) -> crate::Result<()> {
    if !values.is_empty() {
        e.push((key.into(), list(values.iter().map(|n| num(*n, key)).collect::<crate::Result<_>>()?)));
    }
    Ok(())
}

// ── guides ─────────────────────────────────────────────────────────────────────────────────────

/// A guide's keys other than its position, which the versions write differently.
pub(crate) fn take_guide_rest(d: &mut Dict, g: &mut Guide) -> Option<()> {
    if let Some(a) = take(d, "angle") {
        g.angle = a.as_num()?.as_f64();
    }
    g.name = take_text(d, "name");
    g.filter = take_text(d, "filter");
    g.orientation = take_text(d, "orientation");
    g.locked = take_flag(d, "locked");
    g.angle_locked = take_flag(d, "lockAngle");
    g.show_measurement = take_flag(d, "showMeasurement");
    Some(())
}

pub(crate) fn push_guide_rest(e: &mut Vec<(Str, Value)>, g: &Guide) -> Option<()> {
    if g.angle != 0.0 {
        e.push(("angle".into(), num(g.angle, "guide").ok()?));
    }
    push_text(e, "name", &folded_name(g));
    push_text(e, "filter", &g.filter);
    push_text(e, "orientation", &g.orientation);
    push_flag(e, "locked", g.locked);
    push_flag(e, "lockAngle", g.angle_locked);
    push_flag(e, "showMeasurement", g.show_measurement);
    Some(())
}

/// The guide's name with a UFO guideline's colour and identifier added as glyphsLib adds them
/// (`builder/guidelines.py`, `to_glyphs_guidelines`): ` [<colour>]`, then ` [#<identifier>]`. An
/// empty colour or identifier adds nothing.
fn folded_name(g: &Guide) -> Option<String> {
    let color = g.color.as_deref().filter(|c| !c.is_empty());
    let identifier = g.identifier.as_deref().filter(|i| !i.is_empty());
    if color.is_none() && identifier.is_none() {
        return g.name.clone();
    }
    let mut name = g.name.clone().unwrap_or_default();
    if let Some(c) = color {
        name.push_str(&format!(" [{c}]"));
    }
    if let Some(i) = identifier {
        name.push_str(&format!(" [#{i}]"));
    }
    Some(name)
}

pub(crate) fn read_guide(v: &Value) -> Option<Guide> {
    let mut d = v.as_dict()?.clone();
    let mut g = Guide::default();
    if let Some(p) = take(&mut d, "pos") {
        (g.x, g.y) = as_pair(&p, "pos").ok()?;
    }
    take_guide_rest(&mut d, &mut g)?;
    g.extra = into_extra(d);
    Some(g)
}

pub(crate) fn write_guide(g: &Guide) -> Option<Value> {
    let mut e = Vec::new();
    // Left out at the origin, as the format's default.
    if (g.x, g.y) != (0.0, 0.0) {
        e.push(("pos".into(), pair(g.x, g.y, "guide").ok()?));
    }
    push_guide_rest(&mut e, g)?;
    Some(Value::Dict(finish(e, &g.extra)))
}

pub(crate) fn take_guides(d: &mut Dict) -> Vec<Guide> {
    take_checked(d, "guides", read_guide, write_guide)
}

pub(crate) fn push_guides(e: &mut Vec<(Str, Value)>, guides: &[Guide]) -> crate::Result<()> {
    if !guides.is_empty() {
        let items = guides
            .iter()
            .map(|g| write_guide(g).ok_or_else(|| crate::bad("guide", "a value is not a finite number")))
            .collect::<crate::Result<_>>()?;
        e.push(("guides".into(), list(items)));
    }
    Ok(())
}

// ── instances ──────────────────────────────────────────────────────────────────────────────────

fn style_class(v: &Value) -> Option<StyleClass> {
    match v {
        Value::Number(n) => Some(StyleClass::Number(n.value().as_f64())),
        Value::String(s) => Some(StyleClass::Name(s.as_str().to_string())),
        _ => None,
    }
}

fn style_class_value(c: &StyleClass) -> Option<Value> {
    match c {
        StyleClass::Number(n) => num(*n, "class").ok(),
        StyleClass::Name(s) => Some(string(s)),
    }
}

/// `instanceInterpolations` when every weight is a plain number (not an `[x, y]` pair or a
/// version 2 `"{x, y}"` string).
fn interpolation(v: &Value) -> Option<Vec<(String, f64)>> {
    let entries = &v.as_dict()?.entries;
    if entries.is_empty() {
        return None;
    }
    entries.iter().map(|(k, w)| Some((k.as_str().to_string(), w.as_num()?.as_f64()))).collect()
}

fn interpolation_value(weights: &[(String, f64)]) -> Option<Value> {
    let mut d = Dict { layout: Layout::Multiline, ..Default::default() };
    for (m, w) in weights {
        d.entries.push((Str::new(m.as_str()), num(*w, "instanceInterpolations").ok()?));
    }
    Some(Value::Dict(d))
}

/// The instance keys this module types, moved from `d` into `i`.
pub(crate) fn take_instance_fields(d: &mut Dict, i: &mut Instance) {
    i.id = take_text(d, "id");
    i.kind = take_text(d, "type");
    // On by default: written only as `visible = 0`.
    i.visible = take_same(d, "visible", |v| matches!(v.as_num(), Some(Num::Int(0))).then_some(()), |_| Some(int(0))).is_none();
    i.weight_class = take_same(d, "weightClass", style_class, style_class_value);
    i.width_class = take_same(d, "widthClass", style_class, style_class_value);
    i.is_bold = take_flag(d, "isBold");
    i.is_italic = take_flag(d, "isItalic");
    i.link_style = take_text(d, "linkStyle");
    i.manual_interpolation = take_flag(d, "manualInterpolation");
    i.interpolation = take_same(d, "instanceInterpolations", interpolation, |w: &Vec<(String, f64)>| interpolation_value(w)).unwrap_or_default();
}

pub(crate) fn push_instance_fields(e: &mut Vec<(Str, Value)>, i: &Instance) -> crate::Result<()> {
    push_text(e, "id", &i.id);
    push_text(e, "type", &i.kind);
    if !i.visible {
        e.push(("visible".into(), int(0)));
    }
    for (key, class) in [("weightClass", &i.weight_class), ("widthClass", &i.width_class)] {
        if let Some(c) = class {
            e.push((key.into(), style_class_value(c).ok_or_else(|| crate::bad(key, "a class is not a finite number"))?));
        }
    }
    push_flag(e, "isBold", i.is_bold);
    push_flag(e, "isItalic", i.is_italic);
    push_text(e, "linkStyle", &i.link_style);
    push_flag(e, "manualInterpolation", i.manual_interpolation);
    if !i.interpolation.is_empty() {
        let v = interpolation_value(&i.interpolation).ok_or_else(|| crate::bad("instanceInterpolations", "a weight is not a finite number"))?;
        e.push(("instanceInterpolations".into(), v));
    }
    Ok(())
}

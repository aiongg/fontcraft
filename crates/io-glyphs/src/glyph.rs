//! A glyph's metadata ([`GlyphInfo`]), as each format version writes it.
//!
//! | Field | Versions 3 and 4 | Version 2 |
//! |---|---|---|
//! | production name | `production` | the same |
//! | category, subcategory, script | `category`, `subCategory`, `script` | the same |
//! | case, writing direction | `case`, `direction` | none |
//! | metrics keys | `metricLeft`, `metricRight`, `metricWidth`, `metricTop`, `metricBottom`, `metricVertWidth`, `metricVertOrigin` | `leftMetricsKey`, `rightMetricsKey`, `widthMetricsKey`, `topMetricsKey`, `bottomMetricsKey`, `vertWidthMetricsKey`; no vertical origin |
//! | colour label | `color`: an index, or `(r,g,b,a)` with 8-bit components (version 4: from 0 to 1) | an index, or 8-bit components |
//! | tags | `tags` | none |
//! | note | `note` | the same |
//! | lock | `locked = 1` | the same |
//! | last change | `lastChange` | the same |
//! | sort names | `sortName`, `sortNameKeep` | none |
//! | smart-glyph properties | `partsSettings`: `{name, bottomValue, topValue}`, with `bottomName` and `topName` in some files | the same, keys in the order `name`, `bottomName`, `bottomValue`, `topName`, `topValue` |
//! | user data | `userData` | the same |
//!
//! A key becomes a field only when writing the field back gives the same text in the file's
//! version; anything else stays in the glyph's `Extra` as written. So does a value the writer
//! leaves out (`note = ""`, `locked = 0`, an empty list), and the writer writes it from there.
//! Version 4's `axes`, `group` and `groupIdx` stay in `Extra`; colours other than an index or
//! RGBA (version 4's grey, CMYK and palette references) too.
//!
//! Written from the GlyphsSDK format documents (Apache-2.0); the version 2 key order of a
//! smart-glyph property is glyphsLib 6.15.0's (`classes.py`, Apache-2.0; see `NOTICE`).

use fontcraft_model::{ColorLabel, GlyphInfo, MAX_GLYPH_TAGS, MAX_PART_PROPERTIES, MetricsKeys, PartProperty};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Array, Dict, Layout, Num, Str, Value};

use crate::design::str_of;
use crate::{finish, int, into_extra, list, num, string, take};

/// The metrics keys of versions 3 and 4, in [`MetricsKeys::all`] order.
pub(crate) const METRIC_KEYS: [Option<&str>; 7] = [
    Some("metricLeft"),
    Some("metricRight"),
    Some("metricWidth"),
    Some("metricTop"),
    Some("metricBottom"),
    Some("metricVertWidth"),
    Some("metricVertOrigin"),
];
/// The metrics keys of version 2.
const METRIC_KEYS_V2: [Option<&str>; 7] = [
    Some("leftMetricsKey"),
    Some("rightMetricsKey"),
    Some("widthMetricsKey"),
    Some("topMetricsKey"),
    Some("bottomMetricsKey"),
    Some("vertWidthMetricsKey"),
    None,
];
/// A smart-glyph property's keys in the order version 2 writes them (glyphsLib's format 2
/// writer, `classes.py`, 6.15.0; the corpus agrees).
const PART_ORDER_V2: &[&str] = &["name", "bottomName", "bottomValue", "topName", "topValue"];

fn dialect(version: u32) -> Dialect {
    match version {
        2 => Dialect::Glyphs2,
        4 => Dialect::Glyphs4,
        _ => Dialect::Glyphs3,
    }
}

/// Whether two values are written as the same text in `dialect`.
fn same_text(a: &Value, b: &Value, dialect: Dialect) -> bool {
    let (mut x, mut y) = (String::new(), String::new());
    openstep::write_value(&mut x, a, dialect);
    openstep::write_value(&mut y, b, dialect);
    x == y
}

/// The value under `key`, taken out of `d` when writing it back gives the same text.
fn take_same<T>(d: &mut Dict, key: &str, version: u32, read: impl Fn(&Value) -> Option<T>, write: impl Fn(&T) -> Value) -> Option<T> {
    let v = d.get(key)?;
    let t = read(v)?;
    if !same_text(&write(&t), v, dialect(version)) {
        return None;
    }
    take(d, key);
    Some(t)
}

/// A string that is not empty (the writer leaves an empty one out).
fn take_text(d: &mut Dict, key: &str, version: u32) -> Option<String> {
    take_same(d, key, version, |v| str_of(v).filter(|s| !s.is_empty()), |s: &String| string(s))
}

pub(crate) fn read_color(v: &Value, version: u32) -> Option<ColorLabel> {
    let byte = |v: &Value| match v.as_num() {
        Some(Num::Int(i)) => u8::try_from(i).ok(),
        _ => None,
    };
    match v {
        Value::Number(_) => byte(v).map(ColorLabel::Index),
        Value::Array(a) => {
            let [r, g, b, alpha] = a.items.as_slice() else { return None };
            let mut c = [0.0; 4];
            for (slot, item) in c.iter_mut().zip([r, g, b, alpha]) {
                *slot =
                    if version >= 4 { item.as_num().map(Num::as_f64).filter(|f| (0.0..=1.0).contains(f))? } else { f64::from(byte(item)?) / 255.0 };
            }
            Some(ColorLabel::Rgba(c))
        }
        _ => None,
    }
}

/// An 8-bit colour component for a component from 0 to 1.
pub(crate) fn to_byte(c: f64) -> i64 {
    // In range after clamping, so the conversion is exact.
    #[allow(clippy::cast_possible_truncation)]
    let b = (c.clamp(0.0, 1.0) * 255.0).round() as i64;
    b
}

pub(crate) fn write_color(c: &ColorLabel, version: u32) -> Value {
    match c {
        ColorLabel::Index(i) => int(*i),
        ColorLabel::Rgba(parts) => {
            let items = parts.iter().map(|p| if version >= 4 { num(*p, "color").unwrap_or_else(|_| int(0)) } else { int(to_byte(*p)) }).collect();
            Value::Array(Array { items, layout: Layout::Compact, ..Default::default() })
        }
    }
}

/// Whether a colour holds more than 8-bit components can say (versions 2 and 3 write those).
pub(crate) fn color_needs_floats(c: &ColorLabel) -> bool {
    match c {
        ColorLabel::Index(_) => false,
        // A component read from an 8-bit value comes back to the same number.
        #[allow(clippy::cast_precision_loss)]
        ColorLabel::Rgba(parts) => parts.iter().any(|p| (to_byte(*p) as f64) / 255.0 != *p),
    }
}

fn read_tags(v: &Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    if a.items.is_empty() || a.items.len() > MAX_GLYPH_TAGS {
        return None;
    }
    a.items.iter().map(str_of).collect()
}

fn write_tags(tags: &[String]) -> Value {
    list(tags.iter().map(|t| string(t)).collect())
}

fn read_part(v: &Value) -> Option<PartProperty> {
    let mut d = v.as_dict()?.clone();
    let name = str_of(&take(&mut d, "name")?)?;
    let number = |v: Value| v.as_num().map(Num::as_f64).filter(|f| f.is_finite());
    let bottom_value = number(take(&mut d, "bottomValue")?)?;
    let top_value = number(take(&mut d, "topValue")?)?;
    let bottom_name = match take(&mut d, "bottomName") {
        Some(v) => Some(str_of(&v)?),
        None => None,
    };
    let top_name = match take(&mut d, "topName") {
        Some(v) => Some(str_of(&v)?),
        None => None,
    };
    Some(PartProperty { name, bottom_value, top_value, bottom_name, top_name, extra: into_extra(d) })
}

fn write_part(p: &PartProperty, version: u32) -> Value {
    let mut e: Vec<(Str, Value)> = vec![("name".into(), string(&p.name))];
    if let Some(n) = &p.bottom_name {
        e.push(("bottomName".into(), string(n)));
    }
    e.push(("bottomValue".into(), num(p.bottom_value, "bottomValue").unwrap_or_else(|_| int(0))));
    if let Some(n) = &p.top_name {
        e.push(("topName".into(), string(n)));
    }
    e.push(("topValue".into(), num(p.top_value, "topValue").unwrap_or_else(|_| int(0))));
    let mut d = finish(e, &p.extra);
    if version == 2 {
        let rank = |k: &str| PART_ORDER_V2.iter().position(|o| *o == k).unwrap_or(PART_ORDER_V2.len());
        d.entries.sort_by_key(|(k, _)| rank(k.as_str()));
    }
    Value::Dict(d)
}

fn read_parts(v: &Value) -> Option<Vec<PartProperty>> {
    let a = v.as_array()?;
    if a.items.is_empty() || a.items.len() > MAX_PART_PROPERTIES {
        return None;
    }
    a.items.iter().map(read_part).collect()
}

fn write_parts(parts: &[PartProperty], version: u32) -> Value {
    list(parts.iter().map(|p| write_part(p, version)).collect())
}

fn metric_keys(version: u32) -> [Option<&'static str>; 7] {
    if version == 2 { METRIC_KEYS_V2 } else { METRIC_KEYS }
}

/// The glyph's metadata, taken out of its dictionary.
pub(crate) fn take_glyph_info(d: &mut Dict, version: u32) -> GlyphInfo {
    let v3 = version >= 3;
    let mut keys = [None, None, None, None, None, None, None];
    for (slot, key) in keys.iter_mut().zip(metric_keys(version)) {
        *slot = key.and_then(|k| take_text(d, k, version));
    }
    let [left, right, width, top, bottom, vert_width, vert_origin] = keys;
    GlyphInfo {
        production: take_text(d, "production", version),
        category: take_text(d, "category", version),
        sub_category: take_text(d, "subCategory", version),
        script: take_text(d, "script", version),
        case: if v3 { take_text(d, "case", version) } else { None },
        direction: if v3 { take_text(d, "direction", version) } else { None },
        opentype_category: None,
        metrics_keys: MetricsKeys { left, right, width, top, bottom, vert_width, vert_origin },
        color: take_same(d, "color", version, |v| read_color(v, version), |c| write_color(c, version)),
        tags: if v3 { take_same(d, "tags", version, read_tags, |t: &Vec<String>| write_tags(t)).unwrap_or_default() } else { Vec::new() },
        note: take_text(d, "note", version).unwrap_or_default(),
        locked: take_same(d, "locked", version, |v| matches!(v.as_num(), Some(Num::Int(1))).then_some(()), |_| int(1)).is_some(),
        last_change: take_text(d, "lastChange", version),
        sort_name: if v3 { take_text(d, "sortName", version) } else { None },
        sort_name_keep: if v3 { take_text(d, "sortNameKeep", version) } else { None },
        parts: take_same(d, "partsSettings", version, read_parts, |p: &Vec<PartProperty>| write_parts(p, version)).unwrap_or_default(),
        user_data: crate::info::take_user_data(d),
    }
}

/// The glyph's metadata as entries of its dictionary (a version without a key for a field
/// leaves it out; [`crate::info_losses`] lists it).
pub(crate) fn push_glyph_info(e: &mut Vec<(Str, Value)>, info: &GlyphInfo, version: u32) {
    let v3 = version >= 3;
    let mut text = |key: &str, value: &Option<String>| {
        if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
            e.push((key.into(), string(v)));
        }
    };
    text("production", &info.production);
    text("category", &info.category);
    text("subCategory", &info.sub_category);
    text("script", &info.script);
    text("lastChange", &info.last_change);
    if v3 {
        text("case", &info.case);
        text("direction", &info.direction);
        text("sortName", &info.sort_name);
        text("sortNameKeep", &info.sort_name_keep);
    }
    for (key, (_, value)) in metric_keys(version).into_iter().zip(info.metrics_keys.all()) {
        if let Some(key) = key {
            text(key, value);
        }
    }
    if !info.note.is_empty() {
        e.push(("note".into(), string(&info.note)));
    }
    if let Some(c) = &info.color {
        e.push(("color".into(), write_color(c, version)));
    }
    if v3 && !info.tags.is_empty() {
        e.push(("tags".into(), write_tags(&info.tags)));
    }
    if info.locked {
        e.push(("locked".into(), int(1)));
    }
    if !info.parts.is_empty() {
        e.push(("partsSettings".into(), write_parts(&info.parts, version)));
    }
    crate::info::push_user_data(e, &info.user_data);
}

/// What a version cannot hold of the glyphs' metadata, one line each.
pub(crate) fn losses(glyphs: &[std::sync::Arc<fontcraft_model::Glyph>], version: u32) -> Vec<String> {
    let count = |f: &dyn Fn(&GlyphInfo) -> bool| glyphs.iter().filter(|g| f(&g.info)).count();
    let mut checks: Vec<(&str, usize)> = vec![("the glyphs' OpenType categories", count(&|i| i.opentype_category.is_some()))];
    if version == 2 {
        checks.extend([
            ("the glyphs' case", count(&|i| i.case.is_some())),
            ("the glyphs' writing direction", count(&|i| i.direction.is_some())),
            ("the glyphs' tags", count(&|i| !i.tags.is_empty())),
            ("the glyphs' sort names", count(&|i| i.sort_name.is_some() || i.sort_name_keep.is_some())),
            ("the glyphs' vertical origin metrics keys", count(&|i| i.metrics_keys.vert_origin.is_some())),
        ]);
    }
    if version < 4 {
        checks.push(("the glyphs' colours, rounded to 8 bits", count(&|i| i.color.as_ref().is_some_and(color_needs_floats))));
    }
    checks.into_iter().filter(|(_, n)| *n > 0).map(|(what, n)| format!("{what} ({n})")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_plist::openstep;

    fn dict(text: &str) -> Dict {
        match openstep::parse(text).unwrap().root {
            Value::Dict(d) => d,
            _ => panic!(),
        }
    }

    fn written(info: &GlyphInfo, version: u32) -> String {
        let mut e = Vec::new();
        push_glyph_info(&mut e, info, version);
        let mut out = String::new();
        openstep::write_value(&mut out, &Value::Dict(finish(e, &fontcraft_model::Extra::new())), dialect(version));
        out
    }

    #[test]
    fn each_field_reads_in_version_3() {
        let text = "{\ncase = upper;\ncategory = Letter;\ncolor = 4;\ndirection = LTR;\nlastChange = \"2023-07-06 13:12:48 +0000\";\nlocked = 1;\nmetricLeft = \"=H\";\nmetricRight = H;\nmetricVertOrigin = n;\nmetricWidth = zero.tf;\nnote = \"a note\";\npartsSettings = (\n{\nbottomValue = 0;\nname = Width;\ntopValue = 100;\n}\n);\nproduction = uni0041;\nscript = latin;\nsortName = XA;\nsortNameKeep = YA;\nsubCategory = Uppercase;\ntags = (\nnew,\nunfinished\n);\nuserData = {\nk = v;\n};\n}";
        let mut d = dict(text);
        let info = take_glyph_info(&mut d, 3);
        assert!(d.is_empty(), "left over: {d:?}");
        assert_eq!(info.production.as_deref(), Some("uni0041"));
        assert_eq!(
            (info.category.as_deref(), info.sub_category.as_deref(), info.script.as_deref()),
            (Some("Letter"), Some("Uppercase"), Some("latin"))
        );
        assert_eq!((info.case.as_deref(), info.direction.as_deref()), (Some("upper"), Some("LTR")));
        assert_eq!(info.metrics_keys.left.as_deref(), Some("=H"));
        assert_eq!(info.metrics_keys.vert_origin.as_deref(), Some("n"));
        assert_eq!(info.color, Some(ColorLabel::Index(4)));
        assert_eq!(info.tags, ["new", "unfinished"]);
        assert_eq!(info.note, "a note");
        assert!(info.locked);
        assert_eq!(info.last_change.as_deref(), Some("2023-07-06 13:12:48 +0000"));
        assert_eq!((info.sort_name.as_deref(), info.sort_name_keep.as_deref()), (Some("XA"), Some("YA")));
        assert_eq!(info.parts.len(), 1);
        assert_eq!((info.parts[0].bottom_value, info.parts[0].top_value), (0.0, 100.0));
        assert!(info.user_data.get("k").is_some());
        assert_eq!(written(&info, 3), text, "written back as read");
    }

    #[test]
    fn version_2_spells_its_own_keys() {
        let text = "{\ncategory = Mark;\ncolor = (255,0,0,255);\nleftMetricsKey = hyphen;\nnote = \"two\\012lines\";\npartsSettings = (\n{\nname = Width;\nbottomName = Low;\nbottomValue = 310;\ntopName = High;\ntopValue = 413;\n}\n);\nvertWidthMetricsKey = space;\nwidthMetricsKey = space;\n}";
        let mut d = dict(text);
        let info = take_glyph_info(&mut d, 2);
        assert!(d.is_empty(), "left over: {d:?}");
        assert_eq!(info.metrics_keys.left.as_deref(), Some("hyphen"));
        assert_eq!(info.metrics_keys.width.as_deref(), Some("space"));
        assert_eq!(info.metrics_keys.vert_width.as_deref(), Some("space"));
        assert_eq!(info.note, "two\nlines");
        assert_eq!(info.color, Some(ColorLabel::Rgba([1.0, 0.0, 0.0, 1.0])));
        assert_eq!(info.parts[0].bottom_name.as_deref(), Some("Low"));
        assert_eq!(written(&info, 2), text);
        // Version 3 spellings are not version 2 keys.
        let mut d = dict("{\nmetricLeft = H;\ntags = (a);\ncase = upper;\n}");
        let info = take_glyph_info(&mut d, 2);
        assert_eq!(d.len(), 3);
        assert!(info.metrics_keys.is_empty() && info.tags.is_empty() && info.case.is_none());
    }

    #[test]
    fn version_4_colours_are_fractions() {
        let mut d = dict("{\ncolor = (1,0.5,0,1);\n}");
        let info = take_glyph_info(&mut d, 4);
        assert_eq!(info.color, Some(ColorLabel::Rgba([1.0, 0.5, 0.0, 1.0])));
        assert!(written(&info, 4).contains("color = (1,0.5,0,1);"));
        // The same list in version 3 is 8-bit.
        let info = take_glyph_info(&mut dict("{\ncolor = (1,0,0,1);\n}"), 3);
        assert_eq!(info.color, Some(ColorLabel::Rgba([1.0 / 255.0, 0.0, 0.0, 1.0 / 255.0])));
        assert!(written(&info, 3).contains("color = (1,0,0,1);"));
        // Grey, CMYK and palette colours stay as written.
        let mut d = dict("{\ncolor = (0.5,1);\n}");
        assert_eq!(take_glyph_info(&mut d, 4).color, None);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn explicit_defaults_and_odd_spellings_stay_as_written() {
        let mut d =
            dict("{\nnote = \"\";\nlocked = 0;\ntags = ();\npartsSettings = ();\nproduction = \"uni0041\";\ncolor = 300;\nuserData = {\n};\n}");
        let info = take_glyph_info(&mut d, 3);
        assert_eq!(info, GlyphInfo::default());
        assert_eq!(d.len(), 7, "every key stays in Extra");
    }

    #[test]
    fn counts_from_input_are_capped() {
        let many: Vec<String> = (0..=MAX_GLYPH_TAGS).map(|i| format!("t{i}")).collect();
        let mut d = dict(&format!("{{\ntags = (\n{}\n);\n}}", many.join(",\n")));
        assert!(take_glyph_info(&mut d, 3).tags.is_empty());
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn colours_beyond_8_bits_are_reported_for_version_3() {
        let mut g = fontcraft_model::Glyph::new("a");
        g.info.color = Some(ColorLabel::Rgba([0.5, 0.0, 0.0, 1.0]));
        let glyphs = vec![std::sync::Arc::new(g)];
        assert_eq!(losses(&glyphs, 3), ["the glyphs' colours, rounded to 8 bits (1)"]);
        assert!(losses(&glyphs, 4).is_empty());
    }
}

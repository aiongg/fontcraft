//! UFO 1, the first version of the format: read by converting its font information and lib to
//! their UFO 2 form, after which the UFO 2 reader takes over. The document it gives is a UFO 3
//! source, so saving writes UFO 3.
//!
//! The font-info conversion is ported from fontTools' ufoLib (`ufoLib/__init__.py`: the version 1
//! attributes, their version 2 names and the value tables); the lib conversion from defcon
//! (`objects/font.py`, `_convertFromFormatVersion1RoboFabData`). Both MIT; see `NOTICE`.
//!
//! | UFO 1 | UFO 2 and 3 |
//! |---|---|
//! | `fontinfo.plist` keys of version 1 | their version 2 names ([`INFO_KEYS`]); `fontStyle`, `widthName` and `msCharSet` values through their tables; whole numbers as integers; other keys dropped |
//! | `weightValue` -1 | no `openTypeOS2WeightClass` (FontLab wrote -1 for "not set") |
//! | lib `org.robofab.opentype.classes`, `…features` (tag → code), `…featureorder` | `features.fea`: the classes, then each feature in that order (sorted by tag without one), joined by line breaks |
//! | lib `org.robofab.postScriptHintData` | `postscriptBlueValues` and the other blue lists (pairs flattened), `postscriptBlueFuzz`, `…BlueScale`, `…BlueShift`, `…ForceBold`, `…StemSnapH` (`hStems`), `…StemSnapV` (`vStems`) |
//!
//! A value the tables have no entry for, or of the wrong type, is dropped rather than refusing
//! the file. Groups and kerning are the same as in UFO 2, and GLIF files are format 1, as in UFO 2.

use std::collections::HashSet;

use fontcraft_plist::{Array, Dict, Num, Number, Spacing, Str, Value};

use crate::ufo::patch;

/// The `formatVersion` of a UFO 1.
pub(crate) const VERSION: u32 = 1;

/// Every version 1 font-info key with the version 2 key it becomes.
pub(crate) const INFO_KEYS: [(&str, &str); 40] = [
    ("familyName", "familyName"),
    ("styleName", "styleName"),
    ("fullName", "postscriptFullName"),
    ("fontName", "postscriptFontName"),
    ("menuName", "styleMapFamilyName"),
    ("fontStyle", "styleMapStyleName"),
    ("note", "note"),
    ("versionMajor", "versionMajor"),
    ("versionMinor", "versionMinor"),
    ("year", "year"),
    ("copyright", "copyright"),
    ("notice", "openTypeNameDescription"),
    ("trademark", "trademark"),
    ("license", "openTypeNameLicense"),
    ("licenseURL", "openTypeNameLicenseURL"),
    ("createdBy", "openTypeNameManufacturer"),
    ("designer", "openTypeNameDesigner"),
    ("designerURL", "openTypeNameDesignerURL"),
    ("vendorURL", "openTypeNameManufacturerURL"),
    ("unitsPerEm", "unitsPerEm"),
    ("ascender", "ascender"),
    ("descender", "descender"),
    ("capHeight", "capHeight"),
    ("xHeight", "xHeight"),
    ("defaultWidth", "postscriptDefaultWidthX"),
    ("slantAngle", "postscriptSlantAngle"),
    ("italicAngle", "italicAngle"),
    ("widthName", "openTypeOS2WidthClass"),
    ("weightName", "postscriptWeightName"),
    ("weightValue", "openTypeOS2WeightClass"),
    ("fondName", "macintoshFONDName"),
    ("otFamilyName", "openTypeNamePreferredFamilyName"),
    ("otStyleName", "openTypeNamePreferredSubfamilyName"),
    ("otMacName", "openTypeNameCompatibleFullName"),
    ("msCharSet", "postscriptWindowsCharacterSet"),
    ("fondID", "macintoshFONDFamilyID"),
    ("uniqueID", "postscriptUniqueID"),
    ("ttVendor", "openTypeOS2VendorID"),
    ("ttUniqueID", "openTypeNameUniqueID"),
    ("ttVersion", "openTypeNameVersion"),
];

/// `fontStyle` (a FontLab style code) → `styleMapStyleName`. Some UFO 1 files have 0.
const FONT_STYLES: [(i64, &str); 5] = [(64, "regular"), (1, "italic"), (32, "bold"), (33, "bold italic"), (0, "regular")];

/// `widthName` → `openTypeOS2WidthClass`, with the spellings FontLab and others wrote.
const WIDTH_NAMES: [(&str, i64); 13] = [
    ("Ultra-condensed", 1),
    ("Extra-condensed", 2),
    ("Condensed", 3),
    ("Semi-condensed", 4),
    ("Medium (normal)", 5),
    ("Semi-expanded", 6),
    ("Expanded", 7),
    ("Extra-expanded", 8),
    ("Ultra-expanded", 9),
    ("Normal", 5),
    ("All", 5),
    ("medium", 5),
    ("Medium", 5),
];

/// `msCharSet` (a Windows character set) → `postscriptWindowsCharacterSet`.
const CHAR_SETS: [(i64, i64); 20] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (77, 4),
    (128, 5),
    (129, 6),
    (130, 7),
    (134, 8),
    (136, 9),
    (161, 10),
    (162, 11),
    (163, 12),
    (177, 13),
    (178, 14),
    (186, 15),
    (200, 16),
    (204, 17),
    (222, 18),
    (238, 19),
    (255, 20),
];

/// The lib keys of a UFO 1's feature code.
const CLASSES: &str = "org.robofab.opentype.classes";
const FEATURES: &str = "org.robofab.opentype.features";
const FEATURE_ORDER: &str = "org.robofab.opentype.featureorder";
/// The lib key of a UFO 1's PostScript hint data.
const HINT_DATA: &str = "org.robofab.postScriptHintData";

/// Hint-data keys with a single value or a list of numbers, and their font-info keys.
const HINT_VALUES: [(&str, &str); 6] = [
    ("blueFuzz", "postscriptBlueFuzz"),
    ("blueScale", "postscriptBlueScale"),
    ("blueShift", "postscriptBlueShift"),
    ("forceBold", "postscriptForceBold"),
    ("hStems", "postscriptStemSnapH"),
    ("vStems", "postscriptStemSnapV"),
];
/// Hint-data keys holding lists of pairs, and their font-info keys.
const HINT_PAIRS: [(&str, &str); 4] = [
    ("blueValues", "postscriptBlueValues"),
    ("otherBlues", "postscriptOtherBlues"),
    ("familyBlues", "postscriptFamilyBlues"),
    ("familyOtherBlues", "postscriptFamilyOtherBlues"),
];

/// A finite number as an integer when it is whole, as ufoLib converts version 1 values.
fn whole(n: Num) -> Option<Number> {
    match n {
        Num::Int(i) => Some(Number::int(i)),
        Num::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 => {
            // Whole and well inside the range of i64, so the conversion is exact.
            #[allow(clippy::cast_possible_truncation)]
            let i = f as i64;
            Some(Number::int(i))
        }
        Num::Float(f) => Number::float(f),
    }
}

fn int(v: &Value) -> Option<i64> {
    match v.as_num().and_then(whole).map(|n| n.value()) {
        Some(Num::Int(i)) => Some(i),
        _ => None,
    }
}

/// The version 2 value of the version 1 key `key`, or `None` when it has no version 2 form.
fn info_value(key: &str, value: &Value) -> Option<Value> {
    let number = |n: i64| Some(Value::Number(Number::int(n)));
    match key {
        "fontStyle" => {
            let code = int(value)?;
            FONT_STYLES.iter().find(|(c, _)| *c == code).map(|(_, s)| Value::String(Str::from(*s)))
        }
        "widthName" => {
            let name = value.as_str()?;
            WIDTH_NAMES.iter().find(|(n, _)| *n == name).and_then(|(_, w)| number(*w))
        }
        "msCharSet" => {
            let set = int(value)?;
            CHAR_SETS.iter().find(|(s, _)| *s == set).and_then(|(_, c)| number(*c))
        }
        "weightValue" => match int(value)? {
            -1 => None,
            w => number(w),
        },
        // Version 3 has these non-negative, and ufoLib makes them so on reading.
        "versionMinor" => number(int(value)?.checked_abs()?),
        "unitsPerEm" => {
            let n = value.as_num()?.as_f64().abs();
            Number::float(n).and_then(|n| whole(n.value())).map(Value::Number)
        }
        _ => match value {
            Value::Number(n) => whole(n.value()).map(Value::Number),
            Value::Dict(_) | Value::Array(_) | Value::Data(_) => None,
            other => Some(other.clone()),
        },
    }
}

/// A UFO 1's `fontinfo.plist` in its version 2 form, keys sorted as fontTools writes them.
pub(crate) fn info(v1: &Dict) -> Dict {
    let mut out = Dict::new();
    for (old, new) in INFO_KEYS {
        // `Dict::get` takes the first of repeated keys, as the rest of the reader does.
        if let Some(v) = v1.get(old).and_then(|v| info_value(old, v)) {
            patch(&mut out, new, Some(v));
        }
    }
    out
}

/// The numbers of a list, or `None` when an item is not a finite number.
fn numbers(v: &Value) -> Option<Vec<Value>> {
    v.as_array()?.items.iter().map(|i| i.as_num().and_then(whole).map(Value::Number)).collect()
}

/// A list of pairs of numbers, flattened.
fn pairs(v: &Value) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    for pair in &v.as_array()?.items {
        let items = numbers(pair)?;
        if items.len() != 2 {
            return None;
        }
        out.extend(items);
    }
    Some(out)
}

/// Removes `key` from `d` wherever it occurs, returning its first value.
fn take(d: &mut Dict, key: &str) -> Option<Value> {
    let repeats = d.entries.iter().filter(|(k, _)| k.as_str() == key).count();
    if repeats <= 1 {
        let i = d.entries.iter().position(|(k, _)| k.as_str() == key)?;
        return d.remove_at(i).map(|(_, v)| v);
    }
    // A key repeated in malformed input: one pass over the entries, so the work stays linear.
    // Each entry has two gaps (before its key and before its value), then one before the end.
    let gaps = d.spacing.gaps(d.entries.len() * 2).map(<[Box<str>]>::to_vec);
    let mut kept_gaps = Vec::new();
    let mut first = None;
    for (i, (k, v)) in std::mem::take(&mut d.entries).into_iter().enumerate() {
        if k.as_str() == key {
            if first.is_none() {
                first = Some(v);
            }
            continue;
        }
        if let Some(g) = &gaps {
            kept_gaps.extend(g.get(i * 2..i * 2 + 2).unwrap_or_default().iter().map(ToString::to_string));
        }
        d.entries.push((k, v));
    }
    if let Some(g) = gaps {
        kept_gaps.extend(g.last().map(ToString::to_string));
        d.spacing = Spacing::new(kept_gaps);
    }
    first
}

/// Takes the data a UFO 1 kept in its lib out of `lib`: the PostScript hint data goes into
/// `info` (version 2 keys, replacing what is there), and the feature code is returned when there
/// is any.
pub(crate) fn take_lib_data(lib: &mut Dict, info: &mut Dict) -> Option<String> {
    let classes = take(lib, CLASSES);
    let split = take(lib, FEATURES);
    let order = take(lib, FEATURE_ORDER);
    let hints = take(lib, HINT_DATA);

    let mut parts: Vec<&str> = classes.as_ref().and_then(Value::as_str).into_iter().collect();
    if let Some(split) = split.as_ref().and_then(Value::as_dict) {
        let tags: Vec<&str> = match order.as_ref().and_then(Value::as_array) {
            Some(a) => a.items.iter().filter_map(Value::as_str).collect(),
            None => {
                let mut tags: Vec<&str> = split.entries.iter().map(|(k, _)| k.as_str()).collect();
                tags.sort_unstable();
                tags
            }
        };
        // A tag listed twice is written once, so the text stays as long as the lib's.
        let mut seen = HashSet::new();
        for tag in tags {
            if seen.insert(tag)
                && let Some(code) = split.get(tag).and_then(Value::as_str)
            {
                parts.push(code);
            }
        }
    }
    let features = parts.join("\n");

    if let Some(hints) = hints.as_ref().and_then(Value::as_dict) {
        for (from, to) in HINT_VALUES {
            let value = match hints.get(from) {
                Some(Value::Bool(b)) if from == "forceBold" => Some(Value::Bool(*b)),
                Some(v @ Value::Array(_)) if from.ends_with("Stems") => numbers(v).map(|items| Value::Array(Array { items, ..Default::default() })),
                Some(Value::Number(n)) if !from.ends_with("Stems") && from != "forceBold" => whole(n.value()).map(Value::Number),
                _ => None,
            };
            if value.is_some() {
                patch(info, to, value);
            }
        }
        for (from, to) in HINT_PAIRS {
            if let Some(items) = hints.get(from).and_then(pairs) {
                patch(info, to, Some(Value::Array(Array { items, ..Default::default() })));
            }
        }
    }
    (!features.is_empty()).then_some(features)
}

/// The `layercontents.plist` a UFO 3 form of a UFO 1 has: its one layer, the default.
pub(crate) fn layer_contents() -> Value {
    let pair = vec![Value::String(Str::from("public.default")), Value::String(Str::from("glyphs"))];
    Value::Array(Array { items: vec![Value::Array(Array { items: pair, ..Default::default() })], ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(entries: &[(&str, Value)]) -> Dict {
        let mut d = Dict::new();
        for (k, v) in entries {
            d.insert(*k, v.clone());
        }
        d
    }

    fn n(i: i64) -> Value {
        Value::Number(Number::int(i))
    }

    fn f(x: f64) -> Value {
        Value::Number(Number::float(x).unwrap())
    }

    fn s(t: &str) -> Value {
        Value::String(Str::from(t))
    }

    #[test]
    fn version_1_font_info_gets_version_2_names_and_values() {
        let v1 = dict(&[
            ("menuName", s("Sample")),
            ("fontStyle", n(33)),
            ("widthName", s("Condensed")),
            ("msCharSet", n(0)),
            ("weightValue", n(-1)),
            ("fullName", s("Sample Bold Italic")),
            ("notice", s("A description")),
            ("ttVendor", s("NONE")),
            ("unitsPerEm", f(1000.0)),
            ("versionMinor", n(-3)),
            ("slantAngle", f(-12.5)),
            ("unknownKey", s("dropped")),
        ]);
        let v2 = info(&v1);
        let keys: Vec<&str> = v2.entries.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "openTypeNameDescription",
                "openTypeOS2VendorID",
                "openTypeOS2WidthClass",
                "postscriptFullName",
                "postscriptSlantAngle",
                "postscriptWindowsCharacterSet",
                "styleMapFamilyName",
                "styleMapStyleName",
                "unitsPerEm",
                "versionMinor",
            ],
            "renamed, sorted; weightValue -1 and unknown keys dropped"
        );
        assert_eq!(v2.get("styleMapStyleName"), Some(&s("bold italic")));
        assert_eq!(v2.get("openTypeOS2WidthClass"), Some(&n(3)));
        assert_eq!(v2.get("postscriptWindowsCharacterSet"), Some(&n(1)));
        assert_eq!(v2.get("unitsPerEm").and_then(Value::as_num), Some(Num::Int(1000)), "a whole float becomes an integer");
        assert_eq!(v2.get("versionMinor"), Some(&n(3)));
        assert_eq!(v2.get("postscriptSlantAngle").and_then(Value::as_num), Some(Num::Float(-12.5)));
        // Values the tables do not know are dropped, not refused.
        let odd = info(&dict(&[("fontStyle", n(5)), ("widthName", s("Wide")), ("msCharSet", n(3)), ("familyName", s("Odd"))]));
        assert_eq!(odd.entries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["familyName"]);
    }

    #[test]
    fn lib_feature_code_and_hint_data_move_out_of_the_lib() {
        let features = Value::Dict(dict(&[("liga", s("feature liga {} liga;\n")), ("kern", s("feature kern {} kern;\n"))]));
        let order = Value::Array(Array { items: vec![s("liga"), s("missing"), s("kern"), s("liga")], ..Default::default() });
        let pair = |a: i64, b: i64| Value::Array(Array { items: vec![n(a), n(b)], ..Default::default() });
        let hints = Value::Dict(dict(&[
            ("blueValues", Value::Array(Array { items: vec![pair(-10, 0), pair(500, 510)], ..Default::default() })),
            ("hStems", Value::Array(Array { items: vec![n(80)], ..Default::default() })),
            ("blueScale", f(0.039625)),
            ("forceBold", Value::Bool(true)),
            ("otherBlues", s("not pairs")),
        ]));
        let mut lib = dict(&[
            (CLASSES, s("@UC = [A B];\n")),
            ("com.example.keep", n(1)),
            (FEATURES, features.clone()),
            (FEATURE_ORDER, order),
            (HINT_DATA, hints),
        ]);
        let mut info = dict(&[("postscriptBlueScale", f(0.5))]);
        let text = take_lib_data(&mut lib, &mut info);
        assert_eq!(text.as_deref(), Some("@UC = [A B];\n\nfeature liga {} liga;\n\nfeature kern {} kern;\n"));
        assert_eq!(lib.entries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["com.example.keep"]);
        let blues: Vec<Value> = [-10, 0, 500, 510].into_iter().map(n).collect();
        assert_eq!(info.get("postscriptBlueValues"), Some(&Value::Array(Array { items: blues, ..Default::default() })));
        assert_eq!(info.get("postscriptStemSnapH"), Some(&Value::Array(Array { items: vec![n(80)], ..Default::default() })));
        assert_eq!(info.get("postscriptBlueScale"), Some(&f(0.039625)), "the hint data replaces the font info");
        assert_eq!(info.get("postscriptForceBold"), Some(&Value::Bool(true)));
        assert_eq!(info.get("postscriptOtherBlues"), None, "malformed pairs are dropped");
        // Without an order, the features go by tag.
        let mut lib = dict(&[(FEATURES, features)]);
        assert_eq!(take_lib_data(&mut lib, &mut Dict::new()).as_deref(), Some("feature kern {} kern;\n\nfeature liga {} liga;\n"));
        assert!(lib.is_empty());
        assert_eq!(take_lib_data(&mut dict(&[("other", n(1))]), &mut Dict::new()), None);
    }

    #[test]
    fn a_repeated_lib_key_is_taken_in_one_pass_keeping_its_first_value() {
        let mut lib = Dict::new();
        lib.entries.push((Str::from("keep"), n(0)));
        for i in 0..200_000 {
            lib.entries.push((Str::from(CLASSES), s(if i == 0 { "@A = [A];\n" } else { "@B = [B];\n" })));
        }
        lib.entries.push((Str::from("other"), n(1)));
        assert_eq!(take(&mut lib, CLASSES), Some(s("@A = [A];\n")));
        assert_eq!(lib.entries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["keep", "other"]);
        // The whitespace of the entries that stay is kept with them.
        let mut lib = dict(&[("keep", n(0))]);
        for k in [CLASSES, CLASSES, "other"] {
            lib.entries.push((Str::from(k), n(1)));
        }
        lib.spacing = Spacing::new((0..9).map(|i| format!("g{i}")).collect());
        take(&mut lib, CLASSES);
        let gaps: Vec<&str> = lib.spacing.gaps(4).unwrap().iter().map(|g| &**g).collect();
        assert_eq!(gaps, ["g0", "g1", "g6", "g7", "g8"]);
    }
}

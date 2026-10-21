//! Value lists, syntaxes and ranges of the custom parameters the handbook's appendix describes
//! closely enough to check, and the warnings for values that name a master or a palette the font
//! does not have.
//!
//! [`crate::value::check`] runs [`choices`] and [`check`] after the kind check, so a refused value
//! changes nothing. A value list comes from the parameter row's `values` ([`ParamSpec::choices`]):
//! a choice matches in any case and is stored as the list spells it, a number when it is one.
//! [`warnings`] needs the font, so the commands run it on a value they store: the value is kept
//! and the warning returned. Values read from files are not checked when the file opens.
//!
//! Rows: `font.customParameters.add`, `.set`, `.describe` and the `[[parameter]]` rows of
//! `spec/inventory/18-custom-parameters.toml` that name a value list or a syntax (M7.21).

use std::collections::HashSet;

use fontcraft_model::Font;
use fontcraft_plist::{Array, Number, Str, Value};

use crate::catalog::ParamSpec;
use crate::value::{lines, number, per_script};

/// The checks a value can get beyond its kind.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rule {
    /// A whole number from the first to the second bound; a per-script parameter may put
    /// `script:` before it.
    Whole(i64, i64),
    /// A number above 0.
    Positive,
    /// A number of 0 or more.
    NonNegative,
    /// An angle in degrees above -90 and below 90 (per-script allowed).
    Angle,
    /// A percentage of inset from 0 to below 50 (per-script allowed).
    Inset,
    /// An OpenType tag: four printable ASCII characters, spaces only at the end.
    Tag,
    /// Feature tags, one per entry or separated by commas.
    TagList,
    /// `name; code`; with `true`, the name is a feature tag.
    NameAndCode(bool),
    VendorId,
    Panose,
    UnicodeRanges,
    CodePages,
    FsType,
    Gasp,
    Meta,
    Ros,
    OpticalSize,
    LocalInterpolation,
    Color,
    Folder,
    FileName,
    PostScriptName,
    NamePrefix,
    AxisLocation,
}

/// The parameters with a rule, by catalogue name.
const RULES: &[(&str, Rule)] = &[
    ("vendorID", Rule::VendorId),
    ("panose", Rule::Panose),
    ("unicodeRanges", Rule::UnicodeRanges),
    ("codePageRanges", Rule::CodePages),
    ("fsType", Rule::FsType),
    ("gasp Table", Rule::Gasp),
    ("meta Table", Rule::Meta),
    ("ROS", Rule::Ros),
    ("unitsPerEm", Rule::Whole(UPM_MIN, UPM_MAX)),
    ("Scale to UPM", Rule::Whole(UPM_MIN, UPM_MAX)),
    ("Elidable STAT Axis Value Name", Rule::Tag),
    ("Feature for Feature Variations", Rule::Tag),
    ("Remove Features", Rule::TagList),
    ("Add Class", Rule::NameAndCode(false)),
    ("Add Prefix", Rule::NameAndCode(false)),
    ("Add Feature", Rule::NameAndCode(true)),
    ("Replace Class", Rule::NameAndCode(false)),
    ("Replace Prefix", Rule::NameAndCode(false)),
    ("Replace Feature", Rule::NameAndCode(true)),
    ("Optical Size", Rule::OpticalSize),
    ("Local Interpolation", Rule::LocalInterpolation),
    ("Master Color", Rule::Color),
    ("Master Color Dark", Rule::Color),
    ("Master Background Color", Rule::Color),
    ("Master Background Color Dark", Rule::Color),
    ("Master Stroke Color", Rule::Color),
    ("Master Stroke Color Dark", Rule::Color),
    ("Color Palette for CPAL", Rule::Whole(0, U16_MAX)),
    ("Color Palette for SVG", Rule::Whole(0, U16_MAX)),
    ("SBIX to SVG", Rule::Whole(0, U16_MAX)),
    ("blueScale", Rule::Positive),
    ("blueShift", Rule::Whole(0, I16_MAX)),
    ("blueFuzz", Rule::Whole(0, I16_MAX)),
    ("TTFBlueFuzz", Rule::Whole(0, I16_MAX)),
    ("TTFFamilyZonesThreshold", Rule::Whole(0, U16_MAX)),
    ("TTFOvershootSuppressionBelowPPM", Rule::Whole(0, U16_MAX)),
    ("TTFMinimumDistance", Rule::NonNegative),
    ("TrueType Curve Error", Rule::Positive),
    ("Point To Unit Ratio", Rule::Positive),
    ("Grid Spacing", Rule::NonNegative),
    ("EditView Line Height", Rule::Positive),
    ("Export Folder", Rule::Folder),
    ("fileName", Rule::FileName),
    ("Variable Font File Name", Rule::FileName),
    ("postscriptFontName", Rule::PostScriptName),
    ("variationsPostScriptNamePrefix", Rule::NamePrefix),
    ("italicAngle", Rule::Angle),
    ("CJK Grid", Rule::Whole(1, MAX_GRID)),
    ("CJK Grid Horizontal", Rule::Whole(1, MAX_GRID)),
    ("CJK Grid Vertical", Rule::Whole(1, MAX_GRID)),
    ("CJK Guide", Rule::Inset),
    ("Axis Location", Rule::AxisLocation),
];

/// The parameters [`crate::value`] checks with a syntax of its own (Name Table Entry, the glyph
/// lists, Filter, versionString), counted by [`has_check`].
const VALUE_SYNTAXES: &[&str] = &[
    "Name Table Entry",
    "Prevent Name ID",
    "Remove Glyphs",
    "Keep Glyphs",
    "Export Glyphs",
    "Rename Glyphs",
    "Reencode Glyphs",
    "Filter",
    "PreFilter",
    "versionString",
];

/// Parameters whose value names masters, by id or by name.
const NAMES_MASTERS: &[&str] =
    &["Variable Font Origin", "Variation Font Origin", "Get Hints From Master", "Link Metrics With Master", "Disable Masters"];
/// Parameters whose value is an index into the font's Color Palettes.
const NAMES_A_PALETTE: &[&str] = &["Color Palette for CPAL", "Color Palette for SVG"];
/// The font parameter holding the colour palettes.
const PALETTES: &str = "Color Palettes";

const UPM_MIN: i64 = 16;
const UPM_MAX: i64 = 16_384;
const U16_MAX: i64 = 65_535;
const I16_MAX: i64 = 32_767;
/// Most rows or columns of a CJK grid.
const MAX_GRID: i64 = 100;
/// Highest Unicode range bit the OS/2 table defines; 123 to 127 are reserved.
const LAST_UNICODE_RANGE: i64 = 122;
/// Highest PANOSE digit: each category has at most sixteen values.
const LAST_PANOSE_DIGIT: i64 = 15;
/// Highest gasp behaviour flag set (four defined bits).
const LAST_GASP_FLAGS: i64 = 15;
/// Longest PostScript name, in bytes (name ID 6).
const MAX_POSTSCRIPT_NAME: usize = 63;
/// Longest variations PostScript name prefix, in bytes (name ID 25).
const MAX_NAME_PREFIX: usize = 63;
/// Longest language tag of a meta Table entry, in bytes.
const MAX_META_TAG: usize = 64;
/// Longest file or folder name part, in bytes.
const MAX_FILE_NAME: usize = 255;
/// Most warnings one value gives; the rest are counted in the last one.
const MAX_WARNINGS: usize = 10;

/// Windows code pages and their bits in `ulCodePageRange1` and `ulCodePageRange2`, as the
/// compiler maps them; a number below 64 is a bit itself.
const CODE_PAGES: &[i64] = &[
    1252, 1250, 1251, 1253, 1254, 1255, 1256, 1257, 1258, 874, 932, 936, 949, 950, 1361, 869, 866, 865, 864, 863, 862, 861, 860, 857, 855, 852, 775,
    737, 708, 850, 437,
];

fn rule(name: &str) -> Option<Rule> {
    RULES.iter().find(|(n, _)| *n == name).map(|(_, r)| *r)
}

/// Whether a value of `spec` gets a check beyond its kind: a value list, a syntax or a range.
pub fn has_check(spec: &ParamSpec) -> bool {
    !spec.choices.is_empty() || rule(spec.name).is_some() || VALUE_SYNTAXES.contains(&spec.name)
}

/// A number, or the number after a per-script prefix (`latin: 7`).
fn scalar(value: &Value) -> Option<f64> {
    number(value).or_else(|| value.as_str().and_then(per_script).map(|(_, n)| n))
}

/// A whole number from `min` to `max`.
fn whole_in(value: &Value, min: i64, max: i64) -> Option<i64> {
    let n = scalar(value)?;
    // Range-checked first, so the conversion is exact.
    (n.fract() == 0.0 && n >= min as f64 && n <= max as f64).then_some(n as i64)
}

/// The text of a text or number value.
fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.as_str().to_string()),
        Value::Number(_) => number(value).map(|n| n.to_string()),
        _ => None,
    }
}

/// The items of a list value (a single value counts as a list of one).
fn items(value: &Value) -> &[Value] {
    match value {
        Value::Array(a) => a.items.as_slice(),
        other => std::slice::from_ref(other),
    }
}

fn list_of(items: Vec<Value>) -> Value {
    Value::Array(Array { items, ..Array::default() })
}

/// The checked value of a parameter with a value list: each choice (every item of a list) is one
/// of [`ParamSpec::choices`], in any case, stored as the list spells it. Items that are not a text
/// or a number are left to the parameter's rule, and refused when it has none.
pub fn choices(spec: &ParamSpec, value: Value) -> Result<Value, String> {
    if spec.choices.is_empty() {
        return Ok(value);
    }
    let one_of = || format!("each value is one of {}", spec.choices.join(", "));
    let pick = |item: &Value| -> Result<Value, String> {
        let Some(text) = text_of(item) else {
            return if rule(spec.name).is_some() { Ok(item.clone()) } else { Err(one_of()) };
        };
        let text = text.trim();
        let found = spec.choices.iter().find(|c| c.eq_ignore_ascii_case(text)).ok_or_else(|| format!("`{text}` is not a choice: {}", one_of()))?;
        Ok(match found.parse::<i64>() {
            Ok(n) => Value::Number(Number::int(n)),
            Err(_) => Value::String(Str::new(*found)),
        })
    };
    match &value {
        Value::Array(a) => Ok(list_of(a.items.iter().map(pick).collect::<Result<_, _>>()?)),
        other => pick(other),
    }
}

/// The checked value of a parameter with a rule (whole numbers in text read as numbers), or why
/// it is refused. A parameter without a rule passes unchanged.
pub fn check(spec: &ParamSpec, value: Value) -> Result<Value, String> {
    let Some(rule) = rule(spec.name) else { return Ok(value) };
    let text = || text_of(&value).ok_or_else(|| "the value is a text".to_string());
    match rule {
        Rule::Whole(min, max) => {
            whole_in(&value, min, max).map(|_| value.clone()).ok_or_else(|| format!("the value is a whole number from {min} to {max}"))
        }
        Rule::Positive => scalar(&value).filter(|n| *n > 0.0).map(|_| value.clone()).ok_or_else(|| "the value is a number above 0".to_string()),
        Rule::NonNegative => {
            scalar(&value).filter(|n| *n >= 0.0).map(|_| value.clone()).ok_or_else(|| "the value is a number of 0 or more".to_string())
        }
        Rule::Angle => scalar(&value)
            .filter(|n| n.abs() < 90.0)
            .map(|_| value.clone())
            .ok_or_else(|| "the value is an angle in degrees, above -90 and below 90".to_string()),
        Rule::Inset => scalar(&value)
            .filter(|n| (0.0..50.0).contains(n))
            .map(|_| value.clone())
            .ok_or_else(|| "the value is a percentage from 0 to below 50".to_string()),
        Rule::Tag => tag(&text()?).map(|()| value),
        Rule::TagList => {
            for item in items(&value) {
                let line = text_of(item).ok_or("every entry is a feature tag")?;
                for piece in line.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    tag(piece)?;
                }
            }
            Ok(value)
        }
        Rule::NameAndCode(is_tag) => name_and_code(&text()?, is_tag).map(|()| value),
        Rule::VendorId => vendor_id(&text()?).map(|()| value),
        Rule::Panose => {
            let digits = whole_list(&value, 0, LAST_PANOSE_DIGIT, "a PANOSE digit")?;
            if items(&digits).len() == 10 { Ok(digits) } else { Err(format!("the value is ten PANOSE digits, not {}", items(&digits).len())) }
        }
        Rule::UnicodeRanges => whole_list(&value, 0, LAST_UNICODE_RANGE, "a Unicode range bit"),
        Rule::CodePages => code_pages(&value),
        Rule::FsType => fs_type(value),
        Rule::Gasp => gasp(value),
        Rule::Meta => meta(spec, &value).map(|()| value),
        Rule::Ros => {
            let t = text()?;
            fontcraft_model::cid::Ros::parse(&t)
                .map(|_| value)
                .ok_or_else(|| format!("`{t}` is not Registry-Ordering-Supplement, such as Adobe-Japan1-6"))
        }
        Rule::OpticalSize => optical_size(&text()?).map(|()| value),
        Rule::LocalInterpolation => local_interpolation(&text()?).map(|()| value),
        Rule::Color => color(&text()?).map(|()| value),
        Rule::Folder => folder(&text()?).map(|()| value),
        Rule::FileName => file_name(&text()?).map(|()| value),
        Rule::PostScriptName => postscript_name(&text()?).map(|()| value),
        Rule::NamePrefix => name_prefix(&text()?).map(|()| value),
        Rule::AxisLocation => axis_location(&value).map(|()| value),
    }
}

/// Every item a whole number from `min` to `max`, stored as numbers.
fn whole_list(value: &Value, min: i64, max: i64, what: &str) -> Result<Value, String> {
    let out = items(value)
        .iter()
        .map(|item| {
            whole_in(item, min, max).map(|n| Value::Number(Number::int(n))).ok_or_else(|| {
                let shown = text_of(item).unwrap_or_else(|| "an entry".into());
                format!("`{shown}` is not {what}: each entry is a whole number from {min} to {max}")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(list_of(out))
}

/// An OpenType tag: four printable ASCII characters, the first not a space, spaces only at the
/// end.
fn tag(text: &str) -> Result<(), String> {
    let b = text.as_bytes();
    let printable = b.iter().all(|c| (0x20..=0x7E).contains(c));
    let padded_at_end = b.iter().skip_while(|c| **c != b' ').all(|c| *c == b' ');
    if b.len() == 4 && printable && b.first() != Some(&b' ') && padded_at_end {
        Ok(())
    } else {
        Err(format!("`{text}` is not a tag: four ASCII letters, digits or symbols, such as `wght` or `liga`"))
    }
}

/// `name; code`: a name (a feature tag when `is_tag`) without spaces before the first semicolon.
fn name_and_code(text: &str, is_tag: bool) -> Result<(), String> {
    let (name, _) = text.split_once(';').ok_or("the value is a name, a semicolon and the code")?;
    let name = name.trim();
    if is_tag {
        return tag(name);
    }
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Err(format!("`{name}` is not a name: one word before the semicolon"));
    }
    Ok(())
}

/// One to four printable ASCII characters (a shorter id is padded with spaces), or nothing.
fn vendor_id(text: &str) -> Result<(), String> {
    if text.len() <= 4 && text.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        Ok(())
    } else {
        Err(format!("`{text}` is not a vendor id: at most four printable ASCII characters"))
    }
}

/// Code pages by number, bits 0 to 63, or `bit N`; numbers in text are stored as numbers.
fn code_pages(value: &Value) -> Result<Value, String> {
    let out = items(value)
        .iter()
        .map(|item| {
            let shown = text_of(item).unwrap_or_default();
            let bad = || format!("`{shown}` is not a code page: give its number (1252, 932…), a bit from 0 to 63, or `bit N`");
            if let Some(bit) = shown.trim().strip_prefix("bit ") {
                return bit.trim().parse::<i64>().ok().filter(|b| (0..64).contains(b)).map(|_| item.clone()).ok_or_else(bad);
            }
            let page = whole_in(item, 0, i64::from(u16::MAX)).ok_or_else(bad)?;
            if page < 64 || CODE_PAGES.contains(&page) { Ok(Value::Number(Number::int(page))) } else { Err(bad()) }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(list_of(out))
}

/// The embedding bits; the usage levels 1 (restricted), 2 (preview and print) and 3 (editable)
/// exclude each other. The bits themselves come from the row's value list.
fn fs_type(value: Value) -> Result<Value, String> {
    let bits = whole_list(&value, 0, 15, "an embedding bit")?;
    let levels = items(&bits).iter().filter_map(|b| whole_in(b, 1, 3)).collect::<HashSet<_>>();
    if levels.len() > 1 {
        return Err("only one usage level may be set: 1 (restricted), 2 (preview and print) or 3 (editable)".into());
    }
    Ok(bits)
}

/// Sizes in pixels per em (0 to 65535, each once) to behaviour flags (0 to 15); flags in text are
/// stored as numbers.
fn gasp(value: Value) -> Result<Value, String> {
    let Value::Dict(mut d) = value else { return Err("the value is a dictionary of sizes to flags".into()) };
    let mut seen = HashSet::with_capacity(d.entries.len());
    for (key, flags) in &mut d.entries {
        let size = key.as_str().trim().parse::<i64>().ok().filter(|s| (0..=U16_MAX).contains(s));
        let Some(size) = size else { return Err(format!("`{}` is not a size: a whole number from 0 to 65535", key.as_str())) };
        if !seen.insert(size) {
            return Err(format!("the size {size} is given twice"));
        }
        let bits =
            whole_in(flags, 0, LAST_GASP_FLAGS).ok_or_else(|| format!("the flags of size {size} are a whole number from 0 to {LAST_GASP_FLAGS}"))?;
        *flags = Value::Number(Number::int(bits));
    }
    Ok(Value::Dict(d))
}

/// Entries `{tag, data}`: the tag from the row's value list, the data language tags separated by
/// commas, each subtags of 1 to 8 letters or digits joined by hyphens.
fn meta(spec: &ParamSpec, value: &Value) -> Result<(), String> {
    for item in items(value) {
        let d = item.as_dict().ok_or("each entry is a dictionary with `tag` and `data`")?;
        let entry_tag = d.get("tag").and_then(Value::as_str).ok_or("each entry has a `tag`")?;
        if !spec.choices.contains(&entry_tag) {
            return Err(format!("`{entry_tag}` is not a meta tag: {}", spec.choices.join(" or ")));
        }
        let data = d.get("data").and_then(Value::as_str).ok_or("each entry has `data`, a text")?;
        for piece in data.split(',').map(str::trim) {
            let ok = !piece.is_empty()
                && piece.len() <= MAX_META_TAG
                && piece.split('-').all(|s| (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric()));
            if !ok {
                return Err(format!("`{piece}` is not a language tag such as `Latn`, `sr-Cyrl` or `sr-Cyrl-BA`"));
            }
        }
    }
    Ok(())
}

/// `design size; subfamily; range start; range end; menu name`, the four numbers whole decipoints
/// from 0 to 65535.
fn optical_size(text: &str) -> Result<(), String> {
    let parts: Vec<&str> = text.splitn(5, ';').map(str::trim).collect();
    let numbers_ok = parts.iter().take(4).all(|p| p.parse::<u16>().is_ok());
    if parts.len() == 5 && numbers_ok {
        Ok(())
    } else {
        Err("the value is five parts separated by semicolons: design size, subfamily, range start, range end (whole decipoints) and a menu name"
            .into())
    }
}

/// One number per axis, then `include:` and glyph names, all separated by semicolons.
fn local_interpolation(text: &str) -> Result<(), String> {
    let parts: Vec<&str> = text.split(';').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (last, values) = parts.split_last().ok_or("the value is axis values and `include:` with glyph names, separated by semicolons")?;
    let names = last.strip_prefix("include:").ok_or("the value ends with `include:` and the glyph names")?;
    if names.split(',').all(|n| n.trim().is_empty()) {
        return Err("`include:` names no glyph".into());
    }
    if values.is_empty() {
        return Err("the value starts with one interpolation value per axis".into());
    }
    for v in values {
        if !v.parse::<f64>().is_ok_and(f64::is_finite) {
            return Err(format!("`{v}` is not an interpolation value"));
        }
    }
    Ok(())
}

/// `R,G,B` or `R,G,B,A` with components from 0 to 255, or `#RRGGBB` or `#RRGGBBAA`.
fn color(text: &str) -> Result<(), String> {
    let text = text.trim();
    let ok = match text.strip_prefix('#') {
        Some(hex) => matches!(hex.len(), 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => {
            let parts: Vec<&str> = text.split(',').map(str::trim).collect();
            matches!(parts.len(), 3 | 4) && parts.iter().all(|p| p.parse::<f64>().is_ok_and(|n| (0.0..=255.0).contains(&n)))
        }
    };
    if ok { Ok(()) } else { Err(format!("`{text}` is not a colour: red, green, blue and an optional alpha from 0 to 255, separated by commas")) }
}

/// Characters no file or folder name may hold on the common systems.
fn bad_name_char(c: char) -> bool {
    c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// A file name without its extension: no slash, no reserved character, not `.` or `..`.
fn file_name(text: &str) -> Result<(), String> {
    let name = text.trim();
    let ok =
        !name.is_empty() && name.len() <= MAX_FILE_NAME && name != "." && name != ".." && !name.contains('/') && !name.chars().any(bad_name_char);
    if ok { Ok(()) } else { Err(format!("`{text}` is not a file name: no slashes, no \\ : * ? \" < > |, at most {MAX_FILE_NAME} bytes")) }
}

/// A folder inside the export destination: names separated by `/`, none empty, `.` or `..`.
fn folder(text: &str) -> Result<(), String> {
    let path = text.trim();
    if path.is_empty() || path.split('/').any(|part| file_name(part).is_err()) {
        return Err(format!("`{text}` is not a folder inside the export folder: names separated by `/`, none empty or `..`, no \\ : * ? \" < > |"));
    }
    Ok(())
}

/// A PostScript name: printable ASCII without spaces or `[](){}<>/%`, at most 63 bytes.
fn postscript_name(text: &str) -> Result<(), String> {
    let ok = !text.is_empty() && text.len() <= MAX_POSTSCRIPT_NAME && text.bytes().all(|b| b.is_ascii_graphic() && !b"[](){}<>/%".contains(&b));
    if ok {
        Ok(())
    } else {
        Err(format!("`{text}` is not a PostScript name: at most {MAX_POSTSCRIPT_NAME} ASCII characters, no spaces and none of [](){{}}<>/%"))
    }
}

/// A variations PostScript name prefix: ASCII letters and digits, at most 63.
fn name_prefix(text: &str) -> Result<(), String> {
    if !text.is_empty() && text.len() <= MAX_NAME_PREFIX && text.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(format!("`{text}` is not a name prefix: A to Z, a to z and 0 to 9 only, at most {MAX_NAME_PREFIX}"))
    }
}

/// Entries `{Axis, Location}`: an axis name, each once, and a number.
fn axis_location(value: &Value) -> Result<(), String> {
    let mut seen = HashSet::new();
    for item in items(value) {
        let d = item.as_dict().ok_or("each entry is a dictionary with `Axis` and `Location`")?;
        let axis = d.get("Axis").and_then(Value::as_str).filter(|a| !a.trim().is_empty()).ok_or("each entry names its `Axis`")?;
        if !seen.insert(axis) {
            return Err(format!("the axis `{axis}` is given twice"));
        }
        d.get("Location").and_then(number).ok_or_else(|| format!("the `Location` on `{axis}` is a number"))?;
    }
    Ok(())
}

/// Warnings for a stored value of `spec` that names a master or a palette `font` does not have.
/// The value stays as it is: the master or palette may be added later.
pub fn warnings(font: &Font, spec: &ParamSpec, value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if NAMES_MASTERS.contains(&spec.name) {
        let known: HashSet<&str> = font.masters.iter().flat_map(|m| [m.id.as_str(), m.name.as_str()]).collect();
        let missing: Vec<&str> =
            lines(value).unwrap_or_default().into_iter().map(str::trim).filter(|n| !n.is_empty() && !known.contains(n)).collect();
        for name in missing.iter().take(MAX_WARNINGS) {
            out.push(format!("{}: `{name}` names no master of the font; the value is kept", spec.name));
        }
        if missing.len() > MAX_WARNINGS {
            out.push(format!("{}: {} more names name no master", spec.name, missing.len() - MAX_WARNINGS));
        }
    }
    if NAMES_A_PALETTE.contains(&spec.name)
        && let Some(index) = whole_in(value, 0, i64::MAX)
    {
        let count = font.info.params.get(PALETTES).and_then(Value::as_array).map_or(0, |a| a.items.len());
        if !usize::try_from(index).is_ok_and(|i| i < count) {
            out.push(format!("{}: palette {index} is past the {count} palette(s) of {PALETTES}; the value is kept", spec.name));
        }
    }
    out
}

/// Every catalogue entry with a check beyond its kind, by name.
pub fn checked() -> Vec<&'static str> {
    crate::catalog::all().iter().filter(|p| has_check(p)).map(|p| p.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use crate::value::{check as check_value, from_json, to_json};
    use serde_json::{Value as Json, json};

    fn checked_json(name: &str, j: &Json) -> Result<Json, String> {
        let spec = catalog::lookup(name).ok_or_else(|| format!("{name} is not in the catalogue"))?;
        let value = from_json(spec.name, j).map_err(|e| e.to_string())?;
        check_value(spec, &value).map(|v| to_json(&v)).map_err(|e| e.to_string())
    }

    /// One allowed and one refused value per parameter with a rule or a value list.
    fn cases() -> Vec<(&'static str, Json, Json)> {
        vec![
            ("vendorID", json!("ABCD"), json!("ABCDE")),
            ("panose", json!([2, 0, 5, 3, 0, 0, 0, 2, 0, 4]), json!([2, 0, 5, 3, 0, 0, 0, 2, 0])),
            ("unicodeRanges", json!([0, 1, "2"]), json!([0, 125])),
            ("codePageRanges", json!([1252, "932", "bit 3"]), json!([1234])),
            ("fsType", json!([3, 8]), json!([1, 2])),
            ("gasp Table", json!({"8": 10, "65535": "15"}), json!({"8": 16})),
            ("meta Table", json!([{"tag": "dlng", "data": "Latn, sr-Cyrl-BA"}]), json!([{"tag": "xyzw", "data": "Latn"}])),
            ("ROS", json!("Adobe-Japan1-6"), json!("Adobe Japan1")),
            ("unitsPerEm", json!(2048), json!(20_000)),
            ("Scale to UPM", json!(1000), json!(8)),
            ("Post Table Type", json!(3), json!(1)),
            ("Webfont Formats", json!(["woff2", "TTF"]), json!(["OTF"])),
            ("Elidable STAT Axis Value Name", json!("wght"), json!("weight")),
            ("Feature for Feature Variations", json!("rvrn"), json!("rv")),
            ("Remove Features", json!(["liga", "calt, ss01"]), json!(["ligatures"])),
            ("Add Class", json!("caps; A B C"), json!("small caps; A")),
            ("Add Prefix", json!("Languages; languagesystem DFLT dflt;"), json!("no semicolon")),
            ("Add Feature", json!("ss01; sub a by a.ss01;"), json!("stylistic; sub a by b;")),
            ("Replace Class", json!("@caps; A B"), json!("; A")),
            ("Replace Prefix", json!("Languages; languagesystem DFLT dflt;"), json!("Languages")),
            ("Replace Feature", json!("liga; sub f i by fi;"), json!("ligature; sub f i by fi;")),
            ("Optical Size", json!("100; 1; 69; 120; Ten"), json!("100; 1; 69; Ten")),
            ("Local Interpolation", json!("120; include: a, g, s"), json!("120; a, g")),
            ("Master Color", json!("255,0,0,128"), json!("300,0,0")),
            ("Master Color Dark", json!("#00FF00"), json!("#00FF0")),
            ("Master Background Color", json!("250, 250, 250"), json!("white")),
            ("Master Background Color Dark", json!("20,20,20,255"), json!("1,2")),
            ("Master Stroke Color", json!("#112233AA"), json!("1,2,3,4,5")),
            ("Master Stroke Color Dark", json!("0,0,0"), json!("-1,0,0")),
            ("Color Palette for CPAL", json!(1), json!(-1)),
            ("Color Palette for SVG", json!(0), json!(1.5)),
            ("SBIX to SVG", json!(72), json!(70_000)),
            ("blueScale", json!(0.037), json!(0)),
            ("blueShift", json!(7), json!(-1)),
            ("blueFuzz", json!(0), json!(0.5)),
            ("TTFBlueFuzz", json!(1), json!(-2)),
            ("TTFFamilyZonesThreshold", json!(20), json!(-20)),
            ("TTFOvershootSuppressionBelowPPM", json!(30), json!(100_000)),
            ("TTFMinimumDistance", json!(0.25), json!(-0.25)),
            ("TrueType Curve Error", json!(0.6), json!(0)),
            ("Point To Unit Ratio", json!(1), json!(-1)),
            ("Grid Spacing", json!(0.5), json!(-1)),
            ("EditView Line Height", json!(1200), json!(0)),
            ("Export Folder", json!("Trial Versions/Webfonts"), json!("../Elsewhere")),
            ("fileName", json!("MyFont-Bold"), json!("My/Font")),
            ("Variable Font File Name", json!("MyFontVF"), json!("VF:1")),
            ("postscriptFontName", json!("MyFont-BoldCdIt"), json!("My Font")),
            ("variationsPostScriptNamePrefix", json!("MyFontVar"), json!("My-Font")),
            ("italicAngle", json!("latin: 7"), json!(95)),
            ("CJK Grid", json!("han: 4"), json!(0)),
            ("CJK Grid Horizontal", json!(8), json!(1000)),
            ("CJK Grid Vertical", json!(8), json!(-8)),
            ("CJK Guide", json!("kana:5"), json!(60)),
            ("Axis Location", json!([{"Axis": "Weight", "Location": 400}]), json!([{"Axis": "Weight", "Location": "heavy"}])),
        ]
    }

    #[test]
    fn every_check_allows_one_value_and_refuses_another() {
        let cases = cases();
        for (name, good, bad) in &cases {
            assert!(checked_json(name, good).is_ok(), "{name} refused {good}: {:?}", checked_json(name, good));
            assert!(checked_json(name, bad).is_err(), "{name} allowed {bad}");
        }
        // Every rule and every value list has a case.
        for p in catalog::all().iter().filter(|p| rule(p.name).is_some() || !p.choices.is_empty()) {
            assert!(cases.iter().any(|(n, _, _)| *n == p.name), "{} has no case", p.name);
        }
        for (name, _) in RULES {
            assert!(catalog::lookup(name).is_some_and(|p| p.name == *name), "{name} is not a catalogue name");
        }
        assert_eq!((UPM_MIN, UPM_MAX), (i64::from(*crate::UPM_RANGE.start()), i64::from(*crate::UPM_RANGE.end())));
        // The measure of the package: at least 40 parameters check more than their kind.
        assert!(checked().len() >= 40, "{}", checked().len());
    }

    #[test]
    fn choices_and_numbers_are_stored_as_the_catalogue_spells_them() {
        assert_eq!(checked_json("Webfont Formats", &json!("woff2\nttf")), Ok(json!(["WOFF2", "TTF"])));
        assert_eq!(checked_json("fsType", &json!(["8"])), Ok(json!([8])));
        assert_eq!(checked_json("Post Table Type", &json!("2")), Ok(json!(2)));
        assert_eq!(checked_json("panose", &json!("2\n0\n5\n3\n0\n0\n0\n2\n0\n4")), Ok(json!([2, 0, 5, 3, 0, 0, 0, 2, 0, 4])));
        assert_eq!(checked_json("gasp Table", &json!({"8": "10"})), Ok(json!({"8": 10})));
        assert_eq!(checked_json("codePageRanges", &json!(["1252", "bit 3"])), Ok(json!([1252, "bit 3"])));
        let meta = catalog::lookup("meta Table").map(|p| p.choices);
        assert_eq!(meta, Some(&["dlng", "slng"][..]));
    }

    #[test]
    fn values_naming_a_missing_master_or_palette_warn() {
        let mut font = Font::new("Warned");
        let (id, name) = font.masters.first().map(|m| (m.id.clone(), m.name.clone())).unwrap_or_default();
        let spec = |n: &str| *catalog::lookup(n).unwrap();
        let text = |s: &str| Value::String(Str::new(s));
        assert!(warnings(&font, &spec("Variable Font Origin"), &text(&id)).is_empty());
        assert!(warnings(&font, &spec("Get Hints From Master"), &text(&name)).is_empty());
        let w = warnings(&font, &spec("Link Metrics With Master"), &text("Nowhere"));
        assert!(w.len() == 1 && w[0].contains("Nowhere"), "{w:?}");
        let many = list_of((0..30).map(|i| text(&format!("m{i}"))).collect());
        assert_eq!(warnings(&font, &spec("Disable Masters"), &many).len(), MAX_WARNINGS + 1);
        let index = Value::Number(Number::int(1));
        assert_eq!(warnings(&font, &spec("Color Palette for CPAL"), &index).len(), 1);
        let palettes = list_of(vec![list_of(vec![text("255,0,0,255")]), list_of(vec![text("0,0,255,255")])]);
        std::sync::Arc::make_mut(&mut font.info).params.set(PALETTES, palettes);
        assert!(warnings(&font, &spec("Color Palette for CPAL"), &index).is_empty());
        assert!(warnings(&font, &spec("typoAscender"), &text("Nowhere")).is_empty());
    }
}

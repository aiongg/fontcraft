//! `Color Palettes` in the format versions that have no dictionary form for it (M13.65b).
//!
//! Version 4 stores the parameter as `{names; palettes = ({colors; name; backgroundStyle}…)}`
//! with components from 0 to 1 (GlyphsFileFormatv4.md); versions 2 and 3 document only lists of
//! colours with components from 0 to 255. Written as version 2 or 3, a dictionary becomes the
//! version 3 list form (version 2 then spells each colour as `"r,g,b,a"`), keeping the number of
//! components of each colour, and [`losses`] names what the lists cannot hold: palette names,
//! background styles, colour names and any other key. A dictionary whose colours do not convert
//! is written as read, and [`losses`] says so.

use std::borrow::Cow;

use fontcraft_model::{Font, Params};
use fontcraft_plist::{Array, Layout, Number, Value};

use crate::list;

const PARAM: &str = "Color Palettes";
/// The key of a palette's colours, the one part of the dictionary the lists keep.
const COLORS: &str = "colors";

/// `params` as format `version` holds them: below version 4 every `Color Palettes` dictionary as
/// the version 3 lists, and in version 2 every colour list as a `"r,g,b,a"` string.
pub(crate) fn for_version(params: &Params, version: u32) -> Cow<'_, Params> {
    if version >= 4 || !params.0.iter().any(|p| p.name == PARAM) {
        return Cow::Borrowed(params);
    }
    let mut out = params.clone();
    for p in out.0.iter_mut().filter(|p| p.name == PARAM) {
        if let Some(v) = to_lists(&p.value) {
            p.value = v;
        }
        if version == 2
            && let Some(v) = crate::v2::palettes_v2(&p.value)
        {
            p.value = v;
        }
    }
    Cow::Owned(out)
}

/// The version 3 lists of a version 4 dictionary; `None` when `v` is not a dictionary or a colour
/// is not a list of numbers from 0 to 1.
fn to_lists(v: &Value) -> Option<Value> {
    let palettes = v.as_dict()?.get("palettes")?.as_array()?;
    let mut out = Vec::new();
    for palette in &palettes.items {
        let colours = palette.as_dict()?.get(COLORS)?.as_array()?;
        let mut row = Vec::new();
        for c in &colours.items {
            let items = c.as_array()?.items.iter().map(|n| byte(n.as_num()?.as_f64())).collect::<Option<Vec<_>>>()?;
            row.push(Value::Array(Array { items, layout: Layout::Compact, ..Array::default() }));
        }
        out.push(list(row));
    }
    Some(list(out))
}

/// A component from 0 to 1 as an integer from 0 to 255.
fn byte(unit: f64) -> Option<Value> {
    if !unit.is_finite() || !(0.0..=1.0).contains(&unit) {
        return None;
    }
    // Range-checked, so the conversion is exact.
    #[allow(clippy::cast_possible_truncation)]
    let b = (unit * 255.0).round() as i64;
    Some(Value::Number(Number::int(b)))
}

/// What writing format `version` leaves out of the `Color Palettes` dictionaries of the font and
/// its masters, one line each; nothing for version 4.
pub(crate) fn losses(font: &Font, version: u32) -> Vec<String> {
    if version >= 4 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let font_level = font.info.params.0.iter().map(|p| (String::new(), p));
    let masters = font.masters.iter().flat_map(|m| {
        let name = if m.name.is_empty() { &m.id } else { &m.name };
        m.params.0.iter().map(move |p| (format!("master `{name}`: "), p))
    });
    for (at, p) in font_level.chain(masters).filter(|(_, p)| p.name == PARAM && p.value.as_dict().is_some()) {
        if to_lists(&p.value).is_none() {
            out.push(format!("{at}parameter `{PARAM}`: a dictionary whose colours format version {version} cannot list, written as read"));
            continue;
        }
        let parts = dropped(&p.value);
        if !parts.is_empty() {
            out.push(format!("{at}parameter `{PARAM}`: {} (format version {version} stores only the colours)", parts.join(", ")));
        }
    }
    out
}

/// What the lists leave out of a dictionary value: its palette names, background styles, colour
/// names and other keys, with their counts.
fn dropped(v: &Value) -> Vec<String> {
    let Some(top) = v.as_dict() else { return Vec::new() };
    let palettes: &[Value] = top.get("palettes").and_then(Value::as_array).map_or(&[], |a| a.items.as_slice());
    let filled = |key: &str| {
        palettes
            .iter()
            .filter_map(Value::as_dict)
            .filter(|d| match d.get(key) {
                Some(Value::String(s)) => !s.as_str().trim().is_empty(),
                Some(Value::Array(a)) => !a.items.is_empty(),
                Some(Value::Dict(d)) => !d.entries.is_empty(),
                Some(_) => true,
                None => false,
            })
            .count()
    };
    let mut out = Vec::new();
    let named = filled("name");
    if named > 0 {
        out.push(format!("palette names ({named})"));
    }
    let styled = filled("backgroundStyle");
    if styled > 0 {
        out.push(format!("palettes with background styles ({styled})"));
    }
    let colour_names = top.get("names").and_then(Value::as_dict).map_or(0, |d| d.entries.len());
    if colour_names > 0 {
        out.push(format!("colour names ({colour_names})"));
    }
    let known = |k: &str| matches!(k, "name" | "backgroundStyle" | COLORS);
    let other_palette_keys = palettes.iter().filter_map(Value::as_dict).any(|d| d.entries.iter().any(|(k, _)| !known(k.as_str())));
    let other_top_keys = top.entries.iter().any(|(k, _)| !matches!(k.as_str(), "names" | "palettes"));
    if other_palette_keys || other_top_keys {
        out.push("other keys".to_string());
    }
    out
}

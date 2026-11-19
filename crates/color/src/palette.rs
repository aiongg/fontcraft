//! Colour palettes (M11.2): the `Color Palettes` parameter as typed data, a master's own palettes,
//! and the edits the palette commands make.
//!
//! Rows: `colorPalette.*`, `color.palette.fontLevelParameter`, `color.palette.indexFromZero`,
//! `color.palette.multiplePalettes`, `color.palette.perMaster`, `Color Palettes`.
//!
//! Contract (`docs/plans/M11.md` § 3, § 4 › M11.2):
//! - Every palette has the same number of rows ([`Palettes::rows`]); [`Palettes::new`] refuses a
//!   ragged table, and every edit keeps the invariant.
//! - Rows count from 0. Row 65,535 is the text colour and is never stored.
//! - [`parse`] reads the three stored forms ([`Form`]): version 2 lists of `"r,g,b,a"` strings,
//!   version 3 lists of number lists from 0 to 255, and the version 4 dictionary
//!   `{names; palettes = ({colors; name; backgroundStyle}…)}` with components from 0 to 1. A colour
//!   has two components (grey, alpha), four (red, green, blue, alpha) or five (cyan, magenta,
//!   yellow, black, alpha). A component out of range is refused, not clamped.
//! - [`read`]: a master's own `Color Palettes` wins over the font's for that master.
//! - A new row is black in every palette; a new palette copies palette 0; the last palette cannot
//!   be removed; a removed row renumbers the rows after it (the engine renumbers the layers in
//!   the same undo step and refuses while a layer uses the row).
//! - [`store`] writes palettes back in the form the parameter already had, keeping a version 4
//!   value's colour names and palette names in step with the edit. [`to_param`] writes the
//!   version 3 form, which the version 2 and UFO writers convert from.
//! - [`palette_info`] and [`color_names`] read what only the version 4 form stores: palette names,
//!   background styles (`light`, `dark`) and colour names, which `CPAL` version 1 writes as
//!   palette types and labels. [`store_dark`] stores a palette for dark backgrounds in that form.
//! - Pure functions; an edit that fails returns an error and changes nothing.

use fontcraft_model::plist::{Array, Dict, Layout, Num, Number, Str, Value};
use fontcraft_model::{Font, Param};

use crate::{ColorError, MAX_PALETTE_ROWS, MAX_PALETTES};

/// The parameter's name, on the font or on a master.
pub const COLOR_PALETTES_PARAM: &str = "Color Palettes";
/// The palette index `COLR` and the palette layers use for the text colour.
pub const FOREGROUND_INDEX: u16 = 0xFFFF;

/// A palette colour: red, green, blue and alpha, 0–255 each (what `CPAL` stores).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// The colour of a new row.
    pub const BLACK: Rgba = Rgba { r: 0, g: 0, b: 0, a: 255 };

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `#RRGGBBAA`.
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
    }

    /// `#RRGGBB` (opaque) or `#RRGGBBAA`, either case; `None` for anything else.
    pub fn parse_hex(text: &str) -> Option<Rgba> {
        let digits = text.strip_prefix('#')?;
        // ASCII only, so every two-byte slice below falls on character boundaries.
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| digits.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
        match digits.len() {
            6 => Some(Rgba::new(byte(0)?, byte(2)?, byte(4)?, 255)),
            8 => Some(Rgba::new(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        }
    }

    /// The colour as components from 0 to 1, for drawing.
    pub fn to_f64(&self) -> [f64; 4] {
        [self.r, self.g, self.b, self.a].map(|c| f64::from(c) / 255.0)
    }
}

/// A font's or a master's palettes: `palettes()[p][row]`. Every palette has [`Palettes::rows`]
/// entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Palettes {
    rows: usize,
    palettes: Vec<Vec<Rgba>>,
}

impl Palettes {
    /// Palettes from a table, refused when ragged or beyond [`MAX_PALETTES`] or
    /// [`MAX_PALETTE_ROWS`]. No palettes at all is the empty value.
    pub fn new(palettes: Vec<Vec<Rgba>>) -> Result<Palettes, ColorError> {
        if palettes.len() > MAX_PALETTES {
            return Err(ColorError::LimitExceeded { what: "palettes", limit: MAX_PALETTES });
        }
        let rows = palettes.first().map_or(0, Vec::len);
        if rows > MAX_PALETTE_ROWS {
            return Err(ColorError::LimitExceeded { what: "colours in a palette", limit: MAX_PALETTE_ROWS });
        }
        if palettes.iter().any(|p| p.len() != rows) {
            return Err(ColorError::BadPalettes("the palettes have different numbers of colours".into()));
        }
        Ok(Palettes { rows, palettes })
    }

    /// The number of colours in every palette.
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn palettes(&self) -> &[Vec<Rgba>] {
        &self.palettes
    }

    pub fn palette(&self, index: usize) -> Option<&[Rgba]> {
        self.palettes.get(index).map(Vec::as_slice)
    }

    pub fn get(&self, palette: usize, row: usize) -> Option<Rgba> {
        self.palettes.get(palette)?.get(row).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.palettes.is_empty()
    }

    /// A row of `color` appended to every palette (Add Color). With no palette yet, the first
    /// palette is made.
    pub fn add_row(&self, color: Rgba) -> Result<Palettes, ColorError> {
        self.add_row_with(&vec![color; self.palettes.len().max(1)])
    }

    /// A row appended with `colors[p]` in palette `p`: one colour for each palette, or one colour
    /// when there is no palette yet, which makes the first palette.
    pub fn add_row_with(&self, colors: &[Rgba]) -> Result<Palettes, ColorError> {
        let wanted = self.palettes.len().max(1);
        if colors.len() != wanted {
            return Err(ColorError::BadPalettes(format!("{} colours given for {wanted} palettes", colors.len())));
        }
        if self.palettes.is_empty() {
            return Ok(Palettes { rows: 1, palettes: vec![colors.to_vec()] });
        }
        if self.rows >= MAX_PALETTE_ROWS {
            return Err(ColorError::LimitExceeded { what: "colours in a palette", limit: MAX_PALETTE_ROWS });
        }
        let mut next = self.clone();
        for (p, c) in next.palettes.iter_mut().zip(colors) {
            p.push(*c);
        }
        next.rows += 1;
        Ok(next)
    }

    /// Row `row` removed from every palette; the rows after it move down by one.
    pub fn remove_row(&self, row: usize) -> Result<Palettes, ColorError> {
        if row >= self.rows {
            return Err(ColorError::NoSuchRow(row));
        }
        let mut next = self.clone();
        for p in &mut next.palettes {
            if row < p.len() {
                p.remove(row);
            }
        }
        next.rows -= 1;
        Ok(next)
    }

    /// Colour `row` of palette `palette` set to `color`.
    pub fn set(&self, palette: usize, row: usize, color: Rgba) -> Result<Palettes, ColorError> {
        let mut next = self.clone();
        let p = next.palettes.get_mut(palette).ok_or(ColorError::NoSuchPalette(palette))?;
        let slot = p.get_mut(row).ok_or(ColorError::NoSuchRow(row))?;
        *slot = color;
        Ok(next)
    }

    /// A copy of palette 0 appended. With no palette yet, one palette with no colours.
    pub fn add_palette(&self) -> Result<Palettes, ColorError> {
        if self.palettes.is_empty() {
            return Ok(Palettes { rows: 0, palettes: vec![Vec::new()] });
        }
        self.add_palette_from(0)
    }

    /// A copy of palette `source` appended.
    pub fn add_palette_from(&self, source: usize) -> Result<Palettes, ColorError> {
        let copy = self.palettes.get(source).ok_or(ColorError::NoSuchPalette(source))?.clone();
        if self.palettes.len() >= MAX_PALETTES {
            return Err(ColorError::LimitExceeded { what: "palettes", limit: MAX_PALETTES });
        }
        let mut next = self.clone();
        next.palettes.push(copy);
        Ok(next)
    }

    /// `colours` appended as a new palette, one colour per row.
    pub fn add_palette_with(&self, colours: Vec<Rgba>) -> Result<Palettes, ColorError> {
        if self.palettes.len() >= MAX_PALETTES {
            return Err(ColorError::LimitExceeded { what: "palettes", limit: MAX_PALETTES });
        }
        Palettes::new(self.palettes.iter().cloned().chain([colours]).collect())
    }

    /// Palette `index` removed; refused for the last one ([`ColorError::LastPalette`]).
    pub fn remove_palette(&self, index: usize) -> Result<Palettes, ColorError> {
        if index >= self.palettes.len() {
            return Err(ColorError::NoSuchPalette(index));
        }
        if self.palettes.len() == 1 {
            return Err(ColorError::LastPalette);
        }
        let mut next = self.clone();
        next.palettes.remove(index);
        Ok(next)
    }
}

/// Longest palette or colour name read from a version 4 value, in characters; a longer one is
/// cut.
pub const MAX_NAME_CHARS: usize = 256;
/// Most `backgroundStyle` entries of one palette that are read.
const MAX_STYLES: usize = 8;

/// What a version 4 value says about one palette besides its colours.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaletteInfo {
    /// The palette's name; `None` when it has none or an empty one.
    pub name: Option<String>,
    /// `backgroundStyle` holds `light`: the palette is meant for light backgrounds.
    pub light: bool,
    /// `backgroundStyle` holds `dark`: the palette is meant for dark backgrounds.
    pub dark: bool,
}

/// The name and background styles of every palette of `value`, in palette order. The version 2
/// and 3 forms store neither and give an empty list, as does a value that holds no palettes.
pub fn palette_info(value: &Value) -> Vec<PaletteInfo> {
    let Some(list) = value.as_dict().and_then(|d| d.get("palettes")).and_then(Value::as_array) else {
        return Vec::new();
    };
    list.items
        .iter()
        .take(MAX_PALETTES)
        .map(|p| {
            let d = p.as_dict();
            let name = d.and_then(|d| d.get("name")).and_then(Value::as_str).map(cut_name).filter(|n| !n.is_empty());
            let styles: Vec<&str> = match d.and_then(|d| d.get("backgroundStyle")) {
                Some(Value::Array(a)) => a.items.iter().take(MAX_STYLES).filter_map(Value::as_str).collect(),
                Some(Value::String(s)) => vec![s.as_str()],
                _ => Vec::new(),
            };
            PaletteInfo { name, light: styles.contains(&"light"), dark: styles.contains(&"dark") }
        })
        .collect()
}

/// The colour names of a version 4 value (`names`, keyed by row), one entry for each of the first
/// `rows` rows; a row without a name, and every row of the other forms, gives `None`.
pub fn color_names(value: &Value, rows: usize) -> Vec<Option<String>> {
    let mut out = vec![None; rows.min(MAX_PALETTE_ROWS)];
    if let Some(names) = value.as_dict().and_then(|d| d.get("names")).and_then(Value::as_dict) {
        for (key, v) in &names.entries {
            if let (Ok(row), Some(text)) = (key.as_str().trim().parse::<usize>(), v.as_str())
                && let Some(slot) = out.get_mut(row)
            {
                let text = cut_name(text);
                if !text.is_empty() {
                    *slot = Some(text);
                }
            }
        }
    }
    out
}

fn cut_name(text: &str) -> String {
    text.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// The `Color Palettes` value that stores `palettes`, whose last palette was just appended, with
/// that palette named `name` and marked for dark backgrounds. The value takes the version 4 form,
/// the only one that records background styles: a version 4 `previous` keeps its other keys, a
/// version 2 or 3 one is converted.
pub fn store_dark(previous: Option<&Value>, palettes: &Palettes, name: &str) -> Value {
    let v4 = previous.filter(|v| form(v) == Some(Form::V4));
    let mut value = store(v4, palettes, Change::Colors, Form::V4);
    if let Value::Dict(top) = &mut value
        && let Some(Value::Array(list)) = top.get_mut("palettes")
        && let Some(Value::Dict(last)) = list.items.last_mut()
    {
        last.insert("name", Value::String(Str::new(name)));
        last.insert("backgroundStyle", compact(vec![Value::String(Str::new("dark"))]));
    }
    value
}

/// How a `Color Palettes` value is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Lists of `"r,g,b,a"` strings, components 0–255.
    V2,
    /// Lists of number lists, components 0–255.
    V3,
    /// `{names; palettes = ({colors; name; backgroundStyle}…)}`, components 0–1.
    V4,
}

/// The form a stored value is in; `None` when it is neither a list nor a dictionary. A list
/// whose first colour is a string is version 2; any other list is version 3.
pub fn form(value: &Value) -> Option<Form> {
    match value {
        Value::Dict(_) => Some(Form::V4),
        Value::Array(a) => {
            let first = a.items.iter().filter_map(Value::as_array).find_map(|p| p.items.first());
            Some(if matches!(first, Some(Value::String(_))) { Form::V2 } else { Form::V3 })
        }
        _ => None,
    }
}

/// The form a new `Color Palettes` value takes in `font`: version 4 for a document read as
/// `.glyphs` version 4, otherwise version 3.
pub fn form_for(font: &Font) -> Form {
    if font.source.glyphs_version == Some(4) { Form::V4 } else { Form::V3 }
}

/// The `Color Palettes` value that applies to `master` (its own, else the font's), or the
/// font's when `master` is `None`. Disabled parameters are ignored.
pub fn stored<'a>(font: &'a Font, master: Option<&str>) -> Result<Option<&'a Value>, ColorError> {
    let own = match master {
        Some(id) => {
            let m = font.masters.iter().find(|m| m.id == id).ok_or_else(|| ColorError::MissingMaster(id.to_string()))?;
            m.params.get(COLOR_PALETTES_PARAM)
        }
        None => None,
    };
    Ok(own.or_else(|| font.info.params.get(COLOR_PALETTES_PARAM)))
}

/// The palettes that apply to `master` (its own `Color Palettes` when it has one, else the
/// font's), or the font's when `master` is `None`. No parameter: empty palettes.
pub fn read(font: &Font, master: Option<&str>) -> Result<Palettes, ColorError> {
    stored(font, master)?.map_or_else(|| Ok(Palettes::default()), parse)
}

/// A `Color Palettes` value in any of the three forms.
pub fn parse(value: &Value) -> Result<Palettes, ColorError> {
    let (lists, scale): (Vec<&[Value]>, f64) = match value {
        Value::Array(a) => {
            check_palette_count(a.items.len())?;
            let lists = a.items.iter().enumerate().map(|(i, p)| list(p, i)).collect::<Result<_, _>>()?;
            (lists, 255.0)
        }
        Value::Dict(d) => {
            let palettes = d.get("palettes").and_then(Value::as_array).ok_or_else(|| bad("the dictionary has no `palettes` list".into()))?;
            check_palette_count(palettes.items.len())?;
            let lists = palettes
                .items
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let colours = p.as_dict().and_then(|d| d.get("colors")).ok_or_else(|| bad(format!("palette {i} has no `colors` list")))?;
                    list(colours, i)
                })
                .collect::<Result<_, _>>()?;
            (lists, 1.0)
        }
        _ => return Err(bad("the value is neither a list of palettes nor a dictionary".into())),
    };
    let rows = lists.first().map_or(0, |l| l.len());
    let mut palettes = Vec::with_capacity(lists.len());
    for (p, colours) in lists.into_iter().enumerate() {
        if colours.len() != rows {
            return Err(bad(format!("palette {p} has {} colours, palette 0 has {rows}", colours.len())));
        }
        let parsed = colours
            .iter()
            .enumerate()
            .map(|(row, c)| {
                colour(c, scale).ok_or_else(|| bad(format!("palette {p}, colour {row} is not a colour with components from 0 to {scale}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        palettes.push(parsed);
    }
    Palettes::new(palettes)
}

fn bad(message: String) -> ColorError {
    ColorError::BadPalettes(message)
}

fn check_palette_count(n: usize) -> Result<(), ColorError> {
    if n > MAX_PALETTES { Err(ColorError::LimitExceeded { what: "palettes", limit: MAX_PALETTES }) } else { Ok(()) }
}

/// One palette's colours, its length checked before anything is read from it.
fn list(value: &Value, index: usize) -> Result<&[Value], ColorError> {
    let a = value.as_array().ok_or_else(|| bad(format!("palette {index} is not a list of colours")))?;
    if a.items.len() > MAX_PALETTE_ROWS {
        return Err(ColorError::LimitExceeded { what: "colours in a palette", limit: MAX_PALETTE_ROWS });
    }
    Ok(&a.items)
}

/// Most components a stored colour has (CMYK and alpha).
const MAX_COMPONENTS: usize = 5;

/// A stored colour whose components run from 0 to `scale`: a `"r,g,b,a"` string or a list of
/// numbers, with two, four or five components.
fn colour(value: &Value, scale: f64) -> Option<Rgba> {
    let parts: Vec<f64> = match value {
        Value::String(s) => {
            let text = s.as_str();
            if text.split(',').count() > MAX_COMPONENTS {
                return None;
            }
            text.split(',').map(|p| p.trim().parse::<f64>().ok()).collect::<Option<_>>()?
        }
        Value::Array(a) if a.items.len() <= MAX_COMPONENTS => a.items.iter().map(|v| v.as_num().map(Num::as_f64)).collect::<Option<_>>()?,
        _ => return None,
    };
    let unit: Vec<f64> = parts.iter().map(|v| (v.is_finite() && (0.0..=scale).contains(v)).then(|| v / scale)).collect::<Option<_>>()?;
    let [r, g, b, a] = match unit.as_slice() {
        [g, a] => [*g, *g, *g, *a],
        [r, g, b, a] => [*r, *g, *b, *a],
        [c, m, y, k, a] => [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k), *a],
        _ => return None,
    };
    Some(Rgba::new(byte(r), byte(g), byte(b), byte(a)))
}

/// A component from 0 to 1 as 0–255.
// Clamped to 0–255 and rounded, so the conversion is exact.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn byte(unit: f64) -> u8 {
    (unit * 255.0).round().clamp(0.0, 255.0) as u8
}

/// What an edit did, so that [`store`] keeps a version 4 value's names with their colours and
/// palettes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Colours changed, or a row or a palette was appended.
    Colors,
    /// The row was removed; the rows after it moved down by one.
    RemovedRow(usize),
    /// The palette was removed; the palettes after it moved down by one.
    RemovedPalette(usize),
}

/// The `Color Palettes` value that stores `palettes`, in the form of `previous` (the value
/// before the edit), or in `fallback` when there was none. A version 4 value keeps its other
/// keys: colour names follow their rows, palette names and background styles their palettes.
pub fn store(previous: Option<&Value>, palettes: &Palettes, change: Change, fallback: Form) -> Value {
    match previous.and_then(form).unwrap_or(fallback) {
        Form::V2 => list_value(palettes, |c| Value::String(Str::new(format!("{},{},{},{}", c.r, c.g, c.b, c.a)))),
        Form::V3 => list_value(palettes, |c| compact([c.r, c.g, c.b, c.a].into_iter().map(|v| Value::Number(Number::int(i64::from(v)))).collect())),
        Form::V4 => v4_value(previous.and_then(Value::as_dict), palettes, change),
    }
}

/// The `Color Palettes` parameter that stores `palettes`, in the version 3 form (lists of
/// integers 0–255).
pub fn to_param(palettes: &Palettes) -> Param {
    Param::new(COLOR_PALETTES_PARAM, store(None, palettes, Change::Colors, Form::V3))
}

fn multiline(items: Vec<Value>) -> Value {
    Value::Array(Array { items, ..Array::default() })
}

fn compact(items: Vec<Value>) -> Value {
    Value::Array(Array { items, layout: Layout::Compact, ..Array::default() })
}

fn list_value(palettes: &Palettes, colour: impl Fn(&Rgba) -> Value) -> Value {
    multiline(palettes.palettes.iter().map(|p| multiline(p.iter().map(&colour).collect())).collect())
}

/// A component as a number from 0 to 1, to four places: enough to give back the same byte.
fn unit_number(component: u8) -> Value {
    let v = (f64::from(component) / 255.0 * 10_000.0).round() / 10_000.0;
    Value::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
}

fn v4_value(previous: Option<&Dict>, palettes: &Palettes, change: Change) -> Value {
    let mut top = previous.cloned().unwrap_or_default();
    let old: &[Value] = previous.and_then(|d| d.get("palettes")).and_then(Value::as_array).map_or(&[], |a| a.items.as_slice());
    let list = palettes
        .palettes
        .iter()
        .enumerate()
        .map(|(i, colours)| {
            let from = match change {
                Change::RemovedPalette(r) if i >= r => i + 1,
                _ => i,
            };
            let mut d = old.get(from).and_then(Value::as_dict).cloned().unwrap_or_default();
            d.insert("colors", multiline(colours.iter().map(|c| compact([c.r, c.g, c.b, c.a].into_iter().map(unit_number).collect())).collect()));
            Value::Dict(d)
        })
        .collect();
    top.insert("palettes", multiline(list));
    let renamed = match (change, top.get("names")) {
        (Change::RemovedRow(row), Some(Value::Dict(names))) => {
            let mut kept = Dict::new();
            for (k, v) in &names.entries {
                match k.as_str().trim().parse::<usize>() {
                    Ok(i) if i == row => {}
                    Ok(i) if i > row => kept.insert((i - 1).to_string(), v.clone()),
                    _ => kept.insert(k.as_str(), v.clone()),
                }
            }
            Some(kept)
        }
        _ => None,
    };
    if let Some(names) = renamed {
        top.insert("names", Value::Dict(names));
    }
    Value::Dict(top)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use fontcraft_model::plist::openstep;

    use super::*;

    fn value(text: &str) -> Value {
        openstep::parse(text).unwrap().root
    }

    const RED: Rgba = Rgba::new(255, 0, 0, 255);
    const GREEN: Rgba = Rgba::new(0, 255, 0, 255);

    #[test]
    fn the_three_stored_forms_read_the_same() {
        let v2 = value(r#"(("255,0,0,255","0,255,0,255"),("0,0,255,128","128,255"))"#);
        let v3 = value("(((255,0,0,255),(0,255,0,255)),((0,0,255,128),(128,255)))");
        let v4 =
            value("{names = {\"1\" = Leaf;}; palettes = ({colors = ((1,0,0,1),(0,1,0,1)); name = Day;},{colors = ((0,0,1,0.502),(0.502,1));});}");
        let want = Palettes::new(vec![vec![RED, GREEN], vec![Rgba::new(0, 0, 255, 128), Rgba::new(128, 128, 128, 255)]]).unwrap();
        for (v, f) in [(&v2, Form::V2), (&v3, Form::V3), (&v4, Form::V4)] {
            assert_eq!(parse(v).unwrap(), want, "{f:?}");
            assert_eq!(form(v), Some(f));
            // Written back in its own form, it reads the same.
            assert_eq!(parse(&store(Some(v), &want, Change::Colors, Form::V3)).unwrap(), want, "{f:?}");
            assert_eq!(form(&store(Some(v), &want, Change::Colors, Form::V3)), Some(f));
        }
        // CMYK: pure cyan.
        assert_eq!(parse(&value("(((255,0,0,0,255)))")).unwrap().get(0, 0), Some(Rgba::new(0, 255, 255, 255)));
        assert_eq!(parse(&to_param(&want).value).unwrap(), want);
    }

    #[test]
    fn hostile_palettes_are_refused_without_panicking() {
        let ragged = value("(((255,0,0,255),(0,0,0,255)),((1,2,3,4)))");
        assert!(matches!(parse(&ragged), Err(ColorError::BadPalettes(_))));
        for out in ["(((256,0,0,255)))", "(((-1,0,0,255)))", "(((1,2,3)))", "(((1,2,3,4,5,6)))", "((\"a,b,c,d\"))", "((x))", "(x)"] {
            assert!(matches!(parse(&value(out)), Err(ColorError::BadPalettes(_))), "{out}");
        }
        assert!(matches!(parse(&value("{palettes = ({colors = ((2,0,0,1));});}")), Err(ColorError::BadPalettes(_))));
        assert!(matches!(parse(&value("{names = {};}")), Err(ColorError::BadPalettes(_))));
        // 100,000 rows: refused before any colour is read.
        let row = compact(vec![Value::Number(Number::int(0)); 4]);
        let big = multiline(vec![multiline(vec![row.clone(); 100_000])]);
        assert!(matches!(parse(&big), Err(ColorError::LimitExceeded { .. })));
        let many = multiline(vec![multiline(vec![row]); MAX_PALETTES + 1]);
        assert!(matches!(parse(&many), Err(ColorError::LimitExceeded { .. })));
        assert!(Palettes::new(vec![vec![RED], vec![]]).is_err());
    }

    #[test]
    fn edits_keep_every_palette_the_same_length() {
        let p = Palettes::new(vec![vec![RED, GREEN], vec![GREEN, RED]]).unwrap();
        let added = p.add_row(Rgba::BLACK).unwrap();
        assert_eq!(added.rows(), 3);
        assert!(added.palettes().iter().all(|c| c.len() == 3 && c[2] == Rgba::BLACK));
        let removed = added.remove_row(0).unwrap();
        assert_eq!(removed.palettes(), &[vec![GREEN, Rgba::BLACK], vec![RED, Rgba::BLACK]]);
        assert_eq!(p.remove_row(2), Err(ColorError::NoSuchRow(2)));
        assert_eq!(p.set(1, 0, Rgba::BLACK).unwrap().get(1, 0), Some(Rgba::BLACK));
        assert_eq!(p.set(2, 0, RED), Err(ColorError::NoSuchPalette(2)));
        assert_eq!(p.set(0, 5, RED), Err(ColorError::NoSuchRow(5)));
        let three = p.add_palette().unwrap();
        assert_eq!(three.palette(2), Some(&[RED, GREEN][..]));
        assert_eq!(p.add_palette_from(1).unwrap().palette(2), Some(&[GREEN, RED][..]));
        let one = p.remove_palette(0).unwrap();
        assert_eq!(one.palettes(), &[vec![GREEN, RED]]);
        assert_eq!(one.remove_palette(0), Err(ColorError::LastPalette));
        // From nothing: Add Color makes the first palette, Add Palette an empty one.
        assert_eq!(Palettes::default().add_row(RED).unwrap().palettes(), &[vec![RED]]);
        assert_eq!(Palettes::default().add_palette().unwrap().palettes().len(), 1);
    }

    #[test]
    fn version_4_names_follow_their_rows_and_palettes() {
        let v4 = value(
            "{names = {\"0\" = Sky; \"1\" = Leaf; \"2\" = Sun;}; palettes = ({colors = ((1,0,0,1),(0,1,0,1),(0,0,1,1)); name = Day;},{colors = ((0,0,0,1),(0,0,0,1),(0,0,0,1)); name = Night;});}",
        );
        let p = parse(&v4).unwrap();
        let out = store(Some(&v4), &p.remove_row(1).unwrap(), Change::RemovedRow(1), Form::V3);
        let names = out.as_dict().unwrap().get("names").unwrap().as_dict().unwrap();
        assert_eq!(names.get("0").and_then(Value::as_str), Some("Sky"));
        assert_eq!(names.get("1").and_then(Value::as_str), Some("Sun"));
        assert_eq!(names.len(), 2);
        let out = store(Some(&v4), &p.remove_palette(0).unwrap(), Change::RemovedPalette(0), Form::V3);
        let list = out.as_dict().unwrap().get("palettes").unwrap().as_array().unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].as_dict().unwrap().get("name").and_then(Value::as_str), Some("Night"));
    }

    #[test]
    fn a_master_s_own_palettes_win() {
        let mut font = Font::new("Test");
        let font_value = value("(((255,0,0,255)))");
        let master_value = value("(((0,255,0,255)),((0,0,255,255)))");
        Arc::make_mut(&mut font.info).params.set(COLOR_PALETTES_PARAM, font_value);
        let id = font.masters[0].id.clone();
        assert_eq!(read(&font, Some(&id)).unwrap().get(0, 0), Some(RED));
        Arc::make_mut(&mut font.masters)[0].params.set(COLOR_PALETTES_PARAM, master_value);
        let own = read(&font, Some(&id)).unwrap();
        assert_eq!((own.get(0, 0), own.palettes().len()), (Some(GREEN), 2));
        assert_eq!(read(&font, None).unwrap().get(0, 0), Some(RED));
        assert_eq!(read(&font, Some("nope")), Err(ColorError::MissingMaster("nope".into())));
        assert!(read(&Font::new("Empty"), None).unwrap().is_empty());
    }

    #[test]
    fn hex_colours() {
        assert_eq!(Rgba::parse_hex("#ff8000"), Some(Rgba::new(255, 128, 0, 255)));
        assert_eq!(Rgba::parse_hex("#FF800080"), Some(Rgba::new(255, 128, 0, 128)));
        for bad in ["ff8000", "#ff80", "#gg8000", "#ff8000ff00", "#ff800é", ""] {
            assert_eq!(Rgba::parse_hex(bad), None, "{bad}");
        }
        assert_eq!(Rgba::new(1, 2, 255, 16).hex(), "#0102FF10");
    }

    #[test]
    fn version_4_names_and_background_styles_are_read_and_a_dark_palette_is_stored_with_them() {
        let v4 = value(
            "{names = {\"1\" = Leaf; \"9\" = Gone;}; palettes = ({colors = ((1,0,0,1),(0,1,0,1)); name = Day; backgroundStyle = (light);},{colors = ((0,0,1,1),(1,1,1,1));});}",
        );
        let info = palette_info(&v4);
        assert_eq!(info, [PaletteInfo { name: Some("Day".into()), light: true, dark: false }, PaletteInfo::default()]);
        assert_eq!(color_names(&v4, 2), [None, Some("Leaf".to_string())], "a name past the last row is ignored");
        let v3 = value("(((255,0,0,255),(0,255,0,255)))");
        assert!(palette_info(&v3).is_empty());
        assert_eq!(color_names(&v3, 2), [None, None]);

        // A version 4 value keeps its names; a version 3 one becomes version 4.
        for previous in [&v4, &v3] {
            let before = parse(previous).unwrap();
            let next = before.add_palette_with(vec![Rgba::BLACK; 2]).unwrap();
            let stored = store_dark(Some(previous), &next, "Night");
            assert_eq!(form(&stored), Some(Form::V4));
            assert_eq!(parse(&stored).unwrap(), next);
            let info = palette_info(&stored);
            assert_eq!(info.last(), Some(&PaletteInfo { name: Some("Night".into()), light: false, dark: true }));
            assert_eq!(info.len(), next.palettes().len());
        }
        assert_eq!(palette_info(&store_dark(Some(&v4), &parse(&v4).unwrap().add_palette().unwrap(), "Night"))[0].name.as_deref(), Some("Day"));
        assert!(parse(&v4).unwrap().add_palette_with(vec![Rgba::BLACK]).is_err(), "one colour per row");
    }
}

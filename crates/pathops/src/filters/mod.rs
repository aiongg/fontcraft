//! Filters: operations that rewrite one layer's contours, run from the Filter menu, by a command
//! or by an export-time filter string (`docs/spec/11-filters.md`).
//!
//! Every filter implements [`Filter`]: its names, its parameter schema with defaults, the
//! operation on one layer's contours, and how its parameters read from and write to the export
//! string. [`registry`] lists them. The engine makes one `filter.*` command for each entry, and its
//! `filter.apply` and `filter.list` read the same list, so the menu, the dialogs, the CLI, agents
//! and export run one implementation per filter.
//!
//! To add a filter: a module next to `round_corners.rs` with a unit struct that implements
//! [`Filter`], and one line in [`registry`]. The engine needs no change.
//!
//! The export string is `Name; item; item; …` ([`FilterString`]): items are positional values or
//! `Key: value` pairs, booleans are `1` or `0`, and a trailing `include:` or `exclude:` item names
//! the glyphs the filter runs on or skips.

pub mod extrude;
pub mod hatch_outline;
pub mod offset_curve;
pub mod roughen;
pub mod round_corners;
pub mod rounded_font;

use std::fmt;

use crate::PathOpError;
use crate::contour::Contour;

pub use extrude::Extrude;
pub use hatch_outline::HatchOutline;
pub use offset_curve::OffsetCurve;
pub use roughen::Roughen;
pub use round_corners::RoundCorners;
pub use rounded_font::RoundedFont;

/// Longest filter string read, in bytes.
pub const MAX_FILTER_STRING: usize = 1 << 20;
/// Most items one filter string may hold, the name included.
pub const MAX_ITEMS: usize = 256;
/// Most glyph names an `include:` or `exclude:` item may hold.
pub const MAX_SCOPE_NAMES: usize = 65_535;

/// Every filter, in Filter menu order.
pub fn registry() -> &'static [&'static dyn Filter] {
    // One filter per line, so filters added in parallel merge cleanly.
    #[rustfmt::skip]
    static FILTERS: &[&dyn Filter] = &[
        &Extrude,
        &Roughen,
        &OffsetCurve,
        &RoundCorners,
        &HatchOutline,
        &RoundedFont,
    ];
    FILTERS
}

/// The filter with this export name (ASCII case ignored) or command id.
pub fn find(name: &str) -> Option<&'static dyn Filter> {
    registry().iter().copied().find(|f| f.name().eq_ignore_ascii_case(name) || f.id() == name)
}

/// One filter. Implementations are unit structs; they hold no state.
pub trait Filter: Sync {
    /// The command id, `filter.<camelCase name>`, as in the inventory row.
    fn id(&self) -> &'static str;
    /// The name that starts its export string, e.g. `RoundCorner`.
    fn name(&self) -> &'static str;
    /// The menu label, as in the inventory row.
    fn label(&self) -> &'static str;
    /// The command's parameter description for people and agents: targets, each parameter, the
    /// result. Same style as every other command's `params`.
    fn doc(&self) -> &'static str;
    /// The parameters, in dialog order.
    fn params(&self) -> &'static [Param];
    /// Whether a node selection narrows what the filter changes. The command then takes `points`
    /// and `segments` with one glyph, and [`Context::selection`] holds them.
    fn takes_selection(&self) -> bool {
        false
    }
    /// The filter on one layer's contours (its paths; components are never passed). The result
    /// replaces all of them. A node that survives keeps its `tag`, so the caller can carry its
    /// name and user data over. Counts and sizes that grow with the parameters must be capped
    /// before anything is allocated; the result may hold at most [`crate::MAX_NODES`] nodes and
    /// only coordinates within ±[`crate::MAX_COORDINATE`].
    fn apply(&self, contours: &[Contour], args: &Args, cx: &Context) -> Result<Vec<Contour>, PathOpError>;
    /// Whether the filter has an export-string form. A filter without one runs from its command
    /// and from `filter.apply` with named parameters only; its parameters have no slot.
    fn has_string_form(&self) -> bool {
        true
    }
    /// Reads the items of an export string (after the name, without `include`/`exclude`).
    /// Override when a value is written differently from the parameter (a fraction for a
    /// percentage, a word for a flag).
    fn read_items(&self, items: &[Item]) -> Result<Args, String> {
        read_items(self.params(), items)
    }
    /// Writes arguments as export-string items; the inverse of [`Filter::read_items`].
    fn write_items(&self, args: &Args) -> Vec<Item> {
        write_items(self.params(), args)
    }
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    /// A number from `min` to `max`.
    Number {
        min: f64,
        max: f64,
    },
    /// A whole number from `min` to `max`.
    Integer {
        min: i64,
        max: i64,
    },
    Bool,
    /// One of these words (ASCII case ignored when read).
    Choice(&'static [&'static str]),
}

/// Where a parameter sits in the export string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The n-th item without a key, counted from 0 after the name.
    Position(usize),
    /// A `Key: value` item; the key is matched with ASCII case ignored.
    Key(&'static str),
    /// Not in the export string: commands and dialogs only.
    None,
}

/// One parameter of a filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    /// The command parameter's key (camelCase), also the dialog field's id.
    pub key: &'static str,
    /// The dialog field's label.
    pub label: &'static str,
    pub kind: ParamKind,
    /// The value when the parameter is left out, and the dialog's first value. A
    /// [`Value::SameAs`] default takes the other parameter's value, and the dialog's field starts
    /// empty.
    pub default: Value,
    /// A command or string must give it; the default only fills the dialog.
    pub required: bool,
    pub slot: Slot,
    /// One line for people and agents: what it does and its unit.
    pub doc: &'static str,
}

/// A checked argument value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    /// Also the value of an integer parameter, which is whole.
    Number(f64),
    Bool(bool),
    /// One of a choice parameter's words, as the schema spells it.
    Word(&'static str),
    /// Only as a default: the value of the parameter with this key, which must come earlier in
    /// the schema and have a value of its own. A checked [`Args`] never holds it.
    SameAs(&'static str),
}

impl fmt::Display for Value {
    /// The export-string form: booleans as `1` and `0`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(v) => write!(f, "{v}"),
            Self::Bool(b) => write!(f, "{}", u8::from(*b)),
            Self::Word(w) => f.write_str(w),
            Self::SameAs(_) => Ok(()),
        }
    }
}

/// An argument as given, before it is checked against its parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input<'a> {
    Number(f64),
    Bool(bool),
    /// Text from an export string, or a word from a command.
    Text(&'a str),
}

impl fmt::Display for Input<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(v) => write!(f, "{v}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Text(t) => write!(f, "\"{t}\""),
        }
    }
}

/// A filter's arguments: one checked value for every parameter, in schema order.
#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    values: Vec<(&'static str, Value)>,
}

impl Args {
    /// Checks `given` against `params`: every key known and given once, every value of the right
    /// kind and in range, every required parameter present. The others take their defaults.
    pub fn new<'a>(params: &'static [Param], given: impl IntoIterator<Item = (&'a str, Input<'a>)>) -> Result<Args, String> {
        let mut values: Vec<Option<Value>> = vec![None; params.len()];
        for (key, input) in given {
            let Some(i) = params.iter().position(|p| p.key == key) else {
                let known: Vec<&str> = params.iter().map(|p| p.key).collect();
                return Err(format!("unknown parameter `{key}`; this filter takes {}", list(&known)));
            };
            let (Some(slot), Some(param)) = (values.get_mut(i), params.get(i)) else { continue };
            if slot.is_some() {
                return Err(format!("`{key}` is given twice"));
            }
            *slot = Some(convert(param, input)?);
        }
        let mut out: Vec<(&'static str, Value)> = Vec::with_capacity(params.len());
        for (param, value) in params.iter().zip(values) {
            let value = match value {
                Some(v) => v,
                None if param.required => return Err(format!("`{}` is required", param.key)),
                None => resolve(param, &out).ok_or_else(|| format!("`{}` has no default", param.key))?,
            };
            out.push((param.key, value));
        }
        Ok(Args { values: out })
    }

    /// Every parameter at its default, required ones included (the dialog's first values).
    pub fn defaults(params: &'static [Param]) -> Args {
        let mut out: Vec<(&'static str, Value)> = Vec::with_capacity(params.len());
        for p in params {
            out.push((p.key, resolve(p, &out).unwrap_or(p.default)));
        }
        Args { values: out }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.values.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    /// The number `key` holds; 0 when it holds none, which a checked `Args` never does for a
    /// number parameter of its own schema.
    pub fn number(&self, key: &str) -> f64 {
        match self.get(key) {
            Some(Value::Number(v)) => v,
            _ => 0.0,
        }
    }

    /// The flag `key` holds; false when it holds none.
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Value::Bool(true)))
    }

    /// The word `key` holds; empty when it holds none.
    pub fn word(&self, key: &str) -> &'static str {
        match self.get(key) {
            Some(Value::Word(w)) => w,
            _ => "",
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Value)> + '_ {
        self.values.iter().copied()
    }
}

/// `param`'s default, with a [`Value::SameAs`] taken from the values already in `earlier`; `None`
/// when that parameter is not among them.
fn resolve(param: &Param, earlier: &[(&'static str, Value)]) -> Option<Value> {
    match param.default {
        Value::SameAs(other) => earlier.iter().find(|(k, _)| *k == other).map(|(_, v)| *v).filter(|v| !matches!(v, Value::SameAs(_))),
        v => Some(v),
    }
}

fn convert(param: &Param, input: Input) -> Result<Value, String> {
    let key = param.key;
    let number = |input: Input| match input {
        Input::Number(v) => Some(v),
        Input::Text(t) => t.trim().parse::<f64>().ok(),
        Input::Bool(_) => None,
    };
    match param.kind {
        ParamKind::Number { min, max } => number(input)
            .filter(|v| v.is_finite() && (min..=max).contains(v))
            .map(Value::Number)
            .ok_or_else(|| format!("`{key}` must be a number from {min} to {max}, got {input}")),
        ParamKind::Integer { min, max } => {
            let (lo, hi) = (min as f64, max as f64);
            number(input)
                .filter(|v| v.is_finite() && v.fract() == 0.0 && (lo..=hi).contains(v))
                .map(Value::Number)
                .ok_or_else(|| format!("`{key}` must be a whole number from {min} to {max}, got {input}"))
        }
        ParamKind::Bool => match input {
            Input::Bool(b) => Ok(Value::Bool(b)),
            Input::Number(v) if v == 0.0 || v == 1.0 => Ok(Value::Bool(v == 1.0)),
            Input::Text(t) if matches!(t.trim(), "1" | "true") => Ok(Value::Bool(true)),
            Input::Text(t) if matches!(t.trim(), "0" | "false") => Ok(Value::Bool(false)),
            _ => Err(format!("`{key}` must be true or false (1 or 0 in a filter string), got {input}")),
        },
        ParamKind::Choice(words) => match input {
            Input::Text(t) => words
                .iter()
                .copied()
                .find(|w| w.eq_ignore_ascii_case(t.trim()))
                .map(Value::Word)
                .ok_or_else(|| format!("`{key}` must be {}, got {input}", list(words))),
            _ => Err(format!("`{key}` must be {}, got {input}", list(words))),
        },
    }
}

/// `a`, `a or b`, `a, b or c`.
fn list(words: &[&str]) -> String {
    match words {
        [] => "nothing".into(),
        [one] => format!("`{one}`"),
        [rest @ .., last] => format!("{} or `{last}`", rest.iter().map(|w| format!("`{w}`")).collect::<Vec<_>>().join(", ")),
    }
}

// ── Context ─────────────────────────────────────────────────────────────────────────────────────

/// What a filter may read besides the contours and its arguments.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context<'a> {
    /// New and moved points are rounded to multiples of this many units; 0 leaves them as they
    /// are.
    pub grid: f64,
    /// The selected on-curve nodes as (contour, node) indices into the contours passed; empty
    /// when nothing is selected. Only for filters that [take a selection](Filter::takes_selection).
    pub selection: &'a [(usize, usize)],
    /// The master's vertical stem widths, in Font Info order.
    pub vertical_stems: &'a [f64],
    /// The master's horizontal stem widths, in Font Info order.
    pub horizontal_stems: &'a [f64],
    pub metrics: MasterMetrics,
}

/// The master's vertical metrics, in font units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MasterMetrics {
    pub units_per_em: f64,
    pub ascender: f64,
    pub descender: f64,
    pub cap_height: f64,
    pub x_height: f64,
    /// Degrees, clockwise from vertical.
    pub italic_angle: f64,
}

// ── Export strings ──────────────────────────────────────────────────────────────────────────────

/// One item of a filter string after the name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// `None` for a positional value.
    pub key: Option<String>,
    pub value: String,
}

impl Item {
    pub fn positional(value: impl Into<String>) -> Self {
        Self { key: None, value: value.into() }
    }

    pub fn keyed(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { key: Some(key.into()), value: value.into() }
    }
}

impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.key {
            Some(k) => write!(f, "{k}:{}", self.value),
            None => f.write_str(&self.value),
        }
    }
}

/// A parsed filter string: `Name; item; …; include: a b c` (or `exclude: a,b,c`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterString {
    pub name: String,
    /// The items after the name, without `include` and `exclude`.
    pub items: Vec<Item>,
    /// Only these glyphs; empty: no limit.
    pub include: Vec<String>,
    /// Every glyph but these.
    pub exclude: Vec<String>,
}

impl FilterString {
    /// Reads a filter string. Items are separated by semicolons; spaces around items, keys and
    /// values are ignored, and so are empty items (a trailing semicolon). A key is the text before
    /// the first colon of an item. `include` and `exclude` take glyph names separated by spaces or
    /// commas; a string may hold one of them, once.
    pub fn parse(text: &str) -> Result<FilterString, String> {
        if text.len() > MAX_FILTER_STRING {
            return Err(format!("the filter string is longer than {MAX_FILTER_STRING} bytes"));
        }
        let mut items = text.split(';').map(str::trim).filter(|s| !s.is_empty());
        let name = items.next().ok_or("the filter string is empty")?;
        if name.contains(':') || name.contains(char::is_whitespace) {
            return Err(format!("the filter string must start with a filter name, not \"{name}\""));
        }
        let mut out = FilterString { name: name.to_string(), ..Default::default() };
        for (count, item) in items.enumerate() {
            if count + 2 > MAX_ITEMS {
                return Err(format!("the filter string holds more than {MAX_ITEMS} items"));
            }
            let Some((key, value)) = item.split_once(':') else {
                out.items.push(Item::positional(item));
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if key.is_empty() {
                return Err(format!("\"{item}\" has no key before its colon"));
            }
            let scope = if key.eq_ignore_ascii_case("include") {
                &mut out.include
            } else if key.eq_ignore_ascii_case("exclude") {
                &mut out.exclude
            } else {
                if out.items.iter().any(|i| i.key.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(key))) {
                    return Err(format!("`{key}` is given twice"));
                }
                out.items.push(Item::keyed(key, value));
                continue;
            };
            if !scope.is_empty() {
                return Err(format!("`{key}` is given twice"));
            }
            for name in value.split([' ', ',']).filter(|n| !n.is_empty()) {
                if scope.len() >= MAX_SCOPE_NAMES {
                    return Err(format!("`{key}` names more than {MAX_SCOPE_NAMES} glyphs"));
                }
                scope.push(name.to_string());
            }
            if scope.is_empty() {
                return Err(format!("`{key}` names no glyph"));
            }
        }
        if !out.include.is_empty() && !out.exclude.is_empty() {
            return Err("give `include` or `exclude`, not both".into());
        }
        Ok(out)
    }

    /// Whether the glyph scoping lets `glyph` through.
    pub fn includes(&self, glyph: &str) -> bool {
        (self.include.is_empty() || self.include.iter().any(|g| g == glyph)) && !self.exclude.iter().any(|g| g == glyph)
    }

    /// The string for `filter` with `args` and this scoping: no spaces, a semicolon after every
    /// item.
    pub fn write(filter: &dyn Filter, args: &Args, include: &[String], exclude: &[String]) -> String {
        FilterString { name: filter.name().to_string(), items: filter.write_items(args), include: include.to_vec(), exclude: exclude.to_vec() }
            .to_string()
    }
}

impl fmt::Display for FilterString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{};", self.name)?;
        for item in &self.items {
            write!(f, "{item};")?;
        }
        if !self.include.is_empty() {
            write!(f, "include:{};", self.include.join(","))?;
        }
        if !self.exclude.is_empty() {
            write!(f, "exclude:{};", self.exclude.join(","))?;
        }
        Ok(())
    }
}

/// Reads export-string items by the parameters' slots: the n-th positional item fills the
/// parameter at `Slot::Position(n)`, a keyed item the parameter at its `Slot::Key`.
pub fn read_items(params: &'static [Param], items: &[Item]) -> Result<Args, String> {
    let mut given: Vec<(&str, Input)> = Vec::with_capacity(items.len());
    let mut position = 0usize;
    for item in items {
        let param = match &item.key {
            None => {
                let p = params.iter().find(|p| p.slot == Slot::Position(position));
                position += 1;
                p.ok_or_else(|| {
                    let n = params.iter().filter(|p| matches!(p.slot, Slot::Position(_))).count();
                    format!("this filter takes {n} values without a key; \"{}\" is one too many", item.value)
                })?
            }
            Some(key) => params.iter().find(|p| matches!(p.slot, Slot::Key(k) if k.eq_ignore_ascii_case(key))).ok_or_else(|| {
                let keys: Vec<&str> = params.iter().filter_map(|p| if let Slot::Key(k) = p.slot { Some(k) } else { None }).collect();
                format!("unknown key `{key}`; this filter takes {}", list(&keys))
            })?,
        };
        given.push((param.key, Input::Text(&item.value)));
    }
    Args::new(params, given)
}

/// Writes every positional parameter, then each keyed one that is required or not at its
/// default. Parameters without a slot are not written.
pub fn write_items(params: &'static [Param], args: &Args) -> Vec<Item> {
    let mut positional: Vec<(usize, &Param)> =
        params.iter().filter_map(|p| if let Slot::Position(n) = p.slot { Some((n, p)) } else { None }).collect();
    positional.sort_by_key(|(n, _)| *n);
    let value = |p: &Param| args.get(p.key).unwrap_or(p.default);
    let default = |p: &Param| match p.default {
        Value::SameAs(other) => args.get(other).unwrap_or(p.default),
        v => v,
    };
    let mut out: Vec<Item> = positional.into_iter().map(|(_, p)| Item::positional(value(p).to_string())).collect();
    for p in params {
        if let Slot::Key(k) = p.slot
            && (p.required || value(p) != default(p))
        {
            out.push(Item::keyed(k, value(p).to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn a_filter_string_reads_its_items_and_scoping() {
        let s = FilterString::parse(" RoundCorner ; 20 ; 1 ; exclude: a, b c ;").unwrap();
        assert_eq!(s.name, "RoundCorner");
        assert_eq!(s.items, vec![Item::positional("20"), Item::positional("1")]);
        assert_eq!(s.exclude, vec!["a", "b", "c"]);
        assert!(s.includes("d") && !s.includes("b"));
        let f = find("roundcorner").unwrap();
        let args = f.read_items(&s.items).unwrap();
        assert_eq!(args.number("radius"), 20.0);
        assert!(args.flag("visualCorrections"));
        assert_eq!(FilterString::write(f, &args, &[], &s.exclude), "RoundCorner;20;1;exclude:a,b,c;");
    }

    #[test]
    fn hostile_filter_strings_are_errors() {
        for bad in ["", ";;", "Key:1", "RoundCorner;include:a;exclude:b", "RoundCorner;include:a;include:b", "RoundCorner;include: ,"] {
            assert!(FilterString::parse(bad).is_err(), "{bad:?}");
        }
        assert!(FilterString::parse(&"x;".repeat(MAX_ITEMS + 1)).is_err());
        assert!(FilterString::parse(&"x".repeat(MAX_FILTER_STRING + 1)).is_err());
        let f = find("RoundCorner").unwrap();
        for bad in [
            "RoundCorner",
            "RoundCorner;NaN",
            "RoundCorner;inf",
            "RoundCorner;-5",
            "RoundCorner;1e300",
            "RoundCorner;20;2",
            "RoundCorner;20;1;3",
            "RoundCorner;20;Radius:3",
        ] {
            let s = FilterString::parse(bad).unwrap();
            assert!(f.read_items(&s.items).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn every_filter_has_a_unique_name_and_a_well_formed_schema() {
        let mut names = std::collections::HashSet::new();
        for f in registry() {
            assert!(names.insert(f.name().to_ascii_lowercase()), "{} twice", f.name());
            assert!(f.id().starts_with("filter."), "{}", f.id());
            let mut positions: Vec<usize> = f.params().iter().filter_map(|p| if let Slot::Position(n) = p.slot { Some(n) } else { None }).collect();
            positions.sort_unstable();
            assert!(positions.iter().enumerate().all(|(i, n)| i == *n), "{}: positions must run 0, 1, 2…", f.name());
            for (i, p) in f.params().iter().enumerate() {
                if let Value::SameAs(other) = p.default {
                    let earlier = f.params().iter().take(i).find(|q| q.key == other);
                    assert!(earlier.is_some_and(|q| q.kind == p.kind), "{}: `{}` must follow `{other}`, of its kind", f.name(), p.key);
                }
            }
            // The defaults pass the schema's own checks and survive a trip through the string.
            let args = Args::defaults(f.params());
            assert!(args.iter().all(|(_, v)| !matches!(v, Value::SameAs(_))), "{}", f.name());
            let given: Vec<(&str, Input)> = Vec::new();
            let optional = Args::new(f.params(), given);
            assert!(optional.is_ok() || f.params().iter().any(|p| p.required), "{}", f.name());
            if !f.has_string_form() {
                assert!(f.params().iter().all(|p| p.slot == Slot::None), "{}: no string form, so no slots", f.name());
                continue;
            }
            let s = FilterString::parse(&FilterString::write(*f, &args, &[], &[])).unwrap();
            assert_eq!(f.read_items(&s.items).unwrap(), args, "{}", f.name());
        }
    }
}

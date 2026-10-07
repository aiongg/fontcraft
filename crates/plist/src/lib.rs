//! Property lists that remember how they were written (L0, standalone).
//!
//! FontCraft reads and writes two property-list syntaxes: the OpenStep text form that the primary
//! reference's source format uses ([`openstep`]), and XML (planned, for UFO). Both readers keep
//! enough of the original text that writing an unchanged document gives back the same bytes:
//!
//! - **Order.** Dictionaries keep their entries in file order.
//! - **Spelling.** Each format has a canonical way to write a string, a number or a data blob.
//!   A scalar whose source text differs from that (a needlessly quoted string, `1.0` for one, an
//!   escape where a literal character would do) keeps its source text as its *spelling*, and the
//!   writer uses it. A value built in code has no spelling and is written canonically. Counting
//!   spellings is how the corpus tests measure how close the canonical rules are to the reference.
//! - **Layout.** Each dictionary and array records whether it was written over several lines or
//!   on one.
//!
//! Equality ignores spelling and layout: two values are equal when they mean the same.
//!
//! All input is untrusted: parsing returns an error for anything malformed, never panics, and is
//! bounded by [`MAX_INPUT`] and [`MAX_DEPTH`].
#![forbid(unsafe_code)]

pub mod openstep;

use std::fmt;

/// Largest input accepted, in bytes.
pub const MAX_INPUT: usize = 512 << 20;
/// Deepest nesting of dictionaries and arrays accepted.
pub const MAX_DEPTH: usize = 256;

/// A property-list value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Dict(Dict),
    Array(Array),
    String(Str),
    Number(Number),
    Data(Data),
}

/// How a dictionary or array was written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// The opening bracket is followed by a line break; one element per line.
    #[default]
    Multiline,
    /// Everything on one line, as in a coordinate pair.
    Compact,
}

/// A dictionary in file order. Keys may repeat in malformed input; lookups return the first.
#[derive(Clone, Debug, Default)]
pub struct Dict {
    pub entries: Vec<(Str, Value)>,
    pub layout: Layout,
}

impl Dict {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k.as_str() == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries.iter_mut().find(|(k, _)| k.as_str() == key).map(|(_, v)| v)
    }

    /// Replaces the value of an existing key in place, or appends the entry.
    pub fn insert(&mut self, key: impl Into<Str>, value: Value) {
        let key = key.into();
        match self.get_mut(key.as_str()) {
            Some(slot) => *slot = value,
            None => self.entries.push((key, value)),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

/// An array.
#[derive(Clone, Debug, Default)]
pub struct Array {
    pub items: Vec<Value>,
    pub layout: Layout,
}

impl PartialEq for Array {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

/// A string, with its source spelling when that was not the canonical one.
#[derive(Clone, Debug, Default)]
pub struct Str {
    text: String,
    spelling: Option<Box<str>>,
}

impl Str {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), spelling: None }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The source text, including any quotes, when it differs from the canonical spelling.
    pub fn spelling(&self) -> Option<&str> {
        self.spelling.as_deref()
    }

    pub(crate) fn with_spelling(text: String, spelling: Option<Box<str>>) -> Self {
        Self { text, spelling }
    }
}

impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl From<&str> for Str {
    fn from(s: &str) -> Self {
        Str::new(s)
    }
}

impl From<String> for Str {
    fn from(s: String) -> Self {
        Str::new(s)
    }
}

/// A number: an integer when the source had no decimal point and it fits, otherwise a float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub fn as_f64(self) -> f64 {
        match self {
            // Precision loss beyond 2^53 is accepted: font values are far smaller.
            #[allow(clippy::cast_precision_loss)]
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Number {
    value: Num,
    spelling: Option<Box<str>>,
}

impl Number {
    pub fn int(i: i64) -> Self {
        Self { value: Num::Int(i), spelling: None }
    }

    /// `None` when `f` is not finite: property lists cannot hold infinities or NaN.
    pub fn float(f: f64) -> Option<Self> {
        f.is_finite().then_some(Self { value: Num::Float(f), spelling: None })
    }

    pub fn value(&self) -> Num {
        self.value
    }

    pub fn spelling(&self) -> Option<&str> {
        self.spelling.as_deref()
    }

    pub(crate) fn with_spelling(value: Num, spelling: Option<Box<str>>) -> Self {
        Self { value, spelling }
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.value.as_f64() == other.value.as_f64()
    }
}

/// Binary data, with its source spelling when that was not the canonical one.
#[derive(Clone, Debug, Default)]
pub struct Data {
    bytes: Vec<u8>,
    spelling: Option<Box<str>>,
}

impl Data {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, spelling: None }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn spelling(&self) -> Option<&str> {
        self.spelling.as_deref()
    }

    pub(crate) fn with_spelling(bytes: Vec<u8>, spelling: Option<Box<str>>) -> Self {
        Self { bytes, spelling }
    }
}

impl PartialEq for Data {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl Value {
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Array> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_num(&self) -> Option<Num> {
        match self {
            Value::Number(n) => Some(n.value()),
            _ => None,
        }
    }

    /// Calls `f` on this value and every value inside it, depth first.
    pub fn visit<'a>(&'a self, f: &mut impl FnMut(&'a Value)) {
        // Depth is bounded by MAX_DEPTH for parsed values; built values are the caller's.
        f(self);
        match self {
            Value::Dict(d) => d.entries.iter().for_each(|(_, v)| v.visit(f)),
            Value::Array(a) => a.items.iter().for_each(|v| v.visit(f)),
            _ => {}
        }
    }

    /// The source spellings inside this value (dictionary keys included), at most `limit` of them,
    /// in document order. For finding which canonical rule a file breaks.
    pub fn spellings(&self, limit: usize) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        self.visit(&mut |v| {
            let found: Vec<&str> = match v {
                Value::String(s) => s.spelling().into_iter().collect(),
                Value::Number(x) => x.spelling().into_iter().collect(),
                Value::Data(d) => d.spelling().into_iter().collect(),
                Value::Dict(d) => d.entries.iter().filter_map(|(k, _)| k.spelling()).collect(),
                Value::Array(_) => Vec::new(),
            };
            for s in found {
                if out.len() < limit {
                    out.push(s);
                }
            }
        });
        out
    }

    /// How many scalars inside this value (dictionary keys included) carry a source spelling.
    pub fn spelling_count(&self) -> usize {
        let mut n = 0;
        self.visit(&mut |v| match v {
            Value::String(s) => n += usize::from(s.spelling.is_some()),
            Value::Number(x) => n += usize::from(x.spelling.is_some()),
            Value::Data(d) => n += usize::from(d.spelling.is_some()),
            Value::Dict(d) => n += d.entries.iter().filter(|(k, _)| k.spelling.is_some()).count(),
            Value::Array(_) => {}
        });
        n
    }
}

/// A parse error, with its position in the input (1-based line and column, in characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for Error {}

impl Error {
    pub(crate) fn at(text: &str, pos: usize, message: impl Into<String>) -> Self {
        let before = text.get(..pos).unwrap_or(text);
        let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Self { line, column, message: message.into() }
    }
}

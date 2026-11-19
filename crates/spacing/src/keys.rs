//! Metrics keys: parsing, and the display and stored forms.
//!
//! Grammar (spaces anywhere are ignored):
//! ```text
//! field   = number | glyph-name | key        ; a bare name is the whole text
//! key     = ("=" | "==") body ["@" signed]   ; "==" is local to the layer
//! body    = unsigned                          ; constant           =64
//!         | ("+" | "-") unsigned              ; signed constant    =-12
//!         | "|"                               ; own opposite side  =|
//!         | ["|"] name [op unsigned]          ; reference          =|n*0.5
//! ```
//! One operation at most; `@` comes last, after the operation (`=H+8@200`). The width, vertical
//! width and vertical origin fields take no `|` and no `@`; the top and bottom fields take no
//! `@`. In a `=` key a name ends at `+`, `*`,
//! `/`, `@`, `|`, `=` or a parenthesis; a `-` may belong to the name or be the operator, and the
//! longest leading part that names a glyph of the font decides. When no part names a glyph, the
//! shortest reading that leaves a valid rest is taken (`=n-20` with no `n`: glyph `n`, minus 20).

use crate::{Field, MAX_KEY_LEN, MAX_KEY_NUMBER};

/// Where a key applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// On every layer of the glyph (stored on the glyph; typed with one `=` or as a bare name).
    Glyph,
    /// On one layer only (stored on the layer; typed and shown with `==`).
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    fn symbol(self) -> char {
        match self {
            Op::Add => '+',
            Op::Sub => '-',
            Op::Mul => '*',
            Op::Div => '/',
        }
    }
}

/// The one arithmetic step of a calculated key: the referenced metric, `op`, `operand`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Operation {
    pub op: Op,
    pub operand: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// Another glyph's metric: the same field, or the opposite side when `mirrored` (`=|n`).
    /// `bare` is a name typed or stored without `=`.
    Name { name: String, mirrored: bool, op: Option<Operation>, bare: bool },
    /// `=64`.
    Constant(f64),
    /// `=|`: this glyph's own opposite sidebearing.
    SelfMirror,
    /// `=+18`, `=-12`: an offset on a glyph aligned automatically (M6), else a constant.
    Signed(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub scope: Scope,
    pub body: Body,
    /// Measure the referenced sidebearing at this height instead of at the bounding box.
    pub at: Option<f64>,
}

/// What a field's text means.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// A plain value: no key is stored.
    Number(f64),
    Key(Key),
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{kind} (at byte {pos})")]
pub struct KeyError {
    /// Byte position in the text as given.
    pub pos: usize,
    pub kind: KeyErrorKind,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum KeyErrorKind {
    #[error("the key is longer than {MAX_KEY_LEN} bytes")]
    TooLong,
    #[error("the key is empty")]
    Empty,
    #[error("a glyph name or a number is missing")]
    ExpectedName,
    #[error("a number is missing")]
    ExpectedNumber,
    #[error("a number is larger than {MAX_KEY_NUMBER}")]
    NumberTooLarge,
    #[error("a key holds one operation at most")]
    TwoOperations,
    #[error("`{0}` is not expected here")]
    Unexpected(char),
    #[error("the {field} field does not take {what}")]
    NotInField { what: &'static str, field: &'static str },
}

/// The text without spaces, and for each byte of it the byte position in the original.
struct Compact {
    text: String,
    at: Vec<usize>,
    end: usize,
}

impl Compact {
    fn new(text: &str) -> Self {
        let mut out = String::with_capacity(text.len());
        let mut at = Vec::with_capacity(text.len());
        for (i, c) in text.char_indices() {
            if c.is_whitespace() {
                continue;
            }
            out.push(c);
            at.extend(std::iter::repeat_n(i, c.len_utf8()));
        }
        Self { text: out, at, end: text.len() }
    }

    fn pos(&self, i: usize) -> usize {
        self.at.get(i).copied().unwrap_or(self.end)
    }

    fn err(&self, i: usize, kind: KeyErrorKind) -> KeyError {
        KeyError { pos: self.pos(i), kind }
    }
}

/// Reads what was typed into a field: a plain number, a bare glyph name, or a key (`=…`, `==…`).
/// `exists` says whether a glyph of that name is in the font.
pub fn parse_field(text: &str, field: Field, exists: impl Fn(&str) -> bool) -> Result<Entry, KeyError> {
    if text.len() > MAX_KEY_LEN {
        return Err(KeyError { pos: MAX_KEY_LEN, kind: KeyErrorKind::TooLong });
    }
    let c = Compact::new(text);
    if let Some(v) = whole_number(&c, 0, Sign::Optional)? {
        return Ok(Entry::Number(v));
    }
    let (scope, start) = if c.text.starts_with("==") { (Scope::Local, 2) } else { (Scope::Glyph, usize::from(c.text.starts_with('='))) };
    parse_at(&c, start, scope, field, &exists).map(Entry::Key)
}

/// Reads a key as the model stores it; `scope` says where it is stored (a layer's key is local).
/// A stored key never stands for a plain number: `64` is a glyph name, `=64` a constant.
pub fn parse_stored(text: &str, scope: Scope, field: Field, exists: impl Fn(&str) -> bool) -> Result<Key, KeyError> {
    if text.len() > MAX_KEY_LEN {
        return Err(KeyError { pos: MAX_KEY_LEN, kind: KeyErrorKind::TooLong });
    }
    let c = Compact::new(text);
    let start = if c.text.starts_with("==") { 2 } else { usize::from(c.text.starts_with('=')) };
    parse_at(&c, start, scope, field, &exists)
}

fn parse_at(c: &Compact, start: usize, scope: Scope, field: Field, exists: &dyn Fn(&str) -> bool) -> Result<Key, KeyError> {
    let s = c.text.as_str();
    if s.is_empty() {
        return Err(c.err(0, KeyErrorKind::Empty));
    }
    let rest = s.get(start..).unwrap_or("");
    let key = if start == 0 {
        // No `=`: the whole text is one glyph name.
        Key { scope, body: Body::Name { name: s.to_string(), mirrored: false, op: None, bare: true }, at: None }
    } else if rest.is_empty() {
        return Err(c.err(start, KeyErrorKind::ExpectedName));
    } else if rest == "|" {
        Key { scope, body: Body::SelfMirror, at: None }
    } else if let Some(v) = whole_number(c, start, Sign::No)? {
        Key { scope, body: Body::Constant(v), at: None }
    } else if let Some(v) = whole_number(c, start, Sign::Required)? {
        Key { scope, body: Body::Signed(v), at: None }
    } else {
        let mirrored = rest.starts_with('|');
        let from = start + usize::from(mirrored);
        let (name, op, at) = reference(c, from, exists)?;
        Key { scope, body: Body::Name { name, mirrored, op, bare: false }, at }
    };
    check_field(c, &key, field)?;
    Ok(key)
}

fn check_field(c: &Compact, key: &Key, field: Field) -> Result<(), KeyError> {
    let mirror = matches!(key.body, Body::SelfMirror | Body::Name { mirrored: true, .. });
    let what = if !field.is_sidebearing() && mirror {
        Some("`|`")
    } else if key.at.is_some() && !matches!(field, Field::Left | Field::Right) {
        Some("`@`")
    } else {
        None
    };
    match what {
        Some(what) => {
            let ch = if what == "`|`" { '|' } else { '@' };
            let i = c.text.find(ch).unwrap_or(0);
            Err(c.err(i, KeyErrorKind::NotInField { what, field: field.name() }))
        }
        None => Ok(()),
    }
}

/// Characters that end a glyph name inside a `=` key (`-` does not: names may hold it).
fn ends_name(ch: char) -> bool {
    matches!(ch, '+' | '*' | '/' | '@' | '|' | '=' | '(' | ')')
}

/// A glyph name from byte `from`, then an optional operation and `@` height.
fn reference(c: &Compact, from: usize, exists: &dyn Fn(&str) -> bool) -> Result<(String, Option<Operation>, Option<f64>), KeyError> {
    let s = c.text.get(from..).unwrap_or("");
    let run = s.find(ends_name).unwrap_or(s.len());
    if run == 0 {
        return Err(c.err(from, s.chars().next().map_or(KeyErrorKind::ExpectedName, KeyErrorKind::Unexpected)));
    }
    let run_text = s.get(..run).unwrap_or("");
    // Where the name may end: before each `-` after the first character, and at the end of the run.
    let mut cuts: Vec<usize> = run_text.match_indices('-').map(|(i, _)| i).filter(|&i| i > 0).collect();
    cuts.push(run);
    let tail_of = |cut: usize| tail(c, from + cut);
    let known = cuts.iter().rev().find_map(|&cut| {
        let name = s.get(..cut)?;
        if !exists(name) {
            return None;
        }
        tail_of(cut).ok().map(|t| (cut, t))
    });
    let (cut, (op, at)) = match known {
        Some(found) => found,
        None => match cuts.iter().find_map(|&cut| tail_of(cut).ok().map(|t| (cut, t))) {
            Some(found) => found,
            None => return Err(tail_of(run).err().unwrap_or(c.err(from + run, KeyErrorKind::ExpectedName))),
        },
    };
    Ok((s.get(..cut).unwrap_or("").to_string(), op, at))
}

/// What may follow a name: `[op unsigned] ["@" signed]`, and nothing else.
fn tail(c: &Compact, from: usize) -> Result<(Option<Operation>, Option<f64>), KeyError> {
    let s = c.text.as_bytes();
    let mut i = from;
    let op = match s.get(i) {
        Some(b'+') => Some(Op::Add),
        Some(b'-') => Some(Op::Sub),
        Some(b'*') => Some(Op::Mul),
        Some(b'/') => Some(Op::Div),
        _ => None,
    };
    let op = match op {
        Some(op) => {
            let (operand, next) = number(c, i + 1, false)?;
            i = next;
            Some(Operation { op, operand })
        }
        None => None,
    };
    let at = if s.get(i) == Some(&b'@') {
        let (v, next) = number(c, i + 1, true)?;
        i = next;
        Some(v)
    } else {
        None
    };
    match c.text.get(i..).and_then(|r| r.chars().next()) {
        None => Ok((op, at)),
        Some('+' | '-' | '*' | '/') if op.is_some() && at.is_none() => Err(c.err(i, KeyErrorKind::TwoOperations)),
        Some(ch) => Err(c.err(i, KeyErrorKind::Unexpected(ch))),
    }
}

/// A number from byte `from`: `[sign] digits ["." digits]` (or `.5`). Returns it and the byte
/// after it.
fn number(c: &Compact, from: usize, signed: bool) -> Result<(f64, usize), KeyError> {
    let s = c.text.as_bytes();
    let mut i = from;
    if signed && matches!(s.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits_from = i;
    let mut digits = 0usize;
    while s.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
        digits += 1;
    }
    if s.get(i) == Some(&b'.') {
        i += 1;
        while s.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return Err(c.err(digits_from, KeyErrorKind::ExpectedNumber));
    }
    let v: f64 = c.text.get(from..i).and_then(|t| t.parse().ok()).ok_or_else(|| c.err(from, KeyErrorKind::ExpectedNumber))?;
    if !v.is_finite() || v.abs() > MAX_KEY_NUMBER {
        return Err(c.err(from, KeyErrorKind::NumberTooLarge));
    }
    Ok((v, i))
}

#[derive(Clone, Copy, PartialEq)]
enum Sign {
    No,
    Optional,
    Required,
}

/// The text from `from` to the end as one number, if it is one.
fn whole_number(c: &Compact, from: usize, sign: Sign) -> Result<Option<f64>, KeyError> {
    let s = c.text.as_bytes();
    let has_sign = matches!(s.get(from), Some(b'+' | b'-'));
    if (sign == Sign::No && has_sign) || (sign == Sign::Required && !has_sign) {
        return Ok(None);
    }
    if !s.get(from + usize::from(has_sign)).is_some_and(|b| b.is_ascii_digit() || *b == b'.') {
        return Ok(None);
    }
    match number(c, from, has_sign) {
        Ok((v, end)) if end == c.text.len() => Ok(Some(v)),
        Err(e) if e.kind == KeyErrorKind::NumberTooLarge => Err(e),
        _ => Ok(None),
    }
}

impl Key {
    /// The body as text, without the scope's `=`.
    fn body_text(&self) -> String {
        let mut out = match &self.body {
            Body::Name { name, mirrored, op, .. } => {
                let mut t = String::new();
                if *mirrored {
                    t.push('|');
                }
                t.push_str(name);
                if let Some(o) = op {
                    t.push(o.op.symbol());
                    t.push_str(&o.operand.to_string());
                }
                t
            }
            Body::Constant(v) => v.to_string(),
            Body::SelfMirror => "|".into(),
            Body::Signed(v) => {
                if *v < 0.0 || (*v == 0.0 && v.is_sign_negative()) {
                    format!("-{}", v.abs())
                } else {
                    format!("+{v}")
                }
            }
        };
        if let Some(h) = self.at {
            out.push('@');
            out.push_str(&h.to_string());
        }
        out
    }

    fn is_bare(&self) -> bool {
        matches!(self.body, Body::Name { bare: true, .. })
    }

    /// The text a field shows: the stored text, with `==` for a local key.
    pub fn display(&self) -> String {
        match (self.scope, self.is_bare()) {
            (Scope::Local, _) => format!("=={}", self.body_text()),
            (Scope::Glyph, true) => self.body_text(),
            (Scope::Glyph, false) => format!("={}", self.body_text()),
        }
    }

    /// The text the model stores (on the glyph, or on the layer for a local key): `=` and the
    /// body, or the bare name.
    pub fn stored(&self) -> String {
        if self.is_bare() { self.body_text() } else { format!("={}", self.body_text()) }
    }

    /// The glyph the key reads, if it names one.
    pub fn glyph(&self) -> Option<&str> {
        match &self.body {
            Body::Name { name, .. } => Some(name),
            _ => None,
        }
    }
}

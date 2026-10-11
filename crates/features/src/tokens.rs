//! Number value and glyph property tokens, replaced by text before compiling (M8.5;
//! `docs/spec/12-feature-code.md` § 3).
//!
//! Rows: `token.kinds`, `token.expansionTiming`, `token.numberValue.*`, `token.glyphProperty.*`,
//! `kerning.kernCodeTokens`, and the data of `font.features.evaluateToken`.
//!
//! `$name`, `${name * 2 + 10}`, `${glyph:width}`, `${glyph:anchors.top.x / 2}` are evaluated
//! against a [`TargetValues`]; predicate tokens `$[…]` go to [`crate::tokens_predicate`].
//! Arithmetic: `+ - * /`, `*` and `/` first, parentheses, decimal and negative literals;
//! results rounded half away from zero; division by zero, a number that is not finite or a
//! result beyond ±[`MAX_TOKEN_VALUE`] is an error at the token.
//!
//! Reading the code:
//! - A `$` starts a token everywhere except in a comment (`#` to the end of the line) and in a
//!   double-quoted string. A `$` that starts no token is an error.
//! - `$name`: letters, digits and `_`. `${…}` closes at the first `}` and may not cross a line,
//!   a `{`, a `;` or another `$`. `$[…]` closes at the first `]` outside quotes and may not
//!   cross a `;`.
//! - Inside `${glyph:…}` the text after the first `:` is an expression whose names are glyph
//!   properties (`width`, `LSB`, `RSB`, `TSB`, `BSB`, `anchors.<name>.x` and `.y`) or, failing
//!   that, number values. Any name may come anywhere in an expression; literals may come first.
//! - Every error is reported at its token; expansion goes on to report the others, up to
//!   [`MAX_DIAGNOSTICS`].

use std::collections::HashMap;
use std::ops::Range;

use fontcraft_fea::{Diagnostic, Severity};

use crate::source_map::OffsetMap;
use crate::target::GlyphMetric;
use crate::{MAX_ENTRY_BYTES, TargetValues};

/// Most tokens in one entry.
pub const MAX_TOKENS: usize = 100_000;
/// Most bytes of one token's expression.
pub const MAX_EXPRESSION_BYTES: usize = 4_096;
/// Largest magnitude a number token may give: font units fit 16 bits.
pub const MAX_TOKEN_VALUE: f64 = 32_767.0;
/// Deepest nesting of parentheses and signs in one expression.
pub const MAX_EXPRESSION_DEPTH: usize = 64;
/// Most bytes the expanded code of one entry may hold: the assembled text of one target.
pub const MAX_EXPANDED_BYTES: usize = 16 << 20;
/// Most glyph tests the predicate tokens of one entry may make together: each distinct
/// predicate tests every glyph of the target once.
pub const MAX_PREDICATE_WORK: usize = 50_000_000;
/// Most diagnostics one expansion reports.
pub const MAX_DIAGNOSTICS: usize = 1_000;

/// What one token evaluates to.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenValue {
    /// A number or property token, rounded.
    Number(i32),
    /// A predicate token: the glyphs it selects, in the font's order.
    Glyphs(Vec<String>),
}

/// `code` with every token replaced by its value, and the map back to `code`. Diagnostics have
/// no file; their spans are in `code` (the token's range).
pub fn expand(code: &str, values: &dyn TargetValues) -> Result<(String, OffsetMap), Vec<Diagnostic>> {
    if code.len() > MAX_ENTRY_BYTES {
        return Err(vec![error(0..0, format!("the feature code is longer than {MAX_ENTRY_BYTES} bytes"))]);
    }
    let scanned = scan(code, MAX_TOKENS);
    if scanned.tokens.is_empty() && scanned.beyond.is_none() {
        return Ok((code.to_string(), OffsetMap::identity()));
    }

    let mut errors = Vec::new();
    if let Some(at) = scanned.beyond {
        errors.push(error(at..at.saturating_add(1), format!("the code has more than {MAX_TOKENS} tokens")));
    }
    let mut evaluator = Evaluator::new(values);
    let mut out = String::with_capacity(code.len());
    let mut map = OffsetMap::identity();
    let mut copied = 0;
    for token in &scanned.tokens {
        if errors.len() >= MAX_DIAGNOSTICS {
            break;
        }
        let text = match evaluator.text(code, token) {
            Ok(text) => text,
            Err(message) => {
                errors.push(error(token.range.clone(), message));
                continue;
            }
        };
        if !errors.is_empty() {
            continue;
        }
        let before = code.get(copied..token.range.start).unwrap_or_default();
        if out.len().saturating_add(before.len()).saturating_add(text.len()) > MAX_EXPANDED_BYTES {
            errors.push(error(token.range.clone(), format!("the expanded code is longer than {MAX_EXPANDED_BYTES} bytes")));
            break;
        }
        copy(&mut out, &mut map, before, copied);
        let start = out.len();
        out.push_str(&text);
        if !text.is_empty() {
            map.push(start..out.len(), token.range.clone());
        }
        copied = token.range.end;
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let rest = code.get(copied..).unwrap_or_default();
    if out.len().saturating_add(rest.len()) > MAX_EXPANDED_BYTES {
        return Err(vec![error(copied..code.len(), format!("the expanded code is longer than {MAX_EXPANDED_BYTES} bytes"))]);
    }
    copy(&mut out, &mut map, rest, copied);
    Ok((out, map))
}

/// Evaluates one token's text (`$name`, `${…}`, `$[…]`), for Option-click and
/// `font.features.evaluateToken`. The error is a message for the user.
pub fn evaluate(token: &str, values: &dyn TargetValues) -> Result<TokenValue, String> {
    let text = token.trim();
    if text.len() > MAX_EXPRESSION_BYTES.saturating_add(3) {
        return Err(format!("the token is longer than {MAX_EXPRESSION_BYTES} bytes"));
    }
    let scanned = scan(text, 2);
    let [one] = scanned.tokens.as_slice() else {
        return Err(format!("`{text}` is not one token"));
    };
    if one.range != (0..text.len()) {
        return Err(format!("`{text}` is not one token"));
    }
    let mut evaluator = Evaluator::new(values);
    match &one.body {
        Body::Predicate(inner) => evaluator.predicate(text, inner.clone()).map(TokenValue::Glyphs),
        _ => evaluator.number(text, one).map(TokenValue::Number),
    }
}

/// The byte ranges of the tokens in `code`, in order, at most [`MAX_TOKENS`] (syntax colouring,
/// the token under the pointer).
pub fn token_ranges(code: &str) -> Vec<Range<usize>> {
    scan(code, MAX_TOKENS).tokens.into_iter().map(|t| t.range).collect()
}

fn error(span: Range<usize>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::at(String::new(), span, Severity::Error, message)
}

/// Appends `text`, copied from `input_start` of the input, and records it byte for byte.
fn copy(out: &mut String, map: &mut OffsetMap, text: &str, input_start: usize) {
    if text.is_empty() {
        return;
    }
    let start = out.len();
    out.push_str(text);
    map.push(start..out.len(), input_start..input_start.saturating_add(text.len()));
}

// ---------------------------------------------------------------------------------------------
// Finding the tokens
// ---------------------------------------------------------------------------------------------

/// What a token holds, as ranges of the scanned text.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Body {
    /// `$name`: the name.
    Name(Range<usize>),
    /// `${…}`: the text between the braces.
    Expression(Range<usize>),
    /// `$[…]`: the text between the brackets.
    Predicate(Range<usize>),
    /// A `$` that starts no complete token, with the message.
    Broken(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    range: Range<usize>,
    body: Body,
}

struct Scanned {
    tokens: Vec<Token>,
    /// The offset of the first token past the limit.
    beyond: Option<usize>,
}

fn scan(code: &str, limit: usize) -> Scanned {
    let b = code.as_bytes();
    let mut tokens = Vec::new();
    let mut beyond = None;
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'#' => i = find(b, i, |c| c == b'\n').unwrap_or(b.len()),
            b'"' => i = find(b, i.saturating_add(1), |c| c == b'"').map_or(b.len(), |e| e.saturating_add(1)),
            b'$' => {
                if tokens.len() >= limit {
                    beyond = Some(i);
                    break;
                }
                let token = scan_token(code, i);
                i = token.range.end.max(i.saturating_add(1));
                tokens.push(token);
            }
            _ => i = i.saturating_add(1),
        }
    }
    Scanned { tokens, beyond }
}

/// The first offset at or after `from` whose byte satisfies `stop`.
fn find(b: &[u8], from: usize, stop: impl Fn(u8) -> bool) -> Option<usize> {
    b.get(from..)?.iter().position(|&c| stop(c)).map(|p| p.saturating_add(from))
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The token whose `$` is at `start`.
fn scan_token(code: &str, start: usize) -> Token {
    let b = code.as_bytes();
    let open = start.saturating_add(1);
    let inner = open.saturating_add(1);
    match b.get(open) {
        Some(b'{') => match find(b, inner, |c| matches!(c, b'}' | b'\n' | b'{' | b';' | b'$')) {
            Some(end) if b.get(end) == Some(&b'}') => {
                let body = if end - inner > MAX_EXPRESSION_BYTES {
                    Body::Broken("the token's expression is longer than 4096 bytes")
                } else {
                    Body::Expression(inner..end)
                };
                Token { range: start..end.saturating_add(1), body }
            }
            stop => Token { range: start..stop.unwrap_or(b.len()), body: Body::Broken("the token is not closed with `}`") },
        },
        Some(b'[') => {
            let mut quote = None;
            let mut j = inner;
            let stop = loop {
                let Some(&c) = b.get(j) else { break None };
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b']' || c == b';' => break Some(j),
                    None => {}
                }
                j = j.saturating_add(1);
            };
            match stop {
                Some(end) if b.get(end) == Some(&b']') => {
                    let body = if end - inner > MAX_EXPRESSION_BYTES {
                        Body::Broken("the token's predicate is longer than 4096 bytes")
                    } else {
                        Body::Predicate(inner..end)
                    };
                    Token { range: start..end.saturating_add(1), body }
                }
                stop => Token { range: start..stop.unwrap_or(b.len()), body: Body::Broken("the token is not closed with `]`") },
            }
        }
        _ => {
            let rest = code.get(open..).unwrap_or_default();
            let len = rest.char_indices().find(|&(_, c)| !is_name_char(c)).map_or(rest.len(), |(i, _)| i);
            if len == 0 {
                Token { range: start..open, body: Body::Broken("a `$` must start a token: `$name`, `${…}` or `$[…]`") }
            } else if len > MAX_EXPRESSION_BYTES {
                Token { range: start..open + len, body: Body::Broken("the token's name is longer than 4096 bytes") }
            } else {
                Token { range: start..open + len, body: Body::Name(open..open + len) }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Evaluating
// ---------------------------------------------------------------------------------------------

struct Evaluator<'v> {
    values: &'v dyn TargetValues,
    /// The text each distinct predicate expanded to.
    predicates: HashMap<String, Result<String, String>>,
    /// Glyph tests spent on predicates ([`MAX_PREDICATE_WORK`]).
    work: usize,
}

impl<'v> Evaluator<'v> {
    fn new(values: &'v dyn TargetValues) -> Self {
        Self { values, predicates: HashMap::new(), work: 0 }
    }

    /// The text `token` of `code` is replaced by.
    fn text(&mut self, code: &str, token: &Token) -> Result<String, String> {
        match &token.body {
            Body::Predicate(inner) => {
                let key = code.get(inner.clone()).unwrap_or_default().trim();
                if let Some(done) = self.predicates.get(key) {
                    return done.clone();
                }
                let result = self.predicate(code, inner.clone()).map(|names| names.join(" "));
                self.predicates.insert(key.to_string(), result.clone());
                result
            }
            _ => self.number(code, token).map(|v| v.to_string()),
        }
    }

    fn predicate(&mut self, code: &str, inner: Range<usize>) -> Result<Vec<String>, String> {
        self.work = self.work.saturating_add(self.values.glyphs().len().max(1));
        if self.work > MAX_PREDICATE_WORK {
            return Err(format!("the predicate tokens test more than {MAX_PREDICATE_WORK} glyphs together"));
        }
        let text = code.get(inner).unwrap_or_default();
        crate::tokens_predicate::glyphs(text, self.values)
    }

    /// The value of a number or property token.
    fn number(&self, code: &str, token: &Token) -> Result<i32, String> {
        let value = match &token.body {
            Body::Name(name) => {
                let name = code.get(name.clone()).unwrap_or_default();
                number_value(self.values, name)?
            }
            Body::Expression(inner) => {
                let text = code.get(inner.clone()).unwrap_or_default();
                match text.split_once(':') {
                    Some((glyph, expression)) => {
                        let glyph = glyph.trim();
                        if glyph.is_empty() {
                            return Err("a glyph property token needs a glyph name before `:`".into());
                        }
                        if expression.trim().is_empty() {
                            return Err(format!("the token needs a property of `{glyph}` after `:`"));
                        }
                        if self.values.glyph_metric(glyph, GlyphMetric::Width).is_none() {
                            return Err(format!("there is no glyph `{glyph}` with a layer for this target"));
                        }
                        calculate(expression, &Names { values: self.values, glyph: Some(glyph) })?
                    }
                    None => calculate(text, &Names { values: self.values, glyph: None })?,
                }
            }
            Body::Predicate(_) => return Err("a predicate token gives glyph names, not a number".into()),
            Body::Broken(message) => return Err((*message).into()),
        };
        round(value)
    }
}

/// Rounds half away from zero and checks the range.
fn round(value: f64) -> Result<i32, String> {
    let r = value.round();
    if !r.is_finite() || r.abs() > MAX_TOKEN_VALUE {
        return Err(format!("the token's value {value} is beyond ±{MAX_TOKEN_VALUE}"));
    }
    Ok(r as i32)
}

fn number_value(values: &dyn TargetValues, name: &str) -> Result<f64, String> {
    match values.number_value(name) {
        Some(v) if v.is_finite() => Ok(v),
        Some(_) => Err(format!("the number value `{name}` is not a finite number")),
        None => Err(format!("the font has no number value `{name}`")),
    }
}

/// What the names of an expression read: number values, and in a property token the glyph's
/// properties first.
struct Names<'a> {
    values: &'a dyn TargetValues,
    glyph: Option<&'a str>,
}

impl Names<'_> {
    fn get(&self, name: &str) -> Result<f64, String> {
        if let Some(glyph) = self.glyph {
            if let Some(metric) = GlyphMetric::from_name(name) {
                return self
                    .values
                    .glyph_metric(glyph, metric)
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("`{glyph}` has no outline in this target, so it has no {name}"));
            }
            if let Some(path) = name.strip_prefix("anchors.") {
                let Some((anchor, axis)) = path.rsplit_once('.').filter(|(a, axis)| !a.is_empty() && (*axis == "x" || *axis == "y")) else {
                    return Err(format!("`{name}` is not an anchor property: write `anchors.<name>.x` or `anchors.<name>.y`"));
                };
                let (x, y) = self
                    .values
                    .anchor(glyph, anchor)
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .ok_or_else(|| format!("`{glyph}` has no anchor `{anchor}` in this target"))?;
                return Ok(if axis == "x" { x } else { y });
            }
            if name.contains('.') || self.values.number_value(name).is_none() {
                return Err(format!("`{name}` is not a glyph property (width, LSB, RSB, TSB, BSB, anchors.<name>.x or .y) or a number value"));
            }
        }
        if name.contains('.') {
            return Err(format!("`{name}` is not a number value name"));
        }
        number_value(self.values, name)
    }
}

/// One item of an expression.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Item<'a> {
    Number(f64),
    Name(&'a str),
    Op(char),
    Open,
    Close,
}

impl Item<'_> {
    fn describe(&self) -> String {
        match self {
            Item::Number(v) => format!("`{v}`"),
            Item::Name(n) => format!("`{n}`"),
            Item::Op(c) => format!("`{c}`"),
            Item::Open => "`(`".into(),
            Item::Close => "`)`".into(),
        }
    }
}

fn lex(text: &str) -> Result<Vec<Item<'_>>, String> {
    let mut items = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '+' | '-' | '*' | '/' => items.push(Item::Op(c)),
            '(' => items.push(Item::Open),
            ')' => items.push(Item::Close),
            c if c.is_ascii_digit() || c == '.' => {
                let mut end = i + c.len_utf8();
                while let Some(&(j, d)) = chars.peek() {
                    if !(d.is_ascii_digit() || d == '.') {
                        break;
                    }
                    end = j + d.len_utf8();
                    chars.next();
                }
                let literal = text.get(i..end).unwrap_or_default();
                let v: f64 = literal.parse().map_err(|_| format!("`{literal}` is not a number"))?;
                if !v.is_finite() || v.abs() > f64::from(i32::MAX) {
                    return Err(format!("the number `{literal}` is too large"));
                }
                items.push(Item::Number(v));
            }
            c if is_name_char(c) => {
                let mut end = i + c.len_utf8();
                while let Some(&(j, d)) = chars.peek() {
                    if !(is_name_char(d) || d == '.') {
                        break;
                    }
                    end = j + d.len_utf8();
                    chars.next();
                }
                items.push(Item::Name(text.get(i..end).unwrap_or_default()));
            }
            c => return Err(format!("`{c}` cannot be used in a token's expression")),
        }
    }
    Ok(items)
}

/// The value of an arithmetic expression, not rounded.
fn calculate(text: &str, names: &Names) -> Result<f64, String> {
    let items = lex(text)?;
    if items.is_empty() {
        return Err("the token is empty".into());
    }
    let mut parser = Parser { items: &items, pos: 0, names };
    let value = parser.sum(0)?;
    if let Some(item) = items.get(parser.pos) {
        return Err(format!("{} is not expected here", item.describe()));
    }
    Ok(value)
}

struct Parser<'i, 'n> {
    items: &'i [Item<'i>],
    pos: usize,
    names: &'n Names<'n>,
}

fn finite(v: f64) -> Result<f64, String> {
    if v.is_finite() { Ok(v) } else { Err("the token's value is too large".into()) }
}

impl<'i> Parser<'i, '_> {
    fn peek(&self) -> Option<Item<'i>> {
        self.items.get(self.pos).copied()
    }

    fn sum(&mut self, depth: usize) -> Result<f64, String> {
        let mut v = self.product(depth)?;
        while let Some(Item::Op(op @ ('+' | '-'))) = self.peek() {
            self.pos += 1;
            let r = self.product(depth)?;
            v = finite(if op == '+' { v + r } else { v - r })?;
        }
        Ok(v)
    }

    fn product(&mut self, depth: usize) -> Result<f64, String> {
        let mut v = self.signed(depth)?;
        while let Some(Item::Op(op @ ('*' | '/'))) = self.peek() {
            self.pos += 1;
            let r = self.signed(depth)?;
            v = if op == '*' {
                finite(v * r)?
            } else if r == 0.0 {
                return Err("the token divides by zero".into());
            } else {
                finite(v / r)?
            };
        }
        Ok(v)
    }

    fn signed(&mut self, depth: usize) -> Result<f64, String> {
        if depth >= MAX_EXPRESSION_DEPTH {
            return Err(format!("the token's expression nests more than {MAX_EXPRESSION_DEPTH} deep"));
        }
        match self.peek() {
            Some(Item::Op('-')) => {
                self.pos += 1;
                Ok(-self.signed(depth + 1)?)
            }
            Some(Item::Op('+')) => {
                self.pos += 1;
                self.signed(depth + 1)
            }
            _ => self.operand(depth),
        }
    }

    fn operand(&mut self, depth: usize) -> Result<f64, String> {
        let item = self.peek().ok_or_else(|| "the token's expression ends too early".to_string())?;
        self.pos += 1;
        match item {
            Item::Number(v) => Ok(v),
            Item::Name(name) => self.names.get(name),
            Item::Open => {
                let v = self.sum(depth + 1)?;
                match self.peek() {
                    Some(Item::Close) => {
                        self.pos += 1;
                        Ok(v)
                    }
                    Some(other) => Err(format!("{} is not expected here: a `)` is missing", other.describe())),
                    None => Err("a `)` is missing".into()),
                }
            }
            other => Err(format!("{} is not expected here", other.describe())),
        }
    }
}

#[cfg(test)]
#[path = "tokens_tests.rs"]
mod tests;

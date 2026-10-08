//! The OpenStep text property list, as the primary reference's source format writes it.
//!
//! Syntax: `{ key = value; … }` dictionaries, `( a, b )` arrays, `<0a1b>` data, and strings that
//! are either bare tokens or double-quoted with backslash escapes. There are no numbers in the
//! syntax as such; a bare token that reads as a decimal number is a [`Number`] here, and a quoted
//! one stays a string, which is how the reference tells the two apart.
//!
//! Canonical spelling, measured on files the reference app saved (`docs/plans/M1.md` § 8):
//!
//! - A string is written bare when it consists only of ASCII letters, digits, `.` and `_`, has
//!   at least one letter or digit, and does not read as a number ([`is_number_like`]). Otherwise
//!   it is quoted, with `\` and `"` escaped. Format versions 3 and 4 write every other character
//!   literally, line breaks and tabs included; version 2 writes line breaks as the octal escapes
//!   `\012` and `\015` and tabs literally.
//! - A number is written as the shortest decimal text that reads back to the same value, without
//!   an exponent; integers without a decimal point.
//! - Data is written as lowercase hexadecimal digits without spaces.
//! - Multi-line containers put one element per line with no indentation; dictionary entries end
//!   in `;`, array elements are separated by `,`. Version 4 of the format also puts a `,` after
//!   the last element of a multi-line array. Compact arrays are written `(a,b)`.
//!
//! Comments (`//`, `/* */`) are accepted on read and not kept.

use std::borrow::Cow;

use crate::{Array, Data, Dict, Error, Layout, MAX_DEPTH, MAX_INPUT, Num, Number, Str, Value};

/// The writer conventions of one format version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// Format version 2: line breaks in strings as octal escapes.
    Glyphs2,
    /// Format version 3.
    Glyphs3,
    /// Format version 4: a comma after the last element of a multi-line array.
    Glyphs4,
}

/// A parsed file: the root value and what surrounds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub root: Value,
    /// The file began with a UTF-8 byte-order mark.
    pub bom: bool,
    /// The file used CRLF line ends (converted to LF while parsing, restored on writing).
    pub crlf: bool,
    /// Whitespace before the root value (normally none).
    pub leading: String,
    /// Whitespace after the root value (normally one line break).
    pub trailing: String,
}

impl Document {
    pub fn new(root: Value) -> Self {
        Self { root, bom: false, crlf: false, leading: String::new(), trailing: "\n".into() }
    }
}

/// Parses a whole file.
pub fn parse(text: &str) -> Result<Document, Error> {
    if text.len() > MAX_INPUT {
        return Err(Error { line: 1, column: 1, message: format!("input is larger than {MAX_INPUT} bytes") });
    }
    let (bom, text) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let crlf = text.contains("\r\n");
    let text: Cow<'_, str> = if crlf { Cow::Owned(text.replace("\r\n", "\n")) } else { Cow::Borrowed(text) };
    let mut p = Parser { text: &text, bytes: text.as_bytes(), pos: 0 };
    let start = p.pos;
    p.skip_ws()?;
    let leading = p.slice(start, p.pos).to_string();
    let root = p.value(0)?;
    let after = p.pos;
    p.skip_ws()?;
    if p.pos < p.bytes.len() {
        return Err(p.err("unexpected text after the end of the document"));
    }
    let trailing = p.slice(after, p.pos).to_string();
    Ok(Document { root, bom, crlf, leading, trailing })
}

/// Parses a single value, for tests and for values embedded in other text.
pub fn parse_value(text: &str) -> Result<Value, Error> {
    parse(text).map(|d| d.root)
}

/// Reads one token (bare or quoted) as a string, keeping its spelling when it is not the
/// canonical one: the way to rebuild a dictionary key exactly as a file wrote it.
pub fn parse_str_token(token: &str) -> Result<Str, Error> {
    let mut p = Parser { text: token, bytes: token.as_bytes(), pos: 0 };
    let s = match p.peek() {
        Some(b'"') => p.quoted()?,
        _ => p.key()?,
    };
    if p.pos != token.len() {
        return Err(p.err("expected a single string token"));
    }
    Ok(s)
}

/// The text a string is written as: its spelling, or the canonical form.
pub fn str_token(s: &Str) -> String {
    let mut out = String::new();
    write_str(&mut out, s, Dialect::Glyphs3);
    out
}

/// Writes a document. For a document that [`parse`] returned unchanged, the result is the input.
pub fn write(doc: &Document, dialect: Dialect) -> String {
    let mut out = String::new();
    if doc.bom {
        out.push('\u{feff}');
    }
    out.push_str(&doc.leading);
    write_value(&mut out, &doc.root, dialect);
    out.push_str(&doc.trailing);
    if doc.crlf { out.replace('\n', "\r\n") } else { out }
}

/// Writes one value the way it would appear in a file.
pub fn write_value(out: &mut String, value: &Value, dialect: Dialect) {
    match value {
        Value::Dict(d) => {
            out.push('{');
            let multiline = d.layout == Layout::Multiline;
            if multiline {
                out.push('\n');
            }
            for (i, (k, v)) in d.entries.iter().enumerate() {
                if !multiline && i > 0 {
                    out.push(' ');
                }
                write_str(out, k, dialect);
                out.push_str(" = ");
                write_value(out, v, dialect);
                out.push(';');
                if multiline {
                    out.push('\n');
                }
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('(');
            match a.layout {
                Layout::Multiline => {
                    out.push('\n');
                    for (i, v) in a.items.iter().enumerate() {
                        if i > 0 {
                            out.push_str(",\n");
                        }
                        write_value(out, v, dialect);
                    }
                    if !a.items.is_empty() {
                        if dialect == Dialect::Glyphs4 {
                            out.push(',');
                        }
                        out.push('\n');
                    }
                }
                Layout::Compact => {
                    for (i, v) in a.items.iter().enumerate() {
                        if i > 0 {
                            out.push(',');
                        }
                        write_value(out, v, dialect);
                    }
                }
            }
            out.push(')');
        }
        Value::String(s) => write_str(out, s, dialect),
        Value::Number(n) => match n.spelling() {
            Some(sp) => out.push_str(sp),
            None => out.push_str(&canonical_number(n.value())),
        },
        Value::Data(d) => match d.spelling() {
            Some(sp) => out.push_str(sp),
            None => out.push_str(&canonical_data(d.bytes())),
        },
    }
}

fn write_str(out: &mut String, s: &Str, dialect: Dialect) {
    match s.spelling() {
        Some(sp) => out.push_str(sp),
        None if dialect == Dialect::Glyphs2 => out.push_str(&canonical_string_v2(s.as_str())),
        None => out.push_str(&canonical_string(s.as_str())),
    }
}

/// Whether `s` reads as a decimal number: an optional minus sign, digits with at most one
/// decimal point, and at least one digit. A leading zero followed by another digit (`0041`) does
/// not: no writer spells a number that way, and the reference's version 2 files use such tokens
/// for hexadecimal code points.
pub fn is_number_like(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    let b = body.as_bytes();
    if b.first() == Some(&b'0') && b.get(1).is_some_and(u8::is_ascii_digit) {
        return false;
    }
    let mut digits = 0usize;
    let mut points = 0usize;
    for b in body.bytes() {
        match b {
            b'0'..=b'9' => digits += 1,
            b'.' => points += 1,
            _ => return false,
        }
    }
    digits > 0 && points <= 1
}

fn is_bare_safe(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_') && s.bytes().any(|b| b.is_ascii_alphanumeric()) && !is_number_like(s)
}

/// The canonical spelling of a string, quotes included where needed.
pub fn canonical_string(s: &str) -> String {
    if is_bare_safe(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The canonical spelling of a string in format version 2: as [`canonical_string`], with line
/// breaks written as three-digit octal escapes.
pub fn canonical_string_v2(s: &str) -> String {
    if !s.contains(['\n', '\r']) {
        return canonical_string(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\012"),
            '\r' => out.push_str("\\015"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The canonical spelling of a number.
pub fn canonical_number(n: Num) -> String {
    match n {
        Num::Int(i) => i.to_string(),
        // Rust's float formatting is the shortest text that reads back exactly, never with an
        // exponent; whole values come out without a decimal point.
        Num::Float(f) => format!("{f}"),
    }
}

/// The canonical spelling of data.
pub fn canonical_data(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push('<');
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out.push('>');
    out
}

fn spelling_if_not(canonical: &str, source: &str) -> Option<Box<str>> {
    (canonical != source).then(|| source.into())
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

fn is_delimiter(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'{' | b'}' | b'(' | b')' | b';' | b'=' | b',' | b'"' | b'<' | b'>')
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn slice(&self, a: usize, b: usize) -> &'a str {
        self.text.get(a..b).unwrap_or("")
    }

    fn err(&self, message: impl Into<String>) -> Error {
        Error::at(self.text, self.pos, message)
    }

    fn skip_ws(&mut self) -> Result<(), Error> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(b) = self.peek() {
                        if b == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        match self.peek() {
                            None => return Err(Error::at(self.text, start, "unclosed comment")),
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.pos += 2;
                                break;
                            }
                            Some(_) => self.pos += 1,
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn expect(&mut self, b: u8, what: &str) -> Result<(), Error> {
        self.skip_ws()?;
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected {what}")))
        }
    }

    fn layout_after_open(&self) -> Layout {
        if self.peek() == Some(b'\n') { Layout::Multiline } else { Layout::Compact }
    }

    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        self.skip_ws()?;
        match self.peek() {
            Some(b'{') => self.dict(depth),
            Some(b'(') => self.array(depth),
            Some(b'<') => self.data(),
            Some(b'"') => self.quoted().map(Value::String),
            Some(_) => self.bare(),
            None => Err(self.err("expected a value, found the end of the input")),
        }
    }

    fn dict(&mut self, depth: usize) -> Result<Value, Error> {
        if depth >= MAX_DEPTH {
            return Err(self.err(format!("nested more than {MAX_DEPTH} levels deep")));
        }
        self.pos += 1;
        let mut dict = Dict { entries: Vec::new(), layout: self.layout_after_open() };
        loop {
            self.skip_ws()?;
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Dict(dict));
                }
                None => return Err(self.err("unclosed dictionary")),
                Some(_) => {}
            }
            let key = self.key()?;
            self.expect(b'=', "`=` after a dictionary key")?;
            let value = self.value(depth + 1)?;
            self.expect(b';', "`;` after a dictionary value")?;
            dict.entries.push((key, value));
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, Error> {
        if depth >= MAX_DEPTH {
            return Err(self.err(format!("nested more than {MAX_DEPTH} levels deep")));
        }
        self.pos += 1;
        let mut array = Array { items: Vec::new(), layout: self.layout_after_open() };
        loop {
            self.skip_ws()?;
            match self.peek() {
                Some(b')') => {
                    self.pos += 1;
                    return Ok(Value::Array(array));
                }
                None => return Err(self.err("unclosed array")),
                Some(_) => {}
            }
            array.items.push(self.value(depth + 1)?);
            self.skip_ws()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b')') => {}
                _ => return Err(self.err("expected `,` or `)` in an array")),
            }
        }
    }

    fn key(&mut self) -> Result<Str, Error> {
        match self.peek() {
            Some(b'"') => self.quoted(),
            _ => {
                let token = self.bare_token()?;
                Ok(Str::with_spelling(token.to_string(), spelling_if_not(&canonical_string(token), token)))
            }
        }
    }

    fn bare_token(&mut self) -> Result<&'a str, Error> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_delimiter(b) {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.err("expected a value"));
        }
        Ok(self.slice(start, self.pos))
    }

    fn bare(&mut self) -> Result<Value, Error> {
        let token = self.bare_token()?;
        if is_number_like(token) {
            let num = if token.contains('.') { None } else { token.parse::<i64>().ok().map(Num::Int) };
            let num = match num {
                Some(n) => n,
                None => match token.parse::<f64>() {
                    Ok(f) if f.is_finite() => Num::Float(f),
                    _ => return Ok(Value::String(Str::with_spelling(token.to_string(), Some(token.into())))),
                },
            };
            return Ok(Value::Number(Number::with_spelling(num, spelling_if_not(&canonical_number(num), token))));
        }
        Ok(Value::String(Str::with_spelling(token.to_string(), spelling_if_not(&canonical_string(token), token))))
    }

    fn quoted(&mut self) -> Result<Str, Error> {
        let start = self.pos;
        self.pos += 1;
        let mut text = String::new();
        let mut run = self.pos;
        loop {
            match self.peek() {
                None => return Err(Error::at(self.text, start, "unclosed string")),
                Some(b'"') => {
                    text.push_str(self.slice(run, self.pos));
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    text.push_str(self.slice(run, self.pos));
                    self.pos += 1;
                    self.escape(&mut text)?;
                    run = self.pos;
                }
                Some(_) => self.pos += 1,
            }
        }
        let source = self.slice(start, self.pos);
        let spelling = spelling_if_not(&canonical_string(&text), source);
        Ok(Str::with_spelling(text, spelling))
    }

    /// Decodes one escape; `pos` is just after the backslash.
    fn escape(&mut self, text: &mut String) -> Result<(), Error> {
        let Some(b) = self.peek() else { return Err(self.err("unclosed string")) };
        match b {
            b'n' => text.push('\n'),
            b't' => text.push('\t'),
            b'r' => text.push('\r'),
            b'a' => text.push('\u{7}'),
            b'b' => text.push('\u{8}'),
            b'f' => text.push('\u{c}'),
            b'v' => text.push('\u{b}'),
            b'0'..=b'7' => {
                let mut value = 0u32;
                let mut n = 0;
                while n < 3 {
                    match self.peek() {
                        Some(d @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(d - b'0');
                            self.pos += 1;
                            n += 1;
                        }
                        _ => break,
                    }
                }
                // Values above 127 are NeXTSTEP-encoded bytes; they are kept as the code point
                // of the same number, and the source spelling preserves the original.
                text.push(char::from_u32(value).unwrap_or('\u{fffd}'));
                return Ok(());
            }
            b'U' | b'u' => {
                self.pos += 1;
                let unit = self.hex4();
                let c = match unit {
                    Some(hi @ 0xd800..=0xdbff) => {
                        // A high surrogate must be followed by `\U` and a low surrogate.
                        let save = self.pos;
                        let low = if self.peek() == Some(b'\\') && matches!(self.peek_at(1), Some(b'U' | b'u')) {
                            self.pos += 2;
                            self.hex4()
                        } else {
                            None
                        };
                        match low {
                            Some(lo @ 0xdc00..=0xdfff) => char::from_u32(0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)),
                            _ => {
                                self.pos = save;
                                None
                            }
                        }
                    }
                    Some(u) => char::from_u32(u),
                    None => None,
                };
                text.push(c.unwrap_or('\u{fffd}'));
                return Ok(());
            }
            _ => {
                // `\\`, `\"` and any other escaped character stand for themselves.
                let rest = self.text.get(self.pos..).unwrap_or("");
                if let Some(c) = rest.chars().next() {
                    text.push(c);
                    self.pos += c.len_utf8();
                }
                return Ok(());
            }
        }
        self.pos += 1;
        Ok(())
    }

    /// Up to four hexadecimal digits.
    fn hex4(&mut self) -> Option<u32> {
        let mut value = 0u32;
        let mut n = 0;
        while n < 4 {
            match self.peek().and_then(|b| char::from(b).to_digit(16)) {
                Some(d) => {
                    value = value * 16 + d;
                    self.pos += 1;
                    n += 1;
                }
                None => break,
            }
        }
        (n > 0).then_some(value)
    }

    fn data(&mut self) -> Result<Value, Error> {
        let start = self.pos;
        self.pos += 1;
        let mut bytes = Vec::new();
        let mut high: Option<u32> = None;
        loop {
            match self.peek() {
                None => return Err(Error::at(self.text, start, "unclosed data")),
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b) => {
                    let Some(d) = char::from(b).to_digit(16) else { return Err(self.err("expected a hexadecimal digit in data")) };
                    self.pos += 1;
                    match high.take() {
                        None => high = Some(d),
                        // Both digits are below 16, so the byte fits.
                        Some(h) => bytes.push(u8::try_from(h * 16 + d).unwrap_or(0)),
                    }
                }
            }
        }
        if high.is_some() {
            return Err(Error::at(self.text, start, "data has an odd number of hexadecimal digits"));
        }
        let source = self.slice(start, self.pos);
        let spelling = spelling_if_not(&canonical_data(&bytes), source);
        Ok(Value::Data(Data::with_spelling(bytes, spelling)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(text: &str, dialect: Dialect) -> String {
        write(&parse(text).unwrap(), dialect)
    }

    #[test]
    fn a_reference_style_document_round_trips() {
        let text = "{\n.appVersion = \"3343\";\n.formatVersion = 3;\naxes = (\n{\nname = Weight;\ntag = wght;\n}\n);\nguides = (\n{\npos = (-25,193);\n}\n);\nvalue = (\n);\n}\n";
        assert_eq!(roundtrip(text, Dialect::Glyphs3), text);
        let parsed = parse(text).unwrap();
        assert_eq!(parsed.root.spelling_count(), 0, "every token here is canonical");
    }

    #[test]
    fn version_4_puts_a_comma_after_the_last_element() {
        let text = "{\nlist = (\n{\nname = a;\n},\n{\nname = b;\n},\n);\n}\n";
        assert_eq!(roundtrip(text, Dialect::Glyphs4), text);
    }

    #[test]
    fn strings_and_numbers_are_told_apart_by_quoting() {
        let v = parse_value("{\na = 600;\nb = \"600\";\nc = 1.5;\nd = -3;\ne = A.ss01;\n}").unwrap();
        let d = v.as_dict().unwrap();
        assert_eq!(d.get("a").unwrap().as_num(), Some(Num::Int(600)));
        assert_eq!(d.get("b").unwrap().as_str(), Some("600"));
        assert_eq!(d.get("c").unwrap().as_num(), Some(Num::Float(1.5)));
        assert_eq!(d.get("d").unwrap().as_num(), Some(Num::Int(-3)));
        assert_eq!(d.get("e").unwrap().as_str(), Some("A.ss01"));
        let hex = parse_value("{\nunicode = 0041;\n}").unwrap();
        assert_eq!(hex.as_dict().unwrap().get("unicode").unwrap().as_str(), Some("0041"));
        assert_eq!(hex.spelling_count(), 0);
        assert_eq!(v.spelling_count(), 0);
    }

    #[test]
    fn non_canonical_tokens_keep_their_spelling() {
        let text = "{\na = \"m01\";\nb = 1.0;\nc = \"line\\012break\";\nd = a-b;\n}\n";
        let doc = parse(text).unwrap();
        assert_eq!(doc.root.spelling_count(), 4);
        assert_eq!(write(&doc, Dialect::Glyphs3), text);
        assert_eq!(doc.root.as_dict().unwrap().get("c").unwrap().as_str(), Some("line\nbreak"));
    }

    #[test]
    fn canonical_quoting() {
        assert_eq!(canonical_string("Regular"), "Regular");
        assert_eq!(canonical_string(".notdef"), ".notdef");
        assert_eq!(canonical_string(".."), "\"..\"");
        assert_eq!(canonical_string("600"), "\"600\"");
        assert_eq!(canonical_string("-1.5"), "\"-1.5\"");
        assert_eq!(canonical_string("alef-ar"), "\"alef-ar\"");
        assert_eq!(canonical_string("/A"), "\"/A\"");
        assert_eq!(canonical_string(""), "\"\"");
        assert_eq!(canonical_string("say \"hi\"\n"), "\"say \\\"hi\\\"\n\"");
        assert_eq!(canonical_string("国"), "\"国\"");
        assert_eq!(canonical_string("0041"), "0041", "hexadecimal code points stay bare");
    }

    #[test]
    fn version_2_escapes_control_characters() {
        assert_eq!(canonical_string_v2("a\nb\tc"), "\"a\\012b\tc\"", "tabs stay literal");
        assert_eq!(canonical_string_v2("plain"), "plain");
        let mut d = Dict::new();
        d.insert("code", Value::String("sub a by b;\n".into()));
        assert_eq!(write(&Document::new(Value::Dict(d)), Dialect::Glyphs2), "{\ncode = \"sub a by b;\\012\";\n}\n");
        assert_eq!(parse_value("\"sub a by b;\\012\"").unwrap().as_str(), Some("sub a by b;\n"));
    }

    #[test]
    fn values_built_in_code_are_written_canonically() {
        let mut d = Dict::new();
        d.insert("name", Value::String("Bold Italic".into()));
        d.insert("width", Value::Number(Number::float(512.25).unwrap()));
        d.insert(
            "pos",
            Value::Array(Array { items: vec![Value::Number(Number::int(10)), Value::Number(Number::int(-20))], layout: Layout::Compact }),
        );
        let out = write(&Document::new(Value::Dict(d)), Dialect::Glyphs3);
        assert_eq!(out, "{\nname = \"Bold Italic\";\nwidth = 512.25;\npos = (10,-20);\n}\n");
    }

    #[test]
    fn escapes_decode() {
        let v = parse_value(r#""a\"b\\c\U2019\Ud83d\Ude00\t""#).unwrap();
        assert_eq!(v.as_str(), Some("a\"b\\c\u{2019}\u{1f600}\t"));
        let lone = parse_value(r#""\Ud83dx""#).unwrap();
        assert_eq!(lone.as_str(), Some("\u{fffd}x"));
    }

    #[test]
    fn single_tokens_keep_their_spelling() {
        let s = parse_str_token("\"m01\"").unwrap();
        assert_eq!((s.as_str(), s.spelling()), ("m01", Some("\"m01\"")));
        assert_eq!(str_token(&s), "\"m01\"");
        let s = parse_str_token("Regular").unwrap();
        assert_eq!((s.as_str(), s.spelling()), ("Regular", None));
        assert!(parse_str_token("a b").is_err());
        assert!(parse_str_token("\"a").is_err());
    }

    #[test]
    fn data_round_trips() {
        let doc = parse("<0a ff>").unwrap();
        assert!(matches!(&doc.root, Value::Data(d) if d.bytes() == [0x0a, 0xff]));
        assert_eq!(write(&doc, Dialect::Glyphs3), "<0a ff>");
        assert_eq!(canonical_data(&[0x0a, 0xff]), "<0aff>");
    }

    #[test]
    fn bom_and_crlf_are_remembered() {
        let text = "\u{feff}{\r\na = 1;\r\n}\r\n";
        assert_eq!(roundtrip(text, Dialect::Glyphs3), text);
    }

    #[test]
    fn comments_are_skipped() {
        let v = parse_value("{ // note\n a = 1; /* more */ }").unwrap();
        assert_eq!(v.as_dict().unwrap().get("a").unwrap().as_num(), Some(Num::Int(1)));
    }

    #[test]
    fn malformed_input_is_an_error_with_a_position() {
        for bad in ["{", "{a = 1}", "(1 2)", "\"abc", "<0g>", "<abc>", "{a 1;}", "", "{}x", "/* x", "{a = ;}"] {
            let e = parse(bad).unwrap_err();
            assert!(e.line >= 1 && e.column >= 1, "{bad:?}: {e}");
        }
        let e = parse("{\na = 1;\nb = 2\n}").unwrap_err();
        assert_eq!((e.line, e.message.as_str()), (4, "expected `;` after a dictionary value"));
    }

    #[test]
    fn deep_nesting_is_refused() {
        let deep = "(".repeat(MAX_DEPTH + 1) + &")".repeat(MAX_DEPTH + 1);
        assert!(parse(&deep).unwrap_err().message.contains("nested"));
        let ok = "(".repeat(MAX_DEPTH - 1) + &")".repeat(MAX_DEPTH - 1);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn every_prefix_and_corruption_fails_cleanly() {
        let text = "{\n.formatVersion = 3;\nname = \"A \\\"b\\\"\";\npos = (1,-2.5);\ndata = <0aff>;\nlist = (\n{\nx = 1;\n},\n);\n}\n";
        for i in 0..text.len() {
            if let Some(prefix) = text.get(..i) {
                let _ = parse(prefix);
            }
        }
        let bytes = text.as_bytes();
        for i in 0..bytes.len() {
            for b in [b'{', b'}', b'(', b')', b'"', b'\\', b'<', b';', b'=', b',', 0u8, b'\n'] {
                let mut m = bytes.to_vec();
                if let Some(slot) = m.get_mut(i) {
                    *slot = b;
                }
                if let Ok(s) = std::str::from_utf8(&m) {
                    let _ = parse(s);
                }
            }
        }
    }
}

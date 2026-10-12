//! Line access with limits, and the small text codecs the format uses: quoted strings, UTF-7
//! and the length of ASCII85 data.

use crate::{MAX_LINE_BYTES, Result, SfdError};

/// The lines of one file, with one line of look-ahead.
pub(crate) struct Lines<'a> {
    file: &'a str,
    rest: &'a str,
    /// The number of the line last returned by [`Lines::next`].
    no: usize,
    peeked: Option<&'a str>,
}

impl<'a> Lines<'a> {
    pub(crate) fn new(file: &'a str, text: &'a str) -> Self {
        Self { file, rest: text, no: 0, peeked: None }
    }

    fn raw_next(&mut self) -> Result<Option<&'a str>> {
        if self.rest.is_empty() {
            return Ok(None);
        }
        let (line, rest) = self.rest.split_once('\n').unwrap_or((self.rest, ""));
        self.rest = rest;
        if line.len() > MAX_LINE_BYTES {
            return Err(SfdError::Syntax { at: self.at_line(self.no + 1), message: format!("the line is longer than {MAX_LINE_BYTES} bytes") });
        }
        Ok(Some(line.strip_suffix('\r').unwrap_or(line)))
    }

    #[allow(clippy::should_implement_trait)]
    pub(crate) fn next(&mut self) -> Result<Option<&'a str>> {
        let line = match self.peeked.take() {
            Some(l) => Some(l),
            None => self.raw_next()?,
        };
        if line.is_some() {
            self.no += 1;
        }
        Ok(line)
    }

    /// The next line without consuming it.
    pub(crate) fn peek(&mut self) -> Result<Option<&'a str>> {
        if self.peeked.is_none() {
            self.peeked = self.raw_next()?;
        }
        Ok(self.peeked)
    }

    /// The next line, or an error saying which block was not finished.
    pub(crate) fn need(&mut self, block: &str) -> Result<&'a str> {
        self.next()?.ok_or_else(|| self.err(format!("the file ends inside {block}")))
    }

    /// A keyword's value continued over the following lines while a quoted string in it is open
    /// (pickled data and comments can hold line breaks).
    pub(crate) fn value<'s>(&mut self, first: &'s str) -> Result<std::borrow::Cow<'s, str>> {
        if !quote_open(first) {
            return Ok(first.into());
        }
        let mut text = first.to_string();
        while quote_open(&text) {
            let Some(next) = self.next()? else { break };
            if text.len() + next.len() + 1 > MAX_LINE_BYTES {
                return Err(self.err(format!("a quoted string is longer than {MAX_LINE_BYTES} bytes")));
            }
            text.push('\n');
            text.push_str(next);
        }
        Ok(text.into())
    }

    pub(crate) fn at(&self) -> String {
        self.at_line(self.no)
    }

    fn at_line(&self, no: usize) -> String {
        if self.file.is_empty() { format!("line {no}") } else { format!("{}, line {no}", self.file) }
    }

    pub(crate) fn err(&self, message: impl Into<String>) -> SfdError {
        SfdError::Syntax { at: self.at(), message: message.into() }
    }
}

/// Splits `Keyword: value` (or a bare `Keyword`) into its keyword and value.
pub(crate) fn keyword(line: &str) -> (&str, &str) {
    let t = line.trim_start();
    let end = t.find(|c: char| c == ':' || c.is_whitespace()).unwrap_or(t.len());
    let (k, rest) = t.split_at(end);
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    (k, rest.trim())
}

fn quote_open(s: &str) -> bool {
    let mut open = false;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' && open {
            escaped = true;
        } else if c == '"' {
            open = !open;
        }
    }
    open
}

/// A token of a value: a quoted string (unescaped), a brace group (`{12-13 -1,-1}`, kept with
/// its braces) or a word.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok<'a> {
    Quoted(String),
    Group(&'a str),
    Word(&'a str),
}

pub(crate) fn tokens(s: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        rest = rest.trim_start();
        let Some(c) = rest.chars().next() else { break };
        if c == '"' {
            let body = rest.get(1..).unwrap_or("");
            let mut text = String::new();
            let mut escaped = false;
            let mut end = body.len();
            for (i, ch) in body.char_indices() {
                if escaped {
                    text.push(ch);
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    end = i + 1;
                    break;
                } else {
                    text.push(ch);
                }
            }
            out.push(Tok::Quoted(text));
            rest = body.get(end..).unwrap_or("");
        } else if c == '{' {
            let mut depth = 0usize;
            let mut in_quote = false;
            let mut end = rest.len();
            for (i, ch) in rest.char_indices() {
                match ch {
                    '"' => in_quote = !in_quote,
                    '{' if !in_quote => depth += 1,
                    '}' if !in_quote => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let (g, r) = rest.split_at(end);
            out.push(Tok::Group(g));
            rest = r;
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let (w, r) = rest.split_at(end);
            out.push(Tok::Word(w));
            rest = r;
        }
    }
    out
}

/// A finite number.
pub(crate) fn num(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub(crate) fn int(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

/// A value that may be quoted and is UTF-7 encoded.
pub(crate) fn text(value: &str) -> String {
    let v = value.trim();
    match tokens(v).into_iter().next() {
        Some(Tok::Quoted(q)) if v.starts_with('"') => utf7(&q),
        _ => utf7(v),
    }
}

/// A name: UTF-7 inside quotes, as written otherwise.
pub(crate) fn name(value: &str) -> String {
    let v = value.trim();
    match tokens(v).into_iter().next() {
        Some(Tok::Quoted(q)) if v.starts_with('"') => utf7(&q),
        _ => v.to_string(),
    }
}

/// Decodes UTF-7 (RFC 2152): `+` starts base64-encoded UTF-16, ended by `-` (dropped) or by any
/// character outside the base64 alphabet; `+-` is a plus sign. NUL characters are dropped.
pub(crate) fn utf7(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '+' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'-') {
            chars.next();
            out.push('+');
            continue;
        }
        let mut bits: u32 = 0;
        let mut nbits: u32 = 0;
        let mut units: Vec<u16> = Vec::new();
        while let Some(&d) = chars.peek() {
            let Some(v) = base64(d) else { break };
            chars.next();
            bits = (bits << 6) | v;
            nbits += 6;
            if nbits >= 16 {
                nbits -= 16;
                units.push(((bits >> nbits) & 0xFFFF) as u16);
                bits &= (1u32 << nbits) - 1;
            }
        }
        if chars.peek() == Some(&'-') {
            chars.next();
        }
        out.extend(char::decode_utf16(units).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)));
    }
    out.retain(|c| c != '\0');
    out
}

fn base64(c: char) -> Option<u32> {
    let v = match c {
        'A'..='Z' => c as u32 - 'A' as u32,
        'a'..='z' => c as u32 - 'a' as u32 + 26,
        '0'..='9' => c as u32 - '0' as u32 + 52,
        '+' => 62,
        '/' => 63,
        _ => return None,
    };
    Some(v)
}

/// Counts the bytes ASCII85 text decodes to, without decoding it.
#[derive(Default)]
pub(crate) struct A85Count {
    full: usize,
    partial: usize,
}

impl A85Count {
    /// Adds a line; `false` when it holds a character ASCII85 never uses (the data has ended).
    pub(crate) fn feed(&mut self, line: &str) -> bool {
        for c in line.chars() {
            match c {
                'z' if self.partial == 0 => self.full = self.full.saturating_add(4),
                '!'..='u' => {
                    self.partial += 1;
                    if self.partial == 5 {
                        self.partial = 0;
                        self.full = self.full.saturating_add(4);
                    }
                }
                '~' => {}
                c if c.is_whitespace() => {}
                _ => return false,
            }
        }
        true
    }

    pub(crate) fn bytes(&self) -> usize {
        self.full.saturating_add(self.partial.saturating_sub(1))
    }
}

/// `seconds` since 1970 as `YYYY-MM-DD HH:MM:SS +0000`, for years 1 to 9999.
pub(crate) fn unix_date(seconds: i64) -> Option<String> {
    let days = seconds.div_euclid(86_400);
    let secs = seconds.rem_euclid(86_400);
    // Days to a civil date (proleptic Gregorian), after H. Hinnant's public-domain algorithm.
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe.checked_add(era.checked_mul(400)?)? + i64::from(m <= 2);
    if !(1..=9999).contains(&y) {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000", secs / 3600, secs % 3600 / 60, secs % 60))
}

//! Statements of the feature file language fea-rs does not read, taken out of the code before
//! fea-rs parses it (M8.33; rows `fea.gsub8.null`, `fea.table.hhea`, `fea.table.vhea`,
//! `fea.table.os2.scriptAndStrikeout`, `fea.table.gdef.ligatureCaretByDev`,
//! `fea.legacy.except`, `fea.anonymous`).
//!
//! | Statement | What happens |
//! |---|---|
//! | `CaretSlopeRise`, `CaretSlopeRun` (hhea, vhea), `CaretOffset` (vhea), the subscript, superscript and strikeout fields of `OS/2` | written as spaces; the value is kept ([`Lifted::apply_hhea`] and the others add it to the result) |
//! | `LigatureCaretByDev g 300 <device 11 -1>;` | compiled as `LigatureCaretByPos` with a placeholder position, which [`Lifted::apply_carets`] replaces by a caret with the device table |
//! | `except a b, c d sub …;` | `ignore sub a' b, c' d; sub …;`, as AFDKO's feature compiler reads it: a pattern without a mark has its first glyph marked, and a rule without one (a `by` rule) has every glyph marked |
//! | `anonymous tag { … } tag;` | written as spaces; the text comes back in [`Compiled::anonymous`] and in a warning |
//! | `rsub … by NULL;`, `rsub …;` | refused, as fontTools' feaLib refuses it: a reverse substitution lookup has no way to delete a glyph |
//!
//! Line breaks stay where they are and all but the `except` and device caret rewrites keep the
//! text's length; [`Lifted::source`] takes a place in the rewritten text back to the file.
//!
//! Each file is read by itself (fea-rs reads an include statement outside every block or in a
//! feature block only, so no table block goes over into another file), once and without
//! recursion.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use write_fonts_052::tables::gdef::{CaretValue, Gdef};
use write_fonts_052::tables::layout::DeviceOrVariationIndex;

use crate::resolve::{Loaded, skip_comment, skip_string};
use crate::{AnonymousBlock, Compiled, Diagnostic, Limits, MetricsValues, Os2Values, Severity};

/// Most tokens of one statement this pass looks at; a longer statement is left to fea-rs.
const MAX_STATEMENT_TOKENS: usize = 65_536;
/// Most device tables in `LigatureCaretByDev` statements: each takes one of the placeholders.
const MAX_DEVICE_CARETS: usize = 1_024;
/// Most sizes (pixels per em) one device table spans.
const MAX_DEVICE_SIZES: usize = 4_096;
/// Most bytes of an anonymous block's text a warning quotes.
const MAX_QUOTED: usize = 2_000;

/// Text one of these does not appear in holds none of the statements.
const MARKERS: [&str; 8] = ["except", "anon", "rsub", "reversesub", "Caret", "scriptX", "scriptY", "Strikeout"];

/// The code without the statements, and what they said.
pub(crate) struct Lifted {
    pub(crate) files: Loaded,
    /// Per file whose text changed: where, in order.
    edits: HashMap<String, Vec<Placed>>,
    hhea: MetricsValues,
    vhea: MetricsValues,
    os2: Os2Values,
    carets: HashMap<i16, DeviceCaret>,
    anonymous: Vec<AnonymousBlock>,
    /// Warnings about the code, placed in the files as written.
    pub(crate) warnings: Vec<Diagnostic>,
    /// The anonymous blocks quoted, for a compile's warnings.
    pub(crate) anonymous_warnings: Vec<Diagnostic>,
}

/// A caret with a device table.
struct DeviceCaret {
    position: i16,
    device: Device,
}

/// The change in pixels at each size from `first` to `last`.
struct Device {
    first: u16,
    last: u16,
    deltas: Vec<i8>,
}

/// One rewrite: `out` of the rewritten text stands for `source` of the file.
struct Placed {
    out: Range<usize>,
    source: Range<usize>,
}

impl Lifted {
    /// The byte offset in `file` as written of `pos` in its rewritten text.
    pub(crate) fn source(&self, file: &str, pos: usize) -> usize {
        let Some(edits) = self.edits.get(file) else { return pos };
        let before = edits.partition_point(|e| e.out.start <= pos);
        let Some(edit) = before.checked_sub(1).and_then(|i| edits.get(i)) else { return pos };
        match pos.checked_sub(edit.out.end) {
            Some(after) => edit.source.end.saturating_add(after),
            None => edit.source.start.saturating_add(pos.saturating_sub(edit.out.start)).min(edit.source.end),
        }
    }

    /// A message about the rewritten text as one about the file.
    pub(crate) fn place(&self, d: Diagnostic) -> Diagnostic {
        let span = self.source(&d.file, d.span.start)..self.source(&d.file, d.span.end);
        Diagnostic { span: span.start..span.end.max(span.start), ..d }
    }

    pub(crate) fn apply_hhea(&self, values: &mut MetricsValues) {
        metrics(values, &self.hhea);
    }

    pub(crate) fn apply_vhea(&self, values: &mut MetricsValues) {
        metrics(values, &self.vhea);
    }

    pub(crate) fn apply_os2(&self, values: &mut Os2Values) {
        let mut from = self.os2;
        for (_, field) in OS2_FIELDS {
            if let Some(value) = *field(&mut from) {
                *field(values) = Some(value);
            }
        }
    }

    /// Puts the device tables into the ligature carets fea-rs built at the placeholders.
    pub(crate) fn apply_carets(&self, gdef: &mut Gdef) {
        if self.carets.is_empty() {
            return;
        }
        let Some(list) = gdef.lig_caret_list.as_mut() else { return };
        for glyph in &mut list.lig_glyphs {
            let mut changed = false;
            for caret in &mut glyph.caret_values {
                let CaretValue::Format1(plain) = &**caret else { continue };
                if let Some(DeviceCaret { position, device }) = self.carets.get(&plain.coordinate) {
                    let table = DeviceOrVariationIndex::device(device.first, device.last, &device.deltas);
                    caret.set(CaretValue::format_3(*position, table));
                    changed = true;
                }
            }
            if changed {
                glyph.caret_values.sort_by_key(|c| match &**c {
                    CaretValue::Format1(t) => i32::from(t.coordinate),
                    CaretValue::Format2(t) => i32::from(t.caret_value_point_index),
                    CaretValue::Format3(t) => i32::from(t.coordinate),
                });
            }
        }
    }

    pub(crate) fn finish(&self, compiled: &mut Compiled) {
        self.apply_hhea(&mut compiled.hhea);
        self.apply_vhea(&mut compiled.vhea);
        self.apply_os2(&mut compiled.os2);
        compiled.anonymous = self.anonymous.clone();
    }
}

fn metrics(to: &mut MetricsValues, from: &MetricsValues) {
    to.caret_offset = from.caret_offset.or(to.caret_offset);
    to.caret_slope_rise = from.caret_slope_rise.or(to.caret_slope_rise);
    to.caret_slope_run = from.caret_slope_run.or(to.caret_slope_run);
}

type Os2Field = fn(&mut Os2Values) -> &mut Option<i16>;

/// The `OS/2` keywords fea-rs has no grammar for, each with its [`Os2Values`] field.
const OS2_FIELDS: [(&str, Os2Field); 10] = [
    ("SubscriptXSize", |o| &mut o.subscript_x_size),
    ("SubscriptXOffset", |o| &mut o.subscript_x_offset),
    ("SubscriptYSize", |o| &mut o.subscript_y_size),
    ("SubscriptYOffset", |o| &mut o.subscript_y_offset),
    ("SuperscriptXSize", |o| &mut o.superscript_x_size),
    ("SuperscriptXOffset", |o| &mut o.superscript_x_offset),
    ("SuperscriptYSize", |o| &mut o.superscript_y_size),
    ("SuperscriptYOffset", |o| &mut o.superscript_y_offset),
    ("StrikeoutSize", |o| &mut o.strikeout_size),
    ("StrikeoutPosition", |o| &mut o.strikeout_position),
];

fn os2_field<'a>(values: &'a mut Os2Values, keyword: &str) -> Option<&'a mut Option<i16>> {
    OS2_FIELDS.iter().find(|(name, _)| *name == keyword).map(|(_, field)| field(values))
}

/// `files` without the statements of this module. Errors name the files as written.
pub(crate) fn lift(files: &Loaded, limits: &Limits) -> Result<Lifted, Vec<Diagnostic>> {
    let mut lifter = Lifter {
        files,
        texts: BTreeMap::new(),
        placeholders: None,
        max_messages: limits.diagnostics.max(1),
        errors: Vec::new(),
        out: Lifted {
            files: files.clone(),
            edits: HashMap::new(),
            hhea: MetricsValues::default(),
            vhea: MetricsValues::default(),
            os2: Os2Values::default(),
            carets: HashMap::new(),
            anonymous: Vec::new(),
            warnings: Vec::new(),
            anonymous_warnings: Vec::new(),
        },
    };
    if files.values().any(|text| MARKERS.iter().any(|m| text.contains(m))) {
        for (path, text) in files.iter() {
            lifter.file(path, text);
        }
    }
    if !lifter.errors.is_empty() {
        return Err(lifter.errors);
    }
    let Lifter { texts, mut out, .. } = lifter;
    if !texts.is_empty() {
        let mut rewritten: BTreeMap<String, Arc<str>> = (**files).clone();
        rewritten.extend(texts);
        if rewritten.values().fold(0usize, |n, t| n.saturating_add(t.len())) > limits.text_bytes {
            return Err(vec![Diagnostic::request(Severity::Error, format!("the feature code is larger than {} bytes", limits.text_bytes))]);
        }
        out.files = Arc::new(rewritten);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Tokens

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Str,
    Punct(u8),
}

#[derive(Clone, Copy)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

impl Tok {
    fn is(self, byte: u8) -> bool {
        self.kind == Kind::Punct(byte)
    }
}

fn word(text: &str, tok: Tok) -> &str {
    if tok.kind == Kind::Word { text.get(tok.start..tok.end).unwrap_or("") } else { "" }
}

fn is_punct(b: u8) -> bool {
    matches!(b, b'{' | b'}' | b'[' | b']' | b'(' | b')' | b';' | b'\'' | b',' | b'<' | b'>' | b'=')
}

/// The token at or after `pos`, comments skipped.
fn next_token(bytes: &[u8], mut pos: usize) -> Option<Tok> {
    loop {
        match *bytes.get(pos)? {
            b if b.is_ascii_whitespace() => pos += 1,
            b'#' => pos = skip_comment(bytes, pos),
            b'"' => return Some(Tok { kind: Kind::Str, start: pos, end: skip_string(bytes, pos).min(bytes.len()) }),
            b if is_punct(b) => return Some(Tok { kind: Kind::Punct(b), start: pos, end: pos + 1 }),
            _ => {
                let len = bytes
                    .get(pos..)
                    .and_then(|rest| rest.iter().position(|b| b.is_ascii_whitespace() || is_punct(*b) || matches!(b, b'#' | b'"')))
                    .unwrap_or(bytes.len().saturating_sub(pos));
                return Some(Tok { kind: Kind::Word, start: pos, end: pos.saturating_add(len) });
            }
        }
    }
}

/// An anonymous block in feature code: byte ranges in the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnonymousSpan {
    pub tag: Range<usize>,
    /// From the line after the opening brace up to the line of the closing one.
    pub content: Range<usize>,
    /// Behind the closing semicolon.
    pub end: usize,
}

/// The anonymous block whose `anon` or `anonymous` keyword starts at byte `at` of `code`; `None`
/// when no such block starts there or its closing brace, tag and semicolon are not on one line.
/// What is between the braces is not read as code.
pub fn anonymous_block(code: &str, at: usize) -> Option<AnonymousSpan> {
    let bytes = code.as_bytes();
    let keyword = next_token(bytes, at)?;
    if keyword.start != at || !matches!(word(code, keyword), "anon" | "anonymous") {
        return None;
    }
    let tag = next_token(bytes, keyword.end).filter(|t| t.kind == Kind::Word)?;
    let open = next_token(bytes, tag.end).filter(|t| t.is(b'{'))?;
    let name = word(code, tag).as_bytes();
    let same_line = |mut i: usize| {
        while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace() && !matches!(b, b'\n' | b'\r')) {
            i += 1;
        }
        i
    };
    let mut from = open.end;
    let (close, end) = loop {
        let close = from.checked_add(bytes.get(from..)?.iter().position(|b| *b == b'}')?)?;
        from = close + 1;
        let label = same_line(from);
        if bytes.get(label..).is_some_and(|rest| rest.starts_with(name)) {
            let semi = same_line(label.saturating_add(name.len()));
            if bytes.get(semi) == Some(&b';') {
                break (close, semi + 1);
            }
        }
    };
    let inside = bytes.get(open.end..close)?;
    let start = match inside.iter().position(|b| matches!(b, b'\n' | b'\r')) {
        Some(n) => {
            let at = open.end.saturating_add(n);
            let crlf = bytes.get(at) == Some(&b'\r') && bytes.get(at + 1) == Some(&b'\n');
            at + if crlf { 2 } else { 1 }
        }
        None => open.end,
    };
    let rest = bytes.get(start..close).unwrap_or(&[]);
    let last = rest.iter().rposition(|b| matches!(b, b'\n' | b'\r')).map_or(0, |n| n + 1);
    let blank = rest.get(last..).is_some_and(|line| line.iter().all(u8::is_ascii_whitespace));
    let stop = if blank { start.saturating_add(last) } else { close };
    Some(AnonymousSpan { tag: tag.start..tag.end, content: start..stop.max(start), end })
}

/// `text` as spaces, with its line breaks.
fn blank(text: &str) -> String {
    text.bytes().map(|b| if matches!(b, b'\n' | b'\r') { char::from(b) } else { ' ' }).collect()
}

// ---------------------------------------------------------------------------------------------
// The pass

#[derive(Clone, Copy, PartialEq, Eq)]
enum Table {
    Hhea,
    Vhea,
    Os2,
    Gdef,
    Other,
}

/// A change to one file's text.
struct Edit {
    range: Range<usize>,
    with: String,
}

struct Lifter<'a> {
    files: &'a Loaded,
    texts: BTreeMap<String, Arc<str>>,
    /// Numbers low enough to be placeholders that the code itself uses, once looked for, and
    /// the next one to try.
    placeholders: Option<(HashSet<i16>, i16)>,
    max_messages: usize,
    errors: Vec<Diagnostic>,
    out: Lifted,
}

/// The file being read.
struct File<'a> {
    path: &'a str,
    text: &'a str,
    edits: Vec<Edit>,
}

impl Lifter<'_> {
    fn error(&mut self, path: &str, span: Range<usize>, message: impl Into<String>) {
        if self.errors.len() < self.max_messages {
            self.errors.push(Diagnostic::at(path, span, Severity::Error, message));
        }
    }

    fn warn(&mut self, path: &str, span: Range<usize>, message: impl Into<String>) {
        if self.out.warnings.len() < self.max_messages {
            self.out.warnings.push(Diagnostic::at(path, span, Severity::Warning, message));
        }
    }

    fn file(&mut self, path: &str, text: &str) {
        let mut file = File { path, text, edits: Vec::new() };
        self.scan(&mut file);
        let File { edits, .. } = file;
        if edits.is_empty() {
            return;
        }
        let mut rewritten = String::with_capacity(text.len());
        let mut placed = Vec::with_capacity(edits.len());
        let mut copied = 0usize;
        for edit in edits {
            rewritten.push_str(text.get(copied..edit.range.start).unwrap_or(""));
            let start = rewritten.len();
            rewritten.push_str(&edit.with);
            copied = edit.range.end.max(copied);
            placed.push(Placed { out: start..rewritten.len(), source: edit.range });
        }
        rewritten.push_str(text.get(copied..).unwrap_or(""));
        self.texts.insert(path.to_string(), Arc::from(rewritten));
        self.out.edits.insert(path.to_string(), placed);
    }

    fn scan(&mut self, file: &mut File<'_>) {
        let text = file.text;
        let bytes = text.as_bytes();
        let mut blocks: Vec<Option<Table>> = Vec::new();
        let mut statement: Vec<Tok> = Vec::new();
        let mut whole = true;
        // After a block's closing brace: its label, up to the semicolon.
        let mut label = false;
        let mut unterminated = false;
        let mut pos = 0usize;
        while let Some(tok) = next_token(bytes, pos) {
            pos = tok.end.max(pos + 1);
            let table = blocks.last().copied().flatten();
            match tok.kind {
                Kind::Punct(b'{') => {
                    let first = statement.first().copied();
                    let top = blocks.is_empty() && !label && !unterminated;
                    if let Some(first) = first.filter(|t| top && matches!(word(text, *t), "anon" | "anonymous")) {
                        if let Some(span) = anonymous_block(text, first.start) {
                            pos = span.end.max(pos);
                            self.anonymous(file, first.start, span);
                            statement.clear();
                            whole = true;
                            continue;
                        }
                        unterminated = true;
                    }
                    let named = match statement.as_slice() {
                        [keyword, tag] if !label && word(text, *keyword) == "table" => Some(match word(text, *tag) {
                            "hhea" => Table::Hhea,
                            "vhea" => Table::Vhea,
                            "OS/2" => Table::Os2,
                            "GDEF" => Table::Gdef,
                            _ => Table::Other,
                        }),
                        _ => table,
                    };
                    // The nesting check of `resolve` bounds the depth.
                    blocks.push(named);
                    statement.clear();
                    whole = true;
                    label = false;
                }
                Kind::Punct(b'}') => {
                    blocks.pop();
                    statement.clear();
                    whole = true;
                    label = true;
                }
                Kind::Punct(b';') => {
                    if whole && !label {
                        self.statement(file, &statement, tok.end, table);
                    }
                    statement.clear();
                    whole = true;
                    label = false;
                }
                Kind::Word if word(text, tok) == "include" && next_token(bytes, tok.end).is_some_and(|t| t.is(b'(')) => {
                    // The path may hold any character.
                    let Some(len) = bytes.get(tok.end..).and_then(|rest| rest.iter().position(|b| *b == b')')) else { break };
                    pos = tok.end.saturating_add(len).saturating_add(1);
                }
                _ if statement.len() < MAX_STATEMENT_TOKENS => statement.push(tok),
                _ => whole = false,
            }
        }
    }

    fn anonymous(&mut self, file: &mut File<'_>, start: usize, span: AnonymousSpan) {
        let text = file.text;
        let tag = text.get(span.tag.clone()).unwrap_or("").to_string();
        let content = text.get(span.content.clone()).unwrap_or("");
        file.edits.push(Edit { range: start..span.end, with: blank(text.get(start..span.end).unwrap_or("")) });
        if self.out.anonymous_warnings.len() < self.max_messages {
            let quoted: String = content.char_indices().take_while(|(i, _)| *i < MAX_QUOTED).map(|(_, c)| c).collect();
            let cut = if quoted.len() < content.len() { " (cut)" } else { "" };
            let message = format!("the anonymous block `{tag}` is not compiled; its text is{cut}:\n{quoted}");
            self.out.anonymous_warnings.push(Diagnostic::at(file.path, start..span.end, Severity::Warning, message));
        }
        self.out.anonymous.push(AnonymousBlock { tag, text: content.to_string() });
    }

    /// One statement: its tokens without the semicolon, which ends at `end`.
    fn statement(&mut self, file: &mut File<'_>, tokens: &[Tok], end: usize, table: Option<Table>) {
        let Some((first, rest)) = tokens.split_first() else { return };
        let keyword = word(file.text, *first);
        let span = first.start..end;
        match (keyword, table) {
            ("CaretSlopeRise" | "CaretSlopeRun", Some(Table::Hhea | Table::Vhea)) | ("CaretOffset", Some(Table::Vhea)) => {
                let Some(value) = self.metric(file, rest, keyword, span.clone()) else { return };
                let values = if table == Some(Table::Hhea) { &mut self.out.hhea } else { &mut self.out.vhea };
                match keyword {
                    "CaretSlopeRise" => values.caret_slope_rise = Some(value),
                    "CaretSlopeRun" => values.caret_slope_run = Some(value),
                    _ => values.caret_offset = Some(value),
                }
                self.remove(file, span);
            }
            (_, Some(Table::Os2)) if OS2_FIELDS.iter().any(|(name, _)| *name == keyword) => {
                let Some(value) = self.metric(file, rest, keyword, span.clone()) else { return };
                if let Some(field) = os2_field(&mut self.out.os2, keyword) {
                    *field = Some(value);
                }
                self.remove(file, span);
            }
            ("LigatureCaretByDev", Some(Table::Gdef)) => self.device_carets(file, *first, rest, span),
            ("rsub" | "reversesub", None) => {
                let by = top_level(rest).position(|t| word(file.text, *t) == "by");
                let deletes = match by {
                    Some(at) => top_level(rest).nth(at + 1).is_some_and(|t| word(file.text, *t) == "NULL"),
                    None => !top_level(rest).any(|t| word(file.text, *t) == "from"),
                };
                if deletes {
                    self.error(
                        file.path,
                        span,
                        "a reverse chaining substitution cannot delete a glyph: the lookup type has no way to write it; give the replacement after `by`",
                    );
                }
            }
            ("except", None) => self.except(file, *first, rest, span),
            _ => {}
        }
    }

    /// The one number of a table field.
    fn metric(&mut self, file: &File<'_>, rest: &[Tok], keyword: &str, span: Range<usize>) -> Option<i16> {
        let value = match rest {
            [value] => word(file.text, *value).parse::<i64>().ok(),
            _ => None,
        };
        match value.map(i16::try_from) {
            Some(Ok(v)) => Some(v),
            Some(Err(_)) => {
                self.error(file.path, span, format!("the {keyword} value is outside -32768 to 32767"));
                None
            }
            None => {
                self.error(file.path, span, format!("{keyword} takes one whole number"));
                None
            }
        }
    }

    fn remove(&mut self, file: &mut File<'_>, span: Range<usize>) {
        let with = blank(file.text.get(span.clone()).unwrap_or(""));
        file.edits.push(Edit { range: span, with });
    }

    /// `LigatureCaretByDev <glyph or class> (<position> <device>)+;`
    fn device_carets(&mut self, file: &mut File<'_>, keyword: Tok, rest: &[Tok], span: Range<usize>) {
        const FORM: &str = "LigatureCaretByDev takes a glyph or a glyph class, then a position and a device table for each caret, \
             as in `LigatureCaretByDev f_i 300 <device 11 -1>;`";
        let Some(mut left) = items(rest).and_then(|found| found.first().map(|item| item.tokens)).and_then(|n| rest.get(n..)) else {
            return self.error(file.path, span, FORM);
        };
        let mut edits = vec![Edit { range: keyword.start..keyword.end, with: "LigatureCaretByPos".into() }];
        let mut found = false;
        while let Some((position, after)) = left.split_first() {
            let Some(value) = word(file.text, *position).parse::<i16>().ok() else { return self.error(file.path, span, FORM) };
            let close = after.iter().position(|t| t.is(b'>'));
            let device = match (after.split_first(), close) {
                (Some((open, _)), Some(close)) if open.is(b'<') => after.get(1..close),
                _ => None,
            };
            let (Some(device), Some(close)) = (device, close) else { return self.error(file.path, span, FORM) };
            let end = after.get(close).map_or(position.end, |t| t.end);
            let tail = blank(file.text.get(position.end..end).unwrap_or(""));
            let number = match device_table(file.text, device) {
                Ok(None) => value,
                Ok(Some(device)) => {
                    let Some(placeholder) = self.placeholder() else {
                        return self.error(file.path, span, format!("the code has more than {MAX_DEVICE_CARETS} ligature carets with device tables"));
                    };
                    self.out.carets.insert(placeholder, DeviceCaret { position: value, device });
                    placeholder
                }
                Err(why) => return self.error(file.path, position.start..end, why),
            };
            edits.push(Edit { range: position.start..end, with: format!("{number}{tail}") });
            found = true;
            left = after.get(close + 1..).unwrap_or(&[]);
        }
        if !found {
            return self.error(file.path, span, FORM);
        }
        file.edits.append(&mut edits);
    }

    /// A position the code does not use, to stand for a caret with a device table.
    fn placeholder(&mut self) -> Option<i16> {
        if self.out.carets.len() >= MAX_DEVICE_CARETS {
            return None;
        }
        let files = self.files;
        let (used, next) = self.placeholders.get_or_insert_with(|| (files.values().flat_map(|t| negative_numbers(t)).collect(), i16::MIN));
        while used.contains(&*next) {
            *next = next.checked_add(1)?;
        }
        let found = *next;
        *next = next.checked_add(1)?;
        Some(found)
    }

    /// `except <pattern> (, <pattern>)* sub …;`
    fn except(&mut self, file: &mut File<'_>, keyword: Tok, rest: &[Tok], span: Range<usize>) {
        let text = file.text;
        let Some(at) = rest.iter().position(|t| matches!(word(text, *t), "sub" | "substitute" | "rsub" | "reversesub")) else { return };
        let (Some(patterns), Some(rule)) = (rest.get(..at), rest.get(at..)) else { return };
        let Some((verb, rule)) = rule.split_first() else { return };
        let mut edits = vec![Edit { range: keyword.start..keyword.end, with: "ignore sub".into() }];
        for pattern in patterns.split(|t| t.is(b',')) {
            let Some(found) = items(pattern) else { return };
            let Some(first) = found.first() else { return };
            if !found.iter().any(|i| i.marked) {
                edits.push(Edit { range: first.end..first.end, with: "'".into() });
            }
        }
        let Some(last) = patterns.last() else { return };
        edits.push(Edit { range: last.end..last.end, with: ";".into() });
        let by = rule.iter().position(|t| matches!(word(text, *t), "by" | "from"));
        if let (true, Some(by)) = (matches!(word(text, *verb), "sub" | "substitute"), by)
            && rule.get(by).is_some_and(|t| word(text, *t) == "by")
            && let Some(found) = rule.get(..by).and_then(items)
            && !found.iter().any(|i| i.marked)
        {
            edits.extend(found.iter().map(|i| Edit { range: i.end..i.end, with: "'".into() }));
        }
        file.edits.append(&mut edits);
        self.warn(
            file.path,
            span,
            "`except` is the form feature files had before version 1.2: write its sequences as `ignore sub …;` before the rule, and mark the rule's glyphs with `'`",
        );
    }
}

/// The tokens outside brackets.
fn top_level(tokens: &[Tok]) -> impl Iterator<Item = &Tok> {
    let mut depth = 0usize;
    tokens.iter().filter(move |t| match t.kind {
        Kind::Punct(b'[' | b'<' | b'(') => {
            depth = depth.saturating_add(1);
            false
        }
        Kind::Punct(b']' | b'>' | b')') => {
            depth = depth.saturating_sub(1);
            false
        }
        _ => depth == 0,
    })
}

/// A glyph or glyph class of a sequence.
struct Item {
    /// Where it ends, its mark not counted.
    end: usize,
    marked: bool,
    /// The tokens it takes, its mark counted.
    tokens: usize,
}

/// The glyphs and glyph classes of a sequence, as far as it is one; `None` when it starts with
/// something else or has a bracket that is not closed.
fn items(tokens: &[Tok]) -> Option<Vec<Item>> {
    let mut out: Vec<Item> = Vec::new();
    let mut i = 0usize;
    while let Some(tok) = tokens.get(i) {
        let from = i;
        let end = match tok.kind {
            Kind::Word => tok.end,
            Kind::Punct(b'[') => {
                let close = tokens.get(i..)?.iter().position(|t| t.is(b']'))?;
                i = i.saturating_add(close);
                tokens.get(i)?.end
            }
            _ if out.is_empty() => return None,
            _ => break,
        };
        i += 1;
        let marked = tokens.get(i).is_some_and(|t| t.is(b'\''));
        i += usize::from(marked);
        out.push(Item { end, marked, tokens: i - from });
    }
    Some(out)
}

/// The sizes and deltas of `device 11 -1, 12 -2` (the angle brackets taken off); `None` for a
/// NULL device. Sizes between the given ones change nothing, as feaLib fills them.
fn device_table(text: &str, tokens: &[Tok]) -> Result<Option<Device>, String> {
    let mut words = tokens.iter().map(|t| if t.is(b',') { "," } else { word(text, *t) });
    if words.next() != Some("device") {
        return Err("a device table is written `<device 11 -1, 12 -2>` or `<device NULL>`".into());
    }
    let rest: Vec<&str> = words.collect();
    if rest == ["NULL"] {
        return Ok(None);
    }
    let mut sizes: BTreeMap<u16, i8> = BTreeMap::new();
    for pair in rest.split(|w| *w == ",") {
        let parsed = match pair {
            [size, delta] => size.parse::<u16>().ok().zip(delta.parse::<i8>().ok()),
            _ => None,
        };
        let Some((size, delta)) = parsed else {
            return Err("a device table is a list of sizes and changes in pixels (-128 to 127), as in `<device 11 -1, 12 -2>`".into());
        };
        if sizes.insert(size, delta).is_some() {
            return Err(format!("the device table gives the size {size} twice"));
        }
    }
    let (Some(first), Some(last)) = (sizes.keys().next().copied(), sizes.keys().next_back().copied()) else { return Ok(None) };
    if usize::from(last.saturating_sub(first)) >= MAX_DEVICE_SIZES {
        return Err(format!("the device table spans more than {MAX_DEVICE_SIZES} sizes"));
    }
    let deltas = (first..=last).map(|size| sizes.get(&size).copied().unwrap_or(0)).collect();
    Ok(Some(Device { first, last, deltas }))
}

/// The negative 16-bit numbers in `text`.
fn negative_numbers(text: &str) -> impl Iterator<Item = i16> {
    text.split('-').skip(1).filter_map(|rest| {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        rest.get(..digits).and_then(|d| format!("-{d}").parse::<i16>().ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lifted(code: &str) -> Lifted {
        let files: Loaded = Arc::new(BTreeMap::from([("f.fea".to_string(), Arc::from(code))]));
        lift(&files, &Limits::default()).unwrap_or_else(|d| panic!("{d:?}"))
    }

    #[test]
    fn places_in_rewritten_code_are_places_in_the_file() {
        let code = "feature calt {\n  except x a, y a sub [a b] by c;\n  sub d by e;\n} calt;\ntable vhea { CaretOffset\n 5; } vhea;\n";
        let out = lifted(code);
        let text = out.files.get("f.fea").unwrap();
        assert!(text.contains("ignore sub x' a, y' a; sub [a b]' by c;"), "{text}");
        assert_eq!(text.lines().count(), code.lines().count());
        for needle in ["y", "sub [a", "c;", "sub d", "table vhea", "vhea;"] {
            assert_eq!(out.source("f.fea", text.find(needle).unwrap()), code.find(needle).unwrap(), "{needle}");
        }
        assert_eq!(out.source("f.fea", text.len()), code.len());
    }

    #[test]
    fn code_without_these_statements_stays_as_it_is() {
        let code = "@except = [a b]; lookup anon { sub a by b; } anon; feature ss01 { lookup anon; rsub a' b by c; } ss01; # except x sub y\n\
                    table hhea { CaretOffset 5; } hhea;";
        assert_eq!(&**lifted(code).files.get("f.fea").unwrap(), code);
    }

    #[test]
    fn an_anonymous_block_is_taken_to_its_closing_line() {
        let code = "anon sbit { # open\n  } sbit }\n\"{ ;\n  } sbit ; x";
        let span = anonymous_block(code, 0).unwrap();
        assert_eq!(code.get(span.content).unwrap(), "  } sbit }\n\"{ ;\n");
        assert_eq!(code.get(span.end..).unwrap(), " x");
        assert!(anonymous_block("anon sbit { } sbit\n;", 0).is_none());
        assert_eq!(anonymous_block("anon a { b c } a;", 0).unwrap().content, 8..13);
    }

    #[test]
    fn hostile_code_is_read_without_a_panic() {
        for code in [
            "except",
            "except ;",
            "except sub;",
            "except [ sub a by b;",
            "anon",
            "anon {",
            "anon a{",
            "rsub;",
            "table GDEF { LigatureCaretByDev",
            "}}}; rsub a by NULL;",
            "include(",
            "\"x rsub",
        ] {
            let files: Loaded = Arc::new(BTreeMap::from([("f.fea".to_string(), Arc::from(code))]));
            let _ = lift(&files, &Limits::default());
        }
        let many = "anon a { } ".repeat(50_000);
        let files: Loaded = Arc::new(BTreeMap::from([("f.fea".to_string(), Arc::from(many.as_str()))]));
        assert!(lift(&files, &Limits::default()).is_ok());
    }
}

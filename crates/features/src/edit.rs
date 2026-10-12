//! What the code editor of the Features tab asks about the code (M8.15): syntax colouring,
//! completion by position, the names the code defines, and the fixes offered for an error.
//!
//! Rows: `fontInfo.features.syntaxColouring` (the token classes), `.autocomplete` (the data),
//! `font.features.complete`, `font.features.applyFix`.
//!
//! The code is untrusted and at most [`crate::MAX_ENTRY_BYTES`]; these functions run on the UI
//! thread, so each works in one pass over the code (the fix search looks back at most
//! [`MAX_FIX_SCAN`] bytes) and returns at most a bounded number of items. Ranges are byte
//! ranges of the code, always on character boundaries.
//!
//! The fixes (`font.features.applyFix`, our own choice of three):
//! - a glyph name the font has only in another case (`glyph not in font`): the font's spelling;
//! - a class name the code defines only in another case (`undefined glyph class`): that
//!   spelling;
//! - a missing semicolon (`Expected ';'`): one inserted after the statement, before any
//!   comment or space.
//!
//! The third fix of the row, a class used before its entry, moves an entry rather than editing
//! code, so the engine offers it.

use std::collections::HashSet;
use std::ops::Range;

use fontcraft_fea::MAX_NESTING_DEPTH;

use crate::MAX_ENTRY_BYTES;
use crate::source_map::EntryDiagnostic;

/// Most completions one call returns.
pub const MAX_COMPLETIONS: usize = 1_000;
/// Most fixes offered for one diagnostic.
pub const MAX_FIXES: usize = 16;
/// Most coloured stretches [`highlight`] returns; the code after the last one is plain.
pub const MAX_HIGHLIGHTS: usize = 1_000_000;
/// Most class or lookup names [`definitions`] returns of each kind.
pub const MAX_DEFINITIONS: usize = 100_000;
/// Most bytes the missing-semicolon fix looks back over for the end of the statement.
pub const MAX_FIX_SCAN: usize = 64 << 10;

/// What a stretch of code is, for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    Keyword,
    /// A feature, script or language tag.
    Tag,
    GlyphName,
    /// `@Name`.
    ClassName,
    LookupName,
    Number,
    String,
    Comment,
    /// `$name`, `${…}`, `$[…]`.
    Token,
    Punctuation,
}

/// One coloured stretch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub kind: HighlightKind,
}

/// A byte of a glyph name, a tag, a keyword or a number. Bytes of non-ASCII characters count,
/// so a run never ends inside a character.
fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'*' | b'+' | b'^' | b'~') || b >= 0x80
}

/// A byte that starts a name: a digit, a letter, `_`, `.` or a non-ASCII character.
fn starts_name(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.') || b >= 0x80
}

fn is_punctuation(b: u8) -> bool {
    matches!(b, b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'<' | b'>' | b';' | b',' | b'\'' | b'=' | b'|' | b'-')
}

/// The keywords of the feature file specification, its table blocks and the reference's
/// extensions.
fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "anchor"
            | "anchorDef"
            | "anon"
            | "anonymous"
            | "by"
            | "contour"
            | "cursive"
            | "device"
            | "enum"
            | "enumerate"
            | "exclude_dflt"
            | "excludeDFLT"
            | "feature"
            | "from"
            | "ignore"
            | "IgnoreBaseGlyphs"
            | "IgnoreLigatures"
            | "IgnoreMarks"
            | "include"
            | "include_dflt"
            | "includeDFLT"
            | "language"
            | "languagesystem"
            | "lookup"
            | "lookupflag"
            | "mark"
            | "MarkAttachmentType"
            | "markClass"
            | "nameid"
            | "NULL"
            | "parameters"
            | "pos"
            | "position"
            | "required"
            | "reversesub"
            | "RightToLeft"
            | "rsub"
            | "script"
            | "sub"
            | "substitute"
            | "subtable"
            | "table"
            | "useExtension"
            | "UseMarkFilteringSet"
            | "valueRecordDef"
            | "base"
            | "ligature"
            | "ligComponent"
            | "featureNames"
            | "name"
            | "cvParameters"
            | "FeatUILabelNameID"
            | "FeatUITooltipTextNameID"
            | "SampleTextNameID"
            | "ParamUILabelNameID"
            | "Character"
            | "sizemenuname"
            | "conditionset"
            | "variation"
            | "GlyphClassDef"
            | "Attach"
            | "LigatureCaretByPos"
            | "LigatureCaretByIndex"
            | "HorizAxis.BaseTagList"
            | "HorizAxis.BaseScriptList"
            | "HorizAxis.MinMax"
            | "VertAxis.BaseTagList"
            | "VertAxis.BaseScriptList"
            | "VertAxis.MinMax"
            | "FontRevision"
            | "CaretOffset"
            | "Ascender"
            | "Descender"
            | "LineGap"
            | "VertTypoAscender"
            | "VertTypoDescender"
            | "VertTypoLineGap"
            | "VertOriginY"
            | "VertAdvanceY"
            | "TypoAscender"
            | "TypoDescender"
            | "TypoLineGap"
            | "winAscent"
            | "winDescent"
            | "XHeight"
            | "CapHeight"
            | "WeightClass"
            | "WidthClass"
            | "Vendor"
            | "Panose"
            | "UnicodeRange"
            | "CodePageRange"
            | "FSType"
            | "LowerOpSize"
            | "UpperOpSize"
            | "ElidedFallbackName"
            | "ElidedFallbackNameID"
            | "DesignAxis"
            | "AxisValue"
            | "flag"
            | "location"
            | "OlderSiblingFontAttribute"
            | "ElidableAxisValueName"
            | "delete"
            | "del"
            | "standalone"
    )
}

/// The keywords that start a statement, offered at the start of one.
const STATEMENT_KEYWORDS: &[&str] = &[
    "sub",
    "pos",
    "ignore",
    "lookup",
    "lookupflag",
    "script",
    "language",
    "languagesystem",
    "feature",
    "markClass",
    "enum",
    "rsub",
    "substitute",
    "position",
    "reversesub",
    "enumerate",
    "subtable",
    "table",
    "include",
    "parameters",
    "featureNames",
    "cvParameters",
    "sizemenuname",
    "nameid",
    "name",
    "valueRecordDef",
    "anchorDef",
    "conditionset",
    "variation",
    "delete",
    "standalone",
];

/// The flags a `lookupflag` statement takes.
const LOOKUPFLAG_KEYWORDS: &[&str] =
    &["RightToLeft", "IgnoreBaseGlyphs", "IgnoreLigatures", "IgnoreMarks", "MarkAttachmentType", "UseMarkFilteringSet"];

/// Statements that name no glyph.
fn takes_no_glyphs(first: &str) -> bool {
    matches!(
        first,
        "languagesystem"
            | "script"
            | "language"
            | "include"
            | "table"
            | "nameid"
            | "name"
            | "featureNames"
            | "cvParameters"
            | "parameters"
            | "sizemenuname"
            | "feature"
            | "subtable"
    )
}

/// The block a `{` opens, for the name after its `}`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    /// `feature tag { … } tag;`, `table tag { … } tag;`.
    Tagged,
    /// `lookup name { … } name;`.
    Lookup,
    Other,
}

/// What the next name in the code is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Plain,
    /// This many tags follow.
    Tags(u8),
    Lookup,
}

/// The end of the run of name bytes starting at `from`.
fn name_end(bytes: &[u8], from: usize) -> usize {
    let mut j = from;
    while bytes.get(j).copied().is_some_and(is_name_byte) {
        j += 1;
    }
    j
}

/// The index of the first `needle` at or after `from`, if any.
fn find_byte(bytes: &[u8], from: usize, needle: u8) -> Option<usize> {
    bytes.get(from..)?.iter().position(|&b| b == needle).map(|p| from + p)
}

/// A number literal: decimal with an optional fraction, or hexadecimal.
fn is_number(word: &str) -> bool {
    if let Some(hex) = word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")) {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let digits = word.strip_prefix('-').unwrap_or(word);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "0"));
    !whole.is_empty() && whole.bytes().all(|b| b.is_ascii_digit()) && !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit())
}

/// The end of the token that starts with the `$` at `at`.
fn token_end(bytes: &[u8], at: usize) -> usize {
    match bytes.get(at + 1) {
        Some(b'{') => {
            // Closes at the first `}`; never crosses a line, a `{`, a `;` or another `$`.
            let mut j = at + 2;
            while let Some(&b) = bytes.get(j) {
                match b {
                    b'}' => return j + 1,
                    b'\n' | b'{' | b';' | b'$' => return j,
                    _ => j += 1,
                }
            }
            j
        }
        Some(b'[') => {
            // Closes at the first `]` outside quotes; never crosses a `;`.
            let mut j = at + 2;
            let mut quoted = false;
            while let Some(&b) = bytes.get(j) {
                match b {
                    b'"' => quoted = !quoted,
                    b']' if !quoted => return j + 1,
                    b';' if !quoted => return j,
                    _ => {}
                }
                j += 1;
            }
            j
        }
        _ => {
            let mut j = at + 1;
            while bytes.get(j).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
                j += 1;
            }
            j
        }
    }
}

fn push(out: &mut Vec<Highlight>, range: Range<usize>, kind: HighlightKind) {
    if !range.is_empty() {
        out.push(Highlight { range, kind });
    }
}

/// The stretches of `code` to colour, in order, not overlapping. Text not covered is plain.
/// Only the first [`MAX_ENTRY_BYTES`] bytes and at most [`MAX_HIGHLIGHTS`] stretches.
pub fn highlight(code: &str) -> Vec<Highlight> {
    let bytes = code.as_bytes();
    let end = bytes.len().min(MAX_ENTRY_BYTES);
    let mut out: Vec<Highlight> = Vec::new();
    let mut next = Next::Plain;
    let mut opening: Option<Block> = None;
    let mut blocks: Vec<Block> = Vec::new();
    // Blocks past the depth cap, counted so their closing braces pop nothing.
    let mut overflow = 0usize;
    let mut i = 0usize;
    while i < end && out.len() < MAX_HIGHLIGHTS {
        let Some(&b) = bytes.get(i) else { break };
        match b {
            b'#' => {
                let j = find_byte(bytes, i, b'\n').unwrap_or(bytes.len());
                let text = code.get(i..j).unwrap_or("");
                let directive = ["#ifdef", "#ifndef", "#endif", "#else"].iter().any(|d| text.starts_with(*d));
                push(&mut out, i..j, if directive { HighlightKind::Keyword } else { HighlightKind::Comment });
                i = j;
            }
            b'"' => {
                let j = find_byte(bytes, i + 1, b'"').map_or(bytes.len(), |q| q + 1);
                push(&mut out, i..j, HighlightKind::String);
                i = j;
            }
            b'$' => {
                let j = token_end(bytes, i).max(i + 1);
                push(&mut out, i..j, HighlightKind::Token);
                next = Next::Plain;
                i = j;
            }
            b'@' => {
                let j = name_end(bytes, i + 1);
                push(&mut out, i..j, HighlightKind::ClassName);
                i = j;
            }
            b'\\' => {
                let j = name_end(bytes, i + 1);
                push(&mut out, i..j, HighlightKind::GlyphName);
                i = j;
            }
            b'-' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let j = name_end(bytes, i + 1);
                let word = code.get(i..j).unwrap_or("");
                push(&mut out, i..j, if is_number(word) { HighlightKind::Number } else { HighlightKind::GlyphName });
                i = j;
            }
            b if starts_name(b) => {
                let j = name_end(bytes, i);
                let word = code.get(i..j).unwrap_or("");
                let kind = match next {
                    Next::Tags(n) => {
                        next = if n > 1 { Next::Tags(n - 1) } else { Next::Plain };
                        HighlightKind::Tag
                    }
                    Next::Lookup => {
                        next = Next::Plain;
                        HighlightKind::LookupName
                    }
                    Next::Plain if b.is_ascii_digit() => {
                        if is_number(word) {
                            HighlightKind::Number
                        } else {
                            HighlightKind::GlyphName
                        }
                    }
                    Next::Plain if is_keyword(word) => {
                        match word {
                            "feature" | "table" | "variation" => {
                                next = Next::Tags(1);
                                opening = Some(Block::Tagged);
                            }
                            "lookup" => {
                                next = Next::Lookup;
                                opening = Some(Block::Lookup);
                            }
                            "script" | "language" => next = Next::Tags(1),
                            "languagesystem" => next = Next::Tags(2),
                            _ => {}
                        }
                        HighlightKind::Keyword
                    }
                    Next::Plain => HighlightKind::GlyphName,
                };
                push(&mut out, i..j, kind);
                i = j;
            }
            b if is_punctuation(b) => {
                push(&mut out, i..i + 1, HighlightKind::Punctuation);
                match b {
                    b'{' => {
                        next = Next::Plain;
                        if blocks.len() < MAX_NESTING_DEPTH {
                            blocks.push(opening.take().unwrap_or(Block::Other));
                        } else {
                            opening = None;
                            overflow = overflow.saturating_add(1);
                        }
                    }
                    b'}' => {
                        next = Next::Plain;
                        if overflow > 0 {
                            overflow -= 1;
                        } else {
                            next = match blocks.pop() {
                                Some(Block::Tagged) => Next::Tags(1),
                                Some(Block::Lookup) => Next::Lookup,
                                _ => Next::Plain,
                            };
                        }
                    }
                    b';' => {
                        next = Next::Plain;
                        opening = None;
                    }
                    b'[' | b']' => next = Next::Plain,
                    _ => {}
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// The class and lookup names `code` defines (`@Name = …`, `markClass … @Name;`,
/// `lookup Name { … }`), each once, in order of first appearance, at most
/// [`MAX_DEFINITIONS`] of each. Names carry no `@`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definitions {
    pub classes: Vec<String>,
    pub lookups: Vec<String>,
}

/// Adds the names `code` defines to `into` (see [`Definitions`]).
pub fn definitions(code: &str, into: &mut Definitions) {
    let marks = highlight(code);
    let text = |h: &Highlight| code.get(h.range.clone()).unwrap_or("");
    let mut seen_classes: HashSet<String> = into.classes.iter().cloned().collect();
    let mut seen_lookups: HashSet<String> = into.lookups.iter().cloned().collect();
    let mut statement_keyword: Option<&str> = None;
    let mut statement_start = true;
    for (k, h) in marks.iter().enumerate() {
        match h.kind {
            HighlightKind::Punctuation if matches!(text(h), ";" | "{" | "}") => {
                statement_keyword = None;
                statement_start = true;
                continue;
            }
            HighlightKind::ClassName => {
                let next_is_equals = marks.get(k + 1).is_some_and(|n| n.kind == HighlightKind::Punctuation && text(n) == "=");
                let name = text(h).trim_start_matches('@');
                if (next_is_equals || statement_keyword == Some("markClass"))
                    && !name.is_empty()
                    && into.classes.len() < MAX_DEFINITIONS
                    && seen_classes.insert(name.to_string())
                {
                    into.classes.push(name.to_string());
                }
            }
            HighlightKind::LookupName => {
                let opens = marks.get(k + 1).is_some_and(|n| text(n) == "{" || text(n) == "useExtension");
                let name = text(h);
                if opens && into.lookups.len() < MAX_DEFINITIONS && seen_lookups.insert(name.to_string()) {
                    into.lookups.push(name.to_string());
                }
            }
            HighlightKind::Keyword if statement_start => statement_keyword = Some(text(h)),
            _ => {}
        }
        if h.kind != HighlightKind::Comment {
            statement_start = false;
        }
    }
}

/// What a completion inserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    Glyph,
    Class,
    Lookup,
    Keyword,
    Tag,
    Number,
}

/// One completion: `label` replaces `replace` (the partial word before the cursor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    pub replace: Range<usize>,
}

/// The names completions are drawn from: the font's glyphs, the classes and lookups of all
/// entries, the number values.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionNames<'a> {
    pub glyphs: &'a [String],
    pub classes: &'a [String],
    pub lookups: &'a [String],
    pub numbers: &'a [String],
}

/// `name` starts with `typed`, ignoring ASCII case.
fn starts_with_ignoring_case(name: &str, typed: &str) -> bool {
    name.as_bytes().get(..typed.len()).is_some_and(|head| head.eq_ignore_ascii_case(typed.as_bytes()))
}

/// The completions of `kind` among `names` for the partial word `typed`: names starting with
/// it as typed first, then those that match ignoring case; each labelled `sigil` + name.
fn narrow<'a>(
    out: &mut Vec<Completion>,
    names: impl Iterator<Item = &'a str>,
    typed: &str,
    sigil: &str,
    kind: CompletionKind,
    replace: &Range<usize>,
) {
    let room = MAX_COMPLETIONS.saturating_sub(out.len());
    let mut exact = Vec::new();
    let mut loose = Vec::new();
    for name in names {
        if exact.len() >= room {
            break;
        }
        if name.starts_with(typed) {
            exact.push(name);
        } else if loose.len() < room && starts_with_ignoring_case(name, typed) {
            loose.push(name);
        }
    }
    for name in exact.into_iter().chain(loose).take(room) {
        out.push(Completion { label: format!("{sigil}{name}"), kind, replace: replace.clone() });
    }
}

/// The completions at byte offset `cursor` of `code`: glyphs after `sub` and `pos`, lookups
/// after `lookup`, classes after `@`, numbers after `$` and inside `${…}`, feature tags after
/// `feature`, statement keywords (then glyphs) at the start of a statement, flags after
/// `lookupflag`; none in a comment or a string. Narrowed by the partial word before the
/// cursor; at most [`MAX_COMPLETIONS`]. A cursor past the end or inside a character gets none.
pub fn complete(code: &str, cursor: usize, names: &CompletionNames) -> Vec<Completion> {
    let Some(before) = code.get(..cursor) else { return Vec::new() };
    if cursor > MAX_ENTRY_BYTES {
        return Vec::new();
    }
    let bytes = before.as_bytes();
    let mut statement_first: Option<Range<usize>> = None;
    let mut previous: Option<Range<usize>> = None;
    let mut last: Option<Range<usize>> = None;
    let mut in_token = false;
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'#' => match find_byte(bytes, i, b'\n') {
                Some(j) => i = j + 1,
                None => return Vec::new(),
            },
            b'"' => match find_byte(bytes, i + 1, b'"') {
                Some(j) => {
                    statement_first.get_or_insert(i..j + 1);
                    previous = last.replace(i..j + 1);
                    i = j + 1;
                }
                None => return Vec::new(),
            },
            b';' | b'{' | b'}' => {
                statement_first = None;
                previous = None;
                last = None;
                i += 1;
            }
            b'$' if matches!(bytes.get(i + 1), Some(b'{' | b'[')) => {
                let j = token_end(bytes, i);
                let closed = matches!(bytes.get(j.wrapping_sub(1)), Some(b'}' | b']')) && j > i + 2;
                if !closed && j >= bytes.len() {
                    in_token = true;
                    break;
                }
                statement_first.get_or_insert(i..j);
                previous = last.replace(i..j);
                i = j.max(i + 1);
            }
            b'@' | b'$' | b'\\' => {
                let j = name_end(bytes, i + 1);
                statement_first.get_or_insert(i..j);
                previous = last.replace(i..j);
                i = j;
            }
            b if starts_name(b) || (b == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) => {
                let j = name_end(bytes, i + 1);
                statement_first.get_or_insert(i..j);
                previous = last.replace(i..j);
                i = j;
            }
            _ => i += 1,
        }
    }

    let mut out = Vec::new();
    if in_token {
        // Inside `${…}`: the number values.
        let start = bytes.iter().rposition(|b| !(b.is_ascii_alphanumeric() || *b == b'_')).map_or(0, |p| p + 1);
        let typed = before.get(start..).unwrap_or("");
        narrow(&mut out, names.numbers.iter().map(String::as_str), typed, "", CompletionKind::Number, &(start..cursor));
        return out;
    }
    let (partial, word_before, at_statement_start) = match &last {
        Some(word) if word.end == cursor => (word.clone(), previous, statement_first.as_ref() == Some(word)),
        _ => (cursor..cursor, last.clone(), statement_first.is_none()),
    };
    let word = |r: &Range<usize>| before.get(r.clone()).unwrap_or("");
    let typed = word(&partial);
    if let Some(rest) = typed.strip_prefix('@') {
        narrow(&mut out, names.classes.iter().map(String::as_str), rest, "@", CompletionKind::Class, &partial);
        return out;
    }
    if let Some(rest) = typed.strip_prefix('$') {
        narrow(&mut out, names.numbers.iter().map(String::as_str), rest, "$", CompletionKind::Number, &partial);
        return out;
    }
    if let Some(rest) = typed.strip_prefix('\\') {
        let after = partial.start + 1..partial.end;
        narrow(&mut out, names.glyphs.iter().map(String::as_str), rest, "", CompletionKind::Glyph, &after);
        return out;
    }
    let before_word = word_before.as_ref().map(word).unwrap_or("");
    match before_word {
        "lookup" => {
            narrow(&mut out, names.lookups.iter().map(String::as_str), typed, "", CompletionKind::Lookup, &partial);
            return out;
        }
        "feature" => {
            narrow(&mut out, crate::registry::all().iter().map(|t| t.tag), typed, "", CompletionKind::Tag, &partial);
            return out;
        }
        "script" | "language" | "languagesystem" | "table" => return out,
        _ => {}
    }
    if at_statement_start {
        narrow(&mut out, STATEMENT_KEYWORDS.iter().copied(), typed, "", CompletionKind::Keyword, &partial);
        narrow(&mut out, names.glyphs.iter().map(String::as_str), typed, "", CompletionKind::Glyph, &partial);
        return out;
    }
    let first = statement_first.as_ref().map(word).unwrap_or("");
    if first == "lookupflag" {
        narrow(&mut out, LOOKUPFLAG_KEYWORDS.iter().copied(), typed, "", CompletionKind::Keyword, &partial);
    } else if !takes_no_glyphs(first) {
        narrow(&mut out, names.glyphs.iter().map(String::as_str), typed, "", CompletionKind::Glyph, &partial);
    }
    out
}

/// A fix: edits to the entry's code, as (byte range of the code, replacement), not overlapping,
/// in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub label: String,
    pub edits: Vec<(Range<usize>, String)>,
}

impl Fix {
    /// `code` with the edits made; `None` when an edit does not fit the code (a range past
    /// its end, inside a character, or out of order).
    pub fn apply(&self, code: &str) -> Option<String> {
        let mut out = String::with_capacity(code.len().saturating_add(16));
        let mut copied = 0usize;
        for (range, text) in &self.edits {
            if range.start < copied || range.end < range.start {
                return None;
            }
            out.push_str(code.get(copied..range.start)?);
            code.get(range.clone())?;
            out.push_str(text);
            copied = range.end;
        }
        out.push_str(code.get(copied..)?);
        Some(out)
    }
}

/// Where a semicolon goes to end the statement before byte `at` of `code`: after its last
/// character that is not space or part of a comment. `None` when there is no statement there
/// (the start of the code or a block, a statement already ended) or it is further back than
/// [`MAX_FIX_SCAN`] bytes.
fn semicolon_position(code: &str, at: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut end = at.min(bytes.len());
    let floor = end.saturating_sub(MAX_FIX_SCAN);
    loop {
        while end > floor && bytes.get(end - 1).is_some_and(u8::is_ascii_whitespace) {
            end -= 1;
        }
        if end <= floor {
            return None;
        }
        let line_start = bytes.get(floor..end)?.iter().rposition(|&b| b == b'\n').map_or(floor, |p| floor + p + 1);
        let mut quoted = false;
        let comment = bytes.get(line_start..end)?.iter().position(|&b| {
            if b == b'"' {
                quoted = !quoted;
            }
            b == b'#' && !quoted
        });
        match comment {
            Some(c) => end = line_start + c,
            None => break,
        }
    }
    match bytes.get(end - 1) {
        Some(b';' | b'{') | None => None,
        Some(_) => Some(end),
    }
}

/// The fixes offered for `diagnostic` in `code` (the code of the entry it points at), the
/// names coming from `names`; empty when there are none, and for a warning. Each gives code
/// that compiles past that error.
pub fn fixes(code: &str, diagnostic: &EntryDiagnostic, names: &CompletionNames) -> Vec<Fix> {
    let Some(place) = &diagnostic.place else { return Vec::new() };
    if !diagnostic.diagnostic.is_error() {
        return Vec::new();
    }
    let message = diagnostic.message().trim().to_ascii_lowercase();
    let range = place.range.clone();
    let mut out = Vec::new();
    if message.contains("glyph not in font") {
        let Some(text) = code.get(range.clone()) else { return out };
        let (start, name) = match text.strip_prefix('\\') {
            Some(rest) => (range.start + 1, rest),
            None => (range.start, text),
        };
        for glyph in names.glyphs.iter().filter(|g| g.as_str() != name && g.eq_ignore_ascii_case(name)).take(MAX_FIXES) {
            out.push(Fix { label: format!("Replace `{name}` with `{glyph}`"), edits: vec![(start..range.end, glyph.clone())] });
        }
    } else if message.contains("undefined glyph class") {
        let Some(text) = code.get(range.clone()) else { return out };
        // The span holds the `@`; a span after it is read the same.
        let (start, name) = match text.strip_prefix('@') {
            Some(rest) => (range.start + 1, rest),
            None if range.start > 0 && code.as_bytes().get(range.start - 1) == Some(&b'@') => (range.start, text),
            None => return out,
        };
        for class in names.classes.iter().filter(|c| c.as_str() != name && c.eq_ignore_ascii_case(name)).take(MAX_FIXES) {
            out.push(Fix { label: format!("Replace `@{name}` with `@{class}`"), edits: vec![(start..range.end, class.clone())] });
        }
    } else if message == "expected ';'"
        && let Some(at) = semicolon_position(code, range.start)
    {
        out.push(Fix { label: "Insert a semicolon".into(), edits: vec![(at..at, ";".into())] });
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

    use fontcraft_fea::{CompileRequest, GlyphOrder, VirtualFiles};

    use super::*;
    use crate::source_map::{EntryKind, EntryRange, EntryRef};

    const GLYPHS: &[&str] = &[".notdef", "a", "b", "c", "f", "i", "f_i", "A", "Agrave"];

    fn glyphs() -> Vec<String> {
        GLYPHS.iter().map(|g| g.to_string()).collect()
    }

    /// Compiles `code` as a whole feature file; the diagnostics as an entry's, spans unchanged.
    fn diagnostics(code: &str) -> Vec<EntryDiagnostic> {
        let files = VirtualFiles::new("features.fea", code).unwrap();
        let order = GlyphOrder::new(glyphs()).unwrap();
        match fontcraft_fea::compile(&CompileRequest::new(&files, &order)) {
            Ok(_) => Vec::new(),
            Err(found) => found
                .into_iter()
                .map(|d| EntryDiagnostic {
                    place: Some(EntryRange { entry: EntryRef { kind: EntryKind::Prefix, index: 0 }, range: d.span.clone() }),
                    diagnostic: d,
                })
                .collect(),
        }
    }

    fn kinds(code: &str) -> Vec<(&str, HighlightKind)> {
        highlight(code).into_iter().map(|h| (&code[h.range], h.kind)).collect()
    }

    #[test]
    fn syntax_colouring_tells_keywords_tags_names_and_comments_apart() {
        use HighlightKind::*;
        let code = "languagesystem DFLT dflt;\n@Up = [A Agrave];\nlookup L1 {\n  sub a by b; # x\n} L1;\nfeature liga {\n  sub f i by f_i;\n  pos A <0 0 -10 0>;\n  pos ${a:width} \"s\";\n  lookup L1;\n} liga;\n";
        let got = kinds(code);
        let expect = [
            ("languagesystem", Keyword),
            ("DFLT", Tag),
            ("dflt", Tag),
            (";", Punctuation),
            ("@Up", ClassName),
            ("=", Punctuation),
            ("[", Punctuation),
            ("A", GlyphName),
            ("Agrave", GlyphName),
            ("]", Punctuation),
            (";", Punctuation),
            ("lookup", Keyword),
            ("L1", LookupName),
            ("{", Punctuation),
            ("sub", Keyword),
            ("a", GlyphName),
            ("by", Keyword),
            ("b", GlyphName),
            (";", Punctuation),
            ("# x", Comment),
            ("}", Punctuation),
            ("L1", LookupName),
            (";", Punctuation),
            ("feature", Keyword),
            ("liga", Tag),
            ("{", Punctuation),
            ("sub", Keyword),
            ("f", GlyphName),
            ("i", GlyphName),
            ("by", Keyword),
            ("f_i", GlyphName),
            (";", Punctuation),
            ("pos", Keyword),
            ("A", GlyphName),
            ("<", Punctuation),
            ("0", Number),
            ("0", Number),
            ("-10", Number),
            ("0", Number),
            (">", Punctuation),
            (";", Punctuation),
            ("pos", Keyword),
            ("${a:width}", Token),
            ("\"s\"", String),
            (";", Punctuation),
            ("lookup", Keyword),
            ("L1", LookupName),
            (";", Punctuation),
            ("}", Punctuation),
            ("liga", Tag),
            (";", Punctuation),
        ];
        assert_eq!(got, expect);
        // Ranges are in order and never overlap.
        let marks = highlight(code);
        assert!(marks.windows(2).all(|w| w[0].range.end <= w[1].range.start));
    }

    #[test]
    fn colouring_survives_hostile_code() {
        let deep = "{".repeat(10_000);
        for code in ["\"open string", "${never closed", "$[a; b]", "}}}} liga;", "\u{e9}t\u{e9} \\\u{e9}", deep.as_str(), "#ifdef VARIABLE\n#endif"] {
            let marks = highlight(code);
            for m in &marks {
                assert!(code.get(m.range.clone()).is_some(), "{code:?}: {m:?}");
            }
        }
        assert_eq!(kinds("#ifdef VARIABLE")[0].1, HighlightKind::Keyword);
    }

    #[test]
    fn definitions_lists_classes_and_lookups_the_code_defines() {
        let mut defs = Definitions::default();
        definitions(
            "@Up = [A];\nmarkClass acute <anchor 0 0> @TOP;\nlookup L1 { sub a by b; } L1;\nfeature liga { lookup L1; sub @Up by c; } liga;",
            &mut defs,
        );
        definitions("lookup L2 useExtension { sub a by b; } L2;\n@Up = [B];", &mut defs);
        assert_eq!(defs.classes, ["Up", "TOP"]);
        assert_eq!(defs.lookups, ["L1", "L2"]);
    }

    #[test]
    fn completion_offers_what_fits_the_position() {
        let glyphs = glyphs();
        let classes = vec!["Up".to_string(), "upper".to_string(), "Lower".to_string()];
        let lookups = vec!["L1".to_string(), "Other".to_string()];
        let numbers = vec!["tracking".to_string(), "gap".to_string()];
        let names = CompletionNames { glyphs: &glyphs, classes: &classes, lookups: &lookups, numbers: &numbers };
        let labels = |code: &str| complete(code, code.len(), &names).into_iter().map(|c| c.label).collect::<Vec<_>>();

        // Glyphs in a rule, narrowed by the partial word, exact case first.
        assert_eq!(labels("sub f i by f"), ["f", "f_i"]);
        assert_eq!(labels("sub a by A"), ["A", "Agrave", "a"]);
        let c = complete("sub f i by f_", 13, &names);
        assert_eq!(c[0].replace, 11..13);
        assert_eq!(c[0].kind, CompletionKind::Glyph);
        // Classes after @, with the @ in the label and the range.
        assert_eq!(labels("sub @u"), ["@upper", "@Up"]);
        assert_eq!(complete("sub @u", 6, &names)[0].replace, 4..6);
        // Lookups after `lookup`, numbers after `$` and inside `${…}`.
        assert_eq!(labels("feature liga { lookup "), ["L1", "Other"]);
        assert_eq!(labels("pos a $tr"), ["$tracking"]);
        assert_eq!(labels("pos a ${tracking * 2 + g"), ["gap"]);
        // Statement keywords first at a statement's start.
        assert_eq!(labels("lookupflag Ignore")[..2], ["IgnoreBaseGlyphs".to_string(), "IgnoreLigatures".to_string()]);
        assert!(labels("feature liga { su").starts_with(&["sub".to_string(), "substitute".to_string()]));
        assert!(labels("feature sm").contains(&"smcp".to_string()));
        // Nothing in a comment, a string, or a statement that names no glyph.
        assert!(labels("sub a # f").is_empty());
        assert!(labels("name \"f").is_empty());
        assert!(labels("languagesystem DFLT ").is_empty());
        // A cursor past the end or inside a character.
        assert!(complete("sub a", 99, &names).is_empty());
        assert!(complete("sub \u{e9}", 5, &names).is_empty());
    }

    #[test]
    fn completion_is_capped() {
        let glyphs: Vec<String> = (0..5_000).map(|i| format!("g{i}")).collect();
        let names = CompletionNames { glyphs: &glyphs, ..CompletionNames::default() };
        assert_eq!(complete("sub g", 5, &names).len(), MAX_COMPLETIONS);
    }

    /// Each fix, applied, gives code that compiles.
    fn fixed(code: &str, classes: &[String]) -> String {
        let found = diagnostics(code);
        let error = found.iter().find(|d| d.diagnostic.is_error()).unwrap_or_else(|| panic!("{code} compiled"));
        let glyphs = glyphs();
        let names = CompletionNames { glyphs: &glyphs, classes, ..CompletionNames::default() };
        let offered = fixes(code, error, &names);
        assert_eq!(offered.len(), 1, "{code}: {error:?} → {offered:?}");
        let new = offered[0].apply(code).unwrap();
        assert!(diagnostics(&new).iter().all(|d| !d.diagnostic.is_error()), "{new}: {:?}", diagnostics(&new));
        new
    }

    #[test]
    fn each_fix_gives_code_that_compiles() {
        assert_eq!(fixed("feature liga {\n    sub B by c;\n} liga;\n", &[]), "feature liga {\n    sub b by c;\n} liga;\n");
        assert_eq!(
            fixed("feature liga {\n    sub a by b # note\n    # more\n} liga;\n", &[]),
            "feature liga {\n    sub a by b; # note\n    # more\n} liga;\n"
        );
        let classes = vec!["Lower".to_string()];
        assert_eq!(
            fixed("@Lower = [a b];\nfeature liga {\n    sub @lower by c;\n} liga;\n", &classes),
            "@Lower = [a b];\nfeature liga {\n    sub @Lower by c;\n} liga;\n"
        );
    }

    #[test]
    fn no_fix_without_a_place_or_a_match() {
        let code = "feature liga { sub zzz by c; } liga;";
        let found = diagnostics(code);
        let glyphs = glyphs();
        let names = CompletionNames { glyphs: &glyphs, ..CompletionNames::default() };
        assert!(found.iter().all(|d| fixes(code, d, &names).is_empty()));
        let mut lost = found[0].clone();
        lost.place = None;
        assert!(fixes(code, &lost, &names).is_empty());
        assert_eq!(semicolon_position("{   ", 4), None);
        assert_eq!(semicolon_position("", 0), None);
    }
}

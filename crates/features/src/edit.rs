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
//! The fixes (`font.features.applyFix`, our own choice):
//! - a glyph name the font has only in another case (`glyph not in font`): the font's spelling;
//!   else the font's names nearest to it ([`MAX_NEAREST_DISTANCE`] edits at most, one for a name
//!   of at most four bytes), from a bounded scan ([`MAX_NEAREST_SCAN`] names a call, and the
//!   shared budget of [`CompletionNames::nearest_budget`]);
//! - a class name the code defines only in another case (`undefined glyph class`): that
//!   spelling;
//! - a lookup name the code defines only in another case (`lookup is not defined`): that
//!   spelling;
//! - a missing semicolon (`Expected ';'`): one inserted after the statement, before any
//!   comment or space;
//! - a feature or lookup block left open (`Feature block is unclosed`, `Table is unclosed`):
//!   `} tag;` inserted before the next line that only a top-level statement starts, else at
//!   the end of the entry;
//! - a closing tag that differs from the opening one (`expected tag 'liga'`): the opening tag.
//!
//! [`bodies`] reads the classes, mark classes and lookups the code defines with their bodies,
//! for the name previews.
//!
//! The third fix of the row, a class used before its entry, moves an entry rather than editing
//! code, so the engine offers it.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
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
/// Most bytes the missing-semicolon fix looks back over for the end of the statement, and the
/// block-closing fix forward over for the end of the block.
pub const MAX_FIX_SCAN: usize = 64 << 10;
/// Most glyph names the nearest-name fix looks at in one call.
pub const MAX_NEAREST_SCAN: usize = 70_000;
/// Longest name, in bytes, the nearest-name fix compares; longer names are skipped.
pub const MAX_NEAREST_NAME_BYTES: usize = 63;
/// Most nearest names offered for one unknown glyph.
pub const MAX_NEAREST: usize = 5;
/// Most edits between an unknown glyph name and a name offered for it.
pub const MAX_NEAREST_DISTANCE: usize = 2;
/// Names up to this many bytes are offered names one edit away only.
const SHORT_NAME_BYTES: usize = 4;
/// Longest block tag or lookup label the block-closing fix writes.
const MAX_LABEL_BYTES: usize = 64;
/// Most bytes of one class body [`bodies`] keeps; a longer body is left out.
pub const MAX_BODY_BYTES: usize = 64 << 10;
/// Most rules [`bodies`] keeps of one lookup.
pub const MAX_LOOKUP_RULES: usize = 50;
/// Most characters of one rule [`bodies`] keeps; the rest is cut.
pub const MAX_RULE_CHARS: usize = 300;
/// Most `markClass` statements [`bodies`] keeps of one mark class.
pub const MAX_MARK_STATEMENTS: usize = 10_000;

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
///
/// The value keeps the sets of names it holds, so each call to [`definitions`] costs time in
/// proportion to its own code, however many names earlier calls collected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definitions {
    classes: Vec<String>,
    lookups: Vec<String>,
    seen_classes: HashSet<String>,
    seen_lookups: HashSet<String>,
}

impl Definitions {
    /// The class names, in order of first appearance.
    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    /// The lookup names, in order of first appearance.
    pub fn lookups(&self) -> &[String] {
        &self.lookups
    }

    /// Adds class `name` unless it is already there. Not counted against [`MAX_DEFINITIONS`]:
    /// the caller bounds what it adds this way (the font's class entries).
    pub fn add_class(&mut self, name: &str) {
        if self.seen_classes.insert(name.to_string()) {
            self.classes.push(name.to_string());
        }
    }
}

/// Adds the names `code` defines to `into` (see [`Definitions`]).
pub fn definitions(code: &str, into: &mut Definitions) {
    if code.is_empty() {
        return;
    }
    let marks = highlight(code);
    let text = |h: &Highlight| code.get(h.range.clone()).unwrap_or("");
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
                    && into.seen_classes.insert(name.to_string())
                {
                    into.classes.push(name.to_string());
                }
            }
            HighlightKind::LookupName => {
                let opens = marks.get(k + 1).is_some_and(|n| text(n) == "{" || text(n) == "useExtension");
                let name = text(h);
                if opens && into.lookups.len() < MAX_DEFINITIONS && into.seen_lookups.insert(name.to_string()) {
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
    /// The glyph names the nearest-name fix may still look at, shared by the calls of one listing
    /// so that many unknown glyphs stay bounded; `None`: [`MAX_NEAREST_SCAN`] for each call.
    pub nearest_budget: Option<&'a Cell<usize>>,
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
        if out.is_empty() {
            for glyph in nearest_glyphs(name, names.glyphs, names.nearest_budget) {
                out.push(Fix { label: format!("Replace `{name}` with `{glyph}`"), edits: vec![(start..range.end, glyph.to_string())] });
            }
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
    } else if message.contains("lookup is not defined") {
        let Some(name) = code.get(range.clone()) else { return out };
        for lookup in names.lookups.iter().filter(|l| l.as_str() != name && l.eq_ignore_ascii_case(name)).take(MAX_FIXES) {
            out.push(Fix { label: format!("Replace `{name}` with `{lookup}`"), edits: vec![(range.clone(), lookup.clone())] });
        }
    } else if message == "expected ';'"
        && let Some(at) = semicolon_position(code, range.start)
    {
        out.push(Fix { label: "Insert a semicolon".into(), edits: vec![(at..at, ";".into())] });
    } else if (message.contains("feature block is unclosed") || message.contains("table is unclosed"))
        && let Some(fix) = close_block(code, range.clone())
    {
        out.push(fix);
    } else if let Some(open) = diagnostic.message().trim().strip_prefix("expected tag '").and_then(|rest| rest.strip_suffix('\''))
        && let Some(close) = code.get(range.clone())
        && !open.is_empty()
        && open.len() <= MAX_LABEL_BYTES
        && open != close
    {
        out.push(Fix { label: format!("Replace `{close}` with `{open}`"), edits: vec![(range, open.to_string())] });
    }
    out
}

/// The edit distance between `a` and `b` (insertions, deletions and substitutions of bytes)
/// when it is at most `max`, else `None`. Only the cells within `max` of the diagonal are
/// computed, at most `(a.len() + 1) * (2 * max + 3)`.
fn distance_within(a: &[u8], b: &[u8], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let far = max + 1;
    let n = b.len();
    let mut prev: Vec<usize> = (0..=n).map(|j| j.min(far)).collect();
    let mut cur: Vec<usize> = vec![far; n + 1];
    for (row, &ca) in a.iter().enumerate() {
        let i = row + 1;
        let lo = i.saturating_sub(max);
        let hi = (i + max).min(n);
        // The cells next to the band read as too far.
        for cell in cur.iter_mut().take(hi + 2).skip(lo.saturating_sub(1)) {
            *cell = far;
        }
        let mut best = far;
        if lo == 0 {
            if let Some(cell) = cur.get_mut(0) {
                *cell = i.min(far);
            }
            best = i.min(far);
        }
        for j in lo.max(1)..=hi {
            let cb = b.get(j - 1).copied().unwrap_or(0);
            let diagonal = prev.get(j - 1).copied().unwrap_or(far) + usize::from(ca != cb);
            let up = prev.get(j).copied().unwrap_or(far) + 1;
            let left = cur.get(j - 1).copied().unwrap_or(far) + 1;
            let value = diagonal.min(up).min(left).min(far);
            if let Some(cell) = cur.get_mut(j) {
                *cell = value;
            }
            best = best.min(value);
        }
        if best > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.get(n).copied().filter(|d| *d <= max)
}

/// The glyph names nearest to the unknown `name` that differ from it beyond case, nearest first
/// and then by name, at most [`MAX_NEAREST`]. At most [`MAX_NEAREST_SCAN`] names are looked at,
/// and no more than `budget` holds, which is lowered by those looked at.
fn nearest_glyphs<'n>(name: &str, glyphs: &'n [String], budget: Option<&Cell<usize>>) -> Vec<&'n str> {
    let typed = name.as_bytes();
    if typed.is_empty() || typed.len() > MAX_NEAREST_NAME_BYTES {
        return Vec::new();
    }
    let max = if typed.len() <= SHORT_NAME_BYTES { 1 } else { MAX_NEAREST_DISTANCE };
    let allowed = budget.map_or(MAX_NEAREST_SCAN, |b| b.get().min(MAX_NEAREST_SCAN));
    let mut found: Vec<(usize, &str)> = Vec::new();
    let mut looked = 0usize;
    for glyph in glyphs.iter().take(allowed) {
        looked += 1;
        let g = glyph.as_bytes();
        if g.len() > MAX_NEAREST_NAME_BYTES || g.len().abs_diff(typed.len()) > max || glyph.eq_ignore_ascii_case(name) {
            continue;
        }
        if let Some(d) = distance_within(typed, g, max) {
            found.push((d, glyph.as_str()));
        }
    }
    if let Some(b) = budget {
        b.set(b.get().saturating_sub(looked));
    }
    found.sort_unstable();
    found.into_iter().take(MAX_NEAREST).map(|(_, g)| g).collect()
}

/// The words that start a statement only outside feature and lookup blocks: an open block ends
/// before the line that starts with one.
const TOP_LEVEL_WORDS: &[&str] = &["feature", "table", "languagesystem", "variation", "conditionset", "anchorDef", "valueRecordDef"];

/// Whether `code` has the word `word` at byte `at`, not followed by a name character.
fn word_at(code: &str, at: usize, word: &str) -> bool {
    code.get(at..).is_some_and(|rest| rest.starts_with(word)) && !code.as_bytes().get(at + word.len()).copied().is_some_and(is_name_byte)
}

/// The fix for a block left open whose tag or label is at `label`: `} label;` inserted before the
/// first line inside the block, at its own depth, that starts with a word only a top-level
/// statement starts with (and, for a lookup, `lookup`), else after the end of the code. `None`
/// when no `{` follows the label, the block is closed by a brace, the scan passes
/// [`MAX_FIX_SCAN`] bytes, or the code ends inside a nested block.
fn close_block(code: &str, label: Range<usize>) -> Option<Fix> {
    let name = code.get(label.clone())?.trim();
    if name.is_empty() || name.len() > MAX_LABEL_BYTES {
        return None;
    }
    let lookup = code.get(..label.start)?.trim_end().ends_with("lookup");
    let bytes = code.as_bytes();
    let limit = label.end.saturating_add(MAX_FIX_SCAN).min(bytes.len());
    let open = label.end + bytes.get(label.end..limit)?.iter().position(|&b| b == b'{')?;
    if !matches!(code.get(label.end..open)?.trim(), "" | "useExtension") {
        return None;
    }
    let fix = |at: usize, text: String| Fix { label: format!("Close the block with `}} {name};`"), edits: vec![(at..at, text)] };
    let mut depth = 1usize;
    let mut at = open + 1;
    while at < limit {
        let b = bytes.get(at).copied()?;
        match b {
            b'#' => {
                at = bytes.get(at..limit)?.iter().position(|&c| c == b'\n').map_or(limit, |p| at + p);
                continue;
            }
            b'"' => {
                at = bytes.get(at + 1..limit)?.iter().position(|&c| c == b'"').map_or(limit, |p| at + 2 + p);
                continue;
            }
            b'{' => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return None;
                }
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return None;
                }
            }
            b'\n' if depth == 1 => {
                let line = at + 1;
                let first = line + bytes.get(line..limit)?.iter().position(|c| !matches!(c, b' ' | b'\t')).unwrap_or(limit - line);
                let ends = TOP_LEVEL_WORDS.iter().any(|w| word_at(code, first, w)) || (lookup && word_at(code, first, "lookup"));
                if ends {
                    return Some(fix(line, format!("}} {name};\n")));
                }
            }
            _ => {}
        }
        at += 1;
    }
    if limit < bytes.len() || depth != 1 {
        return None;
    }
    let end = code.trim_end().len();
    Some(fix(end, format!("\n}} {name};")))
}

/// The bodies of the classes, mark classes and lookups code defines, for the name previews
/// (`font.features.previewName`). The first definition of a name is kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bodies {
    /// `@Name = body;`: the body between `=` and `;`, at most [`MAX_BODY_BYTES`].
    pub classes: HashMap<String, String>,
    /// `markClass glyphs <anchor …> @Name;`: each statement's glyphs and anchor, in order, at
    /// most [`MAX_MARK_STATEMENTS`] a class.
    pub mark_classes: HashMap<String, Vec<(String, String)>>,
    /// `lookup Name { … } Name;`: its first [`MAX_LOOKUP_RULES`] rules, and whether it has more.
    pub lookups: HashMap<String, (Vec<String>, bool)>,
}

/// The keywords a rule of a lookup starts with.
const RULE_KEYWORDS: &[&str] = &["sub", "substitute", "rsub", "reversesub", "pos", "position", "ignore", "enum", "enumerate"];

/// Adds the classes, mark classes and lookups `code` defines, with their bodies, to `into`; at
/// most [`MAX_DEFINITIONS`] names of each kind. One pass over [`highlight`]'s stretches.
pub fn bodies(code: &str, into: &mut Bodies) {
    if code.is_empty() {
        return;
    }
    let marks = highlight(code);
    let text = |h: &Highlight| code.get(h.range.clone()).unwrap_or("");
    let mut statement: Option<usize> = None;
    let mut depth = 0usize;
    // The open lookup's name and the depth of its body.
    let mut lookup: Option<(String, usize)> = None;
    for (k, h) in marks.iter().enumerate() {
        if h.kind == HighlightKind::Comment {
            continue;
        }
        let t = text(h);
        if h.kind == HighlightKind::Punctuation && matches!(t, ";" | "{" | "}") {
            match t {
                ";" => {
                    if let Some(first) = statement {
                        let inside = lookup.as_ref().filter(|(_, d)| *d == depth).map(|(name, _)| name.as_str());
                        statement_body(code, &marks, first, k, inside, into);
                    }
                }
                "{" => {
                    depth += 1;
                    let opens = statement.and_then(|first| {
                        let keyword = marks.get(first).filter(|m| m.kind == HighlightKind::Keyword && text(m) == "lookup")?;
                        let name = marks.get(first + 1).filter(|m| m.kind == HighlightKind::LookupName)?;
                        (keyword.range.end <= name.range.start).then(|| text(name).to_string())
                    });
                    if lookup.is_none()
                        && let Some(name) = opens
                    {
                        if into.lookups.len() < MAX_DEFINITIONS {
                            into.lookups.entry(name.clone()).or_default();
                        }
                        lookup = Some((name, depth));
                    }
                }
                _ => {
                    if lookup.as_ref().is_some_and(|(_, d)| *d == depth) {
                        lookup = None;
                    }
                    depth = depth.saturating_sub(1);
                }
            }
            statement = None;
            continue;
        }
        if statement.is_none() {
            statement = Some(k);
        }
    }
}

/// Reads one statement, the stretches `first..end` of `marks` (`end` is its `;`): a class
/// definition, a `markClass` statement, or a rule of the lookup `inside`.
fn statement_body(code: &str, marks: &[Highlight], first: usize, end: usize, inside: Option<&str>, into: &mut Bodies) {
    let text = |h: &Highlight| code.get(h.range.clone()).unwrap_or("");
    let (Some(head), Some(semi)) = (marks.get(first), marks.get(end)) else { return };
    let between = |from: usize, to: usize| code.get(from..to).map(str::trim).unwrap_or("");
    let next_is_equals = marks.get(first + 1).is_some_and(|m| m.kind == HighlightKind::Punctuation && text(m) == "=");
    if head.kind == HighlightKind::ClassName && next_is_equals {
        let name = text(head).trim_start_matches('@');
        let Some(equals) = marks.get(first + 1) else { return };
        let body = between(equals.range.end, semi.range.start);
        if !name.is_empty() && body.len() <= MAX_BODY_BYTES && into.classes.len() < MAX_DEFINITIONS && !into.classes.contains_key(name) {
            into.classes.insert(name.to_string(), body.to_string());
        }
    } else if head.kind == HighlightKind::Keyword && text(head) == "markClass" {
        let inner = marks.get(first + 1..end).unwrap_or(&[]);
        let Some(class) = inner.iter().rev().find(|m| m.kind == HighlightKind::ClassName) else { return };
        let Some(anchor) = inner.iter().find(|m| m.kind == HighlightKind::Punctuation && text(m) == "<") else { return };
        let name = text(class).trim_start_matches('@');
        let glyphs = between(head.range.end, anchor.range.start);
        let point = between(anchor.range.start, class.range.start);
        if name.is_empty() || glyphs.len() > MAX_BODY_BYTES || point.len() > MAX_BODY_BYTES {
            return;
        }
        if !into.mark_classes.contains_key(name) && into.mark_classes.len() >= MAX_DEFINITIONS {
            return;
        }
        let list = into.mark_classes.entry(name.to_string()).or_default();
        if list.len() < MAX_MARK_STATEMENTS {
            list.push((glyphs.to_string(), point.to_string()));
        }
    } else if let Some(name) = inside
        && head.kind == HighlightKind::Keyword
        && RULE_KEYWORDS.contains(&text(head))
        && let Some((rules, more)) = into.lookups.get_mut(name)
    {
        if rules.len() >= MAX_LOOKUP_RULES {
            *more = true;
            return;
        }
        let rule = code.get(head.range.start..semi.range.end).unwrap_or("");
        let collapsed = rule.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut kept: String = collapsed.chars().take(MAX_RULE_CHARS).collect();
        if kept.len() < collapsed.len() {
            kept.push('…');
        }
        rules.push(kept);
    }
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
        assert_eq!(defs.classes(), ["Up", "TOP"]);
        assert_eq!(defs.lookups(), ["L1", "L2"]);
    }

    #[test]
    fn completion_offers_what_fits_the_position() {
        let glyphs = glyphs();
        let classes = vec!["Up".to_string(), "upper".to_string(), "Lower".to_string()];
        let lookups = vec!["L1".to_string(), "Other".to_string()];
        let numbers = vec!["tracking".to_string(), "gap".to_string()];
        let names = CompletionNames { glyphs: &glyphs, classes: &classes, lookups: &lookups, numbers: &numbers, nearest_budget: None };
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
    /// The fix for the error whose message contains `wanted`, applied: the code that compiles.
    fn fixed_by(code: &str, wanted: &str, lookups: &[String]) -> String {
        let found = diagnostics(code);
        let error = found
            .iter()
            .find(|d| d.diagnostic.is_error() && d.message().to_ascii_lowercase().contains(wanted))
            .unwrap_or_else(|| panic!("{code}: no error `{wanted}` in {found:?}"));
        let glyphs = glyphs();
        let names = CompletionNames { glyphs: &glyphs, lookups, ..CompletionNames::default() };
        let offered = fixes(code, error, &names);
        let first = offered.first().unwrap_or_else(|| panic!("{code}: {error:?} has no fix"));
        let new = first.apply(code).unwrap();
        assert!(diagnostics(&new).iter().all(|d| !d.diagnostic.is_error()), "{new}: {:?}", diagnostics(&new));
        new
    }

    /// Each new fix, applied, gives code that compiles: the nearest glyph name, a lookup name in
    /// another case, a block left open, a closing tag that differs.
    #[test]
    fn each_new_fix_gives_code_that_compiles() {
        assert_eq!(
            fixed_by("feature liga {\n    sub Agrav by c;\n} liga;\n", "glyph not in font", &[]),
            "feature liga {\n    sub Agrave by c;\n} liga;\n"
        );
        let lookups = vec!["Lone".to_string()];
        assert_eq!(
            fixed_by("lookup Lone {\n    sub a by b;\n} Lone;\nfeature liga {\n    lookup lone;\n} liga;\n", "lookup is not defined", &lookups),
            "lookup Lone {\n    sub a by b;\n} Lone;\nfeature liga {\n    lookup Lone;\n} liga;\n"
        );
        assert_eq!(
            fixed_by("lookup L1 {\n    sub a by b;\nfeature liga {\n    lookup L1;\n} liga;\n", "is unclosed", &[]),
            "lookup L1 {\n    sub a by b;\n} L1;\nfeature liga {\n    lookup L1;\n} liga;\n"
        );
        assert_eq!(
            fixed_by("feature liga {\n    sub a by b;\nfeature calt {\n    sub c by f;\n} calt;\n", "is unclosed", &[]),
            "feature liga {\n    sub a by b;\n} liga;\nfeature calt {\n    sub c by f;\n} calt;\n"
        );
        assert_eq!(fixed_by("feature liga {\n    sub a by b;\n} lgia;\n", "expected tag", &[]), "feature liga {\n    sub a by b;\n} liga;\n");
    }

    #[test]
    fn edit_distance_is_banded_and_exact() {
        assert_eq!(distance_within(b"Agrav", b"Agrave", 2), Some(1));
        assert_eq!(distance_within(b"abc", b"acb", 2), Some(2));
        assert_eq!(distance_within(b"kitten", b"sitting", 2), None);
        assert_eq!(distance_within(b"kitten", b"sitting", 3), Some(3));
        assert_eq!(distance_within(b"", b"ab", 2), Some(2));
        assert_eq!(distance_within(b"a", b"abcd", 2), None);
        // A short name is offered names one edit away only.
        let names: Vec<String> = ["ab", "abc", "xyz"].iter().map(|n| n.to_string()).collect();
        assert_eq!(nearest_glyphs("a", &names, None), vec!["ab"]);
    }

    /// The fix search stays bounded: on 1 MB of code a block left open is looked for over at most
    /// [`MAX_FIX_SCAN`] bytes, and the nearest-name search over a large glyph set takes from the
    /// shared budget.
    #[test]
    fn fix_search_stays_bounded_on_a_megabyte_of_code() {
        let small = "lookup L1 {\n    sub a by b;\nfeature liga {\n    lookup L1;\n} liga;\n";
        let unclosed = diagnostics(small).into_iter().find(|d| d.message().contains("is unclosed")).expect("the block is unclosed");
        let big = format!("lookup L1 {{\n{}", "    sub a by b;\n".repeat(70_000));
        assert!(big.len() > 1 << 20);
        let glyphs = glyphs();
        let names = CompletionNames { glyphs: &glyphs, ..CompletionNames::default() };
        let started = std::time::Instant::now();
        assert!(fixes(&big, &unclosed, &names).is_empty(), "the end of the block is past the scan");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());

        let unknown =
            diagnostics("feature liga {\n    sub Agrav by c;\n} liga;\n").into_iter().find(|d| d.diagnostic.is_error()).expect("an unknown glyph");
        let many: Vec<String> = (0..200_000).map(|i| format!("Agra{i:06}")).collect();
        let budget = Cell::new(100_000);
        let names = CompletionNames { glyphs: &many, nearest_budget: Some(&budget), ..CompletionNames::default() };
        let started = std::time::Instant::now();
        let code = "feature liga {\n    sub Agrav by c;\n} liga;\n";
        fixes(code, &unknown, &names);
        assert_eq!(budget.get(), 100_000 - MAX_NEAREST_SCAN, "one call looks at most at MAX_NEAREST_SCAN names");
        fixes(code, &unknown, &names);
        assert_eq!(budget.get(), 0);
        assert!(fixes(code, &unknown, &names).is_empty(), "an empty budget looks at nothing");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    /// Class definitions, mark classes with their anchors and the first rules of each lookup.
    #[test]
    fn bodies_read_classes_mark_classes_and_lookup_rules() {
        let mut rules = String::new();
        for i in 0..60 {
            rules.push_str(&format!("    sub a{i} by b;\n"));
        }
        let code = format!(
            "@Up = [A Agrave]; # caps\nmarkClass [acutecomb] <anchor 0 500> @TOP;\nmarkClass gravecomb <anchor 10 520> @TOP;\nlookup L1 {{\n    lookupflag 0;\n    sub a by b;\n}} L1;\nlookup L2 {{\n{rules}}} L2;\nfeature liga {{\n    @Low = [a b];\n    lookup L1;\n}} liga;\n"
        );
        let mut got = Bodies::default();
        bodies(&code, &mut got);
        assert_eq!(got.classes.get("Up").map(String::as_str), Some("[A Agrave]"));
        assert_eq!(got.classes.get("Low").map(String::as_str), Some("[a b]"));
        assert_eq!(
            got.mark_classes.get("TOP"),
            Some(&vec![("[acutecomb]".to_string(), "<anchor 0 500>".to_string()), ("gravecomb".to_string(), "<anchor 10 520>".to_string())])
        );
        assert_eq!(got.lookups.get("L1"), Some(&(vec!["sub a by b;".to_string()], false)));
        let (l2, more) = got.lookups.get("L2").expect("L2");
        assert_eq!(l2.len(), MAX_LOOKUP_RULES);
        assert!(*more);
        assert_eq!(l2.first().map(String::as_str), Some("sub a0 by b;"));
    }
    /// A class replaced by a class of another length: the message names both counts the right way round.
    #[test]
    fn a_class_of_another_length_gets_both_counts() {
        let found = diagnostics("@Two = [a b];\n@Three = [a b c];\nfeature liga {\n    sub @Two by @Three;\n} liga;\n");
        assert!(found.iter().any(|d| d.message().starts_with("the replacement class has 3 glyphs and the target class 2")), "{found:?}");
    }
}

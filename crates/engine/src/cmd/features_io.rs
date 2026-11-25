//! Feature files: File › Import › Features… reads a `.fea` file into the font's prefixes, classes
//! and features, and `file.exportFeatures` writes a master's assembled feature code (the `fea`
//! format) or its kerning as a `kern` feature (the `fea-kern` format). Rows: `file.importFeatures`,
//! `dialog.importFeatures`, `file.exportFeatures` (`spec/inventory/07-import-export.toml`), the
//! `fea` and `fea-kern` formats (`spec/inventory/30-formats.toml`).
//!
//! Importing:
//! 1. The file is read (at most [`MAX_TEXT_BYTES`], UTF-8) and every `include(…)` is replaced by
//!    the file it names, tried against the imported file's folder and then the including file's
//!    folder, as the feature-file specification resolves them. An included file must lie inside
//!    the imported file's folder; at most [`fontcraft_fea::MAX_INCLUDE_FILES`] files,
//!    [`fontcraft_fea::MAX_INCLUDE_DEPTH`] levels and [`MAX_TEXT_BYTES`] of text in all. Text
//!    passed as `text` keeps its includes as written.
//! 2. A text in the form the UFO writer gives (`fontcraft_io_ufo::features`) splits back into
//!    the entries it was written from, flags and notes included. Any other text is split at the
//!    top level: a class definition (`@Name = [ … ];` or `@Name = @Other;`) becomes a class, a
//!    `feature tag { … } tag;` block a feature, and each run of other statements and comments
//!    between them a prefix named `Prefix`. A `# Prefix: name` comment starts a prefix of that
//!    name. Statements the split does not recognise (a `useExtension` feature, an `anon` block,
//!    a class whose value is not a list) stay in a prefix as written.
//! 3. The entries are added after the font's, as one undo step. A class or prefix whose name the
//!    font already uses gets the first free numeric suffix (`Upper2`), and the imported code's
//!    references to a renamed class follow it. A feature keeps its tag: blocks of one tag add up
//!    in the feature-file language, and a suffix would not be a tag.
//!
//! The scanner never recurses except through includes, which are bounded by depth; braces only
//! count a depth, capped at [`fontcraft_io_ufo::features::MAX_DEPTH`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use fontcraft_features::{MAX_ENTRIES, MAX_ENTRY_BYTES, MasterValues, Target, assemble};
use fontcraft_io_ufo::features as ufo_features;
use fontcraft_model::{Features, Font, Snippet};
use fontcraft_spacing::kerning::{Direction, GroupSide, Position, Side, facing, group_list, pairs};
use fontcraft_spacing::{metrics_source, round_half_up};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most bytes of feature text one import reads, included files counted.
pub const MAX_TEXT_BYTES: usize = 16 << 20;
/// Most include statements one import follows (an empty file included from many places costs
/// no text, so the text cap alone does not bound the work).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MAX_INCLUDE_STATEMENTS: usize = 100_000;
/// The name an imported prefix gets when no `# Prefix:` comment names it.
pub const PREFIX_NAME: &str = "Prefix";
/// Most names one result lists in each list; the counts are always complete.
const MAX_REPORTED: usize = 1000;
/// Longest file name `fileName` may give, in bytes.
const MAX_FILE_NAME: usize = 200;
/// Bytes that end a word of feature code.
const SEPARATORS: &[u8] = b"{};[]()=,<>'\"#";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.importFeatures",
            "Features…",
            ["File", "Import"],
            None,
            "{file?: path of a .fea file | text?: feature code (exactly one of the two)} adds the prefixes, classes and features of a feature file after the font's, as one undo step. A file's include statements are replaced by the files they name, which must lie inside the file's folder; text keeps its includes as written. A file in the form UFO features are written in splits back into its entries with their flags; any other file splits at the top level: each class definition is a class, each feature block a feature, and each run of other statements and comments between them a prefix named Prefix (a `# Prefix: name` comment names the prefix that follows it). A class or prefix whose name the font already has gets a numeric suffix (Upper2), and the imported code's references to a renamed class follow it; a feature keeps its tag even when the font has one with that tag. Refused: unbalanced braces, an unclosed string, a missing or outside include, nesting deeper than 64 or includes deeper than 50, more than 16 MB of text, no feature code → {source, prefixes, classes, features (the names added), counts: {prefixes, classes, features}, renamed: [{kind, from, to}], sharedTags, includes (files read), unresolvedIncludes}",
            has_doc,
            import_features
        ).files(const { &[FileParam::read("file")] }),
        cmd!(
            "file.exportFeatures",
            "Export Features",
            [],
            None,
            "{format?: \"fea\" (default) | \"fea-kern\", master?: id (default: the default master), path?: file, destination?: folder, fileName?: name without extension (with destination; default Family-Master, or Family-Master-kern for fea-kern)} writes a feature file. fea: the font's enabled prefixes, classes and features assembled for the master as export compiles them (classes, then prefixes, then features; tokens expanded, the syntax extensions lowered), without running Update first. fea-kern: the master's kerning pairs (those of the master it follows, when linked) as a kern feature: the groups the pairs use as @MMK_L_ and @MMK_R_ classes, then a lookup per direction with glyph pairs first, glyph–group pairs as enum rules next and group pairs last; right-to-left pairs also move the glyph. Values are rounded half up; pairs naming missing glyphs, empty groups or names feature code cannot spell are left out and counted in warnings. With path, or destination, the file is written atomically; with neither, the text is returned → {master, format, bytes, entries?, pairs?, classes?, warnings, path?, text?}",
            has_doc,
            export_features
        ).files(const { &[FileParam::write("path"), FileParam::write("destination")] }).writes_files(),
    ]
}

fn clip(text: &str) -> String {
    text.chars().take(60).collect()
}

/// The 1-based line of byte `offset` in `text`.
fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes().iter().take(offset).filter(|&&b| b == b'\n').count() + 1
}

// ── Scanning ──────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Word,
    Comment,
    Str,
    Sym(u8),
}

/// The tokens of feature code, one at a time: kind, start and end (byte offsets). Every
/// boundary is next to an ASCII byte, so each is a character boundary.
struct Lexer<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.text.as_bytes().get(at).copied()
    }

    fn next(&mut self) -> std::result::Result<Option<(Tok, usize, usize)>, String> {
        while self.byte(self.pos).is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
        let start = self.pos;
        let Some(c) = self.byte(start) else { return Ok(None) };
        let kind = match c {
            b'#' => {
                while self.byte(self.pos).is_some_and(|c| c != b'\n' && c != b'\r') {
                    self.pos += 1;
                }
                Tok::Comment
            }
            b'"' => {
                self.pos += 1;
                while self.byte(self.pos).is_some_and(|c| c != b'"') {
                    self.pos += 1;
                }
                if self.byte(self.pos).is_none() {
                    return Err(format!("the string that starts on line {} is never closed", line_of(self.text, start)));
                }
                self.pos += 1;
                Tok::Str
            }
            c if SEPARATORS.contains(&c) => {
                self.pos += 1;
                Tok::Sym(c)
            }
            _ => {
                while self.byte(self.pos).is_some_and(|c| !c.is_ascii_whitespace() && !SEPARATORS.contains(&c)) {
                    self.pos += 1;
                }
                Tok::Word
            }
        };
        Ok(Some((kind, start, self.pos)))
    }

    /// The next token when it is `sym`, consumed; otherwise nothing is consumed.
    fn take_sym(&mut self, sym: u8) -> std::result::Result<Option<usize>, String> {
        let save = self.pos;
        match self.next()? {
            Some((Tok::Sym(s), _, end)) if s == sym => Ok(Some(end)),
            _ => {
                self.pos = save;
                Ok(None)
            }
        }
    }

    /// After `include`: the file name between the parentheses (start and end of the raw text)
    /// and where the statement ends (after its optional `;`). `None` when no `(` follows.
    fn include_path(&mut self, at: usize) -> std::result::Result<Option<(usize, usize, usize)>, String> {
        if self.take_sym(b'(')?.is_none() {
            return Ok(None);
        }
        let start = self.pos;
        while self.byte(self.pos).is_some_and(|c| c != b')') {
            self.pos += 1;
        }
        if self.byte(self.pos).is_none() {
            return Err(format!("the include on line {} has no closing parenthesis", line_of(self.text, at)));
        }
        let end = self.pos;
        self.pos += 1;
        let stmt_end = self.take_sym(b';')?.unwrap_or(self.pos);
        Ok(Some((start, end, stmt_end)))
    }
}

// ── Includes ──────────────────────────────────────────────────────────────────────────────────

/// The include paths a text names, as written (at most [`MAX_REPORTED`]).
fn include_paths(text: &str) -> std::result::Result<Vec<String>, String> {
    let mut lex = Lexer::new(text);
    let mut out = Vec::new();
    while let Some((tok, start, end)) = lex.next()? {
        if tok == Tok::Word
            && text.get(start..end) == Some("include")
            && let Some((a, b, _)) = lex.include_path(start)?
            && out.len() < MAX_REPORTED
        {
            out.push(text.get(a..b).unwrap_or_default().trim().to_string());
        }
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_text(path: &std::path::Path, limit: usize) -> std::result::Result<String, String> {
    use std::io::Read;
    let shown = path.display();
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot open {shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("cannot open {shown}: it is not a file"));
    }
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    std::fs::File::open(path).and_then(|f| f.take(cap).read_to_end(&mut bytes)).map_err(|e| format!("cannot read {shown}: {e}"))?;
    if bytes.len() > limit {
        return Err(format!("{shown} is larger than the {} MB a feature file may hold", MAX_TEXT_BYTES >> 20));
    }
    let text = String::from_utf8(bytes).map_err(|_| format!("{shown} is not UTF-8 text: a feature file is plain text"))?;
    Ok(text.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(text))
}

/// Replaces include statements by the files they name (native targets only).
#[cfg(not(target_arch = "wasm32"))]
struct Inliner {
    /// The imported file's folder, canonical: every included file lies inside it.
    root: PathBuf,
    cache: HashMap<PathBuf, std::sync::Arc<str>>,
    statements: usize,
    /// The files read, relative to `root`, in the order first read.
    files: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Inliner {
    fn push(out: &mut String, piece: &str) -> std::result::Result<(), String> {
        if out.len().saturating_add(piece.len()) > MAX_TEXT_BYTES {
            return Err(format!("the file with its includes holds more than the {} MB a feature file may hold", MAX_TEXT_BYTES >> 20));
        }
        out.push_str(piece);
        Ok(())
    }

    /// The file `raw` names, from `dir` (the including file's folder): its canonical path and
    /// text.
    /// `line` gives the statement's line number, counted only for an error message.
    fn read(&mut self, raw: &str, dir: &std::path::Path, line: impl Fn() -> usize) -> std::result::Result<(PathBuf, std::sync::Arc<str>), String> {
        if raw.is_empty() {
            return Err(format!("the include on line {} names no file", line()));
        }
        let mut found = None;
        for candidate in [self.root.join(raw), dir.join(raw)] {
            if let Ok(path) = std::fs::canonicalize(&candidate) {
                if !path.starts_with(&self.root) {
                    return Err(format!("the included file `{}` lies outside the folder of the imported file", clip(raw)));
                }
                found = Some(path);
                break;
            }
        }
        let Some(path) = found else {
            return Err(format!("the included file `{}` (line {}) does not exist", clip(raw), line()));
        };
        if let Some(text) = self.cache.get(&path) {
            return Ok((path, text.clone()));
        }
        if self.cache.len() >= fontcraft_fea::MAX_INCLUDE_FILES {
            return Err(format!("the file includes more than {} files", fontcraft_fea::MAX_INCLUDE_FILES));
        }
        let text: std::sync::Arc<str> = read_text(&path, MAX_TEXT_BYTES)?.into();
        self.cache.insert(path.clone(), text.clone());
        let shown = path.strip_prefix(&self.root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| raw.to_string());
        if self.files.len() < MAX_REPORTED {
            self.files.push(shown);
        }
        Ok((path, text))
    }

    fn inline(&mut self, text: &str, dir: &std::path::Path, depth: usize, out: &mut String) -> std::result::Result<(), String> {
        let mut lex = Lexer::new(text);
        let mut copied = 0;
        while let Some((tok, start, end)) = lex.next()? {
            if tok != Tok::Word || text.get(start..end) != Some("include") {
                continue;
            }
            let Some((a, b, stmt_end)) = lex.include_path(start)? else { continue };
            let line = || line_of(text, start);
            let raw = text.get(a..b).unwrap_or_default().trim();
            if depth >= fontcraft_fea::MAX_INCLUDE_DEPTH {
                return Err(format!(
                    "includes are nested more than {} deep (line {} includes `{}`): files may include each other",
                    fontcraft_fea::MAX_INCLUDE_DEPTH,
                    line(),
                    clip(raw)
                ));
            }
            self.statements += 1;
            if self.statements > MAX_INCLUDE_STATEMENTS {
                return Err(format!("the file follows more than {MAX_INCLUDE_STATEMENTS} include statements"));
            }
            let (path, body) = self.read(raw, dir, line)?;
            Self::push(out, text.get(copied..start).unwrap_or_default())?;
            let sub_dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| self.root.clone());
            self.inline(&body, &sub_dir, depth + 1, out).map_err(|e| format!("in `{}`: {e}", clip(raw)))?;
            Self::push(out, "\n")?;
            copied = stmt_end;
        }
        Self::push(out, text.get(copied..).unwrap_or_default())
    }
}

// ── Splitting ─────────────────────────────────────────────────────────────────────────────────

/// Where the statement that starts with the token at `start..end` ends: after its `;` at depth
/// 0, after an include's file name, after an anonymous block's closing `} tag;`, or at the end
/// of the text.
fn statement_end(lex: &mut Lexer, tok: Tok, start: usize, end: usize) -> std::result::Result<usize, String> {
    let text = lex.text;
    let word = if tok == Tok::Word { text.get(start..end) } else { None };
    if word == Some("include")
        && let Some((_, _, stmt_end)) = lex.include_path(start)?
    {
        return Ok(stmt_end);
    }
    if matches!(word, Some("anon" | "anonymous")) {
        return anon_end(lex, start);
    }
    let mut depth = 0usize;
    let mut last = end;
    let mut current = Some((tok, start, end));
    while let Some((tok, at, to)) = current {
        last = to;
        match tok {
            Tok::Sym(b'{') => {
                depth += 1;
                if depth > ufo_features::MAX_DEPTH {
                    return Err(format!("blocks are nested more than {} deep on line {}", ufo_features::MAX_DEPTH, line_of(text, at)));
                }
            }
            Tok::Sym(b'}') => {
                depth = depth.checked_sub(1).ok_or_else(|| format!("the `}}` on line {} closes no block", line_of(text, at)))?;
            }
            Tok::Sym(b';') if depth == 0 => return Ok(to),
            Tok::Word if text.get(at..to) == Some("include") => {
                if let Some((_, _, stmt_end)) = lex.include_path(at)? {
                    last = stmt_end;
                }
            }
            _ => {}
        }
        current = lex.next()?;
    }
    if depth > 0 {
        return Err(format!("the block that starts on line {} is never closed", line_of(text, start)));
    }
    Ok(last)
}

/// The end of an anonymous block: its content is not feature code, so it runs to the first
/// `}` followed by its tag and `;`.
fn anon_end(lex: &mut Lexer, start: usize) -> std::result::Result<usize, String> {
    let text = lex.text;
    let never = || format!("the anonymous block that starts on line {} is never closed", line_of(text, start));
    let tag = match lex.next()? {
        Some((Tok::Word, a, b)) => text.get(a..b).unwrap_or_default(),
        _ => return Err(never()),
    };
    let Some(open) = lex.take_sym(b'{')? else { return Err(never()) };
    let body = text.get(open..).unwrap_or_default();
    for (i, _) in body.match_indices('}') {
        let rest = body.get(i + 1..).unwrap_or_default().trim_start();
        if let Some(after) = rest.strip_prefix(tag)
            && after.trim_start().starts_with(';')
        {
            let end = text.len() - after.trim_start().len() + 1;
            lex.pos = end;
            return Ok(end);
        }
    }
    Err(never())
}

/// A block's code as an entry stores it: blank lines at the start and whitespace at the end
/// taken off, and the first line's indentation kept only when it stands on a line of its own.
fn block_code(body: &str) -> String {
    let body = body.trim_end();
    let first = body.find(|c: char| !c.is_whitespace()).unwrap_or(body.len());
    let start = match body.get(..first).and_then(|lead| lead.rfind('\n')) {
        Some(i) => i + 1,
        None => first,
    };
    body.get(start..).unwrap_or_default().to_string()
}

/// A class definition as an entry: `@Name = [ … ];` (its code is the list) or `@Name = @Other;`.
fn as_class(stmt: &str) -> Option<Snippet> {
    let (name, value) = stmt.strip_prefix('@')?.split_once('=')?;
    let name = name.trim();
    let bad = |s: &str| s.is_empty() || s.contains(|c: char| c.is_whitespace() || "#[]{};=\"@".contains(c));
    if bad(name) {
        return None;
    }
    let value = value.trim().strip_suffix(';')?.trim_end();
    let code = match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        Some(inner) if !inner.contains(['[', ']']) => inner.trim(),
        Some(_) => return None,
        None if value.strip_prefix('@').is_some_and(|v| !bad(v)) => value,
        None => return None,
    };
    Some(Snippet { name: name.to_string(), code: code.to_string(), ..Default::default() })
}

fn is_tag(tag: &str) -> bool {
    (1..=4).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_graphic() && !SEPARATORS.contains(&b) && b != b'@' && b != b'\\')
}

/// A `feature tag { … } tag;` block as an entry. A block with `useExtension`, a closing tag
/// that differs, or anything else in its head is not one.
fn as_feature(stmt: &str) -> Option<Snippet> {
    let open = stmt.find('{')?;
    let close = stmt.rfind('}')?;
    if close < open {
        return None;
    }
    let head: Vec<&str> = stmt.get(..open)?.split_whitespace().collect();
    let ["feature", tag] = head.as_slice() else { return None };
    let tail = stmt.get(close + 1..)?.trim().strip_suffix(';')?.trim_end();
    if tail != *tag || !is_tag(tag) {
        return None;
    }
    Some(Snippet { name: tag.to_string(), code: block_code(stmt.get(open + 1..close)?), ..Default::default() })
}

/// The entries of a feature text split at the top level (see the module doc).
fn split(text: &str) -> std::result::Result<Features, String> {
    let mut out = Features::default();
    // The prefix being collected: its name (from a `# Prefix:` comment) and the text it spans.
    let mut run: Option<(Option<String>, usize, usize)> = None;
    let flush = |out: &mut Features, run: &mut Option<(Option<String>, usize, usize)>| {
        if let Some((name, s, e)) = run.take() {
            let code = block_code(text.get(s..e).unwrap_or_default());
            if name.is_some() || !code.is_empty() {
                out.prefixes.push(Snippet { name: name.unwrap_or_else(|| PREFIX_NAME.to_string()), code, ..Default::default() });
            }
        }
    };
    let extend = |run: &mut Option<(Option<String>, usize, usize)>, start: usize, end: usize| match run {
        Some(r) => r.2 = end,
        None => *run = Some((None, start, end)),
    };
    let mut lex = Lexer::new(text);
    while let Some((tok, start, end)) = lex.next()? {
        if out.prefixes.len() + out.classes.len() + out.features.len() > MAX_ENTRIES {
            return Err(format!("the file holds more than {MAX_ENTRIES} prefixes, classes and features"));
        }
        match tok {
            Tok::Comment => {
                let name = text.get(start..end).and_then(|c| c.strip_prefix("# Prefix:")).map(str::trim).filter(|n| !n.is_empty());
                match name {
                    Some(name) => {
                        flush(&mut out, &mut run);
                        run = Some((Some(name.to_string()), end, end));
                    }
                    None => extend(&mut run, start, end),
                }
            }
            // A lone `;` belongs to the statement before it.
            Tok::Sym(b';') => extend(&mut run, start, end),
            Tok::Sym(b'}') => return Err(format!("the `}}` on line {} closes no block", line_of(text, start))),
            _ => {
                let stmt_end = statement_end(&mut lex, tok, start, end)?;
                let stmt = text.get(start..stmt_end).unwrap_or_default();
                if let Some(class) = as_class(stmt) {
                    flush(&mut out, &mut run);
                    out.classes.push(class);
                } else if let Some(feature) = as_feature(stmt) {
                    flush(&mut out, &mut run);
                    out.features.push(feature);
                } else {
                    extend(&mut run, start, stmt_end);
                }
            }
        }
    }
    flush(&mut out, &mut run);
    Ok(out)
}

/// The entries of a feature file: the UFO writer's form read back exactly, else the top-level
/// split.
pub(super) fn entries_of(text: &str) -> std::result::Result<Features, String> {
    let written = ufo_features::read(text);
    if !written.is_empty() && !ufo_features::is_kept_text(&written) {
        let mut f = written;
        for p in &mut f.prefixes {
            if p.name == ufo_features::ANONYMOUS {
                p.name = PREFIX_NAME.to_string();
            }
        }
        return Ok(f);
    }
    split(text)
}

// ── Adding the entries ────────────────────────────────────────────────────────────────────────

/// `name`, or `name` with the first numeric suffix from 2 that `taken` does not hold.
fn unique(name: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(name) {
        return name.to_string();
    }
    // Among taken.len() + 1 suffixes at least one is free.
    (2..taken.len() + 3).map(|n| format!("{name}{n}")).find(|n| !taken.contains(n)).unwrap_or_else(|| format!("{name}{}", taken.len() + 2))
}

/// `code` with each class reference `@old` that `renames` holds written as `@new`. Comments
/// and strings are left alone; code the scanner cannot read comes back unchanged.
fn rename_classes(code: &str, renames: &HashMap<String, String>) -> String {
    if renames.is_empty() {
        return code.to_string();
    }
    let mut lex = Lexer::new(code);
    let mut out = String::with_capacity(code.len());
    let mut copied = 0;
    loop {
        match lex.next() {
            Ok(Some((Tok::Word, start, end))) => {
                let Some(new) = code.get(start..end).and_then(|w| w.strip_prefix('@')).and_then(|n| renames.get(n)) else { continue };
                out.push_str(code.get(copied..start).unwrap_or_default());
                out.push('@');
                out.push_str(new);
                copied = end;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => return code.to_string(),
        }
    }
    out.push_str(code.get(copied..).unwrap_or_default());
    out
}

/// What one import added.
#[derive(Default)]
pub(super) struct Added {
    pub(super) prefixes: Vec<String>,
    pub(super) classes: Vec<String>,
    pub(super) features: Vec<String>,
    pub(super) counts: [usize; 3],
    pub(super) renamed: Vec<Value>,
    pub(super) shared_tags: Vec<String>,
}

fn report(list: &mut Vec<String>, name: &str) {
    if list.len() < MAX_REPORTED {
        list.push(name.to_string());
    }
}

/// Adds `new` after the entries of `into` (see the module doc).
pub(super) fn add_entries(into: &mut Features, mut new: Features) -> Added {
    let mut added = Added::default();
    let mut class_names: HashSet<String> = into.classes.iter().map(|c| c.name.clone()).collect();
    let font_classes = class_names.clone();
    let mut renames: HashMap<String, String> = HashMap::new();
    for c in &mut new.classes {
        let name = unique(&c.name, &class_names);
        if name != c.name {
            if added.renamed.len() < MAX_REPORTED {
                added.renamed.push(json!({"kind": "class", "from": c.name, "to": name}));
            }
            // References follow a class the font already had a class of the same name for; a
            // name the file itself repeats keeps its references as written.
            if font_classes.contains(&c.name) {
                renames.entry(c.name.clone()).or_insert_with(|| name.clone());
            }
            c.name = name;
        }
        class_names.insert(c.name.clone());
    }
    let mut prefix_names: HashSet<String> = into.prefixes.iter().map(|p| p.name.clone()).collect();
    for p in &mut new.prefixes {
        let name = unique(&p.name, &prefix_names);
        if name != p.name {
            if added.renamed.len() < MAX_REPORTED {
                added.renamed.push(json!({"kind": "prefix", "from": p.name, "to": name}));
            }
            p.name = name;
        }
        prefix_names.insert(p.name.clone());
    }
    let tags: HashSet<&str> = into.features.iter().map(|f| f.name.as_str()).collect();
    let mut shared = HashSet::new();
    for f in &new.features {
        if tags.contains(f.name.as_str()) && shared.insert(f.name.clone()) {
            report(&mut added.shared_tags, &f.name);
        }
    }
    for s in new.prefixes.iter_mut().chain(&mut new.classes).chain(&mut new.features) {
        s.code = rename_classes(&s.code, &renames);
    }
    for p in &new.prefixes {
        report(&mut added.prefixes, &p.name);
    }
    for c in &new.classes {
        report(&mut added.classes, &c.name);
    }
    for f in &new.features {
        report(&mut added.features, &f.name);
    }
    added.counts = [new.prefixes.len(), new.classes.len(), new.features.len()];
    into.prefixes.extend(new.prefixes);
    into.classes.extend(new.classes);
    into.features.extend(new.features);
    added
}

// ── file.importFeatures ───────────────────────────────────────────────────────────────────────

/// The text to import: a file's, with its includes inlined, or `text` as given; the files read
/// and the includes left as written.
fn import_text(p: &Params) -> Result<(String, &'static str, Vec<String>, Vec<String>)> {
    match (p.opt_str("file")?, p.opt_str("text")?) {
        (Some(file), None) => {
            if file.trim().is_empty() {
                return Err(p.bad("`file` is empty: choose a feature file"));
            }
            let (text, files) = read_file(p, file)?;
            Ok((text, "file", files, Vec::new()))
        }
        (None, Some(text)) => {
            if text.len() > MAX_TEXT_BYTES {
                return Err(p.bad(format!("`text` is {} bytes long; at most {MAX_TEXT_BYTES} are allowed", text.len())));
            }
            let text = text.strip_prefix('\u{feff}').unwrap_or(text);
            let unresolved = include_paths(text).map_err(|e| p.bad(e))?;
            Ok((text.to_string(), "text", Vec::new(), unresolved))
        }
        (None, None) => Err(p.bad("give `file` or `text`")),
        _ => Err(p.bad("give `file` or `text`, not both")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(p: &Params, file: &str) -> Result<(String, Vec<String>)> {
    let path = std::path::Path::new(file);
    let text = read_text(path, MAX_TEXT_BYTES).map_err(|e| p.bad(e))?;
    let canonical = std::fs::canonicalize(path).map_err(|e| p.bad(format!("cannot open {}: {e}", path.display())))?;
    let root = canonical.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let mut inliner = Inliner { root: root.clone(), cache: HashMap::new(), statements: 0, files: Vec::new() };
    let mut out = String::new();
    inliner.inline(&text, &root, 0, &mut out).map_err(|e| p.bad(e))?;
    Ok((out, inliner.files))
}

#[cfg(target_arch = "wasm32")]
fn read_file(_: &Params, _: &str) -> Result<(String, Vec<String>)> {
    Err(EngineError::Unsupported("reading a file by path in the browser; pass the file's contents as `text`".into()))
}

fn import_features(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.importFeatures", v)?;
    s.font()?;
    let (text, source, files, unresolved) = import_text(&p)?;
    let new = entries_of(&text).map_err(|e| p.bad(e))?;
    if new.is_empty() {
        return Err(p.bad("the file holds no feature code"));
    }
    if let Some(big) = new.prefixes.iter().chain(&new.classes).chain(&new.features).find(|e| e.code.len() > MAX_ENTRY_BYTES) {
        return Err(p.bad(format!("`{}` holds more than the {} MB of code one entry may hold", clip(&big.name), MAX_ENTRY_BYTES >> 20)));
    }
    let count = new.prefixes.len() + new.classes.len() + new.features.len();
    let added = s.edit("Import Features", |font| {
        let features = std::sync::Arc::make_mut(&mut font.features);
        let have = features.prefixes.len() + features.classes.len() + features.features.len();
        if have.saturating_add(count) > MAX_ENTRIES {
            return Err(p.bad(format!("the font would hold more than {MAX_ENTRIES} prefixes, classes and features")));
        }
        Ok(add_entries(features, new))
    })?;
    let [prefixes, classes, features] = added.counts;
    Ok(json!({
        "source": source,
        "prefixes": added.prefixes,
        "classes": added.classes,
        "features": added.features,
        "counts": {"prefixes": prefixes, "classes": classes, "features": features},
        "renamed": added.renamed,
        "sharedTags": added.shared_tags,
        "includes": files,
        "unresolvedIncludes": unresolved,
    }))
}

// ── file.exportFeatures ───────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Fea,
    FeaKern,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Format::Fea => "fea",
            Format::FeaKern => "fea-kern",
        }
    }
}

/// Feature-file keywords a glyph name must be escaped with `\` to stand for itself.
const KEYWORDS: &[&str] = &[
    "anchor",
    "anchorDef",
    "anon",
    "anonymous",
    "by",
    "contour",
    "cursive",
    "device",
    "enum",
    "enumerate",
    "exclude_dflt",
    "excludeDFLT",
    "feature",
    "from",
    "ignore",
    "IgnoreBaseGlyphs",
    "IgnoreLigatures",
    "IgnoreMarks",
    "include",
    "include_dflt",
    "includeDFLT",
    "language",
    "languagesystem",
    "lookup",
    "lookupflag",
    "mark",
    "MarkAttachmentType",
    "markClass",
    "nameid",
    "NULL",
    "parameters",
    "pos",
    "position",
    "required",
    "reversesub",
    "RightToLeft",
    "rsub",
    "script",
    "sub",
    "substitute",
    "subtable",
    "table",
    "useExtension",
    "UseMarkFilteringSet",
    "valueRecordDef",
];

/// A glyph name as feature code spells it, or `None` for a name it cannot spell.
fn fea_glyph(name: &str) -> Option<String> {
    let ok = |c: char| c.is_ascii_alphanumeric() || "._-*+:^|~".contains(c);
    if name.is_empty() || !name.chars().all(ok) || name.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return None;
    }
    Some(if KEYWORDS.contains(&name) { format!("\\{name}") } else { name.to_string() })
}

/// One side of a rule as written: a glyph, or a class with its definition.
enum RuleSide {
    Glyph(String),
    Class(String),
}

/// Why a pair is left out of the kern feature.
#[derive(Default)]
struct LeftOut {
    missing: usize,
    unspellable: usize,
    empty_groups: usize,
    values: usize,
}

/// The master's kerning as a `kern` feature: the text, the rules and the classes written.
fn kern_feature_text(font: &Font, master: &str) -> (String, usize, usize, Vec<String>) {
    let source = metrics_source(font, master).map_or_else(|| master.to_string(), |m| m.id.clone());
    let members: HashMap<GroupSide, HashMap<&str, Vec<&str>>> =
        [GroupSide::Left, GroupSide::Right].into_iter().map(|side| (side, group_list(font, side).into_iter().collect())).collect();
    let mut classes: Vec<String> = Vec::new();
    let mut defined: HashSet<String> = HashSet::new();
    let mut left_out = LeftOut::default();
    let mut lookups: Vec<String> = Vec::new();
    let mut rules = 0usize;
    for direction in [Direction::Ltr, Direction::Rtl] {
        let mut tiers: [Vec<String>; 3] = Default::default();
        for pair in pairs::stored_pairs(font, direction, &source) {
            let value = round_half_up(pair.value);
            if !(-32768.0..=32767.0).contains(&value) {
                left_out.values += 1;
                continue;
            }
            // Within the i16 range just checked.
            let value = value as i32;
            let mut side = |s: Side, position: Position| -> Option<RuleSide> {
                match s {
                    Side::Glyph(name) => {
                        if font.glyph(&name).is_none() {
                            left_out.missing += 1;
                            return None;
                        }
                        let Some(spelled) = fea_glyph(&name) else {
                            left_out.unspellable += 1;
                            return None;
                        };
                        Some(RuleSide::Glyph(spelled))
                    }
                    Side::Group(group) => {
                        let group_side = facing(direction, position);
                        let prefix = match group_side {
                            GroupSide::Right => "MMK_L_",
                            GroupSide::Left => "MMK_R_",
                            _ => return None,
                        };
                        let glyphs: Vec<String> = members
                            .get(&group_side)
                            .and_then(|m| m.get(group.as_str()))
                            .map(|g| g.iter().filter_map(|n| fea_glyph(n)).collect())
                            .unwrap_or_default();
                        if glyphs.is_empty() {
                            left_out.empty_groups += 1;
                            return None;
                        }
                        if group.is_empty() || !group.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
                            left_out.unspellable += 1;
                            return None;
                        }
                        let class = format!("@{prefix}{group}");
                        if defined.insert(class.clone()) {
                            classes.push(format!("{class} = [{}];", glyphs.join(" ")));
                        }
                        Some(RuleSide::Class(class))
                    }
                }
            };
            let Some(left) = side(pair.left(), Position::First) else { continue };
            let Some(right) = side(pair.right(), Position::Second) else { continue };
            let record = match direction {
                Direction::Rtl => format!("<{value} 0 {value} 0>"),
                _ => value.to_string(),
            };
            let (tier, text) = match (&left, &right) {
                (RuleSide::Glyph(l), RuleSide::Glyph(r)) => (0, format!("pos {l} {r} {record};")),
                (RuleSide::Class(l), RuleSide::Class(r)) => (2, format!("pos {l} {r} {record};")),
                (RuleSide::Glyph(l) | RuleSide::Class(l), RuleSide::Glyph(r) | RuleSide::Class(r)) => (1, format!("enum pos {l} {r} {record};")),
            };
            if let Some(t) = tiers.get_mut(tier) {
                t.push(text);
                rules += 1;
            }
        }
        if tiers.iter().all(Vec::is_empty) {
            continue;
        }
        let name = if direction == Direction::Rtl { "kern_rtl" } else { "kern_ltr" };
        let body: Vec<String> = tiers.concat().into_iter().map(|r| format!("    {r}")).collect();
        lookups.push(format!("lookup {name} {{\n{}\n}} {name};", body.join("\n")));
    }
    let mut warnings = Vec::new();
    for (n, what) in [
        (left_out.missing, "name glyphs the font does not have"),
        (left_out.empty_groups, "name kerning groups without members"),
        (left_out.unspellable, "name glyphs or groups that feature code cannot spell"),
        (left_out.values, "have values outside -32768 to 32767"),
    ] {
        if n > 0 {
            warnings.push(format!("{n} kerning pairs were left out: they {what}"));
        }
    }
    if lookups.is_empty() {
        return (String::new(), 0, 0, warnings);
    }
    let mut text = String::new();
    if !classes.is_empty() {
        text.push_str(&classes.join("\n"));
        text.push_str("\n\n");
    }
    text.push_str(&format!("feature kern {{\n{}\n}} kern;\n", lookups.join("\n\n")));
    (text, rules, classes.len(), warnings)
}

fn export_features(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.exportFeatures", v)?;
    let format = match p.opt_str("format")? {
        None | Some("fea") => Format::Fea,
        Some("fea-kern") => Format::FeaKern,
        Some(other) => return Err(p.bad(format!("`format` must be fea or fea-kern, got `{}`", clip(other)))),
    };
    let data = s.glyph_data()?;
    let font = s.font()?;
    let master = super::shared::master(&p, font)?;
    let mut out = json!({"master": master, "format": format.name()});
    let text = match format {
        Format::Fea => {
            let values = MasterValues::new(font, &data, &master).map_err(|e| p.bad(e.to_string()))?;
            let assembly = assemble(font, &font.features, &Target::Master(master.clone()), &values).map_err(|e| p.bad(e.to_string()))?;
            let entries = [&font.features.prefixes, &font.features.classes, &font.features.features]
                .iter()
                .map(|list| list.iter().filter(|e| !e.disabled).count())
                .sum::<usize>();
            if let Some(o) = out.as_object_mut() {
                o.insert("entries".into(), json!(entries));
                o.insert("warnings".into(), json!([]));
            }
            assembly.expanded_text()
        }
        Format::FeaKern => {
            let (text, rules, classes, warnings) = kern_feature_text(font, &master);
            if text.is_empty() {
                let why = warnings.first().map(|w| format!(" ({w})")).unwrap_or_default();
                return Err(p.bad(format!("the master has no kerning pairs to write{why}")));
            }
            if let Some(o) = out.as_object_mut() {
                o.insert("pairs".into(), json!(rules));
                o.insert("classes".into(), json!(classes));
                o.insert("warnings".into(), json!(warnings));
            }
            text
        }
    };
    let Some(o) = out.as_object_mut() else { return Ok(out) };
    o.insert("bytes".into(), json!(text.len()));
    match output_path(&p, font, &master, format)? {
        None => {
            o.insert("text".into(), json!(text));
        }
        Some(path) => {
            write_text(&path, &text)?;
            o.insert("path".into(), json!(path.to_string_lossy()));
        }
    }
    Ok(out)
}

/// Where the export goes: `path`, or `destination` joined with `fileName` and `.fea`; `None`
/// returns the text.
fn output_path(p: &Params, font: &Font, master: &str, format: Format) -> Result<Option<PathBuf>> {
    let name = p.opt_str("fileName")?;
    match (p.opt_str("path")?, p.opt_str("destination")?) {
        (Some(_), Some(_)) => Err(p.bad("give `path` or `destination`, not both")),
        (Some(path), None) => {
            if name.is_some() {
                return Err(p.bad("`fileName` goes with `destination`; `path` names the file itself"));
            }
            if path.trim().is_empty() {
                return Err(p.bad("`path` is empty"));
            }
            Ok(Some(PathBuf::from(path)))
        }
        (None, Some(dir)) => {
            let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => {
                    if n.len() > MAX_FILE_NAME || n.chars().any(|c| c.is_control() || matches!(c, '/' | '\\')) || n == "." || n == ".." {
                        return Err(p.bad(format!("`fileName` must be a file name of at most {MAX_FILE_NAME} bytes without / or \\")));
                    }
                    n.to_string()
                }
                None => default_file_name(font, master, format),
            };
            Ok(Some(PathBuf::from(dir).join(format!("{name}.fea"))))
        }
        (None, None) => {
            if name.is_some() {
                return Err(p.bad("`fileName` goes with `destination`"));
            }
            Ok(None)
        }
    }
}

/// `Family-Master` (`Family-Master-kern` for the kerning), with characters file systems refuse
/// replaced by `-`.
fn default_file_name(font: &Font, master: &str, format: Format) -> String {
    let style = font.masters.iter().find(|m| m.id == master).map(|m| m.name.trim()).unwrap_or_default();
    let joined = [font.family_name.trim(), style].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("-");
    let clean: String = joined
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .take(MAX_FILE_NAME / 4)
        .collect();
    let base = if clean.trim().is_empty() || clean.starts_with('.') { "Features".to_string() } else { clean };
    match format {
        Format::Fea => base,
        Format::FeaKern => format!("{base}-kern"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_text(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fontcraft_format::output::create_dir_all(dir).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", dir.display())))?;
    }
    fontcraft_format::write_atomic(path, text.as_bytes())?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn write_text(_: &std::path::Path, _: &str) -> Result<()> {
    Err(EngineError::Unsupported("writing a file by path in the browser; leave out `path` and `destination` to get the text".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_split_keeps_unrecognised_statements_in_prefixes() {
        let text = "languagesystem DFLT dflt;\n\n@Upper = [A B\n  C];\n@Alias = @Upper;\n@Bad = [a] [b];\n\nlookup L1 {\n  sub a by b;\n} L1;\n\nfeature liga { sub f i by f_i; } liga;\nfeature calt useExtension {\n  sub a by b;\n} calt;\n# Prefix: Tail\nanon TEST {\n } } x ;\n} TEST;\nfeature ss01 {\n  featureNames { name \"Alt\"; };\n  sub a by a.ss01;\n} ss01;\n";
        let f = split(text).unwrap();
        let names = |l: &[Snippet]| l.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&f.classes), ["Upper", "Alias"]);
        assert_eq!(f.classes[0].code, "A B\n  C");
        assert_eq!(f.classes[1].code, "@Upper");
        assert_eq!(names(&f.features), ["liga", "ss01"]);
        assert_eq!(f.features[0].code, "sub f i by f_i;");
        assert_eq!(f.features[1].code, "  featureNames { name \"Alt\"; };\n  sub a by a.ss01;");
        assert_eq!(names(&f.prefixes), ["Prefix", "Prefix", "Prefix", "Tail"]);
        assert_eq!(f.prefixes[0].code, "languagesystem DFLT dflt;");
        assert_eq!(f.prefixes[1].code, "@Bad = [a] [b];\n\nlookup L1 {\n  sub a by b;\n} L1;");
        assert!(f.prefixes[2].code.starts_with("feature calt useExtension {"));
        assert_eq!(f.prefixes[3].code, "anon TEST {\n } } x ;\n} TEST;");
    }

    #[test]
    fn renamed_classes_are_followed_in_code_but_not_in_comments() {
        let renames = HashMap::from([("Upper".to_string(), "Upper2".to_string())]);
        let code = "sub [@Upper @UpperX] by @Upper.sc; # @Upper\n";
        assert_eq!(rename_classes(code, &renames), "sub [@Upper2 @UpperX] by @Upper.sc; # @Upper\n");
        let taken: HashSet<String> = ["A".to_string(), "A2".to_string()].into();
        assert_eq!(unique("A", &taken), "A3");
        assert_eq!(unique("B", &taken), "B");
    }
}

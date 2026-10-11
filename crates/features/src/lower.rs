//! The reference's additions to the feature language, rewritten to standard syntax (M8.4;
//! `docs/spec/12-feature-code.md` § 4).
//!
//! Rows: `featureCode.delete`, `.deleteContextual`, `.deleteLowering`, `.standaloneLookup`,
//! `.standaloneRedundant`, `.standaloneLowering`, `.optionalLookupEndName`,
//! `.multipleLanguages`, `.multipleLanguagesCompiler`, `.multipleSubstitutionClasses`,
//! `.multipleSubstitutionSingletons`, `.lookupflagReplaces`, `.emptyLookupflag`; the static
//! half of `featureCode.ifdefVariable` and `.ifndefVariable`.
//!
//! One pass over the text, without recursion, braces and brackets at most
//! [`fontcraft_fea::MAX_NESTING_DEPTH`] deep. For a static target `#ifdef VARIABLE` blocks are
//! removed and `#ifndef VARIABLE` blocks kept; for the variable target the reverse. Any other
//! symbol, `#else`, a nested block or a stray `#endif` is an error at the directive. A block
//! without `#endif` ends where its enclosing block closes, or at the end of the code. Runs after
//! token expansion.
//!
//! What each extension becomes:
//!
//! | Written | Lowered |
//! |---|---|
//! | `delete g;`, `del g' ctx;` | `sub g by NULL;`, `sub g' ctx by NULL;` |
//! | `lookup n { … };` | `lookup n { … } n;` |
//! | `lookupflag;` | `lookupflag 0;` |
//! | `standalone lookup n { … } n;` in a feature block | the feature block closed before the lookup and reopened after it, with its `script`, `language` and `lookupflag` statements repeated |
//! | `standalone lookup …` outside any block | the keyword dropped |
//! | `sub @A by @B [x];`, `sub @A by @B @One;` | the one-member class written as its glyph, so it repeats |
//! | `language A B C;` and the statements up to the next `script`, `language` or the block's end | the statement and the statements once per tag; a named lookup block is written once and referred to by name after that |
//! | `sub @A by @B @C;` | unchanged: the compiler reads it |
//!
//! A feature entry is lowered with its `feature tag { … } tag;` wrapper: the `standalone`
//! lowering closes and reopens that block, so it needs the tag. Outside every block `standalone`
//! is redundant (a prefix's lookups) and is dropped.
//!
//! A one-member class is only known when its definition is in the same text; a class defined in
//! another entry reaches the compiler as it is.

use std::collections::HashMap;
use std::ops::Range;

use fontcraft_fea::{Diagnostic, MAX_DIAGNOSTICS, MAX_NESTING_DEPTH, Severity};

use crate::source_map::OffsetMap;

/// Most bytes of code one call lowers, and most bytes the lowered text may grow to (the
/// assembled text's cap).
pub const MAX_LOWER_BYTES: usize = 16 << 20;

/// `code` with the extensions lowered, and the map from the result back to `code`. Diagnostics
/// have no file; their spans are in `code`.
pub fn lower(code: &str, variable: bool) -> Result<(String, OffsetMap), Vec<Diagnostic>> {
    if code.len() > MAX_LOWER_BYTES {
        return Err(vec![error(0..0, format!("the feature code is larger than {} MB", MAX_LOWER_BYTES >> 20))]);
    }
    let (text, map) = Lowering::new(code, variable).run()?;
    match split_languages(&text) {
        Ok(None) => Ok((text, map)),
        Ok(Some((split, split_map))) => Ok((split, OffsetMap::compose(&map, &split_map))),
        Err(found) => Err(found.into_iter().map(|d| Diagnostic { span: map.source_range(d.span.clone()), ..d }).collect()),
    }
}

fn error(span: Range<usize>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::at("", span, Severity::Error, message)
}

// ---------------------------------------------------------------------------------------------
// Tokens

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Word,
    Str,
    Comment,
    /// The file name of `include( … )`, which may hold any character but `)`.
    Path,
    Punct(u8),
}

#[derive(Clone, Copy, Debug)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

impl Tok {
    fn range(self) -> Range<usize> {
        self.start..self.end
    }

    fn is(self, byte: u8) -> bool {
        self.kind == Kind::Punct(byte)
    }
}

const PUNCT: &[u8] = b"{}[]();',<>=";

/// A lexer that is cheap to clone, so the pass can look ahead a statement.
#[derive(Clone)]
struct Lexer<'a> {
    code: &'a str,
    pos: usize,
    /// The last token was the word `include`.
    include: bool,
    /// The last token was the `(` of an include statement.
    path: bool,
}

impl<'a> Lexer<'a> {
    fn new(code: &'a str) -> Self {
        Self { code, pos: 0, include: false, path: false }
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.code.as_bytes().get(at).copied()
    }

    fn eat_while(&mut self, keep: impl Fn(u8) -> bool) {
        while self.byte(self.pos).is_some_and(&keep) {
            self.pos += 1;
        }
    }

    fn text(&self, tok: Tok) -> &'a str {
        self.code.get(tok.range()).unwrap_or("")
    }

    /// Continues at `pos`, a token boundary.
    fn reset(&mut self, pos: usize) {
        self.pos = pos;
        self.include = false;
        self.path = false;
    }

    fn next_tok(&mut self) -> Option<Tok> {
        self.eat_while(|b| b.is_ascii_whitespace());
        let start = self.pos;
        let first = self.byte(start)?;
        if std::mem::take(&mut self.path) && first != b')' {
            self.eat_while(|b| b != b')');
            return Some(Tok { kind: Kind::Path, start, end: self.pos });
        }
        let kind = match first {
            b'#' => {
                self.eat_while(|b| b != b'\n' && b != b'\r');
                Kind::Comment
            }
            b'"' => {
                self.pos += 1;
                self.eat_while(|b| b != b'"');
                if self.byte(self.pos).is_some() {
                    self.pos += 1;
                }
                Kind::Str
            }
            b if PUNCT.contains(&b) => {
                self.pos += 1;
                Kind::Punct(b)
            }
            _ => {
                self.eat_while(|b| !b.is_ascii_whitespace() && !PUNCT.contains(&b) && b != b'#' && b != b'"');
                Kind::Word
            }
        };
        let tok = Tok { kind, start, end: self.pos };
        match kind {
            Kind::Comment => {}
            Kind::Punct(b'(') => self.path = std::mem::take(&mut self.include),
            Kind::Word => self.include = self.text(tok) == "include",
            _ => self.include = false,
        }
        Some(tok)
    }

    fn next_significant(&mut self) -> Option<Tok> {
        loop {
            let tok = self.next_tok()?;
            if tok.kind != Kind::Comment {
                return Some(tok);
            }
        }
    }
}

enum Directive<'a> {
    /// `#ifdef` (kept for the variable target) or `#ifndef` (kept for static targets).
    Open {
        keep_if_variable: bool,
        symbol: Option<&'a str>,
    },
    Else,
    Endif,
}

fn directive(comment: &str) -> Option<Directive<'_>> {
    let rest = comment.strip_prefix('#')?;
    let len = rest.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
    let word = rest.get(..len)?;
    let symbol = rest.get(len..).and_then(|tail| tail.split_whitespace().next());
    match word {
        "ifdef" => Some(Directive::Open { keep_if_variable: true, symbol }),
        "ifndef" => Some(Directive::Open { keep_if_variable: false, symbol }),
        "else" | "elif" => Some(Directive::Else),
        "endif" => Some(Directive::Endif),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Output and its offset map

struct Out<'a> {
    code: &'a str,
    text: String,
    segments: Vec<(Range<usize>, Range<usize>)>,
    /// Input bytes up to here are in `text` or were dropped.
    copied: usize,
    /// Where the current stretch of copied text starts, in the output and in the input.
    run_out: usize,
    run_in: usize,
    changed: bool,
}

impl<'a> Out<'a> {
    fn new(code: &'a str) -> Self {
        Self { code, text: String::with_capacity(code.len()), segments: Vec::new(), copied: 0, run_out: 0, run_in: 0, changed: false }
    }

    fn copy_to(&mut self, pos: usize) {
        if pos > self.copied
            && let Some(s) = self.code.get(self.copied..pos)
        {
            self.text.push_str(s);
            self.copied = pos;
        }
    }

    fn flush(&mut self) {
        if self.text.len() > self.run_out {
            self.segments.push((self.run_out..self.text.len(), self.run_in..self.copied));
        }
    }

    fn restart(&mut self, input: usize) {
        self.copied = self.copied.max(input);
        self.run_out = self.text.len();
        self.run_in = self.copied;
        self.changed = true;
    }

    /// Writes `with` in place of `range` of the input (empty for an insertion).
    fn replace(&mut self, range: Range<usize>, with: &str) {
        self.copy_to(range.start);
        self.flush();
        let start = self.text.len();
        self.text.push_str(with);
        if !with.is_empty() {
            self.segments.push((start..self.text.len(), range.clone()));
        }
        self.restart(range.end);
    }

    /// Leaves out the input up to `end`.
    fn skip_to(&mut self, end: usize) {
        self.copy_to(self.copied);
        self.flush();
        self.restart(end);
    }

    fn too_long(&self) -> bool {
        self.text.len() > MAX_LOWER_BYTES
    }

    fn finish(mut self) -> (String, OffsetMap) {
        self.copy_to(self.code.len());
        if !self.changed {
            return (self.text, OffsetMap::identity());
        }
        self.flush();
        let mut map = OffsetMap::identity();
        for (output, input) in self.segments {
            map.push(output, input);
        }
        (self.text, map)
    }
}

// ---------------------------------------------------------------------------------------------
// The pass

/// What fea-rs carries from one statement of a feature block to the next, as the statements
/// that set it. A `standalone` split repeats them when it reopens the block.
#[derive(Default)]
struct FeatureState {
    tag: String,
    use_extension: bool,
    /// (tag, statement)
    script: Option<(String, String)>,
    /// (first tag, statement)
    language: Option<(String, String)>,
    lookupflag: Option<String>,
}

enum Frame {
    Feature,
    Lookup { name: String, reopen: bool },
    Block,
    Bracket,
}

#[derive(Default)]
enum Pending {
    #[default]
    None,
    Feature {
        tag: Option<String>,
        use_extension: bool,
    },
    Lookup {
        name: Option<String>,
        split: bool,
    },
}

#[derive(Clone, Copy)]
enum RecordKind {
    Script,
    Language,
    Lookupflag,
}

/// After a block's closing brace: its end name, then `;`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tail {
    None,
    Name,
    Semi,
}

struct Lowering<'a> {
    code: &'a str,
    variable: bool,
    out: Out<'a>,
    errors: Vec<Diagnostic>,
    fatal: bool,
    stack: Vec<Frame>,
    feature: Option<FeatureState>,
    pending: Pending,
    stmt_start: bool,
    tail: Tail,
    /// The text that reopens the feature block after a standalone lookup's closing statement.
    reopen: Option<String>,
    /// `standalone` was lowered by closing the feature; the `lookup` that follows reopens it.
    split_next: bool,
    delete_pending: bool,
    /// Inside a `lookupflag` statement; whether it named any flag.
    lookupflag: Option<bool>,
    recording: Option<(RecordKind, Vec<&'a str>)>,
    /// Rewrites planned for the current substitution, last first.
    rewrites: Vec<(Range<usize>, String)>,
    skip_until: usize,
    /// One-member classes defined in this text: name to glyph.
    singletons: HashMap<&'a str, &'a str>,
    /// The brace depth of the open `#ifdef`/`#ifndef` block whose contents are kept.
    open_if: Option<usize>,
}

impl<'a> Lowering<'a> {
    fn new(code: &'a str, variable: bool) -> Self {
        Self {
            code,
            variable,
            out: Out::new(code),
            errors: Vec::new(),
            fatal: false,
            stack: Vec::new(),
            feature: None,
            pending: Pending::None,
            stmt_start: true,
            tail: Tail::None,
            reopen: None,
            split_next: false,
            delete_pending: false,
            lookupflag: None,
            recording: None,
            rewrites: Vec::new(),
            skip_until: 0,
            singletons: HashMap::new(),
            open_if: None,
        }
    }

    fn text(&self, tok: Tok) -> &'a str {
        self.code.get(tok.range()).unwrap_or("")
    }

    fn error(&mut self, span: Range<usize>, message: impl Into<String>) {
        if self.errors.len() < MAX_DIAGNOSTICS {
            self.errors.push(error(span, message));
        }
    }

    fn replace(&mut self, range: Range<usize>, with: &str) {
        self.out.replace(range.clone(), with);
        if self.out.too_long() && !self.fatal {
            self.fatal = true;
            self.error(range, format!("the feature code grows past {} MB when its syntax extensions are rewritten", MAX_LOWER_BYTES >> 20));
        }
    }

    fn insert(&mut self, at: usize, with: &str) {
        self.replace(at..at, with);
    }

    fn run(mut self) -> Result<(String, OffsetMap), Vec<Diagnostic>> {
        let mut lexer = Lexer::new(self.code);
        while !self.fatal {
            let Some(tok) = lexer.next_tok() else { break };
            if tok.start < self.skip_until {
                continue;
            }
            if tok.kind == Kind::Comment {
                self.comment(tok, &mut lexer);
            } else {
                self.significant(tok, &lexer);
            }
        }
        if let Some(reopen) = self.reopen.take() {
            self.insert(self.code.len(), &reopen);
        }
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        Ok(self.out.finish())
    }

    fn comment(&mut self, tok: Tok, lexer: &mut Lexer<'a>) {
        let Some(found) = directive(self.text(tok)) else { return };
        match found {
            Directive::Open { keep_if_variable, symbol } => {
                if symbol != Some("VARIABLE") {
                    self.error(tok.range(), "`#ifdef` and `#ifndef` take only the symbol VARIABLE");
                } else if self.open_if.is_some() {
                    self.error(tok.range(), "`#ifdef` and `#ifndef` blocks cannot be nested");
                } else if keep_if_variable == self.variable {
                    self.open_if = Some(self.stack.len());
                } else {
                    self.remove_block(tok, lexer);
                }
            }
            Directive::Else => self.error(
                tok.range(),
                "`#else` is not supported: end the block with `#endif` and write the other branch in its own `#ifdef VARIABLE` or `#ifndef VARIABLE` block",
            ),
            Directive::Endif => {
                if self.open_if.take().is_none() {
                    self.error(tok.range(), "`#endif` without an opening `#ifdef VARIABLE` or `#ifndef VARIABLE`");
                }
            }
        }
    }

    /// Drops a block that does not apply to this target: from its directive through `#endif`,
    /// or up to the brace that closes the enclosing block, or to the end of the code.
    fn remove_block(&mut self, open: Tok, lexer: &mut Lexer<'a>) {
        let mut scan = lexer.clone();
        let mut depth = 0usize;
        let mut end = self.code.len();
        while let Some(tok) = scan.next_tok() {
            match tok.kind {
                Kind::Comment => match directive(self.text(tok)) {
                    Some(Directive::Endif) => {
                        end = tok.end;
                        break;
                    }
                    Some(Directive::Open { .. }) => self.error(tok.range(), "`#ifdef` and `#ifndef` blocks cannot be nested"),
                    Some(Directive::Else) => self.error(tok.range(), "`#else` is not supported"),
                    None => {}
                },
                Kind::Punct(b'{') => depth = depth.saturating_add(1),
                Kind::Punct(b'}') => {
                    if depth == 0 {
                        end = tok.start;
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        self.out.copy_to(open.start);
        self.out.skip_to(end);
        lexer.reset(end);
    }

    fn significant(&mut self, tok: Tok, lexer: &Lexer<'a>) {
        if self.rewrites.last().is_some_and(|(range, _)| range.start == tok.start) {
            if let Some((range, with)) = self.rewrites.pop() {
                self.skip_until = range.end;
                self.replace(range, &with);
            }
            return;
        }
        if self.tail != Tail::None && !tok.is(b';') {
            if self.tail == Tail::Name && tok.kind == Kind::Word {
                self.tail = Tail::Semi;
                return;
            }
            self.tail = Tail::None;
            if let Some(reopen) = self.reopen.take() {
                self.insert(tok.start, &reopen);
            }
        }
        match tok.kind {
            Kind::Punct(b';') => self.semicolon(tok),
            Kind::Punct(b'{') => self.open_brace(tok),
            Kind::Punct(b'}') => self.close_brace(tok, lexer),
            Kind::Punct(b'[') => {
                if self.push(Frame::Bracket, tok) {
                    self.note(tok);
                }
            }
            Kind::Punct(b']') => {
                if matches!(self.stack.last(), Some(Frame::Bracket)) {
                    self.stack.pop();
                }
                self.note(tok);
            }
            Kind::Word if self.stmt_start => self.statement(tok, lexer),
            _ => self.note(tok),
        }
    }

    fn push(&mut self, frame: Frame, tok: Tok) -> bool {
        if self.stack.len() >= MAX_NESTING_DEPTH {
            self.error(tok.range(), format!("braces and brackets are nested more than {MAX_NESTING_DEPTH} deep"));
            self.fatal = true;
            return false;
        }
        self.stack.push(frame);
        true
    }

    /// A token inside a statement.
    fn note(&mut self, tok: Tok) {
        self.stmt_start = false;
        let text = self.text(tok);
        if let Some((_, words)) = &mut self.recording {
            words.push(text);
        }
        if let Some(named) = &mut self.lookupflag {
            *named = true;
        }
        if tok.kind != Kind::Word {
            return;
        }
        match &mut self.pending {
            Pending::Feature { tag, use_extension } => {
                if tag.is_none() {
                    *tag = Some(text.to_string());
                } else if text == "useExtension" {
                    *use_extension = true;
                }
            }
            Pending::Lookup { name, .. } => {
                if name.is_none() {
                    *name = Some(text.to_string());
                }
            }
            Pending::None => {}
        }
    }

    /// The first word of a statement.
    fn statement(&mut self, tok: Tok, lexer: &Lexer<'a>) {
        self.stmt_start = false;
        let text = self.text(tok);
        let in_feature_block = self.feature.is_some() && matches!(self.stack.last(), Some(Frame::Feature));
        match text {
            "feature" => self.pending = Pending::Feature { tag: None, use_extension: false },
            "lookup" => self.pending = Pending::Lookup { name: None, split: std::mem::take(&mut self.split_next) },
            "standalone" => self.standalone(tok, lexer),
            "delete" | "del" => self.delete(tok, lexer),
            "lookupflag" => {
                self.lookupflag = Some(false);
                if self.feature.is_some() {
                    self.recording = Some((RecordKind::Lookupflag, vec![text]));
                }
            }
            "script" if in_feature_block => self.recording = Some((RecordKind::Script, vec![text])),
            "language" if in_feature_block => self.recording = Some((RecordKind::Language, vec![text])),
            "sub" | "substitute" => self.plan_singletons(lexer),
            name if name.starts_with('@') => self.class_definition(name, lexer),
            _ => {}
        }
    }

    fn standalone(&mut self, tok: Tok, lexer: &Lexer<'a>) {
        let mut peek = lexer.clone();
        let words: Vec<Option<Tok>> = (0..4).map(|_| peek.next_significant()).collect();
        let word = |i: usize| words.get(i).copied().flatten();
        let is_word = |t: Option<Tok>, w: &str| t.is_some_and(|t| t.kind == Kind::Word && self.text(t) == w);
        let block = word(1).is_some_and(|t| t.kind == Kind::Word)
            && (word(2).is_some_and(|t| t.is(b'{')) || (is_word(word(2), "useExtension") && word(3).is_some_and(|t| t.is(b'{'))));
        let Some(lookup) = word(0).filter(|t| is_word(Some(*t), "lookup") && block) else {
            self.error(tok.range(), "`standalone` must come before a lookup block: `standalone lookup name { … } name;`");
            return;
        };
        match (&self.feature, self.stack.last()) {
            (Some(feature), Some(Frame::Feature)) => {
                let close = format!("}} {};\n", feature.tag);
                self.replace(tok.start..lookup.start, &close);
                self.split_next = true;
            }
            (_, None) => self.replace(tok.start..lookup.start, ""),
            _ => self.error(tok.range(), "a standalone lookup belongs directly in a feature block, or outside every block"),
        }
        self.stmt_start = true;
    }

    fn delete(&mut self, tok: Tok, lexer: &Lexer<'a>) {
        let mut peek = lexer.clone();
        if peek.next_significant().is_none_or(|t| t.is(b';') || t.is(b'{') || t.is(b'}')) {
            self.error(tok.range(), "`delete` needs the glyph to remove: `delete glyph;` or `delete glyph' context;`");
            return;
        }
        self.replace(tok.range(), "sub");
        self.delete_pending = true;
    }

    /// Records `@Name = [glyph];`, so a multiple substitution can repeat the glyph.
    fn class_definition(&mut self, name: &'a str, lexer: &Lexer<'a>) {
        let mut peek = lexer.clone();
        if !peek.next_significant().is_some_and(|t| t.is(b'=')) {
            return;
        }
        let toks: Vec<Option<Tok>> = (0..4).map(|_| peek.next_significant()).collect();
        let one = match toks.as_slice() {
            [Some(open), Some(glyph), Some(close), Some(semi)] if open.is(b'[') && close.is(b']') && semi.is(b';') => {
                Some(self.text(*glyph)).filter(|g| glyph.kind == Kind::Word && single_glyph(g))
            }
            _ => None,
        };
        match one {
            Some(glyph) => self.singletons.insert(name, glyph),
            None => self.singletons.remove(name),
        };
    }

    /// In a one-to-many substitution, plans a rewrite of each one-member class after `by` to its
    /// glyph, which the compiler repeats for every member of the other classes.
    fn plan_singletons(&mut self, lexer: &Lexer<'a>) {
        enum Item {
            Other,
            Class(Tok),
            Group { range: Range<usize>, first: Option<Tok>, count: usize },
        }
        let mut peek = lexer.clone();
        let mut after_by = false;
        let mut depth = 0usize;
        let mut items: Vec<Item> = Vec::new();
        let mut group: Option<(usize, Option<Tok>, usize)> = None;
        while let Some(tok) = peek.next_significant() {
            if tok.is(b';') || tok.is(b'{') || tok.is(b'}') {
                break;
            }
            match tok.kind {
                Kind::Punct(b'[') => {
                    if depth == 0 {
                        group = Some((tok.start, None, 0));
                    }
                    depth = depth.saturating_add(1);
                }
                Kind::Punct(b']') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0
                        && after_by
                        && let Some((start, first, count)) = group.take()
                    {
                        items.push(Item::Group { range: start..tok.end, first, count });
                    }
                }
                _ if depth > 0 => {
                    if let Some((_, first, count)) = &mut group {
                        first.get_or_insert(tok);
                        *count = count.saturating_add(1);
                    }
                }
                Kind::Word if !after_by => after_by = self.text(tok) == "by",
                Kind::Word if self.text(tok).starts_with('@') => items.push(Item::Class(tok)),
                _ if after_by => items.push(Item::Other),
                _ => {}
            }
        }
        if items.len() < 2 {
            return;
        }
        let mut rewrites = Vec::new();
        for item in items {
            let (range, member) = match item {
                Item::Class(tok) => (tok.range(), Some(tok)),
                Item::Group { range, first, count: 1 } => (range, first),
                _ => continue,
            };
            let Some(member) = member.filter(|t| t.kind == Kind::Word) else { continue };
            let text = self.text(member);
            let glyph = if text.starts_with('@') { self.singletons.get(text).copied() } else { Some(text).filter(|g| single_glyph(g)) };
            if let Some(glyph) = glyph {
                rewrites.push((range, glyph.to_string()));
            }
        }
        rewrites.reverse();
        self.rewrites = rewrites;
    }

    fn end_statement(&mut self) {
        self.pending = Pending::None;
        self.stmt_start = true;
        self.delete_pending = false;
        self.lookupflag = None;
        self.recording = None;
        self.rewrites.clear();
    }

    fn semicolon(&mut self, tok: Tok) {
        if self.delete_pending {
            self.insert(tok.start, " by NULL");
        }
        if self.lookupflag == Some(false) {
            self.insert(tok.start, " 0");
            if let Some((_, words)) = &mut self.recording {
                words.push("0");
            }
        }
        if let Some((kind, words)) = self.recording.take() {
            self.record(kind, &words);
        }
        self.end_statement();
        if self.tail != Tail::None {
            self.tail = Tail::None;
            if let Some(reopen) = self.reopen.take() {
                self.insert(tok.end, &reopen);
            }
        }
    }

    fn record(&mut self, kind: RecordKind, words: &[&str]) {
        let Some(feature) = &mut self.feature else { return };
        let statement = format!("{};", words.join(" "));
        let tag = words.get(1).copied().unwrap_or_default().to_string();
        match kind {
            RecordKind::Script => {
                // fea-rs ignores a script statement that names the language system already in
                // effect; any other one resets the language and the lookup flags.
                let current = feature.script.as_ref().is_some_and(|(t, _)| *t == tag) && feature.language.as_ref().is_none_or(|(l, _)| l == "dflt");
                if !current {
                    feature.script = Some((tag, statement));
                    feature.language = None;
                    feature.lookupflag = None;
                }
            }
            RecordKind::Language => feature.language = Some((tag, statement)),
            RecordKind::Lookupflag => feature.lookupflag = Some(statement),
        }
    }

    fn open_brace(&mut self, tok: Tok) {
        let frame = match std::mem::take(&mut self.pending) {
            Pending::Feature { tag: Some(tag), use_extension } => {
                self.feature = Some(FeatureState { tag, use_extension, ..FeatureState::default() });
                Frame::Feature
            }
            Pending::Lookup { name: Some(name), split } => {
                // Inside a feature block a lookup starts with the block's flags; at the top
                // level with none, so the split lookup states them.
                if split && let Some(flags) = self.feature.as_ref().and_then(|f| f.lookupflag.clone()) {
                    self.insert(tok.end, &format!(" {flags}"));
                }
                Frame::Lookup { name, reopen: split }
            }
            _ => Frame::Block,
        };
        self.push(frame, tok);
        self.end_statement();
    }

    fn close_brace(&mut self, tok: Tok, lexer: &Lexer<'a>) {
        while matches!(self.stack.last(), Some(Frame::Bracket)) {
            self.stack.pop();
        }
        match self.stack.pop() {
            Some(Frame::Feature) => {
                self.feature = None;
                self.tail = Tail::Name;
            }
            Some(Frame::Lookup { name, reopen }) => {
                let mut peek = lexer.clone();
                if peek.next_significant().is_some_and(|t| t.is(b';')) {
                    self.insert(tok.end, &format!(" {name}"));
                }
                if reopen {
                    self.reopen = Some(self.reopen_text());
                }
                self.tail = Tail::Name;
            }
            Some(Frame::Block) => self.tail = Tail::Name,
            Some(Frame::Bracket) | None => {}
        }
        if self.open_if.is_some_and(|depth| depth > self.stack.len()) {
            self.open_if = None;
        }
        self.end_statement();
    }

    fn reopen_text(&self) -> String {
        let Some(feature) = &self.feature else { return String::new() };
        let mut text = format!("\nfeature {}{} {{", feature.tag, if feature.use_extension { " useExtension" } else { "" });
        let script = feature.script.as_ref().map(|(_, s)| s);
        let language = feature.language.as_ref().map(|(_, s)| s);
        for statement in [script, language, feature.lookupflag.as_ref()].into_iter().flatten() {
            text.push(' ');
            text.push_str(statement);
        }
        text
    }
}

/// A glyph name that cannot be a range (`a-z`) or a class.
fn single_glyph(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('@') && !name.contains('-')
}

// ---------------------------------------------------------------------------------------------
// Several languages in one statement

/// Words that may follow the tags of a `language` statement.
const LANGUAGE_OPTIONS: &[&str] = &["exclude_dflt", "include_dflt", "excludeDFLT", "includeDFLT", "required"];

/// A `language` statement with several tags in a feature block, and the rules after it.
struct LanguageSection {
    statement: Range<usize>,
    tags: Vec<String>,
    options: String,
    /// From the end of the statement.
    start: usize,
    /// Named lookup blocks in the section: (range, name).
    lookups: Vec<(Range<usize>, String)>,
}

/// The second pass over already lowered text: `language A B C;` and the statements that follow
/// it, up to the next `script` or `language` statement or the end of the feature block, become
/// `language A;` with the statements, then the same for B and C. A named lookup block is
/// written once; the repeats refer to it by name. `None` when the text has no such statement.
fn split_languages(text: &str) -> Result<Option<(String, OffsetMap)>, Vec<Diagnostic>> {
    let mut out = Out::new(text);
    let mut lexer = Lexer::new(text);
    // For each open brace: whether it opened a feature block.
    let mut stack: Vec<bool> = Vec::new();
    let mut stmt_start = true;
    let mut feature_pending = false;
    let mut section: Option<LanguageSection> = None;
    let mut grown = text.len();
    while let Some(tok) = lexer.next_significant() {
        let in_feature = stack.last() == Some(&true);
        match tok.kind {
            Kind::Punct(b';') => {
                stmt_start = true;
                feature_pending = false;
                continue;
            }
            Kind::Punct(b'{') => {
                if stack.len() >= MAX_NESTING_DEPTH {
                    return Err(vec![error(tok.range(), format!("blocks nest more than {MAX_NESTING_DEPTH} deep"))]);
                }
                stack.push(std::mem::take(&mut feature_pending));
                stmt_start = true;
                continue;
            }
            Kind::Punct(b'}') => {
                if stack.pop() == Some(true)
                    && let Some(open) = section.take()
                {
                    close_section(&mut out, text, open, tok.start, &mut grown)?;
                }
                stmt_start = true;
                continue;
            }
            _ => {}
        }
        let at_start = std::mem::replace(&mut stmt_start, false);
        if !at_start || tok.kind != Kind::Word {
            continue;
        }
        match lexer.text(tok) {
            "feature" => feature_pending = true,
            "script" | "language" if in_feature => {
                if let Some(open) = section.take() {
                    close_section(&mut out, text, open, tok.start, &mut grown)?;
                }
                if lexer.text(tok) == "script" {
                    continue;
                }
                let mut ahead = lexer.clone();
                let mut tags = Vec::new();
                let mut options = String::new();
                let end = loop {
                    let Some(next) = ahead.next_significant() else { break None };
                    match next.kind {
                        Kind::Punct(b';') => break Some(next.end),
                        Kind::Word if LANGUAGE_OPTIONS.contains(&ahead.text(next)) || !options.is_empty() => {
                            options.push(' ');
                            options.push_str(ahead.text(next));
                        }
                        Kind::Word => tags.push(ahead.text(next).to_string()),
                        _ => break None,
                    }
                };
                if let Some(end) = end
                    && tags.len() > 1
                {
                    section = Some(LanguageSection { statement: tok.start..end, tags, options, start: end, lookups: Vec::new() });
                }
            }
            "lookup" if in_feature => {
                // A named block: `lookup name [useExtension] {` … `} name;`.
                let Some(open) = section.as_mut() else { continue };
                let mut ahead = lexer.clone();
                let Some(name) = ahead.next_significant().filter(|t| t.kind == Kind::Word) else { continue };
                let mut brace = ahead.next_significant();
                if brace.is_some_and(|t| ahead.text(t) == "useExtension") {
                    brace = ahead.next_significant();
                }
                if !brace.is_some_and(|t| t.is(b'{')) {
                    continue;
                }
                // The block's end: its closing brace, the end name and `;`.
                let mut depth = 0usize;
                let mut scan = ahead.clone();
                let mut close = None;
                while let Some(t) = scan.next_significant() {
                    if t.is(b'{') {
                        depth += 1;
                    } else if t.is(b'}') {
                        if depth == 0 {
                            close = Some(t);
                            break;
                        }
                        depth -= 1;
                    }
                }
                let Some(close) = close else { continue };
                let mut end = close.end;
                let after_name = scan.next_significant();
                if let Some(t) = after_name.filter(|t| t.kind == Kind::Word) {
                    end = t.end;
                    if let Some(semi) = scan.next_significant().filter(|t| t.is(b';')) {
                        end = semi.end;
                    }
                }
                open.lookups.push((tok.start..end, lexer.text(name).to_string()));
            }
            _ => {}
        }
    }
    if let Some(open) = section.take() {
        close_section(&mut out, text, open, text.len(), &mut grown)?;
    }
    if !out.changed {
        return Ok(None);
    }
    Ok(Some(out.finish()))
}

/// Writes the first language in place of the statement and the repeats at `end`.
fn close_section(out: &mut Out<'_>, text: &str, section: LanguageSection, end: usize, grown: &mut usize) -> Result<(), Vec<Diagnostic>> {
    let rules = text.get(section.start..end).unwrap_or_default();
    let mut repeat = String::with_capacity(rules.len());
    let mut copied = section.start;
    for (range, name) in &section.lookups {
        repeat.push_str(text.get(copied..range.start).unwrap_or_default());
        repeat.push_str(&format!("lookup {name};"));
        copied = range.end;
    }
    repeat.push_str(text.get(copied..end).unwrap_or_default());
    let per_language = repeat.len().saturating_add(32);
    *grown = grown.saturating_add(per_language.saturating_mul(section.tags.len()));
    if *grown > MAX_LOWER_BYTES {
        return Err(vec![error(
            section.statement.clone(),
            format!("the feature code grows past {} MB when its languages are written out one by one", MAX_LOWER_BYTES >> 20),
        )]);
    }
    let mut tags = section.tags.iter();
    let first = tags.next().map(String::as_str).unwrap_or_default();
    out.replace(section.statement.clone(), &format!("language {first}{};", section.options));
    let mut inserted = String::new();
    for tag in tags {
        inserted.push_str(&format!("\nlanguage {tag}{};", section.options));
        inserted.push_str(&repeat);
    }
    out.replace(end..end, &inserted);
    Ok(())
}

#[cfg(test)]
#[path = "lower_tests.rs"]
mod tests;

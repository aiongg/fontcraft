//! Feature code without the glyphs a source subset removes (M12.74; `font.subset` on sources).
//!
//! The code is read as tokens, not compiled, so code that would not compile is pruned as far as
//! its tokens allow and otherwise left as it is. Only names of glyphs the subset removed count:
//! a name the font never had is left alone. What changes:
//! - a glyph class (`[a b c]`, a class definition, a class entry of the source) loses the
//!   removed glyphs; a range (`a.sc-z.sc`) with a removed member is written out as the members
//!   that stay;
//! - a single substitution from one class to another (`sub [a b]' by [a.sc b.sc];`) loses the
//!   same positions on both sides;
//! - a rule (substitution, positioning, `ignore`, `markClass`, ligature carets, `Attach`) that
//!   names a removed glyph outside a class, or a class left empty, is removed;
//! - a class definition left empty is removed, and later rules that use it go with it;
//! - in `GlyphClassDef` an emptied class becomes an empty slot.
//!
//! Glyph tokens (`$[…]`, `${…}`, `$name`) are expanded by the compiler against the glyphs that
//! stay, so they are left as they are. Block headers, lookup references and every other statement
//! stay. The work is one pass over the tokens; range expansion is capped by
//! [`MAX_RANGE_MEMBERS`] for the whole run.

use std::collections::HashSet;
use std::ops::Range;

/// Most glyph names ranges may expand to in one run; past it a range is left as it is, with a
/// warning.
pub(super) const MAX_RANGE_MEMBERS: usize = 1_000_000;

/// Statements whose glyph names are checked; any other statement is left as it is.
const RULES: [&str; 13] = [
    "sub",
    "substitute",
    "rsub",
    "reversesub",
    "pos",
    "position",
    "enum",
    "enumerate",
    "ignore",
    "markClass",
    "LigatureCaretByPos",
    "LigatureCaretByIndex",
    "Attach",
];

/// Words of a rule that are never glyph names.
const KEYWORDS: [&str; 11] = ["by", "from", "NULL", "mark", "base", "ligature", "ligComponent", "cursive", "anchor", "lookup", "useExtension"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A glyph name, keyword, number, tag or range; a leading `\` is part of the token.
    Word,
    /// `@name`.
    Class,
    /// `$…`: a glyph token the compiler expands.
    Token,
    /// A quoted string.
    Str,
    /// One of `; { } [ ] ( ) < > ' , =`.
    Punct(u8),
}

#[derive(Clone, Copy, Debug)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

/// One member of a glyph class as written.
#[derive(Clone, Debug)]
enum Member {
    /// A glyph name of the font (without a leading `\`).
    Name(String),
    /// A range of glyph names: the names it expands to.
    Range(Vec<String>),
    /// `@name`.
    Class(String),
    /// Anything else, as written: a glyph token, a name the font does not have, a range that
    /// cannot be expanded. Always kept.
    Other(String),
}

/// A class as written and as it is after pruning.
struct Pruned {
    /// The member texts that stay, in order.
    kept: Vec<String>,
    /// Members before pruning (a range counts as one).
    before: usize,
    changed: bool,
}

impl Pruned {
    fn emptied(&self) -> bool {
        self.before > 0 && self.kept.is_empty()
    }
}

/// Prunes feature code for one subset: the classes it empties are remembered from one piece of
/// code to the next, in the order the pieces are given.
pub(super) struct Pruner<'a> {
    /// Every glyph name of the source before the subset.
    all: &'a HashSet<String>,
    /// The names the subset removed.
    removed: &'a HashSet<String>,
    /// Classes (glyph and mark classes) left empty.
    empty: HashSet<String>,
    /// Mark classes some `markClass` statement still fills.
    filled: HashSet<String>,
    range_budget: usize,
    pub(super) rules_removed: usize,
    pub(super) classes_removed: Vec<String>,
    pub(super) warnings: Vec<String>,
}

impl<'a> Pruner<'a> {
    pub(super) fn new(all: &'a HashSet<String>, removed: &'a HashSet<String>) -> Self {
        Self {
            all,
            removed,
            empty: HashSet::new(),
            filled: HashSet::new(),
            range_budget: MAX_RANGE_MEMBERS,
            rules_removed: 0,
            classes_removed: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// The code of a class entry of the source (its members, without brackets): `None` when the
    /// subset empties it.
    pub(super) fn class_entry(&mut self, name: &str, code: &str) -> Option<String> {
        let toks = tokenize(code);
        let pruned = self.prune_members(code, &toks);
        if pruned.emptied() {
            self.empty.insert(name.to_string());
            self.classes_removed.push(name.to_string());
            return None;
        }
        Some(if pruned.changed { pruned.kept.join(" ") } else { code.to_string() })
    }

    /// `code` without what names removed glyphs.
    pub(super) fn code(&mut self, code: &str) -> String {
        let toks = tokenize(code);
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut start = 0;
        for (i, t) in toks.iter().enumerate() {
            match t.kind {
                Kind::Punct(b';') => {
                    if let Some(stmt) = toks.get(start..=i) {
                        edits.extend(self.statement(code, stmt));
                    }
                    start = i + 1;
                }
                Kind::Punct(b'{' | b'}') => start = i + 1,
                _ => {}
            }
        }
        if edits.is_empty() {
            return code.to_string();
        }
        let mut out = String::with_capacity(code.len());
        let mut at = 0;
        for (range, text) in edits {
            out.push_str(code.get(at..range.start).unwrap_or_default());
            out.push_str(&text);
            at = range.end;
        }
        out.push_str(code.get(at..).unwrap_or_default());
        out
    }

    /// The edits one statement needs, ending in its `;`.
    fn statement(&mut self, code: &str, stmt: &[Tok]) -> Vec<(Range<usize>, String)> {
        let (Some(first), Some(last)) = (stmt.first(), stmt.last()) else { return Vec::new() };
        let head = text(code, first);
        match first.kind {
            Kind::Class if stmt.get(1).is_some_and(|t| t.kind == Kind::Punct(b'=')) => self.class_definition(code, stmt, first, last),
            Kind::Word if head == "GlyphClassDef" => self.glyph_class_def(code, stmt),
            Kind::Word if RULES.contains(&head) => self.rule(code, stmt, head == "markClass"),
            _ => Vec::new(),
        }
    }

    /// `@name = [ … ];` or `@name = @other;`.
    fn class_definition(&mut self, code: &str, stmt: &[Tok], first: &Tok, last: &Tok) -> Vec<(Range<usize>, String)> {
        let name = text(code, first).trim_start_matches('@').to_string();
        let body = stmt.get(2..stmt.len().saturating_sub(1)).unwrap_or_default();
        let (pruned, literal) = match body.first().map(|t| t.kind) {
            Some(Kind::Punct(b'[')) => match close_of(body, 0) {
                Some(close) => (self.prune_members(code, body.get(1..close).unwrap_or_default()), body.first().zip(body.get(close))),
                None => return Vec::new(),
            },
            _ => (self.prune_members(code, body), None),
        };
        if pruned.emptied() {
            self.empty.insert(name.clone());
            self.classes_removed.push(name);
            return vec![removal(code, first.start, last.end)];
        }
        self.empty.remove(&name);
        match literal {
            Some((open, close)) if pruned.changed => vec![(open.start..close.end, format!("[{}]", pruned.kept.join(" ")))],
            _ => Vec::new(),
        }
    }

    /// `GlyphClassDef A, B, C, D;`: an emptied class becomes an empty slot.
    fn glyph_class_def(&mut self, code: &str, stmt: &[Tok]) -> Vec<(Range<usize>, String)> {
        let mut edits = Vec::new();
        let mut i = 1;
        while let Some(t) = stmt.get(i) {
            match t.kind {
                Kind::Punct(b'[') => {
                    let Some(close) = close_of(stmt, i) else { return edits };
                    let pruned = self.prune_members(code, stmt.get(i + 1..close).unwrap_or_default());
                    if let Some(c) = stmt.get(close)
                        && pruned.changed
                    {
                        let replacement = if pruned.kept.is_empty() { String::new() } else { format!("[{}]", pruned.kept.join(" ")) };
                        edits.push((t.start..c.end, replacement));
                    }
                    i = close + 1;
                }
                Kind::Class => {
                    if self.empty.contains(text(code, t).trim_start_matches('@')) {
                        edits.push((t.start..t.end, String::new()));
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        edits
    }

    /// A rule: removed when it names a removed glyph outside a class or a class left empty;
    /// otherwise its classes lose the removed glyphs.
    fn rule(&mut self, code: &str, stmt: &[Tok], mark_class: bool) -> Vec<(Range<usize>, String)> {
        let (Some(first), Some(last)) = (stmt.first(), stmt.last()) else { return Vec::new() };
        // The class a `markClass` statement defines is its last class name, not a use.
        let defines = if mark_class { stmt.iter().rposition(|t| t.kind == Kind::Class) } else { None };
        let mut literals: Vec<Literal> = Vec::new();
        let mut remove_rule = false;
        let mut nesting = 0usize;
        let mut after_lookup = false;
        let mut after_by = false;
        let mut i = 1;
        while let Some(t) = stmt.get(i) {
            let word = text(code, t);
            match t.kind {
                Kind::Punct(b'<' | b'(') => nesting += 1,
                Kind::Punct(b'>' | b')') => nesting = nesting.saturating_sub(1),
                _ if nesting > 0 => {}
                Kind::Punct(b'[') => {
                    let Some(close) = close_of(stmt, i) else { return Vec::new() };
                    let members = self.members(code, stmt.get(i + 1..close).unwrap_or_default());
                    let marked = stmt.get(close + 1).is_some_and(|n| n.kind == Kind::Punct(b'\''));
                    if let (Some(open), Some(c)) = (stmt.get(i), stmt.get(close)) {
                        literals.push(Literal { span: open.start..c.end, members, marked, after_by });
                    }
                    i = close + 1;
                    continue;
                }
                Kind::Class if Some(i) != defines => remove_rule |= self.empty.contains(word.trim_start_matches('@')),
                Kind::Word if after_lookup => after_lookup = false,
                Kind::Word if word == "lookup" => after_lookup = true,
                Kind::Word if word == "by" => after_by = true,
                Kind::Word if !KEYWORDS.contains(&word) => remove_rule |= self.removed.contains(word.trim_start_matches('\\')),
                _ => {}
            }
            i += 1;
        }

        let mut edits = Vec::new();
        if !remove_rule {
            let head = text(code, first);
            let paired = if matches!(head, "sub" | "substitute" | "rsub" | "reversesub") { pair(&literals) } else { None };
            let mut outcomes: Vec<Pruned> = literals.iter().map(|l| self.prune(&l.members)).collect();
            if let Some((a, b)) = paired
                && let (Some(la), Some(lb)) = (literals.get(a), literals.get(b))
            {
                let gone: Vec<bool> = la.members.iter().zip(&lb.members).map(|(x, y)| self.member_gone(x) || self.member_gone(y)).collect();
                for (index, literal) in [(a, la), (b, lb)] {
                    let kept: Vec<String> = literal.members.iter().zip(&gone).filter(|(_, g)| !**g).map(|(m, _)| member_text(m)).collect();
                    let changed = gone.iter().any(|g| *g);
                    if let Some(slot) = outcomes.get_mut(index) {
                        *slot = Pruned { kept, before: literal.members.len(), changed };
                    }
                }
            }
            remove_rule = outcomes.iter().any(Pruned::emptied);
            if !remove_rule {
                for (literal, outcome) in literals.iter().zip(&outcomes) {
                    if outcome.changed {
                        edits.push((literal.span.clone(), format!("[{}]", outcome.kept.join(" "))));
                    }
                }
            }
        }
        if let Some(name) = defines.and_then(|d| stmt.get(d)).map(|t| text(code, t).trim_start_matches('@').to_string()) {
            if remove_rule {
                if !self.filled.contains(&name) {
                    self.empty.insert(name);
                }
            } else {
                self.empty.remove(&name);
                self.filled.insert(name);
            }
        }
        if remove_rule {
            self.rules_removed += 1;
            return vec![removal(code, first.start, last.end)];
        }
        edits
    }

    /// The members of a class body.
    fn members(&mut self, code: &str, toks: &[Tok]) -> Vec<Member> {
        let mut out: Vec<Member> = Vec::with_capacity(toks.len());
        let mut i = 0;
        while let Some(t) = toks.get(i) {
            let word = text(code, t);
            let member = match t.kind {
                Kind::Class => Member::Class(word.trim_start_matches('@').to_string()),
                Kind::Word => {
                    let name = word.trim_start_matches('\\');
                    // `a - z`: a range written with spaces.
                    if let (Some(dash), Some(end)) = (toks.get(i + 1), toks.get(i + 2))
                        && dash.kind == Kind::Word
                        && text(code, dash) == "-"
                        && end.kind == Kind::Word
                    {
                        i += 3;
                        out.push(self.range(name, text(code, end).trim_start_matches('\\'), &format!("{word} - {}", text(code, end))));
                        continue;
                    }
                    if self.all.contains(name) {
                        Member::Name(name.to_string())
                    } else {
                        match self.split_range(name) {
                            Some((a, b)) => self.range(&a, &b, word),
                            None => Member::Other(word.to_string()),
                        }
                    }
                }
                _ => Member::Other(word.to_string()),
            };
            out.push(member);
            i += 1;
        }
        out
    }

    /// The range from `a` to `b`, written `written`.
    fn range(&mut self, a: &str, b: &str, written: &str) -> Member {
        match expand_range(a, b) {
            Some(names) if names.len() <= self.range_budget => {
                self.range_budget -= names.len();
                Member::Range(names)
            }
            Some(_) => {
                if self.warnings.is_empty() {
                    self.warnings.push(format!(
                        "feature code: ranges expand to more than {MAX_RANGE_MEMBERS} glyph names; `{written}` and later ranges are left as they are"
                    ));
                }
                Member::Other(written.to_string())
            }
            None => Member::Other(written.to_string()),
        }
    }

    /// A word that is not a glyph name of the font split at a hyphen into two names that are.
    fn split_range(&self, word: &str) -> Option<(String, String)> {
        word.match_indices('-').find_map(|(at, _)| {
            let (a, b) = (word.get(..at)?, word.get(at + 1..)?);
            (self.all.contains(a) && self.all.contains(b)).then(|| (a.to_string(), b.to_string()))
        })
    }

    /// The members of a class body after pruning.
    fn prune_members(&mut self, code: &str, toks: &[Tok]) -> Pruned {
        let members = self.members(code, toks);
        self.prune(&members)
    }

    fn prune(&self, members: &[Member]) -> Pruned {
        let mut kept = Vec::with_capacity(members.len());
        let mut changed = false;
        for m in members {
            match m {
                Member::Range(names) if names.iter().any(|n| self.removed.contains(n)) => {
                    changed = true;
                    kept.extend(names.iter().filter(|n| !self.removed.contains(*n)).cloned());
                }
                m if self.member_gone(m) => changed = true,
                m => kept.push(member_text(m)),
            }
        }
        Pruned { kept, before: members.len(), changed }
    }

    /// Whether a member goes as a whole: a removed glyph or an emptied class.
    fn member_gone(&self, m: &Member) -> bool {
        match m {
            Member::Name(n) => self.removed.contains(n),
            Member::Class(c) => self.empty.contains(c),
            Member::Range(names) => names.iter().all(|n| self.removed.contains(n)),
            Member::Other(_) => false,
        }
    }
}

/// A bracketed class in a rule.
struct Literal {
    span: Range<usize>,
    members: Vec<Member>,
    /// Followed by `'`.
    marked: bool,
    /// After the rule's `by`.
    after_by: bool,
}

/// The input and output classes of a single substitution by class: one input class (the marked
/// one, when the rule marks any) and one output class of the same length, both of plain names.
fn pair(literals: &[Literal]) -> Option<(usize, usize)> {
    let before: Vec<usize> = (0..literals.len()).filter(|i| literals.get(*i).is_some_and(|l| !l.after_by)).collect();
    let marked: Vec<usize> = before.iter().copied().filter(|i| literals.get(*i).is_some_and(|l| l.marked)).collect();
    let inputs = if marked.is_empty() { before } else { marked };
    let outputs: Vec<usize> = (0..literals.len()).filter(|i| literals.get(*i).is_some_and(|l| l.after_by)).collect();
    let (&[a], &[b]) = (inputs.as_slice(), outputs.as_slice()) else { return None };
    let (la, lb) = (literals.get(a)?, literals.get(b)?);
    let plain = |l: &Literal| l.members.iter().all(|m| matches!(m, Member::Name(_)));
    (la.members.len() == lb.members.len() && plain(la) && plain(lb)).then_some((a, b))
}

fn member_text(m: &Member) -> String {
    match m {
        Member::Name(n) => name_text(n),
        Member::Range(names) => names.iter().map(|n| name_text(n)).collect::<Vec<_>>().join(" "),
        Member::Class(c) => format!("@{c}"),
        Member::Other(t) => t.clone(),
    }
}

/// A glyph name as feature code writes it: escaped when it would read as a keyword.
fn name_text(n: &str) -> String {
    if KEYWORDS.contains(&n) || RULES.contains(&n) { format!("\\{n}") } else { n.to_string() }
}

/// The names a range from `a` to `b` stands for, as the feature file specification defines it:
/// the two names differ in one place only, a single letter of the same case or a run of up to
/// three digits of the same length. `None` when they do not.
fn expand_range(a: &str, b: &str) -> Option<Vec<String>> {
    let prefix = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let rest_a = a.get(prefix..)?;
    let rest_b = b.get(prefix..)?;
    let suffix = rest_a.bytes().rev().zip(rest_b.bytes().rev()).take_while(|(x, y)| x == y).count();
    let mid_a = rest_a.get(..rest_a.len().checked_sub(suffix)?)?;
    let mid_b = rest_b.get(..rest_b.len().checked_sub(suffix)?)?;
    let (head, tail) = (a.get(..prefix)?, rest_a.get(rest_a.len().checked_sub(suffix)?..)?);
    let letter = |s: &str| s.len() == 1 && s.bytes().all(|c| c.is_ascii_alphabetic());
    if letter(mid_a) && letter(mid_b) {
        let (x, y) = (mid_a.bytes().next()?, mid_b.bytes().next()?);
        if x.is_ascii_uppercase() != y.is_ascii_uppercase() || x > y {
            return None;
        }
        return Some((x..=y).map(|c| format!("{head}{}{tail}", char::from(c))).collect());
    }
    let digits = |s: &str| (1..=3).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit());
    if digits(mid_a) && digits(mid_b) && mid_a.len() == mid_b.len() {
        let (x, y) = (mid_a.parse::<u32>().ok()?, mid_b.parse::<u32>().ok()?);
        if x > y {
            return None;
        }
        let width = mid_a.len();
        return Some((x..=y).map(|n| format!("{head}{n:0width$}{tail}")).collect());
    }
    None
}

/// The byte range a removed statement takes: from `start` to `end`, widened to whole lines when
/// nothing else is on them.
fn removal(code: &str, start: usize, end: usize) -> (Range<usize>, String) {
    let before = code.get(..start).unwrap_or_default();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let lead_blank = before.get(line_start..).is_some_and(|s| s.bytes().all(|b| b == b' ' || b == b'\t'));
    if !lead_blank {
        return (start..end, String::new());
    }
    let after = code.get(end..).unwrap_or_default();
    let line_end = after.find('\n').map_or(code.len(), |i| end + i + 1);
    let trail_blank = code.get(end..line_end).is_some_and(|s| s.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')));
    if trail_blank { (line_start..line_end, String::new()) } else { (start..end, String::new()) }
}

/// The index of the `]` that closes the `[` at `open`.
fn close_of(toks: &[Tok], open: usize) -> Option<usize> {
    toks.iter().skip(open + 1).position(|t| t.kind == Kind::Punct(b']')).map(|p| open + 1 + p)
}

fn text<'c>(code: &'c str, t: &Tok) -> &'c str {
    code.get(t.start..t.end).unwrap_or_default()
}

/// Bytes that end a word.
fn delimiter(b: u8) -> bool {
    b.is_ascii_whitespace() || b"#\";{}[]()<>',=@$".contains(&b)
}

/// The tokens of `code`; spaces and comments are left out.
fn tokenize(code: &str) -> Vec<Tok> {
    let bytes = code.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let start = i;
        let kind = match b {
            b if b.is_ascii_whitespace() => {
                i += 1;
                continue;
            }
            b'#' => {
                i = bytes.iter().skip(i).position(|&c| c == b'\n').map_or(bytes.len(), |p| i + p);
                continue;
            }
            b'"' => {
                i = bytes.iter().skip(i + 1).position(|&c| c == b'"').map_or(bytes.len(), |p| i + 1 + p + 1);
                Kind::Str
            }
            b'$' => {
                i += 1;
                match bytes.get(i) {
                    Some(&open @ (b'[' | b'{')) => {
                        let close = if open == b'[' { b']' } else { b'}' };
                        let mut depth = 0usize;
                        while let Some(&c) = bytes.get(i) {
                            i += 1;
                            if c == open {
                                depth += 1;
                            } else if c == close {
                                depth = depth.saturating_sub(1);
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                    }
                    _ => i = word_end(bytes, i),
                }
                Kind::Token
            }
            b'@' => {
                i = word_end(bytes, i + 1);
                Kind::Class
            }
            b';' | b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'<' | b'>' | b'\'' | b',' | b'=' => {
                i += 1;
                Kind::Punct(b)
            }
            _ => {
                i = word_end(bytes, i + 1);
                Kind::Word
            }
        };
        toks.push(Tok { kind, start, end: i.min(bytes.len()) });
    }
    toks
}

/// The end of the word that runs from `i`. Every delimiter is ASCII, so the end is on a
/// character boundary.
fn word_end(bytes: &[u8], i: usize) -> usize {
    bytes.iter().skip(i).position(|&c| delimiter(c)).map_or(bytes.len(), |p| i + p)
}

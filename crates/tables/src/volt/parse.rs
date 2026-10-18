//! The VOLT project reader: lexer, parser and the parsed statements.
//!
//! Ported from fontTools `voltLib.lexer`, `voltLib.parser` and `voltLib.ast` (MIT, Copyright
//! (c) 2017 Just van Rossum and others; see `NOTICE`). The reader accepts what voltLib
//! accepts and refuses with the same messages; where voltLib would stop with an internal
//! error (an assertion, a missing key), this reader returns an error that names the place.

use crate::{Result, TablesError};

/// Deepest nesting of `ENUM` inside `ENUM`.
pub(super) const MAX_NESTING: usize = 64;

/// A place in the project text: line and column, both from 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Loc {
    pub line: usize,
    pub column: usize,
}

pub(super) fn error_at(loc: Loc, message: impl std::fmt::Display) -> TablesError {
    TablesError::Invalid(format!("<volt>:{}:{}: {message}", loc.line, loc.column))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Name(String),
    Str(String),
    Num(i64),
}

impl Tok {
    /// voltLib compares the next token's value with a keyword whatever its kind, so a string
    /// that spells a keyword matches it too.
    fn is(&self, word: &str) -> bool {
        match self {
            Tok::Name(s) | Tok::Str(s) => s == word,
            Tok::Num(_) => false,
        }
    }
}

struct Lexer<'a> {
    text: &'a [u8],
    pos: usize,
    line: usize,
    line_start: usize,
}

const fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'.' || b == b'_'
}

const fn is_name_continue(b: u8) -> bool {
    is_name_start(b) || b.is_ascii_digit()
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        Self { text: text.as_bytes(), pos: 0, line: 1, line_start: 0 }
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.text.get(at).copied()
    }

    fn loc(&self) -> Loc {
        let column = std::str::from_utf8(self.text.get(self.line_start..self.pos).unwrap_or_default()).map_or(0, |s| s.chars().count());
        Loc { line: self.line, column: column + 1 }
    }

    fn slice(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(self.text.get(start..end).unwrap_or_default()).into_owned()
    }

    /// The next token other than a line break, or `None` at the end of the text.
    fn next(&mut self) -> Result<Option<(Tok, Loc)>> {
        loop {
            while matches!(self.byte(self.pos), Some(b' ' | b'\t')) {
                self.pos += 1;
            }
            let loc = self.loc();
            let start = self.pos;
            let Some(cur) = self.byte(start) else { return Ok(None) };
            let next = self.byte(start + 1);
            match cur {
                b'\n' => {
                    self.pos += 1;
                    self.line += 1;
                    self.line_start = self.pos;
                }
                b'\r' => {
                    self.pos += if next == Some(b'\n') { 2 } else { 1 };
                    self.line += 1;
                    self.line_start = self.pos;
                }
                b'"' => {
                    self.pos += 1;
                    while self.byte(self.pos).is_some_and(|b| !matches!(b, b'"' | b'\r' | b'\n')) {
                        self.pos += 1;
                    }
                    if self.byte(self.pos) == Some(b'"') {
                        self.pos += 1;
                        return Ok(Some((Tok::Str(self.slice(start + 1, self.pos - 1)), loc)));
                    }
                    return Err(error_at(loc, "Expected '\"' to terminate string"));
                }
                b if is_name_start(b) => {
                    self.pos += 1;
                    while self.byte(self.pos).is_some_and(is_name_continue) {
                        self.pos += 1;
                    }
                    return Ok(Some((Tok::Name(self.slice(start, self.pos)), loc)));
                }
                b if b.is_ascii_digit() || (b == b'-' && next.is_some_and(|n| n.is_ascii_digit())) => {
                    self.pos += 1;
                    while self.byte(self.pos).is_some_and(|b| b.is_ascii_digit()) {
                        self.pos += 1;
                    }
                    let text = self.slice(start, self.pos);
                    let value = text.parse::<i64>().map_err(|_| error_at(loc, format!("Number out of range: {text}")))?;
                    return Ok(Some((Tok::Num(value), loc)));
                }
                _ => {
                    let c = std::str::from_utf8(self.text.get(start..).unwrap_or_default())
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or(char::REPLACEMENT_CHARACTER);
                    return Err(error_at(loc, format!("Unexpected character: '{c}'")));
                }
            }
        }
    }
}

// ── the parsed project ─────────────────────────────────────────────────────────────────────────

/// One element of a glyph coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Glyph(String),
    Group(String, Loc),
    Range(String, String, Loc),
    Enum(Vec<Item>),
}

/// A coverage: the elements listed one after the other.
pub(super) type Coverage = Vec<Item>;

/// A `POS` value: advance, x and y placement, each with its device adjustments (size,
/// adjustment) in the order first given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Pos {
    pub adv: Option<i64>,
    pub dx: Option<i64>,
    pub dy: Option<i64>,
    pub adv_adjust: Vec<(i64, i64)>,
    pub dx_adjust: Vec<(i64, i64)>,
    pub dy_adjust: Vec<(i64, i64)>,
}

#[derive(Clone, Debug)]
pub(super) struct GlyphDef {
    pub name: String,
    pub id: i64,
    pub gtype: Option<String>,
    pub components: Option<i64>,
}

#[derive(Clone, Debug)]
pub(super) struct GroupDef {
    pub name: String,
    pub items: Option<Coverage>,
}

#[derive(Clone, Debug)]
pub(super) struct FeatureDef {
    pub tag: String,
    pub lookups: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct LangSysDef {
    pub tag: String,
    pub features: Vec<FeatureDef>,
}

#[derive(Clone, Debug)]
pub(super) struct ScriptDef {
    pub tag: String,
    pub langs: Vec<LangSysDef>,
    pub loc: Loc,
}

/// How a lookup treats marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProcessMarks {
    All,
    None,
    /// Only the marks of this group (mark attachment type).
    Class(String),
}

#[derive(Clone, Debug)]
pub(super) struct Context {
    pub except: bool,
    pub left: Vec<Coverage>,
    pub right: Vec<Coverage>,
}

/// The substitution as written: pairs of source and target coverages.
#[derive(Clone, Debug)]
pub(super) struct Substitution {
    pub pairs: Vec<(Coverage, Coverage)>,
    /// Longest source and target coverage, counted in elements.
    pub max_src: usize,
    pub max_dest: usize,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub(super) enum Position {
    Attach { coverage: Coverage, to: Vec<(Coverage, String)> },
    Cursive { exit: Vec<Coverage>, enter: Vec<Coverage> },
    Pair { first: Vec<Coverage>, second: Vec<Coverage>, pairs: Vec<((i64, i64), (Pos, Pos))> },
    Single(Vec<(Coverage, Pos)>),
}

#[derive(Clone, Debug)]
pub(super) enum Body {
    Sub(Substitution),
    Pos(Position),
}

#[derive(Clone, Debug)]
pub(super) struct LookupDef {
    pub name: String,
    pub process_base: bool,
    pub process_marks: ProcessMarks,
    pub mark_glyph_set: Option<String>,
    pub direction: Option<String>,
    pub reversal: bool,
    pub comments: Option<String>,
    pub context: Vec<Context>,
    pub body: Body,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub(super) struct AnchorDef {
    pub name: String,
    pub glyph: String,
    pub component: i64,
    pub pos: Pos,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub(super) enum Statement {
    Glyph(GlyphDef),
    Group(GroupDef),
    Script(ScriptDef),
    Lookup(LookupDef),
    Anchor(AnchorDef),
    Setting(String),
}

const PARSE_KEYWORDS: [&str; 12] = [
    "CMAP_FORMAT",
    "COMPILER_USEEXTENSIONLOOKUPS",
    "COMPILER_USEPAIRPOSFORMAT2",
    "DEF_ANCHOR",
    "DEF_GLYPH",
    "DEF_GROUP",
    "DEF_LOOKUP",
    "DEF_SCRIPT",
    "DO_NOT_TOUCH_CMAP",
    "GRID_PPEM",
    "PPOSITIONING_PPEM",
    "PRESENTATION_PPEM",
];

// ── the parser ─────────────────────────────────────────────────────────────────────────────────

struct Parser<'a> {
    lexer: Lexer<'a>,
    cur: Option<(Tok, Loc)>,
    next: Option<(Tok, Loc)>,
    cur_loc: Loc,
    groups: Vec<String>,
    scripts: Vec<String>,
    lookups: Vec<String>,
    anchors: std::collections::HashMap<String, Vec<(String, i64)>>,
}

/// Parses a VOLT project into its statements, in the order written.
pub(super) fn parse(text: &str) -> Result<Vec<Statement>> {
    let mut p = Parser {
        lexer: Lexer::new(text),
        cur: None,
        next: None,
        cur_loc: Loc::default(),
        groups: Vec::new(),
        scripts: Vec::new(),
        lookups: Vec::new(),
        anchors: std::collections::HashMap::new(),
    };
    p.advance()?;
    let mut statements = Vec::new();
    while p.next.is_some() {
        p.advance()?;
        let Some(Tok::Name(word)) = p.cur.as_ref().map(|c| &c.0) else {
            return Err(p.err(format!("Expected {}", PARSE_KEYWORDS.join(", "))));
        };
        let word = word.clone();
        let statement = match word.as_str() {
            "DEF_GLYPH" => Statement::Glyph(p.def_glyph()?),
            "DEF_GROUP" => Statement::Group(p.def_group()?),
            "DEF_SCRIPT" => Statement::Script(p.def_script()?),
            "DEF_LOOKUP" => Statement::Lookup(p.def_lookup()?),
            "DEF_ANCHOR" => Statement::Anchor(p.def_anchor()?),
            "GRID_PPEM" | "PRESENTATION_PPEM" | "PPOSITIONING_PPEM" => {
                p.expect_number()?;
                Statement::Setting(word)
            }
            "COMPILER_USEEXTENSIONLOOKUPS" | "COMPILER_USEPAIRPOSFORMAT2" | "DO_NOT_TOUCH_CMAP" => Statement::Setting(word),
            "CMAP_FORMAT" => {
                for _ in 0..3 {
                    p.expect_number()?;
                }
                Statement::Setting(word)
            }
            "END" => break,
            _ => return Err(p.err(format!("Expected {}", PARSE_KEYWORDS.join(", ")))),
        };
        statements.push(statement);
    }
    Ok(statements)
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

impl Parser<'_> {
    fn err(&self, message: impl std::fmt::Display) -> TablesError {
        error_at(self.cur_loc, message)
    }

    fn advance(&mut self) -> Result<()> {
        self.cur = self.next.take();
        if let Some((_, loc)) = &self.cur {
            self.cur_loc = *loc;
        }
        if matches!(&self.cur, Some((Tok::Name(n), _)) if n == "END") {
            return Ok(());
        }
        self.next = self.lexer.next()?;
        Ok(())
    }

    fn next_is(&self, word: &str) -> bool {
        self.next.as_ref().is_some_and(|(t, _)| t.is(word))
    }

    fn next_is_any(&self, words: &[&str]) -> bool {
        words.iter().any(|w| self.next_is(w))
    }

    fn cur_is_keyword(&self, word: &str) -> bool {
        matches!(&self.cur, Some((Tok::Name(n), _)) if n == word)
    }

    fn expect_string(&mut self) -> Result<String> {
        self.advance()?;
        match &self.cur {
            Some((Tok::Str(s), _)) => Ok(s.clone()),
            _ => Err(self.err("Expected a string")),
        }
    }

    fn expect_keyword(&mut self, word: &str) -> Result<()> {
        self.advance()?;
        if self.cur_is_keyword(word) { Ok(()) } else { Err(self.err(format!("Expected \"{word}\""))) }
    }

    fn expect_name(&mut self) -> Result<String> {
        self.advance()?;
        match &self.cur {
            Some((Tok::Name(s), _)) => Ok(s.clone()),
            _ => Err(self.err("Expected a name")),
        }
    }

    fn expect_number(&mut self) -> Result<i64> {
        self.advance()?;
        match &self.cur {
            Some((Tok::Num(n), _)) => Ok(*n),
            _ => Err(self.err("Expected a number")),
        }
    }

    /// Fails at the end of the text, where a loop waiting for its closing keyword would
    /// otherwise never end.
    fn not_at_end(&self, closing: &str) -> Result<()> {
        if self.next.is_none() { Err(self.err(format!("Expected \"{closing}\" before the end of the project"))) } else { Ok(()) }
    }

    fn def_glyph(&mut self) -> Result<GlyphDef> {
        let name = self.expect_string()?;
        self.expect_keyword("ID")?;
        let id = self.expect_number()?;
        if id < 0 {
            return Err(self.err("Invalid glyph ID"));
        }
        if self.next_is("UNICODE") {
            self.expect_keyword("UNICODE")?;
            if self.expect_number()? < 0 {
                return Err(self.err("Invalid glyph UNICODE"));
            }
        } else if self.next_is("UNICODEVALUES") {
            self.expect_keyword("UNICODEVALUES")?;
            let values = self.expect_string()?;
            for value in values.split(',').filter(|v| !v.is_empty()) {
                let digits = value.get(2..).unwrap_or_default();
                if u32::from_str_radix(digits, 16).is_err() {
                    return Err(self.err(format!("invalid literal for int() with base 16: '{digits}'")));
                }
            }
        }
        let mut gtype = None;
        if self.next_is("TYPE") {
            self.expect_keyword("TYPE")?;
            let t = self.expect_name()?;
            if !matches!(t.as_str(), "BASE" | "LIGATURE" | "MARK" | "COMPONENT") {
                return Err(self.err(format!("Expected BASE, LIGATURE, MARK or COMPONENT, got {t}")));
            }
            gtype = Some(t);
        }
        let mut components = None;
        if self.next_is("COMPONENTS") {
            self.expect_keyword("COMPONENTS")?;
            components = Some(self.expect_number()?);
        }
        self.expect_keyword("END_GLYPH")?;
        Ok(GlyphDef { name, id, gtype, components })
    }

    fn def_group(&mut self) -> Result<GroupDef> {
        let loc = self.cur_loc;
        let name = self.expect_string()?;
        let mut items = None;
        if self.next_is("ENUM") {
            items = Some(self.parse_enum(0)?);
        }
        self.expect_keyword("END_GROUP")?;
        if self.groups.iter().any(|g| eq_ignore_case(g, &name)) {
            return Err(error_at(loc, format!("Glyph group \"{name}\" already defined, group names are case insensitive")));
        }
        self.groups.push(name.clone());
        Ok(GroupDef { name, items })
    }

    fn def_script(&mut self) -> Result<ScriptDef> {
        let loc = self.cur_loc;
        if self.next_is("NAME") {
            self.expect_keyword("NAME")?;
            self.expect_string()?;
        }
        self.expect_keyword("TAG")?;
        let tag = self.expect_string()?;
        if self.scripts.iter().any(|s| eq_ignore_case(s, &tag)) {
            return Err(error_at(loc, format!("Script \"{tag}\" already defined, script tags are case insensitive")));
        }
        let mut langs: Vec<LangSysDef> = Vec::new();
        while !self.next_is("END_SCRIPT") {
            self.not_at_end("END_SCRIPT")?;
            self.advance()?;
            let lang = self.langsys()?;
            self.expect_keyword("END_LANGSYS")?;
            if langs.iter().any(|l| eq_ignore_case(&l.tag, &lang.tag)) {
                return Err(error_at(
                    loc,
                    format!("Language \"{}\" already defined in script \"{tag}\", language tags are case insensitive", lang.tag),
                ));
            }
            langs.push(lang);
        }
        self.expect_keyword("END_SCRIPT")?;
        self.scripts.push(tag.clone());
        Ok(ScriptDef { tag, langs, loc })
    }

    fn langsys(&mut self) -> Result<LangSysDef> {
        if !self.cur_is_keyword("DEF_LANGSYS") {
            return Err(self.err("Expected \"DEF_LANGSYS\""));
        }
        if self.next_is("NAME") {
            self.expect_keyword("NAME")?;
            self.expect_string()?;
        }
        self.expect_keyword("TAG")?;
        let tag = self.expect_string()?;
        let mut features = Vec::new();
        while !self.next_is("END_LANGSYS") {
            self.not_at_end("END_LANGSYS")?;
            self.advance()?;
            features.push(self.feature()?);
            self.expect_keyword("END_FEATURE")?;
        }
        Ok(LangSysDef { tag, features })
    }

    fn feature(&mut self) -> Result<FeatureDef> {
        if !self.cur_is_keyword("DEF_FEATURE") {
            return Err(self.err("Expected \"DEF_FEATURE\""));
        }
        self.expect_keyword("NAME")?;
        self.expect_string()?;
        self.expect_keyword("TAG")?;
        let tag = self.expect_string()?;
        let mut lookups = Vec::new();
        while !self.next_is("END_FEATURE") {
            self.expect_keyword("LOOKUP")?;
            lookups.push(self.expect_string()?);
        }
        Ok(FeatureDef { tag, lookups })
    }

    fn def_lookup(&mut self) -> Result<LookupDef> {
        let loc = self.cur_loc;
        let name = self.expect_string()?;
        if !name.chars().next().is_some_and(char::is_alphabetic) {
            return Err(error_at(loc, format!("Lookup name \"{name}\" must start with a letter")));
        }
        if self.lookups.iter().any(|l| eq_ignore_case(l, &name)) {
            return Err(error_at(loc, format!("Lookup \"{name}\" already defined, lookup names are case insensitive")));
        }
        let mut process_base = true;
        if self.next_is("PROCESS_BASE") {
            self.advance()?;
        } else if self.next_is("SKIP_BASE") {
            self.advance()?;
            process_base = false;
        }
        let mut process_marks = ProcessMarks::All;
        let mut mark_glyph_set = None;
        if self.next_is("PROCESS_MARKS") {
            self.advance()?;
            if self.next_is("MARK_GLYPH_SET") {
                self.advance()?;
                mark_glyph_set = Some(self.expect_string()?);
            } else if self.next_is("ALL") {
                self.advance()?;
            } else if self.next_is("NONE") {
                self.advance()?;
                process_marks = ProcessMarks::None;
            } else if matches!(self.next, Some((Tok::Str(_), _))) {
                let group = self.expect_string()?;
                // An empty group name reads as "no marks", as in voltLib.
                process_marks = if group.is_empty() { ProcessMarks::None } else { ProcessMarks::Class(group) };
            } else {
                let kind = match &self.next {
                    Some((Tok::Name(_), _)) => "NAME",
                    Some((Tok::Num(_), _)) => "NUMBER",
                    Some((Tok::Str(_), _)) => "STRING",
                    None => "None",
                };
                return Err(error_at(loc, format!("Expected ALL, NONE, MARK_GLYPH_SET or an ID. Got {kind}")));
            }
        } else if self.next_is("SKIP_MARKS") {
            self.advance()?;
            process_marks = ProcessMarks::None;
        }
        let mut direction = None;
        if self.next_is("DIRECTION") {
            self.expect_keyword("DIRECTION")?;
            let d = self.expect_name()?;
            if !matches!(d.as_str(), "LTR" | "RTL") {
                return Err(self.err(format!("Expected LTR or RTL, got {d}")));
            }
            direction = Some(d);
        }
        let mut reversal = false;
        if self.next_is("REVERSAL") {
            self.expect_keyword("REVERSAL")?;
            reversal = true;
        }
        let mut comments = None;
        if self.next_is("COMMENTS") {
            self.expect_keyword("COMMENTS")?;
            comments = Some(self.expect_string()?.replace("\\n", "\n"));
        }
        let mut context = Vec::new();
        while self.next_is_any(&["EXCEPT_CONTEXT", "IN_CONTEXT"]) {
            context = self.contexts()?;
        }
        let kind = self.expect_name()?;
        let body = match kind.as_str() {
            "AS_SUBSTITUTION" => Body::Sub(self.substitution()?),
            "AS_POSITION" => Body::Pos(self.position()?),
            _ => return Err(error_at(loc, format!("Expected AS_SUBSTITUTION or AS_POSITION. Got {kind}"))),
        };
        self.lookups.push(name.clone());
        Ok(LookupDef { name, process_base, process_marks, mark_glyph_set, direction, reversal, comments, context, body, loc })
    }

    fn contexts(&mut self) -> Result<Vec<Context>> {
        let mut contexts = Vec::new();
        while self.next_is_any(&["EXCEPT_CONTEXT", "IN_CONTEXT"]) {
            let kind = self.expect_name()?;
            if self.next_is("END_CONTEXT") {
                self.expect_keyword("END_CONTEXT")?;
                continue;
            }
            let (mut left, mut right) = (Vec::new(), Vec::new());
            while self.next_is_any(&["LEFT", "RIGHT"]) {
                let side = self.expect_name()?;
                let coverage = self.coverage(0)?;
                if side == "LEFT" {
                    left.push(coverage);
                } else {
                    right.push(coverage);
                }
            }
            self.expect_keyword("END_CONTEXT")?;
            contexts.push(Context { except: kind == "EXCEPT_CONTEXT", left, right });
        }
        Ok(contexts)
    }

    fn substitution(&mut self) -> Result<Substitution> {
        let loc = self.cur_loc;
        if !self.next_is("SUB") {
            return Err(error_at(loc, "Expected SUB"));
        }
        let mut pairs = Vec::new();
        while self.next_is("SUB") {
            self.expect_keyword("SUB")?;
            let src = self.coverage(0)?;
            self.expect_keyword("WITH")?;
            let dest = self.coverage(0)?;
            self.expect_keyword("END_SUB")?;
            pairs.push((src, dest));
        }
        self.expect_keyword("END_SUBSTITUTION")?;
        let max_src = pairs.iter().map(|(s, _)| s.len()).max().unwrap_or(0);
        let max_dest = pairs.iter().map(|(_, d)| d.len()).max().unwrap_or(0);
        if max_src > 1 && max_dest > 1 {
            return Err(error_at(loc, "Invalid substitution type"));
        }
        Ok(Substitution { pairs, max_src, max_dest, loc })
    }

    fn position(&mut self) -> Result<Position> {
        let loc = self.cur_loc;
        let kind = self.expect_name()?;
        let position = match kind.as_str() {
            "ATTACH" => {
                let coverage = self.coverage(0)?;
                self.expect_keyword("TO")?;
                let mut to = Vec::new();
                while !self.next_is("END_ATTACH") {
                    let cov = self.coverage(0)?;
                    self.expect_keyword("AT")?;
                    self.expect_keyword("ANCHOR")?;
                    to.push((cov, self.expect_string()?));
                }
                self.expect_keyword("END_ATTACH")?;
                Position::Attach { coverage, to }
            }
            "ATTACH_CURSIVE" => {
                let (mut exit, mut enter) = (Vec::new(), Vec::new());
                while !self.next_is("ENTER") {
                    self.expect_keyword("EXIT")?;
                    exit.push(self.coverage(0)?);
                }
                while !self.next_is("END_ATTACH") {
                    self.expect_keyword("ENTER")?;
                    enter.push(self.coverage(0)?);
                }
                self.expect_keyword("END_ATTACH")?;
                Position::Cursive { exit, enter }
            }
            "ADJUST_PAIR" => {
                let (mut first, mut second) = (Vec::new(), Vec::new());
                while self.next_is("FIRST") {
                    self.advance()?;
                    first.push(self.coverage(0)?);
                }
                while self.next_is("SECOND") {
                    self.advance()?;
                    second.push(self.coverage(0)?);
                }
                let mut pairs: Vec<((i64, i64), (Pos, Pos))> = Vec::new();
                while !self.next_is("END_ADJUST") {
                    let a = self.expect_number()?;
                    let b = self.expect_number()?;
                    self.expect_keyword("BY")?;
                    let p1 = self.pos()?;
                    let p2 = self.pos()?;
                    // A repeated pair replaces the earlier values in place, as a dictionary does.
                    match pairs.iter_mut().find(|(k, _)| *k == (a, b)) {
                        Some(entry) => entry.1 = (p1, p2),
                        None => pairs.push(((a, b), (p1, p2))),
                    }
                }
                self.expect_keyword("END_ADJUST")?;
                Position::Pair { first, second, pairs }
            }
            "ADJUST_SINGLE" => {
                let mut items = Vec::new();
                while !self.next_is("END_ADJUST") {
                    let cov = self.coverage(0)?;
                    self.expect_keyword("BY")?;
                    items.push((cov, self.pos()?));
                }
                self.expect_keyword("END_ADJUST")?;
                Position::Single(items)
            }
            _ => return Err(error_at(loc, "Expected ATTACH, ATTACH_CURSIVE, ADJUST_PAIR, ADJUST_SINGLE")),
        };
        self.expect_keyword("END_POSITION")?;
        Ok(position)
    }

    fn def_anchor(&mut self) -> Result<AnchorDef> {
        let loc = self.cur_loc;
        let name = self.expect_string()?;
        self.expect_keyword("ON")?;
        self.expect_number()?;
        self.expect_keyword("GLYPH")?;
        let glyph = self.expect_name()?;
        self.expect_keyword("COMPONENT")?;
        let component = self.expect_number()?;
        let known = self.anchors.entry(glyph.clone()).or_default();
        if known.iter().any(|(n, c)| *c == component && eq_ignore_case(n, &name)) {
            return Err(error_at(loc, format!("Anchor \"{name}\" already defined, anchor names are case insensitive")));
        }
        known.push((name.clone(), component));
        if self.next_is("LOCKED") {
            self.advance()?;
        }
        self.expect_keyword("AT")?;
        let pos = self.pos()?;
        self.expect_keyword("END_ANCHOR")?;
        Ok(AnchorDef { name, glyph, component, pos, loc })
    }

    fn adjust_by(&mut self) -> Result<(i64, i64)> {
        self.advance()?;
        if !self.cur_is_keyword("ADJUST_BY") {
            return Err(self.err("Expected \"ADJUST_BY\""));
        }
        let adjustment = self.expect_number()?;
        self.expect_keyword("AT")?;
        let size = self.expect_number()?;
        Ok((size, adjustment))
    }

    /// Device adjustments keyed by size; a repeated size replaces the earlier one in place.
    fn adjustments(&mut self) -> Result<Vec<(i64, i64)>> {
        let mut out: Vec<(i64, i64)> = Vec::new();
        while self.next_is("ADJUST_BY") {
            let (size, adjustment) = self.adjust_by()?;
            match out.iter_mut().find(|(s, _)| *s == size) {
                Some(entry) => entry.1 = adjustment,
                None => out.push((size, adjustment)),
            }
        }
        Ok(out)
    }

    fn pos(&mut self) -> Result<Pos> {
        self.advance()?;
        if !self.cur_is_keyword("POS") {
            return Err(self.err("Expected \"POS\""));
        }
        let mut pos = Pos::default();
        if self.next_is("ADV") {
            self.advance()?;
            pos.adv = Some(self.expect_number()?);
            pos.adv_adjust = self.adjustments()?;
        }
        if self.next_is("DX") {
            self.advance()?;
            pos.dx = Some(self.expect_number()?);
            pos.dx_adjust = self.adjustments()?;
        }
        if self.next_is("DY") {
            self.advance()?;
            pos.dy = Some(self.expect_number()?);
            pos.dy_adjust = self.adjustments()?;
        }
        self.expect_keyword("END_POS")?;
        Ok(pos)
    }

    fn parse_enum(&mut self, depth: usize) -> Result<Coverage> {
        self.expect_keyword("ENUM")?;
        let items = self.coverage(depth + 1)?;
        self.expect_keyword("END_ENUM")?;
        Ok(items)
    }

    fn coverage(&mut self, depth: usize) -> Result<Coverage> {
        if depth > MAX_NESTING {
            return Err(self.err(format!("ENUM nested more than {MAX_NESTING} deep")));
        }
        let mut items = Vec::new();
        let loc = self.cur_loc;
        while self.next_is_any(&["GLYPH", "GROUP", "RANGE", "ENUM"]) {
            if self.next_is("ENUM") {
                items.push(Item::Enum(self.parse_enum(depth)?));
            } else if self.next_is("GLYPH") {
                self.expect_keyword("GLYPH")?;
                items.push(Item::Glyph(self.expect_string()?));
            } else if self.next_is("GROUP") {
                self.expect_keyword("GROUP")?;
                items.push(Item::Group(self.expect_string()?, loc));
            } else {
                self.expect_keyword("RANGE")?;
                let start = self.expect_string()?;
                self.expect_keyword("TO")?;
                let end = self.expect_string()?;
                items.push(Item::Range(start, end, loc));
            }
        }
        Ok(items)
    }
}

//! The FontDame (MTI) reader: lines of tab-separated fields, read into the tables they
//! describe.
//!
//! Ported from fontTools `mtiLib` (MIT, Copyright (c) 2015 Google Inc. and others; see
//! `NOTICE`): the same line rules, keywords and checks. mtiLib builds binary tables; this
//! reader keeps what the text says (rules, classes, anchors, lookup references by name) for
//! the feature-code writer.

use crate::{Result, TablesError};

/// A line of the file: its number and its fields, each trimmed.
#[derive(Clone, Debug)]
pub(super) struct Line {
    pub no: usize,
    pub fields: Vec<String>,
}

impl Line {
    pub fn first(&self) -> &str {
        self.fields.first().map_or("", String::as_str)
    }

    pub fn field(&self, i: usize) -> &str {
        self.fields.get(i).map_or("", String::as_str)
    }
}

fn err(no: usize, message: impl std::fmt::Display) -> TablesError {
    TablesError::Invalid(format!("FontDame line {no}: {message}"))
}

/// The lines that carry content: empty lines and `%` comments are dropped, except the
/// `% subtable` separator. Lines end at `\r\n`, `\r` or `\n`.
pub(super) fn tokenize(text: &str) -> Vec<Line> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    for (i, raw) in text.split('\n').flat_map(|l| l.strip_suffix('\r').unwrap_or(l).split('\r')).enumerate() {
        let mut fields: Vec<String> = raw.split('\t').map(|f| f.trim().to_string()).collect();
        while fields.last().is_some_and(String::is_empty) {
            fields.pop();
        }
        let keep = fields.first().is_some_and(|f| !f.is_empty() && (!f.starts_with('%') || f == "% subtable"));
        if keep {
            out.push(Line { no: i + 1, fields });
        }
    }
    out
}

/// Reading position with mtiLib's stop words: a line whose first field, lower-cased, is a stop
/// word ends the block being read without being consumed.
pub(super) struct Cursor {
    lines: Vec<Line>,
    pos: usize,
    stoppers: Vec<String>,
}

impl Cursor {
    pub fn new(lines: Vec<Line>) -> Self {
        Self { lines, pos: 0, stoppers: Vec::new() }
    }

    fn line_no(&self) -> usize {
        self.lines.get(self.pos).or(self.lines.last()).map_or(0, |l| l.no)
    }

    /// The next line, unless it is a stop word or the end.
    pub fn peek(&self) -> Option<&Line> {
        let line = self.lines.get(self.pos)?;
        if self.stoppers.contains(&line.first().to_lowercase()) { None } else { Some(line) }
    }

    /// The first field of the next line, or `""`.
    pub fn peek_first(&self) -> String {
        self.peek().map(|l| l.first().to_string()).unwrap_or_default()
    }

    /// The next line within the block, or `None` at a stop word or the end.
    pub fn next_in_block(&mut self) -> Option<Line> {
        let line = self.peek()?.clone();
        self.pos += 1;
        Some(line)
    }

    /// The next line, which must be there.
    pub fn next(&mut self) -> Result<Line> {
        let no = self.line_no();
        self.next_in_block().ok_or_else(|| err(no, "the file ends, or a block closes, where a line was expected"))
    }

    pub fn skip(&mut self) {
        self.pos += 1;
    }

    pub fn expect(&mut self, word: &str) -> Result<Line> {
        let line = self.next()?;
        if line.first().to_lowercase() != word {
            return Err(err(line.no, format!("Expected '{word}', got '{}'", line.first().to_lowercase())));
        }
        Ok(line)
    }

    pub fn expect_ending(&mut self, word: &str) -> Result<Line> {
        let line = self.next()?;
        if !line.first().to_lowercase().ends_with(word) {
            return Err(err(line.no, format!("Expected '*{word}', got '{}'", line.first().to_lowercase())));
        }
        Ok(line)
    }

    pub fn push(&mut self, words: &[&str]) {
        self.stoppers.extend(words.iter().map(|w| (*w).to_string()));
    }

    pub fn pop(&mut self, n: usize) {
        self.stoppers.truncate(self.stoppers.len().saturating_sub(n));
    }

    /// Opens a `<tag> begin` … `<tag> end` block.
    pub fn begin(&mut self, tag: &str) -> Result<Line> {
        let line = self.expect_ending(&format!("{tag} begin"))?;
        self.stoppers.push(format!("{tag} end"));
        Ok(line)
    }

    pub fn end(&mut self, tag: &str) -> Result<()> {
        self.pop(1);
        self.expect(&format!("{tag} end")).map(|_| ())
    }

    pub fn position(&self) -> usize {
        self.pos
    }
}

// ── the tables ─────────────────────────────────────────────────────────────────────────────────

/// A value record: x placement, y placement, x advance, y advance.
pub(super) type Value = [Option<i64>; 4];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AnchorPt {
    pub x: i64,
    pub y: i64,
    pub point: Option<i64>,
}

/// A lookup record: the 1-based position in the input sequence and the lookup's name.
pub(super) type Record = (usize, String);

#[derive(Clone, Debug)]
pub(super) enum Rules {
    /// Format 1: backtrack (as listed, nearest glyph first), input, lookahead.
    Glyph(Vec<([Vec<String>; 3], Vec<Record>, usize)>),
    /// Format 2: class definitions for backtrack, input and lookahead, and rules of class
    /// numbers.
    Class { defs: [Option<Vec<(String, u16)>>; 3], rules: Vec<([Vec<u16>; 3], Vec<Record>, usize)> },
    /// Format 3: one coverage per position.
    Coverage { coverages: [Vec<Vec<String>>; 3], records: Vec<Record>, line: usize },
}

#[derive(Clone, Debug)]
pub(super) enum Subtable {
    SingleSub(Vec<(String, String)>),
    MultipleSub(Vec<(String, Vec<String>)>),
    AlternateSub(Vec<(String, Vec<String>)>),
    Ligature(Vec<(String, Vec<String>)>),
    SinglePos(Vec<(String, Value)>),
    PairGlyph(Vec<(String, String, Value, Value)>),
    PairClass {
        first: Vec<(String, u16)>,
        second: Vec<(String, u16)>,
        records: Vec<(u16, u16, Value, Value)>,
        line: usize,
    },
    Cursive(Vec<(String, Option<AnchorPt>, Option<AnchorPt>)>),
    /// Marks (glyph, class, anchor) and bases (glyph, class, ligature component and count,
    /// anchor).
    Mark {
        marks: Vec<(String, u16, AnchorPt)>,
        #[allow(clippy::type_complexity)]
        bases: Vec<(String, u16, Option<(u16, u16)>, AnchorPt)>,
        line: usize,
    },
    Context(Rules),
    Reverse {
        backtrack: Vec<Vec<String>>,
        lookahead: Vec<Vec<String>>,
        mapping: Vec<(String, String)>,
        line: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    SingleSub,
    MultipleSub,
    AlternateSub,
    LigatureSub,
    ContextSub,
    ChainSub,
    ReverseSub,
    SinglePos,
    Pair,
    Kernset,
    Cursive,
    MarkBase,
    MarkLigature,
    MarkMark,
    ContextPos,
    ChainPos,
}

impl Kind {
    fn from(table: &[u8; 4], word: &str) -> Option<Self> {
        Some(match (table, word) {
            (b"GSUB", "single") => Kind::SingleSub,
            (b"GSUB", "multiple") => Kind::MultipleSub,
            (b"GSUB", "alternate") => Kind::AlternateSub,
            (b"GSUB", "ligature") => Kind::LigatureSub,
            (b"GSUB", "context") => Kind::ContextSub,
            (b"GSUB", "chained") => Kind::ChainSub,
            (b"GSUB", "reversechained") => Kind::ReverseSub,
            (b"GPOS", "single") => Kind::SinglePos,
            (b"GPOS", "pair") => Kind::Pair,
            (b"GPOS", "kernset") => Kind::Kernset,
            (b"GPOS", "cursive") => Kind::Cursive,
            (b"GPOS", "mark to base") => Kind::MarkBase,
            (b"GPOS", "mark to ligature") => Kind::MarkLigature,
            (b"GPOS", "mark to mark") => Kind::MarkMark,
            (b"GPOS", "context") => Kind::ContextPos,
            (b"GPOS", "chained") => Kind::ChainPos,
            _ => return None,
        })
    }

    pub fn chained(self) -> bool {
        matches!(self, Kind::ChainSub | Kind::ChainPos)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Lookup {
    pub name: String,
    pub kind: Kind,
    /// The low byte of the lookup flags (right to left, ignore base glyphs, ligatures,
    /// marks; 0x10 uses a mark filtering set).
    pub flags: u16,
    pub mark_attach: u16,
    pub filter_set: Option<u32>,
    pub subtables: Vec<Subtable>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ScriptRow {
    pub script: String,
    pub lang: String,
    pub required: Option<String>,
    pub features: Vec<String>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub(super) struct FeatureRow {
    pub name: String,
    pub tag: String,
    pub lookups: Vec<String>,
    pub line: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Layout {
    pub scripts: Option<Vec<ScriptRow>>,
    pub features: Option<Vec<FeatureRow>>,
    pub lookups: Vec<Lookup>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Gdef {
    pub glyph_classes: Option<Vec<(String, u16)>>,
    pub attach: Option<Vec<(String, Vec<i64>)>>,
    pub carets: Option<Vec<(String, Vec<i64>)>>,
    pub mark_attach: Option<Vec<(String, u16)>>,
    pub filter_sets: Option<Vec<(u32, Vec<String>)>>,
}

#[derive(Clone, Debug)]
pub(super) enum Table {
    Layout([u8; 4], Layout),
    Gdef(Gdef),
}

/// The table a file names in its first line (`FontDame GSUB table`, `Font Chef Table GPOS`).
pub(super) fn header_tag(lines: &[Line]) -> Option<String> {
    let first = lines.first()?.first();
    let words: Vec<&str> = first.split_whitespace().collect();
    if words.first() == Some(&"FontDame") {
        return words.get(1).map(|s| (*s).to_string());
    }
    if words.get(..3) == Some(&["Font", "Chef", "Table"]) {
        return words.get(3).map(|s| (*s).to_string());
    }
    None
}

const GDEF_BLOCKS: [&str; 5] =
    ["class definition begin", "attachment list begin", "carets begin", "mark attachment class definition begin", "markfilter set definition begin"];

/// The table a file without a header describes, judged from its content: GDEF blocks, or the
/// lookup types only one of GSUB and GPOS has, or the shape of a single lookup's lines.
pub(super) fn sniff(lines: &[Line]) -> Option<[u8; 4]> {
    let mut iter = lines.iter().peekable();
    while let Some(line) = iter.next() {
        let first = line.first().to_lowercase();
        if GDEF_BLOCKS.contains(&first.as_str()) {
            return Some(*b"GDEF");
        }
        if first != "lookup" {
            continue;
        }
        match line.field(2).to_lowercase().as_str() {
            "multiple" | "alternate" | "ligature" | "reversechained" => return Some(*b"GSUB"),
            "pair" | "kernset" | "cursive" | "mark to base" | "mark to ligature" | "mark to mark" => return Some(*b"GPOS"),
            "single" => {
                for next in iter.by_ref() {
                    let f = next.first().to_lowercase();
                    if FLAG_WORDS.contains(&f.as_str()) {
                        continue;
                    }
                    if f == "lookup end" {
                        break;
                    }
                    let value_words = ["x placement", "y placement", "x advance", "y advance"];
                    return Some(if next.fields.len() == 3 && value_words.contains(&f.as_str()) { *b"GPOS" } else { *b"GSUB" });
                }
            }
            _ => {}
        }
    }
    None
}

const FLAG_WORDS: [&str; 6] = ["righttoleft", "ignorebaseglyphs", "ignoreligatures", "ignoremarks", "markattachmenttype", "markfiltertype"];

/// The glyph a field names: `U 0041` by code point, `# 12` by glyph id, else the name itself.
pub(super) fn glyph(s: &str, no: usize, order: &[String]) -> Result<String> {
    if let Some(hex) = s.strip_prefix("U ").or_else(|| s.strip_prefix("u ")) {
        let cp = u32::from_str_radix(hex.trim(), 16).map_err(|_| err(no, format!("bad code point in \"{s}\"")))?;
        return Ok(if cp <= 0xFFFF { format!("uni{cp:04X}") } else { format!("u{cp:X}") });
    }
    if let Some(id) = s.strip_prefix("# ") {
        let id: usize = id.trim().parse().map_err(|_| err(no, format!("bad glyph id in \"{s}\"")))?;
        return Ok(order.get(id).cloned().unwrap_or_else(|| format!("glyph{id:05}")));
    }
    if s.is_empty() {
        return Err(err(no, "Glyph name is empty"));
    }
    if s.contains(' ') {
        return Err(err(no, format!("Space found in glyph name: {s}")));
    }
    Ok(s.to_string())
}

fn int(s: &str, no: usize) -> Result<i64> {
    s.trim().parse::<i64>().map_err(|_| err(no, format!("expected a whole number, got \"{s}\"")))
}

fn small(s: &str, no: usize) -> Result<u16> {
    u16::try_from(int(s, no)?).map_err(|_| err(no, format!("\"{s}\" is out of range")))
}

fn split_comma(s: &str) -> Vec<String> {
    if s.is_empty() { Vec::new() } else { s.split(',').map(|p| p.trim().to_string()).collect() }
}

fn int_split(s: &str, no: usize) -> Result<Vec<i64>> {
    if s.is_empty() { Ok(Vec::new()) } else { s.split(',').map(|p| int(p, no)).collect() }
}

fn value_index(what: &str, no: usize) -> Result<usize> {
    let key: String = what.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    match key.as_str() {
        "xplacement" => Ok(0),
        "yplacement" => Ok(1),
        "xadvance" => Ok(2),
        "yadvance" => Ok(3),
        _ => Err(err(no, format!("unknown value \"{what}\""))),
    }
}

fn set_value(v: &mut Value, i: usize, value: i64, no: usize) -> Result<()> {
    let slot = v.get_mut(i).ok_or_else(|| err(no, "bad value index"))?;
    if slot.is_some() {
        return Err(err(no, "the same value is given twice"));
    }
    *slot = Some(value);
    Ok(())
}

fn anchor(data: &[String], no: usize) -> Result<AnchorPt> {
    if data.len() > 2 {
        return Err(err(no, "an anchor has more than two fields"));
    }
    let xy = int_split(data.first().map_or("", String::as_str), no)?;
    let [x, y] = xy.as_slice() else { return Err(err(no, "an anchor needs \"x, y\"")) };
    let point = match data.get(1) {
        Some(p) if !p.is_empty() => Some(int(p, no)?),
        _ => None,
    };
    Ok(AnchorPt { x: *x, y: *y, point })
}

/// Inserts or replaces by key, keeping the first position, as a dictionary does.
fn upsert<K: PartialEq, V>(list: &mut Vec<(K, V)>, key: K, value: V) {
    match list.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => list.push((key, value)),
    }
}

pub(super) struct Reader<'a> {
    pub cur: Cursor,
    pub order: &'a [String],
}

impl Reader<'_> {
    fn glyphs(&self, fields: &[String], no: usize) -> Result<Vec<String>> {
        fields.iter().map(|f| glyph(f, no, self.order)).collect()
    }

    fn class_def(&mut self) -> Result<Vec<(String, u16)>> {
        self.cur.begin("class definition")?;
        let mut out: Vec<(String, u16)> = Vec::new();
        while let Some(line) = self.cur.next_in_block() {
            let g = glyph(line.first(), line.no, self.order)?;
            if out.iter().any(|(n, _)| *n == g) {
                return Err(err(line.no, format!("{g} is in the class definition twice")));
            }
            out.push((g, small(line.field(1), line.no)?));
        }
        self.cur.end("class definition")?;
        Ok(out)
    }

    fn coverage(&mut self) -> Result<Vec<String>> {
        self.cur.begin("coverage definition")?;
        let mut out = Vec::new();
        while let Some(line) = self.cur.next_in_block() {
            out.push(glyph(line.first(), line.no, self.order)?);
        }
        self.cur.end("coverage definition")?;
        Ok(out)
    }

    fn records(&self, fields: &[String], no: usize) -> Result<Vec<Record>> {
        let mut out = Vec::new();
        for f in fields {
            let parts = split_comma(f);
            let [idx, lookup] = parts.as_slice() else { return Err(err(no, format!("a lookup record is \"position, lookup\", got \"{f}\""))) };
            let idx = int(idx, no)?;
            let idx =
                usize::try_from(idx).ok().filter(|i| *i > 0).ok_or_else(|| err(no, format!("lookup record position {idx} is not 1 or more")))?;
            out.push((idx, lookup.clone()));
        }
        Ok(out)
    }

    pub fn read(&mut self, table: [u8; 4]) -> Result<Table> {
        if &table == b"GDEF" {
            return self.gdef().map(Table::Gdef);
        }
        let mut layout = Layout::default();
        while let Some(line) = self.cur.peek().cloned() {
            match line.first().to_lowercase().as_str() {
                "script table begin" => {
                    if layout.scripts.is_some() {
                        return Err(err(line.no, "a second script table"));
                    }
                    layout.scripts = Some(self.script_table()?);
                }
                "feature table begin" => {
                    if layout.features.is_some() {
                        return Err(err(line.no, "a second feature table"));
                    }
                    layout.features = Some(self.feature_table()?);
                }
                "lookup" => {
                    let lookup = self.lookup(&table)?;
                    if layout.lookups.iter().any(|l| l.name == lookup.name) {
                        return Err(err(line.no, format!("Duplicate lookup name: {}", lookup.name)));
                    }
                    layout.lookups.push(lookup);
                }
                _ => self.cur.skip(),
            }
        }
        Ok(Table::Layout(table, layout))
    }

    fn script_table(&mut self) -> Result<Vec<ScriptRow>> {
        self.cur.begin("script table")?;
        let mut rows = Vec::new();
        while let Some(line) = self.cur.next_in_block() {
            if line.fields.len() > 4 {
                return Err(err(line.no, "a script table line has more than four fields"));
            }
            let required = Some(line.field(2).to_string()).filter(|s| !s.is_empty());
            rows.push(ScriptRow {
                script: line.first().to_string(),
                lang: line.field(1).to_string(),
                required,
                features: split_comma(line.field(3)),
                line: line.no,
            });
        }
        self.cur.end("script table")?;
        Ok(rows)
    }

    fn feature_table(&mut self) -> Result<Vec<FeatureRow>> {
        self.cur.begin("feature table")?;
        let mut rows: Vec<FeatureRow> = Vec::new();
        while let Some(line) = self.cur.next_in_block() {
            if line.fields.len() > 3 {
                return Err(err(line.no, "a feature table line has more than three fields"));
            }
            let name = line.first().to_string();
            if rows.iter().any(|r| r.name == name) {
                return Err(err(line.no, format!("Duplicate feature name: {name}")));
            }
            if let Ok(n) = name.parse::<usize>()
                && n != rows.len()
            {
                return Err(err(line.no, format!("feature {n} is listed at position {}", rows.len())));
            }
            rows.push(FeatureRow { name, tag: line.field(1).to_string(), lookups: split_comma(line.field(2)), line: line.no });
        }
        self.cur.end("feature table")?;
        Ok(rows)
    }

    fn lookup(&mut self, table: &[u8; 4]) -> Result<Lookup> {
        let line = self.cur.expect("lookup")?;
        let [_, name, typ] = line.fields.as_slice() else {
            return Err(err(line.no, "a lookup line is \"lookup<tab>name<tab>type\""));
        };
        let kind = Kind::from(table, &typ.to_lowercase())
            .ok_or_else(|| err(line.no, format!("unknown {} lookup type \"{typ}\"", String::from_utf8_lossy(table))))?;
        let (mut flags, mut mark_attach, mut filter_set) = (0u16, 0u16, None);
        while FLAG_WORDS.contains(&self.cur.peek_first().to_lowercase().as_str()) {
            let f = self.cur.next()?;
            let word = f.first().to_lowercase();
            let bit = match word.as_str() {
                "righttoleft" => 1,
                "ignorebaseglyphs" => 2,
                "ignoreligatures" => 4,
                "ignoremarks" => 8,
                _ => 0,
            };
            if bit != 0 {
                match f.field(1).to_lowercase().as_str() {
                    "yes" => flags |= bit,
                    "no" => {}
                    other => return Err(err(f.no, format!("expected yes or no, got \"{other}\""))),
                }
            } else if word == "markattachmenttype" {
                mark_attach = u16::try_from(int(f.field(1), f.no)?)
                    .ok()
                    .filter(|c| *c <= 255)
                    .ok_or_else(|| err(f.no, "mark attachment class out of range"))?;
            } else {
                flags |= 0x10;
                filter_set = Some(u32::try_from(int(f.field(1), f.no)?).map_err(|_| err(f.no, "mark filtering set out of range"))?);
            }
        }
        self.cur.push(&["lookup end"]);
        let mut subtables = Vec::new();
        while self.cur.peek().is_some() {
            self.cur.push(&["% subtable", "subtable end"]);
            while self.cur.peek().is_some() {
                let before = self.cur.position();
                subtables.push(self.subtable(kind)?);
                if self.cur.position() == before {
                    return Err(err(line.no, format!("lookup \"{name}\" has a line that is not part of a {typ} lookup")));
                }
            }
            self.cur.pop(2);
            if matches!(self.cur.peek_first().as_str(), "% subtable" | "subtable end") {
                self.cur.skip();
            }
        }
        self.cur.pop(1);
        self.cur.expect("lookup end")?;
        Ok(Lookup { name: name.clone(), kind, flags, mark_attach, filter_set, subtables, line: line.no })
    }

    fn subtable(&mut self, kind: Kind) -> Result<Subtable> {
        Ok(match kind {
            Kind::SingleSub => {
                let mut m = Vec::new();
                while let Some(line) = self.cur.next_in_block() {
                    let [a, b] = line.fields.as_slice() else { return Err(err(line.no, "a single substitution line has two glyphs")) };
                    upsert(&mut m, glyph(a, line.no, self.order)?, glyph(b, line.no, self.order)?);
                }
                Subtable::SingleSub(m)
            }
            Kind::MultipleSub | Kind::AlternateSub => {
                let mut m = Vec::new();
                while let Some(line) = self.cur.next_in_block() {
                    let g = self.glyphs(&line.fields, line.no)?;
                    let (first, rest) = g.split_first().ok_or_else(|| err(line.no, "an empty line"))?;
                    upsert(&mut m, first.clone(), rest.to_vec());
                }
                if kind == Kind::MultipleSub { Subtable::MultipleSub(m) } else { Subtable::AlternateSub(m) }
            }
            Kind::LigatureSub => {
                let mut m: Vec<(Vec<String>, String)> = Vec::new();
                while let Some(line) = self.cur.next_in_block() {
                    if line.fields.len() < 2 {
                        return Err(err(line.no, "a ligature line has the ligature and its components"));
                    }
                    let g = self.glyphs(&line.fields, line.no)?;
                    let (lig, comps) = g.split_first().ok_or_else(|| err(line.no, "an empty line"))?;
                    upsert(&mut m, comps.to_vec(), lig.clone());
                }
                Subtable::Ligature(m.into_iter().map(|(c, l)| (l, c)).collect())
            }
            Kind::SinglePos => {
                let mut values: Vec<(String, Value)> = Vec::new();
                while let Some(line) = self.cur.next_in_block() {
                    let [what, g, v] = line.fields.as_slice() else { return Err(err(line.no, "a single positioning line has three fields")) };
                    let i = value_index(what, line.no)?;
                    let g = glyph(g, line.no, self.order)?;
                    let v = int(v, line.no)?;
                    let entry = match values.iter().position(|(n, _)| *n == g) {
                        Some(p) => p,
                        None => {
                            values.push((g, [None; 4]));
                            values.len() - 1
                        }
                    };
                    if let Some((_, record)) = values.get_mut(entry) {
                        set_value(record, i, v, line.no)?;
                    }
                }
                Subtable::SinglePos(values)
            }
            Kind::Pair => self.pair()?,
            Kind::Kernset => {
                let side = self.cur.peek_first().split_whitespace().next().unwrap_or_default().to_lowercase();
                if side == "left" || side == "right" {
                    self.cur.push(&["firstclass definition begin", "secondclass definition begin"]);
                    let s = self.pair();
                    self.cur.pop(2);
                    s?
                } else {
                    self.pair()?
                }
            }
            Kind::Cursive => {
                let mut records: Vec<(String, Option<AnchorPt>, Option<AnchorPt>)> = Vec::new();
                while let Some(line) = self.cur.next_in_block() {
                    if !(3..=4).contains(&line.fields.len()) {
                        return Err(err(line.no, "a cursive line has three or four fields"));
                    }
                    let exit = match line.first() {
                        "entry" => false,
                        "exit" => true,
                        other => return Err(err(line.no, format!("expected entry or exit, got \"{other}\""))),
                    };
                    let g = glyph(line.field(1), line.no, self.order)?;
                    let a = anchor(line.fields.get(2..).unwrap_or_default(), line.no)?;
                    let entry = match records.iter().position(|(n, _, _)| *n == g) {
                        Some(p) => p,
                        None => {
                            records.push((g.clone(), None, None));
                            records.len() - 1
                        }
                    };
                    if let Some(r) = records.get_mut(entry) {
                        let slot = if exit { &mut r.2 } else { &mut r.1 };
                        if slot.is_some() {
                            return Err(err(line.no, format!("{g} has two {} anchors", line.first())));
                        }
                        *slot = Some(a);
                    }
                }
                Subtable::Cursive(records)
            }
            Kind::MarkBase | Kind::MarkLigature | Kind::MarkMark => {
                let start = self.cur.peek().map_or(0, |l| l.no);
                let (mut marks, mut bases) = (Vec::new(), Vec::new());
                let base_word = if kind == Kind::MarkLigature { "ligature" } else { "base" };
                while let Some(line) = self.cur.next_in_block() {
                    let typ = line.first();
                    let g = glyph(line.field(1), line.no, self.order)?;
                    if typ == "mark" {
                        let c = small(line.field(2), line.no)?;
                        let a = anchor(line.fields.get(3..).unwrap_or_default(), line.no)?;
                        if marks.iter().any(|(m, _, _): &(String, u16, AnchorPt)| *m == g) {
                            return Err(err(line.no, format!("mark {g} is given twice")));
                        }
                        marks.push((g, c, a));
                    } else if typ == base_word {
                        let (component, c, a) = if kind == Kind::MarkLigature {
                            let comp = (small(line.field(2), line.no)?, small(line.field(3), line.no)?);
                            (Some(comp), small(line.field(4), line.no)?, anchor(line.fields.get(5..).unwrap_or_default(), line.no)?)
                        } else {
                            (None, small(line.field(2), line.no)?, anchor(line.fields.get(3..).unwrap_or_default(), line.no)?)
                        };
                        if bases.iter().any(|(b, k, comp, _): &(String, u16, Option<(u16, u16)>, AnchorPt)| *b == g && *k == c && *comp == component)
                        {
                            return Err(err(line.no, format!("{g} has two anchors for class {c}")));
                        }
                        bases.push((g, c, component, a));
                    } else {
                        return Err(err(line.no, format!("expected mark or {base_word}, got \"{typ}\"")));
                    }
                }
                Subtable::Mark { marks, bases, line: start }
            }
            Kind::ContextSub | Kind::ContextPos | Kind::ChainSub | Kind::ChainPos => Subtable::Context(self.context(kind.chained())?),
            Kind::ReverseSub => {
                let start = self.cur.peek().map_or(0, |l| l.no);
                let (mut backtrack, mut lookahead) = (Vec::new(), Vec::new());
                while self.cur.peek_first().ends_with("coverage definition begin") {
                    let first = self.cur.peek_first();
                    let which = first.get(..first.len().saturating_sub("coverage definition begin".len())).unwrap_or_default().to_lowercase();
                    let cov = self.coverage()?;
                    match which.as_str() {
                        "backtrack" => backtrack.push(cov),
                        "lookahead" => lookahead.push(cov),
                        _ => return Err(err(start, format!("unknown coverage \"{first}\" in a reverse chaining lookup"))),
                    }
                }
                let mut mapping = Vec::new();
                while let Some(line) = self.cur.next_in_block() {
                    let [a, b] = line.fields.as_slice() else { return Err(err(line.no, "a reverse substitution line has two glyphs")) };
                    upsert(&mut mapping, glyph(a, line.no, self.order)?, glyph(b, line.no, self.order)?);
                }
                Subtable::Reverse { backtrack, lookahead, mapping, line: start }
            }
        })
    }

    fn pair(&mut self) -> Result<Subtable> {
        let start = self.cur.peek().map_or(0, |l| l.no);
        let typ = self.cur.peek_first().split_whitespace().next().unwrap_or_default().to_lowercase();
        let side_value = |line: &Line| -> Result<(bool, usize)> {
            let first = line.first();
            let side = first.split_whitespace().next().unwrap_or_default().to_lowercase();
            let right = match side.as_str() {
                "left" => false,
                "right" => true,
                _ => return Err(err(line.no, format!("expected left or right, got \"{side}\""))),
            };
            Ok((right, value_index(first.get(side.len()..).unwrap_or_default(), line.no)?))
        };
        if typ == "left" || typ == "right" {
            let mut pairs: Vec<(String, String, Value, Value)> = Vec::new();
            while let Some(line) = self.cur.next_in_block() {
                if line.fields.len() != 4 {
                    return Err(err(line.no, "a pair line has four fields"));
                }
                let (right, i) = side_value(&line)?;
                let (g1, g2) = (glyph(line.field(1), line.no, self.order)?, glyph(line.field(2), line.no, self.order)?);
                let v = int(line.field(3), line.no)?;
                let entry = match pairs.iter().position(|(a, b, _, _)| *a == g1 && *b == g2) {
                    Some(p) => p,
                    None => {
                        pairs.push((g1, g2, [None; 4], [None; 4]));
                        pairs.len() - 1
                    }
                };
                if let Some(p) = pairs.get_mut(entry) {
                    set_value(if right { &mut p.3 } else { &mut p.2 }, i, v, line.no)?;
                }
            }
            return Ok(Subtable::PairGlyph(pairs));
        }
        if !typ.ends_with("class") {
            return Err(err(start, format!("expected a pair line or a class definition, got \"{typ}\"")));
        }
        let (mut first, mut second) = (None, None);
        while self.cur.peek_first().ends_with("class definition begin") {
            let f = self.cur.peek_first();
            let which = f.get(..f.len().saturating_sub("class definition begin".len())).unwrap_or_default().to_lowercase();
            let slot = match which.as_str() {
                "first" => &mut first,
                "second" => &mut second,
                _ => return Err(err(start, format!("unknown class definition \"{f}\""))),
            };
            if slot.is_some() {
                return Err(err(start, format!("a second \"{f}\"")));
            }
            *slot = Some(self.class_def()?);
        }
        let (Some(first), Some(second)) = (first, second) else {
            return Err(err(start, "a class pair subtable needs both class definitions"));
        };
        let count = |d: &[(String, u16)]| d.iter().map(|(_, c)| *c).max().unwrap_or(0);
        let (n1, n2) = (count(&first), count(&second));
        let mut records: Vec<(u16, u16, Value, Value)> = Vec::new();
        while let Some(line) = self.cur.next_in_block() {
            if line.fields.len() != 4 {
                return Err(err(line.no, "a class pair line has four fields"));
            }
            let (right, i) = side_value(&line)?;
            let (c1, c2, v) = (small(line.field(1), line.no)?, small(line.field(2), line.no)?, int(line.field(3), line.no)?);
            if c1 > n1 || c2 > n2 {
                return Err(err(line.no, format!("class {c1} or {c2} is not defined")));
            }
            let entry = match records.iter().position(|(a, b, _, _)| *a == c1 && *b == c2) {
                Some(p) => p,
                None => {
                    records.push((c1, c2, [None; 4], [None; 4]));
                    records.len() - 1
                }
            };
            if let Some(r) = records.get_mut(entry) {
                set_value(if right { &mut r.3 } else { &mut r.2 }, i, v, line.no)?;
            }
        }
        Ok(Subtable::PairClass { first, second, records, line: start })
    }

    fn context(&mut self, chain: bool) -> Result<Rules> {
        let start = self.cur.peek().map_or(0, |l| l.no);
        let data_len = if chain { 3 } else { 1 };
        // Where each listed sequence goes: backtrack, input, lookahead.
        let slots: &[usize] = if chain { &[0, 1, 2] } else { &[1] };
        let typ = self.cur.peek_first().split_whitespace().next().unwrap_or_default().to_lowercase();
        if typ == "glyph" {
            let mut rules = Vec::new();
            while let Some(line) = self.cur.next_in_block() {
                if line.first().to_lowercase() != "glyph" {
                    return Err(err(line.no, format!("expected a glyph rule, got \"{}\"", line.first())));
                }
                let mut seqs: [Vec<String>; 3] = Default::default();
                for (k, slot) in slots.iter().enumerate() {
                    let g = self.glyphs(&split_comma(line.field(1 + k)), line.no)?;
                    if let Some(s) = seqs.get_mut(*slot) {
                        *s = g;
                    }
                }
                let recs = self.records(line.fields.get(1 + data_len..).unwrap_or_default(), line.no)?;
                rules.push((seqs, recs, line.no));
            }
            return Ok(Rules::Glyph(rules));
        }
        if typ.ends_with("class") {
            let mut defs: [Option<Vec<(String, u16)>>; 3] = Default::default();
            while self.cur.peek_first().ends_with("class definition begin") {
                let f = self.cur.peek_first();
                let which = f.get(..f.len().saturating_sub("class definition begin".len())).unwrap_or_default().to_lowercase();
                let slot = match (chain, which.as_str()) {
                    (_, "") => 1,
                    (true, "backtrack") => 0,
                    (true, "lookahead") => 2,
                    _ => return Err(err(start, format!("unknown class definition \"{f}\""))),
                };
                let def = self.class_def()?;
                let target = defs.get_mut(slot).ok_or_else(|| err(start, "bad class slot"))?;
                if target.is_some() {
                    return Err(err(start, format!("a second \"{f}\"")));
                }
                *target = Some(def);
            }
            let mut rules = Vec::new();
            while let Some(line) = self.cur.next_in_block() {
                if !line.first().to_lowercase().starts_with("class") {
                    return Err(err(line.no, format!("expected a class rule, got \"{}\"", line.first())));
                }
                let mut seqs: [Vec<u16>; 3] = Default::default();
                for (k, slot) in slots.iter().enumerate() {
                    let classes = int_split(line.field(1 + k), line.no)?
                        .into_iter()
                        .map(|c| u16::try_from(c).map_err(|_| err(line.no, format!("class {c} is out of range"))))
                        .collect::<Result<Vec<_>>>()?;
                    if let Some(s) = seqs.get_mut(*slot) {
                        *s = classes;
                    }
                }
                let recs = self.records(line.fields.get(1 + data_len..).unwrap_or_default(), line.no)?;
                rules.push((seqs, recs, line.no));
            }
            return Ok(Rules::Class { defs, rules });
        }
        if typ.ends_with("coverage") {
            let mut coverages: [Vec<Vec<String>>; 3] = Default::default();
            while self.cur.peek_first().ends_with("coverage definition begin") {
                let f = self.cur.peek_first();
                let which = f.get(..f.len().saturating_sub("coverage definition begin".len())).unwrap_or_default().to_lowercase();
                let slot = match (chain, which.as_str()) {
                    (false, "") | (true, "input") => 1,
                    (true, "backtrack") => 0,
                    (true, "lookahead") => 2,
                    _ => return Err(err(start, format!("unknown coverage definition \"{f}\""))),
                };
                let cov = self.coverage()?;
                if let Some(list) = coverages.get_mut(slot) {
                    list.push(cov);
                }
            }
            let mut lines = Vec::new();
            while let Some(line) = self.cur.next_in_block() {
                lines.push(line);
            }
            let [line] = lines.as_slice() else { return Err(err(start, "a coverage-based rule has exactly one \"coverage\" line")) };
            if line.first().to_lowercase() != "coverage" {
                return Err(err(line.no, format!("expected a coverage rule, got \"{}\"", line.first())));
            }
            let records = self.records(line.fields.get(1..).unwrap_or_default(), line.no)?;
            return Ok(Rules::Coverage { coverages, records, line: line.no });
        }
        Err(err(start, format!("expected glyph, class or coverage rules, got \"{typ}\"")))
    }

    fn gdef(&mut self) -> Result<Gdef> {
        let mut g = Gdef::default();
        while let Some(line) = self.cur.peek().cloned() {
            let first = line.first().to_lowercase();
            let twice = || err(line.no, format!("a second \"{}\"", line.first()));
            match first.as_str() {
                "class definition begin" => {
                    if g.glyph_classes.is_some() {
                        return Err(twice());
                    }
                    g.glyph_classes = Some(self.class_def()?);
                }
                "mark attachment class definition begin" => {
                    if g.mark_attach.is_some() {
                        return Err(twice());
                    }
                    g.mark_attach = Some(self.class_def()?);
                }
                "attachment list begin" => {
                    if g.attach.is_some() {
                        return Err(twice());
                    }
                    self.cur.begin("attachment list")?;
                    let mut points: Vec<(String, Vec<i64>)> = Vec::new();
                    while let Some(l) = self.cur.next_in_block() {
                        let name = glyph(l.first(), l.no, self.order)?;
                        if points.iter().any(|(n, _)| *n == name) {
                            return Err(err(l.no, format!("{name} is listed twice")));
                        }
                        let p = l.fields.get(1..).unwrap_or_default().iter().map(|f| int(f, l.no)).collect::<Result<Vec<_>>>()?;
                        points.push((name, p));
                    }
                    self.cur.end("attachment list")?;
                    g.attach = Some(points);
                }
                "carets begin" => {
                    if g.carets.is_some() {
                        return Err(twice());
                    }
                    self.cur.begin("carets")?;
                    let mut carets: Vec<(String, Vec<i64>)> = Vec::new();
                    while let Some(l) = self.cur.next_in_block() {
                        let name = glyph(l.first(), l.no, self.order)?;
                        if carets.iter().any(|(n, _)| *n == name) {
                            return Err(err(l.no, format!("{name} is listed twice")));
                        }
                        let n = int(l.field(1), l.no)?;
                        let c = l.fields.get(2..).unwrap_or_default().iter().map(|f| int(f, l.no)).collect::<Result<Vec<_>>>()?;
                        if usize::try_from(n).ok() != Some(c.len()) {
                            return Err(err(l.no, format!("{name} announces {n} carets and lists {}", c.len())));
                        }
                        carets.push((name, c));
                    }
                    self.cur.end("carets")?;
                    g.carets = Some(carets);
                }
                "markfilter set definition begin" => {
                    if g.filter_sets.is_some() {
                        return Err(twice());
                    }
                    self.cur.begin("set definition")?;
                    let mut sets: Vec<(u32, Vec<String>)> = Vec::new();
                    while let Some(l) = self.cur.next_in_block() {
                        let [name, set] = l.fields.as_slice() else { return Err(err(l.no, "a mark filtering set line has two fields")) };
                        let name = glyph(name, l.no, self.order)?;
                        let set = u32::try_from(int(set, l.no)?).map_err(|_| err(l.no, "mark filtering set out of range"))?;
                        match sets.iter_mut().find(|(s, _)| *s == set) {
                            Some((_, list)) => list.push(name),
                            None => sets.push((set, vec![name])),
                        }
                    }
                    self.cur.end("set definition")?;
                    sets.sort_by_key(|(s, _)| *s);
                    g.filter_sets = Some(sets);
                }
                _ => self.cur.skip(),
            }
        }
        Ok(g)
    }
}

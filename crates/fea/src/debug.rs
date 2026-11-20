//! Lookup debug information: where each compiled lookup came from, for the `Debg` table that
//! fontTools feaLib writes when asked (`feaLib/builder.py`: `buildLookups_`, `makeTable`,
//! `buildDebg`; `ttLib/tables/D__e_b_g.py`; fontTools, MIT License).
//!
//! fea-rs 1.0.0 records a place and a name for each lookup it builds from the code
//! (`Opts::compile_debg`) and writes them as feaLib's JSON, with three differences that are
//! settled here:
//! - a named lookup is placed at its label; feaLib places it at its first rule, so the label is
//!   replaced by the first rule of the block of that name;
//! - columns are byte offsets from 0; feaLib counts characters from 1;
//! - the feature is always null; feaLib records the (script, language, feature) that comes last,
//!   in its feature sort order (feature tag, then language, then script), among the language
//!   systems whose features use the lookup. It is worked out here from the compiled tables.
//!
//! Lookups fea-rs gives no place (the `aalt` lookups, generated lookups) are written with the
//! place `"None"`, as feaLib writes its `aalt` lookups. An inline lookup of a contextual rule
//! takes the place of the contextual lookup that calls it, as fea-rs places it; feaLib places
//! it at the rule that created it, which differs when one contextual lookup has several rules.

use std::collections::{BTreeSet, HashMap};

use fea_rs::compile::Compilation;
use fea_rs::{Kind, Node, NodeOrToken, ParseTree};
use write_fonts_052::tables::layout::{FeatureList, ScriptList};

use crate::check::is_rule;
use crate::resolve::Loaded;
use crate::{Diagnostic, MAX_PATH_BYTES, Severity, Tag};

/// The key feaLib keeps its lookup records under in the `Debg` table's JSON.
pub const DEBG_KEY: &str = "com.github.fonttools.feaLib";

/// The most levels of the parse tree searched for lookup blocks and their first rules.
const MAX_SEARCH_DEPTH: usize = 1_024;

/// A place in one of the files of a compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePlace {
    /// The file, named as [`crate::Diagnostic::file`] names it.
    pub file: String,
    /// Line, from 1.
    pub line: usize,
    /// Column, from 1, in characters.
    pub column: usize,
}

/// Where one compiled lookup came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookupOrigin {
    /// The place of the rule that started the lookup; `None` for a lookup the code does not
    /// spell out (`aalt`, generated lookups).
    pub place: Option<SourcePlace>,
    /// The name of the lookup block it was built from.
    pub name: Option<String>,
    /// (script, language, feature): the last, in feaLib's order, of the language systems whose
    /// features list the lookup; `None` when no feature lists it (a lookup only rules call).
    pub feature: Option<(Tag, Tag, Tag)>,
}

/// The origins of the lookups of `GSUB` and `GPOS`, by lookup index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookupDebug {
    pub gsub: Vec<LookupOrigin>,
    pub gpos: Vec<LookupOrigin>,
}

impl LookupDebug {
    /// The `Debg` table as feaLib writes it: `{"com.github.fonttools.feaLib": {"GSUB": {…},
    /// "GPOS": {…}}}` in Python's default JSON layout, each lookup as `[place, name, feature]`.
    /// `file_name` gives the name written for each file of the compile.
    pub fn to_debg(&self, file_name: &dyn Fn(&str) -> String) -> Vec<u8> {
        let mut out = String::new();
        out.push('{');
        json_string(&mut out, DEBG_KEY);
        out.push_str(": {");
        for (n, (tag, origins)) in [("GSUB", &self.gsub), ("GPOS", &self.gpos)].into_iter().enumerate() {
            if n > 0 {
                out.push_str(", ");
            }
            json_string(&mut out, tag);
            out.push_str(": {");
            for (index, origin) in origins.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                json_string(&mut out, &index.to_string());
                out.push_str(": [");
                let place = match &origin.place {
                    Some(p) => format!("{}:{}:{}", file_name(&p.file), p.line, p.column),
                    None => "None".to_string(),
                };
                json_string(&mut out, &place);
                out.push_str(", ");
                match &origin.name {
                    Some(name) => json_string(&mut out, name),
                    None => out.push_str("null"),
                }
                out.push_str(", ");
                match origin.feature {
                    Some((script, language, feature)) => {
                        out.push('[');
                        json_string(&mut out, &tag_text(script));
                        out.push_str(", ");
                        json_string(&mut out, &tag_text(language));
                        out.push_str(", ");
                        json_string(&mut out, &tag_text(feature));
                        out.push(']');
                    }
                    None => out.push_str("null"),
                }
                out.push(']');
            }
            out.push('}');
        }
        out.push_str("}}");
        out.into_bytes()
    }
}

/// A tag as feaLib keeps it: four characters, trailing spaces included.
fn tag_text(tag: Tag) -> String {
    tag.into_bytes().iter().map(|&b| char::from(b)).collect()
}

/// `text` as a JSON string the way Python's `json.dumps` writes it by default: everything
/// outside printable ASCII escaped, as `\uXXXX` (lowercase, surrogate pairs above U+FFFF)
/// unless it has a short escape.
fn json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// How the places fea-rs writes relate to the files of the compile.
pub(crate) struct Positions<'a> {
    /// The text fea-rs compiled when it is not the files' own (the code written out in one
    /// file with lookup breaks); fea-rs's places are then in this text.
    pub(crate) rewritten: Option<&'a str>,
    /// The file and byte offset in it of a position in the compiled tree.
    pub(crate) locate: &'a dyn Fn(usize) -> Option<(String, usize)>,
}

/// One lookup as fea-rs writes it: the table (`true` for `GPOS`), the lookup index, the place
/// (file, line from 1, byte column from 0) and the name.
struct Raw {
    gpos: bool,
    index: usize,
    place: Option<(String, usize, usize)>,
    name: Option<String>,
}

/// The origins of the lookups `c` holds, from the places fea-rs recorded for them.
pub(crate) fn build(tree: &ParseTree, c: &Compilation, files: &Loaded, positions: &Positions<'_>) -> Result<LookupDebug, Diagnostic> {
    let unreadable = || Diagnostic::request(Severity::Error, "the compiler's lookup debug information could not be read");
    let raw = match &c.debg {
        Some(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(|_| unreadable())?;
            parse(text, files).ok_or_else(unreadable)?
        }
        None => Vec::new(),
    };
    let mut debug = LookupDebug {
        gsub: vec![LookupOrigin::default(); c.gsub.as_ref().map_or(0, |t| t.lookup_list.lookups.len())],
        gpos: vec![LookupOrigin::default(); c.gpos.as_ref().map_or(0, |t| t.lookup_list.lookups.len())],
    };

    let mut first_rules = HashMap::new();
    lookup_blocks(tree.root(), 0, &mut first_rules);
    // A named lookup moves from its label to its block's first rule. The inline lookups of a
    // named contextual lookup carry the label's place without the name, and move with it.
    let mut moved: HashMap<LabelPlace<'_>, Option<(String, usize)>> = HashMap::new();
    for entry in &raw {
        let first = entry.name.as_deref().and_then(|name| first_rules.get(name));
        if let (Some(&at), Some((file, line, column))) = (first, &entry.place) {
            moved.insert((entry.gpos, file.as_str(), *line, *column), (positions.locate)(at));
        }
    }
    let mut lines: HashMap<&str, Lines<'_>> = HashMap::new();
    let rewritten = positions.rewritten.map(Lines::new);
    let mut wanted: Vec<Option<(String, usize)>> = Vec::with_capacity(raw.len());
    for entry in &raw {
        let at = entry.place.as_ref().and_then(|(file, line, column)| match moved.get(&(entry.gpos, file.as_str(), *line, *column)) {
            Some(at) => at.clone(),
            None => match &rewritten {
                Some(text) => (positions.locate)(text.offset(*line, *column)?),
                None => {
                    let (name, text) = files.get_key_value(file.as_str())?;
                    let text = lines.entry(name.as_str()).or_insert_with(|| Lines::new(text));
                    Some((file.clone(), text.offset(*line, *column)?))
                }
            },
        });
        wanted.push(at);
    }
    let places = resolve_places(&wanted, files);
    for (entry, place) in raw.into_iter().zip(places) {
        let origins = if entry.gpos { &mut debug.gpos } else { &mut debug.gsub };
        if let Some(origin) = origins.get_mut(entry.index) {
            origin.place = place;
            origin.name = entry.name;
        }
    }

    if let Some(t) = &c.gsub {
        assign_features(&t.script_list, &t.feature_list, &mut debug.gsub);
    }
    if let Some(t) = &c.gpos {
        assign_features(&t.script_list, &t.feature_list, &mut debug.gpos);
    }
    Ok(debug)
}

/// The line starts of a text. A line ends at a line feed, or at a carriage return not followed
/// by one, as both compilers count lines.
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        for (i, &b) in bytes.iter().enumerate() {
            let ends = b == b'\n' || (b == b'\r' && bytes.get(i + 1) != Some(&b'\n'));
            if ends {
                starts.push(i + 1);
            }
        }
        Self { text, starts }
    }

    /// The byte offset of line `line` (from 1), byte column `column` (from 0).
    fn offset(&self, line: usize, column: usize) -> Option<usize> {
        let offset = self.starts.get(line.checked_sub(1)?)?.checked_add(column)?;
        self.text.is_char_boundary(offset).then_some(offset)
    }

    /// The line (from 1) holding byte `offset`, and the offset that line starts at.
    fn line(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        (line, self.starts.get(line - 1).copied().unwrap_or(0))
    }
}

/// The line and character column (both from 1) of each file and byte offset. The offsets of
/// each file are taken in order, so each line's characters are counted once.
fn resolve_places(wanted: &[Option<(String, usize)>], files: &Loaded) -> Vec<Option<SourcePlace>> {
    let mut out = vec![None; wanted.len()];
    let mut order: Vec<(&str, usize, usize)> =
        wanted.iter().enumerate().filter_map(|(i, w)| w.as_ref().map(|(file, offset)| (file.as_str(), *offset, i))).collect();
    order.sort_unstable();
    let mut current: Option<(&str, Lines<'_>)> = None;
    // The last place counted: its offset, line and column.
    let mut last: Option<(usize, usize, usize)> = None;
    for (file, offset, i) in order {
        if current.as_ref().is_none_or(|(name, _)| *name != file) {
            current = files.get(file).map(|text| (file, Lines::new(text)));
            last = None;
        }
        let Some((_, lines)) = &current else { continue };
        let (line, start) = lines.line(offset);
        let column = match last {
            Some((at, last_line, column)) if last_line == line && at <= offset => {
                lines.text.get(at..offset).map(|between| column + between.chars().count())
            }
            _ => lines.text.get(start..offset).map(|before| before.chars().count() + 1),
        };
        let Some(column) = column else { continue };
        last = Some((offset, line, column));
        if let Some(slot) = out.get_mut(i) {
            *slot = Some(SourcePlace { file: file.to_string(), line, column });
        }
    }
    out
}

/// The lookup blocks under `node`: each label with the position of the block's first rule.
fn lookup_blocks<'a>(node: &'a Node, depth: usize, out: &mut HashMap<&'a str, usize>) {
    if depth > MAX_SEARCH_DEPTH {
        return;
    }
    for child in node.iter_children().filter_map(NodeOrToken::as_node) {
        if child.kind() == Kind::LookupBlockNode {
            let label = child.iter_children().filter_map(NodeOrToken::as_token).find(|t| t.kind == Kind::Label);
            if let (Some(label), Some(rule)) = (label, first_rule(child, depth + 1)) {
                out.entry(label.as_str()).or_insert(rule);
            }
        } else if !is_rule(child.kind()) {
            lookup_blocks(child, depth + 1, out);
        }
    }
}

/// The position of the first rule under `node`.
fn first_rule(node: &Node, depth: usize) -> Option<usize> {
    if depth > MAX_SEARCH_DEPTH {
        return None;
    }
    node.iter_children()
        .filter_map(NodeOrToken::as_node)
        .find_map(|child| if is_rule(child.kind()) { Some(child.range().start) } else { first_rule(child, depth + 1) })
}

type LabelPlace<'a> = (bool, &'a str, usize, usize);

/// (feature tag, language tag, script tag, feature index).
type FeatureKey = ([u8; 4], [u8; 4], [u8; 4], usize);

/// Gives each lookup the language system and feature feaLib records for it: feaLib visits
/// its (script, language, feature) keys sorted by feature tag, then language, then script, and
/// each key overwrites the record of every lookup its feature lists.
fn assign_features(scripts: &ScriptList, features: &FeatureList, origins: &mut [LookupOrigin]) {
    let mut keys: BTreeSet<FeatureKey> = BTreeSet::new();
    for record in &scripts.script_records {
        let script = &record.script;
        let systems = script.default_lang_sys.as_ref().map(|d| (*b"dflt", d)).into_iter();
        let systems = systems.chain(script.lang_sys_records.iter().map(|l| (l.lang_sys_tag.into_bytes(), &*l.lang_sys)));
        for (language, system) in systems {
            let required = (system.required_feature_index != 0xFFFF).then_some(system.required_feature_index);
            for index in required.into_iter().chain(system.feature_indices.iter().copied()) {
                if let Some(feature) = features.feature_records.get(usize::from(index)) {
                    keys.insert((feature.feature_tag.into_bytes(), language, record.script_tag.into_bytes(), usize::from(index)));
                }
            }
        }
    }
    for (feature_tag, language, script, index) in keys {
        let Some(feature) = features.feature_records.get(index) else { continue };
        for &lookup in &feature.feature.lookup_list_indices {
            if let Some(origin) = origins.get_mut(usize::from(lookup)) {
                origin.feature = Some((Tag::new(&script), Tag::new(&language), Tag::new(&feature_tag)));
            }
        }
    }
}

/// fea-rs's `Debg` text: `{"KEY":{"GSUB":{"0":["file:line:col","name",null],…},"GPOS":{…}}}`,
/// with the file written unescaped. A place is matched against the files of the compile, so
/// a file name holding quotes or commas is still read whole. `None` when the text is not in
/// that form.
fn parse(text: &str, files: &Loaded) -> Option<Vec<Raw>> {
    let mut c = Cursor { text, at: 0 };
    c.eat(&format!("{{\"{DEBG_KEY}\":{{"))?;
    let mut out = Vec::new();
    let mut first_table = true;
    loop {
        if c.eat("}}").is_some() {
            return c.done().then_some(out);
        }
        if !first_table {
            c.eat(",")?;
        }
        first_table = false;
        let gpos = if c.eat("\"GSUB\":{").is_some() {
            false
        } else {
            c.eat("\"GPOS\":{")?;
            true
        };
        let mut first = true;
        while c.eat("}").is_none() {
            if !first {
                c.eat(",")?;
            }
            first = false;
            c.eat("\"")?;
            let index = c.number()?;
            c.eat("\":[")?;
            let place = if c.peek() == Some(b',') { None } else { Some(c.place(files)?) };
            c.eat(",")?;
            let name = if c.eat("null").is_some() {
                None
            } else {
                c.eat("\"")?;
                let name = c.until(b'"')?;
                c.eat("\"")?;
                Some(name.to_string())
            };
            c.eat(",null]")?;
            out.push(Raw { gpos, index, place, name });
        }
    }
}

struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or_default()
    }

    fn done(&self) -> bool {
        self.rest().is_empty()
    }

    fn peek(&self) -> Option<u8> {
        self.rest().as_bytes().first().copied()
    }

    fn eat(&mut self, literal: &str) -> Option<()> {
        self.rest().starts_with(literal).then(|| self.at += literal.len())
    }

    fn number(&mut self) -> Option<usize> {
        let digits = self.rest().bytes().take_while(u8::is_ascii_digit).count();
        let n = self.rest().get(..digits)?.parse().ok()?;
        self.at += digits;
        Some(n)
    }

    fn until(&mut self, end: u8) -> Option<&'a str> {
        let rest = self.rest();
        let len = rest.bytes().position(|b| b == end)?;
        self.at += len;
        rest.get(..len)
    }

    /// `"file:line:col"`, the file being one of `files`.
    fn place(&mut self, files: &Loaded) -> Option<(String, usize, usize)> {
        self.eat("\"")?;
        let rest = self.rest();
        // A file name is at most MAX_PATH_BYTES long; the line and column add a few digits.
        let mut end = rest.len().min(MAX_PATH_BYTES + 48);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let window = rest.get(..end).unwrap_or_default();
        for (q, _) in window.match_indices('"') {
            if rest.as_bytes().get(q + 1) != Some(&b',') {
                continue;
            }
            let Some(written) = rest.get(..q) else { continue };
            let Some((head, column)) = written.rsplit_once(':') else { continue };
            let Some((file, line)) = head.rsplit_once(':') else { continue };
            let (Ok(line), Ok(column)) = (line.parse::<usize>(), column.parse::<usize>()) else { continue };
            if files.contains_key(file) {
                self.at += q + 1;
                return Some((file.to_string(), line, column));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{CompileRequest, GlyphOrder, VirtualFiles, compile};

    /// The `Debg` text of a compile of `files` against `names`, each file named as it is keyed.
    fn debg(files: &VirtualFiles, names: Vec<String>) -> String {
        let order = GlyphOrder::new(names).unwrap();
        let compiled = compile(&CompileRequest { debug_lookups: true, ..CompileRequest::new(files, &order) }).unwrap();
        String::from_utf8(compiled.lookup_debug.unwrap().to_debg(&|file: &str| file.to_string())).unwrap()
    }

    /// What fontTools feaLib 4.66.1 writes with `debug=True` for five of its own test files,
    /// each compiled by its file name from its folder.
    const FEALIB: [(&str, &str); 5] = [
        (
            "duplicate_language_stmt.fea",
            r#"{"com.github.fonttools.feaLib": {"GSUB": {"0": ["duplicate_language_stmt.fea:9:9", "one", ["latn", "NLD ", "locl"]], "1": ["duplicate_language_stmt.fea:17:9", "two", ["latn", "NLD ", "locl"]]}, "GPOS": {}}}"#,
        ),
        (
            "multiple_feature_blocks.fea",
            r#"{"com.github.fonttools.feaLib": {"GSUB": {"0": ["multiple_feature_blocks.fea:7:9", "liga_fl", ["latn", "dflt", "liga"]], "1": ["multiple_feature_blocks.fea:13:9", "liga_fi", ["latn", "dflt", "liga"]]}, "GPOS": {}}}"#,
        ),
        (
            "PairPosSubtable.fea",
            r#"{"com.github.fonttools.feaLib": {"GSUB": {}, "GPOS": {"0": ["PairPosSubtable.fea:9:5", "kernlookup", ["latn", "dflt", "kern"]]}}}"#,
        ),
        (
            "script_language_tracking_DFLT.fea",
            r#"{"com.github.fonttools.feaLib": {"GSUB": {}, "GPOS": {"0": ["script_language_tracking_DFLT.fea:8:5", null, ["latn", "dflt", "kern"]], "1": ["script_language_tracking_DFLT.fea:11:5", null, ["latn", "dflt", "kern"]]}}}"#,
        ),
        ("spec6d2.fea", r#"{"com.github.fonttools.feaLib": {"GSUB": {}, "GPOS": {"0": ["spec6d2.fea:12:5", null, ["DFLT", "dflt", "test"]]}}}"#),
    ];

    #[test]
    fn lookup_debug_matches_fealib_on_its_test_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests/feaLib/data");
        if !dir.is_dir() {
            eprintln!("corpus missing; skipped");
            return;
        }
        for (name, expected) in FEALIB {
            let text = std::fs::read_to_string(dir.join(name)).unwrap();
            // Every word outside comments may be a glyph name; the glyph order does not change
            // the debug information.
            let mut names: Vec<String> = vec![".notdef".into()];
            for line in text.lines() {
                let code = line.split('#').next().unwrap_or_default();
                for word in code.split(|c: char| c.is_whitespace() || "[](){}<>;,'\"=@\\".contains(c)) {
                    if word.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '.') && !names.iter().any(|n| n == word) {
                        names.push(word.to_string());
                    }
                }
            }
            let files = VirtualFiles::new(name, &text).unwrap().with_include_dir(dir.clone());
            assert_eq!(debg(&files, names), expected, "{name}");
        }
    }

    #[test]
    fn named_lookups_are_placed_at_their_first_rule_in_the_file_that_holds_it() {
        let main = "languagesystem DFLT dflt;
languagesystem latn dflt;
languagesystem latn TRK;

include(classes.fea)

feature liga {
    lookup FI {
        lookupflag IgnoreMarks;
        sub f i by f_i;
    } FI;
    sub f l by f_l;
} liga;

lookup KERN {
    pos a b -10;
} KERN;

feature kern {
    script latn;
    language TRK exclude_dflt;
    lookup KERN;
} kern;

feature ss01 { featureNames { name \"Ünïcødé\"; }; sub a by a.alt; } ss01;
";
        let classes = "@UC = [A B];\nlookup CALLED {\n  sub a by b;\n} CALLED;\nfeature smcp {\n  sub @UC by [a b];\n} smcp;\n";
        let mut files = VirtualFiles::new("main.fea", main).unwrap();
        files.insert("classes.fea", classes).unwrap();
        let names = ".notdef a b f i l A B f_i f_l a.alt".split(' ').map(String::from).collect();
        // fontTools feaLib 4.66.1, debug=True: a named lookup at its first rule, columns in
        // characters, the feature that sorts last, a lookup no feature lists without one.
        let expected = r#"{"com.github.fonttools.feaLib": {"GSUB": {"0": ["classes.fea:3:3", "CALLED", null], "1": ["classes.fea:6:3", null, ["latn", "dflt", "smcp"]], "2": ["main.fea:10:9", "FI", ["latn", "dflt", "liga"]], "3": ["main.fea:12:5", null, ["latn", "dflt", "liga"]], "4": ["main.fea:25:50", null, ["latn", "dflt", "ss01"]]}, "GPOS": {"0": ["main.fea:16:5", "KERN", ["latn", "TRK ", "kern"]]}}}"#;
        assert_eq!(debg(&files, names), expected);
    }

    #[test]
    fn aalt_lookups_have_no_place_and_no_debug_info_is_kept_unless_asked() {
        let files = VirtualFiles::new("f.fea", "feature aalt { feature salt; } aalt;\nfeature salt { sub a by b; } salt;\n").unwrap();
        let order = GlyphOrder::new(vec![".notdef".into(), "a".into(), "b".into()]).unwrap();
        assert_eq!(compile(&CompileRequest::new(&files, &order)).unwrap().lookup_debug, None);
        let debug = compile(&CompileRequest { debug_lookups: true, ..CompileRequest::new(&files, &order) }).unwrap().lookup_debug.unwrap();
        let salt = debug.gsub.iter().find(|o| o.feature.is_some_and(|f| f.2 == Tag::new(b"salt"))).unwrap();
        assert_eq!(salt.place, Some(SourcePlace { file: "f.fea".into(), line: 2, column: 16 }));
        let aalt = debug.gsub.iter().find(|o| o.feature.is_some_and(|f| f.2 == Tag::new(b"aalt"))).unwrap();
        assert_eq!(aalt.place, None);
        assert!(String::from_utf8(debug.to_debg(&|f: &str| f.to_string())).unwrap().contains("[\"None\", null, [\"DFLT\", \"dflt\", \"aalt\"]]"));
    }

    #[test]
    fn json_strings_are_escaped_as_python_writes_them() {
        let mut out = String::new();
        json_string(&mut out, "a\"b\\c\nd\u{1}é😀\u{7f}");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001\\u00e9\\ud83d\\ude00\\u007f\"");
    }

    #[test]
    fn lines_end_at_line_feeds_carriage_returns_and_both() {
        let text = "ab\r\ncd\ref\ngé h";
        let lines = Lines::new(text);
        assert_eq!(lines.offset(1, 1), Some(1));
        assert_eq!(lines.offset(2, 0), Some(4));
        assert_eq!(lines.offset(3, 1), Some(8));
        assert_eq!(lines.offset(4, 4), Some(14));
        assert_eq!(lines.offset(9, 0), None);
        assert_eq!(lines.offset(4, 2), None, "inside `é`");
        let files: Loaded = std::sync::Arc::new([("f.fea".to_string(), std::sync::Arc::<str>::from(text))].into_iter().collect());
        let at = |offset: usize| Some(("f.fea".to_string(), offset));
        let places = resolve_places(&[at(14), at(4), None, at(8), at(11), at(99)], &files);
        let found: Vec<Option<(usize, usize)>> = places.iter().map(|p| p.as_ref().map(|p| (p.line, p.column))).collect();
        // Columns count characters: `é` is two bytes and one column.
        assert_eq!(found, [Some((4, 4)), Some((2, 1)), None, Some((3, 2)), Some((4, 2)), None]);
    }

    #[test]
    fn places_are_read_against_the_known_files() {
        let files: Loaded = std::sync::Arc::new(
            [("a\",b.fea".to_string(), std::sync::Arc::<str>::from("")), ("x.fea".to_string(), std::sync::Arc::<str>::from(""))]
                .into_iter()
                .collect(),
        );
        let text = format!(
            "{{\"{DEBG_KEY}\":{{\"GSUB\":{{\"0\":[\"a\",b.fea:3:4\",\"one\",null],\"2\":[,null,null]}},\"GPOS\":{{\"1\":[\"x.fea:1:0\",null,null]}}}}}}"
        );
        let raw = parse(&text, &files).unwrap();
        assert_eq!(raw.len(), 3);
        assert_eq!(raw[0].place, Some(("a\",b.fea".to_string(), 3, 4)));
        assert_eq!(raw[0].name.as_deref(), Some("one"));
        assert_eq!((raw[1].index, raw[1].place.is_none()), (2, true));
        assert!(raw[2].gpos && raw[2].index == 1);
        assert!(parse("{\"other\":{}}", &files).is_none());
        assert!(parse(&format!("{{\"{DEBG_KEY}\":{{\"GSUB\":{{\"0\":[\"y.fea:1:0\",null,null]}}}}}}"), &files).is_none(), "an unknown file");
    }
}

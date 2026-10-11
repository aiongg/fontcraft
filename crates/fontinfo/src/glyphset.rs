//! The values of the glyph-set parameters: Remove Glyphs, Keep Glyphs and Export Glyphs (one entry
//! per line: a glyph name, a name pattern with `*`, or a `key=value` query), Rename Glyphs
//! (`old=new`) and Reencode Glyphs (`glyph=HEX[,HEX…]`). The grammar and the query keys are in
//! `docs/spec/18-custom-parameters.md` › Remove Glyphs and Keep Glyphs.
//!
//! Patterns are matched without backtracking, with at most [`crate::MAX_PATTERN_WILDCARDS`] `*`.
//! A value holds at most [`crate::MAX_GLYPHSET_LINES`] lines of
//! [`crate::MAX_GLYPHSET_LINE_BYTES`] each. Blank lines are skipped.
//!
//! An entry with `=` is a query. Keys are case-sensitive, as the handbook spells them; an unknown
//! key is read as an entry that matches nothing ([`GlyphPattern::unknown_key`]), so the export can
//! warn about it. Text values compare exactly except for `*`; a glyph without the property
//! compares as empty text, so `note=` matches glyphs without a note. `unicode` compares the first
//! code point as hexadecimal text of at least four digits, in either case.
//!
//! The compiler (`compile/src/export_params.rs`) matches resolved glyphs; the engine may match
//! source glyphs. Both describe a glyph through [`GlyphFacts`].
//!
//! Rows: `Export Glyphs`, `Keep Glyphs`, `Remove Glyphs`, `Rename Glyphs`, `Reencode Glyphs` (M7.9).

use crate::error::PatternError;

/// Which metrics key a query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricsSide {
    Left,
    Right,
    Width,
    Top,
    Bottom,
    VertWidth,
}

/// Which kerning group a query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// What a query may ask of a glyph, one method per query key.
pub trait GlyphFacts {
    fn name(&self) -> &str;
    /// The code points, first one first (`unicode=` matches the first).
    fn code_points(&self) -> &[u32];
    fn note(&self) -> Option<&str>;
    /// The script as Glyph Info shows it (`latin`, `greek`).
    fn script(&self) -> Option<&str>;
    fn category(&self) -> Option<&str>;
    fn sub_category(&self) -> Option<&str>;
    fn production_name(&self) -> Option<&str>;
    fn metrics_key(&self, side: MetricsSide) -> Option<&str>;
    fn kerning_group(&self, side: KernSide) -> Option<&str>;
    /// Every layer, master layers included.
    fn layer_count(&self) -> usize;
    /// Whether the glyph interpolates across all masters.
    fn masters_compatible(&self) -> bool;
    fn exports(&self) -> bool;
    /// Whether some colour layer uses palette entry `index`.
    fn uses_color_index(&self, index: i64) -> bool;
    /// Whether the glyph has colour data of any kind (palette layers, bitmap colour, full colour).
    fn is_any_color_glyph(&self) -> bool;
    /// Whether the glyph has a bitmap colour layer.
    fn is_apple_color_glyph(&self) -> bool;
    /// Whether the glyph has an alternate (bracket) or intermediate (brace) layer.
    fn has_special_layers(&self) -> bool;
}

/// A text pattern: literal text with `*` standing for any run of characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wildcard {
    /// The text between the `*`s; one part means no `*`.
    parts: Vec<String>,
}

impl Wildcard {
    fn new(text: &str) -> Self {
        Self { parts: text.split('*').map(str::to_string).collect() }
    }

    /// The text itself when it has no `*`.
    pub fn exact(&self) -> Option<&str> {
        match self.parts.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }

    /// Whether `text` matches. The first part is a prefix, the last a suffix, and each part
    /// between is found leftmost after the one before: for `*` alone that choice never misses a
    /// match, so nothing is retried.
    pub fn matches(&self, text: &str) -> bool {
        let Some((first, rest)) = self.parts.split_first() else { return text.is_empty() };
        let Some((last, middle)) = rest.split_last() else { return text == first };
        let Some(mut tail) = text.strip_prefix(first.as_str()) else { return false };
        for part in middle {
            let Some(at) = tail.find(part.as_str()) else { return false };
            tail = tail.get(at + part.len()..).unwrap_or("");
        }
        tail.ends_with(last.as_str())
    }
}

/// A query key whose value is text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKey {
    Name,
    /// The first code point as hexadecimal text.
    Unicode,
    Note,
    Script,
    Category,
    SubCategory,
    Production,
    Metrics(MetricsSide),
    Kerning(KernSide),
}

/// A query key whose value is `0` or `1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagKey {
    MastersCompatible,
    Export,
    IsAnyColorGlyph,
    IsAppleColorGlyph,
    HasSpecialLayers,
}

/// What one entry asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A glyph name, or a name pattern with `*`.
    Name(Wildcard),
    /// `key=value` on a text property.
    Text(TextKey, Wildcard),
    /// `colorIndex=N`: glyphs with a colour layer of palette entry N.
    ColorIndex(i64),
    /// `countOfLayers=N`.
    CountOfLayers(usize),
    /// `key=0` or `key=1`.
    Flag(FlagKey, bool),
    /// `key=value` with a key the grammar does not know; matches nothing.
    UnknownKey(String),
}

/// One entry of Remove, Keep or Export Glyphs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphPattern {
    line: String,
    entry: Entry,
}

impl GlyphPattern {
    /// Reads one entry (already trimmed, not blank). `line` in errors is 1.
    pub fn parse(line: &str) -> Result<Self, PatternError> {
        if line.len() > crate::MAX_GLYPHSET_LINE_BYTES {
            return Err(PatternError::LimitExceeded { what: "bytes in one line", limit: crate::MAX_GLYPHSET_LINE_BYTES });
        }
        let text = line.trim();
        if text.is_empty() {
            return Err(syntax(1, "an entry is empty"));
        }
        if text.matches('*').count() > crate::MAX_PATTERN_WILDCARDS {
            return Err(PatternError::TooManyWildcards { line: 1, limit: crate::MAX_PATTERN_WILDCARDS });
        }
        let entry = match text.split_once('=') {
            None => Entry::Name(Wildcard::new(text)),
            Some((key, value)) => query(key.trim(), value.trim())?,
        };
        Ok(Self { line: text.to_string(), entry })
    }

    /// The entry as written.
    pub fn text(&self) -> &str {
        &self.line
    }

    /// What the entry asks.
    pub fn entry(&self) -> &Entry {
        &self.entry
    }

    /// The glyph name, when the entry is a name without `*` (it matches only that glyph).
    pub fn exact_name(&self) -> Option<&str> {
        match &self.entry {
            Entry::Name(w) => w.exact(),
            _ => None,
        }
    }

    /// The key of a query the grammar does not know.
    pub fn unknown_key(&self) -> Option<&str> {
        match &self.entry {
            Entry::UnknownKey(key) => Some(key),
            _ => None,
        }
    }

    /// Whether the glyph matches.
    pub fn matches(&self, glyph: &dyn GlyphFacts) -> bool {
        match &self.entry {
            Entry::Name(w) => w.matches(glyph.name()),
            Entry::Text(key, w) => match key {
                TextKey::Name => w.matches(glyph.name()),
                TextKey::Unicode => match glyph.code_points().first() {
                    Some(cp) => w.matches(&format!("{cp:04X}")),
                    None => w.matches(""),
                },
                TextKey::Note => w.matches(glyph.note().unwrap_or("")),
                TextKey::Script => w.matches(glyph.script().unwrap_or("")),
                TextKey::Category => w.matches(glyph.category().unwrap_or("")),
                TextKey::SubCategory => w.matches(glyph.sub_category().unwrap_or("")),
                TextKey::Production => w.matches(glyph.production_name().unwrap_or("")),
                TextKey::Metrics(side) => w.matches(glyph.metrics_key(*side).unwrap_or("")),
                TextKey::Kerning(side) => w.matches(glyph.kerning_group(*side).unwrap_or("")),
            },
            Entry::ColorIndex(i) => glyph.uses_color_index(*i),
            Entry::CountOfLayers(n) => glyph.layer_count() == *n,
            Entry::Flag(key, want) => {
                let have = match key {
                    FlagKey::MastersCompatible => glyph.masters_compatible(),
                    FlagKey::Export => glyph.exports(),
                    FlagKey::IsAnyColorGlyph => glyph.is_any_color_glyph(),
                    FlagKey::IsAppleColorGlyph => glyph.is_apple_color_glyph(),
                    FlagKey::HasSpecialLayers => glyph.has_special_layers(),
                };
                have == *want
            }
            Entry::UnknownKey(_) => false,
        }
    }
}

fn syntax(line: usize, message: impl Into<String>) -> PatternError {
    PatternError::Syntax { line, message: message.into() }
}

/// A `key=value` entry.
fn query(key: &str, value: &str) -> Result<Entry, PatternError> {
    let text = |k: TextKey| -> Result<Entry, PatternError> { Ok(Entry::Text(k, Wildcard::new(value))) };
    match key {
        "name" => text(TextKey::Name),
        "unicode" => Ok(Entry::Text(TextKey::Unicode, Wildcard::new(&value.to_ascii_uppercase()))),
        "note" => text(TextKey::Note),
        "script" => text(TextKey::Script),
        "category" => text(TextKey::Category),
        "subCategory" => text(TextKey::SubCategory),
        "production" => text(TextKey::Production),
        "leftMetricsKey" => text(TextKey::Metrics(MetricsSide::Left)),
        "rightMetricsKey" => text(TextKey::Metrics(MetricsSide::Right)),
        "widthMetricsKey" => text(TextKey::Metrics(MetricsSide::Width)),
        "topMetricsKey" => text(TextKey::Metrics(MetricsSide::Top)),
        "bottomMetricsKey" => text(TextKey::Metrics(MetricsSide::Bottom)),
        "vertWidthMetricsKey" => text(TextKey::Metrics(MetricsSide::VertWidth)),
        "leftKerningGroup" => text(TextKey::Kerning(KernSide::Left)),
        "rightKerningGroup" => text(TextKey::Kerning(KernSide::Right)),
        "topKerningGroup" => text(TextKey::Kerning(KernSide::Top)),
        "bottomKerningGroup" => text(TextKey::Kerning(KernSide::Bottom)),
        "colorIndex" => value
            .parse::<i64>()
            .ok()
            .filter(|i| (0..=i64::from(u16::MAX)).contains(i))
            .map(Entry::ColorIndex)
            .ok_or_else(|| syntax(1, format!("`colorIndex` takes a palette index from 0 to 65535, not `{value}`"))),
        "countOfLayers" => {
            value.parse::<usize>().map(Entry::CountOfLayers).map_err(|_| syntax(1, format!("`countOfLayers` takes a whole number, not `{value}`")))
        }
        "mastersCompatible" => flag(key, value, FlagKey::MastersCompatible),
        "export" => flag(key, value, FlagKey::Export),
        "isAnyColorGlyph" => flag(key, value, FlagKey::IsAnyColorGlyph),
        "isAppleColorGlyph" => flag(key, value, FlagKey::IsAppleColorGlyph),
        "hasSpecialLayers" => flag(key, value, FlagKey::HasSpecialLayers),
        "" => Err(syntax(1, "a query needs a key before `=`")),
        other => Ok(Entry::UnknownKey(other.to_string())),
    }
}

fn flag(key: &str, value: &str, k: FlagKey) -> Result<Entry, PatternError> {
    match value {
        "0" => Ok(Entry::Flag(k, false)),
        "1" => Ok(Entry::Flag(k, true)),
        _ => Err(syntax(1, format!("`{key}` takes 0 or 1, not `{value}`"))),
    }
}

/// A whole Remove, Keep or Export Glyphs value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphList {
    pub patterns: Vec<GlyphPattern>,
}

impl GlyphList {
    /// Reads every line (a list value's items, or the lines of a text). Errors name the line.
    pub fn parse(lines: &[&str]) -> Result<Self, PatternError> {
        if lines.len() > crate::MAX_GLYPHSET_LINES {
            return Err(PatternError::LimitExceeded { what: "lines", limit: crate::MAX_GLYPHSET_LINES });
        }
        let mut patterns = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            patterns.push(GlyphPattern::parse(line).map_err(|e| renumber(e, i + 1))?);
        }
        Ok(Self { patterns })
    }

    /// Whether any entry matches the glyph.
    pub fn matches(&self, glyph: &dyn GlyphFacts) -> bool {
        self.patterns.iter().any(|p| p.matches(glyph))
    }
}

/// One `left=right` line, both sides trimmed; `right` may be empty only when `empty_right`.
fn assignment<'a>(line: &'a str, number: usize, empty_right: bool) -> Result<(&'a str, &'a str), PatternError> {
    if line.len() > crate::MAX_GLYPHSET_LINE_BYTES {
        return Err(PatternError::LimitExceeded { what: "bytes in one line", limit: crate::MAX_GLYPHSET_LINE_BYTES });
    }
    let Some((left, right)) = line.split_once('=') else { return Err(syntax(number, format!("`{line}` has no `=`"))) };
    let (left, right) = (left.trim(), right.trim());
    if left.is_empty() {
        return Err(syntax(number, "a glyph name is missing before `=`"));
    }
    if right.is_empty() && !empty_right {
        return Err(syntax(number, "a glyph name is missing after `=`"));
    }
    if left.chars().any(char::is_whitespace) {
        return Err(syntax(number, format!("`{left}` is not a glyph name")));
    }
    Ok((left, right))
}

/// A Rename Glyphs value: (old, new) pairs in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameList {
    pub pairs: Vec<(String, String)>,
}

impl RenameList {
    /// Reads `old=new` lines; blank lines are skipped.
    pub fn parse(lines: &[&str]) -> Result<Self, PatternError> {
        if lines.len() > crate::MAX_GLYPHSET_LINES {
            return Err(PatternError::LimitExceeded { what: "lines", limit: crate::MAX_GLYPHSET_LINES });
        }
        let mut pairs = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (old, new) = assignment(line, i + 1, false)?;
            if new.chars().any(char::is_whitespace) || new.contains('=') {
                return Err(syntax(i + 1, format!("`{new}` is not a glyph name")));
            }
            pairs.push((old.to_string(), new.to_string()));
        }
        Ok(Self { pairs })
    }
}

/// A Reencode Glyphs value: (glyph, code points) in order; no code points clears the glyph's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReencodeList {
    pub entries: Vec<(String, Vec<u32>)>,
}

impl ReencodeList {
    /// Reads `glyph=HEX[,HEX…]` lines; blank lines are skipped.
    pub fn parse(lines: &[&str]) -> Result<Self, PatternError> {
        if lines.len() > crate::MAX_GLYPHSET_LINES {
            return Err(PatternError::LimitExceeded { what: "lines", limit: crate::MAX_GLYPHSET_LINES });
        }
        let mut entries = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (name, right) = assignment(line, i + 1, true)?;
            let mut code_points = Vec::new();
            if !right.is_empty() {
                for item in right.split(',') {
                    let item = item.trim();
                    let digits = if (1..=6).contains(&item.len()) { u32::from_str_radix(item, 16).ok() } else { None };
                    let cp = digits
                        .filter(|cp| char::from_u32(*cp).is_some())
                        .ok_or_else(|| syntax(i + 1, format!("`{item}` is not a Unicode code point in hexadecimal")))?;
                    if !code_points.contains(&cp) {
                        code_points.push(cp);
                    }
                }
            }
            entries.push((name.to_string(), code_points));
        }
        Ok(Self { entries })
    }
}

/// An error from one line, with its line number in the whole value.
fn renumber(e: PatternError, line: usize) -> PatternError {
    match e {
        PatternError::Syntax { message, .. } => PatternError::Syntax { line, message },
        PatternError::TooManyWildcards { limit, .. } => PatternError::TooManyWildcards { line, limit },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct G {
        name: &'static str,
        cps: Vec<u32>,
        note: Option<&'static str>,
        category: Option<&'static str>,
        width_key: Option<&'static str>,
        layers: usize,
        export: bool,
    }

    impl GlyphFacts for G {
        fn name(&self) -> &str {
            self.name
        }
        fn code_points(&self) -> &[u32] {
            &self.cps
        }
        fn note(&self) -> Option<&str> {
            self.note
        }
        fn script(&self) -> Option<&str> {
            Some("latin")
        }
        fn category(&self) -> Option<&str> {
            self.category
        }
        fn sub_category(&self) -> Option<&str> {
            None
        }
        fn production_name(&self) -> Option<&str> {
            None
        }
        fn metrics_key(&self, side: MetricsSide) -> Option<&str> {
            if side == MetricsSide::Width { self.width_key } else { None }
        }
        fn kerning_group(&self, _: KernSide) -> Option<&str> {
            None
        }
        fn layer_count(&self) -> usize {
            self.layers
        }
        fn masters_compatible(&self) -> bool {
            true
        }
        fn exports(&self) -> bool {
            self.export
        }
        fn uses_color_index(&self, _: i64) -> bool {
            false
        }
        fn is_any_color_glyph(&self) -> bool {
            false
        }
        fn is_apple_color_glyph(&self) -> bool {
            false
        }
        fn has_special_layers(&self) -> bool {
            false
        }
    }

    fn glyph(name: &'static str) -> G {
        G { name, layers: 1, export: true, ..Default::default() }
    }

    fn matches(entry: &str, g: &G) -> bool {
        GlyphPattern::parse(entry).unwrap().matches(g)
    }

    #[test]
    fn names_wildcards_and_queries_match_as_the_handbook_lists_them() {
        for name in ["Aogonek", "aogonek", "Eogonek"] {
            assert!(matches("*ogonek", &glyph(name)), "{name}");
        }
        assert!(!matches("*ogonek", &glyph("ogonekcomb")));
        for name in ["K", "Kcommaaccent", "K.ss01"] {
            assert!(matches("K*", &glyph(name)), "{name}");
        }
        for name in ["H.ss01", "Hbar.ss01", "Hcircumflex.ss01"] {
            assert!(matches("H*.ss01", &glyph(name)), "{name}");
        }
        assert!(!matches("H*.ss01", &glyph("H.ss02")));
        // The suffix may not reuse text the prefix took.
        assert!(!matches("ab*ba", &glyph("aba")));
        assert!(matches("a*b*c", &glyph("aXbYbZc")));
        assert!(matches("A", &glyph("A")) && !matches("A", &glyph("Aacute")));

        let mut e = glyph("uni1E00");
        e.cps = vec![0x1E3F, 0x41];
        assert!(matches("unicode=1E*", &e) && matches("unicode=1e3f", &e));
        e.cps = vec![0x41, 0x1E3F];
        assert!(!matches("unicode=1E*", &e), "only the first code point counts");
        assert!(matches("unicode=", &glyph("a.alt")), "no code point compares as empty text");

        let mut n = glyph("x");
        n.note = Some("a draft to delete");
        n.category = Some("Mark");
        n.width_key = Some("=zero.tosf");
        n.layers = 4;
        n.export = false;
        for entry in ["note=*delete*", "category=Mark", "widthMetricsKey=*.tosf*", "countOfLayers=4", "export=0", "script=latin", "name=x"] {
            assert!(matches(entry, &n), "{entry}");
        }
        for entry in ["category=mark", "countOfLayers=3", "export=1", "script=Latin", "subCategory=*x*"] {
            assert!(!matches(entry, &n), "{entry}");
        }
        assert!(matches("subCategory=", &n));
    }

    #[test]
    fn bad_entries_are_refused_with_their_line_and_unknown_keys_match_nothing() {
        let p = GlyphPattern::parse("weight=bold").unwrap();
        assert_eq!(p.unknown_key(), Some("weight"));
        assert!(!p.matches(&glyph("weight=bold")));
        assert_eq!(GlyphPattern::parse("A").unwrap().exact_name(), Some("A"));
        assert_eq!(GlyphPattern::parse("A*").unwrap().exact_name(), None);
        for bad in ["export=2", "countOfLayers=-1", "colorIndex=x", "colorIndex=70000", "=x", "hasSpecialLayers=yes"] {
            assert!(matches!(GlyphPattern::parse(bad), Err(PatternError::Syntax { line: 1, .. })), "{bad}");
        }
        let err = GlyphList::parse(&["A", "", "export=5"]).unwrap_err();
        assert!(matches!(err, PatternError::Syntax { line: 3, .. }), "{err:?}");
        assert_eq!(GlyphList::parse(&["A", " ", "b*"]).unwrap().patterns.len(), 2);
    }

    #[test]
    fn hostile_lists_are_refused_quickly() {
        let lines = vec!["A"; 100_000];
        assert!(matches!(GlyphList::parse(&lines), Err(PatternError::LimitExceeded { what: "lines", .. })));
        let stars = "*".repeat(10_000);
        assert!(matches!(GlyphPattern::parse(&stars), Err(PatternError::LimitExceeded { .. })));
        let stars = "*".repeat(1_000);
        assert!(matches!(GlyphPattern::parse(&stars), Err(PatternError::TooManyWildcards { .. })));
        let sixteen = format!("a{}", "*a".repeat(16));
        let p = GlyphPattern::parse(&sixteen).unwrap();
        let long = "a".repeat(1_000);
        let started = std::time::Instant::now();
        assert!(p.matches(&glyph(Box::leak(long.into_boxed_str()))));
        assert!(!p.matches(&glyph(Box::leak("ab".repeat(500).into_boxed_str()))));
        assert!(started.elapsed().as_millis() < 100);
        let err = GlyphList::parse(&["A", &format!("{}x", "*".repeat(17))]).unwrap_err();
        assert!(matches!(err, PatternError::TooManyWildcards { line: 2, .. }), "{err:?}");
    }

    #[test]
    fn rename_and_reencode_lists_read_their_lines() {
        let r = RenameList::parse(&["g.single=g", "", " dollar = dollar.alt "]).unwrap();
        assert_eq!(r.pairs, vec![("g.single".into(), "g".into()), ("dollar".into(), "dollar.alt".into())]);
        for bad in ["g.single", "=g", "g=", "a=b=c", "e.bold=e, g.alt=g"] {
            assert!(matches!(RenameList::parse(&[bad]), Err(PatternError::Syntax { line: 1, .. })), "{bad}");
        }

        let e = ReencodeList::parse(&["a=0061,0251", "space=", "hyphen=002D, 2010, 2D"]).unwrap();
        assert_eq!(e.entries, vec![("a".into(), vec![0x61, 0x251]), ("space".into(), vec![]), ("hyphen".into(), vec![0x2D, 0x2010])]);
        for bad in ["a=D800", "a=110000", "a=0061,", "a=xyz", "a=1234567", "a"] {
            assert!(matches!(ReencodeList::parse(&["", bad]), Err(PatternError::Syntax { line: 2, .. })), "{bad}");
        }
        let long = format!("a={}", "0061,".repeat(300));
        assert!(matches!(ReencodeList::parse(&[&long]), Err(PatternError::LimitExceeded { .. })));
    }
}

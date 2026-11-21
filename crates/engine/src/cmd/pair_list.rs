//! Kerning pair lists: the plain-text work lists that the Pairs and Phrases section of the
//! Palette steps through (`kerning.loadPairList`, `kerning.stepPairList`; the shell commands live
//! in `fontcraft_ui::shell::kerning_review`). This module reads the files and holds the
//! built-in lists; it registers no command.
//!
//! # Format
//! One row per line. A row is a pair or a phrase: words separated by spaces or tabs, each a glyph
//! name, an `@group` or plain characters (the shell decides which, against the font). The two
//! characters `\n` inside a row break it into lines, which the shell shows one at a time. Blank
//! lines and lines starting with `#` or `%%` are skipped. Text must be UTF-8; a byte-order mark
//! is skipped.

use std::io::Read;

use super::CommandSpec;

/// Longest pair list file, in bytes.
pub const MAX_PAIR_LIST_BYTES: usize = 4 * 1024 * 1024;
/// Most lines a pair list may have, skipped lines included.
pub const MAX_PAIR_LIST_LINES: usize = 100_000;
/// Most words in one row, over all its lines.
pub const MAX_ROW_WORDS: usize = 1_000;
/// Most lines one row breaks into with `\n`.
pub const MAX_ROW_LINES: usize = 100;

/// The built-in pairs list: capital and lowercase pairs that usually need kerning in Latin.
pub const BUILTIN_PAIRS: &str = "\
A V\nA W\nA Y\nA T\nA v\nA y\nF A\nF a\nL T\nL V\nL Y\nP A\nP a\nT a\nT e\nT o\nT r\nV a\nV e\nV o\n\
W a\nW o\nY a\nY o\nr a\nv a\ny o\nf f\nL quoteright\nquoteleft A\n";

/// The built-in phrases list: spacing strings of straight and round letters.
pub const BUILTIN_PHRASES: &str = "\
nnonnonon\nHHOHHOHOH\nnnannbnncnndnnennfnngnn\nHHAHHBHHCHHDHHEHHFHHGHH\nminimum\nHamburgefonstiv\nHAMBURGEFONSTIV\n";

/// One row of a pair list: its line in the file (from 1) and its words, line by line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PairListRow {
    pub line: usize,
    pub lines: Vec<Vec<String>>,
}

impl PairListRow {
    /// Whether the row is written as two words on one line, the form of a pair.
    pub fn is_pair(&self) -> bool {
        matches!(self.lines.as_slice(), [only] if only.len() == 2)
    }

    /// The row as it reads in a list: its lines joined by ` \n `.
    pub fn label(&self) -> String {
        self.lines.iter().map(|l| l.join(" ")).collect::<Vec<_>>().join(" \\n ")
    }
}

/// Reads a pair list file, refusing one over [`MAX_PAIR_LIST_BYTES`] without reading the rest.
pub fn read_pair_list_file(path: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open {path}: {e}"))?;
    let mut bytes = Vec::new();
    let cap = u64::try_from(MAX_PAIR_LIST_BYTES).unwrap_or(u64::MAX).saturating_add(1);
    file.take(cap).read_to_end(&mut bytes).map_err(|e| format!("cannot read {path}: {e}"))?;
    if bytes.len() > MAX_PAIR_LIST_BYTES {
        return Err(format!("{path} is longer than {MAX_PAIR_LIST_BYTES} bytes; a pair list may hold at most that"));
    }
    Ok(bytes)
}

/// Parses a pair list. An empty list (nothing but skipped lines) is an error, so loading one
/// never leaves the section without rows.
pub fn parse_pair_list(bytes: &[u8]) -> Result<Vec<PairListRow>, String> {
    if bytes.len() > MAX_PAIR_LIST_BYTES {
        return Err(format!("the pair list is {} bytes long; at most {MAX_PAIR_LIST_BYTES} are read", bytes.len()));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| {
        let line = bytes.get(..e.valid_up_to()).map_or(1, |ok| ok.iter().filter(|b| **b == b'\n').count() + 1);
        format!("the pair list is not UTF-8 text (line {line})")
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let number = i + 1;
        if number > MAX_PAIR_LIST_LINES {
            return Err(format!("the pair list has more than {MAX_PAIR_LIST_LINES} lines"));
        }
        let raw = raw.strip_suffix('\r').unwrap_or(raw).trim();
        if raw.is_empty() || raw.starts_with('#') || raw.starts_with("%%") {
            continue;
        }
        let mut lines = Vec::new();
        let mut words = 0usize;
        for part in raw.split("\\n") {
            let line: Vec<String> = part.split_whitespace().map(str::to_string).collect();
            if line.is_empty() {
                continue;
            }
            words = words.saturating_add(line.len());
            if words > MAX_ROW_WORDS {
                return Err(format!("line {number} has more than {MAX_ROW_WORDS} words"));
            }
            lines.push(line);
            if lines.len() > MAX_ROW_LINES {
                return Err(format!("line {number} breaks into more than {MAX_ROW_LINES} lines"));
            }
        }
        if !lines.is_empty() {
            rows.push(PairListRow { line: number, lines });
        }
    }
    if rows.is_empty() {
        return Err("the pair list has no rows (every line is blank or a comment)".into());
    }
    Ok(rows)
}

/// The built-in list: pairs, or phrases.
pub fn builtin(pairs: bool) -> Vec<PairListRow> {
    parse_pair_list(if pairs { BUILTIN_PAIRS } else { BUILTIN_PHRASES }.as_bytes()).unwrap_or_default()
}

pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(row: &PairListRow) -> Vec<Vec<&str>> {
        row.lines.iter().map(|l| l.iter().map(String::as_str).collect()).collect()
    }

    #[test]
    fn a_pair_list_reads_pairs_phrases_and_line_breaks_and_skips_comments() {
        let text = "\u{feff}# kerning work\r\nA V\r\n\n%%FONTLAB\n@T @o\nHamburg\\nnonon\n  T  a  \n";
        let rows = parse_pair_list(text.as_bytes()).unwrap();
        assert_eq!(rows.iter().map(|r| r.line).collect::<Vec<_>>(), vec![2, 5, 6, 7]);
        assert_eq!(words(&rows[0]), vec![vec!["A", "V"]]);
        assert!(rows[0].is_pair() && rows[1].is_pair() && rows[3].is_pair());
        assert_eq!(words(&rows[2]), vec![vec!["Hamburg"], vec!["nonon"]]);
        assert!(!rows[2].is_pair());
        assert_eq!(rows[2].label(), "Hamburg \\n nonon");
    }

    #[test]
    fn a_pair_list_over_a_limit_or_without_rows_is_refused() {
        assert!(parse_pair_list(b"# only a comment\n\n").is_err());
        assert!(parse_pair_list(&[0x41, 0x20, 0xff, 0x0a]).unwrap_err().contains("line 1"));
        let many = "A V\n".repeat(MAX_PAIR_LIST_LINES + 1);
        assert!(parse_pair_list(many.as_bytes()).unwrap_err().contains("lines"));
        let wide = "a ".repeat(MAX_ROW_WORDS + 1);
        assert!(parse_pair_list(wide.as_bytes()).unwrap_err().contains("line 1"));
        assert!(parse_pair_list(&vec![b'a'; MAX_PAIR_LIST_BYTES + 1]).is_err());
    }

    #[test]
    fn the_builtin_lists_parse() {
        assert!(builtin(true).len() >= 20 && builtin(true).iter().all(PairListRow::is_pair));
        assert!(!builtin(false).is_empty() && builtin(false).iter().all(|r| !r.is_pair()));
    }
}

//! `cargo xtask brands`: no other vendor's product or company names in what users and agents read.
//!
//! FontCraft names its own features; where a comparison is needed, text says "the reference app".
//! Scanned:
//! * string literals in Rust sources (command labels and params docs, menus, panels, MCP tool
//!   definitions, CLI usage). Comments are skipped, and so is test code: files named `tests.rs` or
//!   `tests_*.rs`, anything under a `tests/` or `benches/` folder, and the item that follows a
//!   `#[cfg(test)]` attribute (a test module, or a single test-only item);
//! * the `description` of every `Cargo.toml`;
//! * every file under `packaging/`.
//!
//! Markdown and the inventory (`docs/`, `spec/`, `README.md`, `ROADMAP.md`) may name the
//! references: they are how the project is specified.
//!
//! A line that must keep a name (an alias old files still use) says `brand-ok` in a comment on
//! the line where the literal starts.
//!
//! What this cannot see: a name assembled at run time or at compile time (`concat!`, `format!`
//! with parts, escapes such as `\u{46}`). It is a net for slips, not a proof.

use std::path::Path;

/// Vendor and product names, lowercase, matched case-insensitively as whole words or phrases.
/// "Glyphs" alone is not listed: it is also the plain word for what a font is made of. The
/// product is caught by the phrases users would write it with.
const DENY: &[&str] = &[
    "adobe",
    "illustrator",
    "photoshop",
    "indesign",
    "acrobat",
    "typekit",
    "myriad",
    "pantone",
    "afdko",
    "makeotf",
    "otfautohint",
    "psautohint",
    "fontlab",
    "fontographer",
    "typetool",
    "robofont",
    "fontforge",
    "fontcreator",
    "glyphs app",
    "glyphsapp",
    "glyphs.app",
    "glyphs mini",
    "glyphs 2",
    "glyphs 3",
    "glyphs 4",
    "glyphs2",
    "glyphs3",
    "glyphs4",
    "schriftgestalt",
    "monotype",
    "linotype",
];

const ALLOW: &str = "brand-ok";
const SELF: &str = "xtask/src/brands.rs";

#[derive(Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    /// 1-based.
    pub line: usize,
    pub term: &'static str,
    pub context: String,
}

/// The denied term in `text`, if any. A term matches as a whole word or phrase: not inside a
/// longer word, but with digits or punctuation next to it ("FontLab8", "www.adobe.com"), and with
/// any amount of white space between the words of a phrase.
pub fn find_term(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    DENY.iter().copied().find(|term| {
        lower.match_indices(term).any(|(i, _)| {
            let before = lower.get(..i).and_then(|s| s.chars().next_back());
            let after = lower.get(i + term.len()..).and_then(|s| s.chars().next());
            before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphabetic())
        })
    })
}

fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == "tests.rs"
        || name.starts_with("tests_")
        || name.ends_with("_tests.rs")
        || path.starts_with("tests/")
        || path.contains("/tests/")
        || path.contains("/benches/")
}

/// What the scanner needs from a Rust source: its string literals, and enough structure to skip
/// the item after a `#[cfg(test)]`.
#[derive(Debug, PartialEq)]
enum Token {
    Literal(usize, String),
    Open,
    Close,
    Semi,
    CfgTest,
}

fn tokens(src: &str) -> Vec<Token> {
    let chars: Vec<char> = src.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let starts = |i: usize, pat: &str| pat.chars().enumerate().all(|(k, p)| chars.get(i + k) == Some(&p));
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 1usize);
    while let Some(c) = at(i) {
        // A raw string, with or without a `b` prefix: r"…", r#"…"#, br#"…"#.
        let raw_at = if c == 'r' {
            Some(i)
        } else if c == 'b' && at(i + 1) == Some('r') {
            Some(i + 1)
        } else {
            None
        };
        let ident_before = i > 0 && at(i - 1).is_some_and(|p| p.is_alphanumeric() || p == '_');
        if let Some(r) = raw_at.filter(|_| !ident_before) {
            let mut j = r + 1;
            let mut hashes = 0;
            while at(j) == Some('#') {
                hashes += 1;
                j += 1;
            }
            if at(j) == Some('"') {
                j += 1;
                let (start, mut text) = (line, String::new());
                loop {
                    match at(j) {
                        None => break,
                        Some('"') if (1..=hashes).all(|k| at(j + k) == Some('#')) => {
                            j += 1 + hashes;
                            break;
                        }
                        Some(ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            text.push(ch);
                            j += 1;
                        }
                    }
                }
                out.push(Token::Literal(start, text));
                i = j;
                continue;
            }
        }
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if at(i + 1) == Some('/') => {
                while at(i).is_some_and(|c| c != '\n') {
                    i += 1;
                }
            }
            '/' if at(i + 1) == Some('*') => {
                i += 2;
                while at(i).is_some() && !(at(i) == Some('*') && at(i + 1) == Some('/')) {
                    if at(i) == Some('\n') {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
            }
            // A lifetime or a char literal: skip it so a quote inside is not taken for a string.
            '\'' => {
                if at(i + 1) == Some('\\') {
                    i += 2;
                    while at(i).is_some_and(|c| c != '\'' && c != '\n') {
                        i += 1;
                    }
                    i += 1;
                } else if at(i + 2) == Some('\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            '"' => {
                let (start, mut text) = (line, String::new());
                i += 1;
                loop {
                    match at(i) {
                        None => break,
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        // A backslash at the end of a line continues the string: Rust drops the
                        // line break and the indentation that follows.
                        Some('\\') if at(i + 1) == Some('\n') => {
                            i += 2;
                            line += 1;
                            while at(i).is_some_and(char::is_whitespace) {
                                if at(i) == Some('\n') {
                                    line += 1;
                                }
                                i += 1;
                            }
                        }
                        Some('\\') => {
                            if let Some(escaped) = at(i + 1) {
                                text.push(escaped);
                            }
                            i += 2;
                        }
                        Some(ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            text.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Token::Literal(start, text));
            }
            '#' if starts(i, "#[cfg(test)]") => {
                out.push(Token::CfgTest);
                i += "#[cfg(test)]".len();
            }
            '{' => {
                out.push(Token::Open);
                i += 1;
            }
            '}' => {
                out.push(Token::Close);
                i += 1;
            }
            ';' => {
                out.push(Token::Semi);
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// String literals of a Rust source with the line each starts on. Comments are skipped, and so is
/// the item that follows each `#[cfg(test)]`: up to its closing brace, or its `;` if it has no body.
pub fn string_literals(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut skipping: Option<usize> = None; // brace depth inside the item being skipped
    for token in tokens(src) {
        match (token, skipping) {
            (Token::CfgTest, None) => skipping = Some(0),
            (Token::Open, Some(depth)) => skipping = Some(depth + 1),
            (Token::Close, Some(depth)) => skipping = if depth <= 1 { None } else { Some(depth - 1) },
            (Token::Semi, Some(0)) => skipping = None,
            (Token::Literal(line, text), None) => out.push((line, text)),
            _ => {}
        }
    }
    out
}

pub fn scan(root: &Path, files: &[String]) -> Vec<Hit> {
    let mut hits = Vec::new();
    for path in files {
        if path == SELF {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(path)) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        let allowed = |line: usize| lines.get(line.wrapping_sub(1)).is_some_and(|l| l.contains(ALLOW));
        if path.ends_with(".rs") && !is_test_path(path) {
            for (line, literal) in string_literals(&text) {
                if let Some(term) = find_term(&literal)
                    && !allowed(line)
                {
                    hits.push(Hit { path: path.clone(), line, term, context: literal });
                }
            }
        } else if path.ends_with("Cargo.toml") {
            for (n, l) in lines.iter().enumerate().filter(|(_, l)| l.trim_start().starts_with("description")) {
                if let Some(term) = find_term(l)
                    && !l.contains(ALLOW)
                {
                    hits.push(Hit { path: path.clone(), line: n + 1, term, context: (*l).to_string() });
                }
            }
        } else if path.starts_with("packaging/") {
            for (n, l) in lines.iter().enumerate() {
                if let Some(term) = find_term(l)
                    && !l.contains(ALLOW)
                {
                    hits.push(Hit { path: path.clone(), line: n + 1, term, context: (*l).to_string() });
                }
            }
        }
    }
    hits
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let hits = scan(root, &files);
    if hits.is_empty() {
        println!("brands: no vendor names in user-visible text");
        return Ok(());
    }
    for h in &hits {
        let context: String = h.context.chars().take(90).collect();
        eprintln!("  {}:{}: \"{}\" in \"{context}\"", h.path, h.line, h.term);
    }
    Err(format!(
        "{} vendor name(s) in user-visible text: say \"the reference app\" or name the feature (docs/development.md › Vendor names)",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_match_as_words_not_inside_words() {
        assert_eq!(find_term("Imported from Adobe tools"), Some("adobe"));
        assert_eq!(find_term("works like Glyphs 3 does"), Some("glyphs 3"));
        assert_eq!(find_term("works like Glyphs   3 does"), Some("glyphs 3"));
        assert_eq!(find_term("open in the Glyphs app"), Some("glyphs app"));
        assert_eq!(find_term("Glyphs3 files"), Some("glyphs3"));
        assert_eq!(find_term("see www.adobe.com"), Some("adobe"));
        assert_eq!(find_term("e.g.FontLab8"), Some("fontlab"));
        assert_eq!(find_term("Add Glyphs…"), None);
        assert_eq!(find_term("the 3 glyphs selected"), None);
        assert_eq!(find_term("reads .glyphs and .glyphspackage sources"), None);
        assert_eq!(find_term("a myriad of options"), Some("myriad"));
        assert_eq!(find_term("fontlabel"), None);
    }

    #[test]
    fn literals_are_found_and_comments_and_tests_skipped() {
        let src = "// \"comment FontLab\"\nconst A: &str = \"one\"; /* \"block\" */\nlet c = '\"'; let l: &'static str = r#\"raw \"two\"\"#;\nlet e = \"esc \\\" three\";\n#[cfg(test)]\nmod tests { const B: &str = \"FontLab\"; fn f() { let _ = \"inner\"; } }\nconst AFTER: &str = \"four\";\n";
        let found: Vec<String> = string_literals(src).into_iter().map(|(_, s)| s).collect();
        assert_eq!(found, ["one", "raw \"two\"", "esc \" three", "four"]);
        assert_eq!(string_literals(src)[0].0, 2);
    }

    #[test]
    fn only_the_item_after_cfg_test_is_skipped() {
        // A test-only item in the middle of a file must not hide the shipped text after it.
        let src = "#[cfg(test)]\nconst T: &str = \"test only\";\npub const LABEL: &str = \"shipped\";\n#[cfg(test)]\nfn helper() { let _ = \"hidden\"; }\npub const MORE: &str = \"also shipped\";\n";
        let found: Vec<String> = string_literals(src).into_iter().map(|(_, s)| s).collect();
        assert_eq!(found, ["shipped", "also shipped"]);
    }

    #[test]
    fn byte_strings_and_continuations_do_not_derail_the_scanner() {
        let src = "let a = br#\"x \"y\"\"#; let b = \"after raw\";\nlet c = \"Glyphs \\\n        3 import\";\nlet d = b\"bytes\";\n";
        let found: Vec<String> = string_literals(src).into_iter().map(|(_, s)| s).collect();
        assert_eq!(found, ["x \"y\"", "after raw", "Glyphs 3 import", "bytes"]);
        assert_eq!(find_term(&found[2]), Some("glyphs 3"));
    }

    #[test]
    fn this_repository_is_clean() {
        let root = crate::root();
        let files = crate::repo_files(&root).unwrap();
        assert_eq!(scan(&root, &files), vec![]);
    }
}

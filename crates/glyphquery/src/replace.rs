//! Regular-expression Find and Replace on text such as glyph names. The replacement may name a
//! capture group as `\N` (one digit, `\0` for the whole match); `\\` is one backslash, and every
//! other character, `$` included, stands for itself.

use regex::Regex;

use crate::predicate::limited_regex;
use crate::{MAX_PATTERN_BYTES, QueryError, fail};

#[derive(Clone, Debug, PartialEq)]
enum Piece {
    Text(String),
    Group(usize),
}

/// A compiled find pattern and replacement.
#[derive(Clone, Debug)]
pub struct Replacement {
    regex: Regex,
    template: Vec<Piece>,
}

impl Replacement {
    /// Compiles `find` (unanchored) within the same limits as search patterns, and reads
    /// `replace`. A group reference the pattern lacks is an error.
    pub fn new(find: &str, replace: &str, case_insensitive: bool) -> Result<Replacement, QueryError> {
        check_replace(replace)?;
        let regex = limited_regex(find, find, case_insensitive)?;
        let template = parse_template(replace);
        let groups = regex.captures_len();
        if let Some(missing) = template.iter().find_map(|p| match p {
            Piece::Group(n) if *n >= groups => Some(*n),
            _ => None,
        }) {
            let has = groups.saturating_sub(1);
            return fail(format!(
                "the replacement refers to group \\{missing}, but the regular expression has {has} group{}",
                if has == 1 { "" } else { "s" }
            ));
        }
        Ok(Replacement { regex, template })
    }

    /// Finds `find` as plain text and puts `replace` in its place as plain text.
    pub fn literal(find: &str, replace: &str, case_insensitive: bool) -> Result<Replacement, QueryError> {
        check_replace(replace)?;
        let regex = limited_regex(find, &regex::escape(find), case_insensitive)?;
        let template = if replace.is_empty() { Vec::new() } else { vec![Piece::Text(replace.to_string())] };
        Ok(Replacement { regex, template })
    }

    /// `text` with every match replaced.
    pub fn apply(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for caps in self.regex.captures_iter(text) {
            let Some(whole) = caps.get(0) else { continue };
            out.push_str(text.get(last..whole.start()).unwrap_or_default());
            for piece in &self.template {
                match piece {
                    Piece::Text(t) => out.push_str(t),
                    Piece::Group(n) => out.push_str(caps.get(*n).map_or("", |m| m.as_str())),
                }
            }
            last = whole.end();
        }
        out.push_str(text.get(last..).unwrap_or_default());
        out
    }
}

fn check_replace(replace: &str) -> Result<(), QueryError> {
    if replace.len() > MAX_PATTERN_BYTES {
        return fail(format!("the replacement is {} bytes long; at most {MAX_PATTERN_BYTES} are allowed", replace.len()));
    }
    Ok(())
}

fn parse_template(replace: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut chars = replace.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some(d) if d.is_ascii_digit() => {
                chars.next();
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(Piece::Group(d.to_digit(10).map_or(0, |n| n as usize)));
            }
            Some('\\') => {
                chars.next();
                text.push('\\');
            }
            _ => text.push('\\'),
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    pieces
}

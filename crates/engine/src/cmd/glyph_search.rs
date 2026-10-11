//! `glyph.search`: the glyphs of the font that the Find Glyphs pop-up lists for a query
//! (`docs/spec/03b-edit-view-display.md`, Find glyphs).
//!
//! The query is split at spaces into terms; a glyph is listed when it matches every term.
//!
//! | Mode | A term matches a glyph when |
//! |---|---|
//! | `name` | the glyph's name contains it, ignoring case |
//! | `unicode` | one of the glyph's code points, written as hexadecimal, contains it (`U+` may lead) |
//! | `all` | either of the two |
//!
//! In every mode a term of one character also matches the glyph encoded with that character.
//! For a query of one term, exact matches come first (the glyph is encoded with the query's
//! character or code point, or its name is the query), then names that differ from the query
//! only in case; the others follow, each group in the font's glyph order.

use fontcraft_model::glyphdata::MAX_QUERY_BYTES;
use fontcraft_model::{Font, Glyph};
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

/// Most results one search returns (`docs/plans/M5.md` § 3, Limits).
pub const MAX_SEARCH_RESULTS: usize = 1000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "glyph.search",
        "Search Glyphs",
        [],
        None,
        "{query: string, mode?: \"name\" | \"unicode\" | \"all\" (default name), limit?: 1..1000 (default 1000)} the glyphs of the font the Find Glyphs pop-up lists: the query is split at spaces and a glyph must match every term. name: the glyph name contains the term, ignoring case; unicode: a code point in hexadecimal contains it (U+ may lead); all: either. A term of one character also finds the glyph encoded with it. Exact matches come first, then the font's glyph order; an empty query finds nothing. The query is at most 256 bytes → {results: [{name, unicodes: [hex]}], count, total}",
        has_doc,
        search
    )]
}

/// Which property of a glyph a term is compared with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Name,
    Unicode,
    All,
}

impl Mode {
    fn parse(text: &str) -> Option<Mode> {
        match text {
            "name" => Some(Mode::Name),
            "unicode" => Some(Mode::Unicode),
            "all" => Some(Mode::All),
            _ => None,
        }
    }
}

/// One term of the query, prepared once for every glyph.
struct Term {
    /// Lowercased, for names.
    lower: String,
    /// Uppercased hexadecimal digits without a `U+`; `None` when the term is not hexadecimal.
    hex: Option<String>,
    /// The term's character when it is one character long.
    single: Option<char>,
}

impl Term {
    fn new(text: &str) -> Term {
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let digits = text.strip_prefix("U+").or_else(|| text.strip_prefix("u+")).unwrap_or(text);
        let hex = (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_hexdigit())).then(|| digits.to_ascii_uppercase());
        Term { lower: text.to_lowercase(), hex, single }
    }

    fn matches(&self, glyph: &Glyph, name_lower: &str, mode: Mode) -> bool {
        if self.single.is_some_and(|c| glyph.unicodes.contains(&u32::from(c))) {
            return true;
        }
        let by_name = || name_lower.contains(&self.lower);
        let by_code = || self.hex.as_ref().is_some_and(|h| glyph.unicodes.iter().any(|u| hex(*u).contains(h.as_str())));
        match mode {
            Mode::Name => by_name(),
            Mode::Unicode => by_code(),
            Mode::All => by_name() || by_code(),
        }
    }

    /// How closely this term, as the whole query, names `glyph`: 0 for the glyph it is the
    /// character, name or code point of, 1 for a name that differs only in case, 2 otherwise.
    fn rank(&self, glyph: &Glyph, mode: Mode, query: &str) -> u8 {
        let char_hit = self.single.is_some_and(|c| glyph.unicodes.contains(&u32::from(c)));
        let code_hit =
            mode != Mode::Name && self.hex.as_ref().and_then(|h| u32::from_str_radix(h, 16).ok()).is_some_and(|cp| glyph.unicodes.contains(&cp));
        let by_name = mode != Mode::Unicode;
        if char_hit || code_hit || (by_name && glyph.name == query) {
            0
        } else if by_name && glyph.name.to_lowercase() == self.lower {
            1
        } else {
            2
        }
    }
}

fn hex(cp: u32) -> String {
    format!("{cp:04X}")
}

/// The glyphs that match `query` in `mode`, closest first (see [`Term::rank`]), and how many
/// matched in all; at most `limit` of them.
fn find<'f>(font: &'f Font, query: &str, mode: Mode, limit: usize) -> (Vec<&'f Glyph>, usize) {
    let terms: Vec<Term> = query.split_whitespace().map(Term::new).collect();
    let one = match terms.as_slice() {
        [] => return (Vec::new(), 0),
        [t] => Some(t),
        _ => None,
    };
    let whole = query.trim();
    let mut ranked: [Vec<&Glyph>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for g in font.glyphs.iter() {
        let lower = g.name.to_lowercase();
        if !terms.iter().all(|t| t.matches(g, &lower, mode)) {
            continue;
        }
        let rank = one.map_or(2, |t| t.rank(g, mode, whole));
        if let Some(list) = ranked.get_mut(usize::from(rank)) {
            list.push(g);
        }
    }
    let total = ranked.iter().map(Vec::len).sum();
    let mut out: Vec<&Glyph> = ranked.into_iter().flatten().collect();
    out.truncate(limit);
    (out, total)
}

fn search(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.search", v)?;
    let query = p.str("query")?;
    if query.len() > MAX_QUERY_BYTES {
        return Err(p.bad(format!("`query` must be at most {MAX_QUERY_BYTES} bytes")));
    }
    let mode = match p.opt_str("mode")? {
        None => Mode::Name,
        Some(text) => Mode::parse(text).ok_or_else(|| {
            let shown: String = text.chars().take(32).collect();
            p.bad(format!("`mode` must be name, unicode or all, got \"{shown}\""))
        })?,
    };
    let max = i64::try_from(MAX_SEARCH_RESULTS).unwrap_or(i64::MAX);
    let limit = p.opt_int("limit", 1, max)?.and_then(|n| usize::try_from(n).ok()).unwrap_or(MAX_SEARCH_RESULTS);
    let font = s.font()?;
    let (found, total) = find(font, query, mode, limit);
    let results: Vec<Value> =
        found.iter().map(|g| json!({ "name": g.name, "unicodes": g.unicodes.iter().map(|u| hex(*u)).collect::<Vec<_>>() })).collect();
    Ok(json!({ "count": results.len(), "total": total, "results": results }))
}

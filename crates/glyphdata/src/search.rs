//! Searching the database (`glyphData.search`; the guess on that row).
//!
//! A query matches a record when it is a case-insensitive substring of the name, the
//! description or the hex code point, or when it is the record's character itself. An empty
//! query matches every record.

use crate::GlyphData;
use crate::names::code_point_name;
use crate::record::Record;

/// Most results one search returns.
pub const MAX_RESULTS: usize = 10_000;
/// Longest query accepted, in bytes; a longer one matches nothing.
pub const MAX_QUERY_BYTES: usize = 256;

/// The field results are sorted by. Ties sort by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Unicode,
    Category,
    SubCategory,
    Script,
    Description,
    Production,
}

impl SortBy {
    pub fn parse(text: &str) -> Option<SortBy> {
        Some(match text {
            "name" => SortBy::Name,
            "unicode" => SortBy::Unicode,
            "category" => SortBy::Category,
            "subCategory" => SortBy::SubCategory,
            "script" => SortBy::Script,
            "description" => SortBy::Description,
            "production" => SortBy::Production,
            _ => return None,
        })
    }
}

struct Query {
    all: bool,
    lower: String,
    upper_hex: String,
    character: Option<u32>,
}

impl Query {
    fn new(text: &str) -> Option<Query> {
        let text = text.trim();
        if text.len() > MAX_QUERY_BYTES {
            return None;
        }
        let mut chars = text.chars();
        let character = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(u32::from(c)),
            _ => None,
        };
        let hex = text.strip_prefix("U+").or_else(|| text.strip_prefix("u+")).unwrap_or(text);
        Some(Query { all: text.is_empty(), lower: text.to_lowercase(), upper_hex: hex.to_ascii_uppercase(), character })
    }

    fn matches(&self, name: &str, description: Option<&str>, unicode: Option<u32>) -> bool {
        if self.all {
            return true;
        }
        if let Some(cp) = unicode
            && (self.character == Some(cp) || format!("{cp:04X}").contains(&self.upper_hex))
        {
            return true;
        }
        name.to_lowercase().contains(&self.lower) || description.is_some_and(|d| d.to_lowercase().contains(&self.lower))
    }
}

impl GlyphData {
    /// Records matching `query`, sorted by `sort_by` (by name when `None`), at most `limit`
    /// (capped at [`MAX_RESULTS`]).
    pub fn search(&self, query: &str, sort_by: Option<SortBy>, limit: usize) -> Vec<Record> {
        let Some(q) = Query::new(query) else {
            return Vec::new();
        };
        let mut out: Vec<Record> =
            self.custom_records().iter().filter(|r| q.matches(&r.name, r.description.as_deref(), r.unicode)).cloned().collect();
        let t = self.tables();
        for i in 0..t.record_count() {
            let Some(name) = t.record_name(i) else { continue };
            if q.matches(name, t.record_description(i), t.record_unicode(i))
                && !self.custom_shadows(name)
                && let Some(r) = t.record(i)
            {
                out.push(r);
            }
        }
        for (first, last) in t.han_ranges() {
            for cp in first..=last {
                let name = code_point_name(cp);
                if q.matches(&name, None, Some(cp))
                    && !self.custom_shadows(&name)
                    && let Some(r) = t.han_record(cp)
                {
                    out.push(r);
                }
            }
        }
        match sort_by.unwrap_or(SortBy::Name) {
            SortBy::Name => out.sort_by(|a, b| a.name.cmp(&b.name)),
            SortBy::Unicode => out.sort_by(|a, b| (a.unicode.is_none(), a.unicode, &a.name).cmp(&(b.unicode.is_none(), b.unicode, &b.name))),
            SortBy::Category => out.sort_by(|a, b| (a.category.is_none(), a.category, &a.name).cmp(&(b.category.is_none(), b.category, &b.name))),
            SortBy::SubCategory => {
                out.sort_by(|a, b| (a.sub_category.is_none(), a.sub_category, &a.name).cmp(&(b.sub_category.is_none(), b.sub_category, &b.name)))
            }
            SortBy::Script => out.sort_by(|a, b| (a.script.is_none(), &a.script, &a.name).cmp(&(b.script.is_none(), &b.script, &b.name))),
            SortBy::Description => {
                out.sort_by(|a, b| (a.description.is_none(), &a.description, &a.name).cmp(&(b.description.is_none(), &b.description, &b.name)))
            }
            SortBy::Production => {
                out.sort_by(|a, b| (a.production.is_none(), &a.production, &a.name).cmp(&(b.production.is_none(), &b.production, &b.name)))
            }
        }
        out.truncate(limit.min(MAX_RESULTS));
        out
    }
}

//! The allow-list: differences from a reference compiler that are accepted, each with its reason.
//!
//! `crates/oracle/allow.toml` is built in. An entry names a table and one or more difference
//! keys (`key`, and more in `keys`; a `*` in a key matches any run of characters, so
//! `hmtx * advance` matches every glyph's advance), and optionally the oracle and one or more
//! parts of the source path it applies to (`source`, `sources`: any of them) and the two values
//! it applies to (`ours`, `theirs`: patterns with `*` as in keys). A table whose every difference
//! is allowed is reported as "allowed", with the reasons.

use serde::{Deserialize, Serialize};

use crate::Difference;

/// The built-in list.
pub const BUILTIN: &str = include_str!("../allow.toml");
/// Most entries an allow-list may have.
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub table: String,
    pub key: String,
    /// More keys the entry accepts, beside `key`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    #[serde(default)]
    pub oracle: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// More parts of source paths the entry applies to, beside `source`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// FontCraft's value, as the report prints it; any value when absent.
    #[serde(default)]
    pub ours: Option<String>,
    /// The reference's value, as the report prints it; any value when absent.
    #[serde(default)]
    pub theirs: Option<String>,
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    allow: Vec<Entry>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AllowList {
    pub entries: Vec<Entry>,
}

impl AllowList {
    /// The built-in list.
    pub fn builtin() -> Result<AllowList, String> {
        AllowList::parse(BUILTIN)
    }

    /// Reads a list; every entry needs a table, a key and a reason.
    pub fn parse(text: &str) -> Result<AllowList, String> {
        let file: File = toml::from_str(text).map_err(|e| format!("allow-list: {e}"))?;
        if file.allow.len() > MAX_ENTRIES {
            return Err(format!("allow-list: more than {MAX_ENTRIES} entries"));
        }
        for (i, e) in file.allow.iter().enumerate() {
            if e.table.trim().is_empty() || e.key.trim().is_empty() || e.reason.trim().is_empty() {
                return Err(format!("allow-list entry {}: `table`, `key` and `reason` must not be empty", i + 1));
            }
        }
        Ok(AllowList { entries: file.allow })
    }

    /// The entry that accepts difference `d` of `table`, for `oracle` and `source`.
    pub fn find(&self, oracle: &str, source: &str, table: &str, d: &Difference) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            let any_source = e.source.is_none() && e.sources.is_empty();
            e.table == table
                && std::iter::once(&e.key).chain(&e.keys).any(|k| key_matches(k, &d.key))
                && e.oracle.as_deref().is_none_or(|o| o == oracle)
                && (any_source || e.source.iter().chain(&e.sources).any(|s| source.contains(s.as_str())))
                && e.ours.as_deref().is_none_or(|p| key_matches(p, &d.ours))
                && e.theirs.as_deref().is_none_or(|p| key_matches(p, &d.theirs))
        })
    }
}

/// Whether `key` matches `pattern`, each `*` in which stands for any run of characters.
fn key_matches(pattern: &str, key: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else { return pattern == key };
    let Some(mut rest) = key.strip_prefix(first) else { return false };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else { return rest.is_empty() };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = rest.get(i + part.len()..).unwrap_or(""),
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::{AllowList, key_matches};
    use crate::Difference;

    #[test]
    fn values_narrow_an_entry() {
        let list =
            AllowList::parse("[[allow]]\ntable = \"head\"\nkey = \"head.fontRevision\"\nours = \"0\"\ntheirs = \"1*\"\nreason = \"r\"\n").unwrap();
        let d = |ours: &str, theirs: &str| Difference { key: "head.fontRevision".into(), ours: ours.into(), theirs: theirs.into() };
        assert!(list.find("fontmake", "s", "head", &d("0", "1")).is_some());
        assert!(list.find("fontmake", "s", "head", &d("0", "1.5")).is_some());
        assert!(list.find("fontmake", "s", "head", &d("2", "1")).is_none());
        assert!(list.find("fontmake", "s", "head", &d("0", "2")).is_none());
    }

    #[test]
    fn stars_match_any_run_of_characters() {
        assert!(key_matches("hmtx.advance", "hmtx.advance"));
        assert!(!key_matches("hmtx.advance", "hmtx.advanced"));
        assert!(key_matches("OS/2.ulUnicodeRange*", "OS/2.ulUnicodeRange2"));
        assert!(key_matches("hmtx * advance", "hmtx acutecomb advance"));
        assert!(!key_matches("hmtx * advance", "hmtx acutecomb lsb"));
        assert!(key_matches("*", "anything"));
        assert!(key_matches("a*b*c", "a-b-b-c"));
        assert!(!key_matches("a*bc", "abc-"));
        assert!(key_matches("ab*ab", "abab"), "the prefix and the suffix may not overlap, and do not here");
        assert!(!key_matches("ab*ab", "aba"));
    }
}

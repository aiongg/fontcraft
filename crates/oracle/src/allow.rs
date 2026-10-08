//! The allow-list: differences from a reference compiler that are accepted, each with its reason.
//!
//! `crates/oracle/allow.toml` is built in. An entry names a table and a difference key (a
//! trailing `*` matches every key that starts with the rest), and optionally the oracle and a
//! part of the source path it applies to. A table whose every difference is allowed is reported
//! as "allowed", with the reasons.

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
    #[serde(default)]
    pub oracle: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
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
            e.table == table
                && key_matches(&e.key, &d.key)
                && e.oracle.as_deref().is_none_or(|o| o == oracle)
                && e.source.as_deref().is_none_or(|s| source.contains(s))
        })
    }
}

fn key_matches(pattern: &str, key: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => key.starts_with(prefix),
        None => pattern == key,
    }
}

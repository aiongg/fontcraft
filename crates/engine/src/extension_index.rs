//! Extension indexes: JSON lists of modules and macros the Extension Manager offers. Plan:
//! `docs/plans/M13.md` § 3 (extension indexes), § 4 › M13.10; owner decision M13 § 7 question 5
//! (local files and HTTPS indexes, no main index).
//!
//! Format (at most [`MAX_INDEX_BYTES`]): `{"format": "fontcraft.extensions", "version": 1,
//! "entries": [{"id", "kind", "version", "titles": {lang: text}, "descriptions": {lang: text},
//! "vendor", "url", "sha256", "size", "homepage"?, "funding"?, "screenshot"?}]}`. Unknown fields
//! are ignored; a missing `entries` list is an empty index. The engine reads bytes the shell
//! hands it; fetching an HTTPS index is the desktop app's job. An entry is installed only after
//! its download matches `size` and `sha256`.
//!
//! The user's list of indexes is the text setting [`SOURCES_SETTING`]: one absolute path or
//! HTTPS URL per line. Updates from an index install automatically unless its line starts with
//! [`MANUAL_PREFIX`]; then they are only offered. An HTTPS URL may carry a user name and
//! password (`https://user:password@host/index.json`), sent as HTTP Basic credentials; every
//! source shown to the user or written to a result goes through [`redact`] first.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `format` value of an index file.
pub const INDEX_FORMAT: &str = "fontcraft.extensions";
/// The newest index format version this build reads.
pub const INDEX_FORMAT_VERSION: u32 = 1;
/// Largest index file, in bytes.
pub const MAX_INDEX_BYTES: usize = 16 << 20;
/// Most entries in one index.
pub const MAX_INDEX_ENTRIES: usize = 10_000;
/// Longest string in an entry, in characters.
pub const MAX_INDEX_STRING: usize = 4_096;
/// Largest screenshot shown, in bytes.
pub const MAX_SCREENSHOT_BYTES: usize = 8 << 20;
/// Most indexes in the user's list.
pub const MAX_INDEXES: usize = 64;
/// Most languages in an entry's `titles` or `descriptions`.
pub const MAX_LANGUAGES: usize = 256;
/// Longest language tag, in characters.
pub const MAX_LANGUAGE_TAG: usize = 64;
/// Longest entry id, in characters.
pub const MAX_ENTRY_ID: usize = 255;
/// The setting that holds the user's list of indexes.
pub const SOURCES_SETTING: &str = "addons.alternatePluginRepos";
/// A line of [`SOURCES_SETTING`] starting with this offers its index's updates without
/// installing them.
pub const MANUAL_PREFIX: &str = "manual ";

/// Why an index or a download is refused.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum IndexError {
    #[error("the index is {0} bytes; at most {MAX_INDEX_BYTES}")]
    TooLarge(usize),
    #[error("the index is not valid JSON: {0}")]
    Json(String),
    #[error("not an extension index: {0}")]
    Format(String),
    #[error("the index is format version {0}; this version of the app reads up to {INDEX_FORMAT_VERSION}")]
    NewerVersion(u64),
    #[error("entry {entry}: {message}")]
    Entry { entry: usize, message: String },
    #[error("the download of `{id}` does not match its index entry: {message}")]
    Mismatch { id: String, message: String },
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

/// What an index entry installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Filter,
    Reporter,
    Check,
    Macro,
}

impl EntryKind {
    /// The kind's name in indexes and command results.
    pub fn name(self) -> &'static str {
        match self {
            EntryKind::Filter => "filter",
            EntryKind::Reporter => "reporter",
            EntryKind::Check => "check",
            EntryKind::Macro => "macro",
        }
    }

    /// Largest file an entry of this kind may name, in bytes.
    pub fn max_size(self) -> u64 {
        let max = match self {
            EntryKind::Macro => crate::macros::MAX_MACRO_BYTES,
            _ => fontcraft_plugins::error::MAX_MODULE_BYTES,
        };
        u64::try_from(max).unwrap_or(u64::MAX)
    }
}

/// One extension an index offers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub kind: EntryKind,
    pub version: String,
    /// Titles by language tag (`en`, `de`…).
    pub titles: BTreeMap<String, String>,
    pub descriptions: BTreeMap<String, String>,
    pub vendor: String,
    /// Where the module or macro file is downloaded from (a path for a local index).
    pub url: String,
    /// SHA-256 of the file, lowercase hex.
    pub sha256: String,
    /// Size of the file, in bytes.
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
}

impl Entry {
    /// Checks downloaded `bytes` against `size` and `sha256`.
    pub fn verify(&self, bytes: &[u8]) -> Result<(), IndexError> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != self.size {
            return Err(IndexError::Mismatch { id: self.id.clone(), message: format!("{} bytes, expected {}", bytes.len(), self.size) });
        }
        let sum = checksum(bytes);
        if !sum.eq_ignore_ascii_case(self.sha256.trim()) {
            let expected: String = self.sha256.chars().take(64).collect();
            return Err(IndexError::Mismatch { id: self.id.clone(), message: format!("SHA-256 {sum}, expected {expected}") });
        }
        Ok(())
    }

    /// The title in `language`, else English, else the id.
    pub fn title(&self, language: &str) -> &str {
        self.titles.get(language).or_else(|| self.titles.get("en")).map_or(self.id.as_str(), String::as_str)
    }

    /// The description in `language`, else English, else nothing.
    pub fn description(&self, language: &str) -> &str {
        self.descriptions.get(language).or_else(|| self.descriptions.get("en")).map_or("", String::as_str)
    }

    /// Reads entry `number` (counted from 1) of an index and checks every field.
    fn from_value(number: usize, value: &Value) -> Result<Entry, IndexError> {
        let bad = |message: String| IndexError::Entry { entry: number, message };
        let mut entry: Entry = serde_json::from_value(value.clone()).map_err(|e| bad(e.to_string()))?;
        entry.check().map_err(bad)?;
        entry.sha256 = entry.sha256.to_ascii_lowercase();
        Ok(entry)
    }

    fn check(&self) -> Result<(), String> {
        let id_ok = !self.id.is_empty() && self.id.chars().count() <= MAX_ENTRY_ID && !self.id.chars().any(|c| c.is_control() || c.is_whitespace());
        if !id_ok {
            return Err(format!("`id` must be 1 to {MAX_ENTRY_ID} characters without spaces or control characters"));
        }
        let single = [("version", &self.version), ("vendor", &self.vendor), ("url", &self.url), ("sha256", &self.sha256)];
        for (name, text) in single {
            line_ok(name, text)?;
        }
        for (name, text) in [("homepage", &self.homepage), ("funding", &self.funding), ("screenshot", &self.screenshot)] {
            if let Some(text) = text {
                line_ok(name, text)?;
            }
        }
        if self.version.trim().is_empty() {
            return Err("`version` is empty".into());
        }
        for (name, map) in [("titles", &self.titles), ("descriptions", &self.descriptions)] {
            if map.len() > MAX_LANGUAGES {
                return Err(format!("`{name}` has {} languages; at most {MAX_LANGUAGES}", map.len()));
            }
            for (lang, text) in map {
                if lang.is_empty() || lang.chars().count() > MAX_LANGUAGE_TAG || lang.chars().any(char::is_control) {
                    return Err(format!("`{name}` has a language tag that is empty, longer than {MAX_LANGUAGE_TAG} or holds control characters"));
                }
                if text.chars().count() > MAX_INDEX_STRING {
                    return Err(format!("`{name}.{lang}` is longer than {MAX_INDEX_STRING} characters"));
                }
            }
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("`sha256` must be 64 hexadecimal digits".into());
        }
        if self.size == 0 || self.size > self.kind.max_size() {
            return Err(format!("`size` must be 1 to {} bytes for a {}", self.kind.max_size(), self.kind.name()));
        }
        file_reference_ok("url", &self.url)?;
        if let Some(screenshot) = &self.screenshot {
            file_reference_ok("screenshot", screenshot)?;
        }
        for (name, link) in [("homepage", &self.homepage), ("funding", &self.funding)] {
            if let Some(link) = link
                && !(link.starts_with("https://") || link.starts_with("http://"))
            {
                return Err(format!("`{name}` must be an https:// or http:// link"));
            }
        }
        Ok(())
    }
}

/// A one-line field: at most [`MAX_INDEX_STRING`] characters, no control characters.
fn line_ok(name: &str, text: &str) -> Result<(), String> {
    if text.chars().count() > MAX_INDEX_STRING {
        return Err(format!("`{name}` is longer than {MAX_INDEX_STRING} characters"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("`{name}` holds control characters"));
    }
    Ok(())
}

/// A file an entry names: an HTTPS URL, a `file://` URL or a path; any other scheme is refused.
fn file_reference_ok(name: &str, reference: &str) -> Result<(), String> {
    if reference.trim().is_empty() {
        return Err(format!("`{name}` is empty"));
    }
    match scheme(reference) {
        None | Some("https" | "file") => Ok(()),
        Some("http") => Err(format!("`{name}` uses http://: only https:// is fetched")),
        Some(other) => Err(format!("`{name}` uses {other}://: only https://, file:// and paths are read")),
    }
}

/// The URL scheme of `text` (`https` for `https://…`), lowercase; `None` for a path.
fn scheme(text: &str) -> Option<&str> {
    let (scheme, _) = text.split_once("://")?;
    let ok = scheme.len() > 1 && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    // `C://` would be a Windows drive, but no drive letter is longer than one character.
    ok.then_some(scheme)
}

/// The `sha256` value an entry gives for a file holding `bytes`: lowercase hex.
pub fn checksum(bytes: &[u8]) -> String {
    fontcraft_plugins::sha256::hex(bytes)
}

/// A parsed index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    pub entries: Vec<Entry>,
}

impl Index {
    /// Reads an index file, checking format, version and every limit; a bad entry refuses the
    /// whole index with its number (counted from 1).
    pub fn parse(bytes: &[u8]) -> Result<Index, IndexError> {
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(IndexError::TooLarge(bytes.len()));
        }
        // serde_json refuses nesting deeper than 128 levels, so the parse is bounded.
        let value: Value = serde_json::from_slice(bytes).map_err(|e| IndexError::Json(e.to_string()))?;
        let Some(object) = value.as_object() else {
            return Err(IndexError::Format("the file holds no JSON object".into()));
        };
        match object.get("format") {
            Some(Value::String(f)) if f == INDEX_FORMAT => {}
            Some(other) => {
                let shown: String = other.to_string().chars().take(64).collect();
                return Err(IndexError::Format(format!("`format` is {shown}, expected \"{INDEX_FORMAT}\"")));
            }
            None => return Err(IndexError::Format(format!("no `format` (expected \"{INDEX_FORMAT}\")"))),
        }
        match object.get("version").map(Value::as_u64) {
            Some(Some(v)) if v == u64::from(INDEX_FORMAT_VERSION) => {}
            Some(Some(v)) if v > u64::from(INDEX_FORMAT_VERSION) => return Err(IndexError::NewerVersion(v)),
            Some(_) => return Err(IndexError::Format(format!("`version` must be a whole number from 1 to {INDEX_FORMAT_VERSION}"))),
            None => return Err(IndexError::Format("no `version`".into())),
        }
        let entries = match object.get("entries") {
            None | Some(Value::Null) => return Ok(Index::default()),
            Some(Value::Array(entries)) => entries,
            Some(_) => return Err(IndexError::Format("`entries` must be a list".into())),
        };
        if entries.len() > MAX_INDEX_ENTRIES {
            return Err(IndexError::Format(format!("{} entries; at most {MAX_INDEX_ENTRIES}", entries.len())));
        }
        let entries = entries.iter().enumerate().map(|(i, v)| Entry::from_value(i.saturating_add(1), v)).collect::<Result<Vec<_>, _>>()?;
        Ok(Index { entries })
    }

    /// The entries of several indexes, in order; for an id listed more than once the first wins.
    pub fn merge(indexes: &[Index]) -> Index {
        let mut seen = BTreeSet::new();
        let entries = indexes.iter().flat_map(|i| &i.entries).filter(|e| seen.insert(e.id.as_str())).cloned().collect();
        Index { entries }
    }

    /// The first entry with `id`, and `version` when given.
    pub fn find(&self, id: &str, version: Option<&str>) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id && version.is_none_or(|v| e.version == v))
    }
}

/// One line of the user's list of indexes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexSource {
    /// The absolute path or HTTPS URL, with its credentials if any.
    pub location: String,
    /// Updates install without asking (the line does not start with [`MANUAL_PREFIX`]).
    pub automatic_updates: bool,
}

impl IndexSource {
    /// The line that stores this source in [`SOURCES_SETTING`].
    pub fn line(&self) -> String {
        if self.automatic_updates { self.location.clone() } else { format!("{MANUAL_PREFIX}{}", self.location) }
    }
}

/// The sources of the user's list, one per non-empty line, in order, at most [`MAX_INDEXES`].
pub fn parse_sources(text: &str) -> Vec<IndexSource> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| match l.strip_prefix(MANUAL_PREFIX) {
            Some(rest) => IndexSource { location: rest.trim().to_string(), automatic_updates: false },
            None => IndexSource { location: l.to_string(), automatic_updates: true },
        })
        .filter(|s| !s.location.is_empty())
        .take(MAX_INDEXES)
        .collect()
}

/// The text of [`SOURCES_SETTING`] for `sources`.
pub fn sources_text(sources: &[IndexSource]) -> String {
    sources.iter().map(IndexSource::line).collect::<Vec<_>>().join("\n")
}

/// Where an index or an entry's file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// Fetched by the desktop app; may carry credentials.
    Https(String),
    /// Read from disk.
    Local(PathBuf),
}

impl Location {
    /// Reads a source line's location: an HTTPS URL, a `file://` URL or an absolute path.
    pub fn of_source(location: &str) -> Result<Location, String> {
        match scheme(location).map(str::to_ascii_lowercase).as_deref() {
            Some("https") => Ok(Location::Https(location.to_string())),
            Some("file") => Ok(Location::Local(PathBuf::from(location.split_once("://").map_or(location, |(_, p)| p)))),
            Some("http") => Err(format!("{}: only https:// indexes are fetched", redact(location))),
            Some(other) => Err(format!("{}: {other}:// is not read; give a full path or an https:// URL", redact(location))),
            None if Path::new(location).is_absolute() => Ok(Location::Local(PathBuf::from(location))),
            None => Err(format!("{location}: give a full path or an https:// URL")),
        }
    }

    /// Where `reference` (an entry's `url` or `screenshot`) points, relative to the index at
    /// `self`: a URL or absolute path as it is, else a path beside a local index or a URL beside
    /// an HTTPS one (which keeps the index's credentials).
    pub fn resolve(&self, reference: &str) -> Location {
        match scheme(reference).map(str::to_ascii_lowercase).as_deref() {
            Some("https") => return Location::Https(reference.to_string()),
            // A file:// reference reads from disk only for a local index; for an HTTPS index it
            // stays a URL, which the fetch refuses.
            Some(_) => {
                return match self {
                    Location::Local(_) => Location::Local(PathBuf::from(reference.split_once("://").map_or(reference, |(_, p)| p))),
                    Location::Https(_) => Location::Https(reference.to_string()),
                };
            }
            None => {}
        }
        match self {
            Location::Local(index) => {
                let path = PathBuf::from(reference);
                match index.parent() {
                    Some(dir) if path.is_relative() => Location::Local(dir.join(path)),
                    _ => Location::Local(path),
                }
            }
            Location::Https(index) => Location::Https(join_url(index, reference)),
        }
    }
}

/// `reference` resolved against the HTTPS URL `base`: from the host for `/path`, else beside
/// the base's last path segment.
fn join_url(base: &str, reference: &str) -> String {
    let base = base.split(['?', '#']).next().unwrap_or(base);
    let after_scheme = base.find("://").map_or(0, |i| i.saturating_add(3));
    let rest = base.get(after_scheme..).unwrap_or_default();
    let host_end = after_scheme.saturating_add(rest.find('/').unwrap_or(rest.len()));
    if reference.starts_with('/') {
        return format!("{}{reference}", base.get(..host_end).unwrap_or(base));
    }
    let dir_end = match base.rfind('/') {
        Some(i) if i >= host_end => i,
        _ => return format!("{base}/{reference}"),
    };
    format!("{}/{reference}", base.get(..dir_end).unwrap_or(base))
}

/// `location` without the user name and password of an HTTPS URL, for messages and results.
pub fn redact(location: &str) -> String {
    let Some((scheme, rest)) = location.split_once("://") else { return location.to_string() };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest.get(..authority_end).unwrap_or(rest);
    match authority.rfind('@') {
        Some(at) => format!("{scheme}://{}", rest.get(at.saturating_add(1)..).unwrap_or_default()),
        None => location.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use serde_json::json;

    fn entry_json(id: &str, version: &str) -> Value {
        json!({
            "id": id, "kind": "filter", "version": version,
            "titles": {"en": "Square", "de": "Quadrat"}, "descriptions": {"en": "Squares every glyph."},
            "vendor": "Tests", "url": "square.wasm", "sha256": "AB".repeat(32), "size": 120,
            "homepage": "https://example.com/square", "funding": "https://example.com/fund",
            "screenshot": "square.png", "futureField": [1, 2, 3],
        })
    }

    fn index(entries: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"format": INDEX_FORMAT, "version": 1, "entries": entries})).unwrap()
    }

    #[test]
    fn parses_an_index_with_every_field_and_ignores_unknown_ones() {
        let parsed = Index::parse(&index(vec![entry_json("org.test.square", "2")])).unwrap();
        let e = &parsed.entries[0];
        assert_eq!((e.id.as_str(), e.kind, e.version.as_str(), e.size), ("org.test.square", EntryKind::Filter, "2", 120));
        assert_eq!((e.title("de"), e.title("fr"), e.description("fr")), ("Quadrat", "Square", "Squares every glyph."));
        assert_eq!(e.sha256, "ab".repeat(32), "stored lowercase");
        assert_eq!(e.screenshot.as_deref(), Some("square.png"));
        assert_eq!(e.funding.as_deref(), Some("https://example.com/fund"));
        let empty = serde_json::to_vec(&json!({"format": INDEX_FORMAT, "version": 1})).unwrap();
        assert_eq!(Index::parse(&empty).unwrap(), Index::default(), "an index with no entries is valid");
    }

    #[test]
    fn hostile_indexes_fail_with_a_message() {
        let with = |key: &str, value: Value| {
            let mut e = entry_json("org.test.square", "2");
            e[key] = value;
            Index::parse(&index(vec![e]))
        };
        let entry_error = |r: Result<Index, IndexError>| match r {
            Err(IndexError::Entry { entry, message }) => (entry, message),
            other => panic!("expected an entry error, got {other:?}"),
        };
        assert!(entry_error(with("sha256", json!("12"))).1.contains("sha256"));
        assert_eq!(entry_error(with("size", json!(-1))).0, 1);
        assert!(entry_error(with("size", json!(1u64 << 40))).1.contains("size"));
        assert!(entry_error(with("size", json!(0))).1.contains("size"));
        assert!(entry_error(with("kind", json!("plugin"))).0 == 1);
        assert!(entry_error(with("id", json!(""))).1.contains("id"));
        assert!(entry_error(with("id", json!("a b"))).1.contains("id"));
        assert!(entry_error(with("vendor", json!("x".repeat(MAX_INDEX_STRING + 1)))).1.contains("vendor"));
        assert!(entry_error(with("url", json!("http://example.com/a.wasm"))).1.contains("https"));
        assert!(entry_error(with("url", json!("javascript://x"))).1.contains("url"));
        assert!(entry_error(with("homepage", json!("file:///etc/passwd"))).1.contains("homepage"));
        assert!(entry_error(with("version", json!("1\u{0}"))).1.contains("control"));
        let many: BTreeMap<String, String> = (0..=MAX_LANGUAGES).map(|i| (format!("l{i}"), "t".into())).collect();
        assert!(entry_error(with("titles", json!(many))).1.contains("languages"));
        // The second entry is the bad one, and its number says so.
        let mut bad = entry_json("org.test.b", "1");
        bad["sha256"] = json!("zz");
        assert_eq!(entry_error(Index::parse(&index(vec![entry_json("org.test.a", "1"), bad]))).0, 2);

        assert!(matches!(Index::parse(b"[1, 2"), Err(IndexError::Json(_))));
        assert!(matches!(Index::parse(b"[]"), Err(IndexError::Format(_))));
        assert!(matches!(Index::parse(br#"{"format": "other", "version": 1}"#), Err(IndexError::Format(_))));
        assert!(matches!(Index::parse(br#"{"format": "fontcraft.extensions", "version": 7}"#), Err(IndexError::NewerVersion(7))));
        assert!(matches!(Index::parse(br#"{"format": "fontcraft.extensions", "version": -1}"#), Err(IndexError::Format(_))));
        assert!(matches!(Index::parse(br#"{"format": "fontcraft.extensions", "version": 1, "entries": {}}"#), Err(IndexError::Format(_))));
        assert!(matches!(Index::parse(&vec![b' '; MAX_INDEX_BYTES + 1]), Err(IndexError::TooLarge(_))));
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(Index::parse(deep.as_bytes()).is_err(), "deep nesting fails without overflowing the stack");
        let too_many = vec![json!(null); MAX_INDEX_ENTRIES + 1];
        assert!(matches!(Index::parse(&index(too_many)), Err(IndexError::Format(m)) if m.contains("entries")));
    }

    #[test]
    fn merging_keeps_the_first_index_entry_of_each_id() {
        let a = Index::parse(&index(vec![entry_json("org.test.x", "1"), entry_json("org.test.y", "1")])).unwrap();
        let b = Index::parse(&index(vec![entry_json("org.test.y", "2"), entry_json("org.test.z", "1")])).unwrap();
        let merged = Index::merge(&[a, b]);
        let got: Vec<(&str, &str)> = merged.entries.iter().map(|e| (e.id.as_str(), e.version.as_str())).collect();
        assert_eq!(got, [("org.test.x", "1"), ("org.test.y", "1"), ("org.test.z", "1")]);
        assert_eq!(merged.find("org.test.y", None).map(|e| e.version.as_str()), Some("1"));
        assert!(merged.find("org.test.y", Some("2")).is_none());
    }

    #[test]
    fn a_download_is_checked_against_size_and_checksum() {
        let bytes = b"module bytes".to_vec();
        let mut e: Entry = serde_json::from_value(entry_json("org.test.square", "1")).unwrap();
        e.size = bytes.len() as u64;
        e.sha256 = fontcraft_plugins::sha256::hex(&bytes).to_uppercase();
        assert_eq!(e.verify(&bytes), Ok(()));
        assert!(matches!(e.verify(b"module bytez"), Err(IndexError::Mismatch { message, .. }) if message.contains("SHA-256")));
        assert!(matches!(e.verify(b"short"), Err(IndexError::Mismatch { message, .. }) if message.contains("bytes")));
    }

    #[test]
    fn sources_locations_and_credentials() {
        let text = "  /srv/index.json\n\nmanual https://user:pa%40ss@example.com/ext/index.json\nhttp://plain.example\n";
        let sources = parse_sources(text);
        assert_eq!(sources.len(), 3);
        assert!(sources[0].automatic_updates && !sources[1].automatic_updates);
        assert_eq!(sources_text(&sources), "/srv/index.json\nmanual https://user:pa%40ss@example.com/ext/index.json\nhttp://plain.example");
        assert!(Location::of_source(&sources[2].location).unwrap_err().contains("https"));
        assert!(Location::of_source("relative/index.json").is_err());
        assert_eq!(redact(&sources[1].location), "https://example.com/ext/index.json");
        assert_eq!(redact("/a@b/index.json"), "/a@b/index.json");

        let remote = Location::of_source(&sources[1].location).unwrap();
        assert_eq!(remote.resolve("files/a.wasm"), Location::Https("https://user:pa%40ss@example.com/ext/files/a.wasm".into()));
        assert_eq!(remote.resolve("/top.wasm"), Location::Https("https://user:pa%40ss@example.com/top.wasm".into()));
        assert_eq!(remote.resolve("https://cdn.example/b.wasm"), Location::Https("https://cdn.example/b.wasm".into()));
        let local = Location::of_source("/srv/index.json").unwrap();
        assert_eq!(local.resolve("a.wasm"), Location::Local(PathBuf::from("/srv/a.wasm")));
        assert_eq!(local.resolve("file:///opt/b.wasm"), Location::Local(PathBuf::from("/opt/b.wasm")));
        let many = "/i.json\n".repeat(MAX_INDEXES + 5);
        assert_eq!(parse_sources(&many).len(), MAX_INDEXES);
    }
}

//! Find and replace in feature code (Font Info › Features find bar, M8.27).
//!
//! Commands: `font.features.find`, `font.features.replace`. Rows: `spec/inventory/06-font-info.toml`.
//!
//! The query is plain text unless `regex` is set. Both forms compile to one regular expression of
//! the `regex` crate, which matches in time linear in the text and never backtracks; the pattern's
//! length and compiled size are capped, so a hostile pattern is refused rather than run. A match of
//! no characters is skipped. Whole Word keeps a match only when no name character (a letter, a
//! digit, `_` or `.`) touches either end. The scope is one entry when `kind` and `name` are given
//! (in `code`, the editor's unsaved text, when that is given too), else every entry in sidebar
//! order: prefixes, classes, features.
//!
//! Replace All over every entry is one undo step and leaves automatic entries alone (they are
//! regenerated from the font, so an edit there would not last); they are listed as skipped.

use std::sync::Arc;

use fontcraft_features::{EntryKind, EntryRef, MAX_ENTRY_BYTES};
use fontcraft_model::Features;
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

use super::features::{entry_param, group, kind_name, snippet_mut};
use super::*;
use crate::params::Params;
use crate::{Result, Session};

/// Most bytes of a query.
pub const MAX_QUERY_BYTES: usize = 1_000;
/// Most bytes of a replacement.
pub const MAX_REPLACEMENT_BYTES: usize = 64 * 1024;
/// Most matches one find lists.
pub const MAX_MATCHES: usize = 10_000;
/// Most bytes the compiled pattern may take.
const MAX_PATTERN_SIZE: usize = 1 << 20;
/// The groups in sidebar order.
const ORDER: [EntryKind; 3] = [EntryKind::Prefix, EntryKind::Class, EntryKind::Feature];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.features.find",
            "Find in Feature Code",
            [],
            None,
            "{query: string (at most 1,000 bytes), matchCase?: bool (default false), wholeWord?: bool (default false), regex?: bool (default false: the query is plain text), kind?: \"prefix\"|\"class\"|\"feature\", name?: string, index?: number, code?: string} the matches of the query, as the find bar lists them: in the entry `kind` and `name` (in `code`, the editor's unsaved text, when given), else in every entry in sidebar order. Whole Word keeps a match no letter, digit, `_` or `.` touches; an empty match is skipped. A regular expression runs in linear time; an invalid or oversized one is refused. At most 10,000 matches → {matches: [{kind, name, index, start, end, line}] (start and end are byte offsets of the entry's code, line counts from 1), count, truncated}",
            has_doc,
            find
        ),
        cmd!(
            "font.features.replace",
            "Replace in Feature Code",
            [],
            None,
            "{query: string (at most 1,000 bytes), matchCase?: bool, wholeWord?: bool, regex?: bool, replacement: string (at most 64 KB; with regex, $1 and ${name} take a group), all?: bool (default false), kind?: \"prefix\"|\"class\"|\"feature\", name?: string, index?: number, code?: string, start?: number} replace the match that begins at byte `start` of the entry `kind` and `name` (Replace), or every match in that entry or, without `kind` and `name`, in every entry (Replace All). With `code` the entry's text is the editor's unsaved text, and the result is stored. One undo step. Replace All over every entry leaves automatic entries alone and lists them as skipped; naming an automatic entry is refused. Refused: no match at `start`, an invalid pattern, a result over 4 MB in one entry → {replaced: number of matches replaced, entries: [{kind, name, index, count}], skipped: [{kind, name, index}], caret?: byte offset after the replacement (Replace)}",
            has_doc,
            replace
        ),
    ]
}

/// A compiled query.
pub(crate) struct Search {
    regex: Regex,
    whole_word: bool,
    /// The replacement expands `$1` and `${name}`.
    expands: bool,
}

/// Whether `c` may be part of a glyph, class or lookup name, for Whole Word.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

impl Search {
    /// The query of the parameters `query`, `matchCase`, `wholeWord` and `regex`.
    pub(crate) fn parse(p: &Params) -> Result<Search> {
        let query = p.str("query")?;
        if query.is_empty() {
            return Err(p.bad("`query` is empty"));
        }
        if query.len() > MAX_QUERY_BYTES {
            return Err(p.bad(format!("`query` has {} bytes, more than {MAX_QUERY_BYTES}", query.len())));
        }
        let match_case = p.bool_or("matchCase", false)?;
        let whole_word = p.bool_or("wholeWord", false)?;
        let regex = p.bool_or("regex", false)?;
        let pattern = if regex { query.to_string() } else { regex::escape(query) };
        let compiled = RegexBuilder::new(&pattern)
            .case_insensitive(!match_case)
            .size_limit(MAX_PATTERN_SIZE)
            .dfa_size_limit(MAX_PATTERN_SIZE)
            .build()
            .map_err(|e| p.bad(format!("the query is not a regular expression this editor can run: {e}")))?;
        Ok(Search { regex: compiled, whole_word, expands: regex })
    }

    /// Whether the match `start..end` of `text` counts.
    fn keeps(&self, text: &str, start: usize, end: usize) -> bool {
        if start == end {
            return false;
        }
        if !self.whole_word {
            return true;
        }
        let before = text.get(..start).and_then(|s| s.chars().next_back());
        let after = text.get(end..).and_then(|s| s.chars().next());
        !before.is_some_and(is_name_char) && !after.is_some_and(is_name_char)
    }

    /// The matches of `text` as byte ranges, at most `room` of them; the second value is whether more were left.
    /// Non-overlapping, left to right, in one pass.
    pub(crate) fn matches(&self, text: &str, room: usize) -> (Vec<(usize, usize)>, bool) {
        let mut out = Vec::new();
        for m in self.regex.find_iter(text) {
            if !self.keeps(text, m.start(), m.end()) {
                continue;
            }
            if out.len() >= room {
                return (out, true);
            }
            out.push((m.start(), m.end()));
        }
        (out, false)
    }

    /// `text` with its matches replaced by `replacement`: every one, or only the one that begins at `only_at`. Returns
    /// the new text, the number replaced and the byte offset after the last replacement. A result over
    /// [`MAX_ENTRY_BYTES`] is refused as soon as it grows past it.
    fn replace(&self, p: &Params, text: &str, replacement: &str, only_at: Option<usize>) -> Result<(String, usize, usize)> {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut count = 0;
        let mut caret = 0;
        for caps in self.regex.captures_iter(text) {
            let Some(m) = caps.get(0) else { continue };
            if !self.keeps(text, m.start(), m.end()) || only_at.is_some_and(|at| at != m.start()) {
                continue;
            }
            out.push_str(text.get(last..m.start()).unwrap_or_default());
            if self.expands {
                caps.expand(replacement, &mut out);
            } else {
                out.push_str(replacement);
            }
            caret = out.len();
            last = m.end();
            count += 1;
            if out.len() > MAX_ENTRY_BYTES {
                return Err(p.bad(format!("the replaced code would have more than the {MAX_ENTRY_BYTES} bytes one entry may hold")));
            }
            if only_at.is_some() {
                break;
            }
        }
        out.push_str(text.get(last..).unwrap_or_default());
        if out.len() > MAX_ENTRY_BYTES {
            return Err(p.bad(format!("the replaced code would have more than the {MAX_ENTRY_BYTES} bytes one entry may hold")));
        }
        Ok((out, count, caret))
    }
}

/// The entry the parameters name, when they name one (`kind` given).
fn scope(p: &Params, f: &Features) -> Result<Option<EntryRef>> {
    if p.get("kind").is_none() {
        return Ok(None);
    }
    entry_param(p, f).map(Some)
}

/// Every entry of `f`, in sidebar order.
fn every_entry(f: &Features) -> Vec<EntryRef> {
    ORDER.iter().flat_map(|kind| (0..group(f, *kind).len()).map(|index| EntryRef { kind: *kind, index })).collect()
}

fn entry_json(f: &Features, r: EntryRef) -> Value {
    let name = group(f, r.kind).get(r.index).map(|e| e.name.as_str()).unwrap_or_default();
    json!({ "kind": kind_name(r.kind), "name": name, "index": r.index })
}

/// The code `code` names, checked against the entry size cap.
fn unsaved_code<'a>(p: &Params<'a>) -> Result<Option<&'a str>> {
    let code = p.opt_str("code")?;
    if let Some(c) = code
        && c.len() > MAX_ENTRY_BYTES
    {
        return Err(p.bad(format!("`code` has {} bytes, more than the {MAX_ENTRY_BYTES} one entry may hold", c.len())));
    }
    Ok(code)
}

// ── font.features.find ───────────────────────────────────────────────────────────────────────────

fn find(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.find", raw)?;
    let search = Search::parse(&p)?;
    let f = &s.font()?.features;
    let code = unsaved_code(&p)?;
    let target = scope(&p, f)?;
    let only = target.is_some();
    let entries: Vec<EntryRef> = match target {
        Some(r) => vec![r],
        None => every_entry(f),
    };
    let mut matches = Vec::new();
    let mut truncated = false;
    for r in entries {
        let Some(e) = group(f, r.kind).get(r.index) else { continue };
        let text = if only { code.unwrap_or(&e.code) } else { &e.code };
        let (found, more) = search.matches(text, MAX_MATCHES.saturating_sub(matches.len()));
        let mut line = 1usize;
        let mut counted = 0usize;
        for (start, end) in found {
            line += text.get(counted..start).map_or(0, |s| s.bytes().filter(|b| *b == b'\n').count());
            counted = start;
            matches.push(json!({ "kind": kind_name(r.kind), "name": e.name, "index": r.index, "start": start, "end": end, "line": line }));
        }
        if more {
            truncated = true;
            break;
        }
    }
    Ok(json!({ "count": matches.len(), "matches": matches, "truncated": truncated }))
}

// ── font.features.replace ────────────────────────────────────────────────────────────────────────

fn replace(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.replace", raw)?;
    let search = Search::parse(&p)?;
    let replacement = p.str("replacement")?;
    if replacement.len() > MAX_REPLACEMENT_BYTES {
        return Err(p.bad(format!("`replacement` has {} bytes, more than {MAX_REPLACEMENT_BYTES}", replacement.len())));
    }
    let all = p.bool_or("all", false)?;
    let start = p.opt_int("start", 0, MAX_ENTRY_BYTES as i64)?.map(|i| i as usize);
    let f = &s.font()?.features;
    let target = scope(&p, f)?;
    let code = unsaved_code(&p)?;
    if !all && (target.is_none() || start.is_none()) {
        return Err(p.bad("Replace needs the entry (`kind`, `name`) and the match's `start`; set `all` to replace every match"));
    }
    if target.is_none() && code.is_some() {
        return Err(p.bad("`code` needs the entry it belongs to (`kind`, `name`)"));
    }
    let only_at = if all { None } else { start };
    let mut changes: Vec<(EntryRef, String, usize)> = Vec::new();
    let mut skipped = Vec::new();
    let mut caret = None;
    let refs: Vec<EntryRef> = match target {
        Some(r) => vec![r],
        None => every_entry(f),
    };
    for r in refs {
        let Some(e) = group(f, r.kind).get(r.index) else { continue };
        if e.automatic {
            if target.is_some() {
                return Err(p.bad(format!(
                    "the {} `{}` is generated automatically: turn off Generate Feature Automatically to edit its code",
                    kind_name(r.kind),
                    e.name
                )));
            }
            skipped.push(entry_json(f, r));
            continue;
        }
        let text = if target.is_some() { code.unwrap_or(&e.code) } else { &e.code };
        let (next, count, after) = search.replace(&p, text, replacement, only_at)?;
        if !all && count == 0 {
            return Err(p.bad(format!("no match of the query begins at byte {} of the {} `{}`", start.unwrap_or(0), kind_name(r.kind), e.name)));
        }
        if !all {
            caret = Some(after);
        }
        if count > 0 || (code.is_some() && next != e.code) {
            changes.push((r, next, count));
        }
    }
    let entries: Vec<Value> = changes
        .iter()
        .map(|(r, _, count)| {
            let mut v = entry_json(f, *r);
            if let Some(o) = v.as_object_mut() {
                o.insert("count".into(), json!(count));
            }
            v
        })
        .collect();
    let replaced: usize = changes.iter().map(|(_, _, count)| count).sum();
    if !changes.is_empty() {
        let command = p.command();
        let label = if all { "Replace All in Feature Code" } else { "Replace in Feature Code" };
        s.edit(label, |font| {
            let features = Arc::make_mut(&mut font.features);
            for (r, text, _) in changes {
                snippet_mut(features, r, command)?.code = text;
            }
            Ok(())
        })?;
    }
    let mut reply = json!({ "replaced": replaced, "entries": entries, "skipped": skipped });
    if let (Some(c), Some(o)) = (caret, reply.as_object_mut()) {
        o.insert("caret".into(), json!(c));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(query: &str, regex: bool, whole_word: bool) -> Search {
        let raw = json!({ "query": query, "regex": regex, "wholeWord": whole_word });
        let p = Params::new("font.features.find", &raw).unwrap();
        Search::parse(&p).unwrap()
    }

    #[test]
    fn whole_word_skips_a_match_inside_a_glyph_name() {
        let s = search("a", false, true);
        assert_eq!(s.matches("sub a a.sc ba by a;", 100).0, vec![(4, 5), (17, 18)]);
    }

    #[test]
    fn a_pattern_that_matches_nothing_finds_no_empty_matches_and_large_text_stays_linear() {
        let s = search("x*", true, false);
        assert!(s.matches("abc", 100).0.is_empty());
        let s = search("(a|aa)*b", true, false);
        let hostile = "a".repeat(1 << 20);
        let started = std::time::Instant::now();
        assert!(s.matches(&hostile, 100).0.is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[test]
    fn an_oversized_pattern_is_refused() {
        let raw = json!({ "query": "a{1000}{1000}", "regex": true });
        let p = Params::new("font.features.find", &raw).unwrap();
        assert!(Search::parse(&p).is_err());
    }
}

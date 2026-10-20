//! The feature-code fixes of the third stage (`docs/spec/21-qa.md` § 5 "Fix calls", steps 35 to
//! 37): the new code of the entries to edit, or the new feature order, worked out from the
//! stored feature code. The calls go through `font.features.setCode` and
//! `font.features.reorder`.

use std::collections::BTreeMap;

use fontcraft_model::{Features, Snippet};
use serde_json::{Value, json};

/// The statement `addDefaultLanguagesystem` puts first.
const DEFAULT_STATEMENT: &str = "languagesystem DFLT dflt;";

/// Byte ranges of the tokens of feature code: words, and the punctuation `;{}[]()<>,=` one
/// character each. Comments and quoted strings are left out.
fn tokens(code: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut word: Option<usize> = None;
    let mut chars = code.char_indices().peekable();
    let end_word = |word: &mut Option<usize>, at: usize, out: &mut Vec<(usize, usize)>| {
        if let Some(start) = word.take() {
            out.push((start, at));
        }
    };
    while let Some((i, c)) = chars.next() {
        match c {
            '#' => {
                end_word(&mut word, i, &mut out);
                while chars.next_if(|(_, c)| *c != '\n').is_some() {}
            }
            '"' => {
                end_word(&mut word, i, &mut out);
                while chars.next_if(|(_, c)| *c != '"').is_some() {}
                chars.next();
            }
            ';' | '{' | '}' | '[' | ']' | '(' | ')' | '<' | '>' | ',' | '=' => {
                end_word(&mut word, i, &mut out);
                out.push((i, i + c.len_utf8()));
            }
            c if c.is_whitespace() => end_word(&mut word, i, &mut out),
            _ => {
                word.get_or_insert(i);
            }
        }
    }
    end_word(&mut word, code.len(), &mut out);
    out
}

/// The text of token `i`.
fn text<'c>(code: &'c str, toks: &[(usize, usize)], i: usize) -> Option<&'c str> {
    toks.get(i).and_then(|&(a, b)| code.get(a..b))
}

/// A byte range of a snippet and the text that replaces it.
type Replacement = (usize, usize, String);

/// `code` with each range of `edits` (sorted, not overlapping) replaced by its text.
fn edited(code: &str, edits: &[Replacement]) -> String {
    let mut out = String::with_capacity(code.len() + 32);
    let mut at = 0;
    for (start, end, with) in edits {
        out.push_str(code.get(at..*start).unwrap_or_default());
        out.push_str(with);
        at = *end;
    }
    out.push_str(code.get(at..).unwrap_or_default());
    out
}

fn kind_name(kind: usize) -> &'static str {
    match kind {
        0 => "prefix",
        1 => "class",
        _ => "feature",
    }
}

/// The prefixes, classes and features, in the order the compiler joins them, with their kind
/// (0, 1, 2) and index.
fn entries(f: &Features) -> impl Iterator<Item = (usize, usize, &Snippet)> {
    [&f.prefixes, &f.classes, &f.features].into_iter().enumerate().flat_map(|(k, list)| list.iter().enumerate().map(move |(i, e)| (k, i, e)))
}

fn set_code(kind: usize, index: usize, e: &Snippet, code: String) -> Value {
    json!({"kind": kind_name(kind), "name": e.name, "index": index, "code": code})
}

fn automatic(kind: usize, e: &Snippet) -> String {
    format!("the {} `{}` is generated automatically: its code cannot be edited", kind_name(kind), e.name)
}

/// Step 35: the `font.features.setCode` calls that put the registered tag in place of each
/// mistyped one (`slips`: mistyped, registered) as the language of every `language` and
/// `languagesystem` statement of the prefixes and features, and why an entry that uses a
/// mistyped tag is left out.
pub(super) fn rename_language_tag(f: &Features, slips: &[(String, String)]) -> (Vec<Value>, Vec<String>) {
    let (mut calls, mut left) = (Vec::new(), Vec::new());
    for (kind, index, e) in entries(f).filter(|(k, ..)| *k != 1) {
        let toks = tokens(&e.code);
        let mut edits = Vec::new();
        for i in 0..toks.len() {
            let at = match text(&e.code, &toks, i) {
                Some("language") => i + 1,
                Some("languagesystem") => i + 2,
                _ => continue,
            };
            if let Some(tag) = text(&e.code, &toks, at)
                && let Some((_, meant)) = slips.iter().find(|(wrong, _)| wrong == tag)
                && let Some(&(a, b)) = toks.get(at)
            {
                edits.push((a, b, meant.clone()));
            }
        }
        if edits.is_empty() {
            continue;
        }
        if e.automatic {
            left.push(automatic(kind, e));
        } else {
            calls.push(set_code(kind, index, e, edited(&e.code, &edits)));
        }
    }
    (calls, left)
}

/// One `languagesystem` statement: where it starts and ends (after its `;`) in its entry's code.
struct Statement {
    kind: usize,
    index: usize,
    start: usize,
    end: usize,
    default: bool,
}

/// The `languagesystem` statements of the entries that are not switched off, in compile order.
fn statements(f: &Features) -> Vec<Statement> {
    let mut out = Vec::new();
    for (kind, index, e) in entries(f).filter(|(.., e)| !e.disabled) {
        let toks = tokens(&e.code);
        for i in 0..toks.len() {
            if text(&e.code, &toks, i) != Some("languagesystem") {
                continue;
            }
            let (Some(&(start, _)), Some(&(_, lang_end))) = (toks.get(i), toks.get(i + 2)) else { continue };
            let end = match (text(&e.code, &toks, i + 3), toks.get(i + 3)) {
                (Some(";"), Some(&(_, b))) => b,
                _ => lang_end,
            };
            let default = text(&e.code, &toks, i + 1) == Some("DFLT") && text(&e.code, &toks, i + 2) == Some("dflt");
            out.push(Statement { kind, index, start, end, default });
        }
    }
    out
}

/// The white space a line holds before `at`, when only white space comes before it.
fn indent(code: &str, at: usize) -> Option<&str> {
    let line_start = code.get(..at)?.rfind('\n').map_or(0, |n| n + 1);
    code.get(line_start..at).filter(|s| s.chars().all(char::is_whitespace))
}

/// Step 36: the `font.features.setCode` calls that put `languagesystem DFLT dflt;` before the
/// first `languagesystem` statement and remove a later one. Nothing when the first statement is
/// already the default one; the reason when an entry to edit is generated automatically.
pub(super) fn add_default_languagesystem(f: &Features) -> Result<Vec<Value>, String> {
    let all = statements(f);
    let Some(first) = all.first() else { return Ok(Vec::new()) };
    if first.default {
        return Ok(Vec::new());
    }
    let mut edits: BTreeMap<(usize, usize), Vec<Replacement>> = BTreeMap::new();
    let snippet = |kind: usize, index: usize| [&f.prefixes, &f.classes, &f.features].get(kind).and_then(|l| l.get(index));
    let Some(code) = snippet(first.kind, first.index).map(|e| e.code.as_str()) else { return Ok(Vec::new()) };
    let insert = match indent(code, first.start) {
        Some(pad) => format!("{DEFAULT_STATEMENT}\n{pad}"),
        None => format!("{DEFAULT_STATEMENT} "),
    };
    edits.entry((first.kind, first.index)).or_default().push((first.start, first.start, insert));
    for st in all.iter().skip(1).filter(|s| s.default) {
        let Some(code) = snippet(st.kind, st.index).map(|e| e.code.as_str()) else { continue };
        // A statement alone on its line goes with its line.
        let rest = code.get(st.end..).unwrap_or_default();
        let line_end = rest.find('\n').map(|n| st.end + n + 1);
        let alone = indent(code, st.start).is_some() && rest.split('\n').next().is_some_and(|s| s.trim().is_empty());
        let span = match (alone, line_end) {
            (true, Some(end)) => (st.start.saturating_sub(indent(code, st.start).map_or(0, str::len)), end),
            (true, None) => (st.start.saturating_sub(indent(code, st.start).map_or(0, str::len)), code.len()),
            (false, _) => (st.start, st.end),
        };
        edits.entry((st.kind, st.index)).or_default().push((span.0, span.1, String::new()));
    }
    let mut calls = Vec::new();
    for ((kind, index), mut list) in edits {
        let Some(e) = snippet(kind, index) else { continue };
        if e.automatic {
            return Err(automatic(kind, e));
        }
        list.sort_by_key(|(a, b, _)| (*a, *b));
        calls.push(set_code(kind, index, e, edited(&e.code, &list)));
    }
    Ok(calls)
}

/// Does `code` call a lookup it does not define (`lookup NAME;`)?
fn uses_outside_lookup(code: &str) -> bool {
    let toks = tokens(code);
    (0..toks.len()).any(|i| text(code, &toks, i) == Some("lookup") && text(code, &toks, i + 2) == Some(";"))
}

/// Step 37: the `font.features.reorder` parameters that move every `smcp` feature to just
/// before the first `liga`, or why the feature order cannot fix the lookup order.
pub(super) fn reorder_features(f: &Features) -> Result<Value, String> {
    let list = &f.features;
    let smcp: Vec<usize> = list.iter().enumerate().filter(|(_, e)| e.name == "smcp").map(|(i, _)| i).collect();
    let Some(liga) = list.iter().position(|e| e.name == "liga") else { return Err("the feature code has no liga feature".into()) };
    let Some(&first_smcp) = smcp.first() else { return Err("the feature code has no smcp feature".into()) };
    if list.iter().filter(|e| e.name == "smcp" || e.name == "liga").any(|e| uses_outside_lookup(&e.code)) {
        return Err("smcp or liga uses lookups defined elsewhere, so the feature order does not decide the lookup order".into());
    }
    if first_smcp < liga {
        return Err("smcp already comes before liga; the lookup order comes from elsewhere in the feature code".into());
    }
    let mut order: Vec<usize> = Vec::with_capacity(list.len());
    for i in 0..list.len() {
        if i == liga {
            order.extend(&smcp);
        }
        if !smcp.contains(&i) {
            order.push(i);
        }
    }
    Ok(json!({"kind": "feature", "order": order}))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn snippet(name: &str, code: &str) -> Snippet {
        Snippet { name: name.into(), code: code.into(), ..Default::default() }
    }

    #[test]
    fn statements_are_rewritten_in_place() {
        let f = Features {
            prefixes: vec![snippet(
                "languagesystems",
                "# languagesystem latn TUR;\nlanguagesystem latn dflt;\n  languagesystem latn TUR;\nlanguagesystem DFLT dflt;\n",
            )],
            features: vec![snippet("locl", "script latn;\nlanguage TUR exclude_dflt;\nsub i by idotaccent;")],
            ..Default::default()
        };
        let (calls, left) = rename_language_tag(&f, &[("TUR".into(), "TRK".into())]);
        assert!(left.is_empty());
        assert_eq!(
            calls[0]["code"],
            "# languagesystem latn TUR;\nlanguagesystem latn dflt;\n  languagesystem latn TRK;\nlanguagesystem DFLT dflt;\n"
        );
        assert_eq!(calls[1]["code"], "script latn;\nlanguage TRK exclude_dflt;\nsub i by idotaccent;");
        let calls = add_default_languagesystem(&f).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0]["code"],
            "# languagesystem latn TUR;\nlanguagesystem DFLT dflt;\nlanguagesystem latn dflt;\n  languagesystem latn TUR;\n"
        );
    }
}

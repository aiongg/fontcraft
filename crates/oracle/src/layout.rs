//! `GSUB`, `GPOS` and `GDEF` compared lookup by lookup (M8.7).
//!
//! Each font's table is decompiled to feature code by `fontcraft_tables::fea_write` (glyphs by
//! name, the reference's glyphs under FontCraft's names where the two name a glyph differently)
//! and split into its parts: the lines outside lookups and features (language systems, the
//! `table GDEF` block), each lookup with the `markClass` statements written before it, and the
//! feature blocks. A lookup's name carries its index in the lookup list; names are replaced by
//! `L<index>` on both sides, so lookup *n* is compared with lookup *n*.
//!
//! Two tables are equivalent when every part holds the same lines once `subtable;` breaks are
//! dropped and each part's lines are sorted: the same rules, split into subtables or ordered
//! differently. Otherwise each part that differs is one difference, keyed by the part (`lookup
//! 3`, `features`, `outside lookups`), with the first line found on one side only.
//!
//! The glyphs a reference makes of alternate layers (`*.BRACKET.*`) and FontCraft's static font does
//! not have (`alternateLayer.staticExport`) leave the reference's classes, and in `GPOS` and `GDEF` its
//! rules about them alone, before the comparison. Where that takes something away, it is one
//! difference of its own, [`ALTERNATE_GLYPHS`], so that the rest of the table is still compared.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_tables::fea_write::{DecompileOptions, decompile};
use fontcraft_tables::sfnt::read;

use crate::Difference;

/// The key of the difference that says which alternate-layer glyphs left the comparison.
pub const ALTERNATE_GLYPHS: &str = "alternate glyphs";
/// Most alternate-layer glyphs that difference names.
const MAX_NAMED: usize = 6;

/// The parts of one decompiled table.
#[derive(Debug, Default, PartialEq)]
struct Parts {
    outside: Vec<String>,
    lookups: BTreeMap<usize, Vec<String>>,
    features: Vec<String>,
}

/// The table `tag` of `bytes` as feature code; `rename` gives glyph names to write instead.
fn text(bytes: &[u8], tag: &str, rename: &BTreeMap<String, String>) -> Result<String, String> {
    let font = read(bytes, 0).map_err(|e| e.to_string())?;
    let mut t = [b' '; 4];
    for (slot, b) in t.iter_mut().zip(tag.bytes()) {
        *slot = b;
    }
    let options = DecompileOptions { tables: vec![t], rename: rename.clone(), ..DecompileOptions::default() };
    decompile(&font, &options).map_err(|e| e.to_string())
}

/// The index a lookup name ends with (`kern_latn_dflt_3`, `GPOS_3`).
fn index_of(name: &str) -> Option<usize> {
    name.rsplit('_').next()?.parse().ok()
}

/// Splits decompiled text into its parts, lookup names replaced by `L<index>`.
fn parts(text: &str) -> Parts {
    // The names first, so every reference to a lookup can be renamed.
    let mut names: Vec<(String, usize)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("lookup ") {
            let name = rest.split_whitespace().next().unwrap_or("");
            if rest.trim_end().ends_with('{')
                && let Some(i) = index_of(name)
            {
                names.push((name.to_string(), i));
            }
        }
    }
    // Longest first, so `GPOS_1` does not rename part of `GPOS_12`.
    names.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    let rename = |line: &str| {
        let mut out = normalise_values(line);
        for (name, i) in &names {
            if out.contains(name.as_str()) {
                out = out.replace(name.as_str(), &format!("L{i}"));
            }
        }
        out
    };

    let mut out = Parts::default();
    let mut pending: Vec<String> = Vec::new();
    let mut current: Option<(usize, Vec<String>)> = None;
    let mut in_feature = false;
    let mut depth = 0usize;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line == "subtable;" || line.starts_with('#') {
            continue;
        }
        if let Some((index, body)) = current.as_mut() {
            if line.starts_with('}') && line.ends_with(';') && depth == 0 {
                let body = std::mem::take(body);
                out.lookups.insert(*index, body);
                current = None;
            } else {
                depth = depth.saturating_add(line.matches('{').count()).saturating_sub(line.matches('}').count());
                body.push(rename(line));
            }
            continue;
        }
        if in_feature {
            out.features.push(rename(line));
            if line.starts_with('}') && line.ends_with(';') {
                in_feature = false;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("lookup ")
            && line.ends_with('{')
        {
            let name = rest.split_whitespace().next().unwrap_or("");
            let index = index_of(name).unwrap_or(usize::MAX);
            current = Some((index, std::mem::take(&mut pending)));
            depth = 0;
            continue;
        }
        if line.starts_with("feature ") && line.ends_with('{') {
            in_feature = true;
            out.features.push(rename(line));
            continue;
        }
        if line.starts_with("markClass ") {
            pending.push(rename(line));
        } else {
            out.outside.push(rename(line));
        }
    }
    out.outside.extend(pending);
    out
}

/// A rule with its value records in one spelling: empty device tables dropped (a variable
/// reference writes them for its variation indices, compared at the default location) and
/// `<0 0 N 0>`, an advance alone, written as `N`.
fn normalise_values(line: &str) -> String {
    let mut out = line.replace(" <device NULL>", "");
    let mut from = 0;
    while let Some(start) = out.get(from..).and_then(|rest| rest.find("<0 0 ")).map(|i| i + from) {
        let body_start = start + "<0 0 ".len();
        let Some(end) = out.get(body_start..).and_then(|rest| rest.find('>')).map(|i| i + body_start) else { break };
        let body = out.get(body_start..end).unwrap_or("");
        match body.strip_suffix(" 0") {
            Some(n) if !n.is_empty() && n.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()) => {
                let n = n.to_string();
                out.replace_range(start..=end, &n);
                from = start + n.len();
            }
            _ => from = end,
        }
    }
    out
}

/// `line` without the glyphs `dropped` in its classes; `None` when `rules` (a rule about dropped
/// glyphs alone says nothing about the others) and it names one outside a class or is left with
/// an empty class.
fn without(line: &str, dropped: &BTreeSet<String>, rules: bool) -> Option<String> {
    let mut out = String::with_capacity(line.len());
    let mut emptied = false;
    let mut rest = line;
    while let Some((before, after)) = rest.split_once('[') {
        let Some((class, tail)) = after.split_once(']') else { break };
        out.push_str(before);
        let kept: Vec<&str> = class.split_whitespace().filter(|g| !dropped.contains(*g)).collect();
        if kept.is_empty() && !class.trim().is_empty() {
            emptied = true;
        } else {
            out.push('[');
            out.push_str(&kept.join(" "));
            out.push(']');
        }
        rest = tail;
    }
    out.push_str(rest);
    let definition = line.trim_start().starts_with("GlyphClassDef");
    let named = || out.split(|c: char| c.is_whitespace() || matches!(c, '\'' | ';' | ',')).any(|g| dropped.contains(g));
    if rules && !definition && (emptied || named()) {
        return None;
    }
    Some(out)
}

fn sorted(lines: &[String]) -> Vec<&str> {
    let mut v: Vec<&str> = lines.iter().map(String::as_str).collect();
    v.sort_unstable();
    v
}

/// The first line of `a` that `b` lacks, and the reverse.
fn first_unmatched(a: &[String], b: &[String]) -> (String, String) {
    let sa: BTreeSet<&str> = a.iter().map(String::as_str).collect();
    let sb: BTreeSet<&str> = b.iter().map(String::as_str).collect();
    let only = |x: &[String], other: &BTreeSet<&str>| {
        let first = x.iter().find(|l| !other.contains(l.as_str())).cloned();
        first.unwrap_or_else(|| format!("{} lines", x.len()))
    };
    (only(a, &sb), only(b, &sa))
}

fn part(key: String, a: &[String], b: &[String], out: &mut Vec<Difference>) {
    if sorted(a) != sorted(b) {
        let (ours, theirs) = first_unmatched(a, b);
        out.push(Difference { key, ours, theirs });
    }
}

/// The differences between the two fonts' table `tag`, lookup by lookup. `rename` maps the
/// reference's glyph names to FontCraft's; `alternates` are the reference's alternate-layer glyphs
/// that FontCraft's font does not have. An empty list means equivalent.
pub(crate) fn compare(
    tag: &str,
    ours: &[u8],
    theirs: &[u8],
    rename: &BTreeMap<String, String>,
    alternates: &BTreeSet<String>,
) -> Result<Vec<Difference>, String> {
    let a = parts(&text(ours, tag, &BTreeMap::new()).map_err(|e| format!("FontCraft's {tag}: {e}"))?);
    let b = text(theirs, tag, rename).map_err(|e| format!("the reference's {tag}: {e}"))?;
    let mut out = Vec::new();
    let b = if alternates.is_empty() {
        b
    } else {
        let kept: Vec<String> = b.lines().filter_map(|l| without(l, alternates, tag != "GSUB")).collect();
        if !kept.iter().map(String::as_str).eq(b.lines()) {
            let mut names: Vec<&str> = alternates.iter().take(MAX_NAMED).map(String::as_str).collect();
            if alternates.len() > MAX_NAMED {
                names.push("…");
            }
            out.push(Difference { key: ALTERNATE_GLYPHS.into(), ours: "absent".into(), theirs: names.join(", ") });
        }
        kept.join("\n")
    };
    let b = parts(&b);
    part("outside lookups".into(), &a.outside, &b.outside, &mut out);
    let indices: BTreeSet<usize> = a.lookups.keys().chain(b.lookups.keys()).copied().collect();
    for i in indices {
        match (a.lookups.get(&i), b.lookups.get(&i)) {
            (Some(x), Some(y)) => part(format!("lookup {i}"), x, y, &mut out),
            (Some(_), None) => out.push(Difference { key: format!("lookup {i}"), ours: "present".into(), theirs: "absent".into() }),
            (None, Some(_)) => out.push(Difference { key: format!("lookup {i}"), ours: "absent".into(), theirs: "present".into() }),
            (None, None) => {}
        }
    }
    part("features".into(), &a.features, &b.features, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_layer_glyphs_leave_the_classes_and_the_rules_about_them_alone() {
        let dropped: BTreeSet<String> = ["A.BRACKET.varAlt01".to_string()].into();
        let w = |line: &str| without(line, &dropped, true);
        assert_eq!(w("    GlyphClassDef [A A.BRACKET.varAlt01], , [acute], ;").as_deref(), Some("    GlyphClassDef [A], , [acute], ;"));
        assert_eq!(w("    GlyphClassDef [A.BRACKET.varAlt01], , , ;").as_deref(), Some("    GlyphClassDef , , , ;"));
        assert_eq!(w("    pos [A A.BRACKET.varAlt01] V -10;").as_deref(), Some("    pos [A] V -10;"));
        assert_eq!(w("    pos A.BRACKET.varAlt01 V -10;"), None);
        assert_eq!(w("    pos [A.BRACKET.varAlt01] V -10;"), None);
        assert_eq!(w("    pos base A <anchor 0 0> mark @top;").as_deref(), Some("    pos base A <anchor 0 0> mark @top;"));
        assert!(without("    sub A by A.BRACKET.varAlt01;", &dropped, false).is_some(), "a substitution stays a difference");
    }

    #[test]
    fn lookups_are_keyed_by_index_and_compared_without_subtable_breaks() {
        let a = "languagesystem DFLT dflt;\n\nlookup kern_DFLT_dflt_0 {\n    pos A V -80;\n    subtable;\n    pos T o -20;\n} kern_DFLT_dflt_0;\n\nfeature kern {\n    script DFLT;\n    lookup kern_DFLT_dflt_0;\n} kern;\n";
        let b = "languagesystem DFLT dflt;\n\nlookup GPOS_0 {\n    pos T o -20;\n    pos A V -80;\n} GPOS_0;\n\nfeature kern {\n    script DFLT;\n    lookup GPOS_0;\n} kern;\n";
        let (pa, pb) = (parts(a), parts(b));
        assert_eq!(pa.lookups.get(&0).map(|l| sorted(l)), pb.lookups.get(&0).map(|l| sorted(l)));
        assert_eq!(pa.features, pb.features);
        assert_eq!(pa.features.get(2).map(String::as_str), Some("lookup L0;"));
        assert_eq!(normalise_values("pos A V <0 0 -41 0 <device NULL> <device NULL> <device NULL> <device NULL>>;"), "pos A V -41;");
        assert_eq!(normalise_values("pos A V <0 0 -41 5>;"), "pos A V <0 0 -41 5>;");
    }
}

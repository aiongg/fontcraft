//! UFO 2 kerning groups, named as UFO 3 names them.
//!
//! A UFO 2 has no prefixes that mark kerning groups. fontTools' `ufoLib` up-converts one when it
//! reads it (`convertUFO1OrUFO2KerningToUFO3Kerning`, `fontTools/ufoLib/converters.py`, MIT; see
//! `NOTICE`): a group whose name starts with `@MMK_L_` (`@MMK_R_`), or that kerning names as a
//! first (second) side, is a first-side (second-side) kerning group, renamed `public.kern1.`
//! (`public.kern2.`) plus its name without that prefix, with a number added when the new name is
//! taken. A side that is also a glyph's name is the glyph.
//!
//! The model holds the UFO 3 names. The writer works out the same renames from the files it read
//! and writes the old names back, so a UFO 2 keeps its group names.

use std::collections::HashSet;

use fontcraft_plist::{Dict, Value};

/// The old and new names of the kerning groups of a UFO 2, first and second sides, in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Renames {
    pub first: Vec<(String, String)>,
    pub second: Vec<(String, String)>,
}

impl Renames {
    /// The new name of the old group `name` used as a first (`true`) or second side.
    pub fn new_name(&self, first: bool, name: &str) -> Option<&str> {
        let list = if first { &self.first } else { &self.second };
        list.iter().find(|(old, _)| old == name).map(|(_, new)| new.as_str())
    }

    /// The old name of the group that got the new name `name`.
    pub fn old_name(&self, name: &str) -> Option<&str> {
        self.first.iter().chain(&self.second).find(|(_, new)| new == name).map(|(old, _)| old.as_str())
    }

    /// Whether `name` is the old name of a kerning group.
    pub fn is_old(&self, name: &str) -> bool {
        self.first.iter().chain(&self.second).any(|(old, _)| old == name)
    }
}

/// `name`, or `name` with the smallest number after it that no group has.
fn unique(name: String, taken: &HashSet<&str>) -> String {
    if !taken.contains(name.as_str()) {
        return name;
    }
    let mut n = 1u64;
    loop {
        let candidate = format!("{name}{n}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

/// The renames ufoLib makes when it reads a UFO 2 with these groups and kerning; `glyphs` are the
/// names of the glyphs in its default layer.
pub(crate) fn ufo2_renames(groups: &Dict, kerning: Option<&Dict>, glyphs: &HashSet<&str>) -> Renames {
    let names: HashSet<&str> = groups.entries.iter().map(|(k, _)| k.as_str()).collect();
    let (mut first, mut second): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let (mut seen_first, mut seen_second): (HashSet<&str>, HashSet<&str>) = (HashSet::new(), HashSet::new());
    for (k, _) in &groups.entries {
        let k = k.as_str();
        if k.starts_with("@MMK_L_") && seen_first.insert(k) {
            first.push(k);
        }
        if k.starts_with("@MMK_R_") && seen_second.insert(k) {
            second.push(k);
        }
    }
    for (f, seconds) in kerning.map(|k| k.entries.as_slice()).unwrap_or_default() {
        let f = f.as_str();
        if names.contains(f) && !glyphs.contains(f) && !f.starts_with("public.kern1.") && seen_first.insert(f) {
            first.push(f);
        }
        for (s, _) in seconds.as_dict().map(|d| d.entries.as_slice()).unwrap_or_default() {
            let s = s.as_str();
            if names.contains(s) && !glyphs.contains(s) && !s.starts_with("public.kern2.") && seen_second.insert(s) {
                second.push(s);
            }
        }
    }
    let rename = |old: &str, prefix: &str, marker: &str| (old.to_string(), unique(format!("{prefix}{}", old.replace(marker, "")), &names));
    Renames {
        first: first.into_iter().map(|g| rename(g, "public.kern1.", "@MMK_L_")).collect(),
        second: second.into_iter().map(|g| rename(g, "public.kern2.", "@MMK_R_")).collect(),
    }
}

/// Whether a `groups.plist` value is a list of glyph names.
pub(crate) fn is_name_list(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.items.iter().all(|i| i.as_str().is_some()))
}

/// The kinds of kerning data [`losses`] reports, as its lines begin.
pub const KERNING_LOSSES: [&str; 4] =
    ["right-to-left kerning pairs", "vertical kerning pairs", "contextual kerning", "the glyphs' top and bottom kerning groups"];

/// What a UFO's `kerning.plist` and `groups.plist` have no place for, one line per kind with a
/// count: a UFO holds left-to-right pairs and first and second side groups only.
pub fn losses(font: &fontcraft_model::Font) -> Vec<String> {
    let k = &font.kerning;
    let counts = [
        k.rtl.pair_count(),
        k.vertical.pair_count(),
        k.context.len(),
        font.glyphs.iter().filter(|g| g.kern_groups.top.is_some() || g.kern_groups.bottom.is_some()).count(),
    ];
    KERNING_LOSSES.iter().zip(counts).filter(|(_, n)| *n > 0).map(|(what, n)| format!("{what} ({n})")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_plist::{Array, Str};

    fn list(names: &[&str]) -> Value {
        Value::Array(Array { items: names.iter().map(|n| Value::String(Str::new(*n))).collect(), ..Default::default() })
    }

    fn dict(entries: Vec<(&str, Value)>) -> Dict {
        let mut d = Dict::new();
        for (k, v) in entries {
            d.entries.push((Str::new(k), v));
        }
        d
    }

    #[test]
    fn ufo2_groups_are_renamed_as_ufolib_renames_them() {
        let groups = dict(vec![
            ("@MMK_L_A", list(&["A", "Aacute"])),
            ("@MMK_R_O", list(&["O"])),
            ("round", list(&["o"])),
            ("public.kern2.round", list(&[])),
            ("caps", list(&["A"])),
            ("o", list(&["o"])),
        ]);
        let kerning = dict(vec![("@MMK_L_A", Value::Dict(dict(vec![("round", Value::Dict(Dict::new()))]))), ("o", Value::Dict(Dict::new()))]);
        let glyphs: HashSet<&str> = ["A", "Aacute", "O", "o"].into_iter().collect();
        let r = ufo2_renames(&groups, Some(&kerning), &glyphs);
        assert_eq!(r.first, [("@MMK_L_A".to_string(), "public.kern1.A".to_string())]);
        assert_eq!(
            r.second,
            [("@MMK_R_O".to_string(), "public.kern2.O".to_string()), ("round".to_string(), "public.kern2.round1".to_string())],
            "a taken name gets a number; a glyph's name is the glyph"
        );
        assert_eq!(r.new_name(true, "@MMK_L_A"), Some("public.kern1.A"));
        assert_eq!(r.old_name("public.kern2.round1"), Some("round"));
        assert!(r.is_old("@MMK_R_O") && !r.is_old("caps"));
    }
}

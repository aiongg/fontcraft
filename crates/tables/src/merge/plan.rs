//! The glyph order of the result: which glyph of which font fills each place, and where each
//! input glyph went. The renaming follows fontTools' `computeMegaGlyphOrder`, and the names of
//! each font its `post` table's `build_psNameMapping` (fontTools 4.66.1, MIT License).

use std::collections::{HashMap, HashSet};

use super::Duplicates;
use crate::limits::MAX_GLYPHS;
use crate::reorder::names::{num_glyphs, post_names};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// One glyph of the result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Slot {
    pub font: usize,
    pub gid: u16,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Plan {
    pub slots: Vec<Slot>,
    /// Per font: the new glyph id of each old one; `None` for a glyph the result does not take.
    pub maps: Vec<Vec<Option<u16>>>,
    pub conflicts: Vec<String>,
}

impl Plan {
    /// The new id of glyph `gid` of font `font`.
    pub fn new_id(&self, font: usize, gid: u16) -> Option<u16> {
        self.maps.get(font).and_then(|m| m.get(usize::from(gid))).copied().flatten()
    }

    /// Every glyph of `font` is in the result, each in its own place.
    pub fn takes_all_of(&self, font: usize) -> bool {
        let Some(map) = self.maps.get(font) else { return false };
        let mut seen = std::collections::HashSet::with_capacity(map.len());
        map.iter().all(|m| m.is_some_and(|g| seen.insert(g)))
    }
}

/// A TrueType font's glyph names as fontTools' `post` reader gives them, and the stored name of
/// each it had to change (`build_psNameMapping`): a name repeated in the font gets `.1`, `.2`…,
/// skipping suffixed names the font already has. Without stored names, glyphs are `glyphNNNNN`
/// (glyph 0 `.notdef`).
pub(super) fn font_glyph_names(font: &SfntFont) -> Result<(Vec<String>, HashMap<String, String>)> {
    let n = num_glyphs(font)?;
    let stored = match font.get(*b"post") {
        Some(post) => post_names(post, n)?,
        None => None,
    };
    let Some(stored) = stored else {
        return Ok(((0..n).map(|g| if g == 0 { ".notdef".to_string() } else { format!("glyph{g:05}") }).collect(), HashMap::new()));
    };
    let in_font: HashSet<&str> = stored.iter().map(String::as_str).collect();
    let mut next: HashMap<String, usize> = HashMap::with_capacity(stored.len());
    let mut mapping = HashMap::new();
    let mut names = Vec::with_capacity(stored.len());
    for ps in &stored {
        let mut name = ps.clone();
        if let Some(&first) = next.get(&name) {
            // The search ends: each step skips a distinct name of the font.
            let mut k = first;
            while in_font.contains(format!("{name}.{k}").as_str()) {
                k += 1;
            }
            next.insert(name.clone(), k + 1);
            name = format!("{name}.{k}");
        }
        next.insert(name.clone(), 1);
        if name != *ps {
            mapping.insert(name.clone(), ps.clone());
        }
        names.push(name);
    }
    Ok((names, mapping))
}

/// The plan for fonts whose glyph names are `names`. `same(a, b)` says whether two input glyphs
/// (font, glyph id) are identical.
pub(super) fn plan(
    names: &[Vec<String>],
    glyph_maps: &[Vec<(String, String)>],
    duplicates: Duplicates,
    same: &dyn Fn((usize, u16), (usize, u16)) -> bool,
) -> Result<Plan> {
    let map_form = glyph_maps.iter().any(|m| !m.is_empty());
    let mut plan = Plan { maps: names.iter().map(|n| vec![None; n.len()]).collect(), ..Plan::default() };
    // Name → slot, and the next suffix to try for a name (fontTools' megaOrder counter).
    let mut by_name: HashMap<String, usize> = HashMap::new();
    let mut counters: HashMap<String, usize> = HashMap::new();

    for (font, font_names) in names.iter().enumerate() {
        let index: HashMap<&str, u16> = font_names.iter().enumerate().filter_map(|(g, n)| Some((n.as_str(), u16::try_from(g).ok()?))).collect();
        let requests: Vec<(String, u16)> = match glyph_maps.get(font).filter(|m| !m.is_empty()) {
            Some(map) => map
                .iter()
                .map(|(new, source)| {
                    let gid = index
                        .get(source.as_str())
                        .copied()
                        .ok_or_else(|| TablesError::BadOptions(format!("font {} has no glyph `{source}` (named in its map)", font + 1)))?;
                    Ok((new.clone(), gid))
                })
                .collect::<Result<_>>()?,
            None => font_names.iter().enumerate().filter_map(|(g, n)| Some((n.clone(), u16::try_from(g).ok()?))).collect(),
        };
        for (name, gid) in requests {
            let slot = match by_name.get(&name).copied() {
                None => push(&mut plan, &mut by_name, &mut counters, font, gid, name)?,
                Some(existing) => {
                    let earlier = plan.slots.get(existing).map(|s| (s.font, s.gid)).unwrap_or_default();
                    if earlier == (font, gid) || (duplicates != Duplicates::Rename && same(earlier, (font, gid))) {
                        existing
                    } else {
                        match duplicates {
                            Duplicates::Rename => {
                                let mut n = counters.get(&name).copied().unwrap_or(1);
                                while by_name.contains_key(&format!("{name}.{n}")) {
                                    n += 1;
                                }
                                counters.insert(name.clone(), n);
                                let renamed = format!("{name}.{n}");
                                plan.conflicts.push(format!("`{name}` of font {} is renamed `{renamed}`", font + 1));
                                push(&mut plan, &mut by_name, &mut counters, font, gid, renamed)?
                            }
                            Duplicates::First => {
                                plan.conflicts.push(format!("`{name}` of font {} is left out: font {} supplies it", font + 1, earlier.0 + 1));
                                existing
                            }
                            Duplicates::Last => {
                                plan.conflicts.push(format!("`{name}` of font {} replaces the glyph of font {}", font + 1, earlier.0 + 1));
                                if let Some(s) = plan.slots.get_mut(existing) {
                                    s.font = font;
                                    s.gid = gid;
                                }
                                existing
                            }
                            Duplicates::Error => {
                                return Err(TablesError::BadOptions(format!(
                                    "fonts {} and {} both have a glyph `{name}` and the two differ",
                                    earlier.0 + 1,
                                    font + 1
                                )));
                            }
                        }
                    }
                }
            };
            if let Some(entry) = plan.maps.get_mut(font).and_then(|m| m.get_mut(usize::from(gid)))
                && entry.is_none()
            {
                *entry = u16::try_from(slot).ok();
            }
        }
    }
    if map_form {
        notdef_first(&mut plan)?;
    }
    Ok(plan)
}

fn push(
    plan: &mut Plan,
    by_name: &mut HashMap<String, usize>,
    counters: &mut HashMap<String, usize>,
    font: usize,
    gid: u16,
    name: String,
) -> Result<usize> {
    let slot = plan.slots.len();
    if slot >= MAX_GLYPHS {
        return Err(TablesError::TooLarge { what: "the merged glyph count", limit: MAX_GLYPHS });
    }
    by_name.insert(name.clone(), slot);
    counters.insert(name.clone(), 1);
    plan.slots.push(Slot { font, gid, name });
    Ok(slot)
}

/// Moves `.notdef` to glyph 0, as every font needs it there.
fn notdef_first(plan: &mut Plan) -> Result<()> {
    let at = plan
        .slots
        .iter()
        .position(|s| s.name == ".notdef")
        .ok_or_else(|| TablesError::BadOptions("the result has no `.notdef`: one of the maps must take it".into()))?;
    if at == 0 {
        return Ok(());
    }
    let slot = plan.slots.remove(at);
    plan.slots.insert(0, slot);
    for map in &mut plan.maps {
        for entry in map.iter_mut().flatten() {
            let old = usize::from(*entry);
            *entry = if old == at {
                0
            } else if old < at {
                *entry + 1
            } else {
                *entry
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn names(list: &[&[&str]]) -> Vec<Vec<String>> {
        list.iter().map(|n| n.iter().map(|s| s.to_string()).collect()).collect()
    }

    #[test]
    fn clashing_names_get_numeric_suffixes_as_pyftmerge_gives_them() {
        let n = names(&[&[".notdef", "A", "A.1"], &[".notdef", "A", "B"], &[".notdef", "A"]]);
        let p = plan(&n, &[], Duplicates::Rename, &|_, _| false).unwrap();
        let order: Vec<&str> = p.slots.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, [".notdef", "A", "A.1", ".notdef.1", "A.2", "B", ".notdef.2", "A.3"]);
        assert_eq!(p.maps[1], [Some(3), Some(4), Some(5)]);
    }

    #[test]
    fn maps_select_rename_and_put_notdef_first() {
        let n = names(&[&[".notdef", "A", "B"], &[".notdef", "a"]]);
        let maps = vec![vec![("A".to_string(), "A".to_string())], vec![("Alt".to_string(), "a".to_string()), (".notdef".into(), ".notdef".into())]];
        let p = plan(&n, &maps, Duplicates::First, &|_, _| false).unwrap();
        let order: Vec<(&str, usize, u16)> = p.slots.iter().map(|s| (s.name.as_str(), s.font, s.gid)).collect();
        assert_eq!(order, [(".notdef", 1, 0), ("A", 0, 1), ("Alt", 1, 1)]);
        assert_eq!(p.maps[0], [None, Some(1), None]);
        assert_eq!(p.maps[1], [Some(0), Some(2)]);

        let no_notdef = vec![vec![("A".to_string(), "A".to_string())], vec![("Alt".to_string(), "a".to_string())]];
        assert!(plan(&n, &no_notdef, Duplicates::First, &|_, _| false).is_err());
    }

    #[test]
    fn duplicate_policies() {
        let n = names(&[&[".notdef", "A"], &[".notdef", "A"]]);
        let first = plan(&n, &[], Duplicates::First, &|_, _| false).unwrap();
        assert_eq!(first.slots.len(), 2);
        assert_eq!(first.maps[1], [Some(0), Some(1)]);
        let last = plan(&n, &[], Duplicates::Last, &|_, _| false).unwrap();
        assert_eq!((last.slots[1].font, last.slots[1].gid), (1, 1));
        assert!(plan(&n, &[], Duplicates::Error, &|_, _| false).is_err());
        let shared = plan(&n, &[], Duplicates::Error, &|_, _| true).unwrap();
        assert_eq!(shared.slots.len(), 2, "identical glyphs are shared silently");
        assert!(shared.conflicts.is_empty());
    }
}

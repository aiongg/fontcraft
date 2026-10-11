//! Kerning: pair values per master, in three directions, and version 4's contextual kerning.
//!
//! Sides are kept as the source spells them: a glyph name, or a group with the source's prefix
//! (`@MMK_L_A`, `@MMK_R_V` in the reference's format, where the right-to-left table swaps the two
//! prefixes). [`side_group`] reads a side either way. Tables keep the order they were read in at
//! every level, because the reference's files keep kerning in the order it was made, not sorted.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Kerning {
    #[serde(default, skip_serializing_if = "KernTable::is_empty")]
    pub ltr: KernTable,
    #[serde(default, skip_serializing_if = "KernTable::is_empty")]
    pub rtl: KernTable,
    #[serde(default, skip_serializing_if = "KernTable::is_empty")]
    pub vertical: KernTable,
    /// Contextual kerning (`.glyphs` version 4 `kerningContext`), in file order. No document
    /// describes the syntax of a context, so the model keeps each one as stored and does not
    /// interpret it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<KernContext>,
}

impl Kerning {
    pub fn is_empty(&self) -> bool {
        self.ltr.is_empty() && self.rtl.is_empty() && self.vertical.is_empty() && self.context.is_empty()
    }
}

/// One kerning context and its value in each master that has one, in file order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KernContext {
    /// The context as the file spells it.
    pub context: String,
    /// Master id → value.
    pub values: Vec<(String, f64)>,
}

/// One direction's pairs, per master, in source order. A table holds at most one entry per
/// master: files can repeat a master, so every loader builds its table with [`KernTable::new`],
/// and deserializing merges the same way.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawKernTable")]
pub struct KernTable(pub Vec<MasterKerning>);

/// [`KernTable`]'s stored form, before its entries are merged.
#[derive(Deserialize)]
struct RawKernTable(Vec<MasterKerning>);

impl From<RawKernTable> for KernTable {
    fn from(raw: RawKernTable) -> Self {
        KernTable::new(raw.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MasterKerning {
    pub master: String,
    pub firsts: Vec<KernFirst>,
}

/// The pairs that start with one side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KernFirst {
    pub first: String,
    pub seconds: Vec<(String, f64)>,
}

impl KernTable {
    /// A table from entries as a file stores them, with each master's later entries merged into
    /// its first: first sides keep the order they first appear in, and a first side met again
    /// appends its pairs to the earlier one's. A pair stored twice keeps both copies in stored
    /// order, so a lookup that takes the first match finds the value it found before the merge.
    pub fn new(entries: Vec<MasterKerning>) -> Self {
        let mut out: Vec<MasterKerning> = Vec::with_capacity(entries.len());
        let mut masters: HashMap<String, usize> = HashMap::new();
        let mut firsts: Vec<HashMap<String, usize>> = Vec::with_capacity(entries.len());
        for m in entries {
            let mi = match masters.get(&m.master) {
                Some(i) => *i,
                None => {
                    masters.insert(m.master.clone(), out.len());
                    out.push(MasterKerning { master: m.master, firsts: Vec::new() });
                    firsts.push(HashMap::new());
                    out.len() - 1
                }
            };
            let (Some(target), Some(at)) = (out.get_mut(mi), firsts.get_mut(mi)) else { continue };
            for f in m.firsts {
                match at.get(&f.first).and_then(|i| target.firsts.get_mut(*i)) {
                    Some(earlier) => earlier.seconds.extend(f.seconds),
                    None => {
                        at.insert(f.first.clone(), target.firsts.len());
                        target.firsts.push(f);
                    }
                }
            }
        }
        KernTable(out)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of pairs over all masters.
    pub fn pair_count(&self) -> usize {
        self.0.iter().flat_map(|m| &m.firsts).map(|f| f.seconds.len()).sum()
    }

    pub fn get(&self, master: &str, first: &str, second: &str) -> Option<f64> {
        let m = self.0.iter().find(|m| m.master == master)?;
        let f = m.firsts.iter().find(|f| f.first == first)?;
        f.seconds.iter().find(|(s, _)| s == second).map(|(_, v)| *v)
    }

    /// Sets a pair, appending the master, first side or pair where they are new.
    pub fn set(&mut self, master: &str, first: &str, second: &str, value: f64) {
        let mi = match self.0.iter().position(|m| m.master == master) {
            Some(i) => i,
            None => {
                self.0.push(MasterKerning { master: master.to_string(), firsts: Vec::new() });
                self.0.len() - 1
            }
        };
        let Some(m) = self.0.get_mut(mi) else { return };
        let fi = match m.firsts.iter().position(|f| f.first == first) {
            Some(i) => i,
            None => {
                m.firsts.push(KernFirst { first: first.to_string(), seconds: Vec::new() });
                m.firsts.len() - 1
            }
        };
        let Some(f) = m.firsts.get_mut(fi) else { return };
        match f.seconds.iter_mut().find(|(s, _)| s == second) {
            Some(slot) => slot.1 = value,
            None => f.seconds.push((second.to_string(), value)),
        }
    }

    /// Removes a pair; empty first sides and masters are removed with it.
    pub fn remove(&mut self, master: &str, first: &str, second: &str) -> Option<f64> {
        let mi = self.0.iter().position(|m| m.master == master)?;
        let m = self.0.get_mut(mi)?;
        let fi = m.firsts.iter().position(|f| f.first == first)?;
        let f = m.firsts.get_mut(fi)?;
        let si = f.seconds.iter().position(|(s, _)| s == second)?;
        let (_, value) = f.seconds.remove(si);
        if f.seconds.is_empty() {
            m.firsts.remove(fi);
        }
        if m.firsts.is_empty() {
            self.0.remove(mi);
        }
        Some(value)
    }
}

/// The group a kerning side names, if it is a group: the reference writes `@MMK_L_` before the
/// group name of a left side and `@MMK_R_` before a right side's (top and bottom: `@MMK_T_`,
/// `@MMK_B_`); a UFO writes `public.kern1.` before a first side's group and `public.kern2.` before
/// a second side's.
pub fn side_group(side: &str) -> Option<&str> {
    ["@MMK_L_", "@MMK_R_", "@MMK_T_", "@MMK_B_", "public.kern1.", "public.kern2."].iter().find_map(|p| side.strip_prefix(p))
}

/// A kerning side as the reference's format spells it: a UFO group (`public.kern1.A`,
/// `public.kern2.O`) becomes `@MMK_L_A`, `@MMK_R_O`; anything else is kept.
pub fn glyphs_side(side: &str) -> std::borrow::Cow<'_, str> {
    if let Some(g) = side.strip_prefix("public.kern1.") {
        format!("@MMK_L_{g}").into()
    } else if let Some(g) = side.strip_prefix("public.kern2.") {
        format!("@MMK_R_{g}").into()
    } else {
        side.into()
    }
}

/// A kerning side as a UFO spells it: `@MMK_L_A` becomes `public.kern1.A` and `@MMK_R_O`
/// `public.kern2.O`; anything else is kept.
pub fn ufo_side(side: &str) -> std::borrow::Cow<'_, str> {
    if let Some(g) = side.strip_prefix("@MMK_L_") {
        format!("public.kern1.{g}").into()
    } else if let Some(g) = side.strip_prefix("@MMK_R_") {
        format!("public.kern2.{g}").into()
    } else {
        side.into()
    }
}

/// A glyph's kerning groups: the group it belongs to on each side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KernGroups {
    /// The group for pairs where this glyph is on the right (its left side faces the pair).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    /// The group for pairs where this glyph is on the left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<String>,
}

impl KernGroups {
    pub fn is_empty(&self) -> bool {
        self.left.is_none() && self.right.is_none() && self.top.is_none() && self.bottom.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_are_set_found_and_removed_in_order() {
        let mut t = KernTable::default();
        t.set("m01", "@MMK_L_A", "@MMK_R_V", -50.0);
        t.set("m01", "@MMK_L_A", "T", -30.0);
        t.set("m01", "@MMK_L_A", "@MMK_R_V", -60.0);
        t.set("m02", "F", "a", -10.0);
        assert_eq!(t.get("m01", "@MMK_L_A", "@MMK_R_V"), Some(-60.0));
        assert_eq!(t.pair_count(), 3);
        let seconds: Vec<&str> = t.0[0].firsts[0].seconds.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(seconds, ["@MMK_R_V", "T"], "insertion order is kept");
        assert_eq!(t.remove("m02", "F", "a"), Some(-10.0));
        assert_eq!(t.0.len(), 1, "an emptied master is removed");
        assert_eq!(t.remove("m02", "F", "a"), None);
    }

    #[test]
    fn a_master_stored_twice_becomes_one_entry_in_stored_order() {
        let first = |name: &str, seconds: &[(&str, f64)]| KernFirst {
            first: name.into(),
            seconds: seconds.iter().map(|(s, v)| (s.to_string(), *v)).collect(),
        };
        let entries = vec![
            MasterKerning { master: "m01".into(), firsts: vec![first("A", &[("V", -50.0)]), first("T", &[("o", -70.0)])] },
            MasterKerning { master: "m02".into(), firsts: vec![first("A", &[("V", -10.0)])] },
            MasterKerning { master: "m01".into(), firsts: vec![first("F", &[("a", -20.0)]), first("A", &[("V", -99.0), ("W", -40.0)])] },
        ];
        let json = serde_json::to_string(&KernTable(entries.clone())).unwrap();
        let mut t = KernTable::new(entries);
        assert_eq!(serde_json::from_str::<KernTable>(&json).unwrap(), t, "deserializing merges too");
        let masters: Vec<&str> = t.0.iter().map(|m| m.master.as_str()).collect();
        assert_eq!(masters, ["m01", "m02"]);
        let m01: Vec<(&str, &str, f64)> =
            t.0[0].firsts.iter().flat_map(|f| f.seconds.iter().map(|(s, v)| (f.first.as_str(), s.as_str(), *v))).collect();
        assert_eq!(m01, [("A", "V", -50.0), ("A", "V", -99.0), ("A", "W", -40.0), ("T", "o", -70.0), ("F", "a", -20.0)]);
        assert_eq!(t.get("m01", "A", "V"), Some(-50.0), "the earlier copy of a pair stored twice wins");
        assert_eq!(t.get("m01", "A", "W"), Some(-40.0));
        assert_eq!(t.get("m01", "F", "a"), Some(-20.0));
        t.set("m01", "F", "a", -25.0);
        assert_eq!(t.0.len(), 2, "setting a pair of the later entry adds no entry");
        assert_eq!(t.remove("m01", "F", "a"), Some(-25.0));
        assert_eq!(t.get("m01", "F", "a"), None);
    }

    #[test]
    fn sides_are_spelled_per_format() {
        assert_eq!(ufo_side("@MMK_L_A"), "public.kern1.A");
        assert_eq!(ufo_side("@MMK_R_O"), "public.kern2.O");
        assert_eq!(glyphs_side("public.kern1.A"), "@MMK_L_A");
        assert_eq!(glyphs_side(&ufo_side("@MMK_R_O")), "@MMK_R_O");
        assert_eq!(ufo_side("T"), "T");
    }

    #[test]
    fn group_sides_are_recognised() {
        assert_eq!(side_group("@MMK_L_A"), Some("A"));
        assert_eq!(side_group("@MMK_R_round"), Some("round"));
        assert_eq!(side_group("public.kern2.O"), Some("O"));
        assert_eq!(side_group("A"), None);
    }
}

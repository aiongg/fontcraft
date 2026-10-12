//! The legacy `kern` table (format 0), written for TrueType exports when the `Export kern Table`
//! parameter is on: the master's kerning with groups expanded into glyph pairs, exceptions first,
//! renamed and removed glyphs ([`ExportEdits`]) followed.
//!
//! Only the master's left-to-right kerning is written. Each glyph pair gets the value that
//! applies to it (an exception overrides its groups' pair), rounded; pairs whose value is 0 are
//! left out. Stored pairs are expanded in lookup order (glyph–glyph, glyph–group, group–glyph,
//! group–group), so when the table is full the exceptions are the pairs that stay.
//!
//! A subtable holds at most [`fontcraft_fontinfo::MAX_KERN_SUBTABLE_PAIRS`] pairs (its 16-bit
//! length) and the table [`fontcraft_fontinfo::MAX_KERN_PAIRS`]; the rest are dropped with a
//! warning. Subtables follow each other in glyph order and never share a pair, since a reader
//! adds up the values of every subtable. CFF exports get no `kern` table.
//!
//! Row: `Export kern Table` (M7.11).

use std::collections::{BTreeMap, HashMap};

use fontcraft_fontinfo::{MAX_KERN_PAIRS, MAX_KERN_SUBTABLE_PAIRS};
use fontcraft_model::{Font, Resolved, ResolvedGlyph, param_flag, side_group};
use fontcraft_spacing::kerning::{Direction, GroupSide, KernIndex, Position, facing};

use crate::export_params::ExportEdits;
use crate::outline::Flavour;

/// The parameter that turns the table on.
pub const PARAMETER: &str = "Export kern Table";

/// Most steps taken while groups are expanded (one per stored pair, per group member listed and
/// per glyph pair looked at); past it the rest are left out with a warning. Two large groups kerned
/// against each other expand to the product of their sizes.
const MAX_EXPANSION_WORK: usize = 4_000_000;

/// Bytes of a format 0 subtable before its pairs: the subtable header and the search fields.
const SUBTABLE_HEADER_LEN: usize = 14;
/// Bytes of one pair: left glyph, right glyph, value.
const PAIR_LEN: usize = 6;

/// The `kern` table's bytes for the glyphs `glyphs` (the final glyph order), or `None` when the
/// parameter is off, the flavour is CFF or there is no kerning.
pub fn build(
    font: &Font,
    resolved: &Resolved,
    glyphs: &[ResolvedGlyph],
    flavour: Flavour,
    edits: &ExportEdits,
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    if !resolved.params.get(PARAMETER).and_then(param_flag).unwrap_or(false) {
        return None;
    }
    if matches!(flavour, Flavour::Cff) {
        warnings.push(format!("{PARAMETER} applies to TrueType exports only: no kern table was written"));
        return None;
    }
    let order: Vec<&str> = glyphs.iter().map(|g| g.name.as_str()).collect();
    let pairs = pairs(font, &resolved.master, &order, edits, warnings);
    if pairs.is_empty() {
        return None;
    }
    Some(encode(&pairs))
}

/// The kerning of `master` as glyph pairs: (left glyph id, right glyph id) in `order` → value.
/// `order` holds the exported names; `edits` maps the source's names to them.
pub(crate) fn pairs(font: &Font, master: &str, order: &[&str], edits: &ExportEdits, warnings: &mut Vec<String>) -> BTreeMap<(u16, u16), i16> {
    let mut out = BTreeMap::new();
    let index = KernIndex::new(font);
    let Ok(table) = index.table(Direction::Ltr, master) else {
        return out;
    };
    let mut gids: HashMap<&str, u16> = HashMap::with_capacity(order.len());
    for (i, name) in order.iter().enumerate() {
        if let Ok(gid) = u16::try_from(i) {
            gids.entry(*name).or_insert(gid);
        }
    }
    let exported = |source: &str| edits.exported_name(source).and_then(|name| gids.get(name).copied());

    let mut stored: Vec<(&str, &str)> = table.pairs().map(|(first, second, _)| (first, second)).collect();
    stored.sort_by(|a, b| (rank(a), a.0, a.1).cmp(&(rank(b), b.0, b.1)));
    let first_side = facing(Direction::Ltr, Position::First);
    let second_side = facing(Direction::Ltr, Position::Second);

    let mut work = 0usize;
    let (mut full, mut cut_short) = (false, false);
    'stored: for (first, second) in stored {
        let lefts = side_glyphs(&index, first, first_side);
        let rights = side_glyphs(&index, second, second_side);
        // Counted even when no pair results: many stored pairs of one large group against empty
        // ones would otherwise list its members again and again for free.
        work = work.saturating_add(lefts.len().saturating_add(rights.len()).saturating_add(1));
        if work > MAX_EXPANSION_WORK {
            cut_short = true;
            break;
        }
        for left in &lefts {
            let (Some(l), Some(li)) = (exported(left), index.glyph_id(left)) else { continue };
            for right in &rights {
                work = work.saturating_add(1);
                if work > MAX_EXPANSION_WORK {
                    cut_short = true;
                    break 'stored;
                }
                let (Some(r), Some(ri)) = (exported(right), index.glyph_id(right)) else { continue };
                if out.contains_key(&(l, r)) {
                    continue;
                }
                // The value that applies, whichever stored pair supplies it.
                let Some(applied) = table.lookup(li, ri) else { continue };
                let value = crate::i16_of(applied.value);
                if value == 0 {
                    continue;
                }
                if out.len() >= MAX_KERN_PAIRS {
                    full = true;
                    break 'stored;
                }
                out.insert((l, r), value);
            }
        }
    }
    if full {
        warnings.push(format!("the kern table holds at most {MAX_KERN_PAIRS} pairs: the other pairs were left out of it"));
    }
    if cut_short {
        warnings.push(format!(
            "expanding the kerning groups takes more than {MAX_EXPANSION_WORK} steps: the kern table holds the pairs found before that"
        ));
    }
    out
}

/// Lookup order of a stored pair: glyph–glyph, glyph–group, group–glyph, group–group.
fn rank(pair: &(&str, &str)) -> u8 {
    match (side_group(pair.0).is_some(), side_group(pair.1).is_some()) {
        (false, false) => 0,
        (false, true) => 1,
        (true, false) => 2,
        (true, true) => 3,
    }
}

/// The glyphs a stored side names: a group's members on `side`, in glyph order, or the glyph.
fn side_glyphs<'a>(index: &'a KernIndex, stored: &'a str, side: GroupSide) -> Vec<&'a str> {
    match side_group(stored) {
        Some(group) => index.members(side, group).collect(),
        None => vec![stored],
    }
}

/// A version 0 `kern` table of format 0 subtables holding `pairs`, sorted by glyph pair.
pub(crate) fn encode(pairs: &BTreeMap<(u16, u16), i16>) -> Vec<u8> {
    let entries: Vec<(&(u16, u16), &i16)> = pairs.iter().collect();
    let chunks: Vec<&[(&(u16, u16), &i16)]> = entries.chunks(MAX_KERN_SUBTABLE_PAIRS).collect();
    let mut out = Vec::with_capacity(4 + chunks.len().saturating_mul(SUBTABLE_HEADER_LEN) + entries.len().saturating_mul(PAIR_LEN));
    let push = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_be_bytes());
    push(&mut out, 0);
    push(&mut out, u16::try_from(chunks.len()).unwrap_or(u16::MAX));
    for chunk in chunks {
        let n = u16::try_from(chunk.len()).unwrap_or(u16::MAX);
        let length = chunk.len().saturating_mul(PAIR_LEN).saturating_add(SUBTABLE_HEADER_LEN);
        let (search_range, entry_selector, range_shift) = search_fields(n);
        push(&mut out, 0);
        push(&mut out, u16::try_from(length).unwrap_or(u16::MAX));
        // Horizontal kerning, format 0 in the high byte.
        push(&mut out, 0x0001);
        push(&mut out, n);
        push(&mut out, search_range);
        push(&mut out, entry_selector);
        push(&mut out, range_shift);
        for ((left, right), value) in chunk {
            push(&mut out, *left);
            push(&mut out, *right);
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
    out
}

/// `searchRange`, `entrySelector` and `rangeShift` for `n` pairs of 6 bytes.
fn search_fields(n: u16) -> (u16, u16, u16) {
    if n == 0 {
        return (0, 0, 0);
    }
    let selector = 15u32.saturating_sub(n.leading_zeros());
    let power = 1u32 << selector;
    let search_range = power.saturating_mul(6);
    let range_shift = (u32::from(n) * 6).saturating_sub(search_range);
    let short = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    (short(search_range), short(selector), short(range_shift))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use fontcraft_model::plist::Value;
    use fontcraft_model::{Font, Param};
    use fontcraft_spacing::kerning::GroupSide;
    use skrifa::FontRef;
    use skrifa::raw::TableProvider;
    use skrifa::raw::tables::kern::SubtableKind;

    use super::*;
    use crate::{Options, compile, compile_ttf};

    fn group(f: &mut Font, glyph: &str, side: GroupSide, name: &str) {
        let mut g = f.glyph_mut(glyph).unwrap();
        match side {
            GroupSide::Left => g.kern_groups.left = Some(name.into()),
            _ => g.kern_groups.right = Some(name.into()),
        }
    }

    fn turn_on(f: &mut Font) {
        Arc::make_mut(&mut f.info).params.0.push(Param::new(PARAMETER, Value::Bool(true)));
    }

    /// The `A`/`V` class font: `A` and `Aacute` share the right-side group `A`, `V` and `W` the
    /// left-side group `V`.
    fn class_font() -> Font {
        let mut f = Font::new("Kern Table Sans");
        f.add_glyph("Aacute", vec![0xC1], 600.0).unwrap();
        for (glyph, side, name) in
            [("A", GroupSide::Right, "A"), ("Aacute", GroupSide::Right, "A"), ("V", GroupSide::Left, "V"), ("W", GroupSide::Left, "V")]
        {
            group(&mut f, glyph, side, name);
        }
        let kerning = Arc::make_mut(&mut f.kerning);
        for (first, second, value) in
            [("@MMK_L_A", "@MMK_R_V", -50.0), ("A", "W", -20.0), ("Aacute", "V", 0.0), ("T", "o", -60.4), ("T", "@MMK_R_V", 0.0)]
        {
            kerning.ltr.set("m01", first, second, value);
        }
        f
    }

    /// Every format 0 subtable's pairs, by glyph name, and the subtables' sizes.
    fn read_back(bytes: &[u8], order: &[String]) -> (BTreeMap<(String, String), i32>, Vec<usize>) {
        let font = FontRef::new(bytes).unwrap();
        let kern = font.kern().unwrap();
        let (mut pairs, mut sizes) = (BTreeMap::new(), Vec::new());
        for subtable in kern.subtables() {
            let subtable = subtable.unwrap();
            assert!(subtable.is_horizontal());
            let SubtableKind::Format0(f0) = subtable.kind().unwrap() else { panic!("not format 0") };
            sizes.push(f0.pairs().len());
            for p in f0.pairs() {
                let name = |gid: u16| order[usize::from(gid)].clone();
                let key = (name(p.left().to_u16()), name(p.right().to_u16()));
                assert!(pairs.insert(key, i32::from(p.value())).is_none(), "a pair in two subtables");
                assert_eq!(f0.kerning(p.left().into(), p.right().into()), Some(i32::from(p.value())), "the binary search finds it");
            }
        }
        (pairs, sizes)
    }

    #[test]
    fn export_kern_table_expands_class_pairs_into_member_pairs() {
        let mut f = class_font();
        let out = compile_ttf(&f, &Options::default()).unwrap();
        assert!(FontRef::new(&out.bytes).unwrap().kern().is_err(), "no kern table without the parameter");

        turn_on(&mut f);
        let out = compile_ttf(&f, &Options::default()).unwrap();
        let (pairs, sizes) = read_back(&out.bytes, &out.glyph_order);
        let want: BTreeMap<(String, String), i32> =
            [("A", "V", -50), ("A", "W", -20), ("Aacute", "W", -50), ("T", "o", -60)].map(|(a, b, v)| ((a.into(), b.into()), v)).into();
        // `Aacute V` and `T @MMK_R_V` are 0: an exception at 0 overrides the class pair and is
        // left out like every pair of value 0.
        assert_eq!(pairs, want);
        assert_eq!(sizes, vec![4]);

        // CFF exports get no kern table, and say so.
        let out = compile(&f, &f.default_target().unwrap(), Flavour::Cff, &Options::default()).unwrap();
        assert!(FontRef::new(&out.bytes).unwrap().kern().is_err(), "no kern table in a CFF export");
        assert!(out.warnings.iter().any(|w| w.starts_with(PARAMETER) && w.contains("TrueType")), "{:?}", out.warnings);
    }

    #[test]
    fn export_kern_table_splits_subtables_and_drops_pairs_past_the_limit() {
        // 256 × 256 member pairs from one class pair: one more than the table holds.
        let mut f = Font::new("Kern Table Sans");
        for i in 0..256 {
            let (l, r) = (format!("l{i:03}"), format!("r{i:03}"));
            f.add_glyph(&l, vec![], 500.0).unwrap();
            f.add_glyph(&r, vec![], 500.0).unwrap();
            group(&mut f, &l, GroupSide::Right, "L");
            group(&mut f, &r, GroupSide::Left, "R");
        }
        let kerning = Arc::make_mut(&mut f.kerning);
        kerning.ltr.set("m01", "@MMK_L_L", "@MMK_R_R", -10.0);
        // The exception is expanded first, so it stays when the table is full.
        kerning.ltr.set("m01", "l255", "r255", -30.0);
        turn_on(&mut f);
        let out = compile_ttf(&f, &Options::default()).unwrap();
        let (pairs, sizes) = read_back(&out.bytes, &out.glyph_order);
        assert_eq!(pairs.len(), MAX_KERN_PAIRS);
        assert_eq!(sizes.first(), Some(&MAX_KERN_SUBTABLE_PAIRS));
        assert_eq!(sizes.len(), MAX_KERN_PAIRS.div_ceil(MAX_KERN_SUBTABLE_PAIRS));
        assert_eq!(pairs.get(&("l255".to_string(), "r255".to_string())), Some(&-30));
        assert_eq!(pairs.get(&("l000".to_string(), "r000".to_string())), Some(&-10));
        assert!(out.warnings.iter().any(|w| w.starts_with("the kern table holds at most")), "{:?}", out.warnings);
    }

    #[test]
    fn stored_pairs_that_expand_to_nothing_still_count_toward_the_work_cap() {
        // One left group of 2,000 members kerned against 2,100 right groups with no members:
        // no glyph pair results, but each stored pair lists the 2,000 members again.
        let mut f = Font::new("Kern Table Sans");
        for i in 0..2_000 {
            let l = format!("l{i:04}");
            f.add_glyph(&l, vec![], 500.0).unwrap();
            group(&mut f, &l, GroupSide::Right, "L");
        }
        let kerning = Arc::make_mut(&mut f.kerning);
        for i in 0..2_100 {
            kerning.ltr.set("m01", "@MMK_L_L", &format!("@MMK_R_empty{i}"), -10.0);
        }
        let order: Vec<&str> = f.glyphs.iter().map(|g| g.name.as_str()).collect();
        let mut warnings = Vec::new();
        let started = std::time::Instant::now();
        let out = pairs(&f, "m01", &order, &ExportEdits::default(), &mut warnings);
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "took {:?}", started.elapsed());
        assert!(out.is_empty());
        assert!(warnings.iter().any(|w| w.contains("expanding the kerning groups")), "{warnings:?}");
    }

    #[test]
    fn export_kern_table_pairs_follow_renamed_and_removed_glyphs() {
        let f = class_font();
        // `A` and `Aacute` swap names; `W` is removed.
        let mut edits = ExportEdits::default();
        edits.renamed.insert("A".into(), "Aacute".into());
        edits.renamed.insert("Aacute".into(), "A".into());
        edits.removed.insert("W".into());
        let order = ["A", "Aacute", "T", "V", "o"];
        let mut warnings = Vec::new();
        let got = pairs(&f, "m01", &order, &edits, &mut warnings);
        // The source `A V` (-50) belongs to the glyph exported as `Aacute`; the source
        // `Aacute V` (0) to `A`, so `A V` is left out.
        let want: BTreeMap<(u16, u16), i16> = [((1, 3), -50), ((2, 4), -60)].into();
        assert_eq!(got, want);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn search_fields_follow_the_largest_power_of_two() {
        assert_eq!(search_fields(1), (6, 0, 0));
        assert_eq!(search_fields(5), (24, 2, 6));
        assert_eq!(search_fields(MAX_KERN_SUBTABLE_PAIRS as u16), (49_152, 13, 16_368));
        // The largest subtable still fits its 16-bit length.
        assert!(MAX_KERN_SUBTABLE_PAIRS * PAIR_LEN + SUBTABLE_HEADER_LEN <= usize::from(u16::MAX));
    }
}

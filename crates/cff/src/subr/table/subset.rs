//! Subsetting a whole `CFF2` table, and the steps pyftsubset runs on `CFF ` and `CFF2` tables
//! after a subset (M12.19b; row `subset`). Ported from the `CFF ` table methods of fontTools'
//! `subset/cff.py` (MIT License, Copyright 2013 Google, Inc.): `subset_glyphs`, the emptied
//! charstrings of `_empty_charstring`, and `prune_post_subset`, which drops the Font DICTs no
//! glyph selects, then expands every subroutine (`--desubroutinize`) or removes the unused ones.
//!
//! The variation store is copied as it is, as the reference leaves it. Font DICTs are dropped only
//! when the table has an FDSelect; the FDSelect is then written again in format 0 or 3, whichever
//! is smaller (format 3 on a tie), as fontTools chooses.

use std::collections::BTreeSet;

use super::super::{CharstringSet, SubrError, desubroutinize_within, prune::prune_unused};
use super::{Entry, Table, invalid};

/// What happens to the subroutines of a subset table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubrMode {
    /// Kept as they are.
    Keep,
    /// The ones no kept glyph reaches are removed and the calls renumbered. Where that is not
    /// possible ([`prune_unused`] refuses, or its work passes a limit), they are kept as they
    /// are: keeping them is always correct.
    Prune,
    /// Every call is expanded; no subroutine is left.
    Expand,
}

/// A subset of one `CFF ` or `CFF2` table.
#[derive(Clone, Copy, Debug)]
pub struct TableSubset<'a> {
    /// Output glyph → input glyph. `None` keeps every glyph in its place; only a `CFF2` table,
    /// which has no charset, can be given a new glyph list.
    pub glyphs: Option<&'a [usize]>,
    /// Output glyphs whose charstring becomes empty: no program in `CFF2`, `endchar` in `CFF `.
    pub emptied: &'a [usize],
    pub subroutines: SubrMode,
    /// With [`SubrMode::Expand`], the most bytes the expanded charstrings may take together.
    pub max_expanded: usize,
}

/// The table with the glyphs of `subset`, without the Font DICTs no glyph selects, its
/// subroutines treated as `subset.subroutines` says.
pub fn subset_table(data: &[u8], cff2: bool, subset: &TableSubset<'_>) -> Result<Vec<u8>, SubrError> {
    subset_table_noted(data, cff2, subset, &mut Vec::new())
}

/// [`subset_table`], adding to `notes` a sentence for each thing the person should know: the
/// subroutines kept because pruning them passed a limit.
pub fn subset_table_noted(data: &[u8], cff2: bool, subset: &TableSubset<'_>, notes: &mut Vec<String>) -> Result<Vec<u8>, SubrError> {
    let table = Table::read(data, cff2)?;
    let n_in = table.set.charstrings.len();
    let new_to_old: Vec<usize> = match subset.glyphs {
        Some(list) if !cff2 => {
            if list.len() != n_in || list.iter().enumerate().any(|(new, old)| new != *old) {
                return Err(SubrError::Unsupported("choosing the glyphs of a CFF table here (its charset would change)".into()));
            }
            list.to_vec()
        }
        Some(list) => list.to_vec(),
        None => (0..n_in).collect(),
    };
    if new_to_old.is_empty() || new_to_old.len() > super::MAX_GLYPHS {
        return Err(invalid(format!("a subset of {} glyphs", new_to_old.len())));
    }
    if let Some(bad) = new_to_old.iter().find(|&&old| old >= n_in) {
        return Err(invalid(format!("the subset keeps glyph {bad}, but the table has {n_in}")));
    }
    let emptied: BTreeSet<usize> = subset.emptied.iter().copied().collect();
    let empty_program: Vec<u8> = if cff2 { Vec::new() } else { vec![14] };
    let charstrings: Vec<Vec<u8>> = new_to_old
        .iter()
        .enumerate()
        .map(|(new, &old)| if emptied.contains(&new) { empty_program.clone() } else { table.set.charstrings.get(old).cloned().unwrap_or_default() })
        .collect();

    // Font DICTs: with an FDSelect, keep the ones the output glyphs select, in their order.
    let fd_of: Vec<usize> = new_to_old.iter().map(|&old| table.set.fd_of(old)).collect();
    let kept_fds: Vec<usize> = if table.fd_select.is_some() {
        fd_of.iter().copied().collect::<BTreeSet<_>>().into_iter().collect()
    } else {
        (0..table.privates.len()).collect()
    };
    let new_fd = |fd: usize| kept_fds.binary_search(&fd).unwrap_or(0);
    let fd_select: Vec<u16> =
        if table.fd_select.is_some() { fd_of.iter().map(|&fd| u16::try_from(new_fd(fd)).unwrap_or(0)).collect() } else { Vec::new() };
    let fd_select_bytes = if table.fd_select.is_some() { Some(write_fd_select(&fd_select)?) } else { None };
    let pick = |items: &[Vec<Vec<u8>>]| -> Vec<Vec<Vec<u8>>> { kept_fds.iter().map(|&fd| items.get(fd).cloned().unwrap_or_default()).collect() };
    let pick_dicts = |items: &[Vec<Entry>]| -> Vec<Vec<Entry>> { kept_fds.iter().map(|&fd| items.get(fd).cloned().unwrap_or_default()).collect() };

    let set = CharstringSet {
        cff2,
        charstrings,
        global_subrs: table.set.global_subrs.clone(),
        local_subrs: pick(&table.set.local_subrs),
        fd_select,
        fd_vsindex: kept_fds.iter().map(|&fd| table.set.fd_vsindex.get(fd).copied().unwrap_or(0)).collect(),
        regions: table.set.regions.clone(),
        max_stack: table.set.max_stack,
    };
    let set = match subset.subroutines {
        SubrMode::Keep => set,
        SubrMode::Expand => desubroutinize_within(&set, subset.max_expanded)?,
        SubrMode::Prune => match prune_unused(&set) {
            Ok(pruned) => pruned,
            Err(SubrError::Unsupported(_)) => set,
            Err(e @ SubrError::TooLarge { .. }) => {
                notes.push(format!("the unused subroutines were kept: {e}"));
                set
            }
            Err(e) => return Err(e),
        },
    };

    let out = Table {
        cff2,
        header: table.header,
        names: table.names,
        strings: table.strings,
        top: table.top.clone(),
        charset: table.charset,
        encoding: table.encoding,
        fd_select: fd_select_bytes.as_deref(),
        vstore: table.vstore,
        font_dicts: table.font_dicts.as_deref().map(pick_dicts),
        privates: pick_dicts(&table.privates),
        set,
    };
    out.write(&out.set)
}

/// An FDSelect in format 0 or 3, whichever is smaller (format 3 on a tie), as fontTools packs it.
fn write_fd_select(fds: &[u16]) -> Result<Vec<u8>, SubrError> {
    let byte = |fd: u16| u8::try_from(fd).map_err(|_| SubrError::TooLarge { what: "Font DICTs in an FDSelect", limit: 256 });
    let mut zero = vec![0u8];
    for &fd in fds {
        zero.push(byte(fd)?);
    }
    let mut ranges: Vec<(usize, u16)> = Vec::new();
    for (g, &fd) in fds.iter().enumerate() {
        if ranges.last().is_none_or(|&(_, last)| last != fd) {
            ranges.push((g, fd));
        }
    }
    let count = u16::try_from(ranges.len()).map_err(|_| SubrError::TooLarge { what: "FDSelect ranges", limit: 65_535 })?;
    let sentinel = u16::try_from(fds.len()).map_err(|_| SubrError::TooLarge { what: "glyphs in an FDSelect", limit: 65_535 })?;
    let mut three = vec![3u8];
    three.extend_from_slice(&count.to_be_bytes());
    for (first, fd) in ranges {
        let first = u16::try_from(first).map_err(|_| SubrError::TooLarge { what: "glyphs in an FDSelect", limit: 65_535 })?;
        three.extend_from_slice(&first.to_be_bytes());
        three.push(byte(fd)?);
    }
    three.extend_from_slice(&sentinel.to_be_bytes());
    Ok(if zero.len() < three.len() { zero } else { three })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
    use read_fonts::ps::cff::CffFontRef;
    use read_fonts::ps::cs::CommandSink;
    use read_fonts::types::{F2Dot14, Fixed, GlyphId};

    use super::*;
    use crate::subr::push_int;

    #[derive(Default)]
    struct Pen(Vec<(char, Vec<i32>)>);

    impl CommandSink for Pen {
        fn move_to(&mut self, x: Fixed, y: Fixed) {
            self.0.push(('M', vec![x.to_bits(), y.to_bits()]));
        }
        fn line_to(&mut self, x: Fixed, y: Fixed) {
            self.0.push(('L', vec![x.to_bits(), y.to_bits()]));
        }
        fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, e: Fixed, f: Fixed) {
            self.0.push(('C', [a, b, c, d, e, f].iter().map(|v| v.to_bits()).collect()));
        }
        fn close(&mut self) {
            self.0.push(('Z', Vec::new()));
        }
    }

    /// Glyph `gid` of a `CFF2` table drawn at the weight `at` (normalized).
    fn draw(table: &[u8], gid: u32, at: f32) -> Vec<(char, Vec<i32>)> {
        let cff = CffFontRef::new_cff2(table, None).unwrap();
        let coords = [F2Dot14::from_f32(at)];
        let gid = GlyphId::new(gid);
        let subfont = cff.subfont(cff.subfont_index(gid).unwrap_or(0), &coords).unwrap();
        let mut pen = Pen::default();
        cff.evaluate_charstring(&subfont, gid, &coords, &mut pen).unwrap();
        pen.0
    }

    fn cs(program: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for word in program.split_whitespace() {
            if let Some(hex) = word.strip_prefix("mask:") {
                out.push(u8::from_str_radix(hex, 16).unwrap());
                continue;
            }
            if let Ok(v) = word.parse::<i64>() {
                push_int(v, &mut out).unwrap();
                continue;
            }
            out.push(match word {
                "rlineto" => 5,
                "callsubr" => 10,
                "blend" => 16,
                "hstemhm" => 18,
                "hintmask" => 19,
                "rmoveto" => 21,
                "callgsubr" => 29,
                _ => panic!("unknown operator {word}"),
            });
        }
        out
    }

    fn index(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        super::super::write_index(&mut out, items, true).unwrap();
        out
    }

    fn int29(v: usize) -> Vec<u8> {
        [vec![29], (v as i32).to_be_bytes().to_vec()].concat()
    }

    const GLOBALS: [&str; 2] = ["10 0 rlineto", "0 20 5 1 blend rlineto"];
    const LOCALS: [&[&str]; 2] = [&["100 100 rmoveto", "50 0 rlineto"], &["200 0 rmoveto", "7 7 rlineto", "0 -40 rlineto"]];
    const GLYPH2: &str = "10 20 30 40 hstemhm hintmask mask:C0";
    const GLYPHS: [&str; 4] = [
        "0 0 rmoveto 500 0 rlineto",
        "0 0 rmoveto -106 callsubr -106 callgsubr",
        "10 20 30 40 hstemhm hintmask mask:C0 -107 callsubr -105 callsubr -106 callgsubr",
        "0 0 rmoveto -105 callsubr",
    ];

    /// A `CFF2` table with one axis and one region, two Font DICTs selected 1, 0, 1, 1 with their
    /// own local subroutines, two global subroutines (one blends) and four glyphs. The same table
    /// is built by the fixture the pyftsubset expectations below come from.
    fn fixture() -> Vec<u8> {
        let gsubrs = index(&GLOBALS.map(cs));
        let region_list = [1u16.to_be_bytes(), 1u16.to_be_bytes(), 0i16.to_be_bytes(), 0x4000i16.to_be_bytes(), 0x4000i16.to_be_bytes()].concat();
        let data = [0u16.to_be_bytes(), 0u16.to_be_bytes(), 1u16.to_be_bytes(), 0u16.to_be_bytes()].concat();
        let (data_at, region_at) = (12u32, 20u32);
        let store = [
            1u16.to_be_bytes().to_vec(),
            region_at.to_be_bytes().to_vec(),
            1u16.to_be_bytes().to_vec(),
            data_at.to_be_bytes().to_vec(),
            data,
            region_list,
        ]
        .concat();
        let vstore = [(store.len() as u16).to_be_bytes().to_vec(), store].concat();
        let privates: Vec<Vec<u8>> =
            LOCALS.iter().map(|subrs| [int29(6), vec![19], index(&subrs.iter().map(|p| cs(p)).collect::<Vec<_>>())].concat()).collect();
        let fd_select = vec![0, 1, 0, 1, 1];
        let charstrings = index(&GLYPHS.map(cs));
        let top_len = 26;
        let mut pos = 5 + top_len + gsubrs.len();
        let vstore_at = pos;
        pos += vstore.len();
        let fds_at = pos;
        pos += fd_select.len();
        let cs_at = pos;
        pos += charstrings.len();
        let fdarray_at = pos;
        pos += index(&[vec![0; 11], vec![0; 11]]).len();
        let mut private_at = Vec::new();
        for p in &privates {
            private_at.push(pos);
            pos += p.len();
        }
        let fdarray = index(&private_at.iter().map(|&at| [int29(6), int29(at), vec![18]].concat()).collect::<Vec<_>>());
        let top = [int29(fdarray_at), vec![12, 36], int29(cs_at), vec![17], int29(fds_at), vec![12, 37], int29(vstore_at), vec![24]].concat();
        assert_eq!(top.len(), top_len);
        let out = [vec![2, 0, 5], (top_len as u16).to_be_bytes().to_vec(), top, gsubrs, vstore, fd_select, charstrings, fdarray, privates.concat()]
            .concat();
        assert_eq!(out.len(), pos);
        out
    }

    fn parts(table: &[u8]) -> CharstringSet {
        Table::read(table, true).unwrap().set
    }

    #[test]
    fn a_cff2_subset_matches_the_reference_subsetter() {
        // pyftsubset --gids=0,2: Font DICT 0 goes, glyph 2 keeps local subroutines 0 and 2 of
        // Font DICT 1 and global subroutine 1, renumbered; .notdef becomes empty.
        let input = fixture();
        let subset = TableSubset { glyphs: Some(&[0, 2]), emptied: &[0], subroutines: SubrMode::Prune, max_expanded: usize::MAX };
        let out = subset_table(&input, true, &subset).unwrap();
        let set = parts(&out);
        assert_eq!(set.charstrings, vec![Vec::new(), cs(&format!("{GLYPH2} -107 callsubr -106 callsubr -107 callgsubr"))]);
        assert_eq!(set.global_subrs, vec![cs(GLOBALS[1])]);
        assert_eq!(set.local_subrs, vec![vec![cs(LOCALS[1][0]), cs(LOCALS[1][2])]]);
        assert_eq!(set.fd_select, vec![0, 0]);
        assert_eq!(set.regions, vec![1], "the variation store stays");
        for at in [0.0, 0.5, 1.0] {
            assert_eq!(draw(&out, 1, at), draw(&input, 2, at), "at {at}");
        }

        // With --desubroutinize nothing is left to call.
        let expand = TableSubset { subroutines: SubrMode::Expand, ..subset };
        let set = parts(&subset_table(&input, true, &expand).unwrap());
        assert_eq!(set.charstrings[1], cs(&format!("{GLYPH2} 200 0 rmoveto 0 -40 rlineto 0 20 5 1 blend rlineto")));
        assert!(set.global_subrs.is_empty() && set.local_subrs.iter().all(Vec::is_empty));
        // The expanded charstrings are held to the limit the caller gives.
        let small = TableSubset { max_expanded: 8, ..expand };
        assert!(matches!(subset_table(&input, true, &small), Err(SubrError::TooLarge { .. })));

        // --retain-gids --notdef-outline: emptied glyph 1 keeps Font DICT 0, which loses its
        // subroutines; .notdef keeps its outline.
        let retain = TableSubset { glyphs: Some(&[0, 1, 2]), emptied: &[1], subroutines: SubrMode::Prune, max_expanded: usize::MAX };
        let set = parts(&subset_table(&input, true, &retain).unwrap());
        assert_eq!(set.fd_select, vec![1, 0, 1]);
        assert_eq!(set.charstrings[..2], [cs(GLYPHS[0]), Vec::new()]);
        assert_eq!(set.local_subrs, vec![Vec::new(), vec![cs(LOCALS[1][0]), cs(LOCALS[1][2])]]);
    }

    #[test]
    fn hostile_subsets_fail_with_a_message() {
        let input = fixture();
        for glyphs in [&[][..], &[0, 9][..]] {
            let subset = TableSubset { glyphs: Some(glyphs), emptied: &[], subroutines: SubrMode::Prune, max_expanded: usize::MAX };
            assert!(matches!(subset_table(&input, true, &subset), Err(SubrError::Invalid(_))), "{glyphs:?}");
        }
        assert!(
            subset_table(&input[..40], true, &TableSubset { glyphs: None, emptied: &[], subroutines: SubrMode::Keep, max_expanded: usize::MAX })
                .is_err()
        );
    }

    #[test]
    fn fd_select_takes_the_smaller_format() {
        // Two glyphs in two Font DICTs: format 0 (3 bytes) beats format 3 (11 bytes).
        assert_eq!(write_fd_select(&[0, 1]).unwrap(), vec![0, 0, 1]);
        // Twenty glyphs in one Font DICT: format 3 (8 bytes) beats format 0 (21 bytes).
        assert_eq!(write_fd_select(&[0; 20]).unwrap(), vec![3, 0, 1, 0, 0, 0, 0, 20]);
    }
}

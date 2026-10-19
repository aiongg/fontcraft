//! The charset: the glyph name of every glyph after `.notdef`.
//!
//! Ported from fontTools `cffLib` (`getStdCharSet`, `CharsetCompiler`, `packCharset0`,
//! `packCharset`; MIT, Copyright (c) 2017 Just van Rossum and contributors). A glyph order that is
//! a prefix of a predefined charset uses its id. Otherwise the smaller of format 0 (one SID per
//! glyph) and format 1 or 2 (ranges of consecutive SIDs; format 2 once a range is longer than 256)
//! is written, format 0 on a tie. Glyph names get their SIDs here, before any Top DICT string.

use crate::Error;
use crate::strings::{EXPERT_CHARSET, EXPERT_SUBSET_CHARSET, ISO_ADOBE_COUNT, STANDARD_STRINGS, Strings};

/// What the Top DICT's `charset` entry points at.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Charset {
    /// A predefined charset: 0 ISOAdobe, 1 Expert, 2 Expert Subset.
    Predefined(u8),
    /// Charset data written into the table.
    Custom(Vec<u8>),
}

/// The charset for glyph `names` (the first is `.notdef`).
pub(crate) fn build(names: &[&[u8]], strings: &mut Strings) -> Result<Charset, Error> {
    if let Some(id) = predefined(names) {
        return Ok(Charset::Predefined(id));
    }
    let mut sids = Vec::with_capacity(names.len());
    for name in names.iter().skip(1) {
        sids.push(strings.sid(name)?);
    }
    let format0 = format0(&sids);
    let ranges = ranges(&sids);
    Ok(Charset::Custom(if ranges.len() < format0.len() { ranges } else { format0 }))
}

/// The predefined charset `names` is a prefix of, checked in id order.
fn predefined(names: &[&[u8]]) -> Option<u8> {
    let iso = |i: usize| STANDARD_STRINGS.get(i).filter(|_| i < ISO_ADOBE_COUNT).map(|s| s.as_bytes());
    let expert = |i: usize| EXPERT_CHARSET.get(i).and_then(|&sid| STANDARD_STRINGS.get(usize::from(sid))).map(|s| s.as_bytes());
    let subset = |i: usize| EXPERT_SUBSET_CHARSET.get(i).and_then(|&sid| STANDARD_STRINGS.get(usize::from(sid))).map(|s| s.as_bytes());
    let matches = |count: usize, name_at: &dyn Fn(usize) -> Option<&'static [u8]>| {
        names.len() <= count && names.iter().enumerate().all(|(i, n)| name_at(i) == Some(*n))
    };
    if matches(ISO_ADOBE_COUNT, &iso) {
        Some(0)
    } else if matches(EXPERT_CHARSET.len(), &expert) {
        Some(1)
    } else if matches(EXPERT_SUBSET_CHARSET.len(), &subset) {
        Some(2)
    } else {
        None
    }
}

pub(crate) fn format0(sids: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 2 * sids.len());
    out.push(0);
    for sid in sids {
        out.extend_from_slice(&sid.to_be_bytes());
    }
    out
}

/// Format 1, or 2 when some range has more than 255 glyphs after its first.
pub(crate) fn ranges(sids: &[u16]) -> Vec<u8> {
    let mut runs: Vec<(u16, u32)> = Vec::new();
    let mut first: Option<u16> = None;
    let mut end: u16 = 0;
    for &sid in sids {
        match first {
            None => first = Some(sid),
            Some(f) if u32::from(end) + 1 != u32::from(sid) => {
                runs.push((f, u32::from(end).saturating_sub(u32::from(f))));
                first = Some(sid);
            }
            Some(_) => {}
        }
        end = sid;
    }
    // The reference closes the last range only when its end SID is not zero; a charset of
    // `.notdef` alone therefore has no ranges.
    if let Some(f) = first.filter(|_| end != 0) {
        runs.push((f, u32::from(end).saturating_sub(u32::from(f))));
    }
    let format2 = runs.iter().any(|&(_, left)| left > 255);
    let mut out = vec![if format2 { 2 } else { 1 }];
    for (first, left) in runs {
        out.extend_from_slice(&first.to_be_bytes());
        if format2 {
            out.extend_from_slice(&u16::try_from(left).unwrap_or(u16::MAX).to_be_bytes());
        } else {
            out.push(u8::try_from(left).unwrap_or(u8::MAX));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn charset(names: &[&str]) -> (Charset, Strings) {
        let mut strings = Strings::new();
        let names: Vec<&[u8]> = names.iter().map(|n| n.as_bytes()).collect();
        (build(&names, &mut strings).unwrap(), strings)
    }

    #[test]
    fn a_prefix_of_iso_adobe_uses_the_predefined_id() {
        assert_eq!(charset(&[".notdef", "space", "exclam"]).0, Charset::Predefined(0));
        assert_eq!(charset(&[".notdef"]).0, Charset::Predefined(0));
        // Out of order: not a prefix.
        assert!(matches!(charset(&[".notdef", "exclam", "space"]).0, Charset::Custom(_)));
    }

    #[test]
    fn expert_prefixes_use_their_ids() {
        // The Expert charset starts .notdef, space, exclamsmall; the subset .notdef, space, dollaroldstyle.
        assert_eq!(charset(&[".notdef", "space", "exclamsmall"]).0, Charset::Predefined(1));
        assert_eq!(charset(&[".notdef", "space", "dollaroldstyle"]).0, Charset::Predefined(2));
    }

    #[test]
    fn consecutive_sids_pack_as_ranges() {
        // A, B, C are SIDs 34, 35, 36: one range beats three format-0 entries.
        let (c, _) = charset(&[".notdef", "A", "B", "C"]);
        assert_eq!(c, Charset::Custom(vec![1, 0, 34, 2]));
    }

    #[test]
    fn scattered_sids_stay_in_format_0() {
        // A (34) and C (36): format 0 is 5 bytes, format 1 is 7.
        let (c, _) = charset(&[".notdef", "A", "C"]);
        assert_eq!(c, Charset::Custom(vec![0, 0, 34, 0, 36]));
    }

    #[test]
    fn a_tie_goes_to_format_0() {
        // One glyph: format 0 is 3 bytes, format 1 is 4.
        let (c, _) = charset(&[".notdef", "A"]);
        assert_eq!(c, Charset::Custom(vec![0, 0, 34]));
    }

    #[test]
    fn long_ranges_switch_to_format_2() {
        let names: Vec<String> = (0..300).map(|i| format!("g{i:03}")).collect();
        let mut all = vec![".notdef"];
        all.extend(names.iter().map(String::as_str));
        let (c, strings) = charset(&all);
        // New names get SIDs from 391 in order: one range of 300.
        assert_eq!(c, Charset::Custom(vec![2, 0x01, 0x87, 0x01, 0x2B]));
        assert_eq!(strings.custom().len(), 300);
    }
}

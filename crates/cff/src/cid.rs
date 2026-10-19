//! CID-keyed `CFF ` tables (M12.41; row `compile.cidKeyedFonts`).
//!
//! [`build`] writes a CID-keyed font: glyphs are addressed by character identifier (CID), not by
//! name, and each glyph belongs to one Font DICT (a sub-font) with its own Private DICT, so each
//! sub-font has its own hinting values and its own `defaultWidthX` and `nominalWidthX`.
//!
//! | Part | What is written |
//! |---|---|
//! | Name INDEX | the CID font name |
//! | Top DICT | `ROS` first, then the name-keyed entries as [`crate::build`] writes them, `CIDCount`, `charset`, `FDSelect`, `FDArray`, `CharStrings` |
//! | charset | the CID of every glyph after glyph 0, format 0 or ranges, whichever is smaller |
//! | FDSelect | format 0 (one byte per glyph) or format 3 (ranges), whichever is smaller; format 0 on a tie |
//! | FDArray | one Font DICT per sub-font: `FontName` and `Private` |
//! | Private DICTs | one per sub-font, after the FDArray, in sub-font order |
//!
//! The layout follows the CFF specification (Adobe Technical Note #5176, § 18 and § 19): header,
//! Name INDEX, Top DICT INDEX, String INDEX, Global Subr INDEX, charset, FDSelect, CharStrings,
//! FDArray, Private DICTs. No subroutines: [`crate::subr::subroutinize_table`] adds global and
//! per-sub-font local ones to the built table.
//!
//! Glyphs are given in glyph order with strictly increasing CIDs; glyph 0 must be CID 0, which
//! the specification reserves for `.notdef`. Every input is untrusted and checked as
//! [`crate::build`] checks it; nothing panics.

use std::collections::HashSet;

use crate::charstring;
use crate::dict;
use crate::index;
use crate::number::{encode_dict_int, ot_round};
use crate::strings::Strings;
use crate::width;
use crate::{Bounds, Error, Glyph, PrivateDict, TopDict, Widths};

/// Highest CID: `CIDCount` is at most 65,535, counting CID 0.
pub const MAX_CID: u32 = 65_534;
/// Most Font DICTs: an FDSelect stores a Font DICT's index in one byte.
pub const MAX_FONT_DICTS: usize = 256;
/// Largest supplement number.
pub const MAX_SUPPLEMENT: i64 = 65_535;
/// Layout passes before giving up.
const MAX_LAYOUT_PASSES: usize = 64;

/// The character collection: registry, ordering and supplement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ros {
    /// ASCII only.
    pub registry: String,
    /// ASCII only.
    pub ordering: String,
    pub supplement: i64,
}

/// One sub-font.
#[derive(Clone, Debug, PartialEq)]
pub struct FontDict {
    /// The sub-font's `FontName`. ASCII only.
    pub name: String,
    pub private: PrivateDict,
    pub widths: Widths,
}

/// A glyph, its CID and its sub-font. The glyph's name is only used in messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CidGlyph {
    pub cid: u32,
    /// Index into [`CidFont::font_dicts`].
    pub fd: usize,
    pub glyph: Glyph,
}

/// A static, CID-keyed CFF font.
#[derive(Clone, Debug, PartialEq)]
pub struct CidFont {
    /// The CID font name (the Name INDEX entry). Latin-1.
    pub name: String,
    pub top: TopDict,
    pub ros: Ros,
    /// `CIDCount`; `None`: the highest CID plus one.
    pub cid_count: Option<u32>,
    /// At least one, at most [`MAX_FONT_DICTS`].
    pub font_dicts: Vec<FontDict>,
    /// In glyph order, CIDs strictly increasing, the first CID 0.
    pub glyphs: Vec<CidGlyph>,
    /// As [`crate::Font::round_tolerance`].
    pub round_tolerance: f64,
    /// As [`crate::Font::specialise`].
    pub specialise: bool,
}

/// A built CID-keyed table.
#[derive(Clone, Debug, PartialEq)]
pub struct CidBuilt {
    pub table: Vec<u8>,
    /// The Top DICT FontBBox (`[0, 0, 0, 0]` when no glyph draws anything).
    pub font_bbox: [i64; 4],
    /// `(defaultWidthX, nominalWidthX)` of each sub-font.
    pub widths: Vec<(i64, i64)>,
    /// Each glyph's drawn bounds, in glyph order, as [`crate::Built::glyph_bounds`].
    pub glyph_bounds: Vec<Option<Bounds>>,
}

/// Where the parts the Top DICT and the Font DICTs point at landed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct At {
    charset: usize,
    fd_select: usize,
    charstrings: usize,
    fd_array: usize,
}

/// Build the CID-keyed `CFF ` table for `font`.
pub fn build(font: &CidFont) -> Result<CidBuilt, Error> {
    validate(font)?;

    let mut widths = Vec::with_capacity(font.font_dicts.len());
    for (i, fd) in font.font_dicts.iter().enumerate() {
        widths.push(match fd.widths {
            Widths::Fixed { default_width_x, nominal_width_x } => (default_width_x, nominal_width_x),
            Widths::Optimise => {
                let own: Vec<i64> = font.glyphs.iter().filter(|g| g.fd == i).map(|g| ot_round(g.glyph.advance) as i64).collect();
                width::optimize(&own).unwrap_or((0, 0))
            }
        });
    }

    let mut charstrings = Vec::with_capacity(font.glyphs.len());
    let mut glyph_bounds = Vec::with_capacity(font.glyphs.len());
    let mut font_box: Option<Bounds> = None;
    for g in &font.glyphs {
        let (default_width_x, nominal_width_x) = widths.get(g.fd).copied().unwrap_or((0, 0));
        let advance = g.glyph.advance;
        let width = if advance == default_width_x as f64 { None } else { Some(ot_round(advance - nominal_width_x as f64)) };
        let built = charstring::build(&g.glyph.name, &g.glyph.path, g.glyph.hints.as_ref(), width, font.round_tolerance, font.specialise, false)?;
        if let Some(b) = built.bounds {
            font_box = Some(font_box.map_or(b, |f| f.union(b)));
        }
        glyph_bounds.push(built.bounds);
        charstrings.push(built.bytes);
    }
    let font_bbox = font_box.map_or([0; 4], Bounds::round_out);

    // String ids: the ROS strings first, then the Top DICT strings, then the sub-font names.
    let mut strings = Strings::new();
    let registry = strings.sid(font.ros.registry.as_bytes())?;
    let ordering = strings.sid(font.ros.ordering.as_bytes())?;
    let mut sid = |s: &Option<String>| s.as_ref().map(|s| strings.sid(&super::latin1(s))).transpose();
    let sids = dict::TopSids {
        version: sid(&font.top.version)?,
        notice: sid(&font.top.notice)?,
        copyright: sid(&font.top.copyright)?,
        full_name: sid(&font.top.full_name)?,
        family_name: sid(&font.top.family_name)?,
        weight: sid(&font.top.weight)?,
    };
    let mut fd_names = Vec::with_capacity(font.font_dicts.len());
    for fd in &font.font_dicts {
        fd_names.push(strings.sid(fd.name.as_bytes())?);
    }

    let privates =
        font.font_dicts.iter().zip(&widths).map(|(fd, &(d, n))| dict::private_dict(&fd.private, d, n)).collect::<Result<Vec<_>, Error>>()?;
    let name_index = {
        let mut out = Vec::new();
        index::write_index(&[super::latin1(&font.name)], &mut out)?;
        out
    };
    let string_index = {
        let mut out = Vec::new();
        index::write_index(strings.custom(), &mut out)?;
        out
    };
    let global_subrs: [u8; 2] = [0, 0];
    let cids: Vec<u16> = font.glyphs.iter().skip(1).map(|g| u16::try_from(g.cid).unwrap_or(u16::MAX)).collect();
    let charset = {
        let format0 = crate::charset::format0(&cids);
        let ranges = crate::charset::ranges(&cids);
        if ranges.len() < format0.len() { ranges } else { format0 }
    };
    let fd_select = fd_select(font)?;
    let charstrings_len = index::index_len(charstrings.iter().map(Vec::len));
    let cid_count = i64::from(font.cid_count.unwrap_or_else(|| font.glyphs.last().map_or(1, |g| g.cid + 1)));
    let head = Head { ros: (registry, ordering, font.ros.supplement), cid_count, sids, font_bbox };

    // Repeat the layout until no position moves: offsets are written in their shortest form, so
    // the Top DICT and the Font DICTs change size with them.
    let mut at = At::default();
    let mut private_at: Vec<usize> = vec![0; privates.len()];
    let mut settled: Option<(Vec<u8>, Vec<Vec<u8>>, usize)> = None;
    let mut last: Option<(At, Vec<usize>)> = None;
    for _ in 0..MAX_LAYOUT_PASSES {
        let top = top_dict(font, &head, &at)?;
        let font_dicts = font_dicts(&fd_names, &privates, &private_at)?;
        let fd_array_len = index::index_len(font_dicts.iter().map(Vec::len));
        let top_index_len = index::index_len(std::iter::once(top.len()));
        let mut next = At { charset: 4 + name_index.len() + top_index_len + string_index.len() + global_subrs.len(), ..At::default() };
        next.fd_select = next.charset + charset.len();
        next.charstrings = next.fd_select + fd_select.len();
        next.fd_array = next.charstrings + charstrings_len;
        let mut pos = next.fd_array + fd_array_len;
        let mut next_private = Vec::with_capacity(privates.len());
        for p in &privates {
            next_private.push(pos);
            pos += p.len();
        }
        if pos > crate::MAX_TABLE_SIZE {
            return Err(Error::TableTooLarge(pos));
        }
        if last.as_ref() == Some(&(next, next_private.clone())) {
            settled = Some((top, font_dicts, pos));
            break;
        }
        last = Some((next, next_private.clone()));
        at = next;
        private_at = next_private;
    }
    let (top, font_dicts, end) = settled.ok_or(Error::LayoutDidNotSettle)?;

    let mut table = Vec::with_capacity(end);
    table.extend_from_slice(&[1, 0, 4, index::off_size(end)]);
    table.extend_from_slice(&name_index);
    index::write_index(&[top], &mut table)?;
    table.extend_from_slice(&string_index);
    table.extend_from_slice(&global_subrs);
    table.extend_from_slice(&charset);
    table.extend_from_slice(&fd_select);
    index::write_index(&charstrings, &mut table)?;
    index::write_index(&font_dicts, &mut table)?;
    for p in &privates {
        table.extend_from_slice(p);
    }
    if table.len() != end {
        return Err(Error::LayoutDidNotSettle);
    }
    Ok(CidBuilt { table, font_bbox, widths, glyph_bounds })
}

/// What the Top DICT holds besides the offsets.
struct Head {
    ros: (u16, u16, i64),
    cid_count: i64,
    sids: dict::TopSids,
    font_bbox: [i64; 4],
}

fn op(out: &mut Vec<u8>, op: &[u8]) {
    out.extend_from_slice(op);
}

/// The Top DICT: `ROS` must come first; the name-keyed entries follow in their usual order, then
/// the CID entries and the offsets in the order fontTools writes them.
fn top_dict(font: &CidFont, head: &Head, at: &At) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let (registry, ordering, supplement) = head.ros;
    encode_dict_int(i64::from(registry), "ROS", &mut out)?;
    encode_dict_int(i64::from(ordering), "ROS", &mut out)?;
    encode_dict_int(supplement, "ROS", &mut out)?;
    op(&mut out, &[12, 30]);
    out.extend(dict::top_dict(&font.top, &head.sids, head.font_bbox, &dict::Pointers::default())?);
    encode_dict_int(head.cid_count, "CIDCount", &mut out)?;
    op(&mut out, &[12, 34]);
    encode_dict_int(at.charset as i64, "charset", &mut out)?;
    op(&mut out, &[15]);
    encode_dict_int(at.fd_select as i64, "FDSelect", &mut out)?;
    op(&mut out, &[12, 37]);
    encode_dict_int(at.fd_array as i64, "FDArray", &mut out)?;
    op(&mut out, &[12, 36]);
    encode_dict_int(at.charstrings as i64, "CharStrings", &mut out)?;
    op(&mut out, &[17]);
    Ok(out)
}

/// One Font DICT per sub-font: `FontName`, then `Private` (size, offset).
fn font_dicts(names: &[u16], privates: &[Vec<u8>], at: &[usize]) -> Result<Vec<Vec<u8>>, Error> {
    let mut out = Vec::with_capacity(names.len());
    for ((name, private), offset) in names.iter().zip(privates).zip(at) {
        let mut d = Vec::new();
        encode_dict_int(i64::from(*name), "FontName", &mut d)?;
        op(&mut d, &[12, 38]);
        encode_dict_int(private.len() as i64, "Private", &mut d)?;
        encode_dict_int(*offset as i64, "Private", &mut d)?;
        op(&mut d, &[18]);
        out.push(d);
    }
    Ok(out)
}

/// The FDSelect: format 0 or format 3, whichever is smaller, format 0 on a tie.
fn fd_select(font: &CidFont) -> Result<Vec<u8>, Error> {
    let fds: Vec<u8> = font.glyphs.iter().map(|g| u8::try_from(g.fd).unwrap_or(u8::MAX)).collect();
    let mut format0 = Vec::with_capacity(1 + fds.len());
    format0.push(0);
    format0.extend_from_slice(&fds);
    let mut ranges: Vec<(u16, u8)> = Vec::new();
    for (gid, &fd) in fds.iter().enumerate() {
        if ranges.last().is_none_or(|&(_, last)| last != fd) {
            let first = u16::try_from(gid).map_err(|_| Error::TooManyGlyphs(fds.len()))?;
            ranges.push((first, fd));
        }
    }
    let count = u16::try_from(ranges.len()).map_err(|_| Error::TooManyGlyphs(fds.len()))?;
    let sentinel = u16::try_from(fds.len()).map_err(|_| Error::TooManyGlyphs(fds.len()))?;
    let mut format3 = Vec::with_capacity(5 + 3 * ranges.len());
    format3.push(3);
    format3.extend_from_slice(&count.to_be_bytes());
    for (first, fd) in ranges {
        format3.extend_from_slice(&first.to_be_bytes());
        format3.push(fd);
    }
    format3.extend_from_slice(&sentinel.to_be_bytes());
    Ok(if format3.len() < format0.len() { format3 } else { format0 })
}

fn cid_error(message: String) -> Error {
    Error::Cid(message)
}

fn validate(font: &CidFont) -> Result<(), Error> {
    let first = font.glyphs.first().ok_or(Error::NoGlyphs)?;
    if font.glyphs.len() > crate::MAX_GLYPHS {
        return Err(Error::TooManyGlyphs(font.glyphs.len()));
    }
    if first.cid != 0 {
        return Err(cid_error(format!("the first glyph, `{}`, must be CID 0, not CID {}", first.glyph.name, first.cid)));
    }
    if font.font_dicts.is_empty() || font.font_dicts.len() > MAX_FONT_DICTS {
        return Err(cid_error(format!("a CID-keyed font needs 1 to {MAX_FONT_DICTS} sub-fonts, not {}", font.font_dicts.len())));
    }
    let mut last: Option<u32> = None;
    for g in &font.glyphs {
        if g.cid > MAX_CID {
            return Err(cid_error(format!("`{}` has CID {}; the highest CID is {MAX_CID}", g.glyph.name, g.cid)));
        }
        if last.is_some_and(|l| g.cid <= l) {
            return Err(cid_error(format!("`{}` has CID {}, which does not follow the CID before it", g.glyph.name, g.cid)));
        }
        last = Some(g.cid);
        if g.fd >= font.font_dicts.len() {
            return Err(cid_error(format!("`{}` names sub-font {}, which does not exist", g.glyph.name, g.fd)));
        }
    }
    let highest = last.unwrap_or(0);
    if let Some(count) = font.cid_count
        && (count <= highest || count > MAX_CID + 1)
    {
        return Err(cid_error(format!("a CIDCount of {count} does not cover CID {highest}, or is more than {}", MAX_CID + 1)));
    }
    let ros = &font.ros;
    for (part, what) in [(&ros.registry, "the registry"), (&ros.ordering, "the ordering")] {
        super::check_string(part, what, true, false)?;
    }
    if !(0..=MAX_SUPPLEMENT).contains(&ros.supplement) {
        return Err(Error::OutOfRange { what: "the supplement".into(), value: ros.supplement as f64 });
    }
    let mut seen: HashSet<&str> = HashSet::with_capacity(font.font_dicts.len());
    for fd in &font.font_dicts {
        super::check_string(&fd.name, "a sub-font name", true, false)?;
        if !seen.insert(fd.name.as_str()) {
            return Err(cid_error(format!("two sub-fonts are named `{}`", fd.name)));
        }
        if let Widths::Fixed { default_width_x, nominal_width_x } = fd.widths {
            for (v, what) in [(default_width_x, "defaultWidthX"), (nominal_width_x, "nominalWidthX")] {
                if (v as f64).abs() > crate::MAX_ADVANCE {
                    return Err(Error::OutOfRange { what: format!("{what} of `{}`", fd.name), value: v as f64 });
                }
            }
        }
        check_private(&fd.private)?;
    }
    // The glyphs and the Top DICT as a name-keyed font checks them; names need not be unique or
    // valid charset entries, so they are checked as messages only.
    let mut proxy = crate::Font {
        name: font.name.clone(),
        top: font.top.clone(),
        glyphs: Vec::new(),
        round_tolerance: font.round_tolerance,
        specialise: font.specialise,
        ..crate::Font::default()
    };
    proxy.glyphs.push(Glyph { name: ".notdef".into(), ..Glyph::default() });
    super::validate(&proxy)?;
    for g in &font.glyphs {
        let mut one = proxy.clone();
        one.glyphs = vec![Glyph { name: ".notdef".into(), ..g.glyph.clone() }];
        super::validate(&one).map_err(|e| rename(e, &g.glyph.name))?;
    }
    Ok(())
}

/// An error about the stand-in name `.notdef`, told about the glyph's own name.
fn rename(e: Error, name: &str) -> Error {
    let fix = |s: String| s.replace("`.notdef`", &format!("`{name}`"));
    match e {
        Error::NotFinite { what } => Error::NotFinite { what: fix(what) },
        Error::OutOfRange { what, value } => Error::OutOfRange { what: fix(what), value },
        Error::TooManySegments { count, .. } => Error::TooManySegments { glyph: name.to_string(), count },
        Error::BadHints { message, .. } => Error::BadHints { glyph: name.to_string(), message },
        other => other,
    }
}

fn check_private(p: &PrivateDict) -> Result<(), Error> {
    let numbers = [p.blue_scale, p.blue_shift, p.blue_fuzz, p.force_bold, p.language_group]
        .into_iter()
        .chain(p.std_hw)
        .chain(p.std_vw)
        .chain(p.blue_values.iter().copied())
        .chain(p.other_blues.iter().copied())
        .chain(p.family_blues.iter().copied())
        .chain(p.family_other_blues.iter().copied())
        .chain(p.stem_snap_h.iter().copied())
        .chain(p.stem_snap_v.iter().copied());
    for n in numbers {
        super::check_number(n, "DICT value")?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "cid_tests.rs"]
mod tests;

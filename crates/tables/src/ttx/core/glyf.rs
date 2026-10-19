//! `glyf`: glyphs sorted by name, simple glyphs as contours of points, composites as components
//! with their transforms, and the instructions through [`super::program`].

use std::collections::{HashMap, HashSet};

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

use super::super::{XmlWriter, fixed_to_str, py_hex};
use super::{Rd, loca_offsets, program};

const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME: u8 = 0x10;
const Y_SAME: u8 = 0x20;
const OVERLAP_SIMPLE: u8 = 0x40;
const CUBIC: u8 = 0x80;

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;
/// The component flags fontTools keeps: round to grid, use my metrics, scaled and unscaled
/// offsets, non-overlapping, overlap compound.
const KEPT_FLAGS: u16 = 0x0004 | 0x0200 | 0x0800 | 0x1000 | 0x0010 | 0x0400;

fn bad(what: &str) -> TablesError {
    TablesError::invalid(format!("a glyph could not be read: {what}"))
}

pub(super) fn write(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let loca = font.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
    let offsets = loca_offsets(loca, font)?;
    let order = w.glyph_order(font)?;
    let mut pos = *offsets.first().ok_or_else(|| TablesError::invalid("the 'loca' table is empty"))?;
    let mut glyphs: HashMap<String, &[u8]> = HashMap::new();
    for (i, next) in offsets.iter().skip(1).enumerate() {
        let name = order.get(i).cloned().unwrap_or_else(|| format!("ttxautoglyph{i}"));
        let bytes = data.get(pos..*next).ok_or_else(|| TablesError::invalid("not enough 'glyf' table data"))?;
        glyphs.insert(name, bytes);
        pos = *next;
    }
    let mut names: Vec<&String> = order.iter().collect();
    names.sort();
    w.blank();
    w.comment("The xMin, yMin, xMax and yMax values\nwill be recalculated by the compiler.");
    w.blank();
    for name in names {
        let Some(bytes) = glyphs.get(name.as_str()) else { continue };
        glyph(bytes, name, &order, w)?;
        w.blank();
        w.check()?;
    }
    Ok(())
}

/// `glyf` as `ttx -g` writes it: in `w`, a `TTGlyph` naming a file for each glyph with an
/// outline (empty glyphs stay inline); each file, named from `prefix`, the glyph name and `ext`,
/// is returned with its text.
pub(crate) fn write_split(
    data: &[u8],
    font: &SfntFont,
    w: &mut XmlWriter,
    prefix: &str,
    ext: &str,
    no_version: bool,
) -> Result<Vec<(String, String)>> {
    let loca = font.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
    let offsets = loca_offsets(loca, font)?;
    let order = w.glyph_order(font)?;
    let mut pos = *offsets.first().ok_or_else(|| TablesError::invalid("the 'loca' table is empty"))?;
    let mut glyphs: HashMap<String, &[u8]> = HashMap::new();
    for (i, next) in offsets.iter().skip(1).enumerate() {
        let name = order.get(i).cloned().unwrap_or_else(|| format!("ttxautoglyph{i}"));
        let bytes = data.get(pos..*next).ok_or_else(|| TablesError::invalid("not enough 'glyf' table data"))?;
        glyphs.insert(name, bytes);
        pos = *next;
    }
    let mut names: Vec<&String> = order.iter().collect();
    names.sort();
    let mut existing: HashSet<String> = HashSet::new();
    let mut files = Vec::new();
    for name in names {
        let Some(bytes) = glyphs.get(name.as_str()) else { continue };
        let contours = bytes.get(..2).map_or(0, |b| i16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]));
        if contours == 0 {
            w.simple_comment("TTGlyph", &[("name", name)], "contains no outline data");
            continue;
        }
        let file = user_name_to_file_name(name, &existing, prefix, ext)?;
        existing.insert(file.to_lowercase());
        w.simple("TTGlyph", &[("src", &file)]);
        let mut g = w.child_at(0);
        g.set_depth(0);
        g.line(super::super::XML_DECLARATION);
        if no_version {
            g.open("ttFont", &[]);
        } else {
            g.open("ttFont", &[("ttLibVersion", super::super::TTX_VERSION)]);
        }
        g.open("glyf", &[]);
        g.comment("The xMin, yMin, xMax and yMax values\nwill be recalculated by the compiler.");
        glyph(bytes, name, &order, &mut g)?;
        g.close("glyf");
        g.close("ttFont");
        g.check()?;
        files.push((file, g.finish()));
        w.check()?;
    }
    Ok(files)
}

/// fontTools' `userNameToFileName` (from ufoLib, MIT): illegal characters become `_`, capitals
/// get a `_` after them, reserved names a `_` before them, and a clash with an existing name
/// (compared without case) takes a 15-digit counter.
fn user_name_to_file_name(user: &str, existing: &HashSet<String>, prefix: &str, suffix: &str) -> Result<String> {
    // fontTools' list also holds the two-character strings `\"` and `\0`, which never match.
    const ILLEGAL: &[char] = &['*', '+', '/', ':', '<', '>', '?', '[', '\\', ']', '|', '\u{7f}'];
    const RESERVED: [&str; 13] = ["con", "prn", "aux", "clock$", "nul", "a:-z:", "com1", "lpt1", "lpt2", "lpt3", "com2", "com3", "com4"];
    const MAX_LEN: usize = 255;
    let mut name: String =
        if prefix.is_empty() && user.starts_with('.') { format!("_{}", user.get(1..).unwrap_or_default()) } else { user.to_string() };
    let mut filtered = String::with_capacity(name.len());
    for c in name.chars() {
        if ILLEGAL.contains(&c) || (u32::from(c) >= 1 && u32::from(c) < 32) {
            filtered.push('_');
        } else if c.to_lowercase().ne(std::iter::once(c)) {
            filtered.push(c);
            filtered.push('_');
        } else {
            filtered.push(c);
        }
    }
    let keep = MAX_LEN.saturating_sub(prefix.chars().count()).saturating_sub(suffix.chars().count());
    name = filtered.chars().take(keep).collect();
    name = name
        .split('.')
        .map(|p| if RESERVED.contains(&p.to_lowercase().as_str()) { format!("_{p}") } else { p.to_string() })
        .collect::<Vec<_>>()
        .join(".");
    let full = format!("{prefix}{name}{suffix}");
    if !existing.contains(&full.to_lowercase()) {
        return Ok(full);
    }
    let room = MAX_LEN.saturating_sub(prefix.chars().count() + suffix.chars().count() + 15);
    let short: String =
        if prefix.chars().count() + name.chars().count() + suffix.chars().count() + 15 > MAX_LEN { name.chars().take(room).collect() } else { name };
    // Each try adds a name to `existing` only once the caller takes it, so the counter needs no
    // more tries than there are names already taken.
    for counter in 1..=existing.len().saturating_add(1) {
        let full = format!("{prefix}{short}{counter:015}{suffix}");
        if !existing.contains(&full.to_lowercase()) {
            return Ok(full);
        }
    }
    Err(TablesError::invalid(format!("no file name could be made for the glyph '{user}'")))
}

fn glyph(data: &[u8], name: &str, order: &[String], w: &mut XmlWriter) -> Result<()> {
    if data.is_empty() {
        w.simple_comment("TTGlyph", &[("name", name)], "contains no outline data");
        return Ok(());
    }
    let mut r = Rd::new(data);
    let contours = r.i16()?;
    let bbox: Vec<String> = (0..4).map(|_| r.i16().map(|v| v.to_string())).collect::<Result<_>>()?;
    if contours == 0 {
        w.simple_comment("TTGlyph", &[("name", name)], "contains no outline data");
        return Ok(());
    }
    let attrs: Vec<(&str, &str)> = ["xMin", "yMin", "xMax", "yMax"].into_iter().zip(bbox.iter().map(String::as_str)).collect();
    let mut all = vec![("name", name)];
    all.extend(attrs);
    let mut body = w.child_at(1);
    if contours == -1 {
        composite(r.rest(), order, &mut body)?;
    } else if contours > 0 {
        simple(r.rest(), usize::try_from(contours).unwrap_or(0), &mut body)?;
    } else {
        return Err(bad("a negative contour count other than -1"));
    }
    w.open("TTGlyph", &all);
    w.append(body);
    w.close("TTGlyph");
    Ok(())
}

fn simple(data: &[u8], contours: usize, w: &mut XmlWriter) -> Result<()> {
    let mut r = Rd::new(data);
    let mut ends: Vec<usize> = Vec::with_capacity(contours);
    for _ in 0..contours {
        let end = usize::from(r.u16()?);
        // Decreasing end points would make later contours repeat earlier points.
        if ends.last().is_some_and(|&last| end < last) {
            return Err(bad("the contour end points decrease"));
        }
        ends.push(end);
    }
    let instruction_length = usize::try_from(r.i16()?).map_err(|_| bad("a negative instruction length"))?;
    let instructions = r.bytes(instruction_length)?;
    let points = ends.last().copied().unwrap_or(0).saturating_add(1);
    let mut flags: Vec<u8> = Vec::with_capacity(points);
    while flags.len() < points {
        let flag = r.u8()?;
        let repeat = if flag & REPEAT != 0 { usize::from(r.u8()?).saturating_add(1) } else { 1 };
        if flags.len().saturating_add(repeat) > points {
            return Err(bad("the flags run past the last point"));
        }
        flags.extend(std::iter::repeat_n(flag, repeat));
    }
    let mut coords: Vec<(i64, i64)> = Vec::with_capacity(points);
    let (mut x, mut y) = (0i64, 0i64);
    let mut xs = Vec::with_capacity(points);
    for &f in &flags {
        let dx = if f & X_SHORT != 0 {
            let v = i64::from(r.u8()?);
            if f & X_SAME != 0 { v } else { -v }
        } else if f & X_SAME != 0 {
            0
        } else {
            i64::from(r.i16()?)
        };
        xs.push(dx);
    }
    for (&f, dx) in flags.iter().zip(&xs) {
        let dy = if f & Y_SHORT != 0 {
            let v = i64::from(r.u8()?);
            if f & Y_SAME != 0 { v } else { -v }
        } else if f & Y_SAME != 0 {
            0
        } else {
            i64::from(r.i16()?)
        };
        x = x.saturating_add(*dx);
        y = y.saturating_add(dy);
        coords.push((x, y));
    }
    let mut first = 0usize;
    for end in &ends {
        w.open("contour", &[]);
        for j in first..=*end {
            let (Some((px, py)), Some(f)) = (coords.get(j), flags.get(j)) else {
                return Err(bad("a contour ends past the last point"));
            };
            let (sx, sy, on) = (px.to_string(), py.to_string(), (f & ON_CURVE).to_string());
            let mut attrs = vec![("x", sx.as_str()), ("y", sy.as_str()), ("on", on.as_str())];
            if f & OVERLAP_SIMPLE != 0 {
                attrs.push(("overlap", "1"));
            }
            if f & CUBIC != 0 {
                attrs.push(("cubic", "1"));
            }
            w.simple("pt", &attrs);
        }
        first = end.saturating_add(1);
        w.close("contour");
    }
    instructions_element(instructions, w);
    Ok(())
}

fn f2dot14(v: i16) -> String {
    fixed_to_str(f64::from(v) / 16384.0, 14)
}

fn composite(data: &[u8], order: &[String], w: &mut XmlWriter) -> Result<()> {
    let mut r = Rd::new(data);
    let mut have_instructions = false;
    loop {
        let flags = r.u16()?;
        let gid = usize::from(r.u16()?);
        let name = order.get(gid).ok_or_else(|| bad("a component names a glyph past the glyph order"))?;
        let words = flags & ARG_1_AND_2_ARE_WORDS != 0;
        let (a1, a2) = match (words, flags & ARGS_ARE_XY_VALUES != 0) {
            (true, true) => (i64::from(r.i16()?), i64::from(r.i16()?)),
            (true, false) => (i64::from(r.u16()?), i64::from(r.u16()?)),
            (false, true) => (i64::from(r.i8()?), i64::from(r.i8()?)),
            (false, false) => (i64::from(r.u8()?), i64::from(r.u8()?)),
        };
        let transform = if flags & WE_HAVE_A_SCALE != 0 {
            let s = r.i16()?;
            Some([s, 0, 0, s])
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let (sx, sy) = (r.i16()?, r.i16()?);
            Some([sx, 0, 0, sy])
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            Some([r.i16()?, r.i16()?, r.i16()?, r.i16()?])
        } else {
            None
        };
        let (s1, s2) = (a1.to_string(), a2.to_string());
        let mut attrs: Vec<(&str, String)> = vec![("glyphName", name.clone())];
        if flags & ARGS_ARE_XY_VALUES != 0 {
            attrs.push(("x", s1));
            attrs.push(("y", s2));
        } else {
            attrs.push(("firstPt", s1));
            attrs.push(("secondPt", s2));
        }
        if let Some([xx, xy, yx, yy]) = transform {
            if xy != 0 || yx != 0 {
                attrs.push(("scalex", f2dot14(xx)));
                attrs.push(("scale01", f2dot14(xy)));
                attrs.push(("scale10", f2dot14(yx)));
                attrs.push(("scaley", f2dot14(yy)));
            } else if xx != yy {
                attrs.push(("scalex", f2dot14(xx)));
                attrs.push(("scaley", f2dot14(yy)));
            } else {
                attrs.push(("scale", f2dot14(xx)));
            }
        }
        attrs.push(("flags", py_hex(i64::from(flags & KEPT_FLAGS))));
        let borrowed: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.simple("component", &borrowed);
        have_instructions |= flags & WE_HAVE_INSTRUCTIONS != 0;
        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }
    if have_instructions {
        let n = usize::try_from(r.i16()?).map_err(|_| bad("a negative instruction length"))?;
        let rest = r.rest();
        instructions_element(rest.get(..n.min(rest.len())).unwrap_or_default(), w);
    }
    Ok(())
}

fn instructions_element(code: &[u8], w: &mut XmlWriter) {
    if code.is_empty() {
        w.simple("instructions", &[]);
    } else {
        w.open("instructions", &[]);
        program::write(code, w);
        w.close("instructions");
    }
}

#[cfg(test)]
mod file_names {
    use super::*;

    #[test]
    fn glyph_file_names_follow_the_ufo_rules() {
        let none = HashSet::new();
        let name = |n: &str| user_name_to_file_name(n, &none, "", "").unwrap();
        assert_eq!(name("a"), "a");
        assert_eq!(name("A"), "A_");
        assert_eq!(name("A.alT"), "A_.alT_");
        assert_eq!(name("T_H"), "T__H_");
        assert_eq!(name(".notdef"), "_notdef");
        assert_eq!(name("con"), "_con");
        assert_eq!(name("CON"), "C_O_N_");
        assert_eq!(name("con.alt"), "_con.alt");
        assert_eq!(name("a/b|c"), "a_b_c");
        assert_eq!(user_name_to_file_name(".notdef", &none, "x.", ".ttx").unwrap(), "x..notdef.ttx");
        let taken: HashSet<String> = ["x.a_.ttx".to_string()].into();
        assert_eq!(user_name_to_file_name("A", &taken, "x.", ".ttx").unwrap(), "x.A_000000000000001.ttx");
    }
}

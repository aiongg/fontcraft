//! Top DICT and Private DICT serialisation.
//!
//! Entry order, defaults and operand forms follow fontTools `cffLib` (`topDictOperators`,
//! `privateDictOperators`, `DictCompiler`; MIT, Copyright (c) 2017 Just van Rossum and
//! contributors): entries are written in the table order, a value equal to its default is left
//! out, arrays of blue zones and stem snaps are written as deltas, and offsets are integers in
//! their shortest form (so the Top DICT's size depends on where things land; see the layout pass
//! in `lib.rs`).

use crate::number::{Number, encode_dict_int, encode_dict_number};
use crate::{Error, PrivateDict, TopDict};

/// Operands one DICT entry may have.
const MAX_OPERANDS: usize = 48;

/// String ids of the Top DICT's string entries, allocated after the charset's.
#[derive(Clone, Copy, Default)]
pub(crate) struct TopSids {
    pub version: Option<u16>,
    pub notice: Option<u16>,
    pub copyright: Option<u16>,
    pub full_name: Option<u16>,
    pub family_name: Option<u16>,
    pub weight: Option<u16>,
}

/// Where the parts the Top DICT points at landed, once the layout pass knows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Pointers {
    /// A predefined charset id, or the charset's offset.
    pub charset: Option<i64>,
    /// Private DICT size and offset.
    pub private: Option<(i64, i64)>,
    pub charstrings: Option<i64>,
}

struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn op(&mut self, op: &[u8]) {
        self.out.extend_from_slice(op);
    }

    fn int(&mut self, v: i64, what: &str) -> Result<(), Error> {
        encode_dict_int(v, what, &mut self.out)
    }

    fn sid(&mut self, sid: Option<u16>, op: &[u8], what: &str) -> Result<(), Error> {
        if let Some(sid) = sid {
            self.int(i64::from(sid), what)?;
            self.op(op);
        }
        Ok(())
    }

    /// A number entry, left out when it equals its default.
    fn number(&mut self, v: Number, default: Option<Number>, op: &[u8], what: &str) -> Result<(), Error> {
        if default.is_some_and(|d| v.same_value(d)) {
            return Ok(());
        }
        encode_dict_number(v, what, &mut self.out)?;
        self.op(op);
        Ok(())
    }

    /// An array entry, left out when it equals its default element by element.
    fn array(&mut self, v: &[Number], default: Option<&[Number]>, op: &[u8], what: &str) -> Result<(), Error> {
        if default.is_some_and(|d| d.len() == v.len() && d.iter().zip(v).all(|(a, b)| a.same_value(*b))) {
            return Ok(());
        }
        check_operands(v.len(), what)?;
        for &n in v {
            encode_dict_number(n, what, &mut self.out)?;
        }
        self.op(op);
        Ok(())
    }

    /// A delta-encoded array (blue zones, stem snaps), left out when empty.
    fn delta(&mut self, v: &[Number], op: &[u8], what: &str) -> Result<(), Error> {
        if v.is_empty() {
            return Ok(());
        }
        check_operands(v.len(), what)?;
        let mut last = Number::Int(0);
        for &n in v {
            encode_dict_number(n.minus(last), what, &mut self.out)?;
            last = n;
        }
        self.op(op);
        Ok(())
    }
}

fn check_operands(count: usize, what: &str) -> Result<(), Error> {
    if count > MAX_OPERANDS {
        return Err(Error::TooManyOperands { what: what.to_string(), count });
    }
    Ok(())
}

/// The CFF default font matrix.
pub(crate) const DEFAULT_MATRIX: [Number; 6] =
    [Number::Real(0.001), Number::Int(0), Number::Int(0), Number::Real(0.001), Number::Int(0), Number::Int(0)];

/// The Top DICT's bytes for this state of the layout.
pub(crate) fn top_dict(top: &TopDict, sids: &TopSids, font_bbox: [i64; 4], at: &Pointers) -> Result<Vec<u8>, Error> {
    let mut w = Writer { out: Vec::new() };
    w.sid(sids.version, &[0], "version")?;
    w.sid(sids.notice, &[1], "Notice")?;
    w.sid(sids.copyright, &[12, 0], "Copyright")?;
    w.sid(sids.full_name, &[2], "FullName")?;
    w.sid(sids.family_name, &[3], "FamilyName")?;
    w.sid(sids.weight, &[4], "Weight")?;
    w.number(top.is_fixed_pitch, Some(Number::Int(0)), &[12, 1], "isFixedPitch")?;
    w.number(top.italic_angle, Some(Number::Int(0)), &[12, 2], "ItalicAngle")?;
    w.number(top.underline_position, Some(Number::Int(-100)), &[12, 3], "UnderlinePosition")?;
    w.number(top.underline_thickness, Some(Number::Int(50)), &[12, 4], "UnderlineThickness")?;
    w.number(top.paint_type, Some(Number::Int(0)), &[12, 5], "PaintType")?;
    w.array(&top.font_matrix, Some(&DEFAULT_MATRIX), &[12, 7], "FontMatrix")?;
    let bbox = font_bbox.map(Number::Int);
    w.array(&bbox, Some(&[Number::Int(0); 4]), &[5], "FontBBox")?;
    w.number(top.stroke_width, Some(Number::Int(0)), &[12, 8], "StrokeWidth")?;
    if let Some(c) = at.charset {
        w.int(c, "charset")?;
        w.op(&[15]);
    }
    if let Some((size, offset)) = at.private {
        w.int(size, "Private")?;
        w.int(offset, "Private")?;
        w.op(&[18]);
    }
    if let Some(c) = at.charstrings {
        w.int(c, "CharStrings")?;
        w.op(&[17]);
    }
    Ok(w.out)
}

/// The Private DICT's bytes.
pub(crate) fn private_dict(p: &PrivateDict, default_width_x: i64, nominal_width_x: i64) -> Result<Vec<u8>, Error> {
    let mut w = Writer { out: Vec::new() };
    w.delta(&p.blue_values, &[6], "BlueValues")?;
    w.delta(&p.other_blues, &[7], "OtherBlues")?;
    w.delta(&p.family_blues, &[8], "FamilyBlues")?;
    w.delta(&p.family_other_blues, &[9], "FamilyOtherBlues")?;
    w.number(p.blue_scale, Some(Number::Real(0.039625)), &[12, 9], "BlueScale")?;
    w.number(p.blue_shift, Some(Number::Int(7)), &[12, 10], "BlueShift")?;
    w.number(p.blue_fuzz, Some(Number::Int(1)), &[12, 11], "BlueFuzz")?;
    if let Some(v) = p.std_hw {
        w.number(v, None, &[10], "StdHW")?;
    }
    if let Some(v) = p.std_vw {
        w.number(v, None, &[11], "StdVW")?;
    }
    w.delta(&p.stem_snap_h, &[12, 12], "StemSnapH")?;
    w.delta(&p.stem_snap_v, &[12, 13], "StemSnapV")?;
    w.number(p.force_bold, Some(Number::Int(0)), &[12, 14], "ForceBold")?;
    w.number(p.language_group, Some(Number::Int(0)), &[12, 17], "LanguageGroup")?;
    w.number(Number::Int(default_width_x), Some(Number::Int(0)), &[20], "defaultWidthX")?;
    w.number(Number::Int(nominal_width_x), Some(Number::Int(0)), &[21], "nominalWidthX")?;
    Ok(w.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_left_out() {
        let bytes = top_dict(&TopDict::default(), &TopSids::default(), [0; 4], &Pointers::default()).unwrap();
        assert!(bytes.is_empty());
        let bytes = private_dict(&PrivateDict::default(), 0, 0).unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn an_integer_equal_to_a_real_default_is_still_a_default() {
        let top = TopDict { italic_angle: Number::Real(0.0), ..TopDict::default() };
        assert!(top_dict(&top, &TopSids::default(), [0; 4], &Pointers::default()).unwrap().is_empty());
        let top = TopDict { italic_angle: Number::Real(-12.0), ..TopDict::default() };
        let bytes = top_dict(&top, &TopSids::default(), [0; 4], &Pointers::default()).unwrap();
        assert_eq!(bytes, [0x1E, 0xE1, 0x2F, 12, 2]);
    }

    #[test]
    fn entries_follow_the_reference_order() {
        let sids = TopSids { version: Some(391), weight: Some(388), ..TopSids::default() };
        let at = Pointers { charset: Some(0), private: Some((10, 500)), charstrings: Some(200) };
        let bytes = top_dict(&TopDict::default(), &sids, [-10, -200, 1000, 800], &at).unwrap();
        let mut want = vec![248, 27, 0]; // version
        want.extend([248, 24, 4]); // Weight
        want.extend([129, 251, 92, 250, 124, 249, 180, 5]); // FontBBox
        want.extend([139, 15]); // charset 0
        want.extend([149, 248, 136, 18]); // Private
        want.extend([247, 92, 17]); // CharStrings
        assert_eq!(bytes, want);
    }

    #[test]
    fn blues_are_deltas() {
        let p = PrivateDict {
            blue_values: vec![Number::Int(-15), Number::Int(0), Number::Int(500), Number::Int(515)],
            std_hw: Some(Number::Int(50)),
            ..PrivateDict::default()
        };
        let bytes = private_dict(&p, 500, 600).unwrap();
        // -15 15 500 15 BlueValues, 50 StdHW, 500 defaultWidthX, 600 nominalWidthX
        let want = vec![124, 154, 248, 136, 154, 6, 189, 10, 248, 136, 20, 248, 236, 21];
        assert_eq!(bytes, want);
        let many = PrivateDict { stem_snap_h: vec![Number::Int(1); 49], ..PrivateDict::default() };
        assert!(matches!(private_dict(&many, 0, 0), Err(Error::TooManyOperands { .. })));
    }
}

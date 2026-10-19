//! The generic walk: a table's bytes read field by field as its layout in [`super::schema`]
//! says, and written as `ttx` writes them. Ported from fontTools' `otBase.BaseTable`,
//! `otConverters` and the `postRead`/`toXML2` methods of `otTables` (MIT).
//!
//! Reading and writing happen in one pass, in field order, which is the order `ttx` writes
//! them in. A table that fails to read anywhere fails as a whole, so a partial write never
//! reaches the dump.
//!
//! The rules the walk keeps from fontTools:
//! - offsets count from the start of the table that holds them; a record inside a table reads
//!   with that table's reader, so its offsets count from the same start;
//! - a null offset writes nothing, or `<Name index="i" empty="1"/>` inside an array;
//! - a count field is written as a comment; the counts named in [`PROPAGATED`] and the value
//!   formats are passed down to the tables read after them;
//! - an array of fixed-size numbers that runs past the end of the table is cut short where the
//!   data ends, as Python's `array` reads it, and fails only when a number is split;
//! - Coverage, ClassDef, the four simple substitutions and the delta-set index maps are written
//!   in their summarised forms, sorted by glyph name.

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::limits::{MAX_LAYOUT_ITEMS, MAX_TTX_DEPTH};
use crate::sfnt::Tag;
use crate::{Result, TablesError};

use super::super::core::glyph_name;
use super::super::{XmlWriter, fixed_to_str, py_float};
use super::schema;

/// How a field is read and written.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Ty {
    U8,
    U16,
    I16,
    U24,
    U32,
    /// A count, written as a comment.
    Count8,
    Count16,
    Count32,
    Tag,
    Glyph,
    NameId,
    StatFlags,
    LookupFlag,
    DeciPoints,
    Fixed,
    F2Dot14,
    Version,
    ValueFormat,
    ValueRecord,
    DeltaValue,
    VarIdxMapValue,
    VarDataValue,
    /// A variation index of 32 bits; `0xFFFFFFFF` (no variation) is written without a value.
    VarIndex,
    /// An angle stored as an F2Dot14 fraction of 180 degrees; the biased kind adds 1.0 first.
    Angle,
    BiasedAngle,
    /// The `uint8` enumerations of COLR, written as their lower-case names.
    ExtendMode,
    CompositeMode,
    /// 64 bytes of ASCII text, ended by the first zero byte.
    Char64,
    /// An AAT lookup table read in place: glyphs to 16-bit values, which are numbers (`uint16`) or
    /// offsets to tables of the named layout.
    AatLookup(&'static str),
    /// Offsets to an AAT lookup and to the data its values count from (the `ankr` table).
    AatLookupData(&'static str),
    /// The CID to glyph map of `cidg`, and the glyph to CID map of `gcid`.
    CidGlyphMap,
    GlyphCidMap,
    /// A record read in place, of the named layout.
    Struct(&'static str),
    /// Offsets of 16, 32 and 24 bits to a table of the named layout.
    Off16(&'static str),
    Off32(&'static str),
    Off24(&'static str),
    /// A lookup subtable; its layout follows `LookupType`.
    SubTable,
    /// The subtable of an extension lookup; its layout follows `ExtensionLookupType`.
    ExtSubTable,
    /// Feature parameters; their layout follows the feature tag.
    FeatureParams,
}

/// How many times a field repeats.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Rep {
    One,
    /// As many times as the named count says, plus the adjustment.
    Count(&'static str, i32),
    /// A fixed number of times.
    Fixed(usize),
}

/// When a field is present.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Cond {
    Always,
    /// The named field has one of these bits set.
    Mask(&'static str, u32),
    /// The named field is at least this value.
    AtLeast(&'static str, i64),
    /// The named field is 1, 2 or 3.
    In123(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Field {
    pub name: &'static str,
    pub ty: Ty,
    pub rep: Rep,
    pub cond: Cond,
}

/// The size of a format number.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Width {
    U8,
    U16,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Layout {
    Plain(&'static [Field]),
    /// A format number first, then the fields of that format.
    Formats(Width, &'static [(u16, &'static [Field])]),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Def {
    pub name: &'static str,
    pub layout: Layout,
}

/// How deep tables may nest. The elements inside the deepest table (a device table inside a
/// value record two levels further down) stay within [`MAX_TTX_DEPTH`], the depth the TTX
/// reader and the JSON form accept.
const MAX_NESTING: usize = MAX_TTX_DEPTH - 4;

/// The values a table passes down to the tables read after it (fontTools' propagated counts).
const PROPAGATED: [&str; 15] = [
    "ClassCount",
    "Class2Count",
    "FeatureTag",
    "SettingsCount",
    "VarRegionCount",
    "MappingCount",
    "RegionAxisCount",
    "DesignAxisCount",
    "DesignAxisRecordSize",
    "AxisValueCount",
    "ValueRecordSize",
    "AxisCount",
    "BaseGlyphRecordCount",
    "LayerRecordCount",
    "AxisIndicesList",
];

/// The fields of a value record, in order: name, whether it is an offset to a device table,
/// whether it is signed. The reserved bits read a number each and are not written.
const VALUE_RECORD: [(&str, bool, bool); 16] = [
    ("XPlacement", false, true),
    ("YPlacement", false, true),
    ("XAdvance", false, true),
    ("YAdvance", false, true),
    ("XPlaDevice", true, false),
    ("YPlaDevice", true, false),
    ("XAdvDevice", true, false),
    ("YAdvDevice", true, false),
    ("Reserved1", false, false),
    ("Reserved2", false, false),
    ("Reserved3", false, false),
    ("Reserved4", false, false),
    ("Reserved5", false, false),
    ("Reserved6", false, false),
    ("Reserved7", false, false),
    ("Reserved8", false, false),
];

/// A position in the table, the start its offsets count from, and what earlier tables passed
/// down (fontTools' `OTTableReader`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Reader {
    base: usize,
    pos: usize,
    state: Rc<Vec<(&'static str, i64)>>,
}

impl Reader {
    /// A reader for the table at `offset` from this one's start, carrying the same state.
    pub(super) fn sub(&self, offset: u32) -> Reader {
        let at = self.base.saturating_add(usize::try_from(offset).unwrap_or(usize::MAX));
        Reader { base: at, pos: at, state: Rc::clone(&self.state) }
    }

    fn get(&self, name: &str) -> Option<i64> {
        self.state.iter().rev().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    fn set(&mut self, name: &'static str, value: i64) {
        let mut state: Vec<(&'static str, i64)> = self.state.iter().filter(|(n, _)| *n != name).copied().collect();
        state.push((name, value));
        self.state = Rc::new(state);
    }
}

/// The values of the fields read so far in one table.
#[derive(Default)]
struct Dict(Vec<(&'static str, i64)>);

impl Dict {
    fn get(&self, name: &str) -> Option<i64> {
        self.0.iter().rev().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    fn need(&self, name: &str, class: &str) -> Result<i64> {
        self.get(name).ok_or_else(|| TablesError::invalid(format!("'{class}' has no '{name}' before it is needed")))
    }
}

/// A walk over one table.
pub(crate) struct Walker<'a> {
    data: &'a [u8],
    /// The tag of the table being walked; lookup subtables follow it (`JSTF` holds GPOS lookups).
    table: Tag,
    order: Rc<Vec<String>>,
    /// The `name` table, for the comments after name IDs.
    name_table: Option<&'a [u8]>,
}

impl<'a> Walker<'a> {
    pub(crate) fn new(data: &'a [u8], table: Tag, order: Rc<Vec<String>>, name_table: Option<&'a [u8]>) -> Self {
        Walker { data, table, order, name_table }
    }

    fn err(&self, what: impl std::fmt::Display) -> TablesError {
        TablesError::invalid(format!("table '{}': {what}", crate::sfnt::tag_str(self.table)))
    }

    fn name(&self, gid: u32) -> String {
        glyph_name(&self.order, gid)
    }

    // ── Reading ───────────────────────────────────────────────────────────────────────────────

    fn take(&self, r: &mut Reader, n: usize) -> Result<&'a [u8]> {
        let end = r.pos.checked_add(n).ok_or_else(|| self.err("an offset past the end of the table"))?;
        let bytes = self.data.get(r.pos..end).ok_or_else(|| self.err("the data runs past the end of the table"))?;
        r.pos = end;
        Ok(bytes)
    }

    pub(super) fn u8(&self, r: &mut Reader) -> Result<u8> {
        Ok(self.take(r, 1)?.first().copied().unwrap_or(0))
    }

    pub(super) fn u16(&self, r: &mut Reader) -> Result<u16> {
        let b = self.take(r, 2)?;
        Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }

    fn u24(&self, r: &mut Reader) -> Result<u32> {
        let b = self.take(r, 3)?;
        Ok(b.iter().fold(0u32, |v, x| (v << 8) | u32::from(*x)))
    }

    pub(super) fn u32(&self, r: &mut Reader) -> Result<u32> {
        let b = self.take(r, 4)?;
        Ok(b.iter().fold(0u32, |v, x| (v << 8) | u32::from(*x)))
    }

    /// The bytes of `count` numbers of `size` bytes, read as Python's `array` reads them: cut
    /// short where the table ends, an error only when the end splits a number. A count below
    /// zero moves the position back, as fontTools' reader does.
    fn array(&self, r: &mut Reader, count: i64, size: usize) -> Result<&'a [u8]> {
        if count <= 0 {
            let back = usize::try_from(count.unsigned_abs()).unwrap_or(usize::MAX).saturating_mul(size);
            r.pos = r.pos.saturating_sub(back);
            return Ok(&[]);
        }
        let len = usize::try_from(count).unwrap_or(usize::MAX).saturating_mul(size);
        let end = r.pos.saturating_add(len);
        let bytes = self.data.get(r.pos.min(self.data.len())..end.min(self.data.len())).unwrap_or_default();
        if size == 0 || !bytes.len().is_multiple_of(size) {
            return Err(self.err("an array ends part-way through a number"));
        }
        r.pos = end;
        Ok(bytes)
    }

    /// Unsigned numbers of `size` bytes from [`Self::array`].
    fn uints(&self, r: &mut Reader, count: i64, size: usize) -> Result<Vec<u32>> {
        let bytes = self.array(r, count, size)?;
        Ok(bytes.chunks(size.max(1)).map(|c| c.iter().fold(0u32, |v, x| (v << 8) | u32::from(*x))).collect())
    }

    /// Signed numbers of `size` bytes (1, 2 or 4) from [`Self::array`].
    fn ints(&self, r: &mut Reader, count: i64, size: usize) -> Result<Vec<i64>> {
        Ok(self.uints(r, count, size)?.into_iter().map(|v| sign(v, size)).collect())
    }

    /// `count` 16-bit offsets, read one by one (an error at the end of the table).
    fn offsets16(&self, r: &mut Reader, count: u16) -> Result<Vec<u16>> {
        let mut out = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            out.push(self.u16(r)?);
        }
        Ok(out)
    }

    // ── Tables ────────────────────────────────────────────────────────────────────────────────

    /// Writes the fields of the top-level layout `class` straight into `w`, with no element of
    /// its own (`ttx` writes the table's element).
    pub(crate) fn write_top(&self, class: &str, w: &mut XmlWriter) -> Result<()> {
        let def = schema::def(class).ok_or_else(|| self.err(format!("no layout for '{class}'")))?;
        let mut r = Reader::default();
        if class == "COLR" {
            // LayerRecordCount follows the offset of the array it counts (fontTools' `COLR.decompile`).
            let mut at = Reader { pos: 12, ..Reader::default() };
            let count = self.u16(&mut at)?;
            r.set("LayerRecordCount", i64::from(count));
        }
        let Layout::Plain(fields) = def.layout else {
            return Err(self.err(format!("'{class}' is not a top-level layout")));
        };
        self.fields(class, fields, &mut r, w, 1)?;
        Ok(())
    }

    /// Writes the table of layout `class` at the reader's position as element `element`.
    pub(super) fn write_struct(
        &self,
        class: &str,
        r: &mut Reader,
        w: &mut XmlWriter,
        element: &str,
        index: Option<usize>,
        depth: usize,
    ) -> Result<()> {
        let attrs: Vec<(&str, String)> = index.map(|i| vec![("index", i.to_string())]).unwrap_or_default();
        self.write_struct_with(class, r, w, element, attrs, depth)
    }

    /// [`Self::write_struct`] with the element's leading attributes given (`index`, or the
    /// `glyph` of an AAT lookup entry).
    fn write_struct_with(
        &self,
        class: &str,
        r: &mut Reader,
        w: &mut XmlWriter,
        element: &str,
        mut attrs: Vec<(&'static str, String)>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_NESTING {
            return Err(TablesError::TooLarge { what: "the nesting of subtables", limit: MAX_NESTING });
        }
        w.check()?;
        match class {
            "Coverage" => return self.coverage(r, w, element, &attrs),
            "ClassDef" => return self.class_def(r, w, element, &attrs),
            "SingleSubst" | "MultipleSubst" | "AlternateSubst" | "LigatureSubst" => return self.simple_subst(class, r, w, element, &attrs),
            "DeltaSetIndexMap" => return self.delta_set_index_map(r, w, element, &attrs),
            "VarIdxMap" => return self.var_idx_map(r, w, element, &attrs),
            "ClipList" => return self.clip_list(r, w, element, &attrs, depth),
            _ => {}
        }
        let def = schema::def(class).ok_or_else(|| self.err(format!("no layout for '{class}'")))?;
        let fields: &[Field] = match def.layout {
            Layout::Plain(fields) => {
                if fields.first().is_some_and(|f| f.name == "Format") {
                    let mut peek = r.clone();
                    attrs.push(("Format", self.u16(&mut peek)?.to_string()));
                }
                fields
            }
            Layout::Formats(width, formats) => {
                let format = match width {
                    Width::U8 => u16::from(self.u8(r)?),
                    Width::U16 => self.u16(r)?,
                };
                attrs.push(("Format", format.to_string()));
                if class == "Paint" {
                    // A Paint names its format in a comment after the start tag (fontTools' `Paint.toXML`).
                    let Some(kind) = PAINT_FORMATS.get(usize::from(format).wrapping_sub(1)) else {
                        return Err(self.err(format!("unknown Paint format {format}")));
                    };
                    let fields = formats.iter().find(|(n, _)| *n == format).map_or(&[][..], |(_, f)| *f);
                    let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    w.open_with_comment(element, &attrs, kind);
                    self.fields(class, fields, r, w, depth)?;
                    w.close(element);
                    return Ok(());
                }
                formats.iter().find(|(n, _)| *n == format).map_or(&[][..], |(_, f)| *f)
            }
        };
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.open(element, &attrs);
        self.fields(class, fields, r, w, depth)?;
        w.close(element);
        Ok(())
    }

    fn fields(&self, class: &str, fields: &[Field], r: &mut Reader, w: &mut XmlWriter, depth: usize) -> Result<()> {
        let mut dict = Dict::default();
        for f in fields {
            match f.rep {
                Rep::One => {
                    if !self.holds(f.cond, &dict, class)? {
                        continue;
                    }
                    self.field(class, f, None, r, w, &mut dict, depth)?;
                }
                Rep::Count(count, adjust) => {
                    let n = match dict.get(count).or_else(|| r.get(count)) {
                        Some(n) => n.saturating_add(i64::from(adjust)),
                        None => return Err(self.err(format!("'{class}' needs '{count}', which no table gives"))),
                    };
                    self.array_field(class, f, n, r, w, &mut dict, depth)?;
                }
                Rep::Fixed(n) => {
                    let n = i64::try_from(n).unwrap_or(0);
                    self.array_field(class, f, n, r, w, &mut dict, depth)?;
                }
            }
            w.check()?;
        }
        Ok(())
    }

    fn holds(&self, cond: Cond, dict: &Dict, class: &str) -> Result<bool> {
        Ok(match cond {
            Cond::Always => true,
            Cond::Mask(name, mask) => dict.need(name, class)? & i64::from(mask) != 0,
            Cond::AtLeast(name, v) => dict.need(name, class)? >= v,
            Cond::In123(name) => (1..=3).contains(&dict.need(name, class)?),
        })
    }

    /// Writes an array field of `n` items.
    #[allow(clippy::too_many_arguments)]
    fn array_field(&self, class: &str, f: &Field, n: i64, r: &mut Reader, w: &mut XmlWriter, dict: &mut Dict, depth: usize) -> Result<()> {
        let numbers: Option<Vec<i64>> = match f.ty {
            Ty::U8 => Some(self.uints(r, n, 1)?.into_iter().map(i64::from).collect()),
            Ty::U16 | Ty::NameId | Ty::StatFlags | Ty::LookupFlag | Ty::Glyph => Some(self.uints(r, n, 2)?.into_iter().map(i64::from).collect()),
            Ty::I16 => Some(self.ints(r, n, 2)?),
            Ty::U32 => Some(self.uints(r, n, 4)?.into_iter().map(i64::from).collect()),
            _ => None,
        };
        if let Some(values) = numbers {
            for (i, v) in values.into_iter().enumerate() {
                let index = i.to_string();
                self.write_number(f.name, f.ty, v, Some(&index), w);
                w.check()?;
            }
            return Ok(());
        }
        let count = usize::try_from(n).unwrap_or(0);
        for i in 0..count {
            self.field(class, f, Some(i), r, w, dict, depth)?;
            w.check()?;
        }
        Ok(())
    }

    /// Reads one field (item `index` of an array, or a single field) and writes it.
    #[allow(clippy::too_many_arguments)]
    fn field(&self, class: &str, f: &Field, index: Option<usize>, r: &mut Reader, w: &mut XmlWriter, dict: &mut Dict, depth: usize) -> Result<()> {
        let index_text = index.map(|i| i.to_string());
        let idx = index_text.as_deref();
        let value: i64 = match f.ty {
            Ty::U8 | Ty::Count8 => i64::from(self.u8(r)?),
            Ty::ExtendMode | Ty::CompositeMode => {
                let v = i64::from(self.u8(r)?);
                if enum_name(f.ty, v).is_none() {
                    return Err(self.err(format!("'{class}' has an unknown {} {v}", f.name)));
                }
                v
            }
            Ty::U16 | Ty::Count16 | Ty::NameId | Ty::StatFlags | Ty::LookupFlag | Ty::Glyph | Ty::DeciPoints | Ty::ValueFormat => {
                i64::from(self.u16(r)?)
            }
            Ty::I16 | Ty::F2Dot14 | Ty::Angle | Ty::BiasedAngle => i64::from(self.u16(r)? as i16),
            Ty::U24 => i64::from(self.u24(r)?),
            Ty::U32 | Ty::Count32 | Ty::Tag | Ty::VarIndex => i64::from(self.u32(r)?),
            Ty::Char64 => {
                let text = self.char64(r);
                let mut attrs: Vec<(&str, &str)> = idx.map(|i| vec![("index", i)]).unwrap_or_default();
                attrs.push(("value", &text));
                w.simple(f.name, &attrs);
                return Ok(());
            }
            Ty::AatLookup(value) => return self.aat_lookup(f.name, value, r, w, depth),
            Ty::AatLookupData(table) => return self.aat_lookup_data(f.name, table, r, w, depth),
            Ty::CidGlyphMap => return self.cid_glyph_map(f.name, r, w),
            Ty::GlyphCidMap => return self.glyph_cid_map(f.name, r, w),
            Ty::Fixed | Ty::Version => i64::from(self.u32(r)? as i32),
            Ty::ValueRecord => {
                let which = if f.name.ends_with('2') { "ValueFormat2" } else { "ValueFormat1" };
                let format = r.get(which).ok_or_else(|| self.err(format!("'{class}' has a value record before its value format")))?;
                return self.value_record(f.name, format, r, w, idx, depth);
            }
            Ty::DeltaValue => {
                let (start, end, format) = (dict.need("StartSize", class)?, dict.need("EndSize", class)?, dict.need("DeltaFormat", class)?);
                return self.delta_value(f.name, start, end, format, r, w, idx);
            }
            Ty::VarDataValue => {
                let regions = dict.need("VarRegionCount", class)?;
                let shorts = dict.need("NumShorts", class)?;
                return self.var_data_value(f.name, regions, shorts, r, w, idx);
            }
            Ty::VarIdxMapValue => return Err(self.err("a delta-set index outside its map")),
            Ty::Struct(target) => return self.write_struct(target, r, w, f.name, index, depth.saturating_add(1)),
            Ty::Off16(target) => {
                let offset = u32::from(self.u16(r)?);
                return self.offset(target, f.name, offset, r, w, index, depth);
            }
            Ty::Off32(target) => {
                let offset = self.u32(r)?;
                return self.offset(target, f.name, offset, r, w, index, depth);
            }
            Ty::Off24(target) => {
                let offset = self.u24(r)?;
                return self.offset(target, f.name, offset, r, w, index, depth);
            }
            Ty::SubTable => {
                let ty = dict.need("LookupType", class)?;
                let target = lookup_class(self.table, ty).ok_or_else(|| self.err(format!("lookup type {ty} is not defined")))?;
                let offset = u32::from(self.u16(r)?);
                return self.offset(target, target, offset, r, w, index, depth);
            }
            Ty::ExtSubTable => {
                let ty = dict.need("ExtensionLookupType", class)?;
                let target = lookup_class(self.table, ty).ok_or_else(|| self.err(format!("extension lookup type {ty} is not defined")))?;
                let offset = self.u32(r)?;
                return self.offset(target, target, offset, r, w, index, depth);
            }
            Ty::FeatureParams => {
                // fontTools picks the generic layout when nothing has been passed down at all, and
                // fails when other values have been but the feature tag has not.
                let target = match r.get("FeatureTag") {
                    Some(tag) => feature_params_class(tag),
                    None if r.state.is_empty() => "FeatureParams",
                    None => return Err(self.err("feature parameters outside a feature record")),
                };
                let offset = u32::from(self.u16(r)?);
                return self.offset(target, target, offset, r, w, index, depth);
            }
        };
        dict.0.push((f.name, value));
        if PROPAGATED.contains(&f.name) {
            r.set(f.name, value);
        }
        if matches!(f.ty, Ty::ValueFormat) {
            r.set(if f.name.ends_with('2') { "ValueFormat2" } else { "ValueFormat1" }, value);
        }
        self.write_number(f.name, f.ty, value, idx, w);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn offset(&self, class: &str, element: &str, offset: u32, r: &Reader, w: &mut XmlWriter, index: Option<usize>, depth: usize) -> Result<()> {
        if offset == 0 {
            if let Some(i) = index {
                w.simple(element, &[("index", &i.to_string()), ("empty", "1")]);
            }
            return Ok(());
        }
        let mut sub = r.sub(offset);
        self.write_struct(class, &mut sub, w, element, index, depth.saturating_add(1))
    }

    /// Writes a number field as `ttx` writes its kind.
    fn write_number(&self, name: &str, ty: Ty, v: i64, index: Option<&str>, w: &mut XmlWriter) {
        let text = match ty {
            Ty::Count8 | Ty::Count16 | Ty::Count32 => {
                w.comment(&format!("{name}={v}"));
                return;
            }
            Ty::Tag => tag_text(v),
            Ty::Glyph => self.name(u32::try_from(v).unwrap_or(0)),
            Ty::DeciPoints => py_float(v as f64 / 10.0),
            Ty::Fixed => fixed_to_str(v as f64 / 65536.0, 16),
            Ty::F2Dot14 => fixed_to_str(v as f64 / 16384.0, 14),
            Ty::Version => version_text(v),
            Ty::Angle => angle_text(v, 0.0),
            Ty::BiasedAngle => angle_text(v, 1.0),
            Ty::ExtendMode | Ty::CompositeMode => enum_name(ty, v).unwrap_or_default().to_string(),
            Ty::VarIndex if v == 0xFFFF_FFFF => {
                match index {
                    Some(i) => w.simple(name, &[("index", i)]),
                    None => w.simple(name, &[]),
                }
                return;
            }
            _ => v.to_string(),
        };
        let mut attrs: Vec<(&str, &str)> = Vec::with_capacity(2);
        if let Some(i) = index {
            attrs.push(("index", i));
        }
        attrs.push(("value", &text));
        let comment = match ty {
            Ty::LookupFlag => lookup_flag_comment(v),
            Ty::StatFlags => stat_flags_comment(v),
            Ty::NameId if v != 0 => self.name_table.map(|data| match super::debug_name(data, v) {
                Some(n) => n,
                None if v == 0xFFFF => "None".to_string(),
                None => "missing from name table".to_string(),
            }),
            _ => None,
        };
        match comment {
            Some(c) if matches!(ty, Ty::LookupFlag) => w.simple_comment(name, &attrs, &c),
            Some(c) => w.simple_spaced_comment(name, &attrs, &c),
            None => w.simple(name, &attrs),
        }
    }

    // ── Value records, device deltas, variation data ───────────────────────────────────────────

    fn value_record(&self, element: &str, format: i64, r: &mut Reader, w: &mut XmlWriter, index: Option<&str>, depth: usize) -> Result<()> {
        if format & 0xFFFF == 0 {
            return Ok(());
        }
        let mut simple: Vec<(&str, String)> = Vec::new();
        if let Some(i) = index {
            simple.push(("index", i.to_string()));
        }
        let mut devices: Vec<(&str, u32)> = Vec::new();
        for (bit, &(name, device, signed)) in VALUE_RECORD.iter().enumerate() {
            if format & (1 << bit) == 0 {
                continue;
            }
            let raw = self.u16(r)?;
            if device {
                if raw != 0 {
                    devices.push((name, u32::from(raw)));
                }
            } else if bit < 4 {
                let v = if signed { i64::from(raw as i16) } else { i64::from(raw) };
                simple.push((name, v.to_string()));
            }
        }
        let attrs: Vec<(&str, &str)> = simple.iter().map(|(k, v)| (*k, v.as_str())).collect();
        if devices.is_empty() {
            w.simple(element, &attrs);
            return Ok(());
        }
        // Device tables are read before anything is written, as fontTools reads the whole record
        // first; their offsets count from the table holding the record.
        w.open(element, &attrs);
        for (name, offset) in devices {
            let mut sub = r.sub(offset);
            self.write_struct("Device", &mut sub, w, name, None, depth.saturating_add(1))?;
        }
        w.close(element);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn delta_value(&self, element: &str, start: i64, end: i64, format: i64, r: &mut Reader, w: &mut XmlWriter, index: Option<&str>) -> Result<()> {
        let items = end.saturating_sub(start).saturating_add(1);
        let bits: u32 = match format {
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let mask = (1i64 << bits) - 1;
        let sign_bit = 1i64 << (bits - 1);
        let mut values = Vec::new();
        let (mut word, mut shift) = (0i64, 0u32);
        for _ in 0..items.max(0) {
            if shift == 0 {
                word = i64::from(self.u16(r)?);
                shift = 16;
            }
            shift -= bits;
            let mut v = (word >> shift) & mask;
            if v & sign_bit != 0 {
                v -= 1 << bits;
            }
            values.push(v);
        }
        let text = py_list(&values);
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        if let Some(i) = index {
            attrs.push(("index", i));
        }
        attrs.push(("value", &text));
        w.simple(element, &attrs);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn var_data_value(&self, element: &str, regions: i64, shorts: i64, r: &mut Reader, w: &mut XmlWriter, index: Option<&str>) -> Result<()> {
        let long_words = shorts & 0x8000 != 0;
        let words = shorts & 0x7FFF;
        let (big, small) = if long_words { (4, 2) } else { (2, 1) };
        let (n1, n2) = (regions.min(words), regions.max(words));
        let mut values = self.ints(r, n1, big)?;
        values.extend(self.ints(r, n2.saturating_sub(n1), small)?);
        if n2 > regions {
            values.truncate(usize::try_from(regions).unwrap_or(0));
        }
        let text = py_list(&values);
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        if let Some(i) = index {
            attrs.push(("index", i));
        }
        attrs.push(("value", &text));
        w.simple(element, &attrs);
        Ok(())
    }

    // ── The summarised forms ───────────────────────────────────────────────────────────────────

    /// The glyph IDs of the coverage table at the reader, in the order fontTools lists them
    /// (ranges sorted by their coverage index). An unknown format lists nothing.
    fn coverage_ids(&self, r: &mut Reader) -> Result<Vec<u32>> {
        let format = self.u16(r)?;
        match format {
            1 => {
                let count = self.u16(r)?;
                self.uints(r, i64::from(count), 2)
            }
            2 => {
                let count = self.u16(r)?;
                let mut ranges = Vec::with_capacity(usize::from(count));
                for _ in 0..count {
                    let start = self.u16(r)?;
                    let end = self.u16(r)?;
                    let index = self.u16(r)?;
                    ranges.push((index, u32::from(start), u32::from(end)));
                }
                ranges.sort_by_key(|(index, _, _)| *index);
                let mut ids = Vec::new();
                for (_, start, end) in ranges {
                    if end >= start {
                        let len = usize::try_from(end - start + 1).unwrap_or(usize::MAX);
                        if ids.len().saturating_add(len) > MAX_LAYOUT_ITEMS {
                            return Err(TablesError::TooLarge { what: "the glyphs of a coverage table", limit: MAX_LAYOUT_ITEMS });
                        }
                        ids.extend(start..=end);
                    }
                }
                Ok(ids)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// The coverage at an offset from the reader's table; a null offset covers nothing.
    fn coverage_at(&self, r: &Reader, offset: u16) -> Result<Vec<u32>> {
        if offset == 0 {
            return Ok(Vec::new());
        }
        self.coverage_ids(&mut r.sub(u32::from(offset)))
    }

    fn coverage(&self, r: &mut Reader, w: &mut XmlWriter, element: &str, attrs: &[(&str, String)]) -> Result<()> {
        let ids = self.coverage_ids(r)?;
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.open(element, &attrs);
        for id in ids {
            w.simple("Glyph", &[("value", &self.name(id))]);
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    fn class_def(&self, r: &mut Reader, w: &mut XmlWriter, element: &str, attrs: &[(&str, String)]) -> Result<()> {
        let format = self.u16(r)?;
        let mut classes: BTreeMap<u32, u16> = BTreeMap::new();
        match format {
            1 => {
                let start = u32::from(self.u16(r)?);
                let count = self.u16(r)?;
                for (i, class) in self.uints(r, i64::from(count), 2)?.into_iter().enumerate() {
                    if class != 0 {
                        classes.insert(start.saturating_add(u32::try_from(i).unwrap_or(u32::MAX)), u16::try_from(class).unwrap_or(0));
                    }
                }
            }
            2 => {
                let count = self.u16(r)?;
                let mut work = 0usize;
                for _ in 0..count {
                    let start = u32::from(self.u16(r)?);
                    let end = u32::from(self.u16(r)?);
                    let class = self.u16(r)?;
                    if class == 0 || end < start {
                        continue;
                    }
                    work = work.saturating_add(usize::try_from(end - start + 1).unwrap_or(usize::MAX));
                    if work > MAX_LAYOUT_ITEMS {
                        return Err(TablesError::TooLarge { what: "the glyphs of a class definition table", limit: MAX_LAYOUT_ITEMS });
                    }
                    for id in start..=end {
                        classes.insert(id, class);
                    }
                }
            }
            _ => {}
        }
        let by_name: BTreeMap<String, u16> = classes.into_iter().map(|(id, class)| (self.name(id), class)).collect();
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.open(element, &attrs);
        for (name, class) in by_name {
            w.simple("ClassDef", &[("glyph", &name), ("class", &class.to_string())]);
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    /// Glyph IDs at an offset from `r`'s table, as a `GlyphCount`-prefixed array.
    fn glyph_list_at(&self, r: &Reader, offset: u16, what: &str) -> Result<Vec<u32>> {
        if offset == 0 {
            return Err(self.err(format!("a null offset to a {what}")));
        }
        let mut sub = r.sub(u32::from(offset));
        let count = self.u16(&mut sub)?;
        self.uints(&mut sub, i64::from(count), 2)
    }

    fn simple_subst(&self, class: &str, r: &mut Reader, w: &mut XmlWriter, element: &str, attrs: &[(&str, String)]) -> Result<()> {
        let format = self.u16(r)?;
        let table = r.clone();
        let coverage_offset = self.u16(r)?;
        let unknown = || self.err(format!("unknown {class} format {format}"));
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        match class {
            "SingleSubst" => {
                let mut mapping: BTreeMap<String, String> = BTreeMap::new();
                match format {
                    1 => {
                        let delta = u32::from(self.u16(r)?);
                        for id in self.coverage_at(&table, coverage_offset)? {
                            mapping.insert(self.name(id), self.name(id.wrapping_add(delta) % 65536));
                        }
                    }
                    2 => {
                        let count = self.u16(r)?;
                        let substitutes = self.uints(r, i64::from(count), 2)?;
                        let input = self.coverage_at(&table, coverage_offset)?;
                        if input.len() != usize::from(count) {
                            return Err(self.err("a single substitution whose coverage and substitutes differ in length"));
                        }
                        for (id, sub) in input.into_iter().zip(substitutes) {
                            mapping.insert(self.name(id), self.name(sub));
                        }
                    }
                    _ => return Err(unknown()),
                }
                w.open(element, &attrs);
                for (from, to) in mapping {
                    w.simple("Substitution", &[("in", &from), ("out", &to)]);
                    w.check()?;
                }
            }
            "MultipleSubst" => {
                if format != 1 {
                    return Err(unknown());
                }
                let count = self.u16(r)?;
                let offsets = self.offsets16(r, count)?;
                let mut sequences = Vec::with_capacity(offsets.len());
                for offset in offsets {
                    sequences.push(self.glyph_list_at(&table, offset, "sequence")?);
                    w.check()?;
                }
                let input = self.coverage_at(&table, coverage_offset)?;
                let mut mapping: BTreeMap<String, String> = BTreeMap::new();
                for (id, out) in input.into_iter().zip(sequences) {
                    let out: Vec<String> = out.into_iter().map(|g| self.name(g)).collect();
                    mapping.insert(self.name(id), out.join(","));
                }
                w.open(element, &attrs);
                for (from, to) in mapping {
                    w.simple("Substitution", &[("in", &from), ("out", &to)]);
                    w.check()?;
                }
            }
            "AlternateSubst" => {
                if format != 1 {
                    return Err(unknown());
                }
                let count = self.u16(r)?;
                let offsets = self.offsets16(r, count)?;
                let mut sets = Vec::with_capacity(offsets.len());
                for offset in offsets {
                    sets.push(self.glyph_list_at(&table, offset, "set of alternates")?);
                    w.check()?;
                }
                let input = self.coverage_at(&table, coverage_offset)?;
                if input.len() != sets.len() {
                    return Err(self.err("an alternate substitution whose coverage and sets differ in length"));
                }
                let mut mapping: BTreeMap<String, Vec<u32>> = BTreeMap::new();
                for (id, set) in input.into_iter().zip(sets) {
                    mapping.insert(self.name(id), set);
                }
                w.open(element, &attrs);
                for (glyph, set) in mapping {
                    w.open("AlternateSet", &[("glyph", &glyph)]);
                    for alt in set {
                        w.simple("Alternate", &[("glyph", &self.name(alt))]);
                    }
                    w.close("AlternateSet");
                    w.check()?;
                }
            }
            _ => {
                if format != 1 {
                    return Err(unknown());
                }
                let count = self.u16(r)?;
                let offsets = self.offsets16(r, count)?;
                let mut sets = Vec::with_capacity(offsets.len());
                for offset in offsets {
                    sets.push(self.ligature_set(&table, offset)?);
                    w.check()?;
                }
                let input = self.coverage_at(&table, coverage_offset)?;
                if input.len() != sets.len() {
                    return Err(self.err("a ligature substitution whose coverage and sets differ in length"));
                }
                let mut mapping: BTreeMap<String, Vec<(u32, Vec<u32>)>> = BTreeMap::new();
                for (id, set) in input.into_iter().zip(sets) {
                    mapping.insert(self.name(id), set);
                }
                w.open(element, &attrs);
                for (glyph, set) in mapping {
                    w.open("LigatureSet", &[("glyph", &glyph)]);
                    for (lig, components) in set {
                        let components: Vec<String> = components.into_iter().map(|g| self.name(g)).collect();
                        w.simple("Ligature", &[("components", &components.join(",")), ("glyph", &self.name(lig))]);
                    }
                    w.close("LigatureSet");
                    w.check()?;
                }
            }
        }
        w.close(element);
        Ok(())
    }

    /// The ligatures of a ligature set: each ligature glyph with its components after the first.
    fn ligature_set(&self, r: &Reader, offset: u16) -> Result<Vec<(u32, Vec<u32>)>> {
        if offset == 0 {
            return Err(self.err("a null offset to a ligature set"));
        }
        let mut set = r.sub(u32::from(offset));
        let table = set.clone();
        let count = self.u16(&mut set)?;
        let mut ligatures = Vec::new();
        for offset in self.offsets16(&mut set, count)? {
            if offset == 0 {
                return Err(self.err("a null offset to a ligature"));
            }
            let mut lig = table.sub(u32::from(offset));
            let glyph = u32::from(self.u16(&mut lig)?);
            let components = self.u16(&mut lig)?;
            ligatures.push((glyph, self.uints(&mut lig, i64::from(components) - 1, 2)?));
        }
        Ok(ligatures)
    }

    /// The entries of a delta-set index map: `outer << 16 | inner` each.
    fn map_entries(&self, r: &mut Reader, entry_format: i64, count: i64) -> Result<Vec<u64>> {
        if entry_format & 0xFFC0 != 0 {
            return Err(self.err(format!("a delta-set index map with entry format {entry_format:#x}")));
        }
        let inner_bits = 1 + (entry_format & 0x000F);
        let inner_mask = (1i64 << inner_bits) - 1;
        let size = usize::try_from(1 + ((entry_format & 0x0030) >> 4)).unwrap_or(1);
        let raw: Vec<u32> = if size == 3 {
            let mut v = Vec::new();
            for _ in 0..count.max(0) {
                v.push(self.u24(r)?);
            }
            v
        } else {
            self.uints(r, count, size)?
        };
        Ok(raw
            .into_iter()
            .map(|raw| {
                // Unbounded as in Python: a wide entry with few inner bits gives an outer index
                // past 16 bits, written as it is.
                let raw = u64::from(raw);
                let inner_mask = u64::try_from(inner_mask).unwrap_or(0);
                let outer = (raw & (0xFFFF_FFFF - inner_mask)) << (16 - inner_bits);
                outer | (raw & inner_mask)
            })
            .collect())
    }

    fn delta_set_index_map(&self, r: &mut Reader, w: &mut XmlWriter, element: &str, attrs: &[(&str, String)]) -> Result<()> {
        let format = self.u8(r)?;
        let (entry_format, count) = match format {
            0 => (i64::from(self.u8(r)?), i64::from(self.u16(r)?)),
            1 => (i64::from(self.u8(r)?), i64::from(self.u32(r)?)),
            _ => return Err(self.err(format!("unknown delta-set index map format {format}"))),
        };
        let mapping = self.map_entries(r, entry_format, count)?;
        let format_text = format.to_string();
        let mut attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        attrs.push(("Format", &format_text));
        w.open(element, &attrs);
        w.comment("Omitted values default to 0xFFFF/0xFFFF (no variations)");
        for (i, value) in mapping.into_iter().enumerate() {
            let index = i.to_string();
            if value == 0xFFFF_FFFF {
                w.simple("Map", &[("index", &index)]);
            } else {
                w.simple("Map", &[("index", &index), ("outer", &(value >> 16).to_string()), ("inner", &(value & 0xFFFF).to_string())]);
            }
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    fn var_idx_map(&self, r: &mut Reader, w: &mut XmlWriter, element: &str, attrs: &[(&str, String)]) -> Result<()> {
        let entry_format = i64::from(self.u16(r)?);
        let count = i64::from(self.u16(r)?);
        let mut mapping = self.map_entries(r, entry_format, count)?;
        let last = *mapping.last().ok_or_else(|| self.err("an empty glyph variation index map"))?;
        let glyphs = self.order.len();
        if mapping.len() < glyphs {
            mapping.resize(glyphs, last);
        }
        let by_name: BTreeMap<&str, u64> = self.order.iter().map(String::as_str).zip(mapping).collect();
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.open(element, &attrs);
        for (glyph, value) in by_name {
            w.simple("Map", &[("glyph", glyph), ("outer", &(value >> 16).to_string()), ("inner", &(value & 0xFFFF).to_string())]);
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    // ── The Apple tables ──────────────────────────────────────────────────────────────────────

    /// 64 bytes as text up to the first zero byte, other bytes than ASCII replaced (fontTools'
    /// `Char64`). Bytes past the table read as nothing; the reads after it fail.
    fn char64(&self, r: &mut Reader) -> String {
        let end = r.pos.saturating_add(64);
        let bytes = self.data.get(r.pos.min(self.data.len())..end.min(self.data.len())).unwrap_or_default();
        r.pos = end;
        let text = bytes.split(|b| *b == 0).next().unwrap_or_default();
        text.iter().map(|b| if b.is_ascii() { char::from(*b) } else { '\u{FFFD}' }).collect()
    }

    /// The entries of an AAT lookup table at the reader's position (fontTools' `AATLookup.read`):
    /// glyph id to 16-bit value, a later segment replacing an earlier one. Arrays of numbers
    /// (`table` false) are cut short where the data ends, as fontTools reads them; offsets are not.
    fn aat_entries(&self, r: &mut Reader, table: bool) -> Result<BTreeMap<u32, u16>> {
        let start = r.pos;
        let format = self.u16(r)?;
        let mut map = BTreeMap::new();
        let mut work = 0usize;
        let mut charge = |n: usize| -> Result<()> {
            work = work.saturating_add(n);
            if work > MAX_LAYOUT_ITEMS {
                return Err(TablesError::TooLarge { what: "the glyphs of an AAT lookup", limit: MAX_LAYOUT_ITEMS });
            }
            Ok(())
        };
        let values = |r: &mut Reader, count: usize| -> Result<Vec<u32>> {
            if table { (0..count).map(|_| self.u16(r).map(u32::from)).collect() } else { self.uints(r, i64::try_from(count).unwrap_or(i64::MAX), 2) }
        };
        match format {
            0 => {
                let count = self.order.len();
                charge(count)?;
                for (gid, v) in values(r, count)?.into_iter().enumerate() {
                    map.insert(u32::try_from(gid).unwrap_or(u32::MAX), v as u16);
                }
            }
            2 | 4 | 6 => {
                let unit = usize::from(self.u16(r)?);
                let units = self.u16(r)?;
                let least = if format == 6 { 4 } else { 6 };
                if unit < least {
                    return Err(self.err(format!("an AAT lookup's unit size {unit} is too small")));
                }
                for i in 0..usize::from(units) {
                    let mut seg = Reader { pos: start.saturating_add(i.saturating_mul(unit)).saturating_add(12), ..r.clone() };
                    if format == 6 {
                        let gid = self.u16(&mut seg)?;
                        let value = self.u16(&mut seg)?;
                        if gid != 0xFFFF {
                            map.insert(u32::from(gid), value);
                        }
                        continue;
                    }
                    let last = self.u16(&mut seg)?;
                    let first = self.u16(&mut seg)?;
                    let value = self.u16(&mut seg)?;
                    if last == 0xFFFF || last < first {
                        continue;
                    }
                    let count = usize::from(last - first) + 1;
                    charge(count)?;
                    if format == 2 {
                        for gid in u32::from(first)..=u32::from(last) {
                            map.insert(gid, value);
                        }
                    } else {
                        let mut array = Reader { pos: start.saturating_add(usize::from(value)), ..r.clone() };
                        for (k, v) in values(&mut array, count)?.into_iter().enumerate() {
                            map.insert(u32::from(first) + u32::try_from(k).unwrap_or(0), v as u16);
                        }
                    }
                }
            }
            8 => {
                let first = u32::from(self.u16(r)?);
                let count = usize::from(self.u16(r)?);
                charge(count)?;
                for (k, v) in values(r, count)?.into_iter().enumerate() {
                    map.insert(first + u32::try_from(k).unwrap_or(0), v as u16);
                }
            }
            _ => return Err(self.err(format!("unsupported AAT lookup format {format}"))),
        }
        Ok(map)
    }

    /// Glyph-to-value entries sorted by glyph name, as fontTools writes a lookup's dictionary.
    fn by_name(&self, map: BTreeMap<u32, u16>) -> Vec<(String, u16)> {
        let mut out: Vec<(String, u16)> = map.into_iter().map(|(gid, v)| (self.name(gid), v)).collect();
        out.sort();
        out.dedup_by(|b, a| a.0 == b.0);
        out
    }

    /// An AAT lookup table read in place (fontTools' `AATLookup.xmlWrite`): one `Lookup` per
    /// glyph, holding a number, or the table its value points at (offsets count from the start of
    /// the table that holds the lookup).
    fn aat_lookup(&self, element: &str, value: &'static str, r: &mut Reader, w: &mut XmlWriter, depth: usize) -> Result<()> {
        let table = value != "uint16";
        let entries = self.by_name(self.aat_entries(r, table)?);
        w.open(element, &[]);
        for (glyph, v) in entries {
            if !table {
                w.simple("Lookup", &[("glyph", &glyph), ("value", &v.to_string())]);
            } else if v == 0 {
                w.simple("Lookup", &[("glyph", &glyph), ("empty", "1")]);
            } else {
                let mut sub = r.sub(u32::from(v));
                self.write_struct_with(value, &mut sub, w, "Lookup", vec![("glyph", glyph)], depth.saturating_add(1))?;
            }
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    /// The anchor lookup of `ankr` (fontTools' `AATLookupWithDataOffset`): an offset to an AAT
    /// lookup of offsets, which count from the second offset.
    fn aat_lookup_data(&self, element: &str, table: &'static str, r: &mut Reader, w: &mut XmlWriter, depth: usize) -> Result<()> {
        let lookup = self.u32(r)?;
        let data = self.u32(r)?;
        let mut sub = r.sub(lookup);
        let entries = self.by_name(self.aat_entries(&mut sub, false)?);
        w.open(element, &[]);
        for (glyph, v) in entries {
            let offset = data.checked_add(u32::from(v)).ok_or_else(|| self.err("an anchor offset past 4 GiB"))?;
            let mut at = r.sub(offset);
            self.write_struct_with(table, &mut at, w, "Lookup", vec![("glyph", glyph)], depth.saturating_add(1))?;
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    /// The CID to glyph map of `cidg`: a count, then a glyph id per CID (0xFFFF for none).
    fn cid_glyph_map(&self, element: &str, r: &mut Reader, w: &mut XmlWriter) -> Result<()> {
        let count = self.u16(r)?;
        let glyphs = self.uints(r, i64::from(count), 2)?;
        w.open(element, &[]);
        for (cid, gid) in glyphs.into_iter().enumerate() {
            if gid != 0xFFFF {
                w.simple("CID", &[("cid", &cid.to_string()), ("glyph", &self.name(gid))]);
                w.check()?;
            }
        }
        w.close(element);
        Ok(())
    }

    /// The glyph to CID map of `gcid`: a count, then a CID per glyph (0xFFFF for none); entries
    /// past the font's glyphs are dropped. Written sorted by glyph name.
    fn glyph_cid_map(&self, element: &str, r: &mut Reader, w: &mut XmlWriter) -> Result<()> {
        let count = self.u16(r)?;
        let cids = self.uints(r, i64::from(count), 2)?;
        let mut entries: Vec<(&str, u32)> = self.order.iter().zip(cids).filter(|(_, cid)| *cid != 0xFFFF).map(|(g, cid)| (g.as_str(), cid)).collect();
        entries.sort();
        w.open(element, &[]);
        for (glyph, cid) in entries {
            w.simple("CID", &[("glyph", glyph), ("value", &cid.to_string())]);
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    /// A COLR clip list, written as fontTools' `ClipList.toXML` writes it: the glyphs that share a
    /// clip box grouped under one `Clip`, the groups in order of their first glyph name. A glyph
    /// that two records give keeps the first record's box; glyph ids past the font are dropped.
    fn clip_list(&self, r: &mut Reader, w: &mut XmlWriter, element: &str, attrs: &[(&str, String)], depth: usize) -> Result<()> {
        let format = self.u8(r)?;
        if format != 1 {
            return Err(self.err(format!("unknown clip list format {format}")));
        }
        let count = self.u32(r)?;
        let mut records = Vec::new();
        for _ in 0..count {
            let start = self.u16(r)?;
            let end = self.u16(r)?;
            let offset = self.u24(r)?;
            records.push((start, end, offset));
        }
        // Every box is read, used or not, as fontTools reads them; a null offset is no box.
        let mut keyed = Vec::with_capacity(records.len());
        for (start, end, offset) in records {
            let key = if offset == 0 { None } else { Some(self.clip_box_key(r, offset)?) };
            keyed.push((start, end, offset, key));
        }
        // Each group: the box's values (its key), where the first box is, and its glyphs.
        let mut groups: Vec<(Vec<i64>, u32, Vec<&str>)> = Vec::new();
        let mut group_of: BTreeMap<Vec<i64>, usize> = BTreeMap::new();
        let mut seen = vec![false; self.order.len()];
        let mut work = 0usize;
        for (start, end, offset, key) in keyed {
            if start > end {
                continue;
            }
            let last = usize::from(end).min(self.order.len().saturating_sub(1));
            for gid in usize::from(start)..=last {
                work = work.saturating_add(1);
                if work > MAX_LAYOUT_ITEMS {
                    return Err(TablesError::TooLarge { what: "the glyph ranges of a clip list", limit: MAX_LAYOUT_ITEMS });
                }
                let (Some(name), Some(done)) = (self.order.get(gid).map(String::as_str), seen.get_mut(gid)) else { continue };
                if *done {
                    continue;
                }
                *done = true;
                let Some(key) = key.clone() else { return Err(self.err("a clip record without a clip box")) };
                match group_of.get(&key).and_then(|i| groups.get_mut(*i)) {
                    Some(group) => group.2.push(name),
                    None => {
                        group_of.insert(key.clone(), groups.len());
                        groups.push((key, offset, vec![name]));
                    }
                }
            }
        }
        for group in &mut groups {
            group.2.sort_unstable();
        }
        groups.sort_by(|a, b| a.2.first().cmp(&b.2.first()));
        let format_text = format.to_string();
        let mut attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        attrs.push(("Format", &format_text));
        w.open(element, &attrs);
        for (_, offset, glyphs) in groups {
            w.open("Clip", &[]);
            for glyph in glyphs {
                w.simple("Glyph", &[("value", glyph)]);
            }
            let mut sub = r.sub(offset);
            self.write_struct("ClipBox", &mut sub, w, "ClipBox", None, depth.saturating_add(1))?;
            w.close("Clip");
            w.check()?;
        }
        w.close(element);
        Ok(())
    }

    /// What tells two clip boxes apart: the values of their format's fields, without the format.
    fn clip_box_key(&self, r: &Reader, offset: u32) -> Result<Vec<i64>> {
        let mut sub = r.sub(offset);
        let format = self.u8(&mut sub)?;
        let fields = match format {
            1 => 4,
            2 => 5,
            _ => return Ok(Vec::new()),
        };
        let mut key = Vec::with_capacity(fields);
        for _ in 0..4 {
            key.push(i64::from(self.u16(&mut sub)? as i16));
        }
        if fields == 5 {
            key.push(i64::from(self.u32(&mut sub)?));
        }
        Ok(key)
    }
}

/// The names of the Paint formats 1 to 32 (fontTools' `PaintFormat`).
const PAINT_FORMATS: [&str; 32] = [
    "PaintColrLayers",
    "PaintSolid",
    "PaintVarSolid",
    "PaintLinearGradient",
    "PaintVarLinearGradient",
    "PaintRadialGradient",
    "PaintVarRadialGradient",
    "PaintSweepGradient",
    "PaintVarSweepGradient",
    "PaintGlyph",
    "PaintColrGlyph",
    "PaintTransform",
    "PaintVarTransform",
    "PaintTranslate",
    "PaintVarTranslate",
    "PaintScale",
    "PaintVarScale",
    "PaintScaleAroundCenter",
    "PaintVarScaleAroundCenter",
    "PaintScaleUniform",
    "PaintVarScaleUniform",
    "PaintScaleUniformAroundCenter",
    "PaintVarScaleUniformAroundCenter",
    "PaintRotate",
    "PaintVarRotate",
    "PaintRotateAroundCenter",
    "PaintVarRotateAroundCenter",
    "PaintSkew",
    "PaintVarSkew",
    "PaintSkewAroundCenter",
    "PaintVarSkewAroundCenter",
    "PaintComposite",
];

/// The COLR composite modes, in the order of their numbers.
const COMPOSITE_MODES: [&str; 28] = [
    "clear",
    "src",
    "dest",
    "src_over",
    "dest_over",
    "src_in",
    "dest_in",
    "src_out",
    "dest_out",
    "src_atop",
    "dest_atop",
    "xor",
    "plus",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color_dodge",
    "color_burn",
    "hard_light",
    "soft_light",
    "difference",
    "exclusion",
    "multiply",
    "hsl_hue",
    "hsl_saturation",
    "hsl_color",
    "hsl_luminosity",
];

/// The name of value `v` of a COLR enumeration; `None` for a number it does not define.
fn enum_name(ty: Ty, v: i64) -> Option<&'static str> {
    let i = usize::try_from(v).ok()?;
    match ty {
        Ty::ExtendMode => ["pad", "repeat", "reflect"].get(i).copied(),
        Ty::CompositeMode => COMPOSITE_MODES.get(i).copied(),
        _ => None,
    }
}

/// An angle stored as F2Dot14 `raw`, as fontTools' `Angle.toString` writes it: `(raw / 16384 +
/// bias) * 180`, in the shortest decimal closer to that multiple of 180 / 16384 than to any other.
fn angle_text(raw: i64, bias: f64) -> String {
    let value = (raw as f64 / 16384.0 + bias) * 180.0;
    nearest_multiple_text(value, 180.0 / 16384.0)
}

/// fontTools' `nearestMultipleShortestRepr`: `value` rounded to a multiple of `factor`, in the
/// fewest decimals that still single out that multiple.
fn nearest_multiple_text(value: f64, factor: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return if value.is_finite() { "0.0".into() } else { py_float(value) };
    }
    let value = (value / factor + 0.5).floor() * factor;
    let eps = 0.5 * factor;
    let (lo, hi) = (value - eps, value + eps);
    if lo.trunc() != hi.trunc() {
        return py_float(value.round_ties_even());
    }
    let lo_s = format!("{lo:.8}");
    let hi_s = format!("{hi:.8}");
    if lo_s.len() != hi_s.len() || lo_s == hi_s {
        return py_float(value);
    }
    let first_diff = lo_s.bytes().zip(hi_s.bytes()).position(|(a, b)| a != b).unwrap_or(lo_s.len());
    let Some(period) = lo_s.find('.') else { return py_float(value) };
    if first_diff <= period {
        return py_float(value);
    }
    format!("{:.*}", first_diff.saturating_sub(period), value)
}

impl XmlWriter {
    /// `<name a="v" …><!-- comment -->` and one level deeper, as fontTools writes a Paint.
    pub(crate) fn open_with_comment(&mut self, name: &str, attributes: &[(&str, &str)], comment: &str) {
        let e = if self.spent() {
            super::super::Element::default()
        } else {
            let before = self.out.len();
            self.start(name, attributes);
            self.out.push('>');
            self.comment_body(comment);
            self.out.push('\n');
            self.charge_text(before);
            self.element(name, attributes)
        };
        self.depth = self.depth.saturating_add(1);
        self.open_elements.push(e);
    }
}

/// The subtable layout of lookup type `ty` in table `table`.
fn lookup_class(table: Tag, ty: i64) -> Option<&'static str> {
    const GSUB: [&str; 8] = [
        "SingleSubst",
        "MultipleSubst",
        "AlternateSubst",
        "LigatureSubst",
        "ContextSubst",
        "ChainContextSubst",
        "ExtensionSubst",
        "ReverseChainSingleSubst",
    ];
    const GPOS: [&str; 9] =
        ["SinglePos", "PairPos", "CursivePos", "MarkBasePos", "MarkLigPos", "MarkMarkPos", "ContextPos", "ChainContextPos", "ExtensionPos"];
    let i = usize::try_from(ty.checked_sub(1)?).ok()?;
    match &table {
        b"GSUB" => GSUB.get(i).copied(),
        b"GPOS" | b"JSTF" => GPOS.get(i).copied(),
        _ => None,
    }
}

/// The feature-parameter layout for a feature tag (`size`, `ss01`–`ss20`, `cv01`–`cv99`).
fn feature_params_class(tag: i64) -> &'static str {
    let tag = tag_text(tag);
    let number =
        |prefix: &str| tag.strip_prefix(prefix).filter(|d| d.len() == 2 && d.bytes().all(|b| b.is_ascii_digit())).and_then(|d| d.parse::<u8>().ok());
    if tag == "size" {
        "FeatureParamsSize"
    } else if number("ss").is_some_and(|n| (1..=20).contains(&n)) {
        "FeatureParamsStylisticSet"
    } else if number("cv").is_some_and(|n| (1..=99).contains(&n)) {
        "FeatureParamsCharacterVariants"
    } else {
        "FeatureParams"
    }
}

/// A tag stored as a big-endian number, as text (each byte one Latin-1 character).
pub(crate) fn tag_text(v: i64) -> String {
    let v = u32::try_from(v & 0xFFFF_FFFF).unwrap_or(0);
    v.to_be_bytes().iter().map(|b| char::from(*b)).collect()
}

/// A signed number from its `size` low bytes.
fn sign(v: u32, size: usize) -> i64 {
    match size {
        1 => i64::from(v as u8 as i8),
        2 => i64::from(v as u16 as i16),
        _ => i64::from(v as i32),
    }
}

/// Python's `"0x%08x" % v`.
fn version_text(v: i64) -> String {
    if v < 0 { format!("0x-{:07x}", v.unsigned_abs()) } else { format!("0x{v:08x}") }
}

/// A list of integers as Python writes it: `[1, -2]`.
fn py_list(values: &[i64]) -> String {
    let items: Vec<String> = values.iter().map(i64::to_string).collect();
    format!("[{}]", items.join(", "))
}

fn lookup_flag_comment(v: i64) -> Option<String> {
    let mut flags: Vec<String> = Vec::new();
    for (bit, name) in
        [(0x01, "rightToLeft"), (0x02, "ignoreBaseGlyphs"), (0x04, "ignoreLigatures"), (0x08, "ignoreMarks"), (0x10, "useMarkFilteringSet")]
    {
        if v & bit != 0 {
            flags.push(name.to_string());
        }
    }
    if v & 0xFF00 != 0 {
        flags.push(format!("markAttachmentType[{}]", v >> 8));
    }
    (!flags.is_empty()).then(|| flags.join(" "))
}

fn stat_flags_comment(v: i64) -> Option<String> {
    let mut flags = Vec::new();
    if v & 0x01 != 0 {
        flags.push("OlderSiblingFontAttribute");
    }
    if v & 0x02 != 0 {
        flags.push("ElidableAxisValueName");
    }
    (!flags.is_empty()).then(|| flags.join(" "))
}

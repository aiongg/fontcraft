//! WOFF 2.0: a 48-byte header, a compact table directory, and every table in one Brotli stream
//! (W3C WOFF File Format 2.0).
//!
//! Both table transforms are supported, in both directions: `glyf`/`loca` (version 0,
//! `woff2/glyf.rs`) and `hmtx` (version 1, `woff2/hmtx.rs`). Collections (`ttcf`) are written
//! and read with the collection directory of the specification (`woff2/collection.rs`).
//!
//! The writer is a port of fontTools' `WOFF2Writer` (`ttLib/woff2.py`, MIT): `DSIG` dropped, bit 11
//! of `head.flags` set, for the `glyf` transform `glyf` and `loca` laid out again with 4-byte glyph
//! padding (and `head.indexToLocFormat` set to match), tables sorted by tag, `checkSumAdjustment`
//! computed for the sfnt laid out in that order, the stream compressed at Brotli quality 11 with a
//! 22-bit window in font mode, the metadata in text mode. The known-tag table and the UIntBase128
//! and 255UInt16 encodings are ported from the same file.

pub(crate) mod collection;
pub(crate) mod glyf;
pub(crate) mod hmtx;

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::compress::{self, Mode, unbrotli};
use crate::sfnt::{self, DSIG, GLYF, HEAD, HMTX, LOCA, OTTO, Table, Tag};
use crate::{Error, Extras, MAX_METADATA, MAX_SFNT, MAX_TABLES, Unwrapped, pad4, put_u16, put_u32, to_u32, u16_at, u32_at};

const FORMAT: &str = "WOFF2";
const SIGNATURE: &[u8; 4] = b"wOF2";
const HEADER: usize = 48;

fn bad(what: impl Into<String>) -> Error {
    Error::invalid(FORMAT, what)
}

/// The tags a directory entry names by index (flags bits 0 to 5); index 63 means the tag follows.
pub const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm", b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT",
    b"EBLC", b"gasp", b"hdmx", b"kern", b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC", b"JSTF", b"MATH",
    b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar", b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar",
    b"gvar", b"hsty", b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat", b"Gloc", b"Feat", b"Sill",
];
/// The flags index that means "an arbitrary tag follows".
const UNKNOWN_TAG: u8 = 0x3f;

/// The index of `tag` in [`KNOWN_TAGS`], or 63.
pub fn known_tag_index(tag: Tag) -> u8 {
    KNOWN_TAGS.iter().position(|k| **k == tag).and_then(|i| u8::try_from(i).ok()).unwrap_or(UNKNOWN_TAG)
}

/// Appends `n` as a UIntBase128: big-endian groups of seven bits, the high bit set on all but
/// the last byte, no leading zero groups.
pub fn push_base128(out: &mut Vec<u8>, n: u32) {
    let mut size = 1;
    let mut rest = n >> 7;
    while rest > 0 {
        size += 1;
        rest >>= 7;
    }
    for i in (0..size).rev() {
        let group = ((n >> (7 * i)) & 0x7f) as u8;
        out.push(if i > 0 { group | 0x80 } else { group });
    }
}

/// Reads a UIntBase128 at `*at` and moves past it. Refuses a leading zero group, more than five
/// bytes and values above `u32::MAX`.
pub fn read_base128(data: &[u8], at: &mut usize) -> Result<u32, Error> {
    let mut value: u32 = 0;
    for i in 0..5 {
        let byte = *data.get(*at).ok_or_else(|| bad("a UIntBase128 number is cut short"))?;
        *at += 1;
        if i == 0 && byte == 0x80 {
            return Err(bad("a UIntBase128 number starts with a zero group"));
        }
        if value & 0xfe00_0000 != 0 {
            return Err(bad("a UIntBase128 number exceeds 32 bits"));
        }
        value = (value << 7) | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(bad("a UIntBase128 number is longer than five bytes"))
}

/// Appends `n` as a 255UInt16, choosing the encodings fontTools chooses.
pub fn push_255u16(out: &mut Vec<u8>, n: u16) {
    match n {
        0..253 => out.push(n as u8),
        253..506 => out.extend_from_slice(&[255, (n - 253) as u8]),
        506..762 => out.extend_from_slice(&[254, (n - 506) as u8]),
        _ => {
            out.push(253);
            out.extend_from_slice(&n.to_be_bytes());
        }
    }
}

/// Reads a 255UInt16 at `*at` and moves past it.
pub fn read_255u16(data: &[u8], at: &mut usize) -> Result<u16, Error> {
    let cut = || bad("a 255UInt16 number is cut short");
    let code = *data.get(*at).ok_or_else(cut)?;
    *at += 1;
    let mut next = || -> Result<u16, Error> {
        let b = *data.get(*at).ok_or_else(cut)?;
        *at += 1;
        Ok(u16::from(b))
    };
    Ok(match code {
        253 => {
            let hi = next()?;
            (hi << 8) | next()?
        }
        254 => next()? + 506,
        255 => next()? + 253,
        _ => u16::from(code),
    })
}

/// One entry of a WOFF2 table directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Woff2Table {
    pub tag: Tag,
    /// The transform version (flags bits 6 and 7). The null transform is 3 for `glyf` and `loca`
    /// and 0 for every other table.
    pub transform: u8,
    /// The table's size in the sfnt.
    pub orig_length: u32,
    /// The size of the transformed table in the stream, for a transformed table.
    pub transform_length: Option<u32>,
    /// Where the table starts in the decompressed stream.
    pub stream_offset: usize,
}

impl Woff2Table {
    /// True when the table is stored transformed.
    pub fn transformed(&self) -> bool {
        if self.tag == GLYF || self.tag == LOCA { self.transform != 3 } else { self.transform != 0 }
    }

    /// The table's size in the decompressed stream.
    pub fn stream_length(&self) -> u32 {
        self.transform_length.unwrap_or(self.orig_length)
    }
}

/// The header and table directory of a WOFF2 file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Woff2Directory {
    pub flavor: u32,
    pub total_sfnt_size: u32,
    pub version: (u16, u16),
    pub tables: Vec<Woff2Table>,
    /// Where the Brotli stream starts in the file, and its length.
    pub compressed: (usize, usize),
    /// Offset, compressed length and uncompressed length of the metadata block.
    pub meta: (u32, u32, u32),
    /// Offset and length of the private data block.
    pub private: (u32, u32),
    /// The collection directory, when the file holds a collection.
    pub collection: Option<Woff2Collection>,
}

/// The collection directory of a WOFF2 collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Woff2Collection {
    /// The collection header's version: `0x00010000` or `0x00020000`.
    pub version: u32,
    pub fonts: Vec<Woff2CollectionFont>,
}

/// One font of a WOFF2 collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Woff2CollectionFont {
    pub flavor: u32,
    /// Indices into [`Woff2Directory::tables`], one per table of the font.
    pub tables: Vec<usize>,
}

impl Woff2Directory {
    /// The size of the decompressed stream: the sum of the tables' stream lengths.
    pub fn stream_size(&self) -> usize {
        self.tables.iter().map(|t| t.stream_length() as usize).fold(0usize, usize::saturating_add)
    }
}

/// Reads and checks the header and table directory of a WOFF2 file: signature, length, reserved
/// field, table count, tags (each once per font), transform versions, the collection directory of
/// a collection, and the size limits.
pub fn directory(woff2: &[u8]) -> Result<Woff2Directory, Error> {
    if woff2.get(..4) != Some(SIGNATURE.as_slice()) {
        return Err(bad("not a WOFF2 file"));
    }
    let cut = || bad("the header is cut short");
    let w = |at| u32_at(woff2, at).ok_or_else(cut);
    let h = |at| u16_at(woff2, at).ok_or_else(cut);
    let flavor = w(4)?;
    let is_collection = flavor == sfnt::TTCF;
    if !is_collection && !sfnt::is_font_flavor(flavor) {
        return Err(bad("the font inside is neither TrueType nor OpenType"));
    }
    if w(8)? as usize != woff2.len() {
        return Err(bad("the file's length differs from the length in its header"));
    }
    let count = usize::from(h(12)?);
    // A collection lists each distinct table of all its fonts; the 16-bit field bounds it.
    if count == 0 || (!is_collection && count > MAX_TABLES) {
        return Err(bad("the table count is out of range"));
    }
    if h(14)? != 0 {
        return Err(bad("the reserved header field is not zero"));
    }
    let total_sfnt_size = w(16)?;
    if total_sfnt_size as usize > MAX_SFNT {
        return Err(bad("the font inside is too large"));
    }
    let compressed_length = w(20)? as usize;
    let version = (h(24)?, h(26)?);
    let meta = (w(28)?, w(32)?, w(36)?);
    let private = (w(40)?, w(44)?);
    let mut at = HEADER;
    let mut tables: Vec<Woff2Table> = Vec::with_capacity(count);
    let mut stream_offset = 0usize;
    let mut orig_total = 0usize;
    for _ in 0..count {
        let flags = *woff2.get(at).ok_or_else(|| bad("the table directory is cut short"))?;
        at += 1;
        let tag: Tag = if flags & 0x3f == UNKNOWN_TAG {
            let tag = woff2.get(at..at + 4).and_then(|t| Tag::try_from(t).ok()).ok_or_else(|| bad("the table directory is cut short"))?;
            at += 4;
            tag
        } else {
            **KNOWN_TAGS.get(usize::from(flags & 0x3f)).ok_or_else(|| bad("unknown table index"))?
        };
        let name = sfnt::tag_str(tag);
        if !is_collection && tables.iter().any(|t| t.tag == tag) {
            return Err(bad(format!("table '{name}' is listed twice")));
        }
        let mut table = Woff2Table { tag, transform: flags >> 6, orig_length: read_base128(woff2, &mut at)?, transform_length: None, stream_offset };
        if table.transformed() {
            table.transform_length = Some(read_base128(woff2, &mut at)?);
        }
        let known = matches!((&tag, table.transform), (b"glyf" | b"loca", 0 | 3) | (b"hmtx", 0 | 1) | (_, 0));
        if !known {
            return Err(bad(format!("table '{name}' uses an unknown transform")));
        }
        if tag == LOCA && table.transformed() && table.transform_length != Some(0) {
            return Err(bad("the transformed loca table must have length 0"));
        }
        stream_offset = stream_offset.saturating_add(table.stream_length() as usize);
        orig_total = orig_total.saturating_add(table.orig_length as usize);
        if stream_offset > MAX_SFNT || orig_total > MAX_SFNT {
            return Err(bad("the font inside is too large"));
        }
        tables.push(table);
    }
    let collection = if is_collection { Some(collection::read_directory(woff2, &mut at, &tables)?) } else { None };
    let fonts: Vec<Vec<usize>> = match &collection {
        Some(c) => c.fonts.iter().map(|f| f.tables.clone()).collect(),
        None => vec![(0..tables.len()).collect()],
    };
    for font in &fonts {
        let state = |tag: Tag| font.iter().filter_map(|&i| tables.get(i)).find(|t| t.tag == tag).map(Woff2Table::transformed);
        if matches!((state(GLYF), state(LOCA)), (Some(true), None | Some(false)) | (None | Some(false), Some(true))) {
            return Err(bad("glyf and loca must be transformed together"));
        }
    }
    let end = at.checked_add(compressed_length).ok_or_else(|| bad("the compressed data lies outside the file"))?;
    if end > woff2.len() {
        return Err(bad("the compressed data lies outside the file"));
    }
    Ok(Woff2Directory { flavor, total_sfnt_size, version, tables, compressed: (at, compressed_length), meta, private, collection })
}

/// The decompressed table stream of a WOFF2 file whose directory has been read.
pub fn stream(woff2: &[u8], dir: &Woff2Directory) -> Result<Vec<u8>, Error> {
    let (start, length) = dir.compressed;
    let data = start.checked_add(length).and_then(|end| woff2.get(start..end)).ok_or_else(|| bad("the compressed data lies outside the file"))?;
    unbrotli(FORMAT, data, dir.stream_size(), "the table data")
}

/// The metadata (decompressed) and the private data of a WOFF2 file, when it has them.
pub type Blocks = (Option<Vec<u8>>, Option<Vec<u8>>);

/// The metadata (decompressed) and the private data of a WOFF2 file whose directory has been read.
pub fn extras(woff2: &[u8], dir: &Woff2Directory) -> Result<Blocks, Error> {
    let data_end = pad4(dir.compressed.0.saturating_add(dir.compressed.1)).unwrap_or(usize::MAX);
    let (meta_offset, meta_length, meta_size) = (dir.meta.0 as usize, dir.meta.1 as usize, dir.meta.2 as usize);
    let mut meta_end = data_end;
    let metadata = if meta_length == 0 {
        None
    } else {
        if meta_size > MAX_METADATA {
            return Err(bad("the metadata is too large"));
        }
        if meta_offset < data_end {
            return Err(bad("the metadata is misplaced"));
        }
        meta_end = meta_offset.checked_add(meta_length).ok_or_else(|| bad("the metadata lies outside the file"))?;
        let raw = woff2.get(meta_offset..meta_end).ok_or_else(|| bad("the metadata lies outside the file"))?;
        Some(unbrotli(FORMAT, raw, meta_size, "the metadata")?)
    };
    let (private_offset, private_length) = (dir.private.0 as usize, dir.private.1 as usize);
    let private = if private_length == 0 {
        None
    } else {
        if private_offset % 4 != 0 || private_offset < meta_end {
            return Err(bad("the private data is misplaced"));
        }
        let end = private_offset.checked_add(private_length).ok_or_else(|| bad("the private data lies outside the file"))?;
        Some(woff2.get(private_offset..end).ok_or_else(|| bad("the private data lies outside the file"))?.to_vec())
    };
    Ok((metadata, private))
}

/// Unwraps a WOFF2 file: the sfnt with the tables in directory order (transformed ones rebuilt),
/// the header's version, the metadata (decompressed) and the private data. A WOFF2 collection
/// gives a collection file with each distinct table once, in directory order.
pub fn decode(woff2: &[u8]) -> Result<Unwrapped, Error> {
    let dir = directory(woff2)?;
    let data = stream(woff2, &dir)?;
    let (metadata, private) = extras(woff2, &dir)?;
    let sfnt = match &dir.collection {
        Some(c) => collection::decode(&dir, c, &data)?,
        None => {
            let owned = tables(&dir, &data)?;
            let list: Vec<Table<'_>> = owned.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
            sfnt::assemble(dir.flavor, &list)?
        }
    };
    Ok(Unwrapped { sfnt, version: dir.version, metadata, private })
}

/// A table's tag and data, borrowed from the table stream or rebuilt.
pub(crate) type OwnedTable<'a> = (Tag, Cow<'a, [u8]>);

/// The sfnt tables of a single-font WOFF2 file, in directory order, from its decompressed table
/// stream; transformed tables are rebuilt.
pub(crate) fn tables<'a>(dir: &Woff2Directory, data: &'a [u8]) -> Result<Vec<OwnedTable<'a>>, Error> {
    let built = build(dir, data, &[(0..dir.tables.len()).collect()])?;
    Ok(dir.tables.iter().map(|t| t.tag).zip(built).collect())
}

/// The sfnt data of every directory entry, from the decompressed table stream. `fonts` lists the
/// entries each font uses; a transformed table is rebuilt once, from the tables of the first font
/// that uses it, so the work grows with the output, not with the number of fonts.
pub(crate) fn build<'a>(dir: &Woff2Directory, data: &'a [u8], fonts: &[Vec<usize>]) -> Result<Vec<Cow<'a, [u8]>>, Error> {
    let raw = |t: &Woff2Table| {
        let end = t.stream_offset.saturating_add(t.stream_length() as usize);
        data.get(t.stream_offset..end).ok_or_else(|| bad("a table lies outside the table data"))
    };
    let entry = |font: &[usize], tag: Tag| font.iter().copied().find(|&i| dir.tables.get(i).is_some_and(|t| t.tag == tag));
    let mut out: Vec<Option<Cow<'a, [u8]>>> = Vec::with_capacity(dir.tables.len());
    for t in &dir.tables {
        out.push(if t.transformed() { None } else { Some(Cow::Borrowed(raw(t)?)) });
    }
    // Each rebuilt glyf entry's loca and glyph xMins, for the other entries that need them.
    let mut rebuilt: BTreeMap<usize, (Vec<u8>, Vec<i16>)> = BTreeMap::new();
    for font in fonts {
        let Some((g, gt)) = entry(font, GLYF).and_then(|g| Some((g, dir.tables.get(g)?))) else { continue };
        if !gt.transformed() {
            continue;
        }
        let pair = || bad("glyf and loca must be transformed together");
        let (l, lt) = entry(font, LOCA).and_then(|l| Some((l, dir.tables.get(l)?))).ok_or_else(pair)?;
        if !lt.transformed() {
            return Err(pair());
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = rebuilt.entry(g) {
            let r = glyf::reconstruct(raw(gt)?)?;
            if let Some(o) = out.get_mut(g) {
                *o = Some(Cow::Owned(r.glyf));
            }
            slot.insert((r.loca, r.x_mins));
        }
        if out.get(l).is_some_and(Option::is_none) {
            let loca = rebuilt.get(&g).map(|r| r.0.clone()).unwrap_or_default();
            if loca.len() != lt.orig_length as usize {
                return Err(bad("the rebuilt loca table differs in size from the original"));
            }
            if let Some(o) = out.get_mut(l) {
                *o = Some(Cow::Owned(loca));
            }
        }
    }
    for font in fonts {
        let Some((h, ht)) = entry(font, HMTX).and_then(|h| Some((h, dir.tables.get(h)?))) else { continue };
        if !ht.transformed() || out.get(h).is_some_and(Option::is_some) {
            continue;
        }
        let untransformed = |tag: Tag| -> Result<Option<&'a [u8]>, Error> {
            match entry(font, tag).and_then(|i| dir.tables.get(i)) {
                Some(t) if !t.transformed() => raw(t).map(Some),
                _ => Ok(None),
            }
        };
        let hhea = untransformed(HHEA)?.ok_or_else(|| bad("the hmtx transform needs an hhea table"))?;
        let metrics = u16_at(hhea, HHEA_METRICS_AT).ok_or_else(|| bad("the hhea table is too short"))?;
        let from_glyf = entry(font, GLYF).and_then(|g| rebuilt.get(&g)).map(|r| Cow::Borrowed(r.1.as_slice()));
        let x_mins = match from_glyf {
            Some(x) => x,
            None => {
                let need = |tag: Tag, name: &str| untransformed(tag)?.ok_or_else(|| bad(format!("the hmtx transform needs a {name} table")));
                let (glyf_data, loca, head, maxp) = (need(GLYF, "glyf")?, need(LOCA, "loca")?, need(HEAD, "head")?, need(MAXP, "maxp")?);
                let long = u16_at(head, HEAD_LOCA_FORMAT_AT).ok_or_else(|| bad("the head table is too short"))? != 0;
                let count = u16_at(maxp, MAXP_GLYPHS_AT).ok_or_else(|| bad("the maxp table is too short"))?;
                Cow::Owned(glyf::x_mins(glyf_data, loca, long, usize::from(count))?)
            }
        };
        let hmtx = hmtx::reconstruct(raw(ht)?, metrics, &x_mins)?;
        if hmtx.len() != ht.orig_length as usize {
            return Err(bad("the rebuilt hmtx table differs in size from the original"));
        }
        if let Some(o) = out.get_mut(h) {
            *o = Some(Cow::Owned(hmtx));
        }
    }
    // The tables' padded sizes, checked before they are put together.
    let mut total = 0usize;
    let mut done = Vec::with_capacity(out.len());
    for (t, bytes) in dir.tables.iter().zip(out) {
        let bytes = bytes.ok_or_else(|| bad(format!("table '{}' uses an unknown transform", sfnt::tag_str(t.tag))))?;
        total = pad4(bytes.len()).and_then(|n| total.checked_add(n)).unwrap_or(usize::MAX);
        if total > MAX_SFNT {
            return Err(bad("the font is too large"));
        }
        done.push(bytes);
    }
    Ok(done)
}

const HHEA: Tag = *b"hhea";
const MAXP: Tag = *b"maxp";
/// Where `numberOfHMetrics` sits in `hhea`.
const HHEA_METRICS_AT: usize = 34;
/// Where `indexToLocFormat` sits in `head`.
const HEAD_LOCA_FORMAT_AT: usize = 50;
/// Where `numGlyphs` sits in `maxp`.
const MAXP_GLYPHS_AT: usize = 4;

/// How [`encode`] stores the tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Apply the `glyf`/`loca` transform to TrueType outlines (on in fontTools' default).
    pub transform_glyf: bool,
    /// Apply the `hmtx` transform where it applies (off in fontTools' default). It needs `glyf`;
    /// it works with or without the `glyf` transform.
    pub transform_hmtx: bool,
    /// Header version, metadata and private data.
    pub extras: Extras,
}

impl Default for Options {
    /// fontTools' default: `glyf` and `loca` transformed, `hmtx` not.
    fn default() -> Self {
        Options { transform_glyf: true, transform_hmtx: false, extras: Extras::default() }
    }
}

impl Options {
    /// Every table stored with the null transform.
    pub fn untransformed() -> Self {
        Options { transform_glyf: false, transform_hmtx: false, extras: Extras::default() }
    }

    /// Both transforms, wherever they apply: the smallest files.
    pub fn all_transforms() -> Self {
        Options { transform_glyf: true, transform_hmtx: true, extras: Extras::default() }
    }
}

/// Bit 11 of `head.flags`: the font has been through a lossless transform.
const LOSSLESS_TRANSFORM: u16 = 1 << 11;
/// Where `flags` sits in `head`.
const HEAD_FLAGS_AT: usize = 16;

/// Wraps an sfnt font in WOFF2 with the transforms `options` asks for. A transform that does not
/// apply is skipped, as fontTools skips it: both on a CFF font; `glyf` when a glyph has cubic
/// points or `loca` does not fit `glyf`; `hmtx` when both side-bearing arrays would be needed, or
/// the table's size does not match `hhea`, `maxp` and `loca`. A damaged glyph also leaves `glyf`
/// untransformed, so wrapping never loses data.
pub fn encode(sfnt_bytes: &[u8], options: &Options) -> Result<Vec<u8>, Error> {
    if u32_at(sfnt_bytes, 0) == Some(sfnt::TTCF) {
        return collection::encode(sfnt_bytes, options);
    }
    let font = sfnt::parse(sfnt_bytes)?;
    let mut tables: Vec<(Tag, Cow<'_, [u8]>)> = font.tables.iter().filter(|t| t.tag != DSIG).map(|t| (t.tag, Cow::Borrowed(t.data))).collect();
    if tables.is_empty() {
        return Err(bad("the font has no tables"));
    }
    tables.sort_by_key(|t| t.0);
    let mut head = match font.get(HEAD) {
        Some(head) => {
            let flags = u16_at(head, HEAD_FLAGS_AT).ok_or_else(|| bad("the head table is too short"))?;
            let mut copy = head.to_vec();
            put_u16(&mut copy, HEAD_FLAGS_AT, flags | LOSSLESS_TRANSFORM);
            Some(copy)
        }
        None => None,
    };
    let truetype = font.flavor != OTTO;
    let loca_format = font.get(HEAD).and_then(|h| u16_at(h, HEAD_LOCA_FORMAT_AT));
    // The glyphs as the original `loca` cuts them; the transforms read them from here.
    let glyphs = match (font.get(GLYF), font.get(LOCA), loca_format) {
        (Some(g), Some(l), Some(format)) if truetype => glyf::glyph_slices(g, l, format != 0),
        _ => None,
    };
    let mut glyf_stream = None;
    if options.transform_glyf
        && let (Some(glyphs), Some(head)) = (&glyphs, &mut head)
        && let Some((glyf_data, loca, long)) = glyf::normalise(glyphs)
    {
        put_u16(head, HEAD_LOCA_FORMAT_AT, u16::from(long));
        glyf_stream = glyf::transform(glyphs, u16::from(long));
        for (tag, data) in &mut tables {
            if *tag == GLYF {
                *data = Cow::Owned(glyf_data.clone());
            } else if *tag == LOCA {
                *data = Cow::Owned(loca.clone());
            }
        }
    }
    let mut hmtx_stream = None;
    if options.transform_hmtx
        && let (Some(glyphs), Some(hmtx_data), Some(hhea), Some(maxp)) = (&glyphs, font.get(HMTX), font.get(HHEA), font.get(MAXP))
        && let (Some(metrics), Some(count)) = (u16_at(hhea, HHEA_METRICS_AT), u16_at(maxp, MAXP_GLYPHS_AT))
        && usize::from(count) == glyphs.len()
        && let Some(x_mins) = glyphs.iter().map(|g| glyf::x_min(g, glyf_stream.is_some())).collect::<Option<Vec<i16>>>()
    {
        hmtx_stream = hmtx::transform(hmtx_data, metrics, &x_mins);
    }
    if let (Some(h), Some(slot)) = (&head, tables.iter_mut().find(|t| t.0 == HEAD)) {
        slot.1 = Cow::Borrowed(h.as_slice());
    }
    let list: Vec<Table<'_>> = tables.iter().map(|(tag, data)| Table { tag: *tag, data }).collect();
    let adjustment = sfnt::checksum_adjustment(font.flavor, &list)?;
    let head = head.as_deref().map(|h| sfnt::with_adjustment(h, adjustment)).transpose()?;
    let mut total_sfnt = 12 + 16 * list.len();
    let mut stored = Vec::with_capacity(list.len());
    for t in &list {
        let data = match (&head, t.tag == HEAD) {
            (Some(h), true) => h.as_slice(),
            _ => t.data,
        };
        let transformed: Option<&[u8]> = match t.tag {
            GLYF => glyf_stream.as_deref(),
            LOCA => glyf_stream.as_ref().map(|_| &[][..]),
            HMTX => hmtx_stream.as_deref(),
            _ => None,
        };
        total_sfnt = total_sfnt.saturating_add(pad4(data.len()).unwrap_or(usize::MAX));
        let version = match (t.tag, transformed.is_some()) {
            (GLYF | LOCA, false) => 3,
            (HMTX, true) => 1,
            _ => 0,
        };
        stored.push(Stored { tag: t.tag, version, orig_length: data.len(), transformed: transformed.is_some(), data: transformed.unwrap_or(data) });
    }
    let version = options.extras.version.unwrap_or_else(|| crate::woff::revision(font.get(HEAD)));
    write_file(font.flavor, &stored, &[], total_sfnt, &options.extras, version)
}

/// One table as [`write_file`] stores it.
pub(crate) struct Stored<'a> {
    pub tag: Tag,
    /// The transform version written in the flags.
    pub version: u8,
    /// The table's size in the sfnt.
    pub orig_length: usize,
    /// Whether `data` is the transformed form, whose length the directory also records.
    pub transformed: bool,
    /// The bytes that go into the table stream.
    pub data: &'a [u8],
}

/// A WOFF2 file: the header, the table directory of `tables`, the `collection` directory (empty
/// for a single font), the compressed table stream, then the metadata and private blocks.
pub(crate) fn write_file(
    flavor: u32,
    tables: &[Stored<'_>],
    collection: &[u8],
    total_sfnt: usize,
    extras: &Extras,
    version: (u16, u16),
) -> Result<Vec<u8>, Error> {
    if total_sfnt > MAX_SFNT {
        return Err(bad("the font is too large"));
    }
    let mut stream = Vec::new();
    let mut directory = Vec::with_capacity(HEADER + 10 * tables.len() + collection.len());
    directory.resize(HEADER, 0);
    for t in tables {
        stream.extend_from_slice(t.data);
        let index = known_tag_index(t.tag);
        directory.push(index | (t.version << 6));
        if index == UNKNOWN_TAG {
            directory.extend_from_slice(&t.tag);
        }
        push_base128(&mut directory, to_u32(FORMAT, t.orig_length, "a table")?);
        if t.transformed {
            push_base128(&mut directory, to_u32(FORMAT, t.data.len(), "a table")?);
        }
    }
    directory.extend_from_slice(collection);
    let compressed = compress::brotli(&stream, Mode::Font)?;
    let mut out = directory;
    let compressed_length = compressed.len();
    out.extend_from_slice(&compressed);
    out.resize(pad4(out.len()).ok_or_else(|| bad("the file is too large"))?, 0);
    let (mut meta_offset, mut meta_length, mut meta_size) = (0, 0, 0);
    let private = extras.private.as_deref().filter(|p| !p.is_empty());
    if let Some(meta) = extras.metadata.as_deref().filter(|m| !m.is_empty()) {
        if meta.len() > MAX_METADATA {
            return Err(bad("the metadata is too large"));
        }
        let packed = compress::brotli(meta, Mode::Text)?;
        meta_offset = out.len();
        meta_length = packed.len();
        meta_size = meta.len();
        out.extend_from_slice(&packed);
    }
    let (mut private_offset, mut private_length) = (0, 0);
    if let Some(private) = private {
        out.resize(pad4(out.len()).ok_or_else(|| bad("the file is too large"))?, 0);
        private_offset = out.len();
        private_length = private.len();
        out.extend_from_slice(private);
    }
    let length = to_u32(FORMAT, out.len(), "the file")?;
    if let Some(sig) = out.get_mut(..4) {
        sig.copy_from_slice(SIGNATURE);
    }
    put_u32(&mut out, 4, flavor);
    put_u32(&mut out, 8, length);
    put_u16(&mut out, 12, u16::try_from(tables.len()).map_err(|_| bad("too many tables"))?);
    put_u32(&mut out, 16, to_u32(FORMAT, total_sfnt, "the font")?);
    put_u32(&mut out, 20, to_u32(FORMAT, compressed_length, "the compressed data")?);
    put_u16(&mut out, 24, version.0);
    put_u16(&mut out, 26, version.1);
    put_u32(&mut out, 28, to_u32(FORMAT, meta_offset, "the file")?);
    put_u32(&mut out, 32, to_u32(FORMAT, meta_length, "the metadata")?);
    put_u32(&mut out, 36, to_u32(FORMAT, meta_size, "the metadata")?);
    put_u32(&mut out, 40, to_u32(FORMAT, private_offset, "the file")?);
    put_u32(&mut out, 44, to_u32(FORMAT, private_length, "the private data")?);
    Ok(out)
}

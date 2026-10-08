//! WOFF 2.0: a 48-byte header, a compact table directory, and every table in one Brotli stream
//! (W3C WOFF File Format 2.0).
//!
//! This version writes and reads the null transform only: every table is stored as it is in the
//! sfnt. A file with transformed `glyf`/`loca` or `hmtx` tables is read up to its directory and
//! then refused with [`Error::Unsupported`]. Collections are refused.
//!
//! The writer is a port of fontTools' `WOFF2Writer` (`ttLib/woff2.py`, MIT): `DSIG` dropped, bit 11
//! of `head.flags` set, tables sorted by tag, `checkSumAdjustment` computed for the sfnt laid
//! out in that order, the stream compressed at Brotli quality 11 with a 22-bit window in font
//! mode, the metadata in text mode. The known-tag table and the UIntBase128 and 255UInt16
//! encodings are ported from the same file.

use crate::compress::{self, Mode, unbrotli};
use crate::sfnt::{self, DSIG, GLYF, HEAD, HMTX, LOCA, Table, Tag};
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
}

impl Woff2Directory {
    /// The size of the decompressed stream: the sum of the tables' stream lengths.
    pub fn stream_size(&self) -> usize {
        self.tables.iter().map(|t| t.stream_length() as usize).fold(0usize, usize::saturating_add)
    }
}

/// Reads and checks the header and table directory of a WOFF2 file: signature, length, reserved
/// field, table count, tags (each once), transform versions, and the size limits. Collections
/// are refused.
pub fn directory(woff2: &[u8]) -> Result<Woff2Directory, Error> {
    if woff2.get(..4) != Some(SIGNATURE.as_slice()) {
        return Err(bad("not a WOFF2 file"));
    }
    let cut = || bad("the header is cut short");
    let w = |at| u32_at(woff2, at).ok_or_else(cut);
    let h = |at| u16_at(woff2, at).ok_or_else(cut);
    let flavor = w(4)?;
    if flavor == sfnt::TTCF {
        return Err(Error::Unsupported { format: FORMAT, what: "a font collection" });
    }
    if !sfnt::is_font_flavor(flavor) {
        return Err(bad("the font inside is neither TrueType nor OpenType"));
    }
    if w(8)? as usize != woff2.len() {
        return Err(bad("the file's length differs from the length in its header"));
    }
    let count = usize::from(h(12)?);
    if count == 0 || count > MAX_TABLES {
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
        if tables.iter().any(|t| t.tag == tag) {
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
    let glyf = tables.iter().find(|t| t.tag == GLYF).map(Woff2Table::transformed);
    let loca = tables.iter().find(|t| t.tag == LOCA).map(Woff2Table::transformed);
    if matches!((glyf, loca), (Some(true), None | Some(false)) | (None | Some(false), Some(true))) {
        return Err(bad("glyf and loca must be transformed together"));
    }
    let end = at.checked_add(compressed_length).ok_or_else(|| bad("the compressed data lies outside the file"))?;
    if end > woff2.len() {
        return Err(bad("the compressed data lies outside the file"));
    }
    Ok(Woff2Directory { flavor, total_sfnt_size, version, tables, compressed: (at, compressed_length), meta, private })
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

/// Unwraps a WOFF2 file whose tables all use the null transform: the sfnt with the tables in
/// directory order, the header's version, the metadata (decompressed) and the private data.
pub fn decode(woff2: &[u8]) -> Result<Unwrapped, Error> {
    let dir = directory(woff2)?;
    if dir.tables.iter().any(Woff2Table::transformed) {
        return Err(Error::Unsupported { format: FORMAT, what: "reading transformed glyf, loca or hmtx tables" });
    }
    let data = stream(woff2, &dir)?;
    let (metadata, private) = extras(woff2, &dir)?;
    let mut tables = Vec::with_capacity(dir.tables.len());
    for t in &dir.tables {
        let end = t.stream_offset.saturating_add(t.orig_length as usize);
        let bytes = data.get(t.stream_offset..end).ok_or_else(|| bad("a table lies outside the table data"))?;
        tables.push(Table { tag: t.tag, data: bytes });
    }
    let sfnt = sfnt::assemble(dir.flavor, &tables)?;
    Ok(Unwrapped { sfnt, version: dir.version, metadata, private })
}

/// How [`encode`] stores the tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Apply the `glyf`/`loca` transform to TrueType outlines (fontTools' default).
    pub transform_glyf: bool,
    /// Apply the `hmtx` transform where it applies (off in fontTools' default).
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
}

/// Bit 11 of `head.flags`: the font has been through a lossless transform.
const LOSSLESS_TRANSFORM: u16 = 1 << 11;
/// Where `flags` sits in `head`.
const HEAD_FLAGS_AT: usize = 16;

/// Wraps an sfnt font in WOFF2. A TrueType font asked for the `glyf` or `hmtx` transform is
/// refused with [`Error::Unsupported`] in this version; for a CFF font the transforms do not
/// apply and are ignored.
pub fn encode(sfnt_bytes: &[u8], options: &Options) -> Result<Vec<u8>, Error> {
    let font = sfnt::parse(sfnt_bytes)?;
    if font.get(GLYF).is_some() && (options.transform_glyf || options.transform_hmtx && font.get(HMTX).is_some()) {
        return Err(Error::Unsupported { format: FORMAT, what: "writing transformed glyf, loca or hmtx tables" });
    }
    let mut tables: Vec<Table<'_>> = font.tables.iter().copied().filter(|t| t.tag != DSIG).collect();
    if tables.is_empty() {
        return Err(bad("the font has no tables"));
    }
    tables.sort_by_key(|t| t.tag);
    let flagged = match font.get(HEAD) {
        Some(head) => {
            let flags = u16_at(head, HEAD_FLAGS_AT).ok_or_else(|| bad("the head table is too short"))?;
            let mut copy = head.to_vec();
            put_u16(&mut copy, HEAD_FLAGS_AT, flags | LOSSLESS_TRANSFORM);
            Some(copy)
        }
        None => None,
    };
    if let (Some(h), Some(slot)) = (&flagged, tables.iter_mut().find(|t| t.tag == HEAD)) {
        slot.data = h;
    }
    let adjustment = sfnt::checksum_adjustment(font.flavor, &tables)?;
    let head = flagged.as_deref().map(|h| sfnt::with_adjustment(h, adjustment)).transpose()?;
    let mut total_sfnt = 12 + 16 * tables.len();
    let mut stream = Vec::new();
    let mut directory = Vec::with_capacity(HEADER + 10 * tables.len());
    directory.resize(HEADER, 0);
    for t in &tables {
        let data = match (&head, t.tag == HEAD) {
            (Some(h), true) => h.as_slice(),
            _ => t.data,
        };
        stream.extend_from_slice(data);
        total_sfnt = total_sfnt.saturating_add(pad4(data.len()).unwrap_or(usize::MAX));
        let index = known_tag_index(t.tag);
        let null = if t.tag == GLYF || t.tag == LOCA { 3 << 6 } else { 0 };
        directory.push(index | null);
        if index == UNKNOWN_TAG {
            directory.extend_from_slice(&t.tag);
        }
        push_base128(&mut directory, to_u32(FORMAT, data.len(), "a table")?);
    }
    if total_sfnt > MAX_SFNT {
        return Err(bad("the font is too large"));
    }
    let compressed = compress::brotli(&stream, Mode::Font)?;
    let mut out = directory;
    let compressed_length = compressed.len();
    out.extend_from_slice(&compressed);
    out.resize(pad4(out.len()).ok_or_else(|| bad("the file is too large"))?, 0);
    let (mut meta_offset, mut meta_length, mut meta_size) = (0, 0, 0);
    let private = options.extras.private.as_deref().filter(|p| !p.is_empty());
    if let Some(meta) = options.extras.metadata.as_deref().filter(|m| !m.is_empty()) {
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
    let version = options.extras.version.unwrap_or_else(|| crate::woff::revision(font.get(HEAD)));
    let length = to_u32(FORMAT, out.len(), "the file")?;
    if let Some(sig) = out.get_mut(..4) {
        sig.copy_from_slice(SIGNATURE);
    }
    put_u32(&mut out, 4, font.flavor);
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

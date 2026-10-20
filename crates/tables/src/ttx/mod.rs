//! TTX: the XML form of a font's tables, written and read as fontTools' `ttx` does (M12.4–M12.9;
//! rows `dump`, `font.dumpTables`, `tables import`, `tables.importTtx`, format `ttx`).
//!
//! [`dump`] writes the `GlyphOrder` pseudo-table and then the tables in the order `ttx` uses
//! (the recommended order of the OpenType specification, the rest sorted by tag, `DSIG` last),
//! handing each to the writer module that knows it. A table no module knows is written as hex
//! with `raw="True"`; a table a module fails to read is written as hex with
//! `ERROR="decompilation error"`, as `ttx` does, unless [`DumpOptions::strict`] is set.
//! [`import`] parses the text into an [`Element`] tree and hands each table element to the
//! reader module that knows it.
//!
//! | Module | Tables | Package |
//! |---|---|---|
//! | [`core`] | `head hhea maxp OS/2 name post cmap hmtx loca glyf gasp vhea vmtx fpgm prep cvt hdmx` | M12.4 |
//! | [`cff`] | `CFF ` `CFF2` `VORG` | M12.5 |
//! | [`otdata`] | GSUB, GPOS, GDEF, BASE, JSTF, MATH, STAT, fvar, avar, meta and other structured tables | M12.6 |
//! | [`var`], [`color`], [`misc`] | variations; COLR, CPAL, SVG, sbix and bitmaps; kern, DSIG, the Apple tables | M12.7 |
//! | [`read_core`], [`read_cff`] | reading the tables of `core` and `cff` | M12.8 |
//! | [`read_otdata`], [`read_var`], [`read_color`] | reading the rest | M12.9 |
//! | [`read_misc`], [`read_bitmap`] | reading `trak`, `ltag`, `sbix`, `EBSC` and the bitmap tables | M12.7c |
//!
//! Every writer module has `write_table(tag, data, font, w) -> Result<bool>` (false: not mine);
//! every reader module has `read_table(element, font) -> Result<Option<(Tag, Vec<u8>)>>`
//! (`None`: not mine). A writer that fails may leave partial output in `w`: [`dump`] gives each
//! table its own writer and keeps the output only when the module succeeds. Writers get glyph
//! names from [`XmlWriter::glyph_order`] and format numbers with [`fixed_to_str`],
//! [`py_float`], [`num2binary`] and [`py_hex`], so every module writes them as `ttx` does.
//!
//! A dump stops with an error past [`MAX_TTX_OUTPUT`]: every writer made from one dump charges
//! the same budget, and once it is spent the writer drops what it is given. Loops over counts
//! read from the font check [`XmlWriter::check`] as they go.
//!
//! The JSON form carries the same element tree (comments left out): each element is an object
//! `{"name", "attributes", "text", "children"}`, with empty members left out. The hex form lists
//! each table as offset, bytes and ASCII.
//!
//! Glyph names follow fontTools: the `CFF ` charset, else the `post` names (made unique with
//! `.1`, `.2`… and kept in `psNames`), else names made from the Unicode `cmap` subtables
//! (`uniXXXX`, the AGLFN names, `glyphNNNNN` for the rest). Parts of fontTools' `ttLib` (MIT)
//! are ported here and in [`core`]: the table order, the tag-to-element mapping, the number
//! formatting and the glyph-name rules.

pub mod cff;
pub mod color;
pub mod core;
pub mod misc;
#[cfg(test)]
mod oracle_digest;
pub mod otdata;
mod read;
pub(crate) mod read_bitmap;
pub mod read_cff;
pub mod read_color;
pub mod read_core;
pub(crate) mod read_misc;
pub mod read_otdata;
pub mod read_var;
pub mod var;

use std::cell::Cell;
use std::rc::Rc;

use crate::limits::MAX_TTX_OUTPUT;
use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::{Result, TablesError};

/// The `ttLibVersion` written on the root element: the version of fontTools whose TTX dialect
/// this writer follows.
pub const TTX_VERSION: &str = "4.66";

/// The form [`dump`] writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DumpFormat {
    /// TTX XML.
    #[default]
    Ttx,
    /// The same tree as JSON.
    Json,
    /// Every table as hex.
    Hex,
}

/// How the images of the bitmap data tables (`EBDT`, `CBDT`, `bdat`) are written, as `ttx -z`
/// chooses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BitmapForm {
    /// One block of hex per image (`rawimagedata`).
    #[default]
    Raw,
    /// One line of hex per pixel row (`rowimagedata`).
    Row,
    /// One line per pixel row, a character per pixel (`bitwiseimagedata`).
    Bitwise,
    /// Each image in a file of its own, named by `extfileimagedata` ([`dump_with_images`]).
    ExtFile,
}

/// A bitmap image written to a file of its own ([`BitmapForm::ExtFile`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageFile {
    /// The path the dump names, relative to the dump's folder: `bitmaps/strike<N>/<file>`.
    pub path: String,
    pub bytes: Vec<u8>,
}

/// What [`dump`] writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DumpOptions {
    pub format: DumpFormat,
    /// Tables to write, in this order; empty writes every table and the `GlyphOrder`
    /// pseudo-table.
    pub tables: Vec<Tag>,
    /// Tables to leave out.
    pub skip: Vec<Tag>,
    /// Tables written as hex even when a module knows them.
    pub hex: Vec<Tag>,
    /// Leave out the `ttLibVersion` attribute of the root, for stable comparisons.
    pub no_version: bool,
    /// Write TrueType programs (`fpgm`, `prep`, glyph instructions) as hex instead of assembly.
    pub no_disassemble: bool,
    /// Stop at the first table that cannot be read instead of writing it as hex.
    pub strict: bool,
    /// How bitmap images are written.
    pub bitmap_form: BitmapForm,
}

/// The font's tables as text, within [`MAX_TTX_OUTPUT`]. Images kept in files of their own
/// ([`BitmapForm::ExtFile`]) need [`dump_with_images`].
pub fn dump(font: &SfntFont, options: &DumpOptions) -> Result<String> {
    dump_within(font, options, MAX_TTX_OUTPUT)
}

/// [`dump`] with a budget of `limit` bytes.
pub(crate) fn dump_within(font: &SfntFont, options: &DumpOptions, limit: usize) -> Result<String> {
    no_image_files(options)?;
    Ok(dump_images_within(font, options, limit)?.0)
}

/// The font's tables as text, and the bitmap images [`BitmapForm::ExtFile`] keeps in files of
/// their own, which the caller writes beside the dump. The images count against the dump's
/// budget, [`MAX_TTX_OUTPUT`].
pub fn dump_with_images(font: &SfntFont, options: &DumpOptions) -> Result<(String, Vec<ImageFile>)> {
    dump_images_within(font, options, MAX_TTX_OUTPUT)
}

fn no_image_files(options: &DumpOptions) -> Result<()> {
    if options.bitmap_form == BitmapForm::ExtFile {
        return Err(TablesError::BadOptions("bitmap images in files of their own are written only with the dump's image files".into()));
    }
    Ok(())
}

fn dump_images_within(font: &SfntFont, options: &DumpOptions, limit: usize) -> Result<(String, Vec<ImageFile>)> {
    check_options(options)?;
    match options.format {
        DumpFormat::Hex => Ok((hex_dump(font, options), Vec::new())),
        DumpFormat::Ttx => {
            let mut w = writer_for(font, options);
            w.limit = limit;
            write_document(font, options, &mut w)?;
            w.check()?;
            let images = w.take_images();
            Ok((w.finish(), images))
        }
        DumpFormat::Json => {
            let mut w = writer_for(font, options);
            w.limit = limit;
            write_document(font, options, &mut w)?;
            w.check()?;
            let mut out = String::new();
            for element in w.tree() {
                element_json(&mut out, element, 0)?;
                out.push('\n');
            }
            Ok((out, w.take_images()))
        }
    }
}

/// The TTX dump split as `ttx -s` splits it: the main file, which names one file per table, then
/// one file per table. Each item is a file name and its text; the main file is `{stem}{ext}` and
/// a table's file is `{stem}.{identifier}{ext}` ([`tag_to_identifier`]), as `ttx` names them.
pub fn dump_split(font: &SfntFont, options: &DumpOptions, stem: &str, ext: &str) -> Result<Vec<(String, String)>> {
    no_image_files(options)?;
    Ok(split_dump(font, options, stem, ext, false)?.0)
}

/// [`dump_split`] with `glyf` split further, as `ttx -g` splits it: the `glyf` file names one
/// file per glyph that has an outline, `{stem}._g_l_y_f.{glyph}{ext}` with the glyph name made
/// safe for file systems as fontTools' `userNameToFileName` makes it.
pub fn dump_split_glyphs(font: &SfntFont, options: &DumpOptions, stem: &str, ext: &str) -> Result<Vec<(String, String)>> {
    no_image_files(options)?;
    Ok(split_dump(font, options, stem, ext, true)?.0)
}

/// [`dump_split`] (or, with `glyphs`, [`dump_split_glyphs`]) with the bitmap images
/// [`BitmapForm::ExtFile`] keeps in files of their own, as [`dump_with_images`] gives them.
pub fn dump_split_with_images(font: &SfntFont, options: &DumpOptions, stem: &str, ext: &str, glyphs: bool) -> Result<SplitDump> {
    split_dump(font, options, stem, ext, glyphs)
}

/// The files of a split dump (name, text), and the image files of its bitmap tables.
pub type SplitDump = (Vec<(String, String)>, Vec<ImageFile>);

fn split_dump(font: &SfntFont, options: &DumpOptions, stem: &str, ext: &str, glyphs: bool) -> Result<SplitDump> {
    check_options(options)?;
    if options.format != DumpFormat::Ttx {
        return Err(TablesError::BadOptions("only the TTX form can be split into one file per table".into()));
    }
    let mut main = writer_for(font, options);
    main.line(XML_DECLARATION);
    let root_attrs = root_attributes(font, options);
    let attrs: Vec<(&str, &str)> = root_attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    main.open("ttFont", &attrs);
    let mut files = Vec::new();
    let mut images = Vec::new();
    for entry in entries(font, options) {
        let ident = match entry {
            Entry::GlyphOrder => "GlyphOrder".to_string(),
            Entry::Table(tag) => tag_to_identifier(tag),
        };
        let name = format!("{stem}.{ident}{ext}");
        let element = match entry {
            Entry::GlyphOrder => "GlyphOrder".to_string(),
            Entry::Table(tag) => tag_to_xml(tag),
        };
        main.simple(&element, &[("src", &name)]);
        let mut part = main.child_at(0);
        part.set_depth(0);
        part.line(XML_DECLARATION);
        if options.no_version {
            part.open("ttFont", &[]);
        } else {
            part.open("ttFont", &[("ttLibVersion", TTX_VERSION)]);
        }
        part.blank();
        let mut glyph_files = Vec::new();
        let split_glyf = match entry {
            Entry::Table(tag) if glyphs && &tag == b"glyf" && !options.hex.contains(&tag) => font.get(tag),
            _ => None,
        };
        let mut written = false;
        if let Some(data) = split_glyf {
            let mut body = part.child_at(1);
            let charged = part.used.get();
            let prefix = format!("{stem}.{ident}.");
            let result = core::write_glyf_split(data, font, &mut body, &prefix, ext, options.no_version);
            body.check()?;
            if let Ok(found) = result {
                part.open(&element, &[]);
                part.append(body);
                part.close(&element);
                part.blank();
                glyph_files = found;
                written = true;
            } else {
                // Written whole instead, as hex when it cannot be read.
                part.used.set(charged);
            }
        }
        if !written {
            write_entry(font, options, &mut part, entry)?;
        }
        part.close("ttFont");
        part.check()?;
        images.extend(part.take_images());
        files.push((name, part.finish()));
        files.extend(glyph_files);
    }
    main.close("ttFont");
    main.check()?;
    let mut out = vec![(format!("{stem}{ext}"), main.finish())];
    out.extend(files);
    Ok((out, images))
}

fn check_options(options: &DumpOptions) -> Result<()> {
    if !options.tables.is_empty() && !options.skip.is_empty() {
        return Err(TablesError::BadOptions("give the tables to write or the tables to leave out, not both".into()));
    }
    Ok(())
}

fn writer_for(font: &SfntFont, options: &DumpOptions) -> XmlWriter {
    let mut w = XmlWriter::new();
    w.disassemble = !options.no_disassemble;
    w.bitmap_form = options.bitmap_form;
    w.tree = options.format == DumpFormat::Json;
    w.glyph_order = Some(glyph_order(font).map(Rc::new));
    w
}

pub(crate) const XML_DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

fn root_attributes(font: &SfntFont, options: &DumpOptions) -> Vec<(&'static str, String)> {
    // `repr()` of the four-byte string, without its quotes.
    let version = match font.flavour {
        Flavour::TrueType => r"\x00\x01\x00\x00",
        Flavour::Cff => "OTTO",
        Flavour::AppleTrue => "true",
    };
    let mut attrs = vec![("sfntVersion", version.to_string())];
    if !options.no_version {
        attrs.push(("ttLibVersion", TTX_VERSION.to_string()));
    }
    attrs
}

#[derive(Clone, Copy)]
enum Entry {
    GlyphOrder,
    Table(Tag),
}

/// What the dump holds, in order: the pseudo-table, then the tables as `ttx` orders them, or the
/// asked-for tables in the order given (`ttx -t` leaves the pseudo-table out).
fn entries(font: &SfntFont, options: &DumpOptions) -> Vec<Entry> {
    if !options.tables.is_empty() {
        let mut seen: Vec<Tag> = Vec::new();
        for tag in &options.tables {
            if font.tables.contains_key(tag) && !seen.contains(tag) {
                seen.push(*tag);
            }
        }
        return seen.into_iter().map(Entry::Table).collect();
    }
    let tags: Vec<Tag> = font.tables.keys().copied().filter(|t| !options.skip.contains(t)).collect();
    let mut out = vec![Entry::GlyphOrder];
    out.extend(sorted_tag_list(&tags).into_iter().map(Entry::Table));
    out
}

fn write_document(font: &SfntFont, options: &DumpOptions, w: &mut XmlWriter) -> Result<()> {
    w.line(XML_DECLARATION);
    let root_attrs = root_attributes(font, options);
    let attrs: Vec<(&str, &str)> = root_attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    w.open("ttFont", &attrs);
    w.blank();
    for entry in entries(font, options) {
        write_entry(font, options, w, entry)?;
        w.check()?;
    }
    w.close("ttFont");
    Ok(())
}

fn write_entry(font: &SfntFont, options: &DumpOptions, w: &mut XmlWriter, entry: Entry) -> Result<()> {
    match entry {
        Entry::GlyphOrder => {
            let order = w.glyph_order(font);
            w.open("GlyphOrder", &[]);
            w.comment("The 'id' attribute is only for humans; it is ignored when parsed.");
            match order {
                Ok(names) => {
                    for (i, name) in names.iter().enumerate() {
                        w.simple("GlyphID", &[("id", &i.to_string()), ("name", name)]);
                    }
                    w.check()?;
                }
                Err(e) if options.strict => return Err(e),
                Err(e) => w.comment(&format!("The glyph order could not be read: {e}")),
            }
            w.close("GlyphOrder");
            w.blank();
            Ok(())
        }
        Entry::Table(tag) => {
            let Some(data) = font.get(tag) else { return Ok(()) };
            write_table(font, options, w, tag, data)
        }
    }
}

fn write_table(font: &SfntFont, options: &DumpOptions, w: &mut XmlWriter, tag: Tag, data: &[u8]) -> Result<()> {
    let element = tag_to_xml(tag);
    if !options.hex.contains(&tag) {
        let mut body = w.child_at(1);
        let charged = w.used.get();
        let written = write_known(tag, data, font, &mut body);
        // A spent budget ends the dump; it is not a fault of the table.
        body.check()?;
        if !matches!(written, Ok(true)) {
            // The body is dropped: what it charged is given back.
            w.used.set(charged);
        }
        match written {
            Ok(true) => {
                w.open(&element, &[]);
                w.append(body);
                w.close(&element);
                w.blank();
                return Ok(());
            }
            Ok(false) => {}
            Err(e) if options.strict => {
                return Err(TablesError::Invalid(format!("table '{}': {e}", crate::sfnt::tag_str(tag))));
            }
            Err(e) => {
                w.open(&element, &[("ERROR", "decompilation error"), ("raw", "True")]);
                w.comment("An error occurred during the decompilation of this table");
                w.comment(&e.to_string());
                w.open("hexdata", &[]);
                w.hex(data);
                w.close("hexdata");
                w.close(&element);
                w.blank();
                return Ok(());
            }
        }
    }
    w.open(&element, &[("raw", "True")]);
    w.open("hexdata", &[]);
    w.hex(data);
    w.close("hexdata");
    w.close(&element);
    w.blank();
    Ok(())
}

fn write_known(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    type TableWriter = fn(Tag, &[u8], &SfntFont, &mut XmlWriter) -> Result<bool>;
    let writers: [TableWriter; 6] =
        [core::write_table, cff::write_table, otdata::write_table, var::write_table, color::write_table, misc::write_table];
    for write in writers {
        if write(tag, data, font, w)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// ── Hex form ───────────────────────────────────────────────────────────────────────────────────

fn hex_dump(font: &SfntFont, options: &DumpOptions) -> String {
    let mut out = String::new();
    for entry in entries(font, options) {
        let Entry::Table(tag) = entry else { continue };
        let Some(data) = font.get(tag) else { continue };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("'{}' {} bytes\n", crate::sfnt::tag_str(tag), data.len()));
        for (row, chunk) in data.chunks(16).enumerate() {
            out.push_str(&format!("{:08x} ", row.saturating_mul(16)));
            for i in 0..16 {
                if i == 8 {
                    out.push(' ');
                }
                match chunk.get(i) {
                    Some(b) => out.push_str(&format!(" {b:02x}")),
                    None => out.push_str("   "),
                }
            }
            out.push_str("  |");
            out.extend(chunk.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }));
            out.push_str("|\n");
        }
    }
    out
}

// ── JSON form ──────────────────────────────────────────────────────────────────────────────────

fn element_json(out: &mut String, e: &Element, depth: usize) -> Result<()> {
    if depth > crate::limits::MAX_TTX_DEPTH {
        return Err(TablesError::TooLarge { what: "the element nesting", limit: crate::limits::MAX_TTX_DEPTH });
    }
    out.push_str("{\"name\":");
    json_str(out, &e.name);
    if !e.attributes.is_empty() {
        out.push_str(",\"attributes\":{");
        for (i, (k, v)) in e.attributes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json_str(out, k);
            out.push(':');
            json_str(out, v);
        }
        out.push('}');
    }
    if !e.text.is_empty() {
        out.push_str(",\"text\":");
        json_str(out, &e.text);
    }
    if !e.children.is_empty() {
        out.push_str(",\"children\":[");
        for (i, c) in e.children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            element_json(out, c, depth.saturating_add(1))?;
        }
        out.push(']');
    }
    out.push('}');
    Ok(())
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ── Tags and table order ───────────────────────────────────────────────────────────────────────

/// The element name `ttx` gives a table: `OS_2` for `OS/2`, the tag without trailing spaces when
/// it is an identifier, [`tag_to_identifier`] otherwise.
pub fn tag_to_xml(tag: Tag) -> String {
    if &tag == b"OS/2" {
        return "OS_2".into();
    }
    let trimmed_len = tag.iter().rposition(|b| *b != b' ').map_or(0, |i| i.saturating_add(1));
    let body = tag.get(..trimmed_len).unwrap_or_default();
    let spaces_only_after = tag.get(trimmed_len..).unwrap_or_default().iter().all(|b| *b == b' ');
    let identifier = match body.split_first() {
        Some((first, rest)) => (first.is_ascii_alphabetic() || *first == b'_') && rest.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_'),
        None => false,
    };
    if identifier && spaces_only_after {
        return body.iter().map(|b| *b as char).collect();
    }
    tag_to_identifier(tag)
}

/// A tag as `ttx` writes it in file names and in elements that are not identifiers: lowercase
/// letters and digits get `_` before, uppercase letters `_` after, other bytes their hex value;
/// trailing spaces are dropped (`glyf` → `_g_l_y_f`, `OS/2` → `O_S_2f_2`).
pub fn tag_to_identifier(tag: Tag) -> String {
    let mut len = tag.len();
    while len > 1 && tag.get(len.saturating_sub(1)) == Some(&b' ') {
        len = len.saturating_sub(1);
    }
    let mut ident = String::new();
    for &b in tag.get(..len).unwrap_or_default() {
        if b.is_ascii_lowercase() || b.is_ascii_digit() {
            ident.push('_');
            ident.push(b as char);
        } else if b.is_ascii_uppercase() {
            ident.push(b as char);
            ident.push('_');
        } else {
            ident.push_str(&format!("{b:x}"));
        }
    }
    if ident.starts_with(|c: char| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    ident
}

const TTF_TABLE_ORDER: [&[u8; 4]; 19] = [
    b"head", b"hhea", b"maxp", b"OS/2", b"hmtx", b"LTSH", b"VDMX", b"hdmx", b"cmap", b"fpgm", b"prep", b"cvt ", b"loca", b"glyf", b"kern", b"name",
    b"post", b"gasp", b"PCLT",
];
const OTF_TABLE_ORDER: [&[u8; 4]; 8] = [b"head", b"hhea", b"maxp", b"OS/2", b"name", b"cmap", b"post", b"CFF "];

/// Tags in the order `ttx` writes them: sorted, `DSIG` last, then the recommended order of the
/// specification (the CFF one when there is a `CFF ` table) moved to the front.
pub fn sorted_tag_list(tags: &[Tag]) -> Vec<Tag> {
    let mut list: Vec<Tag> = tags.to_vec();
    list.sort();
    list.dedup();
    if let Some(i) = list.iter().position(|t| t == b"DSIG") {
        let dsig = list.remove(i);
        list.push(dsig);
    }
    let order: &[&[u8; 4]] = if list.iter().any(|t| t == b"CFF ") { &OTF_TABLE_ORDER } else { &TTF_TABLE_ORDER };
    let mut out = Vec::with_capacity(list.len());
    for tag in order {
        if let Some(i) = list.iter().position(|t| t == *tag) {
            out.push(list.remove(i));
        }
    }
    out.extend(list);
    out
}

// ── Numbers as fontTools writes them ───────────────────────────────────────────────────────────

/// A float as Python's `str()` writes it: the shortest text that reads back as the same value,
/// with `.0` on whole numbers and an exponent below 1e-4 and from 1e16 (`1e-05`, `1e+16`).
pub fn py_float(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let a = v.abs();
    if (1e-4..1e16).contains(&a) {
        let s = format!("{v}");
        if s.contains('.') { s } else { format!("{s}.0") }
    } else {
        let s = format!("{v:e}");
        let Some((mantissa, exp)) = s.split_once('e') else { return s };
        let (sign, digits) = match exp.strip_prefix('-') {
            Some(d) => ('-', d),
            None => ('+', exp),
        };
        let digits = if digits.len() < 2 { format!("0{digits}") } else { digits.to_string() };
        format!("{mantissa}e{sign}{digits}")
    }
}

/// A fixed-point number (`value` already divided by `1 << bits`) as fontTools writes it: the
/// shortest decimal that rounds back to the same fixed-point value (fontTools'
/// `floatToFixedToStr`, `nearestMultipleShortestRepr`).
pub fn fixed_to_str(value: f64, bits: u32) -> String {
    if value == 0.0 || !value.is_finite() || bits > 30 {
        return if value.is_finite() { "0.0".into() } else { py_float(value) };
    }
    let factor = 1.0 / f64::from(1u32 << bits);
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

/// Bits as fontTools' `num2binary` writes them: `bits` binary digits, most significant first, in
/// groups of eight counted from the right (`00000000 00001011`).
pub fn num2binary(value: u64, bits: u32) -> String {
    let mut groups: Vec<String> = Vec::new();
    let mut group = String::new();
    for i in 0..bits {
        let bit = if i < 64 { (value >> i) & 1 } else { 0 };
        group.insert(0, if bit == 1 { '1' } else { '0' });
        if (i + 1) % 8 == 0 {
            groups.push(std::mem::take(&mut group));
        }
    }
    if !group.is_empty() {
        groups.push(group);
    }
    groups.reverse();
    groups.join(" ")
}

/// An integer as Python's `hex()` writes it: `0x1a`, `-0x5`.
pub fn py_hex(value: i64) -> String {
    if value < 0 { format!("-0x{:x}", value.unsigned_abs()) } else { format!("0x{value:x}") }
}

// ── Glyph names ────────────────────────────────────────────────────────────────────────────────

/// The number of glyphs `maxp` gives.
pub fn num_glyphs(font: &SfntFont) -> Result<usize> {
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let bytes = maxp.get(4..6).ok_or_else(|| TablesError::invalid("the 'maxp' table is cut short"))?;
    Ok(usize::from(u16::from_be_bytes([bytes.first().copied().unwrap_or(0), bytes.get(1).copied().unwrap_or(0)])))
}

/// The glyph names fontTools gives the font's glyphs, in glyph order: the `CFF ` charset; else
/// the `post` names, when the table has one for every glyph; else names made from the Unicode
/// `cmap` subtables.
pub fn glyph_order(font: &SfntFont) -> Result<Vec<String>> {
    if let Some(cff) = font.get(*b"CFF ") {
        return cff_glyph_order(cff);
    }
    let count = num_glyphs(font)?;
    if let Some(post) = font.get(*b"post")
        && let Ok(names) = core::post_names(post, count)
        && let Some(order) = names.glyph_order
        && order.len() >= count
    {
        return Ok(order);
    }
    Ok(core::names_from_cmap(font, count))
}

/// The charset of a CFF table as fontTools reads it (`cid00001` names for CID-keyed fonts), made
/// unique with `.1`, `.2`….
fn cff_glyph_order(data: &[u8]) -> Result<Vec<String>> {
    let bad = || TablesError::invalid("the 'CFF ' table could not be read for glyph names");
    let header_size = usize::from(*data.get(2).ok_or_else(bad)?);
    let (_names, after_names) = cff_index(data, header_size)?;
    let (top_dicts, after_top) = cff_index(data, after_names)?;
    let (strings, _) = cff_index(data, after_top)?;
    let top = top_dicts.first().ok_or_else(bad)?;
    let dict = cff_dict(top)?;
    let charstrings = dict.iter().find(|(op, _)| *op == 17).and_then(|(_, args)| args.first()).ok_or_else(bad)?;
    let charstrings = usize::try_from(*charstrings).map_err(|_| bad())?;
    let (glyphs, _) = cff_index(data, charstrings)?;
    let count = glyphs.len();
    let is_cid = dict.iter().any(|(op, _)| *op == 1230);
    let charset = dict.iter().find(|(op, _)| *op == 15).and_then(|(_, args)| args.first()).copied().unwrap_or(0);
    let sid_name = |sid: usize| -> Result<String> {
        use read_fonts::ps::string::STANDARD_STRINGS;
        if let Some(s) = STANDARD_STRINGS.get(sid) {
            return Ok((*s).to_string());
        }
        let custom = strings.get(sid.saturating_sub(STANDARD_STRINGS.len())).ok_or_else(bad)?;
        Ok(custom.iter().map(|&b| b as char).collect())
    };
    let mut names: Vec<String> = Vec::with_capacity(count);
    match charset {
        0 => {
            // The ISOAdobe charset, cut to the glyph count as fontTools cuts it.
            for sid in 0..229.min(count) {
                names.push(sid_name(sid)?);
            }
            return Ok(names);
        }
        1 | 2 => return Err(TablesError::Unsupported("the predefined expert charsets of CFF".into())),
        _ => {}
    }
    let mut pos = usize::try_from(charset).map_err(|_| bad())?;
    let format = *data.get(pos).ok_or_else(bad)?;
    pos = pos.saturating_add(1);
    let name_of = |id: usize| -> Result<String> { if is_cid { Ok(format!("cid{id:05}")) } else { sid_name(id) } };
    names.push(".notdef".into());
    let read16 = |at: usize| -> Result<usize> {
        let b = data.get(at..at.saturating_add(2)).ok_or_else(bad)?;
        Ok(usize::from(b.first().copied().unwrap_or(0)) << 8 | usize::from(b.get(1).copied().unwrap_or(0)))
    };
    match format {
        0 => {
            for _ in 1..count {
                names.push(name_of(read16(pos)?)?);
                pos = pos.saturating_add(2);
            }
        }
        1 | 2 => {
            while names.len() < count {
                let first = read16(pos)?;
                let left = if format == 1 { usize::from(*data.get(pos.saturating_add(2)).ok_or_else(bad)?) } else { read16(pos.saturating_add(2))? };
                pos = pos.saturating_add(if format == 1 { 3 } else { 4 });
                if names.len().saturating_add(left) >= count {
                    return Err(bad());
                }
                for id in first..=first.saturating_add(left) {
                    names.push(name_of(id)?);
                }
            }
        }
        _ => return Err(bad()),
    }
    if names.len() != count {
        return Err(bad());
    }
    Ok(unique_names(names, ".", true))
}

/// fontTools' renaming of repeated names: the second `a` becomes `a.1` (or `a.2` when `a.1` is
/// taken). With `against_all`, a new name also avoids every name of the original list.
pub(crate) fn unique_names(names: Vec<String>, sep: &str, against_all: bool) -> Vec<String> {
    use std::collections::{HashMap, HashSet};
    let originals: HashSet<String> = if against_all { names.iter().cloned().collect() } else { HashSet::new() };
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let mut name = name;
        if let Some(&n) = seen.get(&name) {
            let mut n = n;
            while seen.contains_key(&format!("{name}{sep}{n}")) || originals.contains(&format!("{name}{sep}{n}")) {
                n = n.saturating_add(1);
            }
            seen.insert(name.clone(), n.saturating_add(1));
            name = format!("{name}{sep}{n}");
        }
        seen.insert(name.clone(), 1);
        out.push(name);
    }
    out
}

/// The items of the CFF INDEX at `pos` and the position after it.
fn cff_index(data: &[u8], pos: usize) -> Result<(Vec<&[u8]>, usize)> {
    let bad = || TablesError::invalid("a CFF INDEX is cut short");
    let count = usize::from(u16::from_be_bytes([*data.get(pos).ok_or_else(bad)?, *data.get(pos.saturating_add(1)).ok_or_else(bad)?]));
    if count == 0 {
        return Ok((Vec::new(), pos.saturating_add(2)));
    }
    let off_size = usize::from(*data.get(pos.saturating_add(2)).ok_or_else(bad)?);
    if !(1..=4).contains(&off_size) {
        return Err(TablesError::invalid("a CFF INDEX has a bad offset size"));
    }
    let offsets_at = pos.saturating_add(3);
    let read_offset = |i: usize| -> Result<usize> {
        let at = offsets_at.saturating_add(i.saturating_mul(off_size));
        let bytes = data.get(at..at.saturating_add(off_size)).ok_or_else(bad)?;
        Ok(bytes.iter().fold(0usize, |acc, b| acc << 8 | usize::from(*b)))
    };
    let base = offsets_at.saturating_add(count.saturating_add(1).saturating_mul(off_size)).saturating_sub(1);
    let mut items = Vec::with_capacity(count);
    let mut start = read_offset(0)?;
    for i in 1..=count {
        let end = read_offset(i)?;
        let item = data.get(base.saturating_add(start)..base.saturating_add(end)).ok_or_else(bad)?;
        items.push(item);
        start = end;
    }
    Ok((items, base.saturating_add(start)))
}

/// The operators of a CFF DICT with their integer operands; two-byte operators are `1200 + b1`.
/// Real operands are skipped (read as 0), as glyph names need none.
fn cff_dict(data: &[u8]) -> Result<Vec<(u16, Vec<i64>)>> {
    let bad = || TablesError::invalid("a CFF DICT is cut short");
    let mut out = Vec::new();
    let mut operands: Vec<i64> = Vec::new();
    let mut i = 0usize;
    while let Some(&b0) = data.get(i) {
        i = i.saturating_add(1);
        let byte = |at: usize| data.get(at).copied().map(i64::from).ok_or_else(bad);
        match b0 {
            0..=11 | 13..=21 => out.push((u16::from(b0), std::mem::take(&mut operands))),
            12 => {
                let b1 = data.get(i).copied().ok_or_else(bad)?;
                i = i.saturating_add(1);
                out.push((1200u16.saturating_add(u16::from(b1)), std::mem::take(&mut operands)));
            }
            28 => {
                let v = i16::from_be_bytes([data.get(i).copied().ok_or_else(bad)?, data.get(i.saturating_add(1)).copied().ok_or_else(bad)?]);
                operands.push(i64::from(v));
                i = i.saturating_add(2);
            }
            29 => {
                let b = data.get(i..i.saturating_add(4)).ok_or_else(bad)?;
                let v = b.iter().fold(0u32, |acc, x| acc << 8 | u32::from(*x)) as i32;
                operands.push(i64::from(v));
                i = i.saturating_add(4);
            }
            30 => {
                loop {
                    let nibbles = data.get(i).copied().ok_or_else(bad)?;
                    i = i.saturating_add(1);
                    if nibbles & 0x0F == 0x0F || nibbles >> 4 == 0x0F {
                        break;
                    }
                }
                operands.push(0);
            }
            32..=246 => operands.push(i64::from(b0) - 139),
            247..=250 => {
                operands.push((i64::from(b0) - 247) * 256 + byte(i)? + 108);
                i = i.saturating_add(1);
            }
            251..=254 => {
                operands.push(-(i64::from(b0) - 251) * 256 - byte(i)? - 108);
                i = i.saturating_add(1);
            }
            _ => out.push((u16::from(b0), std::mem::take(&mut operands))),
        }
        if operands.len() > 513 {
            return Err(TablesError::invalid("a CFF DICT has too many operands"));
        }
    }
    Ok(out)
}

// ── TTX text ───────────────────────────────────────────────────────────────────────────────────

/// How [`import`] merges the tables it reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Tables to take from the text; empty takes every table it holds.
    pub tables: Vec<Tag>,
    /// Keep the bounding boxes, `maxp` maxima and `hhea`/`vhea` extremes the text gives instead
    /// of recalculating them from the outlines (`ttx -b`).
    pub keep_bounds: bool,
}

/// `font` with the tables of the TTX `text` added or replaced. `font` may be empty
/// ([`SfntFont::new`]) to build a font from text alone.
///
/// Each table element is compiled as fontTools' `ttx` compiles it (`ttx -m FONT DUMP.ttx`, the
/// timestamp kept as the text gives it), names resolved against the text's `GlyphOrder` or, when
/// it has none, the font's own glyph order; then the fields fontTools' compiler fills from other
/// tables are filled (see the `read` module). A table element with `raw` or `ERROR`, or one no
/// reader knows, is taken from its `hexdata`. A failure changes nothing: the font is returned
/// only when every table was read.
pub fn import(font: SfntFont, text: &str, options: &ImportOptions) -> Result<SfntFont> {
    read::import(font, text, options)
}

/// [`import`] from a document already parsed (a split dump put together by the caller: every
/// element that names another file with `src` replaced by that file's elements).
pub fn import_tree(font: SfntFont, root: &Element, options: &ImportOptions) -> Result<SfntFont> {
    read::import_tree(font, root, options)
}

/// One XML element of a TTX document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// The text directly inside the element, whitespace kept.
    pub text: String,
}

impl Element {
    /// The value of one attribute.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The first child with this name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// Parses TTX text into its root element within [`crate::limits::MAX_TTX_BYTES`],
/// [`crate::limits::MAX_TTX_DEPTH`], [`crate::limits::MAX_TTX_ELEMENTS`] and
/// [`crate::limits::MAX_TTX_TREE_BYTES`].
pub fn parse(text: &str) -> Result<Element> {
    read::parse(text)
}

pub use read::ParseBudget;

/// [`parse`] charging `budget`, so that the files of a dump split into parts share the limits
/// of one text.
pub fn parse_within(text: &str, budget: &mut ParseBudget) -> Result<Element> {
    read::parse_within(text, budget)
}

/// Builds TTX text: indentation of two spaces per level, attributes in the order given, text and
/// attribute values escaped as fontTools escapes them. The element tree is built alongside
/// ([`XmlWriter::tree`]) for the JSON form; comments are not part of it.
#[derive(Clone, Debug)]
pub struct XmlWriter {
    out: String,
    depth: usize,
    /// Elements opened and not yet closed, outermost first.
    open_elements: Vec<Element>,
    /// Elements finished at the outermost level of this writer.
    done: Vec<Element>,
    disassemble: bool,
    glyph_order: Option<Result<Rc<Vec<String>>>>,
    /// Whether the element tree is built (the JSON form needs it, the TTX text does not).
    tree: bool,
    /// Bytes of text and tree written so far, shared with the writers made by [`Self::child_at`].
    used: Rc<Cell<usize>>,
    /// The budget for `used`.
    limit: usize,
    bitmap_form: BitmapForm,
    /// Bitmap images written to files of their own; a writer dropped unappended drops them.
    images: Vec<ImageFile>,
}

impl Default for XmlWriter {
    fn default() -> Self {
        Self {
            out: String::new(),
            depth: 0,
            open_elements: Vec::new(),
            done: Vec::new(),
            disassemble: true,
            glyph_order: None,
            tree: true,
            used: Rc::new(Cell::new(0)),
            limit: MAX_TTX_OUTPUT,
            bitmap_form: BitmapForm::Raw,
            images: Vec::new(),
        }
    }
}

impl XmlWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `<name a="v" …>` and one level deeper.
    pub fn open(&mut self, name: &str, attributes: &[(&str, &str)]) {
        let e = if self.spent() {
            Element::default()
        } else {
            let before = self.out.len();
            self.start(name, attributes);
            self.out.push_str(">\n");
            self.charge_text(before);
            self.element(name, attributes)
        };
        self.depth = self.depth.saturating_add(1);
        self.open_elements.push(e);
    }

    /// `</name>` one level up.
    pub fn close(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        let e = self.open_elements.pop();
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.indent();
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push_str(">\n");
        self.charge_text(before);
        if let Some(e) = e {
            self.attach(e);
        }
    }

    /// `<name a="v" …/>`.
    pub fn simple(&mut self, name: &str, attributes: &[(&str, &str)]) {
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.start(name, attributes);
        self.out.push_str("/>\n");
        self.charge_text(before);
        let e = self.element(name, attributes);
        self.attach(e);
    }

    /// `<name a="v" …/><!-- comment -->` on one line.
    pub fn simple_comment(&mut self, name: &str, attributes: &[(&str, &str)], comment: &str) {
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.start(name, attributes);
        self.out.push_str("/>");
        self.comment_body(comment);
        self.out.push('\n');
        self.charge_text(before);
        let e = self.element(name, attributes);
        self.attach(e);
    }

    /// `<name a="v" …/>  <!-- comment -->`: the comment two spaces after the element, as ttx
    /// writes it after name IDs and STAT flags.
    pub fn simple_spaced_comment(&mut self, name: &str, attributes: &[(&str, &str)], comment: &str) {
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.start(name, attributes);
        self.out.push_str("/>  ");
        self.comment_body(comment);
        self.out.push('\n');
        self.charge_text(before);
        let e = self.element(name, attributes);
        self.attach(e);
    }

    /// A line of text at the current depth.
    pub fn text(&mut self, text: &str) {
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.indent();
        escape_into(&mut self.out, text, false);
        self.out.push('\n');
        self.charge_text(before);
        self.add_text(text);
    }

    /// A line written as given, unescaped, at the current depth; `tree_text` is what the element
    /// tree holds for it.
    pub fn text_raw(&mut self, line: &str, tree_text: &str) {
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.indent();
        self.out.push_str(line);
        self.out.push('\n');
        self.charge_text(before);
        self.add_text(tree_text);
    }

    /// `<!-- text -->` at the current depth; the lines after the first are indented by five
    /// more spaces.
    pub fn comment(&mut self, text: &str) {
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.indent();
        self.comment_body(text);
        self.out.push('\n');
        self.charge_text(before);
    }

    /// An empty line.
    pub fn blank(&mut self) {
        if self.spent() {
            return;
        }
        self.out.push('\n');
        self.charge(1);
    }

    /// Bytes as hex, 16 per line in groups of four, as fontTools' `dumphex` writes them (a short
    /// last line keeps the spaces between its empty groups).
    pub fn hex(&mut self, data: &[u8]) {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        for chunk in data.chunks(16) {
            if self.spent() {
                return;
            }
            let mut hexline = String::with_capacity(32);
            for b in chunk {
                hexline.push(char::from(DIGITS.get(usize::from(b >> 4)).copied().unwrap_or(b'0')));
                hexline.push(char::from(DIGITS.get(usize::from(b & 0xF)).copied().unwrap_or(b'0')));
            }
            let mut line = String::with_capacity(36);
            for j in [0usize, 8, 16, 24] {
                if j > 0 {
                    line.push(' ');
                }
                line.push_str(hexline.get(j..hexline.len().min(j.saturating_add(8))).unwrap_or(""));
            }
            self.text(&line);
        }
    }

    /// Whether TrueType programs are written as assembly (true) or as hex.
    pub fn disassemble(&self) -> bool {
        self.disassemble
    }

    /// The font's glyph names ([`glyph_order`]), read once per dump.
    pub fn glyph_order(&mut self, font: &SfntFont) -> Result<Rc<Vec<String>>> {
        if let Some(order) = &self.glyph_order {
            return order.clone();
        }
        let order = glyph_order(font).map(Rc::new);
        self.glyph_order = Some(order.clone());
        order
    }

    /// An error once the dump has passed its budget ([`MAX_TTX_OUTPUT`]); the text is incomplete
    /// from then on.
    pub fn check(&self) -> Result<()> {
        if self.spent() {
            return Err(TablesError::TooLarge { what: "the text dump", limit: self.limit });
        }
        Ok(())
    }

    /// The text written so far.
    pub fn finish(self) -> String {
        self.out
    }

    /// The elements finished at the outermost level, in order.
    pub fn tree(&self) -> &[Element] {
        &self.done
    }

    /// Sets the indentation level of what is written next.
    pub(crate) fn set_depth(&mut self, depth: usize) {
        self.depth = depth;
    }

    /// An empty writer with the same settings, `levels` deeper than this one.
    pub(crate) fn child_at(&self, levels: usize) -> XmlWriter {
        XmlWriter {
            out: String::new(),
            depth: self.depth.saturating_add(levels),
            open_elements: Vec::new(),
            done: Vec::new(),
            disassemble: self.disassemble,
            glyph_order: self.glyph_order.clone(),
            tree: self.tree,
            used: Rc::clone(&self.used),
            limit: self.limit,
            bitmap_form: self.bitmap_form,
            images: Vec::new(),
        }
    }

    /// Keeps `bytes` as the image file `path`, charged to the budget.
    pub(crate) fn image_file(&mut self, path: String, bytes: Vec<u8>) {
        if self.spent() {
            return;
        }
        self.charge(bytes.len().saturating_add(path.len()));
        self.images.push(ImageFile { path, bytes });
    }

    /// The image files kept so far, taken out of the writer.
    pub(crate) fn take_images(&mut self) -> Vec<ImageFile> {
        std::mem::take(&mut self.images)
    }

    /// Appends what `other` wrote: its text, and its finished elements as children of the
    /// element open here. Both charge the same budget, so nothing is charged again.
    pub(crate) fn append(&mut self, other: XmlWriter) {
        if self.spent() {
            return;
        }
        self.out.push_str(&other.out);
        self.images.extend(other.images);
        if self.glyph_order.is_none() {
            self.glyph_order = other.glyph_order;
        }
        for e in other.done {
            self.attach(e);
        }
    }

    /// A line written as given, outside any element (the XML declaration).
    pub(crate) fn line(&mut self, line: &str) {
        if self.spent() {
            return;
        }
        let before = self.out.len();
        self.indent();
        self.out.push_str(line);
        self.out.push('\n');
        self.charge_text(before);
    }

    fn spent(&self) -> bool {
        self.used.get() > self.limit
    }

    fn charge(&self, bytes: usize) {
        self.used.set(self.used.get().saturating_add(bytes));
    }

    /// Charges what was written to `out` since it was `before` bytes long.
    fn charge_text(&self, before: usize) {
        self.charge(self.out.len().saturating_sub(before));
    }

    /// A tree element, charged at an estimate of its size; an empty one without a tree.
    fn element(&self, name: &str, attributes: &[(&str, &str)]) -> Element {
        if !self.tree {
            return Element::default();
        }
        let pair = 2 * std::mem::size_of::<String>();
        let size = attributes.iter().fold(std::mem::size_of::<Element>().saturating_add(name.len()), |sum, (k, v)| {
            sum.saturating_add(pair).saturating_add(k.len()).saturating_add(v.len())
        });
        self.charge(size);
        element(name, attributes)
    }

    fn attach(&mut self, e: Element) {
        if !self.tree {
            return;
        }
        match self.open_elements.last_mut() {
            Some(parent) => parent.children.push(e),
            None => self.done.push(e),
        }
    }

    fn add_text(&mut self, text: &str) {
        if !self.tree {
            return;
        }
        self.charge(text.len().saturating_add(1));
        if let Some(e) = self.open_elements.last_mut() {
            if !e.text.is_empty() {
                e.text.push('\n');
            }
            e.text.push_str(text);
        }
    }

    fn comment_body(&mut self, text: &str) {
        let mut escaped = String::with_capacity(text.len());
        escape_into(&mut escaped, text, false);
        self.out.push_str("<!-- ");
        for (i, line) in escaped.split('\n').enumerate() {
            if i > 0 {
                self.out.push('\n');
                self.indent();
                self.out.push_str("     ");
            }
            self.out.push_str(line);
        }
        self.out.push_str(" -->");
    }

    fn start(&mut self, name: &str, attributes: &[(&str, &str)]) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        for (key, value) in attributes {
            self.out.push(' ');
            self.out.push_str(key);
            self.out.push_str("=\"");
            escape_into(&mut self.out, value, true);
            self.out.push('"');
        }
    }

    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }
}

fn element(name: &str, attributes: &[(&str, &str)]) -> Element {
    Element {
        name: name.to_string(),
        attributes: attributes.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect(),
        children: Vec::new(),
        text: String::new(),
    }
}

/// fontTools' `escape` (and `escapeattr` for attribute values): `& < >` and carriage returns as
/// entities, characters XML 1.0 does not allow as `?`.
fn escape_into(out: &mut String, text: &str, attribute: bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            '"' if attribute => out.push_str("&quot;"),
            c if is_illegal_xml_char(c) => out.push('?'),
            _ => out.push(c),
        }
    }
}

fn is_illegal_xml_char(c: char) -> bool {
    let n = c as u32;
    matches!(n, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0xFFFE | 0xFFFF)
}

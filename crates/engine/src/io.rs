//! Documents on disk: recognising a source, reading it, writing it back (native builds only; the
//! web app passes bytes instead).
//!
//! A source is recognised by its content, never by its file name: a folder with `metainfo.plist`
//! is a UFO and one with `fontinfo.plist` a `.glyphspackage`, a file that starts with an sfnt,
//! collection, WOFF or WOFF2 signature is a compiled font, a JSON file whose `format` is
//! `fontcraft` is a native file, XML whose root is `<designspace>` a designspace (opened with the
//! UFOs it names, relative to its folder), a ZIP archive a UFOZ, a bitmap font file (BDF, PCF,
//! Windows FNT or FON, TeX PK or GF, a Mac bitmap font, or a `.pcf.gz` or `.bdf.gz`) a
//! bitmap-only font (module `bitmap_font`), and other text that starts with `{` is a `.glyphs`
//! file. Every read is bounded ([`MAX_PACKAGE_FILES`],
//! [`fontcraft_format::MAX_BYTES`] in all) and follows no symlinked folders, so a hostile folder
//! can neither exhaust memory nor loop.

use fontcraft_io_ufo::ufo::FilesFrom;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use fontcraft_model::{Family, Font, Shape, SourceFormat};
use serde_json::{Value, json};

use crate::{EngineError, Result};

mod bitmap_font;
pub use bitmap_font::UNITS_PER_EM as BITMAP_FONT_UNITS_PER_EM;

/// Most files read from one package.
pub const MAX_PACKAGE_FILES: usize = 200_000;

/// The formats a document can be saved in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFormat {
    Native,
    Glyphs,
    GlyphsPackage,
    Ufo,
    /// A UFO in a ZIP archive, under one folder named after the file.
    Ufoz,
    Designspace,
}

impl SaveFormat {
    pub fn name(self) -> &'static str {
        match self {
            SaveFormat::Native => "fontcraft",
            SaveFormat::Glyphs => "glyphs",
            SaveFormat::GlyphsPackage => "glyphspackage",
            SaveFormat::Ufo => "ufo",
            SaveFormat::Ufoz => "ufoz",
            SaveFormat::Designspace => "designspace",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fontcraft" => Some(SaveFormat::Native),
            "glyphs" => Some(SaveFormat::Glyphs),
            "glyphspackage" => Some(SaveFormat::GlyphsPackage),
            "ufo" => Some(SaveFormat::Ufo),
            "ufoz" => Some(SaveFormat::Ufoz),
            "designspace" => Some(SaveFormat::Designspace),
            _ => None,
        }
    }

    /// The format a path's extension names, in any case: a name typed as `Font.UFO` is a UFO.
    pub fn of_path(path: &Path) -> Option<Self> {
        Self::parse(&path.extension()?.to_str()?.to_ascii_lowercase())
    }

    /// The format a document read from `source` is saved in by default.
    pub fn of_source(source: SourceFormat) -> Option<Self> {
        match source {
            SourceFormat::Native => Some(SaveFormat::Native),
            SourceFormat::Glyphs => Some(SaveFormat::Glyphs),
            SourceFormat::GlyphsPackage => Some(SaveFormat::GlyphsPackage),
            SourceFormat::Ufo => Some(SaveFormat::Ufo),
            SourceFormat::Designspace => Some(SaveFormat::Designspace),
            SourceFormat::Ufoz => Some(SaveFormat::Ufoz),
            SourceFormat::Binary => None,
        }
    }

    /// Whether the format writes UFO files: a UFO folder, a UFOZ, or a designspace's UFOs.
    pub fn is_ufo(self) -> bool {
        matches!(self, SaveFormat::Ufo | SaveFormat::Ufoz | SaveFormat::Designspace)
    }

    /// The model's source format for a document saved in this format.
    pub fn source(self) -> SourceFormat {
        match self {
            SaveFormat::Native => SourceFormat::Native,
            SaveFormat::Glyphs => SourceFormat::Glyphs,
            SaveFormat::GlyphsPackage => SourceFormat::GlyphsPackage,
            SaveFormat::Ufo => SourceFormat::Ufo,
            SaveFormat::Ufoz => SourceFormat::Ufoz,
            SaveFormat::Designspace => SourceFormat::Designspace,
        }
    }

    /// The format families whose kept data this format writes back.
    fn families(self) -> &'static [Family] {
        match self {
            SaveFormat::Native => &[Family::Native],
            SaveFormat::Glyphs | SaveFormat::GlyphsPackage => &[Family::Glyphs],
            SaveFormat::Ufo | SaveFormat::Ufoz => &[Family::Ufo],
            SaveFormat::Designspace => &[Family::Ufo, Family::Designspace],
        }
    }
}

/// What a file or folder looked like when it was read or written, to notice that something else
/// changed it since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    files: usize,
    bytes: u64,
    modified: Option<SystemTime>,
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> EngineError {
    EngineError::Io(format!("cannot {what} {}: {e}", path.display()))
}

impl Stamp {
    fn add(&mut self, other: Stamp) {
        self.files += other.files;
        self.bytes += other.bytes;
        self.modified = self.modified.max(other.modified);
    }
}

/// The stamp of a document's files: the file or folder at `path`, and for a designspace also
/// every UFO it names.
pub fn document_stamp(path: &Path, font: &Font) -> Option<Stamp> {
    let mut s = stamp(path)?;
    if font.source.format == SourceFormat::Designspace {
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut seen = Vec::new();
        for m in font.masters.iter() {
            if let Some(f) = m.extra.get(Family::Designspace, "filename").and_then(fontcraft_model::plist::Value::as_str)
                && !seen.contains(&f)
            {
                seen.push(f);
                if let Some(u) = stamp(&dir.join(f)) {
                    s.add(u);
                }
            }
        }
    }
    Some(s)
}

/// The stamp of a file, or of every file in a folder; `None` when there is nothing there.
pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() {
        return Some(Stamp { files: 1, bytes: meta.len(), modified: meta.modified().ok() });
    }
    let mut s = Stamp { files: 0, bytes: 0, modified: None };
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else { continue };
            if m.is_dir() {
                stack.push(entry.path());
            } else if m.is_file() {
                s.files += 1;
                s.bytes += m.len();
                s.modified = s.modified.max(m.modified().ok());
            }
            if s.files > MAX_PACKAGE_FILES {
                return Some(s);
            }
        }
    }
    Some(s)
}

/// A regular file's bytes, at most [`fontcraft_format::MAX_BYTES`].
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| io_err("open", path, e))?;
    if !meta.is_file() {
        return Err(EngineError::Io(format!("cannot open {}: it is not a regular file", path.display())));
    }
    let limit = fontcraft_format::MAX_BYTES as u64;
    let mut bytes = Vec::new();
    // At most one byte past the limit, whatever the file's size claims to be.
    std::fs::File::open(path).map_err(|e| io_err("open", path, e))?.take(limit + 1).read_to_end(&mut bytes).map_err(|e| io_err("read", path, e))?;
    if bytes.len() as u64 > limit {
        return Err(fontcraft_format::FormatError::TooLarge(bytes.len()).into());
    }
    Ok(bytes)
}

/// Every regular file in a folder, by path relative to it with `/`. Symlinks are not followed.
pub fn read_folder(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut total = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| io_err("read the folder", &dir, e))? {
            let entry = entry.map_err(|e| io_err("read the folder", &dir, e))?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|e| io_err("read", &path, e))?;
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            if out.len() >= MAX_PACKAGE_FILES {
                return Err(EngineError::Io(format!("{} holds more than {MAX_PACKAGE_FILES} files", root.display())));
            }
            let rel = path.strip_prefix(root).map_err(|e| EngineError::Internal(e.to_string()))?;
            let rel = rel.to_str().ok_or_else(|| EngineError::Io(format!("{}: a file name is not UTF-8", path.display())))?.replace('\\', "/");
            let bytes = read_file(&path)?;
            total += bytes.len();
            if total > fontcraft_format::MAX_BYTES {
                return Err(fontcraft_format::FormatError::TooLarge(total).into());
            }
            out.insert(rel, bytes);
        }
    }
    Ok(out)
}

/// The folder the feature code of the source at `path` reads its `include` files from, as the
/// reference compilers resolve them: the folder that holds the UFO (for a designspace, its first
/// UFO), else the folder the source file is in. `None` when that cannot be told.
pub fn feature_dir(path: &Path) -> Option<std::path::PathBuf> {
    if path.join("metainfo.plist").is_file() {
        return path.parent().map(Path::to_path_buf);
    }
    let is_designspace = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("designspace"));
    if is_designspace {
        let bytes = read_file(path).ok()?;
        let text = std::str::from_utf8(&bytes).ok()?;
        let first = fontcraft_io_ufo::designspace::ufo_paths(text).ok()?.into_iter().next()?;
        return path.parent()?.join(first).parent().map(Path::to_path_buf);
    }
    path.parent().map(Path::to_path_buf)
}

/// Reads the source at `path`, recognising its format by content, with what reading it left out
/// or read in a plainer form. `face` chooses a member of a font collection.
pub fn open(path: &Path, face: Option<u32>) -> Result<(Font, Vec<String>)> {
    let meta = std::fs::metadata(path).map_err(|e| io_err("open", path, e))?;
    if meta.is_dir() {
        if path.join("metainfo.plist").is_file() {
            let files = read_folder(path)?;
            return Ok(fontcraft_io_ufo::ufo::read_reporting(&files)?);
        }
        if path.join("fontinfo.plist").is_file() {
            let files = read_folder(path)?;
            return Ok((fontcraft_io_glyphs::read_package(&files)?, Vec::new()));
        }
        if path.join("font.props").is_file() {
            let files = read_folder(path)?;
            return sfd_source(path, fontcraft_io_sfd::read_sfdir(&files));
        }
        return Err(EngineError::Io(format!("{} is a folder but not a font source", path.display())));
    }
    let bytes = read_file(path)?;
    if fontcraft_io_ufo::zip::is_zip(&bytes) {
        let files = fontcraft_io_ufo::zip::read_ufoz(&bytes)?;
        let (mut font, warnings) = fontcraft_io_ufo::ufo::read_reporting(&files)?;
        font.source.format = SourceFormat::Ufoz;
        return Ok((font, warnings));
    }
    if bitmap_font::is_gzipped_bitmap_font(path, &bytes) {
        return bitmap_font::open(path, &bytes);
    }
    if fontcraft_format::is_compressed(&bytes) {
        return Ok((fontcraft_format::load(&bytes)?, Vec::new()));
    }
    if fontcraft_decompile::sniff(&bytes).is_some() {
        // A WOFF or WOFF2 is unwrapped once, for the decompiler and for the stored tables. One
        // that cannot be unwrapped goes to the decompiler as it is, which says why.
        let sfnt = fontcraft_decompile::to_sfnt(&bytes).ok();
        let mut d = fontcraft_decompile::decompile(sfnt.as_deref().unwrap_or(&bytes), face)?;
        if let Some(sfnt) = &sfnt {
            keep_stored_tables(&mut d.font, sfnt, face);
        }
        return Ok((d.font, d.warnings));
    }
    if bitmap_font::is_bitmap_font(&bytes) {
        return bitmap_font::open(path, &bytes);
    }
    if fontcraft_io_sfd::is_sfd(&bytes) {
        return sfd_source(path, fontcraft_io_sfd::read_sfd(&bytes));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| EngineError::Io(format!("{} is not a font source: it is not UTF-8 text", path.display())))?;
    let body = text.trim_start_matches('\u{feff}').trim_start();
    if body.starts_with('<') {
        if fontcraft_io_svg::svg_font::is_svg_font(body) {
            let svg = fontcraft_io_svg::svg_font::read_svg_font(&bytes)
                .map_err(|e| EngineError::Io(format!("cannot read the SVG font {}: {e}", path.display())))?;
            return Ok((svg.font, svg.warnings));
        }
        if !is_designspace(body) {
            return Err(EngineError::Io(format!("{} is XML but neither a designspace nor an SVG font", path.display())));
        }
        return open_designspace(path, text);
    }
    if !body.starts_with('{') {
        return Err(EngineError::Io(format!("{} is not a font source this build can read", path.display())));
    }
    if is_native(body) {
        return Ok((fontcraft_format::load(&bytes)?, Vec::new()));
    }
    Ok((fontcraft_io_glyphs::read(text)?, Vec::new()))
}

/// Keeps in `font` the tables of the sfnt `sfnt` (a font or a collection) that a compile can copy
/// back (`fontcraft_compile::extras::STORED_TABLES`, for `font.compile`'s `passThroughTables`),
/// with the glyph order they index: the opened font's, one glyph per glyph id. A file whose
/// table directory cannot be read keeps nothing; the decompiler has read it already.
fn keep_stored_tables(font: &mut Font, sfnt: &[u8], face: Option<u32>) {
    let Ok(file) = skrifa::raw::FileRef::new(sfnt) else { return };
    let face = match file {
        skrifa::raw::FileRef::Font(f) => f,
        skrifa::raw::FileRef::Collection(c) => match face.and_then(|i| c.get(i).ok()) {
            Some(f) => f,
            None => return,
        },
    };
    let tables: Vec<_> =
        face.table_directory().table_records().iter().filter_map(|r| face.table_data(r.tag()).map(|d| (r.tag(), d.as_bytes()))).collect();
    let order: Vec<String> = font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
    fontcraft_compile::extras::store_tables(font, &tables, &order);
}

/// An SFD file or folder as read, with what the reader skipped as warnings. There is no SFD
/// writer, so the document opens view-only as a compiled font does: Save cannot replace the
/// source with another format, and Save As chooses one.
fn sfd_source(path: &Path, read: std::result::Result<fontcraft_io_sfd::Import, fontcraft_io_sfd::SfdError>) -> Result<(Font, Vec<String>)> {
    let import = read.map_err(|e| EngineError::Io(format!("cannot read the SFD source {}: {e}", path.display())))?;
    let mut font = import.font;
    font.source.format = SourceFormat::Binary;
    let warnings = import.skipped.iter().map(|s| format!("skipped {} ({}×)", s.what, s.count)).collect();
    Ok((font, warnings))
}

/// Whether XML text has `<designspace` as its root element: the first element after the
/// declaration, comments and document type.
fn is_designspace(body: &str) -> bool {
    let mut rest = body;
    loop {
        rest = rest.trim_start();
        let skip = if rest.starts_with("<?") {
            rest.find("?>").map(|i| i + 2)
        } else if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with("<!") {
            rest.find('>').map(|i| i + 1)
        } else {
            return rest.strip_prefix("<designspace").is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/'));
        };
        match skip.and_then(|i| rest.get(i..)) {
            Some(r) => rest = r,
            None => return false,
        }
    }
}

/// A designspace and the UFOs it names, read relative to its folder, within the same bounds as
/// one package, with the reading's warnings. While automation roots are in force, a UFO outside
/// the read root is refused.
fn open_designspace(path: &Path, text: &str) -> Result<(Font, Vec<String>)> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut ufos = BTreeMap::new();
    let mut total = text.len();
    for rel in fontcraft_io_ufo::designspace::ufo_paths(text)? {
        let ufo_dir = dir.join(&rel);
        crate::file_access::check_read(&ufo_dir).map_err(|e| EngineError::Io(format!("{}: the source `{rel}`: {e}", path.display())))?;
        if !ufo_dir.join("metainfo.plist").is_file() {
            return Err(EngineError::Io(format!("{}: the source `{rel}` is not a UFO folder", path.display())));
        }
        let files = read_folder(&ufo_dir)?;
        total += files.values().map(Vec::len).sum::<usize>();
        if total > fontcraft_format::MAX_BYTES {
            return Err(fontcraft_format::FormatError::TooLarge(total).into());
        }
        ufos.insert(rel, files);
    }
    Ok(fontcraft_io_ufo::designspace::read_reporting(text, &ufos)?)
}

/// Most files [`check_references`] passes to its callback for one source.
pub const MAX_REFERENCES: usize = 1_024;

/// Passes every file that opening the source at `path` reads besides `path` itself to `admit`,
/// each before any of them is read: the UFOs a designspace names, then the files of the font's
/// Import Font and Import Master parameters, and the UFOs of a linked designspace. Stops at the
/// first refusal. A file that does not open as a source refers to nothing. The automation roots
/// ([`crate::file_access`]) call it on every path a command reads.
pub fn check_references(path: &Path, admit: &mut dyn FnMut(&Path) -> std::result::Result<(), String>) -> std::result::Result<(), String> {
    let mut left = MAX_REFERENCES;
    follow_references(path, true, admit, &mut left)
}

fn follow_references(
    path: &Path,
    links: bool,
    admit: &mut dyn FnMut(&Path) -> std::result::Result<(), String>,
    left: &mut usize,
) -> std::result::Result<(), String> {
    let pass = |p: &Path, admit: &mut dyn FnMut(&Path) -> std::result::Result<(), String>, left: &mut usize| {
        *left = left.checked_sub(1).ok_or_else(|| format!("{} refers to more than {MAX_REFERENCES} files", path.display()))?;
        admit(p)
    };
    let Ok(meta) = std::fs::metadata(path) else { return Ok(()) };
    if !meta.is_dir() {
        // A named pipe or a device refers to nothing, and opening one can wait for a writer
        // forever.
        if !meta.is_file() {
            return Ok(());
        }
        // A compiled font refers to nothing; a few bytes tell, where opening it would decompile it.
        let mut head = Vec::new();
        let Ok(file) = std::fs::File::open(path) else { return Ok(()) };
        if file.take(64).read_to_end(&mut head).is_err() || fontcraft_decompile::sniff(&head).is_some() || is_image(&head) {
            return Ok(());
        }
        let bytes = read_file(path).unwrap_or_default();
        let text = std::str::from_utf8(&bytes).unwrap_or_default();
        let body = text.trim_start_matches('\u{feff}').trim_start();
        if body.starts_with('<') && is_designspace(body) {
            let dir = path.parent().unwrap_or(Path::new("."));
            for rel in fontcraft_io_ufo::designspace::ufo_paths(text).unwrap_or_default() {
                pass(&dir.join(rel), &mut *admit, left)?;
            }
        }
    }
    if !links {
        return Ok(());
    }
    let Ok((font, _)) = open(path, None) else { return Ok(()) };
    for linked in crate::linked::linked_paths(&font, Some(path)) {
        pass(&linked, &mut *admit, left)?;
        // A linked file's own links are not followed (`linked`), but a linked designspace reads
        // its UFOs.
        follow_references(&linked, false, &mut *admit, left)?;
    }
    Ok(())
}

/// Whether `head`, a file's first bytes, starts like a bitmap or PDF file, which refers to no
/// other file.
fn is_image(head: &[u8]) -> bool {
    const MAGIC: [&[u8]; 8] = [b"\x89PNG\r\n\x1a\n", b"\xFF\xD8\xFF", b"II*\0", b"MM\0*", b"%PDF-", b"8BPS", b"GIF87a", b"GIF89a"];
    MAGIC.iter().any(|m| head.starts_with(m))
}

/// Whether text that starts with `{` is a native file: JSON whose first key is a quoted string
/// followed by `:` and which names the `fontcraft` format near the start.
fn is_native(body: &str) -> bool {
    let after = body.trim_start_matches('{').trim_start();
    let json_key = after.starts_with('"')
        && after.split_once('"').and_then(|(_, rest)| rest.split_once('"')).is_some_and(|(_, rest)| rest.trim_start().starts_with(':'));
    json_key && body.get(..body.len().min(4096)).unwrap_or(body).contains("\"fontcraft\"")
}

/// What saving could not keep: extra entries and kept shapes of other format families. The UFO
/// and designspace readers' entries are records of the files as read; only the data some of them
/// hold is reported ([`fontcraft_io_ufo::ufo::kept_data_losses`]), not the file layout, and
/// neither are the layouts the .glyphs reader keeps ([`fontcraft_io_glyphs::LAYOUT_KEY`]).
fn losses(font: &Font, targets: &[Family]) -> Vec<Value> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let record = |f: Family| matches!(f, Family::Ufo | Family::Designspace);
    let mut count = |x: &fontcraft_model::Extra| {
        let layout = |e: &fontcraft_model::ExtraEntry| e.key.as_str().starts_with(fontcraft_io_glyphs::LAYOUT_KEY);
        for e in x.iter().filter(|e| !targets.contains(&e.family) && !record(e.family) && !layout(e)) {
            *counts.entry(family_name(e.family)).or_default() += 1;
        }
    };
    count(&font.extra);
    font.metrics.iter().for_each(|m| count(&m.extra));
    font.masters.iter().for_each(|m| count(&m.extra));
    for g in font.glyphs.iter() {
        count(&g.extra);
        for l in &g.layers {
            count(&l.extra);
            count(&l.info.attrs.extra);
            l.info.guides.iter().for_each(|g| count(&g.extra));
            l.info.annotations.iter().for_each(|a| count(&a.extra));
            l.info.image.iter().for_each(|i| count(&i.extra));
            let background = l.info.background.as_ref();
            if let Some(b) = background {
                count(&b.extra);
                b.guides.iter().for_each(|g| count(&g.extra));
                b.annotations.iter().for_each(|a| count(&a.extra));
                b.image.iter().for_each(|i| count(&i.extra));
            }
            let anchors = l.anchors.iter().chain(background.into_iter().flat_map(|b| b.anchors.iter()));
            anchors.for_each(|a| count(&a.extra));
            for s in l.shapes.iter().chain(background.into_iter().flat_map(|b| b.shapes.iter())) {
                match s {
                    Shape::Path(p) => {
                        count(&p.extra);
                        count(&p.info.attrs.extra);
                        // A node's other attribute keys travel in a UFO's node user data.
                        for n in &p.nodes {
                            let mut x = n.info.extra.clone();
                            if targets.contains(&Family::Ufo) {
                                x.remove(Family::Glyphs, "attr");
                            }
                            count(&x);
                        }
                    }
                    Shape::Component(c) => {
                        count(&c.extra);
                        count(&c.info.attrs.extra);
                    }
                    Shape::Image(i) => count(&i.extra),
                    Shape::Raw(_) => {}
                }
            }
        }
    }
    let mut out: Vec<Value> =
        counts.into_iter().map(|(family, entries)| json!({ "family": family, "entries": entries, "what": "keys read from that format, which this one has no place for" })).collect();
    let mut shapes: BTreeMap<&'static str, usize> = BTreeMap::new();
    for g in font.glyphs.iter() {
        for l in &g.layers {
            for s in l.shapes.iter().chain(l.info.background.iter().flat_map(|b| b.shapes.iter())) {
                if let Shape::Raw(r) = s
                    && !targets.contains(&r.family)
                {
                    *shapes.entry(family_name(r.family)).or_default() += 1;
                }
            }
        }
    }
    for (family, n) in shapes {
        out.push(json!({ "family": family, "entries": n, "what": "shapes read from that format that the model keeps as written, left out" }));
    }
    if !targets.contains(&Family::Designspace) {
        let items = fontcraft_io_ufo::ufo::kept_data_losses(font, targets.contains(&Family::Ufo));
        if !items.is_empty() {
            let family = if font.source.format == SourceFormat::Designspace { "designspace" } else { "ufo" };
            out.push(json!({ "family": family, "entries": items.len(), "what": "data only the UFO or designspace files held", "items": items }));
        }
    }
    // No source format but the native one has a documented place for bitmap strikes.
    if !font.bitmap_strikes.is_empty() {
        let items: Vec<String> = font.bitmap_strikes.iter().map(|s| s.label()).collect();
        out.push(
            json!({ "family": "fontcraft", "entries": items.len(), "what": "bitmap strikes, which this format has no place for", "items": items }),
        );
    }
    out
}

/// Files beside another format's document (a package's UI state, feature files), which a UFO does
/// not take.
fn side_file_losses(font: &Font) -> Option<Value> {
    let foreign = !matches!(font.source.format, SourceFormat::Ufo | SourceFormat::Designspace);
    (foreign && !font.files.is_empty()).then(|| {
        let names: Vec<&str> = font.files.0.keys().map(String::as_str).collect();
        json!({ "family": "glyphspackage", "entries": names.len(), "what": "files beside the glyphs in the package", "files": names })
    })
}

/// Layers a UFO has no place for yet: alternate layers (a designspace holds them, in a font with
/// axes), layers without a name, and the other layers of masters stored as a layer of another
/// UFO.
fn ufo_layer_losses(font: &Font, designspace: bool) -> Option<Value> {
    let n = fontcraft_io_ufo::ufo::layers_left_out(font, designspace);
    let what = if designspace {
        "layers without a name, alternate layers in a font without axes, and other layers of masters stored as a layer of another UFO, which a designspace cannot hold yet"
    } else {
        "alternate layers and layers without a name, which a UFO cannot hold yet"
    };
    (n > 0).then(|| json!({ "family": "glyphs", "entries": n, "what": what }))
}

/// Font information (properties, parameters, settings, user data, instance fields) a UFO, or a
/// designspace and its UFOs, has no place for yet.
fn ufo_info_losses(font: &Font, files: FilesFrom) -> Option<Value> {
    let designspace = files == FilesFrom::Designspace;
    let items = if designspace { fontcraft_io_ufo::designspace::info_losses(font) } else { fontcraft_io_ufo::ufo::info_losses_for(font, files) };
    let family = if font.source.format == SourceFormat::Glyphs { "glyphs" } else { "fontcraft" };
    let what =
        if designspace { "font information a designspace and its UFOs have no place for yet" } else { "font information a UFO has no place for yet" };
    (!items.is_empty()).then(|| json!({ "family": family, "entries": items.len(), "what": what, "items": items }))
}

/// Font information the `.glyphs` version being written has no place for.
fn glyphs_info_losses(font: &Font) -> Option<Value> {
    let items = fontcraft_io_glyphs::info_losses(font);
    (!items.is_empty()).then(
        || json!({ "family": "fontcraft", "entries": items.len(), "what": "font information this .glyphs version has no place for", "items": items }),
    )
}

fn family_name(f: Family) -> &'static str {
    match f {
        Family::Glyphs => "glyphs",
        Family::Ufo => "ufo",
        Family::Designspace => "designspace",
        Family::Binary => "binary",
        Family::Native => "fontcraft",
    }
}

/// `font` with the JPEG and TIFF images its UFOs hold as PNG files, decoded under the image
/// limits ([`fontcraft_io_ufo::images`]); `None` when it has none. A file that does not decode
/// stays as it is, and the loss report names it.
fn ufo_images_as_png(font: &Font, files: FilesFrom) -> Option<Font> {
    let convert = |bytes: &[u8]| -> Option<Vec<u8>> {
        let image = fontcraft_imagery::decode_rgba(bytes, fontcraft_imagery::MAX_PIXELS).ok()?;
        fontcraft_imagery::to_png(&image).ok()
    };
    fontcraft_io_ufo::images::to_png(font, files, &convert)
}

/// The result of a save.
pub struct Saved {
    pub bytes: usize,
    pub files_written: usize,
    pub files_removed: usize,
    pub losses: Vec<Value>,
}

/// What saving `font` in `format` would leave out, as [`save`] reports it in `Saved::losses`,
/// worked out without writing. Images are not converted first, so a JPEG or TIFF image a UFO
/// would hold as PNG is named too.
pub fn save_losses(font: &Font, format: SaveFormat) -> Vec<Value> {
    loss_report(&crate::linked::unlinked(font), format)
}

/// The losses of saving `font`, as [`save`] has prepared it, in `format`.
fn loss_report(font: &Font, format: SaveFormat) -> Vec<Value> {
    let files_from = if format == SaveFormat::Designspace { FilesFrom::Designspace } else { FilesFrom::Single };
    let mut losses = if format == SaveFormat::Native { Vec::new() } else { losses(font, format.families()) };
    if format.is_ufo() {
        losses.extend(ufo_layer_losses(font, format == SaveFormat::Designspace));
        losses.extend(side_file_losses(font));
        losses.extend(ufo_info_losses(font, files_from));
    }
    if matches!(format, SaveFormat::Glyphs | SaveFormat::GlyphsPackage) {
        losses.extend(glyphs_info_losses(font));
    }
    // A single file has no place for a package's side files (UI state, version 4 feature files).
    if format == SaveFormat::Glyphs && !font.files.is_empty() {
        let names: Vec<&str> = font.files.0.keys().map(String::as_str).collect();
        losses.push(json!({ "family": "glyphspackage", "entries": names.len(), "what": "files beside the glyphs in the package", "files": names }));
    }
    losses
}

/// Writes `font` to `path` in `format`. A single file is written atomically. A package writes
/// only the files whose bytes differ from what is on disk, and removes glyph files the document
/// no longer has. Linked glyphs and masters (Import Font, Import Master) are left out.
pub fn save(font: &Font, path: &Path, format: SaveFormat) -> Result<Saved> {
    let own = crate::linked::unlinked(font);
    let font: &Font = &own;
    let files_from = if format == SaveFormat::Designspace { FilesFrom::Designspace } else { FilesFrom::Single };
    let converted = if format.is_ufo() { ufo_images_as_png(font, files_from) } else { None };
    let font: &Font = converted.as_ref().unwrap_or(font);
    let losses = loss_report(font, format);
    match format {
        SaveFormat::Native | SaveFormat::Glyphs => {
            let bytes = if format == SaveFormat::Native {
                // Layouts the .glyphs reader kept for values an edit has since changed.
                let pruned = fontcraft_io_glyphs::without_stale_layouts(font);
                fontcraft_format::save(pruned.as_ref().unwrap_or(font))?
            } else {
                fontcraft_io_glyphs::write(font)?.into_bytes()
            };
            fontcraft_format::write_atomic(path, &bytes)?;
            Ok(Saved { bytes: bytes.len(), files_written: 1, files_removed: 0, losses })
        }
        SaveFormat::GlyphsPackage => {
            let files = fontcraft_io_glyphs::write_package(font)?;
            if path.exists() && !path.is_dir() {
                return Err(EngineError::Io(format!("cannot write a package to {}: it is a file", path.display())));
            }
            write_allowed(path)?;
            std::fs::create_dir_all(path).map_err(|e| io_err("create", path, e))?;
            let on_disk = read_folder(path)?;
            let mut written = 0;
            for (rel, bytes) in &files {
                if on_disk.get(rel) == Some(bytes) {
                    continue;
                }
                let target = safe_join(path, rel)?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io_err("create", parent, e))?;
                }
                fontcraft_format::write_atomic(&target, bytes)?;
                written += 1;
            }
            let mut removed = 0;
            for rel in on_disk.keys().filter(|r| r.starts_with("glyphs/") && r.ends_with(".glyph") && !files.contains_key(*r)) {
                let target = safe_join(path, rel)?;
                std::fs::remove_file(&target).map_err(|e| io_err("remove", &target, e))?;
                removed += 1;
            }
            Ok(Saved { bytes: files.values().map(Vec::len).sum(), files_written: written, files_removed: removed, losses })
        }
        SaveFormat::Ufo => {
            let files = fontcraft_io_ufo::ufo::write(font)?;
            let (written, removed) = write_ufo(path, &files)?;
            Ok(Saved { bytes: files.values().map(Vec::len).sum(), files_written: written, files_removed: removed, losses })
        }
        SaveFormat::Ufoz => {
            let files = fontcraft_io_ufo::ufo::write(font)?;
            if path.is_dir() {
                return Err(EngineError::Io(format!("cannot write a UFOZ to {}: it is a folder", path.display())));
            }
            let bytes = fontcraft_io_ufo::zip::write_ufoz(&files, &ufoz_root(path))?;
            fontcraft_format::write_atomic(path, &bytes)?;
            Ok(Saved { bytes: bytes.len(), files_written: 1, files_removed: 0, losses })
        }
        SaveFormat::Designspace => {
            let out = fontcraft_io_ufo::designspace::write(font)?;
            let dir = path.parent().unwrap_or(Path::new("."));
            let (mut written, mut removed, mut bytes) = (0, 0, out.document.len());
            // Every UFO is checked before any is written, so a refusal writes nothing.
            let mut targets = Vec::with_capacity(out.ufos.len());
            for (rel, files) in &out.ufos {
                let ufo_dir = safe_join_relative(dir, rel)?;
                write_allowed(&ufo_dir)?;
                targets.push((ufo_dir, files));
            }
            for (ufo_dir, files) in targets {
                let (w, r) = write_ufo(&ufo_dir, files)?;
                written += w;
                removed += r;
                bytes += files.values().map(Vec::len).sum::<usize>();
            }
            // The document last, so a crash before it leaves the old document pointing at UFOs
            // that are each complete.
            if std::fs::read(path).ok().as_deref() != Some(out.document.as_bytes()) {
                fontcraft_format::write_atomic(path, out.document.as_bytes())?;
                written += 1;
            }
            Ok(Saved { bytes, files_written: written, files_removed: removed, losses })
        }
    }
}

/// The folder a UFOZ written to `path` holds its UFO in: the file's name with `.ufo`, as UFO
/// writers name it.
fn ufoz_root(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty() && !s.contains(['/', '\\']));
    format!("{}.ufo", stem.as_deref().unwrap_or("font"))
}

/// What [`export_ufos`] wrote: each UFO's path, master and number of files written.
pub struct Exported {
    pub files: Vec<(PathBuf, String, usize)>,
    pub losses: Vec<Value>,
}

/// Writes each of `masters` as its own UFO 3 into the folder `destination`.
pub fn export_ufos(font: &Font, masters: &[String], destination: &Path) -> Result<Exported> {
    let mut font = font.clone();
    font.source.ufo_version = Some(3);
    if let Some(converted) = ufo_images_as_png(&font, FilesFrom::PerMaster) {
        font = converted;
    }
    let mut losses = losses(&font, &[Family::Ufo]);
    losses.extend(ufo_layer_losses(&font, false));
    losses.extend(side_file_losses(&font));
    losses.extend(ufo_info_losses(&font, FilesFrom::PerMaster));
    let mut taken = Vec::new();
    let mut files = Vec::new();
    for id in masters {
        let master = font.master(id)?;
        let name = fontcraft_io_ufo::designspace::ufo_file_name(&font, master, &taken);
        taken.push(name.clone());
        let ufo = fontcraft_io_ufo::ufo::write_one(&font, id)?;
        let path = destination.join(&name);
        let (written, _) = write_ufo(&path, &ufo)?;
        files.push((path, id.clone(), written));
    }
    Ok(Exported { files, losses })
}

/// Writes a UFO's files into `path`: only files whose bytes differ, the index files
/// (`contents.plist`, `layercontents.plist`, `metainfo.plist`) last; then removes glyph files,
/// layer index files and standard files the document no longer has. Refuses to write into a
/// folder that is neither empty nor a UFO.
fn write_ufo(path: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(usize, usize)> {
    if path.exists() && !path.is_dir() {
        return Err(EngineError::Io(format!("cannot write a UFO to {}: it is a file", path.display())));
    }
    if path.is_dir() && !path.join("metainfo.plist").is_file() && std::fs::read_dir(path).map_err(|e| io_err("read", path, e))?.next().is_some() {
        return Err(EngineError::Io(format!("cannot write a UFO to {}: the folder is not empty and not a UFO", path.display())));
    }
    write_allowed(path)?;
    std::fs::create_dir_all(path).map_err(|e| io_err("create", path, e))?;
    let on_disk = read_folder(path)?;
    let is_index = |rel: &str| rel.ends_with("/contents.plist") || rel == "layercontents.plist" || rel == "metainfo.plist";
    let mut written = 0;
    for index_pass in [false, true] {
        for (rel, bytes) in files.iter().filter(|(rel, _)| is_index(rel) == index_pass) {
            if on_disk.get(rel) == Some(bytes) {
                continue;
            }
            let target = safe_join(path, rel)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_err("create", parent, e))?;
            }
            fontcraft_format::write_atomic(&target, bytes)?;
            written += 1;
        }
    }
    const STANDARD: [&str; 5] = ["groups.plist", "kerning.plist", "lib.plist", "features.fea", "layercontents.plist"];
    let stale = |rel: &str| {
        let in_layer = rel.starts_with("glyphs") && rel.split('/').count() == 2;
        let layer_file = rel.ends_with(".glif") || rel.ends_with("/contents.plist") || rel.ends_with("/layerinfo.plist");
        in_layer && layer_file || STANDARD.contains(&rel)
    };
    let mut removed = 0;
    for rel in on_disk.keys().filter(|r| stale(r) && !files.contains_key(*r)) {
        let target = safe_join(path, rel)?;
        std::fs::remove_file(&target).map_err(|e| io_err("remove", &target, e))?;
        removed += 1;
        // A layer folder left empty goes too.
        if let Some(parent) = target.parent()
            && parent != path
            && std::fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none())
        {
            std::fs::remove_dir(parent).map_err(|e| io_err("remove", parent, e))?;
        }
    }
    Ok((written, removed))
}

/// A designspace's `filename` relative to its folder. `..` is allowed, as designspace files use
/// it for UFOs beside their folder, except while automation roots are in force
/// ([`crate::file_access`]); absolute paths and backslashes are not.
fn safe_join_relative(dir: &Path, rel: &str) -> Result<PathBuf> {
    let ok =
        !rel.is_empty() && !rel.starts_with('/') && !rel.contains('\\') && !rel.contains(':') && rel.split('/').all(|p| !p.is_empty() && p != ".");
    if !ok {
        return Err(EngineError::Io(format!("refusing to write the UFO `{rel}`: not a relative path")));
    }
    if crate::file_access::current().is_some() && rel.split('/').any(|p| p == "..") {
        return Err(EngineError::Io(format!(
            "refusing to write the UFO `{rel}`: a source above the designspace's folder is not written while automation file access is confined"
        )));
    }
    Ok(dir.join(rel))
}

/// `root/rel`, refusing a relative path that would leave `root`, and one outside the automation
/// write root.
fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let ok = !rel.is_empty()
        && !rel.starts_with('/')
        && rel.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\\'));
    if !ok {
        return Err(EngineError::Io(format!("refusing to write `{rel}` outside the package")));
    }
    let target = root.join(rel);
    write_allowed(&target)?;
    Ok(target)
}

/// May `path` be written while automation roots are in force (a link inside a package can lead
/// out of the write root)? Always, without them.
fn write_allowed(path: &Path) -> Result<()> {
    crate::file_access::check_write(path).map_err(|e| EngineError::Io(format!("refusing to write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_formats_are_named_by_extensions_in_any_case() {
        assert_eq!(SaveFormat::of_path(Path::new("dir/Font.UFO")), Some(SaveFormat::Ufo));
        assert_eq!(SaveFormat::of_path(Path::new("Font.Glyphs")), Some(SaveFormat::Glyphs));
        assert_eq!(SaveFormat::of_path(Path::new("Font.designspace")), Some(SaveFormat::Designspace));
        assert_eq!(SaveFormat::of_path(Path::new("Font.otf")), None);
        assert_eq!(SaveFormat::of_path(Path::new("Font")), None);
    }

    #[test]
    fn native_files_are_told_from_glyphs_files() {
        assert!(is_native("{\n  \"format\": \"fontcraft\",\n  \"version\": 2"));
        assert!(!is_native("{\n.appVersion = \"3343\";\n.formatVersion = 3;"));
        assert!(!is_native("{\n\"Some Key\" = \"fontcraft\";"));
    }

    #[test]
    fn designspace_documents_are_recognised() {
        assert!(is_designspace("<?xml version='1.0' encoding='UTF-8'?>\n<!-- c -->\n<designspace format=\"5.0\">"));
        assert!(is_designspace("<designspace>"));
        assert!(!is_designspace("<?xml version='1.0'?>\n<plist version=\"1.0\">"));
        assert!(!is_designspace("<designspacex>"));
        assert!(!is_designspace("<?xml never closed"));
    }

    #[test]
    fn svg_fonts_open_as_documents() {
        let dir = std::env::temp_dir().join(format!("fontcraft-svg-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Test.svg");
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><defs><font id="Test" horiz-adv-x="500"><font-face font-family="Test" units-per-em="1000"/><glyph glyph-name="A" unicode="A" d="M0 0L500 0L250 700Z"/></font></defs></svg>"#;
        std::fs::write(&path, svg).unwrap();
        let (font, _) = open(&path, None).unwrap();
        assert!(font.glyphs.iter().any(|g| g.name == "A"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A link inside a folder is not followed, whether it leads to a folder or a file, so a
    /// package cannot make a read reach outside it.
    #[cfg(unix)]
    #[test]
    fn folder_links_are_not_followed() {
        let base = std::env::temp_dir().join(format!("fontcraft-read-folder-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("pkg")).unwrap();
        std::fs::create_dir_all(base.join("outside/glyphs")).unwrap();
        std::fs::write(base.join("pkg/metainfo.plist"), "in").unwrap();
        std::fs::write(base.join("outside/glyphs/a.glif"), "out").unwrap();
        std::fs::write(base.join("outside/secret"), "out").unwrap();
        std::os::unix::fs::symlink(base.join("outside/glyphs"), base.join("pkg/glyphs")).unwrap();
        std::os::unix::fs::symlink(base.join("outside/secret"), base.join("pkg/secret")).unwrap();
        let files = read_folder(&base.join("pkg")).unwrap();
        assert_eq!(files.keys().collect::<Vec<_>>(), ["metainfo.plist"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Checking the references of a named pipe returns at once: opening it would wait for a
    /// writer.
    #[cfg(unix)]
    #[test]
    fn a_named_pipe_refers_to_nothing() {
        let dir = std::env::temp_dir().join(format!("fontcraft-references-fifo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("Font.designspace");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let mut seen = Vec::new();
        check_references(&fifo, &mut |p| {
            seen.push(p.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert!(seen.is_empty(), "{seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_paths_cannot_escape() {
        let root = Path::new("/tmp/pkg");
        assert!(safe_join(root, "glyphs/A_.glyph").is_ok());
        for bad in ["", "/etc/passwd", "../x", "glyphs/../../x", "a//b", "a\\b", "./x"] {
            assert!(safe_join(root, bad).is_err(), "{bad}");
        }
    }
}

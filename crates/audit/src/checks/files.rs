//! File checks (M12.57): the compiled font as a file. Its header and table directory, its size
//! and name, whether every table parses and every glyph draws, and what external tools say about
//! it (`docs/spec/21-qa.md` § 8, "Files"). The rules follow FontBakery's `sfnt_version`,
//! `file_size`, `family/single_directory`, `ots`, `ttx_roundtrip`, `freetype_rasterizer`,
//! `fontvalidator` and `googlefonts/canonical_filename`; the sanitiser, the round trip and the
//! rasteriser are our own parser, text dump and outline drawing rather than external programs.

use read_fonts::tables::name::Name;
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, ReadError, TableProvider};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, Target};

use crate::export::{ValidatorFacts, known_validator_issue};
use crate::services::{ExternalTool, MAX_EXTERNAL_MESSAGES};
use crate::{BinaryCtx, FamilyCtx, Hit, Location, Params, Severity, Sink, Threshold, num};

/// The sfnt version of fonts with TrueType outlines.
const TRUETYPE: u32 = 0x0001_0000;
/// The sfnt version of fonts with CFF or CFF2 outlines (`OTTO`).
const OTTO: u32 = u32::from_be_bytes(*b"OTTO");
/// The sfnt version some older Apple TrueType fonts use (`true`).
const APPLE_TRUE: u32 = u32::from_be_bytes(*b"true");

/// The tables a browser-style sanitiser refuses a font without.
const REQUIRED_TABLES: [&[u8; 4]; 8] = [b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post"];

/// Most directories a single-directory finding lists by name.
const MAX_LISTED_DIRECTORIES: usize = 10;

pub(crate) const WARN_SIZE: Threshold = Threshold {
    name: "warnSize",
    default: 1_048_576.0,
    min: 0.0,
    max: 4_294_967_296.0,
    doc: "File size in bytes above which the size is a warning (1 MiB).",
};

pub(crate) const FAIL_SIZE: Threshold = Threshold {
    name: "failSize",
    default: 9_437_184.0,
    min: 0.0,
    max: 4_294_967_296.0,
    doc: "File size in bytes above which the size is a failure (9 MiB).",
};

pub(crate) const RASTER_PPEM: Threshold =
    Threshold { name: "ppem", default: 48.0, min: 1.0, max: 4_096.0, doc: "The size, in pixels per em, every glyph is drawn at." };

fn tag(t: &[u8; 4]) -> Tag {
    Tag::new(t)
}

/// Does the table directory list `t`? (The table's data may still lie outside the file.)
fn has_table(font: &FontRef<'_>, t: &[u8; 4]) -> bool {
    let wanted = tag(t);
    font.table_directory.table_records().iter().any(|r| r.tag() == wanted)
}

fn is_cff(font: &FontRef<'_>) -> bool {
    has_table(font, b"CFF ") || has_table(font, b"CFF2")
}

fn table_place(table: &str) -> Location {
    Location { table: Some(table.to_string()), ..Location::default() }
}

/// An sfnt version as messages show it: its four letters when they are printable, else hex.
fn version_text(v: u32) -> String {
    let bytes = v.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') { format!("'{}'", String::from_utf8_lossy(&bytes)) } else { format!("0x{v:08X}") }
}

/// `check.files.sfntVersion`: the header's version tag matches the outline format.
pub(crate) fn sfnt_version(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let version = ctx.font.table_directory.sfnt_version();
    let place = Location { field: Some("sfntVersion".into()), ..Location::default() };
    if has_table(&ctx.font, b"glyf") {
        if version != TRUETYPE {
            let message = format!("The font has TrueType outlines but its sfnt version is {}; it should be 0x00010000.", version_text(version));
            sink.push(Hit::new("wrong-sfnt-version-ttf", message).place(place));
        }
    } else if is_cff(&ctx.font) && version != OTTO {
        let message = format!("The font has CFF outlines but its sfnt version is {}; it should be 'OTTO'.", version_text(version));
        sink.push(Hit::new("wrong-sfnt-version-cff", message).place(place));
    }
}

/// A size in bytes as messages show it.
fn size_text(bytes: f64) -> String {
    const MIB: f64 = 1_048_576.0;
    if bytes >= MIB { format!("{} MiB", num(bytes / MIB)) } else { format!("{} KiB", num(bytes / 1024.0)) }
}

/// `check.files.fileSize`: the file on disk, or the font's bytes when it was compiled in memory.
pub(crate) fn file_size(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let size = ctx.file.as_ref().map_or(ctx.bytes.len() as f64, |f| f.size as f64);
    let (warn, fail) = (params.get(&WARN_SIZE), params.get(&FAIL_SIZE));
    if size > fail {
        let message = format!("The font file is {}, larger than the limit of {}.", size_text(size), size_text(fail));
        sink.push(Hit::new("massive-font", message).data("size", size).data("limit", fail));
    } else if size > warn {
        let message = format!("The font file is {}; it should be at most {}.", size_text(size), size_text(warn));
        sink.push(Hit::new("large-font", message).data("size", size).data("limit", warn));
    }
}

/// `check.files.singleDirectory`: the files of one family lie in one directory.
pub(crate) fn single_directory(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if ctx.fonts.iter().all(|f| f.file.is_none()) {
        sink.skip("the fonts were not read from files");
        return;
    }
    let mut directories: Vec<&str> = Vec::new();
    for file in ctx.fonts.iter().filter_map(|f| f.file.as_ref()) {
        let dir = file.directory();
        if !directories.contains(&dir) {
            directories.push(dir);
        }
    }
    if directories.len() > 1 {
        let mut listed: Vec<String> = directories.iter().take(MAX_LISTED_DIRECTORIES).map(|d| format!("`{d}`")).collect();
        if directories.len() > MAX_LISTED_DIRECTORIES {
            listed.push(format!("and {} more", directories.len() - MAX_LISTED_DIRECTORIES));
        }
        let message = format!("The fonts lie in {} directories ({}), so they may not belong to one family.", directories.len(), listed.join(", "));
        sink.push(Hit::new("single-directory", message).data("directories", directories.len() as f64));
    }
}

/// `check.files.sanitiser`: our own strict pass over the table directory and every table we can
/// parse, then the messages of an external sanitiser when one is installed and asked for.
pub(crate) fn sanitiser(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if check_directory(ctx, sink) {
        check_tables(ctx, sink);
    }
    // An external sanitiser runs only when installed and asked for; without one our pass stands.
    if let Ok(messages) = ctx.services.run_external(ExternalTool::Sanitiser, ctx.bytes) {
        for m in messages.iter().take(MAX_EXTERNAL_MESSAGES) {
            let code = match m.severity {
                Severity::Fail => "sanitiser-error",
                Severity::Warn => "sanitiser-warning",
                Severity::Info => continue,
            };
            let place = Location { table: m.table.clone(), glyph: m.glyph.clone().unwrap_or_default(), ..Location::default() };
            sink.push(Hit::new(code, format!("The table sanitiser reports: {}", m.message)).place(place));
        }
    }
}

/// The header and table directory. Returns false when there are no tables to parse.
fn check_directory(ctx: &BinaryCtx<'_>, sink: &mut Sink) -> bool {
    let dir = &ctx.font.table_directory;
    let version = dir.sfnt_version();
    if ![TRUETYPE, OTTO, APPLE_TRUE].contains(&version) {
        let message = format!("The sfnt version {} is not one browsers accept (0x00010000 or 'OTTO').", version_text(version));
        sink.push(Hit::new("bad-sfnt-version", message).place(Location { field: Some("sfntVersion".into()), ..Location::default() }));
    }
    let records = dir.table_records();
    let Some(count) = u32::try_from(records.len()).ok().filter(|n| *n > 0) else {
        sink.push(Hit::new("no-tables", "The font has no tables."));
        return false;
    };
    // The binary-search fields: the largest power of two not above the table count, times 16.
    let selector = 31 - count.leading_zeros();
    let search = 16u32 << selector;
    let shift = (count * 16).saturating_sub(search);
    let stated = (u32::from(dir.search_range()), u32::from(dir.entry_selector()), u32::from(dir.range_shift()));
    if stated != (search, selector, shift) {
        let message = format!(
            "The table directory's search fields are {}, {} and {}; for {count} tables they should be {search}, {selector} and {shift}.",
            stated.0, stated.1, stated.2
        );
        sink.push(Hit::new("bad-search-fields", message).data("tables", f64::from(count)));
    }
    if let Some((a, b)) = records.iter().zip(records.iter().skip(1)).find(|(a, b)| a.tag() >= b.tag()) {
        let message = if a.tag() == b.tag() {
            format!("The table directory lists '{}' twice.", a.tag())
        } else {
            format!("The table directory lists '{}' after '{}'; tables must be sorted by tag.", b.tag(), a.tag())
        };
        sink.push(Hit::new("unsorted-tables", message).place(table_place(&b.tag().to_string())));
    }
    let len = ctx.bytes.len() as u64;
    let mut spans: Vec<(u64, u64, Tag)> = Vec::with_capacity(records.len());
    for r in records {
        let (start, length) = (u64::from(r.offset()), u64::from(r.length()));
        let end = start + length;
        let name = r.tag().to_string();
        if end > len {
            let message = format!("Table '{name}' runs from byte {start} to {end}, past the end of the {len}-byte file.");
            sink.push(Hit::new("table-out-of-bounds", message).place(table_place(&name)));
        } else if start % 4 != 0 {
            let message = format!("Table '{name}' starts at byte {start}, which is not a multiple of 4.");
            sink.push(Hit::new("misaligned-table", message).place(table_place(&name)));
        }
        if length > 0 {
            spans.push((start, end, r.tag()));
        }
    }
    spans.sort_unstable();
    for (a, b) in spans.iter().zip(spans.iter().skip(1)) {
        if b.0 < a.1 {
            let message = format!("Tables '{}' and '{}' overlap.", a.2, b.2);
            sink.push(Hit::new("overlapping-tables", message).place(table_place(&b.2.to_string())));
        }
    }
    for t in REQUIRED_TABLES {
        if !has_table(&ctx.font, t) {
            let name = String::from_utf8_lossy(t).into_owned();
            sink.push(Hit::new("missing-table", format!("The font has no '{name}' table, which every font needs.")).place(table_place(&name)));
        }
    }
    let glyf = has_table(&ctx.font, b"glyf");
    if !glyf && !is_cff(&ctx.font) {
        sink.push(Hit::new("missing-table", "The font has no outline table: neither 'glyf' nor 'CFF ' nor 'CFF2'."));
    }
    if glyf && !has_table(&ctx.font, b"loca") {
        sink.push(Hit::new("missing-table", "The font has a 'glyf' table but no 'loca' table.").place(table_place("loca")));
    }
    true
}

fn bad_table(sink: &mut Sink, table: &str, error: &ReadError) {
    sink.push(Hit::new("bad-table", format!("Table '{table}' cannot be read: {error}.")).place(table_place(table)));
}

/// The index and error of the first item that fails to parse.
fn first_error<T>(items: impl Iterator<Item = Result<T, ReadError>>) -> Option<(usize, ReadError)> {
    items.enumerate().find_map(|(i, r)| r.err().map(|e| (i, e)))
}

/// Parses every table the directory lists and FontCraft can read; tables whose data lies
/// outside the file were reported with the directory.
fn check_tables(ctx: &BinaryCtx<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let present = |t: &[u8; 4]| font.table_data(tag(t)).is_some();
    if present(b"head") {
        match font.head() {
            Ok(head) if head.magic_number() != 0x5F0F_3CF5 => {
                sink.push(Hit::new("bad-table", "Table 'head' has the wrong magic number.").place(table_place("head")));
            }
            Ok(_) => {}
            Err(e) => bad_table(sink, "head", &e),
        }
    }
    let simple: [(&[u8; 4], fn(&FontRef<'_>) -> Option<ReadError>); 13] = [
        (b"hhea", |f| f.hhea().err()),
        (b"maxp", |f| f.maxp().err()),
        (b"hmtx", |f| f.hmtx().err()),
        (b"OS/2", |f| f.os2().err()),
        (b"post", |f| f.post().err()),
        (b"CFF ", |f| f.cff().err()),
        (b"CFF2", |f| f.cff2().err()),
        (b"GDEF", |f| f.gdef().err()),
        (b"fvar", |f| f.fvar().and_then(|t| t.axes().map(|_| ())).err()),
        (b"avar", |f| f.avar().err()),
        (b"gvar", |f| f.gvar().err()),
        (b"HVAR", |f| f.hvar().err()),
        (b"STAT", |f| f.stat().err()),
    ];
    for (t, parse) in simple {
        if present(t)
            && let Some(e) = parse(font)
        {
            bad_table(sink, &String::from_utf8_lossy(t), &e);
        }
    }
    if present(b"name") {
        check_name(font, sink);
    }
    if present(b"cmap") {
        check_cmap(font, sink);
    }
    if present(b"GSUB") {
        let r = font.gsub().and_then(|t| {
            t.script_list()?;
            t.feature_list()?;
            Ok(first_error(t.lookup_list()?.lookups().iter()))
        });
        layout_result(sink, "GSUB", r);
    }
    if present(b"GPOS") {
        let r = font.gpos().and_then(|t| {
            t.script_list()?;
            t.feature_list()?;
            Ok(first_error(t.lookup_list()?.lookups().iter()))
        });
        layout_result(sink, "GPOS", r);
    }
    if present(b"glyf") && present(b"loca") {
        check_glyphs(ctx, sink);
    }
}

fn layout_result(sink: &mut Sink, table: &str, r: Result<Option<(usize, ReadError)>, ReadError>) {
    match r {
        Err(e) => bad_table(sink, table, &e),
        Ok(Some((i, e))) => {
            let place = Location { table: Some(table.into()), lookup: Some(i), ..Location::default() };
            sink.push(Hit::new("bad-table", format!("Lookup {i} of table '{table}' cannot be read: {e}.")).place(place));
        }
        Ok(None) => {}
    }
}

fn check_name(font: &FontRef<'_>, sink: &mut Sink) {
    let name = match font.name() {
        Ok(n) => n,
        Err(e) => return bad_table(sink, "name", &e),
    };
    let data = name.string_data();
    for (i, record) in name.name_record().iter().enumerate() {
        if let Err(e) = record.string(data) {
            let place = Location { name_id: Some(record.name_id().to_u16()), platform: Some(record.platform_id()), ..table_place("name") };
            sink.push(Hit::new("bad-table", format!("Name record {i} of table 'name' cannot be read: {e}.")).place(place));
        }
    }
}

fn check_cmap(font: &FontRef<'_>, sink: &mut Sink) {
    let cmap = match font.cmap() {
        Ok(c) => c,
        Err(e) => return bad_table(sink, "cmap", &e),
    };
    let data = cmap.offset_data();
    for (i, record) in cmap.encoding_records().iter().enumerate() {
        if let Err(e) = record.subtable(data) {
            let message = format!("Subtable {i} of table 'cmap' (platform {}) cannot be read: {e}.", record.platform_id() as u16);
            sink.push(Hit::new("bad-table", message).place(table_place("cmap")));
        }
    }
}

/// Each glyph `loca` points at in `glyf` parses.
fn check_glyphs(ctx: &BinaryCtx<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let (loca, glyf, maxp) = match (font.loca(None), font.glyf(), font.maxp()) {
        (Ok(l), Ok(g), Ok(m)) => (l, g, m),
        (Err(e), _, _) => return bad_table(sink, "loca", &e),
        (_, Err(e), _) => return bad_table(sink, "glyf", &e),
        // A 'maxp' that cannot be read is reported with the tables.
        (_, _, Err(_)) => return,
    };
    for gid in 0..u32::from(maxp.num_glyphs()) {
        if let Err(e) = loca.get_glyf(GlyphId::new(gid), &glyf) {
            let name = ctx.source.and_then(|m| m.glyph(gid)).unwrap_or_default().to_string();
            let place = Location { glyph: name, ..table_place("glyf") };
            sink.push(Hit::new("bad-glyph", format!("Glyph {gid} cannot be read from 'glyf': {e}.")).data("glyphId", f64::from(gid)).place(place));
        }
    }
}

/// `check.files.roundTrip`: the font dumps to the text format and reads back.
pub(crate) fn round_trip(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    match ctx.services.ttx_round_trip(ctx.bytes) {
        Err(u) => sink.skip(u.reason),
        Ok(rt) => {
            if let Some(error) = rt.error {
                let message = format!("Dumping the font to the text format and reading the dump back fails: {error}");
                sink.push(Hit::new("round-trip-failed", message));
            }
        }
    }
}

/// A pen that discards what it is given: drawing is done for its errors only.
struct NullPen;

impl OutlinePen for NullPen {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

/// Does the font carry hinting programs or hints that a hinted draw would run?
fn has_hints(font: &FontRef<'_>) -> bool {
    has_table(font, b"fpgm") || has_table(font, b"prep") || is_cff(font)
}

/// `check.files.rasterises`: every glyph draws at the test size, unhinted and, when the font
/// has hints, hinted with the font's own programs (not pedantic, so only errors a renderer
/// stops on are reported).
pub(crate) fn rasterises(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let Ok(maxp) = ctx.font.maxp() else {
        sink.skip("the font's 'maxp' table cannot be read");
        return;
    };
    let outlines = ctx.font.outline_glyphs();
    if outlines.format().is_none() {
        sink.skip("the font has no outline table that can be drawn");
        return;
    }
    let ppem = params.get(&RASTER_PPEM);
    let size = Size::new(ppem as f32);
    let location = LocationRef::default();
    let hinting = if has_hints(&ctx.font) {
        let options = HintingOptions { engine: Engine::Interpreter, target: Target::default() };
        match HintingInstance::new(&outlines, size, location, options) {
            Ok(instance) => Some(instance),
            Err(e) => {
                let message = format!("The font's hinting programs fail at {} pixels per em: {e}.", num(ppem));
                sink.push(Hit::new("hinting-failed", message).data("ppem", ppem));
                None
            }
        }
    } else {
        None
    };
    let mut pen = NullPen;
    for gid in 0..u32::from(maxp.num_glyphs()) {
        let name = ctx.source.and_then(|m| m.glyph(gid)).unwrap_or_default().to_string();
        let place = Location { glyph: name, ..Location::default() };
        let Some(glyph) = outlines.get(GlyphId::new(gid)) else {
            let message = format!("Glyph {gid} has no outline that can be read.");
            sink.push(Hit::new("no-outline", message).data("glyphId", f64::from(gid)).place(place));
            continue;
        };
        if let Err(e) = glyph.draw(DrawSettings::unhinted(size, location), &mut pen) {
            let message = format!("Glyph {gid} cannot be drawn at {} pixels per em: {e}.", num(ppem));
            sink.push(Hit::new("draw-failed", message).data("glyphId", f64::from(gid)).data("ppem", ppem).place(place));
        } else if let Some(instance) = &hinting
            && let Err(e) = glyph.draw(DrawSettings::hinted(instance, false), &mut pen)
        {
            let message = format!("Glyph {gid} cannot be drawn hinted at {} pixels per em: {e}.", num(ppem));
            sink.push(Hit::new("hinted-draw-failed", message).data("glyphId", f64::from(gid)).data("ppem", ppem).place(place));
        }
    }
}

/// `check.files.externalValidator`: the external validator's messages, less the known false
/// positives. Its errors and warnings are warnings here, its notes information.
pub(crate) fn external_validator(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let messages = match ctx.services.run_external(ExternalTool::Validator, ctx.bytes) {
        Ok(m) => m,
        Err(u) => return sink.skip(u.reason),
    };
    // One face alone does not say whether it is the italic of another face, so the
    // single-style allowance is left off here.
    let facts = ValidatorFacts::of_font(&ctx.font, false);
    for m in messages.iter().take(MAX_EXTERNAL_MESSAGES) {
        if known_validator_issue(m.id.as_deref().unwrap_or_default(), &m.message, &facts).is_some() {
            continue;
        }
        let code = match m.severity {
            Severity::Fail => "validator-error",
            Severity::Warn => "validator-warning",
            Severity::Info => "validator-info",
        };
        let text = match &m.id {
            Some(id) => format!("The external validator reports {id}: {}", m.message),
            None => format!("The external validator reports: {}", m.message),
        };
        let place = Location { table: m.table.clone(), glyph: m.glyph.clone().unwrap_or_default(), ..Location::default() };
        sink.push(Hit::new(code, text).place(place));
    }
}

/// The English name with the first of `ids` the font has, else any of its names, as fontTools'
/// `getDebugName` picks it.
fn best_name(name: &Name<'_>, ids: &[u16]) -> Option<String> {
    let data = name.string_data();
    ids.iter().find_map(|&id| {
        let mut some = None;
        for record in name.name_record().iter().filter(|r| r.name_id().to_u16() == id) {
            let Ok(s) = record.string(data) else { continue };
            let text = s.chars().collect::<String>();
            if matches!((record.platform_id(), record.language_id()), (1, 0) | (3, 0x409)) {
                return Some(text);
            }
            some = Some(text);
        }
        some
    })
}

/// The file name the font's names give it: `Family-Style.ext` for a static font,
/// `Family[tags].ext` (with `-Italic` before the bracket for an italic) for a variable one. The
/// extension is `otf` for CFF outlines and `ttf` otherwise, or the file's own `woff` or `woff2`.
fn canonical_name(font: &FontRef<'_>, current: &str) -> Option<String> {
    let name = font.name().ok()?;
    let family: String = best_name(&name, &[16, 1])?.chars().filter(|c| *c != ' ').collect();
    if family.is_empty() {
        return None;
    }
    let style: String = best_name(&name, &[17, 2]).unwrap_or_default().chars().filter(|c| *c != ' ').collect();
    let style = if style.is_empty() { "Regular".to_string() } else { style };
    let own = current.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let ext = match own.as_deref() {
        Some("woff") => "woff",
        Some("woff2") => "woff2",
        _ if is_cff(font) => "otf",
        _ => "ttf",
    };
    if has_table(font, b"fvar") {
        let mut tags: Vec<String> = font.fvar().ok()?.axes().ok()?.iter().map(|a| a.axis_tag().to_string()).collect();
        tags.sort();
        let italic = if style.contains("Italic") { "-Italic" } else { "" };
        return Some(format!("{family}{italic}[{}].{ext}", tags.join(",")));
    }
    Some(format!("{family}-{style}.{ext}"))
}

/// `check.files.canonicalFileName`: the file name is the one the font's names give it.
pub(crate) fn canonical_file_name(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(file) = &ctx.file else {
        sink.skip("the font was not read from a file");
        return;
    };
    let current = file.name();
    let Some(expected) = canonical_name(&ctx.font, current) else {
        sink.skip("the font's family name cannot be read");
        return;
    };
    if current != expected {
        let message = format!("The file is called `{current}`; the font's names make it `{expected}`.");
        sink.push(Hit::new("bad-filename", message));
    }
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;

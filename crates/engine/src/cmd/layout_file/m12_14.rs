//! The runners of M12.14: `layout.compileFeatures` and `layout.lintFeatures` (rows `features
//! compile`, `features lint`). Oracle: fontTools feaLib.
//!
//! Compile reads the source (feature code; FontDame text or a VOLT project, each turned into
//! feature code by `fontcraft_tables::fontdame` and `::volt`), compiles it with `fontcraft_fea`
//! against the glyph names a compiler sees for the font, and puts the result into the font as
//! feaLib's builder does (`fontcraft_tables::layout_install`): each requested layout table is
//! replaced, or removed when the code leaves it empty; a table block changes only the fields it
//! sets; `usMaxContext` is recomputed. Code with errors writes nothing. With `debugLookups` the
//! font also gets feaLib's `Debg` table, from `fontcraft_fea::LookupDebug`.
//!
//! Lint runs the same compiler. With a glyph set it compiles in memory, so errors of the
//! table-building stage are found too; without one it validates only and leaves out the
//! compiler's messages about glyphs missing from the font, since every name would be missing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use fontcraft_fea::{CompileRequest, Compiled, Diagnostic, GlyphOrder, LayoutTable, MetricsValues, Severity, VirtualFiles};
use fontcraft_tables::fontdame::{self, FontDameOptions};
use fontcraft_tables::layout_install::{self, BlockValues, MetricFields, Os2Fields};
use fontcraft_tables::limits::MAX_FEATURE_SOURCE;
use fontcraft_tables::sfnt::{Tag, WriteOptions, tag_str};
use fontcraft_tables::volt::{self, VoltOptions};
use serde_json::{Value, json};

use super::super::file_io::{self, InputKind, OutputRequest};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// The tables a compile may build, as feaLib names them.
const SUPPORTED: [(&str, Tag); 10] = [
    ("BASE", *b"BASE"),
    ("GDEF", *b"GDEF"),
    ("GPOS", *b"GPOS"),
    ("GSUB", *b"GSUB"),
    ("OS/2", *b"OS/2"),
    ("head", *b"head"),
    ("hhea", *b"hhea"),
    ("name", *b"name"),
    ("vhea", *b"vhea"),
    ("STAT", *b"STAT"),
];
/// The tables that hold a VOLT project inside a font.
const VOLT_TABLES: [Tag; 4] = [*b"TSIV", *b"TSIS", *b"TSIP", *b"TSID"];
/// Most FontDame files compiled together (one per table, and a few spare).
const MAX_SOURCES: usize = 16;
/// Most errors quoted in the message of a failed compile.
const MAX_QUOTED_ERRORS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Syntax {
    Fea,
    Mti,
    Volt,
}

impl Syntax {
    fn name(self) -> &'static str {
        match self {
            Syntax::Fea => "fea",
            Syntax::Mti => "mti",
            Syntax::Volt => "volt",
        }
    }
}

fn tag_text(tag: Tag) -> String {
    tag_str(tag).trim_end().to_string()
}

fn fea_tag_text(tag: fontcraft_fea::Tag) -> String {
    tag_text(tag.into_bytes())
}

/// A text file of at most [`MAX_FEATURE_SOURCE`] bytes, without a byte-order mark.
fn read_text(path: &Path) -> Result<String> {
    let input = file_io::read_input(path)?;
    if input.bytes.len() > MAX_FEATURE_SOURCE {
        return Err(EngineError::Io(format!("{} is larger than {MAX_FEATURE_SOURCE} bytes", path.display())));
    }
    let text = String::from_utf8(input.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", path.display())))?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    })
}

/// The text of a source file and, for a VOLT project stored in a font's `TSIV` table, that
/// font's glyph order.
fn read_source(path: &Path) -> Result<(String, Option<Vec<String>>)> {
    let input = file_io::read_input(path)?;
    if input.kind.is_font() {
        let font = fontcraft_tables::read(&input.bytes, 0)?;
        let tsiv =
            font.get(*b"TSIV").ok_or_else(|| EngineError::Io(format!("{} is a font without a VOLT project (no TSIV table)", path.display())))?;
        let text = std::str::from_utf8(tsiv).map_err(|_| EngineError::Io(format!("the TSIV table of {} is not UTF-8 text", path.display())))?;
        let order = fontcraft_tables::ttx::glyph_order(&font)?;
        return Ok((text.trim_end_matches('\0').to_string(), Some(order)));
    }
    Ok((read_text(path)?, None))
}

/// The syntax of a source, from its first line that is neither empty nor a comment: FontDame
/// text names its table (`FontDame GSUB table`, `Font Chef Table GPOS`), a VOLT project starts
/// with one of its definition keywords, anything else is feature code.
fn detect(text: &str) -> Syntax {
    let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('%')) else {
        return Syntax::Fea;
    };
    let mut words = line.split_whitespace();
    let first = words.next().unwrap_or_default();
    if first == "FontDame" || (first == "Font" && words.next() == Some("Chef") && words.next() == Some("Table")) {
        return Syntax::Mti;
    }
    let volt = ["GRID_PPEM", "PRESENTATION_PPEM", "PPOSITIONING_PPEM", "COMPILER_USEEXTENSIONLOOKUPS", "COMPILER_USEPAIRPOSFORMAT2", "CMAP_FORMAT"];
    if first.starts_with("DEF_") || volt.contains(&first) { Syntax::Volt } else { Syntax::Fea }
}

/// The root file and the folder its includes are read from.
fn virtual_files(p: &Params<'_>, root: &str, text: &str, include_dir: &Path) -> Result<VirtualFiles> {
    let files = VirtualFiles::new(root, text).map_err(|d| p.bad(d.message))?.with_include_dir(include_dir.to_path_buf());
    Ok(match crate::file_access::include_check() {
        Some(check) => files.with_include_check(check),
        None => files,
    })
}

/// The file name of `path` for the compiler and its messages.
fn root_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "features.fea".to_string())
}

/// `includeDir`, else the source's folder.
fn include_dir(p: &Params<'_>, source: &Path) -> Result<PathBuf> {
    Ok(match p.opt_str("includeDir")? {
        Some(dir) => PathBuf::from(dir),
        None => source.parent().map(Path::to_path_buf).unwrap_or_default(),
    })
}

/// The face a `face` parameter names: a number, or a full or PostScript name in a collection.
fn face(p: &Params<'_>, bytes: &[u8]) -> Result<u32> {
    match p.get("face") {
        None => Ok(0),
        Some(Value::String(name)) => super::face_by_name(p, bytes, name),
        Some(v) => v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| p.bad("`face` must be a face number or a face name")),
    }
}

/// Line and column (from 1, columns in characters) of a byte offset of `text`.
fn line_column(text: &str, offset: usize) -> Option<(usize, usize)> {
    let before = text.get(..offset)?;
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = before.get(start..)?.chars().count() + 1;
    Some((line, column))
}

/// Where diagnostics point: the texts of the files they name, read once each.
struct Places<'a> {
    files: &'a VirtualFiles,
    /// The root file as the user named it.
    root_display: String,
    include_dir: PathBuf,
    disk: HashMap<String, Option<String>>,
}

impl<'a> Places<'a> {
    fn new(files: &'a VirtualFiles, root_display: String, include_dir: PathBuf) -> Self {
        Self { files, root_display, include_dir, disk: HashMap::new() }
    }

    /// The file as the user would name it, and the line and column of the diagnostic's start.
    fn place(&mut self, d: &Diagnostic) -> Option<(String, Option<(usize, usize)>)> {
        if d.file.is_empty() {
            return None;
        }
        if let Some(text) = self.files.get(&d.file) {
            let shown = if d.file == self.files.root() { self.root_display.clone() } else { d.file.clone() };
            return Some((shown, line_column(text, d.span.start)));
        }
        // A file the compiler read from the include folder; its text is read again, within the
        // same limits, only to place the message.
        let path = self.include_dir.join(&d.file);
        let text = self.disk.entry(d.file.clone()).or_insert_with(|| {
            crate::file_access::check_read(&path).ok()?;
            read_text(&path).ok()
        });
        let position = text.as_deref().and_then(|t| line_column(t, d.span.start));
        Some((path.display().to_string(), position))
    }

    fn json(&mut self, severity: &str, d: &Diagnostic) -> Value {
        let (file, position) = match self.place(d) {
            Some((file, position)) => (Some(file), position),
            None => (None, None),
        };
        json!({
            "severity": severity,
            "message": d.message,
            "file": file,
            "line": position.map(|p| p.0),
            "column": position.map(|p| p.1),
        })
    }

    /// `file:line:column: error: message`, as compilers print it.
    fn line(&mut self, d: &Diagnostic) -> String {
        let severity = if d.is_error() { "error" } else { "warning" };
        match self.place(d) {
            Some((file, Some((line, column)))) => format!("{file}:{line}:{column}: {severity}: {}", d.message),
            Some((file, None)) => format!("{file}: {severity}: {}", d.message),
            None => format!("{severity}: {}", d.message),
        }
    }
}

fn severity(d: &Diagnostic) -> &'static str {
    match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// The error of a compile that failed: the count and the first errors with their places.
fn compile_failure(places: &mut Places<'_>, diagnostics: &[Diagnostic]) -> EngineError {
    let errors: Vec<&Diagnostic> = diagnostics.iter().filter(|d| d.is_error()).collect();
    let shown: Vec<&Diagnostic> = if errors.is_empty() { diagnostics.iter().collect() } else { errors };
    let count = shown.len();
    let mut text = format!("the feature code has {count} error{}; nothing was written", if count == 1 { "" } else { "s" });
    for d in shown.iter().take(MAX_QUOTED_ERRORS) {
        text.push('\n');
        text.push_str(&places.line(d));
    }
    if count > MAX_QUOTED_ERRORS {
        text.push_str(&format!("\nand {} more", count - MAX_QUOTED_ERRORS));
    }
    EngineError::Interaction(text)
}

/// `tables`, else every supported table.
fn wanted_tables(p: &Params<'_>) -> Result<Vec<Tag>> {
    let Some(list) = p.opt_strings("tables")? else { return Ok(SUPPORTED.iter().map(|(_, t)| *t).collect()) };
    let mut out = Vec::new();
    for item in list {
        let Some((_, tag)) = SUPPORTED.iter().find(|(name, _)| *name == item) else {
            let names: Vec<&str> = SUPPORTED.iter().map(|(n, _)| *n).collect();
            return Err(p.bad(format!("`tables` takes {}, got \"{item}\"", names.join(", "))));
        };
        if !out.contains(tag) {
            out.push(*tag);
        }
    }
    Ok(out)
}

/// The values of the code's table blocks for the tables wanted.
fn block_values(c: &Compiled, wanted: &[Tag]) -> BlockValues {
    let metrics =
        |m: &MetricsValues| MetricFields { ascender: m.ascender, descender: m.descender, line_gap: m.line_gap, caret_offset: m.caret_offset };
    let o = &c.os2;
    let os2 = Os2Fields {
        weight_class: o.weight_class,
        width_class: o.width_class,
        fs_type: o.fs_type,
        family_class: o.family_class,
        panose: o.panose,
        unicode_range: o.unicode_range,
        vendor_id: o.vendor_id.map(|t| t.into_bytes()),
        typo_ascender: o.typo_ascender,
        typo_descender: o.typo_descender,
        typo_line_gap: o.typo_line_gap,
        win_ascent: o.win_ascent,
        win_descent: o.win_descent,
        code_page_range: o.code_page_range,
        x_height: o.x_height,
        cap_height: o.cap_height,
        lower_optical_point_size: o.lower_optical_point_size,
        upper_optical_point_size: o.upper_optical_point_size,
    };
    let has = |t: &Tag| wanted.contains(t);
    BlockValues {
        font_revision: if has(b"head") { c.head.font_revision } else { None },
        hhea: if has(b"hhea") { metrics(&c.hhea) } else { MetricFields::default() },
        vhea: if has(b"vhea") { metrics(&c.vhea) } else { MetricFields::default() },
        os2: if has(b"OS/2") { os2 } else { Os2Fields::default() },
    }
}

/// The glyph names the code refers to, by glyph id: the font's, or with `goadb` the source
/// names it maps them from.
fn source_names(p: &Params<'_>, font: &fontcraft_tables::SfntFont) -> Result<Vec<String>> {
    let mut names = layout_install::compiler_glyph_names(font)?;
    if let Some(goadb) = p.opt_str("goadb")? {
        let parsed = fontcraft_tables::goadb::parse(&read_text(Path::new(goadb))?)?;
        let map: HashMap<&str, &str> = parsed.entries.iter().map(|e| (e.final_name.as_str(), e.source_name.as_str())).collect();
        for name in &mut names {
            if let Some(source) = map.get(name.as_str()) {
                *name = (*source).to_string();
            }
        }
    }
    Ok(names)
}

pub(super) fn compile_features(_: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("layout.compileFeatures", raw)?;
    let sources: Vec<PathBuf> = p.strings("source")?.into_iter().map(PathBuf::from).collect();
    let Some(first) = sources.first().cloned() else { return Err(p.bad("`source` names no file")) };
    if sources.len() > MAX_SOURCES {
        return Err(p.bad(format!("`source` names more than {MAX_SOURCES} files")));
    }
    let font_path = PathBuf::from(p.str("font")?);
    let to_ttx = match p.opt_str("to")?.unwrap_or("font") {
        "font" => false,
        "ttx" => true,
        other => return Err(p.bad(format!("`to` must be \"font\" or \"ttx\", got \"{other}\""))),
    };
    let syntax = match p.opt_str("syntax")? {
        None => None,
        Some("fea") => Some(Syntax::Fea),
        Some("mti") => Some(Syntax::Mti),
        Some("volt") => Some(Syntax::Volt),
        Some(other) => return Err(p.bad(format!("`syntax` must be \"fea\", \"mti\" or \"volt\", got \"{other}\""))),
    };
    let wanted = wanted_tables(&p)?;
    let debug_lookups = p.bool_or("debugLookups", false)?;
    let ship = p.bool_or("ship", false)?;
    let emit_fea = p.opt_str("emitFea")?.map(PathBuf::from);
    let req = OutputRequest::from_params(&p)?;
    if req.in_place && to_ttx {
        return Err(p.bad("`inPlace` would replace the font with TTX text: give `output` or `outputDir`"));
    }

    let input = file_io::read_input(&font_path)?;
    if !input.kind.is_font() {
        return Err(EngineError::Io(format!("{} is not a compiled font ({})", font_path.display(), input.kind.name())));
    }
    let plain = matches!(input.kind, InputKind::TrueType | InputKind::Cff);
    if req.in_place && !plain {
        return Err(p.bad(format!(
            "{} is a {} file and the result is a plain font: give `output` or `outputDir`",
            font_path.display(),
            input.kind.name()
        )));
    }
    let mut font = fontcraft_tables::read(&input.bytes, face(&p, &input.bytes)?)?;

    let mut texts = Vec::with_capacity(sources.len());
    let mut font_order: Option<Vec<String>> = None;
    for source in &sources {
        let (text, order) = read_source(source)?;
        font_order = font_order.or(order);
        texts.push(text);
    }
    let from_font = font_order.is_some();
    let syntax = match syntax {
        Some(s) => s,
        None if from_font => Syntax::Volt,
        None => detect(texts.first().map(String::as_str).unwrap_or_default()),
    };
    if from_font && syntax != Syntax::Volt {
        return Err(p.bad("a font given as `source` holds a VOLT project: `syntax` must be \"volt\""));
    }
    if sources.len() > 1 && syntax != Syntax::Mti {
        return Err(p.bad("several sources are taken only for FontDame text (one file per table)"));
    }
    if emit_fea.is_some() && syntax == Syntax::Fea {
        return Err(p.bad("`emitFea` writes the feature code that FontDame or VOLT input becomes; this source is feature code already"));
    }

    let names = source_names(&p, &font)?;
    let mut warnings: Vec<String> = Vec::new();
    let code = match syntax {
        Syntax::Fea => texts.into_iter().next().unwrap_or_default(),
        Syntax::Mti => {
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            let options = FontDameOptions { glyph_order: names.clone(), ..FontDameOptions::default() };
            let (fea, notes) = fontdame::convert(&refs, &options)?;
            warnings.extend(notes);
            fea
        }
        Syntax::Volt => {
            let options = VoltOptions { glyph_order: font_order, ..VoltOptions::default() };
            let (fea, notes) = volt::convert(texts.first().map(String::as_str).unwrap_or_default(), &options)?;
            warnings.extend(notes);
            fea
        }
    };

    let (root, root_display) = match syntax {
        Syntax::Fea => (root_name(&first), first.display().to_string()),
        _ => (format!("{}.fea", root_name(&first)), format!("{} (as feature code)", first.display())),
    };
    let include_dir = include_dir(&p, &first)?;
    let files = virtual_files(&p, &root, &code, &include_dir)?;
    let order = GlyphOrder::new(names).map_err(|d| EngineError::Interaction(d.message))?;
    let first_name_id = layout_install::first_free_name_id(&font)?;
    let request = CompileRequest { first_name_id, debug_lookups, ..CompileRequest::new(&files, &order) };
    let mut places = Places::new(&files, root_display, include_dir.clone());
    let compiled = match fontcraft_fea::compile(&request) {
        Ok(compiled) => compiled,
        Err(diagnostics) => return Err(compile_failure(&mut places, &diagnostics)),
    };

    // Installed in feaLib's order: the layout tables, names and blocks, then usMaxContext.
    let mut written: Vec<Tag> = Vec::new();
    let mut removed: Vec<Tag> = Vec::new();
    let layout = [(*b"GSUB", &compiled.gsub), (*b"GPOS", &compiled.gpos), (*b"GDEF", &compiled.gdef), (*b"BASE", &compiled.base)];
    for (tag, table) in layout {
        if !wanted.contains(&tag) {
            continue;
        }
        match table {
            Some(bytes) => {
                font.insert(tag, bytes.clone());
                written.push(tag);
            }
            None => {
                if font.remove(tag).is_some() {
                    removed.push(tag);
                }
            }
        }
    }
    if let (true, Some(stat)) = (wanted.contains(b"STAT"), &compiled.stat) {
        font.insert(*b"STAT", stat.clone());
        written.push(*b"STAT");
    }
    if let (true, Some(code_names)) = (wanted.contains(b"name"), &compiled.name) {
        let merged = layout_install::merge_names(font.get(*b"name"), code_names)?;
        font.insert(*b"name", merged);
        written.push(*b"name");
    }
    let report = layout_install::apply_block_values(&mut font, &block_values(&compiled, &wanted))?;
    written.extend(report.changed);
    warnings.extend(report.warnings);
    if layout_install::update_max_context(&mut font)? && !written.contains(b"OS/2") {
        written.push(*b"OS/2");
    }
    if let (true, Some(debug)) = (debug_lookups, &compiled.lookup_debug) {
        let mut debug = debug.clone();
        // As feaLib: a table the compile does not build records no lookups.
        if !wanted.contains(b"GSUB") {
            debug.gsub.clear();
        }
        if !wanted.contains(b"GPOS") {
            debug.gpos.clear();
        }
        // Files are named as feaLib names them: the source as given, an included file joined to
        // the include folder; converted code is named by the file it is written to, if any.
        let shown_root = match syntax {
            Syntax::Fea => first.display().to_string(),
            _ => emit_fea.as_ref().map_or_else(|| "<features>".to_string(), |f| f.display().to_string()),
        };
        let shown = |file: &str| if file == root { shown_root.clone() } else { include_dir.join(file).display().to_string() };
        font.insert(*b"Debg", debug.to_debg(&shown));
        written.push(*b"Debg");
    }
    if ship {
        for tag in VOLT_TABLES {
            if font.remove(tag).is_some() {
                removed.push(tag);
            }
        }
    }

    let (bytes, extension) = if to_ttx {
        let mut tables = written.clone();
        tables.sort_unstable();
        tables.dedup();
        if tables.is_empty() {
            return Err(EngineError::Interaction("the source builds no table to write as TTX".into()));
        }
        let options = fontcraft_tables::ttx::DumpOptions { tables: fontcraft_tables::ttx::sorted_tag_list(&tables), ..Default::default() };
        (fontcraft_tables::ttx::dump(&font, &options)?.into_bytes(), Some("ttx"))
    } else {
        let extension = if plain {
            None
        } else if font.is_cff() {
            Some("otf")
        } else {
            Some("ttf")
        };
        (fontcraft_tables::write(&font, &WriteOptions::default())?, extension)
    };
    let out = file_io::output_path(&font_path, &req, ".features", extension)?;
    if let Some(fea) = &emit_fea {
        // Checked before the font is written, so a refused second file leaves nothing behind.
        file_io::check_write(fea)?;
        if req.no_clobber && fea.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", fea.display())));
        }
        if *fea == out {
            return Err(p.bad("`emitFea` is the output file: give another path"));
        }
    }
    file_io::write_output(&out, &bytes, &req)?;
    if let Some(fea) = &emit_fea {
        let fea_request = OutputRequest { no_clobber: req.no_clobber, ..OutputRequest::default() };
        file_io::write_output(fea, code.as_bytes(), &fea_request)?;
    }

    let features: Vec<Value> = compiled
        .features
        .iter()
        .map(|f| {
            let systems: Vec<Value> = f.language_systems.iter().map(|(s, l)| json!([fea_tag_text(*s), fea_tag_text(*l)])).collect();
            let table = match f.table {
                LayoutTable::Gsub => "GSUB",
                LayoutTable::Gpos => "GPOS",
            };
            json!({ "tag": fea_tag_text(f.tag), "table": table, "languageSystems": systems, "lookups": f.lookups })
        })
        .collect();
    let diagnostics: Vec<Value> = compiled.warnings.iter().map(|d| places.json(severity(d), d)).collect();
    let mut result = json!({
        "output": out,
        "syntax": syntax.name(),
        "tables": written.iter().map(|t| tag_text(*t)).collect::<Vec<_>>(),
        "removed": removed.iter().map(|t| tag_text(*t)).collect::<Vec<_>>(),
        "features": features,
        "diagnostics": diagnostics,
        "warnings": warnings,
    });
    if let (Some(fea), Some(o)) = (&emit_fea, result.as_object_mut()) {
        o.insert("emitFea".into(), json!(fea));
    }
    Ok(result)
}

/// The compiler's messages that only say a glyph is not in the font.
fn about_missing_glyph(d: &Diagnostic) -> bool {
    let m = d.message.as_str();
    m == "glyph not in font" || m == "CID not in font" || (m.starts_with("Range member '") && m.ends_with("' does not exist in font"))
}

/// The first word of each line that is neither empty nor a comment, each name once: a plain
/// glyph list or a GOADB (whose first column is the final name).
fn glyph_list_file(p: &Params<'_>, path: &Path) -> Result<Vec<String>> {
    let text = read_text(path)?;
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some(name) = line.split_whitespace().next() else { continue };
        if seen.insert(name) {
            names.push(name.to_string());
        }
        if names.len() > fontcraft_fea::MAX_GLYPHS {
            return Err(p.bad(format!("{} lists more than {} glyphs", path.display(), fontcraft_fea::MAX_GLYPHS)));
        }
    }
    Ok(names)
}

pub(super) fn lint_features(_: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("layout.lintFeatures", raw)?;
    let source = PathBuf::from(p.str("source")?);
    let all = match p.opt_str("warnings")?.unwrap_or("default") {
        "default" => false,
        "all" => true,
        other => return Err(p.bad(format!("`warnings` must be \"default\" or \"all\", got \"{other}\""))),
    };
    if ["font", "glyphOrder", "glyphNames"].iter().filter(|k| p.get(k).is_some()).count() > 1 {
        return Err(p.bad("give one of `font`, `glyphOrder` and `glyphNames`"));
    }
    let names: Option<Vec<String>> = if let Some(path) = p.opt_str("font")? {
        let input = file_io::read_input(Path::new(path))?;
        if !input.kind.is_font() {
            return Err(EngineError::Io(format!("{path} is not a compiled font ({})", input.kind.name())));
        }
        let font = fontcraft_tables::read(&input.bytes, face(&p, &input.bytes)?)?;
        Some(layout_install::compiler_glyph_names(&font)?)
    } else if let Some(path) = p.opt_str("glyphOrder")? {
        Some(glyph_list_file(&p, Path::new(path))?)
    } else if let Some(list) = p.opt_strings("glyphNames")? {
        if list.len() > fontcraft_fea::MAX_GLYPHS {
            return Err(p.bad(format!("`glyphNames` lists more than {} glyphs", fontcraft_fea::MAX_GLYPHS)));
        }
        let mut seen = HashSet::new();
        Some(list.into_iter().filter(|n| seen.insert(*n)).map(str::to_string).collect())
    } else {
        None
    };
    let (text, font_order) = read_source(&source)?;
    if font_order.is_some() {
        return Err(p.bad("`source` is a font: the lint reads feature code"));
    }
    let include_dir = include_dir(&p, &source)?;
    let files = virtual_files(&p, &root_name(&source), &text, &include_dir)?;
    let checked = names.is_some();
    let order = GlyphOrder::new(names.unwrap_or_else(|| vec![".notdef".to_string()])).map_err(|d| p.bad(d.message))?;
    let request = CompileRequest::new(&files, &order);
    let mut notes: Vec<Value> = Vec::new();
    let found: Vec<Diagnostic> = if checked {
        match fontcraft_fea::compile(&request) {
            Ok(compiled) => {
                if let (true, Some(gpos)) = (all, &compiled.gpos) {
                    for (lookup, gid) in layout_install::marks_used_as_bases(gpos)? {
                        let name = order.names().get(usize::from(gid)).cloned().unwrap_or_else(|| format!("glyph {gid}"));
                        notes.push(json!({
                            "severity": "note",
                            "message": format!("glyph `{name}` is both a mark and a base in attachment lookup {lookup}"),
                            "file": Value::Null,
                            "line": Value::Null,
                            "column": Value::Null,
                        }));
                    }
                }
                compiled.warnings
            }
            Err(diagnostics) => diagnostics,
        }
    } else {
        fontcraft_fea::validate(&request).into_iter().filter(|d| !about_missing_glyph(d)).collect()
    };
    let mut places = Places::new(&files, source.display().to_string(), include_dir);
    let errors = found.iter().filter(|d| d.is_error()).count();
    let warning_count = found.len() - errors;
    let mut diagnostics: Vec<Value> = found.iter().map(|d| places.json(severity(d), d)).collect();
    let note_count = notes.len();
    diagnostics.extend(notes);
    Ok(json!({
        "errors": errors,
        "warnings": warning_count,
        "notes": note_count,
        "glyphsChecked": checked,
        "diagnostics": diagnostics,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_told_apart_by_their_first_line() {
        assert_eq!(detect("# comment\nlanguagesystem DFLT dflt;"), Syntax::Fea);
        assert_eq!(detect("\nFontDame GSUB table\n"), Syntax::Mti);
        assert_eq!(detect("Font Chef Table GPOS\n"), Syntax::Mti);
        assert_eq!(detect("DEF_GLYPH \"a\" ID 1 END_GLYPH"), Syntax::Volt);
        assert_eq!(detect(""), Syntax::Fea);
    }

    #[test]
    fn positions_count_lines_and_characters_from_one() {
        let text = "a\nbé c";
        assert_eq!(line_column(text, 0), Some((1, 1)));
        assert_eq!(line_column(text, 2), Some((2, 1)));
        assert_eq!(line_column(text, 6), Some((2, 4)), "é is two bytes and one column");
        assert_eq!(line_column(text, 4), None, "inside a character");
        assert_eq!(line_column(text, 99), None);
    }
}

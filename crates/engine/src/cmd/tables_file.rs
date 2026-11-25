//! Table actions on compiled fonts: `tables.extract`, `.add`, `.delete`, `.edit`, `.checksum`, `.reorder` and `tables.show` (M12.2; `fontcraft_tables::edit`, `fontcraft_tables::show`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. A WOFF or WOFF2 input is unwrapped, edited and
//! wrapped again with its own settings (header version, metadata, private data, which tables
//! were transformed); a collection member is edited and the collection packed again. Tables no
//! action names are copied byte for byte; the font is rewritten in the recommended table order
//! with fresh checksums, as the reference tool does.
//!
//! As in the reference tool, extracting or deleting a table the font does not have is a warning,
//! not an error.

use std::path::{Path, PathBuf};

use fontcraft_container::{Extras, woff, woff2};
use fontcraft_tables::edit::{EditOptions, checksums, edit};
use fontcraft_tables::sfnt::{parse_tag, tag_str};
use fontcraft_tables::show::{ShowNode, ShowOptions, find, prune, show};
use fontcraft_tables::{SfntFont, TableOrder, TablesError, Tag, WriteOptions};
use serde_json::{Map, Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "tables.extract",
            "Extract Tables",
            [],
            None,
            "{font: path, face?: number|name, tables: [\"TAG\"|\"TAG=FILE\"|{tag, file?}] | \"all\", outputDir?: path, noClobber?: bool} writes the raw bytes of each table to a file (default: the font's name without extension, a period and the tag with other characters than letters and digits as underscores) and changes nothing else; a table the font does not have is a warning → {files: [{tag, path, length}], warnings: [string]}",
            always,
            tables_extract
        ).files(const { &[FileParam::read("font"), FileParam::write("tables").tables(), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "tables.add",
            "Add or Replace Tables",
            [],
            None,
            "{font: path, face?: number|name, tables: [\"TAG=FILE\"|{tag, file?, from?: {font: path, face?: number|name}}], output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} puts raw tables from files or from another font into the font, replacing tables with the same tag, and writes it in the recommended order with fresh checksums (default output: FONT.edited.EXT) → {output, added: [tag], replaced: [tag]}",
            always,
            tables_add
        ).files(const { &[FileParam::read("font"), FileParam::read("tables").tables(), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "tables.delete",
            "Delete Tables",
            [],
            None,
            "{font: path, face?: number|name, tables: [tag], output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} removes the tables and writes the font with fresh checksums; a table the font does not have is a warning → {output, deleted: [tag], warnings: [string]}",
            always,
            tables_delete
        ).files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "tables.edit",
            "Edit Tables",
            [],
            None,
            "{font: path, face?: number|name, extract?: [\"TAG\"|\"TAG=FILE\"|{tag, file?}], delete?: [tag], add?: [\"TAG=FILE\"|{tag, file}], output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} extracts, then deletes, then adds, whatever order the parameters are in, and writes the font once when it changed → {files: [{tag, path, length}], output?, deleted: [tag], added: [tag], replaced: [tag], warnings: [string]}",
            always,
            tables_edit
        ).files(const { &[FileParam::read("font"), FileParam::write("extract").tables(), FileParam::read("add").tables(), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "tables.checksum",
            "Check Table Checksums",
            [],
            None,
            "{font: path, face?: number|name|\"all\", fix?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} checks every table checksum and head.checkSumAdjustment (every member of a collection by default); with fix, writes the font again with correct values (default output: FONT.fixed.EXT) → {ok, faces: [{face, ok, mismatches: [{tag, stored, computed}], adjustment?: {stored, computed}}], output?}",
            always,
            tables_checksum
        ).files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "tables.reorder",
            "Reorder Tables",
            [],
            None,
            "{font: path, face?: number|name, order?: \"recommended\"|[tag], output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} writes the font with its tables in the recommended order, or the tags given first and the others after them in file order; table bytes are unchanged (default output: FONT.reordered.EXT) → {output, order: [tag]}",
            always,
            tables_reorder
        ).files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "tables.show",
            "Show Tables",
            [],
            None,
            "{font: path, face?: number|name, tables?: [tag], path?: string, depth?: number, offsets?: bool, glyphNames?: bool} the decoded fields of tables as a tree (depth 3 by default; deeper records are counted in `more`); path narrows to one record, as GPOS.lookupList.lookups[3]; a table that is not decoded is listed with its length and any error; nothing is written → {tables: [{tag, length, checksum, decoded, truncated, tree, error?}]}, tree = {name, type, value?, offset?, children?, more?}",
            always,
            tables_show
        ).files(const { &[FileParam::read("font")] }),
    ]
}

/// Default depth of `tables.show`, as the command line documents it.
const SHOW_DEPTH: usize = 3;

// ── Reading and writing a font in its container ─────────────────────────────────────────────────

/// How the font was stored, so it can be stored the same way again.
pub(super) enum Wrap {
    Plain,
    Woff(Extras),
    Woff2(woff2::Options),
    /// The members of the collection and the index of the one edited.
    Collection(Vec<SfntFont>, usize),
}

pub(super) struct Loaded {
    pub(super) path: PathBuf,
    pub(super) font: SfntFont,
    pub(super) wrap: Wrap,
}

fn tables_err(e: impl Into<TablesError>) -> EngineError {
    EngineError::Tables(e.into())
}

/// The font at `params[key]`, member `params[face_key]` of a collection.
pub(super) fn load(p: &Params<'_>, key: &str, face_key: &str) -> Result<Loaded> {
    let path = PathBuf::from(p.str(key)?);
    let input = file_io::read_input(&path)?;
    let face = p.get(face_key);
    // A file holding one font accepts that font's index (0) or its PostScript name.
    let plain_face = |p: &Params<'_>, font: &SfntFont| -> Result<()> {
        match face {
            None => Ok(()),
            Some(v) if v.as_u64() == Some(0) || v.as_str() == Some("0") => Ok(()),
            Some(Value::String(name)) if postscript_name(font).as_deref() == Some(name.as_str()) => Ok(()),
            Some(_) => Err(p.bad(format!("{} holds one font: `{face_key}` can only be 0 or its PostScript name", path.display()))),
        }
    };
    let (font, wrap) = match input.kind {
        InputKind::TrueType | InputKind::Cff => {
            let font = fontcraft_tables::read(&input.bytes, 0)?;
            plain_face(p, &font)?;
            (font, Wrap::Plain)
        }
        InputKind::Woff => {
            let unwrapped = woff::decode(&input.bytes).map_err(tables_err)?;
            let font = fontcraft_tables::read(&unwrapped.sfnt, 0)?;
            plain_face(p, &font)?;
            let extras = Extras { version: Some(unwrapped.version), metadata: unwrapped.metadata, private: unwrapped.private };
            (font, Wrap::Woff(extras))
        }
        InputKind::Woff2 => {
            let dir = woff2::directory(&input.bytes).map_err(tables_err)?;
            let transformed = |tag: &Tag| dir.tables.iter().any(|t| &t.tag == tag && t.transformed());
            let (transform_glyf, transform_hmtx) = (transformed(b"glyf"), transformed(b"hmtx"));
            let unwrapped = woff2::decode(&input.bytes).map_err(tables_err)?;
            let font = fontcraft_tables::read(&unwrapped.sfnt, 0)?;
            plain_face(p, &font)?;
            let extras = Extras { version: Some(unwrapped.version), metadata: unwrapped.metadata, private: unwrapped.private };
            (font, Wrap::Woff2(woff2::Options { transform_glyf, transform_hmtx, extras }))
        }
        InputKind::Collection => {
            let fonts = fontcraft_tables::collection::unpack(&input.bytes)?;
            let index = member(p, face_key, &fonts)?;
            let font = fonts.get(index).cloned().ok_or_else(|| p.bad(format!("the collection has no font number {index}")))?;
            (font, Wrap::Collection(fonts, index))
        }
        InputKind::Ttx | InputKind::Other => {
            return Err(EngineError::Io(format!("{} is not a compiled font (TrueType, OpenType, WOFF, WOFF2 or collection)", path.display())));
        }
    };
    Ok(Loaded { path, font, wrap })
}

/// The member a `face` parameter names: an index, or a PostScript name (name ID 6).
fn member(p: &Params<'_>, key: &str, fonts: &[SfntFont]) -> Result<usize> {
    match p.get(key) {
        None => Ok(0),
        Some(Value::Number(n)) => {
            let index = n
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(|| p.bad(format!("`{key}` must be a member index or a PostScript name")))?;
            if index >= fonts.len() {
                return Err(p.bad(format!("the collection has {} fonts; `{key}` {index} does not exist", fonts.len())));
            }
            Ok(index)
        }
        Some(Value::String(name)) => {
            if let Ok(index) = name.parse::<usize>() {
                if index >= fonts.len() {
                    return Err(p.bad(format!("the collection has {} fonts; `{key}` {index} does not exist", fonts.len())));
                }
                return Ok(index);
            }
            fonts
                .iter()
                .position(|f| postscript_name(f).as_deref() == Some(name.as_str()))
                .ok_or_else(|| p.bad(format!("no member of the collection is named `{name}`")))
        }
        Some(_) => Err(p.bad(format!("`{key}` must be a member index or a PostScript name"))),
    }
}

/// Name ID 6 of a font, read from its `name` table.
pub(super) fn postscript_name(font: &SfntFont) -> Option<String> {
    use skrifa::raw::FontRead as _;
    let name = skrifa::raw::tables::name::Name::read(skrifa::raw::FontData::new(font.get(*b"name")?)).ok()?;
    let strings = name.string_data();
    name.name_record().iter().filter(|r| r.name_id().to_u16() == 6).find_map(|r| r.string(strings).ok().map(|s| s.chars().collect()))
}

/// The bytes of `font` stored as `wrap` says.
pub(super) fn save(wrap: &Wrap, font: &SfntFont, order: TableOrder) -> Result<Vec<u8>> {
    match wrap {
        Wrap::Plain => Ok(fontcraft_tables::write(font, &WriteOptions { order })?),
        Wrap::Woff(extras) => {
            let sfnt = fontcraft_tables::write(font, &WriteOptions { order })?;
            woff::encode(&sfnt, extras).map_err(tables_err)
        }
        Wrap::Woff2(options) => {
            let sfnt = fontcraft_tables::write(font, &WriteOptions { order })?;
            woff2::encode(&sfnt, options).map_err(tables_err)
        }
        Wrap::Collection(fonts, index) => {
            let mut fonts = fonts.clone();
            if let Some(slot) = fonts.get_mut(*index) {
                *slot = font.clone();
            }
            Ok(fontcraft_tables::collection::pack(&fonts, &fontcraft_tables::collection::PackOptions::default())?)
        }
    }
}

/// Writes the edited font where the output parameters say; `suffix` names the operation in a
/// derived file name.
pub(super) fn write_font(p: &Params<'_>, loaded: &Loaded, font: &SfntFont, order: TableOrder, suffix: &str) -> Result<PathBuf> {
    let req = OutputRequest::from_params(p)?;
    let bytes = save(&loaded.wrap, font, order)?;
    let out = file_io::output_path(&loaded.path, &req, suffix, None)?;
    file_io::write_output(&out, &bytes, &req)?;
    Ok(out)
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

fn tag_text(tag: Tag) -> String {
    tag_str(tag).trim_end().to_string()
}

fn hex32(v: u32) -> String {
    format!("{v:08X}")
}

fn tag(p: &Params<'_>, text: &str) -> Result<Tag> {
    parse_tag(text).map_err(|e| p.bad(e.to_string()))
}

/// A table and, optionally, a file: `"TAG"`, `"TAG=FILE"` or `{tag, file?}`. A string may hold
/// several, comma-separated, as the reference tool accepts.
fn table_files(p: &Params<'_>, key: &str, items: &[Value]) -> Result<Vec<(Tag, Option<PathBuf>)>> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::String(s) => {
                for part in s.split(',').filter(|s| !s.is_empty()) {
                    let (t, file) = match part.split_once('=') {
                        Some((t, f)) => (t, Some(PathBuf::from(f))),
                        None => (part, None),
                    };
                    out.push((tag(p, t)?, file));
                }
            }
            Value::Object(o) => {
                let t = o.get("tag").and_then(Value::as_str).ok_or_else(|| p.bad(format!("each item of `{key}` needs a `tag`")))?;
                let file = match o.get("file") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(f)) => Some(PathBuf::from(f)),
                    Some(_) => return Err(p.bad(format!("`file` in `{key}` must be a path"))),
                };
                out.push((tag(p, t)?, file));
            }
            _ => return Err(p.bad(format!("`{key}` must list tags, \"TAG=FILE\" strings or {{tag, file}} objects"))),
        }
        if out.len() > fontcraft_tables::limits::MAX_TABLES {
            return Err(p.bad(format!("`{key}` names more than {} tables", fontcraft_tables::limits::MAX_TABLES)));
        }
    }
    Ok(out)
}

/// Plain tags: `["GPOS", "GSUB"]`, `"GPOS,GSUB"`.
fn tags(p: &Params<'_>, key: &str) -> Result<Vec<Tag>> {
    let Some(items) = p.get(key) else { return Ok(Vec::new()) };
    let items = match items {
        Value::String(_) => std::slice::from_ref(items),
        Value::Array(a) => a.as_slice(),
        _ => return Err(p.bad(format!("`{key}` must be a list of table tags"))),
    };
    let pairs = table_files(p, key, items)?;
    if pairs.iter().any(|(_, f)| f.is_some()) {
        return Err(p.bad(format!("`{key}` takes table tags only, without files")));
    }
    Ok(pairs.into_iter().map(|(t, _)| t).collect())
}

/// Where an extracted table goes: the given file, or the font's name without extension, a
/// period and the tag (characters other than letters and digits as underscores, trailing ones
/// dropped), in `outputDir` or beside the font.
fn extract_path(font: &Path, out_dir: Option<&Path>, tag: Tag, file: Option<&PathBuf>) -> PathBuf {
    if let Some(file) = file {
        return file.clone();
    }
    let stem = font.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "font".into());
    let mut part: String = tag_str(tag).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    while part.ends_with('_') {
        part.pop();
    }
    let dir = out_dir.map(Path::to_path_buf).or_else(|| font.parent().map(Path::to_path_buf)).unwrap_or_default();
    dir.join(format!("{stem}.{part}"))
}

/// Writes the extracted tables, refusing before writing any when `noClobber` forbids one.
fn write_extracted(p: &Params<'_>, loaded: &Loaded, items: &[(Tag, Option<PathBuf>)], warnings: &mut Vec<String>) -> Result<Vec<Value>> {
    let out_dir = p.opt_str("outputDir")?.map(PathBuf::from);
    let no_clobber = p.bool_or("noClobber", false)?;
    let mut planned = Vec::new();
    for (tag, file) in items {
        let Some(data) = loaded.font.get(*tag) else {
            warnings.push(format!("the font has no '{}' table", tag_text(*tag)));
            continue;
        };
        let path = extract_path(&loaded.path, out_dir.as_deref(), *tag, file.as_ref());
        if path == loaded.path {
            return Err(EngineError::Io(format!("extracting '{}' would replace the font itself", tag_text(*tag))));
        }
        if no_clobber && path.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", path.display())));
        }
        planned.push((*tag, path, data));
    }
    let req = OutputRequest::default();
    let mut files = Vec::with_capacity(planned.len());
    for (tag, path, data) in planned {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            create_dir(dir)?;
        }
        file_io::write_output(&path, data, &req)?;
        files.push(json!({ "tag": tag_text(tag), "path": path, "length": data.len() }));
    }
    Ok(files)
}

#[cfg(not(target_arch = "wasm32"))]
fn create_dir(dir: &Path) -> Result<()> {
    fontcraft_format::output::create_dir_all(dir).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", dir.display())))
}

#[cfg(target_arch = "wasm32")]
fn create_dir(_: &Path) -> Result<()> {
    Err(EngineError::Unsupported("writing files by path in the browser".into()))
}

/// The bytes of each table to add: from its file, or from another font.
fn added_tables(p: &Params<'_>, key: &str) -> Result<Vec<(Tag, Vec<u8>)>> {
    let items = p.array(key)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Some(from) = item.get("from") {
            let tag_name = item.get("tag").and_then(Value::as_str).ok_or_else(|| p.bad(format!("each item of `{key}` needs a `tag`")))?;
            let tag = tag(p, tag_name)?;
            let mut source = Map::new();
            source.insert("font".into(), from.get("font").cloned().unwrap_or(Value::Null));
            if let Some(face) = from.get("face") {
                source.insert("face".into(), face.clone());
            }
            let source = Value::Object(source);
            let sp = Params::new(p.command(), &source)?;
            let other = load(&sp, "font", "face")?;
            let data = other.font.get(tag).ok_or_else(|| tables_err(TablesError::MissingTable(tag_text(tag))))?;
            out.push((tag, data.to_vec()));
            continue;
        }
        for (tag, file) in table_files(p, key, std::slice::from_ref(item))? {
            let file = file.ok_or_else(|| p.bad(format!("'{}' in `{key}` needs a file (TAG=FILE) or `from`", tag_text(tag))))?;
            out.push((tag, file_io::read_input(&file)?.bytes));
        }
        if out.len() > fontcraft_tables::limits::MAX_TABLES {
            return Err(p.bad(format!("`{key}` names more than {} tables", fontcraft_tables::limits::MAX_TABLES)));
        }
    }
    Ok(out)
}

fn added_and_replaced(font: &SfntFont, add: &[(Tag, Vec<u8>)]) -> (Vec<String>, Vec<String>) {
    let (mut added, mut replaced) = (Vec::new(), Vec::new());
    for (tag, _) in add {
        let list = if font.tables.contains_key(tag) { &mut replaced } else { &mut added };
        if !list.contains(&tag_text(*tag)) {
            list.push(tag_text(*tag));
        }
    }
    (added, replaced)
}

/// The tags the font has, warning about the others.
fn present(font: &SfntFont, tags: Vec<Tag>, warnings: &mut Vec<String>) -> Vec<Tag> {
    let mut out = Vec::with_capacity(tags.len());
    for tag in tags {
        if !font.tables.contains_key(&tag) {
            warnings.push(format!("the font has no '{}' table", tag_text(tag)));
        } else if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn tables_extract(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.extract", p)?;
    let loaded = load(&p, "font", "face")?;
    let items = match p.get("tables") {
        Some(Value::String(s)) if s == "all" => loaded.font.tags().into_iter().map(|t| (t, None)).collect(),
        Some(Value::String(_)) => table_files(&p, "tables", std::slice::from_ref(p.get("tables").unwrap_or(&Value::Null)))?,
        Some(Value::Array(a)) => table_files(&p, "tables", a)?,
        Some(_) => return Err(p.bad("`tables` must be a list of tables or \"all\"")),
        None => return Err(p.bad("`tables` is required")),
    };
    let mut warnings = Vec::new();
    let files = write_extracted(&p, &loaded, &items, &mut warnings)?;
    Ok(json!({ "files": files, "warnings": warnings }))
}

fn tables_add(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.add", p)?;
    let loaded = load(&p, "font", "face")?;
    let add = added_tables(&p, "tables")?;
    let (added, replaced) = added_and_replaced(&loaded.font, &add);
    let options = EditOptions { add, ..EditOptions::default() };
    let result = edit(loaded.font.clone(), &options)?;
    let output = write_font(&p, &loaded, &result.font, options.effective_write_order(), ".edited")?;
    Ok(json!({ "output": output, "added": added, "replaced": replaced }))
}

fn tables_delete(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.delete", p)?;
    let loaded = load(&p, "font", "face")?;
    if p.get("tables").is_none() {
        return Err(p.bad("`tables` is required"));
    }
    let mut warnings = Vec::new();
    let delete = present(&loaded.font, tags(&p, "tables")?, &mut warnings);
    let deleted: Vec<String> = delete.iter().map(|t| tag_text(*t)).collect();
    let options = EditOptions { delete, ..EditOptions::default() };
    let result = edit(loaded.font.clone(), &options)?;
    let output = write_font(&p, &loaded, &result.font, options.effective_write_order(), ".edited")?;
    Ok(json!({ "output": output, "deleted": deleted, "warnings": warnings }))
}

fn tables_edit(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.edit", p)?;
    let loaded = load(&p, "font", "face")?;
    let extract = match p.opt_array("extract")? {
        Some(items) => table_files(&p, "extract", items)?,
        None => match p.get("extract") {
            Some(v @ Value::String(_)) => table_files(&p, "extract", std::slice::from_ref(v))?,
            _ => Vec::new(),
        },
    };
    let mut warnings = Vec::new();
    let delete = present(&loaded.font, tags(&p, "delete")?, &mut warnings);
    let add = if p.get("add").is_some() { added_tables(&p, "add")? } else { Vec::new() };
    if extract.is_empty() && delete.is_empty() && add.is_empty() && warnings.is_empty() {
        return Err(p.bad("give `extract`, `delete` or `add`"));
    }
    // The order is the reference tool's: what is added is compared with the font after deletion.
    let mut after_delete = loaded.font.clone();
    for t in &delete {
        after_delete.remove(*t);
    }
    let (added, replaced) = added_and_replaced(&after_delete, &add);
    let changes = !delete.is_empty() || !add.is_empty();
    let deleted: Vec<String> = delete.iter().map(|t| tag_text(*t)).collect();
    let options = EditOptions { delete, add, ..EditOptions::default() };
    // Check the edit before writing anything.
    let result = edit(loaded.font.clone(), &options)?;
    let bytes = if changes { Some(save(&loaded.wrap, &result.font, options.effective_write_order())?) } else { None };
    let req = OutputRequest::from_params(&p)?;
    let output = match &bytes {
        Some(_) => Some(file_io::output_path(&loaded.path, &req, ".edited", None)?),
        None => None,
    };
    let files = write_extracted(&p, &loaded, &extract, &mut warnings)?;
    if let (Some(bytes), Some(out)) = (&bytes, &output) {
        file_io::write_output(out, bytes, &req)?;
    }
    Ok(json!({ "files": files, "output": output, "deleted": deleted, "added": added, "replaced": replaced, "warnings": warnings }))
}

fn tables_checksum(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.checksum", p)?;
    let path = PathBuf::from(p.str("font")?);
    let input = file_io::read_input(&path)?;
    if !input.kind.is_font() {
        return Err(EngineError::Io(format!("{} is not a compiled font", path.display())));
    }
    let count = fontcraft_tables::sfnt::face_count(&input.bytes)?;
    let faces: Vec<u32> = match p.get("face") {
        None => (0..count).collect(),
        Some(Value::String(s)) if s == "all" => (0..count).collect(),
        Some(Value::Number(n)) => {
            vec![n.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| p.bad("`face` must be a member index, a PostScript name or \"all\""))?]
        }
        Some(Value::String(s)) if s.parse::<u32>().is_ok() => vec![s.parse::<u32>().unwrap_or_default()],
        Some(Value::String(_)) => {
            let fonts = fontcraft_tables::collection::unpack(&input.bytes)?;
            vec![u32::try_from(member(&p, "face", &fonts)?).unwrap_or(u32::MAX)]
        }
        Some(_) => return Err(p.bad("`face` must be a member index, a PostScript name or \"all\"")),
    };
    let fix = p.bool_or("fix", false)?;
    let mut ok = true;
    let mut reports = Vec::with_capacity(faces.len());
    for face in faces {
        let report = checksums(&input.bytes, face)?;
        ok &= report.ok();
        let mismatches: Vec<Value> =
            report.mismatches().map(|m| json!({ "tag": tag_text(m.tag), "stored": hex32(m.stored), "computed": hex32(m.computed) })).collect();
        let mut row = json!({ "face": face, "ok": report.ok(), "mismatches": mismatches });
        if let (Some((stored, computed)), Some(obj)) = (report.adjustment, row.as_object_mut()) {
            obj.insert("adjustment".into(), json!({ "stored": hex32(stored), "computed": hex32(computed) }));
        }
        reports.push(row);
    }
    let mut result = json!({ "ok": ok, "faces": reports });
    if fix {
        // "all" fixes the whole collection: every member is packed again.
        let loaded = if matches!(p.get("face"), Some(Value::String(s)) if s == "all") {
            let whole = json!({ "font": path });
            load(&Params::new("tables.checksum", &whole)?, "font", "face")?
        } else {
            load(&p, "font", "face")?
        };
        let output = write_font(&p, &loaded, &loaded.font, TableOrder::Recommended, ".fixed")?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("output".into(), json!(output));
        }
    }
    Ok(result)
}

fn tables_reorder(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.reorder", p)?;
    let loaded = load(&p, "font", "face")?;
    let order = match p.get("order") {
        None => None,
        Some(Value::String(s)) if s == "recommended" => None,
        Some(_) => Some(tags(&p, "order")?),
    };
    let options = EditOptions { order, ..EditOptions::default() };
    let result = edit(loaded.font.clone(), &options)?;
    let write_order = options.effective_write_order();
    let output = write_font(&p, &loaded, &result.font, write_order, ".reordered")?;
    let laid_out = match write_order {
        TableOrder::Keep => result.font.tags(),
        TableOrder::Recommended => {
            let bytes = fontcraft_tables::write(&result.font, &WriteOptions::default())?;
            fontcraft_tables::read(&bytes, 0)?.order
        }
    };
    Ok(json!({ "output": output, "order": laid_out.into_iter().map(tag_text).collect::<Vec<_>>() }))
}

fn tables_show(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.show", p)?;
    let loaded = load(&p, "font", "face")?;
    let depth = p.opt_int("depth", 0, 1_000)?.map(|d| usize::try_from(d).unwrap_or(SHOW_DEPTH)).unwrap_or(SHOW_DEPTH);
    let offsets = p.bool_or("offsets", false)?;
    let glyph_names = p.bool_or("glyphNames", true)?;
    let path = p.opt_str("path")?;
    let mut wanted = tags(&p, "tables")?;
    if let Some(path) = path {
        let first = path.split(['.', '[']).next().unwrap_or_default();
        wanted = vec![tag(&p, first)?];
    }
    // A path is looked up in the whole tree, then cut to the depth below it.
    let options = ShowOptions { tables: wanted, depth: if path.is_some() { None } else { Some(depth) }, glyph_names };
    let shown = show(&loaded.font, &options)?;
    let mut rows = Vec::with_capacity(shown.len());
    for table in &shown {
        let data = loaded.font.get(table.tag).unwrap_or_default();
        let tree = match path {
            Some(path) => {
                let mut node = find(&shown, path).map_err(|e| p.bad(e.to_string()))?.clone();
                prune(&mut node, depth);
                node
            }
            None => table.root.clone(),
        };
        let mut row = json!({
            "tag": tag_text(table.tag),
            "length": table.length,
            "checksum": hex32(fontcraft_container::sfnt::table_checksum(table.tag, data)),
            "decoded": table.decoded,
            "truncated": table.truncated,
            "tree": node_json(&tree, offsets),
        });
        if let (Some(error), Some(obj)) = (&table.error, row.as_object_mut()) {
            obj.insert("error".into(), json!(error));
        }
        rows.push(row);
    }
    Ok(json!({ "tables": rows }))
}

/// A tree node as JSON. Trees are at most a dozen levels deep (the decoders' structure), so the
/// recursion is bounded.
fn node_json(node: &ShowNode, offsets: bool) -> Value {
    let mut obj = Map::new();
    obj.insert("name".into(), json!(node.name));
    obj.insert("type".into(), json!(node.kind));
    if let Some(value) = &node.value {
        obj.insert("value".into(), json!(value));
    }
    if let (true, Some(offset)) = (offsets, node.offset) {
        obj.insert("offset".into(), json!(offset));
    }
    if !node.children.is_empty() {
        obj.insert("children".into(), Value::Array(node.children.iter().map(|c| node_json(c, offsets)).collect()));
    }
    if node.more > 0 {
        obj.insert("more".into(), json!(node.more));
    }
    Value::Object(obj)
}

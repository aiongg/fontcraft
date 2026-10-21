//! `tables.importTtx`: tables from TTX text into a compiled font (M12.8; `fontcraft_tables::ttx::import`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. The font is read and written as `tables.add` reads
//! and writes it (WOFF, WOFF2 and collection members keep their container). Each dump is
//! compiled as the reference tool compiles it with `-m`, in the order given; a dump split into
//! one file per table is read through its main file, whose `src` attributes name the parts
//! beside it, and a part may name one file per glyph (`ttx -g`). A part path must be relative and
//! stay in the folder of the file that names it, and every part passes the same file-access
//! check as the main file; the main file and its parts share the limits of one dump.
//!
//! A bitmap image kept in a file of its own (`ttx -z extfile`) is read in here, so the tables
//! crate reads no files: `ttx` names the file from the folder it ran in, and the path is taken
//! from the folder of the file that names it, else by its last three parts
//! (`bitmaps/strike<N>/<file>`, as `ttx` lays the files out) below that folder. The file must
//! resolve, links followed, inside the dump's folder, pass the file-access check and stay
//! within [`MAX_IMAGE_FILE_BYTES`]; the dump's images share [`MAX_IMAGE_FILES`] and
//! [`MAX_IMAGE_BYTES`].

use std::path::{Component, Path, PathBuf};

use fontcraft_tables::limits::{MAX_TTX_BYTES, MAX_TTX_GLYPH_PARTS, MAX_TTX_PARTS};
use fontcraft_tables::sfnt::tag_str;
use fontcraft_tables::ttx::{Element, ImportOptions, ParseBudget, import_tree_with_warnings, parse_within};
use fontcraft_tables::{TableOrder, TablesError};
use serde_json::{Value, json};

use super::file_io::{self, InputKind};
use super::tables_file::{load, write_font};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "tables.importTtx",
            "Import Tables from XML",
            [],
            None,
            "{font: path, face?: number|name, dumps: [path], recalcBounds?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} compiles the tables of each TTX dump (in the order given) and puts them in the font, replacing tables with the same tag and leaving the others untouched; glyph names resolve against the dump's GlyphOrder, else the font's own; with recalcBounds (default true) bounding boxes, maxp maxima and hhea/vhea extremes are recalculated from the outlines; the font is written in the recommended order with fresh checksums (default output: FONT.edited.EXT) → {output, added: [tag], replaced: [tag], warnings?: [string]} (warnings: a font's own CFF table that keeps its bytes instead of being compiled again, with the reason)",
            always,
            tables_import_ttx
        )
        .files(const { &[FileParam::read("font"), FileParam::read("dumps"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
    ]
}

fn tables_err(e: TablesError) -> EngineError {
    EngineError::Tables(e)
}

/// The dump at `path` as text.
fn read_text(path: &Path) -> Result<String> {
    let input = file_io::read_input(path)?;
    if input.kind != InputKind::Ttx {
        return Err(EngineError::Io(format!("{} is not TTX text (XML whose root element is ttFont)", path.display())));
    }
    String::from_utf8(input.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", path.display())))
}

/// The folder of the dump at `path`, its links resolved.
fn dump_folder(path: &Path) -> Result<PathBuf> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    dir.canonicalize().map_err(|e| EngineError::Io(format!("{}: {e}", dir.display())))
}

/// `file`, which `by` names as `src`, with its links resolved; refused when that leaves
/// `folder` (resolved), so a link inside the dump's folder cannot pull in a file from elsewhere.
fn inside_folder(folder: &Path, file: &Path, by: &Path, src: &str) -> Result<PathBuf> {
    let resolved = file.canonicalize().map_err(|e| EngineError::Io(format!("{}: {e}", file.display())))?;
    if !resolved.starts_with(folder) {
        return Err(EngineError::Io(format!("{} names the file `{src}`, which leads outside the dump's folder", by.display())));
    }
    Ok(resolved)
}

/// The most image files one dump may name.
const MAX_IMAGE_FILES: usize = 1 << 20;
/// The largest image file, in bytes.
const MAX_IMAGE_FILE_BYTES: usize = 16 << 20;
/// The most bytes the image files of one dump may hold: read in as hex, twice that, they stay
/// within the text one dump may hold.
const MAX_IMAGE_BYTES: usize = MAX_TTX_BYTES / 2;

/// What the image files of one dump have used so far.
#[derive(Default)]
struct ImageBudget {
    files: usize,
    bytes: usize,
}

/// The dump at `path` parsed, with the parts of a split dump read in where `src` names them and
/// bitmap images read in from the files that hold them.
pub(super) fn read_dump(path: &Path) -> Result<Element> {
    let mut budget = ParseBudget::default();
    let mut root = parse_within(&read_text(path)?, &mut budget).map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))?;
    let parts = root.children.iter().filter(|c| c.attribute("src").is_some()).count();
    let mut glyph_parts = 0usize;
    let mut images = ImageBudget::default();
    if parts == 0 {
        for table in &mut root.children {
            read_glyph_parts(table, path, path, &mut budget, &mut glyph_parts)?;
            read_image_files(table, path, path, &mut images)?;
        }
        return Ok(root);
    }
    if parts > MAX_TTX_PARTS {
        return Err(EngineError::Io(format!("{} names {parts} files; a split dump may name at most {MAX_TTX_PARTS}", path.display())));
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let folder = dump_folder(path)?;
    let mut children = Vec::with_capacity(root.children.len());
    for child in std::mem::take(&mut root.children) {
        let Some(src) = child.attribute("src") else {
            let mut child = child;
            read_glyph_parts(&mut child, path, path, &mut budget, &mut glyph_parts)?;
            read_image_files(&mut child, path, path, &mut images)?;
            children.push(child);
            continue;
        };
        if !Path::new(src).components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
            return Err(EngineError::Io(format!("{} names the file `{src}`: parts must be beside it or in a folder below it", path.display())));
        }
        let part_path: PathBuf = dir.join(src);
        let resolved = inside_folder(&folder, &part_path, path, src)?;
        crate::file_access::check_read(&resolved).map_err(|e| EngineError::Io(format!("refusing to read {}: {e}", part_path.display())))?;
        let mut part = parse_within(&read_text(&resolved)?, &mut budget).map_err(|e| EngineError::Io(format!("{}: {e}", part_path.display())))?;
        if part.children.iter().any(|c| c.attribute("src").is_some()) {
            return Err(EngineError::Io(format!("{} names further files at the level of tables; only the main file may", part_path.display())));
        }
        for table in &mut part.children {
            read_glyph_parts(table, &part_path, path, &mut budget, &mut glyph_parts)?;
            read_image_files(table, &part_path, path, &mut images)?;
        }
        children.extend(part.children);
    }
    root.children = children;
    Ok(root)
}

/// The elements of `table` that name a file (`ttx -g` writes one per glyph) replaced by what the
/// file holds: the last element inside its table element, as fontTools' reader takes it. The
/// file sits beside the part that names it, under the same rules as the parts, and inside the
/// folder of `main`, the dump's main file.
fn read_glyph_parts(table: &mut Element, part_path: &Path, main: &Path, budget: &mut ParseBudget, count: &mut usize) -> Result<()> {
    if !table.children.iter().any(|c| c.attribute("src").is_some()) {
        return Ok(());
    }
    let dir = part_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let folder = dump_folder(main)?;
    for child in &mut table.children {
        let Some(src) = child.attribute("src").map(str::to_string) else { continue };
        *count += 1;
        if *count > MAX_TTX_GLYPH_PARTS {
            return Err(EngineError::Io(format!("{} names more than {MAX_TTX_GLYPH_PARTS} glyph files", part_path.display())));
        }
        if !Path::new(&src).components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
            return Err(EngineError::Io(format!("{} names the file `{src}`: parts must be beside it or in a folder below it", part_path.display())));
        }
        let glyph_path = dir.join(&src);
        let resolved = inside_folder(&folder, &glyph_path, part_path, &src)?;
        crate::file_access::check_read(&resolved).map_err(|e| EngineError::Io(format!("refusing to read {}: {e}", glyph_path.display())))?;
        let glyph = parse_within(&read_text(&resolved)?, budget).map_err(|e| EngineError::Io(format!("{}: {e}", glyph_path.display())))?;
        let inner = glyph.children.into_iter().flat_map(|t| t.children).last();
        let Some(inner) = inner else {
            return Err(EngineError::Io(format!("{} holds no element inside its table", glyph_path.display())));
        };
        if inner.attribute("src").is_some() {
            return Err(EngineError::Io(format!("{} names a further file", glyph_path.display())));
        }
        *child = inner;
    }
    Ok(())
}

/// The `extfileimagedata` elements of `table`, when it is a bitmap data table, replaced by
/// `rawimagedata` holding the bytes of the file each names. `holder` is the file the table is
/// in and `main` the dump's main file, whose folder every image must lie in.
fn read_image_files(table: &mut Element, holder: &Path, main: &Path, budget: &mut ImageBudget) -> Result<()> {
    if !matches!(table.name.as_str(), "EBDT" | "CBDT" | "bdat") {
        return Ok(());
    }
    let dir = holder.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut folder = None;
    for glyph in table.children.iter_mut().flat_map(|strike| strike.children.iter_mut()) {
        for data in glyph.children.iter_mut().filter(|d| d.name == "extfileimagedata") {
            let Some(value) = data.attribute("value").map(str::to_string) else {
                return Err(EngineError::Io(format!("{} has an external bitmap image without its file", holder.display())));
            };
            budget.files += 1;
            if budget.files > MAX_IMAGE_FILES {
                return Err(EngineError::Io(format!("{} names more than {MAX_IMAGE_FILES} image files", main.display())));
            }
            if folder.is_none() {
                folder = Some(dump_folder(main)?);
            }
            let Some(root) = &folder else { continue };
            let file = image_path(&dir, &value, holder)?;
            let resolved = inside_folder(root, &file, holder, &value)?;
            crate::file_access::check_read(&resolved).map_err(|e| EngineError::Io(format!("refusing to read {}: {e}", file.display())))?;
            let bytes = file_io::read_input_up_to(&resolved, MAX_IMAGE_FILE_BYTES)?.bytes;
            if bytes.len() > MAX_IMAGE_FILE_BYTES {
                return Err(EngineError::Io(format!(
                    "{} is larger than {MAX_IMAGE_FILE_BYTES} bytes, the most an image file may hold",
                    file.display()
                )));
            }
            budget.bytes = budget.bytes.saturating_add(bytes.len());
            if budget.bytes > MAX_IMAGE_BYTES {
                return Err(EngineError::Io(format!("the image files of {} hold more than {MAX_IMAGE_BYTES} bytes", main.display())));
            }
            *data = Element { name: "rawimagedata".into(), attributes: Vec::new(), children: Vec::new(), text: hex(&bytes) };
        }
    }
    Ok(())
}

/// Where the image file `value`, named in `holder`, which lies in `dir`, is looked for: `value`
/// taken from `dir` when that is a plain path below it and something is there, else its last
/// three parts, when they are `bitmaps/strike<N>/<file>` as `ttx` writes them, below `dir`. An
/// absolute path or one that climbs out with `..` is used only through its last three parts.
fn image_path(dir: &Path, value: &str, holder: &Path) -> Result<PathBuf> {
    let given = Path::new(value);
    let plain = !value.is_empty() && given.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if plain {
        let file = dir.join(given);
        if std::fs::symlink_metadata(&file).is_ok() {
            return Ok(file);
        }
    }
    let tail: Vec<Component<'_>> = given.components().rev().take(3).collect();
    if let [Component::Normal(file), Component::Normal(strike), Component::Normal(bitmaps)] = tail.as_slice()
        && *bitmaps == "bitmaps"
        && strike.to_str().and_then(|s| s.strip_prefix("strike")).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        let file = dir.join("bitmaps").join(strike).join(file);
        if !plain || std::fs::symlink_metadata(&file).is_ok() {
            return Ok(file);
        }
    }
    if plain {
        return Ok(dir.join(given));
    }
    Err(EngineError::Io(format!("{} names the image file `{value}`, which leads outside the dump's folder", holder.display())))
}

/// `bytes` as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn tables_import_ttx(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.importTtx", p)?;
    let loaded = load(&p, "font", "face")?;
    let dumps = match p.get("dumps") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(_) => p.strings("dumps")?,
        None => return Err(p.bad("`dumps` is required")),
    };
    if dumps.is_empty() {
        return Err(p.bad("`dumps` names no file"));
    }
    let options = ImportOptions { keep_bounds: !p.bool_or("recalcBounds", true)?, ..ImportOptions::default() };
    let before: Vec<_> = loaded.font.tables.iter().map(|(t, d)| (*t, d.clone())).collect();
    let mut font = loaded.font.clone();
    let mut warnings: Vec<String> = Vec::new();
    for dump in &dumps {
        let root = read_dump(Path::new(dump))?;
        let (merged, notes) = import_tree_with_warnings(font, &root, &options).map_err(|e| match e {
            TablesError::Invalid(m) => EngineError::Tables(TablesError::Invalid(format!("{dump}: {m}"))),
            other => tables_err(other),
        })?;
        font = merged;
        warnings.extend(notes.into_iter().map(|w| format!("{dump}: {w}")));
    }
    let mut added = Vec::new();
    let mut replaced = Vec::new();
    for (tag, data) in &font.tables {
        match before.iter().find(|(t, _)| t == tag) {
            None => added.push(tag_str(*tag).trim_end().to_string()),
            Some((_, old)) if old != data => replaced.push(tag_str(*tag).trim_end().to_string()),
            Some(_) => {}
        }
    }
    let output = write_font(&p, &loaded, &font, TableOrder::Recommended, ".edited")?;
    let mut result = json!({ "output": output, "added": added, "replaced": replaced });
    if !warnings.is_empty()
        && let Some(o) = result.as_object_mut()
    {
        o.insert("warnings".into(), json!(warnings));
    }
    Ok(result)
}

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

use std::path::{Component, Path, PathBuf};

use fontcraft_tables::limits::{MAX_TTX_GLYPH_PARTS, MAX_TTX_PARTS};
use fontcraft_tables::sfnt::tag_str;
use fontcraft_tables::ttx::{Element, ImportOptions, ParseBudget, import_tree, parse_within};
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
            "{font: path, face?: number|name, dumps: [path], recalcBounds?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} compiles the tables of each TTX dump (in the order given) and puts them in the font, replacing tables with the same tag and leaving the others untouched; glyph names resolve against the dump's GlyphOrder, else the font's own; with recalcBounds (default true) bounding boxes, maxp maxima and hhea/vhea extremes are recalculated from the outlines; the font is written in the recommended order with fresh checksums (default output: FONT.edited.EXT) → {output, added: [tag], replaced: [tag]}",
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

/// The dump at `path` parsed, with the parts of a split dump read in where `src` names them.
fn read_dump(path: &Path) -> Result<Element> {
    let mut budget = ParseBudget::default();
    let mut root = parse_within(&read_text(path)?, &mut budget).map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))?;
    let parts = root.children.iter().filter(|c| c.attribute("src").is_some()).count();
    let mut glyph_parts = 0usize;
    if parts == 0 {
        for table in &mut root.children {
            read_glyph_parts(table, path, path, &mut budget, &mut glyph_parts)?;
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
    for dump in &dumps {
        let root = read_dump(Path::new(dump))?;
        font = import_tree(font, &root, &options).map_err(|e| match e {
            TablesError::Invalid(m) => EngineError::Tables(TablesError::Invalid(format!("{dump}: {m}"))),
            other => tables_err(other),
        })?;
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
    Ok(json!({ "output": output, "added": added, "replaced": replaced }))
}

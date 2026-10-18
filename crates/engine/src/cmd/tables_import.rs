//! `tables.importTtx`: tables from TTX text into a compiled font (M12.8; `fontcraft_tables::ttx::import`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. The font is read and written as `tables.add` reads
//! and writes it (WOFF, WOFF2 and collection members keep their container). Each dump is
//! compiled as the reference tool compiles it with `-m`, in the order given; a dump split into
//! one file per table is read through its main file, whose `src` attributes name the parts
//! beside it.

use std::path::{Path, PathBuf};

use fontcraft_tables::sfnt::tag_str;
use fontcraft_tables::ttx::{Element, ImportOptions, import_tree, parse};
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

/// The dump at `path` parsed, with the parts of a split dump read in where `src` names them.
fn read_dump(path: &Path) -> Result<Element> {
    let mut root = parse(&read_text(path)?).map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))?;
    if root.children.iter().all(|c| c.attribute("src").is_none()) {
        return Ok(root);
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut children = Vec::with_capacity(root.children.len());
    for child in std::mem::take(&mut root.children) {
        let Some(src) = child.attribute("src") else {
            children.push(child);
            continue;
        };
        let part_path: PathBuf = dir.join(src);
        let part = parse(&read_text(&part_path)?).map_err(|e| EngineError::Io(format!("{}: {e}", part_path.display())))?;
        if part.children.iter().any(|c| c.attribute("src").is_some()) {
            return Err(EngineError::Io(format!("{} names further files; dumps split by glyph are not read", part_path.display())));
        }
        children.extend(part.children);
    }
    root.children = children;
    Ok(root)
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

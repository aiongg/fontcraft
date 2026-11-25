//! The codecs `cargo xtask roundtrip` runs, by mode and source kind.
//!
//! - `plist`: the generic property-list layer alone (`fontcraft_plist`), no model. Every property
//!   list in a source is parsed and written back; other files in a package are copied. This is
//!   the M1.1 measure: the layer must not lose or respell anything. Each written file is read
//!   again and must mean the same as the original, so a "different" result is layout only
//!   (indentation, blank lines, spaces a hand edit left), never lost data.
//!   For UFOs and designspace files the layer is the XML tree (`fontcraft_plist::xml`, every
//!   file) and XML property lists (`fontcraft_plist::xmlplist`, every `.plist`).
//! - `model`: through the font model: read into a `Font`, written back from it (`fontcraft_io_glyphs`
//!   for `.glyphs`, `fontcraft_io_ufo` for UFOs and designspace files; a designspace is read with
//!   its UFOs and only the document is compared, the UFOs being samples of their own). The
//!   `spellings` count is not measured in this mode.
//! - `native`: through the native format as well: read into a `Font`, saved as `.fontcraft`
//!   bytes, loaded again, written back (`roundtrip.nativeLosslessFromGlyphs`).
//! - `stable`: what `model` writes, read and written again, gives the same bytes; it compares two
//!   outputs, not the original, and lives in `stable.rs`.

use std::path::Path;

use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Num, Value};

use crate::roundtrip::{Codec, Coded, Kind, Written, read_tree};

pub const MODES: [&str; 4] = ["plist", "model", "native", crate::stable::MODE];

pub fn codec(mode: &str, kind: Kind) -> Option<Codec> {
    match (mode, kind) {
        ("plist", Kind::Glyphs) => Some(glyphs_file),
        ("plist", Kind::GlyphsPackage) => Some(glyphs_package),
        ("model", Kind::Glyphs) => Some(glyphs_file_model),
        ("model", Kind::GlyphsPackage) => Some(glyphs_package_model),
        ("native", Kind::Glyphs) => Some(glyphs_file_native),
        ("native", Kind::GlyphsPackage) => Some(glyphs_package_native),
        ("plist", Kind::Ufo) => Some(ufo_plist),
        ("plist", Kind::Designspace) => Some(designspace_xml),
        ("model", Kind::Ufo) => Some(ufo_model),
        ("model", Kind::Designspace) => Some(designspace_model),
        ("native", Kind::Ufo) => Some(ufo_native),
        ("native", Kind::Designspace) => Some(designspace_native),
        _ => None,
    }
}

fn dialect_of(root: &Value) -> Dialect {
    let version = root.as_dict().and_then(|d| d.get(".formatVersion")).and_then(Value::as_num).map(Num::as_f64);
    if version == Some(4.0) { Dialect::Glyphs4 } else { Dialect::Glyphs3 }
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    String::from_utf8(bytes).map_err(|e| format!("{}: not UTF-8: {e}", path.display()))
}

/// Writes `doc` and checks that the text reads back to the same values.
fn write_checked(doc: &openstep::Document, dialect: Dialect) -> Result<String, String> {
    let out = openstep::write(doc, dialect);
    let again = openstep::parse(&out).map_err(|e| format!("the written text does not parse: {e}"))?;
    if again.root != doc.root {
        return Err("the written text reads back to different values".into());
    }
    Ok(out)
}

fn glyphs_file(path: &Path) -> Result<Coded, String> {
    let doc = openstep::parse(&read_text(path)?).map_err(|e| e.to_string())?;
    let out = write_checked(&doc, dialect_of(&doc.root))?;
    Ok(Coded { written: Written::File(out.into_bytes()), spellings: doc.root.spelling_count() })
}

fn glyphs_package(path: &Path) -> Result<Coded, String> {
    let info = openstep::parse(&read_text(&path.join("fontinfo.plist"))?).map_err(|e| format!("fontinfo.plist: {e}"))?;
    let dialect = dialect_of(&info.root);
    let mut tree = read_tree(path)?;
    let mut spellings = 0;
    for (name, bytes) in &mut tree {
        if !(name.ends_with(".plist") || name.ends_with(".glyph")) {
            continue;
        }
        let text = String::from_utf8(std::mem::take(bytes)).map_err(|e| format!("{name}: not UTF-8: {e}"))?;
        let doc = openstep::parse(&text).map_err(|e| format!("{name}: {e}"))?;
        spellings += doc.root.spelling_count();
        *bytes = write_checked(&doc, dialect).map_err(|e| format!("{name}: {e}"))?.into_bytes();
    }
    Ok(Coded { written: Written::Tree(tree), spellings })
}

fn glyphs_file_model(path: &Path) -> Result<Coded, String> {
    let font = fontcraft_io_glyphs::read(&read_text(path)?).map_err(|e| e.to_string())?;
    let out = fontcraft_io_glyphs::write(&font).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::File(out.into_bytes()), spellings: 0 })
}

fn glyphs_package_model(path: &Path) -> Result<Coded, String> {
    let files = read_tree(path)?;
    let font = fontcraft_io_glyphs::read_package(&files).map_err(|e| e.to_string())?;
    let out = fontcraft_io_glyphs::write_package(&font).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::Tree(out), spellings: 0 })
}

/// A font through the native format and back.
fn via_native(font: &fontcraft_engine::model::Font) -> Result<fontcraft_engine::model::Font, String> {
    let bytes = fontcraft_engine::format::save(font).map_err(|e| format!("saving .fontcraft: {e}"))?;
    let back = fontcraft_engine::format::load(&bytes).map_err(|e| format!("loading .fontcraft: {e}"))?;
    if !back.same_as(font) {
        return Err("the .fontcraft file reads back to a different font".into());
    }
    Ok(back)
}

fn glyphs_file_native(path: &Path) -> Result<Coded, String> {
    let font = fontcraft_io_glyphs::read(&read_text(path)?).map_err(|e| e.to_string())?;
    let out = fontcraft_io_glyphs::write(&via_native(&font)?).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::File(out.into_bytes()), spellings: 0 })
}

fn glyphs_package_native(path: &Path) -> Result<Coded, String> {
    let font = fontcraft_io_glyphs::read_package(&read_tree(path)?).map_err(|e| e.to_string())?;
    let out = fontcraft_io_glyphs::write_package(&via_native(&font)?).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::Tree(out), spellings: 0 })
}

fn ufo_plist(path: &Path) -> Result<Coded, String> {
    use fontcraft_plist::{xml, xmlplist};
    let mut tree = read_tree(path)?;
    let mut spellings = 0;
    for (name, bytes) in &mut tree {
        let is_plist = name.ends_with(".plist");
        if !(is_plist || name.ends_with(".glif")) {
            continue;
        }
        let text = String::from_utf8(std::mem::take(bytes)).map_err(|e| format!("{name}: not UTF-8: {e}"))?;
        let out = if is_plist {
            let doc = xmlplist::parse(&text).map_err(|e| format!("{name}: {e}"))?;
            spellings += doc.root.spelling_count();
            xmlplist::write(&doc)
        } else {
            xml::parse(&text).map_err(|e| format!("{name}: {e}"))?.write()
        };
        *bytes = out.into_bytes();
    }
    Ok(Coded { written: Written::Tree(tree), spellings })
}

fn designspace_xml(path: &Path) -> Result<Coded, String> {
    let doc = fontcraft_plist::xml::parse(&read_text(path)?).map_err(|e| e.to_string())?;
    Ok(Coded { written: Written::File(doc.write().into_bytes()), spellings: 0 })
}

fn ufo_model(path: &Path) -> Result<Coded, String> {
    let font = fontcraft_io_ufo::ufo::read(&read_tree(path)?).map_err(|e| e.to_string())?;
    let out = fontcraft_io_ufo::ufo::write(&font).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::Tree(out), spellings: 0 })
}

/// A designspace read with the UFOs it names, relative to its folder.
fn read_designspace(path: &Path) -> Result<fontcraft_engine::model::Font, String> {
    let text = read_text(path)?;
    let dir = path.parent().ok_or("a designspace without a folder")?;
    let mut ufos = std::collections::BTreeMap::new();
    for rel in fontcraft_io_ufo::designspace::ufo_paths(&text).map_err(|e| e.to_string())? {
        let ufo = dir.join(&rel);
        if !ufo.join("metainfo.plist").is_file() {
            return Err(format!("the UFO `{rel}` is not in the corpus"));
        }
        ufos.insert(rel, read_tree(&ufo)?);
    }
    fontcraft_io_ufo::designspace::read(&text, &ufos).map_err(|e| e.to_string())
}

fn designspace_model(path: &Path) -> Result<Coded, String> {
    let out = fontcraft_io_ufo::designspace::write(&read_designspace(path)?).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::File(out.document.into_bytes()), spellings: 0 })
}

fn ufo_native(path: &Path) -> Result<Coded, String> {
    let font = fontcraft_io_ufo::ufo::read(&read_tree(path)?).map_err(|e| e.to_string())?;
    let out = fontcraft_io_ufo::ufo::write(&via_native(&font)?).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::Tree(out), spellings: 0 })
}

fn designspace_native(path: &Path) -> Result<Coded, String> {
    let out = fontcraft_io_ufo::designspace::write(&via_native(&read_designspace(path)?)?).map_err(|e| format!("writing: {e}"))?;
    Ok(Coded { written: Written::File(out.document.into_bytes()), spellings: 0 })
}

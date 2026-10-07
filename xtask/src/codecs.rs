//! The codecs `cargo xtask roundtrip` runs, by mode and source kind.
//!
//! - `plist`: the generic property-list layer alone (`fontcraft_plist`), no model. Every property
//!   list in a source is parsed and written back; other files in a package are copied. This is
//!   the M1.1 measure: the layer must not lose or respell anything. Each written file is read
//!   again and must mean the same as the original, so a "different" result is layout only
//!   (indentation, blank lines, spaces a hand edit left), never lost data.
//! - `model`: through the font model: read into a `Font`, written back from it (`fontcraft_io_glyphs`
//!   for `.glyphs`). The `spellings` count is not measured in this mode.

use std::path::Path;

use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Num, Value};

use crate::roundtrip::{Codec, Coded, Kind, Written, read_tree};

pub const MODES: [&str; 2] = ["plist", "model"];

pub fn codec(mode: &str, kind: Kind) -> Option<Codec> {
    match (mode, kind) {
        ("plist", Kind::Glyphs) => Some(glyphs_file),
        ("plist", Kind::GlyphsPackage) => Some(glyphs_package),
        ("model", Kind::Glyphs) => Some(glyphs_file_model),
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

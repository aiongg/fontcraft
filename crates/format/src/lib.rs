//! The native `.fontcraft` format (L3).
//!
//! A `.fontcraft` file is UTF-8 JSON:
//! ```json
//! { "format": "fontcraft", "version": 1, "generator": "FontCraft 0.0.1",
//!   "font": { …fontcraft_model::Font… } }
//! ```
//! It is lossless for everything in the model: keys this version does not know are kept in each
//! struct's `extra` map and written back. Readers reject a `version` newer than they support and
//! files larger than [`MAX_BYTES`]. Keys are written in a stable order with two-space indentation,
//! so files diff well under version control (`docs/spec/30-source-formats.md` › round-trip rules).
//!
//! Planned (see ROADMAP.md, M1): optional gzip, an embedded preview, package (directory) form.
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod atomic;
mod gzip;
mod layout;
mod v1;

use fontcraft_model::Font;
use serde::Deserialize;

#[cfg(not(target_arch = "wasm32"))]
pub use atomic::write_atomic;

/// The version this build writes. Older versions are upgraded on load, one step at a time.
pub const VERSION: u32 = 2;
pub const FORMAT: &str = "fontcraft";
pub const EXTENSION: &str = "fontcraft";
/// Largest file [`load`] accepts (a guard against hostile input).
pub const MAX_BYTES: usize = 512 << 20;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("not a FontCraft file: {0}")]
    NotFontcraft(String),
    #[error("this file was written by a newer FontCraft (format version {found}; this build reads up to {supported})")]
    TooNew { found: u32, supported: u32 },
    #[error("the file is too large ({0} bytes)")]
    TooLarge(usize),
    #[error("the font in the file is not valid: {0}")]
    Invalid(#[from] fontcraft_model::ModelError),
    #[error("cannot write the file: {0}")]
    Encode(String),
    #[error("{0}")]
    Io(String),
}

/// The file as read: the header, and the font's JSON left unparsed until the version says which
/// shape it has. The file is tokenised once and the font built once.
#[derive(Deserialize)]
struct FileIn<'a> {
    #[serde(default)]
    format: String,
    #[serde(default)]
    version: u32,
    #[serde(borrow)]
    font: Option<&'a serde_json::value::RawValue>,
}

/// The bytes of a `.fontcraft` file holding `font`.
pub fn save(font: &Font) -> Result<Vec<u8>, FormatError> {
    let value = serde_json::to_value(font).map_err(|e| FormatError::Encode(e.to_string()))?;
    let generator = serde_json::Value::String(format!("FontCraft {}", env!("CARGO_PKG_VERSION")));
    // The header first, by hand, so a reader sees the format and version at the top.
    let mut out = format!("{{\n  \"format\": \"{FORMAT}\",\n  \"version\": {VERSION},\n  \"generator\": {generator},\n  \"font\": ");
    layout::write(&mut out, &value, 1);
    out.push_str("\n}\n");
    // A file read compressed is written compressed (`roundtrip.nativeGzip`).
    Ok(if font.source.compressed { gzip::compress(out.as_bytes()) } else { out.into_bytes() })
}

/// Reads a `.fontcraft` file and checks the font it holds.
pub fn load(bytes: &[u8]) -> Result<Font, FormatError> {
    if bytes.len() > MAX_BYTES {
        return Err(FormatError::TooLarge(bytes.len()));
    }
    let inflated;
    let compressed = gzip::is_gzip(bytes);
    let bytes = if compressed {
        inflated = gzip::decompress(bytes)?;
        inflated.as_slice()
    } else {
        bytes
    };
    let file: FileIn = serde_json::from_slice(bytes).map_err(|e| FormatError::NotFontcraft(e.to_string()))?;
    if file.format != FORMAT {
        return Err(FormatError::NotFontcraft(format!("the `format` key is `{}`, expected `{FORMAT}`", file.format)));
    }
    if file.version > VERSION {
        return Err(FormatError::TooNew { found: file.version, supported: VERSION });
    }
    let raw = file.font.ok_or_else(|| FormatError::NotFontcraft("the file has no `font`".into()))?.get();
    let bad = |e: serde_json::Error| FormatError::NotFontcraft(e.to_string());
    let font = match file.version {
        0 | 1 => serde_json::from_str::<v1::Font>(raw).map_err(bad)?.upgrade(),
        _ => serde_json::from_str::<Font>(raw).map_err(bad)?,
    };
    let mut font = font;
    font.source.compressed = compressed;
    font.check()?;
    Ok(font)
}

/// Whether `bytes` are a gzip-compressed file (a compressed native file starts this way).
pub fn is_compressed(bytes: &[u8]) -> bool {
    gzip::is_gzip(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_round_trips_exactly() {
        let font = Font::new("Round Trip");
        let bytes = save(&font).unwrap();
        assert_eq!(load(&bytes).unwrap(), font);
        // Saving what was loaded gives the same bytes: the format is stable.
        assert_eq!(save(&load(&bytes).unwrap()).unwrap(), bytes);
    }

    #[test]
    fn stems_guides_zones_and_instance_fields_round_trip() {
        use fontcraft_model::{AlignmentZone, Guide, Instance, NameParts, NumberDef, StemDef, StyleClass};
        use std::sync::Arc;
        let mut font = Font::new("Typed");
        font.stems = Arc::new(vec![
            StemDef { name: "stem".into(), ..Default::default() },
            StemDef { horizontal: true, filter: Some("x".into()), ..Default::default() },
        ]);
        font.numbers = Arc::new(vec![NumberDef { name: "gap".into(), ..Default::default() }]);
        let m = &mut Arc::make_mut(&mut font.masters)[0];
        m.stem_values = vec![80.0, 60.5];
        m.number_values = vec![-0.0];
        m.zones = vec![AlignmentZone { pos: 400.0, size: -8.0 }];
        m.guides = vec![Guide { x: 1.0, y: 2.5, angle: 90.0, angle_locked: true, orientation: Some("right".into()), ..Default::default() }];
        m.name_parts = NameParts { weight: Some("Bold".into()), ..Default::default() };
        let bold = Instance {
            name: "Bold".into(),
            visible: false,
            weight_class: Some(StyleClass::Name("Bold".into())),
            width_class: Some(StyleClass::Number(5.0)),
            is_bold: true,
            link_style: Some("Regular".into()),
            interpolation: vec![("m01".into(), 0.25)],
            manual_interpolation: true,
            id: Some("i1".into()),
            kind: Some("variable".into()),
            ..Default::default()
        };
        font.instances = Arc::new(vec![bold, Instance::default()]);
        let bytes = save(&font).unwrap();
        assert_eq!(load(&bytes).unwrap(), font);
        assert_eq!(save(&load(&bytes).unwrap()).unwrap(), bytes);
    }

    #[test]
    fn newer_and_foreign_files_are_rejected_with_a_reason() {
        assert!(matches!(load(br#"{"format":"fontcraft","version":99,"font":{}}"#), Err(FormatError::TooNew { found: 99, .. })));
        assert!(matches!(load(br#"{"format":"other","version":1}"#), Err(FormatError::NotFontcraft(_))));
        assert!(matches!(load(b"\x00\x01\x00\x00"), Err(FormatError::NotFontcraft(_))));
        assert!(matches!(load(b"[1,2,3]"), Err(FormatError::NotFontcraft(_))));
        assert!(matches!(load(br#"{"format":"fontcraft","version":1}"#), Err(FormatError::NotFontcraft(_))));
    }

    #[test]
    fn a_version_1_file_is_upgraded() {
        // Written by the M0 build: fixed master metrics, layers without ids, matrix transforms,
        // unknown keys beside known ones.
        let v1 = br#"{"format":"fontcraft","version":1,"generator":"FontCraft 0.0.1","font":{
            "family_name":"Old","units_per_em":1000,"version_major":1,"version_minor":0,"fromTheFuture":{"k":[1,true,null,2.5]},
            "masters":[{"id":"m01","name":"Regular","ascender":800,"cap_height":700,"x_height":500,"descender":-200}],
            "glyphs":[{"name":"A","unicodes":[65],"layers":[{"master":"m01","width":600,
                "shapes":[{"path":{"closed":true,"nodes":[{"x":0,"y":0,"type":"line","name":"start"},{"x":100,"y":0,"type":"line"},{"x":50,"y":100,"type":"line"}]}}]}]},
              {"name":"B","layers":[{"master":"m01","width":600,"shapes":[{"component":{"glyph":"A","transform":[1,0,0,1,10,0]}}],"anchors":[{"name":"top","x":1,"y":2}]}]}]}}"#;
        let font = load(v1).unwrap();
        let m = font.default_master().unwrap();
        assert_eq!(font.metric_pos(m, fontcraft_model::metric::X_HEIGHT, 0.0), 500.0);
        let a = font.glyph("A").unwrap();
        assert_eq!(a.layers[0].id, "m01");
        assert!(a.layer("m01").is_some());
        let fontcraft_model::Shape::Path(p) = &a.layers[0].shapes[0] else { panic!() };
        assert_eq!(p.nodes[0].extra.get(fontcraft_model::Family::Native, "name").and_then(|v| v.as_str()), Some("start"));
        let b = font.glyph("B").unwrap();
        let fontcraft_model::Shape::Component(c) = &b.layers[0].shapes[0] else { panic!() };
        assert_eq!(c.transform, fontcraft_model::Transform::Matrix([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]));
        assert!(font.extra.get(fontcraft_model::Family::Native, "fromTheFuture").is_some());
        // Saved again, it is a version 2 file that loads to the same font.
        let again = save(&font).unwrap();
        assert!(String::from_utf8_lossy(&again).contains("\"version\": 2"));
        assert_eq!(load(&again).unwrap(), font);
    }

    #[test]
    fn an_invalid_font_is_rejected_on_load() {
        let mut font = Font::new("Bad");
        font.units_per_em = 1;
        let bytes = save(&font).unwrap();
        assert!(matches!(load(&bytes), Err(FormatError::Invalid(_))));
    }

    #[test]
    fn garbage_never_panics() {
        let good = save(&Font::new("Fuzz")).unwrap();
        // Truncations and single-byte corruptions of a valid file.
        for cut in (0..good.len()).step_by(97) {
            let _ = load(good.get(..cut).unwrap_or_default());
        }
        for i in (0..good.len()).step_by(53) {
            let mut bad = good.clone();
            if let Some(b) = bad.get_mut(i) {
                *b = b.wrapping_add(13);
            }
            let _ = load(&bad);
        }
    }
}

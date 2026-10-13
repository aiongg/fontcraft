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
pub mod project;
mod v1;

use fontcraft_model::Font;
use serde::Deserialize;

#[cfg(not(target_arch = "wasm32"))]
pub use atomic::write_atomic;

/// The version this build writes. Older versions are upgraded on load, one step at a time.
pub const VERSION: u32 = 3;
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
    if file.version <= 2 && font.source.glyphs_version != Some(4) {
        upgrade_stroke_positions(&mut font);
    }
    font.source.compressed = compressed;
    font.check()?;
    Ok(font)
}

/// Version 3 stores every stroke position from −1 (inside) to 1 (outside). Earlier versions stored
/// it as the font's `.glyphs` format version counts it, which for anything but version 4 is 0
/// outside, 0.5 centred, 1 inside. An expression that is not a plain number is kept.
fn upgrade_stroke_positions(font: &mut Font) {
    use fontcraft_model::{Dimension, Shape};
    let convert = |d: &mut Option<Dimension>| match d {
        Some(Dimension::Number(p)) => *p = 1.0 - 2.0 * *p,
        Some(Dimension::Expression(e)) => {
            if let Some(p) = e.trim().parse::<f64>().ok().filter(|p| p.is_finite()) {
                *e = format!("{}", 1.0 - 2.0 * p);
            }
        }
        None => {}
    };
    let has = |s: &Shape| match s {
        Shape::Path(p) => p.info.attrs.stroke_pos.is_some(),
        Shape::Component(c) => c.info.attrs.stroke_pos.is_some(),
        _ => false,
    };
    let fix = |shapes: &mut Vec<Shape>| {
        for s in shapes.iter_mut().filter(|s| has(s)) {
            match s {
                Shape::Path(p) => convert(&mut p.info.attrs.stroke_pos),
                Shape::Component(c) => convert(&mut c.info.attrs.stroke_pos),
                _ => {}
            }
        }
    };
    let glyphs = font.glyphs.as_arcs().iter().map(|g| {
        let touched = g.layers.iter().any(|l| l.shapes.iter().chain(l.info.background.iter().flat_map(|b| b.shapes.iter())).any(has));
        if !touched {
            return g.clone();
        }
        let mut g = (**g).clone();
        for l in &mut g.layers {
            fix(&mut l.shapes);
            if l.info.background.as_ref().is_some_and(|b| b.shapes.iter().any(has))
                && let Some(b) = l.info.background.as_mut()
            {
                fix(&mut b.shapes);
            }
        }
        std::sync::Arc::new(g)
    });
    font.glyphs = fontcraft_model::GlyphTable::from_glyphs(glyphs.collect());
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
    fn a_version_2_file_has_its_stroke_positions_converted() {
        use fontcraft_model::{Dimension, Glyph, Layer, Node, NodeKind, Path, Shape};
        let mut font = Font::new("Strokes");
        let master = font.masters.first().map(|m| m.id.clone()).unwrap();
        let mut path = Path::new(false, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(100.0, 0.0, NodeKind::Line)]);
        path.info.attrs.stroke_pos = Some(Dimension::Number(0.0));
        let mut layer = Layer::new(master, 500.0);
        layer.shapes.push(Shape::Path(path));
        let mut glyph = Glyph::new("stroked");
        glyph.layers.push(layer);
        font.glyphs.push(glyph);
        let pos = |f: &Font| match f.glyphs.get("stroked").and_then(|g| g.layers.first()).and_then(|l| l.shapes.first()) {
            Some(Shape::Path(p)) => p.info.attrs.stroke_pos.clone(),
            _ => None,
        };
        let text = String::from_utf8(save(&font).unwrap()).unwrap();
        assert_eq!(pos(&load(text.as_bytes()).unwrap()), Some(Dimension::Number(0.0)), "version 3 keeps the value");
        let old = text.replacen("\"version\": 3", "\"version\": 2", 1);
        // Version 2 stored 0 for outside in a font not from a version 4 file.
        assert_eq!(pos(&load(old.as_bytes()).unwrap()), Some(Dimension::Number(1.0)));
        font.source.glyphs_version = Some(4);
        let v4 = String::from_utf8(save(&font).unwrap()).unwrap().replacen("\"version\": 3", "\"version\": 2", 1);
        assert_eq!(pos(&load(v4.as_bytes()).unwrap()), Some(Dimension::Number(0.0)));
    }

    #[test]
    fn the_glyphs_v2_kerning_flag_is_kept_and_stored_only_when_set() {
        let plain = save(&Font::new("Kerning")).unwrap();
        assert!(!String::from_utf8_lossy(&plain).contains("glyphsV2Kerning"));
        let mut font = Font::new("Kerning");
        font.source.glyphs_v2_kerning = true;
        assert!(load(&save(&font).unwrap()).unwrap().source.glyphs_v2_kerning);
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
    fn designspace_axis_fields_round_trip() {
        use fontcraft_model::{Axis, AxisLabel, AxisRange, LangName};
        use std::sync::Arc;
        let mut font = Font::new("Axes");
        let de = |n: &str| LangName { language: "de".into(), name: n.into() };
        font.axes = Arc::new(vec![
            Axis {
                name: "Weight".into(),
                tag: "wght".into(),
                range: Some(AxisRange { minimum: 100.0, default: 400.0, maximum: 900.5 }),
                label_names: vec![de("Gewicht")],
                labels: vec![AxisLabel {
                    name: "Bold".into(),
                    user_value: 700.0,
                    user_minimum: Some(650.0),
                    linked_user_value: Some(400.0),
                    elidable: true,
                    older_sibling: true,
                    names: vec![de("Fett")],
                    ..Default::default()
                }],
                ordering: Some(2),
                ..Default::default()
            },
            Axis { name: "Italic".into(), tag: "ital".into(), hidden: true, values: vec![0.0, 1.0], ..Default::default() },
        ]);
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0, 0.0];
        let bytes = save(&font).unwrap();
        assert_eq!(load(&bytes).unwrap(), font);
        assert_eq!(save(&load(&bytes).unwrap()).unwrap(), bytes);
    }

    #[test]
    fn glyph_metadata_round_trips() {
        use fontcraft_model::plist::{Dict, Value};
        use fontcraft_model::{ColorLabel, PartProperty, UserData};
        let mut font = Font::new("Meta");
        {
            let mut a = font.glyph_mut("A").unwrap();
            let i = &mut *a.info;
            i.production = Some("uni0041".into());
            i.category = Some("Letter".into());
            i.case = Some("upper".into());
            i.opentype_category = Some("base".into());
            i.metrics_keys.left = Some("=H".into());
            i.metrics_keys.vert_origin = Some("n".into());
            i.color = Some(ColorLabel::Rgba([1.0, 128.0 / 255.0, 0.0, 1.0]));
            i.tags = vec!["new".into()];
            i.note = "two\nlines".into();
            i.locked = true;
            i.last_change = Some("2026-10-08 12:00:00 +0000".into());
            i.sort_name = Some("XA".into());
            i.parts = vec![PartProperty {
                name: "Width".into(),
                bottom_value: 0.0,
                top_value: 100.0,
                bottom_name: Some("Low".into()),
                top_name: None,
                extra: Default::default(),
            }];
            let mut d = Dict::new();
            d.insert("flag", Value::Bool(true));
            i.user_data = UserData(d);
        }
        font.glyph_mut("B").unwrap().info.color = Some(ColorLabel::Index(3));
        let bytes = save(&font).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("\"info\"").count(), 2, "only glyphs with metadata have it");
        assert_eq!(load(&bytes).unwrap(), font);
        assert_eq!(save(&load(&bytes).unwrap()).unwrap(), bytes);
        // Out-of-range values are refused on load.
        let bad = text.replace("\"index\": 3", "\"rgba\": [2, 0, 0, 1]");
        assert!(matches!(load(bad.as_bytes()), Err(FormatError::Invalid(_))), "{bad}");
    }

    #[test]
    fn kerning_context_feature_labels_and_groups_round_trip() {
        use fontcraft_model::{Group, KernContext, Localized, Snippet};
        use std::sync::Arc;
        let mut font = Font::new("Features");
        Arc::make_mut(&mut font.kerning).context.push(KernContext { context: "A V | T".into(), values: vec![("m01".into(), -12.5)] });
        Arc::make_mut(&mut font.features).features.push(Snippet {
            name: "ss01".into(),
            code: "sub a by a.alt;".into(),
            automatic: true,
            labels: vec![Localized { language: "DEU".into(), value: "Alternativ".into(), extra: Default::default() }],
            ..Default::default()
        });
        Arc::make_mut(&mut font.groups).push(Group { name: "round".into(), members: vec!["O".into(), "Q".into()] });
        let bytes = save(&font).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("\"context\"") && text.contains("\"labels\"") && text.contains("\"groups\""), "{text}");
        assert_eq!(load(&bytes).unwrap(), font);
        assert_eq!(save(&load(&bytes).unwrap()).unwrap(), bytes);
        // Empty fields are left out.
        let plain = String::from_utf8(save(&Font::new("Plain")).unwrap()).unwrap();
        assert!(!plain.contains("\"groups\"") && !plain.contains("\"labels\"") && !plain.contains("\"context\""));
    }

    #[test]
    fn layer_data_round_trips() {
        use fontcraft_model::plist::{Dict, Value};
        use fontcraft_model::{
            Anchor, Annotation, AxisRule, Background, ColorLabel, Extra, Family, Guide, Hint, Image, Layer, PaletteEntry, Path, Shape, SmartPole,
            Transform, UserData,
        };
        let mut font = Font::new("Layers");
        {
            let mut a = font.glyph_mut("A").unwrap();
            let master = a.layers[0].master.clone();
            let mut backup = Layer::new(master, 500.0);
            backup.id = "L2".into();
            let i = &mut *backup.info;
            i.name = Some("{550}".into());
            i.attrs.coordinates = vec![Some(550.0), None];
            i.attrs.axis_rules = vec![AxisRule { min: Some(100.0), max: None }];
            i.attrs.color_palette = Some(PaletteEntry::Foreground);
            i.attrs.smart_poles = vec![SmartPole { property: "Width".into(), top: true }];
            i.vert_width = Some(1000.0);
            i.vert_origin = Some(-20.0);
            i.metrics_keys.right = Some("=|".into());
            i.color = Some(ColorLabel::Index(2));
            i.visible = true;
            i.guides = vec![Guide { x: 10.0, angle: 90.0, ..Default::default() }];
            let mut hint = Dict::new();
            hint.insert("type", Value::String("Stem".into()));
            i.hints = vec![Hint { family: Family::Glyphs, value: Value::Dict(hint) }];
            i.annotations =
                vec![Annotation { kind: "Text".into(), x: 1.0, y: 2.0, angle: 0.0, text: Some("hi".into()), width: 30.0, extra: Extra::new() }];
            i.image = Some(Image {
                path: Some("a.png".into()),
                transform: Transform::Matrix([1.0, 0.0, 0.0, 1.0, 5.0, 6.0]),
                crop: Some([0.0, 0.0, 10.0, 10.0]),
                alpha: Some(40.0),
                locked: true,
                extra: Extra::new(),
            });
            i.background = Some(Background {
                shapes: vec![Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0))],
                anchors: vec![Anchor::new("top", 5.0, 10.0)],
                ..Default::default()
            });
            let mut d = Dict::new();
            d.insert("k", Value::Bool(true));
            i.user_data = UserData(d);
            a.layers.push(backup);
        }
        let bytes = save(&font).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("\"info\"").count(), 1, "only layers with data have it");
        assert_eq!(load(&bytes).unwrap(), font);
        assert_eq!(save(&load(&bytes).unwrap()).unwrap(), bytes);
        // Out-of-range values are refused on load.
        let bad = text.replace("\"vert_width\": 1000", "\"vert_width\": 1e300");
        assert!(matches!(load(bad.as_bytes()), Err(FormatError::Invalid(_))), "{bad}");
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
        assert_eq!(p.nodes[0].info.extra.get(fontcraft_model::Family::Native, "name").and_then(|v| v.as_str()), Some("start"));
        let b = font.glyph("B").unwrap();
        let fontcraft_model::Shape::Component(c) = &b.layers[0].shapes[0] else { panic!() };
        assert_eq!(c.transform, fontcraft_model::Transform::Matrix([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]));
        assert!(font.extra.get(fontcraft_model::Family::Native, "fromTheFuture").is_some());
        // Saved again, it is a current file that loads to the same font.
        let again = save(&font).unwrap();
        assert!(String::from_utf8_lossy(&again).contains(&format!("\"version\": {VERSION}")));
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

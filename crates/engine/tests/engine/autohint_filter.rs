//! The Autohinting filter string (`filter.customParameter.autohinting`, M10.31): a Filter or
//! PreFilter parameter that names it autohints an instance's export in either flavour.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as Plist;
use fontcraft_engine::model::plist::openstep::parse_value;
use fontcraft_engine::model::{AlignmentZone, Instance, Node, NodeKind, Param, Params, Path, Shape};
use serde_json::json;
use skrifa::raw::TableProvider;
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::cs::CommandSink;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::raw::types::{Fixed, Tag};
use skrifa::{FontRef, MetadataProvider as _};

fn text(value: &str) -> Plist {
    Plist::String(value.into())
}

/// `l`: a stem from x 100 to 180, 0 to 700 high, on master `m01`, which has zones at the baseline
/// and at 700 and standard stems of 80; one static instance at the master per entry of
/// `instances`, with those parameters.
fn session(instances: &[(&str, Vec<(&str, Plist)>)]) -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let stem = Shape::Path(Path::new(true, vec![n(100.0, 0.0), n(180.0, 0.0), n(180.0, 700.0), n(100.0, 700.0)]));
        font.glyph_mut("l")?.layer_mut("m01").unwrap().shapes = vec![stem];
        let master = &mut Arc::make_mut(&mut font.masters)[0];
        master.zones = vec![AlignmentZone { pos: 0.0, size: -10.0 }, AlignmentZone { pos: 700.0, size: 10.0 }];
        for key in ["postscriptStemSnapH", "postscriptStemSnapV"] {
            master.params.0.push(Param::new(key, parse_value("(80)").unwrap()));
        }
        for (name, params) in instances {
            let params = Params(params.iter().map(|(k, v)| Param::new(*k, v.clone())).collect());
            Arc::make_mut(&mut font.instances).push(Instance { name: (*name).into(), params, ..Default::default() });
        }
        Ok(())
    })
    .unwrap();
    s
}

/// The fonts of an in-memory `file.exportStatic` without the autohint option, by instance name,
/// and the export's warnings.
fn export(s: &mut Session, outlines: &str) -> (Vec<(String, Vec<u8>)>, String) {
    let r = s.execute("file.exportStatic", &json!({"outlines": outlines, "autohint": false, "timestamp": 0})).unwrap();
    let fonts = r["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let data = base64::engine::general_purpose::STANDARD.decode(f["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
            (f["instance"].as_str().unwrap().to_string(), data)
        })
        .collect();
    let mut warnings = r["warnings"].to_string();
    for f in r["fonts"].as_array().unwrap() {
        warnings.push_str(&f["warnings"].to_string());
    }
    (fonts, warnings)
}

fn of<'a>(fonts: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    &fonts.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no font for {name}")).1
}

#[derive(Default)]
struct Stems(Vec<(i32, i32)>);

impl CommandSink for Stems {
    fn move_to(&mut self, _: Fixed, _: Fixed) {}
    fn line_to(&mut self, _: Fixed, _: Fixed) {}
    fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed) {}
    fn close(&mut self) {}
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.0.push((x.to_i32(), dx.to_i32()));
    }
}

/// The vertical stems of `l` in a CFF font.
fn vstems(bytes: &[u8]) -> Vec<(i32, i32)> {
    let font = FontRef::new(bytes).unwrap();
    let gid = font.charmap().map('l').unwrap();
    let table = font.table_data(Tag::new(b"CFF ")).unwrap();
    let cff = CffFontRef::new_cff(table.as_bytes(), 0, None).unwrap();
    let subfont = cff.subfont(0, &[]).unwrap();
    let mut sink = Stems::default();
    cff.evaluate_charstring(&subfont, gid, &[], &mut sink).unwrap();
    sink.0
}

/// The glyph program of `l` in a TrueType font.
fn program(bytes: &[u8]) -> Vec<u8> {
    let font = FontRef::new(bytes).unwrap();
    let gid = font.charmap().map('l').unwrap();
    let (glyf, loca) = (font.glyf().unwrap(), font.loca(None).unwrap());
    match loca.get_glyf(gid, &glyf).unwrap() {
        Some(RawGlyph::Simple(g)) => g.instructions().to_vec(),
        _ => Vec::new(),
    }
}

/// An instance whose Filter (or PreFilter) names Autohinting is autohinted in both flavours with
/// the export option off, even against its own Autohint parameter; the others are not.
#[test]
fn an_autohinting_filter_autohints_the_instance_in_both_flavours() {
    let mut s = session(&[
        ("Plain", vec![]),
        ("Hinted", vec![("Filter", text("Autohinting"))]),
        ("Early", vec![("PreFilter", text(" Autohinting "))]),
        ("Forced", vec![("Autohint", text("0")), ("Filter", text("Autohinting"))]),
        ("Args", vec![("Filter", text("Autohinting; 20"))]),
    ]);
    let (fonts, warnings) = export(&mut s, "cff");
    assert!(vstems(of(&fonts, "Plain")).is_empty(), "{warnings}");
    for name in ["Hinted", "Early", "Forced", "Args"] {
        assert_eq!(vstems(of(&fonts, name)), vec![(100, 180)], "{name}: {warnings}");
    }
    assert!(!warnings.contains("was not used"), "Autohinting is not an unknown filter: {warnings}");
    assert!(warnings.contains("\"Autohinting; 20\" takes no arguments"), "{warnings}");

    let (fonts, warnings) = export(&mut s, "truetype");
    assert!(program(of(&fonts, "Plain")).is_empty(), "{warnings}");
    for name in ["Hinted", "Early", "Forced", "Args"] {
        assert!(!program(of(&fonts, name)).is_empty(), "{name}: {warnings}");
    }
    // The document keeps its parameters: only the export's copy was changed.
    let font = s.font().unwrap();
    assert!(font.instances[1].params.get("Autohint").is_none());
}

/// A master's export has no Autohint parameter to turn on: the string is reported, not used.
#[test]
fn an_autohinting_filter_on_a_master_export_is_reported() {
    let mut s = session(&[]);
    s.edit("filter", |font| {
        Arc::make_mut(&mut font.info).params.0.push(Param::new("Filter", text("Autohinting")));
        Ok(())
    })
    .unwrap();
    let (fonts, warnings) = export(&mut s, "cff");
    assert!(warnings.contains("Autohinting filter string was not used"), "{warnings}");
    assert!(vstems(&fonts[0].1).is_empty());
}

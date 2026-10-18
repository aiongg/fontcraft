//! Imported TrueType hinting (M10.22): opening a hinted font keeps its hinting, a re-export
//! writes it back, and `hint.tt.removeImportedHinting` takes it out in one undo step.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::compile::{Options, compile_ttf};
use fontcraft_engine::model::plist::{Dict, Number, Str, Value as PValue};
use fontcraft_engine::model::{Font, Path, Shape};
use fontcraft_hinting::imported::{ImportedProgram, imported_program, set_imported_program};
use fontcraft_hinting::settings::{CVT_TABLE, FPGM_TABLE_ASSEMBLY, GASP_TABLE, PREP_TABLE_ASSEMBLY};
use serde_json::{Value, json};
use skrifa::FontRef;
use skrifa::raw::TableProvider as _;
use skrifa::raw::types::Tag;

const FPGM: &str = "PUSHB[ ] 0\nFDEF[ ]\nPOP[ ]\nENDF[ ]";

/// A hinted TrueType font, compiled from a document as an opened hinted font leaves it, written
/// to a file and opened.
fn opened_session(tag: &str) -> Session {
    let mut f = Font::new("Hint Import");
    {
        let mut h = f.glyph_mut("H").unwrap();
        h.layers[0].shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0))];
        set_imported_program(&mut h.layers[0], "SVTCA[0]\nPUSHB[ ] 0\nMDAP[1]".into());
    }
    let params = &mut Arc::make_mut(&mut f.masters)[0].params;
    params.set(CVT_TABLE, PValue::String(Str::new("10, -20")));
    params.set(FPGM_TABLE_ASSEMBLY, PValue::String(Str::new(FPGM)));
    params.set(PREP_TABLE_ASSEMBLY, PValue::String(Str::new("PUSHB[ ] 1\nPUSHB[ ] 0\nCALL[ ]")));
    let mut gasp = Dict::new();
    gasp.insert("65535", PValue::Number(Number::int(15)));
    Arc::make_mut(&mut f.info).params.set(GASP_TABLE, PValue::Dict(gasp));
    let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;

    let dir = std::env::temp_dir().join(format!("fontcraft-hint-import-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Hinted.ttf");
    std::fs::write(&path, bytes).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path.display().to_string()}));
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The TrueType export of the session's font.
fn exported(s: &mut Session) -> Vec<u8> {
    let r = run(s, "file.exportStatic", json!({"outlines": "truetype", "timestamp": 0}));
    base64::engine::general_purpose::STANDARD.decode(r["fonts"][0]["files"][0]["dataBase64"].as_str().unwrap()).unwrap()
}

fn has_table(bytes: &[u8], tag: &[u8; 4]) -> bool {
    FontRef::new(bytes).unwrap().table_data(Tag::new(tag)).is_some()
}

/// The program of `H` in an export.
fn h_program(bytes: &[u8]) -> Vec<u8> {
    let font = FontRef::new(bytes).unwrap();
    let gid = skrifa::MetadataProvider::charmap(&font).map('H').unwrap();
    match font.loca(None).unwrap().get_glyf(gid, &font.glyf().unwrap()).unwrap() {
        Some(skrifa::raw::tables::glyf::Glyph::Simple(g)) => g.instructions().to_vec(),
        _ => Vec::new(),
    }
}

#[test]
fn an_opened_hinted_font_exports_its_hinting_until_it_is_removed() {
    let mut s = opened_session("remove");
    let before = exported(&mut s);
    assert!(has_table(&before, b"fpgm") && has_table(&before, b"cvt ") && has_table(&before, b"prep"));
    // SVTCA[0], PUSHB[ ] 0, MDAP[1].
    assert_eq!(h_program(&before), [0x00, 0xB0, 0x00, 0x2F]);

    let font_before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "hint.tt.removeImportedHinting", json!({}));
    assert_eq!(r["glyphs"], 1);
    assert_eq!(r["parameters"], 3, "the three table parameters; gasp stays by default");
    {
        let font = s.font().unwrap();
        let m = font.default_master().unwrap();
        assert!([CVT_TABLE, FPGM_TABLE_ASSEMBLY, PREP_TABLE_ASSEMBLY].iter().all(|n| m.params.get(n).is_none()));
        assert!(font.info.params.get(GASP_TABLE).is_some());
        assert_eq!(imported_program(&font.glyph("H").unwrap().layers[0]), ImportedProgram::Absent);
    }
    let after = exported(&mut s);
    assert!(!has_table(&after, b"fpgm") && !has_table(&after, b"prep") && !has_table(&after, b"cvt "));
    assert!(h_program(&after).is_empty());

    // One undo step gives back the very font; nothing left to remove makes no step.
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &font_before));
    run(&mut s, "hint.tt.removeImportedHinting", json!({"gasp": true}));
    let removed = s.doc().unwrap().font.clone();
    let r = run(&mut s, "hint.tt.removeImportedHinting", json!({"gasp": true}));
    assert_eq!((r["glyphs"].as_u64(), r["parameters"].as_u64()), (Some(0), Some(0)));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &removed));
    assert!(s.font().unwrap().info.params.get(GASP_TABLE).is_none());
}

#[test]
fn removing_only_the_glyph_programs_keeps_the_tables() {
    let mut s = opened_session("programs");
    let r = run(&mut s, "hint.tt.removeImportedHinting", json!({"controlValueProgram": false}));
    assert_eq!((r["glyphs"].as_u64(), r["parameters"].as_u64()), (Some(1), Some(0)));
    let bytes = exported(&mut s);
    assert!(has_table(&bytes, b"fpgm"));
    assert!(h_program(&bytes).is_empty());
}

#[test]
fn bad_parameters_are_refused_and_change_nothing() {
    let mut s = opened_session("refused");
    let before = s.doc().unwrap().font.clone();
    assert!(s.execute("hint.tt.removeImportedHinting", &json!({"gasp": "yes"})).is_err());
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
}

/// A glyph of one point, like the `.ttfautohint` glyph the TrueType autohinter adds, keeps its
/// point and its imported program: outline preparation alone would write it empty.
#[test]
fn a_one_point_glyph_keeps_its_imported_program() {
    use fontcraft_engine::model::{Node, NodeKind};
    let mut f = Font::new("One Point");
    f.add_glyph(".ttfautohint", vec![], 0.0).unwrap();
    {
        let mut g = f.glyph_mut(".ttfautohint").unwrap();
        g.layers[0].shapes = vec![Shape::Path(Path::new(true, vec![Node::new(0.0, 0.0, NodeKind::Line)]))];
        set_imported_program(&mut g.layers[0], "PUSHB[ ] 0\nPOP[ ]".into());
    }
    Arc::make_mut(&mut f.masters)[0].params.set(CVT_TABLE, PValue::String(Str::new("10")));
    let out = compile_ttf(&f, &Options::default()).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    let gid = out.glyph_order.iter().position(|n| n == ".ttfautohint").unwrap();
    let glyph = font.loca(None).unwrap().get_glyf(skrifa::GlyphId::new(gid as u32), &font.glyf().unwrap()).unwrap();
    let Some(skrifa::raw::tables::glyf::Glyph::Simple(g)) = glyph else { panic!("not a simple glyph") };
    assert_eq!(g.num_points(), 1);
    assert_eq!(g.instructions(), [0xB0, 0x00, 0x21]);
}

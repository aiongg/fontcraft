//! TrueType instructions as assembly (M13.51): `hint.tt.setAssembly` stores a glyph program or a
//! font-level program, export writes the bytes, and a parse error changes nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::cmd::tt_assembly::{ProgramTarget, stored_program};
use fontcraft_engine::model::{Font, Path, Shape};
use fontcraft_hinting::settings::{FPGM_TABLE_ASSEMBLY, PREP_TABLE_ASSEMBLY};
use fontcraft_hinting::ttasm;
use serde_json::{Value, json};
use skrifa::FontRef;
use skrifa::raw::TableProvider as _;
use skrifa::raw::types::Tag;

/// A font with `H` (a rectangle of line nodes, so its points are its own `glyf` points) and a
/// second master.
fn session() -> Session {
    let mut f = Font::new("Assembly");
    f.glyph_mut("H").unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0))];
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn exported(s: &mut Session) -> Vec<u8> {
    let r = run(s, "file.exportStatic", json!({"outlines": "truetype", "timestamp": 0}));
    base64::engine::general_purpose::STANDARD.decode(r["fonts"][0]["files"][0]["dataBase64"].as_str().unwrap()).unwrap()
}

fn table(bytes: &[u8], tag: &[u8; 4]) -> Vec<u8> {
    FontRef::new(bytes).unwrap().table_data(Tag::new(tag)).map(|d| d.as_bytes().to_vec()).unwrap_or_default()
}

fn h_program(bytes: &[u8]) -> Vec<u8> {
    let font = FontRef::new(bytes).unwrap();
    let gid = skrifa::MetadataProvider::charmap(&font).map('H').unwrap();
    match font.loca(None).unwrap().get_glyf(gid, &font.glyf().unwrap()).unwrap() {
        Some(skrifa::raw::tables::glyf::Glyph::Simple(g)) => g.instructions().to_vec(),
        _ => Vec::new(),
    }
}

const FPGM: &str = "PUSHB[ ] 0\nFDEF[ ]\n  POP[ ]\nENDF[ ]";
/// The glyph program in the argument shorthand: `MDAP[1] 0` pushes 0 first.
const GLYPH: &str = "SVTCA[0] ; y\nMDAP[1] 0\nPUSHB[ ] 4 0\nCALL[ ]\nIUP[0]";

#[test]
fn assembly_is_stored_exported_and_disassembles_back_equal() {
    let mut s = session();
    let r = run(&mut s, "hint.tt.setAssembly", json!({"target": "fpgm", "assembly": FPGM}));
    assert_eq!((r["changed"].as_bool(), r["bytes"].as_u64(), r["instructions"].as_u64()), (Some(true), Some(5), Some(4)));
    assert!(r["master"].is_null(), "without `master`, every master");
    run(&mut s, "hint.tt.setAssembly", json!({"target": "prep", "assembly": "PUSHB[ ] 1\nPUSHB[ ] 0\nCALL[ ]"}));
    let r = run(&mut s, "hint.tt.setAssembly", json!({"target": {"glyph": "H"}, "assembly": GLYPH}));
    assert_eq!(r["changed"], true);
    assert_eq!(r["warnings"], json!([]), "the master has fpgm and prep, so export writes the glyph's program");

    let font = s.font().unwrap();
    for m in font.masters.iter() {
        assert_eq!(m.params.get(FPGM_TABLE_ASSEMBLY).and_then(|v| v.as_str()), Some(FPGM));
    }
    let stored = stored_program(font, &ProgramTarget::Glyph("H".into()), None).unwrap().unwrap();
    assert!(!stored.stale);
    assert_eq!(stored.text, "SVTCA[0] ; y\nPUSH 0\nMDAP[1]\nPUSHB[ ] 4 0\nCALL[ ]\nIUP[0]");

    let bytes = exported(&mut s);
    let typed = |text: &str| ttasm::disassemble(&ttasm::assemble(text).unwrap()).unwrap();
    assert_eq!(ttasm::disassemble(&table(&bytes, b"fpgm")).unwrap(), typed(FPGM));
    assert_eq!(ttasm::disassemble(&table(&bytes, b"prep")).unwrap(), typed("PUSHB[ ] 1\nPUSHB[ ] 0\nCALL[ ]"));
    assert_eq!(
        ttasm::disassemble(&h_program(&bytes)).unwrap(),
        "SVTCA[0]\nPUSHB[ ] 0\nMDAP[1]\nPUSHB[ ] 4 0\nCALL[ ]\nIUP[0]\n",
        "the glyph's program, the shorthand's push written out"
    );

    // Setting the same text again makes no undo step; empty text removes the program.
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "hint.tt.setAssembly", json!({"target": {"glyph": "H"}, "assembly": GLYPH}));
    assert_eq!(r["changed"], false);
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
    let r = run(&mut s, "hint.tt.setAssembly", json!({"target": {"glyph": "H"}, "assembly": "; nothing\n"}));
    assert_eq!((r["changed"].as_bool(), r["assembly"].is_null()), (Some(true), true));
    assert!(stored_program(s.font().unwrap(), &ProgramTarget::Glyph("H".into()), None).unwrap().is_none());
    assert!(h_program(&exported(&mut s)).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "one undo step");
}

#[test]
fn a_parse_error_changes_nothing() {
    let mut s = session();
    run(&mut s, "hint.tt.setAssembly", json!({"target": "prep", "assembly": "PUSHB[ ] 0\nPOP[ ]"}));
    let before = s.doc().unwrap().font.clone();
    for (target, text) in [
        (json!({"glyph": "H"}), "SVTCA[0]\nMDAP[1] 1\nNOSUCH[ ]"),
        (json!("prep"), "PUSHB[ ] 300"),
        (json!("fpgm"), "FDEF[ ]"),
        (json!({"glyph": "Missing"}), "SVTCA[0]"),
        (json!("cvt"), "SVTCA[0]"),
    ] {
        let e = s.execute("hint.tt.setAssembly", &json!({"target": target, "assembly": text})).unwrap_err().to_string();
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{target}: {e}");
    }
    let e = s.execute("hint.tt.setAssembly", &json!({"target": {"glyph": "H"}, "assembly": "SVTCA[0]\nMDAP[1] 1\nNOSUCH[ ]"})).unwrap_err();
    assert!(e.to_string().contains("line 3"), "the line as typed: {e}");
}

#[test]
fn a_glyph_program_without_font_programs_warns_that_export_leaves_it_out() {
    let mut s = session();
    let r = run(&mut s, "hint.tt.setAssembly", json!({"target": {"glyph": "H"}, "assembly": "SVTCA[0]"}));
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(1));
    assert!(h_program(&exported(&mut s)).is_empty());
}

#[test]
fn a_font_program_for_one_master_leaves_the_others() {
    let mut s = session();
    run(&mut s, "font.master.add", json!({"name": "Bold"}));
    let ids: Vec<String> = s.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
    assert_eq!(ids.len(), 2);
    run(&mut s, "hint.tt.setAssembly", json!({"target": "prep", "master": ids[1], "assembly": "PUSHB[ ] 0\nPOP[ ]"}));
    let font = s.font().unwrap();
    assert!(font.masters[0].params.get(PREP_TABLE_ASSEMBLY).is_none());
    assert_eq!(font.masters[1].params.get(PREP_TABLE_ASSEMBLY).and_then(|v| v.as_str()), Some("PUSHB[ ] 0\nPOP[ ]"));
    let stored = stored_program(font, &ProgramTarget::Prep, Some(&ids[1])).unwrap().unwrap();
    assert_eq!(stored.text, "PUSHB[ ] 0\nPOP[ ]");
    assert!(s.execute("hint.tt.setAssembly", &json!({"target": "prep", "master": "nope", "assembly": "POP[ ]"})).is_err());
}

#[test]
fn an_edited_outline_makes_the_stored_program_stale() {
    let mut s = session();
    run(&mut s, "hint.tt.setAssembly", json!({"target": {"glyph": "H"}, "assembly": "SVTCA[0]"}));
    {
        let mut f = s.font().unwrap().clone();
        f.glyph_mut("H").unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(60.0, 0.0, 150.0, 700.0))];
        s.edit("move", |font| {
            *font = f;
            Ok(())
        })
        .unwrap();
    }
    assert!(stored_program(s.font().unwrap(), &ProgramTarget::Glyph("H".into()), None).unwrap().unwrap().stale);
}

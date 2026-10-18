//! TrueType autohinting (M10.21): `autohint.tt.run`, `hint.tt.autohintGlyph` and the
//! `autohint` option of `file.exportStatic` for TrueType outlines.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as Plist;
use fontcraft_engine::model::{Instance, Node, NodeKind, Param, Path, Shape};
use serde_json::{Value, json};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::raw::types::Tag;
use skrifa::{FontRef, MetadataProvider as _};

/// `l`: a stem from x 100 to 180, 0 to 700 high, on master `m01`, whose metrics give zones at
/// the baseline and the cap height.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let stem = Shape::Path(Path::new(true, vec![n(100.0, 0.0), n(180.0, 0.0), n(180.0, 700.0), n(100.0, 700.0)]));
        font.glyph_mut("l")?.layer_mut("m01").unwrap().shapes = vec![stem];
        Ok(())
    })
    .unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn hint_count(s: &Session, glyph: &str) -> usize {
    s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap().info.hints.len()
}

/// The glyph program of `l` in a TrueType export of the session's font.
fn exported_program(s: &mut Session, autohint: bool) -> (Vec<u8>, Value) {
    let r = run(s, "file.exportStatic", json!({"outlines": "truetype", "autohint": autohint, "timestamp": 0}));
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["fonts"][0]["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    let gid = font.charmap().map('l').unwrap();
    let (glyf, loca) = (font.glyf().unwrap(), font.loca(None).unwrap());
    let program = match loca.get_glyf(gid, &glyf).unwrap() {
        Some(RawGlyph::Simple(g)) => g.instructions().to_vec(),
        _ => Vec::new(),
    };
    assert_eq!(font.table_data(Tag::new(b"fpgm")).is_some(), !program.is_empty());
    (program, r["fonts"][0]["warnings"].clone())
}

#[test]
fn export_static_autohints_truetype_glyphs() {
    let mut s = session();
    assert!(exported_program(&mut s, false).0.is_empty(), "no manual instructions, no program");
    let (program, warnings) = exported_program(&mut s, true);
    assert!(!program.is_empty(), "{warnings}");
}

#[test]
fn hint_tt_autohint_glyph_stores_instructions_in_one_undo_step() {
    let mut s = session();
    let before = s.font().unwrap().clone();
    for bad in [json!({}), json!({"glyphs": ["nope"]}), json!({"glyphs": ["l"], "layer": "nope"})] {
        assert!(s.execute("hint.tt.autohintGlyph", &bad).is_err(), "{bad}");
    }
    assert_eq!(*s.font().unwrap(), before, "a refusal changes nothing");

    let r = run(&mut s, "hint.tt.autohintGlyph", json!({"glyphs": ["l"]}));
    assert_eq!(r["hinted"][0]["glyph"], json!("l"), "{r}");
    // Two snaps (baseline, cap height) and the vertical stem.
    assert!(hint_count(&s, "l") >= 3, "{r}");
    // Stored instructions are manual ones: the export compiles them with autohinting off.
    assert!(!exported_program(&mut s, false).0.is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(hint_count(&s, "l"), 0, "one undo step");
}

#[test]
fn autohint_tt_run_reads_options_and_control_instructions_and_refuses_bad_ones() {
    let mut s = session();
    let before = s.font().unwrap().clone();
    for bad in [
        json!({"instance": "nope"}),
        json!({"glyphs": ["l"], "options": "--hinting-limit=lots"}),
        json!({"glyphs": ["l"], "controlInstructions": "l frobnicate 1"}),
    ] {
        assert!(s.execute("autohint.tt.run", &bad).is_err(), "{bad}");
    }
    assert_eq!(*s.font().unwrap(), before, "a refusal changes nothing");

    let r = run(&mut s, "autohint.tt.run", json!({"glyphs": ["l"], "options": "--dehint"}));
    assert_eq!(hint_count(&s, "l"), 0, "dehint stores nothing: {r}");
    let r = run(&mut s, "autohint.tt.run", json!({"glyphs": ["l"], "options": "--stem-width-mode=nnn --hinting-limit=100"}));
    assert_eq!(r["ignoredOptions"], json!(["stem-width-mode"]), "{r}");
    let plain = hint_count(&s, "l");
    assert!(plain >= 3);
    // A `touch` correction adds a Delta; `nodir` keeps a point out of the instructions.
    run(&mut s, "autohint.tt.run", json!({"glyphs": ["l"], "controlInstructions": "l touch 2 yshift 1 @ 12"}));
    assert_eq!(hint_count(&s, "l"), plain + 1);
    let r = run(&mut s, "autohint.tt.run", json!({"glyphs": ["l"], "controlInstructions": "l nodir 0, 1, 2, 3"}));
    assert_eq!(hint_count(&s, "l"), 0, "{r}");

    // The instance's parameters are read when no option is given.
    s.edit("instance", |font| {
        let mut inst = Instance { name: "Regular".into(), location: vec![400.0], ..Instance::default() };
        inst.params.0.push(Param::new("TTFAutohint options", Plist::String("--dehint".into())));
        Arc::make_mut(&mut font.instances).push(inst);
        Ok(())
    })
    .unwrap();
    run(&mut s, "hint.tt.autohintGlyph", json!({"glyphs": ["l"]}));
    let r = run(&mut s, "autohint.tt.run", json!({"glyphs": ["l"], "instance": "Regular"}));
    assert_eq!(hint_count(&s, "l"), 0, "{r}");
}

/// Sets the TTFAutohint binary path on the font.
fn set_binary(s: &mut Session, path: &str) {
    s.edit("binary", |font| {
        let params = &mut Arc::make_mut(&mut font.info).params.0;
        params.retain(|p| p.name != "TTFAutohint binary path");
        params.push(Param::new("TTFAutohint binary path", Plist::String(path.into())));
        Ok(())
    })
    .unwrap();
}

/// Names `path` as the user's trusted ttfautohint program.
fn trust(s: &mut Session, path: &str) {
    run(s, "settings.set", json!({"id": "export.ttfAutohintBinary", "value": path}));
}

#[test]
fn a_missing_or_failing_external_autohinter_keeps_the_built_in_result_with_a_warning() {
    let mut s = session();
    let (built_in, _) = exported_program(&mut s, true);
    trust(&mut s, "/nonexistent/fontcraft-test/ttfautohint");
    set_binary(&mut s, "/nonexistent/fontcraft-test/ttfautohint");
    let (program, warnings) = exported_program(&mut s, true);
    assert_eq!(program, built_in);
    assert!(warnings.to_string().contains("TTFAutohint binary path"), "{warnings}");
    #[cfg(unix)]
    {
        trust(&mut s, "/bin/false");
        set_binary(&mut s, "/bin/false");
        let (program, warnings) = exported_program(&mut s, true);
        assert_eq!(program, built_in);
        assert!(warnings.to_string().contains("failed"), "{warnings}");
        // A program that hands the font back unchanged: the export writes what it returns.
        trust(&mut s, "/bin/cat");
        set_binary(&mut s, "/bin/cat");
        let (program, warnings) = exported_program(&mut s, true);
        assert_eq!(program, built_in);
        assert!(!warnings.to_string().contains("TTFAutohint binary path"), "{warnings}");
        // Without Autohint the program does not run.
        trust(&mut s, "/bin/false");
        set_binary(&mut s, "/bin/false");
        let (_, warnings) = exported_program(&mut s, false);
        assert!(!warnings.to_string().contains("TTFAutohint binary path"), "{warnings}");
    }
}

#[test]
fn a_document_naming_an_untrusted_program_or_file_arguments_runs_nothing() {
    let mut s = session();
    let (built_in, _) = exported_program(&mut s, true);
    // No trusted program: the document's binary is never started (`/bin/false` would fail).
    set_binary(&mut s, "/bin/false");
    let (program, warnings) = exported_program(&mut s, true);
    assert_eq!(program, built_in);
    let text = warnings.to_string();
    assert!(text.contains("no trusted autohinter") && !text.contains("failed"), "{text}");
    // A trusted program the document does not name is not run either.
    trust(&mut s, "/bin/true");
    let (_, warnings) = exported_program(&mut s, true);
    let text = warnings.to_string();
    assert!(text.contains("not the trusted autohinter") && !text.contains("failed"), "{text}");
    // Words that are not ttfautohint options never reach the program as file names.
    trust(&mut s, "/bin/false");
    s.edit("options", |font| {
        Arc::make_mut(&mut font.info).params.0.push(Param::new("TTFAutohint options", Plist::String("--dehint in.ttf out.ttf".into())));
        Ok(())
    })
    .unwrap();
    let (program, warnings) = exported_program(&mut s, true);
    assert_eq!(program, built_in);
    let text = warnings.to_string();
    assert!(text.contains("cannot be passed to it") && !text.contains("failed"), "{text}");
}

#[test]
fn autohint_tt_run_skips_a_locked_glyph_unless_it_is_named() {
    let mut s = session();
    s.edit("lock", |font| {
        font.glyph_mut("l")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "autohint.tt.run", json!({}));
    assert!(r["skipped"].as_array().unwrap().iter().any(|k| k["glyph"] == "l"), "{r}");
    assert_eq!(hint_count(&s, "l"), 0);
    assert!(s.execute("autohint.tt.run", &json!({"glyphs": ["l"]})).is_err());
}

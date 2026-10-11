//! Metrics of composites under automatic alignment (M6.15): the setters take only offset keys on
//! an aligned glyph, offsets survive Update Metrics and base edits, the nudges leave aligned
//! glyphs alone, Import Metrics switches alignment off or keeps it (Keep Aligned Components), and
//! opening an imported file aligns its composites unless the user setting says not to.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Alignment, Anchor, Component, Font, Glyph, Layer, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changed the font");
    e
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-aligned-metrics-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `A` (outline 20–580, width 600, `top` at (300, 700)), a wide `acutecomb` (outline −400 to 400,
/// `_top` at (0, 500)) and `Aacute` built from both. With `aligned`, `Aacute` is stored as
/// alignment places it (width 600, the acute at (300, 200), so the mark reaches 100 units past
/// each side); without, it is stale (width 1, the acute at the origin).
fn font_with(f: &mut Font, aligned: bool) {
    {
        let mut a = f.glyph_mut("A").unwrap();
        let l = &mut a.layers[0];
        l.shapes = vec![Shape::Path(Path::rect(20.0, 0.0, 580.0, 700.0))];
        l.width = 600.0;
        l.anchors.push(Anchor::new("top", 300.0, 700.0));
    }
    let master = f.masters[0].id.clone();
    let mut acute = Glyph::new("acutecomb");
    let mut l = Layer::new(&master, 0.0);
    l.shapes.push(Shape::Path(Path::rect(-400.0, 720.0, 400.0, 800.0)));
    l.anchors.push(Anchor::new("_top", 0.0, 500.0));
    acute.layers.push(l);
    f.glyphs.push(acute);
    let mut aacute = Glyph::new("Aacute");
    let (width, at) = if aligned { (600.0, (300.0, 200.0)) } else { (1.0, (0.0, 0.0)) };
    let mut l = Layer::new(&master, width);
    l.shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
    l.shapes.push(Shape::Component(Component::new("acutecomb", at.0, at.1)));
    aacute.layers.push(l);
    f.glyphs.push(aacute);
}

fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        font_with(f, true);
        Ok(())
    })
    .unwrap();
    s
}

/// `Aacute`'s width, and the x of its two components.
fn aacute(s: &Session) -> (f64, f64, f64) {
    let l = &s.font().unwrap().glyph("Aacute").unwrap().layers[0];
    let x = |i: usize| match &l.shapes[i] {
        Shape::Component(c) => c.affine().as_coeffs()[4],
        _ => panic!("not a component"),
    };
    (l.width, x(0), x(1))
}

fn alignments(s: &Session) -> Vec<Alignment> {
    let l = &s.font().unwrap().glyph("Aacute").unwrap().layers[0];
    l.shapes
        .iter()
        .map(|sh| match sh {
            Shape::Component(c) => c.info.alignment,
            _ => panic!("not a component"),
        })
        .collect()
}

#[test]
fn spacing_set_on_an_aligned_composite_takes_offsets_only_and_get_derived_metrics_shows_them() {
    let mut s = session();
    let d = run(&mut s, "spacing.getDerivedMetrics", json!({"glyph": "Aacute"}));
    assert_eq!(
        (&d["aligned"], &d["width"], &d["left"], &d["right"], &d["offsets"]),
        (&json!(true), &json!(600.0), &json!(20.0), &json!(20.0), &json!({"left": null, "right": null}))
    );
    let m = run(&mut s, "spacing.getMetrics", json!({"glyph": "Aacute"}));
    assert_eq!((&m["left"], &m["right"]), (&json!(-100.0), &json!(-100.0)), "the outline's sidebearings include the mark");
    let d = run(&mut s, "spacing.getDerivedMetrics", json!({"glyph": "A"}));
    assert_eq!((&d["aligned"], &d["left"]), (&json!(false), &Value::Null), "a glyph with a path is not aligned");

    // Numbers and ordinary keys are refused with a message.
    let e = refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["Aacute"], "value": 50}));
    assert!(e.contains("aligned automatically") && e.contains("=+N"), "{e}");
    refused(&mut s, "spacing.setRightSidebearing", json!({"glyphs": ["Aacute"], "value": "=A"}));
    let e = refused(&mut s, "spacing.setWidth", json!({"glyphs": ["Aacute"], "value": "=+10"}));
    assert!(e.contains("width"), "{e}");
    // One aligned glyph among others fails the whole command.
    refused(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["A", "Aacute"], "value": 40}));

    // An offset is stored and alignment adds it: the bases move right and the width grows.
    let r = run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["Aacute"], "value": "=+30"}));
    assert_eq!((&r["values"]["Aacute"], &r["key"]), (&json!(50.0), &json!("=+30")));
    assert_eq!(aacute(&s), (630.0, 30.0, 330.0));
    assert_eq!(s.font().unwrap().glyph("Aacute").unwrap().info.metrics_keys.left.as_deref(), Some("=+30"));
    let r = run(&mut s, "spacing.setRightSidebearing", json!({"glyphs": ["Aacute"], "value": "==-5"}));
    assert_eq!(r["values"]["Aacute"], json!(15.0));
    assert_eq!(aacute(&s).0, 625.0);
    assert_eq!(s.font().unwrap().glyph("Aacute").unwrap().layers[0].info.metrics_keys.right.as_deref(), Some("=-5"));
    let d = run(&mut s, "spacing.getDerivedMetrics", json!({"glyph": "Aacute"}));
    assert_eq!(d["offsets"], json!({"left": "=+30", "right": "==-5"}));
    assert_eq!(d["width"], json!(625.0));

    // The offsets are in sync by construction, and Update Metrics keeps them and the glyph.
    let found = run(&mut s, "spacing.listOutOfSyncMetrics", json!({"glyphs": ["Aacute"]}));
    assert_eq!(found["findings"], json!([]), "{found}");
    let before = s.font().unwrap().glyph("Aacute").unwrap().clone();
    let r = run(&mut s, "glyph.updateMetrics", json!({"glyphs": ["Aacute"]}));
    assert_eq!(r["updated"], json!([]), "{r}");
    assert_eq!(s.font().unwrap().glyph("Aacute").unwrap(), &before);

    // A base edit keeps the offsets; =+0 removes one.
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["A"], "value": 70}));
    assert_eq!(aacute(&s), (675.0, 30.0, 380.0));
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["Aacute"], "value": "=+0"}));
    assert_eq!(s.font().unwrap().glyph("Aacute").unwrap().info.metrics_keys.left, None);
    assert_eq!(aacute(&s), (645.0, 0.0, 350.0));
}

#[test]
fn spacing_nudges_skip_aligned_composites() {
    let mut s = session();
    let r = run(&mut s, "spacing.increaseLeftSidebearing", json!({"glyphs": ["A", "Aacute"], "amount": 10}));
    assert_eq!((&r["glyphs"], &r["skipped"]), (&json!(["A"]), &json!(["Aacute"])));
    // A moved; Aacute follows it through alignment, not through the nudge.
    assert_eq!(aacute(&s), (610.0, 0.0, 310.0));
    let r = run(&mut s, "spacing.shiftOutlineRight", json!({"glyphs": ["Aacute"]}));
    assert_eq!((&r["glyphs"], &r["skipped"]), (&json!([]), &json!(["Aacute"])));
    assert_eq!(aacute(&s), (610.0, 0.0, 310.0));
}

/// The master's metrics as kerning-text, with the metrics line of each `(glyph, lsb, rsb, width)`
/// in `lines` put in place of the exported one.
fn metrics_text(s: &mut Session, lines: &[(&str, f64, f64, f64)]) -> String {
    let out = run(s, "file.exportMetrics", json!({"format": "metrics"}));
    let mut text: Vec<String> = out["text"].as_str().unwrap().lines().map(str::to_string).collect();
    for (glyph, lsb, rsb, width) in lines {
        let at = text.iter().position(|l| l.starts_with(&format!("metrics\t{glyph}\t"))).unwrap_or_else(|| panic!("no line for {glyph}"));
        text[at] = format!("metrics\t{glyph}\t{lsb}\t{rsb}\t{width}\t\t\t");
    }
    text.join("\n") + "\n"
}

#[test]
fn file_import_metrics_switches_alignment_off_where_values_differ_unless_keep_aligned_components() {
    let mut s = session();
    let import = |s: &mut Session, text: &str, keep: bool| {
        run(
            s,
            "file.importMetrics",
            json!({"text": text, "importKerning": false, "importKerningGroups": false, "importMetrics": "lsbAndWidth", "keepAlignedComponents": keep}),
        )
    };

    // The font's own values: the composite stays aligned.
    let same = metrics_text(&mut s, &[]);
    let r = import(&mut s, &same, false);
    assert_eq!((&r["metrics"]["unaligned"], &r["metrics"]["skipped"]), (&json!([]), &json!([])), "{r}");
    assert_eq!(alignments(&s), [Alignment::Default, Alignment::Default]);

    // A new base whose composite line matches what alignment gives with it: still aligned. The
    // base goes first, so the composite is compared with the moved base.
    let follows = metrics_text(&mut s, &[("A", 70.0, 0.0, 650.0), ("Aacute", -50.0, 0.0, 650.0)]);
    let r = import(&mut s, &follows, false);
    assert_eq!(r["metrics"]["unaligned"], json!([]), "{r}");
    assert_eq!(aacute(&s), (650.0, 0.0, 350.0));
    run(&mut s, "edit.undo", json!({}));

    // Other values: alignment is switched off for the composite's components and the values hold.
    let differs = metrics_text(&mut s, &[("Aacute", -80.0, 0.0, 640.0)]);
    let before = s.doc().unwrap().font.clone();
    let r = import(&mut s, &differs, false);
    assert_eq!(r["metrics"]["unaligned"], json!(["Aacute"]), "{r}");
    assert_eq!(alignments(&s), [Alignment::Disabled, Alignment::Disabled]);
    assert_eq!(aacute(&s), (640.0, 20.0, 320.0));
    let m = run(&mut s, "spacing.getMetrics", json!({"glyph": "Aacute"}));
    assert_eq!(m["left"], json!(-80.0));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "the import is one undo step");

    // Keep Aligned Components: the composite keeps its derived metrics and is listed.
    let r = import(&mut s, &differs, true);
    assert_eq!(r["metrics"]["unaligned"], json!([]));
    let skipped = r["metrics"]["skipped"].as_array().unwrap();
    assert!(skipped.iter().any(|x| x["glyph"] == "Aacute" && x["reason"].as_str().unwrap().contains("Keep Aligned Components")), "{r}");
    assert_eq!(alignments(&s), [Alignment::Default, Alignment::Default]);
    assert_eq!(aacute(&s), (600.0, 0.0, 300.0));
}

#[test]
fn file_open_aligns_an_imported_ufo_unless_the_user_setting_disables_it() {
    let dir = temp("open");
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        font_with(f, false);
        Ok(())
    })
    .unwrap();
    let ufo = dir.join("Stale.ufo");
    run(&mut s, "file.saveAs", json!({ "path": ufo }));

    let mut opened = Session::new();
    let r = run(&mut opened, "file.open", json!({ "path": ufo }));
    assert_eq!(r["aligned"], json!(1), "{r}");
    assert_eq!(aacute(&opened), (600.0, 0.0, 300.0), "the composite follows its base on opening");

    let mut kept = Session::new();
    run(&mut kept, "settings.set", json!({"id": "user.disableAutomaticAlignmentForImportedFiles", "value": true}));
    let r = run(&mut kept, "file.open", json!({ "path": ufo }));
    assert_eq!(r["aligned"], json!(0), "{r}");
    assert_eq!(aacute(&kept), (1.0, 0.0, 0.0), "the components stay where the file puts them");
    assert_eq!(kept.font().unwrap().info.settings.disables_automatic_alignment, Some(true));

    // A compiled font opens view-only and without anchors: its composites stay as compiled.
    let ttf = dir.join("Stale.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf }));
    let mut compiled = Session::new();
    let r = run(&mut compiled, "file.open", json!({ "path": ttf }));
    assert_eq!((&r["viewOnly"], &r["aligned"]), (&json!(true), &json!(0)), "{r}");
    assert_eq!(compiled.font().unwrap().info.settings.disables_automatic_alignment, None);
    let _ = std::fs::remove_dir_all(&dir);
}

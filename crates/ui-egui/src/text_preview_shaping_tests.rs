//! Text Preview shaping and bidi (`textPreview.shaping`, `textPreview.unicodeBidi`): Arabic takes
//! its joining forms and its marks sit on their anchors, a Latin word in right-to-left text is
//! shaped left to right with the compiled kerning, numbers in right-to-left text are ordered as
//! UAX #9 orders them, the preview font is built again once editing pauses or taken from the
//! compile of the same revision, and lines break on shaped widths at UAX #14 opportunities
//! (`textPreview.lineBreaking`).

use std::cell::Cell;

use egui::Pos2;
use serde_json::{Value, json};

use crate::shell::text_preview::{self, Drawn};
use crate::{FontcraftApp, control};

/// A headless window whose clock the test moves.
struct Window {
    ctx: egui::Context,
    time: Cell<f64>,
}

impl Window {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|s| s.animation_time = 0.0);
        Self { ctx, time: Cell::new(0.0) }
    }

    /// One frame `dt` seconds after the last; what the preview drew.
    fn frame(&self, app: &mut FontcraftApp, dt: f64) -> Drawn {
        self.time.set(self.time.get() + dt);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            time: Some(self.time.get()),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        app.state.panels.text_preview.cache.drawn.clone()
    }

    /// Frames past the rebuild pause until no font build runs and the lines in view are shaped.
    fn settle(&self, app: &mut FontcraftApp) -> Drawn {
        for n in 0..1_000 {
            let d = self.frame(app, text_preview::REBUILD_PAUSE + 0.1);
            if n >= 3 && !d.building && d.shaped > 0 {
                return d;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the preview font was not built");
    }
}

fn exec(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
    assert_eq!(r["ok"], true, "{command} {r}");
    r["result"].clone()
}

fn names(d: &Drawn) -> Vec<&str> {
    d.glyphs.iter().map(|(n, _)| &**n).collect()
}

fn at(d: &Drawn, name: &str) -> Pos2 {
    d.glyphs.iter().find(|(n, _)| &**n == name).map(|(_, p)| *p).unwrap_or_else(|| panic!("{name} not drawn: {:?}", names(d)))
}

fn scale(app: &FontcraftApp) -> f32 {
    let tp = &app.state.panels.text_preview;
    (tp.size / f64::from(app.session.font().unwrap().units_per_em)) as f32
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// beh with its initial, medial and final forms (each with a `top` anchor at 200, 600), fatha as
/// a mark with `_top` at 50, 500, the init, medi and fina features, and the preview open.
fn arabic_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    exec(&mut app, "glyph.newGlyph", json!({"name": "beh-ar", "unicodes": ["0628"], "width": 500}));
    for (form, width) in [("init", 400), ("medi", 300), ("fina", 450)] {
        exec(&mut app, "glyph.newGlyph", json!({"name": format!("beh-ar.{form}"), "unicodes": [], "width": width}));
        exec(&mut app, "font.features.addFeature", json!({"tag": form, "code": format!("sub beh-ar by beh-ar.{form};")}));
    }
    exec(&mut app, "glyph.newGlyph", json!({"name": "fatha-ar", "unicodes": ["064E"], "width": 0}));
    exec(&mut app, "glyph.setCategory", json!({"glyphs": ["fatha-ar"], "category": "Mark", "subCategory": "Nonspacing"}));
    for glyph in ["beh-ar", "beh-ar.init", "beh-ar.medi", "beh-ar.fina"] {
        exec(&mut app, "anchor.add", json!({"glyph": glyph, "name": "top", "x": 200, "y": 600}));
    }
    exec(&mut app, "anchor.add", json!({"glyph": "fatha-ar", "name": "_top", "x": 50, "y": 500}));
    for (glyph, x) in [("A", 50), ("V", 100)] {
        exec(&mut app, "path.addRectangle", json!({"glyph": glyph, "x": x, "y": 0, "width": 400, "height": 700}));
    }
    exec(&mut app, "kerning.setPair", json!({"left": "A", "right": "V", "value": -100}));
    exec(&mut app, "window.textPreview", json!({}));
    app
}

#[test]
fn text_preview_shapes_arabic_with_joining_forms_and_marks_on_their_anchors() {
    let mut app = arabic_app();
    let w = Window::new();
    // beh fatha beh beh: initial, medial and final forms, the fatha over the first.
    exec(&mut app, "textPreview.setText", json!({"text": "\u{628}\u{64e}\u{628}\u{628}"}));
    let d = w.settle(&mut app);
    let mut drawn = names(&d);
    drawn.sort_unstable();
    assert_eq!(drawn, ["beh-ar.fina", "beh-ar.init", "beh-ar.medi", "fatha-ar"], "{:?}", d.message);
    let (init, medi, fina) = (at(&d, "beh-ar.init"), at(&d, "beh-ar.medi"), at(&d, "beh-ar.fina"));
    assert!(fina.x < medi.x && medi.x < init.x, "right to left: {init:?} {medi:?} {fina:?}");
    assert!(near(init.x - medi.x, 300.0 * scale(&app)), "the medial form's own width");
    let fatha = at(&d, "fatha-ar");
    let s = scale(&app);
    assert!(near(fatha.x - init.x, 150.0 * s) && near(init.y - fatha.y, 100.0 * s), "the fatha's _top on the initial form's top: {fatha:?} {init:?}");
    assert_eq!(d.message, None);
}

#[test]
fn text_preview_shapes_a_latin_word_in_right_to_left_text_left_to_right_with_the_compiled_kerning() {
    let mut app = arabic_app();
    let w = Window::new();
    exec(&mut app, "textPreview.setText", json!({"text": "\u{628} AV \u{628}"}));
    let d = w.settle(&mut app);
    assert_eq!(names(&d).iter().filter(|n| **n == "beh-ar").count(), 2, "two isolated behs: {:?}", names(&d));
    let (a, v) = (at(&d, "A"), at(&d, "V"));
    let width_a = app.session.font().unwrap().glyph("A").unwrap().layers[0].width as f32;
    assert!(near(v.x - a.x, (width_a - 100.0) * scale(&app)), "AV reads left to right, kerned by GPOS: {a:?} {v:?}");
    let behs: Vec<f32> = d.glyphs.iter().filter(|(n, _)| &**n == "beh-ar").map(|(_, p)| p.x).collect();
    assert!(behs[1] < a.x && v.x < behs[0], "the first beh at the right, the second at the left: {behs:?} {a:?} {v:?}");
}

#[test]
fn text_preview_orders_numbers_in_right_to_left_text_as_unicode_bidi_does() {
    let mut app = arabic_app();
    let w = Window::new();
    exec(&mut app, "textPreview.setText", json!({"text": "\u{5d0} 12 34"}));
    let d = w.settle(&mut app);
    let x = |c: char| d.fallback.iter().find(|(f, _)| *f == c).map(|(_, p)| p.x).unwrap();
    // alef ␣ 12 ␣ 34 in a right-to-left paragraph reads, from the left: 34 12 alef.
    assert!(x('3') < x('4') && x('4') < x('1') && x('1') < x('2') && x('2') < x('\u{5d0}'), "{:?}", d.fallback);
}

#[test]
fn text_preview_builds_its_font_again_once_editing_pauses() {
    let mut app = FontcraftApp::default();
    exec(&mut app, "glyph.newGlyph", json!({"name": "f_i", "width": 450}));
    exec(&mut app, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by f_i;"}));
    exec(&mut app, "window.textPreview", json!({}));
    exec(&mut app, "textPreview.setText", json!({"text": "fi"}));
    let w = Window::new();
    let d = w.settle(&mut app);
    assert_eq!(names(&d), ["f_i"], "liga is on by default, without Compile");

    exec(&mut app, "font.features.setCode", json!({"kind": "feature", "name": "liga", "code": "sub i by A;"}));
    w.frame(&mut app, 0.05);
    let d = w.frame(&mut app, 0.1);
    assert_eq!(names(&d), ["f_i"], "editing has not paused yet");
    assert!(!d.building);
    let d = w.settle(&mut app);
    assert_eq!(names(&d), ["f", "A"]);

    // Code that does not compile leaves the text as laid out by code point, with the reason.
    exec(&mut app, "font.features.setCode", json!({"kind": "feature", "name": "liga", "code": "sub i by nosuchglyph;"}));
    let mut d = w.frame(&mut app, 0.1);
    for _ in 0..1_000 {
        d = w.frame(&mut app, text_preview::REBUILD_PAUSE + 0.1);
        if d.message.is_some() && !d.building {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(d.message.as_deref().is_some_and(|m| m.contains("not shaped")), "{:?}", d.message);
    assert_eq!(names(&d), ["f", "i"]);
    assert_eq!(d.shaped, 0);
}

/// Text Preview takes the build `font.features.compile` made of the same revision instead of
/// compiling the code again: it shows the compiled code, and, when the code does not compile,
/// the compiler's reason, as a build of its own would.
#[test]
fn text_preview_takes_the_build_of_the_compile() {
    let mut app = FontcraftApp::default();
    exec(&mut app, "glyph.newGlyph", json!({"name": "f_i", "width": 450}));
    exec(&mut app, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by f_i;"}));
    exec(&mut app, "window.textPreview", json!({}));
    exec(&mut app, "textPreview.setText", json!({"text": "fi"}));
    let master = exec(&mut app, "font.features.compile", json!({}))["master"].as_str().unwrap().to_string();
    let kept_by_compile = |app: &FontcraftApp| {
        let doc = app.session.doc().unwrap();
        app.session.preview_builds().get(doc.id, doc.revision, &master).is_some_and(|b| b.source_hash.is_some())
    };
    let w = Window::new();
    let d = w.settle(&mut app);
    assert_eq!(names(&d), ["f_i"]);
    assert_eq!(d.master.as_deref(), Some(master.as_str()));
    assert!(kept_by_compile(&app), "Text Preview built no font of its own");

    exec(&mut app, "font.features.setCode", json!({"kind": "feature", "name": "liga", "code": "sub i by nosuchglyph;"}));
    exec(&mut app, "font.features.compile", json!({}));
    let mut d = w.frame(&mut app, 0.1);
    for _ in 0..1_000 {
        if d.message.is_some() && !d.building {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        d = w.frame(&mut app, 0.1);
    }
    assert!(d.message.as_deref().is_some_and(|m| m.contains("not shaped")), "{:?}", d.message);
    assert_eq!(names(&d), ["f", "i"]);
    assert!(kept_by_compile(&app), "the failed build of the compile was shown");
}

/// `docs/plans/M8.md` § 5: with 100,000 characters, the page in view shaped under 8 ms and the
/// whole text's bidirectional analysis under 30 ms (on Inter Roman when the corpus is fetched).
/// Run it in release:
/// `cargo test --release -p fontcraft-ui-egui text_preview_shapes_a_page -- --ignored --nocapture`.
#[test]
#[ignore = "speed measure"]
fn text_preview_shapes_a_page_of_a_long_text_within_the_target() {
    use fontcraft_engine::compile::preview::{PreviewKind, layout_font_with};
    use fontcraft_textlayout::{Measure, PreviewShaper, WidthCache, shape_line};

    let mut app = FontcraftApp::default();
    let inter = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    if inter.is_dir() {
        exec(&mut app, "file.open", json!({"path": inter.to_string_lossy()}));
    }
    let font = app.session.font().unwrap().clone();
    let master = font.masters[0].id.clone();
    let index = app.session.kern_index().unwrap();
    let started = std::time::Instant::now();
    let built = layout_font_with(&font, &master, PreviewKind::Full, &text_preview::build_options(&app.session)).unwrap();
    println!("the full preview font: {:?}", started.elapsed());
    let tags: Vec<_> = built.features.iter().map(|f| f.tag).collect();
    let shaper = PreviewShaper::new(std::sync::Arc::from(built.bytes.as_slice()), &built.glyph_order, &built.private_use, &tags).unwrap();

    // Latin with Hebrew words in it, so the bidirectional analysis runs over the whole text.
    let paragraph = "The quick brown fox \u{5e9}\u{5dc}\u{5d5}\u{5dd} jumps over the lazy dog 1234; Waltz, nymph, for quick jigs vex Bud. ";
    let mut text = String::new();
    while text.chars().count() < 100_000 {
        text.push_str(paragraph);
        if text.len().is_multiple_of(7) {
            text.push('\n');
        }
    }
    let text: String = text.chars().take(100_000).collect();
    exec(&mut app, "textPreview.setText", json!({"text": text}));
    let state = &app.state.panels.text_preview;
    let mut plain = state.clone();
    plain.text = std::sync::Arc::from(text.replace(|c: char| ('\u{5d0}'..='\u{5ea}').contains(&c), "x"));
    let (mut bidi, mut ltr, mut page) = (std::time::Duration::MAX, std::time::Duration::MAX, std::time::Duration::MAX);
    let mut measured = std::time::Duration::MAX;
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let (laid, layout) = text_preview::lay_out(&font, &master, state, 20_000.0, Some(&index), None).unwrap();
        bidi = bidi.min(t.elapsed());
        let t = std::time::Instant::now();
        text_preview::lay_out(&font, &master, &plain, 20_000.0, Some(&index), None).unwrap();
        ltr = ltr.min(t.elapsed());
        let cache = WidthCache::new();
        let t = std::time::Instant::now();
        text_preview::lay_out(&font, &master, state, 20_000.0, Some(&index), Some(Measure { shaper: &shaper, cache: &cache })).unwrap();
        measured = measured.min(t.elapsed());
        let t = std::time::Instant::now();
        for line in 0..30.min(layout.lines.len()) {
            shape_line(&font, &master, &laid, &layout, line, &shaper).unwrap();
        }
        page = page.min(t.elapsed());
    }
    let analysis = bidi.saturating_sub(ltr);
    println!("layout with bidi {bidi:?}, without {ltr:?}: the analysis {analysis:?}; a page of 30 lines shaped in {page:?}");
    println!("layout on shaped widths, the cache empty: {measured:?}");
    if !cfg!(debug_assertions) {
        assert!(page.as_millis() < 8, "a page took {page:?}");
        assert!(analysis.as_millis() < 30, "the analysis took {analysis:?}");
    }
}

/// A, V and a space; `dist` (on by default for horizontal text) moves A and V 100 units closer
/// in feature code, which the document's kerning does not know.
fn dist_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    exec(&mut app, "font.features.addFeature", json!({"tag": "dist", "code": "pos A V -100;\npos V A -100;"}));
    exec(&mut app, "window.textPreview", json!({}));
    exec(&mut app, "textPreview.setText", json!({"text": "AVA AVA AVA"}));
    app
}

#[test]
fn text_preview_breaks_lines_on_shaped_widths_so_kerning_moves_the_break() {
    use fontcraft_engine::compile::preview::{PreviewKind, layout_font_with};
    use fontcraft_textlayout::{Measure, PreviewShaper, WidthCache};

    let mut app = dist_app();
    let font = app.session.font().unwrap().clone();
    let master = font.masters[0].id.clone();
    let index = app.session.kern_index().unwrap();
    let state = &app.state.panels.text_preview;
    let built = layout_font_with(&font, &master, PreviewKind::Full, &text_preview::build_options(&app.session)).unwrap();
    let tags: Vec<_> = built.features.iter().map(|f| f.tag).collect();
    let shaper = PreviewShaper::new(std::sync::Arc::from(built.bytes.as_slice()), &built.glyph_order, &built.private_use, &tags).unwrap();
    let cache = WidthCache::new();
    let measure = Some(Measure { shaper: &shaper, cache: &cache });

    // By code point the text is `length` long; shaped, its six pairs are 600 units shorter. A
    // line 300 units shorter than `length` holds the shaped text only.
    let (_, wide) = text_preview::lay_out(&font, &master, state, 100_000.0, Some(&index), None).unwrap();
    let length = wide.lines[0].length;
    let (_, plain) = text_preview::lay_out(&font, &master, state, length - 300.0, Some(&index), None).unwrap();
    let ranges: Vec<_> = plain.lines.iter().map(|l| l.items.clone()).collect();
    assert_eq!(ranges, [0..8, 8..11], "by code point the line breaks after the second space");
    let (_, shaped) = text_preview::lay_out(&font, &master, state, length - 300.0, Some(&index), measure).unwrap();
    assert_eq!(shaped.lines.len(), 1, "shaped, the kerning makes the text fit");
    assert!((shaped.lines[0].length - (length - 600.0)).abs() < 0.5, "{} {length}", shaped.lines[0].length);
    assert!(cache.len() <= 2, "the second word comes from the cache: {} segments", cache.len());
}

#[test]
fn text_preview_wraps_a_long_line_at_word_boundaries_within_the_window() {
    let mut app = dist_app();
    let words = "AVA ".repeat(200);
    exec(&mut app, "textPreview.setText", json!({"text": words}));
    let w = Window::new();
    let d = w.settle(&mut app);
    assert!(d.lines > 1, "{} lines", d.lines);
    let mut rows: Vec<(f32, f32, &str)> = d.glyphs.iter().map(|(n, p)| (p.y, p.x, &**n)).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut starts = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        if k == 0 || !near(rows[k - 1].0, row.0) {
            starts.push(row.2);
        }
    }
    assert!(starts.len() > 1, "glyphs on {} baselines", starts.len());
    assert!(starts.iter().all(|n| *n == "A"), "every line starts a word: {starts:?}");
    let right = d.glyphs.iter().map(|(_, p)| p.x).fold(f32::MIN, f32::max);
    assert!(right < 1280.0, "{right}");
}

#[test]
fn text_preview_shapes_vertical_text_with_vert_and_breaks_it_into_columns() {
    let mut app = FontcraftApp::default();
    exec(&mut app, "glyph.newGlyph", json!({"name": "A.vert", "unicodes": [], "width": 600}));
    exec(&mut app, "font.features.addFeature", json!({"tag": "vert", "code": "sub A by A.vert;"}));
    exec(&mut app, "window.textPreview", json!({}));
    exec(&mut app, "textPreview.setLayoutOrientation", json!({"orientation": "vertical"}));
    exec(&mut app, "textPreview.setText", json!({"text": "AB"}));
    let w = Window::new();
    let d = w.settle(&mut app);
    assert_eq!(names(&d), ["A.vert", "B"], "vert applies in vertical text");
    let (a, b) = (at(&d, "A.vert"), at(&d, "B"));
    assert!(near(a.x, b.x) && a.y < b.y, "one column, top to bottom: {a:?} {b:?}");

    exec(&mut app, "textPreview.setText", json!({"text": "AB ".repeat(200)}));
    let d = w.settle(&mut app);
    assert!(d.lines > 1, "the text breaks into columns: {}", d.lines);
    assert!(names(&d).iter().all(|n| *n != "A"), "every A is substituted");
}

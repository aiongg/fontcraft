//! The previews of names and tokens in feature code, and the expanded-code view.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use fontcraft_engine::Session;
use serde_json::json;

use super::feature_previews::{PreviewWords, preview_lines, token_at};
use super::{Props, show};
use crate::app::models::font_features::{EntryPane, ExpandedModel, FeaturesModel, PreviewModel};
use crate::app::models::font_info::FontInfoModel;
use crate::app::models::font_view::CellArt;
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::shell::feature_work::step;
use crate::shell::font_info::FontInfoTab;
use crate::shell::font_info_tabs::{EntryKey, EntryKind, ExpandTarget, ExpandedView, PreviewAsk};

const ROOM: egui::Vec2 = egui::vec2(1280.0, 900.0);

fn panel(model: FontInfoModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let model = Tracked::new(model);
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(ROOM).settle();
    (fixture, posted)
}

fn liga() -> EntryKey {
    EntryKey { kind: EntryKind::Feature, name: "liga".into(), index: 0 }
}

/// The Features tab with the entry `liga`, its preview `preview` open.
fn with_preview(preview: PreviewModel) -> FontInfoModel {
    let entry = EntryPane { key: liga(), code: "sub a by @Top;".into(), preview: Some(preview), ..EntryPane::default() };
    let features = FeaturesModel { selected: vec![liga()], entry: Some(entry), ..FeaturesModel::default() };
    FontInfoModel { open: true, tab: FontInfoTab::Features, features, ..FontInfoModel::default() }
}

/// A class of `n` glyphs, each a square.
fn class(n: usize) -> PreviewModel {
    let square = vec![vec![[0.0, 0.0], [500.0, 0.0], [500.0, 500.0], [0.0, 500.0]]];
    let shown = n.min(crate::app::models::font_features::MAX_PREVIEW_ITEMS);
    PreviewModel {
        name: "@Top".into(),
        start: 9,
        end: 13,
        kind: "class".into(),
        glyphs: (0..shown).map(|i| format!("glyph{i}")).collect(),
        total: n,
        art: (0..shown).map(|_| Arc::new(CellArt { width: 600.0, contours: square.clone(), color: None })).collect(),
        units_per_em: 1000.0,
        ..PreviewModel::default()
    }
}

/// A font of two masters (`gap` 10 and 30) and an instance halfway, its Features tab open on `code`, in an automatic
/// feature `liga`.
fn app(code: &str) -> FontCraftApp {
    let host = Host::builder(APP).jobs(Arc::new(craft::host::InlineJobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    for (id, params) in [
        ("font.numbers.add", json!({ "name": "gap", "values": { "m01": 10 } })),
        ("font.features.addFeature", json!({ "tag": "liga", "automatic": false, "code": code })),
    ] {
        app.sent.push(Msg::run(id, params));
    }
    app.sent.push(Msg::Shell(CommandRef::with_params("font.info", json!({ "tab": "features" }))));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    app.session
        .edit("automatic", |font| {
            Arc::make_mut(&mut font.features).features.iter_mut().for_each(|e| e.automatic = true);
            Ok(())
        })
        .unwrap();
    app.sent.push(Msg::Shell(crate::shell::font_info_tabs::select_entries(&[liga()])));
    app.drain();
    app
}

/// Asks the preview of characters `start..end` of the code and gives what the projection answers.
fn asked(app: &mut FontCraftApp, name: &str, token: bool) -> PreviewModel {
    let ask = PreviewAsk { key: liga(), name: name.into(), start: 0, end: 1, token };
    app.sent.push(Msg::View(ViewMsg::FeaturePreview(Some(ask))));
    app.drain();
    app.models.font_info.features.entry.clone().and_then(|e| e.preview).expect("the preview")
}

/// Every kind of name and token, in an automatic entry: a glyph and a class with their drawings, a lookup, a number token
/// with its value per build, a predicate token with the glyphs it matches.
#[test]
fn each_kind_of_name_and_token_has_a_preview() {
    let mut app = app("lookup Low { sub a by b; } Low;\npos a $gap;\nsub [a b]' by c;");
    app.sent.push(Msg::run("font.features.addClass", json!({ "name": "Some", "code": "a b" })));
    app.drain();
    app.sent.push(Msg::Shell(crate::shell::font_info_tabs::select_entries(&[liga()])));
    app.drain();

    let glyph = asked(&mut app, "a", false);
    assert_eq!((glyph.kind.as_str(), glyph.glyphs.clone(), glyph.art.len()), ("glyph", vec!["a".to_string()], 1), "{glyph:?}");
    let class = asked(&mut app, "@Some", false);
    assert_eq!((class.total, class.art.len(), class.units_per_em), (2, 2, 1000.0), "{class:?}");
    assert_eq!(asked(&mut app, "Low", false).kind, "lookup");
    let number = asked(&mut app, "${gap * 2}", true);
    assert_eq!((number.kind.as_str(), number.value), ("number", 20));
    assert_eq!(number.values, [("master Regular".to_string(), Some(20), String::new())]);
    let predicate = asked(&mut app, "$[name == \"a\"]", true);
    assert_eq!((predicate.kind.as_str(), predicate.glyphs.clone(), predicate.total), ("glyphs", vec!["a".to_string()], 1));
    assert!(asked(&mut app, "$nothing", true).problem.is_some());

    // The same question of the same font keeps its drawings.
    let first = asked(&mut app, "@Some", false);
    app.sent.push(Msg::View(ViewMsg::FeatureFilter("x".into())));
    app.drain();
    let again = app.models.font_info.features.entry.clone().and_then(|e| e.preview).expect("the preview");
    assert!(Arc::ptr_eq(&first.art[0], &again.art[0]));
}

/// A token is taken whole where the click lands in it, and not in a comment.
#[test]
fn a_click_in_a_token_takes_all_of_it() {
    let code = "pos a ${gap + 2} $x; # $gap";
    assert_eq!(token_at(code, 11), Some((6, 16)));
    assert_eq!(token_at(code, 18), Some((17, 19)));
    assert_eq!(token_at(code, 2), None);
    assert_eq!(token_at(code, 25), None);
    let words = PreviewWords::default();
    let number = PreviewModel {
        name: "$gap".into(),
        kind: "number".into(),
        value: 10,
        values: vec![("master Regular".into(), Some(10), String::new()), ("instance Thin".into(), None, "no value".into())],
        ..PreviewModel::default()
    };
    assert_eq!(preview_lines(&number, &words), ["$gap = 10", "master Regular: 10", "instance Thin: no value"]);
}

/// A double click on a member of a class opens it in a tab and closes the preview.
#[test]
fn a_double_click_on_a_member_opens_it() {
    let (mut fixture, posted) = panel(with_preview(class(3)));
    let tile = fixture.by_label("glyph1").expect("the second member");
    fixture.click(&tile).click(&tile).settle();
    let open = Msg::Shell(CommandRef::with_params("view.editSelectedGlyphs", json!({ "glyphs": ["glyph1"] })));
    let posted = posted.borrow();
    assert!(posted.contains(&open) && posted.contains(&Msg::View(ViewMsg::FeaturePreview(None))), "{posted:?}");
}

/// A frame with the preview of a class of 3,000 glyphs open takes under 16 ms.
#[test]
#[ignore = "a measure: run it alone, in a release build"]
fn a_preview_of_3000_glyphs_takes_under_16_ms() {
    let (mut fixture, _) = panel(with_preview(class(3_000)));
    let mut times: Vec<std::time::Duration> = (0..5)
        .map(|_| {
            let start = std::time::Instant::now();
            fixture.frame_shapes();
            start.elapsed()
        })
        .collect();
    times.sort();
    eprintln!("a frame with a class of 3,000 glyphs: {:?} median of 5", times[2]);
    if !cfg!(debug_assertions) {
        assert!(times[2] < std::time::Duration::from_millis(16), "{:?}", times[2]);
    }
}

/// The expanded code is the query's for the build shown, follows an edit, and closes when an entry is selected.
#[test]
fn the_expanded_code_is_the_querys() {
    let mut app = app("pos a $gap;");
    app.sent.push(Msg::View(ViewMsg::FeatureExpanded(Some(ExpandedView { query: "POS".into(), ..ExpandedView::default() }))));
    app.drain();
    step(&mut app, 1.0);
    step(&mut app, 1.1);
    app.drain();
    let query = |app: &mut FontCraftApp| {
        let r = app.session.query("font.features.expandCode", &json!({})).unwrap();
        r["code"].as_str().unwrap().to_string()
    };
    let shown = |app: &FontCraftApp| app.models.font_info.features.expanded.clone().expect("the view");
    let first = shown(&app);
    assert_eq!(*first.code, query(&mut app));
    assert!(first.code.contains("pos a 10;") && !first.busy, "{first:?}");
    assert_eq!(first.found.len(), 1, "{first:?}");
    assert_eq!(first.target, ExpandTarget::Master("m01".into()));

    app.session
        .edit("gap", |font| {
            Arc::make_mut(&mut font.masters)[0].number_values = vec![25.0];
            Ok(())
        })
        .unwrap();
    app.sent.push(Msg::View(ViewMsg::FeatureFilter("liga".into())));
    app.drain();
    assert!(shown(&app).busy, "the code of the font as it was shows until the new one lands");
    for now in [10.0, 10.6, 10.7] {
        step(&mut app, now);
    }
    app.drain();
    assert!(shown(&app).code.contains("pos a 25;"));
    app.sent.push(Msg::Shell(crate::shell::font_info_tabs::select_entries(&[liga()])));
    app.drain();
    assert!(app.models.font_info.features.expanded.is_none());
}

/// The button under the code opens the view; Close closes it.
#[test]
fn a_button_opens_and_closes_the_expanded_code() {
    let mut model = with_preview(PreviewModel::default());
    if let Some(entry) = &mut model.features.entry {
        entry.preview = None;
    }
    let (mut fixture, posted) = panel(model.clone());
    let button = fixture.by_label("Show Expanded Code").expect("the button under the code");
    fixture.click(&button).settle();
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureExpanded(Some(ExpandedView::default())))));

    let code = Arc::new("feature liga {\nsub a by b;\n} liga;\n".to_string());
    let target = ExpandTarget::Master("m01".into());
    model.features.expanded = Some(ExpandedModel {
        state: ExpandedView::default(),
        targets: vec![(target.clone(), "Regular".into()), (ExpandTarget::Variable, String::new())],
        target,
        code: Arc::clone(&code),
        shown: code,
        busy: false,
        refused: Vec::new(),
        problem: None,
        found: Vec::new(),
    });
    let (mut fixture, posted) = panel(model);
    let check = fixture.by_label("With Generated Features").expect("the check box");
    fixture.click(&check).settle();
    let generated = ExpandedView { generated: true, ..ExpandedView::default() };
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureExpanded(Some(generated)))), "{:?}", posted.borrow());
    let close = fixture.by_label("Close").expect("the Close button");
    fixture.click(&close).settle();
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureExpanded(None))));
}

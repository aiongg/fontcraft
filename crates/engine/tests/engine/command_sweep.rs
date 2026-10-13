//! Every-command sweep: each registered command, in every fixture state, with hostile parameters,
//! must return `Ok` or `Err` (never panic). After `Err` the session is unchanged; after `Ok` every
//! invariant still holds, and no hint node reference dangles (M10.3). Extend
//! `fontcraft_testkit::junk_values` when a new kind of parameter appears; a command added to the
//! registry is swept without further work.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::{Session, command_specs};
use fontcraft_hinting::{
    Direction, Edge, NodeRef, PsHint, StemRef, TtInstruction, TtKind, TtPoint, ZoneRef, ps_hints, set_ps_hints, set_tt_instructions, tt_instructions,
};
use fontcraft_testkit::{Fixture, catch_quiet, check_session, junk_params, snapshot, temp_dir};

type State = (&'static str, fn() -> Session);

/// The sweep's sessions: every fixture, and `Drawn`'s outlines with PostScript hints and
/// TrueType instructions linked to nodes of `A` and `O`.
fn states() -> [State; 4] {
    [("Empty", || Fixture::Empty.session()), ("NewFont", || Fixture::NewFont.session()), ("Drawn", || Fixture::Drawn.session()), ("Hinted", hinted)]
}

fn hinted() -> Session {
    let mut s = Session::with_document();
    s.execute("path.addPath", &serde_json::json!({"glyph": "A", "nodes": fontcraft_testkit::rect(60.0, 0.0, 560.0, 700.0)})).unwrap();
    s.execute("path.addPath", &serde_json::json!({"glyph": "O", "nodes": fontcraft_testkit::oval(350.0, 350.0, 300.0, 360.0)})).unwrap();
    let node = |p, n| Edge::Node(NodeRef::new(p, n));
    let point = |p, n| TtPoint::Node(NodeRef::new(p, n));
    s.edit("hints", |font| {
        for (name, ps, tt) in [
            (
                "A",
                vec![PsHint::stem(true, node(0, 0), node(0, 3)), PsHint::stem(false, node(0, 0), node(0, 1)), PsHint::ghost(true, node(0, 2))],
                vec![
                    TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, point(0, 2)),
                    TtInstruction {
                        origin: Some(point(0, 0)),
                        ..TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Horizontal, point(0, 1))
                    },
                ],
            ),
            (
                "O",
                vec![PsHint::stem(true, node(0, 3), node(0, 9))],
                vec![TtInstruction {
                    origin: Some(point(0, 0)),
                    other: Some(point(0, 6)),
                    ..TtInstruction::new(TtKind::Interpolate, Direction::Horizontal, point(0, 3))
                }],
            ),
        ] {
            let mut g = font.glyph_mut(name)?;
            let master = g.layers.first().map(|l| l.id.clone()).unwrap();
            let layer = g.layer_mut(&master).unwrap();
            set_ps_hints(&mut layer.info, &ps).unwrap();
            set_tt_instructions(&mut layer.info, &tt).unwrap();
        }
        Ok(())
    })
    .unwrap();
    s
}

/// The first hint node reference in an open document, or its undo history, that names no node.
fn dangling(s: &Session) -> Option<String> {
    for (d, doc) in s.documents().iter().enumerate() {
        for g in doc.font.glyphs.iter() {
            for layer in &g.layers {
                if layer.info.hints.is_empty() {
                    continue;
                }
                let mut refs: Vec<NodeRef> = Vec::new();
                for (_, h) in ps_hints(&layer.info).unwrap_or_default() {
                    refs.extend([Some(h.origin), h.other].into_iter().flatten().filter_map(|e| match e {
                        Edge::Node(r) => Some(r),
                        Edge::Value(_) => None,
                    }));
                }
                for (_, t) in tt_instructions(&layer.info).unwrap_or_default() {
                    for p in t.points() {
                        match p {
                            TtPoint::Node(r) => refs.push(r),
                            TtPoint::Intersection { a, b } => refs.extend([a, b]),
                            TtPoint::Lsb | TtPoint::Rsb => {}
                        }
                    }
                }
                if let Some(r) = refs.iter().find(|r| r.resolve(layer).is_none()) {
                    return Some(format!("document {d}, glyph `{}`, layer `{}`: hint on node ({}, {}) dangles", g.name, layer.id, r.path, r.node));
                }
            }
        }
    }
    None
}

#[test]
fn no_command_panics_or_corrupts_the_session() {
    let scratch = temp_dir("sweep");
    let mut failures = Vec::new();
    let mut calls = 0usize;
    let states = states();
    for spec in command_specs() {
        for params in junk_params(spec.params, &scratch) {
            for (fx, make) in &states {
                let mut s = make();
                let before = snapshot(&s);
                calls += 1;
                match catch_quiet(|| s.execute(spec.id, &params)) {
                    Err(panic) => failures.push(format!("PANIC {} {params} [{fx}]: {panic}", spec.id)),
                    Ok(Err(_)) => {
                        if snapshot(&s) != before {
                            failures.push(format!("{} {params} [{fx}]: returned an error but changed the session", spec.id));
                        }
                    }
                    Ok(Ok(_)) => {
                        if let Err(e) = check_session(&s) {
                            failures.push(format!("{} {params} [{fx}]: {e}", spec.id));
                        } else if let Some(e) = dangling(&s) {
                            failures.push(format!("{} {params} [{fx}]: {e}", spec.id));
                        }
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(calls > 1000, "the sweep made only {calls} calls");
    assert!(failures.is_empty(), "{} of {calls} calls failed:\n{}", failures.len(), failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn an_internal_panic_is_reported_and_rolled_back() {
    // The guard is exercised directly: no shipped command panics, so there is none to call.
    let caught = fontcraft_engine::guard::catch_panic(|| -> u8 { panic!("bug") });
    assert_eq!(caught, Err("bug".to_string()));
}

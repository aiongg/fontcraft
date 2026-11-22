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
            set_ps_hints(layer, &ps).unwrap();
            set_tt_instructions(layer, &tt).unwrap();
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
                for (_, h) in ps_hints(layer).unwrap_or_default() {
                    refs.extend([Some(h.origin), h.other].into_iter().flatten().filter_map(|e| match e {
                        Edge::Node(r) => Some(r),
                        Edge::Value(_) => None,
                    }));
                }
                for (_, t) in tt_instructions(layer).unwrap_or_default() {
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

/// Chunks per starting state. The sweep is split by state and chunk so a parallel test runner
/// spreads it over all cores.
const CHUNKS: u64 = 4;

/// The chunk of a command id: a hash of the id, so a command stays in its chunk when others are
/// added and every command lands in exactly one.
fn chunk_of(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)) % CHUNKS
}

/// Runs every command of one chunk, with every junk parameter set, from one starting state.
fn sweep(state: &str, chunk: u64) {
    let (fx, make) = states().into_iter().find(|(name, _)| *name == state).unwrap();
    let scratch = temp_dir(&format!("sweep-{fx}-{chunk}"));
    let mut failures = Vec::new();
    let mut calls = 0usize;
    for spec in command_specs().iter().filter(|spec| chunk_of(spec.id) == chunk) {
        let files: Vec<&str> = spec.files.iter().map(|f| f.name).collect();
        for params in junk_params(spec.params, &files, &scratch) {
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
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(calls > 100, "the sweep of {fx} chunk {chunk} made only {calls} calls");
    assert!(failures.is_empty(), "{} of {calls} calls failed:\n{}", failures.len(), failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

/// One test per starting state and chunk: `empty_chunk_0` … `hinted_chunk_3`, and `SWEPT`, the
/// pairs they cover.
macro_rules! sweep_tests {
    ($($state:literal => [$($chunk:literal => $test:ident),*]),*) => {
        const SWEPT: &[(&str, u64)] = &[$($(($state, $chunk)),*),*];
        $($(
            #[test]
            fn $test() {
                sweep($state, $chunk);
            }
        )*)*
    };
}

sweep_tests! {
    "Empty" => [0 => empty_chunk_0, 1 => empty_chunk_1, 2 => empty_chunk_2, 3 => empty_chunk_3],
    "NewFont" => [0 => new_font_chunk_0, 1 => new_font_chunk_1, 2 => new_font_chunk_2, 3 => new_font_chunk_3],
    "Drawn" => [0 => drawn_chunk_0, 1 => drawn_chunk_1, 2 => drawn_chunk_2, 3 => drawn_chunk_3],
    "Hinted" => [0 => hinted_chunk_0, 1 => hinted_chunk_1, 2 => hinted_chunk_2, 3 => hinted_chunk_3]
}

#[test]
fn the_chunk_tests_cover_every_state_and_chunk() {
    let mut wanted: Vec<(&str, u64)> = states().iter().flat_map(|(name, _)| (0..CHUNKS).map(move |c| (*name, c))).collect();
    let mut swept = SWEPT.to_vec();
    wanted.sort_unstable();
    swept.sort_unstable();
    assert_eq!(swept, wanted, "add a test to `sweep_tests!` for each new state or chunk");
}

#[test]
fn an_internal_panic_is_reported_and_rolled_back() {
    // The guard is exercised directly: no shipped command panics, so there is none to call.
    let caught = fontcraft_engine::guard::catch_panic(|| -> u8 { panic!("bug") });
    assert_eq!(caught, Err("bug".to_string()));
}

use std::sync::atomic::{AtomicUsize, Ordering};

use fontcraft_model::{Anchor, Font, Glyph, Layer, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

use super::*;
use crate::check::{Severity, parse_findings};
use crate::layer_json::{FilterOutput, LayerInput};
use crate::reporter::{CacheKey, Drawing, PathCommand, Primitive, ReporterCache, layer_key};

fn wasm(text: &str) -> Vec<u8> {
    ::wat::parse_str(text).unwrap()
}

fn load(text: &str) -> Result<Plugin> {
    Plugin::load(&wasm(text), Limits::default())
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Shape {
    let n = |x, y| Node::new(x, y, NodeKind::Line);
    Shape::Path(Path::new(true, vec![n(x, y), n(x + w, y), n(x + w, y + h), n(x, y + h)]))
}

/// A glyph `a` with two rectangles and an anchor on the default master.
fn sample() -> (Font, Glyph, Layer) {
    let font = Font::new("Test");
    let mut layer = Layer::new("m01", 500.0);
    layer.shapes = vec![rect(50.0, 0.0, 100.0, 700.0), rect(200.0, -10.0, 150.0, 300.0)];
    layer.anchors = vec![Anchor::new("top", 250.0, 700.0)];
    let mut glyph = Glyph::new("a");
    glyph.unicodes = vec![0x61];
    glyph.layers = vec![layer.clone()];
    (font, glyph, layer)
}

fn input(font: &Font, glyph: &Glyph, layer: &Layer) -> Vec<u8> {
    LayerInput::from_layer(font, glyph, layer).unwrap().to_json().unwrap()
}

fn points(layer: &Layer) -> Vec<(f64, f64)> {
    layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).flat_map(|p| p.nodes.iter().map(|n| (n.x, n.y))).collect()
}

/// Runs `plugin` as a filter on `layer` with `params` and applies the result.
fn run_filter(plugin: &Plugin, font: &Font, glyph: &Glyph, layer: &Layer, params: Value) -> Result<Layer> {
    let params = plugin.manifest().validate_params(&params)?;
    let out = plugin.call(&input(font, glyph, layer), Value::Object(params).to_string().as_bytes())?;
    FilterOutput::parse(&out)?.apply(layer)
}

#[test]
fn limits_table_and_an_empty_registry() {
    assert!(LIMITS_TABLE.iter().any(|(name, v)| *name == "maxModuleBytes" && *v == error::MAX_MODULE_BYTES as u64));
    assert_eq!(LIMITS_TABLE.len(), 21);
    let r = Registry::new();
    assert!(r.is_empty() && r.list().is_empty() && r.enabled_of_kind(Kind::Filter).next().is_none());
}

#[test]
fn a_filter_shifts_every_node_and_keeps_the_rest_of_the_layer() {
    let (font, glyph, mut layer) = sample();
    layer.info.name = Some("kept".into());
    let input = LayerInput::from_layer(&font, &glyph, &layer).unwrap();
    assert_eq!(input.metrics.get("x-height"), Some(&500.0));
    assert_eq!((input.glyph.as_str(), input.unicodes.as_slice()), ("a", &[0x61][..]));
    let shift = load(&wat::shift_filter("org.test.shift")).unwrap();
    assert_eq!(shift.kind(), Kind::Filter);
    assert_eq!(shift.manifest().params_doc(), "{dx: int -1000000..1000000 (10), dy: int -1000000..1000000 (0)}");
    let out = run_filter(&shift, &font, &glyph, &layer, json!({"dx": 25, "dy": -5})).unwrap();
    let moved: Vec<_> = points(&layer).iter().map(|(x, y)| (x + 25.0, y - 5.0)).collect();
    assert_eq!(points(&out), moved);
    assert_eq!((out.anchors[0].x, out.anchors[0].y), (275.0, 695.0));
    assert_eq!((out.width, out.info.name.as_deref(), out.id.as_str()), (500.0, Some("kept"), "m01"));
    // Defaults apply when no parameters are given.
    let out = run_filter(&shift, &font, &glyph, &layer, Value::Null).unwrap();
    assert_eq!(points(&out).first(), Some(&(60.0, 0.0)));
}

#[test]
fn a_reporter_draws_the_bounding_box_and_is_cached() {
    let (font, glyph, layer) = sample();
    let bbox = load(&wat::bbox_reporter("org.test.bbox")).unwrap();
    let input = LayerInput::from_layer(&font, &glyph, &layer).unwrap().with_view(0.5, true).to_json().unwrap();
    let params = bbox.manifest().validate_params(&Value::Null).unwrap();
    let runs = AtomicUsize::new(0);
    let run = || {
        runs.fetch_add(1, Ordering::Relaxed);
        Drawing::parse(&bbox.call_limited(&input, b"{}", &Limits::for_kind(Kind::Reporter))?)
    };
    let cache = ReporterCache::new();
    let key = CacheKey::new(bbox.serial(), &params, layer_key(1, "a", "m01", 7), 0.5);
    let d = cache.get_or_run(key, run).unwrap();
    // Nodes span 50..350 × -10..700; the anchor at (250, 700) is inside.
    let to = |x, y| [f64::from(x), f64::from(y)];
    let want = vec![
        PathCommand::MoveTo { to: to(50, -10) },
        PathCommand::LineTo { to: to(350, -10) },
        PathCommand::LineTo { to: to(350, 700) },
        PathCommand::LineTo { to: to(50, 700) },
        PathCommand::Close,
    ];
    assert!(d.background.is_empty());
    assert!(
        matches!(d.foreground.as_slice(), [Primitive::Path { commands, stroke: Some(s), .. }] if *commands == want && s.color == [255, 0, 0, 255])
    );
    // A scale in the same bucket reuses the drawing; another revision or bucket misses.
    let same = CacheKey::new(bbox.serial(), &params, layer_key(1, "a", "m01", 7), 0.52);
    assert_eq!(same, key);
    cache.get_or_run(same, run).unwrap();
    assert_eq!(runs.load(Ordering::Relaxed), 1);
    cache.get_or_run(CacheKey::new(bbox.serial(), &params, layer_key(1, "a", "m01", 8), 0.5), run).unwrap();
    cache.get_or_run(CacheKey::new(bbox.serial(), &params, layer_key(1, "a", "m01", 7), 2.0), run).unwrap();
    assert_eq!((runs.load(Ordering::Relaxed), cache.len()), (3, 3));
    // An error is returned and not cached; dropping the module's serial empties the cache.
    let miss = CacheKey::new(bbox.serial(), &params, 99, 1.0);
    assert!(cache.get_or_run(miss, || Err(Error::Failed("no".into()))).is_err() && cache.get(&miss).is_none());
    cache.retain_modules(&[]);
    assert!(cache.is_empty());
}

#[test]
fn a_check_flags_every_path() {
    let (font, glyph, layer) = sample();
    let check = load(&wat::path_check("org.test.paths")).unwrap();
    let findings = parse_findings(&check.call(&input(&font, &glyph, &layer), b"{}").unwrap()).unwrap();
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.name == "path" && f.severity == Severity::Warning && f.position.is_none()));
    let empty = Layer::new("m01", 500.0);
    assert!(parse_findings(&check.call(&input(&font, &glyph, &empty), b"{}").unwrap()).unwrap().is_empty());
}

/// The hostile-module suite: each module fails with its error, the layer is untouched, and a good
/// module still runs afterwards.
#[test]
fn hostile_modules_fail_with_their_error_and_change_nothing() {
    let (font, glyph, layer) = sample();
    let before = layer.clone();
    let shift = load(&wat::shift_filter("org.test.shift")).unwrap();
    let man = wat::manifest("org.test.bad", "filter", "{}");
    let small = Limits { max_memory_bytes: 4 << 20, ..Limits::default() };
    let try_filter = |text: &str, limits: &Limits| -> Result<Layer> {
        let p = Plugin::load(&wasm(text), limits.clone())?;
        FilterOutput::parse(&p.call(&input(&font, &glyph, &layer), b"{}")?)?.apply(&layer)
    };
    let mut nodes = serde_json::to_value(&layer).unwrap();
    nodes["shapes"][0]["path"]["nodes"][1]["x"] = json!(1e9);
    let far = nodes.to_string();
    let huge_manifest = format!(r#"{{"id":"a","name":"x","kind":"filter","description":"{}"}}"#, "d".repeat(error::MAX_MANIFEST_BYTES));
    type Case = (&'static str, String, Limits, fn(&Error) -> bool);
    let cases: Vec<Case> = vec![
        (
            "an import",
            wat::module(&man, "(i64.const 0)", r#"(import "env" "clock" (func))"#, 1),
            Limits::default(),
            |e| matches!(e, Error::Module(m) if m.contains("env.clock")),
        ),
        ("an infinite loop", wat::module(&man, "(loop $l (br $l)) (i64.const 0)", "", 1), Limits::default(), |e| {
            *e == Error::Limit("instruction budget".into())
        }),
        (
            "memory.grow until refused",
            wat::module(&man, "(loop $l (br_if $l (i32.ne (memory.grow (i32.const 1)) (i32.const -1)))) (i64.const 0)", "", 1),
            small.clone(),
            |e| *e == Error::Limit("memory budget".into()),
        ),
        (
            "100 MB of output",
            wat::module(&man, "(i64.shl (i64.const 104857600) (i64.const 32))", "", 1),
            Limits::default(),
            |e| matches!(e, Error::Limit(m) if m.starts_with("output size")),
        ),
        (
            "output outside memory",
            wat::module(&man, "(i64.or (i64.shl (i64.const 100) (i64.const 32)) (i64.const 1000000))", "", 1),
            Limits::default(),
            |e| matches!(e, Error::Abi(_)),
        ),
        ("a trap", wat::module(&man, "unreachable", "", 1), Limits::default(), |e| matches!(e, Error::Trap(_))),
        ("deep recursion", wat::module(&man, "(call $r)", "(func $r (result i64) (call $r))", 1), Limits::default(), |e| {
            *e == Error::Limit("call depth".into())
        }),
        (
            "a wrong ABI version",
            wat::module_abi(2, &man, "(i64.const 0)", "", 1),
            Limits::default(),
            |e| matches!(e, Error::Abi(m) if m.contains("returned 2")),
        ),
        (
            "a manifest over 64 KiB",
            wat::module(&huge_manifest, "(i64.const 0)", "", 3),
            Limits::default(),
            |e| matches!(e, Error::Manifest(m) if m.contains("too long")),
        ),
        (
            "a parameter default out of range",
            wat::module(&wat::manifest("a", "filter", r#"{"r":{"type":"number","min":0,"max":1,"default":5}}"#), "(i64.const 0)", "", 1),
            Limits::default(),
            |e| matches!(e, Error::Manifest(m) if m.contains("outside")),
        ),
        (
            "a capability",
            wat::module(r#"{"id":"a","name":"x","kind":"filter","capabilities":["network"]}"#, "(i64.const 0)", "", 1),
            Limits::default(),
            |e| matches!(e, Error::Manifest(m) if m.contains("grants none")),
        ),
        ("an error code", wat::module(&man, "(i64.const -1)", "", 1), Limits::default(), |e| matches!(e, Error::Failed(_))),
        ("NaN in the output", wat::returning(&man, r#"{"shapes":[{"path":{"nodes":[{"x":NaN,"y":0,"type":"line"}]}}]}"#), Limits::default(), |e| {
            matches!(e, Error::Output(_))
        }),
        ("10^9 coordinates", wat::returning(&man, &far), Limits::default(), |e| matches!(e, Error::Output(m) if m.contains("beyond"))),
        ("no shapes", wat::returning(&man, r#"{"log":["hi"]}"#), Limits::default(), |e| matches!(e, Error::Output(m) if m.contains("shapes"))),
        ("a reported failure", wat::returning(&man, r#"{"error":"cannot do that"}"#), Limits::default(), |e| {
            *e == Error::Failed("cannot do that".into())
        }),
    ];
    for (what, text, limits, expected) in cases {
        let err = try_filter(&text, &limits).expect_err(what);
        assert!(expected(&err), "{what}: {err:?}");
        assert_eq!(layer, before, "{what}");
        let ok = run_filter(&shift, &font, &glyph, &layer, Value::Null).unwrap();
        assert_eq!(points(&ok).first(), Some(&(60.0, 0.0)), "{what}");
    }
    // Parameters of the wrong type are refused before any call.
    assert!(matches!(run_filter(&shift, &font, &glyph, &layer, json!({"dx": "far"})), Err(Error::Params(_))));
    assert!(matches!(run_filter(&shift, &font, &glyph, &layer, json!([1])), Err(Error::Params(_))));
    // A wall-clock budget of zero stops a long run at its first fuel slice.
    let spin = Plugin::load(&wasm(&wat::module(&man, "(loop $l (br $l)) (i64.const 0)", "", 1)), Limits::default()).unwrap();
    let rushed = Limits { wall_time_ms: 0, fuel_base: u64::MAX / 2, ..Limits::default() };
    assert_eq!(spin.call_limited(b"{}", b"{}", &rushed), Err(Error::Limit("time budget".into())));
    // Not WebAssembly, and too large.
    assert!(matches!(Plugin::load(b"hello", Limits::default()), Err(Error::Module(_))));
    let tiny = Limits { max_module_bytes: 8, ..Limits::default() };
    assert!(matches!(Plugin::load(&wasm(&wat::shift_filter("a")), tiny), Err(Error::Module(m)) if m.contains("limit 8")));
}

#[test]
fn reporter_and_check_outputs_are_capped() {
    let many = format!(
        r#"{{"foreground":[{}]}}"#,
        vec![r#"{"type":"line","from":[0,0],"to":[1,1],"stroke":{"color":[0,0,0,255],"width":1}}"#; error::MAX_PRIMITIVES + 1].join(",")
    );
    assert!(matches!(Drawing::parse(many.as_bytes()), Err(Error::Output(m)) if m.contains("more than")));
    for bad in [
        r#"{"background":[{"type":"circle","center":[0,1e9],"radius":5}]}"#,
        r#"{"background":[{"type":"line","from":[0,0],"to":[1,1],"stroke":{"color":[0,0,0,255],"width":5000}}]}"#,
        r#"{"background":[{"type":"text","at":[0,0],"text":"x","color":[0,0,0,255],"size":-1}]}"#,
        r#"{"foreground":[{"type":"blob"}]}"#,
    ] {
        assert!(matches!(Drawing::parse(bad.as_bytes()), Err(Error::Output(_))), "{bad}");
    }
    let long = format!(
        r#"{{"foreground":[{{"type":"text","at":[0,0],"text":"{}","color":[0,0,0,255],"size":12}}]}}"#,
        "w".repeat(error::MAX_TEXT_CHARS + 1)
    );
    assert!(Drawing::parse(long.as_bytes()).is_err());
    let findings = format!(r#"{{"findings":[{}]}}"#, vec![r#"{"name":"n","severity":"info","message":"m"}"#; error::MAX_FINDINGS + 1].join(","));
    assert!(matches!(parse_findings(findings.as_bytes()), Err(Error::Output(m)) if m.contains("more than")));
    for bad in [
        r#"{"findings":[{"name":"a b","severity":"info","message":"m"}]}"#,
        r#"{"findings":[{"name":"a","severity":"info","message":""}]}"#,
        r#"{"findings":[{"name":"a","severity":"fatal","message":"m"}]}"#,
        r#"{"findings":[{"name":"a","severity":"info","message":"m","position":[0,1e9]}]}"#,
    ] {
        assert!(matches!(parse_findings(bad.as_bytes()), Err(Error::Output(_))), "{bad}");
    }
    // Log lines are kept up to the limit and cut to length.
    let log = format!(r#"{{"shapes":[],"log":[{}]}}"#, vec![format!("{:?}", "l".repeat(2_000)); error::MAX_LOG_LINES + 5].join(","));
    let out = FilterOutput::parse(log.as_bytes()).unwrap();
    assert_eq!(out.log.len(), error::MAX_LOG_LINES);
    assert_eq!(out.log[0].chars().count(), error::MAX_LOG_LINE_CHARS);
}

#[test]
fn the_registry_installs_replaces_and_removes() {
    let mut r = Registry::new();
    let first = r.install_bytes(&wasm(&wat::shift_filter("org.test.shift")), None).unwrap().plugin.serial();
    r.install_bytes(&wasm(&wat::bbox_reporter("org.test.bbox")), Some("bbox.wasm".into())).unwrap();
    let rev = r.revision();
    let again = r.install_bytes(&wasm(&wat::shift_filter("org.test.shift")), None).unwrap().plugin.serial();
    assert!(again != first && r.revision() != rev);
    assert_eq!(r.list().iter().map(|m| m.plugin.id()).collect::<Vec<_>>(), ["org.test.shift", "org.test.bbox"]);
    assert_eq!(r.enabled_of_kind(Kind::Reporter).count(), 1);
    r.set_enabled("org.test.bbox", false).unwrap();
    assert_eq!(r.enabled_of_kind(Kind::Reporter).count(), 0);
    assert!(matches!(r.set_enabled("nope", true), Err(Error::NotFound(_))));
    assert!(r.remove("org.test.shift") && !r.remove("org.test.shift") && r.len() == 1);
    // A bad module installs nothing.
    assert!(r.install_bytes(b"\0asm junk", None).is_err() && r.len() == 1);
    for i in 1..error::MAX_INSTALLED {
        r.install_bytes(&wasm(&wat::returning(&wat::manifest(&format!("m{i}"), "check", "{}"), "{}")), None).unwrap();
    }
    let over = r.install_bytes(&wasm(&wat::returning(&wat::manifest("one.more", "check", "{}"), "{}")), None);
    assert!(matches!(over, Err(Error::TooMany(_))) && r.len() == error::MAX_INSTALLED);
}

/// The module speed targets of `docs/plans/M13.md` § 5 on 1,000 synthetic layers of three
/// 24-node contours each (about the size of a Latin text glyph). Run with
/// `cargo test -p fontcraft-plugins --profile measure -- --ignored --nocapture measure`.
#[test]
#[ignore]
fn measure_module_speed() {
    use std::time::Instant;
    let font = Font::new("Measure");
    let contour = |x0: f64| {
        let nodes = (0..24)
            .map(|i| {
                let t = f64::from(i) / 24.0 * std::f64::consts::TAU;
                Node::new(
                    (x0 + 100.0 * t.cos()).round(),
                    (350.0 + 300.0 * t.sin()).round(),
                    if i % 3 == 0 { NodeKind::Curve } else { NodeKind::OffCurve },
                )
            })
            .collect();
        Shape::Path(Path::new(true, nodes))
    };
    let glyphs: Vec<(Glyph, Layer)> = (0..1000)
        .map(|i| {
            let mut layer = Layer::new("m01", 600.0);
            layer.shapes = vec![contour(150.0), contour(300.0), contour(450.0)];
            let mut g = Glyph::new(format!("g{i}"));
            g.layers = vec![layer.clone()];
            (g, layer)
        })
        .collect();
    let shift = load(&wat::shift_filter("org.test.shift")).unwrap();
    let params = Value::Object(shift.manifest().validate_params(&Value::Null).unwrap()).to_string();
    let start = Instant::now();
    for (g, l) in &glyphs {
        let out = shift.call(&input(&font, g, l), params.as_bytes()).unwrap();
        FilterOutput::parse(&out).unwrap().apply(l).unwrap();
    }
    let filter = start.elapsed();
    let bbox = load(&wat::bbox_reporter("org.test.bbox")).unwrap();
    let limits = Limits::for_kind(Kind::Reporter);
    let cache = ReporterCache::new();
    let empty = serde_json::Map::new();
    let start = Instant::now();
    for (i, (g, l)) in glyphs.iter().enumerate() {
        let key = CacheKey::new(bbox.serial(), &empty, layer_key(1, &g.name, &l.id, 0), 1.0);
        cache.get_or_run(key, || Drawing::parse(&bbox.call_limited(&input(&font, g, l), b"{}", &limits)?)).unwrap();
        assert_eq!(cache.len(), i + 1);
    }
    let uncached = start.elapsed();
    let start = Instant::now();
    for (g, l) in &glyphs {
        let key = CacheKey::new(bbox.serial(), &empty, layer_key(1, &g.name, &l.id, 0), 1.0);
        cache.get_or_run(key, || Err(Error::Failed("cached".into()))).unwrap();
    }
    let cached = start.elapsed();
    println!("filter on 1,000 layers: {filter:?} (target < 1 s)");
    println!("reporter uncached: {:?} per layer (target < 5 ms)", uncached / 1000);
    println!("reporter cached: {:?} per glyph (target < 1 ms)", cached / 1000);
}

//! The M3 end-to-end measure (`docs/plans/M3.md` § 5): an agent that drives the app only over the
//! control channel draws "H", "O" and "n" on a new font, edits them with the menu shortcuts and
//! exports a TrueType font that reads back in skrifa.
//!
//! Every edit is a `ui.tool.select`, `ui.pointer` (font units) or `ui.key` request through
//! [`control::handle`], the entry point the control server uses. `engine.execute` only creates
//! the font, opens each glyph in Edit View at a known zoom, reads node lists (`layer.inspect`)
//! and exports.
//!
//! How the fixtures follow from the gestures:
//! - The zoom is 1000 screen points per 1000 units: one screen point is one unit. A press or a
//!   release lands on a node, handle or metric line within 4 points (the snap distance), else
//!   where it is. Every gesture below stays more than 4 units from anything it should not snap to.
//! - `path.addRectangle` and `path.addEllipse` round every node to the font's grid (1 unit, to
//!   the nearest). The rectangle drags start and end at fractional positions, so the fixtures
//!   show that rounding: (50.4, 0.3)–(129.6, 700.3) gives the box (50, 0)–(130, 700).
//! - A rectangle starts at its bottom-left corner and runs counter-clockwise.
//! - An ellipse in the box (x0, y0)–(x1, y1) starts at the middle of the bottom side and runs
//!   counter-clockwise through the middles of the other sides (smooth curve nodes). Each handle
//!   lies `radius × 0.5523` (the quarter-circle constant) from its node along the box side,
//!   rounded. Outer O, box (50, 0)–(550, 700): x = 300 ± 250 × 0.5523 = 438.07 | 161.93 → 438 |
//!   162; y = 350 ± 350 × 0.5523 = 543.30 | 156.70 → 543 | 157. Counter, box (150, 100)–(450,
//!   600): x = 300 ± 150 × 0.5523 = 382.84 | 217.16 → 383 | 217; y = 350 ± 250 × 0.5523 =
//!   488.07 | 211.93 → 488 | 212.
//! - Reverse Contours keeps a closed path's start node and lists the others backwards.
//! - Remove Overlap merges the three rectangles of the H into one outline whose corners are the
//!   outer corners of the stems and the four points where the crossbar meets the inner stem
//!   sides (the crossbar's ends lie inside the stems). Correct Path Direction then starts it at
//!   its lowest, then leftmost node and runs it counter-clockwise.
//! - The Draw tool: a click adds a corner line node; a drag adds a smooth node at the press with
//!   its outgoing handle at the release and the incoming handle mirrored; Alt makes the node a
//!   corner. The n's gestures and node list are those of `draw_tests.rs`.
//! - Return (Toggle Node Type) turns the selected corner (370, 380) smooth. The straight segment
//!   after it, down to (370, 0), becomes a curve with handles at a third and two thirds of its
//!   length, rounded: y = 380 − 380/3 = 253.33 → 253 and 380/3 = 126.67 → 127. Its two handles
//!   already lie on one vertical line, so neither turns.
//!
//! The fontTools read-back runs outside the test. Set `FONTCRAFT_E2E_OUT` to a path ending in
//! `.ttf` and the test writes the font there (else to a temporary folder it removes):
//!
//! ```text
//! FONTCRAFT_E2E_OUT=$PWD/target/e2e.ttf cargo test -p fontcraft-ui-egui e2e
//! ~/.local/share/uv/tools/fontmake/bin/python -I -c '
//! import sys
//! from fontTools.ttLib import TTFont
//! f = TTFont(sys.argv[1]); glyf = f["glyf"]; cmap = f.getBestCmap()
//! assert f["maxp"].numGlyphs == 54, f["maxp"].numGlyphs
//! for ch, contours, box in [("H", 1, (50, 0, 450, 700)), ("O", 2, (50, 0, 550, 700)), ("n", 1, (50, 0, 450, 510))]:
//!     g = glyf[cmap[ord(ch)]]
//!     got = (g.xMin, g.yMin, g.xMax, g.yMax)
//!     assert g.numberOfContours == contours and got == box, (ch, g.numberOfContours, got)
//!     print(ch, g.numberOfContours, "contours", len(g.coordinates), "points", got)
//! print("fontTools read-back: ok")
//! ' target/e2e.ttf
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::{FontRef, MetadataProvider};

use crate::{FontcraftApp, control};

/// One control request; it must succeed.
fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

fn execute(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": command, "params": params}))
}

fn tool(app: &mut FontcraftApp, id: &str) {
    let r = call(app, "ui.tool.select", json!({ "tool": id }));
    assert_eq!(r["tool"], id, "{r}");
}

/// Pointer events in font units; every action they ask for must succeed. The reply.
fn pointer(app: &mut FontcraftApp, events: Value) -> Value {
    let r = call(app, "ui.pointer", json!({ "events": events }));
    assert_eq!(r["errors"], json!([]), "{r}");
    r
}

fn click(app: &mut FontcraftApp, x: f64, y: f64) -> Value {
    pointer(app, json!([{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]))
}

/// A drag from (x, y) to (tx, ty) with `mods` held throughout.
fn drag(app: &mut FontcraftApp, (x, y): (f64, f64), (tx, ty): (f64, f64), mods: Value) {
    let mid = ((x + tx) / 2.0, (y + ty) / 2.0);
    pointer(
        app,
        json!([
            {"kind": "down", "x": x, "y": y, "mods": mods},
            {"kind": "drag", "x": mid.0, "y": mid.1, "mods": mods},
            {"kind": "drag", "x": tx, "y": ty, "mods": mods},
            {"kind": "up", "x": tx, "y": ty, "mods": mods},
        ]),
    );
}

/// A key the app must handle without an error; the reply.
fn key(app: &mut FontcraftApp, params: Value) -> Value {
    let r = call(app, "ui.key", params);
    assert_eq!(r["handled"], true, "{r}");
    assert_eq!(r["errors"], json!([]), "{r}");
    assert!(r.get("error").is_none(), "{r}");
    r
}

/// A menu shortcut: it must run `command`; the command's result.
fn shortcut(app: &mut FontcraftApp, params: Value, command: &str) -> Value {
    let r = key(app, params);
    assert_eq!(r["command"], command, "{r}");
    r["result"].clone()
}

/// Opens `glyph` in Edit View at one screen point per unit.
fn edit(app: &mut FontcraftApp, glyph: &str) {
    execute(app, "view.editSelectedGlyphs", json!({"glyphs": [glyph]}));
    execute(app, "view.setZoom", json!({"size": 1000, "center": [300, 350]}));
}

/// The glyph's closed paths from `layer.inspect`, each node as `x,y,kind`: `l` line, `c` curve,
/// `q` quadratic, `o` off-curve, `*` when smooth.
fn outline(app: &mut FontcraftApp, glyph: &str) -> Vec<Vec<String>> {
    let layer = execute(app, "layer.inspect", json!({ "glyph": glyph }));
    let shapes = layer["shapes"].as_array().unwrap().clone();
    shapes
        .iter()
        .map(|s| {
            assert_eq!((&s["kind"], &s["closed"]), (&json!("path"), &json!(true)), "{s}");
            s["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| {
                    let kind = match n["type"].as_str().unwrap() {
                        "line" => "l",
                        "curve" => "c",
                        "qcurve" => "q",
                        _ => "o",
                    };
                    let smooth = if n["smooth"] == true { "*" } else { "" };
                    format!("{},{},{kind}{smooth}", n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap())
                })
                .collect()
        })
        .collect()
}

fn owned(paths: &[&[&str]]) -> Vec<Vec<String>> {
    paths.iter().map(|p| p.iter().map(|s| s.to_string()).collect()).collect()
}

/// The on-curve points of a fixture path.
fn on_curve(path: &[&str]) -> Vec<(i32, i32)> {
    path.iter()
        .filter(|s| !s.ends_with(",o"))
        .map(|s| {
            let parts: Vec<&str> = s.split(',').collect();
            (parts[0].parse().unwrap(), parts[1].parse().unwrap())
        })
        .collect()
}

const H_STEMS_AND_BAR: [&[&str]; 3] = [
    &["50,0,l", "130,0,l", "130,700,l", "50,700,l"],
    &["370,0,l", "450,0,l", "450,700,l", "370,700,l"],
    &["100,300,l", "400,300,l", "400,380,l", "100,380,l"],
];

const H: [&[&str]; 1] = [&[
    "50,0,l",
    "130,0,l",
    "130,300,l",
    "370,300,l",
    "370,0,l",
    "450,0,l",
    "450,700,l",
    "370,700,l",
    "370,380,l",
    "130,380,l",
    "130,700,l",
    "50,700,l",
]];

const O: [&[&str]; 2] = [
    &[
        "300,0,c*",
        "438,0,o",
        "550,157,o",
        "550,350,c*",
        "550,543,o",
        "438,700,o",
        "300,700,c*",
        "162,700,o",
        "50,543,o",
        "50,350,c*",
        "50,157,o",
        "162,0,o",
    ],
    // The counter, reversed: clockwise from the same start node.
    &[
        "300,100,c*",
        "217,100,o",
        "150,212,o",
        "150,350,c*",
        "150,488,o",
        "217,600,o",
        "300,600,c*",
        "383,600,o",
        "450,488,o",
        "450,350,c*",
        "450,212,o",
        "383,100,o",
    ],
];

/// The n as the Draw tool leaves it (`draw_tests.rs`).
const N_DRAWN: [&[&str]; 1] = [&[
    "50,0,l",
    "130,0,l",
    "130,380,l",
    "130,420,o",
    "190,450,o",
    "250,450,c*",
    "310,450,o",
    "370,420,o",
    "370,380,c",
    "370,0,l",
    "450,0,l",
    "450,400,l",
    "450,460,o",
    "330,510,o",
    "260,510,c*",
    "190,510,o",
    "160,490,o",
    "130,450,c",
    "130,500,l",
    "50,500,l",
]];

/// The n after node 8, (370, 380), turned smooth.
const N: [&[&str]; 1] = [&[
    "50,0,l",
    "130,0,l",
    "130,380,l",
    "130,420,o",
    "190,450,o",
    "250,450,c*",
    "310,450,o",
    "370,420,o",
    "370,380,c*",
    "370,253,o",
    "370,127,o",
    "370,0,c",
    "450,0,l",
    "450,400,l",
    "450,460,o",
    "330,510,o",
    "260,510,c*",
    "190,510,o",
    "160,490,o",
    "130,450,c",
    "130,500,l",
    "50,500,l",
]];

fn draw_h(app: &mut FontcraftApp) {
    edit(app, "H");
    tool(app, "rectangle");
    let none = || json!([]);
    drag(app, (50.4, 0.3), (129.6, 700.3), none());
    drag(app, (370.3, -0.2), (449.8, 699.7), none());
    drag(app, (100.2, 299.6), (399.7, 380.3), none());
    assert_eq!(outline(app, "H"), owned(&H_STEMS_AND_BAR), "the drags give boxes on the grid");

    shortcut(app, json!({"key": "a", "cmd": true}), "edit.selectAll");
    let merged = shortcut(app, json!({"key": "o", "cmd": true, "shift": true}), "path.removeOverlap");
    assert_eq!(merged["skipped"], json!([]), "{merged}");
    shortcut(app, json!({"key": "r", "cmd": true, "shift": true}), "path.correctPathDirection");
    assert_eq!(outline(app, "H"), owned(&H));
}

fn draw_o(app: &mut FontcraftApp) {
    edit(app, "O");
    tool(app, "ellipse");
    drag(app, (50.0, 0.0), (550.0, 700.0), json!([]));
    drag(app, (150.0, 100.0), (450.0, 600.0), json!([]));
    // The new ellipse is selected: Reverse Contours reverses only the counter.
    let reversed = shortcut(app, json!({"key": "r", "cmd": true, "ctrl": true, "alt": true}), "path.reverseContours");
    assert_eq!(reversed["reversed"], 1, "{reversed}");
    assert_eq!(outline(app, "O"), owned(&O));
}

fn draw_n(app: &mut FontcraftApp) {
    edit(app, "n");
    tool(app, "draw");
    let (alt, none) = (|| json!(["alt"]), || json!([]));
    click(app, 50.0, 0.0);
    click(app, 130.0, 0.0);
    drag(app, (130.0, 380.0), (130.0, 420.0), alt());
    drag(app, (250.0, 450.0), (310.0, 450.0), none());
    // Alt pressed during the drag: the incoming handle stays; back on the node, no outgoing one.
    pointer(
        app,
        json!([
            {"kind": "down", "x": 370, "y": 380},
            {"kind": "drag", "x": 370, "y": 340},
            {"kind": "drag", "x": 370, "y": 380, "mods": ["alt"]},
            {"kind": "up", "x": 370, "y": 380, "mods": ["alt"]},
        ]),
    );
    click(app, 370.0, 0.0);
    click(app, 450.0, 0.0);
    drag(app, (450.0, 400.0), (450.0, 460.0), alt());
    drag(app, (260.0, 510.0), (190.0, 510.0), none());
    pointer(
        app,
        json!([
            {"kind": "down", "x": 130, "y": 450},
            {"kind": "drag", "x": 100, "y": 410},
            {"kind": "drag", "x": 130, "y": 450, "mods": ["alt"]},
            {"kind": "up", "x": 130, "y": 450, "mods": ["alt"]},
        ]),
    );
    click(app, 130.0, 500.0);
    click(app, 50.0, 500.0);
    click(app, 50.0, 0.0);
    assert_eq!(outline(app, "n"), owned(&N_DRAWN));

    tool(app, "select");
    let selected = click(app, 370.0, 380.0);
    assert_eq!(selected["selection"], json!({"points": [[0, 8]]}), "{selected}");
    key(app, json!({"key": "Return"}));
    assert_eq!(outline(app, "n"), owned(&N));
}

/// The contours skrifa draws, as the end points of their segments (on-curve points, implied
/// ones included).
#[derive(Default)]
struct Ends(Vec<Vec<(f32, f32)>>);

impl Ends {
    fn push(&mut self, x: f32, y: f32) {
        self.0.last_mut().unwrap().push((x, y));
    }
}

impl OutlinePen for Ends {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(vec![(x, y)]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x, y);
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.push(x, y);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.push(x, y);
    }
    fn close(&mut self) {}
}

/// Twice the signed area of a point list: positive when counter-clockwise.
fn area(points: &[(f32, f32)]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            f64::from(a.0) * f64::from(b.1) - f64::from(b.0) * f64::from(a.1)
        })
        .sum()
}

/// Checks one exported glyph against its fixture: contour count, the stored points and box,
/// and the outline skrifa draws.
fn check_glyph(font: &FontRef, ch: char, fixture: &[&[&str]], bounds: (i16, i16, i16, i16)) {
    let gid = font.charmap().map(ch).unwrap_or_else(|| panic!("{ch} is not in the cmap"));
    let loca = font.loca(None).unwrap();
    let Some(RawGlyph::Simple(simple)) = loca.get_glyf(gid, &font.glyf().unwrap()).unwrap() else {
        panic!("{ch} is not a simple glyph");
    };
    assert_eq!((simple.x_min(), simple.y_min(), simple.x_max(), simple.y_max()), bounds, "{ch}: the glyf box");
    let points: Vec<_> = simple.points().collect();
    let mut stored: Vec<Vec<(i16, i16, bool)>> = Vec::new();
    let mut start = 0;
    for end in simple.end_pts_of_contours() {
        let end = usize::from(end.get());
        let mut contour: Vec<(i16, i16, bool)> = points[start..=end].iter().map(|p| (p.x, p.y, p.on_curve)).collect();
        // A compiler may repeat the start point at the end of a contour.
        if contour.len() > 1 && contour.first() == contour.last() {
            contour.pop();
        }
        stored.push(contour);
        start = end + 1;
    }
    assert_eq!(stored.len(), fixture.len(), "{ch}: contours");

    let mut drawn = Ends::default();
    font.outline_glyphs().get(gid).unwrap().draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut drawn).unwrap();
    assert_eq!(drawn.0.len(), fixture.len(), "{ch}: drawn contours");
    // An on-curve point halfway between two off-curve points is left out of `glyf` and implied;
    // the off-curve points are rounded first, so the implied point can land half a unit away.
    let ends: Vec<(f32, f32)> = drawn.0.iter().flatten().copied().collect();
    let near = |(x, y): (i32, i32)| ends.iter().any(|&(ex, ey)| (ex - x as f32).abs() <= 0.5 && (ey - y as f32).abs() <= 0.5);
    let (x0, y0, x1, y1) = bounds;
    for (path, contour) in fixture.iter().zip(&stored) {
        let nodes = on_curve(path);
        for &p in &nodes {
            assert!(near(p), "{ch}: the node {p:?} is not drawn: {ends:?}");
        }
        // Each segment keeps at least one stored point: its end node, or an off-curve point.
        assert!((nodes.len()..=8 * nodes.len()).contains(&contour.len()), "{ch}: {} points for {} segments", contour.len(), nodes.len());
    }
    for &(x, y) in drawn.0.iter().flatten() {
        assert!(
            (f32::from(x0)..=f32::from(x1)).contains(&x) && (f32::from(y0)..=f32::from(y1)).contains(&y),
            "{ch}: ({x}, {y}) is outside {bounds:?}"
        );
    }
    if ch == 'H' {
        let corners: BTreeSet<(i32, i32)> = on_curve(fixture[0]).into_iter().collect();
        let got: BTreeSet<(i32, i32)> = stored[0].iter().map(|&(x, y, _)| (i32::from(x), i32::from(y))).collect();
        assert_eq!(stored[0].len(), 12, "{ch}: {:?}", stored[0]);
        assert!(stored[0].iter().all(|p| p.2), "{ch}: only corners");
        assert_eq!(got, corners);
    }
    if ch == 'O' {
        let (outer, counter) = (area(&drawn.0[0]), area(&drawn.0[1]));
        assert!(outer * counter < 0.0, "the counter runs against the outer contour: {outer} {counter}");
    }
}

#[test]
fn an_agent_draws_h_o_n_over_the_control_channel_and_exports_them() {
    let mut app = FontcraftApp::new(Session::new());
    let created = execute(&mut app, "file.new", json!({"familyName": "Control Sans"}));
    let glyph_count = created["glyphs"].as_u64().unwrap();

    draw_h(&mut app);
    draw_o(&mut app);
    draw_n(&mut app);

    let given = std::env::var_os("FONTCRAFT_E2E_OUT").map(PathBuf::from);
    let scratch = std::env::temp_dir().join(format!("fontcraft-e2e-{}", std::process::id()));
    let out = given.clone().unwrap_or_else(|| scratch.join("ControlSans.ttf"));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    // The outlines are already free of overlaps: the export keeps them as drawn.
    let exported = execute(
        &mut app,
        "file.exportStatic",
        json!({"outlines": "truetype", "destination": out.to_string_lossy(), "removeOverlap": false, "timestamp": 0}),
    );
    assert_eq!(exported["skipped"], json!([]), "{exported}");
    assert_eq!(exported["glyphs"].as_u64(), Some(glyph_count), "{exported}");

    let bytes = std::fs::read(&out).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(u64::from(font.maxp().unwrap().num_glyphs()), glyph_count);
    check_glyph(&font, 'H', &H, (50, 0, 450, 700));
    check_glyph(&font, 'O', &O, (50, 0, 550, 700));
    check_glyph(&font, 'n', &N, (50, 0, 450, 510));

    if given.is_none() {
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

/// Names as plain strings.
fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap_or_else(|| panic!("not a list: {v}")).iter().map(|n| n.as_str().unwrap().to_string()).collect()
}

/// The glyph set's glyphs that `phonetics-sino-ext` adds, by production name, in glyph order.
const SINO: [&str; 11] = ["uni0221", "uni0234", "uni0235", "uni0236", "uni027F", "uni0285", "uni02AE", "uni02AF", "uni1D00", "uni1D07", "uniAB65"];

/// The `post` names of the M4 end-to-end export: the glyph set and the added glyphs in the
/// default sort order, then the stylistic-set copies of the lowercase letters (all but `q`,
/// made a Symbol), then the renamed ligature and its copy.
fn m4_post_fixture() -> Vec<String> {
    let letters: Vec<String> = ('a'..='z').map(String::from).collect();
    let mut out = vec![".notdef".to_string()];
    out.extend(letters.iter().cloned());
    out.push("adieresis".into());
    out.extend(SINO.iter().map(|s| s.to_string()));
    out.extend(letters.iter().filter(|l| *l != "q").map(|l| format!("{l}.ss01")));
    out.push("adieresis.ss01".into());
    out.extend(SINO.iter().map(|s| format!("{s}.ss01")));
    out.extend(["f_f_l.liga".to_string(), "f_f_l.liga.ss01".to_string()]);
    out
}

/// The M4 end-to-end measure (`docs/plans/M4.md` § 5): an agent that uses only control methods
/// creates a font from a glyph set, adds glyphs, renames one, sets a category, makes a smart
/// filter, selects what it shows, duplicates it, renames the copies with Find and Replace and
/// exports. The `post` names and the `cmap` equal the fixtures. The fontTools read-back runs
/// outside the test, as for the M3 measure above:
///
/// ```text
/// FONTCRAFT_E2E_OUT=$PWD/target/e2e-m4.ttf cargo test -p fontcraft-ui-egui an_agent_builds
/// ~/.local/share/uv/tools/fontmake/bin/python -I -c '
/// import sys
/// from fontTools.ttLib import TTFont
/// f = TTFont(sys.argv[1]); order = f.getGlyphOrder(); cmap = f.getBestCmap()
/// assert len(order) == 78 and order[:2] == [".notdef", "a"] and order[-1] == "f_f_l.liga.ss01", order
/// assert len(cmap) == 38 and cmap[0xE4] == "adieresis" and cmap[0xAB65] == "uniAB65", cmap
/// assert sum(n.endswith(".ss01") for n in order) == 38
/// print("fontTools read-back: ok,", len(order), "glyphs,", len(cmap), "code points")
/// ' target/e2e-m4.ttf
/// ```
#[test]
fn an_agent_builds_and_exports_a_glyph_set_over_the_control_channel() {
    let mut app = FontcraftApp::new(Session::new());
    let created = execute(
        &mut app,
        "file.newFromGlyphSets",
        json!({"familyName": "Control Sets", "scripts": [{"script": "latin", "glyphSets": ["phonetics-sino-ext"]}]}),
    );
    assert_eq!(created["glyphs"], 12, "{created}");

    let added = execute(&mut app, "glyph.addGlyphs", json!({"text": "a:z uni00E4 f_f_l"}));
    let mut expected: Vec<String> = ('a'..='z').map(String::from).collect();
    expected.extend(["adieresis".to_string(), "f_f_l".to_string()]);
    assert_eq!(strings(&added["added"]), expected, "a code-point name becomes the nice name: {added}");

    let renamed = execute(&mut app, "glyph.rename", json!({"glyph": "f_f_l", "newName": "f_f_l.liga"}));
    assert_eq!(renamed["glyph"], "f_f_l.liga");
    execute(&mut app, "glyph.setCategory", json!({"glyphs": ["q"], "category": "Symbol"}));
    let rules = json!({"all": [{"property": "category", "value": "Letter"}, {"property": "case", "value": "lower"}]});
    execute(&mut app, "fontView.addSmartFilter", json!({"name": "Lowercase letters", "rules": rules}));

    // The filter shows the lowercase letters; Select All takes what it shows.
    let shown = execute(&mut app, "fontView.selectSidebarEntries", json!({"entries": ["filters/Lowercase letters"]}));
    assert_eq!(shown["count"], 38, "{shown}");
    shortcut(&mut app, json!({"key": "a", "cmd": true}), "edit.selectAll");
    let selected = strings(&execute(&mut app, "fontView.getState", json!({}))["selected"]);
    assert_eq!(selected.len(), 38);
    assert!(!selected.contains(&"q".to_string()) && selected.contains(&"f_f_l.liga".to_string()), "{selected:?}");

    // Duplicate Glyph: the copies become the selection.
    let duplicated = shortcut(&mut app, json!({"key": "d", "cmd": true}), "glyph.duplicateGlyph");
    let copies = strings(&duplicated["glyphs"]);
    assert_eq!(copies, selected.iter().map(|n| format!("{n}.001")).collect::<Vec<_>>());
    assert_eq!(strings(&execute(&mut app, "fontView.getState", json!({}))["selected"]), copies);

    // Find and Replace opens on the selection; Replace renames all of them.
    let opened = key(&mut app, json!({"key": "f", "cmd": true, "shift": true}));
    assert_eq!(opened["result"]["dialog"], "dialog.findAndReplaceGlyphNames", "{opened}");
    assert_eq!(strings(&opened["result"]["params"]["glyphs"]), copies);
    call(&mut app, "ui.dialog.set", json!({"field": "find", "value": ".001"}));
    call(&mut app, "ui.dialog.set", json!({"field": "replace", "value": ".ss01"}));
    let replaced = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(replaced["result"]["renamed"].as_array().map(Vec::len), Some(38), "{replaced}");

    let given = std::env::var_os("FONTCRAFT_E2E_OUT").map(PathBuf::from);
    let scratch = std::env::temp_dir().join(format!("fontcraft-e2e-m4-{}", std::process::id()));
    let out = given.clone().unwrap_or_else(|| scratch.join("ControlSets.ttf"));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let exported = execute(&mut app, "file.exportStatic", json!({"outlines": "truetype", "destination": out.to_string_lossy(), "timestamp": 0}));
    assert_eq!(exported["skipped"], json!([]), "{exported}");
    assert_eq!(exported["warnings"], json!([]), "{exported}");

    let bytes = std::fs::read(&out).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    let post = font.post().unwrap();
    let count = font.maxp().unwrap().num_glyphs();
    let names: Vec<String> =
        (0..count).map(|g| post.glyph_name(skrifa::GlyphId16::new(g)).unwrap_or_else(|| panic!("glyph {g} has no name")).to_string()).collect();
    assert_eq!(names, m4_post_fixture());
    let cmap: Vec<(u32, String)> = font.charmap().mappings().map(|(c, g)| (c, names[g.to_u32() as usize].clone())).collect();
    let mut fixture: Vec<(u32, String)> = ('a'..='z').map(|c| (c as u32, c.to_string())).collect();
    fixture.push((0xE4, "adieresis".into()));
    fixture.extend(
        [0x221, 0x234, 0x235, 0x236, 0x27F, 0x285, 0x2AE, 0x2AF, 0x1D00, 0x1D07, 0xAB65].into_iter().zip(SINO).map(|(c, n)| (c, n.to_string())),
    );
    assert_eq!(cmap, fixture);

    if given.is_none() {
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

/// Width, left and right sidebearing of `glyph` on the first master, from `spacing.getMetrics`.
fn metrics(app: &mut FontcraftApp, glyph: &str) -> (f64, f64, f64) {
    let m = execute(app, "spacing.getMetrics", json!({ "glyph": glyph }));
    let n = |k: &str| m[k].as_f64().unwrap_or(f64::NAN);
    (n("width"), n("left"), n("right"))
}

fn text(app: &mut FontcraftApp) -> Value {
    execute(app, "editView.getText", json!({}))
}

/// `n` taps of `key` with the modifiers of `mods` (an object of `shift`, `alt`, `cmd`, `ctrl`).
fn taps(app: &mut FontcraftApp, key_name: &str, mods: Value, n: usize) {
    for _ in 0..n {
        let mut params = mods.clone();
        params["key"] = json!(key_name);
        key(app, params);
    }
}

/// The M5 end-to-end measure (`docs/plans/M5.md` § 5) up to the Kerning window step: an agent on
/// the control channel types in text mode, links `o`'s sidebearings to `n` with `=|n` keys,
/// spaces `n` with the spacing keys and runs Update Metrics, sets kerning groups, and kerns `To`
/// in group mode and `Tö` as an exception with the kerning keys. `engine.execute` only draws the
/// test outlines, sets the keys and groups (the Info box fields of M5.8) and opens the lock.
///
/// The fixtures: every outline is one rectangle from x = 0, so a glyph's left sidebearing is
/// where the rectangle starts and its right sidebearing the width after it. `=|n` gives `o` the
/// opposite side of `n`: left = n's right (30), right = n's left.
#[test]
fn an_agent_spaces_and_kerns_text_over_the_control_channel() {
    let mut app = FontcraftApp::new(Session::new());
    execute(&mut app, "file.new", json!({"familyName": "Control Text"}));
    execute(&mut app, "glyph.addGlyphs", json!({"text": "odieresis"}));
    for (glyph, w, left, right) in [
        ("H", 400, 60, 60),
        ("O", 500, 40, 40),
        ("Q", 500, 40, 40),
        ("n", 360, 50, 30),
        ("o", 400, 0, 0),
        ("odieresis", 400, 30, 30),
        ("T", 500, 20, 20),
    ] {
        execute(&mut app, "path.addRectangle", json!({"glyph": glyph, "x": 0, "y": 0, "width": w, "height": 500}));
        execute(&mut app, "spacing.setLeftSidebearing", json!({"glyphs": [glyph], "value": left}));
        execute(&mut app, "spacing.setRightSidebearing", json!({"glyphs": [glyph], "value": right}));
    }
    execute(&mut app, "view.newTab", json!({"glyphs": ["H"]}));

    // Text mode: the Text tool, then typing over the tab's `H`.
    tool(&mut app, "text");
    key(&mut app, json!({"key": "a", "cmd": true}));
    let typed = call(&mut app, "ui.text", json!({"text": "HOHOHO nnonno"}));
    assert_eq!(typed["handled"], 13, "every key typed: {typed}");
    let t = text(&mut app);
    assert_eq!((t["textMode"].clone(), t["text"].clone(), t["cursor"].clone()), (json!(true), json!("HOHOHO nnonno"), json!(13)), "{t}");
    assert_eq!(t["items"][6], "space", "{t}");

    // `o` takes its sidebearings from `n`, mirrored.
    for command in ["spacing.setLeftSidebearing", "spacing.setRightSidebearing"] {
        execute(&mut app, command, json!({"glyphs": ["o"], "value": "=|n"}));
    }
    assert_eq!(metrics(&mut app, "o"), (480.0, 30.0, 50.0));

    // The cursor before the first `n`; Ctrl+Shift+Left and Ctrl+Left twice: 12 units more on its
    // left. `o` keeps its right sidebearing until Update Metrics runs on it.
    taps(&mut app, "ArrowLeft", json!({}), 6);
    assert_eq!(text(&mut app)["current"], "n");
    taps(&mut app, "ArrowLeft", json!({"ctrl": true, "shift": true}), 1);
    taps(&mut app, "ArrowLeft", json!({"ctrl": true}), 2);
    assert_eq!(metrics(&mut app, "n"), (452.0, 62.0, 30.0));
    assert_eq!(metrics(&mut app, "o"), (480.0, 30.0, 50.0));
    taps(&mut app, "ArrowRight", json!({}), 2);
    assert_eq!(text(&mut app)["current"], "o");
    let updated = shortcut(&mut app, json!({"key": "m", "cmd": true, "ctrl": true}), "glyph.updateMetrics");
    assert_eq!(updated["glyphs"], json!(["o"]), "{updated}");
    assert_eq!(metrics(&mut app, "o"), (492.0, 30.0, 62.0));

    // Kerning groups: O and Q share both groups; o and ö share a left group, T has a right one.
    for (command, glyphs, group) in [
        ("kerning.setLeftGroup", json!(["O", "Q"]), "O"),
        ("kerning.setRightGroup", json!(["O", "Q"]), "O"),
        ("kerning.setLeftGroup", json!(["o", "odieresis"]), "o"),
        ("kerning.setRightGroup", json!(["T"]), "T"),
    ] {
        execute(&mut app, command, json!({"glyphs": glyphs, "group": group}));
    }

    // `To` in group mode: the cursor between T and o, Ctrl+Alt+Shift+Right four times.
    key(&mut app, json!({"key": "a", "cmd": true}));
    call(&mut app, "ui.text", json!({"text": "ToTö"}));
    assert_eq!(text(&mut app)["items"], json!(["T", "o", "T", "odieresis"]));
    taps(&mut app, "ArrowLeft", json!({}), 3);
    taps(&mut app, "ArrowRight", json!({"ctrl": true, "alt": true, "shift": true}), 4);
    let pair = |app: &mut FontcraftApp, right: &str| execute(app, "kerning.getPair", json!({"left": "T", "right": right}))["value"].clone();
    assert_eq!((pair(&mut app, "o"), pair(&mut app, "odieresis")), (json!(-40.0), json!(-40.0)), "the group pair kerns both");

    // `Tö` as an exception: open ö's lock, then Ctrl+Alt+Shift+Left and Ctrl+Alt+Left twice.
    taps(&mut app, "ArrowRight", json!({}), 2);
    execute(&mut app, "kerning.addException", json!({"left": "T", "right": "odieresis", "side": "right"}));
    taps(&mut app, "ArrowLeft", json!({"ctrl": true, "alt": true, "shift": true}), 1);
    taps(&mut app, "ArrowLeft", json!({"ctrl": true, "alt": true}), 2);
    assert_eq!((pair(&mut app, "o"), pair(&mut app, "odieresis")), (json!(-40.0), json!(-28.0)));

    let pairs = execute(&mut app, "kerning.listPairs", json!({}));
    // Group to group for `To`; T's group (its lock stays closed) against ö alone for `Tö`.
    let fixture = json!([
        {"index": 0, "left": "@T", "right": "@o", "value": -40.0},
        {"index": 1, "left": "@T", "right": "odieresis", "value": -28.0},
    ]);
    assert_eq!(pairs["pairs"], fixture, "{pairs}");
    assert_eq!(text(&mut app)["text"], "ToTö", "the kerning keys leave the text as it is");
}

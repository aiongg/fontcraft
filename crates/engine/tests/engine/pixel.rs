//! Pixel tool commands (M6.13): `pixel.addPixelGlyph` and `pixel.setPixels` in each mode, the
//! grid spacing they need, the missing pixel glyph, and their limits.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_model::{Alignment, NodeKind, Shape};
use fontcraft_testkit::snapshot;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{id} {p} failed but changed the session");
    e
}

/// The pixels of `glyph`'s first master layer as cells of a 50-unit grid, in shape order, each
/// checked to be a pixel component with alignment off.
fn pixels(s: &Session, glyph: &str) -> Vec<(f64, f64)> {
    let font = s.font().unwrap();
    let layer = font.glyph(glyph).unwrap().layer(&font.masters[0].id).unwrap();
    layer
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Component(c) => {
                assert_eq!(c.glyph, "pixel");
                assert_eq!(c.info.alignment, Alignment::Disabled, "pixels are never aligned automatically");
                let m = c.affine().as_coeffs();
                Some((m[4] / 50.0, m[5] / 50.0))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn pixels_are_painted_toggled_and_removed_on_the_grid() {
    let mut s = Session::with_document();
    let paint = |cells: Value, mode: &str| json!({"glyph": "A", "cells": cells, "mode": mode});

    // The tool needs a grid spacing of 2 or more.
    let e = run_err(&mut s, "pixel.setPixels", paint(json!([[0, 0]]), "add"));
    assert!(e.contains("Grid Spacing"), "{e}");
    run(&mut s, "font.settings.set", json!({"gridSpacing": 50}));

    // Without the glyph named pixel nothing changes and the result says why.
    let before = snapshot(&s);
    let r = run(&mut s, "pixel.setPixels", json!({"glyph": "A", "stroke": [[10, 10]]}));
    assert_eq!(r["pixelGlyphMissing"], true);
    // The command succeeded, so the journal records it; the fonts and history stay as they were.
    let after = snapshot(&s);
    assert_eq!((&after["fonts"], &after["inspect"]), (&before["fonts"], &before["inspect"]));

    // The default pixel is a square as large as the grid spacing, on every master.
    let r = run(&mut s, "pixel.addPixelGlyph", json!({}));
    assert_eq!((r["name"].as_str(), r["size"].as_f64()), (Some("pixel"), Some(50.0)));
    let font = s.font().unwrap();
    let pixel = font.glyph("pixel").unwrap();
    assert_eq!(pixel.layers.len(), font.masters.len());
    for layer in &pixel.layers {
        assert_eq!(layer.width, 50.0);
        let [Shape::Path(square)] = layer.shapes.as_slice() else { panic!("{:?}", layer.shapes) };
        let corners: Vec<(f64, f64)> = square.nodes.iter().map(|n| (n.x, n.y)).collect();
        assert_eq!(corners, [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0)]);
        assert!(square.closed && square.nodes.iter().all(|n| n.kind == NodeKind::Line));
    }
    assert!(run_err(&mut s, "pixel.addPixelGlyph", json!({})).contains("already"));

    let r = run(&mut s, "pixel.setPixels", paint(json!([[1, 0], [2, 3], [1, 0]]), "add"));
    assert_eq!((r["added"].clone(), r["removed"].clone()), (json!([[1, 0], [2, 3]]), json!([])), "repeated cells count once");
    assert_eq!(pixels(&s, "A"), [(1.0, 0.0), (2.0, 3.0)]);

    let r = run(&mut s, "pixel.setPixels", paint(json!([[1, 0], [0, 0]]), "toggle"));
    assert_eq!((r["added"].clone(), r["removed"].clone()), (json!([[0, 0]]), json!([[1, 0]])));
    assert_eq!(pixels(&s, "A"), [(2.0, 3.0), (0.0, 0.0)]);

    // A stroke that starts on a pixel removes; one that starts on an empty cell adds, in every
    // cell it crosses.
    let r = run(&mut s, "pixel.setPixels", json!({"glyph": "A", "stroke": [[10, 10], [160, 10]]}));
    assert_eq!((r["mode"].as_str(), r["removed"].clone(), r["added"].clone()), (Some("remove"), json!([[0, 0]]), json!([])));
    let r = run(&mut s, "pixel.setPixels", json!({"glyph": "A", "stroke": [[60, 10], [160, 10], [160, 60]]}));
    assert_eq!((r["mode"].as_str(), r["added"].clone()), (Some("add"), json!([[1, 0], [2, 0], [3, 0], [3, 1]])));
    assert_eq!(pixels(&s, "A"), [(2.0, 3.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (3.0, 1.0)]);

    // Removing what is not there, or adding what is, changes nothing and makes no undo step.
    let before = snapshot(&s);
    run(&mut s, "pixel.setPixels", paint(json!([[9, 9]]), "remove"));
    run(&mut s, "pixel.setPixels", paint(json!([[1, 0]]), "add"));
    let after = snapshot(&s);
    assert_eq!((&after["fonts"], &after["inspect"]), (&before["fonts"], &before["inspect"]));

    // Refusals.
    assert!(run_err(&mut s, "pixel.setPixels", json!({"glyph": "pixel", "cells": [[0, 0]], "mode": "add"})).contains("contain itself"));
    run_err(&mut s, "pixel.setPixels", json!({"glyph": "A", "cells": [[0.5, 0]]}));
    run_err(&mut s, "pixel.setPixels", json!({"glyph": "A", "cells": [[0, 0]], "stroke": [[0, 0]]}));
    run_err(&mut s, "pixel.setPixels", json!({"glyph": "A", "cells": [[0, 0]], "mode": "spray"}));
    run_err(&mut s, "pixel.setPixels", json!({"glyph": "A", "cells": [[0, 0]], "side": "background"}));
    let many: Vec<[i64; 2]> = (0..65_537).map(|i| [i % 300, i / 300]).collect();
    assert!(run_err(&mut s, "pixel.setPixels", json!({"glyph": "A", "cells": many})).contains("65536"));
    let e = run_err(&mut s, "pixel.setPixels", json!({"glyph": "A", "stroke": [[-1e6, -1e6], [1e6, 1e6]]}));
    assert!(e.contains("crosses"), "{e}");
}

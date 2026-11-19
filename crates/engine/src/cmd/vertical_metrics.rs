//! The vertical origin (O) and vertical width (H) of a glyph layer, set to a number or a metrics
//! key the way the five metric setters of `cmd/spacing.rs` set theirs, which they share. Behaviour
//! notes: `docs/spec/08-spacing-kerning.md` § 1 and § 2.

use fontcraft_spacing::Field;
use serde_json::Value;

use super::*;
use crate::params::TargetKind;

pub fn specs() -> Vec<CommandSpec> {
    let g: &'static [TargetKind] = &[TargetKind::Glyphs];
    vec![
        cmd!(
            "spacing.setVerticalOrigin",
            "Set Vertical Origin",
            [],
            None,
            "{glyphs: [names], layer?: layer id (default: the first master), value: number or metrics key} set the vertical origin of each glyph's layer: how far below the ascender the top of its vertical advance lies (a layer without one has 0). The vertical width stays, so the top and bottom sidebearings change by the same amount in opposite directions. A number removes the key the field holds; a key (`n`, `=n`, `=n+20`, `=-40`, `==u` for this layer only; no `|` or `@`) is stored and applied at once. Empty layers are set too → {glyphs: [names changed], layer, values: {glyph: vertical origin}, key?: text as shown, skipped: []}",
            has_doc,
            set_vertical_origin
        )
        .targets(g),
        cmd!(
            "spacing.setVerticalWidth",
            "Set Vertical Width",
            [],
            None,
            "{glyphs: [names], layer?: layer id (default: the first master), value: number or metrics key} set the vertical width (the vertical advance) of each glyph's layer, from 0 to 65535 (a layer without one has the master's ascender to descender). The vertical origin stays, so the bottom sidebearing takes the change. A number removes the key the field holds; a key (`n`, `=n`, `=n*2`, `=1000`, `==u` for this layer only; no `|` or `@`) is stored and applied at once. Empty layers are set too → {glyphs: [names changed], layer, values: {glyph: vertical width}, key?: text as shown, skipped: []}",
            has_doc,
            set_vertical_width
        )
        .targets(g),
    ]
}

fn set_vertical_origin(s: &mut Session, v: &Value) -> Result<Value> {
    super::spacing::set_metric(s, v, "spacing.setVerticalOrigin", "Set Vertical Origin", Field::VertOrigin)
}

fn set_vertical_width(s: &mut Session, v: &Value) -> Result<Value> {
    super::spacing::set_metric(s, v, "spacing.setVerticalWidth", "Set Vertical Width", Field::VertWidth)
}

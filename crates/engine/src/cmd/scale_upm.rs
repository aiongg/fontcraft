//! `font.info.scaleToUnitsPerEm`: the whole font scaled to a new units-per-em through
//! `fontcraft_fontinfo::scale`, one undo step. Typing a new value with `font.info.set` changes
//! the units per em alone; the two together make the resize trick (`fontInfo.unitsPerEm.resizeTrick`).

use fontcraft_model::UPM_RANGE;
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "font.info.scaleToUnitsPerEm",
        "Scale Font to Units per Em",
        [],
        None,
        "{unitsPerEm: 16–16384} set the units per em and scale every value in font units by new ÷ old (outlines, component offsets, anchors, guides, hints, backgrounds, widths, kerning, metrics, zones, stems and the parameters measured in units), rounded to the grid, as one undo step; refused when a value would pass ±32768 → {unitsPerEm, previous}",
        has_doc,
        scale_to_upm
    )]
}

fn scale_to_upm(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.scaleToUnitsPerEm", p)?;
    let upm =
        p.opt_int("unitsPerEm", i64::from(*UPM_RANGE.start()), i64::from(*UPM_RANGE.end()))?.ok_or_else(|| p.bad("`unitsPerEm` is required"))?;
    let upm = u16::try_from(upm).map_err(|_| p.bad(format!("`unitsPerEm` must be from {} to {}", UPM_RANGE.start(), UPM_RANGE.end())))?;
    let previous = s.font()?.units_per_em;
    s.edit("Scale Font to Units per Em", |font| {
        *font = fontcraft_fontinfo::scale::scale_font(font, upm).map_err(|e| p.bad(e.to_string()))?;
        Ok(())
    })?;
    Ok(json!({ "unitsPerEm": upm, "previous": previous }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Number, Value as PValue};
    use fontcraft_model::{
        AlignmentZone, Anchor, Background, Component, Guide, Hint, KernFirst, KernTable, Kerning, LayerInfo, MasterKerning, Param, Path, Rare, Shape,
        StemDef, Transform,
    };
    use serde_json::json;

    use crate::Session;

    fn run(s: &mut Session, id: &str, p: serde_json::Value) -> serde_json::Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    /// A font holding one of everything the scaling touches: a path, a transformed component, an
    /// anchor, a layer guide, a hint, a background, a brace layer, kerning, metrics, a zone, a
    /// stem and a typoAscender parameter.
    fn session() -> Session {
        let mut s = Session::with_document();
        s.edit("setup", |font| {
            {
                let mut g = font.glyph_mut("A")?;
                let mut brace = g.layer("m01").cloned().unwrap();
                let l = g.layer_mut("m01").unwrap();
                l.shapes.push(Shape::Path(Path::rect(15.0, 0.0, 333.0, 701.0)));
                l.anchors.push(Anchor::new("top", 301.0, 701.0));
                let mut place = fontcraft_model::plist::Dict::new();
                place.insert(
                    "place",
                    PValue::Array(fontcraft_model::plist::Array {
                        items: vec![PValue::Number(Number::int(120)), PValue::Number(Number::int(-21))],
                        ..Default::default()
                    }),
                );
                l.info = Rare::new(LayerInfo {
                    guides: vec![Guide { x: 11.0, y: 250.0, angle: 90.0, ..Default::default() }],
                    hints: vec![Hint { family: fontcraft_model::Family::Glyphs, value: PValue::Dict(place) }],
                    background: Some(Background { shapes: vec![Shape::Path(Path::rect(1.0, 3.0, 5.0, 7.0))], ..Default::default() }),
                    ..Default::default()
                });
                brace.id = "brace-1".into();
                brace.width = 640.0;
                brace.shapes.push(Shape::Path(Path::rect(17.0, 0.0, 351.0, 703.0)));
                brace.info.attrs.coordinates = vec![Some(100.0)];
                g.layers.push(brace);
            }
            {
                let mut g = font.glyph_mut("B")?;
                let l = g.layer_mut("m01").unwrap();
                let mut c = Component::new("A", 13.0, -9.0);
                c.transform = Transform::Decomposed { pos: (13.0, -9.0), scale: (0.75, 0.5), angle: 10.0, slant: (0.0, 0.0) };
                // Kept where it is: automatic alignment would place it at the origin.
                c.info.alignment = fontcraft_model::Alignment::Disabled;
                l.shapes.push(Shape::Component(c));
            }
            font.stems = Arc::new(vec![StemDef { name: "Stem".into(), ..Default::default() }]);
            let m = &mut Arc::make_mut(&mut font.masters)[0];
            m.stem_values = vec![83.0];
            m.zones = vec![AlignmentZone { pos: 550.0, size: 13.0 }];
            m.params.0.push(Param::new("typoAscender", PValue::Number(Number::int(777))));
            font.kerning = Arc::new(Kerning {
                ltr: KernTable(vec![MasterKerning {
                    master: "m01".into(),
                    firsts: vec![KernFirst { first: "A".into(), seconds: vec![("B".into(), -37.0)] }],
                }]),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
        s
    }

    fn node_xs(s: &Session, glyph: &str, layer: &str) -> Vec<f64> {
        let font = s.font().unwrap();
        let l = font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap();
        l.shapes
            .iter()
            .flat_map(|shape| match shape {
                Shape::Path(p) => p.nodes.iter().map(|n| n.x).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn font_info_scale_to_units_per_em_scales_everything_and_scales_back() {
        let mut s = session();
        let before = s.font().unwrap().clone();
        let r = run(&mut s, "font.info.scaleToUnitsPerEm", json!({"unitsPerEm": 2048}));
        assert_eq!(r, json!({"unitsPerEm": 2048, "previous": 1000}));
        {
            let font = s.font().unwrap();
            assert_eq!(font.units_per_em, 2048);
            let a = font.glyph("A").unwrap();
            let l = a.layer("m01").unwrap();
            assert_eq!(l.width, 1229.0); // 600 × 2.048 = 1228.8
            assert_eq!(l.anchors[0].x, 616.0); // 616.448
            assert_eq!(l.info.guides[0].y, 512.0);
            assert_eq!(l.info.guides[0].angle, 90.0);
            let place = l.info.hints[0].value.as_dict().unwrap().get("place").unwrap().as_array().unwrap();
            assert_eq!(place.items, vec![PValue::Number(Number::int(246)), PValue::Number(Number::int(-43))]);
            let brace = a.layers.iter().find(|l| l.id == "brace-1").unwrap();
            assert_eq!(brace.width, 1311.0); // 1310.72
            assert_eq!(brace.info.attrs.coordinates, vec![Some(100.0)]);
            let Shape::Component(c) = &font.glyph("B").unwrap().layer("m01").unwrap().shapes[0] else { panic!("component") };
            assert_eq!(c.transform, Transform::Decomposed { pos: (27.0, -18.0), scale: (0.75, 0.5), angle: 10.0, slant: (0.0, 0.0) });
            let m = &font.masters[0];
            assert_eq!(m.metric_values[0].pos, 1638.0); // ascender 800 → 1638.4
            assert_eq!(m.stem_values, vec![170.0]);
            assert_eq!(m.zones[0], AlignmentZone { pos: 1126.0, size: 27.0 });
            assert_eq!(m.params.get("typoAscender"), Some(&PValue::Number(Number::int(1591))));
            assert_eq!(font.kerning.ltr.get("m01", "A", "B"), Some(-76.0));
        }
        let mut xs = node_xs(&s, "A", "brace-1");
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        assert_eq!(xs, vec![35.0, 719.0]); // 17 and 351 × 2.048, rounded
        run(&mut s, "font.info.scaleToUnitsPerEm", json!({"unitsPerEm": 1000}));
        assert!(*s.font().unwrap() == before, "1000 → 2048 → 1000 changed the font");
    }

    #[test]
    fn font_info_unitsperem_resize_trick_is_two_undo_steps() {
        let mut s = session();
        let start = s.font().unwrap().clone();
        let steps = s.doc().unwrap().history.undo.len();
        run(&mut s, "font.info.set", json!({"values": {"unitsPerEm": 800}}));
        let xs = node_xs(&s, "A", "m01");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node_xs(&s, "A", "m01"), xs, "typing a new units per em leaves the outlines alone");
        s.execute("edit.redo", &json!({})).unwrap();
        run(&mut s, "font.info.scaleToUnitsPerEm", json!({"unitsPerEm": 1000}));
        let font = s.font().unwrap();
        assert_eq!(font.units_per_em, 1000);
        assert_eq!(font.glyph("A").unwrap().layer("m01").unwrap().width, 750.0, "600 at 800 units scaled back to 1000 grows by 1.25");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 2);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.font().unwrap().units_per_em, 800);
        assert_eq!(s.font().unwrap().glyph("A").unwrap().layer("m01").unwrap().width, 600.0);
        run(&mut s, "edit.undo", json!({}));
        assert!(*s.font().unwrap() == start);
    }

    #[test]
    fn font_info_scale_to_units_per_em_refuses_past_the_coordinate_limit() {
        let mut s = session();
        s.edit("far anchor", |font| {
            font.glyph_mut("C")?.layer_mut("m01").unwrap().anchors.push(Anchor::new("far", 9000.0, 0.0));
            Ok(())
        })
        .unwrap();
        let before = s.font().unwrap().clone();
        let steps = s.doc().unwrap().history.undo.len();
        let err = s.execute("font.info.scaleToUnitsPerEm", &json!({"unitsPerEm": 4096})).unwrap_err().to_string();
        assert!(err.contains("glyph `C`"), "{err}");
        assert!(*s.font().unwrap() == before);
        assert_eq!(s.doc().unwrap().history.undo.len(), steps);
        assert!(s.execute("font.info.scaleToUnitsPerEm", &json!({"unitsPerEm": 20000})).is_err());
        assert!(s.execute("font.info.scaleToUnitsPerEm", &json!({})).is_err());
    }
}

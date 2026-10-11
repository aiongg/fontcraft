#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Anchor, Font, Layer, Master, NumberDef, Path, Shape};

use super::{MAX_EXPRESSION_BYTES, MAX_TOKENS, TokenValue, evaluate, expand, token_ranges};
use crate::{MasterValues, TargetValues};

/// Sets `glyph`'s layer of `master`: its width, an optional rectangle (x0, y0, x1, y1) and its
/// anchors. The layer is added when the glyph has none for the master.
fn layer(f: &mut Font, glyph: &str, master: &str, width: f64, rect: Option<(f64, f64, f64, f64)>, anchors: &[(&str, f64, f64)]) {
    let mut g = f.glyph_mut(glyph).unwrap();
    if g.layer(master).is_none() {
        g.layers.push(Layer::new(master, width));
    }
    let l = g.layer_mut(master).unwrap();
    l.width = width;
    l.shapes.clear();
    if let Some((x0, y0, x1, y1)) = rect {
        l.shapes.push(Shape::Path(Path::rect(x0, y0, x1, y1)));
    }
    l.anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
}

/// Two masters with the number values `tracking` (10, 30) and `shift` (−4, 6); `period` and `b`
/// with widths and anchors per master. The same font as the reference library's run that gave
/// the expected texts of [`expansion_equals_the_reference_library_on_both_masters`].
fn font() -> Font {
    let mut f = Font::new("Tokens");
    f.numbers = Arc::new(vec![NumberDef { name: "tracking".into(), ..Default::default() }, NumberDef { name: "shift".into(), ..Default::default() }]);
    {
        let masters = Arc::make_mut(&mut f.masters);
        masters[0].number_values = vec![10.0, -4.0];
        masters.push(Master { id: "m02".into(), name: "Bold".into(), number_values: vec![30.0, 6.0], ..Default::default() });
    }
    f.add_glyph("period", vec![0x2E], 250.0).unwrap();
    layer(&mut f, "period", "m01", 250.0, Some((50.0, 0.0, 200.0, 100.0)), &[("top", 125.0, 500.0)]);
    layer(&mut f, "period", "m02", 290.0, Some((60.0, 0.0, 220.0, 120.0)), &[("top", 145.0, 520.0)]);
    layer(&mut f, "b", "m01", 600.0, None, &[("top_special", 300.0, 710.0)]);
    layer(&mut f, "b", "m02", 640.0, None, &[("top_special", 320.0, 730.0)]);
    f
}

fn expanded(f: &Font, data: &GlyphData, master: &str, code: &str) -> String {
    let values = MasterValues::new(f, data, master).unwrap();
    match expand(code, &values) {
        Ok((text, _)) => text,
        Err(d) => panic!("`{code}` on {master}: {d:?}"),
    }
}

#[test]
fn expansion_equals_the_reference_library_on_both_masters() {
    // Expected texts: the reference library's token expander (glyphsLib `TokenExpander`) on the
    // same font, saved as a source file. Every value is a whole number, so rounding plays no part.
    let cases = [
        ("pos @Uppercase <$tracking 0 ${tracking*2} 0>;", "pos @Uppercase <10 0 20 0>;", "pos @Uppercase <30 0 60 0>;"),
        ("pos period <${tracking + shift} 0 ${tracking * 2 - shift} 0>;", "pos period <6 0 24 0>;", "pos period <36 0 54 0>;"),
        ("pos period <${(tracking + 2) * 3} 0 ${2 * tracking + 10 / 5} 0>;", "pos period <36 0 22 0>;", "pos period <96 0 62 0>;"),
        ("pos period <${period:width} 0 ${period:width*2} 0>;", "pos period <250 0 500 0>;", "pos period <290 0 580 0>;"),
        (
            "pos base b <anchor ${b:anchors.top_special.x} ${b:anchors.top_special.y}> mark @Top;",
            "pos base b <anchor 300 710> mark @Top;",
            "pos base b <anchor 320 730> mark @Top;",
        ),
        ("pos b <${b:width - tracking} 0 ${b:anchors.top_special.x / 2 + shift} 0>;", "pos b <590 0 146 0>;", "pos b <610 0 166 0>;"),
        ("pos period ${shift};", "pos period -4;", "pos period 6;"),
    ];
    let f = font();
    let data = GlyphData::new().unwrap();
    for (code, regular, bold) in cases {
        assert_eq!(expanded(&f, &data, "m01", code), regular, "{code}");
        assert_eq!(expanded(&f, &data, "m02", code), bold, "{code}");
    }
}

#[test]
fn handbook_examples_expand_with_sidebearings_and_anchors() {
    let f = font();
    let data = GlyphData::new().unwrap();
    // `period` is a rectangle from x 50 to 200 in a 250 advance on the first master.
    assert_eq!(expanded(&f, &data, "m01", "pos a.alt <${period:LSB} 0 ${period:LSB*2} 0>;"), "pos a.alt <50 0 100 0>;");
    assert_eq!(expanded(&f, &data, "m01", "pos a <${period:RSB} 0 ${period:width} 0>;"), "pos a <50 0 250 0>;");
    assert_eq!(expanded(&f, &data, "m02", "pos a.alt <${period:LSB} 0 ${period:RSB*3} 0>;"), "pos a.alt <60 0 210 0>;");
    // Kerning code with tokens (`kerning.kernCodeTokens`).
    assert_eq!(expanded(&f, &data, "m02", "pos period period ${-tracking};"), "pos period period -30;");
    assert_eq!(
        expanded(&f, &data, "m01", "pos base b <anchor\n\t${b:anchors.top_special.x}\n\t${b:anchors.top_special.y}> mark @SpecialTopMark;"),
        "pos base b <anchor\n\t300\n\t710> mark @SpecialTopMark;"
    );
}

#[test]
fn results_round_half_away_from_zero() {
    let f = font();
    let data = GlyphData::new().unwrap();
    // 2.5 and −0.5: the reference library truncates to 2 and 0; FontCraft rounds as the
    // feature compiler does (`token.numberValue.expressionDetails`).
    assert_eq!(expanded(&f, &data, "m01", "${tracking/4} ${shift/8}"), "3 -1");
    assert_eq!(expanded(&f, &data, "m01", "${2 + 4 * 5 / 7 - 3}"), "2");
    assert_eq!(expanded(&f, &data, "m01", "${1.25 * 2} ${-(tracking - 12.5)}"), "3 3");
}

#[test]
fn bad_tokens_are_errors_at_the_token() {
    let f = font();
    let data = GlyphData::new().unwrap();
    let values = MasterValues::new(&f, &data, "m01").unwrap();
    let huge = format!("${{1{}}}", "0".repeat(400));
    let cases: Vec<(String, &str, &str)> = vec![
        ("pos a ${tracking/0};".into(), "${tracking/0}", "divides by zero"),
        (format!("pos a {huge};"), huge.as_str(), "too large"),
        ("pos a $nothing;".into(), "$nothing", "no number value `nothing`"),
        ("pos a ${tracking * 40000};".into(), "${tracking * 40000}", "beyond"),
        ("pos a ${b:bounds};".into(), "${b:bounds}", "`bounds` is not a glyph property"),
        ("pos a ${b:anchors.top.x};".into(), "${b:anchors.top.x}", "no anchor `top`"),
        ("pos a ${b:anchors.top_special.z};".into(), "${b:anchors.top_special.z}", "not an anchor property"),
        ("pos a ${b:LSB};".into(), "${b:LSB}", "no outline"),
        ("pos a ${nope:width};".into(), "${nope:width}", "no glyph `nope`"),
        ("pos a ${(tracking};".into(), "${(tracking}", "`)` is missing"),
        ("pos a ${tracking 2};".into(), "${tracking 2}", "not expected"),
        ("pos a ${};".into(), "${}", "empty"),
        ("pos a $ 0;".into(), "$", "must start a token"),
        ("pos a <${a:anchors.\n".into(), "${a:anchors.", "not closed"),
        ("@A = [$[name endswith \".sc\"\n;".into(), "$[name endswith \".sc\"\n", "not closed"),
    ];
    for (code, token, message) in &cases {
        let errors = expand(code, &values).expect_err(code);
        assert_eq!(errors.len(), 1, "{code}: {errors:?}");
        let start = code.find(token).unwrap();
        assert_eq!(errors[0].span, start..start + token.len(), "{code}");
        assert!(errors[0].message.contains(message), "{code}: {}", errors[0].message);
        assert!(errors[0].is_error());
    }
    // Every bad token is reported, not only the first.
    let errors = expand("pos a $x; pos b ${y}; pos c $tracking;", &values).unwrap_err();
    assert_eq!(errors.len(), 2);
}

#[test]
fn comments_and_strings_keep_their_dollar_signs() {
    let f = font();
    let data = GlyphData::new().unwrap();
    let code = "# costs $5 and ${broken\nfeatureNames { name \"$ and $[\"; };\npos period $tracking; # $shift";
    assert_eq!(expanded(&f, &data, "m01", code), "# costs $5 and ${broken\nfeatureNames { name \"$ and $[\"; };\npos period 10; # $shift");
    // Code without tokens is returned as it is, with the identity map.
    let values = MasterValues::new(&f, &data, "m01").unwrap();
    let (text, map) = expand("sub a by b;", &values).unwrap();
    assert_eq!(text, "sub a by b;");
    assert!(map.is_identity());
}

#[test]
fn the_offset_map_leads_back_to_the_stored_code() {
    let f = font();
    let data = GlyphData::new().unwrap();
    let values = MasterValues::new(&f, &data, "m01").unwrap();
    let code = "pos @A <${tracking * 2} 0 $tracking 0>;\nsub x by y;";
    let (text, map) = expand(code, &values).unwrap();
    assert_eq!(text, "pos @A <20 0 10 0>;\nsub x by y;");
    assert_eq!(map.source_offset(text.find("sub").unwrap()), code.find("sub").unwrap());
    assert_eq!(map.source_offset(text.find(" 0 10").unwrap()), code.find(" 0 $tracking").unwrap());
    assert_eq!(map.source_offset(0), 0);
}

#[test]
fn hostile_tokens_are_refused_with_a_message() {
    let f = font();
    let data = GlyphData::new().unwrap();
    let values = MasterValues::new(&f, &data, "m01").unwrap();
    let many = "$tracking ".repeat(MAX_TOKENS + 1);
    let errors = expand(&many, &values).unwrap_err();
    assert!(errors.iter().any(|e| e.message.contains("more than 100000 tokens")));
    assert_eq!(token_ranges(&many).len(), MAX_TOKENS);

    let deep = format!("${{{}tracking{}}}", "(".repeat(1_000), ")".repeat(1_000));
    let errors = expand(&deep, &values).unwrap_err();
    assert!(errors[0].message.contains("nests more than"), "{}", errors[0].message);
    let signs = format!("${{{}1}}", "-".repeat(1_000));
    assert!(expand(&signs, &values).unwrap_err()[0].message.contains("nests more than"));

    let long = format!("${{{}}}", "1+".repeat(MAX_EXPRESSION_BYTES) + "1");
    assert!(expand(&long, &values).unwrap_err()[0].message.contains("longer than 4096 bytes"));
    let unterminated = "${a:anchors.".repeat(10_000);
    assert!(!expand(&unterminated, &values).unwrap_err().is_empty());
    assert!(evaluate(&long, &values).is_err());
}

#[test]
fn evaluate_gives_one_tokens_value_for_the_master() {
    let f = font();
    let data = GlyphData::new().unwrap();
    let regular = MasterValues::new(&f, &data, "m01").unwrap();
    let bold = MasterValues::new(&f, &data, "m02").unwrap();
    assert_eq!(evaluate("$tracking", &regular), Ok(TokenValue::Number(10)));
    assert_eq!(evaluate(" $tracking ", &bold), Ok(TokenValue::Number(30)));
    assert_eq!(evaluate("${b:width}", &bold), Ok(TokenValue::Number(640)));
    assert_eq!(evaluate("${period:anchors.top.y - tracking}", &bold), Ok(TokenValue::Number(490)));
    assert!(evaluate("$tracking $shift", &regular).unwrap_err().contains("not one token"));
    assert!(evaluate("pos $tracking", &regular).unwrap_err().contains("not one token"));
    assert!(evaluate("$missing", &regular).unwrap_err().contains("no number value"));
    assert!(evaluate("", &regular).is_err());
}

#[test]
fn token_ranges_find_every_token_outside_comments() {
    let code = "pos a <$t ${t*2} 0 0>; @B = [$[name endswith \"]\"]]; # $c\n$";
    let ranges = token_ranges(code);
    let texts: Vec<&str> = ranges.iter().map(|r| &code[r.clone()]).collect();
    assert_eq!(texts, ["$t", "${t*2}", "$[name endswith \"]\"]", "$"]);
}

#[test]
fn master_values_read_the_master_layer() {
    let mut f = font();
    let data = GlyphData::new().unwrap();
    assert!(MasterValues::new(&f, &data, "m09").is_err());
    f.glyph_mut("period").unwrap().export = false;
    let values = MasterValues::new(&f, &data, "m02").unwrap();
    assert!(!values.glyphs().iter().any(|g| g == "period"));
    assert_eq!(values.glyphs().first().map(String::as_str), Some(".notdef"));
    assert_eq!(values.number_value("shift"), Some(6.0));
    assert_eq!(values.number_value("Shift"), None);
    assert_eq!(values.anchor("period", "top"), Some((145.0, 520.0)));
    assert_eq!(values.glyph_metric("A", crate::GlyphMetric::Width), None, "A has no layer on the second master");
    drop(values);

    // Propagated anchors replace the stored ones when the compiler passes them.
    let values =
        MasterValues::new(&f, &data, "m01").unwrap().with_anchors(Box::new(|g: &str, a: &str| (g == "b" && a == "top").then_some((1.0, 2.0))));
    assert_eq!(values.anchor("b", "top"), Some((1.0, 2.0)));
    assert_eq!(values.anchor("b", "top_special"), None);
    drop(values);
    // A master that stores fewer values than the font has names reads zero.
    Arc::make_mut(&mut f.masters)[0].number_values.clear();
    let values = MasterValues::new(&f, &data, "m01").unwrap();
    assert_eq!(values.number_value("tracking"), Some(0.0));
}

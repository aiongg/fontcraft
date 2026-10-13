#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use std::collections::HashMap;
use std::path::Path;

use super::*;

fn oracle_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/variable_syntax")
}

fn axes(font: &str) -> FeatureAxes {
    let text = std::fs::read_to_string(oracle_dir().join(format!("{font}.glyphs"))).unwrap();
    FeatureAxes::from_font(&fontcraft_io_glyphs::read(&text).unwrap())
}

/// The whole pass as the assembly runs it for the variable font: directives, then the syntax.
fn variable(code: &str, axes: &FeatureAxes) -> Result<String, Vec<String>> {
    let (lowered, _) = crate::lower::lower(code, true).map_err(|d| d.into_iter().map(|d| d.message).collect::<Vec<_>>())?;
    lower_variable_syntax(&lowered, axes, &LowerTarget::Variable).map(|l| l.text).map_err(|e| e.into_iter().map(|e| e.message).collect())
}

fn at(code: &str, axes: &FeatureAxes, location: &[f64]) -> String {
    let (lowered, _) = crate::lower::lower(code, false).unwrap();
    lower_variable_syntax(&lowered, axes, &LowerTarget::Static(location.to_vec())).unwrap().text
}

fn refused(code: &str, axes: &FeatureAxes) -> VariableSyntaxError {
    let errors = lower_variable_syntax(code, axes, &LowerTarget::Variable).unwrap_err();
    errors.into_iter().next().unwrap()
}

/// Lines trimmed, blank lines dropped, runs of white space as one space, `600.0` as `600`, and
/// condition sets renamed in the order they first appear.
fn normalise(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        out.push_str(&line.split_whitespace().collect::<Vec<_>>().join(" "));
        out.push('\n');
    }
    let b = out.as_bytes();
    let mut numbers = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'.' && b.get(i + 1) == Some(&b'0') && i > 0 && b[i - 1].is_ascii_digit() && !b.get(i + 2).is_some_and(u8::is_ascii_digit) {
            i += 2;
            continue;
        }
        numbers.push(b[i] as char);
        i += 1;
    }
    let mut names: HashMap<String, String> = HashMap::new();
    numbers
        .split_inclusive(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map(|piece| {
            let word = piece.trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
            if word.starts_with("conditionset_") {
                let next = format!("set{}", names.len() + 1);
                let name = names.entry(word.to_string()).or_insert(next).clone();
                format!("{name}{}", &piece[word.len()..])
            } else {
                piece.to_string()
            }
        })
        .collect()
}

#[test]
fn the_synthetic_cases_equal_the_reference_converter() {
    let fonts: HashMap<&str, FeatureAxes> = [("mapped", axes("mapped")), ("plain", axes("plain"))].into_iter().collect();
    let mut cases: Vec<_> = std::fs::read_dir(oracle_dir().join("cases")).unwrap().map(|e| e.unwrap().path()).collect();
    cases.sort();
    assert!(cases.len() >= 30, "{} cases", cases.len());
    let mut failures = Vec::new();
    for case in &cases {
        let code = std::fs::read_to_string(case).unwrap();
        let font = code.lines().next().unwrap().trim_start_matches("# font:").trim();
        let expected = std::fs::read_to_string(oracle_dir().join("expected").join(case.file_name().unwrap())).unwrap();
        match variable(&code, &fonts[font]) {
            Ok(got) if normalise(&got) == normalise(&expected) => {}
            Ok(got) => failures.push(format!("{}:\n--- ours\n{}--- reference\n{}", case.display(), normalise(&got), normalise(&expected))),
            Err(e) => failures.push(format!("{}: refused: {e:?}", case.display())),
        }
    }
    assert!(failures.is_empty(), "{} of {} cases differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

#[test]
fn a_condition_inside_a_lookup_is_refused() {
    let code = "feature rvrn {\nlookup one {\ncondition 150 < wght;\nsub a by b;\n} one;\n} rvrn;";
    let e = refused(code, &axes("mapped"));
    assert!(e.message.contains("inside a lookup block"), "{}", e.message);
    assert_eq!(&code[e.offset..e.offset + e.len], "condition 150 < wght;");
}

#[test]
fn a_condition_outside_a_feature_block_is_refused() {
    let e = refused("condition 150 < wght;\nsub a by b;\nfeature liga { sub f i by fi; } liga;", &axes("mapped"));
    assert!(e.message.contains("inside a feature block"), "{}", e.message);
    assert_eq!(e.offset, 0);
}

#[test]
fn an_unknown_axis_is_refused() {
    let e = refused("feature rvrn {\ncondition 150 < slnt;\nsub a by b;\n} rvrn;", &axes("mapped"));
    assert!(e.message.contains("unknown axis `slnt`") && e.message.contains("`wght`"), "{}", e.message);
    let e = refused("feature kern {\npos a b 10 (ital:1) 20;\n} kern;", &axes("plain"));
    assert!(e.message.contains("unknown axis `ital`"), "{}", e.message);
    let e =
        lower_variable_syntax("feature kern { pos a b 10 (wght:700) 20; } kern;", &FeatureAxes::default(), &LowerTarget::Static(vec![])).unwrap_err();
    assert!(e[0].message.contains("the font has no axes"), "{}", e[0].message);
}

#[test]
fn a_repeated_axis_is_refused() {
    let e = refused("feature rvrn {\ncondition 50 < wght, wght < 200;\nsub a by b;\n} rvrn;", &axes("mapped"));
    assert!(e.message.contains("gives axis `wght` twice"), "{}", e.message);
    let e = refused("feature kern {\npos a b 10 (wght:220 wght:20) 20;\n} kern;", &axes("mapped"));
    assert!(e.message.contains("twice"), "{}", e.message);
}

#[test]
fn a_malformed_range_or_value_is_refused() {
    let m = axes("mapped");
    for range in ["wght", "150 < wght <", "150 wght", "< wght < 200", "150 < wght < 200 < 300"] {
        let e = refused(&format!("feature rvrn {{\ncondition {range};\nsub a by b;\n}} rvrn;"), &m);
        assert!(e.message.contains("invalid condition axis range"), "{range}: {}", e.message);
    }
    let e = refused("feature kern {\npos a <10 0 (wght:220) 20 0 0 0>;\n} kern;", &m);
    assert!(e.message.contains("invalid variable value record"), "{}", e.message);
    let e = refused("feature kern {\npos a 10 (wght:220) 99999999999999999999;\n} kern;", &m);
    assert!(e.message.contains("out of range"), "{}", e.message);
}

#[test]
fn an_inverted_condition_is_dropped_with_a_warning() {
    let lowered =
        lower_variable_syntax("feature rvrn {\ncondition 200 < wght < 100;\nsub a by b;\n} rvrn;", &axes("mapped"), &LowerTarget::Variable).unwrap();
    assert!(!lowered.text.contains("variation"), "{}", lowered.text);
    assert_eq!(lowered.warnings.len(), 1);
}

#[test]
fn static_targets_get_the_values_and_rules_of_their_location() {
    // featureCode.mm.staticVersusVariable, featureCode.conditionLowerInclusive
    let plain = axes("plain");
    let code = "feature kern {\npos a a 8 (wght:700) 14;\npos a <10 0 20 0 (wght:900) 30 0 40 0>;\n} kern;";
    assert!(at(code, &plain, &[400.0]).contains("pos a a 8;\npos a <10 0 20 0>;"), "{}", at(code, &plain, &[400.0]));
    assert!(at(code, &plain, &[700.0]).contains("pos a a 14;\npos a <22 0 32 0>;"));
    assert!(at(code, &plain, &[550.0]).contains("pos a a 11;"));
    // Below the default nothing is stated, so the default value holds.
    assert!(at(code, &plain, &[100.0]).contains("pos a a 8;"));

    let conditional = "feature rvrn {\nsub x by x.always;\ncondition 600 < wght < 800;\nsub a by a.bold;\ncondition;\nsub y by y.always;\n} rvrn;";
    let kept = |location: f64| at(conditional, &plain, &[location]).contains("a.bold");
    assert!(kept(600.0), "the lower limit is inclusive");
    assert!(kept(700.0));
    assert!(!kept(800.0), "the upper limit is strict");
    assert!(!kept(400.0));
    let text = at(conditional, &plain, &[400.0]);
    assert!(text.contains("x.always") && text.contains("y.always") && !text.contains("condition"), "{text}");
}

#[test]
fn condition_sets_are_shared_across_texts_of_one_assembly() {
    let mut syntax = VariableSyntax::new(axes("mapped"), LowerTarget::Variable);
    let one = syntax.lower("feature rvrn {\ncondition 150 < wght;\nsub a by b;\n} rvrn;").unwrap().text;
    let two = syntax.lower("feature calt {\ncondition 150 < wght;\nsub c by d;\n} calt;").unwrap().text;
    let three = syntax.lower("feature salt {\ncondition wdth < 80;\nsub e by f;\n} salt;").unwrap().text;
    assert!(one.contains("conditionset conditionset_1 {"));
    assert!(!two.contains("conditionset conditionset") && two.contains("variation calt conditionset_1 {"), "{two}");
    assert!(three.contains("conditionset conditionset_2 {"), "{three}");
}

#[test]
fn the_map_points_into_the_input() {
    let code = "feature kern {\npos a b 10 (wght:220) 20;\n} kern;\nfeature rvrn {\ncondition 150 < wght;\nsub dollar by dollar.bold;\n} rvrn;";
    let lowered = lower_variable_syntax(code, &axes("mapped"), &LowerTarget::Variable).unwrap();
    let out = lowered.text.find("dollar.bold").unwrap();
    let back = lowered.map.source_offset(out);
    assert_eq!(&code[back..back + 11], "dollar.bold");
    let out = lowered.text.find("pos a b").unwrap();
    assert_eq!(&code[lowered.map.source_offset(out)..][..7], "pos a b");
}

#[test]
fn text_without_the_syntax_is_unchanged() {
    let code = "feature liga {\nsub f i by fi; # (wght:700) 1\nsub a.condition by b;\n} liga;";
    let lowered = lower_variable_syntax(code, &axes("mapped"), &LowerTarget::Variable).unwrap();
    assert_eq!(lowered.text, code);
    assert!(lowered.map.is_identity());
}

#[test]
fn truncated_and_odd_text_never_panics() {
    let m = axes("mapped");
    let mut samples: Vec<String> = Vec::new();
    for case in std::fs::read_dir(oracle_dir().join("cases")).unwrap() {
        samples.push(std::fs::read_to_string(case.unwrap().path()).unwrap());
    }
    samples.push("feature rvrn { condition 1 < wght; { { { } rvrn; condition".into());
    samples.push(format!("feature rvrn {{ {} condition 1 < wght; }} rvrn;", "{".repeat(200)));
    samples.push("pos a 1 (wght:1 (wdth:2) 3 <anchor (wght:1) 1 <<>>> \"(wght:1) 2".into());
    samples.push("feature aaaa{condition;condition , ;condition 1<wght<2,3<wdth;}aaaa;".into());
    samples.push("pos a -99999999999999999999999 (wght:-0.0000000001) 1e5 (opsz:99999999999) 9;".into());
    samples.push("é feature ŵght { condition 1 < wght; sub é by ë; } ŵght; (wght:12) 1 (wght:3) 2".into());
    for text in &samples {
        for end in (0..=text.len()).filter(|e| text.is_char_boundary(*e)) {
            let piece = &text[..end];
            let _ = lower_variable_syntax(piece, &m, &LowerTarget::Variable);
            let _ = lower_variable_syntax(piece, &m, &LowerTarget::Static(vec![150.0, 70.0, 40.0]));
            let _ = lower_variable_syntax(piece, &FeatureAxes::default(), &LowerTarget::Static(vec![]));
        }
    }
}

#[test]
fn many_overlapping_conditions_stay_within_the_limits() {
    let mut code = String::from("feature rvrn {\n");
    for i in 0..MAX_CONDITIONS {
        code.push_str(&format!("condition {} < wght < {}, {} < wdth;\nsub g{i} by g{i}.alt;\n", 20 + i, 220 - i, 60 + i % 30));
    }
    code.push_str("} rvrn;");
    // Either lowered or refused with a message; never unbounded.
    match lower_variable_syntax(&code, &axes("mapped"), &LowerTarget::Variable) {
        Ok(l) => assert!(l.text.len() <= MAX_CODE_BYTES),
        Err(e) => assert!(!e[0].message.is_empty()),
    }
    let mut over = String::from("feature rvrn {\n");
    for i in 0..=MAX_CONDITIONS {
        over.push_str(&format!("condition {} < wght;\nsub a by b{i};\n", 20 + i));
    }
    over.push_str("} rvrn;");
    assert!(refused(&over, &axes("mapped")).message.contains("more than"));
}

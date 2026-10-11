#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_model::plist::{Array, Value};
use fontcraft_model::{Features, Param, Params, Snippet};

use super::*;

fn snippet(name: &str, code: &str) -> Snippet {
    Snippet { name: name.into(), code: code.into(), ..Snippet::default() }
}

fn text(s: &str) -> Value {
    Value::String(s.into())
}

fn list(names: &[&str]) -> Value {
    Value::Array(Array { items: names.iter().map(|n| text(n)).collect(), ..Array::default() })
}

fn params(list: Vec<(&str, Value)>) -> Params {
    Params(list.into_iter().map(|(n, v)| Param::new(n, v)).collect())
}

fn entries() -> Features {
    let mut f = Features::default();
    f.prefixes.push(snippet("Languagesystems", "languagesystem DFLT dflt;"));
    f.prefixes.push(snippet("Old Prefix", "# old"));
    f.classes.push(snippet("Uppercase", "A B"));
    f.classes.push(snippet("Removed", "x"));
    f.features.push(Snippet { automatic: true, ..snippet("liga", "sub f i by f_i;") });
    f.features.push(snippet("calt", "sub a' b by a.alt;"));
    f.features.push(snippet("smcp", "sub a by a.sc;"));
    f
}

#[test]
fn add_replace_and_remove_follow_the_parameters_in_order() {
    let p = params(vec![
        (ADD_CLASS, text("Extra ;e f")),
        (ADD_PREFIX, text("Defs; lookup L { sub a by b; } L;")),
        (ADD_FEATURE, text("ss01;sub a by a.ss01;")),
        (REPLACE_CLASS, text("@Uppercase; A B C")),
        (REPLACE_PREFIX, text("Languagesystems; languagesystem latn dflt;")),
        (REPLACE_FEATURE, text("liga; sub f l by f_l;")),
        (REPLACE_FEATURE, text("zzzz; sub a by b;")),
        (REMOVE_CLASSES, list(&["@Removed"])),
        (REMOVE_PREFIXES, list(&["Old Prefix"])),
        (REMOVE_FEATURES, text("calt, smcp")),
    ]);
    let out = apply_instance_params(&entries(), &p).unwrap();
    let names = |l: &[Snippet]| l.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&out.classes), ["Uppercase", "Extra"]);
    assert_eq!(names(&out.prefixes), ["Languagesystems", "Defs"]);
    // The added feature goes to the end; a Replace of a feature the font lacks does nothing.
    assert_eq!(names(&out.features), ["liga", "ss01"]);
    assert_eq!(out.classes[0].code, "A B C");
    assert_eq!(out.classes[1].code, "e f");
    assert_eq!(out.prefixes[0].code, "languagesystem latn dflt;");
    assert_eq!(out.prefixes[1].code, "lookup L { sub a by b; } L;");
    assert_eq!(out.features[0].code, "sub f l by f_l;");
    assert!(!out.features[0].automatic, "replaced code is the parameter's, not generated");
    assert_eq!(out.features[1].code, "sub a by a.ss01;");

    // A later Remove takes out what an earlier Add put in; a disabled parameter does nothing.
    let mut p = params(vec![(ADD_FEATURE, text("ss02; sub b by b.ss02;")), (REMOVE_FEATURES, list(&["ss02"]))]);
    p.0.push(Param { disabled: true, ..Param::new(REMOVE_FEATURES, list(&["liga"])) });
    let out = apply_instance_params(&entries(), &p).unwrap();
    assert_eq!(names(&out.features), ["liga", "calt", "smcp"]);
}

#[test]
fn a_malformed_parameter_is_an_error_naming_it() {
    for (name, value) in [
        (ADD_CLASS, text("no semicolon")),
        (ADD_FEATURE, text(" ; sub a by b;")),
        (REPLACE_PREFIX, list(&["a; b"])),
        (REMOVE_FEATURES, Value::Bool(true)),
        (OPTICAL_SIZE, text("100; 1; 69")),
        (OPTICAL_SIZE, text("100; 1; 69; 70000; Ten")),
        (OPTICAL_SIZE, text("10.5; 1; 69; 120; Ten")),
    ] {
        let p = params(vec![(ADD_FEATURE, text("ss01; sub a by b;")), (name, value)]);
        let err = apply_instance_params(&entries(), &p).unwrap_err().to_string();
        assert!(err.contains(name), "{name}: {err}");
    }
}

#[test]
fn optical_size_writes_the_size_feature() {
    let p = params(vec![(OPTICAL_SIZE, text("100; 1; 69; 120; Ten"))]);
    let out = apply_instance_params(&entries(), &p).unwrap();
    let size = out.features.last().unwrap();
    assert_eq!(size.name, "size");
    assert_eq!(size.code, "parameters 100 1 69 120;\nsizemenuname 3 1 0x409 \"Ten\";\nsizemenuname 1 0 0 \"Ten\";");

    // Non-ASCII names are escaped for Windows and have no Mac record; subfamily 0 has no range
    // and no name; an existing size feature is replaced.
    let p = params(vec![(OPTICAL_SIZE, text("240; 3; 139; 240; Grö\"ße"))]);
    let out = apply_instance_params(&entries(), &p).unwrap();
    assert_eq!(out.features.last().unwrap().code, "parameters 240 3 139 240;\nsizemenuname 3 1 0x409 \"Gr\\00f6\\0022\\00dfe\";");
    let p = params(vec![(OPTICAL_SIZE, text("100; 1; 69; 120; Ten")), (OPTICAL_SIZE, text("120; 0; 0; 0; Twelve"))]);
    let out = apply_instance_params(&entries(), &p).unwrap();
    let sizes: Vec<_> = out.features.iter().filter(|f| f.name == "size").collect();
    assert_eq!(sizes.len(), 1);
    assert_eq!(sizes[0].code, "parameters 120 0 0 0;");
}

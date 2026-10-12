//! Diagnostic: which keys occur on objects at a given path (`.axes[]`, `.instances[]`…), and how
//! often, in the files listed in the file named by `FONTCRAFT_FILES`. `FONTCRAFT_PATHS` is a
//! comma-separated list of paths. `cargo test -p fontcraft-plist --test plist corpus_keys:: -- --ignored
//! --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use fontcraft_plist::Value;

fn walk(v: &Value, path: &str, want: &[String], out: &mut BTreeMap<String, BTreeMap<String, usize>>) {
    match v {
        Value::Dict(d) => {
            if want.iter().any(|w| w == path) {
                let row = out.entry(path.to_string()).or_default();
                *row.entry("(objects)".into()).or_default() += 1;
                for (k, _) in &d.entries {
                    *row.entry(k.as_str().to_string()).or_default() += 1;
                }
            }
            for (k, v) in &d.entries {
                walk(v, &format!("{path}.{}", k.as_str()), want, out);
            }
        }
        Value::Array(a) => a.items.iter().for_each(|v| walk(v, &format!("{path}[]"), want, out)),
        _ => {}
    }
}

#[test]
#[ignore = "diagnostic; needs FONTCRAFT_FILES and FONTCRAFT_PATHS"]
fn keys_at_paths() {
    let (Ok(list), Ok(paths)) = (std::env::var("FONTCRAFT_FILES"), std::env::var("FONTCRAFT_PATHS")) else { return };
    let want: Vec<String> = paths.split(',').map(str::to_string).collect();
    let mut out = BTreeMap::new();
    for path in std::fs::read_to_string(list).unwrap().lines().filter(|l| !l.is_empty()) {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let Ok(doc) = fontcraft_plist::openstep::parse(&text) else { continue };
        walk(&doc.root, "", &want, &mut out);
    }
    for (path, keys) in &out {
        println!("{path}: {}", keys.iter().map(|(k, n)| format!("{k}={n}")).collect::<Vec<_>>().join(" "));
    }
}

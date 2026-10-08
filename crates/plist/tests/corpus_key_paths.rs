//! Diagnostic: for each object path (`fontMaster[]`, `glyphs[].layers[]`…) in the files listed in
//! the file named by `FONTCRAFT_FILES`, whether keys are sorted, and the unsorted orders seen.
//! `cargo test -p fontcraft-plist --test corpus_key_paths -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use fontcraft_plist::Value;

fn walk(v: &Value, path: &str, out: &mut BTreeMap<String, (usize, usize, BTreeMap<String, usize>)>) {
    match v {
        Value::Dict(d) => {
            let keys: Vec<&str> = d.entries.iter().map(|(k, _)| k.as_str()).collect();
            let row = out.entry(path.to_string()).or_default();
            row.0 += 1;
            if keys.windows(2).all(|w| matches!(w, [a, b] if a.as_bytes() < b.as_bytes())) {
                row.1 += 1;
            } else {
                *row.2.entry(keys.join(" ")).or_default() += 1;
            }
            for (k, v) in &d.entries {
                walk(v, &format!("{path}.{}", k.as_str()), out);
            }
        }
        Value::Array(a) => a.items.iter().for_each(|v| walk(v, &format!("{path}[]"), out)),
        _ => {}
    }
}

#[test]
#[ignore = "diagnostic; needs FONTCRAFT_FILES"]
fn key_paths() {
    let Ok(list) = std::env::var("FONTCRAFT_FILES") else { return };
    let list = std::fs::read_to_string(list).unwrap();
    let mut out = BTreeMap::new();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let Ok(doc) = fontcraft_plist::openstep::parse(&text) else { continue };
        walk(&doc.root, "", &mut out);
    }
    for (path, (n, sorted, orders)) in &out {
        if *n == *sorted || path.contains(".userData") || path.contains("kerning") {
            continue;
        }
        println!("{path}: {sorted}/{n} sorted");
        for (o, c) in orders.iter().take(6) {
            println!("    {c:>5}  {o}");
        }
    }
}

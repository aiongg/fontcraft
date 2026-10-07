//! Diagnostic: are dictionary keys in reference-written `.glyphs` files sorted, and by which
//! order? `FONTCRAFT_FILES` names a file that lists the files, one per line. `cargo test -p fontcraft-plist --test
//! corpus_key_order -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_plist::Value;

#[test]
#[ignore = "diagnostic; needs FONTCRAFT_FILES"]
fn key_order() {
    let Ok(list_path) = std::env::var("FONTCRAFT_FILES") else { return };
    let list = std::fs::read_to_string(list_path).unwrap();
    let (mut dicts, mut bytewise, mut caseless) = (0usize, 0usize, 0usize);
    let mut examples = Vec::new();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let Ok(doc) = fontcraft_plist::openstep::parse(&text) else { continue };
        doc.root.visit(&mut |v| {
            if let Value::Dict(d) = v {
                if d.len() < 2 {
                    return;
                }
                dicts += 1;
                let keys: Vec<&str> = d.entries.iter().map(|(k, _)| k.as_str()).collect();
                let by_bytes = keys.windows(2).all(|w| w[0] < w[1]);
                let by_caseless = keys.windows(2).all(|w| w[0].to_lowercase() <= w[1].to_lowercase());
                bytewise += usize::from(by_bytes);
                caseless += usize::from(by_caseless);
                if !by_bytes && examples.len() < 12 {
                    examples.push(format!("{}: {:?}", path.rsplit('/').next().unwrap_or(""), keys.iter().take(8).collect::<Vec<_>>()));
                }
            }
        });
    }
    println!("dicts with 2+ keys: {dicts}; sorted by bytes: {bytewise}; sorted ignoring case: {caseless}");
    for e in examples {
        println!("  {e}");
    }
}

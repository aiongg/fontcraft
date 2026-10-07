//! Lists the source spellings of the corpus files named in `FONTCRAFT_SPELLINGS` (one path per
//! line), grouped by spelling. A diagnostic, not a gate: `cargo test -p fontcraft-plist --test
//! corpus_spellings -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

#[test]
#[ignore = "diagnostic; needs FONTCRAFT_SPELLINGS"]
fn list_spellings() {
    let Ok(list) = std::env::var("FONTCRAFT_SPELLINGS") else { return };
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let Ok(doc) = fontcraft_plist::openstep::parse(&text) else { continue };
        for s in doc.root.spellings(usize::MAX) {
            let shown: String = s.chars().take(40).collect();
            *seen.entry(shown).or_default() += 1;
        }
    }
    let mut v: Vec<_> = seen.into_iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    for (s, n) in v.iter().take(40) {
        println!("{n:>5}  {s:?}");
    }
}

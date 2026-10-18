//! The CID check (M13.50): `check.cid.definedTwiceOrMissing` on a CID-keyed source
//! (`fontcraft_model::cid`).

use std::collections::{BTreeMap, HashSet};

use fontcraft_model::cid::CidFont;

use crate::{Hit, Location, Params, Sink, SourceCtx};

/// Most glyph or sub-font names one message lists.
const LISTED: usize = 5;

fn names_text<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let all: Vec<&str> = names.collect();
    let shown = all.iter().take(LISTED).copied().collect::<Vec<_>>().join(", ");
    match all.len().saturating_sub(LISTED) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

/// `check.cid.definedTwiceOrMissing`: a CID with more than one glyph, a glyph in more than one
/// sub-font or in none, and the CIDs from 0 to the highest that have no glyph.
pub(crate) fn defined_twice_or_missing(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let cid = match CidFont::read(font) {
        Ok(Some(c)) => c,
        Ok(None) => return sink.skip("the font is not CID-keyed"),
        Err(e) => return sink.skip(format!("the CID-keyed data cannot be read: {e}")),
    };
    // CID → the glyphs of the font mapped to it, in map order; names with no glyph count as none.
    let mut by_cid: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for (name, c) in &cid.cids {
        if font.glyph(name).is_some() && seen.insert(name.as_str()) {
            by_cid.entry(*c).or_default().push(name.as_str());
        }
    }
    for (c, names) in by_cid.iter().filter(|(_, n)| n.len() > 1) {
        let message = format!("CID {c} has {} glyphs: {}.", names.len(), names_text(names.iter().copied()));
        let place = Location { glyph: names.first().map(|n| n.to_string()).unwrap_or_default(), ..Location::default() };
        sink.push(Hit::new("defined-twice", message).data("cid", f64::from(*c)).place(place));
    }
    for g in font.glyphs.iter().filter(|g| seen.contains(g.name.as_str())) {
        let holders: Vec<&str> = cid.subfonts.iter().filter(|s| s.glyphs.iter().any(|n| *n == g.name)).map(|s| s.name.as_str()).collect();
        let place = Location { glyph: g.name.clone(), ..Location::default() };
        match holders.len() {
            0 => {
                let c = cid.cid_of(&g.name).unwrap_or_default();
                let message = format!("Glyph {} has CID {c} but is in no sub-font.", g.name);
                sink.push(Hit::new("no-subfont", message).data("cid", f64::from(c)).place(place));
            }
            1 => {}
            n => {
                let message = format!("Glyph {} is in {n} sub-fonts: {}.", g.name, names_text(holders.into_iter()));
                sink.push(Hit::new("defined-twice", message).place(place));
            }
        }
    }
    // Gaps between the CIDs that have glyphs, from CID 0 on.
    let mut next = 0u32;
    for c in by_cid.keys().copied() {
        if c > next {
            let last = c.saturating_sub(1);
            let message = if last == next { format!("CID {next} has no glyph.") } else { format!("CIDs {next} to {last} have no glyph.") };
            sink.push(Hit::new("missing", message).data("first", f64::from(next)).data("last", f64::from(last)));
        }
        next = c.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use fontcraft_model::Font;
    use fontcraft_model::cid::{CidFont, Ros, SubFont};

    use crate::{AuditContext, Config, LayerReport, NoServices, SourceCtx, audit_source, find};

    const ID: &str = "check.cid.definedTwiceOrMissing";

    fn run(font: &Font) -> LayerReport {
        let audit = AuditContext::new(font);
        audit_source(&SourceCtx { audit: &audit, services: &NoServices }, &[find(ID).unwrap()], &Config::default())
    }

    fn cid_font(cids: &[(&str, u32)], subfonts: &[(&str, &[&str])]) -> Font {
        let mut font = Font::new("Cid");
        let cid = CidFont {
            ros: Ros::parse("Adobe-Identity-0").unwrap(),
            font_name: None,
            cids: cids.iter().map(|(n, c)| (n.to_string(), *c)).collect(),
            subfonts: subfonts
                .iter()
                .map(|(name, glyphs)| SubFont { glyphs: glyphs.iter().map(|g| g.to_string()).collect(), ..SubFont::new(*name) })
                .collect(),
        };
        cid.store(&mut font);
        font
    }

    #[test]
    fn check_cid_defined_twice_or_missing_reports_shared_cids_shared_glyphs_and_gaps() {
        let clean = cid_font(&[(".notdef", 0), ("A", 1), ("B", 2)], &[("Main", &[".notdef", "A", "B"])]);
        let r = run(&clean);
        assert!(r.findings.is_empty(), "{:?}", r.findings);

        let font = cid_font(
            // `gone` is no glyph of the font: its CID 9 counts as missing.
            &[(".notdef", 0), ("A", 1), ("B", 1), ("C", 3), ("D", 4), ("E", 8), ("gone", 9), ("F", 10)],
            &[("Latin", &[".notdef", "A", "B", "E", "F"]), ("Han", &["A", "C"])],
        );
        let r = run(&font);
        let got: Vec<(&str, &str)> = r.findings.iter().map(|f| (f.code, f.message.as_str())).collect();
        assert_eq!(
            got,
            [
                ("defined-twice", "CID 1 has 2 glyphs: A, B."),
                ("defined-twice", "Glyph A is in 2 sub-fonts: Latin, Han."),
                ("no-subfont", "Glyph D has CID 4 but is in no sub-font."),
                ("missing", "CID 2 has no glyph."),
                ("missing", "CIDs 5 to 7 have no glyph."),
                ("missing", "CID 9 has no glyph."),
            ]
        );
        assert!(r.findings.iter().all(|f| f.severity.as_str() == "info"));
        assert_eq!(r.findings[0].location.glyph, "A");
    }

    #[test]
    fn check_cid_defined_twice_or_missing_skips_ordinary_fonts() {
        let r = run(&Font::new("Plain"));
        assert!(r.findings.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].reason, "the font is not CID-keyed");
    }
}

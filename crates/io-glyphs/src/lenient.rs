//! Reading of files that break the format's rules in ways real files do (M1.10). What is read in
//! a plainer form is reported, and nothing is lost on saving:
//!
//! | File | Reads as | Saves as |
//! |---|---|---|
//! | a metric of 9223372036854775808, the number the reference app stores for "no value" | a metric the master does not state (0 where other masters state it) | the same number, while the master's value is 0 |
//! | no masters | one master, with the id the glyphs' layers use | that master |
//! | layers of one glyph with the same id | all but the first get an id of their own, and stay layers of their master | with their new ids |

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{Family, Font, Master, MetricValue};
use fontcraft_plist::{Num, Value};

/// 2^63: the largest 64-bit integer plus one, as the format writes a metric without a value.
const UNSET: f64 = 9_223_372_036_854_775_808.0;
/// The native entry of a master's extra data that holds its `metricValues` as written, when some
/// of them have no value.
pub(crate) const METRIC_VALUES: &str = "com.fontcraft.unsetMetricValues";
/// The id of a master made for a file whose layers name none.
const MASTER_ID: &str = "m01";
/// Most warnings about one file's glyphs.
const MAX_WARNINGS: usize = 100;

/// Whether a number is the format's "no value".
pub(crate) fn is_unset(v: &Value) -> bool {
    v.as_num().is_some_and(|n| Num::as_f64(n) == UNSET)
}

/// Whether an entry of `metricValues` has a number without a value.
pub(crate) fn has_unset(entry: &Value) -> bool {
    entry.as_dict().is_some_and(|d| d.entries.iter().any(|(_, v)| is_unset(v)))
}

/// Makes a font read from a file that breaks the format's rules one the model takes. The warnings
/// say what was changed.
pub(crate) fn repair(font: &mut Font) -> Vec<String> {
    let mut warnings = Vec::new();
    add_master(font, &mut warnings);
    report_unset(font, &mut warnings);
    renumber_layers(font, &mut warnings);
    warnings
}

fn add_master(font: &mut Font, warnings: &mut Vec<String>) {
    if !font.masters.is_empty() {
        return;
    }
    let layers = || font.glyphs.iter().flat_map(|g| g.layers.iter());
    let id = layers().find(|l| l.is_master_layer()).or_else(|| layers().next()).map_or_else(|| MASTER_ID.to_string(), |l| l.master.clone());
    // An axis value a format 2 file leaves out is 100.
    let origin = if font.source.glyphs_version == Some(2) { 100.0 } else { 0.0 };
    warnings.push(format!("the file has no master: one was added, with the id `{id}`"));
    let master = Master {
        id,
        name: "Regular".into(),
        location: vec![origin; font.axes.len()],
        metric_values: vec![MetricValue::default(); font.metrics.len()],
        ..Default::default()
    };
    Arc::make_mut(&mut font.masters).push(master);
}

fn report_unset(font: &Font, warnings: &mut Vec<String>) {
    for m in font.masters.iter() {
        let mut what: Vec<String> =
            crate::v2::metric_keys().filter(|k| m.extra.get(Family::Glyphs, k).is_some_and(is_unset)).map(str::to_string).collect();
        if let Some(kept) = m.extra.get(Family::Native, METRIC_VALUES).and_then(Value::as_array) {
            let kinds = font.metrics.iter().map(|d| d.kind.as_str()).chain(std::iter::repeat("a further metric"));
            what.extend(kept.items.iter().zip(kinds).filter(|(v, _)| has_unset(v)).map(|(_, kind)| kind.to_string()));
        }
        if !what.is_empty() && warnings.len() < MAX_WARNINGS {
            let name = if m.name.is_empty() { &m.id } else { &m.name };
            warnings.push(format!("the master `{name}` has no value for {} (the file states the number that stands for none)", what.join(", ")));
        }
    }
}

fn renumber_layers(font: &mut Font, warnings: &mut Vec<String>) {
    for i in 0..font.glyphs.len() {
        let Some(g) = font.glyphs.as_arcs().get(i) else { continue };
        let mut ids: HashSet<&str> = HashSet::with_capacity(g.layers.len());
        if g.layers.iter().all(|l| ids.insert(l.id.as_str())) {
            continue;
        }
        let mut taken: HashSet<String> = g.layers.iter().map(|l| l.id.clone()).collect();
        let Some(mut g) = font.glyphs.get_mut_at(i) else { continue };
        let name = g.name.clone();
        let mut seen: HashSet<String> = HashSet::new();
        let mut next: HashMap<String, usize> = HashMap::new();
        for l in &mut g.layers {
            if seen.insert(l.id.clone()) {
                continue;
            }
            let number = next.entry(l.id.clone()).or_insert(2);
            let mut id = format!("{} #{number}", l.id);
            while taken.contains(&id) {
                *number += 1;
                id = format!("{} #{number}", l.id);
            }
            if warnings.len() < MAX_WARNINGS {
                warnings.push(format!("the glyph `{name}` has several layers with the id `{}`: one of them has the id `{id}` now", l.id));
            }
            taken.insert(id.clone());
            seen.insert(id.clone());
            l.id = id;
        }
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::metric;

    use super::*;
    use crate::{read_reporting, write};

    fn read(text: &str) -> (Font, Vec<String>) {
        read_reporting(text).unwrap_or_else(|e| panic!("{e}"))
    }

    fn v2(masters: &str, glyphs: &str) -> String {
        format!(
            "{{\nfamilyName = Lenient;\nfontMaster = ({masters});\nglyphs = ({glyphs});\nunitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}}\n"
        )
    }

    const UNSET_MASTER: &str = "{\nascender = 800;\ncapHeight = 9223372036854775808;\ndescender = 0;\nid = m01;\nxHeight = 9223372036854775808;\n}";

    #[test]
    fn a_format_2_metric_without_a_value_reads_as_unset_and_is_saved_so() {
        let (font, warnings) = read(&v2(UNSET_MASTER, ""));
        assert!(warnings.len() == 1 && warnings[0].contains("no value for capHeight, xHeight"), "{warnings:?}");
        let m = &font.masters[0];
        assert!(font.metric(m, metric::CAP_HEIGHT).is_none() && font.metric(m, metric::ASCENDER).is_some());
        let out = write(&font).unwrap();
        assert!(out.contains("capHeight = 9223372036854775808;") && out.contains("xHeight = 9223372036854775808;"), "{out}");

        // Another master states the cap height: this one's is 0 until it is set.
        let two = format!("{UNSET_MASTER},\n{{\nascender = 800;\ncapHeight = 700;\ndescender = 0;\nid = m02;\n}}");
        let (mut font, _) = read(&v2(&two, ""));
        assert_eq!(font.metric(&font.masters[0], metric::CAP_HEIGHT).map(|v| v.pos), Some(0.0));
        assert!(write(&font).unwrap().contains("capHeight = 9223372036854775808;"));
        let at = font.metrics.iter().position(|d| d.kind == metric::CAP_HEIGHT).unwrap();
        Arc::make_mut(&mut font.masters)[0].metric_values[at].pos = 650.0;
        let out = write(&font).unwrap();
        assert!(out.contains("capHeight = 650;") && !out.contains("capHeight = 9223372036854775808;"), "{out}");
    }

    #[test]
    fn a_format_3_metric_without_a_value_reads_as_unset_and_is_saved_so() {
        let text = "{\n.formatVersion = 3;\nfamilyName = Lenient;\nfontMaster = (\n{\nid = m01;\nmetricValues = (\n{\npos = 800;\n},\n{\npos = 9223372036854775808;\n}\n);\nname = Regular;\n}\n);\nglyphs = (\n);\nmetrics = (\n{\ntype = ascender;\n},\n{\ntype = \"x-height\";\n}\n);\nunitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}\n";
        let (mut font, warnings) = read(text);
        assert!(warnings.len() == 1 && warnings[0].contains("`Regular` has no value for x-height"), "{warnings:?}");
        assert_eq!(font.masters[0].metric_values[1], MetricValue::default());
        assert!(write(&font).unwrap().contains("pos = 9223372036854775808;"));
        Arc::make_mut(&mut font.masters)[0].metric_values[1].pos = 500.0;
        let out = write(&font).unwrap();
        assert!(out.contains("pos = 500;") && out.contains("pos = 800;") && !out.contains("9223372036854775808"), "{out}");
    }

    #[test]
    fn a_file_without_masters_gets_one_that_takes_the_layers() {
        let glyph = "\n{\nglyphname = a;\nlayers = (\n{\nlayerId = m7;\nwidth = 611;\n}\n);\nunicode = 0061;\n}\n";
        let (font, warnings) = read(&v2("", glyph));
        assert!(warnings.len() == 1 && warnings[0].contains("no master: one was added, with the id `m7`"), "{warnings:?}");
        assert_eq!(font.glyph("a").unwrap().layer("m7").unwrap().width, 611.0);
        let (again, warnings) = read(&write(&font).unwrap());
        assert!(warnings.is_empty() && again.masters.len() == 1, "{warnings:?}");
        assert_eq!(again.glyph("a").unwrap().layer("m7").unwrap().width, 611.0);
        // No layers to take an id from.
        assert_eq!(read(&v2("", "")).0.masters[0].id, MASTER_ID);
    }

    #[test]
    fn two_layers_with_one_id_are_both_kept() {
        let layer = |width: u32| format!("{{\nlayerId = m01;\nwidth = {width};\n}}");
        let glyph = format!("\n{{\nglyphname = a;\nlayers = (\n{},\n{},\n{}\n);\nunicode = 0061;\n}}\n", layer(500), layer(510), layer(520));
        let (font, warnings) = read(&v2("{\nascender = 800;\ndescender = -200;\nid = m01;\n}", &glyph));
        assert!(warnings.len() == 2 && warnings[0].contains("the glyph `a` has several layers with the id `m01`"), "{warnings:?}");
        let (again, warnings) = read(&write(&font).unwrap());
        assert!(warnings.is_empty(), "{warnings:?}");
        let layers: Vec<(&str, &str, f64)> = again.glyph("a").unwrap().layers.iter().map(|l| (l.id.as_str(), l.master.as_str(), l.width)).collect();
        assert_eq!(layers, [("m01", "m01", 500.0), ("m01 #2", "m01", 510.0), ("m01 #3", "m01", 520.0)]);
    }
}

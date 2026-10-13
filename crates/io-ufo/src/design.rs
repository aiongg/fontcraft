//! The `fontinfo.plist` keys that hold a master's design values: blue zones, stem snaps and
//! guidelines.
//!
//! The rules are ported from glyphsLib 6.15.0 (`builder/blue_values.py`, `builder/masters.py`,
//! `builder/guidelines.py`; Apache-2.0, see `NOTICE`), so a UFO written from a `.glyphs` document
//! holds what glyphsLib would have written (`docs/spec/31-glyphslib-crosswalk.md` § 4.1, § 9.6).
//!
//! | UFO | [`Design`] | Model (placed by [`crate::ufo`]) |
//! |---|---|---|
//! | `postscriptBlueValues`, `postscriptOtherBlues` | alignment zones, by glyphsLib's rule | a zone at a metric's position is that metric's overshoot; the rest are the master's other zones |
//! | `postscriptStemSnapH`, `postscriptStemSnapV` | horizontal and vertical stem widths | unnamed stem definitions of the font, horizontal first, and the master's stem values |
//! | `guidelines` | guides | the master's guides |
//!
//! Reading is checked by writing, as for the rest of the font information ([`crate::fontinfo`]):
//! a key that would not be written back the same is kept as written. The two blue keys are
//! typed or kept together, since one list of zones makes both.

use fontcraft_model::{AlignmentZone, Guide, blue_values};
use fontcraft_plist::{Array, Dict, Value};

use crate::fontinfo::{number, string};

pub(crate) const BLUES: &str = "postscriptBlueValues";
pub(crate) const OTHER_BLUES: &str = "postscriptOtherBlues";
pub(crate) const STEMS_H: &str = "postscriptStemSnapH";
pub(crate) const STEMS_V: &str = "postscriptStemSnapV";
pub(crate) const GUIDELINES: &str = "guidelines";
/// Every key this module types.
pub(crate) const KEYS: [&str; 5] = [BLUES, OTHER_BLUES, STEMS_H, STEMS_V, GUIDELINES];

/// The suffix glyphsLib adds to a locked guide's name.
const LOCKED: &str = " [locked]";

/// A master's design values as one `fontinfo.plist` holds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Design {
    /// Every alignment zone, overshoots included, in descending order of position.
    pub zones: Vec<AlignmentZone>,
    pub stems_h: Vec<f64>,
    pub stems_v: Vec<f64>,
    pub guides: Vec<Guide>,
}

fn finite(v: &Value) -> Option<f64> {
    v.as_num().map(|n| n.as_f64()).filter(|f| f.is_finite())
}

/// A list of finite numbers.
fn numbers(v: &Value) -> Option<Vec<f64>> {
    v.as_array()?.items.iter().map(finite).collect()
}

fn number_list(v: &[f64]) -> Value {
    Value::Array(Array { items: v.iter().map(|f| number(*f)).collect(), ..Default::default() })
}

/// Reads the keys of [`KEYS`] that are not in `raw`. A key that cannot be read is left out, so
/// the check by writing finds it.
pub(crate) fn read(info: &Dict, raw: &[String]) -> Design {
    let is_raw = |k: &str| raw.iter().any(|r| r == k);
    let mut d = Design::default();
    if !is_raw(BLUES) && !is_raw(OTHER_BLUES) {
        let list = |k: &str| info.get(k).map_or(Some(Vec::new()), numbers);
        if let (Some(blues), Some(other)) = (list(BLUES), list(OTHER_BLUES)) {
            d.zones = zones_of(&blues, &other).unwrap_or_default();
        }
    }
    if !is_raw(STEMS_H) {
        d.stems_h = info.get(STEMS_H).and_then(numbers).unwrap_or_default();
    }
    if !is_raw(STEMS_V) {
        d.stems_v = info.get(STEMS_V).and_then(numbers).unwrap_or_default();
    }
    if !is_raw(GUIDELINES) {
        d.guides = info.get(GUIDELINES).and_then(Value::as_array).and_then(|a| a.items.iter().map(guide_of).collect()).unwrap_or_default();
    }
    d
}

/// Writes `d` into `out`, except the keys in `skip`; what has no place goes to `lost`. `mine` is
/// the file as read, if any: an empty list it has for a key that `d` gives nothing stays.
pub(crate) fn write(d: &Design, skip: &[String], mine: Option<&Dict>, out: &mut Vec<(String, Value)>, lost: &mut Vec<String>) {
    let before = out.len();
    write_typed(d, skip, out, lost);
    for key in KEYS {
        let empty = mine.and_then(|m| m.get(key)).filter(|v| v.as_array().is_some_and(|a| a.items.is_empty()));
        if let Some(v) = empty
            && !skip.iter().any(|s| s == key)
            && !out.iter().skip(before).any(|(k, _)| k == key)
        {
            out.push((key.into(), v.without_spellings()));
        }
    }
}

fn write_typed(d: &Design, skip: &[String], out: &mut Vec<(String, Value)>, lost: &mut Vec<String>) {
    let skipped = |k: &str| skip.iter().any(|s| s == k);
    if !skipped(BLUES) && !skipped(OTHER_BLUES) {
        if d.zones.iter().all(|z| z.pos.is_finite() && z.size.is_finite()) {
            let (blues, other) = blue_values(&d.zones);
            if !blues.is_empty() {
                out.push((BLUES.into(), number_list(&blues)));
            }
            if !other.is_empty() {
                out.push((OTHER_BLUES.into(), number_list(&other)));
            }
        } else {
            lost.push("alignment zones: a value is not a finite number".into());
        }
    }
    for (key, stems) in [(STEMS_H, &d.stems_h), (STEMS_V, &d.stems_v)] {
        if skipped(key) || stems.is_empty() {
            continue;
        }
        if stems.iter().all(|s| s.is_finite()) {
            out.push((key.into(), number_list(stems)));
        } else {
            lost.push("stems: a value is not a finite number".into());
        }
    }
    if !skipped(GUIDELINES) && !d.guides.is_empty() {
        let mut items = Vec::new();
        for g in &d.guides {
            match guideline(g) {
                Some(v) => items.push(v),
                None => lost.push("a guide: a value is not a finite number".into()),
            }
            let parts = [
                ("filters", g.filter.is_some()),
                ("angle locks", g.angle_locked),
                ("measurements", g.show_measurement),
                ("label positions", g.orientation.is_some()),
                ("other data", !g.extra.is_empty()),
            ];
            for (what, set) in parts {
                let line = format!("guide {what}");
                if set && !lost.contains(&line) {
                    lost.push(line);
                }
            }
        }
        if !items.is_empty() {
            out.push((GUIDELINES.into(), Value::Array(Array { items, ..Default::default() })));
        }
    }
}

// ── blue values ────────────────────────────────────────────────────────────────────────────────

/// Alignment zones from blue values, as glyphsLib's `to_glyphs_blue_values` makes them: a
/// `BlueValues` pair `(y1, y2)` is the zone `(y1, y2 - y1)`, or `(0, y1)` when `y2` is 0; an
/// `OtherBlues` pair is `(y2, y1 - y2)`. In descending order of position. `None` for a list of
/// odd length.
pub(crate) fn zones_of(blues: &[f64], other: &[f64]) -> Option<Vec<AlignmentZone>> {
    if !blues.len().is_multiple_of(2) || !other.len().is_multiple_of(2) {
        return None;
    }
    let mut zones = Vec::with_capacity((blues.len() + other.len()) / 2);
    for &[y1, y2] in blues.as_chunks::<2>().0 {
        zones.push(if y2 == 0.0 { AlignmentZone { pos: 0.0, size: y1 - y2 } } else { AlignmentZone { pos: y1, size: y2 - y1 } });
    }
    for &[y1, y2] in other.as_chunks::<2>().0 {
        zones.push(AlignmentZone { pos: y2, size: y1 - y2 });
    }
    // A stable sort, as Python's.
    zones.sort_by(|a, b| b.pos.partial_cmp(&a.pos).unwrap_or(std::cmp::Ordering::Equal));
    Some(zones)
}

// ── guidelines ─────────────────────────────────────────────────────────────────────────────────

fn is_vertical(y: Option<f64>, angle: Option<f64>) -> bool {
    y.is_none_or(|y| y == 0.0) && angle.is_none_or(|a| a == 90.0 || a == 270.0)
}

fn is_horizontal(x: Option<f64>, angle: Option<f64>) -> bool {
    x.is_none_or(|x| x == 0.0) && angle.is_none_or(|a| a == 0.0 || a == 180.0)
}

/// A UFO guideline as a guide, as glyphsLib's `to_glyphs_guidelines` reads it: a missing
/// coordinate is 0; a missing angle is 90 for a guideline without `y` and 0 for one without `x`
/// (with both, 90 when `y` is 0, else 0); a name ending in ` [locked]` is a locked guide.
/// glyphsLib reads `y` 0 alone as a vertical line; here it is horizontal, as the UFO
/// specification says. `None` for a key it does not know or a value of the wrong type.
pub(crate) fn guide_of(v: &Value) -> Option<Guide> {
    let d = v.as_dict()?;
    let (mut x, mut y, mut angle) = (None, None, None);
    let mut g = Guide::default();
    for (k, v) in &d.entries {
        match k.as_str() {
            "x" => x = Some(finite(v)?),
            "y" => y = Some(finite(v)?),
            "angle" => angle = Some(finite(v)?),
            "name" => g.name = Some(v.as_str()?.to_string()),
            "color" => g.color = Some(v.as_str()?.to_string()),
            "identifier" => g.identifier = Some(v.as_str()?.to_string()),
            _ => return None,
        }
    }
    if let Some(stem) = g.name.as_deref().and_then(|n| n.strip_suffix(LOCKED)) {
        // A locked guide without a name is written as the suffix alone.
        g.name = (!stem.is_empty()).then(|| stem.to_string());
        g.locked = true;
    }
    g.x = x.unwrap_or(0.0);
    g.y = y.unwrap_or(0.0);
    g.angle = match angle {
        Some(a) => a.rem_euclid(360.0),
        None if y.is_some() && x.is_none() => 0.0,
        None if is_vertical(y, None) => 90.0,
        None => 0.0,
    };
    Some(g)
}

/// Splits ` [#<identifier>]` off the end of a name, as glyphsLib's `IDENTIFIER_NAME_RE` does: the
/// identifier is at least one character and holds no `]`.
fn split_identifier(name: &str) -> Option<(&str, &str)> {
    if name.contains('\n') {
        return None;
    }
    let body = name.strip_suffix(']')?;
    // The last start that leaves a non-empty identifier without `]`, as a greedy match finds it.
    body.rmatch_indices(" [#").find_map(|(i, _)| {
        let id = body.get(i + 3..)?;
        (!id.is_empty() && !id.contains(']')).then(|| Some((body.get(..i)?, id))).flatten()
    })
}

/// Splits ` [<colour>]` off the end of a name, as glyphsLib's `COLOR_NAME_RE` does: the colour is
/// at least one of the characters `0`, `1`, `.`, `,`, `e`, `E`, `+` and `-`.
fn split_color(name: &str) -> Option<(&str, &str)> {
    if name.contains('\n') {
        return None;
    }
    let body = name.strip_suffix(']')?;
    let i = body.rfind(" [")?;
    let color = body.get(i + 2..)?;
    let allowed = |c: char| matches!(c, '0' | '1' | '.' | ',' | 'e' | 'E' | '+' | '-');
    (!color.is_empty() && color.chars().all(allowed)).then(|| Some((body.get(..i)?, color))).flatten()
}

/// A guide as a UFO guideline, as glyphsLib's `to_ufo_guidelines` writes it: `x` alone for a
/// vertical guide through `y` 0, `y` alone for a horizontal one through `x` 0, else all three; a
/// colour or identifier the guide does not have is taken from the end of its name; a locked guide
/// gets ` [locked]` after its name; an empty name is left out. Keys in sorted order, as fontTools
/// writes them. `None` when a value is not finite.
pub(crate) fn guideline(g: &Guide) -> Option<Value> {
    if !(g.x.is_finite() && g.y.is_finite() && g.angle.is_finite()) {
        return None;
    }
    // Between 0 and 360, without the noise the subtraction adds (-62.0205 is 297.9795).
    let angle = if (0.0..360.0).contains(&g.angle) { g.angle } else { (g.angle.rem_euclid(360.0) * 1e9).round() / 1e9 };
    let (x, y, a) = if is_vertical(Some(g.y), Some(angle)) {
        (Some(g.x), None, None)
    } else if is_horizontal(Some(g.x), Some(angle)) {
        (None, Some(g.y), None)
    } else {
        (Some(g.x), Some(g.y), Some(angle))
    };
    let mut name = g.name.clone();
    let mut identifier = g.identifier.clone();
    let mut color = g.color.clone();
    if identifier.is_none()
        && let Some((rest, id)) = name.as_deref().and_then(split_identifier)
    {
        identifier = Some(id.to_string());
        name = Some(rest.to_string());
    }
    if color.is_none()
        && let Some((rest, c)) = name.as_deref().and_then(split_color)
    {
        color = Some(c.to_string());
        name = Some(rest.to_string());
    }
    if g.locked {
        name = Some(format!("{}{LOCKED}", name.unwrap_or_default()));
    }
    let mut d = Dict::new();
    if let Some(a) = a {
        d.insert("angle", number(a));
    }
    if let Some(c) = color {
        d.insert("color", string(&c));
    }
    if let Some(i) = identifier {
        d.insert("identifier", string(&i));
    }
    if let Some(n) = name.filter(|n| !n.is_empty()) {
        d.insert("name", string(&n));
    }
    if let Some(x) = x {
        d.insert("x", number(x));
    }
    if let Some(y) = y {
        d.insert("y", number(y));
    }
    Some(Value::Dict(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_plist::xmlplist;

    fn dict(xml: &str) -> Dict {
        let doc = xmlplist::parse(&format!("<plist version=\"1.0\"><dict>{xml}</dict></plist>")).unwrap();
        let Value::Dict(d) = doc.root else { panic!() };
        d
    }

    fn z(pos: f64, size: f64) -> AlignmentZone {
        AlignmentZone { pos, size }
    }

    #[test]
    fn blue_values_become_zones_and_come_back() {
        // The example glyphsLib's rule is checked with in docs/spec/31-glyphslib-crosswalk.md § 4.1.
        let zones = zones_of(&[-10.0, 0.0, 500.0, 510.0], &[-210.0, -200.0]).unwrap();
        assert_eq!(zones, [z(500.0, 10.0), z(0.0, -10.0), z(-200.0, -10.0)]);
        assert_eq!(blue_values(&zones), (vec![-10.0, 0.0, 500.0, 510.0], vec![-210.0, -200.0]));
        // A zone of positive size below the baseline is a blue value; a zone at 0 is one whatever
        // its size.
        assert_eq!(blue_values(&[z(-200.0, 5.0), z(0.0, 12.0)]), (vec![-200.0, -195.0, 0.0, 12.0], vec![]));
        assert_eq!(zones_of(&[1.0], &[]), None, "an odd number of values");
    }

    #[test]
    fn a_key_is_read_only_when_it_writes_back_the_same() {
        let info = dict(
            "<key>postscriptBlueValues</key><array><integer>-12</integer><integer>0</integer><real>500.5</real><integer>510</integer></array>\
             <key>postscriptStemSnapH</key><array><integer>40</integer><integer>42</integer></array>\
             <key>postscriptStemSnapV</key><array><real>80.0</real></array>",
        );
        let d = read(&info, &[]);
        assert_eq!(d.zones, [z(500.5, 9.5), z(0.0, -12.0)]);
        assert_eq!((d.stems_h.as_slice(), d.stems_v.as_slice()), ([40.0, 42.0].as_slice(), [80.0].as_slice()));
        let (mut out, mut lost) = (Vec::new(), Vec::new());
        write(&d, &[], None, &mut out, &mut lost);
        assert!(lost.is_empty());
        for (k, v) in &out {
            assert_eq!(info.get(k), Some(v), "{k}");
        }
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn guidelines_read_and_write_as_glyphslib_does() {
        let g = |xml: &str| guide_of(&Value::Dict(dict(xml)));
        let v = g("<key>x</key><integer>250</integer>").unwrap();
        assert_eq!((v.x, v.y, v.angle), (250.0, 0.0, 90.0));
        let h = g("<key>name</key><string>top [locked]</string><key>y</key><integer>700</integer>").unwrap();
        assert_eq!((h.x, h.y, h.angle, h.name.as_deref(), h.locked), (0.0, 700.0, 0.0, Some("top"), true));
        let a = g("<key>angle</key><integer>-45</integer><key>color</key><string>1,0,0,1</string><key>identifier</key><string>g1</string>\
                   <key>x</key><integer>10</integer><key>y</key><integer>20</integer>")
        .unwrap();
        assert_eq!((a.angle, a.color.as_deref(), a.identifier.as_deref()), (315.0, Some("1,0,0,1"), Some("g1")));
        assert!(g("<key>x</key><integer>1</integer><key>lib</key><dict/>").is_none(), "an unknown key");
        let back = |g: &Guide| guideline(g).unwrap();
        assert_eq!(back(&v), Value::Dict(dict("<key>x</key><integer>250</integer>")));
        assert_eq!(back(&h), Value::Dict(dict("<key>name</key><string>top [locked]</string><key>y</key><integer>700</integer>")));
        // A guide from a `.glyphs` document: colour and identifier come from its name.
        let named = Guide { x: 5.0, y: 6.0, angle: 30.0, name: Some("mid [1,0,0,1] [#abc]".into()), ..Default::default() };
        assert_eq!(
            back(&named),
            Value::Dict(dict(
                "<key>angle</key><integer>30</integer><key>color</key><string>1,0,0,1</string><key>identifier</key><string>abc</string>\
                 <key>name</key><string>mid</string><key>x</key><integer>5</integer><key>y</key><integer>6</integer>"
            ))
        );
        // Only the characters glyphsLib's pattern allows make a colour.
        assert_eq!(split_color("a [0.5]"), None);
        assert_eq!(split_color("a [0.1,1]"), Some(("a", "0.1,1")));
        assert_eq!(split_identifier("a [#b [#]"), Some(("a", "b [#")));
        assert_eq!(split_identifier("a [#]"), None);
    }

    #[test]
    fn guideline_forms_that_would_change_stay_as_written() {
        // `y` 0 alone is a horizontal line through the origin, and is written so.
        let info = dict("<key>guidelines</key><array><dict><key>y</key><integer>0</integer></dict></array>");
        let d = read(&info, &[]);
        assert_eq!((d.guides[0].x, d.guides[0].y, d.guides[0].angle), (0.0, 0.0, 0.0));
        let (mut out, mut lost) = (Vec::new(), Vec::new());
        write(&d, &[], None, &mut out, &mut lost);
        assert_eq!(out.first().map(|(_, v)| v), info.get(GUIDELINES));
        // A vertical line with all three values writes as `x` alone.
        let info = dict(
            "<key>guidelines</key><array><dict><key>angle</key><integer>90</integer><key>x</key><integer>5</integer>\
             <key>y</key><integer>0</integer></dict></array>",
        );
        let d = read(&info, &[]);
        let (mut out, mut lost) = (Vec::new(), Vec::new());
        write(&d, &[], None, &mut out, &mut lost);
        assert_ne!(out.first().map(|(_, v)| v), info.get(GUIDELINES));
        let d = read(&info, &[GUIDELINES.to_string()]);
        assert!(d.guides.is_empty());
    }
}

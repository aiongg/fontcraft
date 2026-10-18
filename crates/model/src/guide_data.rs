//! What FontCraft keeps on a guide besides its position (M13.36): thickness, markers,
//! magnetism and power-guide links (`docs/plans/M13.md` § 4 › M13.36).
//!
//! The values live under the `com.fontcraft.guide*` keys of [`crate::fc_keys`], inside the
//! guide's `.glyphs` user data: the `userData` dictionary a `.glyphs` guide may carry, kept in
//! the guide's [`Extra`] for the `.glyphs` family. The `.glyphs` writer puts it back, and the
//! native file keeps every family's entries, so the values survive both. A UFO guideline has no
//! place for them.
//!
//! A stored value that is not of the documented shape reads as absent.

use fontcraft_plist::{Array, Dict, Number, Value};

use crate::fc_keys::{GUIDE_MAGNETIC, GUIDE_MARKERS, GUIDE_THICKNESS, POWER_GUIDE_LINKS};
use crate::{Extra, Family, Guide, MAX_COORDINATE};

/// The key of the user-data dictionary in a `.glyphs` guide.
const USER_DATA: &str = "userData";
/// Widest band a guide's thickness may give, in font units, either way.
pub const MAX_GUIDE_THICKNESS: f64 = 10_000.0;
/// Most markers one guide holds.
pub const MAX_GUIDE_MARKERS: usize = 256;
/// Farthest a marker may sit from the guide's knob, in font units.
pub const MAX_MARKER_DISTANCE: f64 = 2.0 * MAX_COORDINATE;
/// Strongest magnetism: the snapping distance in screen points.
pub const MAX_MAGNETISM: f64 = 100.0;
/// Most node links one power guide holds.
pub const MAX_POWER_LINKS: usize = 100_000;
/// Longest glyph name a link may hold.
const MAX_LINK_NAME: usize = 1024;

/// A node a power guide moves. A local guide's links name nodes of its own layer side
/// (`glyph` is `None`); a global guide's name the glyph, and the node is in that glyph's
/// layer for the guide's master (foreground).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Link {
    pub glyph: Option<String>,
    pub shape: usize,
    pub node: usize,
}

impl Link {
    pub fn local(shape: usize, node: usize) -> Self {
        Self { glyph: None, shape, node }
    }

    pub fn global(glyph: impl Into<String>, shape: usize, node: usize) -> Self {
        Self { glyph: Some(glyph.into()), shape, node }
    }
}

fn user_data(extra: &Extra) -> Option<&Dict> {
    extra.get(Family::Glyphs, USER_DATA)?.as_dict()
}

fn get<'g>(g: &'g Guide, key: &str) -> Option<&'g Value> {
    user_data(&g.extra)?.get(key)
}

/// Stores `value` under `key`, or removes the key with `None`; an emptied dictionary is removed.
fn put(g: &mut Guide, key: &str, value: Option<Value>) {
    let mut dict = user_data(&g.extra).cloned().unwrap_or_default();
    match value {
        Some(v) => dict.insert(key, v),
        None => {
            if let Some(i) = dict.entries.iter().position(|(k, _)| k.as_str() == key) {
                dict.remove_at(i);
            }
        }
    }
    if dict.is_empty() {
        g.extra.remove(Family::Glyphs, USER_DATA);
    } else {
        g.extra.insert(Family::Glyphs, USER_DATA, Value::Dict(dict));
    }
}

fn num(v: f64) -> Result<Value, String> {
    Number::float(v).map(Value::Number).ok_or_else(|| format!("{v} is not a finite number"))
}

fn index_value(i: usize) -> Result<Value, String> {
    i64::try_from(i).map(|i| Value::Number(Number::int(i))).map_err(|_| format!("the index {i} is too large"))
}

fn array(items: Vec<Value>) -> Value {
    Value::Array(Array { items, ..Array::default() })
}

fn as_index(v: &Value) -> Option<usize> {
    let n = v.as_num()?.as_f64();
    (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64).then(|| n as usize)
}

/// Whether `t` can be a guide's thickness: finite, not zero, at most [`MAX_GUIDE_THICKNESS`]
/// either way.
pub fn valid_thickness(t: f64) -> bool {
    t.is_finite() && t != 0.0 && t.abs() <= MAX_GUIDE_THICKNESS
}

/// The distance from the guide to its second edge, in font units: positive on the left of the
/// guide's direction, negative on the right.
pub fn thickness(g: &Guide) -> Option<f64> {
    get(g, GUIDE_THICKNESS)?.as_num().map(|n| n.as_f64()).filter(|t| valid_thickness(*t))
}

/// Gives the guide a thickness, or removes it with `None` or 0.
pub fn set_thickness(g: &mut Guide, t: Option<f64>) -> Result<(), String> {
    match t.filter(|t| *t != 0.0) {
        Some(t) if !valid_thickness(t) => {
            Err(format!("a guide's thickness must be a finite number from -{MAX_GUIDE_THICKNESS} to {MAX_GUIDE_THICKNESS}, got {t}"))
        }
        Some(t) => {
            put(g, GUIDE_THICKNESS, Some(num(t)?));
            Ok(())
        }
        None => {
            put(g, GUIDE_THICKNESS, None);
            Ok(())
        }
    }
}

/// The guide's markers: distances along the guide from its knob, in the guide's direction, in
/// font units, sorted. Unusable entries are left out.
pub fn markers(g: &Guide) -> Vec<f64> {
    let Some(list) = get(g, GUIDE_MARKERS).and_then(Value::as_array) else { return Vec::new() };
    let mut out: Vec<f64> = list
        .items
        .iter()
        .take(MAX_GUIDE_MARKERS)
        .filter_map(|v| v.as_num().map(|n| n.as_f64()))
        .filter(|d| d.is_finite() && d.abs() <= MAX_MARKER_DISTANCE)
        .collect();
    out.sort_by(f64::total_cmp);
    out.dedup();
    out
}

/// Replaces the guide's markers; an empty list removes them. They are sorted and repeats dropped.
pub fn set_markers(g: &mut Guide, positions: &[f64]) -> Result<(), String> {
    if positions.len() > MAX_GUIDE_MARKERS {
        return Err(format!("a guide holds at most {MAX_GUIDE_MARKERS} markers, got {}", positions.len()));
    }
    if let Some(bad) = positions.iter().find(|d| !(d.is_finite() && d.abs() <= MAX_MARKER_DISTANCE)) {
        return Err(format!("a marker must be a finite distance of at most {MAX_MARKER_DISTANCE} units from the guide's knob, got {bad}"));
    }
    let mut sorted = positions.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup();
    if sorted.is_empty() {
        put(g, GUIDE_MARKERS, None);
    } else {
        put(g, GUIDE_MARKERS, Some(array(sorted.into_iter().map(num).collect::<Result<_, _>>()?)));
    }
    Ok(())
}

/// How strongly the guide pulls dragged points: the distance, in screen points, within which a
/// dragged selection snaps to it. `None` when the guide is not magnetic.
pub fn magnetism(g: &Guide) -> Option<f64> {
    let v = get(g, GUIDE_MAGNETIC)?;
    let m = match v {
        // Older values may say only whether the guide is magnetic.
        Value::Bool(true) => DEFAULT_MAGNETISM,
        _ => v.as_num()?.as_f64(),
    };
    (m.is_finite() && m > 0.0 && m <= MAX_MAGNETISM).then_some(m)
}

/// The magnetism a guide marked magnetic without a distance has, in screen points.
pub const DEFAULT_MAGNETISM: f64 = 5.0;

/// Sets the guide's magnetism, or removes it with `None` or 0.
pub fn set_magnetism(g: &mut Guide, m: Option<f64>) -> Result<(), String> {
    match m.filter(|m| *m != 0.0) {
        Some(m) if !(m.is_finite() && m > 0.0 && m <= MAX_MAGNETISM) => {
            Err(format!("a guide's magnetism must be a number from 0 to {MAX_MAGNETISM} screen points, got {m}"))
        }
        Some(m) => {
            put(g, GUIDE_MAGNETIC, Some(num(m)?));
            Ok(())
        }
        None => {
            put(g, GUIDE_MAGNETIC, None);
            Ok(())
        }
    }
}

/// The nodes the guide moves, in stored order, repeats and unusable entries left out.
pub fn links(g: &Guide) -> Vec<Link> {
    let Some(list) = get(g, POWER_GUIDE_LINKS).and_then(Value::as_array) else { return Vec::new() };
    let mut out: Vec<Link> = Vec::new();
    for item in list.items.iter().take(MAX_POWER_LINKS) {
        let Some(parts) = item.as_array().map(|a| a.items.as_slice()) else { continue };
        let link = match parts {
            [s, n] => as_index(s).zip(as_index(n)).map(|(s, n)| Link::local(s, n)),
            [glyph, s, n] => glyph
                .as_str()
                .filter(|name| !name.is_empty() && name.len() <= MAX_LINK_NAME)
                .zip(as_index(s).zip(as_index(n)))
                .map(|(name, (s, n))| Link::global(name, s, n)),
            _ => None,
        };
        if let Some(link) = link
            && !out.contains(&link)
        {
            out.push(link);
        }
    }
    out
}

/// Whether the guide is a power guide: it has at least one link.
pub fn is_power_guide(g: &Guide) -> bool {
    !links(g).is_empty()
}

/// Replaces the guide's links; an empty list removes them, leaving an ordinary guide.
pub fn set_links(g: &mut Guide, links: &[Link]) -> Result<(), String> {
    if links.len() > MAX_POWER_LINKS {
        return Err(format!("a power guide holds at most {MAX_POWER_LINKS} links, got {}", links.len()));
    }
    if links.is_empty() {
        put(g, POWER_GUIDE_LINKS, None);
        return Ok(());
    }
    let mut items = Vec::with_capacity(links.len());
    for l in links {
        let mut parts = Vec::with_capacity(3);
        if let Some(name) = &l.glyph {
            if name.is_empty() || name.len() > MAX_LINK_NAME {
                return Err(format!("a link's glyph name must have 1 to {MAX_LINK_NAME} bytes"));
            }
            parts.push(Value::String(name.as_str().into()));
        }
        parts.push(index_value(l.shape)?);
        parts.push(index_value(l.node)?);
        items.push(array(parts));
    }
    put(g, POWER_GUIDE_LINKS, Some(array(items)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_live_in_the_guide_user_data_and_clear_it_when_removed() {
        let mut g = Guide::default();
        set_thickness(&mut g, Some(-12.5)).unwrap();
        set_markers(&mut g, &[40.0, 10.0, 40.0]).unwrap();
        set_magnetism(&mut g, Some(8.0)).unwrap();
        set_links(&mut g, &[Link::local(0, 2), Link::global("H", 1, 3)]).unwrap();
        let data = g.extra.get(Family::Glyphs, "userData").and_then(Value::as_dict).unwrap();
        assert_eq!(data.len(), 4);
        assert_eq!(thickness(&g), Some(-12.5));
        assert_eq!(markers(&g), vec![10.0, 40.0]);
        assert_eq!(magnetism(&g), Some(8.0));
        assert_eq!(links(&g), vec![Link::local(0, 2), Link::global("H", 1, 3)]);
        assert!(set_thickness(&mut g, Some(f64::NAN)).is_err());
        assert!(set_magnetism(&mut g, Some(1000.0)).is_err());
        set_thickness(&mut g, None).unwrap();
        set_markers(&mut g, &[]).unwrap();
        set_magnetism(&mut g, Some(0.0)).unwrap();
        set_links(&mut g, &[]).unwrap();
        assert!(g.extra.is_empty(), "{:?}", g.extra);
    }
}

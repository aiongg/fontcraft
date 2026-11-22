//! The metric lines Edit View shows for one glyph: the master's metrics, with a per-script
//! `ascender`, `capHeight` or `descender` parameter in place of the master's line of that kind
//! for glyphs of that script, `smallCapHeight` in place of the x-height for a small cap, and a
//! `shoulderHeight` line. A replaced line keeps the master's overshoot, so its zone moves with
//! it.
//!
//! The engine's `guide.displayLines`, the canvas and the tools' snapping all read the lines here,
//! so what is drawn is what is snapped to.

use crate::plist::Value;
use crate::{Font, Glyph, MAX_COORDINATE, Master, metric};

/// The kind of the line a `shoulderHeight` parameter adds.
pub const SHOULDER_HEIGHT: &str = "shoulder height";

/// One metric line as it is shown for a glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownMetric {
    /// The metric's index in [`Font::metrics`]; `None` for a line only a parameter gives.
    pub index: Option<usize>,
    pub kind: String,
    pub name: Option<String>,
    pub filter: Option<String>,
    pub y: f64,
    /// The master's overshoot for the metric (zero for a parameter's own line).
    pub over: f64,
    /// A parameter gives the position.
    pub from_param: bool,
}

impl ShownMetric {
    /// A metric with neither a name nor a glyph scope.
    pub fn is_plain(&self) -> bool {
        self.name.is_none() && self.filter.is_none()
    }
}

/// The script a glyph's per-script parameters are looked up by: its own, in lower case.
pub fn glyph_script(glyph: &Glyph) -> Option<String> {
    glyph.info.script.as_deref().map(str::to_lowercase)
}

/// A parameter value with an optional script prefix (`greek: 705`, `han:6`): the script in lower
/// case, and the rest as text.
fn scripted(v: &Value) -> Option<(Option<String>, String)> {
    match v {
        Value::Number(n) => Some((None, n.value().as_f64().to_string())),
        Value::String(s) => Some(match s.as_str().split_once(':') {
            Some((script, rest)) => (Some(script.trim().to_lowercase()), rest.trim().to_string()),
            None => (None, s.as_str().trim().to_string()),
        }),
        _ => None,
    }
}

/// The value of parameter `name` for a glyph's script, from the master's parameters and then the
/// font's: an entry for that script first, else (when `plain` allows it) one without a script.
pub fn script_param(font: &Font, master: &Master, name: &str, script: Option<&str>, plain: bool) -> Option<String> {
    let all: Vec<(Option<String>, String)> = master.params.all(name).chain(font.info.params.all(name)).filter_map(scripted).collect();
    let for_script = script.and_then(|sc| all.iter().find(|(s, _)| s.as_deref() == Some(sc)));
    for_script.or_else(|| if plain { all.iter().find(|(s, _)| s.is_none()) } else { None }).map(|(_, v)| v.clone())
}

/// A parameter's text as a coordinate: finite and within the model's limits.
pub fn param_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite() && n.abs() <= MAX_COORDINATE)
}

/// The metric lines `master` shows for `glyph`, in the font's metric order (the italic angle left
/// out), then the shoulder height when a parameter gives one.
pub fn shown_metrics(font: &Font, master: &Master, glyph: &Glyph) -> Vec<ShownMetric> {
    let script = glyph_script(glyph);
    let script = script.as_deref();
    let small_caps = glyph.info.case.as_deref() == Some("smallCaps");
    let param = |name: &str, plain: bool| script_param(font, master, name, script, plain).as_deref().and_then(param_number);
    let mut out = Vec::new();
    for (i, def) in font.metrics.iter().enumerate() {
        if def.kind == metric::ITALIC_ANGLE {
            continue;
        }
        let value = master.metric_values.get(i).copied().unwrap_or_default();
        let plain = def.name.is_none() && def.filter.is_none();
        let replaced = match def.kind.as_str() {
            _ if !plain => None,
            metric::ASCENDER => param("ascender", false),
            metric::CAP_HEIGHT => param("capHeight", false),
            metric::DESCENDER => param("descender", false),
            metric::X_HEIGHT if small_caps => param("smallCapHeight", true),
            _ => None,
        };
        out.push(ShownMetric {
            index: Some(i),
            kind: def.kind.clone(),
            name: def.name.clone(),
            filter: def.filter.clone(),
            y: replaced.unwrap_or(value.pos),
            over: value.over,
            from_param: replaced.is_some(),
        });
    }
    if let Some(y) = param("shoulderHeight", true) {
        out.push(ShownMetric { index: None, kind: SHOULDER_HEIGHT.into(), name: None, filter: None, y, over: 0.0, from_param: true });
    }
    out
}

/// The position of the plain metric `kind` among `lines`, or `default` when none is shown.
pub fn shown_pos(lines: &[ShownMetric], kind: &str, default: f64) -> f64 {
    lines.iter().find(|m| m.is_plain() && m.kind == kind).map_or(default, |m| m.y)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_script_ascender_replaces_the_masters_line_and_keeps_its_zone() {
        let font = Font::new("Test");
        let master_id = font.masters.first().unwrap().id.clone();
        let mut master = font.master(&master_id).unwrap().clone();
        master.params.set("ascender", Value::String("greek: 705".into()));
        let over = font.metric(&master, metric::ASCENDER).unwrap().over;
        let mut glyph: Glyph = (**font.glyph("A").unwrap()).clone();
        let latin = shown_metrics(&font, &master, &glyph);
        glyph.info.script = Some("Greek".into());
        let greek = shown_metrics(&font, &master, &glyph);
        let asc = |lines: &[ShownMetric]| lines.iter().filter(|m| m.kind == metric::ASCENDER).cloned().collect::<Vec<_>>();
        assert_eq!(asc(&greek).len(), 1, "one ascender line, in place of the master's");
        assert_eq!((asc(&greek)[0].y, asc(&greek)[0].over, asc(&greek)[0].from_param), (705.0, over, true));
        assert!(!asc(&latin)[0].from_param);
        assert_eq!(latin.len(), greek.len());
    }
}

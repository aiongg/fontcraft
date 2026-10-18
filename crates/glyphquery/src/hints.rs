//! Which stored hints count as manual hints for Has Hints, Has PostScript Hints and Has TrueType
//! Hints (`smartFilter.rule.hasHints` and the two after it) and their predicate spellings.
//!
//! The test is the one the export applies (`fontcraft_hinting::ps` and `fontcraft_hinting::tt`,
//! which this crate sits below): a PostScript hint is a stem or ghost by its `type`, written as a
//! name or as the number older files use; a TrueType hint is an instruction of one of the five
//! types whose stored value names every point its type needs. Corner components, caps and other
//! drawing parts stored among the hints are none of these.

use fontcraft_model::{Hint, Layer};
use fontcraft_plist::{Dict, Num, Value};

/// The kind of manual hint a stored hint is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HintFamily {
    PostScript,
    TrueType,
}

fn type_is(v: &Value, names: &[&str], codes: &[i64]) -> bool {
    match v {
        Value::String(s) => names.iter().any(|n| s.as_str().eq_ignore_ascii_case(n)),
        Value::Number(n) => matches!(n.value(), Num::Int(i) if codes.contains(&i)),
        _ => false,
    }
}

/// Whether `d`, a stored hint of a TrueType instruction type, names the points that type needs.
fn attached(d: &Dict, t: &Value) -> bool {
    let has = |k: &str| d.get(k).is_some();
    if type_is(t, &["TTSnap", "TTAnchor", "Anchor", "TTDelta", "Delta"], &[3, 9]) {
        has("origin") || has("target")
    } else if type_is(t, &["TTStem", "TTShift", "TTAlign", "Align"], &[4, 5]) {
        has("origin") && has("target")
    } else if type_is(t, &["TTInterpolate", "Interpolate"], &[6]) {
        has("origin") && has("other1") && has("target")
    } else {
        false
    }
}

/// The kind of manual hint `hint` is, if any.
pub(crate) fn family(hint: &Hint) -> Option<HintFamily> {
    let d = hint.value.as_dict()?;
    let t = d.get("type")?;
    if type_is(t, &["Stem", "TopGhost", "BottomGhost"], &[0, -1, 1]) {
        Some(HintFamily::PostScript)
    } else if attached(d, t) {
        Some(HintFamily::TrueType)
    } else {
        None
    }
}

/// Whether `layer` holds a manual hint of `want` (of either kind when `None`).
pub(crate) fn layer_has(layer: &Layer, want: Option<HintFamily>) -> bool {
    layer.info.hints.iter().filter_map(family).any(|f| want.is_none_or(|w| w == f))
}

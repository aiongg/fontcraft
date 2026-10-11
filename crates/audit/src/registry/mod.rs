//! The registry: every check with its metadata. Profile, fix, thresholds, scope, group,
//! preconditions and message codes live here as data (`docs/spec/21-qa.md` § 1 and § 10,
//! question 12). Ids, severities and scopes match the `[[check]]` rows of
//! `spec/inventory/21-qa-checks.toml`.
//!
//! One file per group of checks, so the packages that add checks never edit the same file. The
//! files are concatenated at compile time in the fixed order of [`PARTS`] into [`CHECKS`].
//!
//! | File | Checks | Package |
//! |---|---|---|
//! | `outlines` | the layer-scope outline checks | M3 |
//! | `components`, `anchors` | `check.components.*`, `check.anchors.*` | M12.46 |
//! | `glyphset` | `check.glyphset.*`, `check.outlines.contourCount` | M12.47 |
//! | `metrics_font` | the font-wide `check.metrics.*` | M12.48 |
//! | `metrics_widths` | the width `check.metrics.*` | M12.49 |
//! | `names` | `check.names.*` | M12.50 |
//! | `tables` | `check.tables.*`, `check.outlines.pointsOutsideBBox` | M12.51 |
//! | `layout` | `check.layout.*` | M12.52 |
//! | `variations_source` | the source `check.variations.*` | M12.53 |
//! | `variations_binary` | the binary `check.variations.*` | M12.54 |
//! | `hinting` | `check.hinting.*` | M12.55 |
//! | `kerning`, `color` | `check.kerning.*`, `check.color.*` | M12.56 |
//! | `files`, `sources` | `check.files.*`, `check.sources.*` | M12.57 |
//! | `family` | `check.family.*` | M12.58 |

mod anchors;
mod color;
mod components;
mod family;
mod files;
mod glyphset;
mod hinting;
mod kerning;
mod layout;
mod metrics_font;
mod metrics_widths;
mod names;
mod outlines;
mod sources;
mod tables;
mod variations_binary;
mod variations_source;

use crate::{CheckGroup, CheckScope, CheckSpec, Profile, Runner, Severity};

/// The group files in registry order.
const PARTS: &[&[CheckSpec]] = &[
    outlines::CHECKS,
    components::CHECKS,
    anchors::CHECKS,
    glyphset::CHECKS,
    metrics_font::CHECKS,
    metrics_widths::CHECKS,
    names::CHECKS,
    tables::CHECKS,
    layout::CHECKS,
    variations_source::CHECKS,
    variations_binary::CHECKS,
    hinting::CHECKS,
    kerning::CHECKS,
    family::CHECKS,
    files::CHECKS,
    sources::CHECKS,
    color::CHECKS,
];

/// Every check, in registry order.
pub static CHECKS: &[CheckSpec] = &ALL;

const ALL: [CheckSpec; count(PARTS)] = concat(PARTS);

/// The fields an outline check that reads one layer shares; each entry sets the rest and takes
/// these with `..LAYER_OUTLINES`.
const LAYER_OUTLINES: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Outlines,
    scope: CheckScope::Layer,
    family: None,
    severity: Severity::Warn,
    profile: Profile::Outlines,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[],
    run: Runner::Layer(no_layer_check),
};

fn no_layer_check(_: &crate::LayerInput<'_>, _: &crate::Params<'_>, _: &mut crate::Sink) {}

const fn count(parts: &[&[CheckSpec]]) -> usize {
    let mut n = 0;
    let mut p = 0;
    while p < parts.len() {
        n += parts[p].len();
        p += 1;
    }
    n
}

// Evaluated at compile time only: an index out of range is a build error, never a panic.
#[allow(clippy::indexing_slicing)]
const fn concat<const N: usize>(parts: &[&[CheckSpec]]) -> [CheckSpec; N] {
    let mut out = [LAYER_OUTLINES; N];
    let mut i = 0;
    let mut p = 0;
    while p < parts.len() {
        let part = parts[p];
        let mut j = 0;
        while j < part.len() {
            out[i] = part[j];
            i += 1;
            j += 1;
        }
        p += 1;
    }
    out
}

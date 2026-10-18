//! Registry entry of the `check.cid.*` check (M13.50).

use crate::Profile::Universal;
use crate::Severity::Info;
use crate::checks::cid;
use crate::{CheckGroup, CheckScope, CheckSpec, Runner};

pub(super) const CHECKS: &[CheckSpec] = &[CheckSpec {
    id: "check.cid.definedTwiceOrMissing",
    label: "CIDs defined twice or missing",
    description: "In a CID-keyed font, a CID has more than one glyph, a glyph is in more than one sub-font or in none, or a CID below the highest one has no glyph. Unused CIDs are common, so the last kind is often harmless.",
    group: CheckGroup::Cid,
    scope: CheckScope::Source,
    family: None,
    severity: Info,
    profile: Universal,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &["defined-twice", "no-subfont", "missing"],
    code_severities: &[],
    preconditions: &[],
    run: Runner::Source(cid::defined_twice_or_missing),
}];

//! Registry entries of the source-scope `check.variations.*` checks (master compatibility) (M12.53).
//!
//! The structural checks read the services' compatibility report and run once per source; the
//! heuristic tests and the open-path check read one glyph with all its masters.

use crate::Precondition::Variable;
use crate::Profile::{Outlines, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::compat::{self, IGNORE_MISSING, KINKINESS, TOLERANCE};
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Runner};

const BASE: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Variations,
    scope: CheckScope::Source,
    family: None,
    severity: Fail,
    profile: Outlines,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[Variable],
    run: Runner::Source(compat::path_count),
};

const FIX_ORDER: Option<FixSpec> = Some(FixSpec {
    id: "fixCompatibility",
    command: Some("glyph.setShapeOrder"),
    safe: false,
    description: "Reorder the master's shapes to the proposed order.",
});

const FIX_START: Option<FixSpec> = Some(FixSpec {
    id: "fixCompatibility",
    command: Some("path.makeNodeFirst"),
    safe: false,
    description: "Reverse the path when the result says so, then make the proposed node its first node.",
});

/// The source-scope variation checks, in the order of the inventory.
pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.variations.glyphMissingInMaster",
        label: "Glyph missing in a master",
        description: "A glyph has layers in some masters but not in others.",
        thresholds: &[IGNORE_MISSING],
        codes: &["missing"],
        run: Runner::Source(compat::glyph_missing),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.openPathInMaster",
        label: "Open path in a master",
        description: "A path is open in a master, so the glyph's masters cannot be compared or interpolated.",
        scope: CheckScope::Glyph,
        fix: Some(FixSpec {
            id: "closePaths",
            command: Some("path.closeOpenPaths"),
            safe: false,
            description: "Close the path: join its ends when they coincide, else add a line.",
        }),
        codes: &["open-path"],
        run: Runner::Glyph(compat::open_path),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.pathCount",
        label: "Path count differs between masters",
        description: "A glyph has a different number of paths in a master than in its neighbour toward the default master.",
        codes: &["path-count"],
        run: Runner::Source(compat::path_count),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.nodeCount",
        label: "Node count differs between masters",
        description: "Matching paths have a different number of nodes in two masters.",
        codes: &["node-count"],
        run: Runner::Source(compat::node_count),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.nodeTypeMismatch",
        label: "Node type differs between masters",
        description: "Matching paths have as many nodes, but a node is a line, curve or off-curve node in one master and another kind in the other.",
        codes: &["node-type"],
        run: Runner::Source(compat::node_type),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.contourOrder",
        label: "Contour order differs between masters",
        description: "The shapes of a glyph pair up better between two masters in another order than the stored one.",
        scope: CheckScope::Glyph,
        severity: Warn,
        thresholds: &[TOLERANCE],
        fix: FIX_ORDER,
        capped: true,
        codes: &["contour-order"],
        run: Runner::Glyph(compat::contour_order_check),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.wrongStartPoint",
        label: "Start point differs between masters",
        description: "A path starts at a different node, or runs the other way, in two masters; the result names the node that matches better.",
        scope: CheckScope::Glyph,
        severity: Warn,
        thresholds: &[TOLERANCE],
        fix: FIX_START,
        capped: true,
        codes: &["wrong-start-point"],
        run: Runner::Glyph(compat::wrong_start_point),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.kink",
        label: "Kink between masters",
        description: "A node that is smooth in two masters becomes a visible corner halfway between them, because its handles have different length ratios.",
        scope: CheckScope::Glyph,
        severity: Warn,
        thresholds: &[TOLERANCE, KINKINESS],
        capped: true,
        codes: &["kink"],
        run: Runner::Glyph(compat::kink),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.underweight",
        label: "Thin interpolation",
        description: "A path halfway between two masters encloses clearly less area than the geometric mean of the two, a sign that its points are paired wrongly.",
        scope: CheckScope::Glyph,
        severity: Warn,
        thresholds: &[TOLERANCE],
        capped: true,
        codes: &["underweight"],
        run: Runner::Glyph(compat::underweight),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.overweight",
        label: "Heavy interpolation",
        description: "A path halfway between two masters encloses more area than either master.",
        scope: CheckScope::Glyph,
        severity: Info,
        profile: Strict,
        thresholds: &[TOLERANCE],
        capped: true,
        codes: &["overweight"],
        run: Runner::Glyph(compat::overweight),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.componentMismatch",
        label: "Components differ between masters",
        description: "A glyph has a different number of components, other base glyphs or another component order in two masters.",
        codes: &["component-mismatch"],
        run: Runner::Source(compat::component_mismatch),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.anchorMismatch",
        label: "Anchors differ between masters",
        description: "A glyph's anchor names differ between two masters.",
        codes: &["anchor-mismatch"],
        run: Runner::Source(compat::anchor_mismatch),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.mastersShareLocation",
        label: "Masters at the same location",
        description: "Two masters, or an intermediate layer and a master or another intermediate layer of the same glyph, sit at the same location.",
        profile: Universal,
        codes: &["masters-share-location", "layer-shares-location"],
        run: Runner::Source(compat::masters_share_location),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.instanceOutsideAxisRange",
        label: "Instance outside the masters' range",
        description: "An exported instance lies beyond the masters on an axis, so it needs extrapolation.",
        severity: Warn,
        profile: Universal,
        codes: &["outside-axis-range"],
        run: Runner::Source(compat::instance_outside_axis_range),
        ..BASE
    },
];

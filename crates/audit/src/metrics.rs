//! Checks against the font's metrics and grid: points just off a metric line or an alignment
//! zone, points off the grid, coordinates out of range.

use fontcraft_model::metric;

use crate::contour::Contour;
use crate::{Hit, LayerInput, Nth, Params, Severity, Sink, Threshold, num};

/// Largest magnitude a coordinate may have in a compiled font (16-bit, with room for deltas).
pub(crate) const MAX_STORED_COORDINATE: f64 = 32766.0;
/// A coordinate within this fraction of a grid step of a grid line is on it.
const GRID_EPSILON: f64 = 1e-6;

pub(crate) const ALIGNMENT_MAX_DISTANCE: Threshold = Threshold {
    name: "maxDistance",
    default: 2.0,
    min: 0.0,
    max: 50.0,
    doc: "On-curve points this many units or less from a metric line, and not on it, are reported.",
};

pub(crate) const ZONE_MARGIN: Threshold = Threshold {
    name: "margin",
    default: 2.0,
    min: 0.0,
    max: 50.0,
    doc: "Extreme points this many units or less outside an alignment zone are reported.",
};

pub(crate) const MAX_EM_MULTIPLE: Threshold = Threshold {
    name: "maxEmMultiple",
    default: 4.0,
    min: 1.0,
    max: 100.0,
    doc: "Coordinates farther from the origin than this many ems are reported.",
};

/// The italic angle of the layer's master, 0 when it has none.
pub(crate) fn italic_angle(input: &LayerInput<'_>) -> f64 {
    input.master.and_then(|m| input.ctx.font.metric(m, metric::ITALIC_ANGLE)).map(|v| v.pos).filter(|v| v.is_finite()).unwrap_or(0.0)
}

#[derive(Clone, Copy, PartialEq)]
enum Case {
    Upper,
    Lower,
    Other,
}

/// The case of the glyph's first code point, or of its name when it is one letter (with any
/// suffix).
fn case(glyph: &fontcraft_model::Glyph) -> Case {
    let c = glyph.unicodes.first().and_then(|u| char::from_u32(*u)).or_else(|| {
        let base = glyph.name.split('.').next().unwrap_or("");
        let mut chars = base.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    });
    match c {
        Some(c) if c.is_uppercase() => Case::Upper,
        Some(c) if c.is_lowercase() => Case::Lower,
        _ => Case::Other,
    }
}

/// An on-curve point within `maxDistance` of a metric line without being on it.
pub(crate) fn alignment_miss(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let Some(master) = input.master else {
        sink.skip("the layer's master does not exist");
        return;
    };
    let max = p.get(&ALIGNMENT_MAX_DISTANCE);
    let font = input.ctx.font;
    let case = case(input.glyph);
    let mut lines: Vec<(&'static str, f64)> = vec![("baseline", font.metric_pos(master, metric::BASELINE, 0.0))];
    for (name, kind) in [("ascender", metric::ASCENDER), ("descender", metric::DESCENDER)] {
        if let Some(v) = font.metric(master, kind) {
            lines.push((name, v.pos));
        }
    }
    if case == Case::Lower
        && let Some(v) = font.metric(master, metric::X_HEIGHT)
    {
        lines.push(("x-height", v.pos));
    }
    if case == Case::Upper
        && let Some(v) = font.metric(master, metric::CAP_HEIGHT)
    {
        lines.push(("cap height", v.pos));
    }
    lines.retain(|(_, y)| y.is_finite());
    for c in input.finite() {
        for (i, n) in c.path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()) {
            let nearest = lines.iter().map(|(name, y)| (*name, *y, (n.y - y).abs())).min_by(|a, b| a.2.total_cmp(&b.2));
            let Some((name, y, d)) = nearest else { continue };
            if d > 0.0 && d <= max {
                sink.push(
                    Hit::new(
                        "alignment-miss",
                        format!("Node {i} of path {} is {} units off the {name} ({}).", Nth(c.shape), num(d), num(y), i = Nth(i)),
                    )
                    .points(vec![[c.shape, i]])
                    .at(n.point())
                    .data("distance", d)
                    .data("metric", y),
                );
            }
        }
    }
}

/// Is on-curve node `i` a vertical extreme of its contour: both neighbouring points at or
/// below it, or both at or above it?
fn vertical_extreme(c: &Contour<'_>, i: usize) -> bool {
    let Some(n) = c.path.nodes.get(i) else { return false };
    let Some((a, b)) = c.joint(i) else { return false };
    let (Some(arrive), Some(leave)) = (a.end_dir(), b.start_dir()) else { return false };
    let (before, after) = (n.y - arrive.y, n.y + leave.y);
    (before <= n.y && after <= n.y) || (before >= n.y && after >= n.y)
}

/// An extreme on-curve point just outside an alignment zone, within `margin` of its edge.
pub(crate) fn near_alignment_zone(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let Some(master) = input.master else {
        sink.skip("the layer's master does not exist");
        return;
    };
    let margin = p.get(&ZONE_MARGIN);
    let zones: Vec<(f64, f64)> = input
        .ctx
        .font
        .alignment_zones(master)
        .into_iter()
        .filter(|z| z.pos.is_finite() && z.size.is_finite() && z.size != 0.0)
        .map(|z| (z.pos.min(z.pos + z.size), z.pos.max(z.pos + z.size)))
        .collect();
    if zones.is_empty() {
        return;
    }
    for c in input.finite() {
        for (i, n) in c.path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()) {
            if !vertical_extreme(c, i) || zones.iter().any(|(lo, hi)| (*lo..=*hi).contains(&n.y)) {
                continue;
            }
            let nearest = zones.iter().map(|(lo, hi)| if n.y < *lo { lo - n.y } else { n.y - hi }).fold(f64::INFINITY, f64::min);
            if nearest > 0.0 && nearest <= margin {
                sink.push(
                    Hit::new(
                        "near-zone",
                        format!("Node {i} of path {} is {} units outside an alignment zone.", Nth(c.shape), num(nearest), i = Nth(i)),
                    )
                    .points(vec![[c.shape, i]])
                    .at(n.point())
                    .data("distance", nearest),
                );
            }
        }
    }
}

/// Node and handle coordinates that are not multiples of the grid step. One finding per layer.
pub(crate) fn nodes_off_grid(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(step) = input.ctx.grid_step else {
        sink.skip("the font has no grid");
        return;
    };
    let off = |v: f64| {
        let q = v / step;
        (q - q.round()).abs() > GRID_EPSILON
    };
    let mut points = Vec::new();
    let mut first = None;
    for c in input.finite() {
        for (i, n) in c.path.nodes.iter().enumerate() {
            if off(n.x) || off(n.y) {
                points.push([c.shape, i]);
                if first.is_none() {
                    first = Some(n.point());
                }
            }
        }
    }
    if points.is_empty() {
        return;
    }
    let count = points.len();
    let mut hit = Hit::new("off-grid", format!("{count} points are not on the grid of {} units.", num(step)))
        .points(points)
        .data("count", count as f64)
        .data("gridStep", step);
    if let Some(p) = first {
        hit = hit.at(p);
    }
    sink.push(hit);
}

/// Coordinates that are not numbers, that a compiled font cannot store, or that lie
/// implausibly far from the em square. One finding per path and kind of problem.
pub(crate) fn coordinates_out_of_range(input: &LayerInput<'_>, p: &Params<'_>, sink: &mut Sink) {
    let em_limit = p.get(&MAX_EM_MULTIPLE) * f64::from(input.ctx.font.units_per_em);
    for c in &input.contours {
        let (mut nan, mut stored, mut far) = (Vec::new(), Vec::new(), Vec::new());
        for (i, n) in c.path.nodes.iter().enumerate() {
            let worst = n.x.abs().max(n.y.abs());
            if !(n.x.is_finite() && n.y.is_finite()) {
                nan.push([c.shape, i]);
            } else if worst > MAX_STORED_COORDINATE {
                stored.push([c.shape, i]);
            } else if worst > em_limit {
                far.push([c.shape, i]);
            }
        }
        if !nan.is_empty() {
            sink.push(
                Hit::new("not-a-number", format!("Path {} has {} coordinates that are not numbers.", Nth(c.shape), nan.len()))
                    .shapes(vec![c.shape])
                    .points(nan),
            );
        }
        if !stored.is_empty() {
            sink.push(
                Hit::new(
                    "beyond-storage",
                    format!(
                        "Path {} has {} points beyond ±{} units, which a compiled font cannot store.",
                        Nth(c.shape),
                        stored.len(),
                        num(MAX_STORED_COORDINATE)
                    ),
                )
                .shapes(vec![c.shape])
                .points(stored),
            );
        }
        if !far.is_empty() {
            sink.push(
                Hit::new("far-from-em", format!("Path {} has {} points more than {} units from the origin.", Nth(c.shape), far.len(), num(em_limit)))
                    .severity(Severity::Warn)
                    .shapes(vec![c.shape])
                    .points(far),
            );
        }
    }
}

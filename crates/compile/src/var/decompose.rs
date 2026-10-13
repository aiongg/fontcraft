//! Which composite glyphs a variable font decomposes (M9.5).
//!
//! Rows: `compile.inconsistentComponentsDecompose`, parameters `Decompose Brace Layers`,
//! `Decompose Components in Variable Font`.
//!
//! Contract: a composite is kept wherever `gvar` can vary it. It is decomposed when its
//! component transforms other than the offsets differ between sources, when Decompose
//! Components in Variable Font is on, and, with Decompose Brace Layers on, when a component's
//! base glyph has intermediate layers the composite lacks. With the parameter off or absent such
//! composites stay composites, as the oracles write them (`docs/plans/M9.md` § 7 question 4,
//! decided).

use fontcraft_interp::{Designspace, GlyphSources};
use fontcraft_model::{Font, ResolvedGlyph, Shape, param_flag};

/// The font parameter that decomposes every composite of a variable font.
pub const DECOMPOSE_COMPONENTS: &str = "Decompose Components in Variable Font";
/// The font parameter that decomposes composites of glyphs with intermediate layers.
pub const DECOMPOSE_BRACE_LAYERS: &str = "Decompose Brace Layers";
/// How far apart two 2×2 transform values may be and still count as the same.
const SAME_TRANSFORM: f64 = 1e-6;

/// Why a composite is decomposed.
#[derive(Clone, Debug, PartialEq)]
pub enum DecomposeReason {
    /// The component at this shape index scales, rotates or slants differently between sources.
    InconsistentTransform { shape: usize },
    /// Decompose Components in Variable Font is on.
    Parameter,
    /// Decompose Brace Layers is on and this base glyph has intermediate layers.
    BraceBase { base: String },
}

fn flag(font: &Font, name: &str) -> Option<bool> {
    font.info.params.0.iter().find(|p| !p.disabled && p.name == name).and_then(|p| param_flag(&p.value))
}

/// Why `sources`' glyph must be decomposed in a variable font; `None` to keep it a composite.
/// The component transforms are compared by [`inconsistent_transform`] on the drawn sources.
pub fn decompose_reason(font: &Font, designspace: &Designspace, sources: &GlyphSources) -> Option<DecomposeReason> {
    let glyph = font.glyph(&sources.glyph)?;
    let bases: Vec<&str> = glyph
        .layers
        .iter()
        .filter(|l| sources.main.iter().any(|s| s.layer_id == l.id))
        .flat_map(|l| l.shapes.iter().filter_map(|s| if let Shape::Component(c) = s { Some(c.glyph.as_str()) } else { None }))
        .collect();
    if bases.is_empty() {
        return None;
    }
    if flag(font, DECOMPOSE_COMPONENTS) == Some(true) {
        return Some(DecomposeReason::Parameter);
    }
    if flag(font, DECOMPOSE_BRACE_LAYERS) == Some(true) {
        let own: Vec<&fontcraft_interp::Location> = sources.main.iter().filter(|s| !s.is_master).map(|s| &s.location).collect();
        for base in bases {
            let Ok(b) = GlyphSources::of(font, designspace, base) else { continue };
            if b.main.iter().any(|s| !s.is_master && !own.contains(&&s.location)) {
                return Some(DecomposeReason::BraceBase { base: base.to_string() });
            }
        }
    }
    None
}

/// The first component whose base or 2×2 transform differs between `sources` (one glyph drawn
/// at each of its sources), or whose sources do not have the same components.
pub fn inconsistent_transform(sources: &[&ResolvedGlyph]) -> Option<DecomposeReason> {
    let first = sources.first()?;
    for other in sources.iter().skip(1) {
        if other.components.len() != first.components.len() || other.contours.is_empty() != first.contours.is_empty() {
            return Some(DecomposeReason::InconsistentTransform { shape: 0 });
        }
        for (shape, (a, b)) in first.components.iter().zip(&other.components).enumerate() {
            let (ca, cb) = (a.transform.as_coeffs(), b.transform.as_coeffs());
            if a.base != b.base || ca.iter().zip(&cb).take(4).any(|(x, y)| (x - y).abs() > SAME_TRANSFORM) {
                return Some(DecomposeReason::InconsistentTransform { shape });
            }
        }
    }
    None
}

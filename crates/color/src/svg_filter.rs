//! The filters of an SVG file as data for a raster drawing (M11.17d): each `filter` usvg resolved
//! on a group, its region and primitives in the group's user space, with the affine from that
//! space to font units. `fontcraft_render::color` runs them on pixels.
//!
//! Rows: `color.preview.textPreview`.
//!
//! Contract:
//! - Drawn primitives: `feGaussianBlur`, `feOffset`, `feFlood`, `feColorMatrix` (every type, each
//!   given as a 4 × 5 matrix), `feComposite` (every operator), `feMerge`, `feBlend` (every mode)
//!   and `feDropShadow`; CSS filter functions reach here as these primitives. Inputs are
//!   `SourceGraphic`, `SourceAlpha` and earlier results; usvg already sends a missing or unknown
//!   `in` to the previous result (or `SourceGraphic` for the first primitive), and a region in
//!   `objectBoundingBox` units to user space.
//! - A filter with any other primitive is [`Unsupported`], and the element is drawn unfiltered.
//! - At most [`MAX_FILTER_PRIMITIVES`] primitives for one element, its filters together, each
//!   `feMerge` input counting as one more. Filtered elements nest at most [`MAX_FILTER_DEPTH`]
//!   deep; a deeper one is drawn unfiltered.

use fontcraft_geom::{Affine, Rect};

/// Most primitives the filters of one element hold together (each `feMerge` input counts as one
/// more), so a filter's work stays within its region's pixels times this.
pub const MAX_FILTER_PRIMITIVES: usize = 32;

/// Deepest nesting of filtered elements: a filtered element inside this many filtered elements
/// is drawn unfiltered.
pub const MAX_FILTER_DEPTH: usize = 4;

/// One `filter` of an element.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgFilter {
    /// From the element's user space (y down) to font units (y up).
    pub to_font: Affine,
    /// The filter region, user space: nothing of the result lies outside it.
    pub region: Rect,
    pub primitives: Vec<FilterPrimitive>,
}

/// One primitive of a filter.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPrimitive {
    /// The primitive subregion, user space: its result is clear outside it.
    pub subregion: Rect,
    /// `color-interpolation-filters` is `linearRGB` (the default): the primitive works on linear
    /// RGB values; else on sRGB values.
    pub linear_rgb: bool,
    pub kind: FilterKind,
}

/// What a primitive reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterInput {
    SourceGraphic,
    SourceAlpha,
    /// The result of the primitive at this index of the same filter, which comes earlier.
    Result(usize),
}

/// What a primitive does. Lengths are in user space.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterKind {
    Blur {
        input: FilterInput,
        std_dev: (f64, f64),
    },
    Offset {
        input: FilterInput,
        dx: f64,
        dy: f64,
    },
    /// Straight sRGB colour, its alpha the flood opacity.
    Flood {
        color: [f64; 4],
    },
    /// Rows R, G, B, A of `[r g b a 1]` multipliers, on straight (unpremultiplied) values from 0
    /// to 1.
    ColorMatrix {
        input: FilterInput,
        matrix: [f64; 20],
    },
    Composite {
        input: FilterInput,
        input2: FilterInput,
        operator: CompositeOperator,
    },
    /// The inputs laid over each other, the first at the bottom.
    Merge {
        inputs: Vec<FilterInput>,
    },
    /// `input` blended over `input2`.
    Blend {
        input: FilterInput,
        input2: FilterInput,
        mode: BlendMode,
    },
    /// The input over its alpha blurred, moved and filled with `color` (straight sRGB, its alpha
    /// the flood opacity).
    DropShadow {
        input: FilterInput,
        dx: f64,
        dy: f64,
        std_dev: (f64, f64),
        color: [f64; 4],
    },
}

/// `feComposite` operators: `input` is the source, `input2` the destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompositeOperator {
    Over,
    In,
    Out,
    Atop,
    Xor,
    /// `k1·i1·i2 + k2·i1 + k3·i2 + k4` on premultiplied values.
    Arithmetic([f64; 4]),
}

/// `feBlend` modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

/// Why an element's filters are not drawn; the element is drawn unfiltered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported(pub String);

/// The filters of a group as data: `to_font` maps the group's user space (its own transform
/// applied) to font units. `Err` when one of them cannot be drawn.
pub fn filters(group: &usvg::Group, to_font: Affine) -> Result<Vec<SvgFilter>, Unsupported> {
    let mut out = Vec::new();
    let mut count = 0usize;
    for filter in group.filters() {
        let mut primitives = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        for p in filter.primitives() {
            count = count.saturating_add(1);
            if let usvg::filter::Kind::Merge(m) = p.kind() {
                count = count.saturating_add(m.inputs().len());
            }
            if count > MAX_FILTER_PRIMITIVES {
                return Err(Unsupported(format!("an SVG element has more than {MAX_FILTER_PRIMITIVES} filter primitives")));
            }
            let kind = kind(p.kind(), &names)?;
            primitives.push(FilterPrimitive {
                subregion: rect(p.rect()),
                linear_rgb: p.color_interpolation() == usvg::filter::ColorInterpolation::LinearRGB,
                kind,
            });
            names.push(p.result());
        }
        out.push(SvgFilter { to_font, region: rect(filter.rect()), primitives });
    }
    Ok(out)
}

fn rect(r: usvg::NonZeroRect) -> Rect {
    Rect::new(f64::from(r.left()), f64::from(r.top()), f64::from(r.right()), f64::from(r.bottom()))
}

/// The input `i` of a primitive whose earlier siblings have the result names `names`; the
/// latest primitive of a name wins, as a later `result` replaces an earlier one.
fn input(i: &usvg::filter::Input, names: &[&str]) -> Result<FilterInput, Unsupported> {
    match i {
        usvg::filter::Input::SourceGraphic => Ok(FilterInput::SourceGraphic),
        usvg::filter::Input::SourceAlpha => Ok(FilterInput::SourceAlpha),
        usvg::filter::Input::Reference(name) => names
            .iter()
            .rposition(|n| n == name)
            .map(FilterInput::Result)
            .ok_or_else(|| Unsupported(format!("an SVG filter primitive reads the result `{name}`, which no earlier primitive gives"))),
    }
}

fn finite(v: f32) -> Result<f64, Unsupported> {
    let v = f64::from(v);
    if v.is_finite() { Ok(v) } else { Err(Unsupported("an SVG filter primitive has a number that is not finite".into())) }
}

fn color(c: usvg::Color, opacity: f32) -> [f64; 4] {
    let alpha = if opacity.is_finite() { f64::from(opacity).clamp(0.0, 1.0) } else { 0.0 };
    [f64::from(c.red) / 255.0, f64::from(c.green) / 255.0, f64::from(c.blue) / 255.0, alpha]
}

fn kind(k: &usvg::filter::Kind, names: &[&str]) -> Result<FilterKind, Unsupported> {
    use usvg::filter::Kind as K;
    Ok(match k {
        K::GaussianBlur(b) => {
            FilterKind::Blur { input: input(b.input(), names)?, std_dev: (finite(b.std_dev_x().get())?, finite(b.std_dev_y().get())?) }
        }
        K::Offset(o) => FilterKind::Offset { input: input(o.input(), names)?, dx: finite(o.dx())?, dy: finite(o.dy())? },
        K::Flood(f) => FilterKind::Flood { color: color(f.color(), f.opacity().get()) },
        K::ColorMatrix(m) => FilterKind::ColorMatrix { input: input(m.input(), names)?, matrix: matrix(m.kind())? },
        K::Composite(c) => FilterKind::Composite {
            input: input(c.input1(), names)?,
            input2: input(c.input2(), names)?,
            operator: match c.operator() {
                usvg::filter::CompositeOperator::Over => CompositeOperator::Over,
                usvg::filter::CompositeOperator::In => CompositeOperator::In,
                usvg::filter::CompositeOperator::Out => CompositeOperator::Out,
                usvg::filter::CompositeOperator::Atop => CompositeOperator::Atop,
                usvg::filter::CompositeOperator::Xor => CompositeOperator::Xor,
                usvg::filter::CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
                    CompositeOperator::Arithmetic([finite(k1)?, finite(k2)?, finite(k3)?, finite(k4)?])
                }
            },
        },
        K::Merge(m) => FilterKind::Merge { inputs: m.inputs().iter().map(|i| input(i, names)).collect::<Result<_, _>>()? },
        K::Blend(b) => FilterKind::Blend { input: input(b.input1(), names)?, input2: input(b.input2(), names)?, mode: blend_mode(b.mode()) },
        K::DropShadow(d) => FilterKind::DropShadow {
            input: input(d.input(), names)?,
            dx: finite(d.dx())?,
            dy: finite(d.dy())?,
            std_dev: (finite(d.std_dev_x().get())?, finite(d.std_dev_y().get())?),
            color: color(d.color(), d.opacity().get()),
        },
        other => return Err(Unsupported(format!("the SVG filter primitive {} is not drawn", element_name(other)))),
    })
}

fn element_name(k: &usvg::filter::Kind) -> &'static str {
    use usvg::filter::Kind as K;
    match k {
        K::ComponentTransfer(_) => "feComponentTransfer",
        K::ConvolveMatrix(_) => "feConvolveMatrix",
        K::DiffuseLighting(_) => "feDiffuseLighting",
        K::DisplacementMap(_) => "feDisplacementMap",
        K::Image(_) => "feImage",
        K::Morphology(_) => "feMorphology",
        K::SpecularLighting(_) => "feSpecularLighting",
        K::Tile(_) => "feTile",
        K::Turbulence(_) => "feTurbulence",
        _ => "of this kind",
    }
}

fn blend_mode(m: usvg::BlendMode) -> BlendMode {
    use usvg::BlendMode as B;
    match m {
        B::Normal => BlendMode::Normal,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
    }
}

/// The 4 × 5 matrix of an `feColorMatrix`, by the formulas of the Filter Effects specification.
fn matrix(kind: &usvg::filter::ColorMatrixKind) -> Result<[f64; 20], Unsupported> {
    use usvg::filter::ColorMatrixKind as M;
    let rgb = |m: [f64; 9]| {
        let [a, b, c, d, e, f, g, h, i] = m;
        [a, b, c, 0.0, 0.0, d, e, f, 0.0, 0.0, g, h, i, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
    };
    Ok(match kind {
        M::Matrix(values) => {
            let mut out = [0.0; 20];
            if values.len() != out.len() {
                return Err(Unsupported("an SVG colour matrix does not have 20 values".into()));
            }
            for (o, v) in out.iter_mut().zip(values) {
                *o = finite(*v)?;
            }
            out
        }
        M::Saturate(s) => {
            let s = finite(s.get())?;
            rgb([
                0.213 + 0.787 * s,
                0.715 - 0.715 * s,
                0.072 - 0.072 * s,
                0.213 - 0.213 * s,
                0.715 + 0.285 * s,
                0.072 - 0.072 * s,
                0.213 - 0.213 * s,
                0.715 - 0.715 * s,
                0.072 + 0.928 * s,
            ])
        }
        M::HueRotate(degrees) => {
            let (sin, cos) = finite(*degrees)?.to_radians().sin_cos();
            rgb([
                0.213 + cos * 0.787 - sin * 0.213,
                0.715 - cos * 0.715 - sin * 0.715,
                0.072 - cos * 0.072 + sin * 0.928,
                0.213 - cos * 0.213 + sin * 0.143,
                0.715 + cos * 0.285 + sin * 0.140,
                0.072 - cos * 0.072 - sin * 0.283,
                0.213 - cos * 0.213 - sin * 0.787,
                0.715 - cos * 0.715 + sin * 0.715,
                0.072 + cos * 0.928 + sin * 0.072,
            ])
        }
        M::LuminanceToAlpha => {
            let mut out = [0.0; 20];
            if let Some(row) = out.get_mut(15..18) {
                row.copy_from_slice(&[0.2125, 0.7154, 0.0721]);
            }
            out
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(svg: &str) -> usvg::Tree {
        usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap()
    }

    fn first_group(t: &usvg::Tree) -> &usvg::Group {
        t.root().children().iter().find_map(|n| if let usvg::Node::Group(g) = n { Some(g.as_ref()) } else { None }).unwrap()
    }

    #[test]
    fn filters_become_primitives_with_named_inputs_and_unsupported_ones_are_refused() {
        let t = tree(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <filter id="f" x="0" y="0" width="100" height="100" filterUnits="userSpaceOnUse">
              <feGaussianBlur in="SourceAlpha" stdDeviation="3" result="blur"/>
              <feOffset dx="4" dy="5" result="moved"/>
              <feColorMatrix type="saturate" values="0"/>
              <feMerge><feMergeNode in="moved"/><feMergeNode in="SourceGraphic"/></feMerge>
            </filter>
            <rect width="50" height="50" fill="#ff0000" filter="url(#f)"/></svg>"##,
        );
        let f = filters(first_group(&t), Affine::IDENTITY).unwrap();
        let [filter] = f.as_slice() else { panic!("{f:?}") };
        assert_eq!(filter.region, Rect::new(0.0, 0.0, 100.0, 100.0));
        let kinds: Vec<&FilterKind> = filter.primitives.iter().map(|p| &p.kind).collect();
        assert_eq!(*kinds[0], FilterKind::Blur { input: FilterInput::SourceAlpha, std_dev: (3.0, 3.0) });
        assert_eq!(*kinds[1], FilterKind::Offset { input: FilterInput::Result(0), dx: 4.0, dy: 5.0 });
        assert_eq!(*kinds[3], FilterKind::Merge { inputs: vec![FilterInput::Result(1), FilterInput::SourceGraphic] });
        assert!(filter.primitives[0].linear_rgb, "linearRGB is the default");

        let t = tree(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <filter id="f"><feTurbulence baseFrequency="0.05"/></filter>
            <rect width="50" height="50" filter="url(#f)"/></svg>"##,
        );
        assert!(filters(first_group(&t), Affine::IDENTITY).is_err());
    }
}

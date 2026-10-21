//! The filters of an SVG file as data for a raster drawing (M11.17d, M11.17e): each `filter`
//! usvg resolved on a group, its region and primitives in the group's user space, with the affine
//! from that space to font units. `fontcraft_render::color` runs them on pixels.
//!
//! Rows: `color.preview.textPreview`, `color.preview.editViewOtherGlyphs`.
//!
//! Contract:
//! - Drawn primitives: `feGaussianBlur`, `feOffset`, `feFlood`, `feColorMatrix` (every type, each
//!   given as a 4 × 5 matrix), `feComposite` (every operator), `feMerge`, `feBlend` (every mode),
//!   `feDropShadow`, `feComponentTransfer` (every function type), `feConvolveMatrix`,
//!   `feMorphology`, `feDisplacementMap`, `feTile`, `feTurbulence`, and `feDiffuseLighting` and
//!   `feSpecularLighting` with a distant, point or spot light. CSS filter functions reach here as
//!   these primitives. Inputs are `SourceGraphic`, `SourceAlpha` and earlier results; usvg
//!   already sends a missing or unknown `in` to the previous result (or `SourceGraphic` for the
//!   first primitive), and a region in `objectBoundingBox` units to user space.
//! - A filter with an `feImage`, a transfer function of more than [`MAX_TRANSFER_VALUES`]
//!   values, or a convolution kernel more than [`MAX_CONVOLVE_ORDER`] on a side is
//!   [`Unsupported`], and the element is drawn unfiltered. `numOctaves` past
//!   [`MAX_TURBULENCE_OCTAVES`] counts as that many: further octaves change no 8-bit value.
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

/// Most values of one `feFuncX` table or discrete function.
pub const MAX_TRANSFER_VALUES: usize = 1_024;

/// Most columns, and most rows, of an `feConvolveMatrix` kernel; each output pixel reads that
/// many pixels squared.
pub const MAX_CONVOLVE_ORDER: u32 = 16;

/// Most `feTurbulence` octaves run. Octave `k` adds at most 2⁻ᵏ of the full range, so the
/// octaves past ten together add less than half of one 8-bit step.
pub const MAX_TURBULENCE_OCTAVES: u32 = 10;

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
    /// Each channel through its function, on straight values from 0 to 1: R, G, B, A.
    ComponentTransfer {
        input: FilterInput,
        functions: [TransferFunction; 4],
    },
    /// Each output pixel the sum of the input pixels under the kernel (rows top to bottom, the
    /// kernel's cell `(column, row)` at index `row · columns + column`) with the target cell
    /// over it, divided by `divisor`, plus `bias`. Kernel cells are sprite pixels.
    ConvolveMatrix {
        input: FilterInput,
        columns: u32,
        rows: u32,
        kernel: Vec<f64>,
        target: (u32, u32),
        divisor: f64,
        bias: f64,
        edge: EdgeMode,
        /// The kernel runs on straight colour and alpha is kept; else on premultiplied RGBA.
        preserve_alpha: bool,
    },
    /// Each output pixel the least (`erode`) or greatest (`dilate`) of the input's channels
    /// within the radius, user space.
    Morphology {
        input: FilterInput,
        dilate: bool,
        radius: (f64, f64),
    },
    /// `input`'s pixels moved by `scale · (C − ½)` user units along x and along y, `C` the
    /// channels of `input2` that `channels` names, straight values from 0 to 1.
    DisplacementMap {
        input: FilterInput,
        input2: FilterInput,
        scale: f64,
        channels: (ColorChannel, ColorChannel),
    },
    /// The input's subregion repeated over the primitive's subregion.
    Tile {
        input: FilterInput,
    },
    /// Perlin turbulence, as the SVG specification's reference code computes it, at the user
    /// space position of each pixel.
    Turbulence {
        base_frequency: (f64, f64),
        /// At most [`MAX_TURBULENCE_OCTAVES`].
        octaves: u32,
        seed: i32,
        stitch: bool,
        /// `fractalNoise`; else `turbulence`.
        fractal: bool,
    },
    /// The input's alpha as a height map lit by `light`: `constant · N·L` times the colour,
    /// opaque.
    DiffuseLighting {
        input: FilterInput,
        surface_scale: f64,
        constant: f64,
        /// Straight sRGB.
        color: [f64; 3],
        light: LightSource,
    },
    /// The input's alpha as a height map lit by `light`: `constant · (N·H)^exponent` times the
    /// colour, its alpha the largest colour channel.
    SpecularLighting {
        input: FilterInput,
        surface_scale: f64,
        constant: f64,
        exponent: f64,
        /// Straight sRGB.
        color: [f64; 3],
        light: LightSource,
    },
}

/// An `feFuncR`, `feFuncG`, `feFuncB` or `feFuncA`.
#[derive(Clone, Debug, PartialEq)]
pub enum TransferFunction {
    Identity,
    /// Linear between the values, spread evenly from 0 to 1; no values is the identity.
    Table(Vec<f64>),
    /// Steps of the values, spread evenly from 0 to 1; no values is the identity.
    Discrete(Vec<f64>),
    Linear {
        slope: f64,
        intercept: f64,
    },
    Gamma {
        amplitude: f64,
        exponent: f64,
        offset: f64,
    },
}

impl TransferFunction {
    /// The function at `c`, from 0 to 1, clamped to 0 to 1.
    pub fn apply(&self, c: f64) -> f64 {
        let c = if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
        let v = match self {
            TransferFunction::Identity => c,
            TransferFunction::Table(v) => match v.len() {
                0 => c,
                1 => v.first().copied().unwrap_or(c),
                len => {
                    let n = (len - 1) as f64;
                    let k = ((c * n).floor() as usize).min(len - 2);
                    let (a, b) = (v.get(k).copied().unwrap_or(0.0), v.get(k + 1).copied().unwrap_or(0.0));
                    a + (c * n - k as f64) * (b - a)
                }
            },
            TransferFunction::Discrete(v) => match v.len() {
                0 => c,
                len => v.get(((c * len as f64).floor() as usize).min(len - 1)).copied().unwrap_or(c),
            },
            TransferFunction::Linear { slope, intercept } => slope * c + intercept,
            TransferFunction::Gamma { amplitude, exponent, offset } => amplitude * c.powf(*exponent) + offset,
        };
        if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// How `feConvolveMatrix` reads past the input's edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeMode {
    /// Transparent.
    None,
    /// The nearest edge pixel.
    Duplicate,
    /// From the opposite edge.
    Wrap,
}

/// A channel `feDisplacementMap` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChannel {
    R,
    G,
    B,
    A,
}

/// The light of `feDiffuseLighting` and `feSpecularLighting`. Positions are user space; angles
/// degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightSource {
    /// From infinitely far: `azimuth` clockwise from the x axis (user space, y down), `elevation`
    /// up from the surface.
    Distant {
        azimuth: f64,
        elevation: f64,
    },
    Point {
        position: [f64; 3],
    },
    Spot {
        position: [f64; 3],
        points_at: [f64; 3],
        /// Focus: the light falls off as the cosine to the axis to this power.
        exponent: f64,
        /// No light beyond this angle from the axis.
        cone: Option<f64>,
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
        K::ComponentTransfer(c) => FilterKind::ComponentTransfer {
            input: input(c.input(), names)?,
            functions: [transfer(c.func_r())?, transfer(c.func_g())?, transfer(c.func_b())?, transfer(c.func_a())?],
        },
        K::ConvolveMatrix(c) => {
            let m = c.matrix();
            if m.columns() > MAX_CONVOLVE_ORDER || m.rows() > MAX_CONVOLVE_ORDER {
                return Err(Unsupported(format!(
                    "an SVG convolution kernel is {} × {}; at most {MAX_CONVOLVE_ORDER} × {MAX_CONVOLVE_ORDER} is drawn",
                    m.columns(),
                    m.rows()
                )));
            }
            let kernel = m.data().iter().map(|v| finite(*v)).collect::<Result<Vec<_>, _>>()?;
            if kernel.len() != m.columns() as usize * m.rows() as usize {
                return Err(Unsupported("an SVG convolution kernel does not have one value per cell".into()));
            }
            FilterKind::ConvolveMatrix {
                input: input(c.input(), names)?,
                columns: m.columns(),
                rows: m.rows(),
                kernel,
                target: (m.target_x(), m.target_y()),
                divisor: finite(c.divisor().get())?,
                bias: finite(c.bias())?,
                edge: match c.edge_mode() {
                    usvg::filter::EdgeMode::None => EdgeMode::None,
                    usvg::filter::EdgeMode::Duplicate => EdgeMode::Duplicate,
                    usvg::filter::EdgeMode::Wrap => EdgeMode::Wrap,
                },
                preserve_alpha: c.preserve_alpha(),
            }
        }
        K::Morphology(m) => FilterKind::Morphology {
            input: input(m.input(), names)?,
            dilate: m.operator() == usvg::filter::MorphologyOperator::Dilate,
            radius: (finite(m.radius_x().get())?, finite(m.radius_y().get())?),
        },
        K::DisplacementMap(d) => FilterKind::DisplacementMap {
            input: input(d.input1(), names)?,
            input2: input(d.input2(), names)?,
            scale: finite(d.scale())?,
            channels: (channel(d.x_channel_selector()), channel(d.y_channel_selector())),
        },
        K::Tile(t) => FilterKind::Tile { input: input(t.input(), names)? },
        K::Turbulence(t) => FilterKind::Turbulence {
            base_frequency: (finite(t.base_frequency_x().get())?, finite(t.base_frequency_y().get())?),
            octaves: t.num_octaves().min(MAX_TURBULENCE_OCTAVES),
            seed: t.seed(),
            stitch: t.stitch_tiles(),
            fractal: t.kind() == usvg::filter::TurbulenceKind::FractalNoise,
        },
        K::DiffuseLighting(l) => FilterKind::DiffuseLighting {
            input: input(l.input(), names)?,
            surface_scale: finite(l.surface_scale())?,
            constant: finite(l.diffuse_constant())?,
            color: rgb(l.lighting_color()),
            light: light(l.light_source())?,
        },
        K::SpecularLighting(l) => FilterKind::SpecularLighting {
            input: input(l.input(), names)?,
            surface_scale: finite(l.surface_scale())?,
            constant: finite(l.specular_constant())?,
            exponent: finite(l.specular_exponent())?,
            color: rgb(l.lighting_color()),
            light: light(l.light_source())?,
        },
        K::Image(_) => return Err(Unsupported("the SVG filter primitive feImage is not drawn".into())),
    })
}

fn rgb(c: usvg::Color) -> [f64; 3] {
    [f64::from(c.red) / 255.0, f64::from(c.green) / 255.0, f64::from(c.blue) / 255.0]
}

fn channel(c: usvg::filter::ColorChannel) -> ColorChannel {
    match c {
        usvg::filter::ColorChannel::R => ColorChannel::R,
        usvg::filter::ColorChannel::G => ColorChannel::G,
        usvg::filter::ColorChannel::B => ColorChannel::B,
        usvg::filter::ColorChannel::A => ColorChannel::A,
    }
}

fn values(v: &[f32]) -> Result<Vec<f64>, Unsupported> {
    if v.len() > MAX_TRANSFER_VALUES {
        return Err(Unsupported(format!("an SVG transfer function has more than {MAX_TRANSFER_VALUES} values")));
    }
    v.iter().map(|v| finite(*v)).collect()
}

fn transfer(f: &usvg::filter::TransferFunction) -> Result<TransferFunction, Unsupported> {
    use usvg::filter::TransferFunction as T;
    Ok(match f {
        T::Identity => TransferFunction::Identity,
        T::Table(v) => TransferFunction::Table(values(v)?),
        T::Discrete(v) => TransferFunction::Discrete(values(v)?),
        T::Linear { slope, intercept } => TransferFunction::Linear { slope: finite(*slope)?, intercept: finite(*intercept)? },
        T::Gamma { amplitude, exponent, offset } => {
            TransferFunction::Gamma { amplitude: finite(*amplitude)?, exponent: finite(*exponent)?, offset: finite(*offset)? }
        }
    })
}

fn light(l: usvg::filter::LightSource) -> Result<LightSource, Unsupported> {
    use usvg::filter::LightSource as L;
    Ok(match l {
        L::DistantLight(d) => LightSource::Distant { azimuth: finite(d.azimuth)?, elevation: finite(d.elevation)? },
        L::PointLight(p) => LightSource::Point { position: [finite(p.x)?, finite(p.y)?, finite(p.z)?] },
        L::SpotLight(s) => LightSource::Spot {
            position: [finite(s.x)?, finite(s.y)?, finite(s.z)?],
            points_at: [finite(s.points_at_x)?, finite(s.points_at_y)?, finite(s.points_at_z)?],
            exponent: finite(s.specular_exponent.get())?,
            cone: s.limiting_cone_angle.map(finite).transpose()?,
        },
    })
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
            <rect id="r" width="5" height="5"/>
            <filter id="f"><feImage href="#r"/></filter>
            <rect width="50" height="50" filter="url(#f)"/></svg>"##,
        );
        let e = filters(first_group(&t), Affine::IDENTITY).unwrap_err();
        assert!(e.0.contains("feImage"), "{e:?}");
    }

    fn one_kind(primitive: &str) -> Result<FilterKind, Unsupported> {
        let t = tree(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <filter id="f">{primitive}</filter><rect width="50" height="50" filter="url(#f)"/></svg>"##
        ));
        let f = filters(first_group(&t), Affine::IDENTITY)?;
        Ok(f.first().and_then(|f| f.primitives.first()).map(|p| p.kind.clone()).unwrap())
    }

    #[test]
    fn the_remaining_primitives_become_data() {
        let k = one_kind(r#"<feComponentTransfer><feFuncR type="table" tableValues="1 0"/><feFuncA type="linear" slope="0.5" intercept="0.25"/></feComponentTransfer>"#).unwrap();
        let FilterKind::ComponentTransfer { functions, .. } = &k else { panic!("{k:?}") };
        assert_eq!(functions[0], TransferFunction::Table(vec![1.0, 0.0]));
        assert_eq!(functions[1], TransferFunction::Identity);
        assert!((functions[3].apply(1.0) - 0.75).abs() < 1e-9);
        assert!((functions[0].apply(0.25) - 0.75).abs() < 1e-9, "a table interpolates");
        assert_eq!(TransferFunction::Discrete(vec![0.2, 0.8]).apply(0.6), 0.8);

        let k = one_kind(r#"<feConvolveMatrix order="3" kernelMatrix="0 0 0 0 1 0 0 0 0" edgeMode="wrap"/>"#).unwrap();
        assert!(matches!(k, FilterKind::ConvolveMatrix { columns: 3, rows: 3, target: (1, 1), edge: EdgeMode::Wrap, .. }), "{k:?}");
        let k = one_kind(r#"<feSpecularLighting specularExponent="20"><fePointLight x="1" y="2" z="3"/></feSpecularLighting>"#).unwrap();
        assert!(matches!(k, FilterKind::SpecularLighting { light: LightSource::Point { position: [1.0, 2.0, 3.0] }, exponent: 20.0, .. }), "{k:?}");
        let k = one_kind(r#"<feTurbulence baseFrequency="0.05 0.1" numOctaves="3" seed="7" type="fractalNoise"/>"#).unwrap();
        assert_eq!(k, FilterKind::Turbulence { base_frequency: (0.05, 0.1), octaves: 3, seed: 7, stitch: false, fractal: true });
    }

    #[test]
    fn hostile_kernels_tables_and_octaves_are_capped() {
        // A 17 × 17 kernel is refused; 16 × 16 is drawn.
        let kernel = |n: u32| {
            let values = vec!["1"; (n * n) as usize].join(" ");
            format!(r#"<feConvolveMatrix order="{n}" kernelMatrix="{values}"/>"#)
        };
        let e = one_kind(&kernel(MAX_CONVOLVE_ORDER + 1)).unwrap_err();
        assert!(e.0.contains("kernel"), "{e:?}");
        assert!(one_kind(&kernel(MAX_CONVOLVE_ORDER)).is_ok());

        let table = vec!["0.5"; MAX_TRANSFER_VALUES + 1].join(" ");
        let e = one_kind(&format!(r#"<feComponentTransfer><feFuncG type="discrete" tableValues="{table}"/></feComponentTransfer>"#)).unwrap_err();
        assert!(e.0.contains("transfer function"), "{e:?}");

        let k = one_kind(r#"<feTurbulence baseFrequency="0.05" numOctaves="2000000000"/>"#).unwrap();
        assert!(matches!(k, FilterKind::Turbulence { octaves: MAX_TURBULENCE_OCTAVES, .. }), "{k:?}");
    }
}

//! SVG plots of variation supports (what fontTools' `varLib.plot` draws in a window) and of
//! planned `avar` maps.
//!
//! One panel per master, in the model's order, laid out in a grid of ⌈√n⌉ columns. Each panel
//! draws the scalar the master's deltas are multiplied by: for one axis a curve over −1…1 through
//! the points `varLib.plot` samples (ten steps on each side of the peak), for two axes the same
//! wireframe in an oblique projection. Every panel marks the master locations with dots at
//! height 1. Panels of masters that vary on an axis outside the plotted ones are left out.

use std::fmt::Write as _;

use read_fonts::tables::gvar::Gvar;
use read_fonts::types::GlyphId;
use read_fonts::{FontData, FontRead};

use super::plan::AxisPlanResult;
use super::{describe_model, fmt_g, support_scalar};
use crate::limits::MAX_TUPLE_VARIATIONS;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::variations::fvar_axes;
use crate::{Result, TablesError};

/// Most panels one plot draws.
pub const MAX_PANELS: usize = 4_096;

/// One panel: a title and the support drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlotPanel {
    pub title: String,
    /// Per axis, `(start, peak, end)`; axes left out do not take part.
    pub support: Vec<(Tag, [f64; 3])>,
}

const COLOURS: [&str; 9] = ["#c8413a", "#2f6fb5", "#3d8f45", "#8556a8", "#d9801b", "#a8921a", "#87573a", "#c9649e", "#6f6f6f"];
const BACKGROUND: &str = "#ffffff";
const INK: &str = "#333333";
const FAINT: &str = "#bbbbbb";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn tag_text(tag: Tag) -> String {
    tag_str(tag).trim_end().to_string()
}

/// `varLib.plot`'s `stops`: ten steps from the start to the peak, ten to the end, and the end.
fn stops(a: f64, b: f64, c: f64) -> Vec<f64> {
    let count = 10.0;
    let mut out: Vec<f64> = (0..10).map(|i| a + (b - a) * f64::from(i) / count).collect();
    out.extend((0..10).map(|i| b + (c - b) * f64::from(i) / count));
    out.push(c);
    out
}

/// The panels' supports on `axes`: `(-1, 0, 1)` for an axis the support leaves out, as
/// `varLib.plot` draws it.
fn tent(support: &[(Tag, [f64; 3])], axis: Tag) -> [f64; 3] {
    support.iter().find(|(t, _)| *t == axis).map_or([-1.0, 0.0, 1.0], |(_, s)| *s)
}

/// The SVG of `panels` over one or two `axes`, with `dots` (locations) marked in each.
pub fn plot_panels(panels: &[PlotPanel], dots: &[Vec<(Tag, f64)>], axes: &[Tag]) -> Result<String> {
    if !(1..=2).contains(&axes.len()) {
        return Err(TablesError::BadOptions("plots show one or two axes".into()));
    }
    if panels.is_empty() {
        return Err(TablesError::BadOptions("there is nothing to plot on these axes".into()));
    }
    if panels.len() > MAX_PANELS {
        return Err(TablesError::TooLarge { what: "the panels of one plot", limit: MAX_PANELS });
    }
    let (w, h) = (280.0, 220.0);
    let n = panels.len();
    let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
    let rows = n.div_ceil(cols);
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" font-family=\"sans-serif\" font-size=\"11\">",
        cols as f64 * w,
        rows as f64 * h,
        cols as f64 * w,
        rows as f64 * h
    );
    let _ = writeln!(svg, "<rect width=\"100%\" height=\"100%\" fill=\"{BACKGROUND}\"/>");
    for (i, panel) in panels.iter().enumerate() {
        let (ox, oy) = ((i % cols) as f64 * w, (i / cols) as f64 * h);
        let colour = COLOURS.get(i % COLOURS.len()).copied().unwrap_or(INK);
        let _ = writeln!(svg, "<g class=\"panel\" transform=\"translate({ox} {oy})\">");
        let _ = writeln!(svg, "<text x=\"{}\" y=\"18\" text-anchor=\"middle\" fill=\"{INK}\">{}</text>", w / 2.0, escape(&panel.title));
        match axes {
            [axis] => one_axis(&mut svg, panel, dots, *axis, colour, w, h),
            [a, b] => two_axes(&mut svg, panel, dots, (*a, *b), colour, w),
            _ => {}
        }
        svg.push_str("</g>\n");
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

fn polyline(svg: &mut String, points: &[(f64, f64)], colour: &str, width: f64) {
    let pts: Vec<String> = points.iter().map(|(x, y)| format!("{x:.2},{y:.2}")).collect();
    let _ = writeln!(svg, "<polyline points=\"{}\" fill=\"none\" stroke=\"{colour}\" stroke-width=\"{width}\"/>", pts.join(" "));
}

fn one_axis(svg: &mut String, panel: &PlotPanel, dots: &[Vec<(Tag, f64)>], axis: Tag, colour: &str, w: f64, h: f64) {
    let (left, right, top, bottom) = (36.0, w - 16.0, 34.0, h - 34.0);
    let px = |x: f64| left + (x + 1.0) / 2.0 * (right - left);
    let py = |y: f64| bottom - y * (bottom - top);
    let _ = writeln!(svg, "<rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{FAINT}\"/>", right - left, bottom - top);
    for x in [-1.0, 0.0, 1.0] {
        let _ = writeln!(svg, "<text x=\"{:.2}\" y=\"{}\" text-anchor=\"middle\" fill=\"{INK}\">{}</text>", px(x), bottom + 14.0, fmt_g(x));
    }
    for y in [0.0, 1.0] {
        let _ = writeln!(svg, "<text x=\"{}\" y=\"{:.2}\" text-anchor=\"end\" fill=\"{INK}\">{}</text>", left - 4.0, py(y) + 4.0, fmt_g(y));
    }
    let _ = writeln!(svg, "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" fill=\"{INK}\">{}</text>", w / 2.0, h - 6.0, escape(&tag_text(axis)));
    let [a, b, c] = tent(&panel.support, axis);
    let points: Vec<(f64, f64)> = stops(a, b, c).into_iter().map(|x| (px(x), py(support_scalar(&[(axis, x)], &panel.support)))).collect();
    polyline(svg, &points, colour, 1.5);
    for loc in dots {
        let x = loc.iter().find(|(t, _)| *t == axis).map_or(0.0, |(_, v)| *v);
        let _ = writeln!(svg, "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"3\" fill=\"{colour}\"/>", px(x), py(1.0));
    }
}

fn two_axes(svg: &mut String, panel: &PlotPanel, dots: &[Vec<(Tag, f64)>], (ax1, ax2): (Tag, Tag), colour: &str, w: f64) {
    let s = 70.0;
    let (cx, cy) = (w / 2.0, 150.0);
    let proj = |x: f64, y: f64, z: f64| (cx + (x - y) * 0.866 * s, cy - (x + y) * 0.25 * s - z * s);
    let base: Vec<(f64, f64)> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0)].iter().map(|&(x, y)| proj(x, y, 0.0)).collect();
    polyline(svg, &base, FAINT, 1.0);
    let (lx, ly) = proj(1.15, -1.0, 0.0);
    let _ = writeln!(svg, "<text x=\"{lx:.2}\" y=\"{:.2}\" fill=\"{INK}\">{}</text>", ly + 12.0, escape(&tag_text(ax1)));
    let (lx, ly) = proj(-1.0, 1.15, 0.0);
    let _ = writeln!(svg, "<text x=\"{lx:.2}\" y=\"{:.2}\" text-anchor=\"end\" fill=\"{INK}\">{}</text>", ly + 12.0, escape(&tag_text(ax2)));
    let xs = tent(&panel.support, ax1);
    let ys = tent(&panel.support, ax2);
    let z = |x: f64, y: f64| support_scalar(&[(ax1, x), (ax2, y)], &panel.support);
    for x in stops(xs[0], xs[1], xs[2]) {
        let line: Vec<(f64, f64)> = ys.iter().map(|&y| proj(x, y, z(x, y))).collect();
        polyline(svg, &line, colour, 0.8);
    }
    for y in stops(ys[0], ys[1], ys[2]) {
        let line: Vec<(f64, f64)> = xs.iter().map(|&x| proj(x, y, z(x, y))).collect();
        polyline(svg, &line, colour, 0.8);
    }
    for loc in dots {
        let get = |a: Tag| loc.iter().find(|(t, _)| *t == a).map_or(0.0, |(_, v)| *v);
        let (x, y) = proj(get(ax1), get(ax2), 1.0);
        let _ = writeln!(svg, "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"3\" fill=\"{colour}\"/>");
    }
}

/// The axes to plot: `axes` when given, else every axis of `found` when there are at most two.
fn chosen_axes(found: &[Tag], axes: Option<&[Tag]>) -> Result<Vec<Tag>> {
    match axes {
        Some(a) if (1..=2).contains(&a.len()) => Ok(a.to_vec()),
        Some(_) => Err(TablesError::BadOptions("give one or two axes to plot".into())),
        None if (1..=2).contains(&found.len()) => Ok(found.to_vec()),
        None if found.is_empty() => Err(TablesError::BadOptions("the locations name no axis".into())),
        None => Err(TablesError::BadOptions(format!(
            "there are {} axes ({}); choose one or two to plot",
            found.len(),
            found.iter().map(|t| tag_text(*t)).collect::<Vec<_>>().join(", ")
        ))),
    }
}

fn on_axes(support: &[(Tag, [f64; 3])], axes: &[Tag]) -> bool {
    support.iter().all(|(t, _)| axes.contains(t))
}

/// The plot of the model of `locations` (normalised). `names` titles the masters (by the
/// caller's order); masters without a name are titled by their location.
pub fn plot_with(locations: &[Vec<(Tag, f64)>], names: &[String], axes: Option<&[Tag]>) -> Result<String> {
    let supports = describe_model(locations)?;
    let mut found: Vec<Tag> = Vec::new();
    for (t, _) in locations.iter().flatten() {
        if !found.contains(t) {
            found.push(*t);
        }
    }
    let axes = chosen_axes(&found, axes)?;
    let panels: Vec<PlotPanel> = supports
        .iter()
        .filter(|s| on_axes(&s.region, &axes))
        .map(|s| {
            let title = names.get(s.master).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| location_title(&s.location));
            PlotPanel { title, support: s.region.clone() }
        })
        .collect();
    plot_panels(&panels, locations, &axes)
}

fn location_title(location: &[(Tag, f64)]) -> String {
    if location.is_empty() {
        return "default".into();
    }
    location.iter().map(|(t, v)| format!("{}={}", tag_text(*t), fmt_g(*v))).collect::<Vec<_>>().join(" ")
}

/// The distinct regions of the font's `gvar` tuples and `HVAR` store, as panels, with the `fvar`
/// axes and the regions' peaks (to mark as dots).
#[allow(clippy::type_complexity)]
pub fn font_panels(font: &SfntFont) -> Result<(Vec<Tag>, Vec<PlotPanel>, Vec<Vec<(Tag, f64)>>)> {
    let axes: Vec<Tag> = fvar_axes(font)?.iter().map(|a| a.tag).collect();
    let mut regions: Vec<Vec<[f64; 3]>> = Vec::new();
    let mut add = |r: Vec<[f64; 3]>| -> Result<()> {
        if r.iter().any(|a| a[1] != 0.0) && !regions.contains(&r) {
            if regions.len() >= MAX_PANELS {
                return Err(TablesError::TooLarge { what: "the distinct regions of the font", limit: MAX_PANELS });
            }
            regions.push(r);
        }
        Ok(())
    };
    let f2 = |bits: i16| f64::from(bits) / 16384.0;
    if let Some(bytes) = font.get(*b"gvar") {
        let gvar = Gvar::read(FontData::new(bytes)).map_err(|e| TablesError::invalid(format!("the gvar table cannot be read: {e}")))?;
        let count = crate::ttx::num_glyphs(font)?;
        for g in 0..count {
            let gid = GlyphId::new(u32::try_from(g).unwrap_or(u32::MAX));
            let Ok(Some(data)) = gvar.glyph_variation_data(gid) else { continue };
            for (n, tuple) in data.tuples().enumerate() {
                if n >= MAX_TUPLE_VARIATIONS {
                    break;
                }
                let peak = tuple.peak();
                let start = tuple.intermediate_start();
                let end = tuple.intermediate_end();
                add((0..axes.len())
                    .map(|i| {
                        let p = peak.get(i).map_or(0.0, |v| f2(v.to_bits()));
                        let s = start.as_ref().and_then(|t| t.get(i)).map_or(p.min(0.0), |v| f2(v.to_bits()));
                        let e = end.as_ref().and_then(|t| t.get(i)).map_or(p.max(0.0), |v| f2(v.to_bits()));
                        if p == 0.0 { [0.0; 3] } else { [s, p, e] }
                    })
                    .collect())?;
            }
        }
    }
    if let Some(bytes) = font.get(*b"HVAR") {
        let hvar = read_fonts::tables::hvar::Hvar::read(FontData::new(bytes))
            .map_err(|e| TablesError::invalid(format!("the HVAR table cannot be read: {e}")))?;
        let store = hvar.item_variation_store().map_err(|e| TablesError::invalid(format!("the HVAR variation store cannot be read: {e}")))?;
        let list = store.variation_region_list().map_err(|e| TablesError::invalid(format!("the HVAR region list cannot be read: {e}")))?;
        for region in list.variation_regions().iter().flatten() {
            add(region
                .region_axes()
                .iter()
                .map(|a| {
                    let p = f2(a.peak_coord().to_bits());
                    if p == 0.0 { [0.0; 3] } else { [f2(a.start_coord().to_bits()), p, f2(a.end_coord().to_bits())] }
                })
                .collect())?;
        }
    }
    let mut panels = Vec::with_capacity(regions.len());
    let mut dots = vec![Vec::new()];
    for r in regions {
        let support: Vec<(Tag, [f64; 3])> = axes.iter().zip(&r).filter(|(_, a)| a[1] != 0.0).map(|(t, a)| (*t, *a)).collect();
        let title =
            support.iter().map(|(t, [s, p, e])| format!("{} {}:{}:{}", tag_text(*t), fmt_g(*s), fmt_g(*p), fmt_g(*e))).collect::<Vec<_>>().join(", ");
        dots.push(support.iter().map(|(t, a)| (*t, a[1])).collect());
        panels.push(PlotPanel { title, support });
    }
    Ok((axes, panels, dots))
}

/// The plot of a font's regions on `axes` (every `fvar` axis when there are at most two).
pub fn plot_font(font: &SfntFont, axes: Option<&[Tag]>) -> Result<String> {
    let (all, panels, dots) = font_panels(font)?;
    let axes = chosen_axes(&all, axes)?;
    let panels: Vec<PlotPanel> = panels.into_iter().filter(|p| on_axes(&p.support, &axes)).collect();
    plot_panels(&panels, &dots, &axes)
}

/// An SVG of planned `avar` maps: one panel per axis with a map, the normalised map over the
/// dashed identity line.
pub fn plan_plot(axes: &[AxisPlanResult]) -> String {
    let planned: Vec<&AxisPlanResult> = axes.iter().filter(|a| !a.normalized.is_empty()).collect();
    let (w, h) = (260.0, 260.0);
    let n = planned.len().max(1);
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{h}\" viewBox=\"0 0 {} {h}\" font-family=\"sans-serif\" font-size=\"11\">",
        n as f64 * w,
        n as f64 * w
    );
    let _ = writeln!(svg, "<rect width=\"100%\" height=\"100%\" fill=\"{BACKGROUND}\"/>");
    if planned.is_empty() {
        let _ = writeln!(svg, "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" fill=\"{INK}\">no axis needs a map</text>", w / 2.0, h / 2.0);
    }
    for (i, axis) in planned.iter().enumerate() {
        let ox = i as f64 * w;
        let colour = COLOURS.get(i % COLOURS.len()).copied().unwrap_or(INK);
        let (left, right, top, bottom) = (30.0, w - 14.0, 30.0, h - 30.0);
        let px = |x: f64| left + (x + 1.0) / 2.0 * (right - left);
        let py = |y: f64| bottom - (y + 1.0) / 2.0 * (bottom - top);
        let _ = writeln!(svg, "<g class=\"panel\" transform=\"translate({ox} 0)\">");
        let _ = writeln!(svg, "<text x=\"{}\" y=\"18\" text-anchor=\"middle\" fill=\"{INK}\">{}</text>", w / 2.0, escape(&tag_text(axis.tag)));
        let _ =
            writeln!(svg, "<rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{FAINT}\"/>", right - left, bottom - top);
        let _ = writeln!(
            svg,
            "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{FAINT}\" stroke-dasharray=\"4 3\"/>",
            px(-1.0),
            py(-1.0),
            px(1.0),
            py(1.0)
        );
        let points: Vec<(f64, f64)> = axis.normalized.iter().map(|(k, v)| (px(*k), py(*v))).collect();
        polyline(&mut svg, &points, colour, 1.5);
        for (x, y) in &points {
            let _ = writeln!(svg, "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"2\" fill=\"{colour}\"/>");
        }
        svg.push_str("</g>\n");
    }
    svg.push_str("</svg>\n");
    svg
}

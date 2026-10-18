//! Small caps, subscripts and superscripts derived from existing glyphs (M13.19): `glyph.addSmallCaps`
//! and `glyph.addSubSuperscripts`.
//!
//! Both commands make new glyphs from the outlines of existing ones and never touch the
//! originals. Each new layer is the source layer scaled about the origin (and, for subscripts and
//! superscripts, shifted up or down), with its stems then made thicker or thinner by the stem
//! filters' code (`fontcraft_pathops::stem_filters`):
//!
//! 1. Change Weight, method lcg, by the vertical stems' change, so the new glyph's zones (its
//!    baseline and top) stay where the scaling put them.
//! 2. Adjust Stems on the horizontal stems, when they should change by a different amount than
//!    the vertical ones; edges on the new glyph's zones stay.
//!
//! A stem scale is the ratio of a new stem's width to the source stem's width. The amount each
//! stem changes is measured on reference glyphs of each master: the vertical and horizontal stems
//! of the capital H (else I) for capitals, figures and symbols, of the lowercase n (else l) for
//! lowercase sources. A master without reference stems gets no stem change, with a warning.
//!
//! A glyph that already exists under the new name gets new layers for the masters the command
//! works on; its other layers stay. The substitutions go into the font's features: lines missing
//! from a hand-written feature with the tag are appended to it, a feature with the tag is added
//! when there is none, and an automatic feature with the tag is regenerated instead. Running a
//! command twice does not repeat a line.
//!
//! Paths are scaled node by node; components are scaled with them, and a component whose base
//! glyph has a derived glyph (made in the same run or already in the font) points at that glyph
//! instead. Images and shapes the model keeps unread are left out.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_features::auto::{self, AutoContext};
use fontcraft_features::{EntryKind, EntryRef, MAX_ENTRIES, MAX_ENTRY_BYTES};
use fontcraft_geom::{Affine, Point};
use fontcraft_model::glyphdata::{Category, GlyphData, validate};
use fontcraft_model::plist::Value as PValue;
use fontcraft_model::{
    Component, Font, Glyph, Layer, MAX_COORDINATE, MAX_GLYPHS, MAX_NAME_LEN, Master, Metrics, NodeKind, Shape, Snippet, Transform, insert_by_sort_key,
};
use fontcraft_pathops::stem_filters::{self as sf, AdjustStems, ChangeWeight, CounterMode, StemRule, WeightMethod};
use fontcraft_pathops::stems::{StemDirection, StemOptions, detect_stems};
use fontcraft_pathops::{Contour, Node as PNode, NodeKind as PKind};
use serde_json::json;

use super::common::{grid, in_range, master_metrics};
use super::stem_filters::MAX_STEM_FILTER_WORK;
use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most bytes of a suffix.
const MAX_SUFFIX_BYTES: usize = 64;
/// Largest scale factor (outline and stems).
const MAX_FACTOR: f64 = 10.0;
/// Most master ids one `masters` list may hold.
const MAX_MASTER_IDS: usize = 1024;
/// A stem change smaller than this, in font units, is not made.
const MIN_STEM_CHANGE: f64 = 0.5;
/// The reference's default size of subscripts and superscripts.
const DEFAULT_SUB_SUPER_SCALE: f64 = 2.0 / 3.0;
/// The reference's default stem scale of subscripts and superscripts.
const DEFAULT_SUB_SUPER_STEMS: f64 = 0.75;
/// The reference's default raise, in units of a 1000-unit em.
const DEFAULT_RAISE: f64 = 450.0;
/// Our default drop for subscripts and scientific inferiors, in units of a 1000-unit em.
const DEFAULT_DROP: f64 = -150.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.addSmallCaps",
            "Add Small Caps…",
            ["Glyph"],
            None,
            "{glyphs?: [names] (default: every glyph with a character; letters without a capital and lowercase pair, and other characters without `includeSymbols`, are left out quietly), masters?: [ids] (default: all), suffix?: string (default \".sc\", \".pc\" with `petite`; a dot is added in front when it has none), height?: units (the small-cap height; default: the master's smallCapHeight parameter, else its x-height), stemScale?: {h?: number, v?: number} (new stem width ÷ the capital's: `v` for vertical stems, `h` for horizontal ones; default `v`: the lowercase n's stem ÷ the capital H's, `h`: the same as `v`), petite?: bool (petite capitals: suffix .pc, features pcap and c2pc), includeSymbols?: bool (figures, punctuation and symbols get a scaled copy too, under their own name plus the suffix, mapped in both features)} makes, for each letter, the small capital named after the lowercase letter plus the suffix (A and a both give a.sc) from the capital's outlines: scaled from the cap height to the small-cap height, its stems thickened toward the lowercase weight. The originals stay as they are. The substitutions go into the features smcp (lowercase → small cap) and c2sc (capital → small cap), or pcap and c2pc: appended to a hand-written feature with the tag, added as a new feature, or regenerated when that feature is automatic. One undo step → {glyphs: [made or remade], created: [names], replaced: [names], skipped: [{glyph, master?, reason}], features: [{tag, action: \"added\" | \"appended\" | \"unchanged\" | \"automatic\", lines}], warnings: [text]}",
            has_doc,
            add_small_caps
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyph.addSubSuperscripts",
            "Add Subscripts / Superscripts…",
            ["Glyph"],
            None,
            "{glyphs?: [names] (default: the figures zero to nine), masters?: [ids] (default: all), suffix: string (a dot is added in front when it has none), feature: tag (four letters, digits or underscores, such as sups, subs, sinf, numr, dnom), scale?: number (default 2/3), stemScale?: number (new stem width ÷ the source's; default 0.75, so the copies look darker than plain scaling makes them), shiftY?: units (default: 450 units per 1000 of the em up for every feature but subs, sinf and dnom; 150 per 1000 down for subs and sinf; 0 for dnom)} makes a scaled and shifted copy of each glyph under the suffixed name and maps each original to its copy in the feature. The originals stay as they are. The substitutions are appended to a hand-written feature with the tag, added as a new feature, or regenerated when that feature is automatic. One undo step → {glyphs: [made or remade], created: [names], replaced: [names], skipped: [{glyph, master?, reason}], features: [{tag, action, lines}], warnings: [text]}",
            has_doc,
            add_sub_superscripts
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

// ── Plans ───────────────────────────────────────────────────────────────────────────────────────

/// One glyph to make.
struct Plan {
    name: String,
    /// The glyph whose outlines it takes.
    source: String,
    /// Takes the lowercase reference stems and the x-height as its top.
    lowercase: bool,
    /// Feature lines that map to it, as (tag, line).
    lines: Vec<(String, String)>,
}

/// How one master's layers are made.
struct MasterCx {
    id: String,
    metrics: Metrics,
    /// Outline scale and vertical shift.
    scale: f64,
    shift: f64,
    /// Stem changes (vertical, horizontal), in font units, for capital-like and lowercase sources.
    upper: (f64, f64),
    lower: (f64, f64),
}

/// What the run made, before it is written.
#[derive(Default)]
struct Made {
    new_glyphs: Vec<Glyph>,
    /// Existing glyph → its new master layers.
    replaced: Vec<(String, Vec<Layer>)>,
    skipped: Vec<Value>,
    warnings: Vec<String>,
}

fn skip(list: &mut Vec<Value>, glyph: &str, master: Option<&str>, reason: impl Into<String>) {
    let mut v = json!({"glyph": glyph, "reason": reason.into()});
    if let (Some(m), Some(o)) = (master, v.as_object_mut()) {
        o.insert("master".into(), json!(m));
    }
    list.push(v);
}

/// The suffix as given, with a dot in front when it has none.
fn suffix(p: &Params, raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "." {
        return Err(p.bad("`suffix` is empty"));
    }
    if raw.len() > MAX_SUFFIX_BYTES {
        return Err(p.bad(format!("`suffix` is longer than {MAX_SUFFIX_BYTES} bytes")));
    }
    let s = if raw.starts_with('.') { raw.to_string() } else { format!(".{raw}") };
    validate(&format!("a{s}"), true).map_err(|e| p.bad(format!("`suffix` `{s}`: {e}")))?;
    Ok(s)
}

/// A factor above 0 and at most [`MAX_FACTOR`].
fn factor(p: &Params, key: &str, v: f64) -> Result<f64> {
    if v.is_finite() && v > 0.0 && v <= MAX_FACTOR { Ok(v) } else { Err(p.bad(format!("`{key}` must be above 0 and at most {MAX_FACTOR}, got {v}"))) }
}

/// The masters named by `masters`, else all.
fn masters<'f>(p: &Params, font: &'f Font) -> Result<Vec<&'f Master>> {
    let Some(ids) = p.opt_strings("masters")? else { return Ok(font.masters.iter().collect()) };
    if ids.len() > MAX_MASTER_IDS {
        return Err(p.bad(format!("`masters` holds {} ids; at most {MAX_MASTER_IDS} are allowed", ids.len())));
    }
    if ids.is_empty() {
        return Err(p.bad("`masters` is empty"));
    }
    let mut out: Vec<&Master> = Vec::new();
    for id in ids {
        let m = font.master(id).map_err(|_| p.bad(format!("there is no master `{id}`")))?;
        if !out.iter().any(|o| o.id == m.id) {
            out.push(m);
        }
    }
    Ok(out)
}

/// The font's glyph for each code point (the first in glyph order).
fn by_code_point(font: &Font) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for g in font.glyphs.iter() {
        for &u in &g.unicodes {
            map.entry(u).or_insert_with(|| g.name.clone());
        }
    }
    map
}

/// The glyph's character: its first code point, else the glyph data's for its name.
fn code_point(font: &Font, data: &GlyphData, name: &str) -> Option<u32> {
    font.glyph(name).and_then(|g| g.unicodes.first().copied()).or_else(|| data.lookup(name, &[]).unicode.map(|u| u.value))
}

fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

/// The (capital, lowercase) pair of a cased letter.
fn case_pair(c: char) -> Option<(char, char)> {
    let (upper, lower) = if c.is_uppercase() {
        (c, single(c.to_lowercase())?)
    } else if c.is_lowercase() {
        (single(c.to_uppercase())?, c)
    } else {
        return None;
    };
    (upper != lower).then_some((upper, lower))
}

/// Figures, punctuation and symbols.
fn is_symbol(data: &GlyphData, cp: u32) -> bool {
    matches!(data.general_category(cp).glyph_category().0, Category::Number | Category::Punctuation | Category::Symbol)
}

fn is_lowercase(font: &Font, data: &GlyphData, name: &str) -> bool {
    match code_point(font, data, name).and_then(char::from_u32) {
        Some(c) => c.is_lowercase(),
        None => name.chars().next().is_some_and(char::is_lowercase),
    }
}

/// A name the naming rules accept, or why not.
fn name_problem(name: &str) -> Option<String> {
    if name.len() > MAX_NAME_LEN {
        return Some(format!("the name `{name}` would be longer than {MAX_NAME_LEN} bytes"));
    }
    validate(name, true).err().map(|e| format!("`{name}`: {e}"))
}

/// What small cap a glyph gives: its plan, or why it gives none.
struct SmallCaps<'a> {
    font: &'a Font,
    data: &'a GlyphData,
    cps: HashMap<u32, String>,
    suffix: String,
    symbols: bool,
    tags: (&'static str, &'static str),
}

impl SmallCaps<'_> {
    fn plan(&self, name: &str) -> std::result::Result<Plan, String> {
        let Some(cp) = code_point(self.font, self.data, name) else { return Err("the glyph has no character".into()) };
        let Some(c) = char::from_u32(cp) else { return Err("the glyph has no character".into()) };
        let (lower_tag, upper_tag) = self.tags;
        if let Some((upper, lower)) = case_pair(c) {
            let Some(upper_name) = self.cps.get(&u32::from(upper)) else {
                return Err(format!("the font has no capital {}", upper.escape_unicode()));
            };
            let lower_glyph = self.cps.get(&u32::from(lower));
            let lower_name = lower_glyph.cloned().unwrap_or_else(|| self.data.nice_name(u32::from(lower)));
            let derived = format!("{lower_name}{}", self.suffix);
            let mut lines = Vec::new();
            if let Some(l) = lower_glyph {
                lines.push((lower_tag.to_string(), format!("sub {l} by {derived};")));
            }
            // A capital maps to the small cap of its own lowercase only (dotless i is not I's).
            if single(upper.to_lowercase()) == Some(lower) {
                lines.push((upper_tag.to_string(), format!("sub {upper_name} by {derived};")));
            }
            return Ok(Plan { name: derived, source: upper_name.clone(), lowercase: false, lines });
        }
        if !is_symbol(self.data, cp) {
            return Err("not a letter with a capital and a lowercase form".into());
        }
        if !self.symbols {
            return Err("not a letter; turn on includeSymbols for figures, punctuation and symbols".into());
        }
        let derived = format!("{name}{}", self.suffix);
        let lines = vec![(lower_tag.to_string(), format!("sub {name} by {derived};")), (upper_tag.to_string(), format!("sub {name} by {derived};"))];
        Ok(Plan { name: derived, source: name.to_string(), lowercase: false, lines })
    }
}

// ── Reference stems ─────────────────────────────────────────────────────────────────────────────

/// The median width of the straight stems running `direction` in the first of `candidates` (code
/// points) the font has with such stems on `master`; round stems when it has no straight ones.
fn reference_stem(font: &Font, cps: &HashMap<u32, String>, master: &str, candidates: &[char], direction: StemDirection) -> Option<f64> {
    for c in candidates {
        let Some(name) = cps.get(&u32::from(*c)) else { continue };
        let Ok(outline) = font.outline(name, master) else { continue };
        let options = StemOptions { direction: Some(direction), ..StemOptions::default() };
        let stems = detect_stems(&outline, &options);
        let straight: Vec<f64> = stems.iter().filter(|s| !s.round).map(|s| s.width).collect();
        let widths = if straight.is_empty() { stems.iter().map(|s| s.width).collect() } else { straight };
        if let Some(m) = median(widths) {
            return Some(m);
        }
    }
    None
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.retain(|w| w.is_finite());
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        return None;
    }
    let hi = v.get(n / 2).copied()?;
    if n % 2 == 1 { Some(hi) } else { v.get(n / 2 - 1).map(|lo| (lo + hi) / 2.0) }
}

/// The reference stems of one master: (capital vertical, capital horizontal, lowercase vertical,
/// lowercase horizontal).
fn reference_stems(font: &Font, cps: &HashMap<u32, String>, master: &str) -> [Option<f64>; 4] {
    let upper = ['H', 'I'];
    let lower = ['n', 'l'];
    [
        reference_stem(font, cps, master, &upper, StemDirection::Vertical),
        reference_stem(font, cps, master, &upper, StemDirection::Horizontal),
        reference_stem(font, cps, master, &lower, StemDirection::Vertical),
        reference_stem(font, cps, master, &lower, StemDirection::Horizontal),
    ]
}

/// Stem changes for sources measured by `(v, h)` reference widths, at outline scale `scale` and
/// stem scales `(ratio_v, ratio_h)`.
fn changes(refs: (Option<f64>, Option<f64>), scale: f64, ratios: (f64, f64)) -> Option<(f64, f64)> {
    let v = refs.0?;
    let h = refs.1.unwrap_or(v);
    Some(((ratios.0 - scale) * v, (ratios.1 - scale) * h))
}

fn check_change(p: &Params, master: &str, (v, h): (f64, f64)) -> Result<()> {
    for a in [v, h] {
        if !(a.is_finite() && a.abs() <= sf::MAX_WEIGHT_CHANGE) {
            return Err(p.bad(format!(
                "master `{master}`: the stems would change by {a:.1} units, more than the {} the stem filters allow",
                sf::MAX_WEIGHT_CHANGE
            )));
        }
    }
    Ok(())
}

/// A number of a master's or the font's parameter `name`.
fn param_number(font: &Font, master: &Master, name: &str) -> Option<f64> {
    let number = |v: &PValue| match v {
        PValue::Number(_) => v.as_num().map(|n| n.as_f64()),
        _ => v.as_str().and_then(|s| s.trim().parse::<f64>().ok()),
    };
    master.params.get(name).and_then(number).or_else(|| font.info.params.get(name).and_then(number)).filter(|v| in_range(*v))
}

// ── Building layers ─────────────────────────────────────────────────────────────────────────────

fn snap(v: f64, step: f64) -> f64 {
    if step > 0.0 { (v / step).round() * step } else { v }
}

fn pkind(k: NodeKind) -> PKind {
    match k {
        NodeKind::Line => PKind::Line,
        NodeKind::Curve => PKind::Curve,
        NodeKind::QCurve => PKind::QCurve,
        NodeKind::OffCurve => PKind::OffCurve,
    }
}

/// A transform in the simplest form that holds `a`.
fn transform_of(a: Affine, step: f64) -> Transform {
    let [xx, yx, xy, yy, dx, dy] = a.as_coeffs();
    let (dx, dy) = (snap(dx, step), snap(dy, step));
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    if near(yx, 0.0) && near(xy, 0.0) {
        if near(xx, 1.0) && near(yy, 1.0) {
            return Transform::translate(dx, dy);
        }
        return Transform::Decomposed { pos: (dx, dy), scale: (xx, yy), angle: 0.0, slant: (0.0, 0.0) };
    }
    Transform::Matrix([xx, yx, xy, yy, dx, dy])
}

/// The paths of `layer` as contours, with the shape index of each.
fn contours(layer: &Layer) -> (Vec<Contour>, Vec<usize>) {
    let (mut out, mut slots) = (Vec::new(), Vec::new());
    for (i, shape) in layer.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        let nodes = path.nodes.iter().map(|n| PNode { point: Point::new(n.x, n.y), kind: pkind(n.kind), smooth: n.smooth, tag: None }).collect();
        out.push(Contour::new(path.closed, nodes));
        slots.push(i);
    }
    (out, slots)
}

/// Writes moved contours back into `layer`, node by node.
fn write_back(layer: &mut Layer, slots: &[usize], moved: &[Contour]) -> std::result::Result<(), String> {
    for (&slot, contour) in slots.iter().zip(moved) {
        let Some(Shape::Path(path)) = layer.shapes.get_mut(slot) else { return Err("a path went missing".into()) };
        if path.nodes.len() != contour.nodes.len() {
            return Err("the stem change altered a path's nodes".into());
        }
        for (node, new) in path.nodes.iter_mut().zip(&contour.nodes) {
            node.x = new.point.x;
            node.y = new.point.y;
        }
    }
    Ok(())
}

/// The new layer made from `src` for one master, or why it cannot be made.
fn derive_layer(
    src: &Layer,
    m: &MasterCx,
    lowercase: bool,
    bases: &HashMap<String, String>,
    step: f64,
    work: &mut usize,
    warnings: &mut Vec<String>,
) -> std::result::Result<Layer, String> {
    let map = Affine::translate((0.0, m.shift)) * Affine::scale(m.scale);
    let at = |x: f64, y: f64| {
        let p = map * Point::new(x, y);
        (snap(p.x, step), snap(p.y, step))
    };
    let mut layer = Layer::new(m.id.clone(), snap(src.width * m.scale, step));
    let mut dropped = 0usize;
    for shape in &src.shapes {
        match shape {
            Shape::Path(path) => {
                let mut path = path.clone();
                for n in &mut path.nodes {
                    (n.x, n.y) = at(n.x, n.y);
                }
                layer.shapes.push(Shape::Path(path));
            }
            Shape::Component(c) => {
                let derived = bases.get(&c.glyph);
                let t = c.transform.affine();
                // A derived base is already scaled and shifted, so only the placement is mapped.
                let placed = if derived.is_some() { map * t * map.inverse() } else { map * t };
                let mut c = Component { transform: transform_of(placed, step), ..c.clone() };
                if let Some(d) = derived {
                    d.clone_into(&mut c.glyph);
                }
                layer.shapes.push(Shape::Component(c));
            }
            Shape::Image(_) | Shape::Raw(_) => dropped += 1,
        }
    }
    if dropped > 0 {
        warnings.push(format!("{dropped} image or unread shape(s) of layer `{}` were left out", src.id));
    }
    for a in &src.anchors {
        let mut a = a.clone();
        (a.x, a.y) = at(a.x, a.y);
        layer.anchors.push(a);
    }

    let (cv, ch) = if lowercase { m.lower } else { m.upper };
    let top_metric = if lowercase { m.metrics.x_height } else { m.metrics.cap_height };
    let (bottom, top) = (m.shift, m.shift + m.scale * top_metric);
    let (shapes, slots) = contours(&layer);
    let mut done = 0.0;
    if cv.abs() >= MIN_STEM_CHANGE && !shapes.is_empty() {
        let method = if top > bottom { WeightMethod::Lcg } else { WeightMethod::Cjk };
        let options = ChangeWeight { amount: cv, method, bottom, top, serif_height: 0.0, counters: CounterMode::Retain, cleanup: false, grid: step };
        let out = sf::change_weight_within(&shapes, &options, work).map_err(|e| format!("the stem change failed: {e}"))?;
        if out.rebuilt {
            return Err("the stem change rebuilt the paths".into());
        }
        write_back(&mut layer, &slots, &out.contours)?;
        layer.width = snap(layer.width + out.width_change, step);
        done = cv;
    }
    let rest = ch - done;
    if rest.abs() >= MIN_STEM_CHANGE && !shapes.is_empty() {
        let (shapes, slots) = contours(&layer);
        let options = AdjustStems {
            vertical: Vec::new(),
            horizontal: vec![StemRule { from: sf::MIN_STEM_WIDTH, to: sf::MAX_FILTER_STEM, set: None, offset: Some(rest) }],
            fixed_heights: vec![bottom, top],
            grid: step,
        };
        match sf::adjust_stems_within(&shapes, &options, work) {
            Ok(out) => write_back(&mut layer, &slots, &out.contours)?,
            Err(e) => warnings.push(format!("layer `{}`: the horizontal stems were left as the weight change made them: {e}", src.id)),
        }
    }

    let numbers = layer
        .shapes
        .iter()
        .flat_map(|s| match s {
            Shape::Path(p) => p.nodes.iter().flat_map(|n| [n.x, n.y]).collect(),
            Shape::Component(c) => c.transform.numbers(),
            _ => Vec::new(),
        })
        .chain(layer.anchors.iter().flat_map(|a| [a.x, a.y]));
    if !numbers.into_iter().all(in_range) || !(in_range(layer.width) && layer.width >= 0.0) {
        return Err(format!("the result would lie outside ±{MAX_COORDINATE} units"));
    }
    Ok(layer)
}

/// Builds every planned glyph's layers. Plans whose new glyph is locked, and layers that cannot
/// be made, go to `skipped`.
fn build(p: &Params, font: &Font, plans: &[Plan], masters: &[MasterCx], derived_name: &dyn Fn(&str) -> String, made: &mut Made) -> Result<()> {
    let step = grid(font);
    let planned: HashSet<&str> = plans.iter().map(|pl| pl.name.as_str()).collect();
    let mut bases: HashMap<String, String> = plans.iter().map(|pl| (pl.source.clone(), pl.name.clone())).collect();
    let mut work = MAX_STEM_FILTER_WORK;
    for plan in plans {
        let Some(source) = font.glyph(&plan.source) else {
            skip(&mut made.skipped, &plan.source, None, "the glyph is gone");
            continue;
        };
        let existing = font.glyph(&plan.name);
        if existing.is_some_and(|g| g.info.locked) {
            skip(&mut made.skipped, &plan.source, None, format!("the glyph `{}` is locked", plan.name));
            continue;
        }
        for layer in &source.layers {
            for c in layer.shapes.iter().filter_map(|s| if let Shape::Component(c) = s { Some(c) } else { None }) {
                if bases.contains_key(&c.glyph) {
                    continue;
                }
                let candidate = derived_name(&c.glyph);
                if candidate != c.glyph && (font.glyph(&candidate).is_some() || planned.contains(candidate.as_str())) {
                    bases.insert(c.glyph.clone(), candidate);
                }
            }
        }
        let mut layers = Vec::new();
        for m in masters {
            let Some(src) = source.layer(&m.id) else {
                skip(&mut made.skipped, &plan.source, Some(m.id.as_str()), "the glyph has no layer for this master");
                continue;
            };
            match derive_layer(src, m, plan.lowercase, &bases, step, &mut work, &mut made.warnings) {
                Ok(l) => layers.push(l),
                Err(reason) => skip(&mut made.skipped, &plan.source, Some(m.id.as_str()), reason),
            }
            if work == 0 {
                return Err(p.bad(format!(
                    "the stem changes reached their work limit at glyph `{}`; nothing was changed: run the command on fewer glyphs at a time",
                    plan.source
                )));
            }
        }
        if layers.is_empty() {
            continue;
        }
        match existing {
            Some(_) => made.replaced.push((plan.name.clone(), layers)),
            None => {
                let mut g = Glyph::new(plan.name.as_str());
                for fm in font.masters.iter() {
                    match layers.iter().position(|l| l.id == fm.id) {
                        Some(i) => g.layers.push(layers.swap_remove(i)),
                        None => {
                            let width = source.layer(&fm.id).map_or(0.0, |l| l.width);
                            g.layers.push(Layer::new(fm.id.clone(), width));
                        }
                    }
                }
                made.new_glyphs.push(g);
            }
        }
    }
    Ok(())
}

// ── Writing ─────────────────────────────────────────────────────────────────────────────────────

/// Writes the substitutions `lines` (tag, line) into the features; returns one report per tag.
fn write_features(font: &mut Font, data: &GlyphData, command: &str, lines: &[(String, String)], warnings: &mut Vec<String>) -> Result<Vec<Value>> {
    let mut tags: Vec<&str> = Vec::new();
    for (tag, _) in lines {
        if !tags.contains(&tag.as_str()) {
            tags.push(tag);
        }
    }
    let mut report = Vec::new();
    for tag in tags {
        let mut wanted: Vec<&str> = Vec::new();
        for (t, line) in lines {
            if t == tag && !wanted.contains(&line.as_str()) {
                wanted.push(line);
            }
        }
        let entries = &font.features.features;
        let automatic = entries.iter().position(|e| e.name == tag && e.automatic);
        let manual = entries.iter().position(|e| e.name == tag && !e.automatic);
        let action = match (manual, automatic) {
            (Some(i), _) => {
                let code = entries.get(i).map(|e| e.code.clone()).unwrap_or_default();
                let have: HashSet<&str> = code.lines().map(str::trim).collect();
                let missing: Vec<&str> = wanted.iter().copied().filter(|l| !have.contains(l)).collect();
                if missing.is_empty() {
                    "unchanged"
                } else {
                    let mut code = code.trim_end().to_string();
                    for l in &missing {
                        if !code.is_empty() {
                            code.push('\n');
                        }
                        code.push_str(l);
                    }
                    code.push('\n');
                    if code.len() > MAX_ENTRY_BYTES {
                        return Err(EngineError::BadParams {
                            command: command.into(),
                            message: format!("the feature `{tag}` would hold more than {MAX_ENTRY_BYTES} bytes"),
                        });
                    }
                    if let Some(e) = Arc::make_mut(&mut font.features).features.get_mut(i) {
                        e.code = code;
                    }
                    "appended"
                }
            }
            (None, Some(i)) => {
                let updated = AutoContext::from_font(font, data, None)
                    .and_then(|cx| auto::update_entry(&font.features, EntryRef { kind: EntryKind::Feature, index: i }, &cx));
                match updated {
                    Ok(updated) => font.features = Arc::new(updated),
                    Err(e) => warnings.push(format!("the automatic feature `{tag}` was not regenerated: {e}")),
                }
                "automatic"
            }
            (None, None) => {
                let f = &font.features;
                if f.prefixes.len() + f.classes.len() + f.features.len() >= MAX_ENTRIES {
                    return Err(EngineError::BadParams {
                        command: command.into(),
                        message: format!("the font already has {MAX_ENTRIES} prefixes, classes and features"),
                    });
                }
                let mut code = wanted.join("\n");
                code.push('\n');
                if code.len() > MAX_ENTRY_BYTES {
                    return Err(EngineError::BadParams {
                        command: command.into(),
                        message: format!("the feature `{tag}` would hold more than {MAX_ENTRY_BYTES} bytes"),
                    });
                }
                Arc::make_mut(&mut font.features).features.push(Snippet { name: tag.to_string(), code, ..Snippet::default() });
                "added"
            }
        };
        report.push(json!({"tag": tag, "action": action, "lines": wanted.len()}));
    }
    Ok(report)
}

/// Builds and writes the planned glyphs and their features as one edit.
fn run_plans(
    s: &mut Session,
    p: &Params,
    label: &str,
    plans: Vec<Plan>,
    masters: Vec<MasterCx>,
    derived_name: &dyn Fn(&str) -> String,
    mut made: Made,
) -> Result<Value> {
    let data = s.glyph_data()?;
    {
        let font = s.font()?;
        if font.glyphs.len().saturating_add(plans.len()) > MAX_GLYPHS {
            return Err(p.bad(format!("the font would hold more than {MAX_GLYPHS} glyphs")));
        }
        build(p, font, &plans, &masters, derived_name, &mut made)?;
    }
    let built: HashSet<String> = made.new_glyphs.iter().map(|g| g.name.clone()).chain(made.replaced.iter().map(|(n, _)| n.clone())).collect();
    let lines: Vec<(String, String)> = plans.iter().filter(|pl| built.contains(&pl.name)).flat_map(|pl| pl.lines.iter().cloned()).collect();
    let created: Vec<String> = made.new_glyphs.iter().map(|g| g.name.clone()).collect();
    let replaced: Vec<String> = made.replaced.iter().map(|(n, _)| n.clone()).collect();
    let glyphs: Vec<&String> = plans.iter().map(|pl| &pl.name).filter(|n| built.contains(*n)).collect();
    let Made { new_glyphs, replaced: new_layers, skipped, mut warnings } = made;
    let features = s.edit(label, |font| {
        insert_by_sort_key(font, &data, new_glyphs)?;
        for (name, layers) in new_layers {
            let mut g = font.glyph_mut(&name)?;
            for new in layers {
                match g.layers.iter_mut().find(|l| l.id == new.id) {
                    Some(old) => {
                        old.width = new.width;
                        old.shapes = new.shapes;
                        old.anchors = new.anchors;
                    }
                    None => g.layers.push(new),
                }
            }
        }
        write_features(font, &data, p.command(), &lines, &mut warnings)
    })?;
    Ok(json!({
        "glyphs": glyphs,
        "created": created,
        "replaced": replaced,
        "skipped": skipped,
        "features": features,
        "warnings": warnings,
    }))
}

// ── glyph.addSmallCaps ──────────────────────────────────────────────────────────────────────────

fn add_small_caps(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.addSmallCaps", v)?;
    let petite = p.bool_or("petite", false)?;
    let symbols = p.bool_or("includeSymbols", false)?;
    let suffix = suffix(&p, p.opt_str("suffix")?.unwrap_or(if petite { ".pc" } else { ".sc" }))?;
    let height = p.opt_f64("height")?;
    if let Some(h) = height
        && !(in_range(h) && h > 0.0)
    {
        return Err(p.bad(format!("`height` must be above 0 and within ±{MAX_COORDINATE}, got {h}")));
    }
    let (ratio_h, ratio_v) = match p.opt_object("stemScale")? {
        None => (None, None),
        Some(m) => {
            if let Some(k) = m.keys().find(|k| !["h", "v"].contains(&k.as_str())) {
                return Err(p.bad(format!("`stemScale` takes h and v, not `{k}`")));
            }
            let get = |k: &str| -> Result<Option<f64>> {
                match m.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(x) => {
                        let n = x.as_f64().ok_or_else(|| p.bad(format!("`stemScale.{k}` must be a number")))?;
                        factor(&p, &format!("stemScale.{k}"), n).map(Some)
                    }
                }
            };
            (get("h")?, get("v")?)
        }
    };
    let data = s.glyph_data()?;
    let font = s.font()?;
    let chosen = p.opt_glyphs("glyphs", font)?;
    let quiet = chosen.is_none();
    let names: Vec<String> = chosen.unwrap_or_else(|| font.glyphs.iter().filter(|g| !g.unicodes.is_empty()).map(|g| g.name.clone()).collect());
    let tags = if petite { ("pcap", "c2pc") } else { ("smcp", "c2sc") };
    let rules = SmallCaps { font, data: &data, cps: by_code_point(font), suffix, symbols, tags };

    let mut made = Made::default();
    let mut plans: Vec<Plan> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for name in &names {
        let plan = rules.plan(name).and_then(|pl| name_problem(&pl.name).map_or(Ok(pl), Err));
        match plan {
            Ok(pl) => match index.get(&pl.name).and_then(|&i| plans.get_mut(i)) {
                Some(same) => {
                    for line in pl.lines {
                        if !same.lines.contains(&line) {
                            same.lines.push(line);
                        }
                    }
                }
                None => {
                    index.insert(pl.name.clone(), plans.len());
                    plans.push(pl);
                }
            },
            Err(reason) if !quiet => skip(&mut made.skipped, name, None, reason),
            Err(_) => {}
        }
    }

    let mut masters_cx = Vec::new();
    for m in masters(&p, font)? {
        let metrics = master_metrics(font, m);
        if !(metrics.cap_height.is_finite() && metrics.cap_height > 0.0) {
            return Err(p.bad(format!("master `{}` has no cap height above 0 to scale from", m.id)));
        }
        let target = height.or_else(|| param_number(font, m, "smallCapHeight")).unwrap_or(metrics.x_height);
        if !(target.is_finite() && target > 0.0) {
            return Err(p.bad(format!("master `{}`: the small-cap height {target} is not above 0", m.id)));
        }
        let scale = factor(&p, "height ÷ cap height", target / metrics.cap_height)?;
        let [uv, uh, lv, _] = reference_stems(font, &rules.cps, &m.id);
        let v = match (ratio_v, uv, lv) {
            (Some(r), _, _) => r,
            (None, Some(u), Some(l)) if u > 0.0 => l / u,
            _ => {
                made.warnings.push(format!("master `{}`: no stems of H and n to compare; the small caps are scaled without stem changes", m.id));
                scale
            }
        };
        let h = ratio_h.unwrap_or(v);
        let upper = match changes((uv, uh), scale, (v, h)) {
            Some(c) => c,
            None => {
                if ratio_v.is_some() || ratio_h.is_some() {
                    made.warnings.push(format!("master `{}`: no stems of H to measure; the small caps are scaled without stem changes", m.id));
                }
                (0.0, 0.0)
            }
        };
        check_change(&p, &m.id, upper)?;
        masters_cx.push(MasterCx { id: m.id.clone(), metrics, scale, shift: 0.0, upper, lower: upper });
    }
    // Owned copies: the closure outlives the borrow of the font.
    let cps = rules.cps.clone();
    let suffix = rules.suffix.clone();
    let cp_of: HashMap<String, u32> = font.glyphs.iter().filter_map(|g| g.unicodes.first().map(|&u| (g.name.clone(), u))).collect();
    let data = Arc::clone(&data);
    let derived_name = move |base: &str| match cp_of.get(base).copied() {
        Some(cp) => match char::from_u32(cp).and_then(case_pair) {
            Some((_, lower)) => format!("{}{suffix}", cps.get(&u32::from(lower)).cloned().unwrap_or_else(|| data.nice_name(u32::from(lower)))),
            None => format!("{base}{suffix}"),
        },
        None => format!("{base}{suffix}"),
    };
    run_plans(s, &p, if petite { "Add Petite Caps" } else { "Add Small Caps" }, plans, masters_cx, &derived_name, made)
}

// ── glyph.addSubSuperscripts ────────────────────────────────────────────────────────────────────

fn add_sub_superscripts(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.addSubSuperscripts", v)?;
    let suffix = suffix(&p, p.str("suffix")?)?;
    let tag = p.str("feature")?;
    if !(tag.len() == 4 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        return Err(p.bad(format!("`feature` must be four letters, digits or underscores, got `{tag}`")));
    }
    let scale = factor(&p, "scale", p.opt_f64("scale")?.unwrap_or(DEFAULT_SUB_SUPER_SCALE))?;
    let ratio = factor(&p, "stemScale", p.opt_f64("stemScale")?.unwrap_or(DEFAULT_SUB_SUPER_STEMS))?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let em = f64::from(font.units_per_em) / 1000.0;
    let shift = match p.opt_f64("shiftY")? {
        Some(y) if in_range(y) => y,
        Some(y) => return Err(p.bad(format!("`shiftY` must be within ±{MAX_COORDINATE}, got {y}"))),
        None => match tag {
            "subs" | "sinf" => DEFAULT_DROP * em,
            "dnom" => 0.0,
            _ => DEFAULT_RAISE * em,
        },
    };
    let names: Vec<String> = match p.opt_glyphs("glyphs", font)? {
        Some(n) => n,
        None => {
            let cps = by_code_point(font);
            let figures: Vec<String> = ('0'..='9').filter_map(|c| cps.get(&u32::from(c)).cloned()).collect();
            if figures.is_empty() {
                return Err(p.bad("the font has no figures zero to nine: give `glyphs`"));
            }
            figures
        }
    };

    let mut made = Made::default();
    let mut plans = Vec::new();
    for name in &names {
        if name.ends_with(suffix.as_str()) {
            skip(&mut made.skipped, name, None, format!("the name already ends in `{suffix}`"));
            continue;
        }
        let derived = format!("{name}{suffix}");
        if let Some(reason) = name_problem(&derived) {
            skip(&mut made.skipped, name, None, reason);
            continue;
        }
        let lines = vec![(tag.to_string(), format!("sub {name} by {derived};"))];
        plans.push(Plan { name: derived, source: name.clone(), lowercase: is_lowercase(font, &data, name), lines });
    }

    let cps = by_code_point(font);
    let mut masters_cx = Vec::new();
    for m in masters(&p, font)? {
        let metrics = master_metrics(font, m);
        let [uv, uh, lv, lh] = reference_stems(font, &cps, &m.id);
        let upper = changes((uv, uh), scale, (ratio, ratio));
        let lower = changes((lv.or(uv), lh.or(lv).or(uh)), scale, (ratio, ratio));
        if upper.is_none() {
            made.warnings.push(format!("master `{}`: no stems of H to measure; the copies are scaled without stem changes", m.id));
        }
        let upper = upper.unwrap_or((0.0, 0.0));
        let lower = lower.unwrap_or(upper);
        check_change(&p, &m.id, upper)?;
        check_change(&p, &m.id, lower)?;
        masters_cx.push(MasterCx { id: m.id.clone(), metrics, scale, shift, upper, lower });
    }
    let derived_name = |base: &str| format!("{base}{suffix}");
    run_plans(s, &p, "Add Subscripts / Superscripts", plans, masters_cx, &derived_name, made)
}

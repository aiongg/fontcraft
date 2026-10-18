//! The `MATH` table (M13.47, `font.setMathData`): math constants, per-glyph math metrics,
//! extended shapes, size variants and glyph assemblies, from the data the source stores in the
//! keys glyphsLib and ufo2ft read, so a source compiles to the same table here and in fontmake.
//!
//! | Data | Where the source keeps it |
//! |---|---|
//! | the constants, `MinConnectorOverlap` among them | the master's user data (a UFO's font lib), [`CONSTANTS_KEY`]: a dictionary of numbers by the specification's names |
//! | an extended shape | the glyph's user data, [`EXTENDED_SHAPE_KEY`] (any value); a UFO lists the names under the same key in its font lib |
//! | size variants and assemblies | the glyph's user data, or a UFO glyph's lib, [`VARIANTS_KEY`]: `hVariants`, `vVariants` (glyph names) and `hAssembly`, `vAssembly` (parts as `[name, flags, start connector, end connector]`) |
//! | italic correction | the anchor `math.ic`: its x minus the advance width |
//! | top accent attachment | the anchor `math.ta`: its x |
//! | math kerning | anchors `math.tr…`, `math.tl…`, `math.br…`, `math.bl…`: each a kern value (x, from the advance on the right sides, negated on the left) at a height (y) |
//!
//! A variant's or part's advance is its glyph's box, as the metrics tables read it. The italic
//! correction of an assembly is that of its last part, which then leaves the glyph italic
//! correction list, as ufo2ft builds it.
//!
//! Ported from ufo2ft (MIT): `outlineCompiler.py`, `BaseOutlineCompiler.setupTable_MATH`; and
//! from fontTools (MIT): `otlLib/builder.py`, `buildMathTable` and its helpers.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fontcraft_model::plist::{Array, Dict, Number, Str, Value};
use fontcraft_model::{Font, Resolved, ResolvedGlyph};
use write_fonts::tables::glyf::Bbox;
use write_fonts::tables::layout::builders::CoverageTableBuilder;
use write_fonts::tables::math::{
    GlyphAssembly, GlyphPartRecord, Math, MathConstants, MathGlyphConstruction, MathGlyphInfo, MathGlyphVariantRecord, MathItalicsCorrectionInfo,
    MathKern, MathKernInfo, MathKernInfoRecord, MathTopAccentAttachment, MathValueRecord, MathVariants, PartFlags,
};
use write_fonts::types::{FWord, GlyphId16, UfWord};

use crate::export_params::ExportEdits;
use crate::{CompileError, i16_of, u16_of};

/// The prefix of every math key.
pub const MATH_PREFIX: &str = "com.nagwa.MATHPlugin.";
/// The master's (font lib's) dictionary of constants.
pub const CONSTANTS_KEY: &str = "com.nagwa.MATHPlugin.constants";
/// A glyph's variants and assemblies.
pub const VARIANTS_KEY: &str = "com.nagwa.MATHPlugin.variants";
/// Marks a glyph as an extended shape; in a UFO's font lib, the list of such glyphs.
pub const EXTENDED_SHAPE_KEY: &str = "com.nagwa.MATHPlugin.extendedShape";
/// The constant that goes into the variants table instead of the constants table.
pub const MIN_CONNECTOR_OVERLAP: &str = "MinConnectorOverlap";
/// The italic correction anchor.
pub const ITALIC_ANCHOR: &str = "math.ic";
/// The top accent attachment anchor.
pub const TOP_ACCENT_ANCHOR: &str = "math.ta";
/// The math kerning anchors' prefixes and the corners they kern.
pub const KERN_SIDES: [(&str, KernSide); 4] =
    [("math.tr", KernSide::TopRight), ("math.tl", KernSide::TopLeft), ("math.br", KernSide::BottomRight), ("math.bl", KernSide::BottomLeft)];

/// Most variants of one glyph in one direction.
pub const MAX_VARIANTS: usize = 256;
/// Most parts of one assembly.
pub const MAX_PARTS: usize = 256;
/// Most kerning points of one corner of a glyph.
pub const MAX_KERN_POINTS: usize = 64;
/// Largest magnitude of any math value; the table stores 16-bit values.
pub const MAX_MATH_VALUE: f64 = 32_767.0;

/// The constants in table order, by the specification's names. The first two and the last are
/// percentages, the third and fourth unsigned heights, the rest value records.
pub const CONSTANT_NAMES: [&str; 56] = [
    "ScriptPercentScaleDown",
    "ScriptScriptPercentScaleDown",
    "DelimitedSubFormulaMinHeight",
    "DisplayOperatorMinHeight",
    "MathLeading",
    "AxisHeight",
    "AccentBaseHeight",
    "FlattenedAccentBaseHeight",
    "SubscriptShiftDown",
    "SubscriptTopMax",
    "SubscriptBaselineDropMin",
    "SuperscriptShiftUp",
    "SuperscriptShiftUpCramped",
    "SuperscriptBottomMin",
    "SuperscriptBaselineDropMax",
    "SubSuperscriptGapMin",
    "SuperscriptBottomMaxWithSubscript",
    "SpaceAfterScript",
    "UpperLimitGapMin",
    "UpperLimitBaselineRiseMin",
    "LowerLimitGapMin",
    "LowerLimitBaselineDropMin",
    "StackTopShiftUp",
    "StackTopDisplayStyleShiftUp",
    "StackBottomShiftDown",
    "StackBottomDisplayStyleShiftDown",
    "StackGapMin",
    "StackDisplayStyleGapMin",
    "StretchStackTopShiftUp",
    "StretchStackBottomShiftDown",
    "StretchStackGapAboveMin",
    "StretchStackGapBelowMin",
    "FractionNumeratorShiftUp",
    "FractionNumeratorDisplayStyleShiftUp",
    "FractionDenominatorShiftDown",
    "FractionDenominatorDisplayStyleShiftDown",
    "FractionNumeratorGapMin",
    "FractionNumDisplayStyleGapMin",
    "FractionRuleThickness",
    "FractionDenominatorGapMin",
    "FractionDenomDisplayStyleGapMin",
    "SkewedFractionHorizontalGap",
    "SkewedFractionVerticalGap",
    "OverbarVerticalGap",
    "OverbarRuleThickness",
    "OverbarExtraAscender",
    "UnderbarVerticalGap",
    "UnderbarRuleThickness",
    "UnderbarExtraDescender",
    "RadicalVerticalGap",
    "RadicalDisplayStyleVerticalGap",
    "RadicalRuleThickness",
    "RadicalExtraAscender",
    "RadicalKernBeforeDegree",
    "RadicalKernAfterDegree",
    "RadicalDegreeBottomRaisePercent",
];

/// Whether `name` is a constant the source may store: one of [`CONSTANT_NAMES`] or
/// [`MIN_CONNECTOR_OVERLAP`].
pub fn is_constant(name: &str) -> bool {
    name == MIN_CONNECTOR_OVERLAP || CONSTANT_NAMES.contains(&name)
}

/// A corner of a glyph that math kerning applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KernSide {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl KernSide {
    /// The parameter name (`topRight`…).
    pub fn key(self) -> &'static str {
        match self {
            KernSide::TopRight => "topRight",
            KernSide::TopLeft => "topLeft",
            KernSide::BottomRight => "bottomRight",
            KernSide::BottomLeft => "bottomLeft",
        }
    }

    /// The anchor name prefix (`math.tr`…).
    pub fn prefix(self) -> &'static str {
        KERN_SIDES.iter().find(|(_, s)| *s == self).map_or("math.tr", |(p, _)| p)
    }

    fn is_right(self) -> bool {
        matches!(self, KernSide::TopRight | KernSide::BottomRight)
    }

    /// The kern value an anchor at `x` gives, for a glyph `width` wide.
    pub fn kern_from_anchor(self, x: f64, width: f64) -> f64 {
        if self.is_right() { x - width } else { -x }
    }

    /// The anchor x that gives the kern value `kern`, for a glyph `width` wide.
    pub fn anchor_from_kern(self, kern: f64, width: f64) -> f64 {
        if self.is_right() { kern + width } else { -kern }
    }
}

/// The side whose anchors `name` belongs to.
pub fn kern_side(name: &str) -> Option<KernSide> {
    KERN_SIDES.iter().find(|(prefix, _)| name.starts_with(prefix)).map(|(_, s)| *s)
}

/// One part of a glyph assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub glyph: String,
    /// 1 for an extender.
    pub flags: u16,
    pub start: f64,
    pub end: f64,
}

/// A glyph's size variants and assemblies, as [`VARIANTS_KEY`] stores them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphVariants {
    pub h_variants: Vec<String>,
    pub v_variants: Vec<String>,
    pub h_assembly: Vec<Part>,
    pub v_assembly: Vec<Part>,
}

/// A number from a property list: a number, or digits as text (the `.glyphs` format may quote
/// them). Only finite values within the table's range.
pub fn number(v: &Value) -> Option<f64> {
    let n = match v {
        Value::Number(n) => n.value().as_f64(),
        Value::String(s) => s.as_str().trim().parse().ok()?,
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => return None,
    };
    (n.is_finite() && n.abs() <= MAX_MATH_VALUE).then_some(n)
}

/// A number as the source stores it: an integer when it is whole.
pub fn number_value(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() <= MAX_MATH_VALUE {
        Value::Number(Number::int(v as i64))
    } else {
        Number::float(v).map_or(Value::Number(Number::int(0)), Value::Number)
    }
}

fn names(v: Option<&Value>) -> Vec<String> {
    let Some(items) = v.and_then(Value::as_array) else { return Vec::new() };
    items.items.iter().filter_map(Value::as_str).take(MAX_VARIANTS).map(str::to_string).collect()
}

fn parts(v: Option<&Value>) -> Option<Vec<Part>> {
    let Some(items) = v.and_then(Value::as_array) else { return Some(Vec::new()) };
    let mut out = Vec::new();
    for item in items.items.iter().take(MAX_PARTS) {
        let fields = &item.as_array()?.items;
        // ufo2ft refuses an assembly whose parts are not four values each.
        let [glyph, flags, start, end] = fields.as_slice() else { return None };
        let flags = number(flags).filter(|f| *f >= 0.0).map_or(0, |f| f as u16);
        out.push(Part { glyph: glyph.as_str()?.to_string(), flags, start: number(start)?, end: number(end)? });
    }
    Some(out)
}

impl GlyphVariants {
    pub fn is_empty(&self) -> bool {
        self.h_variants.is_empty() && self.v_variants.is_empty() && self.h_assembly.is_empty() && self.v_assembly.is_empty()
    }

    /// Reads the stored form. An assembly with a malformed part is left out, as ufo2ft refuses it.
    pub fn from_plist(v: &Value) -> Self {
        let Some(d) = v.as_dict() else { return Self::default() };
        Self {
            h_variants: names(d.get("hVariants")),
            v_variants: names(d.get("vVariants")),
            h_assembly: parts(d.get("hAssembly")).unwrap_or_default(),
            v_assembly: parts(d.get("vAssembly")).unwrap_or_default(),
        }
    }

    /// The stored form; only the keys with content.
    pub fn to_plist(&self) -> Value {
        let list = |items: Vec<Value>| Value::Array(Array { items, ..Default::default() });
        let name_list = |names: &[String]| list(names.iter().map(|n| Value::String(Str::new(n.as_str()))).collect());
        let part_list = |parts: &[Part]| {
            list(
                parts
                    .iter()
                    .map(|p| {
                        list(vec![
                            Value::String(Str::new(p.glyph.as_str())),
                            Value::Number(Number::int(i64::from(p.flags))),
                            number_value(p.start),
                            number_value(p.end),
                        ])
                    })
                    .collect(),
            )
        };
        let mut d = Dict::new();
        if !self.h_assembly.is_empty() {
            d.insert("hAssembly", part_list(&self.h_assembly));
        }
        if !self.h_variants.is_empty() {
            d.insert("hVariants", name_list(&self.h_variants));
        }
        if !self.v_assembly.is_empty() {
            d.insert("vAssembly", part_list(&self.v_assembly));
        }
        if !self.v_variants.is_empty() {
            d.insert("vVariants", name_list(&self.v_variants));
        }
        Value::Dict(d)
    }

    fn renamed(&self, edits: &ExportEdits) -> Self {
        let name = |n: &String| edits.exported_name(n).unwrap_or(n.as_str()).to_string();
        let part = |p: &Part| Part { glyph: name(&p.glyph), ..p.clone() };
        Self {
            h_variants: self.h_variants.iter().map(name).collect(),
            v_variants: self.v_variants.iter().map(name).collect(),
            h_assembly: self.h_assembly.iter().map(part).collect(),
            v_assembly: self.v_assembly.iter().map(part).collect(),
        }
    }
}

/// The constants a master stores, by name, in stored order; unknown names and values that are
/// not numbers are left out. `None` when the master has no constants key.
pub fn stored_constants(data: &fontcraft_model::UserData) -> Option<Vec<(String, f64)>> {
    let d = data.get(CONSTANTS_KEY)?.as_dict();
    Some(
        d.map(|d| {
            d.entries.iter().filter(|(k, _)| is_constant(k.as_str())).filter_map(|(k, v)| Some((k.as_str().to_string(), number(v)?))).collect()
        })
        .unwrap_or_default(),
    )
}

/// Whether the glyph is an extended shape: the key in its user data, or its name in the list a
/// UFO's font lib (the master's user data) keeps.
pub fn is_extended_shape(font: &Font, master: &str, glyph: &str) -> bool {
    let Some(g) = font.glyph(glyph) else { return false };
    g.info.user_data.get(EXTENDED_SHAPE_KEY).is_some()
        || g.layer(master).is_some_and(|l| l.info.user_data.get(EXTENDED_SHAPE_KEY).is_some())
        || font
            .master(master)
            .ok()
            .and_then(|m| m.user_data.get(EXTENDED_SHAPE_KEY))
            .and_then(Value::as_array)
            .is_some_and(|a| a.items.iter().take(fontcraft_model::MAX_GLYPHS).any(|v| v.as_str() == Some(glyph)))
}

/// A glyph's stored variants and assemblies: from its user data, else from its master layer's
/// (a UFO glyph lib).
pub fn stored_variants(font: &Font, master: &str, glyph: &str) -> GlyphVariants {
    let Some(g) = font.glyph(glyph) else { return GlyphVariants::default() };
    g.info
        .user_data
        .get(VARIANTS_KEY)
        .or_else(|| g.layer(master).and_then(|l| l.info.user_data.get(VARIANTS_KEY)))
        .map(GlyphVariants::from_plist)
        .unwrap_or_default()
}

/// What the source states for the `MATH` table, by exported glyph name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MathPlan {
    pub constants: BTreeMap<String, f64>,
    pub extended: BTreeSet<String>,
    /// In the source's glyph order.
    pub variants: Vec<(String, GlyphVariants)>,
}

/// The source's math data for the compiled master, or `None` when it has none: no constants key
/// on the master, no extended shape and no variants.
pub(crate) fn plan(font: Option<&Font>, resolved: &Resolved, edits: &ExportEdits) -> Option<MathPlan> {
    let font = font?;
    let master = resolved.master.as_str();
    let stored = font.master(master).ok().and_then(|m| stored_constants(&m.user_data));
    let mut plan = MathPlan { constants: stored.clone().unwrap_or_default().into_iter().collect(), ..Default::default() };
    for g in font.glyphs.iter() {
        let Some(exported) = edits.exported_name(&g.name) else { continue };
        if is_extended_shape(font, master, &g.name) {
            plan.extended.insert(exported.to_string());
        }
        let v = stored_variants(font, master, &g.name);
        if !v.is_empty() {
            plan.variants.push((exported.to_string(), v.renamed(edits)));
        }
    }
    (stored.is_some() || !plan.extended.is_empty() || !plan.variants.is_empty()).then_some(plan)
}

fn record(v: f64) -> MathValueRecord {
    MathValueRecord::new(FWord::new(i16_of(v)), None)
}

fn constants_table(c: &BTreeMap<String, f64>) -> MathConstants {
    let v = |name: &str| c.get(name).copied().unwrap_or(0.0);
    let r = |name: &str| record(v(name));
    MathConstants {
        script_percent_scale_down: i16_of(v("ScriptPercentScaleDown")),
        script_script_percent_scale_down: i16_of(v("ScriptScriptPercentScaleDown")),
        delimited_sub_formula_min_height: UfWord::new(u16_of(v("DelimitedSubFormulaMinHeight"))),
        display_operator_min_height: UfWord::new(u16_of(v("DisplayOperatorMinHeight"))),
        math_leading: r("MathLeading"),
        axis_height: r("AxisHeight"),
        accent_base_height: r("AccentBaseHeight"),
        flattened_accent_base_height: r("FlattenedAccentBaseHeight"),
        subscript_shift_down: r("SubscriptShiftDown"),
        subscript_top_max: r("SubscriptTopMax"),
        subscript_baseline_drop_min: r("SubscriptBaselineDropMin"),
        superscript_shift_up: r("SuperscriptShiftUp"),
        superscript_shift_up_cramped: r("SuperscriptShiftUpCramped"),
        superscript_bottom_min: r("SuperscriptBottomMin"),
        superscript_baseline_drop_max: r("SuperscriptBaselineDropMax"),
        sub_superscript_gap_min: r("SubSuperscriptGapMin"),
        superscript_bottom_max_with_subscript: r("SuperscriptBottomMaxWithSubscript"),
        space_after_script: r("SpaceAfterScript"),
        upper_limit_gap_min: r("UpperLimitGapMin"),
        upper_limit_baseline_rise_min: r("UpperLimitBaselineRiseMin"),
        lower_limit_gap_min: r("LowerLimitGapMin"),
        lower_limit_baseline_drop_min: r("LowerLimitBaselineDropMin"),
        stack_top_shift_up: r("StackTopShiftUp"),
        stack_top_display_style_shift_up: r("StackTopDisplayStyleShiftUp"),
        stack_bottom_shift_down: r("StackBottomShiftDown"),
        stack_bottom_display_style_shift_down: r("StackBottomDisplayStyleShiftDown"),
        stack_gap_min: r("StackGapMin"),
        stack_display_style_gap_min: r("StackDisplayStyleGapMin"),
        stretch_stack_top_shift_up: r("StretchStackTopShiftUp"),
        stretch_stack_bottom_shift_down: r("StretchStackBottomShiftDown"),
        stretch_stack_gap_above_min: r("StretchStackGapAboveMin"),
        stretch_stack_gap_below_min: r("StretchStackGapBelowMin"),
        fraction_numerator_shift_up: r("FractionNumeratorShiftUp"),
        fraction_numerator_display_style_shift_up: r("FractionNumeratorDisplayStyleShiftUp"),
        fraction_denominator_shift_down: r("FractionDenominatorShiftDown"),
        fraction_denominator_display_style_shift_down: r("FractionDenominatorDisplayStyleShiftDown"),
        fraction_numerator_gap_min: r("FractionNumeratorGapMin"),
        fraction_num_display_style_gap_min: r("FractionNumDisplayStyleGapMin"),
        fraction_rule_thickness: r("FractionRuleThickness"),
        fraction_denominator_gap_min: r("FractionDenominatorGapMin"),
        fraction_denom_display_style_gap_min: r("FractionDenomDisplayStyleGapMin"),
        skewed_fraction_horizontal_gap: r("SkewedFractionHorizontalGap"),
        skewed_fraction_vertical_gap: r("SkewedFractionVerticalGap"),
        overbar_vertical_gap: r("OverbarVerticalGap"),
        overbar_rule_thickness: r("OverbarRuleThickness"),
        overbar_extra_ascender: r("OverbarExtraAscender"),
        underbar_vertical_gap: r("UnderbarVerticalGap"),
        underbar_rule_thickness: r("UnderbarRuleThickness"),
        underbar_extra_descender: r("UnderbarExtraDescender"),
        radical_vertical_gap: r("RadicalVerticalGap"),
        radical_display_style_vertical_gap: r("RadicalDisplayStyleVerticalGap"),
        radical_rule_thickness: r("RadicalRuleThickness"),
        radical_extra_ascender: r("RadicalExtraAscender"),
        radical_kern_before_degree: r("RadicalKernBeforeDegree"),
        radical_kern_after_degree: r("RadicalKernAfterDegree"),
        radical_degree_bottom_raise_percent: i16_of(v("RadicalDegreeBottomRaisePercent")),
    }
}

/// A coverage table over `glyphs` and the values in coverage order.
fn covered<T: Clone>(entries: &BTreeMap<u16, T>) -> (write_fonts::tables::layout::CoverageTable, Vec<T>) {
    let coverage = CoverageTableBuilder::from_glyphs(entries.keys().map(|g| GlyphId16::new(*g)).collect()).build();
    (coverage, entries.values().cloned().collect())
}

/// What one direction's constructions are built from: per glyph, its variants (glyph and
/// advance) and its assembly (parts with advances, and the italic correction).
type Construction = (Vec<(u16, f64)>, Option<(Vec<(u16, u16, f64, f64, f64)>, f64)>);

fn constructions(entries: &BTreeMap<u16, Construction>) -> (Option<write_fonts::tables::layout::CoverageTable>, Vec<MathGlyphConstruction>) {
    if entries.is_empty() {
        return (None, Vec::new());
    }
    let (coverage, values) = covered(entries);
    let built = values
        .into_iter()
        .map(|(variants, assembly)| {
            let records =
                variants.into_iter().map(|(gid, advance)| MathGlyphVariantRecord::new(GlyphId16::new(gid), UfWord::new(u16_of(advance)))).collect();
            let assembly = assembly.map(|(parts, ic)| {
                let parts = parts
                    .into_iter()
                    .map(|(gid, flags, start, end, advance)| {
                        GlyphPartRecord::new(
                            GlyphId16::new(gid),
                            UfWord::new(u16_of(start)),
                            UfWord::new(u16_of(end)),
                            UfWord::new(u16_of(advance)),
                            PartFlags::from_bits_truncate(flags),
                        )
                    })
                    .collect();
                GlyphAssembly::new(record(ic), parts)
            });
            MathGlyphConstruction::new(assembly, records)
        })
        .collect();
    (Some(coverage), built)
}

/// The `MATH` table for the final glyph `order`, whose integer boxes (as `hmtx` reads them) are
/// `boxes`. A glyph the data names that is not in the font is left out with a warning.
pub(crate) fn build(plan: &MathPlan, order: &[ResolvedGlyph], boxes: &[Option<Bbox>], warnings: &mut Vec<String>) -> Result<Vec<u8>, CompileError> {
    let gids: HashMap<&str, u16> = order.iter().enumerate().filter_map(|(i, g)| Some((g.name.as_str(), u16::try_from(i).ok()?))).collect();
    let mut missing: BTreeSet<String> = BTreeSet::new();
    let mut gid = |name: &str| {
        let found = gids.get(name).copied();
        if found.is_none() {
            missing.insert(name.to_string());
        }
        found
    };
    let size = |g: u16, horizontal: bool| {
        boxes
            .get(usize::from(g))
            .copied()
            .flatten()
            .map_or(0.0, |b| if horizontal { f64::from(b.x_max) - f64::from(b.x_min) } else { f64::from(b.y_max) - f64::from(b.y_min) })
    };

    // Glyph info from the anchors, by glyph id.
    let mut italics: BTreeMap<u16, f64> = BTreeMap::new();
    let mut accents: BTreeMap<u16, f64> = BTreeMap::new();
    let mut kerns: BTreeMap<u16, BTreeMap<KernSide, Vec<(f64, f64)>>> = BTreeMap::new();
    for (i, g) in order.iter().enumerate() {
        let Ok(id) = u16::try_from(i) else { break };
        for (name, at) in &g.anchors {
            if name == ITALIC_ANCHOR {
                italics.insert(id, at.x - g.width);
            }
            if name == TOP_ACCENT_ANCHOR {
                accents.insert(id, at.x);
            }
            if let Some(side) = kern_side(name) {
                let points = kerns.entry(id).or_default().entry(side).or_default();
                if points.len() < MAX_KERN_POINTS {
                    points.push((side.kern_from_anchor(at.x, g.width), at.y));
                }
            }
        }
    }

    let mut vertical: BTreeMap<u16, Construction> = BTreeMap::new();
    let mut horizontal: BTreeMap<u16, Construction> = BTreeMap::new();
    let mut extended: BTreeSet<u16> = BTreeSet::new();
    for name in &plan.extended {
        if let Some(g) = gid(name) {
            extended.insert(g);
        }
    }
    for (name, v) in &plan.variants {
        let Some(base) = gid(name) else { continue };
        for (is_h, variants, assembly) in [(true, &v.h_variants, &v.h_assembly), (false, &v.v_variants, &v.v_assembly)] {
            let mut list: Vec<(u16, f64)> = Vec::new();
            for n in variants.iter().take(MAX_VARIANTS) {
                if let Some(g) = gid(n) {
                    list.push((g, size(g, is_h)));
                }
            }
            let mut parts = Vec::new();
            for p in assembly.iter().take(MAX_PARTS) {
                if let Some(g) = gid(&p.glyph) {
                    parts.push((g, p.flags, p.start, p.end, size(g, is_h)));
                }
            }
            let built = if parts.is_empty() {
                None
            } else {
                let last = parts.last().map(|p| p.0);
                Some((parts, last.and_then(|g| italics.remove(&g)).unwrap_or(0.0)))
            };
            if list.is_empty() && built.is_none() {
                continue;
            }
            // The vertical variants of an extended shape are extended shapes too.
            if !is_h && plan.extended.contains(name) {
                extended.extend(list.iter().map(|(g, _)| *g));
            }
            let target = if is_h { &mut horizontal } else { &mut vertical };
            target.insert(base, (list, built));
        }
    }
    if !missing.is_empty() {
        let shown: Vec<&str> = missing.iter().take(10).map(String::as_str).collect();
        warnings.push(format!("the math data names glyphs that are not in the font, which were left out of MATH: {}", shown.join(", ")));
    }

    let italic_info = (!italics.is_empty()).then(|| {
        let (c, v) = covered(&italics);
        MathItalicsCorrectionInfo::new(c, v.into_iter().map(record).collect())
    });
    let accent_info = (!accents.is_empty()).then(|| {
        let (c, v) = covered(&accents);
        MathTopAccentAttachment::new(c, v.into_iter().map(record).collect())
    });
    let extended_coverage =
        (!extended.is_empty()).then(|| CoverageTableBuilder::from_glyphs(extended.iter().map(|g| GlyphId16::new(*g)).collect()).build());
    let kern_info = (!kerns.is_empty()).then(|| {
        let (c, sides) = covered(&kerns);
        let records = sides
            .into_iter()
            .map(|by_side| {
                let kern = |side: KernSide| {
                    let mut points = by_side.get(&side)?.clone();
                    points.sort_by(|a, b| a.1.total_cmp(&b.1));
                    // Every height but the last bounds a kern value; the last value applies above.
                    let heights = points.iter().take(points.len().saturating_sub(1)).map(|p| record(p.1)).collect();
                    Some(MathKern::new(heights, points.iter().map(|p| record(p.0)).collect()))
                };
                MathKernInfoRecord::new(kern(KernSide::TopRight), kern(KernSide::TopLeft), kern(KernSide::BottomRight), kern(KernSide::BottomLeft))
            })
            .collect();
        MathKernInfo::new(c, records)
    });
    let (v_coverage, v_constructions) = constructions(&vertical);
    let (h_coverage, h_constructions) = constructions(&horizontal);
    let overlap = plan.constants.get(MIN_CONNECTOR_OVERLAP).copied().unwrap_or(0.0);
    let math = Math::new(
        constants_table(&plan.constants),
        MathGlyphInfo::new(italic_info, accent_info, extended_coverage, kern_info),
        MathVariants::new(UfWord::new(u16_of(overlap)), v_coverage, h_coverage, v_constructions, h_constructions),
    );
    write_fonts::dump_table(&math).map_err(|e| CompileError::Table { table: "MATH", message: e.to_string() })
}

#[cfg(test)]
#[path = "math_tests.rs"]
mod tests;

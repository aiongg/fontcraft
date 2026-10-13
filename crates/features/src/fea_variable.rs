//! The reference's variable feature syntax lowered to plain feature-file text (M9.11).
//!
//! Rows: `featureCode.ifdefVariable`, `featureCode.ifndefVariable`,
//! `featureCode.ifdefOtherSymbols`, `featureCode.condition`, `featureCode.conditionRange`,
//! `featureCode.conditionLowerInclusive`, `featureCode.conditionMultipleAxes`,
//! `featureCode.conditionScope`, `featureCode.conditionLowering`, `featureCode.variablePos`,
//! `featureCode.variablePosMultiple`, `featureCode.variablePosFourValue`,
//! `featureCode.variablePosCoordinates`, `featureCode.variablePosLowering`,
//! `featureCode.mm.staticVersusVariable`, `switching.conditionalSubstitution`.
//!
//! Ported from glyphsLib 6.15.0 `builder/variable_features.py` (Apache-2.0, see `NOTICE`): the
//! value-record, anchor and scalar forms it recognises, its scope rules for `condition`, the
//! user-coordinate mapping, the overlay order and the text it writes. The overlay itself is
//! `fontcraft_interp::overlay`, a port of fontTools' `overlayFeatureVariations`.
//!
//! Contract:
//! - Runs after [`crate::lower::lower`], which keeps or drops the `#ifdef VARIABLE` and
//!   `#ifndef VARIABLE` blocks by target and refuses other symbols, `#else` and nesting.
//! - [`LowerTarget::Variable`]: a value followed by `(tag:coord …) value` alternatives becomes
//!   a variable scalar `(tag=default…:v tag=coord:v …)` in external (user) coordinates, each
//!   value rounded to a whole number; in a value record or anchor, one scalar per component
//!   (a component equal at every location stays a plain number). A feature block holding
//!   `condition` statements becomes the block with its unconditional rules, then a
//!   `conditionset` and `variation` blocks per region, overlapping regions split into
//!   non-overlapping ones, most specific first. Condition limits are internal coordinates
//!   mapped to external ones; a missing limit is the axis's end; an inverted range is dropped
//!   with a warning. Condition sets are numbered across every call on one [`VariableSyntax`].
//! - [`LowerTarget::Static`]: each value is the variable scalar evaluated at the location;
//!   each condition keeps or drops its rules by whether the location satisfies it (lower limit
//!   inclusive, upper limit strict, as the handbook describes).
//! - Text the lowering does not touch is copied unchanged; [`Lowered::map`] points the output
//!   back into the input.
//! - Refusals (a condition inside a lookup or outside a feature block, an unknown or repeated
//!   axis, a malformed range or value, the limits below) come back with their byte range and a
//!   message; nothing panics.
//!
//! Differences from glyphsLib: a number that ends a glyph name (`a1 (wght:700) 5`) is not read
//! as a value; condition sets are keyed by their numbers, not by their text.

use std::collections::BTreeSet;
use std::ops::Range;

use fontcraft_geom::variation::{VariationModel, normalize_value};
use fontcraft_interp::overlay::{AxisSpan, OverlayRule, RegionBox, overlay};
use fontcraft_model::{AxisRange, Font};

use crate::source_map::OffsetMap;

/// Most condition sets one [`VariableSyntax`] may define.
pub const MAX_CONDITION_SETS: usize = 4_096;
/// Most alternatives one variable value may list.
pub const MAX_VALUE_ALTERNATIVES: usize = 64;
/// Most axes one condition or variable value may name.
pub const MAX_CONDITION_AXES: usize = fontcraft_model::MAX_AXIS_VALUES;
/// Most `condition` statements one feature block may hold.
pub const MAX_CONDITIONS: usize = 64;
/// Longest feature text lowered, and longest output, in bytes.
pub const MAX_CODE_BYTES: usize = 16 * 1024 * 1024;
/// Deepest brace nesting read.
pub const MAX_NESTING_DEPTH: usize = fontcraft_fea::MAX_NESTING_DEPTH;
/// Most refusals reported for one text.
pub const MAX_ERRORS: usize = 100;
/// Largest value or coordinate read, in either direction.
pub const MAX_NUMBER: f64 = 1e9;

/// An axis as the feature code sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureAxis {
    pub tag: String,
    pub name: String,
    /// Minimum, origin and maximum in internal coordinates.
    pub internal: AxisRange,
    /// Minimum, origin and maximum in external coordinates.
    pub external: AxisRange,
    /// External to internal map, `(external, internal)` sorted by external value; empty for
    /// identity.
    pub map: Vec<(f64, f64)>,
}

impl FeatureAxis {
    /// The external coordinate of internal coordinate `v`: the map read backwards, a later
    /// point winning where two share an internal value.
    fn to_external(&self, v: f64) -> f64 {
        if self.map.is_empty() {
            return v;
        }
        fontcraft_model::axes::Mapping::new(self.map.clone(), self.external.default).to_user(v)
    }
}

/// A master as the feature code sees it: its location and number values.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureMaster {
    pub name: String,
    /// Internal coordinates, parallel to [`FeatureAxes::axes`].
    pub location: Vec<f64>,
    /// The master's number values by number name.
    pub numbers: Vec<(String, f64)>,
}

/// The designspace the lowering works in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatureAxes {
    pub axes: Vec<FeatureAxis>,
    pub masters: Vec<FeatureMaster>,
}

impl FeatureAxes {
    /// The axes and masters of `font`, with the axis maps and ranges the designspace export
    /// derives (`fontcraft_model::axes::mapping`).
    pub fn from_font(font: &Font) -> FeatureAxes {
        let axes = font
            .axes
            .iter()
            .enumerate()
            .take(MAX_CONDITION_AXES)
            .map(|(i, axis)| {
                let mapping = fontcraft_model::axes::mapping(font, i);
                let external = fontcraft_model::axes::range(font, i);
                let internal = AxisRange {
                    minimum: mapping.to_design(external.minimum),
                    default: mapping.to_design(external.default),
                    maximum: mapping.to_design(external.maximum),
                };
                let map = if mapping.is_identity() { Vec::new() } else { mapping.into_pairs() };
                FeatureAxis { tag: axis.tag.chars().take(4).collect(), name: axis.name.clone(), internal, external, map }
            })
            .collect::<Vec<_>>();
        let masters = font
            .masters
            .iter()
            .map(|m| FeatureMaster {
                name: m.name.clone(),
                location: (0..axes.len()).map(|i| fontcraft_model::axes::design_value(&m.location, i)).collect(),
                numbers: font.numbers.iter().zip(m.number_values.iter()).map(|(n, v)| (n.name.clone(), *v)).collect(),
            })
            .collect();
        FeatureAxes { axes, masters }
    }

    fn axis(&self, tag: &str) -> Option<(usize, &FeatureAxis)> {
        self.axes.iter().enumerate().find(|(_, a)| a.tag == tag)
    }
}

/// What the code is lowered for.
#[derive(Clone, Debug, PartialEq)]
pub enum LowerTarget {
    /// A static font at this internal location (parallel to [`FeatureAxes::axes`]; a missing
    /// value is the axis origin).
    Static(Vec<f64>),
    /// A variable font.
    Variable,
}

/// Lowered feature text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lowered {
    pub text: String,
    /// From the output back to the input; the identity when the text is unchanged.
    pub map: OffsetMap,
    /// Rules dropped because they can never apply (an inverted condition range).
    pub warnings: Vec<VariableSyntaxError>,
}

/// Why a stretch of feature text could not be lowered.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct VariableSyntaxError {
    /// Byte offset in the input.
    pub offset: usize,
    /// Length in bytes of the stretch the message is about.
    pub len: usize,
    pub message: String,
}

fn refusal(range: Range<usize>, message: impl Into<String>) -> VariableSyntaxError {
    VariableSyntaxError { offset: range.start, len: range.end.saturating_sub(range.start), message: message.into() }
}

/// `code` with the variable syntax lowered for `target`; condition sets are numbered from 1.
pub fn lower_variable_syntax(code: &str, axes: &FeatureAxes, target: &LowerTarget) -> Result<Lowered, Vec<VariableSyntaxError>> {
    VariableSyntax::new(axes.clone(), target.clone()).lower(code)
}

/// The lowering for one assembly: the condition sets defined so far are shared by every text
/// lowered with it, so each is defined once and the names do not clash.
#[derive(Clone, Debug)]
pub struct VariableSyntax {
    axes: FeatureAxes,
    target: LowerTarget,
    /// The `(tag, min, max)` conditions of each set defined so far, sorted by tag, and its name.
    condition_sets: Vec<(ConditionKey, String)>,
}

/// A condition set's `(tag, min, max)` ranges in external coordinates, sorted by tag.
type ConditionKey = Vec<(String, f64, f64)>;
/// A location as `(axis index, internal coordinate)` pairs and the values stated there.
type Alternative = (Vec<(usize, f64)>, Vec<f64>);
/// A stretch of a feature body: its condition (`None`: unconditional), the statement that set
/// it, and the stretch.
type Segment = (Option<Vec<Limit>>, Range<usize>, Range<usize>);

impl VariableSyntax {
    pub fn new(axes: FeatureAxes, target: LowerTarget) -> Self {
        Self { axes, target, condition_sets: Vec::new() }
    }

    /// `code` lowered; see the module contract.
    pub fn lower(&mut self, code: &str) -> Result<Lowered, Vec<VariableSyntaxError>> {
        if code.len() > MAX_CODE_BYTES {
            return Err(vec![refusal(0..0, format!("the feature code is larger than {} MB", MAX_CODE_BYTES >> 20))]);
        }
        let masked = mask(code);
        if !has_variable_syntax(&masked) {
            return Ok(Lowered { text: code.to_string(), map: OffsetMap::identity(), warnings: Vec::new() });
        }
        let (values_text, values_map) = self.lower_values(code)?;
        let mut warnings = Vec::new();
        let (text, conditions_map) = self
            .lower_conditions(&values_text, &mut warnings)
            .map_err(|errors| errors.into_iter().map(|e| remap(e, &values_map)).collect::<Vec<_>>())?;
        let warnings = warnings.into_iter().map(|w| remap(w, &values_map)).collect();
        Ok(Lowered { text, map: OffsetMap::compose(&values_map, &conditions_map), warnings })
    }

    // ── Values ──────────────────────────────────────────────────────────────────────────────

    /// The values pass: value records, anchors and scalars with `(tag:coord)` alternatives.
    fn lower_values(&self, code: &str) -> Result<(String, OffsetMap), Vec<VariableSyntaxError>> {
        let b = code.as_bytes();
        let mut out = Out::default();
        let mut errors = Vec::new();
        let mut copied = 0;
        let mut i = 0;
        while let Some(&c) = b.get(i) {
            let replaced = match c {
                b'#' => {
                    i = line_end(b, i);
                    continue;
                }
                b'"' => {
                    i = string_end(b, i).unwrap_or(i.saturating_add(1));
                    continue;
                }
                b'<' => value_record_end(b, i).map(|end| (end, self.value_record(code, i, end))),
                b'-' | b'0'..=b'9' if !i.checked_sub(1).and_then(|p| b.get(p)).is_some_and(|p| is_name_byte(*p)) => {
                    let Some(number) = number_end(b, i) else {
                        i = i.saturating_add(1);
                        continue;
                    };
                    match scalar_end(b, number) {
                        Some(end) => Some((end, self.scalar(code, i, end))),
                        None => {
                            i = number;
                            continue;
                        }
                    }
                }
                _ => None,
            };
            match replaced {
                Some((end, Ok(Some(text)))) => {
                    out.copy(code, copied..i);
                    out.write(&text, i..end);
                    copied = end;
                    i = end;
                }
                Some((end, Ok(None))) => i = end,
                Some((end, Err(e))) => {
                    if errors.len() < MAX_ERRORS {
                        errors.push(e);
                    }
                    i = end;
                }
                None => i = i.saturating_add(1),
            }
            if out.text.len() > MAX_CODE_BYTES {
                return Err(vec![too_long()]);
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        if copied == 0 && out.text.is_empty() {
            return Ok((code.to_string(), OffsetMap::identity()));
        }
        out.copy(code, copied..code.len());
        Ok((out.text, out.map))
    }

    /// The lowered form of the value record `code[start..end]` (`<` to `>`), or `None` to keep it.
    fn value_record(&self, code: &str, start: usize, end: usize) -> Result<Option<String>, VariableSyntaxError> {
        let range = start..end;
        let record = code.get(start.saturating_add(1)..end.saturating_sub(1)).unwrap_or("").trim();
        if let Some(rest) = record.strip_prefix("anchor") {
            if !record.contains('(') || record.contains("NULL") || record.contains("contourpoint") || has_standard_location(record) {
                return Ok(None);
            }
            let scalars = self.scalars(rest, 2, record, "anchor", &range)?;
            return Ok(Some(format!("<anchor {}>", scalars.join(" "))));
        }
        if !record.contains('(') || has_keyword(record) || has_standard_location(record) {
            return Ok(None);
        }
        let scalars = self.scalars(record, 4, record, "value record", &range)?;
        Ok(Some(format!("<{}>", scalars.join(" "))))
    }

    /// The lowered form of the scalar `code[start..end]`: a default value, then alternatives.
    fn scalar(&self, code: &str, start: usize, end: usize) -> Result<Option<String>, VariableSyntaxError> {
        let range = start..end;
        let text = code.get(range.clone()).unwrap_or("");
        let invalid = || refusal(range.clone(), format!("invalid variable position value `{text}`"));
        let tokens = tokens(text);
        let Some((default, rest)) = tokens.split_first() else { return Ok(None) };
        if rest.len() > MAX_VALUE_ALTERNATIVES.saturating_mul(2) {
            return Err(refusal(range.clone(), format!("a variable value lists more than {MAX_VALUE_ALTERNATIVES} alternatives")));
        }
        let default = self.value(default, &range)?;
        let mut alternatives = Vec::new();
        for pair in rest.chunks(2) {
            let (Some(location), Some(value)) = (pair.first(), pair.get(1)) else { return Err(invalid()) };
            if !location.starts_with('(') || value.starts_with('(') {
                return Err(invalid());
            }
            alternatives.push((self.location(location, &range)?, vec![self.value(value, &range)?]));
        }
        Ok(Some(self.render(&[default], &alternatives, &range)?.join(" ")))
    }

    /// The scalars of a record whose tokens (`tokens(text)`) are `count` default values, then a
    /// location and `count` values per alternative.
    fn scalars(&self, text: &str, count: usize, record: &str, kind: &str, range: &Range<usize>) -> Result<Vec<String>, VariableSyntaxError> {
        let invalid = || refusal(range.clone(), format!("invalid variable {kind}: <{record}>"));
        let tokens = tokens(text);
        let group = count.saturating_add(1);
        if tokens.len() < count.saturating_mul(2).saturating_add(1) || !tokens.len().saturating_sub(count).is_multiple_of(group) {
            return Err(invalid());
        }
        if tokens.len() / group > MAX_VALUE_ALTERNATIVES {
            return Err(refusal(range.clone(), format!("a variable {kind} lists more than {MAX_VALUE_ALTERNATIVES} alternatives")));
        }
        let number = |t: &&str| if t.starts_with('(') { Err(invalid()) } else { self.value(t, range) };
        let defaults = tokens.iter().take(count).map(number).collect::<Result<Vec<_>, _>>()?;
        let mut alternatives = Vec::new();
        for chunk in tokens.get(count..).unwrap_or(&[]).chunks(group) {
            let Some((location, values)) = chunk.split_first() else { return Err(invalid()) };
            if !location.starts_with('(') {
                return Err(invalid());
            }
            let parsed = self.location(location, range)?;
            if parsed.is_empty() {
                return Err(refusal(range.clone(), format!("invalid axis location {location} in {kind}: <{record}>")));
            }
            alternatives.push((parsed, values.iter().map(number).collect::<Result<Vec<_>, _>>()?));
        }
        self.render(&defaults, &alternatives, range)
    }

    /// One output value per component: a scalar for the variable target (a plain number where
    /// every location agrees), the value at the location for a static target.
    fn render(&self, defaults: &[f64], alternatives: &[Alternative], range: &Range<usize>) -> Result<Vec<String>, VariableSyntaxError> {
        match &self.target {
            LowerTarget::Variable => {
                let default_location =
                    self.axes.axes.iter().map(|a| format!("{}={}", a.tag, number_text(a.external.default))).collect::<Vec<_>>().join(",");
                Ok(defaults
                    .iter()
                    .enumerate()
                    .map(|(i, d)| {
                        let value = |vals: &Vec<f64>| vals.get(i).copied().unwrap_or(*d);
                        if alternatives.iter().all(|(_, vals)| value(vals) == *d) {
                            return integer_text(*d);
                        }
                        let mut entries = vec![format!("{default_location}:{}", integer_text(*d))];
                        for (location, vals) in alternatives {
                            entries.push(format!("{}:{}", self.external_location(location), integer_text(value(vals))));
                        }
                        format!("({})", entries.join(" "))
                    })
                    .collect())
            }
            LowerTarget::Static(at) => {
                let normalized = |location: &[(usize, f64)]| -> Vec<f64> {
                    self.axes
                        .axes
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            let v = location.iter().find(|(axis, _)| *axis == i).map_or(a.internal.default, |(_, v)| *v);
                            normalize_value(v, a.internal.minimum, a.internal.default, a.internal.maximum, false).unwrap_or(0.0)
                        })
                        .collect()
                };
                let mut locations = vec![normalized(&[])];
                let mut masters = vec![defaults.to_vec()];
                for (location, vals) in alternatives {
                    locations.push(normalized(location));
                    masters.push(vals.clone());
                }
                let here: Vec<(usize, f64)> = at.iter().copied().enumerate().collect();
                let fail =
                    |e: fontcraft_geom::variation::VariationError| refusal(range.clone(), format!("the variable value cannot be evaluated: {e}"));
                let model = VariationModel::new(&locations, false).map_err(fail)?;
                let values = model.interpolate(&normalized(&here), &masters).map_err(fail)?;
                Ok(values.iter().map(|v| integer_text(*v)).collect())
            }
        }
    }

    /// A rounded value token.
    fn value(&self, token: &str, range: &Range<usize>) -> Result<f64, VariableSyntaxError> {
        token
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && v.abs() <= MAX_NUMBER)
            .map(ot_round)
            .ok_or_else(|| refusal(range.clone(), format!("the value {token} is out of range")))
    }

    /// The `(axis index, internal coordinate)` pairs of a `(tag:coord …)` token.
    fn location(&self, token: &str, range: &Range<usize>) -> Result<Vec<(usize, f64)>, VariableSyntaxError> {
        let b = token.as_bytes();
        let mut out: Vec<(usize, f64)> = Vec::new();
        let mut i = 0;
        while i < b.len() {
            let Some((tag, coord, end)) = axis_spec(b, i) else {
                i = i.saturating_add(1);
                continue;
            };
            i = end;
            let (index, _) = self.known_axis(tag, range)?;
            let coord = coord
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && v.abs() <= MAX_NUMBER)
                .ok_or_else(|| refusal(range.clone(), format!("the coordinate {coord} is out of range")))?;
            if out.iter().any(|(a, _)| *a == index) {
                return Err(refusal(range.clone(), format!("the location {token} gives axis `{tag}` twice")));
            }
            if out.len() >= MAX_CONDITION_AXES {
                return Err(refusal(range.clone(), format!("a location names more than {MAX_CONDITION_AXES} axes")));
            }
            out.push((index, coord));
        }
        Ok(out)
    }

    fn known_axis(&self, tag: &str, range: &Range<usize>) -> Result<(usize, &FeatureAxis), VariableSyntaxError> {
        self.axes.axis(tag).ok_or_else(|| {
            let known = self.axes.axes.iter().map(|a| format!("`{}`", a.tag)).collect::<Vec<_>>();
            let known = if known.is_empty() { "the font has no axes".to_string() } else { format!("expected one of {}", known.join(", ")) };
            refusal(range.clone(), format!("unknown axis `{tag}` in the feature code; {known}"))
        })
    }

    /// `tag=coord,…` in external coordinates.
    fn external_location(&self, location: &[(usize, f64)]) -> String {
        location
            .iter()
            .filter_map(|(i, v)| self.axes.axes.get(*i).map(|a| format!("{}={}", a.tag, number_text(user_coordinate(a, *v)))))
            .collect::<Vec<_>>()
            .join(",")
    }

    // ── Conditions ──────────────────────────────────────────────────────────────────────────

    /// The conditions pass: each feature block holding `condition` statements rewritten.
    fn lower_conditions(&mut self, code: &str, warnings: &mut Vec<VariableSyntaxError>) -> Result<(String, OffsetMap), Vec<VariableSyntaxError>> {
        let masked = mask(code);
        let m = masked.as_bytes();
        let mut out = Out::default();
        let mut errors = Vec::new();
        let mut copied = 0;
        let mut pos = 0;
        let mut outside: Vec<Range<usize>> = Vec::new();
        while let Some(start) = feature_start(m, pos) {
            let FeatureStart { keyword, tag, extension, body } = start;
            let close = match block_end(m, body) {
                Ok(Some(close)) => close,
                Ok(None) => break,
                Err(()) => {
                    errors.push(refusal(keyword..body, format!("the braces are nested more than {MAX_NESTING_DEPTH} deep")));
                    break;
                }
            };
            let tail = feature_tail(m, close.saturating_add(1), code.get(tag.clone()).unwrap_or(""));
            let body_conditions = conditions(m, body..close);
            match tail {
                Some(end) if !body_conditions.is_empty() => {
                    outside.push(copied..keyword);
                    out.copy(code, copied..keyword);
                    let tag_text = code.get(tag).unwrap_or("");
                    match self.feature(code, &masked, tag_text, extension, keyword..end, body..close, &body_conditions, &mut out, warnings) {
                        Ok(()) => {}
                        Err(e) => {
                            if errors.len() < MAX_ERRORS {
                                errors.push(e);
                            }
                        }
                    }
                    copied = end;
                    pos = end;
                }
                _ => pos = tail.unwrap_or(close.saturating_add(1)),
            }
            if out.text.len() > MAX_CODE_BYTES {
                return Err(vec![too_long()]);
            }
        }
        outside.push(copied..code.len());
        for range in outside {
            if let Some(c) = conditions(m, range).first()
                && errors.len() < MAX_ERRORS
            {
                errors.push(refusal(c.statement.clone(), "a `condition` statement must be inside a feature block (`feature tag { … } tag;`)"));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        if out.text.is_empty() && copied == 0 {
            return Ok((code.to_string(), OffsetMap::identity()));
        }
        out.copy(code, copied..code.len());
        Ok((out.text, out.map))
    }

    /// One feature block with conditions, written to `out`.
    #[allow(clippy::too_many_arguments)]
    fn feature(
        &mut self,
        code: &str,
        masked: &str,
        tag: &str,
        extension: bool,
        whole: Range<usize>,
        body: Range<usize>,
        found: &[Condition],
        out: &mut Out,
        warnings: &mut Vec<VariableSyntaxError>,
    ) -> Result<(), VariableSyntaxError> {
        if found.len() > MAX_CONDITIONS {
            return Err(refusal(whole, format!("feature `{tag}` holds more than {MAX_CONDITIONS} `condition` statements")));
        }
        let m = masked.as_bytes();
        let mut segments: Vec<Segment> = Vec::new();
        let mut last = body.start;
        let mut current: Option<(Vec<Limit>, Range<usize>)> = None;
        for c in found {
            let depth = depth_at(m, body.start, c.statement.start);
            if depth > 0 {
                let text = code.get(c.statement.clone()).unwrap_or("").trim();
                return Err(refusal(
                    c.statement.clone(),
                    format!("a `condition` statement cannot be inside a lookup block (feature `{tag}`): {text}"),
                ));
            }
            let (limits, statement) = current.take().map_or((None, whole.start..whole.start), |(l, s)| (Some(l), s));
            segments.push((limits, statement, last..c.statement.start));
            let params = masked.get(c.params.clone()).unwrap_or("");
            // A bare `condition;` gives `None`: the rules after it are unconditional again.
            current = self.limits(params, &c.params)?.map(|l| (l, c.statement.clone()));
            last = c.statement.end;
        }
        let (limits, statement) = current.map_or((None, whole.start..whole.start), |(l, s)| (Some(l), s));
        segments.push((limits, statement, last..body.end));

        let ext = if extension { " useExtension" } else { "" };
        let head = whole.start..body.start;
        let tail = body.end..whole.end;
        match self.target.clone() {
            LowerTarget::Static(at) => {
                let kept: Vec<Range<usize>> = segments
                    .into_iter()
                    .filter(|(limits, _, _)| {
                        limits.as_ref().is_none_or(|l| {
                            l.iter().all(|x| x.holds_at(at.get(x.axis).copied().or_else(|| self.axes.axes.get(x.axis).map(|a| a.internal.default))))
                        })
                    })
                    .map(|(_, _, r)| r)
                    .collect();
                out.write(&format!("feature {tag}{ext} {{\n"), head);
                out.copy_stripped(code, &kept);
                out.write(&format!("\n}} {tag};\n"), tail);
            }
            LowerTarget::Variable => {
                let mut base = Vec::new();
                let mut conditional: Vec<(Vec<Limit>, Range<usize>, Range<usize>)> = Vec::new();
                for (limits, statement, range) in segments {
                    match limits {
                        None => base.push(range),
                        Some(_) if code.get(range.clone()).unwrap_or("").trim().is_empty() => {}
                        Some(l) if l.iter().any(|x| x.user_min > x.user_max) => {
                            let ranges = l
                                .iter()
                                .map(|x| format!("{} < {} < {}", number_text(x.user_min), x.tag, number_text(x.user_max)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            warnings.push(refusal(
                                statement,
                                format!("the rules after condition `{ranges}` in feature `{tag}` are dropped: the condition can never match"),
                            ));
                        }
                        Some(l) => conditional.push((l, statement, range)),
                    }
                }
                out.write(&format!("feature {tag}{ext} {{\n"), head.clone());
                out.copy_stripped(code, &base);
                out.write(&format!("\n}} {tag};\n"), tail.clone());
                self.variations(code, tag, &conditional, &whole, out)?;
            }
        }
        Ok(())
    }

    /// The `conditionset` and `variation` blocks of a feature's conditional segments.
    fn variations(
        &mut self,
        code: &str,
        tag: &str,
        conditional: &[(Vec<Limit>, Range<usize>, Range<usize>)],
        whole: &Range<usize>,
        out: &mut Out,
    ) -> Result<(), VariableSyntaxError> {
        // fontTools merges rules with the same region first, keeping the last one's place.
        let mut rules: Vec<(RegionBox, Vec<usize>)> = Vec::new();
        for (index, (limits, _, _)) in conditional.iter().enumerate().rev() {
            let mut spans: Vec<AxisSpan> = limits.iter().map(|l| AxisSpan { axis: l.axis, min: l.user_min, max: l.user_max }).collect();
            spans.sort_by_key(|s| s.axis);
            spans.retain(|s| (s.min, s.max) != (-1.0, 1.0));
            let region = RegionBox(spans);
            match rules.iter_mut().find(|(r, _)| *r == region) {
                Some((_, indices)) => indices.push(index),
                None => rules.push((region, vec![index])),
            }
        }
        rules.reverse();
        let rules: Vec<OverlayRule<Vec<usize>>> = rules.into_iter().map(|(region, payload)| OverlayRule { boxes: vec![region], payload }).collect();
        let records = overlay(&rules, fontcraft_interp::MAX_OVERLAY_WORK)
            .map_err(|e| refusal(whole.clone(), format!("the conditions of feature `{tag}` cannot be combined: {e}")))?;

        // The overlay skips single-point overlaps; a region inside a broader one goes first.
        let mut ordered: Vec<(RegionBox, Vec<usize>)> = Vec::new();
        for record in records {
            let indices = record.payloads.into_iter().flatten().collect();
            let at = ordered.iter().position(|(other, _)| box_within(&record.region, other)).unwrap_or(ordered.len());
            ordered.insert(at, (record.region, indices));
        }
        for (i, (region, _)) in ordered.iter().enumerate() {
            let mut conditions: Vec<(String, f64, f64)> =
                region.0.iter().filter_map(|s| self.axes.axes.get(s.axis).map(|a| (a.tag.clone(), s.min, s.max))).collect();
            conditions.sort_by(|a, b| a.0.cmp(&b.0));
            let name = match self.condition_sets.iter().find(|(c, _)| *c == conditions) {
                Some((_, name)) => name.clone(),
                None => {
                    if self.condition_sets.len() >= MAX_CONDITION_SETS {
                        return Err(refusal(whole.clone(), format!("the feature code needs more than {MAX_CONDITION_SETS} condition sets")));
                    }
                    let name = format!("conditionset_{}", self.condition_sets.len().saturating_add(1));
                    let body = conditions
                        .iter()
                        .map(|(t, lo, hi)| format!("{t} {} {}", number_text(*lo), number_text(*hi)))
                        .collect::<Vec<_>>()
                        .join(";\n    ");
                    out.write(&format!("\nconditionset {name} {{\n    {body};\n}} {name};\n"), whole.clone());
                    self.condition_sets.push((conditions, name.clone()));
                    name
                }
            };
            let mut merged = BTreeSet::new();
            for (j, (other, indices)) in ordered.iter().enumerate() {
                if j == i || box_within(region, other) {
                    merged.extend(indices.iter().copied());
                }
            }
            for index in merged {
                let Some((_, statement, range)) = conditional.get(index) else { continue };
                out.write(&format!("\nvariation {tag} {name} {{\n"), statement.clone());
                out.copy_stripped(code, std::slice::from_ref(range));
                out.write(&format!("\n}} {tag};\n"), statement.clone());
                if out.text.len() > MAX_CODE_BYTES {
                    return Err(too_long());
                }
            }
        }
        Ok(())
    }

    /// The limits of a condition's parameters, or `None` for a bare `condition;`.
    fn limits(&self, params: &str, range: &Range<usize>) -> Result<Option<Vec<Limit>>, VariableSyntaxError> {
        if params.trim().is_empty() {
            return Ok(None);
        }
        let mut limits: Vec<Limit> = Vec::new();
        for part in params.split(',') {
            let part = part.trim();
            if limits.len() >= MAX_CONDITION_AXES {
                return Err(refusal(range.clone(), format!("a condition names more than {MAX_CONDITION_AXES} axes")));
            }
            let Some((min, tag, max)) = axis_range(part) else {
                return Err(refusal(range.clone(), format!("invalid condition axis range `{part}`: write `min < tag < max`, either limit optional")));
            };
            let (axis, found) = self.known_axis(tag, range)?;
            let parse = |v: &str| {
                v.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && v.abs() <= MAX_NUMBER)
                    .ok_or_else(|| refusal(range.clone(), format!("the limit {v} is out of range")))
            };
            let int_min = min.map(parse).transpose()?;
            let int_max = max.map(parse).transpose()?;
            if limits.iter().any(|l| l.axis == axis) {
                return Err(refusal(range.clone(), format!("the condition gives axis `{tag}` twice")));
            }
            limits.push(Limit {
                axis,
                tag: found.tag.clone(),
                user_min: int_min.map_or(found.external.minimum, |v| user_coordinate(found, v)),
                user_max: int_max.map_or(found.external.maximum, |v| user_coordinate(found, v)),
                int_min,
                int_max,
            });
        }
        Ok(Some(limits))
    }
}

/// One axis range of a condition.
#[derive(Clone, Debug)]
struct Limit {
    axis: usize,
    tag: String,
    /// The range in external coordinates, a missing limit being the axis's end.
    user_min: f64,
    user_max: f64,
    /// The limits as written (internal coordinates).
    int_min: Option<f64>,
    int_max: Option<f64>,
}

impl Limit {
    /// Whether internal coordinate `v` is inside: the lower limit inclusive, the upper strict.
    fn holds_at(&self, v: Option<f64>) -> bool {
        let Some(v) = v else { return false };
        self.int_min.is_none_or(|lo| lo <= v) && self.int_max.is_none_or(|hi| v < hi)
    }
}

/// A `condition …;` statement: the whole statement and its parameters, byte ranges.
#[derive(Clone, Debug)]
struct Condition {
    statement: Range<usize>,
    params: Range<usize>,
}

/// Where a `feature tag {` starts: the keyword, the tag, `useExtension`, the body's first byte.
struct FeatureStart {
    keyword: usize,
    tag: Range<usize>,
    extension: bool,
    body: usize,
}

fn too_long() -> VariableSyntaxError {
    refusal(0..0, format!("the lowered feature code would be larger than {} MB", MAX_CODE_BYTES >> 20))
}

fn remap(mut e: VariableSyntaxError, map: &OffsetMap) -> VariableSyntaxError {
    let range = map.source_range(e.offset..e.offset.saturating_add(e.len));
    e.offset = range.start;
    e.len = range.end.saturating_sub(range.start);
    e
}

// ── Output ──────────────────────────────────────────────────────────────────────────────────

/// Output text with its map back to the input.
#[derive(Default)]
struct Out {
    text: String,
    map: OffsetMap,
}

impl Out {
    fn copy(&mut self, code: &str, range: Range<usize>) {
        if let Some(s) = code.get(range.clone()).filter(|s| !s.is_empty()) {
            let start = self.text.len();
            self.text.push_str(s);
            self.map.push(start..self.text.len(), range);
        }
    }

    fn write(&mut self, s: &str, origin: Range<usize>) {
        let start = self.text.len();
        self.text.push_str(s);
        self.map.push(start..self.text.len(), origin);
    }

    /// The ranges joined, white space trimmed from both ends of the whole.
    fn copy_stripped(&mut self, code: &str, ranges: &[Range<usize>]) {
        let text = |r: &Range<usize>| code.get(r.clone()).unwrap_or("");
        let Some(first) = ranges.iter().position(|r| !text(r).trim().is_empty()) else { return };
        let last = ranges.iter().rposition(|r| !text(r).trim().is_empty()).unwrap_or(first);
        for (i, r) in ranges.iter().enumerate().take(last.saturating_add(1)).skip(first) {
            let s = text(r);
            let mut start = r.start;
            let mut end = r.end;
            if i == first {
                start = start.saturating_add(s.len().saturating_sub(s.trim_start().len()));
            }
            if i == last {
                end = end.saturating_sub(s.len().saturating_sub(s.trim_end().len()));
            }
            self.copy(code, start..end.max(start));
        }
    }
}

// ── Scanning ────────────────────────────────────────────────────────────────────────────────

/// `code` with comments and strings blanked: braces and keywords in them do not count. A
/// string runs to the next `"` on its line; without one the `"` is plain text.
fn mask(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut copied = 0;
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let end = match c {
            b'#' => line_end(b, i),
            b'"' => match string_end(b, i) {
                Some(end) => end,
                None => {
                    i = i.saturating_add(1);
                    continue;
                }
            },
            _ => {
                i = i.saturating_add(1);
                continue;
            }
        };
        out.push_str(code.get(copied..i).unwrap_or(""));
        // Blank byte for byte, so offsets stay the same.
        out.extend(std::iter::repeat_n(' ', end.saturating_sub(i)));
        copied = end;
        i = end;
    }
    out.push_str(code.get(copied..).unwrap_or(""));
    out
}

fn line_end(b: &[u8], from: usize) -> usize {
    b.get(from..).and_then(|rest| rest.iter().position(|c| *c == b'\n')).map_or(b.len(), |n| from.saturating_add(n))
}

/// The end (after the closing quote) of a string starting at `from`, closed on the same line.
fn string_end(b: &[u8], from: usize) -> Option<usize> {
    let rest = b.get(from.saturating_add(1)..)?;
    let n = rest.iter().position(|c| *c == b'"' || *c == b'\n')?;
    (rest.get(n) == Some(&b'"')).then_some(from.saturating_add(n).saturating_add(2))
}

/// Letters, digits, `_`, `.` and every non-ASCII byte: the characters of a name.
fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c >= 0x80
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(u8::is_ascii_whitespace) {
        i = i.saturating_add(1);
    }
    i
}

/// The end of a number `-?\d+(\.\d+)?` at `i`.
fn number_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = if b.get(i) == Some(&b'-') { i.saturating_add(1) } else { i };
    let digits = |mut k: usize| {
        while b.get(k).is_some_and(u8::is_ascii_digit) {
            k = k.saturating_add(1);
        }
        k
    };
    let k = digits(j);
    if k == j {
        return None;
    }
    j = k;
    if b.get(j) == Some(&b'.') && b.get(j.saturating_add(1)).is_some_and(u8::is_ascii_digit) {
        j = digits(j.saturating_add(1));
    }
    Some(j)
}

/// A four-character axis tag at `i`.
fn tag_end(b: &[u8], i: usize) -> Option<usize> {
    let end = i.checked_add(4)?;
    b.get(i..end)?.iter().all(u8::is_ascii_alphanumeric).then_some(end)
}

/// `tag : number` at `i`: the tag, the number and the end.
fn axis_spec(b: &[u8], i: usize) -> Option<(&str, &str, usize)> {
    let tag = tag_end(b, i)?;
    let colon = skip_ws(b, tag);
    if b.get(colon) != Some(&b':') {
        return None;
    }
    let start = skip_ws(b, colon.saturating_add(1));
    let end = number_end(b, start)?;
    let text = |r: Range<usize>| b.get(r).and_then(|s| std::str::from_utf8(s).ok());
    Some((text(i..tag)?, text(start..end)?, end))
}

/// The end of the alternatives `(tag:coord …) value …` after a number ending at `from`; `None`
/// when there is none.
fn scalar_end(b: &[u8], from: usize) -> Option<usize> {
    let mut end = from;
    let mut count = 0;
    while let Some(next) = alternative_end(b, end) {
        end = next;
        count += 1;
        if count > MAX_VALUE_ALTERNATIVES {
            break;
        }
    }
    (count > 0).then_some(end)
}

/// `\s*\(\s*spec(\s*,?\s*spec)*\s*\)\s*number` at `i`.
fn alternative_end(b: &[u8], i: usize) -> Option<usize> {
    let open = skip_ws(b, i);
    if b.get(open) != Some(&b'(') {
        return None;
    }
    let (_, _, mut j) = axis_spec(b, skip_ws(b, open.saturating_add(1)))?;
    loop {
        let mut k = skip_ws(b, j);
        if b.get(k) == Some(&b',') {
            k = skip_ws(b, k.saturating_add(1));
        }
        match axis_spec(b, k) {
            Some((_, _, end)) => j = end,
            None => break,
        }
    }
    let close = skip_ws(b, j);
    if b.get(close) != Some(&b')') {
        return None;
    }
    number_end(b, skip_ws(b, close.saturating_add(1)))
}

/// The end (after `>`) of a value record starting with `<` at `i`; one nested `<…>` allowed, no
/// `;` inside.
fn value_record_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i.saturating_add(1);
    loop {
        match *b.get(j)? {
            b'>' => return Some(j.saturating_add(1)),
            b';' => return None,
            b'<' => {
                let rest = b.get(j.saturating_add(1)..)?;
                let n = rest.iter().position(|c| matches!(c, b'<' | b'>' | b';'))?;
                if rest.get(n) != Some(&b'>') {
                    return None;
                }
                j = j.saturating_add(n).saturating_add(2);
            }
            _ => j = j.saturating_add(1),
        }
    }
}

/// The `(…)` groups and numbers of a value, in order.
fn tokens(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'(' {
            if let Some(n) = b.get(i..).and_then(|rest| rest.iter().position(|c| *c == b')')) {
                let end = i.saturating_add(n).saturating_add(1);
                out.extend(text.get(i..end));
                i = end;
                continue;
            }
        } else if let Some(end) = number_end(b, i) {
            out.extend(text.get(i..end));
            i = end;
            continue;
        }
        i = i.saturating_add(1);
    }
    out
}

/// Whether `record` has `device`, `contourpoint` or `NULL` as a word.
fn has_keyword(record: &str) -> bool {
    ["device", "contourpoint", "NULL"].iter().any(|word| {
        record.match_indices(word).any(|(at, w)| {
            let before = at.checked_sub(1).and_then(|p| record.as_bytes().get(p));
            let after = record.as_bytes().get(at.saturating_add(w.len()));
            let word_byte = |c: &u8| c.is_ascii_alphanumeric() || *c == b'_' || *c >= 0x80;
            !before.is_some_and(word_byte) && !after.is_some_and(word_byte)
        })
    })
}

/// Whether `record` already holds a standard variable value (`wght=400:10`).
fn has_standard_location(record: &str) -> bool {
    let b = record.as_bytes();
    (0..b.len()).any(|i| {
        let Some(tag) = tag_end(b, i) else { return false };
        let eq = skip_ws(b, tag);
        if b.get(eq) != Some(&b'=') {
            return false;
        }
        let Some(end) = number_end(b, skip_ws(b, eq.saturating_add(1))) else { return false };
        let colon = skip_ws(b, end);
        b.get(colon) == Some(&b':') && number_end(b, colon.saturating_add(1)).is_some()
    })
}

/// Whether the masked text holds anything this pass rewrites: a `condition` keyword or a
/// `(tag:` location.
fn has_variable_syntax(masked: &str) -> bool {
    let b = masked.as_bytes();
    !conditions(b, 0..b.len()).is_empty()
        || b.iter().enumerate().any(|(i, c)| {
            *c == b'(' && {
                let at = skip_ws(b, i.saturating_add(1));
                tag_end(b, at).is_some_and(|end| b.get(skip_ws(b, end)) == Some(&b':'))
            }
        })
}

/// The `condition …;` statements in `range` of the masked text: the word `condition` on its own
/// (not part of a name), up to the next `;`.
fn conditions(m: &[u8], range: Range<usize>) -> Vec<Condition> {
    const WORD: &[u8] = b"condition";
    let mut out = Vec::new();
    let mut i = range.start;
    while i.saturating_add(WORD.len()) <= range.end {
        let end = i.saturating_add(WORD.len());
        let is_word = m.get(i..end) == Some(WORD)
            && !i.checked_sub(1).and_then(|p| m.get(p)).is_some_and(|c| is_name_byte(*c))
            && !m.get(end).is_some_and(|c| is_name_byte(*c));
        if !is_word {
            i = i.saturating_add(1);
            continue;
        }
        let Some(semi) = m.get(end..range.end).and_then(|rest| rest.iter().position(|c| *c == b';')).map(|n| end.saturating_add(n)) else {
            break;
        };
        out.push(Condition { statement: i..semi.saturating_add(1), params: end..semi });
        i = semi.saturating_add(1);
    }
    out
}

/// The next `feature tag (useExtension)? {` at or after `from` in the masked text.
fn feature_start(m: &[u8], from: usize) -> Option<FeatureStart> {
    const WORD: &[u8] = b"feature";
    let mut i = from;
    while i < m.len() {
        let at = m.get(i..)?.windows(WORD.len()).position(|w| w == WORD)?.saturating_add(i);
        i = at.saturating_add(1);
        if at.checked_sub(1).and_then(|p| m.get(p)).is_some_and(|c| is_name_byte(*c)) {
            continue;
        }
        let after = at.saturating_add(WORD.len());
        let tag_start = skip_ws(m, after);
        if tag_start == after {
            continue;
        }
        let Some(tag_stop) = tag_end(m, tag_start) else { continue };
        let mut j = tag_stop;
        let mut extension = false;
        let k = skip_ws(m, j);
        if k > j && m.get(k..k.saturating_add(12)) == Some(b"useExtension".as_slice()) {
            j = k.saturating_add(12);
            extension = true;
        }
        let brace = skip_ws(m, j);
        if m.get(brace) == Some(&b'{') {
            return Some(FeatureStart { keyword: at, tag: tag_start..tag_stop, extension, body: brace.saturating_add(1) });
        }
    }
    None
}

/// The index of the `}` closing the block whose body starts at `from`; `Err` past
/// [`MAX_NESTING_DEPTH`].
fn block_end(m: &[u8], from: usize) -> Result<Option<usize>, ()> {
    let mut depth = 1usize;
    for (i, c) in m.iter().enumerate().skip(from) {
        match c {
            b'{' => {
                depth = depth.saturating_add(1);
                if depth > MAX_NESTING_DEPTH {
                    return Err(());
                }
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(Some(i));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

/// The end of `\s*tag\s*;` at `from`.
fn feature_tail(m: &[u8], from: usize, tag: &str) -> Option<usize> {
    let start = skip_ws(m, from);
    let end = start.checked_add(tag.len())?;
    if tag.is_empty() || m.get(start..end) != Some(tag.as_bytes()) {
        return None;
    }
    let semi = skip_ws(m, end);
    (m.get(semi) == Some(&b';')).then_some(semi.saturating_add(1))
}

/// Opening minus closing braces in `m[from..to]`.
fn depth_at(m: &[u8], from: usize, to: usize) -> i64 {
    m.get(from..to).unwrap_or(&[]).iter().fold(0i64, |d, c| match c {
        b'{' => d.saturating_add(1),
        b'}' => d.saturating_sub(1),
        _ => d,
    })
}

/// `[min <] tag [< max]`, the whole of `part`, at least one limit.
fn axis_range(part: &str) -> Option<(Option<&str>, &str, Option<&str>)> {
    let b = part.as_bytes();
    let mut i = 0;
    let mut min = None;
    if let Some(end) = number_end(b, 0) {
        let lt = skip_ws(b, end);
        if b.get(lt) == Some(&b'<') {
            min = part.get(0..end);
            i = skip_ws(b, lt.saturating_add(1));
        }
    }
    let tag_stop = tag_end(b, i)?;
    let tag = part.get(i..tag_stop)?;
    let mut max = None;
    let mut j = tag_stop;
    let lt = skip_ws(b, j);
    if b.get(lt) == Some(&b'<') {
        let start = skip_ws(b, lt.saturating_add(1));
        let end = number_end(b, start)?;
        max = part.get(start..end);
        j = end;
    }
    (j == b.len() && (min.is_some() || max.is_some())).then_some((min, tag, max))
}

/// Whether every axis range of `a` is inside `b`'s (an axis a box leaves out spans everything).
fn box_within(a: &RegionBox, b: &RegionBox) -> bool {
    let span = |r: &RegionBox, axis: usize| r.0.iter().find(|s| s.axis == axis).map_or((f64::NEG_INFINITY, f64::INFINITY), |s| (s.min, s.max));
    a.0.iter().chain(b.0.iter()).all(|s| {
        let (a_min, a_max) = span(a, s.axis);
        let (b_min, b_max) = span(b, s.axis);
        !(a_min < b_min || a_max > b_max)
    })
}

// ── Numbers ─────────────────────────────────────────────────────────────────────────────────

/// fontTools' `otRound`: halves round up.
fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// A whole number as text.
fn integer_text(v: f64) -> String {
    let v = ot_round(v);
    if v == 0.0 { "0".into() } else { format!("{v}") }
}

/// A number as its shortest decimal, without a trailing `.0`.
fn number_text(v: f64) -> String {
    if v == 0.0 { "0".into() } else { format!("{v}") }
}

/// The external coordinate of internal `v` on `axis`, at the precision of a 16.16 fixed-point
/// number and with the fewest decimals that keep it (fontTools' `floatToFixedToStr`).
fn user_coordinate(axis: &FeatureAxis, v: f64) -> f64 {
    let user = axis.to_external(v);
    fixed_shortest(user).parse::<f64>().unwrap_or(user)
}

/// fontTools' `nearestMultipleShortestRepr(value, 1 / 65536)`.
fn fixed_shortest(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".into();
    }
    let factor = 1.0 / 65536.0;
    let value = ot_round(value / factor) * factor;
    let eps = 0.5 * factor;
    let (lo, hi) = (value - eps, value + eps);
    if lo.trunc() != hi.trunc() {
        return format!("{}", value.round_ties_even());
    }
    let lo_s = format!("{lo:.8}");
    let hi_s = format!("{hi:.8}");
    let first_diff = lo_s.bytes().zip(hi_s.bytes()).position(|(a, b)| a != b).unwrap_or(lo_s.len());
    match lo_s.find('.') {
        Some(period) if first_diff > period && lo_s.len() == hi_s.len() => format!("{:.*}", first_diff.saturating_sub(period), value),
        _ => format!("{value}"),
    }
}

#[cfg(test)]
#[path = "fea_variable_tests.rs"]
mod tests;

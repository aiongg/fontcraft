//! Generated lookups as plain data: what FontCraft's own writers produce (the kerning of
//! `fontcraft_features::kern_feature`, the mark attachment of
//! `fontcraft_features::marks::GeneratedMarks`, M9.8's bracket-layer substitutions) and fea-rs
//! merges with the code. The shapes follow `GeneratedMarks`, with every position a [`Metric`] so
//! a variable font can carry deltas. Converted to fea-rs's builders inside this crate (M8.2).

use std::collections::BTreeMap;

use crate::{GlyphId16, Tag};

/// A value in font units: the default location's value, plus deltas over regions for a
/// variable font (empty for a static one). The deltas are what [`crate::FeaVariations::resolve`]
/// returns; M9.9 fills them for kerning and marks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metric {
    pub default: i16,
    /// At most [`crate::MAX_DELTAS`].
    pub deltas: Vec<(Region, i16)>,
}

impl Metric {
    /// A value without deltas.
    pub fn fixed(value: i16) -> Self {
        Self { default: value, deltas: Vec::new() }
    }

    pub fn is_variable(&self) -> bool {
        !self.deltas.is_empty()
    }
}

impl From<i16> for Metric {
    fn from(value: i16) -> Self {
        Self::fixed(value)
    }
}

/// Where a delta applies: one tent per axis, in normalised coordinates (-1 to 1). Axes not
/// listed do not limit the region. At most [`crate::MAX_DELTAS`] axes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region(pub Vec<RegionAxis>);

/// One axis of a [`Region`]: the delta grows from 0 at `start` to full at `peak` and falls to
/// 0 at `end`. `start <= peak <= end`, all finite and within -1 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionAxis {
    pub axis: Tag,
    pub start: f64,
    pub peak: f64,
    pub end: f64,
}

/// An anchor point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Anchor {
    pub x: Metric,
    pub y: Metric,
}

/// A positioning value record; `None` fields are not written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Value {
    pub x_placement: Option<Metric>,
    pub y_placement: Option<Metric>,
    pub x_advance: Option<Metric>,
    pub y_advance: Option<Metric>,
}

/// A glyph–glyph kerning rule. The value applies to the first glyph; the second gets none, as
/// the kern writers write it (right-to-left rules move the glyph with `x_placement` too).
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphPair {
    pub left: GlyphId16,
    pub right: GlyphId16,
    pub value: Value,
}

/// A class–class kerning rule; the value applies to the first glyph, as for [`GlyphPair`].
#[derive(Clone, Debug, PartialEq)]
pub struct ClassPair {
    pub left: Vec<GlyphId16>,
    pub right: Vec<GlyphId16>,
    pub value: Value,
}

/// The rules of one generated lookup.
#[derive(Clone, Debug, PartialEq)]
pub enum GeneratedRules {
    /// Pair adjustment: glyph pairs come before class pairs in the lookup, as the kern writer
    /// orders them.
    Pair { glyph_pairs: Vec<GlyphPair>, class_pairs: Vec<ClassPair> },
    /// Marks of the lookup's class and the anchor each base offers them.
    MarkToBase { marks: Vec<(GlyphId16, Anchor)>, bases: Vec<(GlyphId16, Anchor)> },
    /// Marks and, for each ligature, the anchor of each component (a component may have none).
    MarkToLigature { marks: Vec<(GlyphId16, Anchor)>, ligatures: Vec<(GlyphId16, Vec<Option<Anchor>>)> },
    /// Attaching marks and the marks they stack on.
    MarkToMark { marks: Vec<(GlyphId16, Anchor)>, bases: Vec<(GlyphId16, Anchor)> },
    /// Entry and exit anchors by glyph.
    Cursive { glyphs: Vec<(GlyphId16, Option<Anchor>, Option<Anchor>)> },
}

impl GeneratedRules {
    /// How many rules count towards [`crate::MAX_GENERATED_RULES`], saturating.
    pub fn rule_count(&self) -> usize {
        match self {
            GeneratedRules::Pair { glyph_pairs, class_pairs } => glyph_pairs.len().saturating_add(class_pairs.len()),
            GeneratedRules::MarkToBase { marks, bases } | GeneratedRules::MarkToMark { marks, bases } => marks.len().saturating_add(bases.len()),
            GeneratedRules::MarkToLigature { marks, ligatures } => marks.len().saturating_add(ligatures.len()),
            GeneratedRules::Cursive { glyphs } => glyphs.len(),
        }
    }
}

/// OpenType lookup flags: the low byte's bits, and the mark attachment class in the high byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LookupFlags(pub u16);

impl LookupFlags {
    pub const RIGHT_TO_LEFT: u16 = 0x0001;
    pub const IGNORE_BASE_GLYPHS: u16 = 0x0002;
    pub const IGNORE_LIGATURES: u16 = 0x0004;
    pub const IGNORE_MARKS: u16 = 0x0008;
    /// Set by the conversion when [`GeneratedLookup::filter_set`] is `Some`.
    pub const USE_MARK_FILTERING_SET: u16 = 0x0010;
}

/// One generated lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedLookup {
    /// The anchor name a mark lookup attaches by (`top`), which names its mark class; empty for
    /// pair and cursive lookups.
    pub class: String,
    pub flags: LookupFlags,
    /// The glyphs of the lookup's mark filtering set, sorted, when it has one.
    pub filter_set: Option<Vec<GlyphId16>>,
    pub rules: GeneratedRules,
}

/// Where a feature's lookups are registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureKey {
    pub feature: Tag,
    pub script: Tag,
    pub language: Tag,
}

/// The `GDEF` glyph class of a glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlyphClass {
    Base,
    Ligature,
    Mark,
    Component,
}

/// One condition of a [`FeatureVariation`]: an axis range in normalised coordinates,
/// `min <= max`, both finite and within -1 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Condition {
    pub axis: Tag,
    pub min: f64,
    pub max: f64,
}

/// Substitutions that apply where every condition holds (M9.8's bracket layers), added to
/// feature `tag` through fea-rs's feature variations. fea-rs takes the feature variations of a
/// compile in one call that applies every condition set to the same list of features, so the
/// variations of one compile should share their tag (the font's feature for feature
/// variations, `rvrn` by default); the conversion applies each to the union of the tags.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureVariation {
    pub tag: Tag,
    pub conditions: Vec<Condition>,
    /// Single substitutions, from the default glyph to the alternate.
    pub substitutions: Vec<(GlyphId16, GlyphId16)>,
}

/// Everything FontCraft generates for one compile, for fea-rs to merge with the code.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeneratedLookups {
    /// In the order they are added to the font: the reference compilers' order of generated
    /// features (`curs`, `kern`/`dist`, `abvm`, `blwm`, `mark`, `mkmk`).
    pub lookups: Vec<GeneratedLookup>,
    /// For each feature key, the indices into [`GeneratedLookups::lookups`] it uses, in order.
    pub features: BTreeMap<FeatureKey, Vec<usize>>,
    /// Features whose generated lookups go after every hand-written lookup even where the code
    /// has an `# Automatic Code` marker (a feature writer in append mode).
    pub append: Vec<Tag>,
    /// The `GDEF` glyph classes, used when the code has no `table GDEF` block.
    pub glyph_classes: BTreeMap<GlyphId16, GlyphClass>,
    /// Ligature caret positions, sorted, used when the code has no `table GDEF` block.
    pub carets: BTreeMap<GlyphId16, Vec<Metric>>,
    /// Bracket-layer substitutions of a variable font (M9.8); empty for a static font.
    pub feature_variations: Vec<FeatureVariation>,
    /// The variable font's axis tags in `fvar` order, which the regions of [`Metric::deltas`]
    /// name; empty for a static font, whose values have no deltas. At most [`crate::MAX_AXES`].
    pub axes: Vec<Tag>,
}

impl GeneratedLookups {
    pub fn is_empty(&self) -> bool {
        self.lookups.is_empty() && self.glyph_classes.is_empty() && self.carets.is_empty() && self.feature_variations.is_empty()
    }

    /// The rules that count towards [`crate::MAX_GENERATED_RULES`], saturating.
    pub fn rule_count(&self) -> usize {
        let lookups = self.lookups.iter().fold(0usize, |n, l| n.saturating_add(l.rules.rule_count()));
        self.feature_variations.iter().fold(lookups, |n, v| n.saturating_add(v.substitutions.len()))
    }
}

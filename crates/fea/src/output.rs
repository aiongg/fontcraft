//! What a compile produces: layout tables as bytes, the values the code's table blocks set, and
//! a summary of the features for the Features menu.

use std::collections::BTreeMap;

use crate::{Diagnostic, GlyphClass, GlyphId16, Tag};

/// Which layout table a feature is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutTable {
    Gsub,
    Gpos,
}

/// One feature of the compiled tables.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureSummary {
    pub tag: Tag,
    pub table: LayoutTable,
    /// The (script, language) pairs it is registered for, in table order.
    pub language_systems: Vec<(Tag, Tag)>,
    /// How many lookups it uses, across its language systems, each counted once.
    pub lookups: usize,
}

/// The values a `table head` block sets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeadValues {
    pub font_revision: Option<f64>,
}

/// The values a `table hhea` or `table vhea` block sets. For `vhea` the three metrics are the
/// vertical typographic ascender, descender and line gap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricsValues {
    pub caret_offset: Option<i16>,
    pub ascender: Option<i16>,
    pub descender: Option<i16>,
    pub line_gap: Option<i16>,
}

/// The values a `table OS/2` block sets; `None` fields keep the value the compiler computes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Os2Values {
    pub weight_class: Option<u16>,
    pub width_class: Option<u16>,
    pub fs_type: Option<u16>,
    pub family_class: Option<i16>,
    pub panose: Option<[u8; 10]>,
    /// The four `ulUnicodeRange` words.
    pub unicode_range: Option<[u32; 4]>,
    pub vendor_id: Option<Tag>,
    pub win_ascent: Option<u16>,
    pub win_descent: Option<u16>,
    /// The two `ulCodePageRange` words.
    pub code_page_range: Option<[u32; 2]>,
    pub x_height: Option<i16>,
    pub cap_height: Option<i16>,
    pub typo_ascender: Option<i16>,
    pub typo_descender: Option<i16>,
    pub typo_line_gap: Option<i16>,
    pub lower_optical_point_size: Option<u16>,
    pub upper_optical_point_size: Option<u16>,
}

/// The result of a compile. Tables are serialised; `None` when the code and the generated
/// lookups give none. The `name` table holds only the names the code declares, already moved
/// to [`crate::CompileRequest::first_name_id`]; the compiler merges it with its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compiled {
    pub gsub: Option<Vec<u8>>,
    pub gpos: Option<Vec<u8>>,
    pub gdef: Option<Vec<u8>>,
    pub base: Option<Vec<u8>>,
    pub name: Option<Vec<u8>>,
    pub stat: Option<Vec<u8>>,
    pub head: HeadValues,
    pub hhea: MetricsValues,
    pub vhea: MetricsValues,
    pub os2: Os2Values,
    /// The features of `GSUB` and `GPOS`, in feature-list order.
    pub features: Vec<FeatureSummary>,
    /// `GDEF` came from the code's `table GDEF` block rather than the generated classes.
    pub gdef_from_code: bool,
    /// Warnings that did not stop the compile.
    pub warnings: Vec<Diagnostic>,
}

/// What [`crate::first_pass`] reads from the code for the feature writers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FirstPass {
    /// The `languagesystem` statements, as (script, language), each once, in code order.
    pub language_systems: Vec<(Tag, Tag)>,
    /// The top-level feature blocks, each tag once in code order, with `true` when one of its
    /// blocks holds an `# Automatic Code` marker.
    pub features: Vec<(Tag, bool)>,
    /// `GSUB` compiled from the code alone; `None` when the code has no substitutions.
    pub gsub: Option<Vec<u8>>,
    /// The glyph classes of the code's `table GDEF` block, when it has one.
    pub gdef_classes: Option<BTreeMap<GlyphId16, GlyphClass>>,
    /// Warnings that did not stop the pass.
    pub warnings: Vec<Diagnostic>,
}

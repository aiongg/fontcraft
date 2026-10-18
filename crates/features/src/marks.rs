//! The mark writer (M6.17): `mark`, `mkmk`, `abvm`, `blwm` and `curs` attachment, the `GDEF`
//! glyph classes and the ligature caret list, generated from propagated anchors.
//!
//! Rows: `compile.markFeatureWriter`, `compile.gdefClasses`, `compile.gdefLigatureCarets`,
//! `anchors.markToBase`, `anchors.markToMark`, `anchors.cursiveAttachment`,
//! `anchors.ligatureCarets`, `autoFeature.mark`, `autoFeature.mkmk`, `autoFeature.curs`,
//! `autoFeature.abvm`, `autoFeature.blwm`.
//!
//! Ported from fontc (https://github.com/googlefonts/fontc), Apache-2.0, at commit
//! 4504578c3f74c2739f7e55cae133bbfe773f6d8c: the mark and cursive writer
//! (`fontbe/src/features/marks.rs`), the anchor names (`AnchorKind::new` in `fontir/src/ir.rs`),
//! the feature-writer setting of the mark, cursive and `GDEF` writers
//! (`fontir/src/ir/feature_writers.rs`), the lookup order of generated features
//! (`fea-rs/src/compile/feature_writer.rs`) and the caret order (`fea-rs/src/compile/tables/gdef.rs`).
//! fontc's writer follows ufo2ft's `markFeatureWriter.py`, `cursFeatureWriter.py` and
//! `gdefFeatureWriter.py` (MIT License). The `GDEF` class rules are in [`gdef`].
//!
//! Contract:
//! - The input is plain data ([`MarkInput`]): per glyph, in glyph-id order, its code points, its
//!   category and subcategory (which decide the `GDEF` class) and its anchors per master after
//!   propagation. This crate does not depend on `fontcraft-components`; the compiler builds the
//!   input from `fontcraft_components::propagate::effective_anchors_for_master`.
//! - [`mark_features`] writes one master ([`MarkRequest::master`]). A feature whose
//!   [`MarkTags`] flag is off (the source has a hand-written one without `# Automatic Code`)
//!   is not generated.
//! - The `GPOS` result holds only the generated mark lookups; the compiler merges it with the
//!   kerning of [`crate::kern_feature`]. The cursive lookups come first
//!   ([`MarkFeatures::curs_lookups`]): the kerning goes after them and before the others.
//! - [`generate_marks`] gives the same lookups as plain data ([`GeneratedMarks`]: per lookup its
//!   feature, anchor name, flags, filter set and rules), for M8's feature compiler to merge into
//!   feature code; [`mark_features`] builds the tables from it.
//! - Every count the work grows with is capped ([`MarkLimits`]).
//!
//! What the writer does, in order:
//! 1. Reads each glyph's anchors on the master. A name repeated on one glyph keeps its first
//!    place and its last position. Names say what an anchor is: `_top` a mark's, `top` a base's,
//!    `top_2` the second component's of a ligature, `_2` a ligature component without anchors,
//!    `caret_1` and `vcaret_1` caret positions, `entry` and `exit` cursive attachment. A name the
//!    reference compilers refuse (`_`, `_top_1`, a number 0) is left out with a warning.
//! 2. Gives the glyphs their `GDEF` class ([`gdef`]).
//! 3. Keeps the anchors that take part: a glyph outside the base, mark and ligature classes takes
//!    no part when any glyph has a class; a mark or base anchor only when both `_name` and `name`
//!    occur; cursive anchors always.
//! 4. Groups attachments by anchor name: one mark-to-base, mark-to-ligature and mark-to-mark
//!    lookup per name, in name order. Mark-to-mark lookups use a mark filtering set of the marks
//!    of their name.
//! 5. Splits the lookups between `abvm`/`blwm` and `mark`/`mkmk` by the scripts of the bases:
//!    when the font encodes a character of the Indic scripts, the Universal Shaping Engine's or
//!    Khmer, its glyphs go to `abvm` (names other than `bottom…` and `nukta…`) and `blwm`.
//! 6. Writes `curs` when the font has both an `entry` and an `exit` anchor: one lookup for
//!    left-to-right glyphs and one, right to left, for the others, both skipping marks.
//! 7. Registers every feature for each language system of the feature code (`DFLT dflt` without
//!    one), lookups in the order `curs`, `abvm`, `blwm`, `mark`, `mkmk`.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use fontcraft_model::plist::{Num, Value};
use write_fonts::tables::gdef::{Gdef, GlyphClassDef};
use write_fonts::tables::gpos::builders::{AnchorBuilder, CursivePosBuilder, MarkToBaseBuilder, MarkToLigBuilder, MarkToMarkBuilder};
use write_fonts::tables::gpos::{Gpos, PositionLookup, PositionLookupList};
use write_fonts::tables::layout::builders::{Builder, LookupBuilder};
use write_fonts::tables::layout::{Feature, FeatureList, FeatureRecord, LangSys, LangSysRecord, LookupFlag, Script, ScriptList, ScriptRecord};
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::types::{GlyphId16, Tag};

use crate::FeatureError;
use crate::properties::{
    COMMON_SCRIPT, DFLT_LANG, DFLT_SCRIPT, INHERITED_SCRIPT, ScriptCode, ScriptDirection, dist_feature_enabled_scripts, glyphs_matching,
    script_to_ot_tags, unicode_script_extensions,
};
use read_fonts::tables::gsub::Gsub;

#[path = "marks_contextual.rs"]
pub mod contextual;
#[path = "gdef.rs"]
mod gdef;

/// Most steps (glyphs, anchors and attachment rules visited) the writer may take for one master.
pub const MAX_MARK_WORK: usize = 100_000_000;
/// The highest ligature component number an anchor name may give (`top_1024`). A higher one is
/// left out with a warning: each ligature keeps a slot for every component up to its highest.
pub const MAX_LIGATURE_COMPONENTS: usize = 1_024;

/// One anchor after propagation, in font units.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkAnchor {
    pub name: String,
    pub x: f64,
    pub y: f64,
}

/// A glyph's anchors on one master.
#[derive(Clone, Debug, PartialEq)]
pub struct MasterAnchors {
    pub master: String,
    pub anchors: Vec<MarkAnchor>,
}

/// One glyph of the compiled font.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkGlyph {
    pub name: String,
    pub codepoints: Vec<u32>,
    /// The effective category and subcategory (`Mark`, `Nonspacing`; `Letter`, `Ligature`…).
    pub category: Option<String>,
    pub sub_category: Option<String>,
    /// The class a UFO's `public.openTypeCategories` states: `base`, `mark`, `ligature`,
    /// `component` or `unassigned`. Read only when [`MarkInput::explicit_classes`] is set.
    pub opentype_category: Option<String>,
    /// The effective anchors, one entry per master that has a layer.
    pub anchors: Vec<MasterAnchors>,
}

/// What the writer reads: the glyphs in the compiled font's order (a glyph's index is its id).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkInput {
    pub glyphs: Vec<MarkGlyph>,
    /// The classes are the stated ones ([`MarkGlyph::opentype_category`]), as for a UFO source;
    /// otherwise they come from the categories and the anchors, as for the reference's format.
    pub explicit_classes: bool,
    /// The language systems of the source's feature code (`languagesystem latn dflt;`), as
    /// (script tag, language tag). The generated features are registered for each; for `DFLT
    /// dflt` when there is none.
    pub language_systems: Vec<(String, String)>,
}

/// Which generated features to write. Each is on unless the source has a hand-written feature
/// of that tag without `# Automatic Code`, or its feature-writer setting leaves the writer out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkTags {
    pub mark: bool,
    pub mkmk: bool,
    pub abvm: bool,
    pub blwm: bool,
    pub curs: bool,
    /// The `GDEF` glyph classes and ligature carets. Mark filtering sets are written whenever a
    /// lookup uses one.
    pub gdef: bool,
}

impl Default for MarkTags {
    fn default() -> Self {
        Self { mark: true, mkmk: true, abvm: true, blwm: true, curs: true, gdef: true }
    }
}

impl MarkTags {
    pub const NONE: MarkTags = MarkTags { mark: false, mkmk: false, abvm: false, blwm: false, curs: false, gdef: false };
}

/// The caps [`mark_features`] works under. The default is the `MAX_…` constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkLimits {
    pub work: usize,
}

impl Default for MarkLimits {
    fn default() -> Self {
        Self { work: MAX_MARK_WORK }
    }
}

/// What [`mark_features`] writes from.
#[derive(Clone, Copy, Debug)]
pub struct MarkRequest<'a> {
    pub input: &'a MarkInput,
    /// The master compiled; anchors of other masters are ignored.
    pub master: &'a str,
    pub tags: MarkTags,
    pub limits: MarkLimits,
    /// The feature code's `GSUB` alone ([`fontcraft_fea::FirstPass::gsub`]): the glyphs the
    /// substitutions reach join the `abvm` and `blwm` glyphs of the code points they come from.
    pub gsub: Option<&'a [u8]>,
}

/// The generated attachment and glyph classes.
#[derive(Debug, Default)]
pub struct MarkFeatures {
    /// `None` when no attachment rule is generated.
    pub gpos: Option<Gpos>,
    /// The glyph class definitions and the ligature caret list; `None` when no glyph has a class.
    pub gdef: Option<Gdef>,
    /// How many lookups `gpos` has.
    pub lookups: usize,
    /// How many of them, at the start of the lookup list, are the `curs` lookups.
    pub curs_lookups: usize,
    /// The same lookups, classes and carets as plain data, for merging with feature code (M8).
    pub generated: GeneratedMarks,
    /// What the user should know: anchors left out, values that did not fit.
    pub warnings: Vec<String>,
}

/// An anchor position in font units, rounded.
pub type Point16 = (i16, i16);

/// The rules of one generated lookup.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkRules {
    /// Marks of the lookup's class and the base anchor each base offers them.
    MarkToBase { marks: Vec<(GlyphId16, Point16)>, bases: Vec<(GlyphId16, Point16)> },
    /// Marks and, for each ligature, the anchor of each component (a component may have none).
    MarkToLigature { marks: Vec<(GlyphId16, Point16)>, ligatures: Vec<(GlyphId16, Vec<Option<Point16>>)> },
    /// Attaching marks and the marks they stack on.
    MarkToMark { marks: Vec<(GlyphId16, Point16)>, bases: Vec<(GlyphId16, Point16)> },
    /// Entry and exit anchors by glyph.
    Cursive { glyphs: Vec<(GlyphId16, Option<Point16>, Option<Point16>)> },
}

/// One generated lookup, before it is written to a table.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkLookup {
    /// `mark`, `mkmk`, `abvm`, `blwm` or `curs`.
    pub feature: Tag,
    /// The anchor name the lookup attaches by (`top`); empty for `curs`.
    pub class: String,
    pub flags: LookupFlag,
    /// The glyphs of the lookup's mark filtering set, sorted, when it has one.
    pub filter_set: Option<Vec<GlyphId16>>,
    pub rules: MarkRules,
}

/// The mark writer's output as plain data, independent of the table writer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeneratedMarks {
    /// In the order the reference compilers write generated features: `curs`, `abvm`, `blwm`,
    /// `mark` (mark-to-base, then mark-to-ligature), `mkmk`.
    pub lookups: Vec<MarkLookup>,
    /// The language systems the features are registered for, as (script, language).
    pub language_systems: Vec<(Tag, Tag)>,
    /// The `GDEF` glyph classes (empty when [`MarkTags::gdef`] is off).
    pub classes: BTreeMap<GlyphId16, GlyphClassDef>,
    /// The ligature caret positions, sorted (empty when [`MarkTags::gdef`] is off).
    pub carets: BTreeMap<GlyphId16, Vec<i16>>,
}

const MARK: Tag = Tag::new(b"mark");
const MKMK: Tag = Tag::new(b"mkmk");
const CURS: Tag = Tag::new(b"curs");
const ABVM: Tag = Tag::new(b"abvm");
const BLWM: Tag = Tag::new(b"blwm");

/// How many anchors with problems a warning names before it only counts them.
const NAMED_IN_WARNING: usize = 5;

// ---------------------------------------------------------------------------------------------
// The feature-writer setting
// ---------------------------------------------------------------------------------------------

const DEFAULT_MODULE: &str = "ufo2ft.featureWriters";
const GLYPHSLIB_MARK_MODULE: &str = "glyphsLib.featureWriters.markFeatureWriter";

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriterKind {
    Curs,
    Mark,
    Gdef,
}

/// The mark, cursive and `GDEF` writers' part of the source's feature-writer setting (the
/// [`crate::FEATURE_WRITERS_KEY`] value, `None` without the key): without the key every writer
/// runs; with it only those it lists. An entry that is malformed, or has an option the writer
/// does not take, is dropped. An option fontc refuses to build with is written with the standard
/// settings and a warning. A setting that is not a list runs nothing ([`crate::kern_tags`] warns).
pub fn mark_tags(setting: Option<&Value>, warnings: &mut Vec<String>) -> MarkTags {
    let Some(setting) = setting else { return MarkTags::default() };
    let Some(entries) = setting.as_array() else { return MarkTags::NONE };
    let mut tags = MarkTags::NONE;
    for entry in &entries.items {
        let Some((writer, features)) = writer_entry(entry, warnings) else { continue };
        match writer {
            WriterKind::Curs => tags.curs = true,
            WriterKind::Gdef => tags.gdef = true,
            WriterKind::Mark => {
                let wants = |tag: &str| features.as_ref().is_none_or(|f| f.iter().any(|t| t == tag));
                tags.mark = wants("mark");
                tags.mkmk = wants("mkmk");
                tags.abvm = wants("abvm");
                tags.blwm = wants("blwm");
            }
        }
    }
    tags
}

/// One entry of the setting, when it is a valid entry of one of the three writers: the writer and
/// the tags its `features` option keeps.
fn writer_entry(entry: &Value, warnings: &mut Vec<String>) -> Option<(WriterKind, Option<Vec<String>>)> {
    let dict = entry.as_dict()?;
    let class = dict.get("class")?.as_str()?;
    let module = match dict.get("module") {
        None => DEFAULT_MODULE,
        Some(m) => m.as_str()?,
    };
    let writer = match (module, class) {
        (DEFAULT_MODULE | "ufo2ft.featureWriters.cursFeatureWriter", "CursFeatureWriter") => WriterKind::Curs,
        (DEFAULT_MODULE | "ufo2ft.featureWriters.markFeatureWriter", "MarkFeatureWriter")
        | (GLYPHSLIB_MARK_MODULE, "MarkFeatureWriter" | "ContextualMarkFeatureWriter") => WriterKind::Mark,
        (DEFAULT_MODULE | "ufo2ft.featureWriters.gdefFeatureWriter", "GdefFeatureWriter") => WriterKind::Gdef,
        _ => return None,
    };
    let mut features = None;
    let mut unapplied = Vec::new();
    if let Some(options) = dict.get("options") {
        for (key, value) in &options.as_dict()?.entries {
            match (key.as_str(), writer) {
                ("mode", _) => {
                    // Append mode is read by `crate::append_tags`.
                    if !matches!(value.as_str(), Some("skip" | "append")) {
                        return None;
                    }
                }
                ("features", WriterKind::Gdef) => {
                    let parts = string_list(value)?;
                    if parts.is_empty() || !parts.iter().all(|p| matches!(p.as_str(), "GlyphClassDefs" | "LigatureCarets")) {
                        return None;
                    }
                    unapplied.push(format!("only {} of the GDEF table", parts.join(" and ")));
                }
                ("features", _) => {
                    let tags = string_list(value)?;
                    let allowed: &[&str] = if writer == WriterKind::Curs { &["curs"] } else { &["mark", "mkmk", "abvm", "blwm"] };
                    if tags.is_empty() || !tags.iter().all(|t| allowed.contains(&t.as_str())) {
                        return None;
                    }
                    features = Some(tags);
                }
                ("quantization", WriterKind::Mark) => match value.as_num()? {
                    Num::Int(1) => {}
                    Num::Int(q) => unapplied.push(format!("anchor quantization {q}")),
                    Num::Float(_) => return None,
                },
                ("groupMarkClasses", WriterKind::Mark) => match flag(value)? {
                    false => {}
                    true => unapplied.push("grouped mark classes".to_string()),
                },
                _ => return None,
            }
        }
    }
    for what in unapplied {
        warnings.push(format!("the feature-writer setting asks for {what}, which is not supported: the standard settings are used"));
    }
    Some((writer, features))
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    value.as_array()?.items.iter().map(|v| v.as_str().map(str::to_string)).collect()
}

/// A yes-or-no option: a property-list boolean, or 0 or 1 in a format without booleans.
fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.value() {
            Num::Int(0) => Some(false),
            Num::Int(1) => Some(true),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Anchors
// ---------------------------------------------------------------------------------------------

/// What an anchor is, from its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AnchorKind {
    /// `top`: marks of the group attach to it.
    Base(String),
    /// `_top`: attaches to the group's base anchors.
    Mark(String),
    /// `top_2`: a base anchor of a ligature's component (numbered from 1).
    Ligature {
        group: String,
        index: usize,
    },
    /// `_2`: a ligature component without anchors.
    ComponentMarker(usize),
    /// `caret_1`
    Caret(usize),
    /// `vcaret_1`
    VCaret(usize),
    CursiveEntry,
    CursiveExit,
}

/// The kind of anchor `name` names, or why the reference compilers refuse the name.
pub(crate) fn anchor_kind(name: &str) -> Result<AnchorKind, &'static str> {
    if name == "entry" {
        return Ok(AnchorKind::CursiveEntry);
    }
    if name == "exit" {
        return Ok(AnchorKind::CursiveExit);
    }
    let vertical = name.starts_with('v');
    if let Some(suffix) = name.strip_prefix("caret_").or_else(|| name.strip_prefix("vcaret_")) {
        // A suffix that is not a number still makes a caret; ufo2ft reads such names the same way.
        let index = suffix.parse::<usize>().unwrap_or(1);
        if index == 0 {
            return Err("a caret number must not be 0");
        }
        return Ok(if vertical { AnchorKind::VCaret(index) } else { AnchorKind::Caret(index) });
    }
    if let Some(suffix) = name.strip_prefix('_') {
        if let Ok(index) = suffix.parse::<usize>() {
            if index == 0 {
                return Err("a ligature component number must not be 0");
            }
            return Ok(AnchorKind::ComponentMarker(index));
        }
        if suffix.is_empty() {
            return Err("a mark anchor needs a name after the underscore");
        }
        if suffix.rsplit_once('_').is_some_and(|(_, n)| n.parse::<usize>().is_ok()) {
            return Err("a mark anchor must not end in a ligature component number");
        }
        return Ok(AnchorKind::Mark(suffix.to_string()));
    }
    if let Some((group, suffix)) = name.rsplit_once('_')
        && let Ok(index) = suffix.parse::<usize>()
    {
        if index == 0 {
            return Err("a ligature component number must not be 0");
        }
        return Ok(AnchorKind::Ligature { group: group.to_string(), index });
    }
    Ok(AnchorKind::Base(name.to_string()))
}

/// One anchor of a glyph, its position rounded.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Anchor {
    pub(crate) kind: AnchorKind,
    pub(crate) x: i16,
    pub(crate) y: i16,
}

impl Anchor {
    fn group(&self) -> Option<&str> {
        match &self.kind {
            AnchorKind::Base(g) | AnchorKind::Mark(g) | AnchorKind::Ligature { group: g, .. } => Some(g),
            _ => None,
        }
    }

    fn is_cursive(&self) -> bool {
        matches!(self.kind, AnchorKind::CursiveEntry | AnchorKind::CursiveExit)
    }

    fn is_mark(&self) -> bool {
        matches!(self.kind, AnchorKind::Mark(_))
    }

    fn ligature_index(&self) -> Option<usize> {
        match self.kind {
            AnchorKind::Ligature { index, .. } | AnchorKind::ComponentMarker(index) => Some(index),
            _ => None,
        }
    }

    fn point(&self) -> Point16 {
        (self.x, self.y)
    }
}

/// OpenType rounding (half up) into 16 bits; `None` when the value is not finite or does not fit.
fn round_i16(v: f64) -> Option<i16> {
    if !v.is_finite() {
        return None;
    }
    let r = (v + 0.5).floor();
    (r >= f64::from(i16::MIN) && r <= f64::from(i16::MAX)).then_some(r as i16)
}

/// The anchors of each glyph on the master, by glyph id; glyphs without anchors are left out.
fn read_anchors(req: &MarkRequest, budget: &mut Budget, warnings: &mut Vec<String>) -> Result<BTreeMap<GlyphId16, Vec<Anchor>>, FeatureError> {
    let mut out = BTreeMap::new();
    let mut refused: Vec<String> = Vec::new();
    let mut refused_count = 0usize;
    let mut unplaced: Vec<String> = Vec::new();
    let mut unplaced_count = 0usize;
    for (i, glyph) in req.input.glyphs.iter().enumerate() {
        let Ok(id) = u16::try_from(i) else {
            warnings.push(format!("only the first {} glyphs take part in mark attachment", u16::MAX));
            break;
        };
        budget.spend(1)?;
        let Some(master) = glyph.anchors.iter().find(|m| m.master == req.master) else { continue };
        budget.spend(master.anchors.len())?;
        // A repeated name keeps its first place and its last position, as ufo2ft reads it.
        let mut order: Vec<&MarkAnchor> = Vec::with_capacity(master.anchors.len());
        let mut at: HashMap<&str, usize> = HashMap::new();
        for a in &master.anchors {
            if let Some(&k) = at.get(a.name.as_str()) {
                if let Some(slot) = order.get_mut(k) {
                    *slot = a;
                }
            } else {
                at.insert(a.name.as_str(), order.len());
                order.push(a);
            }
        }
        let mut anchors = Vec::with_capacity(order.len());
        for a in order {
            let kind = match anchor_kind(&a.name) {
                Ok(kind) => kind,
                Err(why) => {
                    refused_count += 1;
                    if refused.len() < NAMED_IN_WARNING {
                        refused.push(format!("`{}` of `{}` ({why})", a.name, glyph.name));
                    }
                    continue;
                }
            };
            if kind_index(&kind).is_some_and(|n| n > MAX_LIGATURE_COMPONENTS) {
                refused_count += 1;
                if refused.len() < NAMED_IN_WARNING {
                    refused.push(format!("`{}` of `{}` (a component number above {MAX_LIGATURE_COMPONENTS})", a.name, glyph.name));
                }
                continue;
            }
            let (Some(x), Some(y)) = (round_i16(a.x), round_i16(a.y)) else {
                unplaced_count += 1;
                if unplaced.len() < NAMED_IN_WARNING {
                    unplaced.push(format!("`{}` of `{}`", a.name, glyph.name));
                }
                continue;
            };
            anchors.push(Anchor { kind, x, y });
        }
        if !anchors.is_empty() {
            out.insert(GlyphId16::new(id), anchors);
        }
    }
    if refused_count > 0 {
        warnings.push(format!(
            "{refused_count} anchors were left out of mark attachment, their names are not valid: {}",
            listed(&refused, refused_count)
        ));
    }
    if unplaced_count > 0 {
        warnings.push(format!(
            "{unplaced_count} anchors were left out of mark attachment, their positions do not fit the font: {}",
            listed(&unplaced, unplaced_count)
        ));
    }
    Ok(out)
}

fn kind_index(kind: &AnchorKind) -> Option<usize> {
    match kind {
        AnchorKind::Ligature { index, .. } | AnchorKind::ComponentMarker(index) | AnchorKind::Caret(index) | AnchorKind::VCaret(index) => {
            Some(*index)
        }
        _ => None,
    }
}

fn listed(named: &[String], count: usize) -> String {
    let mut text = named.join(", ");
    if count > named.len() {
        text.push_str(&format!(" and {} more", count - named.len()));
    }
    text
}

// ---------------------------------------------------------------------------------------------
// The work budget
// ---------------------------------------------------------------------------------------------

/// What the writer may still do ([`MarkLimits::work`]).
struct Budget {
    left: usize,
    limit: usize,
}

impl Budget {
    fn new(limit: usize) -> Self {
        Self { left: limit, limit }
    }

    fn spend(&mut self, n: usize) -> Result<(), FeatureError> {
        self.left = self.left.checked_sub(n).ok_or(FeatureError::TooMany { what: "steps of work", limit: self.limit })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Groups and lookups
// ---------------------------------------------------------------------------------------------

/// A base's anchor, or a ligature's anchors by component (a component may have none).
#[derive(Clone, Debug)]
enum BaseOrLig<T> {
    Base(T),
    Ligature(Vec<Option<T>>),
}

/// The bases and marks of one anchor name.
#[derive(Clone, Debug, Default)]
struct MarkGroup<'a> {
    bases: Vec<(GlyphId16, BaseOrLig<&'a Anchor>)>,
    marks: Vec<(GlyphId16, &'a Anchor)>,
    /// Mark-to-mark: the lookup gets a mark filtering set of the group's marks.
    filter_glyphs: bool,
}

/// A set of glyph ids, one flag per glyph.
struct GlyphSet(Vec<bool>);

impl GlyphSet {
    fn all(n: usize) -> Self {
        GlyphSet(vec![true; n])
    }

    fn none(n: usize) -> Self {
        GlyphSet(vec![false; n])
    }

    fn insert(&mut self, gid: GlyphId16) {
        if let Some(slot) = self.0.get_mut(usize::from(gid.to_u16())) {
            *slot = true;
        }
    }

    fn contains(&self, gid: GlyphId16) -> bool {
        self.0.get(usize::from(gid.to_u16())).copied().unwrap_or(false)
    }

    fn is_empty(&self) -> bool {
        !self.0.iter().any(|b| *b)
    }
}

impl MarkGroup<'_> {
    /// The group with only the bases in `include`; `None` when no base or no mark is left.
    fn only_using_glyphs(&self, include: &GlyphSet) -> Option<MarkGroup<'_>> {
        let bases: Vec<_> = self.bases.iter().filter(|(gid, _)| include.contains(*gid)).cloned().collect();
        if bases.is_empty() || self.marks.is_empty() {
            return None;
        }
        Some(MarkGroup { bases, marks: self.marks.clone(), filter_glyphs: self.filter_glyphs })
    }

    /// The marks in `include` and the bases that are not marks of the group, for a group that
    /// filters (ufo2ft's `_makeMarkFilteringSetClass`).
    fn filter_set(&self, include: &GlyphSet) -> Option<Vec<GlyphId16>> {
        if !self.filter_glyphs {
            return None;
        }
        let marks: HashSet<GlyphId16> = self.marks.iter().map(|(gid, _)| *gid).collect();
        let set: BTreeSet<GlyphId16> = self
            .marks
            .iter()
            .filter(|(gid, _)| include.contains(*gid))
            .map(|(gid, _)| *gid)
            .chain(self.bases.iter().filter(|(gid, _)| !marks.contains(gid)).map(|(gid, _)| *gid))
            .collect();
        Some(set.into_iter().collect())
    }
}

/// Which attachment a group map feeds.
#[derive(Clone, Copy)]
enum Attachment {
    Base,
    Ligature,
    Mark,
}

/// The lookups of one feature (or of `abvm` and `blwm` each), by attachment type.
#[derive(Default)]
struct ByType {
    base: Vec<MarkLookup>,
    lig: Vec<MarkLookup>,
    mark: Vec<MarkLookup>,
}

struct Groups<'a> {
    base: BTreeMap<String, MarkGroup<'a>>,
    lig: BTreeMap<String, MarkGroup<'a>>,
    mark: BTreeMap<String, MarkGroup<'a>>,
}

/// The writer's state for one master: the participating anchors and the classes.
struct MarkWriter<'a> {
    classes: BTreeMap<GlyphId16, GlyphClassDef>,
    /// Only the anchors that take part, by glyph.
    anchors: BTreeMap<GlyphId16, Vec<&'a Anchor>>,
    mark_glyphs: BTreeSet<GlyphId16>,
}

impl<'a> MarkWriter<'a> {
    fn new(
        all: &'a BTreeMap<GlyphId16, Vec<Anchor>>,
        classes: BTreeMap<GlyphId16, GlyphClassDef>,
        budget: &mut Budget,
    ) -> Result<Self, FeatureError> {
        let include: HashSet<GlyphId16> = classes
            .iter()
            .filter(|(_, c)| matches!(c, GlyphClassDef::Base | GlyphClassDef::Mark | GlyphClassDef::Ligature))
            .map(|(g, _)| *g)
            .collect();
        let mut pruned: BTreeMap<GlyphId16, Vec<&Anchor>> = BTreeMap::new();
        let mut base_groups: HashSet<&str> = HashSet::new();
        let mut mark_groups: HashSet<&str> = HashSet::new();
        for (gid, anchors) in all {
            if !include.is_empty() && !include.contains(gid) {
                continue;
            }
            budget.spend(anchors.len())?;
            for anchor in anchors {
                match &anchor.kind {
                    AnchorKind::Base(g) | AnchorKind::Ligature { group: g, .. } => {
                        base_groups.insert(g);
                    }
                    AnchorKind::Mark(g) => {
                        mark_groups.insert(g);
                    }
                    _ => {}
                }
                pruned.entry(*gid).or_default().push(anchor);
            }
        }
        // Marks without bases of their name and bases without marks take no part.
        let used: HashSet<&str> = base_groups.intersection(&mark_groups).copied().collect();
        pruned.retain(|_, anchors| {
            anchors.retain(|a| {
                a.is_cursive()
                    || match a.group() {
                        Some(g) => used.contains(g),
                        None => matches!(a.kind, AnchorKind::ComponentMarker(_)),
                    }
            });
            !anchors.is_empty()
        });
        // A mark glyph has a mark anchor and, when any glyph has a class, the mark class.
        let mark_glyphs = pruned
            .iter()
            .filter(|(gid, anchors)| (classes.is_empty() || classes.get(*gid) == Some(&GlyphClassDef::Mark)) && anchors.iter().any(|a| a.is_mark()))
            .map(|(gid, _)| *gid)
            .collect();
        Ok(MarkWriter { classes, anchors: pruned, mark_glyphs })
    }

    fn mark_to_base_groups(&self) -> BTreeMap<String, MarkGroup<'a>> {
        let mut groups: BTreeMap<String, MarkGroup> = BTreeMap::new();
        for (gid, anchors) in &self.anchors {
            let is_mark = self.mark_glyphs.contains(gid);
            let is_not_base = !self.classes.is_empty() && self.classes.get(gid) != Some(&GlyphClassDef::Base);
            let as_base = !(is_mark || is_not_base);
            for anchor in anchors {
                match &anchor.kind {
                    AnchorKind::Base(group) if as_base => groups.entry(group.clone()).or_default().bases.push((*gid, BaseOrLig::Base(*anchor))),
                    AnchorKind::Mark(group) if is_mark => groups.entry(group.clone()).or_default().marks.push((*gid, *anchor)),
                    _ => {}
                }
            }
        }
        groups
    }

    fn mark_to_mark_groups(&self) -> BTreeMap<String, MarkGroup<'a>> {
        // The mark glyphs, and the names of their mark anchors.
        let mut marks: BTreeSet<GlyphId16> = BTreeSet::new();
        let mut mark_names: BTreeSet<&str> = BTreeSet::new();
        for (gid, anchors) in self.anchors.iter().filter(|(gid, _)| self.mark_glyphs.contains(*gid)) {
            for anchor in anchors {
                if let AnchorKind::Mark(group) = &anchor.kind {
                    marks.insert(*gid);
                    mark_names.insert(group);
                }
            }
        }
        // Marks with a base anchor of a name some mark attaches by are the group's bases.
        let mut groups: BTreeMap<String, MarkGroup> = BTreeMap::new();
        for (gid, anchors) in &self.anchors {
            if !marks.contains(gid) {
                continue;
            }
            for anchor in anchors {
                if let AnchorKind::Base(group) = &anchor.kind
                    && mark_names.contains(group.as_str())
                {
                    let entry = groups.entry(group.clone()).or_default();
                    entry.filter_glyphs = true;
                    entry.bases.push((*gid, BaseOrLig::Base(*anchor)));
                }
            }
        }
        for gid in &marks {
            for anchor in self.anchors.get(gid).into_iter().flatten() {
                if let AnchorKind::Mark(group) = &anchor.kind
                    && let Some(entry) = groups.get_mut(group)
                {
                    entry.marks.push((*gid, *anchor));
                }
            }
        }
        groups
    }

    fn mark_to_ligature_groups(&self, budget: &mut Budget) -> Result<BTreeMap<String, MarkGroup<'a>>, FeatureError> {
        let mut groups: BTreeMap<String, MarkGroup> = BTreeMap::new();
        let mut ligature_names: HashSet<&str> = HashSet::new();
        for (gid, anchors) in &self.anchors {
            let might_be_ligature = self.classes.is_empty() || self.classes.get(gid) == Some(&GlyphClassDef::Ligature);
            if !might_be_ligature {
                continue;
            }
            let Some(components) = anchors.iter().filter_map(|a| a.ligature_index()).max() else { continue };
            // Every group of a glyph gets a slot for each component up to the highest number the
            // glyph uses, so the lengths agree.
            let mut by_group: BTreeMap<&str, Vec<Option<&Anchor>>> = BTreeMap::new();
            for anchor in anchors {
                let AnchorKind::Ligature { group, index } = &anchor.kind else { continue };
                ligature_names.insert(group);
                let slots = match by_group.entry(group) {
                    Entry::Occupied(slots) => slots.into_mut(),
                    Entry::Vacant(entry) => {
                        budget.spend(components)?;
                        entry.insert(vec![None; components])
                    }
                };
                if let Some(slot) = index.checked_sub(1).and_then(|k| slots.get_mut(k)) {
                    *slot = Some(*anchor);
                }
            }
            for (group, slots) in by_group {
                groups.entry(group.to_string()).or_default().bases.push((*gid, BaseOrLig::Ligature(slots)));
            }
        }
        for (gid, anchors) in &self.anchors {
            if !self.mark_glyphs.contains(gid) {
                continue;
            }
            for anchor in anchors {
                if let AnchorKind::Mark(group) = &anchor.kind
                    && ligature_names.contains(group.as_str())
                {
                    groups.entry(group.clone()).or_default().marks.push((*gid, *anchor));
                }
            }
        }
        Ok(groups)
    }

    /// One lookup per name of `groups` that `names` accepts and that has marks and bases in
    /// `include`.
    fn lookups_of(
        groups: &BTreeMap<String, MarkGroup>,
        attachment: Attachment,
        feature: Tag,
        include: &GlyphSet,
        names: impl Fn(&str) -> bool,
        budget: &mut Budget,
    ) -> Result<Vec<MarkLookup>, FeatureError> {
        let mut out = Vec::new();
        for (name, group) in groups {
            if !names(name) {
                continue;
            }
            // The ligature slots are copied twice below (the filtered group and the rules).
            let slots: usize = group
                .bases
                .iter()
                .filter(|(gid, _)| include.contains(*gid))
                .map(|(_, b)| match b {
                    BaseOrLig::Ligature(slots) => slots.len(),
                    BaseOrLig::Base(_) => 0,
                })
                .fold(0, usize::saturating_add);
            budget.spend(slots.saturating_mul(2))?;
            let Some(group) = group.only_using_glyphs(include) else { continue };
            budget.spend(group.marks.len().saturating_add(group.bases.len()))?;
            let filter_set = group.filter_set(include);
            let flags = if filter_set.is_some() { LookupFlag::USE_MARK_FILTERING_SET } else { LookupFlag::empty() };
            let marks: Vec<(GlyphId16, Point16)> = group.marks.iter().map(|(gid, a)| (*gid, a.point())).collect();
            let bases = || {
                group.bases.iter().filter_map(|(gid, b)| match b {
                    BaseOrLig::Base(a) => Some((*gid, a.point())),
                    BaseOrLig::Ligature(_) => None,
                })
            };
            let rules = match attachment {
                Attachment::Base => MarkRules::MarkToBase { marks, bases: bases().collect() },
                Attachment::Mark => MarkRules::MarkToMark { marks, bases: bases().collect() },
                Attachment::Ligature => MarkRules::MarkToLigature {
                    marks,
                    ligatures: group
                        .bases
                        .iter()
                        .filter_map(|(gid, b)| match b {
                            BaseOrLig::Ligature(slots) => Some((*gid, slots.iter().map(|a| a.map(Anchor::point)).collect())),
                            BaseOrLig::Base(_) => None,
                        })
                        .collect(),
                },
            };
            out.push(MarkLookup { feature, class: name.clone(), flags, filter_set, rules });
        }
        Ok(out)
    }

    /// The lookups of the three attachment types: mark-to-base and mark-to-ligature in
    /// `feature`, mark-to-mark in `mark_feature`.
    fn lookups(
        groups: &Groups<'_>,
        feature: Tag,
        mark_feature: Tag,
        include: &GlyphSet,
        names: impl Fn(&str) -> bool + Copy,
        budget: &mut Budget,
    ) -> Result<ByType, FeatureError> {
        Ok(ByType {
            base: Self::lookups_of(&groups.base, Attachment::Base, feature, include, names, budget)?,
            lig: Self::lookups_of(&groups.lig, Attachment::Ligature, feature, include, names, budget)?,
            mark: Self::lookups_of(&groups.mark, Attachment::Mark, mark_feature, include, names, budget)?,
        })
    }

    /// The `curs` lookups: left-to-right glyphs, then the rest right to left, both skipping
    /// marks. None unless the font has both an `entry` and an `exit` anchor.
    fn cursive_lookups(&self, char_map: &HashMap<u32, GlyphId16>, glyph_count: usize) -> Vec<MarkLookup> {
        let kinds = || self.anchors.values().flatten().map(|a| &a.kind);
        if !kinds().any(|k| *k == AnchorKind::CursiveEntry) || !kinds().any(|k| *k == AnchorKind::CursiveExit) {
            return Vec::new();
        }
        let mut ltr_glyphs = GlyphSet::none(glyph_count);
        for (cp, gid) in char_map {
            if script_direction(*cp) == Some(ScriptDirection::LeftToRight) {
                ltr_glyphs.insert(*gid);
            }
        }
        let mut ltr = Vec::new();
        let mut rtl = Vec::new();
        for (gid, anchors) in &self.anchors {
            let mut entry = None;
            let mut exit = None;
            for anchor in anchors {
                match anchor.kind {
                    AnchorKind::CursiveEntry => entry = Some(anchor.point()),
                    AnchorKind::CursiveExit => exit = Some(anchor.point()),
                    _ => {}
                }
            }
            if entry.is_none() && exit.is_none() {
                continue;
            }
            (if ltr_glyphs.contains(*gid) { &mut ltr } else { &mut rtl }).push((*gid, entry, exit));
        }
        [(ltr, LookupFlag::empty()), (rtl, LookupFlag::RIGHT_TO_LEFT)]
            .into_iter()
            .filter(|(glyphs, _)| !glyphs.is_empty())
            .map(|(glyphs, flags)| MarkLookup {
                feature: CURS,
                class: String::new(),
                flags: LookupFlag::IGNORE_MARKS | flags,
                filter_set: None,
                rules: MarkRules::Cursive { glyphs },
            })
            .collect()
    }
}

/// The writing direction of a character's script (its Script property, not the extensions);
/// `None` for the common and inherited scripts.
fn script_direction(cp: u32) -> Option<ScriptDirection> {
    let script = script_of(cp)?;
    if script == COMMON_SCRIPT || script == INHERITED_SCRIPT {
        return None;
    }
    Some(if ScriptDirection::for_script(&script) == ScriptDirection::RightToLeft {
        ScriptDirection::RightToLeft
    } else {
        ScriptDirection::LeftToRight
    })
}

/// The Script property of `cp`. Characters new in Unicode 18, which the ICU data does not know,
/// take their one script extension.
fn script_of(cp: u32) -> Option<ScriptCode> {
    use icu_properties::script::ScriptWithExtensions;
    use icu_properties::{PropertyNamesShort, props::Script};
    let script = ScriptWithExtensions::new().get_script_val32(cp);
    if script != Script::Unknown {
        return PropertyNamesShort::<Script>::new().get(script).and_then(ScriptCode::parse);
    }
    let mut extensions = unicode_script_extensions(cp);
    match (extensions.next(), extensions.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// `abvm` and `blwm` take every name but `bottom…` and `nukta…` above, those below.
fn is_below_mark(name: &str) -> bool {
    name.starts_with("bottom") || name.starts_with("nukta")
}

/// The glyphs of `abvm`/`blwm` and those of `mark`/`mkmk`; a glyph may be in both. Each side
/// also gets the glyphs its code points' glyphs reach through `gsub`.
fn split_abvm(char_map: &HashMap<u32, GlyphId16>, glyph_count: usize, language_systems: &[(Tag, Tag)], gsub: Option<&Gsub>) -> (GlyphSet, GlyphSet) {
    // The Indic scripts, the Universal Shaping Engine's and Khmer.
    let myanmar = ScriptCode::parse("Mymr");
    let abvm_scripts: HashSet<ScriptCode> = dist_feature_enabled_scripts().into_iter().filter(|s| Some(*s) != myanmar).collect();
    // With language systems in the feature code, only the declared scripts count as abvm scripts.
    let declared: HashSet<Tag> = language_systems.iter().map(|(script, _)| *script).filter(|t| *t != DFLT_SCRIPT).collect();
    let filtered: HashSet<ScriptCode> = if declared.is_empty() {
        abvm_scripts.clone()
    } else {
        abvm_scripts.iter().filter(|s| script_to_ot_tags(s).any(|t| declared.contains(&t))).copied().collect()
    };
    let is_abvm = |cp: u32| -> Option<bool> {
        let mut saw = false;
        for script in unicode_script_extensions(cp) {
            if script == COMMON_SCRIPT {
                return None;
            }
            saw |= filtered.contains(&script);
        }
        Some(saw)
    };
    if !char_map.keys().any(|cp| is_abvm(*cp) == Some(true)) {
        return (GlyphSet::none(glyph_count), GlyphSet::all(glyph_count));
    }
    let mut abvm = GlyphSet::none(glyph_count);
    let mut other = GlyphSet::none(glyph_count);
    for gid in glyphs_matching(char_map, is_abvm, gsub).iter() {
        abvm.insert(gid);
    }
    // The other side counts every script that is not an abvm script, declared or not.
    for gid in glyphs_matching(char_map, |cp| Some(unicode_script_extensions(cp).any(|s| !abvm_scripts.contains(&s))), gsub).iter() {
        other.insert(gid);
    }
    for (i, slot) in other.0.iter_mut().enumerate() {
        if !abvm.0.get(i).copied().unwrap_or(false) {
            *slot = true;
        }
    }
    (abvm, other)
}

/// A language-system tag from feature code: up to four printable ASCII characters, padded with
/// spaces (`TRK` is `TRK `).
fn tag_of(text: &str) -> Option<Tag> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
        return None;
    }
    let mut raw = [b' '; 4];
    for (slot, b) in raw.iter_mut().zip(bytes) {
        *slot = *b;
    }
    Some(Tag::new(&raw))
}

/// The lookups, classes and carets of `req.master` as plain data ([`GeneratedMarks`]).
pub fn generate_marks(req: &MarkRequest, warnings: &mut Vec<String>) -> Result<GeneratedMarks, FeatureError> {
    let mut budget = Budget::new(req.limits.work);
    let tags = req.tags;
    let glyph_count = req.input.glyphs.len().min(usize::from(u16::MAX));

    let all = read_anchors(req, &mut budget, warnings)?;
    let classes = gdef::glyph_classes(req.input, &all, warnings);
    let carets = if tags.gdef { gdef::ligature_carets(&all) } else { BTreeMap::new() };

    let mut char_map: HashMap<u32, GlyphId16> = HashMap::new();
    for (i, g) in req.input.glyphs.iter().enumerate().take(glyph_count) {
        let Ok(id) = u16::try_from(i) else { break };
        budget.spend(g.codepoints.len())?;
        for cp in &g.codepoints {
            char_map.entry(*cp).or_insert(GlyphId16::new(id));
        }
    }
    let mut language_systems: Vec<(Tag, Tag)> = req.input.language_systems.iter().filter_map(|(s, l)| Some((tag_of(s)?, tag_of(l)?))).collect();
    language_systems.sort_unstable();
    language_systems.dedup();
    if language_systems.is_empty() {
        language_systems.push((DFLT_SCRIPT, DFLT_LANG));
    }

    let writer = MarkWriter::new(&all, classes.clone(), &mut budget)?;
    let groups = Groups { base: writer.mark_to_base_groups(), lig: writer.mark_to_ligature_groups(&mut budget)?, mark: writer.mark_to_mark_groups() };
    let gsub: Option<Gsub> = req.gsub.and_then(|bytes| read_fonts::FontRead::read(read_fonts::FontData::new(bytes)).ok());
    let (abvm_glyphs, mark_glyphs) = split_abvm(&char_map, glyph_count, &language_systems, gsub.as_ref());

    let mut mark_mkmk =
        if tags.mark || tags.mkmk { MarkWriter::lookups(&groups, MARK, MKMK, &mark_glyphs, |_| true, &mut budget)? } else { ByType::default() };
    if !tags.mark {
        mark_mkmk.base.clear();
        mark_mkmk.lig.clear();
    }
    if !tags.mkmk {
        mark_mkmk.mark.clear();
    }
    let (abvm, blwm) = if abvm_glyphs.is_empty() {
        (ByType::default(), ByType::default())
    } else {
        let abvm =
            if tags.abvm { MarkWriter::lookups(&groups, ABVM, ABVM, &abvm_glyphs, |n| !is_below_mark(n), &mut budget)? } else { ByType::default() };
        let blwm = if tags.blwm { MarkWriter::lookups(&groups, BLWM, BLWM, &abvm_glyphs, is_below_mark, &mut budget)? } else { ByType::default() };
        (abvm, blwm)
    };
    let curs = if tags.curs { writer.cursive_lookups(&char_map, glyph_count) } else { Vec::new() };

    // The order of the reference compilers: curs, then the mark writer's features in tag order,
    // abvm and blwm with their mark-to-mark lookups inside, mark-to-mark of `mark` in `mkmk`.
    let mut lookups = curs;
    for by_type in [abvm, blwm] {
        lookups.extend(by_type.base);
        lookups.extend(by_type.lig);
        lookups.extend(by_type.mark);
    }
    lookups.extend(mark_mkmk.base);
    lookups.extend(mark_mkmk.lig);
    lookups.extend(mark_mkmk.mark);
    Ok(GeneratedMarks { lookups, language_systems, classes: if tags.gdef { classes } else { BTreeMap::new() }, carets })
}

/// The mark attachment, cursive attachment and `GDEF` of `req.master`.
pub fn mark_features(req: &MarkRequest) -> Result<MarkFeatures, FeatureError> {
    let mut warnings = Vec::new();
    let generated = generate_marks(req, &mut warnings)?;
    let (gpos, filter_sets) = build_gpos(&generated)?;
    let gdef = gdef::build(&generated.classes, &generated.carets, filter_sets);
    let lookups = if gpos.is_some() { generated.lookups.len() } else { 0 };
    let curs_lookups = generated.lookups.iter().take_while(|l| matches!(l.rules, MarkRules::Cursive { .. })).count();
    Ok(MarkFeatures { gpos, gdef, lookups, curs_lookups, generated, warnings })
}

/// The filter sets of the lookups, numbered in the order the lookups ask for them, each set once.
#[derive(Default)]
struct FilterSets {
    ids: BTreeMap<Vec<GlyphId16>, u16>,
    sets: Vec<Vec<GlyphId16>>,
}

impl FilterSets {
    fn id(&mut self, set: &[GlyphId16]) -> Result<u16, FeatureError> {
        if let Some(id) = self.ids.get(set) {
            return Ok(*id);
        }
        let id = u16::try_from(self.sets.len()).map_err(|_| too_many("mark filtering sets"))?;
        self.ids.insert(set.to_vec(), id);
        self.sets.push(set.to_vec());
        Ok(id)
    }
}

fn too_many(what: &'static str) -> FeatureError {
    FeatureError::TooMany { what, limit: usize::from(u16::MAX) }
}

fn anchor((x, y): Point16) -> AnchorBuilder {
    AnchorBuilder::new(x, y)
}

fn lookup<T, U>(builder: T, flags: LookupFlag, set: Option<u16>, vs: &mut VariationStoreBuilder) -> write_fonts::tables::layout::Lookup<U>
where
    T: Builder<Output = Vec<U>> + Default,
    U: Default,
{
    LookupBuilder::new_with_lookups(flags, set, vec![builder]).build(vs)
}

/// One lookup written to the table's form. Rules the table builders cannot take are left out
/// rather than written wrong: bases of a lookup without marks, a ligature listed twice.
fn position_lookup(l: &MarkLookup, sets: &mut FilterSets, vs: &mut VariationStoreBuilder) -> Result<PositionLookup, FeatureError> {
    let set = l.filter_set.as_deref().map(|s| sets.id(s)).transpose()?;
    let class = l.class.as_str();
    Ok(match &l.rules {
        MarkRules::MarkToBase { marks, bases } => {
            let mut b = MarkToBaseBuilder::default();
            // The builder numbers classes from the marks, so bases come after them.
            for (gid, p) in marks {
                let _ = b.insert_mark(*gid, class, anchor(*p));
            }
            if !marks.is_empty() {
                for (gid, p) in bases {
                    b.insert_base(*gid, class, anchor(*p));
                }
            }
            PositionLookup::MarkToBase(lookup(b, l.flags, set, vs))
        }
        MarkRules::MarkToMark { marks, bases } => {
            let mut b = MarkToMarkBuilder::default();
            for (gid, p) in marks {
                let _ = b.insert_mark1(*gid, class, anchor(*p));
            }
            if !marks.is_empty() {
                for (gid, p) in bases {
                    b.insert_mark2(*gid, class, anchor(*p));
                }
            }
            PositionLookup::MarkToMark(lookup(b, l.flags, set, vs))
        }
        MarkRules::MarkToLigature { marks, ligatures } => {
            let mut b = MarkToLigBuilder::default();
            for (gid, p) in marks {
                let _ = b.insert_mark(*gid, class, anchor(*p));
            }
            if !marks.is_empty() {
                let mut seen = HashSet::new();
                for (gid, slots) in ligatures {
                    if seen.insert(*gid) {
                        b.insert_ligature(*gid, class, slots.iter().map(|p| p.map(anchor)).collect());
                    }
                }
            }
            PositionLookup::MarkToLig(lookup(b, l.flags, set, vs))
        }
        MarkRules::Cursive { glyphs } => {
            let mut b = CursivePosBuilder::default();
            for (gid, entry, exit) in glyphs {
                b.insert(*gid, entry.map(anchor), exit.map(anchor));
            }
            PositionLookup::Cursive(lookup(b, l.flags, set, vs))
        }
    })
}

/// The table of the generated lookups, with each feature registered for the language systems,
/// and the mark filtering sets the lookups use.
fn build_gpos(generated: &GeneratedMarks) -> Result<(Option<Gpos>, Vec<Vec<GlyphId16>>), FeatureError> {
    if generated.lookups.is_empty() {
        return Ok((None, Vec::new()));
    }
    let mut vs = VariationStoreBuilder::new(0);
    let mut sets = FilterSets::default();
    let mut lookups = Vec::with_capacity(generated.lookups.len());
    let mut features: BTreeMap<Tag, Vec<u16>> = BTreeMap::new();
    for l in &generated.lookups {
        let index = u16::try_from(lookups.len()).map_err(|_| too_many("lookups"))?;
        features.entry(l.feature).or_default().push(index);
        lookups.push(position_lookup(l, &mut sets, &mut vs)?);
    }
    let systems: &[(Tag, Tag)] = if generated.language_systems.is_empty() { &[(DFLT_SCRIPT, DFLT_LANG)] } else { &generated.language_systems };
    let mut keys: BTreeMap<(Tag, Tag, Tag), Vec<u16>> = BTreeMap::new();
    for (tag, indices) in &features {
        for (script, lang) in systems {
            keys.insert((*tag, *lang, *script), indices.clone());
        }
    }
    Ok((Some(gpos_from_keys(keys, lookups)?), sets.sets))
}

/// A `GPOS` table from feature keys (feature, language, script) and their lookup indices, with the
/// records built as fea-rs builds them (see `gpos.rs`): one feature record per tag and list of
/// lookups, numbered in key order; scripts and languages sorted, `dflt` the default language.
pub fn gpos_from_keys(keys: BTreeMap<(Tag, Tag, Tag), Vec<u16>>, lookups: Vec<PositionLookup>) -> Result<Gpos, FeatureError> {
    let mut features: BTreeMap<(Tag, Vec<u16>), u16> = BTreeMap::new();
    let mut scripts: BTreeMap<Tag, BTreeMap<Tag, Vec<u16>>> = BTreeMap::new();
    for ((feature, lang, script), indices) in keys {
        let next = u16::try_from(features.len()).map_err(|_| too_many("features"))?;
        let idx = *features.entry((feature, indices)).or_insert(next);
        scripts.entry(script).or_default().entry(lang).or_default().push(idx);
    }
    let mut records: Vec<Option<FeatureRecord>> = vec![None; features.len()];
    for ((tag, indices), idx) in features {
        if let Some(slot) = records.get_mut(usize::from(idx)) {
            *slot = Some(FeatureRecord::new(tag, Feature::new(None, indices)));
        }
    }
    let feature_list = FeatureList::new(records.into_iter().flatten().collect());
    let script_records = scripts
        .into_iter()
        .map(|(script_tag, langs)| {
            let mut script = Script::default();
            for (lang_tag, indices) in langs {
                let lang_sys = LangSys::new(indices);
                if lang_tag == DFLT_LANG {
                    script.default_lang_sys = lang_sys.into();
                } else {
                    script.lang_sys_records.push(LangSysRecord::new(lang_tag, lang_sys));
                }
            }
            ScriptRecord::new(script_tag, script)
        })
        .collect();
    Ok(Gpos::new(ScriptList::new(script_records), feature_list, PositionLookupList::new(lookups)))
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;

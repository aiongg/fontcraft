//! The kern writer: rules split into lookups by script and writing direction, pairs with marks
//! in lookups of their own, and the lookups assigned to the `kern` and `dist` features of each
//! script.
//!
//! Ported from fontc (https://github.com/googlefonts/fontc), `fontbe/src/features/kern.rs`,
//! Apache-2.0, at commit 4504578c3f74c2739f7e55cae133bbfe773f6d8c (`finalize_kerning`,
//! `KernSplitContext`, `merge_scripts`, `split_lookups_by_script`, `assign_lookups_to_scripts`,
//! `prune_unreferenced_lookups`). fontc follows ufo2ft's `kernFeatureWriter.py`.
//!
//! The feature code's language systems and its `GSUB` closure ([`KernCode`]) come from the
//! compiler's first pass, as in fontc. Each lookup keeps its rules in the compiler's neutral form
//! too ([`fontcraft_fea::GlyphPair`], [`fontcraft_fea::ClassPair`]), for the feature compiler.
//! The `GPOS` written directly ([`crate::kern_feature`]) has no `GDEF`, so there a lookup that
//! skips marks ignores every mark; in the neutral form it gets fontc's mark filtering set of the
//! spacing marks when the font has any.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, btree_map};

use read_fonts::collections::IntSet;
use read_fonts::tables::gsub::Gsub;
use write_fonts::tables::gpos::builders::{PairPosBuilder, ValueRecordBuilder};
use write_fonts::tables::layout::LookupFlag;
use write_fonts::types::{GlyphId16, Tag};

use crate::properties::{
    Bidi, COMMON_SCRIPT, DFLT_LANG, DFLT_SCRIPT, INHERITED_SCRIPT, ScriptCode, ScriptDirection, dist_feature_enabled_scripts, glyphs_by_bidi_class,
    guess_font_scripts, script_to_ot_tags, scripts_by_glyph,
};
use crate::resolve::{KernPair, KernSide};
use crate::{Budget, FeatureError, KernLimits, KernTags, MarkSpacing};

pub(crate) const KERN: Tag = Tag::new(b"kern");
pub(crate) const DIST: Tag = Tag::new(b"dist");
pub(crate) const VKRN: Tag = Tag::new(b"vkrn");

/// The lookups of each group of scripts.
type LookupsByScripts = BTreeMap<BTreeSet<ScriptCode>, Vec<PendingLookup>>;
/// The rules between bases, and those with marks.
type SplitPairs<'b> = (Vec<Cow<'b, KernPair>>, Vec<Cow<'b, KernPair>>);

/// A feature in one script and language system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct FeatureKey {
    pub(crate) feature: Tag,
    pub(crate) language: Tag,
    pub(crate) script: Tag,
}

/// A lookup before it is written: its rules and flags, and the same rules in neutral form.
#[derive(Debug)]
pub(crate) struct PendingLookup {
    pub(crate) builder: PairPosBuilder,
    pub(crate) flags: LookupFlag,
    /// The lookup skips marks (base pairs); `flags` then has `IGNORE_MARKS`.
    pub(crate) ignore_marks: bool,
    pub(crate) glyph_pairs: Vec<fontcraft_fea::GlyphPair>,
    pub(crate) class_pairs: Vec<fontcraft_fea::ClassPair>,
}

/// The kern writer's output: lookups, and for each feature key the indices of its lookups.
#[derive(Debug, Default)]
pub(crate) struct Kerns {
    pub(crate) lookups: Vec<PendingLookup>,
    pub(crate) features: BTreeMap<FeatureKey, Vec<usize>>,
}

/// What the kern writer reads from the feature code (fontc's first pass).
#[derive(Default)]
pub(crate) struct KernCode<'a> {
    /// The languages of each script the code's `languagesystem` statements name.
    pub(crate) languages: BTreeMap<Tag, Vec<Tag>>,
    pub(crate) gsub: Option<Gsub<'a>>,
}

impl<'a> KernCode<'a> {
    /// From the code's language systems, (script, language) in code order, and its `GSUB` bytes
    /// (a table that cannot be read is left out).
    pub(crate) fn new(language_systems: &[(Tag, Tag)], gsub: Option<&'a [u8]>) -> Self {
        let mut languages: BTreeMap<Tag, Vec<Tag>> = BTreeMap::new();
        for (script, lang) in language_systems {
            languages.entry(*script).or_default().push(*lang);
        }
        let gsub = gsub.and_then(|bytes| read_fonts::FontRead::read(read_fonts::FontData::new(bytes)).ok());
        Self { languages, gsub }
    }
}

/// A value record in the compiler's neutral form; `axes` names the axes of its deltas.
pub(crate) fn neutral_value(v: &ValueRecordBuilder, axes: &[Tag]) -> fontcraft_fea::Value {
    let metric = |m: &Option<write_fonts::tables::layout::builders::Metric>| {
        m.as_ref().map(|m| fontcraft_fea::Metric { default: m.default, deltas: crate::var_layout::neutral_deltas(axes, &m.device_or_deltas) })
    };
    fontcraft_fea::Value {
        x_placement: metric(&v.x_placement),
        y_placement: metric(&v.y_placement),
        x_advance: metric(&v.x_advance),
        y_advance: metric(&v.y_advance),
    }
}

/// What the writer left out, for the compile report.
#[derive(Debug, Default)]
pub(crate) struct Skipped {
    /// Rules whose glyphs mix strong left-to-right and right-to-left characters.
    pub(crate) ambiguous_direction: usize,
    /// Rule parts whose sides belong to scripts of opposite directions.
    pub(crate) mixed_direction: usize,
}

/// The lookups and features of `pairs` and of the `vertical` ones (both sorted). `char_map` maps code points to glyphs;
/// `marks` gives the glyphs of the mark class and whether they have an advance.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_kerning(
    pairs: &[KernPair],
    vertical: &[KernPair],
    code: &KernCode,
    char_map: &HashMap<u32, GlyphId16>,
    marks: HashMap<GlyphId16, MarkSpacing>,
    tags: KernTags,
    limits: &KernLimits,
    budget: &mut Budget,
    skipped: &mut Skipped,
    axes: &[Tag],
) -> Result<Kerns, FeatureError> {
    if pairs.is_empty() && vertical.is_empty() {
        return Ok(Kerns::default());
    }
    let fea_scripts: Vec<Tag> = code.languages.keys().copied().collect();
    let known_scripts = guess_font_scripts(char_map, &fea_scripts);
    let mut split_ctx = KernSplitContext::new(char_map, &known_scripts, marks, *limits, code.gsub.as_ref());
    split_ctx.axes = axes.to_vec();
    let lookups = split_ctx.make_lookups(pairs, budget, skipped)?;
    let (lookups_by_script, lookups) = split_lookups_by_script(lookups);
    let mut features: BTreeMap<FeatureKey, Vec<usize>> = assign_lookups_to_scripts(lookups_by_script.clone(), KERN, &code.languages)
        .into_iter()
        .chain(assign_lookups_to_scripts(lookups_by_script, DIST, &code.languages))
        .filter(|(key, _)| if key.feature == KERN { tags.kern } else { tags.dist })
        .collect();
    dedupe_lookups(&mut features);
    let (lookups, features) = prune_unreferenced_lookups(lookups, features);
    let mut kerns = Kerns { lookups, features };
    if !vertical.is_empty() {
        let lookup = split_ctx.vertical_lookup(vertical, budget)?;
        add_vertical(&mut kerns, lookup, vertical, code, char_map);
    }
    Ok(kerns)
}

/// Adds the vertical kerning's `lookup` as the `vkrn` feature of the default script, of every script
/// and language with generated kerning (shapers read one script's record, so a script of its own must
/// name the feature again), and of the code's language systems of the scripts of the kerned glyphs.
fn add_vertical(kerns: &mut Kerns, lookup: PendingLookup, pairs: &[KernPair], code: &KernCode, char_map: &HashMap<u32, GlyphId16>) {
    let mut kerned: IntSet<GlyphId16> = IntSet::empty();
    for pair in pairs {
        kerned.extend(pair.side1.iter().chain(pair.side2.iter()));
    }
    let scripts: BTreeSet<Tag> =
        char_map.iter().filter(|(_, gid)| kerned.contains(**gid)).flat_map(|(cp, _)| crate::evaluate::script_tags(*cp)).collect();
    let mut systems: BTreeSet<(Tag, Tag)> = kerns.features.keys().map(|key| (key.script, key.language)).collect();
    for (script, languages) in &code.languages {
        if *script == DFLT_SCRIPT || scripts.contains(script) {
            systems.extend(languages.iter().map(|language| (*script, *language)));
        }
    }
    systems.insert((DFLT_SCRIPT, DFLT_LANG));
    let index = kerns.lookups.len();
    kerns.lookups.push(lookup);
    for (script, language) in systems {
        kerns.features.insert(FeatureKey { feature: VKRN, language, script }, vec![index]);
    }
}

/// Each feature's lookups once and in order, as fea-rs leaves them (`dedupe_lookups` in
/// `fea-rs/src/compile/features.rs`). A script whose OpenType tag is the default script's
/// (Unknown, for private-use glyphs) adds its lookups to those the default script already has.
fn dedupe_lookups(features: &mut BTreeMap<FeatureKey, Vec<usize>>) {
    for lookups in features.values_mut() {
        lookups.sort_unstable();
        lookups.dedup();
    }
}

/// Drops the lookups no feature refers to, renumbering those that remain.
fn prune_unreferenced_lookups(
    lookups: Vec<PendingLookup>,
    mut features: BTreeMap<FeatureKey, Vec<usize>>,
) -> (Vec<PendingLookup>, BTreeMap<FeatureKey, Vec<usize>>) {
    let referenced = features.values().flatten().copied().collect::<BTreeSet<_>>();
    if referenced.len() == lookups.len() {
        return (lookups, features);
    }
    let remap = referenced.iter().enumerate().map(|(new_idx, old_idx)| (*old_idx, new_idx)).collect::<HashMap<_, _>>();
    let lookups = lookups.into_iter().enumerate().filter_map(|(idx, lookup)| remap.contains_key(&idx).then_some(lookup)).collect();
    for ids in features.values_mut() {
        *ids = ids.iter().filter_map(|idx| remap.get(idx).copied()).collect();
    }
    (lookups, features)
}

/// Turns a map of `[scripts] -> [lookups]` into a map of `script -> [lookup index]` and the
/// lookups in order.
fn split_lookups_by_script(lookups: BTreeMap<BTreeSet<ScriptCode>, Vec<PendingLookup>>) -> (BTreeMap<ScriptCode, Vec<usize>>, Vec<PendingLookup>) {
    let mut lookups_by_script: BTreeMap<ScriptCode, Vec<usize>> = BTreeMap::new();
    let mut ordered_lookups = Vec::new();
    for (scripts, lookups) in lookups {
        for lookup in lookups {
            let idx = ordered_lookups.len();
            ordered_lookups.push(lookup);
            for script in &scripts {
                lookups_by_script.entry(*script).or_default().push(idx);
            }
        }
    }
    (lookups_by_script, ordered_lookups)
}

/// The lookups of each feature key of `current_feature` (`kern` or `dist`). The default script
/// gets the common lookups and, for `kern`, those of the left-to-right scripts (or, when there
/// are none, of the right-to-left ones); every script gets its own lookups and the common and
/// inherited ones.
fn assign_lookups_to_scripts(
    mut lookups_by_script: BTreeMap<ScriptCode, Vec<usize>>,
    current_feature: Tag,
    fea_langs_by_script: &BTreeMap<Tag, Vec<Tag>>,
) -> BTreeMap<FeatureKey, Vec<usize>> {
    let dflt_langs = vec![DFLT_LANG];
    let dist_enabled_scripts = dist_feature_enabled_scripts();
    let is_kern_feature = current_feature == KERN;

    let mut default_lookups = Vec::new();
    if let Some(common_lookups) = lookups_by_script.get(&COMMON_SCRIPT).filter(|_| is_kern_feature) {
        default_lookups.extend(common_lookups.iter().copied());
    }
    // InDesign does not apply the lookups of a script it does not know to the default script:
    // ufo2ft puts the left-to-right lookups there too.
    if is_kern_feature {
        let (mut ltr_lookups, mut rtl_lookups) = (Vec::new(), Vec::new());
        for (script, lookups) in lookups_by_script.iter().filter(|(script, _)| !dist_enabled_scripts.contains(*script)) {
            match ScriptDirection::for_script(script) {
                ScriptDirection::LeftToRight => ltr_lookups.extend(lookups.iter().copied()),
                ScriptDirection::RightToLeft => rtl_lookups.extend(lookups.iter().copied()),
                ScriptDirection::Auto => (),
            }
        }
        if !ltr_lookups.is_empty() {
            default_lookups.extend(ltr_lookups);
        } else {
            default_lookups.extend(rtl_lookups);
        }
        default_lookups.sort_unstable();
    }

    let mut features = BTreeMap::new();
    if !default_lookups.is_empty() {
        for lang in fea_langs_by_script.get(&DFLT_SCRIPT).unwrap_or(&dflt_langs) {
            features.insert(FeatureKey { feature: current_feature, language: *lang, script: DFLT_SCRIPT }, default_lookups.clone());
        }
    }

    let common_lookups = lookups_by_script.remove(&COMMON_SCRIPT);
    let inherited_lookups = lookups_by_script.remove(&INHERITED_SCRIPT);
    let dflt_lookups = match (common_lookups, inherited_lookups) {
        (Some(mut a), Some(b)) => {
            a.extend(b);
            a.sort_unstable();
            a.dedup();
            a
        }
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => Vec::new(),
    };

    for (script, mut lookups) in lookups_by_script.into_iter().filter(|(script, _)| dist_enabled_scripts.contains(script) != is_kern_feature) {
        lookups.extend(dflt_lookups.iter().copied());
        lookups.sort_unstable();
        lookups.dedup();
        for tag in script_to_ot_tags(&script) {
            for lang in fea_langs_by_script.get(&tag).unwrap_or(&dflt_langs) {
                let key = FeatureKey { feature: current_feature, language: *lang, script: tag };
                match features.entry(key) {
                    btree_map::Entry::Vacant(entry) => {
                        entry.insert(lookups.clone());
                    }
                    btree_map::Entry::Occupied(entry) => entry.into_mut().extend_from_slice(&lookups),
                }
            }
        }
    }
    features
}

/// Everything needed to split rules by script and writing direction.
struct KernSplitContext {
    mark_glyphs: HashMap<GlyphId16, MarkSpacing>,
    glyph_scripts: HashMap<GlyphId16, HashSet<ScriptCode>>,
    bidi_glyphs: BTreeMap<Bidi, IntSet<GlyphId16>>,
    dflt_scripts: HashSet<ScriptCode>,
    common_scripts: HashSet<ScriptCode>,
    limits: KernLimits,
    /// The variable font's axes, which the values' deltas span; empty for a static font.
    axes: Vec<Tag>,
}

/// The distinct classes of one lookup, for the cap on classes per side.
#[derive(Default)]
struct LookupSize {
    first_classes: HashSet<IntSet<GlyphId16>>,
    second_classes: HashSet<IntSet<GlyphId16>>,
}

/// The cells of the class tables `builder` will allocate. Each class subtable holds a record for
/// every first class times every second class plus one (the first side uses class 0). The builder
/// starts a new subtable where a class overlaps one already in the current subtable, so one class
/// can count in several subtables.
fn class_cells(builder: &PairPosBuilder, budget: &mut Budget) -> Result<usize, FeatureError> {
    let mut total = 0usize;
    for subtable in builder.iter_class_subtables() {
        let (mut firsts, mut seconds) = (0usize, HashSet::new());
        let mut last: Option<&IntSet<GlyphId16>> = None;
        // The rules come grouped by first class, each group borrowing the same set.
        for (first, second, _, _) in subtable {
            budget.spend(1)?;
            if !last.is_some_and(|l| std::ptr::eq(l, first)) {
                firsts = firsts.saturating_add(1);
                last = Some(first);
            }
            seconds.insert(second);
        }
        total = total.saturating_add(firsts.saturating_mul(seconds.len().saturating_add(1)));
    }
    Ok(total)
}

/// The rules of one lookup, in the table builder and in neutral form.
#[derive(Default)]
struct Rules {
    builder: PairPosBuilder,
    glyph_pairs: Vec<fontcraft_fea::GlyphPair>,
    class_pairs: Vec<fontcraft_fea::ClassPair>,
    axes: Vec<Tag>,
}

impl Rules {
    fn pair(&mut self, left: GlyphId16, value: &ValueRecordBuilder, right: GlyphId16) {
        self.glyph_pairs.push(fontcraft_fea::GlyphPair { left, right, value: neutral_value(value, &self.axes) });
        self.builder.insert_pair(left, value.clone(), right, Default::default());
    }

    fn classes(&mut self, left: IntSet<GlyphId16>, value: ValueRecordBuilder, right: IntSet<GlyphId16>) {
        self.class_pairs.push(fontcraft_fea::ClassPair {
            left: left.iter().collect(),
            right: right.iter().collect(),
            value: neutral_value(&value, &self.axes),
        });
        self.builder.insert_classes(left, value, right, Default::default());
    }
}

/// What the lookups made so far hand the table builder.
#[derive(Default)]
struct Totals {
    glyph_pairs: usize,
    /// The class-table cells of the lookups already closed.
    class_cells: usize,
}

impl KernSplitContext {
    fn new(
        char_map: &HashMap<u32, GlyphId16>,
        known_scripts: &HashSet<ScriptCode>,
        mark_glyphs: HashMap<GlyphId16, MarkSpacing>,
        limits: KernLimits,
        gsub: Option<&Gsub>,
    ) -> Self {
        Self {
            mark_glyphs,
            glyph_scripts: scripts_by_glyph(char_map, known_scripts, gsub),
            bidi_glyphs: glyphs_by_bidi_class(char_map, gsub),
            dflt_scripts: HashSet::from([COMMON_SCRIPT, INHERITED_SCRIPT]),
            common_scripts: HashSet::from([COMMON_SCRIPT]),
            limits,
            axes: Vec::new(),
        }
    }

    /// The lookups of each set of scripts: the rules between bases first, then those with marks.
    fn make_lookups(&self, pairs: &[KernPair], budget: &mut Budget, skipped: &mut Skipped) -> Result<LookupsByScripts, FeatureError> {
        let (base_pairs, mark_pairs) = self.split_base_and_mark_pairs(pairs, budget)?;
        let mut result = BTreeMap::new();
        let mut totals = Totals::default();
        if !base_pairs.is_empty() {
            result = self.make_split_script_kern_lookups(&base_pairs, false, &mut totals, budget, skipped)?;
        }
        if !mark_pairs.is_empty() {
            for (scripts, lookups) in self.make_split_script_kern_lookups(&mark_pairs, true, &mut totals, budget, skipped)? {
                result.entry(scripts).or_default().extend(lookups);
            }
        }
        Ok(result)
    }

    /// One lookup for each set of scripts the rules belong to.
    fn make_split_script_kern_lookups(
        &self,
        pairs: &[Cow<KernPair>],
        are_marks: bool,
        totals: &mut Totals,
        budget: &mut Budget,
        skipped: &mut Skipped,
    ) -> Result<LookupsByScripts, FeatureError> {
        let mut lookups_by_script = BTreeMap::new();
        let kerning_per_script = self.split_kerns(pairs, budget, skipped)?;
        let mut bidi_buf = BTreeSet::new();
        for (scripts, pairs) in kerning_per_script {
            let Some(first_script) = scripts.first() else { continue };
            let script_is_rtl = ScriptDirection::for_script(first_script) == ScriptDirection::RightToLeft;
            let mut builder = Rules { axes: self.axes.clone(), ..Rules::default() };
            let mut size = LookupSize::default();
            for mut pair in pairs {
                bidi_buf.clear();
                for (direction, glyphs) in &self.bidi_glyphs {
                    if !pair.glyphs_are_disjoint(glyphs) {
                        bidi_buf.insert(*direction);
                    }
                }
                if bidi_buf.contains(&Bidi::LeftToRight) && bidi_buf.contains(&Bidi::RightToLeft) {
                    skipped.ambiguous_direction = skipped.ambiguous_direction.saturating_add(1);
                    continue;
                }
                if script_is_rtl && !bidi_buf.contains(&Bidi::LeftToRight) {
                    pair.make_rtl_compatible();
                }
                self.add_to(pair, &mut builder, &mut size, totals, budget)?;
            }
            // The class tables are allocated when the lookup is built: their cells are counted
            // from the builder's subtables, before that.
            let cells = class_cells(&builder.builder, budget)?;
            if cells > self.limits.class_cells {
                return Err(FeatureError::TooMany { what: "kerning class cells in one lookup", limit: self.limits.class_cells });
            }
            if totals.class_cells.saturating_add(cells) > self.limits.total_class_cells {
                return Err(FeatureError::TooMany { what: "kerning class cells", limit: self.limits.total_class_cells });
            }
            budget.spend(cells)?;
            totals.class_cells = totals.class_cells.saturating_add(cells);
            let lookup = self.make_lookup(builder, !are_marks);
            lookups_by_script.insert(scripts, vec![lookup]);
        }
        Ok(lookups_by_script)
    }

    /// The lookup of the vertical rules, which have one direction and need no split by script; it
    /// skips marks unless a rule names one.
    fn vertical_lookup(&self, pairs: &[KernPair], budget: &mut Budget) -> Result<PendingLookup, FeatureError> {
        let mut builder = Rules { axes: self.axes.clone(), ..Rules::default() };
        let (mut size, mut totals) = (LookupSize::default(), Totals::default());
        let mut marks = false;
        for pair in pairs {
            budget.spend_glyphs(pair.side1.len().saturating_add(pair.side2.len()))?;
            marks |= pair.side1.iter().chain(pair.side2.iter()).any(|gid| self.mark_glyphs.contains_key(&gid));
            self.add_to(pair.clone(), &mut builder, &mut size, &mut totals, budget)?;
        }
        let cells = class_cells(&builder.builder, budget)?;
        let limit = self.limits.class_cells.min(self.limits.total_class_cells);
        if cells > limit {
            return Err(FeatureError::TooMany { what: "vertical kerning class cells", limit });
        }
        budget.spend(cells)?;
        Ok(self.make_lookup(builder, !marks))
    }

    fn make_lookup(&self, rules: Rules, ignore_marks: bool) -> PendingLookup {
        let mut flags = LookupFlag::empty();
        if ignore_marks {
            flags |= LookupFlag::IGNORE_MARKS;
        }
        PendingLookup { builder: rules.builder, flags, ignore_marks, glyph_pairs: rules.glyph_pairs, class_pairs: rules.class_pairs }
    }

    /// The rules of each set of scripts, merged where the sets share a script; each list sorted.
    fn split_kerns(
        &self,
        pairs: &[Cow<KernPair>],
        budget: &mut Budget,
        skipped: &mut Skipped,
    ) -> Result<HashMap<BTreeSet<ScriptCode>, Vec<KernPair>>, FeatureError> {
        let mut kerning_per_script: HashMap<BTreeSet<ScriptCode>, Vec<KernPair>> = HashMap::new();
        for pair in pairs {
            // Class pairs of 0 change nothing.
            if pair.side1.is_group() && pair.side2.is_group() && pair.value.is_all_zeros() {
                continue;
            }
            for (scripts, pair) in self.partition_by_script(pair, budget, skipped)? {
                kerning_per_script.entry(scripts).or_default().push(pair);
            }
            if kerning_per_script.len() > self.limits.script_sets {
                return Err(FeatureError::TooMany { what: "script combinations in the kerning", limit: self.limits.script_sets });
            }
        }
        Ok(merge_scripts(kerning_per_script))
    }

    /// The parts of a rule by the scripts and directions of its glyphs.
    fn partition_by_script(
        &self,
        pair: &KernPair,
        budget: &mut Budget,
        skipped: &mut Skipped,
    ) -> Result<Vec<(BTreeSet<ScriptCode>, KernPair)>, FeatureError> {
        budget.spend_glyphs(pair.side1.len().saturating_add(pair.side2.len()))?;
        let mut resolved_scripts: HashMap<GlyphId16, &HashSet<ScriptCode>> = HashMap::new();
        let mut directions = [BTreeMap::<ScriptDirection, IntSet<GlyphId16>>::new(), BTreeMap::new()];
        for (side, dirs) in [&pair.side1, &pair.side2].into_iter().zip(directions.iter_mut()) {
            for glyph in side.iter() {
                let mut scripts = self.glyph_scripts.get(&glyph).unwrap_or(&self.dflt_scripts);
                // A glyph with the common or inherited script counts as common only.
                if !scripts.is_disjoint(&self.dflt_scripts) {
                    scripts = &self.common_scripts;
                }
                resolved_scripts.insert(glyph, scripts);
                for direction in scripts.iter().map(ScriptDirection::for_script) {
                    dirs.entry(direction).or_default().insert(glyph);
                }
            }
        }
        let [side1_directions, side2_directions] = directions;
        let scripts_of = |glyphs: &IntSet<GlyphId16>| -> HashSet<ScriptCode> {
            glyphs.iter().flat_map(|gid| resolved_scripts.get(&gid).into_iter().flat_map(|s| s.iter().copied())).collect()
        };
        let mut out = Vec::new();
        for (side1_dir, side1) in &side1_directions {
            for (side2_dir, side2) in &side2_directions {
                if !side1_dir.plays_nicely_with(side2_dir) {
                    skipped.mixed_direction = skipped.mixed_direction.saturating_add(1);
                    continue;
                }
                let side1_scripts = scripts_of(side1);
                let side2_scripts = scripts_of(side2);
                let mut scripts = side1_scripts.iter().chain(side2_scripts.iter()).copied().collect::<BTreeSet<_>>();
                if ![&side1_scripts, &side2_scripts].iter().all(|x| x.contains(&COMMON_SCRIPT)) {
                    scripts.remove(&COMMON_SCRIPT);
                }
                if let Some(part) = pair.with_new_glyphs(side1.clone(), side2.clone()) {
                    out.push((scripts, part));
                }
            }
        }
        Ok(out)
    }

    /// The rules between bases, and those with a mark on a side. A group with bases and marks is
    /// split into its bases and its marks.
    fn split_base_and_mark_pairs<'b>(&self, pairs: &'b [KernPair], budget: &mut Budget) -> Result<SplitPairs<'b>, FeatureError> {
        enum Content {
            Empty,
            BasesOnly,
            MarksOnly,
            Mixed,
        }
        fn classify(side: &KernSide, marks: &HashMap<GlyphId16, MarkSpacing>) -> Content {
            side.iter().fold(Content::Empty, |val, gid| match (val, marks.contains_key(&gid)) {
                (Content::Empty, true) | (Content::MarksOnly, true) => Content::MarksOnly,
                (Content::Empty, false) | (Content::BasesOnly, false) => Content::BasesOnly,
                _ => Content::Mixed,
            })
        }
        fn split_glyphs(glyphs: &KernSide, marks: &HashMap<GlyphId16, MarkSpacing>) -> (KernSide, KernSide) {
            match glyphs {
                KernSide::Glyph(gid) if !marks.contains_key(gid) => (KernSide::Glyph(*gid), KernSide::empty()),
                KernSide::Glyph(gid) => (KernSide::empty(), KernSide::Glyph(*gid)),
                KernSide::Group(glyphs) => {
                    let (x, y): (IntSet<_>, IntSet<_>) = glyphs.iter().partition(|gid| !marks.contains_key(gid));
                    (KernSide::Group(x), KernSide::Group(y))
                }
            }
        }

        if self.mark_glyphs.is_empty() {
            return Ok((pairs.iter().map(Cow::Borrowed).collect(), Vec::new()));
        }
        let (mut base_pairs, mut mark_pairs) = (Vec::new(), Vec::new());
        for pair in pairs {
            match (&pair.side1, &pair.side2) {
                (KernSide::Glyph(side1), KernSide::Glyph(side2))
                    if !self.mark_glyphs.contains_key(side1) && !self.mark_glyphs.contains_key(side2) =>
                {
                    base_pairs.push(Cow::Borrowed(pair))
                }
                (KernSide::Glyph(_), KernSide::Glyph(_)) => mark_pairs.push(Cow::Borrowed(pair)),
                (side1, side2) => {
                    budget.spend_glyphs(side1.len().saturating_add(side2.len()))?;
                    match (classify(side1, &self.mark_glyphs), classify(side2, &self.mark_glyphs)) {
                        (Content::Empty, _) | (_, Content::Empty) => continue,
                        (Content::BasesOnly, Content::BasesOnly) => base_pairs.push(Cow::Borrowed(pair)),
                        (Content::BasesOnly, Content::MarksOnly)
                        | (Content::MarksOnly, Content::BasesOnly)
                        | (Content::MarksOnly, Content::MarksOnly) => mark_pairs.push(Cow::Borrowed(pair)),
                        (Content::Mixed, _) | (_, Content::Mixed) => {
                            let (side1_bases, side1_marks) = split_glyphs(side1, &self.mark_glyphs);
                            let (side2_bases, side2_marks) = split_glyphs(side2, &self.mark_glyphs);
                            if !side1_bases.is_empty() && !side2_bases.is_empty() {
                                base_pairs.push(Cow::Owned(KernPair {
                                    side1: side1_bases.clone(),
                                    value: pair.value.clone(),
                                    side2: side2_bases.clone(),
                                }));
                            }
                            for (side1, side2) in [(&side1_bases, &side2_marks), (&side1_marks, &side2_bases), (&side1_marks, &side2_marks)] {
                                if !side1.is_empty() && !side2.is_empty() {
                                    mark_pairs.push(Cow::Owned(KernPair { side1: side1.clone(), value: pair.value.clone(), side2: side2.clone() }));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok((base_pairs, mark_pairs))
    }

    /// Adds a rule to a lookup's builder; a rule between a glyph and a group becomes one glyph
    /// pair per member of the group. The caps on glyph pairs and classes are checked before the
    /// builder is given the rule.
    fn add_to(
        &self,
        pair: KernPair,
        builder: &mut Rules,
        size: &mut LookupSize,
        totals: &mut Totals,
        budget: &mut Budget,
    ) -> Result<(), FeatureError> {
        let limits = &self.limits;
        let mut add_glyph_pairs = |n: u64| -> Result<(), FeatureError> {
            budget.spend_glyphs(n)?;
            let n = usize::try_from(n).unwrap_or(usize::MAX);
            totals.glyph_pairs = totals.glyph_pairs.saturating_add(n);
            if totals.glyph_pairs > limits.glyph_pairs {
                return Err(FeatureError::TooMany { what: "glyph pairs in the kerning lookups", limit: limits.glyph_pairs });
            }
            Ok(())
        };
        let KernPair { side1, side2, value } = pair;
        match (side1, side2) {
            (KernSide::Glyph(side1), KernSide::Glyph(side2)) => {
                add_glyph_pairs(1)?;
                builder.pair(side1, &value, side2);
            }
            (KernSide::Glyph(side1), KernSide::Group(side2)) => {
                add_glyph_pairs(side2.len())?;
                for side2 in side2.iter() {
                    builder.pair(side1, &value, side2);
                }
            }
            (KernSide::Group(side1), KernSide::Glyph(side2)) => {
                add_glyph_pairs(side1.len())?;
                for side1 in side1.iter() {
                    builder.pair(side1, &value, side2);
                }
            }
            (KernSide::Group(side1), KernSide::Group(side2)) => {
                budget.spend_glyphs(side1.len().saturating_add(side2.len()).saturating_add(1))?;
                if !size.first_classes.contains(&side1) {
                    size.first_classes.insert(side1.clone());
                }
                if !size.second_classes.contains(&side2) {
                    size.second_classes.insert(side2.clone());
                }
                if size.first_classes.len().max(size.second_classes.len()) > limits.classes_per_lookup {
                    return Err(FeatureError::TooMany { what: "kerning classes on one side of a lookup", limit: limits.classes_per_lookup });
                }
                builder.classes(side1, value, side2);
            }
        }
        Ok(())
    }
}

/// Merges the sets of scripts that share a script, with their rules; each list of rules sorted.
fn merge_scripts(kerning_per_script: HashMap<BTreeSet<ScriptCode>, Vec<KernPair>>) -> HashMap<BTreeSet<ScriptCode>, Vec<KernPair>> {
    let mut sets = kerning_per_script.keys().cloned().collect::<Vec<_>>();
    let mut buf = Vec::with_capacity(sets.len());
    let mut did_merge = true;
    while did_merge {
        did_merge = false;
        while let Some(mut common) = sets.pop() {
            sets.retain(|scripts| {
                if scripts.is_disjoint(&common) {
                    true
                } else {
                    common.extend(scripts.iter().copied());
                    did_merge = true;
                    false
                }
            });
            buf.push(common);
        }
        std::mem::swap(&mut sets, &mut buf);
    }

    let mut result = sets.iter().map(|set| (set.clone(), Vec::new())).collect::<HashMap<_, _>>();
    for (scripts, pairs) in kerning_per_script {
        let Some(merged) = sets.iter().find(|merged| !merged.is_disjoint(&scripts)) else { continue };
        if let Some(list) = result.get_mut(merged) {
            list.extend(pairs);
        }
    }
    result.values_mut().for_each(|pairs| pairs.sort_unstable());
    result
}

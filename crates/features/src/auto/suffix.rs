//! Features that substitute glyphs by their name suffix (M8.10; `docs/spec/12-feature-code.md`
//! § 8).
//!
//! Rows: `autoFeature.smcp`, `c2sc`, `pcap`, `c2pc`, `sups`, `subs`, `sinf`, `numr`, `dnom`, `case`, `hist`, `swsh`, `titl`, `init`, `medi`, `med2`, `fina`, `fin2`, `fin3`, `fwid`, `hwid`, `pwid`, `qwid`, `twid`, `hkna`, `vkna`, `pkna`, `hojo`, `ital`, `ruby`, `vrt2`, `cvXX`, `ssXX`, `salt`, `glyphName.chainedSuffixes`.
//!
//! Each feature writes one `sub base by base.suffix;` line per suffixed glyph whose base (the
//! name without its last suffix) is an exporting glyph, in the glyph order of the suffixed
//! glyphs, with no lookup and no flag. That is the form of the reference's stored code in the
//! corpus for case, sups, subs, sinf, numr, dnom and ss01 to ss08. Because only the last suffix
//! is removed, chained suffixes build on each other: `h.sc.ss02` is the ss02 form of `h.sc`,
//! which is the smcp form of `h` (`glyphName.chainedSuffixes`). A source glyph gets one target
//! per feature: the one of the earliest suffix in the feature's list, then the earliest glyph.

use std::collections::HashMap;

use super::figures::{Style, figure_sets};
use super::{AutoContext, AutoEntry, AutoGlyph, Generator, MAX_GENERATED_BYTES};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// One substitution `sub source by target;`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rule<'a> {
    pub source: &'a str,
    pub target: &'a str,
    /// Which rule a source keeps when it has several: the lowest.
    pub priority: u16,
    /// Where the line sorts: a position in the glyph order.
    pub order: usize,
}

/// The base of `name` for `suffix` (given without its period): `a` for `a.sc` and `sc`.
pub(super) fn base_of<'n>(name: &'n str, suffix: &str) -> Option<&'n str> {
    let base = name.strip_suffix(suffix)?.strip_suffix('.')?;
    (!base.is_empty()).then_some(base)
}

/// The feature `tag` holding `rules` as single substitutions, one line each; none without rules.
/// Refused: code beyond [`MAX_GENERATED_BYTES`].
pub(super) fn single_entry(tag: &str, rules: Vec<Rule<'_>>) -> Result<Option<AutoEntry>, FeatureError> {
    let mut best: HashMap<&str, Rule<'_>> = HashMap::new();
    for rule in rules {
        let keep = best.get(rule.source).is_none_or(|old| (rule.priority, rule.order) < (old.priority, old.order));
        if keep && rule.source != rule.target {
            best.insert(rule.source, rule);
        }
    }
    if best.is_empty() {
        return Ok(None);
    }
    let mut kept: Vec<Rule<'_>> = best.into_values().collect();
    kept.sort_by(|a, b| (a.order, a.source).cmp(&(b.order, b.source)));
    let mut code = String::new();
    for rule in kept {
        code.push_str("sub ");
        code.push_str(rule.source);
        code.push_str(" by ");
        code.push_str(rule.target);
        code.push_str(";\n");
        if code.len() > MAX_GENERATED_BYTES {
            return Err(FeatureError::Limit { what: "bytes of generated code in one entry", limit: MAX_GENERATED_BYTES });
        }
    }
    Ok(Some(AutoEntry { kind: EntryKind::Feature, name: tag.to_owned(), code, optional: false }))
}

/// Which base glyphs a feature accepts.
#[derive(Clone, Copy, Debug)]
enum Bases {
    Any,
    /// Hiragana and katakana only (hkna, vkna, pkna).
    Kana,
}

/// What a feature adds to its suffix rules.
#[derive(Clone, Copy, Debug)]
enum Extra {
    None,
    /// Names with this ending and no period, on the base before it (`onesuperior`).
    Ending(&'static str),
    /// `space` substituted by this glyph, when the feature has other rules.
    Space(&'static str),
    /// Oldstyle default figures substituted by the lining figures of their width.
    LiningFigures,
}

fn is_kana(glyph: &AutoGlyph) -> bool {
    let kana_code = glyph.codepoints.iter().any(|&c| matches!(c, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F | 0x1B000..=0x1B16F));
    kana_code || matches!(glyph.script.as_deref(), Some("kana" | "hiragana" | "katakana"))
}

/// The rules `sub base by base.suffix;` for `suffixes`, the first matching suffix of each glyph
/// deciding; the priority is the suffix's position in the list.
fn suffix_rules<'a>(cx: &'a AutoContext, suffixes: &[&str], bases: Bases) -> Vec<Rule<'a>> {
    let mut rules = Vec::new();
    for (index, glyph) in cx.glyphs().iter().enumerate() {
        let Some((priority, base)) = suffixes.iter().enumerate().find_map(|(p, s)| base_of(&glyph.name, s).map(|b| (p, b))) else {
            continue;
        };
        let Some(base_glyph) = cx.glyph(base) else { continue };
        if matches!(bases, Bases::Kana) && !is_kana(base_glyph) {
            continue;
        }
        rules.push(Rule { source: &base_glyph.name, target: &glyph.name, priority: u16::try_from(priority).unwrap_or(u16::MAX), order: index });
    }
    rules
}

/// A feature whose rules come from its suffixes alone, plus an optional extra.
struct Suffixes {
    names: &'static [&'static str],
    suffixes: &'static [&'static str],
    bases: Bases,
    extra: Extra,
}

impl Generator for Suffixes {
    fn names(&self) -> &'static [&'static str] {
        self.names
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let Some(&tag) = self.names.first() else { return Ok(Vec::new()) };
        let mut rules = suffix_rules(cx, self.suffixes, self.bases);
        let after_suffixes = u16::try_from(self.suffixes.len()).unwrap_or(u16::MAX);
        match self.extra {
            Extra::None => {}
            Extra::Ending(ending) => {
                for (index, glyph) in cx.glyphs().iter().enumerate() {
                    if glyph.name.contains('.') {
                        continue;
                    }
                    let Some(base) = glyph.name.strip_suffix(ending) else { continue };
                    if let Some(base_glyph) = cx.glyph(base) {
                        rules.push(Rule { source: &base_glyph.name, target: &glyph.name, priority: after_suffixes, order: index });
                    }
                }
            }
            Extra::Space(to) => {
                if !rules.is_empty()
                    && let (Some(space), Some(to)) = (cx.glyph("space"), cx.glyph(to))
                {
                    rules.push(Rule { source: &space.name, target: &to.name, priority: after_suffixes, order: usize::MAX });
                }
            }
            Extra::LiningFigures => {
                for set in figure_sets(cx) {
                    let Some(style) = set.default_style().filter(|s| s.oldstyle()) else { continue };
                    let (Some(source), Some(target)) = (set.form(style), set.form(Style::from_parts(style.tabular(), false))) else {
                        continue;
                    };
                    rules.push(Rule { source: source.name, target: target.name, priority: after_suffixes, order: target.index });
                }
            }
        }
        Ok(single_entry(tag, rules)?.into_iter().collect())
    }
}

/// Whether `glyph` is an uppercase letter: by its subcategory, else by its first code point or
/// the first letter of its name.
fn is_upper(glyph: &AutoGlyph) -> bool {
    match glyph.sub_category.as_deref() {
        Some(sub) => sub == "Uppercase",
        None => match glyph.codepoints.first().and_then(|&c| char::from_u32(c)) {
            Some(c) => c.is_uppercase(),
            None => glyph.name.chars().next().is_some_and(|c| c.is_ascii_uppercase()),
        },
    }
}

/// The glyph of the other case of `glyph`: by code point when it has one, else by the name with
/// its first letter's case changed (`a` and `A`, `aacute` and `Aacute`).
fn counterpart<'a>(cx: &'a AutoContext, by_code: &HashMap<u32, &'a AutoGlyph>, glyph: &AutoGlyph, upper: bool) -> Option<&'a AutoGlyph> {
    if let Some(c) = glyph.codepoints.first().and_then(|&c| char::from_u32(c)) {
        let mut other = if upper { c.to_uppercase().collect::<Vec<_>>() } else { c.to_lowercase().collect() };
        if other.len() == 1
            && let Some(o) = other.pop()
            && let Some(&found) = by_code.get(&u32::from(o))
        {
            return Some(found);
        }
    }
    let mut chars = glyph.name.chars();
    let first = chars.next()?;
    let first: String = if upper { first.to_uppercase().collect() } else { first.to_lowercase().collect() };
    cx.glyph(&format!("{first}{}", chars.as_str()))
}

/// Small or petite capitals: from lowercase letters (smcp, pcap) or from capitals (c2sc, c2pc).
/// Suffixes of the feature's own set come first; a shared-set glyph on a base of the other case
/// maps from that base's counterpart (`sub A by a.sc;` in c2sc).
struct Capitals {
    names: &'static [&'static str],
    /// Own suffixes, then the shared ones.
    suffixes: &'static [&'static str],
    from_capitals: bool,
}

impl Generator for Capitals {
    fn names(&self) -> &'static [&'static str] {
        self.names
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let Some(&tag) = self.names.first() else { return Ok(Vec::new()) };
        let mut by_code: HashMap<u32, &AutoGlyph> = HashMap::new();
        for glyph in cx.glyphs() {
            for &c in &glyph.codepoints {
                by_code.entry(c).or_insert(glyph);
            }
        }
        let mut rules = Vec::new();
        for rule in suffix_rules(cx, self.suffixes, Bases::Any) {
            let Some(base) = cx.glyph(rule.source) else { continue };
            if is_upper(base) == self.from_capitals {
                rules.push(rule);
            } else if let Some(other) = counterpart(cx, &by_code, base, self.from_capitals) {
                // A base of the other case ranks after every suffix of the right case.
                let priority = rule.priority.saturating_add(16);
                rules.push(Rule { source: &other.name, priority, ..rule });
            }
        }
        Ok(single_entry(tag, rules)?.into_iter().collect())
    }
}

const SS_TAGS: [&str; 20] = [
    "ss01", "ss02", "ss03", "ss04", "ss05", "ss06", "ss07", "ss08", "ss09", "ss10", "ss11", "ss12", "ss13", "ss14", "ss15", "ss16", "ss17", "ss18",
    "ss19", "ss20",
];

const CV_TAGS: [&str; 99] = [
    "cv01", "cv02", "cv03", "cv04", "cv05", "cv06", "cv07", "cv08", "cv09", "cv10", "cv11", "cv12", "cv13", "cv14", "cv15", "cv16", "cv17", "cv18",
    "cv19", "cv20", "cv21", "cv22", "cv23", "cv24", "cv25", "cv26", "cv27", "cv28", "cv29", "cv30", "cv31", "cv32", "cv33", "cv34", "cv35", "cv36",
    "cv37", "cv38", "cv39", "cv40", "cv41", "cv42", "cv43", "cv44", "cv45", "cv46", "cv47", "cv48", "cv49", "cv50", "cv51", "cv52", "cv53", "cv54",
    "cv55", "cv56", "cv57", "cv58", "cv59", "cv60", "cv61", "cv62", "cv63", "cv64", "cv65", "cv66", "cv67", "cv68", "cv69", "cv70", "cv71", "cv72",
    "cv73", "cv74", "cv75", "cv76", "cv77", "cv78", "cv79", "cv80", "cv81", "cv82", "cv83", "cv84", "cv85", "cv86", "cv87", "cv88", "cv89", "cv90",
    "cv91", "cv92", "cv93", "cv94", "cv95", "cv96", "cv97", "cv98", "cv99",
];

/// Numbered features whose tag is the suffix (ss01 to ss20, cv01 to cv99): one entry per tag
/// that has rules, in tag order.
struct Numbered {
    names: &'static [&'static str],
}

impl Generator for Numbered {
    fn names(&self) -> &'static [&'static str] {
        self.names
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let slots: HashMap<&str, usize> = self.names.iter().enumerate().map(|(i, &t)| (t, i)).collect();
        let mut per_tag: Vec<Vec<Rule<'_>>> = vec![Vec::new(); self.names.len()];
        for (index, glyph) in cx.glyphs().iter().enumerate() {
            let Some((base, suffix)) = glyph.name.rsplit_once('.') else { continue };
            let Some(&slot) = slots.get(suffix) else { continue };
            if base.is_empty() {
                continue;
            }
            let Some(base_glyph) = cx.glyph(base) else { continue };
            if let Some(rules) = per_tag.get_mut(slot) {
                rules.push(Rule { source: &base_glyph.name, target: &glyph.name, priority: 0, order: index });
            }
        }
        let mut entries = Vec::new();
        for (&tag, rules) in self.names.iter().zip(per_tag) {
            entries.extend(single_entry(tag, rules)?);
        }
        Ok(entries)
    }
}

static C2SC: Capitals = Capitals { names: &["c2sc"], suffixes: &["c2sc", "sc", "smcp"], from_capitals: true };
static SMCP: Capitals = Capitals { names: &["smcp"], suffixes: &["smcp", "sc"], from_capitals: false };
static C2PC: Capitals = Capitals { names: &["c2pc"], suffixes: &["c2pc", "pc", "pcap"], from_capitals: true };
static PCAP: Capitals = Capitals { names: &["pcap"], suffixes: &["pcap", "pc"], from_capitals: false };

const fn plain(names: &'static [&'static str], suffixes: &'static [&'static str]) -> Suffixes {
    Suffixes { names, suffixes, bases: Bases::Any, extra: Extra::None }
}

static SUPS: Suffixes = Suffixes { extra: Extra::Ending("superior"), ..plain(&["sups"], &["sups"]) };
static SUBS: Suffixes = Suffixes { extra: Extra::Ending("inferior"), ..plain(&["subs"], &["subs", "sinf"]) };
static SINF: Suffixes = Suffixes { extra: Extra::Ending("inferior"), ..plain(&["sinf"], &["sinf", "subs"]) };
static DNOM: Suffixes = plain(&["dnom"], &["dnom"]);
static NUMR: Suffixes = plain(&["numr"], &["numr"]);
static HIST: Suffixes = plain(&["hist"], &["hist"]);
static CASE: Suffixes = Suffixes { extra: Extra::LiningFigures, ..plain(&["case"], &["case"]) };
static SALT: Suffixes = plain(&["salt"], &["ss01"]);
static SWSH: Suffixes = plain(&["swsh"], &["swsh"]);
static TITL: Suffixes = plain(&["titl"], &["titl"]);
static INIT: Suffixes = plain(&["init"], &["init"]);
static MEDI: Suffixes = plain(&["medi"], &["medi"]);
static MED2: Suffixes = plain(&["med2"], &["med2"]);
static FINA: Suffixes = plain(&["fina"], &["fina"]);
static FIN2: Suffixes = plain(&["fin2"], &["fin2"]);
static FIN3: Suffixes = plain(&["fin3"], &["fin3"]);
static FWID: Suffixes = Suffixes { extra: Extra::Space("emspace"), ..plain(&["fwid"], &["full", "fullwidth"]) };
static HKNA: Suffixes = Suffixes { bases: Bases::Kana, ..plain(&["hkna"], &["hori"]) };
static HOJO: Suffixes = plain(&["hojo"], &["hojo"]);
static HWID: Suffixes = Suffixes { extra: Extra::Space("enspace"), ..plain(&["hwid"], &["half"]) };
static ITAL: Suffixes = plain(&["ital"], &["italic"]);
static PKNA: Suffixes = Suffixes { bases: Bases::Kana, ..plain(&["pkna"], &["proportional"]) };
static PWID: Suffixes = plain(&["pwid"], &["proportional"]);
static QWID: Suffixes = plain(&["qwid"], &["qartwidth", "quartwidth"]);
static RUBY: Suffixes = plain(&["ruby"], &["ruby"]);
static TWID: Suffixes = plain(&["twid"], &["thirdwidth"]);
static VKNA: Suffixes = Suffixes { bases: Bases::Kana, ..plain(&["vkna"], &["vert"]) };
static VRT2: Suffixes = plain(&["vrt2"], &["vert"]);
static CV: Numbered = Numbered { names: &CV_TAGS };
static SS: Numbered = Numbered { names: &SS_TAGS };

/// This module's generators, in the order of the handbook's table.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![
        &C2SC, &SMCP, &C2PC, &PCAP, &SUPS, &SUBS, &SINF, &DNOM, &NUMR, &HIST, &CASE, &CV, &SS, &SALT, &SWSH, &TITL, &INIT, &MEDI, &MED2, &FINA,
        &FIN2, &FIN3, &FWID, &HKNA, &HOJO, &HWID, &ITAL, &PKNA, &PWID, &QWID, &RUBY, &TWID, &VKNA, &VRT2,
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Features;

    use super::*;

    fn glyph(name: &str) -> AutoGlyph {
        AutoGlyph { name: name.to_owned(), ..AutoGlyph::default() }
    }

    fn coded(name: &str, codepoint: u32, sub_category: &str) -> AutoGlyph {
        AutoGlyph {
            name: name.to_owned(),
            codepoints: vec![codepoint],
            category: Some("Letter".to_owned()),
            sub_category: Some(sub_category.to_owned()),
            script: None,
        }
    }

    fn context(glyphs: Vec<AutoGlyph>) -> AutoContext {
        AutoContext::new(glyphs, Arc::new(Features::default())).unwrap()
    }

    fn names(list: &[&str]) -> AutoContext {
        context(list.iter().map(|n| glyph(n)).collect())
    }

    /// The code `generator` writes for `tag`, if any.
    fn code(generator: &dyn Generator, cx: &AutoContext, tag: &str) -> Option<String> {
        generator.generate(cx).unwrap().into_iter().find(|e| e.name == tag).map(|e| {
            assert_eq!(e.kind, EntryKind::Feature);
            assert!(!e.optional);
            e.code
        })
    }

    #[test]
    fn auto_feature_case_writes_one_line_per_suffixed_glyph() {
        // The corpus's form: `sub X by X.case;` in glyph order, chained suffixes from their base.
        let cx = names(&["periodcentered", "periodcentered.loclCAT", "hyphen", "periodcentered.loclCAT.case", "hyphen.case", "orphan.case"]);
        assert_eq!(code(&CASE, &cx, "case").unwrap(), "sub periodcentered.loclCAT by periodcentered.loclCAT.case;\nsub hyphen by hyphen.case;\n");
        assert_eq!(code(&CASE, &names(&["a", "b"]), "case"), None);
    }

    #[test]
    fn auto_feature_case_adds_lining_figures_over_oldstyle_defaults() {
        let cx = names(&["one", "one.lf", "one.tf", "parenleft", "parenleft.case"]);
        assert_eq!(code(&CASE, &cx, "case").unwrap(), "sub one by one.lf;\nsub parenleft by parenleft.case;\n");
    }

    #[test]
    fn auto_feature_ss_xx_keeps_chained_suffixes_and_separate_sets() {
        let cx = names(&["comma", "comma.dnom", "g", "comma.dnom.ss07", "g.ss01", "g.ss07", "g.ss21", "x.ss02"]);
        let entries = SS.generate(&cx).unwrap();
        let tags: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(tags, ["ss01", "ss07"]);
        assert_eq!(code(&SS, &cx, "ss07").unwrap(), "sub comma.dnom by comma.dnom.ss07;\nsub g by g.ss07;\n");
        assert_eq!(code(&SS, &cx, "ss01").unwrap(), "sub g by g.ss01;\n");
        // salt repeats ss01.
        assert_eq!(code(&SALT, &cx, "salt").unwrap(), "sub g by g.ss01;\n");
    }

    #[test]
    fn auto_feature_cv_xx_one_feature_per_number() {
        let cx = names(&["k", "k.cv01", "a", "a.cv12", "a.cv100"]);
        let entries = CV.generate(&cx).unwrap();
        let found: Vec<(&str, &str)> = entries.iter().map(|e| (e.name.as_str(), e.code.as_str())).collect();
        assert_eq!(found, [("cv01", "sub k by k.cv01;\n"), ("cv12", "sub a by a.cv12;\n")]);
    }

    #[test]
    fn auto_feature_sups_takes_superior_names_and_sups_suffixes() {
        // Oswald's form: figures to the unsuffixed superior names.
        let cx = names(&["one", "two", "a", "onesuperior", "twosuperior", "a.sups", "superior", "x.superior"]);
        assert_eq!(code(&SUPS, &cx, "sups").unwrap(), "sub one by onesuperior;\nsub two by twosuperior;\nsub a by a.sups;\n");
    }

    #[test]
    fn auto_feature_subs_and_sinf_share_one_set() {
        let cx = names(&["a", "zero", "one", "a.subs", "zeroinferior", "one.sinf"]);
        let both = "sub a by a.subs;\nsub zero by zeroinferior;\nsub one by one.sinf;\n";
        assert_eq!(code(&SUBS, &cx, "subs").unwrap(), both);
        assert_eq!(code(&SINF, &cx, "sinf").unwrap(), both);
        assert_eq!(code(&NUMR, &cx, "numr"), None);
    }

    #[test]
    fn auto_feature_numr_and_dnom() {
        let cx = names(&["i", "one", "i.numr", "one.numr", "one.dnom"]);
        assert_eq!(code(&NUMR, &cx, "numr").unwrap(), "sub i by i.numr;\nsub one by one.numr;\n");
        assert_eq!(code(&DNOM, &cx, "dnom").unwrap(), "sub one by one.dnom;\n");
    }

    #[test]
    fn auto_feature_smcp_and_c2sc_with_a_shared_set() {
        let cx = context(vec![
            coded("A", 0x41, "Uppercase"),
            coded("B", 0x42, "Uppercase"),
            coded("a", 0x61, "Lowercase"),
            coded("b", 0x62, "Lowercase"),
            coded("germandbls", 0xDF, "Lowercase"),
            glyph("a.sc"),
            glyph("b.sc"),
            glyph("B.c2sc"),
            glyph("germandbls.sc"),
        ]);
        assert_eq!(code(&SMCP, &cx, "smcp").unwrap(), "sub a by a.sc;\nsub b by b.sc;\nsub germandbls by germandbls.sc;\n");
        // A capital's own `.c2sc` form wins over the shared set.
        assert_eq!(code(&C2SC, &cx, "c2sc").unwrap(), "sub A by a.sc;\nsub B by B.c2sc;\n");
    }

    #[test]
    fn auto_feature_pcap_and_c2pc_without_code_points() {
        let cx = names(&["A", "a", "a.pc", "A.c2pc"]);
        assert_eq!(code(&PCAP, &cx, "pcap").unwrap(), "sub a by a.pc;\n");
        assert_eq!(code(&C2PC, &cx, "c2pc").unwrap(), "sub A by A.c2pc;\n");
    }

    #[test]
    fn auto_feature_fwid_and_hwid_add_the_space_rule() {
        let cx = names(&["space", "emspace", "enspace", "A", "A.full", "B", "B.fullwidth", "A.half"]);
        assert_eq!(code(&FWID, &cx, "fwid").unwrap(), "sub A by A.full;\nsub B by B.fullwidth;\nsub space by emspace;\n");
        assert_eq!(code(&HWID, &cx, "hwid").unwrap(), "sub A by A.half;\nsub space by enspace;\n");
        // No width forms, no feature: the space rule alone does not make one.
        assert_eq!(code(&FWID, &names(&["space", "emspace"]), "fwid"), None);
    }

    #[test]
    fn auto_feature_kana_features_need_kana_bases() {
        let cx = context(vec![
            coded("a-hira", 0x3042, "Other"),
            coded("kanji", 0x4E00, "Other"),
            glyph("a-hira.vert"),
            glyph("kanji.vert"),
            glyph("a-hira.proportional"),
            glyph("kanji.proportional"),
            glyph("a-hira.hori"),
            glyph("kanji.hori"),
        ]);
        assert_eq!(code(&VKNA, &cx, "vkna").unwrap(), "sub a-hira by a-hira.vert;\n");
        assert_eq!(code(&VRT2, &cx, "vrt2").unwrap(), "sub a-hira by a-hira.vert;\nsub kanji by kanji.vert;\n");
        assert_eq!(code(&PKNA, &cx, "pkna").unwrap(), "sub a-hira by a-hira.proportional;\n");
        assert_eq!(code(&PWID, &cx, "pwid").unwrap(), "sub a-hira by a-hira.proportional;\nsub kanji by kanji.proportional;\n");
        assert_eq!(code(&HKNA, &cx, "hkna").unwrap(), "sub a-hira by a-hira.hori;\n");
    }

    #[test]
    fn auto_feature_single_suffix_features() {
        let cx = names(&[
            "a",
            "a.hist",
            "a.swsh",
            "a.titl",
            "a.italic",
            "a.ruby",
            "a.hojo",
            "a.qartwidth",
            "b",
            "b.quartwidth",
            "a.thirdwidth",
            "beh-ar",
            "beh-ar.init",
            "beh-ar.medi",
            "beh-ar.fina",
            "beh-ar.med2",
            "beh-ar.fin2",
            "beh-ar.fin3",
        ]);
        let expect: [(&dyn Generator, &str, &str); 14] = [
            (&HIST, "hist", "sub a by a.hist;\n"),
            (&SWSH, "swsh", "sub a by a.swsh;\n"),
            (&TITL, "titl", "sub a by a.titl;\n"),
            (&ITAL, "ital", "sub a by a.italic;\n"),
            (&RUBY, "ruby", "sub a by a.ruby;\n"),
            (&HOJO, "hojo", "sub a by a.hojo;\n"),
            (&QWID, "qwid", "sub a by a.qartwidth;\nsub b by b.quartwidth;\n"),
            (&TWID, "twid", "sub a by a.thirdwidth;\n"),
            (&INIT, "init", "sub beh-ar by beh-ar.init;\n"),
            (&MEDI, "medi", "sub beh-ar by beh-ar.medi;\n"),
            (&FINA, "fina", "sub beh-ar by beh-ar.fina;\n"),
            (&MED2, "med2", "sub beh-ar by beh-ar.med2;\n"),
            (&FIN2, "fin2", "sub beh-ar by beh-ar.fin2;\n"),
            (&FIN3, "fin3", "sub beh-ar by beh-ar.fin3;\n"),
        ];
        for (generator, tag, want) in expect {
            assert_eq!(code(generator, &cx, tag).as_deref(), Some(want), "{tag}");
        }
    }

    #[test]
    fn every_generator_names_its_tags_and_stays_quiet_without_suffixes() {
        let cx = names(&["a", "b", "space", "emspace"]);
        for generator in generators() {
            assert!(!generator.names().is_empty());
            assert!(generator.generate(&cx).unwrap().is_empty(), "{:?}", generator.names());
        }
    }
}

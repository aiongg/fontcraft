//! Figure sets (M8.10).
//!
//! Rows: `autoFeature.onum`, `lnum`, `tnum`, `pnum`.
//!
//! A figure set is a base name and its forms: the unsuffixed glyph and the glyphs suffixed `.lf`
//! (proportional lining), `.tf` (tabular lining), `.osf` (proportional oldstyle) and `.tosf`
//! (tabular oldstyle). The handbook says the default form carries no figure suffix, so the
//! unsuffixed glyph takes the first style its set leaves free, in the order lf, osf, tf, tosf:
//! with only `.tf` forms, as in the corpus, the default is proportional lining. Each feature maps
//! every form to the form of the other width (tnum, pnum) or the other height (onum, lnum) and
//! keeps the other property; a form without that partner gets no rule. Lines follow the glyph
//! order of the suffixed glyph of each pair, as the reference's stored tnum and pnum code does.

use std::collections::HashMap;

use super::suffix::{Rule, base_of, single_entry};
use super::{AutoContext, AutoEntry, Generator};
use crate::FeatureError;

/// The style of one form of a figure set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    /// Proportional lining (`.lf`).
    Lf,
    /// Tabular lining (`.tf`).
    Tf,
    /// Proportional oldstyle (`.osf`).
    Osf,
    /// Tabular oldstyle (`.tosf`).
    Tosf,
}

impl Style {
    const ALL: [Style; 4] = [Style::Lf, Style::Tf, Style::Osf, Style::Tosf];
    /// The order in which the unsuffixed glyph takes a style its set leaves free.
    const DEFAULT_ORDER: [Style; 4] = [Style::Lf, Style::Osf, Style::Tf, Style::Tosf];

    fn suffix(self) -> &'static str {
        match self {
            Style::Lf => "lf",
            Style::Tf => "tf",
            Style::Osf => "osf",
            Style::Tosf => "tosf",
        }
    }

    fn slot(self) -> usize {
        match self {
            Style::Lf => 0,
            Style::Tf => 1,
            Style::Osf => 2,
            Style::Tosf => 3,
        }
    }

    pub(super) fn tabular(self) -> bool {
        matches!(self, Style::Tf | Style::Tosf)
    }

    pub(super) fn oldstyle(self) -> bool {
        matches!(self, Style::Osf | Style::Tosf)
    }

    pub(super) fn from_parts(tabular: bool, oldstyle: bool) -> Style {
        match (tabular, oldstyle) {
            (false, false) => Style::Lf,
            (true, false) => Style::Tf,
            (false, true) => Style::Osf,
            (true, true) => Style::Tosf,
        }
    }
}

/// One glyph of a figure set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Form<'a> {
    pub name: &'a str,
    /// Its position in the context's glyph order.
    pub index: usize,
    /// The unsuffixed glyph.
    pub default: bool,
}

/// A base name and its figure forms.
#[derive(Clone, Debug, Default)]
pub(super) struct FigureSet<'a> {
    forms: [Option<Form<'a>>; 4],
}

impl<'a> FigureSet<'a> {
    pub(super) fn form(&self, style: Style) -> Option<Form<'a>> {
        self.forms.get(style.slot()).copied().flatten()
    }

    /// The style the unsuffixed glyph takes, when the set has one.
    pub(super) fn default_style(&self) -> Option<Style> {
        Style::ALL.into_iter().find(|s| self.form(*s).is_some_and(|f| f.default))
    }
}

/// The figure sets of `cx`'s glyphs, in the order their first suffixed form appears.
pub(super) fn figure_sets(cx: &AutoContext) -> Vec<FigureSet<'_>> {
    let mut sets: Vec<(&str, FigureSet<'_>)> = Vec::new();
    let mut by_base: HashMap<&str, usize> = HashMap::new();
    for (index, glyph) in cx.glyphs().iter().enumerate() {
        let Some((style, base)) = Style::ALL.into_iter().find_map(|s| base_of(&glyph.name, s.suffix()).map(|b| (s, b))) else {
            continue;
        };
        let at = match by_base.get(base) {
            Some(&at) => at,
            None => {
                sets.push((base, FigureSet::default()));
                let at = sets.len().saturating_sub(1);
                by_base.insert(base, at);
                at
            }
        };
        if let Some((_, set)) = sets.get_mut(at)
            && let Some(slot) = set.forms.get_mut(style.slot())
            && slot.is_none()
        {
            *slot = Some(Form { name: &glyph.name, index, default: false });
        }
    }
    if sets.is_empty() {
        return Vec::new();
    }
    let positions: HashMap<&str, usize> = cx.glyphs().iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    sets.into_iter()
        .map(|(base, mut set)| {
            if let Some(&index) = positions.get(base)
                && let Some(free) = Style::DEFAULT_ORDER.into_iter().find(|s| set.form(*s).is_none())
                && let Some(slot) = set.forms.get_mut(free.slot())
                && let Some(glyph) = cx.glyphs().get(index)
            {
                *slot = Some(Form { name: &glyph.name, index, default: true });
            }
            set
        })
        .collect()
}

/// What a figure feature changes.
#[derive(Clone, Copy, Debug)]
enum Change {
    /// Proportional to tabular (tnum).
    ToTabular,
    /// Tabular to proportional (pnum).
    ToProportional,
    /// Lining to oldstyle (onum).
    ToOldstyle,
    /// Oldstyle to lining (lnum).
    ToLining,
}

impl Change {
    fn target(self, from: Style) -> Option<Style> {
        let (tabular, oldstyle) = (from.tabular(), from.oldstyle());
        match self {
            Change::ToTabular if !tabular => Some(Style::from_parts(true, oldstyle)),
            Change::ToProportional if tabular => Some(Style::from_parts(false, oldstyle)),
            Change::ToOldstyle if !oldstyle => Some(Style::from_parts(tabular, true)),
            Change::ToLining if oldstyle => Some(Style::from_parts(tabular, false)),
            _ => None,
        }
    }
}

/// One figure feature.
struct Figures {
    names: &'static [&'static str],
    change: Change,
}

impl Generator for Figures {
    fn names(&self) -> &'static [&'static str] {
        self.names
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let Some(&tag) = self.names.first() else { return Ok(Vec::new()) };
        let mut rules = Vec::new();
        for set in figure_sets(cx) {
            for style in Style::ALL {
                let Some(source) = set.form(style) else { continue };
                let Some(to) = self.change.target(style) else { continue };
                let Some(target) = set.form(to) else { continue };
                let order = if target.default { source.index } else { target.index };
                rules.push(Rule { source: source.name, target: target.name, priority: 0, order });
            }
        }
        Ok(single_entry(tag, rules)?.into_iter().collect())
    }
}

static ONUM: Figures = Figures { names: &["onum"], change: Change::ToOldstyle };
static LNUM: Figures = Figures { names: &["lnum"], change: Change::ToLining };
static TNUM: Figures = Figures { names: &["tnum"], change: Change::ToTabular };
static PNUM: Figures = Figures { names: &["pnum"], change: Change::ToProportional };

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&ONUM, &TNUM, &PNUM, &LNUM]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Features;

    use super::*;
    use crate::auto::AutoGlyph;

    fn context(names: &[&str]) -> AutoContext {
        let glyphs = names.iter().map(|n| AutoGlyph { name: (*n).to_owned(), ..AutoGlyph::default() }).collect();
        AutoContext::new(glyphs, Arc::new(Features::default())).unwrap()
    }

    fn code(generator: &dyn Generator, cx: &AutoContext) -> Option<String> {
        generator.generate(cx).unwrap().into_iter().next().map(|e| e.code)
    }

    #[test]
    fn auto_feature_tnum_and_pnum_follow_the_corpus_form() {
        let cx = context(&["zero", "one", "zero.tf", "one.tf", "colon", "colon.case", "colon.tf", "colon.case.tf"]);
        assert_eq!(code(&TNUM, &cx).unwrap(), "sub zero by zero.tf;\nsub one by one.tf;\nsub colon by colon.tf;\nsub colon.case by colon.case.tf;\n");
        assert_eq!(code(&PNUM, &cx).unwrap(), "sub zero.tf by zero;\nsub one.tf by one;\nsub colon.tf by colon;\nsub colon.case.tf by colon.case;\n");
        assert_eq!(code(&ONUM, &cx), None);
        assert_eq!(code(&LNUM, &cx), None);
    }

    #[test]
    fn auto_feature_onum_and_lnum_keep_the_width() {
        let cx = context(&["zero", "zero.tf", "zero.osf", "zero.tosf", "dollar", "dollar.osf"]);
        assert_eq!(code(&ONUM, &cx).unwrap(), "sub zero by zero.osf;\nsub zero.tf by zero.tosf;\nsub dollar by dollar.osf;\n");
        assert_eq!(code(&LNUM, &cx).unwrap(), "sub zero.tosf by zero.tf;\nsub zero.osf by zero;\nsub dollar.osf by dollar;\n");
        assert_eq!(code(&TNUM, &cx).unwrap(), "sub zero by zero.tf;\nsub zero.osf by zero.tosf;\n");
    }

    #[test]
    fn oldstyle_default_figures_take_the_free_style() {
        // `.lf` and `.tf` forms: the unsuffixed figure is proportional oldstyle.
        let cx = context(&["one", "one.lf", "one.tf"]);
        let sets = figure_sets(&cx);
        assert_eq!(sets.first().and_then(|s| s.default_style()), Some(Style::Osf));
        assert_eq!(code(&LNUM, &cx).unwrap(), "sub one by one.lf;\n");
        assert_eq!(code(&ONUM, &cx).unwrap(), "sub one.lf by one;\n");
    }

    #[test]
    fn a_figure_form_without_its_base_still_pairs_with_its_partner() {
        let cx = context(&["seven.osf", "seven.tosf"]);
        assert_eq!(code(&TNUM, &cx).unwrap(), "sub seven.osf by seven.tosf;\n");
    }
}

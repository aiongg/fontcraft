//! The `<rules>` of a designspace source that its reader did not turn into alternate layers.
//!
//! The designspace reader keeps the document as read in the font's [`Extra`]; a document whose
//! rules came back as alternate layers is marked (`rules`), and its rules are those layers'.
//! Otherwise the rules are read from the kept document here, as designspaceLib reads them:
//! condition sets of design-space ranges (conditions written directly in a `<rule>` make one
//! set), substitutions in order, and `processing="last"`. A static instance swaps the glyphs of
//! each rule that holds at its location ([`swap_glyphs`], ufo2ft's `process_rules_swaps` and
//! `swap_glyph_names`); a variable font gets them as feature variations
//! (`fontcraft_compile::var::featvar`). Ported from fontTools' `designspaceLib`
//! (`evaluateRule`, `evaluateConditions`; MIT) and ufo2ft's `instantiator` (MIT); see `NOTICE`.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::plist::{Value, xml};
use fontcraft_model::{AxisRule, Extra, Family, Font, Glyph, Shape};

/// Most rules, condition sets per rule, conditions per set and substitutions per rule read.
pub const MAX_RULE_ITEMS: usize = 10_000;

/// One `<rule>`: where it holds and what it substitutes.
#[derive(Clone, Debug, PartialEq)]
pub struct DocRule {
    /// Any of these sets; each is parallel to [`Font::axes`] in design coordinates, an open
    /// bound where a condition leaves it out or names no axis. A rule without sets holds nowhere.
    pub sets: Vec<Vec<AxisRule>>,
    pub subs: Vec<(String, String)>,
}

/// A designspace's `<rules>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocRules {
    pub rules: Vec<DocRule>,
    /// `processing="last"`: the substitutions go to `rclt` in a variable font.
    pub last: bool,
}

/// The kept designspace document's rules, unless the reader made them alternate layers; `None`
/// when there are none or the document does not parse. A condition on an axis the font does not
/// have makes its set hold nowhere.
pub fn document_rules(font: &Font) -> Option<DocRules> {
    let extra: &Extra = font.extra.as_ref();
    if extra.get(Family::Designspace, "rules").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let text = extra.get(Family::Designspace, "document").and_then(Value::as_str)?;
    let doc = xml::parse(text).ok()?;
    let el = doc.root.child("rules")?;
    let mut rules = Vec::new();
    for r in el.elements().filter(|e| e.name() == "rule").take(MAX_RULE_ITEMS) {
        let mut sets = Vec::new();
        for s in r.elements().filter(|e| e.name() == "conditionset").take(MAX_RULE_ITEMS) {
            sets.extend(condition_set(font, s));
        }
        if r.elements().any(|e| e.name() == "condition") {
            sets.extend(condition_set(font, r));
        }
        let subs = r
            .elements()
            .filter(|e| e.name() == "sub")
            .take(MAX_RULE_ITEMS)
            .filter_map(|s| Some((s.attr("name")?.to_string(), s.attr("with")?.to_string())))
            .collect();
        rules.push(DocRule { sets, subs });
    }
    (!rules.is_empty()).then(|| DocRules { rules, last: el.attr("processing") == Some("last") })
}

/// The conditions directly in `parent` as one set; `None` when one names an axis the font does
/// not have or has a bound that is not a finite number.
fn condition_set(font: &Font, parent: &xml::Element) -> Option<Vec<AxisRule>> {
    let mut set = vec![AxisRule::default(); font.axes.len()];
    for c in parent.elements().filter(|e| e.name() == "condition").take(MAX_RULE_ITEMS) {
        let axis = font.axes.iter().position(|a| Some(a.name.as_str()) == c.attr("name"))?;
        let bound = |attr: &str| -> Option<Option<f64>> {
            match c.attr(attr) {
                None => Some(None),
                Some(t) => t.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(Some),
            }
        };
        let slot = set.get_mut(axis)?;
        slot.min = bound("minimum")?;
        slot.max = bound("maximum")?;
    }
    Some(set)
}

/// Whether a set holds at a design `location` (parallel to the font's axes): every bound it
/// states, inclusive.
fn holds(set: &[AxisRule], location: &[f64]) -> bool {
    set.iter().enumerate().all(|(i, r)| {
        let v = fontcraft_model::axes::design_value(location, i);
        r.min.is_none_or(|m| m <= v) && r.max.is_none_or(|m| v <= m)
    })
}

/// The substitutions of the rules that hold at `location`, in rule order, for glyphs `names`
/// has (ufo2ft's `process_rules_swaps`).
pub fn swaps_at(rules: &DocRules, location: &[f64], has: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    rules
        .rules
        .iter()
        .filter(|r| r.sets.iter().any(|s| holds(s, location)))
        .flat_map(|r| r.subs.iter())
        .filter(|(old, new)| old != new && has(old))
        .cloned()
        .collect()
}

/// Swaps glyphs `a` and `b` of a static instance's glyphs as ufo2ft's `swap_glyph_names` does:
/// their master layers' outlines, widths and anchors, every component of either, and their
/// kerning groups; code points stay. Returns `false` (changing nothing) when either is missing.
pub fn swap_glyphs(glyphs: &mut [Arc<Glyph>], a: &str, b: &str) -> bool {
    let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let (Some(&ia), Some(&ib)) = (index.get(a), index.get(b)) else { return false };
    let (Some(ga), Some(gb)) = (glyphs.get(ia).cloned(), glyphs.get(ib).cloned()) else { return false };
    let (Some(la), Some(lb)) = (ga.layers.iter().find(|l| l.is_master_layer()), gb.layers.iter().find(|l| l.is_master_layer())) else {
        return false;
    };
    let (la, lb) = (la.clone(), lb.clone());
    let take = |g: &mut Arc<Glyph>, from: &fontcraft_model::Layer, groups: &fontcraft_model::KernGroups| {
        let g = Arc::make_mut(g);
        g.kern_groups = groups.clone();
        if let Some(l) = g.layers.iter_mut().find(|l| l.is_master_layer()) {
            l.shapes.clone_from(&from.shapes);
            l.width = from.width;
            l.anchors.clone_from(&from.anchors);
        }
    };
    if let Some(g) = glyphs.get_mut(ia) {
        take(g, &lb, &gb.kern_groups);
    }
    if let Some(g) = glyphs.get_mut(ib) {
        take(g, &la, &ga.kern_groups);
    }
    for g in glyphs.iter_mut() {
        let refers = g.layers.iter().any(|l| l.shapes.iter().any(|s| matches!(s, Shape::Component(c) if c.glyph == a || c.glyph == b)));
        if !refers {
            continue;
        }
        for l in &mut Arc::make_mut(g).layers {
            for s in &mut l.shapes {
                if let Shape::Component(c) = s {
                    if c.glyph == a {
                        c.glyph = b.to_string();
                    } else if c.glyph == b {
                        c.glyph = a.to_string();
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_model::{Axis, Layer, Path};

    fn font(doc: &str) -> Font {
        let mut f = Font::new("R");
        Arc::make_mut(&mut f.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
        Arc::make_mut(&mut f.extra).push(Family::Designspace, "document", Value::String(doc.into()));
        f
    }

    #[test]
    fn rules_hold_inclusively_in_design_space_and_swap_glyphs() {
        let doc = r#"<designspace format="5.0"><rules>
<rule name="a"><conditionset><condition name="Weight" minimum="600" maximum="700"/></conditionset><sub name="A" with="A.alt"/></rule>
<rule name="b"><condition name="Width" minimum="1"/><sub name="B" with="B.alt"/></rule>
</rules></designspace>"#;
        let rules = document_rules(&font(doc)).unwrap();
        assert_eq!(rules.rules.len(), 2);
        assert!(rules.rules[1].sets.is_empty(), "a condition on an unknown axis holds nowhere");
        let has = |n: &str| n == "A" || n == "B";
        assert!(swaps_at(&rules, &[599.0], has).is_empty());
        assert_eq!(swaps_at(&rules, &[600.0], has), [("A".to_string(), "A.alt".to_string())]);
        let mut raised = font(doc);
        Arc::make_mut(&mut raised.extra).push(Family::Designspace, "rules", Value::Bool(true));
        assert_eq!(document_rules(&raised), None, "rules the reader made alternate layers are those layers'");

        let glyph = |name: &str, w: f64, shape: Shape| {
            let mut g = Glyph::new(name);
            let mut l = Layer::new("m01", w);
            l.shapes.push(shape);
            g.layers.push(l);
            Arc::new(g)
        };
        let comp = fontcraft_model::Component::new("A", 0.0, 0.0);
        let mut glyphs = vec![
            glyph("A", 500.0, Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0))),
            glyph("A.alt", 700.0, Shape::Path(Path::rect(0.0, 0.0, 20.0, 10.0))),
            glyph("Aacute", 500.0, Shape::Component(comp)),
        ];
        assert!(swap_glyphs(&mut glyphs, "A", "A.alt"));
        assert_eq!((glyphs[0].layers[0].width, glyphs[1].layers[0].width), (700.0, 500.0));
        assert!(matches!(&glyphs[2].layers[0].shapes[0], Shape::Component(c) if c.glyph == "A.alt"), "components follow the outlines");
        assert!(!swap_glyphs(&mut glyphs, "A", "missing"));
    }
}

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
/// Most items read from one document's rules together: each rule, condition and substitution
/// counts one, and each condition set one per axis of the font (what it holds). Reading stops
/// there, as it stops at [`MAX_RULE_ITEMS`] in one list.
pub const MAX_RULE_TOTAL: usize = 100_000;

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
    let mut left = MAX_RULE_TOTAL;
    let set_cost = font.axes.len().max(1);
    for r in el.elements().filter(|e| e.name() == "rule").take(MAX_RULE_ITEMS) {
        let Some(after) = left.checked_sub(1) else { break };
        left = after;
        let mut spent = false;
        let mut sets = Vec::new();
        let direct = r.elements().any(|e| e.name() == "condition").then_some(r);
        for s in r.elements().filter(|e| e.name() == "conditionset").take(MAX_RULE_ITEMS).chain(direct) {
            let Some(after) = left.checked_sub(set_cost) else {
                spent = true;
                break;
            };
            left = after;
            match condition_set(font, s, &mut left) {
                Read::Set(set) => sets.push(set),
                Read::Nowhere => {}
                Read::Spent => {
                    spent = true;
                    break;
                }
            }
        }
        let mut subs = Vec::new();
        for s in r.elements().filter(|e| e.name() == "sub").take(if spent { 0 } else { MAX_RULE_ITEMS }) {
            let Some(after) = left.checked_sub(1) else {
                spent = true;
                break;
            };
            left = after;
            if let (Some(name), Some(with)) = (s.attr("name"), s.attr("with")) {
                subs.push((name.to_string(), with.to_string()));
            }
        }
        rules.push(DocRule { sets, subs });
        if spent {
            break;
        }
    }
    (!rules.is_empty()).then(|| DocRules { rules, last: el.attr("processing") == Some("last") })
}

/// What [`condition_set`] read.
enum Read {
    Set(Vec<AxisRule>),
    /// A condition names an axis the font does not have, or a bound that is not a finite number.
    Nowhere,
    /// The document's budget ran out.
    Spent,
}

/// The conditions directly in `parent` as one set, each charged to `left`.
fn condition_set(font: &Font, parent: &xml::Element, left: &mut usize) -> Read {
    let mut set = vec![AxisRule::default(); font.axes.len()];
    for c in parent.elements().filter(|e| e.name() == "condition").take(MAX_RULE_ITEMS) {
        let Some(after) = left.checked_sub(1) else { return Read::Spent };
        *left = after;
        let Some(axis) = font.axes.iter().position(|a| Some(a.name.as_str()) == c.attr("name")) else { return Read::Nowhere };
        let bound = |attr: &str| -> Option<Option<f64>> {
            match c.attr(attr) {
                None => Some(None),
                Some(t) => t.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(Some),
            }
        };
        let (Some(slot), Some(min), Some(max)) = (set.get_mut(axis), bound("minimum"), bound("maximum")) else { return Read::Nowhere };
        slot.min = min;
        slot.max = max;
    }
    Read::Set(set)
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
    apply_swaps(glyphs, &[(a.to_string(), b.to_string())]).is_empty()
}

/// [`swap_glyphs`] for each pair in order, with one index of the glyphs: the swaps compose into
/// one move of master-layer contents between glyphs and one renaming of components, applied
/// once. Gives the pairs that named a glyph without a master layer, or none, and changed nothing.
pub fn apply_swaps(glyphs: &mut [Arc<Glyph>], swaps: &[(String, String)]) -> Vec<(String, String)> {
    let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let has_master = |i: usize| glyphs.get(i).is_some_and(|g| g.layers.iter().any(|l| l.is_master_layer()));
    // `content[i]`: the glyph whose master-layer contents and kerning groups end up in glyph i.
    let mut content: Vec<usize> = (0..glyphs.len()).collect();
    // A component naming `x` ends up naming `renamed[x]`; `named[y]` is the inverse.
    let mut renamed: HashMap<&str, &str> = HashMap::new();
    let mut named: HashMap<&str, &str> = HashMap::new();
    let mut failed = Vec::new();
    for (a, b) in swaps {
        let (Some(&ia), Some(&ib)) = (index.get(a.as_str()), index.get(b.as_str())) else {
            failed.push((a.clone(), b.clone()));
            continue;
        };
        if !has_master(ia) || !has_master(ib) {
            failed.push((a.clone(), b.clone()));
            continue;
        }
        content.swap(ia, ib);
        let (a, b) = (a.as_str(), b.as_str());
        let (from_a, from_b) = (named.get(a).copied().unwrap_or(a), named.get(b).copied().unwrap_or(b));
        renamed.insert(from_a, b);
        renamed.insert(from_b, a);
        named.insert(b, from_a);
        named.insert(a, from_b);
    }
    let renamed: HashMap<String, String> =
        renamed.into_iter().filter(|(from, to)| from != to).map(|(from, to)| (from.to_string(), to.to_string())).collect();
    let moved: Vec<(usize, fontcraft_model::Layer, fontcraft_model::KernGroups)> = content
        .iter()
        .enumerate()
        .filter(|(i, from)| *i != **from)
        .filter_map(|(i, &from)| {
            let g = glyphs.get(from)?;
            Some((i, g.layers.iter().find(|l| l.is_master_layer())?.clone(), g.kern_groups.clone()))
        })
        .collect();
    for (i, from, groups) in moved {
        let Some(g) = glyphs.get_mut(i) else { continue };
        let g = Arc::make_mut(g);
        g.kern_groups = groups;
        if let Some(l) = g.layers.iter_mut().find(|l| l.is_master_layer()) {
            l.shapes = from.shapes;
            l.width = from.width;
            l.anchors = from.anchors;
        }
    }
    if !renamed.is_empty() {
        for g in glyphs.iter_mut() {
            let refers = g.layers.iter().any(|l| l.shapes.iter().any(|s| matches!(s, Shape::Component(c) if renamed.contains_key(&c.glyph))));
            if !refers {
                continue;
            }
            for l in &mut Arc::make_mut(g).layers {
                for s in &mut l.shapes {
                    if let Shape::Component(c) = s
                        && let Some(to) = renamed.get(&c.glyph)
                    {
                        c.glyph.clone_from(to);
                    }
                }
            }
        }
    }
    failed
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

    #[test]
    fn chained_swaps_compose_as_one_at_a_time() {
        let glyph = |name: &str, w: f64, shapes: Vec<Shape>| {
            let mut g = Glyph::new(name);
            let mut l = Layer::new("m01", w);
            l.shapes = shapes;
            g.layers.push(l);
            Arc::new(g)
        };
        let comp = |n: &str| Shape::Component(fontcraft_model::Component::new(n, 0.0, 0.0));
        let start = vec![
            glyph("A", 100.0, vec![comp("C")]),
            glyph("B", 200.0, Vec::new()),
            glyph("C", 300.0, vec![comp("B")]),
            glyph("D", 400.0, vec![comp("A"), comp("C"), comp("E")]),
        ];
        let swaps: Vec<(String, String)> =
            [("A", "B"), ("B", "C"), ("C", "A"), ("A", "missing")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut one_by_one = start.clone();
        let mut failed = Vec::new();
        for (a, b) in &swaps {
            if !swap_glyphs(&mut one_by_one, a, b) {
                failed.push((a.clone(), b.clone()));
            }
        }
        let mut together = start.clone();
        assert_eq!(apply_swaps(&mut together, &swaps), failed);
        assert_eq!(failed, [("A".to_string(), "missing".to_string())]);
        assert_eq!(together, one_by_one);
        let widths: Vec<f64> = together.iter().map(|g| g.layers[0].width).collect();
        assert_eq!(widths, [100.0, 300.0, 200.0, 400.0], "(A B), (B C) and (C A) leave A in place and B and C swapped");
    }

    #[test]
    fn many_rules_are_read_within_one_budget_and_swapped_in_one_pass() {
        // 40 rules that hold everywhere, 3,000 substitutions each, in a font of 10,000 glyphs:
        // swapping one pair at a time would index and scan every glyph 100,000 times.
        let glyphs_count = 10_000usize;
        let mut doc = String::from(r#"<designspace format="5.0"><rules>"#);
        for r in 0..40 {
            doc.push_str(r#"<rule name="r"><conditionset/>"#);
            for i in 0..3_000 {
                let n = r * 3_000 + i;
                doc.push_str(&format!(r#"<sub name="g{}" with="g{}"/>"#, n % glyphs_count, (n + 1) % glyphs_count));
            }
            doc.push_str("</rule>");
        }
        doc.push_str("</rules></designspace>");
        let rules = document_rules(&font(&doc)).unwrap();
        let read: usize = rules.rules.iter().map(|r| 1 + r.sets.len() + r.subs.len()).sum();
        assert!(read <= MAX_RULE_TOTAL && read > MAX_RULE_TOTAL - 3_100, "{read}");
        let mut glyphs: Vec<Arc<Glyph>> = (0..glyphs_count)
            .map(|i| {
                let mut g = Glyph::new(format!("g{i}"));
                let mut l = Layer::new("m01", i as f64);
                l.shapes.push(Shape::Component(fontcraft_model::Component::new(format!("g{}", (i + 7) % glyphs_count), 0.0, 0.0)));
                g.layers.push(l);
                Arc::new(g)
            })
            .collect();
        let swaps = swaps_at(&rules, &[400.0], |_| true);
        assert!(swaps.len() > 95_000, "{}", swaps.len());
        assert!(apply_swaps(&mut glyphs, &swaps).is_empty());
    }

    #[test]
    fn condition_sets_are_charged_per_axis() {
        // 10,000 empty condition sets in each of 20 rules, in a font of 64 axes: each set holds
        // one bound pair per axis.
        let mut doc = String::from(r#"<designspace format="5.0"><rules>"#);
        for _ in 0..20 {
            doc.push_str(r#"<rule name="r">"#);
            doc.push_str(&"<conditionset/>".repeat(10_000));
            doc.push_str("</rule>");
        }
        doc.push_str("</rules></designspace>");
        let mut f = font(&doc);
        for i in 1..64 {
            Arc::make_mut(&mut f.axes).push(Axis { name: format!("A{i}"), tag: format!("A{i:03}"), ..Default::default() });
        }
        let rules = document_rules(&f).unwrap();
        let sets: usize = rules.rules.iter().map(|r| r.sets.len()).sum();
        assert!(sets <= MAX_RULE_TOTAL / 64, "{sets}");
    }
}

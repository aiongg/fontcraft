//! Composition (M6.3): the components a glyph name is built from, and a new composite glyph.
//!
//! Rows: `components.autoComposeOnAddGlyph`, `components.autoComposeNeedsAllParts`,
//! `components.preferMarkWithSameSuffix`, `components.preferCaseMarksForUppercase`,
//! `components.preferNarrowMarksForIJ`, `components.ijBuiltFromDotless`.
//!
//! Contract:
//! - [`components_for`] maps a name to its parts: the glyph database's decomposition (a custom
//!   glyph data file first, then the Unicode canonical decomposition,
//!   [`fontcraft_glyphdata::GlyphData::canonical_decomposition`]) turned into glyph names, the
//!   name's suffix carried to the parts, the preferred-mark rules, and our own table for what
//!   Unicode does not decompose (i, j, ligatures by name parts).
//! - All or nothing: when a part is missing from the font the result is
//!   [`ComponentsError::MissingParts`] naming every missing part, and nothing is built.
//!   `Ok(None)` means no rule decomposes the name.
//! - [`composite_glyph`] builds the glyph: one master layer per master holding the parts as
//!   components, aligned ([`crate::align`]). It does not add the glyph to the font.
//!
//! The rules, in the order they apply (`docs/spec/09-reusing-shapes.md` § 2.1):
//! 1. A custom glyph data record for the exact name with a `decompose` list is taken as written.
//! 2. The name splits into base and suffix (`agrave.sc` → `agrave`, `sc`). A custom record for
//!    the base, then our table (i and j), then the base's Unicode canonical decomposition, then
//!    the base's ligature parts (`f_f_l` → f, f, l) give the parts without the suffix.
//! 3. The Unicode decomposition is expanded fully (U+01FA → A, ringcomb, acutecomb), except that
//!    a letter with a horn (U+01A0 Ohorn) stays whole when the font has it.
//! 4. Our exceptions: a name with `commaaccent` takes commaaccentcomb where Unicode has the
//!    cedilla, gcommaaccent takes commaturnedabovecomb, and dcaron, lcaron, Lcaron and tcaron
//!    prefer caroncomb.alt.
//! 5. i and j followed by a mark above (combining class 230) become idotless and jdotless.
//! 6. Each part is resolved against the font. A base of a decomposition takes the suffix
//!    (`agrave.sc` needs `a.sc`); a part of a ligature takes it when the font has that glyph. A
//!    mark tries, in order: the suffixed mark, the `.case` mark for an uppercase composite, the
//!    `.i` and `.narrow` marks on a dotless i or j, the plain mark. Two marks in a row become
//!    their mark ligature (`circumflexcomb_gravecomb`) when the font has one.

use std::collections::HashSet;

use fontcraft_glyphdata::{Case, Category, GlyphData, MAX_DECOMPOSITION_DEPTH};
use fontcraft_model::{Component, Font, Glyph, Layer, MAX_NAME_LEN, Shape};

use crate::{ComponentsError, MAX_COMPONENT_DEPTH, MAX_RECIPE_PARTS, align};

/// The combining class of the horn (U+031B): a letter that carries it stays a base.
const CCC_ATTACHED_ABOVE_RIGHT: u8 = 216;
/// The combining class of marks above, which take the dot off i and j.
const CCC_ABOVE: u8 = 230;

/// Where a part list came from; the suffix and mark-ligature rules differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A database or Unicode decomposition: one base, then marks.
    Decomposition,
    /// A ligature by its name parts: every part a base unless it is a mark.
    Ligature,
}

/// One component before it is resolved against the font: the names it may take, most preferred
/// first, without the composite's suffix.
#[derive(Clone, Debug)]
struct Slot {
    names: Vec<String>,
    mark: bool,
}

/// The glyph names `name` is composed of, base first; see the module doc.
pub fn components_for(font: &Font, data: &GlyphData, name: &str) -> Result<Option<Vec<String>>, ComponentsError> {
    if name.is_empty() {
        return Ok(None);
    }
    check_name(name)?;
    if let Some(parts) = custom_decomposition(data, name)? {
        let missing: Vec<String> = dedup(parts.iter().filter(|p| font.glyph(p).is_none()).cloned().collect());
        if !missing.is_empty() {
            return Err(ComponentsError::MissingParts { glyph: name.to_string(), missing });
        }
        return refuse_self(name, parts).map(Some);
    }
    let (base, suffix) = data.split_suffix(name);
    let Some((kind, slots)) = raw_slots(font, data, name, base, suffix)? else {
        return Ok(None);
    };
    resolve(font, data, name, base, suffix, kind, slots).map(Some)
}

/// A new glyph `name` holding `parts` as components on a master layer of every master, aligned.
/// Every part must exist in `font` ([`ComponentsError::MissingParts`] otherwise); the code
/// points come from the glyph database.
///
/// When `font` already has a glyph called `name`, the result is that glyph with its master
/// layers' shapes replaced; its anchors, other layers and metadata stay (Create Composite).
pub fn composite_glyph(font: &Font, data: &GlyphData, name: &str, parts: &[String]) -> Result<Glyph, ComponentsError> {
    check_name(name)?;
    if parts.is_empty() {
        return Err(ComponentsError::BadRecipe(format!("`{name}` needs at least one component")));
    }
    if parts.len() > MAX_RECIPE_PARTS {
        return Err(ComponentsError::LimitExceeded { what: "components in one composite", limit: MAX_RECIPE_PARTS });
    }
    let missing = dedup(parts.iter().filter(|p| font.glyph(p).is_none()).cloned().collect());
    if !missing.is_empty() {
        return Err(ComponentsError::MissingParts { glyph: name.to_string(), missing });
    }
    for part in parts {
        if let Some(path) = path_to(font, part, name) {
            let mut cycle = vec![name.to_string()];
            cycle.extend(path);
            return Err(ComponentsError::Cycle(cycle));
        }
    }
    let marks: Vec<bool> = parts.iter().enumerate().map(|(i, p)| i > 0 && is_mark(font, data, p)).collect();
    let mut glyph = match font.glyph(name) {
        Some(existing) => (**existing).clone(),
        None => {
            let mut g = Glyph::new(name);
            let (_, suffix) = data.split_suffix(name);
            if suffix.is_none()
                && let Some(cp) = data.lookup(name, &[]).unicode.map(|u| u.value)
                && !font.glyphs.as_arcs().iter().any(|other| other.unicodes.contains(&cp))
            {
                g.unicodes = vec![cp];
            }
            g
        }
    };
    for master in font.masters.iter() {
        // Bases side by side, each mark over the base before it; alignment then places them.
        let mut shapes = Vec::with_capacity(parts.len());
        let (mut advance, mut base_x) = (0.0_f64, 0.0_f64);
        for (part, mark) in parts.iter().zip(&marks) {
            if *mark {
                shapes.push(Shape::Component(Component::new(part.as_str(), base_x, 0.0)));
            } else {
                base_x = advance;
                shapes.push(Shape::Component(Component::new(part.as_str(), base_x, 0.0)));
                let width = font.glyph(part).and_then(|g| g.layer(&master.id)).map_or(0.0, |l| l.width);
                if width.is_finite() {
                    advance += width;
                }
            }
        }
        match glyph.layer_mut(&master.id) {
            Some(layer) => {
                layer.shapes = shapes;
                layer.width = advance;
            }
            None => {
                let mut layer = Layer::new(master.id.as_str(), advance);
                layer.shapes = shapes;
                glyph.layers.push(layer);
            }
        }
    }
    align_new_glyph(font, glyph)
}

/// `glyph` with its master layers aligned in a copy of `font` that holds it.
fn align_new_glyph(font: &Font, glyph: Glyph) -> Result<Glyph, ComponentsError> {
    let name = glyph.name.clone();
    let mut scratch = font.clone();
    if scratch.glyphs.get(&name).is_some() {
        if let Some(mut slot) = scratch.glyphs.get_mut(&name) {
            *slot = glyph;
        }
    } else {
        scratch.glyphs.push(glyph);
    }
    let mut aligned: Vec<Layer> = Vec::new();
    for master in font.masters.iter() {
        if let Some(layer) = align::align_layer(&scratch, &name, &master.id)? {
            aligned.push(layer);
        }
    }
    let mut out = scratch.glyphs.get(&name).map(|g| (**g).clone()).ok_or_else(|| ComponentsError::MissingGlyph(name.clone()))?;
    for layer in aligned {
        if let Some(slot) = out.layers.iter_mut().find(|l| l.id == layer.id) {
            *slot = layer;
        }
    }
    Ok(out)
}

/// The chain of glyphs from `from` to a component that names `target`, following components on
/// every layer; `None` when there is none. Visits each glyph once, at most
/// [`MAX_COMPONENT_DEPTH`] deep.
fn path_to(font: &Font, from: &str, target: &str) -> Option<Vec<String>> {
    fn walk(font: &Font, at: &str, target: &str, depth: usize, seen: &mut HashSet<String>, path: &mut Vec<String>) -> bool {
        if depth > MAX_COMPONENT_DEPTH || !seen.insert(at.to_string()) {
            return false;
        }
        path.push(at.to_string());
        if at == target {
            return true;
        }
        if let Some(g) = font.glyph(at) {
            for layer in &g.layers {
                for shape in &layer.shapes {
                    if let Shape::Component(c) = shape
                        && walk(font, &c.glyph, target, depth + 1, seen, path)
                    {
                        return true;
                    }
                }
            }
        }
        path.pop();
        false
    }
    let mut path = Vec::new();
    walk(font, from, target, 0, &mut HashSet::new(), &mut path).then_some(path)
}

fn check_name(name: &str) -> Result<(), ComponentsError> {
    if name.len() > MAX_NAME_LEN {
        return Err(ComponentsError::LimitExceeded { what: "bytes in a glyph name", limit: MAX_NAME_LEN });
    }
    Ok(())
}

fn refuse_self(name: &str, parts: Vec<String>) -> Result<Vec<String>, ComponentsError> {
    if parts.iter().any(|p| p == name) {
        return Err(ComponentsError::Cycle(vec![name.to_string(), name.to_string()]));
    }
    Ok(parts)
}

fn dedup(names: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    names.into_iter().filter(|n| seen.insert(n.clone())).collect()
}

/// The `decompose` list of a custom glyph data record named `name`, when it has a non-empty one.
fn custom_decomposition(data: &GlyphData, name: &str) -> Result<Option<Vec<String>>, ComponentsError> {
    let Some(list) = data.record(name).and_then(|r| r.decompose) else {
        return Ok(None);
    };
    let mut parts = Vec::new();
    for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if parts.len() == MAX_RECIPE_PARTS {
            return Err(ComponentsError::LimitExceeded { what: "components in one composite", limit: MAX_RECIPE_PARTS });
        }
        check_name(part)?;
        parts.push(part.to_string());
    }
    Ok((!parts.is_empty()).then_some(parts))
}

/// `name` with `.suffix` appended, or `name` itself.
fn suffixed(name: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(s) => format!("{name}.{s}"),
        None => name.to_string(),
    }
}

/// The glyph's category: stored in the font, else from the glyph database.
pub(crate) fn category_of(font: &Font, data: &GlyphData, name: &str) -> Option<Category> {
    match font.glyph(name) {
        Some(g) => match g.info.category.as_deref() {
            Some(stored) => Category::parse(stored),
            None => data.lookup(name, &g.unicodes).category.map(|c| c.value),
        },
        None => data.lookup(name, &[]).category.map(|c| c.value),
    }
}

/// The glyph's case: stored in the font, else from the glyph database.
pub(crate) fn case_of(font: &Font, data: &GlyphData, name: &str) -> Option<Case> {
    match font.glyph(name) {
        Some(g) => match g.info.case.as_deref() {
            Some(stored) => Case::parse(stored),
            None => data.lookup(name, &g.unicodes).case.map(|c| c.value),
        },
        None => data.lookup(name, &[]).case.map(|c| c.value),
    }
}

pub(crate) fn is_mark(font: &Font, data: &GlyphData, name: &str) -> bool {
    category_of(font, data, name) == Some(Category::Mark)
}

/// The code point the glyph database gives a mark's name without its suffix.
fn mark_code_point(data: &GlyphData, name: &str) -> Option<u32> {
    let (base, _) = data.split_suffix(name);
    data.lookup(base, &[]).unicode.map(|u| u.value)
}

/// The parts before resolution, or `None` when no rule decomposes the name.
fn raw_slots(font: &Font, data: &GlyphData, name: &str, base: &str, suffix: Option<&str>) -> Result<Option<(Kind, Vec<Slot>)>, ComponentsError> {
    let plain = |names: Vec<String>| -> Vec<Slot> { names.into_iter().map(|n| Slot { names: vec![n], mark: false }).collect() };
    if suffix.is_some()
        && let Some(parts) = custom_decomposition(data, base)?
    {
        return Ok(Some((Kind::Decomposition, plain(parts))));
    }
    if let Some(dotless) = match base {
        "i" => Some("idotless"),
        "j" => Some("jdotless"),
        _ => None,
    } {
        // A suffixed i or j (a small capital, say) has no dot to take off unless the font has
        // the suffixed dotless letter.
        if suffix.is_none() || font.glyph(&suffixed(dotless, suffix)).is_some() {
            return Ok(Some((Kind::Decomposition, plain(vec![dotless.to_string(), "dotaccentcomb".to_string()]))));
        }
        return Ok(None);
    }
    let own: Vec<u32> = if suffix.is_none() { font.glyph(name).map(|g| g.unicodes.clone()).unwrap_or_default() } else { Vec::new() };
    if let Some(cp) = data.lookup(base, &own).unicode.map(|u| u.value)
        && let Some(cps) = expand(font, data, cp, suffix, 0)
    {
        let mut slots = plain(cps.iter().map(|c| data.nice_name(*c)).collect());
        exceptions(base, &mut slots);
        return Ok(Some((Kind::Decomposition, slots)));
    }
    if base.contains('_')
        && !base.starts_with('_')
        && let Some(parts) = data.split_ligature(base)
        && parts.len() >= 2
        && parts.iter().all(|p| !p.is_empty())
    {
        if parts.len() > MAX_RECIPE_PARTS {
            return Err(ComponentsError::LimitExceeded { what: "components in one composite", limit: MAX_RECIPE_PARTS });
        }
        for p in &parts {
            check_name(p)?;
        }
        return Ok(Some((Kind::Ligature, plain(parts))));
    }
    Ok(None)
}

/// The full canonical decomposition of `cp`, keeping a first code point that carries a horn
/// whole when the font has its glyph.
fn expand(font: &Font, data: &GlyphData, cp: u32, suffix: Option<&str>, depth: usize) -> Option<Vec<u32>> {
    let d = data.canonical_decomposition(cp)?;
    let (first, rest) = d.split_first()?;
    let mut out = match data.canonical_decomposition(*first) {
        Some(inner) => {
            let horn = inner.iter().skip(1).any(|m| data.canonical_combining_class(*m) == CCC_ATTACHED_ABOVE_RIGHT);
            if (horn && font.glyph(&suffixed(&data.nice_name(*first), suffix)).is_some()) || depth + 1 >= MAX_DECOMPOSITION_DEPTH {
                vec![*first]
            } else {
                expand(font, data, *first, suffix, depth + 1).unwrap_or_else(|| vec![*first])
            }
        }
        None => vec![*first],
    };
    for c in rest {
        out.extend(data.full_canonical_decomposition(*c).unwrap_or_else(|| vec![*c]));
    }
    Some(out)
}

/// Our exceptions to the Unicode decomposition (rule 4 of the module doc).
fn exceptions(base: &str, slots: &mut [Slot]) {
    for slot in slots.iter_mut().skip(1) {
        let Some(first) = slot.names.first().cloned() else { continue };
        if first == "cedillacomb" && base.contains("commaaccent") {
            slot.names = vec![if base == "gcommaaccent" { "commaturnedabovecomb" } else { "commaaccentcomb" }.to_string()];
        } else if first == "caroncomb" && matches!(base, "dcaron" | "lcaron" | "Lcaron" | "tcaron") {
            slot.names = vec!["caroncomb.alt".to_string(), "caroncomb".to_string()];
        }
    }
}

/// The parts resolved against the font; see rules 5 and 6 of the module doc.
fn resolve(
    font: &Font,
    data: &GlyphData,
    name: &str,
    base: &str,
    suffix: Option<&str>,
    kind: Kind,
    mut slots: Vec<Slot>,
) -> Result<Vec<String>, ComponentsError> {
    for (i, slot) in slots.iter_mut().enumerate() {
        let plain = slot.names.last().map(String::as_str).unwrap_or_default();
        slot.mark = !(kind == Kind::Decomposition && i == 0) && is_mark(font, data, plain);
    }
    // Rule 5: the dot of i and j gives way to a mark above.
    let mut dotted_fallback = None;
    if kind == Kind::Decomposition
        && let Some(first) = slots.first().and_then(|s| s.names.first()).cloned()
        && let Some(dotless) = match first.as_str() {
            "i" => Some("idotless"),
            "j" => Some("jdotless"),
            _ => None,
        }
        && slots.iter().skip(1).any(|s| {
            s.mark && s.names.last().and_then(|n| mark_code_point(data, n)).is_some_and(|cp| data.canonical_combining_class(cp) == CCC_ABOVE)
        })
    {
        if let Some(s) = slots.first_mut() {
            s.names = vec![dotless.to_string()];
        }
        dotted_fallback = Some(first);
    }
    let narrow = kind == Kind::Decomposition && slots.first().and_then(|s| s.names.first()).is_some_and(|n| n == "idotless" || n == "jdotless");
    let upper = case_of(font, data, base) == Some(Case::Upper);
    let mark_candidates = |names: &[String]| -> Vec<String> {
        let mut out = Vec::new();
        for n in names {
            if let Some(s) = suffix {
                out.push(format!("{n}.{s}"));
            }
            if upper {
                out.push(format!("{n}.case"));
            }
            if narrow {
                out.push(format!("{n}.i"));
                out.push(format!("{n}.narrow"));
            }
            out.push(n.clone());
        }
        dedup(out)
    };
    let pick = |candidates: &[String]| candidates.iter().find(|c| font.glyph(c).is_some()).cloned();

    let mut parts = Vec::with_capacity(slots.len());
    let mut missing = Vec::new();
    let mut i = 0;
    while let Some(slot) = slots.get(i) {
        if kind == Kind::Decomposition
            && slot.mark
            && let Some(next) = slots.get(i + 1).filter(|s| s.mark)
        {
            let joined: Vec<String> = slot.names.iter().flat_map(|a| next.names.iter().map(move |b| format!("{a}_{b}"))).collect();
            if let Some(found) = pick(&mark_candidates(&joined)) {
                parts.push(found);
                i += 2;
                continue;
            }
        }
        let (candidates, report) = if slot.mark {
            let c = mark_candidates(&slot.names);
            let report = slot.names.last().cloned().unwrap_or_default();
            (c, report)
        } else {
            let mut c: Vec<String> = Vec::new();
            for n in &slot.names {
                c.push(suffixed(n, suffix));
                if kind == Kind::Ligature && suffix.is_some() {
                    c.push(n.clone());
                }
            }
            if i == 0
                && suffix.is_some()
                && let Some(dotted) = &dotted_fallback
            {
                c.push(suffixed(dotted, suffix));
            }
            let report = c.first().cloned().unwrap_or_default();
            (dedup(c), report)
        };
        match pick(&candidates) {
            Some(found) => parts.push(found),
            None => missing.push(report),
        }
        i += 1;
    }
    let missing = dedup(missing);
    if !missing.is_empty() {
        return Err(ComponentsError::MissingParts { glyph: name.to_string(), missing });
    }
    refuse_self(name, parts)
}

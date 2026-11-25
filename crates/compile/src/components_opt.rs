//! Outline steps before the decomposition every build does ([`crate::outline`],
//! `docs/spec/40-compile.md` § 1.3): the erasing of invisible open corners, then the component
//! export options, what a source or the compile options ask of components beyond it.
//!
//! | Asked by | Effect |
//! |---|---|
//! | [`ComponentOptions::erase_open_corners`], else the source ([`OpenCorners`]) | small open corners are erased ([`crate::outline::erase_open_corners`]) |
//! | [`ComponentOptions::decompose_all`], the *Decompose Components* parameter | every composite becomes contours |
//! | the *Decompose Glyphs* parameter | the composites it lists become contours |
//! | a `decomposeComponents` entry in the source's filter list | the composites it names become contours |
//! | [`ComponentOptions::decompose_transformed`], else a `decomposeTransformedComponents` entry | composites with a component whose 2×2 transform is not the identity become contours |
//! | [`ComponentOptions::flatten`], else a `flattenComponents` entry | a component that points at a composite is replaced by the glyphs that composite reaches, transforms combined, so every composite is one level deep |
//!
//! The decomposing steps run first and flattening after them, the order both reference compilers
//! use (fontc's `apply_optional_transformations` in `fontir/src/glyph.rs`, Apache-2.0; ufo2ft's
//! `decomposeComponents.py`, `decomposeTransformedComponents.py` and `flattenComponents.py`,
//! MIT). A glyph decomposed here is drawn as [`crate::outline`] draws a mixed glyph,
//! by the same function, so it keeps its own contours first and a mirrored component's contours
//! are reversed. Flattening stops at a glyph that has contours, or that will be decomposed
//! anyway (mixed, or with a transform `glyf` cannot store), as both references do: that glyph is
//! simple by the time it is written.
//!
//! The filter list is ufo2ft's (`com.github.googlei18n.ufo2ft.filters`). It is read from the
//! built master's user data, where a UFO's lib and a `.glyphs` master's user data are kept, and
//! when the master has none, from the font's (a designspace's lib). An entry names its filter
//! (`name`, spelled as ufo2ft looks it up: spaces dropped, the first letter in either case) and
//! may limit it with an `include` or an `exclude` list of glyph names. Entries of other filters,
//! or of another `namespace`, are skipped; `eraseOpenCorners` is read whatever its namespace
//! (glyphsLib's own, in the sources it writes), and the overlap and anchor filters are read where
//! those steps run.
//!
//! *Keep Overlapping Components* and *Keep Transformed Components* are not read: overlapping and
//! transformed components stay components by default, as in both references
//! (`docs/spec/40-compile.md` § 4a). CFF output decomposes every composite already, so nothing
//! here runs for it.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{Affine, BezPath, PathEl, Point};
use fontcraft_interp::GlyphSources;
use fontcraft_model::plist::Value;
use fontcraft_model::{
    Font, MAX_COMPONENT_DEPTH, ModelError, Resolved, ResolvedComponent, ResolvedGlyph, Shape, SourceFormat, is_ufo_family, param_flag,
};

use crate::outline::{Flavour, decomposed, erase_open_corners, must_decompose};
use crate::{CompileError, Expansion, MAX_COMPONENTS, MAX_FONT_GLYPHS};

/// The user-data key of ufo2ft's filter list.
pub const FILTERS_KEY: &str = "com.github.googlei18n.ufo2ft.filters";
/// The filter that erases invisible open corners, spelled as [`filter_key`] gives it.
const ERASE_OPEN_CORNERS: &str = "eraseOpenCorners";
/// The yes-or-no parameter that decomposes every composite.
pub const DECOMPOSE_COMPONENTS: &str = "Decompose Components";
/// The parameter that lists composites to decompose.
pub const DECOMPOSE_GLYPHS: &str = "Decompose Glyphs";
/// Most entries of a filter list that are read; the rest are skipped with a warning.
pub const MAX_FILTERS: usize = 256;
/// Most names one glyph list may hold (a *Decompose Glyphs* value, an `include` or an `exclude`
/// list). A font holds no more glyphs than this.
pub const MAX_LISTED_NAMES: usize = MAX_FONT_GLYPHS;

/// Component options of a compile ([`crate::Options::components`]). The default does what the
/// source asks and nothing more.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentOptions {
    /// Decompose every composite (`compile.decomposeAllComponents`).
    pub decompose_all: bool,
    /// Decompose composites with a transformed component (`compile.decomposeTransformedComponents`).
    /// `None`: as the source's filter list says.
    pub decompose_transformed: Option<bool>,
    /// Flatten nested composites (`compile.flattenNestedComponents`). `None`: as the source's
    /// filter list says.
    pub flatten: Option<bool>,
    /// Erase invisible open corners (`compile.glyphsOutlinePreSteps`). `None`: as the source asks
    /// ([`OpenCorners`]).
    pub erase_open_corners: Option<bool>,
    /// Glyphs whose open corners stay whatever the settings say: in a variable font, those whose
    /// masters do not all have the same ones, which would no longer be compatible.
    pub keep_open_corners: HashSet<String>,
}

/// The glyphs one filter entry applies to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Scope {
    All,
    Only(HashSet<String>),
    Except(HashSet<String>),
}

impl Scope {
    fn contains(&self, name: &str) -> bool {
        match self {
            Scope::All => true,
            Scope::Only(names) => names.contains(name),
            Scope::Except(names) => !names.contains(name),
        }
    }
}

/// The glyphs of a font whose invisible open corners a build erases when the options do not say:
/// those an `eraseOpenCorners` entry of the filter list reaches; without a filter list, every
/// glyph of a `.glyphs` or native source and none of a UFO, a designspace or a compiled font.
/// This is what glyphsLib and fontc do: glyphsLib lists the filter in every UFO it writes, and a
/// list in a master's user data stands in its place.
#[derive(Clone, Debug)]
pub struct OpenCorners {
    by_master: HashMap<String, Vec<Scope>>,
}

impl OpenCorners {
    pub fn of(font: &Font) -> Self {
        let by_master = font.masters.iter().map(|m| (m.id.clone(), open_corner_scopes(font, &m.id))).collect();
        Self { by_master }
    }

    /// Whether they are erased in every glyph of every master.
    pub fn all(&self) -> bool {
        self.by_master.values().all(|scopes| scopes.contains(&Scope::All))
    }

    /// Whether the open corners of `glyph` in `master` are erased.
    pub fn erased(&self, master: &str, glyph: &str) -> bool {
        self.by_master.get(master).is_some_and(|scopes| reaches(scopes, glyph))
    }

    /// `outline` (of `glyph`, in `master`) as it is filled, without the invisible open corners a
    /// build erases; `None` when that is the outline as it is.
    pub fn filled(&self, master: &str, glyph: &str, outline: &BezPath) -> Option<BezPath> {
        if self.erased(master, glyph) { fontcraft_pathops::erase_invisible_corners(outline) } else { None }
    }
}

/// Erases the invisible open corners of `glyphs`, resolved at `master`, as the options and the
/// source ask.
pub(crate) fn erase_corners(font: Option<&Font>, master: &str, glyphs: &mut [ResolvedGlyph], options: &ComponentOptions) {
    let scopes = match (options.erase_open_corners, font) {
        (Some(true), _) => vec![Scope::All],
        (None, Some(font)) => open_corner_scopes(font, master),
        (Some(false), _) | (None, None) => Vec::new(),
    };
    if !scopes.is_empty() {
        erase_open_corners(glyphs, |name| reaches(&scopes, name) && !options.keep_open_corners.contains(name));
    }
}

/// The glyphs of a variable font with the default master `origin` that the options and the source
/// erase open corners in, and whose masters and intermediate layers do not all lose the same ones
/// ([`ComponentOptions::keep_open_corners`]).
pub(crate) fn uneven_open_corners<'a>(
    font: &Font,
    origin: &str,
    sources: impl Iterator<Item = &'a GlyphSources>,
    options: &ComponentOptions,
) -> HashSet<String> {
    let mut out = HashSet::new();
    let corners = OpenCorners::of(font);
    for source in sources {
        if !options.erase_open_corners.unwrap_or_else(|| corners.erased(origin, &source.glyph)) {
            continue;
        }
        let Some(glyph) = font.glyph(&source.glyph) else { continue };
        let mut first: Option<Vec<Vec<bool>>> = None;
        for layer in source.main.iter().filter_map(|s| glyph.layers.iter().find(|l| l.id == s.layer_id)) {
            let kept: Vec<Vec<bool>> = layer.shapes.iter().map(kept_points).collect();
            if *first.get_or_insert_with(|| kept.clone()) != kept {
                out.insert(source.glyph.clone());
                break;
            }
        }
    }
    out
}

/// Which on-curve points of a path are left when its invisible open corners are erased; empty
/// when it has none.
fn kept_points(shape: &Shape) -> Vec<bool> {
    let Shape::Path(path) = shape else { return Vec::new() };
    let Ok(contour) = path.to_contour() else { return Vec::new() };
    let Some(erased) = fontcraft_pathops::erase_invisible_corners(&contour) else { return Vec::new() };
    let key = |p: Point| (p.x.to_bits(), p.y.to_bits());
    let left: HashSet<(u64, u64)> = erased.elements().iter().filter_map(PathEl::end_point).map(key).collect();
    contour.elements().iter().filter_map(PathEl::end_point).map(|p| left.contains(&key(p))).collect()
}

fn open_corner_scopes(font: &Font, master: &str) -> Vec<Scope> {
    match filter_setting(font, master) {
        Some(setting) => {
            let entries = filter_entries(setting, &|name, _| name == ERASE_OPEN_CORNERS, &mut Vec::new());
            entries.into_iter().map(|(_, scope)| scope).collect()
        }
        None if is_ufo_family(font.source.format) || font.source.format == SourceFormat::Binary => Vec::new(),
        None => vec![Scope::All],
    }
}

/// The component filters of a filter list, each with the scopes of its entries. A filter listed
/// twice runs twice in ufo2ft, which reaches the glyphs of either entry.
#[derive(Clone, Debug, Default, PartialEq)]
struct SourceFilters {
    decompose: Vec<Scope>,
    decompose_transformed: Vec<Scope>,
    flatten: Vec<Scope>,
}

pub(crate) fn reaches(scopes: &[Scope], name: &str) -> bool {
    scopes.iter().any(|s| s.contains(name))
}

/// Applies the component options to `glyphs` (in their final order, after preflight), for a
/// build of `flavour`. Decomposed glyphs come back simple and flattened ones with their new
/// components; everything decomposing creates is taken from `expansion`.
pub(crate) fn apply(
    font: Option<&Font>,
    resolved: &Resolved,
    mut glyphs: Vec<ResolvedGlyph>,
    flavour: Flavour,
    options: &ComponentOptions,
    expansion: &mut Expansion,
    warnings: &mut Vec<String>,
) -> Result<Vec<ResolvedGlyph>, CompileError> {
    erase_corners(font, &resolved.master, &mut glyphs, options);
    if flavour == Flavour::Cff || glyphs.iter().all(|g| g.components.is_empty()) {
        return Ok(glyphs);
    }
    let filters = font.map(|f| source_filters(f, &resolved.master, warnings)).unwrap_or_default();
    let all = options.decompose_all || resolved.params.get(DECOMPOSE_COMPONENTS).and_then(param_flag).unwrap_or(false);
    let listed = listed_glyphs(resolved, warnings);
    let transformed = |name: &str| options.decompose_transformed.unwrap_or_else(|| reaches(&filters.decompose_transformed, name));
    let forced: Vec<bool> = glyphs
        .iter()
        .map(|g| {
            !g.components.is_empty()
                && (all
                    || listed.contains(&g.name)
                    || reaches(&filters.decompose, &g.name)
                    || (transformed(&g.name) && g.components.iter().any(is_transformed)))
        })
        .collect();
    decompose(&mut glyphs, &forced, flavour, expansion)?;
    let flatten = |name: &str| options.flatten.unwrap_or_else(|| reaches(&filters.flatten, name));
    flatten_all(&mut glyphs, flatten, flavour, expansion)?;
    Ok(glyphs)
}

/// Is the component's 2×2 transform anything but the identity? Exact, as both references test it.
fn is_transformed(c: &ResolvedComponent) -> bool {
    c.transform.as_coeffs().get(..4) != Some(&[1.0, 0.0, 0.0, 1.0][..])
}

/// The filter list of `master`, else of the font.
fn filter_setting<'a>(font: &'a Font, master: &str) -> Option<&'a Value> {
    font.master(master).ok().and_then(|m| m.user_data.get(FILTERS_KEY)).or_else(|| font.info.user_data.get(FILTERS_KEY))
}

/// The component filters of the filter list of `master`, else of the font.
fn source_filters(font: &Font, master: &str, warnings: &mut Vec<String>) -> SourceFilters {
    match filter_setting(font, master) {
        Some(setting) => read_filters(setting, warnings),
        None => SourceFilters::default(),
    }
}

fn read_filters(setting: &Value, warnings: &mut Vec<String>) -> SourceFilters {
    let mut out = SourceFilters::default();
    for (key, scope) in filter_entries(setting, &in_ufo2ft, warnings) {
        match key.as_str() {
            "decomposeComponents" => out.decompose.push(scope),
            "decomposeTransformedComponents" => out.decompose_transformed.push(scope),
            "flattenComponents" => out.flatten.push(scope),
            _ => {}
        }
    }
    out
}

/// The scopes of the entries of filter `key` (spelled as [`filter_key`] gives it) in the filter
/// list of `master`, else of the font; empty when the filter is not listed. Problems with the
/// list are left to [`apply`] to report.
pub(crate) fn listed_filter(font: &Font, master: &str, key: &str) -> Vec<Scope> {
    let Some(setting) = filter_setting(font, master) else { return Vec::new() };
    filter_entries(setting, &in_ufo2ft, &mut Vec::new()).into_iter().filter(|(k, _)| k == key).map(|(_, scope)| scope).collect()
}

/// Entries of ufo2ft's own filters: its namespace, or none.
fn in_ufo2ft(_: &str, namespace: Option<&Value>) -> bool {
    namespace.is_none_or(|n| n.as_str() == Some("ufo2ft.filters"))
}

/// The entries of a filter list that `wanted` takes (by the filter's key and the entry's
/// namespace), in list order: each filter's key and the glyphs it reaches.
fn filter_entries(setting: &Value, wanted: &dyn Fn(&str, Option<&Value>) -> bool, warnings: &mut Vec<String>) -> Vec<(String, Scope)> {
    let mut out = Vec::new();
    let Some(entries) = setting.as_array() else {
        warnings.push(format!("the filter list ({FILTERS_KEY}) is not a list, so its component filters were not applied"));
        return out;
    };
    if entries.items.len() > MAX_FILTERS {
        warnings.push(format!("the filter list ({FILTERS_KEY}) has {} entries; only the first {MAX_FILTERS} were read", entries.items.len()));
    }
    for entry in entries.items.iter().take(MAX_FILTERS) {
        let Some(dict) = entry.as_dict() else { continue };
        let Some(name) = dict.get("name").and_then(Value::as_str) else { continue };
        let key = filter_key(name);
        if !wanted(&key, dict.get("namespace")) {
            continue;
        }
        match scope(dict.get("include"), dict.get("exclude")) {
            Ok(s) => out.push((key, s)),
            Err(why) => warnings.push(format!("the {name} filter was not applied: {why}")),
        }
    }
    out
}

/// A filter name as ufo2ft looks up its module: spaces dropped, the first letter lowercase.
fn filter_key(name: &str) -> String {
    let mut letters = name.chars().filter(|c| *c != ' ');
    match letters.next() {
        Some(first) => first.to_lowercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// The scope of an entry with these `include` and `exclude` values. ufo2ft refuses an entry
/// with both.
fn scope(include: Option<&Value>, exclude: Option<&Value>) -> Result<Scope, String> {
    match (include, exclude) {
        (Some(_), Some(_)) => Err("it has both an include and an exclude list".to_string()),
        (Some(v), None) => name_list(v).map(Scope::Only),
        (None, Some(v)) => name_list(v).map(Scope::Except),
        (None, None) => Ok(Scope::All),
    }
}

fn name_list(v: &Value) -> Result<HashSet<String>, String> {
    let Some(list) = v.as_array() else { return Err("its glyph list is not a list".to_string()) };
    if list.items.len() > MAX_LISTED_NAMES {
        return Err(format!("its glyph list has more than {MAX_LISTED_NAMES} names"));
    }
    Ok(list.items.iter().filter_map(Value::as_str).map(str::to_string).collect())
}

/// The glyphs the *Decompose Glyphs* parameters list: a list of names, or one text of names
/// separated by commas or spaces.
fn listed_glyphs(resolved: &Resolved, warnings: &mut Vec<String>) -> HashSet<String> {
    let mut out = HashSet::new();
    for v in resolved.params.all(DECOMPOSE_GLYPHS) {
        let names: Vec<&str> = match v {
            Value::Array(list) => list.items.iter().filter_map(Value::as_str).collect(),
            Value::String(text) => text.as_str().split(|c: char| c == ',' || c.is_whitespace()).filter(|n| !n.is_empty()).collect(),
            _ => {
                warnings.push(format!("the {DECOMPOSE_GLYPHS} parameter is not a list of glyph names, so it was not applied"));
                continue;
            }
        };
        for name in names {
            if out.len() >= MAX_LISTED_NAMES {
                warnings.push(format!("the {DECOMPOSE_GLYPHS} parameters list more than {MAX_LISTED_NAMES} glyphs; the rest were not decomposed"));
                return out;
            }
            out.insert(name.to_string());
        }
    }
    out
}

/// Draws every glyph marked in `forced` as contours.
fn decompose(glyphs: &mut [ResolvedGlyph], forced: &[bool], flavour: Flavour, expansion: &mut Expansion) -> Result<(), CompileError> {
    if !forced.contains(&true) {
        return Ok(());
    }
    let drawn: Vec<(usize, Vec<BezPath>)> = {
        let glyphs: &[ResolvedGlyph] = glyphs;
        let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
        // A glyph the outline stage decomposes anyway is drawn in as its own decomposed outline,
        // as the references draw it once it is simple.
        let marked: Vec<bool> = glyphs.iter().zip(forced).map(|(g, &f)| f || must_decompose(g, flavour)).collect();
        let mut memo = HashMap::new();
        let mut out = Vec::new();
        for (i, _) in forced.iter().enumerate().filter(|(_, f)| **f) {
            out.push((i, decomposed(glyphs, &index, &marked, i, 0, &mut memo, expansion)?));
        }
        out
    };
    for (i, contours) in drawn {
        if let Some(g) = glyphs.get_mut(i) {
            g.contours = contours;
            g.components.clear();
        }
    }
    Ok(())
}

/// Flattens every composite `wanted` names that the outline stage keeps as a composite.
fn flatten_all(glyphs: &mut [ResolvedGlyph], wanted: impl Fn(&str) -> bool, flavour: Flavour, expansion: &mut Expansion) -> Result<(), CompileError> {
    let flat: Vec<(usize, Vec<ResolvedComponent>)> = {
        let glyphs: &[ResolvedGlyph] = glyphs;
        let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
        let mut out = Vec::new();
        for (i, g) in glyphs.iter().enumerate() {
            if g.components.is_empty() || must_decompose(g, flavour) || !wanted(&g.name) {
                continue;
            }
            let leaves = leaves(glyphs, &index, g, flavour, expansion)?;
            if leaves != g.components {
                out.push((i, leaves));
            }
        }
        out
    };
    for (i, components) in flat {
        if let Some(g) = glyphs.get_mut(i) {
            g.components = components;
        }
    }
    Ok(())
}

/// The glyphs `g`'s components reach through pure composites, in drawing order (depth first),
/// each with its combined transform.
fn leaves(
    glyphs: &[ResolvedGlyph],
    index: &HashMap<&str, usize>,
    g: &ResolvedGlyph,
    flavour: Flavour,
    expansion: &mut Expansion,
) -> Result<Vec<ResolvedComponent>, CompileError> {
    let mut out = Vec::new();
    let mut stack: Vec<(&str, Affine, usize)> = g.components.iter().rev().map(|c| (c.base.as_str(), c.transform, 0)).collect();
    while let Some((base, affine, depth)) = stack.pop() {
        expansion.take(1, &g.name)?;
        let nested = index.get(base).and_then(|&j| glyphs.get(j)).filter(|r| !r.components.is_empty() && !must_decompose(r, flavour));
        match nested {
            None => {
                if out.len() >= MAX_COMPONENTS {
                    return Err(CompileError::Glyph {
                        glyph: g.name.clone(),
                        message: format!("flattening its components would give it more than {MAX_COMPONENTS}"),
                    });
                }
                out.push(ResolvedComponent { base: base.to_string(), transform: affine });
            }
            Some(r) => {
                if depth >= MAX_COMPONENT_DEPTH {
                    return Err(ModelError::ComponentCycle(g.name.clone()).into());
                }
                stack.extend(r.components.iter().rev().map(|c| (c.base.as_str(), affine * c.transform, depth + 1)));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "components_opt_tests.rs"]
mod tests;

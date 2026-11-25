//! Checking feature code for every build that compiles it (M8.51): the masters, the static instances and the variable
//! font. `font.features.compile` and `font.features.check` take `targets`.
//!
//! Each target's code is assembled as its export assembles it: an instance's Add, Replace and Remove parameters applied,
//! tokens expanded with the target's values, `#ifdef VARIABLE` blocks kept or dropped, variable syntax lowered at its
//! location. The assembled code is then compiled on its own, without generated kerning and marks; targets whose code is
//! the same text share one compile. An instance between the masters is interpolated only when the code has a `$` token,
//! since nothing else in the code reads its values. Each diagnostic names the targets it occurs in.
//!
//! [`check_targets`] runs on a font and a [`CheckContext`] without the session, so the Features tab runs it on a worker.
//!
//! Limits: at most [`MAX_TARGETS`] targets, [`MAX_DIAGNOSTICS`] diagnostics; each compile is capped by
//! `fontcraft_fea`'s limits.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use fontcraft_fea::{CompileRequest, Diagnostic, FeaVariations, GlyphOrder, MAX_DIAGNOSTICS, Severity};
use fontcraft_features::params::apply_instance_params;
use fontcraft_features::var_info::VariableFeatures;
use fontcraft_features::{EntryDiagnostic, EntryRange, FeatureError, MasterValues, Target, TargetValues, assemble};
use fontcraft_fontinfo::target as lists;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Features, Font, Params as FontParams};
use serde_json::{Value, json};

use super::features_check::CheckContext;
use super::features_compile::{diagnostics_json, glyph_order, group, master_id, request_diagnostic};
use crate::Result;
use crate::feature_state::FeatureState;
use crate::params::Params;

/// Most targets one check takes.
pub const MAX_TARGETS: usize = 256;

/// One build whose feature code is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckTarget {
    /// The master with this id.
    Master(String),
    /// The static instance at this index of `Font::instances`.
    Instance(usize),
    /// The variable font of the whole designspace.
    Variable,
}

impl CheckTarget {
    /// The name the target is reported under.
    pub(crate) fn label(&self, font: &Font) -> String {
        match self {
            CheckTarget::Master(id) => {
                let name = font.masters.iter().find(|m| m.id == *id).map(|m| m.name.as_str()).filter(|n| !n.is_empty());
                format!("master {}", name.unwrap_or(id))
            }
            CheckTarget::Instance(i) => match font.instances.get(*i).map(lists::instance_name).filter(|n| !n.is_empty()) {
                Some(name) => format!("instance {name}"),
                None => format!("instance #{i}"),
            },
            CheckTarget::Variable => "variable font".to_string(),
        }
    }
}

/// One error or warning and the targets it occurs in, as indices into [`TargetCheck::targets`].
#[derive(Clone, Debug, PartialEq)]
pub struct TargetDiagnostic {
    pub found: EntryDiagnostic,
    pub targets: Vec<usize>,
}

/// What a check of several targets found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetCheck {
    /// The targets checked, each with the name it is reported under.
    pub targets: Vec<(CheckTarget, String)>,
    pub found: Vec<TargetDiagnostic>,
    /// Compiles run: targets with the same assembled code share one.
    pub compiled: usize,
    /// Targets past [`MAX_TARGETS`], not checked.
    pub skipped: usize,
}

impl TargetCheck {
    /// The names of the targets `d` occurs in.
    pub fn names(&self, d: &TargetDiagnostic) -> Vec<&str> {
        d.targets.iter().filter_map(|t| self.targets.get(*t)).map(|(_, name)| name.as_str()).collect()
    }

    /// The targets the diagnostic `d` of another compile is limited to: their names when the check found it in some
    /// targets and not in others, else `None`.
    pub fn limited_to(&self, d: &EntryDiagnostic) -> Option<Vec<&str>> {
        self.found.iter().find(|f| same(&f.found, d)).filter(|f| f.targets.len() < self.targets.len()).map(|f| self.names(f))
    }

    /// The diagnostics that are not among `known`, those of the compile of one master.
    pub fn beyond<'a>(&'a self, known: &'a [EntryDiagnostic]) -> impl Iterator<Item = &'a TargetDiagnostic> {
        self.found.iter().filter(move |f| !known.iter().any(|k| same(&f.found, k)))
    }
}

/// The two diagnostics say the same of the same place in the stored code.
fn same(a: &EntryDiagnostic, b: &EntryDiagnostic) -> bool {
    a.place == b.place && a.diagnostic.severity == b.diagnostic.severity && a.diagnostic.message == b.diagnostic.message
}

/// Every target of `font`: its masters, its exported static instances, and the variable font of a font with axes and
/// several masters.
pub fn all_targets(font: &Font) -> Vec<CheckTarget> {
    let masters = font.masters.iter().map(|m| CheckTarget::Master(m.id.clone()));
    let instances = font.instances.iter().enumerate().filter(|(_, inst)| inst.exports && inst.kind.is_none()).map(|(i, _)| CheckTarget::Instance(i));
    let variable = (!font.axes.is_empty() && font.masters.len() > 1).then_some(CheckTarget::Variable);
    masters.chain(instances).chain(variable).collect()
}

/// The `targets` parameter: `"all"`, `"variable"`, or a list of `{master}`, `{instance}` and `"variable"`; with whether
/// it is every target. `None` when absent.
pub(super) fn targets_param(p: &Params, font: &Font) -> Result<Option<(Vec<CheckTarget>, bool)>> {
    let one = |v: &Value| -> Result<CheckTarget> {
        if v.as_str() == Some("variable") {
            return Ok(CheckTarget::Variable);
        }
        if let Some(master) = v.get("master") {
            let name = master.as_str().ok_or_else(|| p.bad("`targets`: `master` is an id or a name"))?;
            return Ok(CheckTarget::Master(master_id(p, font, Some(name))?));
        }
        if let Some(instance) = v.get("instance") {
            let at = lists::parse(instance).and_then(|r| lists::instance(font, &r)).map_err(|e| p.bad(format!("`targets`: `instance`: {e}")))?;
            if font.instances.get(at).is_some_and(|inst| inst.kind.is_some()) {
                return Err(p.bad("`targets`: name a static instance, or \"variable\" for the variable font"));
            }
            return Ok(CheckTarget::Instance(at));
        }
        Err(p.bad("each of `targets` is {master: id|name}, {instance: index|id|name} or \"variable\""))
    };
    match p.get("targets") {
        None => Ok(None),
        Some(Value::String(all)) if all == "all" => Ok(Some((all_targets(font), true))),
        Some(Value::Array(list)) => {
            if list.len() > MAX_TARGETS {
                return Err(p.bad(format!("`targets` holds more than {MAX_TARGETS} targets")));
            }
            let mut out = Vec::new();
            for v in list {
                let target = one(v)?;
                if !out.contains(&target) {
                    out.push(target);
                }
            }
            Ok(Some((out, false)))
        }
        Some(v) => Ok(Some((vec![one(v)?], false))),
    }
}

/// Checks `features` (the font's own, or with a draft in place of an entry's code) for `targets`.
pub fn check_targets(font: &Font, features: &Features, cx: &CheckContext, mut targets: Vec<CheckTarget>) -> TargetCheck {
    let skipped = targets.len().saturating_sub(MAX_TARGETS);
    targets.truncate(MAX_TARGETS);
    let mut out = TargetCheck { skipped, ..TargetCheck::default() };
    let order = match GlyphOrder::new(glyph_order(font)) {
        Ok(order) => order,
        Err(d) => {
            let every = (0..targets.len()).collect();
            out.found.push(TargetDiagnostic { found: EntryDiagnostic { place: None, diagnostic: d }, targets: every });
            out.targets = targets.into_iter().map(|t| (t.label(font), t)).map(|(label, t)| (t, label)).collect();
            return out;
        }
    };
    let mut run = Run { font, features, cx, order: &order, compiled: HashMap::new() };
    let mut places: HashMap<(Option<EntryRangeKey>, Severity, String), usize> = HashMap::new();
    for (at, target) in targets.into_iter().enumerate() {
        for found in run.target(&target) {
            let key = (found.place.as_ref().map(range_key), found.diagnostic.severity, found.diagnostic.message.clone());
            match places.get(&key).copied() {
                Some(i) => {
                    if let Some(known) = out.found.get_mut(i).filter(|k| k.targets.last() != Some(&at)) {
                        known.targets.push(at);
                    }
                }
                None if out.found.len() < MAX_DIAGNOSTICS => {
                    places.insert(key, out.found.len());
                    out.found.push(TargetDiagnostic { found, targets: vec![at] });
                }
                None => {}
            }
        }
        let label = target.label(font);
        out.targets.push((target, label));
    }
    out.compiled = run.compiled.len();
    out
}

/// An entry's range as a map key.
type EntryRangeKey = (fontcraft_features::EntryRef, usize, usize);

fn range_key(place: &EntryRange) -> EntryRangeKey {
    (place.entry, place.range.start, place.range.end)
}

/// One check's inputs, and the diagnostics of each assembled text compiled so far, by its hash.
struct Run<'a> {
    font: &'a Font,
    features: &'a Features,
    cx: &'a CheckContext,
    order: &'a GlyphOrder,
    compiled: HashMap<u64, Vec<Diagnostic>>,
}

impl Run<'_> {
    /// The diagnostics of `target`, placed in the stored entries.
    fn target(&mut self, target: &CheckTarget) -> Vec<EntryDiagnostic> {
        let (font, stored, cx) = (self.font, self.features, self.cx);
        let at = at_target(font, stored, &cx.glyph_data, target, false, |made| self.compile(made));
        let mut out: Vec<EntryDiagnostic> = at.refused.into_iter().map(request_diagnostic).collect();
        match at.read {
            Ok(found) => out.extend(found.into_iter().map(|d| stored_place(stored, &at.features, d))),
            Err(message) => out.push(request_diagnostic(message)),
        }
        out
    }

    /// The diagnostics of the code of one build, compiled unless the same text was before.
    fn compile(&mut self, made: &TargetInputs<'_>) -> Vec<EntryDiagnostic> {
        let variations = made.variations;
        let assembly = match assemble(made.font, made.features, &made.target, made.values) {
            Ok(assembly) => assembly,
            Err(FeatureError::Code(found)) => return found,
            Err(e) => return vec![request_diagnostic(e.to_string())],
        };
        let (map, mut out) = (assembly.map, assembly.warnings);
        let mut files = assembly.files;
        if let Some(dir) = &self.cx.include_dir {
            files = files.with_include_dir(dir.clone());
        }
        if let Some(check) = crate::file_access::include_check() {
            files = files.with_include_check(check);
        }
        let mut hasher = DefaultHasher::new();
        variations.is_some().hash(&mut hasher);
        for (path, text) in files.iter() {
            path.hash(&mut hasher);
            text.hash(&mut hasher);
        }
        let order = self.order;
        let found = self.compiled.entry(hasher.finish()).or_insert_with(|| {
            let mut request = CompileRequest::new(&files, order);
            request.variations = variations.map(|v| v as &dyn FeaVariations);
            match fontcraft_fea::compile(&request) {
                Ok(compiled) => compiled.warnings,
                Err(found) => found,
            }
        });
        out.extend(found.iter().cloned().map(|d| map.map(d)));
        out
    }
}

/// What one build's feature code is assembled from.
pub(crate) struct TargetInputs<'a> {
    /// The font, or the instance's interpolated one.
    pub font: &'a Font,
    /// The entries, an instance's parameters applied.
    pub features: &'a Features,
    pub target: Target,
    pub values: &'a dyn TargetValues,
    /// For the variable font.
    pub variations: Option<&'a VariableFeatures>,
}

/// What [`at_target`] read.
pub(crate) struct AtTarget<'a, T> {
    /// What the reader made, or why the build's values cannot be had.
    pub read: Result<T, String>,
    /// The entries, an instance's parameters applied.
    pub features: Cow<'a, Features>,
    /// The instance's feature parameters that cannot be applied, each with the reason.
    pub refused: Vec<String>,
}

/// Gives `read` what the code of `target` is assembled from. An instance between the masters is interpolated when its
/// code has a token or `interpolate` asks for it, since nothing else reads its values.
pub(crate) fn at_target<'f, T>(
    font: &Font,
    features: &'f Features,
    data: &GlyphData,
    target: &CheckTarget,
    interpolate: bool,
    read: impl FnOnce(&TargetInputs<'_>) -> T,
) -> AtTarget<'f, T> {
    let mut made: Cow<'f, Features> = Cow::Borrowed(features);
    let mut refused = Vec::new();
    let read = match target {
        CheckTarget::Master(id) => MasterValues::new(font, data, id).map_err(|e| e.to_string()).map(|values| {
            let target = Target::Master(id.clone());
            read(&TargetInputs { font, features, target, values: &values, variations: None })
        }),
        CheckTarget::Variable => variable_inputs(font, features, data, read),
        CheckTarget::Instance(index) => match font.instances.get(*index) {
            Some(inst) => {
                // One parameter at a time, so one that cannot be applied is reported and the rest hold.
                for param in inst.params.0.iter().filter(|p| !p.disabled) {
                    match apply_instance_params(&made, &FontParams(vec![param.clone()])) {
                        Ok(next) if next == *made => {}
                        Ok(next) => made = Cow::Owned(next),
                        Err(e) => refused.push(e.to_string()),
                    }
                }
                instance_inputs(font, &made, data, *index, interpolate, read)
            }
            None => Err("the instance is gone".to_string()),
        },
    };
    AtTarget { read, features: made, refused }
}

fn variable_inputs<T>(font: &Font, features: &Features, data: &GlyphData, read: impl FnOnce(&TargetInputs<'_>) -> T) -> Result<T, String> {
    let failed = |e: String| format!("the variable font's code cannot be compiled: {e}");
    let (origin, layout) = fontcraft_compile::var::feature_layout(font).map_err(|e| failed(e.to_string()))?;
    let values = MasterValues::new(font, data, &origin).map_err(|e| failed(e.to_string()))?.for_variable_font();
    let variations = VariableFeatures::new(font, &layout).map_err(|e| failed(e.to_string()))?;
    Ok(read(&TargetInputs { font, features, target: Target::Variable, values: &values, variations: Some(&variations) }))
}

fn instance_inputs<T>(
    font: &Font,
    features: &Features,
    data: &GlyphData,
    index: usize,
    interpolate: bool,
    read: impl FnOnce(&TargetInputs<'_>) -> T,
) -> Result<T, String> {
    let target = fontcraft_model::Target::Instance(index);
    let away = fontcraft_compile::needs_instance_font(font, &target).is_some();
    let at: Cow<'_, Font> = if away && (interpolate || uses_tokens(features)) {
        Cow::Owned(interpolated(font, index).map_err(|e| format!("the instance cannot be interpolated: {e}"))?)
    } else {
        Cow::Borrowed(font)
    };
    let master = match &at {
        Cow::Owned(made) => made.masters.first(),
        // Without tokens the code reads nothing of the instance but its location.
        Cow::Borrowed(font) => {
            let location = font.instances.get(index).filter(|_| !away).map(|inst| &inst.location);
            location.and_then(|l| font.master_at(l)).or(font.masters.first())
        }
    };
    let master = master.map(|m| m.id.clone()).ok_or("the font has no master")?;
    let mut values = MasterValues::new(&at, data, &master).map_err(|e| e.to_string())?;
    if let Some((axes, location)) = fontcraft_compile::instance_feature_axes(font, &target) {
        values = values.with_feature_axes(axes, location);
    }
    Ok(read(&TargetInputs { font: &at, features, target: Target::Master(master), values: &values, variations: None }))
}

/// `font` at its instance `index`, reduced to one master.
fn interpolated(font: &Font, index: usize) -> std::result::Result<Font, String> {
    let options = fontcraft_interp::instance::options_for(font, index).map_err(|e| e.to_string())?;
    fontcraft_interp::instance::instance_font(font, index, &options).map_err(|e| e.to_string())
}

/// Whether an enabled entry's code has a token, whose value may differ between targets.
pub(crate) fn uses_tokens(f: &Features) -> bool {
    f.prefixes.iter().chain(&f.classes).chain(&f.features).any(|e| !e.disabled && e.code.contains('$'))
}

/// `d`, placed in `made` (the entries with an instance's parameters applied), placed in the stored entries: in the entry
/// of the same name and code. A diagnostic in code a parameter wrote has no place there; its message names the entry.
fn stored_place(stored: &Features, made: &Features, mut d: EntryDiagnostic) -> EntryDiagnostic {
    let Some(place) = &mut d.place else { return d };
    let kind = place.entry.kind;
    let Some(entry) = group(made, kind).get(place.entry.index) else { return EntryDiagnostic { place: None, ..d } };
    let list = group(stored, kind);
    let is_it = |e: &fontcraft_model::Snippet| e.name == entry.name && e.code == entry.code;
    let at = if list.get(place.entry.index).is_some_and(is_it) { Some(place.entry.index) } else { list.iter().position(is_it) };
    match at {
        Some(index) => place.entry.index = index,
        None => {
            d.diagnostic.message = format!("{}: {}", entry.name, d.diagnostic.message);
            d.place = None;
        }
    }
    d
}

/// Diagnostics of targets as data: as `font.features.listErrors` gives them, with `targets` (names) and without `id` and
/// `fixes`.
pub(super) fn targets_json<'a>(features: &Features, check: &TargetCheck, found: impl Iterator<Item = &'a TargetDiagnostic>) -> Vec<Value> {
    let found: Vec<&TargetDiagnostic> = found.collect();
    let state = FeatureState { diagnostics: found.iter().map(|f| f.found.clone()).collect(), ..FeatureState::default() };
    let mut list = diagnostics_json(features, &state, None, &Default::default());
    for (value, f) in list.iter_mut().zip(&found) {
        if let Some(object) = value.as_object_mut() {
            object.remove("id");
            object.remove("fixes");
            object.insert("targets".into(), json!(check.names(f)));
        }
    }
    list
}

/// The errors and warnings of `found`.
pub(super) fn count<'a>(found: impl Iterator<Item = &'a TargetDiagnostic>) -> (usize, usize) {
    found.fold((0, 0), |(errors, warnings), f| if f.found.diagnostic.is_error() { (errors + 1, warnings) } else { (errors, warnings + 1) })
}

/// The names of the targets `check` read.
pub(super) fn names_json(check: &TargetCheck) -> Value {
    json!(check.targets.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>())
}

/// A hash of what the check of every target reads, apart from outlines and metrics; `None` when the code has tokens,
/// which may read those.
pub(crate) fn targets_key(font: &Font) -> Option<u64> {
    if uses_tokens(&font.features) {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    for list in [&font.features.prefixes, &font.features.classes, &font.features.features] {
        list.len().hash(&mut hasher);
        for e in list {
            (&e.name, &e.code, e.disabled).hash(&mut hasher);
        }
    }
    for g in font.glyphs.as_arcs() {
        (&g.name, g.export, &g.unicodes).hash(&mut hasher);
    }
    for m in font.masters.iter() {
        (&m.id, &m.name).hash(&mut hasher);
        m.location.iter().chain(&m.number_values).for_each(|v| v.to_bits().hash(&mut hasher));
    }
    for inst in font.instances.iter() {
        (&inst.name, inst.exports, inst.kind.is_some()).hash(&mut hasher);
        inst.location.iter().for_each(|v| v.to_bits().hash(&mut hasher));
        // A parameter may add, replace or remove entries.
        format!("{:?}", inst.params).hash(&mut hasher);
    }
    format!("{:?}", font.axes).hash(&mut hasher);
    Some(hasher.finish())
}

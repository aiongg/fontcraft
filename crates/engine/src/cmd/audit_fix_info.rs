//! Audit fixes in Font Info (M12.63): the source side of the fixes that `docs/spec/21-qa.md` § 5
//! hands to Font Info. A finding on a compiled value is fixed where the compiler reads it: the
//! Font Info field or custom parameter of the font, a master or an instance, never the compiled
//! copy. Each command edits the open document as one undo step, so the check that asked for it
//! passes on the next compile; `dryRun` lists the edits without making them.
//!
//! | Fix id | Command | What it edits |
//! |---|---|---|
//! | `syncVerticalMetrics` | `font.info.syncVerticalMetrics` | the eight vertical-metric parameters of each master |
//! | `syncVersion` | `font.info.syncVersion` | every version string the source sets |
//! | `syncStyleLinking` | `font.instance.syncStyleLinking` | the style-linking settings of the static instances |
//! | `syncFamilyValues` | `font.info.syncFamilyValues` | vertical metrics and underline values, made equal for every font |
//! | `cleanNameRecords` | `font.info.cleanNames` | white space and empty values of the names |
//! | `cleanInstances` | `font.instance.cleanUp` | the order of the static instances and repeated ones |
//! | `sortZones` | `font.info.sortZones` | the alignment-zone pairs of the PostScript zone parameters |
//! | `clearInfoFields` | `font.info.clearGeneratedFields` | values the compiler generates, and the obsolete year |
//!
//! The values the compiler derives when a parameter is absent are the ones of
//! `fontcraft_compile`'s `info::metrics` (fontc's defaults), so a fix starts from what the font
//! compiles to today.

use std::sync::Arc;

use fontcraft_compile::{Flavour, Options, ot_round};
use fontcraft_model::plist::{Num, Number, Str, Value as Plist};
use fontcraft_model::{Font, Instance, Master, Params as InfoParams, Properties, PropertyValue, Target};
use serde_json::json;
use skrifa::raw::TableProvider as _;

use super::common::master_metrics;
use super::*;
use crate::EngineError;
use crate::params::Params;

const SYNC_VERTICAL: &str = "font.info.syncVerticalMetrics";
const SYNC_VERSION: &str = "font.info.syncVersion";
const SYNC_STYLE_LINKING: &str = "font.instance.syncStyleLinking";
const SYNC_FAMILY: &str = "font.info.syncFamilyValues";
const CLEAN_NAMES: &str = "font.info.cleanNames";
const CLEAN_INSTANCES: &str = "font.instance.cleanUp";
const SORT_ZONES: &str = "font.info.sortZones";
const CLEAR_FIELDS: &str = "font.info.clearGeneratedFields";

/// The fix ids of `docs/spec/21-qa.md` § 5 that this module carries out on a source, and the
/// command for each. `audit.fix` runs these for source findings without a glyph; a fix id of a
/// compiled-font fix (`cleanNameRecords`, `cleanInstances`) names its `tables.*` command in the
/// registry and this command when the target is a source.
pub(crate) const SOURCE_FIXES: [(&str, &str); 8] = [
    ("syncVerticalMetrics", SYNC_VERTICAL),
    ("syncVersion", SYNC_VERSION),
    ("syncStyleLinking", SYNC_STYLE_LINKING),
    ("syncFamilyValues", SYNC_FAMILY),
    ("cleanNameRecords", CLEAN_NAMES),
    ("cleanInstances", CLEAN_INSTANCES),
    ("sortZones", SORT_ZONES),
    ("clearInfoFields", CLEAR_FIELDS),
];

/// The command that carries out the fix `id` on a source, if this module has one.
#[allow(dead_code)] // `audit.fix` (M12.44) calls it once it fixes source findings.
pub(crate) fn source_command(id: &str) -> Option<&'static str> {
    SOURCE_FIXES.iter().find(|(fix, _)| *fix == id).map(|(_, command)| *command)
}

/// Most entries a `masters` list may hold.
const MAX_LISTED: usize = 1_024;
/// Most values a zone parameter may hold to be sorted (the CFF limit is 14 per list).
const MAX_ZONE_VALUES: usize = 256;
/// Most items of one list value written into the result.
const MAX_LOGGED_ITEMS: usize = 64;
/// Two instance coordinates closer than this are the same location.
const SAME_LOCATION: f64 = 1e-6;

/// The vertical metrics as custom parameters, in the order [`Vertical::values`] gives them.
const VERTICAL_KEYS: [&str; 8] =
    ["typoAscender", "typoDescender", "typoLineGap", "hheaAscender", "hheaDescender", "hheaLineGap", "winAscent", "winDescent"];
/// Whether the family value of each [`VERTICAL_KEYS`] entry is the lowest (descenders) rather
/// than the highest.
const VERTICAL_LOWEST: [bool; 8] = [false, true, false, false, true, false, false, false];
const UNDERLINE_KEYS: [&str; 2] = ["underlinePosition", "underlineThickness"];
/// The compiler's values when the font sets none.
const UNDERLINE_DEFAULTS: [f64; 2] = [-100.0, 50.0];

/// Where a source keeps a version string: the property and the parameters with the same name.
const VERSION_PROPERTY: &str = "versionString";
const VERSION_PARAMS: [&str; 2] = ["versionString", "openTypeNameVersion"];

/// The PostScript zone parameters, lists of bottom-top pairs.
const ZONE_PARAMS: [&str; 4] = ["postscriptBlueValues", "postscriptOtherBlues", "postscriptFamilyBlues", "postscriptFamilyOtherBlues"];

/// What `check.sources.unnecessaryInfoFields` reports: properties, then parameters.
const GENERATED_PROPERTIES: [&str; 2] = ["uniqueID", "versionString"];
const GENERATED_PARAMS: [&str; 6] = ["openTypeNameUniqueID", "openTypeNameVersion", "postscriptUniqueID", "year", "uniqueID", "versionString"];

/// Name properties that applications show in font menus (name IDs 1, 2, 4, 16, 17, 18 and the
/// style-map names): white space inside them is collapsed too.
const MENU_PROPERTIES: [&str; 9] = [
    "familyNames",
    "styleNames",
    "styleMapFamilyNames",
    "styleMapStyleNames",
    "preferredFamilyNames",
    "preferredSubfamilyNames",
    "compatibleFullNames",
    "fullName",
    "postscriptFullNames",
];
/// Parameters that hold a name record's text (the compiler's name parameters and the UFO
/// spellings), with whether each is a menu name.
const NAME_PARAMS: [(&str, bool); 22] = [
    ("copyright", false),
    ("trademark", false),
    ("manufacturer", false),
    ("designer", false),
    ("description", false),
    ("openTypeNameDescription", false),
    ("license", false),
    ("openTypeNameLicense", false),
    ("licenseURL", false),
    ("openTypeNameLicenseURL", false),
    ("versionString", false),
    ("openTypeNameVersion", false),
    ("uniqueID", false),
    ("openTypeNameUniqueID", false),
    ("postscriptFontName", false),
    ("postscriptFullName", true),
    ("styleMapFamilyName", true),
    ("styleMapStyleName", true),
    ("preferredFamilyName", true),
    ("preferredSubfamilyName", true),
    ("openTypeNamePreferredFamilyName", true),
    ("openTypeNamePreferredSubfamilyName", true),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.info.syncVerticalMetrics",
            "Sync Vertical Metrics",
            [],
            None,
            "{masters?: [id|name] (default every master), dryRun?: bool = false} gives each master one consistent set of vertical metrics, keeping its current line height: the typo and hhea ascenders become the hhea ascender plus the hhea line gap the master compiles to today, both descenders its hhea descender, both line gaps 0, and winAscent and winDescent the highest and lowest point of any glyph of any master. All eight are written as the master's custom parameters; without `masters` the same parameters are removed from the instances, which would override them. One undo step → {changed: [{scope: \"font\"|\"master\"|\"instance\", target, field, old, new}], dryRun}",
            has_doc,
            sync_vertical_metrics
        ),
        cmd!(
            "font.info.syncVersion",
            "Sync Version String",
            [],
            None,
            "{dryRun?: bool = false} rewrites every version string the source sets (the font's and the instances' versionString, and the versionString and openTypeNameVersion parameters) to `Version <major>.<minor, three digits>` from the font's version, keeping what follows the first `;`. A string with a placeholder the compiler fills in is left alone; a source without a version string needs no fix. One undo step → {changed: [{scope, target, field, old, new}], dryRun}",
            has_doc,
            sync_version
        ),
        cmd!(
            "font.instance.syncStyleLinking",
            "Sync Style Linking",
            [],
            None,
            "{dryRun?: bool = false} sets the style linking of each static instance from its style name: Bold, Italic and Bold Italic are linked to Regular with their Bold and Italic flags; Regular has neither flag and no link; another style with the word Italic is the italic of the style without that word (when the font has it) and keeps a Bold flag set by hand; the other styles are left as they are. The compiler derives fsSelection, macStyle and the subfamily name from these settings. One undo step → {changed: [{scope: \"instance\", target, field: \"isBold\"|\"isItalic\"|\"linkStyle\", old, new}], dryRun}",
            has_doc,
            sync_style_linking
        ),
        cmd!(
            "font.info.syncFamilyValues",
            "Sync Family Values",
            [],
            None,
            "{values?: [\"verticalMetrics\" | \"underline\"] (default both), dryRun?: bool = false} makes every font the source exports share the same values, for each group whose values differ between masters or that an instance overrides: the eight vertical metrics take the widest value of any master (highest ascenders, line gaps and win values, lowest descenders), written to every master; underlinePosition and underlineThickness take the first master's value, written to the font. The instances' and, for the underline, the masters' own values of the group are removed. One undo step → {changed: [{scope, target, field, old, new}], dryRun}",
            has_doc,
            sync_family_values
        ),
        cmd!(
            "font.info.cleanNames",
            "Clean Up Names",
            [],
            None,
            "{keepLineBreaks?: bool = false, dryRun?: bool = false} tidies every name the source sets (the family name, the instance names, the name properties of the font, masters and instances in every language, and the name parameters): line breaks become spaces unless keepLineBreaks, white space at either end is removed, runs of white space inside menu names become one space, and a value left empty is deleted (a property with no value left goes too). An empty family or instance name keeps its old value. One undo step → {changed: [{scope, target, field, old, new}], dryRun}",
            has_doc,
            clean_names
        ),
        cmd!(
            "font.instance.cleanUp",
            "Clean Up Instances",
            [],
            None,
            "{dryRun?: bool = false} removes each static instance that repeats an earlier one (same name, location and export setting) and sorts the static instances by their weight coordinate, then by their other coordinates in axis order, so the named instances of a variable font come in rising weight. Other instances keep their places. One undo step → {changed: [{scope: \"instance\", target, field: \"removed\"|\"order\", old, new}], dryRun}",
            has_doc,
            clean_instances
        ),
        cmd!(
            "font.info.sortZones",
            "Sort Alignment Zones",
            [],
            None,
            "{dryRun?: bool = false} puts the lower value first in each pair of the postscriptBlueValues, postscriptOtherBlues, postscriptFamilyBlues and postscriptFamilyOtherBlues parameters of the font, masters and instances. Zones kept as master alignment zones are always written in order. A list with a value that is not a number is left alone. One undo step → {changed: [{scope, target, field, old, new}], dryRun}",
            has_doc,
            sort_zones
        ),
        cmd!(
            "font.info.clearGeneratedFields",
            "Clear Generated Font Info",
            [],
            None,
            "{dryRun?: bool = false} removes from the font and its masters the values the compiler generates itself (unique ID, version string, PostScript unique ID, in property or parameter form) and the obsolete year. One undo step → {changed: [{scope, target, field, old, new: null}], dryRun}",
            has_doc,
            clear_generated_fields
        ),
    ]
}

// ── Shared parts ────────────────────────────────────────────────────────────────────────────────

/// The edits a fix makes, as its result lists them.
#[derive(Default)]
struct Log(Vec<Value>);

impl Log {
    fn note(&mut self, scope: &str, target: &str, field: &str, old: Value, new: Value) {
        self.0.push(json!({"scope": scope, "target": target, "field": field, "old": old, "new": new}));
    }
}

/// Runs `f` on the active font as one undo step labelled `label`, or on a copy when `dry`.
fn apply(s: &mut Session, label: &str, dry: bool, f: impl FnOnce(&mut Font, &mut Log) -> Result<()>) -> Result<Value> {
    let mut log = Log::default();
    if dry {
        let mut copy = Font::clone(s.font()?);
        f(&mut copy, &mut log)?;
    } else {
        s.edit(label, |font| f(font, &mut log))?;
    }
    Ok(json!({"changed": log.0, "dryRun": dry}))
}

/// A number, also written as text; `None` for anything else or a value that is not finite.
fn num(v: &Plist) -> Option<f64> {
    let n = match v {
        Plist::Number(n) => n.value().as_f64(),
        Plist::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// A rounded font-unit value as a parameter value.
fn int(v: f64) -> Plist {
    let r = ot_round(v).clamp(f64::from(i16::MIN), f64::from(u16::MAX));
    Plist::Number(Number::int(r as i64))
}

/// One parameter value for the result: numbers, text and flags as they are, a list item by item.
fn plist_json(v: &Plist) -> Value {
    match v {
        Plist::Array(a) => Value::Array(a.items.iter().take(MAX_LOGGED_ITEMS).map(scalar_json).collect()),
        other => scalar_json(other),
    }
}

fn scalar_json(v: &Plist) -> Value {
    match v {
        Plist::Number(n) => match n.value() {
            Num::Int(i) => json!(i),
            Num::Float(f) => json!(f),
        },
        Plist::String(s) | Plist::Date(s) => json!(s.as_str()),
        Plist::Bool(b) => json!(b),
        Plist::Array(_) => json!("(list)"),
        Plist::Dict(_) => json!("(dictionary)"),
        Plist::Data(_) => json!("(data)"),
    }
}

/// Sets parameter `name` to `value` unless it already holds it.
fn set_param(params: &mut InfoParams, name: &str, value: Plist, (scope, target): (&str, &str), log: &mut Log) {
    let old = params.get(name).cloned();
    if old.as_ref().is_some_and(|o| num(o).is_some() && num(o) == num(&value)) || old.as_ref() == Some(&value) {
        return;
    }
    log.note(scope, target, name, old.as_ref().map_or(Value::Null, plist_json), plist_json(&value));
    params.set(name, value);
}

/// Removes every row of parameter `name`, enabled or not.
fn remove_param(params: &mut InfoParams, name: &str, (scope, target): (&str, &str), log: &mut Log) {
    let old = params.0.iter().find(|p| p.name == name).map(|p| plist_json(&p.value));
    if params.remove(name) > 0 {
        log.note(scope, target, name, old.unwrap_or(Value::Null), Value::Null);
    }
}

/// The value of parameter `name` that applies to master `m`: its own, else the font's.
fn effective(font: &Font, m: &Master, name: &str) -> Option<f64> {
    m.params.get(name).or_else(|| font.info.params.get(name)).and_then(num)
}

/// Whether any instance sets one of `keys`.
fn instances_override(font: &Font, keys: &[&str]) -> bool {
    font.instances.iter().any(|i| keys.iter().any(|k| i.params.0.iter().any(|p| p.name == *k)))
}

fn remove_from_instances(font: &mut Font, keys: &[&str], log: &mut Log) {
    if !instances_override(font, keys) {
        return;
    }
    for inst in Arc::make_mut(&mut font.instances).iter_mut() {
        let name = inst.name.clone();
        for k in keys {
            remove_param(&mut inst.params, k, ("instance", &name), log);
        }
    }
}

/// The masters named by `masters` (ids or names), in font order; every master without it.
fn selected_masters(p: &Params, font: &Font) -> Result<Vec<usize>> {
    let Some(list) = p.opt_strings("masters")? else { return Ok((0..font.masters.len()).collect()) };
    if list.len() > MAX_LISTED {
        return Err(p.bad(format!("`masters` holds {} entries; at most {MAX_LISTED} are allowed", list.len())));
    }
    let mut out = Vec::new();
    for want in list {
        let i = font.masters.iter().position(|m| m.id == want || m.name == want).ok_or_else(|| p.bad(format!("there is no master `{want}`")))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out.sort_unstable();
    Ok(out)
}

// ── font.info.syncVerticalMetrics ───────────────────────────────────────────────────────────────

/// The vertical metrics a master compiles to, rounded as the compiler rounds them.
struct Vertical {
    typo: [f64; 3],
    hhea: [f64; 3],
    win: [f64; 2],
}

impl Vertical {
    /// The compiler's values for `m`: its parameters (or the font's), else the defaults derived
    /// from its ascender and descender.
    fn of(font: &Font, m: &Master) -> Self {
        let upm = f64::from(font.units_per_em);
        let metrics = master_metrics(font, m);
        let (asc, desc) = (metrics.ascender, metrics.descender);
        let get = |k: &str| effective(font, m, k);
        let gap = get("typoLineGap").unwrap_or_else(|| (upm * 1.2 + desc - asc).max(0.0));
        Self {
            typo: [get("typoAscender").unwrap_or(asc), get("typoDescender").unwrap_or(desc), gap].map(ot_round),
            hhea: [get("hheaAscender").unwrap_or(asc + gap), get("hheaDescender").unwrap_or(desc), get("hheaLineGap").unwrap_or(0.0)].map(ot_round),
            win: [get("winAscent").unwrap_or(asc + gap), get("winDescent").map_or(desc.abs(), f64::abs)].map(ot_round),
        }
    }

    /// In [`VERTICAL_KEYS`] order.
    fn values(&self) -> [f64; 8] {
        let ([ta, td, tg], [ha, hd, hg], [wa, wd]) = (self.typo, self.hhea, self.win);
        [ta, td, tg, ha, hd, hg, wa, wd]
    }
}

/// The highest and lowest point of any glyph of master `m`, as its compiled font's `head` table
/// gives them, counting 0 as `check.metrics.winAscentDescent` does.
fn extents(p: &Params, font: &Font, m: &Master) -> Result<(f64, f64)> {
    let compiled = fontcraft_compile::compile(font, &Target::Master(m.id.clone()), Flavour::TrueType, &Options::default())
        .map_err(|e| p.bad(format!("the master {} could not be compiled to measure its glyphs: {e}", m.name)))?;
    let font = skrifa::FontRef::new(&compiled.bytes).map_err(|e| EngineError::Internal(format!("the compiled master cannot be read: {e}")))?;
    let head = font.head().map_err(|e| EngineError::Internal(format!("the compiled master has no readable head table: {e}")))?;
    Ok((f64::from(head.y_max().max(0)), f64::from(head.y_min().min(0))))
}

fn sync_vertical_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(SYNC_VERTICAL, v)?;
    let dry = p.bool_or("dryRun", false)?;
    let whole = p.get("masters").is_none();
    let font = s.font()?;
    let picked = selected_masters(&p, font)?;
    let (mut top, mut bottom) = (0.0_f64, 0.0_f64);
    for m in font.masters.iter() {
        let (high, low) = extents(&p, font, m)?;
        top = top.max(high);
        bottom = bottom.max(-low);
    }
    let plans: Vec<(usize, [f64; 8])> = picked
        .iter()
        .filter_map(|&i| {
            let m = font.masters.get(i)?;
            let now = Vertical::of(font, m);
            let [ha, hd, hg] = now.hhea;
            let (ascent, descent) = (ha + hg, hd);
            Some((i, [ascent, descent, 0.0, ascent, descent, 0.0, top, bottom]))
        })
        .collect();
    apply(s, "Sync Vertical Metrics", dry, |font, log| {
        let masters = Arc::make_mut(&mut font.masters);
        for (i, values) in &plans {
            let Some(m) = masters.get_mut(*i) else { continue };
            let name = m.name.clone();
            for (key, value) in VERTICAL_KEYS.iter().zip(values) {
                set_param(&mut m.params, key, int(*value), ("master", &name), log);
            }
        }
        if whole {
            remove_from_instances(font, &VERTICAL_KEYS, log);
        }
        Ok(())
    })
}

// ── font.info.syncVersion ───────────────────────────────────────────────────────────────────────

/// `old` with its version part replaced by the font's version, keeping what follows the first
/// `;`. `None` when nothing changes or `old` holds a placeholder the compiler fills in.
fn version_text(old: &str, major: u16, minor: u16) -> Option<String> {
    let cut = old.find(';').unwrap_or(old.len());
    let (head, tail) = (old.get(..cut).unwrap_or(old), old.get(cut..).unwrap_or(""));
    if head.contains("{{") || head.contains('%') {
        return None;
    }
    let new = format!("Version {major}.{minor:03}{tail}");
    (new != old).then_some(new)
}

/// Rewrites each value of property `key` that `f` changes.
fn edit_property(props: &mut Properties, key: &str, (scope, target): (&str, &str), log: &mut Log, f: impl Fn(&str) -> Option<String>) {
    for prop in props.0.iter_mut().filter(|p| p.key == key) {
        match &mut prop.value {
            PropertyValue::Single(text) => {
                if let Some(new) = f(text) {
                    log.note(scope, target, key, json!(text), json!(new));
                    *text = new;
                }
            }
            PropertyValue::Localized(values) => {
                for l in values.iter_mut() {
                    if let Some(new) = f(&l.value) {
                        log.note(scope, target, &format!("{key}[{}]", l.language), json!(l.value), json!(new));
                        l.value = new;
                    }
                }
            }
        }
    }
}

/// Rewrites each text parameter called `name` that `f` changes.
fn edit_text_params(params: &mut InfoParams, name: &str, (scope, target): (&str, &str), log: &mut Log, f: impl Fn(&str) -> Option<String>) {
    for row in params.0.iter_mut().filter(|r| r.name == name) {
        let Plist::String(text) = &row.value else { continue };
        if let Some(new) = f(text.as_str()) {
            log.note(scope, target, name, json!(text.as_str()), json!(new));
            row.value = Plist::String(Str::new(new));
        }
    }
}

fn sync_version(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(SYNC_VERSION, v)?;
    let dry = p.bool_or("dryRun", false)?;
    apply(s, "Sync Version String", dry, |font, log| {
        let (major, minor) = (font.version_major, font.version_minor);
        let fix = |old: &str| version_text(old, major, minor);
        let info = Arc::make_mut(&mut font.info);
        edit_property(&mut info.properties, VERSION_PROPERTY, ("font", ""), log, fix);
        for name in VERSION_PARAMS {
            edit_text_params(&mut info.params, name, ("font", ""), log, fix);
        }
        let touches =
            |i: &Instance| i.properties.get(VERSION_PROPERTY).is_some() || VERSION_PARAMS.iter().any(|n| i.params.0.iter().any(|r| r.name == *n));
        if font.instances.iter().any(touches) {
            for inst in Arc::make_mut(&mut font.instances).iter_mut() {
                let name = inst.name.clone();
                edit_property(&mut inst.properties, VERSION_PROPERTY, ("instance", &name), log, fix);
                for param in VERSION_PARAMS {
                    edit_text_params(&mut inst.params, param, ("instance", &name), log, fix);
                }
            }
        }
        Ok(())
    })
}

// ── font.instance.syncStyleLinking ──────────────────────────────────────────────────────────────

/// The style linking the handbook recommends for an instance called `name`: (bold, italic,
/// base style), or `None` to leave the instance alone. `styles` are the names of the font's
/// static instances; a link is made only to one of them.
fn recommended_link(name: &str, inst: &Instance, styles: &[&str]) -> Option<(bool, bool, Option<String>)> {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let italic = words.contains(&"Italic");
    let base = words.iter().filter(|w| **w != "Italic").copied().collect::<Vec<_>>().join(" ");
    let link_to = |style: &str| styles.contains(&style).then(|| style.to_string());
    match (base.as_str(), italic) {
        ("Regular", false) => Some((false, false, None)),
        ("Bold", _) => Some((true, italic, link_to("Regular"))),
        ("" | "Regular", true) => Some((false, true, link_to("Regular"))),
        (_, true) if inst.is_bold => Some((true, true, inst.link_style.clone())),
        (other, true) => Some((false, true, link_to(other).or_else(|| inst.link_style.clone()))),
        _ => None,
    }
}

fn sync_style_linking(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(SYNC_STYLE_LINKING, v)?;
    let dry = p.bool_or("dryRun", false)?;
    apply(s, "Sync Style Linking", dry, |font, log| {
        let styles: Vec<String> = font.instances.iter().filter(|i| i.kind.is_none()).map(|i| i.name.trim().to_string()).collect();
        let styles: Vec<&str> = styles.iter().map(String::as_str).collect();
        let plans: Vec<(usize, (bool, bool, Option<String>))> = font
            .instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind.is_none())
            .filter_map(|(n, i)| recommended_link(&i.name, i, &styles).map(|want| (n, want)))
            .collect();
        let link_of = |l: &Option<String>| l.clone().filter(|l| !l.is_empty());
        let differs = |i: &Instance, (bold, italic, link): &(bool, bool, Option<String>)| {
            i.is_bold != *bold || i.is_italic != *italic || link_of(&i.link_style) != *link
        };
        if !plans.iter().any(|(n, want)| font.instances.get(*n).is_some_and(|i| differs(i, want))) {
            return Ok(());
        }
        let instances = Arc::make_mut(&mut font.instances);
        for (n, (bold, italic, link)) in plans {
            let Some(inst) = instances.get_mut(n) else { continue };
            let name = inst.name.clone();
            if inst.is_bold != bold {
                log.note("instance", &name, "isBold", json!(inst.is_bold), json!(bold));
                inst.is_bold = bold;
            }
            if inst.is_italic != italic {
                log.note("instance", &name, "isItalic", json!(inst.is_italic), json!(italic));
                inst.is_italic = italic;
            }
            if link_of(&inst.link_style) != link {
                log.note("instance", &name, "linkStyle", json!(link_of(&inst.link_style)), json!(link));
                inst.link_style = link;
            }
        }
        Ok(())
    })
}

// ── font.info.syncFamilyValues ──────────────────────────────────────────────────────────────────

fn sync_family_values(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(SYNC_FAMILY, v)?;
    let dry = p.bool_or("dryRun", false)?;
    let (mut vertical, mut underline) = (true, true);
    if let Some(groups) = p.opt_strings("values")? {
        (vertical, underline) = (false, false);
        for g in groups {
            match g {
                "verticalMetrics" => vertical = true,
                "underline" => underline = true,
                other => return Err(p.bad(format!("`values`: `{other}` is not \"verticalMetrics\" or \"underline\""))),
            }
        }
    }
    apply(s, "Sync Family Values", dry, |font, log| {
        if vertical {
            sync_family_vertical(font, log);
        }
        if underline {
            sync_family_underline(font, log);
        }
        Ok(())
    })
}

/// The widest vertical metrics of any master, written to every master.
fn sync_family_vertical(font: &mut Font, log: &mut Log) {
    let all: Vec<[f64; 8]> = {
        let f: &Font = font;
        f.masters.iter().map(|m| Vertical::of(f, m).values()).collect()
    };
    let Some(first) = all.first().copied() else { return };
    if all.iter().all(|v| *v == first) && !instances_override(font, &VERTICAL_KEYS) {
        return;
    }
    let mut widest = first;
    for values in all.iter().skip(1) {
        for ((w, v), lowest) in widest.iter_mut().zip(values).zip(VERTICAL_LOWEST) {
            *w = if lowest { w.min(*v) } else { w.max(*v) };
        }
    }
    for m in Arc::make_mut(&mut font.masters).iter_mut() {
        let name = m.name.clone();
        for (key, value) in VERTICAL_KEYS.iter().zip(widest) {
            set_param(&mut m.params, key, int(value), ("master", &name), log);
        }
    }
    remove_from_instances(font, &VERTICAL_KEYS, log);
}

/// The first master's underline values, written to the font in place of the masters' own.
fn sync_family_underline(font: &mut Font, log: &mut Log) {
    let all: Vec<[f64; 2]> = {
        let f: &Font = font;
        let [k0, k1] = UNDERLINE_KEYS;
        let [d0, d1] = UNDERLINE_DEFAULTS;
        f.masters.iter().map(|m| [effective(f, m, k0).unwrap_or(d0), effective(f, m, k1).unwrap_or(d1)]).collect()
    };
    let Some(first) = all.first().copied() else { return };
    let masters_set = font.masters.iter().any(|m| UNDERLINE_KEYS.iter().any(|k| m.params.0.iter().any(|p| p.name == *k)));
    if all.iter().all(|v| *v == first) && !masters_set && !instances_override(font, &UNDERLINE_KEYS) {
        return;
    }
    let info = Arc::make_mut(&mut font.info);
    for (key, value) in UNDERLINE_KEYS.iter().zip(first) {
        set_param(&mut info.params, key, int(value), ("font", ""), log);
    }
    if masters_set {
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            let name = m.name.clone();
            for key in UNDERLINE_KEYS {
                remove_param(&mut m.params, key, ("master", &name), log);
            }
        }
    }
    remove_from_instances(font, &UNDERLINE_KEYS, log);
}

// ── font.info.cleanNames ────────────────────────────────────────────────────────────────────────

/// `text` with line breaks turned into spaces (unless `keep_breaks`), white space at either end
/// removed and, in a menu name, each run of white space made one space.
fn clean_text(text: &str, menu: bool, keep_breaks: bool) -> String {
    let mut out = String::with_capacity(text.len());
    // After a replaced line break or a collapsed space, further white space is dropped.
    let mut swallow = false;
    for c in text.chars() {
        let line_break = matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if line_break && !keep_breaks {
            let kept = out.trim_end_matches([' ', '\t']).len();
            out.truncate(kept);
            out.push(' ');
            swallow = true;
            continue;
        }
        let blank = if menu { c.is_whitespace() } else { c == ' ' || c == '\t' };
        if blank && swallow {
            continue;
        }
        if blank && menu {
            out.push(' ');
            swallow = true;
            continue;
        }
        swallow = false;
        out.push(c);
    }
    out.trim().to_string()
}

/// Cleans every name property of `props`; values left empty are deleted.
fn clean_properties(props: &mut Properties, keep_breaks: bool, (scope, target): (&str, &str), log: &mut Log) {
    for prop in props.0.iter_mut() {
        let menu = MENU_PROPERTIES.contains(&prop.key.as_str());
        let key = prop.key.clone();
        match &mut prop.value {
            PropertyValue::Single(text) => {
                let new = clean_text(text, menu, keep_breaks);
                if new != *text {
                    log.note(scope, target, &key, json!(text), if new.is_empty() { Value::Null } else { json!(new) });
                    *text = new;
                }
            }
            PropertyValue::Localized(values) => {
                for l in values.iter_mut() {
                    let new = clean_text(&l.value, menu, keep_breaks);
                    if new != l.value {
                        let field = format!("{key}[{}]", l.language);
                        log.note(scope, target, &field, json!(l.value), if new.is_empty() { Value::Null } else { json!(new) });
                        l.value = new;
                    }
                }
                values.retain(|l| !l.value.is_empty());
            }
        }
    }
    props.0.retain(|p| match &p.value {
        PropertyValue::Single(text) => !text.is_empty(),
        PropertyValue::Localized(values) => !values.is_empty(),
    });
}

/// Cleans the name parameters of `params`; rows left empty are deleted.
fn clean_params(params: &mut InfoParams, keep_breaks: bool, (scope, target): (&str, &str), log: &mut Log) {
    for row in params.0.iter_mut() {
        let Some((_, menu)) = NAME_PARAMS.iter().find(|(n, _)| *n == row.name) else { continue };
        let Plist::String(text) = &row.value else { continue };
        let new = clean_text(text.as_str(), *menu, keep_breaks);
        if new != text.as_str() {
            log.note(scope, target, &row.name, json!(text.as_str()), if new.is_empty() { Value::Null } else { json!(new) });
            row.value = Plist::String(Str::new(new));
        }
    }
    params.0.retain(|row| !(NAME_PARAMS.iter().any(|(n, _)| *n == row.name) && matches!(&row.value, Plist::String(t) if t.as_str().is_empty())));
}

fn clean_names(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(CLEAN_NAMES, v)?;
    let dry = p.bool_or("dryRun", false)?;
    let keep = p.bool_or("keepLineBreaks", false)?;
    apply(s, "Clean Up Names", dry, |font, log| {
        let family = clean_text(&font.family_name, true, keep);
        if family != font.family_name && !family.is_empty() {
            log.note("font", "", "familyName", json!(font.family_name), json!(family));
            font.family_name = family;
        }
        // Each part is copied only when something in it changes.
        let mut info = FontInfoParts::of(&font.info.properties, &font.info.params);
        if info.clean(keep, ("font", ""), log) {
            let target = Arc::make_mut(&mut font.info);
            (target.properties, target.params) = (info.properties, info.params);
        }
        let mut masters: Vec<Master> = font.masters.to_vec();
        let mut masters_changed = false;
        for m in masters.iter_mut() {
            let name = m.name.clone();
            let mut parts = FontInfoParts::of(&m.properties, &m.params);
            if parts.clean(keep, ("master", &name), log) {
                (m.properties, m.params) = (parts.properties, parts.params);
                masters_changed = true;
            }
        }
        if masters_changed {
            font.masters = Arc::new(masters);
        }
        let mut instances: Vec<Instance> = font.instances.to_vec();
        let mut instances_changed = false;
        for inst in instances.iter_mut() {
            let old = inst.name.clone();
            let new = clean_text(&old, true, keep);
            if new != old && !new.is_empty() {
                log.note("instance", &old, "name", json!(old), json!(new));
                inst.name = new;
                instances_changed = true;
            }
            let name = inst.name.clone();
            let mut parts = FontInfoParts::of(&inst.properties, &inst.params);
            if parts.clean(keep, ("instance", &name), log) {
                (inst.properties, inst.params) = (parts.properties, parts.params);
                instances_changed = true;
            }
        }
        if instances_changed {
            font.instances = Arc::new(instances);
        }
        Ok(())
    })
}

/// A copy of one level's names, cleaned apart so an unchanged level stays shared.
struct FontInfoParts {
    properties: Properties,
    params: InfoParams,
}

impl FontInfoParts {
    fn of(properties: &Properties, params: &InfoParams) -> Self {
        Self { properties: properties.clone(), params: params.clone() }
    }

    /// Cleans the names; whether anything changed.
    fn clean(&mut self, keep_breaks: bool, at: (&str, &str), log: &mut Log) -> bool {
        let before = log.0.len();
        clean_properties(&mut self.properties, keep_breaks, at, log);
        clean_params(&mut self.params, keep_breaks, at, log);
        log.0.len() != before
    }
}

// ── font.instance.cleanUp ───────────────────────────────────────────────────────────────────────

/// Coordinate `axis` of `location`; a missing one is 0, as the model reads it.
fn coordinate(location: &[f64], axis: usize) -> f64 {
    location.get(axis).copied().unwrap_or(0.0)
}

fn same_instance(a: &Instance, b: &Instance, axes: usize) -> bool {
    a.name == b.name
        && a.exports == b.exports
        && (0..axes.max(a.location.len()).max(b.location.len()))
            .all(|i| (coordinate(&a.location, i) - coordinate(&b.location, i)).abs() <= SAME_LOCATION)
}

/// Rising weight, then the other coordinates in axis order.
fn instance_order(a: &Instance, b: &Instance, weight: Option<usize>, axes: usize) -> std::cmp::Ordering {
    let key = |i: &Instance| weight.into_iter().chain(0..axes).map(|axis| coordinate(&i.location, axis)).collect::<Vec<f64>>();
    let (ka, kb) = (key(a), key(b));
    ka.iter().zip(&kb).map(|(x, y)| x.total_cmp(y)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
}

fn clean_instances(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(CLEAN_INSTANCES, v)?;
    let dry = p.bool_or("dryRun", false)?;
    apply(s, "Clean Up Instances", dry, |font, log| {
        let axes = font.axes.len();
        let weight = font.axes.iter().position(|a| a.tag == "wght");
        let mut kept: Vec<Instance> = Vec::new();
        for inst in font.instances.iter().filter(|i| i.kind.is_none()) {
            if kept.iter().any(|k| same_instance(k, inst, axes)) {
                log.note("instance", &inst.name, "removed", json!(inst.name), Value::Null);
            } else {
                kept.push(inst.clone());
            }
        }
        let before: Vec<String> = kept.iter().map(|i| i.name.clone()).collect();
        kept.sort_by(|a, b| instance_order(a, b, weight, axes));
        let after: Vec<String> = kept.iter().map(|i| i.name.clone()).collect();
        if before != after {
            log.note("instance", "", "order", json!(before), json!(after));
        }
        if log.0.is_empty() {
            return Ok(());
        }
        // Static instances fill the static slots in their new order; the others stay put.
        let mut sorted = kept.into_iter();
        let mut out = Vec::with_capacity(font.instances.len());
        for inst in font.instances.iter() {
            if inst.kind.is_some() {
                out.push(inst.clone());
            } else if let Some(next) = sorted.next() {
                out.push(next);
            }
        }
        font.instances = Arc::new(out);
        Ok(())
    })
}

// ── font.info.sortZones ─────────────────────────────────────────────────────────────────────────

/// `v` with the lower value first in each pair, or `None` when it is in order already, is not a
/// list of numbers or is too long.
fn sorted_pairs(v: &Plist) -> Option<Plist> {
    let list = v.as_array()?;
    if list.items.len() > MAX_ZONE_VALUES || !list.items.iter().all(|x| num(x).is_some()) {
        return None;
    }
    let mut items = list.items.clone();
    let mut changed = false;
    let (pairs, _) = items.as_chunks_mut::<2>();
    for [a, b] in pairs.iter_mut() {
        if num(a) > num(b) {
            std::mem::swap(a, b);
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    let mut out = list.clone();
    out.items = items;
    Some(Plist::Array(out))
}

/// Sorts the zone parameters of one level; whether anything changed.
fn sort_zone_params(params: &mut InfoParams, (scope, target): (&str, &str), log: &mut Log) -> bool {
    let mut changed = false;
    for row in params.0.iter_mut().filter(|r| ZONE_PARAMS.contains(&r.name.as_str())) {
        if let Some(new) = sorted_pairs(&row.value) {
            log.note(scope, target, &row.name, plist_json(&row.value), plist_json(&new));
            row.value = new;
            changed = true;
        }
    }
    changed
}

fn has_unsorted(params: &InfoParams) -> bool {
    params.0.iter().any(|r| ZONE_PARAMS.contains(&r.name.as_str()) && sorted_pairs(&r.value).is_some())
}

fn sort_zones(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(SORT_ZONES, v)?;
    let dry = p.bool_or("dryRun", false)?;
    apply(s, "Sort Alignment Zones", dry, |font, log| {
        if has_unsorted(&font.info.params) {
            sort_zone_params(&mut Arc::make_mut(&mut font.info).params, ("font", ""), log);
        }
        if font.masters.iter().any(|m| has_unsorted(&m.params)) {
            for m in Arc::make_mut(&mut font.masters).iter_mut() {
                let name = m.name.clone();
                sort_zone_params(&mut m.params, ("master", &name), log);
            }
        }
        if font.instances.iter().any(|i| has_unsorted(&i.params)) {
            for inst in Arc::make_mut(&mut font.instances).iter_mut() {
                let name = inst.name.clone();
                sort_zone_params(&mut inst.params, ("instance", &name), log);
            }
        }
        Ok(())
    })
}

// ── font.info.clearGeneratedFields ──────────────────────────────────────────────────────────────

fn sets_generated(properties: &Properties, params: &InfoParams) -> bool {
    GENERATED_PROPERTIES.iter().any(|k| properties.get(k).is_some()) || GENERATED_PARAMS.iter().any(|k| params.0.iter().any(|p| p.name == *k))
}

fn clear_level(properties: &mut Properties, params: &mut InfoParams, (scope, target): (&str, &str), log: &mut Log) {
    for key in GENERATED_PROPERTIES {
        let old = properties.get(key).and_then(|p| p.default_value()).map(str::to_string);
        if properties.remove(key) {
            log.note(scope, target, key, json!(old), Value::Null);
        }
    }
    for key in GENERATED_PARAMS {
        remove_param(params, key, (scope, target), log);
    }
}

fn clear_generated_fields(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(CLEAR_FIELDS, v)?;
    let dry = p.bool_or("dryRun", false)?;
    apply(s, "Clear Generated Font Info", dry, |font, log| {
        if sets_generated(&font.info.properties, &font.info.params) {
            let info = Arc::make_mut(&mut font.info);
            clear_level(&mut info.properties, &mut info.params, ("font", ""), log);
        }
        if font.masters.iter().any(|m| sets_generated(&m.properties, &m.params)) {
            for m in Arc::make_mut(&mut font.masters).iter_mut() {
                let name = m.name.clone();
                clear_level(&mut m.properties, &mut m.params, ("master", &name), log);
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_trims_collapses_and_replaces_breaks() {
        assert_eq!(clean_text("  Sans  Serif ", true, false), "Sans Serif");
        assert_eq!(clean_text("Line one\r\n  line two ", false, false), "Line one line two");
        assert_eq!(clean_text("Keep  two\nlines ", false, true), "Keep  two\nlines");
        assert_eq!(clean_text(" \n ", true, false), "");
    }

    #[test]
    fn version_text_keeps_the_suffix_and_placeholders() {
        assert_eq!(version_text("Version 0.5; build 7", 1, 2).as_deref(), Some("Version 1.002; build 7"));
        assert_eq!(version_text("Version 1.002", 1, 2), None);
        assert_eq!(version_text("Version %d.%03d", 1, 2), None);
        assert_eq!(version_text("", 2, 10).as_deref(), Some("Version 2.010"));
    }

    #[test]
    fn sorted_pairs_swaps_only_reversed_pairs() {
        let list = |v: &[i64]| {
            let mut a = fontcraft_model::plist::Array::default();
            a.items = v.iter().map(|n| Plist::Number(Number::int(*n))).collect();
            Plist::Array(a)
        };
        assert_eq!(sorted_pairs(&list(&[-15, 0, 715, 700, 500])), Some(list(&[-15, 0, 700, 715, 500])));
        assert_eq!(sorted_pairs(&list(&[-15, 0, 700, 715])), None);
        let mut text = fontcraft_model::plist::Array::default();
        text.items = vec![Plist::String(Str::new("x")), Plist::Number(Number::int(0))];
        assert_eq!(sorted_pairs(&Plist::Array(text)), None, "a list with text is left alone");
    }

    #[test]
    fn style_names_give_the_recommended_links() {
        let plain = Instance::default();
        let styles = ["Regular", "Bold", "Italic", "Medium", "Medium Italic"];
        assert_eq!(recommended_link("Bold Italic", &plain, &styles), Some((true, true, Some("Regular".into()))));
        assert_eq!(recommended_link("Italic", &plain, &styles), Some((false, true, Some("Regular".into()))));
        assert_eq!(recommended_link("Medium Italic", &plain, &styles), Some((false, true, Some("Medium".into()))));
        assert_eq!(recommended_link("Regular", &plain, &styles), Some((false, false, None)));
        assert_eq!(recommended_link("Medium", &plain, &styles), None, "other styles keep their settings");
        let hand = Instance { is_bold: true, link_style: Some("Light Italic".into()), ..Instance::default() };
        assert_eq!(recommended_link("Semibold Italic", &hand, &styles), Some((true, true, Some("Light Italic".into()))));
    }
}

//! glyphsLib's lib keys for the font and master data a `fontinfo.plist` has no key for: the font's
//! settings, app version, edit-tab texts, user data and custom parameters, and a master's icon,
//! name parts, custom parameters, position and axis values.
//!
//! Ported from glyphsLib 6.15.0 (Apache-2.0, see `NOTICE`): `builder/custom_params.py` (the
//! parameter handlers and the generic `com.schriftgestaltung.customParameter.<class>.<name>`
//! keys), `builder/font.py`, `builder/masters.py`, `builder/names.py`, `builder/user_data.py`,
//! `builder/common.py` (`to_ufo_color`) and `builder/constants.py`.
//!
//! | Model | Lib key (every master's UFO for the font's data) |
//! |---|---|
//! | the app version | `com.schriftgestaltung.appVersion` |
//! | settings: keyboard increment, automatic alignment, nice names | `com.schriftgestaltung.keyboardIncrement`, `…customParameter.GSFont.disablesAutomaticAlignment`, `…customParameter.GSFont.useNiceNames` (the opposite of `disablesNiceNames`, as 0 or 1) |
//! | the edit-tab texts | `…customParameter.GSFont.DisplayStrings` |
//! | the font's user data, in a single UFO | `com.schriftgestaltung.Glyphs.fontUserData` (in a designspace: the document's `<lib>`) |
//! | font parameters with no `fontinfo.plist` key | `…customParameter.GSFont.<name>`; `Virtual Master` as one list; `Disable Last Change` as `…disablesLastChange` |
//! | `Color Palettes`, `meta Table`, `Don't use Production Names` | `com.github.googlei18n.ufo2ft.colorPalettes` (components from 0 to 1), `public.openTypeMeta` (tag → data), `com.github.googlei18n.ufo2ft.useProductionNames` (negated) |
//! | a master's icon | `…customParameter.GSFontMaster.iconName` |
//! | a master's name parts | `com.schriftgestaltung.weight`, `…width`, `…customName` |
//! | master parameters with no `fontinfo.plist` key | `…customParameter.GSFontMaster.<name>` |
//! | the master's position and its first six design coordinates | `com.schriftgestaltung.fontMasterOrder`; `…customParameter.GSFontMaster.weightValue` … `customValue3` and `com.schriftgestaltung.weightValue`…; written, and read only to check that they still say what the model says |
//! | the master id, in a single UFO | `com.schriftgestaltung.fontMasterID` (a designspace's source `name` holds it) |
//!
//! **Defaults.** glyphsLib writes several keys whatever their value (`useNiceNames = 1`,
//! `iconName = ""`, `weight = Regular`…). A new UFO of a document with a `.glyphs` format version
//! gets them, as glyphsLib would write it; a UFO read from disk keeps the ones it had; other new
//! UFOs get only the keys whose fields hold a value. A default value reads as no value.
//!
//! **Reading is checked by writing.** [`read`] types a key only when the writer gives the same
//! value for it; the others stay user data, written as they are. A font-wide key in a master
//! other than the default one is not user data while it says what the font says.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use fontcraft_model::axes::{self, AXIS_MAPPINGS, ORIGIN};
use fontcraft_model::{Family, Font, Master, Param, Params, UserData};
use fontcraft_plist::{Array, Dict, Num, Number, Str, Value};

use crate::fontinfo;

const FONT_PARAM: &str = "com.schriftgestaltung.customParameter.GSFont.";
const MASTER_PARAM: &str = "com.schriftgestaltung.customParameter.GSFontMaster.";
const PLAIN: &str = "com.schriftgestaltung.";
const APP_VERSION: &str = "com.schriftgestaltung.appVersion";
const KEYBOARD_INCREMENT: &str = "com.schriftgestaltung.keyboardIncrement";
const MASTER_ORDER: &str = "com.schriftgestaltung.fontMasterOrder";
const MASTER_ID: &str = "com.schriftgestaltung.fontMasterID";
const AUTO_ALIGNMENT: &str = "com.schriftgestaltung.customParameter.GSFont.disablesAutomaticAlignment";
const NICE_NAMES: &str = "com.schriftgestaltung.customParameter.GSFont.useNiceNames";
const DISPLAY_STRINGS: &str = "com.schriftgestaltung.customParameter.GSFont.DisplayStrings";
const ICON: &str = "com.schriftgestaltung.customParameter.GSFontMaster.iconName";
const WEIGHT: &str = "com.schriftgestaltung.weight";
const WIDTH: &str = "com.schriftgestaltung.width";
const CUSTOM: &str = "com.schriftgestaltung.customName";
/// The font's user data in a single UFO.
pub(crate) const FONT_USER_DATA: &str = "com.schriftgestaltung.Glyphs.fontUserData";
/// The master id the UFO reader gives a single UFO without [`MASTER_ID`].
pub(crate) const SINGLE_MASTER_ID: &str = "m01";
/// The name part glyphsLib writes when a master has none.
const REGULAR: &str = "Regular";
/// glyphsLib's names of a master's first six design coordinates.
const AXIS_VALUES: [&str; 6] = ["weightValue", "widthValue", "customValue", "customValue1", "customValue2", "customValue3"];
/// Keys after [`FONT_PARAM`] that hold font attributes, not parameters.
const FONT_ATTRS: [&str; 3] = ["disablesAutomaticAlignment", "useNiceNames", "DisplayStrings"];
/// Keys after [`MASTER_PARAM`] that hold master attributes, not parameters.
const MASTER_ATTRS: [&str; 7] = ["iconName", "weightValue", "widthValue", "customValue", "customValue1", "customValue2", "customValue3"];

/// How a parameter with a key of its own is converted.
#[derive(Clone, Copy, PartialEq)]
enum Conv {
    Palettes,
    Meta,
    Negated,
}

/// Parameters written to a key of their own, at the font and at a master (the master's wins):
/// (parameter, key, conversion).
const OWN_KEYS: [(&str, &str, Conv); 3] = [
    ("Color Palettes", "com.github.googlei18n.ufo2ft.colorPalettes", Conv::Palettes),
    ("meta Table", "public.openTypeMeta", Conv::Meta),
    ("Don't use Production Names", "com.github.googlei18n.ufo2ft.useProductionNames", Conv::Negated),
];
/// A parameter written under another name after the class prefix.
const RENAMED: [(&str, &str); 1] = [("Disable Last Change", "disablesLastChange")];
/// The font parameter that may repeat, written as one list.
const LISTED: &str = "Virtual Master";
/// Parameters glyphsLib reads as booleans (`classes.py`, `_CUSTOM_BOOL_PARAMS`) among those this
/// module writes; the `.glyphs` side holds them as 0 or 1.
const BOOLEAN: [&str; 4] = ["DisableAllAutomaticBehaviour", "Use Extension Kerning", "Disable Subroutines", "Disable Last Change"];
/// Parameters glyphsLib applies to the UFO's contents, or that another part of the file holds, so
/// that no lib key carries them.
const ONE_WAY: [&str; 9] = [
    "Filter",
    "PreFilter",
    "Replace Prefix",
    "Replace Feature",
    "Reencode Glyphs",
    "Rename Glyphs",
    "glyphOrder",
    "UFO Filename",
    "Name Table Entry",
];
/// Font parameters a designspace document holds, not repeated in its UFOs.
const IN_DOCUMENT: [&str; 2] = [AXIS_MAPPINGS, ORIGIN];
/// Passes [`read`] makes before it gives up typing.
const MAX_PASSES: usize = 8;

/// Whether this module writes `p` to the lib, `info` being the master's `fontinfo.plist` as read:
/// an enabled parameter with no `fontinfo.plist` key that glyphsLib does not apply elsewhere.
pub(crate) fn carried(p: &Param, info: Option<&Dict>) -> bool {
    !p.disabled && !fontinfo::is_raw(p) && !ONE_WAY.contains(&p.name.as_str()) && !fontinfo::holds(&p.name, info)
}

fn int(i: i64) -> Value {
    Value::Number(Number::int(i))
}

fn string(s: &str) -> Value {
    Value::String(Str::new(s))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.value().as_f64() != 0.0,
        Value::String(s) => !s.as_str().is_empty() && s.as_str() != "0",
        _ => false,
    }
}

/// A value with every dictionary in it sorted by key, as fontTools writes property lists.
pub(crate) fn sorted_keys(v: Value) -> Value {
    // Depth is bounded by the reader's limit for parsed values, and small for built ones.
    match v {
        Value::Dict(mut d) => {
            d.entries = d.entries.into_iter().map(|(k, v)| (k, sorted_keys(v))).collect();
            d.entries.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
            d.spacing = Default::default();
            Value::Dict(d)
        }
        Value::Array(mut a) => {
            a.items = a.items.into_iter().map(sorted_keys).collect();
            Value::Array(a)
        }
        other => other,
    }
}

fn array(items: Vec<Value>) -> Value {
    Value::Array(Array { items, ..Default::default() })
}

// ── conversions ────────────────────────────────────────────────────────────────────────────────

/// A colour of the reference's format (8-bit components, as numbers or as `"r,g,b,a"`; two of
/// them for grey) as ufo2ft's components from 0 to 1.
fn color_to_ufo(c: &Value) -> Option<Value> {
    let parts: Vec<f64> = match c {
        Value::String(s) => s.as_str().split(',').map(|p| p.trim().parse::<f64>().ok()).collect::<Option<_>>()?,
        Value::Array(a) => a.items.iter().map(|v| v.as_num().map(Num::as_f64)).collect::<Option<_>>()?,
        _ => return None,
    };
    let rgba = match parts.as_slice() {
        [g, a] => [*g, *g, *g, *a],
        [r, g, b, a] => [*r, *g, *b, *a],
        _ => return None,
    };
    let items = rgba.iter().map(|v| Number::float(*v / 255.0).map(Value::Number)).collect::<Option<Vec<_>>>()?;
    Some(array(items))
}

/// ufo2ft's colour as 8-bit components, two for grey, as glyphsLib reads it back.
fn color_to_model(c: &Value) -> Option<Value> {
    let parts: Vec<f64> = c.as_array()?.items.iter().map(|v| v.as_num().map(Num::as_f64)).collect::<Option<_>>()?;
    let [r, g, b, a] = parts.as_slice() else { return None };
    let byte = |v: f64| -> Option<Value> {
        let v = (v * 255.0).round();
        // Range-checked, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        (v.is_finite() && (0.0..=255.0).contains(&v)).then(|| int(v as i64))
    };
    let items = if r == g && g == b { vec![byte(*r)?, byte(*a)?] } else { vec![byte(*r)?, byte(*g)?, byte(*b)?, byte(*a)?] };
    Some(array(items))
}

fn palettes(v: &Value, color: fn(&Value) -> Option<Value>) -> Option<Value> {
    let out: Option<Vec<Value>> =
        v.as_array()?.items.iter().map(|p| Some(array(p.as_array()?.items.iter().map(color).collect::<Option<Vec<_>>>()?))).collect();
    out.map(array)
}

/// glyphsLib's `to_ufo_meta_table`: a list of `{tag, data}` as tag → list of data (`appl` and
/// `bild` → the data itself; the last one wins).
fn meta_to_ufo(v: &Value) -> Option<Value> {
    let mut out = Dict::new();
    for e in &v.as_array()?.items {
        let d = e.as_dict()?;
        let (tag, data) = (d.get("tag")?.as_str()?, d.get("data")?.as_str()?);
        if tag == "appl" || tag == "bild" {
            out.insert(tag, string(data));
        } else {
            match out.get_mut(tag) {
                Some(Value::Array(a)) => a.items.push(string(data)),
                _ => out.insert(tag, array(vec![string(data)])),
            }
        }
    }
    Some(sorted_keys(Value::Dict(out)))
}

/// The reverse, each entry `{data, tag}` as the reference's format sorts it.
fn meta_to_model(v: &Value) -> Option<Value> {
    let mut items = Vec::new();
    for (tag, data) in &v.as_dict()?.entries {
        let datas: Vec<&Value> = match data {
            Value::Array(a) => a.items.iter().collect(),
            other => vec![other],
        };
        for d in datas {
            let mut e = Dict::new();
            e.insert("data", string(d.as_str()?));
            e.insert("tag", string(tag.as_str()));
            items.push(Value::Dict(e));
        }
    }
    Some(array(items))
}

fn to_ufo(conv: Conv, v: &Value) -> Option<Value> {
    match conv {
        Conv::Palettes => palettes(v, color_to_ufo),
        Conv::Meta => meta_to_ufo(v),
        Conv::Negated => Some(Value::Bool(!truthy(v))),
    }
}

fn to_model(conv: Conv, v: &Value) -> Option<Value> {
    match conv {
        Conv::Palettes => palettes(v, color_to_model),
        Conv::Meta => meta_to_model(v),
        Conv::Negated => v.as_bool().map(|b| int(i64::from(!b))),
    }
}

/// A generic parameter's value as the lib holds it.
fn param_to_ufo(name: &str, v: &Value) -> Value {
    if BOOLEAN.contains(&name) { Value::Bool(truthy(v)) } else { sorted_keys(v.without_spellings()) }
}

/// The reverse: booleans of [`BOOLEAN`] become 0 or 1.
fn param_to_model(name: &str, v: &Value) -> Value {
    match v {
        Value::Bool(b) if BOOLEAN.contains(&name) => int(i64::from(*b)),
        other => other.without_spellings(),
    }
}

/// The parameter name a key after a class prefix stands for.
fn param_name(tail: &str) -> &str {
    RENAMED.iter().find(|(_, t)| *t == tail).map_or(tail, |(n, _)| *n)
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// What writing one master's UFO needs.
pub(crate) struct Ctx<'a> {
    pub font: &'a Font,
    pub master: &'a Master,
    /// The master's position among the document's masters that have a UFO of their own (not a
    /// layer of another source's UFO), as glyphsLib numbers them.
    pub index: usize,
    pub is_default: bool,
    /// The UFO is a source of a designspace (else a UFO of its own).
    pub designspace: bool,
    /// The UFO was not read from disk: written for a document from elsewhere.
    pub new: bool,
    /// This master's `lib.plist` as read (empty for a new UFO).
    pub kept: &'a Dict,
    /// The default master's `lib.plist` as read.
    pub default_kept: &'a Dict,
    /// This master's `fontinfo.plist` as read.
    pub info: &'a Dict,
}

/// The lib entries glyphsLib's keys give, and what they cannot hold.
#[derive(Default)]
pub(crate) struct Entries {
    pub map: BTreeMap<String, Value>,
    pub lost: Vec<String>,
}

impl Entries {
    fn put(&mut self, key: &str, v: Value, what: &str) {
        if self.map.contains_key(key) {
            self.lost.push(format!("{what} (`{key}` is already set)"));
        } else {
            self.map.insert(key.to_string(), v);
        }
    }
}

impl Ctx<'_> {
    /// Whether a new UFO gets the values glyphsLib writes whatever they are: one of a document
    /// that has a `.glyphs` format version (read from, or saved as, a `.glyphs` source).
    fn writes_defaults(&self) -> bool {
        self.new && self.font.source.glyphs_version.is_some()
    }

    /// A key of this master: its value, else the default where glyphsLib writes it (`always`) or
    /// the file had the key.
    fn key(&self, out: &mut Entries, key: &str, value: Option<Value>, default: Option<Value>, always: bool, what: &str) {
        let has = self.kept.get(key).is_some();
        if let Some(v) = value.or_else(|| default.filter(|_| has || always && self.writes_defaults())) {
            out.put(key, v, what);
        }
    }

    /// A key of the font, in every master's UFO. A master other than the default one whose file
    /// lacked a key the default master's had keeps lacking it.
    fn font_key(&self, out: &mut Entries, key: &str, value: Option<Value>, default: Option<Value>, always: bool, what: &str) {
        if !self.new && !self.is_default && self.kept.get(key).is_none() && self.default_kept.get(key).is_some() {
            return;
        }
        self.key(out, key, value, default, always, what);
    }

    fn params(&self, out: &mut Entries, params: &Params, font_level: bool) {
        let (prefix, attrs): (&str, &[&str]) = if font_level { (FONT_PARAM, &FONT_ATTRS) } else { (MASTER_PARAM, &MASTER_ATTRS) };
        let mut seen: HashSet<&str> = HashSet::new();
        let mut listed: Vec<Value> = Vec::new();
        for p in params.0.iter().filter(|p| carried(p, Some(self.info))) {
            let name = p.name.as_str();
            if font_level && self.designspace && IN_DOCUMENT.contains(&name) {
                continue;
            }
            let what = format!("parameter `{name}`");
            if font_level && name == LISTED {
                listed.push(sorted_keys(p.value.without_spellings()));
                continue;
            }
            if !seen.insert(name) {
                out.lost.push(format!("{what} (repeated)"));
                continue;
            }
            if let Some((_, key, conv)) = OWN_KEYS.iter().find(|(n, ..)| *n == name) {
                match to_ufo(*conv, &p.value) {
                    Some(v) if font_level => self.font_key(out, key, Some(v), None, false, &what),
                    // A master's value replaces the font's.
                    Some(v) => {
                        out.map.insert((*key).to_string(), v);
                    }
                    None => out.lost.push(format!("{what}: not a value `{key}` can hold")),
                }
                continue;
            }
            let tail = RENAMED.iter().find(|(n, _)| *n == name).map_or(name, |(_, t)| *t);
            if attrs.contains(&tail) {
                out.lost.push(format!("{what} (its key holds something else)"));
                continue;
            }
            let key = format!("{prefix}{tail}");
            let value = param_to_ufo(name, &p.value);
            if font_level {
                self.font_key(out, &key, Some(value), None, false, &what);
            } else {
                self.key(out, &key, Some(value), None, false, &what);
            }
        }
        if !listed.is_empty() {
            self.font_key(out, &format!("{prefix}{LISTED}"), Some(array(listed)), None, false, &format!("parameter `{LISTED}`"));
        }
    }
}

/// The lib entries of one master's UFO.
pub(crate) fn entries(c: &Ctx<'_>) -> Entries {
    let (font, m) = (c.font, c.master);
    let s = &font.info.settings;
    let mut out = Entries::default();
    // The font.
    c.font_key(&mut out, APP_VERSION, font.source.app_version.as_deref().map(string), None, false, "the app version");
    c.font_key(&mut out, KEYBOARD_INCREMENT, s.keyboard_increment.map(fontinfo::number), Some(int(1)), true, "the keyboard increment");
    c.font_key(&mut out, AUTO_ALIGNMENT, s.disables_automatic_alignment.map(Value::Bool), Some(Value::Bool(false)), true, "automatic alignment");
    c.font_key(&mut out, NICE_NAMES, s.disables_nice_names.map(|d| int(i64::from(!d))), Some(int(1)), true, "nice names");
    let tabs = (!font.display_strings.is_empty()).then(|| array(font.display_strings.iter().map(|t| string(t)).collect()));
    c.font_key(&mut out, DISPLAY_STRINGS, tabs, Some(array(Vec::new())), false, "the edit-tab texts");
    if !c.designspace {
        let data = (!font.info.user_data.is_empty()).then(|| sorted_keys(Value::Dict(font.info.user_data.0.clone()).without_spellings()));
        c.font_key(&mut out, FONT_USER_DATA, data, Some(Value::Dict(Dict::new())), true, "the font's user data");
    }
    c.params(&mut out, &font.info.params, true);
    // The master.
    let index = i64::try_from(c.index).unwrap_or(i64::MAX);
    c.key(&mut out, MASTER_ORDER, None, Some(int(index)), true, "the master order");
    // A single new UFO keeps an id the reader would not give it.
    let id = (c.new && !c.designspace && m.id != SINGLE_MASTER_ID).then(|| string(&m.id));
    c.key(&mut out, MASTER_ID, id, Some(string(&m.id)), false, "the master id");
    c.key(&mut out, ICON, m.icon.as_deref().map(string), Some(string("")), true, "the master's icon");
    let part = |p: &Option<String>| p.as_deref().map(string);
    c.key(&mut out, WEIGHT, part(&m.name_parts.weight), Some(string(REGULAR)), true, "the master's weight name");
    c.key(&mut out, WIDTH, part(&m.name_parts.width), Some(string(REGULAR)), true, "the master's width name");
    c.key(&mut out, CUSTOM, m.name_parts.custom.as_deref().filter(|s| !s.is_empty()).map(string), None, false, "the master's custom name");
    for (i, name) in AXIS_VALUES.iter().enumerate() {
        let v = axes::design_value(&m.location, i);
        c.key(&mut out, &format!("{MASTER_PARAM}{name}"), None, Some(fontinfo::number(v)), true, "a design coordinate");
        // glyphsLib writes the weight always, the others when not zero.
        c.key(&mut out, &format!("{PLAIN}{name}"), None, (i == 0 || v != 0.0).then(|| fontinfo::number(v)), true, "a design coordinate");
    }
    c.params(&mut out, &m.params, false);
    out
}

// ── reading ────────────────────────────────────────────────────────────────────────────────────

/// One typed key: the master whose lib it is in, and the key.
type At = (String, String);

/// Types glyphsLib's keys in the masters' user data (each UFO's lib, but the keys the UFO reader
/// types itself) into model fields where writing the fields gives the same value; every key the
/// writer gives back unchanged is no longer user data. `default_id` names the master whose UFO
/// gives the font's data. Each pass leaves out the keys the last one could not type; when
/// [`MAX_PASSES`] are not enough, nothing is typed and every key stays user data.
pub(crate) fn read(font: &mut Font, designspace: bool, default_id: &str) {
    let mut excluded: HashSet<At> = HashSet::new();
    for _ in 0..MAX_PASSES {
        let mut f = font.clone();
        let typed = apply(&mut f, designspace, default_id, &excluded);
        let written = written_by_master(&f, designspace, default_id);
        let said = |at: &At| -> Option<&Value> { f.masters.iter().find(|m| m.id == at.0).and_then(|m| m.user_data.get(&at.1)) };
        let bad: Vec<At> = typed.into_iter().filter(|at| written.get(&at.0).and_then(|w| w.get(&at.1)) != said(at) || said(at).is_none()).collect();
        if bad.is_empty() {
            let masters = Arc::make_mut(&mut f.masters);
            for m in masters.iter_mut() {
                let Some(w) = written.get(&m.id) else { continue };
                m.user_data.0.entries.retain(|(k, v)| w.get(k.as_str()) != Some(v));
            }
            *font = f;
            return;
        }
        excluded.extend(bad);
    }
}

/// What the writer gives each master read from a UFO, by master id.
fn written_by_master(f: &Font, designspace: bool, default_id: &str) -> BTreeMap<String, BTreeMap<String, Value>> {
    let lib_of = |m: &Master| m.extra.get(Family::Ufo, "lib.plist").and_then(Value::as_dict).cloned().unwrap_or_default();
    let info_of = |m: &Master| m.extra.get(Family::Ufo, "fontinfo.plist").and_then(Value::as_dict).cloned().unwrap_or_default();
    let default_kept = f.masters.iter().find(|m| m.id == default_id).map(lib_of).unwrap_or_default();
    let mut out = BTreeMap::new();
    for m in f.masters.iter() {
        if m.extra.get(Family::Ufo, "lib.plist").is_none() {
            continue;
        }
        let index = master_index(f, m);
        let (kept, info) = (lib_of(m), info_of(m));
        let ctx = Ctx {
            font: f,
            master: m,
            index,
            is_default: m.id == default_id,
            designspace,
            new: false,
            kept: &kept,
            default_kept: &default_kept,
            info: &info,
        };
        out.insert(m.id.clone(), entries(&ctx).map);
    }
    out
}

/// The model fields the keys in the masters' user data give, but the `excluded` keys; returns
/// the keys that gave a field.
fn apply(f: &mut Font, designspace: bool, default_id: &str, excluded: &HashSet<At>) -> Vec<At> {
    let skip = |id: &str, k: &str| excluded.contains(&(id.to_string(), k.to_string()));
    let mut typed: Vec<At> = Vec::new();
    // The font's, from the default master's UFO.
    let Some(default) = f.masters.iter().find(|m| m.id == default_id) else { return typed };
    let data = default.user_data.0.clone();
    let mut app_version = None;
    let mut tabs = None;
    {
        let info = Arc::make_mut(&mut f.info);
        let s = &mut info.settings;
        for (k, v) in &data.entries {
            let k = k.as_str();
            if skip(default_id, k) {
                continue;
            }
            let done = match k {
                APP_VERSION => v.as_str().map(|t| app_version = Some(t.to_string())),
                KEYBOARD_INCREMENT => v.as_num().map(Num::as_f64).filter(|n| n.is_finite()).map(|n| s.keyboard_increment = (n != 1.0).then_some(n)),
                AUTO_ALIGNMENT => v.as_bool().map(|b| s.disables_automatic_alignment = b.then_some(true)),
                // `useNiceNames` is the opposite: 0 means nice names are off.
                NICE_NAMES => v.as_num().filter(|n| matches!(n, Num::Int(0 | 1))).map(|n| s.disables_nice_names = (n == Num::Int(0)).then_some(true)),
                DISPLAY_STRINGS => v
                    .as_array()
                    .and_then(|a| a.items.iter().map(|t| t.as_str().map(str::to_string)).collect::<Option<Vec<_>>>())
                    .map(|t| tabs = Some(t)),
                FONT_USER_DATA if !designspace => v.as_dict().map(|d| info.user_data = UserData(d.clone())),
                _ => {
                    if let Some((name, _, conv)) = OWN_KEYS.iter().find(|(_, key, _)| *key == k) {
                        to_model(*conv, v).map(|v| info.params.0.push(Param::new(*name, v)))
                    } else if let Some(tail) = k.strip_prefix(FONT_PARAM).filter(|t| !FONT_ATTRS.contains(t)) {
                        if tail == LISTED {
                            v.as_array().map(|a| info.params.0.extend(a.items.iter().map(|x| Param::new(LISTED, x.without_spellings()))))
                        } else {
                            let name = param_name(tail);
                            info.params.0.push(Param::new(name, param_to_model(name, v)));
                            Some(())
                        }
                    } else {
                        None
                    }
                }
            };
            if done.is_some() {
                typed.push((default_id.to_string(), k.to_string()));
            }
        }
    }
    if let Some(v) = app_version {
        f.source.app_version = Some(v);
    }
    if let Some(t) = tabs {
        f.display_strings = Arc::new(t);
    }
    // Each master's own.
    let font_own: Vec<(&str, Option<Value>)> =
        OWN_KEYS.iter().map(|(name, key, conv)| (*key, f.info.params.get(name).and_then(|v| to_ufo(*conv, v)))).collect();
    let masters = Arc::make_mut(&mut f.masters);
    for m in masters.iter_mut() {
        if m.extra.get(Family::Ufo, "lib.plist").is_none() {
            continue;
        }
        let data = m.user_data.0.clone();
        for (k, v) in &data.entries {
            let k = k.as_str();
            if skip(&m.id, k) {
                continue;
            }
            let part = |v: &Value| v.as_str().map(|t| (t != REGULAR).then(|| t.to_string()));
            let done = match k {
                ICON => v.as_str().map(|t| m.icon = (!t.is_empty()).then(|| t.to_string())),
                WEIGHT => part(v).map(|p| m.name_parts.weight = p),
                WIDTH => part(v).map(|p| m.name_parts.width = p),
                CUSTOM => v.as_str().filter(|t| !t.is_empty()).map(|t| m.name_parts.custom = Some(t.to_string())),
                _ => {
                    if let Some((name, key, conv)) = OWN_KEYS.iter().find(|(_, key, _)| *key == k) {
                        // The default master's is the font's; another master's is its own where
                        // it differs from the font's.
                        let font_says = font_own.iter().find(|(x, _)| x == key).and_then(|(_, v)| v.as_ref());
                        if m.id == default_id || font_says == Some(v) {
                            None
                        } else {
                            to_model(*conv, v).map(|v| m.params.0.push(Param::new(*name, v)))
                        }
                    } else if let Some(tail) = k.strip_prefix(MASTER_PARAM).filter(|t| !MASTER_ATTRS.contains(t)) {
                        let name = param_name(tail);
                        m.params.0.push(Param::new(name, param_to_model(name, v)));
                        Some(())
                    } else {
                        None
                    }
                }
            };
            if done.is_some() {
                typed.push((m.id.clone(), k.to_string()));
            }
        }
    }
    typed
}

/// The position [`Ctx::index`] wants for `m`.
pub(crate) fn master_index(font: &Font, m: &Master) -> usize {
    font.masters.iter().take_while(|x| !std::ptr::eq(*x, m)).filter(|x| x.extra.get(Family::Designspace, "layer").is_none()).count()
}

/// The master id a single UFO's lib states, when it is one the model accepts.
pub(crate) fn single_master_id(lib: &Dict) -> Option<String> {
    let id = lib.get(MASTER_ID)?.as_str()?;
    let ok = !id.trim().is_empty() && id.len() <= 255 && !id.chars().any(char::is_control);
    ok.then(|| id.to_string())
}

/// What the libs of a document's UFOs cannot hold of the font's and the masters' data, one line
/// each (the settings and name parts no key holds, and the values the keys cannot take).
pub(crate) fn losses(font: &Font, designspace: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let s = &font.info.settings;
    let mut gone: Vec<&str> = Vec::new();
    let unkept = [
        ("grid length", s.grid_length.is_some()),
        ("grid subdivision", s.grid_sub_division.is_some()),
        ("the larger keyboard increments", s.keyboard_increment_big.is_some() || s.keyboard_increment_huge.is_some()),
        ("keeping alternates together", s.keep_alternates_together.is_some()),
        ("overlap removal in the preview", s.preview_remove_overlap.is_some()),
        ("snapping to objects", s.snap_to_objects.is_some()),
        ("the font type", s.font_type.is_some()),
        ("other settings", !s.extra.is_empty()),
        // A stated default reads back as no value.
        (
            "settings stated as their default",
            s.disables_automatic_alignment == Some(false) || s.disables_nice_names == Some(false) || s.keyboard_increment == Some(1.0),
        ),
    ];
    gone.extend(unkept.iter().filter(|(_, on)| *on).map(|(what, _)| *what));
    if !gone.is_empty() {
        out.push(format!("the font's settings: {}", gone.join(", ")));
    }
    // The master the designspace writer takes as the default.
    let kept_default =
        font.extra.get(Family::Designspace, "defaultSource").and_then(Value::as_str).filter(|id| font.masters.iter().any(|m| m.id == *id));
    let default_id = kept_default.map(str::to_string).or_else(|| axes::origin(font).map(|m| m.id.clone())).unwrap_or_default();
    for m in font.masters.iter() {
        let index = master_index(font, m);
        let name = if m.name.is_empty() { &m.id } else { &m.name };
        if m.visible {
            out.push(format!("master `{name}`: visibility"));
        }
        let defaults = m.name_parts.weight.as_deref() == Some(REGULAR)
            || m.name_parts.width.as_deref() == Some(REGULAR)
            || m.name_parts.custom.as_deref() == Some("")
            || m.icon.as_deref() == Some("");
        if defaults {
            out.push(format!("master `{name}`: an icon or name parts stated as their default"));
        }
        // A master stored as a layer of another source's UFO has no lib of its own.
        if m.extra.get(Family::Designspace, "layer").is_some() {
            let has = m.icon.is_some() || !m.name_parts.is_empty() || m.params.0.iter().any(|p| carried(p, None)) || !m.user_data.is_empty();
            if has {
                out.push(format!("master `{name}`: icon, name parts, parameters or user data, as a layer of another UFO"));
            }
            continue;
        }
        let empty = Dict::new();
        let info = m.extra.get(Family::Ufo, "fontinfo.plist").and_then(Value::as_dict).unwrap_or(&empty);
        let ctx = Ctx { font, master: m, index, is_default: m.id == default_id, designspace, new: true, kept: &empty, default_kept: &empty, info };
        for line in entries(&ctx).lost {
            let line = format!("master `{name}`: {line}");
            if !out.contains(&line) {
                out.push(line);
            }
        }
        // The default master's own value of a parameter with a key of its own comes back as the
        // font's.
        if m.id == default_id {
            for (p, ..) in OWN_KEYS.iter().filter(|(p, ..)| m.params.get(p).is_some()) {
                out.push(format!("master `{name}`: parameter `{p}`, which comes back as the font's"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{designspace, ufo};

    fn text(files: &crate::Files, path: &str) -> String {
        String::from_utf8(files[path].clone()).unwrap()
    }

    /// A one-master font as read from a `.glyphs` source, with data for every font-level key.
    fn glyphs_font() -> Font {
        let mut font = Font::new("Lib Sans");
        font.source.glyphs_version = Some(3);
        font.source.app_version = Some("3335".into());
        font.display_strings = Arc::new(vec!["/A/B".into()]);
        let info = Arc::make_mut(&mut font.info);
        info.settings.keyboard_increment = Some(2.0);
        info.settings.disables_nice_names = Some(true);
        info.settings.grid_length = Some(10.0);
        info.user_data.0.insert("Some Key", string("Some Value"));
        let mut d = Dict::new();
        d.insert("Path", string("files/Other.glyphs"));
        info.params.0.push(Param::new("Import Font", Value::Dict(d)));
        info.params.0.push(Param::new("typoAscender", int(800)));
        let palette = array(vec![array(vec![int(255), int(0), int(0), int(255)]), array(vec![int(128), int(255)])]);
        info.params.0.push(Param::new("Color Palettes", array(vec![palette])));
        info.params.0.push(Param::new(LISTED, string("Weight; 100")));
        info.params.0.push(Param::new(LISTED, string("Weight; 900")));
        info.params.0.push(Param::new("Disable Last Change", int(1)));
        info.params.0.push(Param { disabled: true, ..Param::new("Switched Off", int(1)) });
        let m = &mut Arc::make_mut(&mut font.masters)[0];
        m.icon = Some("Bold".into());
        m.name_parts.weight = Some("Bold".into());
        m.name_parts.custom = Some("Condensed".into());
        m.params.0.push(Param::new("Default Layer Width", string("600")));
        m.user_data.0.insert("org.example.own", int(1));
        font
    }

    #[test]
    fn font_and_master_data_go_to_glyphslib_keys_and_come_back() {
        let font = glyphs_font();
        let files = ufo::write(&font).unwrap();
        let lib = text(&files, "lib.plist");
        for expected in [
            "<key>com.schriftgestaltung.appVersion</key>\n    <string>3335</string>",
            "<key>com.schriftgestaltung.keyboardIncrement</key>\n    <integer>2</integer>",
            "<key>com.schriftgestaltung.customParameter.GSFont.disablesAutomaticAlignment</key>\n    <false/>",
            "<key>com.schriftgestaltung.customParameter.GSFont.useNiceNames</key>\n    <integer>0</integer>",
            "<key>com.schriftgestaltung.customParameter.GSFont.DisplayStrings</key>\n    <array>\n      <string>/A/B</string>",
            "<key>com.schriftgestaltung.Glyphs.fontUserData</key>\n    <dict>\n      <key>Some Key</key>",
            "<key>com.schriftgestaltung.customParameter.GSFont.Import Font</key>\n    <dict>\n      <key>Path</key>",
            "<key>com.github.googlei18n.ufo2ft.colorPalettes</key>\n    <array>\n      <array>\n        <array>\n          <real>1.0</real>",
            "<key>com.schriftgestaltung.customParameter.GSFont.Virtual Master</key>\n    <array>\n      <string>Weight; 100</string>\n      <string>Weight; 900</string>",
            "<key>com.schriftgestaltung.customParameter.GSFont.disablesLastChange</key>\n    <true/>",
            "<key>com.schriftgestaltung.customParameter.GSFontMaster.iconName</key>\n    <string>Bold</string>",
            "<key>com.schriftgestaltung.customParameter.GSFontMaster.Default Layer Width</key>\n    <string>600</string>",
            "<key>com.schriftgestaltung.customParameter.GSFontMaster.weightValue</key>\n    <integer>100</integer>",
            "<key>com.schriftgestaltung.weight</key>\n    <string>Bold</string>",
            "<key>com.schriftgestaltung.width</key>\n    <string>Regular</string>",
            "<key>com.schriftgestaltung.customName</key>\n    <string>Condensed</string>",
            "<key>com.schriftgestaltung.fontMasterOrder</key>\n    <integer>0</integer>",
            "<key>org.example.own</key>",
        ] {
            assert!(lib.contains(expected), "{expected} in {lib}");
        }
        assert!(!lib.contains("Switched Off") && !lib.contains("typoAscender") && !lib.contains("fontMasterID"), "{lib}");
        assert!(text(&files, "fontinfo.plist").contains("openTypeOS2TypoAscender"));
        let back = ufo::read(&files).unwrap();
        assert_eq!(back.source.app_version.as_deref(), Some("3335"));
        assert_eq!(*back.display_strings, ["/A/B"]);
        let s = &back.info.settings;
        assert_eq!((s.keyboard_increment, s.disables_nice_names, s.disables_automatic_alignment), (Some(2.0), Some(true), None));
        assert_eq!(s.grid_length, None, "no key holds the grid");
        assert_eq!(back.info.user_data, font.info.user_data);
        for name in ["Import Font", "Color Palettes", "Disable Last Change"] {
            assert_eq!(back.info.params.get(name), font.info.params.get(name), "{name}");
        }
        assert_eq!(back.info.params.all(LISTED).count(), 2);
        let (m, was) = (&back.masters[0], &font.masters[0]);
        assert_eq!((&m.icon, &m.name_parts, &m.params), (&was.icon, &was.name_parts, &was.params));
        assert_eq!(m.user_data, was.user_data, "the typed keys are not user data");
        assert_eq!(ufo::write(&back).unwrap(), files, "it comes back unchanged");
    }

    #[test]
    fn an_edit_writes_the_new_value_and_leaves_the_rest() {
        let files = ufo::write(&glyphs_font()).unwrap();
        let mut font = ufo::read(&files).unwrap();
        Arc::make_mut(&mut font.info).settings.keyboard_increment = Some(5.0);
        Arc::make_mut(&mut font.masters)[0].icon = None;
        Arc::make_mut(&mut font.info).params.remove("Import Font");
        let again = ufo::write(&font).unwrap();
        let lib = text(&again, "lib.plist");
        assert!(lib.contains("<key>com.schriftgestaltung.keyboardIncrement</key>\n    <integer>5</integer>"), "{lib}");
        assert!(
            lib.contains("<key>com.schriftgestaltung.customParameter.GSFontMaster.iconName</key>\n    <string></string>"),
            "the file had the key: {lib}"
        );
        assert!(!lib.contains("Import Font"), "{lib}");
        let before = text(&files, "lib.plist");
        assert_eq!(before.lines().count(), lib.lines().count() + 5, "only the changed keys");
    }

    #[test]
    fn a_new_ufo_of_another_document_gets_no_defaults() {
        let files = ufo::write(&Font::new("Plain")).unwrap();
        let lib = text(&files, "lib.plist");
        assert!(!lib.contains("schriftgestaltung"), "{lib}");
        // A master id the reader would not give it is kept.
        let mut font = Font::new("Plain");
        let id = "C2ECF50A-02EF-4989-A14C-AF8E838D1105";
        Arc::make_mut(&mut font.masters)[0].id = id.into();
        for n in font.glyphs.iter().map(|g| g.name.clone()).collect::<Vec<_>>() {
            let mut g = font.glyph_mut(&n).unwrap();
            for l in g.layers.iter_mut() {
                l.id = id.into();
                l.master = id.into();
            }
        }
        let files = ufo::write(&font).unwrap();
        assert!(text(&files, "lib.plist").contains("<key>com.schriftgestaltung.fontMasterID</key>"));
        let back = ufo::read(&files).unwrap();
        assert_eq!(back.masters[0].id, id);
        assert!(back.masters[0].user_data.is_empty());
    }

    #[test]
    fn keys_that_say_something_else_stay_user_data() {
        let mut font = Font::new("Odd");
        let m = &mut Arc::make_mut(&mut font.masters)[0];
        m.user_data.0.insert(AUTO_ALIGNMENT, string("yes"));
        m.user_data.0.insert(NICE_NAMES, int(5));
        m.user_data.0.insert(MASTER_ORDER, int(7));
        m.user_data.0.insert("com.schriftgestaltung.customParameter.GSFont.typoAscender", int(1));
        let files = ufo::write(&font).unwrap();
        let back = ufo::read(&files).unwrap();
        assert!(back.info.settings.is_empty());
        assert!(back.info.params.get("typoAscender").is_none(), "a font-info key is not a lib parameter");
        assert_eq!(back.masters[0].user_data.0.len(), 4, "{:?}", back.masters[0].user_data);
        assert_eq!(ufo::write(&back).unwrap(), files);
    }

    #[test]
    fn a_designspace_keeps_font_data_once_and_master_data_per_source() {
        let mut font = glyphs_font();
        Arc::make_mut(&mut font.axes).push(fontcraft_model::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
        let mut bold = font.masters[0].clone();
        bold.id = "m02".into();
        bold.name = "Bold".into();
        bold.location = vec![700.0];
        bold.icon = None;
        bold.params.0.push(Param::new("Color Palettes", array(vec![array(vec![array(vec![int(0), int(255)])])])));
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0];
        Arc::make_mut(&mut font.masters).push(bold);
        let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
        for n in names {
            let mut g = font.glyph_mut(&n).unwrap();
            let mut l = g.layers[0].clone();
            l.id = "m02".into();
            l.master = "m02".into();
            g.layers.push(l);
        }
        Arc::make_mut(&mut font.info).params.0.push(Param::new(AXIS_MAPPINGS, Value::Dict(Dict::new())));
        let out = designspace::write(&font).unwrap();
        assert!(out.document.contains("<lib>\n    <dict>\n      <key>Some Key</key>\n      <string>Some Value</string>"), "{}", out.document);
        let libs: Vec<String> = out.ufos.values().map(|f| text(f, "lib.plist")).collect();
        for lib in &libs {
            assert!(lib.contains("GSFont.Import Font") && !lib.contains("fontUserData") && !lib.contains("Axis Mappings"), "{lib}");
        }
        let back = designspace::read(&out.document, &out.ufos).unwrap();
        assert_eq!(back.info.user_data, font.info.user_data);
        assert_eq!(back.info.params.all(LISTED).count(), 2, "the font's parameters once");
        assert_eq!(back.masters[0].icon.as_deref(), Some("Bold"));
        assert_eq!(back.masters[1].icon, None);
        assert_eq!(back.masters[1].params.get("Color Palettes"), font.masters[1].params.get("Color Palettes"), "a master's own value");
        assert!(back.masters.iter().all(|m| m.user_data.get(FONT_PARAM.trim_end_matches('.')).is_none() && m.user_data.0.len() == 1));
        let again = designspace::write(&back).unwrap();
        assert_eq!((again.document, again.ufos), (out.document, out.ufos), "it comes back unchanged");
    }

    #[test]
    fn colours_go_to_ufo2ft_components_and_back() {
        let c = array(vec![int(255), int(128), int(0), int(255)]);
        let ufo = color_to_ufo(&c).unwrap();
        assert_eq!(color_to_model(&ufo), Some(c));
        let grey = string("64,255");
        let back = color_to_model(&color_to_ufo(&grey).unwrap()).unwrap();
        assert_eq!(back, array(vec![int(64), int(255)]));
        assert!(color_to_ufo(&string("1,2,3,4,5")).is_none(), "CMYK has no ufo2ft form");
    }

    #[test]
    fn the_meta_table_is_grouped_by_tag() {
        let entry = |tag: &str, data: &str| {
            let mut d = Dict::new();
            d.insert("data", string(data));
            d.insert("tag", string(tag));
            Value::Dict(d)
        };
        let v = array(vec![entry("dlng", "de-Latn"), entry("slng", "sr-Cyrl"), entry("dlng", "en")]);
        let ufo = meta_to_ufo(&v).unwrap();
        let d = ufo.as_dict().unwrap();
        assert_eq!(d.get("dlng").and_then(Value::as_array).map(|a| a.items.len()), Some(2));
        let back = meta_to_model(&ufo).unwrap();
        assert_eq!(back.as_array().map(|a| a.items.len()), Some(3));
    }
}

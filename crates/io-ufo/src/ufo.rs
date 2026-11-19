//! A UFO directory: one master of a document.
//!
//! The reader fills the model's fields from the UFO's files and keeps, in the master's
//! [`Extra`] under [`Family::Ufo`], each property list as it was read (with its spellings), keyed
//! by file name: `metainfo.plist`, `fontinfo.plist`, `lib.plist`, `groups.plist`,
//! `kerning.plist`, `layercontents.plist`, and `layerinfo` (a dictionary of each layer's
//! `layerinfo.plist` by layer name). The writer starts from these and changes what the model
//! changed: a kept value is written as it was read as long as it still means what the model holds
//! (so `<real>500.0</real>` stays a real), a changed one is written canonically, a key the model
//! no longer has is removed, and a new key is inserted in sorted order. A document that did not
//! come from a UFO gets the files fontTools would write for it.
//!
//! How the files are laid out (declaration, document type, indentation; the same for GLIF files)
//! is taken from `metainfo.plist` and the first glyph, kept as `plistStyle` and `glifStyle`, and
//! written back the same way.
//!
//! | Model | UFO |
//! |---|---|
//! | family name, units per em, version, note | `fontinfo.plist` `familyName`, `unitsPerEm`, `versionMajor`, `versionMinor`, `note` |
//! | the font's properties, parameters and date; a master's where its file differs from the default master's | the other `fontinfo.plist` keys, through [`crate::fontinfo`] |
//! | a `styleMapStyleName` parameter other than `regular`, `italic`, `bold` and `bold italic`, which ufoLib and fontc refuse in `fontinfo.plist` | `lib.plist` `com.fontcraft.styleMapStyleName` |
//! | the master's user data | `lib.plist`, except the keys below |
//! | the font's settings, app version, edit-tab texts and parameters with no `fontinfo.plist` key; a master's icon, name parts and such parameters; in a single UFO the font's user data and the master id | glyphsLib's `lib.plist` keys ([`crate::libkeys`]): `com.schriftgestaltung.*`, `com.github.googlei18n.ufo2ft.colorPalettes`… |
//! | master name; metrics `ascender`, `cap height`, `x-height`, `descender`, `italic angle` | `styleName`; `ascender`, `capHeight`, `xHeight`, `descender`, `italicAngle` (counter-clockwise, so the sign is flipped) |
//! | metric overshoots and the master's other alignment zones; a `baseline` metric when a zone at 0 needs one | `postscriptBlueValues`, `postscriptOtherBlues` ([`crate::design`]) |
//! | stem definitions (unnamed, horizontal first) and the master's stem values | `postscriptStemSnapH`, `postscriptStemSnapV` |
//! | the master's guides | `guidelines` |
//! | glyph order | `lib.plist` `public.glyphOrder` first, then the rest in `contents.plist` order |
//! | glyph export | `lib.plist` `public.skipExportGlyphs` |
//! | glyph production names, OpenType categories | `lib.plist` `public.postscriptNames`, `public.openTypeCategories` |
//! | glyph code points, note, colour label | the default layer's GLIF: `<unicode>`, `<note>`, the lib's `public.markColor` |
//! | kerning groups | `groups.plist` `public.kern1.*` (the glyph's right side) and `public.kern2.*` (left side); in a UFO 2 the groups ufoLib renames to those names ([`crate::kerning`]), written back under their own |
//! | the font's groups | the other `groups.plist` entries that are lists of names; other values stay as written |
//! | kerning | `kerning.plist`, as the left-to-right table, sides spelled as the UFO spells them (a UFO 2's group sides under their UFO 3 names) |
//! | features | `features.fea`, split into prefixes, classes and features by glyphsLib's markers when that gives the same text, else one unnamed prefix ([`crate::features`]) |
//! | layers | the default layer's GLIF is the master layer; `public.background` holds the master layers' backgrounds, `<name>.background` those of the layers named `<name>`; other UFO layers' GLIFs are further layers named after their UFO layer. Writing names the UFO layers as glyphsLib 6.15.0 does (`builder/layers.py`, `builders.py`; Apache-2.0, see `NOTICE`) |
//! | vertical metrics | the GLIF height and `public.verticalOrigin`, by glyphsLib's rules (`builder/glyph.py`) |
//! | glyph, layer and shape data with no GLIF element (categories, metrics keys, hints, annotations, image crops, component placement, node and anchor user data…); layer ids, order and libs; glyph user data | glyphsLib's GLIF lib keys, `layerinfo.plist` lib keys and `lib.plist` keys ([`crate::glyphkeys`], [`crate::shapekeys`]) |
//! | `data/`, `images/`, other files | [`Font::files`] |
//!
//! A UFO 1 is read as its UFO 2 form (the `ufo1` module converts its font information and lib)
//! and becomes a UFO 3 source: saving writes UFO 3.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{
    Extra, Family, FileStore, Font, FontInfo, Glyph, GlyphTable, Group, KernFirst, KernTable, Kerning, Layer, Master, MasterKerning, MetricDef,
    MetricValue, Param, Source, SourceFormat, StemDef, UserData, metric,
};
use fontcraft_plist::xml::{self, Node as XNode};
use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value, xmlplist};

use crate::design::{self, Design};
use crate::fontinfo::{self, Info};
use crate::glif::{self, MarkColor, Style};
use crate::glyphkeys;
use crate::kerning;
use crate::libkeys;
use crate::names::{glyph_file_name, layer_dir_name};
use crate::shapekeys;
use crate::ufo1;
use crate::{Files, Result, UfoError, bad};

const METAINFO: &str = "metainfo.plist";
const FONTINFO: &str = "fontinfo.plist";
const LIB: &str = "lib.plist";
const GROUPS: &str = "groups.plist";
const KERNING: &str = "kerning.plist";
const LAYERCONTENTS: &str = "layercontents.plist";
const FEATURES: &str = "features.fea";
/// The key of a master's own feature code (see [`write_master`]).
pub(crate) const FEATURES_KEY: &str = FEATURES;
const LAYERINFO: &str = "layerinfo.plist";
const CONTENTS: &str = "contents.plist";
const DEFAULT_DIR: &str = "glyphs";
const KERN1: &str = "public.kern1.";
const KERN2: &str = "public.kern2.";
/// `lib.plist` keys that are model fields of their own rather than user data.
const TYPED_LIB: [&str; 5] = ["public.glyphOrder", "public.skipExportGlyphs", POSTSCRIPT_NAMES, OPENTYPE_CATEGORIES, STYLE_MAP_STYLE_LIB];
const STYLE_MAP_STYLE: &str = "styleMapStyleName";
/// The four style-map styles a UFO's `fontinfo.plist` may state; ufoLib and fontc refuse a file
/// with any other.
const STYLE_MAP_STYLES: [&str; 4] = ["regular", "italic", "bold", "bold italic"];
/// Where the lib keeps a stated style-map style outside [`STYLE_MAP_STYLES`] (fontTools'
/// instantiator writes such a style, `thin` say, into an instance UFO's `fontinfo.plist`).
pub const STYLE_MAP_STYLE_LIB: &str = "com.fontcraft.styleMapStyleName";
/// The font lib's production names, by glyph name.
const POSTSCRIPT_NAMES: &str = "public.postscriptNames";
/// The font lib's `GDEF` classes, by glyph name.
const OPENTYPE_CATEGORIES: &str = "public.openTypeCategories";
/// The creator written into a UFO FontCraft makes.
pub const CREATOR: &str = "org.fontcraft";

/// The `fontinfo.plist` keys that are metrics, with the model's kind for each.
pub(crate) const METRICS: [(&str, &str); 5] = [
    ("ascender", metric::ASCENDER),
    ("capHeight", metric::CAP_HEIGHT),
    ("xHeight", metric::X_HEIGHT),
    ("descender", metric::DESCENDER),
    ("italicAngle", metric::ITALIC_ANGLE),
];

fn ufo_get<'a>(x: &'a Extra, key: &str) -> Option<&'a Value> {
    x.get(Family::Ufo, key)
}

fn text<'a>(files: &'a Files, path: &str) -> Result<Option<&'a str>> {
    match files.get(path) {
        None => Ok(None),
        Some(bytes) => std::str::from_utf8(bytes).map(Some).map_err(|_| bad(path, "the file is not UTF-8")),
    }
}

fn read_plist(files: &Files, path: &str) -> Result<Option<xmlplist::Document>> {
    let Some(t) = text(files, path)? else { return Ok(None) };
    xmlplist::parse(t).map(Some).map_err(|e| UfoError::Syntax { file: path.to_string(), error: e })
}

/// Reads a UFO's property lists, noting each file whose layout differs from the UFO's.
struct Plists<'a> {
    files: &'a Files,
    style: PlistStyle,
    /// Path → [`PlistStyle`] for the files laid out differently.
    styles: Dict,
}

impl Plists<'_> {
    fn value(&mut self, path: &str) -> Result<Option<Value>> {
        let Some(doc) = read_plist(self.files, path)? else { return Ok(None) };
        let style = PlistStyle::of(&doc);
        if style != self.style {
            self.styles.insert(path, style.to_value());
        }
        Ok(Some(doc.root))
    }

    fn dict(&mut self, path: &str) -> Result<Option<Dict>> {
        match self.value(path)? {
            None => Ok(None),
            Some(Value::Dict(d)) => Ok(Some(d)),
            Some(_) => Err(bad(path, "the file does not hold a dictionary")),
        }
    }
}

/// How a UFO's property lists are laid out, as written around the value.
#[derive(Clone, Debug, PartialEq)]
pub struct PlistStyle {
    /// Everything before `<plist`: declaration, document type and the line breaks between.
    pub prolog: String,
    /// The `<plist>` start tag.
    pub tag: String,
    pub indent: String,
    /// The value's indentation level (see [`xmlplist::Document::level`]).
    pub level: usize,
    /// The whitespace between `<plist>` and the value, and between the value and `</plist>`.
    pub before: String,
    pub after: String,
    /// Everything after `</plist>`.
    pub epilog: String,
}

impl PlistStyle {
    pub fn fonttools() -> Self {
        Self::of(&xmlplist::Document::new(Value::Dict(Dict::new())))
    }

    fn of(doc: &xmlplist::Document) -> Self {
        let mut prolog = String::new();
        if doc.bom {
            prolog.push('\u{feff}');
        }
        let mut epilog = String::new();
        for n in &doc.prolog {
            write_xnode(&mut prolog, n);
        }
        for n in &doc.epilog {
            write_xnode(&mut epilog, n);
        }
        let mut shell = doc.plist.clone();
        shell.set_self_closing(false);
        Self {
            prolog,
            tag: shell.start_tag_text(),
            indent: doc.indent.clone(),
            level: doc.level,
            before: doc.before.clone(),
            after: doc.after.clone(),
            epilog,
        }
    }

    pub fn to_value(&self) -> Value {
        let mut d = Dict::new();
        for (k, v) in [
            ("after", &self.after),
            ("before", &self.before),
            ("epilog", &self.epilog),
            ("indent", &self.indent),
            ("prolog", &self.prolog),
            ("tag", &self.tag),
        ] {
            d.insert(k, Value::String(v.as_str().into()));
        }
        d.insert("level", Value::Number(Number::int(i64::try_from(self.level).unwrap_or(1))));
        Value::Dict(d)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let d = v.as_dict()?;
        let s = |k: &str| d.get(k).and_then(Value::as_str).map(str::to_string);
        let level = d.get("level").and_then(Value::as_num).and_then(int_of).and_then(|l| usize::try_from(l).ok())?;
        Some(Self {
            prolog: s("prolog")?,
            tag: s("tag")?,
            indent: s("indent")?,
            level,
            before: s("before")?,
            after: s("after")?,
            epilog: s("epilog")?,
        })
    }

    pub fn write(&self, value: &Value) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(&self.prolog);
        out.push_str(&self.tag);
        out.push_str(&self.before);
        xmlplist::write_value(&mut out, value, &self.indent, self.level);
        out.push_str(&self.after);
        out.push_str("</plist>");
        out.push_str(&self.epilog);
        out.into_bytes()
    }
}

fn write_xnode(out: &mut String, n: &XNode) {
    match n {
        XNode::Element(e) => e.write(out),
        XNode::Text(t) => match t.raw() {
            Some(r) => out.push_str(r),
            None => xml::escape_text(out, t.as_str()),
        },
        XNode::Comment(c) => {
            out.push_str("<!--");
            out.push_str(c);
            out.push_str("-->");
        }
        XNode::CData(c) => {
            out.push_str("<![CDATA[");
            out.push_str(c);
            out.push_str("]]>");
        }
        XNode::Pi(p) => {
            out.push_str("<?");
            out.push_str(p);
            out.push_str("?>");
        }
        XNode::Doctype(d) => {
            out.push_str("<!DOCTYPE");
            out.push_str(d);
            out.push('>');
        }
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_num().map(Num::as_f64)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.items.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// The colour a master layer's lib gives its glyph, when its `public.markColor` reads as one.
pub(crate) fn glif_mark_color(layer: &Layer) -> Option<fontcraft_model::ColorLabel> {
    ufo_get(&layer.extra, "lib").and_then(Value::as_dict).and_then(|d| d.get(glif::MARK_COLOR)).and_then(Value::as_str).and_then(glif::mark_color)
}

/// What one UFO holds, before it joins a document.
pub(crate) struct ReadUfo {
    pub master: Master,
    /// The `fontinfo.plist` values that are font-wide in the model.
    pub family_name: Option<String>,
    pub units_per_em: Option<f64>,
    pub version_major: Option<i64>,
    pub version_minor: Option<i64>,
    pub note: Option<String>,
    /// The other font information (see [`crate::fontinfo`]), without the design values.
    pub info: Info,
    /// The master's blue zones, stem snaps and guidelines ([`crate::design`]), placed on the
    /// master by [`place_design`] once the document's metrics are known.
    pub design: Design,
    /// Those of the design keys kept as written: parameters of the master.
    pub design_raw: Vec<Param>,
    /// Which metric kinds the file has (the master's values follow [`METRICS`]).
    pub metrics: Vec<(&'static str, f64)>,
    pub features: Option<String>,
    /// The groups that are not kerning groups, in file order.
    pub groups: Vec<Group>,
    /// In glyph order.
    pub glyphs: Vec<Glyph>,
    pub kerning: MasterKerning,
    pub files: BTreeMap<String, Arc<[u8]>>,
    pub format_version: u32,
}

/// The id of the master layer's layer in a non-default UFO layer.
fn layer_id(master: &str, layer: &str) -> String {
    glyphkeys::synthetic_id(master, layer)
}

/// The name of the default layer of the UFO a master was read from, as its `layercontents.plist`
/// says (`public.default` when it says none).
pub(crate) fn kept_default_layer(m: &Master) -> String {
    ufo_get(&m.extra, LAYERCONTENTS)
        .and_then(Value::as_array)
        .and_then(|a| {
            a.items.iter().find_map(|p| match p.as_array().map(|a| a.items.as_slice()) {
                Some([n, d]) if d.as_str() == Some(DEFAULT_DIR) => n.as_str().map(str::to_string),
                _ => None,
            })
        })
        .unwrap_or_else(|| "public.default".to_string())
}

/// Reads the UFO in `files` as the master `master_id`.
pub(crate) fn read_master(files: &Files, master_id: &str) -> Result<ReadUfo> {
    let meta_doc = read_plist(files, METAINFO)?.ok_or_else(|| bad(METAINFO, "missing: this is not a UFO"))?;
    let plist_style = PlistStyle::of(&meta_doc);
    let Value::Dict(mut meta) = meta_doc.root else { return Err(bad(METAINFO, "the file does not hold a dictionary")) };
    let file_version = match meta.get("formatVersion").and_then(as_f64) {
        Some(3.0) => 3,
        Some(2.0) => 2,
        Some(1.0) => ufo1::VERSION,
        Some(v) => return Err(UfoError::Unsupported(format!("UFO format version {v}"))),
        None => return Err(bad(METAINFO, "no formatVersion")),
    };
    // A UFO 1 is read as its UFO 2 form and becomes a UFO 3 source ([`crate::ufo1`]).
    let upconvert = file_version == ufo1::VERSION;
    let format_version = if upconvert { 3 } else { file_version };
    if upconvert {
        patch(&mut meta, "formatVersion", Some(Value::Number(Number::int(3))));
    }
    let glif_format = if file_version == 3 { 2 } else { 1 };

    let mut master = Master { id: master_id.to_string(), ..Default::default() };
    let mx = &mut master.extra;
    mx.push(Family::Ufo, "plistStyle", plist_style.to_value());
    mx.push(Family::Ufo, METAINFO, Value::Dict(meta));
    let mut plists = Plists { files, style: plist_style, styles: Dict::new() };

    // fontinfo.plist; a UFO 1's lib holds some of its values.
    let mut info = plists.dict(FONTINFO)?;
    let mut ufo1_lib = None;
    let mut ufo1_features = None;
    if upconvert {
        let mut converted = ufo1::info(info.as_ref().unwrap_or(&Dict::new()));
        let mut lib = plists.dict(LIB).ok().flatten();
        if let Some(lib) = &mut lib {
            ufo1_features = ufo1::take_lib_data(lib, &mut converted);
        }
        info = (info.is_some() || !converted.is_empty()).then_some(converted);
        ufo1_lib = Some(lib);
    }
    // A lib that is not a well-formed property list (some old tools wrote keys without values)
    // is kept as a file and written back as it is.
    let lib = match ufo1_lib {
        Some(lib) => lib,
        None => plists.dict(LIB).ok().flatten(),
    };
    // A style-map style outside the four that the writer moved to the lib ([`write_master`]).
    if let (Some(info), Some(v)) = (&mut info, lib.as_ref().and_then(|d| d.get(STYLE_MAP_STYLE_LIB)))
        && info.get(STYLE_MAP_STYLE).is_none()
    {
        patch(info, STYLE_MAP_STYLE, Some(v.clone()));
    }
    let get = |k: &str| info.as_ref().and_then(|d| d.get(k));
    let family_name = get("familyName").and_then(Value::as_str).map(str::to_string);
    master.name = get("styleName").and_then(Value::as_str).unwrap_or("").to_string();
    let units_per_em = get("unitsPerEm").and_then(as_f64);
    let version_major = get("versionMajor").and_then(Value::as_num).and_then(int_of);
    let version_minor = get("versionMinor").and_then(Value::as_num).and_then(int_of);
    let note = get("note").and_then(Value::as_str).map(str::to_string);
    let mut metrics = Vec::new();
    for (key, kind) in METRICS {
        if let Some(v) = get(key).and_then(as_f64) {
            // The UFO measures the italic angle counter-clockwise; the model clockwise.
            metrics.push((kind, if kind == metric::ITALIC_ANGLE { if v == 0.0 { 0.0 } else { -v } } else { v }));
        }
    }
    let upm = self::units_per_em(units_per_em).map_or(1000.0, f64::from);
    let mut font_info = fontinfo::read(info.as_ref().unwrap_or(&Dict::new()), upm);
    let design = std::mem::take(&mut font_info.design);
    let (design_raw, rest): (Vec<Param>, Vec<Param>) =
        std::mem::take(&mut font_info.params.0).into_iter().partition(|p| fontinfo::is_raw(p) && design::KEYS.contains(&p.name.as_str()));
    font_info.params.0 = rest;
    if let Some(info) = info {
        master.extra.push(Family::Ufo, FONTINFO, Value::Dict(info));
    }

    // lib.plist
    let order = strings(lib.as_ref().and_then(|d| d.get("public.glyphOrder")));
    let skip: HashSet<String> = strings(lib.as_ref().and_then(|d| d.get("public.skipExportGlyphs"))).into_iter().collect();
    let glyph_map = |key: &str| lib.as_ref().and_then(|d| d.get(key)).and_then(Value::as_dict).cloned();
    let (ps_names, ot_categories) = (glyph_map(POSTSCRIPT_NAMES), glyph_map(OPENTYPE_CATEGORIES));
    if let Some(lib) = &lib {
        let mut data = Dict::new();
        for (k, v) in lib.entries.iter().filter(|(k, _)| !TYPED_LIB.contains(&k.as_str())) {
            data.insert(k.as_str(), v.without_spellings());
        }
        master.user_data = UserData(data);
    }
    if let Some(lib) = lib {
        master.extra.push(Family::Ufo, LIB, Value::Dict(lib));
    }

    // layers
    let layer_list: Vec<(String, String)> = match plists.value(LAYERCONTENTS)? {
        Some(root) => {
            let pairs = root.as_array().ok_or_else(|| bad(LAYERCONTENTS, "the file does not hold a list"))?;
            let mut out = Vec::new();
            for item in &pairs.items {
                match item.as_array().map(|a| a.items.as_slice()) {
                    Some([n, d]) => match (n.as_str(), d.as_str()) {
                        (Some(n), Some(d)) => out.push((n.to_string(), d.to_string())),
                        _ => return Err(bad(LAYERCONTENTS, "an entry is not a pair of strings")),
                    },
                    _ => return Err(bad(LAYERCONTENTS, "an entry is not a pair of strings")),
                }
            }
            master.extra.push(Family::Ufo, LAYERCONTENTS, root);
            out
        }
        None => {
            if upconvert {
                master.extra.push(Family::Ufo, LAYERCONTENTS, ufo1::layer_contents());
            }
            vec![("public.default".to_string(), DEFAULT_DIR.to_string())]
        }
    };
    if !layer_list.iter().any(|(_, d)| d == DEFAULT_DIR) {
        return Err(bad(LAYERCONTENTS, "no layer is stored in `glyphs`"));
    }
    let mut layerinfo = Dict::new();
    let mut contents_kept = Dict::new();
    let mut missing_dirs: Vec<Value> = Vec::new();
    let mut glyphs: Vec<Glyph> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut glif_style: Option<Style> = None;
    let mut consumed: HashSet<String> = HashSet::new();
    for (layer_name, dir) in &layer_list {
        let is_default = dir == DEFAULT_DIR;
        if !is_plain_dir(dir) {
            return Err(bad(LAYERCONTENTS, format!("`{dir}` is not a directory name")));
        }
        let contents_path = format!("{dir}/{CONTENTS}");
        let Some(contents) = plists.dict(&contents_path)? else {
            // Listed but absent (a test file's shortcut): an empty layer, and written as one.
            if is_default {
                return Err(bad(&contents_path, "missing"));
            }
            missing_dirs.push(Value::String(dir.as_str().into()));
            continue;
        };
        consumed.insert(contents_path.clone());
        let info_path = format!("{dir}/{LAYERINFO}");
        if let Some(v) = plists.value(&info_path)? {
            layerinfo.insert(layer_name.as_str(), v);
            consumed.insert(info_path);
        }
        for (name, file) in &contents.entries {
            let name = name.as_str();
            let file = file.as_str().ok_or_else(|| bad(&contents_path, format!("`{name}` has no file name")))?;
            if !is_plain_file(file) {
                return Err(bad(&contents_path, format!("`{file}` is not a file name")));
            }
            let path = format!("{dir}/{file}");
            let glif_text = text(files, &path)?.ok_or_else(|| bad(&path, "missing"))?;
            consumed.insert(path.clone());
            let id = if is_default { master_id.to_string() } else { layer_id(master_id, layer_name) };
            let g = glif::read(glif_text, &path, &id, master_id)?;
            let note = g.note;
            let mut layer = g.layer;
            if g.name != name {
                // The file names another glyph than `contents.plist` does (a copied file).
                let pair = vec![Value::String(name.into()), Value::String(g.name.as_str().into())];
                layer.extra.push(Family::Ufo, "glifName", Value::Array(Array { items: pair, layout: Layout::Compact, ..Default::default() }));
            }
            let style = glif_style.get_or_insert_with(|| g.style.clone());
            if g.style != *style {
                layer.extra.push(Family::Ufo, "glifStyle", g.style.to_value());
            }
            if g.format != glif_format {
                layer.extra.push(Family::Ufo, "format", Value::Number(Number::int(i64::from(g.format))));
            }
            if !is_default {
                layer.info.name = Some(layer_name.clone());
                if !g.unicodes.is_empty() {
                    let items = g.unicodes.iter().map(|u| Value::Number(Number::int(i64::from(*u)))).collect();
                    layer.extra.push(Family::Ufo, "unicodes", Value::Array(Array { items, layout: Layout::Compact, ..Default::default() }));
                }
                // A note of another layer is that layer's own.
                if let Some(n) = &note {
                    layer.extra.push(Family::Ufo, "note", Value::String(n.as_str().into()));
                }
            }
            let i = match index.get(name) {
                Some(&i) => i,
                None => {
                    let mut glyph = Glyph::new(name);
                    glyph.export = !skip.contains(name);
                    glyphs.push(glyph);
                    index.insert(name.to_string(), glyphs.len() - 1);
                    glyphs.len() - 1
                }
            };
            let Some(glyph) = glyphs.get_mut(i) else { continue };
            if is_default {
                glyph.unicodes = g.unicodes;
                if let Some(n) = note {
                    glyph.info.note = n;
                }
                if let Some(c) = glif_mark_color(&layer) {
                    glyph.info.color = Some(c);
                }
                if glyph_file_name(name, &HashSet::new()).as_deref() != Some(file) {
                    glyph.file_name = Some(file.to_string());
                }
            } else if glyph.file_name.clone().or_else(|| glyph_file_name(name, &HashSet::new())).as_deref() != Some(file) {
                // Another file name than the glyph has in the default layer of this UFO.
                layer.extra.push(Family::Ufo, "fileName", Value::String(file.into()));
            }
            glyph.layers.push(layer);
        }
        contents_kept.insert(dir.as_str(), Value::Dict(contents));
    }
    master.extra.push(Family::Ufo, "contents", Value::Dict(contents_kept));
    if !missing_dirs.is_empty() {
        master.extra.push(Family::Ufo, "missingLayers", Value::Array(Array { items: missing_dirs, ..Default::default() }));
    }
    if !layerinfo.is_empty() {
        master.extra.push(Family::Ufo, "layerinfo", Value::Dict(layerinfo));
    }
    master.extra.push(Family::Ufo, "glifStyle", glif_style.unwrap_or_else(Style::fonttools).to_value());

    // Backgrounds: a glyph of `public.background` is the master layer's background, one of
    // `<name>.background` that of the layer `<name>`. Then the vertical origins, measured from
    // the ascender as the reference measures them. A glyph's master layer comes first.
    let frame = vertical_frame(&font_info.params, &metrics);
    let vhea = VHEA_PARAMS.iter().all(|n| font_info.params.get(n).is_some());
    for g in &mut glyphs {
        attach_backgrounds(g, master_id);
        for l in &mut g.layers {
            type_vertical(l, frame);
            // A GLIF without a height has height 0, which ufo2ft and fontc write when the font
            // has vertical metrics. The model's "no height" is the default height.
            if vhea
                && l.id == master_id
                && l.info.vert_width.is_none()
                && ufo_get(&l.extra, "defaultHeight").is_none()
                && ufo_get(&l.extra, "height").is_none()
            {
                l.info.vert_width = Some(0.0);
            }
        }
        g.layers.sort_by_key(|l| l.id != master_id);
    }
    // Glyph order: the lib's order first, then the rest as `contents.plist` lists them.
    let mut ordered: Vec<Glyph> = Vec::with_capacity(glyphs.len());
    let mut slots: Vec<Option<Glyph>> = glyphs.into_iter().map(Some).collect();
    for name in &order {
        if let Some(&i) = index.get(name)
            && let Some(slot) = slots.get_mut(i)
            && let Some(g) = slot.take()
        {
            ordered.push(g);
        }
    }
    ordered.extend(slots.into_iter().flatten());
    // Production names and OpenType categories.
    for g in &mut ordered {
        let says = |map: &Option<Dict>| map.as_ref().and_then(|m| m.get(&g.name)).and_then(Value::as_str).map(str::to_string);
        let (production, category) = (says(&ps_names), says(&ot_categories));
        if production.is_some() {
            g.info.production = production;
        }
        if category.is_some() {
            g.info.opentype_category = category;
        }
    }

    // Kerning groups, other groups and kerning. A UFO 2's kerning groups get their UFO 3 names.
    let groups = plists.dict(GROUPS)?;
    let kerning_dict = plists.dict(KERNING)?;
    let renames = match (&groups, file_version) {
        (Some(g), 1 | 2) => {
            let names: HashSet<&str> = ordered.iter().map(|g| g.name.as_str()).collect();
            kerning::ufo2_renames(g, kerning_dict.as_ref(), &names)
        }
        _ => kerning::Renames::default(),
    };
    let mut other_groups = Vec::new();
    if let Some(groups) = &groups {
        let by_name: HashMap<String, usize> = ordered.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        for (group, members) in &groups.entries {
            // A UFO 2 group used on both sides is a kerning group of each.
            let names = [renames.new_name(true, group.as_str()), renames.new_name(false, group.as_str())];
            let names: Vec<&str> = if names.iter().any(Option::is_some) { names.into_iter().flatten().collect() } else { vec![group.as_str()] };
            for name in names {
                let (side_right, short) = match (name.strip_prefix(KERN1), name.strip_prefix(KERN2)) {
                    (Some(s), _) => (true, s),
                    (_, Some(s)) => (false, s),
                    _ => {
                        if kerning::is_name_list(members) {
                            other_groups.push(Group { name: name.to_string(), members: strings(Some(members)) });
                        }
                        continue;
                    }
                };
                for m in strings(Some(members)) {
                    if let Some(g) = by_name.get(&m).and_then(|&i| ordered.get_mut(i)) {
                        let slot = if side_right { &mut g.kern_groups.right } else { &mut g.kern_groups.left };
                        if slot.is_none() {
                            *slot = Some(short.to_string());
                        }
                    }
                }
            }
        }
    }
    if let Some(groups) = groups {
        master.extra.push(Family::Ufo, GROUPS, Value::Dict(groups));
    }
    let mut kerning = MasterKerning { master: master_id.to_string(), firsts: Vec::new() };
    if let Some(k) = kerning_dict {
        for (first, seconds) in &k.entries {
            let seconds = seconds.as_dict().ok_or_else(|| bad(KERNING, format!("`{}` does not map to a dictionary", first.as_str())))?;
            let side = renames.new_name(true, first.as_str()).unwrap_or(first.as_str());
            let mut f = KernFirst { first: side.to_string(), seconds: Vec::with_capacity(seconds.len()) };
            for (second, value) in &seconds.entries {
                let v = as_f64(value).ok_or_else(|| bad(KERNING, format!("the pair `{}` `{}` has no number", first.as_str(), second.as_str())))?;
                f.seconds.push((renames.new_name(false, second.as_str()).unwrap_or(second.as_str()).to_string(), v));
            }
            kerning.firsts.push(f);
        }
        master.extra.push(Family::Ufo, KERNING, Value::Dict(k));
    }

    let file_features = match files.get(FEATURES) {
        Some(bytes) => match std::str::from_utf8(bytes) {
            Ok(t) => Some(t.to_string()),
            // Not text: kept as a file.
            Err(_) => None,
        },
        None => None,
    };
    if file_features.is_some() {
        consumed.insert(FEATURES.to_string());
    }
    let features = ufo1_features.or(file_features);
    for f in [METAINFO, FONTINFO, GROUPS, KERNING, LAYERCONTENTS] {
        consumed.insert(f.to_string());
    }
    if ufo_get(&master.extra, LIB).is_some() {
        consumed.insert(LIB.to_string());
    }
    if !plists.styles.is_empty() {
        master.extra.push(Family::Ufo, "plistStyles", Value::Dict(plists.styles));
    }
    let mut rest = BTreeMap::new();
    for (path, bytes) in files {
        if !consumed.contains(path) {
            rest.insert(path.clone(), Arc::from(bytes.as_slice()));
        }
    }
    Ok(ReadUfo {
        master,
        family_name,
        units_per_em,
        version_major,
        version_minor,
        note,
        info: font_info,
        design,
        design_raw,
        metrics,
        features,
        groups: other_groups,
        glyphs: ordered,
        kerning,
        files: rest,
        format_version,
    })
}

/// The UFO layer that holds the master layers' backgrounds.
pub(crate) const BACKGROUND: &str = "public.background";
/// What a background's UFO layer name adds to its layer's.
pub(crate) const BACKGROUND_SUFFIX: &str = ".background";
/// The layer extra key (designspace family) of a master layer whose background goes to a UFO
/// layer other than `public.background`.
pub(crate) const BACKGROUND_LAYER: &str = "backgroundLayer";

/// The ascender and descender a layer's vertical metrics are measured from: the `typoAscender`
/// and `typoDescender` parameters, else the ascender and descender metrics (glyphsLib's
/// `_get_typo_ascender_descender`).
type Frame = (Option<f64>, Option<f64>);

fn vertical_frame(params: &fontcraft_model::Params, metrics: &[(&str, f64)]) -> Frame {
    let one = |param: &str, kind: &str| {
        let p = params.get(param).and_then(|v| match v {
            Value::String(s) => s.as_str().trim().parse::<f64>().ok(),
            v => v.as_num().map(Num::as_f64),
        });
        p.or_else(|| metrics.iter().find(|(k, _)| *k == kind).map(|(_, v)| *v)).filter(|v| v.is_finite())
    };
    (one("typoAscender", metric::ASCENDER), one("typoDescender", metric::DESCENDER))
}

/// [`vertical_frame`] of `master` in the model.
fn model_frame(font: &Font, master: &Master) -> Frame {
    let info = Info { params: font.info.params.clone(), ..Default::default() };
    let own = Info { params: master.params.clone(), ..Default::default() };
    let params = fontinfo::effective(&info, &own).params;
    let metrics: Vec<(&str, f64)> =
        [metric::ASCENDER, metric::DESCENDER].into_iter().filter_map(|k| font.metric(master, k).map(|m| (k, m.pos))).collect();
    vertical_frame(&params, &metrics)
}

/// The vertical advance a layer of `master` has when it states none: the typographic ascender
/// minus the descender, as glyphsLib measures it.
pub fn default_vertical_width(font: &Font, master: &Master) -> Option<f64> {
    default_height(model_frame(font, master))
}

/// The default advance height: the ascender minus the descender.
fn default_height((a, d): Frame) -> Option<f64> {
    Some(a? - d?)
}

/// A lib's `public.verticalOrigin` as the reference's vertical origin (the ascender minus it),
/// when it is not the ascender itself (the default), the ascender is known and the subtraction
/// gives the lib's number back exactly.
fn vertical_origin_of(lib: Option<&Dict>, ascender: Option<f64>) -> Option<f64> {
    let y = lib?.get(glif::VERTICAL_ORIGIN)?.as_num()?.as_f64();
    let a = ascender?;
    let o = a - y;
    (y.is_finite() && y != a && o.abs() <= fontcraft_model::MAX_COORDINATE && a - o == y).then_some(o)
}

/// The parameters that make a UFO vertical for ufo2ft: its `openTypeVheaVertTypo…` keys.
const VHEA_PARAMS: [&str; 3] = ["vheaVertAscender", "vheaVertDescender", "vheaVertLineGap"];

/// Reads the vertical metrics as glyphsLib's `to_glyphs_glyph_height_and_vertical_origin` does:
/// a height that is the default and a vertical origin at the ascender are no values (the file's
/// own stay, `defaultHeight` marking the height).
fn type_vertical(l: &mut Layer, frame: Frame) {
    if l.info.vert_width.is_some() && l.info.vert_width == default_height(frame) {
        l.info.vert_width = None;
        l.extra.push(Family::Ufo, "defaultHeight", Value::Bool(true));
    }
    if let Some(o) = vertical_origin_of(ufo_get(&l.extra, "lib").and_then(Value::as_dict), frame.0) {
        l.info.vert_origin = Some(o);
    }
    l.info.tidy();
}

/// Whether glyphsLib treats the font as vertical (`_is_vertical`): the font or a master has the
/// three `vhea` parameters (either spelling of each), or a master layer has a vertical metric.
pub(crate) fn is_vertical(font: &Font) -> bool {
    let pairs =
        [["vheaVertAscender", "vheaVertTypoAscender"], ["vheaVertDescender", "vheaVertTypoDescender"], ["vheaVertLineGap", "vheaVertTypoLineGap"]];
    let all = |p: &fontcraft_model::Params| pairs.iter().all(|pair| pair.iter().any(|n| p.get(n).is_some()));
    if all(&font.info.params) || font.masters.iter().any(|m| all(&m.params)) {
        return true;
    }
    let masters: HashSet<&str> = font.masters.iter().map(|m| m.id.as_str()).collect();
    font.glyphs
        .iter()
        .flat_map(|g| g.layers.iter())
        .any(|l| masters.contains(l.id.as_str()) && (l.info.vert_width.is_some() || l.info.vert_origin.is_some()))
}

/// Moves the glyphs of background layers into the backgrounds of their layers: `public.background`
/// to the master layer, `<name>.background` to the layer named `<name>` (glyphsLib's
/// `to_glyphs_layer`). A background whose layer is missing or has one stays a layer.
fn attach_backgrounds(g: &mut Glyph, master_id: &str) {
    let mut i = 0;
    while i < g.layers.len() {
        let target = match g.layers.get(i).and_then(|l| l.info.name.as_deref()) {
            Some(BACKGROUND) => g.layers.iter().position(|f| f.id == master_id && f.master == master_id),
            Some(n) => n
                .strip_suffix(BACKGROUND_SUFFIX)
                .and_then(|fg| g.layers.iter().position(|f| !f.is_master_layer() && f.master == master_id && f.info.name.as_deref() == Some(fg))),
            None => None,
        };
        match target.filter(|&t| t != i && g.layers.get(t).is_some_and(|f| f.info.background.is_none())) {
            Some(t) => {
                let bg = background_of(g.layers.remove(i));
                let t = if t > i { t - 1 } else { t };
                if let Some(f) = g.layers.get_mut(t) {
                    f.info.background = Some(bg);
                }
            }
            None => i += 1,
        }
    }
}

/// A background layer's GLIF as a background: shapes, anchors, guides and image as fields; the
/// advance (`width`, `height`) and everything the GLIF reader kept in its `Extra`.
pub(crate) fn background_of(l: Layer) -> fontcraft_model::Background {
    let mut extra = l.extra;
    if l.width != 0.0 {
        extra.push(Family::Ufo, "width", num_value(l.width));
    }
    if let Some(h) = l.info.vert_width {
        extra.push(Family::Ufo, "height", num_value(h));
    }
    fontcraft_model::Background {
        shapes: l.shapes,
        anchors: l.anchors,
        guides: l.info.guides.clone(),
        image: l.info.image.clone(),
        extra,
        ..Default::default()
    }
}

/// A background as the layer its GLIF is written from (the inverse of [`background_of`]). Its
/// width is the file's, else the one a `.glyphs` file states in the background's user data
/// (glyphsLib's `backgroundWidth`); its user data is its lib but the keys
/// [`crate::glyphkeys`] typed.
pub(crate) fn background_layer(b: &fontcraft_model::Background, master: &str) -> Layer {
    let number = |k: &str| ufo_get(&b.extra, k).and_then(Value::as_num).map(|n| n.as_f64());
    let mut l = Layer::new(master, number("width").or_else(|| glyphkeys::background_width(b)).unwrap_or(0.0));
    l.id = format!("{master}#{BACKGROUND}");
    l.shapes = b.shapes.clone();
    l.anchors = b.anchors.clone();
    l.info.guides = b.guides.clone();
    l.info.image = b.image.clone();
    l.info.vert_width = number("height").filter(|h| h.abs() <= fontcraft_model::MAX_COORDINATE);
    if let Some(lib) = ufo_get(&b.extra, "lib").and_then(Value::as_dict) {
        l.info.user_data = glyphkeys::untyped_user_data(lib, |k| glyphkeys::background_typed(b, k));
    }
    let typed_height = l.info.vert_width.is_some();
    for e in b.extra.iter().filter(|e| !(e.family == Family::Ufo && (e.key.as_str() == "width" || e.key.as_str() == "height" && typed_height))) {
        l.extra.push(e.family, e.key.as_str(), e.value.clone());
    }
    l.info.tidy();
    l
}

fn int_of(n: Num) -> Option<i64> {
    match n {
        Num::Int(i) => Some(i),
        // A whole number written as a real.
        #[allow(clippy::cast_possible_truncation)]
        Num::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => Some(f as i64),
        Num::Float(_) => None,
    }
}

fn is_plain_file(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

fn is_plain_dir(name: &str) -> bool {
    is_plain_file(name)
}

/// The order of a UFO document's metric kinds: the `fontinfo.plist` metrics, with the baseline,
/// which has no key, after the x-height (as the reference orders them).
const KIND_ORDER: [&str; 6] = [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER, metric::ITALIC_ANGLE];

/// The metric kinds of a document made of these UFOs, in [`KIND_ORDER`]: every kind a file has,
/// and the baseline when a zone at 0 has no metric of its file to be the overshoot of.
pub(crate) fn metric_kinds<'a>(read: impl Iterator<Item = &'a ReadUfo> + Clone) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = Vec::new();
    for r in read.clone() {
        for (k, _) in &r.metrics {
            if !kinds.contains(k) {
                kinds.push(k);
            }
        }
    }
    let baseline = read.into_iter().any(|r| {
        r.design.zones.iter().any(|z| z.pos == 0.0 && z.size != 0.0) && !r.metrics.iter().any(|(k, v)| *k != metric::ITALIC_ANGLE && *v == 0.0)
    });
    if baseline {
        kinds.push(metric::BASELINE);
    }
    kinds.sort_by_key(|k| KIND_ORDER.iter().position(|m| m == k));
    kinds
}

/// How many horizontal and vertical stems the font's stem definitions have: those of the first
/// UFO in `read` with any.
pub(crate) fn stem_counts<'a>(mut read: impl Iterator<Item = &'a ReadUfo>) -> (usize, usize) {
    read.find(|r| !r.design.stems_h.is_empty() || !r.design.stems_v.is_empty()).map_or((0, 0), |r| (r.design.stems_h.len(), r.design.stems_v.len()))
}

/// The font's stem definitions for UFO masters: unnamed, the horizontal ones first.
pub(crate) fn stem_defs((h, v): (usize, usize)) -> Vec<StemDef> {
    (0..h + v).map(|i| StemDef { horizontal: i < h, ..Default::default() }).collect()
}

/// Puts a UFO's design values on its master, for a document whose metric kinds are `kinds` and
/// whose stems are `stems` ([`stem_counts`]). A zone at a position one of the file's metrics (or
/// the baseline) has becomes that metric's overshoot, the first such metric without one; a zone
/// of size 0 and the rest stay zones. Stems the definitions cannot hold (another count) are kept
/// as written, as the master's parameters.
pub(crate) fn place_design(
    master: &mut Master,
    metrics: &[(&'static str, f64)],
    design: Design,
    raw: Vec<Param>,
    kinds: &[&'static str],
    stems: (usize, usize),
) {
    let has = |k: &str| k == metric::BASELINE || metrics.iter().any(|(m, _)| *m == k);
    let mut values: Vec<MetricValue> =
        kinds.iter().map(|k| MetricValue { pos: metrics.iter().find(|(m, _)| m == k).map_or(0.0, |(_, v)| *v), over: 0.0 }).collect();
    let mut zones = Vec::new();
    for z in design.zones {
        let at = kinds.iter().zip(values.iter()).position(|(k, v)| *k != metric::ITALIC_ANGLE && has(k) && v.pos == z.pos && v.over == 0.0);
        match at.filter(|_| z.size != 0.0).and_then(|i| values.get_mut(i)) {
            Some(v) => v.over = z.size,
            None => zones.push(z),
        }
    }
    // Trailing metrics the file does not have stay absent, so a compile gives them the format's
    // default (ufo2ft's, which fontc follows for that UFO alone) rather than 0. One the file
    // lacks before one it has is 0: a value list has no gaps.
    let stated = kinds.iter().zip(values.iter()).rposition(|(k, v)| if *k == metric::BASELINE { v.over != 0.0 } else { has(k) });
    values.truncate(stated.map_or(0, |i| i + 1));
    master.metric_values = values;
    master.zones = zones;
    master.guides = design.guides;
    master.params.0.extend(raw);
    let counts = (design.stems_h.len(), design.stems_v.len());
    if counts == stems || counts == (0, 0) {
        master.stem_values = design.stems_h.into_iter().chain(design.stems_v).collect();
    } else {
        let kept = ufo_get(&master.extra, FONTINFO).and_then(Value::as_dict).cloned().unwrap_or_default();
        for key in [design::STEMS_H, design::STEMS_V] {
            let already = master.params.0.iter().any(|p| p.name == key && fontinfo::is_raw(p));
            if let Some(v) = kept.get(key).filter(|_| !already) {
                master.params.0.push(fontinfo::raw_param(key, v));
            }
        }
    }
}

/// Reads a single UFO as a document with one master.
pub fn read(files: &Files) -> Result<Font> {
    read_reporting(files).map(|(font, _)| font)
}

/// [`read`], with warnings about what was read in a plainer form than the files allow.
pub fn read_reporting(files: &Files) -> Result<(Font, Vec<String>)> {
    // The master id glyphsLib keeps in the lib, else the reader's own.
    let lib = read_plist(files, LIB).ok().flatten().and_then(|d| match d.root {
        Value::Dict(d) => Some(d),
        _ => None,
    });
    let master_id = lib.as_ref().and_then(libkeys::single_master_id).unwrap_or_else(|| libkeys::SINGLE_MASTER_ID.to_string());
    let mut r = read_master(files, &master_id)?;
    let kinds = metric_kinds(std::iter::once(&r));
    let stems = stem_counts(std::iter::once(&r));
    let metrics: Vec<MetricDef> = kinds.iter().map(|k| MetricDef::new(k)).collect();
    let mut master = r.master;
    place_design(&mut master, &r.metrics, std::mem::take(&mut r.design), std::mem::take(&mut r.design_raw), &kinds, stems);
    let units_per_em = units_per_em(r.units_per_em)?;
    let features = r.features.as_deref().map(crate::features::read).unwrap_or_default();
    let groups = std::mem::take(&mut r.groups);
    let mut font = Font {
        source: Source { format: SourceFormat::Ufo, ufo_version: Some(r.format_version), ..Default::default() },
        family_name: r.family_name.unwrap_or_default(),
        units_per_em,
        version_major: r.version_major.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        version_minor: r.version_minor.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        axes: Arc::default(),
        metrics: Arc::new(metrics),
        stems: Arc::new(stem_defs(stems)),
        numbers: Arc::default(),
        masters: Arc::new(vec![master]),
        instances: Arc::default(),
        glyphs: GlyphTable::from_glyphs(r.glyphs.into_iter().map(Arc::new).collect()),
        info: Arc::new(FontInfo { properties: r.info.properties, params: r.info.params, date: r.info.date, ..Default::default() }),
        kerning: Arc::new(Kerning {
            ltr: if r.kerning.firsts.is_empty() { KernTable::default() } else { KernTable::new(vec![r.kerning]) },
            ..Default::default()
        }),
        features: Arc::new(features),
        groups: Arc::new(groups),
        display_strings: Arc::default(),
        note: r.note.unwrap_or_default(),
        files: Arc::new(FileStore(r.files)),
        bitmap_strikes: Arc::default(),
        extra: Arc::default(),
    };
    glyphkeys::read(&mut font, &master_id);
    libkeys::read(&mut font, false, &master_id);
    let warnings: Vec<String> = crate::colorkeys::read(&mut font).into_iter().collect();
    crate::gdef::strip_derived(&mut font, &master_id);
    font.check()?;
    Ok((font, warnings))
}

/// The model's units per em for a UFO's `unitsPerEm` (1000 when it has none; rounded).
pub(crate) fn units_per_em(v: Option<f64>) -> Result<u16> {
    let upm = v.unwrap_or(1000.0).round();
    if (1.0..=f64::from(u16::MAX)).contains(&upm) {
        // In range, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(upm as u16)
    } else {
        Err(bad(FONTINFO, format!("unitsPerEm {upm} is out of range")))
    }
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// Sets `key` in a dictionary to `new`: the kept value stays when it means the same (equality
/// ignores spelling, and `500` equals `500.0`), `None` removes the key, and a new key goes in
/// sorted position, as fontTools sorts keys.
pub(crate) fn patch(d: &mut Dict, key: &str, new: Option<Value>) {
    let at = d.entries.iter().position(|(k, _)| k.as_str() == key);
    match (at, new) {
        (Some(i), None) => {
            d.remove_at(i);
        }
        (Some(i), Some(v)) => {
            if let Some((_, old)) = d.entries.get_mut(i)
                && *old != v
            {
                *old = v;
            }
        }
        (None, Some(v)) => {
            let i = d.entries.iter().position(|(k, _)| k.as_str() > key).unwrap_or(d.entries.len());
            d.insert_at(i, key, v);
        }
        (None, None) => {}
    }
}

/// Sets a font lib dictionary that maps glyph names to one glyph field (`public.postscriptNames`,
/// `public.openTypeCategories`). The kept dictionary stays while it says what the model says for
/// every glyph of the font (entries for other glyphs are its own business), and in a master that
/// read a dictionary of its own while the default master's still agrees. Otherwise it is made
/// again: its entries for glyphs the font does not have, and the model's, sorted by name.
fn patch_glyph_map(lib: &mut Dict, key: &str, font: &Font, is_default: bool, default_lib: &Dict, field: impl Fn(&Glyph) -> Option<&str>) {
    let kept = lib.get(key).and_then(Value::as_dict).cloned();
    let default_kept = default_lib.get(key).and_then(Value::as_dict);
    let says = |d: Option<&Dict>, g: &Glyph| d.and_then(|d| d.get(&g.name)).and_then(Value::as_str).map(str::to_string);
    let agrees = |d: Option<&Dict>| font.glyphs.iter().all(|g| says(d, g).as_deref() == field(g));
    if !is_default && agrees(default_kept) && font.glyphs.iter().any(|g| says(kept.as_ref(), g) != says(default_kept, g)) {
        return;
    }
    if agrees(kept.as_ref()) {
        return;
    }
    let mut entries: Vec<(Str, Value)> =
        kept.map(|d| d.entries.into_iter().filter(|(k, _)| font.glyph(k.as_str()).is_none()).collect()).unwrap_or_default();
    for g in font.glyphs.iter() {
        if let Some(v) = field(g) {
            entries.push((g.name.as_str().into(), str_value(v)));
        }
    }
    entries.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    entries.dedup_by(|a, b| a.0.as_str() == b.0.as_str());
    let value = (!entries.is_empty()).then(|| Value::Dict(Dict { entries, ..Default::default() }));
    patch(lib, key, value);
}

/// A number as fontTools would store it: an integer when whole.
fn num_value(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        // In range, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        Value::Number(Number::int(v as i64))
    } else {
        Value::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
    }
}

fn int_value(v: u16) -> Value {
    Value::Number(Number::int(i64::from(v)))
}

fn str_value(s: &str) -> Value {
    Value::String(s.into())
}

/// The kept dictionary `key` of a master, or an empty one.
/// A string of a master's `fontinfo.plist` as read.
pub(crate) fn kept_info_str(m: &Master, key: &str) -> Option<String> {
    kept_dict(m, FONTINFO).0.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn kept_dict(m: &Master, key: &str) -> (Dict, bool) {
    match ufo_get(&m.extra, key).and_then(Value::as_dict) {
        Some(d) => (d.clone(), true),
        None => (Dict::new(), false),
    }
}

/// What one UFO of a document holds when written.
pub(crate) struct Target<'a> {
    /// The master whose UFO this is.
    pub master: &'a Master,
    /// The document's default master: font-wide values a master's file shared with it when
    /// read are written from the model.
    pub default: &'a Master,
    /// Masters stored as a layer of this UFO (a designspace source with a `layer`), with the
    /// layer's name.
    pub layer_masters: Vec<(&'a Master, String)>,
    /// The prefix of this UFO's files in [`Font::files`] (`""` for a single UFO).
    pub prefix: String,
    /// A source of a designspace, whose document holds the font's user data, axis maps and
    /// origin (else a UFO of its own, whose lib holds them).
    pub designspace: bool,
}

/// The value of a font-wide setting for a master: the model's, except that a master whose own
/// value differed from the default master's when read keeps its own while the default master's
/// is still what the model holds.
fn per_master<T: PartialEq>(is_default: bool, model: T, default_read: T, mine_read: T) -> T {
    if !is_default && default_read == model && mine_read != default_read { mine_read } else { model }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort_unstable();
    v
}

/// The files of one master's UFO.
pub(crate) fn write_master(font: &Font, target: &Target<'_>) -> Result<Files> {
    // Full-colour layers of paths are written as layer glyphs of their own.
    let expanded = crate::colorglyphs::expand(font);
    let font = expanded.as_ref().unwrap_or(font);
    // Colour layers in the list are measured once for the whole walk.
    let _boxes = crate::colorkeys::Scope::enter();
    let (master, default) = (target.master, target.default);
    let mut out = Files::new();
    let version = font.source.ufo_version.unwrap_or(3);
    if !(2..=3).contains(&version) {
        return Err(UfoError::Unsupported(format!("writing UFO format version {version}")));
    }
    let glif_format = if version == 2 { 1 } else { 2 };
    let plist_style = ufo_get(&master.extra, "plistStyle").and_then(PlistStyle::from_value).unwrap_or_else(PlistStyle::fonttools);
    let glif_style = ufo_get(&master.extra, "glifStyle").and_then(Style::from_value).unwrap_or_else(Style::fonttools);
    let from_ufo = ufo_get(&master.extra, METAINFO).is_some();
    let styles = ufo_get(&master.extra, "plistStyles").and_then(Value::as_dict);
    let plist = |path: &str, value: &Value| -> Vec<u8> {
        styles.and_then(|s| s.get(path)).and_then(PlistStyle::from_value).unwrap_or_else(|| plist_style.clone()).write(value)
    };

    // metainfo.plist
    let (mut meta, _) = kept_dict(master, METAINFO);
    // A format 2 source written as format 3 needs the layer list format 2 does not have.
    let upgraded = meta.entries.iter().find(|(k, _)| k.as_str() == "formatVersion").and_then(|(_, v)| v.as_num()).is_some_and(|n| n.as_f64() < 3.0);
    // The creator stays as the file says; a new UFO names us.
    if !from_ufo {
        patch(&mut meta, "creator", Some(str_value(CREATOR)));
    }
    patch(&mut meta, "formatVersion", Some(Value::Number(Number::int(i64::from(version)))));
    out.insert(METAINFO.into(), plist(METAINFO, &Value::Dict(meta)));

    // fontinfo.plist. Font-wide values come from the model, except that a master whose file
    // differed from the default master's when read keeps its own value while the model still
    // holds the default master's.
    let (mut info, had_info) = kept_dict(master, FONTINFO);
    let (default_info, _) = kept_dict(default, FONTINFO);
    let is_default = std::ptr::eq(master, default);
    let font_wide = |key: &str, model: Option<Value>| -> Option<Value> {
        if !is_default {
            let (mine, theirs) = (info.get(key), default_info.get(key));
            if theirs == model.as_ref() && mine != theirs {
                return mine.cloned();
            }
        }
        model
    };
    // Whether the model has a value is judged by the default master's file; `font_wide` then
    // keeps this master's own where it differed.
    let family =
        font_wide("familyName", (!font.family_name.is_empty() || default_info.get("familyName").is_some()).then(|| str_value(&font.family_name)));
    let style_name = (!master.name.is_empty() || info.get("styleName").is_some()).then(|| str_value(&master.name));
    // A UFO without unitsPerEm reads as 1000; it is not added on saving.
    let upm = (!from_ufo || font.units_per_em != 1000 || default_info.get("unitsPerEm").is_some()).then(|| num_value(f64::from(font.units_per_em)));
    let upm = font_wide("unitsPerEm", upm);
    // A fractional unitsPerEm (read rounded) stays while the model keeps its rounded value.
    let upm = match (upm, info.get("unitsPerEm")) {
        (Some(new), Some(old)) if as_f64(old).map(f64::round) == as_f64(&new) => Some(old.clone()),
        (new, _) => new,
    };
    let major =
        font_wide("versionMajor", (font.version_major != 0 || default_info.get("versionMajor").is_some()).then(|| int_value(font.version_major)));
    let minor =
        font_wide("versionMinor", (font.version_minor != 0 || default_info.get("versionMinor").is_some()).then(|| int_value(font.version_minor)));
    let note = font_wide("note", (!font.note.is_empty() || default_info.get("note").is_some()).then(|| str_value(&font.note)));
    patch(&mut info, "familyName", family);
    patch(&mut info, "styleName", style_name);
    patch(&mut info, "unitsPerEm", upm);
    patch(&mut info, "versionMajor", major);
    patch(&mut info, "versionMinor", minor);
    patch(&mut info, "note", note);
    for (key, kind) in METRICS {
        let value = font.metric(master, kind).map(|m| if kind == metric::ITALIC_ANGLE && m.pos != 0.0 { -m.pos } else { m.pos });
        // A metric the master has no value for is left out, unless the file had it.
        let value = value.filter(|v| info.get(key).is_some() || !from_ufo || *v != 0.0);
        patch(&mut info, key, value.map(num_value));
    }
    // The rest of the font information. As above, a master whose file differed from the default
    // master's keeps its own value while the default master's is unchanged and this master has
    // none of its own.
    let (read_info, _) = kept_dict(master, FONTINFO);
    let mine_now = info_entries(font, master, &read_info).0;
    let default_now = if is_default { mine_now.clone() } else { info_entries(font, default, &default_info).0 };
    let mut keys: Vec<&str> = read_info.entries.iter().map(|(k, _)| k.as_str()).chain(mine_now.keys().map(String::as_str)).collect();
    keys.retain(|k| !fontinfo::ELSEWHERE.contains(k));
    keys.dedup();
    for key in keys {
        let mut value = mine_now.get(key).cloned();
        // A master's design values are its own, never the font's.
        if !is_default
            && !design::KEYS.contains(&key)
            && value.as_ref() == default_now.get(key)
            && default_info.get(key) == default_now.get(key)
            && read_info.get(key) != default_info.get(key)
        {
            value = read_info.get(key).cloned();
        }
        patch(&mut info, key, value);
    }
    // Style-map names the font's instance left to be computed are not written, as in fontmake's
    // instance UFOs.
    if let Some(stated) = font.info.style_map_stated {
        if !stated.family {
            patch(&mut info, "styleMapFamilyName", None);
        }
        if !stated.style {
            patch(&mut info, "styleMapStyleName", None);
        }
    }
    let off_list = info.get(STYLE_MAP_STYLE).filter(|v| !v.as_str().is_some_and(|s| STYLE_MAP_STYLES.contains(&s))).cloned();
    if off_list.is_some() {
        patch(&mut info, STYLE_MAP_STYLE, None);
    }
    if had_info || !info.is_empty() {
        out.insert(FONTINFO.into(), plist(FONTINFO, &Value::Dict(info)));
    }

    // groups.plist
    let groups = {
        let (mine, _) = kept_dict(master, GROUPS);
        let (default_read, _) = kept_dict(default, GROUPS);
        let default_now = write_groups(font, default, version);
        if !is_default && default_now == default_read && mine != default_read { mine } else { write_groups(font, master, version) }
    };
    // An empty file is written back where there was one.
    if !groups.is_empty() || ufo_get(&master.extra, GROUPS).is_some() {
        out.insert(GROUPS.into(), plist(GROUPS, &Value::Dict(groups)));
    }
    // kerning.plist
    let kerning = write_kerning(font, master, version);
    if !kerning.is_empty() || ufo_get(&master.extra, KERNING).is_some() {
        out.insert(KERNING.into(), plist(KERNING, &Value::Dict(kerning)));
    }
    // Layers.
    let missing: Vec<&str> = ufo_get(&master.extra, "missingLayers")
        .and_then(Value::as_array)
        .map(|a| a.items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut layers: Vec<(String, String)> = Vec::new();
    let kept_layers: Vec<(String, String)> = ufo_get(&master.extra, LAYERCONTENTS)
        .and_then(Value::as_array)
        .map(|a| {
            a.items
                .iter()
                .filter_map(|p| match p.as_array().map(|a| a.items.as_slice()) {
                    Some([n, d]) => Some((n.as_str()?.to_string(), d.as_str()?.to_string())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    // A new UFO's default layer has the name its master's layer lib keeps, as glyphsLib names it.
    let default_name = kept_layers.iter().find(|(_, d)| d == DEFAULT_DIR).map_or_else(|| glyphkeys::default_layer_name(master), |(n, _)| n.clone());
    // Which UFO layers the glyphs use, in the kept order, then in order of appearance.
    let plan = layer_plan(font, target);
    let mut used: Vec<String> = vec![default_name.clone()];
    used.extend(plan.iter().map(|(n, _)| n.clone()));
    for (n, d) in &kept_layers {
        if used.contains(n) || d == DEFAULT_DIR || missing.contains(&d.as_str()) {
            layers.push((n.clone(), d.clone()));
        }
    }
    if !layers.iter().any(|(_, d)| d == DEFAULT_DIR) {
        layers.insert(0, (default_name.clone(), DEFAULT_DIR.to_string()));
    }
    let mut taken: HashSet<String> = layers.iter().map(|(_, d)| d.to_lowercase()).collect();
    for n in &used {
        if !layers.iter().any(|(ln, _)| ln == n) {
            let dir = layer_dir_name(n, &taken).ok_or_else(|| bad(n, "no free directory name for the layer"))?;
            taken.insert(dir.to_lowercase());
            layers.push((n.clone(), dir));
        }
    }
    // Kept layers that no glyph uses any more are still listed when the file listed them empty.
    let layer_value = Value::Array(Array {
        items: layers.iter().map(|(n, d)| Value::Array(Array { items: vec![str_value(n), str_value(d)], ..Default::default() })).collect(),
        ..Default::default()
    });
    let kept_lc = ufo_get(&master.extra, LAYERCONTENTS).filter(|v| **v == layer_value).cloned();
    if version >= 3 && (from_ufo && kept_lc.is_some() || !from_ufo || upgraded || layers.len() > 1 || ufo_get(&master.extra, LAYERCONTENTS).is_some())
    {
        out.insert(LAYERCONTENTS.into(), plist(LAYERCONTENTS, &kept_lc.unwrap_or(layer_value)));
    }
    // lib.plist: the master's user data, glyphsLib's keys for what `fontinfo.plist` cannot hold
    // ([`crate::libkeys`]; a key the user data has is written as the user data says), and the
    // typed keys below.
    let (mut lib, _) = kept_dict(master, LIB);
    let (default_lib, _) = kept_dict(default, LIB);
    let glyphs_keys = libkeys::entries(&libkeys::Ctx {
        font,
        master,
        index: libkeys::master_index(font, master),
        is_default,
        designspace: target.designspace,
        new: !from_ufo,
        kept: &lib,
        default_kept: &default_lib,
        info: &read_info,
    })
    .map;
    // The master's layer libs go to the layers' `layerinfo.plist` ([`glyphkeys::layerinfo`]).
    let layer_names: Vec<String> = layers.iter().map(|(n, _)| n.clone()).collect();
    let read_lib = lib.clone();
    let in_layer = |k: &str| glyphkeys::in_layer_lib(k, &layer_names, &read_lib);
    // The font-wide colour layer list, from the listed layers and the entries the user data keeps.
    let writes_list = crate::colorkeys::writes_list(font, master);
    let stale: Vec<String> = lib
        .entries
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| {
            !TYPED_LIB.contains(k)
                && (master.user_data.get(k).is_none() || in_layer(k))
                && !glyphs_keys.contains_key(*k)
                && !(*k == crate::colorkeys::COLOR_LAYERS && writes_list)
        })
        .map(str::to_string)
        .collect();
    for k in stale {
        patch(&mut lib, &k, None);
    }
    patch(&mut lib, STYLE_MAP_STYLE_LIB, off_list);
    for (k, v) in &master.user_data.0.entries {
        if !TYPED_LIB.contains(&k.as_str()) && !in_layer(k.as_str()) && !(k.as_str() == crate::colorkeys::COLOR_LAYERS && writes_list) {
            patch(&mut lib, k.as_str(), Some(v.without_spellings()));
        }
    }
    for (k, v) in glyphs_keys {
        if master.user_data.get(&k).is_none() {
            patch(&mut lib, &k, Some(v));
        }
    }
    // The list as read stays while it says the same, in its own key order. Colours of the list
    // that are not palette entries go to the end of each palette.
    let palette_base = crate::colorkeys::palette_len(lib.get(crate::colorkeys::COLOR_PALETTES));
    if writes_list && let Some((v, added)) = crate::colorkeys::value(font, master, palette_base) {
        if lib.get(crate::colorkeys::COLOR_LAYERS).is_none_or(|k| !crate::colorkeys::close(k, &v)) {
            patch(&mut lib, crate::colorkeys::COLOR_LAYERS, Some(v));
        }
        if !added.is_empty() {
            let palettes = crate::colorkeys::with_added(lib.get(crate::colorkeys::COLOR_PALETTES), &added);
            patch(&mut lib, crate::colorkeys::COLOR_PALETTES, Some(palettes));
        }
    }
    // The kept list stays while it names the same glyphs, in whatever order, and in a master that
    // read a list of its own (often none) while the default master's is unchanged. A glyph whose
    // file in this UFO says `…Glyphs.Export` (glyphsLib's per-glyph key) says it there, not here.
    let kept_skip = sorted(strings(lib.get("public.skipExportGlyphs")));
    let by_key = |g: &Glyph| {
        from_ufo
            && !kept_skip.contains(&g.name)
            && g.layer(&master.id).and_then(|l| ufo_get(&l.extra, "lib")).and_then(Value::as_dict).is_some_and(|d| d.get(glyphkeys::EXPORT).is_some())
    };
    let skip: Vec<&str> = font.glyphs.iter().filter(|g| !g.export && !by_key(g)).map(|g| g.name.as_str()).collect();
    let model_skip = sorted(skip.iter().map(|s| s.to_string()).collect());
    let default_skip = sorted(strings(kept_dict(default, LIB).0.get("public.skipExportGlyphs")));
    let wanted = per_master(is_default, model_skip.clone(), default_skip, kept_skip.clone());
    if kept_skip != wanted {
        let skip_value = (!skip.is_empty()).then(|| Value::Array(Array { items: skip.iter().map(|s| str_value(s)).collect(), ..Default::default() }));
        patch(&mut lib, "public.skipExportGlyphs", skip_value);
    }
    // The glyph order. Reading puts the glyphs the kept list names first, in its order (it may
    // name glyphs the font lacks and leave some out, as the specification allows), so the list
    // agrees with the model while those glyphs still lead the model's order. A disagreeing list is
    // rewritten from the model; a UFO that had none gets none; a master whose file had its own
    // list keeps it while the default master's still agrees.
    // The glyphs a designspace makes of alternate layers stay out of a new list, as glyphsLib
    // leaves them out.
    // So do the layer glyphs of full-colour layers, which are not in the model's order at all.
    let made = |g: &Glyph| g.extra.get(Family::Designspace, crate::designspace::BRACKET_GLYPH).is_some() || crate::colorglyphs::is_generated(g);
    let order: Vec<String> = font.glyphs.iter().filter(|g| !crate::colorglyphs::is_generated(g)).map(|g| g.name.clone()).collect();
    let listed: Vec<String> = font.glyphs.iter().filter(|g| !made(g)).map(|g| g.name.clone()).collect();
    let agrees = |kept: &[String]| {
        let named: Vec<&String> = order.iter().filter(|n| kept.contains(n)).collect();
        let listed: Vec<&String> = kept.iter().filter(|n| order.contains(n)).collect();
        let mut seen = std::collections::HashSet::new();
        let listed: Vec<&String> = listed.into_iter().filter(|n| seen.insert(*n)).collect();
        named.len() == listed.len() && order.iter().take(named.len()).eq(listed.iter().copied())
    };
    let kept_order = lib.get("public.glyphOrder").map(|v| strings(Some(v)));
    let default_order = kept_dict(default, LIB).0.get("public.glyphOrder").map(|v| strings(Some(v)));
    let keep = if is_default {
        match &kept_order {
            Some(k) => agrees(k),
            None => from_ufo,
        }
    } else {
        default_order.as_deref().is_none_or(agrees) && from_ufo
    };
    if !keep {
        let value = Value::Array(Array { items: listed.iter().map(|n| str_value(n)).collect(), ..Default::default() });
        patch(&mut lib, "public.glyphOrder", Some(value));
    }
    patch_glyph_map(&mut lib, POSTSCRIPT_NAMES, font, is_default, &default_lib, |g| g.info.production.as_deref());
    // A font from another format gets classes from the glyphs' categories and anchors. A UFO's
    // own map is written as read: its entries that equal those classes are not stored on the
    // glyphs (`gdef::strip_derived`) and come back here.
    let derived = crate::gdef::derived(font, &master.id);
    let from_other = !fontcraft_model::is_ufo_family(font.source.format);
    let kept_classes = |d: &Dict, g: &Glyph| {
        d.get(OPENTYPE_CATEGORIES).and_then(Value::as_dict).and_then(|m| m.get(&g.name)).and_then(Value::as_str).map(str::to_string)
    };
    let (own_lib, default_lib_classes) = (lib.clone(), default_lib.clone());
    patch_glyph_map(&mut lib, OPENTYPE_CATEGORIES, font, is_default, &default_lib, |g| {
        if crate::colorglyphs::is_generated(g) {
            return None;
        }
        g.info.opentype_category.as_deref().or_else(|| {
            let class = derived.get(&g.name).copied()?;
            let kept = |d: &Dict| kept_classes(d, g).as_deref() == Some(class);
            (from_other || kept(&own_lib) || kept(&default_lib_classes)).then_some(class)
        })
    });
    if !lib.is_empty() {
        out.insert(LIB.into(), plist(LIB, &Value::Dict(lib)));
    }
    // features.fea
    let fea = crate::features::write(&font.features);
    // A master whose file differed from the default master's when read: `[default's, own]`.
    // `false` stands for no file.
    let model_fea = (!fea.is_empty() || !font.features.is_empty()).then_some(fea);
    let fea = match ufo_get(&master.extra, FEATURES).and_then(Value::as_array).map(|a| a.items.as_slice()) {
        Some([theirs, own]) if !is_default && theirs.as_str() == model_fea.as_deref() => own.as_str().map(str::to_string),
        _ => model_fea,
    };
    if let Some(fea) = fea {
        out.insert(FEATURES.into(), fea.into_bytes());
    }

    let layerinfo = ufo_get(&master.extra, "layerinfo").and_then(Value::as_dict);
    let kept_contents = ufo_get(&master.extra, "contents").and_then(Value::as_dict);
    let text_of = |l: &Layer, key: &str| ufo_get(&l.extra, key).and_then(Value::as_str).map(str::to_string);

    let frame = model_frame(font, master);
    let vertical = !from_ufo && is_vertical(font);
    let doc_default = std::ptr::eq(master, default);
    for (layer_name, dir) in &layers {
        let is_default = dir == DEFAULT_DIR;
        let mut contents = Dict::new();
        let mut files_taken: HashSet<String> = HashSet::new();
        // (glyph name, file, glyph, layer, whether the layer is a background).
        let mut entries: Vec<(String, String, &Glyph, Cow<'_, Layer>, bool)> = Vec::new();
        let mut parts: Vec<(&Glyph, Cow<'_, Layer>, bool)> = Vec::new();
        if is_default {
            parts.extend(font.glyphs.iter().filter_map(|g| g.layer(&master.id).map(|l| (g.as_ref(), Cow::Borrowed(l), false))));
        } else if let Some((_, items)) = plan.iter().find(|(n, _)| n == layer_name) {
            for (g, part) in items {
                parts.push((*g, part.layer(&master.id), matches!(part, Part::Background(_))));
            }
        }
        for (g, layer, is_background) in parts {
            // The layer's own file name, else the one the glyph has in this UFO's default layer.
            let file_of = |l: Option<&Layer>| l.and_then(|l| ufo_get(&l.extra, "fileName")).and_then(Value::as_str).map(str::to_string);
            let kept = file_of(Some(&layer)).or_else(|| file_of(g.layer(&master.id))).or_else(|| g.file_name.clone());
            entries.push((g.name.clone(), kept.unwrap_or_default(), g, layer, is_background));
        }
        for (_, file, _, _, _) in &entries {
            if !file.is_empty() {
                files_taken.insert(file.to_lowercase());
            }
        }
        for (name, file, g, layer, is_background) in &mut entries {
            if file.is_empty() {
                *file = glyph_file_name(name, &files_taken).ok_or_else(|| bad(name, "no free file name for the glyph"))?;
                files_taken.insert(file.to_lowercase());
            }
            let own = code_points_of(ufo_get(&layer.extra, "unicodes"));
            let unicodes: Vec<u32> = if is_default {
                // A master's own code points, where they differed from the default master's
                // when read and the glyph's are unchanged.
                match ufo_get(&layer.extra, "unicodesRead") {
                    Some(read) if code_points_of(Some(read)) == g.unicodes => own,
                    _ => g.unicodes.clone(),
                }
            } else if own.is_empty() && !from_ufo && !*is_background {
                // glyphsLib gives every layer's file the glyph's code points.
                g.unicodes.clone()
            } else {
                own
            };
            let style = ufo_get(&layer.extra, "glifStyle").and_then(Style::from_value).unwrap_or_else(|| glif_style.clone());
            let format =
                ufo_get(&layer.extra, "format").and_then(Value::as_num).and_then(int_of).and_then(|f| u32::try_from(f).ok()).unwrap_or(glif_format);
            // A file that named another glyph keeps doing so while the glyph keeps its name.
            let glif_name = match ufo_get(&layer.extra, "glifName").and_then(Value::as_array).map(|a| a.items.as_slice()) {
                Some([read_as, written]) if read_as.as_str() == Some(name.as_str()) => written.as_str().unwrap_or(name),
                _ => name.as_str(),
            };
            // The glyph's note and colour go to every master's file, except that a master whose
            // file had its own when read keeps it while the glyph's is what was read.
            let color_text = g.info.color.as_ref().and_then(glif::mark_color_text);
            let (note, mark_color) = if is_default {
                let note = match (text_of(layer, "noteRead"), text_of(layer, "note")) {
                    (Some(read), Some(own)) if read == g.info.note => own,
                    _ => g.info.note.clone(),
                };
                let mark = match text_of(layer, "markColorRead") {
                    Some(read) if read == color_text.as_deref().unwrap_or("") => MarkColor::Keep,
                    _ => color_text.as_deref().map_or(MarkColor::Remove, MarkColor::Set),
                };
                (note, mark)
            } else {
                (text_of(layer, "note").unwrap_or_default(), MarkColor::Keep)
            };
            // The vertical metrics, by glyphsLib's `to_ufo_glyph_height_and_vertical_origin`: in
            // a vertical font every layer gets them, the defaults where the layer has none. A
            // background's stay as read.
            let kept_lib = ufo_get(&layer.extra, "lib").and_then(Value::as_dict);
            let default_kept = ufo_get(&layer.extra, "defaultHeight").is_some();
            let (vertical_origin, height) = if *is_background {
                (None, layer.info.vert_width.or_else(|| ufo_get(&layer.extra, "height").and_then(Value::as_num).map(|n| n.as_f64())))
            } else {
                let origin = match (layer.info.vert_origin, frame.0) {
                    (Some(o), Some(a)) => Some(Some(a - o)),
                    (Some(_), None) => None,
                    (None, Some(a)) if vertical => Some(Some(a)),
                    // A value that read as one, since removed.
                    (None, a) => vertical_origin_of(kept_lib, a).map(|_| None),
                };
                let kept_height = ufo_get(&layer.extra, "height").and_then(Value::as_num).map(|n| n.as_f64());
                let height = layer.info.vert_width.or(kept_height).or_else(|| if vertical || default_kept { default_height(frame) } else { None });
                (origin, height)
            };
            // glyphsLib's lib keys, and the outline as they want it.
            let new_file =
                !from_ufo || kept_contents.and_then(|c| c.get(dir)).and_then(Value::as_dict).is_none_or(|d| d.get(name.as_str()).is_none());
            let primary = is_default && doc_default && !*is_background;
            let keys = glyphkeys::entries(&glyphkeys::Glif { font, glyph: g, layer, background: *is_background, primary, new: new_file, format });
            let shaped: Cow<'_, Layer> = if keys.paths_first || keys.zero_width || !keys.anchor_ids.is_empty() {
                let mut l = Layer::clone(layer);
                if keys.paths_first {
                    let (paths, components): (Vec<fontcraft_model::Shape>, Vec<fontcraft_model::Shape>) =
                        l.shapes.into_iter().partition(|s| !matches!(s, fontcraft_model::Shape::Component(_)));
                    l.shapes = paths;
                    l.shapes.extend(components);
                }
                if keys.zero_width {
                    l.width = 0.0;
                }
                for (k, id) in &keys.anchor_ids {
                    if let Some(a) = l.anchors.get_mut(*k) {
                        a.info.identifier = Some(id.clone());
                    }
                }
                Cow::Owned(l)
            } else {
                Cow::Borrowed(&**layer)
            };
            let head = glif::Head { name: glif_name, unicodes: &unicodes, note: &note, mark_color, vertical_origin, height, lib_keys: &keys.map };
            let text = glif::write(head, &shaped, format, &style)?;
            out.insert(format!("{dir}/{file}"), text.into_bytes());
        }
        // The kept contents, for its order and spellings; then the model's glyphs.
        if let Some(kept) = kept_contents.and_then(|c| c.get(dir)).and_then(Value::as_dict) {
            contents = kept.clone();
            contents.entries.retain(|(k, _)| entries.iter().any(|(n, _, _, _, _)| n == k.as_str()));
        }
        for (name, file, _, _, _) in &entries {
            patch(&mut contents, name, Some(str_value(file)));
        }
        if entries.is_empty() && missing.contains(&dir.as_str()) {
            continue;
        }
        let path = format!("{dir}/{CONTENTS}");
        out.insert(path.clone(), plist(&path, &Value::Dict(contents)));
        // The layer's lib: layer ids, the glyphs' layer order and the master's layer lib.
        let kept_info = layerinfo.and_then(|d| d.get(layer_name));
        if version >= 3 || kept_info.is_some() {
            let placed: Vec<(&Glyph, &Layer)> = entries.iter().filter(|e| !e.4).map(|(_, _, g, l, _)| (*g, &**l)).collect();
            let target = glyphkeys::UfoLayer { name: layer_name, is_default, kept: kept_info, parts: &placed };
            if let Some(info) = glyphkeys::layerinfo(font, master, &target, !from_ufo, &read_lib) {
                let path = format!("{dir}/{LAYERINFO}");
                out.insert(path.clone(), plist(&path, &info));
            }
        }
    }
    // Other files, as read. Only a UFO's: another format's side files have no place here.
    if matches!(font.source.format, SourceFormat::Ufo | SourceFormat::Designspace) {
        for (path, bytes) in &font.files.0 {
            if let Some(rel) = path.strip_prefix(target.prefix.as_str()) {
                out.entry(rel.to_string()).or_insert_with(|| bytes.to_vec());
            }
        }
    }
    Ok(out)
}

/// The font information of `master` as UFO keys (those [`crate::fontinfo`] handles), and what has
/// no place there. `read` is the master's `fontinfo.plist` as read.
fn info_entries(font: &Font, master: &Master, read: &Dict) -> fontinfo::Written {
    let font_info =
        Info { properties: font.info.properties.clone(), params: font.info.params.clone(), date: font.info.date.clone(), ..Default::default() };
    let own = Info { properties: master.properties.clone(), params: master.params.clone(), date: None, design: design_of(font, master) };
    fontinfo::write(&fontinfo::effective(&font_info, &own), f64::from(font.units_per_em), Some(read))
}

/// A master's design values as a UFO holds them: as zones, the overshoots of every metric except
/// the italic angle, then the master's other zones (glyphsLib's alignment zones of a format 3
/// master); its stem values split by their definitions' direction; its guides.
fn design_of(font: &Font, m: &Master) -> Design {
    Design { zones: font.alignment_zones(m), stems_h: font.stem_widths(m, true), stems_v: font.stem_widths(m, false), guides: m.guides.clone() }
}

/// Whether a UFO holds the parameter `p` in its lib, under one of glyphsLib's keys, rather than
/// in `fontinfo.plist` (or not at all).
pub fn lib_carries(p: &Param) -> bool {
    libkeys::carried(p, None)
}

/// Which writer a loss check is for. It decides where each master's UFO keeps its other files in
/// [`Font::files`]: under the prefix that writer strips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesFrom {
    /// [`write`]: one UFO, its files from the root of [`Font::files`].
    Single,
    /// [`write_one`]: one UFO per master, its files under the master's designspace file name when
    /// it has one, else from the root.
    PerMaster,
    /// A designspace: each UFO's files under its master's file name; a master without one gets a
    /// new name, under which the font holds no files.
    Designspace,
}

/// What saving `font` as one UFO ([`write`]) leaves out of its font information and user data,
/// one line each.
pub fn info_losses(font: &Font) -> Vec<String> {
    info_losses_for(font, FilesFrom::Single)
}

/// [`info_losses`] for the writer `files` names (not a designspace: see
/// [`crate::designspace::info_losses`]).
pub fn info_losses_for(font: &Font, files: FilesFrom) -> Vec<String> {
    let mut out = font_losses(font, files);
    for i in font.instances.iter() {
        let fields = [
            ("id", i.id.is_some()),
            ("type", i.kind.is_some()),
            ("visibility", !i.visible),
            ("weight class", i.weight_class.is_some()),
            ("width class", i.width_class.is_some()),
            ("style linking", i.is_bold || i.is_italic || i.link_style.is_some()),
            ("manual interpolation", i.manual_interpolation || !i.interpolation.is_empty()),
        ];
        let set: Vec<&str> = fields.iter().filter(|(_, on)| *on).map(|(what, _)| *what).collect();
        if !set.is_empty() {
            out.push(format!("instance `{}`: {}", i.name, set.join(", ")));
        }
        for p in &i.params.0 {
            out.push(format!("instance `{}`: parameter `{}`", i.name, p.name));
        }
        for p in &i.properties.0 {
            out.push(format!("instance `{}`: property `{}`", i.name, p.key));
        }
        if !i.user_data.is_empty() {
            out.push(format!("instance `{}`: user data", i.name));
        }
    }
    out
}

/// What the UFOs of `font` (the sources of a designspace, or one UFO) leave out of the font's and
/// the masters' information, one line each.
pub(crate) fn font_losses(font: &Font, files: FilesFrom) -> Vec<String> {
    // The layers and glyphs as the writer writes them.
    let original = font;
    let expanded = crate::colorglyphs::expand(font);
    let font = expanded.as_ref().unwrap_or(font);
    let designspace = files == FilesFrom::Designspace;
    let mut out: Vec<String> = Vec::new();
    for m in font.masters.iter().filter(|m| m.extra.get(Family::Designspace, "layer").is_none()) {
        let (read, _) = kept_dict(m, FONTINFO);
        let font_info = Info { params: font.info.params.clone(), ..Default::default() };
        let own = Info { params: m.params.clone(), ..Default::default() };
        let name = if m.name.is_empty() { &m.id } else { &m.name };
        for line in info_entries(font, m, &read).1.into_iter().chain(fontinfo::replaced(&font_info, &own, name)) {
            let param = line.split(" (").next().unwrap_or(&line);
            if !out.iter().any(|l| l == &line || (line.contains("replaces it with") && l.split(" (").next() == Some(param))) {
                out.push(line);
            }
        }
    }
    out.extend(glyph_losses(font));
    out.extend(layer_losses(font, files));
    out.extend(crate::colorkeys::losses(font));
    out.extend(crate::colorglyphs::losses(original));
    out.extend(shape_losses(font, designspace));
    out.extend(crate::features::losses(&font.features));
    out.extend(kerning::losses(font));
    out.extend(libkeys::losses(font, designspace));
    // Stems travel as unnamed stem snaps, the horizontal ones first.
    let named = font.stems.iter().filter(|d| !d.name.is_empty() || d.filter.is_some() || !d.extra.is_empty()).count();
    if named > 0 {
        out.push(format!("the font's stem names and filters ({named})"));
    }
    if !font.numbers.is_empty() {
        out.push(format!("the font's numbers ({})", font.numbers.len()));
    }
    // A UFO has a key for the first metric of each standard kind; the overshoots of the others
    // come back as alignment zones.
    let mut seen: Vec<&str> = Vec::new();
    let other = font
        .metrics
        .iter()
        .filter(|d| {
            let first = !seen.contains(&d.kind.as_str());
            seen.push(&d.kind);
            !KIND_ORDER.contains(&d.kind.as_str()) || d.name.is_some() || d.filter.is_some() || !first
        })
        .count();
    if other > 0 {
        out.push(format!("the font's metrics other than the first ascender, cap height, x-height, baseline, descender and italic angle without a name or filter ({other}); their overshoots come back as alignment zones"));
    }
    let italic = font.metrics.iter().position(|d| d.kind == metric::ITALIC_ANGLE);
    for m in font.masters.iter() {
        let name = if m.name.is_empty() { &m.id } else { &m.name };
        if italic.and_then(|i| m.metric_values.get(i)).is_some_and(|v| v.over != 0.0) {
            out.push(format!("master `{name}`: the italic angle's overshoot"));
        }
        // A master stored as a layer of another source's UFO has no font information of its own.
        if m.extra.get(Family::Designspace, "layer").is_some() {
            let parts = [("guides", m.guides.len()), ("alignment zones other than overshoots", m.zones.len()), ("stems", m.stem_values.len())];
            for (what, n) in parts.into_iter().filter(|(_, n)| *n > 0) {
                out.push(format!("master `{name}`: {what} ({n}), as a layer of another UFO"));
            }
        }
    }
    out
}

/// What the UFO and designspace readers hold only in their records of the files as read (the
/// [`Family::Ufo`] and [`Family::Designspace`] entries of [`Extra`]), so that saving the document
/// in another format leaves it out, one line per kind with a count. The other records are file
/// layout (spellings, indentation, file names, element order, which files a UFO had), which the
/// other format does not need.
/// With `ufo_kept` (saving as one UFO), only the designspace document's parts.
pub fn kept_data_losses(font: &Font, ufo_kept: bool) -> Vec<String> {
    let document = font.extra.get(Family::Designspace, "document").and_then(Value::as_str);
    let document = document.map(|t| document_data(t, font.extra.get(Family::Designspace, "rules").is_some())).unwrap_or_default();
    if ufo_kept {
        return document;
    }
    let (mut code_points, mut notes, mut colours, mut elements) = (0, 0, 0, 0);
    for (g, l) in font.glyphs.iter().flat_map(|g| g.layers.iter().map(move |l| (g, l))) {
        let x = &l.extra;
        code_points += usize::from(ufo_get(x, "unicodes").is_some_and(|v| code_points_of(Some(v)) != g.unicodes));
        notes += usize::from(ufo_get(x, "note").and_then(Value::as_str).is_some_and(|n| n != g.info.note));
        colours += usize::from(ufo_get(x, "markColorRead").is_some());
        elements += x.iter().filter(|e| e.family == Family::Ufo && e.key.as_str().starts_with('+')).count();
    }
    let default_id = font.extra.get(Family::Designspace, "defaultSource").and_then(Value::as_str);
    let default = font.masters.iter().find(|m| Some(m.id.as_str()) == default_id).or_else(|| font.masters.first());
    let groups_of = |m: &Master| ufo_get(&m.extra, GROUPS).map(Value::without_spellings);
    let default_groups = default.and_then(groups_of);
    let others = font.masters.iter().filter(|m| default.is_none_or(|d| d.id != m.id));
    let (mut features, mut groups, mut layer_colours) = (0, 0, 0);
    for m in others {
        features += usize::from(ufo_get(&m.extra, FEATURES_KEY).is_some());
        groups += usize::from(groups_of(m).is_some_and(|g| Some(&g) != default_groups.as_ref()));
    }
    for m in font.masters.iter() {
        if let Some(d) = ufo_get(&m.extra, "layerinfo").and_then(Value::as_dict) {
            layer_colours += d.entries.iter().filter(|(_, v)| v.as_dict().is_some_and(|i| i.get("color").is_some())).count();
        }
    }
    let mut out = Vec::new();
    let lines = [
        (code_points, "code points of a master or UFO layer that differ from the glyph's"),
        (notes, "notes of a master or UFO layer that differ from the glyph's"),
        (colours, "mark colours of a master that differ from the glyph's"),
        (elements, "GLIF elements kept as written"),
        (features, "feature code of a master that differs from the default source's"),
        (groups, "groups of a master that differ from the default source's"),
        (layer_colours, "UFO layer colours"),
    ];
    for (n, what) in lines {
        if n > 0 {
            out.push(format!("{what} ({n})"));
        }
    }
    out.extend(document);
    out
}

/// The parts of a kept designspace document no model field holds: rules that are not alternate
/// layers, localised instance names, variable fonts' axis subsets, and other top-level elements.
fn document_data(text: &str, rules_raised: bool) -> Vec<String> {
    const TYPED: [&str; 6] = ["axes", "sources", "instances", "variable-fonts", "lib", "rules"];
    let Ok(doc) = xml::parse(text) else { return Vec::new() };
    let root = &doc.root;
    let mut out = Vec::new();
    if !rules_raised && root.child("rules").is_some_and(|r| r.elements().next().is_some()) {
        out.push("the designspace's rules, which are not alternate layers".to_string());
    }
    let localised =
        root.child("instances").map(|i| i.elements().filter(|e| e.elements().any(|c| c.attr("xml:lang").is_some())).count()).unwrap_or_default();
    if localised > 0 {
        out.push(format!("localised instance names ({localised})"));
    }
    let subsets = root
        .child("variable-fonts")
        .map(|v| v.elements().filter(|e| e.child("axis-subsets").is_some_and(|s| s.elements().next().is_some())).count())
        .unwrap_or_default();
    if subsets > 0 {
        out.push(format!("variable fonts' axis subsets ({subsets})"));
    }
    for e in root.elements().filter(|e| !TYPED.contains(&e.name())) {
        out.push(format!("the designspace's `{}` element", e.name()));
    }
    out
}

/// The glyph metadata a UFO has no place for, not even in glyphsLib's lib keys
/// ([`crate::glyphkeys`]), one line per kind with the number of glyphs that have it.
pub(crate) fn glyph_losses(font: &Font) -> Vec<String> {
    use fontcraft_model::GlyphInfo;
    type Has = fn(&Font, &Glyph, &GlyphInfo) -> bool;
    let kinds: [(&str, Has); 10] = [
        (GLYPH_LOSSES[0], |_, _, i| i.case.is_some() || i.direction.is_some()),
        (GLYPH_LOSSES[1], |_, _, i| i.metrics_keys.all().iter().skip(3).any(|(_, v)| v.is_some())),
        (GLYPH_LOSSES[2], |_, _, i| !i.tags.is_empty()),
        (GLYPH_LOSSES[3], |_, _, i| i.locked),
        (GLYPH_LOSSES[4], |f, g, _| glyphkeys::last_change_lost(f, g)),
        (GLYPH_LOSSES[5], |_, _, i| i.sort_name.is_some() || i.sort_name_keep.is_some()),
        (GLYPH_LOSSES[6], |_, g, _| glyphkeys::parts_lost(g)),
        (GLYPH_LOSSES[7], |_, _, i| i.color.as_ref().is_some_and(|c| glif::mark_color_text(c).is_none())),
        (GLYPH_LOSSES[8], |_, _, i| !glif::note_survives(&i.note)),
        (GLYPH_LOSSES[9], |_, g, _| g.layers.is_empty()),
    ];
    kinds
        .iter()
        .filter_map(|(what, has)| {
            let n = font.glyphs.iter().filter(|g| has(font, g, &g.info)).count();
            (n > 0).then(|| format!("{what} ({n})"))
        })
        .collect()
}

/// The layer data a UFO has no place for, not even in glyphsLib's lib keys, one line per kind
/// with the number of layers that have it. Layers a UFO cannot hold at all ([`layers_left_out`])
/// are not counted here. A designspace (`designspace`) also holds alternate layers, as glyphs of
/// their own, and intermediate layers' coordinates, as sparse sources.
pub(crate) fn layer_losses(font: &Font, files: FilesFrom) -> Vec<String> {
    // Colour layers in the list are measured once for the whole walk.
    let _boxes = crate::colorkeys::Scope::enter();
    let designspace = files == FilesFrom::Designspace;
    use fontcraft_model::{Background, Image, LayerInfo};
    let unfiled = |i: &Option<Image>| i.as_ref().is_some_and(|i| i.path.is_none());
    // A GLIF image names a PNG file in the UFO's `images/` folder. Only a UFO source's files
    // travel, from where the writer finds them ([`FilesFrom`]); other formats are not converted.
    // `None`: the writer finds none of the master's files.
    let ufo_prefix = |master: &str| -> Option<String> {
        if !matches!(font.source.format, SourceFormat::Ufo | SourceFormat::Designspace) {
            return None;
        }
        let file = font.masters.iter().find(|m| m.id == master).and_then(|m| m.extra.get(Family::Designspace, "filename")).and_then(Value::as_str);
        match (files, file) {
            (FilesFrom::Single, _) | (FilesFrom::PerMaster, None) => Some(String::new()),
            (FilesFrom::PerMaster | FilesFrom::Designspace, Some(f)) => Some(format!("{f}/")),
            (FilesFrom::Designspace, None) => None,
        }
    };
    let not_in_images = |i: &Option<Image>, prefix: Option<&str>| {
        i.as_ref().and_then(|i| i.path.as_deref()).is_some_and(|p| {
            let png = std::path::Path::new(p).extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
            !png || prefix.is_none_or(|prefix| !font.files.0.contains_key(&format!("{prefix}images/{p}")))
        })
    };
    let guide_extras = |guides: &[fontcraft_model::Guide]| {
        guides.iter().any(|g| g.filter.is_some() || g.angle_locked || g.show_measurement || g.orientation.is_some())
    };
    let bg = |i: &LayerInfo, f: &dyn Fn(&Background) -> bool| i.background.as_ref().is_some_and(f);
    let ufo2 = font.source.ufo_version == Some(2);
    let mut counts: Vec<(&str, usize)> = LAYER_LOSSES.iter().map(|w| (*w, 0)).collect();
    let layer_master = |id: &str| font.masters.iter().any(|m| m.id == id && m.extra.get(Family::Designspace, "layer").is_some());
    let mut seen: HashSet<(&str, &str, String)> = HashSet::new();
    // Whether every layer of a UFO layer has one id, which its lib then holds.
    let mut ids: HashMap<(&str, String), Option<&str>> = HashMap::new();
    for l in font.glyphs.iter().flat_map(|g| g.layers.iter()).filter(|l| !l.is_master_layer()) {
        if let Some(n) = ufo_layer_name(font, l) {
            let slot = ids.entry((l.master.as_str(), n)).or_insert(Some(l.id.as_str()));
            if *slot != Some(l.id.as_str()) {
                *slot = None;
            }
        }
    }
    let read_back = if designspace { crate::designspace::alternate_ids(font) } else { Vec::new() };
    for (g, l) in font.glyphs.iter().flat_map(|g| g.layers.iter().map(move |l| (g, l))) {
        let alternate = designspace && crate::designspace::alternate_comes_back(font, l);
        let written = if l.is_master_layer() {
            None
        } else if alternate {
            // An alternate layer's name comes back from its bracket glyph's lib; one without a
            // name gets one.
            Some(l.info.name.clone().unwrap_or_default())
        } else {
            ufo_layer_name(font, l)
        };
        if !l.is_master_layer() && (written.is_none() || layer_master(&l.master)) {
            continue;
        }
        // A second layer of one name in a glyph goes to `<name> #<n>`.
        let renamed = !alternate && written.as_ref().is_some_and(|n| !seen.insert((g.name.as_str(), l.master.as_str(), n.clone())));
        let i = &*l.info;
        let mut other_attrs = fontcraft_model::LayerAttrs { color_palette: None, ..i.attrs.clone() };
        if designspace && crate::designspace::intermediate_comes_back(font, l) {
            other_attrs.coordinates.clear();
        }
        if alternate {
            other_attrs.axis_rules.clear();
        }
        let id_lost = if alternate {
            !read_back.iter().any(|(gn, id, back)| *gn == g.name && *id == l.id && *back == l.id)
        } else {
            written.as_deref().is_some_and(|n| {
                l.id != glyphkeys::synthetic_id(&l.master, n)
                    && (renamed || ids.get(&(l.master.as_str(), n.to_string())).copied().flatten() != Some(l.id.as_str()))
            })
        };
        let has = [
            // A master layer's name, an intermediate or colour layer's, which its UFO layer's
            // name replaces (or gives one without a name), and a second one of a name in a glyph.
            if alternate {
                i.name.as_deref().is_none_or(str::is_empty)
            } else if l.is_master_layer() {
                i.name.is_some()
            } else {
                written != i.name || renamed
            },
            !other_attrs.is_empty(),
            i.vert_width == Some(0.0),
            i.vert_origin.is_some() && font.masters.iter().find(|m| m.id == l.master).is_none_or(|m| model_frame(font, m).0.is_none()),
            i.metrics_keys.all().iter().skip(3).any(|(_, v)| v.is_some()),
            i.color.is_some(),
            i.visible,
            glyphkeys::hints_lost(l),
            glyphkeys::annotations_lost(l),
            i.image.as_ref().is_some_and(glyphkeys::image_lost) || bg(i, &|b| b.image.as_ref().is_some_and(glyphkeys::image_lost)),
            unfiled(&i.image) || bg(i, &|b| unfiled(&b.image)),
            guide_extras(&i.guides) || bg(i, &|b| guide_extras(&b.guides)),
            ufo2 && (!i.guides.is_empty() || i.image.is_some() || bg(i, &|b| !b.guides.is_empty() || b.image.is_some())),
            !l.is_master_layer() && id_lost,
            {
                let prefix = ufo_prefix(&l.master);
                not_in_images(&i.image, prefix.as_deref()) || bg(i, &|b| not_in_images(&b.image, prefix.as_deref()))
            },
        ];
        for ((_, n), on) in counts.iter_mut().zip(has) {
            *n += usize::from(on);
        }
    }
    counts.into_iter().filter(|(_, n)| *n > 0).map(|(what, n)| format!("{what} ({n})")).collect()
}

/// The shape data a GLIF file has no place for, not even in glyphsLib's lib keys
/// ([`crate::shapekeys`]), one line per kind with the number of objects that have it, in the
/// layers a UFO holds (a designspace's: alternate layers too).
pub(crate) fn shape_losses(font: &Font, designspace: bool) -> Vec<String> {
    // Colour layers in the list are measured once for the whole walk.
    let _boxes = crate::colorkeys::Scope::enter();
    use fontcraft_model::Shape;
    let mut counts = [0usize; SHAPE_LOSSES.len()];
    let ufo2 = font.source.ufo_version == Some(2);
    let layer_master = |id: &str| font.masters.iter().any(|m| m.id == id && m.extra.get(Family::Designspace, "layer").is_some());
    let layers = font.glyphs.iter().flat_map(|g| g.layers.iter()).filter(|l| {
        l.is_master_layer()
            || (ufo_layer_name(font, l).is_some() && !layer_master(&l.master))
            || designspace && crate::designspace::alternate_comes_back(font, l)
    });
    for l in layers {
        let background = l.info.background.as_ref();
        // A full-colour layer that the colour layer list may split goes through `colorglyphs`,
        // which keeps the fills only; any attribute is counted there.
        let split = crate::colorglyphs::expands(l) || (l.is_master_layer() && l.info.attrs.full_color);
        let shapes = l.shapes.iter().map(|s| (s, false)).chain(background.into_iter().flat_map(|b| b.shapes.iter().map(|s| (s, true))));
        for (s, in_background) in shapes {
            match s {
                Shape::Path(p) => {
                    let any = p.info.locked || !p.info.attrs.is_empty();
                    counts[0] += usize::from(if split && !in_background { any } else { crate::pathkeys::lost(p) });
                    counts[1] += p.nodes.iter().filter(|n| n.locked || !n.orientation.is_left()).count();
                    counts[2] += p.nodes.iter().filter(|n| n.tangent).count();
                    counts[3] += usize::from(!p.closed && p.nodes.first().is_some_and(|n| n.smooth));
                    counts[9] += usize::from(ufo2 && (p.info.identifier.is_some() || p.nodes.iter().any(|n| n.info.identifier.is_some())));
                }
                Shape::Component(c) => {
                    let i = &*c.info;
                    let placed = i.alignment != fontcraft_model::Alignment::Default || i.anchor.is_some() || i.locked || !i.smart_values.is_empty();
                    counts[4] += usize::from(in_background && placed);
                    counts[5] += usize::from(i.master_id.is_some() || !i.orientation.is_left() || !i.attrs.is_empty() || !i.user_data.is_empty());
                    counts[9] += usize::from(ufo2 && i.identifier.is_some());
                }
                Shape::Image(_) => counts[8] += 1,
                Shape::Raw(_) => {}
            }
        }
        for a in l.anchors.iter().chain(background.into_iter().flat_map(|b| b.anchors.iter())) {
            counts[6] += usize::from(a.info.locked || !a.info.orientation.is_left());
            counts[9] += usize::from(ufo2 && (a.info.identifier.is_some() || a.info.color.is_some()));
        }
        counts[7] += usize::from(shapekeys::anchor_data_lost(l, ufo2));
    }
    SHAPE_LOSSES.iter().zip(counts).filter(|(_, n)| *n > 0).map(|(what, n)| format!("{what} ({n})")).collect()
}

/// The kinds of shape data [`shape_losses`] reports, as its lines begin.
pub const SHAPE_LOSSES: [&str; 10] = [
    "the paths' locks and drawing attributes, where neither the path attributes key nor the colour layer list holds them",
    "the nodes' locks and orientations",
    "tangent nodes, written as smooth",
    "smooth first nodes of open paths",
    "the alignment, anchors, locks and smart values of components in backgrounds",
    "the components' source masters, orientations, drawing attributes and user data",
    "the anchors' locks and orientations",
    "the anchors' user data, without an identifier to keep it under",
    "images among the shapes",
    "shape identifiers and anchor colours, which UFO 2 has no place for",
];

/// The kinds of layer data [`layer_losses`] reports, as its lines begin.
pub const LAYER_LOSSES: [&str; 15] = [
    "the layers' names",
    "the layers' attributes other than colour palette entries",
    "the layers' vertical widths of 0",
    "the layers' vertical origins, without an ascender to measure them from",
    "the layers' top, bottom and vertical metrics keys",
    "the layers' colours",
    "the layers' visibility",
    "the backgrounds' hints, and hint settings and keys glyphsLib's form has no place for",
    "the backgrounds' annotations, and annotation keys glyphsLib's form has no place for",
    "the layers' image crops and opacity stated as their default",
    "the layers' images without a file",
    "the layers' guide filters, angle locks, measurements and label positions",
    "the layers' guides and images, which UFO 2 has no place for",
    "the ids of layers other than master layers",
    "the layers' image files, which are not PNG files in the UFO's images folder",
];

/// The kinds of glyph metadata [`glyph_losses`] reports, as its lines begin.
pub const GLYPH_LOSSES: [&str; 10] = [
    "the glyphs' case and writing direction",
    "the glyphs' top, bottom and vertical metrics keys",
    "the glyphs' tags",
    "the glyphs' locks",
    "the glyphs' last-change dates in a time zone other than UTC, or switched off",
    "the glyphs' sort names",
    "the glyphs' smart-glyph properties with keys a UFO has no place for, or end names stated empty",
    "the glyphs' colour labels that have no UFO mark colour",
    "the glyphs' notes, with spaces at line ends and blank lines left out",
    "glyphs without layers, which a UFO has no file for",
];

/// What one glyph puts into a UFO layer: a layer, or a layer's background.
enum Part<'a> {
    Layer(&'a Layer),
    Background(&'a fontcraft_model::Background),
}

impl<'a> Part<'a> {
    fn layer(&self, master: &str) -> Cow<'a, Layer> {
        match self {
            Part::Layer(l) => Cow::Borrowed(*l),
            Part::Background(b) => Cow::Owned(background_layer(b, master)),
        }
    }
}

/// UFO layers by name, each with what the glyphs put into it.
type Plan<'a> = Vec<(String, Vec<(&'a Glyph, Part<'a>)>)>;

/// The UFO layers of `target`'s UFO other than the default one, in the order glyphsLib makes
/// them (`to_ufo_layers`): first the master layers' backgrounds (`public.background`) and the
/// layer masters' layers, then the other layers of the master glyph by glyph, each followed by
/// its background (`<UFO layer>.background`). A glyph that already has a layer of that name
/// gets the next free `<name> #<n>`, as in glyphsLib's `to_ufo_layer`.
fn layer_plan<'a>(font: &'a Font, target: &Target<'a>) -> Plan<'a> {
    let mut plan: Plan<'a> = Vec::new();
    let master = target.master;
    for g in font.glyphs.iter() {
        if let Some(l) = g.layer(&master.id)
            && let Some(b) = l.info.background.as_ref()
        {
            // A glyph made of an alternate layer has that layer's background UFO layer.
            let name = l.extra.get(Family::Designspace, BACKGROUND_LAYER).and_then(Value::as_str).unwrap_or(BACKGROUND);
            plan_add(&mut plan, name.to_string(), g, Part::Background(b), false);
        }
        for (m, name) in &target.layer_masters {
            if let Some(l) = g.layers.iter().find(|l| l.id == m.id && l.master == m.id) {
                plan_add(&mut plan, name.clone(), g, Part::Layer(l), false);
                if let Some(b) = &l.info.background {
                    plan_add(&mut plan, format!("{name}{BACKGROUND_SUFFIX}"), g, Part::Background(b), false);
                }
            }
        }
    }
    for g in font.glyphs.iter() {
        for l in g.layers.iter().filter(|l| l.master == master.id && !l.is_master_layer()) {
            let Some(name) = ufo_layer_name(font, l) else { continue };
            let Some(given) = plan_add(&mut plan, name, g, Part::Layer(l), true) else { continue };
            if let Some(b) = &l.info.background {
                plan_add(&mut plan, format!("{given}{BACKGROUND_SUFFIX}"), g, Part::Background(b), false);
            }
        }
    }
    plan
}

/// Adds a glyph's part to the UFO layer `name`, or, when the glyph already has one there, to the
/// next free `<name> #<n>` (`dedupe`) or nowhere. The name it went to.
fn plan_add<'a>(plan: &mut Plan<'a>, name: String, g: &'a Glyph, part: Part<'a>, dedupe: bool) -> Option<String> {
    let has = |plan: &Plan<'a>, n: &str| plan.iter().any(|(p, items)| p == n && items.iter().any(|(h, _)| std::ptr::eq(*h, g)));
    let mut name = name;
    if has(plan, &name) {
        if !dedupe {
            return None;
        }
        let base = name.clone();
        let mut n = 1;
        while plan.iter().any(|(p, _)| *p == name) {
            name = format!("{base} #{n}");
            n += 1;
        }
    }
    match plan.iter_mut().find(|(p, _)| *p == name) {
        Some((_, items)) => items.push((g, part)),
        None => plan.push((name.clone(), vec![(g, part)])),
    }
    Some(name)
}

/// The UFO layer glyphsLib writes a layer other than a master layer to (`to_ufo_layer`): a
/// colour palette layer to `color.<index>` (65535 for the text colour), an intermediate layer
/// of a version 3 or 4 document to its coordinates in braces (`{550, 100}`; version 2's name
/// already says them), any other to its name. `None` for an alternate layer (glyphsLib makes
/// those separate glyphs with designspace rules), for a layer without a name, which a UFO
/// cannot hold, and for a colour layer the lib's font-wide list holds ([`crate::colorkeys`]).
pub fn ufo_layer_name(font: &Font, l: &Layer) -> Option<String> {
    use fontcraft_model::PaletteEntry;
    let a = &l.info.attrs;
    if !a.axis_rules.is_empty() || crate::colorkeys::listed(font, l).is_some() || crate::colorglyphs::expands(l) {
        return None;
    }
    if let Some(p) = a.color_palette {
        let index = match p {
            PaletteEntry::Index(i) => i,
            PaletteEntry::Foreground => u16::MAX,
        };
        return Some(format!("color.{index}"));
    }
    if !a.coordinates.is_empty() && font.source.glyphs_version != Some(2) {
        // A document read from a UFO or designspace keeps a name that states the coordinates
        // (a version 2 source's, such as `Bold {600}`).
        if font.source.glyphs_version.is_none()
            && let Some(n) = l.info.name.as_deref()
            && braced_coordinates(n).is_some_and(|c| c.iter().map(|v| Some(*v)).eq(a.coordinates.iter().copied()))
        {
            return Some(n.to_string());
        }
        let master = font.masters.iter().find(|m| m.id == l.master);
        let parts = a
            .coordinates
            .iter()
            .enumerate()
            .map(|(i, c)| c.or_else(|| master.and_then(|m| m.location.get(i).copied())).map(|v| glif::fmt_number(v, false)))
            .collect::<Option<Vec<_>>>()?;
        return Some(format!("{{{}}}", parts.join(", ")));
    }
    l.info.name.clone().filter(|n| !n.is_empty())
}

/// The coordinates a layer name states between its first `{` and the `}` after it, as glyphsLib
/// reads a version 2 name (`Bold {600, 100}`).
pub(crate) fn braced_coordinates(name: &str) -> Option<Vec<f64>> {
    let open = name.find('{')?;
    let rest = name.get(open + 1..)?;
    let inner = rest.get(..rest.find('}')?)?;
    let v: Vec<f64> = inner.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|f| f.is_finite())).collect::<Option<_>>()?;
    (v.len() <= fontcraft_model::MAX_AXIS_VALUES).then_some(v)
}

/// How many layers other than master layers the files cannot hold: layers without a name, the
/// other layers of masters stored as a layer of another source's UFO, and alternate layers. A
/// designspace (`designspace`) holds alternate layers, as glyphs of their own and rules, in a
/// font with axes.
pub fn layers_left_out(font: &Font, designspace: bool) -> usize {
    let expanded = crate::colorglyphs::expand(font);
    let font = expanded.as_ref().unwrap_or(font);
    // Colour layers in the list are measured once for the whole walk.
    let _boxes = crate::colorkeys::Scope::enter();
    let layer_master = |id: &str| font.masters.iter().any(|m| m.id == id && m.extra.get(Family::Designspace, "layer").is_some());
    font.glyphs
        .iter()
        .flat_map(|g| g.layers.iter())
        .filter(|l| {
            !l.is_master_layer()
                && (ufo_layer_name(font, l).is_none() || layer_master(&l.master))
                && !(designspace && crate::designspace::alternate_comes_back(font, l))
                && crate::colorkeys::listed(font, l).is_none()
        })
        .count()
}

fn code_points_of(v: Option<&Value>) -> Vec<u32> {
    v.and_then(Value::as_array)
        .map(|a| a.items.iter().filter_map(|v| v.as_num().and_then(int_of)).filter_map(|i| u32::try_from(i).ok()).collect())
        .unwrap_or_default()
}

/// The renames ufoLib makes reading the UFO 2 a master was read from ([`kerning::ufo2_renames`]),
/// worked out from its kept files.
fn kept_renames(font: &Font, master: &Master, version: u32) -> kerning::Renames {
    if version != 2 {
        return kerning::Renames::default();
    }
    let (groups, had) = kept_dict(master, GROUPS);
    if !had {
        return kerning::Renames::default();
    }
    let (kept_kerning, _) = kept_dict(master, KERNING);
    let names: HashSet<&str> =
        font.glyphs.iter().filter(|g| g.layers.iter().any(|l| l.master == master.id && l.is_master_layer())).map(|g| g.name.as_str()).collect();
    kerning::ufo2_renames(&groups, Some(&kept_kerning), &names)
}

/// A kerning group's name in the file: in a UFO 2 its old name, or `@MMK_L_`/`@MMK_R_` and its
/// name for a new group.
fn file_group_name(name: &str, version: u32, renames: &kerning::Renames) -> String {
    if version != 2 {
        return name.to_string();
    }
    if let Some(old) = renames.old_name(name) {
        return old.to_string();
    }
    match (name.strip_prefix(KERN1), name.strip_prefix(KERN2)) {
        (Some(g), _) => format!("@MMK_L_{g}"),
        (_, Some(g)) => format!("@MMK_R_{g}"),
        _ => name.to_string(),
    }
}

/// `groups.plist`: the kept groups, with kerning groups made from the glyphs' kerning groups and
/// the other groups from the font's groups. A kept group that is not a list of names stays.
fn write_groups(font: &Font, master: &Master, version: u32) -> Dict {
    let (mut groups, _) = kept_dict(master, GROUPS);
    let renames = kept_renames(font, master, version);
    let mut made: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for g in font.glyphs.iter() {
        if let Some(r) = &g.kern_groups.right {
            made.entry(file_group_name(&format!("{KERN1}{r}"), version, &renames)).or_default().push(&g.name);
        }
        if let Some(l) = &g.kern_groups.left {
            made.entry(file_group_name(&format!("{KERN2}{l}"), version, &renames)).or_default().push(&g.name);
        }
    }
    // Kerning groups that no glyph is in any more go, unless they were empty already; other groups
    // go when the font no longer has them.
    groups.entries.retain(|(k, v)| {
        let k = k.as_str();
        let kern = k.starts_with(KERN1) || k.starts_with(KERN2) || renames.is_old(k);
        if kern { made.contains_key(k) || strings(Some(v)).is_empty() } else { !kerning::is_name_list(v) || font.groups.iter().any(|g| g.name == k) }
    });
    for g in font.groups.iter() {
        let value = Value::Array(Array { items: g.members.iter().map(|m| str_value(m)).collect(), ..Default::default() });
        patch(&mut groups, &g.name, Some(value));
    }
    for (name, members) in made {
        let kept = groups.get(&name);
        let kept_members = strings(kept);
        // Kept order for the members that stay; new members after them, in glyph order.
        let mut ordered: Vec<&str> = kept_members.iter().map(String::as_str).filter(|m| members.contains(m) || font.glyph(m).is_none()).collect();
        for m in &members {
            if !ordered.contains(m) {
                ordered.push(m);
            }
        }
        let value = Value::Array(Array { items: ordered.iter().map(|m| str_value(m)).collect(), ..Default::default() });
        patch(&mut groups, &name, Some(value));
    }
    groups
}

/// `kerning.plist`: the master's left-to-right pairs, keeping the kept values' spellings.
fn write_kerning(font: &Font, master: &Master, version: u32) -> Dict {
    let (kept, _) = kept_dict(master, KERNING);
    let mut out = Dict { layout: kept.layout, ..Default::default() };
    let Some(mk) = font.kerning.ltr.0.iter().find(|m| m.master == master.id) else {
        // No pairs: the kept file, if it had none either, with its whitespace.
        return if kept.is_empty() { kept } else { out };
    };
    // Group sides read from the reference's format get the UFO prefixes. A UFO's own sides stay
    // as they are: older UFO 3 files name kerning groups `@MMK_L_…` themselves. A UFO 2 gets its
    // groups' old names back.
    let from_ufo = ufo_get(&master.extra, METAINFO).is_some();
    let renames = kept_renames(font, master, version);
    let side = |s: &str| {
        let s = if from_ufo { Cow::Borrowed(s) } else { fontcraft_model::ufo_side(s) };
        if (s.starts_with(KERN1) || s.starts_with(KERN2)) && version == 2 { file_group_name(&s, version, &renames) } else { s.into_owned() }
    };
    for f in &mk.firsts {
        let first = side(&f.first);
        let kept_first = kept.get(&first).and_then(Value::as_dict);
        let mut seconds = Dict::new();
        for (second, v) in &f.seconds {
            let second = side(second);
            let value = match kept_first.and_then(|d| d.get(&second)) {
                Some(old) if as_f64(old) == Some(*v) => old.clone(),
                _ => num_value(*v),
            };
            seconds.entries.push((Str::new(second.as_str()), value));
        }
        out.entries.push((Str::new(first.as_str()), Value::Dict(seconds)));
    }
    // Unchanged pairs, in whatever order: the kept file, with its whitespace.
    let same = |a: &Dict, b: &Dict| a.len() == b.len() && a.entries.iter().all(|(k, v)| b.get(k.as_str()) == Some(v));
    let unchanged = out.len() == kept.len()
        && out.entries.iter().all(|(k, v)| match (v.as_dict(), kept.get(k.as_str()).and_then(Value::as_dict)) {
            (Some(x), Some(y)) => same(x, y),
            _ => false,
        });
    if unchanged { kept } else { out }
}

/// Writes one master of a document as a UFO of its own, with the document's font-wide values.
pub fn write_one(font: &Font, master_id: &str) -> Result<Files> {
    let master = font.master(master_id)?;
    // A master read from a designspace keeps its UFO's other files under the UFO's name.
    let prefix = master.extra.get(Family::Designspace, "filename").and_then(Value::as_str).map(|f| format!("{f}/")).unwrap_or_default();
    write_master(font, &Target { master, default: master, layer_masters: Vec::new(), prefix, designspace: false })
}

/// Writes a document as a single UFO; it must have exactly one master.
pub fn write(font: &Font) -> Result<Files> {
    let [master] = font.masters.as_slice() else {
        return Err(UfoError::Unsupported(format!("a document with {} masters as a single UFO: save it as a designspace", font.masters.len())));
    };
    write_master(font, &Target { master, default: master, layer_masters: Vec::new(), prefix: String::new(), designspace: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_model::{Component, Node, NodeKind, Path, Shape, Transform};

    fn s(b: &[u8]) -> &str {
        std::str::from_utf8(b).unwrap()
    }

    const PLIST_HEAD: &str = "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n";

    /// A UFO of `Font::new` with these `groups.plist` and `kerning.plist` dictionaries, in the
    /// given format version.
    fn ufo_with(groups: &str, kerning: &str, version: u32) -> Files {
        let mut files = write(&Font::new("Groups")).unwrap();
        files.insert(GROUPS.into(), format!("{PLIST_HEAD}  <dict>\n{groups}  </dict>\n</plist>\n").into_bytes());
        files.insert(KERNING.into(), format!("{PLIST_HEAD}  <dict>\n{kerning}  </dict>\n</plist>\n").into_bytes());
        if version == 2 {
            let meta = s(&files[METAINFO]).replace("<integer>3</integer>", "<integer>2</integer>");
            files.insert(METAINFO.into(), meta.into_bytes());
            files.remove(LAYERCONTENTS);
        }
        files
    }

    fn list(names: &[&str]) -> String {
        let items: String = names.iter().map(|n| format!("      <string>{n}</string>\n")).collect();
        format!("    <array>\n{items}    </array>\n")
    }

    #[test]
    fn a_format_2_ufo_written_as_format_3_lists_its_layers() {
        let mut font = read(&ufo_with("", "", 2)).unwrap();
        assert!(!write(&font).unwrap().contains_key(LAYERCONTENTS), "format 2 stays without a layer list");
        font.source.ufo_version = Some(3);
        let files = write(&font).unwrap();
        assert!(s(&files[METAINFO]).contains("<integer>3</integer>"));
        assert!(s(&files[LAYERCONTENTS]).contains("<string>glyphs</string>"), "format 3 requires layercontents.plist");
    }

    #[test]
    fn groups_that_are_not_kerning_groups_are_the_fonts_groups() {
        let groups = format!(
            "    <key>Upper</key>\n{}    <key>odd</key>\n    <string>not a list</string>\n    <key>public.kern1.A</key>\n{}    <key>vowels</key>\n{}",
            list(&["A", "E", "missing"]),
            list(&["A"]),
            list(&["a", "e"])
        );
        let files = ufo_with(&groups, "", 3);
        let mut font = read(&files).unwrap();
        let names: Vec<(&str, usize)> = font.groups.iter().map(|g| (g.name.as_str(), g.members.len())).collect();
        assert_eq!(names, [("Upper", 3), ("vowels", 2)], "kerning groups and values that are not name lists are not groups");
        assert_eq!(font.glyph("A").unwrap().kern_groups.right.as_deref(), Some("A"));
        assert_eq!(write(&font).unwrap(), files);
        // Edits: a member, a removed group and a new one.
        let g = Arc::make_mut(&mut font.groups);
        g[0].members.pop();
        g.remove(1);
        g.push(Group { name: "Zed".into(), members: vec!["Z".into()] });
        let out = write(&font).unwrap();
        let text = s(&out[GROUPS]);
        assert!(text.contains(&format!("<key>Upper</key>\n{}", list(&["A", "E"]))), "{text}");
        assert!(!text.contains("vowels") && text.contains("<key>odd</key>") && text.contains("<key>public.kern1.A</key>"), "{text}");
        assert!(text.contains(&format!("<key>Zed</key>\n{}    <key>odd</key>", list(&["Z"]))), "a new group in sorted order: {text}");
        assert_eq!(read(&out).unwrap().groups, font.groups);
    }

    #[test]
    fn groups_go_to_groups_plist_not_to_the_feature_code() {
        let mut font = Font::new("Groups");
        Arc::make_mut(&mut font.groups).push(Group { name: "round".into(), members: vec!["O".into(), "Q".into()] });
        Arc::make_mut(&mut font.features).classes.push(fontcraft_model::Snippet { name: "Upper".into(), code: "A B".into(), ..Default::default() });
        let files = write(&font).unwrap();
        assert!(s(&files[GROUPS]).contains(&format!("<key>round</key>\n{}", list(&["O", "Q"]))));
        assert_eq!(s(&files[FEATURES]), "@Upper = [ A B\n];\n");
        let back = read(&files).unwrap();
        assert_eq!((back.groups.as_slice(), back.features.classes.len()), (font.groups.as_slice(), 1));
    }

    #[test]
    fn ufo2_kerning_groups_get_ufo3_names_and_keep_their_own_in_the_file() {
        let groups =
            format!("    <key>@MMK_L_A</key>\n{}    <key>caps</key>\n{}    <key>round</key>\n{}", list(&["A"]), list(&["A", "B"]), list(&["O", "Q"]));
        let kerning = "    <key>@MMK_L_A</key>\n    <dict>\n      <key>round</key>\n      <integer>-20</integer>\n    </dict>\n";
        let files = ufo_with(&groups, kerning, 2);
        let mut font = read(&files).unwrap();
        assert_eq!(font.glyph("A").unwrap().kern_groups.right.as_deref(), Some("A"));
        assert_eq!(font.glyph("Q").unwrap().kern_groups.left.as_deref(), Some("round"));
        assert_eq!(font.kerning.ltr.get("m01", "public.kern1.A", "public.kern2.round"), Some(-20.0));
        assert_eq!(font.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["caps"], "only the group no kerning uses");
        assert_eq!(write(&font).unwrap(), files, "the old names are written back");
        // A new kerning group gets the old prefixes; existing ones keep their names.
        font.glyph_mut("V").unwrap().kern_groups.left = Some("V".into());
        Arc::make_mut(&mut font.kerning).ltr.set("m01", "public.kern1.A", "public.kern2.V", -40.0);
        let out = write(&font).unwrap();
        assert!(s(&out[GROUPS]).contains(&format!("<key>@MMK_R_V</key>\n{}", list(&["V"]))), "{}", s(&out[GROUPS]));
        let k = s(&out[KERNING]);
        assert!(k.contains("<key>@MMK_L_A</key>") && k.contains("<key>@MMK_R_V</key>") && k.contains("<key>round</key>"), "{k}");
        assert_eq!(read(&out).unwrap().kerning, font.kerning);
    }

    #[test]
    fn feature_markers_are_read_into_snippets_and_written_back() {
        let fea = "# Prefix: Languagesystems\n# automatic\nlanguagesystem DFLT dflt;\n\n\nfeature ccmp {\n# automatic\nsub a by b;\n\n} ccmp;\n";
        let mut files = write(&Font::new("Fea")).unwrap();
        files.insert(FEATURES.into(), fea.as_bytes().to_vec());
        let mut font = read(&files).unwrap();
        let f = &font.features;
        assert_eq!(
            (f.prefixes[0].name.as_str(), f.prefixes[0].code.as_str(), f.prefixes[0].automatic),
            ("Languagesystems", "languagesystem DFLT dflt;\n", true)
        );
        assert_eq!((f.features[0].name.as_str(), f.features[0].code.as_str(), f.features[0].automatic), ("ccmp", "sub a by b;\n", true));
        assert_eq!(write(&font).unwrap(), files);
        Arc::make_mut(&mut font.features).features[0].code = "sub a by c;".into();
        assert!(s(&write(&font).unwrap()[FEATURES]).contains("feature ccmp {\n# automatic\nsub a by c;\n} ccmp;\n"));
    }

    #[test]
    fn parameters_that_come_back_otherwise_and_layerless_glyphs_are_reported() {
        let mut font = Font::new("Lost");
        let params = &mut Arc::make_mut(&mut font.info).params.0;
        params.push(Param::new("postscriptIsFixedPitch", Value::Number(Number::int(0))));
        params.push(Param::new("openTypeOS2Panose", Value::Array(Array { items: vec![Value::Number(Number::int(2)); 10], ..Default::default() })));
        let panose = Param::new("panose", Value::Array(Array { items: vec![Value::Number(Number::int(3)); 10], ..Default::default() }));
        Arc::make_mut(&mut font.masters)[0].params.0.push(panose);
        font.glyphs.push(Glyph::new("empty"));
        let mut d = MetricDef::new(metric::X_HEIGHT);
        d.filter = Some("case == 3".into());
        Arc::make_mut(&mut font.metrics).push(d);
        let losses = info_losses(&font);
        for line in [
            "parameter `postscriptIsFixedPitch` (comes back as `isFixedPitch`)",
            "parameter `openTypeOS2Panose` (master `Regular` replaces it with `panose`, the same UFO key)",
            "glyphs without layers, which a UFO has no file for (1)",
            "the font's metrics other than the first ascender, cap height, x-height, baseline, descender and italic angle without a name or filter (1); their overshoots come back as alignment zones",
        ] {
            assert!(losses.iter().any(|l| l == line), "{line}: {losses:?}");
        }
    }

    #[test]
    fn kerning_a_ufo_cannot_hold_is_reported() {
        let mut font = Font::new("Lost");
        let k = Arc::make_mut(&mut font.kerning);
        k.rtl.set("m01", "A", "B", -10.0);
        k.vertical.set("m01", "A", "B", -5.0);
        k.context.push(fontcraft_model::KernContext { context: "A | B".into(), values: vec![("m01".into(), 3.0)] });
        font.glyph_mut("A").unwrap().kern_groups.top = Some("top".into());
        let losses = info_losses(&font);
        for line in [
            "right-to-left kerning pairs (1)",
            "vertical kerning pairs (1)",
            "contextual kerning (1)",
            "the glyphs' top and bottom kerning groups (1)",
        ] {
            assert!(losses.iter().any(|l| l == line), "{line}: {losses:?}");
        }
    }

    #[test]
    fn a_new_font_is_written_as_fonttools_writes_it() {
        let mut font = Font::new("Test Sans");
        font.set_metric_pos("m01", metric::ITALIC_ANGLE, 12.0).unwrap();
        {
            let mut a = font.glyph_mut("A").unwrap();
            let layer = a.layer_mut("m01").unwrap();
            layer.shapes.push(Shape::Path(Path::new(
                true,
                vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(300.5, 700.0, NodeKind::Line), Node::new(600.0, 0.0, NodeKind::Line)],
            )));
            a.kern_groups.right = Some("A".into());
        }
        font.glyph_mut("B").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(Component::new("A", 10.0, 0.0)));
        font.glyph_mut("C").unwrap().export = false;
        let files = write(&font).unwrap();
        assert_eq!(
            s(&files["metainfo.plist"]),
            "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n  <dict>\n    <key>creator</key>\n    <string>org.fontcraft</string>\n    <key>formatVersion</key>\n    <integer>3</integer>\n  </dict>\n</plist>\n"
        );
        let info = s(&files["fontinfo.plist"]);
        assert!(info.contains("<key>italicAngle</key>\n    <integer>-12</integer>"), "{info}");
        assert!(info.contains("<key>familyName</key>\n    <string>Test Sans</string>"));
        let glif = s(&files["glyphs/A_.glif"]);
        assert!(glif.contains("<point x=\"300.5\" y=\"700\" type=\"line\"/>"), "{glif}");
        assert!(s(&files["glyphs/B_.glif"]).contains("<component base=\"A\" xOffset=\"10\"/>"));
        assert!(s(&files["groups.plist"]).contains("<key>public.kern1.A</key>"));
        assert!(s(&files["lib.plist"]).contains("<key>public.skipExportGlyphs</key>\n    <array>\n      <string>C</string>"));
        assert!(s(&files["layercontents.plist"]).contains("<string>public.default</string>\n      <string>glyphs</string>"));
        assert!(s(&files["glyphs/contents.plist"]).contains("<key>.notdef</key>\n    <string>_notdef.glif</string>"));

        let back = read(&files).unwrap();
        assert_eq!(back.family_name, "Test Sans");
        assert_eq!(back.glyphs.len(), font.glyphs.len());
        assert_eq!(back.metric_pos(back.default_master().unwrap(), metric::ITALIC_ANGLE, 0.0), 12.0);
        assert_eq!(back.glyph("A").unwrap().kern_groups.right.as_deref(), Some("A"));
        assert!(!back.glyph("C").unwrap().export);
        assert_eq!(back.outline("B", "m01").unwrap().elements().len(), font.outline("B", "m01").unwrap().elements().len());
        assert_eq!(write(&back).unwrap(), files, "a UFO we wrote comes back unchanged");
    }

    #[test]
    fn a_font_from_another_format_gets_opentype_categories_and_a_ufo_keeps_its_own() {
        use fontcraft_model::Anchor;
        let mut font = Font::new("Classes");
        let m = font.masters[0].id.clone();
        font.add_glyph("acutecomb", vec![0x301], 0.0).unwrap();
        font.add_glyph("f_f", vec![], 600.0).unwrap();
        font.add_glyph("Aacute", vec![0xC1], 600.0).unwrap();
        font.glyph_mut("A").unwrap().layers[0].anchors.push(Anchor::new("top", 300.0, 700.0));
        font.glyph_mut("acutecomb").unwrap().layers[0].anchors.push(Anchor::new("_top", 0.0, 500.0));
        font.glyph_mut("f_f").unwrap().layers[0].anchors.push(Anchor::new("top_1", 100.0, 700.0));
        // A composite without anchors of its own takes its components'.
        let mut aacute = font.glyph_mut("Aacute").unwrap();
        aacute.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        aacute.layers[0].shapes.push(Shape::Component(Component::new("acutecomb", 300.0, 200.0)));
        drop(aacute);
        // A stored class wins.
        font.glyph_mut("B").unwrap().info.opentype_category = Some("component".into());
        assert_eq!(font.glyph("A").unwrap().layer(&m).map(|l| l.anchors.len()), Some(1));
        let lib = s(&write(&font).unwrap()["lib.plist"]).to_string();
        let classes = "<key>public.openTypeCategories</key>\n    <dict>\n      <key>A</key>\n      <string>base</string>\n      <key>Aacute</key>\n      <string>base</string>\n      <key>B</key>\n      <string>component</string>\n      <key>acutecomb</key>\n      <string>mark</string>\n      <key>f_f</key>\n      <string>ligature</string>\n    </dict>";
        assert!(lib.contains(classes), "{lib}");
        // Read back, only the class the rules do not give is stored; written again, the map is
        // the same.
        let files = write(&font).unwrap();
        let back = read(&files).unwrap();
        let stored: Vec<(&str, &str)> = back.glyphs.iter().filter_map(|g| Some((g.name.as_str(), g.info.opentype_category.as_deref()?))).collect();
        assert_eq!(stored, [("B", "component")]);
        assert!(s(&write(&back).unwrap()["lib.plist"]).contains(classes));
        // The compiler still sees every class the map states.
        let data = fontcraft_model::glyphdata::GlyphData::new().unwrap();
        let rules = fontcraft_model::ufo_classes::rule_classes(&back, &m, &data);
        let mut effective: Vec<(String, String)> = fontcraft_model::ufo_classes::stated(&back, &m, &rules).into_iter().collect();
        effective.sort();
        let want = [("A", "base"), ("Aacute", "base"), ("B", "component"), ("acutecomb", "mark"), ("f_f", "ligature")];
        assert_eq!(effective, want.map(|(g, c)| (g.to_string(), c.to_string())));
        font.source.format = SourceFormat::Ufo;
        let lib = s(&write(&font).unwrap()["lib.plist"]).to_string();
        assert!(
            lib.contains("<key>public.openTypeCategories</key>\n    <dict>\n      <key>B</key>\n      <string>component</string>\n    </dict>"),
            "{lib}"
        );
    }

    #[test]
    fn glyph_notes_colours_production_names_and_categories_travel() {
        use fontcraft_model::ColorLabel;
        let mut font = Font::new("Meta");
        {
            let mut a = font.glyph_mut("A").unwrap();
            a.info.note = "first line\nsecond".into();
            a.info.color = Some(ColorLabel::Index(4));
            a.info.production = Some("uni0041".into());
            a.info.opentype_category = Some("base".into());
        }
        {
            let mut b = font.glyph_mut("B").unwrap();
            b.info.color = Some(ColorLabel::Rgba([1.0, 128.0 / 255.0, 0.0, 1.0]));
            b.info.production = Some("B".into());
            b.info.category = Some("Letter".into());
        }
        let files = write(&font).unwrap();
        let a = s(&files["glyphs/A_.glif"]);
        assert!(a.contains("  <note>\nfirst line\nsecond\n</note>\n"), "{a}");
        assert!(a.contains("<key>public.markColor</key>\n      <string>0.67,0.95,0.38,1</string>"), "{a}");
        assert!(s(&files["glyphs/B_.glif"]).contains("<string>1,0.502,0,1</string>"), "8-bit components as glyphsLib writes them");
        let lib = s(&files["lib.plist"]);
        assert!(
            lib.contains("<key>public.openTypeCategories</key>\n    <dict>\n      <key>A</key>\n      <string>base</string>\n    </dict>"),
            "{lib}"
        );
        assert!(lib.contains("<key>public.postscriptNames</key>\n    <dict>\n      <key>A</key>\n      <string>uni0041</string>\n      <key>B</key>\n      <string>B</string>"), "{lib}");
        assert!(info_losses(&font).is_empty(), "{:?}", info_losses(&font));
        assert!(s(&files["glyphs/B_.glif"]).contains("<key>com.schriftgestaltung.Glyphs.category</key>"), "the category goes to glyphsLib's key");

        let back = read(&files).unwrap();
        let (a, b) = (back.glyph("A").unwrap(), back.glyph("B").unwrap());
        assert_eq!(a.info.note, "first line\nsecond");
        assert_eq!(a.info.color, Some(ColorLabel::Index(4)));
        assert_eq!((a.info.production.as_deref(), a.info.opentype_category.as_deref()), (Some("uni0041"), Some("base")));
        assert_eq!(b.info.category.as_deref(), Some("Letter"));
        assert_eq!(b.info.color, Some(ColorLabel::Rgba([1.0, 0.502, 0.0, 1.0])));
        assert!(back.default_master().unwrap().user_data.is_empty(), "the lib keys are glyph fields, not user data");
        assert_eq!(write(&back).unwrap(), files, "written back unchanged");

        // Edits are written; a removed colour leaves the lib.
        let mut edited = back.clone();
        {
            let mut a = edited.glyph_mut("A").unwrap();
            a.info.note = "changed".into();
            a.info.color = None;
            a.info.production = None;
        }
        let out = write(&edited).unwrap();
        let a = s(&out["glyphs/A_.glif"]);
        assert!(a.contains("<note>\nchanged\n</note>") && !a.contains("markColor"), "{a}");
        assert!(!s(&out["lib.plist"]).contains("uni0041"));
    }

    #[test]
    fn notes_and_mark_colours_keep_their_spelling() {
        let mut files = write(&Font::new("Spell")).unwrap();
        let glif = s(&files["glyphs/a.glif"]).replace(
            "  <advance",
            "  <note>\n    Indented\n\n    note\n  </note>\n  <lib>\n    <dict>\n      <key>public.markColor</key>\n      <string>1, 0, 0, 1</string>\n      <key>z</key>\n      <string>1</string>\n    </dict>\n  </lib>\n  <advance",
        );
        files.insert("glyphs/a.glif".into(), glif.clone().into_bytes());
        let font = read(&files).unwrap();
        let a = font.glyph("a").unwrap();
        assert_eq!(a.info.note, "Indented\nnote", "read as fontTools reads it");
        assert_eq!(a.info.color, Some(fontcraft_model::ColorLabel::Rgba([1.0, 0.0, 0.0, 1.0])));
        assert_eq!(s(&write(&font).unwrap()["glyphs/a.glif"]), glif);
        // A production name map with entries for glyphs the font lacks stays as written.
        let lib = s(&files["lib.plist"]).replace(
            "<dict>\n",
            "<dict>\n    <key>public.postscriptNames</key>\n    <dict>\n      <key>zz</key>\n      <string>uniFFFF</string>\n      <key>a</key>\n      <string>a</string>\n    </dict>\n",
        );
        files.insert("lib.plist".into(), lib.clone().into_bytes());
        let font = read(&files).unwrap();
        assert_eq!(font.glyph("a").unwrap().info.production.as_deref(), Some("a"));
        assert_eq!(s(&write(&font).unwrap()["lib.plist"]), lib);
    }

    #[test]
    fn an_edit_changes_only_its_files() {
        let font = Font::new("Edit");
        let files = write(&font).unwrap();
        let mut font = read(&files).unwrap();
        font.glyph_mut("a").unwrap().layer_mut("m01").unwrap().width = 512.0;
        font.family_name = "Edited".into();
        let out = write(&font).unwrap();
        let changed: Vec<&str> = out.iter().filter(|(k, v)| files.get(*k) != Some(*v)).map(|(k, _)| k.as_str()).collect();
        assert_eq!(changed, ["fontinfo.plist", "glyphs/a.glif"]);
        assert!(s(&out["glyphs/a.glif"]).contains("<advance width=\"512\"/>"));
    }

    #[test]
    fn kept_spellings_survive_and_changed_values_do_not() {
        let mut files = write(&Font::new("Spell")).unwrap();
        let info = s(&files["fontinfo.plist"]).replace("<integer>800</integer>", "<real>800.0</real>");
        files.insert("fontinfo.plist".into(), info.clone().into_bytes());
        let mut font = read(&files).unwrap();
        assert_eq!(s(&write(&font).unwrap()["fontinfo.plist"]), info);
        font.set_metric_pos("m01", metric::ASCENDER, 810.0).unwrap();
        assert!(s(&write(&font).unwrap()["fontinfo.plist"]).contains("<key>ascender</key>\n    <integer>810</integer>"));
    }

    #[test]
    fn zones_stems_and_guides_reach_fontinfo_and_come_back() {
        use fontcraft_model::{AlignmentZone, Guide, StemDef};
        let mut font = Font::new("Design");
        font.stems = Arc::new(vec![StemDef { name: "stem".into(), ..Default::default() }, StemDef { horizontal: true, ..Default::default() }]);
        let m = &mut Arc::make_mut(&mut font.masters)[0];
        m.stem_values = vec![80.0, 40.5];
        m.zones = vec![AlignmentZone { pos: 400.0, size: -8.0 }];
        m.guides = vec![
            Guide { x: 250.0, angle: 90.0, ..Default::default() },
            Guide { y: 600.0, name: Some("top".into()), locked: true, color: Some("1,0,0,1".into()), ..Default::default() },
        ];
        let files = write(&font).unwrap();
        let info = s(&files["fontinfo.plist"]);
        let list = |key: &str, items: &[&str]| {
            let body: String = items.iter().map(|i| format!("\n      <integer>{i}</integer>")).collect();
            format!("<key>{key}</key>\n    <array>{body}\n    </array>")
        };
        // The overshoots of the five metrics and the master's other zone, by glyphsLib's rule.
        assert!(info.contains(&list("postscriptBlueValues", &["-16", "0", "500", "516", "700", "716", "800", "816"])), "{info}");
        assert!(info.contains(&list("postscriptOtherBlues", &["-216", "-200", "392", "400"])), "{info}");
        assert!(info.contains("<key>postscriptStemSnapH</key>\n    <array>\n      <real>40.5</real>"), "{info}");
        assert!(info.contains(&list("postscriptStemSnapV", &["80"])), "{info}");
        assert!(info.contains("<dict>\n        <key>x</key>\n        <integer>250</integer>\n      </dict>"), "{info}");
        assert!(info.contains("<key>color</key>\n        <string>1,0,0,1</string>\n        <key>name</key>\n        <string>top [locked]</string>"));

        let back = read(&files).unwrap();
        let b = back.default_master().unwrap();
        assert_eq!(b.metric_values, font.masters[0].metric_values, "each zone at a metric is its overshoot again");
        assert_eq!(b.zones, [AlignmentZone { pos: 400.0, size: -8.0 }], "a zone at no metric stays a zone");
        assert_eq!(back.stems.iter().map(|d| (d.name.as_str(), d.horizontal)).collect::<Vec<_>>(), [("", true), ("", false)]);
        assert_eq!(b.stem_values, [40.5, 80.0], "horizontal stems first");
        assert_eq!(b.guides, font.masters[0].guides);
        assert!(back.info.params.is_empty() && b.params.is_empty(), "nothing kept as written");
        assert_eq!(write(&back).unwrap(), files);
        assert_eq!(info_losses(&font), ["the font's stem names and filters (1)"]);
        assert!(info_losses(&back).is_empty());

        // Writing regenerates the arrays from the model.
        let mut edited = back.clone();
        let m = &mut Arc::make_mut(&mut edited.masters)[0];
        m.zones.clear();
        m.metric_values[0].over = 12.0;
        let info = s(&write(&edited).unwrap()["fontinfo.plist"]).to_string();
        assert!(info.contains(&list("postscriptBlueValues", &["-16", "0", "500", "516", "700", "716", "800", "812"])), "{info}");
        assert!(info.contains(&list("postscriptOtherBlues", &["-216", "-200"])), "{info}");
    }

    #[test]
    fn a_zone_at_zero_makes_a_baseline_and_odd_values_stay_as_written() {
        let mut files = write(&Font::new("Zero")).unwrap();
        let info = s(&files["fontinfo.plist"]).to_string();
        // A UFO has no baseline key: the zone at 0 needs the metric.
        let back = read(&files).unwrap();
        assert_eq!(back.metric(back.default_master().unwrap(), metric::BASELINE), Some(MetricValue { pos: 0.0, over: -16.0 }));
        // Pairs glyphsLib's rule would reorder are kept as written, both blue keys together.
        let odd = info.replace("<integer>-216</integer>\n      <integer>-200</integer>", "<integer>-200</integer>\n      <integer>-216</integer>");
        assert_ne!(odd, info);
        files.insert("fontinfo.plist".into(), odd.clone().into_bytes());
        let font = read(&files).unwrap();
        let m = font.default_master().unwrap();
        let raw: Vec<&str> = m.params.0.iter().filter(|p| fontinfo::is_raw(p)).map(|p| p.name.as_str()).collect();
        assert_eq!(raw, ["postscriptBlueValues", "postscriptOtherBlues"]);
        assert!(m.metric_values.iter().all(|v| v.over == 0.0) && m.zones.is_empty());
        assert_eq!(s(&write(&font).unwrap()["fontinfo.plist"]), odd);
    }

    #[test]
    fn broken_ufos_are_errors() {
        let good = write(&Font::new("Broken")).unwrap();
        let mut no_meta = good.clone();
        no_meta.remove("metainfo.plist");
        assert!(read(&no_meta).is_err());
        let mut v4 = good.clone();
        v4.insert("metainfo.plist".into(), s(&good["metainfo.plist"]).replace("<integer>3</integer>", "<integer>4</integer>").into_bytes());
        assert!(read(&v4).is_err());
        let mut missing_glif = good.clone();
        missing_glif.remove("glyphs/a.glif");
        assert!(read(&missing_glif).is_err());
        let mut escape = good.clone();
        escape.insert(
            "glyphs/contents.plist".into(),
            s(&good["glyphs/contents.plist"]).replace("<string>a.glif</string>", "<string>../a.glif</string>").into_bytes(),
        );
        assert!(read(&escape).is_err());
        let mut not_xml = good;
        not_xml.insert("fontinfo.plist".into(), b"\xff\xfe".to_vec());
        assert!(read(&not_xml).is_err());
    }

    /// A font whose glyph `A` has, beside its master layer, the layers a `.glyphs` source has.
    fn layered() -> Font {
        use fontcraft_model::{AxisRule, Background, Guide, Image, PaletteEntry};
        let mut font = Font::new("Layers");
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0];
        let mut a = font.glyph_mut("A").unwrap();
        let rect = || Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0));
        {
            let m = a.layer_mut("m01").unwrap();
            m.info.guides.push(Guide { x: 10.0, angle: 90.0, ..Default::default() });
            m.info.image = Some(Image { path: Some("a.png".into()), transform: Transform::translate(5.0, 0.0), ..Default::default() });
            m.info.background = Some(Background { shapes: vec![rect()], ..Default::default() });
            m.info.user_data.0.insert("org.example", Value::Bool(true));
        }
        let other = |id: &str, name: Option<&str>| {
            let mut l = Layer::new("m01", 600.0);
            l.id = id.into();
            l.info.name = name.map(str::to_string);
            l.shapes.push(rect());
            l
        };
        let backup = other("L1", Some("Backup"));
        let mut second = other("L2", Some("Backup"));
        second.info.background = Some(Background { shapes: vec![rect()], ..Default::default() });
        let mut brace = other("L3", Some("Apr 1 24"));
        brace.info.attrs.coordinates = vec![Some(550.0)];
        let mut color = other("L4", None);
        color.info.attrs.color_palette = Some(PaletteEntry::Foreground);
        let mut bracket = other("L5", Some("Alt"));
        bracket.info.attrs.axis_rules = vec![AxisRule { min: Some(500.0), max: None }];
        let unnamed = other("L6", None);
        a.layers.extend([backup, second, brace, color, bracket, unnamed]);
        drop(a);
        font
    }

    #[test]
    fn backgrounds_and_other_layers_go_to_ufo_layers_as_glyphslib_names_them() {
        let font = layered();
        let files = write(&font).unwrap();
        let lc = s(&files["layercontents.plist"]);
        let names: Vec<&str> = lc.lines().filter_map(|l| l.trim().strip_prefix("<string>")?.strip_suffix("</string>")).step_by(2).collect();
        assert_eq!(names, ["public.default", "public.background", "Backup", "Backup #1", "Backup #1.background", "{550}", "color.65535"]);
        assert!(s(&files["glyphs/A_.glif"]).contains("<image fileName=\"a.png\" xOffset=\"5\"/>"));
        assert!(s(&files["glyphs/A_.glif"]).contains("<guideline x=\"10\"/>"));
        assert!(s(&files["glyphs/A_.glif"]).contains("<key>org.example</key>"));
        assert!(s(&files["glyphs.public.background/A_.glif"]).contains("<contour>"));
        assert!(s(&files["glyphs.B_ackup/A_.glif"]).contains("<unicode hex=\"0041\"/>"), "glyphsLib gives every layer the code points");
        assert_eq!(layers_left_out(&font, false), 2, "the alternate and the unnamed layer");
        let losses = info_losses(&font);
        // The colour layer's palette entry travels in the colour layer mapping; it gets a name.
        for line in [
            "the layers' names (3)",
            "the layers' attributes other than colour palette entries (1)",
            "the ids of layers other than master layers (2)",
        ] {
            assert!(losses.iter().any(|l| l == line), "{line} in {losses:?}");
        }

        // Read back: the backgrounds are backgrounds again, the rest named layers.
        let back = read(&files).unwrap();
        let a = back.glyph("A").unwrap();
        let m = a.layer("m01").unwrap();
        assert_eq!(m.info.background.as_ref().map(|b| b.shapes.len()), Some(1));
        assert_eq!((m.info.guides.len(), m.info.image.as_ref().and_then(|i| i.path.as_deref())), (1, Some("a.png")));
        assert!(m.info.user_data.get("org.example").is_some());
        let names: Vec<Option<&str>> = a.layers.iter().filter(|l| !l.is_master_layer()).map(|l| l.info.name.as_deref()).collect();
        assert_eq!(names, [Some("Backup"), Some("Backup #1"), Some("{550}"), Some("color.65535")]);
        assert!(a.layers.iter().find(|l| l.info.name.as_deref() == Some("Backup #1")).unwrap().info.background.is_some());
        assert_eq!(write(&back).unwrap(), files, "a UFO we wrote comes back unchanged");
    }

    #[test]
    fn image_files_that_do_not_travel_are_losses() {
        const LOST: &str = "the layers' image files, which are not PNG files in the UFO's images folder (1)";
        let lost = |losses: Vec<String>| losses.iter().any(|l| l == LOST);
        let holding = |font: &Font, path: &str| {
            let mut font = font.clone();
            let mut files = fontcraft_model::FileStore::default();
            files.0.insert(path.into(), std::sync::Arc::from(&b"png"[..]));
            font.files = std::sync::Arc::new(files);
            font
        };
        let mut font = layered();
        font.source.format = SourceFormat::Ufo;
        assert!(lost(info_losses(&font)), "a file the font does not hold");
        // Another UFO's images folder, or a folder whose name only ends in `images`, is not the
        // UFO's own.
        for other in ["Other.ufo/images/a.png", "fooimages/a.png"] {
            assert!(lost(info_losses(&holding(&font, other))), "{other}");
        }
        let single = holding(&font, "images/a.png");
        assert!(!lost(info_losses(&single)));
        assert!(!lost(info_losses_for(&single, FilesFrom::PerMaster)));
        // Saved as a designspace, a master without a file name gets a new UFO, which the root's
        // files do not reach.
        assert!(lost(crate::designspace::info_losses(&single)));
        let mut glyphs_source = single.clone();
        glyphs_source.source.format = SourceFormat::Glyphs;
        assert!(lost(info_losses(&glyphs_source)), "only a UFO source's files are written");

        // Read from a designspace: the master's UFO keeps its files under its file name, which
        // the designspace writer and the one-UFO-per-master export strip, and the single-UFO
        // writer does not.
        let mut ds = holding(&font, "Layered.ufo/images/a.png");
        ds.source.format = SourceFormat::Designspace;
        std::sync::Arc::make_mut(&mut ds.masters)[0].extra.insert(Family::Designspace, "filename", Value::String("Layered.ufo".into()));
        assert!(!lost(crate::designspace::info_losses(&ds)));
        assert!(!lost(info_losses_for(&ds, FilesFrom::PerMaster)));
        assert!(lost(info_losses(&ds)));
        assert!(lost(crate::designspace::info_losses(&holding(&ds, "images/a.png"))), "a file at the root is not in the UFO's folder");

        let mut jpeg = single;
        {
            let mut a = jpeg.glyph_mut("A").unwrap();
            a.layer_mut("m01").unwrap().info.image.as_mut().unwrap().path = Some("a.jpg".into());
        }
        assert!(lost(info_losses(&jpeg)), "a JPEG is not converted");
    }

    #[test]
    fn vertical_metrics_follow_glyphslib() {
        let mut font = Font::new("Vert");
        font.set_metric_pos("m01", metric::ASCENDER, 800.0).unwrap();
        font.set_metric_pos("m01", metric::DESCENDER, -200.0).unwrap();
        font.glyph_mut("A").unwrap().layer_mut("m01").unwrap().info.vert_width = Some(900.0);
        font.glyph_mut("B").unwrap().layer_mut("m01").unwrap().info.vert_origin = Some(-20.0);
        let files = write(&font).unwrap();
        // A vertical font: every layer gets a height and an origin, the defaults where it has none.
        assert!(s(&files["glyphs/A_.glif"]).contains("<advance height=\"900\" width=\"600\"/>"), "{}", s(&files["glyphs/A_.glif"]));
        assert!(s(&files["glyphs/A_.glif"]).contains("<key>public.verticalOrigin</key>\n      <integer>800</integer>"));
        assert!(s(&files["glyphs/B_.glif"]).contains("height=\"1000\""));
        assert!(s(&files["glyphs/B_.glif"]).contains("<integer>820</integer>"), "the ascender minus the origin");
        let back = read(&files).unwrap();
        let info = |n: &str| back.glyph(n).unwrap().layer("m01").unwrap().info.clone();
        assert_eq!((info("A").vert_width, info("A").vert_origin), (Some(900.0), None), "the defaults read as none");
        assert_eq!((info("B").vert_width, info("B").vert_origin), (None, Some(-20.0)));
        assert_eq!(write(&back).unwrap(), files, "the defaults are written back where they were");
        // Without an ascender the origin cannot be placed, and the loss report says so.
        let mut flat = Font::new("Flat");
        let i = flat.metrics.iter().position(|d| d.kind == metric::ASCENDER).unwrap();
        Arc::make_mut(&mut flat.metrics).remove(i);
        for m in Arc::make_mut(&mut flat.masters) {
            m.metric_values.remove(i);
        }
        flat.glyph_mut("A").unwrap().layer_mut("m01").unwrap().info.vert_origin = Some(5.0);
        assert!(info_losses(&flat).iter().any(|l| l.starts_with("the layers' vertical origins, without an ascender")), "{:?}", info_losses(&flat));
    }

    #[test]
    fn a_glyph_without_a_height_in_a_vertical_ufo_has_height_0() {
        let mut font = Font::new("Vert");
        for (name, v) in [("vheaVertAscender", 500), ("vheaVertDescender", -500), ("vheaVertLineGap", 0)] {
            Arc::make_mut(&mut font.info).params.0.push(Param::new(name, Value::Number(Number::int(v))));
        }
        let mut files = write(&font).unwrap();
        let b = s(&files["glyphs/B_.glif"]).to_string();
        assert!(b.contains(" height=\"1000\""), "{b}");
        files.insert("glyphs/B_.glif".into(), b.replace(" height=\"1000\"", "").into_bytes());
        let back = read(&files).unwrap();
        let height = |n: &str| back.glyph(n).unwrap().layer("m01").unwrap().info.vert_width;
        assert_eq!((height("A"), height("B")), (None, Some(0.0)), "the default height, and none at all (ufo2ft's 0)");
        assert_eq!(write(&back).unwrap(), files);
    }

    /// A property list of a dictionary whose entries are XML snippets, keys sorted when `sorted`.
    fn plist_dict(entries: &[(&str, &str)], sorted: bool) -> Vec<u8> {
        let mut entries = entries.to_vec();
        if sorted {
            entries.sort_by_key(|(k, _)| *k);
        }
        let body: String = entries.iter().map(|(k, v)| format!("    <key>{k}</key>\n    {v}\n")).collect();
        format!("{PLIST_HEAD}  <dict>\n{body}  </dict>\n</plist>\n").into_bytes()
    }

    /// The same small font as a UFO 1, with its feature code and hint data in the lib, and in the
    /// UFO 3 form the up-conversion gives it.
    fn ufo1_and_ufo3() -> (Files, Files) {
        let shared_info = [
            ("familyName", "<string>Sample</string>"),
            ("styleName", "<string>Bold Italic</string>"),
            ("unitsPerEm", "<integer>1000</integer>"),
            ("ascender", "<integer>750</integer>"),
            ("descender", "<integer>-250</integer>"),
            ("capHeight", "<integer>700</integer>"),
            ("xHeight", "<integer>500</integer>"),
            ("italicAngle", "<integer>-10</integer>"),
            ("versionMajor", "<integer>1</integer>"),
            ("versionMinor", "<integer>2</integer>"),
            ("copyright", "<string>Copyright notice</string>"),
            ("note", "<string>A note</string>"),
        ];
        let renamed = [
            ("menuName", "styleMapFamilyName", "<string>Sample</string>", "<string>Sample</string>"),
            ("fontStyle", "styleMapStyleName", "<integer>33</integer>", "<string>bold italic</string>"),
            ("fullName", "postscriptFullName", "<string>Sample Bold Italic</string>", "<string>Sample Bold Italic</string>"),
            ("fontName", "postscriptFontName", "<string>Sample-BoldItalic</string>", "<string>Sample-BoldItalic</string>"),
            ("designer", "openTypeNameDesigner", "<string>A Designer</string>", "<string>A Designer</string>"),
            ("notice", "openTypeNameDescription", "<string>A description</string>", "<string>A description</string>"),
            ("widthName", "openTypeOS2WidthClass", "<string>Condensed</string>", "<integer>3</integer>"),
            ("weightValue", "openTypeOS2WeightClass", "<integer>700</integer>", "<integer>700</integer>"),
            ("weightName", "postscriptWeightName", "<string>Bold</string>", "<string>Bold</string>"),
            ("ttVendor", "openTypeOS2VendorID", "<string>NONE</string>", "<string>NONE</string>"),
            ("msCharSet", "postscriptWindowsCharacterSet", "<integer>0</integer>", "<integer>1</integer>"),
        ];
        let hints = [
            ("postscriptBlueValues", "<array><integer>-10</integer><integer>0</integer><integer>700</integer><integer>710</integer></array>"),
            ("postscriptStemSnapH", "<array><integer>80</integer></array>"),
            ("postscriptStemSnapV", "<array><integer>90</integer></array>"),
            ("postscriptBlueScale", "<real>0.039625</real>"),
        ];
        let kern = "feature kern {\n    pos A B -20;\n} kern;\n";
        let classes = "@UC = [A B];\n";
        let glif = |name: &str, unicode: &str, format: u32, body: &str| {
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<glyph name=\"{name}\" format=\"{format}\">\n  <advance width=\"600\"/>\n  <unicode hex=\"{unicode}\"/>\n{body}</glyph>\n").into_bytes()
        };
        let triangle = "    <contour>\n      <point x=\"0\" y=\"0\" type=\"line\"/>\n      <point x=\"600\" y=\"0\" type=\"line\"/>\n      <point x=\"300\" y=\"700\" type=\"line\"/>\n    </contour>\n";
        let component = "  <outline>\n    <component base=\"A\" xOffset=\"10\"/>\n  </outline>\n";
        let contents = plist_dict(&[("A", "<string>A_.glif</string>"), ("B", "<string>B_.glif</string>")], false);

        let mut one = Files::new();
        one.insert(
            METAINFO.into(),
            plist_dict(&[("creator", "<string>org.robofab.ufoLib</string>"), ("formatVersion", "<integer>1</integer>")], false),
        );
        let mut info1: Vec<(&str, &str)> = shared_info.to_vec();
        info1.extend(renamed.iter().map(|(old, _, v, _)| (*old, *v)));
        one.insert(FONTINFO.into(), plist_dict(&info1, false));
        let features = format!("<dict><key>kern</key><string>{kern}</string></dict>");
        let hint_data = "<dict><key>blueValues</key><array><array><integer>-10</integer><integer>0</integer></array><array><integer>700</integer><integer>710</integer></array></array><key>hStems</key><array><integer>80</integer></array><key>vStems</key><array><integer>90</integer></array><key>blueScale</key><real>0.039625</real></dict>";
        let classes_xml = format!("<string>{classes}</string>");
        one.insert(
            LIB.into(),
            plist_dict(
                &[
                    ("com.example.keep", "<string>yes</string>"),
                    ("org.robofab.opentype.classes", classes_xml.as_str()),
                    ("org.robofab.opentype.featureorder", "<array><string>kern</string></array>"),
                    ("org.robofab.opentype.features", features.as_str()),
                    ("org.robofab.postScriptHintData", hint_data),
                ],
                false,
            ),
        );
        let groups = [("UC", "<array><string>A</string><string>B</string></array>")];
        let mut groups1 = groups.to_vec();
        groups1.push(("@MMK_L_A", "<array><string>A</string></array>"));
        one.insert(GROUPS.into(), plist_dict(&groups1, false));
        one.insert(KERNING.into(), plist_dict(&[("@MMK_L_A", "<dict><key>B</key><integer>-20</integer></dict>")], false));
        one.insert("glyphs/contents.plist".into(), contents.clone());
        let anchor1 = "    <contour>\n      <point x=\"300\" y=\"700\" type=\"move\" name=\"top\"/>\n    </contour>\n";
        one.insert("glyphs/A_.glif".into(), glif("A", "0041", 1, &format!("  <outline>\n{anchor1}{triangle}  </outline>\n")));
        one.insert("glyphs/B_.glif".into(), glif("B", "0042", 1, component));

        let mut three = Files::new();
        three.insert(
            METAINFO.into(),
            plist_dict(&[("creator", "<string>org.robofab.ufoLib</string>"), ("formatVersion", "<integer>3</integer>")], false),
        );
        let mut info3: Vec<(&str, &str)> = shared_info.to_vec();
        info3.extend(renamed.iter().map(|(_, new, _, v)| (*new, *v)));
        info3.extend(hints);
        three.insert(FONTINFO.into(), plist_dict(&info3, true));
        three.insert(LIB.into(), plist_dict(&[("com.example.keep", "<string>yes</string>")], false));
        three.insert(FEATURES.into(), format!("{classes}\n{kern}").into_bytes());
        let mut groups3 = groups.to_vec();
        groups3.push(("public.kern1.A", "<array><string>A</string></array>"));
        three.insert(GROUPS.into(), plist_dict(&groups3, true));
        three.insert(KERNING.into(), plist_dict(&[("public.kern1.A", "<dict><key>B</key><integer>-20</integer></dict>")], false));
        three.insert(
            LAYERCONTENTS.into(),
            format!("{PLIST_HEAD}  <array>\n    <array>\n      <string>public.default</string>\n      <string>glyphs</string>\n    </array>\n  </array>\n</plist>\n")
                .into_bytes(),
        );
        three.insert("glyphs/contents.plist".into(), contents);
        let anchor3 = "  <anchor x=\"300\" y=\"700\" name=\"top\"/>\n";
        three.insert("glyphs/A_.glif".into(), glif("A", "0041", 2, &format!("{anchor3}  <outline>\n{triangle}  </outline>\n")));
        three.insert("glyphs/B_.glif".into(), glif("B", "0042", 2, component));
        (one, three)
    }

    /// Asserts that two documents hold the same model, leaving out what each file format keeps
    /// for itself (`Extra`).
    fn same_model(a: &Font, b: &Font) {
        assert_eq!(
            (&a.family_name, a.units_per_em, a.version_major, a.version_minor, &a.note),
            (&b.family_name, b.units_per_em, b.version_major, b.version_minor, &b.note)
        );
        assert_eq!(a.info, b.info, "font info");
        assert_eq!((&a.metrics, &a.stems), (&b.metrics, &b.stems));
        let (ma, mb) = (&a.masters[0], &b.masters[0]);
        assert_eq!(
            (&ma.id, &ma.name, &ma.metric_values, &ma.zones, &ma.stem_values),
            (&mb.id, &mb.name, &mb.metric_values, &mb.zones, &mb.stem_values)
        );
        assert_eq!((&ma.properties, &ma.params, &ma.user_data), (&mb.properties, &mb.params, &mb.user_data), "master values");
        assert_eq!(a.groups, b.groups);
        assert_eq!(a.kerning, b.kerning);
        assert_eq!(a.features, b.features);
        let outline = |l: &Layer| -> Vec<String> {
            l.shapes
                .iter()
                .map(|s| match s {
                    fontcraft_model::Shape::Path(p) => format!("{} {:?}", p.closed, p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect::<Vec<_>>()),
                    fontcraft_model::Shape::Component(c) => format!("{} {:?}", c.glyph, c.transform),
                    other => format!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(a.glyphs.len(), b.glyphs.len());
        for (ga, gb) in a.glyphs.iter().zip(b.glyphs.iter()) {
            assert_eq!((&ga.name, &ga.unicodes, &ga.kern_groups, ga.export), (&gb.name, &gb.unicodes, &gb.kern_groups, gb.export));
            let (la, lb) = (ga.layer(&ma.id).unwrap(), gb.layer(&mb.id).unwrap());
            assert_eq!(la.width, lb.width, "{}", ga.name);
            assert_eq!(outline(la), outline(lb), "{}", ga.name);
            let anchors = |l: &Layer| l.anchors.iter().map(|a| (a.name.clone(), a.x, a.y)).collect::<Vec<_>>();
            assert_eq!(anchors(la), anchors(lb), "{}", ga.name);
        }
    }

    #[test]
    fn a_ufo1_opens_as_its_ufo3_form_and_saves_as_ufo3() {
        let (one, three) = ufo1_and_ufo3();
        let font = read(&one).unwrap();
        let expected = read(&three).unwrap();
        assert_eq!(font.source.ufo_version, Some(3));
        same_model(&font, &expected);
        assert_eq!(font.kerning.ltr.get(&font.masters[0].id, "public.kern1.A", "B"), Some(-20.0));
        assert_eq!(font.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["UC"]);
        assert!(!font.masters[0].user_data.0.entries.iter().any(|(k, _)| k.as_str().starts_with("org.robofab")), "the lib keeps only its own keys");

        let out = write(&font).unwrap();
        assert!(s(&out[METAINFO]).contains("<integer>3</integer>"), "{}", s(&out[METAINFO]));
        assert!(out.contains_key(LAYERCONTENTS), "a UFO 3 lists its layers");
        let a = s(&out["glyphs/A_.glif"]);
        assert!(a.contains("format=\"2\"") && a.contains("<anchor"), "{a}");
        assert_eq!(s(&out[FEATURES]), s(&three[FEATURES]));
        let info = s(&out[FONTINFO]);
        assert!(info.contains("<key>styleMapStyleName</key>") && !info.contains("<key>menuName</key>"), "{info}");
        assert!(!s(&out[LIB]).contains("org.robofab"), "{}", s(&out[LIB]));
        let groups = s(&out[GROUPS]);
        assert!(groups.contains("<key>public.kern1.A</key>") && !groups.contains("@MMK_L_A"), "{groups}");
        same_model(&read(&out).unwrap(), &font);
    }

    /// ufoLib and fontc refuse a `fontinfo.plist` whose style-map style is not one of the four
    /// lower-case names; the lib keeps such a style, and reading puts it back.
    #[test]
    fn a_style_map_style_outside_the_four_is_kept_in_the_lib() {
        let stated = |font: &Font| {
            let m = &font.masters[0];
            m.params.get(STYLE_MAP_STYLE).or_else(|| font.info.params.get(STYLE_MAP_STYLE)).and_then(Value::as_str).map(str::to_string)
        };
        for (style, in_info) in [("thin", false), ("Bold", false), ("bold", true)] {
            let mut font = Font::new("Solo");
            Arc::make_mut(&mut font.masters)[0].params.set(STYLE_MAP_STYLE, str_value(style));
            let out = write(&font).unwrap();
            let (info, lib) = (s(&out[FONTINFO]), out.get(LIB).map_or("", |b| s(b)));
            assert_eq!(info.contains("<key>styleMapStyleName</key>"), in_info, "{info}");
            assert_eq!(lib.contains(STYLE_MAP_STYLE_LIB), !in_info, "{lib}");
            let back = read(&out).unwrap();
            assert_eq!(stated(&back).as_deref(), Some(style));
            assert!(back.masters[0].user_data.get(STYLE_MAP_STYLE_LIB).is_none(), "the key is not user data");
            assert_eq!(write(&back).unwrap(), out, "{style}");
        }
    }
}

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
//! | the master's user data | `lib.plist`, except the keys below |
//! | master name; metrics `ascender`, `cap height`, `x-height`, `descender`, `italic angle` | `styleName`; `ascender`, `capHeight`, `xHeight`, `descender`, `italicAngle` (counter-clockwise, so the sign is flipped) |
//! | glyph order | `lib.plist` `public.glyphOrder` first, then the rest in `contents.plist` order |
//! | glyph export | `lib.plist` `public.skipExportGlyphs` |
//! | glyph code points | the default layer's GLIF |
//! | kerning groups | `groups.plist` `public.kern1.*` (the glyph's right side) and `public.kern2.*` (left side) |
//! | kerning | `kerning.plist`, as the left-to-right table, sides spelled as the UFO spells them |
//! | features | `features.fea`, as one unnamed prefix |
//! | layers | the default layer's GLIF is the master layer; other UFO layers' GLIFs are further layers whose `Extra` names their UFO layer (`layer`) |
//! | `data/`, `images/`, other files | [`Font::files`] |

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{
    Extra, Family, Features, FileStore, Font, FontInfo, Glyph, GlyphTable, KernFirst, KernTable, Kerning, Layer, Master, MasterKerning, MetricDef,
    MetricValue, Snippet, Source, SourceFormat, UserData, metric,
};
use fontcraft_plist::xml::{self, Node as XNode};
use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value, xmlplist};

use crate::fontinfo::{self, Info};
use crate::glif::{self, Style};
use crate::names::{glyph_file_name, layer_dir_name};
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
const TYPED_LIB: [&str; 2] = ["public.glyphOrder", "public.skipExportGlyphs"];
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

/// What one UFO holds, before it joins a document.
pub(crate) struct ReadUfo {
    pub master: Master,
    /// The `fontinfo.plist` values that are font-wide in the model.
    pub family_name: Option<String>,
    pub units_per_em: Option<f64>,
    pub version_major: Option<i64>,
    pub version_minor: Option<i64>,
    pub note: Option<String>,
    /// The other font information (see [`crate::fontinfo`]).
    pub info: Info,
    /// Which metric kinds the file has (the master's values follow [`METRICS`]).
    pub metrics: Vec<(&'static str, f64)>,
    pub features: Option<String>,
    /// In glyph order.
    pub glyphs: Vec<Glyph>,
    pub kerning: MasterKerning,
    pub files: BTreeMap<String, Arc<[u8]>>,
    pub format_version: u32,
}

/// The id of the master layer's layer in a non-default UFO layer.
fn layer_id(master: &str, layer: &str) -> String {
    format!("{master}#{layer}")
}

/// Reads the UFO in `files` as the master `master_id`.
pub(crate) fn read_master(files: &Files, master_id: &str) -> Result<ReadUfo> {
    let meta_doc = read_plist(files, METAINFO)?.ok_or_else(|| bad(METAINFO, "missing: this is not a UFO"))?;
    let plist_style = PlistStyle::of(&meta_doc);
    let Value::Dict(meta) = meta_doc.root else { return Err(bad(METAINFO, "the file does not hold a dictionary")) };
    let format_version = match meta.get("formatVersion").and_then(as_f64) {
        Some(3.0) => 3,
        Some(2.0) => 2,
        Some(v) => return Err(UfoError::Unsupported(format!("UFO format version {v}"))),
        None => return Err(bad(METAINFO, "no formatVersion")),
    };
    let glif_format = if format_version == 2 { 1 } else { 2 };

    let mut master = Master { id: master_id.to_string(), ..Default::default() };
    let mx = &mut master.extra;
    mx.push(Family::Ufo, "plistStyle", plist_style.to_value());
    mx.push(Family::Ufo, METAINFO, Value::Dict(meta));
    let mut plists = Plists { files, style: plist_style, styles: Dict::new() };

    // fontinfo.plist
    let info = plists.dict(FONTINFO)?;
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
    let font_info = fontinfo::read(info.as_ref().unwrap_or(&Dict::new()), upm);
    if let Some(info) = info {
        master.extra.push(Family::Ufo, FONTINFO, Value::Dict(info));
    }

    // lib.plist
    // A lib that is not a well-formed property list (some old tools wrote keys without values)
    // is kept as a file and written back as it is.
    let lib = plists.dict(LIB).ok().flatten();
    let order = strings(lib.as_ref().and_then(|d| d.get("public.glyphOrder")));
    let skip: HashSet<String> = strings(lib.as_ref().and_then(|d| d.get("public.skipExportGlyphs"))).into_iter().collect();
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
        None => vec![("public.default".to_string(), DEFAULT_DIR.to_string())],
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
                layer.extra.push(Family::Ufo, "layer", Value::String(layer_name.as_str().into()));
                if !g.unicodes.is_empty() {
                    let items = g.unicodes.iter().map(|u| Value::Number(Number::int(i64::from(*u)))).collect();
                    layer.extra.push(Family::Ufo, "unicodes", Value::Array(Array { items, layout: Layout::Compact, ..Default::default() }));
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

    // A glyph's master layer comes first.
    for g in &mut glyphs {
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

    // Kerning groups and kerning.
    let groups = plists.dict(GROUPS)?;
    if let Some(groups) = &groups {
        let by_name: HashMap<String, usize> = ordered.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        for (group, members) in &groups.entries {
            let (side_right, short) = match (group.as_str().strip_prefix(KERN1), group.as_str().strip_prefix(KERN2)) {
                (Some(s), _) => (true, s),
                (_, Some(s)) => (false, s),
                _ => continue,
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
    if let Some(groups) = groups {
        master.extra.push(Family::Ufo, GROUPS, Value::Dict(groups));
    }
    let mut kerning = MasterKerning { master: master_id.to_string(), firsts: Vec::new() };
    if let Some(k) = plists.dict(KERNING)? {
        for (first, seconds) in &k.entries {
            let seconds = seconds.as_dict().ok_or_else(|| bad(KERNING, format!("`{}` does not map to a dictionary", first.as_str())))?;
            let mut f = KernFirst { first: first.as_str().to_string(), seconds: Vec::with_capacity(seconds.len()) };
            for (second, value) in &seconds.entries {
                let v = as_f64(value).ok_or_else(|| bad(KERNING, format!("the pair `{}` `{}` has no number", first.as_str(), second.as_str())))?;
                f.seconds.push((second.as_str().to_string(), v));
            }
            kerning.firsts.push(f);
        }
        master.extra.push(Family::Ufo, KERNING, Value::Dict(k));
    }

    let features = match files.get(FEATURES) {
        Some(bytes) => match std::str::from_utf8(bytes) {
            Ok(t) => Some(t.to_string()),
            // Not text: kept as a file.
            Err(_) => None,
        },
        None => None,
    };
    for f in [METAINFO, FONTINFO, GROUPS, KERNING, LAYERCONTENTS] {
        consumed.insert(f.to_string());
    }
    if ufo_get(&master.extra, LIB).is_some() {
        consumed.insert(LIB.to_string());
    }
    if features.is_some() {
        consumed.insert(FEATURES.to_string());
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
        metrics,
        features,
        glyphs: ordered,
        kerning,
        files: rest,
        format_version,
    })
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

/// Reads a single UFO as a document with one master.
pub fn read(files: &Files) -> Result<Font> {
    let master_id = "m01";
    let r = read_master(files, master_id)?;
    let mut metrics: Vec<MetricDef> = Vec::new();
    let mut values: Vec<MetricValue> = Vec::new();
    for (kind, v) in &r.metrics {
        metrics.push(MetricDef::new(kind));
        values.push(MetricValue { pos: *v, over: 0.0 });
    }
    let mut master = r.master;
    master.metric_values = values;
    let units_per_em = units_per_em(r.units_per_em)?;
    let mut features = Features::default();
    if let Some(code) = r.features {
        features.prefixes.push(Snippet { name: String::new(), code, ..Default::default() });
    }
    let font = Font {
        source: Source { format: SourceFormat::Ufo, ufo_version: Some(r.format_version), ..Default::default() },
        family_name: r.family_name.unwrap_or_default(),
        units_per_em,
        version_major: r.version_major.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        version_minor: r.version_minor.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        axes: Arc::default(),
        metrics: Arc::new(metrics),
        stems: Arc::default(),
        numbers: Arc::default(),
        masters: Arc::new(vec![master]),
        instances: Arc::default(),
        glyphs: GlyphTable::from_glyphs(r.glyphs.into_iter().map(Arc::new).collect()),
        info: Arc::new(FontInfo { properties: r.info.properties, params: r.info.params, date: r.info.date, ..Default::default() }),
        kerning: Arc::new(Kerning {
            ltr: if r.kerning.firsts.is_empty() { KernTable::default() } else { KernTable(vec![r.kerning]) },
            ..Default::default()
        }),
        features: Arc::new(features),
        display_strings: Arc::default(),
        note: r.note.unwrap_or_default(),
        files: Arc::new(FileStore(r.files)),
        extra: Arc::default(),
    };
    font.check()?;
    Ok(font)
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
fn patch(d: &mut Dict, key: &str, new: Option<Value>) {
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
fn kept_dict(m: &Master, key: &str) -> (Dict, bool) {
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
        if !is_default
            && value.as_ref() == default_now.get(key)
            && default_info.get(key) == default_now.get(key)
            && read_info.get(key) != default_info.get(key)
        {
            value = read_info.get(key).cloned();
        }
        patch(&mut info, key, value);
    }
    if had_info || !info.is_empty() {
        out.insert(FONTINFO.into(), plist(FONTINFO, &Value::Dict(info)));
    }

    // groups.plist
    let groups = {
        let (mine, _) = kept_dict(master, GROUPS);
        let (default_read, _) = kept_dict(default, GROUPS);
        let default_now = write_groups(font, default);
        if !is_default && default_now == default_read && mine != default_read { mine } else { write_groups(font, master) }
    };
    // An empty file is written back where there was one.
    if !groups.is_empty() || ufo_get(&master.extra, GROUPS).is_some() {
        out.insert(GROUPS.into(), plist(GROUPS, &Value::Dict(groups)));
    }
    // kerning.plist
    let kerning = write_kerning(font, master);
    if !kerning.is_empty() || ufo_get(&master.extra, KERNING).is_some() {
        out.insert(KERNING.into(), plist(KERNING, &Value::Dict(kerning)));
    }
    // lib.plist: the master's user data, and the typed keys below.
    let (mut lib, _) = kept_dict(master, LIB);
    let stale: Vec<String> = lib
        .entries
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| !TYPED_LIB.contains(k) && master.user_data.get(k).is_none())
        .map(str::to_string)
        .collect();
    for k in stale {
        patch(&mut lib, &k, None);
    }
    for (k, v) in &master.user_data.0.entries {
        if !TYPED_LIB.contains(&k.as_str()) {
            patch(&mut lib, k.as_str(), Some(v.without_spellings()));
        }
    }
    let skip: Vec<&str> = font.glyphs.iter().filter(|g| !g.export).map(|g| g.name.as_str()).collect();
    // The kept list stays while it names the same glyphs, in whatever order, and in a master that
    // read a list of its own (often none) while the default master's is unchanged.
    let kept_skip = sorted(strings(lib.get("public.skipExportGlyphs")));
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
    let order: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
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
        let value = Value::Array(Array { items: order.iter().map(|n| str_value(n)).collect(), ..Default::default() });
        patch(&mut lib, "public.glyphOrder", Some(value));
    }
    if !lib.is_empty() {
        out.insert(LIB.into(), plist(LIB, &Value::Dict(lib)));
    }
    // features.fea
    let fea = features_text(&font.features);
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
    let default_name = kept_layers.iter().find(|(_, d)| d == DEFAULT_DIR).map_or("public.default".to_string(), |(n, _)| n.clone());
    // Which UFO layers the glyphs use, in the kept order, then in order of appearance.
    let mut used: Vec<String> = vec![default_name.clone()];
    for g in font.glyphs.iter() {
        for l in &g.layers {
            if let Some(n) = ufo_layer_of(l, target)
                && !used.iter().any(|u| u == n)
            {
                used.push(n.to_string());
            }
        }
    }
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
    if version >= 3 && (from_ufo && kept_lc.is_some() || !from_ufo || layers.len() > 1 || ufo_get(&master.extra, LAYERCONTENTS).is_some()) {
        out.insert(LAYERCONTENTS.into(), plist(LAYERCONTENTS, &kept_lc.unwrap_or(layer_value)));
    }
    let layerinfo = ufo_get(&master.extra, "layerinfo").and_then(Value::as_dict);
    let kept_contents = ufo_get(&master.extra, "contents").and_then(Value::as_dict);

    for (layer_name, dir) in &layers {
        let is_default = dir == DEFAULT_DIR;
        let mut contents = Dict::new();
        let mut files_taken: HashSet<String> = HashSet::new();
        let mut entries: Vec<(String, String, &Glyph, &Layer)> = Vec::new();
        for g in font.glyphs.iter() {
            let layer = if is_default { g.layer(&master.id) } else { g.layers.iter().find(|l| ufo_layer_of(l, target) == Some(layer_name)) };
            let Some(layer) = layer else { continue };
            // The layer's own file name, else the one the glyph has in this UFO's default layer.
            let file_of = |l: Option<&Layer>| l.and_then(|l| ufo_get(&l.extra, "fileName")).and_then(Value::as_str).map(str::to_string);
            let kept = file_of(Some(layer)).or_else(|| file_of(g.layer(&master.id))).or_else(|| g.file_name.clone());
            entries.push((g.name.clone(), kept.unwrap_or_default(), g, layer));
        }
        for (_, file, _, _) in &entries {
            if !file.is_empty() {
                files_taken.insert(file.to_lowercase());
            }
        }
        for (name, file, g, layer) in &mut entries {
            if file.is_empty() {
                *file = glyph_file_name(name, &files_taken).ok_or_else(|| bad(name, "no free file name for the glyph"))?;
                files_taken.insert(file.to_lowercase());
            }
            let own = code_points(ufo_get(&layer.extra, "unicodes"));
            let unicodes: Vec<u32> = if is_default {
                // A master's own code points, where they differed from the default master's
                // when read and the glyph's are unchanged.
                match ufo_get(&layer.extra, "unicodesRead") {
                    Some(read) if code_points(Some(read)) == g.unicodes => own,
                    _ => g.unicodes.clone(),
                }
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
            let text = glif::write(glif_name, &unicodes, layer, format, &style)?;
            out.insert(format!("{dir}/{file}"), text.into_bytes());
        }
        // The kept contents, for its order and spellings; then the model's glyphs.
        if let Some(kept) = kept_contents.and_then(|c| c.get(dir)).and_then(Value::as_dict) {
            contents = kept.clone();
            contents.entries.retain(|(k, _)| entries.iter().any(|(n, _, _, _)| n == k.as_str()));
        }
        for (name, file, _, _) in &entries {
            patch(&mut contents, name, Some(str_value(file)));
        }
        if entries.is_empty() && missing.contains(&dir.as_str()) {
            continue;
        }
        let path = format!("{dir}/{CONTENTS}");
        out.insert(path.clone(), plist(&path, &Value::Dict(contents)));
        if let Some(info) = layerinfo.and_then(|d| d.get(layer_name)) {
            let path = format!("{dir}/{LAYERINFO}");
            out.insert(path.clone(), plist(&path, info));
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
    let font_info = Info { properties: font.info.properties.clone(), params: font.info.params.clone(), date: font.info.date.clone() };
    let own = Info { properties: master.properties.clone(), params: master.params.clone(), date: None };
    fontinfo::write(&fontinfo::effective(&font_info, &own), f64::from(font.units_per_em), Some(read))
}

/// What saving `font` as UFOs leaves out of its font information and user data, one line each.
pub fn info_losses(font: &Font) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in font.masters.iter().filter(|m| m.extra.get(Family::Designspace, "layer").is_none()) {
        let (read, _) = kept_dict(m, FONTINFO);
        for line in info_entries(font, m, &read).1 {
            if !out.contains(&line) {
                out.push(line);
            }
        }
    }
    if !font.info.user_data.is_empty() {
        out.push(format!("the font's user data ({} keys)", font.info.user_data.0.len()));
    }
    if !font.info.settings.is_empty() {
        out.push("the font's settings".into());
    }
    // Typed from `.glyphs` and not yet written to UFO or designspace (M1.7 steps 3b and 3c).
    if !font.stems.is_empty() {
        out.push(format!("the font's stems ({})", font.stems.len()));
    }
    if !font.numbers.is_empty() {
        out.push(format!("the font's numbers ({})", font.numbers.len()));
    }
    for m in font.masters.iter() {
        let name = if m.name.is_empty() { &m.id } else { &m.name };
        if !m.guides.is_empty() {
            out.push(format!("master `{name}`: guides ({})", m.guides.len()));
        }
        if !m.zones.is_empty() {
            out.push(format!("master `{name}`: alignment zones other than overshoots ({})", m.zones.len()));
        }
        if !m.name_parts.is_empty() {
            out.push(format!("master `{name}`: name parts"));
        }
    }
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

/// The UFO layer a non-master layer is written to in this UFO: its own `layer` for the target
/// master's layers, the layer master's layer for a layer master's master layers.
fn ufo_layer_of<'a>(l: &'a Layer, target: &'a Target<'_>) -> Option<&'a str> {
    if l.master == target.master.id && l.id != target.master.id {
        return ufo_get(&l.extra, "layer").and_then(Value::as_str);
    }
    target.layer_masters.iter().find(|(m, _)| l.id == m.id && l.master == m.id).map(|(_, name)| name.as_str())
}

fn code_points(v: Option<&Value>) -> Vec<u32> {
    v.and_then(Value::as_array)
        .map(|a| a.items.iter().filter_map(|v| v.as_num().and_then(int_of)).filter_map(|i| u32::try_from(i).ok()).collect())
        .unwrap_or_default()
}

/// The feature code of a document as one `features.fea` text.
pub(crate) fn features_text(f: &Features) -> String {
    let mut out = String::new();
    for p in &f.prefixes {
        out.push_str(&p.code);
    }
    for c in &f.classes {
        out.push_str(&format!("@{} = [{}];\n", c.name, c.code));
    }
    for feat in &f.features {
        out.push_str(&format!("feature {0} {{\n{1}\n}} {0};\n", feat.name, feat.code));
    }
    out
}

/// `groups.plist`: the kept groups, with kerning groups made from the glyphs' kerning groups.
fn write_groups(font: &Font, master: &Master) -> Dict {
    let (mut groups, _) = kept_dict(master, GROUPS);
    let mut made: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for g in font.glyphs.iter() {
        if let Some(r) = &g.kern_groups.right {
            made.entry(format!("{KERN1}{r}")).or_default().push(&g.name);
        }
        if let Some(l) = &g.kern_groups.left {
            made.entry(format!("{KERN2}{l}")).or_default().push(&g.name);
        }
    }
    // Kerning groups that no glyph is in any more go, unless they were empty already.
    groups.entries.retain(|(k, v)| {
        let kern = k.as_str().starts_with(KERN1) || k.as_str().starts_with(KERN2);
        !kern || made.contains_key(k.as_str()) || strings(Some(v)).is_empty()
    });
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
fn write_kerning(font: &Font, master: &Master) -> Dict {
    let (kept, _) = kept_dict(master, KERNING);
    let mut out = Dict { layout: kept.layout, ..Default::default() };
    let Some(mk) = font.kerning.ltr.0.iter().find(|m| m.master == master.id) else {
        // No pairs: the kept file, if it had none either, with its whitespace.
        return if kept.is_empty() { kept } else { out };
    };
    // Group sides read from the reference's format get the UFO prefixes. A UFO's own sides stay
    // as they are: older UFOs name kerning groups `@MMK_L_…` themselves.
    let from_ufo = ufo_get(&master.extra, METAINFO).is_some();
    let side = |s: &str| if from_ufo { s.to_string() } else { fontcraft_model::ufo_side(s).into_owned() };
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
    // Unchanged pairs: the kept file, with its whitespace.
    if out == kept && kept.entries.iter().zip(&out.entries).all(|((a, _), (b, _))| a == b) { kept } else { out }
}

/// Writes one master of a document as a UFO of its own, with the document's font-wide values.
pub fn write_one(font: &Font, master_id: &str) -> Result<Files> {
    let master = font.master(master_id)?;
    // A master read from a designspace keeps its UFO's other files under the UFO's name.
    let prefix = master.extra.get(Family::Designspace, "filename").and_then(Value::as_str).map(|f| format!("{f}/")).unwrap_or_default();
    write_master(font, &Target { master, default: master, layer_masters: Vec::new(), prefix })
}

/// Writes a document as a single UFO; it must have exactly one master.
pub fn write(font: &Font) -> Result<Files> {
    let [master] = font.masters.as_slice() else {
        return Err(UfoError::Unsupported(format!("a document with {} masters as a single UFO: save it as a designspace", font.masters.len())));
    };
    write_master(font, &Target { master, default: master, layer_masters: Vec::new(), prefix: String::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_model::{Component, Node, NodeKind, Path, Shape};

    fn s(b: &[u8]) -> &str {
        std::str::from_utf8(b).unwrap()
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
}

//! What was read, as a model font: one master, its glyphs, names, metrics and kerning.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use fontcraft_model::plist::{Array, Number, Value};
use fontcraft_model::{
    Anchor, Background, ColorLabel, Component, DEFAULT_LANGUAGE, Font, Glyph, GlyphTable, LANGUAGES, Layer, Localized, MAX_COORDINATE, MAX_NAME_LEN,
    MetricValue, NodeKind, Param, Properties, Property, PropertyValue, Shape, Transform, UPM_RANGE, metric,
};

use crate::glyph::{Piece, RawGlyph, RawKern};
use crate::header::{KernClass, RawFont, short};
use crate::{MAX_KERN_PAIRS, Report, Result, SfdError};

const MASTER: &str = "m01";
/// The language the format's US English name records (1033) map to.
const US_ENGLISH: u16 = 0x0409;

/// Name ids of the name table and the property each one is kept in: (id, key, localised).
const NAME_IDS: &[(usize, &str, bool)] = &[
    (0, "copyrights", true),
    (3, "uniqueID", false),
    (5, "versionString", false),
    (6, "postscriptFontName", false),
    (7, "trademarks", true),
    (8, "manufacturers", true),
    (9, "designers", true),
    (10, "descriptions", true),
    (11, "manufacturerURL", false),
    (12, "designerURL", false),
    (13, "licenses", true),
    (14, "licenseURL", false),
    (16, "preferredFamilyNames", true),
    (17, "preferredSubfamilyNames", true),
    (18, "compatibleFullNames", true),
    (19, "sampleTexts", true),
];

/// Header values that are master parameters when stated as absolute values: (keyword, offset
/// keyword or "" when there is none, parameter).
const MASTER_PARAMS: &[(&str, &str, &str)] = &[
    ("HheadAscent", "HheadAOffset", "hheaAscender"),
    ("HheadDescent", "HheadDOffset", "hheaDescender"),
    ("LineGap", "", "hheaLineGap"),
    ("OS2TypoAscent", "OS2TypoAOffset", "typoAscender"),
    ("OS2TypoDescent", "OS2TypoDOffset", "typoDescender"),
    ("OS2LineGap", "", "typoLineGap"),
    ("OS2WinAscent", "OS2WinAOffset", "winAscent"),
    ("OS2WinDescent", "OS2WinDOffset", "winDescent"),
    ("VLineGap", "", "vheaVertLineGap"),
    ("UnderlinePosition", "", "underlinePosition"),
    ("UnderlineWidth", "", "underlineThickness"),
    ("OS2SubXSize", "", "subscriptXSize"),
    ("OS2SubYSize", "", "subscriptYSize"),
    ("OS2SubXOff", "", "subscriptXOffset"),
    ("OS2SubYOff", "", "subscriptYOffset"),
    ("OS2SupXSize", "", "superscriptXSize"),
    ("OS2SupYSize", "", "superscriptYSize"),
    ("OS2SupXOff", "", "superscriptXOffset"),
    ("OS2SupYOff", "", "superscriptYOffset"),
    ("OS2StrikeYSize", "", "strikeoutSize"),
    ("OS2StrikeYPos", "", "strikeoutPosition"),
];

pub(crate) fn build(mut raw: RawFont, report: &mut Report, sort_by_gid: bool) -> Result<Font> {
    if !raw.seen_magic {
        return Err(SfdError::NotSfd("no `SplineFontDB` header".into()));
    }
    let header = std::mem::take(&mut raw.header);
    let h = |k: &str| header.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let hn = |k: &str| h(k).and_then(crate::lines::num).filter(|v| v.abs() <= MAX_COORDINATE);
    let english = raw.lang_names.get(&US_ENGLISH).cloned().unwrap_or_default();
    let english_name = |id: usize| english.get(id).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let family = h("FamilyName").map(crate::lines::utf7).or_else(|| english_name(1)).or_else(|| h("FontName").map(str::to_string));
    let family = family.unwrap_or_else(|| "Untitled".to_string());
    check_name("family", &family)?;
    let ascent = hn("Ascent").unwrap_or(800.0);
    let descent = hn("Descent").unwrap_or(200.0);
    let upm = (ascent + descent).round();
    let units_per_em = u16::try_from(upm as i64).ok().filter(|u| UPM_RANGE.contains(u)).ok_or_else(|| {
        SfdError::Unsupported(format!("an em of {upm} units (ascent {ascent} plus descent {descent}); it must be from 16 to 16384"))
    })?;

    let mut font = Font::new(family.clone());
    font.units_per_em = units_per_em;
    if let Some(v) = h("Version") {
        match version(v) {
            Some((major, minor)) => {
                font.version_major = major;
                font.version_minor = minor;
            }
            None => report.skip("a version that is not a number", "the font header"),
        }
    }

    if sort_by_gid {
        raw.glyphs.sort_by_key(|g| (g.gid.is_none(), g.gid.unwrap_or(0)));
    }
    let mut list = std::mem::take(&mut raw.glyphs);
    let glyph_kerns: Vec<GlyphKerns> =
        list.iter_mut().filter(|g| !g.kerns.is_empty()).map(|g| (g.name.clone(), std::mem::take(&mut g.kerns), g.at.clone())).collect();
    let mut glyphs = build_glyphs(&raw, list, units_per_em, report)?;
    kerning(&mut font, &raw, &glyph_kerns, &mut glyphs, report)?;

    let top = |name: &str| {
        glyphs.list.iter().find(|g| g.name == name).and_then(|g| g.layer(MASTER)).and_then(|l| {
            l.shapes
                .iter()
                .filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None })
                .flat_map(|p| p.nodes.iter())
                .filter(|n| n.kind != NodeKind::OffCurve)
                .map(|n| n.y)
                .reduce(f64::max)
        })
    };
    let em = f64::from(units_per_em);
    let cap = top("H").unwrap_or((em * 0.7).round());
    let x_height = top("x").unwrap_or((em * 0.5).round());
    let master_name = english_name(2).or_else(|| h("Weight").map(str::to_string)).unwrap_or_else(|| "Regular".into());
    check_name("master", &master_name)?;
    {
        let masters = Arc::make_mut(&mut font.masters);
        if let Some(m) = masters.first_mut() {
            m.name = master_name;
            // Font::new defines ascender, cap height, x-height, baseline and descender, in this order.
            m.metric_values = [ascent, cap, x_height, 0.0, -descent].iter().map(|&pos| MetricValue { pos, over: 0.0 }).collect();
            m.params = master_params(&h, &hn, report);
        }
    }
    if let Some(angle) = hn("ItalicAngle").filter(|a| *a != 0.0 && a.abs() < 90.0) {
        font.set_metric_pos(MASTER, metric::ITALIC_ANGLE, angle)?;
    }

    let info = Arc::make_mut(&mut font.info);
    info.properties = properties(&raw, &h, &family, report);
    info.params = font_params(&h);
    if let Some(date) = h("CreationTime").and_then(crate::lines::int).and_then(crate::lines::unix_date) {
        info.date = Some(date);
    }
    font.note = match (h("UComments"), h("Comments")) {
        (Some(u), _) => crate::lines::text(u),
        (None, Some(c)) => c.to_string(),
        (None, None) => String::new(),
    };
    font.glyphs = GlyphTable::from_glyphs(glyphs.list.into_iter().map(Arc::new).collect());
    font.check()?;
    Ok(font)
}

fn check_name(what: &str, name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN || name.chars().any(char::is_control) {
        return Err(SfdError::Unsupported(format!("the {what} name `{}` is empty, too long or holds a control character", short(name))));
    }
    Ok(())
}

/// `001.000` → (1, 0); `1.5` → (1, 500): the digits after the point are thousandths.
fn version(v: &str) -> Option<(u16, u16)> {
    let start = v.find(|c: char| c.is_ascii_digit())?;
    let num: String = v.get(start..)?.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let mut parts = num.split('.');
    let major = parts.next()?.parse::<u16>().ok()?;
    let frac: String = parts.next().unwrap_or("").chars().take(3).collect();
    let minor = if frac.is_empty() { 0 } else { format!("{frac:0<3}").parse::<u16>().ok()? };
    Some((major, minor))
}

/// A glyph's kerning pairs: (first glyph, pairs, where the glyph starts).
type GlyphKerns = (String, Vec<RawKern>, String);

/// The glyphs in model form, with their positions by name and by glyph index, for kerning.
struct Glyphs {
    list: Vec<Glyph>,
    by_name: HashMap<String, usize>,
    by_gid: HashMap<i64, usize>,
}

fn build_glyphs(raw: &RawFont, list: Vec<RawGlyph>, upm: u16, report: &mut Report) -> Result<Glyphs> {
    let mut by_name: HashMap<String, usize> = HashMap::new();
    let mut by_gid: HashMap<i64, usize> = HashMap::new();
    let mut by_unicode: HashMap<u32, usize> = HashMap::new();
    for (i, g) in list.iter().enumerate() {
        if by_name.insert(g.name.clone(), i).is_some() {
            return Err(SfdError::Syntax { at: g.at.clone(), message: format!("a second glyph is named `{}`", g.name) });
        }
        by_gid.entry(g.gid.unwrap_or(i as i64)).or_insert(i);
        if let Some(u) = g.unicode {
            by_unicode.entry(u).or_insert(i);
        }
    }
    let names: Vec<String> = list.iter().map(|g| g.name.clone()).collect();
    let resolve = |gid: i64, unicode: i64| -> Option<String> {
        by_gid.get(&gid).or_else(|| u32::try_from(unicode).ok().and_then(|u| by_unicode.get(&u))).and_then(|&i| names.get(i)).cloned()
    };
    let mut out = Vec::with_capacity(list.len());
    for g in list {
        let width = g.width.unwrap_or(f64::from(upm));
        let mut glyph = Glyph::new(g.name.clone());
        glyph.unicodes = g.unicode.into_iter().chain(g.alt_unicodes.iter().copied()).collect();
        let mut master = Layer::new(MASTER, width);
        let mut extra_layers = Vec::new();
        for (n, pieces) in g.layers {
            let shapes = shapes(pieces, &g.name, &resolve, report);
            match (n, raw.layers.get(&n)) {
                (1, _) => master.shapes.extend(shapes),
                (0, _) => {
                    if !shapes.is_empty() {
                        master.info.background = Some(Background { shapes, ..Background::default() });
                    }
                }
                (_, Some(declared)) => {
                    if !shapes.is_empty() {
                        let mut layer = Layer::new(MASTER, width);
                        layer.id = format!("layer{n}");
                        layer.info.name = Some(declared.name.clone()).filter(|s| !s.trim().is_empty() && s.len() <= MAX_NAME_LEN);
                        layer.shapes = shapes;
                        extra_layers.push(layer);
                    }
                }
                // A layer the font does not declare is part of a multi-layered (Type 3) drawing.
                (_, None) => master.shapes.extend(shapes),
            }
        }
        master.anchors = g.anchors.into_iter().map(|(name, x, y)| Anchor::new(name, x, y)).collect();
        if let Some(v) = g.vert_width {
            master.info.vert_width = Some(v);
        }
        glyph.layers.push(master);
        glyph.layers.extend(extra_layers);
        if let Some(note) = g.note.filter(|n| !n.is_empty()) {
            glyph.info.note = note;
        }
        if let Some(c) = g.color {
            glyph.info.color = Some(ColorLabel::Rgba(c));
        }
        if let Some(cat) = g.category {
            glyph.info.opentype_category = Some(cat.to_string());
        }
        out.push(glyph);
    }
    Ok(Glyphs { list: out, by_name, by_gid })
}

/// A layer's shapes, with references resolved to components.
fn shapes(pieces: Vec<Piece>, own: &str, resolve: &dyn Fn(i64, i64) -> Option<String>, report: &mut Report) -> Vec<Shape> {
    let mut out = Vec::with_capacity(pieces.len());
    for p in pieces {
        match p {
            Piece::Path(p) => out.push(Shape::Path(p)),
            Piece::Ref(r) => match resolve(r.gid, r.unicode) {
                Some(name) if name != own => {
                    let [a, b, c, d, e, f] = r.matrix;
                    let mut comp = Component::new(name, 0.0, 0.0);
                    comp.transform = if [a, b, c, d] == [1.0, 0.0, 0.0, 1.0] { Transform::translate(e, f) } else { Transform::Matrix(r.matrix) };
                    out.push(Shape::Component(comp));
                }
                Some(_) => report.skip("references of a glyph to itself", &r.at),
                None => report.skip("references to glyphs that are not in the font", &r.at),
            },
        }
    }
    out
}

/// Kerning in horizontal kerning subtables: glyph pairs first, then class pairs where no pair of
/// the same sides is set yet.
fn kerning(font: &mut Font, raw: &RawFont, glyph_kerns: &[GlyphKerns], glyphs: &mut Glyphs, report: &mut Report) -> Result<()> {
    let mut table = font.kerning.ltr.clone();
    let mut pairs = 0usize;
    let mut set = |table: &mut fontcraft_model::KernTable, first: &str, second: &str, value: f64| -> Result<()> {
        if table.get(MASTER, first, second).is_none() {
            pairs += 1;
            if pairs > MAX_KERN_PAIRS {
                return Err(SfdError::Unsupported(format!("more than {MAX_KERN_PAIRS} kerning pairs")));
            }
            table.set(MASTER, first, second, value);
        }
        Ok(())
    };
    let usable = |sub: &str, report: &mut Report, at: &str| match raw.subtables.get(sub) {
        Some(s) if s.kern => true,
        Some(s) if s.vertical => {
            report.skip("vertical kerning pairs", at);
            false
        }
        Some(_) => {
            report.skip("pair positioning outside the kerning feature", at);
            false
        }
        // Subtables no lookup declares (older files) are taken as kerning.
        None => true,
    };
    for (first, kerns, at) in glyph_kerns {
        for k in kerns {
            if !usable(k.subtable.as_str(), report, at.as_str()) {
                continue;
            }
            let Some(second) = glyphs.by_gid.get(&k.second_gid).and_then(|&i| glyphs.list.get(i)).map(|g| g.name.clone()) else {
                report.skip("kerning pairs with a glyph that is not in the font", at);
                continue;
            };
            set(&mut table, first.as_str(), &second, k.value)?;
        }
    }
    let (mut used_left, mut used_right) = (BTreeSet::new(), BTreeSet::new());
    for kc in &raw.kern_classes {
        if !usable(kc.subtable.as_str(), report, kc.at.as_str()) {
            continue;
        }
        let lefts = class_sides(kc, true, glyphs, &mut used_left, report);
        let rights = class_sides(kc, false, glyphs, &mut used_right, report);
        let n2 = kc.seconds.len();
        for (i, a_keys) in lefts.iter().enumerate() {
            for (j, b_keys) in rights.iter().enumerate() {
                let value = kc.values.get(i * n2 + j).copied().unwrap_or(0.0);
                if value == 0.0 {
                    continue;
                }
                if a_keys.is_empty() || b_keys.is_empty() {
                    if j == 0 || (i == 0 && !kc.first_zero_listed) {
                        report.skip("class kerning against every other glyph (class 0)", &kc.at);
                    }
                    continue;
                }
                for a in a_keys {
                    for b in b_keys {
                        set(&mut table, a.as_str(), b.as_str(), value)?;
                    }
                }
            }
        }
    }
    Arc::make_mut(&mut font.kerning).ltr = table;
    Ok(())
}

/// The kerning sides of each class of one side of a table: a group (`@MMK_L_x`, `@MMK_R_x`) for
/// the members that have no group on that side yet, and the other members by name. Class 0 is
/// empty unless the table listed it.
fn class_sides(kc: &KernClass, first: bool, glyphs: &mut Glyphs, used: &mut BTreeSet<String>, report: &mut Report) -> Vec<Vec<String>> {
    let classes = if first { &kc.firsts } else { &kc.seconds };
    let mut out = Vec::with_capacity(classes.len());
    for (i, members) in classes.iter().enumerate() {
        if i == 0 && !(first && kc.first_zero_listed) {
            out.push(Vec::new());
            continue;
        }
        let mut keys = Vec::new();
        let mut free = Vec::new();
        for m in members {
            match glyphs.by_name.get(m).copied() {
                Some(gi) => {
                    let Some(g) = glyphs.list.get(gi) else { continue };
                    let side = if first { &g.kern_groups.right } else { &g.kern_groups.left };
                    if side.is_some() { keys.push(m.clone()) } else { free.push(gi) }
                }
                None => report.skip("kerning class members that are not in the font", &kc.at),
            }
        }
        if let Some(name) = free.first().and_then(|&gi| glyphs.list.get(gi)).map(|g| g.name.clone()) {
            let mut group = name.clone();
            let mut n = 2usize;
            while used.contains(&group) {
                group = format!("{name}_{n}");
                n += 1;
            }
            used.insert(group.clone());
            for &gi in &free {
                if let Some(g) = glyphs.list.get_mut(gi) {
                    let slot = if first { &mut g.kern_groups.right } else { &mut g.kern_groups.left };
                    *slot = Some(group.clone());
                }
            }
            keys.push(if first { format!("@MMK_L_{group}") } else { format!("@MMK_R_{group}") });
        }
        out.push(keys);
    }
    out
}

fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::Number(Number::int(v as i64))
    } else {
        Number::float(v).map_or(Value::Number(Number::int(0)), Value::Number)
    }
}

fn int_list(items: impl IntoIterator<Item = i64>) -> Value {
    Value::Array(Array { items: items.into_iter().map(|i| Value::Number(Number::int(i))).collect(), ..Array::default() })
}

fn master_params<'a>(h: &dyn Fn(&str) -> Option<&'a str>, hn: &dyn Fn(&str) -> Option<f64>, report: &mut Report) -> fontcraft_model::Params {
    let mut params = fontcraft_model::Params::default();
    for (key, offset, param) in MASTER_PARAMS {
        let Some(v) = hn(key) else { continue };
        let relative = !offset.is_empty() && h(offset).is_some_and(|o| o == "1");
        if relative {
            if v != 0.0 {
                report.skip("vertical metrics given relative to a computed value", "the font header");
            }
            continue;
        }
        params.0.push(Param::new(*param, number(v)));
    }
    params
}

fn font_params<'a>(h: &dyn Fn(&str) -> Option<&'a str>) -> fontcraft_model::Params {
    let mut params = fontcraft_model::Params::default();
    if let Some(fs) = h("FSType").and_then(crate::lines::int).filter(|v| (0..=0xFFFF).contains(v)) {
        params.0.push(Param::new("fsType", int_list((0..16).filter(|b| fs & (1 << b) != 0))));
    }
    if let Some(p) = h("Panose") {
        let digits: Vec<i64> = p.split_whitespace().filter_map(crate::lines::int).filter(|d| (0..=255).contains(d)).collect();
        if digits.len() == 10 {
            params.0.push(Param::new("panose", int_list(digits)));
        }
    }
    if h("OS2_UseTypoMetrics") == Some("1") {
        params.0.push(Param::new("Use Typo Metrics", Value::Bool(true)));
    }
    params
}

fn properties<'a>(raw: &RawFont, h: &dyn Fn(&str) -> Option<&'a str>, family: &str, report: &mut Report) -> Properties {
    let mut props = Properties::default();
    if let Some(c) = h("Copyright") {
        props.set("copyrights", c, true);
    }
    if let Some(n) = h("FontName") {
        props.set("postscriptFontName", n, false);
    }
    if let Some(n) = h("FullName") {
        props.set("postscriptFullName", n, false);
    }
    if let Some(v) = h("OS2Vendor").map(|v| v.trim_matches('\'').trim()).filter(|v| !v.is_empty() && v.len() <= 4) {
        props.set("vendorID", v, false);
    }
    let tags: BTreeMap<u16, &str> = LANGUAGES.iter().map(|(t, id)| (*id, *t)).collect();
    for (&lang, strings) in &raw.lang_names {
        let tag = if lang == US_ENGLISH {
            DEFAULT_LANGUAGE
        } else if let Some(t) = tags.get(&lang) {
            t
        } else {
            report.skip("names in a language with no OpenType language tag", "the font header");
            continue;
        };
        for (id, value) in strings.iter().enumerate() {
            if value.is_empty() {
                continue;
            }
            match (id, NAME_IDS.iter().find(|(i, _, _)| *i == id)) {
                (1, _) if tag != DEFAULT_LANGUAGE => localized(&mut props, "familyNames", tag, value, Some(family)),
                (1 | 2 | 4, _) => {
                    if tag != DEFAULT_LANGUAGE && id != 4 {
                        report.skip("localised style names", "the font header");
                    }
                }
                (_, Some((_, key, true))) => localized(&mut props, key, tag, value, None),
                (_, Some((_, key, false))) if tag == DEFAULT_LANGUAGE => props.set(key, value, false),
                (_, Some(_)) => report.skip("localised values of names that have one value for all languages", "the font header"),
                (_, None) => report.skip("name table entries with no property", "the font header"),
            }
        }
    }
    props
}

/// Sets `key`'s value for `tag`; a new property starts with `default` as its default value.
fn localized(props: &mut Properties, key: &str, tag: &str, value: &str, default: Option<&str>) {
    let loc = |language: &str, value: &str| Localized { language: language.into(), value: value.into(), extra: Default::default() };
    let p = match props.0.iter().position(|p| p.key == key) {
        Some(i) => props.0.get_mut(i),
        None => {
            let first = default.map(|d| vec![loc(DEFAULT_LANGUAGE, d)]).unwrap_or_default();
            props.0.push(Property { key: key.into(), value: PropertyValue::Localized(first), extra: Default::default() });
            props.0.last_mut()
        }
    };
    let Some(p) = p else { return };
    match &mut p.value {
        PropertyValue::Localized(v) => match v.iter_mut().find(|l| l.language == tag) {
            Some(l) => l.value = value.into(),
            None => v.push(loc(tag, value)),
        },
        PropertyValue::Single(s) if tag == DEFAULT_LANGUAGE => *s = value.into(),
        PropertyValue::Single(_) => {}
    }
}

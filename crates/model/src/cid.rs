//! CID-keyed fonts: the character collection, the glyph-to-CID map and the sub-fonts (font
//! dictionaries), kept in the lib keys and groups that AFDKO reads and writes for CID-keyed UFOs
//! (`docs/CIDKeyedUFOGuide.md` in AFDKO; `tx -ufo` writes the same layout).
//!
//! | Data | Where it is kept |
//! |---|---|
//! | the character collection | [`ROS_KEY`]: `Registry-Ordering-Supplement`, such as `Adobe-Identity-0` |
//! | the CID font's name | [`CID_FONT_NAME_KEY`], optional |
//! | each glyph's CID | [`CID_MAP_KEY`]: a dictionary of glyph name → integer |
//! | the sub-fonts, in index order | [`FD_ARRAY_KEY`]: an array of dictionaries with `FontName` and, optionally, `PrivateDict` (the sub-font's hinting values under UFO font-info names) |
//! | the glyphs of each sub-font | the font's groups `FDArraySelect.<index>` or `FDArraySelect.<index>.<name>` |
//!
//! A UFO reader puts lib keys it has no field for into the master's user data, so the keys are
//! read from the first master's user data, else from the font's user data. [`CidFont::store`]
//! writes them into every master's user data, so each master's UFO is a CID-keyed UFO on its own,
//! and takes them out of the font's user data. A font is CID-keyed when [`ROS_KEY`] is there.
//!
//! What is stored is untrusted: [`CidFont::read`] caps every count, and a malformed key makes it
//! fail with a message instead of guessing. Two glyphs with one CID, a glyph in two sub-fonts and
//! a glyph in none are read as they are; `check.cid.definedTwiceOrMissing` reports them.

use std::collections::BTreeMap;
use std::sync::Arc;

use fontcraft_plist::{Array, Dict, Num, Number, Str, Value};

use crate::{Font, Group, MAX_GLYPHS};

pub const ROS_KEY: &str = "com.adobe.type.ROS"; // brand-ok: the file-format key
pub const CID_FONT_NAME_KEY: &str = "com.adobe.type.CIDFontName"; // brand-ok: the file-format key
pub const CID_MAP_KEY: &str = "com.adobe.type.postscriptCIDMap"; // brand-ok: the file-format key
pub const FD_ARRAY_KEY: &str = "com.adobe.type.postscriptFDArray"; // brand-ok: the file-format key
/// The name of a sub-font's group starts with this, then the index.
pub const FD_SELECT_PREFIX: &str = "FDArraySelect.";

/// Highest CID: a CID-keyed CFF counts at most 65,535 CIDs, from 0.
pub const MAX_CID: u32 = 65_534;
/// Most sub-fonts: the FDSelect of a CFF table stores a sub-font's index in one byte.
pub const MAX_SUBFONTS: usize = 256;
/// Longest registry or ordering, in bytes.
pub const MAX_ROS_PART: usize = 64;
/// Largest supplement number.
pub const MAX_SUPPLEMENT: u32 = 65_535;
/// Longest sub-font or CID font name, in bytes (the PostScript limit for a font name).
pub const MAX_FONT_NAME: usize = 127;

/// A character collection: registry, ordering and supplement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ros {
    pub registry: String,
    pub ordering: String,
    pub supplement: u32,
}

impl Ros {
    /// Reads `Registry-Ordering-Supplement`. The supplement is the part after the last hyphen and
    /// the registry the part before the first, so neither may contain one.
    pub fn parse(text: &str) -> Option<Ros> {
        let (rest, supplement) = text.trim().rsplit_once('-')?;
        let (registry, ordering) = rest.split_once('-')?;
        let supplement: u32 = supplement.parse().ok().filter(|s| *s <= MAX_SUPPLEMENT)?;
        (part_ok(registry) && part_ok(ordering)).then(|| Ros { registry: registry.to_string(), ordering: ordering.to_string(), supplement })
    }

    pub fn text(&self) -> String {
        format!("{}-{}-{}", self.registry, self.ordering, self.supplement)
    }

    /// Is this the Identity ordering, whose CIDs are not tied to any characters?
    pub fn is_identity(&self) -> bool {
        self.ordering == "Identity"
    }
}

/// Can `s` be a registry or an ordering? 1 to [`MAX_ROS_PART`] ASCII letters, digits or `_`;
/// a hyphen would make the joined form ambiguous.
pub fn part_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_ROS_PART && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Can `s` be a sub-font or CID font name? 1 to [`MAX_FONT_NAME`] printable ASCII characters
/// without spaces or the PostScript delimiters `()<>[]{}/%`.
pub fn font_name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_FONT_NAME && s.bytes().all(|b| b.is_ascii_graphic() && !b"()<>[]{}/%".contains(&b))
}

/// `name` with every character a font name may not hold left out, at most [`MAX_FONT_NAME`]
/// bytes; `None` when nothing is left.
pub fn font_name_from(name: &str) -> Option<String> {
    let kept: String = name.chars().filter(|c| c.is_ascii_graphic() && !"()<>[]{}/%".contains(*c)).take(MAX_FONT_NAME).collect();
    (!kept.is_empty()).then_some(kept)
}

/// One sub-font: a font dictionary of the CID-keyed font and the glyphs drawn with it.
#[derive(Clone, Debug, PartialEq)]
pub struct SubFont {
    pub name: String,
    /// The stored dictionary as read (`FontName`, `PrivateDict` and anything else), written back
    /// with `FontName` set to [`SubFont::name`].
    pub dict: Dict,
    /// Glyph names in the group's order.
    pub glyphs: Vec<String>,
}

impl SubFont {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), dict: Dict::new(), glyphs: Vec::new() }
    }

    /// The sub-font's hinting values, under UFO font-info names (`postscriptBlueValues`…).
    pub fn private(&self) -> Option<&Dict> {
        self.dict.get("PrivateDict").and_then(Value::as_dict)
    }

    /// Sets (or, with `None`, removes) one hinting value.
    pub fn set_private(&mut self, key: &str, value: Option<Value>) {
        let mut private = self.private().cloned().unwrap_or_default();
        match value {
            Some(v) => private.insert(key, v),
            None => {
                if let Some(i) = private.entries.iter().position(|(k, _)| k.as_str() == key) {
                    private.remove_at(i);
                }
            }
        }
        match self.dict.entries.iter().position(|(k, _)| k.as_str() == "PrivateDict") {
            Some(i) if private.is_empty() => {
                self.dict.remove_at(i);
            }
            _ if private.is_empty() => {}
            _ => self.dict.insert("PrivateDict", Value::Dict(private)),
        }
    }
}

/// The CID-keyed state of a font.
#[derive(Clone, Debug, PartialEq)]
pub struct CidFont {
    pub ros: Ros,
    pub font_name: Option<String>,
    /// Glyph name → CID, in stored order. A name or a CID may appear twice in malformed data.
    pub cids: Vec<(String, u32)>,
    /// In index order: sub-font `i` is the group `FDArraySelect.i`.
    pub subfonts: Vec<SubFont>,
}

/// Where the keys were found.
fn source(font: &Font) -> Option<&Dict> {
    if let Some(m) = font.masters.first()
        && m.user_data.get(ROS_KEY).is_some()
    {
        return Some(&m.user_data.0);
    }
    font.info.user_data.get(ROS_KEY).map(|_| &font.info.user_data.0)
}

/// A whole number from a stored value: an integer, an integral float or digits as text.
fn whole(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => match n.value() {
            Num::Int(i) => Some(i),
            Num::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 => Some(f as i64),
            Num::Float(_) => None,
        },
        Value::String(s) => s.as_str().trim().parse().ok(),
        _ => None,
    }
}

/// The index and label of a sub-font group's name, `None` for any other group.
fn fd_group(name: &str) -> Option<Result<(usize, Option<&str>), String>> {
    let rest = name.strip_prefix(FD_SELECT_PREFIX)?;
    let (index, label) = match rest.split_once('.') {
        Some((i, l)) => (i, Some(l)),
        None => (rest, None),
    };
    Some(match index.parse::<usize>() {
        Ok(i) if i < MAX_SUBFONTS => Ok((i, label.filter(|l| !l.is_empty()))),
        _ => Err(format!("the group `{name}` does not name a sub-font index from 0 to {}", MAX_SUBFONTS - 1)),
    })
}

/// Is `font` CID-keyed?
pub fn is_cid_keyed(font: &Font) -> bool {
    source(font).is_some()
}

impl CidFont {
    /// The CID-keyed state of `font`: `Ok(None)` for an ordinary font, an error naming the first
    /// malformed key.
    pub fn read(font: &Font) -> Result<Option<CidFont>, String> {
        let Some(data) = source(font) else { return Ok(None) };
        let ros_text = data.get(ROS_KEY).and_then(Value::as_str).unwrap_or_default();
        let ros = Ros::parse(ros_text).ok_or_else(|| format!("`{ROS_KEY}` is `{ros_text}`, not Registry-Ordering-Supplement"))?;
        let font_name = data.get(CID_FONT_NAME_KEY).and_then(Value::as_str).map(str::to_string);
        let mut cids = Vec::new();
        match data.get(CID_MAP_KEY) {
            None => {}
            Some(Value::Dict(map)) => {
                if map.len() > MAX_GLYPHS {
                    return Err(format!("`{CID_MAP_KEY}` maps {} glyphs; at most {MAX_GLYPHS} are read", map.len()));
                }
                for (name, v) in &map.entries {
                    let cid = whole(v)
                        .and_then(|c| u32::try_from(c).ok())
                        .filter(|c| *c <= MAX_CID)
                        .ok_or_else(|| format!("`{CID_MAP_KEY}` gives the glyph `{}` no CID from 0 to {MAX_CID}", name.as_str()))?;
                    cids.push((name.as_str().to_string(), cid));
                }
            }
            Some(_) => return Err(format!("`{CID_MAP_KEY}` is not a dictionary")),
        }
        let mut subfonts: Vec<SubFont> = Vec::new();
        match data.get(FD_ARRAY_KEY) {
            None => {}
            Some(Value::Array(a)) => {
                if a.items.len() > MAX_SUBFONTS {
                    return Err(format!("`{FD_ARRAY_KEY}` lists {} sub-fonts; at most {MAX_SUBFONTS} are allowed", a.items.len()));
                }
                for (i, item) in a.items.iter().enumerate() {
                    let dict = item.as_dict().ok_or_else(|| format!("entry {i} of `{FD_ARRAY_KEY}` is not a dictionary"))?;
                    let name = dict.get("FontName").and_then(Value::as_str).map_or_else(|| format!("FD{i}"), str::to_string);
                    subfonts.push(SubFont { name, dict: dict.clone(), glyphs: Vec::new() });
                }
            }
            Some(_) => return Err(format!("`{FD_ARRAY_KEY}` is not an array")),
        }
        let named = subfonts.len();
        for g in font.groups.iter() {
            let Some(parsed) = fd_group(&g.name) else { continue };
            let (index, label) = parsed?;
            while subfonts.len() <= index {
                let i = subfonts.len();
                subfonts.push(SubFont::new(format!("FD{i}")));
            }
            if let Some(sub) = subfonts.get_mut(index) {
                if index >= named
                    && let Some(label) = label
                {
                    sub.name = label.to_string();
                }
                sub.glyphs.extend(g.members.iter().take(MAX_GLYPHS).cloned());
            }
        }
        Ok(Some(CidFont { ros, font_name, cids, subfonts }))
    }

    /// The CID of `glyph`: its first entry in the map.
    pub fn cid_of(&self, glyph: &str) -> Option<u32> {
        self.cids.iter().find(|(n, _)| n == glyph).map(|(_, c)| *c)
    }

    /// Glyph name → CID, the first entry of each name.
    pub fn cid_map(&self) -> BTreeMap<&str, u32> {
        let mut out = BTreeMap::new();
        for (n, c) in &self.cids {
            out.entry(n.as_str()).or_insert(*c);
        }
        out
    }

    /// The index of the first sub-font that lists `glyph`.
    pub fn subfont_of(&self, glyph: &str) -> Option<usize> {
        self.subfonts.iter().position(|s| s.glyphs.iter().any(|g| g == glyph))
    }

    pub fn subfont_index(&self, name: &str) -> Option<usize> {
        self.subfonts.iter().position(|s| s.name == name)
    }

    pub fn max_cid(&self) -> Option<u32> {
        self.cids.iter().map(|(_, c)| *c).max()
    }

    /// Writes the state into every master's user data and the font's groups, replacing what was
    /// there. The CID map is written in CID order.
    pub fn store(&self, font: &mut Font) {
        let ros = Value::String(Str::new(self.ros.text()));
        let mut sorted: Vec<&(String, u32)> = self.cids.iter().collect();
        sorted.sort_by_key(|(_, c)| *c);
        let mut map = Dict::new();
        map.entries = sorted.into_iter().map(|(n, c)| (Str::new(n.as_str()), Value::Number(Number::int(i64::from(*c))))).collect();
        let fd_array = Value::Array(Array {
            items: self
                .subfonts
                .iter()
                .map(|s| {
                    let mut d = s.dict.clone();
                    match d.entries.iter().position(|(k, _)| k.as_str() == "FontName") {
                        Some(_) => d.insert("FontName", Value::String(Str::new(s.name.as_str()))),
                        None => d.insert_at(0, "FontName", Value::String(Str::new(s.name.as_str()))),
                    }
                    Value::Dict(d)
                })
                .collect(),
            ..Default::default()
        });
        clear_keys(&mut Arc::make_mut(&mut font.info).user_data.0);
        for m in Arc::make_mut(&mut font.masters) {
            let data = &mut m.user_data.0;
            data.insert(ROS_KEY, ros.clone());
            match &self.font_name {
                Some(n) => data.insert(CID_FONT_NAME_KEY, Value::String(Str::new(n.as_str()))),
                None => remove_key(data, CID_FONT_NAME_KEY),
            }
            data.insert(CID_MAP_KEY, Value::Dict(map.clone()));
            data.insert(FD_ARRAY_KEY, fd_array.clone());
        }
        let groups =
            self.subfonts.iter().enumerate().map(|(i, s)| Group { name: format!("{FD_SELECT_PREFIX}{i}.{}", s.name), members: s.glyphs.clone() });
        replace_fd_groups(font, groups.collect());
    }

    /// Makes `font` an ordinary font again: removes the keys and the sub-font groups.
    pub fn remove(font: &mut Font) {
        clear_keys(&mut Arc::make_mut(&mut font.info).user_data.0);
        for m in Arc::make_mut(&mut font.masters) {
            clear_keys(&mut m.user_data.0);
        }
        replace_fd_groups(font, Vec::new());
    }
}

fn remove_key(data: &mut Dict, key: &str) {
    if let Some(i) = data.entries.iter().position(|(k, _)| k.as_str() == key) {
        data.remove_at(i);
    }
}

fn clear_keys(data: &mut Dict) {
    for key in [ROS_KEY, CID_FONT_NAME_KEY, CID_MAP_KEY, FD_ARRAY_KEY] {
        remove_key(data, key);
    }
}

/// Puts `groups` where the first sub-font group was (at the end when there was none) and drops
/// the other sub-font groups.
fn replace_fd_groups(font: &mut Font, groups: Vec<Group>) {
    let at = font.groups.iter().position(|g| g.name.starts_with(FD_SELECT_PREFIX));
    if at.is_none() && groups.is_empty() {
        return;
    }
    let list = Arc::make_mut(&mut font.groups);
    list.retain(|g| !g.name.starts_with(FD_SELECT_PREFIX));
    let at = at.unwrap_or(list.len()).min(list.len());
    for (k, g) in groups.into_iter().enumerate() {
        list.insert(at.saturating_add(k).min(list.len()), g);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn ros_text_reads_and_writes_the_three_parts() {
        let ros = Ros::parse("Adobe-Japan1-6").unwrap();
        assert_eq!((ros.registry.as_str(), ros.ordering.as_str(), ros.supplement), ("Adobe", "Japan1", 6));
        assert_eq!(ros.text(), "Adobe-Japan1-6");
        for bad in ["Adobe-Japan1", "Adobe--0", "-Identity-0", "Adobe-Identity-x", "Adobe-Iden-tity-0", "Adobe-Identity-70000"] {
            assert_eq!(Ros::parse(bad), None, "{bad}");
        }
    }

    /// The layout `tx -ufo` writes for a CID-keyed font with two font dictionaries (AFDKO 4.0):
    /// the keys in the master's user data, the groups `FDArraySelect.<index>.<name>`.
    #[test]
    fn the_afdko_layout_reads_and_stores_back_the_same() {
        let mut font = Font::new("Cid");
        let mut map = Dict::new();
        for (n, c) in [("cid00000", 0), ("cid00001", 1), ("cid00002", 2), ("cid00005", 5)] {
            map.insert(n, Value::Number(Number::int(c)));
        }
        let fd = |name: &str| {
            let mut private = Dict::new();
            private.insert("postscriptBlueFuzz", Value::Number(Number::int(1)));
            let mut d = Dict::new();
            d.insert("FontName", Value::String(Str::new(name)));
            d.insert("PaintType", Value::Number(Number::int(0)));
            d.insert("PrivateDict", Value::Dict(private));
            Value::Dict(d)
        };
        let data = &mut Arc::make_mut(&mut font.masters)[0].user_data.0;
        data.insert(CID_FONT_NAME_KEY, Value::String(Str::new("Cidtest-Regular")));
        data.insert(ROS_KEY, Value::String(Str::new("Adobe-Identity-0")));
        data.insert(CID_MAP_KEY, Value::Dict(map));
        data.insert(FD_ARRAY_KEY, Value::Array(Array { items: vec![fd("Latin"), fd("Han")], ..Default::default() }));
        let groups = Arc::make_mut(&mut font.groups);
        groups.push(Group { name: "FDArraySelect.0.Latin".into(), members: vec!["cid00000".into(), "cid00001".into(), "cid00002".into()] });
        groups.push(Group { name: "FDArraySelect.1.Han".into(), members: vec!["cid00005".into()] });

        let cid = CidFont::read(&font).unwrap().unwrap();
        assert_eq!(cid.ros.text(), "Adobe-Identity-0");
        assert_eq!(cid.font_name.as_deref(), Some("Cidtest-Regular"));
        assert_eq!(cid.cid_of("cid00005"), Some(5));
        assert_eq!(cid.subfonts.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Latin", "Han"]);
        assert_eq!(cid.subfont_of("cid00005"), Some(1));
        assert!(cid.subfonts[1].private().is_some());

        let mut stored = font.clone();
        cid.store(&mut stored);
        assert_eq!(CidFont::read(&stored).unwrap().unwrap(), cid);
        assert_eq!(stored.groups, font.groups);

        CidFont::remove(&mut stored);
        assert!(!is_cid_keyed(&stored));
        assert!(stored.groups.is_empty());
    }

    #[test]
    fn malformed_keys_are_refused_with_the_key_named() {
        let mut font = Font::new("Cid");
        let data = &mut Arc::make_mut(&mut font.info).user_data.0;
        data.insert(ROS_KEY, Value::String(Str::new("Adobe-Identity-0")));
        let mut map = Dict::new();
        map.insert("A", Value::Number(Number::int(70_000)));
        data.insert(CID_MAP_KEY, Value::Dict(map));
        let e = CidFont::read(&font).unwrap_err();
        assert!(e.contains("`A`"), "{e}");
        Arc::make_mut(&mut font.info).user_data.0.insert(CID_MAP_KEY, Value::Dict(Dict::new()));
        Arc::make_mut(&mut font.groups).push(Group { name: "FDArraySelect.999".into(), members: vec![] });
        assert!(CidFont::read(&font).unwrap_err().contains("FDArraySelect.999"));
    }
}

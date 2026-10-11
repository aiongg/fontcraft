//! Glyph names for the normalised text, chosen as the oracle chooses them: the `post` name, else
//! the AGLFN name of the glyph's lowest Unicode code point, else `uniXXXX` or `uXXXXX`, else
//! `glyph.NNNNN`. Glyph 0 is always `.notdef`.
//!
//! Ported from fontc's `otl-normalizer` (MIT OR Apache-2.0). Only the Unicode-platform `cmap`
//! subtables (platform 0, formats 4 and 12) are read, as the oracle reads them.

use std::borrow::Cow;
use std::collections::HashMap;

use fontcraft_model::glyphdata::GlyphData;
use read_fonts::TableProvider;
use read_fonts::tables::cmap::{CmapSubtable, PlatformId};
use read_fonts::tables::post::Post;
use read_fonts::types::{GlyphId16, Version16Dot16};

use crate::limits::MAX_CODE_POINTS;
use crate::{Result, TablesError};

/// The names of the standard Macintosh glyph set that `post` version 2 indices below this refer
/// to.
const STANDARD_NAMES: usize = 258;

/// AGLFN names whose glyph data record carries another name (AGLFN, Adobe, BSD-3-Clause).
const AGLFN_EXTRA: [(u32, &str); 3] = [(0x00B5, "mu"), (0x2126, "Omega"), (0x2206, "Delta")];

/// A name for every glyph of the font.
pub(super) struct NameMap {
    names: Vec<String>,
}

impl NameMap {
    pub(super) fn from_font<'a>(font: &impl TableProvider<'a>) -> Result<Self> {
        let num_glyphs = font.maxp().map_err(|_| TablesError::MissingTable("maxp".into()))?.num_glyphs();
        let reverse = reverse_cmap(font);
        let post = font.post().ok().map(PostNames::new);
        let glyph_data = GlyphData::new().ok();
        let mut names = Vec::with_capacity(usize::from(num_glyphs).max(1));
        names.push(".notdef".to_string());
        for gid in 1..num_glyphs {
            let gid = GlyphId16::new(gid);
            if let Some(name) = post.as_ref().and_then(|p| p.name(gid)) {
                names.push(name.to_string());
                continue;
            }
            let name = match reverse.get(&gid).and_then(|cp| char::from_u32(*cp)) {
                Some(c) => aglfn_name(glyph_data.as_ref(), c).unwrap_or_else(|| {
                    let raw = u32::from(c);
                    if raw <= 0xFFFF { format!("uni{raw:04X}") } else { format!("u{raw:X}") }
                }),
                None => numbered(gid),
            };
            names.push(name);
        }
        Ok(NameMap { names })
    }

    /// The name of `gid`; a glyph past the end of the font gets its number, where the oracle
    /// would stop.
    pub(super) fn get(&self, gid: GlyphId16) -> Cow<'_, str> {
        match self.names.get(usize::from(gid.to_u16())) {
            Some(name) => Cow::Borrowed(name.as_str()),
            None => Cow::Owned(numbered(gid)),
        }
    }
}

fn numbered(gid: GlyphId16) -> String {
    format!("glyph.{:05}", gid.to_u16())
}

/// The AGLFN name of `c`, when it has one.
fn aglfn_name(data: Option<&GlyphData>, c: char) -> Option<String> {
    let cp = u32::from(c);
    if let Some((_, name)) = AGLFN_EXTRA.iter().find(|(p, _)| *p == cp) {
        return Some((*name).to_string());
    }
    let data = data?;
    let record = data.by_code_point(cp)?;
    let want = c.to_string();
    std::iter::once(record.name.as_str())
        .chain(record.production.as_deref())
        .chain(record.alt_names.iter().map(String::as_str))
        .find(|name| data.is_aglfn_name(name) && data.name_to_text(name) == want)
        .map(str::to_string)
}

/// Glyph id to the lowest code point mapped to it by the Unicode-platform subtables.
fn reverse_cmap<'a>(font: &impl TableProvider<'a>) -> HashMap<GlyphId16, u32> {
    let mut reverse: HashMap<GlyphId16, u32> = HashMap::new();
    let Ok(cmap) = font.cmap() else { return reverse };
    let data = cmap.offset_data();
    let mut add = |cp: u32, gid: GlyphId16| {
        let slot = reverse.entry(gid).or_insert(cp);
        *slot = cp.min(*slot);
    };
    for record in cmap.encoding_records().iter().filter(|r| r.platform_id() == PlatformId::Unicode) {
        let Ok(subtable) = record.subtable(data) else { continue };
        match subtable {
            CmapSubtable::Format4(sub) => {
                for (cp, gid) in sub.iter().take(MAX_CODE_POINTS) {
                    if let Ok(gid) = GlyphId16::try_from(gid) {
                        add(cp, gid);
                    }
                }
            }
            CmapSubtable::Format12(sub) => {
                for (cp, gid) in sub.iter().take(MAX_CODE_POINTS) {
                    if let Ok(gid) = GlyphId16::try_from(gid) {
                        add(cp, gid);
                    }
                }
            }
            _ => {}
        }
    }
    reverse
}

/// The names of a `post` table, with the version 2 strings read once.
struct PostNames<'a> {
    post: Post<'a>,
    strings: Vec<Option<&'a str>>,
}

impl<'a> PostNames<'a> {
    fn new(post: Post<'a>) -> Self {
        let mut strings = Vec::new();
        if post.version() == Version16Dot16::VERSION_2_0 {
            let bytes = post.offset_data().as_bytes().get(post.string_data_byte_range()).unwrap_or_default();
            // Indices are 16-bit, so no name past this one can be referred to.
            let most = usize::from(u16::MAX).saturating_sub(STANDARD_NAMES).saturating_add(1);
            let mut pos = 0usize;
            while strings.len() < most {
                let Some(&len) = bytes.get(pos) else { break };
                let start = pos.saturating_add(1);
                let end = start.saturating_add(usize::from(len));
                let Some(raw) = bytes.get(start..end) else { break };
                strings.push(if raw.is_ascii() { std::str::from_utf8(raw).ok() } else { None });
                pos = end;
            }
        }
        PostNames { post, strings }
    }

    fn name(&self, gid: GlyphId16) -> Option<&'a str> {
        let version = self.post.version();
        if version == Version16Dot16::VERSION_1_0 {
            return self.post.glyph_name(gid);
        }
        if version != Version16Dot16::VERSION_2_0 {
            return None;
        }
        let index = usize::from(self.post.glyph_name_index()?.get(usize::from(gid.to_u16()))?.get());
        if index < STANDARD_NAMES {
            return self.post.glyph_name(gid);
        }
        self.strings.get(index - STANDARD_NAMES).copied().flatten()
    }
}

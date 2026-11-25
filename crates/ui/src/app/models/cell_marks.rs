//! What a Font View cell shows of its glyph's state besides the drawing (`fontView.cell.*`): the colour labels, the
//! badges, the Unicode mark of the name line and the note its tooltip starts with.
//!
//! Custom glyph info and out-of-sync metrics keys come from the session's caches, which look again only at the glyphs
//! an edit touched and the glyphs whose keys read them.

use std::collections::HashSet;

use craft::ui::contract::PlainData;
use fontcraft_engine::Session;
use fontcraft_engine::model::{ColorLabel, Font, Glyph};
use fontcraft_engine::spacing_state::SessionSpacingExt as _;

use crate::panels::context_menu::LabelTokens;

/// Most characters of a glyph note a tooltip holds.
pub const MAX_NOTE_CHARS: usize = 400;

/// One cell's marks.
#[derive(PlainData, Clone, Debug, Default, PartialEq, Eq)]
pub struct CellMarks {
    /// The glyph's colour label, and that of its layer of Font View's master.
    pub color: Option<[u8; 3]>,
    pub layer_color: Option<[u8; 3]>,
    /// The layer of Font View's master has an annotation.
    pub annotated: bool,
    /// The glyph stores properties that differ from the glyph database's.
    pub custom: bool,
    /// The glyph does not export.
    pub not_exported: bool,
    /// A metrics key of the glyph, on any layer, no longer matches its value.
    pub out_of_sync: bool,
    pub unicode: UnicodeMark,
    /// The start of the glyph's note.
    pub note: String,
}

impl CellMarks {
    /// The marks of `glyph` in `master`; `custom` and `out_of_sync` as the session's caches have them.
    pub fn of(glyph: &Glyph, master: &str, custom: bool, out_of_sync: bool, display_value: bool) -> CellMarks {
        let layer = glyph.layer(master);
        CellMarks {
            color: glyph.info.color.map(label_rgb),
            layer_color: layer.and_then(|l| l.info.color).map(label_rgb),
            annotated: layer.is_some_and(|l| !l.info.annotations.is_empty()),
            custom,
            not_exported: !glyph.export,
            out_of_sync,
            unicode: UnicodeMark::of(glyph, display_value),
            note: glyph.info.note.trim().chars().take(MAX_NOTE_CHARS).collect(),
        }
    }

    /// Whether a badge or a colour label shows.
    pub fn any(&self) -> bool {
        self.color.is_some() || self.layer_color.is_some() || self.annotated || self.custom || self.not_exported || self.out_of_sync
    }
}

/// What stands for the glyph's first code point at the end of the name line.
#[derive(PlainData, Clone, Debug, Default, PartialEq, Eq)]
pub enum UnicodeMark {
    /// No code point.
    #[default]
    None,
    Marker,
    /// A CJK character, drawn in the interface font; the marker when the font lacks it.
    Char(char),
    /// The code point as four to six hexadecimal digits (`appearance.displayUnicodeValue`).
    Value(String),
}

impl UnicodeMark {
    pub fn of(glyph: &Glyph, display_value: bool) -> UnicodeMark {
        let Some(&cp) = glyph.unicodes.first() else { return UnicodeMark::None };
        if display_value {
            return UnicodeMark::Value(format!("{cp:04X}"));
        }
        match char::from_u32(cp) {
            Some(c) if is_cjk(cp) => UnicodeMark::Char(c),
            _ => UnicodeMark::Marker,
        }
    }
}

/// The scripts whose cells show the character itself: Han, Hiragana, Katakana, Hangul and Bopomofo.
pub fn is_cjk(cp: u32) -> bool {
    matches!(cp,
        0x1100..=0x11FF | 0x2E80..=0x2FDF | 0x3040..=0x30FF | 0x3100..=0x318F | 0x31A0..=0x31BF | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}

/// The shade of a colour label.
pub fn label_rgb(label: ColorLabel) -> [u8; 3] {
    let tokens = LabelTokens::standard();
    match label {
        ColorLabel::Index(i) => tokens.colors.get(usize::from(i)).copied().unwrap_or(tokens.unknown),
        ColorLabel::Rgba([r, g, b, _]) => [r, g, b].map(|v| if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 }),
    }
}

/// The glyphs with a metrics key that is out of sync on any layer the scan covers. Unlinked keys and keys in error do
/// not count.
pub fn out_of_sync(session: &mut Session, font: &Font) -> HashSet<String> {
    let Ok(sync) = session.metrics_sync() else { return HashSet::new() };
    sync.all(font).filter(|f| matches!(f.problem, fontcraft_spacing::Problem::OutOfSync { .. })).map(|f| f.glyph.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_follows_the_first_code_point_and_the_setting() {
        let glyph = |unicodes: Vec<u32>| Glyph { unicodes, ..Glyph::new("x") };
        assert_eq!(UnicodeMark::of(&glyph(vec![]), true), UnicodeMark::None);
        assert_eq!(UnicodeMark::of(&glyph(vec![0x41, 0x61]), false), UnicodeMark::Marker);
        assert_eq!(UnicodeMark::of(&glyph(vec![0x41, 0x61]), true), UnicodeMark::Value("0041".into()));
        assert_eq!(UnicodeMark::of(&glyph(vec![0x5B57]), false), UnicodeMark::Char('字'));
        assert_eq!(UnicodeMark::of(&glyph(vec![0x5B57]), true), UnicodeMark::Value("5B57".into()));
    }
}

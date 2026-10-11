//! Unicode properties the kern writer reads: Script_Extensions and Bidi_Class, with the
//! Unicode 18 changes laid over the ICU4X 2.1 data, as fontc reads them.
//!
//! Ported from fontc (https://github.com/googlefonts/fontc), `fontdrasil/src/unicode18.rs`,
//! Apache-2.0, at commit 4504578c3f74c2739f7e55cae133bbfe773f6d8c. The overlay tables are fontc's,
//! generated there by comparing `icu_properties` 2.1.2 with the Unicode 18.0.0 data of ICU4X.

use icu_properties::props::{BidiClass, Script};
use icu_properties::script::ScriptWithExtensions;
use icu_properties::{CodePointMapData, PropertyNamesShort};

use crate::properties::{ScriptCode, sc};

type Range<T> = (u32, u32, T);

fn lookup<T: Copy>(cp: u32, ranges: &'static [Range<T>]) -> Option<T> {
    let index = ranges.partition_point(|(start, _, _)| *start <= cp);
    let (_, end, value) = ranges.get(index.checked_sub(1)?)?;
    (cp <= *end).then_some(*value)
}

const SCRIPT_EXTENSIONS: &[Range<&[ScriptCode]>] = &[
    (0x0558, 0x0558, &[sc(b"Armn")]),
    (0x058B, 0x058C, &[sc(b"Armn")]),
    (0x05C8, 0x05C9, &[sc(b"Hebr")]),
    (0x0B53, 0x0B54, &[sc(b"Orya")]),
    (0x0B83, 0x0B83, &[sc(b"Knda"), sc(b"Mlym"), sc(b"Taml"), sc(b"Telu")]),
    (0x1ADE, 0x1ADF, &[sc(b"Zinh")]),
    (0x1AEC, 0x1AF0, &[sc(b"Zinh")]),
    (0x1CF5, 0x1CF6, &[sc(b"Beng")]),
    (0x208F, 0x208F, &[sc(b"Zyyy")]),
    (0x209D, 0x209F, &[sc(b"Latn")]),
    (0x20C2, 0x20C4, &[sc(b"Zyyy")]),
    (0x2E60, 0x2E63, &[sc(b"Zyyy")]),
    (0xA7DD, 0xA7DD, &[sc(b"Latn")]),
    (0xA7E2, 0xA7E2, &[sc(b"Latn")]),
    (0xAB6C, 0xAB6D, &[sc(b"Latn")]),
    (0x107BB, 0x107BF, &[sc(b"Latn")]),
    (0x10EC9, 0x10ECF, &[sc(b"Arab")]),
    (0x10ED9, 0x10EEE, &[sc(b"Arab")]),
    (0x10EF0, 0x10EF9, &[sc(b"Arab")]),
    (0x11B0A, 0x11B0A, &[sc(b"Deva")]),
    (0x11DF0, 0x11DF1, &[sc(b"Beng")]),
    (0x1246F, 0x1246F, &[sc(b"Xsux")]),
    (0x12475, 0x1247F, &[sc(b"Xsux")]),
    (0x12550, 0x12586, &[sc(b"Pcun"), sc(b"Xsux")]),
    (0x12587, 0x1258B, &[sc(b"Xsux")]),
    (0x1258C, 0x1258D, &[sc(b"Pcun"), sc(b"Xsux")]),
    (0x1258E, 0x12599, &[sc(b"Xsux")]),
    (0x1259A, 0x125A7, &[sc(b"Pcun"), sc(b"Xsux")]),
    (0x125A8, 0x1264B, &[sc(b"Pcun")]),
    (0x1264C, 0x12686, &[sc(b"Xsux")]),
    (0x18CD6, 0x18CDA, &[sc(b"Kits")]),
    (0x18D1F, 0x18D20, &[sc(b"Tang")]),
    (0x18E00, 0x19191, &[sc(b"Jurc")]),
    (0x191A0, 0x191D2, &[sc(b"Jurc")]),
    (0x1B123, 0x1B123, &[sc(b"Hira")]),
    (0x1B124, 0x1B128, &[sc(b"Kana")]),
    (0x1B168, 0x1B168, &[sc(b"Kana")]),
    (0x1CED2, 0x1CED4, &[sc(b"Zyyy")]),
    (0x1CEDD, 0x1CEDF, &[sc(b"Zyyy")]),
    (0x1CEF1, 0x1CEFD, &[sc(b"Zyyy")]),
    (0x1D127, 0x1D128, &[sc(b"Zinh")]),
    (0x1D1EB, 0x1D1FF, &[sc(b"Zyyy")]),
    (0x1D250, 0x1D25A, &[sc(b"Zyyy")]),
    (0x1D25B, 0x1D25C, &[sc(b"Zinh")]),
    (0x1D25D, 0x1D281, &[sc(b"Zyyy")]),
    (0x1D6A6, 0x1D6A6, &[sc(b"Zyyy")]),
    (0x1DB00, 0x1DB1C, &[sc(b"Zyyy")]),
    (0x1DF1F, 0x1DF24, &[sc(b"Latn")]),
    (0x1DF2B, 0x1DF81, &[sc(b"Latn")]),
    (0x1DF90, 0x1DF96, &[sc(b"Latn")]),
    (0x1DFCD, 0x1DFF2, &[sc(b"Latn")]),
    (0x1DFF3, 0x1DFF4, &[sc(b"Grek")]),
    (0x1DFF5, 0x1DFFF, &[sc(b"Latn")]),
    (0x1F1AE, 0x1F1AE, &[sc(b"Zyyy")]),
    (0x1F6D9, 0x1F6D9, &[sc(b"Zyyy")]),
    (0x1F7DA, 0x1F7DB, &[sc(b"Zyyy")]),
    (0x1F7F1, 0x1F7FF, &[sc(b"Zyyy")]),
    (0x1FA8B, 0x1FA8D, &[sc(b"Zyyy")]),
    (0x1FACC, 0x1FACC, &[sc(b"Zyyy")]),
    (0x1FADD, 0x1FADD, &[sc(b"Zyyy")]),
    (0x1FAEB, 0x1FAEB, &[sc(b"Zyyy")]),
    (0x1FAF9, 0x1FAFA, &[sc(b"Zyyy")]),
    (0x2B81E, 0x2B81E, &[sc(b"Hani")]),
    (0x3D000, 0x3FC3F, &[sc(b"Seal")]),
];

const BIDI_CLASS: &[Range<BidiClass>] = &[
    (0x05C8, 0x05C9, BidiClass::NonspacingMark),
    (0x0B53, 0x0B54, BidiClass::NonspacingMark),
    (0x1ADE, 0x1ADF, BidiClass::NonspacingMark),
    (0x1AEC, 0x1AF0, BidiClass::NonspacingMark),
    (0x1B3A, 0x1B3A, BidiClass::LeftToRight),
    (0x1B3C, 0x1B3C, BidiClass::LeftToRight),
    (0x1B42, 0x1B42, BidiClass::LeftToRight),
    (0x208F, 0x208F, BidiClass::OtherNeutral),
    (0x2E60, 0x2E63, BidiClass::OtherNeutral),
    (0x10ECB, 0x10ECF, BidiClass::NonspacingMark),
    (0x10EF0, 0x10EF9, BidiClass::NonspacingMark),
    (0x11DF0, 0x11DF0, BidiClass::NonspacingMark),
    (0x1CED2, 0x1CED4, BidiClass::OtherNeutral),
    (0x1CEDD, 0x1CEDF, BidiClass::OtherNeutral),
    (0x1CEF1, 0x1CEFD, BidiClass::OtherNeutral),
    (0x1D127, 0x1D128, BidiClass::NonspacingMark),
    (0x1D25B, 0x1D25C, BidiClass::NonspacingMark),
    (0x1DB00, 0x1DB1C, BidiClass::OtherNeutral),
    (0x1F6D9, 0x1F6D9, BidiClass::OtherNeutral),
    (0x1F7DB, 0x1F7DB, BidiClass::OtherNeutral),
    (0x1F7F1, 0x1F7FF, BidiClass::OtherNeutral),
    (0x1FA8B, 0x1FA8D, BidiClass::OtherNeutral),
    (0x1FACC, 0x1FACC, BidiClass::OtherNeutral),
    (0x1FADD, 0x1FADD, BidiClass::OtherNeutral),
    (0x1FAEB, 0x1FAEB, BidiClass::OtherNeutral),
    (0x1FAF9, 0x1FAFA, BidiClass::OtherNeutral),
];

/// The Script_Extensions property of this code point.
pub(crate) fn script_extensions(cp: u32) -> Vec<ScriptCode> {
    match lookup(cp, SCRIPT_EXTENSIONS) {
        Some(scripts) => scripts.to_vec(),
        None => ScriptWithExtensions::new().get_script_extensions_val32(cp).iter().filter_map(script_short_name).collect(),
    }
}

/// The Bidi_Class property of this code point.
pub(crate) fn bidi_class(cp: u32) -> BidiClass {
    lookup(cp, BIDI_CLASS).unwrap_or_else(|| CodePointMapData::<BidiClass>::new().get32(cp))
}

/// Whether this is a script added in Unicode 18, unknown to ICU4X 2.1.
pub(crate) fn is_new_script(script: ScriptCode) -> bool {
    matches!(script.as_str(), "Jurc" | "Pcun" | "Seal")
}

/// The four-letter short name of this ICU script.
fn script_short_name(script: Script) -> Option<ScriptCode> {
    PropertyNamesShort::<Script>::new().get(script).and_then(ScriptCode::parse)
}

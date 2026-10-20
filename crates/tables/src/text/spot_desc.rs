//! The platform, script, language and name descriptions of the spot listings
//! (`name=described`, `cmap=summary`). Ported from AFDKO `c/spot/desc.c` (Apache-2.0,
//! Copyright 2014 Adobe Systems Incorporated); the tables keep spot's indexing, including the
//! gap in the Macintosh languages (codes 128 and up sit at indices 95 and up).

const UNKNOWN: &str = "--unknown--";

const PLATFORM: [&str; 5] = ["Unicode", "Macintosh", "ISO", "Microsoft", "Custom"];

const UNI_SCRIPT: [&str; 6] = ["Unicode 1.0", "Unicode 1.", "ISO 10646 1993", "UTF-16 BMP", "UTF-32", "Unicode Variation Sequences"];

const ISO_SCRIPT: [&str; 3] = ["7-bit ASCII", "ISO 10646", "ISO 8859-1"];

const MAC_SCRIPT: [&str; 33] = [
    "Roman",
    "Japanese",
    "Chinese(Traditional)",
    "Korean",
    "Arabic",
    "Hebrew",
    "Greek",
    "Russian",
    "RSymbol",
    "Devanagari",
    "Gurmukhi",
    "Gujarati",
    "Oriya",
    "Bengali",
    "Tamil",
    "Telugu",
    "Kannada",
    "Malayalam",
    "Sinhalese",
    "Burmese",
    "Khmer",
    "Thai",
    "Laotian",
    "Georgian",
    "Armenian",
    "Chinese(Simplified)",
    "Tibetan",
    "Mongolian",
    "Geez",
    "Slavic",
    "Vietnamese",
    "Sindhi",
    "Uninterp",
];

const MAC_LANGUAGE: [&str; 107] = [
    "English",
    "French",
    "German",
    "Italian",
    "Dutch",
    "Swedish",
    "Spanish",
    "Danish",
    "Portuguese",
    "Norwegian",
    "Hebrew",
    "Japanese",
    "Arabic",
    "Finnish",
    "Greek",
    "Icelandic",
    "Maltese",
    "Turkish",
    "Croatian",
    "Chinese(Traditional)",
    "Urdu",
    "Hindi",
    "Thai",
    "Korean",
    "Lithuanian",
    "Polish",
    "Hungarian",
    "Estonian",
    "Lettish",
    "Saamisk",
    "Faeroese",
    "Farsi",
    "Russian",
    "Chinese(Simplified)",
    "Flemish",
    "Irish",
    "Albanian",
    "Romanian",
    "Czech",
    "Slovak",
    "Slovenian",
    "Yiddish",
    "Serbian",
    "Macedonian",
    "Bulgarian",
    "Ukrainian",
    "Byelorussian",
    "Uzbek",
    "Kazakh",
    "Azerbaijani",
    "Azerbaijan(Armenian)",
    "Armenian",
    "Georgian",
    "Moldavian",
    "Kirghiz",
    "Tajiki",
    "Turkmen",
    "Mongolian",
    "Mongolian(Cyrillic)",
    "Pashto",
    "Kurdish",
    "Kashmiri",
    "Sindhi",
    "Tibetan",
    "Nepali",
    "Sanskrit",
    "Marathi",
    "Bengali",
    "Assamese",
    "Gujarati",
    "Punjabi",
    "Oriya",
    "Malayalam",
    "Kannada",
    "Tamil",
    "Telugu",
    "Sinhalese",
    "Burmese",
    "Khmer",
    "Lao",
    "Vietnamese",
    "Indonesian",
    "Tagalog",
    "MalayRoman",
    "MalayArabic",
    "Amharic",
    "Tigrinya",
    "Galla",
    "Somali",
    "Swahili",
    "Ruanda",
    "Rundi",
    "Chewa",
    "Malagasy",
    "Esperanto",
    "Welsh",
    "Basque",
    "Catalan",
    "Latin",
    "Quechua",
    "Guarani",
    "Aymara",
    "Tatar",
    "Uighur",
    "Dzongkha",
    "Javanese",
    "Sundanese",
];

const MS_SCRIPT: [&str; 11] =
    ["Symbol", "Unicode BMP only", "ShiftJIS", "PRC", "Big5", "Wangsung", "Johab", UNKNOWN, UNKNOWN, UNKNOWN, "Unicode UCS-4"];

const MS_LANGUAGE: [(u16, &str); 58] = [
    (0x0401, "Arabic(Saudi Arabia)"),
    (0x0402, "Bulgarian"),
    (0x0403, "Catalan"),
    (0x0404, "Chinese(Taiwan)"),
    (0x0405, "Czech"),
    (0x0406, "Danish"),
    (0x0407, "German(Standard)"),
    (0x0408, "Greek"),
    (0x0409, "English(American)"),
    (0x040a, "Spanish(Traditional)"),
    (0x040b, "Finnish"),
    (0x040c, "French(Standard)"),
    (0x040e, "Hungarian"),
    (0x040f, "Icelandic"),
    (0x0410, "Italian(Standard)"),
    (0x0411, "Japanese"),
    (0x0412, "Korean"),
    (0x0413, "Dutch(Standard)"),
    (0x0414, "Norwegian(Bokmal)"),
    (0x0415, "Polish"),
    (0x0416, "Portuguese(Brazilian)"),
    (0x0418, "Romanian"),
    (0x0419, "Russian"),
    (0x041a, "Croatian"),
    (0x041b, "Slovak"),
    (0x041c, "Albanian"),
    (0x041d, "Swedish"),
    (0x041f, "Turkish"),
    (0x0422, "Ukrainian"),
    (0x0423, "Byelorussian"),
    (0x0424, "Slovenian"),
    (0x0425, "Estonian"),
    (0x0426, "Latvian"),
    (0x0427, "Lithuanian"),
    (0x042d, "Basque"),
    (0x042f, "Macedonian"),
    (0x0804, "Chinese(PR China)"),
    (0x0807, "German(Swiss)"),
    (0x0809, "English(British)"),
    (0x080a, "Spanish(Mexican)"),
    (0x080c, "French(Belgian)"),
    (0x0810, "Italian(Swiss)"),
    (0x0813, "Belgian(Flemish)"),
    (0x0814, "Norwegian(Nynorsk)"),
    (0x0816, "Portuguese(Standard)"),
    (0x0c04, "Chinese(Hong Kong)"),
    (0x0c07, "German(Austrian)"),
    (0x0c09, "English(Australian)"),
    (0x0c0a, "Spanish(Modern)"),
    (0x0c0c, "French(Canadian)"),
    (0x1004, "Chinese(Singapore)"),
    (0x1007, "German(Luxembourg)"),
    (0x1009, "English(Canadian)"),
    (0x100c, "French(Swiss)"),
    (0x1407, "German(Liechtenstein)"),
    (0x1409, "English(New Zealand)"),
    (0x140c, "French(Luxembourg)"),
    (0x1809, "English(Ireland)"),
];

const NAME: [&str; 9] = ["Copyright", "Family", "Style", "UniqueId", "Full", "Version", "PostScript", "Trademark", "Foundry"];

fn pick(list: &[&'static str], i: u16) -> &'static str {
    list.get(usize::from(i)).copied().unwrap_or(UNKNOWN)
}

pub(super) fn platform(id: u16) -> &'static str {
    pick(&PLATFORM, id)
}

pub(super) fn script(platform: u16, id: u16) -> &'static str {
    match platform {
        0 => pick(&UNI_SCRIPT, id),
        1 => pick(&MAC_SCRIPT, id),
        2 => pick(&ISO_SCRIPT, id),
        3 => pick(&MS_SCRIPT, id),
        4 => "--custom--",
        _ => UNKNOWN,
    }
}

/// `cmap` is true for a cmap subtable's language, where Macintosh languages count from 1 and
/// Windows ones are not languages.
pub(super) fn language(cmap: bool, platform: u16, id: u16) -> &'static str {
    match platform {
        0 => pick(&["Default"], id),
        1 => {
            if cmap && id == 0 {
                return "Unspecific";
            }
            pick(&MAC_LANGUAGE, if cmap { id.saturating_sub(1) } else { id })
        }
        2 => "--ISO--",
        3 if cmap => "--vers--",
        3 => MS_LANGUAGE.binary_search_by_key(&id, |(code, _)| *code).ok().and_then(|i| MS_LANGUAGE.get(i)).map_or(UNKNOWN, |(_, d)| *d),
        4 => "--cust--",
        _ => UNKNOWN,
    }
}

pub(super) fn name(id: u16) -> &'static str {
    match NAME.get(usize::from(id)) {
        Some(d) => d,
        None if id < 255 => "Standard",
        None => "Font-specific",
    }
}

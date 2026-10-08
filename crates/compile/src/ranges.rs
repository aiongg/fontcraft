//! `OS/2` Unicode range and code page bits computed from the code points
//! (`compile.os2UnicodeRanges`, `compile.os2CodePageRanges`).
//!
//! Ported from fontc 1.0.0 (`fontbe/src/os2.rs`, Apache-2.0, see `NOTICE`), which follows
//! fontTools (`O_S_2f_2.py`, MIT) for the range table and ufo2ft (`util.py`, MIT) for the code
//! page rules. The ranges are the blocks of the OpenType specification's table; a code point
//! beyond the Basic Multilingual Plane also sets bit 57.

use std::collections::{BTreeSet, HashSet};

/// `(first, last, bit)`: a code point in `first..=last` sets `bit`. Sorted, not overlapping.
const UNICODE_RANGES: &[(u32, u32, u32)] = &[
    (0x0000, 0x007F, 0),      // Basic Latin
    (0x0080, 0x00FF, 1),      // Latin-1 Supplement
    (0x0100, 0x017F, 2),      // Latin Extended-A
    (0x0180, 0x024F, 3),      // Latin Extended-B
    (0x0250, 0x02AF, 4),      // IPA Extensions
    (0x02B0, 0x02FF, 5),      // Spacing Modifier Letters
    (0x0300, 0x036F, 6),      // Combining Diacritical Marks
    (0x0370, 0x03FF, 7),      // Greek and Coptic
    (0x0400, 0x04FF, 9),      // Cyrillic
    (0x0500, 0x052F, 9),      // Cyrillic Supplement
    (0x0530, 0x058F, 10),     // Armenian
    (0x0590, 0x05FF, 11),     // Hebrew
    (0x0600, 0x06FF, 13),     // Arabic
    (0x0700, 0x074F, 71),     // Syriac
    (0x0750, 0x077F, 13),     // Arabic Supplement
    (0x0780, 0x07BF, 72),     // Thaana
    (0x07C0, 0x07FF, 14),     // NKo
    (0x0900, 0x097F, 15),     // Devanagari
    (0x0980, 0x09FF, 16),     // Bengali
    (0x0A00, 0x0A7F, 17),     // Gurmukhi
    (0x0A80, 0x0AFF, 18),     // Gujarati
    (0x0B00, 0x0B7F, 19),     // Oriya
    (0x0B80, 0x0BFF, 20),     // Tamil
    (0x0C00, 0x0C7F, 21),     // Telugu
    (0x0C80, 0x0CFF, 22),     // Kannada
    (0x0D00, 0x0D7F, 23),     // Malayalam
    (0x0D80, 0x0DFF, 73),     // Sinhala
    (0x0E00, 0x0E7F, 24),     // Thai
    (0x0E80, 0x0EFF, 25),     // Lao
    (0x0F00, 0x0FFF, 70),     // Tibetan
    (0x1000, 0x109F, 74),     // Myanmar
    (0x10A0, 0x10FF, 26),     // Georgian
    (0x1100, 0x11FF, 28),     // Hangul Jamo
    (0x1200, 0x137F, 75),     // Ethiopic
    (0x1380, 0x139F, 75),     // Ethiopic Supplement
    (0x13A0, 0x13FF, 76),     // Cherokee
    (0x1400, 0x167F, 77),     // Unified Canadian Aboriginal Syllabics
    (0x1680, 0x169F, 78),     // Ogham
    (0x16A0, 0x16FF, 79),     // Runic
    (0x1700, 0x171F, 84),     // Tagalog
    (0x1720, 0x173F, 84),     // Hanunoo
    (0x1740, 0x175F, 84),     // Buhid
    (0x1760, 0x177F, 84),     // Tagbanwa
    (0x1780, 0x17FF, 80),     // Khmer
    (0x1800, 0x18AF, 81),     // Mongolian
    (0x1900, 0x194F, 93),     // Limbu
    (0x1950, 0x197F, 94),     // Tai Le
    (0x1980, 0x19DF, 95),     // New Tai Lue
    (0x19E0, 0x19FF, 80),     // Khmer Symbols
    (0x1A00, 0x1A1F, 96),     // Buginese
    (0x1B00, 0x1B7F, 27),     // Balinese
    (0x1B80, 0x1BBF, 112),    // Sundanese
    (0x1C00, 0x1C4F, 113),    // Lepcha
    (0x1C50, 0x1C7F, 114),    // Ol Chiki
    (0x1D00, 0x1D7F, 4),      // Phonetic Extensions
    (0x1D80, 0x1DBF, 4),      // Phonetic Extensions Supplement
    (0x1DC0, 0x1DFF, 6),      // Combining Diacritical Marks Supplement
    (0x1E00, 0x1EFF, 29),     // Latin Extended Additional
    (0x1F00, 0x1FFF, 30),     // Greek Extended
    (0x2000, 0x206F, 31),     // General Punctuation
    (0x2070, 0x209F, 32),     // Superscripts And Subscripts
    (0x20A0, 0x20CF, 33),     // Currency Symbols
    (0x20D0, 0x20FF, 34),     // Combining Diacritical Marks For Symbols
    (0x2100, 0x214F, 35),     // Letterlike Symbols
    (0x2150, 0x218F, 36),     // Number Forms
    (0x2190, 0x21FF, 37),     // Arrows
    (0x2200, 0x22FF, 38),     // Mathematical Operators
    (0x2300, 0x23FF, 39),     // Miscellaneous Technical
    (0x2400, 0x243F, 40),     // Control Pictures
    (0x2440, 0x245F, 41),     // Optical Character Recognition
    (0x2460, 0x24FF, 42),     // Enclosed Alphanumerics
    (0x2500, 0x257F, 43),     // Box Drawing
    (0x2580, 0x259F, 44),     // Block Elements
    (0x25A0, 0x25FF, 45),     // Geometric Shapes
    (0x2600, 0x26FF, 46),     // Miscellaneous Symbols
    (0x2700, 0x27BF, 47),     // Dingbats
    (0x27C0, 0x27EF, 38),     // Miscellaneous Mathematical Symbols-A
    (0x27F0, 0x27FF, 37),     // Supplemental Arrows-A
    (0x2800, 0x28FF, 82),     // Braille Patterns
    (0x2900, 0x297F, 37),     // Supplemental Arrows-B
    (0x2980, 0x29FF, 38),     // Miscellaneous Mathematical Symbols-B
    (0x2A00, 0x2AFF, 38),     // Supplemental Mathematical Operators
    (0x2B00, 0x2BFF, 37),     // Miscellaneous Symbols and Arrows
    (0x2C00, 0x2C5F, 97),     // Glagolitic
    (0x2C60, 0x2C7F, 29),     // Latin Extended-C
    (0x2C80, 0x2CFF, 8),      // Coptic
    (0x2D00, 0x2D2F, 26),     // Georgian Supplement
    (0x2D30, 0x2D7F, 98),     // Tifinagh
    (0x2D80, 0x2DDF, 75),     // Ethiopic Extended
    (0x2DE0, 0x2DFF, 9),      // Cyrillic Extended-A
    (0x2E00, 0x2E7F, 31),     // Supplemental Punctuation
    (0x2E80, 0x2EFF, 59),     // CJK Radicals Supplement
    (0x2F00, 0x2FDF, 59),     // Kangxi Radicals
    (0x2FF0, 0x2FFF, 59),     // Ideographic Description Characters
    (0x3000, 0x303F, 48),     // CJK Symbols And Punctuation
    (0x3040, 0x309F, 49),     // Hiragana
    (0x30A0, 0x30FF, 50),     // Katakana
    (0x3100, 0x312F, 51),     // Bopomofo
    (0x3130, 0x318F, 52),     // Hangul Compatibility Jamo
    (0x3190, 0x319F, 59),     // Kanbun
    (0x31A0, 0x31BF, 51),     // Bopomofo Extended
    (0x31C0, 0x31EF, 61),     // CJK Strokes
    (0x31F0, 0x31FF, 50),     // Katakana Phonetic Extensions
    (0x3200, 0x32FF, 54),     // Enclosed CJK Letters And Months
    (0x3300, 0x33FF, 55),     // CJK Compatibility
    (0x3400, 0x4DBF, 59),     // CJK Unified Ideographs Extension A
    (0x4DC0, 0x4DFF, 99),     // Yijing Hexagram Symbols
    (0x4E00, 0x9FFF, 59),     // CJK Unified Ideographs
    (0xA000, 0xA48F, 83),     // Yi Syllables
    (0xA490, 0xA4CF, 83),     // Yi Radicals
    (0xA500, 0xA63F, 12),     // Vai
    (0xA640, 0xA69F, 9),      // Cyrillic Extended-B
    (0xA700, 0xA71F, 5),      // Modifier Tone Letters
    (0xA720, 0xA7FF, 29),     // Latin Extended-D
    (0xA800, 0xA82F, 100),    // Syloti Nagri
    (0xA840, 0xA87F, 53),     // Phags-pa
    (0xA880, 0xA8DF, 115),    // Saurashtra
    (0xA900, 0xA92F, 116),    // Kayah Li
    (0xA930, 0xA95F, 117),    // Rejang
    (0xAA00, 0xAA5F, 118),    // Cham
    (0xAC00, 0xD7AF, 56),     // Hangul Syllables
    (0xD800, 0xDFFF, 57),     // Non-Plane 0 *
    (0xE000, 0xF8FF, 60),     // Private Use Area (plane 0)
    (0xF900, 0xFAFF, 61),     // CJK Compatibility Ideographs
    (0xFB00, 0xFB4F, 62),     // Alphabetic Presentation Forms
    (0xFB50, 0xFDFF, 63),     // Arabic Presentation Forms-A
    (0xFE00, 0xFE0F, 91),     // Variation Selectors
    (0xFE10, 0xFE1F, 65),     // Vertical Forms
    (0xFE20, 0xFE2F, 64),     // Combining Half Marks
    (0xFE30, 0xFE4F, 65),     // CJK Compatibility Forms
    (0xFE50, 0xFE6F, 66),     // Small Form Variants
    (0xFE70, 0xFEFF, 67),     // Arabic Presentation Forms-B
    (0xFF00, 0xFFEF, 68),     // Halfwidth And Fullwidth Forms
    (0xFFF0, 0xFFFF, 69),     // Specials
    (0x10000, 0x1007F, 101),  // Linear B Syllabary
    (0x10080, 0x100FF, 101),  // Linear B Ideograms
    (0x10100, 0x1013F, 101),  // Aegean Numbers
    (0x10140, 0x1018F, 102),  // Ancient Greek Numbers
    (0x10190, 0x101CF, 119),  // Ancient Symbols
    (0x101D0, 0x101FF, 120),  // Phaistos Disc
    (0x10280, 0x1029F, 121),  // Lycian
    (0x102A0, 0x102DF, 121),  // Carian
    (0x10300, 0x1032F, 85),   // Old Italic
    (0x10330, 0x1034F, 86),   // Gothic
    (0x10380, 0x1039F, 103),  // Ugaritic
    (0x103A0, 0x103DF, 104),  // Old Persian
    (0x10400, 0x1044F, 87),   // Deseret
    (0x10450, 0x1047F, 105),  // Shavian
    (0x10480, 0x104AF, 106),  // Osmanya
    (0x10800, 0x1083F, 107),  // Cypriot Syllabary
    (0x10900, 0x1091F, 58),   // Phoenician
    (0x10920, 0x1093F, 121),  // Lydian
    (0x10A00, 0x10A5F, 108),  // Kharoshthi
    (0x12000, 0x123FF, 110),  // Cuneiform
    (0x12400, 0x1247F, 110),  // Cuneiform Numbers and Punctuation
    (0x1D000, 0x1D0FF, 88),   // Byzantine Musical Symbols
    (0x1D100, 0x1D1FF, 88),   // Musical Symbols
    (0x1D200, 0x1D24F, 88),   // Ancient Greek Musical Notation
    (0x1D300, 0x1D35F, 109),  // Tai Xuan Jing Symbols
    (0x1D360, 0x1D37F, 111),  // Counting Rod Numerals
    (0x1D400, 0x1D7FF, 89),   // Mathematical Alphanumeric Symbols
    (0x1F000, 0x1F02F, 122),  // Mahjong Tiles
    (0x1F030, 0x1F09F, 122),  // Domino Tiles
    (0x20000, 0x2A6DF, 59),   // CJK Unified Ideographs Extension B
    (0x2F800, 0x2FA1F, 61),   // CJK Compatibility Ideographs Supplement
    (0xE0000, 0xE007F, 92),   // Tags
    (0xE0100, 0xE01EF, 91),   // Variation Selectors Supplement
    (0xF0000, 0xFFFFD, 90),   // Private Use (plane 15)
    (0x100000, 0x10FFFD, 90), // Private Use (plane 16)
];

/// The Unicode range bits of `code_points`.
pub fn unicode_range_bits(code_points: &BTreeSet<u32>) -> BTreeSet<u32> {
    let mut bits = BTreeSet::new();
    for &cp in code_points {
        let found = UNICODE_RANGES.binary_search_by(|(from, to, _)| {
            if cp < *from {
                std::cmp::Ordering::Greater
            } else if cp > *to {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        if let Some((_, _, bit)) = found.ok().and_then(|i| UNICODE_RANGES.get(i)) {
            bits.insert(*bit);
        }
        if (0x1_0000..=0x10_FFFF).contains(&cp) {
            bits.insert(57);
        }
    }
    bits
}

/// The code page bits of `code_points`, by the characters each code page is known by; bit 0
/// (Latin 1) when none applies, so that the font works in word processors that check it.
pub fn code_page_bits(code_points: &BTreeSet<u32>) -> BTreeSet<u32> {
    let mut bits = BTreeSet::new();
    let chars: HashSet<char> = code_points.iter().filter_map(|cp| char::from_u32(*cp)).collect();
    let has_ascii = (0x20u32..0x7E).all(|cp| code_points.contains(&cp));
    let has_lineart = chars.contains(&'┤');
    let has = |c: char| chars.contains(&c);
    for c in &chars {
        match c {
            'Þ' if has_ascii => {
                bits.insert(0);
            }
            'Ľ' if has_ascii => {
                bits.insert(1);
                if has_lineart {
                    bits.insert(58);
                }
            }
            'Б' => {
                bits.insert(2);
                if has('Ѕ') && has_lineart {
                    bits.insert(57);
                }
                if has('╜') && has_lineart {
                    bits.insert(49);
                }
            }
            'Ά' => {
                bits.insert(3);
                if has_lineart && has('½') {
                    bits.insert(48);
                }
                if has_lineart && has('√') {
                    bits.insert(60);
                }
            }
            'İ' if has_ascii => {
                bits.insert(4);
                if has_lineart {
                    bits.insert(56);
                }
            }
            'א' => {
                bits.insert(5);
                if has_lineart && has('√') {
                    bits.insert(53);
                }
            }
            'ر' => {
                bits.insert(6);
                if has('√') {
                    bits.insert(51);
                }
                if has_lineart {
                    bits.insert(61);
                }
            }
            'ŗ' if has_ascii => {
                bits.insert(7);
                if has_lineart {
                    bits.insert(59);
                }
            }
            '₫' if has_ascii => {
                bits.insert(8);
            }
            'ๅ' => {
                bits.insert(16);
            }
            'エ' => {
                bits.insert(17);
            }
            'ㄅ' => {
                bits.insert(18);
            }
            'ㄱ' => {
                bits.insert(19);
            }
            '央' => {
                bits.insert(20);
            }
            '곴' => {
                bits.insert(21);
            }
            '♥' if has_ascii => {
                bits.insert(30);
            }
            'þ' if has_ascii && has_lineart => {
                bits.insert(54);
            }
            '╚' if has_ascii => {
                bits.insert(62);
                bits.insert(63);
            }
            'Å' if has_ascii && has_lineart && has('√') => {
                bits.insert(50);
            }
            'é' if has_ascii && has_lineart && has('√') => {
                bits.insert(52);
            }
            'õ' if has_ascii && has_lineart && has('√') => {
                bits.insert(55);
            }
            _ => {}
        }
    }
    if has_ascii && has('‰') && has('∑') {
        bits.insert(29);
    }
    if bits.is_empty() {
        bits.insert(0);
    }
    bits
}

/// Bits as the `n` 32-bit words of a range field, bits beyond them left out.
pub fn words<const N: usize>(bits: &BTreeSet<u32>) -> [u32; N] {
    let mut out = [0u32; N];
    for &b in bits {
        if let Some(word) = out.get_mut((b / 32) as usize) {
            *word |= 1 << (b % 32);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_table_is_sorted_and_disjoint() {
        assert!(UNICODE_RANGES.windows(2).all(|w| w[0].1 < w[1].0), "binary search needs this");
        assert!(UNICODE_RANGES.iter().all(|(a, b, bit)| a <= b && *bit < 128));
    }

    #[test]
    fn code_points_set_their_ranges_and_code_pages() {
        let latin: BTreeSet<u32> = (0x20..0x7F).chain([0xDE, 0x141]).collect();
        assert_eq!(unicode_range_bits(&latin), BTreeSet::from([0, 1, 2]));
        assert_eq!(code_page_bits(&latin), BTreeSet::from([0]));
        let astral = BTreeSet::from([0x1_F600, 0x41]);
        assert_eq!(unicode_range_bits(&astral), BTreeSet::from([0, 57]), "beyond the BMP sets bit 57");
        assert_eq!(code_page_bits(&BTreeSet::from([0x411])), BTreeSet::from([2]), "Cyrillic needs no ASCII");
        assert_eq!(code_page_bits(&BTreeSet::new()), BTreeSet::from([0]), "Latin 1 when nothing applies");
        assert_eq!(words::<2>(&BTreeSet::from([0, 33, 70])), [1, 2], "a bit beyond the words is left out");
    }
}

//! Line break opportunities by the Unicode Line Breaking Algorithm (UAX #14), for Text Preview
//! (row `textPreview.lineBreaking`). Classes come from the Line_Break property in
//! icu_properties; the rules are the algorithm's, LB4 to LB31, with these simplifications:
//!
//! - Classes resolved as LB1 allows: AI, SA, SG, XX, HL and the aksara classes AK, AP and AS
//!   count as AL; CJ as NS; HH as BA; H2, H3, JL and EB as ID; JV, JT, EM, VF and VI attach to the
//!   character before them as combining marks do, so a Hangul syllable written in jamo, an emoji
//!   with its modifier and a consonant with its virama stay together.
//! - LB15 is the rule of Unicode 15.0 (a quotation mark, spaces, then an opening punctuation
//!   mark), and LB25 the pair form of its tailoring example. LB21a, LB21b, LB28a and LB30's East
//!   Asian Width test are left out: the classes above keep those sequences together already, or
//!   only the rare break they allow is lost.
//! - No dictionary: a run of Thai or another SA script breaks only where spaces or punctuation
//!   allow it.
//!
//! [`opportunities`] is linear in the text and keeps no more than the classes of the last few
//! characters.

use icu_properties::CodePointMapData;
use icu_properties::props::LineBreak;

use crate::text::Item;

/// What may happen at the boundary after a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Break {
    /// Inside a character cluster (before a combining mark, after a zero width joiner, between CR
    /// and LF): never, not even inside a word wider than the line.
    Never,
    /// Not a break opportunity; a word wider than the line may still break here.
    Prohibited,
    /// A line may end here.
    Allowed,
    /// A line ends here (after a hard line break).
    Mandatory,
}

/// The classes the rules tell apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Al,
    B2,
    Ba,
    Bb,
    Bk,
    Cb,
    Cl,
    Cm,
    Cp,
    Cr,
    Ex,
    Gl,
    Hy,
    Id,
    In,
    Is,
    Lf,
    Nl,
    Ns,
    Nu,
    Op,
    Po,
    Pr,
    Qu,
    Ri,
    Sp,
    Sy,
    Wj,
    Zw,
    Zwj,
}

/// The class of each Line_Break value, by its ICU4C number.
const CLASSES: [Class; 49] = {
    use Class::*;
    [
        Al,  // 0 XX
        Al,  // 1 AI
        Al,  // 2 AL
        B2,  // 3 B2
        Ba,  // 4 BA
        Bb,  // 5 BB
        Bk,  // 6 BK
        Cb,  // 7 CB
        Cl,  // 8 CL
        Cm,  // 9 CM
        Cr,  // 10 CR
        Ex,  // 11 EX
        Gl,  // 12 GL
        Hy,  // 13 HY
        Id,  // 14 ID
        In,  // 15 IN
        Is,  // 16 IS
        Lf,  // 17 LF
        Ns,  // 18 NS
        Nu,  // 19 NU
        Op,  // 20 OP
        Po,  // 21 PO
        Pr,  // 22 PR
        Qu,  // 23 QU
        Al,  // 24 SA
        Al,  // 25 SG
        Sp,  // 26 SP
        Sy,  // 27 SY
        Zw,  // 28 ZW
        Nl,  // 29 NL
        Wj,  // 30 WJ
        Id,  // 31 H2
        Id,  // 32 H3
        Id,  // 33 JL
        Cm,  // 34 JT
        Cm,  // 35 JV
        Cp,  // 36 CP
        Ns,  // 37 CJ
        Al,  // 38 HL
        Ri,  // 39 RI
        Id,  // 40 EB
        Cm,  // 41 EM
        Zwj, // 42 ZWJ
        Al,  // 43 AK
        Al,  // 44 AP
        Al,  // 45 AS
        Cm,  // 46 VF
        Cm,  // 47 VI
        Ba,  // 48 HH
    ]
};

fn class_of(data: &icu_properties::CodePointMapDataBorrowed<'static, LineBreak>, c: char) -> Class {
    CLASSES.get(usize::from(data.get(c).to_icu4c_value())).copied().unwrap_or(Class::Al)
}

/// The break opportunity after each character of `chars`: entry `k` is the boundary between
/// characters `k` and `k + 1`. The last entry is [`Break::Mandatory`] after a hard line break and
/// [`Break::Allowed`] otherwise (the end of the text).
pub fn opportunities<I: IntoIterator<Item = char>>(chars: I) -> Vec<Break> {
    let data = CodePointMapData::<LineBreak>::new();
    classify(chars.into_iter().map(|c| class_of(&data, c)))
}

/// [`opportunities`] for the items of a text: a line break is a hard line break, a glyph without
/// a character and a placeholder count as letters.
pub(crate) fn item_opportunities(items: &[Item]) -> Vec<Break> {
    let data = CodePointMapData::<LineBreak>::new();
    classify(items.iter().map(|item| match item {
        Item::Glyph { typed: Some(c), .. } | Item::Missing(c) => class_of(&data, *c),
        Item::Glyph { typed: None, .. } | Item::Placeholder => Class::Al,
        Item::LineBreak => Class::Bk,
    }))
}

/// The state of the rules between two characters.
struct State {
    /// The class of the character before the boundary, after LB9 and LB10.
    prev: Class,
    /// The class of the last character before the boundary that is not a space.
    before_spaces: Class,
    /// The character before the boundary is a zero width joiner (LB8a).
    after_zwj: bool,
    /// Regional indicators in a row just before the boundary (LB30a).
    indicators: usize,
}

fn classify(classes: impl Iterator<Item = Class>) -> Vec<Break> {
    use Class::*;
    let (lower, _) = classes.size_hint();
    let mut out = Vec::with_capacity(lower);
    let mut state: Option<State> = None;
    for raw in classes {
        let Some(s) = state.as_mut() else {
            // LB10: a combining mark at the start is a letter.
            let first = if matches!(raw, Cm | Zwj) { Al } else { raw };
            state = Some(State { prev: first, before_spaces: first, after_zwj: raw == Zwj, indicators: usize::from(first == Ri) });
            continue;
        };
        let (decision, absorbed) = decide(s, raw);
        out.push(decision);
        s.after_zwj = raw == Zwj;
        if absorbed {
            continue;
        }
        let class = if matches!(raw, Cm | Zwj) { Al } else { raw };
        s.prev = class;
        if class != Sp {
            s.before_spaces = class;
        }
        s.indicators = if class == Ri { s.indicators + 1 } else { 0 };
    }
    if let Some(s) = state {
        out.push(if matches!(s.prev, Bk | Cr | Lf | Nl) { Break::Mandatory } else { Break::Allowed });
    }
    out
}

/// The opportunity before a character of class `raw`, and whether the character attaches to the
/// one before it (LB9) and so leaves the state as it is.
fn decide(s: &State, raw: Class) -> (Break, bool) {
    use Class::*;
    let (prev, a) = (s.prev, s.before_spaces);
    // LB4, LB5: hard line breaks.
    match prev {
        Cr if raw == Lf => return (Break::Never, false),
        Bk | Cr | Lf | Nl => return (Break::Mandatory, false),
        _ => {}
    }
    // LB6, LB7.
    if matches!(raw, Bk | Cr | Lf | Nl | Sp | Zw) {
        return (Break::Prohibited, false);
    }
    // LB8: after a zero width space, spaces between.
    if a == Zw {
        return (Break::Allowed, false);
    }
    // LB8a.
    if s.after_zwj {
        return (Break::Never, matches!(raw, Cm | Zwj));
    }
    // LB9: a combining mark or joiner takes the class of the character it follows.
    if matches!(raw, Cm | Zwj) && !matches!(prev, Sp | Zw) {
        return (Break::Never, true);
    }
    // LB10.
    let c = if matches!(raw, Cm | Zwj) { Al } else { raw };
    // LB11, LB12, LB12a.
    let prohibited = c == Wj
        || prev == Wj
        || prev == Gl
        || (c == Gl && !matches!(prev, Sp | Ba | Hy))
        // LB13: never before closing punctuation, even after spaces.
        || matches!(c, Cl | Cp | Ex | Is | Sy)
        // LB14 to LB17, spaces between.
        || a == Op
        || (a == Qu && c == Op)
        || (matches!(a, Cl | Cp) && c == Ns)
        || (a == B2 && c == B2);
    if prohibited {
        return (Break::Prohibited, false);
    }
    // LB18.
    if prev == Sp {
        return (Break::Allowed, false);
    }
    // LB19 to LB30a.
    if c == Qu || prev == Qu {
        return (Break::Prohibited, false);
    }
    if c == Cb || prev == Cb {
        return (Break::Allowed, false);
    }
    let prohibited = matches!(c, Ba | Hy | Ns | In) || prev == Bb || keeps_together(prev, c) || (prev == Ri && c == Ri && s.indicators % 2 == 1);
    // LB31.
    (if prohibited { Break::Prohibited } else { Break::Allowed }, false)
}

/// The pairs of LB23 to LB30 that never break.
fn keeps_together(prev: Class, c: Class) -> bool {
    use Class::*;
    matches!(
        (prev, c),
        // LB23, LB23a, LB24.
        (Al, Nu) | (Nu, Al) | (Pr, Id) | (Id, Po) | (Pr | Po, Al) | (Al, Pr | Po)
        // LB25.
        | (Cl | Cp | Nu, Po | Pr) | (Po | Pr, Op | Nu) | (Hy | Is | Nu | Sy, Nu)
        // LB28, LB29, LB30.
        | (Al, Al) | (Is, Al) | (Al | Nu, Op) | (Cp, Al | Nu)
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    /// `text` cut after every allowed or mandatory break.
    fn pieces(text: &str) -> Vec<String> {
        let breaks = opportunities(text.chars());
        let mut out = vec![String::new()];
        for (c, b) in text.chars().zip(breaks) {
            out.last_mut().unwrap().push(c);
            if matches!(b, Break::Allowed | Break::Mandatory) {
                out.push(String::new());
            }
        }
        out.pop();
        out
    }

    #[test]
    fn latin_breaks_after_spaces_and_hyphens_and_never_before_closing_punctuation() {
        assert_eq!(pieces("The quick (brown) fox."), ["The ", "quick ", "(brown) ", "fox."]);
        assert_eq!(pieces("well-known -5 and pre-2000"), ["well-", "known ", "-5 ", "and ", "pre-2000"]);
        assert_eq!(pieces("x ) y !"), ["x ) ", "y !"], "no break before a closing parenthesis or an exclamation mark, even after a space");
        assert_eq!(pieces("$12.50, “quoted” 50%"), ["$12.50, ", "“quoted” ", "50%"]);
        assert_eq!(pieces("no\u{a0}break here"), ["no\u{a0}break ", "here"], "a no-break space glues");
        assert_eq!(pieces("a  b\tc"), ["a  ", "b\t", "c"], "spaces stay with the word before them");
    }

    #[test]
    fn cjk_breaks_between_ideographs_but_not_before_small_kana_or_closing_marks() {
        assert_eq!(pieces("漢字。かな"), ["漢", "字。", "か", "な"]);
        assert_eq!(pieces("きゃく「本」"), ["きゃ", "く", "「本」"]);
        assert_eq!(pieces("한국어 문장"), ["한", "국", "어 ", "문", "장"]);
    }

    #[test]
    fn hard_breaks_are_mandatory_and_clusters_never_break() {
        let b = opportunities("a\u{2028}b\r\nc".chars());
        assert_eq!(b, [Break::Prohibited, Break::Mandatory, Break::Prohibited, Break::Never, Break::Mandatory, Break::Allowed]);
        let b = opportunities("e\u{301}x \u{301}".chars());
        assert_eq!(b[0], Break::Never, "a combining mark stays with its base");
        assert_eq!(b[1], Break::Prohibited, "the base keeps its letter class");
        assert_eq!(b[2], Break::Prohibited);
        assert_eq!(b[3], Break::Allowed, "a combining mark after a space is a letter");
        assert_eq!(opportunities("👍🏽 👩\u{200d}💻".chars())[..4], [Break::Never, Break::Prohibited, Break::Allowed, Break::Never]);
        assert_eq!(pieces("🇫🇷🇩🇪"), ["🇫🇷", "🇩🇪"], "flags pair their regional indicators");
    }
}

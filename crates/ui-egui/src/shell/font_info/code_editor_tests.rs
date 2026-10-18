//! The code editor's layout: colours by class, error underlines, previewable names, and offsets
//! in text with characters of several bytes.

use egui::FontId;
use fontcraft_engine::features::edit::HighlightKind;

use super::{MAX_HIGHLIGHT_BYTES, byte_to_char, char_to_byte, colour, layout_job, previewable_at};
use crate::theme::Tokens;

/// The job's stretches as (text, colour, underlined).
fn stretches(code: &str, errors: &[std::ops::Range<usize>]) -> Vec<(String, egui::Color32, bool)> {
    let tokens = Tokens::dark();
    let job = layout_job(code, &tokens, &FontId::monospace(12.0), errors);
    job.sections
        .iter()
        .map(|s| {
            (job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)].to_string(), s.format.color, s.format.underline.width > 0.0)
        })
        .collect()
}

fn colour_of(code: &str, part: &str) -> egui::Color32 {
    stretches(code, &[]).into_iter().find(|(t, _, _)| t == part).unwrap_or_else(|| panic!("`{part}` is a stretch of `{code}`")).1
}

#[test]
fn features_syntax_colouring_tells_keywords_names_numbers_and_comments_apart() {
    let tokens = Tokens::dark();
    let code = "lookup Two { sub a by b; pos a 20; } Two; # end";
    assert_eq!(colour_of(code, "sub"), colour(HighlightKind::Keyword, &tokens));
    assert_eq!(colour_of(code, "a"), colour(HighlightKind::GlyphName, &tokens));
    assert_eq!(colour_of(code, "20"), colour(HighlightKind::Number, &tokens));
    assert_eq!(colour_of(code, "# end"), colour(HighlightKind::Comment, &tokens));
    let distinct = [HighlightKind::Keyword, HighlightKind::GlyphName, HighlightKind::Number, HighlightKind::Comment, HighlightKind::LookupName];
    for (i, a) in distinct.iter().enumerate() {
        for b in distinct.iter().skip(i + 1) {
            assert_ne!(colour(*a, &tokens), colour(*b, &tokens), "{a:?} and {b:?} look alike");
        }
    }
    // The job holds the code as it is, whatever the colours.
    let all: String = stretches(code, &[]).into_iter().map(|s| s.0).collect();
    assert_eq!(all, code);
}

#[test]
fn features_inline_errors_underline_their_range_only() {
    let code = "sub a by zzz;";
    let parts = stretches(code, &[9..12]);
    let underlined: Vec<&str> = parts.iter().filter(|p| p.2).map(|p| p.0.as_str()).collect();
    assert_eq!(underlined, ["zzz"]);
    // Ranges off character boundaries, past the end, reversed or overlapping are taken as they
    // can be, without losing text.
    let code = "sub é by ü;";
    for errors in [vec![5..6], vec![0..99], vec![7..3], vec![0..5, 2..9]] {
        let all: String = stretches(code, &errors).into_iter().map(|s| s.0).collect();
        assert_eq!(all, code, "{errors:?}");
    }
}

#[test]
fn long_code_is_shown_plain() {
    let code = "sub a by b;\n".repeat(MAX_HIGHLIGHT_BYTES / 12 + 1);
    let parts = stretches(&code, &[0..3]);
    assert_eq!(parts.len(), 1, "one plain stretch past the limit");
    assert!(previewable_at(&code, 4).is_none());
}

#[test]
fn alt_click_previews_glyph_names_classes_and_tokens() {
    let code = "sub @Upper a by $[name == \"b\"];";
    let kind_at = |at: usize| previewable_at(code, at).map(|(r, k)| (&code[r], k));
    assert_eq!(kind_at(6), Some(("@Upper", HighlightKind::ClassName)));
    assert_eq!(kind_at(11), Some(("a", HighlightKind::GlyphName)));
    assert_eq!(kind_at(1), None, "a keyword shows nothing");
    let (token, kind) = kind_at(18).unwrap();
    assert_eq!(kind, HighlightKind::Token);
    assert!(token.starts_with("$["), "{token}");
}

#[test]
fn offsets_convert_between_characters_and_bytes() {
    let text = "aé€b";
    assert_eq!(char_to_byte(text, 2), 3);
    assert_eq!(char_to_byte(text, 4), text.len());
    assert_eq!(char_to_byte(text, 99), text.len());
    assert_eq!(byte_to_char(text, 3), 2);
    assert_eq!(byte_to_char(text, 4), 2, "inside a character counts to its start");
    assert_eq!(byte_to_char(text, 99), 4);
}

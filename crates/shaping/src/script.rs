//! Script runs (M8.18): text split where the script changes, by the Unicode Script_Extensions
//! property from icu_properties. Characters without a script of their own (Common, Inherited
//! and Unknown) join the run around them, as HarfBuzz's itemisation does for the first two.
//! Unknown joins too because the private-use code points of glyphs typed by name are Unknown,
//! and they must be shaped with their neighbours.

use std::ops::Range;

use icu_properties::PropertyNamesShort;
use icu_properties::props::Script;
use icu_properties::script::ScriptWithExtensions;

use crate::Tag;

/// The ISO 15924 tag of the Common script, for text without a script of its own.
pub const COMMON: Tag = Tag::new(b"Zyyy");

/// One run of one script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRun {
    /// Byte range of the text.
    pub range: Range<usize>,
    /// The ISO 15924 script tag (`Latn`, `Arab`), as [`crate::ShapeRequest::script`] takes it.
    pub script: Tag,
}

/// The script runs of `text`, in logical order, covering it without gaps.
///
/// A run keeps the scripts its characters share: a character whose Script_Extensions lists
/// several scripts (the Devanagari danda, the Japanese prolonged sound mark) narrows them, and a
/// run ends before a character that shares none. Characters without a script before the first
/// scripted one belong to the first run; text with no scripted character is one [`COMMON`] run.
pub fn itemize(text: &str) -> Vec<ScriptRun> {
    if text.is_empty() {
        return Vec::new();
    }
    let data = ScriptWithExtensions::new();
    let mut runs = Vec::new();
    let mut start = 0;
    // The scripts every scripted character of the current run has; empty until the first one.
    let mut current: Vec<Script> = Vec::new();
    for (index, ch) in text.char_indices() {
        let set = data.get_script_extensions_val(ch);
        if !set.iter().any(is_scripted) {
            continue;
        }
        // A combining mark stays on the character before it even when its Script_Extensions
        // leave that script out (U+0301 lists Latin and Greek but not Han).
        if data.get_script_val(ch) == Script::Inherited && !current.iter().any(|s| set.contains(s)) {
            continue;
        }
        if current.is_empty() {
            current.extend(set.iter().filter(|s| is_scripted(*s)));
        } else if current.iter().any(|s| set.contains(s)) {
            current.retain(|s| set.contains(s));
        } else {
            runs.push(ScriptRun { range: start..index, script: run_tag(&current) });
            start = index;
            current.clear();
            current.extend(set.iter().filter(|s| is_scripted(*s)));
        }
    }
    runs.push(ScriptRun { range: start..text.len(), script: run_tag(&current) });
    runs
}

fn is_scripted(script: Script) -> bool {
    !matches!(script, Script::Common | Script::Inherited | Script::Unknown)
}

/// The tag of a run's first remaining script; [`COMMON`] for a run without one.
fn run_tag(scripts: &[Script]) -> Tag {
    scripts.first().map_or(COMMON, |s| script_tag(*s))
}

/// The ISO 15924 tag of a Unicode script: its short property value alias (`Latn`).
fn script_tag(script: Script) -> Tag {
    PropertyNamesShort::<Script>::new()
        .get(script)
        .and_then(|name| <[u8; 4]>::try_from(name.as_bytes()).ok())
        .map_or(COMMON, |bytes| Tag::new(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(text: &str) -> Vec<(Range<usize>, Tag)> {
        itemize(text).into_iter().map(|r| (r.range, r.script)).collect()
    }

    #[test]
    fn latin_then_cyrillic_split_after_the_shared_punctuation() {
        // "abc, " is 5 bytes; the comma and space join the Latin run before them.
        assert_eq!(summary("abc, \u{043A}\u{043E}\u{0442}"), vec![(0..5, Tag::new(b"Latn")), (5..11, Tag::new(b"Cyrl"))]);
    }

    #[test]
    fn leading_digits_and_marks_join_the_first_scripted_run() {
        assert_eq!(summary("1 a\u{0301}b"), vec![(0..6, Tag::new(b"Latn"))]);
        assert_eq!(summary("123 !"), vec![(0..5, COMMON)]);
    }

    #[test]
    fn a_mark_stays_with_the_character_before_it() {
        // U+4E00 U+0301 is 5 bytes; the acute's Script_Extensions do not list Han.
        assert_eq!(summary("\u{4E00}\u{0301}B"), vec![(0..5, Tag::new(b"Hani")), (5..6, Tag::new(b"Latn"))]);
    }

    #[test]
    fn a_danda_stays_in_its_devanagari_run() {
        // U+0964 is Common with Script_Extensions listing Devanagari among others.
        assert_eq!(summary("\u{0915}\u{0964}"), vec![(0..6, Tag::new(b"Deva"))]);
    }

    #[test]
    fn private_use_glyphs_typed_by_name_stay_with_their_neighbours() {
        assert_eq!(summary("a\u{F0000}b"), vec![(0..6, Tag::new(b"Latn"))]);
    }

    #[test]
    fn arabic_and_latin_are_separate_runs() {
        let runs = summary("\u{0628}\u{0628} ab");
        assert_eq!(runs, vec![(0..5, Tag::new(b"Arab")), (5..7, Tag::new(b"Latn"))]);
    }
}

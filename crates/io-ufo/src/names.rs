//! The UFO specification's rule for turning a user name (a glyph or layer name) into a file
//! name, as fontTools' `ufoLib.filenames` implements it (MIT; see `NOTICE`).
//!
//! Characters that are illegal in file names on some system become `_`; an uppercase character
//! is followed by `_` so names that differ only in case do not collide on case-insensitive file
//! systems; a leading `.` becomes `_` (unless there is a prefix); a dot-separated part that is a
//! reserved device name on Windows gets a leading `_`; the result is cut to 255 characters with
//! the prefix and suffix; and a name that already exists (compared without case) gets a
//! fifteen-digit counter.

use std::collections::HashSet;

const ILLEGAL: &[char] = &['"', '*', '+', '/', ':', '<', '>', '?', '[', '\\', ']', '(', ')', '|', '\x7f'];
const RESERVED: &[&str] = &[
    "aux", "clock$", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "con", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6",
    "lpt7", "lpt8", "lpt9", "nul", "prn",
];
const MAX_LEN: usize = 255;
/// How many counters are tried before giving up on a name.
const MAX_TRIES: u64 = 1_000_000;

fn is_illegal(c: char) -> bool {
    c < ' ' || ILLEGAL.contains(&c)
}

/// The file name for `user_name`, given the names already taken, lowercased.
pub fn user_name_to_file_name(user_name: &str, existing: &HashSet<String>, prefix: &str, suffix: &str) -> Option<String> {
    let mut name = String::with_capacity(user_name.len() + 8);
    for (i, c) in user_name.chars().enumerate() {
        if i == 0 && c == '.' && prefix.is_empty() || is_illegal(c) {
            name.push('_');
        } else {
            name.push(c);
            if c.to_lowercase().ne(std::iter::once(c)) {
                name.push('_');
            }
        }
    }
    let room = MAX_LEN.saturating_sub(prefix.chars().count() + suffix.chars().count());
    let name: String = name.chars().take(room).collect();
    let name = name
        .split('.')
        .map(|part| if RESERVED.contains(&part.to_lowercase().as_str()) { format!("_{part}") } else { part.to_string() })
        .collect::<Vec<_>>()
        .join(".");
    let full = format!("{prefix}{name}{suffix}");
    if !existing.contains(&full.to_lowercase()) {
        return Some(full);
    }
    // Make room for the counter, then count.
    let fixed = prefix.chars().count() + suffix.chars().count() + 15;
    let stem: String = name.chars().take(MAX_LEN.saturating_sub(fixed)).collect();
    (1..MAX_TRIES).map(|n| format!("{prefix}{stem}{n:015}{suffix}")).find(|f| !existing.contains(&f.to_lowercase()))
}

/// The `.glif` file name of a glyph.
pub fn glyph_file_name(glyph: &str, existing: &HashSet<String>) -> Option<String> {
    user_name_to_file_name(glyph, existing, "", ".glif")
}

/// The directory name of a non-default layer.
pub fn layer_dir_name(layer: &str, existing: &HashSet<String>) -> Option<String> {
    user_name_to_file_name(layer, existing, "glyphs.", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str) -> String {
        glyph_file_name(name, &HashSet::new()).unwrap()
    }

    #[test]
    fn the_specification_examples() {
        // The examples of the UFO 3 conventions page and of fontTools' doctests.
        assert_eq!(f("a"), "a.glif");
        assert_eq!(f("A"), "A_.glif");
        assert_eq!(f("AE"), "A_E_.glif");
        assert_eq!(f("Ae"), "A_e.glif");
        assert_eq!(f("ae"), "ae.glif");
        assert_eq!(f("aE"), "aE_.glif");
        assert_eq!(f("a.alt"), "a.alt.glif");
        assert_eq!(f("A.alt"), "A_.alt.glif");
        assert_eq!(f("A.Alt"), "A_.A_lt.glif");
        assert_eq!(f("A.aLt"), "A_.aL_t.glif");
        assert_eq!(f("A.alT"), "A_.alT_.glif");
        assert_eq!(f("T_H"), "T__H_.glif");
        assert_eq!(f("T_h"), "T__h.glif");
        assert_eq!(f("t_h"), "t_h.glif");
        assert_eq!(f("F_F_I"), "F__F__I_.glif");
        assert_eq!(f("f_f_i"), "f_f_i.glif");
        assert_eq!(f("Aacute_V.swash"), "A_acute_V_.swash.glif");
        assert_eq!(f(".notdef"), "_notdef.glif");
        assert_eq!(f("con"), "_con.glif");
        assert_eq!(f("CON"), "C_O_N_.glif");
        assert_eq!(f("con.alt"), "_con.alt.glif");
        assert_eq!(f("alt.con"), "alt._con.glif");
        assert_eq!(f("a/b"), "a_b.glif");
        assert_eq!(f("\u{c1}"), "\u{c1}_.glif");
    }

    #[test]
    fn clashes_get_a_counter() {
        let taken: HashSet<String> = ["a.glif".to_string()].into();
        assert_eq!(glyph_file_name("A", &HashSet::from(["a_.glif".to_string()])).unwrap(), "A_000000000000001.glif");
        assert_eq!(glyph_file_name("a", &taken).unwrap(), "a000000000000001.glif");
        assert_eq!(layer_dir_name("public.background", &HashSet::new()).unwrap(), "glyphs.public.background");
        assert_eq!(layer_dir_name(".hidden", &HashSet::new()).unwrap(), "glyphs..hidden");
    }

    #[test]
    fn long_names_are_cut() {
        let long = "a".repeat(300);
        let name = f(&long);
        assert_eq!(name.chars().count(), 255);
        assert!(name.ends_with(".glif"));
    }
}

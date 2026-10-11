#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use fontcraft_fea::{Diagnostic, Severity};
use fontcraft_glyphquery::predicate::Predicate;
use fontcraft_model::{Features, Font, Localized, Snippet};

use super::*;
use crate::GlyphMetric;

/// Token values for the tests: the glyphs it exports and its number values.
struct Values {
    target: Target,
    glyphs: Vec<String>,
    numbers: Vec<(&'static str, f64)>,
}

impl Values {
    fn of(font: &Font) -> Self {
        Values { target: Target::Master("m01".into()), glyphs: font.glyphs.iter().map(|g| g.name.clone()).collect(), numbers: Vec::new() }
    }
}

impl TargetValues for Values {
    fn target(&self) -> &Target {
        &self.target
    }
    fn number_value(&self, name: &str) -> Option<f64> {
        self.numbers.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }
    fn glyph_metric(&self, _: &str, _: GlyphMetric) -> Option<f64> {
        None
    }
    fn anchor(&self, _: &str, _: &str) -> Option<(f64, f64)> {
        None
    }
    fn glyphs(&self) -> &[String] {
        &self.glyphs
    }
    fn select(&self, _: &Predicate) -> Result<Vec<String>, String> {
        Err("no predicates in these tests".into())
    }
}

fn snippet(name: &str, code: &str) -> Snippet {
    Snippet { name: name.into(), code: code.into(), ..Snippet::default() }
}

fn label(language: &str, value: &str) -> Localized {
    Localized { language: language.into(), value: value.into(), extra: Default::default() }
}

fn assembled(font: &Font, entries: &Features) -> Assembly {
    assemble(font, entries, &Target::Master("m01".into()), &Values::of(font)).unwrap()
}

/// The (line, column) of byte `offset` of `text`, both from 1.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = offset - before.rfind('\n').map_or(0, |i| i + 1) + 1;
    (line, column)
}

#[test]
fn assembly_equals_the_oracle_feature_file() {
    let font = Font::new("Assembly");
    let mut entries = Features::default();
    entries.classes.push(Snippet { automatic: true, ..snippet("Uppercase", "A B C") });
    entries.classes.push(snippet("@Lowercase", "a b"));
    entries.classes.push(Snippet { disabled: true, ..snippet("Old", "x") });
    entries.prefixes.push(Snippet { automatic: true, ..snippet("Languagesystems", "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\n") });
    entries.prefixes.push(snippet("Defs", "lookup L1 {\n  sub a by b;\n} L1;"));
    entries.prefixes.push(Snippet { disabled: true, ..snippet("Off", "# nothing") });
    entries.features.push(Snippet { automatic: true, ..snippet("liga", "sub f i by f_i;") });
    let labels = vec![label("dflt", "Alt \"one\" \\ two"), label("DEU", "Alternativen"), label("ZHT", "x")];
    entries.features.push(Snippet { labels, ..snippet("ss01", "sub a by a.ss01;") });
    entries.features.push(snippet("kern", "pos A V -50;"));
    entries.features.push(Snippet { disabled: true, ..snippet("smcp", "sub a by a.sc;") });

    let assembly = assembled(&font, &entries);
    // glyphsLib's `_to_ufo_features(font, minimal=True)` for the same entries.
    let oracle = "# automatic\n@Uppercase = [ A B C\n];\n\n@Lowercase = [ a b\n];\n\n# Prefix: Languagesystems\n# automatic\nlanguagesystem DFLT dflt;\nlanguagesystem latn dflt;\n\n\n# Prefix: Defs\nlookup L1 {\n  sub a by b;\n} L1;\n\nfeature liga {\n# automatic\nsub f i by f_i;\n} liga;\n\nfeature ss01 {\nfeatureNames {\n  name \"Alt \\0022one\\0022 \\005c two\";\n  name 3 1 0x407 \"Alternativen\";\n  name 3 1 0x404 \"x\";\n};\nsub a by a.ss01;\n} ss01;\n\nfeature kern {\npos A V -50;\n# Automatic Code\n\n} kern;\n";
    assert_eq!(assembly.expanded_text(), oracle);
    // One file per enabled entry, classes first, disabled entries left out.
    assert_eq!(assembly.files.len(), 8);
    assert!(
        assembly.files.root_text().starts_with("include(entries/class-0.fea);\n\ninclude(entries/class-1.fea);\n\ninclude(entries/prefix-0.fea);")
    );
    assert!(assembly.map.path_of(EntryRef { kind: EntryKind::Class, index: 2 }).is_none());

    // A kern feature that has the marker, or is automatic, gets none added.
    let mut marked = Features::default();
    marked.features.push(snippet("kern", "pos A V -50;\n# Automatic Code\npos V A -20;"));
    assert_eq!(assembled(&font, &marked).expanded_text(), "feature kern {\npos A V -50;\n# Automatic Code\npos V A -20;\n} kern;\n");

    // A label in a language without a Windows id is left out; no labels left, no block.
    let mut unknown = Features::default();
    unknown.features.push(Snippet { labels: vec![label("XYZ", "Unknown"), label("ENG", "")], ..snippet("ss02", "sub b by b.ss02;") });
    assert_eq!(assembled(&font, &unknown).expanded_text(), "feature ss02 {\nsub b by b.ss02;\n} ss02;\n");

    assert_eq!(assembled(&font, &Features::default()).expanded_text(), "");
}

#[test]
fn a_diagnostic_after_a_token_maps_to_the_stored_line_and_column() {
    let font = Font::new("Map");
    let stored = "sub a by b;\npos A $pad;\nsub c by d e;";
    let mut entries = Features::default();
    entries.classes.push(snippet("Uppercase", "A B"));
    entries.features.push(snippet("liga", stored));
    let values = Values { numbers: vec![("pad", 120.0)], ..Values::of(&font) };
    let assembly = assemble(&font, &entries, &Target::Master("m01".into()), &values).unwrap();
    let entry = EntryRef { kind: EntryKind::Feature, index: 0 };
    let path = assembly.map.path_of(entry).unwrap().to_string();
    let file = assembly.files.get(&path).unwrap();

    let at = file.find("d e").unwrap();
    let diagnostic = Diagnostic::at(path.clone(), at..at + 3, Severity::Error, "glyph `d` is not in the font");
    let mapped = assembly.map.map(diagnostic);
    let place = mapped.place.expect("the span is in the entry's code");
    assert_eq!(place.entry, entry);
    assert_eq!(&stored[place.range.clone()], "d e");
    assert_eq!(line_column(stored, place.range.start), (3, 10));

    // Text the assembler wrote around the code belongs to no entry.
    let head = file.find("feature liga").unwrap();
    assert_eq!(assembly.map.locate(&path, head..head + 7), None);
    assert_eq!(assembly.map.locate("features.fea", 0..1), None);
}

#[test]
fn kerning_groups_the_code_uses_are_defined_first() {
    let mut font = Font::new("Groups");
    for (glyph, right, left) in
        [("A", Some("A"), None), ("B", Some("A"), None), ("C", Some("O"), None), ("V", None, Some("V")), ("W", None, Some("V"))]
    {
        let mut g = font.glyph_mut(glyph).unwrap();
        g.kern_groups.right = right.map(str::to_string);
        g.kern_groups.left = left.map(str::to_string);
    }
    let mut entries = Features::default();
    entries.classes.push(snippet("MMK_R_W", "W"));
    entries.features.push(snippet("kern", "pos @MMK_L_A @MMK_R_V -40;\npos @MMK_L_A @MMK_R_W -10;\npos @MMK_L_X a 5;"));
    // B is not exported by this target.
    let mut values = Values::of(&font);
    values.glyphs.retain(|g| g != "B");
    let assembly = assemble(&font, &entries, &Target::Master("m01".into()), &values).unwrap();
    let text = assembly.expanded_text();
    // Only the groups the code uses and no class entry defines; O is unused and X is empty.
    assert!(text.starts_with("@MMK_L_A = [A];\n@MMK_R_V = [V W];\n\n@MMK_R_W = [ W\n];\n\n"), "{text}");
    assert!(!text.contains("@MMK_L_O"));
    assert!(!text.contains("@MMK_L_X ="));
    assert_eq!(assembly.files.root_text().lines().next(), Some("include(entries/kerning-groups.fea);"));
}

#[test]
fn offset_maps_compose_through_two_passes() {
    // Pass 1 replaced the token `$x` of "ab $x cd" by "100": "ab 100 cd".
    let mut first = OffsetMap::default();
    first.push(0..3, 0..3);
    first.push(3..6, 3..5);
    first.push(6..9, 5..8);
    // Pass 2 deleted "ab ": "100 cd".
    let mut second = OffsetMap::default();
    second.push(0..0, 0..3);
    second.push(0..6, 3..9);
    let map = OffsetMap::compose(&first, &second);
    // "cd" maps byte for byte, the token's value to the whole token.
    assert_eq!(map.source_range(4..6), 6..8);
    assert_eq!(map.source_range(1..2), 3..5);
    assert_eq!(map.source_range(0..3), 3..5);
    assert_eq!(map.source_offset(6), 8);
    // The identity on either side gives the other map.
    assert_eq!(OffsetMap::compose(&OffsetMap::identity(), &second), second);
    assert_eq!(OffsetMap::compose(&first, &OffsetMap::identity()), first);
    assert_eq!(OffsetMap::identity().source_range(5..9), 5..9);
}

#[test]
fn pass_errors_and_limits_change_nothing() {
    let font = Font::new("Limits");
    let mut entries = Features::default();
    entries.features.push(snippet("liga", &"a".repeat(MAX_ENTRY_BYTES + 1)));
    assert!(matches!(assembled_err(&font, &entries), FeatureError::Limit { .. }));

    // A disabled entry is not read at all.
    entries.features[0].disabled = true;
    assert_eq!(assembled(&font, &entries).files.len(), 1);
}

fn assembled_err(font: &Font, entries: &Features) -> FeatureError {
    assemble(font, entries, &Target::Master("m01".into()), &Values::of(font)).unwrap_err()
}

#[test]
fn a_standalone_lookup_in_a_feature_entry_is_not_registered_in_the_feature() {
    let font = Font::new("Standalone");
    let stored = "sub a by b;\nstandalone lookup demo {\n  sub c by d;\n} demo;\nsub e' lookup demo f;";
    let mut entries = Features::default();
    entries.features.push(snippet("ccmp", stored));
    let assembly = assembled(&font, &entries);
    let text = assembly.expanded_text();
    let lookup = text.find("lookup demo {").unwrap();
    // The feature block is closed before the lookup and reopened after it.
    let closed = text[..lookup].rfind("} ccmp;").expect("the feature closes before the lookup");
    assert!(text[..closed].contains("feature ccmp {"));
    assert!(text[lookup..].contains("feature ccmp {"));
    assert!(!text.contains("standalone"));

    // The lookup's code still maps back to the stored entry.
    let entry = EntryRef { kind: EntryKind::Feature, index: 0 };
    let path = assembly.map.path_of(entry).unwrap().to_string();
    let file = assembly.files.get(&path).unwrap();
    let at = file.find("sub c by d").unwrap();
    let place = assembly.map.locate(&path, at..at + 10).unwrap();
    assert_eq!(&stored[place.range], "sub c by d");
}

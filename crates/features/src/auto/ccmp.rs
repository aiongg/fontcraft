//! Glyph composition and decomposition (M8.13).
//!
//! Rows: `autoFeature.ccmp`, `anchors.ccmpAlongsideMark`.
//!
//! The Latin ccmp in the form the reference stores it (the corpus's automatic ccmp entries):
//!
//! 1. `ccmp_DFLT_n` with `lookupflag 0;`: ligatures of stacked marks
//!    (`sub brevecomb acutecomb by brevecomb_acutecomb;`), for every glyph whose name joins mark
//!    glyphs of the font with underscores, in glyph order.
//! 2. `ccmp_DFLT_n`: the dotless lookup. `@CombiningTopAccents` holds the font's marks whose code
//!    point is a mark above (canonical combining class 230), sorted by name; it is the lookup's
//!    mark filtering set, and `i` and `j` become `idotless` and `jdotless` before one of them.
//! 3. `ccmp_latn_1`: the Latin compatibility ligatures (`fi`, the `lj` and `dž` digraphs) split
//!    into their letters, registered for `latn` after the lookups.
//!
//! Scripts other than Latin get no ccmp lookups: no source shows their form.

use icu_properties::CodePointMapData;
use icu_properties::props::CanonicalCombiningClass;

use super::{AutoContext, AutoEntry, Generator, MAX_GENERATED_BYTES};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// Most glyphs one ligature name may join; longer names are not mark ligatures.
const MAX_LIGATURE_PARTS: usize = 64;

/// The dotted letters replaced before a top mark, with their dotless forms.
const DOTLESS: [(&str, &str); 2] = [("i", "idotless"), ("j", "jdotless")];

/// The ligatures the Latin lookup splits, in the reference's order, with their letters.
const DECOMPOSED: [(&str, &[&str]); 11] = [
    ("fi", &["f", "i"]),
    ("fl", &["f", "l"]),
    ("lj", &["l", "j"]),
    ("Lj", &["L", "j"]),
    ("LJ", &["L", "J"]),
    ("nj", &["n", "j"]),
    ("Nj", &["N", "j"]),
    ("NJ", &["N", "J"]),
    ("dzcaron", &["d", "zcaron"]),
    ("Dzcaron", &["D", "zcaron"]),
    ("DZcaron", &["D", "Zcaron"]),
];

struct Ccmp;

static CCMP: Ccmp = Ccmp;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&CCMP]
}

impl Generator for Ccmp {
    fn names(&self) -> &'static [&'static str] {
        &["ccmp"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let code = ccmp_code(cx)?;
        if code.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![AutoEntry { kind: EntryKind::Feature, name: "ccmp".to_owned(), code, optional: false }])
    }
}

/// Text that refuses to grow past [`MAX_GENERATED_BYTES`].
#[derive(Default)]
pub(super) struct Code(pub(super) String);

impl Code {
    pub(super) fn push(&mut self, text: &str) -> Result<(), FeatureError> {
        if self.0.len().saturating_add(text.len()) > MAX_GENERATED_BYTES {
            return Err(FeatureError::Limit { what: "bytes of code in one generated entry", limit: MAX_GENERATED_BYTES });
        }
        self.0.push_str(text);
        Ok(())
    }

    /// Opens the next `ccmp_DFLT_n` lookup, a blank line after the one before; returns its name.
    fn start_dflt_lookup(&mut self, count: &mut usize) -> Result<String, FeatureError> {
        if !self.0.is_empty() {
            self.push("\n")?;
        }
        *count = count.saturating_add(1);
        let name = format!("ccmp_DFLT_{count}");
        self.push(&format!("lookup {name} {{\n"))?;
        Ok(name)
    }
}

fn is_mark(cx: &AutoContext, name: &str) -> bool {
    cx.glyph(name).is_some_and(|g| g.category.as_deref() == Some("Mark"))
}

/// The mark glyphs `name` joins, when it is a ligature of two or more marks of the font.
fn mark_ligature_parts<'a>(cx: &AutoContext, name: &'a str) -> Option<Vec<&'a str>> {
    if name.contains('.') || !name.contains('_') {
        return None;
    }
    let parts: Vec<&str> = name.split('_').take(MAX_LIGATURE_PARTS + 1).collect();
    if parts.len() < 2 || parts.len() > MAX_LIGATURE_PARTS {
        return None;
    }
    parts.iter().all(|p| !p.is_empty() && is_mark(cx, p)).then_some(parts)
}

/// The font's marks above, sorted by name.
fn top_marks(cx: &AutoContext) -> Vec<&str> {
    let ccc = CodePointMapData::<CanonicalCombiningClass>::new();
    let mut marks: Vec<&str> = cx
        .glyphs()
        .iter()
        .filter(|g| g.codepoints.first().is_some_and(|&cp| ccc.get32(cp) == CanonicalCombiningClass::Above))
        .map(|g| g.name.as_str())
        .collect();
    marks.sort_unstable();
    marks.dedup();
    marks
}

fn ccmp_code(cx: &AutoContext) -> Result<String, FeatureError> {
    let mut code = Code::default();
    let mut dflt = 0usize;

    let ligatures: Vec<(&str, Vec<&str>)> =
        cx.glyphs().iter().filter_map(|g| mark_ligature_parts(cx, &g.name).map(|parts| (g.name.as_str(), parts))).collect();
    if !ligatures.is_empty() {
        let name = code.start_dflt_lookup(&mut dflt)?;
        code.push("\tlookupflag 0;\n")?;
        for (ligature, parts) in &ligatures {
            code.push(&format!("\tsub {} by {ligature};\n", parts.join(" ")))?;
        }
        code.push(&format!("}} {name};\n"))?;
    }

    let dotless: Vec<(&str, &str)> = DOTLESS.iter().copied().filter(|(d, l)| cx.has(d) && cx.has(l)).collect();
    if !dotless.is_empty() {
        let marks = top_marks(cx);
        if !marks.is_empty() {
            let name = code.start_dflt_lookup(&mut dflt)?;
            code.push(&format!("\t@CombiningTopAccents = [{}];\n", marks.join(" ")))?;
            code.push("\tlookupflag UseMarkFilteringSet @CombiningTopAccents;\n")?;
            for (dotted, dotless) in dotless {
                code.push(&format!("\tsub {dotted}' @CombiningTopAccents by {dotless};\n"))?;
            }
            code.push(&format!("}} {name};\n"))?;
        }
    }

    let splits: Vec<(&str, &[&str])> = DECOMPOSED.iter().copied().filter(|(lig, parts)| cx.has(lig) && parts.iter().all(|p| cx.has(p))).collect();
    if !splits.is_empty() {
        if !code.0.is_empty() {
            code.push("\n")?;
        }
        code.push("lookup ccmp_latn_1 {\n")?;
        for (ligature, parts) in splits {
            code.push(&format!("\tsub {ligature} by {};\n", parts.join(" ")))?;
        }
        code.push("} ccmp_latn_1;\n\nscript latn;\nlookup ccmp_latn_1;\n")?;
    }
    Ok(code.0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::auto::AutoGlyph;

    fn glyph(name: &str, cp: Option<u32>, category: &str) -> AutoGlyph {
        AutoGlyph {
            name: name.to_owned(),
            codepoints: cp.into_iter().collect(),
            category: Some(category.to_owned()),
            sub_category: None,
            script: Some("latin".to_owned()),
        }
    }

    fn cx(glyphs: Vec<AutoGlyph>) -> AutoContext {
        AutoContext::new(glyphs, Arc::default()).unwrap()
    }

    fn ccmp(cx: &AutoContext) -> Option<String> {
        let mut entries = CCMP.generate(cx).unwrap();
        assert!(entries.len() <= 1);
        entries.pop().map(|e| {
            assert_eq!((e.kind, e.name.as_str(), e.optional), (EntryKind::Feature, "ccmp", false));
            e.code
        })
    }

    /// The marks of the corpus's Glyphs 4 source (Markow), in its glyph order: the top marks by
    /// code point, sorted by name; below and overlay marks and unencoded variants left out.
    #[test]
    fn ccmp_dotless_lookup_matches_the_corpus_form() {
        let mut glyphs = vec![
            glyph("i", Some(0x69), "Letter"),
            glyph("idotless", Some(0x131), "Letter"),
            glyph("iogonek", Some(0x12F), "Letter"),
            glyph("j", Some(0x6A), "Letter"),
            glyph("jdotless", Some(0x237), "Letter"),
            glyph("n", Some(0x6E), "Letter"),
            glyph("n_dieresiscomb", None, "Letter"),
        ];
        for (name, cp) in [
            ("dieresiscomb", Some(0x308)),
            ("dotaccentcomb", Some(0x307)),
            ("gravecomb", Some(0x300)),
            ("acutecomb", Some(0x301)),
            ("hungarumlautcomb", Some(0x30B)),
            ("caroncomb.alt", None),
            ("circumflexcomb", Some(0x302)),
            ("caroncomb", Some(0x30C)),
            ("brevecomb", Some(0x306)),
            ("ringcomb", Some(0x30A)),
            ("tildecomb", Some(0x303)),
            ("macroncomb", Some(0x304)),
            ("commaturnedabovecomb", Some(0x312)),
            ("commaaccentcomb", Some(0x326)),
            ("cedillacomb", Some(0x327)),
            ("ogonekcomb", Some(0x328)),
            ("strokeshortcomb", Some(0x335)),
        ] {
            glyphs.push(glyph(name, cp, "Mark"));
        }
        let expected = "lookup ccmp_DFLT_1 {\n\
            \t@CombiningTopAccents = [acutecomb brevecomb caroncomb circumflexcomb commaturnedabovecomb dieresiscomb dotaccentcomb gravecomb hungarumlautcomb macroncomb ringcomb tildecomb];\n\
            \tlookupflag UseMarkFilteringSet @CombiningTopAccents;\n\
            \tsub i' @CombiningTopAccents by idotless;\n\
            \tsub j' @CombiningTopAccents by jdotless;\n\
            } ccmp_DFLT_1;\n";
        assert_eq!(ccmp(&cx(glyphs)).as_deref(), Some(expected));
    }

    /// The corpus's mark-ligature form, and the order of the three lookups when all apply.
    #[test]
    fn ccmp_mark_ligatures_dotless_and_latin_splits_in_order() {
        let only_marks =
            cx(vec![glyph("acutecomb", Some(0x301), "Mark"), glyph("brevecomb", Some(0x306), "Mark"), glyph("brevecomb_acutecomb", None, "Mark")]);
        assert_eq!(
            ccmp(&only_marks).as_deref(),
            Some("lookup ccmp_DFLT_1 {\n\tlookupflag 0;\n\tsub brevecomb acutecomb by brevecomb_acutecomb;\n} ccmp_DFLT_1;\n")
        );

        let all = cx(vec![
            glyph("D", Some(0x44), "Letter"),
            glyph("d", Some(0x64), "Letter"),
            glyph("f", Some(0x66), "Letter"),
            glyph("i", Some(0x69), "Letter"),
            glyph("l", Some(0x6C), "Letter"),
            glyph("zcaron", Some(0x17E), "Letter"),
            glyph("Dzcaron", Some(0x1C5), "Letter"),
            glyph("fi", Some(0xFB01), "Letter"),
            glyph("fl", Some(0xFB02), "Letter"),
            glyph("idotless", Some(0x131), "Letter"),
            glyph("acutecomb", Some(0x301), "Mark"),
            glyph("brevecomb", Some(0x306), "Mark"),
            glyph("brevecomb_acutecomb", None, "Mark"),
        ]);
        let expected = "lookup ccmp_DFLT_1 {\n\
            \tlookupflag 0;\n\
            \tsub brevecomb acutecomb by brevecomb_acutecomb;\n\
            } ccmp_DFLT_1;\n\
            \n\
            lookup ccmp_DFLT_2 {\n\
            \t@CombiningTopAccents = [acutecomb brevecomb];\n\
            \tlookupflag UseMarkFilteringSet @CombiningTopAccents;\n\
            \tsub i' @CombiningTopAccents by idotless;\n\
            } ccmp_DFLT_2;\n\
            \n\
            lookup ccmp_latn_1 {\n\
            \tsub fi by f i;\n\
            \tsub fl by f l;\n\
            \tsub Dzcaron by D zcaron;\n\
            } ccmp_latn_1;\n\
            \n\
            script latn;\n\
            lookup ccmp_latn_1;\n";
        assert_eq!(ccmp(&all).as_deref(), Some(expected));
    }

    /// `anchors.ccmpAlongsideMark`: dotless forms and a top mark call for ccmp; without the
    /// dotless forms, or with a letter in the ligature, nothing is written.
    #[test]
    fn ccmp_written_only_when_the_glyphs_call_for_it() {
        let with_dotless =
            cx(vec![glyph("i", Some(0x69), "Letter"), glyph("idotless", Some(0x131), "Letter"), glyph("gravecomb", Some(0x300), "Mark")]);
        assert!(ccmp(&with_dotless).is_some_and(|c| c.contains("sub i' @CombiningTopAccents by idotless;")));

        let without = cx(vec![
            glyph("i", Some(0x69), "Letter"),
            glyph("n", Some(0x6E), "Letter"),
            glyph("gravecomb", Some(0x300), "Mark"),
            glyph("n_gravecomb", None, "Letter"),
            glyph("fi", Some(0xFB01), "Letter"),
        ]);
        assert_eq!(ccmp(&without), None);
    }
}

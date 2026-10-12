//! The members of the font's feature classes, for `name in class(X)` (`token.predicate.classFunction`).
//!
//! A class entry's code is read as glyph names, `@Other` references and `$[…]` predicate tokens;
//! `#` starts a comment, and brackets, semicolons and commas separate like spaces. Disabled
//! classes are left out. References are followed at most [`MAX_CLASS_DEPTH`] deep and a cycle
//! is an error. The work is capped: [`MAX_CLASS_MEMBERS`] names added over all classes, and
//! [`MAX_CLASS_GLYPH_TESTS`] glyphs tested by the predicate tokens inside classes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::Font;

use crate::predicate::{parse_predicate, predicate_len};
use crate::{Env, MAX_PREDICATE_BYTES, QueryError, Subject, fail, short};

/// Deepest chain of class references followed.
pub const MAX_CLASS_DEPTH: usize = 16;
/// Most member names added over all resolved classes, references counted each time.
pub const MAX_CLASS_MEMBERS: usize = 4_000_000;
/// Most glyph tests the predicate tokens inside classes may make together.
pub const MAX_CLASS_GLYPH_TESTS: usize = 16_000_000;

/// One part of a class entry's code.
enum Part<'c> {
    Name(&'c str),
    Class(&'c str),
    Predicate(&'c str),
}

/// The parts of `code`, in order. An unclosed `$[` is an error.
fn parts(code: &str) -> Result<Vec<Part<'_>>, QueryError> {
    let mut out = Vec::new();
    let mut rest = code;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | ';' | ','));
        if rest.is_empty() {
            return Ok(out);
        }
        if rest.starts_with('#') {
            rest = rest.split_once('\n').map_or("", |(_, after)| after);
            continue;
        }
        if let Some(after) = rest.strip_prefix("$[") {
            let Some(len) = predicate_len(after) else {
                return fail(format!("the predicate token `{}` is not closed with `]` within {MAX_PREDICATE_BYTES} bytes", short(rest)));
            };
            out.push(Part::Predicate(after.get(..len).unwrap_or("")));
            rest = after.get(len + 1..).unwrap_or("");
            continue;
        }
        let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | ';' | ',' | '#')).unwrap_or(rest.len());
        let word = rest.get(..end).unwrap_or("");
        rest = rest.get(end..).unwrap_or("");
        match word.strip_prefix('@') {
            Some(class) => out.push(Part::Class(class)),
            None => out.push(Part::Name(word)),
        }
    }
}

/// The members of each class, by class name. Sets are shared, so handing them out is cheap.
pub type ClassMembers = HashMap<String, Arc<HashSet<String>>>;

/// Resolves feature classes and keeps what it resolved. The caps ([`MAX_CLASS_MEMBERS`],
/// [`MAX_CLASS_GLYPH_TESTS`]) count over the resolver's whole life, so a caller that keeps one
/// for many predicates (one feature-code expansion) bounds the work of all of them together.
pub struct ClassResolver<'a> {
    font: &'a Font,
    data: &'a GlyphData,
    master: Option<&'a str>,
    done: HashMap<String, Arc<HashSet<String>>>,
    in_progress: Vec<String>,
    members: usize,
    tests: usize,
}

impl<'a> ClassResolver<'a> {
    /// Predicate tokens inside classes read `master`'s layers (the first master's when `None`).
    pub fn new(font: &'a Font, data: &'a GlyphData, master: Option<&'a str>) -> Self {
        Self { font, data, master, done: HashMap::new(), in_progress: Vec::new(), members: 0, tests: 0 }
    }

    /// The members of the enabled classes named in `names` (all of them when `None`). A name the
    /// font has no enabled class for is an error.
    pub fn classes(&mut self, names: Option<&[&str]>) -> Result<ClassMembers, QueryError> {
        match names {
            Some(names) => names.iter().map(|n| Ok((n.to_string(), self.resolve(n)?))).collect(),
            None => {
                let font = self.font;
                for c in font.features.classes.iter().filter(|c| !c.disabled) {
                    self.resolve(&c.name)?;
                }
                Ok(self.done.clone())
            }
        }
    }

    fn add(&mut self, set: &mut HashSet<String>, name: &str) -> Result<(), QueryError> {
        self.members += 1;
        if self.members > MAX_CLASS_MEMBERS {
            return fail(format!("the feature classes list more than {MAX_CLASS_MEMBERS} glyph names together"));
        }
        set.insert(name.to_string());
        Ok(())
    }

    fn resolve(&mut self, name: &str) -> Result<Arc<HashSet<String>>, QueryError> {
        if let Some(done) = self.done.get(name) {
            return Ok(Arc::clone(done));
        }
        if self.in_progress.iter().any(|n| n == name) {
            return fail(format!("the class `{}` refers to itself", short(name)));
        }
        if self.in_progress.len() >= MAX_CLASS_DEPTH {
            return fail(format!("the classes refer to each other more than {MAX_CLASS_DEPTH} levels deep"));
        }
        let font = self.font;
        let Some(entry) = font.features.classes.iter().find(|c| c.name == name && !c.disabled) else {
            return fail(format!("the font has no class `{}`", short(name)));
        };
        self.in_progress.push(name.to_string());
        let result = self.members_of(&entry.code);
        self.in_progress.pop();
        let set = Arc::new(result?);
        self.done.insert(name.to_string(), Arc::clone(&set));
        Ok(set)
    }

    fn members_of(&mut self, code: &str) -> Result<HashSet<String>, QueryError> {
        let mut set = HashSet::new();
        for part in parts(code)? {
            match part {
                Part::Name(n) => self.add(&mut set, n)?,
                Part::Class(c) => {
                    for n in self.resolve(c)?.iter() {
                        self.add(&mut set, n)?;
                    }
                }
                Part::Predicate(text) => {
                    let predicate = parse_predicate(text)?;
                    let mut classes = HashMap::new();
                    for c in predicate.classes() {
                        classes.insert(c.to_string(), self.resolve(c)?);
                    }
                    self.tests = self.tests.saturating_add(self.font.glyphs.len());
                    if self.tests > MAX_CLASS_GLYPH_TESTS {
                        return fail(format!("the predicate tokens in the feature classes test more than {MAX_CLASS_GLYPH_TESTS} glyphs together"));
                    }
                    let env = Env::new(self.font, self.data, self.master)?.with_classes(classes);
                    for g in self.font.glyphs.iter() {
                        if predicate.matches(&Subject::new(g), &env) {
                            self.add(&mut set, &g.name)?;
                        }
                    }
                }
            }
        }
        Ok(set)
    }
}

/// The members of the font's enabled feature classes named in `names` (all of them when
/// `None`), with a fresh [`ClassResolver`]. Predicate tokens inside classes read `master`'s
/// layers (the first master's when `None`) and select exporting and non-exporting glyphs alike.
/// A name the font has no enabled class for is an error.
pub fn feature_classes(font: &Font, data: &GlyphData, master: Option<&str>, names: Option<&[&str]>) -> Result<ClassMembers, QueryError> {
    ClassResolver::new(font, data, master).classes(names)
}

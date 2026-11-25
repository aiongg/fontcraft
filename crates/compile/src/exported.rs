//! What an export of one entry writes about the font besides its glyphs, without building it
//! (`font.instance.names`): the `name` records, the style classes and flags, and for a variable font
//! its named instances and axis values.
//!
//! The rules are the compiler's own ([`crate::info`], [`crate::var::names`], [`crate::var::fvar`],
//! [`crate::var::stat`]); this module runs them on a font without the glyphs they do not read. Only
//! the glyphs that text tokens name, and the glyphs their components use, are kept and interpolated.
//! The names that feature code, colour palettes and autohinting add to the `name` table need a whole
//! build and are not here, and a token that reads a glyph reads it before Filter and PreFilter run.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_fontinfo::tokens::{TokenContext, expand, has_tokens, keys};
use fontcraft_model::plist::Value;
use fontcraft_model::{Font, GlyphTable, Params, Properties, PropertyValue, Resolved, Shape, Target};

use crate::info::{self, Info};
use crate::var::stat::StatInput;
use crate::var::{self, VarAxes, VarInstance};
use crate::{CompileError, Options};

/// Most glyphs kept for text tokens.
const MAX_TOKEN_GLYPHS: usize = 1024;
/// Most texts whose unresolved tokens are listed for one entry.
pub const MAX_TOKEN_PROBLEMS: usize = 64;

/// What holds a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextScope {
    Font,
    /// A master, by id.
    Master(String),
    Instance,
}

/// A text whose tokens name nothing: it is written as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenProblem {
    pub scope: TextScope,
    /// A custom parameter rather than a General field.
    pub param: bool,
    /// The property key or parameter name; `familyName` and `styleName` for the built-in names.
    pub key: String,
    /// Which of the parameters of this name the scope has, from 0.
    pub index: usize,
    pub tokens: Vec<String>,
}

/// What the export of a static instance writes.
#[derive(Clone, Debug)]
pub struct ExportedStatic {
    /// The instance resolved without its glyphs, its tokens as written.
    pub resolved: Resolved,
    /// The font the instance is compiled from, with the glyphs its tokens read.
    pub source: Font,
    pub info: Info,
    pub tokens: Vec<TokenProblem>,
    pub warnings: Vec<String>,
}

/// What the export of a variable font setting writes.
#[derive(Clone, Debug)]
pub struct ExportedVariable {
    pub info: Info,
    /// The file name without extension.
    pub file_stem: String,
    /// The axis tags, in `fvar` order.
    pub axes: Vec<String>,
    pub instances: Vec<VarInstance>,
    /// `None` when the font turns the table off.
    pub stat: Option<StatInput>,
    pub tokens: Vec<TokenProblem>,
    pub warnings: Vec<String>,
}

/// One font, ready to say what its exports write.
pub struct ExportedNames<'a> {
    font: &'a Font,
    /// The font with the glyphs its tokens read, and no kerning.
    reduced: Font,
}

impl<'a> ExportedNames<'a> {
    pub fn new(font: &'a Font) -> Self {
        let glyphs = token_glyphs(font);
        let kept =
            if glyphs.is_empty() { Vec::new() } else { font.glyphs.as_arcs().iter().filter(|g| glyphs.contains(g.name.as_str())).cloned().collect() };
        let reduced = Font { glyphs: GlyphTable::from_glyphs(kept), kerning: Arc::default(), ..font.clone() };
        Self { font, reduced }
    }

    /// What [`crate::compile`] writes for the static instance at `index`.
    pub fn static_instance(&self, index: usize, options: &Options) -> Result<ExportedStatic, CompileError> {
        let target = Target::Instance(index);
        let mut warnings = Vec::new();
        let source = match crate::needs_instance_font(self.font, &target) {
            Some(index) => crate::instance_source(&self.reduced, index, &mut warnings)?,
            None => self.reduced.clone(),
        };
        let bare = Font { glyphs: GlyphTable::default(), ..source.clone() };
        let resolved = crate::drawing::resolve(&bare, &target)?;
        let mut named = resolved.clone();
        info::expand_tokens(Some(&source), &mut named, &mut warnings);
        let mut info = info::derive(&named, &options.style_name, &mut warnings);
        if options.version_stamp {
            info::stamp_version(&mut info, &named);
        }
        let tokens = token_problems(&source, &resolved.master, Some(index));
        Ok(ExportedStatic { resolved, source, info, tokens, warnings })
    }

    /// What [`var::compile_variable`] writes for the variable font setting at `setting`.
    pub fn variable_setting(&self, setting: usize, options: &Options) -> Result<ExportedVariable, CompileError> {
        let mut warnings = Vec::new();
        let font = var::setting_font(&self.reduced, Some(setting));
        let font: &Font = &font;
        let designspace = var::variable_designspace(font, Some(setting))?;
        let origin = designspace.origin().cloned().ok_or(CompileError::NotYetImplemented("a designspace without an origin"))?;
        let names = var::names::variable_names(font, Some(setting))?;
        let bare = Font { glyphs: GlyphTable::default(), ..font.clone() };
        let mut resolved = crate::drawing::resolve(&bare, &Target::Master(origin.id))?;
        resolved.family_name.clone_from(&names.family);
        let axes = VarAxes::from_designspace(&designspace)?;
        let tables = var::name_tables(font, &designspace, &axes, &names, var::static_names(&resolved, &names.style))?;
        info::expand_tokens(Some(font), &mut resolved, &mut warnings);
        let mut info = info::derive(&resolved, &names.style, &mut warnings);
        info::add_names(&mut info, &tables.names);
        if options.version_stamp {
            info::stamp_version(&mut info, &resolved);
        }
        Ok(ExportedVariable {
            info,
            file_stem: names.file_stem,
            axes: axes.axes.iter().map(|a| a.tag.to_string()).collect(),
            instances: tables.instances,
            stat: tables.stat_input,
            tokens: token_problems(font, &resolved.master, None),
            warnings,
        })
    }
}

/// Calls `read` with every text of `properties` and `params` a token may stand in: whether it is a parameter's, its key,
/// its place among the parameters of that name, and the text.
fn texts<'a>(properties: &'a Properties, params: &'a Params, mut read: impl FnMut(bool, &'a str, usize, &'a str)) {
    for p in &properties.0 {
        match &p.value {
            PropertyValue::Single(v) => read(false, &p.key, 0, v),
            PropertyValue::Localized(values) => values.iter().for_each(|l| read(false, &p.key, 0, &l.value)),
        }
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for p in &params.0 {
        let next = seen.entry(p.name.as_str()).or_insert(0);
        let index = *next;
        *next = index.saturating_add(1);
        if let Value::String(v) = &p.value
            && !p.disabled
        {
            read(true, &p.name, index, v.as_str());
        }
    }
}

/// The glyphs the text tokens of `font` read, with every glyph their components use in any layer: at most
/// [`MAX_TOKEN_GLYPHS`].
fn token_glyphs(font: &Font) -> HashSet<String> {
    let mut open: Vec<String> = Vec::new();
    let mut add = |text: &str| {
        if has_tokens(text) {
            let named = keys(text).into_iter().filter_map(|key| key.split_once(':')).map(|(glyph, _)| glyph.trim().to_string());
            open.extend(named.filter(|name| font.glyph(name).is_some()));
        }
    };
    add(&font.family_name);
    texts(&font.info.properties, &font.info.params, |_, _, _, text| add(text));
    for m in font.masters.iter() {
        texts(&m.properties, &m.params, |_, _, _, text| add(text));
    }
    for i in font.instances.iter() {
        add(&i.name);
        texts(&i.properties, &i.params, |_, _, _, text| add(text));
    }
    let mut kept = HashSet::new();
    while let Some(name) = open.pop() {
        if kept.len() >= MAX_TOKEN_GLYPHS {
            break;
        }
        let Some(glyph) = font.glyph(&name) else { continue };
        if !kept.insert(name) {
            continue;
        }
        let bases: BTreeSet<&str> = glyph
            .layers
            .iter()
            .flat_map(|l| l.shapes.iter())
            .filter_map(|s| match s {
                Shape::Component(c) => Some(c.glyph.as_str()),
                _ => None,
            })
            .collect();
        open.extend(bases.into_iter().filter(|base| !kept.contains(*base)).map(str::to_string));
    }
    kept
}

/// The texts whose tokens name nothing when `font` is exported at its master `master`, or at its instance `instance`. A
/// text that a narrower scope replaces is not written and not listed.
fn token_problems(font: &Font, master: &str, instance: Option<usize>) -> Vec<TokenProblem> {
    let instance = instance.and_then(|i| font.instances.get(i));
    let master = font.masters.iter().find(|m| m.id == master);
    let width = |glyph: &str, master: &str| font.glyph(glyph).and_then(|g| g.layer(master)).map(|l| l.width);
    let ctx = TokenContext { font, master, instance, glyph_width: &width };
    let mut out: Vec<TokenProblem> = Vec::new();
    let mut check = |scope: TextScope, param: bool, key: &str, index: usize, text: &str| {
        if !has_tokens(text) {
            return;
        }
        let tokens = expand(text, &ctx).unresolved;
        if tokens.is_empty() {
            return;
        }
        match out.iter_mut().find(|p| p.scope == scope && p.param == param && p.key == key && p.index == index) {
            Some(problem) => {
                let new: Vec<String> = tokens.into_iter().filter(|t| !problem.tokens.contains(t)).collect();
                problem.tokens.extend(new);
            }
            None if out.len() < MAX_TOKEN_PROBLEMS => out.push(TokenProblem { scope, param, key: key.to_string(), index, tokens }),
            None => {}
        }
    };
    let holds = |properties: &Properties, params: &Params, param: bool, key: &str| {
        if param { params.0.iter().any(|p| !p.disabled && p.name == key) } else { properties.get(key).is_some() }
    };
    let by_instance = |param: bool, key: &str| instance.is_some_and(|i| holds(&i.properties, &i.params, param, key));
    let by_master = |param: bool, key: &str| master.is_some_and(|m| holds(&m.properties, &m.params, param, key));
    if let Some(i) = instance {
        check(TextScope::Instance, false, "styleName", 0, &i.name);
        texts(&i.properties, &i.params, |param, key, index, text| check(TextScope::Instance, param, key, index, text));
    }
    if let Some(m) = master {
        texts(&m.properties, &m.params, |param, key, index, text| {
            if !by_instance(param, key) {
                check(TextScope::Master(m.id.clone()), param, key, index, text);
            }
        });
    }
    check(TextScope::Font, false, "familyName", 0, &font.family_name);
    texts(&font.info.properties, &font.info.params, |param, key, index, text| {
        if !by_instance(param, key) && !by_master(param, key) {
            check(TextScope::Font, param, key, index, text);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Instance, Path};
    use skrifa::FontRef;
    use skrifa::raw::TableProvider;

    use super::*;
    use crate::{Flavour, compile};

    fn records(bytes: &[u8]) -> Vec<(u16, u16, u16, u16, String)> {
        let font = FontRef::new(bytes).unwrap();
        let name = font.name().unwrap();
        let data = name.string_data();
        let text = |r: &skrifa::raw::tables::name::NameRecord| r.string(data).unwrap().chars().collect::<String>();
        let mut out: Vec<_> =
            name.name_record().iter().map(|r| (r.platform_id(), r.encoding_id(), r.language_id(), r.name_id().to_u16(), text(r))).collect();
        out.sort();
        out
    }

    fn font() -> Font {
        let mut f = Font::new("Slice Sans");
        for (name, width) in [("A", 600.0), ("B", 500.0)] {
            if f.glyph(name).is_none() {
                f.add_glyph(name, Vec::new(), width).unwrap();
            }
            let mut g = f.glyph_mut(name).unwrap();
            let layer = g.layers.first_mut().unwrap();
            layer.width = width;
            layer.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 450.0, 700.0)));
        }
        Arc::make_mut(&mut f.info).properties.set("designers", "Ada {{{familyName}}} {{{nothing}}}", true);
        let mut bold = Instance { name: "Bold".into(), exports: true, is_bold: true, ..Default::default() };
        bold.properties.set("descriptions", "{{{A:width}}} units, {{{no such}}}", true);
        Arc::make_mut(&mut f.instances).push(bold);
        f
    }

    #[test]
    fn the_names_are_those_of_the_compiled_instance_and_bad_tokens_name_their_row() {
        let f = font();
        let options = Options::default();
        let names = ExportedNames::new(&f);
        assert_eq!(names.reduced.glyphs.len(), 1, "only the glyph a token reads is kept");
        let out = names.static_instance(0, &options).unwrap();
        let built = compile(&f, &Target::Instance(0), Flavour::TrueType, &options).unwrap();
        let mut given: Vec<_> = out.info.names.iter().map(|r| (r.platform, r.encoding, r.language, r.id, r.text.clone())).collect();
        given.sort();
        assert_eq!(given, records(&built.bytes));
        assert_eq!(out.info.name(10).as_deref(), Some("600 units, {{{no such}}}"));
        let rows: Vec<_> = out.tokens.iter().map(|t| (t.scope.clone(), t.key.as_str(), t.tokens.clone())).collect();
        assert_eq!(
            rows,
            [(TextScope::Instance, "descriptions", vec!["no such".to_string()]), (TextScope::Font, "designers", vec!["nothing".to_string()])]
        );
    }
}

//! The glyph predicate language (`docs/spec/12-feature-code.md` § 3, Grammar): the text of the
//! Custom smart-filter rule, of `Groups*.plist` predicates and of `$[…]` feature-code tokens.
//! Smart-filter rule trees compile to the same [`Expr`], so one evaluator serves them all.
//!
//! ```text
//! predicate  = term { OR term }            OR also `||`
//! term       = factor { AND factor }       AND also `&&`
//! factor     = NOT factor | "(" predicate ")" | true | false | comparison
//! comparison = key-path operator value | value IN key-path | key-path IN class(Name)
//!            | key-path IN "{" value { "," value } "}" | key-path BETWEEN "{" number "," number "}"
//!            | key-path                     a yes-or-no attribute alone means `== true`
//! operator   = == = != <> < <= =< > >= => beginswith endswith contains like matches
//! value      = "text" | 'text' | number | true | false | yes | no | nil | word
//! ```
//!
//! Keywords and operator words are read in any letter case. NOT binds tighter than AND, and AND
//! tighter than OR. `matches` takes a regular expression that must match the whole value; `like`
//! takes a pattern in which `*` stands for any run of characters and `?` for one character. On a
//! list attribute (`tags`) a comparison holds when it holds for one element; `!=` holds when `==`
//! holds for none.

use std::borrow::Cow;
use std::fmt;

use fontcraft_model::{Alignment, Font, Glyph, Layer, Shape};
use regex::{Regex, RegexBuilder};

use crate::{
    Env, MAX_PATTERN_BYTES, MAX_PREDICATE_BYTES, MAX_PREDICATE_DEPTH, MAX_PREDICATE_TOKENS, MAX_REGEX_NESTING, QueryError, REGEX_SIZE_LIMIT, Subject,
    fail, short,
};

/// A compiled glyph predicate.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// Every part holds; with no parts, always true.
    All(Vec<Expr>),
    /// At least one part holds; with no parts, never.
    Any(Vec<Expr>),
    Not(Box<Expr>),
    Test(Test),
}

/// One comparison of a glyph attribute with a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Test {
    pub attr: Attr,
    pub op: Op,
    pub value: Literal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BeginsWith,
    EndsWith,
    Contains,
    /// A regular expression that matches the whole value.
    Matches,
    /// A pattern with `*` and `?` wildcards that matches the whole value.
    Like,
    /// `name in class(X)`: the value is the class name.
    InClass,
    /// `attr in {a, b}`: the value is a [`Literal::List`] and the attribute equals one of them.
    InList,
    /// `attr between {low, high}`: a number within the two, both included.
    Between,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Str(String),
    Num(f64),
    Bool(bool),
    /// No value: an empty text, no colour label.
    Nil,
    Pattern(Pattern),
    /// The values of `in {…}` and `between {…}`.
    List(Vec<Literal>),
}

/// A regular expression, or a `like` pattern with wildcards, anchored to the whole value and
/// built within [`REGEX_SIZE_LIMIT`].
#[derive(Clone, Debug)]
pub struct Pattern {
    source: String,
    case_insensitive: bool,
    wildcard: bool,
    regex: Regex,
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.case_insensitive == other.case_insensitive && self.wildcard == other.wildcard
    }
}

/// Compiles `compiled`, the regular expression `source` wrapped as the caller needs, within
/// [`MAX_PATTERN_BYTES`], [`REGEX_SIZE_LIMIT`] and [`MAX_REGEX_NESTING`]. Errors name `source`.
pub(crate) fn limited_regex(source: &str, compiled: &str, case_insensitive: bool) -> Result<Regex, QueryError> {
    if source.len() > MAX_PATTERN_BYTES {
        return fail(format!("the regular expression is {} bytes long; at most {MAX_PATTERN_BYTES} are allowed", source.len()));
    }
    RegexBuilder::new(compiled)
        .case_insensitive(case_insensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .nest_limit(MAX_REGEX_NESTING)
        .build()
        .map_err(|e| QueryError(format!("the regular expression `{}` cannot be used: {e}", short(source))))
}

impl Pattern {
    pub fn new(source: &str, case_insensitive: bool) -> Result<Pattern, QueryError> {
        let regex = limited_regex(source, &format!("^(?:{source})$"), case_insensitive)?;
        Ok(Pattern { source: source.to_string(), case_insensitive, wildcard: false, regex })
    }

    /// A `like` pattern: `*` matches any run of characters, `?` one character, everything else
    /// itself.
    pub fn wildcard(source: &str, case_insensitive: bool) -> Result<Pattern, QueryError> {
        let mut compiled = String::from("^(?s:");
        for c in source.chars() {
            match c {
                '*' => compiled.push_str(".*"),
                '?' => compiled.push('.'),
                c => compiled.push_str(&regex::escape(c.encode_utf8(&mut [0; 4]))),
            }
        }
        compiled.push_str(")$");
        let regex = limited_regex(source, &compiled, case_insensitive)?;
        Ok(Pattern { source: source.to_string(), case_insensitive, wildcard: true, regex })
    }

    pub fn is_wildcard(&self) -> bool {
        self.wildcard
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

/// A glyph attribute a predicate can read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attr {
    Name,
    ProductionName,
    Category,
    SubCategory,
    Script,
    Note,
    Case,
    Direction,
    Tags,
    /// The first stored code point as upper-case hexadecimal, at least four digits.
    Unicode,
    /// The character of the first stored code point.
    CharString,
    /// The effective sort name.
    SortName,
    /// The effective description (the character's Unicode name for encoded glyphs).
    Description,
    /// The glyph's kerning group on one side.
    KerningGroup(Side),
    /// The glyph's own metrics key for one metric.
    MetricsKey(KeyOf),
    /// The glyph's colour label number (0 to 11); none when unlabelled.
    ColorIndex,
    Export,
    HasUnicode,
    HasCustomGlyphInfo,
    /// A metrics key on the glyph or on any of its layers.
    HasMetricsKeys,
    /// The glyph is in the environment's out-of-sync set (`Env::with_out_of_sync`).
    MetricsKeysOutOfSync,
    /// The glyph is not in the environment's incompatible set (`Env::with_incompatible`).
    MasterCompatible,
    /// Any of the glyph's kerning groups is set.
    HasKerningGroups,
    /// A yes-or-no property read from the glyph's layers or metadata.
    Flag(Flag),
    CountOfUnicodes,
    CountOfLayers,
    /// Paths on the first master's layer.
    CountOfPaths,
    /// Components on the first master's layer.
    CountOfComponents,
    CountOfTags,
    /// The smart-glyph properties the glyph defines.
    CountOfPartsSettings,
    /// An attribute of the shown master's layer.
    Layer(LayerAttr),
}

/// A side of a glyph, for kerning groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Which of the glyph's metrics keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOf {
    Left,
    Right,
    Width,
    Top,
    Bottom,
    VertWidth,
}

/// Yes-or-no glyph properties. Those that name layer content hold when one layer of the glyph,
/// of any kind, has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    /// Every master layer has automatically aligned components.
    IsAligned,
    /// Stem or TrueType hints (corner components, caps and the like do not count).
    HasHints,
    HasPostScriptHints,
    HasTrueTypeHints,
    /// Corner components.
    HasCorners,
    HasComponents,
    HasAnnotations,
    /// Intermediate, alternate, colour or smart-glyph layers.
    HasSpecialLayers,
    /// Colour palette layers.
    IsColorGlyph,
    /// Colour layers of any kind.
    IsAnyColorGlyph,
    /// Bitmap colour layers.
    IsAppleColorGlyph,
    IsSvgColorGlyph,
    IsFullColorGlyph,
    /// Smart-glyph properties or pole layers.
    IsSmartGlyph,
    Locked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerAttr {
    ColorIndex,
    HasAnnotations,
    /// The layer has automatically aligned components.
    IsAligned,
    HasComponents,
    /// Corner components on the layer.
    HasCorners,
    HasAnchors,
    /// A component of the named glyph is on the layer.
    HasComponent(String),
    CountOfPaths,
    CountOfComponents,
    CountOfAnchors,
    Width,
    /// The bounds of the outline with components resolved; none for an empty layer.
    MinX,
    MaxX,
    MinY,
    MaxY,
    /// The left side bearing: the left bound.
    Lsb,
    /// The right side bearing: the width less the right bound.
    Rsb,
    AnchorX(String),
    AnchorY(String),
}

/// The key path prefix of [`LayerAttr::HasComponent`]: `layer.hasComponent.<glyph name>`.
const HAS_COMPONENT: &str = "layer.hasComponent.";

/// The case values `case` compares with.
pub const CASE_VALUES: &[&str] = &["noCase", "upper", "lower", "smallCaps", "minor", "other"];
/// The direction values `direction` compares with.
pub const DIRECTION_VALUES: &[&str] = &["LTR", "RTL", "BIDI", "VTR", "VTL"];

/// Glyph properties of the reference's scripting interface that describe editor state or need
/// work FontCraft does not do while filtering.
const NOT_YET: &[&str] = &[
    "mastersCompatible",
    "changeCount",
    "lastChange",
    "justLocked",
    "outlineHasChanges",
    "hasAlignedWidth",
    "isCornerGlyph",
    "isHangulKeyGlyph",
    "baseString",
    "glyphDataEntryString",
];

/// The key paths with a fixed meaning, and the attribute each reads. Several spellings of the
/// reference's scripting interface read the same attribute.
const PATHS: &[(&str, Attr)] = &[
    ("name", Attr::Name),
    ("productionName", Attr::ProductionName),
    ("production", Attr::ProductionName),
    ("category", Attr::Category),
    ("subCategory", Attr::SubCategory),
    ("script", Attr::Script),
    ("note", Attr::Note),
    ("case", Attr::Case),
    ("direction", Attr::Direction),
    ("tags", Attr::Tags),
    ("unicode", Attr::Unicode),
    ("string", Attr::CharString),
    ("charString", Attr::CharString),
    ("unicodeString", Attr::CharString),
    ("sortName", Attr::SortName),
    ("description", Attr::Description),
    ("charName", Attr::Description),
    ("leftKerningGroup", Attr::KerningGroup(Side::Left)),
    ("rightKerningGroup", Attr::KerningGroup(Side::Right)),
    ("topKerningGroup", Attr::KerningGroup(Side::Top)),
    ("bottomKerningGroup", Attr::KerningGroup(Side::Bottom)),
    ("leftMetricsKey", Attr::MetricsKey(KeyOf::Left)),
    ("rightMetricsKey", Attr::MetricsKey(KeyOf::Right)),
    ("widthMetricsKey", Attr::MetricsKey(KeyOf::Width)),
    ("topMetricsKey", Attr::MetricsKey(KeyOf::Top)),
    ("bottomMetricsKey", Attr::MetricsKey(KeyOf::Bottom)),
    ("vertWidthMetricsKey", Attr::MetricsKey(KeyOf::VertWidth)),
    ("colorIndex", Attr::ColorIndex),
    ("export", Attr::Export),
    ("hasUnicode", Attr::HasUnicode),
    ("hasCustomGlyphInfo", Attr::HasCustomGlyphInfo),
    ("hasMetricsKeys", Attr::HasMetricsKeys),
    ("metricsKeysOutOfSync", Attr::MetricsKeysOutOfSync),
    ("masterCompatible", Attr::MasterCompatible),
    ("hasKerningGroups", Attr::HasKerningGroups),
    ("isAligned", Attr::Flag(Flag::IsAligned)),
    ("hasHints", Attr::Flag(Flag::HasHints)),
    ("hasPostScriptHints", Attr::Flag(Flag::HasPostScriptHints)),
    ("hasTrueTypeHints", Attr::Flag(Flag::HasTrueTypeHints)),
    ("hasCorners", Attr::Flag(Flag::HasCorners)),
    ("hasComponents", Attr::Flag(Flag::HasComponents)),
    ("hasAnnotations", Attr::Flag(Flag::HasAnnotations)),
    ("hasSpecialLayers", Attr::Flag(Flag::HasSpecialLayers)),
    ("isColorGlyph", Attr::Flag(Flag::IsColorGlyph)),
    ("isColorPaletteGlyph", Attr::Flag(Flag::IsColorGlyph)),
    ("isAnyColorGlyph", Attr::Flag(Flag::IsAnyColorGlyph)),
    ("isAppleColorGlyph", Attr::Flag(Flag::IsAppleColorGlyph)),
    ("isSVGColorGlyph", Attr::Flag(Flag::IsSvgColorGlyph)),
    ("isFullColorGlyph", Attr::Flag(Flag::IsFullColorGlyph)),
    ("isSmartGlyph", Attr::Flag(Flag::IsSmartGlyph)),
    ("locked", Attr::Flag(Flag::Locked)),
    ("countOfUnicodes", Attr::CountOfUnicodes),
    ("countOfLayers", Attr::CountOfLayers),
    ("countOfPaths", Attr::CountOfPaths),
    ("countOfComponents", Attr::CountOfComponents),
    ("countOfTags", Attr::CountOfTags),
    ("countOfPartsSettings", Attr::CountOfPartsSettings),
    ("layer.colorIndex", Attr::Layer(LayerAttr::ColorIndex)),
    ("layer.hasAnnotations", Attr::Layer(LayerAttr::HasAnnotations)),
    ("layer.isAligned", Attr::Layer(LayerAttr::IsAligned)),
    ("layer.hasComponents", Attr::Layer(LayerAttr::HasComponents)),
    ("layer.hasCorners", Attr::Layer(LayerAttr::HasCorners)),
    ("layer.hasAnchors", Attr::Layer(LayerAttr::HasAnchors)),
    ("layer.countOfPaths", Attr::Layer(LayerAttr::CountOfPaths)),
    ("layer.countOfComponents", Attr::Layer(LayerAttr::CountOfComponents)),
    ("layer.countOfAnchors", Attr::Layer(LayerAttr::CountOfAnchors)),
    ("layer.width", Attr::Layer(LayerAttr::Width)),
    ("layer.minX", Attr::Layer(LayerAttr::MinX)),
    ("layer.maxX", Attr::Layer(LayerAttr::MaxX)),
    ("layer.minY", Attr::Layer(LayerAttr::MinY)),
    ("layer.maxY", Attr::Layer(LayerAttr::MaxY)),
    ("layer.LSB", Attr::Layer(LayerAttr::Lsb)),
    ("layer.RSB", Attr::Layer(LayerAttr::Rsb)),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Text,
    TextList,
    Choice(&'static [&'static str]),
    Number,
    Bool,
}

impl Attr {
    pub fn parse(path: &str) -> Result<Attr, QueryError> {
        if let Some((_, attr)) = PATHS.iter().find(|(p, _)| *p == path) {
            return Ok(attr.clone());
        }
        if NOT_YET.contains(&path) {
            return fail(format!("the attribute `{path}` is not supported yet"));
        }
        if let Some(name) = path.strip_prefix(HAS_COMPONENT)
            && !name.is_empty()
        {
            return Ok(Attr::Layer(LayerAttr::HasComponent(name.to_string())));
        }
        let anchor = path.strip_prefix("layer.anchors.");
        match (anchor.and_then(|a| a.strip_suffix(".x")), anchor.and_then(|a| a.strip_suffix(".y"))) {
            (Some(name), _) if !name.is_empty() => Ok(Attr::Layer(LayerAttr::AnchorX(name.to_string()))),
            (_, Some(name)) if !name.is_empty() => Ok(Attr::Layer(LayerAttr::AnchorY(name.to_string()))),
            _ => fail(format!("unknown attribute `{}`", short(path))),
        }
    }

    /// The key path the predicate language spells this attribute with.
    pub fn path(&self) -> String {
        match self {
            Attr::Layer(LayerAttr::AnchorX(n)) => format!("layer.anchors.{n}.x"),
            Attr::Layer(LayerAttr::AnchorY(n)) => format!("layer.anchors.{n}.y"),
            Attr::Layer(LayerAttr::HasComponent(n)) => format!("{HAS_COMPONENT}{n}"),
            // The first spelling in `PATHS` is the one written.
            attr => PATHS.iter().find(|(_, a)| a == attr).map_or_else(String::new, |(p, _)| (*p).to_string()),
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Attr::Name
            | Attr::ProductionName
            | Attr::Category
            | Attr::SubCategory
            | Attr::Script
            | Attr::Note
            | Attr::Unicode
            | Attr::CharString
            | Attr::SortName
            | Attr::Description
            | Attr::KerningGroup(_)
            | Attr::MetricsKey(_) => Kind::Text,
            Attr::Tags => Kind::TextList,
            Attr::Case => Kind::Choice(CASE_VALUES),
            Attr::Direction => Kind::Choice(DIRECTION_VALUES),
            Attr::Export
            | Attr::HasUnicode
            | Attr::HasCustomGlyphInfo
            | Attr::HasMetricsKeys
            | Attr::MetricsKeysOutOfSync
            | Attr::MasterCompatible
            | Attr::HasKerningGroups
            | Attr::Flag(_)
            | Attr::Layer(
                LayerAttr::HasAnnotations
                | LayerAttr::IsAligned
                | LayerAttr::HasComponents
                | LayerAttr::HasCorners
                | LayerAttr::HasAnchors
                | LayerAttr::HasComponent(_),
            ) => Kind::Bool,
            Attr::ColorIndex
            | Attr::CountOfUnicodes
            | Attr::CountOfLayers
            | Attr::CountOfPaths
            | Attr::CountOfComponents
            | Attr::CountOfTags
            | Attr::CountOfPartsSettings
            | Attr::Layer(_) => Kind::Number,
        }
    }
}

impl fmt::Display for Attr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path())
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::BeginsWith => "beginswith",
            Op::EndsWith => "endswith",
            Op::Contains => "contains",
            Op::Matches => "matches",
            Op::Like => "like",
            Op::InClass | Op::InList => "in",
            Op::Between => "between",
        })
    }
}

fn quote(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        if c == '"' || c == '\\' {
            f.write_str("\\")?;
        }
        write!(f, "{c}")?;
    }
    f.write_str("\"")
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Str(s) => quote(f, s),
            Literal::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => write!(f, "{}", *n as i64),
            Literal::Num(n) => write!(f, "{n}"),
            Literal::Bool(b) => write!(f, "{b}"),
            Literal::Nil => f.write_str("nil"),
            Literal::Pattern(p) => quote(f, p.source()),
            Literal::List(items) => {
                f.write_str("{")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("}")
            }
        }
    }
}

impl fmt::Display for Test {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.attr, self.op, &self.value) {
            (Attr::Tags, Op::Eq, Literal::Str(s)) => {
                quote(f, s)?;
                f.write_str(" in tags")
            }
            (attr, Op::InClass, Literal::Str(class)) => write!(f, "{attr} in class({class})"),
            (attr, op, value) => write!(f, "{attr} {op} {value}"),
        }
    }
}

impl fmt::Display for Expr {
    /// The predicate as text that [`parse_predicate`] reads back to an equivalent predicate.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_expr(self, f, 0)
    }
}

/// Precedence: OR 1, AND 2, everything else 3. A part whose precedence is below `min` gets
/// parentheses.
fn write_expr(e: &Expr, f: &mut fmt::Formatter<'_>, min: u8) -> fmt::Result {
    match e {
        Expr::All(v) if v.is_empty() => f.write_str("true"),
        Expr::Any(v) if v.is_empty() => f.write_str("false"),
        Expr::All(v) | Expr::Any(v) if v.len() == 1 => match v.first() {
            Some(only) => write_expr(only, f, min),
            None => Ok(()),
        },
        Expr::All(v) | Expr::Any(v) => {
            let (prec, joiner) = if matches!(e, Expr::All(_)) { (2, " AND ") } else { (1, " OR ") };
            if prec < min {
                f.write_str("(")?;
            }
            for (i, part) in v.iter().enumerate() {
                if i > 0 {
                    f.write_str(joiner)?;
                }
                write_expr(part, f, prec + 1)?;
            }
            if prec < min {
                f.write_str(")")?;
            }
            Ok(())
        }
        Expr::Not(inner) => {
            f.write_str("NOT ")?;
            write_expr(inner, f, 3)
        }
        Expr::Test(t) => write!(f, "{t}"),
    }
}

// ── Evaluation ──────────────────────────────────────────────────────────────────────────────────

impl Expr {
    pub fn matches(&self, s: &Subject, env: &Env) -> bool {
        match self {
            Expr::All(v) => v.iter().all(|e| e.matches(s, env)),
            Expr::Any(v) => v.iter().any(|e| e.matches(s, env)),
            Expr::Not(e) => !e.matches(s, env),
            Expr::Test(t) => t.matches(s, env),
        }
    }

    /// The class names `name in class(…)` tests read, each once, in the order they appear.
    pub fn classes(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            match e {
                Expr::All(v) | Expr::Any(v) => stack.extend(v.iter().rev()),
                Expr::Not(inner) => stack.push(inner),
                Expr::Test(Test { op: Op::InClass, value: Literal::Str(class), .. }) => {
                    if !out.contains(&class.as_str()) {
                        out.push(class.as_str());
                    }
                }
                Expr::Test(_) => {}
            }
        }
        out
    }
}

/// An attribute's value for one glyph.
enum Val<'s> {
    Text(Option<Cow<'s, str>>),
    List(&'s [String]),
    Choice(&'s str),
    /// `None`: no value. A colour given as RGBA, not as a label number, reads as NaN, which equals
    /// no number.
    Num(Option<f64>),
    Bool(bool),
}

fn count(n: usize) -> Val<'static> {
    Val::Num(Some(n as f64))
}

fn text(t: Option<&str>) -> Val<'_> {
    Val::Text(t.map(Cow::Borrowed))
}

/// Whether `layer` has a component the editor places automatically: the font's Disable
/// Automatic Alignment setting is off, and a component's alignment is forced or horizontal, or
/// left to the usual rule on a layer without paths.
fn layer_aligned(font: &Font, layer: &Layer) -> bool {
    if font.info.settings.disables_automatic_alignment == Some(true) {
        return false;
    }
    let has_paths = layer.shapes.iter().any(|s| matches!(s, Shape::Path(_) | Shape::Raw(_)));
    layer.shapes.iter().any(|s| match s {
        Shape::Component(c) => match c.info.alignment {
            Alignment::Disabled => false,
            Alignment::Forced | Alignment::Horizontal => true,
            Alignment::Default => !has_paths,
        },
        _ => false,
    })
}

/// Hint kinds that are drawing parts rather than hints.
const NOT_HINTS: &[&str] = &["Corner", "Cap", "Brush", "Segment", "Line"];

fn flag(f: Flag, g: &Glyph, font: &Font) -> bool {
    let any_layer = |test: &dyn Fn(&Layer) -> bool| g.layers.iter().any(test);
    let any_hint = |test: &dyn Fn(Option<&str>) -> bool| g.layers.iter().any(|l| l.info.hints.iter().any(|h| test(h.kind())));
    let is_hint = |k: Option<&str>| k.is_none_or(|k| !NOT_HINTS.contains(&k));
    let is_tt = |k: Option<&str>| k.is_some_and(|k| k.starts_with("TT"));
    match f {
        Flag::IsAligned => {
            let mut masters = g.layers.iter().filter(|l| l.is_master_layer()).peekable();
            masters.peek().is_some() && masters.all(|l| layer_aligned(font, l))
        }
        Flag::HasHints => any_hint(&is_hint),
        Flag::HasPostScriptHints => any_hint(&|k| is_hint(k) && !is_tt(k)),
        Flag::HasTrueTypeHints => any_hint(&is_tt),
        Flag::HasCorners => any_hint(&|k| k == Some("Corner")),
        Flag::HasComponents => any_layer(&|l| l.shapes.iter().any(|s| matches!(s, Shape::Component(_)))),
        Flag::HasAnnotations => any_layer(&|l| !l.info.annotations.is_empty()),
        Flag::HasSpecialLayers => any_layer(&|l| {
            let a = &l.info.attrs;
            !a.coordinates.is_empty()
                || !a.axis_rules.is_empty()
                || a.color_palette.is_some()
                || a.full_color
                || a.sbix_size.is_some()
                || a.svg
                || !a.smart_poles.is_empty()
        }),
        Flag::IsColorGlyph => any_layer(&|l| l.info.attrs.color_palette.is_some()),
        Flag::IsAnyColorGlyph => any_layer(&|l| {
            let a = &l.info.attrs;
            a.color_palette.is_some() || a.full_color || a.sbix_size.is_some() || a.svg
        }),
        Flag::IsAppleColorGlyph => any_layer(&|l| l.info.attrs.sbix_size.is_some()),
        Flag::IsSvgColorGlyph => any_layer(&|l| l.info.attrs.svg),
        Flag::IsFullColorGlyph => any_layer(&|l| l.info.attrs.full_color),
        Flag::IsSmartGlyph => !g.info.parts.is_empty() || any_layer(&|l| !l.info.attrs.smart_poles.is_empty()),
        Flag::Locked => g.info.locked,
    }
}

/// The bounds (min x, min y, max x, max y) of `glyph`'s outline on `master`, components resolved
/// and curve extremes included; `None` for an empty outline or one that cannot be drawn.
pub(crate) fn outline_bounds(font: &Font, glyph: &Glyph, master: &str) -> Option<[f64; 4]> {
    let path = font.outline_of(glyph, master).ok()?;
    let mut out: Option<[f64; 4]> = None;
    let mut add = |x: f64, y: f64| {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        let [min_x, min_y, max_x, max_y] = out.get_or_insert([x, y, x, y]);
        *min_x = min_x.min(x);
        *min_y = min_y.min(y);
        *max_x = max_x.max(x);
        *max_y = max_y.max(y);
    };
    for seg in path.segments() {
        let c = seg.to_cubic();
        let (xs, ys) = ([c.p0.x, c.p1.x, c.p2.x, c.p3.x], [c.p0.y, c.p1.y, c.p2.y, c.p3.y]);
        add(c.p0.x, c.p0.y);
        add(c.p3.x, c.p3.y);
        for t in cubic_extremes(xs) {
            add(cubic_at(xs, t), c.p0.y);
        }
        for t in cubic_extremes(ys) {
            add(c.p0.x, cubic_at(ys, t));
        }
    }
    out
}

fn cubic_at([p0, p1, p2, p3]: [f64; 4], t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

/// The parameters strictly between 0 and 1 where one coordinate of a cubic Bézier curve turns.
fn cubic_extremes([p0, p1, p2, p3]: [f64; 4]) -> Vec<f64> {
    // The derivative over 3 is a t² + b t + c.
    let (d0, d1, d2) = (p1 - p0, p2 - p1, p3 - p2);
    let (a, b, c) = (d0 - 2.0 * d1 + d2, 2.0 * (d1 - d0), d0);
    let mut roots = Vec::new();
    if a.abs() < 1e-12 {
        if b.abs() > 1e-12 {
            roots.push(-c / b);
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let r = disc.sqrt();
            roots.push((-b + r) / (2.0 * a));
            roots.push((-b - r) / (2.0 * a));
        }
    }
    roots.retain(|t| t.is_finite() && *t > 0.0 && *t < 1.0);
    roots
}

impl Test {
    fn value<'s>(&self, s: &'s Subject, env: &Env) -> Val<'s> {
        use LayerAttr as L;
        use fontcraft_model::ColorLabel;
        let g = s.glyph;
        let colour = |c: Option<ColorLabel>| match c {
            Some(ColorLabel::Index(i)) => Some(f64::from(i)),
            Some(ColorLabel::Rgba(_)) => Some(f64::NAN),
            None => None,
        };
        let paths = |l: &Layer| l.shapes.iter().filter(|x| matches!(x, Shape::Path(_))).count();
        let components = |l: &Layer| l.shapes.iter().filter(|x| matches!(x, Shape::Component(_))).count();
        let owned = |t: Option<String>| Val::Text(t.map(Cow::Owned));
        match &self.attr {
            Attr::Name => text(Some(g.name.as_str())),
            Attr::ProductionName => text(s.props(env).production_name.as_deref()),
            Attr::Category => text(s.props(env).category.as_deref()),
            Attr::SubCategory => text(s.props(env).sub_category.as_deref()),
            Attr::Script => text(s.props(env).script.as_deref()),
            Attr::SortName => text(s.props(env).sort_name.as_deref()),
            Attr::Description => text(s.props(env).description.as_deref()),
            Attr::Note => text(Some(g.info.note.as_str())),
            Attr::Unicode => owned(g.unicodes.first().map(|u| format!("{u:04X}"))),
            Attr::CharString => owned(g.unicodes.first().and_then(|&u| char::from_u32(u)).map(String::from)),
            Attr::KerningGroup(side) => text(match side {
                Side::Left => g.kern_groups.left.as_deref(),
                Side::Right => g.kern_groups.right.as_deref(),
                Side::Top => g.kern_groups.top.as_deref(),
                Side::Bottom => g.kern_groups.bottom.as_deref(),
            }),
            Attr::MetricsKey(key) => {
                let k = &g.info.metrics_keys;
                text(match key {
                    KeyOf::Left => k.left.as_deref(),
                    KeyOf::Right => k.right.as_deref(),
                    KeyOf::Width => k.width.as_deref(),
                    KeyOf::Top => k.top.as_deref(),
                    KeyOf::Bottom => k.bottom.as_deref(),
                    KeyOf::VertWidth => k.vert_width.as_deref(),
                })
            }
            Attr::Case => Val::Choice(s.props(env).case.as_deref().unwrap_or("noCase")),
            Attr::Direction => Val::Choice(s.props(env).direction.as_deref().unwrap_or("LTR")),
            Attr::Tags => Val::List(g.info.tags.as_slice()),
            Attr::ColorIndex => Val::Num(colour(g.info.color)),
            Attr::Export => Val::Bool(g.export),
            Attr::HasUnicode => Val::Bool(!g.unicodes.is_empty()),
            Attr::HasCustomGlyphInfo => Val::Bool(s.props(env).has_custom_info),
            Attr::HasMetricsKeys => Val::Bool(!g.info.metrics_keys.is_empty() || g.layers.iter().any(|l| !l.info.metrics_keys.is_empty())),
            Attr::MetricsKeysOutOfSync => Val::Bool(env.is_out_of_sync(&g.name)),
            Attr::MasterCompatible => Val::Bool(!env.is_incompatible(&g.name)),
            Attr::HasKerningGroups => Val::Bool(!g.kern_groups.is_empty()),
            Attr::Flag(f) => Val::Bool(flag(*f, g, env.font)),
            Attr::CountOfUnicodes => count(g.unicodes.len()),
            Attr::CountOfLayers => count(g.layers.len()),
            Attr::CountOfPaths => count(env.first_layer(g).map_or(0, paths)),
            Attr::CountOfComponents => count(env.first_layer(g).map_or(0, components)),
            Attr::CountOfTags => count(g.info.tags.len()),
            Attr::CountOfPartsSettings => count(g.info.parts.len()),
            Attr::Layer(la) => {
                let layer = env.shown_layer(g);
                let bound = |i: usize| Val::Num(s.bounds(env).and_then(|b| b.get(i).copied()));
                match la {
                    L::ColorIndex => Val::Num(layer.and_then(|l| colour(l.info.color))),
                    L::HasAnnotations => Val::Bool(layer.is_some_and(|l| !l.info.annotations.is_empty())),
                    L::IsAligned => Val::Bool(layer.is_some_and(|l| layer_aligned(env.font, l))),
                    L::HasComponents => Val::Bool(layer.is_some_and(|l| components(l) > 0)),
                    L::HasCorners => Val::Bool(layer.is_some_and(|l| l.info.hints.iter().any(|h| h.kind() == Some("Corner")))),
                    L::HasAnchors => Val::Bool(layer.is_some_and(|l| !l.anchors.is_empty())),
                    L::HasComponent(n) => {
                        Val::Bool(layer.is_some_and(|l| l.shapes.iter().any(|s| matches!(s, Shape::Component(c) if &c.glyph == n))))
                    }
                    L::CountOfPaths => count(layer.map_or(0, paths)),
                    L::CountOfComponents => count(layer.map_or(0, components)),
                    L::CountOfAnchors => count(layer.map_or(0, |l| l.anchors.len())),
                    L::Width => Val::Num(layer.map(|l| l.width)),
                    L::MinX | L::Lsb => bound(0),
                    L::MinY => bound(1),
                    L::MaxX => bound(2),
                    L::MaxY => bound(3),
                    L::Rsb => Val::Num(layer.zip(s.bounds(env)).map(|(l, [_, _, max_x, _])| l.width - max_x)),
                    L::AnchorX(n) => Val::Num(layer.and_then(|l| l.anchors.iter().find(|a| &a.name == n)).map(|a| a.x)),
                    L::AnchorY(n) => Val::Num(layer.and_then(|l| l.anchors.iter().find(|a| &a.name == n)).map(|a| a.y)),
                }
            }
        }
    }

    pub fn matches(&self, s: &Subject, env: &Env) -> bool {
        match self.value(s, env) {
            Val::Text(v) => {
                let v = v.as_deref().unwrap_or("");
                match (self.op, &self.value) {
                    (Op::InClass, Literal::Str(class)) => env.in_class(class, v),
                    (Op::InList, Literal::List(items)) => items.iter().any(|item| text_holds(Op::Eq, item, v)),
                    (op, value) => text_holds(op, value, v),
                }
            }
            Val::List(items) => match (self.op, &self.value) {
                (Op::Ne, value) => !items.iter().any(|t| text_holds(Op::Eq, value, t)),
                (Op::Eq, Literal::Nil) => items.is_empty(),
                (Op::InList, Literal::List(wanted)) => items.iter().any(|t| wanted.iter().any(|w| text_holds(Op::Eq, w, t))),
                (op, value) => items.iter().any(|t| text_holds(op, value, t)),
            },
            Val::Choice(v) => match (self.op, &self.value) {
                (Op::Eq, Literal::Str(want)) => v == want.as_str(),
                (Op::Ne, Literal::Str(want)) => v != want.as_str(),
                (Op::InList, Literal::List(items)) => items.iter().any(|i| matches!(i, Literal::Str(w) if w == v)),
                _ => false,
            },
            Val::Num(v) => match (self.op, &self.value) {
                (Op::Eq, Literal::Nil) => v.is_none(),
                (Op::Ne, Literal::Nil) => v.is_some(),
                (Op::InList, Literal::List(items)) => v.is_some_and(|x| items.iter().any(|i| matches!(i, Literal::Num(n) if *n == x))),
                (Op::Between, Literal::List(items)) => match (v, items.as_slice()) {
                    (Some(x), [Literal::Num(low), Literal::Num(high)]) => *low <= x && x <= *high,
                    _ => false,
                },
                (op, Literal::Num(n)) => match (op, v) {
                    (Op::Ne, None) => true,
                    (_, None) => false,
                    (Op::Eq, Some(x)) => x == *n,
                    (Op::Ne, Some(x)) => x != *n,
                    (Op::Lt, Some(x)) => x < *n,
                    (Op::Le, Some(x)) => x <= *n,
                    (Op::Gt, Some(x)) => x > *n,
                    (Op::Ge, Some(x)) => x >= *n,
                    _ => false,
                },
                _ => false,
            },
            Val::Bool(v) => match (self.op, &self.value) {
                (Op::Eq, Literal::Bool(b)) => v == *b,
                (Op::Ne, Literal::Bool(b)) => v != *b,
                (Op::InList, Literal::List(items)) => items.iter().any(|i| *i == Literal::Bool(v)),
                _ => false,
            },
        }
    }
}

/// A text comparison; an absent text reads as empty, and `nil` equals the empty text.
fn text_holds(op: Op, value: &Literal, text: &str) -> bool {
    match (op, value) {
        (Op::Eq, Literal::Nil) => text.is_empty(),
        (Op::Ne, Literal::Nil) => !text.is_empty(),
        (Op::Eq, Literal::Str(s)) => text == s,
        (Op::Ne, Literal::Str(s)) => text != s,
        (Op::BeginsWith, Literal::Str(s)) => text.starts_with(s.as_str()),
        (Op::EndsWith, Literal::Str(s)) => text.ends_with(s.as_str()),
        (Op::Contains, Literal::Str(s)) => text.contains(s.as_str()),
        (Op::Matches | Op::Like, Literal::Pattern(p)) => p.is_match(text),
        _ => false,
    }
}

// ── Building tests with type checks ─────────────────────────────────────────────────────────────

impl Test {
    /// A comparison, checked against the attribute's type: text attributes take text, numbers
    /// take numbers, choices take one of their values. `matches` compiles its expression here.
    pub fn new(attr: Attr, op: Op, value: Literal) -> Result<Test, QueryError> {
        let shown = format!("`{attr} {op} {value}`");
        let refuse = |why: &str| fail(format!("{shown}: {why}"));
        if let Literal::List(items) = &value {
            let fits = match (op, items.as_slice()) {
                (Op::InList, items) => !items.is_empty(),
                (Op::Between, [Literal::Num(_), Literal::Num(_)]) => true,
                _ => false,
            };
            if !fits {
                return refuse(if op == Op::Between {
                    "`between` takes two numbers in braces"
                } else {
                    "a list in braces needs `in` and at least one value"
                });
            }
            let mut checked = Vec::with_capacity(items.len());
            for item in items {
                let item_op = if op == Op::Between { Op::Ge } else { Op::Eq };
                checked.push(Test::new(attr.clone(), item_op, item.clone())?.value);
            }
            return Ok(Test { attr, op, value: Literal::List(checked) });
        }
        let value = match (attr.kind(), op, value) {
            (Kind::Text | Kind::TextList, Op::Eq | Op::Ne, v @ (Literal::Str(_) | Literal::Nil)) => v,
            (Kind::Text | Kind::TextList, Op::Like, Literal::Str(s)) => Literal::Pattern(Pattern::wildcard(&s, false)?),
            (Kind::Text | Kind::TextList, Op::Like, v @ Literal::Pattern(_)) => v,
            (Kind::Text | Kind::TextList, Op::BeginsWith | Op::EndsWith | Op::Contains, v @ Literal::Str(_)) => v,
            (Kind::Text | Kind::TextList, Op::Matches, Literal::Str(s)) => Literal::Pattern(Pattern::new(&s, false)?),
            (Kind::Text | Kind::TextList, Op::Matches, v @ Literal::Pattern(_)) => v,
            (Kind::Text, Op::InClass, v @ Literal::Str(_)) if attr == Attr::Name => v,
            (Kind::Text | Kind::TextList, _, _) => {
                return refuse("text attributes take ==, !=, beginswith, endswith, contains, like or matches and a text");
            }
            (Kind::Choice(values), Op::Eq | Op::Ne, Literal::Nil) if values.contains(&"noCase") => Literal::Str("noCase".into()),
            (Kind::Choice(values), Op::Eq | Op::Ne, Literal::Str(s)) => {
                if !values.contains(&s.as_str()) {
                    return refuse(&format!("the value must be one of {}", values.join(", ")));
                }
                Literal::Str(s)
            }
            (Kind::Choice(values), _, _) => return refuse(&format!("compare with == or != and one of {}", values.join(", "))),
            (Kind::Number, Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge, v @ Literal::Num(_)) => v,
            (Kind::Number, Op::Eq | Op::Ne, v @ Literal::Nil) => v,
            (Kind::Number, _, _) => return refuse("number attributes take ==, !=, <, <=, > or >= and a number"),
            (Kind::Bool, Op::Eq | Op::Ne, v @ Literal::Bool(_)) => v,
            (Kind::Bool, _, _) => return refuse("yes-or-no attributes take == or != and true or false"),
        };
        Ok(Test { attr, op, value })
    }
}

// ── Parsing ─────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Open,
    Close,
    BraceOpen,
    BraceClose,
    Comma,
    Str(String),
    Num(f64),
    Word(String),
    Cmp(Op),
    And,
    Or,
    Not,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Open => f.write_str("`(`"),
            Tok::Close => f.write_str("`)`"),
            Tok::BraceOpen => f.write_str("`{`"),
            Tok::BraceClose => f.write_str("`}`"),
            Tok::Comma => f.write_str("`,`"),
            Tok::Str(s) => write!(f, "the text \"{}\"", short(s)),
            Tok::Num(n) => write!(f, "the number {n}"),
            Tok::Word(w) => write!(f, "`{}`", short(w)),
            Tok::Cmp(op) => write!(f, "`{op}`"),
            Tok::And => f.write_str("AND"),
            Tok::Or => f.write_str("OR"),
            Tok::Not => f.write_str("NOT"),
        }
    }
}

fn tokenize(text: &str) -> Result<Vec<Tok>, QueryError> {
    let mut toks = Vec::new();
    let mut it = text.char_indices().peekable();
    while let Some((at, c)) = it.next() {
        if toks.len() >= MAX_PREDICATE_TOKENS {
            return fail(format!("the predicate has more than {MAX_PREDICATE_TOKENS} parts"));
        }
        let next = it.peek().map(|&(_, n)| n);
        let tok = match c {
            c if c.is_whitespace() => continue,
            '(' => Tok::Open,
            ')' => Tok::Close,
            '{' => Tok::BraceOpen,
            '}' => Tok::BraceClose,
            ',' => Tok::Comma,
            '=' => match next {
                Some('=') => {
                    it.next();
                    Tok::Cmp(Op::Eq)
                }
                Some('>') => {
                    it.next();
                    Tok::Cmp(Op::Ge)
                }
                Some('<') => {
                    it.next();
                    Tok::Cmp(Op::Le)
                }
                _ => Tok::Cmp(Op::Eq),
            },
            '!' if next == Some('=') => {
                it.next();
                Tok::Cmp(Op::Ne)
            }
            '!' => Tok::Not,
            '<' => match next {
                Some('=') => {
                    it.next();
                    Tok::Cmp(Op::Le)
                }
                Some('>') => {
                    it.next();
                    Tok::Cmp(Op::Ne)
                }
                _ => Tok::Cmp(Op::Lt),
            },
            '>' => {
                if next == Some('=') {
                    it.next();
                    Tok::Cmp(Op::Ge)
                } else {
                    Tok::Cmp(Op::Gt)
                }
            }
            '&' if next == Some('&') => {
                it.next();
                Tok::And
            }
            '|' if next == Some('|') => {
                it.next();
                Tok::Or
            }
            '"' | '\'' => {
                let mut s = String::new();
                let mut closed = false;
                while let Some((_, d)) = it.next() {
                    if d == c {
                        closed = true;
                        break;
                    }
                    if d == '\\' {
                        match it.next() {
                            Some((_, e)) => s.push(e),
                            None => break,
                        }
                    } else {
                        s.push(d);
                    }
                }
                if !closed {
                    return fail(format!("the text starting at byte {at} has no closing quote"));
                }
                Tok::Str(s)
            }
            c if c.is_ascii_digit()
                || (c == '-' && next.is_some_and(|n| n.is_ascii_digit() || n == '.'))
                || (c == '.' && next.is_some_and(|n| n.is_ascii_digit())) =>
            {
                let mut s = String::from(c);
                while let Some(&(_, d)) = it.peek() {
                    let exponent_sign = (d == '-' || d == '+') && s.ends_with(['e', 'E']);
                    if d.is_ascii_digit() || d == '.' || d == 'e' || d == 'E' || exponent_sign {
                        s.push(d);
                        it.next();
                    } else {
                        break;
                    }
                }
                match s.parse::<f64>() {
                    Ok(n) if n.is_finite() => Tok::Num(n),
                    _ => return fail(format!("`{}` is not a number", short(&s))),
                }
            }
            c if c.is_alphanumeric() || c == '_' => {
                let mut s = String::from(c);
                while let Some(&(_, d)) = it.peek() {
                    if d.is_alphanumeric() || d == '_' || d == '.' || d == '-' {
                        s.push(d);
                        it.next();
                    } else {
                        break;
                    }
                }
                match s.to_ascii_lowercase().as_str() {
                    "and" => Tok::And,
                    "or" => Tok::Or,
                    "not" => Tok::Not,
                    "beginswith" => Tok::Cmp(Op::BeginsWith),
                    "endswith" => Tok::Cmp(Op::EndsWith),
                    "contains" => Tok::Cmp(Op::Contains),
                    "matches" => Tok::Cmp(Op::Matches),
                    "like" => Tok::Cmp(Op::Like),
                    "between" => Tok::Cmp(Op::Between),
                    _ => Tok::Word(s),
                }
            }
            other => return fail(format!("unexpected `{other}` at byte {at}")),
        };
        toks.push(tok);
    }
    Ok(toks)
}

/// The value a token spells, or the token when it is not a value.
fn value_of(t: Tok) -> Result<Literal, Tok> {
    Ok(match t {
        Tok::Str(s) => Literal::Str(s),
        Tok::Num(n) => Literal::Num(n),
        Tok::Word(w) if w.eq_ignore_ascii_case("true") || w.eq_ignore_ascii_case("yes") => Literal::Bool(true),
        Tok::Word(w) if w.eq_ignore_ascii_case("false") || w.eq_ignore_ascii_case("no") => Literal::Bool(false),
        Tok::Word(w) if w.eq_ignore_ascii_case("nil") || w.eq_ignore_ascii_case("null") => Literal::Nil,
        Tok::Word(w) => Literal::Str(w),
        t => return Err(t),
    })
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

fn is_word(t: Option<&Tok>, word: &str) -> bool {
    matches!(t, Some(Tok::Word(w)) if w.eq_ignore_ascii_case(word))
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn found(&self) -> String {
        match self.peek() {
            Some(t) => t.to_string(),
            None => "the end".into(),
        }
    }

    fn or(&mut self) -> Result<Expr, QueryError> {
        let mut parts = vec![self.and()?];
        while self.peek() == Some(&Tok::Or) {
            self.pos += 1;
            parts.push(self.and()?);
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap_or(Expr::Any(Vec::new())) } else { Expr::Any(parts) })
    }

    fn and(&mut self) -> Result<Expr, QueryError> {
        let mut parts = vec![self.factor()?];
        while self.peek() == Some(&Tok::And) {
            self.pos += 1;
            parts.push(self.factor()?);
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap_or(Expr::All(Vec::new())) } else { Expr::All(parts) })
    }

    fn factor(&mut self) -> Result<Expr, QueryError> {
        self.depth += 1;
        if self.depth > MAX_PREDICATE_DEPTH {
            return fail(format!("the predicate is nested more than {MAX_PREDICATE_DEPTH} levels deep"));
        }
        let e = self.factor_inner();
        self.depth -= 1;
        e
    }

    fn factor_inner(&mut self) -> Result<Expr, QueryError> {
        match self.bump() {
            Some(Tok::Not) => Ok(Expr::Not(Box::new(self.factor()?))),
            Some(Tok::Open) => {
                let e = self.or()?;
                match self.bump() {
                    Some(Tok::Close) => Ok(e),
                    _ => fail("a `(` has no matching `)`"),
                }
            }
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("true") || w.eq_ignore_ascii_case("truepredicate") => Ok(Expr::All(Vec::new())),
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("false") || w.eq_ignore_ascii_case("falsepredicate") => Ok(Expr::Any(Vec::new())),
            Some(Tok::Word(w)) => self.comparison(&w),
            Some(Tok::Str(s)) => self.membership(Literal::Str(s)),
            Some(Tok::Num(n)) => self.membership(Literal::Num(n)),
            Some(t) => fail(format!("expected a comparison, found {t}")),
            None => fail("the predicate ends where a comparison was expected"),
        }
    }

    /// `"draft" in tags`
    fn membership(&mut self, value: Literal) -> Result<Expr, QueryError> {
        if !is_word(self.peek(), "in") {
            return fail(format!("expected `in` after {value}, found {}", self.found()));
        }
        self.pos += 1;
        let attr = match self.bump() {
            Some(Tok::Word(w)) => Attr::parse(&w)?,
            _ => return fail(format!("expected an attribute after `in`, found {}", self.found())),
        };
        if attr.kind() != Kind::TextList {
            return fail(format!("`{value} in {attr}`: `in` needs a list attribute such as tags"));
        }
        let value = match value {
            Literal::Num(n) => Literal::Str(Literal::Num(n).to_string()),
            v => v,
        };
        Ok(Expr::Test(Test::new(attr, Op::Eq, value)?))
    }

    fn comparison(&mut self, path: &str) -> Result<Expr, QueryError> {
        let attr = Attr::parse(path)?;
        if is_word(self.peek(), "in") {
            self.pos += 1;
            if self.peek() == Some(&Tok::BraceOpen) {
                let items = self.aggregate(&attr)?;
                return Ok(Expr::Test(Test::new(attr, Op::InList, Literal::List(items))?));
            }
            if !is_word(self.peek(), "class") {
                return fail(format!("`{attr} in` must be followed by class(Name) or values in braces, found {}", self.found()));
            }
            self.pos += 1;
            let class = match (self.bump(), self.bump(), self.bump()) {
                (Some(Tok::Open), Some(Tok::Word(c)), Some(Tok::Close)) => c,
                _ => return fail(format!("`{attr} in class(…)` needs one class name in parentheses")),
            };
            return Ok(Expr::Test(Test::new(attr, Op::InClass, Literal::Str(class))?));
        }
        let op = match self.peek() {
            Some(Tok::Cmp(op)) => *op,
            _ if attr.kind() == Kind::Bool => return Ok(Expr::Test(Test::new(attr, Op::Eq, Literal::Bool(true))?)),
            _ => return fail(format!("expected an operator after `{attr}`, found {}", self.found())),
        };
        self.pos += 1;
        if op == Op::Between {
            if self.peek() != Some(&Tok::BraceOpen) {
                return fail(format!("`{attr} between` must be followed by two numbers in braces, found {}", self.found()));
            }
            let items = self.aggregate(&attr)?;
            return Ok(Expr::Test(Test::new(attr, op, Literal::List(items))?));
        }
        let value = match self.bump().map(value_of) {
            Some(Ok(v)) => v,
            Some(Err(t)) => return fail(format!("expected a value after `{attr} {op}`, found {t}")),
            None => return fail(format!("expected a value after `{attr} {op}`")),
        };
        Ok(Expr::Test(Test::new(attr, op, value)?))
    }

    /// `{a, b, …}`, the opening brace next.
    fn aggregate(&mut self, attr: &Attr) -> Result<Vec<Literal>, QueryError> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            match self.bump().map(value_of) {
                Some(Ok(v)) => items.push(v),
                Some(Err(t)) => return fail(format!("expected a value in the braces after `{attr}`, found {t}")),
                None => return fail(format!("the braces after `{attr}` are not closed")),
            }
            match self.bump() {
                Some(Tok::Comma) => {}
                Some(Tok::BraceClose) => return Ok(items),
                Some(t) => return fail(format!("expected `,` or `}}` in the braces after `{attr}`, found {t}")),
                None => return fail(format!("the braces after `{attr}` are not closed")),
            }
        }
    }
}

/// Reads predicate text (the Custom rule, a `Groups*.plist` predicate).
pub fn parse_predicate(text: &str) -> Result<Expr, QueryError> {
    if text.len() > MAX_PREDICATE_BYTES {
        return fail(format!("the predicate is {} bytes long; at most {MAX_PREDICATE_BYTES} are allowed", text.len()));
    }
    let toks = tokenize(text)?;
    if toks.is_empty() {
        return fail("the predicate is empty");
    }
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let e = p.or()?;
    if p.pos < p.toks.len() {
        return fail(format!("unexpected {} after the end of the predicate", p.found()));
    }
    Ok(e)
}

/// A parsed glyph predicate: the language of `$[…]` feature-code tokens and of the Custom
/// smart-filter rule (`token.predicate.*`, `smartFilter.rule.custom`).
pub type Predicate = Expr;

/// The length in bytes of the predicate at the start of `text`, the text after a `$[`: up to
/// the `]` that closes the token. Brackets inside quoted text (`name matches "[a-z]"`) and
/// nested brackets do not close it. `None` when nothing closes it within
/// [`MAX_PREDICATE_BYTES`].
pub fn predicate_len(text: &str) -> Option<usize> {
    predicate_len_within(text, MAX_PREDICATE_BYTES)
}

/// [`predicate_len`] for a predicate at most `max` bytes long. The scan stops after `max + 1`
/// bytes, so a caller that tries every `$[` of a text does bounded work for each.
pub fn predicate_len_within(text: &str, max: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    // Every byte that matters is ASCII, and no byte of a multi-byte character is.
    for (at, &c) in text.as_bytes().iter().enumerate().take(max.saturating_add(1)) {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => quote = Some(c),
            b'[' => depth += 1,
            b']' if depth == 0 => return Some(at),
            b']' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Reads predicate text ([`parse_predicate`]).
pub fn parse(text: &str) -> Result<Predicate, QueryError> {
    parse_predicate(text)
}

/// The names of the glyphs of `env`'s font that `predicate` selects, in font order. Layer
/// attributes read `env`'s master. An error names what cannot be evaluated.
pub fn evaluate(predicate: &Predicate, env: &Env) -> Result<Vec<String>, QueryError> {
    Ok(env.font.glyphs.iter().filter(|g| predicate.matches(&Subject::new(g), env)).map(|g| g.name.clone()).collect())
}

#[cfg(test)]
#[path = "predicate_tests.rs"]
mod tests;

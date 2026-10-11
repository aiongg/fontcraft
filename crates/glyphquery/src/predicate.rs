//! The glyph predicate language (`docs/spec/12-feature-code.md` § 3, Grammar): the text of the
//! Custom smart-filter rule, of `Groups*.plist` predicates and, in M8, of `$[…]` feature tokens.
//! Smart-filter rule trees compile to the same [`Expr`], so one evaluator serves them all.
//!
//! ```text
//! predicate  = term { OR term }            OR also `||`
//! term       = factor { AND factor }       AND also `&&`
//! factor     = NOT factor | "(" predicate ")" | true | false | comparison
//! comparison = key-path operator value | value IN key-path | key-path IN class(Name)
//!            | key-path                     a yes-or-no attribute alone means `== true`
//! operator   = == = != <> < <= > >= beginswith endswith contains matches
//! value      = "text" | 'text' | number | true | false | nil | word
//! ```
//!
//! Keywords and operator words are read in any letter case. `matches` takes a regular expression
//! that must match the whole value. On a list attribute (`tags`) a comparison holds when it holds
//! for one element; `!=` holds when `==` holds for none.

use std::fmt;

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
    /// `name in class(X)`: the value is the class name.
    InClass,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Str(String),
    Num(f64),
    Bool(bool),
    /// No value: an empty text, no colour label.
    Nil,
    Pattern(Pattern),
}

/// A regular expression anchored to the whole value, built within [`REGEX_SIZE_LIMIT`].
#[derive(Clone, Debug)]
pub struct Pattern {
    source: String,
    case_insensitive: bool,
    regex: Regex,
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.case_insensitive == other.case_insensitive
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
        Ok(Pattern { source: source.to_string(), case_insensitive, regex })
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
    /// The glyph's colour label number (0 to 11); none when unlabelled.
    ColorIndex,
    Export,
    HasUnicode,
    HasCustomGlyphInfo,
    /// A metrics key on the glyph or on any of its layers.
    HasMetricsKeys,
    /// The glyph is in the environment's out-of-sync set (`Env::with_out_of_sync`).
    MetricsKeysOutOfSync,
    /// Any of the glyph's kerning groups is set.
    HasKerningGroups,
    CountOfUnicodes,
    CountOfLayers,
    /// Paths on the first master's layer.
    CountOfPaths,
    /// Components on the first master's layer.
    CountOfComponents,
    /// An attribute of the shown master's layer.
    Layer(LayerAttr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerAttr {
    ColorIndex,
    HasAnnotations,
    CountOfPaths,
    CountOfComponents,
    CountOfAnchors,
    Width,
    AnchorX(String),
    AnchorY(String),
}

/// The case values `case` compares with.
pub const CASE_VALUES: &[&str] = &["noCase", "upper", "lower", "smallCaps", "minor", "other"];
/// The direction values `direction` compares with.
pub const DIRECTION_VALUES: &[&str] = &["LTR", "RTL", "BIDI", "VTR", "VTL"];

/// Attributes the handbook names that FontCraft does not evaluate yet.
const NOT_YET: &[&str] = &["isAligned", "hasHints", "isColorGlyph", "layer.maxX", "layer.minX", "layer.maxY", "layer.minY", "layer.isAligned"];

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
        use LayerAttr as L;
        Ok(match path {
            "name" => Attr::Name,
            "productionName" => Attr::ProductionName,
            "category" => Attr::Category,
            "subCategory" => Attr::SubCategory,
            "script" => Attr::Script,
            "note" => Attr::Note,
            "case" => Attr::Case,
            "direction" => Attr::Direction,
            "tags" => Attr::Tags,
            "colorIndex" => Attr::ColorIndex,
            "export" => Attr::Export,
            "hasUnicode" => Attr::HasUnicode,
            "hasCustomGlyphInfo" => Attr::HasCustomGlyphInfo,
            "hasMetricsKeys" => Attr::HasMetricsKeys,
            "metricsKeysOutOfSync" => Attr::MetricsKeysOutOfSync,
            "hasKerningGroups" => Attr::HasKerningGroups,
            "countOfUnicodes" => Attr::CountOfUnicodes,
            "countOfLayers" => Attr::CountOfLayers,
            "countOfPaths" => Attr::CountOfPaths,
            "countOfComponents" => Attr::CountOfComponents,
            "layer.colorIndex" => Attr::Layer(L::ColorIndex),
            "layer.hasAnnotations" => Attr::Layer(L::HasAnnotations),
            "layer.countOfPaths" => Attr::Layer(L::CountOfPaths),
            "layer.countOfComponents" => Attr::Layer(L::CountOfComponents),
            "layer.countOfAnchors" => Attr::Layer(L::CountOfAnchors),
            "layer.width" => Attr::Layer(L::Width),
            _ if NOT_YET.contains(&path) => return fail(format!("the attribute `{path}` is not supported yet")),
            _ => {
                let anchor = path.strip_prefix("layer.anchors.");
                match (anchor.and_then(|a| a.strip_suffix(".x")), anchor.and_then(|a| a.strip_suffix(".y"))) {
                    (Some(name), _) if !name.is_empty() => Attr::Layer(L::AnchorX(name.to_string())),
                    (_, Some(name)) if !name.is_empty() => Attr::Layer(L::AnchorY(name.to_string())),
                    _ => return fail(format!("unknown attribute `{}`", short(path))),
                }
            }
        })
    }

    /// The key path the predicate language spells this attribute with.
    pub fn path(&self) -> String {
        use LayerAttr as L;
        match self {
            Attr::Name => "name".into(),
            Attr::ProductionName => "productionName".into(),
            Attr::Category => "category".into(),
            Attr::SubCategory => "subCategory".into(),
            Attr::Script => "script".into(),
            Attr::Note => "note".into(),
            Attr::Case => "case".into(),
            Attr::Direction => "direction".into(),
            Attr::Tags => "tags".into(),
            Attr::ColorIndex => "colorIndex".into(),
            Attr::Export => "export".into(),
            Attr::HasUnicode => "hasUnicode".into(),
            Attr::HasCustomGlyphInfo => "hasCustomGlyphInfo".into(),
            Attr::HasMetricsKeys => "hasMetricsKeys".into(),
            Attr::MetricsKeysOutOfSync => "metricsKeysOutOfSync".into(),
            Attr::HasKerningGroups => "hasKerningGroups".into(),
            Attr::CountOfUnicodes => "countOfUnicodes".into(),
            Attr::CountOfLayers => "countOfLayers".into(),
            Attr::CountOfPaths => "countOfPaths".into(),
            Attr::CountOfComponents => "countOfComponents".into(),
            Attr::Layer(L::ColorIndex) => "layer.colorIndex".into(),
            Attr::Layer(L::HasAnnotations) => "layer.hasAnnotations".into(),
            Attr::Layer(L::CountOfPaths) => "layer.countOfPaths".into(),
            Attr::Layer(L::CountOfComponents) => "layer.countOfComponents".into(),
            Attr::Layer(L::CountOfAnchors) => "layer.countOfAnchors".into(),
            Attr::Layer(L::Width) => "layer.width".into(),
            Attr::Layer(L::AnchorX(n)) => format!("layer.anchors.{n}.x"),
            Attr::Layer(L::AnchorY(n)) => format!("layer.anchors.{n}.y"),
        }
    }

    fn kind(&self) -> Kind {
        use LayerAttr as L;
        match self {
            Attr::Name | Attr::ProductionName | Attr::Category | Attr::SubCategory | Attr::Script | Attr::Note => Kind::Text,
            Attr::Tags => Kind::TextList,
            Attr::Case => Kind::Choice(CASE_VALUES),
            Attr::Direction => Kind::Choice(DIRECTION_VALUES),
            Attr::Export
            | Attr::HasUnicode
            | Attr::HasCustomGlyphInfo
            | Attr::HasMetricsKeys
            | Attr::MetricsKeysOutOfSync
            | Attr::HasKerningGroups
            | Attr::Layer(L::HasAnnotations) => Kind::Bool,
            Attr::ColorIndex
            | Attr::CountOfUnicodes
            | Attr::CountOfLayers
            | Attr::CountOfPaths
            | Attr::CountOfComponents
            | Attr::Layer(L::ColorIndex | L::CountOfPaths | L::CountOfComponents | L::CountOfAnchors | L::Width | L::AnchorX(_) | L::AnchorY(_)) => {
                Kind::Number
            }
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
            Op::InClass => "in",
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
}

/// An attribute's value for one glyph.
enum Val<'s> {
    Text(Option<&'s str>),
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

impl Test {
    fn value<'s>(&self, s: &'s Subject, env: &Env) -> Val<'s> {
        use LayerAttr as L;
        use fontcraft_model::{ColorLabel, Shape};
        let g = s.glyph;
        let colour = |c: Option<ColorLabel>| match c {
            Some(ColorLabel::Index(i)) => Some(f64::from(i)),
            Some(ColorLabel::Rgba(_)) => Some(f64::NAN),
            None => None,
        };
        let paths = |l: &fontcraft_model::Layer| l.shapes.iter().filter(|x| matches!(x, Shape::Path(_))).count();
        let components = |l: &fontcraft_model::Layer| l.shapes.iter().filter(|x| matches!(x, Shape::Component(_))).count();
        match &self.attr {
            Attr::Name => Val::Text(Some(g.name.as_str())),
            Attr::ProductionName => Val::Text(s.props(env).production_name.as_deref()),
            Attr::Category => Val::Text(s.props(env).category.as_deref()),
            Attr::SubCategory => Val::Text(s.props(env).sub_category.as_deref()),
            Attr::Script => Val::Text(s.props(env).script.as_deref()),
            Attr::Note => Val::Text(Some(g.info.note.as_str())),
            Attr::Case => Val::Choice(s.props(env).case.as_deref().unwrap_or("noCase")),
            Attr::Direction => Val::Choice(s.props(env).direction.as_deref().unwrap_or("LTR")),
            Attr::Tags => Val::List(g.info.tags.as_slice()),
            Attr::ColorIndex => Val::Num(colour(g.info.color)),
            Attr::Export => Val::Bool(g.export),
            Attr::HasUnicode => Val::Bool(!g.unicodes.is_empty()),
            Attr::HasCustomGlyphInfo => Val::Bool(s.props(env).has_custom_info),
            Attr::HasMetricsKeys => Val::Bool(!g.info.metrics_keys.is_empty() || g.layers.iter().any(|l| !l.info.metrics_keys.is_empty())),
            Attr::MetricsKeysOutOfSync => Val::Bool(env.is_out_of_sync(&g.name)),
            Attr::HasKerningGroups => Val::Bool(!g.kern_groups.is_empty()),
            Attr::CountOfUnicodes => count(g.unicodes.len()),
            Attr::CountOfLayers => count(g.layers.len()),
            Attr::CountOfPaths => count(env.first_layer(g).map_or(0, paths)),
            Attr::CountOfComponents => count(env.first_layer(g).map_or(0, components)),
            Attr::Layer(la) => {
                let layer = env.shown_layer(g);
                match la {
                    L::ColorIndex => Val::Num(layer.and_then(|l| colour(l.info.color))),
                    L::HasAnnotations => Val::Bool(layer.is_some_and(|l| !l.info.annotations.is_empty())),
                    L::CountOfPaths => count(layer.map_or(0, paths)),
                    L::CountOfComponents => count(layer.map_or(0, components)),
                    L::CountOfAnchors => count(layer.map_or(0, |l| l.anchors.len())),
                    L::Width => Val::Num(layer.map(|l| l.width)),
                    L::AnchorX(n) => Val::Num(layer.and_then(|l| l.anchors.iter().find(|a| &a.name == n)).map(|a| a.x)),
                    L::AnchorY(n) => Val::Num(layer.and_then(|l| l.anchors.iter().find(|a| &a.name == n)).map(|a| a.y)),
                }
            }
        }
    }

    pub fn matches(&self, s: &Subject, env: &Env) -> bool {
        match self.value(s, env) {
            Val::Text(v) => {
                if self.op == Op::InClass {
                    return matches!(&self.value, Literal::Str(class) if env.in_class(class, v.unwrap_or("")));
                }
                text_holds(self.op, &self.value, v.unwrap_or(""))
            }
            Val::List(items) => match self.op {
                Op::Ne => !items.iter().any(|t| text_holds(Op::Eq, &self.value, t)),
                Op::Eq if self.value == Literal::Nil => items.is_empty(),
                op => items.iter().any(|t| text_holds(op, &self.value, t)),
            },
            Val::Choice(v) => match (self.op, &self.value) {
                (Op::Eq, Literal::Str(want)) => v == want.as_str(),
                (Op::Ne, Literal::Str(want)) => v != want.as_str(),
                _ => false,
            },
            Val::Num(v) => match (self.op, &self.value) {
                (Op::Eq, Literal::Nil) => v.is_none(),
                (Op::Ne, Literal::Nil) => v.is_some(),
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
        (Op::Matches, Literal::Pattern(p)) => p.is_match(text),
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
        let value = match (attr.kind(), op, value) {
            (Kind::Text | Kind::TextList, Op::Eq | Op::Ne, v @ (Literal::Str(_) | Literal::Nil)) => v,
            (Kind::Text | Kind::TextList, Op::BeginsWith | Op::EndsWith | Op::Contains, v @ Literal::Str(_)) => v,
            (Kind::Text | Kind::TextList, Op::Matches, Literal::Str(s)) => Literal::Pattern(Pattern::new(&s, false)?),
            (Kind::Text | Kind::TextList, Op::Matches, v @ Literal::Pattern(_)) => v,
            (Kind::Text, Op::InClass, v @ Literal::Str(_)) if attr == Attr::Name => v,
            (Kind::Text | Kind::TextList, _, _) => {
                return refuse("text attributes take ==, !=, beginswith, endswith, contains or matches and a text");
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
            '=' => {
                if next == Some('=') {
                    it.next();
                }
                Tok::Cmp(Op::Eq)
            }
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
                    _ => Tok::Word(s),
                }
            }
            other => return fail(format!("unexpected `{other}` at byte {at}")),
        };
        toks.push(tok);
    }
    Ok(toks)
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
            if !is_word(self.peek(), "class") {
                return fail(format!("`{attr} in` must be followed by class(Name), found {}", self.found()));
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
        let value = match self.bump() {
            Some(Tok::Str(s)) => Literal::Str(s),
            Some(Tok::Num(n)) => Literal::Num(n),
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("true") || w.eq_ignore_ascii_case("yes") => Literal::Bool(true),
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("false") || w.eq_ignore_ascii_case("no") => Literal::Bool(false),
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("nil") || w.eq_ignore_ascii_case("null") => Literal::Nil,
            Some(Tok::Word(w)) => Literal::Str(w),
            Some(t) => return fail(format!("expected a value after `{attr} {op}`, found {t}")),
            None => return fail(format!("expected a value after `{attr} {op}`")),
        };
        Ok(Expr::Test(Test::new(attr, op, value)?))
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
/// smart-filter rule (`token.predicate.*`, `smartFilter.rule.custom`). M8.6 extends the
/// attributes and the evaluation; these three names are the interface `fontcraft-features`
/// compiles against.
pub type Predicate = Expr;

/// Reads predicate text ([`parse_predicate`]).
pub fn parse(text: &str) -> Result<Predicate, QueryError> {
    parse_predicate(text)
}

/// The names of the glyphs of `env`'s font that `predicate` selects, in font order. Layer
/// attributes read `env`'s master. An error names what cannot be evaluated.
pub fn evaluate(predicate: &Predicate, env: &Env) -> Result<Vec<String>, QueryError> {
    Ok(env.font.glyphs.iter().filter(|g| predicate.matches(&Subject::new(g), env)).map(|g| g.name.clone()).collect())
}

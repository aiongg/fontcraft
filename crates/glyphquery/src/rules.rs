//! Smart-filter rule trees (`docs/spec/05-font-view.md` § 6.3) and their compilation to glyph
//! predicates.
//!
//! JSON form, as `fontView.addSmartFilter` takes it and the user settings store it:
//! `{"all" | "any" | "none": [node…]}`, where a node is a rule `{"property", "op", "value"}` or
//! another group. Groups nest at most [`MAX_GROUP_DEPTH`] deep and hold at most [`MAX_RULES`]
//! nodes in all.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::predicate::{Attr, CASE_VALUES, Expr, Flag, LayerAttr, Literal, Op, Test, parse_predicate};
use crate::{MAX_GROUP_DEPTH, MAX_RULES, MAX_VALUE_BYTES, QueryError, fail, short};
use fontcraft_glyphdata::{Category, SubCategory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    /// Every rule must match.
    All,
    /// At least one rule must match.
    Any,
    /// No rule may match.
    None,
}

impl GroupKind {
    pub fn key(self) -> &'static str {
        match self {
            GroupKind::All => "all",
            GroupKind::Any => "any",
            GroupKind::None => "none",
        }
    }
}

/// A group of rules: the whole rule set of a smart filter, or a nested group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value", into = "serde_json::Value")]
pub struct RuleTree {
    pub kind: GroupKind,
    pub rules: Vec<RuleNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuleNode {
    Rule(Rule),
    Group(RuleTree),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub property: Property,
    pub op: RuleOp,
    pub value: RuleValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuleValue {
    Text(String),
    Number(f64),
    Bool(bool),
    /// Colour label rules: no label.
    NotSet,
}

/// What a rule inspects (the rule list of § 6.3 that M4 evaluates, and Custom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    GlyphName,
    CountOfPaths,
    CountOfComponents,
    Tags,
    Script,
    /// Category & Subcategory: `Letter` or `Letter/Uppercase`.
    Category,
    Case,
    ExportGlyph,
    HasUnicode,
    /// The rules from Has Components to Is Auto-Aligned read the layer of the shown master.
    HasComponents,
    /// Manual hints of either kind, PostScript hints, TrueType hints.
    HasHints,
    HasPostScriptHints,
    HasTrueTypeHints,
    HasCorners,
    HasAnchors,
    /// A component of the glyph named by the rule's value.
    ContainsComponent,
    HasCustomGlyphInfo,
    HasAnnotations,
    IsAutoAligned,
    HasMetricsKeys,
    /// Read from the set the caller gives `Env::with_out_of_sync`.
    MetricsKeysOutOfSync,
    HasKerningGroups,
    /// Read from the set the caller gives `Env::with_incompatible`.
    MasterCompatible,
    /// Intermediate, alternate, colour or smart-glyph layers, on any layer of the glyph.
    HasSpecialLayers,
    ColorLabel,
    LayerColorLabel,
    /// A glyph predicate as text.
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Text,
    Category,
    Case,
    Number,
    Bool,
    Color,
    /// A glyph name, compared whole.
    Glyph,
    Custom,
}

const TEXT_OPS: &[RuleOp] =
    &[RuleOp::Is, RuleOp::IsNot, RuleOp::Contains, RuleOp::DoesNotContain, RuleOp::BeginsWith, RuleOp::EndsWith, RuleOp::Matches];
const NUMBER_OPS: &[RuleOp] = &[RuleOp::Eq, RuleOp::Ne, RuleOp::Lt, RuleOp::Le, RuleOp::Gt, RuleOp::Ge];
const CHOICE_OPS: &[RuleOp] = &[RuleOp::Is, RuleOp::IsNot];

impl Kind {
    fn ops(self) -> &'static [RuleOp] {
        match self {
            Kind::Text => TEXT_OPS,
            Kind::Number => NUMBER_OPS,
            Kind::Category | Kind::Case | Kind::Bool | Kind::Color | Kind::Glyph | Kind::Custom => CHOICE_OPS,
        }
    }
}

impl Property {
    pub const ALL: &'static [Property] = &[
        Property::GlyphName,
        Property::CountOfPaths,
        Property::CountOfComponents,
        Property::Tags,
        Property::Script,
        Property::Category,
        Property::Case,
        Property::ExportGlyph,
        Property::HasUnicode,
        Property::HasComponents,
        Property::HasHints,
        Property::HasPostScriptHints,
        Property::HasTrueTypeHints,
        Property::HasCorners,
        Property::HasAnchors,
        Property::ContainsComponent,
        Property::HasCustomGlyphInfo,
        Property::HasAnnotations,
        Property::IsAutoAligned,
        Property::HasMetricsKeys,
        Property::MetricsKeysOutOfSync,
        Property::HasKerningGroups,
        Property::MasterCompatible,
        Property::HasSpecialLayers,
        Property::ColorLabel,
        Property::LayerColorLabel,
        Property::Custom,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Property::GlyphName => "glyphName",
            Property::CountOfPaths => "countOfPaths",
            Property::CountOfComponents => "countOfComponents",
            Property::Tags => "tags",
            Property::Script => "script",
            Property::Category => "category",
            Property::Case => "case",
            Property::ExportGlyph => "exportGlyph",
            Property::HasUnicode => "hasUnicode",
            Property::HasComponents => "hasComponents",
            Property::HasHints => "hasHints",
            Property::HasPostScriptHints => "hasPostScriptHints",
            Property::HasTrueTypeHints => "hasTrueTypeHints",
            Property::HasCorners => "hasCorners",
            Property::HasAnchors => "hasAnchors",
            Property::ContainsComponent => "containsComponent",
            Property::IsAutoAligned => "isAutoAligned",
            Property::HasCustomGlyphInfo => "hasCustomGlyphInfo",
            Property::HasAnnotations => "hasAnnotations",
            Property::HasMetricsKeys => "hasMetricsKeys",
            Property::MetricsKeysOutOfSync => "metricsKeysOutOfSync",
            Property::HasKerningGroups => "hasKerningGroups",
            Property::MasterCompatible => "masterCompatible",
            Property::HasSpecialLayers => "hasSpecialLayers",
            Property::ColorLabel => "colorLabel",
            Property::LayerColorLabel => "layerColorLabel",
            Property::Custom => "custom",
        }
    }

    pub fn parse(text: &str) -> Option<Property> {
        Property::ALL.iter().copied().find(|p| p.as_str() == text)
    }

    fn kind(self) -> Kind {
        match self {
            Property::GlyphName | Property::Tags | Property::Script => Kind::Text,
            Property::Category => Kind::Category,
            Property::Case => Kind::Case,
            Property::CountOfPaths | Property::CountOfComponents => Kind::Number,
            Property::ExportGlyph
            | Property::HasUnicode
            | Property::HasComponents
            | Property::HasHints
            | Property::HasPostScriptHints
            | Property::HasTrueTypeHints
            | Property::HasCorners
            | Property::HasAnchors
            | Property::HasCustomGlyphInfo
            | Property::HasAnnotations
            | Property::IsAutoAligned
            | Property::HasMetricsKeys
            | Property::MetricsKeysOutOfSync
            | Property::HasKerningGroups
            | Property::MasterCompatible
            | Property::HasSpecialLayers => Kind::Bool,
            Property::ColorLabel | Property::LayerColorLabel => Kind::Color,
            Property::ContainsComponent => Kind::Glyph,
            Property::Custom => Kind::Custom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleOp {
    Is,
    IsNot,
    Contains,
    DoesNotContain,
    BeginsWith,
    EndsWith,
    /// A regular expression that matches the whole value.
    Matches,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl RuleOp {
    const ALL: &'static [RuleOp] = &[
        RuleOp::Is,
        RuleOp::IsNot,
        RuleOp::Contains,
        RuleOp::DoesNotContain,
        RuleOp::BeginsWith,
        RuleOp::EndsWith,
        RuleOp::Matches,
        RuleOp::Eq,
        RuleOp::Ne,
        RuleOp::Lt,
        RuleOp::Le,
        RuleOp::Gt,
        RuleOp::Ge,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RuleOp::Is => "is",
            RuleOp::IsNot => "isNot",
            RuleOp::Contains => "contains",
            RuleOp::DoesNotContain => "doesNotContain",
            RuleOp::BeginsWith => "beginsWith",
            RuleOp::EndsWith => "endsWith",
            RuleOp::Matches => "matches",
            RuleOp::Eq => "==",
            RuleOp::Ne => "!=",
            RuleOp::Lt => "<",
            RuleOp::Le => "<=",
            RuleOp::Gt => ">",
            RuleOp::Ge => ">=",
        }
    }

    pub fn parse(text: &str) -> Option<RuleOp> {
        RuleOp::ALL.iter().copied().find(|o| o.as_str() == text)
    }
}

/// The colour labels by number, in the names `glyph.setColorLabel` accepts.
const COLOR_NAMES: &[&str] =
    &["red", "orange", "brown", "yellow", "lightgreen", "darkgreen", "cyan", "blue", "purple", "pink", "lightgray", "darkgray"];

/// Sidebar and rule labels for the case groups under a category, with the case each stands for.
pub(crate) const CASE_GROUPS: &[(&str, &str)] = &[("Uppercase", "upper"), ("Lowercase", "lower"), ("Smallcaps", "smallCaps"), ("Minor", "minor")];

fn color_index(name: &str) -> Option<usize> {
    let key: String = name.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase().replace("grey", "gray");
    COLOR_NAMES.iter().position(|n| *n == key)
}

// ── Reading JSON ────────────────────────────────────────────────────────────────────────────────

impl TryFrom<Value> for RuleTree {
    type Error = QueryError;

    fn try_from(v: Value) -> Result<RuleTree, QueryError> {
        RuleTree::from_json(&v)
    }
}

impl From<RuleTree> for Value {
    fn from(t: RuleTree) -> Value {
        t.to_json()
    }
}

impl RuleTree {
    /// Reads a rule tree and checks every value (regular expressions and Custom predicates
    /// included), so a tree that reads compiles.
    pub fn from_json(v: &Value) -> Result<RuleTree, QueryError> {
        let mut count = 0;
        let tree = group_from(v, 1, &mut count)?;
        tree.compile()?;
        Ok(tree)
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert(self.kind.key().to_string(), Value::Array(self.rules.iter().map(node_json).collect()));
        Value::Object(m)
    }

    /// What a new smart filter starts with: one rule that every glyph matches.
    pub fn starter() -> RuleTree {
        RuleTree {
            kind: GroupKind::All,
            rules: vec![RuleNode::Rule(Rule { property: Property::GlyphName, op: RuleOp::Contains, value: RuleValue::Text(String::new()) })],
        }
    }

    /// The glyph predicate this tree stands for. A None group is `NOT (a OR b …)`.
    pub fn compile(&self) -> Result<Expr, QueryError> {
        let parts = self
            .rules
            .iter()
            .map(|n| match n {
                RuleNode::Rule(r) => r.compile(),
                RuleNode::Group(g) => g.compile(),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match self.kind {
            GroupKind::All => Expr::All(parts),
            GroupKind::Any => Expr::Any(parts),
            GroupKind::None => Expr::Not(Box::new(Expr::Any(parts))),
        })
    }
}

const GROUP_SHAPE: &str = "a rule group is an object with one key, `all`, `any` or `none`, holding a list of rules";

fn group_from(v: &Value, depth: usize, count: &mut usize) -> Result<RuleTree, QueryError> {
    if depth > MAX_GROUP_DEPTH {
        return fail(format!("rule groups are nested more than {MAX_GROUP_DEPTH} levels deep"));
    }
    let Value::Object(m) = v else { return fail(GROUP_SHAPE) };
    let mut entries = m.iter();
    let (Some((key, list)), None) = (entries.next(), entries.next()) else { return fail(GROUP_SHAPE) };
    let kind = match key.as_str() {
        "all" => GroupKind::All,
        "any" => GroupKind::Any,
        "none" => GroupKind::None,
        _ => return fail(format!("{GROUP_SHAPE}, not `{}`", short(key))),
    };
    let Value::Array(items) = list else { return fail(GROUP_SHAPE) };
    let mut rules = Vec::new();
    for item in items {
        *count += 1;
        if *count > MAX_RULES {
            return fail(format!("a rule tree holds at most {MAX_RULES} rules and groups"));
        }
        let Value::Object(fields) = item else {
            return fail("each rule is an object {property, op, value} or a nested group");
        };
        if fields.contains_key("property") {
            rules.push(RuleNode::Rule(rule_from(fields)?));
        } else {
            rules.push(RuleNode::Group(group_from(item, depth + 1, count)?));
        }
    }
    Ok(RuleTree { kind, rules })
}

fn rule_from(m: &Map<String, Value>) -> Result<Rule, QueryError> {
    if let Some(other) = m.keys().find(|k| !matches!(k.as_str(), "property" | "op" | "value")) {
        return fail(format!("a rule has the keys property, op and value, not `{}`", short(other)));
    }
    let names = || Property::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ");
    let property = match m.get("property") {
        Some(Value::String(s)) => match Property::parse(s) {
            Some(p) => p,
            None => return fail(format!("unknown rule property `{}`; the properties are {}", short(s), names())),
        },
        _ => return fail(format!("a rule's `property` is one of {}", names())),
    };
    let kind = property.kind();
    let op = match m.get("op") {
        None | Some(Value::Null) => *kind.ops().first().unwrap_or(&RuleOp::Is),
        Some(Value::String(s)) => match RuleOp::parse(s) {
            Some(op) => op,
            None => return fail(format!("unknown operator `{}`", short(s))),
        },
        Some(_) => return fail("a rule's `op` is text"),
    };
    if !kind.ops().contains(&op) {
        let ops = kind.ops().iter().map(|o| o.as_str()).collect::<Vec<_>>().join(", ");
        return fail(format!("the rule `{}` takes the operators {ops}, not `{}`", property.as_str(), op.as_str()));
    }
    let value = value_from(property, kind, m.get("value"))?;
    Ok(Rule { property, op, value })
}

fn text_value(property: Property, v: Option<&Value>) -> Result<String, QueryError> {
    match v {
        Some(Value::String(s)) if s.len() <= MAX_VALUE_BYTES => Ok(s.clone()),
        Some(Value::String(s)) => {
            fail(format!("the value of rule `{}` is {} bytes long; at most {MAX_VALUE_BYTES} are allowed", property.as_str(), s.len()))
        }
        _ => fail(format!("the rule `{}` takes a text value", property.as_str())),
    }
}

fn value_from(property: Property, kind: Kind, v: Option<&Value>) -> Result<RuleValue, QueryError> {
    let v = v.filter(|v| !v.is_null());
    Ok(match kind {
        Kind::Text => RuleValue::Text(text_value(property, v)?),
        Kind::Custom => RuleValue::Text(text_value(property, v)?),
        Kind::Glyph => RuleValue::Text(text_value(property, v)?.trim().to_string()),
        Kind::Category => {
            let s = text_value(property, v)?;
            let (cat, sub) = match s.split_once('/') {
                Some((c, sub)) => (c, Some(sub)),
                None => (s.as_str(), None),
            };
            if Category::parse(cat).is_none() {
                let all = Category::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
                return fail(format!("`{}` is not a category; the categories are {all}", short(cat)));
            }
            if let Some(sub) = sub
                && SubCategory::parse(sub).is_none()
                && !CASE_GROUPS.iter().any(|(label, _)| *label == sub)
            {
                return fail(format!("`{}` is not a subcategory", short(sub)));
            }
            RuleValue::Text(s)
        }
        Kind::Case => {
            let s = text_value(property, v)?;
            if !CASE_VALUES.contains(&s.as_str()) {
                return fail(format!("the rule `case` takes one of {}", CASE_VALUES.join(", ")));
            }
            RuleValue::Text(s)
        }
        Kind::Number => match v.and_then(Value::as_f64) {
            Some(n) if n.is_finite() => RuleValue::Number(n),
            _ => return fail(format!("the rule `{}` takes a number", property.as_str())),
        },
        Kind::Bool => match v {
            None => RuleValue::Bool(true),
            Some(Value::Bool(b)) => RuleValue::Bool(*b),
            Some(_) => return fail(format!("the rule `{}` takes true or false", property.as_str())),
        },
        Kind::Color => match v {
            None => RuleValue::NotSet,
            Some(Value::String(s)) if s == "notSet" => RuleValue::NotSet,
            Some(Value::String(s)) => match color_index(s) {
                Some(i) => RuleValue::Number(i as f64),
                None => return fail(format!("`{}` is not a colour label; use 0 to 11, a colour name or notSet", short(s))),
            },
            Some(n) => match n.as_f64() {
                Some(i) if i.fract() == 0.0 && (0.0..=11.0).contains(&i) => RuleValue::Number(i),
                _ => return fail(format!("the rule `{}` takes a label number from 0 to 11, a colour name or notSet", property.as_str())),
            },
        },
    })
}

fn node_json(n: &RuleNode) -> Value {
    match n {
        RuleNode::Group(g) => g.to_json(),
        RuleNode::Rule(r) => {
            let value = match &r.value {
                RuleValue::Text(s) => json!(s),
                RuleValue::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => json!(*n as i64),
                RuleValue::Number(n) => json!(n),
                RuleValue::Bool(b) => json!(b),
                RuleValue::NotSet => Value::Null,
            };
            json!({"property": r.property.as_str(), "op": r.op.as_str(), "value": value})
        }
    }
}

// ── Compiling ───────────────────────────────────────────────────────────────────────────────────

fn test(attr: Attr, op: Op, value: Literal) -> Result<Expr, QueryError> {
    Ok(Expr::Test(Test::new(attr, op, value)?))
}

fn not(e: Expr) -> Expr {
    Expr::Not(Box::new(e))
}

/// A category entry's child: a case group (`Uppercase` is `case == upper`, or the subcategory
/// older files store) or a subcategory.
pub(crate) fn child_expr(label: &str) -> Result<Expr, QueryError> {
    let sub = test(Attr::SubCategory, Op::Eq, Literal::Str(label.to_string()))?;
    match CASE_GROUPS.iter().find(|(l, _)| *l == label) {
        Some((_, case)) => Ok(Expr::Any(vec![sub, test(Attr::Case, Op::Eq, Literal::Str((*case).to_string()))?])),
        None => Ok(sub),
    }
}

/// `Letter` or `Letter/Uppercase`.
pub(crate) fn category_expr(value: &str) -> Result<Expr, QueryError> {
    let (cat, sub) = match value.split_once('/') {
        Some((c, s)) => (c, Some(s)),
        None => (value, None),
    };
    let mut parts = vec![test(Attr::Category, Op::Eq, Literal::Str(cat.to_string()))?];
    if let Some(sub) = sub {
        parts.push(child_expr(sub)?);
    }
    Ok(Expr::All(parts))
}

impl Rule {
    pub fn compile(&self) -> Result<Expr, QueryError> {
        let p = self.property;
        let is = !matches!(self.op, RuleOp::IsNot | RuleOp::Ne);
        let polar = |e: Expr| if is { e } else { not(e) };
        match (p.kind(), &self.value) {
            (Kind::Text, RuleValue::Text(s)) => {
                let attr = match p {
                    Property::Tags => Attr::Tags,
                    Property::Script => Attr::Script,
                    _ => Attr::Name,
                };
                let lit = Literal::Str(s.clone());
                match self.op {
                    RuleOp::Is => test(attr, Op::Eq, lit),
                    RuleOp::IsNot => test(attr, Op::Ne, lit),
                    RuleOp::Contains => test(attr, Op::Contains, lit),
                    RuleOp::DoesNotContain => Ok(not(test(attr, Op::Contains, lit)?)),
                    RuleOp::BeginsWith => test(attr, Op::BeginsWith, lit),
                    RuleOp::EndsWith => test(attr, Op::EndsWith, lit),
                    RuleOp::Matches => test(attr, Op::Matches, lit),
                    _ => fail(format!("the rule `{}` does not take `{}`", p.as_str(), self.op.as_str())),
                }
            }
            (Kind::Category, RuleValue::Text(s)) => Ok(polar(category_expr(s)?)),
            (Kind::Case, RuleValue::Text(s)) => Ok(polar(test(Attr::Case, Op::Eq, Literal::Str(s.clone()))?)),
            (Kind::Number, RuleValue::Number(n)) => {
                let attr = if p == Property::CountOfPaths { Attr::CountOfPaths } else { Attr::CountOfComponents };
                let op = match self.op {
                    RuleOp::Eq => Op::Eq,
                    RuleOp::Ne => Op::Ne,
                    RuleOp::Lt => Op::Lt,
                    RuleOp::Le => Op::Le,
                    RuleOp::Gt => Op::Gt,
                    RuleOp::Ge => Op::Ge,
                    other => return fail(format!("the rule `{}` does not take `{}`", p.as_str(), other.as_str())),
                };
                test(attr, op, Literal::Num(*n))
            }
            (Kind::Bool, RuleValue::Bool(b)) => {
                let attr = match p {
                    Property::ExportGlyph => Attr::Export,
                    Property::HasUnicode => Attr::HasUnicode,
                    Property::HasCustomGlyphInfo => Attr::HasCustomGlyphInfo,
                    Property::HasMetricsKeys => Attr::HasMetricsKeys,
                    Property::MetricsKeysOutOfSync => Attr::MetricsKeysOutOfSync,
                    Property::HasKerningGroups => Attr::HasKerningGroups,
                    Property::MasterCompatible => Attr::MasterCompatible,
                    Property::HasSpecialLayers => Attr::Flag(Flag::HasSpecialLayers),
                    Property::HasComponents => Attr::Layer(LayerAttr::HasComponents),
                    Property::HasHints => Attr::Layer(LayerAttr::HasHints),
                    Property::HasPostScriptHints => Attr::Layer(LayerAttr::HasPostScriptHints),
                    Property::HasTrueTypeHints => Attr::Layer(LayerAttr::HasTrueTypeHints),
                    Property::HasCorners => Attr::Layer(LayerAttr::HasCorners),
                    Property::HasAnchors => Attr::Layer(LayerAttr::HasAnchors),
                    Property::IsAutoAligned => Attr::Layer(LayerAttr::IsAligned),
                    _ => Attr::Layer(LayerAttr::HasAnnotations),
                };
                Ok(polar(test(attr, Op::Eq, Literal::Bool(*b))?))
            }
            (Kind::Color, value) => {
                let attr = if p == Property::ColorLabel { Attr::ColorIndex } else { Attr::Layer(LayerAttr::ColorIndex) };
                let lit = match value {
                    RuleValue::Number(n) => Literal::Num(*n),
                    _ => Literal::Nil,
                };
                Ok(polar(test(attr, Op::Eq, lit)?))
            }
            (Kind::Glyph, RuleValue::Text(s)) => Ok(polar(test(Attr::Layer(LayerAttr::HasComponent(s.clone())), Op::Eq, Literal::Bool(true))?)),
            (Kind::Custom, RuleValue::Text(s)) => Ok(polar(parse_predicate(s)?)),
            _ => fail(format!("the rule `{}` has a value of the wrong kind", p.as_str())),
        }
    }
}

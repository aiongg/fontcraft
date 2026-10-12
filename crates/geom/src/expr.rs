//! Arithmetic expressions over named values (M13.16 fills the parser and evaluator). Users:
//! anchor position expressions and automatic layers (M13.16), construction rules (M13.17), the
//! Space Center's matrix entry (M13.23), the non-linear transform (M13.29).
//!
//! Grammar: numbers (`12`, `0.5`, `.5`, `1e3`), names (`width`, `xHeight`, `n.width`,
//! `anchor:top.x`: letters, digits, `_`, `.` and `:`, not starting with a digit, `.` or `:`),
//! unary `+` and `-`, binary `+ - * /`
//! with the usual precedence, parentheses, and calls of the fixed functions `min`, `max`, `abs`,
//! `round`, `floor`, `ceil`, `sqrt`, `sin`, `cos`, `tan` (radians), `atan2`. What a name means is
//! the caller's: [`Expr::eval`] asks a lookup for each one. White space between tokens is
//! ignored.
//!
//! Limits: source at most [`MAX_EXPR_BYTES`], nesting at most [`MAX_EXPR_DEPTH`], evaluation at
//! most [`MAX_EXPR_STEPS`] steps, and every intermediate and final value finite (division by zero
//! is an error, not infinity).

use std::fmt;

/// Longest expression source, in bytes.
pub const MAX_EXPR_BYTES: usize = 1_024;
/// Deepest nesting of parentheses, calls and unary operators.
pub const MAX_EXPR_DEPTH: usize = 64;
/// Most evaluation steps (one per operation, name or number).
pub const MAX_EXPR_STEPS: usize = 10_000;

/// Why an expression cannot be parsed or evaluated.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprError {
    /// The source is longer than [`MAX_EXPR_BYTES`].
    TooLong(usize),
    /// A syntax error at a byte offset.
    Syntax { at: usize, message: String },
    /// Nested deeper than [`MAX_EXPR_DEPTH`].
    TooDeep,
    /// Evaluation took more than [`MAX_EXPR_STEPS`] steps.
    TooManySteps,
    /// The lookup does not know a name.
    UnknownName(String),
    /// A function the grammar does not have, or a wrong argument count.
    BadCall(String),
    /// Division by zero or a result that is not finite.
    NotFinite,
    /// Kept for callers that match on it; the parser and evaluator never return it.
    Unsupported,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::TooLong(n) => write!(f, "the expression is {n} bytes; at most {MAX_EXPR_BYTES}"),
            ExprError::Syntax { at, message } => write!(f, "at character {at}: {message}"),
            ExprError::TooDeep => write!(f, "the expression is nested more than {MAX_EXPR_DEPTH} deep"),
            ExprError::TooManySteps => write!(f, "the expression takes more than {MAX_EXPR_STEPS} steps"),
            ExprError::UnknownName(name) => write!(f, "unknown name `{name}`"),
            ExprError::BadCall(why) => write!(f, "{why}"),
            ExprError::NotFinite => write!(f, "the result is not a finite number"),
            ExprError::Unsupported => write!(f, "expressions are not supported yet"),
        }
    }
}

impl std::error::Error for ExprError {}

/// A parsed expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    source: String,
    root: Node,
}

/// One node of the parsed tree. Its depth is bounded by the source length ([`MAX_EXPR_BYTES`]),
/// so the recursive walks below stay shallow.
#[derive(Clone, Debug, PartialEq)]
enum Node {
    Num(f64),
    Name(String),
    Neg(Box<Node>),
    Bin(Op, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Func {
    Min,
    Max,
    Abs,
    Round,
    Floor,
    Ceil,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Atan2,
}

impl Func {
    fn from_name(name: &str) -> Option<Func> {
        Some(match name {
            "min" => Func::Min,
            "max" => Func::Max,
            "abs" => Func::Abs,
            "round" => Func::Round,
            "floor" => Func::Floor,
            "ceil" => Func::Ceil,
            "sqrt" => Func::Sqrt,
            "sin" => Func::Sin,
            "cos" => Func::Cos,
            "tan" => Func::Tan,
            "atan2" => Func::Atan2,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Func::Min => "min",
            Func::Max => "max",
            Func::Abs => "abs",
            Func::Round => "round",
            Func::Floor => "floor",
            Func::Ceil => "ceil",
            Func::Sqrt => "sqrt",
            Func::Sin => "sin",
            Func::Cos => "cos",
            Func::Tan => "tan",
            Func::Atan2 => "atan2",
        }
    }

    /// The argument counts the function takes: at least, at most.
    fn arity(self) -> (usize, usize) {
        match self {
            Func::Min | Func::Max => (1, usize::MAX),
            Func::Atan2 => (2, 2),
            _ => (1, 1),
        }
    }
}

impl Expr {
    /// Parses `text`, checking length and nesting.
    pub fn parse(text: &str) -> Result<Expr, ExprError> {
        if text.len() > MAX_EXPR_BYTES {
            return Err(ExprError::TooLong(text.len()));
        }
        let mut p = Parser { text, pos: 0, depth: 0 };
        p.skip_space();
        if p.at_end() {
            return Err(ExprError::Syntax { at: 0, message: "the expression is empty".into() });
        }
        let root = p.expr()?;
        p.skip_space();
        if let Some(c) = p.peek() {
            return Err(p.syntax(format!("unexpected `{c}`")));
        }
        Ok(Expr { source: text.to_string(), root })
    }

    /// The source text as given.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every name the expression uses, each once, in order of first use.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        collect_names(&self.root, &mut out);
        out
    }

    /// The value, asking `lookup` for each name (`None`: [`ExprError::UnknownName`]).
    pub fn eval(&self, lookup: impl Fn(&str) -> Option<f64>) -> Result<f64, ExprError> {
        let mut steps = 0usize;
        eval_node(&self.root, &lookup, &mut steps)
    }
}

fn collect_names(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Num(_) => {}
        Node::Name(n) => {
            if !out.iter().any(|o| o == n) {
                out.push(n.clone());
            }
        }
        Node::Neg(a) => collect_names(a, out),
        Node::Bin(_, a, b) => {
            collect_names(a, out);
            collect_names(b, out);
        }
        Node::Call(_, args) => args.iter().for_each(|a| collect_names(a, out)),
    }
}

fn finite(v: f64) -> Result<f64, ExprError> {
    if v.is_finite() { Ok(v) } else { Err(ExprError::NotFinite) }
}

fn eval_node(node: &Node, lookup: &dyn Fn(&str) -> Option<f64>, steps: &mut usize) -> Result<f64, ExprError> {
    *steps += 1;
    if *steps > MAX_EXPR_STEPS {
        return Err(ExprError::TooManySteps);
    }
    match node {
        Node::Num(v) => finite(*v),
        Node::Name(n) => finite(lookup(n).ok_or_else(|| ExprError::UnknownName(n.clone()))?),
        Node::Neg(a) => finite(-eval_node(a, lookup, steps)?),
        Node::Bin(op, a, b) => {
            let (a, b) = (eval_node(a, lookup, steps)?, eval_node(b, lookup, steps)?);
            finite(match op {
                Op::Add => a + b,
                Op::Sub => a - b,
                Op::Mul => a * b,
                Op::Div if b == 0.0 => return Err(ExprError::NotFinite),
                Op::Div => a / b,
            })
        }
        Node::Call(f, args) => {
            let mut values = Vec::with_capacity(args.len());
            for a in args {
                values.push(eval_node(a, lookup, steps)?);
            }
            let first = values.first().copied().ok_or_else(|| ExprError::BadCall(format!("`{}` needs an argument", f.name())))?;
            finite(match f {
                Func::Min => values.iter().copied().fold(first, f64::min),
                Func::Max => values.iter().copied().fold(first, f64::max),
                Func::Abs => first.abs(),
                Func::Round => first.round(),
                Func::Floor => first.floor(),
                Func::Ceil => first.ceil(),
                Func::Sqrt if first < 0.0 => return Err(ExprError::NotFinite),
                Func::Sqrt => first.sqrt(),
                Func::Sin => first.sin(),
                Func::Cos => first.cos(),
                Func::Tan => first.tan(),
                Func::Atan2 => first.atan2(values.get(1).copied().ok_or_else(|| ExprError::BadCall("`atan2` takes two arguments".into()))?),
            })
        }
    }
}

/// A recursive-descent parser over the source bytes. `depth` counts the parentheses, calls and
/// unary operators open at the current position.
struct Parser<'a> {
    text: &'a str,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.text.get(self.pos..).and_then(|rest| rest.chars().next())
    }

    fn at_end(&self) -> bool {
        self.pos >= self.text.len()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn syntax(&self, message: impl Into<String>) -> ExprError {
        ExprError::Syntax { at: self.pos, message: message.into() }
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH { Err(ExprError::TooDeep) } else { Ok(()) }
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Result<Node, ExprError> {
        let mut left = self.term()?;
        loop {
            self.skip_space();
            let op = match self.peek() {
                Some('+') => Op::Add,
                Some('-') => Op::Sub,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.term()?;
            left = Node::Bin(op, Box::new(left), Box::new(right));
        }
    }

    /// term := unary (('*' | '/') unary)*
    fn term(&mut self) -> Result<Node, ExprError> {
        let mut left = self.unary()?;
        loop {
            self.skip_space();
            let op = match self.peek() {
                Some('*') => Op::Mul,
                Some('/') => Op::Div,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.unary()?;
            left = Node::Bin(op, Box::new(left), Box::new(right));
        }
    }

    /// unary := ('+' | '-') unary | primary
    fn unary(&mut self) -> Result<Node, ExprError> {
        self.skip_space();
        match self.peek() {
            Some(c @ ('+' | '-')) => {
                self.bump();
                self.enter()?;
                let inner = self.unary()?;
                self.leave();
                Ok(if c == '-' { Node::Neg(Box::new(inner)) } else { inner })
            }
            _ => self.primary(),
        }
    }

    /// primary := number | name | name '(' args ')' | '(' expr ')'
    fn primary(&mut self) -> Result<Node, ExprError> {
        self.skip_space();
        match self.peek() {
            None => Err(self.syntax("the expression ends where a value is expected")),
            Some('(') => {
                self.bump();
                self.enter()?;
                let inner = self.expr()?;
                self.skip_space();
                if self.bump() != Some(')') {
                    return Err(self.syntax("`)` expected"));
                }
                self.leave();
                Ok(inner)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            Some(c) if c.is_alphabetic() || c == '_' => self.name_or_call(),
            Some(c) => Err(self.syntax(format!("unexpected `{c}`"))),
        }
    }

    fn number(&mut self) -> Result<Node, ExprError> {
        let start = self.pos;
        let digits = |p: &mut Self| {
            let mut n = 0usize;
            while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                p.bump();
                n += 1;
            }
            n
        };
        let mut count = digits(self);
        if self.peek() == Some('.') {
            self.bump();
            count += digits(self);
        }
        if count == 0 {
            return Err(ExprError::Syntax { at: start, message: "a number needs a digit".into() });
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let mark = self.pos;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if digits(self) == 0 {
                // `2e` or `2ex`: the `e` starts no exponent, so a name follows a number.
                self.pos = mark;
                return Err(self.syntax("an operator is expected after a number"));
            }
        }
        let text = self.text.get(start..self.pos).unwrap_or_default();
        let value: f64 = text.parse().map_err(|_| ExprError::Syntax { at: start, message: format!("`{text}` is not a number") })?;
        if !value.is_finite() {
            return Err(ExprError::NotFinite);
        }
        if self.peek().is_some_and(|c| c.is_alphabetic() || c == '_') {
            return Err(self.syntax("an operator is expected after a number"));
        }
        Ok(Node::Num(value))
    }

    fn name_or_call(&mut self) -> Result<Node, ExprError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':')) {
            self.bump();
        }
        let name = self.text.get(start..self.pos).unwrap_or_default().to_string();
        self.skip_space();
        if self.peek() != Some('(') {
            return Ok(Node::Name(name));
        }
        let func = Func::from_name(&name).ok_or_else(|| ExprError::BadCall(format!("there is no function `{name}`")))?;
        self.bump();
        self.enter()?;
        let mut args = Vec::new();
        self.skip_space();
        if self.peek() == Some(')') {
            self.bump();
        } else {
            loop {
                args.push(self.expr()?);
                self.skip_space();
                match self.bump() {
                    Some(',') => continue,
                    Some(')') => break,
                    _ => return Err(self.syntax("`,` or `)` expected")),
                }
            }
        }
        self.leave();
        let (least, most) = func.arity();
        if args.len() < least || args.len() > most {
            let want = if least == most { format!("{least}") } else { format!("at least {least}") };
            let noun = if least == 1 && most == 1 { "argument" } else { "arguments" };
            return Err(ExprError::BadCall(format!("`{name}` takes {want} {noun}, not {}", args.len())));
        }
        Ok(Node::Call(func, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Result<f64, ExprError> {
        Expr::parse(text)?.eval(|n| match n {
            "width" => Some(600.0),
            "xHeight" => Some(500.0),
            "anchor:top.x" => Some(250.0),
            _ => None,
        })
    }

    #[test]
    fn arithmetic_follows_precedence_and_names_come_from_the_lookup() {
        assert_eq!(value("width/2"), Ok(300.0));
        assert_eq!(value("xHeight + 20"), Ok(520.0));
        assert_eq!(value("1 + 2 * 3 - 4 / 2"), Ok(5.0));
        assert_eq!(value("-(1 + 2) * -2"), Ok(6.0));
        assert_eq!(value("2 - 3 - 4"), Ok(-5.0), "subtraction associates to the left");
        assert_eq!(value("1e3 + .5"), Ok(1000.5));
        assert_eq!(value("max(1, width, 3) + min(2, -1) + abs(-4) + round(2.5) + floor(1.7) + ceil(1.2)"), Ok(600.0 - 1.0 + 4.0 + 3.0 + 1.0 + 2.0));
        assert_eq!(value("sqrt(16) + atan2(0, 1) + cos(0)"), Ok(5.0));
        assert_eq!(value("anchor:top.x - 10"), Ok(240.0));
        let e = Expr::parse("width/2 + xHeight - width").unwrap();
        assert_eq!(e.names(), ["width", "xHeight"]);
        assert_eq!(e.source(), "width/2 + xHeight - width");
    }

    #[test]
    fn errors_name_what_is_wrong() {
        assert_eq!(value("missing + 1"), Err(ExprError::UnknownName("missing".into())));
        assert_eq!(value("1/0"), Err(ExprError::NotFinite));
        assert_eq!(value("width / (xHeight - 500)"), Err(ExprError::NotFinite));
        assert_eq!(value("sqrt(-1)"), Err(ExprError::NotFinite));
        assert_eq!(value("1e308 * 10"), Err(ExprError::NotFinite));
        assert!(matches!(value("pow(2, 3)"), Err(ExprError::BadCall(_))));
        assert!(matches!(value("atan2(1)"), Err(ExprError::BadCall(_))));
        assert!(matches!(value("abs()"), Err(ExprError::BadCall(_))));
        for bad in ["", "   ", "1 +", "(1", "1)", "2x", "1 2", "a b", "*3", "1,2", "#"] {
            assert!(matches!(Expr::parse(bad), Err(ExprError::Syntax { .. })), "{bad:?} is a syntax error");
        }
    }

    #[test]
    fn hostile_expressions_are_refused_without_crashing() {
        let long = "1+".repeat(1 << 19) + "1";
        assert_eq!(Expr::parse(&long), Err(ExprError::TooLong(long.len())));
        let deep = format!("{}1{}", "(".repeat(MAX_EXPR_DEPTH + 1), ")".repeat(MAX_EXPR_DEPTH + 1));
        assert_eq!(Expr::parse(&deep), Err(ExprError::TooDeep));
        let ok = format!("{}1{}", "(".repeat(MAX_EXPR_DEPTH), ")".repeat(MAX_EXPR_DEPTH));
        assert_eq!(value(&ok), Ok(1.0));
        assert_eq!(Expr::parse(&"-".repeat(500)), Err(ExprError::TooDeep));
        let calls = format!("{}1{}", "abs(".repeat(200), ")".repeat(200));
        assert_eq!(Expr::parse(&calls), Err(ExprError::TooDeep));
        // The longest flat chain the size limit admits evaluates within the step limit.
        let chain = "1+".repeat(MAX_EXPR_BYTES / 2 - 1) + "1";
        assert_eq!(value(&chain), Ok((MAX_EXPR_BYTES / 2) as f64));
        assert!(Expr::parse("\u{0}").is_err());
        assert!(Expr::parse("é + 1").unwrap().eval(|_| Some(f64::NAN)).is_err(), "a lookup's NaN is not finite");
    }
}

//! Construction rules (M13.17): a text file that says how glyphs are built from other glyphs.
//! Rows: `font.buildFromConstructions`, `constructions.ruleGrammar`
//! (`spec/inventory/90-secondary.toml`).
//!
//! Ported from Glyph Construction (https://github.com/typemytype/GlyphConstruction),
//! `Lib/glyphConstruction.py` at commit 4ca9583e5ab34f90037fa118074661cb9b970ee4, MIT License,
//! Copyright (c) 2014 Frederik Berlaen. The rule grammar, the order in which a position name is
//! looked up, the point arithmetic of positions, the placement, kerning and flipping of parts and
//! the width and margin rules follow that file; the option comments follow its builder window
//! (`LibExtension/glyphConstructionController.py`). Credit is also in `NOTICE`.
//!
//! # The format
//!
//! One rule per line; blank lines and lines starting with `#` are skipped.
//!
//! | Form | Meaning |
//! |---|---|
//! | `Aacute = A + acute` | build `Aacute` from components: `+` adds a part over the one before it |
//! | `ffi = f & f & i` | `&` starts a new base after the advance of the bases before it |
//! | `A_V = A &\ V` | `\` in a base group applies the kerning between the two bases |
//! | `acute@center,top` | place a part: `@x,y`, `@xy` (both), or `@xx,xy,yx,yy,x,y` (a matrix and a position) |
//! | `acute@O:center,top` | measure from the part `O` of the group instead of the part before; `"O:latn":center` quotes a name |
//! | `acute@~center,~top` | `~` mirrors the part horizontally (on x) or vertically (on y), in place |
//! | `` acute@center,`top+10` `` | backticks hold a formula that uses `+` or `-` |
//! | `\| 00C1, 0162` | code points, hexadecimal |
//! | `! 1, 0, 0, 1` | colour label, red, green, blue and alpha from 0 to 1 |
//! | `^ 400` / `^ 30, 30` | width, or left and right margins: numbers or formulas over other glyphs (`A * 2`, `d'` for the opposite side, `"T-1"` quotes a name) |
//! | `# note` | the rest of the line is the glyph's note |
//! | `*`, `>`, `?` before the name | decompose the result; start from the glyph's own drawing; skip the rule when the glyph exists |
//! | `$name = value`, `{name}` | a variable: declared once (anywhere), used in braces; values may use other variables |
//! | `# OverwriteExistingGlyphs: False`, `# AutoUnicodes: True`, `# MarkGlyphs: 1, 0, 0, 1` | options for the whole file |
//!
//! A part whose name has a suffix the font lacks (`acute.cap`) takes the name before the first dot
//! when the font has that glyph.
//!
//! # Positions
//!
//! A position names a point on the part (the *mark*) and the same point on the glyph it is placed
//! against (the *base*: the part before it, or the one named with `:`); the part moves so the two
//! meet. Each name is looked up, in order, as: an anchor (`_top` on the mark, `top` on the base,
//! propagated anchors included), a guide of the glyph's layer, a guide of the master (on the base
//! it stands for `center` and `top`), `origin`/`width` (x) and `origin`/`height` (y), a font
//! metric (`descender`, `xHeight`, `capHeight`, `ascender`), a percentage of the bounds
//! (`50%`), and `center`, `top`, `bottom`, `left`, `right`, `innerTop`, `innerBottom`,
//! `innerLeft`, `innerRight` of the bounds (`top` is the base's top and the mark's bottom, and so
//! on). A plain number, metric, `origin`, `width` or `height` places the mark itself rather than
//! matching it to the base. Formulas combine names and numbers with `+ - * /`; on the mark side a
//! number only changes a fixed position, as in the original.
//!
//! The x position is a line at the italic angle and the y position a horizontal line (or a guide
//! at its angle); the point is where they cross.
//!
//! # Building
//!
//! [`parse_rules`] reads a file; [`build_rule`] works out one rule's master layers against a font
//! (each master on its own, with that master's metrics, anchors and kerning);
//! [`construct_glyph`] turns the result into a glyph. The engine command runs the rules in order,
//! adding each glyph before the next rule runs, so later rules can use earlier results.
//! Components are stored with automatic alignment off, so they stay where the rule put them.
//!
//! Limits: [`MAX_RULES_BYTES`], [`MAX_RULES`], [`MAX_RULE_PARTS`], [`MAX_VARIABLE_DEPTH`],
//! [`MAX_EXPANDED_BYTES`]; a position formula has the limits of [`fontcraft_geom::expr`].

use std::collections::HashMap;
use std::rc::Rc;

use fontcraft_geom::expr::{Expr, MAX_EXPR_BYTES, MAX_EXPR_DEPTH};
use fontcraft_geom::{Affine, Point, Rect, Shape as _};
use fontcraft_glyphdata::GlyphData;
use fontcraft_model::{
    Alignment, ColorLabel, Component, Font, Glyph, Guide, Layer, MAX_COORDINATE, MAX_NAME_LEN, Master, Metrics, Shape, Transform, metric,
};

use crate::auto_layer::reaches;
use crate::propagate::effective_anchors_with;

/// Largest rules text read, in bytes.
pub const MAX_RULES_BYTES: usize = 4 << 20;
/// Most rules in one file.
pub const MAX_RULES: usize = 65_536;
/// Most parts in one rule, over all its base groups.
pub const MAX_RULE_PARTS: usize = 64;
/// Deepest chain of variables that use other variables.
pub const MAX_VARIABLE_DEPTH: usize = 16;
/// Most text all rules may hold once variables are replaced, in bytes.
pub const MAX_EXPANDED_BYTES: usize = 16 << 20;

/// Why a rules file cannot be used at all. Problems with single rules are listed in
/// [`RuleFile::errors`] or returned by [`build_rule`] instead.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ConstructionError {
    #[error("the rules are {0} bytes; at most {MAX_RULES_BYTES} are read")]
    TooLarge(usize),
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    #[error("line {line}: {message}")]
    Variable { line: usize, message: String },
    #[error("`{glyph}`: {message}")]
    Build { glyph: String, message: String },
}

/// A line that could not be read as a rule; the other rules still run.
#[derive(Clone, Debug, PartialEq)]
pub struct LineError {
    /// 1-based.
    pub line: usize,
    pub text: String,
    pub message: String,
}

/// The options a file sets in comment lines; `None` where it says nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileOptions {
    /// `# OverwriteExistingGlyphs: True|False`.
    pub overwrite: Option<bool>,
    /// `# AutoUnicodes: True|False`.
    pub auto_unicodes: Option<bool>,
    /// `# MarkGlyphs: r, g, b, a` (`Some(Some(…))`) or `# MarkGlyphs: False` (`Some(None)`).
    pub mark_color: Option<Option<[f64; 4]>>,
}

/// A parsed rules file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleFile {
    pub rules: Vec<Rule>,
    pub errors: Vec<LineError>,
    pub options: FileOptions,
}

/// One rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// 1-based line in the file.
    pub line: usize,
    /// The line with its variables replaced.
    pub text: String,
    /// The glyph the rule builds.
    pub glyph: String,
    /// `?`: skip the rule when the glyph exists.
    pub skip_existing: bool,
    /// `*`: decompose the result.
    pub decompose: bool,
    /// `>`: start from the glyph's own drawing.
    pub add_source: bool,
    pub note: Option<String>,
    pub unicodes: Vec<u32>,
    pub color: Option<[f64; 4]>,
    pub metrics: Option<MetricsRule>,
    pub groups: Vec<Group>,
}

/// What follows `^`, as written (formulas are worked out against the font when building).
#[derive(Clone, Debug, PartialEq)]
pub enum MetricsRule {
    Width(String),
    Margins(String, String),
}

/// One base glyph and the parts placed over it (`a + grave`), between `&`.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// `\`: kern this group's base against the base before it.
    pub kern: bool,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub glyph: String,
    pub place: Option<Place>,
}

/// What follows `@`.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// `xx, xy, yx, yy` of a six-value position.
    pub matrix: Option<[f64; 4]>,
    pub x: Axis,
    pub y: Axis,
}

/// The position on one axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    /// The glyph measured against (`O:` or `"O:latn":`), instead of the part before.
    pub base: Option<String>,
    /// `~`: mirror the part on this axis.
    pub flip: bool,
    /// `None` when empty: the part is not moved.
    pub expr: Option<PosExpr>,
}

/// A position formula.
#[derive(Clone, Debug, PartialEq)]
pub struct PosExpr {
    source: String,
    root: PNode,
}

#[derive(Clone, Debug, PartialEq)]
enum PNode {
    Num(f64),
    Name(String),
    /// A percentage, kept with its source text (`50%`).
    Pct(String),
    Neg(Box<PNode>),
    Bin(char, Box<PNode>, Box<PNode>),
}

// ── Reading a file ──────────────────────────────────────────────────────────────────────────────

/// Reads a rules file. A line that is not a rule is listed in [`RuleFile::errors`]; the file as a
/// whole fails only when it is too large, has too many rules, or uses a variable that is missing,
/// circular or expands too far.
pub fn parse_rules(text: &str) -> Result<RuleFile, ConstructionError> {
    if text.len() > MAX_RULES_BYTES {
        return Err(ConstructionError::TooLarge(text.len()));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut options = FileOptions::default();
    let mut vars: HashMap<String, (usize, String)> = HashMap::new();
    let mut errors = Vec::new();
    let mut lines: Vec<(usize, &str)> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let n = i.saturating_add(1);
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            read_option(comment, &mut options);
            continue;
        }
        if let Some(decl) = line.strip_prefix('$') {
            match parse_declaration(decl) {
                Ok((name, value)) => {
                    vars.insert(name, (n, value));
                }
                Err(message) => errors.push(LineError { line: n, text: line.to_string(), message }),
            }
            continue;
        }
        lines.push((n, line));
    }
    if lines.len() > MAX_RULES {
        return Err(ConstructionError::LimitExceeded { what: "rules in one file", limit: MAX_RULES });
    }
    let mut expander = Expander { vars: &vars, done: HashMap::new(), total: 0 };
    let mut rules = Vec::with_capacity(lines.len());
    for (n, line) in lines {
        let line = expander.expand(line, n, 0)?;
        match parse_rule(&line, n) {
            Ok(rule) => rules.push(rule),
            Err(message) => errors.push(LineError { line: n, text: line, message }),
        }
    }
    Ok(RuleFile { rules, errors, options })
}

/// A comment line that sets an option (`# AutoUnicodes: False`); other comments change nothing.
fn read_option(comment: &str, options: &mut FileOptions) {
    let Some((key, value)) = comment.split_once(':') else { return };
    let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
    let flag = || match value.to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    match key.as_str() {
        "overwriteexistingglyphs" => options.overwrite = flag().or(options.overwrite),
        "autounicodes" => options.auto_unicodes = flag().or(options.auto_unicodes),
        "markglyphs" => {
            if flag() == Some(false) {
                options.mark_color = Some(None);
            } else if let Ok(c) = parse_rgba(value) {
                options.mark_color = Some(Some(c));
            }
        }
        _ => {}
    }
}

/// `name = value` after the `$`; a `#` ends the value.
fn parse_declaration(decl: &str) -> Result<(String, String), String> {
    let (name, value) = decl.split_once('=').ok_or_else(|| "a variable is declared as `$name = value`".to_string())?;
    let name = name.trim();
    if !is_identifier(name) {
        return Err(format!("`{name}` is not a variable name: use letters, digits and `_`, not starting with a digit"));
    }
    let value = value.split_once('#').map_or(value, |(v, _)| v).trim();
    Ok((name.to_string(), value.to_string()))
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Replaces `{name}` by variable values, which may use other variables.
struct Expander<'a> {
    vars: &'a HashMap<String, (usize, String)>,
    /// Values already expanded.
    done: HashMap<String, String>,
    /// Bytes produced so far, over every line and value.
    total: usize,
}

impl Expander<'_> {
    /// `text` with its variables replaced. Every piece is charged before it is appended, so a
    /// value used many times fails at the limit instead of filling memory.
    fn expand(&mut self, text: &str, line: usize, depth: usize) -> Result<String, ConstructionError> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let (before, after) = rest.split_at(open);
            self.push(&mut out, before)?;
            let after = after.get(1..).unwrap_or("");
            let name = after.find('}').and_then(|close| after.get(..close)).filter(|n| is_identifier(n));
            let Some(name) = name else {
                self.push(&mut out, "{")?;
                rest = after;
                continue;
            };
            let value = self.value(name, line, depth)?;
            self.push(&mut out, &value)?;
            rest = after.get(name.len().saturating_add(1)..).unwrap_or("");
        }
        self.push(&mut out, rest)?;
        Ok(out)
    }

    fn push(&mut self, out: &mut String, piece: &str) -> Result<(), ConstructionError> {
        self.charge(piece.len())?;
        out.push_str(piece);
        Ok(())
    }

    fn value(&mut self, name: &str, line: usize, depth: usize) -> Result<String, ConstructionError> {
        if let Some(v) = self.done.get(name) {
            return Ok(v.clone());
        }
        if depth >= MAX_VARIABLE_DEPTH {
            return Err(ConstructionError::Variable {
                line,
                message: format!("variables use other variables more than {MAX_VARIABLE_DEPTH} deep (or in a circle) at `{{{name}}}`"),
            });
        }
        let Some((declared, raw)) = self.vars.get(name) else {
            return Err(ConstructionError::Variable { line, message: format!("there is no variable `{name}`: declare it with `${name} = …`") });
        };
        let (declared, raw) = (*declared, raw.clone());
        let v = self.expand(&raw, declared, depth.saturating_add(1))?;
        self.done.insert(name.to_string(), v.clone());
        Ok(v)
    }

    fn charge(&mut self, n: usize) -> Result<(), ConstructionError> {
        self.total = self.total.saturating_add(n);
        if self.total > MAX_EXPANDED_BYTES {
            return Err(ConstructionError::LimitExceeded { what: "bytes of rules once variables are replaced", limit: MAX_EXPANDED_BYTES });
        }
        Ok(())
    }
}

/// Stand-ins for `+` and `-` inside backticks while the rule is split at `+`.
const ESC_ADD: char = '\u{E000}';
const ESC_SUB: char = '\u{E001}';

fn unescape(s: &str) -> String {
    s.replace(ESC_ADD, "+").replace(ESC_SUB, "-")
}

/// One rule line (variables already replaced).
fn parse_rule(line: &str, n: usize) -> Result<Rule, String> {
    let (mut skip_existing, mut decompose, mut add_source) = (false, false, false);
    let mut rest = line.trim_start();
    loop {
        match rest.chars().next() {
            Some('?') => skip_existing = true,
            Some('*') => decompose = true,
            Some('>') => add_source = true,
            _ => break,
        }
        rest = rest.get(1..).unwrap_or("").trim_start();
    }
    let (body, note) = match rest.split_once('#') {
        Some((body, note)) => (body, Some(note.trim().to_string()).filter(|s| !s.is_empty())),
        None => (rest, None),
    };
    if !body.contains('=') {
        return Err("a rule is `name = parts`: there is no `=`".into());
    }
    let compact: String = body.chars().filter(|c| *c != ' ' && *c != '\t').collect();
    let compact = escape_formulas(&compact)?;
    let (name, body) = compact.split_once('=').ok_or_else(|| "a rule is `name = parts`: there is no `=`".to_string())?;
    if body.contains('=') {
        return Err("a rule has one `=`".into());
    }
    let glyph = check_glyph_name(name)?;

    // `^`, `!` and `|` each take the text up to the next of them; a later one replaces an earlier.
    let mut construction = String::new();
    let mut attrs: Vec<(char, String)> = Vec::new();
    let mut key: Option<char> = None;
    for c in body.chars() {
        if matches!(c, '^' | '!' | '|') {
            key = Some(c);
            attrs.retain(|(k, _)| *k != c);
            attrs.push((c, String::new()));
            continue;
        }
        match (key, attrs.last_mut()) {
            (Some(_), Some((_, v))) => v.push(c),
            _ => construction.push(c),
        }
    }
    let mut rule = Rule {
        line: n,
        text: line.to_string(),
        glyph,
        skip_existing,
        decompose,
        add_source,
        note,
        unicodes: Vec::new(),
        color: None,
        metrics: None,
        groups: Vec::new(),
    };
    for (k, v) in attrs {
        match k {
            '^' => rule.metrics = Some(parse_metrics(&unescape(&v))?),
            '!' => rule.color = Some(parse_rgba(&v).map_err(|e| format!("`!` {e}"))?),
            _ => rule.unicodes = parse_unicodes(&v)?,
        }
    }
    if !construction.is_empty() {
        let mut count = 0usize;
        for group in construction.split('&') {
            let kern = group.contains('\\');
            let group = group.replace('\\', "");
            let mut parts = Vec::new();
            for part in group.split('+') {
                count = count.saturating_add(1);
                if count > MAX_RULE_PARTS {
                    return Err(format!("a rule has more than {MAX_RULE_PARTS} parts"));
                }
                parts.push(parse_part(&unescape(part))?);
            }
            rule.groups.push(Group { kern, parts });
        }
    }
    Ok(rule)
}

/// Turns each backtick formula into its text with `+` and `-` escaped, so splitting the rule at
/// `+` leaves it whole.
fn escape_formulas(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        match c {
            '`' => inside = !inside,
            '+' if inside => out.push(ESC_ADD),
            '-' if inside => out.push(ESC_SUB),
            _ => out.push(c),
        }
    }
    if inside {
        return Err("a backtick formula is not closed".into());
    }
    Ok(out)
}

fn check_glyph_name(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("a glyph name is missing".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("a glyph name is longer than {MAX_NAME_LEN} bytes"));
    }
    if name.chars().any(char::is_control) {
        return Err(format!("`{}` holds a control character", name.escape_debug()));
    }
    Ok(name.to_string())
}

fn parse_metrics(v: &str) -> Result<MetricsRule, String> {
    if v.is_empty() {
        return Err("`^` takes a width, or a left and a right margin".into());
    }
    let pieces: Vec<&str> = v.split(',').collect();
    match pieces.as_slice() {
        [w] => Ok(MetricsRule::Width((*w).to_string())),
        [l, r] if !l.is_empty() && !r.is_empty() => Ok(MetricsRule::Margins((*l).to_string(), (*r).to_string())),
        _ => Err("`^` takes a width, or a left and a right margin".into()),
    }
}

fn parse_rgba(v: &str) -> Result<[f64; 4], String> {
    let values: Vec<f64> = v.split(',').map(|s| s.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|_| "takes four numbers".to_string())?;
    match values.as_slice() {
        [r, g, b, a] if values.iter().all(|c| (0.0..=1.0).contains(c)) => Ok([*r, *g, *b, *a]),
        _ => Err("takes four numbers from 0 to 1: red, green, blue and alpha".into()),
    }
}

fn parse_unicodes(v: &str) -> Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for item in v.split(',') {
        let hex = item.trim();
        let hex = hex.strip_prefix("U+").or_else(|| hex.strip_prefix("u+")).unwrap_or(hex);
        let value = u32::from_str_radix(hex, 16).ok().filter(|u| char::from_u32(*u).is_some());
        match value {
            Some(u) if !out.contains(&u) => out.push(u),
            Some(_) => {}
            None => return Err(format!("`{}` is not a code point: write it in hexadecimal (00C1)", item.trim())),
        }
    }
    Ok(out)
}

fn parse_part(s: &str) -> Result<Part, String> {
    let mut pieces = s.split('@');
    let glyph = pieces.next().unwrap_or("");
    let position = pieces.next();
    if pieces.next().is_some() {
        return Err(format!("`{s}` has more than one `@`"));
    }
    let glyph = check_glyph_name(glyph).map_err(|e| format!("a part: {e}"))?;
    let place = position.map(parse_place).transpose()?;
    Ok(Part { glyph, place })
}

fn parse_place(s: &str) -> Result<Place, String> {
    let pieces: Vec<&str> = s.split(',').collect();
    let (matrix, x, y) = match pieces.as_slice() {
        [xx, xy, yx, yy, x, y] => {
            let num = |t: &str| t.parse::<f64>().ok().filter(|v| v.is_finite() && v.abs() <= MAX_COORDINATE);
            let m = [num(xx), num(xy), num(yx), num(yy)];
            let [Some(a), Some(b), Some(c), Some(d)] = m else {
                return Err(format!("`@{s}`: the first four of six values are numbers (xx, xy, yx, yy)"));
            };
            (Some([a, b, c, d]), *x, *y)
        }
        [x, y] => (None, *x, *y),
        [xy] => (None, *xy, *xy),
        _ => return Err(format!("`@{s}` takes one position, two (x, y) or six values (a matrix and x, y)")),
    };
    Ok(Place { matrix, x: parse_axis(x)?, y: parse_axis(y)? })
}

fn parse_axis(s: &str) -> Result<Axis, String> {
    let mut text = s.to_string();
    let mut base = None;
    if let Some(start) = text.find('"') {
        let inner = text.get(start.saturating_add(1)..).unwrap_or("");
        let Some(len) = inner.find('"') else { return Err(format!("`{s}`: a quoted glyph name is not closed")) };
        let name = inner.get(..len).unwrap_or("").to_string();
        let quoted = format!("\"{name}\":");
        if !text.contains(&quoted) {
            return Err(format!("`{s}`: a quoted glyph name is followed by `:` and the position"));
        }
        text = text.replacen(&quoted, "", 1);
        base = Some(check_glyph_name(&name).map_err(|e| format!("`{s}`: {e}"))?);
    } else if let Some((b, e)) = text.split_once(':') {
        if e.contains(':') {
            return Err(format!("`{s}` has more than one `:`; quote a glyph name that holds one"));
        }
        base = Some(check_glyph_name(b).map_err(|e| format!("`{s}`: {e}"))?);
        text = e.to_string();
    }
    let flip = text.contains('~');
    let text = text.replace('~', "");
    let expr = if text.is_empty() { None } else { Some(PosExpr::parse(&text)?) };
    Ok(Axis { base, flip, expr })
}

// ── Position formulas ───────────────────────────────────────────────────────────────────────────

impl PosExpr {
    /// Parses a position formula: numbers, percentages (`50%`), names, `+ - * /` and parentheses.
    pub fn parse(text: &str) -> Result<PosExpr, String> {
        if text.len() > MAX_EXPR_BYTES {
            return Err(format!("a position is {} bytes; at most {MAX_EXPR_BYTES}", text.len()));
        }
        let tokens = tokenize(text)?;
        let mut p = PParser { tokens: &tokens, at: 0, depth: 0, source: text };
        let root = p.expr()?;
        if p.at < tokens.len() {
            return Err(format!("`{text}` is not a position: something follows the end of the formula"));
        }
        Ok(PosExpr { source: text.to_string(), root })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// The names and then the percentages, in the order they are written (the order they are
    /// looked up in).
    fn atoms(&self) -> Vec<&PNode> {
        let mut names = Vec::new();
        let mut pcts = Vec::new();
        let mut stack = vec![&self.root];
        while let Some(n) = stack.pop() {
            match n {
                PNode::Name(_) => names.push(n),
                PNode::Pct(_) => pcts.push(n),
                PNode::Neg(a) => stack.push(a),
                PNode::Bin(_, a, b) => {
                    stack.push(b);
                    stack.push(a);
                }
                PNode::Num(_) => {}
            }
        }
        names.extend(pcts);
        names
    }

    fn eval(&self, env: &HashMap<String, Val>) -> Result<Val, String> {
        eval_node(&self.root, env, 0)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Pct(String),
    Name(String),
    Op(char),
    Open,
    Close,
}

fn tokenize(text: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '0'..='9' | '.' => {
                let mut end = i.saturating_add(1);
                while let Some(&(j, d)) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        end = j.saturating_add(1);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let digits = text.get(i..end).unwrap_or("");
                let value = digits.parse::<f64>().map_err(|_| format!("`{digits}` is not a number"))?;
                if chars.peek().is_some_and(|(_, d)| *d == '%') {
                    chars.next();
                    out.push(Tok::Pct(format!("{digits}%")));
                } else {
                    out.push(Tok::Num(value));
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut end = i.saturating_add(1);
                while let Some(&(j, d)) = chars.peek() {
                    if d.is_ascii_alphanumeric() || d == '_' || d == '.' {
                        end = j.saturating_add(d.len_utf8());
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Name(text.get(i..end).unwrap_or("").to_string()));
            }
            '+' | '-' | '*' | '/' => out.push(Tok::Op(c)),
            '(' => out.push(Tok::Open),
            ')' => out.push(Tok::Close),
            other => return Err(format!("`{other}` cannot stand in a position (`{text}`)")),
        }
    }
    if out.is_empty() {
        return Err("a position is empty".into());
    }
    Ok(out)
}

struct PParser<'a> {
    tokens: &'a [Tok],
    at: usize,
    depth: usize,
    source: &'a str,
}

impl PParser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<&Tok> {
        let t = self.tokens.get(self.at);
        self.at = self.at.saturating_add(1);
        t
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_EXPR_DEPTH {
            return Err(format!("a position is nested more than {MAX_EXPR_DEPTH} deep"));
        }
        Ok(())
    }

    fn expr(&mut self) -> Result<PNode, String> {
        self.enter()?;
        let mut left = self.term()?;
        while let Some(Tok::Op(op @ ('+' | '-'))) = self.peek() {
            let op = *op;
            self.at = self.at.saturating_add(1);
            let right = self.term()?;
            left = PNode::Bin(op, Box::new(left), Box::new(right));
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(left)
    }

    fn term(&mut self) -> Result<PNode, String> {
        let mut left = self.unary()?;
        while let Some(Tok::Op(op @ ('*' | '/'))) = self.peek() {
            let op = *op;
            self.at = self.at.saturating_add(1);
            let right = self.unary()?;
            left = PNode::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<PNode, String> {
        let negate = match self.peek() {
            Some(Tok::Op('-')) => true,
            Some(Tok::Op('+')) => false,
            _ => return self.atom(),
        };
        self.at = self.at.saturating_add(1);
        self.enter()?;
        let inner = self.unary()?;
        self.depth = self.depth.saturating_sub(1);
        Ok(if negate { PNode::Neg(Box::new(inner)) } else { inner })
    }

    fn atom(&mut self) -> Result<PNode, String> {
        let source = self.source;
        match self.next().cloned() {
            Some(Tok::Num(v)) => Ok(PNode::Num(v)),
            Some(Tok::Pct(p)) => Ok(PNode::Pct(p)),
            Some(Tok::Name(n)) => Ok(PNode::Name(n)),
            Some(Tok::Open) => {
                let inner = self.expr()?;
                match self.next() {
                    Some(Tok::Close) => Ok(inner),
                    _ => Err(format!("`{source}`: a `(` is not closed")),
                }
            }
            _ => Err(format!("`{source}` is not a position: a number or name is missing")),
        }
    }
}

/// A value while a position is worked out: a number, or a point. A point on the mark side whose
/// position is not fixed ignores numbers (`tuple_only`), so `top+10` moves only the base's point.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Val {
    Num(f64),
    Pt { x: f64, y: f64, tuple_only: bool },
}

fn eval_node(n: &PNode, env: &HashMap<String, Val>, depth: usize) -> Result<Val, String> {
    // A chain `1+1+…` nests one node per operator, so the tree is at most as deep as the source
    // is long; the parser has already bounded that.
    if depth > MAX_EXPR_BYTES {
        return Err(format!("a position is nested more than {MAX_EXPR_DEPTH} deep"));
    }
    let d = depth.saturating_add(1);
    match n {
        PNode::Num(v) => Ok(Val::Num(*v)),
        PNode::Name(name) | PNode::Pct(name) => env.get(name).copied().ok_or_else(|| format!("`{name}` has no value")),
        PNode::Neg(a) => match eval_node(a, env, d)? {
            Val::Num(v) => Ok(Val::Num(-v)),
            Val::Pt { .. } => Err("a position name cannot be negated; subtract it from a position instead".into()),
        },
        PNode::Bin(op, a, b) => apply(*op, eval_node(a, env, d)?, eval_node(b, env, d)?),
    }
}

fn arith(op: char, a: f64, b: f64) -> Result<f64, String> {
    let v = match op {
        '+' => a + b,
        '-' => a - b,
        '*' => a * b,
        _ if b == 0.0 => return Err("a position divides by zero".into()),
        _ => a / b,
    };
    if v.is_finite() { Ok(v) } else { Err("a position is not a finite number".into()) }
}

fn apply(op: char, a: Val, b: Val) -> Result<Val, String> {
    // A point changes only where the other value is not zero, as the original's point type does.
    let point = |x: f64, y: f64, tuple_only: bool, ox: f64, oy: f64| -> Result<Val, String> {
        let x = if ox != 0.0 { arith(op, x, ox)? } else { x };
        let y = if oy != 0.0 { arith(op, y, oy)? } else { y };
        Ok(Val::Pt { x, y, tuple_only })
    };
    match (a, b) {
        (Val::Num(a), Val::Num(b)) => arith(op, a, b).map(Val::Num),
        (Val::Pt { x, y, tuple_only }, Val::Pt { x: ox, y: oy, .. }) => point(x, y, tuple_only, ox, oy),
        (Val::Pt { x, y, tuple_only: true }, Val::Num(_)) => Ok(Val::Pt { x, y, tuple_only: true }),
        (Val::Pt { x, y, tuple_only: false }, Val::Num(n)) => point(x, y, false, n, n),
        (Val::Num(n), p @ Val::Pt { .. }) if op == '+' || op == '*' => apply(op, p, Val::Num(n)),
        (Val::Num(_), Val::Pt { .. }) => Err(format!("a number cannot come before a position name in `{op}`")),
    }
}

// ── Building ────────────────────────────────────────────────────────────────────────────────────

/// One master layer a rule builds.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltLayer {
    pub master: String,
    pub width: f64,
    pub shapes: Vec<Shape>,
}

/// What a rule builds.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    pub glyph: String,
    pub layers: Vec<BuiltLayer>,
    /// Component glyphs the font does not have. They stay components, as the rule names them.
    pub missing: Vec<String>,
    pub warnings: Vec<String>,
}

/// The kerning between two glyphs in a master: `(master, first, second)`.
pub type KernFn<'a> = &'a dyn Fn(&str, &str, &str) -> f64;

/// Works out `rule` for each master of `masters` against `font` (the glyphs built by earlier
/// rules included). A rule whose parts contain the glyph it builds, or that places a part at a
/// position that cannot be worked out, fails; nothing is changed either way.
pub fn build_rule(font: &Font, data: Option<&GlyphData>, kern: KernFn<'_>, rule: &Rule, masters: &[String]) -> Result<Built, ConstructionError> {
    let fail = |message: String| ConstructionError::Build { glyph: rule.glyph.clone(), message };
    let mut warnings = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for part in rule.groups.iter().flat_map(|g| g.parts.iter()) {
        let name = part_name(font, &part.glyph);
        if name == rule.glyph || (font.glyph(&name).is_some() && reaches(font, &name, &rule.glyph)) {
            return Err(fail(format!("its part `{name}` contains `{}`, so the glyph would contain itself", rule.glyph)));
        }
        if font.glyph(&name).is_none() && !missing.contains(&name) {
            missing.push(name);
        }
    }
    let mut layers = Vec::with_capacity(masters.len());
    for id in masters {
        let master = font.master(id).map_err(|e| fail(e.to_string()))?;
        let mut cx = MasterCx::new(font, data, master);
        let layer = cx.build(rule, kern, &mut warnings).map_err(fail)?;
        layers.push(layer);
    }
    warnings.dedup();
    Ok(Built { glyph: rule.glyph.clone(), layers, missing, warnings })
}

/// The name a part stands for: as written, or without its suffix when the font lacks the
/// suffixed glyph and has the plain one (`acute.cap` → `acute`).
fn part_name(font: &Font, name: &str) -> String {
    if font.glyph(name).is_none()
        && let Some((plain, _)) = name.split_once('.')
        && !plain.is_empty()
        && font.glyph(plain).is_some()
    {
        return plain.to_string();
    }
    name.to_string()
}

/// How [`construct_glyph`] fills in what a rule leaves open.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ApplyOptions {
    /// Give a glyph without code points in its rule the glyph database's code point for its name.
    pub auto_unicodes: bool,
    /// The colour label of glyphs whose rule sets none.
    pub mark_color: Option<[f64; 4]>,
}

/// The glyph `built` describes: the existing glyph with the built master layers replaced (its
/// other layers and data kept), or a new glyph. Code points, note and colour come from the rule,
/// then from `options`.
pub fn construct_glyph(font: &Font, data: Option<&GlyphData>, rule: &Rule, built: &Built, options: &ApplyOptions) -> Glyph {
    let mut g = font.glyph(&built.glyph).map_or_else(|| Glyph::new(&built.glyph), |e| (**e).clone());
    for b in &built.layers {
        let mut layer = Layer::new(&b.master, b.width);
        layer.shapes = b.shapes.clone();
        match g.layers.iter_mut().find(|l| l.id == b.master && l.master == b.master) {
            Some(l) => *l = layer,
            None => g.layers.push(layer),
        }
    }
    if !rule.unicodes.is_empty() {
        g.unicodes = rule.unicodes.clone();
    } else if options.auto_unicodes
        && let Some(data) = data
        && data.split_suffix(&built.glyph).1.is_none()
        && let Some(cp) = data.lookup(&built.glyph, &[]).unicode.map(|u| u.value)
        && !font.glyphs.as_arcs().iter().any(|o| o.name != built.glyph && o.unicodes.contains(&cp))
    {
        g.unicodes = vec![cp];
    }
    if let Some(note) = &rule.note {
        g.info.note = note.clone();
    }
    if let Some(c) = rule.color.or(options.mark_color) {
        g.info.color = Some(ColorLabel::Rgba(c));
    }
    g
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dir {
    X,
    Y,
}

/// What a position needs to know about one glyph in one master.
struct Facts {
    bounds: Option<Rect>,
    width: f64,
    height: f64,
    anchors: Vec<(String, f64, f64)>,
    guides: Vec<Guide>,
}

/// One master's view of the font while a rule is built.
struct MasterCx<'a> {
    font: &'a Font,
    data: Option<&'a GlyphData>,
    master: &'a Master,
    italic: f64,
    metrics: [(&'static str, f64); 4],
    cache: HashMap<String, Option<Rc<Facts>>>,
}

/// (point, angle in degrees, fixed): a point and the direction of the line through it; a fixed
/// position places the mark itself instead of matching it to the base.
type AxisPos = (Point, f64, bool);

impl<'a> MasterCx<'a> {
    fn new(font: &'a Font, data: Option<&'a GlyphData>, master: &'a Master) -> Self {
        let d = Metrics::defaults(font.source.format, font.units_per_em);
        let m = |kind: &str, default: f64| font.metric_pos(master, kind, default);
        MasterCx {
            font,
            data,
            master,
            italic: m(metric::ITALIC_ANGLE, 0.0),
            metrics: [
                ("descender", m(metric::DESCENDER, d.descender)),
                ("xHeight", m(metric::X_HEIGHT, d.x_height)),
                ("capHeight", m(metric::CAP_HEIGHT, d.cap_height)),
                ("ascender", m(metric::ASCENDER, d.ascender)),
            ],
            cache: HashMap::new(),
        }
    }

    fn facts(&mut self, name: &str) -> Option<Rc<Facts>> {
        if let Some(f) = self.cache.get(name) {
            return f.clone();
        }
        let f = self.font.glyph(name).map(|g| {
            let id = self.master.id.as_str();
            let layer = g.layer(id);
            let bounds = self.font.outline_of(g, id).ok().filter(|p| !p.elements().is_empty()).map(|p| p.bounding_box());
            let width = layer.map_or(0.0, |l| l.width);
            let height = layer.and_then(|l| l.info.vert_width).unwrap_or_else(|| bounds.map_or(0.0, |b| b.height()));
            let anchors = effective_anchors_with(self.font, self.data, name, id)
                .map(|list| list.into_iter().map(|a| (a.name, a.x, a.y)).collect())
                .unwrap_or_default();
            let guides = layer.map(|l| l.info.guides.clone()).unwrap_or_default();
            Rc::new(Facts { bounds, width, height, anchors, guides })
        });
        self.cache.insert(name.to_string(), f.clone());
        f
    }

    fn build(&mut self, rule: &Rule, kern: KernFn<'_>, warnings: &mut Vec<String>) -> Result<BuiltLayer, String> {
        let font = self.font;
        let master = self.master.id.clone();
        let mut shapes: Vec<Shape> = Vec::new();
        if rule.add_source
            && let Some(layer) = font.glyph(&rule.glyph).and_then(|g| g.layer(&master))
        {
            shapes.extend(layer.shapes.iter().cloned());
        }
        let mut advance = 0.0_f64;
        let mut previous_base: Option<String> = None;
        for group in &rule.groups {
            let mut base_glyph: Option<String> = None;
            let mut base_transform = Affine::IDENTITY;
            let mut last: Option<String> = None;
            let mut placed: HashMap<String, Affine> = HashMap::new();
            for part in &group.parts {
                let name = part_name(font, &part.glyph);
                let (mut t, mut unflipped) = self.place(last.as_deref(), &name, part.place.as_ref(), &placed, advance, warnings)?;
                if base_glyph.is_none() {
                    if group.kern
                        && let Some(prev) = &previous_base
                    {
                        let k = kern(&master, prev, &name);
                        if k != 0.0 && k.is_finite() {
                            t = t * Affine::translate((k, 0.0));
                            // Marks over this base follow it to its kerned place.
                            unflipped = unflipped * Affine::translate((k, 0.0));
                            advance += k;
                        }
                    }
                    base_transform = t;
                    base_glyph = Some(name.clone());
                }
                placed.insert(name.clone(), unflipped);
                last = Some(name.clone());
                shapes.push(Shape::Component(component(&name, t)?));
            }
            if let Some(base) = &base_glyph
                && let Some(f) = self.facts(base)
            {
                let (p0, p1) = (base_transform * Point::ZERO, base_transform * Point::new(f.width, 0.0));
                advance += (p1.x - p0.x).abs();
            }
            previous_base = base_glyph;
        }
        let mut width = advance;
        match &rule.metrics {
            None => {}
            Some(MetricsRule::Width(v)) => match self.metric_value(v, Metric::Width)? {
                Some(w) => width = w,
                None => warnings.push(format!("the width `{v}` has no value here (a glyph without an outline); the width stays {width}")),
            },
            Some(MetricsRule::Margins(l, r)) => {
                match (self.metric_value(l, Metric::Left)?, self.shapes_bounds(&shapes)) {
                    (Some(value), Some(b)) => {
                        let shift = value - b.x0;
                        shapes.iter_mut().for_each(|s| shift_shape(s, shift));
                        width += shift;
                    }
                    (None, _) => warnings.push(format!("the left margin `{l}` has no value here; it is left as it is")),
                    (Some(_), None) => {}
                }
                match (self.metric_value(r, Metric::Right)?, self.shapes_bounds(&shapes)) {
                    (Some(value), Some(b)) => width = b.x1 + value,
                    (None, _) => warnings.push(format!("the right margin `{r}` has no value here; it is left as it is")),
                    (Some(_), None) => {}
                }
            }
        }
        if !width.is_finite() || width.abs() > MAX_COORDINATE {
            return Err(format!("the width {width} is not a usable number"));
        }
        Ok(BuiltLayer { master, width: clean(width), shapes })
    }

    /// Where a part goes: its transform, and the transform before any mirroring (what later parts
    /// measure from).
    fn place(
        &mut self,
        base: Option<&str>,
        mark: &str,
        place: Option<&Place>,
        placed: &HashMap<String, Affine>,
        advance: f64,
        warnings: &mut Vec<String>,
    ) -> Result<(Affine, Affine), String> {
        let [mut a, mut b, mut c, mut d] = [1.0, 0.0, 0.0, 1.0];
        let (mut x, mut y) = (advance, 0.0);
        let Some(place) = place else {
            let t = Affine::new([a, b, c, d, x, y]);
            return Ok((t, t));
        };
        if let Some(m) = place.matrix {
            [a, b, c, d] = m;
        }
        let base_x = place.x.base.as_deref().or(base);
        let base_y = place.y.base.as_deref().or(base);
        let (mut fixed_x, mut fixed_y) = (false, false);
        if let (Some(ex), Some(ey)) = (&place.x.expr, &place.y.expr) {
            let (p1, a1, fx) = self.axis_position(mark, ex, Dir::X, "_", false, warnings)?;
            let (p2, a2, fy) = self.axis_position(mark, ey, Dir::Y, "_", false, warnings)?;
            (fixed_x, fixed_y) = (fx, fy);
            let mark_at = meet(p1, a1, p2, a2);
            let mut base_at = Point::ZERO;
            if let (Some(bx), Some(by)) = (base_x, base_y)
                && self.font.glyph(bx).is_some()
                && self.font.glyph(by).is_some()
            {
                let (q1, b1, _) = self.axis_position(bx, ex, Dir::X, "", true, warnings)?;
                let (q2, b2, _) = self.axis_position(by, ey, Dir::Y, "", true, warnings)?;
                base_at = meet(q1, b1, q2, b2);
            }
            x += if fixed_x { mark_at.x } else { base_at.x - mark_at.x };
            y += if fixed_y { mark_at.y } else { base_at.y - mark_at.y };
        }
        if !fixed_x && let Some(t) = base_x.and_then(|n| placed.get(n)) {
            x += t.translation().x - advance;
        }
        if !fixed_y && let Some(t) = base_y.and_then(|n| placed.get(n)) {
            y += t.translation().y;
        }
        let unflipped = Affine::new([a, b, c, d, x, y]);
        let mut t = unflipped;
        let bounds = self.facts(mark).and_then(|f| f.bounds);
        if let Some(r) = bounds {
            if place.y.flip {
                let (p0, p1) = (t * Point::new(r.x0, r.y0), t * Point::new(r.x1, r.y1));
                t = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, p0.y + p1.y]) * t;
            }
            if place.x.flip {
                let (p0, p1) = (t * Point::new(r.x0, r.y0), t * Point::new(r.x1, r.y1));
                t = Affine::new([-1.0, 0.0, 0.0, 1.0, p0.x + p1.x, 0.0]) * t;
            }
        }
        if t.as_coeffs().iter().any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE) {
            return Err(format!("`{mark}` is placed at a position that is not a usable number"));
        }
        Ok((t, unflipped))
    }

    /// The point a position names on `glyph` along one axis.
    fn axis_position(
        &mut self,
        glyph: &str,
        expr: &PosExpr,
        dir: Dir,
        prefix: &str,
        is_base: bool,
        warnings: &mut Vec<String>,
    ) -> Result<AxisPos, String> {
        let angle = match dir {
            Dir::X => 90.0 - self.italic,
            Dir::Y => 0.0,
        };
        let Some(f) = self.facts(glyph) else { return Ok((Point::ZERO, angle, false)) };
        let r = f.bounds.unwrap_or(Rect::ZERO);
        let atoms = expr.atoms();
        if atoms.is_empty() {
            let v = match expr.eval(&HashMap::new())? {
                Val::Num(v) => v,
                Val::Pt { .. } => return Err(format!("`{}` is not a position", expr.source())),
            };
            let at = match dir {
                Dir::X => Point::new(v - r.x0, 0.0),
                Dir::Y => Point::new(0.0, v - r.y0),
            };
            return Ok((at, angle, true));
        }
        let mut state: AxisPos = (Point::ZERO, angle, false);
        let mut env = HashMap::new();
        for atom in atoms {
            let (PNode::Name(name) | PNode::Pct(name)) = atom else { continue };
            state = self.resolve(name, state, &f, glyph, dir, is_base, prefix, warnings);
            env.insert(name.clone(), Val::Pt { x: state.0.x, y: state.0.y, tuple_only: !is_base && !state.2 });
        }
        match expr.eval(&env)? {
            Val::Pt { x, y, .. } if x.is_finite() && y.is_finite() => Ok((Point::new(x, y), state.1, state.2)),
            _ => Err(format!("`{}` is not a position", expr.source())),
        }
    }

    /// One name of a position, looked up in the order the module doc gives. A name that matches
    /// nothing keeps the point found so far.
    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &self,
        name: &str,
        state: AxisPos,
        f: &Facts,
        glyph: &str,
        dir: Dir,
        is_base: bool,
        prefix: &str,
        warnings: &mut Vec<String>,
    ) -> AxisPos {
        let (_, angle, fixed) = state;
        let prefixed = format!("{prefix}{name}");
        for wanted in [prefixed.as_str(), name] {
            if let Some((_, x, y)) = f.anchors.iter().find(|(n, _, _)| n == wanted) {
                return (Point::new(*x, *y), angle, fixed);
            }
        }
        for wanted in [prefixed.as_str(), name] {
            if let Some(g) = f.guides.iter().find(|g| g.name.as_deref() == Some(wanted)) {
                return (Point::new(g.x, g.y), g.angle, fixed);
            }
        }
        let mut name = name;
        if let Some(g) = self.master.guides.iter().find(|g| g.name.as_deref() == Some(name)) {
            if !is_base {
                return (Point::new(g.x, g.y), g.angle, fixed);
            }
            name = match dir {
                Dir::X => "center",
                Dir::Y => "top",
            };
        }
        match (dir, name) {
            (_, "origin") => return (Point::ZERO, angle, true),
            (Dir::X, "width") => return (Point::new(f.width, 0.0), angle, true),
            (Dir::Y, "height") => return (Point::new(0.0, f.height), angle, true),
            _ => {}
        }
        let r = f.bounds.unwrap_or(Rect::ZERO);
        if let Some((_, value)) = self.metrics.iter().find(|(n, _)| *n == name) {
            return (Point::new(0.0, value - r.y0), angle, true);
        }
        let mut share = 0.5;
        if let Some(pct) = name.strip_suffix('%')
            && let Ok(v) = pct.parse::<f64>()
            && v.is_finite()
        {
            share = v * 0.01;
            name = "center";
        }
        let (w, h) = (r.width(), r.height());
        let at = match (dir, name) {
            (_, "center") if is_base => Point::new(r.x0 + w * share, r.y0 + h * share),
            (_, "center") => Point::new(r.x0 + w - w * share, r.y0 + h - h * share),
            (Dir::Y, "top") => Point::new(0.0, if is_base { r.y1 } else { r.y0 }),
            (Dir::Y, "innerTop") => Point::new(0.0, r.y1),
            (Dir::Y, "bottom") => Point::new(0.0, if is_base { r.y0 } else { r.y1 }),
            (Dir::Y, "innerBottom") => Point::new(0.0, r.y0),
            (Dir::X, "left") => Point::new(if is_base { r.x0 } else { r.x1 }, 0.0),
            (Dir::X, "innerLeft") => Point::new(r.x0, 0.0),
            (Dir::X, "right") => Point::new(if is_base { r.x1 } else { r.x0 }, 0.0),
            (Dir::X, "innerRight") => Point::new(r.x1, 0.0),
            _ => {
                if !KEYWORDS.contains(&name) && !self.metrics.iter().any(|(n, _)| *n == name) {
                    warnings
                        .push(format!("`{name}` is not an anchor, guide, metric or position of `{glyph}`; it counts as the point found before it"));
                }
                return state;
            }
        };
        (at, angle, fixed)
    }

    /// The bounds of shapes on this master, components drawn through the font.
    fn shapes_bounds(&self, shapes: &[Shape]) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        for s in shapes {
            let path = match s {
                Shape::Path(p) => p.to_bez().ok(),
                Shape::Component(c) => self.font.component_outline(c, &self.master.id).ok(),
                Shape::Image(_) | Shape::Raw(_) => None,
            };
            if let Some(p) = path.filter(|p| !p.elements().is_empty()) {
                let b = p.bounding_box();
                out = Some(out.map_or(b, |o| o.union(b)));
            }
        }
        out
    }

    /// A width or margin: a number, a glyph's own value, or a formula over glyph values. `None`
    /// when a glyph it names has no outline to measure.
    fn metric_value(&mut self, text: &str, metric: Metric) -> Result<Option<f64>, String> {
        if let Ok(v) = text.parse::<f64>() {
            return if v.is_finite() { Ok(Some(v)) } else { Err(format!("`{text}` is not a usable number")) };
        }
        if let Some(f) = self.facts(text) {
            return Ok(metric.of(&f));
        }
        let quoted = text.contains('"');
        let mut formula = String::new();
        let mut rest = text;
        while let Some(c) = rest.chars().next() {
            let (name, after) = if quoted {
                if c != '"' {
                    formula.push(c);
                    rest = rest.get(c.len_utf8()..).unwrap_or("");
                    continue;
                }
                let inner = rest.get(1..).unwrap_or("");
                let Some(end) = inner.find('"') else { return Err(format!("`{text}`: a quoted glyph name is not closed")) };
                (inner.get(..end).unwrap_or(""), inner.get(end.saturating_add(1)..).unwrap_or(""))
            } else if c.is_ascii_alphabetic() || c == '_' || rest.starts_with(".notdef") {
                let end = if rest.starts_with(".notdef") {
                    ".notdef".len()
                } else {
                    rest.find(|d: char| !(d.is_ascii_alphanumeric() || d == '_' || d == '.')).unwrap_or(rest.len())
                };
                let (name, after) = rest.split_at(end);
                if after.starts_with('(') {
                    // A function of the formula language (`max(…)`).
                    formula.push_str(name);
                    rest = after;
                    continue;
                }
                (name, after)
            } else {
                formula.push(c);
                rest = rest.get(c.len_utf8()..).unwrap_or("");
                continue;
            };
            let (flipped, after) = match after.strip_prefix('\'') {
                Some(a) => (true, a),
                None => (false, after),
            };
            let Some(f) = self.facts(name) else { return Err(format!("`{text}`: there is no glyph `{name}`")) };
            let Some(v) = (if flipped { metric.opposite() } else { metric }).of(&f) else { return Ok(None) };
            formula.push_str(&format!("({v})"));
            rest = after;
        }
        let formula = formula.replace('\'', "");
        let value = Expr::parse(&formula).and_then(|e| e.eval(|_| None)).map_err(|e| format!("`{text}`: {e}"))?;
        Ok(Some(value))
    }
}

/// The names a position may use that are not anchors or guides.
const KEYWORDS: [&str; 12] =
    ["origin", "width", "height", "center", "top", "bottom", "left", "right", "innerTop", "innerBottom", "innerLeft", "innerRight"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Metric {
    Width,
    Left,
    Right,
}

impl Metric {
    fn opposite(self) -> Metric {
        match self {
            Metric::Width => Metric::Width,
            Metric::Left => Metric::Right,
            Metric::Right => Metric::Left,
        }
    }

    fn of(self, f: &Facts) -> Option<f64> {
        match self {
            Metric::Width => Some(f.width),
            Metric::Left => f.bounds.map(|b| b.x0),
            Metric::Right => f.bounds.map(|b| f.width - b.x1),
        }
    }
}

/// Where the line through `p1` at `a1` degrees crosses the one through `p2` at `a2`; when they
/// are parallel, the point itself if both lines are the same, else the origin.
fn meet(p1: Point, a1: f64, p2: Point, a2: f64) -> Point {
    let q1 = Point::new(p1.x + a1.to_radians().cos(), p1.y + a1.to_radians().sin());
    let q2 = Point::new(p2.x + a2.to_radians().cos(), p2.y + a2.to_radians().sin());
    let denom = (p1.x - q1.x) * (p2.y - q2.y) - (p1.y - q1.y) * (p2.x - q2.x);
    if (denom * 10_000.0).round() == 0.0 {
        return if p1 == p2 && a1 == a2 { p1 } else { Point::ZERO };
    }
    let c1 = p1.x * q1.y - p1.y * q1.x;
    let c2 = p2.x * q2.y - p2.y * q2.x;
    let x = (c1 * (p2.x - q2.x) - (p1.x - q1.x) * c2) / denom;
    let y = (c1 * (p2.y - q2.y) - (p1.y - q1.y) * c2) / denom;
    // The two directions through the origin of a line are computed in floating point, so a
    // vertical line gives x within 1e-13 of the point's x; snap such noise.
    Point::new(clean(x), clean(y))
}

/// `v` with floating-point noise next to a whole number removed, and without a negative zero.
fn clean(v: f64) -> f64 {
    let r = v.round();
    let v = if (v - r).abs() < 1e-9 { r } else { v };
    if v == 0.0 { 0.0 } else { v }
}

/// A component of `glyph` placed by `t`, kept where it is by automatic alignment.
fn component(glyph: &str, t: Affine) -> Result<Component, String> {
    let [a, b, c, d, e, f] = t.as_coeffs().map(clean);
    let mut comp = Component::new(glyph, e, f);
    if [a, b, c, d] != [1.0, 0.0, 0.0, 1.0] {
        comp.transform = Transform::Matrix([a, b, c, d, e, f]);
    }
    comp.info.alignment = Alignment::Disabled;
    if comp.transform.numbers().iter().any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE) {
        return Err(format!("`{glyph}` is placed at a position that is not a usable number"));
    }
    Ok(comp)
}

/// Moves a shape horizontally.
fn shift_shape(s: &mut Shape, dx: f64) {
    match s {
        Shape::Path(p) => p.nodes.iter_mut().for_each(|n| n.x = clean(n.x + dx)),
        Shape::Component(c) => match &mut c.transform {
            Transform::Decomposed { pos, .. } => pos.0 = clean(pos.0 + dx),
            Transform::Matrix(m) => {
                if let Some(e) = m.get_mut(4) {
                    *e = clean(*e + dx);
                }
            }
        },
        Shape::Image(_) | Shape::Raw(_) => {}
    }
}

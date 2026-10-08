//! `features.fea` as glyphsLib 6.15.0 writes and reads it (`builder/features.py`, Apache-2.0; see
//! `NOTICE`): the model's prefixes, classes and features as one text, with comment markers that
//! let a reader split the text back into them.
//!
//! Writing, in this order and separated by blank lines:
//!
//! - each class: `# automatic` when it is automatic, then `@Name = [ code` and `];` on the next
//!   line;
//! - each prefix: `# Prefix: name` (not for glyphsLib's anonymous prefix, `<anonymous>`),
//!   `# automatic` when it is automatic, then its code;
//! - each feature: `feature tag {`, its notes after `# notes:` (each line behind `# `), its labels
//!   as a `featureNames` block, `# automatic` when it is automatic, its code (behind `# disabled`
//!   and with each line commented out when it is disabled), `# Automatic Code` after the code of a
//!   manual `kern` feature that has no such line, then `} tag;`.
//!
//! The text ends with a line break; a document without feature code gets none. A disabled prefix
//! or class is written as glyphsLib writes a disabled feature: `# disabled`, then its lines
//! commented out (glyphsLib writes them as live code), and a disabled prefix always gets its
//! `# Prefix:` line.
//!
//! Reading splits the text the way glyphsLib does: a `# Prefix:` comment starts a prefix that
//! runs to the next class, feature, table block or prefix; class definitions and feature blocks
//! are classes and features; everything else between them is an anonymous prefix. A prefix's code
//! loses the line breaks the writer adds after it (two before the next item, one at the end;
//! glyphsLib takes two from a named prefix and one from an anonymous one wherever they stand). The
//! result is the model's only when writing it gives the text back byte for byte. Otherwise the text stays
//! as written, as one unnamed prefix, and is written back unchanged.
//!
//! The text is scanned once, without recursion; braces only count a depth (capped at
//! [`MAX_DEPTH`]). An `anon` block, whose content is not feature syntax, keeps the text as written.

use fontcraft_model::{Features, Localized, Snippet};

/// glyphsLib's name for a prefix the text gives no name.
pub const ANONYMOUS: &str = "<anonymous>";
/// The deepest nesting of braces the reader follows; a deeper text is kept as written.
pub const MAX_DEPTH: usize = 64;
/// The most prefixes, classes and features a text may split into; a text with more is kept as
/// written.
pub const MAX_SNIPPETS: usize = 100_000;
/// The line glyphsLib adds to a manual `kern` feature, where the automatic code goes.
const KERN_MARKER: &str = "# Automatic Code";

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// Whether the features are a text kept as written: one unnamed prefix and nothing else.
pub fn is_kept_text(f: &Features) -> bool {
    match (f.prefixes.as_slice(), f.classes.is_empty(), f.features.is_empty()) {
        ([p], true, true) => p.name.is_empty() && !p.automatic && !p.disabled && p.notes.is_empty() && p.labels.is_empty(),
        _ => false,
    }
}

/// Code behind `# disabled`, each line commented out, as glyphsLib writes a disabled feature's.
fn commented_out(code: &str) -> String {
    let mut out = String::from("# disabled\n");
    out.push_str(&code.lines().map(|l| format!("#{l}")).collect::<Vec<_>>().join("\n"));
    out
}

/// The lines after `# disabled` with their comment signs taken off, or `None` when one is not
/// commented out.
fn uncommented(lines: &str) -> Option<String> {
    if lines.is_empty() {
        return Some(String::new());
    }
    Some(lines.split('\n').map(|l| l.strip_prefix('#')).collect::<Option<Vec<_>>>()?.join("\n"))
}

fn automatic(on: bool) -> &'static str {
    if on { "# automatic\n" } else { "" }
}

/// A label's text as a `name` record's string: `\` and `"` escaped.
fn escape(s: &str) -> String {
    s.replace('\\', "\\005c").replace('"', "\\0022")
}

/// The `name` record glyphsLib writes for a label, or `None` for a label it leaves out (no text,
/// or a language without a Windows language id).
pub fn label_record(l: &Localized) -> Option<String> {
    if l.value.is_empty() {
        return None;
    }
    if l.language == fontcraft_model::DEFAULT_LANGUAGE {
        return Some(format!("  name \"{}\";", escape(&l.value)));
    }
    let id = crate::fontinfo::language_id(&l.language)?;
    Some(format!("  name 3 1 0x{id:X} \"{}\";", escape(&l.value)))
}

fn write_feature(f: &Snippet) -> String {
    let mut lines = vec![format!("feature {} {{", f.name)];
    if !f.notes.is_empty() {
        lines.push("# notes:".into());
        lines.extend(f.notes.lines().map(|l| format!("# {l}")));
    }
    let names: Vec<String> = f.labels.iter().filter_map(label_record).collect();
    if !names.is_empty() {
        lines.push("featureNames {".into());
        lines.extend(names);
        lines.push("};".into());
    }
    if f.automatic {
        lines.push("# automatic".into());
    }
    if f.disabled {
        lines.push("# disabled".into());
        lines.extend(f.code.lines().map(|l| format!("#{l}")));
    } else {
        lines.push(f.code.clone());
    }
    if f.name == "kern" && !f.automatic && !f.code.contains(KERN_MARKER) {
        lines.push(format!("{KERN_MARKER}\n"));
    }
    lines.push(format!("}} {};", f.name));
    lines.join("\n")
}

/// The features as one `features.fea` text.
pub fn write(f: &Features) -> String {
    if is_kept_text(f) {
        return f.prefixes.first().map(|p| p.code.clone()).unwrap_or_default();
    }
    let classes: Vec<String> = f
        .classes
        .iter()
        .map(|c| {
            let at = if c.name.starts_with('@') { "" } else { "@" };
            let definition = format!("{at}{} = [ {}\n];", c.name, c.code);
            format!("{}{}", automatic(c.automatic), if c.disabled { commented_out(&definition) } else { definition })
        })
        .collect();
    let prefixes: Vec<String> = f
        .prefixes
        .iter()
        .map(|p| {
            // A disabled prefix always names itself, so that it reads back as one.
            let name = if p.name == ANONYMOUS && !p.disabled { String::new() } else { format!("# Prefix: {}\n", p.name) };
            format!("{name}{}{}", automatic(p.automatic), if p.disabled { commented_out(&p.code) } else { p.code.clone() })
        })
        .collect();
    let features: Vec<String> = f.features.iter().map(write_feature).collect();
    let parts: Vec<String> = [classes, prefixes, features].into_iter().map(|p| p.join("\n\n")).filter(|p| !p.is_empty()).collect();
    let text = parts.join("\n\n") + "\n";
    if text.trim().is_empty() { String::new() } else { text }
}

/// The kinds of feature data [`losses`] reports, as its lines begin.
pub const FEATURE_LOSSES: [&str; 5] = [
    "the features' labels without text or in a language without a Windows language id",
    "the labels of features that are not automatic, which come back as part of their code",
    "the notes of prefixes and classes",
    "feature notes and disabled code that end with a line break, which comes back without it",
    "the keys of prefixes, classes and features that the model keeps as written",
];

/// What `features.fea` leaves out of the model's feature data, one line per kind with a count.
pub fn losses(f: &Features) -> Vec<String> {
    let all = || f.prefixes.iter().chain(&f.classes).chain(&f.features);
    let counts = [
        f.features.iter().map(|s| s.labels.iter().filter(|l| label_record(l).is_none()).count()).sum::<usize>(),
        f.features.iter().filter(|s| !s.automatic && s.labels.iter().any(|l| label_record(l).is_some())).count(),
        f.prefixes.iter().chain(&f.classes).filter(|s| !s.notes.is_empty()).count(),
        f.features.iter().filter(|s| s.notes.ends_with('\n') || s.disabled && s.code.ends_with('\n')).count(),
        all().filter(|s| !s.extra.is_empty()).count(),
    ];
    if is_kept_text(f) {
        return Vec::new();
    }
    FEATURE_LOSSES.iter().zip(counts).filter(|(_, n)| *n > 0).map(|(what, n)| format!("{what} ({n})")).collect()
}

// ── scanning ───────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Word,
    Comment,
    String,
    Symbol(u8),
}

/// The tokens of a feature text: kind, start and end (byte offsets). `None` when the text has an
/// unterminated string.
fn tokens(text: &str) -> Option<Vec<(Tok, usize, usize)>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut include = false;
    while let Some(&c) = b.get(i) {
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let kind = match c {
            b'#' => {
                while b.get(i).is_some_and(|&c| c != b'\n' && c != b'\r') {
                    i += 1;
                }
                Tok::Comment
            }
            b'"' => {
                i += 1;
                while b.get(i).is_some_and(|&c| c != b'"') {
                    i += 1;
                }
                b.get(i)?;
                i += 1;
                Tok::String
            }
            // An include's file name is everything up to the closing parenthesis.
            b'(' if include => {
                while b.get(i).is_some_and(|&c| c != b')') {
                    i += 1;
                }
                b.get(i)?;
                i += 1;
                Tok::String
            }
            b'{' | b'}' | b';' | b'[' | b']' | b'(' | b')' | b'=' | b',' | b'<' | b'>' | b'\'' => {
                i += 1;
                Tok::Symbol(c)
            }
            _ => {
                while b.get(i).is_some_and(|&c| !c.is_ascii_whitespace() && !b"{};[]()=,<>'\"#".contains(&c)) {
                    i += 1;
                }
                Tok::Word
            }
        };
        include = kind == Tok::Word && text.get(start..i) == Some("include");
        out.push((kind, start, i));
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Comment,
    Class,
    Feature,
    Table,
    Other,
}

/// One top-level statement: its kind and where its text starts and ends.
#[derive(Clone, Debug)]
struct Stmt {
    kind: Kind,
    start: usize,
    end: usize,
}

/// The top-level statements of a feature text, as feaLib would list them. `None` for a text
/// whose structure the reader does not follow (unbalanced braces, nesting beyond [`MAX_DEPTH`],
/// an `anon` block).
fn statements(text: &str) -> Option<Vec<Stmt>> {
    let toks = tokens(text)?;
    let mut out: Vec<Stmt> = Vec::new();
    let mut current: Option<Stmt> = None;
    let mut depth = 0usize;
    let mut skip = false;
    for (i, &(kind, start, end)) in toks.iter().enumerate() {
        if std::mem::take(&mut skip) {
            continue;
        }
        if let Some(st) = current.as_mut() {
            st.end = end;
            match kind {
                Tok::Symbol(b'{') => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return None;
                    }
                }
                Tok::Symbol(b'}') => depth = depth.checked_sub(1)?,
                Tok::Symbol(b';') if depth == 0 => out.extend(current.take()),
                _ => {}
            }
            continue;
        }
        match kind {
            Tok::Comment => out.push(Stmt { kind: Kind::Comment, start, end }),
            // A lone `;` belongs to the statement before it: feaLib skips it.
            Tok::Symbol(b';') => match out.last_mut() {
                Some(last) => last.end = end,
                None => out.push(Stmt { kind: Kind::Other, start, end }),
            },
            Tok::Symbol(b'}') => return None,
            _ => {
                let w = text.get(start..end).unwrap_or_default();
                if matches!(w, "anon" | "anonymous") {
                    return None;
                }
                // `include(file)` needs no semicolon: it ends with its file name.
                if w == "include" {
                    let file = toks.get(i + 1).filter(|t| t.0 == Tok::String);
                    skip = file.is_some();
                    out.push(Stmt { kind: Kind::Other, start, end: file.map_or(end, |t| t.2) });
                    continue;
                }
                let next_is_equals = toks.get(i + 1).is_some_and(|t| t.0 == Tok::Symbol(b'='));
                let kind = match w {
                    _ if kind == Tok::Word && w.starts_with('@') && next_is_equals => Kind::Class,
                    "feature" => Kind::Feature,
                    "table" => Kind::Table,
                    _ => Kind::Other,
                };
                if w == "{" {
                    depth = 1;
                }
                current = Some(Stmt { kind, start, end });
            }
        }
    }
    if current.is_some() || depth != 0 {
        return None;
    }
    Some(out)
}

// ── reading ────────────────────────────────────────────────────────────────────────────────────

/// Up to `n` line breaks taken off the end, as glyphsLib's `_rstrip_newlines` takes them.
fn rstrip_newlines(s: &str, n: usize) -> &str {
    let mut s = s;
    for _ in 0..n {
        s = s.strip_suffix('\n').unwrap_or(s);
    }
    s
}

/// A `name` record's string with glyphsLib's two escapes undone.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('\\') {
        out.push_str(rest.get(..i).unwrap_or_default());
        let after = rest.get(i + 1..).unwrap_or_default();
        match after.get(..4).map(str::to_ascii_lowercase).as_deref() {
            Some("005c") => out.push('\\'),
            Some("0022") => out.push('"'),
            _ => {
                out.push('\\');
                rest = after;
                continue;
            }
        }
        rest = after.get(4..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// A label from one line of a `featureNames` block, in the form [`label_record`] writes.
fn read_label(line: &str) -> Option<Localized> {
    let body = line.strip_prefix("  name ")?.strip_suffix(';')?;
    let i = body.find('"')?;
    let (head, string) = (body.get(..i)?, body.get(i..)?);
    let value = unescape(string.strip_prefix('"')?.strip_suffix('"')?);
    let language = match head {
        "" => fontcraft_model::DEFAULT_LANGUAGE.to_string(),
        h => {
            let hex = h.strip_prefix("3 1 0x")?.strip_suffix(' ')?;
            let id = u16::from_str_radix(hex, 16).ok()?;
            crate::fontinfo::language_tag_any(id)?.to_string()
        }
    };
    Some(Localized { language, value, extra: Default::default() })
}

/// The next line of `s` and the rest after its line break.
fn line(s: &str) -> (&str, &str) {
    match s.find('\n') {
        Some(i) => (s.get(..i).unwrap_or_default(), s.get(i + 1..).unwrap_or_default()),
        None => (s, ""),
    }
}

/// Takes `marker` as a whole line off the start of `s`.
fn take_line<'a>(s: &'a str, marker: &str) -> Option<&'a str> {
    let (first, rest) = line(s);
    (first == marker).then_some(rest)
}

/// A feature block in the form [`write_feature`] writes, read back.
fn read_feature(text: &str) -> Option<Snippet> {
    let header_end = text.find('\n')?;
    let tag = text.get(..header_end)?.strip_prefix("feature ")?.strip_suffix(" {")?;
    if tag.is_empty() || tag.contains(char::is_whitespace) {
        return None;
    }
    let body = text.get(header_end + 1..)?.strip_suffix(&format!("}} {tag};"))?.strip_suffix('\n')?;
    let mut rest = body;
    let mut notes: Vec<&str> = Vec::new();
    if let Some(after) = take_line(rest, "# notes:") {
        rest = after;
        loop {
            let (l, after) = line(rest);
            match l.strip_prefix("# ") {
                Some(n) if l != "# automatic" && l != "# disabled" => {
                    notes.push(n);
                    rest = after;
                }
                _ => break,
            }
        }
    }
    let before_names = rest;
    let mut labels = Vec::new();
    if let Some(after) = take_line(rest, "featureNames {") {
        rest = after;
        loop {
            let (l, after) = line(rest);
            rest = after;
            if l == "};" {
                break;
            }
            labels.push(read_label(l)?);
            if rest.is_empty() {
                return None;
            }
        }
    }
    let automatic = match take_line(rest, "# automatic") {
        Some(after) => {
            rest = after;
            true
        }
        None => false,
    };
    // glyphsLib takes labels only from an automatic feature; a manual one keeps them as code.
    if !automatic && !labels.is_empty() {
        labels.clear();
        rest = before_names;
    }
    // The line glyphsLib adds to a manual kern feature.
    if tag == "kern"
        && !automatic
        && let Some(code) = rest.strip_suffix(&format!("\n{KERN_MARKER}\n"))
    {
        rest = code;
    }
    let (code, disabled) = match take_line(rest, "# disabled") {
        Some(after) => (uncommented(after)?, true),
        None => (rest.to_string(), false),
    };
    Some(Snippet { name: tag.to_string(), code, automatic, disabled, notes: notes.join("\n"), labels, ..Default::default() })
}

/// A class definition in the form [`write`] gives it, read back.
fn read_class(text: &str, automatic: bool) -> Option<Snippet> {
    let (name, rest) = text.strip_prefix('@')?.split_once(" = [ ")?;
    let code = rest.strip_suffix("\n];")?;
    Some(Snippet { name: name.to_string(), code: code.to_string(), automatic, ..Default::default() })
}

fn is_comment(text: &str, st: Option<&Stmt>, wanted: &str) -> bool {
    st.is_some_and(|s| s.kind == Kind::Comment && text.get(s.start..s.end) == Some(wanted))
}

/// A disabled class at statement `i` (`# automatic`, if it is automatic, then `# disabled` and the
/// definition commented out, line by line): how many statements it takes, and the class.
fn disabled_class(text: &str, stmts: &[Stmt], i: usize) -> Option<(usize, Snippet)> {
    let automatic = is_comment(text, stmts.get(i), "# automatic");
    let at = if automatic { i + 1 } else { i };
    if !is_comment(text, stmts.get(at), "# disabled") {
        return None;
    }
    let comment = |j: usize| stmts.get(j).filter(|s| s.kind == Kind::Comment).and_then(|s| text.get(s.start..s.end));
    if !comment(at + 1)?.starts_with("#@") {
        return None;
    }
    let mut lines = Vec::new();
    let mut j = at + 1;
    loop {
        let l = comment(j)?;
        lines.push(l.strip_prefix('#')?);
        j += 1;
        if l == "#];" {
            break;
        }
    }
    let mut class = read_class(&lines.join("\n"), automatic)?;
    class.disabled = true;
    Some((j - i, class))
}

fn prefix_name<'a>(text: &'a str, st: &Stmt) -> Option<&'a str> {
    if st.kind != Kind::Comment {
        return None;
    }
    text.get(st.start..st.end)?.strip_prefix("# Prefix: ")
}

/// The text split as glyphsLib splits it, or `None` where the reader cannot follow it.
fn split(text: &str) -> Option<Features> {
    let stmts = statements(text)?;
    let mut out = Features::default();
    let starts = |j: usize| stmts.get(j).map_or(text.len(), |s| s.start);
    let mut unhandled: Option<usize> = None;
    // A prefix's code loses the line breaks the writer puts after it: two before the next item,
    // one at the end of the text.
    let breaks = |to: usize| if to < text.len() { 2 } else { 1 };
    let flush = |out: &mut Features, from: &mut Option<usize>, to: usize| -> Option<()> {
        if let Some(s) = from.take() {
            let code = rstrip_newlines(text.get(s..to)?, breaks(to)).to_string();
            out.prefixes.push(Snippet { name: ANONYMOUS.into(), code, ..Default::default() });
        }
        Some(())
    };
    let mut i = 0;
    while let Some(st) = stmts.get(i) {
        if out.prefixes.len() + out.classes.len() + out.features.len() > MAX_SNIPPETS {
            return None;
        }
        let auto_class = is_comment(text, Some(st), "# automatic") && stmts.get(i + 1).is_some_and(|s| s.kind == Kind::Class);
        if let Some((n, class)) = disabled_class(text, &stmts, i) {
            flush(&mut out, &mut unhandled, st.start)?;
            out.classes.push(class);
            i += n;
        } else if let Some(name) = prefix_name(text, st) {
            flush(&mut out, &mut unhandled, st.start)?;
            let mut j = i + 1;
            while let Some(s) = stmts.get(j) {
                let stop = matches!(s.kind, Kind::Class | Kind::Feature | Kind::Table)
                    || prefix_name(text, s).is_some()
                    || is_comment(text, Some(s), "# automatic") && stmts.get(j + 1).is_some_and(|n| n.kind == Kind::Class)
                    || disabled_class(text, &stmts, j).is_some();
                if stop {
                    break;
                }
                j += 1;
            }
            let automatic = j > i + 1 && is_comment(text, stmts.get(i + 1), "# automatic");
            let first = if automatic { i + 2 } else { i + 1 };
            let mut code =
                if first < j { rstrip_newlines(text.get(starts(first)..starts(j))?, breaks(starts(j))).to_string() } else { String::new() };
            let disabled = first < j && is_comment(text, stmts.get(first), "# disabled");
            if disabled {
                code = uncommented(take_line(&code, "# disabled")?)?;
            }
            out.prefixes.push(Snippet { name: name.to_string(), code, automatic, disabled, ..Default::default() });
            i = j;
        } else if auto_class || st.kind == Kind::Class {
            flush(&mut out, &mut unhandled, st.start)?;
            let class = if auto_class { stmts.get(i + 1)? } else { st };
            out.classes.push(read_class(text.get(class.start..class.end)?, auto_class)?);
            i += if auto_class { 2 } else { 1 };
        } else if st.kind == Kind::Feature {
            flush(&mut out, &mut unhandled, st.start)?;
            out.features.push(read_feature(text.get(st.start..st.end)?)?);
            i += 1;
        } else {
            unhandled.get_or_insert(st.start);
            i += 1;
        }
    }
    flush(&mut out, &mut unhandled, text.len())?;
    Some(out)
}

/// The features a `features.fea` text holds: split into prefixes, classes and features when
/// writing them gives the same text, else the text as one unnamed prefix.
pub fn read(text: &str) -> Features {
    match split(text) {
        Some(f) if !text.is_empty() && write(&f) == text => f,
        _ => Features { prefixes: vec![Snippet { code: text.to_string(), ..Default::default() }], ..Default::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What glyphsLib 6.15.0 writes for a font with two classes, a named and an anonymous prefix,
    /// a stylistic set with notes and labels, a manual kern feature and a disabled feature.
    const GLYPHSLIB: &str = "@Foo = [ a b\n];\n\n@Bar = [ c\n];\n\n# Prefix: Languagesystems\n# automatic\nlanguagesystem DFLT dflt;\n\n# hi\n\nfeature ss01 {\n# notes:\n# line one\n#   line two\nfeatureNames {\n  name \"Alt \\0022a\\0022\";\n  name 3 1 0x407 \"Alternativ\";\n};\n# automatic\nsub a by b;\n} ss01;\n\nfeature kern {\npos a b -10;\n# Automatic Code\n\n} kern;\n\nfeature liga {\n# disabled\n#sub a b by c;\n#sub b c by a;\n} liga;\n";

    fn snippet(name: &str, code: &str) -> Snippet {
        Snippet { name: name.into(), code: code.into(), ..Default::default() }
    }

    fn label(language: &str, value: &str) -> Localized {
        Localized { language: language.into(), value: value.into(), extra: Default::default() }
    }

    fn glyphslib_model() -> Features {
        Features {
            prefixes: vec![Snippet { automatic: true, ..snippet("Languagesystems", "languagesystem DFLT dflt;") }, snippet(ANONYMOUS, "# hi")],
            classes: vec![snippet("Foo", "a b"), snippet("Bar", "c")],
            features: vec![
                Snippet {
                    automatic: true,
                    notes: "line one\n  line two".into(),
                    labels: vec![label("dflt", "Alt \"a\""), label("DEU", "Alternativ")],
                    ..snippet("ss01", "sub a by b;")
                },
                snippet("kern", "pos a b -10;"),
                Snippet { disabled: true, ..snippet("liga", "sub a b by c;\nsub b c by a;") },
            ],
        }
    }

    #[test]
    fn the_writer_writes_what_glyphslib_writes() {
        assert_eq!(write(&glyphslib_model()), GLYPHSLIB);
        assert_eq!(write(&Features::default()), "");
    }

    #[test]
    fn glyphslib_text_reads_back_into_snippets() {
        let f = read(GLYPHSLIB);
        let mut want = glyphslib_model();
        // As glyphsLib reads it: the anonymous prefix after a named one is part of it.
        want.prefixes = vec![Snippet { automatic: true, ..snippet("Languagesystems", "languagesystem DFLT dflt;\n\n# hi") }];
        assert_eq!(f, want);
        assert_eq!(write(&f), GLYPHSLIB);
    }

    #[test]
    fn disabled_prefixes_and_classes_are_commented_out_and_read_back() {
        let f = Features {
            prefixes: vec![Snippet { disabled: true, automatic: true, ..snippet("Old", "languagesystem latn dflt;\nlanguagesystem latn TRK;") }],
            classes: vec![Snippet { disabled: true, ..snippet("Gone", "a b") }, Snippet { disabled: true, automatic: true, ..snippet("Auto", "") }],
            features: vec![snippet("liga", "sub f i by fi;")],
        };
        let text = write(&f);
        assert_eq!(
            text,
            "# disabled\n#@Gone = [ a b\n#];\n\n# automatic\n# disabled\n#@Auto = [ \n#];\n\n# Prefix: Old\n# automatic\n# disabled\n#languagesystem latn dflt;\n#languagesystem latn TRK;\n\nfeature liga {\nsub f i by fi;\n} liga;\n"
        );
        assert_eq!(read(&text), f);
    }

    #[test]
    fn labels_glyphslib_cannot_write_are_left_out() {
        let mut f = Features::default();
        f.features.push(Snippet {
            automatic: true,
            labels: vec![label("dflt", ""), label("SKIPME", "x"), label("ENG", "Set")],
            ..snippet("ss02", "sub a by b;")
        });
        let text = write(&f);
        assert!(text.contains("featureNames {\n  name 3 1 0x409 \"Set\";\n};"), "{text}");
        assert_eq!(read(&text).features[0].labels, [label("ENG", "Set")]);
    }

    #[test]
    fn a_manual_feature_keeps_its_names_in_its_code() {
        let text = "feature ss01 {\nfeatureNames {\n  name \"x\";\n};\nsub a by b;\n} ss01;\n";
        let f = read(text);
        assert_eq!(f.features, [snippet("ss01", "featureNames {\n  name \"x\";\n};\nsub a by b;")]);
        assert_eq!(write(&f), text);
    }

    #[test]
    fn text_that_does_not_write_back_the_same_is_kept_as_written() {
        for text in [
            "",
            "\n",
            "languagesystem DFLT dflt;\nfeature liga {\n  sub f i by fi;\n} liga;",
            "feature liga{\nsub f i by fi;\n}liga;\n",
            "@Upper = [A B C];\n",
            "feature liga {\nsub f i by fi;\n} liga;\n\n@late = [ a\n];\n",
            "feature kern {\npos a b 1;\n} kern;\n",
            "anon x {\n}\n} x;\n",
            "feature ss01 {\nfeatureNames {\n  name 1 0 0 \"mac\";\n};\n# automatic\nsub a by b;\n} ss01;\n",
        ] {
            let f = read(text);
            assert!(is_kept_text(&f), "{text:?}: {f:?}");
            assert_eq!(write(&f), text);
        }
    }

    #[test]
    fn plain_files_split_when_they_write_back_the_same() {
        let text = "languagesystem DFLT dflt;\n\nfeature liga {\n  sub f i by fi;\n} liga;\n";
        let f = read(text);
        assert_eq!(f.prefixes, [snippet(ANONYMOUS, "languagesystem DFLT dflt;")]);
        assert_eq!(f.features, [snippet("liga", "  sub f i by fi;")]);
        assert_eq!(write(&f), text);
    }

    #[test]
    fn an_edit_writes_the_new_code() {
        let mut f = read(GLYPHSLIB);
        f.features[1].code = "pos a b -20;".into();
        f.classes[0].name = "Fooo".into();
        let text = write(&f);
        assert!(text.contains("feature kern {\npos a b -20;\n# Automatic Code\n\n} kern;") && text.starts_with("@Fooo = [ a b\n];"), "{text}");
        assert_eq!(read(&text), f);
    }

    #[test]
    fn hostile_text_does_not_crash() {
        let deep = format!("feature x {{\n{}\n}} x;\n", "lookup a {".repeat(10_000));
        for text in [deep.as_str(), "# Prefix: \n", "#", "\"", "include(", "feature", "@", "@a =", "feature x {\n# notes:\n# a", "\u{feff}é\r\n"] {
            let _ = read(text);
            let f = read(text);
            assert_eq!(write(&f), text);
        }
        for i in 0..GLYPHSLIB.len() {
            if let Some(t) = GLYPHSLIB.get(..i) {
                assert_eq!(write(&read(t)), t);
            }
        }
    }

    #[test]
    fn statements_follow_blocks_strings_comments_and_includes() {
        let text = "languagesystem DFLT dflt; # one\n@A = [a b];\nfeature liga {\nsub f i by fi; # }\nlookup x { sub a by b; } x;\n} liga;\ninclude(x;y.fea)\ntable GDEF { } GDEF;\nname \"}\";\n";
        let kinds: Vec<Kind> = statements(text).unwrap().into_iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Kind::Other, Kind::Comment, Kind::Class, Kind::Feature, Kind::Other, Kind::Table, Kind::Other]);
        for bad in ["feature liga {", "}", "name \"x;", "anon x { } x;", &"{".repeat(MAX_DEPTH + 1)] {
            assert!(statements(bad).is_none(), "{bad:?}");
        }
    }
}

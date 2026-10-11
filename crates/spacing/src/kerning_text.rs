//! The kerning-text format: one master's kerning (and optionally its groups and metrics) as
//! tab-separated lines. The Kerning window copies pairs in it, and the metrics export writes it.
//! Row `kerning-text` in `spec/inventory/30-formats.toml`.
//!
//! Every line is a list of fields separated by tabs. The number of fields and the first field
//! decide what a line is:
//!
//! | Fields | Line | Meaning |
//! |---|---|---|
//! | 3 | `left	right	value` | a pair; a side that starts with `@` is a group, in command spelling (the group that faces the pair) |
//! | 2 | `kerning-text	1` | the header, optional, only as the first line |
//! | 2 | `master	name` | the master the lines were taken from, for information |
//! | 2 | `direction	ltr` | `ltr`, `rtl` or `vertical` |
//! | 4 or more | `group	side	name	member…` | members of the group `name` on `side` (`left`, `right`, `top`, `bottom`); several lines for one group add up |
//! | 8 | `metrics	glyph	lsb	rsb	width	leftKey	rightKey	widthKey` | a glyph's metrics; an empty field has no value |
//!
//! Blank lines are skipped and a line may end in `\r`. Names hold no control characters (the
//! model forbids them), so a tab never occurs inside a field. Values are decimal numbers.
//!
//! Untrusted input: the text, each line and the number of pairs, group members and metrics lines
//! are capped ([`MAX_TEXT_BYTES`], [`MAX_LINE_BYTES`], [`MAX_PAIRS`], [`MAX_GROUP_MEMBERS`],
//! [`MAX_METRICS_LINES`]); values must be finite and in range. An error names the line (from 1)
//! and the byte offset of the field in the text (from 0). [`write`] refuses what [`read`] would
//! refuse, so what it writes always reads back.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use fontcraft_model::{Font, MAX_GLYPHS, MAX_NAME_LEN};

use crate::kerning::pairs::{NewPair, stored_pairs};
use crate::kerning::{Direction, GroupSide, Position, Side, facing, group_list};
use crate::{MAX_KEY_LEN, SIDEBEARING_RANGE, WIDTH_RANGE};

/// Largest kerning text, in bytes.
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
/// Longest line, in bytes, without the line break.
pub const MAX_LINE_BYTES: usize = 4096;
/// Most pairs one text may hold (also the most one paste or import takes).
pub const MAX_PAIRS: usize = 500_000;
/// Most group members over all group lines: each glyph is in at most one group per side.
pub const MAX_GROUP_MEMBERS: usize = 4 * MAX_GLYPHS;
/// Most metrics lines: one per glyph.
pub const MAX_METRICS_LINES: usize = MAX_GLYPHS;
/// The values a kerning pair may have (the 16-bit range the compiled font stores).
pub const VALUE_RANGE: RangeInclusive<f64> = -32_768.0..=32_767.0;
/// The first field of the header line.
pub const HEADER: &str = "kerning-text";
/// The format version this code writes and reads.
pub const VERSION: &str = "1";

/// A parsed kerning text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KerningText {
    pub master: Option<String>,
    pub direction: Option<Direction>,
    /// One entry per group, in the order the groups first appear.
    pub groups: Vec<GroupLine>,
    pub metrics: Vec<MetricsLine>,
    /// In text order.
    pub pairs: Vec<NewPair>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupLine {
    pub side: GroupSide,
    pub name: String,
    pub members: Vec<String>,
}

/// One glyph's metrics in a master; the keys in the form a metrics field shows them (`=H`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricsLine {
    pub glyph: String,
    pub lsb: Option<f64>,
    pub rsb: Option<f64>,
    pub width: Option<f64>,
    pub left_key: Option<String>,
    pub right_key: Option<String>,
    pub width_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ReadError {
    #[error("the kerning text is {0} bytes long; the limit is {MAX_TEXT_BYTES}")]
    TooLarge(usize),
    #[error("line {line} is {len} bytes long; the limit is {MAX_LINE_BYTES}")]
    LineTooLong { line: usize, len: usize },
    #[error("line {line}, byte {byte}: {message}")]
    Bad { line: usize, byte: usize, message: String },
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum WriteError {
    #[error("{what} `{name}` cannot be written: {problem}")]
    Name { what: &'static str, name: String, problem: String },
    #[error("the pair {left} {right} has the value {value}; kerning text takes finite values from -32768 to 32767")]
    Value { left: String, right: String, value: f64 },
    #[error("the metrics of `{glyph}` hold {value}, which is not a finite number in range")]
    Metric { glyph: String, value: f64 },
    #[error("there are {0} pairs; kerning text holds at most {MAX_PAIRS}")]
    TooManyPairs(usize),
    #[error("{0} would make a line longer than {MAX_LINE_BYTES} bytes")]
    LineTooLong(String),
    #[error("the kerning text would be longer than {MAX_TEXT_BYTES} bytes")]
    TooLarge,
}

/// `master`'s own pairs in `direction`, in stored order, with the master's name, the direction
/// and every group on the two sides that face the pairs of that direction. No metrics lines.
pub fn from_master(font: &Font, direction: Direction, master: &str) -> KerningText {
    let pairs = stored_pairs(font, direction, master).iter().map(|p| NewPair { left: p.left(), right: p.right(), value: p.value }).collect();
    let mut groups = Vec::new();
    for side in [facing(direction, Position::First), facing(direction, Position::Second)] {
        for (name, members) in group_list(font, side) {
            groups.push(GroupLine { side, name: name.to_string(), members: members.into_iter().map(str::to_string).collect() });
        }
    }
    KerningText {
        master: font.masters.iter().find(|m| m.id == master).map(|m| m.name.clone()),
        direction: Some(direction),
        groups,
        metrics: Vec::new(),
        pairs,
    }
}

// ---------- writing ----------

/// Writes `doc`: the header, the master and direction lines, group lines, metrics lines, then the
/// pairs, each line ending in `\n`.
pub fn write(doc: &KerningText) -> Result<String, WriteError> {
    if doc.pairs.len() > MAX_PAIRS {
        return Err(WriteError::TooManyPairs(doc.pairs.len()));
    }
    let mut out = String::new();
    let line = |out: &mut String, text: &str, what: &dyn Fn() -> String| -> Result<(), WriteError> {
        if text.len() > MAX_LINE_BYTES {
            return Err(WriteError::LineTooLong(what()));
        }
        if out.len() + text.len() + 1 > MAX_TEXT_BYTES {
            return Err(WriteError::TooLarge);
        }
        out.push_str(text);
        out.push('\n');
        Ok(())
    };
    line(&mut out, &format!("{HEADER}\t{VERSION}"), &|| "the header".into())?;
    if let Some(m) = &doc.master {
        check_name("master", m)?;
        line(&mut out, &format!("master\t{m}"), &|| format!("master `{m}`"))?;
    }
    if let Some(d) = doc.direction {
        line(&mut out, &format!("direction\t{}", d.name()), &|| "the direction".into())?;
    }
    for g in &doc.groups {
        if Side::parse(&format!("@{}", g.name)).is_err() {
            return Err(WriteError::Name { what: "group", name: g.name.clone(), problem: "not a valid group name".into() });
        }
        let prefix = format!("group\t{}\t{}", g.side.name(), g.name);
        let mut text = prefix.clone();
        let mut count = 0;
        for m in &g.members {
            check_name("glyph", m)?;
            if count > 0 && text.len() + 1 + m.len() > MAX_LINE_BYTES {
                line(&mut out, &text, &|| format!("group `{}`", g.name))?;
                text.clone_from(&prefix);
                count = 0;
            }
            text.push('\t');
            text.push_str(m);
            count += 1;
        }
        if count > 0 {
            line(&mut out, &text, &|| format!("group `{}`", g.name))?;
        }
    }
    for m in &doc.metrics {
        check_name("glyph", &m.glyph)?;
        let mut fields = vec![m.glyph.clone()];
        for (v, range) in [(m.lsb, &SIDEBEARING_RANGE), (m.rsb, &SIDEBEARING_RANGE), (m.width, &WIDTH_RANGE)] {
            match v {
                Some(v) if !(v.is_finite() && range.contains(&v)) => return Err(WriteError::Metric { glyph: m.glyph.clone(), value: v }),
                Some(v) => fields.push(number(v)),
                None => fields.push(String::new()),
            }
        }
        for k in [&m.left_key, &m.right_key, &m.width_key] {
            let k = k.as_deref().unwrap_or_default();
            if k.len() > MAX_KEY_LEN || k.chars().any(char::is_control) {
                return Err(WriteError::Name {
                    what: "metrics key",
                    name: k.chars().take(40).collect(),
                    problem: "too long or holds a control character".into(),
                });
            }
            fields.push(k.to_string());
        }
        line(&mut out, &format!("metrics\t{}", fields.join("\t")), &|| format!("the metrics of `{}`", m.glyph))?;
    }
    for p in &doc.pairs {
        let (left, right) = (p.left.command(), p.right.command());
        for (side, text) in [(&p.left, &left), (&p.right, &right)] {
            if Side::parse(text).ok().as_ref() != Some(side) {
                let what = if side.is_group() { "group" } else { "glyph" };
                return Err(WriteError::Name { what, name: side.name().to_string(), problem: "it does not read back as the same side".into() });
            }
        }
        if !(p.value.is_finite() && VALUE_RANGE.contains(&p.value)) {
            return Err(WriteError::Value { left, right, value: p.value });
        }
        line(&mut out, &format!("{left}\t{right}\t{}", number(p.value)), &|| format!("the pair {left} {right}"))?;
    }
    Ok(out)
}

fn check_name(what: &'static str, name: &str) -> Result<(), WriteError> {
    let problem = if name.is_empty() {
        "it is empty"
    } else if name.len() > MAX_NAME_LEN {
        "it is longer than 255 bytes"
    } else if name.chars().any(char::is_control) {
        "it holds a control character"
    } else {
        return Ok(());
    };
    Err(WriteError::Name { what, name: name.chars().take(40).collect(), problem: problem.into() })
}

/// The shortest decimal text that reads back as `v` (`-40`, `12.5`).
fn number(v: f64) -> String {
    format!("{v}")
}

// ---------- reading ----------

/// Reads a kerning text.
pub fn read(text: &str) -> Result<KerningText, ReadError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ReadError::TooLarge(text.len()));
    }
    let mut doc = KerningText::default();
    let mut group_at: HashMap<(GroupSide, String), usize> = HashMap::new();
    let mut members = 0usize;
    let mut offset = 0usize;
    let mut first_line = true;
    for (i, raw) in text.split('\n').enumerate() {
        let line_no = i + 1;
        let start = offset;
        offset += raw.len() + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.len() > MAX_LINE_BYTES {
            return Err(ReadError::LineTooLong { line: line_no, len: line.len() });
        }
        if line.trim().is_empty() {
            continue;
        }
        let was_first = std::mem::replace(&mut first_line, false);
        let mut fields: Vec<(usize, &str)> = Vec::new();
        let mut at = start;
        for f in line.split('\t') {
            fields.push((at, f));
            at += f.len() + 1;
        }
        let bad = |byte: usize, message: String| ReadError::Bad { line: line_no, byte, message };
        match fields.as_slice() {
            [(lb, l), (rb, r), (vb, v)] => {
                if doc.pairs.len() >= MAX_PAIRS {
                    return Err(bad(start, format!("more than {MAX_PAIRS} pairs")));
                }
                let left = Side::parse(l).map_err(|e| bad(*lb, format!("left side: {e}")))?;
                let right = Side::parse(r).map_err(|e| bad(*rb, format!("right side: {e}")))?;
                let value = read_number(v, &VALUE_RANGE).map_err(|m| bad(*vb, m))?;
                doc.pairs.push(NewPair { left, right, value });
            }
            [(kb, k), (vb, v)] => match *k {
                HEADER if was_first => {
                    if *v != VERSION {
                        return Err(bad(*vb, format!("version `{}` is not supported; this program reads version {VERSION}", clip(v))));
                    }
                }
                HEADER => return Err(bad(*kb, "the header may only be the first line".into())),
                "master" => {
                    if doc.master.is_some() {
                        return Err(bad(*kb, "a second master line".into()));
                    }
                    read_name(v).map_err(|m| bad(*vb, format!("master: {m}")))?;
                    doc.master = Some(v.to_string());
                }
                "direction" => {
                    if doc.direction.is_some() {
                        return Err(bad(*kb, "a second direction line".into()));
                    }
                    let d = Direction::from_name(v).ok_or_else(|| bad(*vb, format!("direction `{}` is not ltr, rtl or vertical", clip(v))))?;
                    doc.direction = Some(d);
                }
                other => return Err(bad(*kb, format!("`{}` is not a line kind this format knows (master, direction)", clip(other)))),
            },
            [(_, "group"), (sb, side), (nb, name), rest @ ..] if !rest.is_empty() => {
                let side =
                    GroupSide::from_name(side).ok_or_else(|| bad(*sb, format!("group side `{}` is not left, right, top or bottom", clip(side))))?;
                if let Err(e) = Side::parse(&format!("@{name}")) {
                    return Err(bad(*nb, e.to_string()));
                }
                members += rest.len();
                if members > MAX_GROUP_MEMBERS {
                    return Err(bad(start, format!("more than {MAX_GROUP_MEMBERS} group members")));
                }
                for (mb, m) in rest {
                    read_name(m).map_err(|msg| bad(*mb, format!("group member: {msg}")))?;
                }
                let key = (side, name.to_string());
                let gi = match group_at.get(&key) {
                    Some(gi) => *gi,
                    None => {
                        doc.groups.push(GroupLine { side, name: name.to_string(), members: Vec::new() });
                        group_at.insert(key, doc.groups.len() - 1);
                        doc.groups.len() - 1
                    }
                };
                if let Some(g) = doc.groups.get_mut(gi) {
                    g.members.extend(rest.iter().map(|(_, m)| m.to_string()));
                }
            }
            [(_, "metrics"), (gb, glyph), (lb, lsb), (rb, rsb), (wb, width), (_, lk), (_, rk), (_, wk)] => {
                if doc.metrics.len() >= MAX_METRICS_LINES {
                    return Err(bad(start, format!("more than {MAX_METRICS_LINES} metrics lines")));
                }
                read_name(glyph).map_err(|m| bad(*gb, format!("glyph: {m}")))?;
                let opt = |b: usize, v: &str, range: &RangeInclusive<f64>| -> Result<Option<f64>, ReadError> {
                    if v.trim().is_empty() { Ok(None) } else { read_number(v, range).map(Some).map_err(|m| bad(b, m)) }
                };
                let key = |k: &str| -> Option<String> { (!k.is_empty()).then(|| k.to_string()) };
                for (b, k) in fields.iter().skip(5) {
                    if k.len() > MAX_KEY_LEN {
                        return Err(bad(*b, format!("a metrics key may be at most {MAX_KEY_LEN} bytes long")));
                    }
                }
                doc.metrics.push(MetricsLine {
                    glyph: glyph.to_string(),
                    lsb: opt(*lb, lsb, &SIDEBEARING_RANGE)?,
                    rsb: opt(*rb, rsb, &SIDEBEARING_RANGE)?,
                    width: opt(*wb, width, &WIDTH_RANGE)?,
                    left_key: key(*lk),
                    right_key: key(*rk),
                    width_key: key(*wk),
                });
            }
            [(kb, "metrics"), ..] => return Err(bad(*kb, format!("a metrics line has 8 fields, this one has {}", fields.len()))),
            [(kb, "group"), ..] => return Err(bad(*kb, "a group line needs a side, a name and at least one member".into())),
            _ => return Err(bad(start, format!("a line with {} fields is not a pair (left, right, value)", fields.len()))),
        }
    }
    Ok(doc)
}

fn read_number(text: &str, range: &RangeInclusive<f64>) -> Result<f64, String> {
    let t = text.trim();
    let ok = !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'));
    let v = if ok { t.parse::<f64>().ok() } else { None };
    match v {
        Some(v) if v.is_finite() && range.contains(&v) => Ok(v),
        Some(v) if v.is_finite() => Err(format!("{v} is outside {} to {}", range.start(), range.end())),
        Some(_) => Err(format!("`{}` is too large", clip(t))),
        None => Err(format!("`{}` is not a number", clip(t))),
    }
}

fn read_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        Err("the name is empty".into())
    } else if name.len() > MAX_NAME_LEN {
        Err(format!("the name is longer than {MAX_NAME_LEN} bytes"))
    } else if name.chars().any(char::is_control) {
        Err("the name holds a control character".into())
    } else {
        Ok(())
    }
}

/// At most 40 characters of untrusted text, for a message.
fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(40).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn pair(left: &str, right: &str, value: f64) -> NewPair {
        NewPair { left: Side::parse(left).unwrap(), right: Side::parse(right).unwrap(), value }
    }

    #[test]
    fn every_line_kind_reads_back_and_long_groups_split_across_lines() {
        let members: Vec<String> = (0..800).map(|i| format!("glyph{i:05}")).collect();
        let doc = KerningText {
            master: Some("Regular".into()),
            direction: Some(Direction::Rtl),
            groups: vec![
                GroupLine { side: GroupSide::Left, name: "big".into(), members: members.clone() },
                GroupLine { side: GroupSide::Right, name: "big".into(), members: vec!["a".into()] },
            ],
            metrics: vec![
                MetricsLine {
                    glyph: "a".into(),
                    lsb: Some(12.5),
                    rsb: Some(-3.0),
                    width: Some(500.0),
                    left_key: Some("=n".into()),
                    ..Default::default()
                },
                MetricsLine { glyph: "space".into(), width: Some(250.0), width_key: Some("==n*0.5".into()), ..Default::default() },
            ],
            pairs: vec![pair("@big", "a", -40.0), pair("a", "@big", 0.25), pair("b", "c", -0.0)],
        };
        let text = write(&doc).unwrap();
        assert!(text.lines().all(|l| l.len() <= MAX_LINE_BYTES));
        assert!(text.lines().filter(|l| l.starts_with("group\tleft\tbig\t")).count() > 1, "a long group takes several lines");
        assert!(text.contains("\nmetrics\tspace\t\t\t250\t\t\t==n*0.5\n"), "{text}");
        assert!(text.ends_with("@big\ta\t-40\na\t@big\t0.25\nb\tc\t-0\n"), "{text}");
        assert_eq!(read(&text).unwrap(), doc);
        assert_eq!(read(&text.replace('\n', "\r\n")).unwrap(), doc, "CRLF line ends");
    }

    #[test]
    fn write_refuses_what_read_would_refuse() {
        for bad in [f64::NAN, f64::INFINITY, 40_000.0] {
            let doc = KerningText { pairs: vec![pair("a", "b", bad)], ..Default::default() };
            assert!(matches!(write(&doc), Err(WriteError::Value { .. })), "{bad}");
        }
        let doc = KerningText {
            pairs: vec![NewPair { left: Side::Group("has space".into()), right: Side::Glyph("b".into()), value: 1.0 }],
            ..Default::default()
        };
        assert!(write(&doc).is_err());
        let doc =
            KerningText { pairs: vec![NewPair { left: Side::Glyph("@x".into()), right: Side::Glyph("b".into()), value: 1.0 }], ..Default::default() };
        assert!(write(&doc).is_err(), "a glyph named like a group would read back as a group");
        let doc =
            KerningText { groups: vec![GroupLine { side: GroupSide::Left, name: "g".into(), members: vec!["a\tb".into()] }], ..Default::default() };
        assert!(write(&doc).is_err());
    }

    #[test]
    fn hostile_text_fails_with_a_message() {
        assert!(matches!(read(&"a\tb\t1\n".repeat(MAX_TEXT_BYTES / 6 + 1)), Err(ReadError::TooLarge(_))));
        assert!(matches!(read(&"x".repeat(MAX_LINE_BYTES + 1)), Err(ReadError::LineTooLong { line: 1, .. })));
        let members = vec!["a"; 5000].join("\t");
        let many = format!("group\tleft\tg\t{}\n", &members[..3999]).repeat(MAX_GROUP_MEMBERS / 2000 + 2);
        assert!(read(&many).unwrap_err().to_string().contains("group members"));
        assert!(read("metrics\ta\tx\t\t\t\t\t").unwrap_err().to_string().contains("line 1, byte 10"));
        assert!(read(&format!("metrics\ta\t\t\t\t{}\t\t", "k".repeat(300))).unwrap_err().to_string().contains("metrics key"));
        assert!(read("master\ta\nmaster\tb").is_err());
        assert_eq!(read("").unwrap(), KerningText::default());
    }
}

//! Audit reports (M12.60): the result document as plain text, Markdown or HTML
//! (`audit.export`), the four family reports of the comparison tool (`audit.report.menuNames`,
//! `.metrics`, `.panose`, `.featureLanguages`) and the list of an external validator's messages
//! known to be wrong for well-built fonts (`audit.validatorKnownIssues`).
//!
//! The family reports follow AFDKO's comparefamily (Apache-2.0, Copyright 2015 Adobe Systems
//! Incorporated): which name records it reads, how faces are ordered and grouped into
//! typographic families, which values collapse to one line when every face agrees, how each list
//! is sorted, and the PANOSE value names. Its menu name report is the one its code prints for
//! `-rn`, its metrics report the one for `-rm` (its help text swaps the two). Where it reads a
//! record or a table incorrectly the reports here read it as the OpenType specification says;
//! each such difference is listed on the rows in `spec/inventory/21-qa-checks.toml`.
//!
//! Everything here reads bytes it is given and never fails on them: an unreadable table leaves
//! its values out, and the layout walk stops at [`MAX_LAYOUT_ENTRIES`] and
//! [`MAX_LOOKUP_REFS`].

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use read_fonts::tables::layout::{FeatureList, ScriptList};
use read_fonts::types::Tag;
use read_fonts::{FileRef, FontRef, TableProvider};
use serde::Serialize;

use crate::Severity;

/// Most results one exported report renders.
pub const MAX_EXPORT_RESULTS: usize = 100_000;
/// Most (script, language, feature) entries read from one GSUB or GPOS table.
pub const MAX_LAYOUT_ENTRIES: usize = 100_000;
/// Most lookup indices read from one GSUB or GPOS table, over all its features.
pub const MAX_LOOKUP_REFS: usize = 1_000_000;

// ── The result document as text ─────────────────────────────────────────────────────────────────

/// The file formats of `audit.export`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Text,
    Markdown,
    Html,
}

impl ExportFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "json" => Some(Self::Json),
            "text" => Some(Self::Text),
            "markdown" => Some(Self::Markdown),
            "html" => Some(Self::Html),
            _ => None,
        }
    }

    /// The format a file name asks for by its extension; `None` for any other extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "txt" | "log" => Some(Self::Text),
            "md" | "markdown" => Some(Self::Markdown),
            "html" | "htm" => Some(Self::Html),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Text => "text",
            Self::Markdown => "markdown",
            Self::Html => "html",
        }
    }
}

/// A font of the result document's target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportFont {
    pub id: String,
    pub name: String,
}

/// The counts of the result document's `summary`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportSummary {
    pub fail: usize,
    pub warn: usize,
    pub info: usize,
    pub passed: usize,
    pub skipped: usize,
    pub ignored: usize,
    pub errors: usize,
    pub stale: usize,
}

/// One entry of the result document's `checks`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckRow {
    pub id: String,
    /// `pass`, `fail`, `warn`, `info`, `skipped` or `error`.
    pub status: String,
    pub results: usize,
    pub reason: Option<String>,
}

/// One entry of the result document's `results`, with the location keys a report shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    pub id: String,
    pub check: String,
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub font: Option<String>,
    pub glyph: String,
    pub master: Option<String>,
    pub layer: String,
    pub table: Option<String>,
    pub field: Option<String>,
    pub fonts: Vec<String>,
    pub stale: bool,
}

impl ResultRow {
    /// Where the result is, for people: `glyph/master`, `table.field`, or the fonts involved.
    pub fn place(&self) -> String {
        if !self.glyph.is_empty() {
            let master = self.master.as_deref().filter(|m| !m.is_empty()).or_else(|| Some(self.layer.as_str()).filter(|l| !l.is_empty()));
            return match master {
                Some(m) => format!("{}/{m}", self.glyph),
                None => self.glyph.clone(),
            };
        }
        match (&self.table, &self.field) {
            (Some(t), Some(f)) => format!("{t}.{f}"),
            (Some(t), None) => t.clone(),
            _ if !self.fonts.is_empty() => self.fonts.join(", "),
            _ => "font".into(),
        }
    }
}

/// The parts of an audit result document (`docs/spec/21-qa.md` § 4) that the text, Markdown and
/// HTML reports show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportDoc {
    pub tool_version: String,
    pub profile: String,
    pub target_kind: String,
    pub target_path: Option<String>,
    pub fonts: Vec<ReportFont>,
    pub summary: ReportSummary,
    pub checks: Vec<CheckRow>,
    pub results: Vec<ResultRow>,
}

impl ReportDoc {
    /// The font name a result line shows: its own font, else the target's only font.
    fn font_label<'a>(&'a self, r: &'a ResultRow) -> Option<&'a str> {
        match &r.font {
            Some(id) => Some(self.fonts.iter().find(|f| &f.id == id).map_or(id.as_str(), |f| f.name.as_str())),
            None if self.fonts.len() == 1 => self.fonts.first().map(|f| f.name.as_str()),
            None => None,
        }
        .filter(|n| !n.is_empty())
    }

    fn target_label(&self) -> String {
        if let Some(p) = &self.target_path {
            return p.clone();
        }
        let names: Vec<&str> = self.fonts.iter().map(|f| f.name.as_str()).filter(|n| !n.is_empty()).collect();
        if names.is_empty() { "untitled".into() } else { names.join(", ") }
    }

    /// The results grouped by check, in the order the checks first appear.
    fn by_check(&self) -> Vec<(&str, Vec<&ResultRow>)> {
        let mut groups: Vec<(&str, Vec<&ResultRow>)> = Vec::new();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for r in &self.results {
            match index.get(r.check.as_str()).and_then(|&i| groups.get_mut(i)) {
                Some(g) => g.1.push(r),
                None => {
                    index.insert(r.check.as_str(), groups.len());
                    groups.push((r.check.as_str(), vec![r]));
                }
            }
        }
        groups
    }

    fn summary_line(&self) -> String {
        let s = &self.summary;
        let mut line = format!("{} failed, {} warnings, {} notes; {} checks passed, {} skipped", s.fail, s.warn, s.info, s.passed, s.skipped);
        if s.errors > 0 {
            let _ = write!(line, ", {} failed to run", s.errors);
        }
        if s.stale > 0 {
            let _ = write!(line, "; {} results out of date", s.stale);
        }
        line
    }

    fn skipped_checks(&self) -> impl Iterator<Item = &CheckRow> {
        self.checks.iter().filter(|c| c.status == "skipped" || c.status == "error")
    }
}

fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Fail => "FAIL",
        Severity::Warn => "WARN",
        Severity::Info => "INFO",
    }
}

fn check_title(id: &str) -> (&str, &str) {
    match crate::find(id) {
        Some(c) => (c.label, c.description),
        None => (id, ""),
    }
}

/// Renders `doc` as text, Markdown or HTML. JSON is the result document itself, written by the
/// caller; asked for here it gives the text report.
pub fn render(doc: &ReportDoc, format: ExportFormat) -> String {
    match format {
        ExportFormat::Markdown => render_markdown(doc),
        ExportFormat::Html => render_html(doc),
        ExportFormat::Text | ExportFormat::Json => render_text(doc),
    }
}

/// One line per result, `SEVERITY check-id [font] place: message`, then a summary line.
pub fn render_text(doc: &ReportDoc) -> String {
    let mut out = String::new();
    for r in &doc.results {
        let _ = write!(out, "{} {}", severity_word(r.severity), r.check);
        if let Some(font) = doc.font_label(r) {
            let _ = write!(out, " [{}]", one_line(font));
        }
        let _ = write!(out, " {}: {}", one_line(&r.place()), one_line(&r.message));
        if r.stale {
            out.push_str(" (out of date)");
        }
        out.push('\n');
    }
    out.push_str(&doc.summary_line());
    out.push('\n');
    out
}

/// Line breaks inside a value would split one result over several lines.
fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

fn md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '#' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '\r' | '\n' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

/// A Markdown report: target and summary, then one section per check with its description and
/// its results, then the checks that were skipped.
pub fn render_markdown(doc: &ReportDoc) -> String {
    let mut out = String::from("# Audit report\n\n");
    let _ = writeln!(out, "- Target: {}", md(&doc.target_label()));
    if !doc.profile.is_empty() {
        let _ = writeln!(out, "- Profile: {}", md(&doc.profile));
    }
    let _ = writeln!(out, "- Tool: fontcraft {}", md(&doc.tool_version));
    let s = &doc.summary;
    out.push_str("\n| Fail | Warn | Info | Passed | Skipped |\n|---:|---:|---:|---:|---:|\n");
    let _ = writeln!(out, "| {} | {} | {} | {} | {} |", s.fail, s.warn, s.info, s.passed, s.skipped);
    for (check, rows) in doc.by_check() {
        let (label, description) = check_title(check);
        let _ = write!(out, "\n## {}", md(label));
        if label != check {
            let _ = write!(out, " ({})", md(check));
        }
        out.push_str("\n\n");
        if !description.is_empty() {
            let _ = writeln!(out, "{}\n", md(description));
        }
        for r in rows {
            let _ = write!(out, "- **{}** {}", severity_word(r.severity), md(&r.place()));
            if let Some(font) = doc.font_label(r).filter(|_| doc.fonts.len() > 1) {
                let _ = write!(out, " ({})", md(font));
            }
            let _ = write!(out, ": {}", md(&r.message));
            if r.stale {
                out.push_str(" *(out of date)*");
            }
            out.push('\n');
        }
    }
    let skipped: Vec<&CheckRow> = doc.skipped_checks().collect();
    if !skipped.is_empty() {
        out.push_str("\n## Checks that did not run\n\n");
        for c in skipped {
            let _ = writeln!(out, "- {}: {}", md(&c.id), md(c.reason.as_deref().unwrap_or(&c.status)));
        }
    }
    let _ = writeln!(out, "\n{}", md(&doc.summary_line()));
    out
}

fn html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const HTML_STYLE: &str = "body{font-family:system-ui,sans-serif;margin:2em auto;max-width:60em;padding:0 1em;line-height:1.45;color:#1b1b1b;background:#fff}\
table{border-collapse:collapse}td,th{border:1px solid #ccc;padding:.25em .75em;text-align:right}\
h2{margin-top:1.6em;font-size:1.15em}code{font-size:.9em}ul{padding-left:1.2em}\
.sev{display:inline-block;min-width:3.2em;font-weight:600}.fail{color:#b00020}.warn{color:#8a5a00}.info{color:#1f5fa8}\
.stale{color:#777;font-style:italic}.desc{color:#444}\
@media (prefers-color-scheme:dark){body{color:#e6e6e6;background:#161616}td,th{border-color:#444}.desc{color:#bbb}.fail{color:#ff7b8a}.warn{color:#e8b64c}.info{color:#79b0ff}}";

/// A self-contained HTML page with the same sections as the Markdown report.
pub fn render_html(doc: &ReportDoc) -> String {
    let mut out = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    );
    let _ = writeln!(out, "<title>Audit report: {}</title>", html(&doc.target_label()));
    let _ = writeln!(out, "<style>{HTML_STYLE}</style>\n</head>\n<body>\n<h1>Audit report</h1>");
    let _ = write!(out, "<p>Target: <code>{}</code>", html(&doc.target_label()));
    if !doc.profile.is_empty() {
        let _ = write!(out, "<br>Profile: {}", html(&doc.profile));
    }
    let _ = writeln!(out, "<br>Tool: fontcraft {}</p>", html(&doc.tool_version));
    let s = &doc.summary;
    let _ = writeln!(
        out,
        "<table><tr><th>Fail</th><th>Warn</th><th>Info</th><th>Passed</th><th>Skipped</th></tr><tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr></table>",
        s.fail, s.warn, s.info, s.passed, s.skipped
    );
    for (check, rows) in doc.by_check() {
        let (label, description) = check_title(check);
        let _ = writeln!(out, "<h2 id=\"{}\">{} <code>{}</code></h2>", html(check), html(label), html(check));
        if !description.is_empty() {
            let _ = writeln!(out, "<p class=\"desc\">{}</p>", html(description));
        }
        out.push_str("<ul>\n");
        for r in rows {
            let sev = r.severity.as_str();
            let _ = write!(out, "<li><span class=\"sev {sev}\">{}</span> <code>{}</code>", severity_word(r.severity), html(&r.place()));
            if let Some(font) = doc.font_label(r).filter(|_| doc.fonts.len() > 1) {
                let _ = write!(out, " ({})", html(font));
            }
            let _ = write!(out, ": {}", html(&r.message));
            if r.stale {
                out.push_str(" <span class=\"stale\">(out of date)</span>");
            }
            out.push_str("</li>\n");
        }
        out.push_str("</ul>\n");
    }
    let skipped: Vec<&CheckRow> = doc.skipped_checks().collect();
    if !skipped.is_empty() {
        out.push_str("<h2>Checks that did not run</h2>\n<ul>\n");
        for c in skipped {
            let _ = writeln!(out, "<li><code>{}</code>: {}</li>", html(&c.id), html(c.reason.as_deref().unwrap_or(&c.status)));
        }
        out.push_str("</ul>\n");
    }
    let _ = writeln!(out, "<p>{}</p>\n</body>\n</html>", html(&doc.summary_line()));
    out
}

// ── Faces for the family reports ────────────────────────────────────────────────────────────────

/// How many faces `bytes` holds: 1 for a font, the member count for a collection.
pub fn face_count(bytes: &[u8]) -> Result<u32, String> {
    match FileRef::new(bytes).map_err(|e| format!("cannot read the font: {e}"))? {
        FileRef::Font(_) => Ok(1),
        FileRef::Collection(c) => Ok(c.len()),
    }
}

/// The OS/2 values the reports show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Os2Info {
    pub version: u16,
    pub typo_ascender: i16,
    pub typo_descender: i16,
    pub typo_line_gap: i16,
    pub cap_height: Option<i16>,
    pub x_height: Option<i16>,
    pub weight_class: u16,
    pub width_class: u16,
    pub fs_type: u16,
    pub vendor_id: String,
    pub panose: [u8; 10],
}

/// The `post` values the reports show.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PostInfo {
    pub italic_angle: f64,
    pub underline_position: i16,
    pub underline_thickness: i16,
}

/// What the family reports read from one face.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceInfo {
    pub path: String,
    /// The Macintosh PostScript name when the font has Macintosh names, else the Windows one.
    pub postscript_name: String,
    /// Typographic family (ID 16, else 1) the faces are grouped by, Macintosh records first as
    /// for the PostScript name.
    pub family: String,
    /// Typographic subfamily (ID 17, else 2), from the same platform as `family`.
    pub subfamily: String,
    /// Windows English (3, 1, 0x409) records: IDs 1, 2, 4 and 6, and 16 and 17 with their
    /// fallbacks to 1 and 2.
    pub family_win: String,
    pub subfamily_win: String,
    pub full_name_win: String,
    pub postscript_name_win: String,
    pub typographic_family_win: String,
    pub typographic_subfamily_win: String,
    /// The Macintosh compatible full name (ID 18, else the full name).
    pub compatible_full_name: String,
    pub copyright: String,
    pub trademark: String,
    pub num_glyphs: u16,
    /// `head.fontRevision` rounded down to three decimals after adding 0.0005.
    pub revision: f64,
    /// `head` xMin, yMin, xMax, yMax.
    pub bbox: [i16; 4],
    /// `head.macStyle` bits 0 and 1.
    pub bold: bool,
    pub italic: bool,
    pub os2: Option<Os2Info>,
    pub post: Option<PostInfo>,
    /// Filled for the feature report only.
    pub layout: Option<LayoutInfo>,
}

/// Name IDs the reports read.
const NAME_IDS: [u16; 10] = [0, 1, 2, 4, 5, 6, 7, 16, 17, 18];

impl FaceInfo {
    /// Reads face `face` of `bytes` (an sfnt or a collection). `path` is how reports name the
    /// file. Fails only when the table directory, `head` or `maxp` cannot be read.
    pub fn read(path: &str, bytes: &[u8], face: u32, with_layout: bool) -> Result<Self, String> {
        let font = FontRef::from_index(bytes, face).map_err(|e| format!("cannot read the font: {e}"))?;
        let head = font.head().map_err(|e| format!("cannot read the head table: {e}"))?;
        let maxp = font.maxp().map_err(|e| format!("cannot read the maxp table: {e}"))?;
        let (mac, win, has_mac) = read_names(&font);
        let get = |m: &BTreeMap<u16, String>, id: u16| m.get(&id).cloned().unwrap_or_default();
        let family_win = get(&win, 1);
        let subfamily_win = get(&win, 2);
        let typographic_family_win = win.get(&16).cloned().unwrap_or_else(|| family_win.clone());
        let typographic_subfamily_win = win.get(&17).cloned().unwrap_or_else(|| subfamily_win.clone());
        let full_name_win = get(&win, 4);
        let mut info = FaceInfo {
            path: path.to_string(),
            postscript_name_win: get(&win, 6),
            family_win,
            subfamily_win,
            full_name_win,
            typographic_family_win,
            typographic_subfamily_win,
            num_glyphs: maxp.num_glyphs(),
            revision: ((head.font_revision().to_f64() + 0.0005) * 1000.0).trunc() / 1000.0,
            bbox: [head.x_min(), head.y_min(), head.x_max(), head.y_max()],
            bold: head.mac_style().bits() & 1 != 0,
            italic: head.mac_style().bits() & 2 != 0,
            ..FaceInfo::default()
        };
        if has_mac {
            let family = get(&mac, 1);
            let subfamily = get(&mac, 2);
            let full = get(&mac, 4);
            info.postscript_name = get(&mac, 6);
            info.family = mac.get(&16).cloned().unwrap_or(family);
            info.subfamily = mac.get(&17).cloned().unwrap_or(subfamily);
            info.compatible_full_name = mac.get(&18).cloned().unwrap_or(full);
            info.copyright = get(&mac, 0);
            info.trademark = get(&mac, 7);
        } else {
            info.postscript_name = info.postscript_name_win.clone();
            info.family = info.typographic_family_win.clone();
            info.subfamily = info.typographic_subfamily_win.clone();
            info.compatible_full_name = info.full_name_win.clone();
            info.copyright = get(&win, 0);
            info.trademark = get(&win, 7);
        }
        if let Ok(os2) = font.os2() {
            let mut panose = [0u8; 10];
            for (slot, v) in panose.iter_mut().zip(os2.panose_10()) {
                *slot = *v;
            }
            info.os2 = Some(Os2Info {
                version: os2.version(),
                typo_ascender: os2.s_typo_ascender(),
                typo_descender: os2.s_typo_descender(),
                typo_line_gap: os2.s_typo_line_gap(),
                cap_height: os2.s_cap_height(),
                x_height: os2.sx_height(),
                weight_class: os2.us_weight_class(),
                width_class: os2.us_width_class(),
                fs_type: os2.fs_type(),
                vendor_id: os2.ach_vend_id().to_be_bytes().iter().map(|&b| char::from(b)).collect(),
                panose,
            });
        }
        if let Ok(post) = font.post() {
            info.post = Some(PostInfo {
                italic_angle: post.italic_angle().to_f64(),
                underline_position: post.underline_position().to_i16(),
                underline_thickness: post.underline_thickness().to_i16(),
            });
        }
        if with_layout {
            info.layout = Some(LayoutInfo::read(&font));
        }
        Ok(info)
    }

    /// The sort key the comparison tool orders faces by after the Windows family name: italic
    /// in bit 0, bold in bit 1.
    fn style_key(&self) -> u8 {
        u8::from(self.italic) | (u8::from(self.bold) << 1)
    }
}

/// The Macintosh Roman English and Windows Unicode English records of [`NAME_IDS`], and whether
/// the font has any Macintosh record.
fn read_names(font: &FontRef<'_>) -> (BTreeMap<u16, String>, BTreeMap<u16, String>, bool) {
    let (mut mac, mut win, mut has_mac) = (BTreeMap::new(), BTreeMap::new(), false);
    let Ok(name) = font.name() else { return (mac, win, has_mac) };
    let data = name.string_data();
    for rec in name.name_record() {
        has_mac |= rec.platform_id() == 1;
        let id = rec.name_id().to_u16();
        if !NAME_IDS.contains(&id) {
            continue;
        }
        let target = match (rec.platform_id(), rec.encoding_id(), rec.language_id()) {
            (1, 0, 0) => &mut mac,
            (3, 1, 0x409) => &mut win,
            _ => continue,
        };
        if let Ok(s) = rec.string(data) {
            target.insert(id, s.chars().collect::<String>());
        }
    }
    (mac, win, has_mac)
}

/// The faces of one typographic family, in report order.
#[derive(Clone, Debug)]
pub struct FamilyGroup<'a> {
    pub family: String,
    pub faces: Vec<&'a FaceInfo>,
}

/// Orders the faces by Windows family name, then italic and bold, and groups them by typographic
/// family in the order the families first appear.
pub fn families(faces: &[FaceInfo]) -> Vec<FamilyGroup<'_>> {
    let mut order: Vec<&FaceInfo> = faces.iter().collect();
    order.sort_by(|a, b| a.family_win.cmp(&b.family_win).then(a.style_key().cmp(&b.style_key())));
    let mut groups: Vec<FamilyGroup<'_>> = Vec::new();
    for f in order {
        match groups.iter_mut().find(|g| g.family == f.family) {
            Some(g) => g.faces.push(f),
            None => groups.push(FamilyGroup { family: f.family.clone(), faces: vec![f] }),
        }
    }
    groups
}

// ── Menu name report ────────────────────────────────────────────────────────────────────────────

/// The menu names of one face.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuNameRow {
    pub path: String,
    pub post_script_name: String,
    /// `typographic family/typographic subfamily` (Windows records).
    pub preferred_menu: String,
    /// The Macintosh compatible full name.
    pub mac_compatible_menu: String,
    /// `family/subfamily` (Windows records, IDs 1 and 2).
    pub windows_compatible_menu: String,
    pub family_name: String,
    pub subfamily_name: String,
    pub full_name: String,
    pub typographic_family: String,
    pub typographic_subfamily: String,
    pub compatible_full_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuNameFamily {
    pub family: String,
    pub fonts: Vec<MenuNameRow>,
}

pub fn menu_name_report(faces: &[FaceInfo]) -> Vec<MenuNameFamily> {
    families(faces)
        .into_iter()
        .map(|g| MenuNameFamily {
            family: g.family,
            fonts: g
                .faces
                .into_iter()
                .map(|f| MenuNameRow {
                    path: f.path.clone(),
                    post_script_name: f.postscript_name.clone(),
                    preferred_menu: format!("{}/{}", f.typographic_family_win, f.typographic_subfamily_win),
                    mac_compatible_menu: f.compatible_full_name.clone(),
                    windows_compatible_menu: format!("{}/{}", f.family_win, f.subfamily_win),
                    family_name: f.family_win.clone(),
                    subfamily_name: f.subfamily_win.clone(),
                    full_name: f.full_name_win.clone(),
                    typographic_family: f.typographic_family_win.clone(),
                    typographic_subfamily: f.typographic_subfamily_win.clone(),
                    compatible_full_name: f.compatible_full_name.clone(),
                })
                .collect(),
        })
        .collect()
}

// ── Metrics report ──────────────────────────────────────────────────────────────────────────────

/// A value in a report.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ReportValue {
    Int(i64),
    Number(f64),
    Bool(bool),
    Text(String),
    Ints(Vec<i64>),
}

/// One face's value of a metrics field.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsCell {
    pub post_script_name: String,
    /// `None` when the face lacks the table.
    pub value: Option<ReportValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One field of the metrics report: one value when every face agrees, else a list.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsField {
    pub field: &'static str,
    /// Where the value comes from (`OS/2.usWeightClass`).
    pub source: &'static str,
    /// Every face has the same value.
    pub same: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ReportValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<MetricsCell>,
    /// Why the field is not reported (the first face lacks the table).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstFace {
    pub post_script_name: String,
    pub copyright: String,
    pub trademark: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsFamily {
    pub family: String,
    pub values: Vec<MetricsField>,
    pub first_face: FirstFace,
}

/// How the faces of a field that differs are listed.
#[derive(Clone, Copy)]
enum ListOrder {
    /// Family order.
    Family,
    /// By glyph count, then PostScript name.
    Glyphs,
    /// By the number, smallest first; equal values keep family order.
    Ascending,
    /// By the number, largest first; equal values keep family order.
    Descending,
    /// Faces whose flag is set first; otherwise family order.
    SetFirst,
}

type Getter = fn(&FaceInfo) -> Option<(ReportValue, Option<String>)>;

struct FieldSpec {
    field: &'static str,
    source: &'static str,
    table: &'static str,
    order: ListOrder,
    /// List every face even when they agree.
    always_list: bool,
    get: Getter,
}

fn sort_number(v: &Option<ReportValue>) -> f64 {
    match v {
        Some(ReportValue::Int(n)) => *n as f64,
        Some(ReportValue::Number(n)) => *n,
        Some(ReportValue::Bool(b)) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn metrics_field(spec: &FieldSpec, faces: &[&FaceInfo]) -> MetricsField {
    let cells: Vec<(&FaceInfo, Option<(ReportValue, Option<String>)>)> = faces.iter().map(|f| (*f, (spec.get)(f))).collect();
    let mut out = MetricsField { field: spec.field, source: spec.source, same: false, value: None, label: None, fonts: Vec::new(), skipped: None };
    let Some((_, first)) = cells.first() else { return out };
    let Some((first_value, first_label)) = first.clone() else {
        out.skipped = Some(format!("the first face has no {} table", spec.table));
        return out;
    };
    out.same = cells.iter().all(|(_, c)| c.as_ref().is_some_and(|(v, _)| *v == first_value));
    if out.same && !spec.always_list {
        out.value = Some(first_value);
        out.label = first_label;
        return out;
    }
    let mut rows: Vec<MetricsCell> = cells
        .into_iter()
        .map(|(f, c)| {
            let (value, label) = match c {
                Some((v, l)) => (Some(v), l),
                None => (None, None),
            };
            MetricsCell { post_script_name: f.postscript_name.clone(), value, label }
        })
        .collect();
    match spec.order {
        ListOrder::Family => {}
        ListOrder::Glyphs => rows.sort_by(|a, b| {
            sort_number(&a.value)
                .partial_cmp(&sort_number(&b.value))
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.post_script_name.cmp(&b.post_script_name))
        }),
        ListOrder::Ascending => rows.sort_by(|a, b| sort_number(&a.value).partial_cmp(&sort_number(&b.value)).unwrap_or(Ordering::Equal)),
        ListOrder::Descending | ListOrder::SetFirst => {
            rows.sort_by(|a, b| sort_number(&b.value).partial_cmp(&sort_number(&a.value)).unwrap_or(Ordering::Equal))
        }
    }
    out.fonts = rows;
    out
}

fn int(v: impl Into<i64>) -> ReportValue {
    ReportValue::Int(v.into())
}

/// The embedding permissions an `fsType` value grants, as the comparison tool names them.
pub fn fs_type_label(fs_type: u16) -> String {
    let mut parts = Vec::new();
    if fs_type == 0 {
        parts.push("Installable embedding");
    }
    for (bit, name) in [
        (0x0002, "restricted license embedding"),
        (0x0004, "print and preview"),
        (0x0008, "editable embedding"),
        (0x0100, "no subsetting"),
        (0x0200, "bitmap embedding only"),
    ] {
        if fs_type & bit != 0 {
            parts.push(name);
        }
    }
    parts.join(", ")
}

/// The name of a `usWeightClass` value, or the two names it lies between.
pub fn weight_class_label(w: u16) -> &'static str {
    const NAMES: [&str; 9] = [
        "Thin",
        "Extra-Light (Ultra-light)",
        "Light",
        "Normal (Regular)",
        "Medium",
        "Semi-bold (Demi-bold)",
        "Bold",
        "Extra-Bold (Ultra-bold)",
        "Black (Heavy)",
    ];
    const BETWEEN: [&str; 8] = [
        "Thin to Extra-Light (Ultra-light)",
        "Extra-Light (Ultra-light) to Light",
        "Light to Normal (Regular)",
        "Normal (Regular) to Medium",
        "Medium to Semi-bold (Demi-bold)",
        "Semi-bold (Demi-bold) to Bold",
        "Bold to Extra-bold (Ultra-bold)",
        "Extra-bold (Ultra-bold) to Black (Heavy)",
    ];
    if !(100..=900).contains(&w) {
        return "value out of range";
    }
    let step = usize::from(w / 100) - 1;
    if w % 100 == 0 { NAMES.get(step).copied() } else { BETWEEN.get(step).copied() }.unwrap_or("value out of range")
}

/// The name of a `usWidthClass` value.
pub fn width_class_label(w: u16) -> &'static str {
    const NAMES: [&str; 9] = [
        "Ultra-condensed",
        "Extra-condensed",
        "Condensed",
        "Semi-condensed",
        "Medium (normal)",
        "Semi-expanded",
        "Expanded",
        "Extra-expanded",
        "Ultra-expanded",
    ];
    usize::from(w).checked_sub(1).and_then(|i| NAMES.get(i)).copied().unwrap_or("value out of range")
}

const METRICS_FIELDS: [FieldSpec; 13] = [
    FieldSpec {
        field: "numGlyphs",
        source: "maxp.numGlyphs",
        table: "maxp",
        order: ListOrder::Glyphs,
        always_list: false,
        get: |f| Some((int(f.num_glyphs), None)),
    },
    FieldSpec {
        field: "typoMetrics",
        source: "OS/2.sTypoAscender/sTypoDescender/sTypoLineGap/sCapHeight/sxHeight",
        table: "OS/2",
        order: ListOrder::Family,
        always_list: false,
        get: |f| {
            let o = f.os2.as_ref()?;
            let v = [o.typo_ascender, o.typo_descender, o.typo_line_gap, o.cap_height.unwrap_or(-1), o.x_height.unwrap_or(-1)];
            Some((ReportValue::Ints(v.iter().map(|&n| i64::from(n)).collect()), None))
        },
    },
    FieldSpec {
        field: "italicAngle",
        source: "post.italicAngle",
        table: "post",
        order: ListOrder::Ascending,
        always_list: false,
        get: |f| Some((ReportValue::Number(f.post?.italic_angle), None)),
    },
    FieldSpec {
        field: "bold",
        source: "head.macStyle",
        table: "head",
        order: ListOrder::SetFirst,
        always_list: false,
        get: |f| Some((ReportValue::Bool(f.bold), Some(if f.bold { "Bold" } else { "Not Bold" }.into()))),
    },
    FieldSpec {
        field: "italic",
        source: "head.macStyle",
        table: "head",
        order: ListOrder::SetFirst,
        always_list: false,
        get: |f| Some((ReportValue::Bool(f.italic), Some(if f.italic { "Italic" } else { "Not Italic" }.into()))),
    },
    FieldSpec {
        field: "fontRevision",
        source: "head.fontRevision",
        table: "head",
        order: ListOrder::Descending,
        always_list: false,
        get: |f| Some((ReportValue::Number(f.revision), None)),
    },
    FieldSpec {
        field: "fsType",
        source: "OS/2.fsType",
        table: "OS/2",
        order: ListOrder::Family,
        always_list: false,
        get: |f| {
            let t = f.os2.as_ref()?.fs_type;
            Some((int(t), Some(fs_type_label(t))))
        },
    },
    FieldSpec {
        field: "fontBBox",
        source: "head.xMin/yMin/xMax/yMax",
        table: "head",
        order: ListOrder::Family,
        always_list: true,
        get: |f| Some((ReportValue::Ints(f.bbox.iter().map(|&n| i64::from(n)).collect()), None)),
    },
    FieldSpec {
        field: "usWeightClass",
        source: "OS/2.usWeightClass",
        table: "OS/2",
        order: ListOrder::Descending,
        always_list: false,
        get: |f| {
            let w = f.os2.as_ref()?.weight_class;
            Some((int(w), Some(weight_class_label(w).into())))
        },
    },
    FieldSpec {
        field: "usWidthClass",
        source: "OS/2.usWidthClass",
        table: "OS/2",
        order: ListOrder::Descending,
        always_list: false,
        get: |f| {
            let w = f.os2.as_ref()?.width_class;
            Some((int(w), Some(width_class_label(w).into())))
        },
    },
    FieldSpec {
        field: "underlinePosition",
        source: "post.underlinePosition",
        table: "post",
        order: ListOrder::Descending,
        always_list: false,
        get: |f| Some((int(f.post?.underline_position), None)),
    },
    FieldSpec {
        field: "underlineThickness",
        source: "post.underlineThickness",
        table: "post",
        order: ListOrder::Descending,
        always_list: false,
        get: |f| Some((int(f.post?.underline_thickness), None)),
    },
    FieldSpec {
        field: "vendorId",
        source: "OS/2.achVendID",
        table: "OS/2",
        order: ListOrder::Family,
        always_list: false,
        get: |f| Some((ReportValue::Text(f.os2.as_ref()?.vendor_id.clone()), None)),
    },
];

pub fn metrics_report(faces: &[FaceInfo]) -> Vec<MetricsFamily> {
    families(faces)
        .into_iter()
        .map(|g| {
            let first = g.faces.first();
            MetricsFamily {
                values: METRICS_FIELDS.iter().map(|spec| metrics_field(spec, &g.faces)).collect(),
                first_face: FirstFace {
                    post_script_name: first.map(|f| f.postscript_name.clone()).unwrap_or_default(),
                    copyright: first.map(|f| f.copyright.clone()).unwrap_or_default(),
                    trademark: first.map(|f| f.trademark.clone()).unwrap_or_default(),
                },
                family: g.family,
            }
        })
        .collect()
}

// ── PANOSE report ───────────────────────────────────────────────────────────────────────────────
//
// The digit names of the PANOSE classification, as comparefamily lists them (two misspellings
// corrected).

const PANOSE_KIND: &[&str] = &["Any", "No Fit", "Latin Text", "Latin Hand Written", "Latin Decorative", "Latin Symbol"];

struct PanoseClass {
    fields: [&'static str; 10],
    /// The names of digits 2 to 10; digit 1 is [`PANOSE_KIND`].
    values: [&'static [&'static str]; 9],
}

const WIDTHS: &[&str] = &["Any", "No Fit", "No Width", "Exceptionally Wide", "Super Wide", "Very Wide", "Wide", "Normal", "Narrow", "Very Narrow"];

const PANOSE_CLASSES: [PanoseClass; 4] = [
    PanoseClass {
        fields: [
            "Family Kind",
            "Serif Style",
            "Weight",
            "Proportion",
            "Contrast",
            "Stroke Variation",
            "Arm Style",
            "Letterform",
            "Midline",
            "X-height",
        ],
        values: [
            &[
                "Any",
                "No Fit",
                "Cove",
                "Obtuse Cove",
                "Square Cove",
                "Obtuse Square Cove",
                "Square",
                "Thin",
                "Oval",
                "Exaggerated",
                "Triangle",
                "Normal Sans",
                "Obtuse Sans",
                "Perpendicular Sans",
                "Flared",
                "Rounded",
            ],
            &["Any", "No Fit", "Very Light", "Light", "Thin", "Book", "Medium", "Demi", "Bold", "Heavy", "Black", "Extra Black"],
            &["Any", "No Fit", "Old Style", "Modern", "Even Width", "Extended", "Condensed", "Very Extended", "Very Condensed", "Monospaced"],
            &["Any", "No Fit", "None", "Very Low", "Low", "Medium Low", "Medium", "Medium High", "High", "Very High"],
            &[
                "Any",
                "No Fit",
                "No Variation",
                "Gradual/Diagonal",
                "Gradual/Transitional",
                "Gradual/Vertical",
                "Gradual/Horizontal",
                "Rapid/Vertical",
                "Rapid/Horizontal",
                "Instant/Vertical",
                "Instant/Horizontal",
            ],
            &[
                "Any",
                "No Fit",
                "Straight Arms/Horizontal",
                "Straight Arms/Wedge",
                "Straight Arms/Vertical",
                "Straight Arms/Single Serif",
                "Straight Arms/Double Serif",
                "Non-Straight/Horizontal",
                "Non-Straight/Wedge",
                "Non-Straight/Vertical",
                "Non-Straight/Single Serif",
                "Non-Straight/Double Serif",
            ],
            &[
                "Any",
                "No Fit",
                "Normal/Contact",
                "Normal/Weighted",
                "Normal/Boxed",
                "Normal/Flattened",
                "Normal/Rounded",
                "Normal/Off Center",
                "Normal/Square",
                "Oblique/Contact",
                "Oblique/Weighted",
                "Oblique/Boxed",
                "Oblique/Flattened",
                "Oblique/Rounded",
                "Oblique/Off Center",
                "Oblique/Square",
            ],
            &[
                "Any",
                "No Fit",
                "Standard/Trimmed",
                "Standard/Pointed",
                "Standard/Serifed",
                "High/Trimmed",
                "High/Pointed",
                "High/Serifed",
                "Constant/Trimmed",
                "Constant/Pointed",
                "Constant/Serifed",
                "Low/Trimmed",
                "Low/Pointed",
                "Low/Serifed",
            ],
            &["Any", "No Fit", "Constant/Small", "Constant/Standard", "Constant/Large", "Ducking/Small", "Ducking/Standard", "Ducking/Large"],
        ],
    },
    PanoseClass {
        fields: ["Family Kind", "Tool kind", "Weight", "Spacing", "Aspect Ratio", "Contrast", "Topology", "Form", "Finials", "X-Ascent"],
        values: [
            &[
                "Any",
                "No Fit",
                "Flat Nib",
                "Pressure Point",
                "Engraved",
                "Ball (Round Cap)",
                "Brush",
                "Rough",
                "Felt Pen/Brush Tip",
                "Wild Brush - Drips a lot",
            ],
            &["Any", "No Fit", "Very Light", "Light", "Thin", "Book", "Medium", "Demi", "Bold", "Heavy", "Black", "Extra Black (Nord)"],
            &["Any", "No Fit", "Proportional Spaced", "Monospaced"],
            &["Any", "No Fit", "Very Condensed", "Condensed", "Normal", "Expanded", "Very Expanded"],
            &["Any", "No Fit", "None", "Very Low", "Low", "Medium Low", "Medium", "Medium High", "High", "Very High"],
            &[
                "Any",
                "No Fit",
                "Roman Disconnected",
                "Roman Trailing",
                "Roman Connected",
                "Cursive Disconnected",
                "Cursive Trailing",
                "Cursive Connected",
                "Blackletter Disconnected",
                "Blackletter Trailing",
                "Blackletter Connected",
            ],
            &[
                "Any",
                "No Fit",
                "Upright / No Wrapping",
                "Upright / Some Wrapping",
                "Upright / More Wrapping",
                "Upright / Extreme Wrapping",
                "Oblique / No Wrapping",
                "Oblique / Some Wrapping",
                "Oblique / More Wrapping",
                "Oblique / Extreme Wrapping",
                "Exaggerated / No Wrapping",
                "Exaggerated / Some Wrapping",
                "Exaggerated / More Wrapping",
                "Exaggerated / Extreme Wrapping",
            ],
            &[
                "Any",
                "No Fit",
                "None / No loops",
                "None / Closed loops",
                "None / Open loops",
                "Sharp / No loops",
                "Sharp / Closed loops",
                "Sharp / Open loops",
                "Tapered / No loops",
                "Tapered / Closed loops",
                "Tapered / Open loops",
                "Round / No loops",
                "Round / Closed loops",
                "Round / Open loops",
            ],
            &["Any", "No Fit", "Very Low", "Low", "Medium", "High", "Very High"],
        ],
    },
    PanoseClass {
        fields: ["Family Kind", "Class", "Weight", "Aspect", "Contrast", "Serif Variant", "Treatment", "Lining", "Topology", "Range of Characters"],
        values: [
            &[
                "Any",
                "No Fit",
                "Derivative",
                "Non-standard Topology",
                "Non-standard Elements",
                "Non-standard Aspect",
                "Initials",
                "Cartoon",
                "Picture Stems",
                "Ornamented",
                "Text and Background",
                "Collage",
                "Montage",
            ],
            &["Any", "No Fit", "Very Light", "Light", "Thin", "Book", "Medium", "Demi", "Bold", "Heavy", "Black", "Extra Black"],
            &[
                "Any",
                "No Fit",
                "Super Condensed",
                "Very Condensed",
                "Condensed",
                "Normal",
                "Extended",
                "Very Extended",
                "Super Extended",
                "Monospaced",
            ],
            &[
                "Any",
                "No Fit",
                "None",
                "Very Low",
                "Low",
                "Medium Low",
                "Medium",
                "Medium High",
                "High",
                "Very High",
                "Horizontal Low",
                "Horizontal Medium",
                "Horizontal High",
                "Broken",
            ],
            &[
                "Any",
                "No Fit",
                "Cove",
                "Obtuse Cove",
                "Square Cove",
                "Obtuse Square Cove",
                "Square",
                "Thin",
                "Oval",
                "Exaggerated",
                "Triangle",
                "Normal Sans",
                "Obtuse Sans",
                "Perpendicular Sans",
                "Flared",
                "Rounded",
                "Script",
            ],
            &[
                "Any",
                "No Fit",
                "None - Standard Solid Fill",
                "White / No Fill",
                "Patterned Fill",
                "Complex Fill",
                "Shaped Fill",
                "Drawn / Distressed",
            ],
            &["Any", "No Fit", "None", "Inline", "Outline", "Engraved (Multiple Lines)", "Shadow", "Relief", "Backdrop"],
            &[
                "Any",
                "No Fit",
                "Standard",
                "Square",
                "Multiple Segment",
                "Deco (E,M,S) Waco midlines",
                "Uneven Weighting",
                "Diverse Arms",
                "Diverse Forms",
                "Lombardic Forms",
                "Upper Case in Lower Case",
                "Implied Topology",
                "Horseshoe E and A",
                "Cursive",
                "Blackletter",
                "Swash Variance",
            ],
            &["Any", "No Fit", "Extended Collection", "Literals", "No Lower Case", "Small Caps"],
        ],
    },
    PanoseClass {
        fields: [
            "Family Kind",
            "Kind",
            "Weight",
            "Spacing",
            "Aspect ratio & contrast",
            "Aspect ratio of character 94",
            "Aspect ratio of character 119",
            "Aspect ratio of character 157",
            "Aspect ratio of character 163",
            "Aspect ratio of character 211",
        ],
        values: [
            &[
                "Any",
                "No Fit",
                "Montages",
                "Pictures",
                "Shapes",
                "Scientific",
                "Music",
                "Expert",
                "Patterns",
                "Borders",
                "Icons",
                "Logos",
                "Industry specific",
            ],
            &["Any", "No Fit"],
            &["Any", "No Fit", "Proportional Spaced", "Monospaced"],
            &["Any", "No Fit"],
            WIDTHS,
            WIDTHS,
            WIDTHS,
            WIDTHS,
            WIDTHS,
        ],
    },
];

/// One PANOSE digit with its name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PanoseField {
    pub name: &'static str,
    pub value: u8,
    /// `None` when the value has no name in its class.
    pub label: Option<&'static str>,
    /// The digit differs from the base face's.
    pub differs: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanoseFace {
    pub post_script_name: String,
    /// Empty when the face has no OS/2 table.
    pub digits: Vec<u8>,
    /// The family kind's name (the first digit).
    pub kind: &'static str,
    /// The ten digits named; empty when the family kind is not 2 to 5.
    pub fields: Vec<PanoseField>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PanoseFamily {
    pub family: String,
    /// The face the others are compared with: a face of weight 400, width 5, not italic,
    /// preferably with the subfamily Regular; else the first face. It is listed first.
    pub base: String,
    pub fonts: Vec<PanoseFace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

fn panose_face(f: &FaceInfo, base: Option<&[u8; 10]>) -> PanoseFace {
    let Some(os2) = &f.os2 else {
        return PanoseFace { post_script_name: f.postscript_name.clone(), digits: Vec::new(), kind: "no OS/2 table", fields: Vec::new() };
    };
    let digits = os2.panose;
    let kind_digit = usize::from(digits.first().copied().unwrap_or(0));
    let kind = PANOSE_KIND.get(kind_digit).copied().unwrap_or("value out of range");
    let fields = match kind_digit.checked_sub(2).and_then(|i| PANOSE_CLASSES.get(i)) {
        None => Vec::new(),
        Some(class) => (0..10)
            .map(|i| {
                let value = digits.get(i).copied().unwrap_or(0);
                let names = if i == 0 { Some(PANOSE_KIND) } else { class.values.get(i - 1).copied() };
                PanoseField {
                    name: class.fields.get(i).copied().unwrap_or(""),
                    value,
                    label: names.and_then(|n| n.get(usize::from(value))).copied(),
                    differs: base.is_some_and(|b| b.get(i) != Some(&value)),
                }
            })
            .collect(),
    };
    PanoseFace { post_script_name: f.postscript_name.clone(), digits: digits.to_vec(), kind, fields }
}

pub fn panose_report(faces: &[FaceInfo]) -> Vec<PanoseFamily> {
    families(faces)
        .into_iter()
        .map(|g| {
            let candidates: Vec<&FaceInfo> = g
                .faces
                .iter()
                .copied()
                .filter(|f| f.os2.as_ref().is_some_and(|o| o.weight_class == 400 && o.width_class == 5) && !f.italic)
                .collect();
            let base = candidates.iter().find(|f| f.subfamily == "Regular").or(candidates.first()).or(g.faces.first()).copied();
            let Some(base) = base else {
                return PanoseFamily { family: g.family, base: String::new(), fonts: Vec::new(), skipped: None };
            };
            let Some(base_digits) = base.os2.as_ref().map(|o| o.panose) else {
                return PanoseFamily {
                    family: g.family,
                    base: base.postscript_name.clone(),
                    fonts: Vec::new(),
                    skipped: Some("the base face has no OS/2 table".into()),
                };
            };
            let mut fonts = vec![panose_face(base, None)];
            fonts.extend(g.faces.iter().filter(|f| !std::ptr::eq(**f, base)).map(|f| panose_face(f, Some(&base_digits))));
            PanoseFamily { family: g.family, base: base.postscript_name.clone(), fonts, skipped: None }
        })
        .collect()
}

// ── Feature report ──────────────────────────────────────────────────────────────────────────────

/// A script and its language systems (`dflt` for the default one), sorted.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct ScriptLanguages {
    pub script: String,
    pub languages: Vec<String>,
}

/// A set of lookups that one or more features map to, numbered from 1 in the order the walk
/// first meets it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct LookupSet {
    pub id: usize,
    pub lookups: Vec<u16>,
}

/// One feature's lookup set in each column (`None` where the language system lacks it).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct FeatureRow {
    pub feature: String,
    /// Every column has the same set (or every column lacks it).
    pub same: bool,
    pub sets: Vec<Option<usize>>,
}

/// A (script, language) column of the feature table.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Column {
    pub script: String,
    pub language: String,
}

/// What GSUB or GPOS of one face exposes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableLayout {
    pub table: &'static str,
    pub scripts: Vec<ScriptLanguages>,
    pub columns: Vec<Column>,
    pub lookup_sets: Vec<LookupSet>,
    pub features: Vec<FeatureRow>,
    /// The walk stopped at [`MAX_LAYOUT_ENTRIES`] or [`MAX_LOOKUP_REFS`].
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// GPOS then GSUB of one face.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct LayoutInfo {
    pub tables: Vec<TableLayout>,
}

impl LayoutInfo {
    pub fn read(font: &FontRef<'_>) -> Self {
        let gpos = match font.gpos() {
            Ok(t) => walk_layout("GPOS", t.script_list(), t.feature_list()),
            Err(e) => missing("GPOS", &e),
        };
        let gsub = match font.gsub() {
            Ok(t) => walk_layout("GSUB", t.script_list(), t.feature_list()),
            Err(e) => missing("GSUB", &e),
        };
        LayoutInfo { tables: vec![gpos, gsub] }
    }
}

fn missing(table: &'static str, e: &read_fonts::ReadError) -> TableLayout {
    let error = match e {
        read_fonts::ReadError::TableIsMissing(_) => None,
        other => Some(format!("cannot read the {table} table: {other}")),
    };
    TableLayout { table, error, ..TableLayout::default() }
}

fn tag_text(t: Tag) -> String {
    t.to_be_bytes().iter().map(|&b| char::from(b)).collect::<String>().trim_end().to_string()
}

fn walk_layout(
    table: &'static str,
    scripts: Result<ScriptList<'_>, read_fonts::ReadError>,
    features: Result<FeatureList<'_>, read_fonts::ReadError>,
) -> TableLayout {
    let mut out = TableLayout { table, ..TableLayout::default() };
    let (scripts, features) = match (scripts, features) {
        (Ok(s), Ok(f)) => (s, f),
        (Err(e), _) | (_, Err(e)) => {
            out.error = Some(format!("cannot read the {table} table: {e}"));
            return out;
        }
    };
    let records = features.feature_records();
    let mut set_ids: HashMap<Vec<u16>, usize> = HashMap::new();
    let mut tree: BTreeMap<String, BTreeMap<String, BTreeMap<String, usize>>> = BTreeMap::new();
    let mut tags: BTreeSet<String> = BTreeSet::new();
    let (mut entries, mut refs) = (0usize, 0usize);
    'walk: for sr in scripts.script_records() {
        let stag = tag_text(sr.script_tag());
        tree.insert(stag.clone(), BTreeMap::new());
        let Ok(script) = sr.script(scripts.offset_data()) else { continue };
        let mut systems = Vec::new();
        for lr in script.lang_sys_records() {
            if let Ok(ls) = lr.lang_sys(script.offset_data()) {
                systems.push((tag_text(lr.lang_sys_tag()), ls));
            }
        }
        if let Some(Ok(ls)) = script.default_lang_sys() {
            systems.push(("dflt".to_string(), ls));
        }
        for (ltag, ls) in systems {
            let mut map = BTreeMap::new();
            let required = Some(ls.required_feature_index()).filter(|&i| i != 0xFFFF);
            for index in ls.feature_indices().iter().map(|i| i.get()).chain(required) {
                entries += 1;
                if entries > MAX_LAYOUT_ENTRIES {
                    out.truncated = true;
                    if let Some(s) = tree.get_mut(&stag) {
                        s.insert(ltag, map);
                    }
                    break 'walk;
                }
                let Some(rec) = records.get(usize::from(index)) else { continue };
                let Ok(feature) = rec.feature(features.offset_data()) else { continue };
                let indices = feature.lookup_list_indices();
                refs = refs.saturating_add(indices.len());
                if refs > MAX_LOOKUP_REFS {
                    out.truncated = true;
                    if let Some(s) = tree.get_mut(&stag) {
                        s.insert(ltag, map);
                    }
                    break 'walk;
                }
                let lookups: Vec<u16> = indices.iter().map(|i| i.get()).collect();
                let next = set_ids.len() + 1;
                let id = *set_ids.entry(lookups).or_insert(next);
                let ftag = tag_text(rec.feature_tag());
                tags.insert(ftag.clone());
                map.insert(ftag, id);
            }
            if let Some(s) = tree.get_mut(&stag) {
                s.insert(ltag, map);
            }
        }
    }
    let mut sets: Vec<LookupSet> = set_ids.into_iter().map(|(lookups, id)| LookupSet { id, lookups }).collect();
    sets.sort_by_key(|s| s.id);
    out.lookup_sets = sets;
    for (script, langs) in &tree {
        out.scripts.push(ScriptLanguages { script: script.clone(), languages: langs.keys().cloned().collect() });
        for language in langs.keys() {
            out.columns.push(Column { script: script.clone(), language: language.clone() });
        }
    }
    for tag in tags {
        let sets: Vec<Option<usize>> = tree.values().flat_map(|langs| langs.values().map(|feats| feats.get(&tag).copied())).collect();
        let same = sets.windows(2).all(|w| w.first() == w.get(1));
        out.features.push(FeatureRow { feature: tag, same, sets });
    }
    out
}

/// Faces whose GSUB and GPOS agree, with what those tables expose.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FeatureGroup {
    pub fonts: Vec<String>,
    pub tables: Vec<TableLayout>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FeatureReport {
    /// Every face has the same script, language, feature and lookup structure.
    pub same: bool,
    pub groups: Vec<FeatureGroup>,
}

/// Groups faces (read with their layout) whose layout tables agree; groups are sorted by their
/// first face's PostScript name, faces within a group keep the family order.
pub fn feature_report(faces: &[FaceInfo]) -> FeatureReport {
    let mut groups: Vec<FeatureGroup> = Vec::new();
    let empty = LayoutInfo::default();
    for g in families(faces) {
        for f in g.faces {
            let layout = f.layout.as_ref().unwrap_or(&empty);
            match groups.iter_mut().find(|grp| grp.tables == layout.tables) {
                Some(grp) => grp.fonts.push(f.postscript_name.clone()),
                None => groups.push(FeatureGroup { fonts: vec![f.postscript_name.clone()], tables: layout.tables.clone() }),
            }
        }
    }
    groups.sort_by(|a, b| a.fonts.first().cmp(&b.fonts.first()));
    FeatureReport { same: groups.len() <= 1, groups }
}

// ── External validator messages known to be wrong ───────────────────────────────────────────────

/// A family of an external validator's messages that are wrong or irrelevant for well-built
/// fonts, and when they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownIssue {
    pub id: &'static str,
    /// The validator's message codes.
    pub codes: &'static [&'static str],
    pub reason: &'static str,
    when: When,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum When {
    Always,
    /// The font has CFF or CFF2 outlines.
    Cff,
    /// The message names a TrueType-only table and the font has CFF outlines.
    CffMissingTable,
    /// The message is about the `size` feature's parameters.
    SizeFeature,
    /// The message names a registered feature tag (`ssNN`, `cvNN`).
    RegisteredTag,
    /// The OS/2 table is version 4 or later.
    Os2V4,
    /// The OS/2 table is version 2 or later.
    Os2V2,
    /// The face is not the italic of another face in its family.
    SingleStyle,
    /// The glyph count in the message equals the run of empty glyphs at the end of the font.
    TrailingEmptyGlyphs,
}

/// The messages the validator's documentation lists as wrong for well-built fonts. The private
/// use area warning (W0307) is not among them: the audit reports private use code points itself,
/// in the `strict` profile.
pub const KNOWN_VALIDATOR_ISSUES: &[KnownIssue] = &[
    KnownIssue {
        id: "trueTypeTablesInCff",
        codes: &["W0022"],
        reason: "gasp, kern, hdmx and VDMX are TrueType tables; a font with CFF outlines does not need them",
        when: When::CffMissingTable,
    },
    KnownIssue {
        id: "rasterizerNeedsGlyf",
        codes: &["E6059"],
        reason: "the validator rasterises glyf outlines only; a font with CFF outlines has none",
        when: When::Cff,
    },
    KnownIssue {
        id: "sizeFeatureParams",
        codes: &["E5400"],
        reason: "the registered size feature has feature parameters by definition",
        when: When::SizeFeature,
    },
    KnownIssue {
        id: "registeredFeatureTag",
        codes: &["W5300"],
        reason: "stylistic set and character variant tags are registered",
        when: When::RegisteredTag,
    },
    KnownIssue {
        id: "os2Version4",
        codes: &["E2127", "E2132", "W0051"],
        reason: "the validator does not know OS/2 version 4 and later and calls the table corrupt",
        when: When::Os2V4,
    },
    KnownIssue { id: "os2VersionBelow3", codes: &["W2106"], reason: "OS/2 version 2 and later are valid", when: When::Os2V2 },
    KnownIssue {
        id: "codePagePartialCoverage",
        codes: &["W2101"],
        reason: "a code page bit may be set without every character of the code page",
        when: When::Always,
    },
    KnownIssue {
        id: "lowestRecPpemCff",
        codes: &["W1305"],
        reason: "in a font with CFF outlines the smallest readable size is left to the rasteriser",
        when: When::Cff,
    },
    KnownIssue {
        id: "italicAngleWithoutItalicBit",
        codes: &["E1316", "E2308"],
        reason: "a slanted face that is not the italic of another face keeps its real italic angle without the italic bit",
        when: When::SingleStyle,
    },
    KnownIssue {
        id: "lineMetricsAdvice",
        codes: &["W1404", "W1405", "W1406"],
        reason: "the line gap and hhea values follow the typographic metrics, not the validator's recommendation",
        when: When::Always,
    },
    KnownIssue {
        id: "trailingEmptyGlyphsLoca",
        codes: &["E1703"],
        reason: "empty glyphs at the end of the font make the validator misread loca",
        when: When::TrailingEmptyGlyphs,
    },
];

/// What [`known_validator_issue`] needs to know about the font a message is about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ValidatorFacts {
    pub cff: bool,
    pub os2_version: Option<u16>,
    /// The face is not the italic of another face in its family.
    pub single_style: bool,
    /// How many glyphs at the end of the font have no outline.
    pub trailing_empty_glyphs: usize,
}

impl ValidatorFacts {
    /// Reads the outline format, the OS/2 version and the trailing empty glyphs (from `loca`) of
    /// face `face`; `single_style` comes from the caller, who knows the family.
    pub fn read(bytes: &[u8], face: u32, single_style: bool) -> Self {
        match FontRef::from_index(bytes, face) {
            Ok(font) => Self::of_font(&font, single_style),
            Err(_) => Self { single_style, ..Self::default() },
        }
    }

    /// [`ValidatorFacts::read`] for a face already open.
    pub fn of_font(font: &FontRef<'_>, single_style: bool) -> Self {
        let cff = font.table_data(Tag::new(b"CFF ")).is_some() || font.table_data(Tag::new(b"CFF2")).is_some();
        let os2_version = font.os2().ok().map(|t| t.version());
        let mut trailing = 0usize;
        if let Ok(loca) = font.loca(None) {
            let count = loca.len();
            let mut i = count;
            while i > 0 {
                let (Some(a), Some(b)) = (loca.get_raw(i - 1), loca.get_raw(i)) else { break };
                if a != b {
                    break;
                }
                trailing += 1;
                i -= 1;
            }
        }
        Self { cff, os2_version, single_style, trailing_empty_glyphs: trailing }
    }
}

const TRUETYPE_ONLY_TABLES: [&str; 4] = ["gasp", "kern", "hdmx", "VDMX"];

/// The entry of [`KNOWN_VALIDATOR_ISSUES`] that a validator message (`code`, and its text with
/// details) matches for a font with `facts`; `None` when the message stands.
pub fn known_validator_issue(code: &str, text: &str, facts: &ValidatorFacts) -> Option<&'static KnownIssue> {
    let issue = KNOWN_VALIDATOR_ISSUES.iter().find(|k| k.codes.contains(&code))?;
    let applies = match issue.when {
        When::Always => true,
        When::Cff => facts.cff,
        When::CffMissingTable => facts.cff && TRUETYPE_ONLY_TABLES.iter().any(|t| text.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == *t)),
        When::SizeFeature => text.contains("(size)") || text.contains("'size'"),
        When::RegisteredTag => quoted_tag(text).is_some_and(registered_numbered_tag),
        When::Os2V4 => facts.os2_version.is_some_and(|v| v >= 4),
        When::Os2V2 => facts.os2_version.is_some_and(|v| v >= 2),
        When::SingleStyle => facts.single_style,
        When::TrailingEmptyGlyphs => last_number(text).is_some_and(|n| n > 0 && n == facts.trailing_empty_glyphs),
    };
    applies.then_some(issue)
}

/// The tag in `tag = 'ss01'`.
fn quoted_tag(text: &str) -> Option<&str> {
    let start = text.find('\'')? + 1;
    let rest = text.get(start..)?;
    rest.get(..rest.find('\'')?)
}

/// `ss01` to `ss20` and `cv01` to `cv99`.
fn registered_numbered_tag(tag: &str) -> bool {
    let (Some(prefix), Some(digits)) = (tag.get(..2), tag.get(2..)) else { return false };
    let Ok(n) = digits.parse::<u8>() else { return false };
    digits.len() == 2 && matches!((prefix, n), ("ss", 1..=20) | ("cv", 1..=99))
}

fn last_number(text: &str) -> Option<usize> {
    text.split(|c: char| !c.is_ascii_digit()).rfind(|w| !w.is_empty())?.parse().ok()
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;

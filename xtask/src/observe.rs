//! `cargo xtask observe`: the checklist for sittings with the running reference app.
//!
//! Collects every inventory row that watching the reference app could settle: a row with
//! `guesses` or `observe` steps, a row whose status is `memory`, or an `inferred` row whose `ref`
//! names a handbook chapter. Rows already `observed`, rows with `own = true` (no counterpart in the
//! reference app) and rows about other tools (command-line rows, quality checks, binary tables, the
//! command-line tools area) are left out. The list goes to `docs/observe.md`, grouped first by
//! whether the row's milestone is built, then by the place in the app where the check happens.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use crate::parity::live_ids;
use crate::spec::{self, Inventory, Row};

/// The last built milestone. Rows of this milestone and earlier ones describe behaviour FontCraft
/// already has, so they come first. Move it forward when a milestone lands.
const LAST_BUILT: &str = "M6";

const UNCONFIRMED_STATUSES: &[&str] = &["memory", "inferred"];
/// Row kinds the reference editor cannot settle: the oracles for them are other tools.
const OTHER_TOOL_KINDS: &[&str] = &["cli", "check", "table"];
const OTHER_TOOL_AREAS: &[&str] = &["cli-tools"];

const EDIT: &str = "Edit View: drawing, guides, layers and the Palette";
const TEXT: &str = "Edit View: text mode and preview";
const COMPONENTS: &str = "Components and anchors";
const FILTERS: &str = "Filters";
const SPACING: &str = "Spacing and metrics keys";
const KERNING: &str = "Kerning and the Kerning window";
const FONT_VIEW: &str = "Font View";
const FONT_INFO: &str = "Font Info and features";
const INTERPOLATION: &str = "Interpolation";
const HINTING: &str = "Hinting";
const COLOUR: &str = "Colour fonts";
const FILES: &str = "Files: open, import, export";
const SETTINGS: &str = "Settings and the window";
const SCRIPTS: &str = "Scripts, plug-ins and URLs";
const ELSEWHERE: &str = "Elsewhere";

/// Places in the order a sitting takes them.
const PLACES: &[&str] =
    &[EDIT, TEXT, COMPONENTS, FILTERS, SPACING, KERNING, FONT_VIEW, FONT_INFO, INTERPOLATION, HINTING, COLOUR, FILES, SETTINGS, SCRIPTS, ELSEWHERE];

/// Does watching the reference app have something to settle on this row?
pub fn wanted(r: &Row) -> bool {
    if r.status == "observed" || r.own() || OTHER_TOOL_KINDS.contains(&r.kind) || OTHER_TOOL_AREAS.contains(&r.area.as_str()) {
        return false;
    }
    if !r.guesses().is_empty() || !r.observe().is_empty() || r.status == "memory" {
        return true;
    }
    // An inferred row with nothing written down is our own design; it has something to observe
    // only when it points at a handbook chapter, which describes behaviour the app has.
    r.status == "inferred" && handbook_place(r).is_some()
}

/// The place in the app of the handbook chapter the row's `ref` starts with, if it names one.
fn handbook_place(r: &Row) -> Option<&'static str> {
    let first = r.str("ref").split(';').next().unwrap_or("");
    let mut parts = first.split('›').map(str::trim);
    let chapter = parts.next().unwrap_or("");
    let section = parts.next().unwrap_or("");
    match chapter {
        "Edit View" if matches!(section, "Entering Text" | "Previewing & Testing") => Some(TEXT),
        "Edit View" if section.contains("Anchor") || section.contains("Mark Attachment") => Some(COMPONENTS),
        "Edit View" | "Palette" => Some(EDIT),
        "Filters" => Some(FILTERS),
        "Reusing Shapes" => Some(COMPONENTS),
        "Spacing & Kerning" if section == "Kerning" => Some(KERNING),
        "Spacing & Kerning" => Some(SPACING),
        "Font View" => Some(FONT_VIEW),
        "Font Info" | "Feature Code" => Some(FONT_INFO),
        "Appendix" if section.starts_with("Custom") || section.starts_with("Automatic Feature") => Some(FONT_INFO),
        "Appendix" if section == "Text Tokens" => Some(TEXT),
        "Appendix" if section == "Regular Expressions" => Some(FONT_VIEW),
        "Appendix" if section == "URL Scheme" => Some(SCRIPTS),
        "Interpolation" => Some(INTERPOLATION),
        "PostScript Hinting" | "TrueType Hinting" => Some(HINTING),
        "Color Fonts" => Some(COLOUR),
        "Import & Export" => Some(FILES),
        c if c.starts_with("Create") => Some(FILES),
        c if c.starts_with("Settings") => Some(SETTINGS),
        c if c.starts_with("Extensions") => Some(SCRIPTS),
        _ => None,
    }
}

/// Where in the app the row is checked: the handbook chapter its `ref` starts with, else its
/// menu, else its area.
pub fn place(r: &Row) -> &'static str {
    if let Some(p) = handbook_place(r) {
        return p;
    }
    let menu = r.fields.get("menu").and_then(toml::Value::as_array).and_then(|m| m.first()).and_then(toml::Value::as_str);
    if menu == Some("Filter") {
        return FILTERS;
    }
    match r.area.as_str() {
        "drawing" | "path-ops" | "guides-metrics" | "layers" => EDIT,
        "edit-text" | "proofing" => TEXT,
        "components" => COMPONENTS,
        "spacing-kerning" if r.key.to_ascii_lowercase().contains("kern") => KERNING,
        "spacing-kerning" => SPACING,
        "font-view" => FONT_VIEW,
        "font-info" | "features" => FONT_INFO,
        "interpolation" => INTERPOLATION,
        "hinting-ps" | "hinting-tt" => HINTING,
        "color" => COLOUR,
        "import-export" | "compile" => FILES,
        "ui-chrome" => SETTINGS,
        "extensions" | "automation" => SCRIPTS,
        _ => ELSEWHERE,
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The row's label, or the first sentence of its summary cut to a dozen words.
fn title(r: &Row) -> String {
    let label = r.str("label");
    if !label.is_empty() {
        return one_line(label);
    }
    let summary = one_line(r.str("summary"));
    let sentence = summary.split(". ").next().unwrap_or("").trim_end_matches('.');
    let words: Vec<&str> = sentence.split(' ').collect();
    if words.len() > 12 { format!("{}…", words.iter().take(12).copied().collect::<Vec<_>>().join(" ")) } else { sentence.to_string() }
}

struct Tier {
    title: String,
    rows: Vec<Vec<usize>>,
}

pub fn render(inv: &Inventory, live: &BTreeSet<&str>) -> String {
    let order = |m: &str| inv.milestones.iter().position(|x| x.id == m).unwrap_or(usize::MAX);
    let last_built = order(LAST_BUILT);
    let first_id = inv.milestones.first().map(|m| m.id.as_str()).unwrap_or("");
    let mut tiers = [
        Tier { title: format!("Built milestones ({first_id} to {LAST_BUILT})"), rows: vec![Vec::new(); PLACES.len()] },
        Tier { title: format!("Later milestones (after {LAST_BUILT})"), rows: vec![Vec::new(); PLACES.len()] },
    ];
    let mut picked: Vec<usize> = (0..inv.rows.len()).filter(|&i| inv.rows.get(i).is_some_and(wanted)).collect();
    // Stable: inventory order within a milestone.
    picked.sort_by_key(|&i| inv.rows.get(i).map(|r| order(&r.milestone)).unwrap_or(usize::MAX));
    for &i in &picked {
        let Some(r) = inv.rows.get(i) else { continue };
        let tier = usize::from(order(&r.milestone) > last_built);
        let at = PLACES.iter().position(|p| *p == place(r)).unwrap_or(PLACES.len().saturating_sub(1));
        if let Some(list) = tiers.get_mut(tier).and_then(|t| t.rows.get_mut(at)) {
            list.push(i);
        }
    }
    let rows_of = |ids: &[usize]| ids.iter().filter_map(|&i| inv.rows.get(i)).collect::<Vec<&Row>>();
    let all = rows_of(&picked);
    let guesses: usize = all.iter().map(|r| r.guesses().len()).sum();
    let steps: usize = all.iter().map(|r| r.observe().len()).sum();

    let mut out = String::new();
    let _ = writeln!(out, "# Observation checklist\n");
    let _ = writeln!(
        out,
        "Generated by `cargo xtask observe` from `spec/inventory/*.toml`; do not edit. `cargo xtask ci` fails when it is out of date.\n"
    );
    let _ = writeln!(out, "Clean-room: observe the reference app on our own synthetic fonts; never copy its assets or text.\n");
    let _ = writeln!(out, "## What is listed\n");
    let _ = writeln!(out, "Every inventory row that watching the running reference app could settle:\n");
    let _ = writeln!(out, "- rows with `guesses`: choices of ours where no source says what the reference does;");
    let _ = writeln!(
        out,
        "- rows whose `status` is `memory` (recalled), and rows whose `status` is `inferred` (our design) that point at a handbook chapter;"
    );
    let _ = writeln!(out, "- rows with `observe` steps: one concrete thing to do in the app and what to look for.\n");
    let _ = writeln!(
        out,
        "Left out: rows already `observed`, rows with `own = true` (no counterpart in the reference app), and rows that other tools settle (command-line rows, quality checks, binary tables).\n"
    );
    let _ = writeln!(
        out,
        "Rows of built milestones come first, because their guesses are behaviour FontCraft already has. Within a milestone group the rows are sorted by the place in the app where the check happens, so one sitting can stay in one window. `live` after a row's status marks a command FontCraft registers today, which can be tried side by side.\n"
    );
    let _ = writeln!(out, "## How to record a result\n");
    let _ = writeln!(out, "Edit the row in `spec/inventory/*.toml`:\n");
    let _ = writeln!(out, "1. Write what the app did and the date in the row's `notes`.");
    let _ = writeln!(
        out,
        "2. Remove each guess the observation settles. If the app does something else, change FontCraft's behaviour and the row's `summary` to match."
    );
    let _ = writeln!(out, "3. Remove the `observe` steps you carried out.");
    let _ = writeln!(out, "4. When nothing about the row is left open, set `status = \"observed\"`.");
    let _ = writeln!(out, "5. Run `cargo xtask guesses` and `cargo xtask observe` and commit the regenerated files.\n");
    let _ = writeln!(out, "The checkboxes are for a printed or copied list; a settled row drops out of this file when it is regenerated.\n");
    let _ = writeln!(out, "To add a check, append a step to the row's `observe` list: what to do, and what to look for.\n");

    let _ = writeln!(out, "## Counts\n");
    let _ = writeln!(out, "**{} rows to check: {guesses} guesses and {steps} observe steps.**\n", all.len());
    let _ = writeln!(out, "| Group | Place | Rows |\n|---|---|---|");
    for t in &tiers {
        let total: usize = t.rows.iter().map(Vec::len).sum();
        let _ = writeln!(out, "| **{}** | all | **{total}** |", t.title);
        for (p, ids) in PLACES.iter().zip(&t.rows).filter(|(_, ids)| !ids.is_empty()) {
            let _ = writeln!(out, "| {} | {p} | {} |", t.title, ids.len());
        }
    }
    let _ = writeln!(out);

    for t in &tiers {
        let total: usize = t.rows.iter().map(Vec::len).sum();
        let _ = writeln!(out, "## {} ({total})\n", t.title);
        for (p, ids) in PLACES.iter().zip(&t.rows).filter(|(_, ids)| !ids.is_empty()) {
            let _ = writeln!(out, "### {p} ({})\n", ids.len());
            for r in rows_of(ids) {
                let is_live = r.kind == "command" && live.contains(r.key.as_str());
                let kind = if r.kind == "command" { String::new() } else { format!("{}, ", r.kind) };
                let live_mark = if is_live { ", live" } else { "" };
                let _ = writeln!(out, "- [ ] `{}` ({kind}{}, {}{live_mark}): {}", r.key, r.milestone, r.status, title(r));
                let guesses = r.guesses();
                for g in &guesses {
                    let _ = writeln!(out, "  - Guess: {}", one_line(g));
                }
                if guesses.is_empty() && UNCONFIRMED_STATUSES.contains(&r.status.as_str()) {
                    let why = if r.status == "memory" { "recalled, not confirmed" } else { "our design, not confirmed" };
                    let _ = writeln!(out, "  - Open ({why}): {}", one_line(r.str("summary")));
                }
                for s in r.observe() {
                    let _ = writeln!(out, "  - Observe: {}", one_line(s));
                }
            }
            let _ = writeln!(out);
        }
    }
    out
}

pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let inv = spec::load(root).map_err(|e| format!("{} problem(s) in spec/ (run `cargo xtask spec`)", e.len()))?;
    let live = live_ids();
    let text = render(&inv, &live);
    let path = root.join("docs/observe.md");
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != text {
            return Err("docs/observe.md is out of date: run `cargo xtask observe` and commit the result".into());
        }
        println!("observe: docs/observe.md is up to date");
    } else {
        std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("observe: wrote docs/observe.md");
    }
    let rows: Vec<&Row> = inv.rows.iter().filter(|r| wanted(r)).collect();
    println!(
        "observe: {} rows ({} guesses, {} observe steps)",
        rows.len(),
        rows.iter().map(|r| r.guesses().len()).sum::<usize>(),
        rows.iter().map(|r| r.observe().len()).sum::<usize>()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Area, Milestone};

    fn row(kind: &'static str, key: &str, milestone: &str, status: &str, reference: &str, extra: &[(&str, toml::Value)]) -> Row {
        let mut fields = toml::Table::new();
        fields.insert("ref".into(), toml::Value::String(reference.into()));
        fields.insert("summary".into(), toml::Value::String(format!("What {key} does. More detail.")));
        for (k, v) in extra {
            fields.insert((*k).into(), v.clone());
        }
        Row {
            kind,
            key: key.into(),
            file: "spec/inventory/x.toml".into(),
            area: "drawing".into(),
            milestone: milestone.into(),
            status: status.into(),
            fields,
        }
    }

    fn strings(items: &[&str]) -> toml::Value {
        toml::Value::Array(items.iter().map(|s| toml::Value::String((*s).into())).collect())
    }

    #[test]
    fn open_rows_are_listed_by_built_milestone_then_place_and_settled_rows_are_left_out() {
        let guess = [("guesses", strings(&["We chose this."]))];
        let inv = Inventory {
            areas: vec![Area { id: "drawing".into(), title: "D".into(), weight: 1 }],
            milestones: ["M3", "M6", "M9"].iter().map(|m| Milestone { id: (*m).into(), title: (*m).into() }).collect(),
            rows: vec![
                row("command", "kerning.later", "M9", "handbook", "Spacing & Kerning › Kerning", &guess),
                row("behavior", "font.view", "M6", "inferred", "Font View › Grid View", &[]),
                row("command", "path.live", "M3", "handbook", "Edit View › Draw Tool", &guess),
                row("command", "path.steps", "M3", "handbook", "Palette › Fit Curve", &[("observe", strings(&["Press the key; read the field."]))]),
                row("command", "path.seen", "M3", "observed", "Edit View", &guess),
                row("command", "path.mine", "M3", "inferred", "Our design", &[("own", toml::Value::Boolean(true))]),
                row("check", "check.other", "M3", "inferred", "AFDKO", &[]),
                row("command", "path.plain", "M3", "handbook", "Edit View", &[]),
                row("behavior", "path.ours", "M3", "inferred", "Our design", &[]),
            ],
        };
        let live: BTreeSet<&str> = ["path.live"].into_iter().collect();
        let text = render(&inv, &live);
        assert!(text.contains("**4 rows to check: 2 guesses and 1 observe steps.**"), "{text}");
        for gone in ["path.seen", "path.mine", "check.other", "path.plain", "path.ours"] {
            assert!(!text.contains(gone), "{gone}: {text}");
        }
        assert!(text.contains("- [ ] `path.live` (M3, handbook, live): What path.live does\n  - Guess: We chose this."), "{text}");
        assert!(text.contains("  - Observe: Press the key; read the field."), "{text}");
        assert!(text.contains("- [ ] `font.view` (behavior, M6, inferred): What font.view does\n  - Open (our design, not confirmed): What font.view does. More detail."), "{text}");
        let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s}: {text}"));
        assert!(at("### Edit View: drawing") < at("### Font View"), "{text}");
        assert!(at("`font.view`") < at("## Later milestones"), "{text}");
        assert!(at("## Later milestones") < at("### Kerning and the Kerning window"), "{text}");
    }
}

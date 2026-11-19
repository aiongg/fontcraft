//! The result document the Font Audit panel lists, and how its filters list it.

use std::collections::HashMap;

use craft::ui::contract::PlainData;
use serde_json::{Map, Value, json};

/// The run targets: the `target` of `audit.showPanel` and `audit.panel.run`.
pub const TARGETS: [&str; 5] = ["font", "selectedGlyphs", "currentGlyph", "export", "family"];
/// The list groupings: the `groupBy` of `audit.filterResults`.
pub const GROUP_BY: [&str; 3] = ["check", "glyph", "severity"];
/// The severity choices: the `minSeverity` of `audit.filterResults`.
pub const SEVERITIES: [&str; 3] = ["info", "warn", "fail"];
/// The check groups of the registry.
pub const GROUPS: [&str; 18] = [
    "outlines",
    "components",
    "anchors",
    "glyphset",
    "metrics",
    "names",
    "tables",
    "layout",
    "variations",
    "hinting",
    "kerning",
    "family",
    "files",
    "sources",
    "color",
    "vendor",
    "cid",
    "plugin",
];
/// The built-in profiles, in the order the profile pop-up lists them.
pub const BUILT_IN: [&str; 4] = ["outlines", "opentype", "universal", "strict"];

/// Most results the panel keeps from one run (the engine lists at most this many per document).
pub const MAX_ITEMS: usize = 100_000;
/// Most checks the panel reads from `audit.list`.
pub const MAX_CHECKS: usize = 4_096;
/// Most fonts in a family run, and most fonts one result names.
pub const MAX_FONTS: usize = 256;
/// Most run warnings the panel shows.
const MAX_WARNINGS: usize = 64;
/// Most addresses a result's location gives of each kind that reveal selects.
const MAX_ADDRESSES: usize = 1_000;

/// A result's severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    #[default]
    Info,
    Warn,
    Fail,
}

impl Severity {
    pub fn of(name: &str) -> Severity {
        match name {
            "fail" => Severity::Fail,
            "warn" => Severity::Warn,
            _ => Severity::Info,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warn => "warn",
            Severity::Fail => "fail",
        }
    }
}

/// Which results the panel lists (`audit.filterResults`).
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct Filter {
    /// One of [`SEVERITIES`].
    pub min_severity: String,
    /// [`GROUPS`] ids; empty: every group.
    pub groups: Vec<String>,
    pub text: String,
    pub fixable_only: bool,
    pub current_glyph_only: bool,
    /// One of [`GROUP_BY`].
    pub group_by: String,
    pub show_ignored: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            min_severity: "info".into(),
            groups: Vec::new(),
            text: String::new(),
            fixable_only: false,
            current_glyph_only: false,
            group_by: "check".into(),
            show_ignored: false,
        }
    }
}

impl Filter {
    /// The filter as `ui.inspect` reports it.
    pub fn to_json(&self) -> Value {
        json!({
            "minSeverity": self.min_severity,
            "groups": self.groups,
            "text": self.text,
            "fixableOnly": self.fixable_only,
            "currentGlyphOnly": self.current_glyph_only,
            "groupBy": self.group_by,
            "showIgnored": self.show_ignored,
        })
    }
}

/// What the registry says about a check.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckInfo {
    pub label: String,
    pub description: String,
    pub group: String,
}

/// One result of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub check: String,
    pub severity: Severity,
    pub message: String,
    pub glyph: Option<String>,
    pub master: Option<String>,
    pub layer: Option<String>,
    /// The compiled table, else the instance, a font-level result names.
    pub table: Option<String>,
    pub source_field: Option<String>,
    /// The font id the result is about (`location.font`).
    pub font: Option<String>,
    /// The font ids of a family result (`location.fonts`).
    pub fonts: Vec<String>,
    pub fixable: bool,
    pub safe: bool,
    pub ignored: bool,
    pub stale: bool,
    /// Lower-case check id, glyph, table and message, for the search field.
    haystack: String,
    /// The result as the result document gives it.
    pub raw: Value,
}

impl Item {
    /// A result of the result document (`docs/spec/21-qa.md` § 4); `None` without an id.
    pub fn from_result(raw: Value) -> Option<Item> {
        let text = |ptr: &str| raw.pointer(ptr).and_then(Value::as_str).map(str::to_string);
        let flag = |ptr: &str| raw.pointer(ptr).and_then(Value::as_bool).unwrap_or(false);
        let id = text("/id")?;
        let check = text("/check").unwrap_or_default();
        let message = text("/message").unwrap_or_default();
        let glyph = text("/location/glyph");
        let table = text("/location/table").or_else(|| text("/location/instance"));
        let fonts = raw
            .pointer("/location/fonts")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(MAX_FONTS).filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let haystack = [check.as_str(), glyph.as_deref().unwrap_or(""), table.as_deref().unwrap_or(""), message.as_str()].join(" ").to_lowercase();
        Some(Item {
            id,
            severity: Severity::of(raw.get("severity").and_then(Value::as_str).unwrap_or("info")),
            check,
            message,
            glyph,
            master: text("/location/master"),
            layer: text("/location/layer"),
            table,
            source_field: text("/location/sourceField"),
            font: text("/location/font"),
            fonts,
            fixable: flag("/fix/available"),
            safe: flag("/fix/safe"),
            ignored: flag("/ignored"),
            stale: flag("/stale"),
            haystack,
            raw,
        })
    }

    /// The glyph, else the table, else the font: the place column.
    pub fn place(&self) -> String {
        self.glyph.clone().or_else(|| self.table.clone()).unwrap_or_else(|| "font".into())
    }

    /// `[[shape, node]]` the result's location gives under `key` (`points`, `segments`).
    pub fn addresses(&self, key: &str) -> Vec<[usize; 2]> {
        let Some(list) = self.raw.pointer(&format!("/location/{key}")).and_then(Value::as_array) else { return Vec::new() };
        list.iter()
            .take(MAX_ADDRESSES)
            .filter_map(|p| {
                let p = p.as_array()?;
                Some([usize::try_from(p.first()?.as_u64()?).ok()?, usize::try_from(p.get(1)?.as_u64()?).ok()?])
            })
            .collect()
    }

    /// The shape indices the result's location gives.
    pub fn shapes(&self) -> Vec<usize> {
        self.raw
            .pointer("/location/shapes")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(MAX_ADDRESSES).filter_map(|s| usize::try_from(s.as_u64()?).ok()).collect())
            .unwrap_or_default()
    }

    /// `[x0, y0, x1, y1]` in font units: the result's bounds, else its point.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let numbers = |key: &str| -> Option<Vec<f64>> {
            let list = self.raw.pointer(&format!("/location/{key}"))?.as_array().filter(|a| a.len() <= 4)?;
            list.iter().map(|v| v.as_f64().filter(|n| n.is_finite())).collect()
        };
        if let Some(b) = numbers("bounds") {
            return Some([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]);
        }
        let p = numbers("point")?;
        let (x, y) = (*p.first()?, *p.get(1)?);
        Some([x, y, x, y])
    }
}

/// The result document of the panel's last run, and what it was run for.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The document that was active (for a family, the one the panel ran from).
    pub doc: u64,
    /// Its revision when the run finished.
    pub revision: u64,
    /// One of [`TARGETS`].
    pub target: String,
    /// The `audit.run` parameters, for a run again.
    pub params: Value,
    /// The engine keeps this run for the document, so its ids work in `audit.fix` and `audit.ignoreIssue` (every
    /// target but a family of files, and no cancelled run).
    pub kept: bool,
    pub profile: String,
    pub summary: Map<String, Value>,
    pub items: Vec<Item>,
    pub checks: HashMap<String, CheckInfo>,
    /// `(font id, path)` of each font the run examined.
    pub fonts: Vec<(String, Option<String>)>,
    pub warnings: Vec<String>,
    pub run_id: Option<String>,
    /// The results of a cancelled run: what it had examined.
    pub partial: bool,
    /// The result document without its results, for Export Report.
    pub head: Value,
}

impl Run {
    pub fn read(mut report: Value, doc: (u64, u64), target: String, params: Value, kept: bool, checks: HashMap<String, CheckInfo>) -> Run {
        let items = match report.get_mut("results").map(Value::take) {
            Some(Value::Array(list)) => list.into_iter().take(MAX_ITEMS).filter_map(Item::from_result).collect(),
            _ => Vec::new(),
        };
        let fonts = report
            .pointer("/target/fonts")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(MAX_FONTS)
                    .filter_map(|f| Some((f.get("id")?.as_str()?.to_string(), f.get("path").and_then(Value::as_str).map(str::to_string))))
                    .collect()
            })
            .unwrap_or_default();
        let warnings = report
            .get("warnings")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(MAX_WARNINGS).filter_map(|w| w.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let partial = report.get("partial").and_then(Value::as_bool).unwrap_or(false);
        Run {
            doc: doc.0,
            revision: doc.1,
            target,
            params,
            kept: kept && !partial,
            profile: report.get("profile").and_then(Value::as_str).unwrap_or_default().to_string(),
            summary: report.get("summary").and_then(Value::as_object).cloned().unwrap_or_default(),
            items,
            checks,
            fonts,
            warnings,
            run_id: report.get("runId").and_then(Value::as_str).map(str::to_string),
            partial,
            head: report,
        }
    }

    /// The result document again, for Export Report.
    pub fn document(&self) -> Value {
        let mut doc = self.head.clone();
        if let Some(o) = doc.as_object_mut() {
            o.insert("results".into(), Value::Array(self.items.iter().map(|i| i.raw.clone()).collect()));
        }
        doc
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    fn group_of(&self, item: &Item) -> &str {
        self.checks.get(&item.check).map_or("", |c| c.group.as_str())
    }

    /// A count of the summary.
    pub fn count(&self, key: &str) -> u64 {
        self.summary.get(key).and_then(Value::as_u64).unwrap_or(0)
    }
}

/// One group of the listing: its key (prefixed by the grouping), its name and its results by index in
/// [`Run::items`], in run order. The name is the check's description (else its label), the glyph (empty for
/// font-wide results) or the severity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub key: String,
    pub name: String,
    pub items: Vec<usize>,
}

/// The listed results, grouped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub groups: Vec<Group>,
}

impl Listing {
    /// Every listed result in list order.
    pub fn order(&self) -> impl Iterator<Item = usize> + '_ {
        self.groups.iter().flat_map(|g| g.items.iter().copied())
    }

    pub fn len(&self) -> usize {
        self.groups.iter().map(|g| g.items.len()).sum()
    }
}

/// Whether `filter` lists `item`; `glyph` is the glyph Current Glyph Only keeps.
fn keeps(run: &Run, filter: &Filter, glyph: Option<&str>, needle: &str, item: &Item) -> bool {
    item.severity >= Severity::of(&filter.min_severity)
        && (filter.show_ignored || !item.ignored)
        && (!filter.fixable_only || item.fixable)
        && (!filter.current_glyph_only || (glyph.is_some() && item.glyph.as_deref() == glyph))
        && (filter.groups.is_empty() || filter.groups.iter().any(|g| g == run.group_of(item)))
        && (needle.is_empty() || item.haystack.contains(needle))
}

/// The results `filter` lists, grouped as it says: groups in the order their first result comes (severity:
/// failures, warnings, notes), results in run order within a group.
pub fn listing(run: &Run, filter: &Filter, glyph: Option<&str>) -> Listing {
    let needle = filter.text.trim().to_lowercase();
    let by = filter.group_by.as_str();
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, item) in run.items.iter().enumerate() {
        if !keeps(run, filter, glyph, &needle, item) {
            continue;
        }
        let key = match by {
            "glyph" => item.glyph.clone().unwrap_or_default(),
            "severity" => item.severity.name().to_string(),
            _ => item.check.clone(),
        };
        match index.get(&key).and_then(|&g| groups.get_mut(g)) {
            Some((_, list)) => list.push(i),
            None => {
                index.insert(key.clone(), groups.len());
                groups.push((key, vec![i]));
            }
        }
    }
    if by == "severity" {
        groups.sort_by_key(|(k, _)| std::cmp::Reverse(Severity::of(k)));
    }
    let groups = groups
        .into_iter()
        .map(|(key, items)| {
            let name = match by {
                "glyph" | "severity" => key.clone(),
                _ => run
                    .checks
                    .get(&key)
                    .map_or_else(|| key.clone(), |c| if c.description.is_empty() { c.label.clone() } else { c.description.clone() }),
            };
            Group { key: format!("{by}:{key}"), name, items }
        })
        .collect();
    Listing { groups }
}

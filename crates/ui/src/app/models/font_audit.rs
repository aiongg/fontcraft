//! The Font Audit panel's view model: the panel's run as the filters list it, the selected result's details and
//! what the buttons may do. Projected only while the panel shows.

use std::collections::HashMap;

use craft::ui::contract::{PlainData, Tracked};
use serde_json::Value;

use super::ProjectCx;
use crate::shell::dock::PanelId;
use crate::shell::font_audit::run::{self, Filter, Item};

/// Most characters of a result's data the detail area shows.
const MAX_DATA_CHARS: usize = 2_000;

/// One listed result.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct AuditRow {
    pub id: String,
    /// "info", "warn" or "fail".
    pub severity: String,
    /// The glyph, else the table, else "font".
    pub place: String,
    /// The master's name.
    pub master: String,
    pub message: String,
    pub ignored: bool,
    pub stale: bool,
}

/// One group of the list.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct AuditGroup {
    pub key: String,
    /// "check", "glyph" or "severity".
    pub by: String,
    /// The check's description, the glyph (empty: font-wide) or the severity.
    pub name: String,
    pub open: bool,
    pub rows: Vec<AuditRow>,
}

/// A profile the pop-up lists; a built-in one has no label (the view names it).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ProfileEntry {
    pub id: String,
    pub label: Option<String>,
}

/// Steps of a run: done of total.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq)]
pub struct Steps {
    pub done: u64,
    pub total: u64,
}

/// The run's summary counts.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub fail: u64,
    pub warn: u64,
    pub info: u64,
    pub passed: u64,
    pub skipped: u64,
    pub ignored: u64,
    pub errors: u64,
}

/// One part of a result's location: its kind ("glyph", "master", "layer", "table", "field", "instance",
/// "sourceField", "fonts") and value.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct LocationPart {
    pub kind: String,
    pub value: String,
}

/// The current result, as the detail area shows it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct AuditDetail {
    pub id: String,
    /// The check's label, else its id.
    pub label: String,
    pub description: String,
    pub location: Vec<LocationPart>,
    /// The result's data as JSON, cut to [`MAX_DATA_CHARS`].
    pub data: String,
    pub fixable: bool,
    pub safe: bool,
    /// The fix's description.
    pub fix: String,
    /// The fix preview covers this result: its planned and skipped steps, and how many results it covers.
    pub previewed: bool,
    pub preview_count: usize,
    pub planned: Vec<String>,
    pub skipped: Vec<String>,
}

/// What the Font Audit panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FontAuditModel {
    pub profile: Option<String>,
    pub target: String,
    pub profiles: Vec<ProfileEntry>,
    pub default_profile: String,
    pub progress: Option<Steps>,
    pub has_run: bool,
    /// The run has no results at all.
    pub no_results: bool,
    pub counts: Counts,
    pub partial: bool,
    /// How far a cancelled run got.
    pub partial_steps: Option<Steps>,
    pub warnings: Vec<String>,
    pub filter: Filter,
    pub groups: Vec<AuditGroup>,
    pub listed: usize,
    pub selected: Vec<String>,
    pub current: Option<String>,
    pub detail: Option<AuditDetail>,
    /// Why Fix and Ignore are off: "", "partial", "files", "inactive" or "changed".
    pub blocked: String,
    /// The selected results Fix and Preview Fix take.
    pub fixable: Vec<String>,
    /// The listed results Fix All Safe takes.
    pub safe: Vec<String>,
    /// The selected results Ignore takes, and the selected ignored ones Stop Ignoring takes.
    pub to_ignore: Vec<String>,
    pub ignored: Vec<String>,
    /// Ignore and Stop Ignoring are possible at all (the engine keeps the run).
    pub kept: bool,
    /// A fix preview of this run exists: Apply shows.
    pub preview: bool,
    pub reason: String,
    pub export_open: bool,
    pub export_path: String,
    pub export_format: String,
}

fn location(item: &Item, masters: &HashMap<String, String>) -> Vec<LocationPart> {
    let part = |kind: &str, value: String| LocationPart { kind: kind.into(), value };
    let mut parts = Vec::new();
    if let Some(g) = &item.glyph {
        parts.push(part("glyph", g.clone()));
    }
    if let Some(m) = &item.master {
        parts.push(part("master", masters.get(m).cloned().unwrap_or_else(|| m.clone())));
    }
    if let Some(l) = &item.layer
        && item.master.as_ref() != Some(l)
    {
        parts.push(part("layer", l.clone()));
    }
    for key in ["table", "field", "instance", "sourceField"] {
        if let Some(v) = item.raw.pointer(&format!("/location/{key}")).and_then(Value::as_str) {
            parts.push(part(key, v.to_string()));
        }
    }
    if !item.fonts.is_empty() {
        parts.push(part("fonts", item.fonts.len().to_string()));
    }
    parts
}

pub fn project(model: &mut Tracked<FontAuditModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout.panel_state(&PanelId::FontAudit) == craft::behavior::dock::PanelState::Hidden {
        return;
    }
    let st = &cx.view.font_audit;
    let mut m = FontAuditModel {
        profile: st.profile.clone(),
        target: st.target.clone(),
        profiles: st.profiles.iter().map(|p| ProfileEntry { id: p.id.clone(), label: p.label.clone() }).collect(),
        default_profile: st.default_profile.clone(),
        progress: st.progress.as_ref().map(|p| Steps { done: p.done, total: p.total }),
        filter: st.filter.clone(),
        selected: st.selected.clone(),
        current: st.current.clone(),
        reason: st.reason.clone(),
        export_open: st.export_open,
        export_path: st.export_path.clone(),
        export_format: st.export_format.clone(),
        ..FontAuditModel::default()
    };
    if let Some(run) = &st.run {
        let font = cx.session.font().ok();
        let glyph = font.and_then(|f| crate::canvas::tools::edited_glyph(f, cx.view)).map(str::to_string);
        let active = cx.session.active().filter(|d| d.id == run.doc);
        let revision = active.map(|d| d.revision);
        let masters: HashMap<String, String> =
            active.map(|d| d.font.masters.iter().map(|m| (m.id.clone(), m.name.clone())).collect()).unwrap_or_default();
        let listing = run::listing(run, &st.filter, glyph.as_deref());
        let row = |item: &Item| AuditRow {
            id: item.id.clone(),
            severity: item.severity.name().into(),
            place: item.place(),
            master: item.master.as_ref().map(|id| masters.get(id).cloned().unwrap_or_else(|| id.clone())).unwrap_or_default(),
            message: item.message.clone(),
            ignored: item.ignored,
            stale: item.stale,
        };
        m.groups = listing
            .groups
            .iter()
            .map(|g| AuditGroup {
                key: g.key.clone(),
                by: st.filter.group_by.clone(),
                name: g.name.clone(),
                open: !st.collapsed.contains(&g.key),
                rows: g.items.iter().filter_map(|&i| run.items.get(i)).map(row).collect(),
            })
            .collect();
        m.listed = listing.len();
        m.has_run = true;
        m.no_results = run.items.is_empty();
        m.counts = Counts {
            fail: run.count("fail"),
            warn: run.count("warn"),
            info: run.count("info"),
            passed: run.count("passed"),
            skipped: run.count("skipped"),
            ignored: run.count("ignored"),
            errors: run.count("errors"),
        };
        m.partial = run.partial;
        m.partial_steps = run
            .head
            .pointer("/progress/done")
            .and_then(Value::as_u64)
            .zip(run.head.pointer("/progress/total").and_then(Value::as_u64))
            .map(|(done, total)| Steps { done, total });
        m.warnings = run.warnings.clone();
        m.kept = run.kept;
        m.blocked = if run.partial {
            "partial"
        } else if !run.kept {
            "files"
        } else if revision.is_none() {
            "inactive"
        } else if revision != Some(run.revision) {
            "changed"
        } else {
            ""
        }
        .into();
        let selected: Vec<&Item> = st.selected.iter().filter_map(|id| run.item(id)).collect();
        m.fixable = selected.iter().filter(|i| i.fixable && !i.ignored && !i.stale).map(|i| i.id.clone()).collect();
        m.to_ignore = selected.iter().filter(|i| !i.ignored).map(|i| i.id.clone()).collect();
        m.ignored = selected.iter().filter(|i| i.ignored).map(|i| i.id.clone()).collect();
        m.safe = listing
            .order()
            .filter_map(|i| run.items.get(i))
            .filter(|i| i.fixable && i.safe && !i.ignored && !i.stale)
            .map(|i| i.id.clone())
            .collect();
        let preview = st.preview.as_ref().filter(|p| p.run_id == run.run_id && !p.ids.is_empty());
        m.preview = preview.is_some();
        m.detail = st.current.as_deref().and_then(|c| run.item(c)).map(|item| {
            let info = run.checks.get(&item.check);
            let data = item
                .raw
                .get("data")
                .filter(|d| d.as_object().is_some_and(|o| !o.is_empty()))
                .map(|d| d.to_string().chars().take(MAX_DATA_CHARS).collect());
            let covered = preview.filter(|p| p.ids.contains(&item.id));
            AuditDetail {
                id: item.id.clone(),
                label: info.map(|c| c.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| item.check.clone()),
                description: info.map(|c| c.description.clone()).unwrap_or_default(),
                location: location(item, &masters),
                data: data.unwrap_or_default(),
                fixable: item.fixable,
                safe: item.safe,
                fix: item.raw.pointer("/fix/description").and_then(Value::as_str).unwrap_or_default().to_string(),
                previewed: covered.is_some(),
                preview_count: covered.map_or(0, |p| p.ids.len()),
                planned: covered.map(|p| p.planned.clone()).unwrap_or_default(),
                skipped: covered.map(|p| p.skipped.clone()).unwrap_or_default(),
            }
        });
    }
    super::set(model, m);
}

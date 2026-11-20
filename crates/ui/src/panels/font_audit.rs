//! The Font Audit panel (`window.fontAudit`): runs audits and lists their results.
//!
//! | Control | Posts |
//! |---|---|
//! | profile and target pop-ups | `audit.showPanel {profile}`, `{target}` |
//! | Run; Cancel beside the progress bar | `audit.panel.run {inSteps: true}`; `audit.panel.cancel` |
//! | gear menu | `audit.editProfile`, the export row (`audit.panel.setView {exportOpen}`), `audit.clearCache` |
//! | export row: file, format, Export | `audit.panel.setView {exportPath, exportFormat}`, `audit.panel.exportReport` |
//! | severity choices, group menu, search, Fixable Only, Current Glyph Only, Show Ignored, group-by pop-up | `audit.filterResults` |
//! | the result tree: press, Cmd and Shift press, group disclosure | `audit.panel.setView {selected, current}`, `{groupOpen}` |
//! | double press or Enter on a result | `audit.revealIssue` |
//! | Preview Fix, Apply, Fix, Fix All Safe | `audit.panel.previewFix`, `audit.panel.fix` |
//! | reason field, Ignore, Stop Ignoring | `audit.panel.setView {reason}`, `audit.panel.ignore`, `audit.panel.unignore` |
//! | Previous, Next | `audit.previousIssue`, `audit.nextIssue` |

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::Segmented;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked, Trigger,
};
use craft::ui::display::{Progress, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::font_audit::{AuditGroup, AuditRow, FontAuditModel};
use crate::shell::font_audit::run::{GROUP_BY, GROUPS, SEVERITIES, TARGETS};
use crate::shell::font_audit::{EXPORT_FORMATS, MAX_TEXT};

craft::ui::words! {
    /// The words of the Font Audit panel.
    pub struct FontAuditWords {
        profile: "fontcraftFontAudit.profile" = "Profile";
        default_profile: "fontcraftFontAudit.defaultProfile" = "Default";
        outlines: "fontcraftFontAudit.outlines" = "Outlines";
        opentype: "fontcraftFontAudit.opentype" = "OpenType";
        universal: "fontcraftFontAudit.universal" = "Universal";
        strict: "fontcraftFontAudit.strict" = "Strict";
        target: "fontcraftFontAudit.target" = "Target";
        this_font: "fontcraftFontAudit.thisFont" = "This Font";
        selected_glyphs: "fontcraftFontAudit.selectedGlyphs" = "Selected Glyphs";
        current_glyph: "fontcraftFontAudit.currentGlyph" = "Current Glyph";
        export_target: "fontcraftFontAudit.exportTarget" = "Export of This Font";
        family: "fontcraftFontAudit.family" = "Family of Open Fonts";
        run: "fontcraftFontAudit.run" = "Run";
        run_tip: "fontcraftFontAudit.runTip" = "Run the audit with this profile on this target";
        cancel: "fontcraftFontAudit.cancel" = "Cancel";
        cancel_tip: "fontcraftFontAudit.cancelTip" = "Stop the audit; what it found so far is listed, marked partial";
        progress: "fontcraftFontAudit.progress" = "Audit progress";
        of: "fontcraftFontAudit.of" = "of";
        options: "fontcraftFontAudit.options" = "Options";
        edit_profiles: "fontcraftFontAudit.editProfiles" = "Edit Profiles and Thresholds…";
        export_report: "fontcraftFontAudit.exportReport" = "Export Report…";
        clear_cache: "fontcraftFontAudit.clearCache" = "Clear Cache";
        report_file: "fontcraftFontAudit.reportFile" = "Report file";
        format: "fontcraftFontAudit.format" = "Format";
        export: "fontcraftFontAudit.export" = "Export";
        html: "fontcraftFontAudit.html" = "HTML";
        markdown: "fontcraftFontAudit.markdown" = "Markdown";
        json: "fontcraftFontAudit.json" = "JSON";
        text: "fontcraftFontAudit.text" = "Text";
        hint: "fontcraftFontAudit.hint" = "Choose a profile and a target, then press Run.";
        fail: "fontcraftFontAudit.fail" = "Fail";
        warn: "fontcraftFontAudit.warn" = "Warn";
        info: "fontcraftFontAudit.info" = "Info";
        passed: "fontcraftFontAudit.passed" = "Passed";
        skipped: "fontcraftFontAudit.skipped" = "Skipped";
        ignored_count: "fontcraftFontAudit.ignoredCount" = "Ignored";
        errors: "fontcraftFontAudit.errors" = "Errors";
        partial: "fontcraftFontAudit.partial" = "Partial results: the run was cancelled.";
        partial_after: "fontcraftFontAudit.partialAfter" = "Partial results: the run was cancelled after";
        steps: "fontcraftFontAudit.steps" = "steps";
        severity: "fontcraftFontAudit.severity" = "Severity";
        all: "fontcraftFontAudit.all" = "All";
        warnings_failures: "fontcraftFontAudit.warningsFailures" = "Warnings and Failures";
        failures: "fontcraftFontAudit.failures" = "Failures";
        all_groups: "fontcraftFontAudit.allGroups" = "All Groups";
        groups: "fontcraftFontAudit.groups" = "Groups";
        search: "fontcraftFontAudit.search" = "Search";
        fixable_only: "fontcraftFontAudit.fixableOnly" = "Fixable Only";
        current_glyph_only: "fontcraftFontAudit.currentGlyphOnly" = "Current Glyph Only";
        show_ignored: "fontcraftFontAudit.showIgnored" = "Show Ignored";
        group_by: "fontcraftFontAudit.groupBy" = "Group By";
        by_check: "fontcraftFontAudit.byCheck" = "By Check";
        by_glyph: "fontcraftFontAudit.byGlyph" = "By Glyph";
        by_severity: "fontcraftFontAudit.bySeverity" = "By Severity";
        results: "fontcraftFontAudit.results" = "Results";
        place: "fontcraftFontAudit.place" = "Place";
        master: "fontcraftFontAudit.master" = "Master";
        message: "fontcraftFontAudit.message" = "Message";
        no_issues: "fontcraftFontAudit.noIssues" = "No issues found.";
        no_match: "fontcraftFontAudit.noMatch" = "No result matches the filters.";
        font_wide: "fontcraftFontAudit.fontWide" = "Font-wide results";
        notes: "fontcraftFontAudit.notes" = "Notes";
        warnings: "fontcraftFontAudit.warnings" = "Warnings";
        ignored_mark: "fontcraftFontAudit.ignoredMark" = "(ignored)";
        stale_mark: "fontcraftFontAudit.staleMark" = "(changed since the run)";
        loc_glyph: "fontcraftFontAudit.locGlyph" = "glyph";
        loc_master: "fontcraftFontAudit.locMaster" = "master";
        loc_layer: "fontcraftFontAudit.locLayer" = "layer";
        loc_table: "fontcraftFontAudit.locTable" = "table";
        loc_field: "fontcraftFontAudit.locField" = "field";
        loc_instance: "fontcraftFontAudit.locInstance" = "instance";
        loc_source_field: "fontcraftFontAudit.locSourceField" = "Font Info field";
        loc_fonts: "fontcraftFontAudit.locFonts" = "fonts";
        fix_prefix: "fontcraftFontAudit.fixPrefix" = "Fix:";
        automatic_fix: "fontcraftFontAudit.automaticFix" = "an automatic fix";
        safe: "fontcraftFontAudit.safe" = "safe";
        changes_design: "fontcraftFontAudit.changesDesign" = "changes the design";
        no_fix: "fontcraftFontAudit.noFix" = "No automatic fix.";
        fix_preview: "fontcraftFontAudit.fixPreview" = "Fix preview";
        nothing_changes: "fontcraftFontAudit.nothingChanges" = "Nothing would change.";
        skipped_step: "fontcraftFontAudit.skippedStep" = "Skipped:";
        preview_fix: "fontcraftFontAudit.previewFix" = "Preview Fix";
        preview_fix_tip: "fontcraftFontAudit.previewFixTip" = "Show what fixing the selected results would change";
        apply: "fontcraftFontAudit.apply" = "Apply";
        apply_tip: "fontcraftFontAudit.applyTip" = "Make the changes the preview lists";
        fix: "fontcraftFontAudit.fix" = "Fix";
        fix_tip: "fontcraftFontAudit.fixTip" = "Fix the selected results";
        fix_all_safe: "fontcraftFontAudit.fixAllSafe" = "Fix All Safe";
        fix_all_safe_tip: "fontcraftFontAudit.fixAllSafeTip" = "Fix every listed result whose fix keeps the design";
        reason: "fontcraftFontAudit.reason" = "Reason";
        ignore: "fontcraftFontAudit.ignore" = "Ignore";
        ignore_tip: "fontcraftFontAudit.ignoreTip" = "Hide the selected results from now on, with this reason";
        unignore: "fontcraftFontAudit.unignore" = "Stop Ignoring";
        unignore_tip: "fontcraftFontAudit.unignoreTip" = "List the selected ignored results again";
        previous: "fontcraftFontAudit.previous" = "Previous";
        previous_tip: "fontcraftFontAudit.previousTip" = "Reveal the listed result before this one";
        next: "fontcraftFontAudit.next" = "Next";
        next_tip: "fontcraftFontAudit.nextTip" = "Reveal the listed result after this one";
        why_partial: "fontcraftFontAudit.whyPartial" = "Partial results of a cancelled run cannot be fixed or ignored: press Run again";
        why_files: "fontcraftFontAudit.whyFiles" = "Results of font files are fixed and ignored in the files themselves";
        why_inactive: "fontcraftFontAudit.whyInactive" = "The font of this run is not the active font";
        why_changed: "fontcraftFontAudit.whyChanged" = "The font changed since this run: press Run again";
    }
}

/// Rows the result list shows before it scrolls: the panel body around it scrolls too.
const LIST_ROWS: u16 = 16;

const SEVERITY: ColumnKey = ColumnKey::new("severity");
const PLACE: ColumnKey = ColumnKey::new("place");
const MASTER: ColumnKey = ColumnKey::new("master");
const MESSAGE: ColumnKey = ColumnKey::new("message");

pub struct Props<'a> {
    pub model: &'a Tracked<FontAuditModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &FontAuditModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<FontAuditWords>();
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        header(ui, &words, model, sent);
        if model.export_open && model.has_run {
            export_row(ui, &words, model, sent);
        }
        if !model.has_run {
            Text::new(words.hint.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        summary(ui, &words, model);
        filters(ui, &words, model, sent);
        results(ui, &words, model, sent);
        details(ui, &words, model);
        buttons(ui, &words, model, sent);
    });
}

fn profile_name(words: &FontAuditWords, id: &str, label: Option<&String>) -> Label {
    match (label, id) {
        (Some(l), _) => l.clone().into(),
        (None, "outlines") => words.outlines.clone(),
        (None, "opentype") => words.opentype.clone(),
        (None, "universal") => words.universal.clone(),
        (None, "strict") => words.strict.clone(),
        (None, other) => other.to_string().into(),
    }
}

fn header(ui: &mut Ui, words: &FontAuditWords, model: &FontAuditModel, sent: &mut Vec<Msg>) {
    let default_label = model.profiles.iter().find(|p| p.id == model.default_profile).map(|p| profile_name(words, &p.id, p.label.as_ref()));
    let default_text =
        format!("{} ({})", words.default_profile.as_str(), default_label.as_ref().map_or(model.default_profile.as_str(), Label::as_str));
    let profiles: Vec<Entry<String>> = std::iter::once(Entry::new(String::new(), default_text))
        .chain(model.profiles.iter().map(|p| Entry::new(p.id.clone(), profile_name(words, &p.id, p.label.as_ref()))))
        .collect();
    let target_words = [&words.this_font, &words.selected_glyphs, &words.current_glyph, &words.export_target, &words.family];
    let targets: Vec<Entry<String>> = TARGETS.iter().zip(target_words).map(|(id, label)| Entry::new(id.to_string(), label.clone())).collect();
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let current = model.profile.clone().unwrap_or_default();
        if let Some(picked) = Select::new(words.profile.clone(), Bind::given(current.clone()), &profiles).show(ui).events.committed()
            && *picked != current
        {
            let profile = if picked.is_empty() { Value::Null } else { json!(picked) };
            sent.push(shell("audit.showPanel", json!({ "visible": true, "profile": profile })));
        }
        if let Some(picked) = Select::new(words.target.clone(), Bind::given(model.target.clone()), &targets).show(ui).events.committed()
            && *picked != model.target
        {
            sent.push(shell("audit.showPanel", json!({ "visible": true, "target": picked })));
        }
        match model.progress {
            Some(p) => {
                let fraction = if p.total == 0 { 0.0 } else { (p.done as f32 / p.total as f32).clamp(0.0, 1.0) };
                Progress::new(fraction).label(format!("{} {} {}", p.done, words.of.as_str(), p.total)).show(ui);
                if Button::new(words.cancel.clone()).tooltip(words.cancel_tip.clone()).show(ui).clicked() {
                    sent.push(shell("audit.panel.cancel", json!({})));
                }
            }
            None => {
                if Button::new(words.run.clone()).tooltip(words.run_tip.clone()).show(ui).clicked() {
                    sent.push(shell("audit.panel.run", json!({ "inSteps": true })));
                }
            }
        }
        let opener = Button::new(words.options.clone()).show(ui);
        Menu::new(words.options.clone()).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            if MenuItem::new(words.edit_profiles.clone()).show(ui).ran {
                let profile = model.profile.clone().map_or(Value::Null, Value::String);
                sent.push(shell("audit.editProfile", if profile.is_null() { json!({}) } else { json!({ "profile": profile }) }));
            }
            if MenuItem::new(words.export_report.clone()).enabled(model.has_run).show(ui).ran {
                sent.push(shell("audit.panel.setView", json!({ "exportOpen": !model.export_open })));
            }
            if MenuItem::new(words.clear_cache.clone()).show(ui).ran {
                sent.push(Msg::run("audit.clearCache", json!({})));
            }
        });
    });
}

fn export_row(ui: &mut Ui, words: &FontAuditWords, model: &FontAuditModel, sent: &mut Vec<Msg>) {
    let format_words = [&words.html, &words.markdown, &words.json, &words.text];
    let formats: Vec<Entry<String>> = EXPORT_FORMATS.iter().zip(format_words).map(|(id, label)| Entry::new(id.to_string(), label.clone())).collect();
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let mut path = model.export_path.clone();
        let field = TextField::new(words.report_file.clone(), Bind::bound(&mut path)).placeholder(words.report_file.clone()).show(ui);
        if field.events.committed().is_some() && path != model.export_path {
            sent.push(shell("audit.panel.setView", json!({ "exportPath": path })));
        }
        if let Some(picked) = Select::new(words.format.clone(), Bind::given(model.export_format.clone()), &formats).show(ui).events.committed() {
            sent.push(shell("audit.panel.setView", json!({ "exportFormat": picked })));
        }
        let ready = !path.trim().is_empty();
        if Button::new(words.export.clone()).enabled(ready).show(ui).clicked() {
            sent.push(shell("audit.panel.exportReport", json!({ "path": path.trim(), "format": model.export_format })));
        }
    });
}

fn summary(ui: &mut Ui, words: &FontAuditWords, model: &FontAuditModel) {
    let c = model.counts;
    let mut line = format!(
        "{} {} · {} {} · {} {} · {} {} · {} {}",
        words.fail.as_str(),
        c.fail,
        words.warn.as_str(),
        c.warn,
        words.info.as_str(),
        c.info,
        words.passed.as_str(),
        c.passed,
        words.skipped.as_str(),
        c.skipped
    );
    if c.ignored > 0 {
        line.push_str(&format!(" · {} {}", words.ignored_count.as_str(), c.ignored));
    }
    if c.errors > 0 {
        line.push_str(&format!(" · {} {}", words.errors.as_str(), c.errors));
    }
    Text::new(line).show(ui);
    if model.partial {
        let text = match model.partial_steps {
            Some(s) => format!("{} {} {} {} {}.", words.partial_after.as_str(), s.done, words.of.as_str(), s.total, words.steps.as_str()),
            None => words.partial.as_str().to_string(),
        };
        Text::new(text).show(ui);
    }
    for w in &model.warnings {
        Text::new(w.clone()).shade(Shade::Muted).show(ui);
    }
}

fn filters(ui: &mut Ui, words: &FontAuditWords, model: &FontAuditModel, sent: &mut Vec<Msg>) {
    let f = &model.filter;
    let severity_words = [&words.all, &words.warnings_failures, &words.failures];
    let severities: Vec<Entry<String>> = SEVERITIES.iter().zip(severity_words).map(|(id, label)| Entry::new(id.to_string(), label.clone())).collect();
    let by_words = [&words.by_check, &words.by_glyph, &words.by_severity];
    let by: Vec<Entry<String>> = GROUP_BY.iter().zip(by_words).map(|(id, label)| Entry::new(id.to_string(), label.clone())).collect();
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let mut min = f.min_severity.clone();
        Segmented::new(words.severity.clone(), Bind::bound(&mut min), &severities).show(ui);
        if min != f.min_severity {
            sent.push(shell("audit.filterResults", json!({ "minSeverity": min })));
        }
        let caption = match f.groups.as_slice() {
            [] => words.all_groups.as_str().to_string(),
            [one] => one.clone(),
            more => format!("{} {}", more.len(), words.groups.as_str()),
        };
        let opener = Button::new(caption).show(ui);
        let mut entries = vec![Entry::new(String::new(), words.all_groups.clone()).checked(f.groups.is_empty())];
        entries.extend(GROUPS.iter().map(|g| Entry::new(g.to_string(), g.to_string()).checked(f.groups.iter().any(|x| x == g))));
        if let Some(picked) = Menu::new(words.groups.clone()).opened_by(&opener, Trigger::Click).pick(ui, &entries).picked {
            let groups: Vec<&str> = if picked.is_empty() {
                Vec::new()
            } else {
                let on = f.groups.contains(&picked);
                GROUPS.iter().copied().filter(|g| if *g == picked { !on } else { f.groups.iter().any(|x| x == g) }).collect()
            };
            sent.push(shell("audit.filterResults", json!({ "groups": groups })));
        }
        if let Some(picked) = Select::new(words.group_by.clone(), Bind::given(f.group_by.clone()), &by).show(ui).events.committed()
            && *picked != f.group_by
        {
            sent.push(shell("audit.filterResults", json!({ "groupBy": picked })));
        }
    });
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let mut text = f.text.clone();
        TextField::search(words.search.clone(), Bind::bound(&mut text)).placeholder(words.search.clone()).show(ui);
        if text != f.text {
            let text: String = text.chars().take(MAX_TEXT).collect();
            sent.push(shell("audit.filterResults", json!({ "text": text })));
        }
        for (key, label, on) in [
            ("fixableOnly", &words.fixable_only, f.fixable_only),
            ("currentGlyphOnly", &words.current_glyph_only, f.current_glyph_only),
            ("showIgnored", &words.show_ignored, f.show_ignored),
        ] {
            let mut now = on;
            ToggleButton::new(label.clone(), Bind::bound(&mut now)).show(ui);
            if now != on {
                sent.push(shell("audit.filterResults", json!({ key: now })));
            }
        }
    });
}

/// One entry of the result tree: a group heading or a result.
#[derive(Clone, Copy)]
enum Node<'a> {
    Group(&'a AuditGroup),
    Result(&'a AuditRow),
}

/// The tree's entries: each group's heading, then its results while it is open. Keys are `g:<group key>` and
/// `r:<result id>`.
struct Tree<'a> {
    entries: Vec<Node<'a>>,
}

impl<'a> Tree<'a> {
    fn new(groups: &'a [AuditGroup]) -> Tree<'a> {
        let mut entries = Vec::new();
        for g in groups {
            entries.push(Node::Group(g));
            if g.open {
                entries.extend(g.rows.iter().map(Node::Result));
            }
        }
        Tree { entries }
    }

    fn key_of(e: &Node<'_>) -> String {
        match e {
            Node::Group(g) => format!("g:{}", g.key),
            Node::Result(r) => format!("r:{}", r.id),
        }
    }

    fn find(&self, key: &str) -> Option<Node<'a>> {
        self.entries.iter().find(|e| Tree::key_of(e) == key).copied()
    }
}

impl DisplayOrder<String> for Tree<'_> {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn key(&self, at: usize) -> String {
        self.entries.get(at).map(Tree::key_of).unwrap_or_default()
    }

    fn depth(&self, at: usize) -> usize {
        usize::from(matches!(self.entries.get(at), Some(Node::Result(_))))
    }

    fn has_children(&self, at: usize) -> bool {
        matches!(self.entries.get(at), Some(Node::Group(_)))
    }

    fn open(&self, at: usize) -> bool {
        matches!(self.entries.get(at), Some(Node::Group(g)) if g.open)
    }

    fn is_section(&self, at: usize) -> bool {
        self.has_children(at)
    }

    fn section_stop(&self, at: usize) -> bool {
        self.has_children(at)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.entries.get(at).map(|e| match e {
            Node::Group(g) => Cow::Borrowed(g.name.as_str()),
            Node::Result(r) => Cow::Borrowed(r.message.as_str()),
        })
    }
}

impl ItemSource<String> for Tree<'_> {}

fn group_title(words: &FontAuditWords, g: &AuditGroup) -> String {
    let name = match (g.by.as_str(), g.name.as_str()) {
        ("glyph", "") => words.font_wide.as_str().to_string(),
        ("severity", "fail") => words.failures.as_str().to_string(),
        ("severity", "warn") => words.warnings.as_str().to_string(),
        ("severity", _) => words.notes.as_str().to_string(),
        (_, name) => name.to_string(),
    };
    format!("{name} ({})", g.rows.len())
}

fn severity_word(words: &FontAuditWords, severity: &str) -> Label {
    match severity {
        "fail" => words.fail.clone(),
        "warn" => words.warn.clone(),
        _ => words.info.clone(),
    }
}

fn results(ui: &mut Ui, words: &FontAuditWords, model: &FontAuditModel, sent: &mut Vec<Msg>) {
    if model.groups.is_empty() {
        Text::new(if model.no_results { words.no_issues.clone() } else { words.no_match.clone() }).shade(Shade::Muted).show(ui);
        return;
    }
    let tree = Tree::new(&model.groups);
    let keys: Vec<String> = model.selected.iter().map(|id| format!("r:{id}")).collect();
    let lead = model.current.as_ref().map(|c| format!("r:{c}"));
    let before = Selection::new(keys, lead.clone(), lead.clone());
    let mut after = before.clone();
    let columns = vec![
        ColumnSpec::new(SEVERITY, words.severity.clone(), ColumnWidth::Content),
        ColumnSpec::new(PLACE, words.place.clone(), ColumnWidth::Content),
        ColumnSpec::new(MASTER, words.master.clone(), ColumnWidth::Content),
        ColumnSpec::new(MESSAGE, words.message.clone(), ColumnWidth::fill(1.0)),
    ];
    let out = List::<String>::new(words.results.clone())
        .tree(true)
        .columns(columns)
        .max_rows(LIST_ROWS)
        .selection(Bind::bound(&mut after))
        .show_items(ui, &tree, &mut |key, icx| match tree.find(key) {
            Some(Node::Group(g)) => ListRow::new(group_title(words, g)).section_heading().group(g.open),
            Some(Node::Result(r)) => {
                let mut message = r.message.clone();
                if r.ignored {
                    message = format!("{message} {}", words.ignored_mark.as_str());
                }
                if r.stale {
                    message = format!("{message} {}", words.stale_mark.as_str());
                }
                let mut row = ListRow::new(severity_word(words, &r.severity))
                    .depth(1)
                    .cells(icx.cells())
                    .state(icx.row.clone())
                    .cell(PLACE, r.place.clone())
                    .cell(MASTER, r.master.clone())
                    .cell(MESSAGE, message);
                if r.ignored || r.stale {
                    row = row.ink(RowInk::Muted);
                }
                row
            }
            None => ListRow::new(String::new()),
        });
    if after.items != before.items || after.lead != before.lead {
        let ids: Vec<&str> = after.items.iter().filter_map(|k| k.strip_prefix("r:")).collect();
        let current = after.lead.as_deref().and_then(|k| k.strip_prefix("r:")).map_or(Value::Null, |c| json!(c));
        sent.push(shell("audit.panel.setView", json!({ "selected": ids, "current": current })));
    }
    for event in &out.events {
        match event {
            CollectionEvent::Open(change) => {
                if let Some(group) = change.key.strip_prefix("g:") {
                    sent.push(shell("audit.panel.setView", json!({ "groupOpen": { "group": group, "open": change.open } })));
                }
            }
            CollectionEvent::Activate(key) => {
                if let Some(id) = key.strip_prefix("r:") {
                    sent.push(shell("audit.revealIssue", json!({ "result": id })));
                }
            }
            _ => {}
        }
    }
}

fn location_word<'w>(words: &'w FontAuditWords, kind: &str) -> &'w str {
    match kind {
        "glyph" => words.loc_glyph.as_str(),
        "master" => words.loc_master.as_str(),
        "layer" => words.loc_layer.as_str(),
        "table" => words.loc_table.as_str(),
        "field" => words.loc_field.as_str(),
        "instance" => words.loc_instance.as_str(),
        "sourceField" => words.loc_source_field.as_str(),
        _ => words.loc_fonts.as_str(),
    }
}

fn details(ui: &mut Ui, words: &FontAuditWords, model: &FontAuditModel) {
    let Some(d) = &model.detail else { return };
    Text::new(d.label.clone()).show(ui);
    if !d.description.is_empty() {
        Text::new(d.description.clone()).shade(Shade::Muted).show(ui);
    }
    let place: Vec<String> = d
        .location
        .iter()
        .map(|p| {
            if p.kind == "fonts" {
                format!("{} {}", p.value, words.loc_fonts.as_str())
            } else {
                format!("{} {}", location_word(words, &p.kind), p.value)
            }
        })
        .collect();
    if !place.is_empty() {
        Text::new(place.join(", ")).show(ui);
    }
    if !d.data.is_empty() {
        Text::new(d.data.clone()).shade(Shade::Muted).show(ui);
    }
    let fix = if d.fixable {
        let what = if d.fix.is_empty() { words.automatic_fix.as_str() } else { d.fix.as_str() };
        let kind = if d.safe { words.safe.as_str() } else { words.changes_design.as_str() };
        format!("{} {what} ({kind})", words.fix_prefix.as_str())
    } else {
        words.no_fix.as_str().to_string()
    };
    Text::new(fix).shade(Shade::Muted).show(ui);
    if d.previewed {
        Text::new(format!("{} ({})", words.fix_preview.as_str(), d.preview_count)).show(ui);
        if d.planned.is_empty() {
            Text::new(words.nothing_changes.clone()).shade(Shade::Muted).show(ui);
        }
        for line in &d.planned {
            Text::new(line.clone()).show(ui);
        }
        for line in &d.skipped {
            Text::new(format!("{} {line}", words.skipped_step.as_str())).shade(Shade::Muted).show(ui);
        }
    }
}

fn buttons(ui: &mut Ui, words: &FontAuditWords, model: &FontAuditModel, sent: &mut Vec<Msg>) {
    let editable = model.blocked.is_empty();
    let why = match model.blocked.as_str() {
        "partial" => Some(&words.why_partial),
        "files" => Some(&words.why_files),
        "inactive" => Some(&words.why_inactive),
        "changed" => Some(&words.why_changed),
        _ => None,
    };
    let tip = |label: &Label| why.unwrap_or(label).clone();
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if Button::new(words.preview_fix.clone())
            .tooltip(tip(&words.preview_fix_tip))
            .enabled(editable && !model.fixable.is_empty())
            .show(ui)
            .clicked()
        {
            sent.push(shell("audit.panel.previewFix", json!({ "results": model.fixable })));
        }
        if model.preview && Button::new(words.apply.clone()).tooltip(tip(&words.apply_tip)).enabled(editable).show(ui).clicked() {
            sent.push(shell("audit.panel.fix", json!({ "preview": true })));
        }
        if Button::new(words.fix.clone()).tooltip(tip(&words.fix_tip)).enabled(editable && !model.fixable.is_empty()).show(ui).clicked() {
            sent.push(shell("audit.panel.fix", json!({ "results": model.fixable, "safeOnly": false })));
        }
        if Button::new(words.fix_all_safe.clone())
            .tooltip(tip(&words.fix_all_safe_tip))
            .enabled(editable && !model.safe.is_empty())
            .show(ui)
            .clicked()
        {
            sent.push(shell("audit.panel.fix", json!({ "results": model.safe, "safeOnly": true })));
        }
    });
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let mut reason = model.reason.clone();
        let field = TextField::new(words.reason.clone(), Bind::bound(&mut reason)).placeholder(words.reason.clone()).show(ui);
        if field.events.committed().is_some() && reason != model.reason {
            sent.push(shell("audit.panel.setView", json!({ "reason": reason })));
        }
        if Button::new(words.ignore.clone()).tooltip(tip(&words.ignore_tip)).enabled(editable && !model.to_ignore.is_empty()).show(ui).clicked() {
            sent.push(shell("audit.panel.ignore", json!({ "results": model.to_ignore, "reason": reason })));
        }
        if model.filter.show_ignored
            && Button::new(words.unignore.clone())
                .tooltip(words.unignore_tip.clone())
                .enabled(model.kept && !model.ignored.is_empty())
                .show(ui)
                .clicked()
        {
            sent.push(shell("audit.panel.unignore", json!({ "results": model.ignored })));
        }
        if Button::new(words.previous.clone()).tooltip(words.previous_tip.clone()).show(ui).clicked() {
            sent.push(shell("audit.previousIssue", json!({})));
        }
        if Button::new(words.next.clone()).tooltip(words.next_tip.clone()).show(ui).clicked() {
            sent.push(shell("audit.nextIssue", json!({})));
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;
    use crate::app::models::font_audit::Counts;

    fn model() -> FontAuditModel {
        let row = |id: &str, severity: &str, place: &str| AuditRow {
            id: id.into(),
            severity: severity.into(),
            place: place.into(),
            master: "Regular".into(),
            message: format!("{id} message"),
            ignored: false,
            stale: false,
        };
        FontAuditModel {
            target: "font".into(),
            default_profile: "universal".into(),
            has_run: true,
            counts: Counts { fail: 1, warn: 1, ..Counts::default() },
            filter: crate::shell::font_audit::run::Filter::default(),
            groups: vec![AuditGroup {
                key: "check:check.outlines.openPath".into(),
                by: "check".into(),
                name: "Open paths".into(),
                open: true,
                rows: vec![row("r1", "fail", "B"), row("r2", "warn", "A")],
            }],
            listed: 2,
            selected: vec!["r1".into()],
            current: Some("r1".into()),
            fixable: vec!["r1".into()],
            to_ignore: vec!["r1".into()],
            kept: true,
            export_format: "html".into(),
            ..FontAuditModel::default()
        }
    }

    #[test]
    fn the_buttons_post_the_panel_commands_on_the_selected_results() {
        let model = Tracked::new(model());
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let fix = fixture.by_label("Fix").expect("the Fix button");
        fixture.click(&fix).settle();
        let run = fixture.by_label("Run").expect("the Run button");
        fixture.click(&run).settle();
        let next = fixture.by_label("Next").expect("the Next button");
        fixture.click(&next).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                shell("audit.panel.fix", json!({ "results": ["r1"], "safeOnly": false })),
                shell("audit.panel.run", json!({ "inSteps": true })),
                shell("audit.nextIssue", json!({})),
            ]
        );
    }

    #[test]
    fn a_partial_run_turns_fix_and_ignore_off() {
        let mut m = model();
        m.blocked = "partial".into();
        m.partial = true;
        let model = Tracked::new(m);
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        for label in ["Fix", "Ignore"] {
            let button = fixture.by_label(label).expect("the button");
            fixture.click(&button).settle();
        }
        assert!(posted.borrow().is_empty(), "{:?}", posted.borrow());
    }
}

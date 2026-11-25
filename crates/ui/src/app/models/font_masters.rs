//! The Masters tab's view model: the master list, the selection, and the sections of the selected masters (General,
//! Axes Coordinates, Metrics & Alignment Zones, Stems, Custom Parameters, Number Values).
//!
//! A value the selected masters disagree on is `None` (the field shows mixed); a missing metric, stem or number value
//! reads as zero, and a short location reads the model's default coordinate.

use craft::ui::contract::PlainData;
use fontcraft_engine::fontinfo::rows::RowKind;
use fontcraft_engine::fontinfo::{MAX_MASTERS, MAX_METRIC_DEFS, MAX_STEM_DEFS, Scope};
use fontcraft_engine::model::axes::design_value;
use fontcraft_engine::model::{Font, metric};

use super::font_info::{MAX_SHOWN, RowModel, builtin, rows};
use super::layers::{MasterIcon, icon};
use crate::shell::font_info::Target;
use crate::shell::font_info_masters::MastersTabView;

/// The categories a metric may take, in menu order: the format's spelling and the label shown. The empty category is
/// a metric known only by its name.
pub const METRIC_CATEGORIES: [(&str, &str); 10] = [
    (metric::ASCENDER, "Ascender"),
    (metric::CAP_HEIGHT, "Cap Height"),
    ("slant height", "Slant Height"),
    (metric::X_HEIGHT, "x-Height"),
    ("midHeight", "Mid Height"),
    ("bodyHeight", "Body Height"),
    (metric::DESCENDER, "Descender"),
    (metric::BASELINE, "Baseline"),
    (metric::ITALIC_ANGLE, "Italic Angle"),
    ("", "None (named only)"),
];

/// The label of a metric category; a category the menu lacks shows as stored.
pub fn category_label(kind: &str) -> &str {
    METRIC_CATEGORIES.iter().find(|(k, _)| k.eq_ignore_ascii_case(kind)).map_or(kind, |(_, l)| l)
}

/// What the Masters tab shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MastersTabModel {
    pub masters: Vec<MasterItem>,
    /// The selected masters, in list order; the first master when none is selected. Empty without masters.
    pub selected: Vec<usize>,
    /// The masters' Master Name field.
    pub name: Option<RowModel>,
    /// The first selected master's icon; `icon_mixed` when the selection disagrees.
    pub icon: Option<MasterIcon>,
    pub icon_mixed: bool,
    /// Each axis's name and the selection's coordinate on it.
    pub axes: Vec<(String, Option<f64>)>,
    pub metrics: Vec<MetricItem>,
    pub stems: Vec<StemItem>,
    pub params: Vec<RowModel>,
    /// Each number's name and the selection's value.
    pub numbers: Vec<(String, Option<f64>)>,
    pub designspace: super::designspace::DesignspaceModel,
}

/// One master of the list.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct MasterItem {
    pub name: String,
    pub icon: MasterIcon,
}

/// One metric definition and the selection's values for it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MetricItem {
    /// The metric's name, else its category's label.
    pub label: String,
    /// The glyph scope, empty for none.
    pub scope: String,
    pub position: Option<f64>,
    pub zone: Option<f64>,
    /// An italic angle has no zone.
    pub angle: bool,
}

/// One stem definition and the selection's width for it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct StemItem {
    pub name: String,
    pub horizontal: bool,
    pub scope: String,
    pub value: Option<f64>,
}

impl MastersTabModel {
    /// The selection as the target of the shared rows.
    pub fn target(&self) -> Target {
        Target { scope: Scope::Master, indices: self.selected.clone() }
    }
}

/// The value every one of `values` shares, or `None` when they differ.
fn shared(mut values: impl Iterator<Item = f64>) -> Option<f64> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

/// `exports`: the selected entries of the Exports list; `None` while another tab shows, which leaves the designspace out.
pub fn project(font: &Font, view: &MastersTabView, exports: Option<&[usize]>) -> MastersTabModel {
    let count = font.masters.len().min(MAX_MASTERS);
    if count == 0 {
        return MastersTabModel::default();
    }
    let mut selected: Vec<usize> = view.selected.iter().copied().filter(|i| *i < count).collect();
    if selected.is_empty() {
        selected.push(0);
    }
    let target = Target { scope: Scope::Master, indices: selected.clone() };
    let chosen: Vec<_> = selected.iter().filter_map(|i| font.masters.get(*i)).collect();
    let first = chosen.first().copied();
    let metric_value = |at: usize| {
        let value = |m: &&fontcraft_engine::model::Master| m.metric_values.get(at).copied().unwrap_or_default();
        (shared(chosen.iter().map(|m| value(m).pos)), shared(chosen.iter().map(|m| value(m).over)))
    };
    MastersTabModel {
        masters: font.masters.iter().take(count).map(|m| MasterItem { name: m.name.clone(), icon: icon(font, m) }).collect(),
        name: builtin(font, &target, "name"),
        icon: first.map(|m| icon(font, m)),
        icon_mixed: first.is_some_and(|f| chosen.iter().any(|m| m.icon != f.icon)),
        axes: font
            .axes
            .iter()
            .take(MAX_SHOWN)
            .enumerate()
            .map(|(at, a)| (a.name.clone(), shared(chosen.iter().map(|m| design_value(&m.location, at)))))
            .collect(),
        metrics: font
            .metrics
            .iter()
            .take(MAX_METRIC_DEFS)
            .enumerate()
            .map(|(at, def)| {
                let (position, zone) = metric_value(at);
                MetricItem {
                    label: def.name.clone().unwrap_or_else(|| category_label(&def.kind).to_string()),
                    scope: def.filter.clone().unwrap_or_default(),
                    position,
                    zone,
                    angle: def.kind == metric::ITALIC_ANGLE,
                }
            })
            .collect(),
        stems: font
            .stems
            .iter()
            .take(MAX_STEM_DEFS)
            .enumerate()
            .map(|(at, def)| StemItem {
                name: def.name.clone(),
                horizontal: def.horizontal,
                scope: def.filter.clone().unwrap_or_default(),
                value: shared(chosen.iter().map(|m| m.stem_values.get(at).copied().unwrap_or_default())),
            })
            .collect(),
        params: rows(font, &target, RowKind::Param),
        numbers: font
            .numbers
            .iter()
            .take(MAX_SHOWN)
            .enumerate()
            .map(|(at, n)| (n.name.clone(), shared(chosen.iter().map(|m| m.number_values.get(at).copied().unwrap_or_default()))))
            .collect(),
        designspace: exports.map(|exports| super::designspace::project(font, view, &selected, exports)).unwrap_or_default(),
        selected,
    }
}

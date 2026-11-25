//! The per-feature extents of `HorizAxis.MinMax` and `VertAxis.MinMax` statements (row
//! `fea.table.base.minMax`), which fea-rs parses and does not write.
//!
//! The feature file specification (1.27, § 9.a) allows `, <feature tag> <min>, <max>` after a
//! language system's extents. fea-rs 1.0.0 writes the extents alone, and feaLib refuses the
//! feature tag, so the records follow the specification and the `BASE` table's format: one
//! `FeatMinMaxRecord` per feature, sorted by tag, in the `MinMax` table of its language system.
//! Statements for the same language system go into one table; a later one's values count.

use std::collections::BTreeMap;

use fea_rs::{Kind, Node, NodeOrToken};
use write_fonts_052::tables::base::{Axis, Base, BaseCoord, BaseLangSysRecord, FeatMinMaxRecord, MinMax};

use crate::{Diagnostic, Severity, Tag};

/// Writes the feature extents of the code's `BASE` block into `base`. The messages are about
/// statements that have no place in the table.
pub(crate) fn min_max(root: &Node, base: &mut Base) -> Vec<Diagnostic> {
    for axis in [base.horiz_axis.as_mut(), base.vert_axis.as_mut()].into_iter().flatten() {
        for record in &mut axis.base_script_list.base_script_records {
            // fea-rs writes a record per statement; the table holds one per language.
            let records = std::mem::take(&mut record.base_script.base_lang_sys_records);
            let one_each: BTreeMap<Tag, BaseLangSysRecord> = records.into_iter().map(|r| (r.base_lang_sys_tag, r)).collect();
            record.base_script.base_lang_sys_records = one_each.into_values().collect();
        }
    }
    let mut messages = Vec::new();
    for table in root.iter_children().filter_map(NodeOrToken::as_node).filter(|n| n.kind() == Kind::BaseTableNode) {
        for statement in table.iter_children().filter_map(NodeOrToken::as_node).filter(|n| n.kind() == Kind::BaseMinMaxNode) {
            let Some(found) = Statement::read(statement) else { continue };
            let axis = if found.vertical { base.vert_axis.as_mut() } else { base.horiz_axis.as_mut() };
            if !axis.is_some_and(|axis| found.write(axis)) {
                let axis = if found.vertical { "VertAxis" } else { "HorizAxis" };
                messages.push(Diagnostic::request(
                    Severity::Warning,
                    format!("{axis}.MinMax for the script `{}` was left out: {axis}.BaseScriptList does not list the script", found.script),
                ));
            }
        }
    }
    messages
}

struct Statement {
    vertical: bool,
    script: Tag,
    language: Tag,
    extents: (i16, i16),
    features: Vec<(Tag, (i16, i16))>,
}

impl Statement {
    fn read(node: &Node) -> Option<Self> {
        let vertical = node.iter_tokens().any(|t| t.kind == Kind::VertAxisMinMaxKw);
        let mut tags = tags(node);
        let (script, language) = (tags.next()?, tags.next()?);
        let features = node
            .iter_children()
            .filter_map(NodeOrToken::as_node)
            .filter(|n| n.kind() == Kind::BaseMinMaxFeatureNode)
            .filter_map(|n| Some((self::tags(n).next()?, extents(n)?)))
            .collect();
        Some(Self { vertical, script, language, extents: extents(node)?, features })
    }

    /// Whether the axis has the statement's script.
    fn write(&self, axis: &mut Axis) -> bool {
        let Some(record) = axis.base_script_list.base_script_records.iter_mut().find(|r| r.base_script_tag == self.script) else {
            return false;
        };
        let script = &mut record.base_script;
        let table: Option<&mut MinMax> = if self.language == Tag::new(b"dflt") {
            script.default_min_max.as_mut()
        } else {
            script.base_lang_sys_records.iter_mut().find(|r| r.base_lang_sys_tag == self.language).map(|r| &mut *r.min_max)
        };
        if let Some(table) = table {
            let (min, max) = self.extents;
            table.min_coord.set(BaseCoord::format_1(min));
            table.max_coord.set(BaseCoord::format_1(max));
            let mut records: BTreeMap<Tag, FeatMinMaxRecord> =
                std::mem::take(&mut table.feat_min_max_records).into_iter().map(|r| (r.feature_table_tag, r)).collect();
            for (tag, (min, max)) in &self.features {
                let record = FeatMinMaxRecord::new(*tag, Some(BaseCoord::format_1(*min)), Some(BaseCoord::format_1(*max)));
                records.insert(*tag, record);
            }
            table.feat_min_max_records = records.into_values().collect();
        }
        true
    }
}

/// The two numbers directly in `node`.
fn extents(node: &Node) -> Option<(i16, i16)> {
    let mut numbers =
        node.iter_children().filter_map(NodeOrToken::as_token).filter(|t| t.kind == Kind::Number).filter_map(|t| t.text.parse::<i16>().ok());
    Some((numbers.next()?, numbers.next()?))
}

/// The tags directly in `node`.
fn tags(node: &Node) -> impl Iterator<Item = Tag> {
    node.iter_children().filter_map(NodeOrToken::as_token).filter(|t| t.kind == Kind::Tag).filter_map(|t| Tag::new_checked(t.text.as_bytes()).ok())
}

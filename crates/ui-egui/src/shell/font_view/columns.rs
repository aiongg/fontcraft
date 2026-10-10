//! List View's columns (`fontView.column.*`): one per glyph property of the Glyph Properties
//! section, with what a cell shows, how a column sorts, and the command an inline edit runs.

use fontcraft_geom::Shape as _;
use fontcraft_model::{ColorLabel, Font, Glyph, Prop, PropSource, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::cache::Row;

/// A List View column. The serialised name is the column's id in commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Column {
    Id,
    Char,
    Name,
    Unicode,
    ProductionName,
    Script,
    Category,
    SubCategory,
    Case,
    Direction,
    Export,
    Color,
    Tags,
    SortName,
    Note,
    Components,
    LastChanged,
    Lsb,
    Rsb,
    Width,
    LeftGroup,
    RightGroup,
}

/// The twelve colour label names, in index order (`glyph.colorLabelIndices`).
pub const COLOR_LABEL_NAMES: [&str; 12] =
    ["Red", "Orange", "Brown", "Yellow", "Light Green", "Dark Green", "Cyan", "Blue", "Purple", "Pink", "Light Gray", "Dark Gray"];

/// How a cell is edited in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editing {
    No,
    Text,
    /// A click flips it (Exports).
    Toggle,
}

/// What a column sorts by.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub enum SortValue {
    Number(f64),
    Text(String),
    /// No value: sorts after every value in ascending order.
    None,
}

/// What a cell reads besides its row: the font and the master Font View shows.
pub struct CellCx<'a> {
    pub font: &'a Font,
    pub master: &'a str,
}

impl Column {
    /// Every column, in the default order: the documented columns first, then the properties in
    /// the order the Glyph Properties section lists them (`fontView.column.otherProperties`).
    pub const ALL: [Column; 22] = [
        Column::Id,
        Column::Char,
        Column::Name,
        Column::Unicode,
        Column::ProductionName,
        Column::Script,
        Column::Category,
        Column::SubCategory,
        Column::Case,
        Column::Direction,
        Column::Export,
        Column::Color,
        Column::Tags,
        Column::SortName,
        Column::Note,
        Column::Components,
        Column::LastChanged,
        Column::Lsb,
        Column::Rsb,
        Column::Width,
        Column::LeftGroup,
        Column::RightGroup,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Column::Id => "id",
            Column::Char => "char",
            Column::Name => "name",
            Column::Unicode => "unicode",
            Column::ProductionName => "productionName",
            Column::Script => "script",
            Column::Category => "category",
            Column::SubCategory => "subCategory",
            Column::Case => "case",
            Column::Direction => "direction",
            Column::Export => "export",
            Column::Color => "color",
            Column::Tags => "tags",
            Column::SortName => "sortName",
            Column::Note => "note",
            Column::Components => "components",
            Column::LastChanged => "lastChanged",
            Column::Lsb => "lsb",
            Column::Rsb => "rsb",
            Column::Width => "width",
            Column::LeftGroup => "leftGroup",
            Column::RightGroup => "rightGroup",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Column::Id => "ID",
            Column::Char => "Char",
            Column::Name => "Name",
            Column::Unicode => "Unicode",
            Column::ProductionName => "Production Name",
            Column::Script => "Script",
            Column::Category => "Category",
            Column::SubCategory => "Subcategory",
            Column::Case => "Case",
            Column::Direction => "Direction",
            Column::Export => "Exports",
            Column::Color => "Color",
            Column::Tags => "Tags",
            Column::SortName => "Sort Name",
            Column::Note => "Note",
            Column::Components => "Components",
            Column::LastChanged => "Last Changed",
            Column::Lsb => "LSB",
            Column::Rsb => "RSB",
            Column::Width => "Width",
            Column::LeftGroup => "Left Group",
            Column::RightGroup => "Right Group",
        }
    }

    pub fn parse(id: &str) -> Option<Column> {
        Column::ALL.iter().copied().find(|c| c.id() == id)
    }

    /// Every id, for error messages.
    pub fn ids() -> String {
        Column::ALL.iter().map(|c| c.id()).collect::<Vec<_>>().join(", ")
    }

    /// The column's width in units of the `list_column_unit` token.
    pub fn width_units(self) -> f32 {
        match self {
            Column::Id | Column::Char => 1.0,
            Column::Lsb | Column::Rsb | Column::Width => 1.2,
            Column::Case | Column::Direction | Column::Export => 1.6,
            Column::Unicode | Column::Script | Column::Category | Column::Color | Column::LeftGroup | Column::RightGroup => 2.2,
            Column::SubCategory | Column::SortName => 2.6,
            Column::Name | Column::ProductionName | Column::Tags | Column::Components => 3.2,
            Column::Note | Column::LastChanged => 4.0,
        }
    }

    /// Numbers are drawn flush right.
    pub fn is_numeric(self) -> bool {
        matches!(self, Column::Id | Column::Lsb | Column::Rsb | Column::Width)
    }

    /// Whether and how a cell of the column is edited in place: the columns of properties that
    /// have a set command. Metrics and kerning groups follow with their commands (M5),
    /// components with `glyph.setComponents` (M6).
    pub fn editing(self) -> Editing {
        match self {
            Column::Name
            | Column::Unicode
            | Column::ProductionName
            | Column::Script
            | Column::Category
            | Column::SubCategory
            | Column::Case
            | Column::Direction
            | Column::Color
            | Column::Tags
            | Column::SortName
            | Column::Note => Editing::Text,
            Column::Export => Editing::Toggle,
            _ => Editing::No,
        }
    }

    /// The property behind the cell, for the columns the glyph data can supply.
    fn prop(self, row: &Row) -> Option<&Prop> {
        let p = &row.props;
        Some(match self {
            Column::ProductionName => &p.production_name,
            Column::Script => &p.script,
            Column::Category => &p.category,
            Column::SubCategory => &p.sub_category,
            Column::Case => &p.case,
            Column::Direction => &p.direction,
            Column::SortName => &p.sort_name,
            _ => return None,
        })
    }

    /// The cell's value comes from the glyph data, not from the glyph: drawn dimmed.
    pub fn is_derived(self, row: &Row) -> bool {
        self.prop(row).is_some_and(|p| !matches!(p.source, PropSource::Stored | PropSource::None))
    }

    /// The text a cell shows.
    pub fn text(self, cx: &CellCx, row: &Row) -> String {
        let glyph = &row.glyph;
        if let Some(prop) = self.prop(row) {
            return prop.value.clone().unwrap_or_default();
        }
        match self {
            Column::Id => row.id.to_string(),
            Column::Char => first_char(glyph).map(String::from).unwrap_or_default(),
            Column::Name => glyph.name.clone(),
            Column::Unicode => unicode_text(&glyph.unicodes),
            Column::Export => if glyph.export { "Yes" } else { "No" }.into(),
            Column::Color => glyph.info.color.map(color_name).unwrap_or_default(),
            Column::Tags => glyph.info.tags.join(", "),
            Column::Note => glyph.info.note.lines().next().unwrap_or_default().to_string(),
            Column::Components => components(glyph, cx.master).join(" "),
            Column::LastChanged => glyph.info.last_change.clone().unwrap_or_default(),
            Column::Lsb | Column::Rsb | Column::Width => self.number(cx, row).map(|n| format!("{}", n.round())).unwrap_or_default(),
            Column::LeftGroup => glyph.kern_groups.left.clone().unwrap_or_default(),
            Column::RightGroup => glyph.kern_groups.right.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// The metric columns' values in the shown master.
    fn number(self, cx: &CellCx, row: &Row) -> Option<f64> {
        let layer = row.glyph.layer(cx.master)?;
        if self == Column::Width {
            return Some(layer.width);
        }
        let outline = cx.font.outline_of(&row.glyph, cx.master).ok()?;
        if outline.elements().is_empty() {
            return None;
        }
        let bounds = outline.bounding_box();
        match self {
            Column::Lsb => Some(bounds.x0),
            Column::Rsb => Some(layer.width - bounds.x1),
            _ => None,
        }
        .filter(|n| n.is_finite())
    }

    /// What the column sorts by.
    pub fn sort_value(self, cx: &CellCx, row: &Row) -> SortValue {
        match self {
            Column::Id => SortValue::Number(row.id as f64),
            Column::Unicode => row.glyph.unicodes.first().map_or(SortValue::None, |u| SortValue::Number(f64::from(*u))),
            Column::Char => first_char(&row.glyph).map_or(SortValue::None, |c| SortValue::Number(f64::from(u32::from(c)))),
            Column::Lsb | Column::Rsb | Column::Width => self.number(cx, row).map_or(SortValue::None, SortValue::Number),
            Column::Color => match row.glyph.info.color {
                Some(ColorLabel::Index(i)) => SortValue::Number(f64::from(i)),
                Some(ColorLabel::Rgba(_)) => SortValue::Number(f64::from(u8::MAX)),
                None => SortValue::None,
            },
            Column::Export => SortValue::Number(if row.glyph.export { 0.0 } else { 1.0 }),
            _ => {
                let text = self.text(cx, row);
                if text.is_empty() { SortValue::None } else { SortValue::Text(text.to_lowercase()) }
            }
        }
    }

    /// The text an inline edit of this cell starts from: the stored value, which for derived
    /// properties is empty (typing nothing keeps the glyph data's value).
    pub fn edit_text(self, cx: &CellCx, row: &Row) -> String {
        match self {
            Column::Note => row.glyph.info.note.clone(),
            Column::Color => match row.glyph.info.color {
                Some(ColorLabel::Index(i)) => color_name(ColorLabel::Index(i)),
                _ => String::new(),
            },
            _ if self.is_derived(row) => String::new(),
            _ => self.text(cx, row),
        }
    }

    /// The command that sets the cell of `glyph` to `text` (for a toggle, `text` is ignored).
    pub fn edit_command(self, row: &Row, text: &str) -> Option<(&'static str, Value)> {
        let glyph = row.glyph.name.as_str();
        let text = text.trim();
        let or_null = |t: &str| if t.is_empty() { Value::Null } else { json!(t) };
        Some(match self {
            Column::Name => ("glyph.rename", json!({"glyph": glyph, "newName": text})),
            Column::Unicode => {
                let values: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
                ("glyph.setUnicodes", json!({"glyphs": [glyph], "unicodes": values}))
            }
            Column::ProductionName => ("glyph.setProductionName", json!({"glyphs": [glyph], "productionName": or_null(text)})),
            Column::Script => ("glyph.setScript", json!({"glyphs": [glyph], "script": or_null(text)})),
            Column::Category => ("glyph.setCategory", json!({"glyphs": [glyph], "category": or_null(text)})),
            Column::SubCategory => ("glyph.setCategory", json!({"glyphs": [glyph], "subCategory": or_null(text)})),
            Column::Case => ("glyph.setCase", json!({"glyphs": [glyph], "case": or_null(text)})),
            Column::Direction => ("glyph.setDirection", json!({"glyphs": [glyph], "direction": or_null(text)})),
            Column::SortName => ("glyph.setSortName", json!({"glyphs": [glyph], "sortName": or_null(text)})),
            Column::Note => ("glyph.setNote", json!({"glyphs": [glyph], "note": text})),
            Column::Tags => {
                let tags: Vec<&str> = text.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
                ("glyph.setTags", json!({"glyphs": [glyph], "tags": tags}))
            }
            Column::Color => {
                let color = match text.parse::<u64>() {
                    Ok(i) => json!(i),
                    Err(_) => or_null(text),
                };
                ("glyph.setColorLabel", json!({"glyphs": [glyph], "color": color}))
            }
            Column::Export => ("glyph.setExport", json!({"glyphs": [glyph], "export": !row.glyph.export})),
            _ => return None,
        })
    }
}

/// The glyph's first code point as a character, unless it is a control character.
pub fn first_char(glyph: &Glyph) -> Option<char> {
    glyph.unicodes.first().and_then(|u| char::from_u32(*u)).filter(|c| !c.is_control())
}

/// Code points as four- to six-digit hexadecimal, space separated.
pub fn unicode_text(unicodes: &[u32]) -> String {
    unicodes.iter().map(|u| format!("{u:04X}")).collect::<Vec<_>>().join(" ")
}

pub fn color_name(color: ColorLabel) -> String {
    match color {
        ColorLabel::Index(i) => COLOR_LABEL_NAMES.get(usize::from(i)).map_or_else(|| i.to_string(), |n| (*n).to_string()),
        ColorLabel::Rgba([r, g, b, a]) => {
            let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02X}{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b), byte(a))
        }
    }
}

/// The glyphs the master layer uses as components, in order.
fn components<'g>(glyph: &'g Glyph, master: &str) -> Vec<&'g str> {
    glyph
        .layer(master)
        .map(|l| {
            l.shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Component(c) => Some(c.glyph.as_str()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

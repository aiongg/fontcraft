//! The As Exported section of the Exports tab (M7.32): what the export of the first selected entry writes, read only:
//! its file name, the `name` records, classes and style flags, the style-linked family with the exports linked in it,
//! and a variable font's named instances and axis values. What other active exports take too, and the tokens of the font
//! or a master that name nothing, show in the warning tone.

use craft::theme::variant::Tone;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::Ui;

use crate::app::models::exported_names::{ExportedModel, ExportedName};
use crate::app::models::font_exports::sentence;

craft::ui::words! {
    /// The words of the As Exported section.
    pub struct ExportedWords {
        title: "fontcraftExported.title" = "As Exported";
        file: "fontcraftExported.file" = "File Name";
        classes: "fontcraftExported.classes" = "Weight, Width Class";
        selection: "fontcraftExported.selection" = "Style Flags";
        group: "fontcraftExported.group" = "Style-linked Family";
        linked: "fontcraftExported.linked" = "Linked Exports";
        named: "fontcraftExported.named" = "Named Instances";
        stat: "fontcraftExported.stat" = "Axis Values";
        failed: "fontcraftExported.failed" = "This entry cannot be exported:";
        same_file: "fontcraftExported.sameFile" = "Same file name as";
        same_postscript: "fontcraftExported.samePostscript" = "Same PostScript name as";
        same_slot: "fontcraftExported.sameSlot" = "Same style-linked style as";
        /// The mark of an entry of the list.
        conflict: "fontcraftExported.conflict" = "Another active export has the same file name, PostScript name or style-linked style";
        copyright: "fontcraftExported.copyright" = "Copyright";
        family: "fontcraftExported.family" = "Family";
        subfamily: "fontcraftExported.subfamily" = "Subfamily";
        unique: "fontcraftExported.unique" = "Unique ID";
        full: "fontcraftExported.full" = "Full Name";
        version: "fontcraftExported.version" = "Version";
        postscript: "fontcraftExported.postscript" = "PostScript Name";
        trademark: "fontcraftExported.trademark" = "Trademark";
        manufacturer: "fontcraftExported.manufacturer" = "Manufacturer";
        designer: "fontcraftExported.designer" = "Designer";
        description: "fontcraftExported.description" = "Description";
        vendor_url: "fontcraftExported.vendorUrl" = "Manufacturer URL";
        designer_url: "fontcraftExported.designerUrl" = "Designer URL";
        license: "fontcraftExported.license" = "License";
        license_url: "fontcraftExported.licenseUrl" = "License URL";
        typo_family: "fontcraftExported.typoFamily" = "Typographic Family";
        typo_subfamily: "fontcraftExported.typoSubfamily" = "Typographic Subfamily";
        compatible_full: "fontcraftExported.compatibleFull" = "Compatible Full Name";
        sample: "fontcraftExported.sample" = "Sample Text";
        cid: "fontcraftExported.cid" = "CID findfont Name";
        wws_family: "fontcraftExported.wwsFamily" = "WWS Family";
        wws_subfamily: "fontcraftExported.wwsSubfamily" = "WWS Subfamily";
        light_palette: "fontcraftExported.lightPalette" = "Light Background Palette";
        dark_palette: "fontcraftExported.darkPalette" = "Dark Background Palette";
        prefix: "fontcraftExported.prefix" = "Variations PostScript Prefix";
        name: "fontcraftExported.name" = "Name";
    }
}

impl ExportedWords {
    fn name_label(&self, id: u64) -> &Label {
        match id {
            0 => &self.copyright,
            1 => &self.family,
            2 => &self.subfamily,
            3 => &self.unique,
            4 => &self.full,
            5 => &self.version,
            6 => &self.postscript,
            7 => &self.trademark,
            8 => &self.manufacturer,
            9 => &self.designer,
            10 => &self.description,
            11 => &self.vendor_url,
            12 => &self.designer_url,
            13 => &self.license,
            14 => &self.license_url,
            16 => &self.typo_family,
            17 => &self.typo_subfamily,
            18 => &self.compatible_full,
            19 => &self.sample,
            20 => &self.cid,
            21 => &self.wws_family,
            22 => &self.wws_subfamily,
            23 => &self.light_palette,
            24 => &self.dark_palette,
            25 => &self.prefix,
            _ => &self.name,
        }
    }

    /// `4 Full Name`, with the language of a record that is not the English Windows one: `4 Full Name DEU`.
    fn record(&self, r: &ExportedName) -> String {
        let label = format!("{} {}", r.id, self.name_label(r.id).as_str());
        if r.place.is_empty() { label } else { format!("{label} {}", r.place) }
    }
}

/// One read-only line.
fn line(ui: &mut Ui, label: impl Into<Label>, salt: (&str, usize), text: impl Into<Label>) {
    let text: Label = text.into();
    Field::new(label).id_salt(salt).show(ui, |ui, _| {
        Text::new(text.clone()).show(ui);
    });
}

fn warn(ui: &mut Ui, text: String) {
    Text::new(text).tone(Tone::Warn).show(ui);
}

pub fn view(ui: &mut Ui, model: &ExportedModel) {
    let words = Env::of(ui).words::<ExportedWords>();
    Section::new(words.title.clone()).id_salt("exportsAsExported").show(ui, |ui, _| {
        if let Some(error) = &model.error {
            warn(ui, format!("{} {}", words.failed.as_str(), sentence(error)));
            return;
        }
        for c in &model.conflicts {
            let label = match c.field.as_str() {
                "fileName" => &words.same_file,
                "postscriptName" => &words.same_postscript,
                _ => &words.same_slot,
            };
            warn(ui, format!("{} {}: {}", label.as_str(), c.with, c.value));
        }
        for token in &model.tokens {
            warn(ui, token.clone());
        }
        line(ui, words.file.clone(), ("exportedFile", 0), model.file.clone());
        if let (Some(weight), Some(width)) = (model.weight_class, model.width_class) {
            line(ui, words.classes.clone(), ("exportedClasses", 0), format!("{weight}, {width}"));
        }
        line(ui, words.selection.clone(), ("exportedSelection", 0), model.selection.join(", "));
        if let Some(group) = &model.group {
            line(ui, words.group.clone(), ("exportedGroup", 0), format!("{} ({})", group.family, group.style));
            for (at, (style, name)) in group.others.iter().enumerate() {
                line(ui, words.linked.clone(), ("exportedLinked", at), format!("{style}: {name}"));
            }
        }
        for (at, record) in model.names.iter().enumerate() {
            line(ui, words.record(record), ("exportedName", at), record.text.clone());
        }
        if !model.named.is_empty() {
            Text::new(words.named.clone()).shade(Shade::Muted).show(ui);
        }
        for (at, i) in model.named.iter().enumerate() {
            let text = if i.postscript.is_empty() { i.location.clone() } else { format!("{} ({})", i.location, i.postscript) };
            line(ui, i.name.clone(), ("exportedInstance", at), text);
        }
        if !model.stat.is_empty() {
            Text::new(words.stat.clone()).shade(Shade::Muted).show(ui);
        }
        for (at, (axis, values)) in model.stat.iter().enumerate() {
            line(ui, axis.clone(), ("exportedStat", at), values.clone());
        }
    });
}

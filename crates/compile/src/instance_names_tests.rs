//! Name records and style classes of static instances (M7.8): fontmake's names where it reads a
//! key, the handbook's where no reference compiler does.

use std::sync::Arc;

use fontcraft_model::plist::Value;
use fontcraft_model::{Font, Instance, Localized, Param, Path, Property, PropertyValue, Shape, StyleClass, Target};
use skrifa::FontRef;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::os2::SelectionFlags;

use super::*;

fn font() -> Font {
    let mut f = Font::new("Slice Sans");
    f.version_major = 2;
    f.version_minor = 5;
    let mut a = f.glyph_mut("A").unwrap();
    a.layers[0].width = 600.0;
    a.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
    drop(a);
    f
}

fn add(f: &mut Font, inst: Instance) -> Target {
    let list = Arc::make_mut(&mut f.instances);
    list.push(inst);
    Target::Instance(list.len() - 1)
}

fn instance(name: &str) -> Instance {
    Instance { name: name.into(), exports: true, ..Default::default() }
}

fn localized(key: &str, values: &[(&str, &str)]) -> Property {
    Property {
        key: key.into(),
        value: PropertyValue::Localized(
            values.iter().map(|(l, v)| Localized { language: (*l).into(), value: (*v).into(), extra: Default::default() }).collect(),
        ),
        extra: Default::default(),
    }
}

fn param(name: &str, v: Value) -> Param {
    Param::new(name, v)
}

fn text(s: &str) -> Value {
    Value::String(s.into())
}

struct Built {
    bytes: Vec<u8>,
    warnings: Vec<String>,
}

impl Built {
    fn font(&self) -> FontRef<'_> {
        FontRef::new(&self.bytes).unwrap()
    }

    /// Every record as (platform, encoding, language, id, text), sorted.
    fn records(&self) -> Vec<(u16, u16, u16, u16, String)> {
        let font = self.font();
        let name = font.name().unwrap();
        let data = name.string_data();
        let mut out: Vec<_> = name
            .name_record()
            .iter()
            .map(|r| {
                let text = r.string(data).unwrap().chars().collect::<String>();
                (r.platform_id(), r.encoding_id(), r.language_id(), r.name_id().to_u16(), text)
            })
            .collect();
        out.sort();
        out
    }

    /// The Windows text of `id` in `language`.
    fn name_in(&self, id: u16, language: u16) -> Option<String> {
        self.records().into_iter().find(|r| r.0 == 3 && r.2 == language && r.3 == id).map(|r| r.4)
    }

    fn name(&self, id: u16) -> Option<String> {
        self.name_in(id, 0x409)
    }

    fn selection(&self) -> SelectionFlags {
        self.font().os2().unwrap().fs_selection()
    }
}

fn build_with(f: &Font, target: &Target, options: &Options) -> Built {
    let out = compile(f, target, Flavour::TrueType, options).unwrap();
    Built { bytes: out.bytes, warnings: out.warnings }
}

fn build(f: &Font, target: &Target) -> Built {
    build_with(f, target, &Options::default())
}

#[test]
fn style_linking_gives_ids_1_and_2_and_the_style_bits_as_fontmake_does() {
    let mut f = font();
    let semibold_italic = add(
        &mut f,
        Instance {
            is_italic: true,
            weight_class: Some(StyleClass::Number(600.0)),
            width_class: Some(StyleClass::Name("Condensed".into())),
            ..instance("SemiBold Italic")
        },
    );
    let bold_unlinked = add(&mut f, instance("Bold"));
    let bold_linked = add(&mut f, Instance { is_bold: true, link_style: Some("Light".into()), ..instance("Bold") });

    let out = build(&f, &semibold_italic);
    assert_eq!(out.name(1).as_deref(), Some("Slice Sans SemiBold"));
    assert_eq!(out.name(2).as_deref(), Some("Italic"));
    assert_eq!(out.name(4).as_deref(), Some("Slice Sans SemiBold Italic"));
    assert_eq!(out.name(6).as_deref(), Some("SliceSans-SemiBoldItalic"));
    assert_eq!(out.name(16).as_deref(), Some("Slice Sans"));
    assert_eq!(out.name(17).as_deref(), Some("SemiBold Italic"));
    assert_eq!(out.selection() & (SelectionFlags::ITALIC | SelectionFlags::BOLD | SelectionFlags::REGULAR), SelectionFlags::ITALIC);
    assert_eq!(out.font().head().unwrap().mac_style().bits(), 2, "italic");
    let os2 = out.font().os2().unwrap();
    assert_eq!(os2.us_weight_class(), 600, "the instance's Weight Class without a weight axis");
    assert_eq!(os2.us_width_class(), 3, "the instance's Width Class without a width axis (the handbook)");

    // A style called Bold that is not linked as bold is a Regular of its own family.
    let out = build(&f, &bold_unlinked);
    assert_eq!((out.name(1).as_deref(), out.name(2).as_deref()), (Some("Slice Sans Bold"), Some("Regular")));
    assert!(out.selection().contains(SelectionFlags::REGULAR));
    assert_eq!(out.font().os2().unwrap().us_weight_class(), 400, "no Weight Class: Regular");

    // Linked to Light as its bold.
    let out = build(&f, &bold_linked);
    assert_eq!((out.name(1).as_deref(), out.name(2).as_deref()), (Some("Slice Sans Light"), Some("Bold")));
    assert_eq!(out.selection() & (SelectionFlags::BOLD | SelectionFlags::REGULAR), SelectionFlags::BOLD);
}

#[test]
fn an_instance_s_family_and_style_names_replace_the_font_s_in_every_language() {
    let mut f = font();
    let mut display = instance("Hairline");
    display.properties.0.push(localized("familyNames", &[("dflt", "Slice Display"), ("DEU", "Schnitt Anzeige")]));
    display.properties.0.push(localized("styleNames", &[("dflt", "Thin"), ("DEU", "Dünn")]));
    let display = add(&mut f, display);
    let mut spanish_only = instance("Regular");
    spanish_only.properties.0.push(localized("familyNames", &[("ESP", "Corte")]));
    let spanish_only = add(&mut f, spanish_only);

    let out = build(&f, &display);
    assert_eq!(out.name(1).as_deref(), Some("Slice Display Thin"));
    assert_eq!(out.name(16).as_deref(), Some("Slice Display"));
    assert_eq!(out.name(17).as_deref(), Some("Thin"));
    assert_eq!(out.name(6).as_deref(), Some("SliceDisplay-Thin"));
    assert_eq!(out.name_in(1, 0x407).as_deref(), Some("Schnitt Anzeige"));
    assert_eq!(out.name_in(17, 0x407).as_deref(), Some("Dünn"));

    // Only a default-language family groups the fonts: a Spanish one alone leaves ID 1 alone.
    let out = build(&f, &spanish_only);
    assert_eq!(out.name(1).as_deref(), Some("Slice Sans"));
    assert_eq!(out.name_in(1, 0x0C0A).as_deref(), Some("Corte"));
}

#[test]
fn style_map_keys_postscript_full_name_and_the_names_fontc_leaves_out() {
    let mut f = font();
    let mut inst = instance("Fancy");
    inst.properties.set("styleMapFamilyNames", "Slice Linked", true);
    inst.properties.set("postscriptFullNames", "Slice Sans Fancy Display", true);
    inst.properties.set("compatibleFullNames", "Slice Fancy", true);
    inst.properties.set("WWSSubfamilyName", "Fancy", false);
    inst.params.0.push(param("styleMapStyleName", text("bold italic")));
    let target = add(&mut f, inst);
    let out = build(&f, &target);
    assert_eq!(out.name(1).as_deref(), Some("Slice Linked"));
    assert_eq!(out.name(2).as_deref(), Some("Bold Italic"));
    assert!(out.selection().contains(SelectionFlags::BOLD | SelectionFlags::ITALIC));
    assert_eq!(out.name(4).as_deref(), Some("Slice Sans Fancy Display"));
    assert_eq!(out.name(18).as_deref(), Some("Slice Fancy"));
    assert_eq!(out.name(22).as_deref(), Some("Fancy"));

    // A style-map style name outside the four is ignored, with a warning.
    let mut f = font();
    let mut inst = instance("Fancy");
    inst.params.0.push(param("styleMapStyleName", text("heavy")));
    let target = add(&mut f, inst);
    let out = build(&f, &target);
    assert_eq!(out.name(2).as_deref(), Some("Regular"));
    assert!(out.warnings.iter().any(|w| w.contains("`heavy`")), "{:?}", out.warnings);
}

#[test]
fn name_table_entries_add_up_over_the_levels_and_prevented_ids_are_left_out() {
    let mut f = font();
    let info = Arc::make_mut(&mut f.info);
    info.params.0.push(param("Name Table Entry", text("25; Font level")));
    info.params.0.push(param("Name Table Entry", text("26; Replaced")));
    info.params.0.push(param("Prevent Name ID", text("16")));
    let mut inst = instance("Light Italic");
    inst.params.0.push(param("Name Table Entry", text("26; Instance level")));
    inst.params.0.push(param("Prevent Name ID", text("17")));
    let target = add(&mut f, inst);
    let out = build(&f, &target);
    assert_eq!(out.name(25).as_deref(), Some("Font level"));
    assert_eq!(out.name(26).as_deref(), Some("Instance level"));
    assert_eq!((out.name(16), out.name(17)), (None, None));
    assert_eq!(out.name(1).as_deref(), Some("Slice Sans Light Italic"));

    // Preventing a name the font needs is allowed, with a warning.
    let mut f = font();
    Arc::make_mut(&mut f.info).params.0.push(param("Prevent Name ID", text("6")));
    let out = build(&f, &Target::Master("m01".into()));
    assert_eq!(out.name(6), None);
    assert!(out.warnings.iter().any(|w| w.contains("name ID 6")), "{:?}", out.warnings);
}

#[test]
fn the_version_string_gets_the_stamp_unless_the_font_states_its_own() {
    let mut f = font();
    let plain = add(&mut f, instance("Regular"));
    let stamped = Options { version_stamp: true, ..Options::default() };
    let out = build_with(&f, &plain, &stamped);
    assert_eq!(out.name(5), Some(format!("Version 2.005{}", info::version_stamp())));
    assert_eq!(out.name(3).as_deref(), Some("2.005;NONE;SliceSans-Regular"), "the unique id has no stamp");
    assert_eq!(build(&f, &plain).name(5).as_deref(), Some("Version 2.005"));

    let mut inst = instance("Regular");
    inst.properties.set("versionString", "Version %d.%03d;beta", false);
    let template = add(&mut f, inst);
    assert_eq!(build_with(&f, &template, &stamped).name(5).as_deref(), Some("Version 2.005;beta"));

    let mut inst = instance("Regular");
    inst.properties.set("versionString", "Version %d.%d.%d", false);
    let three = add(&mut f, inst);
    let out = build(&f, &three);
    assert_eq!(out.name(5).as_deref(), Some("Version %d.%d.%d"));
    assert!(out.warnings.iter().any(|w| w.contains("`%` form")), "{:?}", out.warnings);
}

#[test]
fn macintosh_records_are_written_only_when_asked_for() {
    let mut f = font();
    Arc::make_mut(&mut f.info).properties.set("designers", "李", true);
    let target = add(&mut f, instance("Bold Italic"));
    assert!(build(&f, &target).records().iter().all(|r| r.0 == 3), "Windows records only by default");

    Arc::make_mut(&mut f.info).params.0.push(param("Export Mac Name Table Entries", Value::Bool(true)));
    let out = build(&f, &target);
    let mac: Vec<_> = out.records().into_iter().filter(|r| r.0 == 1).collect();
    assert!(mac.iter().all(|r| (r.1, r.2) == (0, 0)), "Roman encoding, English");
    assert!(mac.iter().any(|r| r.3 == 1 && Some(&r.4) == out.name(1).as_ref()), "{mac:?}");
    assert!(!mac.iter().any(|r| r.3 == 9), "Mac Roman cannot hold the designer's name");

    // The compatible table: the full name in ID 1, the subfamily in ID 2.
    let mut f = font();
    Arc::make_mut(&mut f.info).params.0.push(param("Compatible name Table", Value::Bool(true)));
    let target = add(&mut f, instance("Light"));
    let mac: Vec<_> = build(&f, &target).records().into_iter().filter(|r| r.0 == 1).collect();
    assert!(mac.iter().any(|r| r.3 == 1 && r.4 == "Slice Sans Light"), "{mac:?}");
    assert!(mac.iter().any(|r| r.3 == 2 && r.4 == "Light"), "{mac:?}");
    Arc::make_mut(&mut f.info).params.0.push(param("Export Mac Name Table Entries", Value::Bool(false)));
    assert!(build(&f, &target).records().iter().all(|r| r.0 == 3), "turned off explicitly");
}

#[test]
fn text_tokens_in_names_are_expanded_for_the_instance() {
    let mut f = font();
    let mut inst = instance("Bold");
    inst.properties.set("designers", "{{{name}}} team", true);
    inst.properties.set("descriptions", "{{{noSuchKey}}}", true);
    let target = add(&mut f, inst);
    let out = build(&f, &target);
    assert_eq!(out.name(9).as_deref(), Some("Bold team"));
    assert_eq!(out.name(10).as_deref(), Some("{{{noSuchKey}}}"));
    assert!(out.warnings.iter().any(|w| w.contains("`noSuchKey`")), "{:?}", out.warnings);
}

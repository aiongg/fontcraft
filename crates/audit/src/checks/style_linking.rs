//! `check.names.recommendedStyleLinking` (M12.64b): the source's static instances against the
//! style-linking scheme the handbook recommends (`fontInfo.styleLinking.recommendedScheme`).
//!
//! - Bold, Italic and Bold Italic link to Regular with the matching bold and italic flags.
//! - Every other italic links, as its italic, to the upright of the same name without `Italic`.
//! - Any other instance marked bold hides itself from family pickers that show four styles.
//!
//! Only exporting static instances count, and an upright or Regular that the source does not
//! have is not asked for. Names compare without case and surrounding space.

use fontcraft_model::Instance;

use crate::{Hit, Location, Params, Sink, SourceCtx};

/// Most instances one run looks at; a source rarely has more than a few dozen.
const MAX_INSTANCES: usize = 4096;
/// The field a finding names, so the panel opens the Exports tab.
const FIELD: &str = "instance.linkStyle";

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The upright an italic style name stands for: `Medium Italic` → `Medium`.
fn upright_of(name: &str) -> Option<&str> {
    let name = name.trim();
    let cut = name.len().checked_sub(" Italic".len())?;
    let (base, tail) = (name.get(..cut)?, name.get(cut..)?);
    same(tail, " Italic").then_some(base.trim()).filter(|b| !b.is_empty())
}

fn hit(inst: &Instance, code: &'static str, message: String) -> Hit {
    Hit::new(code, message).place(Location { instance: Some(inst.name.clone()), source_field: Some(FIELD.into()), ..Location::default() })
}

fn link_text(inst: &Instance) -> String {
    let flags = match (inst.is_bold, inst.is_italic) {
        (true, true) => "the bold italic",
        (true, false) => "the bold",
        (false, true) => "the italic",
        (false, false) => return "not linked".into(),
    };
    match inst.link_style.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(base) => format!("linked as {flags} of `{base}`"),
        None => format!("marked as {flags} without a base style"),
    }
}

pub(crate) fn recommended_style_linking(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let statics: Vec<&Instance> = ctx.audit.font.instances.iter().filter(|i| i.exports && i.kind.is_none()).take(MAX_INSTANCES).collect();
    let has = |name: &str| statics.iter().any(|i| same(&i.name, name));
    let links_to = |inst: &Instance, base: &str| inst.link_style.as_deref().is_some_and(|l| same(l, base));
    for inst in &statics {
        let name = inst.name.trim();
        let ribbi = [("Bold", true, false), ("Italic", false, true), ("Bold Italic", true, true)].into_iter().find(|(n, _, _)| same(name, n));
        if let Some((style, bold, italic)) = ribbi {
            if has("Regular") && !(inst.is_bold == bold && inst.is_italic == italic && links_to(inst, "Regular")) {
                let flags = match (bold, italic) {
                    (true, true) => "bold italic",
                    (true, false) => "bold",
                    _ => "italic",
                };
                sink.push(hit(
                    inst,
                    "ribbi-not-linked-to-regular",
                    format!("`{style}` is {}; link it as the {flags} of `Regular`.", link_text(inst)),
                ));
            }
            continue;
        }
        if inst.is_bold {
            sink.push(hit(
                inst,
                "bold-hides-style",
                format!("`{name}` is {}, which hides it from family pickers that show only four styles.", link_text(inst)),
            ));
            continue;
        }
        if let Some(upright) = upright_of(name)
            && has(upright)
            && !(inst.is_italic && links_to(inst, upright))
        {
            sink.push(hit(inst, "italic-not-linked-to-upright", format!("`{name}` is {}; link it as the italic of `{upright}`.", link_text(inst))));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use std::sync::Arc;

    use fontcraft_model::{Font, Instance};

    use crate::{AuditContext, Config, NoServices, SourceCtx, audit_source, find};

    fn instance(name: &str, bold: bool, italic: bool, link: Option<&str>) -> Instance {
        Instance { name: name.into(), is_bold: bold, is_italic: italic, link_style: link.map(str::to_string), ..Instance::default() }
    }

    fn findings(instances: Vec<Instance>) -> Vec<(String, &'static str)> {
        let mut font = Font::new("Test");
        font.instances = Arc::new(instances);
        let audit = AuditContext::new(&font);
        let report = audit_source(
            &SourceCtx { audit: &audit, services: &NoServices },
            &[find("check.names.recommendedStyleLinking").unwrap()],
            &Config::default(),
        );
        report.findings.iter().map(|f| (f.location.instance.clone().unwrap_or_default(), f.code)).collect()
    }

    #[test]
    fn recommended_style_linking_follows_the_handbook_scheme() {
        let good = vec![
            instance("Light", false, false, None),
            instance("Light Italic", false, true, Some("Light")),
            instance("Regular", false, false, None),
            instance("Italic", false, true, Some("Regular")),
            instance("Bold", true, false, Some("Regular")),
            instance("Bold Italic", true, true, Some("Regular")),
        ];
        assert!(findings(good).is_empty());

        let bad = vec![
            instance("Light", false, false, None),
            instance("Light Italic", false, false, None),
            instance("Regular", false, false, None),
            instance("Semibold", true, false, Some("Light")),
            instance("Bold", true, false, Some("Light")),
            instance("Bold Italic", true, false, Some("Regular")),
        ];
        assert_eq!(
            findings(bad),
            vec![
                ("Light Italic".to_string(), "italic-not-linked-to-upright"),
                ("Semibold".to_string(), "bold-hides-style"),
                ("Bold".to_string(), "ribbi-not-linked-to-regular"),
                ("Bold Italic".to_string(), "ribbi-not-linked-to-regular"),
            ]
        );

        // Without a Regular or an upright of the same name, nothing is asked for; variable
        // settings and instances that do not export are left out.
        let mut variable = instance("Semibold", true, false, Some("Light"));
        variable.kind = Some("variable".into());
        let mut off = instance("Medium", true, false, None);
        off.exports = false;
        assert!(findings(vec![instance("Bold", false, false, None), instance("Condensed Italic", false, false, None), variable, off]).is_empty());
    }
}

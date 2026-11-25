//! The problems of a font's axes, masters, virtual masters, variable font settings and instances, each with a code,
//! what it names and one sentence. Nothing here refuses: a font whose designspace cannot be built gets
//! [`UNUSABLE`].
//!
//! Rows: `interpolation.describeDesignspace`, `masters.mustCoverDesignspace`, `masters.minimalSetup`.

use fontcraft_model::{Font, axes};

use crate::designspace::{Designspace, Location, variable_settings};
use crate::instance::instances_beyond_masters;
use crate::{InterpError, MAX_INSTANCES, MAX_SOURCES, params};

/// Most problems one call reports.
pub const MAX_PROBLEMS: usize = 200;

/// The designspace of the font, or of one variable font setting, cannot be built.
pub const UNUSABLE: &str = "designspace-unusable";
/// Two masters at one location.
pub const SHARED_LOCATION: &str = "masters-share-location";
/// A virtual master where a master is.
pub const VIRTUAL_SHARES_LOCATION: &str = "virtual-master-shares-location";
/// A Virtual Master parameter names an axis the font does not have.
pub const VIRTUAL_UNKNOWN_AXIS: &str = "virtual-master-unknown-axis";
/// Every master and virtual master sits at one value of an axis.
pub const AXIS_WITHOUT_RANGE: &str = "axis-without-range";
/// An axis mapping that neither rises nor falls.
pub const MAPPING_IGNORED: &str = "axis-mapping-ignored";
/// A variable font cannot start from the origin master.
pub const ORIGIN_NOT_MINIMAL: &str = "origin-not-minimal";
/// Every variable font setting disables a master.
pub const MASTER_NOT_EXPORTED: &str = "master-not-exported";
/// Disable Masters names a master the font does not have.
pub const UNKNOWN_DISABLED_MASTER: &str = "unknown-disabled-master";
/// An exported instance beyond the masters.
pub const INSTANCE_EXTRAPOLATED: &str = "instance-extrapolated";

/// Every code.
pub const CODES: [&str; 10] = [
    UNUSABLE,
    SHARED_LOCATION,
    VIRTUAL_SHARES_LOCATION,
    VIRTUAL_UNKNOWN_AXIS,
    AXIS_WITHOUT_RANGE,
    MAPPING_IGNORED,
    ORIGIN_NOT_MINIMAL,
    MASTER_NOT_EXPORTED,
    UNKNOWN_DISABLED_MASTER,
    INSTANCE_EXTRAPOLATED,
];

/// How close two coordinates are to count as one, relative to their size.
const SAME: f64 = 1e-9;

/// One problem.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Problem {
    /// One of [`CODES`].
    pub code: &'static str,
    /// One sentence.
    pub message: String,
    /// The masters concerned, as indices into `Font::masters`.
    pub masters: Vec<usize>,
    /// The instances or variable font settings concerned, as indices into `Font::instances`.
    pub instances: Vec<usize>,
    /// The axis concerned, as an index into `Font::axes`.
    pub axis: Option<usize>,
}

impl Problem {
    fn new(code: &'static str, message: String) -> Self {
        Problem { code, message, ..Problem::default() }
    }

    fn masters(self, masters: Vec<usize>) -> Self {
        Problem { masters, ..self }
    }

    fn instances(self, instances: Vec<usize>) -> Self {
        Problem { instances, ..self }
    }

    fn axis(self, axis: usize) -> Self {
        Problem { axis: Some(axis), ..self }
    }
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= SAME * (1.0 + a.abs().max(b.abs()))
}

fn same_place(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(*a, *b))
}

/// The problems of `font`, at most [`MAX_PROBLEMS`]: masters and axes first, then the variable font settings, then the
/// instances.
pub fn problems(font: &Font) -> Vec<Problem> {
    let mut out = shared_locations(font);
    match Designspace::new(font) {
        Ok(ds) => {
            axes_without_range(&ds, &mut out);
            out.extend(ds.warnings().iter().map(|w| Problem::new(MAPPING_IGNORED, sentence(w))));
            virtual_masters(font, &ds, &mut out);
            if ds.masters().len() > 1
                && let Err(e) = ds.check_variable_origin()
            {
                let origin = ds.origin().map(|m| m.index).into_iter().collect();
                out.push(Problem::new(ORIGIN_NOT_MINIMAL, format!("A variable font cannot be built: {}.", reason(&e))).masters(origin));
            }
            settings(font, &mut out);
        }
        Err(e) => out.push(Problem::new(UNUSABLE, format!("The axes and masters cannot interpolate: {}.", reason(&e)))),
    }
    extrapolated(font, &mut out);
    out.truncate(MAX_PROBLEMS);
    out
}

/// An error as the part of a sentence after a colon.
fn reason(e: &InterpError) -> String {
    match e {
        InterpError::BadMasterSetup(why) => why.clone(),
        other => other.to_string(),
    }
}

/// A warning as a sentence.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |c| format!("{}{}.", c.to_uppercase(), chars.as_str()))
}

/// The masters that sit where an earlier one does, each with the first there.
fn shared_locations(font: &Font) -> Vec<Problem> {
    let axes = font.axes.len().max(1);
    let mut firsts: Vec<(usize, Vec<f64>)> = Vec::new();
    let mut out = Vec::new();
    for (i, m) in font.masters.iter().enumerate().take(MAX_SOURCES) {
        let place: Vec<f64> = (0..axes).map(|k| axes::design_value(&m.location, k)).collect();
        match firsts.iter().find(|(_, p)| same_place(p, &place)) {
            Some(&(first, _)) => {
                let name = font.masters.get(first).map_or("", |m| m.name.as_str());
                let message = format!("The masters {name} and {} sit at the same location.", m.name);
                out.push(Problem::new(SHARED_LOCATION, message).masters(vec![first, i]));
                if out.len() >= MAX_PROBLEMS {
                    break;
                }
            }
            None => firsts.push((i, place)),
        }
    }
    out
}

fn axes_without_range(ds: &Designspace, out: &mut Vec<Problem>) {
    for a in ds.axes().iter().filter(|a| same(a.internal.minimum, a.internal.maximum)) {
        let message = format!("The {} axis has no range: every master sits at {}.", a.name, number(a.internal.default));
        out.push(Problem::new(AXIS_WITHOUT_RANGE, message).axis(a.font_axis));
    }
}

/// A number in its shortest form, rounded to three decimals.
fn number(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    format!("{}", if r == 0.0 { 0.0 } else { r })
}

fn virtual_masters(font: &Font, ds: &Designspace, out: &mut Vec<Problem>) {
    for v in ds.virtual_masters() {
        let there = |l: &Location| same_place(&l.0, &v.location.0);
        if let Some(m) = ds.masters().iter().find(|m| there(&m.location)) {
            let message = format!("{} sits where the master {} does.", v.name, m.name);
            out.push(Problem::new(VIRTUAL_SHARES_LOCATION, message).masters(vec![m.index]));
        }
    }
    for (n, entry) in params::virtual_masters(font).unwrap_or_default().iter().enumerate() {
        for (axis, _) in entry.coordinates.iter().filter(|(name, _)| !font.axes.iter().any(|a| a.name == *name)) {
            let message = format!("Virtual Master {} names the axis {axis}, which the font does not have.", n.saturating_add(1));
            out.push(Problem::new(VIRTUAL_UNKNOWN_AXIS, message));
        }
    }
}

/// What the variable font settings leave out, and whether each can still be built.
fn settings(font: &Font, out: &mut Vec<Problem>) {
    let all = variable_settings(font);
    let disabled: Vec<(usize, Vec<String>)> =
        all.iter().filter_map(|&i| Some((i, params::disabled_masters(font.instances.get(i)?)))).take(MAX_INSTANCES).collect();
    for (i, names) in disabled.iter().filter(|(_, names)| !names.is_empty()) {
        if out.len() >= MAX_PROBLEMS {
            return;
        }
        let setting = font.instances.get(*i).map_or("", |s| s.name.as_str());
        for name in names.iter().filter(|n| !font.masters.iter().any(|m| m.name == **n)) {
            let message = format!("{setting}: Disable Masters names {name}, which is not a master.");
            out.push(Problem::new(UNKNOWN_DISABLED_MASTER, message).instances(vec![*i]));
        }
        match Designspace::for_setting(font, *i) {
            Err(e) => out.push(Problem::new(UNUSABLE, format!("{setting} cannot be built: {}.", reason(&e))).instances(vec![*i])),
            Ok(ds) => {
                if ds.masters().len() > 1
                    && let Err(e) = ds.check_variable_origin()
                {
                    let message = format!("{setting} cannot be built as a variable font: {}.", reason(&e));
                    out.push(Problem::new(ORIGIN_NOT_MINIMAL, message).instances(vec![*i]));
                }
            }
        }
    }
    if disabled.is_empty() {
        return;
    }
    for (k, m) in font.masters.iter().enumerate().take(MAX_SOURCES) {
        if disabled.iter().all(|(_, names)| names.contains(&m.name)) {
            let message = format!("Every variable font setting leaves the master {} out (Disable Masters).", m.name);
            out.push(Problem::new(MASTER_NOT_EXPORTED, message).masters(vec![k]).instances(disabled.iter().map(|(i, _)| *i).collect()));
        }
    }
}

/// The exported instances beyond the masters.
fn extrapolated(font: &Font, out: &mut Vec<Problem>) {
    if font.masters.len() < 2 {
        return;
    }
    let exported: Vec<usize> = font.instances.iter().enumerate().take(MAX_INSTANCES).filter(|(_, i)| i.exports).map(|(i, _)| i).collect();
    for (&i, beyond) in exported.iter().zip(instances_beyond_masters(font, &exported)) {
        let Some(&first) = beyond.first() else { continue };
        let names: Vec<&str> = beyond.iter().filter_map(|&k| font.axes.get(k)).map(|a| a.name.as_str()).collect();
        let name = font.instances.get(i).map_or("", |inst| inst.name.as_str());
        let message = format!("The instance {name} lies beyond the masters on {}; it is extrapolated.", names.join(", "));
        out.push(Problem::new(INSTANCE_EXTRAPOLATED, message).instances(vec![i]).axis(first));
        if out.len() >= MAX_PROBLEMS {
            return;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Instance;

    use super::*;
    use crate::designspace::tests::font;

    fn codes(font: &Font) -> Vec<&'static str> {
        problems(font).iter().map(|p| p.code).collect()
    }

    /// A duplicated master is named with its original until it moves; the axis nothing moves on, the master away from
    /// the origin on both axes, the master every setting leaves out and the instance beyond the masters are named too.
    #[test]
    fn each_problem_is_named_and_goes_when_it_is_mended() {
        let mut f = font(&[("Weight", "wght"), ("Width", "wdth")], &[("Thin", &[100.0, 100.0]), ("Black", &[900.0, 100.0])]);
        f.instances = Arc::new(Vec::new());
        assert_eq!(codes(&f), [AXIS_WITHOUT_RANGE], "{:?}", problems(&f));
        assert_eq!(problems(&f)[0].axis, Some(1));

        let masters = Arc::make_mut(&mut f.masters);
        let mut copy = masters[1].clone();
        copy.name = "Black Copy".into();
        copy.id = "copy".into();
        masters.push(copy);
        let found = problems(&f);
        assert_eq!(found[0].code, SHARED_LOCATION);
        assert!(found[0].message.contains("Black and Black Copy") && found[0].masters == [1, 2], "{found:?}");

        Arc::make_mut(&mut f.masters)[2].location = vec![900.0, 60.0];
        assert_eq!(codes(&f), [ORIGIN_NOT_MINIMAL], "no master differs from the origin on Width alone");
        Arc::make_mut(&mut f.masters)[2].location = vec![100.0, 60.0];
        assert_eq!(codes(&f), [] as [&str; 0]);

        f.instances = Arc::new(vec![
            Instance { name: "Ultra".into(), location: vec![1000.0, 100.0], exports: true, ..Instance::default() },
            Instance { name: "VF".into(), kind: Some("variable".into()), exports: true, ..Instance::default() },
        ]);
        let setting = &mut Arc::make_mut(&mut f.instances)[1];
        setting.params = params::set_disabled_masters(&setting.params, &["Black Copy".into(), "Gone".into()]);
        let found = problems(&f);
        assert_eq!(codes(&f), [UNKNOWN_DISABLED_MASTER, MASTER_NOT_EXPORTED, INSTANCE_EXTRAPOLATED], "{found:?}");
        assert_eq!((found[1].masters.as_slice(), found[2].instances.as_slice()), ([2].as_slice(), [0].as_slice()));
        assert!(found.iter().all(|p| CODES.contains(&p.code) && p.message.ends_with('.')), "{found:?}");
    }

    /// A designspace that cannot be built is one problem, beside what can still be told.
    #[test]
    fn an_unusable_designspace_is_a_problem() {
        let f = font(&[("Weight", "wght"), ("Other", "wght")], &[("Thin", &[100.0, 0.0]), ("Thin Copy", &[100.0, 0.0])]);
        assert!(codes(&f).starts_with(&[SHARED_LOCATION, UNUSABLE]), "{:?}", problems(&f));
    }
}

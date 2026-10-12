//! The `fvar` table (M9.6).
//!
//! Rows: the `fvar` table row, `compile.fvarInstances`, `instances.namedInstancesInVariable`.
//!
//! Contract: one axis record per [`VarAxis`](super::VarAxis) (hidden axes flagged), one instance
//! record per named instance with its subfamily name and, when the font has any PostScript
//! names for instances, a PostScript name ID; at most 65,535 instances. Names come from the
//! shared [`NameAllocator`].
//!
//! The order of name allocation follows fontc (`fontir` `StaticMetadata::new` and `fontbe`
//! `fvar.rs`, Apache-2.0): every axis name first, then for each instance its subfamily name and
//! its PostScript name. The instance at the default location reuses name ID 2 or 17 when that
//! record holds its name. When one instance has a PostScript name, the others get the "no name"
//! ID 0xFFFF.

use fontcraft_interp::Designspace;
use fontcraft_model::Font;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::types::{Fixed, NameId};

use super::names::{NameAllocator, instance_fvar_name, instance_postscript_name};
use super::{VarAxes, VarInstance};
use crate::CompileError;

/// The most named instances `fvar` holds (its count is 16 bits).
pub const MAX_NAMED_INSTANCES: usize = u16::MAX as usize;

const HIDDEN_AXIS: u16 = 0x0001;
const NO_POSTSCRIPT_NAME: NameId = NameId::new(0xFFFF);

fn fvar_error(message: String) -> CompileError {
    CompileError::Table { table: "fvar", message }
}

/// A user coordinate as `Fixed`, refused when it is not finite or does not fit.
fn fixed(value: f64, what: &str) -> Result<Fixed, CompileError> {
    if !value.is_finite() || value.abs() >= 32_768.0 {
        return Err(fvar_error(format!("{what} {value} is not a usable axis coordinate")));
    }
    Ok(Fixed::from_f64(value))
}

/// The `fvar` table of `axes` and `instances`.
pub fn build_fvar(axes: &VarAxes, instances: &[VarInstance], names: &mut NameAllocator) -> Result<Fvar, CompileError> {
    if axes.axes.is_empty() {
        return Err(fvar_error("a variable font needs at least one axis".into()));
    }
    if axes.axes.len() > usize::from(u16::MAX) {
        return Err(fvar_error(format!("{} axes are more than the table holds", axes.axes.len())));
    }
    if instances.len() > MAX_NAMED_INSTANCES {
        return Err(fvar_error(format!("{} named instances are more than the table holds ({MAX_NAMED_INSTANCES})", instances.len())));
    }

    let mut axis_records = Vec::with_capacity(axes.axes.len());
    for axis in &axes.axes {
        let min = fixed(axis.min, "the minimum of axis")?;
        let default = fixed(axis.default, "the default of axis")?;
        let max = fixed(axis.max, "the maximum of axis")?;
        if !(min <= default && default <= max) {
            return Err(fvar_error(format!("axis `{}` has its default outside its range", axis.tag)));
        }
        let axis_name_id = names.id_for(&axis.name)?;
        let flags = if axis.hidden { HIDDEN_AXIS } else { 0 };
        axis_records.push(VariationAxisRecord { axis_tag: axis.tag, min_value: min, default_value: default, max_value: max, flags, axis_name_id });
    }

    let defaults: Vec<Fixed> = axis_records.iter().map(|a| a.default_value).collect();
    let has_postscript_names = instances.iter().any(|i| i.postscript_name.is_some());
    let mut instance_records = Vec::with_capacity(instances.len());
    for instance in instances {
        if instance.coordinates.len() != axes.axes.len() {
            return Err(fvar_error(format!(
                "instance `{}` has {} coordinates for {} axes",
                instance.name,
                instance.coordinates.len(),
                axes.axes.len()
            )));
        }
        let coordinates = instance.coordinates.iter().map(|c| fixed(*c, "a coordinate of instance")).collect::<Result<Vec<_>, _>>()?;
        let subfamily_name_id =
            if coordinates == defaults { names.id_for_default_instance(&instance.name)? } else { names.id_for(&instance.name)? };
        let post_script_name_id = if has_postscript_names {
            Some(match &instance.postscript_name {
                Some(ps) => names.id_for(ps)?,
                None => NO_POSTSCRIPT_NAME,
            })
        } else {
            None
        };
        instance_records.push(InstanceRecord { subfamily_name_id, flags: 0, coordinates, post_script_name_id });
    }

    Ok(Fvar::new(AxisInstanceArrays::new(axis_records, instance_records)))
}

/// The named instances of the variable font of `designspace`, at their external coordinates,
/// in the order [`Designspace::named_instances`] picks them.
pub fn named_instances(font: &Font, designspace: &Designspace) -> Result<Vec<VarInstance>, CompileError> {
    let picked = designspace.named_instances(font);
    if picked.len() > MAX_NAMED_INSTANCES {
        return Err(fvar_error(format!("more than {MAX_NAMED_INSTANCES} named instances")));
    }
    let mut out = Vec::with_capacity(picked.len());
    for index in picked {
        let Some(instance) = font.instances.get(index) else { continue };
        let user = designspace.to_external(&designspace.instance_location(font, index)?)?;
        out.push(VarInstance { name: instance_fvar_name(instance), postscript_name: instance_postscript_name(instance), coordinates: user.0 });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use write_fonts::dump_table;
    use write_fonts::types::Tag;

    use super::super::VarAxis;
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn wght(min: f64, default: f64, max: f64) -> VarAxes {
        VarAxes { axes: vec![VarAxis { tag: Tag::new(b"wght"), name: "Weight".into(), min, default, max, hidden: false, avar: Vec::new() }] }
    }

    fn instance(name: &str, ps: Option<&str>, w: f64) -> VarInstance {
        VarInstance { name: name.into(), postscript_name: ps.map(str::to_string), coordinates: vec![w] }
    }

    fn static_names() -> NameAllocator {
        NameAllocator::with_static([(NameId::new(1), "Fam".to_string()), (NameId::new(2), "Regular".to_string())])
    }

    // Expected bytes: fontc 1.0.0 on its test sources `WghtVar.glyphs`, `WghtVar_Avar.glyphs`
    // and `InstancePostscript.glyphs`.
    #[test]
    fn fvar_matches_the_oracle_without_instances() {
        let fvar = build_fvar(&wght(400.0, 400.0, 700.0), &[], &mut static_names()).unwrap();
        assert_eq!(hex(&dump_table(&fvar).unwrap()), "0001000000100002000100140000000877676874019000000190000002bc000000000100");
    }

    #[test]
    fn fvar_matches_the_oracle_with_a_default_instance() {
        let mut names = static_names();
        let instances =
            [instance("Light", None, 300.0), instance("Regular", None, 400.0), instance("Medium", None, 500.0), instance("Bold", None, 700.0)];
        let fvar = build_fvar(&wght(300.0, 400.0, 700.0), &instances, &mut names).unwrap();
        assert_eq!(
            hex(&dump_table(&fvar).unwrap()),
            "0001000000100002000100140004000877676874012c00000190000002bc00000000010001010000012c000000020000019000000102000001f400000103000002bc0000"
        );
        let texts: Vec<&str> = names.records().iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, ["Weight", "Light", "Medium", "Bold"]);
    }

    #[test]
    fn fvar_matches_the_oracle_with_postscript_names() {
        let mut names = static_names();
        let instances = [instance("Regular", None, 400.0), instance("Medium", Some("PostMedium"), 500.0), instance("Bold", Some("PostBold"), 700.0)];
        let fvar = build_fvar(&wght(400.0, 400.0, 700.0), &instances, &mut names).unwrap();
        assert_eq!(
            hex(&dump_table(&fvar).unwrap()),
            "0001000000100002000100140003000a77676874019000000190000002bc0000000001000002000001900000ffff0101000001f4000001020103000002bc00000104"
        );
    }

    #[test]
    fn fvar_flags_hidden_axes_and_refuses_bad_input() {
        let mut axes = wght(100.0, 400.0, 900.0);
        if let Some(a) = axes.axes.first_mut() {
            a.hidden = true;
        }
        let fvar = build_fvar(&axes, &[], &mut NameAllocator::new()).unwrap();
        assert_eq!(fvar.axis_instance_arrays.axes.first().map(|a| a.flags), Some(HIDDEN_AXIS));
        assert!(build_fvar(&VarAxes::default(), &[], &mut NameAllocator::new()).is_err());
        assert!(build_fvar(&wght(500.0, 400.0, 900.0), &[], &mut NameAllocator::new()).is_err());
        assert!(build_fvar(&wght(100.0, f64::NAN, 900.0), &[], &mut NameAllocator::new()).is_err());
        let two = VarInstance { coordinates: vec![400.0, 100.0], ..instance("Two", None, 0.0) };
        assert!(build_fvar(&wght(100.0, 400.0, 900.0), &[two], &mut NameAllocator::new()).is_err());
    }
}

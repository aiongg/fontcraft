//! Guide Properties: the properties of one guide, over `guide.setProperties`, previewed on the canvas. A double-click
//! on a guide's line with the Select tool opens it with the guide's address and present values
//! (`fontcraft_tools::guide_drag::properties`); the field defaults apply only when an agent opens it without them.

use fontcraft_engine::model::MAX_COORDINATE;

use super::fields::Field;
use super::spec::{DialogSpec, Fields};

pub const SPECS: &[DialogSpec] =
    &[DialogSpec::new(fontcraft_tools::guide_drag::PROPERTIES_DIALOG, "Guide Properties", "guide.setProperties", Fields::Fixed(fields))];

/// Longest name or colour the fields take (the command takes longer ones).
const MAX_TEXT: usize = 256;

/// The sidebearing references with their menu labels.
const RELATIVE_TO: [(&str, &str); 3] = [("left", "Left Sidebearing"), ("right", "Right Sidebearing"), ("both", "Both Sidebearings")];

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![
        Field::number("x", "X", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "The knob's horizontal position in font units, rounded to the grid"),
        Field::number("y", "Y", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "The knob's vertical position in font units, rounded to the grid"),
        Field::number("angle", "Angle", 0.0, 360.0, 0.0, "Degrees counter-clockwise from horizontal"),
        Field::bool("angleLocked", "Lock Angle", false, "Dragging the guide moves it instead of turning it"),
        Field::choice(
            "relativeTo",
            "Measured From",
            &RELATIVE_TO,
            "left",
            "The sidebearing the guide's horizontal position follows when the width changes",
        ),
        Field::bool("measurement", "Measurement", false, "Show the distances between the guide's crossings with the outlines"),
        Field::text("name", "Name", MAX_TEXT, "Shown at the guide's left end; empty for none"),
        Field::text("color", "Colour", MAX_TEXT, "r,g,b,a with numbers from 0 to 1; empty for the standard local or global colour"),
    ]
}

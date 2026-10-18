//! Glyph programs kept from an opened hinted TrueType font (`hinting.tt.import.glyphPrograms`).
//!
//! Opening the font stores each glyph's program on its layer as assembly text
//! ([`IMPORTED_GLYPH_PROGRAM_KEY`]) beside a fingerprint of the outline it was written for
//! ([`IMPORTED_OUTLINE_KEY`]). Export writes the program back only while the layer's outline still
//! has that fingerprint: the program names points by number, so any change to the points, their
//! order or their on-curve flags, or to the components, makes it meaningless.
//!
//! The fingerprint covers what decides point numbers and positions in `glyf`: each path's
//! closedness and its nodes in order (on-curve flag and coordinates rounded to whole units), and
//! each component's base glyph and transform. It is FNV-1a over that data, written as 16
//! hexadecimal digits, so it stays the same across builds and platforms.

use fontcraft_model::plist::{Str, Value};
use fontcraft_model::{Layer, Shape};

use crate::settings::{IMPORTED_GLYPH_PROGRAM_KEY, IMPORTED_OUTLINE_KEY};

/// The imported program of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportedProgram<'a> {
    /// The layer has none.
    Absent,
    /// The program, as assembly text, for the layer's outline as it is.
    Current(&'a str),
    /// The layer has a program, but its outline changed since the font was opened.
    Stale,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    fn int(&mut self, v: i64) {
        self.bytes(&v.to_le_bytes());
    }
}

/// A whole number for hashing; the model keeps coordinates finite, and `as` saturates.
fn whole(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

/// The fingerprint of the layer's outline (see the module notes).
pub fn outline_fingerprint(layer: &Layer) -> String {
    let mut h = Fnv(FNV_OFFSET);
    for shape in &layer.shapes {
        match shape {
            Shape::Path(p) => {
                h.bytes(if p.closed { b"P" } else { b"p" });
                h.int(i64::try_from(p.nodes.len()).unwrap_or(i64::MAX));
                for n in &p.nodes {
                    h.bytes(if n.on_curve() { b"1" } else { b"0" });
                    h.int(whole(n.x));
                    h.int(whole(n.y));
                }
            }
            Shape::Component(c) => {
                h.bytes(b"C");
                h.int(i64::try_from(c.glyph.len()).unwrap_or(i64::MAX));
                h.bytes(c.glyph.as_bytes());
                // Transform values to 1/10,000, finer than F2Dot14 and whole-unit offsets.
                for v in c.transform.affine().as_coeffs() {
                    h.int(whole(v * 10_000.0));
                }
            }
            // An image is no outline.
            Shape::Image(_) => {}
            _ => h.bytes(b"?"),
        }
    }
    format!("{:016x}", h.0)
}

/// The layer's imported program and whether it still belongs to the outline.
pub fn imported_program(layer: &Layer) -> ImportedProgram<'_> {
    let data = &layer.info.user_data;
    let Some(text) = data.get(IMPORTED_GLYPH_PROGRAM_KEY).and_then(Value::as_str) else { return ImportedProgram::Absent };
    match data.get(IMPORTED_OUTLINE_KEY).and_then(Value::as_str) {
        Some(stored) if stored == outline_fingerprint(layer) => ImportedProgram::Current(text),
        _ => ImportedProgram::Stale,
    }
}

/// Stores `assembly` as the layer's imported program, for the outline the layer has now.
pub fn set_imported_program(layer: &mut Layer, assembly: String) {
    let fingerprint = outline_fingerprint(layer);
    let data = &mut layer.info.user_data.0;
    data.insert(IMPORTED_GLYPH_PROGRAM_KEY, Value::String(Str::new(assembly)));
    data.insert(IMPORTED_OUTLINE_KEY, Value::String(Str::new(fingerprint)));
}

/// Removes the layer's imported program and its fingerprint; whether there was anything to remove.
pub fn remove_imported_program(layer: &mut Layer) -> bool {
    let data = &mut layer.info.user_data.0;
    let mut removed = false;
    for key in [IMPORTED_GLYPH_PROGRAM_KEY, IMPORTED_OUTLINE_KEY] {
        while let Some(i) = data.entries.iter().position(|(k, _)| k.as_str() == key) {
            data.remove_at(i);
            removed = true;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Component, Node, NodeKind, Path, Transform};

    use super::*;

    fn layer() -> Layer {
        let mut l = Layer::new("m01", 500.0);
        let n = |x, y, k| Node::new(x, y, k);
        l.shapes.push(Shape::Path(Path::new(
            true,
            vec![n(0.0, 0.0, NodeKind::Line), n(100.0, 0.0, NodeKind::Line), n(150.0, 50.0, NodeKind::OffCurve), n(100.0, 100.0, NodeKind::QCurve)],
        )));
        l
    }

    #[test]
    fn a_program_belongs_to_the_outline_it_was_stored_with() {
        let mut l = layer();
        assert_eq!(imported_program(&l), ImportedProgram::Absent);
        set_imported_program(&mut l, "SVTCA[0]".into());
        assert_eq!(imported_program(&l), ImportedProgram::Current("SVTCA[0]"));

        // A move below half a unit keeps the exported points, so the program still applies.
        let mut nudged = l.clone();
        let Shape::Path(p) = &mut nudged.shapes[0] else { panic!() };
        p.nodes[1].x += 0.2;
        assert_eq!(imported_program(&nudged), ImportedProgram::Current("SVTCA[0]"));

        for change in 0..3 {
            let mut edited = l.clone();
            match change {
                0 => {
                    let Shape::Path(p) = &mut edited.shapes[0] else { panic!() };
                    p.nodes[1].x += 1.0;
                }
                1 => {
                    let Shape::Path(p) = &mut edited.shapes[0] else { panic!() };
                    p.nodes.rotate_left(1);
                }
                _ => edited.shapes.push(Shape::Component(Component {
                    glyph: "a".into(),
                    transform: Transform::Matrix([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
                    info: Default::default(),
                    extra: Default::default(),
                })),
            }
            assert_eq!(imported_program(&edited), ImportedProgram::Stale, "change {change}");
        }

        assert!(remove_imported_program(&mut l));
        assert_eq!(imported_program(&l), ImportedProgram::Absent);
        assert!(l.info.user_data.is_empty());
        assert!(!remove_imported_program(&mut l));
    }

    #[test]
    fn the_fingerprint_is_stable() {
        // Stored in saved files: a change here strands every imported program.
        assert_eq!(outline_fingerprint(&Layer::new("m01", 0.0)), format!("{FNV_OFFSET:016x}"));
        assert_eq!(outline_fingerprint(&layer()), outline_fingerprint(&layer()));
    }
}

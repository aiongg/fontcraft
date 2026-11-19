//! Loading one glyph of a compiled font at a pixel size and running its program under the
//! debugger's control.
//!
//! The glyph is loaded as skrifa 0.48.0 loads it for hinting (`FreeTypeScaler::load_simple` in
//! `src/outline/glyf/mod.rs`): points scaled to 26.6 with the size's fixed-point scale, four
//! phantom points from the horizontal metrics and the typographic ascender and descender, the
//! phantom points rounded before the program runs. The font and control value programs run
//! first, without stops. Only simple glyphs at the default location are loaded: a composite
//! glyph is refused with a message.

use skrifa::raw::tables::glyf::bytecode::Decoder;
use skrifa::raw::tables::glyf::{Glyph, PointFlags};
use skrifa::raw::types::{F26Dot6, Fixed, GlyphId, Point};
use skrifa::raw::{FontRef, TableProvider};

pub use super::engine::{Outcome, Snapshot, StepInfo, VmPoint};
use super::{HintInstance, HintOutline, Outlines, Target};

/// Most instructions listed for one glyph program (a program holds at most 65,535 bytes).
pub const MAX_LISTED_INSTRUCTIONS: usize = 65_535;

/// The hinting state of one font at one pixel size: `fpgm` and `prep` have run.
pub struct SizeContext<'a> {
    pub outlines: Outlines<'a>,
    pub instance: HintInstance,
    pub ppem: u16,
    scale: Fixed,
    /// Why the control value program failed, if it did; glyphs then run unhinted.
    pub setup_error: Option<String>,
}

/// One glyph program run to a stop.
pub struct GlyphRun {
    pub snapshot: Snapshot,
    /// Outline points, without the four phantom points that end the glyph zone.
    pub point_count: usize,
    /// The last point of each contour.
    pub contours: Vec<u16>,
    /// The glyph program: byte offset and text of each instruction.
    pub listing: Vec<(usize, String)>,
    /// False when the control value program turned hinting off or failed; the program is then
    /// not run.
    pub hinting: bool,
}

impl<'a> SizeContext<'a> {
    /// Runs the font's `fpgm` and `prep` at `ppem` for `target`.
    pub fn new(font: &FontRef<'a>, ppem: u16, target: Target) -> Result<Self, String> {
        let outlines = Outlines::new(font).ok_or("the font has no head or maxp table")?;
        if outlines.units_per_em == 0 {
            return Err("the font's units per em is 0".into());
        }
        let ppem_bits = i32::from(ppem) * 64;
        let scale = Fixed::from_bits(ppem_bits) / Fixed::from_bits(i32::from(outlines.units_per_em));
        // FreeType's rounded ppem, from the scale (skrifa's `HintingInstance::reconfigure`).
        let rounded_ppem = ((scale * Fixed::from_bits(i32::from(outlines.units_per_em))).to_bits() + 32) >> 6;
        let mut instance = HintInstance::default();
        let setup_error = instance.reconfigure(&outlines, scale.to_bits(), rounded_ppem, target, &[]).err().map(|e| e.to_string());
        Ok(Self { outlines, instance, ppem, scale, setup_error })
    }

    fn apply(&self, value: i32) -> F26Dot6 {
        F26Dot6::from_bits((Fixed::from_bits(value) * self.scale).to_bits())
    }

    /// Whether glyph programs run at this size.
    pub fn hinting(&self) -> bool {
        self.setup_error.is_none() && self.instance.is_enabled()
    }

    /// Loads glyph `gid` and runs its program until `stop` says to stop before an instruction.
    pub fn run_glyph(&self, gid: u32, stop: &mut dyn FnMut(&StepInfo) -> bool) -> Result<GlyphRun, String> {
        let font = &self.outlines.font;
        let head = font.head().map_err(|e| format!("the font's head table cannot be read: {e}"))?;
        let loca = font.loca(Some(head.index_to_loc_format() == 1)).map_err(|e| format!("the font's loca table cannot be read: {e}"))?;
        let glyf = font.glyf().map_err(|e| format!("the font's glyf table cannot be read: {e}"))?;
        let id = GlyphId::new(gid);
        let glyph = loca.get_glyf(id, &glyf).map_err(|e| format!("glyph {gid} cannot be read: {e}"))?;
        let simple = match &glyph {
            Some(Glyph::Simple(simple)) => Some(simple),
            Some(Glyph::Composite(_)) => return Err(format!("glyph {gid} is made of components; the debugger runs simple glyphs only")),
            None => None,
        };
        let bounds = match &glyph {
            Some(g) => [g.x_min(), g.x_max(), g.y_min(), g.y_max()].map(i32::from),
            None => [0; 4],
        };
        let hmtx = font.hmtx().map_err(|e| format!("the font's hmtx table cannot be read: {e}"))?;
        let lsb = hmtx.side_bearing(id).map(i32::from).unwrap_or_default();
        let advance = hmtx.advance(id).map(i32::from).unwrap_or_default();
        let [ascent, descent] = self.outlines.os2_vmetrics.map(i32::from);
        let tsb = ascent - bounds[3];
        let vadvance = ascent - descent;
        let p0 = Point::new(bounds[0] - lsb, 0);
        let p2 = Point::new(0, bounds[3] + tsb);
        let phantom_unscaled = [p0, Point::new(p0.x + advance, 0), p2, Point::new(0, p2.y - vadvance)];

        let point_count = simple.map(|s| s.num_points()).unwrap_or(0);
        let mut unscaled = vec![Point::<i32>::default(); point_count];
        let mut flags = vec![PointFlags::default(); point_count];
        let mut contours = Vec::new();
        let mut bytecode: &[u8] = &[];
        if let Some(simple) = simple {
            simple.read_points_fast(&mut unscaled, &mut flags).map_err(|e| format!("glyph {gid}'s points cannot be read: {e}"))?;
            let mut last = 0u16;
            for end in simple.end_pts_of_contours() {
                let end = end.get();
                if end < last {
                    return Err(format!("glyph {gid}'s contour end points are out of order"));
                }
                last = end;
                contours.push(end);
            }
            bytecode = simple.instructions();
        }
        unscaled.extend(phantom_unscaled);
        flags.extend([PointFlags::default(); 4]);
        let mut scaled: Vec<Point<F26Dot6>> = unscaled.iter().map(|p| Point::new(self.apply(p.x), self.apply(p.y))).collect();
        let mut original_scaled = scaled.clone();
        for p in scaled.iter_mut().skip(point_count) {
            p.x = p.x.round();
            p.y = p.y.round();
        }
        let hinting = self.hinting();
        let program = if hinting { bytecode } else { &[] };
        let listing = listing(program);
        let mut phantom = [Point::<F26Dot6>::default(); 4];
        let mut stack = vec![0; self.instance.max_stack()];
        let mut cvt = vec![0; self.instance.cvt_len()];
        let mut storage = vec![0; self.instance.storage_len()];
        let twilight = self.instance.twilight_len();
        let mut twilight_scaled = vec![Point::<F26Dot6>::default(); twilight];
        let mut twilight_original_scaled = vec![Point::<F26Dot6>::default(); twilight];
        let mut twilight_flags = vec![PointFlags::default(); twilight];
        let mut outline = HintOutline {
            glyph_id: id,
            unscaled: &unscaled,
            scaled: &mut scaled,
            original_scaled: &mut original_scaled,
            flags: &mut flags,
            contours: &contours,
            phantom: &mut phantom,
            bytecode: program,
            stack: &mut stack,
            cvt: &mut cvt,
            storage: &mut storage,
            twilight_scaled: &mut twilight_scaled,
            twilight_original_scaled: &mut twilight_original_scaled,
            twilight_flags: &mut twilight_flags,
            is_composite: false,
            coords: &[],
        };
        let snapshot = self.instance.hint_debug(&self.outlines, &mut outline, stop).map_err(|e| format!("the hinting state is inconsistent: {e}"))?;
        Ok(GlyphRun { snapshot, point_count, contours, listing, hinting })
    }
}

/// The instructions of `program` with their byte offsets; a program that cannot be decoded to
/// its end lists what decodes.
pub fn listing(program: &[u8]) -> Vec<(usize, String)> {
    let mut decoder = Decoder::new(program, 0);
    let mut out = Vec::new();
    while let Some(Ok(ins)) = decoder.decode() {
        if out.len() >= MAX_LISTED_INSTRUCTIONS {
            break;
        }
        out.push((ins.pc, ins.to_string()));
    }
    out
}

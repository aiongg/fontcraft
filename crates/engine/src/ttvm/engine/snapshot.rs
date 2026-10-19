//! Stopping before an instruction and copying out the interpreter's state (FontCraft's addition
//! to the ported interpreter).
//!
//! [`Engine::run_stepping`] is [`Engine::run`] with a question asked before each instruction:
//! stop here? Instructions are counted from the start of the program being run, those inside
//! called functions included, so "the state after n steps" is the same on every run.

use skrifa::raw::tables::glyf::PointMarker;

use super::super::program::Program;
use super::super::round::RoundMode;
use super::super::zone::{Zone, ZonePointer};
use super::{Engine, HintError, HintErrorKind, MAX_RUN_INSTRUCTIONS};

/// Where the interpreter is, asked before each instruction runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepInfo {
    /// Instructions run so far in this program.
    pub step: usize,
    pub program: Program,
    /// The instruction's byte offset in `program`.
    pub pc: usize,
    /// Function and instruction calls in progress.
    pub depth: usize,
}

/// How a stepping run ended.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// Stopped before the instruction in [`Snapshot::next`].
    Stopped,
    /// The program ran to its end.
    Finished,
    /// An instruction failed; the state is as the failure left it.
    Failed(HintError),
}

/// One point of a zone. Coordinates are 26.6 fixed point (64 per pixel).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VmPoint {
    pub x: i32,
    pub y: i32,
    /// The scaled position before any instruction moved it.
    pub original_x: i32,
    pub original_y: i32,
    pub on_curve: bool,
    pub touched_x: bool,
    pub touched_y: bool,
}

/// The interpreter's state at a stop.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub outcome: Outcome,
    /// Instructions run before the stop.
    pub steps: usize,
    /// The instruction about to run: its program, byte offset and text. `None` at the end.
    pub next: Option<(Program, usize, String)>,
    pub depth: usize,
    /// The value stack, bottom first.
    pub stack: Vec<i32>,
    /// Projection, dual projection and freedom vectors, as 2.14 fixed point.
    pub projection: (i32, i32),
    pub dual_projection: (i32, i32),
    pub freedom: (i32, i32),
    pub rp: [usize; 3],
    /// Zone pointers: 0 twilight, 1 glyph.
    pub zp: [u8; 3],
    pub loop_counter: u32,
    pub round: &'static str,
    /// Minimum distance, control value cut-in, single width cut-in and single width value, 26.6.
    pub min_distance: i32,
    pub control_value_cutin: i32,
    pub single_width_cutin: i32,
    pub single_width: i32,
    pub delta_base: u16,
    pub delta_shift: u16,
    pub auto_flip: bool,
    pub instruct_control: u8,
    pub backward_compatibility: bool,
    pub glyph: Vec<VmPoint>,
    pub twilight: Vec<VmPoint>,
    pub storage: Vec<i32>,
    /// The control values in pixels, 26.6.
    pub cvt: Vec<i32>,
}

fn zone_points(zone: &Zone) -> Vec<VmPoint> {
    zone.points
        .iter()
        .zip(zone.original.iter())
        .zip(zone.flags.iter())
        .map(|((p, o), f)| VmPoint {
            x: p.x.to_bits(),
            y: p.y.to_bits(),
            original_x: o.x.to_bits(),
            original_y: o.y.to_bits(),
            on_curve: f.is_on_curve(),
            touched_x: f.has_marker(PointMarker::TOUCHED_X),
            touched_y: f.has_marker(PointMarker::TOUCHED_Y),
        })
        .collect()
}

fn round_name(mode: RoundMode) -> &'static str {
    match mode {
        RoundMode::Grid => "grid",
        RoundMode::HalfGrid => "halfGrid",
        RoundMode::DoubleGrid => "doubleGrid",
        RoundMode::DownToGrid => "downToGrid",
        RoundMode::UpToGrid => "upToGrid",
        RoundMode::Off => "off",
        RoundMode::Super => "super",
        RoundMode::Super45 => "super45",
    }
}

fn zp_index(zp: ZonePointer) -> u8 {
    match zp {
        ZonePointer::Twilight => 0,
        ZonePointer::Glyph => 1,
    }
}

impl Engine<'_> {
    /// Runs the current program until `stop` says to stop before an instruction, the program
    /// ends, or an instruction fails; then copies out the state.
    pub fn run_stepping(&mut self, stop: &mut dyn FnMut(&StepInfo) -> bool) -> Snapshot {
        let mut steps = 0usize;
        loop {
            let program = self.program.current;
            let Some(decoded) = self.decode() else {
                return self.snapshot(Outcome::Finished, steps, None);
            };
            let ins = match decoded {
                Ok(ins) => ins,
                Err(e) => return self.snapshot(Outcome::Failed(e), steps, None),
            };
            let info = StepInfo { step: steps, program, pc: ins.pc, depth: self.program.call_stack.len() };
            let next = Some((program, ins.pc, ins.to_string()));
            if stop(&info) {
                return self.snapshot(Outcome::Stopped, steps, next);
            }
            if let Err(e) = self.dispatch(&ins) {
                return self.snapshot(Outcome::Failed(e), steps, next);
            }
            steps += 1;
            if steps > MAX_RUN_INSTRUCTIONS {
                let e = HintError { program, glyph_id: None, pc: ins.pc, opcode: Some(ins.opcode), kind: HintErrorKind::ExceededExecutionBudget };
                return self.snapshot(Outcome::Failed(e), steps, None);
            }
        }
    }

    fn snapshot(&self, outcome: Outcome, steps: usize, next: Option<(Program, usize, String)>) -> Snapshot {
        let g = &self.graphics;
        let vector = |p: skrifa::raw::types::Point<i32>| (p.x, p.y);
        let [twilight, glyph] = &g.zones;
        let storage = (0..self.storage.len()).map(|i| self.storage.get(i).unwrap_or_default()).collect();
        let cvt = (0..self.cvt.len()).map(|i| self.cvt.get(i).map(|v| v.to_bits()).unwrap_or_default()).collect();
        Snapshot {
            outcome,
            steps,
            next,
            depth: self.program.call_stack.len(),
            stack: self.value_stack.values().to_vec(),
            projection: vector(g.proj_vector),
            dual_projection: vector(g.dual_proj_vector),
            freedom: vector(g.freedom_vector),
            rp: [g.rp0, g.rp1, g.rp2],
            zp: [zp_index(g.zp0), zp_index(g.zp1), zp_index(g.zp2)],
            loop_counter: g.loop_counter,
            round: round_name(g.round_state.mode),
            min_distance: g.min_distance.to_bits(),
            control_value_cutin: g.control_value_cutin.to_bits(),
            single_width_cutin: g.single_width_cutin.to_bits(),
            single_width: g.single_width.to_bits(),
            delta_base: g.delta_base,
            delta_shift: g.delta_shift,
            auto_flip: g.auto_flip,
            instruct_control: g.instruct_control,
            backward_compatibility: g.backward_compatibility,
            glyph: zone_points(glyph),
            twilight: zone_points(twilight),
            storage,
            cvt,
        }
    }
}

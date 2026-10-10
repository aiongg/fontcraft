//! The Rotate tool (`R`, `[[tool]] rotate`).
//!
//! | Gesture | Result |
//! |---|---|
//! | click | places the reference point (`transform.setOrigin` with `point`) |
//! | drag | `transform.rotateCounterclockwise` of the selection about the reference point, or about the centre of the selection's box while none is placed; the angle is the pointer's turn about that point since the press |
//! | `Shift` while dragging | the angle in steps of 90° |
//! | `Escape` during a drag | cancel it |
//!
//! With nothing selected the drag turns the whole layer, as the Palette does. The state machine is
//! shared with the Scale tool ([`crate::bbox::Freehand`]).

use crate::bbox::{FreeKind, Freehand};
use crate::event::{Key, KeyEvent, Modifiers, PointerEvent};
use crate::output::{Output, Overlay};
use crate::tool::{Context, Tool};

/// The Rotate tool.
#[derive(Clone, Debug)]
pub struct RotateTool(Freehand);

impl Default for RotateTool {
    fn default() -> Self {
        Self(Freehand::new(FreeKind::Rotate))
    }
}

impl Tool for RotateTool {
    fn id(&self) -> &'static str {
        "rotate"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        self.0.pointer(cx, ev)
    }

    fn key(&mut self, _cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.pressed && ev.key == Key::Escape && self.0.busy() {
            let mut out = self.0.cancel();
            out.handled = true;
            return out;
        }
        Output::default()
    }

    fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        self.0.modifiers(cx, mods)
    }

    fn cancel(&mut self) -> Output {
        self.0.cancel()
    }

    fn busy(&self) -> bool {
        self.0.busy()
    }

    fn overlays(&self) -> Vec<Overlay> {
        self.0.overlays()
    }
}
